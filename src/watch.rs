//! Native notifications are hints; reads are bounded and checked against document versions.
use notify::{EventKind, RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

pub const CONTENT: u8 = 1;
pub const INDEX: u8 = 2;
pub struct Monitor {
    paths: SyncSender<Vec<PathBuf>>,
    flags: Arc<AtomicU8>,
    error: Arc<Mutex<Option<String>>>,
}
impl Monitor {
    pub fn start(root: PathBuf) -> Self {
        let (paths, receiver) = mpsc::sync_channel::<Vec<PathBuf>>(1);
        let flags = Arc::new(AtomicU8::new(0));
        let changes = flags.clone();
        let error = Arc::new(Mutex::new(None));
        let errors = error.clone();
        std::thread::spawn(move || {
            let event_flags = changes.clone();
            let event_errors = errors.clone();
            let event_root = root.clone();
            let watcher = notify::recommended_watcher(
                move |event: notify::Result<notify::Event>| match event {
                    Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                        if !event.paths.is_empty()
                            && event.paths.iter().all(|p| {
                                p.strip_prefix(&event_root).is_ok_and(|p| {
                                    p.components().any(|part| {
                                        matches!(
                                            part.as_os_str().to_str(),
                                            Some(
                                                ".git"
                                                    | "target"
                                                    | "node_modules"
                                                    | ".venv"
                                                    | "__pycache__"
                                            )
                                        )
                                    })
                                })
                            })
                        {
                            return;
                        }
                        let index = !matches!(
                            event.kind,
                            EventKind::Modify(notify::event::ModifyKind::Data(_))
                        );
                        event_flags
                            .fetch_or(CONTENT | if index { INDEX } else { 0 }, Ordering::Relaxed);
                    }
                    Err(e) => {
                        if let Ok(mut error) = event_errors.lock() {
                            *error = Some(e.to_string());
                        }
                        event_flags.fetch_or(CONTENT | INDEX, Ordering::Relaxed);
                    }
                    _ => {}
                },
            );
            let mut watcher = match watcher {
                Ok(mut watcher) => {
                    if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive)
                        && let Ok(mut error) = errors.lock()
                    {
                        *error = Some(e.to_string());
                    }
                    Some(watcher)
                }
                Err(e) => {
                    if let Ok(mut error) = errors.lock() {
                        *error = Some(e.to_string());
                    }
                    None
                }
            };
            let mut parents = BTreeSet::new();
            loop {
                match receiver.recv_timeout(Duration::from_millis(200)) {
                    Ok(paths) => {
                        let next: BTreeSet<_> = paths
                            .iter()
                            .filter(|p| !p.starts_with(&root))
                            .filter_map(|p| p.parent().map(|p| p.to_path_buf()))
                            .collect();
                        if let Some(watcher) = &mut watcher {
                            for old in parents.difference(&next) {
                                let _ = watcher.unwatch(old);
                            }
                            for new in next.difference(&parents) {
                                if let Err(e) = watcher.watch(new, RecursiveMode::NonRecursive)
                                    && let Ok(mut error) = errors.lock()
                                {
                                    *error = Some(e.to_string());
                                }
                            }
                        }
                        parents = next;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self {
            paths,
            flags,
            error,
        }
    }
    pub fn set_paths(&self, paths: Vec<PathBuf>) -> bool {
        self.paths.try_send(paths).is_ok()
    }
    pub fn poll(&self) -> (u8, Option<String>) {
        (
            self.flags.swap(0, Ordering::Relaxed),
            self.error.lock().ok().and_then(|mut e| e.take()),
        )
    }
}

pub struct Snapshot {
    pub id: u64,
    pub revision: u64,
    pub saved_revision: u64,
    pub path: PathBuf,
    pub content: Result<Option<ropey::Rope>, String>,
}
pub struct DiskJob {
    receiver: Receiver<Snapshot>,
}
impl DiskJob {
    pub fn start(documents: Vec<(u64, u64, u64, PathBuf)>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for (id, revision, saved_revision, path) in documents {
                let content = crate::document::read_disk(&path).map_err(|e| e.to_string());
                if sender
                    .send(Snapshot {
                        id,
                        revision,
                        saved_revision,
                        path,
                        content,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self { receiver }
    }
    pub fn poll(&self) -> Result<Option<Snapshot>, mpsc::TryRecvError> {
        match self.receiver.try_recv() {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

pub struct State {
    pub monitor: Monitor,
    pub paths: Vec<PathBuf>,
    pub pending: u8,
    pub last_event: Instant,
    pub last_refresh: Instant,
    pub last_read: Instant,
    pub disk: Option<DiskJob>,
    pub notices: std::collections::HashMap<u64, String>,
}
impl State {
    pub fn new(root: PathBuf) -> Self {
        Self {
            monitor: Monitor::start(root),
            paths: Vec::new(),
            pending: 0,
            last_event: Instant::now(),
            last_refresh: Instant::now(),
            last_read: Instant::now(),
            disk: None,
            notices: std::collections::HashMap::new(),
        }
    }
}
