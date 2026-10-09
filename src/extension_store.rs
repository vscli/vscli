//! Native VSIX storage. Installation never executes package code or lifecycle scripts.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_LIST_BYTES: usize = 8 * 1024 * 1024;
const MAX_LIST_NODES: usize = 50_000;

pub fn default_directory() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "vscli", "vscli")
        .map(|d| d.data_local_dir().join("extensions"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub path: PathBuf,
    pub manifest: Value,
    pub source: String,
    pub sha256: String,
    pub compatibility: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Package {
    id: String,
    version: String,
    directory: String,
    source: String,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    current: Package,
    previous: Option<Package>,
}
#[derive(Default, Serialize, Deserialize)]
struct Registry {
    schema: u32,
    packages: BTreeMap<String, Entry>,
}

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}
impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn registry(&self) -> Result<Registry> {
        let path = self.root.join("registry.json");
        if !path.exists() {
            return Ok(Registry {
                schema: 1,
                ..Default::default()
            });
        }
        let bytes = bounded_read(&path, 4 * MAX_MANIFEST)?;
        let registry: Registry = serde_json::from_slice(&bytes)
            .context("Invalid extension registry; existing packages left untouched")?;
        if registry.schema != 1 {
            bail!("Unsupported extension registry schema");
        }
        for (id, entry) in &registry.packages {
            validate_id(id)?;
            for package in std::iter::once(&entry.current).chain(entry.previous.iter()) {
                if package.id != *id
                    || !package.directory.starts_with("package-")
                    || !safe_component(&package.directory)
                {
                    bail!("Invalid extension registry package path");
                }
            }
        }
        Ok(registry)
    }
    fn lock(&self) -> Result<File> {
        fs::create_dir_all(&self.root)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(".lock"))?;
        lock.try_lock()
            .context("Another extension operation is running; retry when it finishes")?;
        Ok(lock)
    }
    fn write_registry(&self, registry: &Registry) -> Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        let bytes = serde_json::to_vec_pretty(registry)?;
        if bytes.len() as u64 >= 4 * MAX_MANIFEST {
            bail!("Extension registry exceeds size limit");
        }
        temp.write_all(&bytes)?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(self.root.join("registry.json"))
            .context("Cannot atomically replace extension registry")?;
        Ok(())
    }
    fn installed(&self, package: &Package) -> Result<Installed> {
        let path = self.root.join(&package.directory).join("extension");
        let manifest = read_manifest(&path)?;
        let (id, version) = identity(&manifest)?;
        if id != package.id || version != package.version {
            bail!("Installed extension manifest identity changed");
        }
        Ok(Installed {
            id,
            version,
            path,
            compatibility: compatibility(&manifest),
            manifest,
            source: package.source.clone(),
            sha256: package.sha256.clone(),
        })
    }
    pub fn list(&self) -> Result<Vec<Installed>> {
        let registry = self.registry()?;
        let mut bytes = 0usize;
        let mut nodes = 0usize;
        let mut installed = Vec::new();
        for entry in registry.packages.values() {
            let package = self.installed(&entry.current)?;
            bytes = bytes.saturating_add(serde_json::to_vec(&package.manifest)?.len());
            nodes = nodes.saturating_add(manifest_nodes(&package.manifest));
            if bytes > MAX_LIST_BYTES || nodes > MAX_LIST_NODES {
                bail!(
                    "Installed manifest list exceeds the 8 MiB / 50,000 JSON-node metadata budget; use an extension ID directly or remove packages"
                );
            }
            installed.push(package);
        }
        Ok(installed)
    }
    pub fn get(&self, id: &str) -> Result<Installed> {
        let id = id.to_ascii_lowercase();
        validate_id(&id)?;
        let registry = self.registry()?;
        self.installed(
            &registry
                .packages
                .get(&id)
                .context("Extension is not installed")?
                .current,
        )
    }
    pub fn install(&self, archive_path: &Path) -> Result<Installed> {
        let _lock = self.lock()?;
        let mut registry = self.registry()?;
        if !fs::metadata(archive_path)
            .context("Cannot inspect VSIX package")?
            .is_file()
        {
            bail!("VSIX source must be a regular file");
        }
        let mut archive_file = File::open(archive_path).context("Cannot open VSIX package")?;
        let size = archive_file.metadata()?.len();
        if size > MAX_ARCHIVE {
            bail!("VSIX exceeds 128 MiB download/archive limit");
        }
        // Snapshot the archive once: parsing and provenance must refer to identical bytes.
        let mut snapshot = tempfile::tempfile_in(&self.root)?;
        let copied = std::io::copy(
            &mut (&mut archive_file).take(MAX_ARCHIVE + 1),
            &mut snapshot,
        )?;
        if copied > MAX_ARCHIVE {
            bail!("VSIX grew beyond 128 MiB archive limit");
        }
        snapshot.rewind()?;
        let mut digest = Sha256::new();
        std::io::copy(&mut snapshot, &mut digest)?;
        snapshot.rewind()?;
        let digest = format!("{:x}", digest.finalize());
        let (count, footer) = preflight_archive(&mut snapshot)?;
        let metadata = Cell::new(true);
        let reader = ValidatedArchive {
            file: snapshot,
            footer,
            metadata: &metadata,
        };
        let mut zip = zip::ZipArchive::new(reader).context("Invalid VSIX ZIP archive")?;
        metadata.set(false);
        if zip.len() != count {
            bail!("Duplicate VSIX central-directory paths are forbidden");
        }
        let stage = tempfile::Builder::new()
            .prefix(".stage-")
            .tempdir_in(&self.root)?;
        let mut names = HashSet::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let name = entry.name().to_owned();
            validate_archive_path(&name)?;
            if !names.insert(name.trim_end_matches('/').to_lowercase()) {
                bail!("Duplicate or case-aliased VSIX path: {name}");
            }
            let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                bail!("Links and special files are forbidden in VSIX packages: {name}");
            }
            if entry.size() > MAX_FILE {
                bail!("VSIX entry exceeds 32 MiB: {name}");
            }
            total = total
                .checked_add(entry.size())
                .context("VSIX size overflow")?;
            if total > MAX_TOTAL {
                bail!("VSIX exceeds 256 MiB extracted limit");
            }
            let path = stage.path().join(&name);
            if entry.is_dir() {
                fs::create_dir_all(&path)?;
                continue;
            }
            fs::create_dir_all(path.parent().context("Invalid package path")?)?;
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            let declared = entry.size();
            let actual = std::io::copy(&mut (&mut entry).take(MAX_FILE + 1), &mut output)?;
            if actual != declared || actual > MAX_FILE {
                bail!("VSIX entry length does not match its declaration");
            }
            output.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if entry.unix_mode().is_some_and(|m| m & 0o111 != 0) {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
                }
            }
        }
        let manifest = read_manifest(&stage.path().join("extension"))?;
        let (id, version) = identity(&manifest)?;
        let directory = format!("package-{}", uuid::Uuid::new_v4());
        let destination = self.root.join(&directory);
        let package = Package {
            id: id.clone(),
            version,
            directory,
            source: fs::canonicalize(archive_path)?.display().to_string(),
            sha256: digest,
        };
        let old = registry.packages.get(&id).cloned();
        registry.packages.insert(
            id,
            Entry {
                current: package.clone(),
                previous: old.as_ref().map(|e| e.current.clone()),
            },
        );
        fs::rename(stage.path(), &destination).context("Cannot publish staged extension files")?;
        if let Err(error) = self.write_registry(&registry) {
            let _ = fs::remove_dir_all(&destination);
            return Err(error);
        }
        // Old generations remain immutable while a running host might still reference them.
        self.installed(&package)
    }
    pub fn rollback(&self, id: &str) -> Result<Installed> {
        let _lock = self.lock()?;
        let mut registry = self.registry()?;
        let entry = registry
            .packages
            .get_mut(&id.to_ascii_lowercase())
            .context("Extension is not installed")?;
        let previous = entry
            .previous
            .clone()
            .context("No previous installation to restore")?;
        let installed = self.installed(&previous)?;
        entry.previous = Some(std::mem::replace(&mut entry.current, previous));
        self.write_registry(&registry)?;
        Ok(installed)
    }
    pub fn uninstall(&self, id: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut registry = self.registry()?;
        registry
            .packages
            .remove(&id.to_ascii_lowercase())
            .context("Extension is not installed")?;
        self.write_registry(&registry)
    }
}

fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        bail!("Extension metadata exceeds size limit");
    }
    Ok(bytes)
}
fn read_manifest(path: &Path) -> Result<Value> {
    serde_json::from_slice(&bounded_read(&path.join("package.json"), MAX_MANIFEST)?)
        .context("Invalid extension/package.json")
}
fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn validate_id(id: &str) -> Result<()> {
    let parts: Vec<_> = id.split('.').collect();
    if parts.len() != 2 || parts.iter().any(|p| !safe_component(p)) {
        bail!("Extension ID must be publisher.name");
    }
    Ok(())
}
fn identity(manifest: &Value) -> Result<(String, String)> {
    let publisher = manifest["publisher"]
        .as_str()
        .context("Extension manifest has no publisher")?;
    let name = manifest["name"]
        .as_str()
        .context("Extension manifest has no name")?;
    let id = format!("{publisher}.{name}").to_ascii_lowercase();
    validate_id(&id)?;
    let version = manifest["version"]
        .as_str()
        .context("Extension manifest has no version")?;
    if version.is_empty()
        || version.len() > 100
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        bail!("Invalid extension version");
    }
    Ok((id, version.into()))
}
fn validate_archive_path(name: &str) -> Result<()> {
    let trimmed = name.trim_end_matches('/');
    if trimmed.is_empty()
        || name.len() > 1024
        || name.contains('\\')
        || name.chars().any(|c| c.is_control())
    {
        bail!("Unsafe VSIX path: {name:?}");
    }
    for component in trimmed.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains([':', '<', '>', '|', '?', '*'])
            || component.ends_with(['.', ' '])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            bail!("Unsafe or nonportable VSIX path: {name:?}");
        }
    }
    Ok(())
}
pub fn compatibility(manifest: &Value) -> String {
    if manifest
        .get("extensionDependencies")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        "Unsupported dependencies; installed only".into()
    } else if manifest.get("main").and_then(Value::as_str).is_some() {
        "Experimental command host; APIs and engine compatibility unverified".into()
    } else if manifest["contributes"]["themes"].is_array() {
        "Declarative theme; no Node activation required".into()
    } else if manifest["contributes"]["snippets"].is_array() {
        "Declarative snippets; no Node activation required".into()
    } else if manifest.get("browser").is_some() {
        "Browser extension runtime unavailable; installed only".into()
    } else {
        "Declarative package; contribution support varies".into()
    }
}

// ZIP metadata allocates before by_index(). Bound the declared count first and
// compare it with ZipArchive's deduplicated index before extracting anything.
fn preflight_archive(file: &mut File) -> Result<(usize, u64)> {
    let size = file.metadata()?.len();
    let tail_len = size.min(65_535 + 22);
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0; tail_len as usize];
    file.read_exact(&mut tail)?;
    let offset = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            tail[i..].starts_with(b"PK\x05\x06")
                && i + 22 + u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize == tail.len()
        })
        .context("Invalid VSIX end-of-directory record")?;
    let end = &tail[offset..];
    let short = |i| u16::from_le_bytes([end[i], end[i + 1]]);
    let long = |i| u32::from_le_bytes([end[i], end[i + 1], end[i + 2], end[i + 3]]) as u64;
    let count = short(10) as usize;
    if short(4) != 0 || short(6) != 0 || short(8) as usize != count {
        bail!("Multi-disk VSIX archives are unsupported");
    }
    if count > MAX_ENTRIES {
        bail!("VSIX exceeds 20,000 entries; ZIP64 is unsupported");
    }
    let directory_end = long(16)
        .checked_add(long(12))
        .context("Invalid VSIX central-directory size")?;
    if long(16) == u32::MAX as u64
        || long(12) == u32::MAX as u64
        || directory_end != size - tail_len + offset as u64
    {
        bail!("ZIP64, prefixed or malformed VSIX archives are unsupported");
    }
    file.rewind()?;
    Ok((count, size - tail_len + offset as u64))
}

fn manifest_nodes(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(manifest_nodes).sum::<usize>(),
        Value::Object(items) => 1 + items.values().map(manifest_nodes).sum::<usize>(),
        _ => 1,
    }
}

// zip-rs retries earlier EOCD candidates after malformed metadata. Its finder
// seeks to each candidate before parsing it. Only the bounded, preflighted footer
// may supply allocation counts. Payload reads become ordinary reads afterward.
struct ValidatedArchive<'a> {
    file: File,
    footer: u64,
    metadata: &'a Cell<bool>,
}
impl Read for ValidatedArchive<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buffer)
    }
}
impl Seek for ValidatedArchive<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let offset = self.file.seek(position)?;
        if self.metadata.get() && offset != self.footer {
            let mut magic = [0; 4];
            let length = self.file.read(&mut magic)?;
            self.file.seek(SeekFrom::Start(offset))?;
            if length == 4 && magic == *b"PK\x05\x06" {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unexpected VSIX end-of-directory candidate; ambiguous metadata is forbidden",
                ));
            }
        }
        Ok(offset)
    }
}
