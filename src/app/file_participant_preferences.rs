//! Profile-owned native choice reset. Filesystem work belongs to the joined worker.
use super::*;
use crate::{
    file_edit_review::preferences::{Info, Store, Write},
    resource_worker::{Event as WorkerEvent, Outcome, Worker},
};
use anyhow::{Context, ensure};
use std::time::{Duration, Instant};
struct Request {
    id: u64,
    profile: u64,
    path: PathBuf,
    choice: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn app() -> (tempfile::TempDir, App, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let profile = root.join("profile");
        std::fs::create_dir(&profile).unwrap();
        let mut app = App::new(workspace, Profile::Linux);
        app.configure_settings(Some(profile.join("settings.json")))
            .unwrap();
        let store = Store::new(&profile).unwrap();
        std::fs::write(store.path(),serde_json::to_vec(&json!({"schema":1,"values":{(crate::file_edit_review::preferences::KEY):true,"unrelated":"猫🙂"}})).unwrap()).unwrap();
        (directory, app, store.path().to_owned())
    }
    fn finish(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.file_participant_preferences.worker.busy() {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn take_prepared(app: &mut App) -> (u64, std::sync::Arc<Info>) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(event) = app.file_participant_preferences.worker.poll() {
                return match event {
                    WorkerEvent::Prepared { id, info, .. } => (id, info),
                    _ => panic!("Unexpected terminal"),
                };
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn native_reset_runs_with_no_editor_or_javascript_and_preserves_other_profile_values() {
        let (_directory, mut app, path) = app();
        assert!(app.documents.is_empty());
        app.execute("files.participants.resetChoice", Value::Null);
        assert!(app.file_participant_preview_choice_reset());
        finish(&mut app);
        assert_eq!(app.message, "File-operation preview choice reset");
        assert!(app.documents.is_empty());
        assert!(app.extension_host.is_none());
        let state: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(
            state["values"]
                .get(crate::file_edit_review::preferences::KEY)
                .is_none()
        );
        assert_eq!(state["values"]["unrelated"], "猫🙂");
    }
    #[test]
    fn profile_a_b_a_retires_unapproved_reset_without_renewing_old_overlay_or_overlapping_worker() {
        let (directory, mut app, path) = app();
        let original = std::fs::read(&path).unwrap();
        app.reset_file_participant_choice().unwrap();
        take_prepared(&mut app);
        let b = directory.path().join("profile-b/settings.json");
        app.configure_settings(Some(b)).unwrap();
        app.poll_file_participant_preferences();
        assert!(app.file_participant_preferences.worker.busy());
        assert!(app.reset_file_participant_choice().is_err());
        app.configure_settings(Some(path.with_file_name("settings.json")))
            .unwrap();
        assert!(!app.file_participant_preview_choice_reset());
        finish(&mut app);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn authorized_reset_finishes_only_its_original_profile_after_switch() {
        let (directory, mut app, path) = app();
        app.reset_file_participant_choice().unwrap();
        let (id, info) = take_prepared(&mut app);
        assert!(app.participant_preferences_current(id, Some(&info)));
        app.file_participant_preferences
            .worker
            .authorize(id)
            .unwrap();
        let b = directory.path().join("profile-b/settings.json");
        app.configure_settings(Some(b.clone())).unwrap();
        finish(&mut app);
        let state: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(
            state["values"]
                .get(crate::file_edit_review::preferences::KEY)
                .is_none()
        );
        assert!(
            !b.with_file_name("file-participant-preferences.json")
                .exists()
        );
        assert!(!app.file_participant_preview_choice_reset());
    }
    #[test]
    fn dirty_open_preference_buffer_and_exact_disk_bytes_survive_refused_reset() {
        let (_directory, mut app, path) = app();
        let original = std::fs::read(&path).unwrap();
        app.open(&path).unwrap();
        app.doc_mut().insert("dirty ", false);
        let id = app.doc().id;
        let text = app.doc().text.clone();
        app.reset_file_participant_choice().unwrap();
        finish(&mut app);
        assert!(app.message.contains("stored choice retained"));
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text, text);
        assert!(app.doc().dirty());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
    #[test]
    fn real_selected_extension_can_delegate_reset_from_the_empty_editor() {
        let (_directory, mut app, path) = app();
        let extension = path.parent().unwrap().join("extension");
        std::fs::create_dir(&extension).unwrap();
        std::fs::write(extension.join("package.json"),serde_json::to_vec(&json!({"publisher":"fixture","name":"reset","version":"1.0.0","main":"main.cjs","engines":{"vscode":"*"},"activationEvents":["*"]})).unwrap()).unwrap();
        std::fs::write(extension.join("main.cjs"),"const vscode=require('vscode');exports.activate=async()=>{if(vscode.workspace.textDocuments.length)throw Error('expected empty editor');await vscode.commands.executeCommand('files.participants.resetChoice');vscode.window.showInformationMessage('reset delegated');};").unwrap();
        app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if app.extension_host.as_ref().is_some_and(|host| host.ready)
                && !app.file_participant_preferences.worker.busy()
            {
                break;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
        let state: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(
            state["values"]
                .get(crate::file_edit_review::preferences::KEY)
                .is_none()
        );
        assert!(app.documents.is_empty());
    }
}
#[derive(Default)]
pub(super) struct State {
    worker: Worker<Write>,
    next: u64,
    request: Option<Request>,
    reset: Option<(u64, PathBuf)>,
    cached: Option<(u64, PathBuf, Option<bool>)>,
}
impl App {
    pub(super) fn participant_preference_store(&self) -> Result<Store> {
        let settings = self
            .user_settings_path()
            .context("Native settings profile path unavailable")?;
        Store::new(
            settings
                .parent()
                .context("Native profile directory unavailable")?,
        )
    }
    pub(super) fn reset_file_participant_choice(&mut self) -> Result<()> {
        self.write_file_participant_choice(None)?;
        self.message = "Resetting file-operation preview choice…".into();
        Ok(())
    }
    pub(super) fn write_file_participant_choice(&mut self, choice: Option<bool>) -> Result<()> {
        ensure!(
            !self.file_participant_preferences.worker.busy(),
            "File-participant preference worker is still occupied"
        );
        let store = self.participant_preference_store()?;
        let profile = self.settings_profile_generation();
        let id = self
            .file_participant_preferences
            .next
            .checked_add(1)
            .context("File-participant preference identities exhausted")?;
        self.file_participant_preferences.worker.start(
            id,
            store.write(choice, profile),
            Instant::now() + Duration::from_secs(6),
        )?;
        self.file_participant_preferences.next = id;
        self.file_participant_preferences.request = Some(Request {
            id,
            profile,
            path: store.path().to_owned(),
            choice,
        });
        self.file_participant_preferences.reset =
            choice.is_none().then(|| (profile, store.path().to_owned()));
        self.file_participant_preferences.cached = Some((profile, store.path().to_owned(), choice));
        Ok(())
    }
    pub(super) fn cached_file_participant_choice(&self) -> Option<Option<bool>> {
        self.file_participant_preferences
            .cached
            .as_ref()
            .and_then(|(profile, path, choice)| {
                (*profile == self.settings_profile_generation()
                    && self
                        .participant_preference_store()
                        .is_ok_and(|store| store.path() == path))
                .then_some(*choice)
            })
    }
    /// A reset is immediately visible in the original profile, like the source
    /// storage cache. It cannot become a remembered value in a different profile.
    pub fn file_participant_preview_choice_reset(&self) -> bool {
        self.file_participant_preferences
            .reset
            .as_ref()
            .is_some_and(|(profile, path)| {
                *profile == self.settings_profile_generation()
                    && self
                        .participant_preference_store()
                        .is_ok_and(|store| store.path() == path)
            })
    }
    fn participant_preferences_current(&self, id: u64, info: Option<&Info>) -> bool {
        self.file_participant_preferences
            .request
            .as_ref()
            .is_some_and(|request| {
                request.id == id
                    && request.profile == self.settings_profile_generation()
                    && self
                        .participant_preference_store()
                        .is_ok_and(|store| store.path() == request.path)
                    && info.is_none_or(|info| {
                        info.profile == request.profile && info.path == request.path
                    })
            })
    }
    pub(super) fn poll_file_participant_preferences(&mut self) -> bool {
        if let Some(request) = &self.file_participant_preferences.request
            && !self.participant_preferences_current(request.id, None)
        {
            self.file_participant_preferences.worker.retire(request.id);
        }
        let Some(event) = self.file_participant_preferences.worker.poll() else {
            return false;
        };
        match event {
            WorkerEvent::Prepared { id, info, .. } => {
                let opened_dirty = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .any(|doc| {
                        doc.dirty()
                            && doc.path.as_ref().is_some_and(|path| {
                                path == &info.path || path == &info.canonical_path
                            })
                    });
                if !self.participant_preferences_current(id, Some(&info)) || opened_dirty {
                    self.file_participant_preferences.worker.retire(id);
                    if opened_dirty {
                        self.message = "Save the preference file before changing its choice".into();
                    }
                } else if let Err(error) = self.file_participant_preferences.worker.authorize(id) {
                    self.file_participant_preferences.worker.retire(id);
                    self.message = format!("Preference change refused: {error:#}");
                }
            }
            WorkerEvent::Finished { id, result, .. } => {
                let current = self.participant_preferences_current(id, None);
                let remembered = self
                    .file_participant_preferences
                    .request
                    .as_ref()
                    .is_some_and(|request| request.choice.is_some());
                self.file_participant_preferences.request = None;
                if current {
                    self.message = match result {
                        Ok(Outcome::Committed(receipt)) => receipt
                            .commit
                            .and_then(|commit| commit.durability_warning)
                            .map_or_else(
                                || {
                                    if remembered {
                                        "File-operation preview choice remembered".into()
                                    } else {
                                        "File-operation preview choice reset".into()
                                    }
                                },
                                |warning| format!("Choice stored; {warning}"),
                            ),
                        Ok(Outcome::Rejected) => {
                            "File-operation preference reset canceled; stored choice retained"
                                .into()
                        }
                        Ok(Outcome::Expired) => {
                            "File-operation preference reset expired; stored choice retained".into()
                        }
                        Err(error) => format!("File-operation preference reset failed: {error}"),
                    };
                }
            }
        }
        true
    }
}
