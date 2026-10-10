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
    FormattingResult(Request, Result<Value, String>),
    ActionResult(Request, Result<Value, String>),
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
struct FormattingSlot {
    token: u64,
    canceled: bool,
    timed_out: bool,
}
struct ActionSlot {
    token: u64,
    canceled: bool,
    timed_out: bool,
    resolve_original: Option<Value>,
}

const ACTION_BYTES: usize = 2 * 1024 * 1024;

/// Validate unknown resolve data before copying it or recursively serializing it.
fn action_budget(value: &Value) -> Result<()> {
    const NODES: usize = 131_072;
    let mut stack = vec![(value, 0usize)];
    let mut nodes = 0usize;
    let mut string_bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > NODES || depth > 64 {
            bail!("Code action exceeds its node or nesting budget");
        }
        match value {
            Value::String(text) => string_bytes = string_bytes.saturating_add(text.len()),
            Value::Array(items) => {
                if items.len() > NODES.saturating_sub(nodes + stack.len()) {
                    bail!("Code action exceeds its node budget");
                }
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(items) => {
                if items.len() > NODES.saturating_sub(nodes + stack.len()) {
                    bail!("Code action exceeds its node budget");
                }
                for (key, item) in items {
                    string_bytes = string_bytes.saturating_add(key.len());
                    stack.push((item, depth + 1));
                }
            }
            _ => {}
        }
        if string_bytes > ACTION_BYTES {
            bail!("Code action exceeds 2 MiB");
        }
    }
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > ACTION_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other("Code action exceeds 2 MiB"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(0), value)?;
    Ok(())
}

fn validate_action_item(item: &Value) -> Result<()> {
    let object = item.as_object().context("Invalid code action object")?;
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .context("Invalid code action title")?;
    if title.len() > 8192 {
        bail!("Code action title exceeds 8 KiB");
    }
    Ok(())
}

fn validate_action_result(method: &str, value: &Value, original: Option<&Value>) -> Result<()> {
    if method == "textDocument/codeAction" {
        if !value.is_null() {
            let items = value.as_array().context("Invalid code action list")?;
            if items.len() > 300 {
                bail!("Code action source exceeds 300 items");
            }
            action_budget(value)?;
            for item in items {
                validate_action_item(item)?;
            }
        }
    } else if method == "codeAction/resolve" {
        action_budget(value)?;
        validate_action_item(value)?;
        let original = original.context("Missing original code action")?;
        for key in [
            "title",
            "kind",
            "diagnostics",
            "isPreferred",
            "disabled",
            "command",
            "data",
        ] {
            if original.get(key) != value.get(key) {
                bail!("Code action resolution changed immutable field {key}");
            }
        }
    } else {
        action_budget(value)?;
    }
    Ok(())
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
    formatting_slot: Option<FormattingSlot>,
    synced: HashMap<String, Synced>,
    next_id: u64,
    action_slot: Option<ActionSlot>,
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
            formatting_slot: None,
            synced: HashMap::new(),
            next_id: 1,
            action_slot: None,
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
    fn request_identity(&self) -> Result<(u64, u64)> {
        let id = self.next_id;
        if id == 0 || self.pending.contains_key(&id) {
            bail!("Language request identity is no longer available; restart the server");
        }
        let next = id
            .checked_add(1)
            .context("Language request identity exhausted; restart the server")?;
        Ok((id, next))
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
        if matches!(method, "codeAction/resolve" | "workspace/executeCommand") {
            bail!("Action follow-ups require an original source-owned action request");
        }
        if !self.ready {
            bail!("Language server is still initializing");
        }
        let formatting = method == "textDocument/formatting";
        if formatting {
            self.ensure_formatting_available()?;
            if doc.path.as_ref().is_none_or(|path| {
                path.as_os_str().len() > 4096 || path.as_os_str().as_encoded_bytes().contains(&0)
            }) {
                bail!("Native formatting requires a saved path of at most 4 KiB");
            }
            let object = extra
                .as_object()
                .context("Invalid native formatting parameters")?;
            if object.len() != 1 || !object.contains_key("options") {
                bail!("Native formatting accepts only formatting options");
            }
            let options = extra["options"]
                .as_object()
                .context("Invalid native formatting options")?;
            if options.len() > 16
                || !options
                    .get("tabSize")
                    .and_then(Value::as_u64)
                    .is_some_and(|n| (1..=32).contains(&n))
                || !options.get("insertSpaces").is_some_and(Value::is_boolean)
                || options.iter().any(|(key, value)| {
                    key.len() > 128
                        || key.contains('\0')
                        || !(value.is_boolean()
                            || value.is_number()
                            || value.as_str().is_some_and(|text| text.len() <= 4096))
                })
            {
                bail!("Invalid or oversized native formatting options");
            }
        }
        let path = doc
            .path
            .clone()
            .context("Save this file before requesting language features")?;
        let uri = file_uri(&path)?;
        let synced = self
            .synced
            .get(&uri)
            .context("This language server does not handle this file type")?;
        if (formatting || method == "textDocument/codeAction")
            && (synced.id != doc.id
                || synced.path != path
                || synced.revision != doc.revision
                || synced.text_epoch != doc.text_epoch())
        {
            bail!("Document has not been synchronized; retry after language synchronization");
        }
        let sync_version = synced.version;
        if self.pending.len() >= 32 {
            bail!("Too many pending language requests");
        }
        let is_action = method == "textDocument/codeAction";
        if method == "textDocument/signatureHelp" && !self.signature_available() {
            bail!("A parameter hint request is still running or awaiting release");
        }
        if is_action {
            self.ensure_action_available()?;
            action_budget(&extra)?;
            let object = extra
                .as_object()
                .context("Invalid code action parameters")?;
            if object.contains_key("textDocument") || object.contains_key("position") {
                bail!("Code action parameters cannot replace the captured document");
            }
            if doc.secondary.len() >= 10_000 {
                bail!("Code actions support at most 10000 selections");
            }
        }
        if method == "textDocument/documentSymbol" {
            self.ensure_symbol_available()?;
        }
        if is_action && self.synced.len() > 128 {
            bail!("Code actions support at most 128 synchronized buffers");
        }
        if is_action {
            let mut paths = 0usize;
            for synced in self.synced.values() {
                let bytes = synced.path.as_os_str().len();
                paths = paths.saturating_add(bytes);
                if bytes > 8192 || paths > 512 * 1024 {
                    bail!("Code action workspace path budget exceeded");
                }
            }
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
        let (id, next_id) = self.request_identity()?;
        let mut params = json!({"textDocument":{"uri":uri}});
        if !is_action && method != "textDocument/documentSymbol" && !formatting {
            params["position"] = json!(position(doc, doc.cursor));
        }
        if let Some(extra) = extra.as_object() {
            params.as_object_mut().unwrap().extend(extra.clone());
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        self.next_id = next_id;
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
        if is_action {
            self.action_slot = Some(ActionSlot {
                token: id,
                canceled: false,
                timed_out: false,
                resolve_original: None,
            });
        }
        if formatting {
            self.formatting_slot = Some(FormattingSlot {
                token: id,
                canceled: false,
                timed_out: false,
            });
        }
        Ok(())
    }
    pub(crate) fn supports_document_formatting(&self) -> bool {
        match self.capabilities.get("documentFormattingProvider") {
            Some(Value::Bool(true)) => true,
            Some(Value::Object(options)) => options
                .get("workDoneProgress")
                .is_none_or(Value::is_boolean),
            _ => false,
        }
    }
    pub(crate) fn formatting_available(&self) -> bool {
        self.ready
            && self.supports_document_formatting()
            && self.formatting_slot.is_none()
            && self.pending.len() < 32
    }
    pub(crate) fn formatting_channel_closed(&self) -> bool {
        self.formatting_slot
            .as_ref()
            .is_some_and(|slot| slot.timed_out)
    }
    pub(crate) fn formatting_request_current(&self, token: u64) -> bool {
        self.formatting_slot
            .as_ref()
            .is_some_and(|slot| slot.token == token)
            && self
                .pending
                .get(&token)
                .is_some_and(|request| self.request_current(request))
    }
    fn ensure_formatting_available(&self) -> Result<()> {
        if !self.supports_document_formatting() {
            bail!("Language server does not provide native document formatting");
        }
        if self.formatting_channel_closed() {
            bail!(
                "Formatting request timed out; awaiting actual response or language server restart"
            );
        }
        if !self.formatting_available() {
            bail!("A formatting request is still running or awaiting actual response");
        }
        Ok(())
    }
    pub(crate) fn request_document_formatting(&mut self, doc: &Document) -> Result<u64> {
        let token = self.next_id;
        self.request(
            "textDocument/formatting",
            doc,
            json!({"options":{"tabSize":doc.tab_size,"insertSpaces":doc.insert_spaces}}),
        )?;
        Ok(token)
    }
    pub(crate) fn cancel_formatting_request(&mut self, token: u64) -> Result<()> {
        if let Some(slot) = &mut self.formatting_slot
            && slot.token == token
            && !slot.canceled
        {
            slot.canceled = true;
            self.notify("$/cancelRequest", json!({"id":token}))?;
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
        let (id, next_id) = self.request_identity()?;
        self.send(
            json!({"jsonrpc":"2.0","id":id,"method":"workspace/symbol","params":{"query":query}}),
        )?;
        self.next_id = next_id;
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
        self.ready && self.action_slot.is_none() && self.pending.len() < 32
    }
    pub(crate) fn action_channel_closed(&self) -> bool {
        self.action_slot.as_ref().is_some_and(|slot| slot.timed_out)
    }
    #[cfg(test)]
    pub(crate) fn action_request_current(&self, token: u64) -> bool {
        self.action_slot
            .as_ref()
            .is_some_and(|slot| slot.token == token && !slot.canceled && !slot.timed_out)
            && self
                .pending
                .get(&token)
                .is_some_and(|request| self.request_current(request))
    }
    fn ensure_action_available(&self) -> Result<()> {
        if self.action_channel_closed() {
            bail!("A code action timed out; awaiting actual response or language server restart");
        }
        if !self.action_available() {
            bail!("A code action request is already running; retry shortly");
        }
        Ok(())
    }
    pub(crate) fn cancel_action_request(&mut self, token: u64) -> Result<()> {
        if let Some(slot) = &mut self.action_slot
            && slot.token == token
            && !slot.canceled
        {
            slot.canceled = true;
            self.notify("$/cancelRequest", json!({"id":token}))?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn cancel_code_actions(&mut self) -> Result<()> {
        if let Some(token) = self.action_slot.as_ref().map(|slot| slot.token) {
            self.cancel_action_request(token)?;
        }
        Ok(())
    }
    pub fn ensure_command_available(&self) -> Result<()> {
        self.ensure_action_available()
    }
    pub(crate) fn resolve_code_action(&mut self, original: &Request, item: Value) -> Result<u64> {
        if self.capabilities["codeActionProvider"]["resolveProvider"] != true {
            bail!("Language server does not support code action resolution");
        }
        action_budget(&item)?;
        validate_action_item(&item)?;
        self.action_follow_up(original, "codeAction/resolve", item)
    }
    pub(crate) fn execute_action_command(
        &mut self,
        original: &Request,
        params: Value,
    ) -> Result<u64> {
        action_budget(&params)?;
        let object = params
            .as_object()
            .context("Invalid action command parameters")?;
        let command = object
            .get("command")
            .and_then(Value::as_str)
            .context("Invalid action command")?;
        if command.len() > 8192 || object.get("arguments").is_some_and(|args| !args.is_array()) {
            bail!("Invalid or oversized action command");
        }
        self.action_follow_up(original, "workspace/executeCommand", params)
    }
    fn action_follow_up(&mut self, original: &Request, method: &str, params: Value) -> Result<u64> {
        self.ensure_action_available()?;
        if !matches!(
            original.method.as_str(),
            "textDocument/codeAction" | "codeAction/resolve"
        ) || !self.request_current(original)
        {
            bail!("Original code action source or document changed");
        }
        let (id, next_id) = self.request_identity()?;
        let retained = (method == "codeAction/resolve").then(|| params.clone());
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        self.next_id = next_id;
        let mut request = original.clone();
        request.token = id;
        request.method = method.into();
        request.started = Instant::now();
        self.pending.insert(id, request);
        self.action_slot = Some(ActionSlot {
            token: id,
            canceled: false,
            timed_out: false,
            resolve_original: retained,
        });
        Ok(id)
    }
    pub fn follow_up(&mut self, original: &Request, method: &str, params: Value) -> Result<()> {
        if method == "textDocument/codeAction" {
            bail!("Code action discovery requires a synchronized document request");
        }
        if method == "textDocument/formatting" {
            bail!("Formatting requires a synchronized document formatting request");
        }
        if self.pending.len() >= 32 {
            bail!("Too many pending language requests");
        }
        if method == "workspace/executeCommand" {
            self.execute_action_command(original, params)?;
            return Ok(());
        }
        if method == "codeAction/resolve" {
            self.resolve_code_action(original, params)?;
            return Ok(());
        }
        let (id, next_id) = self.request_identity()?;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        self.next_id = next_id;
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
                        .formatting_slot
                        .as_ref()
                        .is_some_and(|slot| slot.token == **id && slot.timed_out)
                    && !self
                        .symbol_slot
                        .as_ref()
                        .is_some_and(|slot| slot.token == **id && slot.timed_out)
                    && !self
                        .action_slot
                        .as_ref()
                        .is_some_and(|slot| slot.token == **id && slot.timed_out)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
        {
            if let Some(slot) = &mut self.formatting_slot
                && slot.token == id
            {
                slot.timed_out = true;
                let notify = !slot.canceled;
                slot.canceled = true;
                let request = self.pending.get(&id).unwrap().clone();
                events.push(Event::FormattingResult(request, Err(
                    "Formatting request timed out; awaiting actual response or language server restart".into()
                )));
                if notify {
                    self.notify("$/cancelRequest", json!({"id":id}))?;
                }
                continue;
            }
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
            if let Some(slot) = &mut self.action_slot
                && slot.token == id
            {
                if slot.timed_out {
                    continue;
                }
                slot.timed_out = true;
                let notify = !slot.canceled;
                let deliver = !slot.canceled;
                slot.canceled = true;
                if deliver {
                    events.push(Event::ActionResult(self.pending.get(&id).unwrap().clone(), Err(
                        "Code action request timed out; awaiting actual response or language server restart".into()
                    )));
                }
                if notify {
                    self.notify("$/cancelRequest", json!({"id":id}))?;
                }
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
            if self.completion_resolve == Some(id) {
                self.completion_resolve = None;
                // A timeout cannot prove that the canceled callback has finished.
                self.completion_resolve_valid = false;
            }
            self.notify("$/cancelRequest", json!({"id":id}))?;
            events.push(Event::Message(format!(
                "Language request timed out: {}",
                request.method
            )));
        }
        for _ in 0..32 {
            let Some(mut message) = self.transport.receive()? else {
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
                                r.method == "workspace/executeCommand"
                                    && self.action_slot.as_ref().is_some_and(|slot| {
                                        slot.token == r.token && !slot.canceled && !slot.timed_out
                                    })
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
                    .action_slot
                    .as_ref()
                    .is_some_and(|slot| slot.token == id)
                {
                    let result = message.get("result");
                    let error = message.get("error");
                    let completed = message["jsonrpc"] == "2.0"
                        && (matches!((result, error), (Some(_), None))
                            || matches!((result,error),(None,Some(error)) if error["code"].as_i64().is_some_and(|code| i32::try_from(code).is_ok()) && error["message"].as_str().is_some()));
                    if !completed {
                        continue;
                    }
                    let slot = self.action_slot.take().unwrap();
                    let request = self.pending.remove(&id).unwrap();
                    if !slot.canceled && !slot.timed_out {
                        let response = if let Some(error) = error {
                            let text = error["message"].as_str().unwrap();
                            let mut end = text.len().min(4000);
                            while !text.is_char_boundary(end) {
                                end -= 1;
                            }
                            Err(format!("{} ({})", &text[..end], error["code"]))
                        } else {
                            let value = message.as_object_mut().unwrap().remove("result").unwrap();
                            validate_action_result(
                                &request.method,
                                &value,
                                slot.resolve_original.as_ref(),
                            )
                            .map(|()| value)
                            .map_err(|error| format!("{error:#}"))
                        };
                        events.push(Event::ActionResult(request, response));
                    }
                    continue;
                }
                if self
                    .formatting_slot
                    .as_ref()
                    .is_some_and(|slot| slot.token == id)
                {
                    let result = message.get("result");
                    let error = message.get("error");
                    let completed = message["jsonrpc"] == "2.0"
                        && (matches!((result, error), (Some(_), None))
                            || matches!((result,error),(None,Some(error)) if error["code"].as_i64().is_some_and(|code| i32::try_from(code).is_ok()) && error["message"].as_str().is_some()));
                    if !completed {
                        continue;
                    }
                    let slot = self.formatting_slot.take().unwrap();
                    let request = self.pending.remove(&id).unwrap();
                    if !slot.canceled && !slot.timed_out {
                        let response = if let Some(error) = error {
                            let text = error["message"].as_str().unwrap();
                            let mut end = text.len().min(4096);
                            while !text.is_char_boundary(end) {
                                end -= 1;
                            }
                            Err(format!("{} ({})", &text[..end], error["code"]))
                        } else {
                            Ok(result.unwrap().clone())
                        };
                        events.push(Event::FormattingResult(request, response));
                    }
                    continue;
                }
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
pub(crate) mod formatting_tests {
    use super::*;
    const PEER: &str = r#"
import json,sys
held={}; calls=[]; cancellations=[]
def send(message):
    data=json.dumps(message).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(data)).encode()+data)
    sys.stdout.buffer.flush()
def reply(message): send({'jsonrpc':'2.0',**message})
def notice(text): reply({'method':'window/showMessage','params':{'type':3,'message':text}})
while True:
    headers={}
    while True:
        line=sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line==b'\r\n': break
        key,value=line.decode().split(':',1); headers[key.lower()]=value.strip()
    message=json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method,ident,params=message.get('method'),message.get('id'),message.get('params',{})
    if method=='initialize':
        reply({'id':ident,'result':{'capabilities':{'textDocumentSync':1,'documentFormattingProvider':json.loads(sys.argv[1])}}})
    elif method=='textDocument/formatting':
        held[ident]=params; calls.append({'id':ident,'params':params}); notice('held-%d'%ident)
    elif method=='$/cancelRequest': cancellations.append(params['id']); notice('ignored-cancel-%d'%params['id'])
    elif method=='fixture/message': send(params)
    elif method=='fixture/ack': notice('ack')
    elif method=='fixture/state': notice('formatter-state:'+json.dumps({'calls':calls,'cancellations':cancellations}))
    elif method=='fixture/release':
        held.pop(params['id'],None)
        if params.get('fail'): reply({'id':params['id'],'error':{'code':-32603,'message':params.get('message','fixture formatting failed')}})
        else: reply({'id':params['id'],'result':params.get('result',[])})
    elif method=='textDocument/hover': reply({'id':ident,'result':None})
    elif method=='workspace/symbol': reply({'id':ident,'result':[]})
    elif method=='shutdown': reply({'id':ident,'result':None})
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
    fn start_with_capability(root: &Path, doc: &Document, capability: Value) -> Client {
        let mut client = Client::start(
            if cfg!(windows) { "python" } else { "python3" },
            &[
                "-u".into(),
                "-c".into(),
                PEER.into(),
                capability.to_string(),
            ],
            root,
            "cpp".into(),
        )
        .unwrap();
        until(&mut client, |client, _| client.ready);
        client.sync(std::slice::from_ref(doc)).unwrap();
        client
    }
    pub(crate) fn start(root: &Path, doc: &Document) -> Client {
        start_with_capability(root, doc, Value::Bool(true))
    }
    pub(crate) fn held(client: &mut Client, token: u64) {
        until(client, |_, events| {
            events.iter().any(|event| {
            matches!(event,Event::Message(message) if message == &format!("held-{token}"))
        })
        });
    }
    fn state(client: &mut Client) -> Value {
        client.notify("fixture/state", json!({})).unwrap();
        let events = until(client, |_, events| {
            events.iter().any(|event| {
            matches!(event,Event::Message(message) if message.starts_with("formatter-state:"))
        })
        });
        events
            .into_iter()
            .find_map(|event| match event {
                Event::Message(message) => message
                    .strip_prefix("formatter-state:")
                    .map(|text| serde_json::from_str(text).unwrap()),
                _ => None,
            })
            .unwrap()
    }
    fn inject(client: &mut Client, message: Value) -> Vec<Event> {
        client.notify("fixture/message", message).unwrap();
        client.notify("fixture/ack", json!({})).unwrap();
        let mut observed = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let events = client.poll().unwrap();
            let acknowledged = events
                .iter()
                .any(|event| matches!(event,Event::Message(message) if message == "ack"));
            observed.extend(events);
            if acknowledged {
                return observed;
            }
            assert!(Instant::now() < deadline, "{}", client.debug_summary());
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn fixture() -> (tempfile::TempDir, Document) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        std::fs::write(&path, "猫🙂 original\r\n").unwrap();
        let doc = Document::open(&path).unwrap();
        (root, doc)
    }

    #[test]
    fn shared_manual_save_formatter_lane_has_exact_params_and_typed_results() {
        let (root, mut doc) = fixture();
        doc.set_indentation(8, false);
        let mut client = start(root.path(), &doc);
        let token = client.next_id;
        client
            .request_in_view(
                "textDocument/formatting",
                &doc,
                json!({"options":{"tabSize":8,"insertSpaces":false}}),
                Some(17),
            )
            .unwrap();
        held(&mut client, token);
        assert!(client.request_document_formatting(&doc).is_err());
        let snapshot = state(&mut client);
        assert_eq!(snapshot["calls"].as_array().unwrap().len(), 1);
        assert_eq!(
            snapshot["calls"][0]["params"],
            json!({
                "textDocument":{"uri":file_uri(doc.path.as_ref().unwrap()).unwrap()},
                "options":{"tabSize":8,"insertSpaces":false}
            })
        );
        let replacement = json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"犬"}]);
        client
            .notify("fixture/release", json!({"id":token,"result":replacement}))
            .unwrap();
        let events = until(&mut client, |client, _| client.formatting_available());
        assert!(events.iter().any(|event| matches!(event,Event::FormattingResult(request,Ok(value)) if request.token == token && request.view == Some(17) && value == &replacement && client.request_current(request))));
        let save = client.request_document_formatting(&doc).unwrap();
        held(&mut client, save);
        assert!(
            client
                .request(
                    "textDocument/formatting",
                    &doc,
                    json!({"options":{"tabSize":4,"insertSpaces":true}})
                )
                .is_err()
        );
        client
            .notify("fixture/release", json!({"id":save,"fail":true}))
            .unwrap();
        let events = until(&mut client, |client, _| client.formatting_available());
        assert!(events.iter().any(|event| matches!(event,Event::FormattingResult(request,Err(error)) if request.token == save && request.view.is_none() && error.contains("fixture formatting failed"))));
        assert_eq!(
            std::fs::read(doc.path.as_ref().unwrap()).unwrap(),
            "猫🙂 original\r\n".as_bytes()
        );
        assert_eq!(doc.save_generation(), 0);
    }

    #[test]
    fn formatter_cancel_retains_actual_capacity_is_idempotent_and_never_publishes_late_edits() {
        let (root, doc) = fixture();
        let mut client = start(root.path(), &doc);
        let token = client.request_document_formatting(&doc).unwrap();
        held(&mut client, token);
        for _ in 0..64 {
            client.cancel_formatting_request(token).unwrap();
            client.cancel_formatting_request(token + 999).unwrap();
            assert!(!client.formatting_available());
            assert!(client.request_document_formatting(&doc).is_err());
        }
        let wire = state(&mut client);
        assert_eq!(wire["cancellations"], json!([token]));
        assert_eq!(wire["calls"].as_array().unwrap().len(), 1);
        assert_eq!(client.pending.len(), 1);
        assert!(!client.formatting_channel_closed());
        client
            .notify(
                "fixture/release",
                json!({"id":token,"result":[{"newText":"STALE"}]}),
            )
            .unwrap();
        let events = until(&mut client, |client, _| client.formatting_available());
        assert!(!events.iter().any(
            |event| matches!(event,Event::FormattingResult(request,_) if request.token == token)
        ));
        assert!(client.pending.is_empty());
        assert_eq!(doc.text.to_string(), "猫🙂 original\r\n");
    }

    #[test]
    fn formatter_timeout_retains_slot_and_malformed_or_wrong_replies_cannot_release_it() {
        let (root, doc) = fixture();
        let mut client = start(root.path(), &doc);
        let token = client.request_document_formatting(&doc).unwrap();
        held(&mut client, token);
        let malformed = [
            json!({"id":token,"result":null}),
            json!({"jsonrpc":"1.0","id":token,"result":null}),
            json!({"jsonrpc":"2.0","id":token}),
            json!({"jsonrpc":"2.0","id":token,"result":null,"error":{"code":-1,"message":"both"}}),
            json!({"jsonrpc":"2.0","id":token,"error":{"code":2147483648_i64,"message":"oversized"}}),
            json!({"jsonrpc":"2.0","id":token,"error":{"code":-1,"message":12}}),
            json!({"jsonrpc":"2.0","id":token+999,"result":[]}),
            json!({"jsonrpc":"2.0","id":token,"method":"window/showMessage","params":{"message":"method is not settlement"},"result":[]}),
        ];
        for message in malformed {
            assert!(
                !inject(&mut client, message)
                    .iter()
                    .any(|event| matches!(event, Event::FormattingResult(_, _)))
            );
            assert_eq!(client.formatting_slot.as_ref().unwrap().token, token);
            assert!(client.pending.contains_key(&token));
            assert!(!client.formatting_available());
        }
        client.pending.get_mut(&token).unwrap().started = Instant::now() - Duration::from_secs(16);
        let events = client.poll().unwrap();
        assert_eq!(events.iter().filter(|event| matches!(event,Event::FormattingResult(request,Err(error)) if request.token==token && error.contains("timed out"))).count(), 1);
        assert!(client.formatting_channel_closed());
        for _ in 0..64 {
            assert!(
                !client
                    .poll()
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event, Event::FormattingResult(_, _)))
            );
            assert!(client.request_document_formatting(&doc).is_err());
        }
        assert_eq!(state(&mut client)["cancellations"], json!([token]));
        let events = inject(
            &mut client,
            json!({"jsonrpc":"2.0","id":token,"error":{"code":-32800,"message":"actually finished"}}),
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::FormattingResult(_, _)))
        );
        assert!(client.formatting_available());
        assert!(!client.formatting_channel_closed());
        assert!(client.pending.is_empty());
    }

    #[test]
    fn native_formatting_ticket_retains_epoch_server_and_sync_lifetime_provenance() {
        let (root, mut doc) = fixture();
        let mut client = start(root.path(), &doc);
        let token = client.request_document_formatting(&doc).unwrap();
        held(&mut client, token);
        let request = client.pending[&token].clone();
        assert!(client.request_current(&request));
        assert!(client.formatting_request_current(token));
        assert!(!client.formatting_request_current(token + 1));
        let next_id = client.next_id;
        assert!(
            client
                .follow_up(&request, "textDocument/formatting", json!({}))
                .is_err()
        );
        assert_eq!(client.next_id, next_id);
        assert_eq!(client.pending.len(), 1);
        doc.insert("dirty ", false);
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂 original\r\n");
        assert_ne!(doc.text_epoch(), request.text_epoch);
        client.sync(std::slice::from_ref(&doc)).unwrap();
        assert!(!client.request_current(&request));
        assert!(!client.formatting_request_current(token));
        client
            .notify("fixture/release", json!({"id":token}))
            .unwrap();
        let events = until(&mut client, |client, _| client.formatting_available());
        assert!(events.iter().any(|event| matches!(event,Event::FormattingResult(reply,Ok(_)) if reply.token==token && !client.request_current(reply))));
        let fresh = client.request_document_formatting(&doc).unwrap();
        held(&mut client, fresh);
        let request = client.pending[&fresh].clone();
        client.sync(&[]).unwrap();
        client.sync(std::slice::from_ref(&doc)).unwrap();
        assert!(!client.request_current(&request));
        assert!(!client.formatting_request_current(fresh));
        let replacement = start(root.path(), &doc);
        assert!(!replacement.request_current(&request));
        client.cancel_formatting_request(fresh).unwrap();
        client
            .notify("fixture/release", json!({"id":fresh}))
            .unwrap();
        until(&mut client, |client, _| client.formatting_available());
        doc.redo();
        assert_eq!(doc.text.to_string(), "dirty 猫🙂 original\r\n");
        assert_eq!(
            std::fs::read(doc.path.as_ref().unwrap()).unwrap(),
            "猫🙂 original\r\n".as_bytes()
        );
    }

    #[test]
    fn static_formatting_capability_and_option_admission_fail_without_wire_or_identity_mutation() {
        let (root, mut doc) = fixture();
        let mut client = start(root.path(), &doc);
        for capability in [
            Value::Null,
            Value::Bool(false),
            json!("true"),
            json!({"workDoneProgress":12}),
        ] {
            client.capabilities["documentFormattingProvider"] = capability;
            let before = client.next_id;
            assert!(!client.supports_document_formatting());
            assert!(client.request_document_formatting(&doc).is_err());
            assert_eq!(client.next_id, before);
            assert!(client.pending.is_empty());
            assert!(client.formatting_slot.is_none());
        }
        client.capabilities["documentFormattingProvider"] = json!({"workDoneProgress":false});
        assert!(client.supports_document_formatting());
        let before = client.next_id;
        for extra in [
            json!({}),
            json!({"options":{"tabSize":0,"insertSpaces":true}}),
            json!({"options":{"tabSize":4,"insertSpaces":"true"}}),
            json!({"options":{"tabSize":4,"insertSpaces":true,"oversized":"x".repeat(4097)}}),
            json!({"options":{"tabSize":4,"insertSpaces":true},"textDocument":{"uri":"file:///other"}}),
        ] {
            assert!(
                client
                    .request("textDocument/formatting", &doc, extra)
                    .is_err()
            );
            assert_eq!(client.next_id, before);
            assert!(client.pending.is_empty());
        }
        doc.insert("unsynced ", false);
        assert!(client.request_document_formatting(&doc).is_err());
        assert_eq!(client.next_id, before);
        assert!(state(&mut client)["calls"].as_array().unwrap().is_empty());
        client.sync(std::slice::from_ref(&doc)).unwrap();
        let token = client.request_document_formatting(&doc).unwrap();
        held(&mut client, token);
        client
            .notify("fixture/release", json!({"id":token,"result":null}))
            .unwrap();
        assert!(until(&mut client, |client, _| client.formatting_available()).iter()
            .any(|event| matches!(event,Event::FormattingResult(request,Ok(value)) if request.token==token && value.is_null())));
    }

    #[test]
    fn checked_native_request_id_exhaustion_preserves_all_constructor_state() {
        let (root, doc) = fixture();
        let mut client = start(root.path(), &doc);
        client
            .request("textDocument/hover", &doc, json!({}))
            .unwrap();
        let events = until(&mut client, |_, events| {
            events.iter().any(|event| matches!(event,Event::Response(request,_) if request.method=="textDocument/hover"))
        });
        let original = events
            .into_iter()
            .find_map(|event| match event {
                Event::Response(request, _) => Some(request),
                _ => None,
            })
            .unwrap();
        client.next_id = u64::MAX;
        assert!(
            client
                .request_document_formatting(&doc)
                .unwrap_err()
                .to_string()
                .contains("exhausted")
        );
        assert!(
            client
                .workspace_symbols("main")
                .unwrap_err()
                .to_string()
                .contains("exhausted")
        );
        assert!(
            client
                .follow_up(&original, "completionItem/resolve", json!({}))
                .unwrap_err()
                .to_string()
                .contains("exhausted")
        );
        assert_eq!(client.next_id, u64::MAX);
        assert!(client.pending.is_empty());
        assert!(client.formatting_slot.is_none());
        assert!(client.symbol_slot.is_none());
        assert!(state(&mut client)["calls"].as_array().unwrap().is_empty());
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
    fn ignored_action_cancellation_retains_slots_until_exact_late_release() {
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
                                Event::ActionResult(request, Ok(_)) if request.token == first => {
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
                    .follow_up(original, method, json!({"title":"Lazy action","data":{},"hold":true,"command":"fixture.hold"}))
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
                    assert!(!events.iter().any(|event| matches!(event, Event::ActionResult(request, _) if request.token == token)));
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
                                .follow_up(original, method, json!({"title":"Lazy action","data":{},"hold":true,"command":"fixture.hold"}))
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
            assert!(
                !client
                    .poll()
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event, Event::ActionResult(_, _)))
            );
            assert_eq!(client.pending.len(), 1);
            assert!(client.action_channel_closed());
            let before = client.next_id;
            for _ in 0..32 {
                let error = if method == "workspace/executeCommand" {
                    client
                        .follow_up(original.as_ref().unwrap(), method, json!({"title":"Lazy action","data":{},"hold":true,"command":"fixture.hold"}))
                        .unwrap_err()
                } else {
                    let error = client
                        .request_code_actions(&doc, json!({}), None)
                        .unwrap_err();
                    if let Some(original) = &original {
                        assert!(
                            client
                                .follow_up(original, method, json!({"title":"Lazy action","data":{},"hold":true,"command":"fixture.hold"}))
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
                    |event| matches!(event, Event::ActionResult(request, _) if request.token == token)
                ));
                if events.iter().any(|event| matches!(event, Event::Message(message) if message == &format!("released-{token}"))) { break; }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                client.action_available(),
                "Exact late reply releases actual capacity"
            );
            assert!(client.pending.is_empty());
            let fresh = client.request_code_actions(&doc, json!({}), None).unwrap();
            loop {
                if client.poll().unwrap().iter().any(|event| {
                    matches!(event,
                    Event::ActionResult(request, Ok(_)) if request.token == fresh)
                }) {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            // Unrelated language reads remain available throughout the action lane lifetime.
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
    fn timed_out_command_blocks_all_action_work_and_rejects_late_edit_authorization() {
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
                    Event::ActionResult(request, Ok(_)) => Some(request),
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
            |event| matches!(event, Event::ActionResult(_, Err(message)) if message.contains("timed out"))
        ));
        doc.insert("new", false);
        client.sync(std::slice::from_ref(&doc)).unwrap();
        let mut newer = original.clone();
        newer.revision = doc.revision;
        let error = client
            .follow_up(&newer, "workspace/executeCommand", command)
            .unwrap_err();
        assert!(error.to_string().contains("restart"));
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

#[cfg(test)]
#[path = "lsp/action_lane_tests.rs"]
mod action_lane_tests;
