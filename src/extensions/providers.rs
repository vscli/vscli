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
    pub language_epoch: u64,
    pub version: u64,
}
#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: u64,
    pub session: u64,
    pub provider: Arc<Provider>,
    pub epoch: u64,
    pub document: u64,
    pub revision: u64,
    pub text_epoch: u64,
    pub language_epoch: u64,
    pub version: u64,
    pub workspace: Arc<Vec<MirrorSnapshot>>,
}
#[derive(Clone)]
pub(crate) struct NavigationRankIntent {
    source: Arc<NavigationRankSource>,
}
struct NavigationRankSource {
    session: u64,
    epoch: u64,
    document: u64,
    revision: u64,
    text_epoch: u64,
    language_epoch: u64,
    version: u64,
    uri: String,
    kind: Kind,
    providers: Vec<Provider>,
    owners: Vec<Arc<std::sync::atomic::AtomicBool>>,
}
struct NavigationRanking {
    id: u64,
    intent: NavigationRankIntent,
    started: Instant,
    canceled: bool,
    expired: bool,
}
pub(crate) struct NavigationRankReply {
    pub id: u64,
    pub result: Result<Vec<Provider>, String>,
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
    ranking: Option<NavigationRanking>,
    ranked: VecDeque<NavigationRankReply>,
    calls: HashMap<u64, Call>,
    occupied: HashMap<u64, Ticket>,
    replies: VecDeque<Reply>,
    signature: Option<Ticket>,
    signature_started: Option<Instant>,
    signature_fenced: bool,
    symbols: Option<Ticket>,
    symbols_started: Option<Instant>,
    symbols_fenced: bool,
}
impl Client {
    pub(crate) fn symbols_available(&self) -> bool {
        !self.providers.symbols_fenced
            && self.providers.symbols.is_none()
            && !self
                .providers
                .calls
                .values()
                .any(|call| call.ticket.provider.kind == Kind::Symbols)
            && !self
                .providers
                .replies
                .iter()
                .any(|reply| reply.ticket.provider.kind == Kind::Symbols)
            && self.language_provider_capacity()
    }
    pub(crate) fn symbol_channel_closed(&self) -> bool {
        self.providers.symbols_fenced
    }
    pub(crate) fn symbol_source_identity(&self, provider: &Provider) -> Option<(u64, u64)> {
        (provider.kind == Kind::Symbols
            && self.provider_owner_ready(&provider.owner)
            && self
                .providers
                .registry
                .current(provider, self.providers.registry.epoch()))
        .then_some((self.session, self.providers.registry.epoch()))
    }
    pub(super) fn symbols_released(&mut self, value: Value) -> Result<()> {
        if value["session"].as_u64() != Some(self.session) {
            return Ok(());
        }
        let Some(ticket) = &self.providers.symbols else {
            return Ok(());
        };
        if value["request"].as_u64() != Some(ticket.id) {
            return Ok(());
        }
        if value["owner"].as_str() != Some(ticket.provider.owner.as_str())
            || value["provider"].as_u64() != Some(ticket.provider.id)
        {
            bail!("Invalid document symbol callback release ownership");
        }
        self.providers.symbols = None;
        self.providers.symbols_started = None;
        self.providers.symbols_fenced = false;
        Ok(())
    }
    pub(crate) fn signature_available(&self) -> bool {
        !self.providers.signature_fenced
            && self.providers.signature.is_none()
            && !self
                .providers
                .calls
                .values()
                .any(|call| call.ticket.provider.kind == Kind::Signature)
            && !self
                .providers
                .replies
                .iter()
                .any(|reply| reply.ticket.provider.kind == Kind::Signature)
            && self.language_provider_capacity()
    }
    pub(crate) fn signature_channel_closed(&self) -> bool {
        self.providers.signature_fenced
    }
    pub(crate) fn signature_source_identity(&self, provider: &Provider) -> Option<(u64, u64)> {
        (provider.kind == Kind::Signature
            && self.provider_owner_ready(&provider.owner)
            && self
                .providers
                .registry
                .current(provider, self.providers.registry.epoch()))
        .then_some((self.session, self.providers.registry.epoch()))
    }
    pub(super) fn signature_released(&mut self, value: Value) -> Result<()> {
        if value["session"].as_u64() != Some(self.session) {
            return Ok(());
        }
        let Some(ticket) = &self.providers.signature else {
            return Ok(());
        };
        if value["request"].as_u64() != Some(ticket.id) {
            return Ok(());
        }
        if value["owner"].as_str() != Some(ticket.provider.owner.as_str())
            || value["provider"].as_u64() != Some(ticket.provider.id)
        {
            bail!("Invalid signature callback release ownership");
        }
        self.providers.signature = None;
        self.providers.signature_started = None;
        self.providers.signature_fenced = false;
        Ok(())
    }
    pub(crate) fn release_signature_help(&self, ticket: &Ticket, help: &Value) -> Result<()> {
        if ticket.session != self.session || ticket.provider.kind != Kind::Signature {
            return Ok(());
        }
        let Some(handle) = help["_vscliSignatureHelpHandle"].as_u64() else {
            return Ok(());
        };
        self.process
            .send(json!({"method":"cancelLanguageProvider","params":{
            "session":self.session,"owner":ticket.provider.owner,"provider":ticket.provider.id,
            "document":ticket.document,"request":ticket.id,"releaseSignatureHelp":true,
            "signatureHandle":handle}}))
    }
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
        self.language_provider_slots() > 0
    }
    pub(crate) fn language_callback_occupied(&self, ticket: &Ticket) -> bool {
        self.providers
            .occupied
            .get(&ticket.id)
            .is_some_and(|original| {
                original.session == ticket.session
                    && original.provider.id == ticket.provider.id
                    && original.provider.owner == ticket.provider.owner
            })
    }
    pub(crate) fn language_provider_slots(&self) -> usize {
        let mut ids: HashSet<u64> = self.providers.occupied.keys().copied().collect();
        ids.extend(self.providers.calls.keys().copied());
        ids.extend(self.providers.replies.iter().map(|reply| reply.ticket.id));
        LIMIT.saturating_sub(ids.len())
    }
    /// Only the exact actual-callback settlement frame releases occupation.
    /// A logical reply, cancellation, timeout or owner retirement does not.
    pub(super) fn language_provider_released(&mut self, value: Value) -> Result<()> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Released {
            session: u64,
            owner: String,
            provider: u64,
            request: u64,
        }
        result_budget(&value, 2)?;
        let release: Released = serde_json::from_value(value)?;
        anyhow::ensure!(
            release.owner.len() <= 128
                && release.provider > 0
                && release.provider <= 9_007_199_254_740_991
                && release.request > 0
                && release.request <= 9_007_199_254_740_991,
            "Invalid language callback release"
        );
        if release.session != self.session {
            return Ok(());
        }
        let Some(ticket) = self.providers.occupied.get(&release.request) else {
            return Ok(());
        };
        anyhow::ensure!(
            ticket.session == release.session
                && ticket.provider.id == release.provider
                && ticket.provider.owner == release.owner,
            "Language callback release ownership mismatch"
        );
        self.providers.occupied.remove(&release.request);
        Ok(())
    }
    pub(crate) fn navigation_registry_epoch(&self) -> u64 {
        self.providers.registry.epoch()
    }
    pub(crate) fn navigation_registry_current(&self, epoch: u64) -> bool {
        self.providers.registry.epoch() == epoch
    }
    pub(crate) fn provider_registration_matches(
        &self,
        provider: &Provider,
        epoch: u64,
        document: &Document,
    ) -> bool {
        self.provider_owner_ready(&provider.owner)
            && self.providers.registry.current(provider, epoch)
            && (provider.kind.is_navigation() || provider.score(document) > 0)
    }
    pub(crate) fn navigation_family_registered(&self, kind: Kind) -> bool {
        kind.is_navigation()
            && self
                .providers
                .registry
                .entries()
                .iter()
                .any(|provider| provider.kind == kind && self.provider_owner_ready(&provider.owner))
    }
    pub(crate) fn capture_navigation_rank(
        &mut self,
        kind: Kind,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
    ) -> Result<NavigationRankIntent> {
        anyhow::ensure!(kind.is_navigation(), "Invalid navigation provider family");
        self.sync_with_hidden(documents, hidden, active)?;
        let doc = documents
            .get(active)
            .context("No active navigation source")?;
        let mirror = self
            .mirror
            .mirrors
            .get(&doc.id)
            .context("Navigation source is not mirrored")?;
        let providers: Vec<_> = self
            .providers
            .registry
            .entries()
            .iter()
            .filter(|provider| provider.kind == kind && self.provider_owner_ready(&provider.owner))
            .cloned()
            .collect();
        anyhow::ensure!(
            providers.len() <= crate::extension_providers::MAX_PROVIDERS,
            "Navigation registry cohort exceeds 128"
        );
        let owners = providers
            .iter()
            .map(|provider| self.owner_lifetimes.capture(&provider.owner))
            .collect::<Result<Vec<_>>>()?;
        Ok(NavigationRankIntent {
            source: Arc::new(NavigationRankSource {
                session: self.session,
                epoch: self.providers.registry.epoch(),
                document: doc.id,
                revision: doc.revision,
                text_epoch: doc.text_epoch(),
                language_epoch: doc.language_epoch(),
                version: mirror.version,
                uri: mirror.uri.clone(),
                kind,
                providers,
                owners,
            }),
        })
    }
    pub(crate) fn navigation_rank_current(
        &self,
        intent: &NavigationRankIntent,
        document: &Document,
    ) -> bool {
        let source = &intent.source;
        source.session == self.session
            && source.epoch == self.providers.registry.epoch()
            && source.document == document.id
            && source.revision == document.revision
            && source.text_epoch == document.text_epoch()
            && source.language_epoch == document.language_epoch()
            && self.service_document_current(document, source.version)
            && document_uri(document).ok().as_ref() == Some(&source.uri)
            && source
                .owners
                .iter()
                .all(|owner| !owner.load(std::sync::atomic::Ordering::Acquire))
            && source.providers.iter().all(|provider| {
                self.provider_owner_ready(&provider.owner)
                    && self.providers.registry.current(provider, source.epoch)
            })
    }
    pub(crate) fn navigation_rank_available(&self) -> bool {
        self.providers.ranking.is_none() && self.providers.ranked.is_empty()
    }
    pub(crate) fn request_navigation_rank(
        &mut self,
        intent: &NavigationRankIntent,
        document: &Document,
    ) -> Result<u64> {
        anyhow::ensure!(
            self.navigation_rank_available(),
            "Previous navigation matching request is still pending"
        );
        anyhow::ensure!(
            self.navigation_rank_current(intent, document),
            "Navigation matching source changed"
        );
        let id = self
            .next_id
            .checked_add(1)
            .filter(|id| *id <= 9_007_199_254_740_991)
            .context("Navigation matching request identities exhausted")?;
        let source = &intent.source;
        self.request("rankLanguageProviders", json!({"session":source.session,"type":source.kind,
            "document":source.document,"version":source.version,"languageEpoch":source.language_epoch}))?;
        self.providers.ranking = Some(NavigationRanking {
            id,
            intent: intent.clone(),
            started: Instant::now(),
            canceled: false,
            expired: false,
        });
        Ok(id)
    }
    pub(crate) fn cancel_navigation_rank(&mut self, id: u64) {
        if let Some(rank) = &mut self.providers.ranking
            && rank.id == id
        {
            rank.canceled = true;
        }
    }
    pub(crate) fn take_navigation_ranks(&mut self) -> VecDeque<NavigationRankReply> {
        std::mem::take(&mut self.providers.ranked)
    }
    pub(super) fn provider_rank_response(&mut self, id: u64, mut message: Value) -> Result<()> {
        if !self
            .providers
            .ranking
            .as_ref()
            .is_some_and(|rank| rank.id == id)
        {
            return Ok(());
        }
        let rank = self.providers.ranking.take().unwrap();
        if rank.canceled || rank.expired {
            return Ok(());
        }
        let result = (|| -> Result<Vec<Provider>> {
            if !message["error"].is_null() {
                bail!(
                    "{}",
                    message["error"]["message"]
                        .as_str()
                        .unwrap_or("Navigation matching failed")
                        .chars()
                        .take(2048)
                        .collect::<String>()
                );
            }
            let value = message["result"].take();
            crate::decorations::json_budget(&value, 64 * 1024)?;
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Ranked {
                providers: Vec<Entry>,
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Entry {
                id: u64,
                owner: String,
            }
            anyhow::ensure!(
                value["providers"]
                    .as_array()
                    .is_some_and(|entries| entries.len() <= 128),
                "Invalid navigation rank count"
            );
            let ranked: Ranked = serde_json::from_value(value)?;
            let mut seen = HashSet::new();
            ranked
                .providers
                .into_iter()
                .map(|entry| {
                    anyhow::ensure!(
                        entry.id > 0 && seen.insert(entry.id),
                        "Duplicate or invalid ranked navigation provider"
                    );
                    let provider = rank
                        .intent
                        .source
                        .providers
                        .iter()
                        .find(|provider| provider.id == entry.id && provider.owner == entry.owner)
                        .context("Navigation rank borrowed a different registry owner or family")?;
                    Ok(provider.clone())
                })
                .collect()
        })()
        .map_err(|error| format!("{error:#}"));
        self.providers
            .ranked
            .push_back(NavigationRankReply { id, result });
        Ok(())
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
            && ticket.language_epoch == doc.language_epoch()
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
                || mirror.eol != document.eol
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
                language_epoch: document.language_epoch(),
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
                        && document.language_epoch() == target.language_epoch
                        && self.mirror.mirrors.get(&document.id).is_some_and(|mirror| {
                            mirror.version == target.version
                                && mirror.uri == target.uri
                                && mirror.revision == target.revision
                                && mirror.text_epoch == target.text_epoch
                                && mirror.eol == document.eol
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
            || (!provider.kind.is_navigation() && provider.score(doc) == 0)
        {
            bail!("Language provider no longer matches its owner/document");
        }
        if !self.language_provider_capacity() {
            bail!("Language provider callback limit reached (8); wait for pending callbacks");
        }
        if provider.kind == Kind::Signature && !self.signature_available() {
            bail!(
                "Signature callback is still running or fenced; wait for actual release or restart the extension host"
            );
        }
        if provider.kind == Kind::Symbols && !self.symbols_available() {
            bail!(
                "Document symbol callback is still running or fenced; wait for actual release or restart the extension host"
            );
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
            id: self
                .next_id
                .checked_add(1)
                .filter(|id| *id <= 9_007_199_254_740_991)
                .context("Language provider request identities exhausted")?,
            session: self.session,
            provider: Arc::new(provider.clone()),
            epoch: self.providers.registry.epoch(),
            document: doc.id,
            revision: doc.revision,
            text_epoch: doc.text_epoch(),
            language_epoch: doc.language_epoch(),
            version: mirror.version,
            workspace,
        };
        self.request("provideLanguage", json!({"session":self.session,"owner":ticket.provider.owner,
            "provider":ticket.provider.id,"document":doc.id,"version":ticket.version,
            "position":lsp::position(doc,doc.cursor),"includeDeclaration":true,"completionContext":options.get("context"),
            "signatureContext":options.get("context"),"signatureRequest":provider.kind == Kind::Signature,
            "symbolRequest":provider.kind == Kind::Symbols,
            "range":options.get("range"),"selection":options.get("selection"),"actionContext":options.get("context"),
            "workspace":ticket.workspace.iter().map(|target| json!({"document":target.document,"uri":target.uri,"version":target.version})).collect::<Vec<_>>(),
            "options":options.get("options").cloned().unwrap_or_else(|| json!({"tabSize":4,"insertSpaces":true}))}))?;
        if provider.kind == Kind::Signature {
            self.providers.signature = Some(ticket.clone());
            self.providers.signature_started = Some(Instant::now());
        }
        if provider.kind == Kind::Symbols {
            self.providers.symbols = Some(ticket.clone());
            self.providers.symbols_started = Some(Instant::now());
        }
        self.providers.occupied.insert(ticket.id, ticket.clone());
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
        ticket.id = self
            .next_id
            .checked_add(1)
            .filter(|id| *id <= 9_007_199_254_740_991)
            .context("Language provider request identities exhausted")?;
        self.request("resolveLanguageCodeAction",json!({"session":self.session,"owner":original.provider.owner,"provider":original.provider.id,"origin":original.id,"handle":handle,"document":document.id,"version":original.version}))?;
        self.providers.occupied.insert(ticket.id, ticket.clone());
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
        ticket.id = self
            .next_id
            .checked_add(1)
            .filter(|id| *id <= 9_007_199_254_740_991)
            .context("Language provider request identities exhausted")?;
        self.request(
            "resolveLanguageCompletion",
            json!({"session":self.session,
            "owner":original.provider.owner,"provider":original.provider.id,
            "origin":original.id,"handle":handle,"document":doc.id,"version":original.version}),
        )?;
        self.providers.occupied.insert(ticket.id, ticket.clone());
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
            anyhow::ensure!(
                call.ticket.session == ticket.session
                    && call.ticket.provider.id == ticket.provider.id
                    && call.ticket.provider.owner == ticket.provider.owner,
                "Language callback cancellation ownership mismatch"
            );
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
        if call.canceled && !matches!(call.ticket.provider.kind, Kind::Signature | Kind::Symbols) {
            return Ok(());
        }
        if call.ticket.provider.kind == Kind::Signature
            && self
                .providers
                .signature
                .as_ref()
                .is_some_and(|occupied| occupied.id == id)
            && message["error"]["message"].as_str() == Some("Language provider deadline exceeded")
        {
            self.providers.signature_fenced = true;
        }
        if call.ticket.provider.kind == Kind::Symbols
            && self
                .providers
                .symbols
                .as_ref()
                .is_some_and(|occupied| occupied.id == id)
            && message["error"]["message"].as_str() == Some("Language provider deadline exceeded")
        {
            self.providers.symbols_fenced = true;
        }
        let result = if call.canceled {
            Err(match call.ticket.provider.kind {
                Kind::Symbols => "Document symbol invocation canceled",
                _ => "Signature invocation canceled",
            }
            .into())
        } else if !message["error"].is_null() {
            Err(message["error"]["message"]
                .as_str()
                .unwrap_or("Language provider failed")
                .chars()
                .take(2048)
                .collect())
        } else {
            let value = message["result"].take();
            (if call.ticket.provider.kind == Kind::Symbols {
                result_budget(&value, 40)
            } else {
                provider_result_budget(&value)
            })
            .and_then(|()| {
                if call.ticket.provider.kind == Kind::Signature
                    && serde_json::to_vec(&value)?.len() > 256 * 1024
                {
                    bail!("Signature help exceeds 256 KiB");
                }
                Ok(())
            })
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
        if let Some(rank) = &mut self.providers.ranking
            && !rank.canceled
            && !rank.expired
            && rank.started.elapsed() >= DEADLINE
        {
            rank.expired = true;
            self.providers.ranked.push_back(NavigationRankReply {
                id: rank.id,
                result: Err(
                    "Navigation matching timed out; original request remains pending".into(),
                ),
            });
        }
        // A canceled wire reply may arrive long before the actual callback ends.
        // Keep its own clock after the generic Call has been removed.
        if self.providers.signature.is_some()
            && self
                .providers
                .signature_started
                .is_some_and(|started| started.elapsed() >= DEADLINE)
        {
            self.providers.signature_fenced = true;
        }
        if self.providers.symbols.is_some()
            && self
                .providers
                .symbols_started
                .is_some_and(|started| started.elapsed() >= DEADLINE)
        {
            self.providers.symbols_fenced = true;
        }
        let expired: Vec<_> = self
            .providers
            .calls
            .values()
            .filter(|c| c.started.elapsed() >= DEADLINE)
            .map(|c| c.ticket.clone())
            .collect();
        for ticket in expired {
            if ticket.provider.kind == Kind::Signature
                && self
                    .providers
                    .signature
                    .as_ref()
                    .is_some_and(|occupied| occupied.id == ticket.id)
            {
                self.providers.signature_fenced = true;
            }
            if ticket.provider.kind == Kind::Symbols
                && self
                    .providers
                    .symbols
                    .as_ref()
                    .is_some_and(|occupied| occupied.id == ticket.id)
            {
                self.providers.symbols_fenced = true;
            }
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
    result_budget(value, 32)
}
fn result_budget(value: &Value, maximum_depth: usize) -> Result<()> {
    let mut stack = vec![(value, 0usize)];
    let (mut nodes, mut bytes) = (0, 0);
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 100_000 || depth > maximum_depth {
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
    fn signature_reply(client: &mut Client, docs: &mut [Document], id: u64) -> Reply {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            client.poll(docs, 0, &Settings::default()).unwrap();
            if let Some(reply) = client.take_provider_replies().pop_front() {
                assert_eq!(reply.ticket.id, id);
                return reply;
            }
            assert!(Instant::now() < deadline, "No signature reply for {id}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn rank_fixture() -> (tempfile::TempDir, Client, Vec<Document>) {
        let root = tempfile::tempdir().unwrap();
        let package = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=c=>{
c.subscriptions.push(v.languages.registerDefinitionProvider({language:'*',exclusive:true},{provideDefinition(){throw Error('ranking must not call provider');}}));
c.subscriptions.push(v.languages.registerDefinitionProvider({language:'cpp',exclusive:true},{provideDefinition(){throw Error('ranking must not call provider');}}));
c.subscriptions.push(v.languages.registerHoverProvider('*',{provideHover(){throw Error('wrong family');}}));};"#,
        );
        let mut doc = Document::from_text("猫🙂\r\n");
        doc.set_language_override(Some("cpp".into())).unwrap();
        let mut docs = vec![doc];
        let mut client = Client::start(
            "node",
            &package,
            root.path(),
            &docs,
            0,
            &Settings::default(),
        )
        .unwrap();
        ready(&mut client, &mut docs);
        assert_eq!(client.providers.registry.entries().len(), 3);
        (root, client, docs)
    }
    fn rank_frame(client: &mut Client, id: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(frame) = client.process.receive().unwrap()
                && frame["id"].as_u64() == Some(id)
            {
                assert!(client.pending.remove(&id).is_some());
                return frame;
            }
            assert!(Instant::now() < deadline, "Real rank frame did not arrive");
            std::thread::yield_now();
        }
    }
    #[test]
    fn navigation_rank_uses_real_host_order_without_invoking_callbacks() {
        let (_root, mut client, docs) = rank_fixture();
        let intent = client
            .capture_navigation_rank(Kind::Definition, &docs, &[], 0)
            .unwrap();
        let first = intent.source.providers[0].id;
        assert_eq!(intent.source.providers[0].score(&docs[0]), 5);
        assert_eq!(intent.source.providers[1].score(&docs[0]), 10);
        let id = client.request_navigation_rank(&intent, &docs[0]).unwrap();
        let frame = rank_frame(&mut client, id);
        client.provider_rank_response(id, frame).unwrap();
        let reply = client.take_navigation_ranks().pop_front().unwrap();
        assert_eq!(reply.id, id);
        assert_eq!(
            reply
                .result
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![first]
        );
        assert!(client.navigation_rank_available());
        assert_eq!(client.language_provider_slots(), 8);
        assert!(client.providers.calls.is_empty());
        assert!(client.providers.occupied.is_empty());
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
        assert!(!docs[0].dirty());
    }
    #[test]
    fn canceled_and_expired_rank_keep_actual_slot_until_exact_real_reply() {
        let (_root, mut client, docs) = rank_fixture();
        let intent = client
            .capture_navigation_rank(Kind::Definition, &docs, &[], 0)
            .unwrap();
        for expire in [false, true] {
            let id = client.request_navigation_rank(&intent, &docs[0]).unwrap();
            if expire {
                client.providers.ranking.as_mut().unwrap().started = Instant::now() - DEADLINE;
                client.expire_language_providers().unwrap();
                client.expire_language_providers().unwrap();
                let errors = client.take_navigation_ranks();
                assert_eq!(errors.len(), 1);
                assert_eq!(errors[0].id, id);
                assert!(errors[0].result.as_ref().unwrap_err().contains("timed out"));
            } else {
                client.cancel_navigation_rank(id);
            }
            assert!(client.pending.contains_key(&id));
            assert!(!client.navigation_rank_available());
            assert!(client.request_navigation_rank(&intent, &docs[0]).is_err());
            client
                .provider_rank_response(id + 1, json!({"result":{"providers":[]}}))
                .unwrap();
            assert!(
                !client.navigation_rank_available(),
                "Foreign reply cannot free actual slot"
            );
            assert_eq!(client.language_provider_slots(), 8);
            let frame = rank_frame(&mut client, id);
            client.provider_rank_response(id, frame.clone()).unwrap();
            assert!(client.take_navigation_ranks().is_empty());
            assert!(client.navigation_rank_available());
            client.provider_rank_response(id, frame).unwrap();
            assert!(client.take_navigation_ranks().is_empty());
        }
    }
    #[test]
    fn rank_rejects_malformed_original_cohorts_and_bounds_error_messages() {
        let (_root, mut client, docs) = rank_fixture();
        let intent = client
            .capture_navigation_rank(Kind::Definition, &docs, &[], 0)
            .unwrap();
        let p = &intent.source.providers[0];
        let foreign = client
            .providers
            .registry
            .entries()
            .iter()
            .find(|p| p.kind == Kind::Hover)
            .unwrap();
        let invalid = [
            json!({"providers":[{"id":p.id,"owner":p.owner},{"id":p.id,"owner":p.owner}]}),
            json!({"providers":[{"id":p.id,"owner":"different.owner"}]}),
            json!({"providers":[{"id":foreign.id,"owner":foreign.owner}]}),
            json!({"providers":[{"id":p.id,"owner":p.owner,"extra":true}]}),
            json!({"providers":[],"extra":true}),
        ];
        for value in invalid {
            let id = client.request_navigation_rank(&intent, &docs[0]).unwrap();
            let mut frame = rank_frame(&mut client, id);
            frame["result"] = value;
            client.provider_rank_response(id, frame).unwrap();
            assert!(
                !client.navigation_rank_available(),
                "Error retains bounded result storage until consumed"
            );
            assert!(
                client
                    .take_navigation_ranks()
                    .pop_front()
                    .unwrap()
                    .result
                    .is_err()
            );
            assert!(client.navigation_rank_available());
            assert_eq!(client.language_provider_slots(), 8);
        }
        let id = client.request_navigation_rank(&intent, &docs[0]).unwrap();
        let mut frame = rank_frame(&mut client, id);
        frame["error"] = json!({"message":"猫".repeat(100_000)});
        client.provider_rank_response(id, frame).unwrap();
        let error = client
            .take_navigation_ranks()
            .pop_front()
            .unwrap()
            .result
            .unwrap_err();
        assert_eq!(error.chars().count(), 2048);
        assert!(client.navigation_rank_available());
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
        assert!(!docs[0].dirty());
    }
    #[test]
    fn held_rank_original_source_cannot_revive_after_edit_undo() {
        let (_root, mut client, mut docs) = rank_fixture();
        let intent = client
            .capture_navigation_rank(Kind::Definition, &docs, &[], 0)
            .unwrap();
        let id = client.request_navigation_rank(&intent, &docs[0]).unwrap();
        let original = (docs[0].id, docs[0].revision, docs[0].text.to_string());
        docs[0].insert("x", false);
        docs[0].undo();
        assert_eq!(
            (docs[0].id, docs[0].revision, docs[0].text.to_string()),
            original
        );
        assert!(!client.navigation_rank_current(&intent, &docs[0]));
        client.sync_with_hidden(&docs, &[], 0).unwrap();
        assert!(!client.navigation_rank_current(&intent, &docs[0]));
        client.cancel_navigation_rank(id);
        assert!(!client.navigation_rank_available());
        let frame = rank_frame(&mut client, id);
        client.provider_rank_response(id, frame).unwrap();
        assert!(client.take_navigation_ranks().is_empty());
        assert!(client.request_navigation_rank(&intent, &docs[0]).is_err());
        let fresh = client
            .capture_navigation_rank(Kind::Definition, &docs, &[], 0)
            .unwrap();
        let next = client.request_navigation_rank(&fresh, &docs[0]).unwrap();
        let frame = rank_frame(&mut client, next);
        client.provider_rank_response(next, frame).unwrap();
        assert!(
            client
                .take_navigation_ranks()
                .pop_front()
                .unwrap()
                .result
                .is_ok()
        );
        docs[0].redo();
        assert_eq!(docs[0].text.to_string(), "x猫🙂\r\n");
        assert!(docs[0].dirty());
        assert_eq!(client.language_provider_slots(), 8);
    }
    #[test]
    fn generic_actual_callbacks_remain_occupied_after_cancel_timeout_and_wrong_release() {
        let root = tempfile::tempdir().unwrap();
        let body = r#"const v=require('vscode'),fs=require('node:fs'),path=require('node:path');
let held=[],entered=0;
exports.activate=c=>{c.subscriptions.push(v.languages.registerHoverProvider('*',{
 provideHover(){fs.writeFileSync(path.join(v.workspace.rootPath,'entered'),String(++entered));return new Promise(resolve=>held.push(resolve));}
}));c.subscriptions.push(v.commands.registerCommand('test.release',()=>{for(const resolve of held.splice(0))resolve(undefined);}));};"#;
        let package = fixture(root.path(), body);
        let mut docs = [Document::from_text("α🙂\r\n")];
        let mut client = Client::start(
            "node",
            &package,
            root.path(),
            &docs,
            0,
            &Settings::default(),
        )
        .unwrap();
        ready(&mut client, &mut docs);
        let mut tickets = Vec::new();
        for _ in 0..8 {
            tickets.push(
                client
                    .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
                    .unwrap()
                    .unwrap(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::fs::read_to_string(root.path().join("entered"))
            .ok()
            .as_deref()
            != Some("8")
        {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline, "Actual callbacks never entered");
            std::thread::yield_now();
        }
        client
            .providers
            .calls
            .get_mut(&tickets[0].id)
            .unwrap()
            .started = Instant::now() - DEADLINE;
        client.expire_language_providers().unwrap();
        assert_eq!(client.providers.occupied.len(), 8);
        assert!(
            client
                .take_provider_replies()
                .pop_front()
                .unwrap()
                .result
                .unwrap_err()
                .contains("timed out")
        );
        for ticket in &tickets {
            client.cancel_language_provider(ticket).unwrap();
        }
        while !client.providers.calls.is_empty() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(
                Instant::now() < deadline,
                "Canceled logical replies never arrived"
            );
            std::thread::yield_now();
        }
        client.take_provider_replies();
        assert_eq!(client.providers.occupied.len(), 8);
        assert_eq!(client.language_provider_slots(), 0);
        let ticket = &tickets[0];
        let release = json!({"session":client.session,"owner":ticket.provider.owner,"provider":ticket.provider.id,"request":ticket.id});
        for field in ["owner", "provider"] {
            let mut wrong = release.clone();
            wrong[field] = if field == "owner" {
                json!("wrong.owner")
            } else {
                json!(ticket.provider.id + 1)
            };
            assert!(client.language_provider_released(wrong).is_err());
            assert_eq!(client.language_provider_slots(), 0);
        }
        let mut wrong = release.clone();
        wrong["extra"] = true.into();
        assert!(client.language_provider_released(wrong).is_err());
        let mut old = release.clone();
        old["session"] = json!(client.session + 1);
        client.language_provider_released(old).unwrap();
        assert_eq!(client.language_provider_slots(), 0);
        // The real held callbacks have not returned. A provider registry reset
        // also cannot acquire their actual slots.
        client.register_language_providers(json!([])).unwrap();
        assert_eq!(client.language_provider_slots(), 0);
        client
            .execute("test.release", None, &docs, 0, &Settings::default())
            .unwrap();
        while !client.providers.occupied.is_empty() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(
                Instant::now() < deadline,
                "Actual release frames never arrived"
            );
            std::thread::yield_now();
        }
        assert_eq!(client.language_provider_slots(), 8);
        client.language_provider_released(release).unwrap();
        assert_eq!(
            client.language_provider_slots(),
            8,
            "Duplicate cannot release another callback"
        );
        assert_eq!(docs[0].text.to_string(), "α🙂\r\n");
        assert!(!docs[0].dirty());
    }
    #[test]
    fn actual_release_before_response_does_not_release_awaiting_reply_capacity() {
        let root = tempfile::tempdir().unwrap();
        let package = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerHoverProvider('*',{provideHover:()=>new v.Hover('real framed result')});"#,
        );
        let mut docs = [Document::from_text("α🙂\r\n")];
        let mut client = Client::start(
            "node",
            &package,
            root.path(),
            &docs,
            0,
            &Settings::default(),
        )
        .unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        let mut release = None;
        let mut response = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while release.is_none() || response.is_none() {
            if let Some(mut frame) = client.process.receive().unwrap() {
                if frame["method"] == "languageProviderReleased"
                    && frame["params"]["request"].as_u64() == Some(ticket.id)
                {
                    release = Some(frame["params"].take());
                } else if frame["id"].as_u64() == Some(ticket.id) {
                    response = Some(frame);
                }
            }
            assert!(Instant::now() < deadline, "Real callback did not settle");
            std::thread::yield_now();
        }
        let release = release.unwrap();
        client.language_provider_released(release.clone()).unwrap();
        assert_eq!(
            client.language_provider_slots(),
            7,
            "Original logical reply still retained"
        );
        client
            .provider_response(ticket.id, response.unwrap())
            .unwrap();
        assert_eq!(
            client.language_provider_slots(),
            7,
            "Unconsumed reply retains its bounded storage"
        );
        let actual = client
            .take_provider_replies()
            .pop_front()
            .unwrap()
            .result
            .unwrap();
        assert_eq!(actual["contents"][0], "real framed result");
        client.pending.remove(&ticket.id);
        assert_eq!(client.language_provider_slots(), 8);
        client.language_provider_released(release).unwrap();
        assert_eq!(client.language_provider_slots(), 8);
    }
    #[test]
    fn held_provider_ticket_refuses_language_metadata_aba_without_text_version_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.cpp");
        std::fs::write(&path, "猫🙂 original\r\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.insert("dirty ", false);
        let id = doc.id;
        let revision = doc.revision;
        let epoch = doc.text_epoch();
        let package = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerHoverProvider('*',{provideHover:()=>new Promise(()=>{})});"#,
        );
        let mut docs = vec![doc];
        let mut client = Client::start(
            "node",
            &package,
            root.path(),
            &docs,
            0,
            &Settings::default(),
        )
        .unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        assert!(client.provider_ticket_current(&ticket, &docs[0]));
        docs[0]
            .set_language_override(Some("python".into()))
            .unwrap();
        assert!(!client.provider_ticket_current(&ticket, &docs[0]));
        docs[0].set_language_override(Some("cpp".into())).unwrap();
        client.sync_with_hidden(&docs, &[], 0).unwrap();
        assert!(!client.provider_ticket_current(&ticket, &docs[0]));
        assert_eq!(client.mirror.mirrors[&id].version, ticket.version);
        assert_eq!(
            (docs[0].id, docs[0].revision, docs[0].text_epoch()),
            (id, revision, epoch)
        );
        docs[0].undo();
        assert_eq!(docs[0].text.to_string(), "猫🙂 original\r\n");
        docs[0].redo();
        assert_eq!(docs[0].text.to_string(), "dirty 猫🙂 original\r\n");
        assert_eq!(std::fs::read(path).unwrap(), "猫🙂 original\r\n".as_bytes());
    }
    #[test]
    fn symbols_wire_depth_preserves_sixteen_levels_without_widening_other_kinds() {
        let range = json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}});
        let mut symbol =
            json!({"name":"leaf","kind":1,"range":range,"selectionRange":range,"children":[]});
        for _ in 0..16 {
            symbol = json!({"name":"parent","kind":1,"range":range,"selectionRange":range,"children":[symbol]});
        }
        let value = json!([symbol]);
        assert!(provider_result_budget(&value).is_err());
        result_budget(&value, 40).unwrap();
        let mut too_deep = json!(null);
        for _ in 0..41 {
            too_deep = json!([too_deep]);
        }
        assert!(result_budget(&too_deep, 40).is_err());
    }
    #[test]
    fn symbols_canceled_wire_keeps_actual_slot_until_exact_framed_settlement() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"
const v=require('vscode'),fs=require('fs'),p=require('path');exports.activate=()=>{
v.languages.registerDocumentSymbolProvider('*',{provideDocumentSymbols(){fs.writeFileSync(p.join(v.workspace.rootPath,'symbols-entered'),'yes');return new Promise(resolve=>{const timer=setInterval(()=>{if(fs.existsSync(p.join(v.workspace.rootPath,'symbols-release'))){clearInterval(timer);resolve([]);}},5);});}});
v.languages.registerSignatureHelpProvider('*',{provideSignatureHelp:()=>null},'(');
};"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let provider = client
            .language_provider(Kind::Symbols, &docs[0])
            .unwrap()
            .clone();
        assert!(client.symbol_source_identity(&provider).is_some());
        let ticket = client
            .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.path().join("symbols-entered").exists() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        client.cancel_language_provider(&ticket).unwrap();
        assert!(
            signature_reply(&mut client, &mut docs, ticket.id)
                .result
                .unwrap_err()
                .contains("canceled")
        );
        assert!(client.providers.calls.is_empty());
        assert!(!client.symbols_available());
        for _ in 0..32 {
            assert!(
                client
                    .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
                    .is_err()
            );
        }
        let signature = client
            .request_language_provider(Kind::Signature, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        assert!(
            signature_reply(&mut client, &mut docs, signature.id)
                .result
                .unwrap()
                .is_null()
        );
        assert!(!client.symbols_available());
        client.providers.symbols_started = Some(Instant::now() - DEADLINE);
        client.expire_language_providers().unwrap();
        assert!(client.symbol_channel_closed());
        std::fs::write(root.path().join("symbols-release"), "yes").unwrap();
        while !client.symbols_available() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!client.symbol_channel_closed());
        assert!(client.providers.symbols.is_none());
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
    }
    #[test]
    fn symbols_timeout_and_early_ack_validate_exact_owner_session_request_and_provider() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerDocumentSymbolProvider('*',{provideDocumentSymbols:()=>new Promise(()=>{})});"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        client.providers.calls.get_mut(&ticket.id).unwrap().started = Instant::now() - DEADLINE;
        client.expire_language_providers().unwrap();
        assert!(
            client
                .take_provider_replies()
                .pop_front()
                .unwrap()
                .result
                .is_err()
        );
        assert!(client.symbol_channel_closed());
        let ack = json!({"session":ticket.session,"owner":ticket.provider.owner,"provider":ticket.provider.id,"request":ticket.id});
        for field in ["session", "request"] {
            let mut foreign = ack.clone();
            foreign[field] = json!(ack[field].as_u64().unwrap() + 1);
            client.symbols_released(foreign).unwrap();
            assert!(client.symbol_channel_closed());
        }
        let mut foreign = ack.clone();
        foreign["owner"] = json!("other.owner");
        assert!(client.symbols_released(foreign).is_err());
        let mut foreign = ack.clone();
        foreign["provider"] = json!(ticket.provider.id + 1);
        assert!(client.symbols_released(foreign).is_err());
        assert!(!client.symbols_available());
        client.symbols_released(ack).unwrap();
        assert!(client.symbols_available());
        assert!(!client.symbol_channel_closed());
        client
            .provider_response(ticket.id, json!({"result":[]}))
            .unwrap();
        assert!(client.take_provider_replies().is_empty());
        // A positive ack before its wire reply must not cause a false fence.
        let next = client
            .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        client.symbols_released(json!({"session":next.session,"owner":next.provider.owner,"provider":next.provider.id,"request":next.id})).unwrap();
        assert!(
            !client.symbols_available(),
            "The wire reply remains outstanding"
        );
        client.providers.calls.get_mut(&next.id).unwrap().started = Instant::now() - DEADLINE;
        client.expire_language_providers().unwrap();
        assert!(!client.symbol_channel_closed());
        assert!(
            client
                .take_provider_replies()
                .pop_front()
                .unwrap()
                .result
                .is_err()
        );
        assert!(client.symbols_available());
    }
    #[test]
    fn symbols_source_a_b_a_and_edit_undo_never_revive_old_ticket_proofs() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerDocumentSymbolProvider('*',{provideDocumentSymbols:()=>new Promise(()=>{})});"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let provider = client
            .language_provider(Kind::Symbols, &docs[0])
            .unwrap()
            .clone();
        let identity = client.symbol_source_identity(&provider).unwrap();
        let ticket = client
            .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        docs[0].insert("transient", false);
        docs[0].undo();
        assert_eq!(docs[0].revision, ticket.revision);
        assert!(!client.provider_ticket_current(&ticket, &docs[0]));
        client.sync_with_hidden(&docs, &[], 0).unwrap();
        let mut forged = ticket.clone();
        forged.version += 1;
        assert!(!client.provider_ticket_current(&forged, &docs[0]));
        client.register_language_providers(json!([])).unwrap();
        assert!(client.symbol_source_identity(&provider).is_none());
        client
            .register_language_providers(json!([provider]))
            .unwrap();
        assert_ne!(
            client.symbol_source_identity(&ticket.provider).unwrap(),
            identity
        );
        assert!(!client.provider_registration_current(&ticket));
        assert!(
            !client.symbols_available(),
            "Registry retirement cannot settle actual work"
        );
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
    }
    #[test]
    fn symbols_global_capacity_and_untitled_flat_and_hierarchical_shapes_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"
const v=require('vscode');exports.activate=()=>{
v.languages.registerDocumentSymbolProvider('*',{provideDocumentSymbols(d){const r=new v.Range(0,0,0,3),s=new v.Range(0,1,0,3),parent=new v.DocumentSymbol('parent','detail',v.SymbolKind.Class,r,s);parent.children=[new v.DocumentSymbol('child','',v.SymbolKind.Method,s,s)];return [parent,new v.SymbolInformation('flat',v.SymbolKind.Function,'container',s,d.uri)];}});
v.languages.registerHoverProvider('*',{provideHover:()=>new Promise(()=>{})});
};"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        let value = signature_reply(&mut client, &mut docs, ticket.id)
            .result
            .unwrap();
        assert_eq!(value[0]["range"]["start"]["character"], 0);
        assert_eq!(value[0]["selectionRange"]["start"]["character"], 1);
        assert_eq!(value[0]["children"][0]["name"], "child");
        assert_eq!(
            value[1]["location"]["uri"],
            client.mirror.mirrors[&docs[0].id].uri
        );
        assert!(value[1].get("children").is_none());
        assert!(client.symbols_available());
        for _ in 0..LIMIT {
            client
                .request_language_provider(Kind::Hover, &docs, &[], 0, json!({}))
                .unwrap()
                .unwrap();
        }
        assert!(!client.symbols_available());
        assert!(
            client
                .request_language_provider(Kind::Symbols, &docs, &[], 0, json!({}))
                .is_err()
        );
        assert!(client.providers.symbols.is_none());
        assert_eq!(client.providers.calls.len(), LIMIT);
    }
    #[test]
    fn signature_original_help_survives_typing_versions_and_explicit_release_retires_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"
const v=require('vscode');let original;exports.activate=()=>v.languages.registerSignatureHelpProvider('*',{
provideSignatureHelp(document,position,token,context){
 if(context.activeSignatureHelp){if(context.activeSignatureHelp!==original||original.opaque.callback()!==42||original.activeSignature!==1||original.activeParameter!==1)throw Error('Original identity or selected indices lost');}
 else {original=new v.SignatureHelp();const a=new v.SignatureInformation('f(猫,🙂)');a.parameters=[new v.ParameterInformation([2,3]),new v.ParameterInformation([4,6])];const b=new v.SignatureInformation('f(left,right)');b.parameters=[new v.ParameterInformation('left'),new v.ParameterInformation('right')];original.signatures=[a,b];original.opaque={callback:()=>42};original.opaque.self=original;}
 return original;
}}, {triggerCharacters:['('],retriggerCharacters:[',']});
"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let provider = client
            .language_provider(Kind::Signature, &docs[0])
            .unwrap()
            .clone();
        assert_eq!(provider.triggers, ["("]);
        assert_eq!(provider.retriggers, [","]);
        assert!(client.signature_source_identity(&provider).is_some());
        let first = client
            .request_language_provider_from(
                &provider,
                &docs,
                &[],
                0,
                json!({"context":{"triggerKind":2,"triggerCharacter":"(","isRetrigger":false}}),
            )
            .unwrap();
        assert!(!client.signature_available());
        let mut help = signature_reply(&mut client, &mut docs, first.id)
            .result
            .unwrap();
        assert!(client.signature_available());
        let first_handle = help["_vscliSignatureHelpHandle"].as_u64().unwrap();
        assert!(help.get("opaque").is_none());
        docs[0].insert("x", false);
        assert!(!client.provider_ticket_current(&first, &docs[0]));
        help["activeSignature"] = json!(1);
        help["activeParameter"] = json!(1);
        let second = client
            .request_language_provider_from(
                &provider,
                &docs,
                &[],
                0,
                json!({"context":{"triggerKind":3,"isRetrigger":true,"activeSignatureHelp":help}}),
            )
            .unwrap();
        let help = signature_reply(&mut client, &mut docs, second.id)
            .result
            .unwrap();
        assert_ne!(
            help["_vscliSignatureHelpHandle"].as_u64().unwrap(),
            first_handle
        );
        client.release_signature_help(&second, &help).unwrap();
        let stale = client
            .request_language_provider_from(
                &provider,
                &docs,
                &[],
                0,
                json!({"context":{"triggerKind":3,"isRetrigger":true,"activeSignatureHelp":help}}),
            )
            .unwrap();
        assert!(
            signature_reply(&mut client, &mut docs, stale.id)
                .result
                .unwrap_err()
                .contains("Stale signature help")
        );
        assert!(client.signature_available());
        assert_eq!(docs[0].text.to_string(), "x猫🙂\r\n");
    }
    #[test]
    fn canceled_signature_wire_reply_keeps_actual_capacity_until_framed_release_ack() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"
const v=require('vscode'),fs=require('fs'),p=require('path');exports.activate=()=>v.languages.registerSignatureHelpProvider('*',{
provideSignatureHelp(){fs.writeFileSync(p.join(v.workspace.rootPath,'signature-entered'),'yes');return new Promise(resolve=>{const timer=setInterval(()=>{if(fs.existsSync(p.join(v.workspace.rootPath,'signature-release'))){clearInterval(timer);resolve(null);}},5);});}
},'(');
"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Signature, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.path().join("signature-entered").exists() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        client.cancel_language_provider(&ticket).unwrap();
        assert!(
            signature_reply(&mut client, &mut docs, ticket.id)
                .result
                .unwrap_err()
                .contains("canceled")
        );
        assert!(
            client.providers.calls.is_empty(),
            "The wire error precedes actual settlement"
        );
        assert!(!client.signature_available());
        assert!(!client.signature_channel_closed());
        for _ in 0..32 {
            assert!(
                client
                    .request_language_provider(Kind::Signature, &docs, &[], 0, json!({}))
                    .is_err()
            );
        }
        client.providers.signature_started = Some(Instant::now() - DEADLINE);
        client.expire_language_providers().unwrap();
        assert!(
            client.signature_channel_closed(),
            "Canceled early replies retain an independent actual-work deadline"
        );
        std::fs::write(root.path().join("signature-release"), "yes").unwrap();
        while !client.signature_available() {
            client.poll(&mut docs, 0, &Settings::default()).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(client.providers.signature.is_none());
        assert!(!client.signature_channel_closed());
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
    }
    #[test]
    fn signature_timeout_fences_until_matching_actual_release_and_ignores_foreign_ack() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerSignatureHelpProvider('*',{provideSignatureHelp:()=>new Promise(()=>{})},'(');"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Signature, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        client.providers.calls.get_mut(&ticket.id).unwrap().started = Instant::now() - DEADLINE;
        client.expire_language_providers().unwrap();
        assert!(client.signature_channel_closed());
        assert!(!client.signature_available());
        assert!(
            client
                .take_provider_replies()
                .pop_front()
                .unwrap()
                .result
                .is_err()
        );
        let ack = json!({"session":ticket.session,"owner":ticket.provider.owner,"provider":ticket.provider.id,"request":ticket.id});
        let mut foreign = ack.clone();
        foreign["session"] = json!(ticket.session + 1);
        client.signature_released(foreign).unwrap();
        let mut foreign = ack.clone();
        foreign["request"] = json!(ticket.id + 1);
        client.signature_released(foreign).unwrap();
        let mut foreign = ack.clone();
        foreign["owner"] = json!("other.owner");
        assert!(client.signature_released(foreign).is_err());
        assert!(client.signature_channel_closed());
        client.signature_released(ack).unwrap();
        assert!(!client.signature_channel_closed());
        assert!(client.signature_available());
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
    }
    #[test]
    fn signature_positive_release_before_late_wire_reply_prevents_false_timeout_fence() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(
            root.path(),
            r#"const v=require('vscode');exports.activate=()=>v.languages.registerSignatureHelpProvider('*',{provideSignatureHelp:()=>new Promise(()=>{})},'(');"#,
        );
        let mut docs = vec![Document::from_text("猫🙂\r\n")];
        let mut client =
            Client::start("node", &path, root.path(), &docs, 0, &Settings::default()).unwrap();
        ready(&mut client, &mut docs);
        let ticket = client
            .request_language_provider(Kind::Signature, &docs, &[], 0, json!({}))
            .unwrap()
            .unwrap();
        client.signature_released(json!({"session":ticket.session,"owner":ticket.provider.owner,"provider":ticket.provider.id,"request":ticket.id})).unwrap();
        assert!(
            !client.signature_available(),
            "A success wire reply is still pending"
        );
        client.providers.calls.get_mut(&ticket.id).unwrap().started = Instant::now() - DEADLINE;
        client.expire_language_providers().unwrap();
        assert!(
            !client.signature_channel_closed(),
            "The callback already supplied positive settlement proof"
        );
        assert!(
            client
                .take_provider_replies()
                .pop_front()
                .unwrap()
                .result
                .is_err()
        );
        assert!(client.signature_available());
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
        Arc::make_mut(&mut forged.provider).owner = "other.owner".into();
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
