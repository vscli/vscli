//! Actual optional-host providers exercise the native picker and document transactions.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Modal},
    extensions::Package,
    keys::Profile,
    lsp::Client,
};

const ORIGINAL: &str = "猫🙂 bad\r\n";
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "Timed out: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn activate(app: &mut App, names: &[&str]) {
    let packages = names
        .iter()
        .map(|name| {
            Package::read(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/fixtures/code-action-provider-{name}")),
            )
            .unwrap()
        })
        .collect();
    app.start_extension_packages(packages).unwrap();
    until(app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
}
fn ready(root: &Path, names: &[&str]) -> App {
    let path = root.join("main.md");
    std::fs::write(&path, ORIGINAL).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.open(&path).unwrap();
    activate(&mut app, names);
    until(&mut app, |app| !app.current_diagnostics().is_empty());
    select(&mut app);
    app
}
fn select(app: &mut App) {
    app.doc_mut().move_to(3, false);
    app.doc_mut().move_to(6, true);
}
fn actions(app: &mut App, command: &str) -> Vec<String> {
    app.execute(command, Value::Null);
    until(app, |app| matches!(app.modal, Some(Modal::Language { .. })));
    let Some(Modal::Language { items, .. }) = &app.modal else {
        unreachable!()
    };
    items.iter().map(|item| item.label.clone()).collect()
}
fn choose(app: &mut App, title: &str) {
    let Some(Modal::Language {
        items, selected, ..
    }) = &mut app.modal
    else {
        panic!("No actions: {}", app.message)
    };
    *selected = items
        .iter()
        .position(|item| item.label.starts_with(title))
        .unwrap_or_else(|| panic!("Missing {title}"));
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
}
fn escape(app: &mut App) {
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
}
fn wait_primary_publication(app: &mut App) {
    until(app, |app| {
        app.current_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "primary original diagnostic")
    });
}

#[test]
fn original_diagnostics_multiple_providers_and_failure_isolation_keep_native_actions() {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path(), &["primary", "secondary", "failing"]);
    app.doc_mut().select_all();
    app.doc_mut().insert("bad 猫🙂\r\n", false);
    until(&mut app, |app| {
        app.current_diagnostics().iter().any(|diagnostic| {
            diagnostic.message == "primary original diagnostic"
                && diagnostic.range.start.character == 0
        })
    });
    app.doc_mut().move_to(0, false);
    app.doc_mut().move_to(3, true);
    app.lsp = Some(
        Client::start(
            "python3",
            &[Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/code_action_server.py")
                .to_string_lossy()
                .into_owned()],
            root.path(),
            "markdown".into(),
        )
        .unwrap(),
    );
    until(&mut app, |app| {
        app.current_diagnostics()
            .iter()
            .any(|d| d.message == "Fix fixture")
    });
    actions(&mut app, "editor.action.quickFix");
    until(
        &mut app,
        |app| matches!(&app.modal, Some(Modal::Language { items, .. }) if items.iter().any(|row| row.label.starts_with("Fix selected text"))),
    );
    let labels = match &app.modal {
        Some(Modal::Language { items, .. }) => items
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        _ => unreachable!(),
    };
    for title in [
        "Primary Unicode fix",
        "Secondary independent fix",
        "Fix selected text",
        "Command must remain disabled",
    ] {
        assert!(
            labels.iter().any(|label| label.starts_with(title)),
            "{title}: {labels:?}"
        );
    }
    let observed: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("primary-observed.json")).unwrap())
            .unwrap();
    assert_eq!(observed["diagnostics"], 1);
    assert_eq!(observed["triggerKind"], 1);
    assert_eq!(observed["selection"], true);
    choose(&mut app, "Secondary independent fix");
    until(&mut app, |app| app.doc().text == "secondary 猫🙂\r\n");
    app.execute("undo", Value::Null);
    wait_primary_publication(&mut app);
    app.doc_mut().move_to(0, false);
    app.doc_mut().move_to(3, true);
    actions(&mut app, "editor.action.quickFix");
    until(
        &mut app,
        |app| matches!(&app.modal, Some(Modal::Language { items, .. }) if items.iter().any(|row| row.label.starts_with("Fix selected text"))),
    );
    choose(&mut app, "Fix selected text");
    assert_eq!(app.doc().text.to_string(), "fixed 猫🙂\r\n");
    // An immediate Undo coalesces into identical host bytes but advances its
    // version. That still needs a change event and fresh diagnostic publication.
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "bad 猫🙂\r\n");
    wait_primary_publication(&mut app);
    app.doc_mut().move_to(0, false);
    app.doc_mut().move_to(3, true);
    actions(&mut app, "editor.action.quickFix");
    until(&mut app, |app| {
        matches!(&app.modal, Some(Modal::Language {items,..})
        if items.iter().any(|row| row.label.starts_with("Primary Unicode fix")))
    });
    app.execute(
        "vscli.extensions.stopSelected",
        serde_json::json!({"id":"qualification.code-action-primary"}),
    );
    if matches!(&app.modal, Some(Modal::Language {items,..})
        if items.iter().any(|row| row.label.starts_with("Primary Unicode fix")))
    {
        choose(&mut app, "Primary Unicode fix");
    }
    assert_eq!(app.doc().text.to_string(), "bad 猫🙂\r\n");
    // Retiring one extension must leave both another optional source and the
    // same native protocol usable in the next user-invoked action request.
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.owner_active("qualification.code-action-secondary")
                && !host
                    .packages
                    .iter()
                    .any(|package| package.id == "qualification.code-action-primary")
        })
    });
    app.doc_mut().move_to(0, false);
    app.doc_mut().move_to(3, true);
    actions(&mut app, "editor.action.quickFix");
    until(&mut app, |app| {
        matches!(&app.modal, Some(Modal::Language {items,..})
        if items.iter().any(|row| row.label.starts_with("Secondary independent fix"))
        && items.iter().any(|row| row.label.starts_with("Fix selected text")))
    });
    let Some(Modal::Language { items, .. }) = &app.modal else {
        unreachable!()
    };
    assert!(
        !items
            .iter()
            .any(|row| row.label.contains("qualification.code-action-primary"))
    );
    choose(&mut app, "Fix selected text");
    assert_eq!(app.doc().text.to_string(), "fixed 猫🙂\r\n");
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "bad 猫🙂\r\n");
    assert_eq!(
        std::fs::read(root.path().join("main.md")).unwrap(),
        ORIGINAL.as_bytes()
    );
}

#[test]
fn invalid_rows_are_atomic_refactor_filters_kinds_and_supported_edit_preserves_shared_dirty_history()
 {
    let root = tempfile::tempdir().unwrap();
    let mut app = ready(root.path(), &["primary", "secondary"]);
    let id = app.doc().id;
    app.execute("workbench.action.splitEditorRight", Value::Null);
    let end = app.doc().len();
    app.doc_mut().move_to(end, false);
    app.doc_mut().insert("dirty", false);
    for title in [
        "Atomic invalid range",
        "Command must remain disabled",
        "Combined edit and command",
    ] {
        select(&mut app);
        actions(&mut app, "editor.action.quickFix");
        let selections = app.doc().selections();
        choose(&mut app, title);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), format!("{ORIGINAL}dirty"));
        assert_eq!(app.doc().selections(), selections);
        assert!(!root.path().join("COMMAND_EXECUTED").exists());
        assert_eq!(
            std::fs::read(root.path().join("main.md")).unwrap(),
            ORIGINAL.as_bytes()
        );
    }
    select(&mut app);
    let labels = actions(&mut app, "editor.action.refactor");
    assert_eq!(
        labels.len(),
        1,
        "Refactor kind excludes both QuickFix-only providers: {labels:?}"
    );
    assert!(labels[0].starts_with("Refactor only"));
    let observed: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("primary-observed.json")).unwrap())
            .unwrap();
    assert_eq!(observed["only"], "refactor");
    escape(&mut app);
    select(&mut app);
    actions(&mut app, "editor.action.quickFix");
    choose(&mut app, "Primary Unicode fix");
    let expected = "猫🙂 fixed\r\nsecond\r\ndirty";
    until(&mut app, |app| app.doc().text == expected);
    assert_eq!(app.documents.len(), 1);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    assert_eq!(app.doc().id, id);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), format!("{ORIGINAL}dirty"));
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), expected);
    app.doc_mut().save().unwrap();
    assert_eq!(
        std::fs::read(root.path().join("main.md")).unwrap(),
        expected.as_bytes()
    );
    app.execute("undo", Value::Null);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
}

#[test]
fn held_secondary_edit_undo_rejects_action_without_destroying_redo_or_bytes() {
    let root = tempfile::tempdir().unwrap();
    let other = root.path().join("other.md");
    std::fs::write(&other, ORIGINAL).unwrap();
    let mut app = ready(root.path(), &["primary"]);
    app.open(&other).unwrap();
    app.open(&root.path().join("main.md")).unwrap();
    select(&mut app);
    actions(&mut app, "editor.action.quickFix");
    let active = app.doc().id;
    let secondary = app
        .documents
        .iter()
        .position(|document| document.id != active)
        .unwrap();
    let revision = app.documents[secondary].revision;
    app.documents[secondary].insert("temporary ", false);
    let edited = app.documents[secondary].text.to_string();
    app.documents[secondary].undo();
    assert_eq!(app.documents[secondary].revision, revision);
    let selections = app.doc().selections();
    choose(&mut app, "Multiple mirrored buffers");
    assert!(
        app.message.contains("changed") || app.message.contains("stale"),
        "{}",
        app.message
    );
    assert_eq!(app.doc().selections(), selections);
    assert!(
        app.documents
            .iter()
            .all(|document| document.text == ORIGINAL)
    );
    app.documents[secondary].redo();
    assert_eq!(app.documents[secondary].text.to_string(), edited);
    for path in [other, root.path().join("main.md")] {
        assert_eq!(std::fs::read(path).unwrap(), ORIGINAL.as_bytes());
    }
}

#[test]
fn lazy_resolve_receives_original_circular_object_and_rejects_edit_undo_while_held() {
    for stale in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut app = ready(root.path(), &["lazy"]);
        actions(&mut app, "editor.action.quickFix");
        if stale {
            std::fs::write(root.path().join("hold-resolve"), "hold").unwrap();
        }
        choose(&mut app, "Lazy original object fix");
        until(&mut app, |_| {
            root.path().join("lazy-resolving.json").exists()
        });
        if stale {
            let revision = app.doc().revision;
            app.doc_mut().insert("temporary", false);
            app.doc_mut().undo();
            assert_eq!(app.doc().revision, revision);
            std::fs::remove_file(root.path().join("hold-resolve")).unwrap();
            until(&mut app, |app| {
                app.message.contains("changed") || app.message.contains("stale")
            });
            assert_eq!(app.doc().text.to_string(), ORIGINAL);
            app.doc_mut().redo();
            assert!(app.doc().text.to_string().contains("temporary"));
        } else {
            until(&mut app, |app| app.doc().text == "猫🙂 resolved\r\n");
            let evidence: Value = serde_json::from_slice(
                &std::fs::read(root.path().join("lazy-resolved.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(evidence["original"], true);
            app.execute("undo", Value::Null);
            assert_eq!(app.doc().text.to_string(), ORIGINAL);
        }
        assert_eq!(
            std::fs::read(root.path().join("main.md")).unwrap(),
            ORIGINAL.as_bytes()
        );
    }
}

#[test]
fn multi_document_action_keeps_hidden_untitled_and_dirty_native_identity_with_per_file_undo() {
    let root = tempfile::tempdir().unwrap();
    let main = root.path().join("main.md");
    let hidden = root.path().join("hidden.md");
    std::fs::write(&main, ORIGINAL).unwrap();
    std::fs::write(&hidden, ORIGINAL).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().eol = "\r\n".into();
    app.doc_mut().insert(ORIGINAL, false);
    let untitled_id = app.doc().id;
    app.open(&main).unwrap();
    let main_id = app.doc().id;
    activate(&mut app, &["primary"]);
    until(&mut app, |app| !app.current_diagnostics().is_empty());
    app.execute("qualification.openHidden", Value::Null);
    until(&mut app, |_| {
        root.path().join("hidden-opened.json").exists()
    });
    let evidence: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("hidden-opened.json")).unwrap())
            .unwrap();
    let hidden_id = evidence["id"].as_u64().unwrap();
    assert_eq!(
        app.documents.len(),
        2,
        "openTextDocument keeps its model hidden"
    );
    select(&mut app);
    actions(&mut app, "editor.action.quickFix");
    choose(&mut app, "Multiple mirrored buffers");
    until(&mut app, |app| app.doc().text == "both\r\n");
    assert_eq!(app.doc().id, main_id);
    assert_eq!(std::fs::read(&main).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(std::fs::read(&hidden).unwrap(), ORIGINAL.as_bytes());
    let untitled = app
        .documents
        .iter_mut()
        .find(|document| document.id == untitled_id)
        .unwrap();
    assert!(untitled.path.is_none());
    assert_eq!(untitled.text.to_string(), "both\r\n");
    untitled.undo();
    assert_eq!(untitled.text.to_string(), ORIGINAL);
    app.doc_mut().save().unwrap();
    assert_eq!(std::fs::read(&main).unwrap(), b"both\r\n");
    assert_eq!(
        std::fs::read(&hidden).unwrap(),
        ORIGINAL.as_bytes(),
        "Saving active model leaves hidden disk alone"
    );
    app.open(&hidden).unwrap();
    assert_eq!(app.doc().id, hidden_id);
    assert_eq!(app.doc().text.to_string(), "both\r\n");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    app.doc_mut().redo();
    app.doc_mut().save().unwrap();
    assert_eq!(std::fs::read(&hidden).unwrap(), b"both\r\n");
    app.open(&main).unwrap();
    app.doc_mut().undo();
    assert_eq!(app.doc().id, main_id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert_eq!(std::fs::read(&main).unwrap(), b"both\r\n");
    assert!(!root.path().join("Untitled").exists());
}

#[test]
#[ignore = "requires actual clangd executable explicitly set in VSCLI_CLANGD"]
fn actual_clangd_and_extension_providers_coexist_without_replacing_native_owner() {
    let program = std::env::var("VSCLI_CLANGD").expect("Set actual clangd executable");
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("main.cpp");
    let original = "int main() { return 0 }\r\n";
    std::fs::write(&source, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&source).unwrap();
    app.lsp = Some(
        Client::start(
            &program,
            &[
                "--background-index=false".into(),
                "--clang-tidy=false".into(),
            ],
            root.path(),
            "cpp".into(),
        )
        .unwrap(),
    );
    until(&mut app, |app| !app.current_diagnostics().is_empty());
    activate(&mut app, &["primary", "secondary", "failing"]);
    let position = app
        .current_diagnostics()
        .iter()
        .next()
        .unwrap()
        .range
        .start
        .character as usize;
    app.doc_mut().move_to(position, false);
    actions(&mut app, "editor.action.quickFix");
    until(&mut app, |app| {
        matches!(&app.modal, Some(Modal::Language { items, .. })
        if items.iter().any(|row| row.label.starts_with("Secondary independent fix"))
        && items.iter().any(|row| matches!(row.action, vscli::app::LanguageAction::CodeAction { .. })))
    });
    let labels = match &app.modal {
        Some(Modal::Language { items, .. }) => items
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        _ => unreachable!(),
    };
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("Secondary independent fix")),
        "{labels:?}"
    );
    let title = match &app.modal {
        Some(Modal::Language { items, .. }) => items
            .iter()
            .find_map(|item| match &item.action {
                vscli::app::LanguageAction::CodeAction { item: value, .. }
                    if value.get("edit").is_some() =>
                {
                    Some(item.label.clone())
                }
                _ => None,
            })
            .expect("Actual clangd should offer missing-semicolon text fix"),
        _ => unreachable!(),
    };
    choose(&mut app, &title);
    assert_eq!(app.doc().text.to_string(), "int main() { return 0; }\r\n");
    assert_eq!(std::fs::read(&source).unwrap(), original.as_bytes());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
}
