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

#[derive(Debug)]
pub enum DiskChange {
    Unchanged,
    Changed(Option<ropey::Rope>),
}
pub struct ReadRequest {
    pub id: u64,
    pub revision: u64,
    pub saved_revision: u64,
    pub text_epoch: u64,
    pub save_generation: u64,
    pub publication_epoch: u64,
    pub path: PathBuf,
    pub baseline: Option<ropey::Rope>,
}
pub struct Snapshot {
    pub id: u64,
    pub revision: u64,
    pub saved_revision: u64,
    pub text_epoch: u64,
    pub save_generation: u64,
    pub publication_epoch: u64,
    pub path: PathBuf,
    pub content: Result<DiskChange, String>,
}
pub struct DiskJob {
    receiver: Receiver<Snapshot>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl DiskJob {
    pub fn start(documents: Vec<ReadRequest>) -> Self {
        Self::start_inner(documents, read_change)
    }
    fn start_inner(
        documents: Vec<ReadRequest>,
        mut read: impl FnMut(&std::path::Path, Option<&ropey::Rope>) -> anyhow::Result<DiskChange>
        + Send
        + 'static,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel(8);
        let worker = std::thread::spawn(move || {
            for ReadRequest {
                id,
                revision,
                saved_revision,
                text_epoch,
                save_generation,
                publication_epoch,
                path,
                baseline,
            } in documents
            {
                let content = read(&path, baseline.as_ref()).map_err(|e| e.to_string());
                if sender
                    .send(Snapshot {
                        id,
                        revision,
                        saved_revision,
                        text_epoch,
                        save_generation,
                        publication_epoch,
                        path,
                        content,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            receiver,
            worker: Some(worker),
        }
    }
    #[cfg(test)]
    pub(crate) fn start_with_read(
        documents: Vec<ReadRequest>,
        read: impl FnMut(&std::path::Path, Option<&ropey::Rope>) -> anyhow::Result<DiskChange>
        + Send
        + 'static,
    ) -> Self {
        Self::start_inner(documents, read)
    }
    pub fn poll(&mut self) -> Result<Option<Snapshot>, mpsc::TryRecvError> {
        match self.receiver.try_recv() {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                // Sender closure is not sufficient proof that actual work has
                // exited. Retain its slot until the thread positively settles.
                if self
                    .worker
                    .as_ref()
                    .is_some_and(|worker| !worker.is_finished())
                {
                    return Ok(None);
                }
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                Err(mpsc::TryRecvError::Disconnected)
            }
        }
    }
}

fn read_change(
    path: &std::path::Path,
    baseline: Option<&ropey::Rope>,
) -> anyhow::Result<DiskChange> {
    if crate::document::disk_matches(path, baseline)? {
        return Ok(DiskChange::Unchanged);
    }
    crate::document::read_disk(path).map(DiskChange::Changed)
}

/// A save failure owns duplicate conflict notices only while its model proof holds.
pub(crate) struct SaveConflictProof {
    pub path: PathBuf,
    pub text_epoch: u64,
    pub save_generation: u64,
}
impl SaveConflictProof {
    pub fn current(&self, document: &crate::document::Document) -> bool {
        document.path.as_ref() == Some(&self.path)
            && document.text_epoch() == self.text_epoch
            && document.save_generation() == self.save_generation
    }
}
pub struct Notice {
    pub(crate) text: String,
    pub(crate) save_conflict: Option<SaveConflictProof>,
}

pub struct State {
    pub monitor: Monitor,
    pub paths: Vec<PathBuf>,
    pub pending: u8,
    pub last_event: Instant,
    pub last_refresh: Instant,
    pub last_read: Instant,
    pub disk: Option<DiskJob>,
    pub publication_epoch: u64,
    pub publication_disabled: bool,
    pub notices: std::collections::HashMap<u64, Notice>,
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
            publication_epoch: 1,
            publication_disabled: false,
            notices: std::collections::HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_checks_distinguish_unchanged_modified_created_deleted_and_invalid_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("watched.txt");
        let text = "first🙂\r\nlast\n".repeat(10000);
        let baseline = ropey::Rope::from_str(&text);
        std::fs::write(&path, &text).unwrap();
        assert!(matches!(
            read_change(&path, Some(&baseline)).unwrap(),
            DiskChange::Unchanged
        ));
        // A same-length mutation must be read, not dismissed by metadata alone.
        let changed = format!("X{}", &text[1..]);
        std::fs::write(&path, &changed).unwrap();
        match read_change(&path, Some(&baseline)).unwrap() {
            DiskChange::Changed(Some(content)) => assert_eq!(content, changed),
            other => panic!("Expected changed text, got {other:?}"),
        }
        std::fs::write(&path, b"invalid\xff").unwrap();
        assert!(read_change(&path, Some(&baseline)).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(
            read_change(&path, Some(&baseline)).unwrap(),
            DiskChange::Changed(None)
        ));
        assert!(matches!(
            read_change(&path, None).unwrap(),
            DiskChange::Unchanged
        ));
        std::fs::write(&path, "").unwrap();
        assert!(
            matches!(read_change(&path, None).unwrap(), DiskChange::Changed(Some(content)) if content.len_bytes() == 0)
        );
    }
}
