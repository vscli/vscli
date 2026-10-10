//! User-level parameter-hint qualification through a real framed native peer.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile, lsp::Client, settings::Settings};

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
fn settle(app: &mut App, duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn trace(root: &tempfile::TempDir) -> Vec<Value> {
    std::fs::read_to_string(root.path().join("requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn requests(root: &tempfile::TempDir) -> Vec<Value> {
    trace(root)
        .into_iter()
        .filter(|value| value["event"] == "request")
        .collect()
}
fn fixture(text: &str, hold: bool) -> (tempfile::TempDir, App) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    std::fs::write(&path, text).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let mut args = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/signature_hints_server.py")
            .to_string_lossy()
            .into_owned(),
        root.path()
            .join("requests.jsonl")
            .to_string_lossy()
            .into_owned(),
        root.path().join("release").to_string_lossy().into_owned(),
    ];
    if hold {
        args.push("--hold-first".into());
    }
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &args,
            root.path(),
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, "native peer ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    (root, app)
}
fn invoke(app: &mut App) {
    app.execute("editor.action.triggerParameterHints", Value::Null);
}
fn character(app: &mut App, character: char) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char(character),
        KeyModifiers::NONE,
    )));
}
fn configuration(app: &mut App, root: &tempfile::TempDir, value: Value) {
    let path = root.path().join("signature-settings.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    app.settings = Settings::load(&[path]).unwrap();
}
#[test]
fn automatic_original_character_overloads_and_retrigger_preserve_dirty_unicode_crlf_history() {
    let original = "sum\r\n// 猫🙂\r\n";
    let (root, mut app) = fixture(original, false);
    let id = app.doc().id;
    app.doc_mut().move_to(3, false);
    character(&mut app, '(');
    assert_eq!(app.doc().cursor, 4);
    until(&mut app, "automatic initial hint", |app| {
        app.signature_help().is_some()
    });
    let recorded = requests(&root);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0]["params"]["position"]["character"], 4);
    assert_eq!(
        recorded[0]["params"]["context"],
        json!({"triggerKind":2,"triggerCharacter":"(","isRetrigger":false})
    );
    let hint = app.signature_help().unwrap();
    assert_eq!(&hint.label[hint.parameter.clone().unwrap()], "int left");
    let before = app.doc().text.to_string();
    let selection = app.doc().selections();
    app.event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
    assert_eq!(app.signature_help().unwrap().signature, 1);
    assert_eq!(
        app.signature_help().unwrap().documentation,
        "Unicode overload"
    );
    assert_eq!(
        &app.signature_help().unwrap().label
            [app.signature_help().unwrap().parameter.clone().unwrap()],
        "猫🙂 left"
    );
    assert_eq!(app.doc().text.to_string(), before);
    assert_eq!(app.doc().selections(), selection);
    app.event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)));
    assert_eq!(app.signature_help().unwrap().signature, 0);
    settle(&mut app, Duration::from_millis(160));
    assert_eq!(
        requests(&root).len(),
        1,
        "overload navigation must be local"
    );
    character(&mut app, '1');
    character(&mut app, ',');
    until(&mut app, "comma retrigger", |app| {
        app.signature_help().is_some_and(|hint| {
            hint.label.starts_with("sum2")
                && hint
                    .parameter
                    .clone()
                    .is_some_and(|range| &hint.label[range] == "int right")
        })
    });
    let recorded = requests(&root);
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[1]["params"]["context"]["triggerKind"], 2);
    assert_eq!(recorded[1]["params"]["context"]["triggerCharacter"], ",");
    assert_eq!(recorded[1]["params"]["context"]["isRetrigger"], true);
    assert_eq!(
        recorded[1]["params"]["context"]["activeSignatureHelp"]["activeSignature"],
        0
    );
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
    let edited = app.doc().text.to_string();
    assert!(edited.contains("\r\n// 猫🙂\r\n"));
    app.execute("closeParameterHints", Value::Null);
    assert!(app.signature_help().is_none());
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "save receipt", |app| !app.saves_pending());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        edited.as_bytes()
    );
    app.execute("undo", Value::Null);
    assert_ne!(app.doc().text.to_string(), edited);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        edited.as_bytes()
    );
}
#[test]
fn ignored_cancellation_keeps_one_actual_callback_and_only_latest_of_32_invocations() {
    let original = "sum(1, 2)\r\n// 猫🙂\r\n";
    let (root, mut app) = fixture(original, true);
    let id = app.doc().id;
    app.doc_mut().move_to(4, false);
    invoke(&mut app);
    until(&mut app, "first actual request held", |_| {
        requests(&root).len() == 1
    });
    for cursor in (0..30).map(|index| index % 3 + 4).chain(std::iter::once(7)) {
        // Actual editor commands carry the legitimate active-hint change
        // context into the following Invoke; direct model mutation does not.
        while app.doc().cursor < cursor {
            app.execute("cursorRight", Value::Null);
        }
        while app.doc().cursor > cursor {
            app.execute("cursorLeft", Value::Null);
        }
        invoke(&mut app);
        app.poll();
    }
    settle(&mut app, Duration::from_millis(160));
    assert_eq!(requests(&root).len(), 1);
    assert!(app.signature_help().is_none());
    assert!(trace(&root).iter().any(|value| value["event"] == "cancel"));
    std::fs::write(root.path().join("release"), b"release").unwrap();
    until(&mut app, "latest invocation after actual release", |app| {
        app.signature_help()
            .is_some_and(|hint| hint.label.starts_with("sum2"))
    });
    settle(&mut app, Duration::from_millis(160));
    let recorded = requests(&root);
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().all(|value| value["maxPending"] == 1));
    assert_eq!(recorded[0]["params"]["position"]["character"], 4);
    assert_eq!(recorded[1]["params"]["position"]["character"], 7);
    assert_eq!(recorded[0]["params"]["context"]["isRetrigger"], false);
    assert_eq!(
        recorded[1]["params"]["context"]["isRetrigger"], true,
        "another Invoke while logical hints remain pending must retrigger"
    );
    assert_eq!(
        &app.signature_help().unwrap().label
            [app.signature_help().unwrap().parameter.clone().unwrap()],
        "int right"
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), original);
    assert!(!app.doc().dirty());
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
}
#[test]
fn paste_does_not_initiate_disabled_auto_allows_manual_and_cycle_boundary_dismisses() {
    let (root, mut app) = fixture("sum\r\n", false);
    configuration(
        &mut app,
        &root,
        json!({"editor.parameterHints.enabled":false,"editor.parameterHints.cycle":false}),
    );
    app.doc_mut().move_to(3, false);
    character(&mut app, '(');
    settle(&mut app, Duration::from_millis(180));
    assert_eq!(requests(&root).len(), 0);
    invoke(&mut app);
    until(
        &mut app,
        "manual invocation despite automatic disabled",
        |app| app.signature_help().is_some(),
    );
    app.execute("showPrevParameterHint", Value::Null);
    assert!(app.signature_help().is_none());
    app.execute("undo", Value::Null);
    configuration(
        &mut app,
        &root,
        json!({"editor.parameterHints.enabled":true}),
    );
    app.event(Event::Paste("sum(".into()));
    settle(&mut app, Duration::from_millis(180));
    assert_eq!(
        requests(&root).len(),
        1,
        "literal paste must not initiate hints"
    );
    assert!(app.signature_help().is_none());
}
#[test]
fn byte_equal_edit_undo_and_settings_restore_cannot_resurrect_held_hint_or_mutate_history() {
    let original = "sum(1, 2)\r\n// 猫🙂\r\n";
    let (root, mut app) = fixture(original, true);
    app.doc_mut().move_to(7, false);
    app.doc_mut().insert("dirty", false);
    let dirty = app.doc().text.to_string();
    let dirty_revision = app.doc().revision;
    invoke(&mut app);
    until(&mut app, "held dirty request", |_| {
        requests(&root).len() == 1
    });
    app.doc_mut().insert("x", false);
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().revision, dirty_revision);
    app.poll();
    let prior = app.settings.clone();
    configuration(
        &mut app,
        &root,
        json!({"editor.parameterHints.enabled":false}),
    );
    app.poll();
    app.settings = prior;
    app.poll();
    std::fs::write(root.path().join("release"), b"release").unwrap();
    until(&mut app, "held actual response received", |_| {
        trace(&root)
            .iter()
            .any(|value| value["event"] == "response")
    });
    settle(&mut app, Duration::from_millis(180));
    assert!(app.signature_help().is_none());
    assert_eq!(requests(&root).len(), 1);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
    app.doc_mut().redo();
    assert!(app.doc().text.to_string().contains("dirtyx"));
    app.doc_mut().undo();
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), original);
}

#[test]
fn saving_and_unrelated_retained_buffer_edits_keep_readonly_hint_without_new_callbacks() {
    let original = "sum(1, 2)\r\n// 猫🙂\r\n";
    let (root, mut app) = fixture(original, false);
    let main_id = app.doc().id;
    let other_path = root.path().join("other.cpp");
    std::fs::write(&other_path, "other\r\n").unwrap();
    app.open(&other_path).unwrap();
    until(&mut app, "other buffer ready", |app| {
        app.doc().id != main_id
    });
    let other_id = app.doc().id;
    app.open(&root.path().join("main.cpp")).unwrap();
    until(&mut app, "main buffer restored", |app| {
        app.doc().id == main_id
    });
    app.doc_mut().move_to(7, false);
    invoke(&mut app);
    until(&mut app, "readonly hint ready", |app| {
        app.signature_help().is_some()
    });
    app.documents
        .iter_mut()
        .find(|document| document.id == other_id)
        .unwrap()
        .insert("dirty 猫🙂", false);
    app.poll();
    assert!(
        app.signature_help().is_some(),
        "unrelated retained models do not authorize signature edits"
    );
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, "save receipt", |app| !app.saves_pending());
    settle(&mut app, Duration::from_millis(160));
    assert!(
        app.signature_help().is_some(),
        "saving unchanged text keeps the readonly hint"
    );
    assert_eq!(requests(&root).len(), 1);
    assert_eq!(app.doc().id, main_id);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
    assert_eq!(std::fs::read(other_path).unwrap(), b"other\r\n");
    assert!(
        app.documents
            .iter()
            .find(|document| document.id == other_id)
            .unwrap()
            .dirty()
    );
}

#[test]
fn native_capability_bounds_reject_oversized_triggers_but_ignore_unknown_large_fields() {
    let (root, mut app) = fixture("sum(1, 2)\r\n", false);
    app.doc_mut().move_to(7, false);
    app.lsp.as_mut().unwrap().capabilities["signatureHelpProvider"]["junk"] =
        json!("z".repeat(1024 * 1024));
    invoke(&mut app);
    until(
        &mut app,
        "large unrelated capability field ignored",
        |app| app.signature_help().is_some(),
    );
    assert_eq!(requests(&root).len(), 1);
    app.execute("closeParameterHints", Value::Null);
    for malformed in [
        json!(vec!["x"; 17]),
        json!(["xx"]),
        json!(["x".repeat(1024 * 1024)]),
        json!([""]),
        json!([5]),
        json!(false),
    ] {
        app.lsp.as_mut().unwrap().capabilities["signatureHelpProvider"]["triggerCharacters"] =
            malformed;
        assert_eq!(app.context()["editorHasSignatureHelpProvider"], false);
        invoke(&mut app);
        assert!(app.message.contains("does not provide"));
        assert!(app.signature_help().is_none());
    }
    app.lsp.as_mut().unwrap().capabilities["signatureHelpProvider"]["triggerCharacters"] =
        json!(vec!["x"; 100_000]);
    character(&mut app, '(');
    settle(&mut app, Duration::from_millis(160));
    assert_eq!(
        requests(&root).len(),
        1,
        "invalid metadata must not start any callback"
    );
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        b"sum(1, 2)\r\n"
    );
    assert!(
        app.doc().dirty(),
        "native typing remains usable without a valid signature capability"
    );
}
