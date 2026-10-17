//! Global public symbols and original native navigation use real selected callbacks.
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
const HIDDEN: &str = "β🙂 hidden\r\n";

fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
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
            "Workspace symbols {phase}: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, id: &str, message: &str) {
    app.execute(&format!("workspaceSymbols.{id}"), Value::Null);
    until(app, message, |app| app.message == message);
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn query(app: &mut App, value: &str) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));
    app.event(Event::Paste(value.into()));
}
fn ready(root: &Path, open: bool) -> App {
    std::fs::write(root.join("source.cpp"), SOURCE).unwrap();
    std::fs::write(root.join("target.cpp"), TARGET).unwrap();
    std::fs::write(root.join("hidden.cpp"), HIDDEN).unwrap();
    std::fs::write(root.join("indexed.cpp"), "δ🙂 index\r\n").unwrap();
    let settings = root.join("settings.json");
    std::fs::write(&settings,serde_json::to_vec(&json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}})).unwrap()).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    if open {
        app.open(&root.join("source.cpp")).unwrap();
        until(&mut app, "source open", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_deref() == Some(root.join("source.cpp").as_path()))
        });
        app.doc_mut().insert("dirty ", false);
    }
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/workspace-symbols-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, "selected host ready", |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "workspaceSymbols.queryZero")
        })
    });
    app
}
fn prepared(app: &mut App) -> u64 {
    command(
        app,
        "prepare",
        "API workspace prepared;sharedDirty/hidden=true",
    );
    app.doc().id
}
fn start(app: &mut App, value: &str) {
    app.execute("workbench.action.showAllSymbols", Value::Null);
    until(app, "original global prompt", |app| {
        matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::Symbols)
        )
    });
    query(app, value);
}
fn list(app: &mut App, value: &str) {
    until(app, &format!("actual list {value}"), |app| {
        app.symbol_items(value).len() == 1
            && app.message.starts_with(&format!(
                "1 workspace symbols for '{value}' · Enter navigates"
            ))
    });
}
fn accept(app: &mut App, root: &Path, name: &str) {
    key(app, KeyCode::Enter);
    until(app, &format!("accepted {name}"), |app| {
        app.prompt.is_none()
            && app.modal.is_none()
            && app
                .active_document()
                .is_some_and(|doc| doc.path.as_deref() == Some(root.join(name).as_path()))
    });
}
fn source(app: &mut App) {
    command(app, "source", "API workspace source;identity/dirty=true");
}
fn unchanged(root: &Path) {
    assert_eq!(
        std::fs::read(root.join("source.cpp")).unwrap(),
        SOURCE.as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("target.cpp")).unwrap(),
        TARGET.as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("hidden.cpp")).unwrap(),
        HIDDEN.as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("indexed.cpp")).unwrap(),
        "δ🙂 index\r\n".as_bytes()
    );
}
fn source_history(app: &mut App, id: u64) {
    source(app);
    assert_eq!(app.doc().id, id);
    app.execute("undo", Value::Null);
    command(app, "sourceUndo", "API workspace source Undo;exact=true");
    app.execute("redo", Value::Null);
    command(app, "sourceRedo", "API workspace source Redo;exact=true");
    assert_eq!(app.doc().id, id);
}

#[test]
fn public_workspace_symbols_are_global_without_editors_and_preserve_exact_values_sort_dedup_and_error_isolation()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, false);
    assert!(app.active_document().is_none());
    assert!(app.documents.is_empty());
    let group = app.editor_groups().active_group();
    command(
        &mut app,
        "queryZero",
        "API workspace public;zeroEditors/fullFields/sort/dedup/isolation=true;resolve=0",
    );
    assert!(app.active_document().is_none());
    assert!(app.documents.is_empty());
    assert_eq!(app.editor_groups().active_group(), group);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn original_global_picker_allows_provider_hidden_indexing_without_focus_or_source_mutation_before_accept()
 {
    for opened in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut app = ready(&root, opened);
        if opened {
            prepared(&mut app);
        }
        let original_id = app.active_document().map(|doc| doc.id);
        let active = app.editor_groups().active_membership();
        let group = app.editor_groups().active_group();
        let documents = app.documents.len();
        let prior: Vec<_> = app
            .documents
            .iter()
            .map(|doc| {
                (
                    doc.id,
                    doc.revision,
                    doc.text.to_string(),
                    doc.selections(),
                    doc.capture_save(doc.path.clone().unwrap()).unwrap(),
                    app.editor_groups().memberships(doc.id).collect::<Vec<_>>(),
                )
            })
            .collect();
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        )));
        until(&mut app, "original Ctrl+T indexing prompt", |app| {
            matches!(
                app.prompt.as_ref().map(|prompt| &prompt.kind),
                Some(PromptKind::Symbols)
            )
        });
        query(&mut app, "index");
        list(&mut app, "index");
        // The awaited provider itself verifies the real opened event and mirror
        // state. Invoking another UI command here would close this picker.
        assert!(matches!(
            app.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::Symbols)
        ));
        assert_eq!(app.prompt.as_ref().unwrap().text, "index");
        assert_eq!(app.editor_groups().active_membership(), active);
        assert_eq!(app.editor_groups().active_group(), group);
        assert_eq!(app.documents.len(), documents);
        assert_eq!(app.active_document().is_none(), !opened);
        for (id, revision, text, selections, proof, members) in &prior {
            let doc = app.documents.iter().find(|doc| doc.id == *id).unwrap();
            assert_eq!(doc.revision, *revision);
            assert_eq!(doc.text.to_string(), *text);
            assert_eq!(doc.selections(), *selections);
            let current = doc.capture_save(doc.path.clone().unwrap()).unwrap();
            assert_eq!(current.text_epoch(), proof.text_epoch());
            assert_eq!(current.save_generation(), proof.save_generation());
            for member in members {
                assert!(app.editor_groups().membership_current(*member));
            }
        }
        accept(&mut app, &root, "indexed.cpp");
        command(
            &mut app,
            "indexed",
            "API workspace index;acceptedIdentity/collapsed=true",
        );
        assert_eq!(app.doc().text.to_string(), "δ🙂 index\r\n");
        assert_eq!(app.doc().cursor, 1);
        assert_eq!(app.doc().anchor, None);
        assert!(!app.doc().dirty());
        if opened {
            source_history(&mut app, original_id.unwrap());
            app.execute("workbench.action.focusSecondEditorGroup", Value::Null);
            assert_eq!(app.doc().text.to_string(), format!("dirty {TARGET}"));
            app.execute("undo", Value::Null);
            command(
                &mut app,
                "targetUndo",
                "API workspace target Undo;exact=true",
            );
            app.execute("redo", Value::Null);
            command(
                &mut app,
                "targetRedo",
                "API workspace target Redo;exact=true",
            );
        }
        unchanged(&root);
        app.execute("vscli.extensions.stop", Value::Null);
    }
}

#[test]
fn original_workspace_navigation_resolves_exact_opaque_symbol_into_shared_dirty_and_hidden_models_without_data_loss()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, true);
    let source_id = app.doc().id;
    let source_proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let source_selections = app.doc().selections();
    prepared(&mut app);
    let target_id = app
        .documents
        .iter()
        .find(|doc| doc.path.as_deref() == Some(root.join("target.cpp").as_path()))
        .unwrap()
        .id;
    let original_memberships: Vec<_> = app.editor_groups().memberships(target_id).collect();
    assert_eq!(original_memberships.len(), 2);
    start(&mut app, "lazy");
    list(&mut app, "lazy");
    accept(&mut app, &root, "target.cpp");
    command(
        &mut app,
        "target",
        "API workspace target;originalResolve/collapsed/shared=true",
    );
    assert_eq!(app.doc().id, target_id);
    assert_eq!(app.doc().cursor, 7);
    assert_eq!(app.doc().anchor, None);
    for member in original_memberships {
        assert!(app.editor_groups().membership_current(member));
    }
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "targetUndo",
        "API workspace target Undo;exact=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "targetRedo",
        "API workspace target Redo;exact=true",
    );
    assert_eq!(app.doc().id, target_id);
    source(&mut app);
    start(&mut app, "hidden");
    list(&mut app, "hidden");
    accept(&mut app, &root, "hidden.cpp");
    command(
        &mut app,
        "hidden",
        "API workspace hidden;identity/collapsed=true",
    );
    assert_eq!(app.doc().cursor, 1);
    assert_eq!(app.doc().anchor, None);
    assert_eq!(app.doc().text.to_string(), HIDDEN);
    source(&mut app);
    assert_eq!(app.doc().id, source_id);
    assert_eq!(app.doc().selections(), source_selections);
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(proof.text_epoch(), source_proof.text_epoch());
    assert_eq!(proof.save_generation(), source_proof.save_generation());
    source_history(&mut app, source_id);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn latest_workspace_list_and_disposed_or_cancelled_resolver_reject_late_results_until_actual_settlement()
 {
    for scenario in 0..4 {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut app = ready(&root, true);
        let source_id = prepared(&mut app);
        let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
        let selections = app.doc().selections();
        eprintln!("Workspace held scenario {scenario}");
        if scenario == 0 {
            command(&mut app, "armList", "API workspace held list armed");
            start(&mut app, "held-list");
            until(&mut app, "actual list callback held", |app| {
                app.message == "API workspace held list;entered=true;actualPending=true"
            });
            query(&mut app, "hidden");
            list(&mut app, "hidden");
            // Accept the original latest picker while the obsolete callback is
            // still held. A separate user command deliberately closes pickers.
            accept(&mut app, &root, "hidden.cpp");
            command(
                &mut app,
                "hidden",
                "API workspace hidden;identity/collapsed=true",
            );
            command(
                &mut app,
                "release",
                "API workspace held list;cancelled/actualSettled=true",
            );
            command(
                &mut app,
                "hidden",
                "API workspace hidden;identity/collapsed=true",
            );
        } else {
            command(&mut app, "armResolve", "API workspace held resolve armed");
            start(&mut app, "lazy-resolve");
            list(&mut app, "lazy-resolve");
            key(&mut app, KeyCode::Enter);
            until(&mut app, "actual resolver held", |app| {
                app.message == "API workspace held resolve;entered=true;actualPending=true"
            });
            if scenario == 1 {
                key(&mut app, KeyCode::Esc);
            } else if scenario == 2 {
                command(
                    &mut app,
                    "dispose",
                    "API workspace provider disposed;original=retained",
                );
            } else {
                let alternate = root.join("alternate-settings.json");
                std::fs::copy(root.join("settings.json"), &alternate).unwrap();
                app.configure_settings(Some(alternate)).unwrap();
                app.configure_settings(Some(root.join("settings.json")))
                    .unwrap();
            }
            command(
                &mut app,
                "release",
                "API workspace held resolve;cancelled/actualSettled=true",
            );
            assert!(app.prompt.is_none());
            assert!(app.modal.is_none());
            assert_eq!(app.doc().id, source_id);
            assert_eq!(app.doc().selections(), selections);
        }
        source(&mut app);
        assert_eq!(app.doc().id, source_id);
        let current = app.doc().capture_save(root.join("source.cpp")).unwrap();
        assert_eq!(current.text_epoch(), proof.text_epoch());
        assert_eq!(current.save_generation(), proof.save_generation());
        source_history(&mut app, source_id);
        unchanged(&root);
        app.execute("vscli.extensions.stop", Value::Null);
    }
}

#[test]
fn selected_workspace_provider_retirement_cannot_open_a_held_resolver_or_erase_native_dirty_histories()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, true);
    let source_id = prepared(&mut app);
    let source_proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let source_selections = app.doc().selections();
    let target = app
        .documents
        .iter()
        .find(|doc| doc.path.as_deref() == Some(root.join("target.cpp").as_path()))
        .unwrap();
    let target_id = target.id;
    let target_proof = target.capture_save(root.join("target.cpp")).unwrap();
    let target_selections = target.selections();
    let target_memberships: Vec<_> = app.editor_groups().memberships(target_id).collect();
    command(&mut app, "armResolve", "API workspace held resolve armed");
    start(&mut app, "lazy-resolve");
    list(&mut app, "lazy-resolve");
    key(&mut app, KeyCode::Enter);
    until(&mut app, "actual resolver before owner retirement", |app| {
        app.message == "API workspace held resolve;entered=true;actualPending=true"
    });
    app.execute(
        "vscli.extensions.stopSelected",
        json!("fixture.workspace-symbols"),
    );
    until(&mut app, "selected owner/host retired", |app| {
        app.extension_host.is_none() && app.prompt.is_none() && app.modal.is_none()
    });
    assert!(app.running);
    assert_eq!(app.doc().id, source_id);
    for (id, text, selections, path, proof) in [
        (
            source_id,
            format!("dirty {SOURCE}"),
            source_selections,
            root.join("source.cpp"),
            source_proof,
        ),
        (
            target_id,
            format!("dirty {TARGET}"),
            target_selections,
            root.join("target.cpp"),
            target_proof,
        ),
    ] {
        let doc = app.documents.iter().find(|doc| doc.id == id).unwrap();
        assert_eq!(doc.text.to_string(), text);
        assert_eq!(doc.selections(), selections);
        let current = doc.capture_save(path).unwrap();
        assert_eq!(current.text_epoch(), proof.text_epoch());
        assert_eq!(current.save_generation(), proof.save_generation());
    }
    for member in target_memberships {
        assert!(app.editor_groups().membership_current(member));
    }
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), SOURCE);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), format!("dirty {SOURCE}"));
    app.execute("workbench.action.focusSecondEditorGroup", Value::Null);
    assert_eq!(app.doc().id, target_id);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), TARGET);
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), format!("dirty {TARGET}"));
    unchanged(&root);
}

#[test]
fn rejected_global_dispatch_cannot_publish_partial_success_or_revive_a_held_list() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, true);
    let source_id = prepared(&mut app);
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let selections = app.doc().selections();
    let group = app.editor_groups().active_membership();
    command(&mut app, "armList", "API workspace held list armed");
    start(&mut app, "held-list");
    until(
        &mut app,
        "actual callback before admission refusal",
        |app| app.message == "API workspace held list;entered=true;actualPending=true",
    );
    // This fits the native prompt's byte budget but is invalid at actual
    // provider admission. An oversized paste would stop at the UI boundary.
    let invalid = "invalid\0query";
    query(&mut app, invalid);
    until(&mut app, "explicit whole-cohort dispatch refusal", |app| {
        app.message
            .starts_with("Workspace symbol cohort dispatch rejected:")
    });
    assert!(app.message.contains("query exceeds bounds"));
    assert!(app.symbol_items(invalid).is_empty());
    assert_eq!(app.doc().id, source_id);
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(app.editor_groups().active_membership(), group);
    command(
        &mut app,
        "release",
        "API workspace held list;cancelled/actualSettled=true",
    );
    assert!(app.prompt.is_none());
    assert_eq!(app.doc().id, source_id);
    assert_eq!(app.doc().selections(), selections);
    let current = app.doc().capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    source_history(&mut app, source_id);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn eight_global_queries_retain_actual_capacity_and_ninth_refuses_until_original_callbacks_settle() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root, false);
    command(
        &mut app,
        "capacity",
        "API workspace capacity;actualCallbacks=8;held=true",
    );
    command(
        &mut app,
        "capacityOverflow",
        "API workspace capacity;ninth=refused;actualStill8=true",
    );
    command(
        &mut app,
        "capacityRelease",
        "API workspace capacity;originals/fresh=true;actualReleased=true",
    );
    assert!(app.active_document().is_none());
    assert!(app.documents.is_empty());
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
