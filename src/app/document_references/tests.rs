use super::*;
use crate::document_references::MAX_AGE;
use std::time::Duration;

fn file(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, "猫🙂\r\noriginal").unwrap();
    path
}
fn opened(root: &Path) -> (App, PathBuf, u64) {
    let path = file(root, "main.cpp");
    let mut app = App::new(root.into(), Profile::Linux);
    app.open(&path).unwrap();
    let id = app.doc().id;
    (app, path, id)
}

#[test]
fn close_last_tab_keeps_original_model_until_exact_reference_expiry() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, path, id) = opened(root.path());
    let now = Instant::now();
    app.acquire_api_document(1, id, now).unwrap();
    let epoch = app.doc().resource_epoch();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert!(app.documents.is_empty());
    assert_eq!(app.hidden_documents.len(), 1);
    assert_eq!(
        (
            app.hidden_documents[0].id,
            app.hidden_documents[0].resource_epoch()
        ),
        (id, epoch)
    );
    assert!(!app.poll_api_document_references(Some(1), now + MAX_AGE - Duration::from_nanos(1)));
    assert!(app.poll_api_document_references(Some(1), now + MAX_AGE));
    assert!(app.hidden_documents.is_empty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "猫🙂\r\noriginal");
}

#[test]
fn expiry_releases_api_ownership_without_destroying_a_visible_editor() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _, id) = opened(root.path());
    let now = Instant::now();
    app.acquire_api_document(1, id, now).unwrap();
    assert!(!app.poll_api_document_references(Some(1), now + MAX_AGE));
    assert_eq!(app.doc().id, id);
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert!(app.documents.is_empty() && app.hidden_documents.is_empty());
}

#[test]
fn independent_acquisition_deadlines_and_reopen_do_not_replace_the_model() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, path, id) = opened(root.path());
    let now = Instant::now();
    app.acquire_api_document(1, id, now).unwrap();
    app.acquire_api_document(1, id, now + Duration::from_secs(30))
        .unwrap();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert!(!app.poll_api_document_references(Some(1), now + MAX_AGE));
    assert_eq!(app.hidden_documents[0].id, id);
    app.open(&path).unwrap();
    assert_eq!(app.doc().id, id);
    assert!(app.hidden_documents.is_empty());
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
}

#[test]
fn dirty_and_saved_history_models_survive_last_reference_release_and_customer_teardown() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, path, id) = opened(root.path());
    let now = Instant::now();
    app.doc_mut().insert("dirty ", false);
    app.acquire_api_document(1, id, now).unwrap();
    let doc = app.documents.remove(0);
    app.hidden_documents.push(doc);
    assert!(!app.poll_api_document_references(None, now + MAX_AGE));
    assert!(app.hidden_documents[0].dirty());
    assert_eq!(app.recovery_documents()[0].id, id);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "猫🙂\r\noriginal");
    app.hidden_documents[0].save().unwrap();
    assert!(!app.hidden_documents[0].dirty());
    assert!(!app.poll_api_document_references(None, now + MAX_AGE));
    app.hidden_documents[0].undo();
    assert_eq!(app.hidden_documents[0].text.to_string(), "猫🙂\r\noriginal");
    assert!(app.hidden_documents[0].dirty());
    app.hidden_documents[0].redo();
    assert_eq!(
        app.hidden_documents[0].text.to_string(),
        "dirty 猫🙂\r\noriginal"
    );
}

#[test]
fn unrelated_native_hidden_models_are_not_owned_by_the_api_ledger() {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    let doc = Document::open(&file(root.path(), "native.cpp")).unwrap();
    let id = doc.id;
    app.hidden_documents.push(doc);
    assert!(!app.poll_api_document_references(None, Instant::now()));
    assert_eq!(app.hidden_documents[0].id, id);
}

#[test]
fn captured_length_uses_utf16_and_canonical_eol_instead_of_raw_disk_bytes() {
    let mut doc = Document::from_text("猫🙂\rA\n🙂\r\nlast");
    doc.eol = "\r\n".into();
    assert_eq!(
        value_length(&doc),
        "猫🙂\r\nA\r\n🙂\r\nlast".encode_utf16().count() as u64
    );
    doc.eol = "\n".into();
    assert_eq!(
        value_length(&doc),
        "猫🙂\nA\n🙂\nlast".encode_utf16().count() as u64
    );
}

#[test]
fn genuine_reacquisition_updates_length_proof_without_rewriting_older_reference_units() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _, id) = opened(root.path());
    let now = Instant::now();
    let original = value_length(app.doc());
    app.acquire_api_document(1, id, now).unwrap();
    assert_eq!(app.api_documents.lengths.len(), 1);
    app.acquire_api_document(1, id, now).unwrap();
    assert_eq!(app.api_documents.lengths.len(), 1);
    app.doc_mut().insert("extra 猫🙂\r\n", false);
    let changed = value_length(app.doc());
    app.acquire_api_document(1, id, now).unwrap();
    assert_eq!(
        app.api_documents
            .ledger
            .as_ref()
            .unwrap()
            .captured_length_units(),
        original * 2 + changed
    );
    let key = model_key(app.doc());
    assert_eq!(app.api_documents.lengths.get(&key).unwrap().units, changed);
}

#[test]
fn routine_input_cancellation_and_stale_teardown_do_not_retire_a_customer() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _, id) = opened(root.path());
    let now = Instant::now();
    app.acquire_api_document(1, id, now).unwrap();
    app.cancel_extension_services();
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
    app.bind_api_document_customer(2).unwrap();
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 0);
    app.acquire_api_document(2, id, now).unwrap();
    app.retire_api_document_customer(1);
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
}

fn discard_finished(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while app.api_documents.discard.is_some() {
        app.poll_api_discard();
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn dirty_discard_reads_current_disk_then_retains_identity_and_undo_history() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, path, id) = opened(root.path());
    app.acquire_api_document(1, id, Instant::now()).unwrap();
    app.doc_mut().insert("unsaved ", false);
    std::fs::write(&path, "external 猫🙂\r\nsecond").unwrap();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert!(matches!(app.modal, Some(Modal::Confirm(AfterSave::Close))));
    app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.api_documents.discard.is_some());
    discard_finished(&mut app);
    assert!(app.documents.is_empty(), "{}", app.message);
    let doc = &mut app.hidden_documents[0];
    assert_eq!(doc.id, id);
    assert_eq!(doc.text.to_string(), "external 猫🙂\r\nsecond");
    assert!(!doc.dirty());
    doc.undo();
    assert_eq!(doc.text.to_string(), "unsaved 猫🙂\r\noriginal");
    assert!(doc.dirty());
    doc.redo();
    assert_eq!(doc.text.to_string(), "external 猫🙂\r\nsecond");
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "external 猫🙂\r\nsecond"
    );
}

#[test]
fn late_discard_cannot_overwrite_newer_text_even_after_edit_undo_aba() {
    for undo in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (mut app, path, id) = opened(root.path());
        app.acquire_api_document(1, id, Instant::now()).unwrap();
        app.doc_mut().insert("unsaved ", false);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        app.doc_mut().insert("newer ", false);
        if undo {
            app.doc_mut().undo();
        }
        let before = app.doc().text.clone();
        discard_finished(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text, before);
        assert!(app.doc().dirty());
        assert!(app.message.contains("refused"), "{}", app.message);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "猫🙂\r\noriginal");
    }
}

#[test]
fn discard_failure_preserves_original_model_membership_bytes_and_history() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, path, id) = opened(root.path());
    app.acquire_api_document(1, id, Instant::now()).unwrap();
    app.doc_mut().insert("unsaved ", false);
    std::fs::remove_file(&path).unwrap();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    discard_finished(&mut app);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "unsaved 猫🙂\r\noriginal");
    assert!(app.doc().dirty());
    assert!(app.active_tab_membership().is_some());
    assert!(!path.exists());
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "猫🙂\r\noriginal");
}

#[test]
fn pending_close_protects_expired_reference_until_real_discard_reader_settles() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _, id) = opened(root.path());
    let now = Instant::now();
    app.acquire_api_document(1, id, now).unwrap();
    app.doc_mut().insert("unsaved ", false);
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    let member = app.active_tab_membership().unwrap();
    let generation = app.tab_close_generation();
    app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.tab_close_pending(member, generation));
    assert!(!app.poll_api_document_references(Some(1), now + MAX_AGE));
    assert_eq!(app.doc().id, id);
    discard_finished(&mut app);
    assert!(app.documents.is_empty());
    assert!(app.hidden_documents.is_empty());
}

#[test]
fn resource_epoch_aba_expiry_cannot_release_the_new_binding_at_the_same_path() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, path, id) = opened(root.path());
    let now = Instant::now();
    app.acquire_api_document(1, id, now).unwrap();
    app.doc_mut()
        .set_resource_path(Some(root.path().join("other.cpp")))
        .unwrap();
    app.doc_mut().set_resource_path(Some(path.clone())).unwrap();
    app.acquire_api_document(1, id, now + Duration::from_secs(30))
        .unwrap();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert!(!app.poll_api_document_references(Some(1), now + MAX_AGE));
    assert_eq!(app.hidden_documents[0].id, id);
    assert_eq!(app.hidden_documents[0].resource_epoch(), 2);
    assert!(app.poll_api_document_references(Some(1), now + MAX_AGE + Duration::from_secs(30)));
    assert!(app.hidden_documents.is_empty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "猫🙂\r\noriginal");
}

#[test]
fn long_native_unicode_paths_keep_reference_receipts_without_reducing_path_admission() {
    let root = tempfile::tempdir().unwrap();
    let mut folder = root.path().to_owned();
    for _ in 0..12 {
        folder = folder.join("猫".repeat(60));
    }
    std::fs::create_dir_all(&folder).unwrap();
    let (mut app, _, id) = opened(&folder);
    assert!(crate::extensions::document_uri(app.doc()).unwrap().len() > 4096);
    app.acquire_api_document(1, id, Instant::now()).unwrap();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    assert_eq!(app.hidden_documents[0].id, id);
}

#[test]
fn discarded_untitled_reverts_only_its_original_references_and_preserves_peer_work() {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    let doc = Document::from_untitled(
        Some("untitled:/api.cpp".into()),
        "猫🙂\r\nunsaved",
        Some("cpp".into()),
        None,
    )
    .unwrap();
    let id = doc.id;
    app.install_preview_document(doc, crate::editor_groups::OpenMode::Committed)
        .unwrap();
    app.acquire_api_document(1, id, Instant::now()).unwrap();
    app.acquire_api_document(1, id, Instant::now()).unwrap();
    let peer = Document::from_untitled(
        Some("untitled:/peer.cpp".into()),
        "peer 猫🙂\r\n",
        Some("cpp".into()),
        None,
    )
    .unwrap();
    let peer_id = peer.id;
    app.hidden_documents.push(peer);
    app.acquire_api_document(1, peer_id, Instant::now())
        .unwrap();
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.documents.is_empty());
    assert_eq!(app.hidden_documents[0].id, peer_id);
    assert!(app.hidden_documents[0].dirty());
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
    assert_eq!(
        app.recovery_documents()[0].text.to_string(),
        "peer 猫🙂\r\n"
    );
}

#[test]
fn replacing_an_api_referenced_preview_preserves_the_hidden_model_and_original_lease() {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    let a = file(root.path(), "a.cpp");
    let b = file(root.path(), "b.cpp");
    let doc = Document::open(&a).unwrap();
    let id = doc.id;
    app.hidden_documents.push(doc);
    app.acquire_api_document(1, id, Instant::now()).unwrap();
    app.open_preview_model(id, crate::editor_groups::OpenMode::Preview)
        .unwrap();
    app.install_preview_document(
        Document::open(&b).unwrap(),
        crate::editor_groups::OpenMode::Preview,
    )
    .unwrap();
    assert_eq!(app.doc().path.as_ref(), Some(&b));
    assert_eq!(app.hidden_documents[0].id, id);
    assert_eq!(app.hidden_documents[0].path.as_ref(), Some(&a));
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(a).unwrap(), "猫🙂\r\noriginal");
}

#[test]
fn late_discard_cannot_revert_after_settings_or_membership_authority_changes() {
    for settings in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (mut app, path, id) = opened(root.path());
        app.acquire_api_document(1, id, Instant::now()).unwrap();
        app.doc_mut().insert("unsaved ", false);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.modal_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        if settings {
            app.configure_settings(None).unwrap();
        } else {
            app.execute("workbench.action.pinEditor", Value::Null);
        }
        discard_finished(&mut app);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "unsaved 猫🙂\r\noriginal");
        assert!(app.doc().dirty());
        assert!(app.active_tab_membership().is_some());
        assert!(app.message.contains("refused"), "{}", app.message);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "猫🙂\r\noriginal");
    }
}

fn actual(root: &Path, virtual_document: bool) -> App {
    let path = file(root, "native.txt");
    let extension = root.join("extension");
    std::fs::create_dir(&extension).unwrap();
    std::fs::write(extension.join("package.json"), r#"{"publisher":"fixture","name":"references","version":"1.0.0","main":"extension.cjs","contributes":{"commands":[{"command":"references.show","title":"Show acquired document"},{"command":"references.cache","title":"Reopen acquired document"},{"command":"references.expired","title":"Check released document"},{"command":"references.refresh","title":"Refresh provider document"}]}}"#).unwrap();
    let source = r#"
const vscode=require('vscode'),assert=require('node:assert/strict');
exports.activate=async context=>{
 const changed=new vscode.EventEmitter();let content=VIRTUAL?'猫🙂\noriginal':'猫🙂\r\noriginal',closed=0;
 if(VIRTUAL) context.subscriptions.push(vscode.workspace.registerTextDocumentContentProvider('preview',{onDidChange:changed.event,provideTextDocumentContent(){return content;}}));
 const resource=VIRTUAL?vscode.Uri.parse('preview:?revision=1'):'native.txt';
 const doc=await vscode.workspace.openTextDocument(resource);
 assert.equal(vscode.window.activeTextEditor,undefined);assert.equal(doc.getText(),content);
 context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(value=>{if(value===doc)closed++;}));
 const register=(id,call)=>context.subscriptions.push(vscode.commands.registerCommand('references.'+id,call));
 register('cache',async()=>{assert.equal(await vscode.workspace.openTextDocument(resource),doc);assert.equal(doc.isClosed,false);assert.equal(closed,0);await vscode.window.showInformationMessage('native reference cached');});
 register('show',async()=>{assert.equal((await vscode.window.showTextDocument(doc,{preview:false})).document,doc);assert.equal(closed,0);await vscode.window.showInformationMessage('native reference shown');});
 register('expired',async()=>{assert.equal(doc.isClosed,true);assert.equal(closed,1);assert(!vscode.workspace.textDocuments.includes(doc));assert.equal(doc.getText(),'猫🙂\r\noriginal');await vscode.window.showInformationMessage('native reference expired');});
 register('refresh',async()=>{content='longer 猫🙂\nprovider';changed.fire(resource);await vscode.window.showInformationMessage('native provider refreshing');});
};
"#.replace("VIRTUAL", if virtual_document { "true" } else { "false" });
    std::fs::write(extension.join("extension.cjs"), source).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
        .unwrap();
    until_actual(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready && host.commands.iter().any(|(_, id)| id == "references.cache")
        })
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "猫🙂\r\noriginal");
    app
}
fn until_actual(app: &mut App, check: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command_actual(app: &mut App, id: &str, receipt: &str) {
    app.execute(id, Value::Null);
    until_actual(app, |app| {
        app.message == receipt && app.extension_host.as_ref().is_some_and(|host| !host.busy())
    });
}

#[test]
fn actual_selected_host_cache_and_last_tab_close_reuse_one_original_acquisition() {
    let root = tempfile::tempdir().unwrap();
    let mut app = actual(root.path(), false);
    let id = app.hidden_documents[0].id;
    let deadline = app.api_documents.ledger.as_ref().unwrap().next_expiry();
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
    command_actual(&mut app, "references.cache", "native reference cached");
    assert_eq!(
        app.api_documents.ledger.as_ref().unwrap().next_expiry(),
        deadline
    );
    command_actual(&mut app, "references.show", "native reference shown");
    app.execute("workbench.action.closeActiveEditor", Value::Null);
    until_actual(&mut app, |app| app.active_document().is_none());
    assert_eq!(app.hidden_documents[0].id, id);
    command_actual(&mut app, "references.cache", "native reference cached");
    command_actual(&mut app, "references.show", "native reference shown");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 1);
    assert_eq!(
        app.api_documents.ledger.as_ref().unwrap().next_expiry(),
        deadline
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("native.txt")).unwrap(),
        "猫🙂\r\noriginal"
    );
}

#[test]
fn actual_selected_host_expiry_publishes_one_close_and_preserves_readable_old_document() {
    let root = tempfile::tempdir().unwrap();
    let mut app = actual(root.path(), false);
    until_actual(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| !host.busy() && host.queued_services().next().is_none())
    });
    let ledger = app.api_documents.ledger.as_ref().unwrap();
    let (session, deadline) = (ledger.customer_epoch(), ledger.next_expiry().unwrap());
    assert!(app.poll_api_document_references(Some(session), deadline));
    assert!(app.hidden_documents.is_empty());
    command_actual(&mut app, "references.expired", "native reference expired");
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 0);
    assert_eq!(
        std::fs::read_to_string(root.path().join("native.txt")).unwrap(),
        "猫🙂\r\noriginal"
    );
}

#[test]
fn actual_scheme_only_provider_open_and_refresh_capture_length_once_without_renewing_reference() {
    let root = tempfile::tempdir().unwrap();
    let mut app = actual(root.path(), true);
    let id = app.hidden_documents[0].id;
    let ledger = app.api_documents.ledger.as_ref().unwrap();
    let (length, deadline) = (ledger.captured_length_units(), ledger.next_expiry());
    assert_eq!(ledger.len(), 1);
    command_actual(&mut app, "references.refresh", "native provider refreshing");
    until_actual(&mut app, |app| {
        app.hidden_documents[0].text == "longer 猫🙂\nprovider"
    });
    assert_eq!(app.hidden_documents[0].id, id);
    let ledger = app.api_documents.ledger.as_ref().unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.captured_length_units(), length);
    assert_eq!(ledger.next_expiry(), deadline);
    app.stop_extension_host();
    until_actual(&mut app, |app| app.hidden_documents.is_empty());
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 0);
}

#[test]
fn actual_cross_extension_concurrent_file_loads_share_one_acquisition_per_uri() {
    let root = tempfile::tempdir().unwrap();
    file(root.path(), "shared.txt");
    file(root.path(), "other.txt");
    let mut packages = Vec::new();
    for name in ["secondary", "primary"] {
        let folder = root.path().join(name);
        std::fs::create_dir(&folder).unwrap();
        let dependencies = if name == "primary" {
            json!(["fixture.secondary"])
        } else {
            json!([])
        };
        let manifest = json!({"publisher":"fixture","name":name,"version":"1.0.0","main":"extension.cjs","extensionDependencies":dependencies});
        std::fs::write(folder.join("package.json"), manifest.to_string()).unwrap();
        let source = if name == "secondary" {
            r#"
const vscode=require('vscode');exports.activate=()=>({open:uri=>vscode.workspace.openTextDocument(uri)});
"#
        } else {
            r#"
const vscode=require('vscode'),assert=require('node:assert/strict');
exports.activate=async()=>{
 const secondary=vscode.extensions.getExtension('fixture.secondary').exports;
 const opened=await Promise.all([vscode.workspace.openTextDocument('shared.txt'),secondary.open(vscode.Uri.file(require('node:path').join(vscode.workspace.rootPath,'shared.txt'))),vscode.workspace.openTextDocument('other.txt')]);
 assert.equal(opened[0],opened[1]);assert.notEqual(opened[0],opened[2]);assert.equal(vscode.workspace.textDocuments.length,2);assert.equal(vscode.window.activeTextEditor,undefined);
 assert.equal(opened[0].getText(),'猫🙂\r\noriginal');assert.equal(opened[2].getText(),'猫🙂\r\noriginal');
 await vscode.window.showInformationMessage('shared native loads qualified');
};
"#
        };
        std::fs::write(folder.join("extension.cjs"), source).unwrap();
        packages.push(crate::extensions::Package::read(&folder).unwrap());
    }
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(packages).unwrap();
    until_actual(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.ready && !host.busy())
    });
    assert!(app.documents.is_empty());
    assert_eq!(app.hidden_documents.len(), 2);
    assert_eq!(app.api_documents.ledger.as_ref().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(root.path().join("shared.txt")).unwrap(),
        "猫🙂\r\noriginal"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("other.txt")).unwrap(),
        "猫🙂\r\noriginal"
    );
}
