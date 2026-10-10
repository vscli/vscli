use super::*;
use crate::session::{Event as SessionEvent, Layout, Pane as SavedPane, SavedFile, View, Worker};
use std::time::{Duration, Instant};

#[derive(PartialEq)]
struct Context {
    epoch: u64,
    workspace: PathBuf,
    documents: Vec<(u64, u64, Option<PathBuf>, Vec<crate::document::Selection>)>,
    panes: Vec<(u64, u64)>,
    active: usize,
    active_pane: usize,
    focus: Focus,
}
pub(super) struct State {
    worker: Option<Worker>,
    ready: bool,
    available: bool,
    automatic: Option<u64>,
    pending: Option<Context>,
    epoch: u64,
    protected: bool,
    explicit_empty: bool,
    captured: Instant,
}
impl Default for State {
    fn default() -> Self {
        Self {
            worker: None,
            ready: false,
            available: false,
            automatic: None,
            pending: None,
            epoch: 0,
            protected: false,
            explicit_empty: false,
            captured: Instant::now(),
        }
    }
}
impl App {
    /// Called after CLI files and recovery have been installed. No filesystem I/O here.
    pub fn configure_session(&mut self, config: Option<&Path>, restore: bool) -> Result<()> {
        if self.session.worker.is_some() {
            anyhow::bail!("Session worker is already configured");
        }
        let Some(config) = config else {
            return Ok(());
        };
        self.sync_pane();
        self.session.worker = Some(Worker::start(
            config.to_owned(),
            self.workspace.root.clone(),
        )?);
        self.session.automatic = restore.then_some(self.session.epoch);
        self.session.protected = restore;
        Ok(())
    }
    pub(super) fn session_interaction(&mut self) {
        self.extension_services.epoch = self.extension_services.epoch.wrapping_add(1);
        self.session.epoch = self.session.epoch.wrapping_add(1);
    }
    fn session_context(&self) -> Context {
        Context {
            epoch: self.session.epoch,
            workspace: self.workspace.root.clone(),
            documents: self
                .documents
                .iter()
                .map(|d| (d.id, d.revision, d.path.clone(), d.selections()))
                .collect(),
            panes: self.panes.iter().map(|p| (p.id, p.document)).collect(),
            active: self.active,
            active_pane: self.active_pane,
            focus: self.focus.clone(),
        }
    }
    pub(super) fn restore_session(&mut self) {
        if !self.documents.is_empty() {
            self.message = "Close editors before restoring the previous clean-file session".into();
            return;
        }
        self.begin_session_restore();
    }
    fn begin_session_restore(&mut self) {
        if self.session.worker.is_none() {
            self.message = "Session storage is disabled or unavailable".into();
            return;
        }
        if !self.session.ready {
            self.message = "Session metadata is still loading; retry shortly".into();
            return;
        }
        if !self.session.available {
            self.message = "No previous clean-file session".into();
            return;
        }
        if self.session.pending.is_some() {
            self.message = "Session restore is still loading; retry shortly".into();
            return;
        }
        if self.documents.len() > 128
            || self
                .documents
                .iter()
                .any(|doc| doc.secondary.len() >= crate::session::MAX_SELECTIONS)
        {
            self.message =
                "Too many existing buffers/selections for guarded session restore".into();
            return;
        }
        let context = self.session_context();
        let skip = self
            .documents
            .iter()
            .filter_map(|doc| doc.path.clone())
            .collect();
        let Some(worker) = &mut self.session.worker else {
            self.message = "Session storage is disabled or unavailable".into();
            return;
        };
        match worker.restore(skip) {
            Ok(()) => {
                self.session.pending = Some(context);
                self.session.protected = true;
                self.message = "Restoring clean-file session…".into();
            }
            Err(error) => self.message = format!("Session restore unavailable: {error:#}"),
        }
    }
    fn capture_session(&self) -> Result<Layout> {
        let mut layout = Layout::default();
        let mut ids = Vec::new();
        for doc in &self.documents {
            if doc.dirty() {
                continue;
            }
            if let Some(path) = &doc.path {
                if layout.files.len() >= crate::session::MAX_DOCUMENTS
                    || path.as_os_str().len() > 4096
                {
                    anyhow::bail!("Session exceeds 32 files or path byte budget");
                }
                layout.files.push(SavedFile {
                    path: path.clone(),
                    view: View::capture(doc, doc.view_state(None))?,
                });
                ids.push(doc.id);
            }
        }
        for (index, pane) in self.panes.iter().enumerate() {
            if let Some(file) = ids.iter().position(|id| *id == pane.document) {
                let doc = self
                    .documents
                    .iter()
                    .find(|d| d.id == pane.document)
                    .unwrap();
                if index == self.active_pane {
                    layout.active_pane = layout.panes.len();
                }
                layout.panes.push(SavedPane {
                    file,
                    view: View::capture(doc, doc.view_state(Some(pane.id)))?,
                });
            }
        }
        if !layout.files.is_empty() && layout.panes.is_empty() {
            layout.panes.push(SavedPane {
                file: 0,
                view: layout.files[0].view.clone(),
            });
        }
        layout.active_file = layout.panes.get(layout.active_pane).map_or(0, |p| p.file);
        layout.horizontal = self.horizontal_split;
        layout.validate()?;
        Ok(layout)
    }
    fn session_snapshot(&self) -> Result<Option<Layout>> {
        if self.session.protected
            || self.session.pending.is_some()
            || self.session.automatic.is_some()
        {
            return Ok(None);
        }
        let layout = self.capture_session()?;
        Ok((!layout.files.is_empty() || self.session.explicit_empty).then_some(layout))
    }
    fn install_session(&mut self, restored: crate::session::Restored) -> Result<()> {
        // Resolve and validate every reference before changing the live document set.
        restored.layout.validate()?;
        let had_existing = !self.documents.is_empty();
        if restored.layout.files.iter().any(|file| {
            !self
                .documents
                .iter()
                .chain(&restored.documents)
                .any(|doc| doc.path.as_ref() == Some(&file.path))
        }) {
            anyhow::bail!("A restored session file is no longer available");
        }
        let crate::session::Restored { layout, documents } = restored;
        for mut doc in documents {
            self.settings.apply(&mut doc);
            self.configure_document_language(&mut doc)?;
            // Existing and recovered documents never receive persisted selections.
            if !self
                .documents
                .iter()
                .any(|existing| existing.path == doc.path)
            {
                let saved = layout
                    .files
                    .iter()
                    .find(|file| Some(&file.path) == doc.path.as_ref())
                    .unwrap();
                saved.view.apply(&mut doc);
                self.documents.push(doc);
            }
        }
        if !had_existing {
            let mut panes = Vec::new();
            for saved in &layout.panes {
                let path = &layout.files[saved.file].path;
                let doc = self
                    .documents
                    .iter_mut()
                    .find(|doc| doc.path.as_ref() == Some(path))
                    .unwrap();
                let id = self.next_pane_id;
                self.next_pane_id += 1;
                doc.activate_view(id);
                saved.view.apply(doc);
                panes.push(Pane {
                    id,
                    document: doc.id,
                });
            }
            self.panes = panes;
            self.active_pane = layout.active_pane;
            self.active = layout.active_file;
            self.horizontal_split = layout.horizontal;
            if !self.panes.is_empty() {
                self.focus_pane(self.active_pane);
            } else {
                self.sync_pane();
            }
        }
        self.session.protected = false;
        self.session.explicit_empty = self.documents.is_empty();
        self.message = format!(
            "Restored clean-file session · {} file(s) · existing/recovered buffers retained",
            layout.files.len()
        );
        Ok(())
    }
    pub(super) fn poll_session(&mut self) -> bool {
        let event = self.session.worker.as_mut().and_then(Worker::poll);
        let changed = event.is_some();
        match event {
            Some(SessionEvent::Ready(available)) => {
                self.session.ready = true;
                self.session.available = available;
                if !available {
                    self.session.protected = false;
                }
                if let Some(epoch) = self.session.automatic.take() {
                    if !available {
                        self.message = "No previous clean-file session".into();
                    } else if epoch == self.session.epoch
                        && self.prompt.is_none()
                        && self.modal.is_none()
                    {
                        self.begin_session_restore();
                    } else {
                        self.message = "Session restore cancelled by newer interaction; saved session retained".into();
                    }
                }
            }
            Some(SessionEvent::Restored(restored)) => {
                if let Some(context) = self.session.pending.take() {
                    if context == self.session_context()
                        && self.prompt.is_none()
                        && self.modal.is_none()
                    {
                        if let Err(error) = self.install_session(restored) {
                            self.message = format!(
                                "Session restore failed; previous session retained: {error:#}"
                            );
                        }
                    } else {
                        self.message = "Session restore cancelled by newer interaction; saved session retained".into();
                    }
                }
            }
            Some(SessionEvent::Failed(error)) => {
                if !self.session.ready {
                    self.session.worker = None;
                    self.session.automatic = None;
                    self.session.protected = true;
                }
                self.session.pending = None;
                self.message =
                    format!("Session state unavailable; previous metadata retained: {error}");
            }
            _ => {}
        }
        if self.session.ready
            && self.session.pending.is_none()
            && self.session.captured.elapsed() >= Duration::from_secs(2)
        {
            self.session.captured = Instant::now();
            let snapshot = self.session_snapshot();
            if let Some(worker) = &mut self.session.worker {
                let result = match snapshot {
                    Ok(Some(layout)) => worker.save(layout),
                    Ok(None) => worker.flush(),
                    Err(e) => Err(e),
                };
                if let Err(error) = result {
                    self.message = format!("Session state not saved: {error:#}");
                    return true;
                }
            }
        }
        changed
    }
    pub fn finish_session(&mut self) -> Result<()> {
        let Some(worker) = self.session.worker.take() else {
            return Ok(());
        };
        worker.finish(self.session_snapshot()?)
    }
    pub(super) fn session_closed_all(&mut self) {
        self.session.explicit_empty = true;
        self.session.protected = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Selection;
    use std::fs;
    fn settle(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            app.poll_session();
            if app.session.ready && app.session.automatic.is_none() && app.session.pending.is_none()
            {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn configured(root: &Path, config: &Path, restore: bool) -> App {
        let mut app = App::new(root.into(), Profile::Linux);
        app.configure_session(Some(config), restore).unwrap();
        settle(&mut app);
        app
    }
    fn seed(root: &Path, config: &Path, path: &Path) {
        let mut app = configured(root, config, false);
        app.open(path).unwrap();
        app.doc_mut().move_to(2, false);
        app.finish_session().unwrap();
    }
    #[test]
    fn restart_restores_global_tabs_shared_panes_and_independent_unicode_crlf_selections() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let first = root.path().join("first.cpp");
        let second = root.path().join("second.cpp");
        let content = "猫 🦀 abc\r\nsecond line\r\n";
        fs::write(&first, content).unwrap();
        fs::write(&second, "other\r\n").unwrap();
        let expected;
        {
            let mut app = configured(root.path(), &config, false);
            app.open(&first).unwrap();
            app.open(&second).unwrap();
            app.open(&first).unwrap();
            app.doc_mut().set_selections(vec![
                Selection {
                    cursor: 6,
                    anchor: Some(3),
                    desired_column: None,
                },
                Selection::caret(12),
            ]);
            app.split_editor(true);
            app.doc_mut().set_selections(vec![Selection {
                cursor: 15,
                anchor: Some(10),
                desired_column: None,
            }]);
            app.doc_mut().top = 1;
            app.focus_pane(0);
            expected = app.capture_session().unwrap();
            app.finish_session().unwrap();
        }
        let mut app = configured(root.path(), &config, true);
        assert_eq!(app.capture_session().unwrap(), expected);
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.panes[0].document, app.panes[1].document);
        assert!(app.documents.iter().all(|d| !d.dirty()));
        let id = app.doc().id;
        app.doc_mut().insert("!", false);
        app.focus_pane(1);
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        app.doc_mut().undo();
        assert_eq!(app.doc().text, content);
        assert!(!app.doc().dirty());
        app.doc_mut().save().unwrap();
        assert_eq!(fs::read(&first).unwrap(), content.as_bytes());
        app.finish_session().unwrap();
    }
    #[test]
    fn recovery_duplicates_are_authoritative_and_do_not_receive_saved_selections() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        let other = root.path().join("other.txt");
        fs::write(&path, "saved disk").unwrap();
        fs::write(&other, "other disk").unwrap();
        let mut original = configured(root.path(), &config, false);
        original.open(&path).unwrap();
        original.open(&other).unwrap();
        original.finish_session().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        let mut a = Document::open(&path).unwrap();
        a.insert("recovered A", false);
        let mut b = Document::open(&path).unwrap();
        b.insert("recovered B", false);
        let ids = [a.id, b.id];
        let selections = [a.selections(), b.selections()];
        app.documents = vec![a, b];
        app.active = 1;
        app.sync_pane();
        let pane = app.panes[0].id;
        app.configure_session(Some(&config), true).unwrap();
        settle(&mut app);
        assert_eq!(app.documents.len(), 3);
        assert_eq!(app.active, 1);
        assert_eq!(app.panes[0].id, pane);
        for index in 0..2 {
            assert_eq!(app.documents[index].id, ids[index]);
            assert_eq!(app.documents[index].selections(), selections[index]);
            assert!(app.documents[index].dirty());
        }
        assert_eq!(fs::read_to_string(path).unwrap(), "saved disk");
        app.finish_session().unwrap();
    }
    #[test]
    fn missing_files_are_retryable_and_failed_or_stale_restore_never_publishes_empty_state() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "abcd").unwrap();
        let path = fs::canonicalize(path).unwrap();
        seed(root.path(), &config, &path);
        fs::remove_file(&path).unwrap();
        let mut failed = configured(root.path(), &config, true);
        assert!(failed.documents.is_empty());
        assert!(failed.session.protected);
        assert!(!path.exists());
        failed.finish_session().unwrap();
        fs::write(&path, "abcd").unwrap();
        let mut stale = configured(root.path(), &config, false);
        stale.restore_session();
        assert!(stale.session.pending.is_some());
        stale.execute("workbench.action.files.newUntitledFile", Value::Null);
        stale.doc_mut().insert("newer unsaved work", false);
        settle(&mut stale);
        assert_eq!(stale.documents.len(), 1);
        assert_eq!(stale.doc().text, "newer unsaved work");
        assert!(stale.session.protected);
        stale.finish_session().unwrap();
        let mut retry = configured(root.path(), &config, true);
        assert_eq!(retry.documents.len(), 1);
        assert_eq!(retry.doc().path.as_ref(), Some(&path));
        assert_eq!(retry.doc().cursor, 2);
        retry.finish_session().unwrap();
    }
    #[test]
    fn failed_restore_can_retry_in_place_without_another_worker() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "abcd").unwrap();
        seed(root.path(), &config, &path);
        fs::remove_file(&path).unwrap();
        let mut app = configured(root.path(), &config, true);
        fs::write(&path, "abcd").unwrap();
        app.restore_session();
        app.restore_session();
        settle(&mut app);
        assert_eq!(app.documents.len(), 1);
        assert!(!app.session.protected);
        app.finish_session().unwrap();
    }
    #[test]
    fn discard_cancel_save_as_and_explicit_close_all_do_not_resurrect_text() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        let moved = fs::canonicalize(root.path()).unwrap().join("moved.txt");
        fs::write(&path, "disk").unwrap();
        let path = fs::canonicalize(path).unwrap();
        seed(root.path(), &config, &path);
        let mut app = configured(root.path(), &config, true);
        app.doc_mut().insert("DISCARDED SECRET", false);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        )));
        assert!(app.doc().dirty());
        assert!(
            !app.capture_session()
                .unwrap()
                .files
                .iter()
                .any(|f| f.path == path)
        );
        app.execute("workbench.action.files.saveAs", Value::Null);
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(app.doc().path.as_ref(), Some(&path));
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert!(app.documents.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "disk");
        app.open(&path).unwrap();
        app.execute("workbench.action.files.saveAs", Value::Null);
        app.prompt.as_mut().unwrap().text = moved.to_string_lossy().into_owned();
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.doc().path.as_ref(), Some(&moved));
        assert_eq!(app.capture_session().unwrap().files[0].path, moved);
        app.execute("workbench.action.closeAllEditors", Value::Null);
        assert!(app.session.explicit_empty);
        app.finish_session().unwrap();
        let mut restored = configured(root.path(), &config, true);
        assert!(restored.documents.is_empty());
        restored.finish_session().unwrap();
        for dir in fs::read_dir(config.join("state/sessions")).unwrap() {
            for file in fs::read_dir(dir.unwrap().path()).unwrap() {
                let path = file.unwrap().path();
                if path.extension().is_some_and(|e| e == "json") {
                    assert!(
                        !fs::read_to_string(path)
                            .unwrap()
                            .contains("DISCARDED SECRET")
                    );
                }
            }
        }
    }
    #[test]
    fn disabled_and_failed_initial_storage_leave_empty_editor_usable() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("vscli.session.restore", Value::Null);
        assert!(app.message.contains("disabled or unavailable"));
        assert!(app.documents.is_empty());
        let config = root.path().join("not-a-directory");
        fs::write(&config, "retain").unwrap();
        app.configure_session(Some(&config), true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while app.session.worker.is_some() {
            app.poll_session();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        app.execute("vscli.session.restore", Value::Null);
        assert!(app.message.contains("disabled or unavailable"));
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("native editing survives", false);
        assert_eq!(app.doc().text, "native editing survives");
        assert_eq!(fs::read_to_string(config).unwrap(), "retain");
        app.finish_session().unwrap();
    }
    #[test]
    fn resize_and_key_release_do_not_cancel_startup_restore() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "abcd").unwrap();
        seed(root.path(), &config, &path);
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_session(Some(&config), true).unwrap();
        app.event(Event::Resize(80, 25));
        app.event(Event::FocusGained);
        app.event(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )));
        settle(&mut app);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.doc().cursor, 2);
        app.finish_session().unwrap();
    }
    #[test]
    fn changed_shorter_unicode_crlf_lines_clamp_selection_without_splitting_eol() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let path = root.path().join("file.txt");
        fs::write(&path, "long first line\r\nsecond\r\n").unwrap();
        let mut app = configured(root.path(), &config, false);
        app.open(&path).unwrap();
        app.doc_mut().set_selections(vec![Selection {
            cursor: 12,
            anchor: Some(7),
            desired_column: None,
        }]);
        app.finish_session().unwrap();
        fs::write(&path, "e\u{301}猫\r\nX\r\n").unwrap();
        let mut app = configured(root.path(), &config, true);
        assert_eq!(app.doc().row(), 0);
        assert_eq!(app.doc().cursor, 3);
        app.doc_mut().insert("!", false);
        app.doc_mut().save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "e\u{301}猫!\r\nX\r\n");
        app.doc_mut().undo();
        assert_eq!(app.doc().text, "e\u{301}猫\r\nX\r\n");
        app.finish_session().unwrap();
    }
}

#[cfg(test)]
mod combined_tests {
    use super::*;
    use std::fs;
    fn until(app: &mut App, condition: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            if condition(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn queued_native_extension_prompt_cancels_restore_and_retains_metadata_for_retry() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let file = root.path().join("saved.txt");
        fs::write(&file, "saved disk\r\n").unwrap();
        let mut seed = App::new(root.path().into(), Profile::Linux);
        seed.configure_session(Some(&config), false).unwrap();
        seed.open(&file).unwrap();
        seed.finish_session().unwrap();
        let extension = root.path().join("extension");
        fs::create_dir(&extension).unwrap();
        fs::write(extension.join("package.json"), r#"{"publisher":"fixture","name":"session-prompt","version":"1.0.0","main":"extension.cjs"}"#).unwrap();
        fs::write(extension.join("extension.cjs"), r#"
const vscode = require('vscode');
exports.activate = context => context.subscriptions.push(vscode.commands.registerCommand('session.prompt', async () => {
 const answer = await vscode.window.showInputBox({title:'Session interruption'});
 await vscode.window.showInformationMessage('session answer='+answer);
}));
"#).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_session(Some(&config), false).unwrap();
        app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
            .unwrap();
        until(&mut app, |a| {
            a.session.ready && a.extension_host.as_ref().is_some_and(|h| h.ready)
        });
        app.start_prompt(PromptKind::Palette, "restore".into());
        app.execute("session.prompt", Value::Null);
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .is_some_and(|h| h.prompt().is_some())
        });
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Palette
        ));
        app.prompt = None; // Native palette acceptance removes itself before dispatch.
        app.execute("vscli.session.restore", Value::Null);
        assert!(app.session.pending.is_some());
        until(&mut app, |a| a.session.pending.is_none());
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Extension(_)
        ));
        assert!(app.documents.is_empty());
        assert!(app.session.protected);
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        until(&mut app, |a| a.message == "session answer=undefined");
        // A popup can be retired by its owner before a loader reply is polled.
        // Its appearance must still invalidate the captured restore generation.
        app.start_prompt(PromptKind::Palette, String::new());
        app.execute("session.prompt", Value::Null);
        until(&mut app, |a| {
            a.extension_host
                .as_ref()
                .is_some_and(|h| h.prompt().is_some())
        });
        app.prompt = None;
        app.execute("vscli.session.restore", Value::Null);
        app.poll_extensions();
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Extension(_)
        ));
        app.cancel_extension_prompt();
        assert!(app.prompt.is_none());
        until(&mut app, |a| {
            a.session.pending.is_none() && a.extension_host.as_ref().is_some_and(|h| !h.busy())
        });
        assert!(app.documents.is_empty());
        app.execute("vscli.session.restore", Value::Null);
        until(&mut app, |a| !a.documents.is_empty());
        assert_eq!(app.doc().text, "saved disk\r\n");
        assert!(app.prompt.is_none());
        let id = app.doc().id;
        app.execute("session.prompt", Value::Null);
        until(&mut app, |a| {
            matches!(
                a.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Extension(_))
            )
        });
        app.event(Event::Paste("answer".into()));
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        until(&mut app, |a| a.message == "session answer=answer");
        assert_eq!(app.doc().id, id);
        assert_eq!(fs::read_to_string(&file).unwrap(), "saved disk\r\n");
        app.finish_session().unwrap();
    }
    #[test]
    fn workspace_symbol_picker_wins_over_pending_restore_and_late_replies_cannot_revive_either_ui()
    {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let file = root.path().join("saved.txt");
        fs::write(&file, "preserved\r\n").unwrap();
        let file = fs::canonicalize(file).unwrap();
        fs::write(root.path().join("other.cpp"), "x 😀foo\r\n").unwrap();
        let mut seed = App::new(root.path().into(), Profile::Linux);
        seed.configure_session(Some(&config), false).unwrap();
        seed.open(&file).unwrap();
        seed.finish_session().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_session(Some(&config), false).unwrap();
        app.lsp = Some(
            crate::lsp::Client::start(
                "python3",
                &[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/symbol_server.py")
                    .to_string_lossy()
                    .into_owned()],
                root.path(),
                "cpp".into(),
            )
            .unwrap(),
        );
        until(&mut app, |a| {
            a.session.ready && a.lsp.as_ref().is_some_and(|c| c.ready)
        });
        app.execute("vscli.session.restore", Value::Null);
        app.execute("workbench.action.showAllSymbols", Value::Null);
        assert!(matches!(
            app.prompt.as_ref().unwrap().kind,
            PromptKind::Symbols
        ));
        until(&mut app, |a| {
            a.session.pending.is_none() && !a.symbol_items("").is_empty()
        });
        assert!(app.documents.is_empty());
        assert!(app.session.protected);
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        app.execute("vscli.session.restore", Value::Null);
        until(&mut app, |a| !a.documents.is_empty());
        assert!(app.prompt.is_none());
        assert_eq!(app.doc().path.as_ref(), Some(&file));
        assert_eq!(app.doc().text, "preserved\r\n");
        assert!(!app.doc().dirty());
        app.finish_session().unwrap();
    }
}
