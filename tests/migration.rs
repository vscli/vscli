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
