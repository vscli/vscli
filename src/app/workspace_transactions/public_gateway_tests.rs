//! Real selected Node extension -> public applyEdit -> native worker -> mirrors
//! -> awaited Boolean. No mocked edit/save acknowledgement or direct wire write.
use super::*;
use crate::settings::Settings;
use crate::{document::Selection, extensions::Package};
use std::{
    fs,
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    thread,
};

const WAIT: Duration = Duration::from_secs(10);
const A: &str = "α😀 A\r\nlast\r\n";
const B: &str = "猫 B\r\n";
const U: &str = "hidden🙂\r\n";
const OWNER: &str = "test.workspace-edit-gateway";
const APPLY: &str = "gateway.apply";
const SAVE: &str = "gateway.save";
const EXTENSION: &str = r#"
'use strict';
const v = require('vscode'), assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path');
exports.activate = context => {
  const root = v.workspace.rootPath;
  function evidence(tag, value) {
    assert.match(tag, /^[a-z0-9-]{1,32}$/);
    const destination = path.join(root, tag + '.json');
    const temporary = path.join(root, tag + '.json.pending');
    fs.writeFileSync(temporary, JSON.stringify(value));
    fs.renameSync(temporary, destination);
  }
  context.subscriptions.push(v.commands.registerCommand('gateway.apply', async request => {
    const documents = request.uris.map(value => {
      const uri = v.Uri.parse(value).toString();
      const matches = v.workspace.textDocuments.filter(d => d.uri.toString() === uri);
      assert.equal(matches.length, 1, 'one original mirrored model per URI');
      return matches[0];
    });
    const originals = documents.map(d => ({ document:d, uri:d.uri.toString(),
      version:d.version, text:d.getText() }));
    assert.deepEqual(originals.map(d => d.text), request.before);
    const seen = [], saves=[];
    const saveListener=v.workspace.onDidSaveTextDocument(document=>{if(documents.includes(document)){assert(saves.length<32);assert.equal(document.isDirty,false);saves.push(document.uri.toString());}});
    const listener = v.workspace.onDidChangeTextDocument(event => {
      if (!documents.includes(event.document)) return;
      assert(seen.length < 32, 'bounded actual change trace');
      seen.push({ uri:event.document.uri.toString(), version:event.document.version,
        texts:documents.map(d => d.getText()), changes:event.contentChanges.map(c => ({ text:c.text, offset:c.rangeOffset, length:c.rangeLength })) });
    });
    try {
      const edit = new v.WorkspaceEdit();
      documents.forEach((d, i) => edit.insert(d.uri, new v.Position(0, 0), request.prefixes[i]));
      const applied = await v.workspace.applyEdit(edit, { isRefactoring:request.refactoring });
      assert.equal(typeof applied, 'boolean', 'real public API result');
      assert.equal(applied, request.applied, 'exact expected native outcome');
      assert.deepEqual(documents.map(d => d.getText()), request.after, 'all mirrors precede awaited result');
      documents.forEach((d, i) => {
        assert.strictEqual(v.workspace.textDocuments.find(x => x.uri.toString() === originals[i].uri), d,
          'the original TextDocument identity survives the transaction');
        assert.equal(d.isClosed, false);
        if (applied) assert(d.version > originals[i].version, 'own committed edits advance versions');
      });
      if (applied) {
        assert.equal(seen.length, documents.length, 'one changed-document event per edited model before await');
        assert.equal(new Set(seen.map(e => e.uri)).size, documents.length);
        seen.forEach(event => {
          assert.deepEqual(event.texts, request.after, 'all mirrors updated before any change callback');
          const index = originals.findIndex(d => d.uri === event.uri);
          assert(event.version > originals[index].version);
          assert.deepEqual(event.changes, [{ text:request.after[index], offset:0, length:originals[index].text.length }],
            'current native full-document mirror event is exact; incremental change-shape parity is separate');
        });
      }
      assert.equal(saves.length,request.expectSaves?documents.length:0,'actual save events precede awaited apply success');
      if(request.expectSaves)documents.forEach(document=>assert.equal(document.isDirty,false));
      request.disks.forEach(file => assert.equal(fs.readFileSync(path.join(root, file.name), 'utf8'), file.text,
        'applyEdit changes no backing file'));
      evidence(request.tag, { applied, texts:documents.map(d => d.getText()),
        versions:documents.map(d => d.version), seen, saves, dirty:documents.map(d => d.isDirty) });
    } finally { listener.dispose(); saveListener.dispose(); }
  }));
  context.subscriptions.push(v.commands.registerCommand('gateway.save', async request => {
    const document = v.workspace.textDocuments.find(d => d.uri.toString() === request.uri);
    assert(document, 'the original file model stays mirrored for public save');
    const saved = await document.save();
    assert.equal(saved, true);
    assert.equal(document.getText(), request.text);
    assert.equal(document.isDirty, false, 'baseline publication precedes await save true');
    assert.equal(fs.readFileSync(path.join(root, request.name), 'utf8'), request.text);
    evidence(request.tag, { saved, text:document.getText(), dirty:document.isDirty });
  }));
};
"#;

struct Fixture {
    app: App,
    _directory: tempfile::TempDir,
    root: PathBuf,
    a: u64,
    b: u64,
    u: u64,
    original_a: String,
    refactoring_auto_save: Option<bool>,
}
impl Fixture {
    fn new(refactoring_auto_save: Option<bool>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        fs::write(root.join("a.txt"), A).unwrap();
        fs::write(root.join("b.txt"), B).unwrap();
        let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
        let mut settings = json!({"vscli.languageServer.enabled":false,
            "breadcrumbs.enabled":false,"files.autoSave":"off",
            "editor.formatOnSave":false,"editor.codeActionsOnSave":{}});
        if let Some(policy) = refactoring_auto_save {
            settings
                .as_object_mut()
                .unwrap()
                .insert("files.refactoring.autoSave".into(), Value::Bool(policy));
        }
        app.settings = Settings::from_values(
            settings.as_object().unwrap().clone(),
            "public workspace edit fixture",
        )
        .unwrap();
        app.open(&root.join("a.txt")).unwrap();
        until(&mut app, "native A open", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&root.join("a.txt")))
        });
        let a = app.doc().id;
        app.doc_mut().insert("dirty λ ", false);
        let original_a = app.doc().text.to_string();
        app.execute("workbench.action.splitEditor", Value::Null);
        assert_eq!(
            app.panes.len(),
            2,
            "two native views share the original A model"
        );
        let first = app.panes[0].id;
        let second = app.panes[1].id;
        app.doc_mut().activate_view(first);
        app.doc_mut().set_selections(vec![Selection {
            cursor: 2,
            anchor: Some(4),
            desired_column: None,
        }]);
        app.doc_mut().activate_view(second);
        app.doc_mut().set_selections(vec![Selection {
            cursor: 4,
            anchor: Some(2),
            desired_column: None,
        }]);
        let hidden_file = Document::open_existing(&root.join("b.txt")).unwrap();
        let b = hidden_file.id;
        let mut hidden_untitled = Document::from_text(U);
        hidden_untitled.eol = "\r\n".into();
        let u = hidden_untitled.id;
        app.hidden_documents.extend([hidden_file, hidden_untitled]);
        let extension = root.join("extension");
        fs::create_dir(&extension).unwrap();
        fs::write(extension.join("package.json"), serde_json::to_vec(&json!({
            "publisher":"test","name":"workspace-edit-gateway","version":"1.0.0","main":"index.cjs",
            "contributes":{"commands":[{"command":APPLY,"title":"Public applyEdit fixture"},
                {"command":SAVE,"title":"Public save after applyEdit fixture"}]}
        })).unwrap()).unwrap();
        fs::write(extension.join("index.cjs"), EXTENSION).unwrap();
        app.configure_extension_storage(Some(&root.join("storage")));
        app.start_extension_packages(vec![Package::read(&extension).unwrap()])
            .unwrap();
        until(&mut app, "actual selected host and commands", |app| {
            app.extension_host.as_ref().is_some_and(|host| {
                host.ready
                    && host.owner_active(OWNER)
                    && [APPLY, SAVE]
                        .iter()
                        .all(|id| host.commands.iter().any(|(_, command)| command == id))
            })
        });
        Self {
            app,
            _directory: directory,
            root,
            a,
            b,
            u,
            original_a,
            refactoring_auto_save,
        }
    }
    fn doc(&self, id: u64) -> &Document {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .find(|doc| doc.id == id)
            .unwrap()
    }
    fn doc_mut(&mut self, id: u64) -> &mut Document {
        self.app
            .documents
            .iter_mut()
            .chain(&mut self.app.hidden_documents)
            .find(|doc| doc.id == id)
            .unwrap()
    }
    fn uri(&self, id: u64) -> String {
        self.doc(id)
            .path
            .as_ref()
            .map(|path| crate::lsp::file_uri(path).unwrap())
            .unwrap_or_else(|| format!("untitled:vscli-{id}"))
    }
    fn issue(
        &mut self,
        tag: &str,
        ids: &[u64],
        prefixes: &[&str],
        refactoring: bool,
        applied: bool,
    ) {
        let before: Vec<_> = ids
            .iter()
            .map(|id| self.doc(*id).text.to_string())
            .collect();
        let after: Vec<_> = before
            .iter()
            .zip(prefixes)
            .map(|(text, prefix)| {
                if applied {
                    format!("{prefix}{text}")
                } else {
                    text.clone()
                }
            })
            .collect();
        let uris: Vec<_> = ids.iter().map(|id| self.uri(*id)).collect();
        let expect_saves =
            applied && refactoring && ids.len() > 1 && self.refactoring_auto_save.unwrap_or(true);
        let disk_a = ids
            .iter()
            .position(|id| *id == self.a)
            .filter(|_| expect_saves)
            .map_or_else(|| A.to_owned(), |index| after[index].clone());
        let disk_b = ids
            .iter()
            .position(|id| *id == self.b)
            .filter(|_| expect_saves)
            .map_or_else(|| B.to_owned(), |index| after[index].clone());
        self.app.execute(
            APPLY,
            json!({"tag":tag,"uris":uris,"prefixes":prefixes,
            "before":before,"after":after,"refactoring":refactoring,"applied":applied,"expectSaves":expect_saves,
            "disks":[{"name":"a.txt","text":disk_a},{"name":"b.txt","text":disk_b}]}),
        );
    }
    fn await_record(&mut self, tag: &str) -> Value {
        let destination = self.root.join(format!("{tag}.json"));
        until(
            &mut self.app,
            "atomic extension evidence and joined native receipt",
            |app| {
                destination.is_file()
                    && !app.workspace_transactions_pending()
                    && app
                        .extension_host
                        .as_ref()
                        .is_some_and(|host| host.queued_services().next().is_none())
            },
        );
        serde_json::from_slice(&fs::read(destination).unwrap()).unwrap()
    }
    fn gate(&mut self, point: GatePoint) -> ActualGate {
        let (worker, entered, release) = fixture_worker(vec![point]);
        self.app.workspace_transactions.worker = worker;
        ActualGate {
            point,
            entered,
            release: Some(release),
        }
    }
    fn disks_original(&self) {
        assert_eq!(fs::read(self.root.join("a.txt")).unwrap(), A.as_bytes());
        assert_eq!(fs::read(self.root.join("b.txt")).unwrap(), B.as_bytes());
    }
}
struct ActualGate {
    point: GatePoint,
    entered: Receiver<GatePoint>,
    release: Option<SyncSender<()>>,
}
impl ActualGate {
    fn wait(&self, app: &mut App) {
        let deadline = Instant::now() + WAIT;
        loop {
            app.poll();
            match self.entered.try_recv() {
                Ok(point) => {
                    assert_eq!(point, self.point);
                    return;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    panic!("Actual workspace gate disconnected before admission")
                }
            }
            assert!(
                Instant::now() < deadline,
                "Actual workspace gate {:?}: {}",
                self.point,
                app.message
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn resume(&mut self) {
        self.release.take().unwrap().try_send(()).unwrap();
    }
}
impl Drop for ActualGate {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.try_send(());
        }
    }
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn public_apply_edit_publishes_all_shared_hidden_and_untitled_mirrors_before_true_then_groups_history()
 {
    let mut fixture = Fixture::new(None);
    let (a, b, u) = (fixture.a, fixture.b, fixture.u);
    let document_ids: Vec<_> = fixture
        .app
        .documents
        .iter()
        .chain(&fixture.app.hidden_documents)
        .map(|doc| doc.id)
        .collect();
    let first = fixture.app.panes[0].id;
    let second = fixture.app.panes[1].id;
    let active = fixture.app.doc().id;
    let view = fixture.app.active_pane;
    fixture.issue("multi", &[a, b, u], &["λ A ", "λ B ", "λ U "], false, true);
    let record = fixture.await_record("multi");
    assert_eq!(record["applied"], true);
    assert_eq!(record["seen"].as_array().unwrap().len(), 3);
    assert_eq!(
        fixture
            .app
            .documents
            .iter()
            .chain(&fixture.app.hidden_documents)
            .map(|doc| doc.id)
            .collect::<Vec<_>>(),
        document_ids
    );
    assert_eq!(
        fixture.app.documents.len(),
        1,
        "hidden targets never become visible tabs"
    );
    assert_eq!(fixture.app.hidden_documents.len(), 2);
    assert_eq!(fixture.app.doc().id, active);
    assert_eq!(fixture.app.active_pane, view);
    assert_eq!(fixture.doc(a).view_state(Some(first)).cursor, 6);
    assert_eq!(fixture.doc(a).view_state(Some(first)).anchor, Some(8));
    assert_eq!(fixture.doc(a).view_state(Some(second)).cursor, 8);
    assert_eq!(fixture.doc(a).view_state(Some(second)).anchor, Some(6));
    let after_a = format!("λ A {}", fixture.original_a);
    assert_eq!(fixture.doc(a).text.to_string(), after_a);
    assert_eq!(fixture.doc(b).text.to_string(), format!("λ B {B}"));
    assert_eq!(fixture.doc(u).text.to_string(), format!("λ U {U}"));
    fixture.disks_original();
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.doc(a).text.to_string(), fixture.original_a);
    assert_eq!(fixture.doc(b).text.to_string(), B);
    assert_eq!(fixture.doc(u).text.to_string(), U);
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.doc(a).text.to_string(), after_a);
    assert_eq!(fixture.doc(b).text.to_string(), format!("λ B {B}"));
    assert_eq!(fixture.doc(u).text.to_string(), format!("λ U {U}"));
    fixture.disks_original();
    let generation = fixture.doc(a).save_generation();
    let a_uri = fixture.uri(a);
    fixture.app.execute(
        SAVE,
        json!({"tag":"saved","uri":a_uri,"name":"a.txt","text":after_a}),
    );
    assert_eq!(fixture.await_record("saved")["saved"], true);
    assert_eq!(fixture.doc(a).save_generation(), generation + 1);
    assert!(!fixture.doc(a).dirty());
    assert_eq!(
        fs::read(fixture.root.join("a.txt")).unwrap(),
        after_a.as_bytes()
    );
    assert_eq!(fs::read(fixture.root.join("b.txt")).unwrap(), B.as_bytes());
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.doc(a).text.to_string(), fixture.original_a);
    assert_eq!(fixture.doc(b).text.to_string(), B);
    assert_eq!(fixture.doc(u).text.to_string(), U);
    assert!(
        fixture.doc(a).dirty(),
        "grouped Undo preserves independently committed Save baseline"
    );
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.doc(a).text.to_string(), after_a);
    assert!(!fixture.doc(a).dirty());
    assert_eq!(
        fs::read(fixture.root.join("a.txt")).unwrap(),
        after_a.as_bytes()
    );
}

#[test]
fn public_one_resource_refactoring_uses_native_grouped_edit_without_autosave_under_every_policy() {
    for policy in [None, Some(true), Some(false)] {
        let mut fixture = Fixture::new(policy);
        let a = fixture.a;
        let epoch = fixture.doc(a).text_epoch();
        let generation = fixture.doc(a).save_generation();
        fixture.issue("refactoring", &[a], &["R "], true, true);
        let record = fixture.await_record("refactoring");
        assert_eq!(record["applied"], true);
        assert_eq!(fixture.app.workspace_transactions.worker.next, 1);
        assert_eq!(fixture.doc(a).save_generation(), generation);
        assert!(fixture.doc(a).dirty());
        assert_eq!(
            fixture.doc(a).text.to_string(),
            format!("R {}", fixture.original_a)
        );
        fixture.app.execute("undo", Value::Null);
        assert_eq!(fixture.doc(a).text.to_string(), fixture.original_a);
        assert!(fixture.doc(a).text_epoch() > epoch);
        fixture.app.execute("redo", Value::Null);
        assert_eq!(
            fixture.doc(a).text.to_string(),
            format!("R {}", fixture.original_a)
        );
        assert_eq!(fixture.doc(a).save_generation(), generation);
        assert_eq!(fixture.doc(fixture.b).text.to_string(), B);
        assert_eq!(fixture.doc(fixture.u).text.to_string(), U);
        fixture.disks_original();
    }
}

#[test]
fn public_multi_resource_refactoring_default_waits_for_actual_saves_and_preserves_grouped_history()
{
    for policy in [None, Some(true)] {
        let mut fixture = Fixture::new(policy);
        let (a, b) = (fixture.a, fixture.b);
        fixture.issue("refactoring", &[a, b], &["R ", "R "], true, true);
        let record = fixture.await_record("refactoring");
        assert_eq!(record["applied"], true);
        assert_eq!(record["saves"].as_array().unwrap().len(), 2);
        assert_eq!(fixture.app.workspace_transactions.worker.next, 1);
        assert!(!fixture.app.native_save_worker_busy());
        let after_a = format!("R {}", fixture.original_a);
        let after_b = format!("R {B}");
        assert_eq!(
            fs::read(fixture.root.join("a.txt")).unwrap(),
            after_a.as_bytes()
        );
        assert_eq!(
            fs::read(fixture.root.join("b.txt")).unwrap(),
            after_b.as_bytes()
        );
        assert!(!fixture.doc(a).dirty());
        assert!(!fixture.doc(b).dirty());
        assert_eq!(fixture.doc(fixture.u).text, U);
        fixture.app.execute("undo", Value::Null);
        assert_eq!(fixture.doc(a).text, fixture.original_a);
        assert_eq!(fixture.doc(b).text, B);
        assert!(fixture.doc(a).dirty());
        assert!(fixture.doc(b).dirty());
        assert_eq!(
            fs::read(fixture.root.join("b.txt")).unwrap(),
            after_b.as_bytes()
        );
        fixture.app.execute("redo", Value::Null);
        assert!(!fixture.doc(a).dirty());
        assert!(!fixture.doc(b).dirty());
    }
}

#[test]
fn real_second_public_request_keeps_prequeue_source_proof_after_edit_undo_while_first_is_held() {
    let mut fixture = Fixture::new(None);
    let (a, b) = (fixture.a, fixture.b);
    let mut gate = fixture.gate(GatePoint::BeforePrepare);
    fixture.issue("first", &[a], &["one "], false, true);
    gate.wait(&mut fixture.app);
    fixture.issue("second", &[b], &["two "], false, false);
    until(
        &mut fixture.app,
        "second real request captured behind first",
        |app| {
            app.extension_host
                .as_ref()
                .is_some_and(|host| host.queued_services().count() == 2)
        },
    );
    let a_epoch = fixture.doc(a).text_epoch();
    fixture.doc_mut(b).insert("independent ", false);
    fixture.doc_mut(b).undo();
    assert_eq!(fixture.doc(b).text.to_string(), B);
    gate.resume();
    assert_eq!(fixture.await_record("first")["applied"], true);
    assert_eq!(fixture.await_record("second")["applied"], false);
    assert_eq!(
        fixture.app.workspace_transactions.worker.next, 1,
        "stale second capture never starts a fresh worker"
    );
    assert_eq!(
        fixture.doc(a).text.to_string(),
        format!("one {}", fixture.original_a)
    );
    assert!(fixture.doc(a).text_epoch() > a_epoch);
    assert_eq!(fixture.doc(b).text.to_string(), B);
    fixture.doc_mut(b).redo();
    assert_eq!(fixture.doc(b).text.to_string(), format!("independent {B}"));
    fixture.disks_original();
}

#[test]
fn actual_selected_host_drop_after_prepared_reply_retires_caller_and_retains_slot_until_join() {
    let mut fixture = Fixture::new(None);
    let (a, b, u) = (fixture.a, fixture.b, fixture.u);
    let original_a = fixture.doc(a).text.clone();
    let epoch = fixture.doc(a).text_epoch();
    let mut gate = fixture.gate(GatePoint::AfterReply);
    fixture.issue(
        "retired",
        &[a, b, u],
        &["old A ", "old B ", "old U "],
        false,
        true,
    );
    gate.wait(&mut fixture.app);
    let token = fixture
        .app
        .workspace_transactions
        .pending
        .as_ref()
        .unwrap()
        .token;
    assert!(fixture.app.workspace_transactions_pending());
    drop(fixture.app.extension_host.take().unwrap());
    fixture.app.poll();
    assert!(
        fixture.app.workspace_transactions_pending(),
        "retired actual host does not release held worker capacity"
    );
    assert!(fixture.app.workspace_transaction_completion().is_none());
    assert_eq!(fixture.doc(a).text, original_a);
    gate.resume();
    until(
        &mut fixture.app,
        "retired actual worker joined and receipt cleared",
        |app| !app.workspace_transactions_pending(),
    );
    assert!(fixture.app.extension_host.is_none());
    assert!(fixture.app.workspace_transactions.worker.active.is_none());
    assert!(fixture.app.workspace_transactions.worker.next >= token);
    assert!(
        !fixture.root.join("retired.json").exists(),
        "dead session receives no fabricated successful callback"
    );
    assert_eq!(fixture.doc(a).text, original_a);
    assert_eq!(fixture.doc(a).text_epoch(), epoch);
    assert_eq!(fixture.doc(b).text.to_string(), B);
    assert_eq!(fixture.doc(u).text.to_string(), U);
    fixture.app.execute("undo", Value::Null);
    assert_eq!(
        fixture.doc(a).text.to_string(),
        A,
        "original dirty edit still owns its native Undo"
    );
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.doc(a).text, original_a);
    fixture.disks_original();
}
