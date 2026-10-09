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
    let start = Instant::now();
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "Timed out: {}\n{}",
            app.message,
            app.lsp.as_ref().map_or_else(
                || "Language server disconnected".into(),
                Client::debug_summary
            )
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn native_lsp_lifecycle_completion_formatting_and_stale_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("example.rs");
    std::fs::write(&path, "ans\n").unwrap();
    let mut app = App::new(dir.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_server.py");
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[fixture.to_string_lossy().into_owned()],
            dir.path(),
            "rust".into(),
        )
        .unwrap(),
    );
    until(&mut app, |a| !a.current_diagnostics().is_empty());
    assert_eq!(app.current_diagnostics()[0].message, "Fixture diagnostic");
    app.doc_mut().move_to(3, false);
    app.execute("editor.action.showHover", Value::Null);
    until(&mut app, |a| matches!(a.modal, Some(Modal::Text { .. })));
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(&mut app, |a| a.suggestion_model().is_some());
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert_eq!(app.doc().text.to_string(), "answer\n");
    app.execute("editor.action.formatDocument", Value::Null);
    app.doc_mut().insert("!", false);
    until(&mut app, |a| {
        a.message.contains("Document changed since this request")
    });
    assert_eq!(app.doc().text.to_string(), "answer!\n");
    app.execute("editor.action.formatDocument", Value::Null);
    until(&mut app, |a| a.doc().text == "formatted\n");
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "answer!\n");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "ans\n");
    app.execute("editor.action.showHover", Value::Null);
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('d'),
        KeyModifiers::NONE,
    )));
    assert!(app.documents.is_empty());
    until(&mut app, |a| {
        a.message.contains("Document changed since this request")
    });
    assert!(app.documents.is_empty());
    assert!(app.current_diagnostics().is_empty());
    assert!(app.modal.is_none());
}

#[test]
#[ignore = "requires clangd installed; run explicitly for real-server qualification"]
fn real_clangd_diagnostics_hover_and_formatting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("example.c");
    std::fs::write(
        &path,
        "int add(int a,int b){return a+b;}\nint main(){return add(1,missing);}\n",
    )
    .unwrap();
    let mut app = App::new(dir.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    app.lsp = Some(
        Client::start(
            "clangd",
            &["--background-index=false".into(), "-j=2".into()],
            dir.path(),
            "c".into(),
        )
        .unwrap(),
    );
    until(&mut app, |a| !a.current_diagnostics().is_empty());
    assert!(
        app.current_diagnostics()
            .iter()
            .any(|d| d.message.contains("missing"))
    );
    app.doc_mut().move_to(5, false);
    app.execute("editor.action.showHover", Value::Null);
    until(&mut app, |a| matches!(a.modal, Some(Modal::Text { .. })));
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    app.execute("editor.action.formatDocument", Value::Null);
    until(&mut app, |a| a.message.starts_with("Applied "));
    assert!(app.doc().text.to_string().contains("int add(int a, int b)"));
    assert!(app.doc().dirty());
}
