//! Optional isolated CommonJS extension host. Rust owns document transactions.
use crate::{document::Document, lsp, transport::Process};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
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
    document: u64,
    version: u64,
    edits: Vec<lsp::TextEdit>,
}
pub struct Client {
    // Drop the process before deleting its embedded runtime files on Windows.
    process: Process,
    _runtime: tempfile::TempDir,
    mirror: MirrorState,
    pending: HashMap<u64, Pending>,
    next_id: u64,
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
            mirror: MirrorState::default(),
            pending: HashMap::new(),
            next_id: 0,
            keybindings: None,
            ready: false,
            commands: Vec::new(),
            identity: String::new(),
        };
        let (next, state) = client.mirror.next(documents, active)?;
        client.request(
            "initialize",
            json!({"protocol":2, "extension": extension, "root": root, "state": state}),
        )?;
        client.mirror = next;
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
        args: Option<Value>,
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
        let args: Vec<_> = args.into_iter().collect();
        self.request("execute", json!({"command":command, "args":args}))
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
                    if message["result"]["protocol"] != 2 {
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
}
