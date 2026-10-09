use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[derive(Clone)]
struct Closed {
    id: u64,
    path: PathBuf,
    row: usize,
    column: usize,
}
#[derive(PartialEq)]
struct Context {
    workspace: PathBuf,
    focus: Focus,
    pane: Option<u64>,
    document: Option<(u64, u64, usize)>,
    generation: u64,
}
enum Target {
    Existing(PathBuf),
    Loaded(Box<Document>),
}
struct Pending {
    receiver: Receiver<Result<Target, String>>,
    context: Context,
    closed: Option<Closed>,
}
#[derive(Default)]
pub(super) struct State {
    closed: Vec<Closed>,
    sequence: u64,
    generation: u64,
    pending: Option<Pending>,
}
impl App {
    fn navigation_context(&self) -> Context {
        Context {
            workspace: self.workspace.root.clone(),
            focus: self.focus.clone(),
            pane: self.panes.get(self.active_pane).map(|pane| pane.id),
            document: self
                .active_document()
                .map(|doc| (doc.id, doc.revision, doc.cursor)),
            generation: self.navigation.generation,
        }
    }
    pub fn configure_recents(&mut self, root: Option<&Path>) {
        self.recent_files.configure(root);
    }
    pub fn finish_recents(&mut self) -> Result<()> {
        self.recent_files.finish()
    }
    pub(super) fn cancel_navigation(&mut self) {
        self.navigation.generation = self.navigation.generation.wrapping_add(1);
    }
    pub(super) fn remember_active_file(&mut self) {
        if let Some(path) = self.active_document().and_then(|doc| doc.path.clone()) {
            self.recent_files.touch(path);
        }
    }
    pub(super) fn record_closed(&mut self) {
        if let Some(doc) = self.active_document()
            && let Some(path) = &doc.path
        {
            let (path, row, column) = (path.clone(), doc.row(), doc.visual_column());
            self.navigation.sequence = self.navigation.sequence.wrapping_add(1);
            self.navigation.closed.push(Closed {
                id: self.navigation.sequence,
                path,
                row,
                column,
            });
            if self.navigation.closed.len() > crate::recent::LIMIT {
                self.navigation.closed.remove(0);
            }
        }
        self.cancel_navigation();
    }
    pub fn recent_items(&self, query: &str) -> Vec<&Path> {
        let mut entries: Vec<_> = self
            .recent_files
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                score(&entry.path.to_string_lossy(), query)
                    .map(|score| (score, index, entry.path.as_path()))
            })
            .collect();
        entries.sort_by_key(|(score, index, _)| (std::cmp::Reverse(*score), *index));
        entries.into_iter().map(|(_, _, path)| path).collect()
    }
    pub(super) fn accept_recent(&mut self, query: &str, index: usize) {
        let paths = self.recent_items(query);
        if let Some(path) = paths.get(index.min(paths.len().saturating_sub(1))) {
            self.open_navigation((*path).to_owned(), None);
        } else {
            self.message = "No matching recent files".into();
        }
    }
    pub(super) fn reopen_closed(&mut self) {
        if let Some(closed) = self.navigation.closed.last().cloned() {
            self.open_navigation(closed.path.clone(), Some(closed));
        } else {
            self.message = "No closed file-backed editors in this session".into();
        }
    }
    fn focus_existing_navigation(&mut self, path: &Path) -> bool {
        if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|doc| doc.path.as_deref() == Some(path))
        {
            let doc = self.hidden_documents.remove(index);
            self.install_open_document(doc);
            return true;
        }
        let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.path.as_deref() == Some(path))
        else {
            return false;
        };
        let id = self.documents[index].id;
        if let Some(pane) = self.panes.iter().position(|pane| pane.document == id) {
            self.focus_pane(pane);
        } else {
            self.active = index;
            self.focus = Focus::Editor;
            self.sync_pane();
        }
        self.remember_active_file();
        self.message = format!("Focused {}", self.doc().name());
        true
    }
    pub(super) fn open_hidden_aware(&mut self, path: PathBuf) {
        self.open_navigation_mode(path, None, true);
    }
    fn open_navigation(&mut self, path: PathBuf, closed: Option<Closed>) {
        self.open_navigation_mode(path, closed, false);
    }
    fn open_navigation_mode(&mut self, path: PathBuf, closed: Option<Closed>, allow_missing: bool) {
        if self.focus_existing_navigation(&path) {
            self.finish_reopen(closed.as_ref());
            self.cancel_navigation();
            return;
        }
        if self.navigation.pending.is_some() {
            self.message = "A recent file is still loading; retry shortly".into();
            return;
        }
        self.cancel_navigation();
        let context = self.navigation_context();
        let (sender, receiver) = mpsc::sync_channel(1);
        let existing: Vec<_> = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter_map(|d| d.path.clone())
            .collect();
        let workspace = self.workspace.root.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Target> {
                let path = super::extension_services::resolved(&path, &workspace)?;
                if existing.contains(&path) {
                    return Ok(Target::Existing(path));
                }
                let doc = if allow_missing {
                    Document::open(&path)?
                } else {
                    Document::open_existing(&path)?
                };
                Ok(Target::Loaded(Box::new(doc)))
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        self.navigation.pending = Some(Pending {
            receiver,
            context,
            closed,
        });
        self.message = "Opening recent file…".into();
    }
    fn finish_reopen(&mut self, closed: Option<&Closed>) {
        if let Some(closed) = closed {
            self.navigation.closed.retain(|entry| entry.id != closed.id);
        }
    }
    pub(super) fn poll_navigation(&mut self) -> bool {
        let mut changed = self.recent_files.poll();
        if changed && let Some(error) = &self.recent_files.error {
            self.message = format!("Recent files unavailable: {error}");
        }
        let Some(pending) = &self.navigation.pending else {
            return changed;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return changed,
            Err(TryRecvError::Disconnected) => Err("Recent-file loader stopped".into()),
        };
        let pending = self.navigation.pending.take().unwrap();
        changed = true;
        if pending.context != self.navigation_context()
            || self.prompt.is_some()
            || self.modal.is_some()
        {
            return changed;
        }
        match result {
            Ok(Target::Existing(path)) => {
                if self.focus_existing_navigation(&path) {
                    self.finish_reopen(pending.closed.as_ref());
                } else {
                    self.message =
                        "Resolved native buffer was closed before navigation completed".into();
                }
            }
            Ok(Target::Loaded(mut doc)) => {
                if !self.focus_existing_navigation(doc.path.as_ref().unwrap()) {
                    self.settings.apply(&mut doc);
                    if let Some(closed) = &pending.closed {
                        doc.move_to(doc.position_at(closed.row, closed.column), false);
                    }
                    self.install_open_document(*doc);
                }
                self.finish_reopen(pending.closed.as_ref());
            }
            Err(error) => {
                self.message =
                    format!("Recent file could not be opened (history retained): {error}")
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.navigation.pending.is_some() {
            app.poll_navigation();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn command(app: &mut App, id: &str) {
        app.execute(id, Value::Null);
    }
    #[test]
    fn canceled_close_discard_save_as_and_reopen_preserve_disk_and_clamp_cursor() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("original.txt");
        std::fs::write(&path, "original\nlast line").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("unsaved", false);
        command(&mut app, "workbench.action.closeActiveEditor");
        app.modal_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.navigation.closed.is_empty());
        assert!(app.doc().dirty());
        command(&mut app, "workbench.action.closeActiveEditor");
        app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(app.active_document().is_none());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "original\nlast line"
        );
        std::fs::write(&path, "changed").unwrap();
        command(&mut app, "workbench.action.reopenClosedEditor");
        wait(&mut app);
        assert_eq!(app.doc().text.to_string(), "changed");
        assert!(!app.doc().dirty());
        assert!(app.navigation.closed.is_empty());
        app.doc_mut().move_to(7, false);
        app.doc_mut().insert("!", false);
        command(&mut app, "workbench.action.closeActiveEditor");
        app.modal_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(app.active_document().is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed!");
        std::fs::write(&path, "x").unwrap();
        command(&mut app, "workbench.action.reopenClosedEditor");
        wait(&mut app);
        assert_eq!(app.doc().cursor, 1);
        command(&mut app, "workbench.action.files.newUntitledFile");
        app.doc_mut().insert("new saved", false);
        command(&mut app, "workbench.action.closeActiveEditor");
        app.modal_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        let unsaved_id = app.doc().id;
        app.prompt_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.doc().id, unsaved_id);
        assert!(app.doc().dirty());
        assert!(app.navigation.closed.is_empty());
        command(&mut app, "workbench.action.closeActiveEditor");
        app.modal_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        let saved = root.path().join("saved-as.txt");
        app.prompt.as_mut().unwrap().text = saved.to_string_lossy().into_owned();
        app.accept_prompt();
        let saved = std::fs::canonicalize(saved).unwrap();
        assert_eq!(app.navigation.closed.last().unwrap().path, saved);
        command(&mut app, "workbench.action.reopenClosedEditor");
        wait(&mut app);
        assert_eq!(app.doc().path.as_ref(), Some(&saved));
        assert_eq!(app.doc().text.to_string(), "new saved");
    }
    #[test]
    fn open_shared_dirty_file_without_disk_and_retry_missing_closed_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file.txt");
        std::fs::write(&path, "saved").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("dirty", false);
        let id = app.doc().id;
        app.split_editor(false);
        command(&mut app, "workbench.action.closeActiveEditor");
        assert_eq!(app.documents.len(), 1);
        std::fs::remove_file(&path).unwrap();
        command(&mut app, "workbench.action.reopenClosedEditor");
        assert!(app.navigation.pending.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "dirtysaved");
        app.accept_recent("file.txt", 0);
        assert_eq!(app.doc().id, id);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "saved");
        command(&mut app, "workbench.action.closeActiveEditor");
        command(&mut app, "workbench.action.reopenClosedEditor");
        wait(&mut app);
        assert!(app.active_document().is_none());
        assert_eq!(app.navigation.closed.len(), 1);
        assert!(!path.exists());
        std::fs::write(&path, "returned").unwrap();
        command(&mut app, "workbench.action.reopenClosedEditor");
        wait(&mut app);
        assert_eq!(app.doc().text.to_string(), "returned");
    }
    #[test]
    fn stale_open_replies_never_replace_new_context_or_consume_closed_history() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file.txt");
        std::fs::write(&path, "saved").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        command(&mut app, "workbench.action.closeActiveEditor");
        command(&mut app, "workbench.action.reopenClosedEditor");
        command(&mut app, "workbench.action.files.newUntitledFile");
        app.doc_mut().insert("retain", false);
        let id = app.doc().id;
        wait(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "retain");
        assert_eq!(app.navigation.closed.len(), 1);
        command(&mut app, "workbench.action.reopenClosedEditor");
        app.workspace.root = root.path().join("changed-context");
        wait(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.navigation.closed.len(), 1);
    }
    #[test]
    fn repeated_reopen_while_loading_completes_the_original_request() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file.txt");
        std::fs::write(&path, "saved").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        command(&mut app, "workbench.action.closeActiveEditor");
        command(&mut app, "workbench.action.reopenClosedEditor");
        command(&mut app, "workbench.action.reopenClosedEditor");
        wait(&mut app);
        assert_eq!(app.doc().path, Some(std::fs::canonicalize(path).unwrap()));
        assert!(app.navigation.closed.is_empty());
    }
    #[test]
    fn close_all_records_only_file_editors_and_recent_state_uses_config_root() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("native");
        let profile = config.join("imports/profile");
        std::fs::create_dir_all(&profile).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_settings(Some(profile.join("settings.json")))
            .unwrap();
        app.configure_recents(Some(&config));
        for name in ["a.txt", "b.txt"] {
            let path = root.path().join(name);
            std::fs::write(&path, name).unwrap();
            app.open(&path).unwrap();
        }
        command(&mut app, "workbench.action.files.newUntitledFile");
        command(&mut app, "workbench.action.closeAllEditors");
        assert!(app.documents.is_empty());
        assert_eq!(app.navigation.closed.len(), 2);
        app.finish_recents().unwrap();
        assert_eq!(
            crate::recent::read(&config.join("state/recent-files.json"))
                .unwrap()
                .len(),
            2
        );
        assert!(!profile.join("state").exists());
    }
}
