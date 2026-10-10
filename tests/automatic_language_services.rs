use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile};
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settle(app: &mut App) {
    let end = Instant::now() + Duration::from_millis(180);
    while Instant::now() < end {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn configure(app: &mut App, root: &Path) -> PathBuf {
    let user = root.join("user.json");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auto_language_server.py");
    let settings = json!({
        "[cpp][c]":{"vscli.languageServer.program":"python3", "vscli.languageServer.args":[fixture, root.join("cpp.pid")]},
        "[rust]":{"vscli.languageServer.program":"python3", "vscli.languageServer.args":[fixture, root.join("rust.pid")]}
    });
    std::fs::write(&user, serde_json::to_vec(&settings).unwrap()).unwrap();
    app.configure_settings(Some(user.clone())).unwrap();
    user
}
#[test]
fn automatic_start_language_transitions_crash_retry_and_disable_preserve_shared_edits() {
    let root = tempfile::tempdir().unwrap();
    let cpp = root.path().join("main.cpp");
    let rust = root.path().join("main.rs");
    std::fs::write(&cpp, "sum(1, 2)\r\n").unwrap();
    std::fs::write(&rust, "fn main() {}\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    configure(&mut app, root.path());
    app.configure_language_services(None, false).unwrap();
    settle(&mut app);
    assert!(app.documents.is_empty());
    assert!(!root.path().join("cpp.pid").exists());
    app.open(&cpp).unwrap();
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    let id = app.doc().id;
    app.execute("workbench.action.splitEditorRight", Value::Null);
    app.doc_mut().insert("//", false);
    app.open(&rust).unwrap();
    until(&mut app, |a| {
        a.lsp.as_ref().is_some_and(|c| c.ready) && root.path().join("rust.pid").exists()
    });
    assert_eq!(
        app.documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .text
            .to_string(),
        "//sum(1, 2)\r\n"
    );
    app.open(&cpp).unwrap();
    until(&mut app, |a| {
        a.lsp.as_ref().is_some_and(|c| c.ready)
            && std::fs::read_to_string(root.path().join("cpp.pid"))
                .unwrap()
                .lines()
                .count()
                == 2
    });
    std::fs::write(root.path().join("cpp.crash"), "").unwrap();
    until(&mut app, |a| a.lsp.is_none());
    settle(&mut app);
    assert_eq!(
        std::fs::read_to_string(root.path().join("cpp.pid"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    std::fs::remove_file(root.path().join("cpp.crash")).unwrap();
    app.execute("vscli.languageServer.restart", Value::Null);
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    app.execute("vscli.languageServer.disable", Value::Null);
    assert!(app.lsp.is_none());
    settle(&mut app);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(std::fs::read(&cpp).unwrap(), b"//sum(1, 2)\r\n");
    app.execute("undo", Value::Null);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(std::fs::read(&cpp).unwrap(), b"sum(1, 2)\r\n");
    app.execute("vscli.languageServer.enable", Value::Null);
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    drop(app);
    #[cfg(unix)]
    for marker in ["cpp.pid", "rust.pid"] {
        for pid in std::fs::read_to_string(root.path().join(marker))
            .unwrap()
            .lines()
        {
            let pid: i32 = pid.parse().unwrap();
            // The fixture records our direct children; shutdown must reap them.
            assert_eq!(
                unsafe { libc::kill(pid, 0) },
                -1,
                "server {pid} survived shutdown"
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
}
#[test]
fn failed_discovery_retries_only_after_explicit_restart_or_settings_change() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    std::fs::write(&path, "sum(1, 2)\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let settings = root.path().join("missing.json");
    std::fs::write(
        &settings,
        r#"{"vscli.languageServer.program":"vscli-missing-server-fixture"}"#,
    )
    .unwrap();
    app.configure_settings(Some(settings)).unwrap();
    app.configure_language_services(None, false).unwrap();
    until(&mut app, |a| a.message.contains("not installed"));
    app.message = "kept status".into();
    settle(&mut app);
    assert_eq!(app.message, "kept status");
    configure(&mut app, root.path());
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    app.doc_mut().move_to(7, false);
    app.execute("editor.action.triggerParameterHints", Value::Null);
    until(&mut app, |a| a.signature_help().is_some());
    app.execute("vscli.languageServer.restart", Value::Null);
    assert!(app.signature_help().is_none());
    assert!(!app.doc().dirty());
}
#[test]
fn manual_override_ignores_language_selection_and_automatic_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    let settings = root.path().join("disabled.json");
    std::fs::write(&settings, r#"{"vscli.languageServer.enabled":false}"#).unwrap();
    app.configure_settings(Some(settings)).unwrap();
    let launch = vscli::language_services::Launch {
        workspace: root.path().into(),
        program: "python3".into(),
        language: "cpp".into(),
        args: vec![
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/auto_language_server.py")
                .to_string_lossy()
                .into_owned(),
            root.path()
                .join("manual.pid")
                .to_string_lossy()
                .into_owned(),
        ],
    };
    app.configure_language_services(Some(launch), false)
        .unwrap();
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    assert!(app.documents.is_empty());
    app.execute("vscli.languageServer.status", Value::Null);
    assert!(app.message.contains("python3 (cpp) · ready"));
}

#[test]
fn modal_focus_change_rejects_offered_start_and_retry_keeps_current_dirty_document() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    std::fs::write(&path, "sum(1, 2)\r\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    configure(&mut app, root.path());
    app.open(&path).unwrap();
    app.configure_language_services(None, false).unwrap();
    app.poll(); // Submit startup; deliberately do not consume its offer yet.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("cpp.pid").exists() {
        assert!(Instant::now() < deadline, "background server did not start");
        std::thread::sleep(Duration::from_millis(2));
    }
    app.execute("workbench.action.showCommands", Value::Null);
    settle(&mut app);
    assert!(root.path().join("cpp.pid").exists());
    assert!(app.lsp.is_none());
    assert!(app.prompt.is_some());
    app.event(crossterm::event::Event::Key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ),
    ));
    app.doc_mut().insert("//", false);
    let id = app.doc().id;
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "//sum(1, 2)\r\n");
    assert_eq!(
        std::fs::read_to_string(root.path().join("cpp.pid"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"sum(1, 2)\r\n");
}

#[test]
#[ignore = "requires installed clangd; automatic C++ qualification"]
fn real_clangd_starts_for_cpp_without_manual_configuration() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    let text = "int sum(int left, int right);\nint main() { return sum(1, 2); }\n";
    std::fs::write(&path, text).unwrap();
    std::fs::write(root.path().join("compile_flags.txt"), "-std=c++17\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.configure_language_services(None, false).unwrap();
    settle(&mut app);
    assert!(app.lsp.is_none());
    app.open(&path).unwrap();
    app.doc_mut().move_to(text.find("1, 2").unwrap() + 3, false);
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    app.execute("editor.action.triggerParameterHints", Value::Null);
    until(&mut app, |a| a.signature_help().is_some());
    let hint = app.signature_help().unwrap();
    assert!(hint.label.contains("sum"));
    assert!(&hint.label[hint.parameter.clone().unwrap()].contains("right"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert!(!app.doc().dirty());
}

#[test]
fn repository_program_arguments_need_user_opt_in_and_reload_preserves_buffer_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.cpp");
    std::fs::write(&path, "original\r\n").unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    let user = configure(&mut app, root.path());
    std::fs::create_dir(root.path().join(".vscode")).unwrap();
    let marker = root.path().join("repository.pid");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auto_language_server.py");
    std::fs::write(root.path().join(".vscode/settings.json"), serde_json::to_vec(&json!({
        "vscli.languageServer.allowWorkspaceConfiguration": true,
        "[cpp]": {"vscli.languageServer.program":"python3", "vscli.languageServer.args":[fixture, marker]}
    })).unwrap()).unwrap();
    app.configure_settings(Some(user.clone())).unwrap();
    app.open(&path).unwrap();
    let id = app.doc().id;
    app.doc_mut().insert("//", false);
    app.configure_language_services(None, false).unwrap();
    until(&mut app, |a| a.lsp.as_ref().is_some_and(|c| c.ready));
    assert!(root.path().join("cpp.pid").exists());
    assert!(
        !marker.exists(),
        "repository enabled its own executable configuration"
    );
    let mut values: Value = serde_json::from_slice(&std::fs::read(&user).unwrap()).unwrap();
    values["vscli.languageServer.allowWorkspaceConfiguration"] = json!(true);
    std::fs::write(&user, serde_json::to_vec(&values).unwrap()).unwrap();
    app.configure_settings(Some(user)).unwrap();
    until(&mut app, |a| {
        marker.exists() && a.lsp.as_ref().is_some_and(|c| c.ready)
    });
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "//original\r\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"original\r\n");
}
