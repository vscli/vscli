use super::*;
use crate::{autosave::Policy, save_worker::GatePoint};
use serde_json::json;
use std::{
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

const ORIGINAL: &str = "猫🙂 base\r\nlast\r\n";
const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    path: PathBuf,
    profile: PathBuf,
    start: Instant,
}
impl Fixture {
    fn new(settings: Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let path = root.join("file.cpp");
        let profile = root.join("profile.json");
        std::fs::write(&path, ORIGINAL).unwrap();
        std::fs::write(&profile, settings.to_string()).unwrap();
        let start = Instant::now();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.saving.autosave_now = Some(start);
        app.configure_settings(Some(profile.clone())).unwrap();
        assert!(app.settings.warnings.is_empty());
        app.open(&path).unwrap();
        until(&mut app, "open configured file", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&path))
        });
        Self {
            _directory: directory,
            app,
            path,
            profile,
            start,
        }
    }
    fn clock(&mut self, millis: u64) {
        self.app.saving.autosave_now = Some(self.start + Duration::from_millis(millis));
        self.app.poll();
    }
    fn gate(&mut self, points: Vec<GatePoint>) -> Gates {
        assert!(!self.app.saves_pending());
        let (worker, entered, release) = Worker::fixture_gated(points);
        self.app.saving.worker = worker;
        Gates { entered, release }
    }
    fn reload(&mut self, settings: Value) {
        std::fs::write(&self.profile, settings.to_string()).unwrap();
        self.app
            .settings_loader
            .as_mut()
            .unwrap()
            .force_reload()
            .unwrap();
    }
    fn disk(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap()
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
fn after_delay() -> Value {
    json!({"files.autoSave":"afterDelay", "files.autoSaveDelay":1000})
}

#[test]
fn actual_default_debounce_restarts_after_edit_undo_even_when_bytes_match() {
    let mut fixture = Fixture::new(json!({"files.autoSave":"afterDelay"}));
    assert_eq!(
        fixture.app.settings.auto_save("cpp"),
        Policy::AfterDelay { delay_ms: 1000 }
    );
    fixture.app.doc_mut().insert("dirty λ ", false);
    let captured = fixture.app.doc().text.clone();
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.clock(999);
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    let epoch = fixture.app.doc().text_epoch();
    fixture.app.doc_mut().insert("temporary", false);
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text, captured);
    assert!(fixture.app.doc().text_epoch() > epoch);
    fixture.clock(999);
    fixture.clock(1998);
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    fixture.clock(1999);
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    assert_eq!(fixture.app.saving.next_id, 1);
    assert!(
        fixture
            .app
            .saving
            .active
            .as_ref()
            .unwrap()
            .automatic
            .is_some()
    );
    gates.release();
    until(&mut fixture.app, "publish debounce save", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.disk(), captured.to_string().as_bytes());
    assert!(!fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().save_generation(), 1);
}

#[test]
fn loaded_language_policies_and_untitled_models_do_not_bypass_eligibility() {
    for (settings, expected) in [
        (
            json!({"files.autoSave":"afterDelay", "[cpp]":{"files.autoSave":"off"}}),
            Policy::Off,
        ),
        (
            json!({"files.autoSave":"off", "[cpp]":{"files.autoSave":"afterDelay"}}),
            Policy::AfterDelay { delay_ms: 1000 },
        ),
    ] {
        let mut fixture = Fixture::new(settings);
        assert_eq!(fixture.app.settings.auto_save("cpp"), expected);
        let file_id = fixture.app.doc().id;
        fixture.app.doc_mut().insert("file dirty ", false);
        let captured = fixture.app.doc().text.clone();
        fixture
            .app
            .execute("workbench.action.files.newUntitledFile", Value::Null);
        fixture.app.doc_mut().insert("untitled 猫🙂\r\n", false);
        let untitled_id = fixture.app.doc().id;
        fixture.clock(0);
        let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
        fixture.clock(1000);
        if expected == Policy::Off {
            for _ in 0..64 {
                fixture.app.poll();
            }
            assert!(!fixture.app.saves_pending());
            assert_eq!(fixture.app.saving.next_id, 0);
            assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
        } else {
            gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
            assert_eq!(
                fixture
                    .app
                    .saving
                    .active
                    .as_ref()
                    .unwrap()
                    .snapshot
                    .document_id(),
                file_id
            );
            gates.release();
            until(&mut fixture.app, "save eligible file only", |app| {
                !app.saves_pending()
            });
            assert_eq!(fixture.disk(), captured.to_string().as_bytes());
            assert_eq!(fixture.app.saving.next_id, 1);
        }
        assert_eq!(fixture.app.doc().id, untitled_id);
        assert!(fixture.app.doc().dirty());
        assert!(fixture.app.doc().path.is_none());
        assert_eq!(fixture.app.doc().save_generation(), 0);
        assert!(fixture.app.prompt.is_none());
    }
}

#[test]
fn hidden_due_file_is_saved_while_active_file_keeps_typing() {
    let mut fixture = Fixture::new(after_delay());
    let hidden_path = fixture.path.with_file_name("hidden.cpp");
    std::fs::write(&hidden_path, "hidden λ\r\n").unwrap();
    let mut hidden = Document::open(&hidden_path).unwrap();
    hidden.insert("hidden dirty ", false);
    let hidden_id = hidden.id;
    let hidden_text = hidden.text.clone();
    fixture.app.hidden_documents.push(hidden);
    fixture.app.doc_mut().insert("hot ", false);
    let active_id = fixture.app.doc().id;
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    for millis in [400, 800, 1000] {
        fixture.app.doc_mut().insert("typing ", false);
        fixture.clock(millis);
    }
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    assert_eq!(
        fixture
            .app
            .saving
            .active
            .as_ref()
            .unwrap()
            .snapshot
            .document_id(),
        hidden_id
    );
    fixture.app.doc_mut().insert("newest 🙂 ", false);
    fixture.clock(1100);
    let active_text = fixture.app.doc().text.clone();
    let selections = fixture.app.doc().selections();
    gates.release();
    until(&mut fixture.app, "publish hidden save", |app| {
        !app.saves_pending()
    });
    assert_eq!(
        std::fs::read(&hidden_path).unwrap(),
        hidden_text.to_string().as_bytes()
    );
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    assert_eq!(fixture.app.doc().id, active_id);
    assert_eq!(fixture.app.doc().text, active_text);
    assert_eq!(fixture.app.doc().selections(), selections);
    assert!(fixture.app.doc().dirty());
    let hidden = fixture
        .app
        .hidden_documents
        .iter()
        .find(|doc| doc.id == hidden_id)
        .unwrap();
    assert!(!hidden.dirty());
    assert_eq!(hidden.save_generation(), 1);
    assert_eq!(fixture.app.saving.next_id, 1);
}

#[test]
fn profile_a_b_a_retires_unapproved_actual_automatic_work() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("dirty ", false);
    let dirty = fixture.app.doc().text.clone();
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.clock(1000);
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    let old_generation = fixture.app.saving.autosave_generation;
    let other = fixture.profile.with_file_name("other-profile.json");
    std::fs::write(&other, "{\"files.autoSave\":\"off\"}").unwrap();
    fixture.app.configure_settings(Some(other)).unwrap();
    fixture
        .app
        .configure_settings(Some(fixture.profile.clone()))
        .unwrap();
    assert!(fixture.app.saving.autosave_generation >= old_generation + 2);
    gates.release();
    until(&mut fixture.app, "retire old profile save", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.app.saving.next_id, 1);
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    assert_eq!(fixture.app.doc().text, dirty);
    assert!(fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().save_generation(), 0);
    fixture.clock(1999);
    assert!(!fixture.app.saves_pending());
}

#[test]
fn real_policy_reload_off_on_restarts_debounce_and_retires_cached_arc() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("dirty ", false);
    fixture.clock(0);
    let old = fixture
        .app
        .saving
        .autosave_settings
        .as_ref()
        .unwrap()
        .clone();
    let generation = fixture.app.saving.autosave_generation;
    fixture.clock(900);
    fixture.reload(json!({"files.autoSave":"off"}));
    until(&mut fixture.app, "load autosave off", |app| {
        app.settings.auto_save("cpp") == Policy::Off
    });
    assert!(fixture.app.saving.autosave_generation > generation);
    assert!(!std::sync::Arc::ptr_eq(
        &old,
        fixture.app.saving.autosave_settings.as_ref().unwrap()
    ));
    fixture.clock(5000);
    assert_eq!(fixture.app.saving.next_id, 0);
    fixture.reload(after_delay());
    until(&mut fixture.app, "load autosave on", |app| {
        app.settings.auto_save("cpp") == Policy::AfterDelay { delay_ms: 1000 }
    });
    fixture.clock(5999);
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.clock(6000);
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    gates.release();
    until(
        &mut fixture.app,
        "save after fresh loaded policy delay",
        |app| !app.saves_pending(),
    );
    assert_eq!(fixture.app.saving.next_id, 1);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn failed_automatic_snapshot_is_suppressed_but_original_manual_save_retries() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("dirty ", false);
    let captured = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture.clock(1000);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    std::fs::write(&fixture.path, "external update\r\n").unwrap();
    gates.release();
    until(
        &mut fixture.app,
        "retain failed automatic snapshot",
        |app| !app.saves_pending(),
    );
    assert_eq!(fixture.disk(), b"external update\r\n");
    assert_eq!(fixture.app.doc().text, captured);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert!(fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().save_generation(), 0);
    fixture.clock(10000);
    for _ in 0..64 {
        fixture.app.poll();
    }
    assert_eq!(fixture.app.saving.next_id, 1);
    assert!(!fixture.app.saves_pending());
    std::fs::write(&fixture.path, ORIGINAL).unwrap();
    let manual = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    manual.reach(&mut fixture.app, GatePoint::BeforePrepared);
    let active = fixture.app.saving.active.as_ref().unwrap();
    assert!(active.automatic.is_none());
    assert_eq!(active.snapshot.text_epoch(), epoch);
    assert_eq!(active.snapshot.text(), &captured);
    manual.release();
    until(&mut fixture.app, "publish explicit manual retry", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.disk(), captured.to_string().as_bytes());
    assert_eq!(fixture.app.saving.next_id, 2);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn new_edit_requalifies_a_failed_automatic_epoch_after_its_own_delay() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("dirty ", false);
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.clock(1000);
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    // Edit/Undo invalidates only this real unapproved epoch while retaining dirty bytes.
    fixture.app.doc_mut().insert("temporary ", false);
    fixture.app.doc_mut().undo();
    gates.release();
    until(&mut fixture.app, "reject stale automatic epoch", |app| {
        !app.saves_pending()
    });
    // Failed old work must not suppress a later new text epoch.
    fixture.app.doc_mut().insert("new epoch λ ", false);
    let newest = fixture.app.doc().text.clone();
    fixture.clock(1000);
    fixture.clock(1999);
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    let retry = fixture.gate(vec![GatePoint::BeforeCommit]);
    fixture.clock(2000);
    retry.reach(&mut fixture.app, GatePoint::BeforeCommit);
    assert_eq!(
        fixture.app.saving.active.as_ref().unwrap().snapshot.text(),
        &newest
    );
    retry.release();
    until(&mut fixture.app, "publish new epoch autosave", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.disk(), newest.to_string().as_bytes());
    assert_eq!(fixture.app.saving.next_id, 2);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn authorized_autosave_preserves_newer_dirty_text_then_debounces_next_snapshot() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("captured ", false);
    let captured = fixture.app.doc().text.clone();
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
    fixture.clock(1000);
    gates.reach(&mut fixture.app, GatePoint::BeforeCommit);
    fixture.app.doc_mut().insert("newer 🙂 ", false);
    let newest = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    let selections = fixture.app.doc().selections();
    fixture.clock(1200);
    gates.release();
    gates.reach(&mut fixture.app, GatePoint::BeforeFinish);
    assert_eq!(fixture.disk(), captured.to_string().as_bytes());
    assert_eq!(fixture.app.doc().save_generation(), 0);
    gates.release();
    until(
        &mut fixture.app,
        "publish older automatic snapshot",
        |app| !app.saves_pending(),
    );
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert_eq!(fixture.app.doc().text, newest);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().selections(), selections);
    assert!(fixture.app.doc().dirty());
    fixture.clock(2199);
    assert!(!fixture.app.saves_pending());
    let next = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.clock(2200);
    next.reach(&mut fixture.app, GatePoint::BeforePrepared);
    let snapshot = &fixture.app.saving.active.as_ref().unwrap().snapshot;
    assert_eq!(snapshot.text(), &newest);
    assert_eq!(snapshot.baseline(), Some(&captured));
    assert_eq!(snapshot.save_generation(), 1);
    next.release();
    until(&mut fixture.app, "publish newer debounce snapshot", |app| {
        !app.saves_pending()
    });
    assert_eq!(fixture.disk(), newest.to_string().as_bytes());
    assert_eq!(fixture.app.doc().save_generation(), 2);
    assert!(!fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text, captured);
    assert!(fixture.app.doc().dirty());
}

#[test]
fn actual_invalid_settings_reload_pauses_automation_until_valid_settings_arrive() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("dirty ", false);
    fixture.clock(0);
    std::fs::write(&fixture.profile, "{ malformed").unwrap();
    fixture
        .app
        .settings_loader
        .as_mut()
        .unwrap()
        .force_reload()
        .unwrap();
    until(&mut fixture.app, "load invalid actual settings", |app| {
        app.settings_error.is_some()
    });
    fixture.clock(10000);
    for _ in 0..64 {
        fixture.app.poll();
    }
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.app.saving.next_id, 0);
    assert!(fixture.app.doc().dirty());
    assert_eq!(fixture.disk(), ORIGINAL.as_bytes());
    fixture.reload(after_delay());
    until(&mut fixture.app, "load valid actual settings", |app| {
        app.settings_error.is_none()
    });
    fixture.clock(10999);
    assert!(!fixture.app.saves_pending());
    let gates = fixture.gate(vec![GatePoint::BeforePrepared]);
    fixture.clock(11000);
    gates.reach(&mut fixture.app, GatePoint::BeforePrepared);
    gates.release();
    until(
        &mut fixture.app,
        "save after recovered settings delay",
        |app| !app.saves_pending(),
    );
    assert!(!fixture.app.doc().dirty());
    assert_eq!(fixture.app.saving.next_id, 1);
}

#[test]
fn quit_retains_actual_autosave_capacity_and_never_dispatches_after_closing() {
    let mut fixture = Fixture::new(after_delay());
    fixture.app.doc_mut().insert("captured ", false);
    let captured = fixture.app.doc().text.clone();
    fixture.clock(0);
    let gates = fixture.gate(vec![GatePoint::AfterFinishReply]);
    fixture.clock(1000);
    gates.reach(&mut fixture.app, GatePoint::AfterFinishReply);
    assert_eq!(fixture.disk(), captured.to_string().as_bytes());
    assert_eq!(fixture.app.doc().save_generation(), 0);
    fixture.app.execute("workbench.action.quit", Value::Null);
    fixture.clock(10000);
    for _ in 0..64 {
        fixture.app.poll();
    }
    assert!(fixture.app.running);
    assert!(fixture.app.saving.worker.busy());
    assert!(fixture.app.saving.closing.is_some());
    assert_eq!(fixture.app.saving.next_id, 1);
    assert!(fixture.app.saving.latest.is_none());
    gates.release();
    until(
        &mut fixture.app,
        "quit after actual automatic finish",
        |app| !app.running,
    );
    assert!(!fixture.app.saves_pending());
    assert_eq!(fixture.app.saving.next_id, 1);
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(fixture.disk(), captured.to_string().as_bytes());
}
