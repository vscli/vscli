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
pub struct Client {
    identity: Arc<()>,
    transport: crate::transport::Process,
    pending: HashMap<u64, Request>,
    completion_resolve: Option<u64>,
    completion_resolve_valid: bool,
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
        if self.ready
            && let Some(path) = &doc.path
        {
            let uri = file_uri(path)?;
            if self.synced.contains_key(&uri) {
                self.notify(
                    "textDocument/didSave",
                    json!({"textDocument":{"uri":uri},"text":doc.text.to_string()}),
                )?;
            }
        }
        Ok(())
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
        if is_action {
            self.ensure_action_available()?;
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
        Ok(())
    }
    pub(crate) fn has_symbol_request(&self) -> bool {
        self.pending.values().any(|r| {
            matches!(
                r.method.as_str(),
                "textDocument/documentSymbol" | "workspace/symbol"
            )
        })
    }
    pub(crate) fn cancel_symbol_requests(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, r)| {
                matches!(
                    r.method.as_str(),
                    "textDocument/documentSymbol" | "workspace/symbol"
                )
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.pending.remove(&id);
            self.notify("$/cancelRequest", json!({"id":id}))?;
        }
        Ok(())
    }
    pub(crate) fn workspace_symbols(&mut self, query: &str) -> Result<()> {
        if !self.ready || self.pending.len() >= 32 {
            bail!("Language server unavailable or request queue full");
        }
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
        Ok(())
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
            self.pending.remove(&id);
            self.notify("$/cancelRequest", json!({"id":id}))?;
        }
        Ok(())
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
            .filter(|(_, r)| r.started.elapsed() > Duration::from_secs(15))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
        {
            let request = self.pending.remove(&id).unwrap();
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
                        events.push(Event::Message(format!("Language request failed: {error}")));
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
mod tests {
    use super::*;
    use crate::transport::read_message;
    use std::io::BufReader;
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
