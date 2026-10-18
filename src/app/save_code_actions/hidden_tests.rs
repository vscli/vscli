use super::*;
use crate::{
    app::saving::document_interests::{Admission, Interest, NativeSaveSource, Receipt},
    settings::Settings,
};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

const ORIGINAL: &str = "猫🙂 A\r\n";
const PEER: &str = r#"
import json,sys,urllib.parse,urllib.request
from pathlib import Path
root=Path(sys.argv[1]);docs={}
def send(message):
 body=json.dumps({'jsonrpc':'2.0',**message},ensure_ascii=False).encode()
 sys.stdout.buffer.write(f'Content-Length: {len(body)}\r\n\r\n'.encode()+body);sys.stdout.buffer.flush()
while True:
 headers={}
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  key,value=line.decode().split(':',1);headers[key.lower()]=value.strip()
 m=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
 method=m.get('method');p=m.get('params',{});ident=m.get('id');uri=p.get('textDocument',{}).get('uri')
 if method=='initialize':send({'id':ident,'result':{'capabilities':{'textDocumentSync':{'openClose':True,'change':1,'save':{'includeText':True}},'codeActionProvider':{'resolveProvider':True},'documentFormattingProvider':True}}})
 elif method=='textDocument/didOpen':docs[uri]={'text':p['textDocument']['text'],'version':p['textDocument']['version']}
 elif method=='textDocument/didChange':docs[uri]={'text':p['contentChanges'][0]['text'],'version':p['textDocument']['version']}
 elif method=='fixture/release':send({'id':p['id'],'result':p['result']})
 elif method=='shutdown':send({'id':ident,'result':None})
 elif method=='exit':break
 row={'method':method,'id':ident,'params':p,'source':docs.get(uri)}
 if method=='textDocument/didSave':row['disk']=Path(urllib.request.url2pathname(urllib.parse.urlparse(uri).path)).read_bytes().decode('utf-8')
 with (root/'events.jsonl').open('a',encoding='utf-8') as f:f.write(json.dumps(row,ensure_ascii=False)+'\n')
"#;
struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    app: App,
    path: PathBuf,
    id: u64,
    active: u64,
    owner: Arc<AtomicBool>,
    local: Arc<AtomicBool>,
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
fn rows(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("events.jsonl"))
        .ok()
        .map(|value| {
            value
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        })
        .unwrap_or_default()
}
impl Fixture {
    fn new(actions: bool, formatting: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("hidden.cpp");
        let active_path = root.join("active.cpp");
        fs::write(&path, ORIGINAL).unwrap();
        fs::write(&active_path, "active unchanged\r\n").unwrap();
        let peer = root.join("peer.py");
        fs::write(&peer, PEER).unwrap();
        let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
        app.settings = Settings::from_values(json!({
            "editor.codeActionsOnSave":if actions {json!({"source.fixAll":"explicit"})} else {json!({})},
            "editor.formatOnSave":formatting,"files.autoSave":"off","breadcrumbs.enabled":false,
            "vscli.languageServer.enabled":false,
        }).as_object().unwrap().clone(), "native hidden save participants").unwrap();
        let mut hidden = Document::open_existing(&path).unwrap();
        app.settings.apply(&mut hidden);
        hidden.insert("RAW ", false);
        let id = hidden.id;
        app.hidden_documents.push(hidden);
        app.open(&active_path).unwrap();
        until(&mut app, "open visible", |app| {
            !app.documents.is_empty() && app.doc().path.as_ref() == Some(&active_path)
        });
        let active = app.doc().id;
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
        until(&mut app, "native initialize", |app| {
            app.lsp.as_ref().is_some_and(|client| client.ready)
        });
        Self {
            _directory: directory,
            root,
            app,
            path,
            id,
            active,
            owner: Arc::new(AtomicBool::new(false)),
            local: Arc::new(AtomicBool::new(false)),
        }
    }
    fn hidden(&self) -> &Document {
        self.app
            .hidden_documents
            .iter()
            .find(|doc| doc.id == self.id)
            .unwrap()
    }
    fn hidden_mut(&mut self) -> &mut Document {
        self.app
            .hidden_documents
            .iter_mut()
            .find(|doc| doc.id == self.id)
            .unwrap()
    }
    fn save(&mut self) -> Interest {
        let source = NativeSaveSource::capture(&self.app, self.id).unwrap();
        let Admission::Enqueued(interest) = self
            .app
            .request_refactoring_save_interest(1, self.owner.clone(), self.local.clone(), source)
            .unwrap()
        else {
            panic!("Expected named save");
        };
        interest
    }
    fn request(&mut self, method: &str) -> Value {
        let root = self.root.clone();
        until(&mut self.app, "actual participant request", |_| {
            rows(&root).iter().any(|row| row["method"] == method)
        });
        rows(&self.root)
            .into_iter()
            .find(|row| row["method"] == method)
            .unwrap()
    }
    fn release(&self, request: &Value, result: Value) {
        self.app
            .lsp
            .as_ref()
            .unwrap()
            .fixture_notify(
                "fixture/release",
                json!({"id":request["id"],"result":result}),
            )
            .unwrap();
    }
    fn finish(&mut self, interest: Interest) -> Receipt {
        until(&mut self.app, "actual native joined receipt", |app| {
            app.refactoring_save_completion(interest).is_some()
        });
        self.app
            .take_refactoring_save_completion(interest)
            .unwrap()
            .receipt
    }
}
fn edit(text: &str) -> Value {
    json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"newText":text})
}

#[test]
fn hidden_actions_then_formatter_publish_didchange_before_actual_save_and_didsave_without_show() {
    let mut f = Fixture::new(true, true);
    let original_id = f.id;
    let interest = f.save();
    let action = f.request("textDocument/codeAction");
    assert_eq!(
        action["params"]["textDocument"]["uri"],
        crate::lsp::file_uri(&f.path).unwrap()
    );
    assert_eq!(action["source"]["text"], format!("RAW {ORIGINAL}"));
    f.release(&action, json!([{"title":"fix hidden","kind":"source.fixAll","edit":{
        "documentChanges":[{"textDocument":{"uri":crate::lsp::file_uri(&f.path).unwrap(),"version":action["source"]["version"]},"edits":[edit("ACTION ")]}]
    }}]));
    let format = f.request("textDocument/formatting");
    assert_eq!(format["source"]["text"], format!("ACTION {ORIGINAL}"));
    assert!(
        format["source"]["version"].as_i64().unwrap()
            > action["source"]["version"].as_i64().unwrap()
    );
    f.release(&format, json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":7}},"newText":"FORMATTED "}]));
    assert_eq!(
        f.finish(interest),
        Receipt::Committed {
            document: original_id,
            save_generation: 1
        }
    );
    let root = f.root.clone();
    until(&mut f.app, "actual didSave", |_| {
        rows(&root)
            .iter()
            .any(|row| row["method"] == "textDocument/didSave")
    });
    let events = rows(&f.root);
    let uri = crate::lsp::file_uri(&f.path).unwrap();
    let target: Vec<_> = events
        .iter()
        .filter(|row| row["params"]["textDocument"]["uri"] == uri)
        .collect();
    assert_eq!(
        target
            .iter()
            .filter(|row| row["method"] == "textDocument/didOpen")
            .count(),
        1
    );
    assert!(
        !target
            .iter()
            .any(|row| row["method"] == "textDocument/didClose")
    );
    let changed = target
        .iter()
        .position(|row| {
            row["method"] == "textDocument/didChange"
                && row["source"]["text"] == format!("FORMATTED {ORIGINAL}")
        })
        .unwrap();
    let saved = target
        .iter()
        .position(|row| row["method"] == "textDocument/didSave")
        .unwrap();
    assert!(changed < saved);
    assert_eq!(target[saved]["disk"], format!("FORMATTED {ORIGINAL}"));
    assert_eq!(
        target[saved]["params"]["text"],
        format!("FORMATTED {ORIGINAL}")
    );
    assert_eq!(f.hidden().id, original_id);
    assert_eq!(f.app.doc().id, f.active);
    assert_eq!(f.app.hidden_documents.len(), 1);
    assert!(!f.app.documents.iter().any(|doc| doc.id == original_id));
}

#[test]
fn hidden_formatting_edit_undo_invalidates_callback_and_retains_actual_slot_until_reply() {
    let mut f = Fixture::new(false, true);
    let interest = f.save();
    let request = f.request("textDocument/formatting");
    let before = f.hidden().text.clone();
    f.hidden_mut().insert("independent ", false);
    f.hidden_mut().undo();
    assert_eq!(f.hidden().text, before);
    f.app.poll();
    assert!(!f.app.lsp.as_ref().unwrap().formatting_available());
    f.release(&request, json!([edit("STALE ")]));
    assert_eq!(f.finish(interest), Receipt::NotSaved);
    until(&mut f.app, "actual canceled formatter settlement", |app| {
        app.lsp.as_ref().unwrap().formatting_available()
    });
    assert_eq!(f.hidden().text, before);
    assert_eq!(fs::read_to_string(&f.path).unwrap(), ORIGINAL);
    assert_eq!(f.hidden().save_generation(), 0);
    assert_eq!(f.app.doc().id, f.active);
}

#[test]
fn hidden_action_cohort_cancel_keeps_callback_occupied_and_does_not_apply_late_edit() {
    let mut f = Fixture::new(true, false);
    let interest = f.save();
    let request = f.request("textDocument/codeAction");
    let before = f.hidden().text.clone();
    f.local.store(true, Ordering::Release);
    f.app.poll();
    assert!(!f.owner.load(Ordering::Acquire));
    assert!(!f.app.lsp.as_ref().unwrap().action_available());
    f.release(
        &request,
        json!([{"title":"late","kind":"source.fixAll","edit":{"changes":{
            crate::lsp::file_uri(&f.path).unwrap():[edit("STALE ")]
        }}}]),
    );
    assert_eq!(f.finish(interest), Receipt::NotSaved);
    until(&mut f.app, "actual canceled action settlement", |app| {
        app.lsp.as_ref().unwrap().action_available()
    });
    assert_eq!(f.hidden().text, before);
    assert_eq!(fs::read_to_string(&f.path).unwrap(), ORIGINAL);
}
