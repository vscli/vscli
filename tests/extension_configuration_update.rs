use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
fn until(app: &mut App, check: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn ready(root: &Path) -> App {
    let user = root.join("profile/settings.json");
    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
    std::fs::write(&user,"{\r\n  // user 猫🙂\r\n  \"fixture.value\": \"before\",\r\n  \"unknown.keep\": [1, {\"x\": true}]\r\n}\r\n").unwrap();
    let file = root.join("main.cpp");
    std::fs::write(&file, "λ🙂 original\r\nsecond\r\n").unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(user)).unwrap();
    app.open(&file).unwrap();
    app.execute("workbench.action.splitEditor", Value::Null);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/configuration-update-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    app
}
#[test]
fn public_configuration_values_removal_and_events_reach_real_profile_before_void_with_native_history_retained()
 {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path());
    let id = app.doc().id;
    let original = app.doc().text.to_string();
    app.doc_mut().insert("dirty ", true);
    let dirty = app.doc().text.to_string();
    let source = root.path().join("source-settings.json");
    std::fs::write(&source, "source unchanged\r\n").unwrap();
    app.execute("fixture.configuration.roundtrip", Value::Null);
    until(&mut app, |app| {
        app.message == "Configuration roundtrip persisted"
    });
    let user = std::fs::read_to_string(root.path().join("profile/settings.json")).unwrap();
    assert!(user.contains("// user 猫🙂\r\n"));
    assert!(user.contains("\"unknown.keep\": [1, {\"x\": true}]"));
    assert_eq!(
        vscli::jsonc::parse::<Value>(&user).unwrap()["fixture.value"],
        Value::Null
    );
    let workspace = std::fs::read_to_string(root.path().join(".vscode/settings.json")).unwrap();
    assert_eq!(
        vscli::jsonc::parse::<Value>(&workspace).unwrap()["fixture.value"],
        serde_json::json!(["workspace", "λ🙂"])
    );
    assert_eq!(
        std::fs::read_to_string(source).unwrap(),
        "source unchanged\r\n"
    );
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(
        std::fs::read(root.path().join("main.cpp")).unwrap(),
        original.as_bytes()
    );
    assert!(!root.path().join(".vscode/.vscli-write.lock").exists());
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_language_updates_affect_native_editor_before_ack_and_preserve_unrelated_jsonc() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".vscode")).unwrap();
    let path = root.path().join(".vscode/settings.json");
    std::fs::write(&path,"{\r\n // workspace 猫🙂\r\n \"fixture.value\": \"workspace\",\r\n \"[cpp]\": {\"fixture.value\": false, \"keep.language\": 7},\r\n \"keep.root\": true\r\n}\r\n").unwrap();
    let mut app = ready(root.path());
    let id = app.doc().id;
    let original = app.doc().text.to_string();
    app.execute("fixture.configuration.language", Value::Null);
    until(&mut app, |app| {
        app.message == "Language configuration persisted"
    });
    let bytes = std::fs::read_to_string(path).unwrap();
    let value = vscli::jsonc::parse::<Value>(&bytes).unwrap();
    assert!(bytes.contains("// workspace 猫🙂\r\n"));
    assert_eq!(value["fixture.value"], "root");
    assert!(value["[cpp]"].get("fixture.value").is_none());
    assert_eq!(value["[cpp]"]["keep.language"], 7);
    assert_eq!(value["[cpp]"]["editor.tabSize"], 3);
    assert_eq!(value["keep.root"], true);
    assert_eq!(app.doc().tab_size, 3);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(app.doc().id, id);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn dirty_shared_settings_buffer_refuses_public_write_without_losing_model_history_or_disk() {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path());
    let path = root.path().join("profile/settings.json");
    let disk = std::fs::read(&path).unwrap();
    app.open(&path).unwrap();
    app.execute("workbench.action.splitEditor", Value::Null);
    let id = app.doc().id;
    let original = app.doc().text.to_string();
    app.doc_mut().insert("dirty ", true);
    let dirty = app.doc().text.to_string();
    app.execute("fixture.configuration.refused", Value::Null);
    until(&mut app, |app| {
        app.message == "Dirty configuration preserved"
    });
    assert_eq!(std::fs::read(path).unwrap(), disk);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), dirty);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), dirty);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn open_clean_settings_mirrors_and_native_undo_are_current_before_public_configuration_success() {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path());
    let path = root.path().join("profile/settings.json");
    app.open(&path).unwrap();
    app.execute("workbench.action.splitEditor", Value::Null);
    let id = app.doc().id;
    let original = app.doc().text.to_string();
    app.execute("fixture.configuration.clean", Value::Null);
    until(&mut app, |app| {
        app.message == "Open configuration refreshed"
    });
    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("after-clean"));
    assert_eq!(app.doc().text.to_string(), updated);
    assert_eq!(app.doc().id, id);
    assert!(!app.doc().dirty());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), updated);
    assert!(!app.doc().dirty());
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn api_hidden_clean_settings_are_refreshed_without_showing_or_redirecting_the_active_document() {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path());
    let id = app.doc().id;
    let original = app.doc().text.to_string();
    let path = root.path().join("profile/settings.json");
    app.execute("fixture.configuration.clean", serde_json::json!(path));
    until(&mut app, |app| {
        app.message == "Open configuration refreshed"
    });
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), original);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("after-clean")
    );
    app.execute("vscli.extensions.stop", Value::Null);
}

#[cfg(unix)]
#[test]
fn atomic_settings_replacement_does_not_redirect_a_different_hardlinked_resource_or_its_clean_model()
 {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path());
    let path = root.path().join("profile/settings.json");
    let alias = root.path().join("hardlink.json");
    std::fs::hard_link(&path, &alias).unwrap();
    let original = std::fs::read_to_string(&alias).unwrap();
    app.open(&alias).unwrap();
    let id = app.doc().id;
    app.execute("fixture.configuration.roundtrip", Value::Null);
    until(&mut app, |app| {
        app.message == "Configuration roundtrip persisted"
    });
    assert_eq!(std::fs::read_to_string(&alias).unwrap(), original);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(app.doc().id, id);
    assert!(!app.doc().dirty());
    assert!(std::fs::read_to_string(path).unwrap().contains("null"));
    app.execute("vscli.extensions.stop", Value::Null);
}
