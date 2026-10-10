//! Worker-only destination guards shared by native persistence coordinators.
//!
//! Model authorization belongs to the caller. These opaque guards retain native
//! handles, a cooperating-writer lock and the observed destination baseline.
//! Settings-writer migration is deferred until its Windows fix is qualified.
//! Final checks cannot close an uncooperative writer's check-to-rename race.
use anyhow::{Context, Result, bail, ensure};
use ropey::Rope;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata, OpenOptions, Permissions},
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

#[cfg(test)]
use crate::save_worker::diagnostics::{Span, Stage};

pub const MAX_BYTES: usize = 32 * 1024 * 1024;
const PATH_BYTES: usize = 4096;
const MODELS: usize = 128;
const MODEL_PATH_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy)]
pub enum ParentPolicy {
    Existing,
    Create,
}
pub enum Expected {
    /// Freeze the bounded existing destination, or its absence, at preparation.
    Captured,
    /// A new destination must not replace an existing file.
    Missing,
    /// Ordinary save must still match the model's previously observed disk Rope.
    Exact(Option<Rope>),
}
pub struct Options {
    pub max_bytes: usize,
    pub parent: ParentPolicy,
    pub expected: Expected,
    pub models: Vec<(u64, PathBuf)>,
}
#[derive(Clone, Debug)]
pub struct PreparedInfo {
    pub requested_path: PathBuf,
    pub canonical_path: PathBuf,
    pub matching_models: Vec<u64>,
    pub baseline_sha256: Option<[u8; 32]>,
}
#[derive(Debug)]
pub struct Commit {
    pub path: PathBuf,
    pub sha256: [u8; 32],
    pub bytes: usize,
    pub durability_warning: Option<String>,
}
pub(crate) fn validate_path(path: &Path) -> Result<()> {
    let bytes = path.as_os_str().as_encoded_bytes();
    ensure!(
        !bytes.is_empty()
            && bytes.len() <= PATH_BYTES
            && !bytes.contains(&0)
            && path.file_name().is_some(),
        "Persistence path requires a filename and 1–4096 bytes without NUL"
    );
    Ok(())
}
pub(crate) fn validate_models(models: &[(u64, PathBuf)]) -> Result<()> {
    ensure!(
        models.len() <= MODELS,
        "Persistence inventory exceeds 128 models"
    );
    let mut ids = BTreeSet::new();
    let mut bytes = 0;
    for (id, path) in models {
        ensure!(
            ids.insert(*id),
            "Persistence inventory contains duplicate model IDs"
        );
        validate_path(path)?;
        bytes += path.as_os_str().len();
        ensure!(
            bytes <= MODEL_PATH_BYTES,
            "Persistence model paths exceed 512 KiB"
        );
    }
    Ok(())
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
/// Resolve parent aliases without following a final payload symlink.
pub(crate) fn destination(path: &Path) -> Result<PathBuf> {
    validate_path(path)?;
    let canonical = fs::canonicalize(parent(path))?.join(path.file_name().unwrap());
    validate_path(&canonical)?;
    Ok(canonical)
}

struct LockedFile(File);
impl Drop for LockedFile {
    fn drop(&mut self) {
        #[cfg(test)]
        let _progress = Span::enter(Stage::LockUnlockBegin, Stage::LockUnlockReturned);
        let _ = self.0.unlock();
    }
}
fn open_regular(path: &Path, lock: bool) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.file_type().is_file(),
            "Persistence payload/lock must be an ordinary file; symlinks are refused"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(lock).create(lock).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.file_type().is_file(),
        "Persistence payload/lock must be an ordinary file"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Persistence payload/lock must not be a reparse point"
        );
    }
    Ok(file)
}
#[derive(PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(windows)]
    volume: u64,
    #[cfg(windows)]
    id: [u8; 16],
}
fn file_identity(file: &File) -> Result<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(FileIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::{ffi::c_void, os::windows::io::AsRawHandle};
        #[repr(C)]
        struct IdInfo {
            volume: u64,
            id: [u8; 16],
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandleEx(
                handle: *mut c_void,
                class: i32,
                output: *mut c_void,
                bytes: u32,
            ) -> i32;
        }
        let mut info = IdInfo {
            volume: 0,
            id: [0; 16],
        };
        // SAFETY: File owns the live handle; FileIdInfo(0x12) fills the
        // initialized repr(C)24-byte output during this synchronous call.
        let success = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                0x12,
                (&mut info as *mut IdInfo).cast(),
                std::mem::size_of::<IdInfo>() as u32,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Windows persistence identity unavailable");
        }
        Ok(FileIdentity {
            volume: info.volume,
            id: info.id,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        bail!("Native persistence identity unavailable on this platform")
    }
}
fn open_directory(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000 | 0x00200000); // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.file_type().is_dir(),
        "Persistence parent must remain an ordinary directory"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Persistence parent must not be a reparse point"
        );
    }
    Ok(file)
}
#[derive(PartialEq, Eq)]
struct Signature {
    identity: FileIdentity,
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    mode: u32,
}
impl Signature {
    fn of(file: &File) -> Result<Self> {
        Self::with_metadata(file, &file.metadata()?)
    }
    fn with_metadata(file: &File, metadata: &Metadata) -> Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            identity: file_identity(file)?,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            mode: metadata.mode(),
        })
    }
}
struct Present {
    handle: File,
    signature: Signature,
    permissions: Permissions,
    sha256: [u8; 32],
}
enum Baseline {
    Missing,
    Present(Present),
}
impl Baseline {
    fn read(path: &Path, max_bytes: usize, expected: &Expected) -> Result<Self> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ensure!(
                    !matches!(expected, Expected::Exact(Some(_))),
                    "File changed on disk: saved backing file is missing"
                );
                return Ok(Self::Missing);
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) => ensure!(
                metadata.file_type().is_file(),
                "Persistence target must be an ordinary file; symlinks are refused"
            ),
        }
        ensure!(
            !matches!(expected, Expected::Missing | Expected::Exact(None)),
            "File already exists; existing bytes preserved"
        );
        let mut file = open_regular(path, false)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.len() <= max_bytes as u64,
            "Persistence baseline exceeds its byte budget"
        );
        let signature = Signature::with_metadata(&file, &metadata)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 16 * 1024];
        let mut bytes = 0;
        if let Expected::Exact(Some(expected)) = expected {
            ensure!(
                expected.len_bytes() as u64 == metadata.len(),
                "File changed on disk; existing bytes preserved"
            );
            for chunk in expected.chunks() {
                for expected in chunk.as_bytes().chunks(buffer.len()) {
                    file.read_exact(&mut buffer[..expected.len()])
                        .context("File changed while checking its saved baseline")?;
                    ensure!(
                        &buffer[..expected.len()] == expected,
                        "File changed on disk; existing bytes preserved"
                    );
                    hash.update(expected);
                    bytes += expected.len();
                }
            }
            ensure!(
                read_retry(&mut file, &mut buffer[..1])? == 0,
                "File grew while checking its saved baseline"
            );
        } else {
            loop {
                let allowed = (max_bytes + 1 - bytes).min(buffer.len());
                let count = read_retry(&mut file, &mut buffer[..allowed])?;
                if count == 0 {
                    break;
                }
                bytes += count;
                ensure!(
                    bytes <= max_bytes,
                    "Persistence baseline grew beyond its byte budget"
                );
                hash.update(&buffer[..count]);
            }
        }
        ensure!(
            bytes as u64 == signature.len && Signature::of(&file)? == signature,
            "Persistence target changed while being read"
        );
        ensure!(
            Signature::of(&open_regular(path, false)?)? == signature,
            "Persistence target pathname changed while being read"
        );
        Ok(Self::Present(Present {
            handle: file,
            signature,
            permissions: metadata.permissions(),
            sha256: hash.finalize().into(),
        }))
    }
    fn hash(&self) -> Option<[u8; 32]> {
        match self {
            Self::Missing => None,
            Self::Present(present) => Some(present.sha256),
        }
    }
    fn matches(&self, current: &Self) -> Result<bool> {
        Ok(match (self, current) {
            (Self::Missing, Self::Missing) => true,
            (Self::Present(old), Self::Present(new)) => {
                old.signature == new.signature
                    && old.sha256 == new.sha256
                    && Signature::of(&old.handle)? == old.signature
            }
            _ => false,
        })
    }
}
fn read_retry(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match file.read(buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}
fn matching_models(
    models: &[(u64, PathBuf)],
    target: &Path,
    baseline: &Baseline,
) -> Result<Vec<u64>> {
    let mut matching = Vec::new();
    for (id, path) in models {
        let matches = (|| -> Result<bool> {
            match fs::symlink_metadata(path) {
                Ok(_) => {
                    let canonical = fs::canonicalize(path)?;
                    let identity = file_identity(&open_regular(&canonical, false)?)?;
                    Ok(canonical == target || matches!(baseline,
                        Baseline::Present(present) if present.signature.identity == identity))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(destination(path)? == target),
                Err(error) => Err(error.into()),
            }
        })().with_context(|| format!("Cannot resolve persistence model {id} at {}; close the unresolved model before retrying", path.display()))?;
        if matches {
            matching.push(*id);
        }
    }
    Ok(matching)
}

/// All filesystem operations on this guard must run on a background worker.
pub struct Target {
    requested: PathBuf,
    path: PathBuf,
    parent: PathBuf,
    parent_handle: File,
    parent_identity: FileIdentity,
    lock: LockedFile,
    lock_path: PathBuf,
    lock_identity: FileIdentity,
    baseline: Baseline,
    expected: Expected,
    max_bytes: usize,
    models: Vec<(u64, PathBuf)>,
    matching: Vec<u64>,
}
impl Target {
    pub fn open(requested: &Path, options: Options) -> Result<Self> {
        validate_path(requested)?;
        validate_models(&options.models)?;
        ensure!(
            options.max_bytes > 0 && options.max_bytes <= MAX_BYTES,
            "Persistence byte budget must be within 1–32 MiB"
        );
        if let Expected::Exact(Some(rope)) = &options.expected {
            ensure!(
                rope.len_bytes() <= options.max_bytes,
                "Saved baseline exceeds its byte budget"
            );
        }
        if matches!(options.parent, ParentPolicy::Create) {
            fs::create_dir_all(parent(requested))?;
        }
        let path = destination(requested)?;
        let parent = path.parent().unwrap().to_path_buf();
        let parent_handle = open_directory(&parent)?;
        let parent_identity = file_identity(&parent_handle)?;
        let mut name = path.file_name().unwrap().to_os_string();
        name.push(".vscli-write.lock");
        let lock_path = parent.join(name);
        validate_path(&lock_path)?;
        let lock = open_regular(&lock_path, true)?;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => bail!("Persistence lock unavailable: {error}"),
            }
        }
        let lock = LockedFile(lock);
        let lock_identity = file_identity(&lock.0)?;
        let baseline = Baseline::read(&path, options.max_bytes, &options.expected)?;
        if let Baseline::Present(present) = &baseline {
            ensure!(
                !present.permissions.readonly(),
                "Persistence target is read-only"
            );
        }
        let matching = matching_models(&options.models, &path, &baseline)?;
        Ok(Self {
            requested: requested.into(),
            path,
            parent,
            parent_handle,
            parent_identity,
            lock,
            lock_path,
            lock_identity,
            baseline,
            expected: options.expected,
            max_bytes: options.max_bytes,
            models: options.models,
            matching,
        })
    }
    pub fn info(&self) -> PreparedInfo {
        PreparedInfo {
            requested_path: self.requested.clone(),
            canonical_path: self.path.clone(),
            matching_models: self.matching.clone(),
            baseline_sha256: self.baseline.hash(),
        }
    }
    /// The settings patcher needs bytes; large document saves never call this.
    pub fn read_bytes(&mut self, limit: usize) -> Result<Option<Vec<u8>>> {
        ensure!(
            limit <= self.max_bytes,
            "Persistence read exceeds its configured byte budget"
        );
        let Baseline::Present(present) = &mut self.baseline else {
            return Ok(None);
        };
        ensure!(
            present.signature.len <= limit as u64,
            "Persistence read exceeds its byte budget"
        );
        present.handle.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::with_capacity(present.signature.len as usize);
        Read::by_ref(&mut present.handle)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= limit
                && <[u8; 32]>::from(Sha256::digest(&bytes)) == present.sha256
                && Signature::of(&present.handle)? == present.signature,
            "Persistence baseline changed before parsing"
        );
        Ok(Some(bytes))
    }
    pub fn stage_rope(self, rope: &Rope) -> Result<Prepared> {
        ensure!(
            rope.len_bytes() <= self.max_bytes,
            "Save snapshot exceeds its byte budget"
        );
        self.stage(rope.len_bytes(), |writer| rope.write_to(writer))
    }
    pub fn stage_bytes(self, bytes: &[u8]) -> Result<Prepared> {
        ensure!(
            bytes.len() <= self.max_bytes,
            "Persistence output exceeds its byte budget"
        );
        self.stage(bytes.len(), |writer| writer.write_all(bytes))
    }
    fn stage(
        self,
        bytes: usize,
        write: impl FnOnce(&mut HashWriter<BufWriter<&mut File>>) -> std::io::Result<()>,
    ) -> Result<Prepared> {
        let mut temporary = tempfile::NamedTempFile::new_in(&self.parent)?;
        if let Baseline::Present(present) = &self.baseline {
            temporary
                .as_file()
                .set_permissions(present.permissions.clone())?;
        }
        let hash = {
            let mut writer = HashWriter {
                inner: BufWriter::new(temporary.as_file_mut()),
                hash: Sha256::new(),
                bytes: 0,
                limit: self.max_bytes,
            };
            write(&mut writer)?;
            writer.flush()?;
            ensure!(
                writer.bytes == bytes,
                "Persistence output length changed during staging"
            );
            writer.hash.finalize().into()
        };
        temporary.as_file().sync_all()?;
        let cleanup = TemporaryCleanup {
            parent: self.parent_handle.try_clone()?,
            source: temporary.as_file().try_clone()?,
            leaf: temporary.path().file_name().unwrap().to_os_string(),
        };
        Ok(Prepared {
            target: self,
            temporary,
            bytes,
            sha256: hash,
            _cleanup: cleanup,
        })
    }
    fn recheck(&self) -> Result<()> {
        let current = Baseline::read(&self.path, self.max_bytes, &self.expected)?;
        ensure!(
            self.baseline.matches(&current)?,
            "Persistence target changed externally after preparation; existing bytes preserved"
        );
        ensure!(
            matching_models(&self.models, &self.path, &current)? == self.matching,
            "Persistence model aliases changed after preparation; existing bytes preserved"
        );
        ensure!(
            fs::canonicalize(parent(&self.requested))? == self.parent,
            "Persistence requested parent changed after preparation"
        );
        ensure!(
            file_identity(&self.parent_handle)? == self.parent_identity
                && file_identity(&open_directory(&self.parent)?)? == self.parent_identity,
            "Persistence parent directory identity changed after preparation"
        );
        ensure!(
            file_identity(&self.lock.0)? == self.lock_identity
                && file_identity(&open_regular(&self.lock_path, false)?)? == self.lock_identity,
            "Persistence lock pathname no longer names the owned lock"
        );
        Ok(())
    }
}
struct HashWriter<W> {
    inner: W,
    hash: Sha256,
    bytes: usize,
    limit: usize,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes) {
            return Err(std::io::Error::other(
                "Persistence output exceeds its byte budget",
            ));
        }
        let count = self.inner.write(bytes)?;
        self.hash.update(&bytes[..count]);
        self.bytes += count;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
// NamedTempFile remembers a pathname. Retain native handles too so a rejected
// parent-directory move does not strand its staged file under the new name.
struct TemporaryCleanup {
    parent: File,
    source: File,
    leaf: std::ffi::OsString,
}
impl Drop for TemporaryCleanup {
    fn drop(&mut self) {
        #[cfg(test)]
        let _progress = Span::enter(
            Stage::TemporaryCleanupBegin,
            Stage::TemporaryCleanupReturned,
        );
        #[cfg(unix)]
        {
            use std::{
                ffi::CString,
                os::{
                    fd::{AsRawFd, FromRawFd},
                    unix::ffi::OsStrExt,
                },
            };
            let Ok(leaf) = CString::new(self.leaf.as_bytes()) else {
                return;
            };
            // SAFETY: owned parent descriptor and NUL-terminated leaf remain
            // alive. O_NONBLOCK|O_NOFOLLOW bounds special-file substitutions.
            let descriptor = unsafe {
                libc::openat(
                    self.parent.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if descriptor < 0 {
                return;
            }
            // SAFETY: successful openat returned a newly owned descriptor.
            let named = unsafe { File::from_raw_fd(descriptor) };
            if let (Ok(named), Ok(owned)) = (file_identity(&named), file_identity(&self.source))
                && named == owned
            {
                // SAFETY: unlink only the still-owned leaf in the retained
                // original parent. A different named inode is left untouched.
                let _ = unsafe { libc::unlinkat(self.parent.as_raw_fd(), leaf.as_ptr(), 0) };
            }
        }
        #[cfg(windows)]
        {
            use std::{
                ffi::{OsString, c_void},
                os::windows::{ffi::OsStringExt, io::AsRawHandle},
            };
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetFinalPathNameByHandleW(
                    handle: *mut c_void,
                    output: *mut u16,
                    chars: u32,
                    flags: u32,
                ) -> u32;
            }
            // Windows extended paths have a finite 32768 UTF-16-unit ceiling.
            // This cleanup runs only on the background worker's RAII path.
            let mut buffer = vec![0u16; 32768];
            // SAFETY: source owns the live handle; the initialized output has
            // exactly the advertised units. Flags0 requests normalized DOS path.
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    self.source.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    0,
                )
            } as usize;
            if length == 0 || length >= buffer.len() {
                return;
            }
            let path = PathBuf::from(OsString::from_wide(&buffer[..length]));
            if path.file_name() != Some(self.leaf.as_os_str()) {
                return;
            }
            let Some(parent) = path.parent() else {
                return;
            };
            if !matches!((open_directory(parent).and_then(|file| file_identity(&file)), file_identity(&self.parent)),
                (Ok(named), Ok(owned)) if named == owned)
            {
                return;
            }
            if let (Ok(named), Ok(owned)) = (
                open_regular(&path, false).and_then(|file| file_identity(&file)),
                file_identity(&self.source),
            ) && named == owned
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}
pub struct Prepared {
    target: Target,
    temporary: tempfile::NamedTempFile,
    bytes: usize,
    sha256: [u8; 32],
    _cleanup: TemporaryCleanup,
}
impl Prepared {
    pub fn info(&self) -> PreparedInfo {
        self.target.info()
    }
    pub fn commit(self) -> Result<Commit> {
        {
            #[cfg(test)]
            let _progress = Span::enter(Stage::RecheckBegin, Stage::RecheckReturned);
            self.target.recheck()?;
        }
        if matches!(self.target.baseline, Baseline::Missing) {
            #[cfg(test)]
            let _progress = Span::enter(Stage::NoClobberBegin, Stage::NoClobberReturned);
            self.temporary
                .persist_noclobber(&self.target.path)
                .map_err(|error| error.error)?;
        } else {
            #[cfg(test)]
            let _progress = Span::enter(Stage::ReplaceBegin, Stage::ReplaceReturned);
            replace_existing(self.temporary, &self.target.path)?;
        }
        #[cfg(unix)]
        let durability_warning = self.target.parent_handle.sync_all().err().map(message);
        #[cfg(not(unix))]
        let durability_warning = None;
        Ok(Commit {
            path: self.target.path.clone(),
            sha256: self.sha256,
            bytes: self.bytes,
            durability_warning,
        })
    }
}
fn replace_existing(temporary: tempfile::NamedTempFile, path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        // Own cleanup before keep clears TEMPORARY and disables its original
        // TempPath. Rust's rename supports replacement while baseline handles
        // remain held; tempfile's legacy MoveFileEx replacement does not.
        let mut cleanup = tempfile::TempPath::try_from_path(temporary.path().to_path_buf())?;
        let (_source_handle, source) = temporary.keep().map_err(|error| error.error)?;
        fs::rename(&source, path)?;
        cleanup.disable_cleanup(true);
    }
    #[cfg(not(windows))]
    {
        temporary.persist(path).map_err(|error| error.error)?;
    }
    Ok(())
}
pub(crate) fn message(error: impl std::fmt::Display) -> String {
    let mut text = format!("{error}");
    if text.len() > 4096 {
        let mut end = 4096;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(expected: Expected) -> Options {
        Options {
            max_bytes: MAX_BYTES,
            parent: ParentPolicy::Existing,
            expected,
            models: Vec::new(),
        }
    }
    fn stage(path: &Path, expected: Expected, text: &str) -> Prepared {
        Target::open(path, options(expected))
            .unwrap()
            .stage_rope(&Rope::from_str(text))
            .unwrap()
    }
    fn files(directory: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        paths
    }
    fn assert_clean(path: &Path) {
        let mut lock = path.file_name().unwrap().to_os_string();
        lock.push(".vscli-write.lock");
        let lock = path.with_file_name(lock);
        assert_eq!(
            files(path.parent().unwrap()),
            vec![path.to_path_buf(), lock.clone()]
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        let file = open_regular(&lock, false).unwrap();
        file.try_lock().unwrap();
        file.unlock().unwrap();
    }
    #[test]
    fn exact_rope_streaming_preserves_unicode_crlf_permissions_and_receipt_hash() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        let original = "猫🙂 baseline\r\nsecond\r\n";
        fs::write(&path, original).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, Permissions::from_mode(0o640)).unwrap();
        }
        let mut target = Target::open(
            &path,
            options(Expected::Exact(Some(Rope::from_str(original)))),
        )
        .unwrap();
        assert_eq!(
            target.read_bytes(1024).unwrap().unwrap(),
            original.as_bytes()
        );
        let next = "猫🙂 new\r\nsecond\r\n";
        let prepared = target.stage_rope(&Rope::from_str(next)).unwrap();
        assert_eq!(
            prepared.info().baseline_sha256,
            Some(Sha256::digest(original).into())
        );
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
        let committed = prepared.commit().unwrap();
        assert_eq!(committed.bytes, next.len());
        assert_eq!(committed.sha256, <[u8; 32]>::from(Sha256::digest(next)));
        assert_eq!(fs::read(&path).unwrap(), next.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        assert_clean(&path);
    }
    #[test]
    fn exact_baseline_rejects_changed_deleted_and_replaced_backing_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        fs::write(&path, "baseline\r\n").unwrap();
        assert!(
            Target::open(
                &path,
                options(Expected::Exact(Some(Rope::from_str("wrong"))))
            )
            .is_err()
        );
        let prepared = stage(
            &path,
            Expected::Exact(Some(Rope::from_str("baseline\r\n"))),
            "new",
        );
        fs::write(&path, "external\r\n").unwrap();
        assert!(prepared.commit().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external\r\n");
        let prepared = stage(
            &path,
            Expected::Exact(Some(Rope::from_str("external\r\n"))),
            "new",
        );
        fs::remove_file(&path).unwrap();
        assert!(prepared.commit().is_err());
        assert!(!path.exists());
        assert!(
            Target::open(
                &path,
                options(Expected::Exact(Some(Rope::from_str("external\r\n"))))
            )
            .is_err()
        );
        fs::write(&path, "external\r\n").unwrap();
        let prepared = stage(&path, Expected::Captured, "new");
        let replacement = root.path().join("replacement");
        fs::write(&replacement, "external\r\n").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(
            prepared.commit().is_err(),
            "Equal bytes cannot authorize a replacement native file identity"
        );
        assert_eq!(fs::read(&path).unwrap(), b"external\r\n");
        assert_clean(&path);
    }
    #[test]
    fn missing_target_recheck_and_no_clobber_preserve_a_concurrent_create() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new.txt");
        let prepared = stage(&path, Expected::Missing, "ours 猫🙂\r\n");
        assert!(!path.exists());
        fs::write(&path, "theirs\r\n").unwrap();
        assert!(prepared.commit().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"theirs\r\n");
        assert_clean(&path);
        let absent_parent = root.path().join("absent").join("new.txt");
        assert!(Target::open(&absent_parent, options(Expected::Missing)).is_err());
        assert!(!absent_parent.parent().unwrap().exists());
        let mut create = options(Expected::Missing);
        create.parent = ParentPolicy::Create;
        let committed = Target::open(&absent_parent, create)
            .unwrap()
            .stage_bytes(b"created\r\n")
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(committed.bytes, 9);
        assert_eq!(fs::read(&absent_parent).unwrap(), b"created\r\n");
    }
    #[test]
    fn readonly_and_permission_changes_refuse_without_altering_existing_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        fs::write(&path, "baseline").unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let prepared = stage(&path, Expected::Captured, "new");
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        assert!(prepared.commit().is_err());
        assert!(Target::open(&path, options(Expected::Captured)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"baseline");
        assert!(fs::metadata(&path).unwrap().permissions().readonly());
        fs::set_permissions(&path, original).unwrap();
        assert_clean(&path);
    }
    #[test]
    fn full_32_mib_streaming_and_output_read_inventory_bounds_are_enforced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.txt");
        let rope = Rope::from_str(&"x".repeat(MAX_BYTES));
        let committed = Target::open(&path, options(Expected::Missing))
            .unwrap()
            .stage_rope(&rope)
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(committed.bytes, MAX_BYTES);
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_BYTES as u64);
        let oversized = Rope::from_str(&"x".repeat(MAX_BYTES + 1));
        let target = Target::open(&path, options(Expected::Exact(Some(rope)))).unwrap();
        assert!(target.stage_rope(&oversized).is_err());
        let mut target = Target::open(&path, options(Expected::Captured)).unwrap();
        assert!(target.read_bytes(1024).is_err());
        drop(target);
        let mut duplicate = options(Expected::Captured);
        duplicate.models = vec![(1, path.clone()), (1, path.clone())];
        assert!(Target::open(&path, duplicate).is_err());
        let mut too_many = options(Expected::Captured);
        too_many.models = (0..129).map(|id| (id, path.clone())).collect();
        assert!(Target::open(&path, too_many).is_err());
        assert_clean(&path);
    }
    #[cfg(unix)]
    #[test]
    fn aliases_hardlinks_membership_and_owned_lock_replacement_are_checked() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("main.cpp");
        fs::write(&path, "baseline").unwrap();
        let linked = root.path().join("hardlink.cpp");
        fs::hard_link(&path, &linked).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(root.path(), &alias).unwrap();
        let mut config = options(Expected::Captured);
        config.models = vec![
            (1, path.clone()),
            (2, linked.clone()),
            (3, alias.join("main.cpp")),
        ];
        let prepared = Target::open(&path, config)
            .unwrap()
            .stage_bytes(b"new")
            .unwrap();
        assert_eq!(prepared.info().matching_models, vec![1, 2, 3]);
        fs::remove_file(&linked).unwrap();
        fs::write(&linked, "unrelated").unwrap();
        assert!(prepared.commit().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"baseline");
        let prepared = stage(&path, Expected::Captured, "new");
        let lock = root.path().join("main.cpp.vscli-write.lock");
        fs::remove_file(&lock).unwrap();
        fs::write(&lock, "replacement lock").unwrap();
        assert!(prepared.commit().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"baseline");
    }
    #[cfg(unix)]
    #[test]
    fn parent_move_and_alias_retarget_preserve_payload_and_remove_owned_temporary() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("main.cpp");
        fs::write(&path, "baseline").unwrap();
        let prepared = stage(&path, Expected::Captured, "new");
        let moved = root.path().join("moved");
        fs::rename(&parent, &moved).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::write(&path, "unrelated").unwrap();
        assert!(prepared.commit().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"unrelated");
        assert_eq!(fs::read(moved.join("main.cpp")).unwrap(), b"baseline");
        assert_clean(&moved.join("main.cpp"));
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&moved, &alias).unwrap();
        let prepared = stage(&alias.join("main.cpp"), Expected::Captured, "new");
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&parent, &alias).unwrap();
        assert!(prepared.commit().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"unrelated");
        assert_clean(&moved.join("main.cpp"));
    }
    #[cfg(unix)]
    #[test]
    fn payload_symlink_fifo_and_lock_symlink_are_refused_without_blocking() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real.txt");
        fs::write(&real, "real").unwrap();
        let path = root.path().join("target.txt");
        std::os::unix::fs::symlink(&real, &path).unwrap();
        assert!(Target::open(&path, options(Expected::Captured)).is_err());
        fs::remove_file(&path).unwrap();
        let native = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the owned, terminated pathname is valid for this synchronous fixture call.
        assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
        assert!(Target::open(&path, options(Expected::Captured)).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, "baseline").unwrap();
        let lock = root.path().join("target.txt.vscli-write.lock");
        // Earlier payload refusals leave the stable regular sidecar behind.
        // Prove its ownership was released before replacing this fixture path.
        let owned = open_regular(&lock, false).unwrap();
        owned.try_lock().unwrap();
        owned.unlock().unwrap();
        drop(owned);
        fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink(&real, &lock).unwrap();
        assert!(Target::open(&path, options(Expected::Captured)).is_err());
        assert_eq!(fs::read(&real).unwrap(), b"real");
        assert_eq!(fs::read(&path).unwrap(), b"baseline");
    }
}
