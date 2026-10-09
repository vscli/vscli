use super::*;
use crate::extension_store::{Installed, Store};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, SyncSender, sync_channel},
};
pub(super) enum Action {
    List,
    Search(String),
    RegistryInstall(crate::extension_registry::Entry),
    CheckUpdates,
    Install(PathBuf),
    Uninstall(String),
    Rollback(String),
}
enum Output {
    Listed(Vec<Installed>, String),
    RegistryItems(Vec<crate::extension_registry::Entry>, String),
    RefreshFailed(String),
    Started(Handoff<Box<crate::extensions::Client>>),
    Stopped,
}
// The worker owns the last reference until the UI accepts or discards its ticket.
// Dropping a ticket only releases a channel; process teardown stays in that worker.
struct Handoff<T> {
    value: Arc<Mutex<Option<T>>>,
    _release: SyncSender<()>,
}
impl<T> Handoff<T> {
    fn take(&self) -> Result<T> {
        self.value
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Extension handoff unavailable"))?
            .take()
            .ok_or_else(|| anyhow::anyhow!("Extension handoff already consumed"))
    }
}
fn handoff<T>(value: T) -> (Handoff<T>, impl FnOnce()) {
    let value = Arc::new(Mutex::new(Some(value)));
    let (release, released) = sync_channel(1);
    let worker_value = value.clone();
    (
        Handoff {
            value,
            _release: release,
        },
        move || {
            let _ = released.recv();
            let value = worker_value
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            drop(value);
        },
    )
}
fn retire<T: Send + 'static>(value: T) -> Receiver<()> {
    let (sender, receiver) = sync_channel(1);
    std::thread::spawn(move || {
        drop(value);
        let _ = sender.send(());
    });
    receiver
}
pub(super) struct Job {
    view: Option<u64>,
    epoch: Option<u64>,
    discard_host: bool,
    receiver: Receiver<std::result::Result<Output, String>>,
}
impl Job {
    pub(super) fn startup(&self) -> bool {
        self.epoch.is_some()
    }
}
impl App {
    pub(super) fn manage_extension(&mut self, action: Action) {
        if self.extension_job.is_some() {
            self.message = "An extension operation is already running".into();
            return;
        }
        let Some(directory) = self.extensions_directory.clone() else {
            self.message = "No extension storage directory available".into();
            return;
        };
        static NEXT_VIEW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let view = NEXT_VIEW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.focus = Focus::Editor;
        self.prompt = None;
        self.modal = Some(Modal::ExtensionsLoading(view));
        let (sender, receiver) = sync_channel(1);
        let registry = self.extension_registry.clone();
        std::thread::spawn(move || {
            let store = Store::new(directory);
            let result = (|| -> Result<Output> {
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let message = match action {
                    Action::Search(query) => return Ok(Output::RegistryItems(
                        registry.search(&query, &cancel)?,
                        "Open VSX results · Enter installs the displayed version; code is not activated".into(),
                    )),
                    Action::CheckUpdates => {
                        let report = registry.updates(&store.list()?, &cancel)?;
                        let message = if let Some(first) = report.notices.first() {
                            format!("{} update check notice(s): {first}", report.notices.len())
                        } else { "Available stable updates · Enter installs · running snapshots stay unchanged".into() };
                        return Ok(Output::RegistryItems(report.items, message));
                    }
                    Action::RegistryInstall(entry) => {
                        let item = registry.install(&entry, &store, &cancel)?;
                        format!("Installed {}@{} from Open VSX; code activation remains explicit", item.id, item.version)
                    }
                    Action::List => {
                        "Installed packages · installation does not imply compatibility".into()
                    }
                    Action::Install(path) => {
                        let item = store.install(&path)?;
                        format!(
                            "Installed {}@{} · {}",
                            item.id, item.version, item.compatibility
                        )
                    }
                    Action::Uninstall(id) => {
                        store.uninstall(&id)?;
                        format!("Uninstalled {id}; running hosts retain their files")
                    }
                    Action::Rollback(id) => {
                        let item = store.rollback(&id)?;
                        format!(
                            "Restored {}@{}; restart its host to use this version",
                            item.id, item.version
                        )
                    }
                };
                match store.list() {
                    Ok(items) => Ok(Output::Listed(items, message)),
                    Err(error) => Ok(Output::RefreshFailed(format!(
                        "{message}; installed-list refresh failed: {error:#}"
                    ))),
                }
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        self.extension_job = Some(Job {
            receiver,
            discard_host: false,
            view: Some(view),
            epoch: None,
        });
        self.message = "Extension operation running…".into();
    }
    pub(super) fn run_installed_extension(&mut self, item: Installed) {
        let mut packages = self.extension_packages.clone();
        let package = crate::extensions::Package::installed(&item);
        packages.retain(|p| p.id != package.id);
        packages.push(package);
        if let Err(error) = self.start_extension_packages(packages) {
            self.message = format!("Cannot start extension session: {error:#}");
        }
    }
    pub fn start_extension_packages(
        &mut self,
        packages: Vec<crate::extensions::Package>,
    ) -> Result<()> {
        self.start_extension_packages_lazy(packages, None)?;
        self.resume_automatic_extensions();
        Ok(())
    }
    pub(super) fn start_extension_packages_lazy(
        &mut self,
        mut packages: Vec<crate::extensions::Package>,
        activation: Option<Vec<String>>,
    ) -> Result<()> {
        if self.extension_job.is_some() || self.extension_retirement.is_some() {
            anyhow::bail!("An extension operation or retirement is already running");
        }
        if packages.is_empty() || packages.len() > 8 {
            anyhow::bail!("Select between one and eight code extensions");
        }
        packages.sort_by(|a, b| a.id.cmp(&b.id));
        if packages.windows(2).any(|pair| pair[0].id == pair[1].id) {
            anyhow::bail!("Duplicate selected extension");
        }
        let mut prepared = crate::extensions::Client::prepare_with_hidden(
            &self.documents,
            &self.hidden_documents,
            self.active,
            &self.settings,
        )?;
        if let Some(activation) = activation {
            prepared = prepared.with_activation(activation)?;
        }

        self.extension_epoch += 1;
        let epoch = self.extension_epoch;
        self.extension_packages = packages.clone();
        let (sender, receiver) = sync_channel(1);
        let node = self.extension_node.clone();
        let root = self.workspace.root.clone();
        self.cancel_extension_prompt();
        let previous = self.extension_host.take().map(|mut host| {
            host.cancel_prompts();
            host
        });
        self.keymap.clear_extension_bindings();
        self.clear_active_extension_bindings();
        std::thread::spawn(move || {
            // Retire the prior cohort before spawning another; never run two Node sessions.
            drop(previous);
            let result =
                crate::extensions::Client::start_many_prepared(&node, &packages, &root, prepared);
            match result {
                Ok(host) => {
                    let (ticket, retire) = handoff(Box::new(host));
                    let _ = sender.send(Ok(Output::Started(ticket)));
                    retire();
                    let _ = sender.send(Ok(Output::Stopped));
                }
                Err(error) => {
                    let _ = sender.send(Err(format!("{error:#}")));
                }
            }
        });
        self.extension_job = Some(Job {
            receiver,
            discard_host: false,
            view: None,
            epoch: Some(epoch),
        });
        self.message = format!(
            "Starting extension session ({} packages)…",
            self.extension_packages.len()
        );
        Ok(())
    }
    pub(super) fn restart_extensions(&mut self) {
        if let Err(error) = self.start_extension_packages(self.extension_packages.clone()) {
            self.message = format!("Cannot restart extension session: {error:#}");
        }
    }
    pub(super) fn stop_selected_extension(&mut self, id: &str) {
        if self.extension_job.is_some() || self.extension_retirement.is_some() {
            self.message = "An extension operation or retirement is already running".into();
            return;
        }
        let mut packages = self.extension_packages.clone();
        packages.retain(|p| p.id != id);
        if packages.len() == self.extension_packages.len() {
            self.message = format!("{id} is not selected in this session");
        } else if packages.is_empty() {
            self.stop_extension_host();
            self.extension_packages.clear();
        } else if let Err(error) = self.start_extension_packages(packages) {
            self.message = format!("Cannot stop selected extension: {error:#}");
        }
    }
    pub(super) fn retire_extension_host(&mut self, mut host: crate::extensions::Client) {
        self.clear_active_extension_bindings();
        self.cancel_extension_prompt();
        host.cancel_prompts();
        // A session has at most one live process. Starts are blocked until its
        // retirement acknowledges, including when a storage operation is active.
        assert!(self.extension_retirement.is_none());
        self.extension_retirement = Some(retire(host));
    }
    pub(super) fn stop_extension_host(&mut self) {
        self.pause_automatic_extensions();
        self.cancel_extension_start();
        self.keymap.clear_extension_bindings();
        if let Some(host) = self.extension_host.take() {
            self.retire_extension_host(host);
        }
        self.message = "Extension host stopped".into();
    }
    pub(super) fn cancel_extension_start(&mut self) {
        self.extension_epoch += 1;
        if let Some(job) = &mut self.extension_job {
            job.discard_host = true;
        }
    }
    pub(super) fn shutdown_extensions(&mut self) {
        self.cancel_extension_start();
        if let Some(host) = self.extension_host.take() {
            self.retire_extension_host(host);
        }
        // This runs only during App destruction, after editing has ended. Keep
        // canceled-start and retirement workers alive long enough to reap children.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.extension_retirement.is_some()
            || self.extension_job.as_ref().is_some_and(|job| job.startup())
        {
            self.poll_extension_management();
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    pub fn extension_status(&self, id: &str) -> String {
        if let Some(host) = &self.extension_host
            && let Some(package) = host.packages.iter().find(|p| p.id == id)
        {
            return format!(
                "{} {}",
                if host.ready {
                    match host.activation_states.get(id).map(String::as_str) {
                        Some("active") => "running",
                        Some("failed") => "failed",
                        Some("activating") => "activating",
                        _ => "dormant",
                    }
                } else {
                    "starting"
                },
                package.version
            );
        }
        if let Some(package) = self.extension_packages.iter().find(|p| p.id == id) {
            return format!(
                "selected {} ({})",
                package.version,
                if self
                    .extension_job
                    .as_ref()
                    .is_some_and(|j| j.epoch.is_some() && !j.discard_host)
                {
                    "starting"
                } else {
                    "stopped"
                }
            );
        }
        "inactive".into()
    }
    pub(super) fn poll_extension_management(&mut self) -> bool {
        let mut changed = false;
        if self.extension_retirement.as_ref().is_some_and(|receiver| {
            !matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            )
        }) {
            self.extension_retirement = None;
            changed = true;
        }
        let Some(job) = &self.extension_job else {
            return changed;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return changed,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Extension worker disconnected".into())
            }
        };
        let discard_host =
            job.discard_host || job.epoch.is_some_and(|epoch| epoch != self.extension_epoch);
        let current_view =
            matches!(self.modal, Some(Modal::ExtensionsLoading(id)) if Some(id) == job.view);
        let show_list = current_view && self.prompt.is_none() && self.focus == Focus::Editor;
        if current_view {
            self.modal = None;
        }
        if !matches!(result, Ok(Output::Started(_))) {
            self.extension_job = None;
        }
        match result {
            Err(error) => self.message = format!("Extension operation failed: {error}"),
            Ok(Output::RegistryItems(items, message)) => {
                self.message = message;
                if show_list {
                    self.modal = Some(Modal::ExtensionRegistry { items, selected: 0 });
                }
            }
            Ok(Output::Listed(items, message)) => {
                self.refresh_extension_catalog();
                self.message = message;
                if show_list {
                    self.modal = Some(Modal::Extensions { items, selected: 0 });
                }
            }
            Ok(Output::RefreshFailed(message)) => self.message = message,
            Ok(Output::Stopped) => {}
            Ok(Output::Started(_)) if discard_host => {}
            Ok(Output::Started(ticket)) => match ticket.take() {
                Ok(host) => {
                    self.keymap.clear_extension_bindings();
                    self.extension_host = Some(*host);
                    self.message = "Starting extension host…".into();
                }
                Err(error) => self.message = format!("Cannot accept extension host: {error:#}"),
            },
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn registry_reply_after_dismissal_preserves_new_prompt_and_dirty_buffer() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("edit.cpp");
        std::fs::write(&path, "original").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        app.doc_mut().insert("dirty", false);
        let id = app.doc().id;
        let revision = app.doc().revision;
        let (sender, receiver) = sync_channel(1);
        app.extension_job = Some(Job {
            view: Some(99),
            epoch: None,
            discard_host: false,
            receiver,
        });
        app.modal = Some(Modal::ExtensionsLoading(99));
        app.modal = None;
        app.start_prompt(PromptKind::QuickOpen, "later".into());
        sender
            .send(Ok(Output::RegistryItems(
                Vec::new(),
                "Search finished".into(),
            )))
            .unwrap();
        app.poll_extension_management();
        assert!(app.extension_job.is_none());
        assert!(app.modal.is_none());
        assert!(matches!(
            app.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::QuickOpen)
        ));
        assert_eq!(app.prompt.as_ref().unwrap().text, "later");
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().revision, revision);
        assert_eq!(app.doc().text.to_string(), "dirtyoriginal");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "original");
    }
    #[test]
    fn discarded_handoff_runs_blocking_teardown_in_the_worker() {
        struct Probe {
            dropped: SyncSender<std::thread::ThreadId>,
            release: Receiver<()>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                self.dropped.send(std::thread::current().id()).unwrap();
                self.release.recv().unwrap();
            }
        }
        let (ready, receiver) = sync_channel(1);
        let (dropped, drop_started) = sync_channel(1);
        let (release, released) = sync_channel(1);
        let (finished, finish) = sync_channel(1);
        let worker = std::thread::spawn(move || {
            let (ticket, retire) = handoff(Probe {
                dropped,
                release: released,
            });
            ready.send(ticket).ok().unwrap();
            retire();
            finished.send(()).unwrap();
        });
        let ticket = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(ticket);
        let teardown_thread = drop_started.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(teardown_thread, std::thread::current().id());
        assert!(finish.try_recv().is_err());
        // The UI can continue while the deliberately stalled teardown is owned elsewhere.
        let temp = tempfile::tempdir().unwrap();
        let mut app = App::new(temp.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.execute("type", json!({"text":"responsive"}));
        assert_eq!(app.doc().text.to_string(), "responsive");
        release.send(()).unwrap();
        finish.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
    }
    #[test]
    fn canceled_start_keeps_worker_slot_until_cleanup_acknowledges() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = App::new(temp.path().into(), Profile::Linux);
        let (sender, receiver) = sync_channel(1);
        app.extension_job = Some(Job {
            discard_host: false,
            receiver,
            view: None,
            epoch: None,
        });
        app.cancel_extension_start();
        app.manage_extension(Action::List);
        assert!(app.message.contains("already running"));
        assert!(!app.poll_extension_management());
        sender.send(Ok(Output::Stopped)).ok().unwrap();
        assert!(app.poll_extension_management());
        assert!(app.extension_job.is_none());
        assert!(app.documents.is_empty());
    }
    #[test]
    fn dismissed_loading_reply_cannot_replace_a_later_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = App::new(temp.path().into(), Profile::Linux);
        let (sender, receiver) = sync_channel(1);
        app.extension_job = Some(Job {
            discard_host: false,
            receiver,
            view: Some(99),
            epoch: None,
        });
        app.modal = Some(Modal::ExtensionsLoading(99));
        app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        app.start_prompt(PromptKind::QuickOpen, "unsaved query".into());
        sender
            .send(Ok(Output::Listed(Vec::new(), "Loaded".into())))
            .ok()
            .unwrap();
        assert!(app.poll_extension_management());
        assert!(app.modal.is_none());
        assert_eq!(app.prompt.as_ref().unwrap().text, "unsaved query");
    }

    #[test]
    fn committed_install_is_reported_even_when_listing_another_package_fails() {
        use std::{fs::File, io::Write};
        use zip::{ZipWriter, write::SimpleFileOptions};
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("package.vsix");
        let store = Store::new(temp.path().join("extensions"));
        for name in ["broken", "next"] {
            let mut zip = ZipWriter::new(File::create(&archive).unwrap());
            zip.start_file("extension/package.json", SimpleFileOptions::default())
                .unwrap();
            write!(
                zip,
                "{}",
                json!({"publisher":"example","name":name,"version":"1.0.0"})
            )
            .unwrap();
            zip.finish().unwrap();
            if name == "broken" {
                let item = store.install(&archive).unwrap();
                std::fs::write(item.path.join("package.json"), "broken metadata").unwrap();
            }
        }
        let mut app = App::new(temp.path().into(), Profile::Linux);
        app.extensions_directory = Some(store.root().into());
        app.manage_extension(Action::Install(archive));
        let started = std::time::Instant::now();
        while app.extension_job.is_some() {
            app.poll_extension_management();
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            app.message.starts_with("Installed example.next@1.0.0"),
            "{}",
            app.message
        );
        assert!(app.message.contains("installed-list refresh failed"));
        assert_eq!(store.get("example.next").unwrap().version, "1.0.0");
        assert!(app.modal.is_none());
    }
    #[test]
    fn stalled_retirement_does_not_block_input_and_retains_its_slot() {
        struct Probe(Receiver<()>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.recv().unwrap();
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        let (release, blocked) = sync_channel(1);
        app.extension_retirement = Some(retire(Probe(blocked)));
        app.execute("vscli.extensions.restart", Value::Null);
        assert!(app.message.contains("already running"));
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.execute("type", json!({"text":"responsive"}));
        assert_eq!(app.doc().text.to_string(), "responsive");
        assert!(!app.poll_extension_management());
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.extension_retirement.is_some() {
            app.poll_extension_management();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn outdated_start_ticket_is_retired_without_replacing_native_state() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let host = crate::extensions::Client::start(
            "node",
            &fixture,
            directory.path(),
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap();
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || {
            let (ticket, cleanup) = handoff(Box::new(host));
            sender.send(Ok(Output::Started(ticket))).ok().unwrap();
            cleanup();
            sender.send(Ok(Output::Stopped)).ok().unwrap();
        });
        app.extension_epoch = 2;
        app.extension_job = Some(Job {
            receiver,
            view: None,
            epoch: Some(1),
            discard_host: false,
        });
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.execute("type", json!({"text":"retained"}));
        let id = app.doc().id;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.extension_job.is_some() {
            app.poll_extension_management();
            assert!(app.extension_host.is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "retained");
        assert!(app.doc().dirty());
    }
    #[test]
    fn canceled_session_start_stays_stopped_and_preserves_native_edits() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        app.start_extension_packages(vec![crate::extensions::Package::read(&fixture).unwrap()])
            .unwrap();
        app.execute("vscli.extensions.stop", Value::Null);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.execute("type", json!({"text":"unsaved"}));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.extension_job.is_some() {
            app.poll();
            assert!(app.extension_host.is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.doc().text.to_string(), "unsaved");
        assert_eq!(app.extension_packages.len(), 1);
    }
    #[test]
    fn session_readiness_releases_the_operation_slot_before_another_picker_action() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.extensions_directory = Some(directory.path().join("store"));
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        app.start_extension_packages(vec![crate::extensions::Package::read(&fixture).unwrap()])
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !app.extension_host.as_ref().is_some_and(|host| host.ready) {
            app.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.extension_job.is_none());
        app.manage_extension(Action::List);
        assert!(matches!(app.modal, Some(Modal::ExtensionsLoading(_))));
    }
}
