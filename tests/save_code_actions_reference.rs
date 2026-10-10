//! Scoped consumers of actual pinned source-action save observations.
//! Eligibility is compared across all fourteen cases. Native text/history and
//! framed callbacks are compared only for six named cohorts with at most one
//! mutating action per supported family; broader provider behavior is retained
//! as reference evidence rather than claimed as native equivalence.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::App,
    keys::Profile,
    lsp::Client,
    settings::{SaveActionFamily, SaveActionPolicy, SaveActionReason, Settings},
};

const REFERENCE: &str = "tests/vscode-reference";
const PIN: &str = "912bb683695358a54ae0c670461738984cbb5b95";
const IMPLEMENTED: &[&str] = &[
    "ancestor-never-child",
    "language-composite-single-merge",
    "workspace-array-replaces-object",
    "child-only-dot-boundary",
    "after-delay-always-skipped",
    "after-delay-array-skipped",
];
const PEER: &str = r#"
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
text, count = '', 0
definitions = [('source.organizeImports',1,'imports=1'),
    ('source.fixAll.child',2,'child=1'), ('source.fixAll',0,'fix=1'),
    ('source.fixAllX',3,'other=1')]
def record(kind, **fields):
    global count
    count += 1
    assert count <= 128
    with (root/'protocol.jsonl').open('a',encoding='utf-8') as stream:
        stream.write(json.dumps({'kind':kind,**fields},ensure_ascii=False)+'\n')
def send(message):
    body=json.dumps({'jsonrpc':'2.0',**message},ensure_ascii=False).encode()
    assert len(body) <= 16384
    sys.stdout.buffer.write(('Content-Length: %s\r\n\r\n'%len(body)).encode()+body)
    sys.stdout.buffer.flush()
while True:
    headers, size = {}, 0
    while True:
        line=sys.stdin.buffer.readline(8193)
        if not line: sys.exit(0)
        size += len(line)
        assert size <= 8192
        if line == b'\r\n': break
        key,value=line.decode().split(':',1); headers[key.lower()]=value.strip()
    length=int(headers['content-length']); assert 0<=length<=65536
    body=sys.stdin.buffer.read(length); assert len(body)==length
    message=json.loads(body)
    method,ident,params=message.get('method'),message.get('id'),message.get('params',{})
    if method=='initialize':
        send({'id':ident,'result':{'capabilities':{
            'codeActionProvider':{'codeActionKinds':[row[0] for row in definitions]},
            'textDocumentSync':{'openClose':True,'change':1,'save':{'includeText':True}}}}})
    elif method=='textDocument/didOpen':
        text=params['textDocument']['text']; record('open',text=text)
    elif method=='textDocument/didChange':
        text=params['contentChanges'][-1]['text']; record('change',text=text)
    elif method=='textDocument/codeAction':
        assert set(params)=={'textDocument','range','context'}
        assert params['context']['triggerKind']==2
        assert params['context']['diagnostics']==[]
        only=params['context']['only']; assert len(only)==1
        assert only[0] in ('source.fixAll','source.organizeImports')
        record('actions',only=only[0],triggerKind=2,text=text)
        uri=params['textDocument']['uri']; lines=text.splitlines()
        actions=[]
        for kind,line,replacement in definitions:
            actions.append({'title':kind,'kind':kind,'edit':{'changes':{uri:[{
                'range':{'start':{'line':line,'character':0},'end':{'line':line,
                    'character':len(lines[line].encode('utf-16-le'))//2}},'newText':replacement}]}}})
        send({'id':ident,'result':actions})
    elif method=='textDocument/didSave':
        assert 'text' in params; record('save',text=params['text'])
    elif method=='shutdown': send({'id':ident,'result':None})
    elif method=='exit': break
"#;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn source(name: &str) -> Vec<u8> {
    assert_eq!(Path::new(name).components().count(), 1);
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(REFERENCE)
            .join(name),
    )
    .unwrap()
}
fn corpus() -> Vec<Value> {
    let (directory, stem) = match std::env::var_os("VSCLI_SAVE_CODE_ACTIONS_REFERENCE_DIR") {
        Some(path) => (PathBuf::from(path), "save-code-actions"),
        None => (
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(REFERENCE)
                .join("baselines/1.95.0/save-code-actions"),
            "linux",
        ),
    };
    let trace = fs::read(directory.join(format!("{stem}.json"))).unwrap();
    let evidence = fs::read(directory.join(format!("{stem}-evidence.json"))).unwrap();
    let proof: Value = serde_json::from_slice(
        &fs::read(directory.join(format!("{stem}-provenance.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(proof["version"], "1.95.0");
    assert_eq!(proof["commit"], PIN);
    assert_eq!(proof["caseCount"], 14);
    assert_eq!(proof["snapshotCount"], 162);
    assert_eq!(proof["traceSha256"], sha(&trace));
    assert_eq!(proof["evidenceSha256"], sha(&evidence));
    assert_eq!(
        proof["casesSha256"],
        sha(&source("save-code-actions-cases.json"))
    );
    let rows: Vec<Value> = serde_json::from_slice(&trace).unwrap();
    let raw: Vec<Value> = serde_json::from_slice(&evidence).unwrap();
    let runs = proof["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 14);
    assert_eq!(raw.len(), rows.len());
    assert_eq!(runs.len(), rows.len());
    assert_eq!(
        rows.iter()
            .map(|r| r["observations"].as_array().unwrap().len())
            .sum::<usize>(),
        162
    );
    for ((row, raw), run) in rows.iter().zip(&raw).zip(runs) {
        assert_eq!(row["name"], raw["name"]);
        assert_eq!(row["name"], run["name"]);
        assert_eq!(run["commit"], PIN);
        assert_eq!(raw["setup"]["before"], raw["setup"]["after"]);
        assert_eq!(row["effective"], raw["setup"]["effective"]);
        assert_eq!(row["effectiveKeys"], raw["setup"]["effectiveKeys"]);
        assert_eq!(row["observations"], raw["observations"]);
        let hashes = run["sources"].as_object().unwrap();
        assert_eq!(hashes.len(), 9);
        for (file, expected) in hashes {
            assert_eq!(expected, &json!(sha(&source(file))));
        }
        for saved in raw["saved"].as_array().unwrap() {
            assert_eq!(saved["text"], saved["disk"]);
        }
        let callbacks: Vec<_> = raw["callbacks"].as_array().unwrap().iter()
            .filter(|c|c["phase"]=="target")
            .map(|c|json!({"only":c["only"],"triggerKind":c["triggerKind"],"text":c["text"],"returned":c["returned"]})).collect();
        assert_eq!(row["callbacks"], json!(callbacks));
    }
    rows
}
fn fixtures() -> Vec<Value> {
    serde_json::from_slice(&source("save-code-actions-cases.json")).unwrap()
}
fn settings(root: &Path, fixture: &Value) -> Settings {
    let mut base = json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,
        "editor.quickSuggestions":false,"editor.parameterHints.enabled":false,
        "editor.formatOnSave":false,"files.autoSave":fixture["autosave"],"files.autoSaveDelay":500,
        "editor.tabSize":2,"editor.insertSpaces":true});
    base.as_object_mut()
        .unwrap()
        .extend(fixture["user"].as_object().unwrap().clone());
    let user = root.join("consumer-user.json");
    let workspace = root.join("consumer-workspace.json");
    fs::write(&user, serde_json::to_vec(&base).unwrap()).unwrap();
    fs::write(
        &workspace,
        serde_json::to_vec(&fixture["workspace"]).unwrap(),
    )
    .unwrap();
    Settings::load(&[user, workspace]).unwrap()
}

#[test]
fn supported_policy_eligibility_matches_all_fourteen_observed_save_results() {
    let rows = corpus();
    for fixture in fixtures() {
        let row = rows.iter().find(|r| r["name"] == fixture["name"]).unwrap();
        let root = tempfile::tempdir().unwrap();
        let reason = if fixture["autosave"] == "afterDelay" {
            SaveActionReason::AfterDelay
        } else {
            SaveActionReason::Explicit
        };
        let policy = settings(root.path(), &fixture).save_code_actions("plaintext", reason);
        let text = row["observations"][2]["text"].as_str().unwrap();
        for (family, kind, marker) in [
            (SaveActionFamily::FixAll, "source.fixAll", "fix=1"),
            (SaveActionFamily::FixAll, "source.fixAll.child", "child=1"),
            (
                SaveActionFamily::OrganizeImports,
                "source.organizeImports",
                "imports=1",
            ),
        ] {
            let admitted = match &policy {
                SaveActionPolicy::Native(plan) => plan.allows(family, kind),
                _ => false,
            };
            assert_eq!(
                admitted,
                text.lines().any(|line| line == marker),
                "{}: {kind}",
                fixture["name"]
            );
        }
        if let SaveActionPolicy::Native(plan) = policy {
            assert!(!plan.allows(SaveActionFamily::FixAll, "source.fixAllX"));
            assert!(plan.families.len() <= 2);
        }
    }
}
fn protocol(root: &Path) -> Vec<Value> {
    let bytes = fs::read(root.join("protocol.jsonl")).unwrap_or_default();
    assert!(bytes.len() <= 128 * 16 * 1024);
    bytes
        .split_inclusive(|b| *b == b'\n')
        .filter(|line| line.last() == Some(&b'\n'))
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn compare(app: &App, file: &Path, id: u64, snapshot: &Value) {
    let action = snapshot["action"].as_str().unwrap();
    assert_eq!(app.doc().id, id, "{action}: model identity");
    assert_eq!(
        app.doc().text.to_string(),
        snapshot["text"].as_str().unwrap(),
        "{action}: text"
    );
    assert_eq!(
        app.doc().dirty(),
        snapshot["dirty"].as_bool().unwrap(),
        "{action}: dirty"
    );
    assert_eq!(
        app.doc().cursor,
        snapshot["primary"]["cursor"].as_u64().unwrap() as usize,
        "{action}: cursor"
    );
    assert_eq!(
        app.doc().anchor.unwrap_or(app.doc().cursor),
        snapshot["primary"]["anchor"].as_u64().unwrap() as usize,
        "{action}: anchor"
    );
    assert_eq!(
        fs::read(file).unwrap(),
        snapshot["disk"].as_str().unwrap().as_bytes(),
        "{action}: exact disk"
    );
    assert!(app.extension_host.is_none());
}
fn run(name: &str) {
    assert!(IMPLEMENTED.contains(&name));
    let row = corpus().into_iter().find(|r| r["name"] == name).unwrap();
    let fixture = fixtures().into_iter().find(|r| r["name"] == name).unwrap();
    let observations = row["observations"].as_array().unwrap();
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.txt");
    fs::write(&file, observations[0]["text"].as_str().unwrap()).unwrap();
    let script = root.path().join("source-actions.py");
    fs::write(&script, PEER).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.extension_node = root
        .path()
        .join("node-unavailable")
        .to_string_lossy()
        .into_owned();
    app.settings = settings(root.path(), &fixture);
    app.open(&file).unwrap();
    let id = app.doc().id;
    let end = app.doc().len();
    app.doc_mut().move_to(end, false);
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                script.to_string_lossy().into_owned(),
                root.path().to_string_lossy().into_owned(),
            ],
            root.path(),
            "plaintext".into(),
        )
        .unwrap(),
    );
    until(&mut app, "actual native didOpen", |app| {
        app.lsp.as_ref().is_some_and(|c| c.ready)
            && protocol(root.path()).iter().any(|e| e["kind"] == "open")
    });
    compare(&app, &file, id, &observations[0]);
    app.execute("type", json!({"text":"λ🙂"}));
    compare(&app, &file, id, &observations[1]);
    until(&mut app, "typed sync", |_| {
        protocol(root.path())
            .iter()
            .any(|e| e["kind"] == "change" && e["text"] == observations[1]["text"])
    });
    if fixture["autosave"] != "afterDelay" {
        app.execute("workbench.action.files.save", Value::Null);
    }
    until(
        &mut app,
        "native saved receipt and committed didSave",
        |app| {
            app.doc()
                .capture_save(file.clone())
                .unwrap()
                .save_generation()
                == 1
                && !app.saves_pending()
                && protocol(root.path())
                    .iter()
                    .filter(|e| e["kind"] == "save")
                    .count()
                    == 1
        },
    );
    compare(&app, &file, id, &observations[2]);
    for snapshot in observations.iter().skip(3) {
        let command = snapshot["action"].as_str().unwrap();
        assert!(matches!(command, "undo" | "redo"));
        app.execute(command, Value::Null);
        compare(&app, &file, id, snapshot);
    }
    let events = protocol(root.path());
    let actual: Vec<_> = events
        .iter()
        .filter(|e| e["kind"] == "actions")
        .map(|e| json!({"only":e["only"],"triggerKind":e["triggerKind"],"text":e["text"]}))
        .collect();
    let expected: Vec<_> = row["callbacks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| json!({"only":e["only"],"triggerKind":e["triggerKind"],"text":e["text"]}))
        .collect();
    if name == "child-only-dot-boundary" {
        // Native discovery projects child-only settings onto its one family
        // root query, then filters returned kinds locally. Keep the observed
        // upstream child query intact and qualify this difference explicitly.
        assert_eq!(actual.len(), 1);
        assert_eq!(expected.len(), 1);
        assert_eq!(actual[0]["only"], "source.fixAll");
        assert_eq!(expected[0]["only"], "source.fixAll.child");
        assert_eq!(actual[0]["text"], expected[0]["text"]);
        assert_eq!(actual[0]["triggerKind"], expected[0]["triggerKind"]);
    } else {
        assert_eq!(
            actual, expected,
            "{name}: family ordering and fresh callback text"
        );
    }
    let saved: Vec<_> = events.iter().filter(|e| e["kind"] == "save").collect();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["text"], observations[2]["text"]);
    assert_eq!(
        app.doc()
            .capture_save(file.clone())
            .unwrap()
            .save_generation(),
        1
    );
    assert!(app.extension_host.is_none());
}

#[test]
fn native_single_action_family_workflows_match_fifty_two_snapshots() {
    for name in &IMPLEMENTED[..4] {
        run(name);
    }
}
#[test]
fn native_after_delay_workflows_match_six_snapshots_without_actions() {
    for name in &IMPLEMENTED[4..] {
        run(name);
    }
}
