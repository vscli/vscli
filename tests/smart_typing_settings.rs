//! Actual App/settings-loader journeys; generated ownership is observed by edits.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile};

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn type_character(app: &mut App, character: char) {
    key(app, KeyCode::Char(character));
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
fn configuration(extra: Value) -> Value {
    let mut value = json!({"vscli.languageServer.enabled":false,"editor.tabSize":2,"editor.autoClosingBrackets":"languageDefined","editor.autoClosingOvertype":"auto","editor.autoClosingDelete":"auto","editor.autoIndent":"brackets"});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    value
}
fn fixture(original: &str) -> (tempfile::TempDir, App, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    let settings = root.path().join("settings.json");
    std::fs::write(&path, original).unwrap();
    std::fs::write(
        &settings,
        serde_json::to_vec(&configuration(json!({}))).unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.configure_settings(Some(settings.clone())).unwrap();
    app.open(&path).unwrap();
    (root, app, settings)
}
fn reload(app: &mut App, path: &PathBuf, configuration: Value) {
    std::fs::write(path, serde_json::to_vec(&configuration).unwrap()).unwrap();
    until(app, "actual asynchronous settings reload", |app| {
        app.settings.extension_layers().first() == configuration.as_object()
    });
    assert!(
        app.lsp.is_none(),
        "qualification stays native without language-service processes"
    );
}

#[test]
fn configured_indent_modes_drive_physical_enter_and_preserve_crlf_save_undo() {
    let original = "if (ok)\r\n    run();\r\n// 猫🙂\r\n";
    for (mode, indentation) in [
        ("none", ""),
        ("keep", "    "),
        ("brackets", "    "),
        ("advanced", ""),
        ("full", ""),
    ] {
        let (root, mut app, settings) = fixture(original);
        reload(
            &mut app,
            &settings,
            configuration(json!({"editor.tabSize":4,"[cpp]":{"editor.autoIndent":mode}})),
        );
        let id = app.doc().id;
        app.doc_mut().move_to(19, false);
        key(&mut app, KeyCode::Enter);
        let expected = format!("if (ok)\r\n    run();\r\n{indentation}\r\n// 猫🙂\r\n");
        assert_eq!(app.doc().text.to_string(), expected, "mode {mode}");
        assert_eq!(app.doc().cursor, 21 + indentation.len(), "mode {mode}");
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        let source = root.path().join("main.cpp");
        assert_eq!(std::fs::read(&source).unwrap(), original.as_bytes());
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, "save receipt", |app| !app.saves_pending());
        assert_eq!(std::fs::read(&source).unwrap(), expected.as_bytes());
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().cursor, 19);
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, "save receipt", |app| !app.saves_pending());
        assert_eq!(std::fs::read(&source).unwrap(), original.as_bytes());
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), expected);
        assert_eq!(app.doc().id, id);
        assert!(app.lsp.is_none());
    }
}

#[test]
fn actual_reload_a_b_a_and_undo_redo_never_revive_retired_generated_delimiters() {
    let original = "猫🙂 \r\n";
    let (root, mut app, settings) = fixture(original);
    let id = app.doc().id;
    let options_a = app.settings.typing("cpp");
    app.doc_mut().move_to(3, false);
    type_character(&mut app, '(');
    let paired = "猫🙂 ()\r\n";
    assert_eq!(app.doc().text.to_string(), paired);
    assert_eq!(app.doc().cursor, 4);
    reload(
        &mut app,
        &settings,
        configuration(json!({"editor.autoClosingBrackets":"never"})),
    );
    assert_ne!(app.settings.typing("cpp"), options_a);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    reload(&mut app, &settings, configuration(json!({})));
    assert_eq!(app.settings.typing("cpp"), options_a);
    app.doc_mut().move_to(4, false);
    type_character(&mut app, ')');
    let edited = "猫🙂 ())\r\n";
    assert_eq!(
        app.doc().text.to_string(),
        edited,
        "equal effective settings must not revive retired generation from Undo"
    );
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "save receipt", |app| !app.saves_pending());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        edited.as_bytes()
    );
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    assert!(app.doc().dirty());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        edited.as_bytes()
    );
}

#[test]
fn unrelated_reload_and_other_language_override_preserve_generated_skip_and_paired_delete() {
    let original = "猫🙂 \r\n";
    let (root, mut app, settings) = fixture(original);
    let options = app.settings.typing("cpp");
    app.doc_mut().move_to(3, false);
    type_character(&mut app, '(');
    let paired = app.doc().text.to_string();
    let revision = app.doc().revision;
    reload(
        &mut app,
        &settings,
        configuration(
            json!({"editor.lineNumbers":"off","editor.parameterHints.enabled":false,"[json]":{"editor.autoClosingBrackets":"never"}}),
        ),
    );
    assert_eq!(app.settings.typing("cpp"), options);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(
        app.doc().text.to_string(),
        original,
        "unchanged effective typing must preserve generated deletion ownership"
    );
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    assert_eq!(app.doc().cursor, 4);
    type_character(&mut app, ')');
    assert_eq!(app.doc().text.to_string(), paired);
    assert_eq!(
        app.doc().revision,
        revision,
        "an owned closer skips without a new text transaction"
    );
    assert_eq!(app.doc().cursor, 5);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
}

#[test]
fn invalid_settings_reload_preserves_effective_configuration_ownership_and_disk() {
    let original = "猫🙂 \r\n";
    let (root, mut app, settings) = fixture(original);
    let previous = app.settings.clone();
    app.doc_mut().move_to(3, false);
    type_character(&mut app, '(');
    std::fs::write(&settings, b"{broken").unwrap();
    until(&mut app, "invalid actual settings reload", |app| {
        app.message.contains("previous settings retained")
    });
    assert!(app.settings == previous);
    type_character(&mut app, ')');
    assert_eq!(app.doc().text.to_string(), "猫🙂 ()\r\n");
    assert_eq!(app.doc().cursor, 5);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
}

#[test]
fn physical_enter_and_explicit_line_break_use_effective_indent_but_distinct_caret_contracts() {
    let original = "  {}\r\n// 猫🙂\r\n";
    let (root, mut app, settings) = fixture(original);
    let id = app.doc().id;
    app.doc_mut().move_to(3, false);
    key(&mut app, KeyCode::Enter);
    let bracket_lines = "  {\r\n    \r\n  }\r\n// 猫🙂\r\n";
    assert_eq!(app.doc().text.to_string(), bracket_lines);
    assert_eq!(
        app.doc().cursor,
        9,
        "physical Enter moves into the indented body"
    );
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(app.doc().cursor, 3);
    app.execute("lineBreakInsert", Value::Null);
    assert_eq!(app.doc().text.to_string(), bracket_lines);
    assert_eq!(
        app.doc().cursor,
        3,
        "the original command ID keeps its caret before the line break"
    );
    app.execute("undo", Value::Null);
    reload(
        &mut app,
        &settings,
        configuration(json!({"editor.autoIndent":"keep"})),
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().text.to_string(), "  {\r\n  }\r\n// 猫🙂\r\n");
    assert_eq!(app.doc().cursor, 7);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(app.doc().id, id);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
}

#[test]
fn shared_view_ownership_and_save_as_profile_change_preserve_identity_history_and_disk() {
    let original = "猫🙂 \r\n";
    let (root, mut app, _settings) = fixture(original);
    let id = app.doc().id;
    app.doc_mut().move_to(3, false);
    type_character(&mut app, '(');
    let paired = "猫🙂 ()\r\n";
    app.execute("workbench.action.splitEditorRight", Value::Null);
    assert_eq!(app.doc().id, id);
    type_character(&mut app, ')');
    assert_eq!(
        app.doc().text.to_string(),
        "猫🙂 ())\r\n",
        "the new view does not inherit generated ownership"
    );
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    app.execute("workbench.action.focusFirstEditorGroup", Value::Null);
    type_character(&mut app, ')');
    assert_eq!(
        app.doc().text.to_string(),
        paired,
        "original owning view still skips its generated closer"
    );
    app.doc_mut().move_to(4, false);
    let destination = root.path().join("saved.json");
    app.execute("workbench.action.files.saveAs", Value::Null);
    app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    key(&mut app, KeyCode::Enter);
    until(&mut app, "save receipt", |app| !app.saves_pending());
    assert_eq!(app.language(), "json");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), paired);
    assert_eq!(std::fs::read(&destination).unwrap(), paired.as_bytes());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(
        app.language(),
        "json",
        "Undo cannot revert the saved path/profile"
    );
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    app.doc_mut().move_to(4, false);
    type_character(&mut app, ')');
    assert_eq!(
        app.doc().text.to_string(),
        "猫🙂 ())\r\n",
        "saved profile must not inherit old generated ownership from Undo"
    );
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read(destination).unwrap(), paired.as_bytes());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
}
