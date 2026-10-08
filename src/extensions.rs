//! Optional isolated CommonJS extension host. Rust owns document transactions.
use crate::{document::Document, lsp, transport::Process};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
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
    document: u64,
    version: u64,
    edits: Vec<lsp::TextEdit>,
}
pub struct Client {
    // Drop the process before deleting its embedded runtime files on Windows.
    process: Process,
    _runtime: tempfile::TempDir,
    mirrors: HashMap<u64, Mirror>,
    pending: HashMap<u64, Pending>,
    next_id: u64,
    generation: u64,
    last_stamp: Value,
    pub keybindings: Option<Value>,
    pub ready: bool,
    pub commands: Vec<(String, String)>,
    pub identity: String,
}
impl Client {
    pub fn start(
        node: &str,
        extension: &Path,
        root: &Path,
        documents: &[Document],
        active: usize,
    ) -> Result<Self> {
        let extension =
            std::fs::canonicalize(extension).context("Cannot open extension directory")?;
        let runtime = tempfile::tempdir()?;
        for (name, source) in [
            (
                "api-types.cjs",
                include_str!("../extension-host/api-types.cjs"),
            ),
            ("api.cjs", include_str!("../extension-host/api.cjs")),
            ("host.cjs", include_str!("../extension-host/host.cjs")),
        ] {
            std::fs::write(runtime.path().join(name), source)?;
        }
        let process = Process::start(
            node,
            &[runtime
                .path()
                .join("host.cjs")
                .to_string_lossy()
                .into_owned()],
            root,
        )?;
        let mut client = Self {
            process,
            _runtime: runtime,
            mirrors: HashMap::new(),
            pending: HashMap::new(),
            next_id: 0,
            generation: 0,
            last_stamp: Value::Null,
            keybindings: None,
            ready: false,
            commands: Vec::new(),
            identity: String::new(),
        };
        let state = client.state(documents, active)?;
        client.request(
            "initialize",
            json!({"protocol":1, "extension": extension, "root": root, "state": state}),
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
    pub fn execute(
        &mut self,
        command: &str,
        args: Value,
        documents: &[Document],
        active: usize,
    ) -> Result<()> {
        if !self.ready {
            bail!("Extension host is not ready");
        }
        if !self.commands.iter().any(|(_, id)| id == command) {
            bail!("Extension command is not registered: {command}");
        }
        self.sync(documents, active)?;
        let args = match args {
            Value::Null => vec![],
            Value::Array(args) => args,
            other => vec![other],
        };
        self.request("execute", json!({"command":command, "args":args}))
    }
    fn stamp(documents: &[Document], active: usize) -> Value {
        json!({"documents": documents.iter().map(|d| json!([d.id, d.revision, d.path, d.dirty()])).collect::<Vec<_>>(),
            "active": documents.get(active).map(|d| d.id), "selections": documents.get(active).map(selections)})
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
            snapshots.push(json!({"id":doc.id, "uri":uri, "text":doc.text.to_string(), "version":mirror.version,
                "languageId":doc.path.as_deref().map_or("plaintext", lsp::language), "isDirty":doc.dirty()}));
        }
        self.mirrors
            .retain(|id, _| documents.iter().any(|d| d.id == *id));
        self.generation += 1;
        self.last_stamp = Self::stamp(documents, active);
        Ok(
            json!({"generation":self.generation, "documents":snapshots, "active":documents.get(active).map(|d|d.id),
            "selections":documents.get(active).map(selections).unwrap_or_default()}),
        )
    }
    fn sync(&mut self, documents: &[Document], active: usize) -> Result<()> {
        if Self::stamp(documents, active) != self.last_stamp {
            let state = self.state(documents, active)?;
            self.process
                .send(json!({"method":"state", "params":state}))?;
        }
        Ok(())
    }
    fn apply_edit(&self, edit: Edit, documents: &mut [Document]) -> Result<bool> {
        let Some(mirror) = self.mirrors.get(&edit.document) else {
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
    pub fn poll(&mut self, documents: &mut [Document], active: usize) -> Result<Vec<String>> {
        self.sync(documents, active)?;
        let mut messages = Vec::new();
        for _ in 0..16 {
            let Some(message) = self.process.receive()? else {
                break;
            };
            if let Some(method) = message["method"].as_str() {
                match method {
                    "edit" => {
                        let result = serde_json::from_value::<Edit>(message["params"].clone())
                            .map_err(anyhow::Error::from)
                            .and_then(|edit| self.apply_edit(edit, documents));
                        match result {
                            Ok(applied) => {
                                let state = self.state(documents, active)?;
                                self.process.send(json!({"id":message["id"], "result":{"applied":applied, "state":state}}))?;
                            }
                            Err(error) => self.process.send(
                                json!({"id":message["id"], "error":{"message":error.to_string()}}),
                            )?,
                        }
                    }
                    "commands" => {
                        let commands: Vec<String> =
                            serde_json::from_value(message["params"].clone())?;
                        if commands.len() > 1024 {
                            bail!("Extension command limit exceeded");
                        }
                        let old: HashMap<_, _> = self
                            .commands
                            .drain(..)
                            .map(|(label, id)| (id, label))
                            .collect();
                        self.commands = commands
                            .into_iter()
                            .map(|id| (old.get(&id).cloned().unwrap_or_else(|| id.clone()), id))
                            .collect();
                    }
                    "message" => messages.push(
                        message["params"]
                            .as_str()
                            .unwrap_or("Extension message")
                            .chars()
                            .take(2048)
                            .collect(),
                    ),
                    _ => bail!("Unknown extension request: {method}"),
                }
            } else if let Some(id) = message["id"].as_u64()
                && let Some(pending) = self.pending.remove(&id)
            {
                if !message["error"].is_null() {
                    let error = message["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown extension error");
                    if pending.method == "initialize" {
                        bail!("Extension activation failed: {error}");
                    }
                    messages.push(format!("Extension command failed: {error}"));
                    continue;
                }
                if pending.method == "initialize" {
                    if message["result"]["protocol"] != 1 {
                        bail!("Unsupported extension host protocol version");
                    }
                    self.keybindings = Some(message["result"]["keybindings"].clone());
                    self.ready = true;
                    self.identity = message["result"]["id"]
                        .as_str()
                        .unwrap_or("extension")
                        .into();
                    if let Some(contributions) = message["result"]["contributions"].as_array() {
                        for (label, id) in &mut self.commands {
                            if let Some(item) = contributions
                                .iter()
                                .find(|item| item["command"].as_str() == Some(id.as_str()))
                            {
                                let title = item["title"].as_str().unwrap_or(id);
                                *label = format!("Extension: {title}");
                            }
                        }
                    }
                    messages.push(format!(
                        "Extension ready: {} ({} commands)",
                        self.identity,
                        self.commands.len()
                    ));
                }
            }
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
