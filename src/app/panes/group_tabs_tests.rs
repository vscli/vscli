use super::*;
use std::{
    fs,
    time::{Duration, Instant},
};

fn command(app: &mut App, id: &str) {
    app.execute(id, Value::Null);
}
fn answer(app: &mut App, ch: char) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char(ch),
        KeyModifiers::NONE,
    )));
}
fn file(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, "猫🙂 value\r\nnext\r\n").unwrap();
    fs::canonicalize(path).unwrap()
}
fn group_documents(app: &App, index: usize) -> Vec<u64> {
    app.editor_groups.groups()[index]
        .tabs()
        .iter()
        .map(|tab| tab.document())
        .collect()
}
fn group_names(app: &App, index: usize) -> Vec<String> {
    group_documents(app, index)
        .iter()
        .map(|id| {
            app.documents
                .iter()
                .find(|doc| doc.id == *id)
                .unwrap()
                .name()
        })
        .collect()
}
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn committed_tabs_insert_right_and_recent_close_chooses_previously_active_not_neighbor() {
    let root = tempfile::tempdir().unwrap();
    let paths = ["a.cpp", "b.cpp", "c.cpp", "d.cpp"].map(|name| file(root.path(), name));
    let mut app = App::new(root.path().into(), Profile::Linux);
    for path in &paths[..3] {
        app.open(path).unwrap();
    }
    app.open(&paths[1]).unwrap();
    app.open(&paths[3]).unwrap();
    assert_eq!(
        group_names(&app, 0),
        vec!["a.cpp", "b.cpp", "d.cpp", "c.cpp"]
    );
    app.open(&paths[2]).unwrap();
    app.open(&paths[0]).unwrap();
    app.open(&paths[1]).unwrap();
    let before = app
        .documents
        .iter()
        .find(|doc| doc.path.as_ref() == Some(&paths[0]))
        .unwrap()
        .id;
    command(&mut app, "workbench.action.closeActiveEditor");
    assert!(app.modal.is_none());
    assert_eq!(group_names(&app, 0), vec!["a.cpp", "d.cpp", "c.cpp"]);
    assert_eq!(app.doc().id, before);
    assert_eq!(
        fs::read(&paths[1]).unwrap(),
        "猫🙂 value\r\nnext\r\n".as_bytes()
    );
}

#[test]
fn current_group_opens_and_global_navigation_differ_from_in_group_wrapping() {
    let root = tempfile::tempdir().unwrap();
    let a = file(root.path(), "a.cpp");
    let b = file(root.path(), "b.cpp");
    let c = file(root.path(), "c.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&a).unwrap();
    let a_id = app.doc().id;
    app.open(&b).unwrap();
    let b_id = app.doc().id;
    command(&mut app, "workbench.action.splitEditor");
    app.open(&c).unwrap();
    let c_id = app.doc().id;
    assert_eq!(group_documents(&app, 0), vec![a_id, b_id]);
    assert_eq!(group_documents(&app, 1), vec![b_id, c_id]);
    app.focus_pane(0);
    for (id, group, doc) in [
        ("workbench.action.nextEditorInGroup", 0, a_id),
        ("workbench.action.previousEditorInGroup", 0, b_id),
        ("workbench.action.nextEditor", 1, b_id),
        ("workbench.action.nextEditor", 1, c_id),
        ("workbench.action.nextEditor", 0, a_id),
        ("workbench.action.previousEditor", 1, c_id),
    ] {
        command(&mut app, id);
        assert_eq!(app.active_pane, group, "{id}");
        assert_eq!(app.doc().id, doc, "{id}");
    }
    assert_eq!(app.documents.len(), 3);
}

#[test]
fn split_copies_view_but_first_ordinary_membership_starts_at_zero_and_history_stays_shared() {
    let root = tempfile::tempdir().unwrap();
    let a = file(root.path(), "a.cpp");
    let b = file(root.path(), "b.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&a).unwrap();
    let a_id = app.doc().id;
    app.doc_mut().move_to(2, false);
    app.open(&b).unwrap();
    let b_id = app.doc().id;
    app.doc_mut().move_to(4, false);
    app.open(&a).unwrap();
    command(&mut app, "workbench.action.splitEditor");
    assert_eq!(app.doc().cursor, 2);
    app.doc_mut().move_to(3, false);
    app.open(&b).unwrap();
    assert_eq!(app.doc().id, b_id);
    assert_eq!(app.doc().cursor, 0);
    app.focus_pane(0);
    app.open(&b).unwrap();
    assert_eq!(app.doc().cursor, 4);
    app.open(&a).unwrap();
    assert_eq!(app.doc().cursor, 2);
    app.focus_pane(1);
    app.open(&a).unwrap();
    assert_eq!(app.doc().cursor, 3);
    app.doc_mut().insert("!", false);
    let edited = app.doc().text.to_string();
    app.focus_pane(0);
    app.open(&a).unwrap();
    assert_eq!(app.doc().id, a_id);
    assert_eq!(app.doc().text.to_string(), edited);
    app.doc_mut().undo();
    assert_eq!(app.doc().text, "猫🙂 value\r\nnext\r\n");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), edited);
    assert_eq!(app.documents.len(), 2);
    assert_eq!(fs::read(&a).unwrap(), "猫🙂 value\r\nnext\r\n".as_bytes());
}

#[test]
fn shared_dirty_tab_close_keeps_model_and_last_dirty_tab_requires_review() {
    let root = tempfile::tempdir().unwrap();
    let a = file(root.path(), "a.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&a).unwrap();
    let id = app.doc().id;
    app.doc_mut().insert("DIRTY λ🙂 ", false);
    let edited = app.doc().text.to_string();
    command(&mut app, "workbench.action.splitEditor");
    command(&mut app, "workbench.action.closeActiveEditor");
    assert!(app.modal.is_none());
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    assert_eq!(app.doc().text.to_string(), edited);
    command(&mut app, "workbench.action.closeActiveEditor");
    assert!(matches!(app.modal, Some(Modal::Confirm(AfterSave::Close))));
    let member = app.active_tab_membership().unwrap();
    answer(&mut app, 'c');
    assert!(app.modal.is_none());
    assert!(app.editor_groups.membership_current(member));
    app.doc_mut().undo();
    assert_eq!(app.doc().text, "猫🙂 value\r\nnext\r\n");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), edited);
    assert_eq!(fs::read(a).unwrap(), "猫🙂 value\r\nnext\r\n".as_bytes());
}

#[test]
fn close_group_save_then_cancel_and_discard_preserves_shared_dirty_model_and_real_receipt() {
    let root = tempfile::tempdir().unwrap();
    let a = file(root.path(), "a.cpp");
    let b = file(root.path(), "b.cpp");
    let c = file(root.path(), "c.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&a).unwrap();
    let a_id = app.doc().id;
    app.doc_mut().insert("A unsaved ", false);
    let a_text = app.doc().text.to_string();
    command(&mut app, "workbench.action.splitEditor");
    app.open(&b).unwrap();
    let b_id = app.doc().id;
    app.doc_mut().insert("B saved λ🙂 ", false);
    let b_text = app.doc().text.to_string();
    app.open(&c).unwrap();
    let c_id = app.doc().id;
    app.doc_mut().insert("C discard ", false);
    let c_text = app.doc().text.to_string();
    command(&mut app, "workbench.action.closeEditorsInGroup");
    assert!(matches!(app.modal, Some(Modal::Confirm(AfterSave::Close))));
    assert_eq!(app.doc().id, b_id);
    answer(&mut app, 's');
    until(&mut app, |app| {
        app.doc().id == c_id && matches!(app.modal, Some(Modal::Confirm(AfterSave::Close)))
    });
    assert_eq!(fs::read(&b).unwrap(), b_text.as_bytes());
    assert!(!app.documents.iter().any(|doc| doc.id == b_id));
    answer(&mut app, 'c');
    assert!(app.closing_group.is_none());
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.doc().id, c_id);
    assert_eq!(app.doc().text.to_string(), c_text);
    assert_eq!(app.editor_groups.memberships(a_id).count(), 2);
    command(&mut app, "workbench.action.closeEditorsInGroup");
    assert_eq!(app.doc().id, c_id);
    answer(&mut app, 'd');
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.doc().id, a_id);
    assert_eq!(app.doc().text.to_string(), a_text);
    assert!(app.doc().dirty());
    app.doc_mut().undo();
    assert_eq!(app.doc().text, "猫🙂 value\r\nnext\r\n");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), a_text);
    assert_eq!(fs::read(a).unwrap(), "猫🙂 value\r\nnext\r\n".as_bytes());
    assert_eq!(fs::read(c).unwrap(), "猫🙂 value\r\nnext\r\n".as_bytes());
}

#[test]
fn accepted_new_membership_retires_old_group_close_batch_without_sweeping_new_tab() {
    let root = tempfile::tempdir().unwrap();
    let a = file(root.path(), "a.cpp");
    let b = file(root.path(), "b.cpp");
    let c = file(root.path(), "c.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&a).unwrap();
    app.doc_mut().insert("A dirty ", false);
    app.open(&b).unwrap();
    app.doc_mut().insert("B dirty ", false);
    command(&mut app, "workbench.action.closeEditorsInGroup");
    assert!(app.closing_group.is_some());
    assert!(matches!(app.modal, Some(Modal::Confirm(_))));
    let old = app.closing_group.as_ref().unwrap().proof.clone();
    app.open(&c).unwrap();
    assert!(!app.editor_groups.group_proof_current(&old));
    app.advance_close_editor_group();
    assert!(app.closing_group.is_none());
    assert!(app.modal.is_none());
    assert_eq!(group_names(&app, 0).len(), 3);
    assert_eq!(app.doc().path.as_ref(), Some(&c));
    assert!(app.documents.iter().filter(|doc| doc.dirty()).count() == 2);
    for path in [a, b, c] {
        assert_eq!(fs::read(path).unwrap(), "猫🙂 value\r\nnext\r\n".as_bytes());
    }
}

#[test]
fn full_group_rejections_preserve_hidden_dirty_model_new_source_and_existing_history() {
    let root = tempfile::tempdir().unwrap();
    let a = file(root.path(), "a.cpp");
    let hidden = file(root.path(), "hidden.cpp");
    let new = file(root.path(), "new.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&a).unwrap();
    let id = app.doc().id;
    app.doc_mut().insert("A unsaved ", false);
    let edited = app.doc().text.to_string();
    for _ in 1..crate::editor_groups::MAX_TABS_PER_GROUP {
        app.documents.push(Document::default());
    }
    app.sync_pane();
    assert_eq!(group_documents(&app, 0).len(), 128);
    assert!(!app.group_fallback);
    let mut retained = Document::open(&hidden).unwrap();
    retained.insert("HIDDEN λ🙂 ", false);
    let hidden_id = retained.id;
    let hidden_text = retained.text.to_string();
    app.hidden_documents.push(retained);
    let proof = app.editor_groups.proof();
    let active = app.doc().id;
    assert!(app.open(&hidden).is_err());
    assert!(
        app.install_open_document(Document::open(&new).unwrap())
            .is_err()
    );
    command(&mut app, "workbench.action.files.newUntitledFile");
    assert!(app.editor_groups.proof_current(&proof));
    assert_eq!(app.documents.len(), 128);
    assert_eq!(app.doc().id, active);
    assert_eq!(app.hidden_documents.len(), 1);
    assert_eq!(app.hidden_documents[0].id, hidden_id);
    assert_eq!(app.hidden_documents[0].text.to_string(), hidden_text);
    app.open(&a).unwrap();
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), edited);
    app.doc_mut().undo();
    assert_eq!(app.doc().text, "猫🙂 value\r\nnext\r\n");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), edited);
    for path in [a, hidden, new] {
        assert_eq!(fs::read(path).unwrap(), "猫🙂 value\r\nnext\r\n".as_bytes());
    }
}

#[test]
fn distributed_live_models_do_not_enter_recovery_fallback_or_bypass_a_full_target_group() {
    let root = tempfile::tempdir().unwrap();
    let extra = file(root.path(), "extra.cpp");
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.documents = (0..128).map(|_| Document::default()).collect();
    app.active = 127;
    app.sync_pane();
    command(&mut app, "workbench.action.splitEditor");
    app.open(&extra).unwrap();
    let extra_id = app.doc().id;
    assert_eq!(app.documents.len(), 129);
    assert!(!app.group_fallback);
    assert_eq!(group_documents(&app, 0).len(), 128);
    assert_eq!(group_documents(&app, 1).len(), 2);
    app.doc_mut().insert("EXTRA dirty ", false);
    let text = app.doc().text.to_string();
    app.focus_pane(0);
    let before = app.editor_groups.proof();
    assert!(app.open(&extra).is_err());
    assert!(app.editor_groups.proof_current(&before));
    assert!(!app.group_fallback);
    assert_eq!(app.documents.len(), 129);
    app.focus_pane(1);
    assert_eq!(app.doc().id, extra_id);
    assert_eq!(app.doc().text.to_string(), text);
    app.doc_mut().undo();
    assert_eq!(app.doc().text, "猫🙂 value\r\nnext\r\n");
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), text);
    assert_eq!(
        fs::read(extra).unwrap(),
        "猫🙂 value\r\nnext\r\n".as_bytes()
    );
}
