//! Actual selected providers, complete public query results and native location UX.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Modal},
    extensions::Package,
    keys::Profile,
};
const ORIGINAL: &str = "猫🙂 source\r\nnext\r\n";
const DIRTY: &str = "dirty 猫🙂 source\r\nnext\r\n";
const ONE: &str = "α🙂 targetA\r\n";
const QUERIES: &str =
    "API nav queries;five/fullLinks/sort/dedup=true;inactive/virtual/language/isolation=true";
const COMMANDS: [&str; 5] = [
    "editor.action.revealDefinition",
    "editor.action.goToReferences",
    "editor.action.goToImplementation",
    "editor.action.goToTypeDefinition",
    "editor.action.revealDeclaration",
];
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    until_phase(app, "native state", predicate);
}
fn until_phase(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut terminal = Terminal::new(TestBackend::new(150, 35)).unwrap();
        terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Navigation fixture timeout during {phase}: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn receipt(app: &mut App, text: &str) {
    until_phase(app, text, |app| app.message == text);
}
fn command(app: &mut App, id: &str, text: &str) {
    app.execute(id, Value::Null);
    receipt(app, text);
}
fn ready(root: &Path) -> App {
    std::fs::write(root.join("source.cpp"), ORIGINAL).unwrap();
    std::fs::write(root.join("one.cpp"), ONE).unwrap();
    std::fs::write(root.join("two.cpp"), "β🙂 targetB\r\n").unwrap();
    std::fs::write(root.join("three.cpp"), "γ🙂 targetC\r\n").unwrap();
    std::fs::write(root.join("held.hold"), "hold\r\n").unwrap();
    let settings = root.join("settings.json");
    std::fs::write(&settings,serde_json::to_vec(&json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}})).unwrap()).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("source.cpp")).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("source.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/navigation-providers-extension");
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![Package::read(&package).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "navigationProviders.source")
        })
    });
    app
}
fn source(app: &mut App) {
    command(
        app,
        "navigationProviders.source",
        "API navigation source;identity=true;dirty=true",
    );
}
fn unchanged(root: &Path, deleted: bool) {
    assert_eq!(
        std::fs::read(root.join("source.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
    if deleted {
        assert!(!root.join("one.cpp").exists());
    } else {
        assert_eq!(std::fs::read(root.join("one.cpp")).unwrap(), ONE.as_bytes());
    }
    assert_eq!(
        std::fs::read(root.join("two.cpp")).unwrap(),
        "β🙂 targetB\r\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("three.cpp")).unwrap(),
        "γ🙂 targetC\r\n".as_bytes()
    );
}
fn original_history(app: &mut App, id: u64) {
    source(app);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    app.execute("undo", Value::Null);
    command(
        app,
        "navigationProviders.originalClean",
        "API navigation original Undo;clean=true",
    );
    app.execute("redo", Value::Null);
    command(
        app,
        "navigationProviders.originalDirty",
        "API navigation original Redo;dirty=true",
    );
    assert_eq!(app.doc().id, id);
}
fn picker(app: &mut App, command: &str) {
    app.execute(command, Value::Null);
    until_phase(
        app,
        &format!("{command}: aggregate picker with three locations"),
        |app| matches!(&app.modal,Some(Modal::Language{items,..})if items.len()==3),
    );
    let Some(Modal::Language {
        items, selected, ..
    }) = &mut app.modal
    else {
        unreachable!()
    };
    *selected = items
        .iter()
        .position(|row| row.label.contains("one.cpp"))
        .expect("retained target in native aggregate picker");
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    until_phase(
        app,
        &format!("{command}: accepted retained target"),
        |app| {
            app.active_document().is_some_and(|doc| {
                doc.path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("one.cpp"))
            }) && app.modal.is_none()
        },
    );
}
#[test]
fn five_provider_families_aggregate_native_picker_and_collapse_reveal_into_deleted_retained_target()
{
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let source_id = app.doc().id;
    let source_proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let source_selections = app.doc().selections();
    command(
        &mut app,
        "navigationProviders.prepareTarget",
        "API navigation target;retained/shared=true;dirty=true",
    );
    let target_id = app
        .documents
        .iter()
        .find(|doc| doc.path.as_deref() == Some(root.join("one.cpp").as_path()))
        .unwrap()
        .id;
    assert_eq!(app.editor_groups().memberships(target_id).count(), 2);
    let target = app
        .documents
        .iter()
        .find(|doc| doc.id == target_id)
        .unwrap();
    let target_proof = target.capture_save(root.join("one.cpp")).unwrap();
    let target_selections = target.selections();
    std::fs::remove_file(root.join("one.cpp")).unwrap();
    // Settle the real missing-file baseline before issuing any navigation.
    // Provider contexts include retained model baselines, so this setup change
    // must not race the target request and its asynchronous reply.
    until_phase(&mut app, "deleted target watch publication", |app| {
        app.documents
            .iter()
            .find(|doc| doc.id == target_id)
            .is_some_and(|doc| doc.saved_revision == u64::MAX)
    });
    let target = app
        .documents
        .iter()
        .find(|doc| doc.id == target_id)
        .unwrap();
    let settled_proof = target.capture_save(root.join("one.cpp")).unwrap();
    assert_eq!(target.text.to_string(), format!("dirty {ONE}"));
    assert_eq!(target.selections(), target_selections);
    assert_eq!(settled_proof.text_epoch(), target_proof.text_epoch());
    assert_eq!(
        settled_proof.save_generation(),
        target_proof.save_generation()
    );
    assert_eq!(app.editor_groups().memberships(target_id).count(), 2);
    for id in COMMANDS {
        eprintln!("Navigation family {id}: source setup, picker, target acceptance");
        source(&mut app);
        picker(&mut app, id);
        command(
            &mut app,
            "navigationProviders.target",
            "API navigation reveal;selectionStart=7;collapsed=true;identity=true",
        );
        assert_eq!(app.doc().id, target_id);
        assert_eq!(app.doc().cursor, 7);
        assert_eq!(app.doc().anchor, None);
        assert!(app.doc().secondary.is_empty());
        assert_eq!(app.doc().text.to_string(), format!("dirty {ONE}"));
        assert_eq!(app.documents.len(), 2);
        assert!(app.editor_groups().memberships(target_id).count() >= 2);
    }
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "navigationProviders.targetUndo",
        "API navigation target Undo;exactSource=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "navigationProviders.targetRedo",
        "API navigation target Redo;exactSource=true",
    );
    assert_eq!(app.doc().id, target_id);
    source(&mut app);
    assert_eq!(app.doc().id, source_id);
    assert_eq!(app.doc().selections(), source_selections);
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(proof.text_epoch(), source_proof.text_epoch());
    assert_eq!(proof.save_generation(), source_proof.save_generation());
    original_history(&mut app, source_id);
    unchanged(&root, true);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_queries_preserve_complete_links_sort_dedup_inactive_virtual_and_effective_language_without_editing()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let selections = app.doc().selections();
    command(&mut app, "navigationProviders.queries", QUERIES);
    assert_ne!(app.doc().id, id);
    assert_eq!(
        app.doc().path.as_deref(),
        Some(root.join("two.cpp").as_path())
    );
    assert_eq!(app.doc().text.to_string(), "β🙂 targetB\r\n");
    assert!(!app.doc().dirty());
    let retained = app.documents.iter().find(|doc| doc.id == id).unwrap();
    assert_eq!(retained.text.to_string(), DIRTY);
    assert_eq!(retained.selections(), selections);
    let current = retained.capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    original_history(&mut app, id);
    unchanged(&root, false);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn held_navigation_reply_cannot_revive_source_language_view_or_registration_aba() {
    for change in 0..4 {
        eprintln!(
            "Navigation held scenario {change}: source=editUndo/languageABA/viewABA/disposal"
        );
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut app = ready(&root);
        let id = app.doc().id;
        command(
            &mut app,
            "navigationProviders.armHeld",
            "API navigation held armed",
        );
        app.execute(COMMANDS[0], Value::Null);
        until_phase(&mut app, "actual held callback entry", |app| {
            app.message
                .starts_with("API navigation held;entered=true;source=")
        });
        match change {
            0 => {
                app.doc_mut().insert("X", false);
                app.execute("undo", Value::Null);
                assert_eq!(app.doc().text.to_string(), DIRTY);
            }
            1 => command(
                &mut app,
                "navigationProviders.languageABA",
                "API navigation language ABA;source=unchanged",
            ),
            2 => {
                app.open(&root.join("two.cpp")).unwrap();
                until(&mut app, |app| {
                    app.active_document().is_some_and(|doc| {
                        doc.path.as_deref() == Some(root.join("two.cpp").as_path())
                    })
                });
                app.open(&root.join("source.cpp")).unwrap();
                assert_eq!(app.doc().id, id);
            }
            _ => command(
                &mut app,
                "navigationProviders.disposeHeld",
                "API navigation registration disposed;source=unchanged",
            ),
        }
        command(
            &mut app,
            "navigationProviders.releaseHeld",
            "API navigation held released;actualCallback=true",
        );
        assert!(app.modal.is_none());
        assert!(app.prompt.is_none());
        source(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), DIRTY);
        if change == 0 {
            app.execute("redo", Value::Null);
            assert_eq!(app.doc().text.to_string(), format!("dirty X{}", ORIGINAL));
            app.execute("undo", Value::Null);
            assert_eq!(app.doc().text.to_string(), DIRTY);
        }
        original_history(&mut app, id);
        unchanged(&root, false);
        app.execute("vscli.extensions.stop", Value::Null);
    }
}
#[test]
fn eight_actual_public_navigation_callbacks_hold_capacity_until_settlement_and_fresh_queries_work()
{
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let before = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let selections = app.doc().selections();
    command(
        &mut app,
        "navigationProviders.capacity",
        "API navigation capacity;actualCallbacks=8;held=true",
    );
    command(
        &mut app,
        "navigationProviders.capacityOverflow",
        "API navigation capacity;ninth=refused;actualStill8=true",
    );
    command(
        &mut app,
        "navigationProviders.capacityRelease",
        "API navigation capacity;originals/fresh=true;actualReleased=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().selections(), selections);
    let after = app.doc().capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(after.text_epoch(), before.text_epoch());
    assert_eq!(after.save_generation(), before.save_generation());
    original_history(&mut app, id);
    unchanged(&root, false);
    app.execute("vscli.extensions.stop", Value::Null);
}
