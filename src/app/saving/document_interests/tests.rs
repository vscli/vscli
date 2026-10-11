//! Actual single-worker persistence tests. A private test-only Caller factory
//! supplies the native core caller lease without starting Node. Host lifetime,
//! protocol answer ordering and ALS are separate gateway qualification.
use super::*;
use crate::{document::Selection, save_worker::GatePoint};
use std::{
    fs,
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    thread,
};
const A: &str = "猫🙂 A\r\nlast\r\n";
const B: &str = "β🙂 B\r\n";
const WAIT: Duration = Duration::from_secs(5);
struct Fixture {
    _dir: tempfile::TempDir,
    app: App,
    a: PathBuf,
    b: PathBuf,
    a_id: u64,
    b_id: u64,
}
impl Fixture {
    fn new(hidden: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        fs::write(&a, A).unwrap();
        fs::write(&b, B).unwrap();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.settings = crate::settings::Settings::from_values(serde_json::json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off",
            "editor.formatOnSave":false,"editor.codeActionsOnSave":{}
        }).as_object().unwrap().clone(), "native document save fixture").unwrap();
        let a_id;
        if hidden {
            let mut doc = Document::open(&a).unwrap();
            app.settings.apply(&mut doc);
            a_id = doc.id;
            app.hidden_documents.push(doc);
        } else {
            app.open(&a).unwrap();
            wait(&mut app, |app| {
                app.documents
                    .get(app.active)
                    .is_some_and(|doc| doc.path.as_ref() == Some(&a))
            });
            a_id = app.doc().id;
        }
        app.open(&b).unwrap();
        wait(&mut app, |app| {
            app.documents
                .get(app.active)
                .is_some_and(|doc| doc.path.as_ref() == Some(&b))
        });
        let b_id = app.doc().id;
        Self {
            _dir: dir,
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
        let (worker, entered, release) = Worker::fixture_gated(points);
        self.app.replace_save_worker_fixture(worker);
        Gates { entered, release }
    }
    fn save(&mut self, request: &str, id: u64) -> Interest {
        match self
            .app
            .request_document_save_interest(Caller::fixture(17, request, "test.owner"), id)
            .unwrap()
        {
            Admission::Enqueued(interest) => interest,
            Admission::NeedsPath(_) => panic!("Expected named save"),
        }
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
            app.poll_native_saves();
            match self.entered.try_recv() {
                Ok(actual) => {
                    assert_eq!(actual, point);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    panic!("Gate stopped {point:?}: {}", app.message)
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(Instant::now() < deadline, "Gate {point:?}: {}", app.message);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn release(&self) {
        self.release.try_send(()).unwrap();
    }
}
fn wait(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    while !predicate(app) {
        app.poll();
        assert!(Instant::now() < deadline, "{}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}
fn finish(app: &mut App) -> Completion {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll_native_saves();
        if let Some(done) = app.take_document_save_completion() {
            return done;
        }
        assert!(Instant::now() < deadline, "Receipt: {}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn inactive_and_hidden_save_awaits_actual_join_and_keeps_dirty_active_editor() {
    for hidden in [false, true] {
        let mut f = Fixture::new(hidden);
        let a_id = f.a_id;
        let b_id = f.b_id;
        f.model_mut(a_id).insert("captured λ ", false);
        let captured = f.model(a_id).text.clone();
        f.model_mut(b_id).insert("B dirty ", false);
        let b_text = f.model(b_id).text.clone();
        let b_selection = f.model(b_id).selections();
        let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::AfterFinishReply]);
        let interest = f.save("save-a", a_id);
        gates.reach(&mut f.app, GatePoint::BeforeCommit);
        assert!(f.app.take_document_save_completion().is_none());
        assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
        gates.release();
        gates.reach(&mut f.app, GatePoint::AfterFinishReply);
        assert_eq!(fs::read(&f.a).unwrap(), captured.to_string().as_bytes());
        assert_eq!(f.model(a_id).save_generation(), 0);
        assert!(f.app.take_document_save_completion().is_none());
        assert!(f.app.native_save_worker_busy());
        gates.release();
        let done = finish(&mut f.app);
        assert_eq!(done.interest, interest);
        assert_eq!(done.caller.request, "save-a");
        assert_eq!(
            done.receipt,
            Receipt::Committed {
                document: a_id,
                save_generation: 1
            }
        );
        assert_eq!(f.app.doc().id, b_id);
        assert_eq!(f.model(b_id).text, b_text);
        assert_eq!(f.model(b_id).selections(), b_selection);
        assert!(f.model(b_id).dirty());
        assert_eq!(fs::read(&f.b).unwrap(), B.as_bytes());
        assert!(!f.model(a_id).dirty());
        assert_eq!(f.model(a_id).id, a_id);
        f.model_mut(b_id).undo();
        assert_eq!(f.model(b_id).text.to_string(), B);
        f.model_mut(b_id).redo();
        assert_eq!(f.model(b_id).text, b_text);
    }
}
#[test]
fn preauthorization_edit_undo_rejects_same_revision_and_preserves_redo() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    f.model_mut(id).insert("seed ", false);
    let before = f.model(id).text.clone();
    let revision = f.model(id).revision;
    let selections = f.model(id).selections();
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    f.save("aba", id);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    f.model_mut(id).insert("later ", false);
    f.model_mut(id).undo();
    assert_eq!(f.model(id).text, before);
    assert_eq!(f.model(id).revision, revision);
    f.app.poll_native_saves();
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.take_document_save_completion().is_none());
    gates.release();
    let done = finish(&mut f.app);
    assert_eq!(done.receipt, Receipt::NotSaved);
    assert_eq!(f.model(id).save_generation(), 0);
    assert_eq!(f.model(id).selections(), selections);
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
    f.model_mut(id).redo();
    assert_eq!(f.model(id).text.to_string(), format!("seed later {A}"));
    f.model_mut(id).undo();
    assert_eq!(f.model(id).text, before);
}
#[test]
fn authorized_receipt_keeps_newer_work_dirty_and_undo_baseline_exact() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    f.model_mut(id).insert("seed ", false);
    let captured = f.model(id).text.clone();
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    f.save("newer", id);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    f.model_mut(id).insert("newer ", false);
    let newer = f.model(id).text.clone();
    let epoch = f.model(id).text_epoch();
    let selections = f.model(id).selections();
    gates.release();
    let done = finish(&mut f.app);
    assert_eq!(
        done.receipt,
        Receipt::Committed {
            document: id,
            save_generation: 1
        }
    );
    assert_eq!(f.model(id).text, newer);
    assert_eq!(f.model(id).text_epoch(), epoch);
    assert_eq!(f.model(id).selections(), selections);
    assert!(f.model(id).dirty());
    assert_eq!(fs::read(&f.a).unwrap(), captured.to_string().as_bytes());
    assert_eq!(
        f.model(id).capture_save(f.a.clone()).unwrap().baseline(),
        Some(&captured)
    );
    f.model_mut(id).undo();
    assert_eq!(f.model(id).text, captured);
    assert!(!f.model(id).dirty());
    f.model_mut(id).redo();
    assert_eq!(f.model(id).text, newer);
    assert!(f.model(id).dirty());
}
#[test]
fn retired_owner_after_authorization_never_cancels_commit_or_publishes_other_receipt() {
    let mut f = Fixture::new(true);
    let id = f.a_id;
    f.model_mut(id).insert("authorized ", false);
    let text = f.model(id).text.clone();
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    let old = f.save("old", id);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    f.app.retire_document_save_callers(17, "test.owner");
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.take_document_save_completion().is_none());
    gates.release();
    let done = finish(&mut f.app);
    assert_eq!(done.interest, old);
    assert!(done.retired);
    assert_eq!(done.caller.session, 17);
    assert_eq!(
        done.receipt,
        Receipt::Committed {
            document: id,
            save_generation: 1
        }
    );
    assert_eq!(fs::read(&f.a).unwrap(), text.to_string().as_bytes());
    assert!(f.app.take_document_save_completion().is_none());
    assert_eq!(f.model(id).save_generation(), 1);
}
#[test]
fn manual_supersession_waits_for_rejected_actual_save_then_closes_exact_interest() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    f.model_mut(id).insert("pending ", false);
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    let old = f.save("old", id);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    f.app.request_native_save(None).unwrap();
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.take_document_save_completion().is_none());
    gates.release();
    let done = finish(&mut f.app);
    assert_eq!(done.interest, old);
    assert_eq!(done.receipt, Receipt::NotSaved);
    let deadline = Instant::now() + WAIT;
    while f.app.saves_pending() {
        f.app.poll_native_saves();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
    assert!(f.model(id).dirty());
    assert_eq!(f.app.doc().id, f.b_id);
}
#[test]
fn untitled_path_cancel_and_accept_keep_original_model_and_other_editor() {
    let mut f = Fixture::new(false);
    f.app
        .execute("workbench.action.files.newUntitledFile", Value::Null);
    let id = f.app.doc().id;
    f.app.doc_mut().insert("猫🙂 new\r\n", false);
    let original = f.app.doc().text.clone();
    f.app.open(&f.b).unwrap();
    assert_eq!(f.app.doc().id, f.b_id);
    let Admission::NeedsPath(first) = f
        .app
        .request_document_save_interest(Caller::fixture(17, "cancel", "test.owner"), id)
        .unwrap()
    else {
        panic!("Expected path")
    };
    f.app.begin_document_save_path_prompt(first, id).unwrap();
    assert_eq!(f.app.doc().id, f.b_id);
    f.app.cancel_save_continuations();
    f.app.prompt = None;
    let done = f.app.take_document_save_completion().unwrap();
    assert_eq!(done.receipt, Receipt::NotSaved);
    assert_eq!(f.model(id).text, original);
    assert!(f.model(id).path.is_none());
    let Admission::NeedsPath(second) = f
        .app
        .request_document_save_interest(Caller::fixture(17, "accept", "test.owner"), id)
        .unwrap()
    else {
        panic!("Expected path")
    };
    let target = f.a.parent().unwrap().join("new.txt");
    f.app
        .accept_document_save_path(second, target.clone())
        .unwrap();
    let done = finish(&mut f.app);
    assert_eq!(
        done.receipt,
        Receipt::Committed {
            document: id,
            save_generation: 1
        }
    );
    assert_eq!(f.model(id).id, id);
    assert_eq!(f.model(id).path.as_ref(), Some(&target));
    assert_eq!(f.app.doc().id, f.b_id);
    assert_eq!(fs::read(target).unwrap(), original.to_string().as_bytes());
}
#[test]
fn eight_path_waits_duplicate_and_overflow_refuse_before_model_or_counter_change() {
    let mut f = Fixture::new(false);
    f.app
        .execute("workbench.action.files.newUntitledFile", Value::Null);
    let id = f.app.doc().id;
    f.app.doc_mut().insert("seed", false);
    let text = f.app.doc().text.clone();
    let epoch = f.app.doc().text_epoch();
    let selections = f.app.doc().selections();
    assert!(matches!(
        f.app
            .request_document_save_interest(Caller::fixture(17, "r0", "test.owner"), id)
            .unwrap(),
        Admission::NeedsPath(_)
    ));
    let first_counter = f.app.saving.document_interests.next;
    assert!(
        f.app
            .request_document_save_interest(Caller::fixture(17, "r0", "test.owner"), id)
            .unwrap_err()
            .to_string()
            .contains("Duplicate")
    );
    assert_eq!(f.app.saving.document_interests.next, first_counter);
    for index in 1..8 {
        assert!(matches!(
            f.app
                .request_document_save_interest(
                    Caller::fixture(17, &format!("r{index}"), "test.owner"),
                    id
                )
                .unwrap(),
            Admission::NeedsPath(_)
        ));
    }
    let counter = f.app.saving.document_interests.next;
    assert!(
        f.app
            .request_document_save_interest(Caller::fixture(17, "ninth", "test.owner"), id)
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    assert_eq!(f.app.saving.document_interests.next, counter);
    assert_eq!(f.app.doc().text, text);
    assert_eq!(f.app.doc().text_epoch(), epoch);
    assert_eq!(f.app.doc().selections(), selections);
    f.app.retire_document_save_callers(17, "test.owner");
    let mut count = 0;
    while let Some(done) = f.app.take_document_save_completion() {
        assert_eq!(done.receipt, Receipt::NotSaved);
        count += 1;
    }
    assert_eq!(count, 8);
    assert!(matches!(
        f.app
            .request_document_save_interest(Caller::fixture(18, "ninth", "test.owner"), id)
            .unwrap(),
        Admission::NeedsPath(_)
    ));
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), "");
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text, text);
}
#[test]
fn settings_profile_aba_before_prepared_refuses_without_losing_primary_or_redo() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    f.model_mut(id).set_selections(vec![Selection {
        anchor: Some(2),
        cursor: 0,
        desired_column: None,
    }]);
    let selections = f.model(id).selections();
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    f.save("settings", id);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    let original = f.app.settings.clone();
    f.app.invalidate_settings_profile().unwrap();
    f.app.settings = original;
    gates.release();
    let done = finish(&mut f.app);
    assert_eq!(done.receipt, Receipt::NotSaved);
    assert_eq!(f.model(id).selections(), selections);
    assert_eq!(f.model(id).text.to_string(), A);
    assert_eq!(f.model(id).save_generation(), 0);
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
}
#[test]
fn expired_unapproved_interest_retains_real_worker_slot_until_join() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    let interest = f.save("expired", id);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    f.app
        .saving
        .document_interests
        .entries
        .iter_mut()
        .find(|entry| entry.id == interest)
        .unwrap()
        .started = Instant::now() - PREAUTH - Duration::from_secs(1);
    f.app.poll_native_saves();
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.take_document_save_completion().is_none());
    gates.release();
    assert_eq!(finish(&mut f.app).receipt, Receipt::NotSaved);
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
    assert_eq!(f.model(id).save_generation(), 0);
}

#[test]
fn actual_external_conflict_never_reports_false_commit_or_overwrites_disk() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    f.model_mut(id).insert("seed ", false);
    let text = f.model(id).text.clone();
    let revision = f.model(id).revision;
    let epoch = f.model(id).text_epoch();
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    f.save("conflict", id);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    fs::write(&f.a, "foreign λ\r\n").unwrap();
    gates.release();
    let done = finish(&mut f.app);
    assert!(matches!(done.receipt, Receipt::Failed(_)));
    assert_eq!(fs::read(&f.a).unwrap(), "foreign λ\r\n".as_bytes());
    assert_eq!(f.model(id).text, text);
    assert_eq!(f.model(id).revision, revision);
    assert_eq!(f.model(id).text_epoch(), epoch);
    assert_eq!(f.model(id).save_generation(), 0);
    assert!(f.model(id).dirty());
    f.model_mut(id).undo();
    assert_eq!(f.model(id).text.to_string(), A);
    f.model_mut(id).redo();
    assert_eq!(f.model(id).text, text);
}

#[test]
fn aged_path_acceptance_starts_preauthorization_before_idle_dispatch() {
    let mut f = Fixture::new(false);
    f.app
        .execute("workbench.action.files.newUntitledFile", Value::Null);
    let id = f.app.doc().id;
    f.app.doc_mut().insert("猫🙂 path wait\r\n", false);
    let text = f.app.doc().text.clone();
    f.app.open(&f.b).unwrap();
    let Admission::NeedsPath(interest) = f
        .app
        .request_document_save_interest(Caller::fixture(17, "aged", "test.owner"), id)
        .unwrap()
    else {
        panic!("Expected path")
    };
    f.app
        .saving
        .document_interests
        .entries
        .iter_mut()
        .find(|entry| entry.id == interest)
        .unwrap()
        .started = Instant::now() - PREAUTH - Duration::from_secs(1);
    let target = f.a.parent().unwrap().join("aged.txt");
    assert!(!f.app.native_save_worker_busy());
    f.app
        .accept_document_save_path(interest, target.clone())
        .unwrap();
    assert_eq!(
        finish(&mut f.app).receipt,
        Receipt::Committed {
            document: id,
            save_generation: 1
        }
    );
    assert_eq!(fs::read(target).unwrap(), text.to_string().as_bytes());
    assert_eq!(f.app.doc().id, f.b_id);
}

#[test]
fn failed_worker_admission_rolls_back_exact_interest_and_never_writes_later() {
    let mut f = Fixture::new(false);
    // Use a real settled worker with a higher identity to force start refusal,
    // after enqueue has published the desired intent. No result is fabricated.
    let mut worker = Worker::default();
    worker
        .try_start(
            10,
            f.model(f.a_id).capture_save(f.a.clone()).unwrap(),
            vec![],
        )
        .unwrap();
    worker.reject(10);
    let deadline = Instant::now() + WAIT;
    while !matches!(worker.poll(), Some(SaveEvent::Finished { .. })) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    f.app.replace_save_worker_fixture(worker);
    f.app
        .execute("workbench.action.files.newUntitledFile", Value::Null);
    let id = f.app.doc().id;
    f.app.doc_mut().insert("never written", false);
    let text = f.app.doc().text.clone();
    let epoch = f.app.doc().text_epoch();
    let Admission::NeedsPath(interest) = f
        .app
        .request_document_save_interest(Caller::fixture(17, "failed", "test.owner"), id)
        .unwrap()
    else {
        panic!("Expected path")
    };
    let target = f.a.parent().unwrap().join("refused.txt");
    assert!(
        f.app
            .accept_document_save_path(interest, target.clone())
            .unwrap_err()
            .to_string()
            .contains("monotonically")
    );
    f.app.retire_document_save_interest(interest);
    assert_eq!(
        f.app.take_document_save_completion().unwrap().receipt,
        Receipt::NotSaved
    );
    for _ in 0..16 {
        f.app.poll_native_saves();
    }
    assert!(f.app.saving.latest.is_none());
    assert!(!f.app.native_save_worker_busy());
    assert!(!target.exists());
    assert_eq!(f.model(id).text, text);
    assert_eq!(f.model(id).text_epoch(), epoch);
    assert!(f.model(id).dirty());
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
}

#[test]
fn orphan_service_intent_cannot_dispatch_as_an_ordinary_save() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    f.model_mut(id).insert("orphan ", false);
    // A removed acknowledgement entry must fail closed at both ownership seams.
    f.app.saving.latest = Some(Intent {
        id: 19,
        document: id,
        destination: None,
        continuation: None,
        automatic: None,
        stage: Stage::Capture,
        notice: None,
        service: Some(Interest(99)),
    });
    assert!(f.app.check_document_save_intent(19).is_err());
    assert!(f.app.dispatch_native_save().is_err());
    for _ in 0..16 {
        f.app.poll_native_saves();
    }
    assert!(!f.app.native_save_worker_busy());
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
    assert!(f.model(id).dirty());
}

#[test]
fn orphan_service_worker_cannot_authorize_without_exact_interest() {
    let mut f = Fixture::new(false);
    let id = f.a_id;
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    let interest = f.save("missing-interest", id);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    f.app
        .saving
        .document_interests
        .entries
        .retain(|entry| entry.id != interest);
    let worker = f.app.saving.active.as_ref().unwrap().id;
    assert!(f.app.authorize_document_save_interest(worker).is_err());
    gates.release();
    let deadline = Instant::now() + WAIT;
    while f.app.native_save_worker_busy() {
        f.app.poll_native_saves();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fs::read(&f.a).unwrap(), A.as_bytes());
    assert_eq!(f.model(id).save_generation(), 0);
}
