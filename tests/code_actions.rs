use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Modal},
    keys::Profile,
    lsp::Client,
};
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "Timed out: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn fixture(two: bool) -> (tempfile::TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    let main = root.path().join("main.rs");
    std::fs::write(&main, "bad\r\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    if two {
        let other = root.path().join("other.rs");
        std::fs::write(&other, "bad\r\n").unwrap();
        app.open(&other).unwrap();
    }
    app.open(&main).unwrap();
    app.lsp = Some(
        Client::start(
            "python3",
            &[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/code_action_server.py")
                .to_string_lossy()
                .into_owned()],
            root.path(),
            "rust".into(),
        )
        .unwrap(),
    );
    until(&mut app, |app| !app.current_diagnostics().is_empty());
    select_word(&mut app);
    (root, app)
}
fn select_word(app: &mut App) {
    app.doc_mut().move_to(0, false);
    app.doc_mut().move_to(3, true);
}
fn actions(app: &mut App) {
    app.execute("editor.action.quickFix", Value::Null);
    until(app, |app| matches!(app.modal, Some(Modal::Language { .. })));
}
fn choose(app: &mut App, title: &str) {
    let Some(Modal::Language {
        items, selected, ..
    }) = &mut app.modal
    else {
        panic!("No action picker: {}", app.message)
    };
    *selected = items
        .iter()
        .position(|item| item.label.starts_with(title))
        .unwrap_or_else(|| panic!("Missing {title}"));
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
}
#[test]
fn native_quick_fix_resolve_and_commands_preserve_crlf_shared_identity_and_undo() {
    let (root, mut app) = fixture(false);
    let id = app.doc().id;
    app.execute("workbench.action.splitEditorRight", Value::Null);
    actions(&mut app);
    choose(&mut app, "Fix selected text");
    assert_eq!(app.doc().text.to_string(), "fixed\r\n");
    assert_eq!(app.doc().id, id);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "bad\r\n");
    select_word(&mut app);
    app.execute("editor.action.refactor", Value::Null);
    until(&mut app, |app| {
        matches!(app.modal, Some(Modal::Language { .. }))
    });
    choose(&mut app, "Resolve refactor");
    until(&mut app, |app| app.doc().text == "resolved\r\n");
    app.execute("undo", Value::Null);
    for title in ["Execute command", "Legacy command"] {
        select_word(&mut app);
        actions(&mut app);
        choose(&mut app, title);
        until(&mut app, |app| app.message == "Fixture command applied");
        assert_eq!(app.doc().text.to_string(), "commanded\r\n");
        app.execute("undo", Value::Null);
    }
    assert_eq!(
        std::fs::read(root.path().join("main.rs")).unwrap(),
        b"bad\r\n"
    );
}
#[test]
fn invalid_multi_file_edits_are_atomic_and_supported_dirty_buffers_keep_their_identity() {
    let (root, mut app) = fixture(true);
    let original: Vec<_> = app
        .documents
        .iter()
        .map(|doc| (doc.id, doc.text.to_string()))
        .collect();
    for title in [
        "Disabled fix",
        "Atomic invalid target",
        "Stale version",
        "Resource operation",
        "Annotated edit",
        "Combined edit and command",
        "Unknown edit form",
        "Overlapping edits",
    ] {
        select_word(&mut app);
        actions(&mut app);
        choose(&mut app, title);
        assert!(app.message.contains("failed"), "{title}: {}", app.message);
        for (doc, (id, text)) in app.documents.iter().zip(&original) {
            assert_eq!(doc.id, *id);
            assert_eq!(doc.text.to_string(), *text, "{title}");
        }
    }
    for doc in &mut app.documents {
        doc.move_to(doc.len(), false);
        doc.insert("dirty", false);
    }
    select_word(&mut app);
    actions(&mut app);
    choose(&mut app, "Multiple dirty buffers");
    for doc in &app.documents {
        assert_eq!(doc.text.to_string(), "both\r\ndirty");
        assert!(doc.dirty());
    }
    for doc in &mut app.documents {
        doc.undo();
        assert_eq!(doc.text.to_string(), "bad\r\ndirty");
    }
    assert_eq!(
        std::fs::read(root.path().join("main.rs")).unwrap(),
        b"bad\r\n"
    );
    assert_eq!(
        std::fs::read(root.path().join("other.rs")).unwrap(),
        b"bad\r\n"
    );
}
#[test]
fn secondary_revision_changes_and_closed_targets_reject_the_entire_edit() {
    let (_root, mut app) = fixture(true);
    actions(&mut app);
    app.documents[0].insert("new", false);
    let active_before = app.doc().text.to_string();
    choose(&mut app, "Multiple dirty buffers");
    assert!(app.message.contains("changed since"));
    assert_eq!(app.doc().text.to_string(), active_before);
    assert_eq!(app.documents[0].text.to_string(), "newbad\r\n");
    actions(&mut app);
    app.documents.remove(0);
    app.active = 0;
    app.sync_pane();
    choose(&mut app, "Multiple dirty buffers");
    assert!(app.message.contains("closed or replaced"));
    assert_eq!(app.doc().text.to_string(), active_before);
}
#[test]
fn stale_reply_resolve_and_command_callbacks_do_not_change_newer_editor_context() {
    let (_root, mut app) = fixture(false);
    app.execute("editor.action.quickFix", Value::Null);
    app.doc_mut().move_to(0, false);
    until(&mut app, |app| app.message.contains("context changed"));
    assert!(app.modal.is_none());
    select_word(&mut app);
    actions(&mut app);
    choose(&mut app, "Resolve refactor");
    app.doc_mut().insert("keep", false);
    until(&mut app, |app| app.message.contains("Document changed"));
    assert_eq!(app.doc().text.to_string(), "keep\r\n");
    app.execute("undo", Value::Null);
    select_word(&mut app);
    actions(&mut app);
    choose(&mut app, "Execute command");
    app.doc_mut().insert("retain", false);
    until(&mut app, |app| app.message == "Fixture command rejected");
    assert_eq!(app.doc().text.to_string(), "retain\r\n");
    app.execute("undo", Value::Null);
    select_word(&mut app);
    app.execute("editor.action.quickFix", Value::Null);
    app.execute("workbench.action.closeAllEditors", Value::Null);
    until(&mut app, |app| app.message.contains("Document changed"));
    assert!(app.documents.is_empty());
    app.execute("editor.action.quickFix", Value::Null);
    assert!(app.documents.is_empty());
}
#[test]
fn commands_reject_new_modal_context_and_unsupported_closed_files_without_creating_them() {
    let (_root, mut app) = fixture(false);
    actions(&mut app);
    choose(&mut app, "Atomic invalid target");
    assert!(app.message.contains("outside the synchronized"));
    actions(&mut app);
    choose(&mut app, "Execute command");
    app.execute("workbench.action.showCommands", Value::Null);
    until(&mut app, |app| app.message == "Fixture command rejected");
    assert!(app.prompt.is_some());
    assert_eq!(app.doc().text.to_string(), "bad\r\n");
}
#[test]
fn unsolicited_server_edits_are_rejected_without_changing_buffer_or_disk() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.rs");
    std::fs::write(&path, "bad\r\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let server =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/code_action_server.py");
    app.lsp = Some(
        Client::start(
            "python3",
            &[
                server.to_string_lossy().into_owned(),
                "--unsolicited".into(),
            ],
            root.path(),
            "rust".into(),
        )
        .unwrap(),
    );
    until(&mut app, |app| app.message == "Fixture command rejected");
    assert_eq!(app.doc().text.to_string(), "bad\r\n");
    assert!(!app.doc().dirty());
    assert_eq!(std::fs::read(path).unwrap(), b"bad\r\n");
}

#[test]
fn text_edit_budgets_reject_without_partial_mutation_and_diagnostics_are_bounded() {
    let (_root, mut app) = fixture(false);
    for (edits, expected) in [
        (
            vec![
                serde_json::json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"x"});
                4097
            ],
            "4096 text edits",
        ),
        (
            vec![
                serde_json::json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"x".repeat(4*1024*1024+1)}),
            ],
            "4 MiB replacement",
        ),
    ] {
        actions(&mut app);
        let uri = vscli::lsp::file_uri(app.doc().path.as_ref().unwrap()).unwrap();
        if let Some(Modal::Language { items, .. }) = &mut app.modal
            && let vscli::app::LanguageAction::CodeAction { item, .. } = &mut items[0].action
        {
            item["edit"] = serde_json::json!({"changes":{uri:edits}});
        }
        choose(&mut app, "Fix selected text");
        assert!(app.message.contains(expected), "{}", app.message);
        assert_eq!(app.doc().text.to_string(), "bad\r\n");
    }
    let diagnostic = app.current_diagnostics()[0].clone();
    let doc = app.doc();
    let key = doc.path.clone().unwrap();
    let id = doc.id;
    let revision = doc.revision;
    app.diagnostics
        .insert(key, (id, revision, vec![diagnostic; 129]));
    app.execute("editor.action.quickFix", Value::Null);
    assert!(app.message.contains("128 diagnostics"));
}

#[test]
#[ignore = "requires clangd installed; run explicitly for C++ quick-fix qualification"]
fn real_clangd_cpp_quick_fix_is_native_undoable_and_saved_only_on_request() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    let original = "int main() {\n  int value = 1\n  return value;\n}\n";
    std::fs::write(&path, original).unwrap();
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
    until(&mut app, |app| !app.current_diagnostics().is_empty());
    let diagnostic = app
        .current_diagnostics()
        .iter()
        .find(|item| item.message.to_lowercase().contains("expected ';'"))
        .unwrap_or_else(|| {
            panic!(
                "Expected missing semicolon diagnostic: {:?}",
                app.current_diagnostics()
            )
        })
        .clone();
    let cursor = vscli::lsp::offset(app.doc(), diagnostic.range.start).unwrap();
    app.doc_mut().move_to(cursor, false);
    actions(&mut app);
    let Some(Modal::Language {
        items, selected, ..
    }) = &mut app.modal
    else {
        unreachable!()
    };
    *selected = items
        .iter()
        .position(|item| item.label.contains(';'))
        .unwrap_or_else(|| {
            panic!(
                "No semicolon action: {:?}",
                items.iter().map(|item| &item.label).collect::<Vec<_>>()
            )
        });
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    until(&mut app, |app| {
        app.doc().text.to_string().contains("value = 1;")
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("redo", Value::Null);
    app.execute("workbench.action.files.save", Value::Null);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("value = 1;")
    );
}
