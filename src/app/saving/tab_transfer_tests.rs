//! Cross-group transfers with real document/view and save-worker ownership.
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
const NEXT: &str = "workbench.action.moveEditorToNextGroup";
const PREVIOUS: &str = "workbench.action.moveEditorToPreviousGroup";
const FIRST: &str = "workbench.action.moveEditorToFirstGroup";
const LAST: &str = "workbench.action.moveEditorToLastGroup";
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

fn policy(app: &mut App, key: &str, value: Value) {
    let mut values = app.settings.extension_layers()[0].clone();
    values.insert(key.into(), value);
    app.settings = crate::settings::Settings::from_values(values, "transfer fixture").unwrap();
}
fn single(f: &mut Fixture) {
    for document in [f.b_id, f.c_id] {
        let member = f.member(document);
        f.focus(member);
        command(&mut f.app, CLOSE);
        assert!(!f.app.editor_groups.membership_current(member));
    }
    f.focus(f.member(f.a_id));
}

#[test]
fn transfer_reuses_shared_target_projects_selection_and_preserves_redo_other_views_and_seals() {
    let mut f = Fixture::new();
    let source = f.member(f.a_id);
    command(&mut f.app, "workbench.action.splitEditor");
    let target = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(5, false);
    command(&mut f.app, "workbench.action.splitEditorDown");
    let other = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(7, false);
    let foreign_view = view(f.app.doc(), other.group);
    f.focus(source);
    f.app.doc_mut().move_to(0, false);
    f.app.doc_mut().insert("λ", false);
    f.app.doc_mut().undo();
    f.app.doc_mut().set_selections(vec![
        crate::document::Selection::caret(2),
        crate::document::Selection::caret(12),
    ]);
    let expected = view(f.app.doc(), source.group);
    let epoch = f.app.doc().text_epoch();
    let revision = f.app.doc().revision;
    let layout = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
        .unwrap();
    draw(&mut f.app);
    let frame = f.app.editor_presentation.proof().unwrap().clone();
    command(&mut f.app, NEXT);
    assert_eq!(f.app.active_tab_membership(), Some(target));
    assert!(!f.app.editor_groups.membership_current(source));
    assert_eq!(f.app.doc().id, f.a_id);
    assert_eq!(view(f.app.doc(), target.group), expected);
    assert_eq!(view(f.app.doc(), other.group), foreign_view);
    assert_eq!(
        (f.app.doc().revision, f.app.doc().text_epoch()),
        (revision, epoch)
    );
    assert_eq!(order(&f.app, source.group), [f.b_id, f.c_id]);
    assert!(!f.app.editor_presentation.current(
        &frame,
        f.app.editor_layout(),
        f.app.editor_groups()
    ));
    let after = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
        .unwrap();
    assert!(layout.same_revision(&after));
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), format!("λ{A_DISK}"));
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), A_DISK);
    assert_eq!(view(f.app.doc(), other.group), foreign_view);
    f.assert_original_disks();
}

#[test]
fn next_group_creation_before_last_source_collapse_preserves_dirty_identity_for_right_and_down() {
    for direction in ["right", "down"] {
        let mut f = Fixture::new();
        single(&mut f);
        policy(
            &mut f.app,
            "workbench.editor.openSideBySideDirection",
            json!(direction),
        );
        let source = f.app.active_tab_membership().unwrap();
        let text = f.edited(source, "dirty λ ");
        f.app.doc_mut().set_selections(vec![
            crate::document::Selection::caret(2),
            crate::document::Selection::caret(12),
        ]);
        let selected = f.app.doc().selections();
        let epoch = f.app.doc().text_epoch();
        let old_geometry = f
            .app
            .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
            .unwrap();
        command(&mut f.app, NEXT);
        let target = f.app.active_tab_membership().unwrap();
        assert_ne!(target.group, source.group);
        assert_ne!(target.tab, source.tab);
        assert_eq!(f.app.editor_groups.groups().len(), 1);
        assert_eq!(f.app.doc().id, f.a_id);
        assert_eq!(f.app.doc().text.to_string(), text);
        assert_eq!(f.app.doc().text_epoch(), epoch);
        assert_eq!(f.app.doc().selections(), selected);
        assert!(!f.app.editor_groups.membership_current(source));
        assert!(
            !old_geometry.same_revision(
                &f.app
                    .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
                    .unwrap()
            )
        );
        f.app.doc_mut().undo();
        assert_eq!(f.app.doc().text.to_string(), A_DISK);
        f.app.doc_mut().redo();
        assert_eq!(f.app.doc().text.to_string(), text);
        f.assert_original_disks();
    }
}

#[test]
fn transfer_keeps_sticky_preview_modes_and_tiny_logical_tree_without_source_text_edit() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    f.pin(a);
    command(&mut f.app, "workbench.action.splitEditor");
    let shared = f.app.active_tab_membership().unwrap();
    f.app
        .install_preview_document(
            Document::open_existing(&f.c).unwrap(),
            crate::editor_groups::OpenMode::Preview,
        )
        .unwrap();
    let preview = f.app.active_tab_membership().unwrap();
    assert!(f.app.active_editor_is_preview());
    f.focus(a);
    let ids: Vec<_> = f.app.documents.iter().map(|document| document.id).collect();
    command(&mut f.app, NEXT);
    assert_eq!(f.app.active_tab_membership(), Some(shared));
    assert!(is_sticky(&f.app, shared));
    assert_eq!(
        f.app.editor_groups.group(shared.group).unwrap().preview(),
        Some(preview)
    );
    assert_eq!(
        f.app
            .documents
            .iter()
            .map(|document| document.id)
            .collect::<Vec<_>>(),
        ids
    );
    let tiny = f
        .app
        .project_editor_layout(ratatui::layout::Rect::new(0, 0, 1, 1))
        .unwrap();
    assert!(tiny.active_only());
    command(&mut f.app, FIRST);
    assert_eq!(f.app.active_tab_membership().unwrap().document, f.a_id);
    assert_eq!(f.app.doc().text.to_string(), A_DISK);
    f.assert_original_disks();
}

#[test]
fn changed_transfer_policy_and_late_layout_refusal_preserve_views_history_and_all_memberships() {
    for (key, value) in [
        ("workbench.editor.openPositioning", json!("left")),
        ("workbench.editor.closeEmptyGroups", json!(false)),
    ] {
        let mut f = Fixture::new();
        let source = f.member(f.a_id);
        command(&mut f.app, "workbench.action.splitEditor");
        f.focus(source);
        f.app.doc_mut().insert("x", false);
        f.app.doc_mut().undo();
        policy(&mut f.app, key, value);
        let groups = f.app.editor_groups.clone();
        let selection = f.app.doc().selections();
        let epoch = f.app.doc().text_epoch();
        command(&mut f.app, FIRST); // No-op bypasses unsupported changed policy.
        assert_eq!(f.app.editor_groups, groups);
        command(&mut f.app, PREVIOUS); // First group has no destination.
        assert_eq!(f.app.editor_groups, groups);
        command(&mut f.app, NEXT);
        assert!(
            f.app.message.contains("Group transfer rejected"),
            "{}",
            f.app.message
        );
        assert_eq!(f.app.editor_groups, groups);
        assert_eq!(f.app.doc().selections(), selection);
        assert_eq!(f.app.doc().text_epoch(), epoch);
        f.app.doc_mut().redo();
        assert!(f.app.doc().text.to_string().contains('x'));
        f.assert_original_disks();
    }
    let mut f = Fixture::new();
    let source = f.member(f.a_id);
    command(&mut f.app, "workbench.action.splitEditor");
    f.focus(source);
    let ids: Vec<_> = f
        .app
        .editor_groups
        .groups()
        .iter()
        .rev()
        .map(|group| group.id())
        .collect();
    let plan = f
        .app
        .editor_layout
        .prepare_flat(&ids, crate::editor_layout::Axis::Rows)
        .unwrap();
    f.app.editor_layout.commit(plan).unwrap(); // Real incompatible staged layout.
    let groups = f.app.editor_groups.clone();
    let selection = f.app.doc().selections();
    let epoch = f.app.doc().text_epoch();
    command(&mut f.app, NEXT);
    assert!(f.app.message.contains("order differ"), "{}", f.app.message);
    assert_eq!(f.app.editor_groups, groups);
    assert_eq!(f.app.doc().selections(), selection);
    assert_eq!(f.app.doc().text_epoch(), epoch);
    f.assert_original_disks();
}

#[test]
fn moved_save_close_origin_publishes_real_receipt_but_never_closes_reused_destination() {
    for newer in [false, true] {
        let mut f = Fixture::new();
        let source = f.member(f.a_id);
        let captured = f.edited(source, "captured 🙂 ");
        let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
        command(&mut f.app, "workbench.action.files.save");
        gates.reach(&mut f.app, GatePoint::BeforeCommit);
        command(&mut f.app, CLOSE);
        assert!(f.app.modal.is_none());
        assert!(f.app.saving.active.as_ref().unwrap().continuation.is_some());
        command(&mut f.app, "workbench.action.splitEditor");
        let target = f.app.active_tab_membership().unwrap();
        f.focus(source);
        command(&mut f.app, NEXT);
        assert_eq!(f.app.active_tab_membership(), Some(target));
        assert!(!f.app.editor_groups.membership_current(source));
        assert!(f.app.saving.active.as_ref().unwrap().continuation.is_some());
        assert!(f.app.saving.worker.busy());
        gates.release();
        gates.reach(&mut f.app, GatePoint::BeforeFinish);
        assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
        assert_eq!(f.app.doc().save_generation(), 0);
        let expected = if newer {
            f.app.execute("type", json!({"text":"newer λ "}));
            f.app.doc().text.to_string()
        } else {
            captured.clone()
        };
        gates.release();
        until(&mut f.app, "original moved receipt", |app| {
            !app.saves_pending()
        });
        assert_eq!(f.app.active_tab_membership(), Some(target));
        assert!(f.app.editor_groups.membership_current(target));
        assert_eq!(f.app.doc().id, f.a_id);
        assert_eq!(f.app.doc().save_generation(), 1);
        assert_eq!(f.app.doc().dirty(), newer);
        assert_eq!(f.app.doc().text.to_string(), expected);
        assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
        if newer {
            f.app.doc_mut().undo();
            assert_eq!(f.app.doc().text.to_string(), captured);
            f.app.doc_mut().redo();
            assert_eq!(f.app.doc().text.to_string(), expected);
        }
        assert_eq!(std::fs::read(&f.b).unwrap(), B_DISK.as_bytes());
        assert_eq!(std::fs::read(&f.c).unwrap(), C_DISK.as_bytes());
    }
}

#[test]
fn save_as_receipt_survives_single_source_collapse_and_publishes_original_destination() {
    let mut f = Fixture::new();
    single(&mut f);
    let source = f.app.active_tab_membership().unwrap();
    let captured = f.edited(source, "Save As λ ");
    let destination = f.a.parent().unwrap().join("renamed.cpp");
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    command(&mut f.app, "workbench.action.files.saveAs");
    assert!(matches!(
        f.app.prompt.as_ref().unwrap().kind,
        PromptKind::SaveAs
    ));
    f.app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    command(&mut f.app, NEXT);
    let target = f.app.active_tab_membership().unwrap();
    assert_ne!(target.group, source.group);
    assert_eq!(f.app.editor_groups.groups().len(), 1);
    gates.release();
    until(&mut f.app, "original Save As receipt", |app| {
        !app.saves_pending()
    });
    assert_eq!(f.app.active_tab_membership(), Some(target));
    assert_eq!(f.app.doc().id, f.a_id);
    assert_eq!(f.app.doc().path.as_ref(), Some(&destination));
    assert_eq!(f.app.doc().save_generation(), 1);
    assert_eq!(std::fs::read(&destination).unwrap(), captured.as_bytes());
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), A_DISK);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), captured);
    f.assert_original_disks();
}

#[test]
fn unrelated_approved_save_close_still_finishes_after_transfer_retires_remaining_batch() {
    let mut f = Fixture::new();
    let a = f.member(f.a_id);
    let b = f.member(f.b_id);
    let c = f.member(f.c_id);
    f.focus(c);
    command(&mut f.app, "workbench.action.splitEditor");
    let shared_c = f.app.active_tab_membership().unwrap();
    let captured = f.edited(a, "approved A λ ");
    let b_text = f.edited(b, "retained B 🙂 ");
    f.focus(a);
    let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    start_batch_save(&mut f.app, a);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    f.focus(b);
    command(&mut f.app, NEXT);
    let moved_b = f.app.active_tab_membership().unwrap();
    assert_ne!(moved_b, b);
    assert!(f.app.closing_group.is_none());
    assert!(f.app.saving.active.as_ref().unwrap().continuation.is_some());
    gates.release();
    gates.reach(&mut f.app, GatePoint::BeforeFinish);
    assert!(f.app.editor_groups.membership_current(a));
    gates.release();
    until(&mut f.app, "unrelated approved close", |app| {
        !app.saves_pending()
    });
    assert!(!f.app.editor_groups.membership_current(a));
    assert_eq!(f.app.active_tab_membership(), Some(moved_b));
    assert!(f.app.editor_groups.membership_current(moved_b));
    assert!(f.app.editor_groups.membership_current(c));
    assert!(f.app.editor_groups.membership_current(shared_c));
    assert_eq!(f.model(f.b_id).text.to_string(), b_text);
    f.model_mut(f.b_id).undo();
    assert_eq!(f.model(f.b_id).text.to_string(), B_DISK);
    f.model_mut(f.b_id).redo();
    assert_eq!(f.model(f.b_id).text.to_string(), b_text);
    assert_eq!(std::fs::read(&f.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&f.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn next_creation_honors_right_down_geometry_and_endpoint_noops_do_not_mutate_state() {
    for direction in ["right", "down"] {
        let mut f = Fixture::new();
        let source = f.member(f.a_id);
        policy(
            &mut f.app,
            "workbench.editor.openSideBySideDirection",
            json!(direction),
        );
        command(&mut f.app, NEXT);
        let target = f.app.active_tab_membership().unwrap();
        assert_eq!(f.app.editor_groups.groups().len(), 2);
        assert_eq!(order(&f.app, source.group), [f.b_id, f.c_id]);
        let geometry = f
            .app
            .project_editor_layout(ratatui::layout::Rect::new(0, 0, 120, 36))
            .unwrap();
        let left = geometry.placement(source.group).unwrap().outer;
        let right = geometry.placement(target.group).unwrap().outer;
        if direction == "right" {
            assert_eq!(left.y, right.y);
            assert!(left.x < right.x);
        } else {
            assert_eq!(left.x, right.x);
            assert!(left.y < right.y);
        }
        let groups = f.app.editor_groups.clone();
        command(&mut f.app, LAST);
        assert_eq!(f.app.editor_groups, groups);
        command(&mut f.app, FIRST);
        assert_eq!(f.app.doc().id, f.a_id);
        assert_eq!(f.app.editor_groups.groups().len(), 1);
        assert_eq!(order(&f.app, source.group), [f.b_id, f.c_id, f.a_id]);
        f.assert_original_disks();
    }
}

#[test]
fn unapproved_dirty_close_modal_retires_on_transfer_without_discarding_work_or_redo() {
    let mut f = Fixture::new();
    let source = f.member(f.a_id);
    let text = f.edited(source, "unsaved λ ");
    command(&mut f.app, CLOSE);
    assert!(matches!(
        f.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    let generation = f.app.saving.close_generation;
    command(&mut f.app, NEXT);
    assert!(f.app.modal.is_none());
    assert!(f.app.close_membership.is_none());
    assert_eq!(f.app.saving.close_generation, generation);
    assert_eq!(f.app.doc().id, f.a_id);
    assert_eq!(f.app.doc().text.to_string(), text);
    assert!(f.app.doc().dirty());
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), A_DISK);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), text);
    f.assert_original_disks();
}
