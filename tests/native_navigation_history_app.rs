//! Navigation history uses native models without a language server or JavaScript.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{fs, path::Path};
use vscli::{app::App, keys::Profile};

const BACK: &str = "workbench.action.navigateBack";
const FORWARD: &str = "workbench.action.navigateForward";

fn execute(app: &mut App, command: &str) {
    app.execute(command, Value::Null);
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn configured(root: &Path) -> App {
    let settings = root.join("settings.json");
    fs::write(&settings, r#"{"vscli.languageServer.enabled":false}"#).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.extension_node = root.join("node-unavailable").to_string_lossy().into_owned();
    app.configure_settings(Some(settings)).unwrap();
    app
}
fn source(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    let text: String = (1..=25)
        .map(|row| format!("row {row:02} 猫🙂 value\r\n"))
        .collect();
    fs::write(&path, text).unwrap();
    path
}
fn line(app: &mut App, row: usize) {
    execute(app, "workbench.action.gotoLine");
    app.prompt.as_mut().unwrap().text = row.to_string();
    key(app, KeyCode::Enter);
    assert_eq!(app.doc().row(), row - 1);
}

#[test]
fn back_forward_reuses_dirty_unicode_crlf_models_and_preserves_undo_and_disk() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let first = source(&root, "first.txt");
    let second = source(&root, "second.txt");
    let original = fs::read(&first).unwrap();
    let mut app = configured(&root);
    app.open(&first).unwrap();
    let first_id = app.doc().id;
    line(&mut app, 15);
    execute(&mut app, "cursorEnd");
    app.doc_mut().insert(" DIRTY", false);
    let dirty = app.doc().text.to_string();
    let cursor = app.doc().cursor;
    let revision = app.doc().revision;
    app.open(&second).unwrap();
    let second_id = app.doc().id;
    assert!(app.can_navigate_back());
    execute(&mut app, BACK);
    assert_eq!(app.doc().id, first_id);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert!(app.doc().dirty());
    assert_eq!(fs::read(&first).unwrap(), original);
    assert!(app.can_navigate_forward());
    execute(&mut app, FORWARD);
    assert_eq!(app.doc().id, second_id);
    execute(&mut app, BACK);
    assert_eq!(app.doc().id, first_id);
    execute(&mut app, "undo");
    assert_eq!(app.doc().text.to_string().as_bytes(), original);
    execute(&mut app, "redo");
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().id, first_id);
    execute(&mut app, "workbench.action.files.save");
    assert_eq!(fs::read(&first).unwrap(), dirty.as_bytes());
    assert_eq!(fs::read(&second).unwrap(), original);
    assert!(app.extension_host.is_none());
    assert!(app.lsp.is_none());
}

#[test]
fn switching_after_back_truncates_forward_and_keeps_the_new_destination() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let paths: Vec<_> = ["first.txt", "second.txt", "third.txt", "fourth.txt"]
        .map(|name| source(&root, name))
        .into();
    let mut app = configured(&root);
    for path in &paths[..3] {
        app.open(path).unwrap();
    }
    execute(&mut app, BACK);
    assert_eq!(app.doc().path.as_ref(), Some(&paths[1]));
    assert!(app.can_navigate_forward());
    app.open(&paths[3]).unwrap();
    let fourth = app.doc().id;
    assert!(!app.can_navigate_forward());
    execute(&mut app, FORWARD);
    assert_eq!(app.doc().id, fourth);
    execute(&mut app, BACK);
    assert_eq!(app.doc().path.as_ref(), Some(&paths[1]));
    execute(&mut app, FORWARD);
    assert_eq!(app.doc().id, fourth);
    assert!(app.documents.iter().all(|document| !document.dirty()));
}

#[test]
fn split_pane_history_restores_primary_selection_without_changing_the_other_view() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let first = source(&root, "first.txt");
    let second = source(&root, "second.txt");
    let original = fs::read(&first).unwrap();
    let mut app = configured(&root);
    app.open(&first).unwrap();
    let first_id = app.doc().id;
    line(&mut app, 3);
    app.doc_mut().add_cursor(1);
    let other_pane = app.panes[0].id;
    let other_view = app.doc().view_state(Some(other_pane));
    let other_cursor = other_view.cursor;
    let other_anchor = other_view.anchor;
    let other_secondary = other_view.secondary.clone();
    execute(&mut app, "workbench.action.splitEditorRight");
    let pane = app.panes[app.active_pane].id;
    app.doc_mut().clear_secondary();
    line(&mut app, 18);
    execute(&mut app, "cursorRightSelect");
    execute(&mut app, "cursorRightSelect");
    let cursor = app.doc().cursor;
    let anchor = app.doc().anchor;
    app.open(&second).unwrap();
    execute(&mut app, BACK);
    assert_eq!(app.doc().id, first_id);
    assert_eq!(app.panes[app.active_pane].id, pane);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().anchor, anchor);
    let other_view = app.doc().view_state(Some(other_pane));
    assert_eq!(other_view.cursor, other_cursor);
    assert_eq!(other_view.anchor, other_anchor);
    assert_eq!(other_view.secondary, other_secondary);
    assert_eq!(app.doc().text.to_string().as_bytes(), original);
    assert!(!app.doc().dirty());
}

#[test]
fn history_follows_save_as_identity_and_reuses_a_dirty_deleted_backing_model() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let first = source(&root, "first.txt");
    let second = source(&root, "second.txt");
    let destination = root.join("renamed.txt");
    let original = fs::read(&first).unwrap();
    let mut app = configured(&root);
    app.open(&first).unwrap();
    let id = app.doc().id;
    line(&mut app, 15);
    execute(&mut app, "cursorEnd");
    app.doc_mut().insert(" DIRTY", false);
    app.open(&second).unwrap();
    execute(&mut app, BACK);
    execute(&mut app, "workbench.action.files.saveAs");
    app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().path.as_ref(), Some(&destination));
    let saved = fs::read(&destination).unwrap();
    app.doc_mut().insert(" unsaved", false);
    let dirty = app.doc().text.to_string();
    app.open(&second).unwrap();
    fs::remove_file(&first).unwrap();
    fs::remove_file(&destination).unwrap();
    execute(&mut app, BACK);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().path.as_ref(), Some(&destination));
    assert_eq!(app.doc().text.to_string(), dirty);
    assert!(app.doc().dirty());
    execute(&mut app, "undo");
    assert_eq!(app.doc().text.to_string().as_bytes(), saved);
    execute(&mut app, "redo");
    assert_eq!(app.doc().text.to_string(), dirty);
    let revision = app.doc().revision;
    let saved_revision = app.doc().saved_revision;
    let disk_baseline = app.doc().disk_content.clone();
    execute(&mut app, "workbench.action.files.save");
    assert!(app.message.contains("File changed on disk"));
    assert!(!destination.exists());
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().saved_revision, saved_revision);
    assert_eq!(app.doc().disk_content, disk_baseline);
    assert!(app.doc().dirty());
    // Ordinary Save protects an externally removed backing file. Deliberately
    // recover the same authoritative buffer through the native Save As prompt.
    let recovered = root.join("recovered.txt");
    execute(&mut app, "workbench.action.files.saveAs");
    app.prompt.as_mut().unwrap().text = recovered.to_string_lossy().into_owned();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().path.as_ref(), Some(&recovered));
    assert_eq!(fs::read(&recovered).unwrap(), dirty.as_bytes());
    app.open(&second).unwrap();
    execute(&mut app, BACK);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().path.as_ref(), Some(&recovered));
    execute(&mut app, "undo");
    assert_eq!(app.doc().text.to_string().as_bytes(), saved);
    execute(&mut app, "redo");
    assert_eq!(app.doc().text.to_string(), dirty);
    execute(&mut app, "workbench.action.files.save");
    assert_eq!(fs::read(&recovered).unwrap(), dirty.as_bytes());
    assert!(!destination.exists());
    assert!(!first.exists());
    assert_eq!(fs::read(&second).unwrap(), original);
}

#[test]
fn discarded_closed_untitled_history_cannot_resurrect_unsaved_text() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let first = source(&root, "first.txt");
    let second = source(&root, "second.txt");
    let mut app = configured(&root);
    app.open(&first).unwrap();
    execute(&mut app, "workbench.action.files.newUntitledFile");
    let retired = app.doc().id;
    app.doc_mut().insert("DISCARDED SECRET 猫🙂", false);
    app.open(&second).unwrap();
    execute(&mut app, "workbench.action.previousEditor");
    assert_eq!(app.doc().id, retired);
    execute(&mut app, "workbench.action.closeActiveEditor");
    assert!(app.modal.is_some());
    key(&mut app, KeyCode::Char('d'));
    assert!(app.documents.iter().all(|document| document.id != retired));
    for _ in 0..16 {
        if !app.can_navigate_back() {
            break;
        }
        execute(&mut app, BACK);
        assert_ne!(app.doc().id, retired);
        assert!(app.doc().path.is_some());
        assert!(!app.doc().text.to_string().contains("DISCARDED SECRET"));
    }
    assert!(
        !app.can_navigate_back(),
        "The small journey should be exhausted"
    );
    assert!(app.documents.iter().all(|document| document.path.is_some()));
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
}
