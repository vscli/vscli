use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Client, keys::Profile};
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let start = Instant::now();
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "Timed out: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn fixture(app: &mut App) {
    let extension =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
    app.extension_host = Some(
        Client::start(
            "node",
            &extension,
            &app.workspace.root,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(app, |a| a.extension_host.as_ref().is_some_and(|h| h.ready));
}
#[test]
fn unchanged_command_extension_edits_native_unicode_buffers_and_preserves_undo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("space 猫#.txt");
    std::fs::write(&path, "猫\n🙂\na").unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    app.doc_mut().select_all();
    fixture(&mut app);
    assert!(
        app.palette_items("Fixture Sort")
            .iter()
            .any(|(_, id)| *id == "fixture.sort")
    );
    app.execute("fixture.sort", Value::Null);
    until(&mut app, |a| a.message.contains("sort applied=true"));
    assert_eq!(app.doc().text.to_string(), "a\n猫\n🙂");
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "猫\n🙂\na");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "猫\n🙂\na");
    app.execute("fixture.version", Value::Null);
    until(&mut app, |a| a.message == "version=3");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), "a\n猫\n🙂");
    app.execute("vscli.extensions.stop", Value::Null);
    assert!(app.extension_host.is_none());
    assert_eq!(app.keymap.shortcut("fixture.sort"), "");
    assert_eq!(
        app.keymap.shortcut("editor.debug.action.toggleBreakpoint"),
        "f9"
    );
    assert_eq!(app.doc().text.to_string(), "a\n猫\n🙂");
}
#[test]
fn invalid_stale_and_crashed_extensions_cannot_destroy_native_edits() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.doc_mut().insert("original", false);
    fixture(&mut app);
    app.execute("fixture.invalid", Value::Null);
    until(&mut app, |a| a.message.contains("overlapping edits"));
    assert_eq!(app.doc().text.to_string(), "original");
    app.execute("fixture.unsupported", Value::Null);
    until(&mut app, |a| {
        a.message
            .contains("not implemented: window.createWebviewPanel")
    });
    assert!(app.extension_host.is_some());
    app.execute("fixture.stale", Value::Null);
    // Let the extension submit its edit against the old version without polling
    // Rust: the next poll syncs the user's newer revision before considering it.
    let started = Instant::now();
    while !directory.path().join("edit-submitted").exists() {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    app.doc_mut().insert("!", false);
    until(&mut app, |a| a.message.contains("stale applied=false"));
    assert_eq!(app.doc().text.to_string(), "original!");
    app.execute("fixture.crash", Value::Null);
    until(&mut app, |a| a.extension_host.is_none());
    assert!(app.message.contains("Extension host stopped"));
    assert_eq!(app.keymap.shortcut("fixture.sort"), "");
    assert_eq!(
        app.keymap.shortcut("editor.debug.action.toggleBreakpoint"),
        "f9"
    );
    app.doc_mut().insert(" still editable", false);
    assert_eq!(app.doc().text.to_string(), "original! still editable");
}
#[test]
#[ignore = "requires an unpacked upstream Sort Lines extension in VSCLI_TEST_SORT_LINES"]
fn upstream_sort_lines_runs_without_source_changes() {
    let extension = PathBuf::from(
        std::env::var_os("VSCLI_TEST_SORT_LINES").expect("Set VSCLI_TEST_SORT_LINES"),
    );
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.doc_mut().insert("zebra\napple\npear", false);
    app.doc_mut().select_all();
    app.extension_host = Some(
        Client::start(
            "node",
            &extension,
            &app.workspace.root,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| h.ready)
    });
    app.event(Event::Key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)));
    until(&mut app, |a| a.doc().text == "apple\npear\nzebra");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "zebra\napple\npear");
}

#[test]
fn extension_keybinding_arguments_preserve_arrays_null_and_absence() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    let bindings = directory.path().join("keybindings.json");
    std::fs::write(
        &bindings,
        r#"[
      {"key":"f8","command":"fixture.args","args":["a",2]},
      {"key":"f7","command":"fixture.args","args":null},
      {"key":"f6","command":"fixture.args"}
    ]"#,
    )
    .unwrap();
    app.keymap.load(&bindings).unwrap();
    fixture(&mut app);
    for (key, expected) in [(8, "args=[[\"a\",2]]"), (7, "args=[null]"), (6, "args=[]")] {
        app.event(Event::Key(KeyEvent::new(
            KeyCode::F(key),
            KeyModifiers::NONE,
        )));
        until(&mut app, |a| a.message == expected);
    }
}

#[test]
fn extension_settings_are_ready_at_activation_and_reload_without_losing_valid_values() {
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.json");
    let config_dir = directory.path().join(".vscode");
    std::fs::create_dir(&config_dir).unwrap();
    let workspace = config_dir.join("settings.json");
    std::fs::write(&user, r#"{"fixture.value":"user"}"#).unwrap();
    std::fs::write(&workspace, r#"{"fixture.value":"workspace"}"#).unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.configure_settings(Some(user.clone())).unwrap();
    fixture(&mut app);
    let report = |app: &mut App, value: &str, changes: usize| {
        let expected =
            format!(r#"config={{"activation":"workspace","value":"{value}","changes":{changes}}}"#);
        app.execute("fixture.configuration", Value::Null);
        until(app, |a| a.message == expected);
    };
    report(&mut app, "workspace", 0);
    std::fs::write(&workspace, "invalid JSON").unwrap();
    until(&mut app, |a| {
        a.message.contains("previous settings retained")
    });
    report(&mut app, "workspace", 0);
    std::fs::remove_file(&workspace).unwrap();
    until(&mut app, |a| a.message == "configuration changed=user");
    report(&mut app, "user", 1);
    std::fs::write(&user, r#"{"fixture.value":"updated"}"#).unwrap();
    until(&mut app, |a| a.message == "configuration changed=updated");
    report(&mut app, "updated", 2);
    assert!(app.extension_host.as_ref().is_some_and(|h| h.ready));
}

#[test]
#[ignore = "requires an unpacked upstream Sort Lines extension in VSCLI_TEST_SORT_LINES"]
fn upstream_sort_lines_reads_user_workspace_settings_and_live_changes() {
    let extension = PathBuf::from(
        std::env::var_os("VSCLI_TEST_SORT_LINES").expect("Set VSCLI_TEST_SORT_LINES"),
    );
    let directory = tempfile::tempdir().unwrap();
    let user = directory.path().join("user.json");
    let config_dir = directory.path().join(".vscode");
    std::fs::create_dir(&config_dir).unwrap();
    let workspace = config_dir.join("settings.json");
    std::fs::write(&user, r#"{"sortLines.sortEntireFile":false}"#).unwrap();
    std::fs::write(&workspace, r#"{"sortLines.sortEntireFile":true}"#).unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.configure_settings(Some(user)).unwrap();
    app.doc_mut().insert("zebra\napple\npear", false);
    app.extension_host = Some(
        Client::start(
            "node",
            &extension,
            &app.workspace.root,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| h.ready)
    });
    app.event(Event::Key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)));
    until(&mut app, |a| a.doc().text == "apple\npear\nzebra");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "zebra\napple\npear");
    std::fs::remove_file(&workspace).unwrap();
    until(&mut app, |a| a.message.starts_with("Settings reloaded"));
    app.event(Event::Key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)));
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| !h.busy())
    });
    assert_eq!(app.doc().text.to_string(), "zebra\napple\npear");
    std::fs::write(&workspace, r#"{"sortLines.sortEntireFile":true}"#).unwrap();
    until(&mut app, |a| a.message.starts_with("Settings reloaded"));
    app.event(Event::Key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)));
    until(&mut app, |a| a.doc().text == "apple\npear\nzebra");
}
