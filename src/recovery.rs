use crate::document::{Document, Selection};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, Serializer};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct SavedDocument<T = String> {
    path: Option<PathBuf>,
    text: T,
    disk_content: Option<T>,
    cursor: usize,
    #[serde(default)]
    selections: Vec<Selection>,
}
#[derive(Serialize, Deserialize)]
struct Session<T = String> {
    version: u32,
    documents: Vec<SavedDocument<T>>,
}

struct SharedText(ropey::Rope);
impl Serialize for SharedText {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // serde_json streams Display fragments, escaping each rope chunk without
        // materializing a document-sized String. The v1 JSON format is unchanged.
        serializer.collect_str(&self.0)
    }
}

impl Session<SharedText> {
    fn capture(documents: &[Document]) -> Self {
        Self::capture_refs(documents.iter().collect::<Vec<_>>().as_slice())
    }
    fn capture_refs(documents: &[&Document]) -> Self {
        Self {
            version: 1,
            documents: documents
                .iter()
                .filter(|d| d.dirty())
                .map(|d| SavedDocument {
                    path: d.path.clone(),
                    text: SharedText(d.text.clone()),
                    disk_content: d.disk_content.clone().map(SharedText),
                    cursor: d.cursor,
                    selections: d.selections(),
                })
                .collect(),
        }
    }
}

pub struct Recovery {
    pub directory: PathBuf,
    path: PathBuf,
    lock_path: PathBuf,
    _lock: File,
}
impl Drop for Recovery {
    fn drop(&mut self) {
        // A concurrent fork can briefly inherit this open file description.
        // Explicit unlock releases the session even before that child execs.
        let _ = self._lock.unlock();
    }
}
impl Recovery {
    pub fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = directory.join(format!("{}-{stamp}.json", std::process::id()));
        let lock_path = path.with_extension("lock");
        let lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        lock.lock()?;
        Ok(Self {
            directory,
            path,
            lock_path,
            _lock: lock,
        })
    }
    pub fn restore(&self) -> Result<(Vec<Document>, Vec<PathBuf>)> {
        let mut docs = Vec::new();
        let mut consumed = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") || path == self.path {
                continue;
            }
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path.with_extension("lock"))?;
            if lock.try_lock().is_err() {
                continue;
            }
            let data = fs::read_to_string(&path)?;
            let Ok(session) = serde_json::from_str::<Session>(&data) else {
                continue;
            };
            if session.version != 1 {
                continue;
            }
            for saved in session.documents {
                let baseline = saved.disk_content.as_deref().map(ropey::Rope::from_str);
                let mut doc = Document::from_rope(baseline.clone().unwrap_or_default());
                doc.path = saved.path;
                doc.disk_content = baseline;
                doc.move_to(saved.cursor.min(doc.len()), false);
                // Undo recovered edits returns to the actual saved baseline, not a clean
                // snapshot containing unsaved text. Empty recovered files matter too.
                doc.select_all();
                doc.insert(&saved.text, false);
                doc.move_to(saved.cursor.min(doc.len()), false);
                if !saved.selections.is_empty() {
                    doc.set_selections(saved.selections);
                }
                docs.push(doc);
            }
            consumed.push(path);
        }
        Ok((docs, consumed))
    }
    pub fn persist(&self, documents: &[Document]) -> Result<()> {
        self.persist_snapshot(&Session::capture(documents))
    }
    fn persist_snapshot(&self, session: &Session<SharedText>) -> Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        {
            let mut writer = BufWriter::new(temp.as_file_mut());
            serde_json::to_writer(&mut writer, session)?;
            writer.flush()?;
        }
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|e| e.error)
            .context("Could not save recovery snapshot")?;
        Ok(())
    }
    pub fn consume(paths: &[PathBuf]) {
        for path in paths {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(path.with_extension("lock"));
        }
    }
    pub fn finish(self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        self._lock.unlock()?;
        fs::remove_file(&self.lock_path)?;
        Ok(())
    }
}

type Signature = Vec<(u64, Option<PathBuf>, u64, u64)>;
struct Request {
    signature: Signature,
    session: Session<SharedText>,
}

/// One in-flight snapshot, with no backlog of obsolete document copies. The
/// worker owns the session lock until queued I/O completes, including shutdown.
pub struct Worker {
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<Result<Signature>>,
    thread: Option<JoinHandle<Recovery>>,
    pending: bool,
    persisted: Option<Signature>,
}
impl Worker {
    pub fn start(store: Recovery) -> Result<Self> {
        Self::start_with(store, Recovery::persist_snapshot)
    }
    fn start_with(
        store: Recovery,
        mut write: impl FnMut(&Recovery, &Session<SharedText>) -> Result<()> + Send + 'static,
    ) -> Result<Self> {
        let (sender, requests) = mpsc::sync_channel::<Request>(1);
        let (results, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("vscli-recovery".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    let result = write(&store, &request.session).map(|()| request.signature);
                    if results.send(result).is_err() {
                        break;
                    }
                }
                store
            })?;
        Ok(Self {
            sender: Some(sender),
            receiver,
            thread: Some(thread),
            pending: false,
            persisted: None,
        })
    }
    pub fn poll(&mut self) -> Result<()> {
        if self.pending {
            match self.receiver.try_recv() {
                Ok(result) => {
                    self.pending = false;
                    self.persisted = Some(result?);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = false;
                    anyhow::bail!("Recovery worker stopped before acknowledging its snapshot");
                }
            }
        }
        Ok(())
    }
    /// Capture only cheap shared ropes and metadata on the input thread. An
    /// acknowledgement records the submitted revisions, never the current ones.
    pub fn submit(&mut self, documents: &[Document]) -> Result<bool> {
        self.submit_refs(&documents.iter().collect::<Vec<_>>())
    }
    pub fn submit_refs(&mut self, documents: &[&Document]) -> Result<bool> {
        if self.pending {
            return Ok(false);
        }
        let signature: Signature = documents
            .iter()
            .map(|d| (d.id, d.path.clone(), d.revision, d.saved_revision))
            .collect();
        if self.persisted.as_ref() == Some(&signature) {
            return Ok(false);
        }
        self.sender
            .as_ref()
            .context("Recovery worker already closed")?
            .try_send(Request {
                signature,
                session: Session::capture_refs(documents),
            })
            .map_err(|_| anyhow::anyhow!("Recovery worker cannot accept a snapshot"))?;
        self.pending = true;
        Ok(true)
    }
    fn join(&mut self) -> Result<Recovery> {
        // At most one result can be waiting in the capacity-one result channel,
        // so the worker cannot block sending it while shutdown waits for I/O.
        self.sender.take();
        self.thread
            .take()
            .context("Recovery worker already closed")?
            .join()
            .map_err(|_| anyhow::anyhow!("Recovery worker panicked"))
    }
    /// Interruption must write the latest state after all earlier writes finish.
    /// This deliberately waits during shutdown, outside the interactive loop.
    pub fn preserve(self, documents: &[Document]) -> Result<()> {
        self.preserve_refs(&documents.iter().collect::<Vec<_>>())
    }
    pub fn preserve_refs(mut self, documents: &[&Document]) -> Result<()> {
        self.join()?
            .persist_snapshot(&Session::capture_refs(documents))
    }
    /// Normal confirmed exit removes the journal only after the worker stops.
    pub fn finish(mut self) -> Result<()> {
        self.join()?.finish()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            // Unexpected error exit retains the last queued snapshot and lock
            // until the writer is done; it must not race another editor's restore.
            let _ = self.join();
        }
    }
}
pub fn default_directory() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "vscli", "vscli")
        .map(|d| d.state_dir().unwrap_or(d.data_local_dir()).join("recovery"))
}
pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "vscli", "vscli")
        .map(|d| d.config_dir().join("keybindings.json"))
}
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace(|c: char| c.is_control(), "�")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn settle(worker: &mut Worker) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.pending {
            worker.poll()?;
            assert!(
                Instant::now() < deadline,
                "Recovery worker did not acknowledge"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    fn read_session(path: &Path) -> Session {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn shared_snapshots_stream_v1_json_and_retain_text_baselines_and_selections() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        let baseline = "quote: \" slash: \\ unicode: 猫🙂\r\n".repeat(10000);
        fs::write(&file, &baseline).unwrap();
        let mut d = Document::open(&file).unwrap();
        d.insert("edited\t\u{1}e\u{301}", false);
        d.set_selections(vec![Selection::caret(3), Selection::caret(15)]);
        let expected = d.text.to_string();
        let selections = d.selections();
        let snapshot = Session::capture(std::slice::from_ref(&d));
        d.select_all();
        d.insert("later edits must not mutate captured ropes", false);
        let store = Recovery::new(dir.path().join("state")).unwrap();
        store.persist_snapshot(&snapshot).unwrap();
        let saved = read_session(&store.path);
        assert_eq!(saved.version, 1);
        assert_eq!(saved.documents[0].text, expected);
        assert_eq!(
            saved.documents[0].disk_content.as_deref(),
            Some(baseline.as_str())
        );
        assert_eq!(saved.documents[0].selections, selections);
        drop(store);
        let restored = Recovery::new(dir.path().join("state")).unwrap();
        let (mut documents, _) = restored.restore().unwrap();
        assert_eq!(documents[0].text, expected);
        assert_eq!(documents[0].selections(), selections);
        documents[0].undo();
        assert_eq!(documents[0].text, baseline);
        assert!(!documents[0].dirty());
    }

    #[test]
    fn stalled_writes_do_not_block_submissions_or_acknowledge_later_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let store = Recovery::new(dir.path().into()).unwrap();
        let path = store.path.clone();
        let (started, began) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let mut first = true;
        let mut worker = Worker::start_with(store, move |store, session| {
            if first {
                first = false;
                started.send(())?;
                gate.recv_timeout(Duration::from_secs(5))?;
            }
            store.persist_snapshot(session)
        })
        .unwrap();
        let mut document = Document::default();
        document.insert("first", false);
        assert!(worker.submit(std::slice::from_ref(&document)).unwrap());
        began.recv_timeout(Duration::from_secs(5)).unwrap();
        document.insert(" second", false);
        // These calls complete while the writer is explicitly blocked. There is
        // no timing threshold or filesystem-speed assumption in this assertion.
        for _ in 0..100 {
            assert!(!worker.submit(std::slice::from_ref(&document)).unwrap());
            worker.poll().unwrap();
        }
        release.send(()).unwrap();
        settle(&mut worker).unwrap();
        assert_eq!(read_session(&path).documents[0].text, "first");
        // The old acknowledgement must not suppress this newer document state.
        assert!(worker.submit(std::slice::from_ref(&document)).unwrap());
        settle(&mut worker).unwrap();
        assert_eq!(read_session(&path).documents[0].text, "first second");
        assert!(!worker.submit(std::slice::from_ref(&document)).unwrap());
        // Closing the dirty document must replace its journal with an empty one.
        assert!(worker.submit(&[]).unwrap());
        settle(&mut worker).unwrap();
        assert!(read_session(&path).documents.is_empty());
        worker.finish().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn failed_snapshots_retain_last_good_state_and_retry_the_same_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = Recovery::new(dir.path().into()).unwrap();
        let path = store.path.clone();
        let mut document = Document::default();
        document.insert("last good", false);
        store.persist(std::slice::from_ref(&document)).unwrap();
        let mut first = true;
        let mut worker = Worker::start_with(store, move |store, session| {
            if first {
                first = false;
                anyhow::bail!("Injected recovery write failure");
            }
            store.persist_snapshot(session)
        })
        .unwrap();
        document.insert(" newest", false);
        worker.submit(std::slice::from_ref(&document)).unwrap();
        assert!(
            settle(&mut worker)
                .unwrap_err()
                .to_string()
                .contains("Injected")
        );
        assert_eq!(read_session(&path).documents[0].text, "last good");
        assert!(worker.submit(std::slice::from_ref(&document)).unwrap());
        settle(&mut worker).unwrap();
        assert_eq!(read_session(&path).documents[0].text, "last good newest");
        // An unexpected error exit retains the journal and releases its lock.
        drop(worker);
        let other = Recovery::new(dir.path().into()).unwrap();
        assert_eq!(other.restore().unwrap().0[0].text, "last good newest");
    }

    #[test]
    fn shutdown_serializes_final_snapshot_or_cleanup_after_pending_writes() {
        for preserve in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = Recovery::new(dir.path().into()).unwrap();
            let path = store.path.clone();
            let lock = store.lock_path.clone();
            let (started, began) = mpsc::sync_channel(1);
            let (release, gate) = mpsc::sync_channel(1);
            let mut worker = Worker::start_with(store, move |store, session| {
                started.send(())?;
                gate.recv_timeout(Duration::from_secs(5))?;
                store.persist_snapshot(session)
            })
            .unwrap();
            let mut document = Document::default();
            document.insert("older", false);
            worker.submit(std::slice::from_ref(&document)).unwrap();
            began.recv_timeout(Duration::from_secs(5)).unwrap();
            document.insert(" newest", false);
            let shutdown = std::thread::spawn(move || {
                if preserve {
                    worker.preserve(&[document])
                } else {
                    worker.finish()
                }
            });
            release.send(()).unwrap();
            shutdown.join().unwrap().unwrap();
            if preserve {
                let other = Recovery::new(dir.path().into()).unwrap();
                assert_eq!(other.restore().unwrap().0[0].text, "older newest");
            } else {
                assert!(
                    !path.exists(),
                    "A late write must not resurrect discarded work"
                );
                assert!(!lock.exists());
            }
        }
    }

    #[test]
    fn live_sessions_are_skipped_and_crashes_restore_unsaved_text() {
        let dir = tempfile::tempdir().unwrap();
        let a = Recovery::new(dir.path().into()).unwrap();
        let mut d = Document::default();
        d.insert("unsaved🙂", false);
        a.persist(&[d]).unwrap();
        let b = Recovery::new(dir.path().into()).unwrap();
        assert!(b.restore().unwrap().0.is_empty());
        drop(a);
        let (docs, paths) = b.restore().unwrap();
        assert_eq!(docs[0].text.to_string(), "unsaved🙂");
        assert!(docs[0].dirty());
        b.persist(&docs).unwrap();
        Recovery::consume(&paths);
        b.finish().unwrap();
    }

    #[test]
    fn recovering_a_deleted_file_buffer_preserves_dirty_state_and_real_undo_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, "original").unwrap();
        let a = Recovery::new(dir.path().join("state")).unwrap();
        let mut d = Document::open(&file).unwrap();
        d.select_all();
        d.insert("", false);
        a.persist(&[d]).unwrap();
        drop(a);
        let b = Recovery::new(dir.path().join("state")).unwrap();
        let (mut docs, old) = b.restore().unwrap();
        assert!(docs[0].is_empty());
        assert!(docs[0].dirty());
        docs[0].undo();
        assert_eq!(docs[0].text.to_string(), "original");
        assert!(!docs[0].dirty());
        b.persist(&docs).unwrap();
        Recovery::consume(&old);
        b.finish().unwrap();
    }
}
