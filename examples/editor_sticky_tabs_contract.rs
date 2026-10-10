//! Strict replay of the independently observed eighteen-case sticky cohort.
//! Groups + real Documents; App, terminal input, persistence and mouse are separate.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use vscli::{
    document::{Document, Selection},
    editor_groups::{Change, Groups, Membership, OpenMode},
};

const ROOT: &str = "tests/vscode-reference";
// This scratch file is intended to be installed under examples/ before compiling.
const CASE_SOURCE: &str = include_str!("../tests/vscode-reference/editor-sticky-tabs-cases.json");
const COMMIT: &str = "912bb683695358a54ae0c670461738984cbb5b95";
const SCOPE: &str = "Independent sticky-prefix/preview membership, original pin/unpin/protected and forced clean Close/batches, per-profile close policy, public groups/views/identities/events and unchanged Unicode CRLF disks; no graphical mouse or dirty dialog";
const SETUP_SCOPE: &str = "Fixed separately labelled public API opens/selections and original pin/focus commands establish clean membership/view/MRU inputs; not target output retries";
const SETTLEMENT_SCOPE: &str = "Awaited acknowledged operation then 100ms unchanged public state; no target retry or preferred-output predicate";
const SOURCES: [(&str, &str); 8] = [
    (
        "editor-sticky-tabs-cases.json",
        "75393744f2fe70f6f1ddfea5f00794cf45ddb88619d2692e20640bb709f65cf9",
    ),
    (
        "editor-sticky-tabs-run.cjs",
        "a3a3792f52a8ea2e091aea03ebf99256a2273533821f64a34d1d10267f349cda",
    ),
    (
        "editor-sticky-tabs-suite.cjs",
        "0821192a8135f054f054ec9eb766b50c5a27418b9fa68ffdc4ae770399f40a35",
    ),
    (
        "editor-sticky-tabs-worker.cjs",
        "d4b56dd8fd343badbd35328b8b548ed5fe1021248b6b60ec771318fe1a3d845c",
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
const MONOTONIC_ROOT: &str = "tests/vscode-reference-sticky-monotonic-candidate";
const MONOTONIC_SUITE: &str = "2891fc2d0895ba2963db3b2f026c9f5b6b1b877106630d2cbab920a6c13299d8";
const MONOTONIC_ARCHIVE: &str =
    "tests/vscode-reference/observations/1.95.0/sticky-monotonic/3e81350";

fn current_baseline() -> Result<Inputs> {
    load_with_contract(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(MONOTONIC_ARCHIVE)
            .join("linux/target/vscode-reference/result/editor-sticky-tabs"),
        SourceContract::Monotonic,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceContract {
    Historical,
    Monotonic,
}
impl SourceContract {
    fn root(self) -> &'static str {
        match self {
            Self::Historical => ROOT,
            Self::Monotonic => MONOTONIC_ROOT,
        }
    }
    fn sources(self) -> [(&'static str, &'static str); 8] {
        let mut sources = SOURCES;
        if self == Self::Monotonic {
            sources[2].1 = MONOTONIC_SUITE;
        }
        sources
    }
    fn maximum_elapsed(self) -> u64 {
        match self {
            Self::Historical => 3100,
            Self::Monotonic => 2999,
        }
    }
}

const FILES: [(&str, &str); 8] = [
    ("a.txt", "猫🙂 a-fixture\r\nline-a\r\n"),
    ("b.txt", "β🙂 b-fixture\r\nline-b\r\n"),
    ("c.txt", "γ🙂 c-fixture\r\nline-c\r\n"),
    ("d.txt", "δ🙂 d-fixture\r\nline-d\r\n"),
    ("e.txt", "ε🙂 e-fixture\r\nline-e\r\n"),
    ("f.txt", "ζ🙂 f-fixture\r\nline-f\r\n"),
    ("g.txt", "η🙂 g-fixture\r\nline-g\r\n"),
    ("h.txt", "θ🙂 h-fixture\r\nline-h\r\n"),
];
const EMPTY_BOUNDARIES: [&str; 4] = [
    "forced-close-clean-sticky-default-policy",
    "never-policy-ordinary-close-sticky",
    "mouse-policy-keyboard-close-sticky",
    "close-all-clean-nonsticky-empty-group-boundary",
];
const MAX_BYTES: u64 = 16 * 1024 * 1024;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn keys(value: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let object = value.as_object().context("Expected object")?;
    ensure!(
        required.iter().all(|key| object.contains_key(*key))
            && object
                .keys()
                .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str())),
        "Object fields differ"
    );
    Ok(())
}
fn array(value: &Value, cap: usize) -> Result<&[Value]> {
    let array = value.as_array().context("Expected array")?;
    ensure!(array.len() <= cap, "Array budget exceeded");
    Ok(array)
}
fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    })
}
fn read(path: &Path, total: &mut u64) -> Result<Vec<u8>> {
    let file = fs::File::open(path).with_context(|| format!("Read {}", path.display()))?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= MAX_BYTES,
        "Reference file budget/type invalid"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Reference grew beyond budget"
    );
    *total = total
        .checked_add(bytes.len() as u64)
        .context("Read byte counter exhausted")?;
    ensure!(
        *total <= 64 * 1024 * 1024,
        "Aggregate reference bytes exceeded64MiB"
    );
    Ok(bytes)
}
fn parsed(bytes: &[u8]) -> Result<Value> {
    Ok(serde_json::from_slice(bytes)?)
}
fn point(text: &str, scalar: usize) -> Result<Value> {
    ensure!(scalar <= text.chars().count(), "Selection exceeds text");
    let prefix: String = text.chars().take(scalar).collect();
    ensure!(
        !(prefix.ends_with('\r') && text.chars().nth(scalar) == Some('\n')),
        "Selection splits CRLF"
    );
    let line = prefix.bytes().filter(|b| *b == b'\n').count();
    let start = prefix.rfind('\n').map_or(0, |p| p + 1);
    Ok(json!({"line":line,"character":prefix[start..].encode_utf16().count(),"scalar":scalar}))
}
fn validate_editor(editor: &Value) -> Result<()> {
    keys(
        editor,
        &[
            "resource",
            "viewColumn",
            "documentObject",
            "text",
            "dirty",
            "version",
            "eol",
            "selections",
        ],
        &[],
    )?;
    ensure!(
        FILES.iter().any(|(name, _)| editor["resource"] == *name),
        "Editor resource outside fixtures"
    );
    let text = editor["text"].as_str().context("Editor text missing")?;
    ensure!(
        text.len() < 4096
            && editor["dirty"].is_boolean()
            && matches!(editor["eol"].as_str(), Some("CRLF" | "LF")),
        "Editor text/state invalid"
    );
    ensure!(
        editor["documentObject"]
            .as_u64()
            .is_some_and(|id| (1..=128).contains(&id))
            && editor["version"].as_u64().is_some_and(|v| v > 0),
        "Public identity/version invalid"
    );
    ensure!(
        editor["viewColumn"]
            .as_u64()
            .is_some_and(|v| (1..=4).contains(&v)),
        "Editor column invalid"
    );
    let selections = array(&editor["selections"], 128)?;
    ensure!(!selections.is_empty(), "Missing selection");
    for selection in selections {
        keys(selection, &["anchor", "cursor"], &[])?;
        for field in ["anchor", "cursor"] {
            keys(&selection[field], &["line", "character", "scalar"], &[])?;
            let scalar = usize::try_from(
                selection[field]["scalar"]
                    .as_u64()
                    .context("Missing scalar")?,
            )?;
            ensure!(
                selection[field] == point(text, scalar)?,
                "Selection UTF16/scalar mismatch"
            );
        }
    }
    Ok(())
}
fn validate_tab(tab: &Value) -> Result<()> {
    keys(
        tab,
        &["resource", "active", "dirty", "pinned", "preview"],
        &[],
    )?;
    ensure!(
        FILES.iter().any(|(name, _)| tab["resource"] == *name),
        "Tab outside fixtures"
    );
    ensure!(
        ["active", "dirty", "pinned", "preview"]
            .iter()
            .all(|key| tab[*key].is_boolean())
            && !(tab["pinned"] == true && tab["preview"] == true),
        "Tab flags invalid"
    );
    Ok(())
}
fn validate_snapshot(value: &Value) -> Result<()> {
    keys(
        value,
        &["groups", "active", "visible", "documents"],
        &["action", "opened", "requestedPreview"],
    )?;
    let groups = array(&value["groups"], 4)?;
    let visible = array(&value["visible"], 4)?;
    ensure!(
        !groups.is_empty() && groups.iter().filter(|g| g["active"] == true).count() == 1,
        "Group inventory invalid"
    );
    let documents = array(&value["documents"], 8)?;
    ensure!(documents.len() == 8, "Document inventory invalid");
    for (document, (name, initial)) in documents.iter().zip(FILES) {
        keys(
            document,
            &[
                "resource",
                "loaded",
                "documentObject",
                "text",
                "dirty",
                "version",
                "disk",
            ],
            &[],
        )?;
        ensure!(
            document["resource"] == name
                && document["disk"] == initial
                && document["loaded"].is_boolean()
                && document["dirty"].is_boolean()
                && document["text"].as_str().is_some_and(|s| s.len() < 4096),
            "Fixture bytes/state differ"
        );
        if document["loaded"] == true {
            ensure!(
                document["documentObject"]
                    .as_u64()
                    .is_some_and(|id| (1..=128).contains(&id))
                    && document["version"].as_u64().is_some_and(|v| v > 0),
                "Loaded public identity invalid"
            );
        } else {
            ensure!(
                document["documentObject"].is_null()
                    && document["version"].is_null()
                    && document["dirty"] == false
                    && document["text"] == initial,
                "Unloaded state inconsistent"
            );
        }
    }
    let empty = groups.len() == 1 && groups[0]["tabs"] == json!([]);
    if empty {
        ensure!(
            visible.is_empty() && value["active"].is_null(),
            "Empty group still has editor"
        );
    } else {
        ensure!(visible.len() == groups.len(), "Visible inventory differs");
        validate_editor(&value["active"])?;
    }
    let mut tab_count = 0;
    for (index, group) in groups.iter().enumerate() {
        keys(group, &["viewColumn", "active", "tabs"], &[])?;
        ensure!(
            group["viewColumn"] == index + 1 && group["active"].is_boolean(),
            "Group order invalid"
        );
        let tabs = array(&group["tabs"], 8)?;
        tab_count += tabs.len();
        if empty {
            continue;
        }
        ensure!(
            !tabs.is_empty()
                && tabs.iter().filter(|t| t["active"] == true).count() == 1
                && tabs.iter().filter(|t| t["preview"] == true).count() <= 1,
            "Membership inventory invalid"
        );
        let mut seen_nonsticky = false;
        let mut resources = BTreeSet::new();
        for tab in tabs {
            validate_tab(tab)?;
            ensure!(
                resources.insert(tab["resource"].as_str().unwrap()),
                "Duplicate group resource"
            );
            if tab["pinned"] == true {
                ensure!(!seen_nonsticky, "Sticky prefix broken");
            } else {
                seen_nonsticky = true;
            }
            let doc = documents
                .iter()
                .find(|doc| doc["resource"] == tab["resource"])
                .unwrap();
            ensure!(
                doc["loaded"] == true && doc["dirty"] == tab["dirty"],
                "Tab/document flags inconsistent"
            );
        }
        let editor = &visible[index];
        validate_editor(editor)?;
        let active = tabs.iter().find(|t| t["active"] == true).unwrap();
        ensure!(
            editor["viewColumn"] == index + 1 && editor["resource"] == active["resource"],
            "Visible membership differs"
        );
        let doc = documents
            .iter()
            .find(|doc| doc["resource"] == editor["resource"])
            .unwrap();
        ensure!(
            ["documentObject", "text", "dirty", "version"]
                .iter()
                .all(|key| editor[*key] == doc[*key]),
            "Visible model identity differs"
        );
        if group["active"] == true {
            ensure!(*editor == value["active"], "Active editor differs");
        }
    }
    ensure!(tab_count <= 8, "Total memberships exceeded8");
    Ok(())
}
fn state(value: &Value) -> Result<Value> {
    let mut value = value.clone();
    let object = value.as_object_mut().context("Snapshot missing")?;
    object.remove("action");
    object.remove("opened");
    object.remove("requestedPreview");
    Ok(value)
}
fn validate_settlement(value: &Value, contract: SourceContract) -> Result<()> {
    keys(value, &["observed", "reads", "elapsedMs", "scope"], &[])?;
    ensure!(
        value["scope"] == SETTLEMENT_SCOPE
            && value["reads"]
                .as_u64()
                .is_some_and(|v| (2..=10000).contains(&v))
            && value["elapsedMs"]
                .as_u64()
                .is_some_and(|v| (100..=contract.maximum_elapsed()).contains(&v)),
        "Settlement differs"
    );
    validate_snapshot(&value["observed"])
}
fn validate_changes(value: &Value, supplemental: bool) -> Result<()> {
    let changes = array(value, 128)?;
    let mut bytes = 0;
    for change in changes {
        keys(change, &["text", "rangeOffset", "rangeLength"], &[])?;
        let text = change["text"].as_str().context("Changed text missing")?;
        bytes += text.len();
        ensure!(
            bytes <= if supplemental { 65536 } else { 4096 },
            "Changed text budget exceeded"
        );
        ensure!(
            ["rangeOffset", "rangeLength"]
                .iter()
                .all(|key| change[*key].as_u64().is_some_and(|v| v <= i32::MAX as u64)),
            "Change range invalid"
        );
    }
    Ok(())
}
fn validate_events(value: &Value) -> Result<()> {
    for event in array(value, 1024)? {
        match event["kind"].as_str().context("Event kind missing")? {
            "tabs" => {
                keys(event, &["kind", "opened", "closed", "changed"], &[])?;
                for key in ["opened", "closed", "changed"] {
                    for tab in array(&event[key], 8)? {
                        validate_tab(tab)?;
                    }
                }
            }
            "groups" => {
                keys(event, &["kind", "opened", "closed", "changed"], &[])?;
                for key in ["opened", "closed", "changed"] {
                    for column in array(&event[key], 4)? {
                        ensure!(
                            column.as_u64().is_some_and(|v| (1..=4).contains(&v)),
                            "Event group invalid"
                        );
                    }
                }
            }
            "active" => {
                keys(event, &["kind", "editor"], &[])?;
                if !event["editor"].is_null() {
                    validate_editor(&event["editor"])?;
                }
            }
            "visible" => {
                keys(event, &["kind", "editors"], &[])?;
                for editor in array(&event["editors"], 4)? {
                    validate_editor(editor)?;
                }
            }
            "selection" => {
                keys(event, &["kind", "selectionKind", "editor"], &[])?;
                ensure!(
                    event["selectionKind"].is_null()
                        || event["selectionKind"]
                            .as_u64()
                            .is_some_and(|v| (1..=3).contains(&v)),
                    "Selection event kind invalid"
                );
                validate_editor(&event["editor"])?;
            }
            "document-change" | "document-close" => {
                let changed = event["kind"] == "document-change";
                keys(
                    event,
                    if changed {
                        &[
                            "kind",
                            "resource",
                            "documentObject",
                            "dirty",
                            "version",
                            "changes",
                        ]
                    } else {
                        &["kind", "resource", "documentObject"]
                    },
                    &[],
                )?;
                ensure!(
                    FILES.iter().any(|(name, _)| event["resource"] == *name)
                        && event["documentObject"]
                            .as_u64()
                            .is_some_and(|v| (1..=128).contains(&v)),
                    "Document event resource/identity invalid"
                );
                if changed {
                    ensure!(
                        event["dirty"].is_boolean()
                            && event["version"].as_u64().is_some_and(|v| v > 0),
                        "Document event state invalid"
                    );
                    validate_changes(&event["changes"], false)?;
                }
            }
            kind => bail!("Unknown fixture event: {kind}"),
        }
    }
    Ok(())
}
fn validate_supplemental(
    value: &Value,
    normal_count: usize,
    setups: usize,
    targets: usize,
) -> Result<()> {
    let events = array(value, 256)?;
    let mut bytes = 0;
    for event in events {
        let changed = event["kind"] == "document-change";
        ensure!(
            changed || event["kind"] == "document-close",
            "Supplemental kind invalid"
        );
        keys(
            event,
            &[
                "kind",
                "uri",
                "scheme",
                "phase",
                "fixtureEventCount",
                "documentObject",
                "languageId",
                "dirty",
                "version",
            ],
            if changed { &["changes"] } else { &[] },
        )?;
        ensure!(
            changed == event.get("changes").is_some(),
            "Supplemental changes missing/unexpected"
        );
        let uri = event["uri"].as_str().context("Supplemental URI missing")?;
        let scheme = event["scheme"]
            .as_str()
            .context("Supplemental scheme missing")?;
        ensure!(
            !scheme.is_empty()
                && scheme != "file"
                && uri.len() <= 4096
                && uri.starts_with(&format!("{scheme}:")),
            "Supplemental URI/scheme invalid"
        );
        ensure!(
            event["languageId"].as_str().is_some_and(|v| v.len() <= 128)
                && event["documentObject"]
                    .as_u64()
                    .is_some_and(|v| (1..=128).contains(&v))
                && event["version"].as_u64().is_some_and(|v| v > 0)
                && event["dirty"].is_boolean()
                && event["fixtureEventCount"]
                    .as_u64()
                    .is_some_and(|v| v <= normal_count as u64),
            "Supplemental state invalid"
        );
        match event["phase"]["kind"].as_str() {
            Some("readiness" | "initial" | "verification") => {
                keys(&event["phase"], &["kind"], &[])?
            }
            Some("setup" | "target") => {
                keys(&event["phase"], &["kind", "index"], &[])?;
                let count = if event["phase"]["kind"] == "setup" {
                    setups
                } else {
                    targets
                };
                ensure!(
                    event["phase"]["index"]
                        .as_u64()
                        .is_some_and(|v| v < count as u64),
                    "Supplemental phase index invalid"
                );
            }
            _ => bail!("Supplemental phase invalid"),
        }
        if changed {
            validate_changes(&event["changes"], true)?;
        }
        bytes += serde_json::to_vec(event)?.len();
        ensure!(
            bytes <= 512 * 1024,
            "Supplemental aggregate exceeded budget"
        );
    }
    Ok(())
}
fn ordered_slice(global: &[Value], local: &[Value], cursor: &mut usize) -> Result<()> {
    if local.is_empty() {
        return Ok(());
    }
    let relative = global
        .get(*cursor..)
        .context("Event cursor invalid")?
        .windows(local.len())
        .position(|window| window == local)
        .context("Operation event slice missing/out of global order")?;
    *cursor += relative + local.len();
    Ok(())
}
fn profile_hash(policy: &Value, close: &str) -> Result<String> {
    let mut pairs = vec![
        ("telemetry.telemetryLevel", json!("off")),
        ("update.mode", json!("none")),
        ("extensions.autoUpdate", json!(false)),
        ("extensions.autoCheckUpdates", json!(false)),
        ("security.workspace.trust.enabled", json!(false)),
        ("workbench.startupEditor", json!("none")),
    ];
    for key in [
        "workbench.editor.enablePreview",
        "workbench.editor.enablePreviewFromQuickOpen",
        "workbench.editor.enablePreviewFromCodeNavigation",
        "workbench.editor.openPositioning",
        "workbench.editor.focusRecentEditorAfterClose",
        "workbench.editor.closeEmptyGroups",
        "workbench.editor.revealIfOpen",
    ] {
        pairs.push((key, policy[key].clone()));
    }
    pairs.extend([
        ("workbench.editor.preventPinnedEditorClose", json!(close)),
        ("files.autoSave", json!("off")),
        ("editor.quickSuggestions", json!(false)),
        ("editor.parameterHints.enabled", json!(false)),
        ("editor.detectIndentation", json!(false)),
        ("editor.tabSize", json!(4)),
        ("editor.insertSpaces", json!(true)),
        ("editor.wordWrap", json!("off")),
        ("editor.autoClosingQuotes", json!("never")),
        ("editor.autoClosingBrackets", json!("never")),
        ("editor.formatOnSave", json!(false)),
        ("editor.codeActionsOnSave", json!({})),
    ]);
    let encoded = pairs
        .into_iter()
        .map(|(key, value)| {
            Ok(format!(
                "{}:{}",
                serde_json::to_string(key)?,
                serde_json::to_string(&value)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(",");
    Ok(hash(format!("{{{encoded}}}").as_bytes()))
}
fn product_identity(platform: &str, architecture: &str) -> Result<(String, String)> {
    if platform == "linux" && architecture == "x64" {
        // Frozen independent unattended actual capture, not derived from native replay.
        return Ok((
            "3a9db06699900b7da70447f3e1cc5ca65cdbfceab99e9eb94757ffcbe582e08d".into(),
            "31a0d92e34790ab32a143d8e68a5886b5680a31e0683b17bdb3dcfeba1a1fd78".into(),
        ));
    }
    let download = match (platform, architecture) {
        ("linux", "arm64") => "linux-arm64",
        ("darwin", "x64") => "darwin",
        ("darwin", "arm64") => "darwin-arm64",
        ("win32", "x64") => "win32-x64-archive",
        ("win32", "arm64") => "win32-arm64-archive",
        _ => bail!("Unknown reference platform/architecture"),
    };
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/vscode-reference/cache")
        .join(format!("vscode-{download}-1.95.0"));
    let (product, executable) = if platform == "darwin" {
        (
            base.join("Visual Studio Code.app/Contents/Resources/app/product.json"),
            base.join("Visual Studio Code.app/Contents/MacOS/Electron"),
        )
    } else {
        (
            base.join("resources/app/product.json"),
            base.join(if platform == "win32" {
                "Code.exe"
            } else {
                "code"
            }),
        )
    };
    let mut total = 0;
    let product = read(&product, &mut total)?;
    ensure!(
        parsed(&product)?["commit"] == COMMIT,
        "Installed pinned product commit differs"
    );
    let file = fs::File::open(&executable).context(
        "Fresh non-Linux-x64 captures require their pinned local launcher for independent proof",
    )?;
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= 512 * 1024 * 1024,
        "Launcher budget invalid"
    );
    let mut reader = file.take(512 * 1024 * 1024 + 1);
    let mut sha = Sha256::new();
    let mut count = 0u64;
    let mut block = [0u8; 32768];
    loop {
        let n = reader.read(&mut block)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(count <= 512 * 1024 * 1024, "Launcher grew beyond budget");
        sha.update(&block[..n]);
    }
    let after = reader.get_ref().metadata()?;
    ensure!(
        before.len() == count && after.len() == count && before.modified()? == after.modified()?,
        "Launcher changed during proof"
    );
    Ok((hash(&product), format!("{:x}", sha.finalize())))
}

struct Corpus {
    cases: Vec<Value>,
    rows: Vec<Value>,
    raw: Vec<Value>,
    platform: String,
}
struct Artifact {
    bytes: Vec<u8>,
    value: Value,
}
struct Inputs {
    contract: SourceContract,
    trace: Artifact,
    evidence: Artifact,
    proof: Value,
    runs: Vec<(Artifact, Value)>,
}
fn artifact(path: &Path, total: &mut u64) -> Result<Artifact> {
    let bytes = read(path, total)?;
    let value = parsed(&bytes)?;
    Ok(Artifact { bytes, value })
}
fn load(directory: &Path) -> Result<Inputs> {
    load_with_contract(directory, SourceContract::Historical)
}
fn verify_sources(contract: SourceContract, total: &mut u64) -> Result<()> {
    for (name, expected) in contract.sources() {
        ensure!(
            hash(&read(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(contract.root())
                    .join(name),
                total
            )?) == expected,
            "Frozen source digest differs: {name}"
        );
    }
    Ok(())
}
fn load_with_contract(directory: &Path, contract: SourceContract) -> Result<Inputs> {
    let mut total = 0;
    verify_sources(contract, &mut total)?;
    let trace = artifact(&directory.join("editor-sticky-tabs.json"), &mut total)?;
    let evidence = artifact(
        &directory.join("editor-sticky-tabs-evidence.json"),
        &mut total,
    )?;
    let proof = artifact(
        &directory.join("editor-sticky-tabs-provenance.json"),
        &mut total,
    )?
    .value;
    let cases: Value = serde_json::from_str(CASE_SOURCE)?;
    let mut runs = Vec::new();
    for case in array(&cases["cases"], 18)? {
        let name = case["name"].as_str().context("Case name missing")?;
        ensure!(
            !name.is_empty()
                && name.len() < 128
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "Unsafe case path"
        );
        runs.push((
            artifact(&directory.join(format!("{name}-evidence.json")), &mut total)?,
            artifact(
                &directory.join(format!("{name}-provenance.json")),
                &mut total,
            )?
            .value,
        ));
    }
    Ok(Inputs {
        contract,
        trace,
        evidence,
        proof,
        runs,
    })
}

fn compare(corpus: &Corpus) -> Result<Value> {
    let mut compared = Vec::new();
    let mut boundaries = Vec::new();
    let mut target_count = 0;
    let mut setup_count = 0;
    for (index, case) in corpus.cases.iter().enumerate() {
        let directory = tempfile::tempdir()?;
        let root = fs::canonicalize(directory.path())?;
        for (name, text) in FILES {
            fs::write(root.join(name), text)?;
        }
        let mut native = Replay::new(root);
        let name = case["name"].as_str().context("Case name missing")?;
        let close = case["closePolicy"]
            .as_str()
            .context("Close policy missing")?;
        for (step, gesture) in array(&case["setup"], 32)?.iter().enumerate() {
            native
                .setup(gesture)
                .with_context(|| format!("Case{name} setup{step} {gesture}"))?;
            let actual = native.snapshot("setup", None)?;
            let mut observed = projection(
                &corpus.raw[index]["setup"]["operations"][step]["settlement"]["observed"],
            );
            observed["action"] = json!("setup");
            ensure!(
                actual == observed,
                "Case{name} setup{step} {gesture}: native={actual}; reference={observed}"
            );
            setup_count += 1;
        }
        let mut observations = vec![native.snapshot("initial", None)?];
        for (step, gesture) in array(&case["steps"], 8)?.iter().enumerate() {
            native
                .step(gesture, close)
                .with_context(|| format!("Case{name} target{step} {gesture}"))?;
            let action = if gesture.get("open").is_some() {
                "vscode.open"
            } else {
                gesture["command"].as_str().unwrap()
            };
            observations.push(native.snapshot(action, Some(gesture))?);
        }
        let expected = array(&corpus.rows[index]["observations"], 9)?;
        ensure!(
            expected.len() == observations.len(),
            "Replay dropped target frames"
        );
        for (frame, (actual, observed)) in observations.iter().zip(expected).enumerate() {
            let mut expected = projection(observed);
            let empty = observed["groups"] == json!([{"viewColumn":1,"active":true,"tabs":[]}]);
            if empty {
                ensure!(
                    EMPTY_BOUNDARIES.contains(&name)
                        && frame == 1
                        && actual["groups"] == json!([])
                        && observed["active"].is_null()
                        && observed["visible"] == json!([]),
                    "Unexpected empty-group qualification boundary"
                );
                boundaries.push(json!({"case":name,"frame":frame,"scope":"Upstream one empty active group, native zero groups; absent editors and every text/disk still compared","rawGroups":observed["groups"],"nativeGroups":actual["groups"]}));
                expected["groups"] = json!([]);
            }
            ensure!(
                *actual == expected,
                "Case{name} frame{frame} original{}: native={actual}; reference={expected}",
                observed["action"]
            );
            target_count += 1;
        }
        compared.push(json!({"name":name,"observations":observations}));
    }
    ensure!(
        target_count == 65 && setup_count == 91 && boundaries.len() == 4,
        "Complete comparison counts/boundaries differ"
    );
    Ok(
        json!({"platform":corpus.platform,"caseCount":18,"targetSnapshotCount":target_count,"setupSnapshotCount":setup_count,
        "compared":compared,"emptyGroupBoundaries":boundaries,"scope":"Groups and real file Documents only; public allocation/cache/event/virtual-model evidence verified but outside equality; no App save ownership, graphical mouse, terminal key/UI or sticky persistence parity"}),
    )
}
fn main() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() == 1 && args[0] == "--verify-monotonic-source" {
        verify_sources(SourceContract::Monotonic, &mut 0)?;
        println!("Verified all eight frozen monotonic sticky inputs");
        return Ok(());
    }
    let inputs = if args.is_empty() {
        current_baseline()?
    } else if args.first().is_some_and(|arg| arg == "--monotonic") {
        ensure!(
            args.len() == 2,
            "Monotonic capture requires its explicit full directory"
        );
        load_with_contract(&PathBuf::from(&args[1]), SourceContract::Monotonic)?
    } else {
        ensure!(
            args.len() == 1
                && args
                    .first()
                    .is_none_or(|arg| !arg.to_string_lossy().starts_with("--")),
            "Expected no arguments, a full historical directory or --monotonic <full current directory>"
        );
        load(&PathBuf::from(&args[0]))?
    };
    // Preflight ALL artifacts and late fields before temporary fixture writes.
    let corpus = validate(&inputs)?;
    println!("{}", serde_json::to_string_pretty(&compare(&corpus)?)?);
    Ok(())
}

fn validate(inputs: &Inputs) -> Result<Corpus> {
    let sources = inputs.contract.sources();
    let case_source: Value = serde_json::from_str(CASE_SOURCE)?;
    ensure!(
        hash(CASE_SOURCE.as_bytes()) == SOURCES[0].1,
        "Compiled immutable cases changed"
    );
    let cases = array(&case_source["cases"], 18)?;
    let rows = array(&inputs.trace.value, 18)?;
    let raw = array(&inputs.evidence.value, 18)?;
    let proof = &inputs.proof;
    keys(
        proof,
        &[
            "version",
            "commit",
            "platform",
            "architecture",
            "caseCount",
            "snapshotCount",
            "setupObservationCount",
            "traceSha256",
            "evidenceSha256",
            "casesSha256",
            "launchedExecutableSha256",
            "files",
            "runs",
        ],
        &[],
    )?;
    ensure!(
        proof["version"] == "1.95.0" && proof["commit"] == COMMIT,
        "Pinned identity differs"
    );
    ensure!(
        cases.len() == 18
            && rows.len() == 18
            && raw.len() == 18
            && inputs.runs.len() == 18
            && proof["caseCount"] == 18
            && proof["snapshotCount"] == 65
            && proof["setupObservationCount"] == 91,
        "Complete corpus differs"
    );
    ensure!(
        proof["traceSha256"] == hash(&inputs.trace.bytes)
            && proof["evidenceSha256"] == hash(&inputs.evidence.bytes)
            && proof["casesSha256"] == SOURCES[0].1,
        "Aggregate digest differs"
    );
    let platform = proof["platform"].as_str().context("Platform missing")?;
    let arch = proof["architecture"]
        .as_str()
        .context("Architecture missing")?;
    let (product_hash, executable_hash) = product_identity(platform, arch)?;
    ensure!(
        proof["launchedExecutableSha256"] == executable_hash,
        "Actual launcher digest differs"
    );
    let run_proofs = array(&proof["runs"], 18)?;
    ensure!(run_proofs.len() == 18, "Run proof count differs");
    let mut fixture_inventory = Vec::new();
    let mut targets = 0;
    let mut setups = 0;
    for (index, case) in cases.iter().enumerate() {
        let checked = (|| -> Result<()> {
            let row = &rows[index];
            let evidence = &raw[index];
            let (individual, run) = &inputs.runs[index];
            keys(row, &["name", "observations"], &[])?;
            keys(
                evidence,
                &[
                    "name",
                    "setup",
                    "observations",
                    "settlements",
                    "details",
                    "events",
                    "supplementalEvents",
                    "scope",
                ],
                &[],
            )?;
            keys(
                run,
                &[
                    "version",
                    "commit",
                    "platform",
                    "architecture",
                    "observedAt",
                    "name",
                    "productSha256",
                    "launchedExecutableSha256",
                    "evidenceSha256",
                    "sources",
                    "profileSettingsSha256",
                    "closePolicy",
                    "scope",
                ],
                &[],
            )?;
            ensure!(
                case["name"] == row["name"]
                    && case["name"] == evidence["name"]
                    && case["name"] == run["name"],
                "Ordered case differs"
            );
            ensure!(
                *run == run_proofs[index]
                    && individual.value == *evidence
                    && run["evidenceSha256"] == hash(&individual.bytes),
                "Individual/aggregate evidence differs"
            );
            ensure!(
                ["version", "commit", "platform", "architecture"]
                    .iter()
                    .all(|key| run[*key] == proof[*key])
                    && run["productSha256"] == product_hash
                    && run["launchedExecutableSha256"] == executable_hash,
                "Run product proof differs"
            );
            ensure!(
                run["observedAt"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.len() < 64),
                "Actual run timestamp missing"
            );
            ensure!(
                digest(&run["profileSettingsSha256"])
                    && run["scope"] == SCOPE
                    && evidence["scope"] == SCOPE,
                "Run scope/profile invalid"
            );
            let hashes = run["sources"].as_object().context("Sources missing")?;
            ensure!(
                hashes.len() == sources.len()
                    && sources
                        .iter()
                        .all(|(name, digest)| hashes[*name] == *digest),
                "Run source proof differs"
            );
            let close = case["closePolicy"].as_str().context("Policy missing")?;
            ensure!(
                matches!(close, "keyboardAndMouse" | "keyboard" | "mouse" | "never")
                    && run["closePolicy"] == close
                    && run["profileSettingsSha256"] == profile_hash(&case_source["policy"], close)?,
                "Effective profile/policy differs"
            );
            let setup_steps = array(&case["setup"], 32)?;
            let steps = array(&case["steps"], 8)?;
            let setup = &evidence["setup"];
            keys(
                setup,
                &[
                    "commandInventory",
                    "effectiveConfiguration",
                    "closePolicy",
                    "scope",
                    "operations",
                ],
                &[],
            )?;
            let mut effective = case_source["policy"].clone();
            effective["workbench.editor.preventPinnedEditorClose"] = json!(close);
            ensure!(
                setup["scope"] == SETUP_SCOPE
                    && setup["closePolicy"] == close
                    && setup["effectiveConfiguration"] == effective,
                "Independent setup differs"
            );
            let mut inventory = vec![json!("vscode.open")];
            for step in setup_steps.iter().chain(steps) {
                if let Some(command) = step.get("command")
                    && !inventory.contains(command)
                {
                    inventory.push(command.clone());
                }
            }
            ensure!(
                setup["commandInventory"] == json!(inventory),
                "Original inventory differs"
            );
            let events = array(&evidence["events"], 1024)?;
            validate_events(&evidence["events"])?;
            let supplemental = array(&evidence["supplementalEvents"], 256)?;
            validate_supplemental(
                &evidence["supplementalEvents"],
                events.len(),
                setup_steps.len(),
                steps.len(),
            )?;
            let operations = array(&setup["operations"], 32)?;
            ensure!(
                operations.len() == setup_steps.len(),
                "Setup operation count differs"
            );
            let mut normal_cursor = 0;
            let mut supplemental_cursor = 0;
            for (position, (operation, gesture)) in operations.iter().zip(setup_steps).enumerate() {
                keys(
                    operation,
                    &[
                        "step",
                        "gesture",
                        "settlement",
                        "events",
                        "supplementalEvents",
                    ],
                    &[],
                )?;
                ensure!(
                    operation["step"] == position && operation["gesture"] == *gesture,
                    "Original setup gesture differs"
                );
                validate_settlement(&operation["settlement"], inputs.contract)?;
                validate_events(&operation["events"])?;
                ordered_slice(
                    events,
                    array(&operation["events"], 1024)?,
                    &mut normal_cursor,
                )?;
                let local = array(&operation["supplementalEvents"], 256)?;
                ensure!(
                    local
                        .iter()
                        .all(|event| event["phase"] == json!({"kind":"setup","index":position})),
                    "Setup supplemental phase differs"
                );
                ordered_slice(supplemental, local, &mut supplemental_cursor)?;
            }
            let snapshots = array(&row["observations"], 9)?;
            let settlements = array(&evidence["settlements"], 9)?;
            let details = array(&evidence["details"], 8)?;
            ensure!(
                row["observations"] == evidence["observations"]
                    && snapshots.len() == steps.len() + 1
                    && settlements.len() == snapshots.len()
                    && details.len() == steps.len(),
                "Target evidence inventory differs"
            );
            for (frame, (snapshot, settlement)) in snapshots.iter().zip(settlements).enumerate() {
                let framed = (|| -> Result<()> {
                    validate_snapshot(snapshot)?;
                    validate_settlement(settlement, inputs.contract)?;
                    ensure!(
                        state(snapshot)? == settlement["observed"],
                        "Snapshot/settlement differs"
                    );
                    if frame == 0 {
                        ensure!(
                            snapshot["action"] == "initial"
                                && snapshot.get("opened").is_none()
                                && snapshot.get("requestedPreview").is_none(),
                            "Initial gesture differs"
                        );
                    } else {
                        let step = &steps[frame - 1];
                        let detail = &details[frame - 1];
                        keys(
                            detail,
                            &["step", "gesture", "events", "supplementalEvents"],
                            &[],
                        )?;
                        ensure!(
                            detail["step"] == frame - 1 && detail["gesture"] == *step,
                            "Original target gesture differs"
                        );
                        if let Some(open) = step.get("open") {
                            ensure!(
                                snapshot["action"] == "vscode.open"
                                    && snapshot["opened"] == *open
                                    && snapshot["requestedPreview"] == step["preview"],
                                "Original open mode differs"
                            );
                        } else {
                            ensure!(
                                snapshot["action"] == step["command"]
                                    && snapshot.get("opened").is_none()
                                    && snapshot.get("requestedPreview").is_none(),
                                "Original target command differs"
                            );
                        }
                        validate_events(&detail["events"])?;
                        ordered_slice(events, array(&detail["events"], 1024)?, &mut normal_cursor)?;
                        let local = array(&detail["supplementalEvents"], 256)?;
                        ensure!(
                            local
                                .iter()
                                .all(|event| event["phase"]
                                    == json!({"kind":"target","index":frame-1})),
                            "Target supplemental phase differs"
                        );
                        ordered_slice(supplemental, local, &mut supplemental_cursor)?;
                        if matches!(
                            step["command"].as_str(),
                            Some(
                                "workbench.action.closeActiveEditor"
                                    | "workbench.action.closeActivePinnedEditor"
                                    | "workbench.action.closeEditorsInGroup"
                                    | "workbench.action.closeAllEditors"
                            )
                        ) {
                            ensure!(
                                array(&snapshots[frame - 1]["documents"], 8)?
                                    .iter()
                                    .all(|d| d["dirty"] == false),
                                "Close was not independently clean"
                            );
                        }
                    }
                    Ok(())
                })();
                framed.with_context(|| {
                    format!("Targetframe{frame}, gesture{}", snapshot["action"])
                })?;
            }
            targets += snapshots.len();
            setups += operations.len();
            for (name, text) in FILES {
                fixture_inventory.push(
                    json!({"case":case["name"],"resource":name,"sha256":hash(text.as_bytes())}),
                );
            }
            Ok(())
        })();
        checked.with_context(|| format!("Case{}", case["name"]))?;
    }
    ensure!(
        targets == 65 && setups == 91 && proof["files"] == json!(fixture_inventory),
        "Complete fixture/frame counts differ"
    );
    Ok(Corpus {
        cases: cases.to_vec(),
        rows: rows.to_vec(),
        raw: raw.to_vec(),
        platform: platform.into(),
    })
}

fn editor_projection(editor: &Value) -> Value {
    if editor.is_null() {
        return Value::Null;
    }
    json!({
        "resource":editor["resource"], "viewColumn":editor["viewColumn"], "text":editor["text"],
        "dirty":editor["dirty"], "eol":editor["eol"], "selections":editor["selections"]
    })
}
fn projection(snapshot: &Value) -> Value {
    let documents: Vec<_> = snapshot["documents"].as_array().unwrap().iter().map(|doc| json!({"resource":doc["resource"],"text":doc["text"],"dirty":doc["dirty"],"disk":doc["disk"]})).collect();
    let visible: Vec<_> = snapshot["visible"]
        .as_array()
        .unwrap()
        .iter()
        .map(editor_projection)
        .collect();
    let mut value = json!({"action":snapshot["action"],"groups":snapshot["groups"],"active":editor_projection(&snapshot["active"]),"visible":visible,"documents":documents});
    if snapshot.get("opened").is_some() {
        value["opened"] = snapshot["opened"].clone();
        value["requestedPreview"] = snapshot["requestedPreview"].clone();
    }
    value
}

struct Replay {
    groups: Groups,
    documents: BTreeMap<u64, (String, Document)>,
    resources: BTreeMap<String, u64>,
    root: PathBuf,
}
impl Replay {
    fn new(root: PathBuf) -> Self {
        Self {
            groups: Groups::default(),
            documents: BTreeMap::new(),
            resources: BTreeMap::new(),
            root,
        }
    }
    fn resource(&self, id: u64) -> &str {
        &self.documents[&id].0
    }
    fn document(&mut self, name: &str) -> Result<u64> {
        if let Some(id) = self.resources.get(name) {
            return Ok(*id);
        }
        ensure!(
            FILES.iter().any(|(resource, _)| *resource == name),
            "Native open outside fixtures"
        );
        let doc = Document::open(&self.root.join(name))?;
        let id = doc.id;
        self.documents.insert(id, (name.into(), doc));
        self.resources.insert(name.into(), id);
        Ok(id)
    }
    fn activate(&mut self, change: &Change) -> Result<()> {
        for removed in &change.removed {
            self.documents
                .get_mut(&removed.document)
                .context("Removed model missing")?
                .1
                .remove_view(removed.group.value());
        }
        let Some(active) = change.active else {
            return Ok(());
        };
        self.documents
            .get_mut(&active.document)
            .context("Active model missing")?
            .1
            .activate_view(active.group.value());
        Ok(())
    }
    fn open(&mut self, name: &str, preview: bool) -> Result<()> {
        let id = self.document(name)?;
        let group = self.groups.active_group();
        let existing = group.is_some_and(|group| {
            self.groups
                .memberships(id)
                .any(|member| member.group == group)
        });
        let replacement = if preview && !existing {
            self.groups
                .active_group()
                .and_then(|group| self.groups.group(group))
                .and_then(|group| group.preview())
                .filter(|member| {
                    let doc = &self.documents[&member.document].1;
                    !doc.dirty() && doc.path.is_some()
                })
        } else {
            None
        };
        let change = self.groups.open_mode(
            id,
            if preview {
                OpenMode::Preview
            } else {
                OpenMode::Committed
            },
            replacement,
        )?;
        self.activate(&change)?;
        if !existing {
            self.documents
                .get_mut(&id)
                .unwrap()
                .1
                .set_selections(vec![Selection::caret(0)]);
        }
        Ok(())
    }
    fn setup(&mut self, step: &Value) -> Result<()> {
        if step.get("api").is_none() {
            return self.step(step, "keyboardAndMouse");
        }
        let column = usize::try_from(step["group"].as_u64().context("Setup column missing")?)?;
        let name = step["resource"]
            .as_str()
            .context("Setup resource missing")?;
        let preview = step["preview"].as_bool().context("Setup mode missing")?;
        if column == self.groups.groups().len() + 1 && !self.groups.groups().is_empty() {
            // Fixed setup adapter, never an invented target command. Remove
            // transient copied membership before comparing final setup state.
            let change = self.groups.split_active()?;
            let copied = change.active.context("Setup split lost membership")?;
            self.activate(&change)?;
            self.open(name, preview)?;
            ensure!(
                self.groups
                    .active_membership()
                    .is_some_and(|m| m.document != copied.document),
                "Setup new-group adapter requires distinct resource"
            );
            let removed = self.groups.close(copied)?;
            self.activate(&removed)?;
        } else {
            if !self.groups.groups().is_empty() {
                let id = self
                    .groups
                    .groups()
                    .get(column.checked_sub(1).context("Setup column zero")?)
                    .context("Setup group missing")?
                    .id();
                let change = self.groups.focus_group(id)?;
                self.activate(&change)?;
            } else {
                ensure!(column == 1, "First setup group must be1");
            }
            self.open(name, preview)?;
        }
        if let Some(selection) = step.get("selection") {
            let active = self
                .groups
                .active_membership()
                .context("Setup has no editor")?;
            let cursor = usize::try_from(
                selection["cursor"]
                    .as_u64()
                    .context("Setup cursor missing")?,
            )?;
            let anchor = usize::try_from(
                selection["anchor"]
                    .as_u64()
                    .context("Setup anchor missing")?,
            )?;
            self.documents
                .get_mut(&active.document)
                .unwrap()
                .1
                .set_selections(vec![Selection {
                    cursor,
                    anchor: Some(anchor),
                    desired_column: None,
                }]);
        }
        Ok(())
    }
    fn close_subset(&mut self, group: vscli::editor_groups::GroupId) -> Result<()> {
        let proof = self.groups.group_proof(group)?;
        let targets = self
            .groups
            .group(group)
            .context("Close group missing")?
            .tabs()
            .iter()
            .filter(|tab| !tab.is_sticky())
            .map(|tab| Membership {
                group,
                tab: tab.id(),
                document: tab.document(),
            })
            .collect::<Vec<_>>();
        ensure!(
            targets
                .iter()
                .all(|m| !self.documents[&m.document].1.dirty()),
            "Native batch target dirty"
        );
        let change = self.groups.close_memberships(&proof, &targets)?;
        self.activate(&change)
    }
    fn step(&mut self, step: &Value, close_policy: &str) -> Result<()> {
        if let Some(name) = step["open"].as_str() {
            return self.open(
                name,
                step["preview"]
                    .as_bool()
                    .context("Original open mode missing")?,
            );
        }
        let active = self
            .groups
            .active_membership()
            .context("Original target has no editor")?;
        let change = match step["command"].as_str().context("Command missing")? {
            "workbench.action.pinEditor" => Some(self.groups.set_sticky(active, true)?),
            "workbench.action.unpinEditor" => Some(self.groups.set_sticky(active, false)?),
            "workbench.action.focusFirstEditorGroup"
            | "workbench.action.focusSecondEditorGroup" => {
                let index =
                    usize::from(step["command"] == "workbench.action.focusSecondEditorGroup");
                let id = self
                    .groups
                    .groups()
                    .get(index)
                    .context("Focus group missing")?
                    .id();
                Some(self.groups.focus_group(id)?)
            }
            "workbench.action.splitEditor" => Some(self.groups.split_active()?),
            "type" => {
                self.documents.get_mut(&active.document).unwrap().1.insert(
                    step["args"]["text"]
                        .as_str()
                        .context("Typed text missing")?,
                    false,
                );
                self.groups.promote_document(active.document)?;
                None
            }
            "undo" | "redo" => {
                let doc = &mut self.documents.get_mut(&active.document).unwrap().1;
                if step["command"] == "undo" {
                    doc.undo();
                } else {
                    doc.redo();
                }
                if doc.dirty() {
                    self.groups.promote_document(active.document)?;
                }
                None
            }
            "workbench.action.closeActiveEditor" | "workbench.action.closeActivePinnedEditor" => {
                ensure!(
                    !self.documents[&active.document].1.dirty(),
                    "Native active-close target dirty"
                );
                let sticky = self
                    .groups
                    .group(active.group)
                    .unwrap()
                    .tabs()
                    .iter()
                    .find(|t| t.id() == active.tab)
                    .unwrap()
                    .is_sticky();
                let protected = step["command"] == "workbench.action.closeActiveEditor"
                    && sticky
                    && matches!(close_policy, "keyboardAndMouse" | "keyboard");
                if protected {
                    self.groups
                        .next_nonsticky_recent(active.group)
                        .or_else(|| self.groups.next_nonsticky_recent_any_group())
                        .map(|member| self.groups.focus(member))
                        .transpose()?
                } else {
                    Some(self.groups.close(active)?)
                }
            }
            "workbench.action.closeEditorsInGroup" => {
                self.close_subset(active.group)?;
                None
            }
            "workbench.action.closeAllEditors" => {
                let ids = self
                    .groups
                    .groups()
                    .iter()
                    .map(|g| g.id())
                    .collect::<Vec<_>>();
                for id in ids {
                    self.close_subset(id)?;
                }
                None
            }
            command => bail!("Unqualified native sticky command: {command}"),
        };
        if let Some(change) = change {
            self.activate(&change)?;
        }
        Ok(())
    }
    fn editor(&self, member: Membership, column: usize) -> Result<Value> {
        let (resource, doc) = &self.documents[&member.document];
        let view = doc.view_state(Some(member.group.value()));
        let text = doc.text.to_string();
        let mut selections = vec![Selection {
            cursor: view.cursor,
            anchor: view.anchor,
            desired_column: None,
        }];
        selections.extend(view.secondary.clone());
        let selections: Vec<_> = selections.iter().map(|selection| Ok(json!({"anchor":point(&text,selection.anchor.unwrap_or(selection.cursor))?,"cursor":point(&text,selection.cursor)?}))).collect::<Result<_>>()?;
        Ok(
            json!({"resource":resource,"viewColumn":column,"text":text,"dirty":doc.api_dirty(),"eol":if doc.eol == "\r\n" {"CRLF"} else {"LF"},"selections":selections}),
        )
    }
    fn snapshot(&self, action: &str, step: Option<&Value>) -> Result<Value> {
        let mut groups = Vec::new();
        let mut visible = Vec::new();
        let mut active = Value::Null;
        for (index, group) in self.groups.groups().iter().enumerate() {
            let tabs: Vec<_> = group.tabs().iter().map(|tab| json!({"resource":self.resource(tab.document()),"active":group.active().unwrap().id()==tab.id(),"dirty":self.documents[&tab.document()].1.dirty(),"pinned":tab.is_sticky(),"preview":tab.is_preview()})).collect();
            groups.push(json!({"viewColumn":index+1,"active":self.groups.active_group()==Some(group.id()),"tabs":tabs}));
            let tab = group.active().unwrap();
            let editor = self.editor(
                Membership {
                    group: group.id(),
                    tab: tab.id(),
                    document: tab.document(),
                },
                index + 1,
            )?;
            if self.groups.active_group() == Some(group.id()) {
                active = editor.clone();
            }
            visible.push(editor);
        }
        let mut documents = Vec::new();
        for (name, initial) in FILES {
            let (text, dirty) = self
                .resources
                .get(name)
                .map_or((initial.into(), false), |id| {
                    let doc = &self.documents[id].1;
                    (doc.text.to_string(), doc.api_dirty())
                });
            let disk = fs::read_to_string(self.root.join(name))?;
            ensure!(
                disk == initial,
                "Native sticky journey changed fixture disk"
            );
            documents.push(json!({"resource":name,"text":text,"dirty":dirty,"disk":disk}));
        }
        let mut value = json!({"action":action,"groups":groups,"active":active,"visible":visible,"documents":documents});
        if let Some(step) = step
            && step.get("open").is_some()
        {
            value["opened"] = step["open"].clone();
            value["requestedPreview"] = step.get("preview").cloned().unwrap_or(json!(true));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn genuine_three_platform_archives_preserve_complete_inputs_and_outputs() {
        let archives = [
            (
                "linux",
                11677123920u64,
                "67269dabcfc88b0c69917a464b9c83349348068854f2c5118be1b976e1436d41",
            ),
            (
                "darwin",
                11677184243,
                "40001d94a54321e76a9fd379c5e999eccfdc7ce71b36305cbf4cab43b64f0984",
            ),
            (
                "win32",
                11677436283,
                "9c711b9263472047fde9b8b376db27747e8cfe01ed053c4fd42dd9e4ab2f38b0",
            ),
        ];
        for (platform, artifact, inventory_hash) in archives {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(MONOTONIC_ARCHIVE)
                .join(platform);
            let mut total = 0;
            let bytes = read(&root.join("inventory.json"), &mut total).unwrap();
            assert_eq!(hash(&bytes), inventory_hash);
            let inventory = parsed(&bytes).unwrap();
            assert_eq!(inventory["run"], 38071676391u64);
            assert_eq!(
                inventory["head"],
                "3e813500114f6c30d9e00fb15d54d5b85e1fb69b"
            );
            assert_eq!(inventory["artifact"], artifact);
            assert_eq!(inventory["platform"], platform);
            assert_eq!(inventory["fileCount"], 49);
            let files = inventory["files"].as_array().unwrap();
            assert_eq!(files.len(), 49);
            let mut original_bytes = 0u64;
            for file in files {
                let path = Path::new(file["path"].as_str().unwrap());
                assert!(!path.is_absolute());
                assert!(
                    path.components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))
                );
                let bytes = read(&root.join(path), &mut total).unwrap();
                assert_eq!(file["bytes"], bytes.len());
                assert_eq!(file["sha256"], hash(&bytes));
                original_bytes += bytes.len() as u64;
            }
            assert_eq!(inventory["bytes"], original_bytes);
            for (name, expected) in SourceContract::Monotonic.sources() {
                let path = root.join(MONOTONIC_ROOT).join(name);
                assert_eq!(hash(&read(&path, &mut total).unwrap()), expected);
            }
            // These byte/source checks do not bypass the pinned launcher guard
            // needed for full Mac/Windows artifact admission and native replay.
        }
    }
    #[test]
    fn genuine_monotonic_linux_baseline_admits_and_replays_every_original_frame() {
        let inputs = current_baseline().unwrap();
        let corpus = validate(&inputs).unwrap();
        let actual = compare(&corpus).unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(MONOTONIC_ARCHIVE)
            .join("linux/target/vscode-reference/result/editor-sticky-tabs/native-comparison.json");
        let expected = parsed(&read(&path, &mut 0).unwrap()).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["caseCount"], 18);
        assert_eq!(actual["targetSnapshotCount"], 65);
        assert_eq!(actual["setupSnapshotCount"], 91);
        assert_eq!(actual["emptyGroupBoundaries"].as_array().unwrap().len(), 4);

        // A genuine current receipt never selects its own source contract.
        let mut historical = inputs;
        historical.contract = SourceContract::Historical;
        assert!(validate(&historical).is_err());
    }
    fn baseline() -> Inputs {
        load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/vscode-reference/baselines/1.95.0/editor-sticky-tabs-observer-corrected",
            ),
        )
        .unwrap()
    }
    fn bytes(value: &Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(value).unwrap();
        bytes.push(b'\n');
        bytes
    }
    fn reseal(inputs: &mut Inputs) {
        inputs.trace.bytes = bytes(&inputs.trace.value);
        inputs.evidence.bytes = bytes(&inputs.evidence.value);
        inputs.proof["traceSha256"] = json!(hash(&inputs.trace.bytes));
        inputs.proof["evidenceSha256"] = json!(hash(&inputs.evidence.bytes));
        for i in 0..18 {
            inputs.runs[i].0.value = inputs.evidence.value[i].clone();
            inputs.runs[i].0.bytes = bytes(&inputs.runs[i].0.value);
            inputs.runs[i].1["evidenceSha256"] = json!(hash(&inputs.runs[i].0.bytes));
            inputs.proof["runs"][i] = inputs.runs[i].1.clone();
        }
    }
    fn synthetic_monotonic_metadata() -> Inputs {
        // Explicitly synthetic source-routing fixture, derived from genuine
        // historical bytes. This is never published as a corrected capture.
        let mut inputs = baseline();
        inputs.contract = SourceContract::Monotonic;
        for i in 0..18 {
            inputs.runs[i].1["sources"]["editor-sticky-tabs-suite.cjs"] = json!(MONOTONIC_SUITE);
            inputs.proof["runs"][i] = inputs.runs[i].1.clone();
        }
        inputs
    }
    #[test]
    fn source_contracts_bind_distinct_fixed_roots_without_hash_self_selection() {
        assert_ne!(
            SourceContract::Historical.root(),
            SourceContract::Monotonic.root()
        );
        let historical = SourceContract::Historical.sources();
        let current = SourceContract::Monotonic.sources();
        assert_eq!(historical.len(), 8);
        assert_eq!(current.len(), 8);
        assert_eq!(historical[2].0, "editor-sticky-tabs-suite.cjs");
        for i in 0..8 {
            if i == 2 {
                assert_ne!(historical[i].1, current[i].1);
                assert_eq!(current[i].1, MONOTONIC_SUITE);
            } else {
                assert_eq!(historical[i], current[i]);
            }
        }
        verify_sources(SourceContract::Historical, &mut 0).unwrap();
        verify_sources(SourceContract::Monotonic, &mut 0).unwrap();
    }
    #[test]
    fn historical_and_synthetic_current_metadata_cannot_cross_source_contracts() {
        let mut historical = baseline();
        validate(&historical).unwrap();
        historical.contract = SourceContract::Monotonic;
        assert!(
            format!("{:#}", validate(&historical).err().unwrap())
                .contains("Run source proof differs")
        );
        let mut current = synthetic_monotonic_metadata();
        validate(&current).unwrap();
        current.contract = SourceContract::Historical;
        assert!(
            format!("{:#}", validate(&current).err().unwrap()).contains("Run source proof differs")
        );
        for index in [0, 17] {
            let mut current = synthetic_monotonic_metadata();
            current.runs[index].1["sources"]["editor-sticky-tabs-suite.cjs"] = json!(SOURCES[2].1);
            current.proof["runs"][index] = current.runs[index].1.clone();
            assert!(
                format!("{:#}", validate(&current).err().unwrap())
                    .contains("Run source proof differs")
            );
        }
    }
    #[test]
    fn monotonic_settlement_bounds_are_stricter_without_rewriting_historical_limits() {
        let inputs = baseline();
        let mut settlement =
            inputs.evidence.value[0]["setup"]["operations"][0]["settlement"].clone();
        for elapsed in [100, 2999] {
            settlement["elapsedMs"] = json!(elapsed);
            validate_settlement(&settlement, SourceContract::Monotonic).unwrap();
            validate_settlement(&settlement, SourceContract::Historical).unwrap();
        }
        for elapsed in [3000, 3100] {
            settlement["elapsedMs"] = json!(elapsed);
            assert!(validate_settlement(&settlement, SourceContract::Monotonic).is_err());
            validate_settlement(&settlement, SourceContract::Historical).unwrap();
        }
        for elapsed in [99, 3101, 13721] {
            settlement["elapsedMs"] = json!(elapsed);
            assert!(validate_settlement(&settlement, SourceContract::Monotonic).is_err());
            assert!(validate_settlement(&settlement, SourceContract::Historical).is_err());
        }
    }
    #[test]
    fn coherently_resealed_late_setup_or_target_cannot_pass_current_preflight() {
        for setup in [true, false] {
            let mut inputs = synthetic_monotonic_metadata();
            if setup {
                inputs.evidence.value[17]["setup"]["operations"][4]["settlement"]["elapsedMs"] =
                    json!(13721);
            } else {
                inputs.evidence.value[17]["settlements"][1]["elapsedMs"] = json!(3000);
            }
            reseal(&mut inputs);
            assert!(
                format!("{:#}", validate(&inputs).err().unwrap()).contains("Settlement differs")
            );
        }
    }
    #[test]
    fn genuine_windows_late_setup_is_rejected_by_the_offline_settlement_guard() {
        let archive = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/vscode-reference/observations/1.95.0/sticky-settlement-deadline/951f2ad-win32",
        );
        for (name, expected) in SOURCES {
            assert_eq!(
                hash(&read(&archive.join("sources").join(name), &mut 0).unwrap()),
                expected
            );
        }
        let inputs = load(&archive.join("result")).unwrap();
        assert_eq!(inputs.proof["platform"], "win32");
        assert_eq!(inputs.proof["caseCount"], 18);
        let case = inputs
            .evidence
            .value
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "close-editors-in-group-excludes-sticky")
            .unwrap();
        assert_eq!(
            case["setup"]["operations"][0]["settlement"]["elapsedMs"],
            13721
        );
        assert_eq!(case["setup"]["operations"][0]["settlement"]["reads"], 5);
        assert_eq!(case["settlements"][1]["elapsedMs"], 133);
        assert_eq!(inputs.proof["traceSha256"], hash(&inputs.trace.bytes));
        assert_eq!(inputs.proof["evidenceSha256"], hash(&inputs.evidence.bytes));
        for (i, (evidence, proof)) in inputs.runs.iter().enumerate() {
            assert_eq!(inputs.proof["runs"][i], *proof);
            assert_eq!(proof["evidenceSha256"], hash(&evidence.bytes));
            assert_eq!(evidence.value, inputs.evidence.value[i]);
            assert_eq!(proof["sources"].as_object().unwrap().len(), SOURCES.len());
            for (name, expected) in SOURCES {
                assert_eq!(proof["sources"][name], expected);
            }
        }
        // Full Windows admission additionally verifies its actual product and
        // executable bytes. Those remain a remote Windows gate, not fabricated
        // local fixtures or a relaxed product_identity check.
        for contract in [SourceContract::Historical, SourceContract::Monotonic] {
            let error =
                validate_settlement(&case["setup"]["operations"][0]["settlement"], contract)
                    .err()
                    .expect("Genuine late setup must remain rejected");
            assert_eq!(error.to_string(), "Settlement differs");
            validate_settlement(&case["settlements"][1], contract).unwrap();
        }
    }
    #[test]
    fn compares_all_targets_and_fixed_setup_without_sticky_exclusions() {
        let inputs = baseline();
        let corpus = validate(&inputs).unwrap();
        let report = compare(&corpus).unwrap();
        assert_eq!(report["targetSnapshotCount"], 65);
        assert_eq!(report["setupSnapshotCount"], 91);
        assert_eq!(report["compared"].as_array().unwrap().len(), 18);
        assert_eq!(report["emptyGroupBoundaries"].as_array().unwrap().len(), 4);
    }
    #[test]
    fn wrong_source_product_and_launcher_fail_preflight() {
        for kind in ["source", "product", "launcher"] {
            let mut inputs = baseline();
            match kind {
                "source" => {
                    inputs.runs[0].1["sources"]["editor-sticky-tabs-suite.cjs"] =
                        json!("0".repeat(64))
                }
                "product" => inputs.runs[0].1["productSha256"] = json!("0".repeat(64)),
                _ => inputs.proof["launchedExecutableSha256"] = json!("0".repeat(64)),
            }
            inputs.proof["runs"][0] = inputs.runs[0].1.clone();
            assert!(validate(&inputs).is_err(), "{kind}");
        }
    }
    #[test]
    fn coherent_hashes_cannot_change_original_target_or_mode() {
        let mut inputs = baseline();
        inputs.trace.value[0]["observations"][1]["requestedPreview"] = json!(true);
        inputs.evidence.value[0]["observations"][1]["requestedPreview"] = json!(true);
        reseal(&mut inputs);
        assert!(
            validate(&inputs)
                .err()
                .unwrap()
                .to_string()
                .contains("Case")
        );
        let mut inputs = baseline();
        inputs.trace.value[0]["observations"][2]["action"] = json!("workbench.action.keepEditor");
        inputs.evidence.value[0]["observations"][2]["action"] =
            json!("workbench.action.keepEditor");
        reseal(&mut inputs);
        assert!(validate(&inputs).is_err());
    }
    #[test]
    fn late_invalid_setup_and_changed_policy_refuse_before_replay() {
        let mut inputs = baseline();
        inputs.evidence.value[17]["setup"]["operations"][4]["gesture"]["command"] =
            json!("workbench.action.focusSecondEditorGroup");
        reseal(&mut inputs);
        assert!(validate(&inputs).is_err());
        let mut inputs = baseline();
        inputs.evidence.value[17]["setup"]["effectiveConfiguration"]["workbench.editor.preventPinnedEditorClose"] =
            json!("never");
        reseal(&mut inputs);
        assert!(validate(&inputs).is_err());
    }
    #[test]
    fn missing_protected_case_or_replaced_fixture_cannot_be_hidden() {
        let mut inputs = baseline();
        inputs.trace.value[8] = inputs.trace.value[9].clone();
        inputs.evidence.value[8] = inputs.evidence.value[9].clone();
        reseal(&mut inputs);
        assert!(validate(&inputs).is_err());
        let mut inputs = baseline();
        inputs.trace.value[17]["observations"][1]["documents"][7]["disk"] = json!("foreign");
        inputs.evidence.value[17]["observations"][1]["documents"][7]["disk"] = json!("foreign");
        reseal(&mut inputs);
        assert!(validate(&inputs).is_err());
    }
    #[test]
    fn supplemental_scope_keeps_windows_uri_and_rejects_file_or_bad_phase() {
        let good = json!([{"kind":"document-change","uri":"output:log-C%3A%5Cfixture","scheme":"output","phase":{"kind":"readiness"},
            "fixtureEventCount":0,"documentObject":1,"languageId":"Log","dirty":false,"version":2,
            "changes":[{"text":"猫🙂\r\n","rangeOffset":0,"rangeLength":0}]}]);
        validate_supplemental(&good, 0, 1, 1).unwrap();
        let mut bad = good.clone();
        bad[0]["uri"] = json!("file:///C:/outside.txt");
        bad[0]["scheme"] = json!("file");
        assert!(validate_supplemental(&bad, 0, 1, 1).is_err());
        let mut bad = good.clone();
        bad[0]["phase"] = json!({"kind":"target","index":1});
        assert!(validate_supplemental(&bad, 0, 1, 1).is_err());
        let mut bad = good;
        bad[0]["changes"][0]["text"] = json!("x".repeat(65537));
        assert!(validate_supplemental(&bad, 0, 1, 1).is_err());
    }
    #[test]
    fn unicode_geometry_and_actual_empty_group_boundary_are_strict() {
        let mut inputs = baseline();
        inputs.trace.value[0]["observations"][0]["active"]["selections"][0]["cursor"]["character"] =
            json!(99);
        inputs.evidence.value[0]["observations"][0]["active"]["selections"][0]["cursor"]["character"] =
            json!(99);
        reseal(&mut inputs);
        assert!(validate(&inputs).is_err());
        let inputs = baseline();
        let corpus = validate(&inputs).unwrap();
        for name in EMPTY_BOUNDARIES {
            let row = corpus.rows.iter().find(|r| r["name"] == name).unwrap();
            assert_eq!(
                row["observations"][1]["groups"],
                json!([{"viewColumn":1,"active":true,"tabs":[]}])
            );
        }
    }
}
