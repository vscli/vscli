use super::*;
use crate::{document::Selection, save_worker::GatePoint};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::{
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

const ORIGINAL_A: &str = "猫🙂 A base\r\nlast\r\n";
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
        app.open(&a).unwrap();
        until(&mut app, "open A", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&a))
        });
        let a_id = app.doc().id;
        app.open(&b).unwrap();
        until(&mut app, "open B", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&b))
        });
        let b_id = app.doc().id;
        app.open(&a).unwrap();
        until(&mut app, "refocus A", |app| app.doc().id == a_id);
        Self {
            _directory: directory,
            app,
            a,
            b,
            a_id,
            b_id,
        }
    }
    fn model(&self, id: u64) -> &Document {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .find(|doc| doc.id == id)
            .unwrap()
    }
    fn model_mut(&mut self, id: u64) -> &mut Document {
        self.app
            .documents
            .iter_mut()
            .chain(&mut self.app.hidden_documents)
            .find(|doc| doc.id == id)
            .unwrap()
    }
    fn gates(&mut self, points: Vec<GatePoint>) -> Gates {
        assert!(!self.app.saves_pending());
        let (worker, entered, release) = Worker::fixture_gated(points);
        self.app.saving.worker = worker;
        Gates { entered, release }
    }
    fn save(&mut self) {
        self.app.execute("workbench.action.files.save", Value::Null);
        assert!(self.app.saves_pending(), "{}", self.app.message);
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
        let index = self
            .app
            .panes
            .iter()
            .position(|pane| pane.id == a_pane)
            .unwrap();
        self.app.focus_pane(index);
        (a_pane, b_pane)
    }
}
struct Gates {
    entered: Receiver<GatePoint>,
    release: SyncSender<()>,
}
impl Gates {
    fn reach(&self, app: &mut App, point: GatePoint) {
        let deadline = Instant::now() + WAIT;
        loop {
            app.poll();
            match self.entered.try_recv() {
                Ok(actual) => {
                    assert_eq!(actual, point);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    panic!("save gate disconnected before {point:?}: {}", app.message)
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "save never reached {point:?}: {}",
                app.message
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn release(&self) {
        self.release.try_send(()).unwrap();
    }
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn real_prepared_save_rejects_edit_undo_aba_without_losing_redo_or_disk_bytes() {
    let mut fixture = Fixture::new();
    let gates = fixture.gates(vec![GatePoint::BeforePrepared]);
    let epoch = fixture.app.doc().text_epoch();
    let selections = fixture.app.doc().selections();
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    fixture.app.doc_mut().insert("λ", false);
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    assert!(!fixture.app.doc().dirty());
    assert!(fixture.app.doc().text_epoch() > epoch);
    assert_eq!(fixture.app.doc().selections(), selections);
    gates.release();
    until(&mut fixture.app, "retire stale preparation", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    assert_eq!(fixture.model(fixture.a_id).id, fixture.a_id);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), format!("λ{ORIGINAL_A}"));
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_A);
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn authorized_snapshot_save_survives_newer_typing_and_active_editor_switch() {
    let mut fixture = Fixture::new();
    fixture.app.doc_mut().insert("captured ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.app.doc_mut().insert("newer ", false);
    let newer = fixture.app.doc().text.clone();
    let newer_epoch = fixture.app.doc().text_epoch();
    let newer_selections = fixture.app.doc().selections();
    fixture.app.open(&fixture.b).unwrap();
    until(
        &mut fixture.app,
        "switch to B during authorized save",
        |app| app.doc().id == fixture.b_id,
    );
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    assert_eq!(fixture.model(fixture.a_id).text, newer);
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    gates.release();
    until(
        &mut fixture.app,
        "publish original snapshot receipt",
        |app| !app.saves_pending(),
    );
    let a = fixture.model(fixture.a_id);
    assert_eq!(a.text, newer);
    assert_eq!(a.text_epoch(), newer_epoch);
    assert_eq!(a.selections(), newer_selections);
    assert_eq!(a.disk_content.as_ref(), Some(&captured));
    assert_eq!(a.save_generation(), 1);
    assert!(a.dirty());
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    assert_eq!(fixture.model(fixture.b_id).text.to_string(), ORIGINAL_B);
    assert_eq!(fixture.model(fixture.b_id).save_generation(), 0);
    fixture.model_mut(fixture.a_id).undo();
    assert_eq!(fixture.model(fixture.a_id).text, captured);
    assert!(!fixture.model(fixture.a_id).dirty());
    fixture.model_mut(fixture.a_id).redo();
    assert_eq!(fixture.model(fixture.a_id).text, newer);
    assert!(fixture.model(fixture.a_id).dirty());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn latest_intent_recaptures_after_receipt_and_finish_reply_retains_actual_capacity() {
    let mut fixture = Fixture::new();
    fixture.app.doc_mut().insert("first ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::AfterFinishReply]);
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.app.doc_mut().insert("second ", false);
    for _ in 0..32 {
        fixture.save();
    }
    assert_eq!(fixture.app.saving.next_id, 1);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::AfterFinishReply);
    fixture.app.doc_mut().insert("latest λ ", false);
    let latest = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    fixture.save();
    for _ in 0..128 {
        fixture.app.poll();
    }
    assert!(fixture.app.saving.worker.busy());
    assert_eq!(fixture.app.saving.next_id, 1);
    assert_eq!(
        fixture.app.saving.active.as_ref().unwrap().snapshot.text(),
        &captured
    );
    assert_eq!(fixture.app.doc().save_generation(), 0);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    gates.release();
    until(
        &mut fixture.app,
        "dispatch recaptured latest intent",
        |app| {
            app.saving
                .active
                .as_ref()
                .is_some_and(|active| active.id == 2)
        },
    );
    let second = &fixture.app.saving.active.as_ref().unwrap().snapshot;
    assert_eq!(second.text(), &latest);
    assert_eq!(second.text_epoch(), epoch);
    assert_eq!(second.save_generation(), 1);
    assert_eq!(second.baseline(), Some(&captured));
    until(&mut fixture.app, "publish latest save", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.app.saving.next_id, 2);
    assert_eq!(fixture.app.doc().text, latest);
    assert_eq!(fixture.app.doc().save_generation(), 2);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        latest.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn receipt_preserves_two_live_shared_views_and_preexisting_redo() {
    let mut fixture = Fixture::new();
    fixture.app.doc_mut().insert("captured λ ", false);
    let first_pane = fixture.app.panes[fixture.app.active_pane].id;
    fixture
        .app
        .execute("workbench.action.splitEditor", Value::Null);
    let second_pane = fixture.app.panes[fixture.app.active_pane].id;
    assert_ne!(first_pane, second_pane);
    assert_eq!(fixture.app.panes.len(), 2);
    assert!(
        fixture
            .app
            .panes
            .iter()
            .all(|pane| pane.document == fixture.a_id)
    );
    let end = fixture.app.doc().text.len_chars();
    fixture.app.doc_mut().move_to(end, false);
    fixture.app.doc_mut().insert("tail 🙂\r\n", false);
    let captured = fixture.app.doc().text.clone();
    fixture.app.doc_mut().insert("redo 猫", false);
    let redone = fixture.app.doc().text.clone();
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text, captured);
    fixture.app.doc_mut().set_selections(vec![
        Selection {
            cursor: 8,
            anchor: Some(3),
            desired_column: None,
        },
        Selection::caret(1),
    ]);
    fixture.app.focus_pane(0);
    fixture.app.doc_mut().set_selections(vec![
        Selection {
            cursor: 1,
            anchor: Some(6),
            desired_column: None,
        },
        Selection::caret(10),
    ]);
    let view_selections = |doc: &Document, id| {
        let view = doc.view_state(Some(id));
        (view.cursor, view.anchor, view.secondary.clone())
    };
    let first = view_selections(fixture.app.doc(), first_pane);
    let second = view_selections(fixture.app.doc(), second_pane);
    assert_ne!(first, second);
    let epoch = fixture.app.doc().text_epoch();
    let gates = fixture.gates(vec![GatePoint::BeforeCommit]);
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.app.focus_pane(1);
    gates.release();
    until(&mut fixture.app, "publish shared model receipt", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.app.panes.len(), 2);
    assert_eq!(fixture.app.panes[0].id, first_pane);
    assert_eq!(fixture.app.panes[1].id, second_pane);
    assert!(
        fixture
            .app
            .panes
            .iter()
            .all(|pane| pane.document == fixture.a_id)
    );
    assert_eq!(fixture.app.doc().id, fixture.a_id);
    assert_eq!(fixture.app.panes[fixture.app.active_pane].id, second_pane);
    assert_eq!(view_selections(fixture.app.doc(), first_pane), first);
    assert_eq!(view_selections(fixture.app.doc(), second_pane), second);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().text, captured);
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, redone);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text, captured);
    assert!(!fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_ne!(fixture.app.doc().text, captured);
    assert!(fixture.app.doc().dirty());
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn save_as_prompt_keeps_origin_across_pane_switch_and_shared_history() {
    let mut fixture = Fixture::new();
    let (a_pane, b_pane) = fixture.two_panes();
    fixture.app.doc_mut().insert("captured ", false);
    fixture.app.doc_mut().set_selections(vec![
        Selection {
            cursor: 1,
            anchor: Some(5),
            desired_column: None,
        },
        Selection::caret(8),
    ]);
    let captured = fixture.app.doc().text.clone();
    let selections = fixture.app.doc().selections();
    let destination = fixture.a.with_file_name("saved.json");
    let gates = fixture.gates(vec![GatePoint::BeforeCommit]);
    fixture
        .app
        .execute("workbench.action.files.saveAs", Value::Null);
    assert!(fixture.app.prompt.is_some());
    let index = fixture
        .app
        .panes
        .iter()
        .position(|pane| pane.id == b_pane)
        .unwrap();
    fixture.app.focus_pane(index);
    fixture.app.prompt.as_mut().unwrap().text.clear();
    fixture
        .app
        .event(Event::Paste(destination.to_string_lossy().into_owned()));
    key(&mut fixture.app, KeyCode::Enter);
    assert!(fixture.app.prompt.is_none());
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    assert!(!destination.exists());
    assert_eq!(
        fixture
            .app
            .saving
            .active
            .as_ref()
            .unwrap()
            .snapshot
            .document_id(),
        fixture.a_id
    );
    gates.release();
    until(&mut fixture.app, "originating Save As receipt", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    assert_eq!(
        fixture.model(fixture.a_id).path.as_ref(),
        Some(&destination)
    );
    assert_eq!(fixture.model(fixture.a_id).text, captured);
    assert_eq!(fixture.model(fixture.a_id).selections(), selections);
    assert!(!fixture.model(fixture.a_id).dirty());
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 1);
    assert!(
        fixture
            .app
            .panes
            .iter()
            .any(|pane| pane.id == a_pane && pane.document == fixture.a_id)
    );
    fixture.model_mut(fixture.a_id).undo();
    assert_eq!(fixture.model(fixture.a_id).text.to_string(), ORIGINAL_A);
    assert!(fixture.model(fixture.a_id).dirty());
    fixture.model_mut(fixture.a_id).redo();
    assert_eq!(fixture.model(fixture.a_id).text, captured);
    assert!(!fixture.model(fixture.a_id).dirty());
    assert_eq!(
        std::fs::read(destination).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn close_after_save_targets_original_pane_and_retains_postauthorization_edits() {
    for newer in [false, true] {
        let mut fixture = Fixture::new();
        let (a_pane, b_pane) = fixture.two_panes();
        fixture.app.doc_mut().insert("captured ", false);
        let captured = fixture.app.doc().text.clone();
        let gates = fixture.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
        fixture
            .app
            .execute("workbench.action.closeActiveEditor", Value::Null);
        assert!(matches!(
            fixture.app.modal,
            Some(Modal::Confirm(AfterSave::Close))
        ));
        key(&mut fixture.app, KeyCode::Char('s'));
        gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
        if newer {
            fixture.app.doc_mut().insert("newer λ ", false);
        }
        let live = fixture.app.doc().text.clone();
        let index = fixture
            .app
            .panes
            .iter()
            .position(|pane| pane.id == b_pane)
            .unwrap();
        fixture.app.focus_pane(index);
        let b_selections = fixture.app.doc().selections();
        gates.release();
        gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
        assert!(fixture.app.panes.iter().any(|pane| pane.id == a_pane));
        assert_eq!(fixture.app.doc().id, fixture.b_id);
        gates.release();
        until(&mut fixture.app, "settle original pane close", |app| {
            !app.saves_pending()
        });
        assert_eq!(fixture.app.doc().id, fixture.b_id);
        assert_eq!(fixture.app.panes[fixture.app.active_pane].id, b_pane);
        assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL_B);
        assert_eq!(fixture.app.doc().selections(), b_selections);
        assert_eq!(fixture.app.doc().save_generation(), 0);
        assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
        assert_eq!(
            std::fs::read(&fixture.a).unwrap(),
            captured.to_string().as_bytes()
        );
        if newer {
            assert!(
                fixture
                    .app
                    .panes
                    .iter()
                    .any(|pane| pane.id == a_pane && pane.document == fixture.a_id)
            );
            assert_eq!(fixture.model(fixture.a_id).text, live);
            assert!(fixture.model(fixture.a_id).dirty());
            fixture.model_mut(fixture.a_id).undo();
            assert_eq!(fixture.model(fixture.a_id).text, captured);
            assert!(!fixture.model(fixture.a_id).dirty());
        } else {
            assert!(!fixture.app.panes.iter().any(|pane| pane.id == a_pane));
        }
    }
}

#[test]
fn clean_quit_waits_for_actual_committed_worker_finish_and_receipt() {
    let mut fixture = Fixture::new();
    let gates = fixture.gates(vec![GatePoint::BeforeFinish]);
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert_eq!(fixture.app.doc().save_generation(), 0);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    fixture.app.execute("workbench.action.quit", Value::Null);
    for _ in 0..128 {
        fixture.app.poll();
    }
    assert!(
        fixture.app.running,
        "quit cannot abandon a committed actual worker"
    );
    assert!(fixture.app.saves_pending());
    assert!(fixture.app.saving.worker.busy());
    assert_eq!(fixture.app.saving.next_id, 1);
    gates.release();
    until(&mut fixture.app, "quit after actual settlement", |app| {
        !app.running
    });
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 1);
    assert!(!fixture.model(fixture.a_id).dirty());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn hidden_dirty_hardlink_model_refuses_real_save_and_preserves_both_histories() {
    let mut fixture = Fixture::new();
    let alias = fixture.a.with_file_name("retained-hardlink.cpp");
    std::fs::hard_link(&fixture.a, &alias).unwrap();
    reject_hidden_alias(&mut fixture, alias);
}

#[cfg(unix)]
#[test]
fn hidden_recovered_raw_third_parent_alias_refuses_real_save() {
    let mut fixture = Fixture::new();
    let parent = fixture.app.workspace.root.join("first/second");
    std::fs::create_dir_all(&parent).unwrap();
    let alias = parent.join("third");
    std::os::unix::fs::symlink(&fixture.app.workspace.root, &alias).unwrap();
    reject_hidden_alias(&mut fixture, alias.join("a.cpp"));
}

fn reject_hidden_alias(fixture: &mut Fixture, alias: PathBuf) {
    let mut hidden = Document::from_text(ORIGINAL_A);
    hidden.path = Some(alias.clone());
    hidden.disk_content = Some(ropey::Rope::from_str(ORIGINAL_A));
    hidden.insert("hidden unsaved λ ", false);
    let hidden_id = hidden.id;
    let hidden_text = hidden.text.clone();
    let hidden_epoch = hidden.text_epoch();
    let hidden_selections = hidden.selections();
    fixture.app.hidden_documents.push(hidden);
    fixture.app.doc_mut().insert("visible unsaved 猫 ", false);
    let visible = fixture.app.doc().text.clone();
    let visible_epoch = fixture.app.doc().text_epoch();
    let gates = fixture.gates(vec![GatePoint::BeforePrepared]);
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    gates.release();
    until(
        &mut fixture.app,
        "reject conflicting retained alias",
        |app| !app.saves_pending(),
    );
    assert_eq!(fixture.model(fixture.a_id).text, visible);
    assert_eq!(fixture.model(fixture.a_id).text_epoch(), visible_epoch);
    assert!(fixture.model(fixture.a_id).dirty());
    assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
    assert_eq!(fixture.model(hidden_id).text, hidden_text);
    assert_eq!(fixture.model(hidden_id).text_epoch(), hidden_epoch);
    assert_eq!(fixture.model(hidden_id).selections(), hidden_selections);
    assert!(fixture.model(hidden_id).dirty());
    assert_eq!(fixture.model(hidden_id).save_generation(), 0);
    fixture.model_mut(hidden_id).undo();
    assert_eq!(fixture.model(hidden_id).text.to_string(), ORIGINAL_A);
    fixture.model_mut(hidden_id).redo();
    assert_eq!(fixture.model(hidden_id).text, hidden_text);
    fixture.model_mut(fixture.a_id).undo();
    assert_eq!(fixture.model(fixture.a_id).text.to_string(), ORIGINAL_A);
    fixture.model_mut(fixture.a_id).redo();
    assert_eq!(fixture.model(fixture.a_id).text, visible);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
    assert_eq!(std::fs::read(alias).unwrap(), ORIGINAL_A.as_bytes());
}

#[test]
fn authorized_save_did_save_uses_original_uri_and_exact_persisted_snapshot() {
    let mut fixture = Fixture::new();
    let peer = fixture.app.workspace.root.join("save-peer.py");
    let log = fixture.app.workspace.root.join("save-peer.jsonl");
    std::fs::write(&peer, r#"import json,sys
log=sys.argv[1]
def send(message):
    data=json.dumps({'jsonrpc':'2.0',**message}).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        k,v=line.decode().split(':',1);headers[k.lower()]=v.strip()
    message=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    with open(log,'a',encoding='utf-8') as file: file.write(json.dumps(message)+'\n')
    if message.get('method')=='initialize': send({'id':message['id'],'result':{'capabilities':{'textDocumentSync':{'openClose':True,'change':1,'save':{'includeText':True}}}}})
    elif message.get('method')=='shutdown': send({'id':message['id'],'result':None})
    elif message.get('method')=='exit': sys.exit(0)
"#).unwrap();
    fixture.app.lsp = Some(
        crate::lsp::Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                peer.to_string_lossy().into_owned(),
                log.to_string_lossy().into_owned(),
            ],
            &fixture.app.workspace.root,
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut fixture.app, "save peer ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    fixture.app.doc_mut().insert("persisted ", false);
    let captured = fixture.app.doc().text.clone();
    let gates = fixture.gates(vec![GatePoint::BeforeCommit]);
    fixture.save();
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.app.doc_mut().insert("newer unsaved λ ", false);
    fixture.app.open(&fixture.b).unwrap();
    until(
        &mut fixture.app,
        "switch B for native notification",
        |app| app.doc().id == fixture.b_id,
    );
    gates.release();
    until(&mut fixture.app, "receipt publishes didSave", |app| {
        !app.saves_pending()
    });
    let deadline = Instant::now() + WAIT;
    let saved = loop {
        let records: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let saved: Vec<Value> = records
            .into_iter()
            .filter(|record| record["method"] == "textDocument/didSave")
            .collect();
        if !saved.is_empty() {
            break saved;
        }
        fixture.app.poll();
        assert!(
            Instant::now() < deadline,
            "native didSave never reached peer"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0]["params"]["textDocument"]["uri"],
        json!(crate::lsp::file_uri(&fixture.a).unwrap())
    );
    assert_eq!(saved[0]["params"]["text"], json!(captured.to_string()));
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    assert!(fixture.model(fixture.a_id).dirty());
    assert_eq!(fixture.model(fixture.b_id).save_generation(), 0);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(std::fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
}

#[test]
fn authorized_preview_save_retains_undo_clean_model_before_new_preview_admission() {
    use crate::editor_groups::OpenMode;
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("preview-a.cpp");
    let b = root.path().join("preview-b.cpp");
    std::fs::write(&a, ORIGINAL_A).unwrap();
    std::fs::write(&b, ORIGINAL_B).unwrap();
    let mut app = App::new(root.path().into(), crate::keys::Profile::Linux);
    app.install_preview_document(Document::open(&a).unwrap(), OpenMode::Preview)
        .unwrap();
    let member = app.active_tab_membership().unwrap();
    let (worker, entered, release) = Worker::fixture_gated(vec![GatePoint::BeforeCommit]);
    app.saving.worker = worker;
    // A direct model transaction deliberately isolates admission's pending-save
    // guard from the normal input barrier that already commits dirty previews.
    app.doc_mut().insert("captured λ ", false);
    let captured = app.doc().text.to_string();
    app.request_native_save(None).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll_native_saves();
        match entered.try_recv() {
            Ok(point) => {
                assert_eq!(point, GatePoint::BeforeCommit);
                break;
            }
            Err(TryRecvError::Disconnected) => {
                panic!("Authorized preview save disconnected: {}", app.message)
            }
            Err(TryRecvError::Empty) => {}
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.saving.active.as_ref().unwrap().authorized);
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), ORIGINAL_A);
    assert!(!app.doc().dirty());
    assert!(app.document_save_pending(member.document));
    assert!(app.active_editor_is_preview());
    let epoch = app.doc().text_epoch();
    app.install_preview_document(Document::open(&b).unwrap(), OpenMode::Preview)
        .unwrap();
    assert!(app.editor_groups.membership_current(member));
    assert_eq!(app.documents.len(), 2);
    assert_ne!(app.doc().id, member.document);
    assert_eq!(std::fs::read(&a).unwrap(), ORIGINAL_A.as_bytes());
    release.send(()).unwrap();
    until(
        &mut app,
        "publish original authorized preview save",
        |app| !app.saves_pending(),
    );
    assert_eq!(std::fs::read(&a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&b).unwrap(), ORIGINAL_B.as_bytes());
    let retained = app
        .documents
        .iter()
        .find(|doc| doc.id == member.document)
        .unwrap();
    assert_eq!(retained.text.to_string(), ORIGINAL_A);
    assert_eq!(retained.text_epoch(), epoch);
    assert_eq!(retained.save_generation(), 1);
    assert!(retained.dirty());
    assert_eq!(
        app.editor_groups.group(member.group).unwrap().tabs().len(),
        2
    );
    assert!(
        app.editor_groups
            .group(member.group)
            .unwrap()
            .tabs()
            .iter()
            .find(|tab| tab.id() == member.tab)
            .is_some_and(|tab| !tab.is_preview())
    );
    app.focus_tab(member).unwrap();
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), captured);
    assert!(!app.doc().dirty());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), ORIGINAL_A);
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read(&a).unwrap(), captured.as_bytes());
    assert_eq!(std::fs::read(&b).unwrap(), ORIGINAL_B.as_bytes());
}
