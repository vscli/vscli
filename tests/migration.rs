use std::{fs, process::Command};
#[test]
fn cli_preview_activation_and_partial_binding_import_preserve_source_bytes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("User");
    let destination = root.path().join("native");
    fs::create_dir_all(source.join("snippets")).unwrap();
    let settings = "{// original\n\"editor.tabSize\":2,\"extension.unknown\":true,}";
    fs::write(source.join("settings.json"), settings).unwrap();
    let keys = r#"[{"key":"f6","command":"type","args":{"text":"imported"}},{"key":"f7","command":"type","when":"foo === bar ? yes : no"}]"#;
    fs::write(source.join("keybindings.json"), keys).unwrap();
    fs::write(
        source.join("snippets/global.code-snippets"),
        r#"{"Example":{"body":"$0"}}"#,
    )
    .unwrap();
    let run = |apply| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vscli"));
        command
            .arg("--import-vscode")
            .arg(&source)
            .arg("--vscode-extensions")
            .arg(root.path().join("source-extensions"))
            .arg("--config-dir")
            .arg(&destination);
        if apply {
            command.arg("--apply-import");
        }
        command.output().unwrap()
    };
    let preview = run(false);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(!destination.exists());
    let report: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(report["files"].as_array().unwrap().len(), 3);
    assert!(report["notices"].to_string().contains("skipped"));
    let applied = run(true);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let active = vscli::migration::active_directory(&destination).unwrap();
    assert_eq!(
        fs::read_to_string(active.join("settings.json")).unwrap(),
        settings
    );
    assert_eq!(
        fs::read_to_string(active.join("keybindings.json")).unwrap(),
        keys
    );
    assert_eq!(
        fs::read_to_string(source.join("settings.json")).unwrap(),
        settings
    );
    let mut map = vscli::keys::Keymap::new(vscli::keys::Profile::Linux);
    let (count, skipped) = map.load_imported(&active.join("keybindings.json")).unwrap();
    assert_eq!(count, 1);
    assert_eq!(skipped.len(), 1);
    let old = fs::read(destination.join("active-profile.json")).unwrap();
    fs::write(source.join("settings.json"), "{broken").unwrap();
    assert!(!run(true).status.success());
    assert_eq!(
        fs::read(destination.join("active-profile.json")).unwrap(),
        old
    );
}

#[test]
fn local_package_import_uses_selected_native_store_and_rollback_restores_original_configuration() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("User");
    let extensions = root.path().join("extensions");
    let package = extensions.join("fixture.native-1.0.0");
    let native = root.path().join("native");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&package).unwrap();
    fs::write(
        source.join("settings.json"),
        "{// keep bytes\n\"editor.tabSize\":2,}",
    )
    .unwrap();
    fs::write(package.join("package.json"),r#"{"publisher":"fixture","name":"native","version":"1.0.0","main":"main.cjs","engines":{"vscode":"^1.95.0"}}"#).unwrap();
    fs::write(
        package.join("main.cjs"),
        "throw new Error('must not execute')",
    )
    .unwrap();
    fs::write(extensions.join("extensions.json"),serde_json::to_vec(&serde_json::json!([{"identifier":{"id":"fixture.native"},"version":"1.0.0","relativeLocation":"fixture.native-1.0.0","location":{"scheme":"file","path":package}}])).unwrap()).unwrap();
    let imported = Command::new(env!("CARGO_BIN_EXE_vscli"))
        .args(["--import-vscode"])
        .arg(&source)
        .arg("--vscode-extensions")
        .arg(&extensions)
        .arg("--config-dir")
        .arg(&native)
        .args(["--import-extensions", "--apply-import"])
        .output()
        .unwrap();
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(report["extensions_imported"], 1);
    assert_eq!(report["extensions_execution_enabled"], 0);
    let listed = Command::new(env!("CARGO_BIN_EXE_vscli"))
        .arg("--config-dir")
        .arg(&native)
        .arg("--list-extensions")
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let packages: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(packages[0]["id"], "fixture.native");
    assert_eq!(packages[0]["version"], "1.0.0");
    let rollback = Command::new(env!("CARGO_BIN_EXE_vscli"))
        .arg("--config-dir")
        .arg(&native)
        .arg("--rollback-import")
        .output()
        .unwrap();
    assert!(
        rollback.status.success(),
        "{}",
        String::from_utf8_lossy(&rollback.stderr)
    );
    assert_eq!(vscli::migration::active_directory(&native).unwrap(), native);
    assert_eq!(
        fs::read_to_string(source.join("settings.json")).unwrap(),
        "{// keep bytes\n\"editor.tabSize\":2,}"
    );
}

#[test]
fn exported_profile_cli_preview_configuration_apply_and_unresolved_packages_keep_atomic_state() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("setup.code-profile");
    let native = root.path().join("native");
    let settings = "{// export comments\r\n\"editor.tabSize\":2,\"unknown.flag\":true,}\r\n";
    let export = serde_json::json!({"name":"Export λ🙂","settings":serde_json::json!({"settings":settings}).to_string(),"extensions":"[{\"identifier\":{\"id\":\"fixture.missing\"},\"version\":\"1.0.0\"}]","future":{"retained":true}}).to_string();
    fs::write(&source, &export).unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_vscli"))
            .arg("--import-vscode")
            .arg(&source)
            .arg("--vscode-extensions")
            .arg(root.path().join("missing-extensions"))
            .arg("--extension-node")
            .arg(root.path().join("no-node"))
            .arg("--config-dir")
            .arg(&native)
            .args(extra)
            .output()
            .unwrap()
    };
    let preview = run(&[]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(report["profile"]["name"], "Export λ🙂");
    assert_eq!(report["unresolved_extensions"].as_array().unwrap().len(), 1);
    assert!(!native.exists());
    assert!(!run(&["--vscode-profile", "ignored"]).status.success());
    let blocked = run(&["--apply-import", "--import-extensions"]);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("fixture.missing@1.0.0"));
    assert!(!native.exists());
    let applied = run(&["--apply-import"]);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let active = vscli::migration::active_directory(&native).unwrap();
    assert_eq!(
        fs::read(active.join("settings.json")).unwrap(),
        settings.as_bytes()
    );
    assert_eq!(
        fs::read(active.join("source.code-profile")).unwrap(),
        export.as_bytes()
    );
    let pointer = fs::read(native.join("active-profile.json")).unwrap();
    assert!(
        !run(&["--apply-import", "--import-extensions"])
            .status
            .success()
    );
    assert_eq!(
        fs::read(native.join("active-profile.json")).unwrap(),
        pointer
    );
    assert_eq!(fs::read(&source).unwrap(), export.as_bytes());
}
