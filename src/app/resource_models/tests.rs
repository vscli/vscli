use super::*;
use std::{sync::mpsc, thread};

fn fixture() -> (tempfile::TempDir, App, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let path = root.join("source.cpp");
    std::fs::write(&path, "original 猫🙂\r\n").unwrap();
    let mut app = App::new(root, Profile::Linux);
    app.open(&path).unwrap();
    (directory, app, path)
}
fn settle(app: &mut App) {
    let until = Instant::now() + Duration::from_secs(10);
    while app.resource_models_pending() {
        app.poll_resource_models();
        assert!(Instant::now() < until, "{}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn actual_background_revert_keeps_shared_identity_eol_history_and_original_saved_bytes() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("dirty λ ", false);
    let id = app.doc().id;
    let generation = app.doc().save_generation();
    let save = app.doc().capture_save(path.clone()).unwrap();
    app.split_editor(true);
    std::fs::write(&path, "external 猫🙂\n").unwrap();
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    assert!(app.resource_models_pending());
    settle(&mut app);
    assert_eq!(app.message, "Reloaded from disk");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "external 猫🙂\n");
    assert_eq!(app.doc().eol, "\n");
    assert_eq!(app.doc().save_generation(), generation + 1);
    assert!(!app.doc().dirty());
    assert!(app.doc().check_save_snapshot(&save).is_err());
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "dirty λ original 猫🙂\r\n");
    assert_eq!(app.doc().eol, "\r\n");
    assert!(app.doc().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "external 猫🙂\n".as_bytes());
    app.doc_mut().redo();
    assert!(!app.doc().dirty());
    assert_eq!(app.doc().eol, "\n");
}
#[test]
fn edit_undo_rejects_held_reload_and_retains_actual_worker_slot_until_exit() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("dirty ", false);
    let before = app.doc().text.clone();
    let baseline = app.doc().disk_content.clone();
    let (release, gate) = mpsc::channel();
    let (entered, ready) = mpsc::sync_channel(1);
    app.resource_models.gate = Some(ReloadGate {
        entered,
        release: gate,
    });
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    app.doc_mut().insert("independent ", false);
    app.doc_mut().undo();
    app.poll_resource_models();
    assert!(app.resource_models_pending());
    assert!(app.resource_models.run.as_ref().unwrap().retired);
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    assert!(app.message.contains("still occupied"));
    assert!(app.resource_models_pending());
    release.send(()).unwrap();
    settle(&mut app);
    assert_eq!(app.doc().text, before);
    assert_eq!(app.doc().disk_content, baseline);
    app.doc_mut().redo();
    assert_eq!(
        app.doc().text.to_string(),
        "dirty independent original 猫🙂\r\n"
    );
    assert_eq!(std::fs::read(path).unwrap(), "original 猫🙂\r\n".as_bytes());
}
#[test]
fn navigation_is_independent_but_original_language_aba_retires_held_revert() {
    let (_directory, mut app, path) = fixture();
    let (release, gate) = mpsc::channel();
    let (entered, ready) = mpsc::sync_channel(1);
    app.resource_models.gate = Some(ReloadGate {
        entered,
        release: gate,
    });
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    app.doc_mut().move_to(3, false);
    assert!(
        app.resource_reload_current(app.resource_models.run.as_ref().unwrap())
            .is_ok()
    );
    app.doc_mut()
        .set_language_override(Some("rust".into()))
        .unwrap();
    app.doc_mut().set_language_override(None).unwrap();
    app.poll_resource_models();
    release.send(()).unwrap();
    settle(&mut app);
    assert!(app.message.contains("original buffer retained"));
    assert_eq!(app.doc().cursor, 3);
    assert_eq!(app.doc().text.to_string(), "original 猫🙂\r\n");
    assert_eq!(std::fs::read(path).unwrap(), "original 猫🙂\r\n".as_bytes());
}
#[test]
fn missing_file_refuses_without_discarding_dirty_text_or_its_baseline_history() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("dirty ", false);
    let original = app.doc().capture_resource_model().unwrap();
    let baseline = app.doc().disk_content.clone();
    std::fs::remove_file(path).unwrap();
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    settle(&mut app);
    assert!(app.message.contains("original buffer retained"));
    original.check_current(app.doc()).unwrap();
    assert_eq!(app.doc().disk_content, baseline);
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "original 猫🙂\r\n");
}

#[test]
fn explicit_save_waits_for_original_reader_and_does_not_overwrite_revert_disk_contents() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("dirty ", false);
    let (release, gate) = mpsc::channel();
    let (entered, ready) = mpsc::sync_channel(1);
    app.resource_models.gate = Some(ReloadGate {
        entered,
        release: gate,
    });
    std::fs::write(&path, "external\n").unwrap();
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(app.document_save_pending(app.doc().id));
    assert!(app.request_native_save(None).is_err());
    assert!(!app.saves_pending());
    assert_eq!(std::fs::read(&path).unwrap(), b"external\n");
    release.send(()).unwrap();
    settle(&mut app);
    assert_eq!(app.doc().text.to_string(), "external\n");
}

#[test]
fn closed_original_model_cannot_publish_into_new_same_path_identity() {
    let (_directory, mut app, path) = fixture();
    let original = app.doc().id;
    let (release, gate) = mpsc::channel();
    let (entered, ready) = mpsc::sync_channel(1);
    app.resource_models.gate = Some(ReloadGate {
        entered,
        release: gate,
    });
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    app.remove_active();
    app.open(&path).unwrap();
    assert_ne!(app.doc().id, original);
    let replacement = app.doc().capture_resource_model().unwrap();
    app.poll_resource_models();
    release.send(()).unwrap();
    settle(&mut app);
    replacement.check_current(app.doc()).unwrap();
    assert!(app.message.contains("original buffer retained"));
}

#[test]
fn original_settings_profile_change_refuses_prepared_native_reload_without_replacing_bytes() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("dirty ", false);
    let original = app.doc().capture_resource_model().unwrap();
    let (release, gate) = mpsc::channel();
    let (entered, ready) = mpsc::sync_channel(1);
    app.resource_models.gate = Some(ReloadGate {
        entered,
        release: gate,
    });
    app.start_resource_model_reload(app.doc().id, app.doc().resource_epoch());
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    app.settings_error = Some("new settings failed".into());
    app.poll_resource_models();
    release.send(()).unwrap();
    settle(&mut app);
    original.check_current(app.doc()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), "original 猫🙂\r\n".as_bytes());
    assert!(app.message.contains("original buffer retained"));
}

#[test]
fn revert_confirmation_keeps_original_document_when_another_editor_opens_behind_it() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("original dirty ", false);
    let original = app.doc().id;
    app.execute("workbench.action.files.revert", Value::Null);
    let other = path.parent().unwrap().join("other.cpp");
    std::fs::write(&other, "other data\n").unwrap();
    app.open(&other).unwrap();
    app.doc_mut().insert("other dirty ", false);
    let other_id = app.doc().id;
    let other_snapshot = app.doc().capture_resource_model().unwrap();
    std::fs::write(&path, "original external\n").unwrap();
    app.event(crossterm::event::Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert_eq!(
        app.resource_models
            .run
            .as_ref()
            .unwrap()
            .source
            .proof()
            .document,
        original
    );
    settle(&mut app);
    assert_eq!(app.doc().id, other_id);
    other_snapshot.check_current(app.doc()).unwrap();
    let restored = app
        .documents
        .iter()
        .find(|document| document.id == original)
        .unwrap();
    assert_eq!(restored.text.to_string(), "original external\n");
    assert!(!restored.dirty());
    assert_eq!(std::fs::read(other).unwrap(), b"other data\n");
}

#[test]
fn resource_path_aba_during_confirmation_refuses_before_starting_disk_read() {
    let (_directory, mut app, path) = fixture();
    app.doc_mut().insert("dirty ", false);
    app.execute("workbench.action.files.revert", Value::Null);
    app.doc_mut()
        .set_resource_path(Some(path.with_file_name("other.cpp")))
        .unwrap();
    app.doc_mut().set_resource_path(Some(path.clone())).unwrap();
    app.event(crossterm::event::Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert!(!app.resource_models_pending());
    assert!(app.message.contains("Original confirmed resource changed"));
    assert_eq!(app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    assert_eq!(std::fs::read(path).unwrap(), "original 猫🙂\r\n".as_bytes());
}
