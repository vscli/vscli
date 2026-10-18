//! Native refactoring authority and exact receipts on the actual joined save worker.
use super::*;
use crate::save_worker::GatePoint;
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
    fn public_save(&mut self, request: &str, id: u64) -> Interest {
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
fn finish(app: &mut App, interest: Interest) -> Completion {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll_native_saves();
        if let Some(done) = app.take_refactoring_save_completion(interest) {
            return done;
        }
        assert!(Instant::now() < deadline, "Native receipt: {}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}
fn native_save(
    f: &mut Fixture,
    cohort: u64,
    lifetime: Arc<AtomicBool>,
    source: NativeSaveSource,
) -> Interest {
    match f
        .app
        .request_refactoring_save_interest(
            cohort,
            lifetime,
            Arc::new(AtomicBool::new(false)),
            source,
        )
        .unwrap()
    {
        Admission::Enqueued(id) => id,
        Admission::NeedsPath(_) => panic!("Expected named native save"),
    }
}

#[test]
fn native_hidden_save_uses_real_worker_without_host_and_requires_join_before_receipt() {
    let mut f = Fixture::new(true);
    let a = f.a_id;
    let b = f.b_id;
    f.model_mut(a).insert("refactor λ ", false);
    f.model_mut(b).insert("other dirty ", false);
    let text = f.model(a).text.clone();
    let other = f.model(b).capture_workspace_source().unwrap();
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    assert_eq!(source.document(), a);
    let flag = Arc::new(AtomicBool::new(false));
    let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::AfterFinishReply]);
    let interest = native_save(&mut f, 1, flag, source);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    f.app.retire_all_document_save_callers();
    f.app.retire_document_save_callers(0, "");
    assert!(f.app.document_save_completion().is_none());
    assert!(f.app.take_document_save_completion().is_none());
    assert!(f.app.refactoring_save_completion(interest).is_none());
    assert_eq!(fs::read_to_string(&f.a).unwrap(), A);
    gates.release();
    gates.reach(&mut f.app, GatePoint::AfterFinishReply);
    assert_eq!(fs::read_to_string(&f.a).unwrap(), text.to_string());
    assert_eq!(f.model(a).save_generation(), 0);
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.refactoring_save_completion(interest).is_none());
    gates.release();
    let done = finish(&mut f.app, interest);
    assert_eq!(done.interest, interest);
    assert!(done.caller.is_refactoring());
    assert_eq!(done.caller.session, 0);
    assert!(done.caller.owner.is_empty());
    assert!(done.caller.request.is_empty());
    assert!(!done.retired);
    assert_eq!(
        done.receipt,
        Receipt::Committed {
            document: a,
            save_generation: 1
        }
    );
    assert!(!f.model(a).dirty());
    assert!(other.current(f.model(b)));
    assert_eq!(f.app.doc().id, b);
    assert_eq!(fs::read_to_string(&f.b).unwrap(), B);
}

#[test]
fn native_lifetime_retirement_before_authorization_keeps_worker_until_join_and_disk_unchanged() {
    let mut f = Fixture::new(true);
    let a = f.a_id;
    f.model_mut(a).insert("unapproved ", false);
    let text = f.model(a).text.clone();
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    let interest = native_save(&mut f, 2, flag.clone(), source);
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    flag.store(true, Ordering::Release);
    f.app.poll_native_saves();
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.refactoring_save_completion(interest).is_none());
    assert!(f.app.take_document_save_completion().is_none());
    gates.release();
    let done = finish(&mut f.app, interest);
    assert!(done.retired);
    assert_eq!(done.receipt, Receipt::NotSaved);
    assert_eq!(f.model(a).text, text);
    assert_eq!(f.model(a).save_generation(), 0);
    assert_eq!(fs::read_to_string(&f.a).unwrap(), A);
    assert!(!f.app.native_save_worker_busy());
}

#[test]
fn native_lifetime_retirement_after_authorization_keeps_commit_and_newer_unsaved_work() {
    let mut f = Fixture::new(true);
    let a = f.a_id;
    f.model_mut(a).insert("authorized ", false);
    let captured = f.model(a).text.clone();
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    let interest = native_save(&mut f, 3, flag.clone(), source);
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    f.model_mut(a).insert("newer ", false);
    let current = f.model(a).text.clone();
    flag.store(true, Ordering::Release);
    f.app.poll_native_saves();
    assert!(f.app.native_save_worker_busy());
    assert!(f.app.refactoring_save_completion(interest).is_none());
    gates.release();
    let done = finish(&mut f.app, interest);
    assert!(done.retired);
    assert_eq!(
        done.receipt,
        Receipt::Committed {
            document: a,
            save_generation: 1
        }
    );
    assert_eq!(fs::read_to_string(&f.a).unwrap(), captured.to_string());
    assert_eq!(f.model(a).text, current);
    assert!(f.model(a).dirty());
    f.model_mut(a).undo();
    assert_eq!(f.model(a).text, captured);
    assert!(!f.model(a).dirty());
}

#[test]
fn deferred_native_source_cannot_recapture_edit_undo_or_changed_settings() {
    let mut f = Fixture::new(true);
    let a = f.a_id;
    f.model_mut(a).insert("original publication ", false);
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    let revision = f.model(a).revision;
    f.model_mut(a).insert("independent ", false);
    f.model_mut(a).undo();
    assert_eq!(f.model(a).revision, revision);
    assert!(source.current(&f.app).is_err());
    assert!(
        f.app
            .request_refactoring_save_interest(
                4,
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                source
            )
            .is_err()
    );
    assert!(f.app.saving.document_interests.entries.is_empty());
    assert!(!f.app.native_save_worker_busy());
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    f.app.settings = crate::settings::Settings::default();
    assert!(source.current(&f.app).is_err());
    assert!(
        f.app
            .request_refactoring_save_interest(
                5,
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                source
            )
            .is_err()
    );
    assert_eq!(fs::read_to_string(&f.a).unwrap(), A);
}

#[test]
fn native_authority_keeps_public_constructor_gates_and_exact_lifetime_identity() {
    let flag = Arc::new(AtomicBool::new(false));
    let local = Arc::new(AtomicBool::new(false));
    assert!(Caller::new(0, "request", "owner").is_err());
    assert!(Caller::new(1, "request", "").is_err());
    assert!(Caller::new(1, "", "owner").is_err());
    assert!(Caller::refactoring(0, flag.clone(), local.clone()).is_err());
    let native = Caller::refactoring(6, flag.clone(), local.clone()).unwrap();
    assert_eq!(
        native,
        Caller::refactoring(6, flag.clone(), local.clone()).unwrap()
    );
    assert_ne!(
        native,
        Caller::refactoring(6, Arc::new(AtomicBool::new(false)), local.clone()).unwrap()
    );
    assert_ne!(
        native,
        Caller::refactoring(7, flag.clone(), local.clone()).unwrap()
    );
    let mut f = Fixture::new(true);
    assert!(native.current(&f.app).is_ok());
    assert!(
        f.app
            .request_document_save_interest(native.clone(), f.a_id)
            .is_err()
    );
    flag.store(true, Ordering::Release);
    assert!(native.current(&f.app).is_err());
    assert!(Caller::refactoring(6, flag, local).is_err());
}

#[test]
fn targeted_native_receipts_do_not_steal_other_native_or_public_completions() {
    let mut f = Fixture::new(true);
    let a = f.a_id;
    let b = f.b_id;
    f.model_mut(a).insert("native ", false);
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    let first = native_save(&mut f, 8, Arc::new(AtomicBool::new(false)), source);
    let deadline = Instant::now() + WAIT;
    while f.app.refactoring_save_completion(first).is_none() {
        f.app.poll_native_saves();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(f.app.document_save_completion().is_none());
    assert!(f.app.take_document_save_completion().is_none());
    f.app.retire_all_document_save_callers();
    assert!(!f.app.refactoring_save_completion(first).unwrap().retired);
    assert!(
        f.app
            .take_refactoring_save_completion(Interest(u64::MAX))
            .is_none()
    );
    f.model_mut(b).insert("public ", false);
    let public = f.public_save("public-later", b);
    let deadline = Instant::now() + WAIT;
    while f.app.document_save_completion().is_none() {
        f.app.poll_native_saves();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(f.app.take_refactoring_save_completion(public).is_none());
    let received = f.app.take_document_save_completion().unwrap();
    assert_eq!(received.interest, public);
    assert!(!received.caller.is_refactoring());
    assert_eq!(
        f.app.refactoring_save_completion(first).unwrap().interest,
        first
    );
    assert_eq!(
        f.app
            .take_refactoring_save_completion(first)
            .unwrap()
            .interest,
        first
    );
    assert!(f.app.take_refactoring_save_completion(first).is_none());
    assert!(f.app.saving.document_interests.entries.is_empty());
}

#[test]
fn local_cohort_retirement_is_checked_at_worker_authority_without_retiring_original_owner() {
    let mut f = Fixture::new(true);
    let a = f.a_id;
    f.model_mut(a).insert("local pending ", false);
    let source = NativeSaveSource::capture(&f.app, a).unwrap();
    let owner = Arc::new(AtomicBool::new(false));
    let local = Arc::new(AtomicBool::new(false));
    let gates = f.gates(vec![GatePoint::BeforePrepared]);
    let Admission::Enqueued(interest) = f
        .app
        .request_refactoring_save_interest(9, owner.clone(), local.clone(), source)
        .unwrap()
    else {
        panic!("Expected named save");
    };
    gates.reach(&mut f.app, GatePoint::BeforePrepared);
    let worker = f.app.saving.active.as_ref().unwrap().id;
    local.store(true, Ordering::Release);
    assert!(!owner.load(Ordering::Acquire));
    assert!(
        f.app.authorize_document_save_interest(worker).is_err(),
        "Actual authorization must check both original flags"
    );
    f.app.poll_native_saves();
    assert!(f.app.native_save_worker_busy());
    gates.release();
    let done = finish(&mut f.app, interest);
    assert!(done.retired);
    assert_eq!(done.receipt, Receipt::NotSaved);
    assert!(!owner.load(Ordering::Acquire));
    assert_eq!(fs::read_to_string(&f.a).unwrap(), A);
}

#[test]
fn native_save_as_uses_original_untitled_proof_and_actual_publication_without_host() {
    let mut f = Fixture::new(true);
    let doc = Document::from_text("untitled 猫🙂\r\n");
    let document = doc.id;
    f.app.hidden_documents.push(doc);
    let source = NativeSaveSource::capture(&f.app, document).unwrap();
    let owner = Arc::new(AtomicBool::new(false));
    let local = Arc::new(AtomicBool::new(false));
    let Admission::NeedsPath(interest) = f
        .app
        .request_refactoring_save_interest(10, owner, local, source)
        .unwrap()
    else {
        panic!("Expected native Save As lease");
    };
    let destination = f.app.workspace.root.join("native-save-as.txt");
    f.app
        .accept_document_save_path(interest, destination.clone())
        .unwrap();
    let done = finish(&mut f.app, interest);
    assert_eq!(
        done.receipt,
        Receipt::Committed {
            document,
            save_generation: 1
        }
    );
    assert_eq!(f.model(document).path.as_ref(), Some(&destination));
    assert_eq!(
        fs::read_to_string(destination).unwrap(),
        "untitled 猫🙂\r\n"
    );
    assert_eq!(f.app.doc().id, f.b_id);
}

#[test]
fn eight_native_receipt_slots_are_bounded_and_generic_cleanup_cannot_release_them() {
    let mut f = Fixture::new(true);
    let owner = Arc::new(AtomicBool::new(false));
    let local = Arc::new(AtomicBool::new(false));
    let mut interests = Vec::new();
    for index in 0..8 {
        let doc = Document::from_text(&format!("target {index}"));
        let document = doc.id;
        f.app.hidden_documents.push(doc);
        let source = NativeSaveSource::capture(&f.app, document).unwrap();
        let Admission::NeedsPath(interest) = f
            .app
            .request_refactoring_save_interest(11, owner.clone(), local.clone(), source)
            .unwrap()
        else {
            panic!("Expected untitled native path slot");
        };
        interests.push(interest);
    }
    assert!(!f.app.refactoring_save_capacity());
    let source = NativeSaveSource::capture(&f.app, f.a_id).unwrap();
    assert!(
        f.app
            .request_refactoring_save_interest(11, owner.clone(), local.clone(), source)
            .unwrap_err()
            .to_string()
            .contains("limit reached (8)")
    );
    f.app.retire_all_document_save_callers();
    f.app.retire_document_save_callers(0, "");
    f.app.poll_native_saves();
    assert_eq!(f.app.saving.document_interests.entries.len(), 8);
    assert!(f.app.document_save_completion().is_none());
    local.store(true, Ordering::Release);
    f.app.poll_native_saves();
    assert!(
        !f.app.refactoring_save_capacity(),
        "Completed receipts retain actual metadata capacity until exact take"
    );
    for interest in interests {
        let done = f.app.take_refactoring_save_completion(interest).unwrap();
        assert!(done.retired);
        assert_eq!(done.receipt, Receipt::NotSaved);
    }
    assert!(f.app.refactoring_save_capacity());
    assert!(!owner.load(Ordering::Acquire));
}
