//! Diagnostic assertions follow real worker gates; traces never control work.
use super::*;
use diagnostics::Stage;
use std::sync::mpsc::SyncSender;

fn until<T>(mut sample: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = sample() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "diagnostic worker fixture timed out"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn prepared(worker: &mut Worker) -> PreparedInfo {
    match until(|| worker.poll()) {
        Event::Prepared(info) => info,
        Event::Finished { result, .. } => panic!("unexpected preparation: {result:?}"),
    }
}
struct Release(SyncSender<()>);
impl Release {
    fn release(&self) {
        self.0.try_send(()).unwrap();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

#[test]
fn actual_missing_save_progress_separates_commit_cleanup_send_and_positive_exit() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("new.txt");
    let text = "猫🙂 diagnostic\r\nλ\r\n";
    let mut doc = crate::document::Document::from_text(text);
    let snapshot = doc.capture_save(target.clone()).unwrap();
    let (mut worker, entered, release) = Worker::fixture_gated(vec![
        GatePoint::BeforeCommit,
        GatePoint::BeforeFinish,
        GatePoint::AfterFinishReply,
    ]);
    let release = Release(release);
    worker.try_start(1, snapshot.clone(), vec![]).unwrap();
    let progress = worker.fixture_progress().unwrap().clone();
    let info = prepared(&mut worker);
    worker.authorize(info.id).unwrap();
    assert_eq!(
        until(|| {
            assert!(worker.poll().is_none());
            entered.try_recv().ok()
        }),
        GatePoint::BeforeCommit
    );
    let stages = || {
        progress
            .snapshot()
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>()
    };
    assert!(stages().contains(&Stage::AuthorizationReceived));
    assert!(!stages().contains(&Stage::CommitCalling));
    assert!(!target.exists());
    release.release();
    assert_eq!(
        until(|| {
            assert!(worker.poll().is_none());
            entered.try_recv().ok()
        }),
        GatePoint::BeforeFinish
    );
    assert_eq!(std::fs::read(&target).unwrap(), text.as_bytes());
    for stage in [
        Stage::CommitCalling,
        Stage::RecheckBegin,
        Stage::RecheckReturned,
        Stage::NoClobberBegin,
        Stage::NoClobberReturned,
        Stage::TemporaryCleanupBegin,
        Stage::TemporaryCleanupReturned,
        Stage::LockUnlockBegin,
        Stage::LockUnlockReturned,
        Stage::CommitCallReturned,
        Stage::WorkCommitted,
    ] {
        assert!(
            stages().contains(&stage),
            "missing actual {stage:?}: {}",
            worker.fixture_status()
        );
    }
    assert!(!stages().contains(&Stage::TerminalSending));
    assert!(worker.busy());
    release.release();
    assert_eq!(
        until(|| {
            assert!(worker.poll().is_none());
            entered.try_recv().ok()
        }),
        GatePoint::AfterFinishReply
    );
    assert!(stages().contains(&Stage::TerminalSent));
    assert!(!stages().contains(&Stage::WorkerScopeExit));
    assert!(worker.busy());
    assert_eq!(doc.save_generation(), 0);
    release.release();
    match until(|| worker.poll()) {
        Event::Finished {
            snapshot: returned,
            result: Ok(Outcome::Committed(commit)),
            ..
        } => {
            assert!(stages().contains(&Stage::WorkerScopeExit));
            assert!(!worker.busy());
            doc.publish_save(&returned, commit.path).unwrap();
        }
        other => panic!("unexpected final worker event: {other:?}"),
    }
    assert_eq!(doc.text.to_string(), text);
    assert_eq!(doc.save_generation(), 1);
    assert_eq!(std::fs::read(&target).unwrap(), text.as_bytes());
    assert!(
        progress
            .snapshot()
            .windows(2)
            .all(|pair| pair[0].1 <= pair[1].1)
    );
}

#[test]
fn actual_external_create_records_recheck_failure_without_inventing_no_clobber() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("new.txt");
    let mut doc = crate::document::Document::from_text("猫🙂 original\r\n");
    doc.move_to(doc.len(), false);
    doc.insert("retained λ", false);
    let changed = doc.text.to_string();
    doc.undo();
    let original = doc.text.to_string();
    let epoch = doc.text_epoch();
    let snapshot = doc.capture_save(target.clone()).unwrap();
    let (mut worker, entered, release) = Worker::fixture_gated(vec![GatePoint::BeforeCommit]);
    let release = Release(release);
    worker.try_start(1, snapshot, vec![]).unwrap();
    let progress = worker.fixture_progress().unwrap().clone();
    let info = prepared(&mut worker);
    worker.authorize(info.id).unwrap();
    assert_eq!(
        until(|| {
            assert!(worker.poll().is_none());
            entered.try_recv().ok()
        }),
        GatePoint::BeforeCommit
    );
    std::fs::write(&target, "external concurrent bytes").unwrap();
    release.release();
    assert!(matches!(
        until(|| worker.poll()),
        Event::Finished { result: Err(_), .. }
    ));
    let stages = progress
        .snapshot()
        .into_iter()
        .map(|r| r.0)
        .collect::<Vec<_>>();
    assert!(stages.contains(&Stage::RecheckReturned));
    assert!(stages.contains(&Stage::WorkError));
    assert!(stages.contains(&Stage::TemporaryCleanupReturned));
    assert!(stages.contains(&Stage::WorkerScopeExit));
    assert!(!stages.contains(&Stage::NoClobberBegin));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "external concurrent bytes"
    );
    assert_eq!(doc.text.to_string(), original);
    assert_eq!(doc.text_epoch(), epoch);
    assert_eq!(doc.save_generation(), 0);
    doc.redo();
    assert_eq!(doc.text.to_string(), changed);
}
