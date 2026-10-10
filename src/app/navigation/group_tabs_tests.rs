use super::super::navigation_history::Reason;
use super::*;
use std::time::Duration;

fn file(root: &Path, name: &str, text: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::canonicalize(path).unwrap()
}

#[test]
fn rejected_recent_and_reopen_admission_restore_history_observation() {
    for recent in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let source_text = "猫🙂 body\r\n".repeat(24);
        let source = file(root.path(), "source.cpp", &source_text);
        let target = file(root.path(), "retained-target.cpp", "hidden 猫🙂\r\n");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&source).unwrap();
        let source_id = app.doc().id;
        app.doc_mut().insert("source redo ", false);
        app.doc_mut().undo();
        for _ in 1..crate::editor_groups::MAX_TABS_PER_GROUP {
            app.documents.push(Document::default());
        }
        app.sync_pane();
        let mut hidden = Document::open_existing(&target).unwrap();
        hidden.insert("unsaved hidden ", false);
        let hidden_id = hidden.id;
        let hidden_text = hidden.text.to_string();
        let hidden_epoch = hidden.text_epoch();
        let hidden_saved = hidden.save_generation();
        app.hidden_documents.push(hidden);
        let proof = app.editor_groups().proof();
        assert_eq!(app.editor_groups().groups()[0].tabs().len(), 128);
        assert!(!app.group_fallback);
        if recent {
            app.recent_files.touch(target.clone());
            app.accept_recent("retained-target.cpp", 0);
        } else {
            app.navigation.closed.push(Closed {
                id: 1,
                path: target.clone(),
                row: 0,
                column: 0,
            });
            app.reopen_closed();
            assert_eq!(app.navigation.closed.len(), 1);
        }
        assert!(app.message.contains("rejected"), "{}", app.message);
        assert!(app.navigation.pending.is_none());
        assert_eq!(app.doc().id, source_id);
        assert_eq!(app.doc().cursor, 0);
        assert_eq!(app.doc().text.to_string(), source_text);
        assert_eq!(app.editor_groups().proof(), proof);
        assert_eq!(app.hidden_documents.len(), 1);
        let hidden = &app.hidden_documents[0];
        assert_eq!(hidden.id, hidden_id);
        assert_eq!(hidden.text.to_string(), hidden_text);
        assert_eq!(hidden.text_epoch(), hidden_epoch);
        assert_eq!(hidden.save_generation(), hidden_saved);
        assert!(hidden.dirty());
        // This public helper's returned state checks the failed path restored
        // observation, rather than accepting unrelated old history entries.
        let suppressed = app.suspend_navigation_observation();
        assert!(!suppressed, "admission failure left history suspended");
        app.resume_navigation_observation(suppressed, Reason::Ordinary);

        let position = crate::lsp::Position {
            line: 20,
            character: 1,
        };
        app.language_action(&LanguageAction::Location {
            path: source.clone(),
            range: crate::lsp::Range {
                start: position,
                end: position,
            },
        })
        .unwrap();
        assert_eq!(app.doc().row(), 20);
        assert_eq!(app.doc().column(), 1);
        app.execute("workbench.action.navigateBack", Value::Null);
        assert_eq!(app.doc().id, source_id);
        assert_eq!(app.doc().cursor, 0);
        assert!(app.can_navigate_forward());
        app.execute("workbench.action.navigateForward", Value::Null);
        assert_eq!(app.doc().row(), 20);
        assert_eq!(app.doc().column(), 1);
        assert_eq!(app.doc().text.to_string(), source_text);
        app.doc_mut().redo();
        assert_eq!(
            app.doc().text.to_string(),
            format!("source redo {source_text}")
        );
        assert_eq!(std::fs::read(source).unwrap(), source_text.as_bytes());
        assert_eq!(std::fs::read(target).unwrap(), "hidden 猫🙂\r\n".as_bytes());
    }
}

#[test]
fn rejected_actual_loaded_recent_and_reopen_restore_history_observation() {
    for recent in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let source_text = "猫🙂 body\r\n".repeat(24);
        let source = file(root.path(), "source.cpp", &source_text);
        let target = file(root.path(), "loaded-target.cpp", "loaded 猫🙂\r\n");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&source).unwrap();
        let source_id = app.doc().id;
        app.doc_mut().insert("source redo ", false);
        app.doc_mut().undo();
        for _ in 1..crate::editor_groups::MAX_TABS_PER_GROUP {
            app.documents.push(Document::default());
        }
        app.sync_pane();
        let proof = app.editor_groups().proof();
        assert_eq!(app.editor_groups().groups()[0].tabs().len(), 128);
        assert!(!app.group_fallback);
        if recent {
            app.recent_files.touch(target.clone());
            app.accept_recent("loaded-target.cpp", 0);
        } else {
            app.navigation.closed.push(Closed {
                id: 1,
                path: target.clone(),
                row: 0,
                column: 0,
            });
            app.reopen_closed();
            assert_eq!(app.navigation.closed.len(), 1);
        }
        assert!(
            app.navigation.pending.is_some(),
            "actual new-file worker was not dispatched"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.navigation.pending.is_some() {
            app.poll_navigation();
            assert!(std::time::Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            app.message.contains("File could not be opened"),
            "{}",
            app.message
        );
        assert!(app.message.contains("128 tabs"), "{}", app.message);
        assert_eq!(app.documents.len(), 128);
        if !recent {
            assert_eq!(app.navigation.closed.len(), 1);
        }
        assert_eq!(app.doc().id, source_id);
        assert_eq!(app.doc().cursor, 0);
        assert_eq!(app.doc().text.to_string(), source_text);
        assert_eq!(app.editor_groups().proof(), proof);
        assert!(app.hidden_documents.is_empty());
        // This public helper's returned state checks the failed path restored
        // observation, rather than accepting unrelated old history entries.
        let suppressed = app.suspend_navigation_observation();
        assert!(!suppressed, "admission failure left history suspended");
        app.resume_navigation_observation(suppressed, Reason::Ordinary);

        let position = crate::lsp::Position {
            line: 20,
            character: 1,
        };
        app.language_action(&LanguageAction::Location {
            path: source.clone(),
            range: crate::lsp::Range {
                start: position,
                end: position,
            },
        })
        .unwrap();
        assert_eq!(app.doc().row(), 20);
        assert_eq!(app.doc().column(), 1);
        app.execute("workbench.action.navigateBack", Value::Null);
        assert_eq!(app.doc().id, source_id);
        assert_eq!(app.doc().cursor, 0);
        assert!(app.can_navigate_forward());
        app.execute("workbench.action.navigateForward", Value::Null);
        assert_eq!(app.doc().row(), 20);
        assert_eq!(app.doc().column(), 1);
        assert_eq!(app.doc().text.to_string(), source_text);
        app.doc_mut().redo();
        assert_eq!(
            app.doc().text.to_string(),
            format!("source redo {source_text}")
        );
        assert_eq!(std::fs::read(source).unwrap(), source_text.as_bytes());
        assert_eq!(std::fs::read(target).unwrap(), "loaded 猫🙂\r\n".as_bytes());
    }
}

#[test]
fn actual_alias_response_rejects_public_shared_group_focus_aba() {
    let root = tempfile::tempdir().unwrap();
    let source_text = "猫🙂 source\r\nsecond\r\n";
    let target_text = "hidden 猫🙂\r\n";
    let source = file(root.path(), "source.cpp", source_text);
    let target = file(root.path(), "target.cpp", target_text);
    let alias_dir = root.path().join("alias-prefix");
    std::fs::create_dir(&alias_dir).unwrap();
    let alias = alias_dir.join("..").join("target.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&source).unwrap();
    app.doc_mut().insert("redo ", false);
    app.doc_mut().undo();
    app.doc_mut().move_to(1, false);
    app.execute("workbench.action.splitEditor", Value::Null);
    let mut hidden = Document::open_existing(&target).unwrap();
    hidden.insert("unsaved ", false);
    let hidden_id = hidden.id;
    let hidden_text = hidden.text.to_string();
    let hidden_epoch = hidden.text_epoch();
    app.hidden_documents.push(hidden);
    let source_id = app.doc().id;
    let epoch = app.doc().text_epoch();
    let revision = app.doc().revision;
    let saved_revision = app.doc().saved_revision;
    let saved = app.doc().save_generation();
    let selections = app.doc().selections();
    let pane = app.active_pane;
    app.open(&alias).unwrap();
    let pending = app
        .navigation
        .pending
        .take()
        .expect("actual alias resolver dispatched");
    let original_context = pending.context.clone();
    let original_generation = app.navigation.generation;
    let original_interaction = app.navigation.interaction;
    let proof = app.editor_groups().proof();

    // Hold the *actual* canonical-alias result at its delivery boundary. The
    // bounded proxy forwards the unchanged worker response only after release.
    let (sender, receiver) = mpsc::sync_channel(1);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let Pending {
        receiver: actual,
        context,
        closed,
        intent,
    } = pending;
    app.navigation.pending = Some(Pending {
        receiver,
        context,
        closed,
        intent,
    });
    let target_for_worker = target.clone();
    let proxy = std::thread::spawn(move || {
        let result = actual.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(&result, Ok(Target::Existing(path)) if path == &target_for_worker));
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        sender.send(result).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    app.focus_pane(0);
    app.focus_pane(pane);
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.doc().id, source_id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text_epoch(), epoch);
    assert_eq!(app.doc().saved_revision, saved_revision);
    assert_eq!(app.doc().save_generation(), saved);
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(app.doc().text.to_string(), source_text);
    assert_eq!(app.navigation.generation, original_generation);
    assert_eq!(app.navigation.interaction, original_interaction);
    assert!(!app.editor_groups().proof_current(&proof));
    assert!(app.navigation_context() != original_context);
    app.poll_navigation();
    assert!(app.navigation.pending.is_some());
    assert_eq!(app.hidden_documents[0].id, hidden_id);
    release_tx.send(()).unwrap();
    proxy.join().unwrap();
    app.poll_navigation();
    assert!(app.navigation.pending.is_none());
    assert_eq!(app.doc().id, source_id);
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.doc().selections(), selections);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.hidden_documents.len(), 1);
    assert_eq!(app.hidden_documents[0].id, hidden_id);
    assert_eq!(app.hidden_documents[0].text.to_string(), hidden_text);
    assert_eq!(app.hidden_documents[0].text_epoch(), hidden_epoch);
    assert!(app.hidden_documents[0].dirty());
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), format!("redo {source_text}"));
    assert_eq!(std::fs::read(source).unwrap(), source_text.as_bytes());
    assert_eq!(std::fs::read(target).unwrap(), target_text.as_bytes());
}
