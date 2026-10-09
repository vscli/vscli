use vscli::{
    keys::{Keymap, Profile},
    settings::Settings,
    tasks,
};

fn reject_nonregular(root: &std::path::Path) {
    let keybindings = root.join("keybindings.json");
    let mut keymap = Keymap::new(Profile::Linux);
    let previous = serde_json::to_value(&keymap.bindings).unwrap();
    assert!(
        keymap
            .load(&keybindings)
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
    assert_eq!(serde_json::to_value(&keymap.bindings).unwrap(), previous);
    assert!(
        Settings::load(&[root.join("settings.json")])
            .err()
            .unwrap()
            .to_string()
            .contains("regular file")
    );
    assert!(
        tasks::load(root)
            .err()
            .unwrap()
            .to_string()
            .contains("regular file")
    );
}

#[test]
fn configuration_directories_are_rejected_without_changing_bindings() {
    let directory = tempfile::tempdir().unwrap();
    for relative in ["keybindings.json", "settings.json", ".vscode/tasks.json"] {
        std::fs::create_dir_all(directory.path().join(relative)).unwrap();
    }
    reject_nonregular(directory.path());
}

#[cfg(unix)]
#[test]
fn configuration_fifos_are_rejected_without_waiting_for_a_writer() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".vscode")).unwrap();
    let paths = ["keybindings.json", "settings.json", ".vscode/tasks.json"]
        .map(|relative| directory.path().join(relative));
    assert!(
        std::process::Command::new("mkfifo")
            .args(paths)
            .status()
            .unwrap()
            .success()
    );
    let root = directory.path().to_owned();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        reject_nonregular(&root);
        let _ = sender.send(());
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("Configuration must reject FIFOs without waiting for a writer");
}

#[cfg(unix)]
#[test]
fn symlinks_to_regular_configuration_files_remain_supported() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".vscode")).unwrap();
    for (target, link, contents) in [
        ("keys-target.json", "keybindings.json", "[]"),
        ("settings-target.json", "settings.json", "{}"),
        (
            "tasks-target.json",
            ".vscode/tasks.json",
            r#"{"version":"2.0.0","tasks":[]}"#,
        ),
    ] {
        let target = directory.path().join(target);
        std::fs::write(&target, contents).unwrap();
        std::os::unix::fs::symlink(target, directory.path().join(link)).unwrap();
    }
    assert_eq!(
        Keymap::new(Profile::Linux)
            .load(&directory.path().join("keybindings.json"))
            .unwrap(),
        0
    );
    assert!(Settings::load(&[directory.path().join("settings.json")]).is_ok());
    assert!(tasks::load(directory.path()).unwrap().is_empty());
}
