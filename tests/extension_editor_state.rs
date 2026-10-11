//! Actual selected extension language/selection services, preserving native histories and disks.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, PromptKind},
    extensions::Client,
    keys::Profile,
    recovery::Recovery,
};

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
            "Editor state fixture timeout: {}",
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
    std::fs::write(root.join("b.txt"), "other\r\n").unwrap();
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
    assert_eq!(app.doc().text.to_string(), DIRTY);
    let extension =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-state-extension");
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
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "editorState.language")
        })
    });
    app
}
fn disk_unchanged(root: &Path) {
    assert_eq!(
        std::fs::read(root.join("a.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"other\r\n");
}
fn original_history(app: &mut App, root: &Path, id: u64, revision: u64) {
    command(
        app,
        "editorState.original",
        "API editor state original;identity=true;dirty=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    command(
        app,
        "editorState.clean",
        "API editor state original undo;clean=true",
    );
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    command(
        app,
        "editorState.dirty",
        "API editor state original redo;dirty=true",
    );
    disk_unchanged(root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn native_selection_overflow_pauses_mirrors_without_stopping_host_and_resumes_original_model() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let original_count = app.documents.len();
    for _ in 0..2 {
        let mut hidden = vscli::document::Document::from_text("retained hidden");
        hidden.secondary = vec![vscli::document::Selection::caret(0); 8192];
        app.documents.push(hidden);
    }
    until(&mut app, |app| {
        app.message.contains("Extension document mirrors paused")
    });
    assert!(app.extension_host.as_ref().is_some_and(|host| host.ready));
    app.doc_mut().move_to(0, false);
    app.doc_mut().insert("more ", false);
    app.poll();
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), format!("more {DIRTY}"));
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), DIRTY);
    disk_unchanged(&root);
    app.documents.truncate(original_count);
    until(&mut app, |app| {
        app.message == "Extension document mirrors resumed"
    });
    command(
        &mut app,
        "editorState.language",
        "API language lifecycle;identity=true;registered=true;quiet=true",
    );
    original_history(&mut app, &root, id, revision);
}
#[test]
fn public_language_change_same_object_events_noop_unknown_and_contributed_id_preserve_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    command(
        &mut app,
        "editorState.language",
        "API language lifecycle;identity=true;registered=true;quiet=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().language_override(), Some("python"));
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    disk_unchanged(&root);
    original_history(&mut app, &root, id, revision);
    assert_eq!(app.doc().language_override(), Some("python"));
}
#[test]
fn public_editor_selections_reverse_multiple_utf16_empty_inactive_and_native_event_preserve_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    command(
        &mut app,
        "editorState.selections",
        "API selections;reverse=true;multiple=true;invalid=refused",
    );
    assert_eq!(app.doc().anchor, Some(8));
    assert_eq!(app.doc().cursor, 6);
    assert_eq!(app.doc().secondary.len(), 1);
    assert_eq!(app.doc().secondary[0].anchor, Some(12));
    assert_eq!(app.doc().secondary[0].cursor, 16);
    command(
        &mut app,
        "editorState.clamp",
        "API selections clamped;UTF16=true;empty=origin",
    );
    assert_eq!(app.doc().cursor, 0);
    assert_eq!(app.doc().anchor.unwrap_or(app.doc().cursor), 0);
    assert!(app.doc().secondary.is_empty());
    command(
        &mut app,
        "editorState.inactive",
        "API inactive selections;focus=other;identity=true",
    );
    assert_ne!(app.doc().id, id);
    assert_eq!(
        app.doc().path.as_deref(),
        Some(root.join("b.txt").as_path())
    );
    assert_eq!(app.doc().text.to_string(), "other\r\n");
    assert!(!app.doc().dirty());
    command(
        &mut app,
        "editorState.armMovement",
        "API native movement armed",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().anchor, Some(16));
    assert_eq!(app.doc().cursor, 13);
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    until(&mut app, |app| {
        app.message == "API native movement event;identity=true"
    });
    assert_eq!(app.doc().cursor, 16);
    assert_eq!(app.doc().anchor.unwrap_or(app.doc().cursor), 16);
    assert_eq!(app.doc().revision, revision);
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    disk_unchanged(&root);
    original_history(&mut app, &root, id, revision);
}
#[test]
fn public_explicit_language_survives_actual_save_as_resource_transition_and_dirty_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let original_id = app.doc().id;
    let original_revision = app.doc().revision;
    command(
        &mut app,
        "editorState.prepareSave",
        "API language save prepared;python=true;CRLF=true",
    );
    let id = app.doc().id;
    assert_ne!(id, original_id);
    assert!(app.doc().path.is_none());
    assert_eq!(app.doc().language_override(), Some("python"));
    app.execute("editorState.save", Value::Null);
    until(&mut app, |app| {
        app.prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
    });
    let destination = root.join("state-saved.cpp");
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));
    app.event(Event::Paste(destination.to_string_lossy().into_owned()));
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    until(&mut app, |app| {
        app.message == "API language saved;python=true;events=close/open/save"
    });
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().path.as_ref(), Some(&destination));
    assert_eq!(app.doc().language_override(), Some("python"));
    assert_eq!(std::fs::read(&destination).unwrap(), ORIGINAL.as_bytes());
    assert!(!app.doc().dirty());
    command(
        &mut app,
        "editorState.editSaved",
        "API saved language edited;python=true;dirty=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), format!("file {ORIGINAL}"));
    let recovery = Recovery::new(root.join("recovery")).unwrap();
    recovery.persist(&app.documents).unwrap();
    drop(recovery);
    let recovery = Recovery::new(root.join("recovery")).unwrap();
    let (restored, paths) = recovery.restore().unwrap();
    assert_eq!(paths.len(), 1);
    let restored_saved = restored
        .iter()
        .find(|doc| doc.path.as_ref() == Some(&destination))
        .unwrap();
    assert_eq!(restored_saved.language_override(), Some("python"));
    assert_eq!(restored_saved.text.to_string(), format!("file {ORIGINAL}"));
    assert!(restored_saved.dirty());
    // Recovery is separate from the live authoritative model and its shared Undo history.
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    assert_eq!(app.doc().language_override(), Some("python"));
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), format!("file {ORIGINAL}"));
    assert!(app.doc().dirty());
    assert_eq!(app.doc().language_override(), Some("python"));
    assert_eq!(std::fs::read(&destination).unwrap(), ORIGINAL.as_bytes());
    Recovery::consume(&paths);
    recovery.finish().unwrap();
    original_history(&mut app, &root, original_id, original_revision);
}
