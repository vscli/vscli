//! One editor context, independently owned native and optional action sources.
use super::extension_services::Context;
use super::workspace_edits::{
    WorkspaceEditPolicy, WorkspaceEditTarget, WorkspaceVersion, apply_workspace_edit,
};
use super::*;
use crate::{extension_providers::Kind, extensions::providers::Ticket, lsp::Request};
use anyhow::{Context as _, bail};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct State {
    cohort: Option<Cohort>,
}
struct Cohort {
    context: Context,
    settings: crate::settings::Settings,
    native: Option<(u64, Arc<()>)>,
    native_queued: Option<(Arc<()>, Value)>,
    native_origin: Option<Arc<Request>>,
    tickets: Vec<Ticket>,
    resolve: Option<Ticket>,
    rows: Vec<Row>,
    shown: bool,
    selected: bool,
    source_limit: usize,
    truncated: bool,
}
enum Source {
    Native(Arc<Request>),
    Extension(Ticket),
}
struct Row {
    source: Source,
    item: Value,
}
const TITLE: &str = " Code Actions · Enter applies · Esc closes ";
impl App {
    pub(super) fn finish_code_actions(&mut self) {
        // A successful server command can still be finishing; retain its real
        // transport slot. The changed models have already retired authorization.
        if let Some(cohort) = self.actions.cohort.take()
            && let Some(host) = &mut self.extension_host
        {
            for ticket in cohort.tickets.iter().chain(cohort.resolve.iter()) {
                let _ = host.cancel_language_provider(ticket);
            }
        }
    }
    pub(super) fn cancel_code_actions(&mut self) {
        if let Some(cohort) = self.actions.cohort.take() {
            if let Some((token, server)) = &cohort.native
                && let Some(client) = &mut self.lsp
                && Arc::ptr_eq(server, &client.identity())
            {
                let _ = client.cancel_action_request(*token);
            }
            if cohort.shown
                && matches!(&self.modal, Some(Modal::Language { title, .. }) if title == TITLE)
            {
                self.modal = None;
            }
            if let Some(host) = &mut self.extension_host {
                for ticket in cohort.tickets.iter().chain(cohort.resolve.iter()) {
                    let _ = host.cancel_language_provider(ticket);
                }
            }
        }
    }
    fn action_context_current(&self) -> bool {
        self.actions.cohort.as_ref().is_some_and(|cohort| {
            cohort.context.same_editor(self) && cohort.settings == self.settings
        })
    }
    pub(super) fn poll_code_actions(&mut self) -> bool {
        if self.actions.cohort.is_some()
            && (!self.action_context_current()
                || self.prompt.is_some()
                || self.modal.as_ref().is_some_and(
                    |modal| !matches!(modal, Modal::Language { title, .. } if title == TITLE),
                ))
        {
            self.cancel_code_actions();
            self.message = "Document changed or editor context changed since code action request; request actions again".into();
            return true;
        }
        self.dispatch_queued_native_action()
    }
    fn dispatch_queued_native_action(&mut self) -> bool {
        let Some(cohort) = self.actions.cohort.as_ref() else {
            return false;
        };
        let Some((origin, _)) = &cohort.native_queued else {
            return false;
        };
        if cohort.selected {
            return false;
        }
        let Some(client) = &self.lsp else {
            self.actions.cohort.as_mut().unwrap().native_queued = None;
            return true;
        };
        if !Arc::ptr_eq(origin, &client.identity()) || client.action_channel_closed() {
            self.actions.cohort.as_mut().unwrap().native_queued = None;
            self.message = "Native code action source changed or timed out; restart the server and request actions again".into();
            return true;
        }
        if !client.action_available() {
            return false;
        }
        let (origin, extra) = self
            .actions
            .cohort
            .as_mut()
            .unwrap()
            .native_queued
            .take()
            .unwrap();
        let client = self.lsp.as_mut().unwrap();
        let result = client.sync(&self.documents).and_then(|()| {
            client.request_code_actions(
                &self.documents[self.active],
                extra,
                self.panes.get(self.active_pane).map(|pane| pane.id),
            )
        });
        match result {
            Ok(token) => self.actions.cohort.as_mut().unwrap().native = Some((token, origin)),
            Err(error) => self.message = format!("Native code action source: {error:#}"),
        }
        true
    }
    pub(super) fn code_action_ui_event(&mut self, event: &Event) {
        let picker = self
            .actions
            .cohort
            .as_ref()
            .is_some_and(|cohort| cohort.shown)
            && matches!(&self.modal, Some(Modal::Language { title, .. }) if title == TITLE);
        if picker
            && self.action_context_current()
            && matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release && key.modifiers.is_empty() && matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Enter | KeyCode::Tab))
        {
            self.actions
                .cohort
                .as_mut()
                .unwrap()
                .context
                .accept_next_interaction();
        } else if matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release)
            || matches!(event, Event::Paste(_))
            || matches!(event, Event::Mouse(mouse) if mouse.kind != MouseEventKind::Moved)
        {
            let popup = if picker { self.modal.take() } else { None };
            let stale = self.actions.cohort.is_some() && !self.action_context_current();
            self.cancel_code_actions();
            if popup.is_some() {
                self.modal = popup;
            }
            if stale {
                self.message =
                    "Document changed or editor context changed since code action request".into();
            }
        }
    }
    pub(super) fn request_all_code_actions(&mut self, kind: Option<&str>) {
        self.cancel_code_actions();
        if self.active_document().is_none() {
            self.message = "Open a document before requesting code actions".into();
            return;
        }
        if let Err(error) = Context::check_budget(self) {
            self.message = error.to_string();
            return;
        }
        self.focus = Focus::Editor;
        self.prompt = None;
        self.modal = None;
        let doc = self.doc();
        let selected_range = doc.selection().unwrap_or(doc.cursor..doc.cursor);
        let (start, end) = (selected_range.start, selected_range.end);
        let range = crate::lsp::Range {
            start: crate::lsp::position(doc, start),
            end: crate::lsp::position(doc, end),
        };
        let selection = json!({"anchor":crate::lsp::position(doc,doc.anchor.unwrap_or(doc.cursor)),"active":crate::lsp::position(doc,doc.cursor)});
        let diagnostics = self
            .current_diagnostics()
            .iter()
            .filter(|diagnostic| {
                (
                    diagnostic.range.start.line,
                    diagnostic.range.start.character,
                ) <= (range.end.line, range.end.character)
                    && (diagnostic.range.end.line, diagnostic.range.end.character)
                        >= (range.start.line, range.start.character)
            })
            .take(129)
            .map(|diagnostic| serde_json::to_value(diagnostic).unwrap_or(Value::Null))
            .collect::<Vec<_>>();
        if diagnostics.len() > 128 {
            self.message = "Code actions support at most 128 diagnostics in the selected range; narrow the selection".into();
            return;
        }
        let mut context = json!({"triggerKind":1});
        if let Some(kind) = kind {
            context["only"] = json!(kind);
        }
        let mut native_context = context.clone();
        if let Some(kind) = kind {
            native_context["only"] = json!([kind]);
        }
        native_context["diagnostics"] = json!(diagnostics);
        self.actions.cohort = Some(Cohort {
            context: Context::capture(self),
            settings: self.settings.clone(),
            native: None,
            native_queued: None,
            native_origin: None,
            tickets: Vec::new(),
            resolve: None,
            rows: Vec::new(),
            shown: false,
            selected: false,
            source_limit: 300,
            truncated: false,
        });
        let mut failures = Vec::new();
        if let Some(client) = self.lsp.as_mut().filter(|client| {
            client.capabilities["codeActionProvider"] != Value::Null
                && client.capabilities["codeActionProvider"] != false
        }) {
            let extra = json!({"range":range,"context":native_context});
            if !client.action_available() && !client.action_channel_closed() {
                self.actions.cohort.as_mut().unwrap().native_queued =
                    Some((client.identity(), extra));
            } else {
                let result = client.sync(&self.documents).and_then(|()| {
                    client.request_code_actions(
                        &self.documents[self.active],
                        extra,
                        self.panes.get(self.active_pane).map(|pane| pane.id),
                    )
                });
                match result {
                    Ok(token) => {
                        self.actions.cohort.as_mut().unwrap().native =
                            Some((token, client.identity()))
                    }
                    Err(error) => failures.push(error.to_string()),
                }
            }
        }
        if let Some(host) = &mut self.extension_host {
            let providers = host.sync_configuration(&self.settings).and_then(|()| {
                host.language_providers(Kind::CodeAction, &self.documents[self.active], kind)
            });
            match providers {
                Ok(providers) => {
                    for provider in providers {
                        match host.request_language_provider_from(
                            &provider,
                            &self.documents,
                            &self.hidden_documents,
                            self.active,
                            json!({"range":range,"selection":selection,"context":context}),
                        ) {
                            Ok(ticket) => {
                                self.actions.cohort.as_mut().unwrap().tickets.push(ticket)
                            }
                            Err(error) => failures.push(format!("{}: {error}", provider.owner)),
                        }
                    }
                }
                Err(error) => failures.push(error.to_string()),
            }
        }
        let cohort = self.actions.cohort.as_mut().unwrap();
        cohort.source_limit = 300
            / (usize::from(cohort.native.is_some() || cohort.native_queued.is_some())
                + cohort.tickets.len())
            .max(1);
        self.message = if cohort.native.is_none()
            && cohort.native_queued.is_none()
            && cohort.tickets.is_empty()
        {
            format!(
                "No ready code action providers{}",
                if failures.is_empty() {
                    String::new()
                } else {
                    format!(": {}", failures.join("; "))
                }
            )
        } else if failures.is_empty() {
            "Requesting code actions…".into()
        } else {
            format!(
                "Requesting available code actions · {}",
                failures.join("; ")
            )
        };
    }
    pub(super) fn native_action_pending_owned(&self, request: &Request) -> bool {
        self.action_context_current()
            && self.actions.cohort.as_ref().is_some_and(|cohort| {
                cohort.native.as_ref().is_some_and(|(token, server)| {
                    *token == request.token && Arc::ptr_eq(server, &request.server)
                })
            })
    }
    pub(super) fn track_native_action(&mut self, request: &Request, token: u64) {
        if let Some(cohort) = &mut self.actions.cohort {
            cohort.native = Some((token, request.server.clone()));
        }
    }
    pub(super) fn native_action_result(
        &mut self,
        request: Request,
        result: Result<Value, String>,
    ) -> Result<()> {
        if !self.native_action_pending_owned(&request) {
            return Ok(());
        }
        match result {
            Ok(value) if request.method != "workspace/executeCommand" => {
                self.native_action_reply(request, value)
            }
            Ok(_) => {
                self.actions.cohort.as_mut().unwrap().native = None;
                self.finish_code_actions();
                Ok(())
            }
            Err(error) => {
                self.actions.cohort.as_mut().unwrap().native = None;
                self.message = format!("Native code action source: {error}");
                Ok(())
            }
        }
    }
    pub(super) fn native_action_context_current(&self, request: &Request) -> Result<()> {
        let current = self.action_context_current()
            && self.actions.cohort.as_ref().is_some_and(|cohort| {
                cohort.native_origin.as_ref().is_some_and(|origin| {
                    Arc::ptr_eq(&origin.workspace, &request.workspace)
                        && Arc::ptr_eq(&origin.server, &request.server)
                }) || (request.method == "textDocument/codeAction"
                    && cohort.native.as_ref().is_some_and(|(token, server)| {
                        *token == request.token && Arc::ptr_eq(server, &request.server)
                    }))
            });
        if !current {
            bail!(
                "Document changed or code action editor context changed since request; request actions again"
            );
        }
        Ok(())
    }
    pub(super) fn select_native_action(&mut self, request: &Request) -> Result<()> {
        self.code_action_current(request)?;
        let cohort = self
            .actions
            .cohort
            .as_mut()
            .context("Code action picker closed")?;
        if cohort.selected {
            bail!("A code action was already selected");
        }
        cohort.selected = true;
        cohort.native_queued = None;
        Ok(())
    }
    pub(super) fn native_action_reply(&mut self, request: Request, value: Value) -> Result<()> {
        if self.native_action_context_current(&request).is_err() {
            return Ok(());
        }
        if request.method == "codeAction/resolve" {
            return self.apply_code_action(&request, &value, true);
        }
        self.code_action_current(&request)?;
        let request = Arc::new(request);
        let cohort = self.actions.cohort.as_mut().unwrap();
        cohort.native = None;
        cohort.native_origin = Some(request.clone());
        if cohort.selected {
            return Ok(());
        }
        let mut items = match value {
            Value::Null => Vec::new(),
            Value::Array(items) => items,
            _ => bail!("Invalid native code action response"),
        };
        if items.len() > 300 {
            bail!("Native code action source exceeds 300 items");
        }
        items.sort_by_key(|item| {
            (
                item.get("disabled").is_some(),
                !item["isPreferred"].as_bool().unwrap_or(false),
            )
        });
        cohort.truncated |= items.len() > cohort.source_limit;
        items.truncate(cohort.source_limit);
        cohort.rows.extend(items.into_iter().map(|item| Row {
            source: Source::Native(request.clone()),
            item,
        }));
        self.show_action_rows();
        Ok(())
    }
    fn extension_action_current(&self, ticket: &Ticket) -> bool {
        self.action_context_current()
            && self.actions.cohort.as_ref().is_some_and(|cohort| {
                cohort
                    .tickets
                    .iter()
                    .chain(cohort.resolve.iter())
                    .any(|owned| owned.id == ticket.id && owned.session == ticket.session)
            })
            && self.active_document().is_some_and(|document| {
                self.extension_host.as_ref().is_some_and(|host| {
                    host.provider_ticket_current(ticket, document)
                        && host.provider_workspace_current(
                            ticket,
                            &self.documents,
                            &self.hidden_documents,
                        )
                })
            })
    }
    pub(super) fn extension_action_reply(&mut self, ticket: Ticket, result: Result<Value, String>) {
        if !self.extension_action_current(&ticket) {
            return;
        }
        let resolving = self
            .actions
            .cohort
            .as_ref()
            .unwrap()
            .resolve
            .as_ref()
            .is_some_and(|resolve| resolve.id == ticket.id);
        if self.actions.cohort.as_ref().unwrap().selected && !resolving {
            return;
        }
        let result = result.map_err(anyhow::Error::msg).and_then(|value| {
            if resolving {
                self.apply_extension_code_action(&ticket, &value, true)
            } else {
                let items = value
                    .as_array()
                    .context("Invalid extension code action response")?;
                if items.len() > 300 {
                    bail!("Extension code action source exceeds 300 items");
                }
                let cohort = self.actions.cohort.as_mut().unwrap();
                let mut items = items.iter().collect::<Vec<_>>();
                items.sort_by_key(|item| {
                    (
                        item.get("disabled").is_some(),
                        !item["isPreferred"].as_bool().unwrap_or(false),
                    )
                });
                cohort.truncated |= items.len() > cohort.source_limit;
                cohort
                    .rows
                    .extend(
                        items
                            .into_iter()
                            .take(cohort.source_limit)
                            .cloned()
                            .map(|item| Row {
                                source: Source::Extension(ticket.clone()),
                                item,
                            }),
                    );
                self.show_action_rows();
                Ok(())
            }
        });
        if let Err(error) = result {
            self.message = format!("Code action source {}: {error:#}", ticket.provider.owner);
        }
    }
    fn show_action_rows(&mut self) {
        if !self.action_context_current() {
            return;
        }
        let cohort = self.actions.cohort.as_mut().unwrap();
        if !cohort.shown {
            cohort.rows.sort_by_key(|row| {
                (
                    row.item.get("disabled").is_some(),
                    !row.item["isPreferred"].as_bool().unwrap_or(false),
                )
            });
        }
        if cohort.selected {
            return;
        }
        let truncated = cohort.truncated;
        let items = cohort
            .rows
            .iter()
            .take(300)
            .map(|row| {
                let title = row.item["title"]
                    .as_str()
                    .unwrap_or("Untitled action")
                    .chars()
                    .take(1024)
                    .collect::<String>();
                let reason = row.item["disabled"]["reason"]
                    .as_str()
                    .map(|reason| {
                        format!(
                            " (disabled: {})",
                            reason.chars().take(1024).collect::<String>()
                        )
                    })
                    .unwrap_or_default();
                let (source, action) = match &row.source {
                    Source::Native(request) => (
                        "native".to_owned(),
                        LanguageAction::CodeAction {
                            request: request.clone(),
                            item: row.item.clone(),
                        },
                    ),
                    Source::Extension(ticket) => (
                        ticket.provider.owner.clone(),
                        LanguageAction::ExtensionCodeAction {
                            ticket: ticket.clone(),
                            item: row.item.clone(),
                        },
                    ),
                };
                LanguageItem {
                    label: format!("{title}{reason} · {source}"),
                    action,
                }
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.message = "No code actions available from completed sources".into();
            return;
        }
        let selected = match &self.modal {
            Some(Modal::Language {
                title, selected, ..
            }) if title == TITLE => (*selected).min(items.len() - 1),
            _ => 0,
        };
        cohort.shown = true;
        self.modal = Some(Modal::Language {
            title: TITLE.into(),
            items,
            selected,
        });
        if truncated {
            self.message = "Code action sources exceed the shared 300-row budget; showing a bounded share per source".into();
        }
    }
    pub(super) fn apply_extension_code_action(
        &mut self,
        ticket: &Ticket,
        item: &Value,
        resolved: bool,
    ) -> Result<()> {
        if !self.extension_action_current(ticket) || self.modal.is_some() || self.prompt.is_some() {
            bail!("Extension code action context changed");
        }
        if !resolved {
            let cohort = self.actions.cohort.as_mut().unwrap();
            cohort.selected = true;
            cohort.native_queued = None;
        }
        if item.get("disabled").is_some() {
            bail!(
                "Code action is disabled: {}",
                item["disabled"]["reason"]
                    .as_str()
                    .unwrap_or("unsupported action")
            );
        }
        if item.get("command").is_some() {
            bail!("Extension code action commands are unsupported; no edits applied");
        }
        if !resolved && item.get("edit").is_none() && ticket.provider.resolves {
            let host = self
                .extension_host
                .as_mut()
                .context("Extension host stopped")?;
            let resolve = host.request_action_resolve(
                ticket,
                item,
                &self.documents,
                &self.hidden_documents,
                self.active,
            )?;
            self.actions.cohort.as_mut().unwrap().resolve = Some(resolve);
            self.message = "Resolving extension code action…".into();
            return Ok(());
        }
        let edit = item
            .get("edit")
            .context("Code action contains no supported edit")?;
        let targets = ticket
            .workspace
            .iter()
            .map(|target| WorkspaceEditTarget {
                uri: target.uri.clone(),
                path: target.path.clone(),
                document: target.document,
                revision: target.revision,
                text_epoch: target.text_epoch,
                version: WorkspaceVersion::Extension(target.version),
            })
            .collect::<Vec<_>>();
        let host = self
            .extension_host
            .as_ref()
            .context("Extension host stopped")?;
        let outcome = apply_workspace_edit(
            &mut self.documents,
            &mut self.hidden_documents,
            edit,
            &targets,
            WorkspaceEditPolicy {
                require_versions: true,
                max_total_document_bytes: Some(4 * 1024 * 1024),
            },
            |target, document| match target.version {
                WorkspaceVersion::Extension(version) => {
                    host.service_document_current(document, version)
                }
                _ => false,
            },
        )?;
        self.message = format!(
            "Applied extension code action to {} buffers; review and save (Undo is per file)",
            outcome.buffers
        );
        self.preview_edit_barrier();
        self.cancel_code_actions();
        Ok(())
    }
}
