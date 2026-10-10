//! Original same-group moves with real document/view and save-worker ownership.
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
const PIN: &str = "workbench.action.pinEditor";
const CLOSE: &str = "workbench.action.closeActiveEditor";
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
        app.extension_node = "vscli-reorder-fixture-node-missing".into();
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
    fn pin(&mut self, member: Membership) {
        self.focus(member);
        command(&mut self.app, PIN);
        assert!(is_sticky(&self.app, member), "{}", self.app.message);
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
fn is_sticky(app: &App, member: Membership) -> bool {
    app.editor_groups
        .group(member.group)
        .unwrap()
        .tabs()
        .iter()
        .find(|tab| tab.id() == member.tab && tab.document() == member.document)
        .unwrap()
        .is_sticky()
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
const LEFT: &str = "workbench.action.moveEditorLeftInGroup";
const RIGHT: &str = "workbench.action.moveEditorRightInGroup";
fn draw(app: &mut App) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
}
type ViewWitness = (
    usize,
    Option<usize>,
    Vec<crate::document::Selection>,
    usize,
    usize,
);
fn view(doc: &Document, group: crate::editor_groups::GroupId) -> ViewWitness {
    let value = doc.view_state(Some(group.value()));
    (
        value.cursor,
        value.anchor,
        value.secondary.clone(),
        value.top,
        value.left,
    )
}
fn order(app: &App, group: crate::editor_groups::GroupId) -> Vec<u64> {
    app.editor_groups
        .group(group)
        .unwrap()
        .tabs()
        .iter()
        .map(|tab| tab.document())
        .collect()
}

#[test]
fn actual_move_changes_only_current_group_order_and_preserves_shared_views_redo_and_layout() {
    let mut f = Fixture::new();
    let source = f.member(f.a_id);
    f.app.doc_mut().move_to(2, false);
    command(&mut f.app, "workbench.action.splitEditorRight");
    let shared = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(5, false);
    f.app
        .doc_mut()
        .secondary
        .push(crate::document::Selection::caret(12));
    f.app
        .open_preview_model(f.b_id, crate::editor_groups::OpenMode::Committed)
        .unwrap();
    f.focus(shared);
    f.app.execute("type", json!({"text":"X"}));
    let edited = f.app.doc().text.to_string();
    f.app.doc_mut().undo();
    let text = f.app.doc().text.to_string();
    let epoch = f.app.doc().text_epoch();
    let revision = f.app.doc().revision;
    let generation = f.app.doc().save_generation();
    draw(&mut f.app);
    let old_frame = f.app.editor_presentation.proof().unwrap().clone();
    let peer = f.app.editor_groups.group_proof(source.group).unwrap();
    let old_layout = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
        .unwrap();
    let source_view = view(f.model(f.a_id), source.group);
    let shared_view = view(f.model(f.a_id), shared.group);
    f.app.focus = Focus::Explorer;
    command(&mut f.app, RIGHT);
    assert_eq!(order(&f.app, shared.group), vec![f.b_id, f.a_id]);
    assert_eq!(f.app.active_tab_membership(), Some(shared));
    assert!(f.app.focus == Focus::Explorer);
    assert!(f.app.editor_groups.group_proof_current(&peer));
    let current = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
        .unwrap();
    assert!(old_layout.same_revision(&current));
    assert!(!f.app.editor_presentation.current(
        &old_frame,
        f.app.editor_layout(),
        f.app.editor_groups()
    ));
    assert_eq!(view(f.model(f.a_id), source.group), source_view);
    assert_eq!(view(f.model(f.a_id), shared.group), shared_view);
    assert_eq!(f.model(f.a_id).text.to_string(), text);
    assert_eq!(f.model(f.a_id).text_epoch(), epoch);
    assert_eq!(f.model(f.a_id).revision, revision);
    assert_eq!(f.model(f.a_id).save_generation(), generation);
    f.model_mut(f.a_id).redo();
    assert_eq!(f.model(f.a_id).text.to_string(), edited);
    f.model_mut(f.a_id).undo();
    assert_eq!(f.model(f.a_id).text.to_string(), text);
    f.assert_original_disks();
}

#[test]
fn direct_edge_noop_retains_sealed_hits_group_proofs_and_pending_redo() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    let edited = f.edited(a, "λ🙂 ");
    f.app.doc_mut().undo();
    draw(&mut f.app);
    let frame = f.app.editor_presentation.proof().unwrap().clone();
    let groups = f.app.editor_groups.proof();
    let order_before = order(&f.app, a.group);
    let epoch = f.app.doc().text_epoch();
    let interaction = f.app.extension_services.epoch;
    command(&mut f.app, LEFT);
    assert_eq!(f.app.editor_groups.proof(), groups);
    assert!(f.app.editor_presentation.current(
        &frame,
        f.app.editor_layout(),
        f.app.editor_groups()
    ));
    assert_eq!(order(&f.app, a.group), order_before);
    assert_eq!(f.app.extension_services.epoch, interaction);
    assert_eq!(f.app.doc().text_epoch(), epoch);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), edited);
    f.assert_original_disks();
    let mut empty = App::new(f.a.parent().unwrap().to_path_buf(), Profile::Linux);
    command(&mut empty, LEFT);
    command(&mut empty, RIGHT);
    assert!(empty.documents.is_empty());
    assert!(empty.editor_groups.groups().is_empty());
}

#[test]
fn moved_preview_commits_and_crossing_sticky_prefix_rechecks_ordinary_close_policy() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    f.pin(a);
    f.focus(f.member(f.c_id));
    let path = f.a.with_file_name("preview.cpp");
    std::fs::write(&path, A_DISK).unwrap();
    f.app
        .install_preview_document(
            Document::open(&path).unwrap(),
            crate::editor_groups::OpenMode::Preview,
        )
        .unwrap();
    let preview = f.app.active_tab_membership().unwrap();
    assert_eq!(
        f.app.editor_groups.group(preview.group).unwrap().preview(),
        Some(preview)
    );
    command(&mut f.app, RIGHT); // Already last: no implicit preview commit.
    assert_eq!(
        f.app.editor_groups.group(preview.group).unwrap().preview(),
        Some(preview)
    );
    command(&mut f.app, LEFT);
    assert_eq!(
        f.app.editor_groups.group(preview.group).unwrap().preview(),
        None
    );
    command(&mut f.app, LEFT);
    command(&mut f.app, LEFT);
    assert!(is_sticky(&f.app, preview));
    let member_count = f
        .app
        .editor_groups
        .group(preview.group)
        .unwrap()
        .tabs()
        .len();
    command(&mut f.app, CLOSE);
    assert!(f.app.editor_groups.membership_current(preview));
    assert_eq!(
        f.app
            .editor_groups
            .group(preview.group)
            .unwrap()
            .tabs()
            .len(),
        member_count
    );
    assert_ne!(f.app.active_tab_membership(), Some(preview));
    assert_eq!(std::fs::read(path).unwrap(), A_DISK.as_bytes());
    f.assert_original_disks();
}

#[test]
fn reorder_retires_remaining_group_batch_but_approved_origin_save_close_finishes_exactly() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    let b = f.member(f.b_id);
    let c = f.member(f.c_id);
    let b_edited = f.edited(b, "retained B λ ");
    let captured = f.edited(a, "approved A 🙂 ");
    let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    start_batch_save(&mut f.app, a);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    f.focus(b);
    let b_cursor = f.app.doc().cursor;
    command(&mut f.app, RIGHT);
    assert!(f.app.closing_group.is_none());
    assert!(f.app.saving.worker.busy());
    assert_eq!(order(&f.app, a.group), vec![f.a_id, f.c_id, f.b_id]);
    assert_eq!(f.model(f.a_id).save_generation(), 0);
    gates.release();
    gates.reach(&mut f.app, GatePoint::BeforeFinish);
    assert!(f.app.editor_groups.membership_current(a));
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(f.model(f.a_id).save_generation(), 0);
    gates.release();
    until(&mut f.app, "original approved receipt", |app| {
        !app.saves_pending()
    });
    assert!(!f.app.editor_groups.membership_current(a));
    assert!(f.app.editor_groups.membership_current(b));
    assert!(f.app.editor_groups.membership_current(c));
    assert_eq!(f.app.active_tab_membership(), Some(b));
    assert_eq!(f.app.doc().cursor, b_cursor);
    assert_eq!(f.model(f.b_id).text.to_string(), b_edited);
    assert_eq!(f.model(f.b_id).save_generation(), 0);
    f.model_mut(f.b_id).undo();
    assert_eq!(f.model(f.b_id).text.to_string(), B_DISK);
    f.model_mut(f.b_id).redo();
    assert_eq!(f.model(f.b_id).text.to_string(), b_edited);
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&f.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn crossing_into_sticky_during_authorized_save_publishes_receipt_but_rejects_original_close() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    let b = f.member(f.b_id);
    f.pin(b); // [sticky B, A, C].
    let captured = f.edited(a, "authorized λ🙂 ");
    let epoch = f.app.doc().text_epoch();
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    command(&mut f.app, CLOSE);
    assert!(matches!(
        f.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    answer(&mut f.app, 's');
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    command(&mut f.app, LEFT);
    assert!(is_sticky(&f.app, a));
    assert!(f.app.saving.worker.busy());
    assert_eq!(f.model(f.a_id).text_epoch(), epoch);
    gates.release();
    until(
        &mut f.app,
        "saved but newly sticky origin retained",
        |app| !app.saves_pending(),
    );
    assert!(f.app.editor_groups.membership_current(a));
    assert!(f.app.editor_groups.membership_current(b));
    assert!(f.app.editor_groups.membership_current(f.member(f.c_id)));
    assert_eq!(f.app.active_tab_membership(), Some(a));
    assert_eq!(f.model(f.a_id).save_generation(), 1);
    assert!(!f.model(f.a_id).dirty());
    assert_eq!(f.model(f.a_id).text.to_string(), captured);
    f.model_mut(f.a_id).undo();
    assert_eq!(f.model(f.a_id).text.to_string(), A_DISK);
    f.model_mut(f.a_id).redo();
    assert_eq!(f.model(f.a_id).text.to_string(), captured);
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&f.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), C_DISK.as_bytes());
}
