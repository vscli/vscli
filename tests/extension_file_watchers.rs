use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Client, keys::Profile};
fn until(app: &mut App, check: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn public_os_watcher_reports_external_create_change_delete_and_disposal_preserves_native_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let source = root.join("a.txt");
    let original = "猫🙂 original\r\n";
    std::fs::write(&source, original).unwrap();
    let settings = root.join("settings.json");
    std::fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,
            "editor.codeActionsOnSave":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.clone(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&source).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|d| d.path.as_ref() == Some(&source))
    });
    app.doc_mut().insert("dirty ", false);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let extension =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/file-watcher-extension");
    let node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.extension_host = Some(
        Client::start(
            &node,
            &extension,
            &root,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|h| h.ready && h.commands.iter().any(|(_, id)| id == "watch.arm"))
    });
    app.execute("watch.arm", Value::Null);
    until(&mut app, |app| app.message == "API watching files");
    // The public API is synchronous while OS subscription startup is asynchronous.
    // Create fresh external paths until an actual subscribed event establishes readiness.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut index = 0;
    let observed = loop {
        assert!(
            Instant::now() < deadline,
            "No actual OS event: {}",
            app.message
        );
        let name = format!("watch-{index}.rs");
        index += 1;
        std::fs::write(root.join(&name), "猫 created\r\n").unwrap();
        let next = Instant::now() + Duration::from_millis(50);
        while Instant::now() < next {
            app.poll();
            if app.message.starts_with("API created: ") {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if let Some(name) = app.message.strip_prefix("API created: ") {
            break name.to_owned();
        }
    };
    let watched = root.join(&observed);
    app.execute("watch.change", Value::Null);
    until(&mut app, |app| {
        app.message == "API waiting changed"
            && app.extension_host.as_ref().is_some_and(|host| !host.busy())
    });
    std::fs::write(root.join("watch-queued.rs"), "unrelated create\r\n").unwrap();
    std::fs::write(&watched, "🙂 changed\r\n").unwrap();
    until(&mut app, |app| {
        app.message == format!("API changed: {observed}")
    });
    app.execute("watch.delete", Value::Null);
    until(&mut app, |app| {
        app.message == "API waiting deleted"
            && app.extension_host.as_ref().is_some_and(|host| !host.busy())
    });
    std::fs::remove_file(&watched).unwrap();
    until(&mut app, |app| {
        app.message == format!("API deleted: {observed}")
    });
    app.execute("watch.dispose", Value::Null);
    until(&mut app, |app| app.message == "API watcher disposed");
    std::fs::write(root.join("watch-after.rs"), "late\r\n").unwrap();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    app.execute("watch.verify", Value::Null);
    until(&mut app, |app| {
        app.message == "API disposed watcher stayed silent"
    });
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), format!("dirty {original}"));
    assert_eq!(std::fs::read(&source).unwrap(), original.as_bytes());
    assert_eq!(app.documents.len(), 1);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), original);
    assert!(!app.doc().dirty());
    app.execute("watch.clean", Value::Null);
    until(&mut app, |app| {
        app.message == "API watcher retained native Undo"
    });
    app.execute("vscli.extensions.stop", Value::Null);
}
