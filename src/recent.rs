//! Bounded recent-file metadata; disk merges happen in one background worker.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub const LIMIT: usize = 100;
const MAX_BYTES: usize = 512 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub opened: u64,
}
#[derive(Deserialize, Serialize)]
struct Saved {
    schema: u32,
    files: Vec<Entry>,
}
fn valid(entry: &Entry) -> bool {
    entry.path.is_absolute() && entry.path.as_os_str().len() <= 4096
}
fn merge(files: &mut Vec<Entry>, updates: &[Entry]) {
    for update in updates {
        if !valid(update) {
            continue;
        }
        if let Some(old) = files.iter_mut().find(|old| old.path == update.path) {
            old.opened = old.opened.max(update.opened);
        } else {
            files.push(update.clone());
        }
    }
    files.sort_by(|a, b| b.opened.cmp(&a.opened).then(a.path.cmp(&b.path)));
    files.truncate(LIMIT);
}
pub fn read(path: &Path) -> Result<Vec<Entry>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("Recent-file state must be a regular file")
        }
        _ => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        bail!("Recent-file state exceeds 512 KiB");
    }
    let saved: Saved = serde_json::from_slice(&bytes).context("Invalid recent-file state")?;
    if saved.schema != 1 || saved.files.len() > LIMIT || saved.files.iter().any(|file| !valid(file))
    {
        bail!("Invalid recent-file metadata or entry budget");
    }
    let mut files = Vec::new();
    merge(&mut files, &saved.files);
    Ok(files)
}
fn persist(path: &Path, updates: &[Entry]) -> Result<Vec<Entry>> {
    let parent = path.parent().context("Missing recent state directory")?;
    fs::create_dir_all(parent)?;
    let lock_path = parent.join("recent-files.lock");
    if let Ok(meta) = fs::symlink_metadata(&lock_path)
        && !meta.file_type().is_file()
    {
        bail!("Recent-file lock must be a regular file");
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Recent-file state lock unavailable: {error}"
                ));
            }
        }
    }
    let mut files = read(path)?;
    merge(&mut files, updates);
    publish(path, &files)?;
    Ok(files)
}
fn publish(path: &Path, files: &[Entry]) -> Result<()> {
    let parent = path.parent().context("Missing recent state directory")?;
    let bytes = serde_json::to_vec(&Saved {
        schema: 1,
        files: files.to_vec(),
    })?;
    if bytes.len() > MAX_BYTES {
        bail!("Recent-file state exceeds 512 KiB");
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}
struct Pending {
    receiver: Receiver<Result<Vec<Entry>, String>>,
    updates: Vec<Entry>,
}
#[derive(Default)]
pub struct State {
    pub files: Vec<Entry>,
    path: Option<PathBuf>,
    queued: Vec<Entry>,
    pending: Option<Pending>,
    pub error: Option<String>,
    clock: u64,
}
impl State {
    pub fn configure(&mut self, root: Option<&Path>) {
        if self.path.is_some() || self.pending.is_some() {
            return;
        }
        self.path = root.map(|root| root.join("state/recent-files.json"));
        self.start();
    }
    fn start(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.pending.is_some() {
            return;
        }
        let updates = std::mem::take(&mut self.queued);
        let input = updates.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = if input.is_empty() {
                read(&path)
            } else {
                persist(&path, &input)
            };
            let _ = sender.send(result.map_err(|error| format!("{error:#}")));
        });
        self.pending = Some(Pending { receiver, updates });
    }
    pub fn touch(&mut self, path: PathBuf) {
        self.clock = self.clock.saturating_add(1).max(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros()
                .min(u64::MAX as u128) as u64,
        );
        let entry = Entry {
            path,
            opened: self.clock,
        };
        merge(&mut self.files, std::slice::from_ref(&entry));
        merge(&mut self.queued, &[entry]);
        self.start();
    }
    pub fn poll(&mut self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Recent-file worker stopped".into()),
        };
        let pending = self.pending.take().unwrap();
        match result {
            Ok(mut files) => {
                merge(&mut files, &self.files);
                self.clock = self.clock.max(files.first().map_or(0, |file| file.opened));
                self.files = files;
                self.error = None;
                if !self.queued.is_empty() {
                    self.start();
                }
            }
            Err(error) => {
                merge(&mut self.queued, &pending.updates);
                self.error = Some(error);
            }
        }
        true
    }
    /// Normal shutdown waits for already queued metadata, without performing I/O on input frames.
    pub fn finish(&mut self) -> Result<()> {
        if self.pending.is_none() && !self.queued.is_empty() {
            self.start();
        }
        while self.pending.is_some() {
            self.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        if let Some(error) = &self.error {
            bail!("Recent files were not persisted: {error}");
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(path: &Path, stamp: u64) -> Entry {
        Entry {
            path: path.to_owned(),
            opened: stamp,
        }
    }
    #[test]
    fn concurrent_merges_do_not_replace_newer_entries_and_have_a_fixed_budget() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state/recent-files.json");
        let handles: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                let file = root.path().join(format!("{n}.txt"));
                std::thread::spawn(move || {
                    let update = entry(&file, n);
                    let result = persist(&path, std::slice::from_ref(&update));
                    (update, result)
                })
            })
            .collect();
        let mut retained = Vec::new();
        for handle in handles {
            let (update, result) = handle.join().unwrap();
            if let Err(error) = result {
                // The production deadline is intentional. Slow filesystems may
                // reject contention; callers retain that batch for another action.
                assert!(
                    error
                        .to_string()
                        .starts_with("Recent-file state lock unavailable:"),
                    "{error:#}"
                );
                retained.push(update);
            }
        }
        if !retained.is_empty() {
            persist(&path, &retained).unwrap();
        }
        assert_eq!(read(&path).unwrap().len(), 8);
        let file = root.path().join("7.txt");
        persist(&path, &[entry(&file, 1)]).unwrap();
        assert_eq!(read(&path).unwrap()[0].opened, 7);
        let updates: Vec<_> = (0..150)
            .map(|n| entry(&root.path().join(format!("file{n}")), n + 20))
            .collect();
        assert_eq!(persist(&path, &updates).unwrap().len(), LIMIT);
    }
    #[test]
    fn malformed_oversized_or_unpublishable_state_is_retained() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recent-files.json");
        for bytes in [
            b"malformed".to_vec(),
            vec![b' '; MAX_BYTES + 1],
            br#"{"schema":2,"files":[]}"#.to_vec(),
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(persist(&path, &[entry(&root.path().join("x"), 1)]).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "retained").unwrap();
        assert!(persist(&path, &[entry(&root.path().join("x"), 1)]).is_err());
        assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "retained");
    }
    #[test]
    fn metadata_limits_and_failed_atomic_publication_preserve_existing_data() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recent-files.json");
        for files in [
            vec![entry(Path::new("relative"), 1)],
            vec![entry(&root.path().join("x"), 1); LIMIT + 1],
        ] {
            let bytes = serde_json::to_vec(&Saved { schema: 1, files }).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(read(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("original"), "retained").unwrap();
        assert!(publish(&path, &[entry(&root.path().join("x"), 1)]).is_err());
        assert_eq!(
            fs::read_to_string(path.join("original")).unwrap(),
            "retained"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn queued_updates_survive_outstanding_initial_load_and_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let mut state = State::default();
        state.configure(Some(root.path()));
        for n in 0..20 {
            state.touch(root.path().join(format!("file{n}")));
        }
        state.finish().unwrap();
        let saved = read(&root.path().join("state/recent-files.json")).unwrap();
        assert_eq!(saved.len(), 20);
        assert_eq!(saved[0].path, root.path().join("file19"));
        assert!(!root.path().join("imports/state").exists());
    }
    #[test]
    fn lock_deadline_retains_pending_updates_for_a_later_shutdown_retry() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("state");
        fs::create_dir_all(&directory).unwrap();
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("recent-files.lock"))
            .unwrap();
        lock.lock().unwrap();
        let mut state = State::default();
        state.configure(Some(root.path()));
        let file = root.path().join("retained.cpp");
        state.touch(file.clone());
        assert!(state.finish().is_err());
        assert_eq!(state.files[0].path, file);
        assert_eq!(state.queued[0].path, file);
        assert!(state.pending.is_none());
        assert!(!directory.join("recent-files.json").exists());
        drop(lock);
        state.finish().unwrap();
        assert!(state.queued.is_empty());
        assert!(state.error.is_none());
        assert_eq!(
            read(&directory.join("recent-files.json")).unwrap()[0].path,
            file
        );
    }
}
