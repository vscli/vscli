//! Session-only locations. History never owns document content or Undo state.
use super::*;
use std::sync::Arc;

const LIMIT: usize = 50;
const PATH_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reason {
    Ordinary,
    Jump,
    EditorChange,
    HistoryTravel,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn untitled(root: &Path, text: &str) -> App {
        let mut app = App::new(root.into(), Profile::Linux);
        app.documents.push(Document::from_text(text));
        app.sync_pane();
        app.observe_navigation(Reason::EditorChange);
        app
    }
    fn jump(app: &mut App, line: usize) {
        let at = app.doc().line_start(line);
        app.navigation_input_interaction();
        app.doc_mut().move_to(at, false);
        app.observe_navigation(Reason::Jump);
    }
    #[test]
    fn nearby_motion_coalesces_and_new_jump_truncates_forward_at_fifty_entries() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = untitled(directory.path(), &"x\n".repeat(700));
        app.doc_mut().move_to(2, false);
        app.observe_navigation(Reason::Ordinary);
        assert_eq!(app.navigation_history.entries.len(), 1);
        jump(&mut app, 2);
        assert_eq!(app.navigation_history.entries.len(), 2);
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().row(), 1);
        assert!(app.can_navigate_forward());
        jump(&mut app, 3);
        assert!(!app.can_navigate_forward());
        for line in (20..=610).step_by(10) {
            jump(&mut app, line);
        }
        assert_eq!(app.navigation_history.entries.len(), LIMIT);
        assert_eq!(app.navigation_history.current, Some(LIMIT - 1));
        assert_eq!(
            app.navigation_history
                .entries
                .last()
                .unwrap()
                .location
                .cursor
                .line,
            610
        );
        for _ in 0..LIMIT - 1 {
            app.navigate_history(Direction::Back);
        }
        assert!(!app.can_navigate_back());
        assert!(app.can_navigate_forward());
    }
    #[test]
    fn utf16_history_normalizes_primary_range_and_clamps_without_scanning_long_lines() {
        let directory = tempfile::tempdir().unwrap();
        let before = format!("{}猫🙂\r\nsecond\r\n", "a".repeat(1024 * 1024));
        let mut app = untitled(directory.path(), &before);
        let start = 1024 * 1024;
        app.doc_mut()
            .set_selections(vec![crate::document::Selection {
                anchor: Some(start + 2),
                cursor: start,
                desired_column: None,
            }]);
        app.observe_navigation(Reason::Ordinary);
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        jump(&mut app, 1);
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().cursor, start + 2);
        assert_eq!(app.doc().anchor, Some(start));
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), before);
        let doc = Document::from_text("猫🙂\r\n");
        assert_eq!(
            offset(
                &doc,
                crate::lsp::Position {
                    line: 0,
                    character: 2
                }
            ),
            1
        );
        assert_eq!(
            offset(
                &doc,
                crate::lsp::Position {
                    line: 0,
                    character: usize::MAX
                }
            ),
            2
        );
        assert_eq!(
            offset(
                &doc,
                crate::lsp::Position {
                    line: usize::MAX,
                    character: usize::MAX
                }
            ),
            doc.len()
        );
    }
    #[test]
    fn hidden_dirty_deleted_model_is_promoted_with_identity_disk_and_undo_intact() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.cpp");
        let second = directory.path().join("second.cpp");
        let initial = "猫🙂\r\n";
        std::fs::write(&first, initial).unwrap();
        std::fs::write(&second, "second\r\n").unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.open(&first).unwrap();
        app.doc_mut().move_to(1, false);
        app.doc_mut().insert("dirty", false);
        app.observe_navigation(Reason::Ordinary);
        let document = app.doc().id;
        let epoch = app.doc().text_epoch();
        let selection = app.doc().selections();
        app.open(&second).unwrap();
        let index = app
            .documents
            .iter()
            .position(|doc| doc.id == document)
            .unwrap();
        app.hidden_documents.push(app.documents.remove(index));
        app.active = 0;
        app.sync_pane();
        std::fs::remove_file(&first).unwrap();
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().id, document);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().selections(), selection);
        assert_eq!(app.doc().text.to_string(), "猫dirty🙂\r\n");
        assert!(app.doc().dirty());
        assert!(!first.exists());
        assert!(app.hidden_documents.is_empty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), initial);
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "猫dirty🙂\r\n");
        assert!(!first.exists());
    }
    #[cfg(unix)]
    #[test]
    fn retired_identity_and_parent_alias_reuse_the_new_hidden_dirty_model_without_disk() {
        use std::time::{Duration, Instant};
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.cpp");
        let second = directory.path().join("second.cpp");
        std::fs::write(&first, "猫🙂\r\n").unwrap();
        std::fs::write(&second, "second\r\n").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(directory.path(), &alias).unwrap();
        let aliased = alias.join("first.cpp");
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.open(&first).unwrap();
        let retired = app.doc().id;
        app.doc_mut().path = Some(aliased);
        app.doc_mut().move_to(1, false);
        app.observe_navigation(Reason::Ordinary);
        app.open(&second).unwrap();
        let old = app
            .documents
            .iter()
            .position(|doc| doc.id == retired)
            .unwrap();
        app.documents.remove(old);
        app.active = 0;
        app.sync_pane();
        let mut hidden = Document::open_existing(&first).unwrap();
        hidden.move_to(1, false);
        hidden.insert("dirty", false);
        let id = hidden.id;
        let epoch = hidden.text_epoch();
        app.hidden_documents.push(hidden);
        std::fs::remove_file(&first).unwrap();
        app.navigate_history(Direction::Back);
        assert!(app.history_loader_busy());
        let until = Instant::now() + Duration::from_secs(3);
        while app.history_loader_busy() {
            assert!(Instant::now() < until, "{}", app.message);
            app.poll_navigation();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), "猫dirty🙂\r\n");
        assert!(app.doc().dirty());
        assert!(!first.exists());
        assert_eq!(app.documents.len(), 2);
        assert!(app.hidden_documents.is_empty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\n");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "猫dirty🙂\r\n");
        assert!(!first.exists());
    }
    #[test]
    fn recorded_pane_selection_restores_primary_without_touching_other_shared_view() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = untitled(directory.path(), "α\r\n猫🙂\r\n");
        app.doc_mut()
            .set_selections(vec![crate::document::Selection {
                anchor: Some(5),
                cursor: 3,
                desired_column: None,
            }]);
        app.observe_navigation(Reason::Ordinary);
        let first = app.panes[0].id;
        app.split_editor(false);
        app.doc_mut().move_to(1, false);
        app.observe_navigation(Reason::EditorChange);
        let second = app.panes[1].id;
        let epoch = app.doc().text_epoch();
        app.navigate_history(Direction::Back);
        assert_eq!(app.panes[app.active_pane].id, first);
        assert_eq!(app.doc().cursor, 5);
        assert_eq!(app.doc().anchor, Some(3));
        app.navigate_history(Direction::Forward);
        assert_eq!(app.panes[app.active_pane].id, second);
        assert_eq!(app.doc().cursor, 1);
        assert_eq!(app.doc().anchor, None);
        assert_eq!(app.doc().text_epoch(), epoch);
    }
    #[test]
    fn save_as_uses_current_live_path_and_closed_untitled_is_not_recreated() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = untitled(directory.path(), "猫🙂\r\n");
        let document = app.doc().id;
        let saved = directory.path().join("saved.cpp");
        app.doc_mut().save_to(&saved, false).unwrap();
        let saved = std::fs::canonicalize(saved).unwrap();
        let second = directory.path().join("second.cpp");
        std::fs::write(&second, "second\r\n").unwrap();
        let second = std::fs::canonicalize(second).unwrap();
        app.open(&second).unwrap();
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().id, document);
        assert_eq!(app.doc().path.as_deref(), Some(saved.as_path()));
        assert_eq!(
            app.navigation_history.entries[0]
                .location
                .resource
                .as_deref()
                .map(PathBuf::as_path),
            Some(saved.as_path())
        );
        let mut app = untitled(directory.path(), "discarded");
        let discarded = app.doc().id;
        app.open(&second).unwrap();
        let index = app
            .documents
            .iter()
            .position(|doc| doc.id == discarded)
            .unwrap();
        app.documents.remove(index);
        app.active = 0;
        app.sync_pane();
        app.observe_navigation(Reason::Ordinary);
        app.navigate_history(Direction::Back);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().path.as_deref(), Some(second.as_path()));
        assert!(!app.can_navigate_back());
    }
    #[test]
    fn workspace_round_trip_retires_session_locations_without_changing_the_model() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = untitled(directory.path(), &"猫🙂\r\n".repeat(20));
        jump(&mut app, 10);
        let id = app.doc().id;
        let before = app.doc().text.to_string();
        assert!(app.can_navigate_back());
        let root = app.workspace.root.clone();
        app.workspace.root = root.join("other");
        assert!(!app.can_navigate_back());
        app.observe_navigation(Reason::Ordinary);
        app.workspace.root = root;
        app.observe_navigation(Reason::Ordinary);
        assert_eq!(app.navigation_history.entries.len(), 1);
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), before);
        assert_eq!(app.doc().row(), 10);
        assert!(!app.can_navigate_back());
        assert!(!app.can_navigate_forward());
    }
    #[test]
    fn save_as_updates_older_locations_before_closed_model_reopen() {
        use std::time::{Duration, Instant};
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.cpp");
        let saved = directory.path().join("saved.cpp");
        let other = directory.path().join("other.cpp");
        let initial = "猫🙂\r\n".repeat(20);
        std::fs::write(&original, &initial).unwrap();
        std::fs::write(&other, "other\r\n").unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.open(&original).unwrap();
        let model = app.doc().id;
        jump(&mut app, 10);
        app.doc_mut().insert("saved dirty ", false);
        let expected = app.doc().text.to_string();
        app.doc_mut().save_to(&saved, false).unwrap();
        let saved = std::fs::canonicalize(saved).unwrap();
        app.open(&other).unwrap();
        assert!(
            app.navigation_history
                .entries
                .iter()
                .filter(|entry| entry.location.document == model)
                .all(
                    |entry| entry.location.resource.as_deref().map(PathBuf::as_path)
                        == Some(saved.as_path())
                )
        );
        let index = app
            .documents
            .iter()
            .position(|doc| doc.id == model)
            .unwrap();
        app.documents.remove(index);
        app.active = 0;
        app.sync_pane();
        app.navigate_history(Direction::Back);
        let until = Instant::now() + Duration::from_secs(3);
        while app.history_loader_busy() {
            assert!(Instant::now() < until, "{}", app.message);
            app.poll_navigation();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(app.doc().path.as_deref(), Some(saved.as_path()));
        assert_eq!(app.doc().text.to_string(), expected);
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().path.as_deref(), Some(saved.as_path()));
        assert_eq!(std::fs::read(&original).unwrap(), initial.as_bytes());
        assert_eq!(std::fs::read(&saved).unwrap(), expected.as_bytes());
    }
    #[test]
    fn cross_file_location_jump_records_final_target_once_without_default_cursor_entry() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.cpp");
        let second = directory.path().join("second.cpp");
        std::fs::write(&first, "source\r\n").unwrap();
        std::fs::write(&second, "猫🙂\r\n".repeat(20)).unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.open(&first).unwrap();
        let source = app.doc().id;
        let position = crate::lsp::Position {
            line: 10,
            character: 1,
        };
        app.language_action(&LanguageAction::Location {
            path: second,
            range: crate::lsp::Range {
                start: position,
                end: position,
            },
        })
        .unwrap();
        let target = app.doc().id;
        assert_eq!(app.doc().row(), 10);
        assert_eq!(app.navigation_history.entries.len(), 2);
        app.navigate_history(Direction::Back);
        assert_eq!(app.doc().id, source);
        app.navigate_history(Direction::Forward);
        assert_eq!(app.doc().id, target);
        assert_eq!(app.doc().row(), 10);
        assert_eq!(app.doc().column(), 1);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Back,
    Forward,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Location {
    document: u64,
    resource: Option<Arc<PathBuf>>,
    pane: u64,
    anchor: Option<crate::lsp::Position>,
    cursor: crate::lsp::Position,
}
impl Location {
    fn first_line(&self) -> usize {
        self.anchor
            .map_or(self.cursor.line, |a| a.line.min(self.cursor.line))
    }
    fn capture(app: &App, prior: Option<&Self>) -> Option<Self> {
        let doc = app.active_document()?;
        let pane = app.panes.get(app.active_pane)?.id;
        let resource = doc
            .path
            .as_ref()
            .filter(|p| p.as_os_str().len() <= PATH_BYTES)
            .map(|path| {
                prior
                    .and_then(|p| p.resource.as_ref())
                    .filter(|p| p.as_ref() == path)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(path.clone()))
            });
        Some(Self {
            document: doc.id,
            resource,
            pane,
            anchor: doc.anchor.map(|at| position(doc, at)),
            cursor: position(doc, doc.cursor),
        })
    }
}
#[derive(Clone)]
struct Entry {
    id: u64,
    location: Location,
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    document: u64,
    pane: u64,
    models: usize,
    epoch: u64,
    saved: u64,
    cursor: usize,
    anchor: Option<usize>,
}
impl Stamp {
    fn capture(app: &App) -> Option<Self> {
        let doc = app.active_document()?;
        Some(Self {
            document: doc.id,
            pane: app.panes.get(app.active_pane)?.id,
            models: app
                .documents
                .len()
                .saturating_add(app.hidden_documents.len()),
            epoch: doc.text_epoch(),
            saved: doc.save_generation(),
            cursor: doc.cursor,
            anchor: doc.anchor,
        })
    }
}
#[derive(Clone)]
pub(super) struct Travel {
    token: u64,
    entry: u64,
    location: Location,
    pub(super) context: super::navigation::Context,
}
impl Travel {
    pub(super) fn resource(&self) -> Option<&Path> {
        self.location.resource.as_deref().map(PathBuf::as_path)
    }
}
#[derive(Default)]
pub(super) struct State {
    workspace: Option<Arc<PathBuf>>,
    entries: Vec<Entry>,
    current: Option<usize>,
    next_entry: u64,
    next_ticket: u64,
    desired: Option<Travel>,
    queued: Option<Travel>,
    suppressed: bool,
    observed: Option<Stamp>,
}

// Rope's UTF-16 counters avoid traversing a very long line on each observation.
fn position(doc: &Document, at: usize) -> crate::lsp::Position {
    let at = at.min(doc.len());
    let line = doc.text.char_to_line(at);
    crate::lsp::Position {
        line,
        character: doc.text.char_to_utf16_cu(at) - doc.text.char_to_utf16_cu(doc.line_start(line)),
    }
}
fn offset(doc: &Document, position: crate::lsp::Position) -> usize {
    let row = position.line.min(doc.line_count() - 1);
    let start = doc.line_start(row);
    let end = doc.line_end(row);
    let first = doc.text.char_to_utf16_cu(start);
    let count = doc.text.char_to_utf16_cu(end) - first;
    // A changed line can leave the old column inside a surrogate pair. Floor
    // to its scalar start rather than exposing an invalid native position.
    doc.text
        .utf16_cu_to_char(first + position.character.min(count))
}

impl App {
    pub fn can_navigate_back(&self) -> bool {
        self.navigation_channel_available()
            && self.navigation_history.workspace.as_deref() == Some(&self.workspace.root)
            && self
                .navigation_history
                .current
                .is_some_and(|index| index > 0)
    }
    pub fn can_navigate_forward(&self) -> bool {
        self.navigation_channel_available()
            && self.navigation_history.workspace.as_deref() == Some(&self.workspace.root)
            && self
                .navigation_history
                .current
                .is_some_and(|index| index + 1 < self.navigation_history.entries.len())
    }
    pub(super) fn history_navigation_cancelled(&mut self) {
        self.navigation_history.desired = None;
        self.navigation_history.queued = None;
    }
    pub(super) fn suspend_navigation_observation(&mut self) -> bool {
        let previous = self.navigation_history.suppressed;
        self.navigation_history.suppressed = true;
        previous
    }
    pub(super) fn resume_navigation_observation(&mut self, previous: bool, reason: Reason) {
        self.navigation_history.suppressed = previous;
        self.observe_navigation(reason);
    }
    pub(super) fn observe_navigation(&mut self, reason: Reason) {
        if self.navigation_history.workspace.as_deref() != Some(&self.workspace.root) {
            self.cancel_navigation();
            self.navigation_history.workspace = Some(Arc::new(self.workspace.root.clone()));
            self.navigation_history.entries.clear();
            self.navigation_history.current = None;
            self.navigation_history.observed = None;
        }
        if reason == Reason::HistoryTravel
            || self.navigation_history.suppressed
            || self.navigation_history.desired.is_some()
            || self.focus != Focus::Editor
        {
            return;
        }
        let Some(stamp) = Stamp::capture(self) else {
            self.prune_navigation_history();
            self.navigation_history.observed = None;
            return;
        };
        if self.navigation_history.observed.as_ref() == Some(&stamp) && reason != Reason::Jump {
            return;
        }
        self.prune_navigation_history();
        let prior = self
            .navigation_history
            .current
            .and_then(|index| self.navigation_history.entries.get(index));
        let Some(location) = Location::capture(self, prior.map(|e| &e.location)) else {
            return;
        };
        let replace = prior.is_some_and(|prior| {
            let prior = &prior.location;
            prior.document == location.document && prior.pane == location.pane && {
                let distance = prior.first_line().abs_diff(location.first_line());
                distance == 0 || (distance < 10 && reason != Reason::Jump)
            }
        });
        let state = &mut self.navigation_history;
        state.observed = Some(stamp);
        // Save As changes the resource of this model, including older history
        // locations in other panes. Preserve their selections and model ID.
        for entry in &mut state.entries {
            if entry.location.document == location.document {
                entry.location.resource = location.resource.clone();
            }
        }
        if replace {
            state.entries[state.current.unwrap()].location = location;
        } else if let Some(id) = state.next_entry.checked_add(1) {
            state.next_entry = id;
            state
                .entries
                .truncate(state.current.map_or(0, |index| index + 1));
            state.entries.push(Entry { id, location });
            if state.entries.len() > LIMIT {
                state.entries.remove(0);
            }
            state.current = Some(state.entries.len() - 1);
        } else {
            self.message = "Navigation history ticket limit reached".into();
        }
    }
    fn prune_navigation_history(&mut self) {
        let current = self
            .navigation_history
            .current
            .and_then(|i| self.navigation_history.entries.get(i))
            .map(|e| e.id);
        self.navigation_history.entries.retain(|entry| {
            entry.location.resource.is_some()
                || self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .any(|doc| doc.id == entry.location.document)
        });
        self.navigation_history.current = current
            .and_then(|id| {
                self.navigation_history
                    .entries
                    .iter()
                    .position(|e| e.id == id)
            })
            .or_else(|| {
                (!self.navigation_history.entries.is_empty())
                    .then(|| self.navigation_history.entries.len() - 1)
            });
    }
    pub(super) fn navigate_history(&mut self, direction: Direction) {
        self.observe_navigation(Reason::Ordinary);
        self.prune_navigation_history();
        if !self.navigation_channel_available() {
            self.message = "Navigation interaction limit reached; restart the editor".into();
            return;
        }
        let state = &self.navigation_history;
        let base = state
            .desired
            .as_ref()
            .and_then(|travel| state.entries.iter().position(|e| e.id == travel.entry))
            .or(state.current);
        let target = base.and_then(|index| match direction {
            Direction::Back => index.checked_sub(1),
            Direction::Forward => (index + 1 < state.entries.len()).then_some(index + 1),
        });
        let Some(entry) = target.and_then(|index| state.entries.get(index)).cloned() else {
            self.message = match direction {
                Direction::Back => "No previous navigation location",
                Direction::Forward => "No next navigation location",
            }
            .into();
            return;
        };
        let Some(token) = state.next_ticket.checked_add(1) else {
            self.message = "Navigation history ticket limit reached".into();
            return;
        };
        self.cancel_navigation();
        self.navigation_history.next_ticket = token;
        let travel = Travel {
            token,
            entry: entry.id,
            location: entry.location,
            context: self.navigation_context(),
        };
        self.navigation_history.desired = Some(travel.clone());
        self.dispatch_history_travel(travel);
    }
    fn history_travel_current(&self, travel: &Travel) -> bool {
        self.navigation_channel_available()
            && self
                .navigation_history
                .desired
                .as_ref()
                .is_some_and(|current| current.token == travel.token)
            && travel.context == self.navigation_context()
            && self.prompt.is_none()
            && self.modal.is_none()
            && self
                .navigation_history
                .entries
                .iter()
                .any(|entry| entry.id == travel.entry)
    }
    fn dispatch_history_travel(&mut self, travel: Travel) {
        if !self.history_travel_current(&travel) {
            self.history_travel_failed(&travel);
            return;
        }
        let live = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == travel.location.document)
            .or_else(|| {
                travel.resource().and_then(|path| {
                    self.documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .find(|doc| doc.path.as_deref() == Some(path))
                })
            })
            .map(|doc| doc.id);
        if let Some(document) = live {
            if let Err(error) = self.apply_history_target(&travel, document) {
                self.history_travel_failed(&travel);
                self.message = format!("Navigation history: {error:#}");
            }
        } else if travel.resource().is_none() {
            self.history_travel_failed(&travel);
            self.prune_navigation_history();
            self.message = "The untitled navigation target was closed".into();
        } else if self.history_loader_busy() {
            self.navigation_history.queued = Some(travel);
            self.message = "Navigation queued until the current file load finishes".into();
        } else {
            self.open_history_resource(travel);
        }
    }
    pub(super) fn dispatch_queued_history(&mut self) -> bool {
        if self.history_loader_busy() {
            return false;
        }
        let Some(travel) = self.navigation_history.queued.take() else {
            return false;
        };
        self.dispatch_history_travel(travel);
        true
    }
    pub(super) fn history_travel_failed(&mut self, travel: &Travel) {
        if self
            .navigation_history
            .desired
            .as_ref()
            .is_some_and(|current| current.token == travel.token)
        {
            self.navigation_history.desired = None;
        }
    }
    pub(super) fn apply_history_target(&mut self, travel: &Travel, document: u64) -> Result<()> {
        if !self.history_travel_current(travel) {
            anyhow::bail!("Navigation history context changed");
        }
        if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|doc| doc.id == document)
        {
            let language = self.hidden_documents[index]
                .path
                .as_deref()
                .map_or("plaintext", crate::languages::language);
            let configuration = self.language_configuration(language);
            self.hidden_documents[index].set_language_configuration(configuration)?;
        }
        let visible = self.documents.iter().position(|doc| doc.id == document);
        let hidden = self
            .hidden_documents
            .iter()
            .position(|doc| doc.id == document);
        if visible.is_none() && hidden.is_none() {
            anyhow::bail!("Navigation history model was closed");
        }
        let target_group = self
            .editor_groups
            .groups()
            .iter()
            .find(|group| group.id().value() == travel.location.pane)
            .map(|group| group.id());
        // One engine operation selects both target group and tab. Admission
        // and checked counters complete before any model/view publication.
        let change = if self.group_fallback {
            None
        } else if let Some(group) = target_group {
            Some(self.editor_groups.open_in_group(group, document)?)
        } else {
            Some(self.editor_groups.open(document)?)
        };
        let suspended = self.suspend_navigation_observation();
        let index = visible.unwrap_or_else(|| {
            self.documents
                .push(self.hidden_documents.remove(hidden.unwrap()));
            self.documents.len() - 1
        });
        self.active = index;
        self.focus = Focus::Editor;
        if let Some(change) = change {
            self.apply_group_change(change);
        } else {
            if let Some(pane) = self
                .panes
                .iter()
                .position(|pane| pane.id == travel.location.pane)
            {
                self.active_pane = pane;
            }
            self.sync_pane();
        }
        let cursor = offset(self.doc(), travel.location.cursor);
        let anchor = travel
            .location
            .anchor
            .map(|anchor| offset(self.doc(), anchor));
        // The pinned workbench restores history selections as forward ranges,
        // independently of the original editor's selection direction.
        let (cursor, anchor) = anchor.map_or((cursor, None), |anchor| {
            (cursor.max(anchor), Some(cursor.min(anchor)))
        });
        self.doc_mut()
            .set_selections(vec![crate::document::Selection {
                cursor,
                anchor,
                desired_column: None,
            }]);
        self.remember_active_file();
        let location = Location::capture(self, Some(&travel.location)).unwrap();
        let index = self
            .navigation_history
            .entries
            .iter()
            .position(|entry| entry.id == travel.entry)
            .unwrap();
        self.navigation_history.entries[index].location = location;
        self.navigation_history.current = Some(index);
        self.navigation_history.observed = Stamp::capture(self);
        self.navigation_history.desired = None;
        self.navigation_history.queued = None;
        self.resume_navigation_observation(suspended, Reason::HistoryTravel);
        self.message = format!("Navigation history · {}", self.doc().name());
        Ok(())
    }
    pub(super) fn install_history_document(
        &mut self,
        travel: &Travel,
        mut doc: Document,
    ) -> Result<()> {
        if !self.history_travel_current(travel) {
            anyhow::bail!("Navigation history context changed");
        }
        if !self.group_fallback {
            if let Some(group) = self
                .editor_groups
                .groups()
                .iter()
                .find(|group| group.id().value() == travel.location.pane)
            {
                self.editor_groups.can_open_in_group(group.id(), doc.id)?;
            } else {
                self.can_admit_editor(doc.id)?;
            }
        }
        self.settings.apply(&mut doc);
        self.configure_document_language(&mut doc)?;
        let document = doc.id;
        self.documents.push(doc);
        self.apply_history_target(travel, document)
    }
}
