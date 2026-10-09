use super::*;
use crate::signature::Hint;
#[derive(PartialEq)]
struct Context {
    document: u64,
    revision: u64,
    path: Option<PathBuf>,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
}
impl Context {
    fn capture(app: &App) -> Option<Self> {
        let doc = app.active_document()?;
        Some(Self {
            document: doc.id,
            revision: doc.revision,
            path: doc.path.clone(),
            selections: doc.selections(),
            pane: app.panes.get(app.active_pane).map(|p| p.id),
        })
    }
    fn valid(&self, app: &App) -> bool {
        Context::capture(app).as_ref() == Some(self)
            && app.focus == Focus::Editor
            && app.prompt.is_none()
            && app.modal.is_none()
            && app.lsp.as_ref().is_some_and(|c| c.ready)
    }
}
#[derive(Default)]
pub(super) struct State {
    context: Option<Context>,
    pending: bool,
    hint: Option<Hint>,
}
impl App {
    pub(super) fn has_signature_provider(&self) -> bool {
        if self.has_extension_provider(crate::extension_providers::Kind::Signature) {
            return true;
        }
        self.active_document()
            .and_then(|doc| doc.path.as_ref())
            .is_some_and(|path| {
                let language = crate::lsp::language(path);
                self.lsp.as_ref().is_some_and(|client| {
                    client.ready
                        && client.capabilities["signatureHelpProvider"].is_object()
                        && (client.language == language
                            || (client.language == "cpp" && language == "c"))
                })
            })
    }

    pub fn signature_help(&self) -> Option<&Hint> {
        if let Some(hint) = self.extension_signature_help() {
            return Some(hint);
        }
        self.signature
            .context
            .as_ref()
            .filter(|c| c.valid(self))
            .and(self.signature.hint.as_ref())
    }
    pub(super) fn clear_signature(&mut self) {
        self.clear_extension_signature();
        self.signature = State::default();
        if let Some(client) = &mut self.lsp {
            let _ = client.cancel_signature_help();
        }
    }
    pub(super) fn refresh_signature(&mut self) -> bool {
        if self
            .signature
            .context
            .as_ref()
            .is_some_and(|c| !c.valid(self))
        {
            self.clear_signature();
            true
        } else {
            false
        }
    }
    pub(super) fn request_signature(&mut self) {
        self.clear_signature();
        if self.extension_language_request("textDocument/signatureHelp", json!({})) {
            return;
        }
        if self.active_document().is_none() {
            self.message = "Open a file before requesting parameter hints".into();
            return;
        }
        if !self.has_signature_provider() {
            self.message = "The configured language server does not provide parameter hints".into();
            return;
        }
        self.focus = Focus::Editor;
        let context = Context::capture(self);
        let client = self.lsp.as_mut().unwrap();
        match client.sync(&self.documents).and_then(|_| {
            client.request(
                "textDocument/signatureHelp",
                &self.documents[self.active],
                json!({"context":{"triggerKind":1,"isRetrigger":false}}),
            )
        }) {
            Ok(()) => {
                self.signature = State {
                    context,
                    pending: true,
                    hint: None,
                };
                self.message = "Loading parameter hints…".into();
            }
            Err(error) => self.message = format!("Parameter hints: {error:#}"),
        }
    }
    pub(super) fn signature_response(
        &mut self,
        request: &crate::lsp::Request,
        response: &Value,
    ) -> Result<()> {
        if !self.signature.pending
            || !self
                .signature
                .context
                .as_ref()
                .is_some_and(|c| c.valid(self))
        {
            return Ok(());
        }
        self.request_current(request)?;
        self.signature.pending = false;
        self.signature.hint = Hint::parse(response)?;
        self.message = if self.signature.hint.is_some() {
            "Parameter hints · Escape dismisses"
        } else {
            "No parameter hints available"
        }
        .into();
        Ok(())
    }
}
