//! Actual interrupt/error shutdown boundary, including the final recovery write.
use super::*;
use crate::{recovery, save_worker::GatePoint};
use std::{
    fs,
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const ORIGINAL_A: &str = "猫🙂 A baseline\r\nlast\r\n";
const ORIGINAL_B: &str = "β🙂 B baseline\r\n";
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
        let root = fs::canonicalize(directory.path()).unwrap();
        let a = root.join("a.cpp");
        let b = root.join("b.cpp");
        fs::write(&a, ORIGINAL_A).unwrap();
        fs::write(&b, ORIGINAL_B).unwrap();
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
            "native shutdown fixture",
        )
        .unwrap();
        app.open(&a).unwrap();
        until(&mut app, |app| app.doc().path.as_ref() == Some(&a));
        let a_id = app.doc().id;
        app.open(&b).unwrap();
        until(&mut app, |app| app.doc().path.as_ref() == Some(&b));
        let b_id = app.doc().id;
        app.open(&a).unwrap();
        until(&mut app, |app| app.doc().id == a_id);
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
        let (worker, entered, release) = Worker::fixture_gated(points);
        assert!(!self.app.saves_pending());
        self.app.saving.worker = worker;
        Gates { entered, release }
    }
    fn focus_b(&mut self) {
        self.app.open(&self.b).unwrap();
        until(&mut self.app, |app| app.doc().id == self.b_id);
    }
    fn recovery_worker(&self) -> recovery::Worker {
        recovery::Worker::start(
            recovery::Recovery::new(self.a.parent().unwrap().join("recovery")).unwrap(),
        )
        .unwrap()
    }
    fn assert_cleanup(&self) {
        assert!(
            fs::read_dir(self.a.parent().unwrap())
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".tmp")),
            "Shutdown returned before adjacent temporary-file cleanup"
        );
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.a.with_file_name("a.cpp.vscli-write.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        lock.unlock().unwrap();
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
                Err(TryRecvError::Disconnected) => panic!("Save gate disconnected: {point:?}"),
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "Save gate {point:?}: {}",
                app.message
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    /// The caller has positively observed the current held boundary. Advance
    /// subsequent real boundaries while settle_persistence blocks on the UI.
    fn release_during_shutdown(self, remaining: Vec<GatePoint>) -> JoinHandle<()> {
        thread::spawn(move || {
            self.release.try_send(()).unwrap();
            for point in remaining {
                assert_eq!(self.entered.recv_timeout(WAIT).unwrap(), point);
                self.release.try_send(()).unwrap();
            }
        })
    }
}
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Shutdown fixture setup: {}",
            app.message
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn preserve_and_restore(fixture: &Fixture, worker: recovery::Worker) -> Vec<Document> {
    let refs: Vec<_> = fixture
        .app
        .documents
        .iter()
        .chain(&fixture.app.hidden_documents)
        .collect();
    worker.preserve_refs(&refs).unwrap();
    let store = recovery::Recovery::new(fixture.a.parent().unwrap().join("recovery")).unwrap();
    let (documents, consumed) = store.restore().unwrap();
    assert_eq!(
        consumed.len(),
        1,
        "Final recovery snapshot was not available"
    );
    recovery::Recovery::consume(&consumed);
    store.finish().unwrap();
    documents
}

#[test]
fn shutdown_settles_authorized_original_model_before_preserving_latest_recovery() {
    let stages = [
        GatePoint::BeforeCommit,
        GatePoint::BeforeFinish,
        GatePoint::AfterFinishReply,
    ];
    for (stage, &point) in stages.iter().enumerate() {
        let mut fixture = Fixture::new();
        fixture.app.doc_mut().insert("captured ", false);
        let captured = fixture.app.doc().text.clone();
        let gates = fixture.gates(stages.to_vec());
        fixture
            .app
            .request_native_save(Some(AfterSave::Close))
            .unwrap();
        gates.reach(&mut fixture.app, stages[0]);
        assert!(fixture.app.saving.active.as_ref().unwrap().authorized);
        fixture.app.doc_mut().insert("newer λ🙂 ", false);
        let newer = fixture.app.doc().text.clone();
        let epoch = fixture.app.doc().text_epoch();
        let selections = fixture.app.doc().selections();
        fixture.focus_b();
        for &next in stages.iter().take(stage + 1).skip(1) {
            gates.release.try_send(()).unwrap();
            gates.reach(&mut fixture.app, next);
        }
        assert_eq!(fixture.model(fixture.a_id).save_generation(), 0);
        assert!(
            fixture.app.saves_pending(),
            "Actual worker slot retired early at {point:?}"
        );
        // An earlier recovery request has the old baseline. The final real
        // preserve_refs must supersede it after publishing the save receipt.
        let mut recovery = fixture.recovery_worker();
        let refs: Vec<_> = fixture.app.documents.iter().collect();
        assert!(recovery.submit_refs(&refs).unwrap());
        let generation = fixture.app.saving.close_generation;
        let panes = fixture.app.panes.len();
        let release = gates.release_during_shutdown(stages[stage + 1..].to_vec());
        fixture.app.settle_persistence();
        release.join().unwrap();
        assert!(!fixture.app.persistence_pending());
        assert!(fixture.app.saving.active.is_none());
        assert!(fixture.app.saving.latest.is_none());
        assert!(fixture.app.saving.closing.is_none());
        assert!(fixture.app.saving.close_generation > generation);
        assert_eq!(fixture.app.panes.len(), panes);
        assert_eq!(fixture.app.documents.len(), 2);
        assert_eq!(fixture.app.doc().id, fixture.b_id);
        assert!(fixture.app.modal.is_none());
        let a = fixture.model(fixture.a_id);
        assert_eq!(a.text, newer);
        assert_eq!(a.text_epoch(), epoch);
        assert_eq!(a.selections(), selections);
        assert_eq!(a.disk_content.as_ref(), Some(&captured));
        assert_eq!(a.save_generation(), 1);
        assert!(a.dirty());
        assert_eq!(
            fs::read(&fixture.a).unwrap(),
            captured.to_string().as_bytes()
        );
        assert_eq!(fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
        fixture.assert_cleanup();
        let mut recovered = preserve_and_restore(&fixture, recovery);
        assert_eq!(recovered.len(), 1);
        let restored = &mut recovered[0];
        assert_eq!(restored.path.as_ref(), Some(&fixture.a));
        assert_eq!(restored.text, newer);
        assert_eq!(restored.selections(), selections);
        assert_eq!(restored.disk_content.as_ref(), Some(&captured));
        assert!(restored.dirty());
        restored.undo();
        assert_eq!(restored.text, captured);
        assert!(!restored.dirty());
        restored.redo();
        assert_eq!(restored.text, newer);
        assert!(restored.dirty());
        fixture.model_mut(fixture.a_id).undo();
        assert_eq!(fixture.model(fixture.a_id).text, captured);
        assert!(!fixture.model(fixture.a_id).dirty());
        fixture.model_mut(fixture.a_id).redo();
        assert_eq!(fixture.model(fixture.a_id).text, newer);
        assert!(fixture.model(fixture.a_id).dirty());
        assert_eq!(
            fs::read(&fixture.a).unwrap(),
            captured.to_string().as_bytes()
        );
    }
}

#[test]
fn shutdown_retires_unapproved_work_and_never_dispatches_latest_queued_intent() {
    for queued in [false, true] {
        let mut fixture = Fixture::new();
        fixture.app.doc_mut().insert("captured ", false);
        let gates = fixture.gates(vec![GatePoint::BeforePrepared, GatePoint::BeforeFinish]);
        fixture
            .app
            .request_native_save(Some(AfterSave::Close))
            .unwrap();
        gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
        assert!(!fixture.app.saving.active.as_ref().unwrap().authorized);
        fixture.app.doc_mut().insert("latest λ🙂 ", false);
        let latest_a = fixture.app.doc().text.clone();
        let epoch_a = fixture.app.doc().text_epoch();
        let selections_a = fixture.app.doc().selections();
        fixture.focus_b();
        fixture.app.doc_mut().insert("latest B🙂 ", false);
        let latest_b = fixture.app.doc().text.clone();
        let epoch_b = fixture.app.doc().text_epoch();
        if queued {
            fixture.app.request_native_save(None).unwrap();
            assert_eq!(
                fixture.app.saving.latest.as_ref().unwrap().document,
                fixture.b_id
            );
        }
        let recovery = fixture.recovery_worker();
        let release = gates.release_during_shutdown(vec![GatePoint::BeforeFinish]);
        fixture.app.settle_persistence();
        release.join().unwrap();
        assert!(!fixture.app.persistence_pending());
        assert_eq!(
            fixture.app.saving.next_id, 1,
            "Queued work launched during shutdown"
        );
        assert!(fixture.app.saving.latest.is_none());
        assert!(fixture.app.saving.active.is_none());
        assert_eq!(fixture.app.doc().id, fixture.b_id);
        assert_eq!(fixture.app.documents.len(), 2);
        assert!(fixture.app.modal.is_none());
        assert_eq!(fs::read(&fixture.a).unwrap(), ORIGINAL_A.as_bytes());
        assert_eq!(fs::read(&fixture.b).unwrap(), ORIGINAL_B.as_bytes());
        for (id, text, epoch) in [
            (fixture.a_id, &latest_a, epoch_a),
            (fixture.b_id, &latest_b, epoch_b),
        ] {
            let doc = fixture.model(id);
            assert_eq!(&doc.text, text);
            assert_eq!(doc.text_epoch(), epoch);
            assert_eq!(doc.save_generation(), 0);
            assert!(doc.dirty());
        }
        assert_eq!(fixture.model(fixture.a_id).selections(), selections_a);
        fixture.assert_cleanup();
        let mut restored = preserve_and_restore(&fixture, recovery);
        assert_eq!(restored.len(), 2);
        for (path, latest, baseline) in [
            (&fixture.a, &latest_a, ORIGINAL_A),
            (&fixture.b, &latest_b, ORIGINAL_B),
        ] {
            let doc = restored
                .iter_mut()
                .find(|doc| doc.path.as_ref() == Some(path))
                .unwrap();
            assert_eq!(&doc.text, latest);
            assert_eq!(doc.disk_content.as_ref().unwrap().to_string(), baseline);
            doc.undo();
            assert_eq!(doc.text.to_string(), baseline);
            assert!(!doc.dirty());
            doc.redo();
            assert_eq!(&doc.text, latest);
            assert!(doc.dirty());
        }
        fixture.model_mut(fixture.a_id).undo();
        assert_eq!(
            fixture.model(fixture.a_id).text.to_string(),
            format!("captured {ORIGINAL_A}")
        );
        fixture.model_mut(fixture.a_id).redo();
        assert_eq!(fixture.model(fixture.a_id).text, latest_a);
        fixture.model_mut(fixture.b_id).undo();
        assert_eq!(fixture.model(fixture.b_id).text.to_string(), ORIGINAL_B);
        fixture.model_mut(fixture.b_id).redo();
        assert_eq!(fixture.model(fixture.b_id).text, latest_b);
    }
}
