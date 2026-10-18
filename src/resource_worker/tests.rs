use super::*;
use crate::persistence::{self, Expected, Options, ParentPolicy, Target};
use std::{fs, path::PathBuf, time::Duration};

struct WriteFile {
    path: PathBuf,
    bytes: Vec<u8>,
    commit_gate: Option<TestGate>,
}
impl Transaction for WriteFile {
    type Prepared = (persistence::Prepared, Option<TestGate>);
    type Info = persistence::PreparedInfo;
    type Receipt = persistence::Commit;
    fn prepare(self, control: &Preparation) -> Result<(Self::Prepared, Self::Info)> {
        control.check()?;
        let target = Target::open(
            &self.path,
            Options {
                max_bytes: 1024,
                parent: ParentPolicy::Existing,
                expected: Expected::Captured,
                models: vec![],
            },
        )?;
        let info = target.info();
        let staged = target.stage_bytes(&self.bytes)?;
        control.check()?;
        Ok(((staged, self.commit_gate), info))
    }
    fn commit((staged, gate): Self::Prepared) -> Result<Self::Receipt> {
        if let Some(gate) = gate {
            gate.started.send(())?;
            gate.release.recv()?;
        }
        staged.commit()
    }
}
fn gate() -> (TestGate, Receiver<()>, SyncSender<()>) {
    let (started, began) = mpsc::sync_channel(1);
    let (release, receiver) = mpsc::sync_channel(1);
    (
        TestGate {
            started,
            release: receiver,
        },
        began,
        release,
    )
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(6)
}
fn next<T: Transaction>(worker: &mut Worker<T>) -> Event<T::Info, T::Receipt> {
    let expires = deadline();
    loop {
        if let Some(event) = worker.poll() {
            return event;
        }
        assert!(
            Instant::now() < expires,
            "Actual resource worker did not settle"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn write(path: &std::path::Path) -> WriteFile {
    WriteFile {
        path: path.into(),
        bytes: b"new binary\0\xff\r\n".to_vec(),
        commit_gate: None,
    }
}
fn original() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.cpp");
    fs::write(&path, "original 猫🙂\r\n").unwrap();
    (directory, path)
}

#[test]
fn prepared_original_path_receipt_does_not_write_until_exact_explicit_authorization() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    worker.start(1, write(&path), deadline()).unwrap();
    let Event::Prepared {
        id,
        info,
        expires_at,
    } = next(&mut worker)
    else {
        panic!("Actual prepared receipt missing");
    };
    assert_eq!(id, 1);
    assert!(expires_at > Instant::now());
    assert_eq!(info.canonical_path, fs::canonicalize(&path).unwrap());
    assert_eq!(fs::read(&path).unwrap(), "original 猫🙂\r\n".as_bytes());
    assert!(worker.authorize(2).is_err());
    assert!(worker.awaiting_authorization(1));
    worker.authorize(1).unwrap();
    assert!(worker.authorize(1).is_err());
    let Event::Finished {
        id: 1,
        retired: false,
        authorized: true,
        result: Ok(Outcome::Committed(receipt)),
    } = next(&mut worker)
    else {
        panic!("Actual commit receipt missing");
    };
    assert_eq!(receipt.bytes, write(&path).bytes.len());
    assert_eq!(fs::read(&path).unwrap(), write(&path).bytes);
    assert!(!worker.busy());
    assert!(worker.start(1, write(&path), deadline()).is_err());
}

#[test]
fn rejected_prepared_work_preserves_disk_and_retains_slot_until_actual_exit_and_join() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    let (after_reply, began, release) = gate();
    worker.after_reply = Some(after_reply);
    worker.start(4, write(&path), deadline()).unwrap();
    assert!(matches!(next(&mut worker), Event::Prepared { id: 4, .. }));
    assert!(!worker.retire(3));
    assert!(worker.retire(4));
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(worker.poll().is_none());
    assert!(worker.busy());
    assert!(worker.start(5, write(&path), deadline()).is_err());
    assert!(worker.authorize(4).is_err());
    assert_eq!(fs::read(&path).unwrap(), "original 猫🙂\r\n".as_bytes());
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut worker),
        Event::Finished {
            id: 4,
            retired: true,
            authorized: false,
            result: Ok(Outcome::Rejected),
        }
    ));
    assert!(!worker.busy());
}

#[test]
fn authorized_owner_retirement_preserves_committed_bytes_and_actual_joined_receipt() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    let (after_reply, began, release) = gate();
    worker.after_reply = Some(after_reply);
    worker.start(6, write(&path), deadline()).unwrap();
    assert!(matches!(next(&mut worker), Event::Prepared { id: 6, .. }));
    worker.authorize(6).unwrap();
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), write(&path).bytes);
    assert!(worker.retire(6));
    assert!(worker.poll().is_none());
    assert!(worker.busy());
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut worker),
        Event::Finished {
            id: 6,
            retired: true,
            authorized: true,
            result: Ok(Outcome::Committed(_)),
        }
    ));
    assert_eq!(fs::read(&path).unwrap(), write(&path).bytes);
}

#[test]
fn late_authorization_cannot_renew_the_original_deadline_or_release_a_held_worker() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    let (after_reply, began, release) = gate();
    worker.after_reply = Some(after_reply);
    let original_deadline = Instant::now() + Duration::from_millis(250);
    worker.start(7, write(&path), original_deadline).unwrap();
    let Event::Prepared { expires_at, .. } = next(&mut worker) else {
        panic!("Expected preparation before original deadline");
    };
    assert_eq!(expires_at, original_deadline);
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(worker.authorize(7).is_err());
    assert!(worker.poll().is_none());
    assert!(worker.busy());
    assert_eq!(fs::read(&path).unwrap(), "original 猫🙂\r\n".as_bytes());
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut worker),
        Event::Finished {
            id: 7,
            authorized: false,
            result: Ok(Outcome::Expired),
            ..
        }
    ));
}

#[test]
fn actual_commit_rechecks_original_destination_after_ui_authorization() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    worker.start(8, write(&path), deadline()).unwrap();
    assert!(matches!(next(&mut worker), Event::Prepared { id: 8, .. }));
    fs::write(&path, "external bytes\r\n").unwrap();
    worker.authorize(8).unwrap();
    let Event::Finished {
        id: 8,
        authorized: true,
        result: Err(error),
        ..
    } = next(&mut worker)
    else {
        panic!("Expected actual destination conflict");
    };
    assert!(error.contains("changed externally"));
    assert_eq!(fs::read(&path).unwrap(), b"external bytes\r\n");
}

#[test]
fn shutdown_joins_authorized_physical_work_before_returning_the_owned_receipt() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    let (commit_gate, began, release) = gate();
    let mut request = write(&path);
    request.commit_gate = Some(commit_gate);
    worker.start(9, request, deadline()).unwrap();
    assert!(matches!(next(&mut worker), Event::Prepared { id: 9, .. }));
    worker.authorize(9).unwrap();
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done, settled) = mpsc::sync_channel(1);
    let shutdown = thread::spawn(move || {
        done.send(worker.settle()).unwrap();
    });
    assert!(settled.recv_timeout(Duration::from_millis(20)).is_err());
    assert_eq!(fs::read(&path).unwrap(), "original 猫🙂\r\n".as_bytes());
    release.send(()).unwrap();
    assert!(matches!(
        settled.recv_timeout(Duration::from_secs(5)).unwrap(),
        Some(Event::Finished {
            id: 9,
            retired: true,
            authorized: true,
            result: Ok(Outcome::Committed(_)),
        })
    ));
    shutdown.join().unwrap();
    assert_eq!(fs::read(&path).unwrap(), write(&path).bytes);
}

struct FailedAfterEffect(PathBuf);
impl Transaction for FailedAfterEffect {
    type Prepared = PathBuf;
    type Info = ();
    type Receipt = ();
    fn prepare(self, control: &Preparation) -> Result<(Self::Prepared, Self::Info)> {
        control.check()?;
        Ok((self.0, ()))
    }
    fn commit(path: Self::Prepared) -> Result<Self::Receipt> {
        fs::write(path, "first completed effect")?;
        anyhow::bail!("second operation failed; first effect remains");
    }
}
#[test]
fn execution_failure_reports_authorized_phase_without_manufacturing_a_rollback() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    worker
        .start(10, FailedAfterEffect(path.clone()), deadline())
        .unwrap();
    assert!(matches!(next(&mut worker), Event::Prepared { id: 10, .. }));
    worker.authorize(10).unwrap();
    assert!(matches!(
        next(&mut worker),
        Event::Finished {
            authorized: true,
            result: Err(_),
            ..
        }
    ));
    assert_eq!(fs::read_to_string(path).unwrap(), "first completed effect");
}

struct HeldPreparation {
    gate: TestGate,
    path: PathBuf,
}
impl Transaction for HeldPreparation {
    type Prepared = PathBuf;
    type Info = ();
    type Receipt = ();
    fn prepare(self, control: &Preparation) -> Result<(Self::Prepared, Self::Info)> {
        self.gate.started.send(())?;
        self.gate.release.recv()?;
        control.check()?;
        Ok((self.path, ()))
    }
    fn commit(path: Self::Prepared) -> Result<Self::Receipt> {
        fs::write(path, "must not commit")?;
        Ok(())
    }
}
#[test]
fn cancellation_during_held_preparation_retains_actual_capacity_and_rejects_late_work() {
    let (_directory, path) = original();
    let mut worker = Worker::default();
    let (gate, began, release) = gate();
    worker
        .start(
            11,
            HeldPreparation {
                gate,
                path: path.clone(),
            },
            deadline(),
        )
        .unwrap();
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(worker.retire(11));
    assert!(worker.busy());
    assert!(worker.poll().is_none());
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut worker),
        Event::Finished {
            id: 11,
            retired: true,
            authorized: false,
            result: Err(_),
        }
    ));
    assert_eq!(fs::read(&path).unwrap(), "original 猫🙂\r\n".as_bytes());
}
