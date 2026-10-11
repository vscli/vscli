//! Real selected host visible editor identities and native pane-owned selections.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
const ORIGINAL: &str = "猫🙂 A\r\nlast\r\n";
const DIRTY: &str = "dirty 猫🙂 A\r\nlast\r\n";
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Editor inventory fixture timeout: {}",
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
    std::fs::write(root.join("a.cpp"), ORIGINAL).unwrap();
    std::fs::write(root.join("b.txt"), b"other\r\n").unwrap();
    let settings = root.join("settings.json");
    std::fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off",
            "editor.formatOnSave":false,"editor.codeActionsOnSave":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("a.cpp")).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("a.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    let extension =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-inventory-extension");
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![Package::read(&extension).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "editorInventory.shared")
        })
    });
    app
}
fn unchanged(root: &Path) {
    assert_eq!(
        std::fs::read(root.join("a.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"other\r\n");
}
fn shared_and_replace(app: &mut App) {
    let id = app.doc().id;
    let cursor = app.doc().cursor;
    command(
        app,
        "editorInventory.shared",
        "API inventory shared;editors=distinct;document=shared;focus=first",
    );
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.doc().cursor, cursor);
    command(
        app,
        "editorInventory.focusLanguage",
        "API inventory focus/language;stable=true;visibleEvents=quiet",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.doc().language_override(), Some("python"));
    command(
        app,
        "editorInventory.replace",
        "API inventory replaced;first=retained;second=other",
    );
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.panes[0].document, id);
    assert_eq!(app.panes[1].document, app.doc().id);
    assert_ne!(app.doc().id, id);
    assert_eq!(app.active_pane, 1);
}
#[test]
fn visible_editor_shared_model_distinct_views_inactive_setter_and_retired_editor_preserve_native_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    shared_and_replace(&mut app);
    let other_id = app.doc().id;
    let other_cursor = app.doc().cursor;
    command(
        &mut app,
        "editorInventory.inactive",
        "API inventory inactive;exactPane=true;focus=other",
    );
    assert_eq!(app.doc().id, other_id);
    assert_eq!(app.doc().cursor, other_cursor);
    assert_eq!(app.doc().text.to_string(), "other\r\n");
    assert!(!app.doc().dirty());
    command(
        &mut app,
        "editorInventory.retired",
        "API inventory retired;replacement=untouched",
    );
    assert_eq!(app.doc().id, other_id);
    assert_eq!(app.doc().cursor, other_cursor);
    command(
        &mut app,
        "editorInventory.original",
        "API inventory original;identity=true;dirty=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().anchor, Some(16));
    assert_eq!(app.doc().cursor, 13);
    assert_eq!(app.doc().revision, revision);
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    assert_eq!(app.doc().text.to_string(), DIRTY);
    command(
        &mut app,
        "editorInventory.armMovement",
        "API inventory native movement armed",
    );
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    until(&mut app, |app| {
        app.message == "API inventory native movement;pane=first;event=true"
    });
    assert_eq!(app.doc().cursor, 16);
    assert_eq!(app.doc().revision, revision);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorInventory.clean",
        "API inventory undo;clean=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorInventory.dirty",
        "API inventory redo;dirty=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    assert_eq!(app.panes.len(), 2);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn visible_editor_group_close_updates_survivor_column_then_empty_workbench_inventory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    shared_and_replace(&mut app);
    let survivor = app.doc().id;
    command(
        &mut app,
        "editorInventory.original",
        "API inventory original;identity=true;dirty=true",
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorInventory.clean",
        "API inventory undo;clean=true",
    );
    // Only clean models are closed. The independent test above qualifies dirty shared Undo/Redo.
    app.execute("workbench.action.closeEditorsInGroup", Value::Null);
    until(&mut app, |app| {
        app.panes.len() == 1 && app.doc().id == survivor
    });
    command(
        &mut app,
        "editorInventory.columns",
        "API inventory close group;survivor=stable;column=1;event=true",
    );
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.doc().id, survivor);
    assert_eq!(app.active_pane, 0);
    app.execute("workbench.action.closeAllEditors", Value::Null);
    until(&mut app, |app| app.active_document().is_none());
    command(
        &mut app,
        "editorInventory.empty",
        "API inventory empty;visible=0;active=undefined",
    );
    assert!(app.panes.is_empty());
    assert!(app.active_document().is_none());
    assert!(app.editor_groups().groups().is_empty());
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
