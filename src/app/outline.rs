//! Current-model Outline. Publication owns metadata, never document content.
use super::*;
use crate::{extension_providers::Kind, extensions::providers::Ticket, outline::Tree};
use anyhow::{Context as _, bail};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

const DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineStatus {
    Hidden,
    NoDocument,
    Unsupported,
    Loading,
    Updating,
    Ready,
    Error,
}

pub(super) struct DocumentSymbolsView<'a> {
    pub tree: Option<&'a Tree>,
    pub active: Option<usize>,
    pub generation: u64,
    pub current: bool,
    pub status: OutlineStatus,
}

pub struct OutlineView<'a> {
    pub status: OutlineStatus,
    pub tree: Option<&'a Tree>,
    pub visible: &'a [usize],
    pub selected: Option<usize>,
    pub active: Option<usize>,
    pub actionable: bool,
    pub follow_cursor: bool,
    pub message: &'a str,
    pub source: &'a str,
}

#[derive(Clone)]
enum Source {
    Native(Arc<()>),
    Extension {
        session: u64,
        epoch: u64,
        provider: u64,
    },
    Unsupported,
}
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(left), Self::Native(right)) => Arc::ptr_eq(left, right),
            (
                Self::Extension {
                    session: a,
                    epoch: b,
                    provider: c,
                },
                Self::Extension {
                    session: x,
                    epoch: y,
                    provider: z,
                },
            ) => (a, b, c) == (x, y, z),
            (Self::Unsupported, Self::Unsupported) => true,
            _ => false,
        }
    }
}
#[derive(Clone, PartialEq)]
struct Stamp {
    document: u64,
    revision: u64,
    epoch: u64,
    saved: u64,
    path: Option<PathBuf>,
    workspace: PathBuf,
    source: Source,
}
impl Stamp {
    fn capture(doc: &Document, source: Source, workspace: &Path) -> Self {
        Self {
            document: doc.id,
            revision: doc.revision,
            epoch: doc.text_epoch(),
            saved: doc.save_generation(),
            path: doc.path.clone(),
            workspace: workspace.into(),
            source,
        }
    }
    fn matches(&self, doc: &Document, source: &Source, workspace: &Path) -> bool {
        self.document == doc.id
            && self.revision == doc.revision
            && self.epoch == doc.text_epoch()
            && self.saved == doc.save_generation()
            && self.path == doc.path
            && self.source == *source
            && self.workspace == workspace
    }
    fn same_resource(&self, other: &Self) -> bool {
        self.document == other.document
            && self.path == other.path
            && self.source == other.source
            && self.workspace == other.workspace
    }
    fn uri(&self) -> Result<String> {
        self.path.as_ref().map_or_else(
            || Ok(format!("untitled:vscli-{}", self.document)),
            |path| crate::lsp::file_uri(path),
        )
    }
}
enum Call {
    Native { token: u64, instance: Arc<()> },
    Extension(Ticket),
}
struct Pending {
    stamp: Stamp,
    generation: u64,
    call: Call,
}
enum Proof {
    Native(crate::lsp::Request),
    Extension(Ticket),
}
struct Published {
    stamp: Stamp,
    proof: Proof,
    tree: Tree,
    ranges: Vec<(usize, usize)>,
    identities: Vec<[u8; 32]>,
}
pub(super) struct State {
    enabled: bool,
    follow_cursor: bool,
    current: Option<Stamp>,
    published: Option<Published>,
    pending: Option<Pending>,
    wanted: Option<Instant>,
    generation: u64,
    publication: u64,
    exhausted: bool,
    status: OutlineStatus,
    message: String,
    source: String,
    visible: Vec<usize>,
    collapsed: HashSet<[u8; 32]>,
    selected: Option<usize>,
    active: Option<usize>,
    cursor: Option<(u64, u64, usize, u64)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: false,
            follow_cursor: true,
            current: None,
            published: None,
            pending: None,
            wanted: None,
            generation: 0,
            publication: 0,
            exhausted: false,
            status: OutlineStatus::Hidden,
            message: String::new(),
            source: String::new(),
            visible: Vec::new(),
            collapsed: HashSet::new(),
            selected: None,
            active: None,
            cursor: None,
        }
    }
}

fn check_budget(app: &App) -> Result<()> {
    super::extension_services::Context::check_budget(app)?;
    if app.workspace.root.as_os_str().len() > 4096
        || app
            .documents
            .iter()
            .chain(&app.hidden_documents)
            .filter_map(|doc| doc.path.as_ref())
            .any(|path| path.as_os_str().len() > 4096)
    {
        bail!("Outline supports workspace and document paths up to 4 KiB");
    }
    Ok(())
}

fn identities(tree: &Tree) -> Vec<[u8; 32]> {
    let mut output: Vec<[u8; 32]> = Vec::with_capacity(tree.nodes.len());
    let mut counts = HashMap::<(Option<usize>, [u8; 32]), u32>::new();
    for node in &tree.nodes {
        let mut hash = Sha256::new();
        if let Some(parent) = node.parent {
            hash.update(output[parent]);
        }
        hash.update([node.kind]);
        hash.update((node.name.len() as u64).to_le_bytes());
        hash.update(node.name.as_bytes());
        hash.update((node.container.len() as u64).to_le_bytes());
        hash.update(node.container.as_bytes());
        let base: [u8; 32] = hash.finalize().into();
        let ordinal = counts.entry((node.parent, base)).or_default();
        let mut hash = Sha256::new();
        hash.update(base);
        hash.update(ordinal.to_le_bytes());
        *ordinal += 1;
        output.push(hash.finalize().into());
    }
    output
}

impl App {
    fn outline_source(&self) -> Source {
        let Some(doc) = self.active_document() else {
            return Source::Unsupported;
        };
        if let Some(host) = &self.extension_host
            && let Some(provider) = host.language_provider(Kind::Symbols, doc)
            && let Some((session, epoch)) = host.symbol_source_identity(provider)
        {
            return Source::Extension {
                session,
                epoch,
                provider: provider.id,
            };
        }
        if let Some(path) = &doc.path
            && let Some(client) = &self.lsp
        {
            let language = crate::lsp::language(path);
            let capability = &client.capabilities["documentSymbolProvider"];
            if client.ready
                && (capability == true || capability.is_object())
                && (client.language == language || client.language == "cpp" && language == "c")
            {
                return Source::Native(client.identity());
            }
        }
        Source::Unsupported
    }
    fn outline_stamp_current(&self, stamp: &Stamp) -> bool {
        self.active_document()
            .is_some_and(|doc| stamp.matches(doc, &self.outline_source(), &self.workspace.root))
    }
    fn outline_published_current(&self) -> bool {
        self.outline.published.as_ref().is_some_and(|published| {
            self.outline_stamp_current(&published.stamp)
                && self.outline_proof_current(&published.proof)
        })
    }
    fn outline_proof_current(&self, proof: &Proof) -> bool {
        match proof {
            Proof::Native(request) => self
                .lsp
                .as_ref()
                .is_some_and(|client| client.request_current(request)),
            Proof::Extension(ticket) => self.active_document().is_some_and(|doc| {
                self.extension_host
                    .as_ref()
                    .is_some_and(|host| host.provider_ticket_current(ticket, doc))
            }),
        }
    }
    pub(super) fn document_symbols_view(&self) -> DocumentSymbolsView<'_> {
        DocumentSymbolsView {
            tree: self.outline.published.as_ref().map(|p| &p.tree),
            active: self.outline.active,
            generation: self.outline.publication,
            status: self.outline.status,
            current: self.outline.status == OutlineStatus::Ready
                && self.outline_published_current(),
        }
    }
    fn document_symbols_demand(&self) -> bool {
        (self.outline.enabled && self.sidebar) || self.breadcrumbs_demand()
    }
    pub fn outline_view(&self) -> OutlineView<'_> {
        OutlineView {
            status: if self.outline.enabled {
                self.outline.status
            } else {
                OutlineStatus::Hidden
            },
            tree: self.outline.published.as_ref().map(|p| &p.tree),
            visible: &self.outline.visible,
            selected: self.outline.selected,
            active: self.outline.active,
            actionable: self.outline.status == OutlineStatus::Ready
                && self.outline_published_current(),
            follow_cursor: self.outline.follow_cursor,
            message: &self.outline.message,
            source: &self.outline.source,
        }
    }
    fn cancel_outline_pending(&mut self) {
        let Some(pending) = self.outline.pending.take() else {
            return;
        };
        match pending.call {
            Call::Native { token, instance } => {
                if let Some(client) = &mut self.lsp
                    && Arc::ptr_eq(&instance, &client.identity())
                {
                    let _ = client.cancel_symbol_request(token);
                }
            }
            Call::Extension(ticket) => {
                if let Some(host) = &mut self.extension_host
                    && host.session == ticket.session
                {
                    let _ = host.cancel_language_provider(&ticket);
                }
            }
        }
    }
    fn clear_outline_tree(&mut self) {
        self.outline.published = None;
        self.outline.visible.clear();
        self.outline.collapsed.clear();
        self.outline.selected = None;
        self.outline.active = None;
        self.outline.cursor = None;
    }
    pub(super) fn observe_outline(&mut self) -> bool {
        if self.outline.exhausted {
            return false;
        }
        let source = self.outline_source();
        let Some(doc) = self.active_document() else {
            if self.outline.status == OutlineStatus::NoDocument
                && self.outline.current.is_none()
                && self.outline.pending.is_none()
                && self.outline.wanted.is_none()
                && self.outline.published.is_none()
            {
                return false;
            }
            let changed = self.outline.status != OutlineStatus::NoDocument;
            self.cancel_outline_pending();
            self.clear_outline_tree();
            self.outline.current = None;
            self.outline.wanted = None;
            self.outline.status = OutlineStatus::NoDocument;
            self.outline.message = "Open a document to view its symbols".into();
            self.outline.source.clear();
            return changed;
        };
        let retired_proof = self.outline.status != OutlineStatus::Error
            && self.outline.pending.is_none()
            && self.outline.wanted.is_none()
            && self.outline.published.as_ref().is_some_and(|published| {
                published.stamp.matches(doc, &source, &self.workspace.root)
                    && !self.outline_proof_current(&published.proof)
            });
        let changed = retired_proof
            || self
                .outline
                .current
                .as_ref()
                .is_none_or(|stamp| !stamp.matches(doc, &source, &self.workspace.root));
        if changed {
            if let Err(error) = check_budget(self) {
                let message = format!("{error:#}");
                let notify =
                    self.outline.status != OutlineStatus::Error || self.outline.message != message;
                self.cancel_outline_pending();
                self.clear_outline_tree();
                self.outline.wanted = None;
                self.outline_error(&message);
                return notify;
            }
            let stamp = Stamp::capture(doc, source, &self.workspace.root);
            self.cancel_outline_pending();
            if self
                .outline
                .published
                .as_ref()
                .is_some_and(|p| !p.stamp.same_resource(&stamp))
            {
                self.clear_outline_tree();
            }
            let Some(generation) = self.outline.generation.checked_add(1) else {
                self.outline.exhausted = true;
                self.outline.wanted = None;
                self.outline.status = OutlineStatus::Error;
                self.outline.message =
                    "Outline generation limit reached; restart the editor".into();
                return true;
            };
            self.outline.generation = generation;
            self.outline.source = match &stamp.source {
                Source::Native(_) => self
                    .lsp
                    .as_ref()
                    .map_or_else(String::new, |c| format!("{} · native", c.language)),
                Source::Extension { .. } => self
                    .extension_host
                    .as_ref()
                    .and_then(|h| h.language_provider(Kind::Symbols, self.doc()))
                    .map_or_else(String::new, |p| format!("{} · extension", p.owner)),
                Source::Unsupported => String::new(),
            };
            self.outline.status = if matches!(stamp.source, Source::Unsupported) {
                OutlineStatus::Unsupported
            } else if self.outline.published.is_some() {
                OutlineStatus::Updating
            } else {
                OutlineStatus::Loading
            };
            self.outline.message = if self.outline.status == OutlineStatus::Unsupported {
                "No document-symbol provider for this file".into()
            } else {
                String::new()
            };
            self.outline.wanted =
                (!matches!(stamp.source, Source::Unsupported)).then(|| Instant::now() + DEBOUNCE);
            self.outline.current = Some(stamp);
            self.outline.cursor = None;
        }
        if !self.document_symbols_demand()
            || matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Symbols)
            )
        {
            if self.outline.pending.is_some() {
                self.cancel_outline_pending();
                self.outline.wanted = Some(Instant::now() + DEBOUNCE);
            }
            return changed;
        }
        self.update_outline_active() || changed
    }
    pub(super) fn poll_outline(&mut self) -> bool {
        let changed = self.observe_outline();
        if !self.document_symbols_demand()
            || self.outline.exhausted
            || self.outline.pending.is_some()
            || self.outline.wanted.is_none_or(|at| Instant::now() < at)
            || matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Symbols)
            )
        {
            return changed;
        }
        let Some(current) = self.outline.current.as_ref() else {
            return changed;
        };
        if !self.outline_stamp_current(current) {
            return changed;
        }
        // Capacity and ownership checks precede all path/stamp cloning. A
        // canceled actual callback can hold this lane for many input frames.
        match &current.source {
            Source::Native(_) => {
                if self.native_symbol_picker_waiting() {
                    return changed;
                }
                let Some(client) = &self.lsp else {
                    return changed;
                };
                if !client.symbol_available() {
                    if client.symbol_channel_closed() {
                        let notify = self.outline.status != OutlineStatus::Error;
                        if notify {
                            self.outline_error("Symbol callback timed out; awaiting actual settlement or server restart");
                        }
                        return notify || changed;
                    }
                    return changed;
                }
            }
            Source::Extension { .. } => {
                let Some(host) = &self.extension_host else {
                    return changed;
                };
                if !host.symbols_available() {
                    if host.symbol_channel_closed() {
                        let notify = self.outline.status != OutlineStatus::Error;
                        if notify {
                            self.outline_error("Extension symbol callback timed out; awaiting actual settlement or host restart");
                        }
                        return notify || changed;
                    }
                    return changed;
                }
            }
            Source::Unsupported => return changed,
        }
        if let Err(error) = check_budget(self) {
            self.outline.wanted = None;
            self.outline_error(&format!("{error:#}"));
            return true;
        }
        let stamp = self.outline.current.as_ref().unwrap().clone();
        let call = match &stamp.source {
            Source::Native(instance) => {
                let client = self.lsp.as_mut().unwrap();
                client
                    .sync(&self.documents)
                    .and_then(|()| client.request_document_symbols(&self.documents[self.active]))
                    .map(|token| Call::Native {
                        token,
                        instance: instance.clone(),
                    })
            }
            Source::Extension { .. } => {
                let host = self.extension_host.as_mut().unwrap();
                host.sync_configuration(&self.settings)
                    .and_then(|()| {
                        host.request_language_provider(
                            Kind::Symbols,
                            &self.documents,
                            &self.hidden_documents,
                            self.active,
                            json!({}),
                        )
                    })
                    .and_then(|ticket| ticket.context("Outline provider no longer matches"))
                    .map(Call::Extension)
            }
            Source::Unsupported => unreachable!(),
        };
        self.outline.wanted = None;
        match call {
            Ok(call) => {
                self.outline.status = if self.outline.published.is_some() {
                    OutlineStatus::Updating
                } else {
                    OutlineStatus::Loading
                };
                self.outline.message.clear();
                self.outline.pending = Some(Pending {
                    stamp,
                    generation: self.outline.generation,
                    call,
                });
            }
            Err(error) => self.outline_error(&format!("{error:#}")),
        }
        true
    }
    fn outline_error(&mut self, message: &str) {
        self.outline.status = OutlineStatus::Error;
        self.outline.message = message.chars().take(240).collect();
    }
    pub(super) fn outline_native_owned(&self, request: &crate::lsp::Request) -> bool {
        self.outline.pending.as_ref().is_some_and(|p|matches!(&p.call,Call::Native {token,instance} if *token==request.token && Arc::ptr_eq(instance,&request.server)))
    }
    pub(super) fn outline_native_response(
        &mut self,
        request: &crate::lsp::Request,
        value: &Value,
    ) -> Result<()> {
        if !self.outline_native_owned(request) {
            return Ok(());
        }
        let pending = self.outline.pending.take().unwrap();
        if pending.generation != self.outline.generation
            || !self.outline_stamp_current(&pending.stamp)
            || self
                .lsp
                .as_ref()
                .is_none_or(|client| !client.request_current(request))
        {
            return Ok(());
        }
        self.publish_outline(pending.stamp, Proof::Native(request.clone()), value)
    }
    pub(super) fn outline_native_failure(&mut self, request: &crate::lsp::Request, error: &str) {
        if !self.outline_native_owned(request) {
            return;
        }
        let pending = self.outline.pending.take().unwrap();
        if pending.generation == self.outline.generation
            && self.outline_stamp_current(&pending.stamp)
        {
            self.outline_error(error);
        }
    }
    pub(super) fn outline_provider_owned(&self, ticket: &Ticket) -> bool {
        self.outline.pending.as_ref().is_some_and(|p|matches!(&p.call,Call::Extension(original) if original.id==ticket.id && original.session==ticket.session))
    }
    pub(super) fn outline_provider_response(
        &mut self,
        ticket: &Ticket,
        result: Result<Value, String>,
    ) -> Result<()> {
        if !self.outline_provider_owned(ticket) {
            return Ok(());
        }
        let pending = self.outline.pending.take().unwrap();
        if pending.generation != self.outline.generation
            || !self.outline_stamp_current(&pending.stamp)
            || self
                .extension_host
                .as_ref()
                .is_none_or(|host| !host.provider_ticket_current(ticket, self.doc()))
        {
            return Ok(());
        }
        match result {
            Ok(value) => {
                self.publish_outline(pending.stamp, Proof::Extension(ticket.clone()), &value)
            }
            Err(error) => {
                self.outline_error(&error);
                Ok(())
            }
        }
    }
    fn publish_outline(&mut self, stamp: Stamp, proof: Proof, value: &Value) -> Result<()> {
        let selected = self.outline.selected.and_then(|index| {
            self.outline
                .published
                .as_ref()?
                .identities
                .get(index)
                .copied()
        });
        let result = (|| -> Result<Published> {
            let tree = crate::outline::parse_document(value, &stamp.uri()?)?;
            tree.validate_document(self.doc())?;
            let ranges = tree
                .nodes
                .iter()
                .map(|node| {
                    Ok((
                        crate::outline::strict_offset(self.doc(), node.range.start)?,
                        crate::outline::strict_offset(self.doc(), node.range.end)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let identities = identities(&tree);
            Ok(Published {
                stamp,
                proof,
                tree,
                ranges,
                identities,
            })
        })();
        match result {
            Ok(published) => {
                let Some(publication) = self.outline.publication.checked_add(1) else {
                    self.outline.exhausted = true;
                    self.outline_error(
                        "Document-symbol publication identity exhausted; restart the editor",
                    );
                    bail!("Document-symbol publication identity exhausted");
                };
                self.outline.publication = publication;
                self.outline
                    .collapsed
                    .retain(|id| published.identities.contains(id));
                self.outline.selected = selected.and_then(|id| {
                    published
                        .identities
                        .iter()
                        .position(|candidate| *candidate == id)
                });
                self.outline.published = Some(published);
                self.outline.status = OutlineStatus::Ready;
                self.outline.message.clear();
                self.outline.cursor = None;
                self.outline.active = None;
                self.rebuild_outline_rows();
                self.update_outline_active();
                Ok(())
            }
            Err(error) => {
                self.outline_error(&format!("{error:#}"));
                Err(error)
            }
        }
    }
    fn rebuild_outline_rows(&mut self) {
        self.outline.visible.clear();
        let Some(published) = &self.outline.published else {
            self.outline.selected = None;
            return;
        };
        let mut hidden = vec![false; published.tree.nodes.len()];
        for (index, node) in published.tree.nodes.iter().enumerate() {
            hidden[index] = node.parent.is_some_and(|parent| {
                hidden[parent]
                    || self
                        .outline
                        .collapsed
                        .contains(&published.identities[parent])
            });
            if !hidden[index] {
                self.outline.visible.push(index);
            }
        }
        if self
            .outline
            .selected
            .is_none_or(|selected| !self.outline.visible.contains(&selected))
        {
            self.outline.selected = self.outline.visible.first().copied();
        }
    }
    fn update_outline_active(&mut self) -> bool {
        if !self.outline_published_current() {
            return false;
        }
        let pane = self.panes.get(self.active_pane).map_or(0, |p| p.id);
        let doc = self.doc();
        let cursor = (doc.id, doc.text_epoch(), doc.cursor, pane);
        if self.outline.cursor == Some(cursor) {
            return false;
        }
        let published = self.outline.published.as_ref().unwrap();
        let active = published
            .ranges
            .iter()
            .enumerate()
            .filter(|(_, range)| range.0 <= doc.cursor && doc.cursor <= range.1)
            .min_by_key(|(index, range)| {
                (
                    u8::MAX - published.tree.nodes[*index].depth,
                    range.1 - range.0,
                )
            })
            .map(|(index, _)| index);
        let changed = self.outline.active != active;
        self.outline.cursor = Some(cursor);
        self.outline.active = active;
        if changed
            && self.outline.follow_cursor
            && let Some(active) = active
        {
            let mut parent = self.outline.published.as_ref().unwrap().tree.nodes[active].parent;
            while let Some(index) = parent {
                let published = self.outline.published.as_ref().unwrap();
                self.outline.collapsed.remove(&published.identities[index]);
                parent = published.tree.nodes[index].parent;
            }
            self.outline.selected = Some(active);
            self.rebuild_outline_rows();
        }
        changed
    }
    pub(super) fn focus_outline(&mut self) {
        self.outline.enabled = true;
        self.sidebar = true;
        self.focus = Focus::Outline;
        self.observe_outline();
        if self.outline.pending.is_none()
            && self.outline.status == OutlineStatus::Error
            && !self.outline.exhausted
        {
            self.outline.wanted = Some(Instant::now());
            self.outline.message.clear();
            self.outline.status = if self.outline.published.is_some() {
                OutlineStatus::Updating
            } else {
                OutlineStatus::Loading
            };
        }
        self.message =
            "Outline · arrows navigate · Enter reveals · Escape returns to editor".into();
    }
    pub(super) fn outline_expand_all(&mut self, expand: bool) {
        self.outline.enabled = true;
        self.sidebar = true;
        self.observe_outline();
        self.outline.collapsed.clear();
        if !expand && let Some(published) = &self.outline.published {
            for node in &published.tree.nodes {
                if let Some(parent) = node.parent {
                    self.outline.collapsed.insert(published.identities[parent]);
                }
            }
        }
        self.rebuild_outline_rows();
    }
    pub(super) fn outline_follow_cursor(&mut self) {
        self.outline.follow_cursor = !self.outline.follow_cursor;
        if self.outline.follow_cursor {
            self.outline.cursor = None;
            self.outline.active = None;
            self.update_outline_active();
        }
        self.message = format!(
            "Outline: Follow Cursor {}",
            if self.outline.follow_cursor {
                "on"
            } else {
                "off"
            }
        );
    }
    fn outline_accept(&mut self) -> Result<()> {
        if self.focus != Focus::Outline
            || self.prompt.is_some()
            || self.modal.is_some()
            || !self.outline_view().actionable
        {
            bail!("Outline changed or is still updating; wait for current symbols");
        }
        let index = self
            .outline
            .selected
            .context("No selected outline symbol")?;
        if !self.outline.visible.contains(&index) {
            bail!("Outline symbol is no longer visible");
        }
        self.reveal_document_symbol(index, self.outline.publication)
    }
    pub(super) fn reveal_document_symbol(&mut self, index: usize, generation: u64) -> Result<()> {
        if generation != self.outline.publication || !self.document_symbols_view().current {
            bail!("Document symbols changed or are still updating");
        }
        let published = self
            .outline
            .published
            .as_ref()
            .context("No current document symbols")?;
        let node = published
            .tree
            .nodes
            .get(index)
            .context("No current document symbol")?;
        let offset = crate::outline::strict_offset(self.doc(), node.selection_range.start)?;
        let label: String = node.name.chars().take(120).collect();
        self.navigation_input_interaction();
        let previous = self.suspend_navigation_observation();
        self.doc_mut().clear_secondary();
        self.doc_mut().move_to(offset, false);
        self.focus = Focus::Editor;
        self.sync_pane();
        self.resume_navigation_observation(previous, navigation_history::Reason::Jump);
        self.update_outline_active();
        self.message = format!("Symbol · {label}");
        Ok(())
    }
    pub(crate) fn outline_offset(&self, height: u16) -> usize {
        self.outline
            .selected
            .and_then(|selected| {
                self.outline
                    .visible
                    .iter()
                    .position(|index| *index == selected)
            })
            .unwrap_or(0)
            .saturating_sub(height.saturating_sub(1) as usize)
    }
    pub(super) fn outline_click(&mut self, row: usize) {
        self.focus = Focus::Outline;
        self.outline.selected = self
            .outline
            .visible
            .get(self.outline_offset(self.outline_area.height) + row)
            .copied();
        if let Err(error) = self.outline_accept() {
            self.message = error.to_string();
        }
    }
    pub(super) fn outline_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Editor,
            KeyCode::Tab => self.focus = Focus::Explorer,
            KeyCode::Enter => {
                if let Err(error) = self.outline_accept() {
                    self.message = error.to_string();
                }
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => {
                let position = self
                    .outline
                    .selected
                    .and_then(|selected| {
                        self.outline
                            .visible
                            .iter()
                            .position(|index| *index == selected)
                    })
                    .unwrap_or(0);
                let last = self.outline.visible.len().saturating_sub(1);
                let page = self.outline_area.height.max(1) as usize;
                let position = match key.code {
                    KeyCode::Up => position.saturating_sub(1),
                    KeyCode::Down => (position + 1).min(last),
                    KeyCode::Home => 0,
                    KeyCode::End => last,
                    KeyCode::PageUp => position.saturating_sub(page),
                    _ => position.saturating_add(page).min(last),
                };
                self.outline.selected = self.outline.visible.get(position).copied();
            }
            KeyCode::Left | KeyCode::Right => {
                if let Some(index) = self.outline.selected
                    && let Some(published) = &self.outline.published
                {
                    let id = published.identities[index];
                    let child = published
                        .tree
                        .nodes
                        .iter()
                        .position(|node| node.parent == Some(index));
                    if key.code == KeyCode::Left {
                        if child.is_some() && !self.outline.collapsed.contains(&id) {
                            self.outline.collapsed.insert(id);
                        } else if let Some(parent) = published.tree.nodes[index].parent {
                            self.outline.selected = Some(parent);
                        }
                    } else if child.is_some() && self.outline.collapsed.contains(&id) {
                        self.outline.collapsed.remove(&id);
                    } else if let Some(child) = child {
                        self.outline.selected = Some(child);
                    }
                    self.rebuild_outline_rows();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::{Event, outline_tests};

    fn fixture() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        std::fs::write(&path, "猫🙂\r\nbody\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        // Keep an independent existing Redo entry: outlining/navigation must
        // never consume editing history or write the target's backing file.
        app.doc_mut().insert("seed", false);
        app.doc_mut().undo();
        app.lsp = Some(outline_tests::start(root.path(), app.doc()));
        app.focus_outline();
        (root, app)
    }
    fn symbols() -> Value {
        json!([{"name":"猫","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":4}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}])
    }
    fn launch(app: &mut App) -> u64 {
        app.outline.wanted = Some(Instant::now());
        assert!(app.poll_outline());
        let token = match &app.outline.pending.as_ref().unwrap().call {
            Call::Native { token, .. } => *token,
            Call::Extension(_) => panic!("native fixture unexpectedly used an extension"),
        };
        outline_tests::held(app.lsp.as_mut().unwrap(), token);
        token
    }
    fn release(app: &mut App, token: u64, value: Value) -> Result<()> {
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":token,"result":value}))
            .unwrap();
        let events = outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
        for event in events {
            if let Event::Response(request, value) = event {
                app.outline_native_response(&request, &value)?;
            }
        }
        Ok(())
    }
    fn assert_integrity_and_redo(app: &mut App, id: u64, epoch: u64) {
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), "猫🙂\r\nbody\r\n");
        assert_eq!(
            std::fs::read(app.doc().path.as_ref().unwrap()).unwrap(),
            "猫🙂\r\nbody\r\n".as_bytes()
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "seed猫🙂\r\nbody\r\n");
    }
    #[test]
    fn malformed_refresh_preserves_previous_tree_inert_without_automatic_retry_spin() {
        let (_root, mut app) = fixture();
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        let token = launch(&mut app);
        release(&mut app, token, symbols()).unwrap();
        assert!(app.outline_view().actionable);
        // A protocol lifetime change makes the previous proof stale even when
        // the model, bytes and text epoch remain identical.
        app.lsp.as_mut().unwrap().sync(&[]).unwrap();
        app.lsp.as_mut().unwrap().sync(&app.documents).unwrap();
        assert!(!app.outline_view().actionable);
        assert!(app.observe_outline());
        let token = launch(&mut app);
        let mut bad = symbols();
        bad[0]["selectionRange"]["end"]["line"] = json!(500);
        assert!(release(&mut app, token, bad).is_err());
        assert_eq!(app.outline_view().status, OutlineStatus::Error);
        assert_eq!(app.outline_view().tree.unwrap().nodes[0].name, "猫");
        assert!(!app.outline_view().actionable);
        let generation = app.outline.generation;
        for _ in 0..32 {
            assert!(!app.poll_outline());
            assert!(app.outline.pending.is_none());
            assert!(app.outline.wanted.is_none());
            assert!(app.lsp.as_ref().unwrap().symbol_available());
            assert_eq!(app.outline.generation, generation);
        }
        app.focus_outline();
        assert!(app.outline.wanted.is_some());
        assert_integrity_and_redo(&mut app, id, epoch);
    }
    #[test]
    fn held_outline_workspace_round_trip_retires_original_callback_without_model_mutation() {
        let (root, mut app) = fixture();
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        let selections = app.doc().selections();
        let token = launch(&mut app);
        let workspace = app.workspace.root.clone();
        let generation = app.outline.generation;
        app.workspace.root = root.path().join("another-workspace");
        assert!(app.observe_outline());
        app.workspace.root = workspace;
        assert!(app.observe_outline());
        assert!(app.outline.generation >= generation + 2);
        assert!(app.outline.pending.is_none());
        assert!(!app.lsp.as_ref().unwrap().symbol_available());
        release(&mut app, token, symbols()).unwrap();
        assert!(app.outline_view().tree.is_none());
        assert!(!app.outline_view().actionable);
        assert_eq!(app.doc().selections(), selections);
        let fresh = launch(&mut app);
        assert_ne!(fresh, token);
        release(&mut app, fresh, symbols()).unwrap();
        assert!(app.outline_view().actionable);
        assert_integrity_and_redo(&mut app, id, epoch);
    }
    #[test]
    fn synchronized_close_reopen_requires_new_proof_before_outline_can_reveal() {
        let (_root, mut app) = fixture();
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        let token = launch(&mut app);
        release(&mut app, token, symbols()).unwrap();
        assert!(app.outline_view().actionable);
        app.lsp.as_mut().unwrap().sync(&[]).unwrap();
        app.lsp.as_mut().unwrap().sync(&app.documents).unwrap();
        assert!(!app.outline_view().actionable);
        assert!(app.outline_accept().is_err());
        assert!(app.observe_outline());
        let generation = app.outline.generation;
        for _ in 0..32 {
            app.observe_outline();
            assert_eq!(app.outline.generation, generation);
        }
        let fresh = launch(&mut app);
        release(&mut app, fresh, symbols()).unwrap();
        assert!(app.outline_view().actionable);
        app.doc_mut().secondary = vec![crate::document::Selection::caret(2)];
        app.doc_mut().anchor = Some(2);
        app.doc_mut().cursor = 1;
        app.outline_accept().unwrap();
        assert_eq!(
            app.doc().selections(),
            vec![crate::document::Selection::caret(0)]
        );
        assert_integrity_and_redo(&mut app, id, epoch);
    }
    #[test]
    fn oversized_paths_and_empty_model_state_do_not_capture_or_queue_outline_requests() {
        let (_root, mut app) = fixture();
        app.outline.current = None;
        app.doc_mut().path = Some(PathBuf::from(format!("{}.cpp", "x".repeat(4097))));
        assert!(app.observe_outline());
        assert_eq!(app.outline_view().status, OutlineStatus::Error);
        assert!(app.outline.current.is_none());
        assert!(app.outline.pending.is_none());
        assert!(app.outline.wanted.is_none());
        assert!(app.outline_view().message.contains("4 KiB"));
        app.documents.clear();
        assert!(app.observe_outline());
        for _ in 0..32 {
            assert!(!app.observe_outline());
            assert_eq!(app.outline_view().status, OutlineStatus::NoDocument);
            assert!(app.outline.pending.is_none());
            assert!(app.outline.wanted.is_none());
        }
    }
    #[test]
    fn breadcrumbs_same_resource_labels_stay_inert_through_held_edit_undo_refresh() {
        let (_root, mut app) = fixture();
        let token = launch(&mut app);
        release(&mut app, token, symbols()).unwrap();
        app.doc_mut().move_to(1, false);
        app.observe_outline();
        app.observe_breadcrumbs();
        app.execute("breadcrumbs.focusAndSelect", Value::Null);
        assert!(app.breadcrumbs_view().picker.is_some());
        let generation = app.breadcrumbs_view().generation;
        let id = app.doc().id;
        app.doc_mut().insert("x", false);
        app.doc_mut().undo();
        app.lsp.as_mut().unwrap().sync(&app.documents).unwrap();
        app.observe_outline();
        app.observe_breadcrumbs();
        assert!(app.breadcrumbs_view().generation > generation);
        assert!(app.breadcrumbs_view().picker.is_none());
        assert_eq!(app.breadcrumbs_view().elements.last().unwrap().label, "猫");
        assert!(app.breadcrumbs_view().updating);
        let selections = app.doc().selections();
        let epoch = app.doc().text_epoch();
        app.execute("breadcrumbs.revealFocused", Value::Null);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert!(app.focus == Focus::Breadcrumbs);
        let token = launch(&mut app);
        for _ in 0..32 {
            app.poll_outline();
            app.poll_breadcrumbs();
            assert!(!app.lsp.as_ref().unwrap().symbol_available());
            assert_eq!(app.breadcrumbs_view().elements.last().unwrap().label, "猫");
            assert!(app.breadcrumbs_view().picker.is_none());
        }
        let mut fresh = symbols();
        fresh[0]["name"] = json!("fresh猫");
        release(&mut app, token, fresh).unwrap();
        app.observe_breadcrumbs();
        assert_eq!(
            app.breadcrumbs_view().elements.last().unwrap().label,
            "fresh猫"
        );
        assert!(!app.breadcrumbs_view().updating);
        app.execute("breadcrumbs.focusAndSelect", Value::Null);
        assert!(app.breadcrumbs_view().picker.is_some());
        app.execute("breadcrumbs.revealFocusedFromTreeAside", Value::Null);
        assert!(app.message.contains("not supported"));
        assert!(app.breadcrumbs_view().picker.is_some());
        assert_eq!(app.doc().selections(), selections);
        app.execute("list.select", Value::Null);
        assert!(app.focus == Focus::Editor);
        assert_eq!(app.doc().cursor, 0);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(
            std::fs::read(app.doc().path.as_ref().unwrap()).unwrap(),
            "猫🙂\r\nbody\r\n".as_bytes()
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "猫x🙂\r\nbody\r\n");
    }
}
