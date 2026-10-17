//! Native presentation and guarded application of optional extension language results.
use super::extension_services::Context;
use super::*;
use crate::{extension_providers::Kind, extensions::providers::Ticket, lsp};
use anyhow::{Context as _, bail};
use std::sync::mpsc::{self, Receiver, TryRecvError};
struct Lease {
    ticket: Ticket,
    context: Context,
    shown: bool,
}
struct SymbolIntent {
    provider: crate::extension_providers::Provider,
    source: (u64, u64),
    context: Context,
    extra: Value,
}
struct Loading {
    started: std::time::Instant,
    generation: u64,
    ticket: Ticket,
    context: Context,
    range: lsp::Range,
    receiver: Receiver<Result<Target, String>>,
}
enum Target {
    Existing(PathBuf),
    Loaded(Box<Document>),
}
#[derive(Default)]
pub(super) struct State {
    generation: u64,
    lease: Option<Lease>,
    symbol_intent: Option<SymbolIntent>,
    completion_resolves: std::collections::HashMap<u64, Ticket>,
    loading: Option<Loading>,
    pub symbols: Vec<crate::symbols::Symbol>,
}
impl App {
    pub(super) fn request_provider_completion_resolve(
        &mut self,
        original: &Ticket,
        item: &Value,
    ) -> Result<Ticket> {
        Context::check_budget(self)?;
        let lease = self
            .extension_providers
            .lease
            .as_ref()
            .context("Completion provider lease is no longer current")?;
        if original.id != lease.ticket.id || !self.provider_current(original, &lease.context) {
            bail!("Completion provider editor context changed");
        }
        let host = self
            .extension_host
            .as_mut()
            .context("Extension host stopped")?;
        let ticket = host.request_completion_resolve(
            original,
            item,
            &self.documents,
            &self.hidden_documents,
            self.active,
        )?;
        self.extension_providers
            .completion_resolves
            .insert(ticket.id, ticket.clone());
        Ok(ticket)
    }
    pub(super) fn cancel_provider_completion_resolve(&mut self, ticket: &Ticket) {
        self.extension_providers
            .completion_resolves
            .remove(&ticket.id);
        if let Some(host) = &mut self.extension_host {
            let _ = host.cancel_language_provider(ticket);
        }
    }
    pub(super) fn completion_provider_ticket(&self) -> Option<Ticket> {
        self.extension_providers
            .lease
            .as_ref()
            .filter(|lease| {
                lease.ticket.provider.kind == Kind::Completion
                    && self.provider_current(&lease.ticket, &lease.context)
            })
            .map(|lease| lease.ticket.clone())
    }
    pub(super) fn advance_completion_provider_interaction(&mut self) {
        if self.completion_provider_ticket().is_some()
            && let Some(lease) = &mut self.extension_providers.lease
        {
            lease.context.accept_next_interaction();
        }
    }
    pub(super) fn has_extension_provider(&self, kind: Kind) -> bool {
        self.active_document().is_some_and(|d| {
            self.extension_host
                .as_ref()
                .is_some_and(|h| h.language_provider(kind, d).is_some())
        })
    }
    pub(super) fn cancel_extension_provider(&mut self) {
        self.extension_providers.generation = self.extension_providers.generation.wrapping_add(1);
        if let Some(lease) = self.extension_providers.lease.take()
            && let Some(host) = &mut self.extension_host
        {
            let _ = host.cancel_language_provider(&lease.ticket);
        }
        for (_, ticket) in self.extension_providers.completion_resolves.drain() {
            if let Some(host) = &mut self.extension_host {
                let _ = host.cancel_language_provider(&ticket);
            }
        }
        self.extension_providers.symbols.clear();
        self.extension_providers.symbol_intent = None;
        // A canceled filesystem read retains the one worker slot until it exits.
    }
    fn provider_current(&self, ticket: &Ticket, context: &Context) -> bool {
        context.same_editor(self)
            && self.active_document().is_some_and(|doc| {
                self.extension_host
                    .as_ref()
                    .is_some_and(|host| host.provider_ticket_current(ticket, doc))
            })
    }
    pub(super) fn provider_ui_event(&mut self, event: &Event) {
        let allowed = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let picker = matches!(&self.modal, Some(Modal::Language { items, .. }) if items.iter().any(|i| matches!(i.action, language::LanguageAction::Provider { .. })));
                let symbols = self.provider_symbols_active()
                    && matches!(
                        self.prompt.as_ref().map(|p| &p.kind),
                        Some(PromptKind::Symbols)
                    );
                (picker
                    && matches!(
                        key.code,
                        KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                            | KeyCode::Enter
                            | KeyCode::Tab
                    ))
                    || (symbols
                        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                        && matches!(
                            key.code,
                            KeyCode::Char(_)
                                | KeyCode::Backspace
                                | KeyCode::Delete
                                | KeyCode::Left
                                | KeyCode::Right
                                | KeyCode::Home
                                | KeyCode::End
                                | KeyCode::Up
                                | KeyCode::Down
                                | KeyCode::Enter
                        ))
            }
            Event::Paste(_) => {
                self.provider_symbols_active()
                    && matches!(
                        self.prompt.as_ref().map(|p| &p.kind),
                        Some(PromptKind::Symbols)
                    )
            }
            _ => false,
        };
        let current = self
            .extension_providers
            .lease
            .as_ref()
            .is_some_and(|l| self.provider_current(&l.ticket, &l.context));
        if allowed
            && current
            && let Some(lease) = &mut self.extension_providers.lease
        {
            lease.context.accept_next_interaction();
        }
        let queued_current = self
            .extension_providers
            .symbol_intent
            .as_ref()
            .is_some_and(|intent| self.symbol_intent_current(intent));
        if allowed
            && queued_current
            && let Some(intent) = &mut self.extension_providers.symbol_intent
        {
            intent.context.accept_next_interaction();
        }
    }
    pub(super) fn extension_language_request(&mut self, method: &str, extra: Value) -> bool {
        let Some(kind) = Kind::from_method(method) else {
            return false;
        };
        if !self.has_extension_provider(kind) {
            return false;
        }
        if kind == Kind::Signature {
            self.request_signature();
            return true;
        }
        self.cancel_extension_provider();
        if let Err(error) = Context::check_budget(self) {
            self.message = format!("Extension language request rejected: {error}");
            return true;
        }
        if self.prompt.is_some() || self.modal.is_some() || self.focus != Focus::Editor {
            self.message =
                "Focus the editor before requesting an extension language provider".into();
            return true;
        }
        if kind == Kind::Symbols {
            self.start_prompt(PromptKind::Symbols, String::new());
            let provider = self
                .extension_host
                .as_ref()
                .unwrap()
                .language_provider(Kind::Symbols, self.doc())
                .unwrap()
                .clone();
            let source = self
                .extension_host
                .as_ref()
                .unwrap()
                .symbol_source_identity(&provider);
            if let Some(source) = source {
                self.extension_providers.symbol_intent = Some(SymbolIntent {
                    provider,
                    source,
                    context: Context::capture(self),
                    extra,
                });
                self.poll_provider_symbol_intent();
            } else {
                self.prompt = None;
                self.message = "Extension symbol source changed; request symbols again".into();
            }
            return true;
        }
        let context = Context::capture(self);
        let host = self.extension_host.as_mut().unwrap();
        let result = host.sync_configuration(&self.settings).and_then(|()| {
            host.request_language_provider(
                kind,
                &self.documents,
                &self.hidden_documents,
                self.active,
                extra,
            )
        });
        match result {
            Ok(Some(ticket)) => {
                self.extension_providers.lease = Some(Lease {
                    ticket,
                    context,
                    shown: false,
                });
                self.message = format!("Extension language request: {method}");
            }
            Ok(None) => (),
            Err(error) => self.message = format!("Extension language request failed: {error:#}"),
        }
        true
    }
    pub(super) fn poll_extension_providers(&mut self) -> bool {
        let mut changed = false;
        if self
            .extension_providers
            .lease
            .as_ref()
            .is_some_and(|l| !self.provider_current(&l.ticket, &l.context))
        {
            self.cancel_extension_provider();
            changed = true;
        }
        let replies = self
            .extension_host
            .as_mut()
            .map(|h| h.take_provider_replies())
            .unwrap_or_default();
        for reply in replies {
            if self.outline_provider_owned(&reply.ticket) {
                changed = true;
                if let Err(error) = self.outline_provider_response(&reply.ticket, reply.result) {
                    self.message = format!("Outline: {error:#}");
                }
                continue;
            }
            if reply.ticket.provider.kind == Kind::Signature {
                changed = true;
                if let Err(error) = self.extension_signature_reply(&reply.ticket, reply.result) {
                    self.message = format!("Extension parameter hints: {error:#}");
                }
                continue;
            }
            if reply.ticket.provider.kind == Kind::CodeAction {
                changed = true;
                self.extension_action_reply(reply.ticket, reply.result);
                continue;
            }
            if let Some(ticket) = self
                .extension_providers
                .completion_resolves
                .remove(&reply.ticket.id)
            {
                changed = true;
                if ticket.session != reply.ticket.session {
                    continue;
                }
                match reply.result {
                    Ok(value) => {
                        if let Err(error) =
                            self.provider_suggestion_resolve_response(&ticket, value)
                        {
                            self.provider_suggestion_resolve_error(&ticket, error.to_string());
                        }
                    }
                    Err(message) => self.provider_suggestion_resolve_error(&ticket, message),
                }
                continue;
            }
            if !self.extension_providers.lease.as_ref().is_some_and(|l| {
                l.ticket.id == reply.ticket.id
                    && l.ticket.session == reply.ticket.session
                    && self.provider_current(&l.ticket, &l.context)
            }) {
                continue;
            }
            changed = true;
            let result = reply
                .result
                .map_err(anyhow::Error::msg)
                .and_then(|v| self.provider_result(&reply.ticket, v));
            if let Err(error) = result {
                self.cancel_extension_provider();
                self.message = format!("Extension language response: {error:#}");
            }
        }
        if self
            .extension_providers
            .loading
            .as_ref()
            .is_some_and(|loading| {
                loading.generation == self.extension_providers.generation
                    && loading.started.elapsed() >= std::time::Duration::from_secs(3)
            })
        {
            self.cancel_extension_provider();
            self.message =
                "Extension location read timed out; worker slot retained until completion".into();
            changed = true;
        }
        if let Some(loading) = &self.extension_providers.loading {
            match loading.receiver.try_recv() {
                Err(TryRecvError::Empty) => (),
                result => {
                    let loading = self.extension_providers.loading.take().unwrap();
                    changed = true;
                    if loading.generation == self.extension_providers.generation
                        && self.provider_current(&loading.ticket, &loading.context)
                        && self.prompt.is_none()
                        && self.modal.is_none()
                    {
                        let result = result
                            .map_err(|_| anyhow::anyhow!("Provider navigation worker stopped"))
                            .and_then(|v| v.map_err(anyhow::Error::msg))
                            .and_then(|target| self.install_provider_target(target, loading.range));
                        if let Err(error) = result {
                            self.message = format!("Extension location: {error:#}");
                        }
                    }
                }
            }
        }
        self.poll_provider_symbol_intent() || changed
    }
    fn symbol_intent_current(&self, intent: &SymbolIntent) -> bool {
        intent.context.same_editor(self)
            && self.modal.is_none()
            && matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Symbols)
            )
            && self.active_document().is_some_and(|doc| {
                self.extension_host.as_ref().is_some_and(|host| {
                    host.symbol_source_identity(&intent.provider) == Some(intent.source)
                        && host
                            .language_provider(Kind::Symbols, doc)
                            .is_some_and(|selected| {
                                selected.id == intent.provider.id
                                    && selected.owner == intent.provider.owner
                            })
                })
            })
    }
    fn poll_provider_symbol_intent(&mut self) -> bool {
        let Some(intent) = &self.extension_providers.symbol_intent else {
            return false;
        };
        if !self.symbol_intent_current(intent) {
            self.cancel_symbols();
            self.message = "Extension symbol request expired; request symbols again".into();
            return true;
        }
        let host = self.extension_host.as_ref().unwrap();
        if !host.symbols_available() {
            self.message = if host.symbol_channel_closed() {
                "Symbols: awaiting actual timed-out callback release or extension host restart"
                    .into()
            } else {
                "Waiting for the previous extension symbol callback to finish…".into()
            };
            return false;
        }
        let intent = self.extension_providers.symbol_intent.take().unwrap();
        let host = self.extension_host.as_mut().unwrap();
        let result = host.sync_configuration(&self.settings).and_then(|()| {
            host.request_language_provider_from(
                &intent.provider,
                &self.documents,
                &self.hidden_documents,
                self.active,
                intent.extra,
            )
        });
        match result {
            Ok(ticket) => {
                self.extension_providers.lease = Some(Lease {
                    ticket,
                    context: intent.context,
                    shown: false,
                });
                self.message = "Loading extension symbols…".into();
            }
            Err(error) => {
                self.prompt = None;
                self.message = format!("Extension symbol request failed: {error:#}");
            }
        }
        true
    }
    fn provider_result(&mut self, ticket: &Ticket, value: Value) -> Result<()> {
        if self.modal.is_some() || (self.prompt.is_some() && ticket.provider.kind != Kind::Symbols)
        {
            bail!("Input context changed")
        }
        if let Some(lease) = &mut self.extension_providers.lease {
            lease.shown = true;
        }
        if ticket.provider.kind == Kind::Completion {
            return self.provider_suggestion_response(ticket, value);
        }
        if value.is_null() {
            self.message = "Extension language provider returned no results".into();
            return Ok(());
        }
        match ticket.provider.kind {
            Kind::Hover => {
                self.modal = Some(Modal::Text {
                    title: " Extension Hover · Esc closes ".into(),
                    text: language::content_text(&value["contents"]),
                    scroll: 0,
                })
            }
            Kind::Completion => unreachable!("Completion responses use suggestion routing"),
            Kind::CodeAction => unreachable!("Code actions use combined source routing"),
            Kind::Formatting => {
                let changes = provider_edits(self.doc(), serde_json::from_value(value)?)?;
                self.doc_mut().apply_changes(changes);
                self.preview_edit_barrier();
                self.message =
                    "Extension formatting applied; review and save (Undo available)".into();
                self.cancel_extension_provider();
            }
            Kind::Definition | Kind::References => {
                let values = value.as_array().context("Invalid extension locations")?;
                if values.len() > 512 {
                    bail!("Extension locations exceed 512 entries")
                }
                let mut items = Vec::new();
                for item in values {
                    let (path, range) = location(item)?;
                    items.push(language::LanguageItem {
                        label: format!(
                            "{}:{}:{}",
                            self.workspace.relative(&path),
                            range.start.line + 1,
                            range.start.character + 1
                        ),
                        action: language::LanguageAction::Provider {
                            ticket: ticket.clone(),
                            item: item.clone(),
                        },
                    });
                }
                if items.len() == 1 {
                    self.language_action(&items.remove(0).action)?;
                } else {
                    self.modal = Some(Modal::Language {
                        title: " Extension Locations · Enter opens · Esc closes ".into(),
                        items,
                        selected: 0,
                    });
                }
            }
            Kind::Signature => {
                bail!("Signature results must use their independent request controller");
            }
            Kind::Symbols => {
                let fallback = PathBuf::from(self.doc().name());
                self.extension_providers.symbols = crate::symbols::parse(
                    &value,
                    Some(self.doc().path.as_deref().unwrap_or(&fallback)),
                )?;
                self.message = format!(
                    "{} extension symbols",
                    self.extension_providers.symbols.len()
                );
            }
        }
        Ok(())
    }
    pub(super) fn apply_provider_action(&mut self, ticket: &Ticket, item: &Value) -> Result<()> {
        Context::check_budget(self)?;
        let lease = self
            .extension_providers
            .lease
            .as_ref()
            .context("Extension picker is no longer current")?;
        if lease.ticket.id != ticket.id
            || !self.provider_current(ticket, &lease.context)
            || self.prompt.is_some()
        {
            bail!("Editor context changed; request the provider again")
        }
        match ticket.provider.kind {
            Kind::Completion => {
                self.apply_completion_item(item, self.doc().cursor)?;
                self.cancel_extension_provider();
                self.message = "Extension completion applied".into();
            }
            Kind::Definition | Kind::References => {
                let (path, range) = location(item)?;
                self.open_provider_location(ticket.clone(), path, range)?;
            }
            _ => bail!("Unsupported extension picker action"),
        }
        Ok(())
    }
    fn open_provider_location(
        &mut self,
        ticket: Ticket,
        path: PathBuf,
        range: lsp::Range,
    ) -> Result<()> {
        if self.extension_providers.loading.is_some() {
            bail!("Previous extension location is still loading")
        }
        if self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .any(|d| d.path.as_ref() == Some(&path))
        {
            return self.install_provider_target(Target::Existing(path), range);
        }
        let context = Context::capture(self);
        let open: Vec<_> = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter_map(|d| d.path.clone())
            .collect();
        let bytes: usize = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .map(|d| d.text.len_bytes())
            .sum();
        let remaining = (4 * 1024 * 1024usize).saturating_sub(bytes);
        let (tx, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<Target> {
                let path = extension_services::resolved(&path, Path::new("/"))?;
                if open.contains(&path) {
                    Ok(Target::Existing(path))
                } else {
                    Ok(Target::Loaded(Box::new(Document::open_existing_bounded(
                        &path,
                        remaining as u64,
                    )?)))
                }
            })();
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        self.extension_providers.loading = Some(Loading {
            started: std::time::Instant::now(),
            generation: self.extension_providers.generation,
            ticket,
            context,
            range,
            receiver,
        });
        self.message = "Loading extension location…".into();
        Ok(())
    }
    fn install_provider_target(&mut self, target: Target, range: lsp::Range) -> Result<()> {
        let (path, loaded) = match target {
            Target::Existing(path) => (path, None),
            Target::Loaded(doc) => (doc.path.clone().context("Location has no path")?, Some(doc)),
        };
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|d| d.path.as_ref() == Some(&path))
            .or(loaded.as_deref())
            .context("Location buffer closed")?;
        let start = lsp::offset(doc, range.start)?;
        let end = lsp::offset(doc, range.end)?;
        if start > end {
            bail!("Reversed location range")
        }
        let previous = self.suspend_navigation_observation();
        if let Some(doc) = loaded {
            let mut doc = *doc;
            self.settings.apply(&mut doc);
            self.install_open_document(doc)?;
        } else {
            let result = self.open_with_intent(&path, navigation::OpenIntent::Location(range));
            self.resume_navigation_observation(
                previous,
                if result.is_ok() {
                    navigation_history::Reason::Jump
                } else {
                    navigation_history::Reason::Ordinary
                },
            );
            result?;
            self.message = "Extension location opened".into();
            return Ok(());
        }
        self.doc_mut().clear_secondary();
        self.doc_mut().move_to(start, false);
        if start != end {
            self.doc_mut().move_to(end, true);
        }
        self.resume_navigation_observation(previous, navigation_history::Reason::Jump);
        self.message = "Extension location opened".into();
        Ok(())
    }
    pub(super) fn provider_symbols_active(&self) -> bool {
        self.extension_providers.symbol_intent.is_some()
            || self
                .extension_providers
                .lease
                .as_ref()
                .is_some_and(|l| l.ticket.provider.kind == Kind::Symbols)
    }
    pub(super) fn accept_provider_symbol(&mut self, query: &str, selected: usize) {
        if self.extension_providers.symbol_intent.is_some()
            || self
                .extension_providers
                .lease
                .as_ref()
                .is_some_and(|lease| lease.ticket.provider.kind == Kind::Symbols && !lease.shown)
        {
            self.prompt = Some(Prompt::new(PromptKind::Symbols, query.into()));
            self.message = "Wait for current extension symbols".into();
            return;
        }
        let result = (|| -> Result<()> {
            let lease = self
                .extension_providers
                .lease
                .as_ref()
                .context("Symbols expired")?;
            if !self.provider_current(&lease.ticket, &lease.context) || !lease.shown {
                bail!("Wait for current extension symbols")
            }
            let symbols = self.symbol_items(query);
            let symbol = symbols
                .get(selected.min(symbols.len().saturating_sub(1)))
                .context("No matching symbols")?;
            let start = lsp::offset(self.doc(), symbol.range.start)?;
            let end = lsp::offset(self.doc(), symbol.range.end)?;
            if start > end {
                bail!("Reversed symbol range")
            }
            self.doc_mut().clear_secondary();
            self.doc_mut().move_to(start, false);
            self.observe_navigation(navigation_history::Reason::Jump);
            Ok(())
        })();
        self.cancel_extension_provider();
        if let Err(error) = result {
            self.message = format!("Extension symbols: {error:#}");
        }
    }
}
fn location(value: &Value) -> Result<(PathBuf, lsp::Range)> {
    let uri = value["uri"]
        .as_str()
        .or(value["targetUri"].as_str())
        .context("Missing location URI")?;
    let url = url::Url::parse(uri)?;
    if url.query().is_some() || url.fragment().is_some() {
        bail!("Location URI query/fragment is unsupported")
    }
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Only local file locations are supported"))?;
    let range = value
        .get("range")
        .or(value.get("targetSelectionRange"))
        .context("Missing location range")?;
    Ok((path, serde_json::from_value(range.clone())?))
}
fn provider_edits(
    doc: &Document,
    mut edits: Vec<lsp::TextEdit>,
) -> Result<Vec<(std::ops::Range<usize>, String)>> {
    if edits.len() > 4096 {
        bail!("Extension edit count exceeds 4096")
    }
    let mut bytes = 0usize;
    for edit in &mut edits {
        edit.new_text = edit
            .new_text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', &doc.eol);
        bytes = bytes
            .checked_add(edit.new_text.len())
            .context("Edit byte count overflow")?;
        if bytes > 1024 * 1024 {
            bail!("Normalized extension edits exceed 1 MiB")
        }
    }
    lsp::edits(doc, edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn until(app: &mut App, check: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if check(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn app(root: &Path) -> App {
        std::fs::write(root.join("input.sql"), "sel 🙂\r\nfrom table;\r\n").unwrap();
        std::fs::write(root.join("target.sql"), "α🙂 target\r\n").unwrap();
        let mut app = App::new(root.into(), Profile::Linux);
        app.open(&root.join("input.sql")).unwrap();
        let package = crate::extensions::Package::read(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/language-provider-extension"),
        )
        .unwrap();
        app.start_extension_packages(vec![package]).unwrap();
        until(&mut app, |a| a.has_extension_provider(Kind::Completion));
        app
    }
    fn request(app: &mut App, kind: Kind) {
        app.language_request(kind.method(), json!({}));
        until(app, |a| {
            if kind == Kind::Signature {
                return a.signature_help().is_some() || a.message.contains("response:");
            }
            a.extension_providers
                .lease
                .as_ref()
                .is_some_and(|l| l.shown)
                || a.message.contains("response:")
        });
        assert!(!app.message.contains("response:"), "{}", app.message);
    }
    fn held_symbol_app(root: &Path) -> (App, Ticket) {
        std::fs::write(root.join("input.sql"), "sel 🙂\r\nfrom table;\r\n").unwrap();
        let package = root.join("provider");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(package.join("package.json"), r#"{"publisher":"vscli-tests","name":"held-symbol-picker","version":"1.0.0","main":"index.cjs","engines":{"vscode":"^1.95.0"}}"#).unwrap();
        let root_json = serde_json::to_string(root).unwrap();
        std::fs::write(package.join("index.cjs"), format!(r#"
const vscode=require('vscode'),fs=require('node:fs'),path=require('node:path'),root={root_json};
let count=0;
exports.activate=context=>context.subscriptions.push(vscode.languages.registerDocumentSymbolProvider('*',{{
 provideDocumentSymbols(){{
  count++;fs.appendFileSync(path.join(root,'entered'),'x');
  const symbols=[new vscode.DocumentSymbol('query','',vscode.SymbolKind.Function,new vscode.Range(0,0,0,3),new vscode.Range(0,0,0,3))];
  if(count>1)return symbols;
  return new Promise(resolve=>{{const timer=setInterval(()=>{{if(fs.existsSync(path.join(root,'release'))){{clearInterval(timer);resolve(symbols);}}}},2);}});
 }}
}}));
"#)).unwrap();
        let mut app = App::new(root.into(), Profile::Linux);
        app.open(&root.join("input.sql")).unwrap();
        app.start_extension_packages(vec![crate::extensions::Package::read(&package).unwrap()])
            .unwrap();
        until(&mut app, |a| a.has_extension_provider(Kind::Symbols));
        let provider = app
            .extension_host
            .as_ref()
            .unwrap()
            .language_provider(Kind::Symbols, app.doc())
            .unwrap()
            .clone();
        let ticket = app
            .extension_host
            .as_mut()
            .unwrap()
            .request_language_provider_from(
                &provider,
                &app.documents,
                &app.hidden_documents,
                app.active,
                json!({}),
            )
            .unwrap();
        until(&mut app, |_| root.join("entered").exists());
        app.extension_host
            .as_mut()
            .unwrap()
            .cancel_language_provider(&ticket)
            .unwrap();
        (app, ticket)
    }
    fn outline_symbol_app(root: &Path) -> App {
        document_symbol_app(root, true)
    }
    fn document_symbol_app(root: &Path, focus_outline: bool) -> App {
        let text = "猫🙂 parent\r\n  child 猫\r\n}\r\n";
        std::fs::write(root.join("input.sql"), text).unwrap();
        let package = root.join("outline-provider");
        std::fs::create_dir(&package).unwrap();
        std::fs::write(package.join("package.json"), r#"{"publisher":"vscli-tests","name":"outline-publication","version":"1.0.0","main":"index.cjs","engines":{"vscode":"^1.95.0"}}"#).unwrap();
        let root_json = serde_json::to_string(root).unwrap();
        std::fs::write(package.join("index.cjs"),format!(r#"
const vscode=require('vscode'),fs=require('node:fs'),path=require('node:path'),root={root_json};
let generation=1,registration;
function register(context){{
 registration=vscode.languages.registerDocumentSymbolProvider('*',{{provideDocumentSymbols(document){{
  fs.appendFileSync(path.join(root,'uris'),document.uri.toString()+'\n');
  const parent=new vscode.DocumentSymbol('parent-'+generation,'class detail',vscode.SymbolKind.Class,new vscode.Range(0,0,2,1),new vscode.Range(0,4,0,10));
  const child=new vscode.DocumentSymbol('child-'+generation,'method detail',vscode.SymbolKind.Method,new vscode.Range(1,0,1,9),new vscode.Range(1,2,1,7));
  child.tags=[vscode.SymbolTag.Deprecated];parent.children.push(child);
  if(generation===1)return [parent];
  return new Promise((resolve,reject)=>{{
   const timer=setInterval(()=>{{if(fs.existsSync(path.join(root,'new-release'))){{clearInterval(timer);clearTimeout(deadline);resolve([parent]);}}}},2);
   const deadline=setTimeout(()=>{{clearInterval(timer);reject(new Error('document-symbol fixture release was not acknowledged within 30 seconds'));}},30000);
  }});
 }}}});context.subscriptions.push(registration);
}}
exports.activate=context=>{{register(context);context.subscriptions.push(vscode.commands.registerCommand('test.symbols.retire',()=>{{registration.dispose();generation++;register(context);}}));}};
"#)).unwrap();
        let mut app = App::new(root.into(), Profile::Linux);
        app.open(&root.join("input.sql")).unwrap();
        let end = app.doc().text.len_chars();
        app.doc_mut().move_to(end, false);
        app.doc_mut().insert("Δ", false);
        app.start_extension_packages(vec![crate::extensions::Package::read(&package).unwrap()])
            .unwrap();
        until(&mut app, |a| a.has_extension_provider(Kind::Symbols));
        if focus_outline {
            app.execute("outline.focus", Value::Null);
        } else {
            app.sidebar = false;
        }
        until(&mut app, |a| a.document_symbols_view().current);
        app
    }
    fn reveal_outline_child(app: &mut App) {
        app.execute("outline.focus", Value::Null);
        app.event(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)));
        app.event(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        assert_eq!(app.outline_view().visible.len(), 1);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.outline_view().visible.len(), 2);
        app.event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.focus == Focus::Editor);
        assert_eq!(
            app.doc().cursor,
            13,
            "UTF-16 child start maps to a collapsed scalar caret"
        );
        assert!(app.doc().anchor.is_none());
        assert!(app.doc().secondary.is_empty());
    }
    #[test]
    fn extension_outline_publication_hierarchy_and_untitled_keyboard_reveal_preserve_crlf_history()
    {
        let root = tempfile::tempdir().unwrap();
        let mut app = outline_symbol_app(root.path());
        let original = "猫🙂 parent\r\n  child 猫\r\n}\r\n";
        let dirty = format!("{original}Δ");
        let file_id = app.doc().id;
        assert!(app.lsp.is_none());
        assert!(app.doc().dirty());
        let view = app.outline_view();
        assert_eq!(view.status, super::super::OutlineStatus::Ready);
        assert!(view.source.contains("outline-publication"));
        let tree = view.tree.unwrap();
        assert_eq!(tree.nodes.len(), 2);
        assert_eq!(tree.nodes[1].parent, Some(0));
        assert_eq!(tree.nodes[0].name, "parent-1");
        assert_eq!(tree.nodes[1].detail, "method detail");
        assert!(tree.nodes[1].deprecated);
        app.doc_mut().add_cursor(1);
        reveal_outline_child(&mut app);
        assert_eq!(app.doc().id, file_id);
        assert_eq!(app.doc().text.to_string(), dirty);
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            original.as_bytes()
        );
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), original);
        assert!(
            !app.outline_view().actionable,
            "edit→Undo requires a fresh Outline proof"
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), dirty);
        app.doc_mut().save().unwrap();
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            dirty.as_bytes()
        );
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert(&dirty, false);
        let untitled = app.doc().id;
        until(&mut app, |a| a.outline_view().actionable);
        assert!(app.doc().path.is_none());
        assert_ne!(untitled, file_id);
        assert_eq!(app.outline_view().tree.unwrap().nodes[1].parent, Some(0));
        app.doc_mut().add_cursor(1);
        reveal_outline_child(&mut app);
        assert_eq!(app.doc().id, untitled);
        assert_eq!(app.doc().text.to_string(), dirty);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "");
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), dirty);
        assert!(
            std::fs::read_to_string(root.path().join("uris"))
                .unwrap()
                .contains(&format!("untitled:vscli-{untitled}"))
        );
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            dirty.as_bytes()
        );
    }
    #[test]
    fn extension_outline_owner_source_round_trip_retires_published_tree_until_new_callback_settles()
    {
        let root = tempfile::tempdir().unwrap();
        let mut app = outline_symbol_app(root.path());
        let original = app.doc().text.to_string();
        let source = app
            .extension_host
            .as_ref()
            .unwrap()
            .language_provider(Kind::Symbols, app.doc())
            .unwrap()
            .id;
        app.extension_host
            .as_mut()
            .unwrap()
            .execute_with_hidden(
                "test.symbols.retire",
                None,
                &app.documents,
                &app.hidden_documents,
                app.active,
                &app.settings,
            )
            .unwrap();
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .unwrap()
                .language_provider(Kind::Symbols, a.doc())
                .is_some_and(|provider| provider.id != source)
        });
        assert!(!app.outline_view().actionable);
        assert!(app.outline_view().tree.is_none());
        let selections = app.doc().selections();
        app.execute("outline.focus", Value::Null);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.focus == Focus::Outline);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text.to_string(), original);
        assert!(app.lsp.is_none());
        std::fs::write(root.path().join("new-release"), "").unwrap();
        until(&mut app, |a| a.outline_view().actionable);
        assert_eq!(app.outline_view().tree.unwrap().nodes[0].name, "parent-2");
        assert!(app.outline_view().source.contains("outline-publication"));
        reveal_outline_child(&mut app);
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            "猫🙂 parent\r\n  child 猫\r\n}\r\n".as_bytes()
        );
    }
    #[test]
    fn explicit_extension_symbol_picker_from_outline_keeps_one_latest_intent_until_actual_release()
    {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _held) = held_symbol_app(root.path());
        let original = app.doc().text.to_string();
        let document = app.doc().id;
        for _ in 0..32 {
            app.focus = Focus::Outline;
            app.start_symbols(false);
            assert!(app.focus == Focus::Editor);
            assert!(app.extension_providers.symbol_intent.is_some());
            assert!(app.extension_providers.lease.is_none());
            assert!(app.provider_symbols_active());
            assert!(
                !app.native_symbol_picker_waiting(),
                "must not fall back to native symbols"
            );
            app.poll();
        }
        for character in "query".chars() {
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )));
        }
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.extension_providers.symbol_intent.is_some());
        assert!(matches!(
            app.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Symbols)
        ));
        assert_eq!(std::fs::read(root.path().join("entered")).unwrap(), b"x");
        std::fs::write(root.path().join("release"), "").unwrap();
        until(&mut app, |a| !a.extension_providers.symbols.is_empty());
        assert!(app.extension_providers.symbol_intent.is_none());
        assert_eq!(std::fs::read(root.path().join("entered")).unwrap(), b"xx");
        assert_eq!(app.symbol_items("query").len(), 1);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.doc().id, document);
        assert_eq!(app.doc().cursor, 0);
        assert_eq!(app.doc().text.to_string(), original);
        assert!(app.prompt.is_none());
    }
    #[test]
    fn queued_extension_symbol_picker_edit_undo_retires_without_dispatching_or_fallback() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _held) = held_symbol_app(root.path());
        let original = app.doc().text.to_string();
        app.start_symbols(false);
        assert!(app.extension_providers.symbol_intent.is_some());
        app.doc_mut().insert("dirty", false);
        app.doc_mut().undo();
        app.poll();
        assert!(app.extension_providers.symbol_intent.is_none());
        assert!(app.prompt.is_none());
        std::fs::write(root.path().join("release"), "").unwrap();
        until(&mut app, |a| {
            a.extension_host.as_ref().unwrap().symbols_available()
        });
        assert_eq!(std::fs::read(root.path().join("entered")).unwrap(), b"x");
        assert!(!app.native_symbol_picker_waiting());
        assert!(app.extension_providers.symbols.is_empty());
        assert_eq!(app.doc().text.to_string(), original);
    }
    fn completion_resolve_app(root: &Path) -> App {
        std::fs::write(root.join("input.sql"), "sel 🙂\r\nfrom table;\r\n").unwrap();
        let mut app = App::new(root.into(), Profile::Linux);
        app.open(&root.join("input.sql")).unwrap();
        let package = crate::extensions::Package::read(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/completion-resolve-extension"),
        )
        .unwrap();
        app.start_extension_packages(vec![package]).unwrap();
        until(&mut app, |a| a.has_extension_provider(Kind::Completion));
        app
    }
    #[test]
    fn resolved_extension_snippet_and_import_are_one_unicode_crlf_undo_transaction() {
        let root = tempfile::tempdir().unwrap();
        let mut app = completion_resolve_app(root.path());
        let original = app.doc().text.to_string();
        let id = app.doc().id;
        app.doc_mut().move_to(3, false);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::CONTROL,
        )));
        until(&mut app, |a| {
            a.suggestion_model().is_some_and(|m| {
                m.item(0)
                    .is_some_and(|i| i.value["detail"] == "Resolved import and snippet")
            })
        });
        app.event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        until(&mut app, |a| {
            a.doc().text == "SELECT 猫 🙂\r\n-- import 界\r\nfrom table;\r\n"
        });
        assert_eq!(app.doc().id, id);
        assert!(app.doc().in_snippet());
        assert_eq!(app.doc().selected_text().as_deref(), Some("猫"));
        app.doc_mut().save().unwrap();
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            app.doc().text.to_string().as_bytes()
        );
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().id, id);
        app.doc_mut().save().unwrap();
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            original.as_bytes()
        );
    }
    #[test]
    fn held_extension_resolve_cannot_apply_after_edit_undo_same_revision() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("hold-resolve"), b"hold").unwrap();
        let mut app = completion_resolve_app(root.path());
        let original = app.doc().text.to_string();
        let id = app.doc().id;
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::CONTROL,
        )));
        until(&mut app, |_| root.path().join("resolve-started").exists());
        let revision = app.doc().revision;
        let epoch = app.doc().text_epoch();
        app.doc_mut().insert("transient", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().revision, revision);
        assert_ne!(app.doc().text_epoch(), epoch);
        app.poll();
        std::fs::remove_file(root.path().join("hold-resolve")).unwrap();
        until(&mut app, |a| {
            a.extension_providers.completion_resolves.is_empty()
        });
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().id, id);
        assert!(!app.doc().in_snippet());
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            original.as_bytes()
        );
    }
    #[test]
    fn immediate_dispatch_synchronizes_new_configuration_before_callback() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let id = app.doc().id;
        let before = app.doc().text.to_string();
        app.settings = crate::settings::Settings::from_values(
            serde_json::from_value(json!({"fixture.formatted":"CURRENT 🙂\n"})).unwrap(),
            "immediate configuration test",
        )
        .unwrap();
        // No App::poll between replacing settings and issuing the provider RPC.
        app.language_request(Kind::Formatting.method(), json!({}));
        until(&mut app, |a| a.doc().text == "CURRENT 🙂\r\n");
        assert_eq!(app.doc().id, id);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), before);
    }
    #[test]
    fn automatic_completion_uses_active_provider_and_tab_preserves_native_undo() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let id = app.doc().id;
        app.doc_mut().move_to(3, true);
        app.doc_mut().insert("", false);
        for character in "sel".chars() {
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )));
        }
        until(&mut app, App::suggestion_acceptable);
        assert!(app.modal.is_none());
        app.event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "SELECT 🙂\r\nfrom table;\r\n");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "sel 🙂\r\nfrom table;\r\n");
    }
    #[test]
    fn rejected_popup_edit_retires_its_provider_lease_without_changing_bytes_or_undo() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        app.doc_mut().move_to(3, false);
        app.doc_mut().insert("x", false);
        app.doc_mut().undo();
        let id = app.doc().id;
        let before = app.doc().text.to_string();
        let selections = app.doc().selections();
        app.settings = crate::settings::Settings::from_values(
            serde_json::from_value(json!({"fixture.badCompletion":true})).unwrap(),
            "invalid popup completion fixture",
        )
        .unwrap();
        request(&mut app, Kind::Completion);
        assert!(app.suggestion_acceptable());
        app.event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert!(
            app.message.starts_with("Suggestion rejected:"),
            "{}",
            app.message
        );
        assert!(app.extension_providers.lease.is_none());
        assert!(app.suggestion_model().is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), before);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            before.as_bytes()
        );
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "selx 🙂\r\nfrom table;\r\n");
    }
    #[test]
    fn all_seven_native_provider_workflows_keep_crlf_identity_and_undo() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let id = app.doc().id;
        app.doc_mut().move_to(3, false);
        request(&mut app, Kind::Completion);
        assert!(app.suggestion_model().is_some());
        app.event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.doc().text.to_string(),
            "SELECT 🙂\r\nfrom table;\r\n",
            "{}",
            app.message
        );
        assert_eq!(app.doc().id, id);
        app.doc_mut().undo();
        request(&mut app, Kind::Hover);
        assert!(
            matches!(&app.modal,Some(Modal::Text { text, .. }) if text.contains("Native provider hover"))
        );
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        request(&mut app, Kind::Signature);
        assert_eq!(app.signature_help().unwrap().label, "query(🙂value)");
        app.clear_signature();
        app.start_symbols(false);
        until(&mut app, |a| !a.extension_providers.symbols.is_empty());
        assert!(app.symbol_items("query")[0].label.contains("Function"));
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.doc().cursor, 0, "{}", app.message);
        request(&mut app, Kind::Definition);
        until(&mut app, |a| {
            a.doc()
                .path
                .as_ref()
                .is_some_and(|p| p.ends_with("target.sql"))
        });
        assert_eq!(app.doc().selected_text().as_deref(), Some("🙂"));
        let target_id = app.doc().id;
        let end = app.doc().text.len_chars();
        app.doc_mut().move_to(end, false);
        app.doc_mut().insert("猫", false);
        std::fs::remove_file(root.path().join("target.sql")).unwrap();
        app.open(&root.path().join("input.sql")).unwrap();
        // References reuse the deleted, dirty target identity without reading disk.
        request(&mut app, Kind::References);
        until(&mut app, |a| a.extension_providers.loading.is_none());
        assert_eq!(app.doc().id, target_id);
        assert!(app.documents.iter().any(|d| d.id == target_id && d.dirty()));
        app.open(&root.path().join("input.sql")).unwrap();
        app.language_request(
            Kind::Formatting.method(),
            json!({"options":{"tabSize":2,"insertSpaces":true}}),
        );
        until(&mut app, |a| {
            a.message.starts_with("Extension formatting applied")
        });
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "SELECT 🙂\r\nFROM table;\r\n");
        app.doc_mut().save().unwrap();
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            b"SELECT \xf0\x9f\x99\x82\r\nFROM table;\r\n"
        );
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "sel 🙂\r\nfrom table;\r\n");
    }
    #[test]
    fn stale_picker_and_malformed_edit_cannot_change_native_text_or_selections() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        request(&mut app, Kind::Completion);
        let item = app
            .suggestion_model()
            .unwrap()
            .item(0)
            .unwrap()
            .value
            .clone();
        let ticket = app.completion_provider_ticket().unwrap();
        let action = LanguageAction::Provider { ticket, item };
        app.doc_mut().insert("newer ", false);
        let before = app.doc().text.to_string();
        let selections = app.doc().selections();
        assert!(app.language_action(&action).is_err());
        assert_eq!(app.doc().text.to_string(), before);
        assert_eq!(app.doc().selections(), selections);
        app.doc_mut().undo();
        request(&mut app, Kind::Completion);
        let ticket = app
            .extension_providers
            .lease
            .as_ref()
            .unwrap()
            .ticket
            .clone();
        app.modal = None;
        for item in [
            json!({"label":"snippet","insertText":"${CLIPBOARD}","insertTextFormat":2}),
            json!({"label":"bad","textEdit":{"range":{"start":{"line":0,"character":5},"end":{"line":0,"character":6}},"newText":"x"}}),
        ] {
            let before = app.doc().text.to_string();
            let selections = app.doc().selections();
            assert!(app.apply_provider_action(&ticket, &item).is_err());
            assert_eq!(app.doc().text.to_string(), before);
            assert_eq!(app.doc().selections(), selections);
        }
    }
    #[test]
    fn original_signature_dismiss_binding_resolves_before_context_expiry() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        request(&mut app, Kind::Signature);
        assert!(app.signature_help().is_some());
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::SHIFT)));
        assert!(app.signature_help().is_none());
        assert!(
            app.extension_providers.lease.is_none(),
            "Shift+Escape must execute the native close command, not only hide a stale hint"
        );
    }
    #[test]
    fn untitled_provider_symbols_filter_and_navigate_without_creating_a_file() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("sel 🙂\r\n", false);
        let id = app.doc().id;
        let before = app.doc().text.to_string();
        app.start_symbols(false);
        until(&mut app, |a| !a.extension_providers.symbols.is_empty());
        assert!(app.symbol_items("query")[0].label.contains("Untitled"));
        for character in "query".chars() {
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )));
        }
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.doc().id, id);
        assert!(app.doc().path.is_none());
        assert_eq!(app.doc().cursor, 0);
        assert_eq!(app.doc().text.to_string(), before);
        assert!(app.prompt.is_none());
        assert!(!root.path().join("Untitled").exists());
    }
    #[test]
    fn provider_requests_and_picker_acceptance_reject_aggregate_selection_overflow() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let disk = std::fs::read(root.path().join("input.sql")).unwrap();
        app.doc_mut().insert("dirty ", false);
        for _ in 0..2 {
            let mut hidden = Document::default();
            hidden.secondary = vec![crate::document::Selection::caret(0); 8190];
            app.hidden_documents.push(hidden);
        }
        // Two hidden snapshots of 8191 + active document/pane of 1 = 16384.
        request(&mut app, Kind::Completion);
        let item = app
            .suggestion_model()
            .unwrap()
            .item(0)
            .unwrap()
            .value
            .clone();
        let ticket = app.completion_provider_ticket().unwrap();
        let action = LanguageAction::Provider { ticket, item };
        let id = app.doc().id;
        let revision = app.doc().revision;
        let text = app.doc().text.to_string();
        app.hidden_documents[1]
            .secondary
            .push(crate::document::Selection::caret(0));
        let error = app.language_action(&action).unwrap_err();
        assert!(error.to_string().contains("16,384"));
        assert!(app.extension_language_request(Kind::Completion.method(), json!({})));
        assert!(app.message.contains("16,384"));
        assert!(app.extension_providers.lease.is_none());
        assert!(app.prompt.is_none() && app.modal.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().text.to_string(), text);
        assert!(app.doc().dirty());
        app.doc_mut().undo();
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string().as_bytes(), disk);
        assert_eq!(std::fs::read(root.path().join("input.sql")).unwrap(), disk);
    }
    #[test]
    fn provider_leases_reject_hidden_edits_and_inactive_view_changes() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        app.hidden_documents
            .push(Document::from_text("hidden unsaved"));
        app.split_editor(false);
        request(&mut app, Kind::Completion);
        let item = app
            .suggestion_model()
            .unwrap()
            .item(0)
            .unwrap()
            .value
            .clone();
        let ticket = app.completion_provider_ticket().unwrap();
        let action = LanguageAction::Provider { ticket, item };
        let original = app.doc().text.to_string();
        let active = app.active_pane;
        let other = app.panes[0].id;
        let current = app.panes[active].id;
        app.doc_mut().activate_view(other);
        app.doc_mut().move_to(2, false);
        app.doc_mut().activate_view(current);
        assert!(app.language_action(&action).is_err());
        assert_eq!(app.doc().text.to_string(), original);
        request(&mut app, Kind::Completion);
        let item = app
            .suggestion_model()
            .unwrap()
            .item(0)
            .unwrap()
            .value
            .clone();
        let ticket = app.completion_provider_ticket().unwrap();
        let action = LanguageAction::Provider { ticket, item };
        app.hidden_documents[0].insert("newer ", false);
        assert!(app.language_action(&action).is_err());
        assert_eq!(app.doc().text.to_string(), original);
        request(&mut app, Kind::Completion);
        let before = app.doc().view_state(Some(other)).clone();
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let after = app.doc().view_state(Some(other));
        // Shared view positions map with the accepted text edit; no secondary
        // cursor is adopted into the active view and the other view remains valid.
        assert_eq!(app.doc().selections().len(), 1);
        assert!(after.cursor >= before.cursor);
        assert_eq!(app.doc().text.to_string(), "SELECT 🙂\r\nfrom table;\r\n");
    }
    #[test]
    fn canceled_location_loader_retains_its_slot_and_never_installs_a_late_target() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        request(&mut app, Kind::Hover);
        app.modal = None;
        let ticket = app
            .extension_providers
            .lease
            .as_ref()
            .unwrap()
            .ticket
            .clone();
        let context = Context::capture(&app);
        let (sender, receiver) = mpsc::sync_channel(1);
        let range = lsp::Range {
            start: lsp::Position {
                line: 0,
                character: 1,
            },
            end: lsp::Position {
                line: 0,
                character: 3,
            },
        };
        app.extension_providers.loading = Some(Loading {
            ticket: ticket.clone(),
            context,
            range,
            receiver,
            generation: app.extension_providers.generation,
            started: std::time::Instant::now(),
        });
        let source = app.doc().id;
        app.cancel_extension_provider();
        assert!(
            app.open_provider_location(ticket.clone(), root.path().join("target.sql"), range)
                .is_err()
        );
        app.poll_extension_providers();
        assert!(app.extension_providers.loading.is_some());
        sender
            .send(Ok(Target::Loaded(Box::new(
                Document::open_existing(&root.path().join("target.sql")).unwrap(),
            ))))
            .unwrap();
        app.poll_extension_providers();
        assert!(app.extension_providers.loading.is_none());
        assert_eq!(app.doc().id, source);
        assert_eq!(app.documents.len(), 1);
        // Invalid UTF-16 targets are validated before making another tab visible.
        let invalid = lsp::Range {
            start: lsp::Position {
                line: 0,
                character: 2,
            },
            end: range.end,
        };
        assert!(
            app.install_provider_target(
                Target::Loaded(Box::new(
                    Document::open_existing(&root.path().join("target.sql")).unwrap()
                )),
                invalid
            )
            .is_err()
        );
        assert_eq!(app.doc().id, source);
        assert_eq!(app.documents.len(), 1);
    }
    #[test]
    fn normalization_budget_and_overlaps_are_validated_before_mutation() {
        let mut doc = Document::from_text("α🙂\r\n");
        doc.eol = "\r\n".into();
        let range = lsp::Range {
            start: lsp::Position {
                line: 0,
                character: 0,
            },
            end: lsp::Position {
                line: 0,
                character: 3,
            },
        };
        let edit = |text: String| lsp::TextEdit {
            range,
            new_text: text,
        };
        assert!(provider_edits(&doc, vec![edit("\n".repeat(524289))]).is_err());
        assert!(provider_edits(&doc, vec![edit("x".into()), edit("y".into())]).is_err());
        let changes = provider_edits(&doc, vec![edit("a\nb\rc\r\nd".into())]).unwrap();
        assert_eq!(changes[0].1, "a\r\nb\r\nc\r\nd");
        assert_eq!(doc.text.to_string(), "α🙂\r\n");
    }
    #[test]
    fn default_breadcrumbs_extension_publication_and_reveal_need_no_outline_sidebar_or_native_server()
     {
        let root = tempfile::tempdir().unwrap();
        let mut app = document_symbol_app(root.path(), false);
        let id = app.doc().id;
        let original = "猫🙂 parent\r\n  child 猫\r\n}\r\n";
        let dirty = format!("{original}Δ");
        assert!(!app.sidebar);
        assert!(app.lsp.is_none());
        assert_eq!(
            app.outline_view().status,
            super::super::OutlineStatus::Hidden
        );
        app.doc_mut().add_cursor(1);
        app.doc_mut().move_to(13, false);
        app.observe_outline();
        app.observe_breadcrumbs();
        assert_eq!(
            app.breadcrumbs_view()
                .elements
                .iter()
                .filter(|item| item.node.is_some())
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["parent-1", "child-1"]
        );
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('.'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )));
        assert_eq!(
            app.breadcrumbs_view().picker.as_ref().unwrap().rows[0].label,
            "child-1"
        );
        let generation = app.breadcrumbs_view().generation;
        app.event(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('.'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyEventKind::Release,
        )));
        assert!(app.focus == Focus::Breadcrumbs);
        assert!(app.breadcrumbs_view().picker.is_some());
        assert_eq!(app.breadcrumbs_view().generation, generation);
        let selections = app.doc().selections();
        let epoch = app.doc().text_epoch();
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));
        assert!(app.message.contains("filtering is not supported"));
        app.event(Event::Paste("unsafe edit\r\n猫".into()));
        assert!(app.message.contains("filtering is not supported"));
        assert!(app.focus == Focus::Breadcrumbs);
        assert!(app.breadcrumbs_view().picker.is_some());
        assert_eq!(app.breadcrumbs_view().generation, generation);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), dirty);
        app.execute("list.select", Value::Null);
        assert!(app.focus == Focus::Editor);
        assert_eq!(app.doc().cursor, 13);
        assert!(app.doc().anchor.is_none());
        assert!(app.doc().secondary.is_empty());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), dirty);
        assert!(app.doc().dirty());
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            original.as_bytes()
        );
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), original);
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), dirty);
    }
    #[test]
    fn breadcrumb_extension_source_round_trip_retires_picker_and_retains_one_actual_lane() {
        fn phase(message: &str) {
            use std::io::Write;
            // Direct stderr is visible even while libtest captures a held test.
            let _ = writeln!(
                std::io::stderr().lock(),
                "breadcrumb source round trip: {message}"
            );
        }
        struct ReleaseHeldCallback(PathBuf);
        impl Drop for ReleaseHeldCallback {
            fn drop(&mut self) {
                // On assertion failure, release the real callback before App's
                // host teardown. Preserve the failing assertion as the oracle.
                let _ = std::fs::write(&self.0, "");
            }
        }
        let root = tempfile::tempdir().unwrap();
        phase("starting document-symbol host");
        let mut app = document_symbol_app(root.path(), false);
        let _release_on_unwind = ReleaseHeldCallback(root.path().join("new-release"));
        phase("initial tree published");
        app.doc_mut().move_to(13, false);
        app.observe_outline();
        app.observe_breadcrumbs();
        app.execute("breadcrumbs.focusAndSelect", Value::Null);
        assert!(app.breadcrumbs_view().picker.is_some());
        let original = app.doc().text.to_string();
        let selections = app.doc().selections();
        let id = app.doc().id;
        let epoch = app.doc().text_epoch();
        let old = app
            .extension_host
            .as_ref()
            .unwrap()
            .language_provider(Kind::Symbols, app.doc())
            .unwrap()
            .id;
        app.extension_host
            .as_mut()
            .unwrap()
            .execute_with_hidden(
                "test.symbols.retire",
                None,
                &app.documents,
                &app.hidden_documents,
                app.active,
                &app.settings,
            )
            .unwrap();
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .unwrap()
                .language_provider(Kind::Symbols, a.doc())
                .is_some_and(|provider| provider.id != old)
        });
        until(&mut app, |_| {
            std::fs::read_to_string(root.path().join("uris"))
                .unwrap()
                .lines()
                .count()
                == 2
        });
        phase("replacement callback is held");
        assert!(app.breadcrumbs_view().picker.is_none());
        assert!(!app.document_symbols_view().current);
        assert!(
            app.breadcrumbs_view()
                .elements
                .iter()
                .all(|element| element.node.is_none())
        );
        app.breadcrumbs_picker_click(0);
        app.execute("breadcrumbs.revealFocused", Value::Null);
        assert_eq!(app.doc().selections(), selections);
        let second = app
            .extension_host
            .as_ref()
            .unwrap()
            .language_provider(Kind::Symbols, app.doc())
            .unwrap()
            .id;
        app.extension_host
            .as_mut()
            .unwrap()
            .execute_with_hidden(
                "test.symbols.retire",
                None,
                &app.documents,
                &app.hidden_documents,
                app.active,
                &app.settings,
            )
            .unwrap();
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .unwrap()
                .language_provider(Kind::Symbols, a.doc())
                .is_some_and(|provider| provider.id != second)
        });
        for _ in 0..32 {
            app.poll();
            assert!(!app.extension_host.as_ref().unwrap().symbols_available());
            assert!(app.lsp.is_none());
            assert!(app.breadcrumbs_view().picker.is_none());
            assert_eq!(
                std::fs::read_to_string(root.path().join("uris"))
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
            assert_eq!(app.doc().selections(), selections);
        }
        phase("retiring held source and releasing callback");
        std::fs::write(root.path().join("new-release"), "").unwrap();
        until(&mut app, |a| a.document_symbols_view().current);
        assert_eq!(
            app.document_symbols_view().tree.unwrap().nodes[0].name,
            "parent-3"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("uris"))
                .unwrap()
                .lines()
                .count(),
            3
        );
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(
            std::fs::read(root.path().join("input.sql")).unwrap(),
            "猫🙂 parent\r\n  child 猫\r\n}\r\n".as_bytes()
        );
        // Current publication may precede the exact actual-release event. The
        // fixture must acknowledge that event before retiring its process.
        until(&mut app, |a| {
            a.extension_host.as_ref().unwrap().symbols_available()
        });
        phase("actual callback released; dropping host");
        drop(app);
        phase("host dropped");
    }
}
