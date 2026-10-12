//! Storage/worker coupling with actual native document snapshots. This is not
//! public WorkspaceEdit execution or a substitute for App caller/model authority.
use super::*;
use crate::{
    document::Document,
    resource_worker::{Event, Outcome, Preparation, Transaction, Worker},
};
use ropey::Rope;
use std::{fs, thread, time::Duration};

struct Source {
    proof: ModelProof,
    resource: PathBuf,
    text: Rope,
    eol: String,
}
impl Source {
    fn capture(document: &Document) -> Self {
        Self {
            proof: ModelProof {
                document: document.id,
                revision: document.revision,
                text_epoch: document.text_epoch(),
                language_epoch: document.language_epoch(),
                resource_epoch: document.resource_epoch(),
                saved_revision: document.saved_revision,
                save_generation: document.save_generation(),
            },
            resource: document.path.clone().unwrap(),
            text: document.text.clone(),
            eol: document.eol.clone(),
        }
    }
}
struct Stage {
    source: Source,
    destination: PathBuf,
    journal: PathBuf,
}
struct Info {
    journal: PathBuf,
    proof: ModelProof,
}
impl Transaction for Stage {
    type Prepared = StagedJournal;
    type Info = Info;
    type Receipt = StagedJournal;
    fn prepare(self, preparation: &Preparation) -> Result<(Self::Prepared, Self::Info)> {
        let control = Control::new(preparation.cancel_flag(), preparation.deadline());
        control.check()?;
        // Materialize the immutable native Rope and archive metadata on the
        // actual worker, never on the input/rendering thread.
        let archive = ModelArchive {
            proof: self.source.proof.clone(),
            resource: self.source.resource.clone(),
            text: self.source.text.to_string().into_bytes(),
            metadata: serde_json::to_vec(&serde_json::json!({"eol":self.source.eol}))?,
        };
        let capture = Capture::prepare(
            vec![Operation::Move {
                source: self.source.resource,
                destination: self.destination,
                options: FileOptions::default(),
            }],
            vec![archive],
            Limits::default(),
            &control,
        )?;
        let staged = StagedJournal::persist(capture, &self.journal, &control)?;
        let info = Info {
            journal: staged.path().into(),
            proof: self.source.proof,
        };
        Ok((staged, info))
    }
    fn commit(staged: Self::Prepared) -> Result<Self::Receipt> {
        // Accepted work gets a bounded verification phase, not a renewed source
        // capture. The real future executor must separately checkpoint effects.
        let canceled = AtomicBool::new(false);
        let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
        staged.verify_originals(&control)?;
        Ok(staged)
    }
}
fn next(worker: &mut Worker<Stage>) -> Event<Info, StagedJournal> {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(event) = worker.poll() {
            return event;
        }
        assert!(
            Instant::now() < until,
            "Actual archive worker did not settle"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn setup() -> (tempfile::TempDir, Document, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("original.cpp");
    fs::write(&path, "original 猫🙂\r\n").unwrap();
    let mut document = Document::open_existing(&path).unwrap();
    document.insert("dirty ", false);
    (directory, document, root)
}

#[test]
fn real_native_document_archive_is_staged_on_joined_worker_without_resource_execution() {
    let (_directory, document, root) = setup();
    let source = Source::capture(&document);
    let mut worker = Worker::default();
    worker
        .start(
            1,
            Stage {
                source,
                destination: root.join("renamed.cpp"),
                journal: root.join("journal"),
            },
            Instant::now() + Duration::from_secs(6),
        )
        .unwrap();
    let Event::Prepared { id: 1, info, .. } = next(&mut worker) else {
        panic!("original archive preparation")
    };
    assert_eq!(info.proof.document, document.id);
    assert_eq!(info.proof.text_epoch, document.text_epoch());
    assert!(worker.busy());
    assert_eq!(
        fs::read(root.join("original.cpp")).unwrap(),
        "original 猫🙂\r\n".as_bytes()
    );
    assert!(!root.join("renamed.cpp").exists());
    worker.authorize(1).unwrap();
    let Event::Finished {
        authorized: true,
        result: Ok(Outcome::Committed(staged)),
        ..
    } = next(&mut worker)
    else {
        panic!("joined archive receipt")
    };
    assert!(!worker.busy());
    assert_eq!(staged.state(), ExecutionState::NotStarted);
    let canceled = AtomicBool::new(false);
    let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
    let recovery = staged.recovery(&control).unwrap();
    let archive = &recovery.capture().archives()[0];
    assert_eq!(archive.proof.document, document.id);
    assert_eq!(archive.text, "dirty original 猫🙂\r\n".as_bytes());
    assert_eq!(archive.text, document.text.to_string().as_bytes());
    assert!(document.dirty());
    assert_eq!(recovery.path(), info.journal);
    assert!(!root.join("renamed.cpp").exists());
}

#[test]
fn canceled_archive_keeps_original_cow_text_after_native_edit_undo_without_fake_effects() {
    let (_directory, mut document, root) = setup();
    let source = Source::capture(&document);
    let original_epoch = source.proof.text_epoch;
    let mut worker = Worker::default();
    worker
        .start(
            2,
            Stage {
                source,
                destination: root.join("renamed.cpp"),
                journal: root.join("journal"),
            },
            Instant::now() + Duration::from_secs(6),
        )
        .unwrap();
    document.insert("independent ", false);
    document.undo();
    assert!(document.text_epoch() > original_epoch);
    let Event::Prepared { id: 2, info, .. } = next(&mut worker) else {
        panic!("original COW archive")
    };
    assert_eq!(info.proof.text_epoch, original_epoch);
    // The future App controller must compare the actual native model proof.
    // This test explicitly retires that original operation rather than renewing
    // its source merely because Undo restored matching text/revision.
    worker.retire(2);
    assert!(matches!(
        next(&mut worker),
        Event::Finished {
            authorized: false,
            result: Ok(Outcome::Rejected),
            ..
        }
    ));
    let canceled = AtomicBool::new(false);
    let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
    let recovery = RecoveryAuthority::load(&info.journal, Limits::default(), &control).unwrap();
    assert_eq!(recovery.state(), ExecutionState::NotStarted);
    assert_eq!(
        recovery.capture().archives()[0].proof.text_epoch,
        original_epoch
    );
    assert_eq!(
        recovery.capture().archives()[0].text,
        "dirty original 猫🙂\r\n".as_bytes()
    );
    assert_eq!(document.text.to_string(), "dirty original 猫🙂\r\n");
    assert!(document.dirty());
    assert!(!root.join("renamed.cpp").exists());
    assert_eq!(
        fs::read(root.join("original.cpp")).unwrap(),
        "original 猫🙂\r\n".as_bytes()
    );
}
