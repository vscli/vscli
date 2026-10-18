use super::*;
use crate::resource_journal::{
    Capture, FileOptions, Limits, Operation, RecoveryAuthority, StagedJournal,
};
use std::sync::atomic::{AtomicBool, Ordering};

fn document() -> Document {
    let mut document = Document::from_text("original 猫🙂\r\nsecond\n");
    document.path = Some(std::env::temp_dir().join("resource.cpp"));
    document.disk_content = Some(document.text.clone());
    document.set_language_override(Some("cpp".into())).unwrap();
    document.insert("dirty ", false);
    document
}
fn archive(source: &ResourceModelSource) -> ModelArchive {
    source
        .archive(
            &Control::new(
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5),
            ),
            1024 * 1024,
            1024 * 1024,
        )
        .unwrap()
}
#[test]
fn cow_source_keeps_original_dirty_bytes_and_baseline_after_edit_undo() {
    let mut document = document();
    let source = document.capture_resource_model().unwrap();
    let original = document.text.to_string();
    let baseline = document.disk_content.as_ref().unwrap().to_string();
    document.insert("independent ", false);
    document.undo();
    assert_eq!(document.text.to_string(), original);
    assert!(source.check_current(&document).is_err());
    let archive = archive(&source);
    assert_eq!(archive.text, original.as_bytes());
    let metadata: serde_json::Value = serde_json::from_slice(&archive.metadata).unwrap();
    assert_eq!(
        metadata["diskContent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<String>(),
        baseline
    );
    assert_eq!(metadata["eol"], "\r\n");
    assert_eq!(metadata["encoding"], "utf8");
    assert_eq!(metadata["languageOverride"], "cpp");
    assert_eq!(metadata["dirty"], true);
}
#[test]
fn views_remain_independent_of_source_authority() {
    let mut document = document();
    let source = document.capture_resource_model().unwrap();
    document.cursor = document.len();
    document.top = 8;
    document.left = 4;
    source.check_current(&document).unwrap();
    assert_eq!(archive(&source).proof, *source.proof());
}
#[test]
fn language_and_path_aba_never_renew_original_authority() {
    let mut document = document();
    let source = document.capture_resource_model().unwrap();
    document.set_language_override(Some("rust".into())).unwrap();
    document.set_language_override(Some("cpp".into())).unwrap();
    assert!(source.check_current(&document).is_err());
    let source = document.capture_resource_model().unwrap();
    let original = document.path.clone();
    document
        .set_resource_path(Some(std::env::temp_dir().join("other.cpp")))
        .unwrap();
    document.set_resource_path(original).unwrap();
    assert!(source.check_current(&document).is_err());
}
#[test]
fn history_head_and_saved_baseline_publication_invalidate_original_source() {
    let mut document = document();
    let source = document.capture_resource_model().unwrap();
    let revision = document.revision;
    document.undo.last_mut().unwrap().history_identity = std::sync::Arc::new(());
    assert_eq!(document.revision, revision);
    assert!(source.check_current(&document).is_err());
    let source = document.capture_resource_model().unwrap();
    document.save_generation += 1;
    assert!(source.check_current(&document).is_err());
}
#[test]
fn archive_charges_escaped_baselines_and_shared_raw_budget_before_allocation() {
    let mut document = document();
    document.disk_content = Some(Rope::from_str(&"\"\\\n".repeat(4096)));
    let source = document.capture_resource_model().unwrap();
    let canceled = AtomicBool::new(false);
    let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
    assert!(source.archive(&control, 1024 * 1024, 100).is_err());
    assert!(
        source
            .archive(&control, document.text.len_bytes(), 1024 * 1024)
            .is_err()
    );
    assert!(
        source
            .archive(&control, crate::resource_journal::MAX_BYTES + 1, 100)
            .is_err()
    );
    assert!(
        source
            .archive(&control, 1024 * 1024, 8 * 1024 * 1024 + 1)
            .is_err()
    );
    canceled.store(true, Ordering::Release);
    assert!(source.archive(&control, 1024 * 1024, 1024 * 1024).is_err());
    let live = AtomicBool::new(false);
    let expired = Control::new(&live, Instant::now());
    assert!(source.archive(&expired, 1024 * 1024, 1024 * 1024).is_err());
}
#[test]
fn only_native_writable_file_models_can_supply_resource_authority() {
    let mut document = Document::from_text("untitled");
    assert!(document.capture_resource_model().is_err());
    document.path = Some("relative.cpp".into());
    assert!(document.capture_resource_model().is_err());
    document.path = Some(std::env::temp_dir().join("file.cpp"));
    document.eol = "unsupported".into();
    assert!(document.capture_resource_model().is_err());
    let source = self::document().capture_resource_model().unwrap();
    assert!(source.check_current(&self::document()).is_err());
    let mut virtual_document = Document::from_virtual("preview:/a".into(), "readonly").unwrap();
    virtual_document.path = Some(std::env::temp_dir().join("readonly.cpp"));
    assert!(virtual_document.capture_resource_model().is_err());
}
#[test]
fn missing_and_empty_baselines_remain_distinct_in_archive_metadata() {
    let mut document = document();
    document.disk_content = None;
    let absent = archive(&document.capture_resource_model().unwrap());
    let absent: serde_json::Value = serde_json::from_slice(&absent.metadata).unwrap();
    assert!(absent["diskContent"].is_null());
    document.disk_content = Some(Rope::new());
    let empty = archive(&document.capture_resource_model().unwrap());
    let empty: serde_json::Value = serde_json::from_slice(&empty.metadata).unwrap();
    assert!(empty["diskContent"].is_array());
    assert!(
        empty["diskContent"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value == "")
    );
}
#[test]
fn actual_native_archive_survives_durable_cold_recovery_with_exact_unsaved_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let path = root.join("original.cpp");
    fs::write(&path, "original 猫🙂\r\n").unwrap();
    let mut document = Document::open_existing(&path).unwrap();
    document.insert("dirty ", false);
    let source = document.capture_resource_model().unwrap();
    let original = archive(&source);
    let canceled = AtomicBool::new(false);
    let control = Control::new(&canceled, Instant::now() + Duration::from_secs(5));
    let capture = Capture::prepare(
        vec![Operation::Delete {
            path: path.clone(),
            options: FileOptions::default(),
        }],
        vec![original.clone()],
        Limits::default(),
        &control,
    )
    .unwrap();
    let journal = root.join("journal");
    let staged = StagedJournal::persist(capture, &journal, &control).unwrap();
    drop(staged);
    let recovered = RecoveryAuthority::load(&journal, Limits::default(), &control).unwrap();
    assert_eq!(recovered.capture().archives(), &[original]);
    assert_eq!(fs::read(path).unwrap(), "original 猫🙂\r\n".as_bytes());
    source.check_current(&document).unwrap();
    assert!(document.dirty());
}
