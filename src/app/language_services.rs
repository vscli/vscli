use super::*;
use crate::language_services::{Event as ServiceEvent, Launch, Worker, builtin};

#[derive(Default)]
enum Mode {
    #[default]
    Off,
    Automatic,
    Manual(Launch),
}
#[derive(Clone, PartialEq)]
struct Context {
    document: Option<u64>,
    pane: Option<u64>,
    focus: Focus,
}
enum Pending {
    Start {
        generation: u64,
        launch: Launch,
        context: Context,
    },
    Retirement,
}
#[derive(Default)]
pub(super) struct State {
    mode: Mode,
    disabled: bool,
    worker: Option<Worker>,
    pending: Option<Pending>,
    selected: Option<Launch>,
    active: Option<Launch>,
    attempted: bool,
    generation: u64,
    retiring: Option<crate::lsp::Client>,
    blocked_notice: Option<String>,
    status: String,
}
impl App {
    /// The binary enables automatic services explicitly; embedded native editors
    /// remain independent of installed external programs unless configured.
    pub fn configure_language_services(
        &mut self,
        manual: Option<Launch>,
        disabled: bool,
    ) -> Result<()> {
        if self.language_services.worker.is_some() {
            anyhow::bail!("Language services already configured");
        }
        if let Some(launch) = &manual {
            launch.validate()?;
        }
        self.language_services.mode = manual.map_or(Mode::Automatic, Mode::Manual);
        self.language_services.disabled = disabled;
        self.language_services.worker = Some(Worker::start()?);
        Ok(())
    }
    fn language_service_context(&self) -> Context {
        Context {
            document: self.active_document().map(|doc| doc.id),
            pane: self.panes.get(self.active_pane).map(|pane| pane.id),
            focus: self.focus.clone(),
        }
    }
    fn desired_language_service(&mut self) -> Option<Launch> {
        if self.language_services.disabled {
            return None;
        }
        match &self.language_services.mode {
            Mode::Off => None,
            Mode::Manual(launch) => Some(launch.clone()),
            Mode::Automatic => {
                self.active_document()?.path.as_ref()?;
                let language = self.language().to_owned();
                let configuration = self.settings.language_server(&language);
                if configuration.blocked_workspace {
                    if self.language_services.blocked_notice.as_ref() != Some(&language) {
                        self.message = "Workspace language-server program/args ignored; enable vscli.languageServer.allowWorkspaceConfiguration in user settings to allow them".into();
                        self.language_services.blocked_notice = Some(language.clone());
                    }
                } else {
                    self.language_services.blocked_notice = None;
                }
                if !configuration.enabled {
                    return None;
                }
                let (program, language) = match configuration.program {
                    Some(program) => (program, language),
                    None => {
                        let (program, language) = builtin(&language)?;
                        (program.into(), language.into())
                    }
                };
                Some(Launch {
                    workspace: self.workspace.root.clone(),
                    language,
                    program,
                    args: configuration.args,
                })
            }
        }
    }
    fn clear_language_service_ui(&mut self) {
        self.clear_signature();
        self.cancel_symbols();
        self.diagnostics.clear();
        if matches!(self.modal, Some(Modal::Language { .. }))
            || matches!(&self.modal, Some(Modal::Text { title, .. }) if title.trim().starts_with("Hover ·"))
        {
            self.modal = None;
        }
    }
    pub(super) fn language_service_failed(&mut self, error: &str) {
        self.clear_language_service_ui();
        self.language_services.active = None;
        self.language_services.attempted = true;
        self.language_services.status =
            format!("Language server stopped: {error}; use Language: Restart Server");
        self.message = self.language_services.status.clone();
        if self.language_services.worker.is_some() {
            self.language_services.retiring = self.lsp.take();
        } else {
            // Existing embedding callers may still supply their own Client.
            self.lsp = None;
        }
    }
    pub(super) fn poll_language_services(&mut self) -> bool {
        if self.language_services.worker.is_none() {
            return false;
        }
        let desired = self.desired_language_service();
        let mut changed = false;
        if desired != self.language_services.selected {
            self.language_services.selected = desired.clone();
            self.language_services.generation = self.language_services.generation.wrapping_add(1);
            self.language_services.attempted = false;
            changed = true;
        }
        if self.language_services.active != desired && self.lsp.is_some() {
            self.clear_language_service_ui();
            self.language_services.retiring = self.lsp.take();
            self.language_services.active = None;
            changed = true;
        }
        if let Some(event) = self
            .language_services
            .worker
            .as_ref()
            .and_then(Worker::poll)
        {
            let pending = self.language_services.pending.take();
            match (event, pending) {
                (
                    ServiceEvent::Started(reply_generation, result),
                    Some(Pending::Start {
                        generation,
                        launch,
                        context,
                    }),
                ) => {
                    let manual = matches!(self.language_services.mode, Mode::Manual(_));
                    let current = generation == reply_generation
                        && generation == self.language_services.generation
                        && desired.as_ref() == Some(&launch)
                        && (manual
                            || (context == self.language_service_context()
                                && self.prompt.is_none()
                                && self.modal.is_none()));
                    if current {
                        match result {
                            Ok(offer) => {
                                self.lsp = offer.accept();
                                self.language_services.active =
                                    self.lsp.as_ref().map(|_| launch.clone());
                                self.language_services.status =
                                    format!("Starting language server: {}", launch.program);
                                self.message = self.language_services.status.clone();
                            }
                            Err(error) => {
                                self.language_services.status =
                                    format!("Language service unavailable: {error}");
                                self.message = self.language_services.status.clone();
                            }
                        }
                        self.language_services.attempted = true;
                    } else {
                        // Unaccepted Client ownership stays on the worker. Retire
                        // its offer before any subsequent startup, even disabled.
                        self.language_services.attempted = false;
                        if self
                            .language_services
                            .worker
                            .as_ref()
                            .unwrap()
                            .retire(None)
                            .is_ok()
                        {
                            self.language_services.pending = Some(Pending::Retirement);
                        }
                    }
                }
                (ServiceEvent::Retired, _) => {}
                _ => {}
            }
            changed = true;
        }
        if self.language_services.worker.as_ref().unwrap().stopped() {
            self.language_services.pending = None;
            let status = "Language service worker stopped; restart the editor to retry";
            if self.language_services.status != status {
                self.language_services.status = status.into();
                self.message = status.into();
                changed = true;
            }
            return changed;
        }
        if self.language_services.pending.is_some() {
            return changed;
        }
        if self.language_services.retiring.is_some() {
            let client = self.language_services.retiring.take();
            match self
                .language_services
                .worker
                .as_ref()
                .unwrap()
                .retire(client)
            {
                Ok(()) => self.language_services.pending = Some(Pending::Retirement),
                Err(client) => self.language_services.retiring = *client,
            }
            return true;
        }
        if self.lsp.is_none()
            && !self.language_services.attempted
            && let Some(launch) = desired
        {
            let manual = matches!(self.language_services.mode, Mode::Manual(_));
            if !manual
                && (self.focus != Focus::Editor || self.prompt.is_some() || self.modal.is_some())
            {
                return changed;
            }
            let generation = self.language_services.generation;
            let context = self.language_service_context();
            if self
                .language_services
                .worker
                .as_ref()
                .unwrap()
                .launch(generation, launch.clone())
                .is_ok()
            {
                self.language_services.pending = Some(Pending::Start {
                    generation,
                    launch,
                    context,
                });
                changed = true;
            }
        }
        changed
    }
    pub(super) fn language_service_command(&mut self, command: &str) {
        match command {
            "vscli.languageServer.disable" => {
                self.language_services.disabled = true;
                self.clear_language_service_ui();
                if self.lsp.is_some() {
                    self.language_services.retiring = self.lsp.take();
                }
                self.language_services.active = None;
                self.language_services.status = "Language services disabled for this window".into();
            }
            "vscli.languageServer.enable" | "vscli.languageServer.restart" => {
                self.language_services.disabled = false;
                self.language_services.generation =
                    self.language_services.generation.wrapping_add(1);
                self.language_services.attempted = false;
                self.clear_language_service_ui();
                if self.lsp.is_some() {
                    self.language_services.retiring = self.lsp.take();
                }
                self.language_services.active = None;
                self.language_services.status = "Language server restart requested".into();
            }
            _ => {
                self.message = if let Some(launch) = &self.language_services.active {
                    format!(
                        "Language server: {} ({}) · {}",
                        launch.program,
                        launch.language,
                        if self.lsp.as_ref().is_some_and(|client| client.ready) {
                            "ready"
                        } else {
                            "initializing"
                        }
                    )
                } else if self.language_services.status.is_empty() {
                    "No active language server for this file".into()
                } else {
                    self.language_services.status.clone()
                };
                return;
            }
        }
        self.message = self.language_services.status.clone();
    }
    pub(super) fn shutdown_language_services(&mut self) {
        if let Some(worker) = self.language_services.worker.take() {
            worker.finish(
                self.lsp
                    .take()
                    .or_else(|| self.language_services.retiring.take()),
            );
        }
    }
}
