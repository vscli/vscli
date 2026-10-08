use super::*;
use crate::extension_store::{Installed, Store};
use std::sync::mpsc::{Receiver, sync_channel};
pub(super) enum Action {
    List,
    Install(PathBuf),
    Uninstall(String),
    Rollback(String),
}
enum Output {
    Listed(Vec<Installed>, String),
    Started(Box<crate::extensions::Client>),
}
pub(super) struct Job {
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
                Ok(Output::Listed(store.list()?, message))
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        self.extension_job = Some(Job {
            receiver,
            discard_host: false,
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
        std::thread::spawn(move || {
            let result =
                crate::extensions::Client::start_prepared(&node, &item.path, &root, prepared)
                    .map(|host| Output::Started(Box::new(host)))
                    .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        self.extension_job = Some(Job {
            receiver,
            discard_host: false,
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
        self.extension_job = None;
        match result {
            Err(error) => self.message = format!("Extension operation failed: {error}"),
            Ok(Output::Listed(items, message)) => {
                self.message = message;
                if self.prompt.is_none() && self.modal.is_none() && self.focus == Focus::Editor {
                    self.modal = Some(Modal::Extensions { items, selected: 0 });
                }
            }
            Ok(Output::Started(_)) if discard_host => {}
            Ok(Output::Started(host)) => {
                self.keymap.clear_extension_bindings();
                self.extension_host = Some(*host);
                self.message = "Starting extension host…".into();
            }
        }
        true
    }
}
