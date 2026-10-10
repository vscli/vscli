use super::*;
use crate::{keys::Profile, settings::Settings};
use serde_json::json;
use std::fs;

const ORIGINAL: &str = "猫🙂 A\r\nlast\r\n";
const PEER: &str = r#"
import json,sys
from pathlib import Path
root=Path(sys.argv[1]); documents={}
def send(value):
 body=json.dumps({'jsonrpc':'2.0',**value},ensure_ascii=False).encode()
 sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode()+body);sys.stdout.buffer.flush()
while True:
 headers={}
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  key,value=line.decode().split(':',1);headers[key.lower()]=value.strip()
 m=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
 method=m.get('method');p=m.get('params',{});ident=m.get('id')
 if method=='initialize':send({'id':ident,'result':{'capabilities':{'textDocumentSync':{'openClose':True,'change':1,'save':{'includeText':True}},'codeActionProvider':{'resolveProvider':True},'documentFormattingProvider':True}}})
 elif method in ('textDocument/didOpen','textDocument/didChange'):
  uri=p['textDocument']['uri'];documents[uri]=p.get('text',p.get('contentChanges',[{}])[0].get('text',''))
 elif method in ('textDocument/codeAction','codeAction/resolve','textDocument/formatting'):
  text=documents.get(p.get('textDocument',{}).get('uri'),'')
  with (root/'requests.jsonl').open('a',encoding='utf-8') as f:f.write(json.dumps({'id':ident,'method':method,'params':p,'text':text[:4096],'length':len(text)})+'\n')
 elif method=='fixture/release':send({'id':p['id'],'result':p['result']})
 elif method=='$/cancelRequest':
  with (root/'cancels.jsonl').open('a',encoding='utf-8') as f:f.write(json.dumps(p)+'\n')
 elif method=='textDocument/didSave':
  with (root/'saved.jsonl').open('a',encoding='utf-8') as f:f.write(json.dumps(p,ensure_ascii=False)+'\n')
 elif method=='shutdown':send({'id':ident,'result':None})
 elif method=='exit':break
"#;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    app: App,
    path: PathBuf,
    other: PathBuf,
    id: u64,
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
impl Fixture {
    fn new(format: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("a.cpp");
        let other = root.join("b.cpp");
        fs::write(&path, ORIGINAL).unwrap();
        fs::write(&other, "B unchanged\r\n").unwrap();
        let peer = root.join("peer.py");
        fs::write(&peer, PEER).unwrap();
        let mut app = App::new(root.clone(), Profile::Linux);
        app.settings=Settings::from_values(json!({"editor.codeActionsOnSave":{"source.fixAll":"explicit","source.organizeImports":"explicit"},"editor.formatOnSave":format,"files.autoSave":"off","breadcrumbs.enabled":false}).as_object().unwrap().clone(),"source actions fixture").unwrap();
        assert!(
            app.settings.warnings.is_empty(),
            "{:?}",
            app.settings.warnings
        );
        app.open(&other).unwrap();
        until(&mut app, "open B", |a| {
            a.doc().path.as_ref() == Some(&other)
        });
        app.open(&path).unwrap();
        until(&mut app, "open A", |a| a.doc().path.as_ref() == Some(&path));
        let id = app.doc().id;
        app.lsp = Some(
            crate::lsp::Client::start(
                "python3",
                &[
                    "-u".into(),
                    peer.to_string_lossy().into_owned(),
                    root.to_string_lossy().into_owned(),
                ],
                &root,
                "cpp".into(),
            )
            .unwrap(),
        );
        until(&mut app, "native initialize", |a| {
            a.lsp.as_ref().is_some_and(|c| c.ready)
        });
        Self {
            _dir: dir,
            root,
            app,
            path,
            other,
            id,
        }
    }
    fn dirty(&mut self) {
        self.app.doc_mut().insert("RAW ", false);
    }
    fn rows(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("requests.jsonl"))
            .ok()
            .map(|s| {
                s.lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn token(&mut self, ordinal: usize) -> u64 {
        let root = self.root.clone();
        until(&mut self.app, "actual peer request", |_| {
            fs::read_to_string(root.join("requests.jsonl"))
                .ok()
                .is_some_and(|s| {
                    s.lines()
                        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                        .count()
                        >= ordinal
                })
        });
        self.rows()[ordinal - 1]["id"].as_u64().unwrap()
    }
    fn release(&mut self, token: u64, result: Value) {
        self.app
            .lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":token,"result":result}))
            .unwrap();
    }
    fn save(&mut self) -> u64 {
        self.app.execute("workbench.action.files.save", Value::Null);
        self.token(1)
    }
    fn saved(&mut self) {
        until(&mut self.app, "native receipt", |a| !a.saves_pending());
    }
    fn model(&self) -> &Document {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .find(|d| d.id == self.id)
            .unwrap()
    }
    fn action(&self, kind: &str, edits: Value) -> Value {
        json!({"title":"fixture edit","kind":kind,"edit":{"changes":{crate::lsp::file_uri(&self.path).unwrap():edits}}})
    }
}
fn edit(start: usize, end: usize, text: &str) -> Value {
    json!({"range":{"start":{"line":0,"character":start},"end":{"line":0,"character":end}},"newText":text})
}

#[test]
fn queued_opted_in_save_escape_before_first_callback_preserves_text_redo_and_disk() {
    let mut f = Fixture::new(false);
    f.dirty();
    f.app.doc_mut().insert("new", false);
    f.app.doc_mut().undo();
    let before = f.model().text.to_string();
    let epoch = f.model().text_epoch();
    f.app.execute("workbench.action.files.save", Value::Null);
    assert!(f.app.saving.latest.is_some());
    assert!(f.app.saving.actions.active.is_none());
    assert!(f.rows().is_empty());
    f.app.event(crossterm::event::Event::Key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ),
    ));
    assert!(!f.app.saves_pending());
    assert!(f.app.message.contains("code actions canceled"));
    f.app.poll();
    assert!(f.rows().is_empty());
    assert_eq!(f.model().id, f.id);
    assert_eq!(f.model().text_epoch(), epoch);
    assert_eq!(f.model().text.to_string(), before);
    assert_eq!(f.model().cursor, 4);
    assert!(f.model().dirty());
    assert_eq!(fs::read(&f.path).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(fs::read(&f.other).unwrap(), b"B unchanged\r\n");
    f.app.doc_mut().redo();
    assert_eq!(f.model().text.to_string(), "RAW new".to_owned() + ORIGINAL);
    assert_eq!(f.model().cursor, 7);
}

#[test]
fn queued_format_only_save_is_not_retired_by_source_action_escape() {
    let mut f = Fixture::new(true);
    f.app.settings = Settings::from_values(
        json!({"editor.formatOnSave":true,"files.autoSave":"off","breadcrumbs.enabled":false})
            .as_object()
            .unwrap()
            .clone(),
        "format-only fixture",
    )
    .unwrap();
    f.dirty();
    f.app.execute("workbench.action.files.save", Value::Null);
    let intent = f.app.saving.latest.as_ref().unwrap().id;
    assert!(f.app.saving.actions.active.is_none());
    assert!(
        !f.app
            .retire_save_code_actions("unrelated source action invalidation")
    );
    assert!(!f.app.escape_save_code_actions());
    assert_eq!(f.app.saving.latest.as_ref().map(|i| i.id), Some(intent));
    let formatting = f.token(1);
    assert_eq!(f.rows()[0]["method"], "textDocument/formatting");
    f.release(formatting, Value::Null);
    f.saved();
    assert_eq!(
        fs::read(&f.path).unwrap(),
        ("RAW ".to_owned() + ORIGINAL).as_bytes()
    );
    assert_eq!(f.model().id, f.id);
    assert!(!f.model().dirty());
    assert_eq!(fs::read(&f.other).unwrap(), b"B unchanged\r\n");
}

#[test]
fn owned_stages_discover_fresh_imports_then_format_and_undo_independently() {
    let mut f = Fixture::new(true);
    f.dirty();
    let first = f.save();
    let first_edit = f.action("source.fixAll", json!([edit(0, 3, "FIX")]));
    let stale_second = f.action("source.fixAll", json!([edit(0, 3, "BAD")]));
    f.release(first, json!([first_edit, stale_second]));
    let second = f.token(2);
    assert_eq!(f.rows()[1]["text"], "FIX 猫🙂 A\r\nlast\r\n");
    assert_eq!(
        f.rows()[1]["params"]["context"]["only"],
        json!(["source.organizeImports"])
    );
    f.release(
        second,
        json!([f.action("source.organizeImports", json!([edit(0, 0, "use z;\n")]))]),
    );
    let formatting = f.token(3);
    assert_eq!(f.rows()[2]["method"], "textDocument/formatting");
    assert_eq!(f.rows()[2]["text"], "use z;\r\nFIX 猫🙂 A\r\nlast\r\n");
    f.release(formatting, json!([edit(0, 6, "use zz;")]));
    f.saved();
    let persisted = "use zz;\r\nFIX 猫🙂 A\r\nlast\r\n";
    assert_eq!(fs::read(&f.path).unwrap(), persisted.as_bytes());
    assert!(!f.model().dirty());
    f.app.doc_mut().undo();
    assert_eq!(
        f.model().text.to_string(),
        "use z;\r\nFIX 猫🙂 A\r\nlast\r\n"
    );
    f.app.doc_mut().undo();
    assert_eq!(f.model().text.to_string(), "FIX 猫🙂 A\r\nlast\r\n");
    f.app.doc_mut().undo();
    assert_eq!(f.model().text.to_string(), "RAW 猫🙂 A\r\nlast\r\n");
    for _ in 0..3 {
        f.app.doc_mut().redo();
    }
    assert_eq!(f.model().text.to_string(), persisted);
    assert!(!f.model().dirty());
    assert_eq!(fs::read(&f.other).unwrap(), b"B unchanged\r\n");
}
#[test]
fn lazy_without_data_and_whole_combined_command_skip_preserve_origin_across_pane_switch() {
    let mut f = Fixture::new(false);
    f.dirty();
    let first = f.save();
    let mut combined = f.action("source.fixAll", json!([edit(0, 3, "BAD")]));
    combined["command"] = json!({"title":"command","command":"fixture.sideeffect"});
    f.release(
        first,
        json!([combined,{"title":"lazy","kind":"source.fixAll"}]),
    );
    let resolve = f.token(2);
    assert_eq!(f.rows()[1]["method"], "codeAction/resolve");
    assert!(f.rows()[1]["params"].get("data").is_none());
    f.app.open(&f.other).unwrap();
    until(&mut f.app, "focus B", |a| {
        a.doc().path.as_ref() == Some(&f.other)
    });
    let mut resolved = f.action("source.fixAll", json!([edit(0, 3, "FIX")]));
    resolved["title"] = json!("lazy");
    f.release(resolve, resolved);
    let organize = f.token(3);
    f.release(organize, Value::Null);
    f.saved();
    assert_eq!(f.model().text.to_string(), "FIX 猫🙂 A\r\nlast\r\n");
    assert_eq!(f.app.doc().path.as_ref(), Some(&f.other));
    assert_eq!(f.app.doc().text.to_string(), "B unchanged\r\n");
    assert_eq!(
        fs::read(&f.path).unwrap(),
        f.model().text.to_string().as_bytes()
    );
    assert!(
        f.app.message.contains("command-bearing"),
        "{}",
        f.app.message
    );
}
#[test]
fn edit_undo_nonce_and_profile_aba_retire_held_callbacks_without_losing_redo() {
    for variant in 0..3 {
        let mut f = Fixture::new(false);
        f.dirty();
        let first = f.save();
        let before = f.model().text.to_string();
        let intent = f.app.saving.latest.as_ref().unwrap().id;
        match variant {
            0 => {
                assert_eq!(f.model().cursor, 4); // dirty() leaves the caret after "RAW ".
                f.app.doc_mut().insert("new", false);
                f.app.doc_mut().undo();
                assert_eq!(f.model().cursor, 4);
            }
            1 => {
                f.app.execute("workbench.action.files.save", Value::Null);
                assert_ne!(f.app.saving.latest.as_ref().map(|i| i.id), Some(intent));
            }
            _ => {
                f.app.invalidate_settings_profile().unwrap();
                f.app.settings_profile_loaded();
                f.app.invalidate_settings_profile().unwrap();
                f.app.settings_profile_loaded();
            }
        }
        f.app.poll();
        f.release(
            first,
            json!([f.action("source.fixAll", json!([edit(0, 3, "BAD")]))]),
        );
        until(&mut f.app, "settle canceled action", |a| {
            a.lsp.as_ref().unwrap().action_available()
        });
        if variant == 1 {
            f.saved();
            assert_eq!(fs::read(&f.path).unwrap(), before.as_bytes());
        } else {
            assert_eq!(fs::read(&f.path).unwrap(), ORIGINAL.as_bytes());
        }
        assert_eq!(f.model().text.to_string(), before);
        assert_eq!(f.model().id, f.id);
        if variant == 0 {
            f.app.doc_mut().redo();
            assert_eq!(f.model().text.to_string(), "RAW new".to_owned() + ORIGINAL);
            assert_eq!(f.model().cursor, 7);
        }
        assert_eq!(f.rows().len(), 1);
    }
}
#[test]
fn deadline_keeps_actual_capacity_busy_skips_fresh_save_and_late_result_is_inert() {
    let mut f = Fixture::new(false);
    f.dirty();
    let first = f.save();
    let before = f.model().text.to_string();
    f.app.saving.actions.active.as_mut().unwrap().started = Instant::now() - DEADLINE;
    f.app.poll();
    f.saved();
    assert_eq!(fs::read(&f.path).unwrap(), before.as_bytes());
    assert!(!f.app.lsp.as_ref().unwrap().action_available());
    f.app.execute("workbench.action.files.save", Value::Null);
    f.saved();
    assert_eq!(f.rows().len(), 1);
    f.release(
        first,
        json!([f.action("source.fixAll", json!([edit(0, 3, "BAD")]))]),
    );
    until(&mut f.app, "late terminal releases actual capacity", |a| {
        a.lsp.as_ref().unwrap().action_available()
    });
    assert_eq!(f.model().text.to_string(), before);
    assert_eq!(fs::read(&f.path).unwrap(), before.as_bytes());
    assert!(f.app.modal.is_none());
    assert_eq!(f.rows().len(), 1);
}
#[test]
fn invalid_secondary_edit_and_escape_preserve_exact_bytes_redo_and_disk() {
    let mut f = Fixture::new(false);
    f.dirty();
    let first = f.save();
    let uri = crate::lsp::file_uri(&f.other).unwrap();
    let invalid = json!({"title":"other","kind":"source.fixAll","edit":{"changes":{crate::lsp::file_uri(&f.path).unwrap():[edit(0,3,"BAD")],uri:[edit(0,0,"BAD")]}}});
    f.release(first, json!([invalid]));
    let second = f.token(2);
    f.release(second, Value::Null);
    f.saved();
    assert_eq!(f.model().text.to_string(), "RAW 猫🙂 A\r\nlast\r\n");
    assert_eq!(fs::read(&f.other).unwrap(), b"B unchanged\r\n");
    let before = fs::read(&f.path).unwrap();
    assert_eq!(f.model().cursor, 4); // No rejected action moved the originating caret.
    f.app.doc_mut().insert("new", false);
    f.app.doc_mut().undo();
    assert_eq!(f.model().cursor, 4);
    let count = f.rows().len();
    f.app.execute("workbench.action.files.save", Value::Null);
    let held = f.token(count + 1);
    assert!(f.app.escape_save_code_actions());
    assert!(!f.app.saves_pending());
    f.release(
        held,
        json!([f.action("source.fixAll", json!([edit(0, 3, "BAD")]))]),
    );
    until(&mut f.app, "Escape late release", |a| {
        a.lsp.as_ref().unwrap().action_available()
    });
    assert_eq!(fs::read(&f.path).unwrap(), before);
    f.app.doc_mut().redo();
    assert_eq!(f.model().text.to_string(), "RAW new猫🙂 A\r\nlast\r\n");
    assert_eq!(f.model().cursor, 7);
}

#[test]
fn admitted_edit_survives_as_unsaved_work_when_post_action_sync_exceeds_transport_limit() {
    let mut f = Fixture::new(false);
    let prefix = "x".repeat(15 * 1024 * 1024);
    f.app.doc_mut().insert(&prefix, false);
    let before = f.model().text.to_string();
    let first = f.save();
    // The real peer has acknowledged receiving the smaller initial document.
    // Its edit keeps the native document below32MiB but pushes didChange over
    // the independent16MiB protocol frame limit, so publication must fail.
    f.app.saving.actions.active.as_mut().unwrap().started = Instant::now();
    let added = "y".repeat(1536 * 1024);
    f.release(
        first,
        json!([f.action("source.fixAll", json!([edit(0, 0, &added)]))]),
    );
    until(&mut f.app, "post-action publication failure", |a| {
        !a.saves_pending()
    });
    assert!(f.app.message.contains("Save retired"), "{}", f.app.message);
    assert!(f.model().dirty());
    assert_eq!(f.model().id, f.id);
    assert_eq!(f.model().text.len_bytes(), before.len() + added.len());
    assert_eq!(fs::read(&f.path).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(f.rows().len(), 1);
    f.app.doc_mut().undo();
    assert_eq!(f.model().text.to_string(), before);
    f.app.doc_mut().redo();
    assert_eq!(f.model().text.len_bytes(), before.len() + added.len());
}
