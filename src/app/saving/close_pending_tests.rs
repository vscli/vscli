//! Real save boundaries for closing before its successful receipt is published.
use super::*;
use crate::save_worker::GatePoint;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::{
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

const ORIGINAL_A: &str = "猫🙂 A baseline\r\nlast\r\n";
const ORIGINAL_B: &str = "B λ unchanged\r\n";
const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    a: PathBuf,
    b: PathBuf,
    a_id: u64,
    b_id: u64,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let a = root.join("a.cpp");
        let b = root.join("b.cpp");
        std::fs::write(&a, ORIGINAL_A).unwrap();
        std::fs::write(&b, ORIGINAL_B).unwrap();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.settings = crate::settings::Settings::from_values(
            serde_json::json!({
                "vscli.languageServer.enabled":false,
                "breadcrumbs.enabled":false,
                "files.autoSave":"off"
            })
            .as_object()
            .unwrap()
            .clone(),
            "pending-close fixture",
        )
        .unwrap();
        app.open(&b).unwrap();
        until(&mut app, "open B", |app| {
            app.doc().path.as_ref() == Some(&b)
        });
        let b_id = app.doc().id;
        app.open(&a).unwrap();
        until(&mut app, "open A", |app| {
            app.doc().path.as_ref() == Some(&a)
        });
        let a_id = app.doc().id;
        Self {
            _directory: directory,
            app,
            a,
            b,
            a_id,
            b_id,
        }
    }
    fn gate(&mut self, points: Vec<GatePoint>) -> Gates {
        assert!(!self.app.saves_pending());
        let (worker, entered, release) = Worker::fixture_gated(points);
        self.app.saving.worker = worker;
        Gates { entered, release }
    }
    fn two_panes(&mut self) -> (u64, u64) {
        // These persistence cases need one sole membership per original
        // model. Committed tabs now retain historical membership, so explicitly
        // close B's first-group tab before constructing the second group.
        let b = self
            .app
            .editor_groups
            .memberships(self.b_id)
            .next()
            .unwrap();
        self.app.focus_tab(b).unwrap();
        self.app
            .execute("workbench.action.closeActiveEditor", Value::Null);
        self.app
            .execute("workbench.action.splitEditor", Value::Null);
        self.app.open(&self.b).unwrap();
        until(&mut self.app, "B in second pane", |app| {
            app.doc().path.as_ref() == Some(&self.b)
        });
        self.b_id = self.app.doc().id;
        let shared_a = self
            .app
            .editor_groups
            .memberships(self.a_id)
            .find(|member| Some(member.group) == self.app.editor_groups.active_group())
            .unwrap();
        self.app.focus_tab(shared_a).unwrap();
        self.app
            .execute("workbench.action.closeActiveEditor", Value::Null);
        assert_eq!(self.app.editor_groups.memberships(self.a_id).count(), 1);
        assert_eq!(self.app.editor_groups.memberships(self.b_id).count(), 1);
        let b_pane = self.app.panes[self.app.active_pane].id;
        let a_pane = self
            .app
            .panes
            .iter()
            .find(|pane| pane.document == self.a_id)
            .unwrap()
            .id;
        self.focus(a_pane);
        (a_pane, b_pane)
    }
    fn focus(&mut self, pane: u64) {
        let index = self
            .app
            .panes
            .iter()
            .position(|candidate| candidate.id == pane)
            .unwrap();
        self.app.focus_pane(index);
    }
    fn close(&mut self) {
        self.app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
        )));
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
                Ok(point) => {
                    assert_eq!(point, expected);
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
fn escape(app: &mut App) {
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
}

#[test]
fn save_as_visible_disk_before_receipt_defers_original_close_without_confirmation() {
    let mut fixture = Fixture::new();
    fixture.app.doc_mut().insert("captured λ ", false);
    let captured = fixture.app.doc().text.clone();
    let original_pane = fixture.app.panes[fixture.app.active_pane].id;
    let destination = fixture.a.with_file_name("saved.json");
    let gates = fixture.gate(vec![GatePoint::AfterFinishReply]);
    fixture
        .app
        .execute("workbench.action.files.saveAs", Value::Null);
    fixture.app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    fixture.app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    gates.reach(&mut fixture.app, GatePoint::AfterFinishReply);
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(fixture.app.doc().path.as_ref(), Some(&fixture.a));
    assert_eq!(fixture.app.doc().save_generation(), 0);
    fixture.close();
    assert!(
        fixture.app.modal.is_none(),
        "close must await the actual receipt"
    );
    let continuation = fixture
        .app
        .saving
        .active
        .as_ref()
        .unwrap()
        .continuation
        .as_ref()
        .unwrap();
    assert!(matches!(continuation.action, AfterSave::Close));
    assert_eq!(continuation.pane, Some(original_pane));
    assert_eq!(continuation.generation, fixture.app.saving.close_generation);
    assert!(fixture.app.saving.worker.busy());
    gates.release();
    until(
        &mut fixture.app,
        "publish Save As then close origin",
        |app| !app.saves_pending(),
    );
    assert!(
        !fixture
            .app
            .documents
            .iter()
            .any(|doc| doc.id == fixture.a_id)
    );
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    assert!(fixture.app.modal.is_none());
    fixture
        .app
        .execute("workbench.action.reopenClosedEditor", Value::Null);
    until(&mut fixture.app, "reopen saved origin", |app| {
        app.doc().path.as_ref() == Some(&destination)
    });
    assert_eq!(fixture.app.doc().text, captured);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn authorized_save_close_keeps_original_pane_ownership_across_switch_and_newer_edits() {
    for newer in [false, true] {
        let mut fixture = Fixture::new();
        let (a_pane, b_pane) = fixture.two_panes();
        fixture.app.doc_mut().insert("captured ", false);
        let captured = fixture.app.doc().text.clone();
        let gates = fixture.gate(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
        fixture
            .app
            .execute("workbench.action.files.save", Value::Null);
        gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
        fixture.close();
        assert!(fixture.app.modal.is_none());
        if newer {
            fixture.app.doc_mut().insert("newer 猫🙂 ", false);
        }
        let latest = fixture.app.doc().text.clone();
        let epoch = fixture.app.doc().text_epoch();
        let selections = fixture.app.doc().selections();
        fixture.focus(b_pane);
        gates.release();
        gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
        assert!(fixture.app.panes.iter().any(|pane| pane.id == a_pane));
        assert_eq!(fixture.app.doc().id, fixture.b_id);
        gates.release();
        until(
            &mut fixture.app,
            "publish original close continuation",
            |app| !app.saves_pending(),
        );
        assert_eq!(fixture.app.doc().id, fixture.b_id);
        assert_eq!(fixture.app.panes[fixture.app.active_pane].id, b_pane);
        assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_B);
        assert!(!fixture.app.doc().dirty());
        assert_eq!(fixture.app.doc().save_generation(), 0);
        assert_eq!(
            std::fs::read(&fixture.a).unwrap(),
            captured.to_string().as_bytes()
        );
        assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
        if newer {
            assert!(fixture.app.panes.iter().any(|pane| pane.id == a_pane));
            assert!(fixture.app.message.contains("newer unsaved edits"));
            fixture.focus(a_pane);
            assert_eq!(fixture.app.doc().id, fixture.a_id);
            assert_eq!(fixture.app.doc().text, latest);
            assert_eq!(fixture.app.doc().text_epoch(), epoch);
            assert_eq!(fixture.app.doc().selections(), selections);
            assert!(fixture.app.doc().dirty());
            assert_eq!(fixture.app.doc().save_generation(), 1);
            fixture.app.doc_mut().undo();
            assert_eq!(fixture.app.doc().text, captured);
            assert!(!fixture.app.doc().dirty());
            fixture.app.doc_mut().redo();
            assert_eq!(fixture.app.doc().text, latest);
        } else {
            assert!(!fixture.app.panes.iter().any(|pane| pane.id == a_pane));
        }
    }
}

#[test]
fn escape_cancels_attached_close_but_preserves_actual_save_and_history() {
    let mut fixture = Fixture::new();
    fixture.app.doc_mut().insert("captured ", false);
    let captured = fixture.app.doc().text.clone();
    let pane = fixture.app.panes[fixture.app.active_pane].id;
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.close();
    assert!(
        fixture
            .app
            .saving
            .active
            .as_ref()
            .unwrap()
            .continuation
            .is_some()
    );
    let generation = fixture.app.saving.close_generation;
    escape(&mut fixture.app);
    assert!(fixture.app.saving.close_generation > generation);
    assert!(
        fixture
            .app
            .saving
            .active
            .as_ref()
            .unwrap()
            .continuation
            .is_none()
    );
    assert!(fixture.app.saving.worker.busy());
    gates.release();
    until(
        &mut fixture.app,
        "publish save after canceling close",
        |app| !app.saves_pending(),
    );
    assert_eq!(fixture.app.doc().id, fixture.a_id);
    assert_eq!(fixture.app.panes[fixture.app.active_pane].id, pane);
    assert_eq!(fixture.app.doc().text, captured);
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, captured);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn close_attaches_to_latest_matching_intent_and_escape_retires_its_continuation() {
    for cancel in [false, true] {
        let mut fixture = Fixture::new();
        fixture.app.doc_mut().insert("first ", false);
        let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
        fixture
            .app
            .execute("workbench.action.files.save", Value::Null);
        gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
        fixture.app.doc_mut().insert("latest λ🙂 ", false);
        let latest = fixture.app.doc().text.clone();
        fixture
            .app
            .execute("workbench.action.files.save", Value::Null);
        fixture.close();
        assert!(fixture.app.modal.is_none());
        assert!(
            fixture
                .app
                .saving
                .active
                .as_ref()
                .unwrap()
                .continuation
                .is_none()
        );
        let queued = fixture.app.saving.latest.as_ref().unwrap();
        assert_eq!(queued.document, fixture.a_id);
        assert!(matches!(
            queued.continuation.as_ref().unwrap().action,
            AfterSave::Close
        ));
        if cancel {
            escape(&mut fixture.app);
            assert!(
                fixture
                    .app
                    .saving
                    .latest
                    .as_ref()
                    .unwrap()
                    .continuation
                    .is_none()
            );
        }
        gates.release();
        until(
            &mut fixture.app,
            "publish recaptured latest close save",
            |app| !app.saves_pending(),
        );
        assert_eq!(fixture.app.saving.next_id, 2);
        assert_eq!(
            std::fs::read(&fixture.a).unwrap(),
            latest.to_string().as_bytes()
        );
        assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
        assert!(fixture.app.modal.is_none());
        if cancel {
            assert_eq!(fixture.app.doc().id, fixture.a_id);
            assert_eq!(fixture.app.doc().text, latest);
            assert_eq!(fixture.app.doc().save_generation(), 2);
            assert!(!fixture.app.doc().dirty());
        } else {
            assert!(
                !fixture
                    .app
                    .documents
                    .iter()
                    .any(|doc| doc.id == fixture.a_id)
            );
            assert_eq!(fixture.app.doc().id, fixture.b_id);
        }
    }
}

#[test]
fn pending_close_does_not_replace_existing_quit_continuation() {
    let mut fixture = Fixture::new();
    fixture.app.doc_mut().insert("captured ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture.app.execute("workbench.action.quit", Value::Null);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Quit))
    ));
    fixture.app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
    )));
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.close();
    assert!(fixture.app.modal.is_none());
    assert!(matches!(
        fixture
            .app
            .saving
            .active
            .as_ref()
            .unwrap()
            .continuation
            .as_ref()
            .unwrap()
            .action,
        AfterSave::Quit
    ));
    gates.release();
    until(
        &mut fixture.app,
        "preserve broader quit after receipt",
        |app| !app.running,
    );
    assert!(!fixture.app.saves_pending());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn pending_save_closes_original_inactive_tab_without_closing_its_group() {
    let mut fixture = Fixture::new();
    let origin = fixture.app.active_tab_membership().unwrap();
    fixture.app.doc_mut().insert("owned 猫🙂 ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.close();
    assert!(fixture.app.modal.is_none());
    let other = fixture
        .app
        .editor_groups
        .memberships(fixture.b_id)
        .next()
        .unwrap();
    assert_eq!(origin.group, other.group);
    fixture.app.focus_tab(other).unwrap();
    let selection = fixture.app.doc().selections();
    gates.release();
    until(&mut fixture.app, "inactive original tab receipt", |app| {
        !app.saves_pending()
    });
    assert!(!fixture.app.editor_groups.membership_current(origin));
    assert!(fixture.app.editor_groups.membership_current(other));
    assert_eq!(fixture.app.active_tab_membership(), Some(other));
    assert_eq!(fixture.app.editor_groups.groups().len(), 1);
    assert_eq!(fixture.app.doc().selections(), selection);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn pending_save_cannot_close_reopened_membership_of_same_shared_model() {
    let mut fixture = Fixture::new();
    let original = fixture.app.active_tab_membership().unwrap();
    fixture.app.doc_mut().insert("owned ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.close();
    fixture
        .app
        .execute("workbench.action.splitEditor", Value::Null);
    let shared = fixture.app.active_tab_membership().unwrap();
    fixture.app.focus_tab(original).unwrap();
    fixture.close(); // Shared dirty close removes this view without discard.
    assert!(!fixture.app.editor_groups.membership_current(original));
    assert!(fixture.app.editor_groups.membership_current(shared));
    fixture.app.open(&fixture.a).unwrap();
    let reopened = fixture.app.active_tab_membership().unwrap();
    assert_eq!(reopened.group, original.group);
    assert_eq!(reopened.document, original.document);
    assert_ne!(reopened.tab, original.tab);
    gates.release();
    until(&mut fixture.app, "retired original tab receipt", |app| {
        !app.saves_pending()
    });
    assert!(fixture.app.editor_groups.membership_current(reopened));
    assert!(fixture.app.editor_groups.membership_current(shared));
    assert_eq!(fixture.app.active_tab_membership(), Some(reopened));
    assert_eq!(fixture.app.doc().text, captured);
    assert!(!fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, captured);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
}

#[test]
fn changed_group_membership_retires_batch_after_owned_save_without_sweeping_new_tabs() {
    let mut fixture = Fixture::new();
    let original = fixture.app.active_tab_membership().unwrap();
    fixture.app.doc_mut().insert("owned ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture
        .app
        .execute("workbench.action.closeEditorsInGroup", Value::Null);
    assert!(matches!(
        fixture.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    fixture.app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
    )));
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture
        .app
        .execute("workbench.action.files.newUntitledFile", Value::Null);
    fixture.app.doc_mut().insert("new unsaved Ω", false);
    let newcomer = fixture.app.active_tab_membership().unwrap();
    gates.release();
    until(&mut fixture.app, "retired group batch receipt", |app| {
        !app.saves_pending()
    });
    assert!(fixture.app.closing_group.is_none());
    assert!(fixture.app.modal.is_none());
    assert!(!fixture.app.editor_groups.membership_current(original));
    assert!(fixture.app.editor_groups.membership_current(newcomer));
    assert!(
        fixture
            .app
            .editor_groups
            .memberships(fixture.b_id)
            .next()
            .is_some()
    );
    assert_eq!(fixture.app.active_tab_membership(), Some(newcomer));
    assert_eq!(fixture.app.doc().text.to_string(), "new unsaved Ω");
    assert!(fixture.app.doc().dirty());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn close_group_waits_for_authorized_save_even_after_undo_makes_origin_clean() {
    let mut fixture = Fixture::new();
    let original = fixture.app.active_tab_membership().unwrap();
    fixture.app.doc_mut().insert("captured dirty λ ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.app.doc_mut().undo();
    assert!(!fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    fixture
        .app
        .execute("workbench.action.closeEditorsInGroup", Value::Null);
    assert!(fixture.app.modal.is_none());
    assert!(fixture.app.editor_groups.membership_current(original));
    assert!(
        fixture
            .app
            .documents
            .iter()
            .any(|doc| doc.id == fixture.a_id)
    );
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    gates.release();
    until(
        &mut fixture.app,
        "close-group pending original baseline receipt",
        |app| !app.saves_pending(),
    );
    assert!(fixture.app.editor_groups.membership_current(original));
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    assert!(fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(fixture.app.message.contains("newer unsaved edits"));
    assert!(fixture.app.closing_group.is_none());
    assert!(fixture.app.close_membership.is_none());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, captured);
    assert!(!fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    assert!(fixture.app.doc().dirty());
    fixture.app.cancel_save_continuations();
}
