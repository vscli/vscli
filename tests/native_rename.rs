//! Real no-Node LSP preparation and grouped rename, with capability receipts.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, PromptKind},
    keys::Profile,
    lsp::Client,
};
const SOURCE: &str = "猫🙂 source\r\nnext\r\n";
const DIRTY: &str = "dirty 猫🙂 source\r\nnext\r\n";
const RENAMED: &str = "dirty 猫🙂 renamed\r\nnext\r\n";
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Native rename {phase}: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn ready(root: &Path, mode: &str) -> App {
    std::fs::write(root.join("source.cpp"), SOURCE).unwrap();
    let config = root.join("settings.json");
    std::fs::write(&config,r#"{"vscli.languageServer.enabled":false,"files.autoSave":"off","files.refactoring.autoSave":false,"editor.formatOnSave":false,"breadcrumbs.enabled":false}"#).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(config)).unwrap();
    app.open(&root.join("source.cpp")).unwrap();
    until(&mut app, "source open", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("source.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    app.doc_mut().move_to(10, false);
    let python = std::env::var("VSCLI_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let server = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rename_server.py");
    app.lsp = Some(
        Client::start(
            &python,
            &[
                server.to_string_lossy().into_owned(),
                mode.into(),
                root.join("requests.jsonl").to_string_lossy().into_owned(),
            ],
            root,
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, "framed server ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    assert!(app.extension_host.is_none());
    app
}
fn methods(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("requests.jsonl"))
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .into()
        })
        .collect()
}

#[test]
fn no_node_original_f2_honors_boolean_range_placeholder_and_default_behavior_server_capabilities() {
    for mode in ["no-prepare", "range", "placeholder", "default"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut app = ready(&root, mode);
        let id = app.doc().id;
        let member = app.editor_groups().active_membership().unwrap();
        let generation = app
            .doc()
            .capture_save(root.join("source.cpp"))
            .unwrap()
            .save_generation();
        key(&mut app, KeyCode::F(2));
        let placeholder = if mode == "placeholder" {
            "Original source label"
        } else {
            "source"
        };
        until(&mut app, "source-derived prepared prompt", |app| {
            matches!(
                app.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Rename)
            ) && app.prompt.as_ref().unwrap().text == placeholder
        });
        assert_eq!(app.doc().text.to_string(), DIRTY);
        assert_eq!(app.doc().id, id);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        )));
        app.event(Event::Paste("renamed".into()));
        key(&mut app, KeyCode::Enter);
        until(&mut app, "actual native transaction publication", |app| {
            app.prompt.is_none() && app.doc().text == RENAMED
        });
        assert!(app.extension_host.is_none());
        assert_eq!(app.editor_groups().active_membership(), Some(member));
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        assert_eq!(
            app.doc()
                .capture_save(root.join("source.cpp"))
                .unwrap()
                .save_generation(),
            generation
        );
        assert_eq!(
            std::fs::read(root.join("source.cpp")).unwrap(),
            SOURCE.as_bytes()
        );
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), DIRTY);
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), RENAMED);
        app.execute("undo", Value::Null);
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), SOURCE);
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), DIRTY);
        assert_eq!(app.doc().id, id);
        let trace = methods(&root);
        assert_eq!(
            trace
                .iter()
                .filter(|method| method.as_str() == "textDocument/prepareRename")
                .count(),
            usize::from(mode != "no-prepare")
        );
        assert_eq!(
            trace
                .iter()
                .filter(|method| method.as_str() == "textDocument/rename")
                .count(),
            1
        );
        assert_eq!(
            std::fs::read(root.join("source.cpp")).unwrap(),
            SOURCE.as_bytes()
        );
    }
}

#[test]
fn no_node_null_or_error_preparation_refuses_before_name_or_edit_and_preserves_original_histories()
{
    for mode in ["null", "error"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut app = ready(&root, mode);
        let id = app.doc().id;
        let selections = app.doc().selections();
        let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
        key(&mut app, KeyCode::F(2));
        until(&mut app, "actual server preparation refusal", |app| {
            app.message.contains(if mode == "null" {
                "language server rejected rename"
            } else {
                "rename rejected by fixture"
            })
        });
        assert!(app.prompt.is_none());
        assert!(app.modal.is_none());
        assert!(app.extension_host.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), DIRTY);
        assert_eq!(app.doc().selections(), selections);
        let current = app.doc().capture_save(root.join("source.cpp")).unwrap();
        assert_eq!(current.text_epoch(), proof.text_epoch());
        assert_eq!(current.save_generation(), proof.save_generation());
        let trace = methods(&root);
        assert_eq!(
            trace
                .iter()
                .filter(|method| method.as_str() == "textDocument/prepareRename")
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|method| method.as_str() == "textDocument/rename")
                .count(),
            0
        );
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), SOURCE);
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), DIRTY);
        assert_eq!(
            std::fs::read(root.join("source.cpp")).unwrap(),
            SOURCE.as_bytes()
        );
    }
}
