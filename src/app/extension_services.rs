use super::*;
use crate::extensions::services::{NativeService, Operation, SERVICE_LIFETIME};
use anyhow::{Context as _, bail};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
const MAX_HIDDEN: usize = 16;
const MAX_MODELS: usize = 128;
const MAX_CONTEXT_SELECTIONS: usize = 16_384;
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, PartialEq)]
struct DocumentContext {
    id: u64,
    epoch: u64,
    revision: u64,
    saved: u64,
    path: Option<PathBuf>,
    selections: Vec<crate::document::Selection>,
}
#[derive(Clone, PartialEq)]
pub(super) struct Context {
    snapshot: Option<Snapshot>,
}
#[derive(Clone, PartialEq)]
struct Snapshot {
    epoch: u64,
    workspace: PathBuf,
    focus: Focus,
    active: Option<u64>,
    panes: Vec<(u64, u64)>,
    pane_selections: Vec<(usize, Option<usize>, Vec<crate::document::Selection>)>,
    active_pane: usize,
    visible: usize,
    documents: Vec<DocumentContext>,
}
impl Context {
    pub(super) fn check_budget(app: &App) -> Result<()> {
        if app
            .documents
            .len()
            .saturating_add(app.hidden_documents.len())
            > MAX_MODELS
        {
            bail!("Native extension contexts support at most 128 retained models");
        }
        // Count the actual retained snapshots, including an active view that is
        // captured both as a document and as a pane. Inspect lengths only.
        let mut selections = 0usize;
        let mut add = |secondary: usize| -> Result<()> {
            selections = selections.saturating_add(secondary).saturating_add(1);
            if selections > MAX_CONTEXT_SELECTIONS {
                bail!(
                    "Native extension context exceeds 16,384 document/pane selections; reduce selections and retry"
                );
            }
            Ok(())
        };
        for doc in app.documents.iter().chain(&app.hidden_documents) {
            add(doc.secondary.len())?;
        }
        for pane in &app.panes {
            if let Some(doc) = app.documents.iter().find(|doc| doc.id == pane.document) {
                add(doc.view_state(Some(pane.id)).secondary.len())?;
            }
        }
        Ok(())
    }
    pub(super) fn capture(app: &App) -> Self {
        if Self::check_budget(app).is_err() {
            // Preserve the infallible caller API, but never compare two invalid
            // captures as an authorized context or clone over-budget selections.
            return Self { snapshot: None };
        }
        Self {
            snapshot: Some(Snapshot {
                epoch: app.extension_services.epoch,
                workspace: app.workspace.root.clone(),
                focus: app.focus.clone(),
                active: app.active_document().map(|doc| doc.id),
                panes: app.panes.iter().map(|p| (p.id, p.document)).collect(),
                pane_selections: app
                    .panes
                    .iter()
                    .filter_map(|p| {
                        app.documents.iter().find(|d| d.id == p.document).map(|d| {
                            let view = d.view_state(Some(p.id));
                            (view.cursor, view.anchor, view.secondary.clone())
                        })
                    })
                    .collect(),
                active_pane: app.active_pane,
                visible: app.documents.len(),
                documents: app
                    .documents
                    .iter()
                    .chain(&app.hidden_documents)
                    .map(|d| DocumentContext {
                        id: d.id,
                        epoch: d.text_epoch(),
                        revision: d.revision,
                        saved: d.saved_revision,
                        path: d.path.clone(),
                        selections: d.selections(),
                    })
                    .collect(),
            }),
        }
    }
    pub(super) fn same_editor(&self, app: &App) -> bool {
        self.snapshot.is_some() && self == &Self::capture(app)
    }
    pub(super) fn accept_next_interaction(&mut self) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.epoch = snapshot.epoch.wrapping_add(1);
        }
    }
    fn valid(&self, app: &App) -> bool {
        self.same_editor(app) && app.prompt.is_none() && app.modal.is_none()
    }
}
enum Output {
    Existing(PathBuf),
    Loaded(Box<Document>),
    State(Value),
}
struct Job {
    service: NativeService,
    context: Option<Context>,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<Output, String>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
#[derive(Default)]
pub(super) struct State {
    pub config_root: Option<PathBuf>,
    pub epoch: u64,
    job: Option<Job>,
    contexts: std::collections::HashMap<(u64, String), Context>,
}
pub(super) fn resolved(path: &Path, workspace: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    };
    if let Ok(canonical) = std::fs::canonicalize(&path) {
        return Ok(canonical);
    }
    Ok(
        std::fs::canonicalize(path.parent().context("Document path has no parent")?)?
            .join(path.file_name().context("Document path has no filename")?),
    )
}
impl App {
    pub(super) fn cancel_extension_services(&mut self) {
        if let Some(job) = &self.extension_services.job {
            job.cancel.store(true, Ordering::Release);
        }
    }
    pub fn configure_extension_storage(&mut self, root: Option<&Path>) {
        self.extension_services.config_root = root.map(Path::to_owned);
    }
    pub(super) fn poll_extension_services(&mut self) -> bool {
        let Some(mut host) = self.extension_host.take() else {
            self.extension_services.contexts.clear();
            if let Some(job) = &self.extension_services.job {
                job.cancel.store(true, Ordering::Release);
            }
            // Retain an occupied worker until it actually finishes even after host retirement.
            if self
                .extension_services
                .job
                .as_ref()
                .is_some_and(|job| !matches!(job.receiver.try_recv(), Err(TryRecvError::Empty)))
            {
                self.extension_services.job = None;
            }
            return false;
        };
        let changed = self.poll_services_with_host(&mut host);
        self.extension_host = Some(host);
        changed
    }
    fn poll_services_with_host(&mut self, host: &mut crate::extensions::Client) -> bool {
        // Capture every queued document operation when first observed by App,
        // including requests waiting behind a filesystem write. A later focus,
        // pane or input change must not become their new authorization context.
        let pending: std::collections::HashSet<_> = host
            .queued_services()
            .map(|service| (service.request.session, service.id.clone()))
            .collect();
        self.extension_services
            .contexts
            .retain(|key, _| pending.contains(key));
        for service in host
            .queued_services()
            .filter(|service| !matches!(service.operation, Operation::State(_)))
        {
            let key = (service.request.session, service.id.clone());
            if !self.extension_services.contexts.contains_key(&key) {
                // An over-budget receipt is permanently invalid too. Never
                // rebind a queued operation after the user reduces selections.
                let context = Context::capture(self);
                self.extension_services.contexts.insert(key, context);
            }
        }
        if let Some(job) = &self.extension_services.job {
            let valid = host.service_valid(&job.service).is_ok()
                && job
                    .context
                    .as_ref()
                    .is_none_or(|context| context.valid(self));
            if !valid {
                job.cancel.store(true, Ordering::Release);
            }
            let result = match job.receiver.try_recv() {
                Ok(result) => result.map_err(anyhow::Error::msg),
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    Err(anyhow::anyhow!("Native service worker stopped"))
                }
            };
            let job = self.extension_services.job.take().unwrap();
            let result = if valid && !job.cancel.load(Ordering::Acquire) {
                result.and_then(|output| self.install_service_output(output))
            } else {
                Err(anyhow::anyhow!(
                    "Native service cancelled by newer editor or host context; retry"
                ))
            };
            self.answer_native_service(host, &job.service, result);
            return true;
        }
        let Some(service) = host.service().cloned() else {
            return false;
        };
        let validation = host.service_valid(&service).and_then(|()| {
            if !matches!(service.operation, Operation::State(_)) {
                Context::check_budget(self)?;
            }
            if !matches!(service.operation, Operation::State(_))
                && !self
                    .extension_services
                    .contexts
                    .get(&(service.request.session, service.id.clone()))
                    .is_some_and(|context| context.valid(self))
            {
                bail!("Native document service cancelled by newer editor context; retry");
            }
            if !matches!(service.operation, Operation::State(_))
                && (self.prompt.is_some() || self.modal.is_some())
            {
                bail!("Native document service cannot replace an active prompt or modal");
            }
            Ok(())
        });
        if let Err(error) = validation {
            self.answer_native_service(host, &service, Err(error));
            return true;
        }
        match &service.operation {
            Operation::Open(open) => {
                let path = PathBuf::from(&open.path);
                let path = if path.is_absolute() {
                    path
                } else {
                    self.workspace.root.join(path)
                };
                if let Some(doc) = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|d| d.path.as_ref() == Some(&path))
                {
                    self.answer_native_service(
                        host,
                        &service,
                        Ok(serde_json::json!({"document":doc.id})),
                    );
                    return true;
                }
                let remaining = MAX_BYTES.saturating_sub(
                    self.documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .map(|d| d.text.len_bytes())
                        .sum::<usize>(),
                );
                let existing: Vec<_> = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .filter_map(|d| d.path.clone())
                    .collect();
                let workspace = self.workspace.root.clone();
                let capacity = self.hidden_documents.len() < MAX_HIDDEN;
                let (sender, receiver) = mpsc::sync_channel(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                let started = service.started;
                std::thread::spawn(move || {
                    let result = (|| -> Result<Output> {
                        if flag.load(Ordering::Acquire) || started.elapsed() >= SERVICE_LIFETIME {
                            bail!("Document open cancelled");
                        }
                        let path = resolved(&path, &workspace)?;
                        if existing.contains(&path) {
                            return Ok(Output::Existing(path));
                        }
                        if !capacity {
                            bail!("Hidden document limit reached (16)");
                        }
                        let doc = Document::open_existing_bounded(&path, remaining as u64)?;
                        if flag.load(Ordering::Acquire) || started.elapsed() >= SERVICE_LIFETIME {
                            bail!("Document open cancelled");
                        }
                        Ok(Output::Loaded(Box::new(doc)))
                    })()
                    .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
                let context = self
                    .extension_services
                    .contexts
                    .get(&(service.request.session, service.id.clone()))
                    .cloned();
                self.extension_services.job = Some(Job {
                    service,
                    context,
                    cancel,
                    receiver,
                });
            }
            Operation::State(patch) => {
                let Some(store) = host.state_store() else {
                    self.answer_native_service(
                        host,
                        &service,
                        Err(anyhow::anyhow!(
                            "Extension state storage was not configured"
                        )),
                    );
                    return true;
                };
                let patch = patch.clone();
                let owner = service.request.owner.clone();
                let started = service.started;
                let (sender, receiver) = mpsc::sync_channel(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                std::thread::spawn(move || {
                    let result = store
                        .update_with(
                            &owner,
                            patch.scope,
                            &patch.key,
                            (!patch.remove).then_some(patch.value),
                            || {
                                if flag.load(Ordering::Acquire)
                                    || started.elapsed() >= SERVICE_LIFETIME
                                {
                                    bail!("Extension state update cancelled before commit");
                                }
                                Ok(())
                            },
                        )
                        .map(|values| Output::State(Value::Object(values)))
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(result);
                });
                self.extension_services.job = Some(Job {
                    service,
                    context: None,
                    cancel,
                    receiver,
                });
            }
            Operation::Show(show) => {
                let result = self.show_extension_document(host, show);
                self.answer_native_service(host, &service, result);
            }
            Operation::Command(command) => {
                let result = if self.active_document().is_none() || self.focus != Focus::Editor {
                    Err(anyhow::anyhow!(
                        "Native delegated command requires the focused editor"
                    ))
                } else {
                    self.execute(&command.id, Value::Null);
                    Ok(Value::Null)
                };
                self.answer_native_service(host, &service, result);
            }
        }
        true
    }
    fn install_service_output(&mut self, output: Output) -> Result<Value> {
        match output {
            Output::State(values) => Ok(serde_json::json!({"values":values})),
            Output::Existing(path) => {
                let doc = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|d| d.path.as_ref() == Some(&path))
                    .context("Document closed while resolving its path")?;
                Ok(serde_json::json!({"document":doc.id}))
            }
            Output::Loaded(mut doc) => {
                if let Some(existing) = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|d| d.path == doc.path)
                {
                    return Ok(serde_json::json!({"document":existing.id}));
                }
                if self.hidden_documents.len() >= MAX_HIDDEN
                    || self
                        .documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .map(|d| d.text.len_bytes())
                        .sum::<usize>()
                        + doc.text.len_bytes()
                        > MAX_BYTES
                {
                    bail!("Hidden document or mirror budget changed; retry");
                }
                self.settings.apply(&mut doc);
                let id = doc.id;
                self.hidden_documents.push(*doc);
                Ok(serde_json::json!({"document":id}))
            }
        }
    }
    fn show_extension_document(
        &mut self,
        host: &crate::extensions::Client,
        show: &crate::extensions::services::Show,
    ) -> Result<Value> {
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|d| d.id == show.document)
            .context("Document is no longer open")?;
        if !host.service_document_current(doc, show.version) {
            bail!("Document changed before showTextDocument; retry");
        }
        let selection = show
            .selection
            .as_ref()
            .map(|range| -> Result<_> {
                let start = crate::lsp::offset(doc, range.start)?;
                let end = crate::lsp::offset(doc, range.end)?;
                if start > end {
                    bail!("Selection range is reversed");
                }
                Ok((start, end))
            })
            .transpose()?;
        if let Some(index) = self
            .hidden_documents
            .iter()
            .position(|d| d.id == show.document)
        {
            let doc = self.hidden_documents.remove(index);
            self.install_open_document(doc);
        } else {
            self.active = self
                .documents
                .iter()
                .position(|d| d.id == show.document)
                .unwrap();
            self.focus = Focus::Editor;
            self.sync_pane();
        }
        if let Some((start, end)) = selection {
            self.doc_mut().clear_secondary();
            self.doc_mut().move_to(start, false);
            self.doc_mut().move_to(end, true);
            self.sync_pane();
        }
        self.remember_active_file();
        Ok(serde_json::json!({"document":show.document}))
    }
    fn answer_native_service(
        &mut self,
        host: &mut crate::extensions::Client,
        service: &NativeService,
        result: Result<Value>,
    ) {
        self.extension_services
            .contexts
            .remove(&(service.request.session, service.id.clone()));
        if service.request.session != host.session {
            return;
        }
        // The full authoritative mirror is queued before the promise settles.
        let result = result.and_then(|value| {
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)?;
            Ok(value)
        });
        if let Err(error) = host.answer_service(service, result) {
            self.message = format!("Native extension service reply failed: {error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    fn fixture(root: &Path) -> PathBuf {
        let folder = root.join("extension");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("package.json"),r#"{"publisher":"test","name":"documents","version":"1.0.0","main":"index.cjs","contributes":{"commands":[{"command":"documents.show","title":"Show hidden model"},{"command":"documents.edit","title":"Edit native model"},{"command":"documents.state","title":"Update state"}]}}"#).unwrap();
        std::fs::write(folder.join("index.cjs"),r#"
const vscode = require('vscode'); const assert = require('node:assert/strict');
exports.activate = async context => {
  let opened; vscode.workspace.onDidOpenTextDocument(doc=>{opened=doc; assert.equal(vscode.window.activeTextEditor,undefined);});
  const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(require('node:path').join(vscode.workspace.rootPath,'input.txt')));
  assert.equal(opened,doc); assert.equal(vscode.window.activeTextEditor,undefined); assert.deepEqual(vscode.window.visibleTextEditors,[]);
  assert.equal(doc.getText(),'α😀\r\nsecond');
  await context.globalState.update('launches',context.globalState.get('launches',0)+1);
  vscode.commands.registerCommand('documents.show',async()=>{
    const editor=await vscode.window.showTextDocument(doc,{preview:false,selection:new vscode.Range(0,1,0,3)});
    assert.equal(editor.document,doc); assert.equal(vscode.window.activeTextEditor,editor); assert.equal(editor.selection.end.character,3);
    await vscode.window.showInformationMessage('shown same identity');
  });
  vscode.commands.registerCommand('documents.edit',async()=>{
    const editor=vscode.window.activeTextEditor;
    assert(await editor.edit(edit=>edit.insert(new vscode.Position(1,0),'changed ')));
    await vscode.commands.executeCommand('undo'); assert.equal(doc.getText(),'α😀\r\nsecond');
    await vscode.commands.executeCommand('redo'); assert.equal(doc.getText(),'α😀\r\nchanged second');
    await vscode.window.showInformationMessage('native undo redo mirrored');
  });
  vscode.commands.registerCommand('documents.unawaited',()=>{
    void context.globalState.update('unawaited','persisted');
    void vscode.window.showInformationMessage('returned unawaited update');
  });
  vscode.commands.registerCommand('documents.state',async()=>{
    await Promise.all([context.globalState.update('__proto__',null),context.workspaceState.update('workspace','local')]);
    assert(context.globalState.keys().includes('__proto__')); assert.equal(context.globalState.get('__proto__'),null);
    await vscode.window.showInformationMessage('state '+context.globalState.get('launches'));
  });
};
"#).unwrap();
        folder
    }
    #[test]
    fn activation_opens_hidden_then_shows_same_model_native_undo_and_persistent_state() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.txt"), "α😀\r\nsecond").unwrap();
        let package = crate::extensions::Package::read(&fixture(root.path())).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_extension_storage(Some(config.path()));
        app.start_extension_packages(vec![package]).unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        assert!(app.documents.is_empty());
        assert!(app.active_document().is_none());
        assert!(app.panes.is_empty());
        assert_eq!(app.hidden_documents.len(), 1);
        let id = app.hidden_documents[0].id;
        app.execute("documents.show", Value::Null);
        until(&mut app, |app| app.message.contains("shown same identity"));
        assert_eq!(app.doc().id, id);
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.doc().selections().len(), 1);
        assert_eq!(app.doc().selected_text().as_deref(), Some("😀"));
        app.execute("documents.edit", Value::Null);
        until(&mut app, |app| {
            app.message.contains("native undo redo mirrored")
        });
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "α😀\r\nchanged second");
        assert_eq!(
            std::fs::read_to_string(root.path().join("input.txt")).unwrap(),
            "α😀\r\nsecond"
        );
        app.execute("documents.state", Value::Null);
        until(&mut app, |app| app.message.contains("state 1"));
        app.execute("documents.unawaited", Value::Null);
        until(&mut app, |app| {
            app.message.contains("returned unawaited update")
                && app.extension_host.as_ref().is_some_and(|host| !host.busy())
                && app.extension_services.job.is_none()
        });
        let store = crate::extension_state::Store::new(config.path(), root.path()).unwrap();
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Global)
                .unwrap()["unawaited"],
            "persisted"
        );
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Global)
                .unwrap()["launches"],
            1
        );
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Global)
                .unwrap()["__proto__"],
            Value::Null
        );
        assert_eq!(
            store
                .read("test.documents", crate::extension_state::Scope::Workspace)
                .unwrap()["workspace"],
            "local"
        );
    }
    #[test]
    fn show_validates_utf16_before_visibility_and_preserves_other_shared_view_selections() {
        let root = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents
            .push(Document::from_text("α😀\r\nsecond"));
        let id = app.hidden_documents[0].id;
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &[],
            &app.hidden_documents,
            0,
            &app.settings,
        )
        .unwrap();
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        host.queue_service("nativeDocumentShow",json!("bad-range"),json!({"session":host.session,"owner":owner,"generation":1,"args":{"document":id,"version":1,"selection":{"start":{"line":0,"character":2},"end":{"line":0,"character":3}}}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.documents.is_empty());
        assert_eq!(app.hidden_documents[0].id, id);
        let doc = app.hidden_documents.remove(0);
        app.install_open_document(doc);
        app.doc_mut().move_to(1, false);
        app.split_editor(false);
        app.doc_mut().move_to(5, false);
        app.doc_mut().secondary.push(crate::document::Selection {
            cursor: 7,
            anchor: None,
            desired_column: None,
        });
        let first_pane = app.panes[0].id;
        let second_pane = app.panes[1].id;
        let first_cursor = app.doc().view_state(Some(first_pane)).cursor;
        let mut host = app.extension_host.take().unwrap();
        host.sync_with_hidden(&app.documents, &[], app.active)
            .unwrap();
        // Call the validated application stage directly: transport queue versions
        // and generation rejection are covered separately in service tests.
        app.show_extension_document(
            &host,
            &crate::extensions::services::Show {
                document: id,
                version: 1,
                selection: Some(crate::lsp::Range {
                    start: crate::lsp::Position {
                        line: 0,
                        character: 1,
                    },
                    end: crate::lsp::Position {
                        line: 0,
                        character: 3,
                    },
                }),
            },
        )
        .unwrap();
        assert_eq!(app.panes[app.active_pane].id, second_pane);
        assert_eq!(app.doc().selected_text().as_deref(), Some("😀"));
        assert!(app.doc().secondary.is_empty());
        assert_eq!(app.doc().view_state(Some(first_pane)).cursor, first_cursor);
        assert_eq!(app.doc().text.to_string(), "α😀\r\nsecond");
        app.extension_host = Some(host);
    }
    #[test]
    fn hidden_dirty_recovery_preserves_native_undo_baseline_without_clean_hidden_models() {
        let root = tempfile::tempdir().unwrap();
        let recovery = tempfile::tempdir().unwrap();
        let file = root.path().join("hidden.txt");
        std::fs::write(&file, "original").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let mut dirty = Document::open_existing(&file).unwrap();
        dirty.insert("dirty ", false);
        app.hidden_documents.push(dirty);
        app.hidden_documents
            .push(Document::from_text("clean hidden"));
        assert!(app.documents.is_empty());
        assert_eq!(app.recovery_documents().len(), 1);
        let worker = crate::recovery::Worker::start(
            crate::recovery::Recovery::new(recovery.path().into()).unwrap(),
        )
        .unwrap();
        worker.preserve_refs(&app.recovery_documents()).unwrap();
        let store = crate::recovery::Recovery::new(recovery.path().into()).unwrap();
        let (mut docs, _) = store.restore().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].text.to_string(), "dirty original");
        assert!(docs[0].dirty());
        docs[0].undo();
        assert_eq!(docs[0].text.to_string(), "original");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
    }
    #[test]
    fn stopping_a_host_cancels_accepted_state_before_commit_and_preserves_prior_payload() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        let prepared = crate::extensions::Client::prepare(&[], 0, &app.settings)
            .unwrap()
            .with_storage_root(Some(config.path().into()));
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        let store = host.state_store().unwrap();
        store
            .update(
                &owner,
                crate::extension_state::Scope::Global,
                "retained",
                Some(json!(1)),
            )
            .unwrap();
        let owner_path = std::fs::read_dir(config.path().join("state/extension-storage"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let payload = owner_path.join("global.json");
        let before = std::fs::read(&payload).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(owner_path.join("global.lock"))
            .unwrap();
        lock.lock().unwrap();
        host.queue_service("nativeStateWrite",json!("pending-write"),json!({"session":host.session,"owner":owner,"command":1,"commandOwner":owner,"args":{"scope":"global","key":"retained","remove":false,"value":2}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.extension_services.job.is_some());
        app.stop_extension_host();
        assert!(app.extension_services.job.is_some());
        drop(lock);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(std::fs::read(payload).unwrap(), before);
        assert_eq!(
            store
                .read(&owner, crate::extension_state::Scope::Global)
                .unwrap()["retained"],
            1
        );
    }
    #[test]
    fn queued_show_keeps_receipt_context_while_waiting_behind_state_commit() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents.push(Document::from_text("retained"));
        let id = app.hidden_documents[0].id;
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &[],
            &app.hidden_documents,
            0,
            &app.settings,
        )
        .unwrap()
        .with_storage_root(Some(config.path().into()));
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        let owner_path = std::fs::read_dir(config.path().join("state/extension-storage"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(owner_path.join("global.lock"))
            .unwrap();
        lock.lock().unwrap();
        host.queue_service("nativeStateWrite",json!("first-state"),json!({"session":host.session,"owner":owner,"args":{"scope":"global","key":"first","remove":false,"value":true}})).unwrap();
        host.queue_service("nativeDocumentShow",json!("queued-show"),json!({"session":host.session,"owner":owner,"generation":1,"args":{"document":id,"version":1,"selection":null}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.extension_services.job.is_some());
        app.focus = Focus::Explorer;
        drop(lock);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_extension_services();
        assert!(app.documents.is_empty());
        assert_eq!(app.hidden_documents[0].id, id);
        assert!(app.focus == Focus::Explorer);
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        let store = crate::extension_state::Store::new(config.path(), root.path()).unwrap();
        assert_eq!(
            store
                .read(&owner, crate::extension_state::Scope::Global)
                .unwrap()["first"],
            true
        );
    }
    #[test]
    fn over_budget_queued_services_cannot_rebind_after_held_worker_and_selection_reduction() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("dirty.txt");
        let unopened = root.path().join("unopened.txt");
        std::fs::write(&path, "original猫\r\n").unwrap();
        std::fs::write(&unopened, "must stay unopened").unwrap();
        let mut dirty = Document::open_existing(&path).unwrap();
        dirty.insert("dirty ", false);
        dirty.secondary = vec![crate::document::Selection::caret(0); 8192];
        let id = dirty.id;
        let revision = dirty.revision;
        let text = dirty.text.to_string();
        let mut other = Document::default();
        other.secondary = vec![crate::document::Selection::caret(0); 8191];
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents = vec![dirty, other];
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let prepared = crate::extensions::Client::prepare_with_hidden(
            &[],
            &app.hidden_documents,
            0,
            &app.settings,
        )
        .unwrap();
        let mut host =
            crate::extensions::Client::start_prepared("node", &extension, root.path(), prepared)
                .unwrap();
        let owner = host.packages[0].id.clone();
        let session = host.session;
        host.queue_service("nativeStateWrite",json!("held-state"),json!({"session":session,"owner":owner,"args":{"scope":"global","key":"first","remove":false,"value":true}})).unwrap();
        host.queue_service(
            "nativeDocumentOpen",
            json!("queued-open"),
            json!({"session":session,"owner":owner,"generation":1,"args":{"path":unopened}}),
        )
        .unwrap();
        host.queue_service("nativeDocumentShow",json!("queued-show"),json!({"session":session,"owner":owner,"generation":1,"args":{"document":id,"version":1,"selection":null}})).unwrap();
        // Hold the existing worker's completion deterministically: the queue
        // must record both document contexts even before its reply can arrive.
        let (sender, receiver) = mpsc::sync_channel(1);
        app.extension_services.job = Some(Job {
            service: host.service().unwrap().clone(),
            context: None,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
        });
        app.extension_host = Some(host);
        assert!(Context::check_budget(&app).is_err());
        app.poll_extension_services();
        assert_eq!(app.extension_services.contexts.len(), 2);
        assert!(
            app.extension_services
                .contexts
                .values()
                .all(|context| context.snapshot.is_none())
        );
        for doc in &mut app.hidden_documents {
            doc.secondary.clear();
        }
        Context::check_budget(&app).unwrap();
        app.poll_extension_services();
        assert!(
            app.extension_services
                .contexts
                .values()
                .all(|context| context.snapshot.is_none())
        );
        assert!(app.extension_services.job.is_some());
        sender
            .send(Ok(Output::State(json!({"first":true}))))
            .unwrap();
        app.poll_extension_services();
        // Hidden selection changes do not alter mirror generations: receipt
        // context rejection must do the work, rather than a stale wire version.
        for _ in 0..2 {
            let host = app.extension_host.as_ref().unwrap();
            host.service_valid(host.service().unwrap()).unwrap();
            app.poll_extension_services();
            assert!(app.extension_services.job.is_none());
            assert!(app.documents.is_empty());
            assert_eq!(app.hidden_documents.len(), 2);
        }
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert!(app.extension_services.contexts.is_empty());
        assert_eq!(app.hidden_documents[0].id, id);
        assert_eq!(app.hidden_documents[0].revision, revision);
        assert_eq!(app.hidden_documents[0].text.to_string(), text);
        assert!(app.hidden_documents[0].dirty());
        app.hidden_documents[0].undo();
        assert_eq!(app.hidden_documents[0].id, id);
        assert_eq!(app.hidden_documents[0].text.to_string(), "original猫\r\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original猫\r\n");
        assert_eq!(
            std::fs::read_to_string(&unopened).unwrap(),
            "must stay unopened"
        );
    }
    #[test]
    fn context_selection_budget_counts_document_and_inactive_pane_snapshots_before_cloning() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("kept.txt");
        std::fs::write(&path, "original猫\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("dirty ", false);
        app.split_editor(false);
        let current = app.panes[app.active_pane].id;
        let other = app.panes.iter().find(|pane| pane.id != current).unwrap().id;
        app.doc_mut().secondary = vec![crate::document::Selection::caret(0); 8190];
        app.hidden_documents.push(Document::default());
        // Document + active pane each retain 8191; hidden + inactive pane each 1.
        Context::check_budget(&app).unwrap();
        let context = Context::capture(&app);
        let snapshot = context.snapshot.as_ref().unwrap();
        assert_eq!(
            snapshot
                .documents
                .iter()
                .map(|doc| doc.selections.len())
                .sum::<usize>()
                + snapshot
                    .pane_selections
                    .iter()
                    .map(|(_, _, secondary)| secondary.len() + 1)
                    .sum::<usize>(),
            MAX_CONTEXT_SELECTIONS
        );
        assert!(context.same_editor(&app));
        let id = app.doc().id;
        let revision = app.doc().revision;
        let text = app.doc().text.to_string();
        app.doc_mut().activate_view(other);
        app.doc_mut()
            .secondary
            .push(crate::document::Selection::caret(0));
        app.doc_mut().activate_view(current);
        assert!(
            Context::check_budget(&app)
                .unwrap_err()
                .to_string()
                .contains("16,384")
        );
        let invalid = Context::capture(&app);
        assert!(
            invalid.snapshot.is_none(),
            "over-budget capture must allocate no snapshot"
        );
        assert!(
            !invalid.same_editor(&app),
            "invalid sentinels cannot authorize one another"
        );
        assert!(!context.same_editor(&app));
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().text.to_string(), text);
        assert!(app.doc().dirty());
        app.doc_mut().activate_view(other);
        app.doc_mut().secondary.pop();
        app.doc_mut().activate_view(current);
        assert!(context.same_editor(&app));
        app.doc_mut().undo();
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "original猫\r\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original猫\r\n");
    }
    #[test]
    fn service_selection_limit_rejects_without_snapshot_worker_or_native_edits() {
        let root = tempfile::tempdir().unwrap();
        let mut app = queued_app(root.path());
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("dirty猫\r\n", false);
        app.doc_mut().secondary = vec![crate::document::Selection::caret(0); 8192];
        let id = app.doc().id;
        let revision = app.doc().revision;
        assert!(
            Context::check_budget(&app)
                .unwrap_err()
                .to_string()
                .contains("16,384")
        );
        app.poll_extension_services();
        assert!(app.extension_services.contexts.is_empty());
        assert!(app.extension_services.job.is_none());
        assert!(app.hidden_documents.is_empty());
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().text.to_string(), "dirty猫\r\n");
        assert!(app.doc().dirty());
        app.doc_mut().undo();
        assert!(app.doc().is_empty());
        assert_eq!(app.doc().id, id);
    }
    #[test]
    fn model_count_limit_rejects_before_capturing_context_or_starting_file_work() {
        let root = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.documents = (0..129).map(|_| Document::default()).collect();
        assert!(Context::capture(&app).snapshot.is_none());
        let mut host = crate::extensions::Client::start(
            "node",
            &extension,
            root.path(),
            &app.documents,
            0,
            &app.settings,
        )
        .unwrap();
        host.queue_service("nativeDocumentOpen",json!("too-many-models"),json!({"session":host.session,"owner":host.packages[0].id,"generation":1,"args":{"path":"input.txt"}})).unwrap();
        app.extension_host = Some(host);
        app.poll_extension_services();
        assert!(app.extension_services.contexts.is_empty());
        assert!(app.extension_services.job.is_none());
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.documents.len(), 129);
        assert!(app.extension_host.as_ref().unwrap().service().is_none());
    }
    fn queued_app(root: &Path) -> App {
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut app = App::new(root.into(), Profile::Linux);
        let mut host =
            crate::extensions::Client::start("node", &extension, root, &[], 0, &app.settings)
                .unwrap();
        let owner = host.packages[0].id.clone();
        host.queue_service("nativeDocumentOpen",json!("synthetic"),json!({"session":host.session,"owner":owner,"generation":1,"args":{"path":root.join("input.txt")}})).unwrap();
        app.extension_host = Some(host);
        app
    }
    #[test]
    fn stale_worker_reply_never_adds_hidden_models_and_occupied_slot_is_retained() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.txt"), "text").unwrap();
        let mut app = queued_app(root.path());
        assert!(app.poll_extension_services());
        assert!(app.extension_services.job.is_some());
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("newer typing", false);
        let id = app.doc().id;
        app.cancel_extension_services();
        assert!(app.extension_services.job.is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.extension_services.job.is_some() {
            app.poll_extension_services();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.hidden_documents.is_empty());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "newer typing");
    }
    #[cfg(unix)]
    #[test]
    fn native_alias_open_of_deleted_hidden_file_moves_identity_without_reading_disk() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("real");
        std::fs::create_dir(&folder).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&folder, &alias).unwrap();
        let path = folder.join("input.txt");
        std::fs::write(&path, "original").unwrap();
        let mut doc = Document::open_existing(&path).unwrap();
        let id = doc.id;
        doc.insert("dirty ", false);
        std::fs::remove_file(&path).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.hidden_documents.push(doc);
        app.open(&alias.join("input.txt")).unwrap();
        // A second request rejected by the occupied slot must leave the first
        // accepted loader valid rather than canceling both native opens.
        app.open(&alias.join("input.txt")).unwrap();
        assert!(app.documents.is_empty());
        until(&mut app, |app| app.active_document().is_some());
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "dirty original");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "original");
        assert!(!path.exists());
        assert!(app.hidden_documents.is_empty());
    }
}
