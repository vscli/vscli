//! Source-bound rename preparation, native input and atomic workspace editing.
use super::*;
use crate::{
    extension_providers::{Kind, Provider},
    extensions::providers::{MirrorSnapshot, NavigationRankIntent, NavigationRankReply, Ticket},
    lsp,
};
use anyhow::{Context as _, bail, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Source {
    id: u64,
    revision: u64,
    epoch: u64,
    language: u64,
    path: Option<PathBuf>,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
    groups: crate::editor_groups::UiProof,
    focus: Focus,
    workspace: PathBuf,
    profile: u64,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
}
impl Source {
    fn capture(app: &App) -> Result<Self> {
        super::extension_services::Context::check_budget(app)?;
        let doc = app
            .active_document()
            .context("Open an editable document before renaming")?;
        ensure!(
            !doc.is_read_only(),
            "Rename is unavailable for a read-only document"
        );
        ensure!(
            app.focus == Focus::Editor && app.prompt.is_none() && app.modal.is_none(),
            "Focus the editor before renaming"
        );
        Ok(Self {
            id: doc.id,
            revision: doc.revision,
            epoch: doc.text_epoch(),
            language: doc.language_epoch(),
            path: doc.path.clone(),
            selections: doc.selections(),
            pane: app.panes.get(app.active_pane).map(|pane| pane.id),
            groups: app.editor_groups.proof(),
            focus: app.focus.clone(),
            workspace: app.workspace.root.clone(),
            profile: app.settings_profile_generation(),
            settings: app.settings.extension_layers().clone(),
        })
    }
    fn current(&self, app: &App) -> bool {
        app.settings_error.is_none()
            && app.workspace.root == self.workspace
            && app.settings_profile_generation() == self.profile
            && Arc::ptr_eq(&self.settings, app.settings.extension_layers())
            && app.focus == self.focus
            && app.editor_groups.proof() == self.groups
            && app.panes.get(app.active_pane).map(|pane| pane.id) == self.pane
            && app.active_document().is_some_and(|doc| {
                doc.id == self.id
                    && doc.revision == self.revision
                    && doc.text_epoch() == self.epoch
                    && doc.language_epoch() == self.language
                    && doc.path == self.path
                    && doc.selections() == self.selections
            })
    }
}
enum Backend {
    Extension {
        intent: NavigationRankIntent,
        rank: Option<u64>,
        providers: Vec<Provider>,
        ticket: Option<Ticket>,
    },
    Native {
        instance: Arc<()>,
        token: Option<u64>,
    },
}
#[derive(PartialEq, Eq)]
enum Phase {
    Rank,
    Prepare,
    Name,
    Edits,
    Apply,
}
struct Run {
    source: Source,
    backend: Backend,
    phase: Phase,
    position: lsp::Position,
    inventory: Arc<Vec<MirrorSnapshot>>,
    index: usize,
    rejects: Vec<String>,
    fallback: Option<Value>,
    placeholder: String,
    name: String,
    started: Instant,
    capture: Option<Arc<super::workspace_transactions::Capture>>,
    transaction: Option<u64>,
    lifetime: Arc<AtomicBool>,
}
#[derive(Default)]
pub(super) struct State {
    run: Option<Run>,
    retired_transaction: Option<u64>,
}

impl App {
    pub(super) fn cancel_rename(&mut self) {
        if let Some(run) = self.rename.run.take() {
            run.lifetime.store(true, Ordering::Release);
            match run.backend {
                Backend::Extension { rank, ticket, .. } => {
                    if let Some(host) = &mut self.extension_host {
                        if let Some(rank) = rank {
                            host.cancel_navigation_rank(rank);
                        }
                        if let Some(ticket) = ticket {
                            let _ = host.cancel_language_provider(&ticket);
                        }
                    }
                }
                Backend::Native { token, .. } => {
                    if let (Some(client), Some(token)) = (&mut self.lsp, token) {
                        let _ = client.cancel_rename_request(token);
                    }
                }
            }
            if let Some(token) = run.transaction {
                self.cancel_workspace_transaction(token);
                self.rename.retired_transaction = Some(token);
            }
        }
        if matches!(
            self.prompt.as_ref().map(|prompt| &prompt.kind),
            Some(PromptKind::Rename)
        ) {
            self.prompt = None;
        }
    }
    pub(super) fn retire_rename_before_publication(
        &mut self,
        host: Option<&crate::extensions::Client>,
    ) {
        let stale = self.rename.run.as_ref().is_some_and(|run| {
            if run.phase != Phase::Apply {
                return false;
            }
            if run.transaction.is_some_and(|token| {
                self.workspace_transaction_completion()
                    .is_some_and(|done| done.token == token)
            }) {
                return false;
            }
            !run.source.current(self)
                || self.prompt.is_some()
                || self.modal.is_some()
                || match &run.backend {
                    Backend::Extension { intent, .. } => {
                        host.is_none_or(|host| !host.navigation_rank_current(intent, self.doc()))
                    }
                    Backend::Native { instance, .. } => self.lsp.as_ref().is_none_or(|client| {
                        !client.ready || !Arc::ptr_eq(instance, &client.identity())
                    }),
                }
        });
        if stale {
            self.cancel_rename();
        }
    }
    pub(super) fn rename_ui_event(&mut self, event: &Event) {
        let naming = self
            .rename
            .run
            .as_ref()
            .is_some_and(|run| run.phase == Phase::Name)
            && matches!(
                self.prompt.as_ref().map(|prompt| &prompt.kind),
                Some(PromptKind::Rename)
            );
        let allowed = matches!(event, Event::Resize(..))
            || matches!(event,Event::Key(key) if key.kind==KeyEventKind::Release)
            || naming
                && match event {
                    Event::Paste(_) => true,
                    Event::Key(key) => {
                        key.kind == KeyEventKind::Release || key.code != KeyCode::Esc
                    }
                    _ => false,
                };
        if !allowed {
            self.cancel_rename();
        }
    }
    pub(super) fn rename_escape(&mut self, event: &Event) -> bool {
        if self.rename.run.is_some()
            && matches!(event,Event::Key(key) if key.kind!=KeyEventKind::Release && key.code==KeyCode::Esc)
        {
            self.cancel_rename();
            true
        } else {
            false
        }
    }
    fn rename_inventory(&mut self) -> Result<Arc<Vec<MirrorSnapshot>>> {
        if let Some(host) = &mut self.extension_host {
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)?;
            return host.mirrored_snapshotset(&self.documents, &self.hidden_documents);
        }
        ensure!(
            self.documents.len() + self.hidden_documents.len() <= 128,
            "Rename supports at most 128 retained models"
        );
        self.documents
            .iter()
            .chain(&self.hidden_documents)
            .map(|doc| {
                Ok(MirrorSnapshot {
                    document: doc.id,
                    uri: crate::extensions::document_uri(doc)?,
                    path: doc.path.clone(),
                    revision: doc.revision,
                    text_epoch: doc.text_epoch(),
                    language_epoch: doc.language_epoch(),
                    // Native model proof, explicitly without an extension wire version.
                    version: 0,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(Arc::new)
    }
    fn native_rename_supported_here(&self) -> bool {
        self.active_document().is_some_and(|doc| {
            doc.path.as_ref().is_some_and(|path| {
                let language = doc
                    .language_override()
                    .unwrap_or_else(|| lsp::language(path));
                self.lsp.as_ref().is_some_and(|client| {
                    client.ready
                        && client.supports_rename()
                        && (client.language == language
                            || client.language == "cpp" && language == "c")
                })
            })
        })
    }
    pub(super) fn start_rename(&mut self) {
        self.cancel_rename();
        let result = (|| -> Result<()> {
            let source = Source::capture(self)?;
            let inventory = self.rename_inventory()?;
            let position = lsp::position(self.doc(), self.doc().cursor);
            let backend = if self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.navigation_family_registered(Kind::Rename))
            {
                let host = self.extension_host.as_mut().unwrap();
                host.sync_configuration(&self.settings)?;
                Backend::Extension {
                    intent: host.capture_navigation_rank(
                        Kind::Rename,
                        &self.documents,
                        &self.hidden_documents,
                        self.active,
                    )?,
                    rank: None,
                    providers: Vec::new(),
                    ticket: None,
                }
            } else {
                ensure!(
                    self.native_rename_supported_here(),
                    "No rename provider is available for this document"
                );
                let client = self
                    .lsp
                    .as_ref()
                    .context("No rename provider is available")?;
                ensure!(
                    client.ready && client.supports_rename(),
                    "No rename provider is available"
                );
                Backend::Native {
                    instance: client.identity(),
                    token: None,
                }
            };
            let phase = if matches!(backend, Backend::Extension { .. }) {
                Phase::Rank
            } else {
                Phase::Prepare
            };
            self.rename.run = Some(Run {
                source,
                backend,
                phase,
                position,
                inventory,
                index: 0,
                rejects: Vec::new(),
                fallback: None,
                placeholder: String::new(),
                name: String::new(),
                started: Instant::now(),
                capture: None,
                transaction: None,
                lifetime: Arc::new(AtomicBool::new(false)),
            });
            self.message = "Preparing rename…".into();
            self.dispatch_rename()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.cancel_rename();
            self.message = format!("Rename: {error:#}");
        }
    }
    fn rename_current(&self) -> bool {
        self.rename.run.as_ref().is_some_and(|run| {
            run.source.current(self)
                && !run.lifetime.load(Ordering::Acquire)
                && self.modal.is_none()
                && match &run.backend {
                    Backend::Extension { intent, .. } => self
                        .extension_host
                        .as_ref()
                        .is_some_and(|host| host.navigation_rank_current(intent, self.doc())),
                    Backend::Native { instance, .. } => self.lsp.as_ref().is_some_and(|client| {
                        client.ready && Arc::ptr_eq(instance, &client.identity())
                    }),
                }
                && if run.phase == Phase::Name {
                    matches!(
                        self.prompt.as_ref().map(|p| &p.kind),
                        Some(PromptKind::Rename)
                    )
                } else {
                    self.prompt.is_none()
                }
        })
    }
    fn show_rename_name(&mut self, value: Value) -> Result<()> {
        let range: lsp::Range = serde_json::from_value(value["range"].clone())?;
        let start = lsp::offset(self.doc(), range.start)?;
        let end = lsp::offset(self.doc(), range.end)?;
        ensure!(start <= end, "Invalid rename preparation range");
        let text = value["placeholder"]
            .as_str()
            .filter(|text| text.len() <= 64 * 1024)
            .context("Invalid rename placeholder")?
            .to_owned();
        self.rename
            .run
            .as_mut()
            .context("Rename retired")?
            .placeholder = text.clone();
        self.rename.run.as_mut().unwrap().phase = Phase::Name;
        self.start_prompt(PromptKind::Rename, text);
        self.message = "Rename symbol · Enter applies · Esc cancels".into();
        Ok(())
    }
    fn reject_rename_reason(&mut self, reason: &str) -> Result<()> {
        let run = self.rename.run.as_mut().context("Rename retired")?;
        ensure!(
            reason.len() <= 4096
                && run.rejects.iter().map(String::len).sum::<usize>() + reason.len() <= 16 * 1024,
            "Rename rejection details exceed bounds"
        );
        run.rejects.push(reason.to_owned());
        Ok(())
    }
    fn dispatch_rename(&mut self) -> Result<()> {
        if !self.rename_current() {
            bail!("Original rename source changed");
        }
        let run = self.rename.run.as_mut().unwrap();
        match &mut run.backend {
            Backend::Extension {
                intent,
                rank,
                providers,
                ticket,
            } => {
                let host = self
                    .extension_host
                    .as_mut()
                    .context("Rename host stopped")?;
                if run.phase == Phase::Rank {
                    if rank.is_none() && host.navigation_rank_available() {
                        *rank = Some(
                            host.request_navigation_rank(intent, &self.documents[self.active])?,
                        );
                    }
                    return Ok(());
                }
                if ticket.is_some() || matches!(run.phase, Phase::Name | Phase::Apply) {
                    return Ok(());
                }
                if run.phase == Phase::Prepare
                    && (run.index >= providers.len() || !providers[run.index].resolves)
                {
                    run.index = 0;
                    if !run.rejects.is_empty() {
                        bail!("{}", run.rejects.join("\n"));
                    }
                    let fallback = run
                        .fallback
                        .clone()
                        .context("Rename has no original fallback")?;
                    return self.show_rename_name(fallback);
                }
                if run.index >= providers.len() {
                    bail!("{}", run.rejects.join("\n"));
                }
                if host.language_provider_capacity() {
                    let name = (run.phase == Phase::Edits).then_some(run.name.as_str());
                    *ticket = Some(host.request_rename_provider(
                        &providers[run.index],
                        &self.documents,
                        &self.hidden_documents,
                        self.active,
                        run.position,
                        name,
                    )?);
                }
            }
            Backend::Native { token, .. } => {
                if token.is_some() || matches!(run.phase, Phase::Name | Phase::Apply) {
                    return Ok(());
                }
                let client = self.lsp.as_mut().context("Rename server stopped")?;
                if run.phase == Phase::Prepare && !client.rename_preparation_supported() {
                    return self.show_rename_name(default_location(&self.documents[self.active])?);
                }
                client.sync(&self.documents)?;
                let name = (run.phase == Phase::Edits).then_some(run.name.as_str());
                *token = Some(client.request_rename(
                    &self.documents[self.active],
                    name,
                    run.source.pane,
                )?);
            }
        }
        Ok(())
    }
    pub(super) fn accept_rename_name(&mut self, name: String) {
        let result = (|| -> Result<()> {
            let run = self
                .rename
                .run
                .as_ref()
                .context("Original rename has retired")?;
            ensure!(
                run.source.current(self) && run.phase == Phase::Name,
                "Original rename source changed"
            );
            if name == run.placeholder || name.chars().all(js_whitespace) {
                self.cancel_rename();
                return Ok(());
            }
            ensure!(name.len() <= 64 * 1024, "Rename name exceeds 64 KiB");
            let run = self.rename.run.as_mut().unwrap();
            run.name = name;
            run.rejects.clear();
            run.phase = Phase::Edits;
            run.started = Instant::now();
            self.message = "Computing rename edits…".into();
            self.dispatch_rename()
        })();
        if let Err(error) = result {
            self.cancel_rename();
            self.message = format!("Rename: {error:#}");
        }
    }
    pub(super) fn rename_rank_owned(&self, id: u64) -> bool {
        self.rename.run.as_ref().is_some_and(
            |run| matches!(run.backend,Backend::Extension{rank:Some(rank),..} if rank==id),
        )
    }
    pub(super) fn rename_rank_reply(&mut self, reply: NavigationRankReply) -> Result<()> {
        ensure!(self.rename_current(), "Original rename source changed");
        let providers = reply.result.map_err(anyhow::Error::msg)?;
        if providers.is_empty() {
            ensure!(
                self.native_rename_supported_here(),
                "No rename provider matches the source"
            );
            let client = self
                .lsp
                .as_ref()
                .context("No rename provider matches the source")?;
            ensure!(
                client.ready && client.supports_rename(),
                "No rename provider matches the source"
            );
            let run = self.rename.run.as_mut().unwrap();
            run.backend = Backend::Native {
                instance: client.identity(),
                token: None,
            };
            run.phase = Phase::Prepare;
        } else {
            let run = self.rename.run.as_mut().unwrap();
            if let Backend::Extension {
                providers: cohort,
                rank,
                ..
            } = &mut run.backend
            {
                *cohort = providers;
                *rank = None;
            }
            run.fallback = reply.fallback;
            run.phase = Phase::Prepare;
        }
        self.dispatch_rename()
    }
    pub(super) fn rename_provider_reply(
        &mut self,
        ticket: Ticket,
        result: Result<Value, String>,
    ) -> Result<()> {
        let owned=self.rename.run.as_ref().is_some_and(|run| matches!(&run.backend,Backend::Extension{ticket:Some(original),..} if original.id==ticket.id && original.session==ticket.session));
        if !owned {
            return Ok(());
        }
        ensure!(
            self.rename_current()
                && self
                    .extension_host
                    .as_ref()
                    .is_some_and(|host| host.provider_ticket_current(&ticket, self.doc())),
            "Original rename callback source changed"
        );
        let value = result.map_err(anyhow::Error::msg)?;
        if let Backend::Extension {
            ticket: pending, ..
        } = &mut self.rename.run.as_mut().unwrap().backend
        {
            *pending = None;
        }
        if value.is_null()
            || value["rejectReason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        {
            if let Some(reason) = value["rejectReason"]
                .as_str()
                .filter(|reason| !reason.is_empty())
            {
                self.reject_rename_reason(reason)?;
            } else if self.rename.run.as_ref().unwrap().phase == Phase::Edits {
                self.reject_rename_reason("No result.")?;
            }
            self.rename.run.as_mut().unwrap().index += 1;
            return self.dispatch_rename();
        }
        if self.rename.run.as_ref().unwrap().phase == Phase::Prepare {
            self.show_rename_name(value)
        } else {
            self.apply_rename_edits(value, Some(ticket))
        }
    }
    pub(super) fn native_rename_reply(
        &mut self,
        request: lsp::Request,
        result: Result<Value, String>,
    ) -> Result<()> {
        let owned=self.rename.run.as_ref().is_some_and(|run| matches!(&run.backend,Backend::Native{instance,token:Some(token)} if *token==request.token && Arc::ptr_eq(instance,&request.server)));
        if !owned {
            return Ok(());
        }
        ensure!(
            self.rename_current()
                && self
                    .lsp
                    .as_ref()
                    .is_some_and(|client| client.request_current(&request)),
            "Original rename server/source changed"
        );
        let value = result.map_err(anyhow::Error::msg)?;
        if let Backend::Native { token, .. } = &mut self.rename.run.as_mut().unwrap().backend {
            *token = None;
        }
        if self.rename.run.as_ref().unwrap().phase == Phase::Prepare {
            ensure!(
                !value.is_null(),
                "The language server rejected rename at this position"
            );
            let value = if value["defaultBehavior"] == true {
                default_location(self.doc())?
            } else if value.get("range").is_some() {
                value
            } else {
                let range: lsp::Range = serde_json::from_value(value.clone())?;
                let start = lsp::offset(self.doc(), range.start)?;
                let end = lsp::offset(self.doc(), range.end)?;
                ensure!(
                    start <= end && end - start <= 64 * 1024,
                    "Rename range exceeds bounds"
                );
                json!({"range":range,"placeholder":self.doc().text.slice(start..end).to_string()})
            };
            self.show_rename_name(value)
        } else {
            self.apply_rename_edits(native_edit_dto(&value, &request)?, None)
        }
    }
    fn apply_rename_edits(&mut self, value: Value, ticket: Option<Ticket>) -> Result<()> {
        let run = self
            .rename
            .run
            .as_ref()
            .context("Original rename retired")?;
        let entries = text_entries(&value)?;
        if entries.is_empty() {
            self.rename.run = None;
            self.message = "Rename returned no changes".into();
            return Ok(());
        }
        let lifetime = if let Some(ticket) = ticket {
            self.extension_host
                .as_ref()
                .context("Rename host stopped")?
                .rename_owner_lifetime(&ticket)?
        } else {
            run.lifetime.clone()
        };
        let origin = super::workspace_transactions::Origin::native("native-rename", lifetime)?;
        let args = json!({"entries":entries,"metadata":{"isRefactoring":true}});
        let capture =
            self.capture_workspace_transaction(origin, &args, &run.inventory, Instant::now())?;
        let run = self.rename.run.as_mut().unwrap();
        run.phase = Phase::Apply;
        run.capture = Some(capture);
        run.started = Instant::now();
        self.message = "Preparing rename transaction…".into();
        Ok(())
    }
    pub(super) fn poll_rename(&mut self) -> bool {
        if let Some(token) = self.rename.retired_transaction
            && self.take_workspace_transaction_completion(token).is_some()
        {
            self.rename.retired_transaction = None;
        }
        if let Some(token) = self.rename.run.as_ref().and_then(|run| run.transaction)
            && let Some(done) = self.take_workspace_transaction_completion(token)
        {
            self.rename.run = None;
            self.message = if done.applied {
                "Renamed atomically · Undo restores the workspace".into()
            } else {
                format!(
                    "Rename rejected; work retained: {}",
                    done.reason.unwrap_or_default()
                )
            };
            return true;
        }
        if self.rename.run.is_none() {
            return false;
        }
        let result = (|| -> Result<()> {
            ensure!(self.rename_current(), "Original rename source changed");
            let run = self.rename.run.as_ref().unwrap();
            ensure!(
                run.phase == Phase::Name
                    || run.phase == Phase::Apply
                    || run.started.elapsed() < Duration::from_secs(15),
                "Rename deadline elapsed"
            );
            if run.phase == Phase::Apply {
                ensure!(
                    run.transaction.is_some() || run.started.elapsed() < Duration::from_secs(5),
                    "Rename preparation queue deadline elapsed; previous actual worker remains occupied"
                );
                if run.transaction.is_none() && !self.workspace_transactions_pending() {
                    let capture = self
                        .rename
                        .run
                        .as_mut()
                        .unwrap()
                        .capture
                        .take()
                        .context("Rename capture missing")?;
                    let token = self.start_workspace_transaction(capture)?;
                    self.rename.run.as_mut().unwrap().transaction = Some(token);
                }
            } else {
                self.dispatch_rename()?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.cancel_rename();
            self.message = format!("Rename: {error:#}");
            return true;
        }
        false
    }
}

fn js_whitespace(value: char) -> bool {
    matches!(value,'\u{0009}'..='\u{000D}'|'\u{0020}'|'\u{00A0}'|'\u{1680}'|'\u{2000}'..='\u{200A}'|'\u{2028}'|'\u{2029}'|'\u{202F}'|'\u{205F}'|'\u{3000}'|'\u{FEFF}')
}

fn default_location(doc: &Document) -> Result<Value> {
    use std::sync::OnceLock;
    static WORD: OnceLock<regex::Regex> = OnceLock::new();
    let regex=WORD.get_or_init(|| regex::Regex::new(r#"(-?[0-9]*\.[0-9][A-Za-z0-9_]*)|([^`~!@#$%^&*()\-=+\[\]{}\\|;:'\",.<>/?\x09-\x0D\x20\u{00A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}]+)"#).expect("static word expression"));
    let cursor = doc.cursor;
    let line = doc.text.char_to_line(cursor);
    let mut start = cursor;
    let mut units = 0;
    while start > doc.line_start(line) && units + doc.text.char(start - 1).len_utf16() <= 500 {
        start -= 1;
        units += doc.text.char(start).len_utf16();
    }
    let mut end = cursor;
    units = 0;
    while end < doc.line_end(line) && units + doc.text.char(end).len_utf16() <= 500 {
        units += doc.text.char(end).len_utf16();
        end += 1;
    }
    let text = doc.text.slice(start..end).to_string();
    let position = doc.text.slice(start..cursor).len_bytes();
    let range = regex
        .find_iter(&text)
        .find(|value| value.start() <= position && value.end() >= position);
    let (start, end, placeholder) = range
        .map(|value| {
            (
                start + text[..value.start()].chars().count(),
                start + text[..value.end()].chars().count(),
                value.as_str(),
            )
        })
        .unwrap_or((cursor, cursor, ""));
    Ok(
        json!({"range":{"start":lsp::position(doc,start),"end":lsp::position(doc,end)},"placeholder":placeholder}),
    )
}

fn text_entries(value: &Value) -> Result<Vec<Value>> {
    crate::decorations::json_budget(value, 2 * 1024 * 1024)?;
    let edits = value["edits"]
        .as_array()
        .filter(|edits| edits.len() <= 4096)
        .context("Invalid rename edit cohort")?;
    let mut entries: Vec<(String, Vec<Value>)> = Vec::new();
    for edit in edits {
        ensure!(
            edit.as_object().is_some_and(|row| row.keys().all(|key| [
                "resource",
                "textEdit",
                "versionId",
                "metadata"
            ]
            .contains(&key.as_str()))),
            "Unsupported rename edit row"
        );
        ensure!(
            edit["versionId"].is_null(),
            "Versioned provider rename edits require explicit target admission"
        );
        ensure!(
            edit["metadata"].is_null()
                && edit["oldResource"].is_null()
                && edit["newResource"].is_null(),
            "Rename resource/annotation operations require their native backend"
        );
        let uri = edit["resource"]
            .as_str()
            .context("Rename text resource is missing")?;
        let text = &edit["textEdit"];
        ensure!(
            text["insertAsSnippet"] != true && text["eol"].is_null(),
            "Rename snippet/EOL operations require their native backend"
        );
        let content = text["text"].as_str().context("Rename text is missing")?;
        let range = &text["range"];
        let coordinate = |key: &str| {
            range[key]
                .as_u64()
                .filter(|value| *value > 0 && *value <= i32::MAX as u64)
                .map(|value| value - 1)
                .context("Invalid rename text range")
        };
        let range = json!({"start":{"line":coordinate("startLineNumber")?,"character":coordinate("startColumn")?},"end":{"line":coordinate("endLineNumber")?,"character":coordinate("endColumn")?}});
        let index = if let Some(index) = entries.iter().position(|(key, _)| key == uri) {
            index
        } else {
            ensure!(entries.len() < 128, "Rename exceeds 128 resources");
            entries.push((uri.into(), Vec::new()));
            entries.len() - 1
        };
        entries[index]
            .1
            .push(json!({"range":range,"newText":content}));
    }
    Ok(entries
        .into_iter()
        .map(|(uri, edits)| json!({"uri":uri,"edits":edits}))
        .collect())
}
fn native_edit_dto(value: &Value, request: &lsp::Request) -> Result<Value> {
    crate::decorations::json_budget(value, 2 * 1024 * 1024)?;
    if value.is_null() {
        return Ok(json!({"edits":[]}));
    }
    ensure!(
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| [
                "changes",
                "documentChanges",
                "changeAnnotations"
            ]
            .contains(&key.as_str()))),
        "Malformed native rename workspace edit"
    );
    ensure!(
        value["changeAnnotations"].is_null(),
        "Rename annotations require their native confirmation backend"
    );
    let mut edits = Vec::new();
    let mut add = |uri: &str, changes: &Value| -> Result<()> {
        for change in changes
            .as_array()
            .context("Invalid native rename text edits")?
        {
            ensure!(
                change["annotationId"].is_null(),
                "Rename annotations require their native backend"
            );
            let range: lsp::Range = serde_json::from_value(change["range"].clone())?;
            ensure!(
                [
                    range.start.line,
                    range.start.character,
                    range.end.line,
                    range.end.character
                ]
                .into_iter()
                .all(|coordinate| coordinate < i32::MAX as usize),
                "Native rename range exceeds protocol bounds"
            );
            let text = change["newText"]
                .as_str()
                .context("Invalid native rename text")?;
            ensure!(edits.len() < 4096, "Rename exceeds 4096 edits");
            edits.push(json!({"resource":uri,"textEdit":{"range":{"startLineNumber":range.start.line+1,"startColumn":range.start.character+1,"endLineNumber":range.end.line+1,"endColumn":range.end.character+1},"text":text}}));
        }
        Ok(())
    };
    if let Some(changes) = value.get("changes") {
        for (uri, changes) in changes
            .as_object()
            .context("Invalid native rename changes")?
        {
            add(uri, changes)?;
        }
    }
    if let Some(changes) = value.get("documentChanges") {
        for change in changes
            .as_array()
            .context("Invalid native rename documentChanges")?
        {
            ensure!(
                change["kind"].is_null(),
                "Rename resource operations require their native backend"
            );
            let uri = change["textDocument"]["uri"]
                .as_str()
                .context("Missing native rename URI")?;
            if !change["textDocument"]["version"].is_null() {
                let version = change["textDocument"]["version"]
                    .as_i64()
                    .context("Malformed native rename target version")?;
                let path = url::Url::parse(uri)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("Unsupported native rename URI"))?;
                ensure!(
                    request
                        .workspace
                        .get(&path)
                        .is_some_and(|snapshot| snapshot.version == version),
                    "Native rename target version differs from the original request"
                );
            }
            add(uri, &change["edits"])?;
        }
    }
    Ok(json!({"edits":edits}))
}
