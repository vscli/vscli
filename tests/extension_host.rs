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
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
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
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
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
fn held_extension_edit_rejects_edit_undo_revision_reuse_without_losing_native_redo_or_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared 猫.txt");
    let original = "original 猫\r\n🙂\r\n";
    std::fs::write(&path, original).unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let identity = app.doc().id;
    let revision = app.doc().revision;
    let selections = app.doc().selections();
    fixture(&mut app);
    app.execute("fixture.stale", Value::Null);
    // The extension has already sent a versioned edit; hold native polling so
    // both text mutations happen before any mirror observes the intermediate state.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.path().join("edit-submitted").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    app.doc_mut().insert("transient:", false);
    app.doc_mut().undo();
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().selections(), selections);
    until(&mut app, |app| app.message == "stale applied=false");
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), original);
    assert!(!app.doc().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    app.execute("fixture.version", Value::Null);
    until(&mut app, |app| app.message == "version=2");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), format!("transient:{original}"));
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}
#[test]
#[ignore = "requires an unpacked upstream Sort Lines extension in VSCLI_TEST_SORT_LINES"]
fn upstream_sort_lines_runs_without_source_changes() {
    let extension = PathBuf::from(
        std::env::var_os("VSCLI_TEST_SORT_LINES").expect("Set VSCLI_TEST_SORT_LINES"),
    );
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
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
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
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
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
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
fn configuration_listeners_observe_native_edits_before_the_configuration_change() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    fixture(&mut app);
    app.doc_mut().insert("current native edit", false);
    let configuration = directory.path().join(".vscode");
    std::fs::create_dir(&configuration).unwrap();
    std::fs::write(
        configuration.join("settings.json"),
        r#"{"fixture.value":"changed"}"#,
    )
    .unwrap();
    app.configure_settings(None).unwrap();
    // No intermediate poll: both document and settings are unsynchronized.
    app.execute("fixture.observedDocument", Value::Null);
    until(&mut app, |a| {
        a.message == "configuration document=current native edit"
    });
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
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
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

#[test]
fn extension_commands_run_with_no_open_editors_before_and_after_last_tab_closes() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    fixture(&mut app);
    for with_editor in [false, true, false] {
        if with_editor {
            app.execute("workbench.action.files.newUntitledFile", Value::Null);
        } else if !app.documents.is_empty() {
            app.execute("workbench.action.closeActiveEditor", Value::Null);
        }
        app.execute("fixture.configuration", Value::Null);
        until(&mut app, |app| app.message.starts_with("config="));
        assert_eq!(app.documents.is_empty(), !with_editor);
        assert!(app.extension_host.is_some());
    }
}

fn package(folder: &std::path::Path, name: &str, source: &str) -> vscli::extensions::Package {
    let path = folder.join(name);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("package.json"), serde_json::json!({"publisher":"session", "name":name, "version":"1.0.0", "main":"extension.cjs", "contributes": {"configuration": {"properties": {format!("{name}.value"): {"default": name}}}}}).to_string()).unwrap();
    std::fs::write(path.join("extension.cjs"), source).unwrap();
    vscli::extensions::Package::read(&path).unwrap()
}

#[test]
fn shared_packages_execute_cross_commands_and_observe_one_native_document() {
    let directory = tempfile::tempdir().unwrap();
    let a = package(
        directory.path(),
        "a",
        r#"
      const vscode = require('vscode');
      exports.activate = context => {
        if (vscode.workspace.getConfiguration('b').get('value') !== 'b') throw new Error('Missing other package defaults');
        const document = vscode.window.activeTextEditor.document;
        context.subscriptions.push(vscode.commands.registerCommand('session.a', async () => {
          const applied = await vscode.window.activeTextEditor.edit(edit => edit.insert(new vscode.Position(0, 0), 'A'));
          return vscode.window.showInformationMessage(`a=${applied};${document.version};${document.getText()}`);
        }));
      };
    "#,
    );
    let b = package(
        directory.path(),
        "b",
        r#"
      const vscode = require('vscode');
      exports.activate = context => {
        const document = vscode.window.activeTextEditor.document;
        let events = 0;
        context.subscriptions.push(vscode.workspace.onDidChangeTextDocument(event => {
          if (event.document !== document) throw new Error('Changed document identity');
          events++;
        }));
        context.subscriptions.push(vscode.commands.registerCommand('session.b', () => vscode.commands.executeCommand('session.a')));
        context.subscriptions.push(vscode.commands.registerCommand('session.observe', () => vscode.window.showInformationMessage(`b=${events};${document.version};${document.getText()}`)));
      };
    "#,
    );
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("original", false);
    let identity = app.doc().id;
    app.extension_host = Some(
        Client::start_many(
            "node",
            &[b, a],
            directory.path(),
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| h.ready)
    });
    assert_eq!(
        app.extension_host.as_ref().unwrap().packages[0].id,
        "session.a"
    );
    app.execute("session.b", Value::Null);
    until(&mut app, |a| a.message == "a=true;2;Aoriginal");
    app.execute("session.observe", Value::Null);
    until(&mut app, |a| a.message == "b=1;2;Aoriginal");
    assert_eq!(app.doc().id, identity);
    assert!(app.doc().dirty());
    app.doc_mut().undo();
    app.execute("session.observe", Value::Null);
    until(&mut app, |a| a.message == "b=2;3;original");
}

#[test]
fn duplicate_packages_and_cumulative_contributions_reject_before_start() {
    let directory = tempfile::tempdir().unwrap();
    let a = package(directory.path(), "a", "exports.activate = () => {};");
    let mut app = App::new(directory.path().into(), Profile::Linux);
    assert!(
        Client::start_many(
            "missing-node",
            &[a.clone(), a.clone()],
            directory.path(),
            &app.documents,
            app.active,
            &app.settings
        )
        .err()
        .unwrap()
        .to_string()
        .contains("Duplicate extension")
    );
    let b = package(directory.path(), "b", "exports.activate = () => {};");
    for item in [&a, &b] {
        let file = item.path.join("package.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        manifest["contributes"]["commands"] =
            serde_json::json!(vec![serde_json::json!({"command":"unused"}); 600]);
        std::fs::write(file, manifest.to_string()).unwrap();
    }
    assert!(
        Client::start_many(
            "missing-node",
            &[a, b],
            directory.path(),
            &app.documents,
            app.active,
            &app.settings
        )
        .err()
        .unwrap()
        .to_string()
        .contains("contribution limit")
    );
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("still native", false);
    assert_eq!(app.doc().text.to_string(), "still native");
}

#[test]
fn simultaneous_packages_edits_accept_one_version_and_preserve_undo() {
    let directory = tempfile::tempdir().unwrap();
    let source = |name: &str| {
        format!(
            r#"
      const vscode = require('vscode'), fs = require('node:fs'), path = require('node:path');
      exports.activate = context => context.subscriptions.push(vscode.commands.registerCommand('race.{name}', async () => {{
        const edit = vscode.window.activeTextEditor.edit(builder => builder.insert(new vscode.Position(0, 0), '{name}'));
        fs.writeFileSync(path.join(vscode.workspace.rootPath, '{name}.submitted'), 'ready');
        const applied = await edit;
        fs.writeFileSync(path.join(vscode.workspace.rootPath, '{name}.result'), String(applied));
      }}));
    "#
        )
    };
    let packages = vec![
        package(directory.path(), "a", &source("a")),
        package(directory.path(), "b", &source("b")),
    ];
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("original", false);
    app.start_extension_packages(packages).unwrap();
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| h.ready)
    });
    app.execute("race.a", Value::Null);
    app.execute("race.b", Value::Null);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !["a.submitted", "b.submitted"]
        .iter()
        .all(|name| directory.path().join(name).exists())
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    until(&mut app, |_| {
        ["a.result", "b.result"]
            .iter()
            .all(|name| directory.path().join(name).exists())
    });
    let mut results = ["a", "b"].map(|name| {
        std::fs::read_to_string(directory.path().join(format!("{name}.result"))).unwrap()
    });
    results.sort();
    assert_eq!(results, ["false", "true"]);
    assert!(matches!(
        app.doc().text.to_string().as_str(),
        "aoriginal" | "boriginal"
    ));
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "original");
}

#[test]
fn activation_failure_keeps_valid_native_transaction_and_restart_is_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let a = package(
        directory.path(),
        "a",
        r#"
      const vscode = require('vscode');
      exports.activate = async context => {
        context.subscriptions.push(vscode.commands.registerCommand('staged.a', () => {}));
        await vscode.window.activeTextEditor.edit(edit => edit.insert(new vscode.Position(0, 0), 'valid '));
      };
    "#,
    );
    let b = package(
        directory.path(),
        "b",
        "exports.activate = () => { throw new Error('deliberate failure'); };",
    );
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("original", false);
    let id = app.doc().id;
    app.start_extension_packages(vec![a, b]).unwrap();
    until(&mut app, |a| a.message.contains("deliberate failure"));
    assert!(app.extension_host.is_none());
    assert!(app.palette_items("staged.a").is_empty());
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "valid original");
    assert!(app.doc().dirty());
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "original");
    app.doc_mut().insert(" editable", false);
    assert_eq!(app.doc().text.to_string(), "original editable");
    assert_eq!(app.extension_packages.len(), 2);
}

#[test]
fn duplicate_or_native_command_activation_fails_without_publishing_a_partial_registry() {
    for conflict in ["collision", "type", "undo", "cursorLeft"] {
        let directory = tempfile::tempdir().unwrap();
        let source = |id: &str| {
            format!(
                "const vscode = require('vscode'); exports.activate = context => context.subscriptions.push(vscode.commands.registerCommand('{id}', () => {{}}));"
            )
        };
        let a = package(directory.path(), "a", &source("collision"));
        let b = package(directory.path(), "b", &source(conflict));
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.start_extension_packages(vec![b, a]).unwrap();
        until(&mut app, |a| a.message.contains("activation failed"));
        assert!(app.extension_host.is_none());
        assert!(app.palette_items("collision").is_empty());
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.execute("type", serde_json::json!({"text":"native"}));
        assert_eq!(app.doc().text.to_string(), "native");
    }
}

#[cfg(unix)]
#[test]
fn fifo_manifest_rejects_before_opening_and_never_blocks_selection() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("package.json");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&manifest)
            .status()
            .unwrap()
            .success()
    );
    let path = directory.path().to_owned();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        sender
            .send(
                vscli::extensions::Package::read(&path)
                    .err()
                    .unwrap()
                    .to_string(),
            )
            .unwrap();
    });
    let error = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("FIFO manifest must not block opening");
    assert!(error.contains("regular file"));
}
