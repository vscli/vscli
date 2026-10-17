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
};

const ORIGINAL: &str = "猫🙂 A\r\nlast\r\n";
const DIRTY: &str = "dirty 猫🙂 A\r\nlast\r\n";
const INITIAL: &str = "猫🙂 generated\r\nint first = 1;\r\n";
const ASSOCIATED: &str = "猫🙂 associated\nint associated = 2;\n";
const PROTECTED: &[u8] = b"existing bytes must survive\r\n";
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Untitled fixture timeout: {}",
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
    app.open(&root.join("a.txt")).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("a.txt").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    let extension = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/untitled-document-extension");
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
            host.ready && host.commands.iter().any(|(_, id)| id == "untitled.options")
        })
    });
    app
}
fn save_as(app: &mut App) {
    app.execute("untitled.save", Value::Null);
    until(app, |app| {
        app.prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
    });
}
fn original_history(app: &mut App, root: &Path, id: u64, revision: u64, cursor: usize) {
    command(
        app,
        "untitled.original",
        "API untitled original restored;dirty=true;identity=true",
    );
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
    command(
        app,
        "untitled.clean",
        "API untitled original clean history retained",
    );
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    command(
        app,
        "untitled.dirty",
        "API untitled original dirty history retained",
    );
    assert_eq!(
        std::fs::read(root.join("a.txt")).unwrap(),
        ORIGINAL.as_bytes()
    );
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_untitled_options_hidden_open_initial_undo_and_real_save_as_cancel_accept_preserve_identity()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    let original = app.doc().capture_save(root.join("a.txt")).unwrap();
    command(
        &mut app,
        "untitled.options",
        "API untitled options;hidden=true;fresh=true;CRLF=true",
    );
    let untitled = app.doc().id;
    assert_ne!(untitled, id);
    assert!(app.doc().path.is_none());
    assert!(!app.doc().is_read_only());
    assert!(app.doc().api_dirty());
    assert_eq!(app.doc().text.to_string(), INITIAL);
    command(
        &mut app,
        "untitled.edit",
        "API untitled edit=true;event=true",
    );
    assert_eq!(app.doc().text.to_string(), format!("edit {INITIAL}"));
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "untitled.initial",
        "API untitled initial retained;dirty=true",
    );
    let initial_revision = app.doc().revision;
    let initial_epoch = app
        .doc()
        .capture_save(root.join("options-saved.cpp"))
        .unwrap()
        .text_epoch();
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), INITIAL);
    assert_eq!(app.doc().revision, initial_revision);
    assert_eq!(
        app.doc()
            .capture_save(root.join("options-saved.cpp"))
            .unwrap()
            .text_epoch(),
        initial_epoch
    );
    command(
        &mut app,
        "untitled.edit",
        "API untitled edit=true;event=true",
    );
    command(
        &mut app,
        "untitled.undo",
        "API untitled public undo;initial=true;dirty=true",
    );
    save_as(&mut app);
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    until(&mut app, |app| {
        app.message == "API untitled cancelled;dirty=true;event=false"
    });
    assert_eq!(app.doc().id, untitled);
    assert!(app.doc().path.is_none());
    assert!(!root.join("options-saved.cpp").exists());
    save_as(&mut app);
    let destination = root.join("options-saved.cpp");
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
        app.message == "API untitled saved;transition=true;events=close/open/save"
    });
    assert_eq!(app.doc().id, untitled);
    assert_eq!(app.doc().path.as_ref(), Some(&destination));
    assert_eq!(std::fs::read(&destination).unwrap(), INITIAL.as_bytes());
    assert!(!app.doc().dirty());
    command(
        &mut app,
        "untitled.editSaved",
        "API new file edited;dirty=true;oldClosed=true",
    );
    assert_eq!(app.doc().id, untitled);
    assert!(app.doc().dirty());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "untitled.cleanSaved",
        "API new file undo;clean=true;oldClosed=true",
    );
    assert_eq!(app.doc().id, untitled);
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "untitled.dirtySaved",
        "API new file redo;dirty=true;oldClosed=true",
    );
    assert_eq!(app.doc().id, untitled);
    assert_eq!(std::fs::read(&destination).unwrap(), INITIAL.as_bytes());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "untitled.cleanSaved",
        "API new file undo;clean=true;oldClosed=true",
    );
    command(
        &mut app,
        "untitled.original",
        "API untitled original restored;dirty=true;identity=true",
    );
    let after = app.doc().capture_save(root.join("a.txt")).unwrap();
    assert_eq!(after.text_epoch(), original.text_epoch());
    assert_eq!(after.save_generation(), original.save_generation());
    original_history(&mut app, &root, id, revision, cursor);
}
#[test]
fn public_associated_untitled_repeat_open_preserves_edited_identity_and_save_uses_missing_target_without_prompt()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    command(
        &mut app,
        "untitled.associated",
        "API associated untitled;cached=true;cpp=true",
    );
    let associated = app.doc().id;
    assert!(app.doc().path.is_none());
    assert_eq!(app.doc().text.to_string(), ASSOCIATED);
    assert!(!root.join("new.cpp").exists());
    command(
        &mut app,
        "untitled.saveAssociated",
        "API associated saved;direct=true;transition=true;events=close/open/save",
    );
    assert!(app.prompt.is_none());
    assert_eq!(app.doc().id, associated);
    assert_eq!(app.doc().path.as_ref(), Some(&root.join("new.cpp")));
    assert!(!app.doc().dirty());
    assert_eq!(
        std::fs::read(root.join("new.cpp")).unwrap(),
        ASSOCIATED.as_bytes()
    );
    original_history(&mut app, &root, id, revision, cursor);
}
#[test]
fn public_associated_untitled_existing_and_newly_appeared_targets_refuse_overwrite_and_keep_unsaved_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    std::fs::write(root.join("exists.cpp"), PROTECTED).unwrap();
    command(
        &mut app,
        "untitled.existing",
        "API associated existing rejected;original=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(std::fs::read(root.join("exists.cpp")).unwrap(), PROTECTED);
    command(
        &mut app,
        "untitled.late",
        "API associated late target ready;dirty=true",
    );
    let late = app.doc().id;
    let late_revision = app.doc().revision;
    let late_epoch = app
        .doc()
        .capture_save(root.join("late.cpp"))
        .unwrap()
        .text_epoch();
    // A real competing writer appears AFTER opening the associated blank resource.
    std::fs::write(root.join("late.cpp"), PROTECTED).unwrap();
    command(
        &mut app,
        "untitled.protected",
        "API associated late save refused;dirty=true;event=false",
    );
    assert!(app.prompt.is_none());
    assert_eq!(app.doc().id, late);
    assert_eq!(app.doc().revision, late_revision);
    assert_eq!(
        app.doc()
            .capture_save(root.join("late.cpp"))
            .unwrap()
            .text_epoch(),
        late_epoch
    );
    assert!(app.doc().path.is_none());
    assert!(app.doc().api_dirty());
    assert_eq!(std::fs::read(root.join("late.cpp")).unwrap(), PROTECTED);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, late);
    assert_eq!(app.doc().text.to_string(), "");
    assert!(app.doc().api_dirty());
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), ASSOCIATED);
    assert_eq!(std::fs::read(root.join("late.cpp")).unwrap(), PROTECTED);
    original_history(&mut app, &root, id, revision, cursor);
}

#[test]
fn fresh_api_untitled_names_skip_retained_recovered_resource_without_replacing_identity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let original_id = app.doc().id;
    let reserved = vscli::document::Document::from_untitled(
        Some("untitled:Untitled-1".into()),
        "reserved retained 猫",
        None,
        None,
    )
    .unwrap();
    let reserved_id = reserved.id;
    app.documents.push(reserved);
    command(
        &mut app,
        "untitled.options",
        "API untitled options;hidden=true;fresh=true;CRLF=true",
    );
    assert_ne!(app.doc().id, reserved_id);
    assert_ne!(app.doc().untitled_uri(), Some("untitled:Untitled-1"));
    let retained = app
        .documents
        .iter()
        .find(|document| document.id == reserved_id)
        .unwrap();
    assert_eq!(retained.untitled_uri(), Some("untitled:Untitled-1"));
    assert_eq!(retained.text.to_string(), "reserved retained 猫");
    assert!(retained.api_dirty());
    assert!(
        app.documents
            .iter()
            .any(|document| document.id == original_id && document.text == DIRTY)
    );
    assert_eq!(
        std::fs::read(root.join("a.txt")).unwrap(),
        ORIGINAL.as_bytes()
    );
    app.execute("vscli.extensions.stop", Value::Null);
}
