use super::*;
use crate::extension_state::{Scope, Store};
use std::collections::VecDeque;

pub(crate) const SAVE_SERVICE_LIFETIME: Duration = Duration::from_secs(360);
pub(crate) const SERVICE_LIFETIME: Duration = Duration::from_secs(3);
pub(crate) const NATIVE_COMMANDS: &[&str] = &[
    "undo",
    "redo",
    "editor.action.selectAll",
    "cursorLeft",
    "cursorRight",
    "cursorUp",
    "cursorDown",
    "cursorHome",
    "cursorEnd",
    "cursorTop",
    "cursorBottom",
];
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Request {
    pub session: u64,
    pub owner: String,
    pub command: Option<u64>,
    pub command_owner: Option<String>,
    pub generation: Option<u64>,
    pub args: Value,
}
#[derive(Clone)]
pub(crate) enum Operation {
    Open(Open),
    Save(Save),
    WorkspaceApply(Arc<crate::app::workspace_transactions::Capture>),
    FindFiles(
        Arc<crate::workspace_file_search::Capture>,
        Arc<std::sync::atomic::AtomicBool>,
    ),
    UntitledOpen(Arc<super::untitled::Capture>),
    VirtualOpen(Arc<super::text_content::Capture>),
    VirtualChange(Arc<super::text_content::Capture>),
    EditorState(Arc<super::editor_state::Capture>),
    Show(Show),
    Command(Command),
    State(StatePatch),
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Save {
    pub document: u64,
    pub version: u64,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Open {
    pub path: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Show {
    pub document: u64,
    pub version: u64,
    pub selection: Option<lsp::Range>,
    #[serde(default)]
    pub view_column: Option<i32>,
    #[serde(default)]
    pub preserve_focus: bool,
    #[serde(default)]
    pub preview: Option<bool>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Command {
    pub id: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatePatch {
    pub scope: Scope,
    pub key: String,
    pub remove: bool,
    pub value: Value,
}
#[derive(Clone)]
pub(crate) struct NativeService {
    pub id: String,
    pub request: Request,
    pub operation: Operation,
    pub started: Instant,
    payload_bytes: usize,
    save_origin: Option<Mirror>,
    save_path_wait: Option<Instant>,
    pub(crate) caller_retired: Arc<std::sync::atomic::AtomicBool>,
}
impl NativeService {
    pub fn fresh(&self) -> Result<()> {
        let lifetime = if matches!(self.operation, Operation::Save(_)) {
            SAVE_SERVICE_LIFETIME
        } else if matches!(self.operation, Operation::WorkspaceApply(_)) {
            Duration::from_secs(5)
        } else if matches!(self.operation, Operation::FindFiles(..)) {
            Duration::from_secs(10)
        } else {
            SERVICE_LIFETIME
        };
        if self.started.elapsed() >= lifetime {
            bail!("Native extension service expired; retry");
        }
        Ok(())
    }
}
pub(crate) fn bounded_error(error: &anyhow::Error) -> String {
    let mut text = format!("{error:#}");
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}
fn save_wait_origin_matches(pending: &Pending, request: &Request) -> bool {
    pending.method == "initialize"
        || pending.method == "activate" && pending.owner.is_none()
        || pending.owner.as_deref() == request.command_owner.as_deref()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSearchCancel {
    session: u64,
    owner: String,
    request: String,
}
impl Client {
    pub(crate) fn set_file_search_context(
        &mut self,
        context: Option<crate::workspace_file_search::Context>,
    ) {
        self.file_search_context = context;
    }
    pub(crate) fn cancel_native_file_search(&mut self, params: Value) -> Result<()> {
        if serde_json::to_vec(&params)?.len() > 1024 {
            bail!("File discovery cancellation exceeds bounds");
        }
        let request: FileSearchCancel = serde_json::from_value(params)?;
        self.validate_native_origin(request.session, &request.owner, None, None)?;
        // Only this exact native-admitted request owns the cancellation flag.
        if let Some(service) = self.services.iter().find(|service| {
            service.id == request.request
                && service.request.session == request.session
                && service.request.owner == request.owner
        }) && let Operation::FindFiles(_, flag) = &service.operation
        {
            flag.store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(())
    }
    pub(crate) fn set_workspace_edit_context(
        &mut self,
        context: Option<crate::app::workspace_transactions::CaptureContext>,
    ) {
        self.workspace_edit_context = context;
    }
    #[cfg(test)]
    pub(crate) fn queue_service(&mut self, method: &str, id: Value, params: Value) -> Result<()> {
        self.queue_service_with_models(method, id, params, &[], &[])
    }
    pub(crate) fn queue_service_with_models(
        &mut self,
        method: &str,
        id: Value,
        params: Value,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<()> {
        let started = Instant::now();
        if self.services.len() >= 8 {
            bail!("Native extension service queue limit reached");
        }
        let payload_bytes = serde_json::to_vec(&params)?.len();
        let limit = if matches!(
            method,
            "nativeEditorSelections" | "nativeEditorDecorations" | "nativeEditorInsertSnippet"
        ) {
            1024 * 1024
        } else if matches!(
            method,
            "nativeWorkspaceApplyEdit"
                | "nativeUntitledDocumentOpen"
                | "nativeVirtualDocumentOpen"
                | "nativeVirtualDocumentChange"
        ) {
            2 * 1024 * 1024
        } else {
            72 * 1024
        };
        if payload_bytes > limit
            || self
                .services
                .iter()
                .map(|service| service.payload_bytes)
                .sum::<usize>()
                .saturating_add(payload_bytes)
                > 8 * 1024 * 1024
        {
            bail!("Native extension service exceeds method or queue byte budget");
        }
        let id = id
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .context("Invalid native service ID")?
            .to_owned();
        if self.services.iter().any(|request| request.id == id) {
            bail!("Duplicate native service ID");
        }
        let mut request: Request = serde_json::from_value(params)?;
        self.validate_native_origin(
            request.session,
            &request.owner,
            request.command,
            request.command_owner.as_deref(),
        )?;
        let caller_retired = self.owner_lifetimes.capture(&request.owner)?;
        let operation = match method {
            "nativeDocumentLanguage"
            | "nativeEditorSelections"
            | "nativeEditorOptions"
            | "nativeEditorReveal"
            | "nativeEditorDecorations"
            | "nativeEditorInsertSnippet" => {
                let capture =
                    self.capture_editor_operation(&request.args, method, documents, hidden)?;
                request.args = Value::Null;
                Operation::EditorState(capture)
            }
            "nativeUntitledDocumentOpen" => {
                let capture = self.capture_untitled(&request.args, documents, hidden)?;
                request.args = Value::Null;
                Operation::UntitledOpen(capture)
            }
            "nativeVirtualDocumentOpen" | "nativeVirtualDocumentChange" => {
                let change = method == "nativeVirtualDocumentChange";
                let capture =
                    self.capture_text_content(&request.args, change, documents, hidden)?;
                request.args = Value::Null;
                if change {
                    Operation::VirtualChange(capture)
                } else {
                    Operation::VirtualOpen(capture)
                }
            }
            "nativeFindFiles" => {
                let context = self
                    .file_search_context
                    .clone()
                    .context("Native file discovery context unavailable")?;
                let capture =
                    crate::workspace_file_search::Capture::new(context, &request.args, started)?;
                request.args = Value::Null;
                Operation::FindFiles(capture, Arc::new(std::sync::atomic::AtomicBool::new(false)))
            }
            "nativeWorkspaceApplyEdit" => {
                use crate::app::workspace_transactions::{Origin, capture_workspace_transaction};
                let context = self
                    .workspace_edit_context
                    .clone()
                    .context("Workspace edit context is unavailable")?;
                let entries = request.args["entries"]
                    .as_array()
                    .filter(|entries| entries.len() <= 128)
                    .context("Workspace target budget exceeded")?;
                let mut targets = Vec::with_capacity(entries.len());
                for entry in entries {
                    let document = entry["document"]
                        .as_u64()
                        .context("Invalid workspace target")?;
                    let mirror = self
                        .mirror
                        .mirrors
                        .get(&document)
                        .context("Workspace target is not mirrored")?;
                    let doc = documents
                        .iter()
                        .chain(hidden)
                        .find(|doc| doc.id == document)
                        .context("Workspace target is not retained")?;
                    if mirror.revision != doc.revision
                        || mirror.text_epoch != doc.text_epoch()
                        || mirror.language_epoch != doc.language_epoch()
                        || mirror.eol != doc.eol
                    {
                        bail!("Workspace target changed before native receipt");
                    }
                    targets.push(super::providers::MirrorSnapshot {
                        document,
                        uri: mirror.uri.clone(),
                        path: doc.path.clone(),
                        revision: mirror.revision,
                        text_epoch: mirror.text_epoch,
                        language_epoch: mirror.language_epoch,
                        version: mirror.version,
                    });
                }
                let origin =
                    Origin::new(request.session, &id, &request.owner, caller_retired.clone())?;
                let capture = capture_workspace_transaction(
                    documents,
                    hidden,
                    &targets,
                    origin,
                    &request.args,
                    context,
                    started,
                )?;
                // The immutable native capture owns the admitted text and proofs.
                request.args = Value::Null;
                Operation::WorkspaceApply(capture)
            }
            "nativeDocumentOpen" => {
                let open: Open = serde_json::from_value(request.args.clone())?;
                if open.path.len() > 16 * 1024 || open.path.is_empty() {
                    bail!("Invalid document path");
                }
                Operation::Open(open)
            }
            "nativeDocumentSave" => {
                let save: Save = serde_json::from_value(request.args.clone())?;
                if save.document == 0 || save.version == 0 {
                    bail!("Invalid save model/version");
                }
                Operation::Save(save)
            }
            "nativeDocumentShow" => Operation::Show(serde_json::from_value(request.args.clone())?),
            "nativeCommand" => {
                let command: Command = serde_json::from_value(request.args.clone())?;
                if !NATIVE_COMMANDS.contains(&command.id.as_str()) {
                    bail!(
                        "Native command delegation is not implemented: {}",
                        command.id
                    );
                }
                Operation::Command(command)
            }
            "nativeStateWrite" => Operation::State(serde_json::from_value(request.args.clone())?),
            _ => bail!("Unknown native service"),
        };
        if !matches!(
            operation,
            Operation::State(_)
                | Operation::Save(_)
                | Operation::WorkspaceApply(_)
                | Operation::FindFiles(..)
                | Operation::UntitledOpen(_)
                | Operation::VirtualOpen(_)
                | Operation::VirtualChange(_)
                | Operation::EditorState(_)
        ) && request.generation != Some(self.mirror.generation)
            && !(matches!(
                operation,
                Operation::Open(_) | Operation::Show(_) | Operation::Command(_)
            ) && self.mirror.show_generation_current(request.generation))
        {
            bail!("Native document state changed before service request; retry");
        }
        let save_origin = if let Operation::Save(save) = &operation {
            let mirror = self
                .mirror
                .mirrors
                .get(&save.document)
                .context("Save model is not mirrored")?;
            if mirror.version != save.version || mirror.uri.len() > 4096 || mirror.eol.len() > 2 {
                bail!("Save source mirror changed or exceeds metadata budget");
            }
            Some(mirror.clone())
        } else {
            None
        };
        self.services.push_back(NativeService {
            id,
            request,
            operation,
            started,
            payload_bytes,
            save_origin,
            save_path_wait: None,
            caller_retired,
        });
        Ok(())
    }
    // Exact service-owned human wait; caller owner and originating command
    // owner can differ. No unrelated pending command receives a deadline reset.
    pub(crate) fn set_document_save_path_wait(
        &mut self,
        service: &NativeService,
        waiting: bool,
    ) -> Result<()> {
        let current = self
            .services
            .front_mut()
            .filter(|current| {
                current.id == service.id
                    && current.request.session == service.request.session
                    && current.request.owner == service.request.owner
                    && matches!(current.operation, Operation::Save(_))
            })
            .context("Document save service no longer pending")?;
        if waiting {
            if current.save_path_wait.is_none() {
                current.save_path_wait = Some(Instant::now());
            }
        } else if current.save_path_wait.take().is_some()
            && let Some(command) = current.request.command
            && let Some(pending) = self.pending.get_mut(&command)
            && save_wait_origin_matches(pending, &current.request)
        {
            pending.started = Instant::now();
        }
        Ok(())
    }
    pub(super) fn document_save_command_waiting(&self, command: u64) -> bool {
        self.services.iter().any(|service| {
            matches!(service.operation, Operation::Save(_))
                && service.request.session == self.session
                && service.request.command == Some(command)
                && service
                    .save_path_wait
                    .is_some_and(|started| started.elapsed() < super::prompts::PROMPT_LIFETIME)
                && self
                    .pending
                    .get(&command)
                    .is_some_and(|pending| save_wait_origin_matches(pending, &service.request))
        })
    }
    pub(super) fn expire_document_save_path_waits(&mut self) {
        for service in &mut self.services {
            if service
                .save_path_wait
                .is_some_and(|started| started.elapsed() >= super::prompts::PROMPT_LIFETIME)
            {
                service.save_path_wait = None;
                if let Some(command) = service.request.command
                    && let Some(pending) = self.pending.get_mut(&command)
                    && save_wait_origin_matches(pending, &service.request)
                {
                    pending.started = Instant::now();
                }
            }
        }
    }
    pub(crate) fn queued_services(&self) -> impl Iterator<Item = &NativeService> {
        self.services.iter()
    }
    pub(crate) fn service(&self) -> Option<&NativeService> {
        self.services.front()
    }
    pub(crate) fn service_valid(&self, service: &NativeService) -> Result<()> {
        service.fresh()?;
        if service
            .caller_retired
            .load(std::sync::atomic::Ordering::Acquire)
        {
            bail!("Original native caller lifetime has retired");
        }
        if !matches!(
            service.operation,
            Operation::State(_)
                | Operation::Save(_)
                | Operation::WorkspaceApply(_)
                | Operation::FindFiles(..)
                | Operation::UntitledOpen(_)
                | Operation::VirtualOpen(_)
                | Operation::VirtualChange(_)
                | Operation::EditorState(_)
        ) && service.request.generation != Some(self.mirror.generation)
            && !(matches!(
                service.operation,
                Operation::Open(_) | Operation::Show(_) | Operation::Command(_)
            ) && self
                .mirror
                .show_generation_current(service.request.generation))
        {
            bail!("Native document state changed before service completion; retry");
        }
        if !self
            .services
            .front()
            .is_some_and(|current| current.id == service.id)
        {
            bail!("Native service is no longer pending");
        }
        if matches!(service.operation, Operation::Save(_)) {
            self.validate_native_origin(service.request.session, &service.request.owner, None, None)
        } else if matches!(
            service.operation,
            Operation::State(_)
                | Operation::WorkspaceApply(_)
                | Operation::FindFiles(..)
                | Operation::UntitledOpen(_)
                | Operation::VirtualOpen(_)
                | Operation::VirtualChange(_)
                | Operation::EditorState(_)
        ) {
            // Receipt already authorized the exact origin. An accepted durable
            // patch may outlive that command; never bind it to a later command.
            self.validate_selected_owner(service.request.session, &service.request.owner)
        } else {
            self.validate_native_origin(
                service.request.session,
                &service.request.owner,
                service.request.command,
                service.request.command_owner.as_deref(),
            )
        }
    }
    pub(crate) fn answer_service(
        &mut self,
        service: &NativeService,
        result: Result<Value>,
    ) -> Result<()> {
        let id = &service.id;
        if !self.services.front().is_some_and(|request| {
            request.id == *id
                && request.request.session == service.request.session
                && request.request.owner == service.request.owner
        }) {
            bail!("Native service response no longer pending");
        }
        if matches!(service.operation, Operation::Save(_)) {
            self.set_document_save_path_wait(service, false)?;
        }
        let message = match result {
            Ok(result) => json!({"id":id,"result":result}),
            Err(error) => json!({"id":id,"error":{"message":bounded_error(&error)}}),
        };
        self.process.send(message)?;
        self.services.pop_front();
        Ok(())
    }
    pub(crate) fn service_document_current(&self, document: &Document, version: u64) -> bool {
        self.mirror.mirrors.get(&document.id).is_some_and(|mirror| {
            mirror.version == version
                && mirror.revision == document.revision
                && mirror.text_epoch == document.text_epoch()
                && document_uri(document).ok().as_ref() == Some(&mirror.uri)
                && mirror.eol == document.eol
        })
    }
    pub(crate) fn service_save_document_current(
        &self,
        service: &NativeService,
        document: &Document,
    ) -> bool {
        let Operation::Save(save) = &service.operation else {
            return false;
        };
        service.save_origin.as_ref().is_some_and(|origin| {
            document.id == save.document
                && document.revision == origin.revision
                && document.text_epoch() == origin.text_epoch
                && document.save_generation() == origin.save_generation
                && document.eol == origin.eol
                && document_uri(document).ok().as_ref() == Some(&origin.uri)
                && self.service_document_current(document, save.version)
        })
    }
    pub(crate) fn state_store(&self) -> Option<Store> {
        self.state_store.clone()
    }
}
impl Prepared {
    pub fn with_storage_root(mut self, root: Option<PathBuf>) -> Self {
        self.storage_root = root;
        self
    }
}
pub(crate) fn initial_state(store: Option<&Store>, packages: &[Package]) -> Result<Value> {
    let mut values = serde_json::Map::new();
    for package in packages {
        let read = |scope| -> Result<_> {
            Ok(match store {
                Some(store) => store.read(&package.id, scope)?,
                None => serde_json::Map::new(),
            })
        };
        values.insert(
            package.id.clone(),
            json!({"global":read(Scope::Global)?,"workspace":read(Scope::Workspace)?}),
        );
    }
    Ok(Value::Object(values))
}
pub(super) fn empty_queue() -> VecDeque<NativeService> {
    VecDeque::new()
}

#[cfg(test)]
mod tests {

    #[test]
    fn native_open_and_commands_tolerate_display_only_but_reject_source_editor_and_origin_changes()
    {
        for (method, args) in [
            ("nativeDocumentOpen", json!({"path":"next.cpp"})),
            ("nativeCommand", json!({"id":"cursorRight"})),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let extension =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
            let mut doc = Document::from_text("猫🙂 original\r\n");
            let mut client = Client::start(
                "node",
                &extension,
                directory.path(),
                &[],
                0,
                &Settings::default(),
            )
            .unwrap();
            let owner = client.packages[0].id.clone();
            client.pending.insert(
                900,
                Pending {
                    owner: Some(owner.clone()),
                    method: "execute".into(),
                    started: Instant::now(),
                },
            );
            let stamp = json!({"activeEditor":"editor-1","editors":[{
                "id":"editor-1","document":doc.id,"viewColumn":1,
                "selections":[],"options":{"tabSize":4},"visibleRanges":[]}]});
            let (initial, _) = client
                .mirror
                .next_refs_with_editor_stamp(&[&doc], Some(&doc), Some(&stamp))
                .unwrap();
            let generation = initial.generation;
            let mut measured = stamp.clone();
            measured["editors"][0]["visibleRanges"] =
                json!([{"start":{"line":0,"character":0},"end":{"line":1,"character":0}}]);
            let (next, _) = initial
                .next_refs_with_editor_stamp(&[&doc], Some(&doc), Some(&measured))
                .unwrap();
            assert!(next.generation > generation);
            client.mirror = next;
            let request = json!({"session":client.session,"owner":owner,"command":900,"commandOwner":owner,"generation":generation,"args":args});
            client
                .queue_service(method, json!("original"), request.clone())
                .unwrap();
            let original = client.service().unwrap().clone();
            assert!(client.service_valid(&original).is_ok(), "{method}");
            for (field, value) in [
                ("command", json!(901)),
                ("owner", json!("other.owner")),
                ("commandOwner", json!("other.owner")),
                ("session", json!(client.session + 1)),
                ("generation", json!(client.mirror.generation + 1)),
                ("generation", json!(generation - 1)),
            ] {
                let mut invalid = request.clone();
                invalid[field] = value;
                assert!(
                    client
                        .queue_service(method, json!("invalid"), invalid)
                        .is_err(),
                    "{method} {field}"
                );
            }
            measured["editors"][0]["visibleRanges"] = json!([]);
            let (next, _) = client
                .mirror
                .next_refs_with_editor_stamp(&[&doc], Some(&doc), Some(&measured))
                .unwrap();
            client.mirror = next;
            assert!(
                client.service_valid(&original).is_ok(),
                "{method}: contiguous viewport ABA remains presentation-only"
            );
            let command = client.pending.remove(&900).unwrap();
            assert!(
                client.service_valid(&original).is_err(),
                "{method}: original command remains required"
            );
            client.pending.insert(900, command);
            let session = client.session;
            client.session += 1;
            assert!(client.service_valid(&original).is_err());
            client.session = session;
            let baseline = client.mirror.clone();
            for (field, value) in [
                ("id", json!("editor-replaced")),
                ("viewColumn", json!(2)),
                ("document", json!(doc.id + 1)),
                (
                    "selections",
                    json!([{"anchor":{"line":0,"character":0},"active":{"line":0,"character":1}}]),
                ),
            ] {
                let mut changed = measured.clone();
                changed["editors"][0][field] = value;
                let (next, _) = baseline
                    .next_refs_with_editor_stamp(&[&doc], Some(&doc), Some(&changed))
                    .unwrap();
                client.mirror = next;
                assert!(
                    client.service_valid(&original).is_err(),
                    "{method}: native editor {field} is outside the interval"
                );
                assert!(
                    client
                        .queue_service(method, json!("changed-editor"), request.clone())
                        .is_err()
                );
            }
            client.mirror = baseline;
            doc.insert("X", false);
            doc.undo();
            assert_eq!(doc.text.to_string(), "猫🙂 original\r\n");
            let (changed, _) = client
                .mirror
                .next_refs_with_editor_stamp(&[&doc], Some(&doc), Some(&measured))
                .unwrap();
            client.mirror = changed;
            assert!(
                client.service_valid(&original).is_err(),
                "{method}: source ABA retires completion"
            );
            assert!(
                client
                    .queue_service(method, json!("stale-source"), request)
                    .is_err()
            );
            assert_eq!(client.services.len(), 1);
            assert_eq!(client.service().unwrap().id, "original");
        }
    }

    #[test]
    fn file_search_cancellation_requires_exact_owner_session_request_and_operation() {
        use std::sync::atomic::Ordering;
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let document = Document::from_text("dirty 猫🙂\r\n");
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[document],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        let context = crate::workspace_file_search::Context::new(
            directory.path().into(),
            Arc::new(Vec::new()),
            0,
        )
        .unwrap();
        client.set_file_search_context(Some(context));
        let args = json!({"session":client.session,"owner":owner,"args":{
            "include":{"pattern":"**/*.rs","baseUri":null},
            "exclude":{"mode":"none","pattern":null},"maxResults":null}});
        client
            .queue_service("nativeFindFiles", json!("search-original"), args)
            .unwrap();
        let service = client.service().unwrap().clone();
        let Operation::FindFiles(_, flag) = &service.operation else {
            panic!("wrong operation");
        };
        for params in [
            json!({"session":client.session,"owner":owner,"request":"another-request"}),
            json!({"session":client.session+1,"owner":owner,"request":"search-original"}),
            json!({"session":client.session,"owner":"unknown.owner","request":"search-original"}),
            json!({"session":client.session,"owner":owner,"request":"search-original","extra":true}),
        ] {
            let _ = client.cancel_native_file_search(params);
            assert!(!flag.load(Ordering::Acquire));
        }
        client
            .cancel_native_file_search(json!({"session":client.session,"owner":owner,
            "request":"search-original"}))
            .unwrap();
        assert!(flag.load(Ordering::Acquire));
        client
            .answer_service(&service, Ok(json!({"uris":[],"cancelled":true})))
            .unwrap();
        client
            .queue_service(
                "nativeStateWrite",
                json!("state-original"),
                json!({
            "session":client.session,"owner":owner,"args":{
                "scope":"global","key":"test","remove":false,"value":true}}),
            )
            .unwrap();
        let state = client.service().unwrap().clone();
        client
            .cancel_native_file_search(json!({"session":client.session,"owner":owner,
            "request":"state-original"}))
            .unwrap();
        assert!(matches!(
            client.service().unwrap().operation,
            Operation::State(_)
        ));
        assert!(client.service_valid(&state).is_ok());
    }
    #[test]
    fn queued_native_save_keeps_original_caller_lifetime_across_reactivation() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let document = Document::from_text("original 猫🙂\r\n");
        let document_id = document.id;
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[document],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        let version = client.mirror.mirrors[&document_id].version;
        let args = json!({"session":client.session,"owner":owner,
            "args":{"document":document_id,"version":version}});
        client
            .queue_service("nativeDocumentSave", json!("original-save"), args.clone())
            .unwrap();
        let original = client.service().unwrap().clone();
        assert!(client.service_valid(&original).is_ok());
        client
            .register_activation(&json!([{"id":owner,"state":"failed"}]))
            .unwrap();
        assert!(client.service_valid(&original).is_err());
        client
            .register_activation(&json!([{"id":owner,"state":"active"}]))
            .unwrap();
        assert!(!client.owner_retired(&owner));
        assert!(
            client.service_valid(&original).is_err(),
            "an owner string cannot renew a queued caller"
        );
        client
            .answer_service(&original, Err(anyhow::anyhow!("original caller retired")))
            .unwrap();
        client
            .queue_service("nativeDocumentSave", json!("replacement-save"), args)
            .unwrap();
        let replacement = client.service().unwrap().clone();
        assert!(client.service_valid(&replacement).is_ok());
        assert!(!Arc::ptr_eq(
            &original.caller_retired,
            &replacement.caller_retired
        ));
        assert!(
            original
                .caller_retired
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(
            !replacement
                .caller_retired
                .load(std::sync::atomic::Ordering::Acquire)
        );
        drop(client);
        assert!(
            replacement
                .caller_retired
                .load(std::sync::atomic::Ordering::Acquire)
        );
    }

    #[test]
    fn exact_save_as_service_wait_pauses_and_resets_only_its_original_command() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let document = Document::from_text("猫🙂 unsaved\r\n");
        let document_id = document.id;
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[document],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        let old = Instant::now() - Duration::from_secs(45);
        client.pending.insert(
            900,
            Pending {
                owner: Some(owner.clone()),
                method: "execute".into(),
                started: old,
            },
        );
        client.pending.insert(
            901,
            Pending {
                owner: Some(owner.clone()),
                method: "execute".into(),
                started: old,
            },
        );
        let version = client.mirror.mirrors[&document_id].version;
        client
            .queue_service(
                "nativeDocumentSave",
                json!("save-wait"),
                json!({"session":client.session,"owner":owner,
            "command":900,"commandOwner":owner,"generation":client.mirror.generation,
            "args":{"document":document_id,"version":version}}),
            )
            .unwrap();
        let service = client.service().unwrap().clone();
        client.set_document_save_path_wait(&service, true).unwrap();
        assert!(client.document_save_command_waiting(900));
        assert!(!client.document_save_command_waiting(901));
        let original_wait = client.service().unwrap().save_path_wait;
        client.set_document_save_path_wait(&service, true).unwrap();
        assert_eq!(
            client.service().unwrap().save_path_wait,
            original_wait,
            "poll must not renew human wait"
        );
        client.services.front_mut().unwrap().save_path_wait =
            Some(Instant::now() - Duration::from_secs(301));
        assert!(!client.document_save_command_waiting(900));
        client.expire_document_save_path_waits();
        assert!(client.service().unwrap().save_path_wait.is_none());
        client.set_document_save_path_wait(&service, false).unwrap();
        assert!(client.pending[&900].started > old);
        assert_eq!(client.pending[&901].started, old);
        let resumed = client.pending[&900].started;
        client.set_document_save_path_wait(&service, false).unwrap();
        assert_eq!(
            client.pending[&900].started, resumed,
            "no waiting lease means no additional reset"
        );
        let mut stale = service;
        stale.id = "old-service".into();
        assert!(client.set_document_save_path_wait(&stale, true).is_err());
        assert_eq!(client.pending[&900].started, resumed);
    }
    #[test]
    fn save_human_wait_keeps_registered_caller_separate_from_origin() {
        let request: Request = serde_json::from_value(json!({"session":1,"owner":"callee.b",
            "command":9,"commandOwner":"origin.a","args":{"document":1,"version":1}}))
        .unwrap();
        let started = Instant::now();
        let original = Pending {
            owner: Some("origin.a".into()),
            method: "execute".into(),
            started,
        };
        assert!(save_wait_origin_matches(&original, &request));
        let callee = Pending {
            owner: Some("callee.b".into()),
            method: "execute".into(),
            started,
        };
        assert!(!save_wait_origin_matches(&callee, &request));
        let initialization = Pending {
            owner: None,
            method: "initialize".into(),
            started,
        };
        assert!(save_wait_origin_matches(&initialization, &request));
    }
    #[test]
    fn save_dto_rejects_unknown_and_invalid_version_types_without_open_show_coercion() {
        use super::Save;
        let parsed: Save =
            serde_json::from_value(serde_json::json!({"document":1,"version":2})).unwrap();
        assert_eq!(parsed.document, 1);
        assert_eq!(parsed.version, 2);
        for invalid in [
            serde_json::json!({"document":1,"version":2,"path":"other"}),
            serde_json::json!({"document":1,"version":2.5}),
            serde_json::json!({"document":1,"version":-1}),
            serde_json::json!({"document":1}),
        ] {
            assert!(serde_json::from_value::<Save>(invalid).is_err());
        }
    }
    use super::*;
    #[test]
    fn late_reply_cannot_consume_same_request_id_in_a_replacement_session() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        let mut request = json!({"session":client.session,"owner":owner,"args":{"scope":"global","key":"accepted","remove":false,"value":true}});
        client
            .queue_service("nativeStateWrite", json!("host-1"), request.clone())
            .unwrap();
        let old = client.service().unwrap().clone();
        client.services.clear();
        client.session += 1;
        request["session"] = client.session.into();
        client
            .queue_service("nativeStateWrite", json!("host-1"), request)
            .unwrap();
        assert!(
            client
                .answer_service(&old, Err(anyhow::anyhow!("old worker stopped")))
                .is_err()
        );
        assert_eq!(client.services.len(), 1);
        assert_eq!(client.service().unwrap().request.session, client.session);
        assert!(client.service_valid(client.service().unwrap()).is_ok());
    }
    #[test]
    fn accepted_state_patch_outlives_its_origin_without_rebinding_or_crossing_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        client.queue_service("nativeStateWrite",json!("unawaited"),json!({"session":client.session,"owner":owner,"command":1,"commandOwner":owner,"args":{"scope":"global","key":"accepted","remove":false,"value":true}})).unwrap();
        let accepted = client.service().unwrap().clone();
        client.pending.remove(&1); // originating initialize/command has returned
        assert!(client.service_valid(&accepted).is_ok());
        client.pending.insert(
            2,
            Pending {
                owner: Some("other.owner".into()),
                method: "execute".into(),
                started: Instant::now(),
            },
        );
        assert!(client.service_valid(&accepted).is_ok());
        client.session += 1;
        assert!(client.service_valid(&accepted).is_err());
        client.session -= 1;
        client.packages.clear();
        assert!(client.service_valid(&accepted).is_err());
    }
    #[test]
    fn service_queue_rejects_unknown_fields_wrong_origins_stale_generations_and_overflow() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &[],
            0,
            &Settings::default(),
        )
        .unwrap();
        let request = json!({"session":client.session,"owner":client.packages[0].id,"generation":1,"args":{"path":"one"}});
        let mut bad = request.clone();
        bad["args"]["unsupported"] = true.into();
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("bad"), bad)
                .is_err()
        );
        let mut bad = request.clone();
        bad["owner"] = "other.owner".into();
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("bad"), bad)
                .is_err()
        );
        let mut bad = request.clone();
        bad["generation"] = 0.into();
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("bad"), bad)
                .is_err()
        );
        let mut bad = request.clone();
        bad["args"] = json!({"id":"workbench.action.tasks.runTask"});
        assert!(
            client
                .queue_service("nativeCommand", json!("bad"), bad)
                .is_err()
        );
        for index in 0..8 {
            client
                .queue_service(
                    "nativeDocumentOpen",
                    json!(format!("request-{index}")),
                    request.clone(),
                )
                .unwrap();
        }
        assert!(
            client
                .queue_service("nativeDocumentOpen", json!("ninth"), request)
                .is_err()
        );
        let service = client.service().unwrap().clone();
        assert!(client.service_valid(&service).is_ok());
        let original_mirror = client.mirror.clone();
        let changed_document = Document::from_text("real source change 猫🙂\r\n");
        let (changed, _) = client
            .mirror
            .next_refs(&[&changed_document], Some(&changed_document))
            .unwrap();
        assert!(changed.generation > original_mirror.generation);
        assert!(!changed.show_generation_current(service.request.generation));
        client.mirror = changed;
        assert!(client.service_valid(&service).is_err());
        client.mirror = original_mirror;
        assert!(client.service_valid(&service).is_ok());
        let mut expired = service;
        expired.started = Instant::now() - SERVICE_LIFETIME;
        assert!(client.service_valid(&expired).is_err());
    }
}

#[cfg(test)]
mod save_wait_poll_tests;
