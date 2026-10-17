//! A global, owner-bound workspace symbol cohort; no active document is required.
use super::*;
use crate::{
    extensions::providers::{WorkspaceSymbolProvider, WorkspaceSymbolTicket},
    lsp,
    symbols::Symbol,
};
use anyhow::{Context as _, ensure};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
const LIMIT: usize = 512;
const BYTES: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub(super) struct Context {
    root: PathBuf,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    profile: u64,
}
impl Context {
    fn capture(app: &App) -> Self {
        Self {
            root: app.workspace.root.clone(),
            settings: app.settings.extension_layers().clone(),
            profile: app.settings_profile_generation(),
        }
    }
    pub(super) fn current(&self, app: &App) -> bool {
        app.settings_error.is_none()
            && self.root == app.workspace.root
            && self.profile == app.settings_profile_generation()
            && Arc::ptr_eq(&self.settings, app.settings.extension_layers())
    }
}
struct Item {
    ticket: WorkspaceSymbolTicket,
    value: Value,
    name: String,
    kind: u64,
    uri: String,
    range: Option<lsp::Range>,
    container: String,
    display: Symbol,
}
struct Selected {
    item: Item,
    request: Option<WorkspaceSymbolTicket>,
    navigation: super::symbols::Context,
}
struct Run {
    context: Context,
    cohort: Vec<WorkspaceSymbolProvider>,
    waiting: VecDeque<WorkspaceSymbolProvider>,
    tickets: Vec<WorkspaceSymbolTicket>,
    awaiting: HashSet<u64>,
    query: String,
    changed: Option<Instant>,
    started: Instant,
    items: Vec<Item>,
    selected: Option<Selected>,
    bytes: usize,
    failed: usize,
    ready: bool,
}
#[derive(Default)]
pub(super) struct State {
    run: Option<Run>,
}
impl App {
    pub(super) fn workspace_provider_symbols_active(&self) -> bool {
        self.workspace_symbols.run.is_some()
    }
    pub(super) fn workspace_provider_symbols_waiting(&self) -> bool {
        self.workspace_symbols
            .run
            .as_ref()
            .is_some_and(|run| !run.ready || run.selected.is_some())
    }
    pub(super) fn workspace_provider_symbol_items(&self, query: &str) -> Vec<&Symbol> {
        let Some(run) = &self.workspace_symbols.run else {
            return Vec::new();
        };
        let mut items: Vec<_> = run
            .items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                score(&item.display.label, query).map(|score| (score, idx, &item.display))
            })
            .collect();
        items.sort_by_key(|(score, idx, _)| (std::cmp::Reverse(*score), *idx));
        items.into_iter().map(|(_, _, item)| item).collect()
    }
    pub(super) fn cancel_workspace_provider_symbols(&mut self) {
        if let Some(run) = self.workspace_symbols.run.take()
            && let Some(host) = &mut self.extension_host
        {
            for ticket in run.tickets {
                let _ = host.cancel_workspace_symbol(&ticket);
            }
            if let Some(selected) = run.selected
                && let Some(ticket) = selected.request
            {
                let _ = host.cancel_workspace_symbol(&ticket);
            }
        }
    }
    pub(super) fn start_workspace_provider_symbols(&mut self) -> bool {
        let Some(host) = &self.extension_host else {
            return false;
        };
        let cohort = match host.workspace_symbol_providers() {
            Ok(cohort) if !cohort.is_empty() => cohort,
            Ok(_) => return false,
            Err(error) => {
                self.message = format!("Workspace symbols: {error:#}");
                return true;
            }
        };
        self.focus = Focus::Editor;
        self.start_prompt(PromptKind::Symbols, String::new());
        self.workspace_symbols.run = Some(Run {
            context: Context::capture(self),
            waiting: cohort.clone().into(),
            cohort,
            tickets: Vec::new(),
            awaiting: HashSet::new(),
            query: String::new(),
            changed: None,
            started: Instant::now(),
            items: Vec::new(),
            selected: None,
            bytes: 0,
            failed: 0,
            ready: false,
        });
        self.message = "Loading workspace symbols…".into();
        self.dispatch_workspace_provider_symbols();
        true
    }
    fn workspace_provider_symbols_current(&self) -> bool {
        self.workspace_symbol_source_current()
            && self.workspace_symbols.run.as_ref().is_some_and(|run| {
                if let Some(selected) = &run.selected {
                    selected.navigation.valid(self) && self.prompt.is_none()
                } else {
                    matches!(
                        self.prompt.as_ref().map(|prompt| &prompt.kind),
                        Some(PromptKind::Symbols)
                    )
                }
            })
    }
    fn workspace_symbol_source_current(&self) -> bool {
        self.workspace_symbols.run.as_ref().is_some_and(|run| {
            run.context.current(self)
                && self.extension_host.as_ref().is_some_and(|host| {
                    run.cohort
                        .iter()
                        .all(|source| host.workspace_symbol_provider_current(source))
                })
        })
    }
    fn dispatch_workspace_provider_symbols(&mut self) {
        if !self.workspace_provider_symbols_current() {
            return;
        }
        let host = self.extension_host.as_mut().unwrap();
        let run = self.workspace_symbols.run.as_mut().unwrap();
        if run.changed.is_some() {
            return;
        }
        if let Some(selected) = &mut run.selected {
            if selected.request.is_none() && host.language_provider_capacity() {
                match host
                    .request_workspace_symbol_resolve(&selected.item.ticket, &selected.item.value)
                {
                    Ok(ticket) => selected.request = Some(ticket),
                    Err(error) => {
                        self.message = format!("Workspace symbol resolve: {error:#}");
                        self.cancel_workspace_provider_symbols();
                    }
                }
            }
            return;
        }
        while host.language_provider_capacity() {
            let Some(source) = run.waiting.pop_front() else {
                break;
            };
            match host.request_workspace_symbols_from(&source, &run.query) {
                Ok(ticket) => {
                    run.awaiting.insert(ticket.id);
                    run.tickets.push(ticket);
                }
                Err(error) => {
                    // A pre-wire admission failure is not an isolated provider
                    // exception. Never publish a silently incomplete cohort.
                    self.message = format!("Workspace symbol cohort dispatch rejected: {error:#}");
                    self.cancel_workspace_provider_symbols();
                    return;
                }
            }
        }
    }
    pub(super) fn poll_workspace_provider_symbols(&mut self) -> bool {
        let replies = self
            .extension_host
            .as_mut()
            .map(|host| host.take_workspace_symbol_replies())
            .unwrap_or_default();
        let mut changed = !replies.is_empty();
        if self.workspace_symbols.run.is_none() {
            return changed;
        }
        if !self.workspace_provider_symbols_current() {
            self.cancel_workspace_provider_symbols();
            return true;
        }
        // Query supersession is observed before consuming a queued old list.
        let query = self.prompt.as_ref().map(|prompt| prompt.text.clone());
        if let Some(query) = query
            && self
                .workspace_symbols
                .run
                .as_ref()
                .is_some_and(|run| run.selected.is_none() && run.query != query)
        {
            let run = self.workspace_symbols.run.as_mut().unwrap();
            if let Some(host) = &mut self.extension_host {
                for ticket in run.tickets.drain(..) {
                    let _ = host.cancel_workspace_symbol(&ticket);
                }
            }
            run.query = query;
            run.items.clear();
            run.awaiting.clear();
            run.waiting = run.cohort.clone().into();
            run.bytes = 0;
            run.failed = 0;
            run.ready = false;
            run.changed = Some(Instant::now());
            run.started = Instant::now();
            self.message = "Waiting to search workspace symbols…".into();
            changed = true;
        }
        for reply in replies {
            if self.workspace_symbols.run.is_none() {
                break;
            }
            let resolving = self
                .workspace_symbols
                .run
                .as_ref()
                .and_then(|run| run.selected.as_ref())
                .and_then(|selected| selected.request.as_ref())
                .is_some_and(|ticket| ticket.id == reply.ticket.id);
            if resolving {
                let run = self.workspace_symbols.run.take().unwrap();
                let selected = run.selected.unwrap();
                let result = reply.result.map_err(anyhow::Error::msg).and_then(|value| {
                    let value = if value.is_null() {
                        selected.item.value
                    } else {
                        value
                    };
                    let item = parse_item(selected.item.ticket.clone(), value)?;
                    self.open_global_symbol(
                        &item.uri,
                        item.range,
                        selected.item.ticket.clone(),
                        run.context.clone(),
                    )
                });
                if let Some(host) = &mut self.extension_host {
                    for ticket in run.tickets {
                        let _ = host.cancel_workspace_symbol(&ticket);
                    }
                }
                if let Err(error) = result {
                    self.message = format!("Workspace symbol navigation: {error:#}");
                }
                continue;
            }
            let run = self.workspace_symbols.run.as_mut().unwrap();
            if !run.awaiting.remove(&reply.ticket.id) {
                continue;
            }
            let value = match reply.result {
                Ok(value) => value,
                Err(_) => {
                    run.failed += 1;
                    continue;
                }
            };
            let result = (|| -> Result<()> {
                let entries = value["symbols"]
                    .as_array()
                    .context("Invalid workspace symbol list")?;
                ensure!(
                    run.items.len().saturating_add(entries.len()) <= LIMIT,
                    "Workspace symbols exceed 512 entries; narrow the query"
                );
                let bytes = crate::decorations::json_budget(&value, BYTES)?;
                ensure!(
                    run.bytes.saturating_add(bytes) <= BYTES,
                    "Workspace symbol cohort exceeds 1 MiB"
                );
                let mut prepared = Vec::new();
                for value in entries {
                    prepared.push(parse_item(reply.ticket.clone(), value.clone())?);
                }
                run.bytes += bytes;
                run.items.extend(prepared);
                Ok(())
            })();
            if let Err(error) = result {
                self.cancel_workspace_provider_symbols();
                self.message = format!("Workspace symbol cohort rejected: {error:#}");
                return true;
            }
        }
        let Some(run) = &mut self.workspace_symbols.run else {
            return changed;
        };
        if run.started.elapsed() >= DEADLINE && (!run.ready || run.selected.is_some()) {
            self.cancel_workspace_provider_symbols();
            self.message = "Workspace symbol search deadline elapsed".into();
            return true;
        }
        if run
            .changed
            .is_some_and(|when| when.elapsed() >= Duration::from_millis(200))
        {
            run.changed = None;
            changed = true;
        }
        self.dispatch_workspace_provider_symbols();
        let Some(run) = &mut self.workspace_symbols.run else {
            return changed;
        };
        if !run.ready
            && run.selected.is_none()
            && run.changed.is_none()
            && run.waiting.is_empty()
            && run.awaiting.is_empty()
        {
            run.items.sort_by(compare_items);
            // The source groups against the FIRST item of each sorted group.
            // Intersection equality is not transitive; retain that exact group head.
            let mut unique: Vec<Item> = Vec::new();
            for item in std::mem::take(&mut run.items) {
                if unique
                    .last()
                    .is_none_or(|first| compare_items(first, &item) != std::cmp::Ordering::Equal)
                {
                    unique.push(item);
                }
            }
            run.items = unique;
            run.ready = true;
            self.message = format!(
                "{} workspace symbols for '{}' · Enter navigates{}",
                run.items.len(),
                run.query,
                if run.failed > 0 {
                    format!(" · {} providers failed", run.failed)
                } else {
                    String::new()
                }
            );
            changed = true;
        }
        changed
    }
    pub(super) fn accept_workspace_provider_symbol(&mut self, query: &str, selected: usize) {
        let result = (|| -> Result<()> {
            ensure!(
                self.workspace_symbol_source_current(),
                "Workspace symbol source expired"
            );
            let run = self.workspace_symbols.run.as_ref().unwrap();
            ensure!(
                run.ready && run.query == query,
                "Wait for symbols matching the current query"
            );
            let items = self.workspace_provider_symbol_items(query);
            let display = *items
                .get(selected.min(items.len().saturating_sub(1)))
                .context("No matching workspace symbols")?;
            let index = run
                .items
                .iter()
                .position(|item| std::ptr::eq(&item.display, display))
                .context("Workspace symbol item expired")?;
            super::extension_services::Context::check_budget(self)?;
            let navigation = super::symbols::Context::capture(self);
            let run = self.workspace_symbols.run.as_mut().unwrap();
            let item = run.items.remove(index);
            if item.ticket.provider.resolves {
                run.selected = Some(Selected {
                    item,
                    request: None,
                    navigation,
                });
                run.started = Instant::now();
                self.message = "Resolving workspace symbol…".into();
                self.dispatch_workspace_provider_symbols();
            } else {
                let context = run.context.clone();
                let ticket = item.ticket.clone();
                self.open_global_symbol(&item.uri, item.range, ticket, context)?;
                self.cancel_workspace_provider_symbols();
            }
            Ok(())
        })();
        if let Err(error) = result {
            if self
                .workspace_symbols
                .run
                .as_ref()
                .is_some_and(|run| !run.ready)
            {
                self.prompt = Some(Prompt::new(PromptKind::Symbols, query.into()));
            } else {
                self.cancel_workspace_provider_symbols();
            }
            self.message = format!("Workspace symbols: {error:#}");
        }
    }
}
fn parse_item(ticket: WorkspaceSymbolTicket, value: Value) -> Result<Item> {
    crate::decorations::json_budget(&value, BYTES)?;
    let text = |field: &Value, name: &str| -> Result<String> {
        let value = field
            .as_str()
            .with_context(|| format!("Invalid workspace symbol {name}"))?;
        ensure!(
            value.len() <= 4096 && !value.contains('\0'),
            "Workspace symbol {name} exceeds bounds"
        );
        Ok(value.into())
    };
    let name = text(&value["name"], "name")?;
    ensure!(!name.is_empty(), "Empty workspace symbol name");
    let kind = value["kind"]
        .as_u64()
        .filter(|kind| (1..=26).contains(kind))
        .context("Invalid workspace symbol kind")?;
    let uri = text(&value["location"]["uri"], "URI")?;
    url::Url::parse(&uri)?;
    let container = value
        .get("containerName")
        .map(|value| text(value, "container"))
        .transpose()?
        .unwrap_or_default();
    let range = value["location"]
        .get("range")
        .map(|value| -> Result<lsp::Range> {
            let range: lsp::Range = serde_json::from_value(value.clone())?;
            ensure!(
                (range.start.line, range.start.character) <= (range.end.line, range.end.character)
                    && [
                        range.start.line,
                        range.start.character,
                        range.end.line,
                        range.end.character
                    ]
                    .iter()
                    .all(|value| *value <= i32::MAX as usize),
                "Invalid workspace symbol range"
            );
            Ok(range)
        })
        .transpose()?;
    let path = url::Url::parse(&uri)?
        .to_file_path()
        .unwrap_or_else(|_| PathBuf::from(&uri));
    let origin = lsp::Position {
        line: 0,
        character: 0,
    };
    let display = Symbol {
        label: format!(
            "{name}  [{kind}]  {container}  {uri}{}",
            range
                .map(|range| format!(":{}", range.start.line + 1))
                .unwrap_or_default()
        ),
        path,
        range: range.unwrap_or(lsp::Range {
            start: origin,
            end: origin,
        }),
    };
    Ok(Item {
        ticket,
        value,
        name,
        kind,
        uri,
        range,
        container,
        display,
    })
}
fn compare_items(a: &Item, b: &Item) -> std::cmp::Ordering {
    let text = |a: &str, b: &str| a.encode_utf16().cmp(b.encode_utf16());
    text(&a.name, &b.name)
        .then_with(|| a.kind.cmp(&b.kind))
        .then_with(|| text(&a.uri, &b.uri))
        .then_with(|| {
            compare_locations(
                a.range,
                a.ticket.provider.resolves,
                b.range,
                b.ticket.provider.resolves,
            )
        })
        .then_with(|| text(&a.container, &b.container))
}
fn compare_locations(
    a: Option<lsp::Range>,
    a_resolves: bool,
    b: Option<lsp::Range>,
    b_resolves: bool,
) -> std::cmp::Ordering {
    if let (Some(a), Some(b)) = (a, b) {
        let (as_, ae) = (
            (a.start.line, a.start.character),
            (a.end.line, a.end.character),
        );
        let (bs, be) = (
            (b.start.line, b.start.character),
            (b.end.line, b.end.character),
        );
        if as_ < be && bs < ae {
            std::cmp::Ordering::Equal
        } else {
            as_.cmp(&bs).then_with(|| ae.cmp(&be))
        }
    } else {
        b_resolves.cmp(&a_resolves)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_symbol_range_order_distinguishes_overlap_touch_and_missing_resolver() {
        let range = |start, end| {
            Some(lsp::Range {
                start: lsp::Position {
                    line: 0,
                    character: start,
                },
                end: lsp::Position {
                    line: 0,
                    character: end,
                },
            })
        };
        let (a, b, c) = (range(0, 2), range(1, 3), range(2, 4));
        assert_eq!(
            compare_locations(a, false, b, false),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_locations(b, false, c, false),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_locations(a, false, c, false),
            std::cmp::Ordering::Less,
            "Touching ranges are not intersecting"
        );
        assert_eq!(
            compare_locations(None, true, a, false),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_locations(None, false, a, true),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_locations(a, true, b, false),
            std::cmp::Ordering::Equal,
            "Resolver preference applies only when at least one range is absent"
        );
        assert_eq!(
            compare_locations(range(2, 2), false, range(2, 2), false),
            std::cmp::Ordering::Equal
        );
    }
}
#[test]
fn oversized_global_symbol_target_refuses_before_model_focus_or_optional_host_loss() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("too-big.cpp");
    std::fs::write(&target, vec![b'x'; 4 * 1024 * 1024]).unwrap();
    let package = root.path().join("provider");
    std::fs::create_dir(&package).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"publisher":"test","name":"big-symbol","version":"1","main":"index.cjs"}"#,
    )
    .unwrap();
    std::fs::write(package.join("index.cjs"),r#"const v=require('vscode');exports.activate=()=>v.languages.registerWorkspaceSymbolProvider({provideWorkspaceSymbols(){return [new v.SymbolInformation('big',v.SymbolKind.Function,'',new v.Location(v.Uri.joinPath(v.workspace.workspaceFolders[0].uri,'too-big.cpp'),new v.Range(0,0,0,1)))];}});"#).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("猫🙂 unsaved\r\n", false);
    let original = (
        app.doc().id,
        app.doc().text.to_string(),
        app.doc().selections(),
    );
    let saved = app
        .doc()
        .capture_save(root.path().join("original.cpp"))
        .unwrap();
    app.start_extension_packages(vec![crate::extensions::Package::read(&package).unwrap()])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.extension_host.as_ref().is_some_and(|host| {
        host.ready
            && host
                .workspace_symbol_providers()
                .is_ok_and(|providers| providers.len() == 1)
    }) {
        app.poll();
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::yield_now();
    }
    app.execute("workbench.action.showAllSymbols", Value::Null);
    while app.native_symbol_picker_waiting() {
        app.poll();
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::yield_now();
    }
    assert_eq!(app.symbol_items("").len(), 1);
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    while !app.message.starts_with("Symbol navigation:") {
        app.poll();
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::yield_now();
    }
    assert!(app.message.contains("budget"), "{}", app.message);
    assert_eq!(app.documents.len(), 1);
    assert!(app.hidden_documents.is_empty());
    assert_eq!(
        (
            app.doc().id,
            app.doc().text.to_string(),
            app.doc().selections()
        ),
        original
    );
    assert!(app.extension_host.as_ref().is_some_and(|host| {
        host.ready
            && host
                .workspace_symbol_providers()
                .is_ok_and(|providers| providers.len() == 1)
    }));
    let current = app
        .doc()
        .capture_save(root.path().join("original.cpp"))
        .unwrap();
    assert_eq!(
        (current.text_epoch(), current.save_generation()),
        (saved.text_epoch(), saved.save_generation())
    );
    app.doc_mut().undo();
    assert!(app.doc().is_empty());
    app.doc_mut().redo();
    assert_eq!(app.doc().text.to_string(), original.1);
    assert_eq!(std::fs::metadata(target).unwrap().len(), 4 * 1024 * 1024);
}
