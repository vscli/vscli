use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile, lsp::Client};
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settle(app: &mut App) {
    let end = Instant::now() + Duration::from_millis(260);
    while Instant::now() < end {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn fixture(invalid: bool) -> (tempfile::TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    std::fs::write(&path, "sum(1, 2)\r\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    app.doc_mut().move_to(7, false);
    let mut args = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/signature_server.py")
            .to_string_lossy()
            .into_owned(),
    ];
    if invalid {
        args.push("--invalid".into());
    }
    app.lsp = Some(Client::start("python3", &args, root.path(), "cpp".into()).unwrap());
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    (root, app)
}
fn hints(app: &mut App) {
    app.execute("editor.action.triggerParameterHints", Value::Null);
}
#[test]
fn explicit_hints_are_nonmodal_themed_and_never_change_shared_text_or_save() {
    let (root, mut app) = fixture(false);
    let id = app.doc().id;
    app.execute("workbench.action.splitEditorRight", Value::Null);
    let before = app.doc().text.to_string();
    hints(&mut app);
    until(&mut app, |a| a.signature_help().is_some());
    let hint = app.signature_help().unwrap();
    assert_eq!(&hint.label[hint.parameter.clone().unwrap()], "int count");
    assert!(hint.documentation.contains("Number of values"));
    assert!(app.modal.is_none());
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| vscli::ui::draw(f, &mut app)).unwrap();
    let screen = terminal.backend().buffer();
    let text: String = screen.content.iter().map(|c| c.symbol()).collect();
    assert!(text.contains("Parameter Hints 1/1"));
    assert!(text.contains("int count"));
    assert!(screen.content.iter().any(|c| c.symbol() == "c"
        && c.fg == app.theme.colors.accent
        && c.modifier.contains(ratatui::style::Modifier::BOLD)));
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), before);
    assert!(!app.doc().dirty());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        before.as_bytes()
    );
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(app.signature_help().is_none());
    assert_eq!(app.doc().text.to_string(), before);
}
#[test]
fn cancellations_late_replies_and_newer_requests_keep_context_and_native_work() {
    let (_root, mut app) = fixture(false);
    hints(&mut app);
    hints(&mut app);
    until(&mut app, |a| a.signature_help().is_some());
    assert!(app.signature_help().unwrap().label.starts_with("sum2"));
    settle(&mut app);
    assert!(app.signature_help().unwrap().label.starts_with("sum2"));
    hints(&mut app);
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    settle(&mut app);
    assert!(app.signature_help().is_none());
    hints(&mut app);
    app.doc_mut().insert("keep", false);
    settle(&mut app);
    assert!(app.signature_help().is_none());
    assert!(app.doc().text.to_string().contains("keep"));
    app.execute("undo", Value::Null);
    hints(&mut app);
    app.execute("workbench.action.showCommands", Value::Null);
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    settle(&mut app);
    assert!(app.signature_help().is_none());
    hints(&mut app);
    app.execute("workbench.action.splitEditorRight", Value::Null);
    settle(&mut app);
    assert!(app.signature_help().is_none());
    hints(&mut app);
    app.execute("workbench.action.closeAllEditors", Value::Null);
    settle(&mut app);
    assert!(app.documents.is_empty());
    hints(&mut app);
    assert!(app.documents.is_empty());
    assert!(app.signature_help().is_none());
}
#[test]
fn malformed_response_and_unsupported_servers_leave_editor_usable() {
    let (_root, mut app) = fixture(true);
    hints(&mut app);
    until(&mut app, |a| a.message.contains("outside the signature"));
    assert!(app.signature_help().is_none());
    assert!(!app.doc().dirty());
    app.lsp.as_mut().unwrap().language = "rust".into();
    assert_eq!(
        app.context()["editorHasSignatureHelpProvider"],
        Value::Bool(false)
    );
    hints(&mut app);
    assert!(app.message.contains("does not provide"));
    app.lsp.as_mut().unwrap().language = "cpp".into();
    app.lsp.as_mut().unwrap().capabilities = Value::Null;
    hints(&mut app);
    assert!(app.message.contains("does not provide"));
    app.execute("type", serde_json::json!({"text":"ok"}));
    assert!(app.doc().text.to_string().contains("ok"));
}
#[test]
#[ignore = "requires installed clangd; run explicitly for real C++ signature qualification"]
fn real_clangd_cpp_signature_has_active_parameter_without_mutating_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    let text = "int sum(int left, int right);\nint main() { return sum(1, 2); }\n";
    std::fs::write(&path, text).unwrap();
    std::fs::write(root.path().join("compile_flags.txt"), "-std=c++17\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    app.doc_mut().move_to(text.find("1, 2").unwrap() + 3, false);
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
    hints(&mut app);
    until(&mut app, |a| a.signature_help().is_some());
    let hint = app.signature_help().unwrap();
    assert!(hint.label.contains("sum"));
    assert!(&hint.label[hint.parameter.clone().unwrap()].contains("right"));
    assert_eq!(app.doc().text.to_string(), text);
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert!(!app.doc().dirty());
}
