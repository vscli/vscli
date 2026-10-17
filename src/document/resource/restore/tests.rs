use super::*;
use crate::resource_journal::{
    Capture, ExecutionLimits, ExecutionPolicy, ExecutionState, Executor, FileOptions, Limits,
    Operation, Presence, StagedJournal,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    canceled: AtomicBool,
    deadline: Instant,
}
impl Fixture {
    fn new() -> Self {
        Self {
            canceled: AtomicBool::new(false),
            deadline: Instant::now() + Duration::from_secs(10),
        }
    }
    fn control(&self) -> Control<'_> {
        Control::new(&self.canceled, self.deadline)
    }
}
fn model(name: &str, text: &str) -> Document {
    let mut document = Document::from_text(text);
    document.path = Some(std::env::temp_dir().join(name));
    document.disk_content = Some(document.text.clone());
    document
}
fn dirty_source(text: &str) -> Document {
    let mut document = model("source.cpp", "source baseline\r\n");
    document.select_all();
    document.insert(text, false);
    document.eol = infer_eol(&document.text, DEFAULT_EOL).into();
    document
}
fn transfer(source: &Document, target: &mut Document, disk: &str, fixture: &Fixture) {
    let prepared = source
        .capture_resource_model()
        .unwrap()
        .prepare_transfer(
            target.capture_resource_model().unwrap(),
            Rope::from_str(disk),
            &fixture.control(),
        )
        .unwrap();
    target
        .prepare_resource_model_commit(prepared, &fixture.control(), 8 * 1024 * 1024)
        .unwrap()
        .publish();
}
#[test]
fn surviving_target_keeps_its_identity_views_and_history_instead_of_source_versions() {
    let fixture = Fixture::new();
    let source = dirty_source("stable\r\nα😀 restored tail\r\n");
    let mut target = model("target.cpp", "stable\r\nα😀 old tail\n");
    target.move_to(16, false);
    target.insert("prior ", false);
    let old_text = target.text.clone();
    let old_eol = target.eol.clone();
    let id = target.id;
    let path = target.path.clone();
    target.activate_view(10);
    target.move_to(target.len(), false);
    target.activate_view(20);
    target.move_to(target.len(), false);
    let prepared = source
        .capture_resource_model()
        .unwrap()
        .prepare_transfer(
            target.capture_resource_model().unwrap(),
            Rope::from_str("source baseline\r\n"),
            &fixture.control(),
        )
        .unwrap();
    target.move_to(2, false); // latest view navigation remains independently owned
    target
        .prepare_resource_model_commit(prepared, &fixture.control(), 8 * 1024 * 1024)
        .unwrap()
        .publish();
    assert_eq!(target.id, id);
    assert_ne!(target.id, source.id);
    assert_eq!(target.path, path);
    assert_eq!(target.text, source.text);
    assert_eq!(target.eol, "\r\n");
    assert!(target.dirty());
    assert_eq!(target.view_state(Some(20)).cursor, 2);
    assert_eq!(target.view_state(Some(10)).cursor, target.len());
    target.undo();
    assert_eq!(target.text, old_text);
    assert_eq!(target.eol, old_eol);
    target.redo();
    assert_eq!(target.text, source.text);
    assert_eq!(target.eol, "\r\n");
    target.undo();
    target.undo();
    assert_eq!(target.text.to_string(), "stable\r\nα😀 old tail\n");
    assert_eq!(source.text.to_string(), "stable\r\nα😀 restored tail\r\n");
}
#[test]
fn equal_dirty_text_marks_target_dirty_without_advancing_text_version_and_save_clears_it() {
    let fixture = Fixture::new();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("target.cpp");
    fs::write(&path, "same").unwrap();
    let mut target = Document::open_existing(&path).unwrap();
    let source = dirty_source("same");
    let revision = target.revision;
    let epoch = target.text_epoch();
    let history = target.undo.len();
    let pending_save = target.capture_save(path.clone()).unwrap();
    fs::write(&path, "source baseline\r\n").unwrap(); // actual resource postimage
    transfer(&source, &mut target, "source baseline\r\n", &fixture);
    assert_eq!(target.revision, revision);
    assert_eq!(target.text_epoch(), epoch);
    assert_eq!(target.undo.len(), history);
    assert!(target.dirty());
    assert!(target.check_save_snapshot(&pending_save).is_err());
    target.save().unwrap();
    assert!(!target.dirty());
    assert_eq!(fs::read(path).unwrap(), b"same");
}
#[test]
fn clean_source_reloads_actual_postimage_instead_of_stale_source_model_text() {
    let fixture = Fixture::new();
    let source = model("source.cpp", "old clean model\r\n");
    let mut target = model("target.cpp", "old target\n");
    let id = target.id;
    transfer(&source, &mut target, "actual disk 猫🙂\r\n", &fixture);
    assert_eq!(target.id, id);
    assert_eq!(target.text.to_string(), "actual disk 猫🙂\r\n");
    assert_eq!(target.disk_content.as_ref().unwrap(), &target.text);
    assert!(!target.dirty());
    target.undo();
    assert_eq!(target.text.to_string(), "old target\n");
    assert_eq!(target.eol, "\n");
    assert!(target.dirty());
    target.redo();
    assert!(!target.dirty());
    assert_eq!(target.eol, "\r\n");
}
#[test]
fn missing_target_gets_fresh_initial_dirty_identity_and_no_borrowed_source_history() {
    let fixture = Fixture::new();
    let source = dirty_source("restored 猫🙂\r\n");
    let snapshot = source.capture_resource_model().unwrap();
    let mut target = snapshot
        .restore_new(
            std::env::temp_dir().join("new.cpp"),
            Rope::from_str("source baseline\r\n"),
            &fixture.control(),
        )
        .unwrap();
    assert_ne!(target.id, source.id);
    assert_eq!(target.revision, 0);
    assert_eq!(target.text, source.text);
    assert!(target.undo.is_empty());
    assert!(target.dirty());
    target.undo();
    assert!(target.dirty());
    target.insert("independent ", false);
    target.undo();
    assert_eq!(target.text, source.text);
    assert!(target.dirty());
    assert!(
        snapshot
            .restore_new("relative.cpp".into(), Rope::new(), &fixture.control())
            .is_err()
    );
}
#[test]
fn explicit_and_inferred_languages_follow_original_target_restoration_rules() {
    let fixture = Fixture::new();
    let mut source = dirty_source("dirty");
    let snapshot = source.capture_resource_model().unwrap();
    let inherited = snapshot
        .restore_new(
            std::env::temp_dir().join("unknown.extension"),
            Rope::new(),
            &fixture.control(),
        )
        .unwrap();
    assert_eq!(inherited.language_override(), Some("cpp"));
    assert_eq!(inherited.explicit_language_override(), None);
    let plain = snapshot
        .restore_new(
            std::env::temp_dir().join("plain.txt"),
            Rope::new(),
            &fixture.control(),
        )
        .unwrap();
    assert_eq!(plain.language_override(), None);
    let python = snapshot
        .restore_new(
            std::env::temp_dir().join("target.py"),
            Rope::new(),
            &fixture.control(),
        )
        .unwrap();
    assert_eq!(python.language_override(), None);
    assert_eq!(
        crate::lsp::language(python.path.as_ref().unwrap()),
        "python"
    );
    source.set_language_override(Some("cpp".into())).unwrap();
    let explicit = source
        .capture_resource_model()
        .unwrap()
        .restore_new(
            std::env::temp_dir().join("explicit.txt"),
            Rope::new(),
            &fixture.control(),
        )
        .unwrap();
    assert_eq!(explicit.explicit_language_override(), Some("cpp"));
    source
        .set_language_override(Some("plaintext".into()))
        .unwrap();
    let mut target = model("target.rs", "old");
    target.set_language_override(Some("rust".into())).unwrap();
    transfer(&source, &mut target, "baseline", &fixture);
    assert_eq!(target.explicit_language_override(), Some("rust"));
}
#[test]
fn inferred_language_provenance_survives_archive_and_explicit_selection_retires_it() {
    let fixture = Fixture::new();
    let source = model("source.cpp", "code");
    let mut target = source
        .capture_resource_model()
        .unwrap()
        .restore_new(
            std::env::temp_dir().join("unknown.extension"),
            Rope::from_str("code"),
            &fixture.control(),
        )
        .unwrap();
    let original = target.capture_resource_model().unwrap();
    let archive = original
        .archive(&fixture.control(), 1024 * 1024, 1024 * 1024)
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&archive.metadata).unwrap();
    assert!(metadata["languageOverride"].is_null());
    assert_eq!(metadata["languageHint"], "cpp");
    target.set_language_override(Some("cpp".into())).unwrap();
    assert!(original.check_current(&target).is_err());
    target.set_language_override(None).unwrap();
    assert_eq!(target.language_override(), None);
}
#[test]
fn eol_only_restore_is_versioned_and_undo_redo_restore_metadata_and_original_bytes() {
    let fixture = Fixture::new();
    let mut source = dirty_source("same");
    source.eol = "\r\n".into();
    let mut target = model("target.cpp", "same");
    target.eol = "\n".into();
    let revision = target.revision;
    let epoch = target.text_epoch();
    transfer(&source, &mut target, "baseline", &fixture);
    assert!(target.revision > revision);
    assert!(target.text_epoch() > epoch);
    assert_eq!(target.eol, "\r\n");
    let epoch = target.text_epoch();
    target.undo();
    assert_eq!(target.eol, "\n");
    assert_eq!(target.text.to_string(), "same");
    assert!(target.text_epoch() > epoch);
    target.redo();
    assert_eq!(target.eol, "\r\n");
    assert!(target.dirty());
}
#[test]
fn canceled_expired_stale_and_retimed_restore_never_publish_model_or_baseline() {
    let fixture = Fixture::new();
    let source = dirty_source("new");
    let mut target = model("target.cpp", "old");
    let captured = target.capture_resource_model().unwrap();
    let prepared = source
        .capture_resource_model()
        .unwrap()
        .prepare_transfer(
            target.capture_resource_model().unwrap(),
            Rope::from_str("disk"),
            &fixture.control(),
        )
        .unwrap();
    let retimed = Control::new(&fixture.canceled, fixture.deadline + Duration::from_secs(1));
    assert!(
        target
            .prepare_resource_model_commit(prepared, &retimed, 8 * 1024 * 1024)
            .is_err()
    );
    captured.check_current(&target).unwrap();
    let prepared = source
        .capture_resource_model()
        .unwrap()
        .prepare_transfer(
            target.capture_resource_model().unwrap(),
            Rope::from_str("disk"),
            &fixture.control(),
        )
        .unwrap();
    target.insert("independent", false);
    target.undo();
    assert!(
        target
            .prepare_resource_model_commit(prepared, &fixture.control(), 8 * 1024 * 1024)
            .is_err()
    );
    fixture.canceled.store(true, Ordering::Release);
    assert!(
        target
            .capture_resource_model()
            .unwrap()
            .prepare_reload(Rope::new(), &fixture.control())
            .is_err()
    );
    let live = AtomicBool::new(false);
    let expired = Control::new(&live, Instant::now());
    assert!(
        target
            .capture_resource_model()
            .unwrap()
            .prepare_reload(Rope::new(), &expired)
            .is_err()
    );
    assert_eq!(target.text.to_string(), "old");
    assert_eq!(target.disk_content.as_ref().unwrap().to_string(), "old");
}
#[test]
fn allocation_refusal_and_dropped_commit_preserve_original_document_and_history() {
    let fixture = Fixture::new();
    let source = dirty_source("new");
    let mut target = model("target.cpp", "old");
    let original = target.capture_resource_model().unwrap();
    let prepared = source
        .capture_resource_model()
        .unwrap()
        .prepare_transfer(
            target.capture_resource_model().unwrap(),
            Rope::new(),
            &fixture.control(),
        )
        .unwrap();
    assert!(
        target
            .prepare_resource_model_commit(prepared, &fixture.control(), 512)
            .is_err()
    );
    original.check_current(&target).unwrap();
    let prepared = source
        .capture_resource_model()
        .unwrap()
        .prepare_transfer(
            target.capture_resource_model().unwrap(),
            Rope::new(),
            &fixture.control(),
        )
        .unwrap();
    drop(
        target
            .prepare_resource_model_commit(prepared, &fixture.control(), 8 * 1024 * 1024)
            .unwrap(),
    );
    original.check_current(&target).unwrap();
    assert!(
        target
            .capture_resource_model()
            .unwrap()
            .prepare_reload(Rope::from_str("binary\0"), &fixture.control())
            .is_err()
    );
    let large = Rope::from_str(&"x".repeat(33 * 1024 * 1024));
    assert!(
        target
            .capture_resource_model()
            .unwrap()
            .prepare_reload(large, &fixture.control())
            .is_err()
    );
}

#[test]
fn resource_replace_move_forces_current_decoration_endpoints_instead_of_retaining_old_spans() {
    use crate::decorations::{DecorationInput, DecorationType};
    let fixture = Fixture::new();
    let source = dirty_source("ab");
    let mut target = model("target.cpp", "a");
    let kind = DecorationType::new(
        "restore-marker".into(),
        "fixture.owner".into(),
        serde_json::json!({"rangeBehavior":0,"color":"#ff0000"}),
    )
    .unwrap();
    let input: DecorationInput = serde_json::from_value(serde_json::json!({
        "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}
    }))
    .unwrap();
    target
        .apply_editor_decorations(&target.editor_view_identity(), kind, vec![input])
        .unwrap();
    let theme = crate::theme::Theme::default();
    assert_eq!(
        target.decoration_overlay(0..1, &theme).style_at(0).fg,
        Some(ratatui::style::Color::Rgb(255, 0, 0))
    );
    transfer(&source, &mut target, "disk", &fixture);
    assert_eq!(target.text.to_string(), "ab");
    assert_eq!(target.decoration_overlay(0..2, &theme).style_at(0).fg, None);
    assert_eq!(target.editor_decoration_count(), 1);
}

#[test]
fn clean_reload_charges_and_publishes_shared_backing_once_without_halving_native_file_limit() {
    let fixture = Fixture::new();
    let mut target = model("target.cpp", "old");
    let loaded = Rope::from_str(&"x".repeat(17 * 1024 * 1024));
    let backing = loaded.clone();
    let prepared = target
        .capture_resource_model()
        .unwrap()
        .prepare_reload(loaded, &fixture.control())
        .unwrap();
    target
        .prepare_resource_model_commit(prepared, &fixture.control(), 8 * 1024 * 1024)
        .unwrap()
        .publish();
    assert_eq!(target.text.len_bytes(), 17 * 1024 * 1024);
    assert!(target.text.is_instance(&backing));
    assert!(
        target
            .text
            .is_instance(target.disk_content.as_ref().unwrap())
    );
    assert!(!target.dirty());
}
#[cfg(target_os = "linux")]
#[test]
fn actual_overwrite_move_restores_dirty_source_into_surviving_target_and_archives_displaced_work() {
    let fixture = Fixture::new();
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let from = root.join("source.cpp");
    let to = root.join("target.cpp");
    fs::write(&from, "source disk 猫🙂\r\n").unwrap();
    fs::write(&to, "target disk\n").unwrap();
    let mut source = Document::open_existing(&from).unwrap();
    source.insert("dirty source ", false);
    let mut target = Document::open_existing(&to).unwrap();
    target.insert("displaced dirty ", false);
    let source_capture = source.capture_resource_model().unwrap();
    let target_capture = target.capture_resource_model().unwrap();
    let original_target = target_capture
        .archive(&fixture.control(), 1024 * 1024, 1024 * 1024)
        .unwrap();
    let source_archive = source_capture
        .archive(&fixture.control(), 1024 * 1024, 1024 * 1024)
        .unwrap();
    let capture = Capture::prepare(
        vec![Operation::Move {
            source: from.clone(),
            destination: to.clone(),
            options: FileOptions {
                overwrite: Presence::Value(true),
                ..FileOptions::default()
            },
        }],
        vec![source_archive, original_target.clone()],
        Limits::default(),
        &fixture.control(),
    )
    .unwrap();
    let staged =
        StagedJournal::persist(capture, &root.join("journal"), &fixture.control()).unwrap();
    source_capture.check_current(&source).unwrap();
    target_capture.check_current(&target).unwrap();
    let mut executor = Executor::authorize(
        staged,
        ExecutionPolicy::default(),
        ExecutionLimits::default(),
        &fixture.control(),
    )
    .unwrap();
    assert_eq!(
        executor.execute_authorized(&fixture.control()).state,
        ExecutionState::Completed
    );
    let disk = crate::document::read_disk(&to).unwrap().unwrap();
    let id = target.id;
    let restored = source_capture
        .prepare_transfer(target_capture, disk, &fixture.control())
        .unwrap();
    target
        .prepare_resource_model_commit(restored, &fixture.control(), 8 * 1024 * 1024)
        .unwrap()
        .publish();
    assert_eq!(target.id, id);
    assert_ne!(target.id, source.id);
    assert_eq!(target.text, source.text);
    assert!(target.dirty());
    assert!(!from.exists());
    assert_eq!(fs::read(&to).unwrap(), "source disk 猫🙂\r\n".as_bytes());
    assert_eq!(executor.originals().archives()[1], original_target);
    target.undo();
    assert_eq!(target.text.to_string(), "displaced dirty target disk\n");
    assert_eq!(
        executor.rollback_authorized(&fixture.control()).state,
        ExecutionState::RolledBack
    );
    assert_eq!(fs::read(from).unwrap(), "source disk 猫🙂\r\n".as_bytes());
    assert_eq!(fs::read(to).unwrap(), b"target disk\n");
}
