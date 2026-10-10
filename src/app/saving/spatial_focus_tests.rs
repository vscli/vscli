//! Topology focus with real native models, presentation and save-worker receipts.
//! Uses actual save-worker phase gates, never inferred receipt timing.
use super::*;
use crate::{editor_groups::Membership, keys::Profile, save_worker::GatePoint};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::{
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

const A_DISK: &str = "猫🙂 A baseline\r\nlast\r\n";
const B_DISK: &str = "B λ baseline\r\nlast\r\n";
const C_DISK: &str = "C e\u{301} baseline\r\nlast\r\n";
const WAIT: Duration = Duration::from_secs(5);
const GROUP_CLOSE: &str = "workbench.action.closeEditorsInGroup";

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    a: PathBuf,
    b: PathBuf,
    c: PathBuf,
    a_id: u64,
    b_id: u64,
    c_id: u64,
}
impl Fixture {
    fn new() -> Self {
        Self::with_policy(None)
    }
    fn with_policy(policy: Option<&str>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let a = root.join("a.cpp");
        let b = root.join("b.cpp");
        let c = root.join("c.cpp");
        for (path, text) in [(&a, A_DISK), (&b, B_DISK), (&c, C_DISK)] {
            std::fs::write(path, text).unwrap();
        }
        let mut app = App::new(root, Profile::Linux);
        app.extension_node = "vscli-spatial-fixture-node-missing".into();
        app.sidebar = false;
        let mut settings = json!({
            "vscli.languageServer.enabled":false,
            "breadcrumbs.enabled":false,
            "files.autoSave":"off",
            "editor.formatOnSave":false,
            "editor.codeActionsOnSave":{}
        })
        .as_object()
        .unwrap()
        .clone();
        if let Some(policy) = policy {
            settings.insert(
                "workbench.editor.preventPinnedEditorClose".into(),
                json!(policy),
            );
        }
        app.settings =
            crate::settings::Settings::from_values(settings, "sticky actual-worker fixture")
                .unwrap();
        let mut ids = Vec::new();
        for path in [&a, &b, &c] {
            app.open(path).unwrap();
            until(&mut app, "fixture open", |app| {
                app.active_document()
                    .is_some_and(|doc| doc.path.as_ref() == Some(path))
            });
            ids.push(app.doc().id);
        }
        let a_id = ids[0];
        let member = app.editor_groups.memberships(a_id).next().unwrap();
        app.focus_tab(member).unwrap();
        Self {
            _directory: directory,
            app,
            a,
            b,
            c,
            a_id,
            b_id: ids[1],
            c_id: ids[2],
        }
    }
    fn member(&self, document: u64) -> Membership {
        self.app.editor_groups.memberships(document).next().unwrap()
    }
    fn focus(&mut self, member: Membership) {
        self.app.focus_tab(member).unwrap();
    }
    fn model(&self, document: u64) -> &Document {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .find(|doc| doc.id == document)
            .unwrap()
    }
    fn model_mut(&mut self, document: u64) -> &mut Document {
        self.app
            .documents
            .iter_mut()
            .chain(&mut self.app.hidden_documents)
            .find(|doc| doc.id == document)
            .unwrap()
    }
    fn edited(&mut self, member: Membership, text: &str) -> String {
        self.focus(member);
        self.app.doc_mut().move_to(0, false);
        self.app.execute("type", json!({"text":text}));
        assert!(self.app.doc().dirty());
        self.app.doc().text.to_string()
    }
    fn gates(&mut self, points: Vec<GatePoint>) -> Gates {
        assert!(!self.app.saves_pending());
        let (worker, entered, release) = Worker::fixture_gated(points);
        self.app.saving.worker = worker;
        Gates { entered, release }
    }
    fn assert_original_disks(&self) {
        assert_eq!(std::fs::read(&self.a).unwrap(), A_DISK.as_bytes());
        assert_eq!(std::fs::read(&self.b).unwrap(), B_DISK.as_bytes());
        assert_eq!(std::fs::read(&self.c).unwrap(), C_DISK.as_bytes());
    }
}
struct Gates {
    entered: Receiver<GatePoint>,
    release: SyncSender<()>,
}
impl Gates {
    fn reach(&self, app: &mut App, expected: GatePoint) {
        let deadline = Instant::now() + WAIT;
        loop {
            app.poll();
            match self.entered.try_recv() {
                Ok(actual) => {
                    assert_eq!(actual, expected);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    panic!("{expected:?} disconnected: {}", app.message)
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(Instant::now() < deadline, "{expected:?}: {}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn release(&self) {
        self.release.try_send(()).unwrap();
    }
}
fn until(app: &mut App, phase: &str, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if ready(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn command(app: &mut App, command: &str) {
    app.execute(command, Value::Null);
}
fn answer(app: &mut App, key: char) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char(key),
        KeyModifiers::NONE,
    )));
}
fn start_batch_save(app: &mut App, origin: Membership) {
    command(app, GROUP_CLOSE);
    assert!(matches!(app.modal, Some(Modal::Confirm(AfterSave::Close))));
    assert_eq!(app.active_tab_membership(), Some(origin));
    answer(app, 's');
    assert!(app.saves_pending(), "{}", app.message);
}

impl Drop for Gates {
    fn drop(&mut self) {
        let _ = self.release.try_send(());
    }
}

const LEFT: &str = "workbench.action.focusLeftGroup";
const RIGHT: &str = "workbench.action.focusRightGroup";
const UP: &str = "workbench.action.focusAboveGroup";
const DOWN: &str = "workbench.action.focusBelowGroup";
fn draw(app: &mut App, width: u16, height: u16) {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
}
fn nested(f: &mut Fixture) -> [Membership; 3] {
    let left = f.member(f.a_id);
    f.focus(left);
    f.app.doc_mut().move_to(2, false);
    command(&mut f.app, "workbench.action.splitEditorRight");
    let above = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(4, false);
    command(&mut f.app, "workbench.action.splitEditorDown");
    let below = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(6, false);
    f.app
        .doc_mut()
        .secondary
        .push(crate::document::Selection::caret(12));
    [left, above, below]
}
type ViewWitness = (
    usize,
    Option<usize>,
    Vec<crate::document::Selection>,
    usize,
    usize,
);
fn views(f: &Fixture, members: &[Membership]) -> Vec<ViewWitness> {
    members
        .iter()
        .map(|member| {
            let view = f
                .model(member.document)
                .view_state(Some(member.group.value()));
            (
                view.cursor,
                view.anchor,
                view.secondary.clone(),
                view.top,
                view.left,
            )
        })
        .collect()
}

#[test]
fn directional_focus_uses_group_mru_preserves_shared_views_history_and_retires_old_source_hits() {
    let mut f = Fixture::new();
    let members = nested(&mut f);
    f.app.execute("type", json!({"text":"Xλ"}));
    let edited = f.app.doc().text.to_string();
    f.app.doc_mut().undo();
    draw(&mut f.app, 120, 36);
    let before_views = views(&f, &members);
    let epoch = f.app.doc().text_epoch();
    let revision = f.app.doc().revision;
    let before_layout = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
        .unwrap();
    let frame = f.app.editor_presentation.proof().unwrap().clone();
    command(&mut f.app, LEFT);
    assert_eq!(f.app.active_tab_membership(), Some(members[0]));
    assert_eq!(f.app.doc().cursor, before_views[0].0);
    assert!(!f.app.editor_presentation.current(
        &frame,
        f.app.editor_layout(),
        f.app.editor_groups()
    ));
    command(&mut f.app, RIGHT); // Both stacked neighbors: recently active lower wins.
    assert_eq!(f.app.active_tab_membership(), Some(members[2]));
    f.focus(members[1]);
    command(&mut f.app, LEFT);
    command(&mut f.app, RIGHT);
    assert_eq!(f.app.active_tab_membership(), Some(members[1]));
    command(&mut f.app, UP); // Outer top boundary wraps to lower existing group.
    assert_eq!(f.app.active_tab_membership(), Some(members[2]));
    command(&mut f.app, DOWN);
    assert_eq!(f.app.active_tab_membership(), Some(members[1]));
    assert_eq!(views(&f, &members), before_views);
    assert_eq!(f.model(f.a_id).text_epoch(), epoch);
    assert_eq!(f.model(f.a_id).revision, revision);
    let after = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
        .unwrap();
    assert!(before_layout.same_revision(&after));
    assert_eq!(f.app.editor_groups.memberships(f.a_id).count(), 3);
    f.model_mut(f.a_id).redo();
    assert_eq!(f.model(f.a_id).text.to_string(), edited);
    f.model_mut(f.a_id).undo();
    assert_eq!(f.model(f.a_id).text.to_string(), A_DISK);
    f.assert_original_disks();
}

#[test]
fn single_group_focus_enters_editor_without_creating_group_or_invalidating_unchanged_topology() {
    let mut f = Fixture::new();
    let origin = f.app.active_tab_membership();
    draw(&mut f.app, 120, 36);
    let frame = f.app.editor_presentation.proof().unwrap().clone();
    let groups = f.app.editor_groups.proof();
    let ids: Vec<_> = f.app.documents.iter().map(|doc| doc.id).collect();
    f.app.focus = Focus::Explorer;
    for command_id in [LEFT, RIGHT, UP, DOWN] {
        command(&mut f.app, command_id);
    }
    assert!(f.app.focus == Focus::Editor);
    assert_eq!(f.app.active_tab_membership(), origin);
    assert_eq!(f.app.editor_groups.proof(), groups);
    assert!(f.app.editor_presentation.current(
        &frame,
        f.app.editor_layout(),
        f.app.editor_groups()
    ));
    assert_eq!(
        f.app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(f.app.editor_groups.groups().len(), 1);
    f.assert_original_disks();
    let mut empty = App::new(f.a.parent().unwrap().to_path_buf(), Profile::Linux);
    for command_id in [LEFT, RIGHT, UP, DOWN] {
        command(&mut empty, command_id);
    }
    assert!(empty.documents.is_empty() && empty.editor_groups.groups().is_empty());
}

#[test]
fn tiny_projection_focuses_retained_hidden_group_without_collapsing_tree_or_view_identity() {
    let mut f = Fixture::new();
    let members = nested(&mut f);
    let before_views = views(&f, &members);
    let geometry = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 1, 1))
        .unwrap();
    assert!(geometry.active_only());
    draw(&mut f.app, 10, 3);
    assert!(f.app.editor_presentation.proof().is_none());
    command(&mut f.app, LEFT);
    assert_eq!(f.app.active_tab_membership(), Some(members[0]));
    command(&mut f.app, RIGHT);
    assert_eq!(f.app.active_tab_membership(), Some(members[2]));
    assert_eq!(f.app.editor_groups.groups().len(), 3);
    assert_eq!(views(&f, &members), before_views);
    let after = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 1, 1))
        .unwrap();
    assert!(after.same_revision(&geometry));
    assert!(after.active_only());
    assert_eq!(f.model(f.a_id).text.to_string(), A_DISK);
    f.assert_original_disks();
}

#[test]
fn authorized_original_save_receipt_survives_directional_focus_and_newer_origin_edit() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    let c = f.member(f.c_id);
    f.focus(c);
    command(&mut f.app, "workbench.action.splitEditorRight");
    let peer = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(2, false);
    let captured = f.edited(a, "captured λ🙂 ");
    let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut f.app, "workbench.action.files.save");
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    command(&mut f.app, RIGHT);
    assert_eq!(f.app.active_tab_membership(), Some(peer));
    assert!(f.app.saving.worker.busy());
    f.model_mut(f.a_id).insert("newer ", false);
    let newer = f.model(f.a_id).text.to_string();
    gates.release();
    gates.reach(&mut f.app, GatePoint::BeforeFinish);
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(f.model(f.a_id).save_generation(), 0);
    gates.release();
    until(&mut f.app, "original receipt after spatial focus", |app| {
        !app.saves_pending()
    });
    assert_eq!(f.app.active_tab_membership(), Some(peer));
    assert_eq!(f.app.doc().cursor, 2);
    assert_eq!(f.model(f.a_id).save_generation(), 1);
    assert!(f.model(f.a_id).dirty());
    assert_eq!(f.model(f.a_id).text.to_string(), newer);
    f.model_mut(f.a_id).undo();
    assert_eq!(f.model(f.a_id).text.to_string(), captured);
    f.model_mut(f.a_id).undo();
    assert_eq!(f.model(f.a_id).text.to_string(), A_DISK);
    f.model_mut(f.a_id).redo();
    f.model_mut(f.a_id).redo();
    assert_eq!(f.model(f.a_id).text.to_string(), newer);
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&f.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn directional_focus_keeps_remaining_group_close_batch_and_approved_exact_origin_receipt() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    let b = f.member(f.b_id);
    let c = f.member(f.c_id);
    f.focus(c);
    command(&mut f.app, "workbench.action.splitEditorRight");
    let peer = f.app.active_tab_membership().unwrap();
    let b_edited = f.edited(b, "retained B ");
    let captured = f.edited(a, "approved A 🙂 ");
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    start_batch_save(&mut f.app, a);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    assert!(f.app.closing_group.is_some());
    command(&mut f.app, RIGHT);
    assert_eq!(f.app.active_tab_membership(), Some(peer));
    assert!(f.app.closing_group.is_some());
    assert!(f.app.saving.worker.busy());
    gates.release();
    until(
        &mut f.app,
        "original close then original next dirty tab",
        |app| !app.saves_pending() && matches!(app.modal, Some(Modal::Confirm(AfterSave::Close))),
    );
    assert!(!f.app.editor_groups.membership_current(a));
    assert!(f.app.editor_groups.membership_current(b));
    assert!(f.app.editor_groups.membership_current(c));
    assert!(f.app.editor_groups.membership_current(peer));
    assert_eq!(f.app.active_tab_membership(), Some(b));
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(f.model(f.b_id).text.to_string(), b_edited);
    answer(&mut f.app, 'c');
    assert!(f.app.modal.is_none());
    assert!(f.app.editor_groups.membership_current(b));
    f.model_mut(f.b_id).undo();
    assert_eq!(f.model(f.b_id).text.to_string(), B_DISK);
    f.model_mut(f.b_id).redo();
    assert_eq!(f.model(f.b_id).text.to_string(), b_edited);
    assert_eq!(std::fs::read(&f.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), C_DISK.as_bytes());
}
