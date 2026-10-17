use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

fn fixture(root: &Path) -> discovery::Source {
    let user = root.join("User");
    let extensions = root.join("source-extensions");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(extensions.join("fixture.native-1.0.0")).unwrap();
    fs::write(
        user.join("settings.json"),
        "{// original\n\"editor.tabSize\":2,}",
    )
    .unwrap();
    fs::write(
        user.join("keybindings.json"),
        r#"[{"key":"f6","command":"type","args":{"text":"imported"}}]"#,
    )
    .unwrap();
    fs::write(extensions.join("fixture.native-1.0.0/package.json"),r#"{"publisher":"fixture","name":"native","version":"1.0.0","main":"main.cjs","engines":{"vscode":"^1.95.0"}}"#).unwrap();
    fs::write(
        extensions.join("fixture.native-1.0.0/main.cjs"),
        "throw new Error('code must not execute during migration')",
    )
    .unwrap();
    fs::write(extensions.join("extensions.json"),serde_json::to_vec(&json!([{"identifier":{"id":"fixture.native"},"version":"1.0.0","relativeLocation":"fixture.native-1.0.0","location":{"scheme":"file","path":extensions.join("fixture.native-1.0.0")}}])).unwrap()).unwrap();
    discovery::default_source("Fixture", &user, &extensions).unwrap()
}
#[test]
fn config_and_exact_packages_publish_together_with_explicit_grants_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let native = root.path().join("native");
    fs::create_dir(&native).unwrap();
    fs::write(native.join("settings.json"), "native original").unwrap();
    let options = ApplyOptions {
        import_extensions: true,
        activate_extensions: false,
    };
    let first = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(&native, options, &AtomicBool::new(false), &mut |_, _, _| {})
        .unwrap();
    let directory = first.activated_profile.unwrap();
    assert_eq!(first.extensions_imported, 1);
    assert_eq!(first.extensions_execution_enabled, 0);
    assert_eq!(
        fs::read(directory.join("settings.json")).unwrap(),
        fs::read(source.user.join("settings.json")).unwrap()
    );
    let store =
        crate::extension_store::Store::new(imported_extensions_directory(&directory).unwrap());
    assert_eq!(store.list().unwrap()[0].version, "1.0.0");
    assert!(
        !crate::extension_activation::state::read(&directory.join("extensions-enabled.json"))
            .unwrap()
            .extensions["fixture.native"]
    );
    let second = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(
            &native,
            ApplyOptions {
                activate_extensions: true,
                ..options
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    assert_eq!(second.extensions_execution_enabled, 1);
    assert_eq!(rollback(&native).unwrap(), directory);
    assert_eq!(active_directory(&native).unwrap(), directory);
    assert!(second.activated_profile.unwrap().exists());
    assert_eq!(rollback(&native).unwrap(), native);
    assert_eq!(
        fs::read_to_string(native.join("settings.json")).unwrap(),
        "native original"
    );
    assert!(store.get("fixture.native").is_ok());
}
#[test]
fn cancellation_after_copy_does_not_activate_or_remove_existing_profile() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let native = root.path().join("native");
    let first = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply(&native)
        .unwrap();
    let pointer = fs::read(native.join("active-profile.json")).unwrap();
    let cancel = AtomicBool::new(false);
    let result = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(
            &native,
            ApplyOptions {
                import_extensions: true,
                activate_extensions: true,
            },
            &cancel,
            &mut |_, _, _| cancel.store(true, Ordering::Release),
        );
    assert!(result.is_err());
    assert_eq!(
        fs::read(native.join("active-profile.json")).unwrap(),
        pointer
    );
    assert!(first.activated_profile.unwrap().is_dir());
    assert!(fs::read_dir(native.join("imports")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".staging-")
    }));
}
#[test]
fn named_profile_resource_inheritance_copies_correct_bytes_without_modifying_sources() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path());
    let named = source.user.join("profiles/named");
    fs::create_dir_all(&named).unwrap();
    fs::write(named.join("keybindings.json"), "[]").unwrap();
    source.user = named;
    source.id = "named".into();
    source
        .use_default
        .insert("settings".into(), Value::Bool(true));
    source
        .use_default
        .insert("extensions".into(), Value::Bool(true));
    let preview = preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let report = preview.apply(&root.path().join("native")).unwrap();
    let active = report.activated_profile.unwrap();
    assert_eq!(
        fs::read_to_string(active.join("keybindings.json")).unwrap(),
        "[]"
    );
    assert_eq!(
        fs::read(active.join("settings.json")).unwrap(),
        fs::read(source.default_user.join("settings.json")).unwrap()
    );
    assert_eq!(report.extensions.unwrap().selected.len(), 1);
}
