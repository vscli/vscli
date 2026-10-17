//! Immutable local-package snapshots, published together by one registry replacement.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_LOCAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_LOCAL_FILE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_LOCAL_ENTRIES: usize = 1_000_000;
const MAX_PATH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct DirectorySource {
    pub path: PathBuf,
    pub id: String,
    pub version: String,
    pub manifest_sha256: String,
}
impl Store {
    /// Snapshot local packages without starting code. Call from a background worker.
    /// Any failure leaves the previous registry and its installed generations intact.
    pub fn import_directories(
        &self,
        sources: &[DirectorySource],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize, &str),
    ) -> Result<Vec<Installed>> {
        if sources.len() > 2048 {
            bail!("Local import exceeds 2048 packages");
        }
        let _lock = self.lock()?;
        let mut registry = self.registry()?;
        let stage = tempfile::Builder::new()
            .prefix(".import-")
            .tempdir_in(&self.root)?;
        let mut packages = Vec::new();
        let mut seen = HashSet::new();
        let mut budget = Budget {
            bytes: 0,
            entries: 0,
            path_bytes: 0,
            cancel,
        };
        for (index, source) in sources.iter().enumerate() {
            check(cancel)?;
            validate_id(&source.id)?;
            if !seen.insert(&source.id) {
                bail!("Duplicate local package ID: {}", source.id);
            }
            let root =
                fs::canonicalize(&source.path).context("Local extension source is missing")?;
            if !root.is_dir() || stage.path().starts_with(&root) || root.starts_with(stage.path()) {
                bail!("Local package and snapshot directories must be separate");
            }
            let manifest = bounded_read(&root.join("package.json"), MAX_MANIFEST)?;
            if format!("{:x}", Sha256::digest(&manifest)) != source.manifest_sha256 {
                bail!(
                    "Local package manifest changed after preview: {}",
                    source.id
                );
            }
            let directory = format!("package-{}", uuid::Uuid::new_v4());
            let destination = stage.path().join(&directory).join("extension");
            fs::create_dir_all(&destination)?;
            let mut digest = Sha256::new();
            copy_tree(&root, &root, &destination, 0, &mut budget, &mut digest)?;
            let after = bounded_read(&root.join("package.json"), MAX_MANIFEST)?;
            if after != manifest {
                bail!("Local package changed while copying: {}", source.id);
            }
            let snapshot_manifest = read_manifest(&destination)?;
            let (id, version) = identity(&snapshot_manifest)?;
            if id != source.id || version != source.version {
                bail!("Local package snapshot identity/version differs from preview");
            }
            let package = Package {
                id: id.clone(),
                version,
                directory,
                source: format!("local-directory:{}", root.display()),
                sha256: format!("{:x}", digest.finalize()),
            };
            let previous = registry
                .packages
                .get(&id)
                .map(|entry| entry.current.clone());
            registry.packages.insert(
                id,
                Entry {
                    current: package.clone(),
                    previous,
                },
            );
            packages.push(package);
            progress(index + 1, sources.len(), &source.id);
        }
        check(cancel)?;
        let mut published = Vec::new();
        let result = (|| -> Result<()> {
            for package in &packages {
                let destination = self.root.join(&package.directory);
                fs::rename(stage.path().join(&package.directory), &destination)?;
                published.push(destination);
            }
            check(cancel)?;
            self.write_registry(&registry)
        })();
        if let Err(error) = result {
            for path in published {
                let _ = fs::remove_dir_all(path);
            }
            return Err(error);
        }
        packages
            .iter()
            .map(|package| self.installed(package))
            .collect()
    }
}
struct Budget<'a> {
    bytes: u64,
    entries: usize,
    path_bytes: usize,
    cancel: &'a AtomicBool,
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        bail!("Local extension import canceled before activation");
    }
    Ok(())
}
fn copy_tree(
    root: &Path,
    source: &Path,
    destination: &Path,
    depth: usize,
    budget: &mut Budget<'_>,
    digest: &mut Sha256,
) -> Result<()> {
    check(budget.cancel)?;
    if depth > 64 {
        bail!("Local extension directories exceed 64 levels");
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        budget.entries += 1;
        budget.path_bytes = budget
            .path_bytes
            .saturating_add(entry.path().as_os_str().as_encoded_bytes().len());
        if budget.entries > MAX_LOCAL_ENTRIES || budget.path_bytes > MAX_PATH_BYTES {
            bail!("Local package inventory exceeds its bounded entry/path budget");
        }
        children.push(entry);
    }
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        check(budget.cancel)?;
        let input = entry.path();
        let output = destination.join(entry.file_name());
        let relative = input.strip_prefix(root)?;
        digest.update(relative.as_os_str().as_encoded_bytes());
        digest.update([0]);
        let metadata = fs::symlink_metadata(&input)?;
        if metadata.is_dir() {
            digest.update(b"directory\0");
            fs::create_dir(&output)?;
            copy_tree(root, &input, &output, depth + 1, budget, digest)?;
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(&input)?;
            if target.is_absolute() {
                bail!(
                    "Local package contains an absolute link: {}",
                    relative.display()
                );
            }
            let resolved = fs::canonicalize(&input)?;
            if !resolved.starts_with(root) {
                bail!(
                    "Local package link escapes its package: {}",
                    relative.display()
                );
            }
            digest.update(b"symlink\0");
            digest.update((target.as_os_str().as_encoded_bytes().len() as u64).to_le_bytes());
            digest.update(target.as_os_str().as_encoded_bytes());
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &output)?;
            #[cfg(windows)]
            if resolved.is_dir() {
                std::os::windows::fs::symlink_dir(&target, &output)?;
            } else {
                std::os::windows::fs::symlink_file(&target, &output)?;
            }
            #[cfg(not(any(unix, windows)))]
            bail!("This platform cannot snapshot local package links");
        } else if metadata.is_file() {
            if metadata.len() > MAX_LOCAL_FILE {
                bail!("Local extension file exceeds 2 GiB: {}", relative.display());
            }
            budget.bytes = budget
                .bytes
                .checked_add(metadata.len())
                .context("Local snapshot size overflow")?;
            if budget.bytes > MAX_LOCAL_BYTES {
                bail!("Local extension import exceeds 16 GiB");
            }
            digest.update(b"file\0");
            digest.update(metadata.len().to_le_bytes());
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let mut reader = options.open(&input)?;
            let before = reader.metadata()?;
            let mut writer = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&output)?;
            let mut copied = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                check(budget.cancel)?;
                let length = reader.read(&mut buffer)?;
                if length == 0 {
                    break;
                }
                copied += length as u64;
                if copied > metadata.len() {
                    bail!(
                        "Local package file grew while copying: {}",
                        relative.display()
                    );
                }
                writer.write_all(&buffer[..length])?;
                digest.update(&buffer[..length]);
            }
            let after = fs::symlink_metadata(&input)?;
            if copied != metadata.len()
                || before.modified()? != after.modified()?
                || after.len() != before.len()
                || !after.is_file()
            {
                bail!(
                    "Local package file changed while copying: {}",
                    relative.display()
                );
            }
            writer.sync_all()?;
            fs::set_permissions(output, metadata.permissions())?;
        } else {
            bail!(
                "Local package contains a special file: {}",
                relative.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(root: &Path, name: &str, version: &str) -> DirectorySource {
        let path = root.join(format!("{name}-{version}"));
        fs::create_dir_all(&path).unwrap();
        let manifest = format!(
            r#"{{"publisher":"fixture","name":"{name}","version":"{version}","main":"main.cjs","engines":{{"vscode":"^1.95.0"}}}}"#
        );
        fs::write(path.join("package.json"), &manifest).unwrap();
        fs::write(
            path.join("main.cjs"),
            "throw new Error('never execute during install')",
        )
        .unwrap();
        DirectorySource {
            path,
            id: format!("fixture.{name}"),
            version: version.into(),
            manifest_sha256: format!("{:x}", Sha256::digest(manifest.as_bytes())),
        }
    }
    #[test]
    fn local_snapshots_preserve_bytes_history_and_never_activate_code() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("store"));
        let first = source(root.path(), "native", "1.0.0");
        fs::write(first.path.join("data"), "λ🙂\r\n").unwrap();
        let installed = store
            .import_directories(
                std::slice::from_ref(&first),
                &AtomicBool::new(false),
                &mut |_, _, _| {},
            )
            .unwrap();
        assert_eq!(
            fs::read(installed[0].path.join("data")).unwrap(),
            "λ🙂\r\n".as_bytes()
        );
        let second = source(root.path(), "native", "2.0.0");
        store
            .import_directories(&[second], &AtomicBool::new(false), &mut |_, _, _| {})
            .unwrap();
        assert_eq!(store.get(&first.id).unwrap().version, "2.0.0");
        assert_eq!(store.rollback(&first.id).unwrap().version, "1.0.0");
        assert_eq!(
            fs::read_to_string(first.path.join("data")).unwrap(),
            "λ🙂\r\n"
        );
    }
    #[test]
    fn late_invalid_package_and_cancellation_leave_the_existing_registry_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("store"));
        let original = source(root.path(), "original", "1.0.0");
        store
            .import_directories(&[original], &AtomicBool::new(false), &mut |_, _, _| {})
            .unwrap();
        let before = fs::read(store.root.join("registry.json")).unwrap();
        let first = source(root.path(), "one", "1.0.0");
        let bad = source(root.path(), "two", "1.0.0");
        fs::write(bad.path.join("package.json"), "{}").unwrap();
        assert!(
            store
                .import_directories(
                    &[first.clone(), bad],
                    &AtomicBool::new(false),
                    &mut |_, _, _| {}
                )
                .is_err()
        );
        assert_eq!(fs::read(store.root.join("registry.json")).unwrap(), before);
        let cancel = AtomicBool::new(false);
        assert!(
            store
                .import_directories(&[first], &cancel, &mut |_, _, _| cancel
                    .store(true, Ordering::Release))
                .is_err()
        );
        assert_eq!(fs::read(store.root.join("registry.json")).unwrap(), before);
        assert_eq!(store.list().unwrap().len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn internal_links_and_executable_modes_survive_but_escaping_links_fail_atomically() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().join("store"));
        let source = source(root.path(), "native", "1.0.0");
        fs::set_permissions(
            source.path.join("main.cjs"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("main.cjs", source.path.join("linked.cjs")).unwrap();
        let installed = store
            .import_directories(
                std::slice::from_ref(&source),
                &AtomicBool::new(false),
                &mut |_, _, _| {},
            )
            .unwrap();
        assert_eq!(
            fs::read_link(installed[0].path.join("linked.cjs")).unwrap(),
            PathBuf::from("main.cjs")
        );
        assert_eq!(
            fs::metadata(installed[0].path.join("main.cjs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        let before = fs::read(store.root.join("registry.json")).unwrap();
        fs::write(root.path().join("outside"), "do not copy").unwrap();
        symlink("../outside", source.path.join("escape")).unwrap();
        assert!(
            store
                .import_directories(&[source], &AtomicBool::new(false), &mut |_, _, _| {})
                .is_err()
        );
        assert_eq!(fs::read(store.root.join("registry.json")).unwrap(), before);
    }
}
