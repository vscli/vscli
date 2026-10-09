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
    documents: Vec<(u64, u64, Option<PathBuf>)>,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
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
                .map(|d| (d.id, d.revision, d.path.clone()))
                .collect(),
            selections: app
                .active_document()
                .map_or_else(Vec::new, Document::selections),
            pane: app.panes.get(app.active_pane).map(|p| p.id),
            focus: app.focus.clone(),
        }
    }
    fn valid(&self, app: &App) -> bool {
        self == &Self::capture(app) && app.modal.is_none()
    }
}
struct Loading {
    receiver: Receiver<Result<Document, String>>,
    context: Context,
    generation: u64,
    symbol: Symbol,
}
#[derive(Default)]
pub(super) struct State {
    context: Option<Context>,
    workspace: bool,
    items: Vec<Symbol>,
    query: String,
    changed: Option<Instant>,
    pending: bool,
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
        self.symbols.context = None;
        self.symbols.items.clear();
        self.symbols.pending = false;
        self.symbols.changed = None;
        self.symbols.generation = self.symbols.generation.wrapping_add(1);
        if let Some(client) = &mut self.lsp {
            let _ = client.cancel_symbol_requests();
        }
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
        if self.documents.len() > 128 {
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
        self.symbols.workspace = workspace;
        self.symbols.query.clear();
        self.send_symbol_request();
    }
    fn send_symbol_request(&mut self) {
        let Some(client) = self.lsp.as_mut() else {
            return;
        };
        let result = client
            .cancel_symbol_requests()
            .and_then(|_| client.sync(&self.documents))
            .and_then(|_| {
                if self.symbols.workspace {
                    client.workspace_symbols(&self.symbols.query)
                } else {
                    client.request(
                        "textDocument/documentSymbol",
                        &self.documents[self.active],
                        json!({}),
                    )
                }
            });
        self.symbols.pending = result.is_ok();
        self.symbols.changed = None;
        self.message = match result {
            Ok(()) => "Loading symbols…".into(),
            Err(error) => format!("Symbols: {error:#}"),
        };
    }
    pub fn symbol_items(&self, query: &str) -> Vec<&Symbol> {
        let mut items: Vec<_> = self
            .symbols
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| score(&item.label, query).map(|score| (score, index, item)))
            .collect();
        items.sort_by_key(|(score, index, _)| (std::cmp::Reverse(*score), *index));
        items.into_iter().map(|(_, _, item)| item).collect()
    }
    pub fn symbols_workspace(&self) -> bool {
        self.symbols.workspace
    }
    pub(super) fn symbol_response(
        &mut self,
        request: &lsp::Request,
        response: &Value,
    ) -> Result<()> {
        if !self.symbols.pending
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
        std::thread::spawn(move || {
            let _ = sender.send(Document::open_existing(&path).map_err(|e| format!("{e:#}")));
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
        let id = self.documents[index].id;
        if self.active_document().is_some_and(|doc| doc.id == id) {
            self.focus = Focus::Editor;
        } else if let Some(pane) = self.panes.iter().position(|p| p.document == id) {
            self.focus_pane(pane);
        } else {
            self.active = index;
            self.focus = Focus::Editor;
            self.sync_pane();
        }
        self.doc_mut().clear_secondary();
        self.doc_mut().move_to(offset, false);
        self.sync_pane();
        self.remember_active_file();
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
                    let result = result.map_err(anyhow::Error::msg).and_then(|mut doc| {
                        if let Some(index) = self.symbol_document(
                            doc.path.as_ref().context("Loaded symbol has no path")?,
                        ) {
                            return self.focus_symbol(index, &loader.symbol.range);
                        }
                        let offset = symbol_offset(&doc, &loader.symbol.range)?;
                        self.settings.apply(&mut doc);
                        doc.move_to(offset, false);
                        self.install_open_document(doc);
                        Ok(())
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
            || !self.lsp.as_ref().is_some_and(|c| c.ready)
            || !matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Symbols)
            )
        {
            self.cancel_symbols();
            return true;
        }
        if self.symbols.pending && self.lsp.as_ref().is_some_and(|c| !c.has_symbol_request()) {
            self.symbols.pending = false;
            changed = true;
        }
        if self.symbols.workspace {
            let query = self.prompt.as_ref().unwrap().text.clone();
            if query != self.symbols.query {
                self.symbols.query = query;
                self.symbols.items.clear();
                self.symbols.pending = false;
                self.symbols.changed = Some(Instant::now());
                self.message = "Waiting to search symbols…".into();
                if let Some(client) = &mut self.lsp {
                    let _ = client.cancel_symbol_requests();
                }
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
            .send(Ok(Document::open_existing(&path).unwrap()))
            .unwrap();
        app.poll_symbols();
        assert!(app.symbols.loading.is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "unsaved");
        assert_eq!(app.prompt.as_ref().unwrap().text, "native query");
    }
}
