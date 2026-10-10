//! Public Outline journeys: native source, keyboard ownership and document integrity.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Focus, OutlineStatus},
    keys::Profile,
    lsp::Client,
};

const CASES: &str = include_str!("vscode-reference/outline-cases.json");
fn original() -> String {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    cases[0]["text"].as_str().unwrap().into()
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn records(root: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("peer.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn fixture(hold: bool, flat: bool) -> (tempfile::TempDir, App, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temporary.path()).unwrap();
    let source = root.join("main.cpp");
    std::fs::write(&source, original()).unwrap();
    let mut app = App::new(root.clone(), Profile::Linux);
    app.open(&source).unwrap();
    until(&mut app, "fixture open", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_ref() == Some(&source))
    });
    let mut arguments = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/outline_server.py")
            .to_string_lossy()
            .into_owned(),
        root.join("peer.jsonl").to_string_lossy().into_owned(),
        root.join("release").to_string_lossy().into_owned(),
    ];
    if hold {
        arguments.push("--hold-first".into());
    }
    if flat {
        arguments.push("--flat".into());
    }
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &arguments,
            &root,
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, "native fixture ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    (temporary, app, source)
}
fn ready(app: &mut App) {
    app.execute("outline.focus", Value::Null);
    until(app, "current actionable Outline", |app| {
        let view = app.outline_view();
        view.status == OutlineStatus::Ready && view.actionable
    });
}
#[test]
fn hierarchy_keyboard_reveal_follow_cursor_and_history_preserve_unicode_crlf_undo() {
    let (_temporary, mut app, source) = fixture(false, false);
    let identity = app.doc().id;
    ready(&mut app);
    assert!(app.focus == Focus::Outline);
    {
        let view = app.outline_view();
        let tree = view.tree.unwrap();
        assert!(tree.hierarchical);
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["demo", "Widget", "render(int value)", "reset()", "main()"]
        );
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| node.parent)
                .collect::<Vec<_>>(),
            [None, Some(0), Some(1), Some(1), None]
        );
        assert_eq!(view.visible, &[0, 1, 2, 3, 4]);
    }
    let text = app.doc().text.to_string();
    let selections = app.doc().selections();
    app.execute("outline.collapse", Value::Null);
    assert_eq!(app.outline_view().visible, &[0, 4]);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.outline_view().visible, &[0, 1, 4]);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.outline_view().selected, Some(1));
    key(&mut app, KeyCode::Right);
    assert_eq!(app.outline_view().visible, &[0, 1, 2, 3, 4]);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.outline_view().selected, Some(2));
    assert_eq!(app.doc().text.to_string(), text);
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(app.doc().id, identity);
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    key(&mut app, KeyCode::Enter);
    assert!(app.focus == Focus::Editor);
    let render = original().find("render(int").unwrap();
    let render = original()[..render].chars().count();
    assert_eq!(app.doc().cursor, render);
    assert_eq!(
        app.doc().anchor,
        None,
        "Ordinary Outline selection collapses at identifier start"
    );
    assert_eq!(app.outline_view().active, Some(2));
    app.execute("workbench.action.navigateBack", Value::Null);
    assert_eq!(app.doc().cursor, 0);
    app.execute("workbench.action.navigateForward", Value::Null);
    assert_eq!(app.doc().cursor, render);
    key(&mut app, KeyCode::Char('猫'));
    assert!(app.doc().dirty());
    assert_eq!(app.doc().id, identity);
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    let mut edited = original();
    let byte = edited.char_indices().nth(render).unwrap().0;
    edited.insert(byte, '猫');
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "explicit Unicode CRLF save", |_| {
        std::fs::read(&source).unwrap() == edited.as_bytes()
    });
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original());
    assert_eq!(app.doc().id, identity);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "Undo explicit save", |_| {
        std::fs::read(&source).unwrap() == original().as_bytes()
    });
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), edited);
    assert_eq!(app.doc().id, identity);
}

#[test]
fn pending_outline_is_nonactionable_and_late_reply_cannot_reveal_after_edit_undo() {
    let (temporary, mut app, source) = fixture(true, false);
    let root = std::fs::canonicalize(temporary.path()).unwrap();
    let identity = app.doc().id;
    app.execute("outline.focus", Value::Null);
    until(&mut app, "held actual symbol request", |_| {
        records(&root).iter().any(|row| row["event"] == "request")
    });
    assert!(!app.outline_view().actionable);
    let before = app.doc().selections();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().selections(), before);
    assert_eq!(app.doc().text.to_string(), original());
    key(&mut app, KeyCode::Esc);
    assert!(app.focus == Focus::Editor);
    key(&mut app, KeyCode::Char('é'));
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original());
    app.execute("outline.focus", Value::Null);
    assert!(!app.outline_view().actionable);
    assert_eq!(
        records(&root)
            .iter()
            .filter(|row| row["event"] == "request")
            .count(),
        1
    );
    std::fs::write(root.join("release"), b"release canceled actual work").unwrap();
    until(
        &mut app,
        "fresh epoch symbol tree after actual release",
        |app| {
            app.outline_view().actionable
                && records(&root)
                    .iter()
                    .filter(|row| row["event"] == "request")
                    .count()
                    == 2
        },
    );
    let requests = records(&root)
        .into_iter()
        .filter(|row| row["event"] == "request")
        .collect::<Vec<_>>();
    assert_eq!(
        requests
            .iter()
            .map(|row| row["pending"].as_u64().unwrap())
            .max(),
        Some(1)
    );
    assert!(requests[1]["version"].as_u64() > requests[0]["version"].as_u64());
    assert_eq!(
        app.doc().cursor,
        0,
        "Late same-byte old version must not navigate"
    );
    assert_eq!(app.doc().id, identity);
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
}

#[test]
fn flat_provider_nodes_remain_independent_actionable_leaf_targets() {
    let (_temporary, mut app, source) = fixture(false, true);
    ready(&mut app);
    let view = app.outline_view();
    let tree = view.tree.unwrap();
    assert!(!tree.hierarchical);
    assert_eq!(tree.nodes.len(), 2);
    assert!(tree.nodes.iter().all(|node| node.parent.is_none()));
    assert_eq!(view.visible, &[0, 1]);
    app.execute("outline.collapse", Value::Null);
    assert_eq!(app.outline_view().visible, &[0, 1]);
    key(&mut app, KeyCode::Enter);
    let byte = original().find("render(int").unwrap();
    assert_eq!(app.doc().cursor, original()[..byte].chars().count());
    assert_eq!(app.doc().anchor, None);
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
}

#[test]
fn follow_cursor_toggle_preserves_explicit_tree_selection_and_readonly_document_state() {
    let (_temporary, mut app, source) = fixture(false, false);
    ready(&mut app);
    assert!(app.outline_view().follow_cursor);
    app.execute("outline.followCursor", Value::Null);
    assert!(!app.outline_view().follow_cursor);
    key(&mut app, KeyCode::End);
    assert_eq!(app.outline_view().selected, Some(4));
    key(&mut app, KeyCode::Esc);
    app.execute("workbench.action.gotoLine", Value::Null);
    assert!(app.prompt.is_some());
    app.event(Event::Paste("5".into()));
    key(&mut app, KeyCode::Enter);
    assert!(app.prompt.is_none());
    until(&mut app, "render enclosing highlight", |app| {
        app.outline_view().active == Some(2)
    });
    assert_eq!(app.outline_view().selected, Some(4));
    let selections = app.doc().selections();
    let identity = app.doc().id;
    app.execute("outline.collapse", Value::Null);
    assert_eq!(app.outline_view().visible, &[0, 4]);
    app.execute("outline.followCursor", Value::Null);
    assert!(app.outline_view().follow_cursor);
    assert_eq!(app.outline_view().selected, Some(2));
    assert_eq!(app.outline_view().visible, &[0, 1, 2, 3, 4]);
    assert_eq!(app.doc().text.to_string(), original());
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(app.doc().id, identity);
    assert!(!app.doc().dirty());
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
}

#[test]
#[ignore = "requires actual clangd executable explicitly set in VSCLI_CLANGD"]
fn actual_clangd_namespace_class_methods_reveal_exact_utf16_and_preserve_dirty_work() {
    let program = std::env::var("VSCLI_CLANGD").expect("Set actual clangd executable");
    let temporary = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temporary.path()).unwrap();
    let source = root.join("main.cpp");
    std::fs::write(&source, original()).unwrap();
    std::fs::write(root.join("compile_flags.txt"), "-std=c++17\n").unwrap();
    let mut app = App::new(root.clone(), Profile::Linux);
    app.open(&source).unwrap();
    until(&mut app, "actual C++ source open", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_ref() == Some(&source))
    });
    let identity = app.doc().id;
    app.lsp = Some(
        Client::start(
            &program,
            &[
                "--background-index=false".into(),
                "--clang-tidy=false".into(),
            ],
            &root,
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, "actual clangd ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    ready(&mut app);
    let tree = app.outline_view().tree.unwrap();
    let namespace = tree
        .nodes
        .iter()
        .position(|node| node.name == "demo")
        .expect("Actual clangd namespace");
    let class = tree
        .nodes
        .iter()
        .position(|node| node.name == "Widget")
        .expect("Actual clangd class");
    let render = tree
        .nodes
        .iter()
        .position(|node| node.name.starts_with("render"))
        .expect("Actual clangd method");
    assert_eq!(tree.nodes[class].parent, Some(namespace));
    assert_eq!(tree.nodes[render].parent, Some(class));
    assert_eq!(tree.nodes[render].selection_range.start.line, 4);
    assert_eq!(tree.nodes[render].selection_range.start.character, 17);
    // Follow the actual displayed hierarchy with physical keys, never mutate
    // controller selection or bypass range validation.
    app.execute("outline.expand", Value::Null);
    key(&mut app, KeyCode::Home);
    let row = app
        .outline_view()
        .visible
        .iter()
        .position(|index| *index == render)
        .unwrap();
    for _ in 0..row {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(app.outline_view().selected, Some(render));
    key(&mut app, KeyCode::Enter);
    let byte = original().find("render(int").unwrap();
    assert_eq!(app.doc().cursor, original()[..byte].chars().count());
    assert_eq!(app.doc().anchor, None);
    key(&mut app, KeyCode::Char('猫'));
    assert_eq!(app.doc().id, identity);
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    app.execute("undo", json!(null));
    assert_eq!(app.doc().text.to_string(), original());
    assert_eq!(app.doc().id, identity);
}
