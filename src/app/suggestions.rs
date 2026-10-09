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
enum Source {
    Native {
        instance: Arc<()>,
        request: crate::lsp::Request,
    },
    Provider(Ticket),
}
struct Pending {
    origin: Awaiting,
    context: Context,
    started: Instant,
}
struct Popup {
    model: Model,
    source: Source,
    context: Context,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    popup: Option<Popup>,
}
pub(super) fn command(command: &str) -> bool {
    matches!(
        command,
        "acceptSelectedSuggestion"
            | "acceptSelectedSuggestionOnEnter"
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
    while start > 0
        && (doc.text.char(start - 1).is_alphanumeric() || doc.text.char(start - 1) == '_')
    {
        start -= 1;
        if doc.cursor - start > crate::suggestions::MAX_PREFIX {
            bail!("Completion prefix exceeds 1 KiB");
        }
    }
    let prefix = doc.text.slice(start..doc.cursor).to_string();
    if prefix.len() > crate::suggestions::MAX_PREFIX {
        bail!("Completion prefix exceeds 1 KiB");
    }
    Ok(prefix)
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
        if self.documents.len() + self.hidden_documents.len() > 128
            || self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|doc| doc.secondary.len() >= 4096)
        {
            bail!("Suggestions support at most 128 buffers and 4096 selections per buffer");
        }
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
    pub fn suggestion_model(&self) -> Option<&Model> {
        self.suggestions
            .popup
            .as_ref()
            .filter(|popup| {
                !popup.model.is_empty()
                    && popup.context.same_editor(self)
                    && self.suggestion_source_current(&popup.source)
                    && self.prompt.is_none()
                    && self.modal.is_none()
                    && self.focus == Focus::Editor
            })
            .map(|popup| &popup.model)
    }
    pub(super) fn cancel_suggestions(&mut self) {
        let pending = self.suggestions.pending.take();
        let popup = self.suggestions.popup.take();
        if (pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.origin, Awaiting::Native { .. }))
            || popup
                .as_ref()
                .is_some_and(|popup| matches!(popup.source, Source::Native { .. })))
            && let Some(client) = &mut self.lsp
        {
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
        self.cancel_suggestions();
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
                context,
                started: Instant::now(),
            });
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(error) = result {
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
        });
        Ok(())
    }
    pub(super) fn advance_suggestion_interaction(&mut self) {
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
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && matches!(self.keymap.resolve(&keys::token(*key), &self.context()), Resolution::Command(id, _) if command(&id))
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
            "selectNextSuggestion" => self.suggestions.popup.as_mut().unwrap().model.step(1),
            "selectPrevSuggestion" => self.suggestions.popup.as_mut().unwrap().model.step(-1),
            "selectNextPageSuggestion" => self.suggestions.popup.as_mut().unwrap().model.step(8),
            "selectPrevPageSuggestion" => self.suggestions.popup.as_mut().unwrap().model.step(-8),
            "acceptSelectedSuggestion" | "acceptSelectedSuggestionOnEnter" => {
                let popup = self.suggestions.popup.take().unwrap();
                let Some(item) = popup.model.selected_item() else {
                    return;
                };
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
                self.cancel_suggestions();
            }
            _ => {}
        }
    }
    pub(super) fn poll_suggestions(&mut self) -> bool {
        let stale = self.suggestions.pending.as_ref().is_some_and(|pending| {
            !pending.context.same_editor(self) || pending.started.elapsed() > Duration::from_secs(6)
        }) || self.suggestions.popup.as_ref().is_some_and(|popup| {
            !popup.context.same_editor(self) || !self.suggestion_source_current(&popup.source)
        });
        if stale {
            self.cancel_suggestions();
        }
        stale
    }
}
