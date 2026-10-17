//! Actual selected optional-host refactoring edits followed by native owned saves.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, PromptKind},
    extensions::Package,
    keys::Profile,
};

const SOURCE: &str = "猫🙂 source\r\nnext\r\n";
const TARGET: &str = "α🙂 target\r\n";
const HIDDEN: &str = "γ🙂 source\r\n";
const UNOPENED: &str = "β🙂 source\r\n";
const UNRELATED: &str = "δ🙂 unrelated\r\n";
const PREPARED: &str = "API refactoring cohort prepared;dirty/shared/hidden/unopened=true";
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        Terminal::new(TestBackend::new(180, 40))
            .unwrap()
            .draw(|frame| vscli::ui::draw(frame, app))
            .unwrap();
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Refactoring {phase}: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, id: &str, expected: &str) {
    app.execute(&format!("refactorFixture.{id}"), Value::Null);
    until(app, expected, |app| app.message == expected);
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn accept(app: &mut App, value: &str) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));
    app.event(Event::Paste(value.into()));
    key(app, KeyCode::Enter);
}
fn ready(root: &Path, policy: Option<bool>) -> App {
    for (name, text) in [
        ("source", SOURCE),
        ("target", TARGET),
        ("hidden", HIDDEN),
        ("unopened", UNOPENED),
        ("unrelated", UNRELATED),
    ] {
        std::fs::write(root.join(format!("{name}.cpp")), text).unwrap();
    }
    let mut settings = json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}});
    if let Some(policy) = policy {
        settings["files.refactoring.autoSave"] = json!(policy);
    }
    let configuration = root.join("settings.json");
    std::fs::write(&configuration, serde_json::to_vec(&settings).unwrap()).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(configuration)).unwrap();
    app.open(&root.join("source.cpp")).unwrap();
    until(&mut app, "source open", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("source.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/refactoring-save-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, "selected host ready", |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "refactorFixture.prepare")
        })
    });
    app
}
fn saved_text(name: &str) -> String {
    match name {
        "source" => "dirty 猫🙂 renamed\r\nnext\r\n".into(),
        "target" => "dirty α🙂 renamed\r\n".into(),
        "hidden" => "dirty γ🙂 renamed\r\n".into(),
        "unopened" => "β🙂 renamed\r\n".into(),
        _ => unreachable!(),
    }
}
fn original_disks(root: &Path) {
    for (name, text) in [
        ("source", SOURCE),
        ("target", TARGET),
        ("hidden", HIDDEN),
        ("unopened", UNOPENED),
        ("unrelated", UNRELATED),
    ] {
        assert_eq!(
            std::fs::read(root.join(format!("{name}.cpp"))).unwrap(),
            text.as_bytes(),
            "{name}"
        );
    }
}
fn saved_disks(root: &Path, conflict: bool) {
    for name in ["source", "target", "hidden", "unopened"] {
        let expected = if conflict && name == "target" {
            "external conflict 猫🙂\r\n".into()
        } else {
            saved_text(name)
        };
        assert_eq!(
            std::fs::read(root.join(format!("{name}.cpp"))).unwrap(),
            expected.as_bytes(),
            "{name}"
        );
    }
    assert_eq!(
        std::fs::read(root.join("unrelated.cpp")).unwrap(),
        UNRELATED.as_bytes()
    );
}
fn model<'a>(app: &'a App, root: &Path, name: &str) -> &'a vscli::document::Document {
    let path = root.join(format!("{name}.cpp"));
    app.recovery_documents()
        .into_iter()
        .find(|doc| doc.path.as_ref() == Some(&path))
        .unwrap()
}

#[test]
fn default_public_multiresource_refactoring_saves_all_before_true_and_grouped_history_retains_disks()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, None);
    let source = app.doc().id;
    command(&mut app, "prepare", PREPARED);
    let target = model(&app, &root, "target").id;
    let members: Vec<_> = app.editor_groups().memberships(target).collect();
    assert_eq!(members.len(), 2);
    original_disks(&root);
    command(
        &mut app,
        "apply",
        "API refactoring default;applied=true;didSave=4;identity/shared/hidden=true",
    );
    saved_disks(&root, false);
    for name in ["source", "target"] {
        assert!(!model(&app, &root, name).dirty());
        assert_eq!(
            model(&app, &root, name)
                .capture_save(root.join(format!("{name}.cpp")))
                .unwrap()
                .save_generation(),
            1
        );
    }
    assert_eq!(app.doc().id, source);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "undo",
        "API refactoring grouped Undo;allOriginal=true;didSave=4",
    );
    for name in ["source", "target", "hidden", "unopened"] {
        assert!(model(&app, &root, name).dirty());
    }
    saved_disks(&root, false);
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "redo",
        "API refactoring Redo;applied=true;didSave=4;identity/shared/hidden=true",
    );
    saved_disks(&root, false);
    assert_eq!(app.doc().id, source);
    for member in members {
        assert!(app.editor_groups().membership_current(member));
    }
    assert_eq!(app.editor_groups().memberships(target).count(), 2);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn original_f2_default_refactoring_waits_for_complete_four_target_save_cohort() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, None);
    let source = app.doc().id;
    command(&mut app, "prepare", PREPARED);
    command(&mut app, "armRename", "API refactoring F2 armed");
    app.doc_mut().move_to(10, false);
    key(&mut app, KeyCode::F(2));
    until(&mut app, "original F2 prompt", |app| {
        matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::Rename)
        )
    });
    accept(&mut app, "renamed");
    // An intermediate clean source/idle save lane does not prove aggregate completion.
    until(&mut app, "whole F2 save completion", |app| {
        app.message == "Renamed atomically · Undo restores the workspace"
    });
    command(
        &mut app,
        "applied",
        "API refactoring F2;applied=true;didSave=4;identity/shared/hidden=true",
    );
    saved_disks(&root, false);
    assert_eq!(app.doc().id, source);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "undo",
        "API refactoring grouped Undo;allOriginal=true;didSave=4",
    );
    saved_disks(&root, false);
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "redo",
        "API refactoring Redo;applied=true;didSave=4;identity/shared/hidden=true",
    );
    saved_disks(&root, false);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn explicit_false_multiresource_policy_keeps_whole_text_cohort_unsaved() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, Some(false));
    command(&mut app, "prepare", PREPARED);
    command(
        &mut app,
        "applyOff",
        "API refactoring off;applied=true;didSave=0;identity/shared/hidden=true",
    );
    original_disks(&root);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "undo",
        "API refactoring grouped Undo;allOriginal=true;didSave=0",
    );
    original_disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn default_single_resource_and_unsupported_mixed_refactoring_keep_native_disks_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, None);
    command(
        &mut app,
        "unsupported",
        "API refactoring unsupported;wholeRefused=true;didSave=0",
    );
    assert!(!root.join("unsupported-created.cpp").exists());
    original_disks(&root);
    command(
        &mut app,
        "single",
        "API refactoring single;applied=true;didSave=0;dirty=true",
    );
    original_disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

fn await_warning(app: &mut App, expected: &str) -> bool {
    let mut warning = false;
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        app.poll();
        warning |= app.message.contains("refactoring save")
            && app.message.contains("edits remain applied");
        if app.message == expected {
            return warning;
        }
        assert!(
            Instant::now() < deadline,
            "Await public warning/receipt: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn public_refactoring_save_as_accepted_closes_old_mirror_opens_saved_mirror_before_true() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, None);
    let source = app.doc().id;
    app.execute("refactorFixture.saveAs", Value::Null);
    until(&mut app, "cohort actual Save As", |app| {
        matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::SaveAs)
        )
    });
    let hidden = app
        .recovery_documents()
        .into_iter()
        .find(|doc| doc.path.is_none())
        .unwrap()
        .id;
    let destination = root.join("saved-untitled 猫 # %'(.cpp");
    accept(&mut app, destination.to_str().unwrap());
    until(&mut app, "public Save As awaited true", |app| {
        app.message == "API refactoring SaveAs accepted;applied=true;didSave=2;namedCommitted=true"
    });
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        "saved untitled 猫🙂 before\r\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("source.cpp")).unwrap(),
        format!("named dirty {SOURCE}").as_bytes()
    );
    assert_eq!(app.doc().id, source);
    // Only after the public receipt, reveal the real saved model to inspect
    // its native ID without making private App state part of this API test.
    app.open(&destination).unwrap();
    until(&mut app, "inspect saved native identity", |app| {
        app.doc().path.as_ref() == Some(&destination)
    });
    assert_eq!(app.doc().id, hidden);
    assert!(!app.doc().dirty());
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn public_refactoring_save_as_cancel_keeps_applied_true_dirty_untitled_and_warns() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, None);
    app.execute("refactorFixture.cancelSaveAs", Value::Null);
    until(&mut app, "cohort actual Save As", |app| {
        matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::SaveAs)
        )
    });
    key(&mut app, KeyCode::Esc);
    assert!(await_warning(
        &mut app,
        "API refactoring SaveAs canceled;applied=true;didSave=1;namedCommitted=true"
    ));
    assert_eq!(
        std::fs::read(root.join("source.cpp")).unwrap(),
        format!("named dirty {SOURCE}").as_bytes()
    );
    let hidden = app
        .recovery_documents()
        .into_iter()
        .find(|doc| doc.path.is_none())
        .unwrap();
    assert_eq!(hidden.text.to_string(), "saved untitled 猫🙂 before\r\n");
    assert!(hidden.dirty());
    assert!(!root.join("untitled.txt").exists());
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn conflict_preserves_atomic_applied_text_successful_peer_saves_and_external_disk_without_retry() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, None);
    command(&mut app, "prepare", PREPARED);
    std::fs::write(root.join("target.cpp"), "external conflict 猫🙂\r\n").unwrap();
    app.execute("refactorFixture.conflict", Value::Null);
    assert!(await_warning(
        &mut app,
        "API refactoring conflict;applied=true;didSave=3;dirtyFailedTarget=true;noRetry=true"
    ));
    saved_disks(&root, true);
    assert!(model(&app, &root, "target").dirty());
    assert_eq!(
        model(&app, &root, "target").text.to_string(),
        saved_text("target")
    );
    for _ in 0..50 {
        app.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    saved_disks(&root, true);
    assert_eq!(
        model(&app, &root, "target")
            .capture_save(root.join("target.cpp"))
            .unwrap()
            .save_generation(),
        0
    );
    app.execute("vscli.extensions.stop", Value::Null);
}
