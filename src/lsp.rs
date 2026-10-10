//! Native stdio LSP transport. Subprocess I/O never blocks the input/render loop.
use crate::document::{Document, Selection};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Diagnostic {
    pub range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<u8>,
    pub message: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct TextEdit {
    pub range: Range,
    #[serde(rename = "newText")]
    pub new_text: String,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub(crate) token: u64,
    pub method: String,
    pub document_id: u64,
    pub revision: u64,
    pub text_epoch: u64,
    pub(crate) server: Arc<()>,
    pub(crate) sync_version: i64,
    pub cursor: usize,
    pub path: PathBuf,
    pub selections: Vec<Selection>,
    pub view: Option<u64>,
    pub workspace: Arc<HashMap<PathBuf, Snapshot>>,
    started: Instant,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub id: u64,
    pub revision: u64,
    pub text_epoch: u64,
    pub version: i64,
}
#[derive(Clone, Debug)]
pub struct DiagnosticPublication {
    pub uri: String,
    pub path: PathBuf,
    pub server: Arc<()>,
    pub snapshot: Snapshot,
    /// An omitted version cannot establish delayed publication provenance.
    pub versioned: bool,
    pub items: Vec<Diagnostic>,
    pub(crate) serialized_bytes: usize,
}
pub enum Event {
    Ready,
    ApplyEdit(Value, Option<Request>, Value),
    Response(Request, Value),
    SignatureFailure(Request, String),
    SymbolFailure(Request, String),
    Diagnostics(DiagnosticPublication),
    Message(String),
}
struct Synced {
    id: u64,
    revision: u64,
    text_epoch: u64,
    version: i64,
    path: PathBuf,
}
struct SymbolSlot {
    token: u64,
    canceled: bool,
    timed_out: bool,
}
pub struct Client {
    identity: Arc<()>,
    transport: crate::transport::Process,
    pending: HashMap<u64, Request>,
    completion_resolve: Option<u64>,
    completion_resolve_valid: bool,
    signature_occupied: Option<u64>,
    signature_timed_out: bool,
    symbol_slot: Option<SymbolSlot>,
    synced: HashMap<String, Synced>,
    next_id: u64,
    command_channel_valid: bool,
    action_channel_valid: bool,
    next_document_version: i64,
    root_uri: String,
    pub language: String,
    pub ready: bool,
    pub capabilities: Value,
    started: Instant,
}
pub fn file_uri(path: &Path) -> Result<String> {
    Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| anyhow::anyhow!("Not an absolute file path: {}", path.display()))
}
pub fn uri_path(uri: &str) -> Result<PathBuf> {
    let path = Url::parse(uri)?
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Unsupported document URI: {uri}"))?;
    // URL decoding drops Windows' verbatim prefix and may return a filesystem
    // alias. Match the identity used by Document::open when the path is resolvable.
    Ok(crate::document::absolute_path(&path).unwrap_or(path))
}
pub fn position(doc: &Document, cursor: usize) -> Position {
    let cursor = cursor.min(doc.len());
    let line = doc.text.char_to_line(cursor);
    let character = doc
        .text
        .slice(doc.line_start(line)..cursor)
        .chars()
        .map(char::len_utf16)
        .sum();
    Position { line, character }
}
pub fn offset(doc: &Document, position: Position) -> Result<usize> {
    if position.line >= doc.line_count() {
        bail!("Language server returned an invalid line");
    }
    let start = doc.line_start(position.line);
    let mut units = 0;
    let mut cursor = start;
    for ch in doc.text.slice(start..doc.line_end(position.line)).chars() {
        if units == position.character {
            return Ok(cursor);
        }
        units += ch.len_utf16();
        cursor += 1;
        if units > position.character {
            bail!("Language server position splits a UTF-16 character");
        }
    }
    if units == position.character {
        Ok(cursor)
    } else {
        bail!("Language server returned an invalid column")
    }
}
pub fn edits(
    doc: &Document,
    edits: Vec<TextEdit>,
) -> Result<Vec<(std::ops::Range<usize>, String)>> {
    let mut changes = Vec::new();
    for edit in edits {
        let start = offset(doc, edit.range.start)?;
        let end = offset(doc, edit.range.end)?;
        if start > end {
            bail!("Language server edit has a reversed range");
        }
        changes.push((start..end, edit.new_text));
    }
    changes.sort_by_key(|(range, _)| (range.start, range.end));
    for pair in changes.windows(2) {
        if pair[0].0.end > pair[1].0.start || pair[0].0.start == pair[1].0.start {
            bail!("Language server returned overlapping edits");
        }
    }
    let removed: usize = changes
        .iter()
        .map(|(r, _)| doc.text.slice(r.clone()).len_bytes())
        .sum();
    let added: usize = changes.iter().map(|(_, text)| text.len()).sum();
    if (doc.text.len_bytes() - removed + added) as u64 > crate::document::MAX_FILE_BYTES {
        bail!("Language server edit exceeds document size limit");
    }
    Ok(changes)
}
pub use crate::languages::language;

const MAX_DIAGNOSTIC_BYTES: usize = 2 * 1024 * 1024;
struct DiagnosticBudget(usize);
impl Write for DiagnosticBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::other("Diagnostic publication exceeds 2 MiB"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn publication_items(value: &Value) -> Result<(Vec<Diagnostic>, usize)> {
    let values = value.as_array().context("Diagnostics must be an array")?;
    if values.len() > 5000 {
        bail!("Diagnostic publication exceeds 5000 items");
    }
    for item in values {
        let message = item["message"]
            .as_str()
            .context("Diagnostic message must be text")?;
        if message.len() > 4096 {
            bail!("Diagnostic message exceeds 4 KiB");
        }
        if item.get("severity").is_some_and(|severity| {
            severity
                .as_u64()
                .is_none_or(|value| !(1..=4).contains(&value))
        }) {
            bail!("Diagnostic severity must be between 1 and 4");
        }
    }
    let mut budget = DiagnosticBudget(MAX_DIAGNOSTIC_BYTES);
    serde_json::to_writer(&mut budget, value)?;
    let items: Vec<Diagnostic> = serde_json::from_value(value.clone())?;
    if items.iter().any(|item| {
        [
            item.range.start.line,
            item.range.start.character,
            item.range.end.line,
            item.range.end.character,
        ]
        .iter()
        .any(|coordinate| *coordinate > i32::MAX as usize)
    }) {
        bail!("Diagnostic coordinates exceed LSP uinteger bounds");
    }
    if items.iter().any(|item| {
        (item.range.start.line, item.range.start.character)
            > (item.range.end.line, item.range.end.character)
    }) {
        bail!("Diagnostic range is reversed");
    }
    Ok((items, MAX_DIAGNOSTIC_BYTES - budget.0))
}

impl Client {
    pub fn debug_summary(&self) -> String {
        format!(
            "ready={}, synced={}, pending={}\n{}",
            self.ready,
            self.synced.len(),
            self.pending.len(),
            self.transport.stderr_tail()
        )
    }
    pub fn start(program: &str, args: &[String], root: &Path, language: String) -> Result<Self> {
        Self::start_with_isolation(program, args, root, language, false)
    }
    pub(crate) fn start_isolated(
        program: &str,
        args: &[String],
        root: &Path,
        language: String,
    ) -> Result<Self> {
        Self::start_with_isolation(program, args, root, language, true)
    }
    fn start_with_isolation(
        program: &str,
        args: &[String],
        root: &Path,
        language: String,
        isolated: bool,
    ) -> Result<Self> {
        let root_uri = file_uri(root)?;
        let transport = if isolated {
            crate::transport::Process::start_isolated(program, args, root)?
        } else {
            crate::transport::Process::start(program, args, root)?
        };
        let client = Self {
            identity: Arc::new(()),
            transport,
            pending: HashMap::new(),
            completion_resolve: None,
            completion_resolve_valid: true,
            signature_occupied: None,
            signature_timed_out: false,
            symbol_slot: None,
            synced: HashMap::new(),
            next_id: 1,
            command_channel_valid: true,
            action_channel_valid: true,
            next_document_version: 1,
            root_uri,
            language,
            ready: false,
            capabilities: Value::Null,
            started: Instant::now(),
        };
        client.send(json!({"jsonrpc":"2.0", "id":0, "method":"initialize", "params":{
            "processId":std::process::id(), "clientInfo":{"name":"vscli","version":env!("CARGO_PKG_VERSION")},
            "rootUri":client.root_uri, "workspaceFolders":[{"uri":client.root_uri,"name":root.file_name().unwrap_or_default().to_string_lossy()}],
            "capabilities":{
                "general":{"positionEncodings":["utf-16"]},
                "workspace":{"symbol":{"symbolKind":{"valueSet":(1..=26).collect::<Vec<_>>()}},"configuration":true,"workspaceFolders":true,"applyEdit":true,"workspaceEdit":{"documentChanges":true,"resourceOperations":[],"failureHandling":"transactional"}},
                "textDocument":{
                    "synchronization":{"didSave":true}, "publishDiagnostics":{"versionSupport":true},
                    "hover":{"contentFormat":["plaintext","markdown"]},
                    "signatureHelp":{"signatureInformation":{"documentationFormat":["plaintext"],"parameterInformation":{"labelOffsetSupport":true},"activeParameterSupport":true},"contextSupport":true},
                    "completion":{"contextSupport":true,"completionItem":{"snippetSupport":true,"documentationFormat":["plaintext","markdown"],"resolveSupport":{"properties":["detail","documentation","additionalTextEdits"]}}},
                    "documentSymbol":{"hierarchicalDocumentSymbolSupport":true},
                    "definition":{"linkSupport":true}, "references":{}, "formatting":{}, "rename":{},
                    "codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["","quickfix","refactor","refactor.extract","refactor.inline","refactor.rewrite","source","source.organizeImports","source.fixAll"]}},"isPreferredSupport":true,"disabledSupport":true,"dataSupport":true,"resolveSupport":{"properties":["edit"]}}
                }
            }
        }}))?;
        Ok(client)
    }
    fn send(&self, value: Value) -> Result<()> {
        self.transport.send(value)
    }
    #[cfg(test)]
    pub(crate) fn fixture_notify(&self, method: &str, params: Value) -> Result<()> {
        self.notify(method, params)
    }
    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub fn sync(&mut self, documents: &[Document]) -> Result<()> {
        if !self.ready {
            return Ok(());
        }
        let mut open = Vec::new();
        for doc in documents {
            let Some(path) = &doc.path else {
                continue;
            };
            let lang = language(path);
            if lang != self.language && !(self.language == "cpp" && lang == "c") {
                continue;
            }
            let uri = file_uri(path)?;
            open.push(uri.clone());
            match self.synced.get(&uri) {
                None => {
                    let version = self.allocate_document_version()?;
                    self.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":lang,"version":version,"text":doc.text.to_string()}}))?;
                    self.synced.insert(
                        uri,
                        Synced {
                            id: doc.id,
                            revision: doc.revision,
                            text_epoch: doc.text_epoch(),
                            version,
                            path: path.clone(),
                        },
                    );
                }
                Some(old)
                    if old.revision != doc.revision
                        || old.text_epoch != doc.text_epoch()
                        || old.id != doc.id =>
                {
                    let version = self.allocate_document_version()?;
                    self.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":doc.text.to_string()}]}))?;
                    self.synced.insert(
                        uri,
                        Synced {
                            id: doc.id,
                            revision: doc.revision,
                            text_epoch: doc.text_epoch(),
                            version,
                            path: path.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
        for uri in self
            .synced
            .keys()
            .filter(|uri| !open.contains(uri))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}))?;
            self.synced.remove(&uri);
        }
        Ok(())
    }
    fn allocate_document_version(&mut self) -> Result<i64> {
        // Never reuse a version after closing/reopening a URI: a delayed command
        // callback must not match a different lifetime of the same file.
        if self.next_document_version > i64::from(i32::MAX) {
            bail!("Language document version limit reached; restart the language server");
        }
        let version = self.next_document_version;
        self.next_document_version += 1;
        Ok(version)
    }
    pub fn diagnostic_current(&self, publication: &DiagnosticPublication, doc: &Document) -> bool {
        self.ready
            && Arc::ptr_eq(&publication.server, &self.identity)
            && doc.path.as_ref() == Some(&publication.path)
            && doc.id == publication.snapshot.id
            && doc.revision == publication.snapshot.revision
            && doc.text_epoch() == publication.snapshot.text_epoch
            && self.synced.get(&publication.uri).is_some_and(|synced| {
                synced.path == publication.path
                    && synced.id == publication.snapshot.id
                    && synced.revision == publication.snapshot.revision
                    && synced.text_epoch == publication.snapshot.text_epoch
                    && synced.version == publication.snapshot.version
            })
    }
    pub fn workspace_snapshot_current(&self, path: &Path, snapshot: &Snapshot) -> bool {
        self.ready
            && file_uri(path)
                .ok()
                .and_then(|uri| self.synced.get(&uri))
                .is_some_and(|synced| {
                    synced.path == path
                        && synced.id == snapshot.id
                        && synced.revision == snapshot.revision
                        && synced.text_epoch == snapshot.text_epoch
                        && synced.version == snapshot.version
                })
    }
    /// A native reply/action retains the client and synchronized document lifetime.
    /// The app also compares the live model, selections, focus and pane before edits.
    pub fn request_current(&self, request: &Request) -> bool {
        if !self.ready || !Arc::ptr_eq(&request.server, &self.identity) {
            return false;
        }
        let Ok(uri) = file_uri(&request.path) else {
            return false;
        };
        self.synced.get(&uri).is_some_and(|synced| {
            synced.path == request.path
                && synced.id == request.document_id
                && synced.revision == request.revision
                && synced.text_epoch == request.text_epoch
                && synced.version == request.sync_version
                && request.workspace.get(&request.path).is_none_or(|snapshot| {
                    snapshot.id == synced.id
                        && snapshot.revision == synced.revision
                        && snapshot.text_epoch == synced.text_epoch
                        && snapshot.version == synced.version
                })
        })
    }
    pub fn saved(&self, doc: &Document) -> Result<()> {
        if let Some(path) = &doc.path {
            self.saved_snapshot(path, &doc.text)?;
        }
        Ok(())
    }
    /// Notify a successful persistence operation using its committed bytes.
    /// Async callers must retain that Rope, rather than reading a newer live
    /// buffer after the filesystem worker finishes. This does not sync or edit
    /// any model; notifications are restricted to currently synchronized URIs.
    pub fn saved_snapshot(&self, path: &Path, committed: &ropey::Rope) -> Result<()> {
        if !self.ready {
            return Ok(());
        }
        let Some(include_text) = self.save_include_text() else {
            return Ok(());
        };
        if path.as_os_str().len() > 8192 {
            bail!("Saved document path exceeds 8 KiB");
        }
        let uri = file_uri(path)?;
        if !self.synced.contains_key(&uri) {
            return Ok(());
        }
        self.transport
            .saved_snapshot(uri, include_text.then(|| committed.clone()))
    }
    fn save_include_text(&self) -> Option<bool> {
        match self.capabilities.get("textDocumentSync")? {
            // Keep legacy numeric synchronization compatible with the official
            // language client's resolved options: Full/Incremental imply Save
            // without text; None does not request a save notification.
            Value::Number(kind) if matches!(kind.as_u64(), Some(1 | 2)) => Some(false),
            Value::Object(options) => match options.get("save")? {
                Value::Bool(true) => Some(false),
                Value::Object(save) => match save.get("includeText") {
                    None | Some(Value::Bool(false)) => Some(false),
                    Some(Value::Bool(true)) => Some(true),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }
    pub fn request(&mut self, method: &str, doc: &Document, extra: Value) -> Result<()> {
        self.request_in_view(method, doc, extra, None)
    }
    pub fn request_in_view(
        &mut self,
        method: &str,
        doc: &Document,
        extra: Value,
        view: Option<u64>,
    ) -> Result<()> {
        if !self.ready {
            bail!("Language server is still initializing");
        }
        let path = doc
            .path
            .clone()
            .context("Save this file before requesting language features")?;
        let uri = file_uri(&path)?;
        let sync_version = self
            .synced
            .get(&uri)
            .context("This language server does not handle this file type")?
            .version;
        if self.pending.len() >= 32 {
            bail!("Too many pending language requests");
        }
        let is_action = method == "textDocument/codeAction";
        if method == "textDocument/signatureHelp" && !self.signature_available() {
            bail!("A parameter hint request is still running or awaiting release");
        }
        if is_action {
            self.ensure_action_available()?;
        }
        if method == "textDocument/documentSymbol" {
            self.ensure_symbol_available()?;
        }
        if is_action && self.synced.len() > 128 {
            bail!("Code actions support at most 128 synchronized buffers");
        }
        let workspace = if is_action {
            self.synced
                .values()
                .map(|s| {
                    (
                        s.path.clone(),
                        Snapshot {
                            id: s.id,
                            revision: s.revision,
                            text_epoch: s.text_epoch,
                            version: s.version,
                        },
                    )
                })
                .collect()
        } else {
            HashMap::new()
        };
        let id = self.next_id;
        self.next_id += 1;
        let mut params = json!({"textDocument":{"uri":uri}});
        if !is_action && method != "textDocument/documentSymbol" {
            params["position"] = json!(position(doc, doc.cursor));
        }
        if let Some(extra) = extra.as_object() {
            params.as_object_mut().unwrap().extend(extra.clone());
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        self.pending.insert(
            id,
            Request {
                token: id,
                method: method.into(),
                document_id: doc.id,
                revision: doc.revision,
                text_epoch: doc.text_epoch(),
                server: self.identity.clone(),
                sync_version,
                cursor: doc.cursor,
                path,
                selections: if is_action {
                    doc.selections()
                } else {
                    Vec::new()
                },
                view,
                workspace: Arc::new(workspace),
                started: Instant::now(),
            },
        );
        if method == "textDocument/signatureHelp" {
            self.signature_occupied = Some(id);
        }
        if method == "textDocument/documentSymbol" {
            self.symbol_slot = Some(SymbolSlot {
                token: id,
                canceled: false,
                timed_out: false,
            });
        }
        Ok(())
    }
    pub(crate) fn symbol_available(&self) -> bool {
        self.ready && self.symbol_slot.is_none() && self.pending.len() < 32
    }
    pub(crate) fn symbol_channel_closed(&self) -> bool {
        self.symbol_slot.as_ref().is_some_and(|slot| slot.timed_out)
    }
    fn ensure_symbol_available(&self) -> Result<()> {
        if self.symbol_channel_closed() {
            bail!("Symbol request timed out; awaiting actual release or language server restart");
        }
        if !self.symbol_available() {
            bail!("A symbol request is still running or awaiting actual release");
        }
        Ok(())
    }
    pub(crate) fn request_document_symbols(&mut self, doc: &Document) -> Result<u64> {
        let token = self.next_id;
        self.request("textDocument/documentSymbol", doc, json!({}))?;
        Ok(token)
    }
    pub(crate) fn cancel_symbol_request(&mut self, token: u64) -> Result<()> {
        if let Some(slot) = &mut self.symbol_slot
            && slot.token == token
            && !slot.canceled
        {
            slot.canceled = true;
            self.notify("$/cancelRequest", json!({"id":token}))?;
        }
        Ok(())
    }
    pub(crate) fn workspace_symbols(&mut self, query: &str) -> Result<u64> {
        self.ensure_symbol_available()?;
        let id = self.next_id;
        self.next_id += 1;
        self.send(
            json!({"jsonrpc":"2.0","id":id,"method":"workspace/symbol","params":{"query":query}}),
        )?;
        self.pending.insert(
            id,
            Request {
                token: id,
                method: "workspace/symbol".into(),
                document_id: 0,
                revision: 0,
                text_epoch: 0,
                server: self.identity.clone(),
                sync_version: 0,
                cursor: 0,
                path: PathBuf::new(),
                selections: Vec::new(),
                view: None,
                workspace: Arc::new(HashMap::new()),
                started: Instant::now(),
            },
        );
        self.symbol_slot = Some(SymbolSlot {
            token: id,
            canceled: false,
            timed_out: false,
        });
        Ok(id)
    }
    pub fn action_available(&self) -> bool {
        self.action_channel_valid
            && !self.pending.values().any(|request| {
                matches!(
                    request.method.as_str(),
                    "textDocument/codeAction" | "codeAction/resolve" | "workspace/executeCommand"
                )
            })
    }
    pub(crate) fn action_channel_closed(&self) -> bool {
        !self.action_channel_valid
    }
    fn ensure_action_available(&self) -> Result<()> {
        if !self.action_channel_valid {
            bail!(
                "A language server code action timed out; restart the language server before requesting actions again"
            );
        }
        if !self.action_available() {
            bail!("A code action request is already running; retry shortly");
        }
        Ok(())
    }
    pub(crate) fn cancel_code_actions(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, request)| {
                matches!(
                    request.method.as_str(),
                    "textDocument/codeAction" | "codeAction/resolve" | "workspace/executeCommand"
                )
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            // Advisory cancellation cannot establish that server work has ended.
            // Keep both the original provenance and the actual occupied slot.
            self.notify("$/cancelRequest", json!({"id":id}))?;
        }
        Ok(())
    }
    pub fn ensure_command_available(&self) -> Result<()> {
        if !self.command_channel_valid {
            bail!(
                "A language server command timed out; restart the language server before running commands again"
            );
        }
        if self.pending.len() >= 32 {
            bail!("Too many pending language requests");
        }
        if self
            .pending
            .values()
            .any(|r| r.method == "workspace/executeCommand")
        {
            bail!("A language server command is already running");
        }
        Ok(())
    }
    pub fn follow_up(&mut self, original: &Request, method: &str, params: Value) -> Result<()> {
        if self.pending.len() >= 32 {
            bail!("Too many pending language requests");
        }
        if method == "workspace/executeCommand" {
            self.ensure_command_available()?;
        }
        if method == "codeAction/resolve" {
            self.ensure_action_available()?;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let mut request = original.clone();
        request.token = id;
        request.method = method.into();
        request.started = Instant::now();
        self.pending.insert(id, request);
        Ok(())
    }
    pub fn acknowledge_edit(&self, id: Value, result: Result<()>) -> Result<()> {
        let result = match result {
            Ok(()) => json!({"applied":true}),
            Err(error) => json!({"applied":false,"failureReason":format!("{error:#}")}),
        };
        self.send(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }

    pub(crate) fn cancel_signature_help(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, r)| r.method == "textDocument/signatureHelp")
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            // Cancellation is advisory. Only its exact response or process
            // retirement proves that this actual callback slot was released.
            self.notify("$/cancelRequest", json!({"id":id}))?;
        }
        Ok(())
    }
    pub(crate) fn signature_available(&self) -> bool {
        self.ready && self.signature_occupied.is_none() && self.pending.len() < 32
    }
    pub(crate) fn signature_channel_closed(&self) -> bool {
        self.signature_timed_out
    }
    pub(crate) fn request_signature_help(
        &mut self,
        doc: &Document,
        extra: Value,
        view: Option<u64>,
    ) -> Result<u64> {
        let token = self.next_id;
        self.request_in_view("textDocument/signatureHelp", doc, extra, view)?;
        Ok(token)
    }
    pub(crate) fn request_completion(
        &mut self,
        doc: &Document,
        extra: Value,
        view: Option<u64>,
    ) -> Result<u64> {
        let token = self.next_id;
        self.request_in_view("textDocument/completion", doc, extra, view)?;
        Ok(token)
    }
    pub(crate) fn request_code_actions(
        &mut self,
        doc: &Document,
        extra: Value,
        view: Option<u64>,
    ) -> Result<u64> {
        let token = self.next_id;
        self.request_in_view("textDocument/codeAction", doc, extra, view)?;
        Ok(token)
    }
    pub(crate) fn completion_resolve_available(&self) -> bool {
        self.ready
            && self.completion_resolve_valid
            && self.completion_resolve.is_none()
            && self.pending.len() < 32
    }
    pub(crate) fn completion_resolve_closed(&self) -> bool {
        !self.completion_resolve_valid
    }
    pub(crate) fn resolve_completion(&mut self, original: &Request, item: Value) -> Result<u64> {
        if !self.completion_resolve_available() {
            bail!("A completion resolution is still running");
        }
        let token = self.next_id;
        self.follow_up(original, "completionItem/resolve", item)?;
        self.completion_resolve = Some(token);
        Ok(token)
    }
    pub(crate) fn cancel_completion_resolve(&mut self, token: u64) -> Result<()> {
        // Keep the request and its slot until its response or transport deadline.
        // Repeated cursor/selection changes must not admit unbounded server work.
        if self.completion_resolve == Some(token) && self.pending.contains_key(&token) {
            self.notify("$/cancelRequest", json!({"id":token}))?;
        }
        Ok(())
    }
    pub(crate) fn identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    pub(crate) fn cancel_completions(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, request)| request.method == "textDocument/completion")
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.pending.remove(&id);
            self.notify("$/cancelRequest", json!({"id": id}))?;
        }
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Vec<Event>> {
        if self.transport.exited() {
            bail!("Language server process exited");
        }
        let mut events = Vec::new();
        if !self.ready && self.started.elapsed() > Duration::from_secs(30) {
            bail!("Language server initialization timed out");
        }
        for id in self
            .pending
            .iter()
            .filter(|(id, r)| {
                r.started.elapsed() > Duration::from_secs(15)
                    && !self
                        .symbol_slot
                        .as_ref()
                        .is_some_and(|slot| slot.token == **id && slot.timed_out)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
        {
            if let Some(slot) = &mut self.symbol_slot
                && slot.token == id
            {
                slot.timed_out = true;
                let request = self.pending.get(&id).unwrap().clone();
                events.push(Event::SymbolFailure(
                    request,
                    "Symbol request timed out; awaiting actual release or language server restart"
                        .into(),
                ));
                self.notify("$/cancelRequest", json!({"id":id}))?;
                continue;
            }
            let request = self.pending.remove(&id).unwrap();
            if request.method == "textDocument/signatureHelp" {
                // Retain the actual occupied token after the response deadline.
                // A late acknowledgement releases capacity without reviving it.
                self.signature_timed_out = true;
                events.push(Event::SignatureFailure(
                    request.clone(),
                    "Parameter hint request timed out; awaiting actual release or server restart"
                        .into(),
                ));
            }
            if matches!(
                request.method.as_str(),
                "textDocument/codeAction" | "codeAction/resolve"
            ) {
                self.action_channel_valid = false;
            }
            if self.completion_resolve == Some(id) {
                self.completion_resolve = None;
                // A timeout cannot prove that the canceled callback has finished.
                self.completion_resolve_valid = false;
            }
            if request.method == "workspace/executeCommand" {
                // Cancellation cannot establish that all server callbacks have stopped.
                self.command_channel_valid = false;
            }
            self.notify("$/cancelRequest", json!({"id":id}))?;
            events.push(Event::Message(format!(
                "Language request timed out: {}",
                request.method
            )));
        }
        for _ in 0..32 {
            let Some(message) = self.transport.receive()? else {
                break;
            };
            if let Some(method) = message["method"].as_str() {
                let params = &message["params"];
                if let Some(id) = message.get("id") {
                    if method == "workspace/applyEdit" {
                        let request = self
                            .pending
                            .values()
                            .find(|r| {
                                self.command_channel_valid && r.method == "workspace/executeCommand"
                            })
                            .cloned();
                        events.push(Event::ApplyEdit(
                            id.clone(),
                            request,
                            params["edit"].clone(),
                        ));
                        continue;
                    }
                    let result = match method {
                        "workspace/configuration" => Some(Value::Array(
                            params["items"]
                                .as_array()
                                .map_or(Vec::new(), |items| vec![json!({}); items.len()]),
                        )),
                        "workspace/workspaceFolders" => {
                            Some(json!([{"uri":self.root_uri,"name":"workspace"}]))
                        }
                        "window/workDoneProgress/create" | "window/showMessageRequest" => {
                            Some(Value::Null)
                        }
                        _ => None,
                    };
                    self.send(if let Some(result) = result { json!({"jsonrpc":"2.0","id":id,"result":result}) } else { json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Client method not supported"}}) })?;
                } else if method == "textDocument/publishDiagnostics" {
                    if let Some(uri) = params["uri"].as_str()
                        && let Some(synced) = self.synced.get(uri)
                        && match params.get("version") {
                            None => true,
                            Some(value) => value.as_i64() == Some(synced.version),
                        }
                    {
                        match publication_items(&params["diagnostics"]) {
                            Ok((items, serialized_bytes)) => {
                                events.push(Event::Diagnostics(DiagnosticPublication {
                                    uri: uri.to_owned(),
                                    path: synced.path.clone(),
                                    server: self.identity.clone(),
                                    snapshot: Snapshot {
                                        id: synced.id,
                                        revision: synced.revision,
                                        text_epoch: synced.text_epoch,
                                        version: synced.version,
                                    },
                                    versioned: params.get("version").is_some(),
                                    items,
                                    serialized_bytes,
                                }))
                            }
                            Err(error) => events.push(Event::Message(format!(
                                "Diagnostic publication rejected: {error:#}"
                            ))),
                        }
                    }
                } else if method == "window/showMessage"
                    && let Some(text) = params["message"].as_str()
                {
                    events.push(Event::Message(text.chars().take(2000).collect()));
                }
            } else if let Some(id) = message["id"].as_u64() {
                if self
                    .symbol_slot
                    .as_ref()
                    .is_some_and(|slot| slot.token == id)
                {
                    // Only an exact completed response releases actual capacity.
                    let result = message.get("result");
                    let error = message.get("error");
                    let completed = message["jsonrpc"] == "2.0"
                        && (matches!((result, error), (Some(_), None))
                            || matches!((result,error),(None,Some(error)) if error["code"].as_i64().is_some_and(|code| i32::try_from(code).is_ok()) && error["message"].as_str().is_some()));
                    if !completed {
                        continue;
                    }
                    let slot = self.symbol_slot.take().unwrap();
                    let request = self.pending.remove(&id).unwrap();
                    if !slot.canceled && !slot.timed_out {
                        if let Some(error) = error {
                            events.push(Event::SymbolFailure(request, format!("{error}")));
                        } else {
                            events.push(Event::Response(request, result.unwrap().clone()));
                        }
                    }
                    continue;
                }
                if self.signature_occupied == Some(id) {
                    if message.get("result").is_none() && message.get("error").is_none() {
                        // A malformed numeric-id message is not positive
                        // evidence that the actual callback has settled.
                        continue;
                    }
                    self.signature_occupied = None;
                    self.signature_timed_out = false;
                }
                if id == 0 && !self.ready {
                    if let Some(error) = message.get("error") {
                        bail!("Language server initialization failed: {error}");
                    }
                    self.capabilities = message["result"]["capabilities"].clone();
                    if self.capabilities["positionEncoding"]
                        .as_str()
                        .is_some_and(|s| s != "utf-16")
                    {
                        bail!("Language server selected an unsupported position encoding");
                    }
                    self.ready = true;
                    self.notify("initialized", json!({}))?;
                    events.push(Event::Ready);
                } else if let Some(request) = self.pending.remove(&id) {
                    if self.completion_resolve == Some(id) {
                        self.completion_resolve = None;
                    }
                    if request.method == "completionItem/resolve" && message.get("error").is_some()
                    {
                        events.push(Event::Response(
                            request,
                            json!({"_vscliResolveError":message["error"]}),
                        ));
                    } else if let Some(error) = message.get("error") {
                        if request.method == "textDocument/signatureHelp" {
                            events.push(Event::SignatureFailure(request, format!("{error}")));
                        } else {
                            events
                                .push(Event::Message(format!("Language request failed: {error}")));
                        }
                    } else {
                        events.push(Event::Response(request, message["result"].clone()));
                    }
                }
            }
        }
        Ok(events)
    }
}
#[cfg(test)]
mod save_notification_tests {
    use super::*;

    const PEER: &str = r#"
import json,sys
records=[]
def send(message):
    data=json.dumps({'jsonrpc':'2.0',**message}).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1)
        headers[key.lower()]=value.strip()
    message=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method,ident=message.get('method'),message.get('id')
    if method=='initialize':
        send({'id':ident,'result':{'capabilities':json.loads(sys.argv[1])}})
    elif method=='fixture/capture':
        send({'method':'window/showMessage','params':{'type':3,'message':'fixture-capture:'+json.dumps(records)}})
        records=[]
    elif method=='shutdown': send({'id':ident,'result':None})
    elif method=='exit': break
    else: records.append(message)
"#;
    fn start(root: &Path, capabilities: Value) -> Client {
        let mut client = Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                "-u".into(),
                "-c".into(),
                PEER.into(),
                capabilities.to_string(),
            ],
            root,
            "cpp".into(),
        )
        .unwrap();
        outline_tests::until(&mut client, |client, _| client.ready);
        client
    }
    fn capture(client: &mut Client) -> Vec<Value> {
        capture_with_warnings(client).0
    }
    fn capture_with_warnings(client: &mut Client) -> (Vec<Value>, Vec<String>) {
        // FIFO framed input guarantees this acknowledgement follows all prior
        // notifications, including the positive absence tests.
        client
            .fixture_notify("fixture/capture", Value::Null)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut warnings = Vec::new();
        loop {
            for event in client.poll().unwrap() {
                if let Event::Message(message) = event {
                    if let Some(records) = message.strip_prefix("fixture-capture:") {
                        return (serde_json::from_str(records).unwrap(), warnings);
                    }
                    warnings.push(message);
                }
            }
            assert!(Instant::now() < deadline, "{}", client.debug_summary());
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn document(root: &Path) -> Document {
        let path = root.join("猫🙂 main.cpp");
        std::fs::write(&path, "猫🙂 committed\r\nsecond\r\n").unwrap();
        Document::open_existing(&path).unwrap()
    }

    #[test]
    fn save_notifications_negotiate_boolean_object_numeric_and_absent_options() {
        let root = tempfile::tempdir().unwrap();
        let doc = document(root.path());
        let cases = [
            (json!({"textDocumentSync":{"save":true}}), Some(false)),
            (json!({"textDocumentSync":{"save":false}}), None),
            (json!({"textDocumentSync":{"save":{}}}), Some(false)),
            (
                json!({"textDocumentSync":{"save":{"includeText":false}}}),
                Some(false),
            ),
            (
                json!({"textDocumentSync":{"save":{"includeText":true}}}),
                Some(true),
            ),
            (
                json!({"textDocumentSync":{"openClose":true,"change":1}}),
                None,
            ),
            (json!({"textDocumentSync":0}), None),
            (json!({"textDocumentSync":1}), Some(false)),
            (json!({"textDocumentSync":2}), Some(false)),
            (json!({}), None),
            (
                json!({"textDocumentSync":{"save":{"includeText":"true"}}}),
                None,
            ),
            (json!({"textDocumentSync":3}), None),
        ];
        for (capabilities, expected) in cases {
            let mut client = start(root.path(), capabilities.clone());
            client.sync(std::slice::from_ref(&doc)).unwrap();
            client.saved(&doc).unwrap();
            let messages = capture(&mut client);
            let saves: Vec<_> = messages
                .iter()
                .filter(|message| message["method"] == "textDocument/didSave")
                .collect();
            assert_eq!(
                saves.len(),
                usize::from(expected.is_some()),
                "{capabilities}"
            );
            if let Some(include_text) = expected {
                let params = &saves[0]["params"];
                assert_eq!(
                    params["textDocument"]["uri"],
                    file_uri(doc.path.as_ref().unwrap()).unwrap()
                );
                assert_eq!(params.get("text").is_some(), include_text, "{capabilities}");
                if include_text {
                    assert_eq!(params["text"], doc.text.to_string());
                }
            }
        }
    }

    #[test]
    fn save_snapshot_uses_committed_unicode_crlf_bytes_independently_of_the_live_buffer() {
        let root = tempfile::tempdir().unwrap();
        let mut doc = document(root.path());
        let committed = doc.text.clone();
        let mut client = start(
            root.path(),
            json!({"textDocumentSync":{"save":{"includeText":true}}}),
        );
        client.sync(std::slice::from_ref(&doc)).unwrap();
        doc.move_to(2, false);
        doc.insert(" UNSAVED", false);
        let live = doc.text.to_string();
        client.sync(std::slice::from_ref(&doc)).unwrap();
        client
            .saved_snapshot(doc.path.as_ref().unwrap(), &committed)
            .unwrap();
        client.saved(&doc).unwrap();
        let messages = capture(&mut client);
        let saves: Vec<_> = messages
            .iter()
            .filter(|message| message["method"] == "textDocument/didSave")
            .collect();
        assert_eq!(saves.len(), 2);
        assert_eq!(saves[0]["params"]["text"], "猫🙂 committed\r\nsecond\r\n");
        assert_eq!(saves[1]["params"]["text"], live);
        assert_ne!(saves[0]["params"]["text"], saves[1]["params"]["text"]);
        assert_eq!(doc.text.to_string(), live);
        assert!(doc.dirty());
        assert_eq!(
            std::fs::read(doc.path.as_ref().unwrap()).unwrap(),
            committed.to_string().as_bytes()
        );
    }

    #[test]
    fn unsynchronized_and_closed_paths_do_not_emit_save_notifications() {
        let root = tempfile::tempdir().unwrap();
        let doc = document(root.path());
        let path = doc.path.as_ref().unwrap();
        let mut client = start(
            root.path(),
            json!({"textDocumentSync":{"save":{"includeText":true}}}),
        );
        client.saved_snapshot(path, &doc.text).unwrap();
        assert!(
            capture(&mut client)
                .iter()
                .all(|message| message["method"] != "textDocument/didSave")
        );
        client.sync(std::slice::from_ref(&doc)).unwrap();
        client.saved_snapshot(path, &doc.text).unwrap();
        assert_eq!(
            capture(&mut client)
                .iter()
                .filter(|message| message["method"] == "textDocument/didSave")
                .count(),
            1
        );
        client.sync(&[]).unwrap();
        client.saved_snapshot(path, &doc.text).unwrap();
        assert!(
            capture(&mut client)
                .iter()
                .all(|message| message["method"] != "textDocument/didSave")
        );
    }

    #[test]
    fn oversized_raw_and_escaped_snapshots_fail_before_publish_without_poisoning_the_channel() {
        let root = tempfile::tempdir().unwrap();
        let doc = document(root.path());
        let path = doc.path.as_ref().unwrap();
        let mut client = start(
            root.path(),
            json!({"textDocumentSync":{"save":{"includeText":true}}}),
        );
        client.sync(std::slice::from_ref(&doc)).unwrap();
        let raw = ropey::Rope::from_str(&"a".repeat(16 * 1024 * 1024 + 1));
        assert!(
            client
                .saved_snapshot(path, &raw)
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
        let escaped = ropey::Rope::from_str(&"\0".repeat(3 * 1024 * 1024));
        // Raw length admission is constant-time; the worker detects escaped
        // overflow before emitting any frame, and reports it independently of
        // the successful filesystem save without retiring the language server.
        client.saved_snapshot(path, &escaped).unwrap();
        client.saved_snapshot(path, &doc.text).unwrap();
        let (messages, warnings) = capture_with_warnings(&mut client);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Save succeeded; LSP save notification failed"));
        assert!(warnings[0].contains("16 MiB"));
        let saves: Vec<_> = messages
            .iter()
            .filter(|message| message["method"] == "textDocument/didSave")
            .collect();
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0]["params"]["text"], doc.text.to_string());
        // A server that did not request text does not flatten or reject a large
        // committed Rope: the small notification carries only its URI.
        client.capabilities = json!({"textDocumentSync":{"save":true}});
        client.saved_snapshot(path, &raw).unwrap();
        let messages = capture(&mut client);
        assert_eq!(messages.len(), 1);
        assert!(messages[0]["params"].get("text").is_none());
    }
}

#[cfg(test)]
pub(crate) mod outline_tests {
    use super::*;
    const PEER: &str = r#"
import json,sys
held={}
def send(message):
    data=json.dumps({'jsonrpc':'2.0',**message}).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
def notice(text): send({'method':'window/showMessage','params':{'type':3,'message':text}})
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1)
        headers[key.lower()]=value.strip()
    message=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method,ident,params=message.get('method'),message.get('id'),message.get('params',{})
    if method=='initialize': send({'id':ident,'result':{'capabilities':{'textDocumentSync':1,'documentSymbolProvider':True,'workspaceSymbolProvider':True}}})
    elif method in ('textDocument/documentSymbol','workspace/symbol'):
        held[ident]=method
        notice('held-%d'%ident)
        if method=='workspace/symbol': notice('query-'+params['query'])
    elif method=='fixture/release':
        ident=params['id']
        held.pop(ident)
        if params.get('fail'): send({'id':ident,'error':{'code':-32603,'message':'fixture symbol failure'}})
        else: send({'id':ident,'result':params.get('result',[])})
    elif method=='fixture/message': send(params)
    elif method=='fixture/ack': notice('ack')
    elif method=='textDocument/hover': send({'id':ident,'result':None})
    elif method=='shutdown': send({'id':ident,'result':None})
    elif method=='exit': break
"#;
    pub(crate) fn until(
        client: &mut Client,
        predicate: impl Fn(&Client, &[Event]) -> bool,
    ) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let events = client.poll().unwrap();
            if predicate(client, &events) {
                return events;
            }
            assert!(Instant::now() < deadline, "{}", client.debug_summary());
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    pub(crate) fn start(root: &Path, doc: &Document) -> Client {
        let mut client = Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &["-u".into(), "-c".into(), PEER.into()],
            root,
            "cpp".into(),
        )
        .unwrap();
        until(&mut client, |client, _| client.ready);
        client.sync(std::slice::from_ref(doc)).unwrap();
        client
    }
    pub(crate) fn held(client: &mut Client, token: u64) {
        until(client, |_, events| {
            events.iter().any(|event|matches!(event,Event::Message(message) if message == &format!("held-{token}")))
        });
    }
    #[test]
    fn symbol_actual_slot_survives_ignored_cancel_timeout_and_malformed_acknowledgements() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        std::fs::write(&path, "猫🙂\r\n").unwrap();
        let doc = Document::open_existing(&path).unwrap();
        let mut client = start(root.path(), &doc);
        let token = client.request_document_symbols(&doc).unwrap();
        held(&mut client, token);
        for _ in 0..32 {
            client.cancel_symbol_request(token).unwrap();
            assert!(!client.symbol_available());
            assert!(client.request_document_symbols(&doc).is_err());
            assert!(client.workspace_symbols("latest").is_err());
        }
        assert_eq!(client.pending.len(), 1);
        for message in [
            json!({"id":token}),
            json!({"id":token,"error":null}),
            json!({"id":token,"result":[],"error":{"code":-1,"message":"both"}}),
            json!({"id":token+999,"result":[]}),
            json!({"id":token.to_string(),"result":[]}),
        ] {
            client.notify("fixture/message", message).unwrap();
        }
        client.notify("fixture/ack", json!({})).unwrap();
        until(&mut client, |_, events| {
            events
                .iter()
                .any(|event| matches!(event,Event::Message(message) if message=="ack"))
        });
        assert_eq!(client.symbol_slot.as_ref().unwrap().token, token);
        for message in [
            json!({"jsonrpc":"1.0","id":token,"result":[]}),
            json!({"id":token,"error":{"code":(i32::MAX as i64)+1,"message":"invalid code"}}),
        ] {
            client.notify("fixture/message", message).unwrap();
            client.notify("fixture/ack", json!({})).unwrap();
            until(&mut client, |_, events| {
                events
                    .iter()
                    .any(|event| matches!(event, Event::Message(message) if message == "ack"))
            });
            assert_eq!(client.symbol_slot.as_ref().unwrap().token, token);
            assert!(!client.symbol_available());
        }
        client.pending.get_mut(&token).unwrap().started = Instant::now() - Duration::from_secs(16);
        let events = client.poll().unwrap();
        assert!(events.iter().any(|event|matches!(event,Event::SymbolFailure(request,message) if request.token==token && message.contains("timed out"))));
        assert!(client.symbol_channel_closed());
        assert!(!client.symbol_available());
        assert_eq!(client.pending.len(), 1);
        assert!(
            !client
                .poll()
                .unwrap()
                .iter()
                .any(|event| matches!(event, Event::SymbolFailure(_, _)))
        );
        client
            .notify("fixture/release", json!({"id":token}))
            .unwrap();
        let late = until(&mut client, |client, _| client.symbol_available());
        assert!(
            !late
                .iter()
                .any(|event| matches!(event,Event::Response(request,_) if request.token==token))
        );
        assert!(!client.symbol_channel_closed());
        let failure = client.workspace_symbols("latest").unwrap();
        held(&mut client, failure);
        client
            .notify("fixture/release", json!({"id":failure,"fail":true}))
            .unwrap();
        until(&mut client, |_, events| {
            events.iter().any(|event|matches!(event,Event::SymbolFailure(request,message) if request.token==failure && message.contains("fixture symbol failure")))
        });
        assert!(client.symbol_available());
        let fresh = client.request_document_symbols(&doc).unwrap();
        held(&mut client, fresh);
        client
            .notify("fixture/release", json!({"id":fresh}))
            .unwrap();
        until(&mut client, |_, events| {
            events.iter().any(|event|matches!(event,Event::Response(request,value) if request.token==fresh && value==&json!([])))
        });
        assert!(client.symbol_available());
        assert_eq!(std::fs::read(&path).unwrap(), "猫🙂\r\n".as_bytes());
    }
    #[test]
    fn symbol_replies_retain_exact_server_and_synchronized_model_lifetime() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        std::fs::write(&path, "猫🙂\r\n").unwrap();
        let mut doc = Document::open_existing(&path).unwrap();
        let mut client = start(root.path(), &doc);
        let token = client.request_document_symbols(&doc).unwrap();
        held(&mut client, token);
        let request = client.pending[&token].clone();
        let revision = doc.revision;
        let selection = doc.selections();
        doc.insert("dirty", false);
        doc.undo();
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.selections(), selection);
        assert_ne!(doc.text_epoch(), request.text_epoch);
        client.sync(std::slice::from_ref(&doc)).unwrap();
        assert!(!client.request_current(&request));
        client
            .notify("fixture/release", json!({"id":token}))
            .unwrap();
        let responses = until(&mut client, |client, _| client.symbol_available());
        assert!(responses.iter().any(|event|matches!(event,Event::Response(reply,_) if reply.token==token && !client.request_current(reply))));
        let fresh = client.request_document_symbols(&doc).unwrap();
        held(&mut client, fresh);
        let request = client.pending[&fresh].clone();
        client.sync(&[]).unwrap();
        client.sync(std::slice::from_ref(&doc)).unwrap();
        assert!(!client.request_current(&request));
        let replacement = start(root.path(), &doc);
        assert!(!replacement.request_current(&request));
        client.cancel_symbol_request(fresh).unwrap();
        client
            .notify("fixture/release", json!({"id":fresh}))
            .unwrap();
        until(&mut client, |client, _| client.symbol_available());
        doc.redo();
        assert_eq!(doc.text.to_string(), "dirty猫🙂\r\n");
        assert_eq!(std::fs::read(&path).unwrap(), "猫🙂\r\n".as_bytes());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::read_message;
    use std::io::BufReader;
    #[test]
    fn signature_actual_slot_survives_cancellation_and_timeout_until_exact_release() {
        const PEER: &str = r#"
import json,sys
held={}
def send(message):
    data=json.dumps({'jsonrpc':'2.0',**message}).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
def notice(value): send({'method':'window/showMessage','params':{'type':3,'message':value}})
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1)
        headers[key.lower()]=value.strip()
    message=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method,ident,params=message.get('method'),message.get('id'),message.get('params',{})
    if method=='initialize': send({'id':ident,'result':{'capabilities':{'textDocumentSync':1,'signatureHelpProvider':{'triggerCharacters':['(', ',']}}}})
    elif method=='textDocument/signatureHelp':
        if params.get('fail'): send({'id':ident,'error':{'code':-32603,'message':'fixture failure'}})
        elif params.get('hold'):
            held[ident]={'signatures':[{'label':'held(int value)'}]}
            notice('held-%d'%ident)
        else: send({'id':ident,'result':{'signatures':[{'label':'fresh()'}]}})
    elif method=='$/cancelRequest': notice('ignored-cancel-%d'%params['id'])
    elif method=='fixture/release': send({'id':params['id'],'result':held.pop(params['id'])})
    elif method=='fixture/unknown': send({'id':params['id'],'result':None})
    elif method=='textDocument/hover': send({'id':ident,'result':None})
    elif method=='shutdown': send({'id':ident,'result':None})
    elif method=='exit': break
"#;
        fn until(client: &mut Client, predicate: impl Fn(&Client, &[Event]) -> bool) -> Vec<Event> {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let events = client.poll().unwrap();
                if predicate(client, &events) {
                    return events;
                }
                assert!(Instant::now() < deadline, "{}", client.debug_summary());
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        std::fs::write(&path, "sum(猫🙂)\r\n").unwrap();
        let doc = Document::open(&path).unwrap();
        let mut client = Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &["-u".into(), "-c".into(), PEER.into()],
            root.path(),
            "cpp".into(),
        )
        .unwrap();
        until(&mut client, |client, _| client.ready);
        client.sync(std::slice::from_ref(&doc)).unwrap();
        let token = client
            .request_signature_help(&doc, json!({"hold":true}), Some(17))
            .unwrap();
        until(&mut client, |_, events| {
            events.iter().any(|event| matches!(event, Event::Message(message) if message == &format!("held-{token}")))
        });
        for _ in 0..32 {
            client.cancel_signature_help().unwrap();
            assert!(!client.signature_available());
            assert!(
                client
                    .request_signature_help(&doc, json!({}), None)
                    .is_err()
            );
        }
        assert_eq!(client.pending.len(), 1);
        assert_eq!(client.signature_occupied, Some(token));
        client.pending.get_mut(&token).unwrap().started = Instant::now() - Duration::from_secs(16);
        let events = client.poll().unwrap();
        assert!(events.iter().any(|event| matches!(event, Event::SignatureFailure(request, message) if request.token == token && message.contains("timed out"))));
        assert!(client.signature_channel_closed());
        assert!(!client.signature_available());
        client
            .notify("fixture/unknown", json!({"id":token + 999}))
            .unwrap();
        client
            .request("textDocument/hover", &doc, json!({}))
            .unwrap();
        until(&mut client, |_, events| {
            events.iter().any(|event| matches!(event, Event::Response(request, _) if request.method == "textDocument/hover"))
        });
        assert!(client.signature_channel_closed());
        client
            .notify("fixture/release", json!({"id":token}))
            .unwrap();
        let late = until(&mut client, |client, _| client.signature_available());
        assert!(
            !late.iter().any(
                |event| matches!(event, Event::Response(request, _) if request.token == token)
            )
        );
        assert!(!client.signature_channel_closed());
        let failure = client
            .request_signature_help(&doc, json!({"fail":true}), None)
            .unwrap();
        until(&mut client, |_, events| {
            events.iter().any(|event| matches!(event, Event::SignatureFailure(request, message) if request.token == failure && message.contains("fixture failure")))
        });
        assert!(client.signature_available());
        let fresh = client
            .request_signature_help(&doc, json!({}), None)
            .unwrap();
        until(&mut client, |_, events| {
            events.iter().any(|event| matches!(event, Event::Response(request, value) if request.token == fresh && value["signatures"][0]["label"] == "fresh()"))
        });
        assert!(client.signature_available());
        assert_eq!(std::fs::read(path).unwrap(), "sum(猫🙂)\r\n".as_bytes());
    }

    #[test]
    fn ignored_action_cancellation_retains_slots_and_timeout_blocks_further_action_work() {
        // A real framed peer confirms receipt and explicitly ignores cancellation.
        // Only the client's private timer is advanced; there is no deadline sleep.
        const PEER: &str = r#"
import json,sys
held={}
def send(message):
    data=json.dumps({'jsonrpc':'2.0',**message}).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
def notice(value): send({'method':'window/showMessage','params':{'type':3,'message':value}})
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1)
        headers[key.lower()]=value.strip()
    message=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method,ident,params=message.get('method'),message.get('id'),message.get('params',{})
    if method=='initialize':
        send({'id':ident,'result':{'capabilities':{'textDocumentSync':1,'codeActionProvider':{'resolveProvider':True}}}})
    elif method in ('textDocument/codeAction','codeAction/resolve','workspace/executeCommand'):
        result=[{'title':'Lazy action','data':{}}] if method=='textDocument/codeAction' else None
        if params.get('hold'):
            held[ident]=result
            notice('held-%d'%ident)
        else: send({'id':ident,'result':result})
    elif method=='$/cancelRequest': notice('ignored-cancel-%d'%params['id'])
    elif method=='fixture/release':
        token=params['id']
        send({'id':token,'result':held.pop(token)})
        notice('released-%d'%token)
    elif method=='textDocument/hover': send({'id':ident,'result':None})
    elif method=='shutdown': send({'id':ident,'result':None})
    elif method=='exit': break
"#;
        for method in [
            "textDocument/codeAction",
            "codeAction/resolve",
            "workspace/executeCommand",
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("main.rs");
            std::fs::write(&path, "猫🙂\r\n").unwrap();
            let doc = Document::open(&path).unwrap();
            let mut client = Client::start(
                if cfg!(windows) { "python" } else { "python3" },
                &["-u".into(), "-c".into(), PEER.into()],
                root.path(),
                "rust".into(),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !client.ready {
                client.poll().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            client.sync(std::slice::from_ref(&doc)).unwrap();
            let first = client
                .request_code_actions(
                    &doc,
                    json!({"hold":method == "textDocument/codeAction"}),
                    None,
                )
                .unwrap();
            let original = if method == "textDocument/codeAction" {
                None
            } else {
                Some(loop {
                    if let Some(request) =
                        client
                            .poll()
                            .unwrap()
                            .into_iter()
                            .find_map(|event| match event {
                                Event::Response(request, _) if request.token == first => {
                                    Some(request)
                                }
                                _ => None,
                            })
                    {
                        break request;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                })
            };
            let token = if let Some(original) = &original {
                let token = client.next_id;
                client
                    .follow_up(original, method, json!({"hold":true}))
                    .unwrap();
                token
            } else {
                first
            };
            for expected in [format!("held-{token}"), format!("ignored-cancel-{token}")] {
                if expected.starts_with("ignored") {
                    client.cancel_code_actions().unwrap();
                }
                loop {
                    let events = client.poll().unwrap();
                    assert!(!events.iter().any(|event| matches!(event, Event::Response(request, _) if request.token == token)));
                    if events.iter().any(
                        |event| matches!(event, Event::Message(message) if message == &expected),
                    ) {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
                assert!(!client.action_available());
                assert_eq!(client.pending.len(), 1, "{method}");
                for _ in 0..32 {
                    assert!(client.request_code_actions(&doc, json!({}), None).is_err());
                    if let Some(original) = &original {
                        assert!(
                            client
                                .follow_up(original, method, json!({"hold":true}))
                                .is_err()
                        );
                    }
                    assert_eq!(
                        client.pending.len(),
                        1,
                        "Ignored cancellation must retain real capacity"
                    );
                }
            }
            client.pending.get_mut(&token).unwrap().started =
                Instant::now() - Duration::from_secs(16);
            assert!(client.poll().unwrap().iter().any(
                |event| matches!(event, Event::Message(message) if message.contains("timed out"))
            ));
            assert!(client.pending.is_empty());
            let before = client.next_id;
            for _ in 0..32 {
                let error = if method == "workspace/executeCommand" {
                    client
                        .follow_up(original.as_ref().unwrap(), method, json!({"hold":true}))
                        .unwrap_err()
                } else {
                    let error = client
                        .request_code_actions(&doc, json!({}), None)
                        .unwrap_err();
                    if let Some(original) = &original {
                        assert!(
                            client
                                .follow_up(original, method, json!({"hold":true}))
                                .unwrap_err()
                                .to_string()
                                .contains("restart")
                        );
                    }
                    assert!(!client.action_available());
                    error
                };
                assert!(error.to_string().contains("restart"));
                assert_eq!(
                    client.next_id, before,
                    "A timeout cannot admit more ignored server work"
                );
            }
            client
                .notify("fixture/release", json!({"id":token}))
                .unwrap();
            loop {
                let events = client.poll().unwrap();
                assert!(!events.iter().any(
                    |event| matches!(event, Event::Response(request, _) if request.token == token)
                ));
                if events.iter().any(|event| matches!(event, Event::Message(message) if message == &format!("released-{token}"))) { break; }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            // The closed channel is source-specific: unrelated language reads work.
            client
                .request("textDocument/hover", &doc, json!({}))
                .unwrap();
            loop {
                if client.poll().unwrap().iter().any(|event| matches!(event, Event::Response(request, _) if request.method == "textDocument/hover")) { break; }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(std::fs::read(&path).unwrap(), "猫🙂\r\n".as_bytes());
            assert_eq!(doc.text.to_string(), "猫🙂\r\n");
        }
    }
    #[test]
    fn timed_out_completion_resolution_cannot_admit_more_ignored_server_work() {
        let root = tempfile::tempdir().unwrap();
        for marker in ["resolve", "resolve_hold"] {
            std::fs::write(root.path().join(marker), "").unwrap();
        }
        let path = root.path().join("main.rs");
        std::fs::write(&path, "ans").unwrap();
        let doc = Document::open(&path).unwrap();
        let mut client = Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                format!(
                    "{}/tests/fixtures/suggestions_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                root.path().to_string_lossy().into(),
            ],
            root.path(),
            "rust".into(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready {
            client.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        client.sync(std::slice::from_ref(&doc)).unwrap();
        client.request_completion(&doc, json!({}), None).unwrap();
        let (request, item) = loop {
            if let Some(response) = client
                .poll()
                .unwrap()
                .into_iter()
                .find_map(|event| match event {
                    Event::Response(request, value) => Some((request, value[0].clone())),
                    _ => None,
                })
            {
                break response;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        let token = client.resolve_completion(&request, item.clone()).unwrap();
        client.cancel_completion_resolve(token).unwrap();
        assert!(!client.completion_resolve_available());
        assert!(client.resolve_completion(&request, item.clone()).is_err());
        client.pending.get_mut(&token).unwrap().started = Instant::now() - Duration::from_secs(16);
        client.poll().unwrap();
        assert!(client.completion_resolve_closed());
        assert!(!client.completion_resolve_available());
        assert!(client.resolve_completion(&request, item).is_err());
        std::fs::remove_file(root.path().join("resolve_hold")).unwrap();
        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            assert!(
                !client
                    .poll()
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event, Event::Response(_, _)))
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(client.completion_resolve_closed());
    }
    #[test]
    fn timed_out_command_disables_new_commands_and_late_callbacks_until_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.rs");
        std::fs::write(&path, "bad").unwrap();
        let mut doc = Document::open(&path).unwrap();
        let mut client = Client::start(
            "python3",
            &[format!(
                "{}/tests/fixtures/code_action_server.py",
                env!("CARGO_MANIFEST_DIR")
            )],
            root.path(),
            "rust".into(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready {
            client.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        client.sync(std::slice::from_ref(&doc)).unwrap();
        client.request("textDocument/codeAction", &doc, json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"context":{"diagnostics":[]}})).unwrap();
        let original = loop {
            let response = client
                .poll()
                .unwrap()
                .into_iter()
                .find_map(|event| match event {
                    Event::Response(request, _) => Some(request),
                    _ => None,
                });
            if let Some(request) = response {
                break request;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        let command = json!({"command":"fixture.action","arguments":[{"uri":file_uri(doc.path.as_ref().unwrap()).unwrap(),"version":1,"span":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}}}]});
        client
            .follow_up(&original, "workspace/executeCommand", command.clone())
            .unwrap();
        client.pending.values_mut().next().unwrap().started =
            Instant::now() - Duration::from_secs(16);
        assert!(client.poll().unwrap().iter().any(
            |event| matches!(event, Event::Message(message) if message.contains("timed out"))
        ));
        doc.insert("new", false);
        client.sync(std::slice::from_ref(&doc)).unwrap();
        let mut newer = original.clone();
        newer.revision = doc.revision;
        let error = client
            .follow_up(&newer, "workspace/executeCommand", command)
            .unwrap_err();
        assert!(error.to_string().contains("restart the language server"));
        loop {
            let callback = client
                .poll()
                .unwrap()
                .into_iter()
                .find_map(|event| match event {
                    Event::ApplyEdit(id, request, _) => Some((id, request)),
                    _ => None,
                });
            if let Some((id, request)) = callback {
                assert!(
                    request.is_none(),
                    "Late callback must not acquire a newer snapshot"
                );
                client
                    .acknowledge_edit(id, Err(anyhow::anyhow!("Timed-out command")))
                    .unwrap();
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(client.ensure_command_available().is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"bad");
    }
    #[test]
    fn protocol_paths_preserve_document_identity_across_uri_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spaces 🌍.rs");
        std::fs::write(&path, "text").unwrap();
        let doc = Document::open(&path).unwrap();
        let native = doc.path.as_ref().unwrap();
        assert_eq!(&uri_path(&file_uri(native).unwrap()).unwrap(), native);
        assert_eq!(&uri_path(&file_uri(&path).unwrap()).unwrap(), native);
    }
    #[test]
    fn utf16_edits_are_validated_before_mutation() {
        let mut doc = Document::from_text("猫🙂x\r\nend");
        assert_eq!(
            position(&doc, 2),
            Position {
                line: 0,
                character: 3
            }
        );
        assert_eq!(
            offset(
                &doc,
                Position {
                    line: 0,
                    character: 3
                }
            )
            .unwrap(),
            2
        );
        assert!(
            offset(
                &doc,
                Position {
                    line: 0,
                    character: 2
                }
            )
            .is_err()
        );
        let edit = TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 1,
                },
                end: Position {
                    line: 0,
                    character: 3,
                },
            },
            new_text: "done".into(),
        };
        assert!(edits(&doc, vec![edit.clone(), edit.clone()]).is_err());
        let changes = edits(&doc, vec![edit]).unwrap();
        doc.apply_changes(changes);
        assert_eq!(doc.text.to_string(), "猫donex\r\nend");
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂x\r\nend");
    }
    #[test]
    fn framing_accepts_fragmented_messages_and_rejects_oversized_or_duplicate_lengths() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":"ok"}"#;
        let packet = format!(
            "Content-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );
        let mut reader = BufReader::with_capacity(1, packet.as_bytes());
        assert_eq!(read_message(&mut reader).unwrap()["result"], "ok");
        assert!(
            read_message(&mut BufReader::new(
                &b"Content-Length: 999999999\r\n\r\n"[..]
            ))
            .is_err()
        );
        assert!(
            read_message(&mut BufReader::new(
                &b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}"[..]
            ))
            .is_err()
        );
    }
}
