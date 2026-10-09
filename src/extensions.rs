//! Optional isolated CommonJS extension host. Rust owns document transactions.
use crate::{document::Document, lsp, settings::Settings, transport::Process};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PACKAGES: usize = 8;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub version: String,
    pub path: PathBuf,
    pub sha256: Option<String>,
}
impl Package {
    pub fn read(path: &Path) -> Result<Self> {
        let path = std::fs::canonicalize(path).context("Cannot open extension directory")?;
        let (_, manifest) = read_manifest(&path)?;
        let publisher = manifest["publisher"]
            .as_str()
            .context("Extension manifest has no publisher")?;
        let name = manifest["name"]
            .as_str()
            .context("Extension manifest has no name")?;
        let id = format!("{publisher}.{name}").to_ascii_lowercase();
        if id.split('.').count() != 2
            || id.split('.').any(|part| {
                part.is_empty()
                    || part.len() > 100
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        {
            bail!("Extension ID must be publisher.name");
        }
        let version = manifest["version"]
            .as_str()
            .context("Extension manifest has no version")?
            .to_owned();
        if version.is_empty() || version.len() > 100 {
            bail!("Invalid extension version");
        }
        Ok(Self {
            id,
            version,
            path,
            sha256: None,
        })
    }
    pub fn installed(item: &crate::extension_store::Installed) -> Self {
        Self {
            id: item.id.clone(),
            version: item.version.clone(),
            path: item.path.clone(),
            sha256: Some(item.sha256.clone()),
        }
    }
}
fn read_manifest(path: &Path) -> Result<(usize, Value)> {
    let manifest = path.join("package.json");
    if !std::fs::metadata(&manifest)?.is_file() {
        bail!("Extension manifest must be a regular file");
    }
    let mut bytes = Vec::new();
    std::fs::File::open(manifest)?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        bail!("Extension manifest exceeds 1 MiB");
    }
    Ok((bytes.len(), serde_json::from_slice(&bytes)?))
}
fn manifest_nodes(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(manifest_nodes).sum::<usize>(),
        Value::Object(values) => 1 + values.values().map(manifest_nodes).sum::<usize>(),
        _ => 1,
    }
}
fn validate_packages(packages: &[Package]) -> Result<Vec<Package>> {
    if packages.is_empty() || packages.len() > MAX_PACKAGES {
        bail!("Select between one and eight code extensions");
    }
    let mut selected = packages.to_vec();
    selected.sort_by(|a, b| a.id.cmp(&b.id));
    let mut ids = HashSet::new();
    let (mut bytes, mut nodes, mut bindings, mut commands) = (0, 0, 0, 0);
    for package in &selected {
        if !ids.insert(package.id.clone()) {
            bail!("Duplicate extension: {}", package.id);
        }
        let (size, manifest) = read_manifest(&package.path)?;
        bytes += size;
        nodes += manifest_nodes(&manifest);
        if bytes > 8 * 1024 * 1024 || nodes > 50_000 {
            bail!("Extension session manifest budget exceeded (8 MiB / 50,000 nodes)");
        }
        let id = format!(
            "{}.{}",
            manifest["publisher"].as_str().unwrap_or(""),
            manifest["name"].as_str().unwrap_or("")
        )
        .to_ascii_lowercase();
        if id != package.id || manifest["version"].as_str() != Some(package.version.as_str()) {
            bail!("Extension identity changed: {}", package.id);
        }
        let count = |value: &Value| match value {
            Value::Null => 0,
            Value::Array(v) => v.len(),
            _ => 1,
        };
        bindings += count(&manifest["contributes"]["keybindings"]);
        commands += count(&manifest["contributes"]["commands"]);
        if bindings > 1024 || commands > 1024 {
            bail!("Extension session contribution limit exceeded");
        }
    }
    Ok(selected)
}
#[derive(Deserialize)]
struct Registration {
    id: String,
    owner: String,
}

#[derive(Clone)]
struct Mirror {
    revision: u64,
    uri: String,
    version: u64,
}
struct Pending {
    method: String,
    started: Instant,
}
#[derive(Deserialize)]
struct Edit {
    session: u64,
    owner: String,
    document: u64,
    version: u64,
    edits: Vec<lsp::TextEdit>,
}
pub struct Prepared {
    mirror: MirrorState,
    state: Value,
    configuration: Arc<Vec<serde_json::Map<String, Value>>>,
}
pub struct Client {
    // Drop the process before deleting its embedded runtime files on Windows.
    process: Process,
    _runtime: tempfile::TempDir,
    mirror: MirrorState,
    configuration: Arc<Vec<serde_json::Map<String, Value>>>,
    pending: HashMap<u64, Pending>,
    next_id: u64,
    heartbeat: Instant,
    pub keybindings: Option<Value>,
    pub ready: bool,
    pub commands: Vec<(String, String)>,
    pub identity: String,
    pub session: u64,
    pub packages: Vec<Package>,
    pub binding_sets: Option<Vec<(String, Value)>>,
    command_owners: HashMap<String, String>,
    titles: HashMap<(String, String), String>,
}
impl Client {
    pub fn start(
        node: &str,
        extension: &Path,
        root: &Path,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Self> {
        Self::start_many(
            node,
            &[Package::read(extension)?],
            root,
            documents,
            active,
            settings,
        )
    }
    pub fn start_many(
        node: &str,
        packages: &[Package],
        root: &Path,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Self> {
        Self::start_many_prepared(
            node,
            packages,
            root,
            Self::prepare(documents, active, settings)?,
        )
    }
    pub fn prepare(documents: &[Document], active: usize, settings: &Settings) -> Result<Prepared> {
        let (mirror, state) = MirrorState::default().next(documents, active)?;
        Ok(Prepared {
            mirror,
            state,
            configuration: settings.extension_layers().clone(),
        })
    }
    pub fn start_prepared(
        node: &str,
        extension: &Path,
        root: &Path,
        prepared: Prepared,
    ) -> Result<Self> {
        Self::start_many_prepared(node, &[Package::read(extension)?], root, prepared)
    }
    pub fn start_many_prepared(
        node: &str,
        packages: &[Package],
        root: &Path,
        prepared: Prepared,
    ) -> Result<Self> {
        let packages = validate_packages(packages)?;
        static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let session = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let runtime = tempfile::tempdir()?;
        for (name, source) in [
            (
                "api-types.cjs",
                include_str!("../extension-host/api-types.cjs"),
            ),
            ("api.cjs", include_str!("../extension-host/api.cjs")),
            (
                "configuration.cjs",
                include_str!("../extension-host/configuration.cjs"),
            ),
            ("host.cjs", include_str!("../extension-host/host.cjs")),
        ] {
            std::fs::write(runtime.path().join(name), source)?;
        }
        let process = Process::start_isolated(
            node,
            &[
                "--max-old-space-size=256".into(),
                runtime
                    .path()
                    .join("host.cjs")
                    .to_string_lossy()
                    .into_owned(),
            ],
            root,
        )?;
        let mut client = Self {
            process,
            _runtime: runtime,
            mirror: prepared.mirror,
            configuration: prepared.configuration,
            pending: HashMap::new(),
            next_id: 0,
            heartbeat: Instant::now(),
            keybindings: None,
            ready: false,
            commands: Vec::new(),
            identity: packages
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            session,
            packages,
            binding_sets: None,
            command_owners: HashMap::new(),
            titles: HashMap::new(),
        };
        client.request(
            "initialize",
            json!({"protocol":4, "session": session, "extensions": client.packages,
                "reservedCommands": crate::app::native_command_ids(), "root": root, "state": prepared.state,
                "configuration": client.configuration.as_ref()}),
        )?;
        Ok(client)
    }
    fn request(&mut self, method: &str, params: Value) -> Result<()> {
        if self.pending.len() >= 64 {
            bail!("Extension request limit reached");
        }
        self.next_id += 1;
        self.process
            .send(json!({"id":self.next_id, "method":method, "params":params}))?;
        self.pending.insert(
            self.next_id,
            Pending {
                method: method.into(),
                started: Instant::now(),
            },
        );
        Ok(())
    }
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }
    fn sync_configuration(&mut self, settings: &Settings) -> Result<()> {
        if !Arc::ptr_eq(&self.configuration, settings.extension_layers()) {
            self.process.send(
                json!({"method":"configuration", "params":settings.extension_layers().as_ref()}),
            )?;
            self.configuration = settings.extension_layers().clone();
        }
        Ok(())
    }
    pub fn execute(
        &mut self,
        command: &str,
        args: Option<Value>,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        if !self.ready {
            bail!("Extension host is not ready");
        }
        if !self.commands.iter().any(|(_, id)| id == command) {
            bail!("Extension command is not registered: {command}");
        }
        self.sync(documents, active)?;
        self.sync_configuration(settings)?;
        let args: Vec<_> = args.into_iter().collect();
        self.request("execute", json!({"session":self.session, "owner":self.command_owners[command], "command":command, "args":args}))
    }
    fn sync(&mut self, documents: &[Document], active: usize) -> Result<()> {
        if MirrorState::stamp(documents, active) != self.mirror.last_stamp {
            let (next, state) = self.mirror.next(documents, active)?;
            self.process
                .send(json!({"method":"state", "params":state}))?;
            self.mirror = next;
        }
        Ok(())
    }
    fn apply_edit(&self, edit: Edit, documents: &mut [Document]) -> Result<bool> {
        if edit.session != self.session || !self.packages.iter().any(|p| p.id == edit.owner) {
            return Ok(false);
        }
        let Some(mirror) = self.mirror.mirrors.get(&edit.document) else {
            return Ok(false);
        };
        let total: usize = documents.iter().map(|d| d.text.len_bytes()).sum();
        let Some(doc) = documents.iter_mut().find(|d| d.id == edit.document) else {
            return Ok(false);
        };
        if edit.version != mirror.version
            || doc.revision != mirror.revision
            || document_uri(doc)? != mirror.uri
        {
            return Ok(false);
        }
        let changes = lsp::edits(doc, edit.edits)?;
        let removed: usize = changes
            .iter()
            .map(|(range, _)| doc.text.slice(range.clone()).len_bytes())
            .sum();
        let added: usize = changes.iter().map(|(_, text)| text.len()).sum();
        if total - removed + added > MAX_DOCUMENT_BYTES {
            bail!("Extension edit exceeds the 4 MiB mirror budget");
        }
        doc.apply_changes(changes);
        Ok(true)
    }
    fn register_commands(&mut self, value: Value) -> Result<()> {
        let registrations: Vec<Registration> = serde_json::from_value(value)?;
        if registrations.len() > 1024 {
            bail!("Extension command limit exceeded");
        }
        let mut commands = Vec::new();
        let mut owners = HashMap::new();
        let reserved = crate::app::native_command_ids();
        for registration in registrations {
            if registration.id.is_empty()
                || registration.id.len() > 1024
                || registration.id.starts_with("cursor")
                || reserved.contains(&registration.id)
                || !self.packages.iter().any(|p| p.id == registration.owner)
                || owners
                    .insert(registration.id.clone(), registration.owner.clone())
                    .is_some()
            {
                bail!("Invalid extension command ownership: {}", registration.id);
            }
            let title = self
                .titles
                .get(&(registration.owner.clone(), registration.id.clone()))
                .cloned()
                .unwrap_or_else(|| format!("{} [{}]", registration.id, registration.owner));
            commands.push((title, registration.id));
        }
        commands.sort_by(|a, b| a.1.cmp(&b.1));
        self.commands = commands;
        self.command_owners = owners;
        Ok(())
    }
    pub fn poll(
        &mut self,
        documents: &mut [Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Vec<String>> {
        self.sync(documents, active)?;
        self.sync_configuration(settings)?;
        if self.ready
            && self.pending.is_empty()
            && self.heartbeat.elapsed() >= Duration::from_secs(5)
        {
            self.request("ping", json!({"session":self.session}))?;
            self.heartbeat = Instant::now();
        }
        let mut messages = Vec::new();
        for _ in 0..16 {
            let Some(mut message) = self.process.receive()? else {
                break;
            };
            if let Some(method) = message["method"].as_str() {
                match method {
                    "edit" => {
                        let result = if message["params"]["edits"]
                            .as_array()
                            .is_some_and(|edits| edits.len() > 4096)
                        {
                            Err(anyhow::anyhow!("Extension edit count limit exceeded"))
                        } else {
                            serde_json::from_value::<Edit>(message["params"].take())
                                .map_err(anyhow::Error::from)
                                .and_then(|edit| self.apply_edit(edit, documents))
                        };
                        match result {
                            Ok(applied) => {
                                // Ordered notifications precede the acknowledgement.
                                self.sync(documents, active)?;
                                self.process.send(
                                    json!({"id":message["id"], "result":{"applied":applied}}),
                                )?;
                            }
                            Err(error) => self.process.send(
                                json!({"id":message["id"], "error":{"message":error.to_string()}}),
                            )?,
                        }
                    }
                    "commands" => {
                        if message["params"]["session"].as_u64() != Some(self.session) {
                            continue;
                        }
                        if !self.ready {
                            bail!("Extension commands arrived before session activation completed");
                        }
                        self.register_commands(message["params"]["commands"].clone())?;
                    }
                    "message" => {
                        let params = &message["params"];
                        if params["session"].as_u64() == Some(self.session)
                            && self
                                .packages
                                .iter()
                                .any(|p| params["owner"].as_str() == Some(&p.id))
                        {
                            messages.push(
                                params["text"]
                                    .as_str()
                                    .unwrap_or("Extension message")
                                    .chars()
                                    .take(2048)
                                    .collect(),
                            );
                        }
                    }
                    _ => bail!("Unknown extension request: {method}"),
                }
            } else if let Some(id) = message["id"].as_u64()
                && let Some(pending) = self.pending.remove(&id)
            {
                if !message["error"].is_null() {
                    let error = message["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown extension error");
                    if pending.method == "initialize" || pending.method == "ping" {
                        bail!(
                            "Extension {} failed: {error}",
                            if pending.method == "initialize" {
                                "activation"
                            } else {
                                "heartbeat"
                            }
                        );
                    }
                    messages.push(format!("Extension command failed: {error}"));
                    continue;
                }
                if pending.method == "initialize" {
                    let result = &message["result"];
                    if result["protocol"] != 4 || result["session"].as_u64() != Some(self.session) {
                        bail!("Unsupported or outdated extension session protocol");
                    }
                    let extensions = result["extensions"]
                        .as_array()
                        .context("Extension session packages missing")?;
                    if extensions.len() != self.packages.len() {
                        bail!("Extension session package count changed");
                    }
                    let mut bindings = Vec::new();
                    let mut owners = HashSet::new();
                    for item in extensions {
                        let owner = item["id"]
                            .as_str()
                            .context("Extension session identity missing")?;
                        if !owners.insert(owner)
                            || !self.packages.iter().any(|p| {
                                p.id == owner
                                    && Some(p.version.as_str()) == item["version"].as_str()
                            })
                        {
                            bail!("Extension session identity changed");
                        }
                        bindings.push((owner.to_owned(), item["keybindings"].clone()));
                        if let Some(contributions) = item["contributions"].as_array() {
                            if contributions.len() > 1024 {
                                bail!("Extension command contribution limit exceeded");
                            }
                            for contribution in contributions {
                                if let Some(id) = contribution["command"].as_str() {
                                    let title: String = contribution["title"]
                                        .as_str()
                                        .unwrap_or(id)
                                        .chars()
                                        .take(256)
                                        .collect();
                                    self.titles.insert(
                                        (owner.into(), id.into()),
                                        format!("Extension: {title} [{owner}]"),
                                    );
                                }
                            }
                        }
                    }
                    bindings.sort_by(|a, b| a.0.cmp(&b.0));
                    // Keep single-package callers' contribution access intact.
                    self.keybindings = (bindings.len() == 1).then(|| bindings[0].1.clone());
                    self.binding_sets = Some(bindings);
                    self.register_commands(result["commands"].clone())?;
                    self.ready = true;
                    messages.push(format!(
                        "Extension ready: {} ({} commands)",
                        self.identity,
                        self.commands.len()
                    ));
                }
            }
        }
        if self.process.exited() {
            bail!("Extension process exited\n{}", self.process.stderr_tail());
        }
        if self
            .pending
            .values()
            .any(|p| p.started.elapsed() > Duration::from_secs(30))
        {
            bail!("Extension host timed out; native editing remains available");
        }
        Ok(messages)
    }
}
#[derive(Clone, Default)]
struct MirrorState {
    mirrors: HashMap<u64, Mirror>,
    generation: u64,
    last_stamp: Value,
}
impl MirrorState {
    fn stamp(documents: &[Document], active: usize) -> Value {
        json!({"documents": documents.iter().map(|d| json!([d.id, d.revision, d.path, d.dirty()])).collect::<Vec<_>>(),
            "active": documents.get(active).map(|d| d.id), "selections": documents.get(active).map(selections)})
    }
    fn next(&self, documents: &[Document], active: usize) -> Result<(Self, Value)> {
        // Only publish the new baseline after the transport accepts the update.
        let mut next = self.clone();
        let state = next.state(documents, active)?;
        Ok((next, state))
    }
    fn state(&mut self, documents: &[Document], active: usize) -> Result<Value> {
        if documents.iter().map(|d| d.text.len_bytes()).sum::<usize>() > MAX_DOCUMENT_BYTES {
            bail!(
                "Extension document mirrors currently support at most 4 MiB total; native editing remains available"
            );
        }
        let mut snapshots = Vec::new();
        for doc in documents {
            let uri = document_uri(doc)?;
            let needs_text = self
                .mirrors
                .get(&doc.id)
                .is_none_or(|old| old.revision != doc.revision);
            let mirror = self.mirrors.entry(doc.id).or_insert_with(|| Mirror {
                revision: doc.revision,
                uri: uri.clone(),
                version: 1,
            });
            if mirror.revision != doc.revision || mirror.uri != uri {
                mirror.version += 1;
                mirror.revision = doc.revision;
                mirror.uri = uri.clone();
            }
            let mut snapshot = json!({"id":doc.id, "uri":uri, "version":mirror.version,
                "languageId":doc.path.as_deref().map_or("plaintext", lsp::language), "isDirty":doc.dirty()});
            if needs_text {
                snapshot["text"] = doc.text.to_string().into();
            }
            snapshots.push(snapshot);
        }
        let ids: HashSet<_> = documents.iter().map(|d| d.id).collect();
        self.mirrors.retain(|id, _| ids.contains(id));
        self.generation += 1;
        self.last_stamp = Self::stamp(documents, active);
        Ok(
            json!({"generation":self.generation, "documents":snapshots, "active":documents.get(active).map(|d|d.id),
            "selections":documents.get(active).map(selections).unwrap_or_default()}),
        )
    }
}
fn document_uri(doc: &Document) -> Result<String> {
    doc.path
        .as_deref()
        .map_or_else(|| Ok(format!("untitled:vscli-{}", doc.id)), lsp::file_uri)
}
fn selections(doc: &Document) -> Vec<Value> {
    std::iter::once((doc.anchor.unwrap_or(doc.cursor), doc.cursor))
        .chain(doc.secondary.iter().map(|s|(s.anchor.unwrap_or(s.cursor),s.cursor)))
        .map(|(anchor,active)|json!({"anchor":lsp::position(doc,anchor),"active":lsp::position(doc,active)})).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_updates_are_bounded_metadata_and_edits_only_send_changed_documents() {
        let mut docs = vec![
            Document::from_text(&"0123456789abcdef\n".repeat(100_000)),
            Document::from_text("other"),
        ];
        let baseline = MirrorState::default();
        let (first, initial) = baseline.next(&docs, 0).unwrap();
        assert!(serde_json::to_vec(&initial).unwrap().len() > 1_700_000);
        // Encoding an update that never gets queued cannot advance the baseline.
        assert_eq!(baseline.next(&docs, 0).unwrap().1, initial);
        docs[0].move_to(20, false);
        let (second, cursor) = first.next(&docs, 0).unwrap();
        assert!(serde_json::to_vec(&cursor).unwrap().len() < 1024);
        assert!(
            cursor["documents"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| d.get("text").is_none())
        );
        assert_eq!(cursor["documents"][0]["version"], 1);
        docs[1].insert("!", false);
        let (third, edited) = second.next(&docs, 1).unwrap();
        assert!(edited["documents"][0].get("text").is_none());
        assert_eq!(edited["documents"][1]["text"], "!other");
        assert_eq!(edited["documents"][1]["version"], 2);
        docs[1].undo();
        let (_, undone) = third.next(&docs, 1).unwrap();
        assert_eq!(undone["documents"][1]["version"], 3);
        assert_eq!(undone["documents"][1]["text"], "other");
    }
    #[test]
    fn path_dirty_and_lifecycle_updates_preserve_content_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut docs = vec![Document::from_text("original")];
        let (first, _) = MirrorState::default().next(&docs, 0).unwrap();
        docs[0].path = Some(directory.path().join("renamed.rs"));
        let (second, renamed) = first.next(&docs, 0).unwrap();
        assert_eq!(renamed["documents"][0]["version"], 2);
        assert_eq!(renamed["documents"][0]["languageId"], "rust");
        assert!(renamed["documents"][0].get("text").is_none());
        docs[0].saved_revision = u64::MAX;
        let (third, dirty) = second.next(&docs, 0).unwrap();
        assert_eq!(dirty["documents"][0]["version"], 2);
        assert_eq!(dirty["documents"][0]["isDirty"], true);
        assert!(dirty["documents"][0].get("text").is_none());
        let (closed, update) = third.next(&[], 0).unwrap();
        assert_eq!(update["documents"], json!([]));
        assert!(update["active"].is_null());
        let (_, reopened) = closed.next(&docs, 0).unwrap();
        assert_eq!(reopened["documents"][0]["text"], "original");
    }
    #[test]
    fn outdated_owner_edits_and_expired_heartbeat_cannot_mutate_native_documents() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut documents = vec![Document::from_text("original")];
        let settings = Settings::default();
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &documents,
            0,
            &settings,
        )
        .unwrap();
        let edit = |session, owner| Edit {
            session,
            owner,
            document: documents[0].id,
            version: 1,
            edits: Vec::new(),
        };
        assert!(
            !client
                .apply_edit(
                    edit(client.session + 1, client.packages[0].id.clone()),
                    &mut [Document::from_text("unrelated")]
                )
                .unwrap()
        );
        let edit = Edit {
            session: client.session,
            owner: "unknown.owner".into(),
            document: documents[0].id,
            version: 1,
            edits: Vec::new(),
        };
        assert!(!client.apply_edit(edit, &mut documents).unwrap());
        client.pending.insert(
            u64::MAX,
            Pending {
                method: "ping".into(),
                started: Instant::now() - Duration::from_secs(31),
            },
        );
        assert!(
            client
                .poll(&mut documents, 0, &settings)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert_eq!(documents[0].text.to_string(), "original");
    }
}
