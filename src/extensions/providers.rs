//! Bounded native ownership and lifetime checks for optional language callbacks.
use super::*;
use crate::extension_providers::{Kind, Provider, Registry};
use std::collections::VecDeque;
const LIMIT: usize = 8;
const DEADLINE: Duration = Duration::from_secs(6);
#[derive(Clone, Debug)]
pub struct MirrorSnapshot {
    pub uri: String,
    pub document: u64,
    pub path: Option<PathBuf>,
    pub revision: u64,
    pub text_epoch: u64,
    pub version: u64,
}
#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: u64,
    pub session: u64,
    pub provider: Provider,
    pub epoch: u64,
    pub document: u64,
    pub revision: u64,
    pub text_epoch: u64,
    pub version: u64,
    pub workspace: Arc<Vec<MirrorSnapshot>>,
}
struct Call {
    ticket: Ticket,
    canceled: bool,
    started: Instant,
}
pub(crate) struct Reply {
    pub ticket: Ticket,
    pub result: Result<Value, String>,
}
#[derive(Default)]
pub(super) struct State {
    registry: Registry,
    calls: HashMap<u64, Call>,
    replies: VecDeque<Reply>,
}
impl Client {
    pub(crate) fn provider_owner_ready(&self, owner: &str) -> bool {
        self.owner_active(owner)
    }
    pub(crate) fn language_provider(&self, kind: Kind, doc: &Document) -> Option<&Provider> {
        self.providers
            .registry
            .entries()
            .iter()
            .filter(|p| p.kind == kind && self.provider_owner_ready(&p.owner))
            .filter_map(|p| {
                let score = p.score(doc);
                (score > 0).then_some((score, p.id, p))
            })
            .max_by_key(|(score, id, _)| (*score, *id))
            .map(|(_, _, p)| p)
    }
    pub(crate) fn register_language_providers(&mut self, value: Value) -> Result<()> {
        let owners: Vec<_> = self.packages.iter().map(|p| p.id.as_str()).collect();
        self.providers.registry.replace(value, &owners)
    }
    pub(crate) fn language_provider_capacity(&self) -> bool {
        self.providers.calls.len() + self.providers.replies.len() < LIMIT
    }
    pub(crate) fn provider_registration_current(&self, ticket: &Ticket) -> bool {
        ticket.session == self.session
            && self.provider_owner_ready(&ticket.provider.owner)
            && self
                .providers
                .registry
                .current(&ticket.provider, ticket.epoch)
    }
    pub(crate) fn provider_ticket_current(&self, ticket: &Ticket, doc: &Document) -> bool {
        self.provider_registration_current(ticket)
            && ticket.document == doc.id
            && ticket.revision == doc.revision
            && ticket.text_epoch == doc.text_epoch()
            && self.service_document_current(doc, ticket.version)
    }
    pub(crate) fn request_language_provider(
        &mut self,
        kind: Kind,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        options: Value,
    ) -> Result<Option<Ticket>> {
        let doc = documents.get(active).context("No active document")?;
        let Some(provider) = self.language_provider(kind, doc).cloned() else {
            return Ok(None);
        };
        self.request_language_provider_from(&provider, documents, hidden, active, options)
            .map(Some)
    }
    pub(crate) fn language_providers(
        &self,
        kind: Kind,
        document: &Document,
        only: Option<&str>,
    ) -> Result<Vec<Provider>> {
        let mut providers: Vec<_> =
            self.providers
                .registry
                .entries()
                .iter()
                .filter(|provider| {
                    provider.kind == kind
                        && self.provider_owner_ready(&provider.owner)
                        && provider.score(document) > 0
                })
                .filter(|provider| {
                    only.is_none_or(|only| {
                        provider.action_kinds.as_ref().is_none_or(|kinds| {
                            kinds.iter().any(|kind| kind_intersects(only, kind))
                        })
                    })
                })
                .cloned()
                .collect();
        if providers.len() > LIMIT {
            bail!(
                "More than 8 extension code action providers match; disable providers or narrow the requested kind"
            );
        }
        providers.sort_by(|a, b| {
            b.score(document)
                .cmp(&a.score(document))
                .then_with(|| a.owner.cmp(&b.owner))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(providers)
    }
    pub(crate) fn mirrored_snapshotset(
        &self,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<Arc<Vec<MirrorSnapshot>>> {
        if documents.len().saturating_add(hidden.len()) > 128 {
            bail!("Code action snapshots support at most128 models");
        }
        let mut result = Vec::new();
        let mut ids = HashSet::new();
        let mut uris = HashSet::new();
        for document in documents.iter().chain(hidden) {
            let mirror = self
                .mirror
                .mirrors
                .get(&document.id)
                .context("Workspace target is not mirrored")?;
            if mirror.revision != document.revision
                || mirror.text_epoch != document.text_epoch()
                || !ids.insert(document.id)
                || !uris.insert(&mirror.uri)
            {
                bail!("Workspace mirror changed or has duplicate identity");
            }
            result.push(MirrorSnapshot {
                uri: mirror.uri.clone(),
                document: document.id,
                path: document.path.clone(),
                revision: document.revision,
                text_epoch: document.text_epoch(),
                version: mirror.version,
            });
        }
        result.sort_by_key(|target| target.document);
        Ok(Arc::new(result))
    }
    pub(crate) fn provider_workspace_current(
        &self,
        ticket: &Ticket,
        documents: &[Document],
        hidden: &[Document],
    ) -> bool {
        ticket.workspace.iter().all(|target| {
            documents
                .iter()
                .chain(hidden)
                .find(|document| document.id == target.document)
                .is_some_and(|document| {
                    document.path == target.path
                        && document.revision == target.revision
                        && document.text_epoch() == target.text_epoch
                        && self.mirror.mirrors.get(&document.id).is_some_and(|mirror| {
                            mirror.version == target.version
                                && mirror.uri == target.uri
                                && mirror.revision == target.revision
                                && mirror.text_epoch == target.text_epoch
                        })
                })
        })
    }
    pub(crate) fn request_language_provider_from(
        &mut self,
        provider: &Provider,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        options: Value,
    ) -> Result<Ticket> {
        let doc = documents.get(active).context("No active document")?;
        if !self.provider_owner_ready(&provider.owner)
            || !self
                .providers
                .registry
                .current(provider, self.providers.registry.epoch())
            || provider.score(doc) == 0
        {
            bail!("Language provider no longer matches its owner/document");
        }
        if !self.language_provider_capacity() {
            bail!("Language provider callback limit reached (8); wait for pending callbacks");
        }
        self.sync_with_hidden(documents, hidden, active)?;
        let mirror = self
            .mirror
            .mirrors
            .get(&doc.id)
            .context("Document is not mirrored")?;
        let workspace = if provider.kind == Kind::CodeAction {
            self.mirrored_snapshotset(documents, hidden)?
        } else {
            Arc::new(Vec::new())
        };
        let ticket = Ticket {
            id: self.next_id + 1,
            session: self.session,
            provider: provider.clone(),
            epoch: self.providers.registry.epoch(),
            document: doc.id,
            revision: doc.revision,
            text_epoch: doc.text_epoch(),
            version: mirror.version,
            workspace,
        };
        self.request("provideLanguage", json!({"session":self.session,"owner":ticket.provider.owner,
            "provider":ticket.provider.id,"document":doc.id,"version":ticket.version,
            "position":lsp::position(doc,doc.cursor),"includeDeclaration":true,"completionContext":options.get("context"),
            "range":options.get("range"),"selection":options.get("selection"),"actionContext":options.get("context"),
            "workspace":ticket.workspace.iter().map(|target| json!({"document":target.document,"uri":target.uri,"version":target.version})).collect::<Vec<_>>(),
            "options":options.get("options").cloned().unwrap_or_else(|| json!({"tabSize":4,"insertSpaces":true}))}))?;
        self.providers.calls.insert(
            ticket.id,
            Call {
                ticket: ticket.clone(),
                canceled: false,
                started: Instant::now(),
            },
        );
        Ok(ticket)
    }
    pub(crate) fn request_action_resolve(
        &mut self,
        original: &Ticket,
        item: &Value,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
    ) -> Result<Ticket> {
        let document = documents.get(active).context("No active document")?;
        if original.provider.kind != Kind::CodeAction
            || !original.provider.resolves
            || !self.provider_ticket_current(original, document)
            || !self.provider_workspace_current(original, documents, hidden)
        {
            bail!("Code action provider ownership or workspace changed");
        }
        let handle = item["_vscliCodeActionHandle"]
            .as_u64()
            .filter(|handle| *handle > 0 && *handle <= 9_007_199_254_740_991)
            .context("Code action has no current resolve handle")?;
        if !self.language_provider_capacity() {
            bail!("Language provider callback limit reached (8); wait for pending callbacks");
        }
        self.sync_with_hidden(documents, hidden, active)?;
        let mut ticket = original.clone();
        ticket.id = self.next_id + 1;
        self.request("resolveLanguageCodeAction",json!({"session":self.session,"owner":original.provider.owner,"provider":original.provider.id,"origin":original.id,"handle":handle,"document":document.id,"version":original.version}))?;
        self.providers.calls.insert(
            ticket.id,
            Call {
                ticket: ticket.clone(),
                canceled: false,
                started: Instant::now(),
            },
        );
        Ok(ticket)
    }
    pub(crate) fn request_completion_resolve(
        &mut self,
        original: &Ticket,
        item: &Value,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
    ) -> Result<Ticket> {
        let doc = documents.get(active).context("No active document")?;
        if original.provider.kind != Kind::Completion
            || !original.provider.resolves
            || !self.provider_ticket_current(original, doc)
        {
            bail!("Completion provider ownership or document changed");
        }
        let handle = item["_vscliCompletionHandle"]
            .as_u64()
            .filter(|handle| *handle > 0 && *handle <= 9_007_199_254_740_991)
            .context("Completion has no current resolve handle")?;
        if !self.language_provider_capacity() {
            bail!("Language provider callback limit reached (8); wait for pending callbacks");
        }
        self.sync_with_hidden(documents, hidden, active)?;
        let mut ticket = original.clone();
        ticket.id = self.next_id + 1;
        self.request(
            "resolveLanguageCompletion",
            json!({"session":self.session,
            "owner":original.provider.owner,"provider":original.provider.id,
            "origin":original.id,"handle":handle,"document":doc.id,"version":original.version}),
        )?;
        self.providers.calls.insert(
            ticket.id,
            Call {
                ticket: ticket.clone(),
                canceled: false,
                started: Instant::now(),
            },
        );
        Ok(ticket)
    }
    pub(crate) fn cancel_language_provider(&mut self, ticket: &Ticket) -> Result<()> {
        if ticket.session != self.session {
            return Ok(());
        }
        if let Some(call) = self.providers.calls.get_mut(&ticket.id) {
            if call.canceled {
                return Ok(());
            }
            call.canceled = true;
        }
        // Completed completion requests still own host item handles until retired.
        self.process
            .send(json!({"method":"cancelLanguageProvider","params":{
            "session":self.session,"owner":ticket.provider.owner,"request":ticket.id}}))?;
        // Retain the call and generic pending origin until the real reply or deadline.
        Ok(())
    }
    pub(crate) fn take_provider_replies(&mut self) -> VecDeque<Reply> {
        std::mem::take(&mut self.providers.replies)
    }
    pub(super) fn provider_response(&mut self, id: u64, mut message: Value) -> Result<()> {
        let Some(call) = self.providers.calls.remove(&id) else {
            return Ok(());
        };
        if call.canceled {
            return Ok(());
        }
        let result = if !message["error"].is_null() {
            Err(message["error"]["message"]
                .as_str()
                .unwrap_or("Language provider failed")
                .chars()
                .take(2048)
                .collect())
        } else {
            let value = message["result"].take();
            provider_result_budget(&value)
                .map(|()| value)
                .map_err(|e| e.to_string())
        };
        self.providers.replies.push_back(Reply {
            ticket: call.ticket,
            result,
        });
        Ok(())
    }
    pub(super) fn expire_language_providers(&mut self) -> Result<()> {
        let expired: Vec<_> = self
            .providers
            .calls
            .values()
            .filter(|c| c.started.elapsed() >= DEADLINE)
            .map(|c| c.ticket.clone())
            .collect();
        for ticket in expired {
            self.cancel_language_provider(&ticket)?;
            self.providers.calls.remove(&ticket.id);
            self.pending.remove(&ticket.id);
            self.providers.replies.push_back(Reply {
                ticket,
                result: Err("Language provider timed out".into()),
            });
        }
        Ok(())
    }
}
fn kind_intersects(a: &str, b: &str) -> bool {
    a.is_empty()
        || b.is_empty()
        || a == b
        || a.strip_prefix(b)
            .is_some_and(|suffix| suffix.starts_with('.'))
        || b.strip_prefix(a)
            .is_some_and(|suffix| suffix.starts_with('.'))
}
fn provider_result_budget(value: &Value) -> Result<()> {
    let mut stack = vec![(value, 0usize)];
    let (mut nodes, mut bytes) = (0, 0);
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 100_000 || depth > 32 {
            bail!("Language provider result exceeds node/depth budget")
        }
        match value {
            Value::String(s) => bytes += s.len(),
            Value::Array(a) => stack.extend(a.iter().map(|v| (v, depth + 1))),
            Value::Object(o) => {
                bytes += o.keys().map(String::len).sum::<usize>();
                stack.extend(o.values().map(|v| (v, depth + 1)));
            }
            _ => (),
        }
        if bytes > 1024 * 1024 {
            bail!("Language provider result exceeds 1 MiB")
        }
    }
    if serde_json::to_vec(value)?.len() > 1024 * 1024 {
        bail!("Language provider result exceeds 1 MiB")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, body: &str) -> PathBuf {
        let folder = root.join("providers");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(
            folder.join("package.json"),
            r#"{"publisher":"test","name":"providers","version":"1","main":"index.cjs"}"#,
        )
        .unwrap();
        std::fs::write(folder.join("index.cjs"), body).unwrap();
        folder
    }
    fn ready(client: &mut Client, docs: &mut [Document]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready {
            client.poll(docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn completed_native_completion_lease_retirement_invalidates_original_host_handle() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerCompletionItemProvider('*',{provideCompletionItems:()=>[{label:'item'}],resolveCompletionItem:item=>item});"#,
        );
        let mut docs = vec![Document::from_text("α🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Completion, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while client.providers.replies.is_empty() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let completion = client
            .take_provider_replies()
            .pop_front()
            .unwrap()
            .result
            .unwrap();
        assert!(!client.providers.calls.contains_key(&ticket.id));
        assert!(client.provider_ticket_current(&ticket, &docs[0]));
        client.cancel_language_provider(&ticket).unwrap();
        let resolve = client
            .request_completion_resolve(&ticket, &completion["items"][0], &docs, &[], 0)
            .unwrap();
        while client.providers.replies.is_empty() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let reply = client.take_provider_replies().pop_front().unwrap();
        assert_eq!(reply.ticket.id, resolve.id);
        assert!(
            reply
                .result
                .unwrap_err()
                .contains("Stale completion resolve handle")
        );
        assert_eq!(docs[0].text.to_string(), "α🙂\r\n");
    }
    #[test]
    fn provider_and_hidden_service_versions_reject_edit_undo_before_sync() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerHoverProvider('*',{provideHover:()=>new Promise(()=>{})});"#,
        );
        let mut docs = vec![Document::from_text("α🙂\r\n")];
        let mut hidden = vec![Document::from_text("hidden 猫\r\n")];
        let package = Package::read(&path).unwrap();
        let prepared =
            Client::prepare_with_hidden(&docs, &hidden, 0, &Settings::default()).unwrap();
        let mut client =
            Client::start_many_prepared("node", &[package], root.path(), prepared).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready {
            client
                .poll_with_hidden(&mut docs, &mut hidden, 0, &Settings::default())
                .unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let ticket = client
            .request_language_provider(Kind::Hover, &docs, &hidden, 0, json!({}))
            .unwrap()
            .unwrap();
        let revision = docs[0].revision;
        for doc in docs.iter_mut().chain(&mut hidden) {
            assert!(client.service_document_current(doc, 1));
            doc.insert("transient", false);
            doc.undo();
            assert!(!client.service_document_current(doc, 1));
        }
        assert_eq!(docs[0].revision, revision);
        assert!(!client.provider_ticket_current(&ticket, &docs[0]));
        client.sync_with_hidden(&docs, &hidden, 0).unwrap();
        for doc in docs.iter().chain(&hidden) {
            assert!(client.service_document_current(doc, 2));
            assert!(!client.service_document_current(doc, 1));
        }
        assert!(!client.provider_ticket_current(&ticket, &docs[0]));
        // Even a forged current exposed version cannot revive an old ticket epoch.
        let mut forged = ticket.clone();
        forged.version = 2;
        assert!(!client.provider_ticket_current(&forged, &docs[0]));
        hidden[0].redo();
        assert_eq!(hidden[0].text.to_string(), "transienthidden 猫\r\n");
    }
    #[test]
    fn canceled_callbacks_retain_slots_until_reply_and_never_cross_owner_session_or_registry() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerHoverProvider('*',{provideHover:()=>new Promise(()=>{})});"#,
        );
        let mut docs = vec![Document::from_text("α🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let mut tickets = Vec::new();
        for _ in 0..8 {
            let ticket = client
                .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
                .unwrap()
                .unwrap();
            client.cancel_language_provider(&ticket).unwrap();
            tickets.push(ticket);
        }
        assert_eq!(client.providers.calls.len(), 8);
        assert!(
            client
                .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
                .is_err()
        );
        assert!(client.provider_ticket_current(&tickets[0], &docs[0]));
        let mut forged = tickets[0].clone();
        forged.session += 1;
        assert!(!client.provider_ticket_current(&forged, &docs[0]));
        forged = tickets[0].clone();
        forged.provider.owner = "other.owner".into();
        assert!(!client.provider_ticket_current(&forged, &docs[0]));
        let origin = tickets[0].id;
        assert!(
            client
                .validate_native_origin(
                    client.session,
                    "test.providers",
                    Some(origin),
                    Some("test.providers")
                )
                .is_ok()
        );
        assert!(
            client
                .validate_native_origin(
                    client.session,
                    "test.providers",
                    Some(origin),
                    Some("other.owner")
                )
                .is_err()
        );
        client.register_language_providers(json!([])).unwrap();
        assert!(!client.provider_ticket_current(&tickets[0], &docs[0]));
        for ticket in tickets {
            client
                .provider_response(ticket.id, json!({"result":{"contents":"late"}}))
                .unwrap();
            client.pending.remove(&ticket.id);
        }
        assert!(client.take_provider_replies().is_empty());
        assert!(client.providers.calls.is_empty());
        assert_eq!(docs[0].text.to_string(), "α🙂\r\n");
    }
    #[test]
    fn callback_timeout_and_malformed_result_are_bounded_and_retryable() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerHoverProvider('*',{provideHover:()=>new Promise(()=>{})});"#,
        );
        let mut docs = vec![Document::from_text("untitled")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        client.providers.calls.get_mut(&ticket.id).unwrap().started = Instant::now() - DEADLINE;
        client.expire_language_providers().unwrap();
        assert!(!client.pending.contains_key(&ticket.id));
        let reply = client.take_provider_replies().pop_front().unwrap();
        assert!(reply.result.unwrap_err().contains("timed out"));
        let ticket = client
            .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        client
            .provider_response(ticket.id, json!({"result":"x".repeat(1024*1024+1)}))
            .unwrap();
        assert!(
            client
                .take_provider_replies()
                .pop_front()
                .unwrap()
                .result
                .is_err()
        );
        let mut nested = json!(null);
        for _ in 0..33 {
            nested = json!([nested]);
        }
        assert!(provider_result_budget(&nested).is_err());
        assert!(provider_result_budget(&json!(vec![0; 100_001])).is_err());
    }
}

#[cfg(test)]
mod code_action_tests {
    use super::*;
    fn client(root: &Path, documents: &mut [Document], body: &str) -> Client {
        let extension = root.join("actions");
        std::fs::create_dir(&extension).unwrap();
        std::fs::write(
            extension.join("package.json"),
            r#"{"publisher":"test","name":"actions","version":"1","main":"index.cjs"}"#,
        )
        .unwrap();
        std::fs::write(extension.join("index.cjs"), body).unwrap();
        let mut client =
            Client::start("node", &extension, root, documents, 0, &Settings::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !client.ready {
            client.poll(documents, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        client
    }
    fn options() -> Value {
        json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":3}},"selection":{"anchor":{"line":0,"character":3},"active":{"line":0,"character":1}},"context":{"triggerKind":1}})
    }
    fn reply(client: &mut Client, docs: &mut [Document], id: u64) -> Reply {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            client.poll(docs, 0, &Settings::default()).unwrap();
            if let Some(reply) = client
                .take_provider_replies()
                .into_iter()
                .find(|reply| reply.ticket.id == id)
            {
                return reply;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn code_action_cohort_enumerates_every_matching_provider_and_rejects_overflow() {
        let root = tempfile::tempdir().unwrap();
        let mut docs = vec![Document::from_text("猫🙂x\r\n")];
        let mut client = client(
            root.path(),
            &mut docs,
            r#"const v=require('vscode');exports.activate=()=>{v.languages.registerCodeActionsProvider('*',{provideCodeActions:()=>[]},{providedCodeActionKinds:[v.CodeActionKind.QuickFix]});v.languages.registerCodeActionsProvider('*',{provideCodeActions:()=>[]},{providedCodeActionKinds:[v.CodeActionKind.Refactor]});};"#,
        );
        let all = client
            .language_providers(Kind::CodeAction, &docs[0], None)
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            client
                .language_providers(Kind::CodeAction, &docs[0], Some("refactor.extract"))
                .unwrap()
                .len(),
            1
        );
        let ticket = client
            .request_language_provider_from(&all[0], &docs, &[], 0, options())
            .unwrap();
        assert_eq!(ticket.workspace.len(), 1);
        assert!(
            reply(&mut client, &mut docs, ticket.id)
                .result
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        client.register_language_providers(json!((1..=9).map(|id|json!({"id":id,"owner":"test.actions","type":"codeaction","selector":[{"language":"*"}],"triggers":[],"resolves":false})).collect::<Vec<_>>())).unwrap();
        assert!(
            client
                .language_providers(Kind::CodeAction, &docs[0], None)
                .unwrap_err()
                .to_string()
                .contains("More than 8")
        );
    }
    #[test]
    fn framed_code_action_resolve_keeps_original_target_snapshot_and_rejects_secondary_drift() {
        let root = tempfile::tempdir().unwrap();
        let mut docs = vec![
            Document::from_text("猫🙂x\r\n"),
            Document::from_text("header\r\n"),
        ];
        let mut client = client(
            root.path(),
            &mut docs,
            r#"const v=require('vscode');let original;exports.activate=()=>v.languages.registerCodeActionsProvider('*',{provideCodeActions(){original=new v.CodeAction('original',v.CodeActionKind.QuickFix);original.opaque={closure:()=>42};original.opaque.self=original.opaque;return [original];},resolveCodeAction(item){if(item!==original||item.opaque.closure()!==42||item.opaque.self!==item.opaque)throw Error('identity lost');item.title='mutated';item.edit=new v.WorkspaceEdit();item.edit.insert(v.workspace.textDocuments[1].uri,new v.Position(0,0),'// import\r\n');return item;}});"#,
        );
        let provider = client
            .language_providers(Kind::CodeAction, &docs[0], None)
            .unwrap()
            .remove(0);
        let original = client
            .request_language_provider_from(&provider, &docs, &[], 0, options())
            .unwrap();
        let rows = reply(&mut client, &mut docs, original.id).result.unwrap();
        let resolve = client
            .request_action_resolve(&original, &rows[0], &docs, &[], 0)
            .unwrap();
        assert!(Arc::ptr_eq(&resolve.workspace, &original.workspace));
        let resolved = reply(&mut client, &mut docs, resolve.id).result.unwrap();
        assert_eq!(resolved["title"], "original");
        assert_eq!(
            resolved["edit"]["documentChanges"][0]["textDocument"]["version"],
            1
        );
        docs[1].insert("x", false);
        docs[1].undo();
        assert!(!client.provider_workspace_current(&original, &docs, &[]));
        assert!(
            client
                .request_action_resolve(&original, &rows[0], &docs, &[], 0)
                .is_err()
        );
        assert_eq!(docs[1].text.to_string(), "header\r\n");
    }
    #[test]
    fn completed_action_origin_cancellation_retires_host_handle_without_native_call_record() {
        let root = tempfile::tempdir().unwrap();
        let mut docs = vec![Document::from_text("猫🙂x")];
        let mut client = client(
            root.path(),
            &mut docs,
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerCodeActionsProvider('*',{provideCodeActions:()=>[new v.CodeAction('lazy',v.CodeActionKind.QuickFix)],resolveCodeAction:item=>item});"#,
        );
        let provider = client
            .language_providers(Kind::CodeAction, &docs[0], None)
            .unwrap()
            .remove(0);
        let original = client
            .request_language_provider_from(&provider, &docs, &[], 0, options())
            .unwrap();
        let rows = reply(&mut client, &mut docs, original.id).result.unwrap();
        assert!(!client.providers.calls.contains_key(&original.id));
        client.cancel_language_provider(&original).unwrap();
        let resolve = client
            .request_action_resolve(&original, &rows[0], &docs, &[], 0)
            .unwrap();
        assert!(
            reply(&mut client, &mut docs, resolve.id)
                .result
                .unwrap_err()
                .contains("Stale code action")
        );
    }
}
