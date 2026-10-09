//! User-owned execution grants. Call disk operations from startup or a worker.
use super::{Preferences, Scope};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Paths {
    pub global: Option<PathBuf>,
    pub workspace: Option<PathBuf>,
}
impl Paths {
    pub fn new(config_root: Option<&Path>, workspace: &Path) -> Result<Self> {
        let workspace = crate::document::absolute_path(workspace)?;
        if !workspace.is_dir() {
            bail!("Extension workspace must be a directory");
        }
        let uri = crate::lsp::file_uri(&workspace)?;
        let key = format!("{:x}", Sha256::digest(uri.as_bytes()));
        Ok(Self {
            global: config_root.map(|root| root.join("extensions-enabled.json")),
            workspace: config_root.map(|root| {
                root.join("extension-workspaces")
                    .join(format!("{key}.json"))
            }),
        })
    }
    pub fn path(&self, scope: Scope) -> Result<&Path> {
        match scope {
            Scope::Global => self.global.as_deref(),
            Scope::Workspace => self.workspace.as_deref(),
        }
        .context("Native user configuration is unavailable; remembered enablement was not changed")
    }
    pub fn read(&self) -> Result<(Preferences, Preferences)> {
        let read_optional = |path: &Option<PathBuf>| -> Result<Preferences> {
            path.as_deref()
                .map(read)
                .transpose()
                .map(|value| value.unwrap_or_default())
        };
        Ok((
            read_optional(&self.global)?,
            read_optional(&self.workspace)?,
        ))
    }
}

pub fn read(path: &Path) -> Result<Preferences> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Preferences::default());
        }
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("Extension grant state must be a regular file")
        }
        _ => {}
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        bail!("Extension grant state exceeds 1 MiB");
    }
    let preferences: Preferences = serde_json::from_slice(&bytes)
        .context("Invalid extension grant state; previous grants retained")?;
    preferences.validate()?;
    Ok(preferences)
}

struct Lease(File);
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn lease(path: &Path) -> Result<Lease> {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    let lock_path = PathBuf::from(name);
    if let Ok(metadata) = fs::symlink_metadata(&lock_path)
        && !metadata.file_type().is_file()
    {
        bail!("Extension grant lock must be a regular file");
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Lease(file)),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Extension grant lease unavailable: {error}"
                ));
            }
        }
    }
}

/// Merge one explicitly requested grant under a bounded cross-process lease.
/// A caller must validate the installed package before invoking this operation.
pub fn change(path: &Path, id: &str, enabled: bool) -> Result<Preferences> {
    let id = id.to_ascii_lowercase();
    super::validate_id(&id)?;
    let parent = path.parent().context("Missing extension grant directory")?;
    fs::create_dir_all(parent)?;
    let _lease = lease(path)?;
    let mut preferences = read(path)?;
    preferences.set(&id, enabled)?;
    let mut bytes = serde_json::to_vec_pretty(&preferences)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_BYTES {
        bail!("Extension grant state exceeds 1 MiB");
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .context("Cannot atomically publish extension grant state")?;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_paths_are_user_owned_and_repository_files_cannot_grant_execution() {
        let root = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".vscli")).unwrap();
        fs::write(
            root.path().join(".vscli/extensions-enabled.json"),
            r#"{"schema":1,"extensions":{"test.malicious":true}}"#,
        )
        .unwrap();
        let paths = Paths::new(Some(config.path()), root.path()).unwrap();
        assert!(paths.global.as_ref().unwrap().starts_with(config.path()));
        assert!(paths.workspace.as_ref().unwrap().starts_with(config.path()));
        let (global, workspace) = paths.read().unwrap();
        assert!(Preferences::effective(&global, &workspace).is_empty());
        change(paths.path(Scope::Workspace).unwrap(), "test.safe", true).unwrap();
        assert_eq!(fs::read_dir(root.path().join(".vscli")).unwrap().count(), 1);
        let other = tempfile::tempdir().unwrap();
        assert_ne!(
            paths.workspace,
            Paths::new(Some(config.path()), other.path())
                .unwrap()
                .workspace
        );
        assert!(
            Paths::new(None, root.path())
                .unwrap()
                .path(Scope::Workspace)
                .is_err()
        );
    }
    #[test]
    fn concurrent_patches_merge_without_losing_unrelated_grants() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("grants.json");
        change(&path, "test.original", true).unwrap();
        let gate = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|i| {
                let path = path.clone();
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        match change(&path, &format!("test.p{i}"), i != 2) {
                            Ok(_) => return,
                            Err(error)
                                if error
                                    .to_string()
                                    .starts_with("Extension grant lease unavailable:")
                                    && Instant::now() < deadline => {}
                            Err(error) => panic!("{error:#}"),
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let result = read(&path).unwrap();
        assert_eq!(result.extensions.len(), 5);
        assert!(result.extensions["test.original"]);
        for i in 0..4 {
            assert_eq!(result.extensions[&format!("test.p{i}")], i != 2);
        }
    }
    #[test]
    fn malformed_or_held_state_never_overwrites_previous_bytes_or_grants() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("grants.json");
        change(&path, "test.original", true).unwrap();
        let original = fs::read(&path).unwrap();
        let held = lease(&path).unwrap();
        assert!(change(&path, "test.other", true).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        drop(held);
        fs::write(&path, "malformed").unwrap();
        assert!(change(&path, "test.other", true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"malformed");
        fs::write(&path, original).unwrap();
        change(&path, "test.other", true).unwrap();
        assert!(read(&path).unwrap().extensions["test.original"]);
    }
    #[cfg(unix)]
    #[test]
    fn fifo_and_symlink_state_reject_before_opening() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let started = Instant::now();
        assert!(read(&fifo).is_err());
        assert!(change(&fifo, "test.other", true).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        let regular = root.path().join("regular");
        change(&regular, "test.original", true).unwrap();
        let bytes = fs::read(&regular).unwrap();
        let alias = root.path().join("alias");
        symlink(&regular, &alias).unwrap();
        assert!(change(&alias, "test.other", true).is_err());
        assert_eq!(fs::read(&regular).unwrap(), bytes);
    }
}
