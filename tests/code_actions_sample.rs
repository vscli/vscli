//! Opt-in conformance using the unchanged official sample, not a production extension.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Modal},
    extensions::Package,
    keys::Profile,
};

const ORIGINAL: &str = "猫🙂 :) emoji\r\n";
const DIRTY: &str = "dirty 猫🙂 :) emoji\r\n";

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
fn verify(directory: &Path) {
    assert!(
        std::process::Command::new("python")
            .args(["tests/prepare_code_actions_sample.py", "--verify-only"])
            .arg(directory)
            .status()
            .unwrap()
            .success()
    );
}
fn ready(root: &Path, dirty: bool) -> (App, std::path::PathBuf) {
    let directory = std::path::PathBuf::from(
        std::env::var_os("VSCLI_CODE_ACTIONS_SAMPLE")
            .expect("Set prepared official sample directory"),
    );
    verify(&directory);
    let package = Package::read(&directory).unwrap();
    assert_eq!(package.id, "vscode-samples.code-actions-sample");
    assert_eq!(package.version, "0.0.2");
    let path = root.join("input.md");
    std::fs::write(&path, ORIGINAL).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.open(&path).unwrap();
    if dirty {
        app.doc_mut().insert("dirty ", false);
    }
    app.start_extension_packages(vec![package]).unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
            && app
                .current_diagnostics()
                .iter()
                .any(|d| d.message.contains("When you say 'emoji'"))
    });
    (app, directory)
}
fn select(app: &mut App, start: usize, end: usize) {
    app.doc_mut().move_to(start, false);
    app.doc_mut().move_to(end, true);
}
fn actions(app: &mut App) -> Vec<String> {
    app.execute("editor.action.quickFix", Value::Null);
    until(app, |app| matches!(app.modal, Some(Modal::Language { .. })));
    let Some(Modal::Language { items, .. }) = &app.modal else {
        unreachable!()
    };
    items.iter().map(|item| item.label.clone()).collect()
}
fn choose_index(app: &mut App, index: usize) {
    let Some(Modal::Language { selected, .. }) = &mut app.modal else {
        panic!("No code actions")
    };
    *selected = index;
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
}

#[test]
#[ignore = "requires unchanged compiled official sample 0.0.2 in VSCLI_CODE_ACTIONS_SAMPLE"]
fn official_two_providers_keep_supported_edits_and_reject_command_rows_individually() {
    for emoji in ["😺", "😀", "💩"] {
        let root = tempfile::tempdir().unwrap();
        let (mut app, directory) = ready(root.path(), true);
        let path = root.path().join("input.md");
        assert_eq!(app.doc().text.to_string(), DIRTY);
        let id = app.doc().id;
        app.execute("workbench.action.splitEditorRight", Value::Null);
        select(&mut app, 9, 17);
        let labels = actions(&mut app);
        assert_eq!(
            labels.len(),
            5,
            "Both matching providers must contribute: {labels:?}"
        );
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.starts_with("Convert to "))
                .count(),
            3
        );
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.starts_with("Learn more..."))
                .count(),
            2
        );
        // Each unsupported command remains a row; accepting either must preserve
        // text, identity, history, shared views and disk without opening a browser.
        for command_index in labels
            .iter()
            .enumerate()
            .filter_map(|(index, label)| label.starts_with("Learn more...").then_some(index))
        {
            let selections = app.doc().selections();
            choose_index(&mut app, command_index);
            assert!(
                app.message.contains("failed") || app.message.contains("unsupported"),
                "{}",
                app.message
            );
            assert_eq!(app.doc().id, id);
            assert_eq!(app.doc().text.to_string(), DIRTY);
            assert_eq!(app.doc().selections(), selections);
            assert_eq!(std::fs::read(&path).unwrap(), ORIGINAL.as_bytes());
            assert_eq!(actions(&mut app).len(), 5);
        }
        let labels = if let Some(Modal::Language { items, .. }) = &app.modal {
            items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>()
        } else {
            unreachable!()
        };
        choose_index(
            &mut app,
            labels
                .iter()
                .position(|label| label.starts_with(&format!("Convert to {emoji}")))
                .unwrap(),
        );
        let expected = DIRTY.replace(":)", emoji);
        until(&mut app, |app| app.doc().text == expected);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.panes.len(), 2);
        assert!(app.panes.iter().all(|pane| pane.document == id));
        assert!(app.doc().dirty());
        assert_eq!(std::fs::read(&path).unwrap(), ORIGINAL.as_bytes());
        app.execute("undo", Value::Null);
        assert_eq!(
            app.doc().text.to_string(),
            DIRTY,
            "One Undo restores only the action"
        );
        app.execute("redo", Value::Null);
        assert_eq!(app.doc().text.to_string(), expected);
        app.doc_mut().save().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), DIRTY);
        app.execute("undo", Value::Null);
        assert_eq!(
            app.doc().text.to_string(),
            ORIGINAL,
            "Earlier dirty work remains separately undoable"
        );
        app.doc_mut().save().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), ORIGINAL.as_bytes());
        verify(&directory);
    }
}

#[test]
#[ignore = "requires unchanged compiled official sample 0.0.2 in VSCLI_CODE_ACTIONS_SAMPLE"]
fn official_empty_newer_provider_does_not_hide_older_fixes_and_retirement_invalidates_actions() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, directory) = ready(root.path(), false);
    let id = app.doc().id;
    let diagnostic = app
        .current_diagnostics()
        .iter()
        .find(|d| d.message.contains("When you say 'emoji'"))
        .unwrap();
    assert_eq!(diagnostic.range.start.character, 7, "UTF-16 after 猫🙂");
    assert_eq!(diagnostic.range.end.character, 12);
    select(&mut app, 3, 5);
    let labels = actions(&mut app);
    assert_eq!(
        labels.len(),
        4,
        "Newer diagnostic provider is empty, older smiley provider survives"
    );
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    select(&mut app, 6, 11);
    let labels = actions(&mut app);
    assert_eq!(labels.len(), 1);
    assert!(labels[0].starts_with("Learn more..."));
    choose_index(&mut app, 0);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    select(&mut app, 3, 11);
    let labels = actions(&mut app);
    let chosen = labels
        .iter()
        .position(|label| label.starts_with("Convert to 😀"))
        .unwrap();
    app.execute(
        "vscli.extensions.stopSelected",
        json!({"id":"vscode-samples.code-actions-sample"}),
    );
    until(&mut app, |app| app.current_diagnostics().is_empty());
    // The controller may proactively close retired rows. If it retains a picker,
    // its acceptance path must reject the retired owner instead of applying it.
    if matches!(app.modal, Some(Modal::Language { .. })) {
        choose_index(&mut app, chosen);
    }
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert_eq!(
        std::fs::read(root.path().join("input.md")).unwrap(),
        ORIGINAL.as_bytes()
    );
    verify(&directory);
}
