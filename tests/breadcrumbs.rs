//! Public Breadcrumbs journeys; optional symbol work is separate from native file navigation.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Focus},
    breadcrumbs::ElementKind,
    keys::Profile,
    lsp::Client,
};

const CASES: &str = include_str!("vscode-reference/breadcrumbs-cases.json");
fn original() -> String {
    serde_json::from_str::<Value>(CASES).unwrap()[0]["text"]
        .as_str()
        .unwrap()
        .into()
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
fn records(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("peer.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
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
    let mut args = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/breadcrumbs_server.py")
            .to_string_lossy()
            .into_owned(),
        root.join("peer.jsonl").to_string_lossy().into_owned(),
        root.join("release").to_string_lossy().into_owned(),
    ];
    if hold {
        args.push("--hold-first".into());
    }
    if flat {
        args.push("--flat".into());
    }
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &args,
            &root,
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, "native peer ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    (temporary, app, source)
}
fn ready(app: &mut App) {
    until(app, "current symbol breadcrumbs", |app| {
        !app.breadcrumbs_view().updating
            && app.breadcrumbs_view().elements.iter().any(|element| {
                matches!(element.kind, ElementKind::Symbol | ElementKind::RootSymbols)
            })
    });
}
fn goto(app: &mut App, line: &str) {
    app.execute("workbench.action.gotoLine", Value::Null);
    assert!(app.prompt.is_some());
    app.event(Event::Paste(line.into()));
    key(app, KeyCode::Enter);
    assert!(app.prompt.is_none());
}
fn scalar(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap()].chars().count()
}

#[test]
fn file_trail_without_lsp_or_node_and_blank_window_focus_preserve_document_state() {
    let temporary = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temporary.path()).unwrap();
    let source = root.join("src/猫/main.cpp");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, original()).unwrap();
    let mut app = App::new(root, Profile::Linux);
    assert!(!app.breadcrumbs_view().possible);
    app.execute("breadcrumbs.focus", Value::Null);
    assert!(app.active_document().is_none());
    app.open(&source).unwrap();
    until(&mut app, "native file trail", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_ref() == Some(&source))
            && app.breadcrumbs_view().visible
    });
    assert!(app.lsp.is_none());
    assert_eq!(
        app.breadcrumbs_view()
            .elements
            .iter()
            .map(|element| element.label.as_str())
            .collect::<Vec<_>>(),
        ["src", "猫", "main.cpp"]
    );
    let identity = app.doc().id;
    let selections = app.doc().selections();
    app.execute("breadcrumbs.focus", Value::Null);
    assert!(app.focus == Focus::Breadcrumbs);
    assert_eq!(app.breadcrumbs_view().focused, Some(2));
    app.execute("breadcrumbs.selectFocused", Value::Null);
    assert!(app.breadcrumbs_view().picker.is_none());
    app.execute("breadcrumbs.selectEditor", Value::Null);
    assert!(app.focus == Focus::Editor);
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(app.doc().text.to_string(), original());
    assert!(!app.doc().dirty());
    assert_eq!(std::fs::read(source).unwrap(), original().as_bytes());
}

#[test]
fn hierarchical_trail_picker_reveal_history_and_unicode_crlf_save_undo_are_atomic() {
    let (_temporary, mut app, source) = fixture(false, false);
    ready(&mut app);
    let identity = app.doc().id;
    goto(&mut app, "6");
    assert_eq!(
        app.breadcrumbs_view()
            .elements
            .iter()
            .map(|e| e.label.as_str())
            .collect::<Vec<_>>(),
        ["main.cpp", "demo", "Widget", "render(int value)"]
    );
    app.execute("breadcrumbs.focusAndSelect", Value::Null);
    assert!(app.focus == Focus::Breadcrumbs);
    let picker = app.breadcrumbs_view().picker.unwrap();
    assert_eq!(
        picker
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        ["render(int value)", "reset()"]
    );
    assert_eq!(picker.selected, 0);
    let before = app.doc().selections();
    key(&mut app, KeyCode::Down);
    assert_eq!(app.breadcrumbs_view().picker.unwrap().selected, 1);
    assert_eq!(app.doc().selections(), before);
    key(&mut app, KeyCode::Enter);
    assert!(app.focus == Focus::Editor);
    let reset = scalar(&original(), "reset()");
    assert_eq!(app.doc().cursor, reset);
    assert_eq!(app.doc().anchor, None);
    assert_eq!(app.doc().id, identity);
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    app.execute("workbench.action.navigateBack", Value::Null);
    assert_eq!(app.doc().cursor, before[0].cursor);
    app.execute("workbench.action.navigateForward", Value::Null);
    assert_eq!(app.doc().cursor, reset);
    key(&mut app, KeyCode::Char('猫'));
    let edited = app.doc().text.to_string();
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "Unicode CRLF save", |app| {
        !app.saves_pending() && std::fs::read(&source).unwrap() == edited.as_bytes()
    });
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original());
    assert_eq!(app.doc().id, identity);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "Undo save", |app| {
        !app.saves_pending() && std::fs::read(&source).unwrap() == original().as_bytes()
    });
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), edited);
    assert_eq!(app.doc().id, identity);
}

#[test]
fn flat_symbol_information_never_fabricates_container_crumbs() {
    let (_temporary, mut app, source) = fixture(false, true);
    ready(&mut app);
    app.doc_mut()
        .move_to(scalar(&original(), "render(int value)"), false);
    app.poll();
    assert_eq!(
        app.breadcrumbs_view()
            .elements
            .iter()
            .map(|e| e.label.as_str())
            .collect::<Vec<_>>(),
        ["main.cpp", "render(int value)"]
    );
    app.execute("breadcrumbs.focusAndSelect", Value::Null);
    assert_eq!(
        app.breadcrumbs_view()
            .picker
            .unwrap()
            .rows
            .iter()
            .map(|r| r.label.as_str())
            .collect::<Vec<_>>(),
        ["render(int value)", "main()"]
    );
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().cursor, scalar(&original(), "main()"));
    assert_eq!(app.doc().anchor, None);
    assert_eq!(std::fs::read(source).unwrap(), original().as_bytes());
}

#[test]
fn shared_outline_demand_keeps_one_actual_request_and_edit_undo_rejects_old_tree() {
    let (temporary, mut app, source) = fixture(true, false);
    let root = std::fs::canonicalize(temporary.path()).unwrap();
    let identity = app.doc().id;
    until(&mut app, "held breadcrumb request", |_| {
        records(&root).iter().any(|r| r["event"] == "request")
    });
    app.execute("outline.focus", Value::Null);
    app.execute("breadcrumbs.focus", Value::Null);
    assert_eq!(
        records(&root)
            .iter()
            .filter(|r| r["event"] == "request")
            .count(),
        1
    );
    assert!(app.breadcrumbs_view().updating);
    app.execute("breadcrumbs.selectEditor", Value::Null);
    key(&mut app, KeyCode::Char('é'));
    app.execute("undo", json!(null));
    assert_eq!(app.doc().text.to_string(), original());
    assert!(
        app.breadcrumbs_view()
            .elements
            .iter()
            .all(|e| !matches!(e.kind, ElementKind::Symbol | ElementKind::RootSymbols))
    );
    std::fs::write(root.join("release"), b"release old actual work").unwrap();
    ready(&mut app);
    let requests = records(&root)
        .into_iter()
        .filter(|r| r["event"] == "request")
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .map(|r| r["pending"].as_u64().unwrap())
            .max(),
        Some(1)
    );
    assert!(requests[1]["version"].as_u64() > requests[0]["version"].as_u64());
    assert_eq!(app.doc().cursor, 0);
    assert_eq!(app.doc().id, identity);
    assert_eq!(std::fs::read(source).unwrap(), original().as_bytes());
}

#[test]
fn reveal_collapses_only_active_selection_cohort_and_public_pane_aba_retires_picker() {
    use vscli::document::Selection;
    let (_temporary, mut app, source) = fixture(false, false);
    ready(&mut app);
    key(&mut app, KeyCode::Char('猫'));
    let dirty = app.doc().text.to_string();
    ready(&mut app);
    goto(&mut app, "6");
    let identity = app.doc().id;
    let other_selections = app.doc().selections();
    app.execute("workbench.action.splitEditorRight", Value::Null);
    assert_eq!(app.panes.len(), 2);
    let render = scalar(&dirty, "render(int value)");
    app.doc_mut().set_selections(vec![
        Selection {
            cursor: render + 2,
            anchor: Some(render + 5),
            desired_column: None,
        },
        Selection::caret(scalar(&dirty, "main()")),
    ]);
    app.poll();
    app.execute("breadcrumbs.focusAndSelect", Value::Null);
    assert!(app.breadcrumbs_view().picker.is_some());
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().selections(), vec![Selection::caret(render)]);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert!(app.doc().dirty());
    assert_eq!(app.doc().id, identity);
    app.focus_pane(0);
    assert_eq!(app.doc().selections(), other_selections);
    app.focus_pane(1);
    app.execute("breadcrumbs.focusAndSelect", Value::Null);
    assert!(app.breadcrumbs_view().picker.is_some());
    let selected = app.doc().selections();
    // No poll between A→B→A: accepted public navigation must itself retire
    // the old presentation; an identical final context cannot revive it.
    app.focus_pane(0);
    app.focus_pane(1);
    assert!(app.breadcrumbs_view().picker.is_none());
    app.execute("breadcrumbs.revealFocused", Value::Null);
    assert_eq!(app.doc().selections(), selected);
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original());
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().id, identity);
}

#[test]
#[ignore = "requires actual clangd executable explicitly set in VSCLI_CLANGD"]
fn actual_clangd_breadcrumbs_enclosing_trail_and_sibling_reveal_preserve_dirty_work() {
    use vscli::app::OutlineStatus;
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
    let end = app.doc().len();
    app.doc_mut().move_to(end, false);
    app.doc_mut().insert("// unsaved 猫", false);
    let dirty = app.doc().text.to_string();
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
    assert_eq!(
        app.outline_view().status,
        OutlineStatus::Hidden,
        "Breadcrumbs demand must work before Outline activation"
    );
    goto(&mut app, "6");
    let view = app.breadcrumbs_view();
    assert!(view.visible && !view.updating);
    let symbols = view
        .elements
        .iter()
        .filter(|element| element.kind == ElementKind::Symbol)
        .map(|element| element.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(symbols.len(), 3);
    assert_eq!(symbols[0], "demo");
    assert_eq!(symbols[1], "Widget");
    assert!(
        symbols[2].starts_with("render"),
        "Actual clangd method crumb: {symbols:?}"
    );
    app.execute("breadcrumbs.focusAndSelect", Value::Null);
    let picker = app.breadcrumbs_view().picker.unwrap();
    assert_eq!(picker.selected, 0);
    assert_eq!(picker.rows.len(), 2);
    assert!(picker.rows[0].label.starts_with("render"));
    assert!(picker.rows[1].label.starts_with("reset"));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.focus == Focus::Editor);
    assert_eq!(app.doc().cursor, scalar(&dirty, "reset()"));
    assert_eq!(app.doc().anchor, None);
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read(&source).unwrap(), original().as_bytes());
    // Enabling the second consumer must reuse the already-current publication,
    // rather than clear it and wait for an independent document-symbol producer.
    app.execute("outline.focus", Value::Null);
    assert_eq!(app.outline_view().status, OutlineStatus::Ready);
    assert!(app.outline_view().actionable);
    assert!(
        app.outline_view()
            .tree
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.name.starts_with("reset"))
    );
    key(&mut app, KeyCode::Esc);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "actual clangd dirty UnicodeCRLF save", |_| {
        std::fs::read(&source).unwrap() == dirty.as_bytes()
    });
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original());
    assert_eq!(app.doc().id, identity);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "actual clangd Undo save", |app| {
        !app.saves_pending() && std::fs::read(&source).unwrap() == original().as_bytes()
    });
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().id, identity);
}
