//! Native settings writes through public App commands and real profile copies.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile, migration};

const TOGGLE: &str = "breadcrumbs.toggle";
const ORIGINAL: &str = "{\r\n  // 猫🙂 imported settings remain in their original order\r\n  \"breadcrumbs.enabled\": true, // keep this comment\r\n  \"vscli.languageServer.enabled\": false,\r\n  \"extension.unknown\": {\"breadcrumbs.enabled\": \"nested unchanged\"},\r\n}\r\n";

fn execute(app: &mut App, command: &str) {
    app.execute(command, Value::Null);
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
fn configured(root: &Path, user: &Path, file: &Path) -> App {
    let mut app = App::new(root.into(), Profile::Linux);
    app.extension_node = root.join("node-unavailable").to_string_lossy().into_owned();
    app.configure_settings(Some(user.into())).unwrap();
    app.open(file).unwrap();
    assert!(app.extension_host.is_none());
    assert!(app.lsp.is_none());
    app
}
fn files(root: &Path) -> (PathBuf, PathBuf) {
    let user = root.join("native-settings.json");
    fs::write(&user, ORIGINAL).unwrap();
    let file = root.join("main.cpp");
    fs::write(&file, "猫🙂 original\r\n").unwrap();
    (user, file)
}
fn saved(app: &mut App, path: &Path, expected: &str, enabled: bool) {
    until(
        app,
        "write committed and forced settings reload settled",
        |app| {
            fs::read(path).is_ok_and(|bytes| bytes == expected.as_bytes())
                && app.settings.breadcrumbs("cpp").enabled == enabled
                && app.breadcrumbs_view().visible == enabled
                && !app.message.contains("Saving Breadcrumbs")
        },
    );
    assert!(app.extension_host.is_none());
    assert!(app.lsp.is_none());
}

#[test]
fn imported_profile_toggle_preserves_jsonc_bytes_and_original_source_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let source = root.join("original-vscode-user");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("settings.json"), ORIGINAL).unwrap();
    let keybindings = "[\r\n // original 猫🙂\r\n]\r\n";
    fs::write(source.join("keybindings.json"), keybindings).unwrap();
    let profile_root = root.join("native-profile");
    let report = migration::preview(&source, Profile::Linux)
        .unwrap()
        .apply(&profile_root)
        .unwrap();
    let user = report.activated_profile.unwrap().join("settings.json");
    let file = root.join("main.cpp");
    fs::write(&file, "猫🙂 original\r\n").unwrap();
    let mut app = configured(&root, &user, &file);
    app.doc_mut().move_to(2, false);
    app.doc_mut().insert(" DIRTY", false);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let dirty = app.doc().text.to_string();
    assert!(app.breadcrumbs_view().visible);
    execute(&mut app, TOGGLE);
    let expected = ORIGINAL.replacen(
        "\"breadcrumbs.enabled\": true",
        "\"breadcrumbs.enabled\": false",
        1,
    );
    saved(&mut app, &user, &expected, false);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert!(app.doc().dirty());
    assert_eq!(fs::read(&file).unwrap(), "猫🙂 original\r\n".as_bytes());
    execute(&mut app, "undo");
    assert_eq!(app.doc().text.to_string(), "猫🙂 original\r\n");
    execute(&mut app, "redo");
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(
        fs::read(source.join("settings.json")).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert_eq!(
        fs::read(source.join("keybindings.json")).unwrap(),
        keybindings.as_bytes()
    );
    drop(app);
    let active = migration::active_directory(&profile_root).unwrap();
    let restarted = configured(&root, &active.join("settings.json"), &file);
    assert!(!restarted.settings.breadcrumbs("cpp").enabled);
    assert!(!restarted.breadcrumbs_view().visible);
    assert_eq!(
        fs::read(source.join("settings.json")).unwrap(),
        ORIGINAL.as_bytes()
    );
}

#[test]
fn workspace_root_winner_is_patched_without_changing_user_or_language_siblings() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    fs::create_dir(root.join(".vscode")).unwrap();
    let workspace = root.join(".vscode/settings.json");
    let original = "{\r\n // workspace 猫🙂\r\n \"breadcrumbs.enabled\": true,\r\n \"[python]\": {\"breadcrumbs.enabled\": false},\r\n \"unknown\": 0x12,\r\n}\r\n";
    fs::write(&workspace, original).unwrap();
    let mut app = configured(&root, &user, &file);
    execute(&mut app, TOGGLE);
    let expected = original.replacen(
        "\"breadcrumbs.enabled\": true",
        "\"breadcrumbs.enabled\": false",
        1,
    );
    saved(&mut app, &workspace, &expected, false);
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
    assert!(!app.settings.breadcrumbs("python").enabled);
    drop(app);
    let restarted = configured(&root, &user, &file);
    assert!(!restarted.breadcrumbs_view().visible);
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
}

#[test]
fn rapid_toggles_keep_only_the_latest_boolean_and_reload_before_restart() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    let mut app = configured(&root, &user, &file);
    for index in 0..33 {
        execute(&mut app, TOGGLE);
        assert_eq!(app.breadcrumbs_view().visible, index % 2 == 1);
    }
    // No poll has authorized even the first preparation.
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
    let expected = ORIGINAL.replacen(
        "\"breadcrumbs.enabled\": true",
        "\"breadcrumbs.enabled\": false",
        1,
    );
    saved(&mut app, &user, &expected, false);
    drop(app);
    assert!(!configured(&root, &user, &file).breadcrumbs_view().visible);
}

#[test]
fn equal_value_language_override_refuses_without_an_optimistic_presentation_change() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    let original = "{\r\n \"vscli.languageServer.enabled\": false,\r\n \"breadcrumbs.enabled\": true,\r\n \"[cpp][rust]\": {\"breadcrumbs.enabled\": false},\r\n \"[cpp]\": {\"breadcrumbs.enabled\": true}, // equal to root\r\n}\r\n";
    fs::write(&user, original).unwrap();
    let mut app = configured(&root, &user, &file);
    execute(&mut app, TOGGLE);
    assert!(
        app.message.contains("overridden by [cpp]"),
        "{}",
        app.message
    );
    assert!(app.breadcrumbs_view().visible);
    assert_eq!(fs::read(&user).unwrap(), original.as_bytes());
    assert!(app.settings.breadcrumbs("cpp").enabled);
    assert!(!app.settings.breadcrumbs("rust").enabled);
}

#[test]
fn dirty_shared_settings_buffer_refuses_without_mutating_views_history_or_disk() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    let mut app = configured(&root, &user, &file);
    app.open(&user).unwrap();
    app.doc_mut().move_to(4, false);
    app.doc_mut().insert(" DIRTY猫🙂", false);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let saved_revision = app.doc().saved_revision;
    let dirty = app.doc().text.to_string();
    let baseline = app.doc().disk_content.as_ref().unwrap().to_string();
    execute(&mut app, "workbench.action.splitEditorRight");
    let views: Vec<_> = app
        .panes
        .iter()
        .map(|pane| {
            let view = app.doc().view_state(Some(pane.id));
            (pane.id, view.cursor, view.anchor, view.secondary.clone())
        })
        .collect();
    assert_eq!(views.len(), 2);
    execute(&mut app, TOGGLE);
    assert!(app.message.contains("unsaved changes"), "{}", app.message);
    assert!(app.breadcrumbs_view().visible);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().saved_revision, saved_revision);
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(
        app.doc().disk_content.as_ref().unwrap().to_string(),
        baseline
    );
    for (pane, cursor, anchor, secondary) in views {
        let view = app.doc().view_state(Some(pane));
        assert_eq!(view.cursor, cursor);
        assert_eq!(view.anchor, anchor);
        assert_eq!(view.secondary, secondary);
    }
    execute(&mut app, "undo");
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    execute(&mut app, "redo");
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(app.doc().id, id);
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
}

#[cfg(unix)]
#[test]
fn canonical_parent_alias_cannot_bypass_dirty_target_authorization() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    let alias = root.join("parent-alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let alias_user = alias.join(user.file_name().unwrap());
    let mut app = configured(&root, &alias_user, &file);
    app.open(&user).unwrap();
    app.doc_mut().move_to(4, false);
    app.doc_mut().insert(" DIRTY猫🙂", false);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let dirty = app.doc().text.to_string();
    execute(&mut app, TOGGLE);
    until(&mut app, "canonical dirty target refused", |app| {
        app.message.contains("Settings write refused") && app.message.contains("unsaved changes")
    });
    assert!(app.breadcrumbs_view().visible);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), dirty);
    execute(&mut app, "undo");
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    execute(&mut app, "redo");
    assert_eq!(app.doc().text.to_string(), dirty);
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(fs::read(&alias_user).unwrap(), ORIGINAL.as_bytes());
}

#[test]
fn profile_a_b_a_retires_an_unapproved_write_then_accepts_a_fresh_intent() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    let other = root.join("other-profile.json");
    fs::write(&other, ORIGINAL).unwrap();
    let mut app = configured(&root, &user, &file);
    execute(&mut app, TOGGLE);
    assert!(!app.breadcrumbs_view().visible);
    app.configure_settings(Some(other.clone())).unwrap();
    app.configure_settings(Some(user.clone())).unwrap();
    assert!(app.breadcrumbs_view().visible);
    // The old worker may have prepared, but cannot have received authorization:
    // only App::poll can deliver it and the profile identity already changed.
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(fs::read(&other).unwrap(), ORIGINAL.as_bytes());
    execute(&mut app, TOGGLE);
    let expected = ORIGINAL.replacen(
        "\"breadcrumbs.enabled\": true",
        "\"breadcrumbs.enabled\": false",
        1,
    );
    saved(&mut app, &user, &expected, false);
    assert_eq!(fs::read(&other).unwrap(), ORIGINAL.as_bytes());
    drop(app);
    assert!(!configured(&root, &user, &file).breadcrumbs_view().visible);
}

#[test]
fn malformed_configured_profile_refuses_and_read_only_failure_restores_presentation() {
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let (user, file) = files(&root);
    let mut app = configured(&root, &user, &file);
    let malformed = "{\r\n // 猫🙂 must remain intact\r\n \"breadcrumbs.enabled\": [}\r\n";
    fs::write(&user, malformed).unwrap();
    assert!(app.configure_settings(Some(user.clone())).is_err());
    execute(&mut app, TOGGLE);
    assert!(app.message.contains("failed to load"), "{}", app.message);
    assert!(app.breadcrumbs_view().visible);
    assert_eq!(fs::read(&user).unwrap(), malformed.as_bytes());
    fs::write(&user, ORIGINAL).unwrap();
    app.configure_settings(Some(user.clone())).unwrap();
    let permissions = fs::metadata(&user).unwrap().permissions();
    let mut read_only = permissions.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&user, read_only).unwrap();
    execute(&mut app, TOGGLE);
    until(&mut app, "read-only write failed", |app| {
        app.message.contains("Settings write failed") && app.message.contains("read-only")
    });
    assert!(app.breadcrumbs_view().visible);
    assert!(app.settings.breadcrumbs("cpp").enabled);
    assert_eq!(fs::read(&user).unwrap(), ORIGINAL.as_bytes());
    assert!(fs::metadata(&user).unwrap().permissions().readonly());
    fs::set_permissions(&user, permissions).unwrap();
}
