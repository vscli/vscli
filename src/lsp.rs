//! Native stdio LSP transport. Subprocess I/O never blocks the input/render loop.
use crate::document::{Document, Selection};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
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
    pub method: String,
    pub document_id: u64,
    pub revision: u64,
    pub cursor: usize,
    pub path: PathBuf,
    pub selections: Vec<Selection>,
    pub view: Option<u64>,
    pub workspace: Arc<HashMap<PathBuf, Snapshot>>,
    started: Instant,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: u64,
    pub revision: u64,
    pub version: i64,
}
pub enum Event {
    Ready,
    ApplyEdit(Value, Option<Request>, Value),
    Response(Request, Value),
    Diagnostics(PathBuf, Vec<Diagnostic>),
    Message(String),
}
struct Synced {
    id: u64,
    revision: u64,
    version: i64,
    path: PathBuf,
}
pub struct Client {
    transport: crate::transport::Process,
    pending: HashMap<u64, Request>,
    synced: HashMap<String, Synced>,
    next_id: u64,
    command_channel_valid: bool,
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
        let root_uri = file_uri(root)?;
        let transport = crate::transport::Process::start(program, args, root)?;
        let client = Self {
            transport,
            pending: HashMap::new(),
            synced: HashMap::new(),
            next_id: 1,
            command_channel_valid: true,
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
                    "completion":{"completionItem":{"snippetSupport":false,"documentationFormat":["plaintext","markdown"]}},
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
                            version,
                            path: path.clone(),
                        },
                    );
                }
                Some(old) if old.revision != doc.revision || old.id != doc.id => {
                    let version = self.allocate_document_version()?;
                    self.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":doc.text.to_string()}]}))?;
                    self.synced.insert(
                        uri,
                        Synced {
                            id: doc.id,
                            revision: doc.revision,
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
        if !self.synced.contains_key(&uri) {
            bail!("This language server does not handle this file type");
        }
        if self.pending.len() >= 32 {
            bail!("Too many pending language requests");
        }
        let is_action = method == "textDocument/codeAction";
        if is_action && !self.action_available() {
            bail!("A code action request is already running; retry shortly");
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
                method: method.into(),
                document_id: doc.id,
                revision: doc.revision,
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
                method: "workspace/symbol".into(),
                document_id: 0,
                revision: 0,
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
        !self.pending.values().any(|request| {
            matches!(
                request.method.as_str(),
                "textDocument/codeAction" | "codeAction/resolve" | "workspace/executeCommand"
            )
        })
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
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let mut request = original.clone();
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
    pub fn poll(&mut self) -> Result<Vec<Event>> {
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
                        && params["version"]
                            .as_i64()
                            .is_none_or(|v| v == synced.version)
                    {
                        let diagnostics: Vec<Diagnostic> =
                            serde_json::from_value(params["diagnostics"].clone())
                                .unwrap_or_default();
                        events.push(Event::Diagnostics(
                            synced.path.clone(),
                            diagnostics.into_iter().take(5000).collect(),
                        ));
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
                    if let Some(error) = message.get("error") {
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
