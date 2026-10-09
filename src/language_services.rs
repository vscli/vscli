//! Bounded executable discovery and process ownership for native language services.
use crate::lsp::Client;
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub workspace: PathBuf,
    pub language: String,
    pub program: String,
    pub args: Vec<String>,
}
impl Launch {
    pub fn validate(&self) -> Result<()> {
        if !self.workspace.is_absolute()
            || self.program.is_empty()
            || self.program.len() > 4096
            || self.args.len() > 32
            || self.args.iter().any(|a| a.len() > 4096)
            || self.args.iter().map(String::len).sum::<usize>() > 16384
        {
            bail!("Language server configuration exceeds native launch limits");
        }
        Ok(())
    }
}
pub fn builtin(language: &str) -> Option<(&'static str, &'static str)> {
    match language {
        "c" | "cpp" => Some(("clangd", "cpp")),
        "rust" => Some(("rust-analyzer", "rust")),
        _ => None,
    }
}
fn executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}
fn discover(launch: &Launch, search: &std::ffi::OsStr) -> Result<PathBuf> {
    launch.validate()?;
    let program = Path::new(&launch.program);
    if program.is_absolute() || program.components().count() > 1 {
        let path = if program.is_absolute() {
            program.to_owned()
        } else {
            launch.workspace.join(program)
        };
        if !executable(&path) {
            bail!(
                "Language server executable is unavailable: {}",
                path.display()
            );
        }
        return Ok(path);
    }
    if search.len() > 65536 {
        bail!("Executable PATH exceeds 64 KiB");
    }
    for (index, folder) in std::env::split_paths(search).enumerate() {
        if index >= 128 {
            bail!("Executable PATH exceeds 128 entries");
        }
        // Empty/relative entries implicitly execute code in the opened repository.
        if !folder.is_absolute() {
            continue;
        }
        let path = folder.join(program);
        #[cfg(windows)]
        let path = if path.extension().is_none() {
            path.with_extension("exe")
        } else {
            path
        };
        if executable(&path) {
            return Ok(path);
        }
    }
    bail!(
        "{} is not installed on the executable PATH; install it or configure a server, then use Language: Restart Server",
        launch.program
    )
}

/// The worker retains an ownership reference until explicitly retired. Dropping
/// an unaccepted response on the UI thread therefore never waits for a process.
pub struct Offer(Arc<Mutex<Option<Client>>>);
impl Offer {
    pub fn accept(self) -> Option<Client> {
        self.0.lock().ok()?.take()
    }
}
enum Job {
    Start(u64, Launch),
    Retire(Box<Option<Client>>),
}
pub enum Event {
    Started(u64, Result<Offer, String>),
    Retired,
}
struct Shutdown(Arc<Mutex<Option<Client>>>);
impl Drop for Shutdown {
    fn drop(&mut self) {
        if let Ok(mut client) = self.0.lock() {
            drop(client.take());
        }
    }
}
pub struct Worker {
    sender: Option<SyncSender<Job>>,
    receiver: Option<Receiver<Event>>,
    shutdown: Arc<Mutex<Option<Client>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start() -> Result<Self> {
        Self::start_with_path(std::env::var_os("PATH").unwrap_or_default())
    }
    fn start_with_path(search: OsString) -> Result<Self> {
        let (sender, jobs) = mpsc::sync_channel(1);
        let (events, receiver) = mpsc::sync_channel(1);
        let shutdown = Arc::new(Mutex::new(None));
        let owner = Shutdown(shutdown.clone());
        let thread = std::thread::Builder::new()
            .name("vscli-language-services".into())
            .spawn(move || {
                let _owner = owner;
                let mut offered: Option<Arc<Mutex<Option<Client>>>> = None;
                while let Ok(job) = jobs.recv() {
                    if let Some(slot) = offered.take()
                        && let Ok(mut client) = slot.lock()
                    {
                        drop(client.take());
                    }
                    let event = match job {
                        Job::Start(generation, launch) => {
                            let result = discover(&launch, &search)
                                .and_then(|path| {
                                    let program = path
                                        .to_str()
                                        .context("Language executable path is not UTF-8")?;
                                    Client::start_isolated(
                                        program,
                                        &launch.args,
                                        &launch.workspace,
                                        launch.language,
                                    )
                                })
                                .map(|client| {
                                    let slot = Arc::new(Mutex::new(Some(client)));
                                    offered = Some(slot.clone());
                                    Offer(slot)
                                })
                                .map_err(|error| format!("{error:#}"));
                            Event::Started(generation, result)
                        }
                        Job::Retire(client) => {
                            drop(client);
                            Event::Retired
                        }
                    };
                    if events.send(event).is_err() {
                        break;
                    }
                }
                if let Some(slot) = offered
                    && let Ok(mut client) = slot.lock()
                {
                    drop(client.take());
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            receiver: Some(receiver),
            shutdown,
            thread: Some(thread),
        })
    }
    pub fn launch(&self, generation: u64, launch: Launch) -> Result<()> {
        self.sender
            .as_ref()
            .context("Language worker stopped")?
            .try_send(Job::Start(generation, launch))
            .map_err(|_| anyhow::anyhow!("Language worker is busy"))
    }
    pub fn retire(&self, client: Option<Client>) -> Result<(), Box<Option<Client>>> {
        let Some(sender) = &self.sender else {
            return Err(Box::new(client));
        };
        sender
            .try_send(Job::Retire(Box::new(client)))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(Job::Retire(client))
                | mpsc::TrySendError::Disconnected(Job::Retire(client)) => client,
                _ => unreachable!(),
            })
    }
    pub fn stopped(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
    pub fn poll(&self) -> Option<Event> {
        self.receiver.as_ref()?.try_recv().ok()
    }
    pub fn finish(mut self, client: Option<Client>) {
        if let Ok(mut slot) = self.shutdown.lock() {
            *slot = client;
        }
        self.receiver.take();
        self.sender.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.thread.as_ref().is_some_and(|t| !t.is_finished()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        if self.thread.as_ref().is_some_and(|t| t.is_finished()) {
            let _ = self.thread.take().unwrap().join();
        }
        // A blocked discovery/retirement keeps its sole worker and owned process;
        // the editor does not wait indefinitely or spawn replacement workers.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_is_bounded_and_skips_implicit_workspace_search() {
        let root = tempfile::tempdir().unwrap();
        let launch = Launch {
            workspace: root.path().into(),
            language: "cpp".into(),
            program: "fixture-server".into(),
            args: vec![],
        };
        std::fs::write(root.path().join("fixture-server"), "test").unwrap();
        assert!(discover(&launch, std::ffi::OsStr::new(".")).is_err());
        assert!(discover(&launch, &OsString::from("x".repeat(65537))).is_err());
        assert!(
            discover(
                &Launch {
                    args: vec!["x".into(); 33],
                    ..launch.clone()
                },
                std::ffi::OsStr::new("")
            )
            .is_err()
        );
        assert!(
            discover(
                &Launch {
                    program: root.path().to_string_lossy().into_owned(),
                    ..launch
                },
                std::ffi::OsStr::new("")
            )
            .is_err()
        );
        assert_eq!(builtin("c"), builtin("cpp"));
        assert_eq!(builtin("rust"), Some(("rust-analyzer", "rust")));
        assert_eq!(builtin("plaintext"), None);
    }
    #[test]
    fn pending_reply_shutdown_does_not_wait_for_the_ui_to_drain() {
        let root = tempfile::tempdir().unwrap();
        let worker = Worker::start_with_path(OsString::new()).unwrap();
        worker
            .launch(
                1,
                Launch {
                    workspace: root.path().into(),
                    language: "cpp".into(),
                    program: "missing".into(),
                    args: vec![],
                },
            )
            .unwrap();
        let before = Instant::now();
        worker.finish(None);
        assert!(before.elapsed() < Duration::from_secs(2));
    }
}
