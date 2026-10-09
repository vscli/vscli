use super::extension_services::Context;
use super::*;
use crate::{extensions::providers::Ticket, suggestions::Model};
use anyhow::{Context as _, bail};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

enum Awaiting {
    Native { instance: Arc<()>, token: u64 },
    Provider(Ticket),
}
#[derive(Clone)]
enum Source {
    Native {
        instance: Arc<()>,
        request: crate::lsp::Request,
    },
    Provider(Ticket),
}
struct Pending {
    settings: crate::settings::Settings,
    origin: Awaiting,
    context: Context,
    started: Instant,
}
struct Popup {
    model: Model,
    source: Source,
    context: Context,
    settings: crate::settings::Settings,
}
enum ResolveOrigin {
    Native { instance: Arc<()>, token: u64 },
    Provider(Ticket),
}
struct Resolving {
    origin: ResolveOrigin,
    context: Context,
    original: Value,
    started: Instant,
}
enum Fence {
    Native(Arc<()>),
    Provider(Ticket),
}
struct Cached {
    source: Fence,
    model: Model,
    context: Context,
    started: Instant,
}
struct Queued {
    context: Context,
    due: Instant,
    started: Instant,
    extra: Value,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    popup: Option<Popup>,
    cached: Option<Cached>,
    queued: Option<Queued>,
    resolving: Option<Resolving>,
    accept_after_resolve: Option<Value>,
    resolve_due: Option<Instant>,
}
pub(super) fn typing_command(command: &str) -> bool {
    matches!(command, "type" | "deleteLeft" | "deleteRight")
}
pub(super) fn command(command: &str) -> bool {
    matches!(
        command,
        "acceptSelectedSuggestion"
            | "hideSuggestWidget"
            | "selectNextSuggestion"
            | "selectPrevSuggestion"
            | "selectNextPageSuggestion"
            | "selectPrevPageSuggestion"
    )
}
fn prefix(app: &App) -> Result<String> {
    let doc = app.active_document().context("No active editor")?;
    let mut start = doc.cursor;
    let mut characters = doc.text.chars_at(doc.cursor);
    let mut bytes = 0;
    while let Some(character) = characters.prev() {
        if !character.is_alphanumeric() && character != '_' {
            break;
        }
        start -= 1;
        bytes += character.len_utf8();
        if bytes > crate::suggestions::MAX_PREFIX {
            bail!("Completion prefix exceeds 1 KiB");
        }
    }
    Ok(doc.text.slice(start..doc.cursor).to_string())
}
impl App {
    fn suggestion_context(&self) -> Result<Context> {
        if self.prompt.is_some()
            || self.modal.is_some()
            || self.focus != Focus::Editor
            || self.active_document().is_none()
        {
            bail!("Focus the editor before requesting suggestions");
        }
        Context::check_budget(self)?;
        Ok(Context::capture(self))
    }
    fn suggestion_source_current(&self, source: &Source) -> bool {
        match source {
            Source::Native { instance, .. } => self
                .lsp
                .as_ref()
                .is_some_and(|client| Arc::ptr_eq(instance, &client.identity())),
            Source::Provider(ticket) => self.active_document().is_some_and(|doc| {
                self.extension_host
                    .as_ref()
                    .is_some_and(|host| host.provider_ticket_current(ticket, doc))
            }),
        }
    }
    fn cached_source_current(&self, source: &Fence) -> bool {
        match source {
            Fence::Native(instance) => self
                .lsp
                .as_ref()
                .is_some_and(|client| Arc::ptr_eq(instance, &client.identity())),
            Fence::Provider(ticket) => self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.provider_registration_current(ticket)),
        }
    }
    pub fn suggestion_acceptable(&self) -> bool {
        self.suggestions.popup.as_ref().is_some_and(|popup| {
            !popup.model.is_empty()
                && popup.context.same_editor(self)
                && popup.settings == self.settings
                && self.suggestion_source_current(&popup.source)
                && self.prompt.is_none()
                && self.modal.is_none()
                && self.focus == Focus::Editor
        })
    }
    pub fn suggestion_model(&self) -> Option<&Model> {
        if self.suggestion_acceptable() {
            self.suggestions.popup.as_ref().map(|popup| &popup.model)
        } else {
            self.suggestions
                .cached
                .as_ref()
                .filter(|cached| {
                    !cached.model.is_empty()
                        && cached.context.same_editor(self)
                        && self.cached_source_current(&cached.source)
                        && self.prompt.is_none()
                        && self.modal.is_none()
                        && self.focus == Focus::Editor
                })
                .map(|cached| &cached.model)
        }
    }
    pub(super) fn cancel_suggestions(&mut self) {
        self.suggestions.cached = None;
        self.suggestions.queued = None;
        self.suggestions.accept_after_resolve = None;
        self.suggestions.resolve_due = None;
        if let Some(resolving) = self.suggestions.resolving.take() {
            match resolving.origin {
                ResolveOrigin::Native { instance, token } => {
                    if let Some(client) = &mut self.lsp
                        && Arc::ptr_eq(&instance, &client.identity())
                    {
                        let _ = client.cancel_completion_resolve(token);
                    }
                }
                ResolveOrigin::Provider(ticket) => self.cancel_provider_completion_resolve(&ticket),
            }
        }
        let pending = self.suggestions.pending.take();
        let popup = self.suggestions.popup.take();
        let native_current = self.lsp.as_ref().is_some_and(|client| {
            let identity = client.identity();
            pending.as_ref().is_some_and(|pending| matches!(&pending.origin, Awaiting::Native { instance, .. } if Arc::ptr_eq(instance, &identity)))
                || popup.as_ref().is_some_and(|popup| matches!(&popup.source, Source::Native { instance, .. } if Arc::ptr_eq(instance, &identity)))
        });
        if native_current && let Some(client) = &mut self.lsp {
            let _ = client.cancel_completions();
        }
        if pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.origin, Awaiting::Provider(_)))
            || popup
                .as_ref()
                .is_some_and(|popup| matches!(popup.source, Source::Provider(_)))
        {
            self.cancel_extension_provider();
        }
    }
    pub(super) fn request_suggestions(&mut self, extra: Value) {
        self.request_suggestions_inner(extra, false);
    }
    fn request_suggestions_inner(&mut self, extra: Value, automatic: bool) {
        let cached = if automatic {
            self.suggestions.cached.take()
        } else {
            None
        };
        self.cancel_suggestions();
        self.suggestions.cached = cached;
        let result = (|| {
            let context = self.suggestion_context()?;
            prefix(self)?;
            let origin =
                if self.extension_language_request("textDocument/completion", extra.clone()) {
                    Awaiting::Provider(
                        self.completion_provider_ticket()
                            .context("Extension completion request was not admitted")?,
                    )
                } else {
                    let doc = &self.documents[self.active];
                    let client = self.lsp.as_mut().context("Language server is not ready")?;
                    client.sync(&self.documents)?;
                    let token = client.request_completion(
                        doc,
                        extra,
                        self.panes.get(self.active_pane).map(|pane| pane.id),
                    )?;
                    Awaiting::Native {
                        instance: client.identity(),
                        token,
                    }
                };
            self.suggestions.pending = Some(Pending {
                origin,
                settings: self.settings.clone(),
                context,
                started: Instant::now(),
            });
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(error) = result
            && !automatic
        {
            self.message = format!("Suggestions unavailable: {error:#}");
        }
    }
    pub(super) fn native_suggestion_response(
        &mut self,
        request: crate::lsp::Request,
        response: Value,
    ) -> Result<()> {
        let Some(pending) = &self.suggestions.pending else {
            return Ok(());
        };
        let Awaiting::Native { instance, token } = &pending.origin else {
            return Ok(());
        };
        if *token != request.token
            || !pending.context.same_editor(self)
            || !self
                .lsp
                .as_ref()
                .is_some_and(|client| Arc::ptr_eq(instance, &client.identity()))
        {
            return Ok(());
        }
        let instance = instance.clone();
        self.install_suggestions(Source::Native { instance, request }, response)
    }
    pub(super) fn provider_suggestion_response(
        &mut self,
        ticket: &Ticket,
        response: Value,
    ) -> Result<()> {
        let Some(pending) = &self.suggestions.pending else {
            return Ok(());
        };
        if !matches!(&pending.origin, Awaiting::Provider(old) if old.id == ticket.id && old.session == ticket.session && old.epoch == ticket.epoch)
            || !pending.context.same_editor(self)
        {
            return Ok(());
        }
        self.install_suggestions(Source::Provider(ticket.clone()), response)
    }
    fn install_suggestions(&mut self, source: Source, response: Value) -> Result<()> {
        let pending = self
            .suggestions
            .pending
            .take()
            .context("No current suggestion request")?;
        if self.prompt.is_some() || self.modal.is_some() || !self.suggestion_source_current(&source)
        {
            return Ok(());
        }
        let response = if response.is_null() {
            json!([])
        } else {
            response
        };
        self.suggestions.cached = None;
        let model = Model::parse(response, &prefix(self)?)?;
        if model.is_empty() {
            if matches!(source, Source::Provider(_)) {
                self.cancel_extension_provider();
            }
            self.cancel_suggestions();
            return Ok(());
        }
        self.suggestions.popup = Some(Popup {
            model,
            source,
            context: pending.context,
            settings: pending.settings,
        });
        self.suggestions.resolve_due = Some(Instant::now() + Duration::from_millis(120));
        Ok(())
    }
    pub(super) fn advance_suggestion_interaction(&mut self) {
        let cached_current = self
            .suggestions
            .cached
            .as_ref()
            .is_some_and(|cached| cached.context.same_editor(self));
        let queued_current = self
            .suggestions
            .queued
            .as_ref()
            .is_some_and(|queued| queued.context.same_editor(self));
        if cached_current {
            self.suggestions
                .cached
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        if queued_current {
            self.suggestions
                .queued
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        if self
            .suggestions
            .resolving
            .as_ref()
            .is_some_and(|pending| pending.context.same_editor(self))
        {
            self.suggestions
                .resolving
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        let pending_current = self
            .suggestions
            .pending
            .as_ref()
            .is_some_and(|pending| pending.context.same_editor(self));
        let popup_current = self
            .suggestions
            .popup
            .as_ref()
            .is_some_and(|popup| popup.context.same_editor(self));
        if pending_current {
            self.suggestions
                .pending
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        if popup_current {
            self.suggestions
                .popup
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        if pending_current || popup_current {
            self.advance_completion_provider_interaction();
        }
    }
    pub(super) fn suggestion_ui_event(&mut self, event: &Event) {
        if self.suggestions.pending.is_none()
            && self.suggestions.popup.is_none()
            && self.suggestions.cached.is_none()
            && self.suggestions.queued.is_none()
        {
            return;
        }
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && matches!(
                self.keymap.resolve(&keys::token(*key), &self.context()),
                Resolution::Command(_, _)
            )
        {
            self.advance_suggestion_interaction();
        }
    }
    pub(super) fn execute_suggestion_command(&mut self, command: &str) {
        if command == "hideSuggestWidget" {
            self.cancel_suggestions();
            return;
        }
        if self.suggestion_model().is_none() {
            self.cancel_suggestions();
            return;
        }
        match command {
            "selectNextSuggestion"
            | "selectPrevSuggestion"
            | "selectNextPageSuggestion"
            | "selectPrevPageSuggestion" => {
                let model = if let Some(popup) = &mut self.suggestions.popup {
                    &mut popup.model
                } else {
                    &mut self.suggestions.cached.as_mut().unwrap().model
                };
                let delta = match command {
                    "selectNextSuggestion" => 1,
                    "selectPrevSuggestion" => -1,
                    "selectNextPageSuggestion" => 8,
                    _ => -8,
                };
                model.step(delta);
                self.suggestions.accept_after_resolve = None;
                self.suggestions.resolve_due = Some(Instant::now() + Duration::from_millis(120));
            }
            "acceptSelectedSuggestion" => {
                if !self.suggestion_acceptable() {
                    self.cancel_suggestions();
                    return;
                }
                if self.selected_suggestion_needs_resolve() {
                    self.suggestions.accept_after_resolve = self
                        .suggestions
                        .popup
                        .as_ref()
                        .and_then(|popup| popup.model.selected_item())
                        .map(|item| item.value.clone());
                    self.suggestions.resolve_due = Some(Instant::now());
                    self.message = "Resolving suggestion · Esc cancels".into();
                    self.poll_suggestion_resolution();
                    return;
                }
                self.apply_selected_suggestion();
            }
            _ => {}
        }
    }
    fn apply_selected_suggestion(&mut self) {
        if !self.suggestion_acceptable() {
            self.cancel_suggestions();
            return;
        }
        let popup = self.suggestions.popup.take().unwrap();
        let Some(item) = popup.model.selected_item() else {
            return;
        };
        let provider = matches!(&popup.source, Source::Provider(_));
        let action = match popup.source {
            Source::Native { request, .. } => LanguageAction::Completion {
                request,
                item: item.value.clone(),
            },
            Source::Provider(ticket) => LanguageAction::Provider {
                ticket,
                item: item.value.clone(),
            },
        };
        if let Err(error) = self.language_action(&action) {
            self.message = format!("Suggestion rejected: {error:#}");
        }
        if provider {
            self.cancel_extension_provider();
        }
        self.cancel_suggestions();
    }
    pub fn suggestion_resolving(&self) -> bool {
        self.suggestions.resolving.is_some() || self.suggestions.accept_after_resolve.is_some()
    }
    fn selected_suggestion_needs_resolve(&self) -> bool {
        self.suggestions.popup.as_ref().is_some_and(|popup| {
            popup
                .model
                .selected_item()
                .is_some_and(|item| !item.resolved)
                && match &popup.source {
                    Source::Native { .. } => self.lsp.as_ref().is_some_and(|client| {
                        client.capabilities["completionProvider"]["resolveProvider"] == true
                    }),
                    Source::Provider(ticket) => ticket.provider.resolves,
                }
        })
    }
    fn poll_suggestion_resolution(&mut self) -> bool {
        if let Some(pending) = &self.suggestions.resolving {
            if pending.started.elapsed() > Duration::from_secs(6) {
                self.cancel_suggestions();
                self.message = "Suggestion resolution timed out; request suggestions again".into();
                return true;
            }
            return false;
        }
        if !self.suggestion_acceptable()
            || !self.selected_suggestion_needs_resolve()
            || self
                .suggestions
                .resolve_due
                .is_none_or(|due| due > Instant::now())
        {
            return false;
        }
        let popup = self.suggestions.popup.as_ref().unwrap();
        // Inspect capacity before cloning retained item/context snapshots.
        match &popup.source {
            Source::Native { .. } => {
                if self
                    .lsp
                    .as_ref()
                    .is_some_and(|client| client.completion_resolve_closed())
                {
                    self.cancel_suggestions();
                    self.message = "Suggestion resolution timed out; restart the language server before resolving again".into();
                    return true;
                }
                if !self
                    .lsp
                    .as_ref()
                    .is_some_and(|client| client.completion_resolve_available())
                {
                    return false;
                }
            }
            Source::Provider(_) => {
                if !self
                    .extension_host
                    .as_ref()
                    .is_some_and(|host| host.language_provider_capacity())
                {
                    return false;
                }
            }
        }
        let source = popup.source.clone();
        let original = popup.model.selected_item().unwrap().value.clone();
        let context = popup.context.clone();
        let origin = match source {
            Source::Native { instance, request } => {
                let Some(client) = &mut self.lsp else {
                    return false;
                };
                client
                    .resolve_completion(&request, original.clone())
                    .map(|token| ResolveOrigin::Native { instance, token })
            }
            Source::Provider(ticket) => self
                .request_provider_completion_resolve(&ticket, &original)
                .map(ResolveOrigin::Provider),
        };
        match origin {
            Ok(origin) => {
                self.suggestions.resolving = Some(Resolving {
                    origin,
                    context,
                    original,
                    started: Instant::now(),
                });
                true
            }
            Err(error) => {
                self.cancel_suggestions();
                self.message = format!("Suggestion resolution failed: {error:#}");
                true
            }
        }
    }
    pub(super) fn native_suggestion_resolve_response(
        &mut self,
        request: crate::lsp::Request,
        response: Value,
    ) -> Result<()> {
        let Some(pending) = &self.suggestions.resolving else {
            return Ok(());
        };
        if !matches!(&pending.origin, ResolveOrigin::Native { instance, token } if *token == request.token && self.lsp.as_ref().is_some_and(|client| Arc::ptr_eq(instance,&client.identity())))
        {
            return Ok(());
        }
        if let Some(error) = response.get("_vscliResolveError") {
            self.fail_suggestion_resolution(format!("{error}"));
            return Ok(());
        }
        self.finish_suggestion_resolution(response)
    }
    pub(super) fn provider_suggestion_resolve_response(
        &mut self,
        ticket: &Ticket,
        response: Value,
    ) -> Result<()> {
        if !self.suggestions.resolving.as_ref().is_some_and(|pending| matches!(&pending.origin, ResolveOrigin::Provider(old) if old.id == ticket.id && old.session == ticket.session && old.epoch == ticket.epoch)) { return Ok(()); }
        self.finish_suggestion_resolution(response)
    }
    pub(super) fn provider_suggestion_resolve_error(&mut self, ticket: &Ticket, message: String) {
        if self.suggestions.resolving.as_ref().is_some_and(|pending| matches!(&pending.origin, ResolveOrigin::Provider(old) if old.id == ticket.id && old.session == ticket.session)) {
            self.fail_suggestion_resolution(message);
        }
    }
    fn fail_suggestion_resolution(&mut self, message: String) {
        let Some(pending) = self.suggestions.resolving.take() else {
            return;
        };
        if pending.context.same_editor(self)
            && self.suggestion_acceptable()
            && self
                .suggestions
                .popup
                .as_ref()
                .and_then(|popup| popup.model.selected_item())
                .is_some_and(|item| item.value == pending.original)
        {
            self.cancel_suggestions();
            self.message = format!("Suggestion resolution failed: {message}");
        }
    }
    fn finish_suggestion_resolution(&mut self, response: Value) -> Result<()> {
        let pending = self.suggestions.resolving.take().unwrap();
        if !pending.context.same_editor(self) || !self.suggestion_acceptable() {
            return Ok(());
        }
        let popup = self.suggestions.popup.as_mut().unwrap();
        if !popup
            .model
            .selected_item()
            .is_some_and(|item| item.value == pending.original)
        {
            return Ok(());
        }
        let accepted = self.suggestions.accept_after_resolve.as_ref() == Some(&pending.original);
        if let Err(error) = popup
            .model
            .replace_selected_resolved(&pending.original, response)
        {
            self.cancel_suggestions();
            self.message = format!("Suggestion resolution rejected: {error:#}");
            return Ok(());
        }
        self.suggestions.accept_after_resolve = None;
        if accepted {
            self.apply_selected_suggestion();
        }
        Ok(())
    }
    fn edit_stamp(&self) -> Option<(u64, u64, Option<char>)> {
        if self.focus != Focus::Editor || self.prompt.is_some() || self.modal.is_some() {
            return None;
        }
        self.active_document()
            .map(|doc| (doc.id, doc.text_epoch(), None))
    }
    pub(super) fn suggestion_edit_command(
        &self,
        command: &str,
        args: Option<&Value>,
    ) -> Option<(u64, u64, Option<char>)> {
        let mut stamp = typing_command(command)
            .then(|| self.edit_stamp())
            .flatten()?;
        if command == "type" {
            stamp.2 = args
                .and_then(|args| args["text"].as_str())
                .filter(|text| text.chars().count() == 1)
                .and_then(|text| text.chars().next());
        }
        Some(stamp)
    }
    pub(super) fn suggestion_edit_event(&self, event: &Event) -> Option<(u64, u64, Option<char>)> {
        match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && matches!(
                        key.code,
                        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
                    ) =>
            {
                self.edit_stamp().map(|mut stamp| {
                    if let KeyCode::Char(character) = key.code {
                        stamp.2 = Some(character);
                    }
                    stamp
                })
            }
            _ => None,
        }
    }
    fn completion_trigger(&self, character: char) -> bool {
        let character = character.to_string();
        if let Some(provider) = self.active_document().and_then(|doc| {
            self.extension_host.as_ref().and_then(|host| {
                host.language_provider(crate::extension_providers::Kind::Completion, doc)
            })
        }) {
            return provider.triggers.contains(&character);
        }
        self.lsp.as_ref().is_some_and(|client| {
            client.capabilities["completionProvider"]["triggerCharacters"]
                .as_array()
                .is_some_and(|triggers| {
                    triggers
                        .iter()
                        .take(32)
                        .any(|value| value.as_str() == Some(&character))
                })
        })
    }
    pub(super) fn observe_suggestion_edit(&mut self, before: Option<(u64, u64, Option<char>)>) {
        let Some(before) = before else {
            return;
        };
        let Some(doc) = self.active_document() else {
            return;
        };
        if doc.id != before.0 || doc.text_epoch() == before.1 {
            return;
        }
        if !self.has_extension_provider(crate::extension_providers::Kind::Completion)
            && !self.lsp.as_ref().is_some_and(|client| {
                client.ready
                    && (client.capabilities["completionProvider"].is_object()
                        || client.capabilities["completionProvider"] == true)
            })
        {
            self.cancel_suggestions();
            return;
        }
        let settings = self.settings.suggestions(self.language());
        let word = doc.cursor > 0 && {
            let character = doc.text.char(doc.cursor - 1);
            character.is_alphanumeric() || character == '_'
        };
        let character = before
            .2
            .filter(|character| doc.cursor > 0 && doc.text.char(doc.cursor - 1) == *character);
        let trigger =
            character.filter(|character| settings.triggers && self.completion_trigger(*character));
        let result = (|| {
            if trigger.is_none() && (!settings.quick || !word) {
                bail!("No automatic completion trigger");
            }
            let context = self.suggestion_context()?;
            // With no labels to filter, defer the bounded word scan to dispatch.
            // Raw input bursts must not repeatedly scan an ever-growing prefix.
            let prefix = (self.suggestions.popup.is_some() || self.suggestions.cached.is_some())
                .then(|| prefix(self))
                .transpose()?;
            Ok((context, prefix))
        })();
        let model = self
            .suggestions
            .popup
            .take()
            .map(|popup| {
                (
                    popup.model,
                    match popup.source {
                        Source::Native { instance, .. } => Fence::Native(instance),
                        Source::Provider(ticket) => Fence::Provider(ticket),
                    },
                )
            })
            .or_else(|| {
                self.suggestions
                    .cached
                    .take()
                    .map(|cached| (cached.model, cached.source))
            });
        self.cancel_suggestions();
        let Ok((context, prefix)) = result else {
            return;
        };
        if let Some((mut model, source)) = model
            && let Some(prefix) = prefix
            && model.filter(&prefix).is_ok()
            && !model.is_empty()
        {
            self.suggestions.cached = Some(Cached {
                source,
                model,
                context: context.clone(),
                started: Instant::now(),
            });
        }
        let extra = trigger.map_or_else(|| json!({"context":{"triggerKind":1}}), |character| json!({"context":{"triggerKind":2,"triggerCharacter":character.to_string()}}));
        self.suggestions.queued = Some(Queued {
            context,
            due: Instant::now() + Duration::from_millis(settings.delay_ms),
            started: Instant::now(),
            extra,
        });
    }
    pub(super) fn dispatch_queued_suggestions(&mut self) -> bool {
        let Some(queued) = &self.suggestions.queued else {
            return false;
        };
        let settings = self.settings.suggestions(self.language());
        let enabled = if queued.extra["context"]["triggerKind"] == 2 {
            settings.triggers
        } else {
            settings.quick
        };
        if !queued.context.same_editor(self) || queued.started.elapsed() > Duration::from_secs(6) {
            self.cancel_suggestions();
            return true;
        }
        if !enabled {
            self.cancel_suggestions();
            return true;
        }
        if queued.due > Instant::now() {
            return false;
        }
        if self.has_extension_provider(crate::extension_providers::Kind::Completion)
            && !self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.language_provider_capacity())
        {
            return false;
        }
        let queued = self.suggestions.queued.take().unwrap();
        self.request_suggestions_inner(queued.extra, true);
        true
    }
    pub(super) fn poll_suggestions(&mut self) -> bool {
        let stale = self.suggestions.pending.as_ref().is_some_and(|pending| {
            let source_current = match &pending.origin {
                Awaiting::Native { instance, .. } => self
                    .lsp
                    .as_ref()
                    .is_some_and(|client| Arc::ptr_eq(instance, &client.identity())),
                Awaiting::Provider(ticket) => self
                    .extension_host
                    .as_ref()
                    .is_some_and(|host| host.provider_registration_current(ticket)),
            };
            !source_current
                || !pending.context.same_editor(self)
                || pending.settings != self.settings
                || pending.started.elapsed() > Duration::from_secs(6)
        }) || self.suggestions.cached.as_ref().is_some_and(|cached| {
            !cached.context.same_editor(self)
                || !self.cached_source_current(&cached.source)
                || cached.started.elapsed() > Duration::from_secs(6)
        }) || self
            .suggestions
            .queued
            .as_ref()
            .is_some_and(|queued| !queued.context.same_editor(self))
            || self.suggestions.popup.as_ref().is_some_and(|popup| {
                !popup.context.same_editor(self)
                    || popup.settings != self.settings
                    || !self.suggestion_source_current(&popup.source)
            });
        if stale {
            self.cancel_suggestions();
        }
        self.poll_suggestion_resolution() || stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifier_prefix_retains_the_utf8_byte_bound_across_rope_chunks() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("prefix.rs");
        let identifier = "é".repeat(crate::suggestions::MAX_PREFIX / 2);
        let text = format!("{} {identifier}", "x".repeat(4096));
        std::fs::write(&path, &text).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().move_to(text.chars().count(), false);
        assert_eq!(prefix(&app).unwrap(), identifier);
        app.doc_mut().insert("é", false);
        assert!(prefix(&app).unwrap_err().to_string().contains("1 KiB"));
        app.doc_mut().insert(" ", false);
        assert_eq!(prefix(&app).unwrap(), "");
    }
    #[test]
    fn suggestion_dispatch_uses_reserved_default_ids_and_does_not_swallow_an_extension_alias() {
        let reserved = native_command_ids();
        for profile in [Profile::Linux, Profile::Macos, Profile::Windows] {
            for binding in Keymap::new(profile)
                .bindings
                .into_iter()
                .filter(|binding| command(&binding.command))
            {
                assert!(reserved.contains(&binding.command), "{}", binding.command);
            }
        }
        // This is not a pinned native command and remains available to packages.
        assert!(!command("acceptSelectedSuggestionOnEnter"));
    }
}
