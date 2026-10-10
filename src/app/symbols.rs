use super::*;
use crate::{lsp, symbols::Symbol};
use anyhow::{Context as _, bail};
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
struct Context {
    workspace: PathBuf,
    document: Option<u64>,
    documents: Vec<(u64, u64, u64, Option<PathBuf>)>,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
    groups: crate::editor_groups::UiProof,
    focus: Focus,
}
impl Context {
    fn capture(app: &App) -> Self {
        Self {
            workspace: app.workspace.root.clone(),
            document: app.active_document().map(|d| d.id),
            documents: app
                .documents
                .iter()
                .chain(&app.hidden_documents)
                .map(|d| (d.id, d.revision, d.text_epoch(), d.path.clone()))
                .collect(),
            selections: app
                .active_document()
                .map_or_else(Vec::new, Document::selections),
            pane: app.panes.get(app.active_pane).map(|p| p.id),
            groups: app.editor_groups.proof(),
            focus: app.focus.clone(),
        }
    }
    fn valid(&self, app: &App) -> bool {
        self == &Self::capture(app) && app.modal.is_none()
    }
}
struct Loading {
    receiver: Receiver<Result<Target, String>>,
    context: Context,
    generation: u64,
    symbol: Symbol,
}
enum Target {
    Open(PathBuf),
    Loaded(Box<Document>),
}
fn load_target(path: &Path, open: &[PathBuf]) -> Result<Target> {
    let path = crate::document::absolute_path(path)?;
    if open.contains(&path) {
        // Parent aliases can still resolve after the file is deleted. Reuse the
        // captured native buffer rather than requiring its backing file to exist.
        return Ok(Target::Open(path));
    }
    Ok(Target::Loaded(Box::new(Document::open_existing(&path)?)))
}
#[derive(Default)]
pub(super) struct State {
    context: Option<Context>,
    workspace: bool,
    items: Vec<Symbol>,
    query: String,
    changed: Option<Instant>,
    pending: bool,
    queued: bool,
    token: Option<u64>,
    source: Option<std::sync::Arc<()>>,
    generation: u64,
    loading: Option<Loading>,
}
impl App {
    pub(super) fn invalidate_symbol_context(&mut self) {
        let picker_stale = self.symbols.context.as_ref().is_some_and(|c| {
            !c.valid(self)
                || !matches!(
                    self.prompt.as_ref().map(|p| &p.kind),
                    Some(PromptKind::Symbols)
                )
        });
        let loader_stale = self.symbols.loading.as_ref().is_some_and(|loader| {
            loader.generation == self.symbols.generation
                && (!loader.context.valid(self) || self.prompt.is_some())
        });
        if picker_stale || loader_stale {
            self.cancel_symbols();
        }
    }
    pub(super) fn cancel_symbols(&mut self) {
        if self.provider_symbols_active() {
            self.cancel_extension_provider();
        }
        self.cancel_owned_symbol_request();
        self.symbols.context = None;
        self.symbols.items.clear();
        self.symbols.pending = false;
        self.symbols.queued = false;
        self.symbols.source = None;
        self.symbols.changed = None;
        self.symbols.generation = self.symbols.generation.wrapping_add(1);
        if matches!(
            self.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Symbols)
        ) {
            self.prompt = None;
        }
        // The worker owns its slot until completion, even after cancellation.
    }
    pub(super) fn start_symbols(&mut self, workspace: bool) {
        self.cancel_symbols();
        if self.focus == Focus::Outline {
            self.focus = Focus::Editor;
        }
        if !workspace && self.extension_language_request("textDocument/documentSymbol", json!({})) {
            return;
        }
        if self.documents.len() + self.hidden_documents.len() > 128 {
            self.message = "Symbol navigation supports at most 128 open buffers".into();
            return;
        }
        let provider = if workspace {
            "workspaceSymbolProvider"
        } else {
            "documentSymbolProvider"
        };
        let supported = self.lsp.as_ref().is_some_and(|c| {
            c.ready
                && (c.capabilities[provider] == true || c.capabilities[provider].is_object())
                && (workspace
                    || self
                        .active_document()
                        .and_then(|d| d.path.as_ref())
                        .is_some_and(|p| {
                            let language = lsp::language(p);
                            c.language == language || c.language == "cpp" && language == "c"
                        }))
        });
        if !supported {
            self.message =
                "The configured language server does not provide these symbols for this editor"
                    .into();
            return;
        }
        self.focus = Focus::Editor;
        self.start_prompt(PromptKind::Symbols, String::new());
        self.symbols.context = Some(Context::capture(self));
        self.symbols.source = self.lsp.as_ref().map(lsp::Client::identity);
        self.symbols.workspace = workspace;
        self.symbols.query.clear();
        self.send_symbol_request();
    }
    fn symbol_source_current(&self) -> bool {
        self.symbols.source.as_ref().is_some_and(|source| {
            self.lsp
                .as_ref()
                .is_some_and(|client| std::sync::Arc::ptr_eq(source, &client.identity()))
        })
    }
    fn cancel_owned_symbol_request(&mut self) {
        if let Some(token) = self.symbols.token.take()
            && self.symbol_source_current()
            && let Some(client) = &mut self.lsp
        {
            let _ = client.cancel_symbol_request(token);
        }
    }
    pub(super) fn native_symbol_picker_waiting(&self) -> bool {
        self.symbols.context.is_some()
            && self.symbol_source_current()
            && (self.symbols.pending || self.symbols.queued || self.symbols.changed.is_some())
    }
    pub(super) fn symbol_request_owned(&self, request: &lsp::Request) -> bool {
        self.symbols.token == Some(request.token)
            && self
                .symbols
                .source
                .as_ref()
                .is_some_and(|source| std::sync::Arc::ptr_eq(source, &request.server))
    }
    pub(super) fn symbol_failure(&mut self, request: &lsp::Request, message: String) {
        if !self.symbol_request_owned(request) {
            return;
        }
        self.symbols.token = None;
        self.symbols.pending = false;
        self.message = format!("Symbols: {message}");
    }
    fn send_symbol_request(&mut self) {
        if !self.symbol_source_current() {
            self.cancel_symbols();
            return;
        }
        let Some(client) = self.lsp.as_mut() else {
            return;
        };
        if !client.symbol_available() {
            self.symbols.queued = true;
            self.symbols.pending = true;
            self.symbols.changed = None;
            self.message = if client.symbol_channel_closed() {
                "Symbols: awaiting actual timed-out request release or language server restart"
                    .into()
            } else {
                "Waiting for the previous symbol request to finish…".into()
            };
            return;
        }
        let result = client.sync(&self.documents).and_then(|_| {
            if self.symbols.workspace {
                client.workspace_symbols(&self.symbols.query)
            } else {
                client.request_document_symbols(&self.documents[self.active])
            }
        });
        self.symbols.pending = result.is_ok();
        self.symbols.queued = false;
        self.symbols.changed = None;
        self.message = match result {
            Ok(token) => {
                self.symbols.token = Some(token);
                "Loading symbols…".into()
            }
            Err(error) => format!("Symbols: {error:#}"),
        };
    }
    pub fn symbol_items(&self, query: &str) -> Vec<&Symbol> {
        let source = if self.provider_symbols_active() {
            &self.extension_providers.symbols
        } else {
            &self.symbols.items
        };
        let mut items: Vec<_> = source
            .iter()
            .enumerate()
            .filter_map(|(index, item)| score(&item.label, query).map(|score| (score, index, item)))
            .collect();
        items.sort_by_key(|(score, index, _)| (std::cmp::Reverse(*score), *index));
        items.into_iter().map(|(_, _, item)| item).collect()
    }
    pub fn symbols_workspace(&self) -> bool {
        !self.provider_symbols_active() && self.symbols.workspace
    }
    pub(super) fn symbol_response(
        &mut self,
        request: &lsp::Request,
        response: &Value,
    ) -> Result<()> {
        if !self.symbol_request_owned(request)
            || !self.symbols.pending
            || !self.symbol_source_current()
            || !self.symbols.context.as_ref().is_some_and(|c| c.valid(self))
            || !matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Symbols)
            )
            || (request.method == "workspace/symbol") != self.symbols.workspace
        {
            return Ok(());
        }
        if self.symbols.workspace && self.prompt.as_ref().unwrap().text != self.symbols.query {
            return Ok(());
        }
        self.symbols.pending = false;
        self.symbols.token = None;
        let path = if self.symbols.workspace {
            None
        } else {
            self.request_current(request)?;
            Some(request.path.as_path())
        };
        let items = crate::symbols::parse(response, path)?;
        if !self.symbols.workspace {
            for item in &items {
                symbol_offset(self.doc(), &item.range)?;
            }
        }
        self.message = if self.symbols.workspace {
            format!(
                "{} symbols for '{}' · Enter navigates",
                items.len(),
                self.symbols.query
            )
        } else {
            format!("{} symbols · type to search · Enter navigates", items.len())
        };
        self.symbols.items = items;
        Ok(())
    }
    pub(super) fn accept_symbol(&mut self, query: &str, selected: usize) {
        if self.provider_symbols_active() {
            self.accept_provider_symbol(query, selected);
            return;
        }
        if !self.symbols.context.as_ref().is_some_and(|c| c.valid(self)) {
            self.cancel_symbols();
            return;
        }
        if self.symbols.pending
            || self.symbols.workspace
                && (self.symbols.query != query || self.symbols.changed.is_some())
        {
            self.prompt = Some(Prompt::new(PromptKind::Symbols, query.into()));
            self.message = "Wait for symbols matching the current query".into();
            return;
        }
        let items = self.symbol_items(query);
        let Some(symbol) = items
            .get(selected.min(items.len().saturating_sub(1)))
            .cloned()
            .cloned()
        else {
            self.prompt = Some(Prompt::new(PromptKind::Symbols, query.into()));
            self.message = "No matching symbols".into();
            return;
        };
        if let Err(error) = self.reveal_hidden_symbol(&symbol.path, &symbol.range) {
            self.cancel_symbols();
            self.message = format!("Symbol navigation: {error:#}");
            return;
        }
        if let Some(index) = self.symbol_document(&symbol.path) {
            let result = self.focus_symbol(index, &symbol.range);
            self.cancel_symbols();
            if let Err(error) = result {
                self.message = format!("Symbol navigation: {error:#}");
            }
            return;
        }
        if self.symbols.loading.is_some() {
            self.message = "A symbol file is still loading; retry shortly".into();
            self.cancel_symbols();
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let path = symbol.path.clone();
        let open: Vec<_> = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .filter_map(|d| d.path.clone())
            .collect();
        std::thread::spawn(move || {
            let _ = sender.send(load_target(&path, &open).map_err(|e| format!("{e:#}")));
        });
        self.symbols.context = None;
        self.symbols.items.clear();
        self.symbols.loading = Some(Loading {
            receiver,
            context: Context::capture(self),
            generation: self.symbols.generation,
            symbol,
        });
        self.message = "Opening symbol file…".into();
    }
    fn reveal_hidden_symbol(&mut self, path: &Path, range: &lsp::Range) -> Result<()> {
        if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|d| d.path.as_deref() == Some(path))
        {
            symbol_offset(&self.hidden_documents[index], range)?;
            self.can_admit_editor(self.hidden_documents[index].id)?;
            let doc = self.hidden_documents.remove(index);
            self.install_open_document(doc)?;
        }
        Ok(())
    }
    fn symbol_document(&self, path: &Path) -> Option<usize> {
        let uri = lsp::file_uri(path).ok();
        self.documents.iter().position(|d| {
            d.path
                .as_ref()
                .is_some_and(|p| p == path || uri.is_some() && lsp::file_uri(p).ok() == uri)
        })
    }
    fn focus_symbol(&mut self, index: usize, range: &lsp::Range) -> Result<()> {
        let offset = symbol_offset(&self.documents[index], range)?;
        let previous = self.suspend_navigation_observation();
        self.can_admit_editor(self.documents[index].id)?;
        self.active = index;
        self.focus = Focus::Editor;
        self.sync_pane();
        self.doc_mut().clear_secondary();
        self.doc_mut().move_to(offset, false);
        self.sync_pane();
        self.remember_active_file();
        self.resume_navigation_observation(previous, navigation_history::Reason::Jump);
        self.message = format!("Symbol in {}", self.doc().name());
        Ok(())
    }
    pub(super) fn poll_symbols(&mut self) -> bool {
        let mut changed = false;
        if let Some(loader) = &self.symbols.loading {
            let result = match loader.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("Symbol loader stopped".into())),
            };
            if let Some(result) = result {
                let loader = self.symbols.loading.take().unwrap();
                changed = true;
                if loader.generation == self.symbols.generation
                    && loader.context.valid(self)
                    && self.prompt.is_none()
                {
                    let result =
                        result
                            .map_err(anyhow::Error::msg)
                            .and_then(|target| match target {
                                Target::Open(path) => {
                                    self.reveal_hidden_symbol(&path, &loader.symbol.range)?;
                                    let index = self
                                        .symbol_document(&path)
                                        .context("Resolved symbol buffer is no longer open")?;
                                    self.focus_symbol(index, &loader.symbol.range)
                                }
                                Target::Loaded(mut doc) => {
                                    self.reveal_hidden_symbol(
                                        doc.path.as_ref().context("Loaded symbol has no path")?,
                                        &loader.symbol.range,
                                    )?;
                                    if let Some(index) = self.symbol_document(
                                        doc.path.as_ref().context("Loaded symbol has no path")?,
                                    ) {
                                        return self.focus_symbol(index, &loader.symbol.range);
                                    }
                                    let offset = symbol_offset(&doc, &loader.symbol.range)?;
                                    self.settings.apply(&mut doc);
                                    doc.move_to(offset, false);
                                    self.install_open_document(*doc)?;
                                    Ok(())
                                }
                            });
                    if let Err(error) = result {
                        self.message = format!("Symbol navigation: {error:#}");
                    }
                }
            }
        }
        if self.symbols.context.is_none() {
            return changed;
        }
        if !self.symbols.context.as_ref().unwrap().valid(self)
            || !self.symbol_source_current()
            || !self.lsp.as_ref().is_some_and(|c| c.ready)
            || !matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Symbols)
            )
        {
            self.cancel_symbols();
            return true;
        }
        if self.symbols.workspace {
            let query = self.prompt.as_ref().unwrap().text.clone();
            if query != self.symbols.query {
                self.symbols.query = query;
                self.symbols.items.clear();
                self.cancel_owned_symbol_request();
                self.symbols.pending = false;
                self.symbols.queued = false;
                self.symbols.changed = Some(Instant::now());
                self.message = "Waiting to search symbols…".into();
                changed = true;
            }
            if self
                .symbols
                .changed
                .is_some_and(|when| when.elapsed() >= Duration::from_millis(200))
            {
                self.send_symbol_request();
                changed = true;
            }
        }
        if self.symbols.queued
            && self
                .lsp
                .as_ref()
                .is_some_and(|client| client.symbol_available())
        {
            self.send_symbol_request();
            changed = true;
        }
        changed
    }
}
fn symbol_offset(doc: &Document, range: &lsp::Range) -> Result<usize> {
    let start = lsp::offset(doc, range.start)?;
    let end = lsp::offset(doc, range.end)?;
    if start > end {
        bail!("Reversed symbol range");
    }
    Ok(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn native_fixture() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        std::fs::write(&path, "猫🙂\r\nbody\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.lsp = Some(crate::lsp::outline_tests::start(root.path(), app.doc()));
        (root, app)
    }
    #[test]
    fn group_focus_round_trip_cannot_revive_identical_editor_context() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("猫🙂 dirty", false);
        app.execute("workbench.action.splitEditor", Value::Null);
        app.focus_pane(0);
        let before = Context::capture(&app);
        let model = (
            app.doc().id,
            app.doc().revision,
            app.doc().text_epoch(),
            app.doc().selections(),
        );
        app.focus_pane(1);
        app.focus_pane(0);
        assert_eq!(
            model,
            (
                app.doc().id,
                app.doc().revision,
                app.doc().text_epoch(),
                app.doc().selections()
            )
        );
        assert!(!before.valid(&app));
        assert!(Context::capture(&app).valid(&app));
        assert_eq!(app.doc().text.to_string(), "猫🙂 dirty");
        app.doc_mut().undo();
        assert!(app.doc().is_empty());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "猫🙂 dirty");
    }

    #[test]
    fn picker_cancellation_cannot_cancel_an_unowned_outline_token() {
        let (_root, mut app) = native_fixture();
        let doc = &app.documents[app.active];
        let outline = app
            .lsp
            .as_mut()
            .unwrap()
            .request_document_symbols(doc)
            .unwrap();
        crate::lsp::outline_tests::held(app.lsp.as_mut().unwrap(), outline);
        for _ in 0..32 {
            app.cancel_symbols();
        }
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":outline}))
            .unwrap();
        let events = crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
        assert!(events.iter().any(
            |event| matches!(event,lsp::Event::Response(request,_) if request.token==outline)
        ));
        app.start_symbols(false);
        let picker = app.symbols.token.unwrap();
        crate::lsp::outline_tests::held(app.lsp.as_mut().unwrap(), picker);
        app.cancel_symbols();
        for _ in 0..32 {
            app.start_symbols(false);
            assert!(app.symbols.queued);
            assert!(app.symbols.token.is_none());
        }
        assert!(app.native_symbol_picker_waiting());
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":picker}))
            .unwrap();
        let events = crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
        assert!(
            !events.iter().any(
                |event| matches!(event,lsp::Event::Response(request,_) if request.token==picker)
            )
        );
        app.poll_symbols();
        let latest = app.symbols.token.unwrap();
        assert_ne!(latest, picker);
        crate::lsp::outline_tests::held(app.lsp.as_mut().unwrap(), latest);
        app.cancel_symbols();
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":latest}))
            .unwrap();
        crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
    }
    #[test]
    fn latest_workspace_query_waits_for_actual_canceled_request_release() {
        let (_root, mut app) = native_fixture();
        app.start_symbols(true);
        let original = app.symbols.token.unwrap();
        crate::lsp::outline_tests::held(app.lsp.as_mut().unwrap(), original);
        for index in 0..32 {
            app.prompt.as_mut().unwrap().text = format!("query-{index}");
            app.poll_symbols();
            assert!(app.symbols.token.is_none());
            assert!(!app.lsp.as_ref().unwrap().symbol_available());
        }
        app.prompt.as_mut().unwrap().text = "latest".into();
        app.poll_symbols();
        app.symbols.changed = Some(Instant::now() - Duration::from_millis(201));
        app.poll_symbols();
        assert!(app.symbols.queued);
        assert!(app.native_symbol_picker_waiting());
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":original}))
            .unwrap();
        crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
        app.poll_symbols();
        let latest = app.symbols.token.unwrap();
        assert_ne!(latest, original);
        crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |_, events| {
            events.iter().any(
                |event| matches!(event,lsp::Event::Message(message) if message=="query-latest"),
            )
        });
        assert_eq!(app.symbols.query, "latest");
        app.cancel_symbols();
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":latest}))
            .unwrap();
        crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
    }
    #[test]
    fn held_native_picker_reply_cannot_revive_after_live_edit_undo() {
        let (_root, mut app) = native_fixture();
        app.start_symbols(false);
        let token = app.symbols.token.unwrap();
        crate::lsp::outline_tests::held(app.lsp.as_mut().unwrap(), token);
        let id = app.doc().id;
        let revision = app.doc().revision;
        let text = app.doc().text.to_string();
        let selections = app.doc().selections();
        app.doc_mut().insert("dirty", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().revision, revision);
        let response = json!([{"name":"stale","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}]);
        app.lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":token,"result":response}))
            .unwrap();
        let events = crate::lsp::outline_tests::until(app.lsp.as_mut().unwrap(), |client, _| {
            client.symbol_available()
        });
        for event in events {
            if let lsp::Event::Response(request, response) = event {
                app.symbol_response(&request, &response).unwrap();
            }
        }
        assert!(app.symbols.items.is_empty());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), text);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(
            std::fs::read(app.doc().path.as_ref().unwrap()).unwrap(),
            text.as_bytes()
        );
        app.poll_symbols();
        assert!(app.symbols.context.is_none());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), "dirty".to_owned() + &text);
    }
    #[test]
    fn late_alias_resolution_cannot_move_a_changed_dirty_buffer() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("open.cpp");
        std::fs::write(&path, "abc").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        let path = app.doc().path.clone().unwrap();
        let id = app.doc().id;
        let (sender, receiver) = mpsc::sync_channel(1);
        let position = lsp::Position {
            line: 0,
            character: 0,
        };
        app.symbols.loading = Some(Loading {
            receiver,
            context: Context::capture(&app),
            generation: app.symbols.generation,
            symbol: Symbol {
                label: "open".into(),
                path: path.clone(),
                range: lsp::Range {
                    start: position,
                    end: position,
                },
            },
        });
        app.doc_mut().insert("unsaved", false);
        let cursor = app.doc().cursor;
        std::fs::remove_file(&path).unwrap();
        sender.send(Ok(Target::Open(path))).unwrap();
        assert!(app.poll_symbols());
        assert!(app.symbols.loading.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().cursor, cursor);
        assert_eq!(app.doc().text.to_string(), "unsavedabc");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "abc");
    }
    #[test]
    fn pending_closed_file_load_does_not_block_existing_dirty_buffer_navigation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("open.cpp");
        std::fs::write(&path, "abc").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("dirty", false);
        let id = app.doc().id;
        std::fs::remove_file(&path).unwrap();
        let position = lsp::Position {
            line: 0,
            character: 1,
        };
        let symbol = Symbol {
            label: "open".into(),
            path: app.doc().path.clone().unwrap(),
            range: lsp::Range {
                start: position,
                end: position,
            },
        };
        let (_sender, receiver) = mpsc::sync_channel(1);
        app.symbols.loading = Some(Loading {
            receiver,
            context: Context::capture(&app),
            generation: app.symbols.generation,
            symbol: symbol.clone(),
        });
        app.symbols.items = vec![symbol];
        app.symbols.context = Some(Context::capture(&app));
        app.accept_symbol("", 0);
        assert!(app.symbols.loading.is_some());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().cursor, 1);
        assert_eq!(app.doc().text.to_string(), "dirtyabc");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "abc");
    }
    #[test]
    fn canceled_loader_keeps_its_slot_and_cannot_replace_native_input_or_buffers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("target.cpp");
        std::fs::write(&path, "abc").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("unsaved", false);
        let id = app.doc().id;
        let (sender, receiver) = mpsc::sync_channel(1);
        let position = lsp::Position {
            line: 0,
            character: 0,
        };
        app.symbols.loading = Some(Loading {
            receiver,
            context: Context::capture(&app),
            generation: app.symbols.generation,
            symbol: Symbol {
                label: "target".into(),
                path: path.clone(),
                range: lsp::Range {
                    start: position,
                    end: position,
                },
            },
        });
        app.start_prompt(PromptKind::Palette, "native query".into());
        assert!(app.symbols.loading.is_some());
        for _ in 0..10 {
            app.cancel_symbols();
            assert!(app.symbols.loading.is_some());
        }
        sender
            .send(Ok(Target::Loaded(Box::new(
                Document::open_existing(&path).unwrap(),
            ))))
            .unwrap();
        app.poll_symbols();
        assert!(app.symbols.loading.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "unsaved");
        assert_eq!(app.prompt.as_ref().unwrap().text, "native query");
    }
    #[test]
    fn held_symbol_load_cannot_revive_after_source_edit_and_undo() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("target.cpp");
        std::fs::write(&path, "target\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("猫🙂 original\r\n", false);
        let id = app.doc().id;
        let revision = app.doc().revision;
        let selections = app.doc().selections();
        let text = app.doc().text.to_string();
        let (sender, receiver) = mpsc::sync_channel(1);
        let position = lsp::Position {
            line: 0,
            character: 0,
        };
        app.symbols.loading = Some(Loading {
            receiver,
            context: Context::capture(&app),
            generation: app.symbols.generation,
            symbol: Symbol {
                label: "target".into(),
                path: path.clone(),
                range: lsp::Range {
                    start: position,
                    end: position,
                },
            },
        });
        app.doc_mut().insert("new edit", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text.to_string(), text);
        sender
            .send(Ok(Target::Loaded(Box::new(
                Document::open_existing(&path).unwrap(),
            ))))
            .unwrap();
        app.poll_symbols();
        assert!(app.symbols.loading.is_none());
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), text);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(std::fs::read(&path).unwrap(), b"target\r\n");
    }
}
