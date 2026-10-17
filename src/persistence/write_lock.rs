//! Process-local resource locks, like VS Code's disk provider resource map.
//! Workers wait here; no lock file is created and the UI never waits on this.
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Condvar, LazyLock, Mutex},
    time::{Duration, Instant},
};

const MAX_ACTIVE: usize = 128;
static LOCKS: LazyLock<(Mutex<HashSet<PathBuf>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashSet::new()), Condvar::new()));

/// Held across preparation, authorization and replacement, shared by document
/// saves and settings patches. Other processes are external writers: their
/// changes are checked through the original disk baseline, not a sidecar lock.
pub(crate) struct WriteLock {
    key: PathBuf,
}
impl WriteLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        Self::acquire_with_timeout(path, Duration::from_secs(1))
    }
    fn acquire_with_timeout(path: &Path, timeout: Duration) -> Result<Self> {
        let key = super::destination(path)?;
        // Match the provider's case-insensitive resource comparison on these
        // platforms. Lossy non-Unicode collisions only serialize extra work.
        #[cfg(any(windows, target_os = "macos"))]
        let key = PathBuf::from(key.to_string_lossy().to_lowercase());
        let deadline = Instant::now() + timeout;
        let (mutex, changed) = &*LOCKS;
        let mut held = mutex.lock().unwrap_or_else(|error| error.into_inner());
        while held.contains(&key) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(
                !remaining.is_zero(),
                "Persistence resource lock unavailable"
            );
            (held, _) = changed
                .wait_timeout(held, remaining)
                .unwrap_or_else(|error| error.into_inner());
        }
        ensure!(
            held.len() < MAX_ACTIVE,
            "Persistence resource lock budget exceeded"
        );
        held.try_reserve(1)
            .context("Persistence resource lock allocation refused")?;
        held.insert(key.clone());
        Ok(Self { key })
    }
    #[cfg(test)]
    pub(crate) fn try_acquire(path: &Path) -> Result<Self> {
        Self::acquire_with_timeout(path, Duration::ZERO)
    }
}
impl Drop for WriteLock {
    fn drop(&mut self) {
        let (mutex, changed) = &*LOCKS;
        mutex
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.key);
        changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::mpsc, thread};

    #[test]
    fn same_resource_serializes_different_resources_remain_independent_and_no_files_appear() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.txt");
        let b = root.path().join("b.txt");
        let first = WriteLock::acquire(&a).unwrap();
        assert!(WriteLock::try_acquire(&a).is_err());
        let other = WriteLock::try_acquire(&b).unwrap();
        drop(other);
        let (entered, waiting) = mpsc::sync_channel(1);
        let (acquired, result) = mpsc::sync_channel(1);
        let path = a.clone();
        let worker = thread::spawn(move || {
            entered.send(()).unwrap();
            let lock = WriteLock::acquire(&path).unwrap();
            acquired.send(()).unwrap();
            drop(lock);
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result.try_recv().is_err());
        drop(first);
        result.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        drop(WriteLock::try_acquire(&a).unwrap());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn unwinding_releases_the_original_resource_without_persisted_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recover.txt");
        assert!(
            std::panic::catch_unwind(|| {
                let _lock = WriteLock::acquire(&path).unwrap();
                panic!("worker failure");
            })
            .is_err()
        );
        drop(WriteLock::try_acquire(&path).unwrap());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_parent_aliases_share_one_resource_lock() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        let alias = root.path().join("alias");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let lock = WriteLock::acquire(&real.join("file.txt")).unwrap();
        assert!(WriteLock::try_acquire(&alias.join("file.txt")).is_err());
        drop(lock);
        drop(WriteLock::try_acquire(&alias.join("file.txt")).unwrap());
        assert_eq!(fs::read_dir(&real).unwrap().count(), 0);
    }
}
