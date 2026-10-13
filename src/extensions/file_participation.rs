//! Native admission for full file-participant DTOs. No filesystem/model effects.
//! Replies and actual completion are independent; cancellation cannot free work.
use super::*;
use crate::files::Action;
use anyhow::ensure;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;
const MAX_REPLY_BYTES: usize = 3 * 1024 * 1024;

struct Source {
    id: u64,
    session: u64,
    started: Instant,
    deadline: Instant,
    retired: AtomicBool,
    dispatch_owners: OnceLock<Vec<(String, Arc<AtomicBool>)>>,
    receipt: OnceLock<Receipt>,
}
struct TargetProof {
    snapshot: providers::MirrorSnapshot,
    eol: String,
}
struct Receipt {
    targets: Vec<TargetProof>,
    owners: Vec<(String, Arc<AtomicBool>)>,
}
#[derive(Clone)]
pub struct FileParticipationTicket {
    source: Arc<Source>,
}
impl FileParticipationTicket {
    pub fn id(&self) -> u64 {
        self.source.id
    }
    pub fn deadline(&self) -> Instant {
        self.source.deadline
    }
}
struct PendingParticipation {
    ticket: FileParticipationTicket,
    frame: Value,
    sent: bool,
    released: bool,
    canceled: bool,
    cancel_queued: bool,
    consumed: bool,
    reply: Option<std::result::Result<Option<Value>, String>>,
}
#[derive(Default)]
pub(super) struct State {
    pub enabled: bool,
    pending: Option<PendingParticipation>,
}
impl State {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }
}
impl Client {
    /// Experimental native coordinator boundary. The default editor does not
    /// negotiate public will subscriptions until pre-operation apply is wired.
    pub fn file_participation_available(&self) -> bool {
        self.file_participation.enabled && self.ready && self.file_participation.pending.is_none()
    }
    pub fn start_file_participation(
        &mut self,
        action: &Action,
        documents: &[Document],
        hidden: &[Document],
        timeout: Duration,
    ) -> Result<FileParticipationTicket> {
        ensure!(
            self.file_participation_available(),
            "File participation unavailable or actually occupied"
        );
        ensure!(
            !self.mirror_suspended
                && !timeout.is_zero()
                && timeout <= Duration::from_millis(MAX_TIMEOUT_MS),
            "Invalid file participation timeout/mirror state"
        );
        let id = self
            .next_id
            .checked_add(1)
            .filter(|id| *id < (1u64 << 53))
            .context("File participation identities exhausted")?;
        let frame = file_operations::frame(action, self.session, id)?;
        self.mirrored_snapshotset(documents, hidden)?;
        let started = Instant::now();
        let ticket = FileParticipationTicket {
            source: Arc::new(Source {
                id,
                session: self.session,
                started,
                deadline: started
                    .checked_add(timeout)
                    .context("File participation deadline overflow")?,
                retired: AtomicBool::new(false),
                dispatch_owners: OnceLock::new(),
                receipt: OnceLock::new(),
            }),
        };
        self.next_id = id;
        self.file_participation.pending = Some(PendingParticipation {
            ticket: ticket.clone(),
            frame,
            sent: false,
            released: false,
            canceled: false,
            cancel_queued: false,
            consumed: false,
            reply: None,
        });
        Ok(ticket)
    }
    pub fn file_participation_current(
        &self,
        ticket: &FileParticipationTicket,
        documents: &[Document],
        hidden: &[Document],
    ) -> bool {
        self.file_participation_current_at(ticket, documents, hidden, Instant::now())
    }
    fn file_participation_current_at(
        &self,
        ticket: &FileParticipationTicket,
        documents: &[Document],
        hidden: &[Document],
        now: Instant,
    ) -> bool {
        let source = &ticket.source;
        if source.session != self.session || source.retired.load(Ordering::Acquire) {
            return false;
        }
        let Some(receipt) = source.receipt.get() else {
            return now < source.deadline;
        };
        receipt.owners.iter().all(|(owner, retired)| {
            !retired.load(Ordering::Acquire)
                && self
                    .activation_states
                    .get(owner)
                    .is_some_and(|state| matches!(state.as_str(), "active" | "activating"))
        }) && receipt.targets.iter().all(|proof| {
            let target = &proof.snapshot;
            let mut matching = documents
                .iter()
                .chain(hidden)
                .filter(|doc| doc.id == target.document);
            let Some(doc) = matching.next() else {
                return false;
            };
            let Some(mirror) = self.mirror.mirrors.get(&target.document) else {
                return false;
            };
            matching.next().is_none()
                && document_uri(doc).ok().as_ref() == Some(&target.uri)
                && mirror.uri == target.uri
                && mirror.version == target.version
                && doc.path == target.path
                && doc.revision == target.revision
                && doc.text_epoch() == target.text_epoch
                && doc.resource_epoch() == target.resource_epoch
                && doc.eol == proof.eol
        })
    }
    pub fn cancel_file_participation(&mut self, ticket: &FileParticipationTicket) -> bool {
        let Some(pending) = self
            .file_participation
            .pending
            .as_mut()
            .filter(|pending| Arc::ptr_eq(&pending.ticket.source, &ticket.source))
        else {
            return false;
        };
        if pending.canceled || pending.consumed {
            return false;
        }
        pending.canceled = true;
        pending.ticket.source.retired.store(true, Ordering::Release);
        pending.reply = Some(Err(
            "File participation canceled; actual callbacks remain occupied".into(),
        ));
        if pending.sent && !pending.released {
            pending.cancel_queued = true;
        } else if !pending.sent {
            pending.released = true;
        }
        true
    }
    pub fn take_file_participation_reply(
        &mut self,
        ticket: &FileParticipationTicket,
        documents: &[Document],
        hidden: &[Document],
    ) -> Option<std::result::Result<Option<Value>, String>> {
        let current = self.file_participation_current(ticket, documents, hidden);
        let pending = self
            .file_participation
            .pending
            .as_mut()
            .filter(|pending| Arc::ptr_eq(&pending.ticket.source, &ticket.source))?;
        let mut reply = pending.reply.take()?;
        pending.consumed = true;
        if reply.is_ok() && !current {
            reply = Err("File participation original proof changed before consumption".into());
        }
        self.release_file_participation_slot();
        Some(reply)
    }
    fn release_file_participation_slot(&mut self) {
        if self
            .file_participation
            .pending
            .as_ref()
            .is_some_and(|pending| pending.released && pending.consumed)
        {
            self.file_participation.pending = None;
        }
    }
    pub(super) fn poll_file_participation(
        &mut self,
        documents: &[Document],
        hidden: &[Document],
        now: Instant,
    ) -> Result<()> {
        let retire = self
            .file_participation
            .pending
            .as_ref()
            .filter(|pending| !pending.canceled && pending.reply.is_none() && !pending.consumed)
            .filter(|pending| {
                !self.file_participation_current_at(&pending.ticket, documents, hidden, now)
            })
            .map(|pending| pending.ticket.clone());
        if let Some(ticket) = retire {
            self.cancel_file_participation(&ticket);
            if let Some(pending) = &mut self.file_participation.pending {
                pending.reply = Some(Err(if now >= ticket.deadline() {
                    "File participation original deadline exceeded"
                } else {
                    "File participation original caller/model/configuration changed"
                }
                .into()));
            }
        }
        if let Some(pending) = &mut self.file_participation.pending
            && pending.cancel_queued
        {
            match self.process.send(json!({"method":"cancelFileParticipation","params":{"session":self.session,"request":pending.ticket.id()}})) {
                Ok(()) => pending.cancel_queued = false,
                Err(error) if error.downcast_ref::<crate::transport::OutputBackpressure>().is_some() => {},
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub(super) fn flush_file_participation(&mut self) -> Result<()> {
        let owners = self
            .packages
            .iter()
            .filter_map(|package| {
                self.owner_lifetimes
                    .capture(&package.id)
                    .ok()
                    .map(|lease| (package.id.clone(), lease))
            })
            .collect::<Vec<_>>();
        let Some(pending) = &mut self.file_participation.pending else {
            return Ok(());
        };
        if pending.sent || pending.canceled || self.pending.len() >= 64 {
            return Ok(());
        }
        let remaining = pending
            .ticket
            .deadline()
            .saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        let timeout = remaining
            .as_millis()
            .saturating_add(u128::from(
                !remaining.subsec_nanos().is_multiple_of(1_000_000),
            ))
            .min(u128::from(MAX_TIMEOUT_MS));
        let message = json!({"id":pending.ticket.id(),"method":"willFileOperation","params":{"session":self.session,"frame":pending.frame,"timeout":timeout}});
        match self.process.send(message) {
            Ok(()) => {
                pending
                    .ticket
                    .source
                    .dispatch_owners
                    .set(owners)
                    .map_err(|_| anyhow::anyhow!("File participation was dispatched twice"))?;
                pending.sent = true;
                self.pending.insert(
                    pending.ticket.id(),
                    super::Pending {
                        owner: None,
                        method: "willFileOperation".into(),
                        started: pending.ticket.source.started,
                    },
                );
            }
            Err(error)
                if error
                    .downcast_ref::<crate::transport::OutputBackpressure>()
                    .is_some() => {}
            Err(error) => return Err(error),
        }
        Ok(())
    }
    pub(super) fn file_participation_response(
        &mut self,
        id: u64,
        mut message: Value,
        documents: &[Document],
        hidden: &[Document],
    ) {
        let Some(ticket) = self
            .file_participation
            .pending
            .as_ref()
            .filter(|pending| {
                pending.ticket.id() == id
                    && pending.sent
                    && !pending.canceled
                    && !pending.consumed
                    && pending.reply.is_none()
            })
            .map(|pending| pending.ticket.clone())
        else {
            return;
        };
        let reply = if !message["error"].is_null() {
            Err(message["error"]["message"]
                .as_str()
                .unwrap_or("File participation failed")
                .chars()
                .take(4096)
                .collect())
        } else {
            validate_reply(message["result"].take())
                .and_then(|value| {
                    ensure!(
                        Instant::now() < ticket.deadline(),
                        "File participation original deadline exceeded before receipt"
                    );
                    let receipt = self.capture_file_participation_receipt(
                        &ticket,
                        value.as_ref(),
                        documents,
                        hidden,
                    )?;
                    ticket.source.receipt.set(receipt).map_err(|_| {
                        anyhow::anyhow!("File participation receipt already captured")
                    })?;
                    Ok(value)
                })
                .map_err(|error| format!("{error:#}"))
        };
        if let Some(pending) = &mut self.file_participation.pending {
            pending.reply = Some(reply);
        }
    }
    fn capture_file_participation_receipt(
        &self,
        ticket: &FileParticipationTicket,
        value: Option<&Value>,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<Receipt> {
        let mut receipt = Receipt {
            targets: Vec::new(),
            owners: Vec::new(),
        };
        let Some(value) = value else {
            return Ok(receipt);
        };
        let provenance = &value["source"];
        let dispatched = ticket
            .source
            .dispatch_owners
            .get()
            .context("File participation has no original dispatch owners")?;
        let mut owners = HashSet::new();
        for owner in provenance["owners"]
            .as_array()
            .context("Invalid file participant owners")?
        {
            let owner = owner.as_str().context("Invalid file participant owner")?;
            ensure!(owners.insert(owner), "Duplicate file participant owner");
            let (_, lease) = dispatched
                .iter()
                .find(|(original, _)| original == owner)
                .context("File participant contributor was not originally selected")?;
            ensure!(
                !lease.load(Ordering::Acquire)
                    && self
                        .activation_states
                        .get(owner)
                        .is_some_and(|state| matches!(state.as_str(), "active" | "activating")),
                "File participant original contributor retired"
            );
            receipt.owners.push((owner.into(), lease.clone()));
        }
        let models = provenance["models"]
            .as_array()
            .context("Invalid file participant models")?;
        let mut uris = HashSet::new();
        let mut ids = HashSet::new();
        for model in models {
            let uri = model["uri"]
                .as_str()
                .filter(|uri| uri.len() <= 4096)
                .context("Invalid file participant target URI")?;
            let id = model["document"]
                .as_u64()
                .filter(|id| *id > 0 && *id < (1u64 << 53))
                .context("Invalid file participant target identity")?;
            let version = model["version"]
                .as_u64()
                .filter(|version| *version > 0 && *version < (1u64 << 53))
                .context("Invalid file participant target version")?;
            ensure!(
                uris.insert(uri) && ids.insert(id),
                "Ambiguous file participant target provenance"
            );
            let mut matching = documents.iter().chain(hidden).filter(|doc| doc.id == id);
            let doc = matching
                .next()
                .context("File participant original target closed")?;
            ensure!(
                matching.next().is_none() && document_uri(doc)? == uri,
                "File participant original target identity changed"
            );
            let snapshots = self.mirrored_snapshotset(std::slice::from_ref(doc), &[])?;
            let snapshot = &snapshots[0];
            ensure!(
                snapshot.uri == uri && snapshot.version == version,
                "File participant original target version changed"
            );
            receipt.targets.push(TargetProof {
                snapshot: snapshot.clone(),
                eol: doc.eol.clone(),
            });
        }
        let mut required = HashSet::new();
        for edit in value["edit"]["edits"].as_array().expect("validated stream") {
            if edit.get("textEdit").is_some()
                && let Some(version) = edit.get("versionId")
            {
                let uri = edit["resource"]
                    .as_str()
                    .context("Invalid versioned file participant URI")?;
                let version = version
                    .as_u64()
                    .context("Invalid versioned file participant edit")?;
                ensure!(
                    receipt
                        .targets
                        .iter()
                        .any(|target| target.snapshot.uri == uri
                            && target.snapshot.version == version),
                    "File participant edit has no original private target proof"
                );
                required.insert(uri);
            }
        }
        ensure!(
            required.len() == receipt.targets.len(),
            "Unused file participant target proof"
        );
        Ok(receipt)
    }
    pub(super) fn file_participation_released(&mut self, params: &Value) -> Result<()> {
        if params["session"].as_u64() != Some(self.session) {
            return Ok(());
        }
        let Some(pending) = self.file_participation.pending.as_mut().filter(|pending| {
            Some(pending.ticket.id()) == params["request"].as_u64() && pending.sent
        }) else {
            return Ok(());
        };
        ensure!(
            params.as_object().is_some_and(|fields| fields.len() == 3
                && fields
                    .keys()
                    .all(|key| matches!(key.as_str(), "session" | "request" | "accepted")))
                && params["accepted"].is_boolean(),
            "Invalid actual file participation completion"
        );
        pending.released = true;
        self.release_file_participation_slot();
        Ok(())
    }
}
fn validate_reply(value: Value) -> Result<Option<Value>> {
    if value.is_null() {
        return Ok(None);
    }
    ensure!(
        serde_json::to_vec(&value)?.len() <= MAX_REPLY_BYTES,
        "File participation reply exceeds byte budget"
    );
    let fields = value
        .as_object()
        .context("Invalid file participation reply")?;
    ensure!(
        fields.len() == 3
            && fields.contains_key("edit")
            && fields.contains_key("extensionNames")
            && fields.contains_key("source"),
        "Invalid file participation reply fields"
    );
    let edit = value["edit"]
        .as_object()
        .context("Invalid file participation edit")?;
    ensure!(
        serde_json::to_vec(edit)?.len() <= 2 * 1024 * 1024,
        "File participation edit exceeds wire budget"
    );
    ensure!(
        edit.len() == 1
            && edit
                .get("edits")
                .and_then(Value::as_array)
                .is_some_and(|edits| edits.len() <= 4096),
        "Invalid file participation edit stream"
    );
    let names = value["extensionNames"]
        .as_array()
        .context("Invalid file participation names")?;
    ensure!(
        !names.is_empty()
            && names.len() <= 8
            && names
                .iter()
                .all(|name| name.as_str().is_some_and(|name| name.len() <= 4096)),
        "File participation names exceed selected cohort budget"
    );
    let source = value["source"]
        .as_object()
        .context("Invalid file participant provenance")?;
    ensure!(
        source.len() == 2
            && source
                .get("owners")
                .and_then(Value::as_array)
                .is_some_and(|owners| !owners.is_empty() && owners.len() <= 8)
            && source
                .get("models")
                .and_then(Value::as_array)
                .is_some_and(|models| models.len() <= 128),
        "Invalid file participant provenance budget"
    );
    ensure!(
        source["models"]
            .as_array()
            .expect("bounded models")
            .iter()
            .all(
                |model| model.as_object().is_some_and(|fields| fields.len() == 3
                    && fields
                        .keys()
                        .all(|key| matches!(key.as_str(), "uri" | "document" | "version")))
            ),
        "Invalid file participant model proof fields"
    );
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(enabled: bool) -> (tempfile::TempDir, Client, Vec<Document>, Settings, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let extension = root.join("extension");
        std::fs::create_dir(&extension).unwrap();
        std::fs::write(extension.join("package.json"),serde_json::to_vec(&json!({"publisher":"fixture","name":"participants","displayName":"Trusted participant","version":"1.0.0","main":"main.cjs","engines":{"vscode":"*"},"activationEvents":["*"],"contributes":{"commands":[{"command":"fixture.release","title":"Release participant"}]}})).unwrap()).unwrap();
        std::fs::write(extension.join("main.cjs"),r#"
const vscode=require('vscode'),fs=require('node:fs'),path=require('node:path');let release;
exports.activate=context=>{
 const root=vscode.workspace.workspaceFolders[0].uri.fsPath;
 // Public package metadata must not change the trusted contribution label.
 vscode.extensions.getExtension(context.extension.id).packageJSON.displayName='spoofed';
 const result=()=>{
  const edit=new vscode.WorkspaceEdit(),uri=vscode.workspace.textDocuments[0].uri;
  edit.replace(uri,new vscode.Range(0,0,0,0),'contributed 猫🙂\r\n');
  edit.createFile(vscode.Uri.file(path.join(root,'binary.dat')),{contents:new Uint8Array([0,1,255]),overwrite:true},{label:'binary',needsConfirmation:true});
  edit.renameFile(uri,vscode.Uri.file(path.join(root,'moved.cpp')),{overwrite:true});
  edit.set(vscode.Uri.file(path.join(root,'snippet.cpp')),[vscode.SnippetTextEdit.replace(new vscode.Range(0,0,0,0),new vscode.SnippetString('${1:猫}'))]);
  edit.set(vscode.Uri.parse('notebook:///tmp/a'),[vscode.NotebookEdit.updateNotebookMetadata({flag:true})]);
  return edit;
 };
 const listener=event=>{
  const target=event.files[0].newUri||event.files[0];
  fs.appendFileSync(path.join(root,'entered.log'),target.fsPath+'\n');
  event.waitUntil(target.fsPath.endsWith('held.cpp')?new Promise(resolve=>{release=resolve;}).then(result):Promise.resolve(result()));
 };
 context.subscriptions.push(vscode.workspace.onWillCreateFiles(listener),vscode.workspace.onWillRenameFiles(listener),vscode.workspace.onWillDeleteFiles(listener),vscode.commands.registerCommand('fixture.release',()=>release?.()));
};
"#).unwrap();
        let file = root.join("original 猫.cpp");
        std::fs::write(&file, "disk 猫🙂\r\n").unwrap();
        let mut doc = Document::open(&file).unwrap();
        doc.insert("dirty λ ", false);
        let mut documents = vec![doc];
        let settings = Settings::default();
        let mut prepared = Client::prepare(&documents, 0, &settings).unwrap();
        if enabled {
            prepared = prepared.with_file_participation();
        }
        // Disabled negotiation is tested below without activating a package that
        // intentionally requires the experimental will API during activation.
        let package = if enabled {
            extension
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension")
        };
        let mut client = Client::start_prepared("node", &package, &root, prepared).unwrap();
        until(&mut client, &mut documents, &settings, |client| {
            client.ready
        });
        (directory, client, documents, settings, root)
    }
    fn until(
        client: &mut Client,
        documents: &mut [Document],
        settings: &Settings,
        condition: impl Fn(&Client) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition(client) {
            client.poll(documents, 0, settings).unwrap();
            assert!(
                Instant::now() < deadline,
                "{}",
                client.process.stderr_tail()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn has_reply(client: &Client) -> bool {
        client
            .file_participation
            .pending
            .as_ref()
            .is_some_and(|pending| pending.reply.is_some())
    }
    #[test]
    fn real_selected_extension_returns_full_ordered_dto_without_native_model_or_disk_effects() {
        let (_directory, mut client, mut documents, settings, root) = fixture(true);
        let identity = documents[0].id;
        let original = documents[0].text.clone();
        let baseline = documents[0].disk_content.clone();
        let ticket = client
            .start_file_participation(
                &Action::CreateFile(root.join("created.cpp")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        until(&mut client, &mut documents, &settings, has_reply);
        let reply = client
            .take_file_participation_reply(&ticket, &documents, &[])
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply["extensionNames"], json!(["Trusted participant"]));
        let edits = reply["edit"]["edits"].as_array().unwrap();
        assert_eq!(edits.len(), 5);
        assert_eq!(edits[0]["textEdit"]["text"], "contributed 猫🙂\r\n");
        assert_eq!(edits[0]["versionId"], 1);
        assert_eq!(
            edits[1]["options"]["contents"],
            json!({"type":"base64","value":"AAH/"})
        );
        assert_eq!(edits[1]["metadata"]["needsConfirmation"], true);
        assert!(edits[2]["oldResource"].is_string());
        assert_eq!(edits[3]["textEdit"]["insertAsSnippet"], true);
        assert_eq!(edits[4]["cellEdit"]["editType"], 5);
        until(
            &mut client,
            &mut documents,
            &settings,
            Client::file_participation_available,
        );
        assert_eq!(documents[0].id, identity);
        assert_eq!(documents[0].text, original);
        assert_eq!(documents[0].disk_content, baseline);
        assert!(documents[0].dirty());
        assert_eq!(
            std::fs::read(documents[0].path.as_ref().unwrap()).unwrap(),
            "disk 猫🙂\r\n".as_bytes()
        );
        assert!(!root.join("created.cpp").exists());
        assert!(!root.join("binary.dat").exists());
        assert!(!root.join("moved.cpp").exists());
    }
    #[test]
    fn canceled_real_callback_keeps_capacity_through_late_result_and_exact_actual_release() {
        let (_directory, mut client, mut documents, settings, root) = fixture(true);
        let ticket = client
            .start_file_participation(
                &Action::CreateFile(root.join("held.cpp")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        until(&mut client, &mut documents, &settings, |_| {
            root.join("entered.log").exists()
        });
        client
            .poll_file_participation(&documents, &[], ticket.deadline())
            .unwrap();
        assert!(
            client
                .take_file_participation_reply(&ticket, &documents, &[])
                .unwrap()
                .unwrap_err()
                .contains("original deadline")
        );
        assert!(!client.file_participation_available());
        assert!(
            client
                .start_file_participation(
                    &Action::CreateFolder(root.join("blocked")),
                    &documents,
                    &[],
                    Duration::from_secs(5)
                )
                .is_err()
        );
        client
            .file_participation_released(
                &json!({"session":client.session+1,"request":ticket.id(),"accepted":true}),
            )
            .unwrap();
        client
            .file_participation_released(
                &json!({"session":client.session,"request":ticket.id()+100,"accepted":true}),
            )
            .unwrap();
        assert!(!client.file_participation_available());
        client
            .execute("fixture.release", None, &documents, 0, &settings)
            .unwrap();
        until(
            &mut client,
            &mut documents,
            &settings,
            Client::file_participation_available,
        );
        assert!(
            client
                .take_file_participation_reply(&ticket, &documents, &[])
                .is_none()
        );
        let next = client
            .start_file_participation(
                &Action::Trash(root.join("ordinary.cpp")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        assert!(!client.cancel_file_participation(&ticket));
        client
            .file_participation_released(
                &json!({"session":client.session,"request":ticket.id(),"accepted":true}),
            )
            .unwrap();
        assert!(!client.file_participation_available());
        until(&mut client, &mut documents, &settings, has_reply);
        assert!(
            client
                .take_file_participation_reply(&next, &documents, &[])
                .unwrap()
                .is_ok()
        );
        assert!(!root.join("held.cpp").exists());
    }
    #[test]
    fn staged_retirement_never_invokes_js_and_expired_admission_is_not_renewed() {
        let (_directory, mut client, documents, _settings, root) = fixture(true);
        let action = Action::Rename {
            from: root.join("a"),
            to: root.join("b"),
        };
        let first = client
            .start_file_participation(&action, &documents, &[], Duration::from_secs(5))
            .unwrap();
        assert!(client.cancel_file_participation(&first));
        assert!(
            client
                .take_file_participation_reply(&first, &documents, &[])
                .unwrap()
                .is_err()
        );
        assert!(client.file_participation_available());
        let second = client
            .start_file_participation(&action, &documents, &[], Duration::from_secs(5))
            .unwrap();
        client
            .poll_file_participation(&documents, &[], second.deadline())
            .unwrap();
        client.flush_file_participation().unwrap();
        assert!(
            client
                .take_file_participation_reply(&second, &documents, &[])
                .unwrap()
                .unwrap_err()
                .contains("deadline")
        );
        assert!(client.file_participation_available());
        assert!(!root.join("entered.log").exists());
        assert!(!client.pending.contains_key(&first.id()));
        assert!(!client.pending.contains_key(&second.id()));
    }
    #[test]
    fn returned_target_guards_history_and_owner_aba_but_allows_language_and_baseline_changes() {
        for change in ["history", "language", "owner", "baseline"] {
            let (_directory, mut client, mut documents, settings, root) = fixture(true);
            let ticket = client
                .start_file_participation(
                    &Action::CreateFile(root.join("created.cpp")),
                    &documents,
                    &[],
                    Duration::from_secs(5),
                )
                .unwrap();
            until(&mut client, &mut documents, &settings, has_reply);
            let text = documents[0].text.clone();
            match change {
                "history" => {
                    documents[0].insert("temporary ", false);
                    documents[0].undo();
                }
                "language" => {
                    documents[0]
                        .set_language_override(Some("cpp".into()))
                        .unwrap();
                    documents[0].set_language_override(None).unwrap();
                }
                "owner" => {
                    let mut states = client.activation_states.clone();
                    states.insert("fixture.participants".into(), "failed".into());
                    client.owner_lifetimes.publish(&states);
                    states.insert("fixture.participants".into(), "active".into());
                    client.owner_lifetimes.publish(&states);
                }
                "baseline" => documents[0].save().unwrap(),
                _ => unreachable!(),
            }
            assert_eq!(documents[0].text, text);
            let result = client
                .take_file_participation_reply(&ticket, &documents, &[])
                .unwrap();
            assert_eq!(
                result.is_ok(),
                matches!(change, "language" | "baseline"),
                "{change}"
            );
            assert!(!root.join("created.cpp").exists());
        }
    }
    #[test]
    fn legitimate_native_edits_during_callback_use_conversion_time_versions_and_unrelated_edits_do_not_retire_the_receipt()
     {
        let (_directory, mut client, mut documents, settings, root) = fixture(true);
        let ticket = client
            .start_file_participation(
                &Action::CreateFile(root.join("held.cpp")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        until(&mut client, &mut documents, &settings, |_| {
            root.join("entered.log").exists()
        });
        documents[0].insert("while waiting ", false);
        client.poll(&mut documents, 0, &settings).unwrap();
        assert!(!has_reply(&client));
        assert!(client.file_participation_current(&ticket, &documents, &[]));
        let converted_version = client.mirror.mirrors[&documents[0].id].version;
        client
            .execute("fixture.release", None, &documents, 0, &settings)
            .unwrap();
        until(&mut client, &mut documents, &settings, has_reply);
        let mut unrelated = vec![Document::from_text("unrelated")];
        unrelated[0].insert("more ", false);
        let reply = client
            .take_file_participation_reply(&ticket, &documents, &unrelated)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply["edit"]["edits"][0]["versionId"], converted_version);
        assert_eq!(reply["source"]["models"][0]["document"], documents[0].id);
        assert_eq!(reply["source"]["owners"], json!(["fixture.participants"]));
        assert!(!root.join("held.cpp").exists());
    }
    #[test]
    fn valid_reply_cannot_be_consumed_after_a_native_source_change() {
        let (_directory, mut client, mut documents, settings, root) = fixture(true);
        let ticket = client
            .start_file_participation(
                &Action::CreateFile(root.join("created.cpp")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        until(&mut client, &mut documents, &settings, has_reply);
        documents[0].insert("new user input ", false);
        let current = documents[0].text.clone();
        assert!(
            client
                .take_file_participation_reply(&ticket, &documents, &[])
                .unwrap()
                .unwrap_err()
                .contains("before consumption")
        );
        assert_eq!(documents[0].text, current);
        assert!(!root.join("created.cpp").exists());
    }
    #[test]
    fn timely_receipt_ends_callback_budget_but_never_adopts_a_replacement_native_target() {
        let (_directory, mut client, mut documents, settings, root) = fixture(true);
        let ticket = client
            .start_file_participation(
                &Action::CreateFile(root.join("created.cpp")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        until(&mut client, &mut documents, &settings, has_reply);
        assert!(client.file_participation_current_at(
            &ticket,
            &documents,
            &[],
            ticket.deadline() + Duration::from_secs(1)
        ));
        let replacement = Document::open(documents[0].path.as_ref().unwrap()).unwrap();
        assert_ne!(replacement.id, documents[0].id);
        documents[0] = replacement;
        assert!(
            client
                .take_file_participation_reply(&ticket, &documents, &[])
                .unwrap()
                .is_err()
        );
        assert_eq!(documents[0].text.to_string(), "disk 猫🙂\r\n");
        assert!(!root.join("created.cpp").exists());
    }
    #[test]
    fn actual_writer_pressure_preserves_original_identity_deadline_and_frame_until_admission() {
        let (_directory, mut client, mut documents, settings, root) = fixture(true);
        let (entered, release) = client.fixture_gate_native_writer().unwrap();
        client.fixture_native_output_barrier().unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!((0..256).any(|_| client.fixture_native_output_probe().is_err()));
        let ticket = client
            .start_file_participation(
                &Action::CreateFolder(root.join("created")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        let original = client
            .file_participation
            .pending
            .as_ref()
            .unwrap()
            .frame
            .clone();
        let deadline = ticket.deadline();
        client.flush_file_participation().unwrap();
        let pending = client.file_participation.pending.as_ref().unwrap();
        assert!(!pending.sent);
        assert_eq!(pending.frame, original);
        assert_eq!(pending.ticket.deadline(), deadline);
        assert!(!root.join("entered.log").exists());
        release.send(()).unwrap();
        until(&mut client, &mut documents, &settings, has_reply);
        assert!(
            client
                .take_file_participation_reply(&ticket, &documents, &[])
                .unwrap()
                .is_ok()
        );
        assert_eq!(ticket.deadline(), deadline);
    }
    #[test]
    fn disabled_invalid_foreign_and_malformed_admissions_do_not_release_or_manufacture_work() {
        let (_directory, mut disabled, documents, _settings, root) = fixture(false);
        let initial = disabled.next_id;
        assert!(
            disabled
                .start_file_participation(
                    &Action::CreateFile(root.join("a")),
                    &documents,
                    &[],
                    Duration::from_secs(5)
                )
                .is_err()
        );
        assert_eq!(disabled.next_id, initial);
        let (_directory, mut client, documents, _settings, root) = fixture(true);
        assert!(
            client
                .start_file_participation(
                    &Action::CreateFile("relative".into()),
                    &documents,
                    &[],
                    Duration::from_secs(5)
                )
                .is_err()
        );
        assert!(
            client
                .start_file_participation(
                    &Action::CreateFile(root.join("a")),
                    &documents,
                    &[],
                    Duration::ZERO
                )
                .is_err()
        );
        let ticket = client
            .start_file_participation(
                &Action::CreateFile(root.join("a")),
                &documents,
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        client.file_participation.pending.as_mut().unwrap().sent = true;
        assert!(
            client
                .file_participation_released(
                    &json!({"session":client.session,"request":ticket.id(),"accepted":"yes"})
                )
                .is_err()
        );
        assert!(!client.file_participation_available());
        client.file_participation_response(
            ticket.id(),
            json!({"result":{"edit":{"edits":[]},"extensionNames":[],"extra":true}}),
            &documents,
            &[],
        );
        assert!(
            client
                .take_file_participation_reply(&ticket, &documents, &[])
                .unwrap()
                .is_err()
        );
        assert!(!client.file_participation_available());
        client
            .file_participation_released(
                &json!({"session":client.session,"request":ticket.id(),"accepted":true}),
            )
            .unwrap();
        assert!(client.file_participation_available());
        let provenance = json!({"owners":["fixture.participants"],"models":[]});
        assert!(
            validate_reply(
                json!({"edit":{"edits":[]},"extensionNames":vec!["x";9],"source":provenance})
            )
            .unwrap_err()
            .to_string()
            .contains("names")
        );
        assert!(validate_reply(json!({"edit":{"edits":vec![json!({});4097]},"extensionNames":["x"],"source":provenance})).unwrap_err().to_string().contains("stream"));
        assert!(validate_reply(json!({"edit":{"edits":[{"metadata":{"label":"x".repeat(2*1024*1024)}}]},"extensionNames":["x"],"source":provenance})).unwrap_err().to_string().contains("wire budget"));
    }
}
