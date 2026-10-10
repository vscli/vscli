use super::*;
use crate::files::{Action, Job};
impl App {
    pub(super) fn start_file_job(&mut self, action: Action) {
        if self.saves_pending() {
            self.message = "Wait for pending saves before changing file paths".into();
            return;
        }
        if self.file_job.is_some() {
            self.message = "A file operation is already running".into();
            return;
        }
        self.file_job = Some(Job::start(action));
        self.message = "File operation running…".into();
    }
    pub(super) fn refresh_files(&mut self) {
        self.entries = directory_entries(&self.explorer_dir);
        self.explorer_selected = self
            .explorer_selected
            .min(self.entries.len().saturating_sub(1));
        self.workspace.refresh();
        self.message = "Explorer refreshed; rebuilding file index".into();
    }
    pub(super) fn poll_files(&mut self) -> bool {
        let Some(result) = self.file_job.as_mut().and_then(Job::poll) else {
            return false;
        };
        self.file_job = None;
        match result {
            Err(error) => self.message = format!("File operation failed: {error}"),
            Ok(action) => {
                self.refresh_files();
                match action {
                    Action::CreateFile(path) => {
                        if let Err(e) = self.open(&path) {
                            self.message = format!("Created file, but open failed: {e:#}");
                        }
                    }
                    Action::CreateFolder(path) => {
                        self.message = format!("Created folder {}", path.display())
                    }
                    Action::Rename { from, to } => {
                        let to = std::fs::canonicalize(&to).unwrap_or(to);
                        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                            if let Some(path) = &doc.path
                                && let Ok(relative) = path.strip_prefix(&from)
                            {
                                doc.path = Some(if relative.as_os_str().is_empty() {
                                    to.clone()
                                } else {
                                    to.join(relative)
                                });
                            }
                        }
                        self.diagnostics.clear();
                        self.message = format!("Renamed to {}", to.display());
                    }
                    Action::Trash(path) => {
                        let mut retained = 0;
                        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                            if doc.path.as_ref().is_some_and(|p| p.starts_with(&path)) {
                                doc.path = None;
                                doc.disk_content = None;
                                doc.saved_revision = u64::MAX;
                                retained += 1;
                            }
                        }
                        self.diagnostics.clear();
                        self.message = format!(
                            "Moved to system trash; retained {retained} open buffer(s) as unsaved copies"
                        );
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_keeps_unsaved_buffer_and_save_targets_the_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("before.txt");
        let to = dir.path().join("after.txt");
        std::fs::write(&from, "original").unwrap();
        let mut app = App::new(dir.path().into(), Profile::Linux);
        app.open(&from).unwrap();
        app.doc_mut().insert("unsaved ", false);
        let id = app.doc().id;
        app.start_file_job(Action::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        let start = std::time::Instant::now();
        while app.file_job.is_some() {
            app.poll();
            assert!(start.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            app.doc().path.as_ref(),
            Some(&std::fs::canonicalize(&to).unwrap())
        );
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        app.doc_mut().save().unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(to).unwrap(), "unsaved original");
    }

    #[test]
    fn disk_visible_save_refuses_rename_until_actual_finished_receipt() {
        use crate::save_worker::{GatePoint, Worker};
        use std::{
            sync::mpsc::TryRecvError,
            time::{Duration, Instant},
        };
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let from = root.join("source.txt");
        let to = root.join("renamed.txt");
        std::fs::write(&from, "").unwrap();
        let mut app = App::new(root, Profile::Linux);
        app.extension_node = "vscli-rename-proof-node-missing".into();
        app.open(&from).unwrap();
        app.doc_mut().insert("猫🙂 contents\r\n", false);
        let bytes = app.doc().text.to_string();
        let identity = (
            app.doc().id,
            app.doc().text_epoch(),
            app.doc().revision,
            app.doc().selections(),
            app.doc().save_generation(),
        );
        let member = app.editor_groups.active_membership().unwrap();
        let (worker, entered, release) = Worker::fixture_gated(vec![GatePoint::BeforeFinish]);
        app.replace_save_worker_fixture(worker);
        app.execute("workbench.action.files.save", Value::Null);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll();
            match entered.try_recv() {
                Ok(point) => {
                    assert_eq!(point, GatePoint::BeforeFinish);
                    break;
                }
                Err(TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "{}", app.message);
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("Save fixture gate disconnected: {error}"),
            }
        }
        assert_eq!(std::fs::read(&from).unwrap(), bytes.as_bytes());
        assert!(app.saves_pending());
        assert!(app.doc().dirty());
        assert_eq!(app.doc().save_generation(), identity.4);
        app.start_file_job(Action::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        assert!(app.file_job.is_none());
        assert!(!to.exists());
        assert_eq!(
            app.message,
            "Wait for pending saves before changing file paths"
        );
        assert_eq!(
            identity,
            (
                app.doc().id,
                app.doc().text_epoch(),
                app.doc().revision,
                app.doc().selections(),
                app.doc().save_generation()
            )
        );
        assert_eq!(app.editor_groups.active_membership(), Some(member));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.saves_pending() {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.message.contains("Saved source.txt"), "{}", app.message);
        assert!(!app.doc().dirty());
        assert_eq!(app.doc().save_generation(), identity.4 + 1);
        app.start_file_job(Action::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        assert!(app.file_job.is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.file_job.is_some() {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), bytes.as_bytes());
        assert!(app.message.contains("Renamed to"), "{}", app.message);
        assert_eq!(app.doc().id, identity.0);
        assert_eq!(app.doc().path.as_ref(), Some(&to));
        assert_eq!(app.editor_groups.active_membership(), Some(member));
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "");
        assert!(app.doc().dirty());
        assert_eq!(std::fs::read(&to).unwrap(), bytes.as_bytes());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), bytes);
        assert!(!app.doc().dirty());
    }
}
