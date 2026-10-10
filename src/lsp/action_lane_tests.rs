//! A real framed peer proves actual occupancy independently of logical interest.
use super::*;

const PEER: &str = r#"
import json,sys
held={}
def send(packet):
    data=json.dumps(packet).encode()
    assert len(data)<=16*1024*1024
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
def notice(text): send({'jsonrpc':'2.0','method':'window/showMessage','params':{'type':3,'message':text}})
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline(8193)
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1)
        headers[key.lower()]=value.strip()
    length=int(headers['content-length']); assert 0<=length<=16*1024*1024
    message=json.loads(sys.stdin.buffer.read(length))
    method,ident,params=message.get('method'),message.get('id'),message.get('params',{})
    if method=='initialize':
        send({'jsonrpc':'2.0','id':ident,'result':{'capabilities':{'textDocumentSync':1,'codeActionProvider':{'resolveProvider':True},'executeCommandProvider':{'commands':['fixture.action']}}}})
    elif method in ('textDocument/codeAction','codeAction/resolve','workspace/executeCommand'):
        item={'title':'Lazy action','kind':'source.fixAll','data':{'token':42,'nested':['猫🙂']}}
        result=[item] if method=='textDocument/codeAction' else ({**params,'edit':{'changes':{}}} if method=='codeAction/resolve' else None)
        packet={'jsonrpc':'2.0','id':ident,'result':result}
        if params.get('hold'):
            held[ident]=packet; notice('held-%d'%ident)
        else: send(packet)
    elif method=='$/cancelRequest': notice('cancel-%d'%params['id'])
    elif method=='fixture/packet':
        send(params['packet']); notice(params['barrier'])
    elif method=='fixture/release':
        packet=held.pop(params['id'])
        if params.get('error'): packet={'jsonrpc':'2.0','id':params['id'],'error':{'code':-32603,'message':'held failure'}}
        if 'result' in params: packet={'jsonrpc':'2.0','id':params['id'],'result':params['result']}
        send(packet); notice('released-%d'%params['id'])
    elif method=='textDocument/hover': send({'jsonrpc':'2.0','id':ident,'result':None})
    elif method=='shutdown': send({'jsonrpc':'2.0','id':ident,'result':None})
    elif method=='exit': break
"#;

struct Fixture {
    client: Client,
    document: Document,
    _root: tempfile::TempDir,
}
fn until(client: &mut Client, predicate: impl Fn(&Client, &[Event]) -> bool) -> Vec<Event> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut received = Vec::new();
    loop {
        received.extend(client.poll().unwrap());
        if predicate(client, &received) {
            return received;
        }
        assert!(
            Instant::now() < deadline,
            "Actual action protocol barrier timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.rs");
    std::fs::write(&path, "猫🙂\r\n").unwrap();
    let document = Document::open(&path).unwrap();
    let mut client = Client::start(
        if cfg!(windows) { "python" } else { "python3" },
        &["-u".into(), "-c".into(), PEER.into()],
        root.path(),
        "rust".into(),
    )
    .unwrap();
    until(&mut client, |client, _| client.ready);
    client.sync(std::slice::from_ref(&document)).unwrap();
    Fixture {
        client,
        document,
        _root: root,
    }
}
fn notice(events: &[Event], text: &str) -> bool {
    events
        .iter()
        .any(|event| matches!(event,Event::Message(message) if message==text))
}
fn discovery(f: &mut Fixture) -> (Request, Value) {
    let token = f
        .client
        .request_code_actions(&f.document, json!({}), None)
        .unwrap();
    until(&mut f.client, |_, events| {
        events
            .iter()
            .any(|event| matches!(event,Event::ActionResult(request,_) if request.token==token))
    })
    .into_iter()
    .find_map(|event| match event {
        Event::ActionResult(request, Ok(value)) if request.token == token => {
            Some((request, value[0].clone()))
        }
        _ => None,
    })
    .unwrap()
}

#[test]
fn exact_cancellation_and_malformed_envelopes_cannot_release_an_actual_action() {
    let mut f = fixture();
    let token = f
        .client
        .request_code_actions(&f.document, json!({"hold":true}), Some(31))
        .unwrap();
    until(&mut f.client, |_, events| {
        notice(events, &format!("held-{token}"))
    });
    f.client.cancel_action_request(token + 1).unwrap();
    assert!(f.client.action_request_current(token));
    f.client.cancel_action_request(token).unwrap();
    until(&mut f.client, |_, events| {
        notice(events, &format!("cancel-{token}"))
    });
    for _ in 0..32 {
        f.client.cancel_action_request(token).unwrap();
        assert!(
            f.client
                .request_code_actions(&f.document, json!({}), None)
                .is_err()
        );
        assert_eq!(f.client.pending.len(), 1);
    }
    for (index, packet) in [
        json!({"jsonrpc":"2.0","id":token}),
        json!({"jsonrpc":"2.0","id":token,"result":[],"error":{"code":-1,"message":"bad"}}),
        json!({"jsonrpc":"1.0","id":token,"result":[]}),
        json!({"jsonrpc":"2.0","id":token,"error":{"code":"bad","message":"bad"}}),
        json!({"jsonrpc":"2.0","id":token+1,"result":[]}),
    ]
    .into_iter()
    .enumerate()
    {
        let barrier = format!("malformed-{index}");
        f.client
            .notify("fixture/packet", json!({"packet":packet,"barrier":barrier}))
            .unwrap();
        let events = until(&mut f.client, |_, events| notice(events, &barrier));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::ActionResult(_, _)))
        );
        assert!(!f.client.action_available());
        assert_eq!(f.client.pending.len(), 1);
    }
    f.client
        .notify("fixture/release", json!({"id":token,"error":true}))
        .unwrap();
    let events = until(&mut f.client, |_, events| {
        notice(events, &format!("released-{token}"))
    });
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::ActionResult(_, _)))
    );
    assert!(f.client.action_available());
    assert!(f.client.pending.is_empty());
    discovery(&mut f);
    assert_eq!(f.document.text.to_string(), "猫🙂\r\n");
    assert_eq!(
        std::fs::read(f.document.path.as_ref().unwrap()).unwrap(),
        "猫🙂\r\n".as_bytes()
    );
}

#[test]
fn all_action_methods_timeout_once_retain_provenance_and_recover_only_on_terminal_reply() {
    for method in [
        "textDocument/codeAction",
        "codeAction/resolve",
        "workspace/executeCommand",
    ] {
        let mut f = fixture();
        let token = if method == "textDocument/codeAction" {
            f.client
                .request_code_actions(&f.document, json!({"hold":true}), None)
                .unwrap()
        } else {
            let (original, mut item) = discovery(&mut f);
            item["hold"] = json!(true);
            if method == "codeAction/resolve" {
                f.client.resolve_code_action(&original, item).unwrap()
            } else {
                f.client
                    .execute_action_command(
                        &original,
                        json!({"command":"fixture.action","hold":true}),
                    )
                    .unwrap()
            }
        };
        until(&mut f.client, |_, events| {
            notice(events, &format!("held-{token}"))
        });
        let original = f.client.pending[&token].clone();
        assert!(f.client.action_request_current(token));
        f.client.pending.get_mut(&token).unwrap().started =
            Instant::now() - Duration::from_secs(16);
        let events = f.client.poll().unwrap();
        assert_eq!(events.iter().filter(|event| matches!(event,Event::ActionResult(request,Err(message)) if request.token==token && Arc::ptr_eq(&request.server,&original.server) && Arc::ptr_eq(&request.workspace,&original.workspace) && message.contains("timed out"))).count(),1);
        assert_eq!(f.client.pending.len(), 1);
        assert!(f.client.action_channel_closed());
        assert!(!f.client.action_request_current(token));
        for _ in 0..32 {
            assert!(
                f.client
                    .request("textDocument/codeAction", &f.document, json!({}))
                    .is_err()
            );
            assert!(
                f.client
                    .follow_up(&original, "codeAction/resolve", json!({"title":"Lazy"}))
                    .is_err()
            );
            assert!(
                f.client
                    .follow_up(
                        &original,
                        "workspace/executeCommand",
                        json!({"command":"fixture.action"})
                    )
                    .is_err()
            );
            assert!(
                !f.client
                    .poll()
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event, Event::ActionResult(_, _)))
            );
            assert_eq!(f.client.pending.len(), 1);
        }
        f.client
            .request("textDocument/hover", &f.document, json!({}))
            .unwrap();
        until(&mut f.client, |_, events| {
            events.iter().any(|event| matches!(event,Event::Response(request,_) if request.method=="textDocument/hover"))
        });
        f.client
            .notify("fixture/release", json!({"id":token}))
            .unwrap();
        let events = until(&mut f.client, |_, events| {
            notice(events, &format!("released-{token}"))
        });
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::ActionResult(_, _)))
        );
        assert!(f.client.action_available());
        assert!(!f.client.action_channel_closed());
        discovery(&mut f);
    }
}

#[test]
fn resolved_action_preserves_opaque_data_and_rejects_immutable_mutation_with_released_capacity() {
    let mut f = fixture();
    let (original, mut item) = discovery(&mut f);
    item["hold"] = json!(true);
    let token = f
        .client
        .resolve_code_action(&original, item.clone())
        .unwrap();
    until(&mut f.client, |_, events| {
        notice(events, &format!("held-{token}"))
    });
    f.client
        .notify("fixture/release", json!({"id":token}))
        .unwrap();
    let events = until(&mut f.client, |_, events| {
        notice(events, &format!("released-{token}"))
    });
    assert!(events.iter().any(|event| matches!(event,Event::ActionResult(request,Ok(value)) if request.token==token && value["data"]==item["data"] && value["edit"]==json!({"changes":{}}))));
    assert!(f.client.action_available());
    for key in [
        "title",
        "kind",
        "diagnostics",
        "isPreferred",
        "disabled",
        "command",
        "data",
    ] {
        let token = f
            .client
            .resolve_code_action(&original, item.clone())
            .unwrap();
        until(&mut f.client, |_, events| {
            notice(events, &format!("held-{token}"))
        });
        let mut changed = item.clone();
        changed[key] = json!("changed");
        f.client
            .notify("fixture/release", json!({"id":token,"result":changed}))
            .unwrap();
        let events = until(&mut f.client, |_, events| {
            notice(events, &format!("released-{token}"))
        });
        assert!(events.iter().any(|event| matches!(event,Event::ActionResult(request,Err(message)) if request.token==token && message.contains(key))));
        assert!(f.client.action_available());
    }
}

#[test]
fn native_source_or_same_bytes_document_lifetime_change_rejects_followup_before_admission() {
    let mut f = fixture();
    let (original, item) = discovery(&mut f);
    let before = f.client.next_id;
    let mut wrong = original.clone();
    wrong.server = Arc::new(());
    assert!(f.client.resolve_code_action(&wrong, item.clone()).is_err());
    f.document.insert("x", false);
    f.document.undo();
    f.client.sync(std::slice::from_ref(&f.document)).unwrap();
    assert_eq!(f.document.text.to_string(), "猫🙂\r\n");
    assert!(
        f.client
            .resolve_code_action(&original, item.clone())
            .is_err()
    );
    assert!(
        f.client
            .execute_action_command(&original, json!({"command":"fixture.action"}))
            .is_err()
    );
    assert_eq!(f.client.next_id, before);
    assert!(f.client.pending.is_empty());
    assert!(f.client.action_available());
    let mut replacement = fixture();
    assert!(
        replacement
            .client
            .resolve_code_action(&original, item)
            .is_err()
    );
    assert!(replacement.client.pending.is_empty());
}

#[test]
fn action_json_admission_bounds_escaped_bytes_depth_items_and_preserves_valid_data() {
    let too_escaped = json!({"title":"title","data":"\u{0001}".repeat(ACTION_BYTES/6+1)});
    assert!(action_budget(&too_escaped).is_err());
    let mut deep = json!(0);
    for _ in 0..65 {
        deep = json!([deep]);
    }
    assert!(action_budget(&deep).is_err());
    assert!(
        validate_action_result(
            "textDocument/codeAction",
            &json!(vec![json!({"title":"item"}); 301]),
            None
        )
        .is_err()
    );
    assert!(
        validate_action_result(
            "textDocument/codeAction",
            &json!([{"title":"ok"},{"title":17}]),
            None
        )
        .is_err()
    );
    assert!(validate_action_result("textDocument/codeAction", &Value::Null, None).is_ok());
    let valid = json!({"title":"猫🙂","data":{"opaque":[{"value":false},42,null]}});
    let mut resolved = valid.clone();
    resolved["edit"] = json!({"changes":{}});
    assert!(validate_action_result("codeAction/resolve", &resolved, Some(&valid)).is_ok());
    let mut f = fixture();
    let (original, _) = discovery(&mut f);
    let before = f.client.next_id;
    assert!(
        f.client
            .resolve_code_action(&original, too_escaped)
            .is_err()
    );
    assert!(
        f.client
            .execute_action_command(
                &original,
                json!({"command":"fixture.action","arguments":deep})
            )
            .is_err()
    );
    assert_eq!(f.client.next_id, before);
    assert!(f.client.pending.is_empty());
}

#[test]
fn generic_entry_points_cannot_bypass_the_shared_action_lane_or_source_proof() {
    let mut f = fixture();
    let (original, _) = discovery(&mut f);
    let before = f.client.next_id;
    for method in ["codeAction/resolve", "workspace/executeCommand"] {
        assert!(f.client.request(method, &f.document, json!({})).is_err());
        assert!(
            f.client
                .request_in_view(method, &f.document, json!({}), Some(4))
                .is_err()
        );
    }
    assert!(
        f.client
            .follow_up(&original, "textDocument/codeAction", json!({}))
            .is_err()
    );
    assert_eq!(f.client.next_id, before);
    assert!(f.client.pending.is_empty());
    assert!(f.client.action_available());
    discovery(&mut f);
}
