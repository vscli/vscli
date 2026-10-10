//! Installed VSIX metadata through the real asynchronous native App loader.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, extension_activation::Scope, extension_store::Store, keys::Profile};
use zip::{ZipWriter, write::SimpleFileOptions};

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
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
fn install(root: &Path, store: &Store, version: &str, open: char, close: char) {
    let archive = root.join("native.vsix");
    let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
    let manifest = json!({"publisher":"fixture","name":"native","version":version,"main":"main.cjs", "activationEvents":["*"],"engines":{"vscode":"^1.95.0"},"contributes":{"languages":[{"id":"cpp","configuration":"cpp.json"},{"id":"json","configuration":"json.json"}]}});
    for (path, bytes) in [
        ("extension/package.json", manifest.to_string()),
        ("extension/main.cjs", "throw new Error('Native configuration must not execute package code');".into()),
        ("extension/cpp.json", json!({"autoClosingPairs":[{"open":open.to_string(),"close":close.to_string()}],"surroundingPairs":[[open.to_string(),close.to_string()]],"brackets":[[open.to_string(),close.to_string()]],"autoCloseBefore":" "}).to_string()),
        ("extension/json.json", json!({"autoClosingPairs":[{"open":"(","close":")"}],"brackets":[["(",")"]]}).to_string()),
    ] {
        zip.start_file(path, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    store.install(&archive).unwrap();
}
fn install_refresh_witness(root: &Path, store: &Store) {
    let archive = root.join("witness.vsix");
    let mut zip = ZipWriter::new(fs::File::create(&archive).unwrap());
    zip.start_file("extension/package.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(json!({"publisher":"fixture","name":"witness","version":"1.0.0","contributes":{"languages":[{"id":"rust","configuration":"rust.json"}]}}).to_string().as_bytes()).unwrap();
    zip.start_file("extension/rust.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"{}").unwrap();
    zip.finish().unwrap();
    store.install(&archive).unwrap();
}
fn configured(root: &Path, store: &Store) -> App {
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    let settings = config.join("settings.json");
    fs::write(
        &settings,
        json!({"vscli.languageServer.enabled":false,"editor.autoIndent":"brackets"}).to_string(),
    )
    .unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.extensions_directory = Some(store.root().into());
    app.extension_node = root.join("node-unavailable").to_string_lossy().into_owned();
    app.configure_settings(Some(settings)).unwrap();
    app.configure_extension_activation(Some(&config));
    app
}
fn preference(app: &mut App, enabled: bool, scope: Scope) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        match app.set_extension_enabled("fixture.native", enabled, scope) {
            Ok(()) => return,
            Err(error) if error.to_string().contains("already") => {
                assert!(Instant::now() < deadline, "{error:#}");
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("{error:#}"),
        }
    }
}

#[test]
fn installed_code_package_data_applies_before_and_after_open_without_node_or_execution_grant() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("store"));
    install(root.path(), &store, "1.0.0", '<', '>');
    let original = "猫🙂 \r\n";
    let first = root.path().join("main.cpp");
    fs::write(&first, original).unwrap();
    let second = root.path().join("later.cpp");
    fs::write(&second, original).unwrap();
    let mut app = configured(root.path(), &store);
    app.open(&first).unwrap();
    let id = app.doc().id;
    until(&mut app, "declarative background publication", |app| {
        app.language_configuration("cpp").is_some()
    });
    assert!(!app.extension_enabled("fixture.native"));
    assert!(app.extension_host.is_none());
    assert!(app.lsp.is_none());
    app.doc_mut().move_to(3, false);
    key(&mut app, KeyCode::Char('<'));
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>\r\n");
    assert_eq!(app.doc().id, id);
    assert_eq!(fs::read(&first).unwrap(), original.as_bytes());
    app.execute("workbench.action.files.save", Value::Null);
    assert_eq!(fs::read(&first).unwrap(), "猫🙂 <>\r\n".as_bytes());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>\r\n");
    app.open(&second).unwrap();
    app.doc_mut().move_to(3, false);
    key(&mut app, KeyCode::Char('<'));
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>\r\n");
    assert!(app.extension_host.is_none());
    assert!(app.lsp.is_none());
}

#[test]
fn remembered_disable_workspace_override_and_a_b_a_do_not_resurrect_generated_ownership() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("store"));
    install(root.path(), &store, "1.0.0", '<', '>');
    let source = root.path().join("main.cpp");
    fs::write(&source, "猫🙂 \r\n").unwrap();
    let mut app = configured(root.path(), &store);
    app.open(&source).unwrap();
    until(&mut app, "initial catalog", |app| {
        app.language_configuration("cpp").is_some()
    });
    let original_identity = app.language_configuration("cpp").unwrap().identity.clone();
    app.doc_mut().move_to(3, false);
    key(&mut app, KeyCode::Char('<'));
    let paired = app.doc().text.to_string();
    preference(&mut app, false, Scope::Global);
    until(&mut app, "explicit global disable", |app| {
        app.language_configuration("cpp").is_none()
    });
    app.execute("undo", Value::Null);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    preference(&mut app, true, Scope::Workspace);
    until(&mut app, "workspace overrides global disable", |app| {
        app.language_configuration("cpp").is_some()
    });
    assert_eq!(
        app.language_configuration("cpp").unwrap().identity,
        original_identity
    );
    app.doc_mut().move_to(4, false);
    key(&mut app, KeyCode::Char('>'));
    assert_eq!(
        app.doc().text.to_string(),
        "猫🙂 <>>\r\n",
        "equal configuration must not revive retired Undo provenance"
    );
    preference(&mut app, false, Scope::Workspace);
    until(&mut app, "explicit workspace disable", |app| {
        app.language_configuration("cpp").is_none()
    });
    assert_eq!(fs::read(&source).unwrap(), "猫🙂 \r\n".as_bytes());
    app.execute("workbench.action.files.save", Value::Null);
    assert_eq!(fs::read(&source).unwrap(), "猫🙂 <>>\r\n".as_bytes());
    let mut restarted = configured(root.path(), &store);
    until(&mut restarted, "remembered disabled registry", |app| {
        app.extension_activation_status("fixture.native") == "disabled"
    });
    assert!(restarted.language_configuration("cpp").is_none());
}

#[test]
fn installed_update_rollback_uninstall_and_same_source_refresh_guard_native_history() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("store"));
    install(root.path(), &store, "1.0.0", '<', '>');
    let source = root.path().join("main.cpp");
    fs::write(&source, "猫🙂 \r\n").unwrap();
    let mut app = configured(root.path(), &store);
    app.open(&source).unwrap();
    until(&mut app, "version one", |app| {
        app.language_configuration("cpp").is_some()
    });
    let identity = app.language_configuration("cpp").unwrap().identity.clone();
    app.doc_mut().move_to(3, false);
    key(&mut app, KeyCode::Char('<'));
    let paired = app.doc().text.to_string();
    install_refresh_witness(root.path(), &store);
    app.refresh_extension_catalog();
    // The newly installed, unrelated language proves a fresh catalog was
    // published rather than accepting the previous snapshot as the barrier.
    until(&mut app, "same installed generation", |app| {
        app.language_configuration("rust").is_some()
    });
    assert_eq!(
        app.language_configuration("cpp").unwrap().identity,
        identity
    );
    key(&mut app, KeyCode::Char('>'));
    assert_eq!(app.doc().text.to_string(), paired);
    // Skipping consumes its generated mark. Create a fresh owned pair so the
    // upgrade/rollback assertion actually tests retirement of live ownership.
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "猫🙂 \r\n");
    key(&mut app, KeyCode::Char('<'));
    assert_eq!(app.doc().text.to_string(), paired);
    app.doc_mut().move_to(4, false);
    install(root.path(), &store, "2.0.0", '[', ']');
    app.refresh_extension_catalog();
    until(&mut app, "upgraded generation", |app| {
        app.language_configuration("cpp")
            .is_some_and(|c| c.identity.version == "2.0.0")
    });
    store.rollback("fixture.native").unwrap();
    app.refresh_extension_catalog();
    until(&mut app, "rolled back generation", |app| {
        app.language_configuration("cpp")
            .is_some_and(|c| c.identity == identity)
    });
    key(&mut app, KeyCode::Char('>'));
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>>\r\n");
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "猫🙂 \r\n");
    key(&mut app, KeyCode::Char('<'));
    assert_eq!(app.doc().text.to_string(), paired);
    store.uninstall("fixture.native").unwrap();
    app.refresh_extension_catalog();
    until(&mut app, "uninstalled generation", |app| {
        app.language_configuration("cpp").is_none()
    });
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "猫🙂 \r\n");
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), paired);
    app.doc_mut().move_to(4, false);
    key(&mut app, KeyCode::Char('>'));
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>>\r\n");
    assert!(app.extension_host.is_none());
    assert!(app.lsp.is_none());
}

#[test]
fn save_as_cpp_to_json_rebinds_native_configuration_preserving_identity_crlf_and_history() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("store"));
    install(root.path(), &store, "1.0.0", '<', '>');
    let source = root.path().join("main.cpp");
    let destination = root.path().join("saved.json");
    fs::write(&source, "猫🙂 \r\n").unwrap();
    let mut app = configured(root.path(), &store);
    app.open(&source).unwrap();
    until(&mut app, "both language configurations", |app| {
        app.language_configuration("cpp").is_some() && app.language_configuration("json").is_some()
    });
    let id = app.doc().id;
    app.doc_mut().move_to(3, false);
    key(&mut app, KeyCode::Char('<'));
    app.execute("workbench.action.files.saveAs", Value::Null);
    app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.language(), "json");
    assert_eq!(app.doc().id, id);
    assert_eq!(fs::read(&destination).unwrap(), "猫🙂 <>\r\n".as_bytes());
    assert_eq!(fs::read(&source).unwrap(), "猫🙂 \r\n".as_bytes());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "猫🙂 \r\n");
    assert_eq!(app.language(), "json");
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>\r\n");
    app.doc_mut().move_to(4, false);
    key(&mut app, KeyCode::Char('>'));
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>>\r\n");
    let end = app.doc().line_end(0);
    app.doc_mut().move_to(end, false);
    key(&mut app, KeyCode::Char('('));
    assert_eq!(app.doc().text.to_string(), "猫🙂 <>>()\r\n");
    assert_eq!(app.doc().id, id);
    app.execute("workbench.action.files.save", Value::Null);
    assert_eq!(fs::read(&destination).unwrap(), "猫🙂 <>>()\r\n".as_bytes());
    assert!(app.lsp.is_none());
}
