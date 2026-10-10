//! UI ownership of background settings writes. No filesystem calls here.
use super::*;
use crate::{
    settings::RootWriteScope,
    settings_writer::{Event as WriteEvent, Intent, Outcome, Writer},
};
use anyhow::{Context, bail, ensure};

const MODELS: usize = 128;
const PATH_BYTES: usize = 512 * 1024;

struct ModelProof {
    id: u64,
    path: Option<PathBuf>,
    text_epoch: u64,
    save_generation: u64,
}
struct Request {
    id: u64,
    profile: u64,
    workspace: PathBuf,
    path: PathBuf,
    scope: RootWriteScope,
    language: String,
    models: Vec<ModelProof>,
    committed: bool,
}
#[derive(Default)]
pub(super) struct State {
    writer: Writer,
    profile: u64,
    configured: bool,
    ready: bool,
    latest: Option<Request>,
}
impl App {
    #[cfg(test)]
    pub(super) fn fixture_take_settings_prepared(
        &mut self,
    ) -> Option<crate::settings_writer::PreparedInfo> {
        self.settings_writes.writer.poll().map(|event| match event {
            WriteEvent::Prepared(info) => info,
            other => panic!("Expected actual settings preparation, received {other:?}"),
        })
    }
    #[cfg(test)]
    pub(super) fn fixture_authorize_settings_prepared(
        &mut self,
        info: &crate::settings_writer::PreparedInfo,
    ) -> Result<()> {
        self.authorize_settings_write(info)?;
        self.invalidate_disk_watch_publications()?;
        self.settings_writes.writer.authorize(info.id)
    }
    pub(super) fn settings_writes_busy(&self) -> bool {
        self.settings_writes.writer.busy()
    }
    pub(super) fn retire_unapproved_settings_write(&mut self) {
        if let Some(request) = self.settings_writes.latest.as_ref() {
            self.settings_writes.writer.cancel(request.id);
        }
    }
    pub(super) fn settings_persistence_configured(&self) -> bool {
        self.settings_writes.configured
    }
    pub(super) fn invalidate_settings_profile(&mut self) -> Result<()> {
        let next = self
            .settings_writes
            .profile
            .checked_add(1)
            .context("Settings profile identity exhausted; restart the editor")?;
        if let Some(request) = self.settings_writes.latest.take() {
            self.settings_writes.writer.cancel(request.id);
        }
        self.settings_writes.profile = next;
        self.retire_save_formatting("settings ownership changed during formatting");
        self.retire_autosave_interest();
        self.settings_writes.configured = true;
        self.settings_writes.ready = false;
        self.set_breadcrumbs_override(None);
        Ok(())
    }
    pub(super) fn settings_profile_loaded(&mut self) {
        self.settings_writes.ready = true;
    }
    pub(super) fn settings_profile_load_failed(&mut self) {
        self.settings_writes.ready = false;
        self.retire_save_formatting("settings ownership changed during formatting");
        self.retire_autosave_interest();
    }
    pub(super) fn request_persistent_breadcrumbs(&mut self, enabled: bool) -> Result<()> {
        ensure!(
            self.settings_writes.ready,
            "Settings profile failed to load; fix its settings before writing"
        );
        let language = self
            .active_document()
            .and_then(|doc| doc.path.as_deref())
            .map_or("plaintext", crate::languages::language)
            .to_owned();
        let scope = self
            .settings
            .root_write_target("breadcrumbs.enabled", &language)?;
        let path = match scope {
            RootWriteScope::User => self
                .settings_user
                .clone()
                .context("User settings path unavailable; configure a native settings profile")?,
            RootWriteScope::Workspace => self.workspace.root.join(".vscode/settings.json"),
        };
        let intent = Intent::new(
            path.clone(),
            self.settings_writes.profile,
            "breadcrumbs.enabled".into(),
            Value::Bool(enabled),
        )?;
        let mut models = Vec::new();
        let mut bytes = 0;
        for doc in self.documents.iter().chain(&self.hidden_documents) {
            ensure!(
                models.len() < MODELS,
                "Settings write proof exceeds 128 document models"
            );
            if let Some(doc_path) = &doc.path {
                bytes += doc_path.as_os_str().len();
                ensure!(
                    bytes <= PATH_BYTES,
                    "Settings write model paths exceed 512 KiB"
                );
                if doc_path == &path && doc.dirty() {
                    bail!(
                        "Settings buffer has unsaved changes; save or close it before toggling Breadcrumbs"
                    );
                }
            }
            models.push(ModelProof {
                id: doc.id,
                path: doc.path.clone(),
                text_epoch: doc.text_epoch(),
                save_generation: doc.save_generation(),
            });
        }
        let intent = intent.with_models(
            models
                .iter()
                .filter_map(|proof| proof.path.clone().map(|path| (proof.id, path)))
                .collect(),
        )?;
        let id = self.settings_writes.writer.request(intent)?;
        self.settings_writes.latest = Some(Request {
            id,
            profile: self.settings_writes.profile,
            workspace: self.workspace.root.clone(),
            path,
            scope,
            language,
            models,
            committed: false,
        });
        // This presentation override is owned by the latest write. Errors retire
        // it; successful reloads replace it with the persisted setting.
        self.set_breadcrumbs_override(Some(enabled));
        self.message = "Saving Breadcrumbs setting…".into();
        Ok(())
    }
    fn authorize_settings_write(&self, info: &crate::settings_writer::PreparedInfo) -> Result<()> {
        ensure!(
            self.settings_writes.ready,
            "Settings reload failed; write retired"
        );
        let request = self
            .settings_writes
            .latest
            .as_ref()
            .context("Settings write retired")?;
        ensure!(
            request.id == info.id
                && request.profile == info.profile
                && info.profile == self.settings_writes.profile
                && request.path == info.path
                && request.workspace == self.workspace.root,
            "Settings profile or workspace changed; write retired"
        );
        ensure!(
            self.settings
                .root_write_target("breadcrumbs.enabled", &request.language)?
                == request.scope,
            "Settings scope changed; write retired"
        );
        let mut count = 0;
        for doc in self.documents.iter().chain(&self.hidden_documents) {
            count += 1;
            ensure!(
                count <= MODELS,
                "Settings write proof exceeds 128 document models"
            );
            let Some(path) = &doc.path else {
                continue;
            };
            let proof = request
                .models
                .iter()
                .find(|proof| proof.id == doc.id)
                .context("Document opened during settings preparation; retry the toggle")?;
            ensure!(
                proof.path.as_ref() == Some(path),
                "Document path changed during settings preparation; retry the toggle"
            );
            if info.matching_models.contains(&doc.id)
                || path == &info.path
                || path == &info.canonical_path
            {
                ensure!(
                    !doc.dirty(),
                    "Settings buffer has unsaved changes; existing bytes preserved"
                );
                ensure!(
                    proof.path == doc.path
                        && proof.text_epoch == doc.text_epoch()
                        && proof.save_generation == doc.save_generation(),
                    "Settings buffer changed during preparation; existing bytes preserved"
                );
            }
        }
        Ok(())
    }
    pub(super) fn poll_settings_writes(&mut self) -> bool {
        let Some(event) = self.settings_writes.writer.poll() else {
            return false;
        };
        match event {
            WriteEvent::Prepared(info) => {
                let result = self.authorize_settings_write(&info).and_then(|()| {
                    self.invalidate_disk_watch_publications()?;
                    self.settings_writes.writer.authorize(info.id)
                });
                if let Err(error) = result {
                    self.settings_writes.writer.reject(info.id);
                    if self
                        .settings_writes
                        .latest
                        .as_ref()
                        .is_some_and(|request| request.id == info.id)
                    {
                        self.settings_writes.latest = None;
                        self.set_breadcrumbs_override(None);
                        self.message = format!("Settings write refused: {error:#}");
                    }
                }
            }
            WriteEvent::Finished { id, result } => {
                // Every committed file write fences watcher snapshots, including
                // writes authorized before switching to a different profile.
                let committed = matches!(&result, Ok(Outcome::Committed { .. }));
                let watch_warning = if committed {
                    self.invalidate_disk_watch_publications()
                        .err()
                        .map(|error| format!("disk watcher requires restart: {error:#}"))
                } else {
                    None
                };
                if !self.settings_writes.latest.as_ref().is_some_and(|request| {
                    request.id == id && request.profile == self.settings_writes.profile
                }) {
                    if let Some(warning) = watch_warning {
                        self.message = warning;
                    }
                    return committed;
                }
                match result {
                    Ok(Outcome::Committed {
                        durability_warning, ..
                    }) => {
                        self.settings_writes.latest.as_mut().unwrap().committed = true;
                        let result = self
                            .settings_loader
                            .as_mut()
                            .context("Settings loader unavailable")
                            .and_then(crate::settings::Loader::force_reload);
                        if let Err(error) = result {
                            self.settings_writes.latest = None;
                            self.set_breadcrumbs_override(None);
                            self.message = format!("Setting saved; reload unavailable: {error:#}");
                        } else {
                            let warning = match (durability_warning, watch_warning) {
                                (Some(durability), Some(watch)) => {
                                    Some(format!("directory sync failed: {durability}; {watch}"))
                                }
                                (Some(durability), None) => {
                                    Some(format!("directory sync failed: {durability}"))
                                }
                                (None, watch) => watch,
                            };
                            self.message = warning.map_or_else(
                                || "Breadcrumbs setting saved".into(),
                                |warning| format!("Setting saved; {warning}"),
                            );
                        }
                    }
                    result => {
                        self.settings_writes.latest = None;
                        self.set_breadcrumbs_override(None);
                        self.message = match result {
                            Err(error) => {
                                format!("Settings write failed; existing bytes preserved: {error}")
                            }
                            _ => "Settings write retired; existing bytes preserved".into(),
                        };
                    }
                }
            }
        }
        true
    }
    pub(super) fn settings_reload_settled(&mut self) -> bool {
        if self
            .settings_writes
            .latest
            .as_ref()
            .is_some_and(|request| request.committed)
        {
            self.settings_writes.latest = None;
            self.set_breadcrumbs_override(None);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, settings_writer::PreparedInfo};
    use std::time::{Duration, Instant};

    const ORIGINAL: &str = "{ // untouched 猫\r\n  \"breadcrumbs.enabled\": true\r\n}\r\n";

    struct Fixture {
        _workspace: tempfile::TempDir,
        _profile: tempfile::TempDir,
        app: App,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let workspace = tempfile::tempdir().unwrap();
            let profile = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(workspace.path()).unwrap();
            let directory = std::fs::canonicalize(profile.path()).unwrap();
            let path = directory.join("settings.json");
            std::fs::write(&path, ORIGINAL).unwrap();
            let mut app = App::new(root, crate::keys::Profile::Linux);
            app.configure_settings(Some(path.clone())).unwrap();
            Self {
                _workspace: workspace,
                _profile: profile,
                app,
                path,
            }
        }
        fn retain_clean_settings(&mut self) -> u64 {
            let document = Document::open(&self.path).unwrap();
            let id = document.id;
            assert!(!document.dirty());
            self.app.documents.push(document);
            id
        }
        fn prepared(&mut self) -> PreparedInfo {
            self.app.request_persistent_breadcrumbs(false).unwrap();
            let id = self.app.settings_writes.latest.as_ref().unwrap().id;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                // Deliberately bypass App::poll: it would authorize preparation.
                if let Some(event) = self.app.settings_writes.writer.poll() {
                    let WriteEvent::Prepared(info) = event else {
                        panic!("unexpected write event: {event:?}")
                    };
                    assert_eq!(info.id, id);
                    assert!(self.app.settings_writes.writer.awaiting_authorization(id));
                    assert_eq!(std::fs::read(&self.path).unwrap(), ORIGINAL.as_bytes());
                    return info;
                }
                assert!(
                    Instant::now() < deadline,
                    "settings preparation did not finish"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn retire(&mut self, info: &PreparedInfo) {
            self.app.settings_writes.writer.reject(info.id);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(event) = self.app.settings_writes.writer.poll() {
                    let WriteEvent::Finished { id, result } = event else {
                        panic!("unexpected write event: {event:?}")
                    };
                    assert_eq!(id, info.id);
                    assert!(matches!(result, Ok(Outcome::Rejected)), "{result:?}");
                    assert!(!self.app.settings_writes.writer.busy());
                    assert_eq!(std::fs::read(&self.path).unwrap(), ORIGINAL.as_bytes());
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "rejected actual writer did not settle"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn prepared_write_rejects_clean_edit_undo_aba_and_preserves_redo() {
        let mut fixture = Fixture::new();
        let id = fixture.retain_clean_settings();
        let original_selections = fixture.app.doc().selections();
        let epoch = fixture.app.doc().text_epoch();
        let save_generation = fixture.app.doc().save_generation();
        let info = fixture.prepared();
        assert!(info.matching_models.contains(&id));
        fixture.app.doc_mut().insert("λ", false);
        fixture.app.doc_mut().undo();
        let doc = fixture.app.doc();
        assert_eq!(doc.id, id);
        assert_eq!(doc.text.to_string(), ORIGINAL);
        assert_eq!(doc.selections(), original_selections);
        assert!(!doc.dirty());
        assert!(doc.text_epoch() > epoch);
        assert_eq!(doc.save_generation(), save_generation);
        assert!(
            fixture
                .app
                .authorize_settings_write(&info)
                .unwrap_err()
                .to_string()
                .contains("changed during preparation")
        );
        fixture.retire(&info);
        fixture.app.doc_mut().redo();
        assert_eq!(fixture.app.doc().text.to_string(), format!("λ{ORIGINAL}"));
        assert!(fixture.app.doc().dirty());
        fixture.app.doc_mut().undo();
        assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
        assert!(!fixture.app.doc().dirty());
        assert_eq!(fixture.app.doc().id, id);
        assert_eq!(fixture.app.doc().save_generation(), save_generation);
        assert_eq!(std::fs::read(&fixture.path).unwrap(), ORIGINAL.as_bytes());
    }

    fn retained_dirty_alias(fixture: &mut Fixture, alias: PathBuf) -> u64 {
        // Recovery restores a historical raw path, not Document::absolute_path.
        let mut document = Document::from_text(ORIGINAL);
        document.path = Some(alias);
        document.disk_content = Some(ropey::Rope::from_str(ORIGINAL));
        document.move_to(document.len(), false);
        document.insert("// unsaved λ\r\n", false);
        assert!(document.dirty());
        let id = document.id;
        fixture.app.hidden_documents.push(document);
        id
    }
    fn reject_dirty_alias(fixture: &mut Fixture, id: u64) {
        let original_text = fixture.app.hidden_documents[0].text.clone();
        let selections = fixture.app.hidden_documents[0].selections();
        let epoch = fixture.app.hidden_documents[0].text_epoch();
        let save_generation = fixture.app.hidden_documents[0].save_generation();
        let info = fixture.prepared();
        assert!(
            info.matching_models.contains(&id),
            "worker failed to identify retained alias"
        );
        assert!(
            fixture
                .app
                .authorize_settings_write(&info)
                .unwrap_err()
                .to_string()
                .contains("unsaved changes")
        );
        fixture.retire(&info);
        let doc = &fixture.app.hidden_documents[0];
        assert_eq!(doc.id, id);
        assert_eq!(doc.text, original_text);
        assert_eq!(doc.selections(), selections);
        assert_eq!(doc.text_epoch(), epoch);
        assert_eq!(doc.save_generation(), save_generation);
        assert!(doc.dirty());
        fixture.app.hidden_documents[0].undo();
        assert_eq!(fixture.app.hidden_documents[0].text.to_string(), ORIGINAL);
        assert!(!fixture.app.hidden_documents[0].dirty());
        fixture.app.hidden_documents[0].redo();
        assert_eq!(fixture.app.hidden_documents[0].text, original_text);
        assert_eq!(std::fs::read(&fixture.path).unwrap(), ORIGINAL.as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn hidden_recovered_third_parent_symlink_alias_rejects_prepared_write() {
        let mut fixture = Fixture::new();
        let raw_parent = fixture.app.workspace.root.join("first/second");
        std::fs::create_dir_all(&raw_parent).unwrap();
        let alias = raw_parent.join("third");
        std::os::unix::fs::symlink(fixture.path.parent().unwrap(), &alias).unwrap();
        let alias = alias.join("settings.json");
        assert_ne!(alias, fixture.path);
        let id = retained_dirty_alias(&mut fixture, alias);
        reject_dirty_alias(&mut fixture, id);
    }

    #[test]
    fn hidden_dirty_hardlink_alias_rejects_prepared_write() {
        let mut fixture = Fixture::new();
        let alias = fixture.path.with_file_name("retained-hardlink.json");
        std::fs::hard_link(&fixture.path, &alias).unwrap();
        let id = retained_dirty_alias(&mut fixture, alias.clone());
        reject_dirty_alias(&mut fixture, id);
        assert_eq!(std::fs::read(&alias).unwrap(), ORIGINAL.as_bytes());
    }

    #[test]
    fn profile_a_b_a_retires_held_preparation_without_authorizing_old_a() {
        let mut fixture = Fixture::new();
        let info = fixture.prepared();
        let old_profile = fixture.app.settings_writes.profile;
        let other = fixture.path.with_file_name("other-profile.json");
        std::fs::write(&other, ORIGINAL).unwrap();
        fixture.app.configure_settings(Some(other.clone())).unwrap();
        fixture
            .app
            .configure_settings(Some(fixture.path.clone()))
            .unwrap();
        assert_eq!(fixture.app.settings_writes.profile, old_profile + 2);
        assert!(fixture.app.settings_writes.ready);
        assert!(
            fixture
                .app
                .authorize_settings_write(&info)
                .unwrap_err()
                .to_string()
                .contains("retired")
        );
        assert!(
            fixture
                .app
                .settings_writes
                .writer
                .authorize(info.id)
                .is_err()
        );
        fixture.retire(&info);
        assert_eq!(std::fs::read(other).unwrap(), ORIGINAL.as_bytes());
    }

    #[test]
    fn new_unrelated_file_model_and_changed_path_require_a_fresh_preparation() {
        for new_model in [true, false] {
            let mut fixture = Fixture::new();
            let other = fixture.app.workspace.root.join("other.cpp");
            std::fs::write(&other, "// unrelated 猫\r\n").unwrap();
            let unrelated = Document::open(&other).unwrap();
            if !new_model {
                fixture.app.documents.push(unrelated);
            }
            let info = fixture.prepared();
            if new_model {
                fixture.app.documents.push(Document::open(&other).unwrap());
            } else {
                fixture.app.documents[0].path =
                    Some(fixture.app.workspace.root.join("changed.cpp"));
            }
            let message = fixture
                .app
                .authorize_settings_write(&info)
                .unwrap_err()
                .to_string();
            assert!(message.contains(if new_model {
                "Document opened"
            } else {
                "Document path changed"
            }));
            assert!(message.contains("retry the toggle"));
            fixture.retire(&info);
            assert_eq!(
                std::fs::read(other).unwrap(),
                "// unrelated 猫\r\n".as_bytes()
            );
        }
    }

    #[test]
    fn actual_failed_reload_blocks_writes_until_a_valid_reload_arrives() {
        let mut fixture = Fixture::new();
        let invalid = b"{ malformed settings";
        std::fs::write(&fixture.path, invalid).unwrap();
        fixture
            .app
            .settings_loader
            .as_mut()
            .unwrap()
            .force_reload()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while fixture.app.settings_writes.ready {
            fixture.app.poll();
            assert!(
                Instant::now() < deadline,
                "failed settings reload never arrived"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(fixture.app.settings_error.is_some());
        assert!(fixture.app.settings.breadcrumbs("plaintext").enabled);
        assert!(
            fixture
                .app
                .request_persistent_breadcrumbs(false)
                .unwrap_err()
                .to_string()
                .contains("failed to load")
        );
        assert!(!fixture.app.settings_writes.writer.busy());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), invalid);
        std::fs::write(&fixture.path, ORIGINAL).unwrap();
        fixture
            .app
            .settings_loader
            .as_mut()
            .unwrap()
            .force_reload()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !fixture.app.settings_writes.ready {
            fixture.app.poll();
            assert!(
                Instant::now() < deadline,
                "valid settings reload never arrived"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(fixture.app.settings_error.is_none());
        let info = fixture.prepared();
        assert!(fixture.app.authorize_settings_write(&info).is_ok());
        fixture.retire(&info);
    }
}
