use super::*;
use crate::extension_store::{Installed, Store};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, SyncSender, sync_channel},
};
pub(super) enum Action {
    List,
    Install(PathBuf),
    Uninstall(String),
    Rollback(String),
}
enum Output {
    Listed(Vec<Installed>, String),
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
pub(super) struct Job {
    view: Option<u64>,
    discard_host: bool,
    receiver: Receiver<std::result::Result<Output, String>>,
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
        std::thread::spawn(move || {
            let store = Store::new(directory);
            let result = (|| -> Result<Output> {
                let message = match action {
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
        });
        self.message = "Extension operation running…".into();
    }
    pub(super) fn run_installed_extension(&mut self, item: Installed) {
        if self.extension_job.is_some() {
            self.message = "An extension operation is already running".into();
            return;
        }
        let prepared = match crate::extensions::Client::prepare(
            &self.documents,
            self.active,
            &self.settings,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.message = format!("Cannot start extension: {error:#}");
                return;
            }
        };
        let (sender, receiver) = sync_channel(1);
        let node = self.extension_node.clone();
        let root = self.workspace.root.clone();
        let previous = self.extension_host.take();
        self.keymap.clear_extension_bindings();
        std::thread::spawn(move || {
            let result =
                crate::extensions::Client::start_prepared(&node, &item.path, &root, prepared);
            // Process::drop waits for its child; keep every pending-start teardown here.
            drop(previous);
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
        });
        self.message = format!("Starting {}…", item.id);
    }
    pub(super) fn cancel_extension_start(&mut self) {
        if let Some(job) = &mut self.extension_job {
            job.discard_host = true;
        }
    }
    pub(super) fn poll_extension_management(&mut self) -> bool {
        let Some(job) = &self.extension_job else {
            return false;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Extension worker disconnected".into())
            }
        };
        let discard_host = job.discard_host;
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
            Ok(Output::Listed(items, message)) => {
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
}
