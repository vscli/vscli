//! Scratch-only App integration qualification. Intended child of app::saving.
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
const UNPIN: &str = "workbench.action.unpinEditor";
const CLOSE: &str = "workbench.action.closeActiveEditor";
const FORCE_CLOSE: &str = "workbench.action.closeActivePinnedEditor";
const GROUP_CLOSE: &str = "workbench.action.closeEditorsInGroup";
const ALL_CLOSE: &str = "workbench.action.closeAllEditors";

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

#[test]
fn ordinary_close_policy_preserves_protected_sticky_and_only_focuses_mru_alternative() {
    for (policy, protected) in [
        (None, true),
        (Some("keyboardAndMouse"), true),
        (Some("keyboard"), true),
        (Some("mouse"), false),
        (Some("never"), false),
    ] {
        let mut fixture = Fixture::with_policy(policy);
        let a = fixture.member(fixture.a_id);
        let b = fixture.member(fixture.b_id);
        let c = fixture.member(fixture.c_id);
        fixture.focus(c);
        fixture.focus(b);
        fixture.pin(a);
        let before = fixture
            .model(fixture.a_id)
            .capture_save(fixture.a.clone())
            .unwrap();
        let original_members: Vec<_> = fixture.app.editor_groups.groups()[0]
            .tabs()
            .iter()
            .map(|tab| tab.id())
            .collect();
        command(&mut fixture.app, CLOSE);
        assert!(fixture.app.modal.is_none());
        assert!(!fixture.app.saves_pending());
        if protected {
            assert!(fixture.app.editor_groups.membership_current(a));
            assert_eq!(fixture.app.active_tab_membership(), Some(b));
            assert_eq!(
                fixture.app.editor_groups.groups()[0]
                    .tabs()
                    .iter()
                    .map(|tab| tab.id())
                    .collect::<Vec<_>>(),
                original_members
            );
            let model = fixture.model(fixture.a_id);
            assert_eq!(model.id, fixture.a_id);
            assert_eq!(model.text_epoch(), before.text_epoch());
            assert_eq!(model.save_generation(), before.save_generation());
            assert!(!model.dirty());
        } else {
            assert!(!fixture.app.editor_groups.membership_current(a));
            assert!(fixture.app.editor_groups.membership_current(b));
            assert!(fixture.app.editor_groups.membership_current(c));
        }
        fixture.assert_original_disks();
    }
}

#[test]
fn protected_close_all_sticky_is_noop_even_when_active_has_unsaved_text() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let edited = fixture.edited(a, "dirty λ🙂 ");
    for member in [
        a,
        fixture.member(fixture.b_id),
        fixture.member(fixture.c_id),
    ] {
        fixture.pin(member);
    }
    fixture.focus(a);
    let proof = fixture.app.editor_groups.proof();
    let epoch = fixture.app.doc().text_epoch();
    let selections = fixture.app.doc().selections();
    command(&mut fixture.app, CLOSE);
    assert!(fixture.app.editor_groups.proof_current(&proof));
    assert!(fixture.app.modal.is_none() && !fixture.app.saves_pending());
    assert_eq!(fixture.app.active_tab_membership(), Some(a));
    assert_eq!(fixture.app.doc().text.to_string(), edited);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().selections(), selections);
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), A_DISK);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), edited);
    fixture.assert_original_disks();
}

#[test]
fn newly_sticky_original_batch_target_keeps_tab_after_successful_actual_save_receipt() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let b = fixture.member(fixture.b_id);
    let captured = fixture.edited(a, "captured λ🙂 ");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    start_batch_save(&mut fixture.app, a);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.pin(a);
    let b_edited = fixture.edited(b, "new focused B ");
    assert!(fixture.app.saving.worker.busy());
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.saving.worker.busy());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    assert!(fixture.model(fixture.a_id).dirty());
    assert!(fixture.app.editor_groups.membership_current(a));
    gates.release();
    until(
        &mut fixture.app,
        "saved but protected original retained",
        |app| !app.saves_pending(),
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert!(is_sticky(&fixture.app, a));
    assert!(fixture.app.closing_group.is_none() && fixture.app.modal.is_none());
    assert_eq!(fixture.app.active_tab_membership(), Some(b));
    assert_eq!(fixture.model(fixture.b_id).text.to_string(), b_edited);
    assert!(fixture.model(fixture.b_id).dirty());
    let model = fixture.model_mut(fixture.a_id);
    assert_eq!(model.id, a.document);
    assert_eq!(model.save_generation(), 1);
    assert!(!model.dirty());
    model.undo();
    assert_eq!(model.text.to_string(), A_DISK);
    assert!(model.dirty());
    model.redo();
    assert_eq!(model.text.to_string(), captured);
    assert!(!model.dirty());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&fixture.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn unrelated_peer_pin_insert_or_reorder_preserves_approved_original_save_close_and_retires_remaining_batch()
 {
    for mutation in ["pin-peer", "insert-new", "reorder-peer"] {
        let mut fixture = Fixture::new();
        let a = fixture.member(fixture.a_id);
        let b = fixture.member(fixture.b_id);
        let captured = fixture.edited(a, "owned A ");
        let b_edited = fixture.edited(b, "remaining B ");
        fixture.focus(a);
        let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
        start_batch_save(&mut fixture.app, a);
        gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
        let newcomer = match mutation {
            "pin-peer" => {
                fixture.pin(b);
                None
            }
            "insert-new" => {
                command(&mut fixture.app, "workbench.action.files.newUntitledFile");
                let member = fixture.app.active_tab_membership().unwrap();
                fixture
                    .app
                    .execute("type", json!({"text":"new unsaved Ω🙂"}));
                Some(member)
            }
            "reorder-peer" => {
                fixture.pin(b);
                command(&mut fixture.app, UNPIN);
                assert!(!is_sticky(&fixture.app, b));
                None
            }
            _ => unreachable!(),
        };
        gates.release();
        gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
        assert!(fixture.app.editor_groups.membership_current(a));
        assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
        gates.release();
        until(
            &mut fixture.app,
            "owned original close with stale remaining batch",
            |app| !app.saves_pending(),
        );
        assert!(
            !fixture.app.editor_groups.membership_current(a),
            "approved original must close: {mutation}"
        );
        assert!(fixture.app.editor_groups.membership_current(b));
        assert!(
            fixture
                .app
                .editor_groups
                .memberships(fixture.c_id)
                .next()
                .is_some()
        );
        assert!(fixture.app.closing_group.is_none() && fixture.app.modal.is_none());
        assert_eq!(fixture.model(fixture.b_id).text.to_string(), b_edited);
        if let Some(newcomer) = newcomer {
            assert!(fixture.app.editor_groups.membership_current(newcomer));
            assert_eq!(fixture.app.active_tab_membership(), Some(newcomer));
            assert_eq!(fixture.app.doc().text.to_string(), "new unsaved Ω🙂");
        }
        let model = fixture.model_mut(fixture.b_id);
        model.undo();
        assert_eq!(model.text.to_string(), B_DISK);
        model.redo();
        assert_eq!(model.text.to_string(), b_edited);
        assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
        assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
        assert_eq!(std::fs::read(&fixture.c).unwrap(), C_DISK.as_bytes());
    }
}

#[test]
fn forced_dirty_sticky_close_any_mode_finishes_original_after_unrelated_pin() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let b = fixture.member(fixture.b_id);
    let captured = fixture.edited(a, "forced captured λ🙂 ");
    fixture.pin(a);
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut fixture.app, FORCE_CLOSE);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    answer(&mut fixture.app, 's');
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    // AnyMode is deliberate force authorization, not a snapshot of a group proof.
    command(&mut fixture.app, UNPIN);
    command(&mut fixture.app, PIN);
    fixture.pin(b);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.editor_groups.membership_current(a));
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    gates.release();
    until(&mut fixture.app, "forced exact original closes", |app| {
        !app.saves_pending()
    });
    assert!(!fixture.app.editor_groups.membership_current(a));
    assert!(fixture.app.editor_groups.membership_current(b));
    assert!(is_sticky(&fixture.app, b));
    assert!(fixture.app.modal.is_none());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
    assert_eq!(std::fs::read(&fixture.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn ordinary_unprotected_close_any_mode_survives_pin_during_authorized_save() {
    let mut fixture = Fixture::with_policy(Some("never"));
    let a = fixture.member(fixture.a_id);
    let captured = fixture.edited(a, "permitted λ ");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit]);
    command(&mut fixture.app, CLOSE);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    answer(&mut fixture.app, 's');
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.pin(a);
    gates.release();
    until(
        &mut fixture.app,
        "unprotected close remains AnyMode",
        |app| !app.saves_pending(),
    );
    assert!(!fixture.app.editor_groups.membership_current(a));
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
}

#[test]
fn standalone_save_as_prompt_and_receipt_remain_on_origin_through_pin_unpin_and_focus_switch() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let b = fixture.member(fixture.b_id);
    let captured = fixture.edited(a, "SaveAs λ🙂 ");
    let destination = fixture.a.with_file_name("fresh-save-as.cpp");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut fixture.app, "workbench.action.files.saveAs");
    assert!(fixture.app.prompt.is_some());
    command(&mut fixture.app, PIN);
    command(&mut fixture.app, UNPIN);
    assert!(
        fixture.app.prompt.is_some(),
        "metadata must not discard standalone SaveAs origin"
    );
    fixture.app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    fixture.app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.pin(a);
    fixture.focus(b);
    let b_cursor = fixture.app.doc().cursor;
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.saving.worker.busy());
    assert_eq!(std::fs::read(&destination).unwrap(), captured.as_bytes());
    assert_eq!(fixture.model(fixture.a_id).path.as_ref(), Some(&fixture.a));
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    gates.release();
    until(
        &mut fixture.app,
        "standalone SaveAs original receipt",
        |app| !app.saves_pending(),
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert!(is_sticky(&fixture.app, a));
    assert_eq!(fixture.app.active_tab_membership(), Some(b));
    assert_eq!(fixture.app.doc().cursor, b_cursor);
    assert_eq!(fixture.model(fixture.a_id).id, a.document);
    assert_eq!(
        fixture.model(fixture.a_id).path.as_ref(),
        Some(&destination)
    );
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 1);
    assert!(!fixture.model(fixture.a_id).dirty());
    let model = fixture.model_mut(fixture.a_id);
    model.undo();
    assert_eq!(model.text.to_string(), A_DISK);
    model.redo();
    assert_eq!(model.text.to_string(), captured);
    assert_eq!(std::fs::read(&destination).unwrap(), captured.as_bytes());
    fixture.assert_original_disks();
}

#[test]
fn close_attached_to_standalone_pending_save_retires_only_newly_pinned_close() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let captured = fixture.edited(a, "pending λ🙂 ");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut fixture.app, "workbench.action.files.save");
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    command(&mut fixture.app, CLOSE);
    assert!(fixture.app.modal.is_none());
    assert!(fixture.app.saving.worker.busy());
    fixture.pin(a);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.editor_groups.membership_current(a));
    gates.release();
    until(
        &mut fixture.app,
        "receipt without newly protected close",
        |app| !app.saves_pending(),
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 1);
    assert!(!fixture.model(fixture.a_id).dirty());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
}

#[test]
fn shared_dirty_group_subset_preserves_excluded_sticky_view_identity_caret_and_undo() {
    let mut fixture = Fixture::new();
    let source = fixture.member(fixture.a_id);
    fixture.focus(source);
    fixture.app.doc_mut().move_to(2, false);
    fixture.pin(source);
    command(&mut fixture.app, "workbench.action.splitEditor");
    let shared = fixture.app.active_tab_membership().unwrap();
    assert_eq!(shared.document, source.document);
    assert_ne!(shared.tab, source.tab);
    assert!(!is_sticky(&fixture.app, shared));
    fixture.app.doc_mut().move_to(5, false);
    fixture.app.execute("type", json!({"text":"λ🙂"}));
    let edited = fixture.app.doc().text.to_string();
    assert_eq!(
        fixture
            .app
            .editor_groups
            .memberships(source.document)
            .count(),
        2
    );
    command(&mut fixture.app, GROUP_CLOSE);
    assert!(fixture.app.modal.is_none() && !fixture.app.saves_pending());
    assert!(!fixture.app.editor_groups.membership_current(shared));
    assert!(fixture.app.editor_groups.membership_current(source));
    assert!(is_sticky(&fixture.app, source));
    assert_eq!(fixture.app.editor_groups.groups().len(), 1);
    assert_eq!(fixture.app.doc().id, source.document);
    assert_eq!(fixture.app.doc().cursor, 2);
    assert_eq!(fixture.app.doc().text.to_string(), edited);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), A_DISK);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), edited);
    assert_eq!(fixture.app.doc().save_generation(), 0);
    fixture.assert_original_disks();
}

#[test]
fn all_editor_subset_reviews_last_eligible_shared_membership_once_and_preserves_dirty_stickies() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let b = fixture.member(fixture.b_id);
    let c = fixture.member(fixture.c_id);
    let a_edited = fixture.edited(a, "sticky A λ🙂 ");
    fixture.pin(a);
    let b_edited = fixture.edited(b, "save B λ🙂 ");
    // C is sticky at source; split B is a second eligible membership.
    let c_edited = fixture.edited(c, "sticky C λ🙂 ");
    fixture.pin(c);
    fixture.focus(b);
    command(&mut fixture.app, "workbench.action.splitEditor");
    let shared_b = fixture.app.active_tab_membership().unwrap();
    assert_eq!(shared_b.document, b.document);
    assert_eq!(fixture.app.editor_groups.memberships(b.document).count(), 2);
    command(&mut fixture.app, ALL_CLOSE);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    assert_eq!(fixture.app.doc().id, b.document);
    answer(&mut fixture.app, 'c');
    assert!(fixture.app.modal.is_none());
    assert!(
        fixture
            .app
            .editor_groups
            .memberships(b.document)
            .next()
            .is_some()
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert!(fixture.app.editor_groups.membership_current(c));
    assert_eq!(fixture.model(fixture.b_id).text.to_string(), b_edited);
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
    // Explicit retry authorizes current eligibility, not a stale old target set.
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut fixture.app, ALL_CLOSE);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    assert_eq!(fixture.app.doc().id, b.document);
    answer(&mut fixture.app, 's');
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(
        fixture
            .app
            .editor_groups
            .memberships(b.document)
            .next()
            .is_some()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), b_edited.as_bytes());
    gates.release();
    until(&mut fixture.app, "all nonsticky targets settled", |app| {
        !app.saves_pending() && app.modal.is_none()
    });
    assert!(
        fixture
            .app
            .editor_groups
            .memberships(b.document)
            .next()
            .is_none()
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert!(fixture.app.editor_groups.membership_current(c));
    assert!(is_sticky(&fixture.app, a) && is_sticky(&fixture.app, c));
    assert_eq!(fixture.model(fixture.a_id).text.to_string(), a_edited);
    assert!(fixture.model(fixture.a_id).dirty());
    assert_eq!(fixture.model(fixture.c_id).text.to_string(), c_edited);
    assert!(fixture.model(fixture.c_id).dirty());
    let c_model = fixture.model_mut(fixture.c_id);
    c_model.undo();
    assert_eq!(c_model.text.to_string(), C_DISK);
    c_model.redo();
    assert_eq!(c_model.text.to_string(), c_edited);
    let model = fixture.model_mut(fixture.a_id);
    model.undo();
    assert_eq!(model.text.to_string(), A_DISK);
    model.redo();
    assert_eq!(model.text.to_string(), a_edited);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), A_DISK.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), b_edited.as_bytes());
    assert_eq!(std::fs::read(&fixture.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn group_subset_saves_only_last_plain_dirty_target_and_retains_dirty_sticky_with_undo() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let b = fixture.member(fixture.b_id);
    let c = fixture.member(fixture.c_id);
    let a_edited = fixture.edited(a, "excluded λ🙂 ");
    fixture.pin(a);
    let b_edited = fixture.edited(b, "eligible λ🙂 ");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    start_batch_save(&mut fixture.app, b);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    assert_eq!(fixture.app.doc().id, b.document);
    assert!(fixture.app.editor_groups.membership_current(a));
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.editor_groups.membership_current(b));
    assert_eq!(std::fs::read(&fixture.b).unwrap(), b_edited.as_bytes());
    gates.release();
    until(
        &mut fixture.app,
        "group nonsticky subset completed",
        |app| !app.saves_pending() && app.modal.is_none(),
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert!(!fixture.app.editor_groups.membership_current(b));
    assert!(!fixture.app.editor_groups.membership_current(c));
    assert_eq!(fixture.app.editor_groups.groups().len(), 1);
    assert_eq!(fixture.app.active_tab_membership(), Some(a));
    assert_eq!(fixture.app.doc().text.to_string(), a_edited);
    assert_eq!(fixture.app.doc().save_generation(), 0);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), A_DISK);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), a_edited);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), A_DISK.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), b_edited.as_bytes());
    assert_eq!(std::fs::read(&fixture.c).unwrap(), C_DISK.as_bytes());
}

#[test]
fn standalone_save_pinning_never_cancels_actual_worker_or_baseline_publication() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let b = fixture.member(fixture.b_id);
    let captured = fixture.edited(a, "ordinary saved λ🙂 ");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut fixture.app, "workbench.action.files.save");
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.pin(a);
    command(&mut fixture.app, UNPIN);
    fixture.pin(a);
    fixture.focus(b);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.saving.worker.busy());
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    assert!(fixture.app.editor_groups.membership_current(a));
    gates.release();
    until(&mut fixture.app, "standalone saved receipt", |app| {
        !app.saves_pending()
    });
    assert!(fixture.app.editor_groups.membership_current(a));
    assert_eq!(fixture.app.active_tab_membership(), Some(b));
    let model = fixture.model_mut(fixture.a_id);
    assert_eq!(model.id, a.document);
    assert_eq!(model.save_generation(), 1);
    assert_eq!(model.text.to_string(), captured);
    assert!(!model.dirty());
    model.undo();
    assert_eq!(model.text.to_string(), A_DISK);
    model.redo();
    assert_eq!(model.text.to_string(), captured);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
}

#[test]
fn forced_close_cancel_and_newer_edit_preserve_dirty_sticky_original_and_its_redo() {
    let mut fixture = Fixture::new();
    let a = fixture.member(fixture.a_id);
    let captured = fixture.edited(a, "captured λ🙂 ");
    fixture.pin(a);
    command(&mut fixture.app, FORCE_CLOSE);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    answer(&mut fixture.app, 'c');
    assert!(fixture.app.modal.is_none() && !fixture.app.saves_pending());
    assert!(fixture.app.editor_groups.membership_current(a));
    fixture.assert_original_disks();
    let gates = fixture.gates(vec![GatePoint::BeforeCommit]);
    command(&mut fixture.app, FORCE_CLOSE);
    answer(&mut fixture.app, 's');
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture
        .app
        .execute("type", json!({"text":"newer e\u{301}🙂 "}));
    let newer = fixture.app.doc().text.to_string();
    assert_ne!(newer, captured);
    gates.release();
    until(
        &mut fixture.app,
        "forced snapshot preserves newer dirty model",
        |app| !app.saves_pending(),
    );
    assert!(fixture.app.editor_groups.membership_current(a));
    assert!(is_sticky(&fixture.app, a));
    assert_eq!(fixture.app.doc().id, a.document);
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().text.to_string(), newer);
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), captured);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), newer);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
}

#[test]
fn newly_pinned_reopened_same_document_cannot_inherit_stale_membership_close() {
    let mut fixture = Fixture::new();
    let original = fixture.member(fixture.a_id);
    let captured = fixture.edited(original, "shared captured λ🙂 ");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    command(&mut fixture.app, "workbench.action.files.save");
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    // First attach the close to the sole membership's pending save. A subsequent
    // split lets an explicit second close remove the original view immediately.
    command(&mut fixture.app, CLOSE);
    assert!(fixture.app.editor_groups.membership_current(original));
    assert!(fixture.app.modal.is_none());
    command(&mut fixture.app, "workbench.action.splitEditor");
    let shared = fixture.app.active_tab_membership().unwrap();
    fixture.focus(original);
    command(&mut fixture.app, CLOSE);
    assert!(!fixture.app.editor_groups.membership_current(original));
    fixture.app.open(&fixture.a).unwrap();
    until(
        &mut fixture.app,
        "same model reopened in original group",
        |app| {
            app.active_tab_membership()
                .is_some_and(|member| member.document == original.document && member != shared)
        },
    );
    let reopened = fixture.app.active_tab_membership().unwrap();
    assert_eq!(reopened.group, original.group);
    assert_ne!(reopened.tab, original.tab);
    fixture.pin(reopened);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert!(fixture.app.editor_groups.membership_current(reopened));
    gates.release();
    until(
        &mut fixture.app,
        "original save receipt cannot close replacement",
        |app| !app.saves_pending(),
    );
    assert!(fixture.app.editor_groups.membership_current(reopened));
    assert!(fixture.app.editor_groups.membership_current(shared));
    assert_eq!(fixture.model(fixture.a_id).id, original.document);
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 1);
    assert!(!fixture.model(fixture.a_id).dirty());
    assert_eq!(fixture.app.active_tab_membership(), Some(reopened));
    let model = fixture.model_mut(fixture.a_id);
    model.undo();
    assert_eq!(model.text.to_string(), A_DISK);
    model.redo();
    assert_eq!(model.text.to_string(), captured);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B_DISK.as_bytes());
}
