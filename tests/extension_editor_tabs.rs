//! Actual selected-host tab inventory and native close/save decisions.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Modal, PromptKind},
    extensions::Package,
    keys::Profile,
};
const ORIGINAL: &str = "猫🙂 tabs\r\nlast\r\n";
const DIRTY: &str = "dirty 猫🙂 tabs\r\nlast\r\n";
const UNTITLED: &str = "猫🙂 new\r\ntail\r\n";
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut terminal = Terminal::new(TestBackend::new(140, 32)).unwrap();
        terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Public tabs fixture timeout: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn receipt(app: &mut App, text: &str) {
    until(app, |app| app.message == text);
}
fn command(app: &mut App, id: &str, text: &str) {
    app.execute(id, Value::Null);
    receipt(app, text);
}
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn ready(root: &Path) -> App {
    std::fs::write(root.join("a.cpp"), ORIGINAL).unwrap();
    std::fs::write(root.join("b.txt"), b"other\r\n").unwrap();
    std::fs::write(root.join("c.txt"), b"third\r\n").unwrap();
    let settings = root.join("settings.json");
    std::fs::write(&settings,serde_json::to_vec(&json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}})).unwrap()).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("a.cpp")).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("a.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    let package =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-tabs-extension");
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![Package::read(&package).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready && host.commands.iter().any(|(_, id)| id == "editorTabs.setup")
        })
    });
    app
}
fn disks(root: &Path, a: &str) {
    assert_eq!(std::fs::read(root.join("a.cpp")).unwrap(), a.as_bytes());
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"other\r\n");
    assert_eq!(std::fs::read(root.join("c.txt")).unwrap(), b"third\r\n");
}
fn setup(app: &mut App) {
    command(
        app,
        "editorTabs.setup",
        "API tabs inventory;readonly=true;inputs=7;sharedMemberships=true;events=true",
    );
}
fn peer(app: &mut App) {
    command(
        app,
        "editorTabs.peer",
        "API tabs protected peer;dirty=true;origin=inactive",
    );
}
fn begin_close(app: &mut App, id: &str) {
    app.execute(id, Value::Null);
    until(app, |app| matches!(app.modal, Some(Modal::Confirm(_))));
}
fn peer_history(app: &mut App, id: u64) {
    command(
        app,
        "editorTabs.showPeer",
        "API tabs peer active;identity=true",
    );
    assert_eq!(app.doc().id, id);
    app.execute("undo", Value::Null);
    command(app, "editorTabs.peerClean", "API tabs peer Undo;clean=true");
    assert_eq!(app.doc().text.to_string(), "other\r\n");
    app.execute("redo", Value::Null);
    command(app, "editorTabs.peerDirty", "API tabs peer Redo;dirty=true");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "peer other\r\n");
}
#[test]
fn public_tabs_frozen_live_inventory_shared_memberships_sticky_flags_and_original_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    let cursor = app.doc().cursor;
    setup(&mut app);
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    assert_eq!(app.active_pane, 0);
    assert_eq!(app.doc().cursor, cursor);
    app.execute("workbench.action.pinEditor", Value::Null);
    command(
        &mut app,
        "editorTabs.pinned",
        "API tabs pinned;sticky=true;preview=false",
    );
    app.execute("workbench.action.unpinEditor", Value::Null);
    command(
        &mut app,
        "editorTabs.unpinned",
        "API tabs unpinned;committed=true",
    );
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorTabs.clean",
        "API tabs clean history;identities=true",
    );
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorTabs.history",
        "API tabs dirty shared history;identities=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    disks(&root, ORIGINAL);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_clean_tab_array_group_close_retired_brands_and_real_empty_welcome_group() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    setup(&mut app);
    command(
        &mut app,
        "editorTabs.closeClean",
        "API tabs close clean;array/group=true;arrayPreserveFocus=true;retired/fake=refused;empty=true",
    );
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    assert!(app.modal.is_none());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorTabs.clean",
        "API tabs clean history;identities=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    command(
        &mut app,
        "editorTabs.welcome",
        "API tabs welcome;oneEmptyGroup=true;activeTab=undefined;editors=0",
    );
    assert!(app.active_document().is_none());
    assert!(app.panes.is_empty());
    disks(&root, ORIGINAL);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_dirty_tab_cancel_retains_source_and_discard_never_touches_independent_dirty_peer() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let original_id = app.doc().id;
    let original_proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    let original_cursor = app.doc().cursor;
    peer(&mut app);
    let peer_id = app.doc().id;
    let peer_cursor = app.doc().cursor;
    begin_close(&mut app, "editorTabs.cancel");
    key(&mut app, KeyCode::Esc);
    receipt(
        &mut app,
        "API tabs dirty Cancel=false;source/history=retained;peer=unchanged",
    );
    assert_eq!(app.doc().id, peer_id);
    assert_eq!(app.doc().cursor, peer_cursor);
    command(
        &mut app,
        "editorTabs.original",
        "API tabs original retained;identity/history=true",
    );
    assert_eq!(app.doc().id, original_id);
    assert_eq!(app.doc().cursor, original_cursor);
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), original_proof.text_epoch());
    assert_eq!(current.save_generation(), original_proof.save_generation());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorTabs.clean",
        "API tabs clean history;identities=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorTabs.dirty",
        "API tabs original Redo;dirty=true",
    );
    command(
        &mut app,
        "editorTabs.showPeer",
        "API tabs peer active;identity=true",
    );
    begin_close(&mut app, "editorTabs.discard");
    key(&mut app, KeyCode::Char('d'));
    receipt(
        &mut app,
        "API tabs dirty Discard=true;events=true;peer=unchanged",
    );
    assert_eq!(app.doc().id, peer_id);
    assert_eq!(app.doc().cursor, peer_cursor);
    assert_eq!(app.doc().text.to_string(), "peer other\r\n");
    command(
        &mut app,
        "editorTabs.reopenDiscarded",
        "API tabs discarded reopened;diskBaseline=true;peer=unchanged",
    );
    peer_history(&mut app, peer_id);
    disks(&root, ORIGINAL);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_tab_save_close_actual_receipts_and_untitled_save_as_cancel_accept_preserve_peer_history()
{
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    peer(&mut app);
    let peer_id = app.doc().id;
    begin_close(&mut app, "editorTabs.saved");
    key(&mut app, KeyCode::Char('s'));
    receipt(
        &mut app,
        "API tabs dirty Save=true;receipt/event=true;peer=unchanged",
    );
    disks(&root, DIRTY);
    assert_eq!(app.doc().id, peer_id);
    command(
        &mut app,
        "editorTabs.untitled",
        "API tabs untitled ready;dirty=true;identity=true",
    );
    let untitled_id = app.doc().id;
    assert_eq!(app.doc().text.to_string(), UNTITLED);
    begin_close(&mut app, "editorTabs.cancelUntitled");
    key(&mut app, KeyCode::Char('s'));
    until(&mut app, |app| {
        app.prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
    });
    key(&mut app, KeyCode::Esc);
    receipt(
        &mut app,
        "API tabs untitled SaveAs Cancel=false;identity/source=true",
    );
    assert_eq!(app.doc().id, untitled_id);
    assert_eq!(app.doc().text.to_string(), UNTITLED);
    assert!(app.doc().dirty());
    assert!(!root.join("tabs-saved.cpp").exists());
    begin_close(&mut app, "editorTabs.saveUntitled");
    key(&mut app, KeyCode::Char('s'));
    until(&mut app, |app| {
        app.prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
    });
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));
    app.event(Event::Paste(
        root.join("tabs-saved.cpp").to_string_lossy().into_owned(),
    ));
    key(&mut app, KeyCode::Enter);
    receipt(
        &mut app,
        "API tabs untitled SaveAs=true;close/open/save=true;peer=unchanged",
    );
    assert_eq!(
        std::fs::read(root.join("tabs-saved.cpp")).unwrap(),
        UNTITLED.as_bytes()
    );
    peer_history(&mut app, peer_id);
    disks(&root, DIRTY);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn selected_extension_retirement_cancels_only_its_dirty_tab_close_without_losing_native_models() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let original_id = app.doc().id;
    let original_selections = app.doc().selections();
    let original_proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    let original_member = app.editor_groups().memberships(original_id).next().unwrap();
    peer(&mut app);
    let peer_id = app.doc().id;
    let peer_selections = app.doc().selections();
    let peer_proof = app.doc().capture_save(root.join("b.txt")).unwrap();
    let peer_member = app.editor_groups().memberships(peer_id).next().unwrap();
    begin_close(&mut app, "editorTabs.cancel");
    // This is an actual selected-owner stop while its public close Promise is
    // pending, without a synthetic native frame or a Cancel key response.
    app.execute(
        "vscli.extensions.stopSelected",
        json!("fixture.editor-tabs"),
    );
    until(&mut app, |app| {
        app.extension_host.is_none() && app.modal.is_none()
    });
    assert!(app.prompt.is_none());
    assert!(app.running);
    assert_eq!(app.documents.len(), 2);
    assert_eq!(app.panes.len(), 2);
    assert!(app.editor_groups().membership_current(original_member));
    assert!(app.editor_groups().membership_current(peer_member));
    assert_eq!(app.doc().id, peer_id);
    for (id, text, selections, path, proof) in [
        (
            original_id,
            DIRTY,
            &original_selections,
            root.join("a.cpp"),
            &original_proof,
        ),
        (
            peer_id,
            "peer other\r\n",
            &peer_selections,
            root.join("b.txt"),
            &peer_proof,
        ),
    ] {
        let doc = app.documents.iter().find(|doc| doc.id == id).unwrap();
        assert_eq!(doc.text.to_string(), text);
        assert!(doc.dirty());
        assert_eq!(doc.selections(), *selections);
        let current = doc.capture_save(path).unwrap();
        assert_eq!(current.text_epoch(), proof.text_epoch());
        assert_eq!(current.save_generation(), proof.save_generation());
    }
    app.execute("workbench.action.focusFirstEditorGroup", Value::Null);
    assert_eq!(app.doc().id, original_id);
    assert_eq!(app.doc().selections(), original_selections);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, original_id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    app.execute("workbench.action.focusSecondEditorGroup", Value::Null);
    assert_eq!(app.doc().id, peer_id);
    assert_eq!(app.doc().selections(), peer_selections);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "other\r\n");
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, peer_id);
    assert_eq!(app.doc().text.to_string(), "peer other\r\n");
    assert!(app.doc().dirty());
    disks(&root, ORIGINAL);
}
