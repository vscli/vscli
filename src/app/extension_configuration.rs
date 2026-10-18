//! Extension configuration writes reuse the native settings writer and proofs.
use super::*;
use crate::{
    extensions::services::NativeService,
    settings_writer::{Event, Intent, Outcome, Writer},
};
use anyhow::{Context as _, ensure};
use std::sync::{Arc, atomic::Ordering};

#[derive(Clone)]
pub(crate) struct WriteContext {
    profile: u64,
    workspace: PathBuf,
    user: Option<PathBuf>,
}
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Args {
    key: String,
    value: Value,
    remove: bool,
    target: u8,
    language: Option<String>,
    resource: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Capture {
    context: WriteContext,
    args: Args,
    path: PathBuf,
}
impl WriteContext {
    pub(crate) fn capture(&self, value: Value) -> Result<Arc<Capture>> {
        let args: Args = serde_json::from_value(value)?;
        ensure!(matches!(args.target, 1..=3), "Invalid configuration target");
        if args.target == 3 {
            let resource = args
                .resource
                .as_ref()
                .context("WorkspaceFolder target requires a resource")?;
            ensure!(
                resource.len() <= 4096,
                "Configuration resource exceeds 4096 bytes"
            );
            let uri = url::Url::parse(resource)?;
            let path = uri.to_file_path().map_err(|_| {
                anyhow::anyhow!("Configuration folder resource requires a local file URI")
            })?;
            ensure!(
                !path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir)),
                "Configuration folder resource contains unresolved parent segments"
            );
            let root = crate::lsp::extension_host_path(&self.workspace)?
                .to_str()
                .context("Configuration workspace must be UTF-8")?
                .to_owned();
            let resource = crate::lsp::extension_host_path(&path)?
                .to_str()
                .context("Configuration resource must be UTF-8")?
                .to_owned();
            let (root, resource) = if cfg!(windows) {
                (
                    root.to_lowercase().replace('\\', "/"),
                    resource.to_lowercase().replace('\\', "/"),
                )
            } else {
                (root, resource)
            };
            ensure!(
                resource == root
                    || resource
                        .strip_prefix(root.trim_end_matches('/'))
                        .is_some_and(|suffix| suffix.starts_with('/')),
                "Configuration resource is outside the selected workspace"
            );
        }
        let path = if args.target == 1 {
            self.user
                .clone()
                .context("Global settings path is unavailable")?
        } else {
            self.workspace.join(".vscode/settings.json")
        };
        // Reuse native value/depth/key/path admission before retaining the RPC.
        Intent::configuration(
            path.clone(),
            self.profile,
            args.key.clone(),
            (!args.remove).then(|| args.value.clone()),
            args.language.clone(),
        )?;
        Ok(Arc::new(Capture {
            context: self.clone(),
            args,
            path,
        }))
    }
}
impl Capture {
    fn current(&self, app: &App) -> bool {
        app.settings_write_ready()
            && app.settings_error.is_none()
            && self.context.profile == app.settings_profile_generation()
            && self.context.workspace == app.workspace.root
            && self.context.user == app.settings_user
    }
}
struct Active {
    service: NativeService,
    capture: Arc<Capture>,
    models: super::settings_persistence::WriteModels,
    id: u64,
    reload: Option<Arc<()>>,
    ready: Option<std::result::Result<(), String>>,
    refusal: Option<String>,
}
#[derive(Default)]
pub(super) struct State {
    writer: Writer,
    active: Option<Active>,
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.active.is_some() || self.writer.busy()
    }
    pub(super) fn pending(&self, session: u64, id: &str) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.service.request.session == session && active.service.id == id
        })
    }
}
impl App {
    pub(super) fn configuration_write_context(&self) -> Option<WriteContext> {
        (self.settings_write_ready()
            && self.settings_loader.is_some()
            && self.settings_error.is_none())
        .then(|| WriteContext {
            profile: self.settings_profile_generation(),
            workspace: self.workspace.root.clone(),
            user: self.settings_user.clone(),
        })
    }
    pub(super) fn start_extension_configuration_write(
        &mut self,
        service: NativeService,
        capture: Arc<Capture>,
    ) -> Result<()> {
        if self.extension_configuration.busy() || self.settings_writes_busy() {
            return Ok(());
        }
        ensure!(
            capture.current(self),
            "Configuration profile changed before write; retry"
        );
        let models = self.capture_settings_write_models(&capture.path)?;
        let intent = Intent::configuration(
            capture.path.clone(),
            capture.context.profile,
            capture.args.key.clone(),
            (!capture.args.remove).then(|| capture.args.value.clone()),
            capture.args.language.clone(),
        )?
        .with_models(models.paths())?;
        let id = self.extension_configuration.writer.request(intent)?;
        self.extension_configuration.active = Some(Active {
            service,
            capture,
            models,
            id,
            reload: None,
            ready: None,
            refusal: None,
        });
        Ok(())
    }
    pub(super) fn poll_extension_configuration_writes(&mut self) -> bool {
        let Some(mut active) = self.extension_configuration.active.take() else {
            return false;
        };
        let caller_current = !active.service.caller_retired.load(Ordering::Acquire)
            && active.service.fresh().is_ok()
            && self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.session == active.service.request.session);
        if (!caller_current || !active.capture.current(self)) && active.ready.is_none() {
            self.extension_configuration.writer.cancel(active.id);
            active.refusal =
                Some("Configuration caller, profile or workspace changed; write retired".into());
        }
        let mut changed = false;
        if let Some(event) = self.extension_configuration.writer.poll() {
            changed = true;
            match event {
                Event::Prepared(info) => {
                    let authorize = (|| -> Result<()> {
                        ensure!(
                            active.id == info.id
                                && active.capture.current(self)
                                && caller_current
                                && active.refusal.is_none(),
                            "Configuration write admission retired"
                        );
                        self.authorize_settings_write_models(&info, &active.models)?;
                        self.invalidate_disk_watch_publications()?;
                        self.extension_configuration.writer.authorize(info.id)
                    })();
                    if let Err(error) = authorize {
                        active.refusal = Some(format!("{error:#}"));
                        self.extension_configuration.writer.reject(info.id);
                    }
                }
                Event::Finished { id, result } => {
                    if id == active.id {
                        let committed = matches!(&result, Ok(Outcome::Committed { .. }));
                        if committed {
                            let _ = self.invalidate_disk_watch_publications();
                        }
                        match result {
                            Ok(Outcome::Committed{..}|Outcome::Unchanged) if active.refusal.is_none() && caller_current && active.capture.current(self)=>{
                                let reload=self.settings_reload_identity();
                                match self.settings_loader.as_mut().context("Settings loader is unavailable").and_then(crate::settings::Loader::force_reload) {
                                    Ok(())=>active.reload=Some(reload),
                                    Err(error)=>active.ready=Some(Err(format!("Setting persisted; reload failed: {error:#}"))),
                                }
                            }
                            Ok(Outcome::Committed{..})=>active.ready=Some(Err("Configuration write completed for its original profile after caller retirement".into())),
                            Ok(_)=>active.ready=Some(Err(active.refusal.clone().unwrap_or_else(||"Configuration write was not authorized".into()))),
                            Err(error)=>active.ready=Some(Err(error)),
                        }
                    }
                }
            }
        }
        if let Some(reload) = &active.reload
            && !Arc::ptr_eq(reload, &self.settings_reload_identity())
        {
            active.reload = None;
            active.ready = Some(
                if caller_current && active.capture.current(self) && self.settings_error.is_none() {
                    Ok(())
                } else {
                    Err(
                        "Setting persisted; original configuration reload/caller is unavailable"
                            .into(),
                    )
                },
            );
            changed = true;
        }
        if !self.extension_configuration.writer.busy()
            && (!caller_current || !active.capture.current(self))
        {
            if active.ready.is_none() {
                active.ready = Some(Err(active
                    .refusal
                    .clone()
                    .unwrap_or_else(|| "Configuration write retired".into())));
            }
            if self
                .extension_host
                .as_ref()
                .is_none_or(|host| host.session != active.service.request.session)
            {
                return true;
            }
        }
        self.extension_configuration.active = Some(active);
        changed
    }
    pub(super) fn complete_extension_configuration_write(
        &mut self,
        host: &mut crate::extensions::Client,
    ) -> bool {
        let Some(mut active) = self.extension_configuration.active.take() else {
            return false;
        };
        let Some(ready) = active.ready.take() else {
            self.extension_configuration.active = Some(active);
            return false;
        };
        if host.session != active.service.request.session
            || host
                .service()
                .is_none_or(|service| service.id != active.service.id)
        {
            self.extension_configuration.active = Some(active);
            return false;
        }
        let result = ready.clone().map_err(anyhow::Error::msg).and_then(|()| {
            host.service_valid(&active.service)?;
            ensure!(
                active.capture.current(self),
                "Configuration profile changed before receipt"
            );
            Ok(Value::Null)
        });
        let answer = (|| -> Result<()> {
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)?;
            host.sync_configuration(&self.settings)?;
            host.answer_service(&active.service, result)
        })();
        match answer {
            Ok(()) => true,
            Err(error) => {
                self.message = format!("Configuration receipt retained: {error:#}");
                active.ready = Some(ready);
                self.extension_configuration.active = Some(active);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn retired_profile_round_trip_owner_and_newly_dirty_target_cannot_authorize_original_configuration_work()
     {
        for retire in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(root.path()).unwrap();
            let path = root.join("settings.json");
            let original = "{\"fixture.value\": \"before\"}\r\n";
            std::fs::write(&path, original).unwrap();
            let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
            app.configure_settings(Some(path.clone())).unwrap();
            if retire == 2 {
                app.open(&path).unwrap();
            }
            let package = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/configuration-update-extension");
            app.start_extension_packages(vec![crate::extensions::Package::read(&package).unwrap()])
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while app.extension_host.as_ref().is_none_or(|host| !host.ready) {
                app.poll();
                assert!(Instant::now() < deadline, "{}", app.message);
                std::thread::sleep(Duration::from_millis(1));
            }
            app.execute("fixture.configuration.roundtrip", Value::Null);
            while app.extension_configuration.active.is_none() {
                // Do not poll the native writer: hold real preparation before authorization.
                app.poll_extensions();
                assert!(Instant::now() < deadline, "{}", app.message);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(app.extension_configuration.writer.busy());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            match retire {
                0 => {
                    let other = root.join("other.json");
                    std::fs::write(&other, "{}").unwrap();
                    app.configure_settings(Some(other.clone())).unwrap();
                    app.configure_settings(Some(path.clone())).unwrap();
                    assert_eq!(std::fs::read_to_string(other).unwrap(), "{}");
                }
                1 => app
                    .extension_configuration
                    .active
                    .as_ref()
                    .unwrap()
                    .service
                    .caller_retired
                    .store(true, Ordering::Release),
                _ => {
                    app.doc_mut().insert("unsaved ", true);
                }
            }
            while app.extension_configuration.busy() {
                app.poll();
                assert!(Instant::now() < deadline, "{}", app.message);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            if retire == 2 {
                assert!(app.doc().dirty());
                assert!(app.doc().text.to_string().starts_with("unsaved "));
                app.execute("undo", Value::Null);
                assert_eq!(app.doc().text.to_string(), original);
            }
            assert!(!root.join(".vscli-write.lock").exists());
            app.execute("vscli.extensions.stop", Value::Null);
        }
    }
}
