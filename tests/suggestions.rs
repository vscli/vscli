use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile, lsp::Client};

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn until(app: &mut App, condition: impl Fn(&App) -> bool) {
    let started = Instant::now();
    loop {
        app.poll();
        if condition(app) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn settle(app: &mut App) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(200) {
        app.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn requests(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn app(root: &Path, text: &str) -> App {
    let path = root.join("input.rs");
    std::fs::write(&path, text).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.open(&path).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/suggestions_server.py");
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                fixture.to_string_lossy().into(),
                root.to_string_lossy().into(),
            ],
            root,
            "rust".into(),
        )
        .unwrap(),
    );
    until(&mut app, |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    app
}

#[test]
fn automatic_typing_coalesces_and_tab_acceptance_preserves_crlf_unicode_identity_and_undo() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path(), " 🙂\r\n");
    let id = app.doc().id;
    for character in "ans".chars() {
        key(&mut app, KeyCode::Char(character));
    }
    assert!(requests(root.path()).is_empty());
    until(&mut app, App::suggestion_acceptable);
    assert!(app.modal.is_none());
    assert_eq!(app.suggestion_model().unwrap().len(), 1);
    assert_eq!(requests(root.path()).len(), 1);
    assert_eq!(requests(root.path())[0]["position"]["character"], 3);
    assert_eq!(requests(root.path())[0]["context"]["triggerKind"], 1);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "answer 🙂\r\n");
    app.doc_mut().save().unwrap();
    assert_eq!(
        std::fs::read(root.path().join("input.rs")).unwrap(),
        "answer 🙂\r\n".as_bytes()
    );
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "ans 🙂\r\n");
    assert!(app.suggestion_model().is_none());
}

#[test]
fn pending_filter_is_nonmodal_but_old_edit_cannot_be_accepted_and_tab_still_indents() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path(), "an 🙂\r\n");
    app.doc_mut().move_to(2, false);
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(&mut app, App::suggestion_acceptable);
    assert_eq!(app.suggestion_model().unwrap().len(), 2);
    std::fs::write(root.path().join("hold"), "").unwrap();
    key(&mut app, KeyCode::Char('s'));
    assert_eq!(app.suggestion_model().unwrap().len(), 1);
    assert!(!app.suggestion_acceptable());
    until(&mut app, |_| requests(root.path()).len() == 2);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.doc().cursor, 3);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.doc().text.to_string(), "ans  🙂\r\n");
    std::fs::remove_file(root.path().join("hold")).unwrap();
    settle(&mut app);
    assert!(app.suggestion_model().is_none());
    assert_eq!(app.doc().text.to_string(), "ans  🙂\r\n");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "ans 🙂\r\n");
}

#[test]
fn unsynced_edit_undo_and_focus_round_trip_reject_held_replies() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path(), "ans 🙂\r\n");
    app.doc_mut().move_to(3, false);
    std::fs::write(root.path().join("hold"), "").unwrap();
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(&mut app, |_| requests(root.path()).len() == 1);
    let revision = app.doc().revision;
    app.doc_mut().insert("x", false);
    app.doc_mut().undo();
    assert_eq!(app.doc().revision, revision);
    std::fs::remove_file(root.path().join("hold")).unwrap();
    settle(&mut app);
    assert!(app.suggestion_model().is_none());
    assert_eq!(app.doc().text.to_string(), "ans 🙂\r\n");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), "ansx 🙂\r\n");
    app.doc_mut().undo();
    std::fs::write(root.path().join("hold"), "").unwrap();
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(&mut app, |_| requests(root.path()).len() == 2);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Right);
    std::fs::remove_file(root.path().join("hold")).unwrap();
    settle(&mut app);
    assert!(app.suggestion_model().is_none());
    assert_eq!(app.doc().cursor, 3);
}

#[test]
fn scoped_settings_disable_quick_suggestions_but_allow_trigger_characters_and_enter_off() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path(), "");
    let settings = root.path().join("settings.json");
    std::fs::write(&settings, r#"{"editor.quickSuggestions":false,"editor.quickSuggestionsDelay":0,"editor.acceptSuggestionOnEnter":"off"}"#).unwrap();
    app.settings = vscli::settings::Settings::load(&[settings]).unwrap();
    key(&mut app, KeyCode::Char('a'));
    settle(&mut app);
    assert!(requests(root.path()).is_empty());
    key(&mut app, KeyCode::Char('.'));
    until(&mut app, App::suggestion_acceptable);
    assert_eq!(
        requests(root.path())[0]["context"],
        json!({"triggerKind":2,"triggerCharacter":"."})
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().text.to_string(), "a.\n");
    assert!(app.suggestion_model().is_none());
    key(&mut app, KeyCode::Backspace);
    settle(&mut app);
    assert_eq!(
        requests(root.path()).len(),
        1,
        "Deleting back to a trigger character must not retrigger it"
    );
}

#[test]
fn user_conditional_bindings_override_popup_defaults_and_snippet_tab_keeps_precedence() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path(), "ans");
    app.doc_mut().move_to(3, false);
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(&mut app, App::suggestion_acceptable);
    let bindings = root.path().join("keys.json");
    std::fs::write(&bindings, r#"[{"key":"tab","command":"type","args":{"text":"CUSTOM"},"when":"suggestWidgetVisible"}]"#).unwrap();
    app.keymap.load(&bindings).unwrap();
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.doc().text.to_string(), "ansCUSTOM");
    app.keymap = vscli::keys::Keymap::new(Profile::Linux);
    app.doc_mut().select_all();
    app.execute(
        "editor.action.insertSnippet",
        json!({"snippet":"${1:ans} $2"}),
    );
    let before = app.doc().text.to_string();
    app.execute("editor.action.triggerSuggest", Value::Null);
    until(&mut app, App::suggestion_acceptable);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.doc().text.to_string(), before);
    assert!(app.doc().in_snippet());
    assert_eq!(app.doc().cursor, 4);
    assert!(app.suggestion_model().is_none());
}
