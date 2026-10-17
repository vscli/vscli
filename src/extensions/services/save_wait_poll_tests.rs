//! Actual two-owner selected host + App poll. Only clocks are aged. The target
//! deliberately holds command completion AFTER its real save result, removing
//! terminal-reply races from the exact clock assertions without fake receipts.
use super::*;
use crate::app::{App, PromptKind};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::{fs, thread};
const WAIT: Duration = Duration::from_secs(10);
const TEXT: &str = "猫🙂 wait\r\nlast\r\n";
const ORIGIN: &str = "wait.a";
const CALLER: &str = "wait.b";

struct Fixture {
    app: App,
    _dir: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
        app.settings = Settings::from_values(
            json!({"vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,
                "files.autoSave":"off","editor.formatOnSave":false,
                "editor.codeActionsOnSave":{}})
            .as_object()
            .unwrap()
            .clone(),
            "real save human-wait fixture",
        )
        .unwrap();
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().eol = "\r\n".into();
        app.doc_mut().insert(TEXT, false);
        let a = package(
            &root,
            "a",
            &["wait.bridge"],
            r#"
const vscode = require('vscode');
exports.activate = context => {
  context.subscriptions.push(vscode.commands.registerCommand('wait.bridge', () =>
    vscode.commands.executeCommand('wait.target')));
};"#,
        );
        let b = package(
            &root,
            "b",
            &["wait.target", "wait.release", "wait.stall"],
            r#"
const vscode = require('vscode'); const assert = require('node:assert/strict');
exports.activate = context => {
  let release;
  context.subscriptions.push(vscode.commands.registerCommand('wait.target', async () => {
    assert.equal(release, undefined, 'one target invocation');
    const document = vscode.workspace.textDocuments.find(d => d.isUntitled);
    assert(document, 'original untitled model');
    const saved = await document.save();
    assert.equal(typeof saved, 'boolean');
    const completion = new Promise(resolve => { release = () => resolve(saved); });
    void vscode.window.showInformationMessage('WAIT-SAVE-' + saved);
    return completion;
  }));
  context.subscriptions.push(vscode.commands.registerCommand('wait.release', () => {
    assert(release, 'actual save result precedes release'); release(); release = undefined;
  }));
  context.subscriptions.push(vscode.commands.registerCommand('wait.stall', () => {
    void vscode.window.showInformationMessage('WAIT-STALL-STARTED');
    return new Promise(() => {});
  }));
};"#,
        );
        app.configure_extension_storage(Some(&root.join("storage")));
        app.start_extension_packages(vec![a, b]).unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| {
                host.ready
                    && host.owner_active(ORIGIN)
                    && host.owner_active(CALLER)
                    && ["wait.bridge", "wait.target", "wait.release", "wait.stall"]
                        .iter()
                        .all(|command| host.commands.iter().any(|(_, id)| id.as_str() == *command))
            })
        });
        Self {
            app,
            _dir: dir,
            root,
        }
    }
    fn begin(&mut self) -> NativeService {
        self.app.execute("wait.bridge", Value::Null);
        until(&mut self.app, |app| {
            app.prompt
                .as_ref()
                .is_some_and(|p| matches!(p.kind, PromptKind::SaveAs))
                && app.extension_host.as_ref().is_some_and(|host| {
                    host.service().is_some_and(|s| {
                        matches!(s.operation, Operation::Save(_)) && s.save_path_wait.is_some()
                    })
                })
        });
        let host = self.host();
        let service = host.service().unwrap().clone();
        assert_eq!(service.request.owner, CALLER);
        assert_eq!(service.request.command_owner.as_deref(), Some(ORIGIN));
        let command = service.request.command.unwrap();
        assert_eq!(host.pending[&command].method, "execute");
        assert_eq!(host.pending[&command].owner.as_deref(), Some(ORIGIN));
        match &service.operation {
            Operation::Save(save) => assert_eq!(save.document, self.app.doc().id),
            _ => panic!("Actual native save service required"),
        }
        service
    }
    fn host(&self) -> &Client {
        self.app.extension_host.as_ref().unwrap()
    }
    fn host_mut(&mut self) -> &mut Client {
        self.app.extension_host.as_mut().unwrap()
    }
    fn age_command(&mut self, command: u64) -> Instant {
        let started = Instant::now().checked_sub(Duration::from_secs(45)).unwrap();
        self.host_mut().pending.get_mut(&command).unwrap().started = started;
        started
    }
    fn poll_host(&mut self) -> Result<Vec<String>> {
        let app = &mut self.app;
        app.extension_host.as_mut().unwrap().poll_with_hidden(
            &mut app.documents,
            &mut app.hidden_documents,
            app.active,
            &app.settings,
        )
    }
    fn submit(&mut self, command: &str) -> u64 {
        let app = &mut self.app;
        let host = app.extension_host.as_mut().unwrap();
        let before: HashSet<_> = host.pending.keys().copied().collect();
        host.execute_with_hidden(
            command,
            None,
            &app.documents,
            &app.hidden_documents,
            app.active,
            &app.settings,
        )
        .unwrap();
        let added: Vec<_> = host
            .pending
            .keys()
            .filter(|id| !before.contains(*id))
            .copied()
            .collect();
        assert_eq!(added.len(), 1, "One actual execute request");
        added[0]
    }
    fn await_result(&mut self, saved: bool) {
        let marker = format!("WAIT-SAVE-{saved}");
        until(&mut self.app, |app| {
            app.message == marker
                && !app.saves_pending()
                && app
                    .extension_host
                    .as_ref()
                    .is_some_and(|host| host.service().is_none())
        });
    }
    fn assert_resets_once(&mut self, command: u64, old: Instant) {
        let resumed = self.host().pending[&command].started;
        assert!(
            resumed > old,
            "Leaving human wait resets exact origin deadline"
        );
        for _ in 0..4 {
            self.app.poll();
            assert!(self.host().service().is_none());
            assert_eq!(
                self.host().pending[&command].started,
                resumed,
                "Actual polls cannot renew a completed/canceled wait"
            );
        }
        self.submit("wait.release");
        until(&mut self.app, |app| {
            app.extension_host
                .as_ref()
                .is_some_and(|host| !host.pending.contains_key(&command))
        });
        assert!(self.host().ready);
    }
    fn escape(&mut self) {
        self.app
            .event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    }
    fn accept(&mut self, path: &Path) {
        assert!(matches!(
            self.app.prompt.as_ref().unwrap().kind,
            PromptKind::SaveAs
        ));
        self.app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        )));
        self.app.event(Event::Paste(path.to_str().unwrap().into()));
        assert_eq!(
            self.app.prompt.as_ref().unwrap().text,
            path.to_str().unwrap()
        );
        self.app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
    }
}
fn package(root: &Path, name: &str, commands: &[&str], source: &str) -> Package {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    let commands: Vec<_> = commands
        .iter()
        .map(|id| json!({"command":id,"title":id}))
        .collect();
    fs::write(
        path.join("package.json"),
        serde_json::to_vec(&json!({
            "publisher":"wait","name":name,"version":"1.0.0","main":"index.cjs",
            "contributes":{"commands":commands}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(path.join("index.cjs"), source).unwrap();
    Package::read(&path).unwrap()
}
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Actual host/App boundary: {}",
            app.message
        );
        thread::sleep(Duration::from_millis(2));
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Model {
    id: u64,
    path: Option<PathBuf>,
    text: String,
    selections: Vec<crate::document::Selection>,
    epoch: u64,
    revision: u64,
    saved_revision: u64,
    save_generation: u64,
    dirty: bool,
}
fn model(app: &App) -> Model {
    let doc = app.doc();
    Model {
        id: doc.id,
        path: doc.path.clone(),
        text: doc.text.to_string(),
        selections: doc.selections(),
        epoch: doc.text_epoch(),
        revision: doc.revision,
        saved_revision: doc.saved_revision,
        save_generation: doc.save_generation(),
        dirty: doc.dirty(),
    }
}
fn assert_unsaved_history(f: &mut Fixture, original: &Model) {
    assert_eq!(&model(&f.app), original);
    assert!(!f.root.join("untitled.txt").exists());
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), "");
    assert_eq!(f.app.doc().id, original.id);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), original.text);
    assert_eq!(f.app.doc().revision, original.revision);
    assert_eq!(f.app.doc().save_generation(), original.save_generation);
}

#[test]
fn actual_aged_origin_survives_app_poll_without_renewing_human_wait() {
    let mut f = Fixture::new();
    let original = model(&f.app);
    let service = f.begin();
    let command = service.request.command.unwrap();
    let old = f.age_command(command);
    for _ in 0..4 {
        f.app.poll();
        assert!(
            f.app.extension_host.is_some(),
            "Actual human wait lost its host: {}",
            f.app.message
        );
        assert_eq!(f.host().pending[&command].started, old);
        assert_eq!(
            f.host().service().unwrap().save_path_wait,
            service.save_path_wait
        );
        assert!(f.host().document_save_command_waiting(command));
        assert_eq!(model(&f.app), original);
        assert!(
            !f.app.saves_pending(),
            "Path wait has no persistence operation yet"
        );
    }
    f.escape();
    f.await_result(false);
    f.assert_resets_once(command, old);
    assert_unsaved_history(&mut f, &original);
}

#[test]
fn actual_unrelated_aged_command_still_times_out_during_origin_human_wait() {
    let mut f = Fixture::new();
    let original = model(&f.app);
    let service = f.begin();
    let command = service.request.command.unwrap();
    let old = f.age_command(command);
    let unrelated = f.submit("wait.stall");
    assert_ne!(unrelated, command);
    until(&mut f.app, |app| app.message == "WAIT-STALL-STARTED");
    assert_eq!(f.host().pending[&unrelated].owner.as_deref(), Some(CALLER));
    let unrelated_old = f.age_command(unrelated);
    let error = f.poll_host().unwrap_err();
    assert!(
        error.to_string().contains("Extension host timed out"),
        "{error:#}"
    );
    assert!(f.host().document_save_command_waiting(command));
    assert!(!f.host().document_save_command_waiting(unrelated));
    assert_eq!(f.host().pending[&command].started, old);
    assert_eq!(f.host().pending[&unrelated].started, unrelated_old);
    assert_eq!(
        f.host().service().unwrap().save_path_wait,
        service.save_path_wait
    );
    assert_unsaved_history(&mut f, &original);
}

#[test]
fn actual_escape_cancellation_resets_only_original_command_once() {
    let mut f = Fixture::new();
    let original = model(&f.app);
    let service = f.begin();
    let command = service.request.command.unwrap();
    let old = f.age_command(command);
    let unrelated = f.submit("wait.stall");
    until(&mut f.app, |app| app.message == "WAIT-STALL-STARTED");
    let unrelated_started = f.host().pending[&unrelated].started;
    f.escape();
    assert!(f.app.prompt.is_none());
    f.await_result(false);
    assert_eq!(f.host().pending[&unrelated].started, unrelated_started);
    f.assert_resets_once(command, old);
    assert_eq!(
        f.host().pending[&unrelated].started,
        unrelated_started,
        "Cancellation must not reset another real pending command"
    );
    assert_unsaved_history(&mut f, &original);
}

#[test]
fn actual_path_acceptance_commits_then_resets_original_command_once() {
    let mut f = Fixture::new();
    let original = model(&f.app);
    let service = f.begin();
    let command = service.request.command.unwrap();
    let old = f.age_command(command);
    let target = f.root.join("accepted.txt");
    f.accept(&target);
    f.await_result(true);
    f.assert_resets_once(command, old);
    let saved = model(&f.app);
    assert_eq!(saved.id, original.id);
    assert_eq!(saved.text, original.text);
    assert_eq!(saved.selections, original.selections);
    assert_eq!(saved.epoch, original.epoch);
    assert_eq!(saved.revision, original.revision);
    assert_eq!(saved.saved_revision, original.revision);
    assert_eq!(saved.save_generation, original.save_generation + 1);
    assert_eq!(saved.path, Some(target.clone()));
    assert!(!saved.dirty);
    assert_eq!(fs::read(&target).unwrap(), TEXT.as_bytes());
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), "");
    assert!(f.app.doc().dirty());
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), TEXT);
    assert!(!f.app.doc().dirty());
    assert_eq!(fs::read(target).unwrap(), TEXT.as_bytes());
}

#[test]
fn actual_expired_path_wait_cannot_be_renewed_by_app_poll_or_write_later() {
    let mut f = Fixture::new();
    let original = model(&f.app);
    let service = f.begin();
    let command = service.request.command.unwrap();
    let old = f.age_command(command);
    let expired = Duration::from_secs(301);
    f.app
        .fixture_age_document_save_path(service.request.session, &service.id, expired)
        .unwrap();
    f.host_mut().services.front_mut().unwrap().save_path_wait =
        Some(Instant::now().checked_sub(expired).unwrap());
    assert!(!f.host().document_save_command_waiting(command));
    f.poll_host().unwrap();
    let resumed = f.host().pending[&command].started;
    assert!(
        resumed > old,
        "Actual poll expires wait before generic command deadline"
    );
    assert!(f.host().service().unwrap().save_path_wait.is_none());
    f.await_result(false);
    assert!(f.app.prompt.is_none());
    assert_eq!(f.host().pending[&command].started, resumed);
    f.assert_resets_once(command, old);
    for _ in 0..4 {
        f.app.poll();
        assert!(!f.app.saves_pending());
        assert!(f.host().service().is_none());
        assert!(!f.root.join("untitled.txt").exists());
    }
    assert!(
        f.app
            .fixture_age_document_save_path(service.request.session, &service.id, expired)
            .is_err(),
        "A terminated exact Path interest cannot be renewed"
    );
    assert_unsaved_history(&mut f, &original);
}
