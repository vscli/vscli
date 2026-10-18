use super::*;
use std::{fs, thread};

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    root: PathBuf,
    original: PathBuf,
    destination: PathBuf,
    peer: u64,
}
fn spin(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !predicate(app) {
        app.poll();
        assert!(Instant::now() < until, "{}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}
impl Fixture {
    fn new(mode: &str, timeout: u64) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(directory.path()).unwrap();
        let root = base.join("workspace");
        let profile = base.join("profile");
        let extension = base.join("extension");
        for directory in [&root, &profile, &extension] {
            fs::create_dir(directory).unwrap();
        }
        let original = root.join("a.cpp");
        let destination = root.join("b.cpp");
        fs::write(&original, "original 猫🙂\r\n").unwrap();
        fs::write(root.join("peer.cpp"), "peer 猫🙂\r\n").unwrap();
        fs::write(root.join("mode"), mode).unwrap();
        fs::write(
            profile.join("settings.json"),
            serde_json::to_vec(&json!({
                "vscli.languageServer.enabled": false, "files.participants.timeout": timeout,
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            extension.join("package.json"),
            include_str!("../../../tests/fixtures/file-participants-extension/package.json"),
        )
        .unwrap();
        fs::write(
            extension.join("main.cjs"),
            include_str!("../../../tests/fixtures/file-participants-extension/main.cjs"),
        )
        .unwrap();
        let mut app = App::new(root.clone(), Profile::Linux);
        app.configure_settings(Some(profile.join("settings.json")))
            .unwrap();
        app.open(&original).unwrap();
        app.doc_mut().insert("dirty ", false);
        app.split_editor(true);
        let peer_doc = Document::open_existing(&root.join("peer.cpp")).unwrap();
        let peer = peer_doc.id;
        app.hidden_documents.push(peer_doc);
        app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
            .unwrap();
        spin(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        Self {
            _directory: directory,
            app,
            root,
            original,
            destination,
            peer,
        }
    }
    fn start(&mut self) {
        self.app.start_file_job(Action::Rename {
            from: self.original.clone(),
            to: self.destination.clone(),
        });
    }
    fn review(&mut self) {
        self.start();
        spin(&mut self.app, |app| {
            matches!(app.modal, Some(Modal::FileEditReview(_)))
        });
    }
    fn finish(&mut self) {
        spin(&mut self.app, |app| {
            !app.file_participation_pending() && app.file_job.is_none()
        });
    }
    fn records(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
#[test]
fn public_will_review_applies_shared_and_hidden_text_before_original_rename_and_did_event() {
    let mut f = Fixture::new("text", 60_000);
    let id = f.app.doc().id;
    f.review();
    assert!(f.original.exists() && !f.destination.exists());
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    assert!(f.app.request_native_save(None).is_err());
    f.app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    f.finish();
    spin(&mut f.app, |_| {
        fs::read_to_string(f.root.join("log")).is_ok_and(|log| log.contains("\"did\":true"))
    });
    assert_eq!(f.app.doc().id, id);
    assert_eq!(f.app.doc().path.as_ref(), Some(&f.destination));
    assert_eq!(
        f.app.doc().text.to_string(),
        "participant dirty original 猫🙂\r\n",
        "{}",
        f.app.message
    );
    assert!(f.app.panes.iter().all(|pane| pane.document == id));
    let peer = |app: &App| {
        app.hidden_documents
            .iter()
            .find(|doc| doc.id == f.peer)
            .unwrap()
            .text
            .to_string()
    };
    assert_eq!(peer(&f.app), "participant peer 猫🙂\r\n");
    assert_eq!(
        f.records()[1]["text"],
        "participant dirty original 猫🙂\r\n"
    );
    assert_eq!(f.records()[1]["peer"], "participant peer 猫🙂\r\n");
    f.app.undo_editor();
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    assert_eq!(peer(&f.app), "peer 猫🙂\r\n");
    f.app.redo_editor();
    assert_eq!(
        f.app.doc().text.to_string(),
        "participant dirty original 猫🙂\r\n"
    );
    assert_eq!(
        fs::read(&f.destination).unwrap(),
        "original 猫🙂\r\n".as_bytes()
    );
    assert_eq!(
        fs::read(f.root.join("peer.cpp")).unwrap(),
        "peer 猫🙂\r\n".as_bytes()
    );
}
#[test]
fn preview_uses_dirty_original_ranges_and_skip_keeps_history_and_original_operation() {
    let mut f = Fixture::new("text", 60_000);
    f.review();
    f.app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    f.app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let Some(Modal::FileEditReview(dialog)) = &f.app.modal else {
        panic!("review missing")
    };
    assert!(
        dialog
            .details()
            .iter()
            .any(|line| line.contains("proposed replacement"))
    );
    assert!(
        dialog
            .details()
            .iter()
            .any(|line| line.contains("+ participant "))
    );
    assert!(
        !dialog
            .details()
            .iter()
            .any(|line| line.contains("baseline diff is not loaded"))
    );
    f.app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    f.finish();
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    f.app.undo_editor();
    assert_eq!(f.app.doc().text.to_string(), "original 猫🙂\r\n");
    f.app.redo_editor();
    assert_eq!(
        fs::read(f.destination).unwrap(),
        "original 猫🙂\r\n".as_bytes()
    );
}
#[test]
fn invalid_original_destination_is_refused_before_any_extension_callback_or_edit() {
    let mut f = Fixture::new("text", 60_000);
    fs::write(&f.destination, "existing destination").unwrap();
    f.start();
    f.finish();
    assert!(f.records().is_empty());
    assert!(
        f.app.message.contains("before callbacks"),
        "{}",
        f.app.message
    );
    assert_eq!(fs::read(f.destination).unwrap(), b"existing destination");
    assert_eq!(
        fs::read(f.original).unwrap(),
        "original 猫🙂\r\n".as_bytes()
    );
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
}
#[test]
fn review_target_edit_undo_aba_cancels_original_operation_and_preserves_redo() {
    let mut f = Fixture::new("text", 60_000);
    f.review();
    f.app.doc_mut().insert("independent ", false);
    f.app.doc_mut().undo();
    f.finish();
    assert!(f.original.exists() && !f.destination.exists());
    assert!(f.app.modal.is_none());
    f.app.doc_mut().redo();
    assert_eq!(
        f.app.doc().text.to_string(),
        "dirty independent original 猫🙂\r\n"
    );
}
#[test]
fn external_original_write_during_review_prevents_text_apply_and_file_effects() {
    let mut f = Fixture::new("text", 60_000);
    f.review();
    fs::write(&f.original, "external replacement").unwrap();
    f.app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    f.finish();
    assert!(f.original.exists() && !f.destination.exists());
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    assert_eq!(fs::read(f.original).unwrap(), b"external replacement");
}
#[test]
fn unsupported_mixed_contribution_refuses_whole_additional_edit_without_losing_original_dto() {
    let mut f = Fixture::new("mixed", 60_000);
    f.review();
    let Some(Modal::FileEditReview(dialog)) = &f.app.modal else {
        unreachable!()
    };
    assert_eq!(
        dialog.overview().original()["edits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    f.finish();
    assert!(f.destination.exists() && !f.root.join("extra.cpp").exists());
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    assert!(
        f.app.message.contains("resource/notebook edits require"),
        "{}",
        f.app.message
    );
}
#[test]
fn timeout_and_escape_continue_original_operation_but_retain_actual_callback_capacity() {
    for cancel in [false, true] {
        let mut f = Fixture::new("hold", if cancel { 60_000 } else { 100 });
        f.start();
        spin(&mut f.app, |_| f.root.join("log").exists());
        if cancel {
            f.app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        f.finish();
        assert!(f.destination.exists());
        assert!(
            !f.app
                .extension_host
                .as_ref()
                .unwrap()
                .file_participation_available()
        );
        assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
        fs::write(f.root.join("release"), "release").unwrap();
        spin(&mut f.app, |app| {
            app.extension_host
                .as_ref()
                .unwrap()
                .file_participation_available()
        });
    }
}
#[test]
fn zero_timeout_disables_callbacks_while_preserving_native_rename_and_dirty_model() {
    let mut f = Fixture::new("text", 0);
    f.start();
    f.finish();
    assert!(f.destination.exists());
    assert!(!f.records().iter().any(|record| record["will"] == true));
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
}

#[test]
fn remembered_apply_bypasses_initial_dialog_but_confirmation_still_requires_preview() {
    for mode in ["text", "confirm"] {
        let mut f = Fixture::new(mode, 60_000);
        f.app.write_file_participant_choice(Some(false)).unwrap();
        spin(&mut f.app, |app| {
            app.message == "File-operation preview choice remembered"
        });
        f.start();
        if mode == "confirm" {
            spin(&mut f.app, |app| {
                matches!(app.modal, Some(Modal::FileEditReview(_)))
            });
            let Some(Modal::FileEditReview(dialog)) = &f.app.modal else {
                unreachable!()
            };
            assert_eq!(
                dialog.phase(),
                crate::file_edit_review::dialog::Phase::Preview
            );
            assert!(f.original.exists() && !f.destination.exists());
            // Rows -> Details -> Apply. The user's explicit reviewed selection
            // is the only permitted way to bypass the annotation confirmation.
            for code in [KeyCode::Tab, KeyCode::Tab, KeyCode::Enter] {
                f.app.key(KeyEvent::new(code, KeyModifiers::NONE));
            }
        }
        f.finish();
        assert!(f.app.modal.is_none());
        assert_eq!(
            f.app.doc().text.to_string(),
            "participant dirty original 猫🙂\r\n"
        );
    }
}
#[test]
fn settings_profile_aba_cancels_review_without_applying_edits_or_original_operation() {
    let mut f = Fixture::new("text", 60_000);
    f.review();
    let original_profile = f.app.user_settings_path().unwrap();
    f.app
        .configure_settings(Some(f.root.join("other-settings.json")))
        .unwrap();
    f.app.configure_settings(Some(original_profile)).unwrap();
    f.finish();
    assert!(f.original.exists() && !f.destination.exists());
    assert_eq!(f.app.doc().text.to_string(), "dirty original 猫🙂\r\n");
    assert!(f.app.modal.is_none());
}
