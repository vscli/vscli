use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Client, keys::Profile};

const ORIGINAL: &str = "猫🙂 A\r\nlast\r\n";
const DIRTY: &str = "dirty 猫🙂 A\r\nlast\r\n";
const FILES: &[(&str, &str)] = &[
    ("a.txt", ORIGINAL),
    ("src/keep.rs", "keep\r\n"),
    ("src/excluded.rs", "excluded\r\n"),
    ("src/sub/deep.rs", "deep\r\n"),
    ("ignored/hidden.rs", "hidden\r\n"),
    ("assets/note.txt", "note\r\n"),
    (".gitignore", "src/keep.rs\n"),
];

fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "FindFiles fixture timeout: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn unchanged(root: &Path) {
    for (name, text) in FILES {
        assert_eq!(
            std::fs::read(root.join(name)).unwrap(),
            text.as_bytes(),
            "Search modified {name}"
        );
    }
}
fn ready(root: &Path) -> App {
    for (name, text) in FILES {
        let file = root.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let settings = root.join("settings.json");
    std::fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off",
            "editor.formatOnSave":false,"editor.codeActionsOnSave":{},
            "files.exclude":{"**/excluded.rs":true,"**/ignored/**":true},
            "search.exclude":{"**/keep.rs":true}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("a.txt")).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|d| d.path.as_deref() == Some(root.join("a.txt").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    let extension =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/find-files-extension");
    let node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.extension_host = Some(
        Client::start(
            &node,
            &extension,
            root,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready && host.commands.iter().any(|(_, id)| id == "find.query")
        })
    });
    app
}
fn search_retains_history(command: &str, receipt: &str) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let saved = app.doc().saved_revision;
    let cursor = app.doc().cursor;
    let anchor = app.doc().anchor;
    let snapshot = app.doc().capture_save(root.join("a.txt")).unwrap();
    app.execute(command, Value::Null);
    until(&mut app, |app| app.message == receipt);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().saved_revision, saved);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().anchor, anchor);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    let after = app.doc().capture_save(root.join("a.txt")).unwrap();
    assert_eq!(after.text_epoch(), snapshot.text_epoch());
    assert_eq!(after.save_generation(), snapshot.save_generation());
    assert_eq!(
        app.documents.len(),
        1,
        "Search must not open matching documents"
    );
    unchanged(&root);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    app.execute("find.clean", Value::Null);
    until(&mut app, |app| {
        app.message == "API search retained clean identity"
    });
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    app.execute("find.dirty", Value::Null);
    until(&mut app, |app| {
        app.message == "API search retained dirty identity"
    });
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn selected_host_find_files_scopes_limits_and_relative_base_preserve_dirty_history_and_disks() {
    search_retains_history(
        "find.query",
        "API find scopes=true;relative=true;limits=true",
    );
}
#[test]
fn selected_host_find_files_pre_cancel_fresh_search_and_active_cancel_preserve_dirty_history_and_disks()
 {
    search_retains_history("find.cancel", "API find cancelled=true;pre-fresh=true");
}
