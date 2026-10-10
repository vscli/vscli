//! Scoped native consumer of the pinned save-formatting reference.
//!
//! Public App commands + actual framed formatter are compared for text, disk,
//! dirty state, primary selection, Undo/Redo and formatter invocation. Native
//! model versions, UI state and future source-actions-on-save are not claimed.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile, lsp::Client, settings::Settings};

const ROOT: &str = "tests/vscode-reference";
const BASELINE: &str = "tests/vscode-reference/baselines/1.95.0/save-formatting";
const COMMIT: &str = "912bb683695358a54ae0c670461738984cbb5b95";
const SERVER: &str = r#"
import json, pathlib, sys
root, mode = pathlib.Path(sys.argv[1]), sys.argv[2]
text = ''
count = 0

def record(kind, **fields):
    global count
    count += 1
    assert count <= 128, 'Native reference fixture trace budget exceeded'
    with (root / 'protocol.jsonl').open('a', encoding='utf-8') as stream:
        stream.write(json.dumps({'kind':kind, **fields}, ensure_ascii=False) + '\n')

def send(message):
    body = json.dumps({'jsonrpc':'2.0', **message}, ensure_ascii=False).encode()
    assert len(body) <= 16384
    sys.stdout.buffer.write(('Content-Length: %s\r\n\r\n' % len(body)).encode() + body)
    sys.stdout.buffer.flush()

while True:
    headers = {}
    size = 0
    while True:
        line = sys.stdin.buffer.readline(8193)
        if not line: sys.exit(0)
        size += len(line)
        assert size <= 8192
        if line == b'\r\n': break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    length = int(headers['content-length'])
    assert 0 <= length <= 65536
    body = sys.stdin.buffer.read(length)
    assert len(body) == length
    message = json.loads(body)
    method, ident, params = message.get('method'), message.get('id'), message.get('params', {})
    if method == 'initialize':
        send({'id':ident, 'result':{'capabilities':{'documentFormattingProvider':True,
            'textDocumentSync':{'openClose':True,'change':1,'save':{'includeText':True}}}}})
    elif method == 'textDocument/didOpen':
        text = params['textDocument']['text']
        record('open', text=text)
    elif method == 'textDocument/didChange':
        text = params['contentChanges'][-1]['text']
        record('change', text=text)
    elif method == 'textDocument/formatting':
        assert set(params) == {'textDocument','options'}
        record('format', text=text, result=mode)
        line = text.splitlines()[1]
        assert line == 'let value=1;'
        result = None if mode == 'null' else [{'range':{'start':{'line':1,'character':0},
            'end':{'line':1,'character':len(line.encode('utf-16-le'))//2}},'newText':'let value = 1;'}]
        send({'id':ident, 'result':result})
    elif method == 'textDocument/didSave':
        assert 'text' in params, 'Negotiated committed save text absent'
        record('save', text=params['text'])
    elif method == 'shutdown': send({'id':ident,'result':None})
    elif method == 'exit': break
"#;

fn artifact(relative: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
}
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn corpus() -> Vec<Value> {
    // Fresh captures are qualified only for the data and platform in their
    // provenance; an override never falls back to the frozen Linux baseline.
    let (directory, stem) = match std::env::var_os("VSCLI_SAVE_FORMATTING_REFERENCE_DIR") {
        Some(directory) => (PathBuf::from(directory), "save-formatting"),
        None => (
            Path::new(env!("CARGO_MANIFEST_DIR")).join(BASELINE),
            "linux",
        ),
    };
    let projection = fs::read(directory.join(format!("{stem}.json"))).unwrap();
    let raw = fs::read(directory.join(format!("{stem}-evidence.json"))).unwrap();
    let provenance: Value = serde_json::from_slice(
        &fs::read(directory.join(format!("{stem}-provenance.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["version"], "1.95.0");
    assert_eq!(provenance["commit"], COMMIT);
    assert_eq!(provenance["caseCount"], 4);
    assert_eq!(provenance["snapshotCount"], 18);
    assert_eq!(provenance["traceSha256"], sha256(&projection));
    assert_eq!(provenance["evidenceSha256"], sha256(&raw));
    assert_eq!(
        provenance["casesSha256"],
        sha256(&artifact(&format!("{ROOT}/save-formatting-cases.json")))
    );
    let evidence: Vec<Value> = serde_json::from_slice(&raw).unwrap();
    let rows: Vec<Value> = serde_json::from_slice(&projection).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(evidence.len(), rows.len());
    let runs = provenance["runs"].as_array().unwrap();
    assert_eq!(runs.len(), rows.len());
    for ((row, raw), run) in rows.iter().zip(&evidence).zip(runs) {
        assert_eq!(row["name"], raw["name"]);
        assert_eq!(row["name"], run["name"]);
        assert_eq!(row["observations"], raw["observations"]);
        assert_eq!(run["commit"], COMMIT);
        assert_eq!(raw["setup"]["before"], raw["setup"]["after"]);
        let source_hashes = run["sources"].as_object().unwrap();
        assert_eq!(source_hashes.len(), 9);
        for (name, expected) in source_hashes {
            assert!(Path::new(name).components().count() == 1);
            assert_eq!(
                expected,
                &json!(sha256(&artifact(&format!("{ROOT}/{name}"))))
            );
        }
        let callbacks: Vec<_> = raw["callbacks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|callback| callback["phase"] == "target")
            .map(|callback| {
                let mut result = json!({"kind":callback["kind"],"text":callback["text"]});
                if let Some(value) = callback.get("result") {
                    result["result"] = value.clone();
                }
                result
            })
            .collect();
        assert_eq!(row["callbacks"], json!(callbacks));
    }
    assert_eq!(
        rows.iter()
            .map(|row| row["observations"].as_array().unwrap().len())
            .sum::<usize>(),
        18
    );
    let implemented: Vec<_> = rows
        .iter()
        .filter(|row| {
            matches!(
                row["name"].as_str(),
                Some(
                    "explicit-file-unicode-crlf"
                        | "explicit-null-no-extra-undo"
                        | "after-delay-skips-formatter"
                )
            )
        })
        .collect();
    assert_eq!(implemented.len(), 3);
    assert_eq!(
        implemented
            .iter()
            .map(|row| row["observations"].as_array().unwrap().len())
            .sum::<usize>(),
        13
    );
    let future = rows
        .iter()
        .find(|row| row["name"] == "source-action-before-file-format")
        .unwrap();
    assert_eq!(future["callbacks"][0]["kind"], "sourceAction");
    assert_eq!(future["callbacks"][1]["kind"], "format");
    rows
}
fn protocol(root: &Path) -> Vec<Value> {
    let bytes = fs::read(root.join("protocol.jsonl")).unwrap_or_default();
    assert!(bytes.len() <= 128 * 16 * 1024);
    bytes
        .split_inclusive(|byte| *byte == b'\n')
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
fn compare(app: &App, source: &Path, id: u64, snapshot: &Value) {
    let action = snapshot["action"].as_str().unwrap();
    assert_eq!(app.doc().id, id, "{action}: original model identity");
    assert_eq!(
        app.doc().text.to_string(),
        snapshot["text"].as_str().unwrap(),
        "{action}"
    );
    assert_eq!(
        app.doc().dirty(),
        snapshot["dirty"].as_bool().unwrap(),
        "{action}: dirty"
    );
    assert_eq!(
        app.doc().cursor,
        snapshot["primary"]["cursor"].as_u64().unwrap() as usize,
        "{action}: primary cursor"
    );
    assert_eq!(
        app.doc().anchor.unwrap_or(app.doc().cursor),
        snapshot["primary"]["anchor"].as_u64().unwrap() as usize,
        "{action}: primary anchor"
    );
    assert_eq!(
        fs::read(source).unwrap(),
        snapshot["disk"].as_str().unwrap().as_bytes(),
        "{action}: exact committed Unicode/CRLF bytes"
    );
    assert!(
        app.extension_host.is_none(),
        "Native comparison started Node"
    );
}
fn run(name: &str) {
    let row = corpus()
        .into_iter()
        .find(|row| row["name"] == name)
        .unwrap();
    let observations = row["observations"].as_array().unwrap();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("main.txt");
    fs::write(&source, observations[0]["text"].as_str().unwrap()).unwrap();
    let script = root.path().join("formatter.py");
    fs::write(&script, SERVER).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.extension_node = root
        .path()
        .join("node-unavailable")
        .to_string_lossy()
        .into_owned();
    let settings = root.path().join("consumer-settings.json");
    fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,
            "editor.quickSuggestions":false,"editor.parameterHints.enabled":false,
            "editor.formatOnSave":true,"editor.formatOnSaveMode":"file",
            "editor.tabSize":2,"editor.insertSpaces":true,
            "files.autoSave":if name=="after-delay-skips-formatter" {"afterDelay"}else{"off"},
            "files.autoSaveDelay":500
        }))
        .unwrap(),
    )
    .unwrap();
    app.settings = Settings::load(&[settings]).unwrap();
    app.open(&source).unwrap();
    let id = app.doc().id;
    let end = app.doc().text.len_chars();
    app.doc_mut().move_to(end, false);
    app.lsp = Some(
        Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                script.to_string_lossy().into_owned(),
                root.path().to_string_lossy().into_owned(),
                if name == "explicit-null-no-extra-undo" {
                    "null".into()
                } else {
                    "edit".into()
                },
            ],
            root.path(),
            "plaintext".into(),
        )
        .unwrap(),
    );
    until(&mut app, "actual native didOpen", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
            && protocol(root.path())
                .iter()
                .any(|event| event["kind"] == "open")
    });
    compare(&app, &source, id, &observations[0]);
    app.execute("type", json!({"text":"λ🙂"}));
    compare(&app, &source, id, &observations[1]);
    until(&mut app, "actual synchronized typed input", |_| {
        protocol(root.path())
            .iter()
            .any(|event| event["kind"] == "change" && event["text"] == observations[1]["text"])
    });
    if name != "after-delay-skips-formatter" {
        app.execute("workbench.action.files.save", Value::Null);
    }
    until(
        &mut app,
        "actual save receipt and committed didSave",
        |app| {
            app.doc()
                .capture_save(source.clone())
                .unwrap()
                .save_generation()
                == 1
                && !app.saves_pending()
                && protocol(root.path())
                    .iter()
                    .filter(|event| event["kind"] == "save")
                    .count()
                    == 1
        },
    );
    compare(&app, &source, id, &observations[2]);
    for snapshot in observations.iter().skip(3) {
        let command = snapshot["action"].as_str().unwrap();
        assert!(matches!(command, "undo" | "redo"));
        app.execute(command, Value::Null);
        compare(&app, &source, id, snapshot);
    }
    let events = protocol(root.path());
    let actual: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "format")
        .map(|event| json!({"kind":event["kind"],"text":event["text"],"result":event["result"]}))
        .collect();
    assert_eq!(
        json!(actual),
        row["callbacks"],
        "{name}: exact formatter target inputs/invocations"
    );
    let save = events.iter().find(|event| event["kind"] == "save").unwrap();
    assert_eq!(
        save["text"], observations[2]["disk"],
        "{name}: didSave uses committed snapshot"
    );
    assert!(!app.saves_pending());
}

#[test]
fn native_explicit_file_matches_pinned_formatting_save_and_separate_undo_redo() {
    run("explicit-file-unicode-crlf");
}
#[test]
fn native_null_formatter_matches_pinned_save_without_an_extra_undo_step() {
    run("explicit-null-no-extra-undo");
}
#[test]
fn native_after_delay_matches_pinned_committed_bytes_and_zero_formatter_invocations() {
    run("after-delay-skips-formatter");
}
