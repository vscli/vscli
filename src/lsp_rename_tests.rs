//! Real framed peer settlement, independent of advisory cancel and UI deadlines.
use super::*;

const ORIGINAL: &str = "猫🙂 source\r\nnext\r\n";
const PEER: &str = r#"
import json,sys
from pathlib import Path
trace=Path(sys.argv[1]); held={}; calls=[]; cancellations=[]; records=0
def send(value):
 data=json.dumps(value,ensure_ascii=False).encode();sys.stdout.buffer.write(b'Content-Length: '+str(len(data)).encode()+b'\r\n\r\n'+data);sys.stdout.buffer.flush()
def notice(text):send({'jsonrpc':'2.0','method':'window/showMessage','params':{'type':3,'message':text}})
while True:
 length=None
 while True:
  line=sys.stdin.buffer.readline()
  if not line:sys.exit(0)
  if line in (b'\r\n',b'\n'):break
  if line.lower().startswith(b'content-length:'):length=int(line.split(b':',1)[1])
 if length is None:continue
 assert 0<=length<=4*1024*1024
 data=sys.stdin.buffer.read(length);assert len(data)==length
 message=json.loads(data);method=message.get('method');ident=message.get('id');params=message.get('params',{})
 records+=1;assert records<=128
 with trace.open('a',encoding='utf-8') as file:file.write(json.dumps({'method':method,'params':params},ensure_ascii=False)+'\n')
 if method=='initialize':send({'jsonrpc':'2.0','id':ident,'result':{'capabilities':{'positionEncoding':'utf-16','textDocumentSync':1,'renameProvider':{'prepareProvider':True}}}})
 elif method in ('textDocument/prepareRename','textDocument/rename'):
  assert set(params)==({'textDocument','position'} if method.endswith('prepareRename') else {'textDocument','position','newName'})
  assert set(params['textDocument'])=={'uri'}
  held[ident]=message;calls.append(ident);notice('rename-held:'+str(ident))
 elif method=='$/cancelRequest':
  assert params['id'] in held;cancellations.append(params['id']);notice('rename-cancelled:'+str(params['id']))
 elif method=='fixture/malformed':
  ident=params['id'];assert ident in held
  for frame in [
   {'jsonrpc':'1.0','id':ident,'result':None},
   {'jsonrpc':'2.0','id':ident,'result':None,'error':{'code':-1,'message':'both'}},
   {'jsonrpc':'2.0','id':ident,'error':{'code':2147483648,'message':'invalid code'}},
   {'jsonrpc':'2.0','id':ident,'error':{'code':-1,'message':12}},
   {'jsonrpc':'2.0','id':ident,'error':[]},
   {'jsonrpc':'2.0','id':ident},
   {'jsonrpc':'2.0','id':str(ident),'result':None},
   {'jsonrpc':'2.0','id':ident,'method':'window/showMessage','params':{'message':'not settlement'},'result':None}]:send(frame)
  notice('rename-malformed:'+str(ident))
 elif method=='fixture/release':
  ident=params['id'];assert ident in held;del held[ident]
  send({'jsonrpc':'2.0','id':ident,'result':params['result'],'document_id':999999,'revision':999999,'text_epoch':999999,'source':{'path':'forged.cpp','cursor':999999}})
  notice('rename-released:'+str(ident))
 elif method=='fixture/state':notice('rename-state:'+json.dumps({'calls':calls,'cancellations':cancellations,'held':list(held)}))
 elif method is None and ident is not None:
  # This is our deliberately malformed server REQUEST's client response, not
  # a new client request. Echoing an error would fabricate rename settlement.
  assert message.get('error',{}).get('code')==-32601
  notice('rename-peer-response:'+str(ident))
 elif method=='shutdown':send({'jsonrpc':'2.0','id':ident,'result':None})
 elif method=='exit':break
 elif ident is not None:send({'jsonrpc':'2.0','id':ident,'error':{'code':-32601,'message':'unexpected fixture method'}})
"#;

fn until(client: &mut Client, predicate: impl Fn(&Client, &[Event]) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    loop {
        events.extend(client.poll().unwrap());
        if predicate(client, &events) {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "Framed rename peer did not reach its actual barrier"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn marker(client: &mut Client, text: &str) -> Vec<Event> {
    until(client, |_, events| {
        events
            .iter()
            .any(|event| matches!(event, Event::Message(value) if value == text))
    })
}
fn fixture() -> (tempfile::TempDir, Document, Client) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.cpp");
    std::fs::write(&path, ORIGINAL).unwrap();
    let mut document = Document::open(&path).unwrap();
    document.insert("dirty ", false);
    let server = directory.path().join("rename_peer.py");
    std::fs::write(&server, PEER).unwrap();
    let python = std::env::var("VSCLI_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let mut client = Client::start(
        &python,
        &[
            server.to_string_lossy().into_owned(),
            directory
                .path()
                .join("requests.jsonl")
                .to_string_lossy()
                .into_owned(),
        ],
        directory.path(),
        "cpp".into(),
    )
    .unwrap();
    until(&mut client, |client, _| client.ready);
    client.sync(std::slice::from_ref(&document)).unwrap();
    assert!(client.rename_preparation_supported());
    (directory, document, client)
}
fn state(client: &mut Client) -> Value {
    client.notify("fixture/state", json!({})).unwrap();
    until(client, |_, events| {
        events
            .iter()
            .any(|event| matches!(event, Event::Message(text) if text.starts_with("rename-state:")))
    })
    .into_iter()
    .find_map(|event| match event {
        Event::Message(text) => text
            .strip_prefix("rename-state:")
            .map(|value| serde_json::from_str(value).unwrap()),
        _ => None,
    })
    .unwrap()
}
fn no_result(events: &[Event]) {
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::RenameResult(_, _)))
    );
}

#[test]
fn held_prepare_ignores_advisory_cancel_and_malformed_frames_until_actual_terminal_reply() {
    let (directory, document, mut client) = fixture();
    let token = client.request_rename(&document, None, Some(17)).unwrap();
    marker(&mut client, &format!("rename-held:{token}"));
    let original = client.pending[&token].clone();
    assert!(client.request_current(&original));
    assert_eq!(original.document_id, document.id);
    assert_eq!(original.text_epoch, document.text_epoch());
    let selections = document.selections();
    client.cancel_rename_request(token + 1000).unwrap();
    assert!(!client.rename_slot.as_ref().unwrap().canceled);
    client.cancel_rename_request(token).unwrap();
    no_result(&marker(&mut client, &format!("rename-cancelled:{token}")));
    client
        .notify("fixture/malformed", json!({"id":token}))
        .unwrap();
    no_result(&until(&mut client, |_, events| {
        [
            format!("rename-malformed:{token}"),
            format!("rename-peer-response:{token}"),
        ]
        .iter()
        .all(|expected| {
            events
                .iter()
                .any(|event| matches!(event,Event::Message(text) if text == expected))
        })
    }));
    assert_eq!(client.rename_slot.as_ref().unwrap().token, token);
    assert!(client.pending.contains_key(&token));
    assert!(client.ensure_rename_available().is_err());
    let next = client.next_id;
    assert!(
        client
            .request_rename(&document, Some("blocked"), Some(17))
            .is_err()
    );
    assert_eq!(client.next_id, next);
    let peer = state(&mut client);
    assert_eq!(peer["calls"], json!([token]));
    assert_eq!(peer["cancellations"], json!([token]));
    assert_eq!(peer["held"], json!([token]));
    client.notify("fixture/release", json!({"id":token,"result":{"start":{"line":0,"character":6},"end":{"line":0,"character":9}}})).unwrap();
    no_result(&marker(&mut client, &format!("rename-released:{token}")));
    assert!(client.rename_slot.is_none());
    assert!(!client.pending.contains_key(&token));
    assert!(client.ensure_rename_available().is_ok());

    let fresh = client.request_rename(&document, None, Some(17)).unwrap();
    marker(&mut client, &format!("rename-held:{fresh}"));
    let value = json!({"start":{"line":0,"character":6},"end":{"line":0,"character":9}});
    client
        .notify("fixture/release", json!({"id":fresh,"result":value}))
        .unwrap();
    let events = marker(&mut client, &format!("rename-released:{fresh}"));
    let (request, result) = events
        .into_iter()
        .find_map(|event| match event {
            Event::RenameResult(request, result) if request.token == fresh => {
                Some((request, result))
            }
            _ => None,
        })
        .expect("A genuinely completed fresh request must emit its original receipt");
    assert_eq!(result.unwrap(), value);
    assert_eq!(request.document_id, document.id);
    assert_eq!(request.revision, document.revision);
    assert_eq!(request.text_epoch, document.text_epoch());
    assert_eq!(request.cursor, document.cursor);
    assert_eq!(request.path, document.path.as_ref().unwrap().clone());
    assert!(Arc::ptr_eq(&request.server, &original.server));
    assert!(client.request_current(&request));
    assert_eq!(document.selections(), selections);
    assert_eq!(document.text.to_string(), format!("dirty {ORIGINAL}"));
    assert_eq!(
        std::fs::read(directory.path().join("main.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
}

#[test]
fn held_edit_timeout_reports_once_retains_actual_slot_and_cannot_revive_after_edit_undo() {
    let (directory, mut document, mut client) = fixture();
    let token = client
        .request_rename(&document, Some("renamed"), Some(17))
        .unwrap();
    marker(&mut client, &format!("rename-held:{token}"));
    let original = client.pending[&token].clone();
    let selections = document.selections();
    assert!(client.request_current(&original));
    document.insert("X", false);
    document.undo();
    assert_eq!(document.text.to_string(), format!("dirty {ORIGINAL}"));
    assert_eq!(document.selections(), selections);
    assert_ne!(document.text_epoch(), original.text_epoch);
    client.sync(std::slice::from_ref(&document)).unwrap();
    assert!(!client.request_current(&original));
    client.pending.get_mut(&token).unwrap().started = Instant::now() - Duration::from_secs(16);
    let events = client.poll().unwrap();
    assert_eq!(events.iter().filter(|event| matches!(event,Event::RenameResult(request,Err(message)) if request.token == token && message.contains("timed out"))).count(), 1);
    // The peer can acknowledge cancellation during that same poll. Retain its
    // already observed barrier rather than waiting for a second notification.
    let canceled = format!("rename-cancelled:{token}");
    if !events
        .iter()
        .any(|event| matches!(event, Event::Message(value) if value == &canceled))
    {
        no_result(&marker(&mut client, &canceled));
    }
    client
        .notify("fixture/malformed", json!({"id":token}))
        .unwrap();
    no_result(&until(&mut client, |_, events| {
        [
            format!("rename-malformed:{token}"),
            format!("rename-peer-response:{token}"),
        ]
        .iter()
        .all(|expected| {
            events
                .iter()
                .any(|event| matches!(event,Event::Message(text) if text == expected))
        })
    }));
    assert!(client.rename_slot.as_ref().unwrap().timed_out);
    assert!(client.pending.contains_key(&token));
    assert!(client.ensure_rename_available().is_err());
    let next = client.next_id;
    assert!(
        client
            .request_rename(&document, Some("blocked"), Some(17))
            .is_err()
    );
    assert_eq!(client.next_id, next);
    let peer = state(&mut client);
    assert_eq!(peer["calls"], json!([token]));
    assert_eq!(peer["cancellations"], json!([token]));
    client
        .notify(
            "fixture/release",
            json!({"id":token,"result":{"changes":{}}}),
        )
        .unwrap();
    no_result(&marker(&mut client, &format!("rename-released:{token}")));
    assert!(client.rename_slot.is_none());
    assert!(!client.pending.contains_key(&token));
    assert!(client.ensure_rename_available().is_ok());
    assert!(!client.request_current(&original));
    document.redo();
    assert_eq!(document.text.to_string(), format!("dirty X{ORIGINAL}"));
    assert_eq!(
        std::fs::read(directory.path().join("main.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
}
