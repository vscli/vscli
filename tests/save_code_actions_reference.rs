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
const LEGACY_SAVE_OBSERVER: &str =
    "d8288ec09b2c896edcb760c218da108be430ce29433e0e19523d360d792c8641";
const READY_SAVE_OBSERVER: &str =
    "9ff83ee188612b4bdcdc74d59e8261de658eb58b8a9815bb12fe1fb91c80cf47";
const AUX_INPUT: &str = "readiness=0\r\n";
const OBSERVED_SCOPE: &str = "Separate public auxiliary Save and exact null-formatter callback establish save-contribution installation; no target output predicate or target command retry";
const SKIPPED_SCOPE: &str =
    "After-delay target retains original zero-action reason scope; no auxiliary explicit Save";

#[derive(serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AuxiliaryState {
    resource: String,
    uri: String,
    version: u64,
    text: String,
    dirty: bool,
    disk: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AuxiliaryAttempt {
    index: usize,
    before: AuxiliaryState,
    prepared: AuxiliaryState,
    after: AuxiliaryState,
    callback_start: usize,
    callback_count: usize,
    matched: bool,
    elapsed_ms: u64,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AuxiliaryCallback {
    attempt: Option<usize>,
    uri: String,
    version: u64,
    text: Option<String>,
    resource: Option<String>,
    exact_document: bool,
    during_save: bool,
    cancelled: bool,
    matched: bool,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AuxiliaryLimits {
    max_attempts: usize,
    deadline_ms: u64,
    retry_interval_ms: u64,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AuxiliaryConfiguration {
    format_on_save: bool,
    format_on_save_mode: String,
    auto_save: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedReadiness {
    status: String,
    resource: String,
    input: String,
    effective: AuxiliaryConfiguration,
    limits: AuxiliaryLimits,
    initial: AuxiliaryState,
    #[serde(rename = "final")]
    final_state: AuxiliaryState,
    attempts: Vec<AuxiliaryAttempt>,
    callbacks: Vec<AuxiliaryCallback>,
    scope: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SkippedReadiness {
    status: String,
    resource: String,
    input: String,
    disk: String,
    attempts: Vec<Value>,
    callbacks: Vec<Value>,
    scope: String,
}
fn auxiliary_keys(value: &Value, keys: &[&str]) -> anyhow::Result<()> {
    let Some(object) = value.as_object() else {
        anyhow::bail!("Invalid auxiliary metadata shape");
    };
    anyhow::ensure!(
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)),
        "Invalid auxiliary metadata shape"
    );
    Ok(())
}
fn auxiliary_budget(value: &Value) -> anyhow::Result<()> {
    fn visit(
        value: &Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
    ) -> anyhow::Result<()> {
        *nodes += 1;
        anyhow::ensure!(
            depth <= 8 && *nodes <= 1024,
            "Auxiliary metadata exceeds bounds"
        );
        match value {
            Value::String(text) => *bytes = bytes.saturating_add(text.len()),
            Value::Array(array) => {
                for child in array {
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
            Value::Object(object) => {
                for (key, child) in object {
                    *bytes = bytes.saturating_add(key.len());
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
            _ => *bytes = bytes.saturating_add(32),
        }
        anyhow::ensure!(*bytes <= 512 * 1024, "Auxiliary metadata exceeds bounds");
        Ok(())
    }
    let mut nodes = 0;
    let mut bytes = 0;
    visit(value, 0, &mut nodes, &mut bytes)
}
fn auxiliary_state(state: &AuxiliaryState, uri: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.resource == "readiness.ini"
            && state.uri == uri
            && (1..=i32::MAX as u64).contains(&state.version)
            && state.text.len() <= AUX_INPUT.len() + 16
            && state.disk.len() <= AUX_INPUT.len() + 16,
        "Invalid auxiliary state identity or bounds"
    );
    Ok(())
}
fn auxiliary_callback(
    callback: &AuxiliaryCallback,
    uri: &str,
    attempt: Option<usize>,
    state: &AuxiliaryState,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        callback.uri == uri
            && callback.resource.as_deref() == Some("readiness.ini")
            && callback.exact_document
            && (1..=i32::MAX as u64).contains(&callback.version)
            && callback
                .text
                .as_ref()
                .is_some_and(|text| text.len() <= AUX_INPUT.len() + 16),
        "Unexpected auxiliary callback resource or bounds"
    );
    let callback_text = callback.text.as_deref().expect("callback text validated");
    let suffix = callback_text
        .strip_prefix(AUX_INPUT)
        .ok_or_else(|| anyhow::anyhow!("Auxiliary callback bytes changed"))?;
    let current_suffix = state
        .text
        .strip_prefix(AUX_INPUT)
        .ok_or_else(|| anyhow::anyhow!("Auxiliary callback bytes changed"))?;
    let first_version = state
        .version
        .checked_sub(current_suffix.len() as u64)
        .ok_or_else(|| anyhow::anyhow!("Auxiliary callback version changed"))?;
    anyhow::ensure!(
        suffix.bytes().all(|byte| byte == b'x') && suffix.len() <= 16,
        "Auxiliary callback bytes changed"
    );
    anyhow::ensure!(
        callback.version == first_version + suffix.len() as u64
            && callback.version <= state.version,
        "Auxiliary callback version changed"
    );
    anyhow::ensure!(
        callback.attempt == attempt && callback.during_save == attempt.is_some(),
        "Auxiliary callback lifetime changed"
    );
    let matched = attempt.is_some()
        && callback.version == state.version
        && callback.text.as_deref() == Some(state.text.as_str())
        && !callback.cancelled;
    anyhow::ensure!(
        callback.matched == matched,
        "Auxiliary callback match proof changed"
    );
    Ok(matched)
}
/// Pure preflight: it never writes fixture bytes or samples the filesystem.
/// Legacy and candidate source versions remain explicit; absence is not a
/// permissive fallback once the candidate's observer hash has been captured.
fn validate_auxiliary_readiness(raw: &Value, observer: &str) -> anyhow::Result<()> {
    if matches!(observer, LEGACY_SAVE_OBSERVER | READY_SAVE_OBSERVER) {
        anyhow::ensure!(
            raw.get("configurationDiagnostics").is_none(),
            "Unexpected configuration diagnostics in earlier observation"
        );
    }
    if observer == LEGACY_SAVE_OBSERVER {
        anyhow::ensure!(
            raw["setup"].get("saveParticipantReadiness").is_none(),
            "Unexpected auxiliary readiness in legacy observation"
        );
        return Ok(());
    }
    anyhow::ensure!(
        observer == READY_SAVE_OBSERVER,
        "Unsupported save-participant observer contract"
    );
    validate_auxiliary_protocol(raw)
}
fn validate_auxiliary_protocol(raw: &Value) -> anyhow::Result<()> {
    let readiness = raw["setup"].get("saveParticipantReadiness");
    let readiness =
        readiness.ok_or_else(|| anyhow::anyhow!("Missing candidate auxiliary readiness"))?;
    auxiliary_budget(readiness)?;
    let autosave = raw["setup"]["effective"]["autoSave"].as_str();
    if autosave == Some("afterDelay") {
        auxiliary_keys(
            readiness,
            &[
                "status",
                "resource",
                "input",
                "disk",
                "attempts",
                "callbacks",
                "scope",
            ],
        )?;
        let proof: SkippedReadiness = serde_json::from_value(readiness.clone())
            .map_err(|_| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
        anyhow::ensure!(
            proof.status == "not-run-after-delay"
                && proof.resource == "readiness.ini"
                && proof.input == AUX_INPUT
                && proof.disk == AUX_INPUT
                && proof.attempts.is_empty()
                && proof.callbacks.is_empty()
                && proof.scope == SKIPPED_SCOPE,
            "Invalid after-delay auxiliary scope"
        );
        return Ok(());
    }
    anyhow::ensure!(
        autosave == Some("off"),
        "Invalid auxiliary target save reason"
    );
    auxiliary_keys(
        readiness,
        &[
            "status",
            "resource",
            "input",
            "effective",
            "limits",
            "initial",
            "final",
            "attempts",
            "callbacks",
            "scope",
        ],
    )?;
    auxiliary_keys(
        &readiness["effective"],
        &["formatOnSave", "formatOnSaveMode", "autoSave"],
    )?;
    auxiliary_keys(
        &readiness["limits"],
        &["maxAttempts", "deadlineMs", "retryIntervalMs"],
    )?;
    let states = &["resource", "uri", "version", "text", "dirty", "disk"];
    auxiliary_keys(&readiness["initial"], states)?;
    auxiliary_keys(&readiness["final"], states)?;
    let attempts = readiness["attempts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
    let callbacks = readiness["callbacks"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
    anyhow::ensure!(
        (1..=16).contains(&attempts.len()) && callbacks.len() <= 16,
        "Auxiliary metadata exceeds bounds"
    );
    for attempt in attempts {
        auxiliary_keys(
            attempt,
            &[
                "index",
                "before",
                "prepared",
                "after",
                "callbackStart",
                "callbackCount",
                "matched",
                "elapsedMs",
            ],
        )?;
        for name in ["before", "prepared", "after"] {
            auxiliary_keys(&attempt[name], states)?;
        }
    }
    for callback in callbacks {
        auxiliary_keys(
            callback,
            &[
                "attempt",
                "uri",
                "version",
                "text",
                "resource",
                "exactDocument",
                "duringSave",
                "cancelled",
                "matched",
            ],
        )?;
    }
    let proof: ObservedReadiness = serde_json::from_value(readiness.clone())
        .map_err(|_| anyhow::anyhow!("Invalid auxiliary metadata shape"))?;
    anyhow::ensure!(
        proof.status == "observed"
            && proof.resource == "readiness.ini"
            && proof.input == AUX_INPUT
            && proof.scope == OBSERVED_SCOPE
            && proof.effective.format_on_save
            && proof.effective.format_on_save_mode == "file"
            && proof.effective.auto_save == "off",
        "Invalid auxiliary configuration or scope"
    );
    anyhow::ensure!(
        proof.limits.max_attempts == 16
            && proof.limits.deadline_ms == 5000
            && proof.limits.retry_interval_ms == 250,
        "Auxiliary readiness limits changed"
    );
    let uri = &proof.initial.uri;
    anyhow::ensure!(
        uri.len() <= 4096
            && !uri.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
            && !uri.contains('\\'),
        "Invalid auxiliary URI"
    );
    let parsed = url::Url::parse(uri).map_err(|_| anyhow::anyhow!("Invalid auxiliary URI"))?;
    anyhow::ensure!(
        parsed.scheme() == "file"
            && parsed.host_str().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.path().starts_with('/')
            && parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                == Some("readiness.ini"),
        "Invalid auxiliary URI"
    );
    auxiliary_state(&proof.initial, uri)?;
    anyhow::ensure!(
        proof.initial.text == AUX_INPUT && proof.initial.disk == AUX_INPUT && !proof.initial.dirty,
        "Auxiliary initial bytes changed"
    );
    let mut previous = &proof.initial;
    let mut cursor = 0;
    let mut elapsed = 0;
    for (offset, attempt) in proof.attempts.iter().enumerate() {
        anyhow::ensure!(
            attempt.index == offset + 1 && attempt.before == *previous,
            "Auxiliary attempt continuity changed"
        );
        for state in [&attempt.before, &attempt.prepared, &attempt.after] {
            auxiliary_state(state, uri)?;
        }
        anyhow::ensure!(
            attempt.prepared.version == attempt.before.version.checked_add(1).unwrap_or(0)
                && attempt.prepared.text == format!("{}x", attempt.before.text)
                && attempt.prepared.disk == attempt.before.disk
                && attempt.prepared.dirty,
            "Auxiliary prepared edit changed"
        );
        anyhow::ensure!(
            attempt.after.version == attempt.prepared.version
                && attempt.after.text == attempt.prepared.text
                && attempt.after.disk == attempt.after.text
                && !attempt.after.dirty,
            "Auxiliary null-formatter receipt changed"
        );
        anyhow::ensure!(
            attempt.elapsed_ms >= elapsed && attempt.elapsed_ms < 5000,
            "Auxiliary readiness deadline changed"
        );
        elapsed = attempt.elapsed_ms;
        let end = attempt
            .callback_start
            .checked_add(attempt.callback_count)
            .ok_or_else(|| anyhow::anyhow!("Auxiliary callback slices changed"))?;
        anyhow::ensure!(
            attempt.callback_start >= cursor && end <= proof.callbacks.len(),
            "Auxiliary callback slices changed"
        );
        // Outside-Save callbacks are retained and checked; they cannot authorize
        // readiness and are never silently filtered from the evidence.
        for callback in &proof.callbacks[cursor..attempt.callback_start] {
            auxiliary_callback(callback, uri, None, &attempt.before)?;
        }
        let mut matched = 0;
        for callback in &proof.callbacks[attempt.callback_start..end] {
            matched += usize::from(auxiliary_callback(
                callback,
                uri,
                Some(attempt.index),
                &attempt.prepared,
            )?);
        }
        let final_attempt = offset + 1 == proof.attempts.len();
        anyhow::ensure!(
            matched == usize::from(final_attempt) && attempt.matched == final_attempt,
            "Auxiliary final positive witness changed"
        );
        cursor = end;
        previous = &attempt.after;
    }
    for callback in &proof.callbacks[cursor..] {
        auxiliary_callback(callback, uri, None, previous)?;
    }
    anyhow::ensure!(
        proof.final_state == *previous,
        "Auxiliary final snapshot changed"
    );
    Ok(())
}

const DIAGNOSTIC_SAVE_OBSERVER: &str =
    "21f5ae5c7713c9053da9bbdfa660f19f59fc1b3045e6415a49ddc2d7970d52b2";
const DIAGNOSTIC_SCOPE: &str = "Public target resource/language configuration at source callbacks, didSave and all relevant configuration events; independent diagnostic metadata, no migration forcing, preferred target predicate or target retry";
const DIAGNOSTIC_KEYS: &[&str] = &[
    "codeActionsOnSave",
    "formatOnSave",
    "autoSave",
    "autoSaveDelay",
];
const DIAGNOSTIC_EVENTS: &[&str] = &[
    "editor.codeActionsOnSave",
    "editor.formatOnSave",
    "files.autoSave",
    "files.autoSaveDelay",
];

fn diagnostic_shape(value: &Value, keys: &[&str]) -> anyhow::Result<()> {
    auxiliary_keys(value, keys).map_err(|_| anyhow::anyhow!("Invalid diagnostic metadata shape"))
}
/// Count compact JSON bytes directly, without copying or serializing the object.
/// All numbers in the frozen protocol/configuration cohort are integers.
fn diagnostic_json_budget(value: &Value) -> anyhow::Result<()> {
    fn add(total: &mut usize, size: usize) -> anyhow::Result<()> {
        *total = total
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("Diagnostic metadata exceeds bounds"))?;
        anyhow::ensure!(*total <= 524288, "Diagnostic metadata exceeds bounds");
        Ok(())
    }
    fn quoted(text: &str, total: &mut usize) -> anyhow::Result<()> {
        add(total, 2)?;
        for byte in text.bytes() {
            add(
                total,
                match byte {
                    b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2,
                    0..=31 => 6,
                    _ => 1,
                },
            )?;
        }
        Ok(())
    }
    fn visit(
        value: &Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
    ) -> anyhow::Result<()> {
        *nodes += 1;
        anyhow::ensure!(
            depth <= 16 && *nodes <= 400000,
            "Diagnostic metadata exceeds bounds"
        );
        match value {
            Value::Null => add(bytes, 4)?,
            Value::Bool(boolean) => add(bytes, if *boolean { 4 } else { 5 })?,
            Value::String(text) => quoted(text, bytes)?,
            Value::Number(number) => {
                anyhow::ensure!(
                    number.is_i64() || number.is_u64(),
                    "Invalid diagnostic number"
                );
                // One bounded scalar conversion, never whole-object serialization.
                add(bytes, number.to_string().len())?;
            }
            Value::Array(array) => {
                add(bytes, 2)?;
                add(bytes, array.len().saturating_sub(1))?;
                for child in array {
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
            Value::Object(object) => {
                add(bytes, 2)?;
                add(bytes, object.len().saturating_sub(1))?;
                for (key, child) in object {
                    quoted(key, bytes)?;
                    add(bytes, 1)?;
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
        }
        Ok(())
    }
    let mut nodes = 0;
    let mut bytes = 0;
    visit(value, 0, &mut nodes, &mut bytes)
}
/// Producer bounds are per copied effective/inspect/context value, not global.
fn diagnostic_value_budget(value: &Value) -> anyhow::Result<()> {
    fn visit(
        value: &Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
    ) -> anyhow::Result<()> {
        *nodes += 1;
        anyhow::ensure!(
            depth <= 8 && *nodes <= 1024,
            "Diagnostic value exceeds bounds"
        );
        match value {
            Value::Null | Value::Bool(_) => *bytes += 8,
            Value::Number(_) => *bytes += 32,
            Value::String(text) => *bytes = bytes.saturating_add(text.len()),
            Value::Array(array) => {
                for child in array {
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
            Value::Object(object) => {
                for (key, child) in object {
                    *bytes = bytes.saturating_add(key.len());
                    visit(child, depth + 1, nodes, bytes)?;
                }
            }
        }
        anyhow::ensure!(*bytes <= 32768, "Diagnostic value exceeds bounds");
        Ok(())
    }
    let mut nodes = 0;
    let mut bytes = 0;
    visit(value, 0, &mut nodes, &mut bytes)
}
fn diagnostic_setting(value: &Value, key: &str) -> anyhow::Result<()> {
    let valid = match key {
        "codeActionsOnSave" => match value {
            Value::Array(array) => array.iter().all(|kind| {
                kind.as_str()
                    .is_some_and(|kind| !kind.is_empty() && kind.len() <= 512)
            }),
            Value::Object(object) => object.iter().all(|(kind, setting)| {
                !kind.is_empty()
                    && kind.len() <= 512
                    && (setting.is_boolean()
                        || matches!(setting.as_str(), Some("explicit" | "always" | "never")))
            }),
            _ => false,
        },
        "formatOnSave" => value.is_boolean(),
        "autoSave" => matches!(
            value.as_str(),
            Some("off" | "afterDelay" | "onFocusChange" | "onWindowChange")
        ),
        "autoSaveDelay" => value.as_u64().is_some_and(|delay| delay <= i32::MAX as u64),
        _ => false,
    };
    anyhow::ensure!(valid, "Invalid diagnostic configuration value");
    Ok(())
}
fn diagnostic_snapshot(value: &Value, uri: &str) -> anyhow::Result<Option<u64>> {
    diagnostic_shape(
        value,
        &[
            "resource",
            "uri",
            "languageId",
            "open",
            "version",
            "exactDocument",
            "effective",
            "inspect",
        ],
    )?;
    anyhow::ensure!(
        value["resource"] == "main.txt"
            && value["uri"] == uri
            && value["languageId"] == "plaintext",
        "Diagnostic resource identity changed"
    );
    let version = match value["open"].as_bool() {
        Some(false) => {
            anyhow::ensure!(
                value["version"].is_null() && value["exactDocument"].is_null(),
                "Diagnostic document identity changed"
            );
            None
        }
        Some(true) => {
            let version = value["version"]
                .as_u64()
                .filter(|version| (1..=i32::MAX as u64).contains(version));
            anyhow::ensure!(
                version.is_some() && value["exactDocument"] == true,
                "Diagnostic document identity changed"
            );
            version
        }
        None => anyhow::bail!("Diagnostic document identity changed"),
    };
    for map in ["effective", "inspect"] {
        diagnostic_value_budget(&value[map])?;
        diagnostic_shape(&value[map], DIAGNOSTIC_KEYS)?;
    }
    for key in DIAGNOSTIC_KEYS {
        diagnostic_setting(&value["effective"][*key], key)?;
        let inspected = value["inspect"][*key]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Invalid diagnostic inspection"))?;
        let prefix = if key.starts_with("autoSave") {
            "files"
        } else {
            "editor"
        };
        anyhow::ensure!(
            inspected.get("key").and_then(Value::as_str)
                == Some(format!("{prefix}.{key}").as_str()),
            "Diagnostic inspection accessor changed"
        );
        for (field, item) in inspected {
            match field.as_str() {
                "key" => (),
                "languageIds" => {
                    let ids = item
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("Invalid diagnostic inspection"))?;
                    let mut unique = std::collections::BTreeSet::new();
                    anyhow::ensure!(
                        ids.len() <= 64
                            && ids
                                .iter()
                                .all(|id| id.as_str().is_some_and(|id| !id.is_empty()
                                    && id.len() <= 128
                                    && unique.insert(id))),
                        "Invalid diagnostic inspection"
                    );
                }
                "defaultValue"
                | "globalValue"
                | "workspaceValue"
                | "workspaceFolderValue"
                | "defaultLanguageValue"
                | "globalLanguageValue"
                | "workspaceLanguageValue"
                | "workspaceFolderLanguageValue" => diagnostic_setting(item, key)?,
                _ => anyhow::bail!("Invalid diagnostic inspection"),
            }
        }
    }
    Ok(version)
}
fn validate_configuration_diagnostics(raw: &Value) -> anyhow::Result<()> {
    let proof = raw
        .get("configurationDiagnostics")
        .ok_or_else(|| anyhow::anyhow!("Missing configuration diagnostics"))?;
    diagnostic_json_budget(proof)?;
    diagnostic_shape(
        proof,
        &["protocol", "limits", "initial", "final", "records", "scope"],
    )?;
    anyhow::ensure!(
        proof["protocol"] == "save-configuration-diagnostics-v1"
            && proof["scope"] == DIAGNOSTIC_SCOPE,
        "Diagnostic protocol changed"
    );
    diagnostic_shape(
        &proof["limits"],
        &[
            "maxRecords",
            "maxValueNodes",
            "maxValueDepth",
            "maxValueBytes",
            "maxTotalBytes",
        ],
    )?;
    anyhow::ensure!(
        proof["limits"]
            == json!({"maxRecords":128,"maxValueNodes":1024,"maxValueDepth":8,"maxValueBytes":32768,"maxTotalBytes":524288}),
        "Diagnostic limits changed"
    );
    let uri = proof["initial"]["uri"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid diagnostic URI"))?;
    anyhow::ensure!(
        uri.len() <= 4096
            && !uri.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
            && !uri.contains('\\'),
        "Invalid diagnostic URI"
    );
    let parsed = url::Url::parse(uri).map_err(|_| anyhow::anyhow!("Invalid diagnostic URI"))?;
    anyhow::ensure!(
        parsed.scheme() == "file"
            && parsed.host_str().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.path().starts_with('/')
            && parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                == Some("main.txt"),
        "Invalid diagnostic URI"
    );
    let name = raw["name"]
        .as_str()
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        })
        .ok_or_else(|| anyhow::anyhow!("Diagnostic resource identity changed"))?;
    anyhow::ensure!(
        parsed.path().ends_with(&format!("/{name}/w/main.txt")),
        "Diagnostic resource identity changed"
    );
    anyhow::ensure!(
        diagnostic_snapshot(&proof["initial"], uri)?.is_none(),
        "Diagnostic initial document changed"
    );
    let final_version = diagnostic_snapshot(&proof["final"], uri)?
        .ok_or_else(|| anyhow::anyhow!("Diagnostic final document changed"))?;
    let records = proof["records"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid diagnostic metadata shape"))?;
    anyhow::ensure!(records.len() <= 128, "Diagnostic records exceed bounds");
    let callbacks = raw["callbacks"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Diagnostic callback correlation changed"))?;
    let saved = raw["saved"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Diagnostic save correlation changed"))?;
    let changes = raw["changes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Diagnostic final version correlation changed"))?;
    let mut final_public_version = raw["setup"]["after"]["version"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Diagnostic final version correlation changed"))?;
    for event in callbacks.iter().chain(saved).chain(changes) {
        final_public_version = final_public_version.max(
            event["version"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Diagnostic final version correlation changed"))?,
        );
    }
    anyhow::ensure!(
        final_version == final_public_version,
        "Diagnostic final version correlation changed"
    );
    let mut provider_index = 0;
    let mut saved_index = 0;
    let mut previous_version = None;
    let mut elapsed = 0;
    let mut target = false;
    for (index, record) in records.iter().enumerate() {
        diagnostic_shape(
            record,
            &[
                "sequence",
                "origin",
                "phase",
                "elapsedMs",
                "snapshot",
                "context",
            ],
        )?;
        anyhow::ensure!(
            record["sequence"].as_u64() == Some(index as u64 + 1),
            "Diagnostic sequence changed"
        );
        let phase = record["phase"].as_str();
        anyhow::ensure!(
            matches!(phase, Some("setup" | "target")) && !(target && phase == Some("setup")),
            "Diagnostic phase changed"
        );
        target |= phase == Some("target");
        let current_elapsed = record["elapsedMs"]
            .as_u64()
            .filter(|value| *value >= elapsed && *value <= 300000)
            .ok_or_else(|| anyhow::anyhow!("Diagnostic clock changed"))?;
        elapsed = current_elapsed;
        let version = diagnostic_snapshot(&record["snapshot"], uri)?;
        anyhow::ensure!(
            previous_version.is_none()
                || version.is_some_and(|version| version >= previous_version.unwrap_or(0)),
            "Diagnostic document lifetime changed"
        );
        previous_version = version;
        diagnostic_value_budget(&record["context"])?;
        match record["origin"].as_str() {
            Some("configuration-change") => {
                diagnostic_shape(&record["context"], &["affected"])?;
                let affected = record["context"]["affected"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("Diagnostic event scope changed"))?;
                anyhow::ensure!(
                    !affected.is_empty() && affected.len() <= 4,
                    "Diagnostic event scope changed"
                );
                let mut next = 0;
                for key in affected {
                    let position = DIAGNOSTIC_EVENTS
                        .iter()
                        .position(|expected| key == *expected)
                        .filter(|position| *position >= next)
                        .ok_or_else(|| anyhow::anyhow!("Diagnostic event scope changed"))?;
                    next = position + 1;
                }
            }
            Some("source-provider") => {
                diagnostic_shape(&record["context"], &["only", "triggerKind", "cancelled"])?;
                let callback = callbacks
                    .get(provider_index)
                    .ok_or_else(|| anyhow::anyhow!("Diagnostic callback correlation changed"))?;
                anyhow::ensure!(
                    version.is_some()
                        && record["phase"] == callback["phase"]
                        && record["snapshot"]["version"] == callback["version"]
                        && record["context"]["only"] == callback["only"]
                        && record["context"]["triggerKind"] == callback["triggerKind"]
                        && record["context"]["cancelled"] == callback["cancelled"]
                        && matches!(record["context"]["triggerKind"].as_u64(), Some(1 | 2))
                        && record["context"]["cancelled"].is_boolean()
                        && (record["context"]["only"].is_null()
                            || record["context"]["only"]
                                .as_str()
                                .is_some_and(|kind| kind.len() <= 512)),
                    "Diagnostic callback correlation changed"
                );
                provider_index += 1;
            }
            Some("did-save") => {
                let save = saved
                    .get(saved_index)
                    .ok_or_else(|| anyhow::anyhow!("Diagnostic save correlation changed"))?;
                anyhow::ensure!(
                    version.is_some()
                        && record["context"].is_null()
                        && record["phase"] == save["phase"]
                        && record["snapshot"]["version"] == save["version"],
                    "Diagnostic save correlation changed"
                );
                saved_index += 1;
            }
            _ => anyhow::bail!("Diagnostic origin changed"),
        }
    }
    anyhow::ensure!(
        final_version >= previous_version.unwrap_or(0),
        "Diagnostic document lifetime changed"
    );
    anyhow::ensure!(
        provider_index == callbacks.len(),
        "Diagnostic callback correlation changed"
    );
    anyhow::ensure!(
        saved_index == saved.len(),
        "Diagnostic save correlation changed"
    );
    Ok(())
}
/// Explicit source contract: metadata cannot implicitly authorize a new hash.
fn validate_save_observer_metadata(raw: &Value, observer: &str) -> anyhow::Result<()> {
    if matches!(
        observer,
        LEGACY_SAVE_OBSERVER | READY_SAVE_OBSERVER | DIAGNOSTIC_SAVE_OBSERVER
    ) {
        anyhow::ensure!(
            raw.get("stableConfiguration").is_none()
                && raw["setup"].get("configurationReadiness").is_none()
                && raw["setup"].get("setupAuthorization").is_none(),
            "Unexpected stable metadata in earlier observation"
        );
    }
    match observer {
        LEGACY_SAVE_OBSERVER | READY_SAVE_OBSERVER => {
            anyhow::ensure!(
                raw.get("configurationDiagnostics").is_none(),
                "Unexpected configuration diagnostics in earlier observation"
            );
            validate_auxiliary_readiness(raw, observer)
        }
        DIAGNOSTIC_SAVE_OBSERVER => {
            validate_configuration_diagnostics(raw)?;
            // The source/protocol version has been explicitly admitted above.
            // Its auxiliary readiness implementation is unchanged byte-for-byte.
            validate_auxiliary_protocol(raw)
        }
        _ => anyhow::bail!("Unsupported save-participant observer contract"),
    }
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
        validate_save_observer_metadata(raw, hashes["save-code-actions.cjs"].as_str().unwrap())
            .unwrap();
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

#[cfg(test)]
mod auxiliary_readiness_tests {
    use super::*;

    // Synthetic protocol fixture for the pure validator. This is not captured
    // evidence, never written as a baseline and never used by native workflows.
    fn synthetic_witness() -> Value {
        let state = |index: u64, dirty: bool, disk_index: u64| {
            json!({
                "resource":"readiness.ini", "uri":"file:///tmp/vscli-probe/w/readiness.ini",
                "version":index+1, "text":format!("{AUX_INPUT}{}", "x".repeat(index as usize)),
                "dirty":dirty, "disk":format!("{AUX_INPUT}{}", "x".repeat(disk_index as usize))
            })
        };
        let initial = state(0, false, 0);
        let first = state(1, false, 1);
        let second = state(2, false, 2);
        json!({"setup":{"effective":{"autoSave":"off"},"saveParticipantReadiness":{
            "status":"observed", "resource":"readiness.ini", "input":AUX_INPUT,
            "effective":{"formatOnSave":true,"formatOnSaveMode":"file","autoSave":"off"},
            "limits":{"maxAttempts":16,"deadlineMs":5000,"retryIntervalMs":250},
            "initial":initial,"final":second,"scope":OBSERVED_SCOPE,
            "attempts":[
                {"index":1,"before":initial,"prepared":state(1,true,0),"after":first,
                    "callbackStart":0,"callbackCount":0,"matched":false,"elapsedMs":15},
                {"index":2,"before":first,"prepared":state(2,true,1),"after":second,
                    "callbackStart":0,"callbackCount":1,"matched":true,"elapsedMs":285}
            ],
            "callbacks":[{"attempt":2,"uri":"file:///tmp/vscli-probe/w/readiness.ini",
                "version":3,"text":format!("{AUX_INPUT}xx"),"resource":"readiness.ini",
                "exactDocument":true,"duringSave":true,"cancelled":false,"matched":true}]
        }}})
    }
    fn synthetic_skip() -> Value {
        json!({"setup":{"effective":{"autoSave":"afterDelay"},"saveParticipantReadiness":{
            "status":"not-run-after-delay","resource":"readiness.ini","input":AUX_INPUT,
            "disk":AUX_INPUT,"attempts":[],"callbacks":[],"scope":SKIPPED_SCOPE
        }}})
    }
    fn positive_witness() -> Value {
        let value = synthetic_witness();
        validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER).unwrap();
        value
    }
    #[test]
    fn actual_original_cohort_retains_legacy_auxiliary_contract() {
        let evidence: Vec<Value> = serde_json::from_str(include_str!(
            "vscode-reference/baselines/1.95.0/save-code-actions/linux-evidence.json"
        ))
        .unwrap();
        assert_eq!(evidence.len(), 14);
        for row in &evidence {
            validate_auxiliary_readiness(row, LEGACY_SAVE_OBSERVER).unwrap();
            assert_eq!(
                validate_auxiliary_readiness(row, READY_SAVE_OBSERVER)
                    .unwrap_err()
                    .to_string(),
                "Missing candidate auxiliary readiness"
            );
        }
    }
    #[test]
    fn synthetic_protocol_accepts_late_installation_and_explicit_after_delay_skip() {
        positive_witness();
        validate_auxiliary_readiness(&synthetic_skip(), READY_SAVE_OBSERVER).unwrap();
    }
    #[test]
    fn synthetic_protocol_preserves_outside_save_callback_without_authorizing_it() {
        let mut value = positive_witness();
        let proof = &mut value["setup"]["saveParticipantReadiness"];
        let outside = json!({"attempt":null,"uri":"file:///tmp/vscli-probe/w/readiness.ini",
            "version":1,"text":AUX_INPUT,"resource":"readiness.ini","exactDocument":true,
            "duringSave":false,"cancelled":false,"matched":false});
        proof["callbacks"]
            .as_array_mut()
            .unwrap()
            .insert(0, outside);
        proof["attempts"][0]["callbackStart"] = json!(1);
        proof["attempts"][1]["callbackStart"] = json!(1);
        validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER).unwrap();
        value["setup"]["saveParticipantReadiness"]["callbacks"][0]["matched"] = json!(true);
        assert_eq!(
            validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                .unwrap_err()
                .to_string(),
            "Auxiliary callback match proof changed"
        );
    }
    #[test]
    fn candidate_metadata_cannot_be_removed_or_added_to_legacy_observations() {
        let value = positive_witness();
        assert_eq!(
            validate_auxiliary_readiness(&value, LEGACY_SAVE_OBSERVER)
                .unwrap_err()
                .to_string(),
            "Unexpected auxiliary readiness in legacy observation"
        );
        assert_eq!(
            validate_auxiliary_readiness(&value, "unknown-source")
                .unwrap_err()
                .to_string(),
            "Unsupported save-participant observer contract"
        );
        let mut missing = value;
        missing["setup"]
            .as_object_mut()
            .unwrap()
            .remove("saveParticipantReadiness");
        assert_eq!(
            validate_auxiliary_readiness(&missing, READY_SAVE_OBSERVER)
                .unwrap_err()
                .to_string(),
            "Missing candidate auxiliary readiness"
        );
    }
    #[test]
    fn unknown_resource_and_cancelled_false_positive_are_not_filtered_from_evidence() {
        for foreign in [true, false] {
            let mut value = positive_witness();
            let callback = &mut value["setup"]["saveParticipantReadiness"]["callbacks"][0];
            let expected = if foreign {
                callback["uri"] = json!("file:///tmp/vscli-probe/w/main.txt");
                callback["resource"] = Value::Null;
                callback["exactDocument"] = json!(false);
                "Unexpected auxiliary callback resource or bounds"
            } else {
                callback["cancelled"] = json!(true);
                "Auxiliary callback match proof changed"
            };
            assert_eq!(
                validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }
    #[test]
    fn late_auxiliary_mutations_and_callback_slice_overlap_reject_the_complete_proof() {
        for mode in 0..4 {
            let mut value = positive_witness();
            let proof = &mut value["setup"]["saveParticipantReadiness"];
            let expected = match mode {
                0 => {
                    proof["attempts"][1]["prepared"]["text"] = json!("replacement");
                    "Auxiliary prepared edit changed"
                }
                1 => {
                    proof["attempts"][1]["after"]["disk"] = json!("not committed");
                    "Auxiliary null-formatter receipt changed"
                }
                2 => {
                    proof["attempts"][1]["callbackStart"] = json!(usize::MAX);
                    "Auxiliary callback slices changed"
                }
                _ => {
                    proof["final"]["version"] = json!(4);
                    "Auxiliary final snapshot changed"
                }
            };
            assert_eq!(
                validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }
    #[test]
    fn altered_limits_expired_witness_and_missing_null_fields_cannot_pass() {
        for mode in 0..4 {
            let mut value = positive_witness();
            let proof = &mut value["setup"]["saveParticipantReadiness"];
            let expected = match mode {
                0 => {
                    proof["limits"]["deadlineMs"] = json!(5001);
                    "Auxiliary readiness limits changed"
                }
                1 => {
                    proof["attempts"][1]["elapsedMs"] = json!(5000);
                    "Auxiliary readiness deadline changed"
                }
                2 => {
                    proof["callbacks"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("attempt");
                    "Invalid auxiliary metadata shape"
                }
                _ => {
                    proof["unknown"] = json!(true);
                    "Invalid auxiliary metadata shape"
                }
            };
            assert_eq!(
                validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }
    #[test]
    fn invalid_auxiliary_uri_and_stale_version_cannot_fake_matching_identity() {
        for version in [false, true] {
            let mut value = positive_witness();
            let proof = &mut value["setup"]["saveParticipantReadiness"];
            let expected = if version {
                proof["callbacks"][0]["version"] = json!(2);
                "Auxiliary callback version changed"
            } else {
                proof["initial"]["uri"] = json!("https://example.com/readiness.ini");
                "Invalid auxiliary URI"
            };
            assert_eq!(
                validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }
    #[test]
    fn after_delay_skip_cannot_hide_auxiliary_saves_or_changed_input() {
        for mode in 0..3 {
            let mut value = synthetic_skip();
            validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER).unwrap();
            let proof = &mut value["setup"]["saveParticipantReadiness"];
            match mode {
                0 => proof["attempts"] = json!([{}]),
                1 => proof["disk"] = json!("changed"),
                _ => proof["status"] = json!("observed"),
            }
            assert_eq!(
                validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                    .unwrap_err()
                    .to_string(),
                "Invalid after-delay auxiliary scope"
            );
        }
    }
    #[test]
    fn auxiliary_byte_budget_rejects_before_typed_payload_cloning() {
        let mut value = positive_witness();
        value["setup"]["saveParticipantReadiness"]["initial"]["text"] =
            json!("x".repeat(512 * 1024));
        assert_eq!(
            validate_auxiliary_readiness(&value, READY_SAVE_OBSERVER)
                .unwrap_err()
                .to_string(),
            "Auxiliary metadata exceeds bounds"
        );
    }
}

#[cfg(test)]
mod remote_auxiliary_artifact_tests {
    use super::*;
    const CANDIDATE_REVISION: &str = "2c5ee40fe0ba3cf3ff7112c0cf7bb71d3987515b";
    const INPUTS: &[(&str, &str)] = &[
        (
            "save-code-actions.cjs",
            "9ff83ee188612b4bdcdc74d59e8261de658eb58b8a9815bb12fe1fb91c80cf47",
        ),
        (
            "save-code-actions-cases.json",
            "a96e7078cd7e710cb33fc0c3d5e4e40e9cd0283e884035ff552dfd37b1a9be9c",
        ),
        (
            "save-code-actions-suite.cjs",
            "8ffef860482a52be151605f42b67a9b032f26b36ce919867f5eb59d8b0cba40f",
        ),
        (
            "save-code-actions-run.cjs",
            "2799585ec9b86cb0a7f890ff4c9f0af8f9f8994f7f0aa791f7a048c6704ee7bd",
        ),
        (
            "save-code-actions-worker.cjs",
            "80191d7df1843c79ac27ae3f44e1400f7e9af7dd894384c37fec44e861772c24",
        ),
        (
            "supervisor.cjs",
            "d0bb864b1b699fe29d6b4e24de7a1fa587fe122b6b4a92e9c74cfd25a74d5b22",
        ),
        (
            "package-lock.json",
            "74d30a58472daabec49f33d54bc1bfd1b9fb8d0858eec22a022c2464c263ca8c",
        ),
        (
            "package.json",
            "4358149fa583c7a8402d0396e06bb88b1ddb6f5ca82c2002c4b841da1a14bec3",
        ),
        (
            "extension.cjs",
            "64115fcd8cfbe8fc111970b4f536dff54e505f92625dbce117e2cd60e4490c75",
        ),
    ];
    fn read_bounded(path: &Path, maximum: usize) -> Vec<u8> {
        let bytes = fs::read(path).unwrap();
        assert!(
            bytes.len() <= maximum,
            "Artifact input exceeds declared bounds"
        );
        bytes
    }
    #[test]
    #[ignore = "requires a genuine remote-only readiness artifact; validates metadata, not target parity"]
    fn genuine_remote_readiness_artifact_preflight_preserves_full_target_evidence() {
        let root = PathBuf::from(
            std::env::var_os("VSCLI_SAVE_ACTIONS_READINESS_ARTIFACT_DIR")
                .expect("Set artifact directory to one extracted per-platform artifact root"),
        );
        let input = root.join("tests/vscode-reference-save-actions-readiness-candidate");
        let output = root.join("target/save-actions-readiness");
        let result = output.join("result");
        let revision = read_bounded(&output.join("candidate-revision.txt"), 128);
        assert_eq!(
            std::str::from_utf8(&revision).unwrap().trim(),
            CANDIDATE_REVISION
        );
        let manifest: Value =
            serde_json::from_slice(&read_bounded(&input.join("source-sha256.json"), 4096)).unwrap();
        let verified: Value = serde_json::from_slice(&read_bounded(
            &output.join("verified-source-sha256.json"),
            4096,
        ))
        .unwrap();
        assert_eq!(manifest, verified);
        assert_eq!(manifest.as_object().unwrap().len(), INPUTS.len());
        for (name, expected) in INPUTS {
            assert_eq!(manifest[*name], *expected);
            assert_eq!(
                sha(&read_bounded(&input.join(name), 8 * 1024 * 1024)),
                *expected
            );
        }
        let cases: Vec<Value> = serde_json::from_slice(&read_bounded(
            &input.join("save-code-actions-cases.json"),
            64 * 1024,
        ))
        .unwrap();
        let trace_bytes = read_bounded(&result.join("save-code-actions.json"), 8 * 1024 * 1024);
        let evidence_bytes = read_bounded(
            &result.join("save-code-actions-evidence.json"),
            8 * 1024 * 1024,
        );
        let proof: Value = serde_json::from_slice(&read_bounded(
            &result.join("save-code-actions-provenance.json"),
            128 * 1024,
        ))
        .unwrap();
        assert_eq!(proof["version"], "1.95.0");
        assert_eq!(proof["commit"], PIN);
        assert_eq!(proof["caseCount"], 14);
        assert_eq!(proof["snapshotCount"], 162);
        assert_eq!(proof["traceSha256"], sha(&trace_bytes));
        assert_eq!(proof["evidenceSha256"], sha(&evidence_bytes));
        assert_eq!(
            proof["casesSha256"],
            manifest["save-code-actions-cases.json"]
        );
        assert!(matches!(
            proof["platform"].as_str(),
            Some("linux" | "darwin" | "win32")
        ));
        assert!(matches!(
            proof["architecture"].as_str(),
            Some("x64" | "arm64")
        ));
        let trace: Vec<Value> = serde_json::from_slice(&trace_bytes).unwrap();
        let raw: Vec<Value> = serde_json::from_slice(&evidence_bytes).unwrap();
        let runs = proof["runs"].as_array().unwrap();
        assert_eq!(cases.len(), 14);
        assert_eq!(trace.len(), 14);
        assert_eq!(raw.len(), 14);
        assert_eq!(runs.len(), 14);
        assert_eq!(
            trace
                .iter()
                .map(|row| row["observations"].as_array().unwrap().len())
                .sum::<usize>(),
            162
        );
        let mut names = std::collections::BTreeSet::new();
        // Complete archive admission occurs before ANY readiness preflight.
        // This test is deliberately pure evidence qualification: no App, LSP,
        // fixture writes or expected native target output is involved.
        for (((case, row), evidence), run) in cases.iter().zip(&trace).zip(&raw).zip(runs) {
            let name = case["name"].as_str().unwrap();
            assert!(
                name.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            );
            assert!(names.insert(name));
            assert_eq!(row["name"], name);
            assert_eq!(evidence["name"], name);
            assert_eq!(run["name"], name);
            assert_eq!(run["version"], proof["version"]);
            assert_eq!(run["commit"], PIN);
            assert_eq!(run["platform"], proof["platform"]);
            assert_eq!(run["architecture"], proof["architecture"]);
            assert_eq!(run["sources"], manifest);
            let product = run["productSha256"].as_str().unwrap();
            assert_eq!(product.len(), 64);
            assert!(
                product
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
            assert_eq!(evidence["setup"]["before"], evidence["setup"]["after"]);
            assert!(evidence["setup"]["before"].is_object());
            assert_eq!(row["effective"], evidence["setup"]["effective"]);
            assert_eq!(row["effectiveKeys"], evidence["setup"]["effectiveKeys"]);
            assert_eq!(row["observations"], evidence["observations"]);
            assert_eq!(evidence["scope"], run["scope"]);
            for saved in evidence["saved"].as_array().unwrap() {
                assert_eq!(saved["text"], saved["disk"]);
            }
            let individual =
                read_bounded(&result.join(format!("{name}-evidence.json")), 1024 * 1024);
            assert_eq!(run["evidenceSha256"], sha(&individual));
            assert_eq!(
                serde_json::from_slice::<Value>(&individual).unwrap(),
                *evidence
            );
            let individual_run: Value = serde_json::from_slice(&read_bounded(
                &result.join(format!("{name}-provenance.json")),
                64 * 1024,
            ))
            .unwrap();
            assert_eq!(individual_run, *run);
        }
        for evidence in &raw {
            validate_auxiliary_readiness(evidence, READY_SAVE_OBSERVER).unwrap();
        }
        println!(
            "Validated genuine {} auxiliary metadata for 14 cases/162 retained frames; target parity was not asserted",
            proof["platform"]
        );
    }
}

mod configuration_diagnostics_tests {
    use super::*;
    const CANDIDATE_REVISION: &str = "67ab7955cd5c533aaf77241a34d71acd7aef8c3c";
    const INPUTS: &[(&str, &str)] = &[
        (
            "save-code-actions.cjs",
            "21f5ae5c7713c9053da9bbdfa660f19f59fc1b3045e6415a49ddc2d7970d52b2",
        ),
        (
            "save-code-actions-cases.json",
            "a96e7078cd7e710cb33fc0c3d5e4e40e9cd0283e884035ff552dfd37b1a9be9c",
        ),
        (
            "save-code-actions-suite.cjs",
            "8ffef860482a52be151605f42b67a9b032f26b36ce919867f5eb59d8b0cba40f",
        ),
        (
            "save-code-actions-run.cjs",
            "2799585ec9b86cb0a7f890ff4c9f0af8f9f8994f7f0aa791f7a048c6704ee7bd",
        ),
        (
            "save-code-actions-worker.cjs",
            "80191d7df1843c79ac27ae3f44e1400f7e9af7dd894384c37fec44e861772c24",
        ),
        (
            "supervisor.cjs",
            "d0bb864b1b699fe29d6b4e24de7a1fa587fe122b6b4a92e9c74cfd25a74d5b22",
        ),
        (
            "package-lock.json",
            "74d30a58472daabec49f33d54bc1bfd1b9fb8d0858eec22a022c2464c263ca8c",
        ),
        (
            "package.json",
            "4358149fa583c7a8402d0396e06bb88b1ddb6f5ca82c2002c4b841da1a14bec3",
        ),
        (
            "extension.cjs",
            "64115fcd8cfbe8fc111970b4f536dff54e505f92625dbce117e2cd60e4490c75",
        ),
    ];
    fn read_bounded(path: &Path, maximum: usize) -> Vec<u8> {
        let bytes = fs::read(path).unwrap();
        assert!(
            bytes.len() <= maximum,
            "Artifact input exceeds declared bounds"
        );
        bytes
    }
    // Pure admission of a committed genuine artifact. No App, LSP or fixture writes.
    fn archive(platform: &str) -> (Vec<Value>, Vec<Value>) {
        assert!(matches!(platform, "linux" | "darwin" | "win32"));
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(
                "tests/vscode-reference/observations/1.95.0/save-configuration-diagnostics/67ab795",
            )
            .join(platform);
        let input =
            root.join("tests/vscode-reference-save-actions-migration-diagnostics-candidate");
        let output = root.join("target/save-actions-migration-diagnostics");
        let result = output.join("result");
        let revision = read_bounded(&output.join("candidate-revision.txt"), 128);
        assert_eq!(
            std::str::from_utf8(&revision).unwrap().trim(),
            CANDIDATE_REVISION
        );
        let manifest: Value =
            serde_json::from_slice(&read_bounded(&input.join("source-sha256.json"), 4096)).unwrap();
        let verified: Value = serde_json::from_slice(&read_bounded(
            &output.join("verified-source-sha256.json"),
            4096,
        ))
        .unwrap();
        assert_eq!(manifest, verified);
        assert_eq!(manifest.as_object().unwrap().len(), INPUTS.len());
        for (name, expected) in INPUTS {
            assert_eq!(manifest[*name], *expected);
            assert_eq!(
                sha(&read_bounded(&input.join(name), 8 * 1024 * 1024)),
                *expected
            );
        }
        let cases: Vec<Value> = serde_json::from_slice(&read_bounded(
            &input.join("save-code-actions-cases.json"),
            64 * 1024,
        ))
        .unwrap();
        let trace_bytes = read_bounded(&result.join("save-code-actions.json"), 8 * 1024 * 1024);
        let evidence_bytes = read_bounded(
            &result.join("save-code-actions-evidence.json"),
            8 * 1024 * 1024,
        );
        let proof: Value = serde_json::from_slice(&read_bounded(
            &result.join("save-code-actions-provenance.json"),
            128 * 1024,
        ))
        .unwrap();
        assert_eq!(proof["version"], "1.95.0");
        assert_eq!(proof["commit"], PIN);
        assert_eq!(proof["caseCount"], 14);
        assert_eq!(proof["snapshotCount"], 162);
        assert_eq!(proof["traceSha256"], sha(&trace_bytes));
        assert_eq!(proof["evidenceSha256"], sha(&evidence_bytes));
        assert_eq!(
            proof["casesSha256"],
            manifest["save-code-actions-cases.json"]
        );
        assert_eq!(proof["platform"], platform);
        assert!(matches!(
            proof["architecture"].as_str(),
            Some("x64" | "arm64")
        ));
        let trace: Vec<Value> = serde_json::from_slice(&trace_bytes).unwrap();
        let raw: Vec<Value> = serde_json::from_slice(&evidence_bytes).unwrap();
        let runs = proof["runs"].as_array().unwrap();
        assert_eq!(cases.len(), 14);
        assert_eq!(trace.len(), 14);
        assert_eq!(raw.len(), 14);
        assert_eq!(runs.len(), 14);
        assert_eq!(
            trace
                .iter()
                .map(|row| row["observations"].as_array().unwrap().len())
                .sum::<usize>(),
            162
        );
        let mut names = std::collections::BTreeSet::new();
        // Complete archive admission occurs before ANY readiness preflight.
        // This test is deliberately pure evidence qualification: no App, LSP,
        // fixture writes or expected native target output is involved.
        for (((case, row), evidence), run) in cases.iter().zip(&trace).zip(&raw).zip(runs) {
            let name = case["name"].as_str().unwrap();
            assert!(
                name.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            );
            assert!(names.insert(name));
            assert_eq!(row["name"], name);
            assert_eq!(evidence["name"], name);
            assert_eq!(run["name"], name);
            assert_eq!(run["version"], proof["version"]);
            assert_eq!(run["commit"], PIN);
            assert_eq!(run["platform"], proof["platform"]);
            assert_eq!(run["architecture"], proof["architecture"]);
            assert_eq!(run["sources"], manifest);
            let product = run["productSha256"].as_str().unwrap();
            assert_eq!(product.len(), 64);
            assert!(
                product
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
            assert_eq!(evidence["setup"]["before"], evidence["setup"]["after"]);
            assert!(evidence["setup"]["before"].is_object());
            assert_eq!(row["effective"], evidence["setup"]["effective"]);
            assert_eq!(row["effectiveKeys"], evidence["setup"]["effectiveKeys"]);
            assert_eq!(row["observations"], evidence["observations"]);
            let callbacks: Vec<_> = evidence["callbacks"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|callback| callback["phase"] == "target")
                .map(|callback| {
                    json!({"only":callback["only"], "triggerKind":callback["triggerKind"],
                    "text":callback["text"], "returned":callback["returned"]})
                })
                .collect();
            assert_eq!(row["callbacks"], json!(callbacks));
            assert_eq!(evidence["scope"], run["scope"]);
            for saved in evidence["saved"].as_array().unwrap() {
                assert_eq!(saved["text"], saved["disk"]);
            }
            let individual =
                read_bounded(&result.join(format!("{name}-evidence.json")), 1024 * 1024);
            assert_eq!(run["evidenceSha256"], sha(&individual));
            assert_eq!(
                serde_json::from_slice::<Value>(&individual).unwrap(),
                *evidence
            );
            let individual_run: Value = serde_json::from_slice(&read_bounded(
                &result.join(format!("{name}-provenance.json")),
                64 * 1024,
            ))
            .unwrap();
            assert_eq!(individual_run, *run);
        }
        for evidence in &raw {
            validate_save_observer_metadata(evidence, DIAGNOSTIC_SAVE_OBSERVER).unwrap();
        }
        (raw, trace)
    }
    fn positive() -> Value {
        archive("linux")
            .0
            .into_iter()
            .find(|row| row["name"] == "ancestor-false-child")
            .unwrap()
    }
    fn rejects(mutate: impl FnOnce(&mut Value), expected: &str) {
        let mut raw = positive(); // Genuine source/hash/protocol/trace admitted first.
        mutate(&mut raw);
        assert_eq!(
            validate_save_observer_metadata(&raw, DIAGNOSTIC_SAVE_OBSERVER)
                .unwrap_err()
                .to_string(),
            expected
        );
    }
    fn record<'a>(raw: &'a mut Value, origin: &str) -> &'a mut Value {
        raw["configurationDiagnostics"]["records"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|record| record["origin"] == origin)
            .unwrap()
    }
    #[test]
    fn genuine_three_platform_diagnostics_preserve_full_projection_and_timeline() {
        let baseline: Value = serde_json::from_slice(&read_bounded(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(REFERENCE)
                .join("baselines/1.95.0/save-code-actions/linux.json"),
            8 * 1024 * 1024,
        ))
        .unwrap();
        for platform in ["linux", "darwin", "win32"] {
            let (raw, trace) = archive(platform);
            assert_eq!(
                json!(trace),
                baseline,
                "Actual 67ab target projection changed"
            );
            for (name, key) in [
                ("ancestor-false-child", "source.fixAll.child"),
                ("source-ancestor-false-family", "source.fixAll"),
            ] {
                let row = raw.iter().find(|row| row["name"] == name).unwrap();
                let proof = &row["configurationDiagnostics"];
                assert_eq!(
                    proof["initial"]["effective"]["codeActionsOnSave"][key],
                    false
                );
                let records = proof["records"].as_array().unwrap();
                let save = records
                    .iter()
                    .position(|record| {
                        record["origin"] == "did-save" && record["phase"] == "target"
                    })
                    .unwrap();
                for record in &records[..=save] {
                    assert_eq!(
                        record["snapshot"]["effective"]["codeActionsOnSave"][key],
                        false
                    );
                }
                assert!(
                    records.iter().skip(save + 1).any(|record| record["origin"]
                        == "configuration-change"
                        && record["snapshot"]["effective"]["codeActionsOnSave"][key] == "never")
                );
                assert_eq!(
                    proof["final"]["effective"]["codeActionsOnSave"][key],
                    "never"
                );
            }
        }
    }
    #[test]
    fn diagnostic_metadata_requires_its_exact_observer_contract() {
        let raw = positive();
        for observer in [LEGACY_SAVE_OBSERVER, READY_SAVE_OBSERVER] {
            assert_eq!(
                validate_save_observer_metadata(&raw, observer)
                    .unwrap_err()
                    .to_string(),
                "Unexpected configuration diagnostics in earlier observation"
            );
            assert_eq!(
                validate_auxiliary_readiness(&raw, observer)
                    .unwrap_err()
                    .to_string(),
                "Unexpected configuration diagnostics in earlier observation"
            );
        }
        assert_eq!(
            validate_save_observer_metadata(&raw, "unknown")
                .unwrap_err()
                .to_string(),
            "Unsupported save-participant observer contract"
        );
        rejects(
            |raw| {
                raw.as_object_mut()
                    .unwrap()
                    .remove("configurationDiagnostics");
            },
            "Missing configuration diagnostics",
        );
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["unexpected"] = json!(true);
            },
            "Invalid diagnostic metadata shape",
        );
    }
    #[test]
    fn diagnostic_sequence_clock_and_phase_are_not_normalized() {
        rejects(
            |raw| {
                let first = raw["configurationDiagnostics"]["records"][0].clone();
                raw["configurationDiagnostics"]["records"] = json!(vec![first; 129]);
            },
            "Diagnostic records exceed bounds",
        );
        rejects(
            |raw| raw["configurationDiagnostics"]["records"][0]["sequence"] = json!(2),
            "Diagnostic sequence changed",
        );
        rejects(
            |raw| raw["configurationDiagnostics"]["records"][0]["elapsedMs"] = json!(-1),
            "Diagnostic clock changed",
        );
        rejects(
            |raw| raw["configurationDiagnostics"]["records"][0]["origin"] = json!("unknown"),
            "Diagnostic origin changed",
        );
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["records"][1]["phase"] = json!("target");
                raw["configurationDiagnostics"]["records"][2]["phase"] = json!("setup");
            },
            "Diagnostic phase changed",
        );
    }
    #[test]
    fn diagnostic_document_uri_language_and_final_version_are_exact() {
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["initial"]["uri"] = json!("file:///other/main.txt")
            },
            "Diagnostic resource identity changed",
        );
        rejects(
            |raw| record(raw, "did-save")["snapshot"]["uri"] = json!("file:///other/main.txt"),
            "Diagnostic resource identity changed",
        );
        rejects(
            |raw| record(raw, "did-save")["snapshot"]["languageId"] = json!("cpp"),
            "Diagnostic resource identity changed",
        );
        rejects(
            |raw| record(raw, "did-save")["snapshot"]["exactDocument"] = json!(false),
            "Diagnostic document identity changed",
        );
        rejects(
            |raw| {
                record(raw, "did-save")["snapshot"]
                    .as_object_mut()
                    .unwrap()
                    .remove("version");
            },
            "Invalid diagnostic metadata shape",
        );
        rejects(
            |raw| record(raw, "did-save")["snapshot"]["version"] = json!(0),
            "Diagnostic document identity changed",
        );
        rejects(
            |raw| {
                let version = raw["configurationDiagnostics"]["final"]["version"]
                    .as_u64()
                    .unwrap();
                raw["configurationDiagnostics"]["final"]["version"] = json!(version + 1);
            },
            "Diagnostic final version correlation changed",
        );
    }
    #[test]
    fn diagnostic_callbacks_and_save_records_cannot_be_dropped_or_rewritten() {
        rejects(
            |raw| record(raw, "source-provider")["context"]["only"] = json!("changed"),
            "Diagnostic callback correlation changed",
        );
        for (origin, error) in [
            ("source-provider", "Diagnostic callback correlation changed"),
            ("did-save", "Diagnostic save correlation changed"),
        ] {
            rejects(
                |raw| {
                    let records = raw["configurationDiagnostics"]["records"]
                        .as_array_mut()
                        .unwrap();
                    let index = records
                        .iter()
                        .position(|record| record["origin"] == origin)
                        .unwrap();
                    records.remove(index);
                    for (index, record) in records.iter_mut().enumerate() {
                        record["sequence"] = json!(index + 1);
                    }
                },
                error,
            );
        }
    }
    #[test]
    fn diagnostic_configuration_event_and_inspection_scope_is_closed() {
        rejects(
            |raw| record(raw, "configuration-change")["context"]["affected"] = json!(["unknown"]),
            "Diagnostic event scope changed",
        );
        rejects(
            |raw| {
                record(raw, "configuration-change")["context"]["affected"] =
                    json!(["editor.codeActionsOnSave", "editor.codeActionsOnSave"])
            },
            "Diagnostic event scope changed",
        );
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["final"]["inspect"]["autoSave"]["key"] =
                    json!("editor.autoSave")
            },
            "Diagnostic inspection accessor changed",
        );
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["final"]["inspect"]["autoSave"]["unknown"] =
                    json!(false)
            },
            "Invalid diagnostic inspection",
        );
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["final"]["effective"]["formatOnSave"] =
                    json!("true")
            },
            "Invalid diagnostic configuration value",
        );
    }
    #[test]
    fn diagnostic_whole_and_per_value_budgets_reject_before_schema_work() {
        rejects(
            |raw| raw["configurationDiagnostics"]["scope"] = json!("x".repeat(524288)),
            "Diagnostic metadata exceeds bounds",
        );
        rejects(
            |raw| {
                raw["configurationDiagnostics"]["final"]["inspect"]["autoSave"]["oversized"] =
                    json!("x".repeat(32768))
            },
            "Diagnostic value exceeds bounds",
        );
        rejects(
            |raw| {
                let mut nested = json!(false);
                for _ in 0..9 {
                    nested = json!([nested]);
                }
                record(raw, "source-provider")["context"] = nested;
            },
            "Diagnostic value exceeds bounds",
        );
    }
    #[test]
    fn previous_observer_metadata_contracts_remain_independently_accepted() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join(REFERENCE);
        let legacy: Vec<Value> = serde_json::from_slice(&read_bounded(
            &base.join("baselines/1.95.0/save-code-actions/linux-evidence.json"),
            8 * 1024 * 1024,
        ))
        .unwrap();
        for raw in &legacy {
            validate_save_observer_metadata(raw, LEGACY_SAVE_OBSERVER).unwrap();
        }
        for platform in ["linux", "darwin", "win32"] {
            let raw: Vec<Value> = serde_json::from_slice(&read_bounded(
                &base
                    .join("observations/1.95.0/save-participant-readiness/2c5ee40")
                    .join(platform)
                    .join("target/save-actions-readiness/result/save-code-actions-evidence.json"),
                8 * 1024 * 1024,
            ))
            .unwrap();
            assert_eq!(raw.len(), 14);
            for row in &raw {
                validate_save_observer_metadata(row, READY_SAVE_OBSERVER).unwrap();
            }
        }
    }
}

#[path = "save_code_actions_reference/stable_v2.rs"]
mod stable_v2;
