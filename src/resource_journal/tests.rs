use super::*;
use std::{fs, sync::atomic::AtomicBool, time::Duration};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    canceled: AtomicBool,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        Self {
            _directory: directory,
            root,
            canceled: AtomicBool::new(false),
        }
    }
    fn control(&self) -> Control<'_> {
        Control::new(&self.canceled, Instant::now() + Duration::from_secs(10))
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
    fn capture(&self, operations: Vec<Operation>) -> Capture {
        Capture::prepare(operations, Vec::new(), Limits::default(), &self.control()).unwrap()
    }
    fn stage(&self, capture: Capture) -> StagedJournal {
        StagedJournal::persist(capture, &self.root.join("journal"), &self.control()).unwrap()
    }
}
fn delete(path: PathBuf) -> Operation {
    Operation::Delete {
        path,
        options: FileOptions::default(),
    }
}
fn create(path: PathBuf, contents: Option<Vec<u8>>) -> Operation {
    Operation::Create {
        path,
        options: FileOptions::default(),
        contents,
    }
}
fn move_to(source: PathBuf, destination: PathBuf) -> Operation {
    Operation::Move {
        source,
        destination,
        options: FileOptions::default(),
    }
}

#[test]
fn binary_preimages_raw_create_contents_and_absence_survive_durable_recovery_without_execution() {
    let f = Fixture::new();
    let original = f.file("original.bin", &[0, 128, 255, 13, 10]);
    let mut operations = vec![
        delete(original.clone()),
        create(f.root.join("new.bin"), Some(vec![0, 255, 128])),
        create(f.root.join("absent.bin"), None),
        create(f.root.join("empty.bin"), Some(Vec::new())),
    ];
    if let Operation::Create { options, .. } = &mut operations[1] {
        options.overwrite = Presence::Value(false);
        options.ignore_if_exists = Presence::Value(true);
        options.max_size = Presence::Value(-0.5);
    }
    let expected = operations.clone();
    let capture = f.capture(operations);
    assert_eq!(
        capture.file_contents(&original),
        Some([0, 128, 255, 13, 10].as_slice())
    );
    let staged = f.stage(capture);
    let recovery = staged.recovery(&f.control()).unwrap();
    assert_eq!(recovery.capture().operations(), expected);
    assert_eq!(recovery.state(), ExecutionState::NotStarted);
    assert_eq!(
        recovery.capture().file_contents(&original),
        Some([0, 128, 255, 13, 10].as_slice())
    );
    assert_eq!(fs::read(original).unwrap(), [0, 128, 255, 13, 10]);
    for name in ["new.bin", "absent.bin", "empty.bin"] {
        assert!(!f.root.join(name).exists());
    }
}

#[test]
fn dependent_create_move_delete_chain_is_retained_in_order_with_original_missing_resources() {
    let f = Fixture::new();
    let a = f.root.join("future/a.bin");
    let b = f.root.join("future/b.bin");
    let operations = vec![
        create(a.clone(), Some(vec![65, 0, 255])),
        move_to(a.clone(), b.clone()),
        delete(b.clone()),
    ];
    let capture = f.capture(operations.clone());
    assert!(capture.resources().iter().all(|resource| !resource.present));
    assert!(
        capture
            .resources()
            .iter()
            .all(|resource| resource.missing_ancestors == vec![f.root.join("future")])
    );
    let journal = f.stage(capture);
    assert_eq!(
        journal
            .recovery(&f.control())
            .unwrap()
            .capture()
            .operations(),
        operations
    );
    assert!(
        !f.root.join("future").exists(),
        "Staging must not execute a net-state rewrite or mkdir operand parents"
    );
}

#[test]
fn directory_trees_capture_every_original_child_and_displaced_destination_bytes() {
    let f = Fixture::new();
    let source = f.root.join("source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/raw.bin"), [0, 255, 128]).unwrap();
    fs::write(source.join(".dotfile"), b"keep dot").unwrap();
    let target = f.root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("dirty-destination.bin"), b"displaced baseline").unwrap();
    let mut operation = move_to(source.clone(), target.clone());
    if let Operation::Move { options, .. } = &mut operation {
        options.overwrite = Presence::Value(true);
        options.copy = Presence::Value(true);
    }
    let staged = f.stage(f.capture(vec![operation]));
    let recovery = staged.recovery(&f.control()).unwrap();
    assert_eq!(recovery.capture().nodes().count(), 6);
    assert_eq!(
        recovery
            .capture()
            .file_contents(&source.join("nested/raw.bin")),
        Some([0, 255, 128].as_slice())
    );
    assert_eq!(
        recovery
            .capture()
            .file_contents(&target.join("dirty-destination.bin")),
        Some(b"displaced baseline".as_slice())
    );
    assert!(source.exists() && target.exists());
}

#[test]
fn dirty_native_model_archives_keep_proof_text_and_opaque_view_history_metadata_durable() {
    let f = Fixture::new();
    let file = f.file("target.txt", b"disk baseline\r\n");
    let archive = ModelArchive {
        proof: ModelProof {
            document: 91,
            revision: 8,
            text_epoch: 10,
            language_epoch: 3,
            resource_epoch: 0,
            saved_revision: 2,
            save_generation: 4,
        },
        resource: file.clone(),
        text: "dirty 猫🙂\r\n".as_bytes().to_vec(),
        metadata: vec![0, 255, 65],
    };
    let capture = Capture::prepare(
        vec![delete(file.clone())],
        vec![archive.clone()],
        Limits::default(),
        &f.control(),
    )
    .unwrap();
    let journal = f.stage(capture);
    assert_eq!(
        journal.recovery(&f.control()).unwrap().capture().archives(),
        &[archive]
    );
    assert_eq!(fs::read(file).unwrap(), b"disk baseline\r\n");
}

#[test]
fn all_option_presence_folder_recursive_trash_and_size_values_survive_without_silent_noops() {
    let f = Fixture::new();
    let source = f.file("source", b"before");
    let options = FileOptions {
        overwrite: Presence::Value(false),
        ignore_if_exists: Presence::Value(true),
        ignore_if_not_exists: Presence::Value(false),
        recursive: Presence::Value(true),
        copy: Presence::Value(false),
        folder: Presence::Value(true),
        skip_trash_bin: Presence::Value(false),
        max_size: Presence::Value(12.5),
    };
    let operations = vec![
        Operation::Delete {
            path: source,
            options: options.clone(),
        },
        Operation::Create {
            path: f.root.join("future-folder"),
            options,
            contents: Some(vec![0, 128, 255]),
        },
    ];
    let expected = operations.clone();
    let staged = f.stage(f.capture(operations));
    assert_eq!(
        staged
            .recovery(&f.control())
            .unwrap()
            .capture()
            .operations(),
        expected
    );
    assert!(!f.root.join("future-folder").exists());
}

#[test]
fn foreign_replacement_with_identical_bytes_refuses_original_and_recovery_authority() {
    let f = Fixture::new();
    let file = f.file("source", b"same bytes");
    let staged = f.stage(f.capture(vec![delete(file.clone())]));
    fs::rename(&file, f.root.join("original-held")).unwrap();
    fs::write(&file, b"same bytes").unwrap();
    assert!(staged.verify_originals(&f.control()).is_err());
    assert!(RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).is_err());
    assert_eq!(fs::read(file).unwrap(), b"same bytes");
}

#[test]
fn parent_aba_with_same_resource_bytes_refuses_recovery_without_foreign_overwrite() {
    let f = Fixture::new();
    let parent = f.root.join("parent");
    fs::create_dir(&parent).unwrap();
    let file = parent.join("source");
    fs::write(&file, b"same bytes").unwrap();
    let staged = f.stage(f.capture(vec![delete(file.clone())]));
    fs::rename(&parent, f.root.join("original-parent")).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(&file, b"same bytes").unwrap();
    assert!(staged.verify_originals(&f.control()).is_err());
    assert!(RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).is_err());
    assert_eq!(fs::read(file).unwrap(), b"same bytes");
}

#[test]
fn originally_missing_parent_or_target_appearing_invalidates_capture() {
    let f = Fixture::new();
    let target = f.root.join("future/new.bin");
    let capture = f.capture(vec![create(target.clone(), None)]);
    fs::create_dir(f.root.join("future")).unwrap();
    assert!(capture.verify_originals(&f.control()).is_err());
    let capture = f.capture(vec![create(target.clone(), None)]);
    fs::write(&target, b"foreign").unwrap();
    assert!(capture.verify_originals(&f.control()).is_err());
    assert_eq!(fs::read(target).unwrap(), b"foreign");
}

#[cfg(unix)]
#[test]
fn final_and_descendant_symlinks_are_refused_while_parent_alias_is_originally_proved() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let file = f.file("original", b"keep");
    let link = f.root.join("link");
    symlink(&file, &link).unwrap();
    assert!(
        Capture::prepare(
            vec![delete(link)],
            Vec::new(),
            Limits::default(),
            &f.control()
        )
        .is_err()
    );
    let tree = f.root.join("tree");
    fs::create_dir(&tree).unwrap();
    symlink(&file, tree.join("child")).unwrap();
    assert!(
        Capture::prepare(
            vec![delete(tree)],
            Vec::new(),
            Limits::default(),
            &f.control()
        )
        .is_err()
    );
    let parent = f.root.join("parent");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("file"), b"alias").unwrap();
    let alias = f.root.join("parent-alias");
    symlink(&parent, &alias).unwrap();
    let capture = f.capture(vec![delete(alias.join("file"))]);
    assert_eq!(capture.resources()[0].canonical, parent.join("file"));
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("file"), b"alias").unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&other, &alias).unwrap();
    assert!(capture.verify_originals(&f.control()).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_directory_alias_cannot_place_journal_inside_a_destructive_operand() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let tree = f.root.join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("child"), b"keep").unwrap();
    let alias = f.root.join("alias");
    symlink(&tree, &alias).unwrap();
    let capture = f.capture(vec![delete(tree.clone())]);
    assert!(StagedJournal::persist(capture, &alias.join("journal"), &f.control()).is_err());
    assert!(!tree.join("journal").exists());
}

#[test]
fn operation_node_byte_and_archive_budgets_fail_before_resource_mutation() {
    let f = Fixture::new();
    let file = f.file("large", &[0; 100]);
    let limits = Limits {
        max_bytes: 99,
        ..Limits::default()
    };
    assert!(
        Capture::prepare(vec![delete(file.clone())], Vec::new(), limits, &f.control()).is_err()
    );
    let tree = f.root.join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("a"), b"a").unwrap();
    fs::write(tree.join("b"), b"b").unwrap();
    let limits = Limits {
        max_nodes: 2,
        ..Limits::default()
    };
    assert!(Capture::prepare(vec![delete(tree)], Vec::new(), limits, &f.control()).is_err());
    let limits = Limits {
        max_operations: 1,
        ..Limits::default()
    };
    assert!(
        Capture::prepare(
            vec![delete(file.clone()), create(f.root.join("new"), None)],
            Vec::new(),
            limits,
            &f.control()
        )
        .is_err()
    );
    let archive = ModelArchive {
        proof: ModelProof {
            document: 1,
            revision: 1,
            text_epoch: 1,
            language_epoch: 1,
            resource_epoch: 0,
            saved_revision: 0,
            save_generation: 0,
        },
        resource: file.clone(),
        text: vec![0; 100],
        metadata: Vec::new(),
    };
    assert!(
        Capture::prepare(
            vec![delete(file.clone())],
            vec![archive],
            Limits {
                max_bytes: 99,
                ..Limits::default()
            },
            &f.control()
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), [0; 100]);
    assert!(!f.root.join("new").exists());
}

#[test]
fn cancellation_and_original_deadline_prevent_capture_and_durable_admission() {
    let f = Fixture::new();
    let file = f.file("file", b"keep");
    let capture = f.capture(vec![delete(file.clone())]);
    f.canceled.store(true, Ordering::Release);
    assert!(
        Capture::prepare(
            vec![delete(file.clone())],
            Vec::new(),
            Limits::default(),
            &f.control()
        )
        .is_err()
    );
    assert!(StagedJournal::persist(capture, &f.root.join("journal"), &f.control()).is_err());
    assert!(!f.root.join("journal").exists());
    f.canceled.store(false, Ordering::Release);
    let expired = Control::new(&f.canceled, Instant::now() - Duration::from_secs(1));
    assert!(
        Capture::prepare(
            vec![delete(file.clone())],
            Vec::new(),
            Limits::default(),
            &expired
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"keep");
}

#[test]
fn malformed_manifest_payload_corruption_and_unknown_execution_state_refuse_recovery() {
    for kind in [
        "unknown-field",
        "unknown-version",
        "execution",
        "blob-path",
        "payload",
    ] {
        let f = Fixture::new();
        let file = f.file("file", b"original");
        let staged = f.stage(f.capture(vec![delete(file.clone())]));
        let manifest = staged.path().join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        match kind {
            "unknown-field" => value["foreign"] = serde_json::json!(true),
            "unknown-version" => value["version"] = serde_json::json!(99),
            "execution" => value["state"] = serde_json::json!("Committed"),
            "blob-path" => value["nodes"][0]["contents"]["name"] = serde_json::json!("../file"),
            "payload" => {
                let name = value["nodes"][0]["contents"]["name"].as_str().unwrap();
                fs::write(staged.path().join(name), b"tampered").unwrap();
            }
            _ => unreachable!(),
        }
        if kind != "payload" {
            fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        assert!(
            RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).is_err(),
            "{kind}"
        );
        assert_eq!(fs::read(file).unwrap(), b"original");
    }
}

#[test]
fn deep_missing_parent_prefixes_and_aggregate_resource_metadata_are_bounded() {
    let f = Fixture::new();
    let mut deep = f.root.clone();
    for _ in 0..100 {
        deep.push("missing");
    }
    let error = Capture::prepare(
        vec![create(deep.join("file"), None)],
        Vec::new(),
        Limits {
            max_manifest_bytes: 4096,
            ..Limits::default()
        },
        &f.control(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("budget"));
    assert!(!f.root.join("missing").exists());
    let operations = (0..20)
        .map(|index| create(f.root.join(format!("file-{index}")), None))
        .collect();
    assert!(
        Capture::prepare(
            operations,
            Vec::new(),
            Limits {
                max_manifest_bytes: 4096,
                ..Limits::default()
            },
            &f.control()
        )
        .is_err()
    );
}

#[test]
fn unrelated_dirty_archive_is_refused_without_resource_effect() {
    let f = Fixture::new();
    let operand = f.file("operand", b"disk");
    let unrelated = f.file("unrelated", b"unrelated disk");
    let archive = ModelArchive {
        proof: ModelProof {
            document: 42,
            revision: 3,
            text_epoch: 4,
            language_epoch: 5,
            resource_epoch: 0,
            saved_revision: 2,
            save_generation: 6,
        },
        resource: unrelated.clone(),
        text: b"unsaved unrelated work".to_vec(),
        metadata: b"history".to_vec(),
    };
    assert!(
        Capture::prepare(
            vec![delete(operand.clone())],
            vec![archive],
            Limits::default(),
            &f.control()
        )
        .is_err()
    );
    assert_eq!(fs::read(operand).unwrap(), b"disk");
    assert_eq!(fs::read(unrelated).unwrap(), b"unrelated disk");
}

#[test]
fn journal_parent_replacement_refuses_live_and_restart_authority() {
    let f = Fixture::new();
    let file = f.file("operand", b"keep");
    let parent = f.root.join("journal-parent");
    fs::create_dir(&parent).unwrap();
    let staged = StagedJournal::persist(
        f.capture(vec![delete(file.clone())]),
        &parent.join("journal"),
        &f.control(),
    )
    .unwrap();
    let retired = f.root.join("retired-parent");
    fs::rename(&parent, &retired).unwrap();
    fs::create_dir(&parent).unwrap();
    // Moving the original journal itself into a new parent does not renew its
    // ORIGINAL parent authority even though its own inode is unchanged.
    fs::rename(retired.join("journal"), parent.join("journal")).unwrap();
    assert!(staged.verify_originals(&f.control()).is_err());
    assert!(
        RecoveryAuthority::load(&parent.join("journal"), Limits::default(), &f.control()).is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn journal_requested_parent_alias_retarget_refuses_recovery() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let file = f.file("operand", b"keep");
    let parent = f.root.join("parent");
    let other = f.root.join("other");
    fs::create_dir(&parent).unwrap();
    fs::create_dir(&other).unwrap();
    let alias = f.root.join("alias");
    symlink(&parent, &alias).unwrap();
    let staged = StagedJournal::persist(
        f.capture(vec![delete(file)]),
        &alias.join("journal"),
        &f.control(),
    )
    .unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&other, &alias).unwrap();
    assert!(staged.verify_originals(&f.control()).is_err());
    assert!(RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).is_err());
}

#[test]
fn durable_mapping_inventory_and_late_invalid_record_refuse_whole_recovery() {
    for kind in [
        "canonical",
        "ancestor",
        "missing",
        "extra-resource",
        "late-record",
    ] {
        let f = Fixture::new();
        let first = f.file("first", b"first original");
        let last = f.file("last", b"last original");
        let staged = f.stage(f.capture(vec![delete(first.clone()), delete(last.clone())]));
        let manifest = staged.path().join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        match kind {
            "canonical" => value["resources"][0]["canonical"] = serde_json::json!(last),
            "ancestor" => {
                value["resources"][0]["ancestor_requested"] =
                    serde_json::json!(f.root.join("missing"))
            }
            "missing" => {
                value["resources"][0]["missing_ancestors"] =
                    serde_json::json!([f.root.join("missing")])
            }
            "extra-resource" => {
                let extra = value["resources"][0].clone();
                value["resources"].as_array_mut().unwrap().push(extra);
            }
            "late-record" => value["nodes"][1]["contents"]["sha256"][0] = serde_json::json!(256),
            _ => unreachable!(),
        }
        fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).is_err(),
            "{kind}"
        );
        assert_eq!(fs::read(first).unwrap(), b"first original");
        assert_eq!(fs::read(last).unwrap(), b"last original");
    }
}

#[cfg(unix)]
#[test]
fn symlink_payload_and_non_utf8_tree_names_refuse_authority() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let f = Fixture::new();
    let file = f.file("operand", b"keep");
    let staged = f.stage(f.capture(vec![delete(file.clone())]));
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(staged.path().join("manifest.json")).unwrap()).unwrap();
    let payload = staged
        .path()
        .join(value["nodes"][0]["contents"]["name"].as_str().unwrap());
    fs::remove_file(&payload).unwrap();
    symlink(&file, &payload).unwrap();
    assert!(RecoveryAuthority::load(staged.path(), Limits::default(), &f.control()).is_err());
    let tree = f.root.join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(
        tree.join(std::ffi::OsString::from_vec(vec![255])),
        b"binary name",
    )
    .unwrap();
    assert!(
        Capture::prepare(
            vec![delete(tree)],
            Vec::new(),
            Limits::default(),
            &f.control()
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"keep");
}

#[test]
fn explicit_null_and_absent_options_survive_ordered_roundtrip_distinct_from_false() {
    let f = Fixture::new();
    let target = f.root.join("target");
    let mut operations = Vec::new();
    for overwrite in [
        Presence::Absent,
        Presence::Null,
        Presence::Value(false),
        Presence::Value(true),
    ] {
        operations.push(Operation::Create {
            path: target.clone(),
            contents: None,
            options: FileOptions {
                overwrite,
                ignore_if_exists: Presence::Value(true),
                ..FileOptions::default()
            },
        });
    }
    let expected = operations.clone();
    let staged = f.stage(f.capture(operations));
    assert_eq!(
        staged
            .recovery(&f.control())
            .unwrap()
            .capture()
            .operations(),
        expected
    );
    assert!(!Presence::<bool>::Absent.is_present());
    assert!(Presence::<bool>::Null.is_present());
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn special_tree_entry_is_explicitly_refused_without_opening_or_mutating_it() {
    use std::os::unix::net::UnixListener;
    let f = Fixture::new();
    let tree = f.root.join("tree");
    fs::create_dir(&tree).unwrap();
    let socket = tree.join("socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    assert!(
        Capture::prepare(
            vec![delete(tree)],
            Vec::new(),
            Limits::default(),
            &f.control()
        )
        .is_err()
    );
    assert!(socket.exists());
}
