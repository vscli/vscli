//! Bounded native storage for extension Mementos. All calls belong on workers.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_STATE_BYTES: usize = 256 * 1024;
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_KEYS: usize = 1024;

struct StateLock<'a>(&'a File);
impl Drop for StateLock<'_> {
    fn drop(&mut self) {
        // Closing this descriptor alone can leave a Unix lock alive in a child
        // forked before exec. Explicitly release the shared open-description
        // lock on every return/unwind while this worker still owns its file.
        let _ = self.0.unlock();
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    Global,
    Workspace,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    schema: u32,
    values: Map<String, Value>,
}

fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn owner_valid(owner: &str) -> bool {
    owner.len() <= 201
        && owner.split('.').count() == 2
        && owner.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
}
fn directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => bail!("Extension state directory must be an ordinary directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => directory(path),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}
fn open_regular(path: &Path, create: bool) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => bail!("Extension state must be a regular file"),
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
        options
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .mode(0o600);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Extension state must be a regular file");
    }
    Ok(file)
}
fn validate_key(key: &str) -> Result<()> {
    if key.len() > 1024 {
        bail!("Extension state keys exceed 1 KiB");
    }
    Ok(())
}
fn validate_values(values: &Map<String, Value>) -> Result<()> {
    if values.len() > MAX_KEYS {
        bail!("Extension state exceeds 1,024 keys");
    }
    for key in values.keys() {
        validate_key(key)?;
    }
    validate_nodes(values.values())
}
fn validate_nodes<'a>(values: impl Iterator<Item = &'a Value>) -> Result<()> {
    let mut pending: Vec<_> = values.map(|value| (value, 0)).collect();
    let mut nodes = 0;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        if nodes > 10_000 || depth > 32 {
            bail!("Extension state exceeds 10,000 values or 32 nesting levels");
        }
        let mut push = |child| -> Result<()> {
            if pending.len() + nodes >= 10_000 {
                bail!("Extension state exceeds 10,000 values");
            }
            pending.push((child, depth + 1));
            Ok(())
        };
        match value {
            Value::Array(items) => {
                for item in items {
                    push(item)?;
                }
            }
            Value::Object(items) => {
                for item in items.values() {
                    push(item)?;
                }
            }
            _ => {}
        }
        if pending.len() + nodes > 10_000 {
            bail!("Extension state exceeds 10,000 values");
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    workspace: String,
}
impl Store {
    pub fn new(config_root: &Path, workspace: &Path) -> Result<Self> {
        // Only the explicitly configured root may itself be a symlink. Generated
        // storage components and payload/lock files must not redirect writes.
        fs::create_dir_all(config_root)?;
        let config_root = fs::canonicalize(config_root)?;
        let state = config_root.join("state");
        directory(&state)?;
        let root = state.join("extension-storage");
        directory(&root)?;
        let workspace = fs::canonicalize(workspace)?;
        Ok(Self {
            root,
            workspace: digest(workspace.as_os_str().as_encoded_bytes()),
        })
    }
    fn path(&self, owner: &str, scope: Scope) -> Result<PathBuf> {
        if !owner_valid(owner) {
            bail!("Invalid extension state owner");
        }
        let directory_path = self
            .root
            .join(digest(owner.to_ascii_lowercase().as_bytes()));
        directory(&directory_path)?;
        let name = match scope {
            Scope::Global => "global".to_owned(),
            Scope::Workspace => format!("workspace-{}", self.workspace),
        };
        Ok(directory_path.join(format!("{name}.json")))
    }
    pub fn read(&self, owner: &str, scope: Scope) -> Result<Map<String, Value>> {
        Self::read_path(&self.path(owner, scope)?)
    }
    fn read_path(path: &Path) -> Result<Map<String, Value>> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(error) => return Err(error.into()),
            _ => {}
        }
        let mut bytes = Vec::new();
        open_regular(path, false)?
            .take(MAX_STATE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_STATE_BYTES {
            bail!("Extension state exceeds 256 KiB");
        }
        let saved: Saved = serde_json::from_slice(&bytes).context("Invalid extension state")?;
        if saved.schema != 1 {
            bail!("Unsupported extension state schema");
        }
        validate_values(&saved.values)?;
        Ok(saved.values)
    }
    /// Apply one key patch to the latest committed state under a worker-side lock.
    /// None removes the key; Some(Null) stores JSON null.
    pub fn update(
        &self,
        owner: &str,
        scope: Scope,
        key: &str,
        value: Option<Value>,
    ) -> Result<Map<String, Value>> {
        self.update_with(owner, scope, key, value, || Ok(()))
    }
    pub(crate) fn update_with(
        &self,
        owner: &str,
        scope: Scope,
        key: &str,
        value: Option<Value>,
        before_commit: impl FnOnce() -> Result<()>,
    ) -> Result<Map<String, Value>> {
        validate_key(key)?;
        if let Some(value) = &value {
            validate_nodes(std::iter::once(value))?;
        }
        if let Some(value) = &value
            && serde_json::to_vec(value)?.len() > MAX_VALUE_BYTES
        {
            bail!("Extension state value exceeds 64 KiB");
        }
        let path = self.path(owner, scope)?;
        let lock = open_regular(&path.with_extension("lock"), true)?;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => bail!("Extension state lock unavailable: {error}"),
            }
        }
        let _lock = StateLock(&lock);
        let mut values = Self::read_path(&path)?;
        if let Some(value) = value {
            values.insert(key.to_owned(), value);
        } else {
            values.remove(key);
        }
        validate_values(&values)?;
        let bytes = serde_json::to_vec(&Saved {
            schema: 1,
            values: values.clone(),
        })?;
        if bytes.len() > MAX_STATE_BYTES {
            bail!("Extension state exceeds 256 KiB");
        }
        let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        before_commit()?;
        temporary.persist(&path).map_err(|error| error.error)?;
        #[cfg(unix)]
        File::open(path.parent().unwrap())?.sync_all()?;
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(&directory.path().join("config"), directory.path()).unwrap();
        (directory, store)
    }
    #[test]
    fn state_isolated_by_owner_workspace_and_native_config_root() {
        let (directory, store) = fixture();
        store
            .update("sample.one", Scope::Global, "count", Some(json!(7)))
            .unwrap();
        store
            .update("sample.one", Scope::Workspace, "count", Some(json!(3)))
            .unwrap();
        assert_eq!(store.read("sample.one", Scope::Global).unwrap()["count"], 7);
        assert!(store.read("sample.two", Scope::Global).unwrap().is_empty());
        let workspace = directory.path().join("other");
        fs::create_dir(&workspace).unwrap();
        let other = Store::new(&directory.path().join("config"), &workspace).unwrap();
        assert_eq!(other.read("sample.one", Scope::Global).unwrap()["count"], 7);
        assert!(
            other
                .read("sample.one", Scope::Workspace)
                .unwrap()
                .is_empty()
        );
        let isolated = Store::new(&directory.path().join("isolated"), directory.path()).unwrap();
        assert!(
            isolated
                .read("sample.one", Scope::Global)
                .unwrap()
                .is_empty()
        );
        assert!(store.read("../escape", Scope::Global).is_err());
        let values = store
            .update("sample.one", Scope::Global, "__proto__", Some(Value::Null))
            .unwrap();
        assert_eq!(values.get("__proto__"), Some(&Value::Null));
        let values = store
            .update("sample.one", Scope::Global, "count", None)
            .unwrap();
        assert!(!values.contains_key("count"));
        assert!(values.contains_key("__proto__"));
    }
    #[test]
    fn malformed_unknown_oversized_and_precommit_failure_preserve_prior_bytes() {
        let (_directory, store) = fixture();
        let path = store.path("sample.one", Scope::Global).unwrap();
        for bytes in [
            b"broken".to_vec(),
            br#"{"schema":1,"values":{},"unknown":true}"#.to_vec(),
            vec![b' '; MAX_STATE_BYTES + 1],
        ] {
            fs::write(&path, &bytes).unwrap();
            assert!(store.read("sample.one", Scope::Global).is_err());
            assert!(
                store
                    .update("sample.one", Scope::Global, "new", Some(json!(1)))
                    .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_file(&path).unwrap();
        store
            .update("sample.one", Scope::Global, "old", Some(json!(1)))
            .unwrap();
        let old = fs::read(&path).unwrap();
        assert!(
            store
                .update_with(
                    "sample.one",
                    Scope::Global,
                    "new",
                    Some(json!(2)),
                    || bail!("injected before atomic commit")
                )
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), old);
    }
    #[test]
    fn value_tree_key_and_whole_state_budgets_are_enforced() {
        let (_directory, store) = fixture();
        assert!(
            store
                .update(
                    "sample.one",
                    Scope::Global,
                    &"k".repeat(1025),
                    Some(json!(0))
                )
                .is_err()
        );
        assert!(
            store
                .update(
                    "sample.one",
                    Scope::Global,
                    "value",
                    Some(json!("x".repeat(MAX_VALUE_BYTES)))
                )
                .is_err()
        );
        let mut deep = json!(0);
        for _ in 0..34 {
            deep = json!([deep]);
        }
        assert!(
            store
                .update("sample.one", Scope::Global, "value", Some(deep))
                .is_err()
        );
        assert!(
            store
                .update(
                    "sample.one",
                    Scope::Global,
                    "value",
                    Some(json!(vec![0; 10_001]))
                )
                .is_err()
        );
        for index in 0..4 {
            store
                .update(
                    "sample.one",
                    Scope::Global,
                    &format!("key{index}"),
                    Some(json!("x".repeat(60 * 1024))),
                )
                .unwrap();
        }
        let before = store.read("sample.one", Scope::Global).unwrap();
        assert!(
            store
                .update(
                    "sample.one",
                    Scope::Global,
                    "overflow",
                    Some(json!("x".repeat(60 * 1024)))
                )
                .is_err()
        );
        assert_eq!(store.read("sample.one", Scope::Global).unwrap(), before);
    }
    #[test]
    fn concurrent_instances_merge_patches_and_lock_timeout_remains_retryable() {
        let (_directory, store) = fixture();
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || {
                    (
                        index,
                        store.update(
                            "sample.one",
                            Scope::Global,
                            &format!("key{index}"),
                            Some(json!(index)),
                        ),
                    )
                })
            })
            .collect();
        for worker in workers {
            let (index, result) = worker.join().unwrap();
            if result.is_err() {
                store
                    .update(
                        "sample.one",
                        Scope::Global,
                        &format!("key{index}"),
                        Some(json!(index)),
                    )
                    .unwrap();
            }
        }
        let values = store.read("sample.one", Scope::Global).unwrap();
        assert_eq!(values.len(), 8);
        for index in 0..8 {
            assert_eq!(values[&format!("key{index}")], index);
        }
        let lock = open_regular(
            &store
                .path("sample.one", Scope::Global)
                .unwrap()
                .with_extension("lock"),
            true,
        )
        .unwrap();
        lock.try_lock().unwrap();
        assert!(
            store
                .update("sample.one", Scope::Global, "retry", Some(json!(9)))
                .is_err()
        );
        assert_eq!(store.read("sample.one", Scope::Global).unwrap(), values);
        drop(lock);
        store
            .update("sample.one", Scope::Global, "retry", Some(json!(9)))
            .unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn update_releases_inherited_lock_before_child_exit_on_success_and_failure() {
        use std::os::fd::{FromRawFd, OwnedFd};

        // A child forked while an update holds the lock retains the same open
        // file description until exec/exit. Keep it alive past the update using
        // only async-signal-safe operations after fork, like a delayed pre_exec.
        struct ForkGate {
            release: File,
            child: libc::pid_t,
        }
        impl ForkGate {
            fn new() -> Result<Self> {
                let mut pipe = [-1; 2];
                if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                // SAFETY: pipe returned two newly owned descriptors.
                let read = unsafe { OwnedFd::from_raw_fd(pipe[0]) };
                let write = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
                let child = unsafe { libc::fork() };
                if child == -1 {
                    return Err(std::io::Error::last_os_error().into());
                }
                if child == 0 {
                    // Do not invoke Rust destructors, allocation, or test APIs
                    // in the child of the multithreaded test runner.
                    unsafe {
                        libc::close(pipe[1]);
                        let mut byte = 0u8;
                        let mut result;
                        loop {
                            result = libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
                            if result >= 0 {
                                break;
                            }
                        }
                        libc::_exit(if result == 1 { 0 } else { 1 });
                    }
                }
                drop(read);
                Ok(Self {
                    release: write.into(),
                    child,
                })
            }
        }
        impl Drop for ForkGate {
            fn drop(&mut self) {
                // Release and reap even if an assertion unwinds.
                let _ = self.release.write_all(&[1]);
                loop {
                    let result = unsafe { libc::waitpid(self.child, std::ptr::null_mut(), 0) };
                    if result >= 0
                        || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                    {
                        break;
                    }
                }
            }
        }

        for fail in [false, true] {
            let (_directory, store) = fixture();
            store
                .update("sample.one", Scope::Global, "old", Some(json!(1)))
                .unwrap();
            let path = store.path("sample.one", Scope::Global).unwrap();
            let before = fs::read(&path).unwrap();
            let mut child = None;
            let result =
                store.update_with("sample.one", Scope::Global, "new", Some(json!(2)), || {
                    child = Some(ForkGate::new()?);
                    if fail {
                        bail!("injected before atomic commit");
                    }
                    Ok(())
                });
            assert_eq!(result.is_err(), fail);
            assert!(child.is_some(), "the update must reach its locked callback");
            let lock = open_regular(&path.with_extension("lock"), true).unwrap();
            assert!(
                lock.try_lock().is_ok(),
                "update must release its lock before the gated child exits (failure={fail})"
            );
            lock.unlock().unwrap();
            if fail {
                assert_eq!(fs::read(&path).unwrap(), before);
            } else {
                assert_eq!(store.read("sample.one", Scope::Global).unwrap()["new"], 2);
            }
            drop(child);
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_fifo_payloads_or_lock_files_are_rejected_before_open() {
        use std::os::unix::fs::symlink;
        let (directory, store) = fixture();
        let path = store.path("sample.one", Scope::Global).unwrap();
        let outside = directory.path().join("source.json");
        fs::write(&outside, br#"{"schema":1,"values":{"original":true}}"#).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(store.read("sample.one", Scope::Global).is_err());
        assert!(
            store
                .update("sample.one", Scope::Global, "new", Some(json!(1)))
                .is_err()
        );
        assert!(fs::read_to_string(&outside).unwrap().contains("original"));
        fs::remove_file(&path).unwrap();
        for target in [&path, &path.with_extension("lock")] {
            if target.exists() {
                fs::remove_file(target).unwrap();
            }
            let name = std::ffi::CString::new(target.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            assert!(
                store
                    .update("sample.one", Scope::Global, "new", Some(json!(1)))
                    .is_err()
            );
            fs::remove_file(target).unwrap();
        }
    }
}
