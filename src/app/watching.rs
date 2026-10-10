use super::*;
use crate::watch::{CONTENT, DiskChange, DiskJob, INDEX, Notice, ReadRequest, SaveConflictProof};
use std::time::{Duration, Instant};

impl App {
    /// The save worker already observed this exact disk conflict. Retain its
    /// proof so a later duplicate watcher notice cannot erase the more
    /// actionable save failure. Other models and newer epochs remain separate.
    pub(super) fn retain_save_conflict_notice(
        &mut self,
        snapshot: &crate::document::SaveSnapshot,
        error: &str,
    ) {
        if !error.starts_with("File changed on disk") {
            return;
        }
        let Some(path) = snapshot
            .source_path()
            .filter(|path| *path == snapshot.target())
        else {
            return;
        };
        if let Some(doc) = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| {
                doc.id == snapshot.document_id()
                    && doc.path.as_deref() == snapshot.source_path()
                    && doc.revision == snapshot.revision()
                    && doc.saved_revision == snapshot.saved_revision()
                    && doc.text_epoch() == snapshot.text_epoch()
                    && doc.save_generation() == snapshot.save_generation()
            })
        {
            self.watch.notices.insert(
                doc.id,
                Notice {
                    text: format!("Save failed; unsaved work retained: {error}"),
                    save_conflict: Some(SaveConflictProof {
                        path: path.to_owned(),
                        text_epoch: doc.text_epoch(),
                        save_generation: doc.save_generation(),
                    }),
                },
            );
        }
    }
    /// Fence earlier disk snapshots without abandoning their actual worker.
    /// The next read uses the latest model proofs after that worker settles.
    pub(super) fn invalidate_disk_watch_publications(&mut self) -> Result<()> {
        if self.watch.publication_disabled {
            anyhow::bail!(
                "Disk refresh is disabled after publication generation exhaustion; restart the editor"
            );
        }
        let Some(next) = self.watch.publication_epoch.checked_add(1) else {
            self.watch.publication_disabled = true;
            self.message = "Disk refresh disabled: publication generation exhausted; buffers retained, restart the editor".into();
            anyhow::bail!("Disk refresh publication generation exhausted; restart the editor");
        };
        self.watch.publication_epoch = next;
        self.watch.pending |= CONTENT;
        // An explicit persistence boundary requests refresh immediately; native
        // notification debounce remains unchanged for unrelated changes.
        self.watch.last_event = Instant::now() - Duration::from_millis(300);
        Ok(())
    }
    pub(super) fn poll_watching(&mut self) -> bool {
        let paths: Vec<_> = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter_map(|d| d.path.clone())
            .collect();
        if paths != self.watch.paths && self.watch.monitor.set_paths(paths.clone()) {
            self.watch.paths = paths;
        }
        let (flags, error) = self.watch.monitor.poll();
        if flags != 0 {
            self.watch.pending |= flags;
            self.watch.last_event = Instant::now();
        }
        let mut changed = false;
        if let Some(error) = error {
            self.message =
                format!("File watcher: {error}; open files are also checked periodically");
            changed = true;
        }
        let settled = self.watch.last_event.elapsed() >= Duration::from_millis(300);
        if self.watch.pending & INDEX != 0
            && !self.workspace.indexing
            && (settled || self.watch.last_refresh.elapsed() >= Duration::from_secs(2))
        {
            let selected = self
                .entries
                .get(self.explorer_selected)
                .map(|e| e.path.clone());
            self.entries = directory_entries(&self.explorer_dir);
            self.explorer_selected = selected
                .and_then(|p| self.entries.iter().position(|e| e.path == p))
                .unwrap_or(
                    self.explorer_selected
                        .min(self.entries.len().saturating_sub(1)),
                );
            self.workspace.refresh();
            self.watch.pending &= !INDEX;
            self.watch.last_refresh = Instant::now();
            changed = true;
        }
        if !self.watch.publication_disabled
            && self.file_job.is_none()
            && self.watch.disk.is_none()
            && ((self.watch.pending & CONTENT != 0 && settled)
                || self.watch.last_read.elapsed() >= Duration::from_secs(2))
        {
            let documents = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .filter_map(|d| {
                    d.path.clone().map(|path| ReadRequest {
                        id: d.id,
                        revision: d.revision,
                        saved_revision: d.saved_revision,
                        text_epoch: d.text_epoch(),
                        save_generation: d.save_generation(),
                        publication_epoch: self.watch.publication_epoch,
                        path,
                        baseline: d.disk_content.clone(),
                    })
                })
                .collect();
            self.watch.disk = Some(DiskJob::start(documents));
            self.watch.pending &= !CONTENT;
            self.watch.last_read = Instant::now();
        }
        if self.file_job.is_some() {
            return changed;
        }
        for _ in 0..4 {
            let result = match self.watch.disk.as_mut().map(DiskJob::poll) {
                Some(Ok(Some(snapshot))) => snapshot,
                Some(Err(_)) => {
                    self.watch.disk = None;
                    break;
                }
                _ => break,
            };
            if self.watch.publication_disabled
                || result.publication_epoch != self.watch.publication_epoch
            {
                self.watch.pending |= CONTENT;
                continue;
            }
            if self.document_save_pending(result.id) {
                self.watch.pending |= CONTENT;
                continue;
            }
            let Some(doc) = self
                .documents
                .iter_mut()
                .chain(&mut self.hidden_documents)
                .find(|d| d.id == result.id && d.path.as_ref() == Some(&result.path))
            else {
                continue;
            };
            if doc.revision != result.revision
                || doc.saved_revision != result.saved_revision
                || doc.text_epoch() != result.text_epoch
                || doc.save_generation() != result.save_generation
            {
                self.watch.pending |= CONTENT;
                continue;
            }
            let preserve_save_failure = self.watch.notices.get(&doc.id).is_some_and(|notice| {
                notice
                    .save_conflict
                    .as_ref()
                    .is_some_and(|proof| proof.current(doc))
            });
            let (notice, conflict) = match result.content {
                Ok(DiskChange::Unchanged) => {
                    self.watch.notices.remove(&doc.id);
                    continue;
                }
                Ok(DiskChange::Changed(Some(content))) if !doc.dirty() => {
                    doc.reload_content(content);
                    self.watch.notices.remove(&doc.id);
                    self.message = format!(
                        "Reloaded {} after external change · Undo restores previous content",
                        doc.name()
                    );
                    changed = true;
                    continue;
                }
                Ok(DiskChange::Changed(Some(_))) => (
                    format!(
                        "{} changed on disk; unsaved edits retained. Save As or compare before reloading",
                        doc.name()
                    ),
                    true,
                ),
                Ok(DiskChange::Changed(None)) => {
                    changed |= doc.saved_revision != u64::MAX;
                    doc.saved_revision = u64::MAX;
                    (
                        format!(
                            "{} was removed from disk; buffer retained. Save As to preserve it",
                            doc.name()
                        ),
                        true,
                    )
                }
                Err(error) => (
                    format!("Cannot refresh {}: {error}; buffer retained", doc.name()),
                    false,
                ),
            };
            if conflict && preserve_save_failure {
                continue;
            }
            if self
                .watch
                .notices
                .get(&doc.id)
                .map(|existing| &existing.text)
                != Some(&notice)
            {
                self.watch.notices.insert(
                    doc.id,
                    Notice {
                        text: notice.clone(),
                        save_conflict: None,
                    },
                );
                self.message = notice;
                changed = true;
            }
        }
        self.watch.notices.retain(|id, _| {
            self.documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|d| d.id == *id)
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
        let start = Instant::now();
        loop {
            app.poll();
            if predicate(app) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(6), "{}", app.message);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn later_identical_disk_conflict_preserves_actual_save_failure_and_model_history() {
        for deleted in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("source.cpp");
            let original = "猫🙂 original\r\n";
            let foreign = "FOREIGN猫🙂\r\n";
            std::fs::write(&path, original).unwrap();
            let mut app = App::new(root.path().into(), Profile::Linux);
            app.open(&path).unwrap();
            let id = app.doc().id;
            app.doc_mut().insert("DIRTY", false);
            let text = app.doc().text.clone();
            let selections = app.doc().selections();
            let epoch = app.doc().text_epoch();
            if deleted {
                std::fs::remove_file(&path).unwrap();
            } else {
                std::fs::write(&path, foreign).unwrap();
            }
            app.request_native_save(None).unwrap();
            let deadline = Instant::now() + Duration::from_secs(8);
            while app.saves_pending() {
                app.poll_native_saves();
                assert!(Instant::now() < deadline, "{}", app.message);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(app.message.starts_with("Save failed"), "{}", app.message);
            assert!(app.message.contains("changed on disk"));
            let failure = app.message.clone();
            for _ in 0..2 {
                let release = hold_sampled_disk_reply(&mut app);
                release.send(()).unwrap();
                drain_retained_job(&mut app);
                assert_eq!(app.message, failure);
            }
            assert_eq!(app.doc().id, id);
            assert_eq!(app.doc().text, text);
            assert_eq!(app.doc().selections(), selections);
            assert_eq!(app.doc().text_epoch(), epoch);
            assert!(app.doc().dirty());
            if deleted {
                assert!(!path.exists());
            } else {
                assert_eq!(std::fs::read(&path).unwrap(), foreign.as_bytes());
            }
            app.doc_mut().undo();
            assert_eq!(app.doc().text.to_string(), original);
            app.doc_mut().redo();
            assert_eq!(app.doc().text, text);
            if deleted {
                assert!(!path.exists());
            } else {
                assert_eq!(std::fs::read(&path).unwrap(), foreign.as_bytes());
            }
        }
    }

    #[test]
    fn newer_edit_and_another_document_receive_fresh_disk_conflict_notices() {
        let (_directory, mut app, path) = clean_fixture("original 猫🙂\r\n");
        app.doc_mut().insert("DIRTY", false);
        let snapshot = app.doc().capture_save(path.clone()).unwrap();
        let error = "File changed on disk; existing bytes preserved";
        app.retain_save_conflict_notice(&snapshot, error);
        app.message = format!("Save failed; unsaved work retained: {error}");
        let failure = app.message.clone();
        std::fs::write(&path, "foreign 猫🙂\r\n").unwrap();
        app.doc_mut().insert("NEW", false);
        let expected = app.doc().text.clone();
        let release = hold_sampled_disk_reply(&mut app);
        release.send(()).unwrap();
        drain_retained_job(&mut app);
        assert_ne!(app.message, failure);
        assert!(
            app.message
                .contains("changed on disk; unsaved edits retained")
        );
        assert_eq!(app.doc().text, expected);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "foreign 猫🙂\r\n");

        let (_other_directory, mut app, path) = clean_fixture("A 猫🙂\r\n");
        app.doc_mut().insert("DIRTY A", false);
        let snapshot = app.doc().capture_save(path.clone()).unwrap();
        app.retain_save_conflict_notice(&snapshot, error);
        let a_id = app.doc().id;
        let a_text = app.doc().text.clone();
        app.message = failure.clone();
        let other = path.with_file_name("other.cpp");
        std::fs::write(&other, "B 猫🙂\r\n").unwrap();
        app.documents.push(Document::open(&other).unwrap());
        app.active = 1;
        app.doc_mut().insert("DIRTY B", false);
        let b_text = app.doc().text.clone();
        std::fs::write(&other, "foreign B\r\n").unwrap();
        let release = hold_sampled_disk_reply(&mut app);
        release.send(()).unwrap();
        drain_retained_job(&mut app);
        assert_ne!(app.message, failure);
        assert!(app.message.starts_with("other.cpp changed on disk"));
        assert_eq!(app.doc().text, b_text);
        assert_eq!(app.documents[0].id, a_id);
        assert_eq!(app.documents[0].text, a_text);
        assert_eq!(std::fs::read_to_string(other).unwrap(), "foreign B\r\n");
    }
    #[test]
    fn external_reload_undo_dirty_conflict_and_live_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "original").unwrap();
        let mut app = App::new(dir.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        until(&mut app, |a| !a.workspace.indexing);
        let id = app.doc().id;
        std::fs::write(&path, "external").unwrap();
        until(&mut app, |a| a.doc().text == "external");
        assert!(!app.doc().dirty());
        assert_eq!(app.doc().id, id);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "original");
        assert!(app.doc().dirty());
        std::fs::write(&path, "new external").unwrap();
        until(&mut app, |a| a.message.contains("unsaved edits retained"));
        assert_eq!(app.doc().text.to_string(), "original");
        assert!(app.doc_mut().save().is_err());
        let added = dir.path().join("added.txt");
        std::fs::write(&added, "new").unwrap();
        until(&mut app, |a| a.workspace.files.contains(&added));
        std::fs::remove_file(&added).unwrap();
        until(&mut app, |a| {
            !a.workspace.indexing && !a.workspace.files.contains(&added)
        });
    }

    #[test]
    fn deleted_clean_buffer_is_retained_dirty_and_revert_preserves_views() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.txt");
        std::fs::write(&path, "preserve me").unwrap();
        let mut app = App::new(dir.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.execute("workbench.action.splitEditor", serde_json::Value::Null);
        let id = app.doc().id;
        std::fs::remove_file(&path).unwrap();
        until(&mut app, |a| a.doc().dirty());
        assert_eq!(app.doc().text.to_string(), "preserve me");
        assert!(app.doc_mut().save().is_err());
        std::fs::write(&path, "restored").unwrap();
        app.modal = Some(Modal::Revert);
        app.event(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.doc().id, id);
        assert!(app.panes.iter().all(|p| p.document == id));
        assert_eq!(app.doc().text.to_string(), "restored");
        assert!(!app.doc().dirty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "preserve me");
        assert!(app.doc().dirty());
    }
    fn clean_fixture(text: &str) -> (tempfile::TempDir, App, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, text).unwrap();
        let mut app = App::new(root, Profile::Linux);
        app.documents.push(Document::open(&path).unwrap());
        (directory, app, path)
    }
    fn hold_sampled_disk_reply(app: &mut App) -> std::sync::mpsc::Sender<()> {
        let doc = app.doc();
        let request = ReadRequest {
            id: doc.id,
            revision: doc.revision,
            saved_revision: doc.saved_revision,
            text_epoch: doc.text_epoch(),
            save_generation: doc.save_generation(),
            publication_epoch: app.watch.publication_epoch,
            path: doc.path.clone().unwrap(),
            baseline: doc.disk_content.clone(),
        };
        let (sampled_tx, sampled) = std::sync::mpsc::sync_channel(1);
        let (release, gate) = std::sync::mpsc::channel();
        app.watch.disk = Some(DiskJob::start_with_read(vec![request], move |path, _| {
            let content = crate::document::read_disk(path)?;
            sampled_tx.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(DiskChange::Changed(content))
        }));
        app.watch.last_read = Instant::now();
        sampled.recv_timeout(Duration::from_secs(10)).unwrap();
        release
    }
    fn drain_retained_job(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.watch.disk.is_some() {
            app.poll_watching();
            assert!(
                Instant::now() < deadline,
                "retained disk job did not settle"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn held_disk_reply_cannot_revive_revision_after_edit_undo_or_destroy_redo() {
        let original = "original 猫\r\n";
        let (_directory, mut app, path) = clean_fixture(original);
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        let revision = app.doc().revision;
        let save_generation = app.doc().save_generation();
        let selections = app.doc().selections();
        std::fs::write(&path, "external λ\r\n").unwrap();
        let release = hold_sampled_disk_reply(&mut app);
        app.doc_mut().insert("X", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().revision, revision);
        assert!(app.doc().text_epoch() > epoch);
        assert_eq!(app.doc().save_generation(), save_generation);
        assert!(!app.doc().dirty());
        release.send(()).unwrap();
        drain_retained_job(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().selections(), selections);
        assert!(!app.doc().dirty());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), format!("X{original}"));
        app.doc_mut().undo();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "external λ\r\n");
        assert_ne!(app.watch.pending & CONTENT, 0);
    }

    #[test]
    fn held_disk_reply_cannot_publish_after_successful_byte_equal_save() {
        let original = "original 猫\r\n";
        let (_directory, mut app, path) = clean_fixture(original);
        let epoch = app.doc().text_epoch();
        let save_generation = app.doc().save_generation();
        std::fs::write(&path, "external before save\r\n").unwrap();
        let release = hold_sampled_disk_reply(&mut app);
        std::fs::write(&path, original).unwrap();
        app.doc_mut().save().unwrap();
        assert_eq!(app.doc().text_epoch(), epoch);
        assert!(app.doc().save_generation() > save_generation);
        release.send(()).unwrap();
        drain_retained_job(&mut app);
        assert_eq!(app.doc().text.to_string(), original);
        assert!(!app.doc().dirty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
        app.doc_mut().undo();
        assert_eq!(
            app.doc().text.to_string(),
            original,
            "stale reload must not add Undo"
        );
    }

    #[test]
    fn real_settings_commit_fences_held_precommit_snapshot_and_refreshes_once() {
        let original = "{\r\n  \"breadcrumbs.enabled\": true\r\n}\r\n";
        let (_directory, mut app, path) = clean_fixture(original);
        app.configure_settings(Some(path.clone())).unwrap();
        let id = app.doc().id;
        let initial_epoch = app.doc().text_epoch();
        let initial_save = app.doc().save_generation();
        let external = "{ // precommit 猫\r\n  \"breadcrumbs.enabled\": true\r\n}\r\n";
        std::fs::write(&path, external).unwrap();
        let release = hold_sampled_disk_reply(&mut app);
        let old_publication = app.watch.publication_epoch;
        for _ in 0..128 {
            app.invalidate_disk_watch_publications().unwrap();
            app.poll_watching();
        }
        assert!(
            app.watch.disk.is_some(),
            "held actual reader must retain capacity"
        );
        assert_eq!(app.doc().text.to_string(), original);
        app.request_persistent_breadcrumbs(false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll_settings_writes();
            if app.message == "Breadcrumbs setting saved" {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "real settings write did not commit: {}",
                app.message
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.watch.publication_epoch > old_publication + 128);
        let persisted = std::fs::read_to_string(&path).unwrap();
        assert_eq!(persisted, external.replace("true", "false"));
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().text_epoch(), initial_epoch);
        assert_eq!(app.doc().save_generation(), initial_save);
        release.send(()).unwrap();
        drain_retained_job(&mut app);
        assert_eq!(
            app.doc().text.to_string(),
            original,
            "precommit snapshot was applied"
        );
        app.doc_mut().undo();
        assert_eq!(
            app.doc().text.to_string(),
            original,
            "precommit snapshot added Undo"
        );
        until(&mut app, |app| app.doc().text == persisted);
        assert_eq!(app.doc().id, id);
        assert!(!app.doc().dirty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), original);
        assert!(app.doc().dirty());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), persisted);
        assert!(!app.doc().dirty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), persisted);
    }

    #[test]
    fn exhausted_publication_epoch_retires_held_reply_and_disables_future_refresh() {
        let original = "original 猫\r\n";
        let (_directory, mut app, path) = clean_fixture(original);
        std::fs::write(&path, "external λ\r\n").unwrap();
        let release = hold_sampled_disk_reply(&mut app);
        app.watch.publication_epoch = u64::MAX;
        assert!(app.invalidate_disk_watch_publications().is_err());
        assert!(app.watch.publication_disabled);
        assert!(app.message.contains("generation exhausted"));
        assert!(app.watch.disk.is_some());
        release.send(()).unwrap();
        drain_retained_job(&mut app);
        assert_eq!(app.doc().text.to_string(), original);
        app.watch.last_read = Instant::now() - Duration::from_secs(3);
        app.poll_watching();
        assert!(app.watch.disk.is_none());
        assert!(app.invalidate_disk_watch_publications().is_err());
        assert_eq!(app.watch.publication_epoch, u64::MAX);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "external λ\r\n");
    }
}
