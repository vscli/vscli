//! Real settings preparation and native save gates share the destination lock.
use super::*;
use crate::{recovery, save_worker::GatePoint, settings_writer};
use std::{
    fs,
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(5);
const SETTINGS: &str = "{ // native profile 猫🙂\r\n  \"breadcrumbs.enabled\": true,\r\n  \"vscli.languageServer.enabled\": false,\r\n  \"files.autoSave\": \"off\"\r\n}\r\n";
const ORIGINAL: &str = "猫🙂 source baseline\r\nlast\r\n";

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    settings: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let settings = root.join("settings.json");
        let source = root.join("source.cpp");
        fs::write(&settings, SETTINGS).unwrap();
        fs::write(&source, ORIGINAL).unwrap();
        let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
        app.extension_node = root.join("node-unavailable").to_string_lossy().into_owned();
        app.configure_settings(Some(settings.clone())).unwrap();
        Self {
            _directory: directory,
            app,
            settings,
            source,
        }
    }
    fn prepared_settings(&mut self) -> settings_writer::PreparedInfo {
        self.app.request_persistent_breadcrumbs(false).unwrap();
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(info) = self.app.fixture_take_settings_prepared() {
                assert_eq!(info.path, self.settings);
                assert_eq!(fs::read(&self.settings).unwrap(), SETTINGS.as_bytes());
                assert!(self.app.settings_writes_busy());
                return info;
            }
            assert!(Instant::now() < deadline, "No actual settings preparation");
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn gate(&mut self, points: Vec<GatePoint>) -> Gates {
        let (worker, entered, release) = Worker::fixture_gated(points);
        assert!(!self.app.saves_pending());
        self.app.saving.worker = worker;
        Gates { entered, release }
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
                Err(TryRecvError::Disconnected) => panic!("Native gate disconnected"),
                Err(TryRecvError::Empty) => {}
            }
            assert!(Instant::now() < deadline, "{point:?}: {}", app.message);
            thread::sleep(Duration::from_millis(1));
        }
    }
}
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}
fn lock(path: &Path) -> fs::File {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".vscli-write.lock");
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_file_name(name))
        .unwrap()
}
fn assert_cleanup(root: &Path, targets: &[&Path]) {
    assert!(fs::read_dir(root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp")
    }));
    for target in targets {
        let lock = lock(target);
        lock.try_lock().unwrap();
        lock.unlock().unwrap();
    }
}

#[test]
fn real_settings_preparation_serializes_native_save_and_preserves_dirty_settings_undo() {
    let mut fixture = Fixture::new();
    fixture.app.open(&fixture.settings).unwrap();
    until(&mut fixture.app, |app| {
        app.doc().path.as_ref() == Some(&fixture.settings)
    });
    let id = fixture.app.doc().id;
    let info = fixture.prepared_settings();
    let held_lock = lock(&fixture.settings);
    assert!(matches!(
        held_lock.try_lock(),
        Err(fs::TryLockError::WouldBlock)
    ));
    assert_eq!(info.matching_models, vec![id]);
    fixture.app.doc_mut().insert("// unsaved λ🙂\r\n", false);
    let dirty = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    let selections = fixture.app.doc().selections();
    let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.app.request_native_save(None).unwrap();
    assert!(fixture.app.saves_pending());
    assert!(fixture.app.settings_writes_busy());
    let refusal = fixture
        .app
        .request_persistent_breadcrumbs(true)
        .unwrap_err();
    assert!(refusal.to_string().contains("unsaved changes"));
    assert_eq!(fs::read(&fixture.settings).unwrap(), SETTINGS.as_bytes());
    assert_eq!(fixture.app.doc().id, id);
    assert_eq!(fixture.app.doc().text, dirty);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    // Retire the positively observed settings preparation. Native staging can
    // reach its real gate only after the other worker drops this shared lock.
    fixture.app.retire_unapproved_settings_write();
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    until(&mut fixture.app, |app| !app.settings_writes_busy());
    assert!(matches!(
        held_lock.try_lock(),
        Err(fs::TryLockError::WouldBlock)
    ));
    assert_eq!(fs::read(&fixture.settings).unwrap(), SETTINGS.as_bytes());
    gates.release.try_send(()).unwrap();
    until(&mut fixture.app, |app| !app.persistence_pending());
    assert_eq!(fixture.app.doc().id, id);
    assert_eq!(fixture.app.doc().text, dirty);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().selections(), selections);
    assert_eq!(fixture.app.doc().disk_content.as_ref(), Some(&dirty));
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(
        fs::read(&fixture.settings).unwrap(),
        dirty.to_string().as_bytes()
    );
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), SETTINGS);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, dirty);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(fs::read(&fixture.source).unwrap(), ORIGINAL.as_bytes());
    assert_cleanup(fixture.settings.parent().unwrap(), &[&fixture.settings]);
}

#[test]
fn shutdown_joins_both_authorized_lanes_before_recovery_uses_committed_native_baseline() {
    let mut fixture = Fixture::new();
    fixture.app.open(&fixture.source).unwrap();
    until(&mut fixture.app, |app| {
        app.doc().path.as_ref() == Some(&fixture.source)
    });
    let id = fixture.app.doc().id;
    fixture.app.doc_mut().insert("captured ", false);
    let captured = fixture.app.doc().text.clone();
    let points = vec![
        GatePoint::BeforeCommit,
        GatePoint::BeforeFinish,
        GatePoint::AfterFinishReply,
    ];
    let gates = fixture.gate(points);
    fixture
        .app
        .request_native_save(Some(AfterSave::Close))
        .unwrap();
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    assert!(fixture.app.saving.active.as_ref().unwrap().authorized);
    let settings = fixture.prepared_settings();
    fixture
        .app
        .fixture_authorize_settings_prepared(&settings)
        .unwrap();
    assert!(fixture.app.settings_writes_busy());
    assert!(fixture.app.saves_pending());
    fixture.app.doc_mut().insert("newer λ🙂 ", false);
    let newer = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    let selections = fixture.app.doc().selections();
    let root = fixture.source.parent().unwrap();
    let recovery_path = root.join("recovery");
    let recovery =
        recovery::Worker::start(recovery::Recovery::new(recovery_path.clone()).unwrap()).unwrap();
    let release = thread::spawn(move || {
        gates.release.try_send(()).unwrap();
        for next in [GatePoint::BeforeFinish, GatePoint::AfterFinishReply] {
            assert_eq!(gates.entered.recv_timeout(WAIT).unwrap(), next);
            gates.release.try_send(()).unwrap();
        }
    });
    fixture.app.settle_persistence();
    release.join().unwrap();
    assert!(!fixture.app.persistence_pending());
    assert!(!fixture.app.settings_writes_busy());
    assert!(fixture.app.saving.active.is_none());
    assert_eq!(fixture.app.documents.len(), 1);
    assert_eq!(fixture.app.doc().id, id);
    assert_eq!(fixture.app.doc().text, newer);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().selections(), selections);
    assert_eq!(fixture.app.doc().disk_content.as_ref(), Some(&captured));
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(fixture.app.doc().dirty());
    assert_eq!(
        fs::read(&fixture.source).unwrap(),
        captured.to_string().as_bytes()
    );
    assert_eq!(
        fs::read(&fixture.settings).unwrap(),
        SETTINGS.replace("true", "false").as_bytes()
    );
    assert_cleanup(root, &[&fixture.source, &fixture.settings]);
    recovery
        .preserve_refs(&fixture.app.recovery_documents())
        .unwrap();
    let store = recovery::Recovery::new(recovery_path).unwrap();
    let (mut restored, consumed) = store.restore().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].path.as_ref(), Some(&fixture.source));
    assert_eq!(restored[0].text, newer);
    assert_eq!(restored[0].disk_content.as_ref(), Some(&captured));
    assert_eq!(restored[0].selections(), selections);
    restored[0].undo();
    assert_eq!(restored[0].text, captured);
    assert!(!restored[0].dirty());
    restored[0].redo();
    assert_eq!(restored[0].text, newer);
    assert!(restored[0].dirty());
    recovery::Recovery::consume(&consumed);
    store.finish().unwrap();
}
