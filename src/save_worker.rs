//! One actual native save worker; the UI retains at most one latest intent.
//! A queued intent must be recaptured when this worker positively finishes.
use crate::{
    document::SaveSnapshot,
    persistence::{self, Expected, Options, ParentPolicy, Target},
};
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const AUTHORIZATION: Duration = Duration::from_secs(6);

#[derive(Debug)]
pub struct PreparedInfo {
    pub id: u64,
    pub requested_path: PathBuf,
    pub canonical_path: PathBuf,
    pub matching_models: Vec<u64>,
    pub baseline_sha256: Option<[u8; 32]>,
    pub expires_at: Instant,
}
#[derive(Debug)]
pub enum Outcome {
    Committed(persistence::Commit),
    Rejected,
    Expired,
}
#[derive(Debug)]
pub enum Event {
    Prepared(PreparedInfo),
    Finished {
        id: u64,
        snapshot: SaveSnapshot,
        result: std::result::Result<Outcome, String>,
    },
}
enum WorkerEvent {
    Prepared(PreparedInfo),
    Finished(std::result::Result<Outcome, String>),
}
enum Decision {
    Authorize,
    Reject,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Awaiting,
    Committing,
    Retiring,
    Finishing,
}
struct Pending {
    id: u64,
    snapshot: SaveSnapshot,
    control: SyncSender<Decision>,
    receiver: Receiver<WorkerEvent>,
    thread: JoinHandle<()>,
    phase: Phase,
    deadline: Option<Instant>,
    finished: Option<std::result::Result<Outcome, String>>,
}
#[derive(Default)]
pub struct Worker {
    last_id: u64,
    pending: Option<Pending>,
    #[cfg(test)]
    gate: Option<FixtureGate>,
}
impl Worker {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// Observe only bounded scalar state; do not drain replies or wait for work.
    #[cfg(test)]
    pub(crate) fn fixture_status(&self) -> String {
        let Some(pending) = &self.pending else {
            return "worker=idle".into();
        };
        let phase = match pending.phase {
            Phase::Preparing => "preparing",
            Phase::Awaiting => "awaiting",
            Phase::Committing => "committing",
            Phase::Retiring => "retiring",
            Phase::Finishing => "finishing",
        };
        let deadline_remaining_ms = pending.deadline.map(|deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
        });
        format!(
            "worker(id={}, phase={phase}, thread_finished={}, terminal_reply={}, deadline_remaining_ms={deadline_remaining_ms:?})",
            pending.id,
            pending.thread.is_finished(),
            pending.finished.is_some(),
        )
    }
    pub fn awaiting_authorization(&self, id: u64) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.id == id
                && pending.phase == Phase::Awaiting
                && pending
                    .deadline
                    .is_some_and(|deadline| Instant::now() < deadline)
        })
    }
    pub fn try_start(
        &mut self,
        id: u64,
        snapshot: SaveSnapshot,
        inventory: Vec<(u64, PathBuf)>,
    ) -> Result<()> {
        self.start(id, snapshot, inventory, AUTHORIZATION)
    }
    fn start(
        &mut self,
        id: u64,
        snapshot: SaveSnapshot,
        inventory: Vec<(u64, PathBuf)>,
        authorization: Duration,
    ) -> Result<()> {
        ensure!(!self.busy(), "A native save worker is still active");
        ensure!(
            id > self.last_id,
            "Save request identity must increase monotonically"
        );
        persistence::validate_path(snapshot.target())?;
        if let Some(path) = snapshot.source_path() {
            persistence::validate_path(path)?;
        }
        persistence::validate_models(&inventory)?;
        ensure!(
            snapshot.text().len_bytes() <= persistence::MAX_BYTES
                && snapshot
                    .baseline()
                    .is_none_or(|rope| rope.len_bytes() <= persistence::MAX_BYTES),
            "Save snapshot exceeds the 32 MiB document budget"
        );
        let (sender, receiver) = mpsc::sync_channel(2);
        let (control, decisions) = mpsc::sync_channel(1);
        let captured = snapshot.clone();
        #[cfg(test)]
        let gate = self.gate.take();
        let thread = thread::Builder::new()
            .name("vscli-native-save".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(
                        id,
                        &captured,
                        inventory,
                        &sender,
                        &decisions,
                        authorization,
                        #[cfg(test)]
                        gate.as_ref(),
                    )
                }))
                .map_err(|_| "Native save worker panicked".to_owned())
                .and_then(|result| result.map_err(persistence::message));
                // All destination handles, tempfiles and lock guards are gone before
                // terminal notification. Actual capacity still waits for thread join.
                #[cfg(test)]
                if let Some(gate) = &gate {
                    let _ = gate.pause(GatePoint::BeforeFinish);
                    if gate.points.contains(&GatePoint::Disconnect) {
                        return;
                    }
                }
                let _ = sender.send(WorkerEvent::Finished(result));
                #[cfg(test)]
                if let Some(gate) = &gate {
                    let _ = gate.pause(GatePoint::AfterFinishReply);
                }
            })?;
        self.last_id = id;
        self.pending = Some(Pending {
            id,
            snapshot,
            control,
            receiver,
            thread,
            phase: Phase::Preparing,
            deadline: None,
            finished: None,
        });
        Ok(())
    }
    /// The UI must check the snapshot and all worker-resolved alias model proofs
    /// immediately before this nonblocking authorization send.
    pub fn authorize(&mut self, id: u64) -> Result<()> {
        let pending = self
            .pending
            .as_mut()
            .context("No native save preparation")?;
        ensure!(
            pending.id == id && pending.phase == Phase::Awaiting,
            "Native save preparation is no longer owned"
        );
        ensure!(
            pending
                .deadline
                .is_some_and(|deadline| Instant::now() < deadline),
            "Native save authorization expired"
        );
        pending
            .control
            .try_send(Decision::Authorize)
            .context("Native save worker is no longer awaiting authorization")?;
        pending.phase = Phase::Committing;
        Ok(())
    }
    /// Reject only unapproved work. Once authorized, a real commit may finish
    /// and its successful disk receipt must still be published to its owner.
    pub fn reject(&mut self, id: u64) -> bool {
        if let Some(pending) = &mut self.pending
            && pending.id == id
            && matches!(pending.phase, Phase::Preparing | Phase::Awaiting)
        {
            let _ = pending.control.try_send(Decision::Reject);
            pending.phase = Phase::Retiring;
            return true;
        }
        false
    }
    pub fn poll(&mut self) -> Option<Event> {
        for _ in 0..2 {
            let pending = self.pending.as_mut()?;
            if pending.phase == Phase::Finishing && pending.thread.is_finished() {
                let pending = self.pending.take().unwrap();
                let joined = pending.thread.join();
                let result = if joined.is_ok() {
                    pending
                        .finished
                        .unwrap_or_else(|| Err("Native save worker ended without a result".into()))
                } else {
                    Err("Native save worker panicked".into())
                };
                return Some(Event::Finished {
                    id: pending.id,
                    snapshot: pending.snapshot,
                    result,
                });
            }
            match pending.receiver.try_recv() {
                Ok(WorkerEvent::Prepared(info)) => {
                    if pending.phase == Phase::Preparing && Instant::now() < info.expires_at {
                        pending.phase = Phase::Awaiting;
                        pending.deadline = Some(info.expires_at);
                        return Some(Event::Prepared(info));
                    }
                    let _ = pending.control.try_send(Decision::Reject);
                    pending.phase = Phase::Retiring;
                }
                Ok(WorkerEvent::Finished(result)) => {
                    pending.finished = Some(result);
                    pending.phase = Phase::Finishing;
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    if pending.finished.is_none() {
                        pending.finished = Some(Err("Native save worker disconnected".into()));
                        pending.phase = Phase::Finishing;
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path};
    fn document(root: &Path) -> Document {
        let path = root.join("main.cpp");
        fs::write(&path, "猫🙂 baseline\r\n").unwrap();
        let mut doc = Document::open_existing(&path).unwrap();
        doc.move_to(2, false);
        doc.insert(" captured", false);
        doc
    }
    fn snapshot(doc: &Document) -> SaveSnapshot {
        doc.capture_save(doc.path.clone().unwrap()).unwrap()
    }
    fn next(worker: &mut Worker) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = worker.poll() {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "Native save worker did not settle"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn prepared(worker: &mut Worker, id: u64) -> PreparedInfo {
        match next(worker) {
            Event::Prepared(info) => {
                assert_eq!(info.id, id);
                info
            }
            Event::Finished { result, .. } => panic!("Unexpected finish: {result:?}"),
        }
    }
    fn finished(
        worker: &mut Worker,
        id: u64,
    ) -> (SaveSnapshot, std::result::Result<Outcome, String>) {
        match next(worker) {
            Event::Finished {
                id: actual,
                snapshot,
                result,
            } => {
                assert_eq!(actual, id);
                (snapshot, result)
            }
            Event::Prepared(info) => panic!("Unexpected preparation: {info:?}"),
        }
    }
    fn assert_cleanup(path: &Path) {
        let lock = path.with_file_name(format!(
            "{}.vscli-write.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .unwrap();
        file.try_lock().unwrap();
        file.unlock().unwrap();
        let mut paths: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        let mut expected = vec![path.to_path_buf(), lock];
        expected.sort();
        assert_eq!(paths, expected);
    }
    #[test]
    fn authorized_save_returns_original_snapshot_after_new_typing_and_preserves_dirty_undo() {
        let root = tempfile::tempdir().unwrap();
        let mut doc = document(root.path());
        let path = doc.path.clone().unwrap();
        let captured = snapshot(&doc);
        let saved_text = captured.text().to_string();
        let (mut worker, entered, release) = Worker::fixture_gated(vec![GatePoint::BeforeCommit]);
        worker.try_start(1, captured.clone(), vec![]).unwrap();
        let info = prepared(&mut worker, 1);
        doc.check_save_snapshot(&captured).unwrap();
        worker.authorize(1).unwrap();
        assert_eq!(
            entered.recv_timeout(Duration::from_secs(3)).unwrap(),
            GatePoint::BeforeCommit
        );
        doc.insert(" NEWER", false);
        let newer = doc.text.to_string();
        let epoch = doc.text_epoch();
        assert!(
            !worker.reject(1),
            "Authorized filesystem work cannot be canceled"
        );
        assert!(worker.try_start(2, snapshot(&doc), vec![]).is_err());
        assert_eq!(fs::read(&path).unwrap(), "猫🙂 baseline\r\n".as_bytes());
        release.send(()).unwrap();
        let (receipt, result) = finished(&mut worker, 1);
        let Outcome::Committed(commit) = result.unwrap() else {
            panic!("Expected commit");
        };
        assert_eq!(receipt.document_id(), doc.id);
        assert_eq!(receipt.text().to_string(), saved_text);
        assert_eq!(commit.path, info.canonical_path);
        assert_eq!(
            commit.sha256,
            <[u8; 32]>::from(Sha256::digest(saved_text.as_bytes()))
        );
        assert_eq!(fs::read(&path).unwrap(), saved_text.as_bytes());
        doc.publish_save(&receipt, commit.path).unwrap();
        assert_eq!(doc.text.to_string(), newer);
        assert_eq!(doc.text_epoch(), epoch);
        assert!(doc.dirty());
        doc.undo();
        assert_eq!(doc.text.to_string(), saved_text);
        assert!(!doc.dirty());
        doc.redo();
        assert_eq!(doc.text.to_string(), newer);
        assert!(doc.dirty());
        assert_cleanup(&path);
    }
    #[test]
    fn rejected_staged_work_keeps_one_actual_slot_and_allows_a_fresh_snapshot_after_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let mut doc = document(root.path());
        let path = doc.path.clone().unwrap();
        let original = fs::read(&path).unwrap();
        let (mut worker, entered, release) = Worker::fixture_gated(vec![GatePoint::BeforePrepared]);
        let captured = snapshot(&doc);
        worker.try_start(1, captured.clone(), vec![]).unwrap();
        assert_eq!(
            entered.recv_timeout(Duration::from_secs(3)).unwrap(),
            GatePoint::BeforePrepared
        );
        doc.insert(" then Undo", false);
        doc.undo();
        assert_eq!(doc.revision, captured.revision());
        assert!(doc.check_save_snapshot(&captured).is_err());
        assert!(worker.reject(1));
        for id in 2..34 {
            assert!(worker.try_start(id, snapshot(&doc), vec![]).is_err());
            assert!(worker.busy());
        }
        assert_eq!(fs::read(&path).unwrap(), original);
        release.send(()).unwrap();
        assert!(matches!(
            finished(&mut worker, 1).1.unwrap(),
            Outcome::Rejected
        ));
        assert!(!worker.busy());
        assert_cleanup(&path);
        worker.try_start(2, snapshot(&doc), vec![]).unwrap();
        prepared(&mut worker, 2);
        worker.authorize(2).unwrap();
        assert!(matches!(
            finished(&mut worker, 2).1.unwrap(),
            Outcome::Committed(_)
        ));
        assert_eq!(fs::read(&path).unwrap(), doc.text.to_string().as_bytes());
        assert!(worker.try_start(2, snapshot(&doc), vec![]).is_err());
        assert_cleanup(&path);
    }
    #[test]
    fn terminal_reply_does_not_release_capacity_until_the_actual_worker_thread_exits() {
        let root = tempfile::tempdir().unwrap();
        let doc = document(root.path());
        let path = doc.path.clone().unwrap();
        let (mut worker, entered, release) =
            Worker::fixture_gated(vec![GatePoint::AfterFinishReply]);
        worker.try_start(1, snapshot(&doc), vec![]).unwrap();
        prepared(&mut worker, 1);
        worker.authorize(1).unwrap();
        assert_eq!(
            entered.recv_timeout(Duration::from_secs(3)).unwrap(),
            GatePoint::AfterFinishReply
        );
        assert_eq!(fs::read(&path).unwrap(), doc.text.to_string().as_bytes());
        assert!(worker.poll().is_none());
        assert!(worker.busy());
        assert!(worker.try_start(2, snapshot(&doc), vec![]).is_err());
        assert_cleanup(&path);
        release.send(()).unwrap();
        assert!(matches!(
            finished(&mut worker, 1).1.unwrap(),
            Outcome::Committed(_)
        ));
        assert!(!worker.busy());
    }
    #[test]
    fn expired_authorization_and_wrong_request_ids_never_persist() {
        let root = tempfile::tempdir().unwrap();
        let doc = document(root.path());
        let path = doc.path.clone().unwrap();
        let original = fs::read(&path).unwrap();
        let mut worker = Worker::default();
        worker
            .start(1, snapshot(&doc), vec![], Duration::from_secs(1))
            .unwrap();
        prepared(&mut worker, 1);
        assert!(worker.authorize(99).is_err());
        assert!(!worker.reject(99));
        let (_, result) = finished(&mut worker, 1);
        assert!(matches!(result.unwrap(), Outcome::Expired));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_cleanup(&path);
    }
    #[test]
    fn actual_panic_and_disconnected_terminal_delivery_cleanup_before_capacity_release() {
        for point in [GatePoint::Panic, GatePoint::Disconnect] {
            let root = tempfile::tempdir().unwrap();
            let doc = document(root.path());
            let path = doc.path.clone().unwrap();
            let original = fs::read(&path).unwrap();
            let (mut worker, _entered, _release) = Worker::fixture_gated(vec![point]);
            worker.try_start(1, snapshot(&doc), vec![]).unwrap();
            if point == GatePoint::Disconnect {
                prepared(&mut worker, 1);
                worker.reject(1);
            }
            let (_, result) = finished(&mut worker, 1);
            assert!(result.unwrap_err().contains(if point == GatePoint::Panic {
                "panicked"
            } else {
                "disconnected"
            }));
            assert!(!worker.busy());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_cleanup(&path);
        }
    }
    #[test]
    fn save_as_recovers_a_retained_buffer_after_its_old_parent_is_deleted_without_clobber() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old-parent");
        fs::create_dir(&old).unwrap();
        let doc = document(&old);
        let old_path = doc.path.clone().unwrap();
        fs::remove_file(&old_path).unwrap();
        fs::remove_dir(&old).unwrap();
        let target = root.path().join("recovered.cpp");
        let mut worker = Worker::default();
        worker
            .try_start(1, doc.capture_save(target.clone()).unwrap(), vec![])
            .unwrap();
        prepared(&mut worker, 1);
        worker.authorize(1).unwrap();
        let (snapshot, result) = finished(&mut worker, 1);
        let Outcome::Committed(commit) = result.unwrap() else {
            panic!("Expected recovery");
        };
        assert_eq!(commit.path, fs::canonicalize(&target).unwrap());
        assert_eq!(
            fs::read(&target).unwrap(),
            snapshot.text().to_string().as_bytes()
        );
        assert!(!old.exists());
        worker
            .try_start(2, doc.capture_save(target.clone()).unwrap(), vec![])
            .unwrap();
        assert!(
            finished(&mut worker, 2)
                .1
                .unwrap_err()
                .contains("already exists")
        );
        assert_eq!(fs::read(&target).unwrap(), doc.text.to_string().as_bytes());
        assert_cleanup(&target);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending
            && matches!(
                pending.phase,
                Phase::Preparing | Phase::Awaiting | Phase::Retiring
            )
        {
            let _ = pending.control.try_send(Decision::Reject);
        }
    }
}
fn work(
    id: u64,
    snapshot: &SaveSnapshot,
    inventory: Vec<(u64, PathBuf)>,
    sender: &SyncSender<WorkerEvent>,
    decisions: &Receiver<Decision>,
    authorization: Duration,
    #[cfg(test)] gate: Option<&FixtureGate>,
) -> Result<Outcome> {
    let canonical = persistence::destination(snapshot.target())?;
    // A Save As to a new missing destination can recover a retained buffer
    // whose old parent disappeared. Never weaken ordinary same-path baseline
    // protection; unresolved historical paths only select no-clobber creation.
    let same_path = snapshot.source_path().is_some_and(|source| {
        source == snapshot.target()
            || persistence::destination(source).is_ok_and(|path| path == canonical)
    });
    let expected = if same_path {
        Expected::Exact(snapshot.baseline().cloned())
    } else {
        Expected::Missing
    };
    let prepared = Target::open(
        snapshot.target(),
        Options {
            max_bytes: persistence::MAX_BYTES,
            parent: ParentPolicy::Existing,
            expected,
            models: inventory,
        },
    )?
    .stage_rope(snapshot.text())?;
    #[cfg(test)]
    if let Some(gate) = gate {
        gate.pause(GatePoint::BeforePrepared)?;
        if gate.points.contains(&GatePoint::Panic) {
            panic!("fixture native save panic after staging");
        }
    }
    let info = prepared.info();
    let expires_at = Instant::now() + authorization;
    if sender
        .send(WorkerEvent::Prepared(PreparedInfo {
            id,
            requested_path: info.requested_path,
            canonical_path: info.canonical_path,
            matching_models: info.matching_models,
            baseline_sha256: info.baseline_sha256,
            expires_at,
        }))
        .is_err()
    {
        return Ok(Outcome::Rejected);
    }
    match decisions.recv_timeout(authorization) {
        Ok(Decision::Reject) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Ok(Outcome::Rejected);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => return Ok(Outcome::Expired),
        Ok(Decision::Authorize) => ensure!(
            Instant::now() < expires_at,
            "Native save authorization expired"
        ),
    }
    #[cfg(test)]
    if let Some(gate) = gate {
        gate.pause(GatePoint::BeforeCommit)?;
    }
    Ok(Outcome::Committed(prepared.commit()?))
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GatePoint {
    BeforePrepared,
    BeforeCommit,
    BeforeFinish,
    AfterFinishReply,
    Panic,
    Disconnect,
}
#[cfg(test)]
struct FixtureGate {
    points: Vec<GatePoint>,
    entered: SyncSender<GatePoint>,
    release: Receiver<()>,
}
#[cfg(test)]
impl FixtureGate {
    fn pause(&self, point: GatePoint) -> Result<()> {
        if self.points.contains(&point) {
            self.entered
                .send(point)
                .context("Save fixture gate observer disconnected")?;
            self.release
                .recv_timeout(Duration::from_secs(6))
                .context("Save fixture gate timed out")?;
        }
        Ok(())
    }
}
#[cfg(test)]
impl Worker {
    /// Gates only the next actual worker, at real staged/authorized/cleanup
    /// boundaries. No fake filesystem success or model publication is injected.
    pub(crate) fn fixture_gated(
        points: Vec<GatePoint>,
    ) -> (Self, Receiver<GatePoint>, SyncSender<()>) {
        let (entered, receiver) = mpsc::sync_channel(3);
        let (release, resume) = mpsc::sync_channel(3);
        (
            Self {
                last_id: 0,
                pending: None,
                gate: Some(FixtureGate {
                    points,
                    entered,
                    release: resume,
                }),
            },
            receiver,
            release,
        )
    }
}
