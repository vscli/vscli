use super::*;
use crate::{
    extension_providers::{Kind, Provider},
    extensions::providers::Ticket,
    signature::{Hint, Model},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
struct Context {
    document: u64,
    revision: u64,
    text_epoch: u64,
    path: Option<PathBuf>,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
    workspace: PathBuf,
    input_epoch: u64,
    focus: Focus,
}
impl Context {
    fn check_budget(app: &App) -> Result<()> {
        if app
            .active_document()
            .is_some_and(|doc| doc.secondary.len() >= 16_384)
        {
            anyhow::bail!("Parameter hints support at most 16,384 selections");
        }
        Ok(())
    }
    fn capture(app: &App) -> Option<Self> {
        Self::check_budget(app).ok()?;
        let doc = app.active_document()?;
        Some(Self {
            document: doc.id,
            revision: doc.revision,
            text_epoch: doc.text_epoch(),
            path: doc.path.clone(),
            selections: doc.selections(),
            pane: app.panes.get(app.active_pane).map(|pane| pane.id),
            workspace: app.workspace.root.clone(),
            input_epoch: app.extension_services.epoch,
            focus: app.focus.clone(),
        })
    }
    fn same_editor(&self, app: &App) -> bool {
        app.active_document().is_some_and(|doc| {
            self.document == doc.id
                && self.revision == doc.revision
                && self.text_epoch == doc.text_epoch()
                && self.path == doc.path
                && self.pane == app.panes.get(app.active_pane).map(|pane| pane.id)
                && self.workspace == app.workspace.root
                && self.input_epoch == app.extension_services.epoch
                && self.focus == app.focus
                && self.selections.len() == doc.secondary.len().saturating_add(1)
                && self.selections.first().is_some_and(|selection| {
                    selection.cursor == doc.cursor && selection.anchor == doc.anchor
                })
                && self.selections[1..] == doc.secondary
        })
    }
    fn accept_next_interaction(&mut self) {
        self.input_epoch = self.input_epoch.wrapping_add(1);
    }
}

#[derive(Clone)]
enum Source {
    Native {
        instance: Arc<()>,
        capabilities: Value,
    },
    Provider {
        provider: Provider,
        session: u64,
        epoch: u64,
    },
}
impl Source {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Native {
                    instance: a,
                    capabilities: ac,
                },
                Self::Native {
                    instance: b,
                    capabilities: bc,
                },
            ) => Arc::ptr_eq(a, b) && ac == bc,
            (
                Self::Provider {
                    provider: a,
                    session: as_,
                    epoch: ae,
                },
                Self::Provider {
                    provider: b,
                    session: bs,
                    epoch: be,
                },
            ) => as_ == bs && ae == be && a.id == b.id && a.owner == b.owner,
            _ => false,
        }
    }
    fn trigger(&self, character: char, active: bool) -> bool {
        let expected = character.to_string();
        match self {
            Self::Provider { provider, .. } => {
                provider.triggers.contains(&expected)
                    || (active && provider.retriggers.contains(&expected))
            }
            Self::Native { capabilities, .. } => {
                let matches = |key: &str| {
                    capabilities[key].as_array().is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(expected.as_str()))
                    })
                };
                matches("triggerCharacters") || (active && matches("retriggerCharacters"))
            }
        }
    }
}
#[derive(Clone)]
enum Origin {
    Native { instance: Arc<()>, token: u64 },
    Provider(Ticket),
}
struct Pending {
    source: Source,
    origin: Origin,
    context: Context,
    settings: crate::settings::Settings,
    canceled: bool,
}
struct Active {
    source: Source,
    origin: Origin,
    context: Context,
    settings: crate::settings::Settings,
    model: Model,
}
#[derive(Clone, Copy)]
enum Trigger {
    Invoke,
    Character(char),
    Change,
}
impl Trigger {
    fn merge(self, newer: Self) -> Self {
        match newer {
            Self::Change => self,
            _ => newer,
        }
    }
    fn json(self, retrigger: bool, previous: Option<Value>) -> Value {
        let mut value = json!({"triggerKind": match self { Self::Invoke => 1, Self::Character(_) => 2, Self::Change => 3 }, "isRetrigger":retrigger});
        if let Self::Character(character) = self {
            value["triggerCharacter"] = json!(character.to_string());
        }
        if let Some(previous) = previous {
            value["activeSignatureHelp"] = previous;
        }
        value
    }
}
struct Queued {
    source: Source,
    context: Context,
    settings: crate::settings::Settings,
    trigger: Trigger,
    retrigger: bool,
    due: Instant,
}
#[derive(Clone, PartialEq)]
struct DocumentStamp {
    document: u64,
    epoch: u64,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
}
impl DocumentStamp {
    fn capture(app: &App) -> Option<Self> {
        let doc = app.active_document()?;
        if doc.secondary.len() >= 16_384 {
            return None;
        }
        Some(Self {
            document: doc.id,
            epoch: doc.text_epoch(),
            selections: doc.selections(),
            pane: app.panes.get(app.active_pane).map(|pane| pane.id),
        })
    }
}
pub(super) struct Stamp {
    before: Option<DocumentStamp>,
    source: Option<Source>,
    settings: Option<crate::settings::Settings>,
    active: bool,
    typed: Option<char>,
    mouse: bool,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    active: Option<Active>,
    queued: Option<Queued>,
    observed: Option<DocumentStamp>,
    fence_noticed: bool,
}
pub(super) fn command(command: &str) -> bool {
    matches!(
        command,
        "closeParameterHints"
            | "showNextParameterHint"
            | "showPrevParameterHint"
            | "editor.action.triggerParameterHints"
    )
}
fn native_triggers(capabilities: &Value) -> Option<Value> {
    capabilities.as_object()?;
    let mut bounded = json!({});
    for key in ["triggerCharacters", "retriggerCharacters"] {
        if let Some(value) = capabilities.get(key) {
            let values = value.as_array()?;
            if values.len() > 16
                || values.iter().any(|value| {
                    value
                        .as_str()
                        .is_none_or(|text| text.len() > 4 || text.chars().count() != 1)
                })
            {
                return None;
            }
            bounded[key] = value.clone();
        }
    }
    Some(bounded)
}
impl App {
    fn signature_source(&self) -> Option<Source> {
        let doc = self.active_document()?;
        if let Some(host) = &self.extension_host
            && let Some(provider) = host.language_provider(Kind::Signature, doc)
            && let Some((session, epoch)) = host.signature_source_identity(provider)
        {
            return Some(Source::Provider {
                provider: provider.clone(),
                session,
                epoch,
            });
        }
        let path = doc.path.as_ref()?;
        let language = crate::lsp::language(path);
        let client = self.lsp.as_ref()?;
        let capabilities = native_triggers(&client.capabilities["signatureHelpProvider"])?;
        (client.ready
            && (client.language == language || (client.language == "cpp" && language == "c")))
            .then(|| Source::Native {
                instance: client.identity(),
                capabilities,
            })
    }
    fn signature_source_current(&self, source: &Source) -> bool {
        self.signature_source()
            .is_some_and(|current| source.same(&current))
    }
    fn signature_context_current(
        &self,
        context: &Context,
        settings: &crate::settings::Settings,
        source: &Source,
    ) -> bool {
        self.focus == Focus::Editor
            && self.prompt.is_none()
            && self.modal.is_none()
            && context.same_editor(self)
            && settings == &self.settings
            && self.signature_source_current(source)
    }
    pub(super) fn has_signature_provider(&self) -> bool {
        self.signature_source().is_some()
    }
    pub fn signature_help(&self) -> Option<&Hint> {
        self.signature
            .active
            .as_ref()
            .filter(|active| {
                self.signature_context_current(&active.context, &active.settings, &active.source)
            })
            .map(|active| active.model.hint())
    }
    fn signature_triggered(&self) -> bool {
        self.signature_help().is_some()
            || self.signature.pending.as_ref().is_some_and(|pending| {
                !pending.canceled
                    && self.signature_context_current(
                        &pending.context,
                        &pending.settings,
                        &pending.source,
                    )
            })
            || self.signature.queued.as_ref().is_some_and(|queued| {
                self.signature_context_current(&queued.context, &queued.settings, &queued.source)
            })
    }
    fn cancel_signature_origin(&mut self, origin: &Origin) {
        match origin {
            Origin::Native { instance, .. } => {
                if let Some(client) = &mut self.lsp
                    && Arc::ptr_eq(instance, &client.identity())
                {
                    let _ = client.cancel_signature_help();
                }
            }
            Origin::Provider(ticket) => {
                if let Some(host) = &mut self.extension_host {
                    let _ = host.cancel_language_provider(ticket);
                }
            }
        }
    }
    fn release_signature_active(&mut self) {
        if let Some(active) = self.signature.active.take()
            && let Origin::Provider(ticket) = &active.origin
            && let Some(host) = &mut self.extension_host
        {
            let _ = host.release_signature_help(ticket, &active.model.context());
        }
    }
    fn cancel_signature_pending(&mut self) {
        let origin = self.signature.pending.as_mut().and_then(|pending| {
            if pending.canceled {
                None
            } else {
                pending.canceled = true;
                Some(pending.origin.clone())
            }
        });
        if let Some(origin) = origin {
            self.cancel_signature_origin(&origin);
        }
    }
    pub(super) fn clear_signature(&mut self) {
        self.signature.queued = None;
        self.release_signature_active();
        self.cancel_signature_pending();
        // The transport retains the occupied actual slot until positive release.
    }
    pub(super) fn refresh_signature(&mut self) -> bool {
        self.poll_signature()
    }
    fn signature_stamp(&self, typed: Option<char>, mouse: bool) -> Stamp {
        let source = (self.focus == Focus::Editor && self.prompt.is_none() && self.modal.is_none())
            .then(|| self.signature_source())
            .flatten();
        let eligible = source.is_some() && Context::check_budget(self).is_ok();
        Stamp {
            before: eligible.then(|| DocumentStamp::capture(self)).flatten(),
            source,
            settings: eligible.then(|| self.settings.clone()),
            active: eligible && self.signature_triggered(),
            typed,
            mouse,
        }
    }
    pub(super) fn signature_edit_event(&self, event: &Event) -> Stamp {
        let typed = match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && !key.modifiers.intersects(
                        KeyModifiers::CONTROL
                            | KeyModifiers::ALT
                            | KeyModifiers::SUPER
                            | KeyModifiers::META,
                    ) =>
            {
                if let KeyCode::Char(character) = key.code {
                    Some(character)
                } else {
                    None
                }
            }
            _ => None,
        };
        self.signature_stamp(typed, matches!(event, Event::Mouse(_)))
    }
    pub(super) fn signature_edit_command(&self, command: &str, args: Option<&Value>) -> Stamp {
        let typed = (command == "type")
            .then(|| {
                args.and_then(|value| value["text"].as_str())
                    .and_then(|text| text.chars().last())
            })
            .flatten();
        self.signature_stamp(typed, false)
    }
    pub(super) fn advance_signature_interaction(&mut self, command_name: &str) {
        if !command(command_name)
            && !super::suggestions::command(command_name)
            && command_name != "editor.action.triggerSuggest"
            && command_name != "workbench.action.files.save"
        {
            return;
        }
        let active_current = self
            .signature
            .active
            .as_ref()
            .is_some_and(|active| active.context.same_editor(self));
        let pending_current = self
            .signature
            .pending
            .as_ref()
            .is_some_and(|pending| pending.context.same_editor(self));
        let queued_current = self
            .signature
            .queued
            .as_ref()
            .is_some_and(|queued| queued.context.same_editor(self));
        if active_current {
            self.signature
                .active
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        if pending_current {
            self.signature
                .pending
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
        if queued_current {
            self.signature
                .queued
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        }
    }
    pub(super) fn signature_ui_event(&mut self, event: &Event) {
        if self.signature.active.is_none()
            && self.signature.pending.is_none()
            && self.signature.queued.is_none()
        {
            return;
        }
        let Event::Key(key) = event else {
            return;
        };
        if key.kind == KeyEventKind::Release {
            return;
        }
        if let Resolution::Command(name, _) =
            self.keymap.resolve(&keys::token(*key), &self.context())
        {
            self.advance_signature_interaction(&name);
        }
    }
    pub(super) fn observe_signature_edit(&mut self, stamp: Stamp) {
        let Some(before) = stamp.before else {
            return;
        };
        let Some(after) = DocumentStamp::capture(self) else {
            self.clear_signature();
            return;
        };
        if before == after {
            return;
        }
        if stamp.mouse
            || before.document != after.document
            || before.pane != after.pane
            || stamp.settings.as_ref() != Some(&self.settings)
            || self.focus != Focus::Editor
            || self.prompt.is_some()
            || self.modal.is_some()
        {
            self.clear_signature();
            return;
        }
        let Some(source) = stamp
            .source
            .filter(|source| self.signature_source_current(source))
        else {
            self.clear_signature();
            return;
        };
        if Context::check_budget(self).is_err() {
            self.clear_signature();
            return;
        }
        let character = stamp
            .typed
            .filter(|character| source.trigger(*character, stamp.active));
        let trigger = if let Some(character) = character {
            if !stamp.active && !self.settings.parameter_hints(self.language()).enabled {
                return;
            }
            Some(Trigger::Character(character))
        } else {
            stamp.active.then_some(Trigger::Change)
        };
        let Some(trigger) = trigger else {
            return;
        };
        // Inner execute and outer event can report the same actual gesture. A
        // stronger original character may still upgrade an existing Change.
        if self.signature.observed.as_ref() == Some(&after) {
            if let Some(queued) = &mut self.signature.queued {
                queued.trigger = queued.trigger.merge(trigger);
            }
            return;
        }
        self.signature.observed = Some(after);
        self.queue_signature(source, trigger, stamp.active, false);
    }
    fn queue_signature(
        &mut self,
        source: Source,
        trigger: Trigger,
        retrigger: bool,
        immediate: bool,
    ) {
        let Some(context) = Context::capture(self) else {
            self.clear_signature();
            return;
        };
        let trigger = self
            .signature
            .queued
            .as_ref()
            .filter(|queued| queued.source.same(&source) && queued.settings == self.settings)
            .map_or(trigger, |queued| queued.trigger.merge(trigger));
        let previously_triggered = self.signature_triggered();
        self.cancel_signature_pending();
        if !retrigger && !previously_triggered {
            self.release_signature_active();
        }
        self.signature.queued = Some(Queued {
            source,
            context,
            settings: self.settings.clone(),
            trigger,
            retrigger: retrigger || previously_triggered,
            due: Instant::now()
                + if immediate {
                    Duration::ZERO
                } else {
                    Duration::from_millis(120)
                },
        });
        self.signature.fence_noticed = false;
    }
    pub(super) fn request_signature(&mut self) {
        if self.active_document().is_none() {
            self.message = "Open a file before requesting parameter hints".into();
            return;
        }
        let Some(source) = self.signature_source() else {
            self.clear_signature();
            self.message = "The configured language server does not provide parameter hints".into();
            return;
        };
        if self.prompt.is_some() || self.modal.is_some() {
            self.clear_signature();
            return;
        }
        if let Err(error) = Context::check_budget(self) {
            self.clear_signature();
            self.message = format!("Parameter hints: {error:#}");
            return;
        }
        self.focus = Focus::Editor;
        let retrigger = self.signature_triggered();
        self.queue_signature(source, Trigger::Invoke, retrigger, true);
        self.message = "Loading parameter hints…".into();
        self.poll_signature();
    }
    pub(super) fn cycle_signature(&mut self, forward: bool) {
        if self.signature_help().is_none()
            || self
                .signature
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.canceled)
        {
            return;
        }
        let cycle = self.settings.parameter_hints(self.language()).cycle;
        if !self
            .signature
            .active
            .as_mut()
            .unwrap()
            .model
            .cycle(forward, cycle)
        {
            self.clear_signature();
        }
    }
    pub(super) fn poll_signature(&mut self) -> bool {
        let mut changed = false;
        if self.signature.pending.as_ref().is_some_and(|pending| {
            !pending.canceled
                && !self.signature_context_current(
                    &pending.context,
                    &pending.settings,
                    &pending.source,
                )
        }) {
            self.cancel_signature_pending();
            changed = true;
        }
        let retired =
            self.signature
                .pending
                .as_ref()
                .is_some_and(|pending| match &pending.origin {
                    Origin::Native { instance, .. } => self
                        .lsp
                        .as_ref()
                        .is_none_or(|client| !Arc::ptr_eq(instance, &client.identity())),
                    Origin::Provider(ticket) => self
                        .extension_host
                        .as_ref()
                        .is_none_or(|host| !host.provider_registration_current(ticket)),
                });
        if retired {
            self.signature.pending = None;
            changed = true;
        }
        if self.signature.queued.as_ref().is_some_and(|queued| {
            !self.signature_context_current(&queued.context, &queued.settings, &queued.source)
        }) {
            self.signature.queued = None;
            changed = true;
        }
        let ongoing = self.signature.queued.is_some()
            || self
                .signature
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.canceled);
        if !ongoing
            && self.signature.active.as_ref().is_some_and(|active| {
                !self.signature_context_current(&active.context, &active.settings, &active.source)
            })
        {
            self.release_signature_active();
            changed = true;
        }
        let Some(queued) = &self.signature.queued else {
            return changed;
        };
        if self.signature.pending.is_some() || queued.due > Instant::now() {
            return changed;
        }
        let (available, closed) = match &queued.source {
            Source::Native { .. } => self.lsp.as_ref().map_or((false, false), |client| {
                (
                    client.signature_available(),
                    client.signature_channel_closed(),
                )
            }),
            Source::Provider { .. } => {
                self.extension_host.as_ref().map_or((false, false), |host| {
                    (host.signature_available(), host.signature_channel_closed())
                })
            }
        };
        if !available {
            if closed && !self.signature.fence_noticed {
                self.signature.fence_noticed = true;
                self.message = "Parameter hints are awaiting actual callback release or language service restart".into();
                changed = true;
            }
            return changed;
        }
        let queued = self.signature.queued.take().unwrap();
        let previous = self
            .signature
            .active
            .as_ref()
            .filter(|active| {
                queued.retrigger
                    && active.source.same(&queued.source)
                    && active.settings == queued.settings
                    && active.context.document == queued.context.document
                    && active.context.path == queued.context.path
                    && active.context.pane == queued.context.pane
                    && active.context.workspace == queued.context.workspace
            })
            .map(|active| active.model.context());
        let options = json!({"context":queued.trigger.json(queued.retrigger, previous)});
        let origin = match &queued.source {
            Source::Native { instance, .. } => {
                let doc = &self.documents[self.active];
                let client = self.lsp.as_mut().unwrap();
                client
                    .sync(&self.documents)
                    .and_then(|_| {
                        client.request_signature_help(
                            doc,
                            options,
                            self.panes.get(self.active_pane).map(|pane| pane.id),
                        )
                    })
                    .map(|token| Origin::Native {
                        instance: instance.clone(),
                        token,
                    })
            }
            Source::Provider { provider, .. } => {
                let host = self.extension_host.as_mut().unwrap();
                host.sync_configuration(&self.settings)
                    .and_then(|_| {
                        host.request_language_provider_from(
                            provider,
                            &self.documents,
                            &self.hidden_documents,
                            self.active,
                            options,
                        )
                    })
                    .map(Origin::Provider)
            }
        };
        match origin {
            Ok(origin) => {
                self.signature.pending = Some(Pending {
                    source: queued.source,
                    origin,
                    context: queued.context,
                    settings: queued.settings,
                    canceled: false,
                })
            }
            Err(error) => {
                self.release_signature_active();
                self.message = format!("Parameter hints: {error:#}");
            }
        }
        true
    }
    fn signature_origin_matches(&self, origin: &Origin) -> bool {
        self.signature
            .pending
            .as_ref()
            .is_some_and(|pending| match (&pending.origin, origin) {
                (
                    Origin::Native {
                        instance: a,
                        token: at,
                    },
                    Origin::Native {
                        instance: b,
                        token: bt,
                    },
                ) => at == bt && Arc::ptr_eq(a, b),
                (Origin::Provider(a), Origin::Provider(b)) => {
                    a.id == b.id
                        && a.session == b.session
                        && a.epoch == b.epoch
                        && a.provider.id == b.provider.id
                        && a.provider.owner == b.provider.owner
                }
                _ => false,
            })
    }
    fn finish_signature(&mut self, origin: Origin, result: Result<&Value, &str>) -> Result<()> {
        if !self.signature_origin_matches(&origin) {
            self.release_signature_result(&origin, result.ok());
            return Ok(());
        }
        let pending = self.signature.pending.take().unwrap();
        if pending.canceled
            || !self.signature_context_current(&pending.context, &pending.settings, &pending.source)
        {
            self.release_signature_result(&origin, result.ok());
            return Ok(());
        }
        let model = match result {
            Ok(value) => Model::parse(value),
            Err(message) => {
                self.release_signature_active();
                self.message = format!("Parameter hints: {message}");
                return Ok(());
            }
        };
        let model = match model {
            Ok(model) => model,
            Err(error) => {
                self.release_signature_active();
                self.release_signature_result(&origin, result.ok());
                return Err(error);
            }
        };
        self.release_signature_active();
        self.signature.active = model.map(|model| Active {
            source: pending.source,
            origin,
            context: pending.context,
            settings: pending.settings,
            model,
        });
        self.message = if self.signature.active.is_some() {
            "Parameter hints · Escape dismisses"
        } else {
            "No parameter hints available"
        }
        .into();
        Ok(())
    }
    fn release_signature_result(&self, origin: &Origin, result: Option<&Value>) {
        if let Origin::Provider(ticket) = origin
            && let Some(value) = result
            && let Some(host) = &self.extension_host
        {
            let _ = host.release_signature_help(ticket, value);
        }
    }
    pub(super) fn signature_response(
        &mut self,
        request: &crate::lsp::Request,
        response: &Value,
    ) -> Result<()> {
        let origin = Origin::Native {
            instance: request.server.clone(),
            token: request.token,
        };
        if self.signature_origin_matches(&origin)
            && self
                .signature
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.canceled)
            && let Err(error) = self.request_current(request)
        {
            return self.finish_signature(origin, Err(&format!("{error:#}")));
        }
        self.finish_signature(origin, Ok(response))
    }
    pub(super) fn signature_failure(&mut self, request: &crate::lsp::Request, message: &str) {
        let origin = Origin::Native {
            instance: request.server.clone(),
            token: request.token,
        };
        let _ = self.finish_signature(origin, Err(message));
    }
    pub(super) fn extension_signature_reply(
        &mut self,
        ticket: &Ticket,
        result: Result<Value, String>,
    ) -> Result<()> {
        if self.signature_origin_matches(&Origin::Provider(ticket.clone()))
            && self
                .signature
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.canceled)
            && !self.active_document().is_some_and(|document| {
                self.extension_host
                    .as_ref()
                    .is_some_and(|host| host.provider_ticket_current(ticket, document))
            })
        {
            self.release_signature_result(&Origin::Provider(ticket.clone()), result.as_ref().ok());
            return self.finish_signature(
                Origin::Provider(ticket.clone()),
                Err("Parameter hint document mirror changed"),
            );
        }
        self.finish_signature(
            Origin::Provider(ticket.clone()),
            result.as_ref().map_err(String::as_str),
        )
    }
}
