use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, PromptKind},
    keys::Profile,
    lsp::Client,
};
fn until(app: &mut App, condition: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if condition(app) {
            return;
        }
        assert!(Instant::now() < end, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settle(app: &mut App) {
    let end = Instant::now() + Duration::from_millis(450);
    while Instant::now() < end {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn query(app: &mut App, value: &str) {
    app.prompt.as_mut().unwrap().select_all = true;
    app.event(Event::Paste(value.into()));
}
fn fixture(open: bool) -> (tempfile::TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    for name in ["main.cpp", "other.cpp", "invalid.cpp"] {
        std::fs::write(root.path().join(name), "x 😀foo\r\n").unwrap();
    }
    let mut app = App::new(root.path().into(), Profile::Linux);
    if open {
        app.open(&root.path().join("main.cpp")).unwrap();
    }
    app.lsp = Some(
        Client::start(
            "python3",
            &[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/symbol_server.py")
                .to_string_lossy()
                .into_owned()],
            root.path(),
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    (root, app)
}
fn workspace(app: &mut App, value: &str) {
    app.execute("workbench.action.showAllSymbols", Value::Null);
    query(app, value);
    until(app, |a| !a.symbol_items(value).is_empty());
}
#[test]
fn hierarchical_document_symbols_navigate_utf16_without_editing_shared_buffers() {
    let (root, mut app) = fixture(true);
    let id = app.doc().id;
    let revision = app.doc().revision;
    app.doc_mut().add_cursor(5);
    let other_selections = app.doc().selections();
    app.execute("workbench.action.splitEditorRight", Value::Null);
    app.doc_mut().add_cursor(6);
    let pane = app.active_pane;
    app.execute("workbench.action.gotoSymbol", Value::Null);
    until(&mut app, |a| a.symbol_items("").len() == 2);
    query(&mut app, "child");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().cursor, 2);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert!(app.panes.iter().all(|p| p.document == id));
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.doc().selections().len(), 1);
    app.focus_pane(0);
    assert_eq!(app.doc().selections(), other_selections);
    app.focus_pane(pane);
    assert_eq!(app.doc().cursor, 2);
    assert!(!app.doc().dirty());
    app.doc_mut().insert("Z", false);
    assert_eq!(app.doc().text.to_string(), "x Z😀foo\r\n");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "x 😀foo\r\n");
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        b"x \xf0\x9f\x98\x80foo\r\n"
    );
}
#[test]
fn workspace_symbols_focus_dirty_deleted_shared_targets_and_preserve_undo() {
    let (root, mut app) = fixture(true);
    app.open(&root.path().join("other.cpp")).unwrap();
    let end = app.doc().len();
    app.doc_mut().move_to(end, false);
    app.doc_mut().insert("dirty", false);
    let id = app.doc().id;
    app.execute("workbench.action.splitEditorRight", Value::Null);
    app.open(&root.path().join("main.cpp")).unwrap();
    workspace(&mut app, "other");
    std::fs::remove_file(root.path().join("other.cpp")).unwrap();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().id, id);
    assert!(app.doc().text.to_string().ends_with("dirty"));
    assert_eq!(app.doc().cursor, 2);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "x 😀foo\r\n");
}
#[test]
fn empty_workspace_async_navigation_requires_existing_files_and_valid_ranges() {
    let (root, mut app) = fixture(false);
    workspace(&mut app, "other");
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| a.active_document().is_some());
    assert_eq!(app.doc().cursor, 2);
    assert_eq!(app.documents.len(), 1);
    for name in ["missing", "invalid"] {
        workspace(&mut app, name);
        let id = app.doc().id;
        let cursor = app.doc().cursor;
        key(&mut app, KeyCode::Enter);
        until(&mut app, |a| a.message.starts_with("Symbol navigation:"));
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().cursor, cursor);
    }
    assert!(!root.path().join("missing.cpp").exists());
    #[cfg(unix)]
    {
        let fifo = root.path().join("fifo.cpp");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        workspace(&mut app, "fifo");
        key(&mut app, KeyCode::Enter);
        until(&mut app, |a| a.message.contains("Not a regular file"));
        assert_eq!(app.documents.len(), 1);
    }

    for query_text in ["unresolved", "nonfile", "oversized"] {
        app.execute("workbench.action.showAllSymbols", Value::Null);
        query(&mut app, query_text);
        until(&mut app, |a| a.message.starts_with("Language response:"));
        assert!(app.symbol_items("").is_empty());
    }
}
#[test]
fn canceled_debounced_and_changed_context_replies_do_not_restore_old_picker_or_cursor() {
    let (_root, mut app) = fixture(true);
    app.execute("workbench.action.gotoSymbol", Value::Null);
    key(&mut app, KeyCode::Esc);
    settle(&mut app);
    assert!(app.prompt.is_none());
    app.execute("workbench.action.gotoSymbol", Value::Null);
    app.doc_mut().insert("keep", false);
    settle(&mut app);
    assert!(app.prompt.is_none());
    assert!(app.doc().text.to_string().starts_with("keep"));
    app.execute("undo", Value::Null);
    app.execute("workbench.action.showAllSymbols", Value::Null);
    query(&mut app, "slow");
    until(&mut app, |a| a.message == "Loading symbols…"); // initial request may still be pending; allow debounce.
    let end = Instant::now() + Duration::from_millis(240);
    while Instant::now() < end {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    query(&mut app, "other");
    until(&mut app, |a| !a.symbol_items("other").is_empty());
    settle(&mut app);
    assert!(
        app.symbol_items("")
            .iter()
            .all(|s| s.label.starts_with("other"))
    );
    app.execute("workbench.action.showCommands", Value::Null);
    settle(&mut app);
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Palette
    ));
}
#[test]
fn target_changes_and_navigation_load_context_reject_without_cursor_mutation() {
    let (root, mut app) = fixture(true);
    app.open(&root.path().join("other.cpp")).unwrap();
    let target = app.doc().id;
    app.open(&root.path().join("main.cpp")).unwrap();
    workspace(&mut app, "other");
    app.documents
        .iter_mut()
        .find(|d| d.id == target)
        .unwrap()
        .insert("changed", false);
    key(&mut app, KeyCode::Enter);
    assert!(app.doc().name().contains("main.cpp"));
    assert!(app.prompt.is_none());
    app.execute("workbench.action.closeAllEditors", Value::Null); // dirty confirmation is canceled explicitly.
    key(&mut app, KeyCode::Esc);
    let (root, mut app) = fixture(true);
    workspace(&mut app, "other");
    key(&mut app, KeyCode::Enter);
    app.start_prompt(PromptKind::Palette, "native query".into());
    key(&mut app, KeyCode::Esc);
    settle(&mut app);
    assert_eq!(app.documents.len(), 1);
    assert!(app.doc().name().contains("main.cpp"));
    assert!(root.path().join("other.cpp").exists());
}
#[test]
#[ignore = "requires installed clangd; explicit C++ document/workspace symbol qualification"]
fn real_clangd_cpp_document_and_workspace_symbols_preserve_source() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    let text = "int twice(int value) { return value * 2; }\nint main() { return twice(2); }\n";
    std::fs::write(&path, text).unwrap();
    std::fs::write(root.path().join("compile_flags.txt"), "-std=c++17\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    app.lsp = Some(
        Client::start(
            "clangd",
            &["--background-index=false".into(), "-j=2".into()],
            root.path(),
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    app.execute("workbench.action.gotoSymbol", Value::Null);
    until(&mut app, |a| !a.symbol_items("twice").is_empty());
    query(&mut app, "twice");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().cursor, 4);
    workspace(&mut app, "main");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().row(), 1);
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert!(!app.doc().dirty());
}
