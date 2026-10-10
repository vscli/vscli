//! Strict replay of independently captured preview membership workflows.
//!
//! This uses Groups plus real Documents, not App's open/save/loader ownership.
//! Sticky-only cases stay verified raw evidence and are explicitly excluded.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use vscli::{
    document::{Document, Selection},
    editor_groups::{Change, Groups, Membership, Navigate, OpenMode},
};

const ROOT: &str = "tests/vscode-reference";
const COMMIT: &str = "912bb683695358a54ae0c670461738984cbb5b95";
const FILES: [(&str, &str); 4] = [
    ("a.txt", "猫🙂 alpha\r\nline-a\r\n"),
    ("b.txt", "β🙂 beta\r\nline-b\r\n"),
    ("c.txt", "γ🙂 gamma\r\nline-c\r\n"),
    ("d.txt", "δ🙂 delta\r\nline-d\r\n"),
];
const SOURCES: [&str; 9] = [
    "editor-preview-tabs.cjs",
    "editor-preview-tabs-cases.json",
    "editor-preview-tabs-suite.cjs",
    "editor-preview-tabs-run.cjs",
    "editor-preview-tabs-worker.cjs",
    "supervisor.cjs",
    "package-lock.json",
    "package.json",
    "extension.cjs",
];
const EXCLUDED: [&str; 2] = [
    "sticky-pin-unpin-and-committed-distinction",
    "historical-sticky-prefix-and-unpin-placement",
];
const SCOPE: &str = "Preview and sticky text-file/Untitled tabs, explicit default positioning/revealIfOpen/MRU policy, public groups and visible selections; no graphical double-click or private stack access";
const SETUP_SCOPE: &str = "Initial file API setup only; every following tab/group/navigation/edit target uses its original public command once";
const SETTLEMENT_SCOPE: &str = "Awaited command completion then 100ms unchanged public state; no target retry or expected-output predicate";
const MAX_BYTES: u64 = 16 * 1024 * 1024;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Reference is not a regular file");
    ensure!(metadata.len() <= MAX_BYTES, "Reference exceeds byte budget");
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Reference grew beyond byte budget"
    );
    Ok(bytes)
}
fn source(name: &str) -> Result<Vec<u8>> {
    ensure!(SOURCES.contains(&name), "Unknown observer source");
    read(&Path::new(env!("CARGO_MANIFEST_DIR")).join(ROOT).join(name))
}
fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}
fn policy() -> Value {
    json!({
        "workbench.editor.enablePreview":true,
        "workbench.editor.enablePreviewFromQuickOpen":false,
        "workbench.editor.enablePreviewFromCodeNavigation":false,
        "workbench.editor.openPositioning":"right",
        "workbench.editor.focusRecentEditorAfterClose":true,
        "workbench.editor.closeEmptyGroups":true,
        "workbench.editor.revealIfOpen":false
    })
}
fn state_without_gesture(observation: &Value) -> Result<Value> {
    let mut value = observation.clone();
    let object = value
        .as_object_mut()
        .context("Observation must be an object")?;
    object.remove("action");
    object.remove("opened");
    object.remove("requestedPreview");
    Ok(value)
}
fn point(text: &str, scalar: usize) -> Result<Value> {
    ensure!(
        scalar <= text.chars().count(),
        "Selection exceeds scalar length"
    );
    let prefix: String = text.chars().take(scalar).collect();
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let start = prefix.rfind('\n').map_or(0, |position| position + 1);
    let character = prefix[start..].encode_utf16().count();
    Ok(json!({"line":line,"character":character,"scalar":scalar}))
}
fn validate_editor(editor: &Value) -> Result<()> {
    let text = editor["text"].as_str().context("Editor text missing")?;
    ensure!(
        text.len() < 4096
            && editor["dirty"].is_boolean()
            && matches!(editor["eol"].as_str(), Some("CRLF" | "LF")),
        "Editor text, dirty state or EOL invalid"
    );
    ensure!(
        editor["documentObject"].as_u64().is_some_and(|id| id > 0)
            && editor["version"]
                .as_u64()
                .is_some_and(|version| version > 0),
        "Raw public identity/version missing"
    );
    let selections = editor["selections"]
        .as_array()
        .context("Editor selections missing")?;
    ensure!(
        !selections.is_empty() && selections.len() <= 128,
        "Editor selection budget invalid"
    );
    for selection in selections {
        for name in ["anchor", "cursor"] {
            let scalar = usize::try_from(
                selection[name]["scalar"]
                    .as_u64()
                    .context("Selection scalar missing")?,
            )?;
            ensure!(
                selection[name] == point(text, scalar)?,
                "Selection UTF16/scalar evidence inconsistent"
            );
        }
    }
    Ok(())
}
fn validate_snapshot(snapshot: &Value, case: &str, frame: usize) -> Result<()> {
    let groups = snapshot["groups"].as_array().context("Groups missing")?;
    ensure!(
        !groups.is_empty() && groups.len() <= 4,
        "Group bounds invalid"
    );
    ensure!(
        groups
            .iter()
            .filter(|group| group["active"] == true)
            .count()
            == 1,
        "Active group inventory invalid"
    );
    let visible = snapshot["visible"]
        .as_array()
        .context("Visible editors missing")?;
    ensure!(
        visible.len() == groups.len(),
        "Visible editor inventory differs"
    );
    validate_editor(&snapshot["active"])?;
    let documents = snapshot["documents"]
        .as_array()
        .context("Documents missing")?;
    ensure!(
        (4..=5).contains(&documents.len()),
        "Document inventory invalid"
    );
    for (index, (name, text)) in FILES.iter().enumerate() {
        let document = &documents[index];
        ensure!(
            document["resource"] == *name
                && document["disk"] == *text
                && document["text"].is_string()
                && document["dirty"].is_boolean()
                && document["loaded"].is_boolean(),
            "Fixture disk or document evidence differs"
        );
        if document["loaded"] == true {
            ensure!(
                document["documentObject"].as_u64().is_some_and(|id| id > 0)
                    && document["version"]
                        .as_u64()
                        .is_some_and(|version| version > 0),
                "Loaded public document identity missing"
            );
        } else {
            ensure!(
                document["documentObject"].is_null()
                    && document["version"].is_null()
                    && document["dirty"] == false
                    && document["text"] == *text,
                "Unloaded document evidence differs"
            );
        }
    }
    if documents.len() == 5 {
        let untitled = &documents[4];
        ensure!(
            untitled["resource"] == "untitled-1"
                && untitled["loaded"] == true
                && untitled["disk"].is_null(),
            "Untitled inventory invalid"
        );
    }
    for (index, (group, editor)) in groups.iter().zip(visible).enumerate() {
        ensure!(
            group["viewColumn"] == index + 1 && group["active"].is_boolean(),
            "Ordered group identity invalid"
        );
        let tabs = group["tabs"].as_array().context("Tabs missing")?;
        ensure!(
            !tabs.is_empty()
                && tabs.len() <= 8
                && tabs.iter().filter(|tab| tab["active"] == true).count() == 1,
            "Tab inventory invalid"
        );
        ensure!(
            tabs.iter().filter(|tab| tab["preview"] == true).count() <= 1,
            "Multiple previews in one group"
        );
        for tab in tabs {
            ensure!(
                ["active", "dirty", "pinned", "preview"]
                    .iter()
                    .all(|field| tab[*field].is_boolean()),
                "Tab flags invalid"
            );
            let document = documents
                .iter()
                .find(|document| document["resource"] == tab["resource"])
                .context("Tab resource outside fixture")?;
            // The actual Untitled observer exposes independent input and
            // TextDocument dirtiness: after Undo to empty, the input/tab is
            // clean while the public TextDocument stays dirty. Preserve both.
            let captured_untitled_boundary = case == "untitled-retention-after-undo"
                && (5..=7).contains(&frame)
                && document["resource"] == "untitled-1"
                && document["text"] == ""
                && document["dirty"] == true
                && tab["dirty"] == false;
            ensure!(
                document["loaded"] == true
                    && (document["dirty"] == tab["dirty"] || captured_untitled_boundary)
                    && !(tab["preview"] == true && tab["pinned"] == true),
                "Tab/document flags inconsistent"
            );
        }
        validate_editor(editor)?;
        let active_tab = tabs.iter().find(|tab| tab["active"] == true).unwrap();
        ensure!(
            editor["viewColumn"] == index + 1 && editor["resource"] == active_tab["resource"],
            "Visible membership differs"
        );
        let document = documents
            .iter()
            .find(|document| document["resource"] == editor["resource"])
            .unwrap();
        ensure!(
            ["documentObject", "text", "dirty", "version"]
                .iter()
                .all(|field| editor[*field] == document[*field]),
            "Visible/public document identity differs"
        );
        if group["active"] == true {
            ensure!(
                *editor == snapshot["active"],
                "Active editor differs from focused group"
            );
        }
    }
    Ok(())
}

struct Corpus {
    cases: Vec<Value>,
    rows: Vec<Value>,
    platform: String,
}
fn validate(trace: &[u8], evidence: &[u8], proof: &Value) -> Result<Corpus> {
    let case_bytes = source("editor-preview-tabs-cases.json")?;
    let cases: Vec<Value> = serde_json::from_slice(&case_bytes)?;
    let rows: Vec<Value> = serde_json::from_slice(trace)?;
    let raw: Vec<Value> = serde_json::from_slice(evidence)?;
    ensure!(
        proof["version"] == "1.95.0"
            && proof["commit"] == COMMIT
            && matches!(
                proof["platform"].as_str(),
                Some("linux" | "darwin" | "win32")
            )
            && matches!(proof["architecture"].as_str(), Some("x64" | "arm64")),
        "Pinned product identity differs"
    );
    ensure!(proof["traceSha256"] == hash(trace), "Trace digest differs");
    ensure!(
        proof["evidenceSha256"] == hash(evidence),
        "Evidence digest differs"
    );
    ensure!(
        proof["casesSha256"] == hash(&case_bytes),
        "Case source digest differs"
    );
    ensure!(
        cases.len() == 11
            && rows.len() == 11
            && raw.len() == 11
            && proof["caseCount"] == 11
            && proof["snapshotCount"] == 86,
        "Complete corpus inventory differs"
    );
    let runs = proof["runs"].as_array().context("Actual runs missing")?;
    ensure!(runs.len() == 11, "Actual run inventory differs");
    let mut sources = BTreeMap::new();
    for name in SOURCES {
        sources.insert(name, hash(&source(name)?));
    }
    let mut file_inventory = Vec::new();
    let mut snapshot_count = 0;
    let mut excluded_count = 0;
    for (((case, row), raw), run) in cases.iter().zip(&rows).zip(&raw).zip(runs) {
        ensure!(
            case["name"] == row["name"]
                && case["name"] == raw["name"]
                && case["name"] == run["name"],
            "Ordered case identity differs"
        );
        ensure!(
            ["version", "commit", "platform", "architecture"]
                .iter()
                .all(|field| run[*field] == proof[*field]),
            "Actual run product identity differs"
        );
        let hashes = run["sources"]
            .as_object()
            .context("Actual source inventory missing")?;
        ensure!(
            hashes.len() == SOURCES.len(),
            "Actual source inventory differs"
        );
        for (name, hash) in &sources {
            ensure!(
                hashes[*name] == *hash,
                "Observer source digest differs: {name}"
            );
        }
        for field in ["productSha256", "evidenceSha256", "profileSettingsSha256"] {
            ensure!(digest(&run[field]), "Actual run digest invalid: {field}");
        }
        // The frozen observer records product.json, not executable bytes.
        ensure!(
            run["productSha256"] == runs[0]["productSha256"]
                && run["profileSettingsSha256"] == runs[0]["profileSettingsSha256"],
            "Actual product/profile digest differs between runs"
        );
        if proof["platform"] == "linux" && proof["architecture"] == "x64" {
            ensure!(
                run["productSha256"]
                    == "3a9db06699900b7da70447f3e1cc5ca65cdbfceab99e9eb94757ffcbe582e08d",
                "Frozen Linux product digest differs"
            );
        }
        ensure!(
            run["profileSettingsSha256"]
                == "d447a059432c2277a612c46fb5a0520463ba4d1d238d7fdfa472b4397b01e0e5",
            "Actual profile settings digest differs"
        );
        let mut encoded = serde_json::to_vec_pretty(raw)?;
        encoded.push(b'\n');
        ensure!(
            run["evidenceSha256"] == hash(&encoded),
            "Individual run evidence digest differs"
        );
        ensure!(
            raw["scope"] == SCOPE && run["scope"] == SCOPE && raw["setup"]["scope"] == SETUP_SCOPE,
            "Observer scope differs"
        );
        let setup = &raw["setup"];
        ensure!(
            setup["action"] == "api.openTextDocument/showTextDocument"
                && setup["resource"] == "a.txt"
                && setup["effectiveConfiguration"] == policy(),
            "Independent setup/policy differs"
        );
        let steps = case["steps"].as_array().context("Case steps missing")?;
        ensure!(steps.len() <= 8, "Case gesture budget exceeded");
        let mut inventory = vec![json!("vscode.open")];
        for step in steps {
            if let Some(command) = step.get("command")
                && !inventory.contains(command)
            {
                inventory.push(command.clone());
            }
        }
        ensure!(
            setup["commandInventory"] == json!(inventory),
            "Original command inventory differs"
        );
        ensure!(
            row["observations"] == raw["observations"],
            "Raw trace projection differs"
        );
        let snapshots = row["observations"]
            .as_array()
            .context("Observations missing")?;
        let settlements = raw["settlements"]
            .as_array()
            .context("Settlements missing")?;
        let details = raw["details"]
            .as_array()
            .context("Gesture evidence missing")?;
        ensure!(
            snapshots.len() == steps.len() + 1
                && settlements.len() == snapshots.len()
                && details.len() == steps.len(),
            "Gesture/snapshot inventory differs"
        );
        ensure!(
            setup["settlement"] == settlements[0] && snapshots[0]["action"] == "initial",
            "Initial setup differs"
        );
        ensure!(
            raw["events"]
                .as_array()
                .is_some_and(|events| events.len() <= 512),
            "Actual event budget invalid"
        );
        for (index, (snapshot, settlement)) in snapshots.iter().zip(settlements).enumerate() {
            let checked = (|| -> Result<()> {
                validate_snapshot(
                    snapshot,
                    case["name"].as_str().context("Case name missing")?,
                    index,
                )?;
                ensure!(
                    state_without_gesture(snapshot)? == settlement["observed"]
                        && settlement["scope"] == SETTLEMENT_SCOPE
                        && settlement["reads"]
                            .as_u64()
                            .is_some_and(|reads| (2..=10000).contains(&reads))
                        && settlement["elapsedMs"]
                            .as_u64()
                            .is_some_and(|ms| (100..=3100).contains(&ms)),
                    "Independent settlement evidence differs"
                );
                if index > 0 {
                    let step = &steps[index - 1];
                    let detail = &details[index - 1];
                    ensure!(
                        detail["step"] == index - 1
                            && detail["gesture"] == *step
                            && detail["events"]
                                .as_array()
                                .is_some_and(|events| events.len() <= 512),
                        "Original gesture evidence differs"
                    );
                    if let Some(open) = step.get("open") {
                        ensure!(
                            FILES.iter().any(|(name, _)| open == *name)
                                && snapshot["action"] == "vscode.open"
                                && snapshot["opened"] == *open
                                && snapshot["requestedPreview"]
                                    == step.get("preview").cloned().unwrap_or(json!(true)),
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
                }
                Ok(())
            })();
            checked.with_context(|| {
                format!(
                    "Case {} frame {index} gesture {}",
                    case["name"].as_str().unwrap_or("<missing>"),
                    snapshot["action"].as_str().unwrap_or("<missing>")
                )
            })?;
        }
        if EXCLUDED.contains(&case["name"].as_str().context("Case name missing")?) {
            excluded_count += 1;
        }
        snapshot_count += snapshots.len();
        for (name, text) in FILES {
            file_inventory
                .push(json!({"case":case["name"],"resource":name,"sha256":hash(text.as_bytes())}));
        }
    }
    ensure!(
        snapshot_count == 86 && excluded_count == 2 && proof["files"] == json!(file_inventory),
        "Complete fixture inventory differs"
    );
    Ok(Corpus {
        cases,
        rows,
        platform: proof["platform"].as_str().unwrap().to_owned(),
    })
}

fn editor_projection(editor: &Value) -> Value {
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
    platform: String,
}
impl Replay {
    fn new(root: PathBuf, platform: &str) -> Self {
        Self {
            groups: Groups::default(),
            documents: BTreeMap::new(),
            resources: BTreeMap::new(),
            root,
            platform: platform.into(),
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
        let active = change
            .active
            .context("Native replay lost active membership")?;
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
    fn step(&mut self, step: &Value) -> Result<()> {
        if let Some(name) = step["open"].as_str() {
            return self.open(name, step["preview"].as_bool().unwrap_or(true));
        }
        let active = self
            .groups
            .active_membership()
            .context("Target has no active editor")?;
        let change = match step["command"].as_str().context("Command missing")? {
            "workbench.action.keepEditor" => Some(self.groups.keep(active)?),
            "workbench.action.previousEditorInGroup" => {
                Some(self.groups.navigate(Navigate::PreviousInGroup)?)
            }
            "workbench.action.focusFirstEditorGroup"
            | "workbench.action.focusSecondEditorGroup" => {
                let index =
                    usize::from(step["command"] == "workbench.action.focusSecondEditorGroup");
                Some(
                    self.groups.focus_group(
                        self.groups
                            .groups()
                            .get(index)
                            .context("Focus group missing")?
                            .id(),
                    )?,
                )
            }
            "workbench.action.splitEditor" => Some(self.groups.split_active()?),
            "workbench.action.files.newUntitledFile" => {
                let mut doc = Document::from_text("");
                // Replay the captured product platform's default, not the host's.
                doc.eol = if self.platform == "win32" {
                    "\r\n"
                } else {
                    "\n"
                }
                .into();
                let id = doc.id;
                self.documents.insert(id, ("untitled-1".into(), doc));
                self.resources.insert("untitled-1".into(), id);
                Some(self.groups.open(id)?)
            }
            "cursorRight" => {
                self.documents
                    .get_mut(&active.document)
                    .unwrap()
                    .1
                    .navigate_cursors("cursorRight", false, 0);
                None
            }
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
            "undo" => {
                self.documents.get_mut(&active.document).unwrap().1.undo();
                if self.documents[&active.document].1.dirty() {
                    self.groups.promote_document(active.document)?;
                }
                None
            }
            command => bail!("Unqualified native preview command: {command}"),
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
                "Native preview journey changed fixture disk"
            );
            documents.push(json!({"resource":name,"text":text,"dirty":dirty,"disk":disk}));
        }
        if let Some(id) = self.resources.get("untitled-1") {
            let doc = &self.documents[id].1;
            documents.push(json!({"resource":"untitled-1","text":doc.text.to_string(),"dirty":doc.api_dirty(),"disk":null}));
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
fn replay(case: &Value, platform: &str) -> Result<Value> {
    let directory = tempfile::tempdir()?;
    let root = fs::canonicalize(directory.path())?;
    for (name, text) in FILES {
        fs::write(root.join(name), text)?;
    }
    let mut native = Replay::new(root, platform);
    native.open("a.txt", true)?;
    let mut observations = vec![native.snapshot("initial", None)?];
    for step in case["steps"].as_array().context("Replay steps missing")? {
        native.step(step)?;
        let action = if step.get("open").is_some() {
            "vscode.open"
        } else {
            step["command"].as_str().unwrap()
        };
        observations.push(native.snapshot(action, Some(step))?);
    }
    Ok(json!({"name":case["name"],"observations":observations}))
}
fn compare(corpus: &Corpus) -> Result<Value> {
    let mut compared = Vec::new();
    let mut excluded = Vec::new();
    let mut differences = Vec::new();
    let mut snapshots = 0;
    for (case, row) in corpus.cases.iter().zip(&corpus.rows) {
        let name = case["name"].as_str().unwrap();
        if EXCLUDED.contains(&name) {
            excluded.push(json!({"name":name,"reason":"Sticky ordering is deferred; complete raw evidence was verified","rawSnapshots":row["observations"].as_array().unwrap().len()}));
            continue;
        }
        let expected = json!({"name":row["name"],"observations":row["observations"].as_array().unwrap().iter().map(projection).collect::<Vec<_>>()});
        let native = replay(case, &corpus.platform)?;
        if native != expected {
            differences.push(format!("{name}: native={native}, reference={expected}"));
        }
        snapshots += native["observations"].as_array().unwrap().len();
        compared.push(native);
    }
    ensure!(
        compared.len() == 9 && snapshots == 68 && excluded.len() == 2,
        "Native comparison scope changed"
    );
    if !differences.is_empty() {
        bail!(
            "Native preview trace differences:\n{}",
            differences.join("\n")
        );
    }
    Ok(json!({"compared":compared,"excludedStickyCases":excluded}))
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() <= 1,
        "Expected optional full reference directory"
    );
    let (directory, stem) = args.first().map_or_else(
        || {
            (
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(ROOT)
                    .join("baselines/1.95.0/editor-preview-tabs"),
                "linux",
            )
        },
        |directory| (PathBuf::from(directory), "editor-preview-tabs"),
    );
    let trace = read(&directory.join(format!("{stem}.json")))?;
    let evidence = read(&directory.join(format!("{stem}-evidence.json")))?;
    let proof: Value =
        serde_json::from_slice(&read(&directory.join(format!("{stem}-provenance.json")))?)?;
    // Entire raw provenance preflight precedes native fixture creation/replay.
    let corpus = validate(&trace, &evidence, &proof)?;
    println!("{}", serde_json::to_string_pretty(&compare(&corpus)?)?);
    eprintln!(
        "Compared 9 native preview/committed cases / 68 complete snapshots; verified 2 deferred sticky cases / 18 raw snapshots. App/save/loader/UI and public version/cache allocation are outside this projection; product.json hash is recorded, executable hash is absent."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn baseline() -> (Vec<u8>, Vec<u8>, Value) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(ROOT)
            .join("baselines/1.95.0/editor-preview-tabs");
        (
            read(&root.join("linux.json")).unwrap(),
            read(&root.join("linux-evidence.json")).unwrap(),
            serde_json::from_slice(&read(&root.join("linux-provenance.json")).unwrap()).unwrap(),
        )
    }
    fn bytes(value: &Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(value).unwrap();
        bytes.push(b'\n');
        bytes
    }
    fn tampered(change: impl FnOnce(&mut Value, &mut Value)) -> String {
        let (trace, evidence, mut proof) = baseline();
        validate(&trace, &evidence, &proof).unwrap();
        let mut rows: Value = serde_json::from_slice(&trace).unwrap();
        let mut raw: Value = serde_json::from_slice(&evidence).unwrap();
        change(&mut rows, &mut raw);
        let trace = bytes(&rows);
        let evidence = bytes(&raw);
        proof["traceSha256"] = json!(hash(&trace));
        proof["evidenceSha256"] = json!(hash(&evidence));
        for index in 0..11 {
            proof["runs"][index]["evidenceSha256"] = json!(hash(&bytes(&raw[index])));
        }
        validate(&trace, &evidence, &proof)
            .err()
            .expect("Tampered coherent hashes must still fail original input/evidence checks")
            .root_cause()
            .to_string()
    }
    #[test]
    fn accepts_full_raw_corpus_and_compares_all_nine_supported_cases() {
        let (trace, evidence, proof) = baseline();
        let corpus = validate(&trace, &evidence, &proof).unwrap();
        let result = compare(&corpus).unwrap();
        assert_eq!(result["compared"].as_array().unwrap().len(), 9);
        assert_eq!(result["excludedStickyCases"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn empty_untitled_undo_preserves_distinct_public_tab_and_document_dirty_evidence() {
        let (trace, evidence, proof) = baseline();
        let corpus = validate(&trace, &evidence, &proof).unwrap();
        for frame in 5..=7 {
            let snapshot = &corpus.rows[3]["observations"][frame];
            assert_eq!(snapshot["documents"][4]["text"], "");
            assert_eq!(snapshot["documents"][4]["dirty"], true);
            let tab = snapshot["groups"][0]["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tab| tab["resource"] == "untitled-1")
                .unwrap();
            assert_eq!(tab["dirty"], false);
            assert_eq!(projection(snapshot)["documents"][4]["dirty"], true);
        }
    }

    #[test]
    fn wrong_observer_and_product_are_rejected_before_any_fixture_creation() {
        let (trace, evidence, mut proof) = baseline();
        validate(&trace, &evidence, &proof).unwrap();
        proof["runs"][0]["sources"]["editor-preview-tabs.cjs"] = json!("0".repeat(64));
        assert_eq!(
            validate(&trace, &evidence, &proof)
                .err()
                .unwrap()
                .to_string(),
            "Observer source digest differs: editor-preview-tabs.cjs"
        );
        let (_, _, mut proof) = baseline();
        proof["commit"] = json!("0".repeat(40));
        assert_eq!(
            validate(&trace, &evidence, &proof)
                .err()
                .unwrap()
                .to_string(),
            "Pinned product identity differs"
        );
    }
    #[test]
    fn coherent_digest_rewrite_cannot_change_original_command() {
        assert_eq!(
            tampered(|rows, raw| {
                rows[1]["observations"][2]["action"] = json!("workbench.action.pinEditor");
                raw[1]["observations"][2]["action"] = json!("workbench.action.pinEditor");
            }),
            "Original target command differs"
        );
    }
    #[test]
    fn coherent_digest_rewrite_cannot_change_requested_preview_mode() {
        assert_eq!(
            tampered(|rows, raw| {
                rows[8]["observations"][2]["requestedPreview"] = json!(true);
                raw[8]["observations"][2]["requestedPreview"] = json!(true);
            }),
            "Original open mode differs"
        );
    }
    #[test]
    fn deferred_sticky_evidence_cannot_disappear_from_raw_preflight() {
        let (trace, evidence, proof) = baseline();
        validate(&trace, &evidence, &proof).unwrap();
        let mut rows: Value = serde_json::from_slice(&trace).unwrap();
        rows.as_array_mut().unwrap().remove(10);
        let trace = bytes(&rows);
        let mut proof = proof;
        proof["traceSha256"] = json!(hash(&trace));
        assert_eq!(
            validate(&trace, &evidence, &proof)
                .err()
                .unwrap()
                .to_string(),
            "Complete corpus inventory differs"
        );
    }
    #[test]
    fn fixture_bytes_and_scalar_evidence_cannot_be_replaced_under_updated_hashes() {
        assert_eq!(
            tampered(|rows, raw| {
                rows[0]["observations"][0]["documents"][0]["disk"] = json!("foreign");
                raw[0]["observations"][0]["documents"][0]["disk"] = json!("foreign");
            }),
            "Fixture disk or document evidence differs"
        );
        assert_eq!(
            tampered(|rows, raw| {
                rows[0]["observations"][0]["active"]["selections"][0]["cursor"]["character"] =
                    json!(1);
                raw[0]["observations"][0]["active"]["selections"][0]["cursor"]["character"] =
                    json!(1);
            }),
            "Selection UTF16/scalar evidence inconsistent"
        );
    }
}
