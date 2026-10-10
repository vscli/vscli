use super::*;
use crate::editor_groups::{RestoreGroup, UiProof};
use crate::session::{
    Event as SessionEvent, Group as SavedGroup, Layout, Pane as SavedPane, SavedFile,
    Tab as SavedTab, View, Worker,
};
use anyhow::Context as _;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
struct ModelContext {
    id: u64,
    revision: u64,
    text_epoch: u64,
    save_generation: u64,
    path: Option<PathBuf>,
    selections: Vec<crate::document::Selection>,
}
#[derive(PartialEq)]
struct Context {
    epoch: u64,
    workspace: PathBuf,
    documents: Vec<ModelContext>,
    groups: UiProof,
    fallback: bool,
    panes: Vec<(u64, u64)>,
    active: usize,
    active_pane: usize,
    focus: Focus,
}
pub(super) struct State {
    worker: Option<Worker>,
    ready: bool,
    available: bool,
    automatic: Option<u64>,
    pending: Option<Context>,
    epoch: u64,
    protected: bool,
    explicit_empty: bool,
    captured: Instant,
}
impl Default for State {
    fn default() -> Self {
        Self {
            worker: None,
            ready: false,
            available: false,
            automatic: None,
            pending: None,
            epoch: 0,
            protected: false,
            explicit_empty: false,
            captured: Instant::now(),
        }
    }
}
impl App {
    /// Called after CLI files and recovery have been installed. No filesystem I/O here.
    pub fn configure_session(&mut self, config: Option<&Path>, restore: bool) -> Result<()> {
        if self.session.worker.is_some() {
            anyhow::bail!("Session worker is already configured");
        }
        let Some(config) = config else {
            return Ok(());
        };
        self.sync_pane();
        self.session.worker = Some(Worker::start(
            config.to_owned(),
            self.workspace.root.clone(),
        )?);
        self.session.automatic = restore.then_some(self.session.epoch);
        self.session.protected = restore;
        Ok(())
    }
    pub(super) fn session_interaction(&mut self) {
        self.extension_services.epoch = self.extension_services.epoch.wrapping_add(1);
        self.session.epoch = self.session.epoch.wrapping_add(1);
    }
    fn session_context(&self) -> Context {
        Context {
            epoch: self.session.epoch,
            workspace: self.workspace.root.clone(),
            documents: self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .map(|d| ModelContext {
                    id: d.id,
                    revision: d.revision,
                    text_epoch: d.text_epoch(),
                    save_generation: d.save_generation(),
                    path: d.path.clone(),
                    selections: d.selections(),
                })
                .collect(),
            groups: self.editor_groups.proof(),
            fallback: self.group_fallback,
            panes: self.panes.iter().map(|p| (p.id, p.document)).collect(),
            active: self.active,
            active_pane: self.active_pane,
            focus: self.focus.clone(),
        }
    }
    pub(super) fn restore_session(&mut self) {
        if !self.documents.is_empty() {
            self.message = "Close editors before restoring the previous clean-file session".into();
            return;
        }
        self.begin_session_restore();
    }
    fn begin_session_restore(&mut self) {
        if self.session.worker.is_none() {
            self.message = "Session storage is disabled or unavailable".into();
            return;
        }
        if !self.session.ready {
            self.message = "Session metadata is still loading; retry shortly".into();
            return;
        }
        if !self.session.available {
            self.message = "No previous clean-file session".into();
            return;
        }
        if self.session.pending.is_some() {
            self.message = "Session restore is still loading; retry shortly".into();
            return;
        }
        if self.documents.len() + self.hidden_documents.len()
            > crate::editor_groups::MAX_MEMBERSHIPS
            || self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|doc| doc.secondary.len() >= crate::session::MAX_SELECTIONS)
        {
            self.message =
                "Too many existing buffers/selections for guarded session restore".into();
            return;
        }
        let mut model_ids = std::collections::HashSet::new();
        let mut path_bytes = 0usize;
        if self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .any(|doc| {
                if !model_ids.insert(doc.id) {
                    return true;
                }
                let Some(path) = &doc.path else {
                    return false;
                };
                let bytes = path.as_os_str().as_encoded_bytes();
                path_bytes = path_bytes.saturating_add(bytes.len());
                bytes.len() > 4096 || bytes.contains(&0) || path_bytes > 512 * 1024
            })
        {
            self.message = "Session restore proof exceeds distinct-model/path bounds; all buffers and previous metadata retained".into();
            return;
        }
        let context = self.session_context();
        let skip = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter_map(|doc| doc.path.clone())
            .collect();
        let Some(worker) = &mut self.session.worker else {
            self.message = "Session storage is disabled or unavailable".into();
            return;
        };
        match worker.restore(skip) {
            Ok(()) => {
                self.session.pending = Some(context);
                self.session.protected = true;
                self.message = "Restoring clean-file session…".into();
            }
            Err(error) => self.message = format!("Session restore unavailable: {error:#}"),
        }
    }
    fn capture_session(&self) -> Result<Layout> {
        if self.group_fallback {
            return self.capture_legacy_session();
        }
        let mut layout = Layout {
            groups: Some(Vec::new()),
            horizontal: self.horizontal_split,
            ..Layout::default()
        };
        let mut ids = Vec::new();
        let mut paths = std::collections::HashSet::new();
        let mut groups = Vec::new();
        for group in self.editor_groups.groups() {
            let mut tabs = Vec::new();
            let mut tab_ids = Vec::new();
            for tab in group.tabs() {
                let doc = self
                    .documents
                    .iter()
                    .find(|doc| doc.id == tab.document())
                    .ok_or_else(|| anyhow::anyhow!("Session tab model is not retained"))?;
                if doc.dirty() {
                    continue;
                }
                let Some(path) = &doc.path else {
                    continue;
                };
                if path.as_os_str().len() > 4096 {
                    anyhow::bail!("Session path exceeds 4 KiB");
                }
                let view = View::capture(doc, doc.view_state(Some(group.id().value())))?;
                let file = if let Some(file) = ids.iter().position(|id| *id == doc.id) {
                    file
                } else {
                    if ids.len() >= crate::session::MAX_DOCUMENTS || !paths.insert(path.clone()) {
                        anyhow::bail!("Session exceeds 32 unique clean file paths");
                    }
                    ids.push(doc.id);
                    layout.files.push(SavedFile {
                        path: path.clone(),
                        view: view.clone(),
                    });
                    ids.len() - 1
                };
                tabs.push(SavedTab { file, view });
                tab_ids.push(tab.id());
            }
            if tabs.is_empty() {
                continue;
            }
            let recent = group
                .recent()
                .iter()
                .filter_map(|id| tab_ids.iter().position(|tab| tab == id))
                .collect::<Vec<_>>();
            let active = group
                .active()
                .and_then(|tab| tab_ids.iter().position(|id| *id == tab.id()))
                .or_else(|| recent.first().copied())
                .ok_or_else(|| anyhow::anyhow!("Session clean tab has no recent order"))?;
            if self.editor_groups.active_group() == Some(group.id()) {
                layout.active_group = groups.len();
            }
            layout.panes.push(SavedPane {
                file: tabs[active].file,
                view: tabs[active].view.clone(),
            });
            groups.push(SavedGroup {
                tabs,
                active,
                recent,
            });
        }
        layout.active_pane = layout.active_group;
        layout.active_file = layout
            .panes
            .get(layout.active_pane)
            .map_or(0, |pane| pane.file);
        layout.groups = Some(groups);
        layout.validate()?;
        Ok(layout)
    }
    fn capture_legacy_session(&self) -> Result<Layout> {
        let mut layout = Layout::default();
        let mut ids = Vec::new();
        for doc in &self.documents {
            if doc.dirty() {
                continue;
            }
            if let Some(path) = &doc.path {
                if layout.files.len() >= crate::session::MAX_DOCUMENTS
                    || path.as_os_str().len() > 4096
                {
                    anyhow::bail!("Session exceeds 32 files or path byte budget");
                }
                layout.files.push(SavedFile {
                    path: path.clone(),
                    view: View::capture(doc, doc.view_state(None))?,
                });
                ids.push(doc.id);
            }
        }
        for (index, pane) in self.panes.iter().enumerate() {
            if let Some(file) = ids.iter().position(|id| *id == pane.document) {
                let doc = self
                    .documents
                    .iter()
                    .find(|d| d.id == pane.document)
                    .unwrap();
                if index == self.active_pane {
                    layout.active_pane = layout.panes.len();
                }
                layout.panes.push(SavedPane {
                    file,
                    view: View::capture(doc, doc.view_state(Some(pane.id)))?,
                });
            }
        }
        if !layout.files.is_empty() && layout.panes.is_empty() {
            layout.panes.push(SavedPane {
                file: 0,
                view: layout.files[0].view.clone(),
            });
        }
        layout.active_file = layout.panes.get(layout.active_pane).map_or(0, |p| p.file);
        layout.horizontal = self.horizontal_split;
        layout.validate()?;
        layout.normalized()
    }
    fn session_snapshot(&self) -> Result<Option<Layout>> {
        if self.session.protected
            || self.session.pending.is_some()
            || self.session.automatic.is_some()
        {
            return Ok(None);
        }
        let layout = self.capture_session()?;
        Ok((!layout.files.is_empty() || self.session.explicit_empty).then_some(layout))
    }
    fn install_session(&mut self, restored: crate::session::Restored) -> Result<()> {
        self.install_session_with(restored, |app, doc| {
            app.settings.apply(doc);
            app.configure_document_language(doc)
        })
    }
    fn install_session_with(
        &mut self,
        restored: crate::session::Restored,
        mut configure: impl FnMut(&Self, &mut Document) -> Result<()>,
    ) -> Result<()> {
        // Configure and import the complete batch before changing any live model,
        // membership, view, focus, or checked engine identity counter.
        let layout = restored.layout.normalized()?;
        let had_existing = !self.documents.is_empty() || !self.hidden_documents.is_empty();
        let mut staged = Vec::new();
        let mut incoming_ids = std::collections::HashSet::new();
        for mut doc in restored.documents {
            if !incoming_ids.insert(doc.id)
                || self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .any(|old| old.id == doc.id)
            {
                anyhow::bail!("Duplicate restored document identity");
            }
            if self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|old| old.path == doc.path)
            {
                continue;
            }
            if !layout
                .files
                .iter()
                .any(|file| doc.path.as_ref() == Some(&file.path))
            {
                anyhow::bail!("Restored document is absent from the session file table");
            }
            configure(self, &mut doc)?;
            staged.push(doc);
        }
        let mut file_ids = Vec::new();
        for file in &layout.files {
            let id = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .chain(&staged)
                .find(|doc| doc.path.as_ref() == Some(&file.path))
                .map(|doc| doc.id)
                .ok_or_else(|| anyhow::anyhow!("A restored session file is no longer available"))?;
            file_ids.push(id);
        }
        let combined = self.documents.len() + self.hidden_documents.len() + staged.len();
        if combined > crate::editor_groups::MAX_MEMBERSHIPS {
            anyhow::bail!(
                "Session restore exceeds the retained model proof limit; recovery retained"
            );
        }
        let unassigned = self
            .documents
            .iter()
            .filter(|doc| self.editor_groups.memberships(doc.id).next().is_none())
            .map(|doc| doc.id)
            .collect::<Vec<_>>();
        let occupied = self
            .editor_groups
            .active_group()
            .and_then(|id| self.editor_groups.group(id))
            .map_or(0, |group| group.tabs().len());
        // Existing memberships may legitimately span all four groups. Only
        // models that still need admission consume the active group's slots.
        if had_existing
            && (self.group_fallback
                || unassigned.len() + staged.len()
                    > crate::editor_groups::MAX_TABS_PER_GROUP.saturating_sub(occupied))
        {
            self.documents.extend(staged);
            self.group_fallback = true;
            self.sync_pane();
            self.session.protected = false;
            self.session.explicit_empty = self.documents.is_empty();
            self.message = "Clean session files appended; editor-group layout unavailable beyond the active group's 128-tab limit; all recovery retained".into();
            return Ok(());
        }
        self.documents
            .try_reserve(staged.len())
            .context("Cannot reserve retained session models; all buffers retained")?;
        let mut engine = self.editor_groups.clone();
        if had_existing {
            let mut change = crate::editor_groups::Change {
                previous: engine.active_membership(),
                ..crate::editor_groups::Change::default()
            };
            let active = engine.active_membership();
            for id in unassigned
                .iter()
                .copied()
                .chain(staged.iter().map(|doc| doc.id))
            {
                let admitted = engine.open(id)?;
                change.changed |= admitted.changed;
                change.inserted.extend(admitted.inserted);
                change.created_groups.extend(admitted.created_groups);
                change.promoted.extend(admitted.promoted);
            }
            if let Some(active) = active {
                engine.focus(active)?;
            }
            // No session-saved selections are applied to authoritative recovery,
            // and existing group order/MRU/view payloads are retained.
            change.active = engine.active_membership();
            let layout = self.prepare_group_layout(&engine, &change, None)?;
            self.documents.extend(staged);
            self.publish_group_layout(engine, layout);
            self.project_editor_groups();
        } else {
            let saved_groups = layout.groups.as_ref().expect("normalized groups");
            let imported = saved_groups
                .iter()
                .map(|group| RestoreGroup {
                    documents: group.tabs.iter().map(|tab| file_ids[tab.file]).collect(),
                    active: group.active,
                    recent: group.recent.clone(),
                })
                .collect::<Vec<_>>();
            engine.import(&imported, layout.active_group)?;
            let editor_layout = self.prepare_import_group_layout(
                &engine,
                None,
                if layout.horizontal {
                    crate::editor_layout::Axis::Rows
                } else {
                    crate::editor_layout::Axis::Columns
                },
            )?;
            for (group, saved) in engine.groups().iter().zip(saved_groups) {
                for tab in &saved.tabs {
                    let doc = staged
                        .iter_mut()
                        .find(|doc| doc.id == file_ids[tab.file])
                        .expect("complete file table checked");
                    doc.activate_view(group.id().value());
                    tab.view.apply(doc);
                }
            }
            self.documents = staged;
            self.publish_group_layout(engine, editor_layout);
            self.group_fallback = false;
            self.horizontal_split = layout.horizontal;
            self.project_editor_groups();
        }
        self.session.protected = false;
        self.session.explicit_empty = self.documents.is_empty();
        self.focus = if self.documents.is_empty() {
            self.focus.clone()
        } else {
            Focus::Editor
        };
        self.message = format!(
            "Restored clean-file session · {} file(s) · existing/recovered buffers retained",
            layout.files.len()
        );
        Ok(())
    }
    pub(super) fn poll_session(&mut self) -> bool {
        let event = self.session.worker.as_mut().and_then(Worker::poll);
        let changed = event.is_some();
        match event {
            Some(SessionEvent::Ready(available)) => {
                self.session.ready = true;
                self.session.available = available;
                if !available {
                    self.session.protected = false;
                }
                if let Some(epoch) = self.session.automatic.take() {
                    if !available {
                        self.message = "No previous clean-file session".into();
                    } else if epoch == self.session.epoch
                        && self.prompt.is_none()
                        && self.modal.is_none()
                    {
                        self.begin_session_restore();
                    } else {
                        self.message = "Session restore cancelled by newer interaction; saved session retained".into();
                    }
                }
            }
            Some(SessionEvent::Restored(restored)) => {
                if let Some(context) = self.session.pending.take() {
                    if context == self.session_context()
                        && self.prompt.is_none()
                        && self.modal.is_none()
                    {
                        if let Err(error) = self.install_session(restored) {
                            self.message = format!(
                                "Session restore failed; previous session retained: {error:#}"
                            );
                        }
                    } else {
                        self.message = "Session restore cancelled by newer interaction; saved session retained".into();
                    }
                }
            }
            Some(SessionEvent::Failed(error)) => {
                if !self.session.ready {
                    self.session.worker = None;
                    self.session.automatic = None;
                    self.session.protected = true;
                }
                self.session.pending = None;
                self.message =
                    format!("Session state unavailable; previous metadata retained: {error}");
            }
            _ => {}
        }
        if self.session.ready
            && self.session.pending.is_none()
            && self.session.captured.elapsed() >= Duration::from_secs(2)
        {
            self.session.captured = Instant::now();
            let snapshot = self.session_snapshot();
            if let Some(worker) = &mut self.session.worker {
                let result = match snapshot {
                    Ok(Some(layout)) => worker.save(layout),
                    Ok(None) => worker.flush(),
                    Err(e) => Err(e),
                };
                if let Err(error) = result {
                    self.message = format!("Session state not saved: {error:#}");
                    return true;
                }
            }
        }
        changed
    }
    pub fn finish_session(&mut self) -> Result<()> {
        let Some(worker) = self.session.worker.take() else {
            return Ok(());
        };
        worker.finish(self.session_snapshot()?)
    }
    pub(super) fn session_closed_all(&mut self) {
        self.session.explicit_empty = true;
        self.session.protected = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Selection;
    use std::fs;
    fn settle(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            app.poll_session();
            if app.session.ready && app.session.automatic.is_none() && app.session.pending.is_none()
            {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn configured(root: &Path, config: &Path, restore: bool) -> App {
        let mut app = App::new(root.into(), Profile::Linux);
        app.configure_session(Some(config), restore).unwrap();
        settle(&mut app);
        app
    }
    fn seed(root: &Path, config: &Path, path: &Path) {
        let mut app = configured(root, config, false);
        app.open(path).unwrap();
        app.doc_mut().move_to(2, false);
        app.finish_session().unwrap();
    }
    #[test]
    fn restart_restores_global_tabs_shared_panes_and_independent_unicode_crlf_selections() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let first = root.path().join("first.cpp");
        let second = root.path().join("second.cpp");
        let content = "猫 🦀 abc\r\nsecond line\r\n";
        fs::write(&first, content).unwrap();
        fs::write(&second, "other\r\n").unwrap();
        let expected;
        {
            let mut app = configured(root.path(), &config, false);
            app.open(&first).unwrap();
            app.open(&second).unwrap();
            app.open(&first).unwrap();
            app.doc_mut().set_selections(vec![
                Selection {
                    cursor: 6,
                    anchor: Some(3),
                    desired_column: None,
                },
                Selection::caret(12),
            ]);
            app.split_editor(true);
            app.doc_mut().set_selections(vec![Selection {
                cursor: 15,
                anchor: Some(10),
                desired_column: None,
            }]);
            app.doc_mut().top = 1;
            app.focus_pane(0);
            expected = app.capture_session().unwrap();
            app.finish_session().unwrap();
        }
        let mut app = configured(root.path(), &config, true);
        assert_eq!(app.capture_session().unwrap(), expected);
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.panes[0].document, app.panes[1].document);
        assert!(app.documents.iter().all(|d| !d.dirty()));
        let id = app.doc().id;
        app.doc_mut().insert("!", false);
        app.focus_pane(1);
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text, content);
        assert!(!app.doc().dirty());
        app.doc_mut().save().unwrap();
        assert_eq!(fs::read(&first).unwrap(), content.as_bytes());
        app.finish_session().unwrap();
    }
    #[test]
    fn recovery_duplicates_are_authoritative_and_do_not_receive_saved_selections() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        let other = root.path().join("other.txt");
        fs::write(&path, "saved disk").unwrap();
        fs::write(&other, "other disk").unwrap();
        let mut original = configured(root.path(), &config, false);
        original.open(&path).unwrap();
        original.open(&other).unwrap();
        original.finish_session().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let mut a = Document::open(&path).unwrap();
        a.insert("recovered A", false);
        let mut b = Document::open(&path).unwrap();
        b.insert("recovered B", false);
        let ids = [a.id, b.id];
        let selections = [a.selections(), b.selections()];
        app.documents = vec![a, b];
        app.active = 1;
        app.sync_pane();
        let pane = app.panes[0].id;
        app.configure_session(Some(&config), true).unwrap();
        settle(&mut app);
        assert_eq!(app.documents.len(), 3);
        assert_eq!(app.active, 1);
        assert_eq!(app.panes[0].id, pane);
        for index in 0..2 {
            assert_eq!(app.documents[index].id, ids[index]);
            assert_eq!(app.documents[index].selections(), selections[index]);
            assert!(app.documents[index].dirty());
        }
        assert_eq!(fs::read_to_string(path).unwrap(), "saved disk");
        app.finish_session().unwrap();
    }
    #[test]
    fn missing_files_are_retryable_and_failed_or_stale_restore_never_publishes_empty_state() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "abcd").unwrap();
        let path = fs::canonicalize(path).unwrap();
        seed(root.path(), &config, &path);
        fs::remove_file(&path).unwrap();
        let mut failed = configured(root.path(), &config, true);
        assert!(failed.documents.is_empty());
        assert!(failed.session.protected);
        assert!(!path.exists());
        failed.finish_session().unwrap();
        fs::write(&path, "abcd").unwrap();
        let mut stale = configured(root.path(), &config, false);
        stale.restore_session();
        assert!(stale.session.pending.is_some());
        stale.execute("workbench.action.files.newUntitledFile", Value::Null);
        stale.doc_mut().insert("newer unsaved work", false);
        settle(&mut stale);
        assert_eq!(stale.documents.len(), 1);
        assert_eq!(stale.doc().text, "newer unsaved work");
        assert!(stale.session.protected);
        stale.finish_session().unwrap();
        let mut retry = configured(root.path(), &config, true);
        assert_eq!(retry.documents.len(), 1);
        assert_eq!(retry.doc().path.as_ref(), Some(&path));
        assert_eq!(retry.doc().cursor, 2);
        retry.finish_session().unwrap();
    }
    #[test]
    fn failed_restore_can_retry_in_place_without_another_worker() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "abcd").unwrap();
        seed(root.path(), &config, &path);
        fs::remove_file(&path).unwrap();
        let mut app = configured(root.path(), &config, true);
        fs::write(&path, "abcd").unwrap();
        app.restore_session();
        app.restore_session();
        settle(&mut app);
        assert_eq!(app.documents.len(), 1);
        assert!(!app.session.protected);
        app.finish_session().unwrap();
    }
    #[test]
    fn discard_cancel_save_as_and_explicit_close_all_do_not_resurrect_text() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        let moved = fs::canonicalize(root.path()).unwrap().join("moved.txt");
        fs::write(&path, "disk").unwrap();
        let path = fs::canonicalize(path).unwrap();
        seed(root.path(), &config, &path);
        let mut app = configured(root.path(), &config, true);
        app.doc_mut().insert("DISCARDED SECRET", false);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        )));
        assert!(app.doc().dirty());
        assert!(
            !app.capture_session()
                .unwrap()
                .files
                .iter()
                .any(|f| f.path == path)
        );
        app.execute("workbench.action.files.saveAs", Value::Null);
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.doc().path.as_ref(), Some(&path));
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert!(app.documents.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "disk");
        app.open(&path).unwrap();
        app.execute("workbench.action.files.saveAs", Value::Null);
        app.prompt.as_mut().unwrap().text = moved.to_string_lossy().into_owned();
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.saves_pending() {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.doc().path.as_ref(), Some(&moved));
        assert_eq!(app.capture_session().unwrap().files[0].path, moved);
        app.execute("workbench.action.closeAllEditors", Value::Null);
        assert!(app.session.explicit_empty);
        app.finish_session().unwrap();
        let mut restored = configured(root.path(), &config, true);
        assert!(restored.documents.is_empty());
        restored.finish_session().unwrap();
        for dir in fs::read_dir(config.join("state/sessions")).unwrap() {
            for file in fs::read_dir(dir.unwrap().path()).unwrap() {
                let path = file.unwrap().path();
                if path.extension().is_some_and(|e| e == "json") {
                    assert!(
                        !fs::read_to_string(path)
                            .unwrap()
                            .contains("DISCARDED SECRET")
                    );
                }
            }
        }
    }
    #[test]
    fn disabled_and_failed_initial_storage_leave_empty_editor_usable() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("vscli.session.restore", Value::Null);
        assert!(app.message.contains("disabled or unavailable"));
        assert!(app.documents.is_empty());
        let config = root.path().join("not-a-directory");
        fs::write(&config, "retain").unwrap();
        app.configure_session(Some(&config), true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while app.session.worker.is_some() {
            app.poll_session();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        app.execute("vscli.session.restore", Value::Null);
        assert!(app.message.contains("disabled or unavailable"));
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("native editing survives", false);
        assert_eq!(app.doc().text, "native editing survives");
        assert_eq!(fs::read_to_string(config).unwrap(), "retain");
        app.finish_session().unwrap();
    }
    #[test]
    fn resize_and_key_release_do_not_cancel_startup_restore() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "abcd").unwrap();
        seed(root.path(), &config, &path);
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_session(Some(&config), true).unwrap();
        app.event(Event::Resize(80, 25));
        app.event(Event::FocusGained);
        app.event(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )));
        settle(&mut app);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().cursor, 2);
        app.finish_session().unwrap();
    }
    #[test]
    fn changed_shorter_unicode_crlf_lines_clamp_selection_without_splitting_eol() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "long first line\r\nsecond\r\n").unwrap();
        let mut app = configured(root.path(), &config, false);
        app.open(&path).unwrap();
        app.doc_mut().set_selections(vec![Selection {
            cursor: 12,
            anchor: Some(7),
            desired_column: None,
        }]);
        app.finish_session().unwrap();
        fs::write(&path, "e\u{301}猫\r\nX\r\n").unwrap();
        let mut app = configured(root.path(), &config, true);
        assert_eq!(app.doc().row(), 0);
        assert_eq!(app.doc().cursor, 3);
        app.doc_mut().insert("!", false);
        app.doc_mut().save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "e\u{301}猫!\r\nX\r\n");
        app.doc_mut().undo();
        assert_eq!(app.doc().text, "e\u{301}猫\r\nX\r\n");
        app.finish_session().unwrap();
    }
    fn grouped_fixture(root: &Path) -> crate::session::Restored {
        use crate::session::{Position, SavedSelection};
        let mut files = Vec::new();
        let mut documents = Vec::new();
        for name in ["a.cpp", "b.cpp"] {
            let path = root.join(name);
            fs::write(&path, "猫🙂 value\r\nnext\r\n").unwrap();
            let doc = Document::open(&path).unwrap();
            files.push(SavedFile {
                path: doc.path.clone().unwrap(),
                view: View::capture(&doc, doc.view_state(None)).unwrap(),
            });
            documents.push(doc);
        }
        let view = |cursor: usize, anchor: Option<usize>| View {
            selections: vec![SavedSelection {
                cursor: Position {
                    line: 0,
                    character: cursor,
                },
                anchor: anchor.map(|character| Position { line: 0, character }),
            }],
            top: 0,
            left: 0,
        };
        let groups = vec![
            SavedGroup {
                tabs: vec![
                    SavedTab {
                        file: 0,
                        view: view(3, Some(0)),
                    },
                    SavedTab {
                        file: 1,
                        view: view(4, None),
                    },
                ],
                active: 0,
                recent: vec![0, 1],
            },
            SavedGroup {
                tabs: vec![
                    SavedTab {
                        file: 0,
                        view: view(5, None),
                    },
                    SavedTab {
                        file: 1,
                        view: view(1, None),
                    },
                ],
                active: 1,
                recent: vec![1, 0],
            },
        ];
        let panes = groups
            .iter()
            .map(|group| SavedPane {
                file: group.tabs[group.active].file,
                view: group.tabs[group.active].view.clone(),
            })
            .collect();
        let layout = Layout {
            files,
            panes,
            active_file: 1,
            active_pane: 1,
            active_group: 1,
            horizontal: true,
            groups: Some(groups),
        }
        .normalized()
        .unwrap();
        crate::session::Restored { layout, documents }
    }
    #[test]
    fn grouped_restore_keeps_inactive_tab_order_recent_and_all_shared_historical_views() {
        let root = tempfile::tempdir().unwrap();
        let restored = grouped_fixture(root.path());
        let expected = restored.layout.clone();
        let ids = restored
            .documents
            .iter()
            .map(|doc| doc.id)
            .collect::<Vec<_>>();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_session(restored).unwrap();
        assert_eq!(app.capture_session().unwrap(), expected);
        assert_eq!(
            app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
            ids
        );
        assert_eq!(app.editor_groups.groups().len(), 2);
        let left = app.editor_groups.groups()[0].id().value();
        let right = app.editor_groups.groups()[1].id().value();
        assert_eq!(app.documents[0].view_state(Some(left)).cursor, 3);
        assert_eq!(app.documents[0].view_state(Some(left)).anchor, Some(0));
        assert_eq!(app.documents[0].view_state(Some(right)).cursor, 5);
        assert_eq!(app.documents[1].view_state(Some(left)).cursor, 4);
        assert_eq!(app.documents[1].view_state(Some(right)).cursor, 1);
        assert_eq!(app.doc().id, ids[1]);
        assert_eq!(app.doc().cursor, 1);
        app.doc_mut().undo();
        assert_eq!(app.doc().text, "猫🙂 value\r\nnext\r\n");
        assert!(!app.doc().dirty());
    }
    #[test]
    fn late_configuration_failure_leaves_models_engine_views_and_identity_counters_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let restored = grouped_fixture(root.path());
        let mut app = App::new(root.path().into(), Profile::Linux);
        let mut recovered = Document::from_text("recovered 猫🙂\r\n");
        recovered.insert("dirty", false);
        app.documents.push(recovered);
        app.sync_pane();
        let engine = app.editor_groups.clone();
        let context = app.session_context();
        let bytes = app.doc().text.to_string();
        let mut calls = 0;
        let result = app.install_session_with(restored, |_, doc| {
            calls += 1;
            doc.tab_size = 2;
            if calls == 2 {
                anyhow::bail!("injected late language configuration refusal");
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.editor_groups, engine);
        assert!(app.session_context() == context);
        assert_eq!(app.doc().text.to_string(), bytes);
        app.doc_mut().undo();
        assert_eq!(app.doc().text, "recovered 猫🙂\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), bytes);
    }
    #[test]
    fn legacy_live_layout_migrates_global_tabs_and_views_into_groups_before_install() {
        let root = tempfile::tempdir().unwrap();
        let mut restored = grouped_fixture(root.path());
        restored.layout.groups = None;
        restored.layout.active_group = 0;
        let expected = restored.layout.normalized().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_session(restored).unwrap();
        assert_eq!(app.capture_session().unwrap(), expected);
        assert_eq!(app.editor_groups.groups()[0].tabs().len(), 2);
        assert_eq!(app.editor_groups.groups()[1].tabs().len(), 1);
        assert_eq!(app.active_pane, 1);
    }
    #[test]
    fn group_focus_aba_retires_session_context_even_with_identical_model_selections() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.install_session(grouped_fixture(root.path())).unwrap();
        let before = app.session_context();
        let left = app.editor_groups.groups()[0].id();
        let right = app.editor_groups.groups()[1].id();
        app.editor_groups.focus_group(left).unwrap();
        app.editor_groups.focus_group(right).unwrap();
        assert!(app.session_context() != before);
        assert_eq!(app.session_context().panes, before.panes);
        assert_eq!(app.session_context().documents, before.documents);
    }
    #[test]
    fn distributed_existing_models_retain_groups_when_clean_session_files_fit() {
        for new_files in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut restored = grouped_fixture(root.path());
            let mut app = App::new(root.path().into(), Profile::Linux);
            app.documents = (0..129)
                .map(|index| Document::from_text(&format!("retained {index} 猫🙂\r\n")))
                .collect();
            if !new_files {
                for (index, doc) in restored.documents.drain(..).enumerate() {
                    app.documents[index] = doc;
                }
            }
            app.documents[0].insert("dirty", false);
            app.documents[0].undo();
            let ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
            app.editor_groups
                .import(
                    &[
                        RestoreGroup {
                            documents: ids[..128].to_vec(),
                            active: 0,
                            recent: (0..128).collect(),
                        },
                        RestoreGroup {
                            documents: vec![ids[0], ids[128]],
                            active: 1,
                            recent: vec![1, 0],
                        },
                    ],
                    1,
                )
                .unwrap();
            // This fixture imports the membership engine directly; establish
            // its companion topology before exercising production restoration.
            app.editor_layout = app
                .prepare_import_group_layout(
                    &app.editor_groups,
                    None,
                    crate::editor_layout::Axis::Columns,
                )
                .unwrap();
            let left = app.editor_groups.groups()[0].id().value();
            let right = app.editor_groups.groups()[1].id().value();
            app.documents[0].activate_view(left);
            app.documents[0].move_to(1, false);
            app.documents[0].move_to(3, true);
            app.documents[0].top = 7;
            app.documents[0].activate_view(right);
            app.documents[0].move_to(4, false);
            app.documents[0].move_to(2, true);
            app.documents[0].left = 9;
            app.project_editor_groups();
            let engine = app.editor_groups.clone();
            let panes = app
                .panes
                .iter()
                .map(|pane| (pane.id, pane.document))
                .collect::<Vec<_>>();
            let left_view =
                View::capture(&app.documents[0], app.documents[0].view_state(Some(left))).unwrap();
            let right_view =
                View::capture(&app.documents[0], app.documents[0].view_state(Some(right))).unwrap();
            let epoch = app.documents[0].text_epoch();
            let generation = app.documents[0].save_generation();
            let original = app.documents[0].text.clone();

            app.install_session(restored).unwrap();

            assert!(!app.group_fallback);
            assert_eq!(app.editor_groups.groups().len(), 2);
            assert_eq!(app.documents.len(), if new_files { 131 } else { 129 });
            assert_eq!(
                app.panes
                    .iter()
                    .map(|pane| (pane.id, pane.document))
                    .collect::<Vec<_>>(),
                panes
            );
            assert_eq!(app.doc().id, ids[128]);
            assert_eq!(app.editor_groups.groups()[0], engine.groups()[0]);
            assert_eq!(
                app.editor_groups.active_membership(),
                engine.active_membership()
            );
            if new_files {
                assert_eq!(app.editor_groups.groups()[1].tabs().len(), 4);
                assert_eq!(
                    &app.editor_groups.groups()[1].tabs()[..2],
                    engine.groups()[1].tabs()
                );
            } else {
                assert_eq!(app.editor_groups, engine);
            }
            assert_eq!(
                app.documents[..129]
                    .iter()
                    .map(|doc| doc.id)
                    .collect::<Vec<_>>(),
                ids
            );
            assert_eq!(
                View::capture(&app.documents[0], app.documents[0].view_state(Some(left))).unwrap(),
                left_view
            );
            assert_eq!(
                View::capture(&app.documents[0], app.documents[0].view_state(Some(right))).unwrap(),
                right_view
            );
            assert_eq!(app.documents[0].text, original);
            assert_eq!(app.documents[0].text_epoch(), epoch);
            assert_eq!(app.documents[0].save_generation(), generation);
            app.documents[0].redo();
            assert_eq!(
                app.documents[0].text.to_string(),
                format!("dirty{original}")
            );
            app.documents[0].undo();
            assert_eq!(app.documents[0].text, original);
            for name in ["a.cpp", "b.cpp"] {
                assert_eq!(
                    fs::read_to_string(root.path().join(name)).unwrap(),
                    "猫🙂 value\r\nnext\r\n"
                );
            }
        }
    }
    #[test]
    fn over_cap_recovery_retains_every_variant_and_history_while_appending_clean_files() {
        let root = tempfile::tempdir().unwrap();
        let restored = grouped_fixture(root.path());
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.documents = (0..129)
            .map(|index| {
                let mut doc = Document::from_text(&format!("recovered {index} 猫🙂\r\n"));
                doc.insert("dirty", false);
                doc
            })
            .collect();
        app.active = 128;
        app.sync_pane();
        let ids = app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>();
        let primary = app.doc().id;
        let before = app.doc().text.to_string();
        app.install_session(restored).unwrap();
        assert!(app.group_fallback);
        assert!(app.message.contains("all recovery retained"));
        assert_eq!(app.documents.len(), 131);
        assert_eq!(
            app.documents[..129]
                .iter()
                .map(|doc| doc.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(app.doc().id, primary);
        app.doc_mut().undo();
        assert_eq!(app.doc().text, "recovered 128 猫🙂\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), before);
    }
}

#[cfg(test)]
mod combined_tests {
    use super::*;
    use std::fs;
    fn until(app: &mut App, condition: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if condition(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn queued_native_extension_prompt_cancels_restore_and_retains_metadata_for_retry() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let file = root.path().join("saved.txt");
        fs::write(&file, "saved disk\r\n").unwrap();
        let mut seed = App::new(root.path().into(), Profile::Linux);
        seed.configure_session(Some(&config), false).unwrap();
        seed.open(&file).unwrap();
        seed.finish_session().unwrap();
        let extension = root.path().join("extension");
        fs::create_dir(&extension).unwrap();
        fs::write(extension.join("package.json"), r#"{"publisher":"fixture","name":"session-prompt","version":"1.0.0","main":"extension.cjs"}"#).unwrap();
        fs::write(extension.join("extension.cjs"), r#"
const vscode = require('vscode');
exports.activate = context => context.subscriptions.push(vscode.commands.registerCommand('session.prompt', async () => {
 const answer = await vscode.window.showInputBox({title:'Session interruption'});
 await vscode.window.showInformationMessage('session answer='+answer);
}));
"#).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_session(Some(&config), false).unwrap();
        app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
            .unwrap();
        until(&mut app, |a| {
            a.session.ready && a.extension_host.as_ref().is_some_and(|h| h.ready)
        });
        app.start_prompt(PromptKind::Palette, "restore".into());
        app.execute("session.prompt", Value::Null);
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .is_some_and(|h| h.prompt().is_some())
        });
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Palette
        ));
        app.prompt = None; // Native palette acceptance removes itself before dispatch.
        app.execute("vscli.session.restore", Value::Null);
        assert!(app.session.pending.is_some());
        until(&mut app, |a| a.session.pending.is_none());
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Extension(_)
        ));
        assert!(app.documents.is_empty());
        assert!(app.session.protected);
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        until(&mut app, |a| a.message == "session answer=undefined");
        // A popup can be retired by its owner before a loader reply is polled.
        // Its appearance must still invalidate the captured restore generation.
        app.start_prompt(PromptKind::Palette, String::new());
        app.execute("session.prompt", Value::Null);
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .is_some_and(|h| h.prompt().is_some())
        });
        app.prompt = None;
        app.execute("vscli.session.restore", Value::Null);
        app.poll_extensions();
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Extension(_)
        ));
        app.cancel_extension_prompt();
        assert!(app.prompt.is_none());
        until(&mut app, |a| {
            a.session.pending.is_none() && a.extension_host.as_ref().is_some_and(|h| !h.busy())
        });
        assert!(app.documents.is_empty());
        app.execute("vscli.session.restore", Value::Null);
        until(&mut app, |a| !a.documents.is_empty());
        assert_eq!(app.doc().text, "saved disk\r\n");
        assert!(app.prompt.is_none());
        let id = app.doc().id;
        app.execute("session.prompt", Value::Null);
        until(&mut app, |a| {
            matches!(
                a.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Extension(_))
            )
        });
        app.event(Event::Paste("answer".into()));
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        until(&mut app, |a| a.message == "session answer=answer");
        assert_eq!(app.doc().id, id);
        assert_eq!(fs::read_to_string(&file).unwrap(), "saved disk\r\n");
        app.finish_session().unwrap();
    }
    #[test]
    fn workspace_symbol_picker_wins_over_pending_restore_and_late_replies_cannot_revive_either_ui()
    {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let file = root.path().join("saved.txt");
        fs::write(&file, "preserved\r\n").unwrap();
        let file = fs::canonicalize(file).unwrap();
        fs::write(root.path().join("other.cpp"), "x 😀foo\r\n").unwrap();
        let mut seed = App::new(root.path().into(), Profile::Linux);
        seed.configure_session(Some(&config), false).unwrap();
        seed.open(&file).unwrap();
        seed.finish_session().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_session(Some(&config), false).unwrap();
        app.lsp = Some(
            crate::lsp::Client::start(
                "python3",
                &[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/symbol_server.py")
                    .to_string_lossy()
                    .into_owned()],
                root.path(),
                "cpp".into(),
            )
            .unwrap(),
        );
        until(&mut app, |a| {
            a.session.ready && a.lsp.as_ref().is_some_and(|c| c.ready)
        });
        app.execute("vscli.session.restore", Value::Null);
        app.execute("workbench.action.showAllSymbols", Value::Null);
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Symbols
        ));
        until(&mut app, |a| {
            a.session.pending.is_none() && !a.symbol_items("").is_empty()
        });
        assert!(app.documents.is_empty());
        assert!(app.session.protected);
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        app.execute("vscli.session.restore", Value::Null);
        until(&mut app, |a| !a.documents.is_empty());
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().path.as_ref(), Some(&file));
        assert_eq!(app.doc().text, "preserved\r\n");
        assert!(!app.doc().dirty());
        app.finish_session().unwrap();
    }
}
