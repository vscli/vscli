use super::*;
use crate::watch::{CONTENT, DiskChange, DiskJob, INDEX, ReadRequest};
use std::time::{Duration, Instant};

impl App {
    pub(super) fn poll_watching(&mut self) -> bool {
        let paths: Vec<_> = self
            .documents
            .iter()
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
        if self.file_job.is_none()
            && self.watch.disk.is_none()
            && ((self.watch.pending & CONTENT != 0 && settled)
                || self.watch.last_read.elapsed() >= Duration::from_secs(2))
        {
            let documents = self
                .documents
                .iter()
                .filter_map(|d| {
                    d.path.clone().map(|path| ReadRequest {
                        id: d.id,
                        revision: d.revision,
                        saved_revision: d.saved_revision,
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
            let result = match self.watch.disk.as_ref().map(DiskJob::poll) {
                Some(Ok(Some(snapshot))) => snapshot,
                Some(Err(_)) => {
                    self.watch.disk = None;
                    break;
                }
                _ => break,
            };
            let Some(doc) = self
                .documents
                .iter_mut()
                .find(|d| d.id == result.id && d.path.as_ref() == Some(&result.path))
            else {
                continue;
            };
            if doc.revision != result.revision || doc.saved_revision != result.saved_revision {
                self.watch.pending |= CONTENT;
                continue;
            }
            let notice = match result.content {
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
                Ok(DiskChange::Changed(Some(_))) => format!(
                    "{} changed on disk; unsaved edits retained. Save As or compare before reloading",
                    doc.name()
                ),
                Ok(DiskChange::Changed(None)) => {
                    doc.saved_revision = u64::MAX;
                    format!(
                        "{} was removed from disk; buffer retained. Save As to preserve it",
                        doc.name()
                    )
                }
                Err(error) => format!("Cannot refresh {}: {error}; buffer retained", doc.name()),
            };
            if self.watch.notices.get(&doc.id) != Some(&notice) {
                self.watch.notices.insert(doc.id, notice.clone());
                self.message = notice;
                changed = true;
            }
        }
        self.watch
            .notices
            .retain(|id, _| self.documents.iter().any(|d| d.id == *id));
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
}
