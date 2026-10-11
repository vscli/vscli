//! Public snippet insertion uses actual native linked fields, views and source-owned history.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
const ORIGINAL: &str = "猫🙂 base\r\nnext\r\n";
const DIRTY: &str = "dirty 猫🙂 base\r\nnext\r\n";
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut terminal = Terminal::new(TestBackend::new(140, 32)).unwrap();
        terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Editor snippet fixture timeout: {}",
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-snippets-extension");
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![Package::read(&extension).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "editorSnippets.linked")
        })
    });
    app
}
fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.event(Event::Key(KeyEvent::new(code, modifiers)));
}
fn unchanged(root: &Path) {
    assert_eq!(
        std::fs::read(root.join("a.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"other\r\n");
}
#[test]
fn public_snippet_builder_linked_unicode_fields_native_tab_backtab_escape_and_grouped_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    command(
        &mut app,
        "editorSnippets.linked",
        "API snippets linked;builder=true;events=true;UTF16=true",
    );
    assert_eq!(app.doc().text.to_string(), format!("猫-猫-🙂{DIRTY}"));
    assert!(app.doc().in_snippet());
    assert_eq!(app.doc().anchor, Some(0));
    assert_eq!(app.doc().cursor, 1);
    assert_eq!(app.doc().secondary.len(), 1);
    app.event(Event::Paste("x".into()));
    command(
        &mut app,
        "editorSnippets.typed",
        "API snippets typed;linked=true",
    );
    assert_eq!(app.doc().text.to_string(), format!("x-x-🙂{DIRTY}"));
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    command(
        &mut app,
        "editorSnippets.field2",
        "API snippets Tab;field=2;UTF16=true",
    );
    assert_eq!(app.doc().selected_text().as_deref(), Some("🙂"));
    key(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
    command(
        &mut app,
        "editorSnippets.back",
        "API snippets BackTab;field=1;linked=true",
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    command(
        &mut app,
        "editorSnippets.escaped",
        "API snippets Escape;secondary=removed",
    );
    assert!(!app.doc().in_snippet());
    app.event(Event::Paste("q".into()));
    command(
        &mut app,
        "editorSnippets.q",
        "API snippets escaped typing;firstOnly=true",
    );
    assert_eq!(app.doc().text.to_string(), format!("q-x-🙂{DIRTY}"));
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.textTyped",
        "API snippets typed Undo;source=true",
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.expanded",
        "API snippets expansion Undo;source=true",
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.clean",
        "API snippets original Undo;clean=true",
    );
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.expanded",
        "API snippets expansion Undo;source=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.textTyped",
        "API snippets typed Undo;source=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.q",
        "API snippets escaped typing;firstOnly=true",
    );
    assert_eq!(app.doc().id, id);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_snippet_inactive_shared_editor_focus_location_overloads_empty_fallback_and_atomic_array_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    command(
        &mut app,
        "editorSnippets.inactive",
        "API snippets inactive;focus=second;session=target;events=true",
    );
    assert_eq!(app.active_pane, 1);
    assert!(app.doc().in_snippet());
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    command(
        &mut app,
        "editorSnippets.first",
        "API snippets first pane;selection=original",
    );
    assert_eq!(app.active_pane, 0);
    assert!(!app.doc().in_snippet());
    assert_eq!(app.doc().cursor, 6);
    command(
        &mut app,
        "editorSnippets.overloads",
        "API snippets overloads;omitted/empty/Position/Range/arrays=true",
    );
    assert_eq!(app.active_pane, 1);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "Mdirty 猫🙂 base\r\nMR\r\n");
    assert!(!app.doc().in_snippet());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.arrayUndo",
        "API snippets array Undo;both=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.arrayFinal",
        "API snippets array Redo;both=true",
    );
    // Every default-stop API call owns one history boundary, including both array edits together.
    for _ in 0..8 {
        app.execute("undo", Value::Null);
    }
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.clean",
        "API snippets original Undo;clean=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    for _ in 0..8 {
        app.execute("redo", Value::Null);
    }
    command(
        &mut app,
        "editorSnippets.arrayFinal",
        "API snippets array Redo;both=true",
    );
    assert_eq!(app.doc().id, id);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_snippet_stale_reentrant_source_retired_editor_and_readonly_document_refuse_without_losing_edits()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    command(
        &mut app,
        "editorSnippets.inactive",
        "API snippets inactive;focus=second;session=target;events=true",
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    command(
        &mut app,
        "editorSnippets.reentrant",
        "API snippets stale getter;insert=refused;edit=retained",
    );
    assert_eq!(app.doc().text.to_string(), format!("stale {DIRTY}"));
    assert_eq!(app.doc().id, id);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    command(
        &mut app,
        "editorSnippets.retiredReadonly",
        "API snippets retired=reject;readonly=false;source=unchanged",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.clean",
        "API snippets original Undo;clean=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    assert_eq!(app.doc().id, id);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_snippet_whitespace_options_and_false_stops_keep_open_history_without_touching_other_model()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let original_id = app.doc().id;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    command(
        &mut app,
        "editorSnippets.optionsPrepare",
        "API snippets options;separateUntitled=true;CRLF=true",
    );
    let options_id = app.doc().id;
    assert_ne!(options_id, original_id);
    assert!(app.doc().path.is_none());
    assert!(app.doc().dirty());
    assert_eq!(app.doc().text.to_string(), "  tail\r\n");
    command(
        &mut app,
        "editorSnippets.optionsDefault",
        "API snippets default whitespace;childIndent=4;events=true",
    );
    assert_eq!(app.doc().text.to_string(), "  first\r\n    childtail\r\n");
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.optionsInitial",
        "API snippets options Undo;initial=true;dirty=true",
    );
    command(
        &mut app,
        "editorSnippets.optionsKeep",
        "API snippets keepWhitespace;childIndent=2;events=true",
    );
    assert_eq!(app.doc().text.to_string(), "  first\r\n  childtail\r\n");
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.optionsInitial",
        "API snippets options Undo;initial=true;dirty=true",
    );
    command(
        &mut app,
        "editorSnippets.optionsGrouped",
        "API snippets false stops;twoInsertions=true;openBoundary=true",
    );
    assert_eq!(app.doc().text.to_string(), "  ABtail\r\n");
    // Real typing must extend the explicitly open API history boundary.
    key(&mut app, KeyCode::Char('C'), KeyModifiers::NONE);
    command(
        &mut app,
        "editorSnippets.optionsTyped",
        "API snippets false stops typing;ABC=true",
    );
    assert_eq!(app.doc().text.to_string(), "  ABCtail\r\n");
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.optionsInitial",
        "API snippets options Undo;initial=true;dirty=true",
    );
    assert_eq!(app.doc().id, options_id);
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.optionsTyped",
        "API snippets false stops typing;ABC=true",
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.optionsInitial",
        "API snippets options Undo;initial=true;dirty=true",
    );
    command(
        &mut app,
        "editorSnippets.optionsOriginal",
        "API snippets options protected original;identity=true;history=true",
    );
    assert_eq!(app.doc().id, original_id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    let after = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(after.text_epoch(), proof.text_epoch());
    assert_eq!(after.save_generation(), proof.save_generation());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorSnippets.clean",
        "API snippets original Undo;clean=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorSnippets.dirty",
        "API snippets dirty history;source=true",
    );
    let options = app
        .documents
        .iter()
        .find(|doc| doc.id == options_id)
        .unwrap();
    assert_eq!(options.text.to_string(), "  tail\r\n");
    assert!(options.dirty());
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
