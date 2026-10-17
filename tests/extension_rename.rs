//! Actual public rename queries and original F2 grouped native transactions.
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
const RENAMED: &str = "dirty 猫🙂 renamed\r\nnext\r\n";
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut terminal = Terminal::new(TestBackend::new(160, 35)).unwrap();
        terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Rename phase {phase}: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, id: &str, message: &str) {
    app.execute(&format!("renameFixture.{id}"), Value::Null);
    until(app, message, |app| app.message == message);
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn ready(root: &Path) -> App {
    ready_with_refactoring(root, Some(false))
}
fn ready_with_refactoring(root: &Path, refactoring: Option<bool>) -> App {
    for (name, text) in [
        ("source.cpp", SOURCE),
        ("target.cpp", TARGET),
        ("hidden.cpp", HIDDEN),
        ("unopened.cpp", UNOPENED),
    ] {
        std::fs::write(root.join(name), text).unwrap();
    }
    let settings = root.join("settings.json");
    let mut configuration = json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}});
    if let Some(value) = refactoring {
        configuration["files.refactoring.autoSave"] = json!(value);
    }
    std::fs::write(&settings, serde_json::to_vec(&configuration).unwrap()).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("source.cpp")).unwrap();
    until(&mut app, "original source open", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("source.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rename-provider-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, "selected host ready", |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "renameFixture.queries")
        })
    });
    app
}
fn source(app: &mut App) {
    command(app, "source", "API rename source;identity=true");
}
fn prompt(app: &mut App) {
    key(app, KeyCode::F(2));
    until(app, "prepared original Rename prompt", |app| {
        matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::Rename)
        ) && app.prompt.as_ref().unwrap().text == "source"
    });
}
fn accept(app: &mut App, name: &str) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));
    app.event(Event::Paste(name.into()));
    key(app, KeyCode::Enter);
}
fn disks(root: &Path) {
    for (name, text) in [
        ("source.cpp", SOURCE),
        ("target.cpp", TARGET),
        ("hidden.cpp", HIDDEN),
        ("unopened.cpp", UNOPENED),
    ] {
        assert_eq!(std::fs::read(root.join(name)).unwrap(), text.as_bytes());
    }
    for name in ["created.cpp", "old.cpp", "new.cpp", "deleted.cpp"] {
        assert!(!root.join(name).exists());
    }
}
fn source_history(app: &mut App, id: u64) {
    source(app);
    assert_eq!(app.doc().id, id);
    app.execute("undo", Value::Null);
    command(app, "sourceClean", "API rename source Undo;exact=true");
    app.execute("redo", Value::Null);
    command(app, "sourceDirty", "API rename source Redo;exact=true");
    assert_eq!(app.doc().id, id);
}

#[test]
fn public_rename_queries_preserve_preparation_fallback_index_empty_name_brand_and_source_value_conversion_without_mutation()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let selections = app.doc().selections();
    command(
        &mut app,
        "queries",
        "API rename queries;prepare/fallback/index/empty/fullValue=true;noMutation=true",
    );
    assert_ne!(app.doc().id, id);
    assert_eq!(
        app.doc().path.as_deref(),
        Some(root.join("target.cpp").as_path())
    );
    assert_eq!(app.doc().text.to_string(), TARGET);
    assert!(!app.doc().dirty());
    let retained = app.documents.iter().find(|doc| doc.id == id).unwrap();
    assert_eq!(retained.selections(), selections);
    assert_eq!(app.documents.len(), 2);
    assert!(app.modal.is_none());
    assert!(app.prompt.is_none());
    let current = retained.capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    source_history(&mut app, id);
    disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn original_f2_groups_dirty_shared_hidden_and_unopened_text_targets_with_one_complete_undo_redo_and_no_save()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let source_member = app.editor_groups().active_membership().unwrap();
    command(
        &mut app,
        "prepareTargets",
        "API rename targets;shared/hiddenDirty=true;unopenedUntouched=true",
    );
    let target = app
        .documents
        .iter()
        .find(|doc| doc.path.as_deref() == Some(root.join("target.cpp").as_path()))
        .unwrap();
    let target_id = target.id;
    let members: Vec<_> = app.editor_groups().memberships(target_id).collect();
    assert_eq!(members.len(), 2);
    assert!(
        app.documents
            .iter()
            .all(|doc| doc.path.as_deref() != Some(root.join("unopened.cpp").as_path()))
    );
    prompt(&mut app);
    accept(&mut app, "renamed");
    until(&mut app, "atomic native rename publication", |app| {
        app.prompt.is_none()
            && app.modal.is_none()
            && app
                .active_document()
                .is_some_and(|doc| doc.id == id && doc.text == RENAMED)
    });
    command(
        &mut app,
        "verifyApplied",
        "API rename applied;grouped/source/shared/hidden/unopened=true;didSave=0",
    );
    assert_eq!(app.editor_groups().active_membership(), Some(source_member));
    for member in &members {
        assert!(app.editor_groups().membership_current(*member));
    }
    assert_eq!(app.editor_groups().memberships(target_id).count(), 2);
    disks(&root);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "verifyUndo",
        "API rename Undo;grouped/source/shared/hidden/unopened=true;didSave=0",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), format!("dirty {SOURCE}"));
    disks(&root);
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "verifyRedo",
        "API rename Redo;grouped/source/shared/hidden/unopened=true;didSave=0",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), RENAMED);
    for member in members {
        assert!(app.editor_groups().membership_current(member));
    }
    disks(&root);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "verifyUndo",
        "API rename Undo;grouped/source/shared/hidden/unopened=true;didSave=0",
    );
    source_history(&mut app, id);
    disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn original_single_resource_rename_keeps_dirty_history_and_disk_under_default_refactoring_policy() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready_with_refactoring(&root, None);
    let id = app.doc().id;
    let membership = app.editor_groups().active_membership().unwrap();
    let generation = app
        .doc()
        .capture_save(root.join("source.cpp"))
        .unwrap()
        .save_generation();
    prompt(&mut app);
    accept(&mut app, "renamed");
    until(&mut app, "single resource native publication", |app| {
        app.prompt.is_none() && app.modal.is_none() && app.doc().text == RENAMED
    });
    command(
        &mut app,
        "verifySingle",
        "API rename single;identity/dirty=true;defaultPolicy/didSave=0",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.editor_groups().active_membership(), Some(membership));
    assert_eq!(
        app.doc()
            .capture_save(root.join("source.cpp"))
            .unwrap()
            .save_generation(),
        generation
    );
    disks(&root);
    app.execute("undo", Value::Null);
    command(&mut app, "sourceDirty", "API rename source Redo;exact=true");
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "verifySingle",
        "API rename single;identity/dirty=true;defaultPolicy/didSave=0",
    );
    app.execute("undo", Value::Null);
    source_history(&mut app, id);
    disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn actual_held_prepare_and_edit_cancel_on_source_undo_settings_aba_and_provider_retirement_without_late_publication()
 {
    for scenario in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut app = ready(&root);
        let id = app.doc().id;
        let selections = app.doc().selections();
        eprintln!("Rename held scenario {scenario}");
        let phase = if scenario == 0 { "prepare" } else { "edit" };
        command(
            &mut app,
            if scenario == 0 {
                "armPrepare"
            } else {
                "armEdit"
            },
            &format!("API rename held {phase} armed"),
        );
        if scenario == 0 {
            key(&mut app, KeyCode::F(2));
        } else {
            prompt(&mut app);
            accept(&mut app, "ignored");
        }
        until(&mut app, &format!("actual {phase} callback held"), |app| {
            app.message == format!("API rename held {phase};entered=true;actualPending=true")
        });
        if scenario == 0 {
            app.event(Event::Paste("X".into()));
            app.execute("undo", Value::Null);
        } else if scenario == 1 {
            let alternate = root.join("alternate-settings.json");
            std::fs::copy(root.join("settings.json"), &alternate).unwrap();
            app.configure_settings(Some(alternate)).unwrap();
            app.configure_settings(Some(root.join("settings.json")))
                .unwrap();
        } else {
            command(
                &mut app,
                "dispose",
                "API rename provider disposed;source=retained",
            );
        }
        if scenario != 2 {
            until(
                &mut app,
                "actual token cancelled before control command",
                |app| {
                    app.message
                        == format!("API rename held {phase};cancelled=true;actualPending=true")
                },
            );
        }
        command(
            &mut app,
            "release",
            &format!("API rename held {phase};cancelled/actualSettled=true"),
        );
        assert!(app.prompt.is_none());
        assert!(app.modal.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), format!("dirty {SOURCE}"));
        assert_eq!(app.doc().selections(), selections);
        if scenario == 0 {
            app.execute("redo", Value::Null);
            command(
                &mut app,
                "heldRedo",
                "API rename held Redo;intentionalEdit/lateReply=false",
            );
            app.execute("undo", Value::Null);
            assert_eq!(app.doc().text.to_string(), format!("dirty {SOURCE}"));
        }
        source_history(&mut app, id);
        disks(&root);
        app.execute("vscli.extensions.stop", Value::Null);
    }
}

#[test]
fn eight_public_rename_callbacks_retain_capacity_and_ninth_refuses_until_actual_settlement() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let selections = app.doc().selections();
    command(
        &mut app,
        "capacity",
        "API rename capacity;actualCallbacks=8;held=true",
    );
    command(
        &mut app,
        "capacityOverflow",
        "API rename capacity;ninth=refused;actualStill8=true",
    );
    command(
        &mut app,
        "capacityRelease",
        "API rename capacity;originals/fresh=true;actualReleased=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().selections(), selections);
    let current = app.doc().capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    source_history(&mut app, id);
    disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[cfg(unix)]
#[test]
fn public_unopened_alias_receipt_reuses_the_original_dirty_model_and_grouped_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    std::os::unix::fs::symlink(root.join("source.cpp"), root.join("source-alias.cpp")).unwrap();
    command(
        &mut app,
        "aliasApply",
        "API rename alias;originalDirtyIdentity/mirrorBeforeAck=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.doc().path, Some(root.join("source.cpp")));
    app.execute("undo", Value::Null);
    command(&mut app, "sourceDirty", "API rename source Redo;exact=true");
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "dirty 猫🙂 aliased\r\nnext\r\n");
    app.execute("undo", Value::Null);
    source_history(&mut app, id);
    disks(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
