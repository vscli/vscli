//! Actual local filesystem effects. Parent coordinator executes these tests;
//! journal author does not launch Cargo, VS Code or terminal journeys.
use super::*;
use std::{fs, sync::atomic::AtomicBool, time::Duration};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    canceled: AtomicBool,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        Self {
            _temp: temp,
            root,
            canceled: AtomicBool::new(false),
        }
    }
    fn control(&self) -> Control<'_> {
        Control::new(&self.canceled, Instant::now() + Duration::from_secs(20))
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
    fn stage(&self, operations: Vec<Operation>, archives: Vec<ModelArchive>) -> StagedJournal {
        let capture =
            Capture::prepare(operations, archives, Limits::default(), &self.control()).unwrap();
        StagedJournal::persist(capture, &self.root.join("journal"), &self.control()).unwrap()
    }
    fn executor(&self, operations: Vec<Operation>) -> Executor {
        let policy = ExecutionPolicy {
            encoded_empty: operations
                .iter()
                .filter_map(|operation| {
                    if let Operation::Create {
                        path,
                        contents: None,
                        ..
                    } = operation
                    {
                        Some(EncodedEmpty {
                            requested: path.clone(),
                            bytes: Vec::new(),
                        })
                    } else {
                        None
                    }
                })
                .collect(),
            ..ExecutionPolicy::default()
        };
        Executor::authorize(
            self.stage(operations, Vec::new()),
            policy,
            ExecutionLimits::default(),
            &self.control(),
        )
        .unwrap()
    }
}
fn create(path: PathBuf, contents: Option<Vec<u8>>, options: FileOptions) -> Operation {
    Operation::Create {
        path,
        contents,
        options,
    }
}
fn rename(source: PathBuf, destination: PathBuf, options: FileOptions) -> Operation {
    Operation::Move {
        source,
        destination,
        options,
    }
}
fn delete(path: PathBuf, options: FileOptions) -> Operation {
    Operation::Delete { path, options }
}
fn overwrite() -> FileOptions {
    FileOptions {
        overwrite: Presence::Value(true),
        ..FileOptions::default()
    }
}

#[cfg(target_os = "linux")]
#[test]
fn dependent_ordered_create_move_move_delete_executes_and_rolls_back_reverse_physical_effects() {
    let f = Fixture::new();
    let a = f.root.join("a");
    let b = f.file("b", b"displaced original");
    let c = f.root.join("c");
    let untouched = f.file("unrelated", b"untouched");
    let mut executor = f.executor(vec![
        create(a.clone(), Some(vec![0, 255, 128]), FileOptions::default()),
        rename(a.clone(), b.clone(), overwrite()),
        rename(b.clone(), c.clone(), FileOptions::default()),
        delete(c.clone(), FileOptions::default()),
    ]);
    let receipt = executor.execute_authorized(&f.control());
    assert_eq!(
        receipt.state,
        ExecutionState::Completed,
        "{:?}",
        receipt.error
    );
    assert_eq!(receipt.operations.len(), 4);
    assert_eq!(receipt.steps.len(), 5);
    assert!(!a.exists() && !b.exists() && !c.exists());
    assert_eq!(
        fs::read(executor.path().join("parked-4")).unwrap(),
        [0, 255, 128]
    );
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert!(!a.exists() && !c.exists());
    assert_eq!(fs::read(&b).unwrap(), b"displaced original");
    assert_eq!(fs::read(untouched).unwrap(), b"untouched");
    let recovered =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    assert_eq!(recovered.state(), ExecutionState::RolledBack);
    recovered.verify_current(&f.control()).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn overwrite_move_parks_displaced_binary_and_restores_original_identities() {
    let f = Fixture::new();
    let source = f.file("source", &[0, 255, 1]);
    let target = f.file("target", &[128, 0, 7]);
    let source_id = filesystem::identity(&filesystem::open(&source, false).unwrap()).unwrap();
    let target_id = filesystem::identity(&filesystem::open(&target, false).unwrap()).unwrap();
    let mut executor = f.executor(vec![rename(source.clone(), target.clone(), overwrite())]);
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), [0, 255, 1]);
    assert_eq!(
        fs::read(executor.path().join("parked-0")).unwrap(),
        [128, 0, 7]
    );
    assert_eq!(
        filesystem::identity(&filesystem::open(&target, false).unwrap()).unwrap(),
        source_id
    );
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(&source).unwrap(), [0, 255, 1]);
    assert_eq!(fs::read(&target).unwrap(), [128, 0, 7]);
    assert_eq!(
        filesystem::identity(&filesystem::open(&source, false).unwrap()).unwrap(),
        source_id
    );
    assert_eq!(
        filesystem::identity(&filesystem::open(&target, false).unwrap()).unwrap(),
        target_id
    );
}

#[cfg(target_os = "linux")]
#[test]
fn overwrite_create_rollback_and_dirty_archive_preserve_baselines_and_original_proof() {
    use crate::document::Document;
    let f = Fixture::new();
    let target = f.file("target", "baseline 猫🙂\r\n".as_bytes());
    let mut document = Document::open_existing(&target).unwrap();
    document.insert("unsaved ", false);
    let source = document.capture_resource_model().unwrap();
    let archive = source
        .archive(&f.control(), MAX_BYTES, 8 * 1024 * 1024)
        .unwrap();
    let original = archive.clone();
    let staged = f.stage(
        vec![create(target.clone(), Some(vec![0, 255, 128]), overwrite())],
        vec![archive],
    );
    source.check_current(&document).unwrap();
    let mut executor = Executor::authorize(
        staged,
        ExecutionPolicy::default(),
        ExecutionLimits::default(),
        &f.control(),
    )
    .unwrap();
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    assert_eq!(fs::read(&target).unwrap(), [0, 255, 128]);
    assert_eq!(
        executor.originals().archives(),
        std::slice::from_ref(&original)
    );
    let recovered =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    assert_eq!(recovered.originals().archives(), [original]);
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(target).unwrap(), "baseline 猫🙂\r\n".as_bytes());
    source.check_current(&document).unwrap();
    assert!(document.dirty());
}

#[cfg(target_os = "linux")]
#[test]
fn recursive_directory_delete_restores_binary_dotfiles_and_displaced_subtree() {
    let f = Fixture::new();
    let tree = f.root.join("tree");
    fs::create_dir_all(tree.join("nested")).unwrap();
    fs::write(tree.join("nested/raw"), [0, 255, 128]).unwrap();
    fs::write(tree.join(".lock"), b"ordinary payload").unwrap();
    let mut executor = f.executor(vec![delete(
        tree.clone(),
        FileOptions {
            recursive: Presence::Value(true),
            ..FileOptions::default()
        },
    )]);
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    assert!(!tree.exists());
    assert_eq!(
        fs::read(executor.path().join("parked-0/nested/raw")).unwrap(),
        [0, 255, 128]
    );
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(tree.join("nested/raw")).unwrap(), [0, 255, 128]);
    assert_eq!(fs::read(tree.join(".lock")).unwrap(), b"ordinary payload");
}

#[cfg(target_os = "linux")]
#[test]
fn directory_copy_is_a_real_copy_preserves_source_and_restores_displaced_destination() {
    let f = Fixture::new();
    let source = f.root.join("source");
    let target = f.root.join("target");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/raw"), [0, 255, 128]).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("old"), b"old target").unwrap();
    let mut executor = f.executor(vec![rename(
        source.clone(),
        target.clone(),
        FileOptions {
            copy: Presence::Value(true),
            ..overwrite()
        },
    )]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(
        outcome.state,
        ExecutionState::Completed,
        "{:?}",
        outcome.error
    );
    assert_eq!(fs::read(source.join("nested/raw")).unwrap(), [0, 255, 128]);
    assert_eq!(fs::read(target.join("nested/raw")).unwrap(), [0, 255, 128]);
    assert!(!target.join("old").exists());
    assert_ne!(
        filesystem::identity(&filesystem::open(&source.join("nested/raw"), false).unwrap())
            .unwrap(),
        filesystem::identity(&filesystem::open(&target.join("nested/raw"), false).unwrap())
            .unwrap()
    );
    let recovery =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    recovery.verify_current(&f.control()).unwrap();
    let mut recovered = recovery.into_rollback_authorized(&f.control()).unwrap();
    assert_eq!(
        recovered.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(target.join("old")).unwrap(), b"old target");
    assert_eq!(fs::read(source.join("nested/raw")).unwrap(), [0, 255, 128]);
}

#[cfg(target_os = "linux")]
#[test]
fn single_binary_file_copy_cold_rollback_keeps_source_and_restores_overwritten_identity() {
    let f = Fixture::new();
    let source = f.file("source.bin", &[0, 255, 128]);
    let target = f.file("target.bin", &[128, 0, 255]);
    let source_id = filesystem::identity(&filesystem::open(&source, false).unwrap()).unwrap();
    let target_id = filesystem::identity(&filesystem::open(&target, false).unwrap()).unwrap();
    let mut executor = f.executor(vec![rename(
        source.clone(),
        target.clone(),
        FileOptions {
            copy: Presence::Value(true),
            ..overwrite()
        },
    )]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(
        outcome.state,
        ExecutionState::Completed,
        "{:?}",
        outcome.error
    );
    assert_eq!(fs::read(&source).unwrap(), [0, 255, 128]);
    assert_eq!(fs::read(&target).unwrap(), [0, 255, 128]);
    assert_eq!(
        filesystem::identity(&filesystem::open(&source, false).unwrap()).unwrap(),
        source_id
    );
    assert_ne!(
        filesystem::identity(&filesystem::open(&target, false).unwrap()).unwrap(),
        source_id
    );
    let recovery =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    let mut restored = recovery.into_rollback_authorized(&f.control()).unwrap();
    assert_eq!(
        restored.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(&target).unwrap(), [128, 0, 255]);
    assert_eq!(
        filesystem::identity(&filesystem::open(&target, false).unwrap()).unwrap(),
        target_id
    );
}

#[cfg(target_os = "linux")]
#[test]
fn create_missing_parents_and_folders_records_each_step_and_reverse_removes_them() {
    let f = Fixture::new();
    let target = f.root.join("a/b/c/file");
    let folder = f.root.join("a/b/new-dir");
    let mut executor = f.executor(vec![
        create(
            target.clone(),
            Some(b"new".to_vec()),
            FileOptions::default(),
        ),
        create(
            folder.clone(),
            Some(vec![255]),
            FileOptions {
                folder: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
    ]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(
        outcome.state,
        ExecutionState::Completed,
        "{:?}",
        outcome.error
    );
    assert_eq!(outcome.steps.len(), 5);
    assert!(folder.is_dir());
    assert_eq!(fs::read(target).unwrap(), b"new");
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert!(!f.root.join("a").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn explicit_null_or_false_overwrite_does_not_enable_ignore_if_exists() {
    for value in [Presence::Null, Presence::Value(false)] {
        let f = Fixture::new();
        let target = f.file("target", b"original");
        let mut executor = f.executor(vec![create(
            target.clone(),
            Some(b"new".to_vec()),
            FileOptions {
                overwrite: value,
                ignore_if_exists: Presence::Value(true),
                ..FileOptions::default()
            },
        )]);
        let outcome = executor.execute_authorized(&f.control());
        assert_eq!(outcome.state, ExecutionState::Stopped);
        assert!(outcome.steps.is_empty());
        assert_eq!(fs::read(target).unwrap(), b"original");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn absent_overwrite_ignore_existing_and_ignore_missing_delete_are_recorded_noops() {
    let f = Fixture::new();
    let target = f.file("target", b"keep");
    let absent = f.root.join("absent");
    let mut executor = f.executor(vec![
        create(
            target.clone(),
            None,
            FileOptions {
                ignore_if_exists: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
        delete(
            absent,
            FileOptions {
                ignore_if_not_exists: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
    ]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(outcome.state, ExecutionState::Completed);
    assert!(outcome.steps.is_empty());
    assert!(outcome.operations.iter().all(|operation| operation.skipped));
    assert_eq!(fs::read(target).unwrap(), b"keep");
}

#[cfg(target_os = "linux")]
#[test]
fn absent_and_empty_contents_remain_distinct_and_selected_bom_is_written_exactly() {
    let f = Fixture::new();
    let absent = f.root.join("absent");
    let empty = f.root.join("empty");
    let staged = f.stage(
        vec![
            create(absent.clone(), None, FileOptions::default()),
            create(empty.clone(), Some(Vec::new()), FileOptions::default()),
        ],
        Vec::new(),
    );
    let policy = ExecutionPolicy {
        encoded_empty: vec![EncodedEmpty {
            requested: absent.clone(),
            bytes: vec![239, 187, 191],
        }],
        ..ExecutionPolicy::default()
    };
    let mut executor =
        Executor::authorize(staged, policy, ExecutionLimits::default(), &f.control()).unwrap();
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    assert_eq!(fs::read(absent).unwrap(), [239, 187, 191]);
    assert!(fs::read(empty).unwrap().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn max_size_suppresses_public_delete_inverse_but_internal_binary_rollback_remains() {
    let f = Fixture::new();
    let target = f.file("target", &[0, 255, 128]);
    let mut executor = f.executor(vec![delete(
        target.clone(),
        FileOptions {
            max_size: Presence::Value(1.5),
            ..FileOptions::default()
        },
    )]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(outcome.state, ExecutionState::Completed);
    assert!(outcome.operations[0].delete_inverse_suppressed);
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(target).unwrap(), [0, 255, 128]);
}

#[cfg(target_os = "linux")]
#[test]
fn later_semantic_failure_exposes_applied_prefix_and_allows_explicit_reverse_rollback() {
    let f = Fixture::new();
    let created = f.root.join("created");
    let missing = f.root.join("missing");
    let mut executor = f.executor(vec![
        create(
            created.clone(),
            Some(b"applied prefix".to_vec()),
            FileOptions::default(),
        ),
        delete(missing, FileOptions::default()),
    ]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(outcome.state, ExecutionState::Stopped);
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.operations.len(), 1);
    assert_eq!(fs::read(&created).unwrap(), b"applied prefix");
    assert!(outcome.error.is_some());
    let recovery =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    assert_eq!(recovery.state(), ExecutionState::Stopped);
    let mut recovered = recovery.into_rollback_authorized(&f.control()).unwrap();
    assert_eq!(
        recovered.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert!(!created.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn foreign_postimage_bytes_and_same_bytes_foreign_identity_refuse_rollback() {
    for replace in [false, true] {
        let f = Fixture::new();
        let source = f.file("source", b"owned");
        let target = f.root.join("target");
        let mut executor = f.executor(vec![rename(
            source.clone(),
            target.clone(),
            FileOptions::default(),
        )]);
        assert_eq!(
            executor.execute_authorized(&f.control()).state,
            ExecutionState::Completed
        );
        if replace {
            let other = f.file("foreign", b"owned");
            fs::rename(other, &target).unwrap();
        } else {
            fs::write(&target, b"foreign bytes").unwrap();
        }
        let before = fs::read(&target).unwrap();
        let recovery =
            ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
        assert!(recovery.verify_current(&f.control()).is_err());
        assert!(recovery.into_rollback_authorized(&f.control()).is_err());
        assert!(executor.rollback_authorized(&f.control()).error.is_some());
        assert_eq!(fs::read(target).unwrap(), before);
        assert!(!source.exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn foreign_child_in_created_directory_refuses_rollback_and_preserves_all_work() {
    let f = Fixture::new();
    let folder = f.root.join("folder");
    let mut executor = f.executor(vec![create(
        folder.clone(),
        None,
        FileOptions {
            folder: Presence::Value(true),
            ..FileOptions::default()
        },
    )]);
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    fs::write(folder.join("foreign"), b"foreign").unwrap();
    assert!(executor.rollback_authorized(&f.control()).error.is_some());
    assert_eq!(fs::read(folder.join("foreign")).unwrap(), b"foreign");
}

#[cfg(target_os = "linux")]
#[test]
fn unmatched_durable_intent_is_ambiguous_even_when_effect_occurred_or_looks_absent() {
    for remove_effect in [false, true] {
        let f = Fixture::new();
        let target = f.root.join("target");
        let mut executor = f.executor(vec![create(
            target.clone(),
            Some(b"owned".to_vec()),
            FileOptions::default(),
        )]);
        assert_eq!(
            executor.execute_authorized(&f.control()).state,
            ExecutionState::Completed
        );
        // Retain the actual durable header + intent and discard the later
        // acknowledgement prefix, representing a crash before receipt durability.
        for index in 1..4 {
            fs::remove_file(executor.path().join(format!("record-{index}.json"))).unwrap();
        }
        if remove_effect {
            fs::remove_file(&target).unwrap();
        }
        let recovered =
            ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
        assert_eq!(recovered.state(), ExecutionState::Ambiguous);
        assert!(recovered.has_unacknowledged_intent());
        assert!(recovered.into_rollback_authorized(&f.control()).is_err());
        assert_eq!(target.exists(), !remove_effect);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn interrupted_rollback_intent_does_not_authorize_retry_after_effect() {
    let f = Fixture::new();
    let target = f.root.join("target");
    let mut executor = f.executor(vec![create(
        target.clone(),
        Some(b"owned".to_vec()),
        FileOptions::default(),
    )]);
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    fs::remove_file(executor.path().join("record-5.json")).unwrap();
    fs::remove_file(executor.path().join("record-6.json")).unwrap();
    let recovered =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    assert_eq!(recovered.state(), ExecutionState::Ambiguous);
    assert!(recovered.into_rollback_authorized(&f.control()).is_err());
    assert!(!target.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn corruption_gap_late_invalid_record_and_original_payload_corruption_refuse_whole_inspection() {
    for kind in ["gap", "late", "chain", "payload"] {
        let f = Fixture::new();
        let source = f.file("source", b"original");
        let target = f.root.join("target");
        let mut executor = f.executor(vec![rename(source, target.clone(), FileOptions::default())]);
        assert_eq!(
            executor.execute_authorized(&f.control()).state,
            ExecutionState::Completed
        );
        match kind {
            "gap" => fs::remove_file(executor.path().join("record-1.json")).unwrap(),
            "late" => fs::write(
                executor.path().join("record-4.json"),
                b"{\"version\":2,\"unknown\":true}",
            )
            .unwrap(),
            "chain" => {
                let path = executor.path().join("record-1.json");
                let mut json: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                json["previous_sha256"][0] = serde_json::json!(256);
                fs::write(path, serde_json::to_vec(&json).unwrap()).unwrap();
            }
            "payload" => fs::write(executor.path().join("payload-0.bin"), b"changed").unwrap(),
            _ => unreachable!(),
        }
        assert!(
            ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).is_err(),
            "{kind}"
        );
        assert_eq!(fs::read(target).unwrap(), b"original");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn unsupported_trash_and_missing_empty_encoding_refuse_before_any_resource_effect() {
    for trash in [false, true] {
        let f = Fixture::new();
        let old = f.file("old", b"keep");
        let new = f.root.join("new");
        let staged = f.stage(
            vec![
                create(new.clone(), None, FileOptions::default()),
                delete(old.clone(), FileOptions::default()),
            ],
            Vec::new(),
        );
        let policy = ExecutionPolicy {
            use_trash: trash,
            encoded_empty: if trash {
                vec![EncodedEmpty {
                    requested: new.clone(),
                    bytes: Vec::new(),
                }]
            } else {
                Vec::new()
            },
        };
        assert!(
            Executor::authorize(staged, policy, ExecutionLimits::default(), &f.control()).is_err()
        );
        assert!(!new.exists());
        assert_eq!(fs::read(old).unwrap(), b"keep");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn nonrecursive_nonempty_delete_and_overlapping_moves_stop_without_effect() {
    for overlapping in [false, true] {
        let f = Fixture::new();
        let tree = f.root.join("tree");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("child"), b"keep").unwrap();
        let operation = if overlapping {
            rename(
                tree.clone(),
                tree.join("descendant"),
                FileOptions::default(),
            )
        } else {
            delete(tree.clone(), FileOptions::default())
        };
        let mut executor = f.executor(vec![operation]);
        let outcome = executor.execute_authorized(&f.control());
        assert_eq!(outcome.state, ExecutionState::Stopped);
        assert!(outcome.steps.is_empty());
        assert_eq!(fs::read(tree.join("child")).unwrap(), b"keep");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn execution_step_private_byte_and_record_budgets_bound_actual_effects() {
    for kind in ["step", "private", "record"] {
        let f = Fixture::new();
        let source = f.file("source", b"original");
        let target = f.root.join("target");
        let operations = if kind == "step" {
            vec![
                rename(source.clone(), target.clone(), FileOptions::default()),
                delete(target.clone(), FileOptions::default()),
            ]
        } else {
            vec![delete(source.clone(), FileOptions::default())]
        };
        let staged = f.stage(operations, Vec::new());
        let limits = ExecutionLimits {
            max_steps: if kind == "step" { 1 } else { 16384 },
            max_private_bytes: if kind == "private" { 1 } else { MAX_BYTES },
            max_record_bytes: if kind == "record" {
                1024
            } else {
                16 * 1024 * 1024
            },
        };
        let admitted =
            Executor::authorize(staged, ExecutionPolicy::default(), limits, &f.control());
        if kind == "record" {
            assert!(admitted.is_err());
            assert_eq!(fs::read(source).unwrap(), b"original");
            continue;
        }
        let mut executor = admitted.unwrap();
        let outcome = executor.execute_authorized(&f.control());
        assert_eq!(
            outcome.state,
            ExecutionState::Stopped,
            "{kind} {:?}",
            outcome.error
        );
        if kind == "step" {
            assert!(!source.exists());
            assert_eq!(fs::read(target).unwrap(), b"original");
            assert_eq!(outcome.steps.len(), 1);
        } else {
            assert_eq!(fs::read(source).unwrap(), b"original");
            assert!(outcome.steps.is_empty());
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn original_parent_aba_after_completed_effect_refuses_cold_rollback_authority() {
    let f = Fixture::new();
    let parent = f.root.join("parent");
    fs::create_dir(&parent).unwrap();
    let source = parent.join("source");
    let target = parent.join("target");
    fs::write(&source, b"owned").unwrap();
    let mut executor = f.executor(vec![rename(source, target.clone(), FileOptions::default())]);
    assert_eq!(
        executor.execute_authorized(&f.control()).state,
        ExecutionState::Completed
    );
    fs::rename(&parent, f.root.join("old-parent")).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(&target, b"owned").unwrap();
    let recovery =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    assert!(recovery.into_rollback_authorized(&f.control()).is_err());
    assert_eq!(fs::read(target).unwrap(), b"owned");
}

#[test]
fn legacy_missing_resource_epoch_is_inspectable_but_not_new_execution_authority() {
    let f = Fixture::new();
    let file = f.file("file", b"disk");
    let archive = ModelArchive {
        proof: ModelProof {
            document: 1,
            revision: 2,
            text_epoch: 3,
            language_epoch: 4,
            resource_epoch: 0,
            saved_revision: 1,
            save_generation: 0,
        },
        resource: file.clone(),
        text: b"dirty".to_vec(),
        metadata: Vec::new(),
    };
    let staged = f.stage(
        vec![delete(file.clone(), FileOptions::default())],
        vec![archive],
    );
    let manifest = staged.path().join("manifest.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    json["archives"][0]["proof"]
        .as_object_mut()
        .unwrap()
        .remove("resource_epoch");
    fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();
    let recovered =
        RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).unwrap();
    assert!(recovered.capture().has_legacy_archive_proofs());
    assert_eq!(recovered.capture().archives()[0].proof.resource_epoch, 0);
    assert!(
        Executor::authorize(
            staged,
            ExecutionPolicy::default(),
            ExecutionLimits::default(),
            &f.control()
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"disk");
}

#[cfg(target_os = "linux")]
#[test]
fn contiguous_create_batch_runs_folders_first_with_original_indices_and_no_global_reordering() {
    let f = Fixture::new();
    let file = f.root.join("directory/file");
    let directory = f.root.join("directory");
    let renamed = f.root.join("renamed");
    let last_folder = f.root.join("last");
    let mut executor = f.executor(vec![
        create(file.clone(), Some(b"file".to_vec()), FileOptions::default()),
        create(
            directory.clone(),
            None,
            FileOptions {
                folder: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
        rename(file.clone(), renamed.clone(), FileOptions::default()),
        create(
            last_folder.clone(),
            None,
            FileOptions {
                folder: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
    ]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(
        outcome.state,
        ExecutionState::Completed,
        "{:?}",
        outcome.error
    );
    assert_eq!(
        outcome
            .operations
            .iter()
            .map(|receipt| receipt.operation)
            .collect::<Vec<_>>(),
        [1, 0, 2, 3]
    );
    assert_eq!(
        outcome
            .steps
            .iter()
            .map(|receipt| receipt.operation)
            .collect::<Vec<_>>(),
        [1, 0, 2, 3]
    );
    assert_eq!(fs::read(renamed).unwrap(), b"file");
    assert!(directory.is_dir() && last_folder.is_dir());
    let recovered =
        ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
    recovered.verify_current(&f.control()).unwrap();
    let mut recovered = recovered.into_rollback_authorized(&f.control()).unwrap();
    assert_eq!(
        recovered.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert!(!directory.exists() && !last_folder.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn create_ignore_if_exists_is_decided_before_folder_batch_effects() {
    let f = Fixture::new();
    let file = f.root.join("directory");
    let nested = f.root.join("directory/nested");
    let mut executor = f.executor(vec![
        create(
            file.clone(),
            Some(b"file".to_vec()),
            FileOptions {
                ignore_if_exists: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
        create(
            nested.clone(),
            None,
            FileOptions {
                folder: Presence::Value(true),
                ..FileOptions::default()
            },
        ),
    ]);
    let outcome = executor.execute_authorized(&f.control());
    assert_eq!(outcome.state, ExecutionState::Stopped);
    assert_eq!(
        outcome
            .operations
            .iter()
            .map(|receipt| receipt.operation)
            .collect::<Vec<_>>(),
        [1]
    );
    assert!(nested.is_dir());
    assert_eq!(
        executor.rollback_authorized(&f.control()).state,
        ExecutionState::RolledBack
    );
    assert!(!file.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn rename_and_copy_ignore_decisions_do_not_change_after_earlier_batch_effects() {
    for copy in [false, true] {
        let f = Fixture::new();
        let a = f.file("a", b"first");
        let b = f.file("b", b"second");
        let c = f.root.join("c");
        let mut executor = f.executor(vec![
            rename(
                a.clone(),
                c.clone(),
                FileOptions {
                    copy: Presence::Value(copy),
                    ..FileOptions::default()
                },
            ),
            rename(
                b.clone(),
                c.clone(),
                FileOptions {
                    copy: Presence::Value(copy),
                    ignore_if_exists: Presence::Value(true),
                    ..FileOptions::default()
                },
            ),
        ]);
        let outcome = executor.execute_authorized(&f.control());
        assert_eq!(
            outcome.state,
            ExecutionState::Stopped,
            "{:?}",
            outcome.error
        );
        assert_eq!(outcome.operations.len(), 1);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(fs::read(&c).unwrap(), b"first");
        assert_eq!(fs::read(&b).unwrap(), b"second");
        assert_eq!(a.exists(), copy);
        let recovery =
            ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
        let mut recovered = recovery.into_rollback_authorized(&f.control()).unwrap();
        assert_eq!(
            recovered.rollback_authorized(&f.control()).state,
            ExecutionState::RolledBack
        );
        assert_eq!(fs::read(a).unwrap(), b"first");
        assert!(!c.exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn whole_move_copy_batch_preflight_refuses_missing_later_source_before_any_operand_effect() {
    for copy in [false, true] {
        let f = Fixture::new();
        let a = f.file("a", b"original");
        let b = f.root.join("b");
        let c = f.root.join("c");
        let mut executor = f.executor(vec![
            rename(
                a.clone(),
                b.clone(),
                FileOptions {
                    copy: Presence::Value(copy),
                    ..FileOptions::default()
                },
            ),
            rename(
                b.clone(),
                c.clone(),
                FileOptions {
                    copy: Presence::Value(copy),
                    ..FileOptions::default()
                },
            ),
        ]);
        let outcome = executor.execute_authorized(&f.control());
        assert_eq!(outcome.state, ExecutionState::Stopped);
        assert!(outcome.steps.is_empty() && outcome.operations.is_empty());
        assert_eq!(fs::read(a).unwrap(), b"original");
        assert!(!b.exists() && !c.exists());
        let recovered =
            ExecutionRecovery::load(executor.path(), Limits::default(), &f.control()).unwrap();
        recovered.verify_current(&f.control()).unwrap();
    }
}
