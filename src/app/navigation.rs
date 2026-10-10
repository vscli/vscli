use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};
pub(super) enum OpenIntent {
    Focus,
    Location(crate::lsp::Range),
    Search(crate::search::Hit),
    Debug { line: usize, column: usize },
    Settings,
    History(super::navigation_history::Travel),
}

#[cfg(test)]
mod history_tests {
    use super::super::navigation_history::{Direction, Reason};
    use super::*;
    use std::time::{Duration, Instant};
    fn three(root: &Path) -> (App, Vec<PathBuf>) {
        let mut app = App::new(root.into(), Profile::Linux);
        let mut paths = Vec::new();
        for name in ["a.cpp", "b.cpp", "c.cpp"] {
            let path = root.join(name);
            std::fs::write(&path, format!("{name} 猫🙂\r\n")).unwrap();
            let path = std::fs::canonicalize(path).unwrap();
            app.open(&path).unwrap();
            paths.push(path);
        }
        app.documents.drain(..2);
        app.active = 0;
        app.sync_pane();
        (app, paths)
    }
    fn hold(app: &mut App) -> mpsc::SyncSender<Result<Target, String>> {
        let (sender, receiver) = mpsc::sync_channel(1);
        app.navigation.pending = Some(Pending {
            receiver,
            context: app.navigation_context(),
            closed: None,
            intent: OpenIntent::Focus,
        });
        sender
    }
    fn finish(app: &mut App) {
        let until = Instant::now() + Duration::from_secs(3);
        while app.navigation.pending.is_some() {
            assert!(Instant::now() < until, "{}", app.message);
            app.poll_navigation();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn repeated_history_travel_retains_actual_slot_and_only_latest_destination_commits() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, paths) = three(root.path());
        let sender = hold(&mut app);
        let original = app.navigation.pending.as_ref().unwrap().context.clone();
        app.navigate_history(Direction::Back);
        app.navigate_history(Direction::Back);
        for _ in 0..32 {
            app.navigate_history(Direction::Back);
        }
        assert_eq!(app.doc().path.as_deref(), Some(paths[2].as_path()));
        assert!(app.navigation.pending.as_ref().unwrap().context == original);
        assert!(app.can_navigate_back());
        assert!(!app.can_navigate_forward());
        sender.send(Err("Retired held file load".into())).unwrap();
        app.poll_navigation();
        finish(&mut app);
        assert_eq!(app.doc().path.as_deref(), Some(paths[0].as_path()));
        assert!(!app.can_navigate_back());
        assert!(app.can_navigate_forward());
        assert_eq!(app.documents.len(), 2);
        assert!(
            app.documents
                .iter()
                .all(|doc| doc.path.as_deref() != Some(paths[1].as_path()))
        );
    }
    #[test]
    fn queued_history_context_cannot_revive_after_edit_undo_or_input_pane_round_trip() {
        for keyboard in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (mut app, paths) = three(root.path());
            let sender = hold(&mut app);
            app.navigate_history(Direction::Back);
            let id = app.doc().id;
            let before = app.doc().text.to_string();
            let cursor = app.doc().cursor;
            if keyboard {
                app.navigation_input_interaction();
                app.focus = Focus::Explorer;
                app.focus = Focus::Editor;
            } else {
                app.doc_mut().insert("dirty", false);
                app.doc_mut().undo();
                assert_eq!(app.doc().cursor, cursor);
            }
            sender
                .send(Ok(Target::Loaded(Box::new(
                    Document::open_existing(&paths[0]).unwrap(),
                ))))
                .unwrap();
            app.poll_navigation();
            assert!(app.navigation.pending.is_none());
            assert_eq!(app.documents.len(), 1);
            assert_eq!(app.doc().id, id);
            assert_eq!(app.doc().text.to_string(), before);
            assert!(app.can_navigate_back());
            assert!(!app.can_navigate_forward());
            assert_eq!(std::fs::read(&paths[2]).unwrap(), before.as_bytes());
            if !keyboard {
                app.doc_mut().redo();
                assert_eq!(app.doc().text.to_string(), "dirty".to_owned() + &before);
            }
        }
    }
    #[test]
    fn failed_closed_resource_load_preserves_committed_stack_and_unsaved_source() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, paths) = three(root.path());
        app.doc_mut().insert("dirty", false);
        app.observe_navigation(Reason::Ordinary);
        let id = app.doc().id;
        let before = app.doc().text.to_string();
        std::fs::remove_file(&paths[1]).unwrap();
        app.navigate_history(Direction::Back);
        finish(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), before);
        assert!(app.doc().dirty());
        assert!(app.can_navigate_back());
        assert!(!app.can_navigate_forward());
        assert!(app.message.contains("history retained"));
        app.doc_mut().undo();
        assert_eq!(
            app.doc().text.to_string(),
            before.trim_start_matches("dirty")
        );
    }
    #[test]
    fn loader_capture_budget_rejects_before_dispatch_and_preserves_redo() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, paths) = three(root.path());
        app.doc_mut().insert("dirty", false);
        app.doc_mut().undo();
        let before = app.doc().text.to_string();
        for _ in 0..128 {
            app.documents.push(Document::from_text(""));
        }
        app.navigate_history(Direction::Back);
        assert!(app.navigation.pending.is_none());
        assert_eq!(app.doc().path.as_deref(), Some(paths[2].as_path()));
        assert!(app.can_navigate_back());
        assert!(!app.can_navigate_forward());
        assert!(app.message.contains("128 models"));
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "dirty".to_owned() + &before);
        assert_ne!(
            std::fs::read(&paths[2]).unwrap(),
            app.doc().text.to_string().as_bytes()
        );
    }
    #[test]
    fn exhausted_interaction_fence_never_wraps_or_installs_a_held_old_reply() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, paths) = three(root.path());
        let sender = hold(&mut app);
        let id = app.doc().id;
        app.navigation.interaction = u64::MAX - 1;
        app.navigation_input_interaction();
        app.navigation_input_interaction();
        assert_eq!(app.navigation.interaction, u64::MAX);
        app.navigate_history(Direction::Back);
        assert!(app.message.contains("restart the editor"));
        sender
            .send(Ok(Target::Loaded(Box::new(
                Document::open_existing(&paths[0]).unwrap(),
            ))))
            .unwrap();
        app.poll_navigation();
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().id, id);
        assert!(!app.can_navigate_back());
        assert!(!app.can_navigate_forward());
    }
    #[test]
    fn duplicate_held_reopen_preserves_only_the_current_original_request() {
        for invalidation in [0, 1, 2] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.cpp");
            let target = root.path().join("target.cpp");
            std::fs::write(&source, "source 猫🙂\r\n").unwrap();
            std::fs::write(&target, "target 猫🙂\r\n").unwrap();
            let mut app = App::new(root.path().into(), Profile::Linux);
            app.open(&source).unwrap();
            let id = app.doc().id;
            let before = app.doc().text.to_string();
            let closed = Closed {
                id: 1,
                path: target.clone(),
                row: 0,
                column: 0,
            };
            app.navigation.closed.push(closed.clone());
            let (sender, receiver) = mpsc::sync_channel(1);
            let context = app.navigation_context();
            app.navigation.pending = Some(Pending {
                receiver,
                context: context.clone(),
                closed: Some(closed),
                intent: OpenIntent::Focus,
            });
            assert!(app.duplicate_reopen_pending());
            if invalidation == 1 {
                app.doc_mut().insert("dirty", false);
                app.doc_mut().undo();
            } else if invalidation == 2 {
                app.navigation_input_interaction();
            }
            assert_eq!(app.duplicate_reopen_pending(), invalidation == 0);
            app.execute("workbench.action.reopenClosedEditor", Value::Null);
            assert!(app.navigation.pending.as_ref().unwrap().context == context);
            sender
                .send(Ok(Target::Loaded(Box::new(
                    Document::open_existing(&target).unwrap(),
                ))))
                .unwrap();
            app.poll_navigation();
            assert!(app.navigation.pending.is_none());
            if invalidation == 0 {
                assert_eq!(
                    app.doc().path,
                    Some(std::fs::canonicalize(&target).unwrap())
                );
                assert!(app.navigation.closed.is_empty());
            } else {
                assert_eq!(app.doc().id, id);
                assert_eq!(app.doc().text.to_string(), before);
                assert_eq!(app.navigation.closed.len(), 1);
                if invalidation == 1 {
                    app.doc_mut().redo();
                    assert_eq!(app.doc().text.to_string(), "dirty".to_owned() + &before);
                }
            }
            assert_eq!(std::fs::read(&source).unwrap(), before.as_bytes());
            assert_eq!(
                std::fs::read(&target).unwrap(),
                "target 猫🙂\r\n".as_bytes()
            );
        }
    }
}
impl OpenIntent {
    pub(super) fn validate(&self, doc: &Document) -> Result<()> {
        if let Self::Location(range) = self {
            let start = crate::lsp::offset(doc, range.start)?;
            let end = crate::lsp::offset(doc, range.end)?;
            if start > end {
                anyhow::bail!("Navigation range is reversed");
            }
        }
        Ok(())
    }
    pub(super) fn apply(&self, app: &mut App) {
        match self {
            Self::Focus => {}
            Self::Location(range) => {
                // Validated against this exact target before focus changed.
                let start = crate::lsp::offset(app.doc(), range.start).unwrap();
                let end = crate::lsp::offset(app.doc(), range.end).unwrap();
                app.doc_mut().clear_secondary();
                app.doc_mut().move_to(start, false);
                app.doc_mut().move_to(end, true);
            }
            Self::Search(hit) => {
                app.doc_mut().clear_secondary();
                let row = hit.row.min(app.doc().line_count() - 1);
                let start = app.doc().line_start(row);
                if crate::search::line_hash(&app.doc().line(row)) == hit.line_hash {
                    app.doc_mut().move_to(start + hit.column, false);
                    app.doc_mut().move_to(start + hit.column + hit.length, true);
                } else {
                    app.doc_mut().move_to(start, false);
                    app.message =
                        "Search result changed; rerun Find in Files for current matches".into();
                }
            }
            Self::Debug { line, column } => {
                app.doc_mut().clear_secondary();
                let row = line.saturating_sub(1).min(app.doc().line_count() - 1);
                let position = app
                    .doc()
                    .line_start(row)
                    .saturating_add(column.saturating_sub(1))
                    .min(app.doc().line_end(row));
                app.doc_mut().move_to(position, false);
                app.message = if app.doc().dirty() {
                    "Debugger stopped; source is modified, displayed lines may differ from disk"
                } else {
                    "Debugger stopped · F5 continue · F10 step · Ctrl+Shift+D stack/variables"
                }
                .into();
            }
            Self::Settings => {
                if app.doc().is_empty() && app.doc().disk_content.is_none() {
                    app.doc_mut().insert("{\n}\n", false);
                }
            }
            Self::History(_) => {}
        }
        if matches!(
            self,
            Self::Location(_) | Self::Search(_) | Self::Debug { .. }
        ) {
            app.observe_navigation(super::navigation_history::Reason::Jump);
        }
    }
}
#[derive(Clone)]
struct Closed {
    id: u64,
    path: PathBuf,
    row: usize,
    column: usize,
}
#[derive(Clone, PartialEq)]
pub(super) struct Context {
    workspace: PathBuf,
    focus: Focus,
    pane: Option<u64>,
    groups: crate::editor_groups::UiProof,
    fallback: bool,
    document: Option<(u64, u64, u64, u64, usize, Option<usize>)>,
    generation: u64,
    interaction: u64,
}
enum Target {
    Existing(PathBuf),
    Loaded(Box<Document>),
}
struct Pending {
    receiver: Receiver<Result<Target, String>>,
    context: Context,
    closed: Option<Closed>,
    intent: OpenIntent,
}
#[derive(Default)]
pub(super) struct State {
    closed: Vec<Closed>,
    sequence: u64,
    generation: u64,
    interaction: u64,
    pending: Option<Pending>,
}
impl App {
    pub(super) fn navigation_context(&self) -> Context {
        Context {
            workspace: self.workspace.root.clone(),
            focus: self.focus.clone(),
            pane: self.panes.get(self.active_pane).map(|pane| pane.id),
            groups: self.editor_groups.proof(),
            fallback: self.group_fallback,
            document: self.active_document().map(|doc| {
                (
                    doc.id,
                    doc.revision,
                    doc.text_epoch(),
                    doc.save_generation(),
                    doc.cursor,
                    doc.anchor,
                )
            }),
            generation: self.navigation.generation,
            interaction: self.navigation.interaction,
        }
    }
    pub fn configure_recents(&mut self, root: Option<&Path>) {
        self.recent_files.configure(root);
    }
    pub fn finish_recents(&mut self) -> Result<()> {
        self.recent_files.finish()
    }
    pub(super) fn cancel_navigation(&mut self) {
        self.navigation.generation = self.navigation.generation.saturating_add(1);
        self.history_navigation_cancelled();
    }
    pub(super) fn navigation_input_interaction(&mut self) {
        self.navigation.interaction = self.navigation.interaction.saturating_add(1);
        self.cancel_navigation();
    }
    pub(super) fn history_loader_busy(&self) -> bool {
        self.navigation.pending.is_some()
    }
    pub(super) fn duplicate_reopen_pending(&self) -> bool {
        self.navigation_channel_available()
            && self.prompt.is_none()
            && self.modal.is_none()
            && self.navigation.pending.as_ref().is_some_and(|pending| {
                matches!(pending.intent, OpenIntent::Focus)
                    && pending.closed.as_ref().is_some_and(|closed| {
                        self.navigation
                            .closed
                            .last()
                            .is_some_and(|latest| latest.id == closed.id)
                    })
                    && pending.context == self.navigation_context()
            })
    }
    pub(super) fn navigation_channel_available(&self) -> bool {
        self.navigation.generation < u64::MAX && self.navigation.interaction < u64::MAX
    }
    pub(super) fn open_history_resource(&mut self, travel: super::navigation_history::Travel) {
        let path = travel.resource().unwrap().to_owned();
        self.open_navigation_mode(path, None, false, OpenIntent::History(travel));
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
            let Some(next) = self.navigation.sequence.checked_add(1) else {
                return;
            };
            self.navigation.sequence = next;
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
    pub(super) fn record_closed_tab(&mut self, member: crate::editor_groups::Membership) {
        if let Some(doc) = self.documents.iter().find(|doc| doc.id == member.document)
            && let Some(path) = &doc.path
        {
            let cursor = doc.view_state(Some(member.group.value())).cursor;
            let row = doc.text.char_to_line(cursor);
            let column =
                doc.display_width_slice(doc.text.slice(doc.text.line_to_char(row)..cursor));
            let Some(next) = self.navigation.sequence.checked_add(1) else {
                return;
            };
            self.navigation.sequence = next;
            self.navigation.closed.push(Closed {
                id: next,
                path: path.clone(),
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
    fn focus_existing_navigation(&mut self, path: &Path) -> Result<bool> {
        if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|doc| doc.path.as_deref() == Some(path))
        {
            self.can_admit_editor(self.hidden_documents[index].id)?;
            let doc = self.hidden_documents.remove(index);
            self.install_open_document(doc)?;
            return Ok(true);
        }
        let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.path.as_deref() == Some(path))
        else {
            return Ok(false);
        };
        self.can_admit_editor(self.documents[index].id)?;
        self.active = index;
        self.focus = Focus::Editor;
        self.sync_pane();
        self.remember_active_file();
        self.message = format!("Focused {}", self.doc().name());
        Ok(true)
    }
    fn focus_existing_intent(&mut self, path: &Path, intent: &OpenIntent) -> Result<bool> {
        let Some(doc) = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.path.as_deref() == Some(path))
        else {
            return Ok(false);
        };
        intent.validate(doc)?;
        if let OpenIntent::History(travel) = intent {
            let document = doc.id;
            self.apply_history_target(travel, document)?;
            return Ok(true);
        }
        let suspended = self.suspend_navigation_observation();
        match self.focus_existing_navigation(path) {
            Ok(true) => {
                intent.apply(self);
                self.resume_navigation_observation(
                    suspended,
                    super::navigation_history::Reason::Jump,
                );
                Ok(true)
            }
            Ok(false) => {
                self.resume_navigation_observation(
                    suspended,
                    super::navigation_history::Reason::Ordinary,
                );
                Ok(false)
            }
            Err(error) => {
                self.resume_navigation_observation(
                    suspended,
                    super::navigation_history::Reason::Ordinary,
                );
                Err(error)
            }
        }
    }
    pub(super) fn open_hidden_aware(&mut self, path: PathBuf, intent: OpenIntent) {
        self.open_navigation_mode(path, None, true, intent);
    }
    pub(super) fn open_welcome_recent(&mut self, path: PathBuf) {
        self.open_navigation(path, None);
    }
    fn open_navigation(&mut self, path: PathBuf, closed: Option<Closed>) {
        self.open_navigation_mode(path, closed, false, OpenIntent::Focus);
    }
    fn open_navigation_mode(
        &mut self,
        path: PathBuf,
        closed: Option<Closed>,
        allow_missing: bool,
        intent: OpenIntent,
    ) {
        match self.focus_existing_intent(&path, &intent) {
            Ok(true) => {
                self.finish_reopen(closed.as_ref());
                self.cancel_navigation();
                return;
            }
            Err(error) => {
                if let OpenIntent::History(travel) = &intent {
                    self.history_travel_failed(travel);
                }
                self.message = format!("Navigation target rejected: {error:#}");
                return;
            }
            Ok(false) => {}
        }
        if self.navigation.pending.is_some() {
            self.message = "A recent file is still loading; retry shortly".into();
            return;
        }
        if !matches!(intent, OpenIntent::History(_)) {
            self.cancel_navigation();
        }
        if !self.navigation_channel_available() {
            if let OpenIntent::History(travel) = &intent {
                self.history_travel_failed(travel);
            }
            self.message = "Navigation interaction limit reached; restart the editor".into();
            return;
        }
        if self
            .documents
            .len()
            .saturating_add(self.hidden_documents.len())
            > 128
            || self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .filter_map(|doc| doc.path.as_ref())
                .any(|path| path.as_os_str().len() > 4096)
        {
            if let OpenIntent::History(travel) = &intent {
                self.history_travel_failed(travel);
            }
            self.message =
                "File navigation loading supports at most 128 models with 4 KiB paths".into();
            return;
        }
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
            intent,
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
            return self.dispatch_queued_history() || changed;
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
            if let OpenIntent::History(travel) = &pending.intent {
                self.history_travel_failed(travel);
            }
            return self.dispatch_queued_history() || changed;
        }
        let result = result
            .map_err(anyhow::Error::msg)
            .and_then(|target| -> Result<()> {
                match target {
                    Target::Existing(path) => {
                        if !self.focus_existing_intent(&path, &pending.intent)? {
                            anyhow::bail!(
                                "Resolved native buffer was closed before navigation completed"
                            );
                        }
                    }
                    Target::Loaded(mut doc) => {
                        if !self
                            .focus_existing_intent(doc.path.as_ref().unwrap(), &pending.intent)?
                        {
                            pending.intent.validate(&doc)?;
                            if let OpenIntent::History(travel) = &pending.intent {
                                return self.install_history_document(travel, *doc);
                            }
                            self.settings.apply(&mut doc);
                            if let Some(closed) = &pending.closed {
                                doc.move_to(doc.position_at(closed.row, closed.column), false);
                            }
                            let suspended = self.suspend_navigation_observation();
                            if let Err(error) = self.install_open_document(*doc) {
                                self.resume_navigation_observation(
                                    suspended,
                                    super::navigation_history::Reason::Ordinary,
                                );
                                return Err(error);
                            }
                            pending.intent.apply(self);
                            self.resume_navigation_observation(
                                suspended,
                                super::navigation_history::Reason::Jump,
                            );
                        }
                    }
                }
                self.finish_reopen(pending.closed.as_ref());
                Ok(())
            });
        if let Err(error) = result {
            if let OpenIntent::History(travel) = &pending.intent {
                self.history_travel_failed(travel);
            }
            self.message = format!("File could not be opened (history retained): {error:#}");
        }
        self.dispatch_queued_history() || changed
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
    fn settle_saves(app: &mut App, phase: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.saves_pending() {
            app.poll();
            if !app.saves_pending() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{phase}: {}",
                app.save_fixture_status(),
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn deferred_settings_search_debug_and_location_never_mutate_the_previous_editor() {
        let root = tempfile::tempdir().unwrap();
        let held = root.path().join("hidden.txt");
        std::fs::write(&held, "hidden").unwrap();
        let target = root.path().join("target.txt");
        std::fs::write(&target, "α😀 hit\r\nsecond").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.documents.push(Document::default());
        app.documents[0].saved_revision = u64::MAX;
        app.hidden_documents
            .push(Document::open_existing(&held).unwrap());
        app.sync_pane();
        let previous = app.doc().id;
        let settings = root.path().join("settings.json");
        app.settings_user = Some(settings.clone());
        app.execute("workbench.action.openSettingsJson", Value::Null);
        assert_eq!(app.doc().id, previous);
        assert!(app.doc().is_empty());
        wait(&mut app);
        assert_eq!(
            app.doc().path.as_ref(),
            Some(
                &std::fs::canonicalize(root.path())
                    .unwrap()
                    .join("settings.json")
            )
        );
        assert_eq!(app.doc().text.to_string(), "{\n}\n");
        assert!(
            app.documents
                .iter()
                .find(|doc| doc.id == previous)
                .unwrap()
                .is_empty()
        );
        let settings_id = app.doc().id;
        let settings_cursor = app.doc().cursor;
        let range = crate::lsp::Range {
            start: crate::lsp::Position {
                line: 0,
                character: 1,
            },
            end: crate::lsp::Position {
                line: 0,
                character: 3,
            },
        };
        app.language_action(&LanguageAction::Location {
            path: target.clone(),
            range,
        })
        .unwrap();
        assert_eq!(app.doc().id, settings_id);
        assert_eq!(app.doc().cursor, settings_cursor);
        wait(&mut app);
        assert_eq!(app.doc().selected_text().as_deref(), Some("😀"));
        let target_id = app.doc().id;
        let invalid = crate::lsp::Range {
            start: crate::lsp::Position {
                line: 0,
                character: 2,
            },
            end: crate::lsp::Position {
                line: 0,
                character: 3,
            },
        };
        app.active = app
            .documents
            .iter()
            .position(|doc| doc.id == settings_id)
            .unwrap();
        app.sync_pane();
        assert!(
            app.language_action(&LanguageAction::Location {
                path: app
                    .documents
                    .iter()
                    .find(|doc| doc.id == target_id)
                    .unwrap()
                    .path
                    .clone()
                    .unwrap(),
                range: invalid
            })
            .is_err()
        );
        assert_eq!(app.doc().id, settings_id);
        assert_eq!(app.doc().cursor, settings_cursor);
        let invalid_file = root.path().join("invalid.txt");
        std::fs::write(&invalid_file, "α😀").unwrap();
        let count = app.documents.len();
        app.language_action(&LanguageAction::Location {
            path: invalid_file,
            range: invalid,
        })
        .unwrap();
        wait(&mut app);
        assert_eq!(app.doc().id, settings_id);
        assert_eq!(app.doc().cursor, settings_cursor);
        assert_eq!(app.documents.len(), count);
        let other = root.path().join("other.txt");
        std::fs::write(&other, "prefix match\nlast").unwrap();
        let hit = crate::search::Hit {
            path: other.clone(),
            row: 0,
            column: 7,
            length: 5,
            line: "prefix match".into(),
            line_hash: crate::search::line_hash("prefix match"),
        };
        app.open_with_intent(&other, OpenIntent::Search(hit))
            .unwrap();
        assert_eq!(app.doc().id, settings_id);
        wait(&mut app);
        assert_eq!(app.doc().selected_text().as_deref(), Some("match"));
        let debug = root.path().join("debug.cpp");
        std::fs::write(&debug, "first\nsecond").unwrap();
        let source = app.doc().id;
        app.open_with_intent(&debug, OpenIntent::Debug { line: 2, column: 3 })
            .unwrap();
        assert_eq!(app.doc().id, source);
        wait(&mut app);
        assert_eq!(app.doc().row(), 1);
        assert_eq!(app.doc().cursor, 8);
        assert_eq!(
            app.documents
                .iter()
                .find(|doc| doc.id == settings_id)
                .unwrap()
                .text
                .to_string(),
            "{\n}\n"
        );
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "α😀 hit\r\nsecond"
        );
        assert!(!settings.exists());
    }
    #[test]
    fn changed_editor_cancels_deferred_settings_initialization() {
        let root = tempfile::tempdir().unwrap();
        let held = root.path().join("held.txt");
        std::fs::write(&held, "held").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.documents.push(Document::default());
        app.hidden_documents
            .push(Document::open_existing(&held).unwrap());
        app.sync_pane();
        let previous = app.doc().id;
        let settings = root.path().join("settings.json");
        app.settings_user = Some(settings.clone());
        app.execute("workbench.action.openSettingsJson", Value::Null);
        app.doc_mut().insert("newer", false);
        wait(&mut app);
        assert_eq!(app.doc().id, previous);
        assert_eq!(app.doc().text.to_string(), "newer");
        assert!(!settings.exists());
        assert_eq!(app.documents.len(), 1);
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
        settle_saves(&mut app, "existing-file close-save");
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
        settle_saves(&mut app, "untitled SaveAs close-save");
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

#[cfg(test)]
mod group_tabs_tests;
