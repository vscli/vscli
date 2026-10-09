//! Opt-in native integrity checks using unchanged, externally prepared packages.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Modal},
    extensions::Package,
    keys::Profile,
};
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
fn activate(app: &mut App, variable: &str, id: &str, version: &str) {
    let directory =
        PathBuf::from(std::env::var(variable).expect("Set the pinned package directory"));
    let package = Package::read(&directory).unwrap();
    assert_eq!(package.id, id);
    assert_eq!(package.version, version);
    app.start_extension_packages(vec![package]).unwrap();
    until(app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
}
fn save_undo(app: &mut App, path: &Path, id: u64, original: &str, expected: &str) {
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    assert_eq!(app.doc().text.to_string(), expected);
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    app.doc_mut().save().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected.as_bytes());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), original);
    app.doc_mut().save().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}
#[test]
#[ignore = "requires compiled pinned NPM Intellisense 1.4.5 in VSCLI_NPM_INTELLISENSE"]
fn unchanged_npm_intellisense_completes_native_crlf_buffer_and_undo() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("package.json"),
        r#"{"dependencies":{"lodash":"4.17.21"}}"#,
    )
    .unwrap();
    let path = root.path().join("input.js");
    let original = "import value from 'lo';\r\n// 🙂\r\n";
    std::fs::write(&path, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let id = app.doc().id;
    app.doc_mut()
        .move_to(original.find("lo").unwrap() + 2, false);
    activate(
        &mut app,
        "VSCLI_NPM_INTELLISENSE",
        "christian-kohler.npm-intellisense",
        "1.4.5",
    );
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(
        &mut app,
        |app| matches!(&app.modal, Some(Modal::Language { items, .. }) if items.iter().any(|i| i.label == "lodash")),
    );
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    save_undo(
        &mut app,
        &path,
        id,
        original,
        "import value from 'lodash';\r\n// 🙂\r\n",
    );
}
#[test]
#[ignore = "requires compiled pinned SQL Formatter VSCode 4.2.6 in VSCLI_SQL_FORMATTER"]
fn unchanged_sql_formatter_preserves_shared_identity_crlf_bytes_and_undo() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.sql");
    let original = "select '🙂' as greeting, id from users where id=1;\r\n";
    let expected = "select\r\n    '🙂' as greeting,\r\n    id\r\nfrom\r\n    users\r\nwhere\r\n    id = 1;\r\n";
    std::fs::write(&path, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let id = app.doc().id;
    app.execute("workbench.action.splitEditor", Value::Null);
    activate(
        &mut app,
        "VSCLI_SQL_FORMATTER",
        "renesaarsoo.sql-formatter-vsc",
        "4.2.6",
    );
    app.execute("editor.action.formatDocument", Value::Null);
    until(&mut app, |app| {
        app.message.starts_with("Extension formatting applied")
    });
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    save_undo(&mut app, &path, id, original, expected);
}
