//! Opt-in diagnostics qualification for the unchanged pinned Write Good Linter.
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};

const VERY: &str = "猫🙂 This is very unique.\r\n";
const PASSIVE: &str = "猫🙂 The cat was killed.\r\n";

fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn package() -> Package {
    let directory = PathBuf::from(
        std::env::var_os("VSCLI_WRITE_GOOD").expect("Set prepared pinned package directory"),
    );
    let package = Package::read(&directory).unwrap();
    assert_eq!(package.id, "travisthetechie.write-good-linter");
    assert_eq!(package.version, "0.1.7");
    let status = std::process::Command::new("python")
        .arg("tests/prepare_write_good.py")
        .arg("--verify-only")
        .arg(&directory)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "Pinned package preparation provenance changed"
    );
    package
}
fn ready(root: &std::path::Path, only_save: bool) -> App {
    let settings = root.join("settings.json");
    std::fs::write(
        &settings,
        json!({"write-good.debounce-time-in-ms":0,
        "write-good.only-lint-on-save":only_save})
        .to_string(),
    )
    .unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.start_extension_packages(vec![package()]).unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    assert!(app.active_document().is_none());
    app
}
fn has(app: &App, text: &str) -> bool {
    app.current_diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.message.contains(text))
}

#[test]
#[ignore = "requires unchanged compiled Write Good Linter 0.1.7 in VSCLI_WRITE_GOOD"]
fn unchanged_write_good_lints_open_edit_undo_close_and_owner_retirement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("document.md");
    std::fs::write(&path, VERY).unwrap();
    let mut app = ready(root.path(), false);
    app.open(&path).unwrap();
    let id = app.doc().id;
    let selections = app.doc().selections();
    until(&mut app, |app| has(app, "weasel word"));
    assert_eq!(app.doc().text.to_string(), VERY);
    assert_eq!(app.doc().selections(), selections);
    assert!(!app.doc().dirty());
    let diagnostic = app
        .current_diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.message.contains("weasel word"))
        .unwrap();
    assert_eq!(diagnostic.severity, Some(2));
    assert_eq!(diagnostic.range.start.line, 0);
    assert_eq!(
        diagnostic.range.start.character, 12,
        "Published offsets are UTF-16 after 猫🙂"
    );
    assert_eq!(diagnostic.range.end.character, 16);
    app.doc_mut().select_all();
    app.doc_mut().insert(PASSIVE, false);
    until(&mut app, |app| has(app, "passive voice"));
    assert_eq!(app.doc().id, id);
    assert_eq!(std::fs::read(&path).unwrap(), VERY.as_bytes());
    assert!(app.doc().dirty());
    app.doc_mut().save().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), PASSIVE.as_bytes());
    app.execute("undo", Value::Null);
    until(&mut app, |app| has(app, "weasel word"));
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), VERY);
    assert_eq!(std::fs::read(&path).unwrap(), PASSIVE.as_bytes());
    app.doc_mut().save().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), VERY.as_bytes());
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    until(&mut app, |app| app.active_document().is_none());
    assert!(app.current_diagnostics().is_empty());
    app.open(&path).unwrap();
    until(&mut app, |app| has(app, "weasel word"));
    let reopened = app.doc().id;
    app.execute(
        "vscli.extensions.stopSelected",
        json!({"id":"travisthetechie.write-good-linter"}),
    );
    until(&mut app, |app| app.current_diagnostics().is_empty());
    assert_eq!(app.doc().id, reopened);
    assert_eq!(app.doc().text.to_string(), VERY);
    assert_eq!(std::fs::read(&path).unwrap(), VERY.as_bytes());
}

#[test]
#[ignore = "requires unchanged compiled Write Good Linter 0.1.7 in VSCLI_WRITE_GOOD"]
fn unchanged_write_good_only_save_waits_for_actual_successful_native_save() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("document.md");
    std::fs::write(&path, VERY).unwrap();
    let mut app = ready(root.path(), true);
    app.open(&path).unwrap();
    let id = app.doc().id;
    until(&mut app, |app| has(app, "weasel word"));
    app.doc_mut().select_all();
    app.doc_mut().insert(PASSIVE, false);
    // Old publications must disappear immediately after a text epoch changes;
    // the unchanged extension must not publish the replacement until a save.
    let before_save = Instant::now() + Duration::from_millis(250);
    while Instant::now() < before_save {
        app.poll();
        assert!(
            app.current_diagnostics().is_empty(),
            "only-lint-on-save published before saving"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(std::fs::read(&path).unwrap(), VERY.as_bytes());
    std::fs::write(&path, "EXTERNAL\r\n").unwrap();
    let selections = app.doc().selections();
    assert!(app.doc_mut().save().is_err());
    let failed_save = Instant::now() + Duration::from_millis(150);
    while Instant::now() < failed_save {
        app.poll();
        assert!(
            app.current_diagnostics().is_empty(),
            "Failed save emitted a save event"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), PASSIVE);
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(std::fs::read(&path).unwrap(), b"EXTERNAL\r\n");
    std::fs::write(&path, VERY).unwrap();
    app.doc_mut().save().unwrap();
    until(&mut app, |app| has(app, "passive voice"));
    assert_eq!(app.doc().id, id);
    assert_eq!(std::fs::read(&path).unwrap(), PASSIVE.as_bytes());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), VERY);
    let undo_without_save = Instant::now() + Duration::from_millis(150);
    while Instant::now() < undo_without_save {
        app.poll();
        assert!(
            app.current_diagnostics().is_empty(),
            "Undo emitted a save event"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    app.doc_mut().save().unwrap();
    until(&mut app, |app| has(app, "weasel word"));
    assert_eq!(app.doc().id, id);
    assert_eq!(std::fs::read(&path).unwrap(), VERY.as_bytes());
}
