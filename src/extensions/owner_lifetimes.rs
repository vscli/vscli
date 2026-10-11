//! Native-owned selected caller lifetimes, independent of command correlation.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct OwnerLifetimes {
    owners: HashMap<String, Arc<AtomicBool>>,
}

impl OwnerLifetimes {
    pub(super) fn new(packages: &[Package]) -> Self {
        Self {
            owners: packages
                .iter()
                .map(|package| (package.id.clone(), Arc::new(AtomicBool::new(false))))
                .collect(),
        }
    }

    pub(super) fn capture(&self, owner: &str) -> Result<Arc<AtomicBool>> {
        let lifetime = self
            .owners
            .get(owner)
            .context("Native caller is not selected")?;
        if lifetime.load(Ordering::Acquire) {
            bail!("Native caller lifetime has retired");
        }
        Ok(lifetime.clone())
    }

    pub(super) fn admit_selected(&mut self, packages: &[Package]) {
        for package in packages {
            self.owners
                .entry(package.id.clone())
                .or_insert_with(|| Arc::new(AtomicBool::new(false)));
        }
    }

    // Only a completely validated native activation registry reaches this method.
    // A new activation can acquire a fresh lifetime but never renew an old one.
    pub(super) fn publish(&mut self, states: &std::collections::BTreeMap<String, String>) {
        for (owner, state) in states {
            let Some(lifetime) = self.owners.get_mut(owner) else {
                continue;
            };
            if matches!(state.as_str(), "failed" | "disposed") {
                lifetime.store(true, Ordering::Release);
            } else if matches!(state.as_str(), "activating" | "active")
                && lifetime.load(Ordering::Acquire)
            {
                *lifetime = Arc::new(AtomicBool::new(false));
            }
        }
    }
}

impl Drop for OwnerLifetimes {
    fn drop(&mut self) {
        for lifetime in self.owners.values() {
            lifetime.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newly_added_extension_can_open_and_save_during_real_activation() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let create = |name: &str, source: &str| {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(
                path.join("package.json"),
                serde_json::to_vec(&json!({
                    "publisher":"lifetime","name":name,"version":"1.0.0","main":"index.cjs"
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(path.join("index.cjs"), source).unwrap();
            Package::read(&path).unwrap()
        };
        let first = create("first", "exports.activate = () => {};");
        let added = create(
            "added",
            r#"
const v = require('vscode'), assert = require('node:assert/strict');
exports.activate = async () => {
  const d = await v.workspace.openTextDocument(v.Uri.joinPath(v.Uri.file(v.workspace.rootPath), 'original.txt'));
  assert.equal(d.getText(), 'original 猫🙂\r\n');
  assert.equal(await d.save(), true);
  await v.window.showInformationMessage('new native caller saved');
};
"#,
        );
        let text = "original 猫🙂\r\n";
        let file = root.join("original.txt");
        std::fs::write(&file, text).unwrap();
        let mut app = crate::app::App::new(root.clone(), crate::keys::Profile::Linux);
        app.settings = Settings::from_values(
            json!({"vscli.languageServer.enabled":false,"files.autoSave":"off",
                "editor.formatOnSave":false,"editor.codeActionsOnSave":{}})
            .as_object()
            .unwrap()
            .clone(),
            "native lifetime addition fixture",
        )
        .unwrap();
        app.configure_extension_storage(Some(&root.join("storage")));
        app.start_extension_packages(vec![first]).unwrap();
        let wait = |app: &mut crate::app::App, done: fn(&crate::app::App) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                app.poll();
                if done(app) {
                    break;
                }
                assert!(
                    !app.extension_host.as_ref().is_some_and(|host| host
                        .activation_states
                        .values()
                        .any(|state| state == "failed")),
                    "Activation failed: {:?}; {}",
                    app.extension_host
                        .as_ref()
                        .map(|host| &host.activation_states),
                    app.message
                );
                assert!(Instant::now() < deadline, "{}", app.message);
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        wait(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        let original = app
            .extension_host
            .as_ref()
            .unwrap()
            .owner_lifetimes
            .capture("lifetime.first")
            .unwrap();
        let request = ActivationRequest {
            surface_declarations: Default::default(),
            additions: vec![added],
            targets: vec!["lifetime.added".into()],
            owner: Some("lifetime.added".into()),
        };
        app.extension_host
            .as_mut()
            .unwrap()
            .activate_prepared_with_hidden(
                &request,
                json!({}),
                &app.documents,
                &app.hidden_documents,
                app.active,
                &app.settings,
            )
            .unwrap();
        wait(&mut app, |app| {
            app.extension_host
                .as_ref()
                .is_some_and(|host| host.owner_active("lifetime.added") && !host.activating())
        });
        let host = app.extension_host.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &original,
            &host.owner_lifetimes.capture("lifetime.first").unwrap()
        ));
        assert!(!original.load(Ordering::Acquire));
        assert!(host.owner_lifetimes.capture("lifetime.added").is_ok());
        assert!(
            app.documents.is_empty(),
            "activation must not open an editor"
        );
        assert_eq!(app.hidden_documents.len(), 1);
        let document = &app.hidden_documents[0];
        assert_eq!(document.text.to_string(), text);
        assert_eq!(document.path.as_ref(), Some(&file));
        assert_eq!(document.save_generation(), 1);
        assert!(!document.dirty());
        assert_eq!(std::fs::read(&file).unwrap(), text.as_bytes());
        assert!(!app.saves_pending());
    }

    fn packages() -> Vec<Package> {
        ["caller.a", "caller.b"]
            .into_iter()
            .map(|id| Package {
                id: id.into(),
                version: "1.0.0".into(),
                path: PathBuf::from(id),
                sha256: None,
            })
            .collect()
    }

    fn states(a: &str, b: &str) -> std::collections::BTreeMap<String, String> {
        [("caller.a".into(), a.into()), ("caller.b".into(), b.into())].into()
    }

    #[test]
    fn retirement_and_reactivation_never_renew_original_native_caller() {
        let mut registry = OwnerLifetimes::new(&packages());
        let first = registry.capture("caller.a").unwrap();
        let other = registry.capture("caller.b").unwrap();
        registry.publish(&states("active", "active"));
        assert!(Arc::ptr_eq(&first, &registry.capture("caller.a").unwrap()));
        registry.publish(&states("failed", "active"));
        assert!(first.load(Ordering::Acquire));
        assert!(!other.load(Ordering::Acquire));
        assert!(registry.capture("caller.a").is_err());
        registry.publish(&states("dormant", "active"));
        assert!(registry.capture("caller.a").is_err());
        registry.publish(&states("activating", "active"));
        let replacement = registry.capture("caller.a").unwrap();
        assert!(!replacement.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&first, &replacement));
        registry.publish(&states("active", "active"));
        assert!(Arc::ptr_eq(
            &replacement,
            &registry.capture("caller.a").unwrap()
        ));
        assert!(first.load(Ordering::Acquire));
        registry.publish(&states("disposed", "active"));
        assert!(replacement.load(Ordering::Acquire));
        assert!(!other.load(Ordering::Acquire));
    }

    #[test]
    fn dropping_registry_retires_every_selected_lifetime_and_session_isolation_holds() {
        let registry = OwnerLifetimes::new(&packages());
        let first = registry.capture("caller.a").unwrap();
        let other = registry.capture("caller.b").unwrap();
        let next = OwnerLifetimes::new(&packages());
        let replacement = next.capture("caller.a").unwrap();
        assert!(registry.capture("unselected.c").is_err());
        drop(registry);
        assert!(first.load(Ordering::Acquire));
        assert!(other.load(Ordering::Acquire));
        assert!(!replacement.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&first, &replacement));
    }
}
