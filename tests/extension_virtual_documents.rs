use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Client, keys::Profile};

const ORIGINAL: &str = "猫🙂 A\r\nlast\r\n";
const DIRTY: &str = "dirty 猫🙂 A\r\nlast\r\n";
const FIRST: &str = "// 猫🙂\nint first = 1;\nnext\n";
const REFRESHED: &str = "// β🙂\nint refreshed = 2;\n";
const LATEST: &str = "// latest 猫🙂\nint latest = 3;\n";
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Virtual document fixture timeout: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, id: &str, receipt: &str) {
    app.execute(id, Value::Null);
    until(app, |app| app.message == receipt);
}
fn ready(root: &Path) -> App {
    std::fs::write(root.join("a.txt"), ORIGINAL).unwrap();
    let config = root.join("settings.json");
    std::fs::write(
        &config,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off",
            "editor.formatOnSave":false,"editor.codeActionsOnSave":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(config)).unwrap();
    app.open(&root.join("a.txt")).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("a.txt").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    let extension =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/virtual-document-extension");
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
            host.ready && host.commands.iter().any(|(_, id)| id == "virtual.open")
        })
    });
    app
}
fn ordinary_history(
    app: &mut App,
    root: &Path,
    id: u64,
    revision: u64,
    cursor: usize,
    restore: bool,
) {
    if restore {
        command(
            app,
            "virtual.original",
            "API original restored;dirty=true;identity=true",
        );
    }
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    assert_eq!(
        std::fs::read(root.join("a.txt")).unwrap(),
        ORIGINAL.as_bytes()
    );
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    command(app, "virtual.clean", "API original clean history retained");
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    command(app, "virtual.dirty", "API original dirty history retained");
    assert_eq!(
        std::fs::read(root.join("a.txt")).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert!(!root.join("sample.cpp").exists());
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn selected_content_provider_open_readonly_refresh_preserve_native_dirty_identity_and_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    let proof = app.doc().capture_save(root.join("a.txt")).unwrap();
    command(
        &mut app,
        "virtual.open",
        "API virtual opened;identity=true;LF=true",
    );
    let virtual_id = app.doc().id;
    let virtual_revision = app.doc().revision;
    assert_ne!(virtual_id, id);
    assert!(app.doc().path.is_none());
    assert!(app.doc().is_read_only());
    assert_eq!(app.doc().text.to_string(), FIRST);
    command(
        &mut app,
        "virtual.readonly",
        "API virtual readonly;edit=false;save=false",
    );
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
    )));
    app.event(Event::Paste("forbidden 猫🙂\r\n".into()));
    app.execute("workbench.action.files.save", Value::Null);
    assert!(
        app.prompt.is_none(),
        "Readonly Save must never launch Save As"
    );
    assert_eq!(app.doc().id, virtual_id);
    assert_eq!(app.doc().revision, virtual_revision);
    assert_eq!(app.doc().text.to_string(), FIRST);
    assert!(!app.doc().dirty());
    command(
        &mut app,
        "virtual.refresh",
        "API virtual refreshed;identity=true;event=true",
    );
    assert_eq!(app.doc().id, virtual_id);
    assert_eq!(app.doc().text.to_string(), REFRESHED);
    assert!(app.doc().revision > virtual_revision);
    assert!(!app.doc().dirty());
    command(
        &mut app,
        "virtual.original",
        "API original restored;dirty=true;identity=true",
    );
    let after = app.doc().capture_save(root.join("a.txt")).unwrap();
    assert_eq!(after.text_epoch(), proof.text_epoch());
    assert_eq!(after.save_generation(), proof.save_generation());
    ordinary_history(&mut app, &root, id, revision, cursor, false);
}
#[test]
fn held_obsolete_virtual_refresh_latest_wins_and_disposed_cached_document_closes_without_touching_original()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    command(
        &mut app,
        "virtual.open",
        "API virtual opened;identity=true;LF=true",
    );
    let virtual_id = app.doc().id;
    command(
        &mut app,
        "virtual.refresh",
        "API virtual refreshed;identity=true;event=true",
    );
    command(
        &mut app,
        "virtual.hold",
        "API virtual obsolete callback held",
    );
    assert_eq!(app.doc().text.to_string(), REFRESHED);
    command(
        &mut app,
        "virtual.latest",
        "API virtual latest wins;identity=true",
    );
    assert_eq!(app.doc().id, virtual_id);
    assert_eq!(app.doc().text.to_string(), LATEST);
    command(
        &mut app,
        "virtual.dispose",
        "API virtual disposed;cached=true;readonly=true",
    );
    assert_eq!(app.doc().id, virtual_id);
    assert_eq!(app.doc().text.to_string(), LATEST);
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert!(app.prompt.is_none());
    assert_eq!(app.doc().id, id);
    ordinary_history(&mut app, &root, id, revision, cursor, false);
}
