//! Clean-file layout metadata. Each process owns a leased slot; all I/O runs on one worker.
use crate::document::{Document, Selection};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub const MAX_DOCUMENTS: usize = 32;
pub const MAX_VIEWS: usize = 4;
pub const MAX_SELECTIONS: usize = 128;
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_READ_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SLOTS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedSelection {
    pub cursor: Position,
    pub anchor: Option<Position>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub selections: Vec<SavedSelection>,
    pub top: usize,
    pub left: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedFile {
    pub path: PathBuf,
    pub view: View,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    pub file: usize,
    pub view: View,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub files: Vec<SavedFile>,
    pub panes: Vec<Pane>,
    pub active_file: usize,
    pub active_pane: usize,
    pub horizontal: bool,
}
impl Layout {
    pub fn validate(&self) -> Result<()> {
        if self.files.len() > MAX_DOCUMENTS || self.panes.len() > MAX_VIEWS {
            bail!("Session exceeds 32 files or four panes");
        }
        let mut paths = std::collections::HashSet::new();
        for file in &self.files {
            if !file.path.is_absolute()
                || file.path.as_os_str().len() > 4096
                || !paths.insert(&file.path)
            {
                bail!("Invalid or duplicate session file path");
            }
            file.view.validate()?;
        }
        if self.files.is_empty() {
            if !self.panes.is_empty() || self.active_file != 0 || self.active_pane != 0 {
                bail!("Invalid empty session");
            }
        } else if self.active_file >= self.files.len()
            || self.panes.is_empty()
            || self.active_pane >= self.panes.len()
            || self.panes[self.active_pane].file != self.active_file
        {
            bail!("Invalid active session file or pane");
        }
        for pane in &self.panes {
            if pane.file >= self.files.len() {
                bail!("Invalid session pane file");
            }
            pane.view.validate()?;
        }
        Ok(())
    }
}
impl View {
    pub fn capture(doc: &Document, view: &crate::document::ViewState) -> Result<Self> {
        if view.secondary.len() >= MAX_SELECTIONS {
            bail!("Session view exceeds 128 selections");
        }
        let position = |value: usize| {
            let value = value.min(doc.len());
            let line = doc.text.char_to_line(value);
            Position {
                line,
                character: value - doc.text.line_to_char(line),
            }
        };
        let mut selections = vec![SavedSelection {
            cursor: position(view.cursor),
            anchor: view.anchor.map(position),
        }];
        selections.extend(view.secondary.iter().map(|s| SavedSelection {
            cursor: position(s.cursor),
            anchor: s.anchor.map(position),
        }));
        let saved = Self {
            selections,
            top: view.top,
            left: view.left,
        };
        saved.validate()?;
        Ok(saved)
    }
    fn validate(&self) -> Result<()> {
        if self.selections.is_empty() || self.selections.len() > MAX_SELECTIONS {
            bail!("Session view exceeds 128 selections or has no cursor");
        }
        Ok(())
    }
    pub fn apply(&self, doc: &mut Document) {
        let position = |p: &Position| {
            let row = p.line.min(doc.line_count() - 1);
            doc.line_start(row) + p.character.min(doc.line_end(row) - doc.line_start(row))
        };
        let selections = self
            .selections
            .iter()
            .map(|s| Selection {
                cursor: position(&s.cursor),
                anchor: s.anchor.as_ref().map(position),
                desired_column: None,
            })
            .collect();
        doc.set_selections(selections);
        doc.top = self.top.min(doc.line_count() - 1);
        doc.left = self.left.min(crate::document::MAX_FILE_BYTES as usize);
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    schema: u32,
    workspace: PathBuf,
    stamp: u64,
    layout: Layout,
}
fn regular_open(path: &Path, create: bool) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => bail!("Session state must be a regular file"),
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(create)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Session state must be a regular file");
    }
    Ok(file)
}
fn lock(file: &File) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => bail!("Session metadata lock unavailable: {error}"),
        }
    }
}
fn read(path: &Path, workspace: &Path) -> Result<Saved> {
    let mut bytes = Vec::new();
    regular_open(path, false)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Session metadata exceeds 1 MiB");
    }
    let saved: Saved = serde_json::from_slice(&bytes).context("Invalid session metadata")?;
    if saved.schema != 1 || saved.workspace != workspace {
        bail!("Session schema/workspace mismatch");
    }
    saved.layout.validate()?;
    Ok(saved)
}
struct Store {
    directory: PathBuf,
    workspace: PathBuf,
    path: PathBuf,
    lease_path: PathBuf,
    lease: File,
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = self.lease.unlock();
    }
}
impl Store {
    fn new(config: &Path, workspace: &Path) -> Result<(Self, Option<Layout>)> {
        let workspace = fs::canonicalize(workspace)?;
        let digest = Sha256::digest(workspace.as_os_str().as_encoded_bytes());
        let directory = config.join("state/sessions").join(format!("{digest:x}"));
        fs::create_dir_all(&directory)?;
        let registry = regular_open(&directory.join("registry.lock"), true)?;
        lock(&registry)?;
        let mut slots = Vec::new();
        for (count, entry) in fs::read_dir(&directory)?.enumerate() {
            if count >= 64 {
                bail!("Session directory entry budget exceeded");
            }
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "lock")
                && path.file_name().is_some_and(|n| n != "registry.lock")
            {
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    bail!("Invalid session slot");
                };
                if uuid::Uuid::parse_str(stem).is_err() {
                    bail!("Invalid session slot");
                }
                let lease = regular_open(&path, false)?;
                // Windows requires write access for an exclusive file lock.
                drop(lease);
                let lease = regular_open(&path, true)?;
                match lease.try_lock() {
                    Ok(()) => {
                        let saved_path = path.with_extension("json");
                        let saved = if saved_path.exists() {
                            Some(read(&saved_path, &workspace)?)
                        } else {
                            None
                        };
                        slots.push((path, Some(lease), saved));
                    }
                    Err(fs::TryLockError::WouldBlock) => slots.push((path, None, None)),
                    Err(error) => return Err(error.into()),
                }
            }
        }
        slots.sort_by_key(|(_, _, saved)| saved.as_ref().map_or(0, |s| s.stamp));
        let previous = slots
            .iter()
            .rev()
            .find_map(|(_, _, saved)| saved.as_ref().map(|s| s.layout.clone()));
        if slots.len() > MAX_SLOTS {
            bail!("Session slot budget exceeded");
        }
        let newest = slots.iter().rposition(|(_, _, saved)| saved.is_some());
        if slots.len() >= MAX_SLOTS {
            let Some(index) = slots
                .iter()
                .enumerate()
                .position(|(index, (_, lease, _))| lease.is_some() && Some(index) != newest)
            else {
                bail!("All eight session slots are in use");
            };
            let (path, lease, _) = slots.remove(index);
            match fs::remove_file(path.with_extension("json")) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            drop(lease);
            fs::remove_file(path)?;
        }
        let lease_path = directory.join(format!("{}.lock", uuid::Uuid::new_v4()));
        let lease = regular_open(&lease_path, true)?;
        lease.try_lock()?;
        let path = lease_path.with_extension("json");
        Ok((
            Self {
                directory,
                workspace,
                path,
                lease_path,
                lease,
            },
            previous,
        ))
    }
    fn publish(&self, layout: &Layout) -> Result<()> {
        self.publish_with(layout, || Ok(()))
    }
    fn publish_with(
        &self,
        layout: &Layout,
        before_publish: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        layout.validate()?;
        let saved = Saved {
            schema: 1,
            workspace: self.workspace.clone(),
            stamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_micros()
                .min(u64::MAX as u128) as u64,
            layout: layout.clone(),
        };
        let bytes = serde_json::to_vec(&saved)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("Session metadata exceeds 1 MiB");
        }
        if self.path.exists() {
            let _ = regular_open(&self.path, false)?;
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        before_publish()?;
        temp.persist(&self.path).map_err(|e| e.error)?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
    fn finish(self) -> Result<()> {
        if !self.path.exists() {
            let registry = regular_open(&self.directory.join("registry.lock"), true)?;
            lock(&registry)?;
            fs::remove_file(&self.lease_path)?;
        }
        Ok(())
    }
}

pub struct Restored {
    pub layout: Layout,
    pub documents: Vec<Document>,
}
fn restore(layout: &Layout, skip: &[PathBuf]) -> Result<Restored> {
    restore_with_budget(layout, skip, MAX_READ_BYTES)
}
fn restore_with_budget(layout: &Layout, skip: &[PathBuf], mut remaining: u64) -> Result<Restored> {
    layout.validate()?;
    let mut documents = Vec::new();
    for file in &layout.files {
        if skip.contains(&file.path) {
            continue;
        }
        let doc = Document::open_existing_bounded(&file.path, remaining)?;
        remaining = remaining
            .checked_sub(doc.text.len_bytes() as u64)
            .context("Session files exceed 128 MiB")?;
        documents.push(doc);
    }
    Ok(Restored {
        layout: layout.clone(),
        documents,
    })
}
enum Request {
    Save(Layout),
    Restore(Vec<PathBuf>),
    Finish(Option<Layout>),
}
pub enum Event {
    Ready(bool),
    Saved,
    Restored(Restored),
    Failed(String),
}
pub struct Worker {
    sender: SyncSender<Request>,
    receiver: Receiver<Event>,
    pending: bool,
    queued: Option<Layout>,
    saved: Option<Layout>,
    inflight: Option<Layout>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}
impl Worker {
    pub fn start(config: PathBuf, workspace: PathBuf) -> Result<Self> {
        let (sender, requests) = mpsc::sync_channel(1);
        let (events, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("vscli-session".into())
            .spawn(move || {
                let (store, previous) = match Store::new(&config, &workspace) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = events.send(Event::Failed(format!("{error:#}")));
                        return Err(error);
                    }
                };
                if events.send(Event::Ready(previous.is_some())).is_err() {
                    return store.finish();
                }
                while let Ok(request) = requests.recv() {
                    let result = match request {
                        Request::Save(layout) => store.publish(&layout).map(|()| Event::Saved),
                        Request::Restore(skip) => previous
                            .as_ref()
                            .context("No previous clean-file session")
                            .and_then(|layout| restore(layout, &skip))
                            .map(Event::Restored),
                        Request::Finish(layout) => {
                            if let Some(layout) = layout {
                                store.publish(&layout)?;
                            }
                            return store.finish();
                        }
                    };
                    if events
                        .send(result.unwrap_or_else(|e| Event::Failed(format!("{e:#}"))))
                        .is_err()
                    {
                        break;
                    }
                }
                store.finish()
            })?;
        Ok(Self {
            sender,
            receiver,
            pending: true,
            queued: None,
            saved: None,
            inflight: None,
            thread: Some(thread),
        })
    }
    pub fn poll(&mut self) -> Option<Event> {
        let event = match self.receiver.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) if self.pending => {
                Event::Failed("Session worker stopped".into())
            }
            Err(_) => return None,
        };
        self.pending = false;
        if matches!(event, Event::Saved) {
            self.saved = self.inflight.take();
        } else if matches!(event, Event::Failed(_))
            && let Some(layout) = self.inflight.take()
            && self.queued.is_none()
        {
            self.queued = Some(layout);
        }
        Some(event)
    }
    pub fn save(&mut self, layout: Layout) -> Result<()> {
        if !self.pending && self.saved.as_ref() == Some(&layout) && self.queued.is_none() {
            return Ok(());
        }
        self.queued = Some(layout);
        self.flush()
    }
    pub fn flush(&mut self) -> Result<()> {
        if !self.pending
            && let Some(layout) = self.queued.take()
        {
            if let Err(error) = self.sender.try_send(Request::Save(layout.clone())) {
                self.queued = Some(layout);
                return Err(error).context("Session worker unavailable");
            }
            self.inflight = Some(layout);
            self.pending = true;
        }
        Ok(())
    }
    pub fn restore(&mut self, skip: Vec<PathBuf>) -> Result<()> {
        if self.pending {
            bail!("Session worker is busy; retry shortly");
        }
        self.sender
            .try_send(Request::Restore(skip))
            .context("Session worker unavailable")?;
        self.pending = true;
        Ok(())
    }
    /// A final snapshot supersedes queued state. None deliberately suppresses
    /// unsent publication after failed/stale restoration; issued I/O may finish.
    pub fn finish(self, layout: Option<Layout>) -> Result<()> {
        self.finish_with_timeout(layout, Duration::from_secs(5))
    }
    fn finish_with_timeout(mut self, layout: Option<Layout>, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut request = Some(Request::Finish(layout));
        loop {
            // Drain replies even when input stopped before the initial Ready event.
            while self.receiver.try_recv().is_ok() {}
            if let Some(next) = request.take() {
                match self.sender.try_send(next) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(next)) => request = Some(next),
                    Err(mpsc::TrySendError::Disconnected(_)) => {}
                }
            }
            if self.thread.as_ref().unwrap().is_finished() {
                return self
                    .thread
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| anyhow::anyhow!("Session worker panicked"))?;
            }
            if Instant::now() >= deadline {
                // Dropping channels cancels further replies/work. A blocked operation
                // keeps the lease until the sole detached worker actually exits.
                bail!("Session shutdown timed out; last published metadata retained");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(path: &Path) -> Layout {
        let view = View {
            selections: vec![SavedSelection {
                cursor: Position {
                    line: 0,
                    character: 2,
                },
                anchor: None,
            }],
            top: 0,
            left: 0,
        };
        Layout {
            files: vec![SavedFile {
                path: path.to_owned(),
                view: view.clone(),
            }],
            panes: vec![Pane { file: 0, view }],
            ..Layout::default()
        }
    }
    fn await_event(worker: &mut Worker) -> Event {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(event) = worker.poll() {
                return event;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn leases_isolate_live_instances_workspaces_and_config_roots() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let other = root.path().join("other");
        fs::create_dir(&other).unwrap();
        let file = root.path().join("file.txt");
        fs::write(&file, "disk text").unwrap();
        let saved = layout(&file);
        let (first, prior) = Store::new(&config, root.path()).unwrap();
        assert!(prior.is_none());
        first.publish(&saved).unwrap();
        let (second, prior) = Store::new(&config, root.path()).unwrap();
        assert!(prior.is_none(), "A live instance must not be restored");
        assert_ne!(first.path, second.path);
        drop(first);
        let (third, prior) = Store::new(&config, root.path()).unwrap();
        assert_eq!(prior, Some(saved));
        assert!(
            Store::new(&root.path().join("other-config"), root.path())
                .unwrap()
                .1
                .is_none()
        );
        assert!(Store::new(&config, &other).unwrap().1.is_none());
        second.finish().unwrap();
        third.finish().unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "disk text");
    }
    #[test]
    fn capacity_never_evicts_live_leases_or_the_only_retryable_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let missing = root.path().join("missing.txt");
        let saved = layout(&missing);
        let (previous, _) = Store::new(&config, root.path()).unwrap();
        previous.publish(&saved).unwrap();
        let path = previous.path.clone();
        let bytes = fs::read(&path).unwrap();
        drop(previous);
        let mut live = Vec::new();
        for _ in 0..7 {
            live.push(Store::new(&config, root.path()).unwrap().0);
        }
        assert!(Store::new(&config, root.path()).is_err());
        assert!(restore(&saved, &[]).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
        for store in live {
            assert!(store.lease_path.exists());
            store.finish().unwrap();
        }
    }
    #[test]
    fn malformed_oversized_and_failed_publication_retain_previous_bytes() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let saved = layout(&root.path().join("file.txt"));
        store.publish(&saved).unwrap();
        let before = fs::read(&store.path).unwrap();
        let mut bad = saved.clone();
        bad.panes[0].file = 90;
        assert!(store.publish(&bad).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert!(
            store
                .publish_with(&Layout::default(), || bail!(
                    "injected failure after sync before rename"
                ))
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert_eq!(read(&store.path, root.path()).unwrap().layout, saved);
        for bytes in [b"malformed".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            fs::write(&store.path, &bytes).unwrap();
            assert!(read(&store.path, root.path()).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
        }
    }
    #[test]
    fn restore_bounds_combined_reads_and_never_creates_missing_files() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.txt");
        let second = root.path().join("second.txt");
        fs::write(&first, "123456").unwrap();
        fs::write(&second, "abcdef").unwrap();
        let mut saved = layout(&first);
        saved.files.push(SavedFile {
            path: second.clone(),
            view: saved.files[0].view.clone(),
        });
        assert!(restore_with_budget(&saved, &[], 11).is_err());
        assert_eq!(
            restore_with_budget(&saved, &[], 12)
                .unwrap()
                .documents
                .len(),
            2
        );
        assert_eq!(
            restore_with_budget(&saved, std::slice::from_ref(&first), 6)
                .unwrap()
                .documents
                .len(),
            1
        );
        fs::remove_file(&second).unwrap();
        assert!(restore(&saved, &[]).is_err());
        assert!(!second.exists());
        let mut bad = saved.clone();
        bad.files = vec![saved.files[0].clone(); 33];
        assert!(bad.validate().is_err());
        bad = saved.clone();
        bad.panes = vec![saved.panes[0].clone(); 5];
        assert!(bad.validate().is_err());
        bad = saved.clone();
        bad.files[0].view.selections = vec![saved.files[0].view.selections[0].clone(); 129];
        assert!(bad.validate().is_err());
        assert_eq!(fs::read_to_string(first).unwrap(), "123456");
    }
    #[test]
    fn queued_snapshots_coalesce_and_shutdown_drains_initial_and_save_replies() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let saved = layout(&root.path().join("first.txt"));
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        worker.save(saved.clone()).unwrap(); // Ready has not been polled.
        worker.finish(Some(saved.clone())).unwrap();
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(true)));
        worker.save(Layout::default()).unwrap();
        worker.save(saved.clone()).unwrap();
        worker.finish(Some(saved.clone())).unwrap(); // Saved may still be queued.
        let (_, previous) = Store::new(&config, root.path()).unwrap();
        assert_eq!(previous, Some(saved));
    }
    #[test]
    fn returning_to_acknowledged_state_while_another_write_runs_preserves_latest_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let a = layout(&root.path().join("a.txt"));
        let b = layout(&root.path().join("b.txt"));
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(false)));
        worker.save(a.clone()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.save(b).unwrap();
        worker.save(a.clone()).unwrap();
        assert_eq!(worker.queued.as_ref(), Some(&a));
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.flush().unwrap();
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.finish(None).unwrap();
        assert_eq!(Store::new(&config, root.path()).unwrap().1, Some(a));
    }
    #[test]
    fn shutdown_deadline_does_not_join_a_blocked_worker() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (events, receiver) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            wait.recv().unwrap();
            let _ = events.send(Event::Ready(false));
            drop(requests);
            Ok(())
        });
        let worker = Worker {
            sender,
            receiver,
            pending: true,
            queued: None,
            saved: None,
            inflight: None,
            thread: Some(thread),
        };
        let started = Instant::now();
        assert!(
            worker
                .finish_with_timeout(None, Duration::from_millis(30))
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn fifo_state_and_targets_are_rejected_without_opening_a_blocking_reader() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("fifo");
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(regular_open(&fifo, false).is_err());
        assert!(restore(&layout(&fifo), &[]).is_err());
    }
}
