//! Bounded two-phase settings persistence. All filesystem work stays on one worker.
//!
//! Preparation may create parent directories and a stable adjacent lock file,
//! but leaves settings bytes unchanged until explicit UI authorization. The
//! lock coordinates cooperating writers; an uncooperative writer can still
//! race the final baseline check and atomic rename.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata, OpenOptions, Permissions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

const FILE_BYTES: usize = 1024 * 1024;
const PATH_BYTES: usize = 4096;
const KEY_BYTES: usize = 1024;
const PATCHES: usize = 16;
const VALUE_BYTES: usize = 64 * 1024;
const AUTHORIZATION: Duration = Duration::from_secs(6);
const MODELS: usize = 128;
const MODEL_PATH_BYTES: usize = 512 * 1024;

#[derive(Clone)]
pub struct Intent {
    path: PathBuf,
    profile: u64,
    key: String,
    value: Value,
    models: Vec<(u64, PathBuf)>,
}
impl Intent {
    /// Profile is a caller-owned identity, not a path hash. A changed profile
    /// must use a new identity even if it targets the same settings pathname.
    pub fn new(path: PathBuf, profile: u64, key: String, value: Value) -> Result<Self> {
        validate_path(&path)?;
        ensure!(
            !key.is_empty() && key.len() <= KEY_BYTES && !key.contains('\0'),
            "Setting key must contain 1–1024 bytes without NUL"
        );
        value_bytes(&value)?;
        Ok(Self {
            path,
            profile,
            key,
            value,
            models: Vec::new(),
        })
    }
    /// Snapshot all retained file-backed models. The worker resolves aliases;
    /// the UI later validates returned matching IDs and their dirty/epoch state.
    pub fn with_models(mut self, models: Vec<(u64, PathBuf)>) -> Result<Self> {
        validate_models(&models)?;
        self.models = models;
        Ok(self)
    }
}
#[derive(Clone)]
struct Batch {
    id: u64,
    path: PathBuf,
    profile: u64,
    patches: Vec<(String, Value)>,
    models: Vec<(u64, PathBuf)>,
}
impl Batch {
    fn same_target(&self, intent: &Intent) -> bool {
        self.path == intent.path && self.profile == intent.profile
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.patches.len() <= PATCHES,
            "Settings batch exceeds 16 keys"
        );
        let mut bytes = 0;
        for (_, value) in &self.patches {
            bytes += value_bytes(value)?;
            ensure!(bytes <= VALUE_BYTES, "Settings batch values exceed 64 KiB");
        }
        validate_models(&self.models)?;
        Ok(())
    }
}
fn validate_models(models: &[(u64, PathBuf)]) -> Result<()> {
    ensure!(
        models.len() <= MODELS,
        "Settings model inventory exceeds 128 models"
    );
    let mut ids = BTreeSet::new();
    let mut bytes = 0usize;
    for (id, path) in models {
        ensure!(
            ids.insert(*id),
            "Settings model inventory contains duplicate IDs"
        );
        validate_path(path)?;
        bytes += path.as_os_str().len();
        ensure!(
            bytes <= MODEL_PATH_BYTES,
            "Settings model paths exceed 512 KiB"
        );
    }
    Ok(())
}
fn validate_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty() && path.as_os_str().len() <= PATH_BYTES,
        "Settings path must contain 1–4096 bytes"
    );
    ensure!(
        !path.as_os_str().as_encoded_bytes().contains(&0),
        "Settings path contains NUL"
    );
    ensure!(
        path.file_name().is_some(),
        "Settings path requires a filename"
    );
    Ok(())
}
struct Count(usize);
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > VALUE_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("Setting value exceeds 64 KiB"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn value_bytes(value: &Value) -> Result<usize> {
    ensure!(
        !value.is_array() && !value.is_object(),
        "Settings writer accepts scalar values only"
    );
    if let Value::String(text) = value {
        ensure!(text.len() <= VALUE_BYTES, "Setting value exceeds 64 KiB");
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value)?;
    Ok(count.0)
}

#[derive(Clone, Debug)]
pub struct PreparedInfo {
    pub id: u64,
    pub path: PathBuf,
    pub canonical_path: PathBuf,
    pub profile: u64,
    pub baseline_sha256: Option<[u8; 32]>,
    pub expires_at: Instant,
    pub matching_models: Vec<u64>,
}
#[derive(Debug)]
pub enum Outcome {
    Committed {
        path: PathBuf,
        sha256: [u8; 32],
        bytes: usize,
        durability_warning: Option<String>,
    },
    Rejected,
    Expired,
}
#[derive(Debug)]
pub enum Event {
    Prepared(PreparedInfo),
    Finished {
        id: u64,
        result: std::result::Result<Outcome, String>,
    },
}
#[derive(Debug)]
enum Decision {
    Authorize,
    Reject,
}
enum WorkerEvent {
    Prepared(PreparedInfo),
    Finished(std::result::Result<Outcome, String>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Awaiting,
    Committing,
    Retiring,
    Finishing,
}
struct Pending {
    batch: Batch,
    control: SyncSender<Decision>,
    receiver: Receiver<WorkerEvent>,
    thread: JoinHandle<()>,
    phase: Phase,
    deadline: Option<Instant>,
    finished: Option<std::result::Result<Outcome, String>>,
}
#[derive(Default)]
pub struct Writer {
    next_id: u64,
    pending: Option<Pending>,
    queued: Option<Batch>,
}
impl Writer {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.queued.is_some()
    }
    pub fn awaiting_authorization(&self, id: u64) -> bool {
        self.pending.as_ref().is_some_and(|p| {
            p.batch.id == id
                && p.phase == Phase::Awaiting
                && p.deadline.is_some_and(|at| Instant::now() < at)
        })
    }
    /// One latest batch is retained. Same-target uncommitted scalar patches
    /// survive coalescing; a changed path/profile starts an independent batch.
    pub fn request(&mut self, intent: Intent) -> Result<u64> {
        let id = self
            .next_id
            .checked_add(1)
            .context("Settings writer identity exhausted; restart the editor")?;
        let base = self
            .queued
            .as_ref()
            .filter(|batch| batch.same_target(&intent))
            .or_else(|| {
                self.pending
                    .as_ref()
                    .filter(|p| {
                        matches!(p.phase, Phase::Preparing | Phase::Awaiting)
                            && p.batch.same_target(&intent)
                    })
                    .map(|p| &p.batch)
            });
        let mut batch = if let Some(base) = base {
            base.clone()
        } else {
            Batch {
                id,
                path: intent.path.clone(),
                profile: intent.profile,
                patches: Vec::new(),
                models: Vec::new(),
            }
        };
        batch.id = id;
        // A newer UI inventory supersedes the previous snapshot even when
        // scalar patches from the same unapproved target are carried forward.
        batch.models = intent.models;
        if let Some((_, value)) = batch.patches.iter_mut().find(|(key, _)| key == &intent.key) {
            *value = intent.value;
        } else {
            batch.patches.push((intent.key, intent.value));
        }
        batch.validate()?;
        if self.pending.is_none() {
            let pending = spawn(batch)?;
            self.next_id = id;
            self.pending = Some(pending);
            self.queued = None;
        } else {
            self.next_id = id;
            self.queued = Some(batch);
            if let Some(pending) = &mut self.pending
                && matches!(pending.phase, Phase::Preparing | Phase::Awaiting)
            {
                let _ = pending.control.try_send(Decision::Reject);
                pending.phase = Phase::Retiring;
            }
        }
        Ok(id)
    }
    /// Caller checks current profile/dirty-model/generation guards immediately
    /// before this call. An already authorized commit cannot be canceled.
    pub fn authorize(&mut self, id: u64) -> Result<()> {
        ensure!(self.queued.is_none(), "Settings preparation was superseded");
        let pending = self.pending.as_mut().context("No settings preparation")?;
        ensure!(
            pending.batch.id == id && pending.phase == Phase::Awaiting,
            "Settings preparation is no longer owned"
        );
        ensure!(
            pending.deadline.is_some_and(|at| Instant::now() < at),
            "Settings authorization expired"
        );
        pending
            .control
            .try_send(Decision::Authorize)
            .context("Settings worker is no longer awaiting authorization")?;
        pending.phase = Phase::Committing;
        Ok(())
    }
    pub fn reject(&mut self, id: u64) -> bool {
        self.cancel(id)
    }
    pub fn cancel(&mut self, id: u64) -> bool {
        if self.queued.as_ref().is_some_and(|batch| batch.id == id) {
            self.queued = None;
            return true;
        }
        if let Some(pending) = &mut self.pending
            && pending.batch.id == id
            && matches!(pending.phase, Phase::Preparing | Phase::Awaiting)
        {
            let _ = pending.control.try_send(Decision::Reject);
            pending.phase = Phase::Retiring;
            return true;
        }
        false
    }
    /// Nonblocking. Actual capacity is released only after worker termination,
    /// including tempfile cleanup and explicit advisory-lock release.
    pub fn poll(&mut self) -> Option<Event> {
        if self.pending.is_none()
            && let Some(batch) = self.queued.take()
        {
            let id = batch.id;
            match spawn(batch) {
                Ok(pending) => self.pending = Some(pending),
                Err(error) => {
                    return Some(Event::Finished {
                        id,
                        result: Err(message(error)),
                    });
                }
            }
        }
        for _ in 0..2 {
            let pending = self.pending.as_mut()?;
            if pending.phase == Phase::Finishing && pending.thread.is_finished() {
                let pending = self.pending.take().unwrap();
                let id = pending.batch.id;
                let joined = pending.thread.join();
                let result = if joined.is_ok() {
                    pending
                        .finished
                        .unwrap_or_else(|| Err("Settings worker ended without a result".into()))
                } else {
                    Err("Settings worker panicked".into())
                };
                return Some(Event::Finished { id, result });
            }
            match pending.receiver.try_recv() {
                Ok(WorkerEvent::Prepared(info)) => {
                    if pending.phase == Phase::Preparing
                        && self.queued.is_none()
                        && Instant::now() < info.expires_at
                    {
                        pending.phase = Phase::Awaiting;
                        pending.deadline = Some(info.expires_at);
                        return Some(Event::Prepared(info));
                    }
                    let _ = pending.control.try_send(Decision::Reject);
                    pending.phase = Phase::Retiring;
                }
                Ok(WorkerEvent::Finished(result)) => {
                    pending.finished = Some(result);
                    pending.phase = Phase::Finishing;
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    if pending.finished.is_none() {
                        pending.finished = Some(Err("Settings worker disconnected".into()));
                        pending.phase = Phase::Finishing;
                    }
                }
            }
        }
        None
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending
            && matches!(
                pending.phase,
                Phase::Preparing | Phase::Awaiting | Phase::Retiring
            )
        {
            let _ = pending.control.try_send(Decision::Reject);
        }
    }
}
fn spawn(batch: Batch) -> Result<Pending> {
    spawn_with_timeout(batch, AUTHORIZATION)
}
fn spawn_with_timeout(batch: Batch, authorization: Duration) -> Result<Pending> {
    let (sender, receiver) = mpsc::sync_channel(2);
    let (control, decisions) = mpsc::sync_channel(1);
    let worker_batch = batch.clone();
    let thread = thread::Builder::new()
        .name("vscli-settings-write".into())
        .spawn(move || {
            // Resource-owning scope returns before the terminal notification.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(&worker_batch, &sender, &decisions, authorization)
            }))
            .map_err(|_| "Settings worker panicked".to_owned())
            .and_then(|result| result.map_err(message));
            let _ = sender.send(WorkerEvent::Finished(result));
        })?;
    Ok(Pending {
        batch,
        control,
        receiver,
        thread,
        phase: Phase::Preparing,
        deadline: None,
        finished: None,
    })
}
fn message(error: impl std::fmt::Display) -> String {
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

struct LockedFile(File);
impl Drop for LockedFile {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn open_regular(path: &Path, lock: bool) -> Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.file_type().is_file(),
            "Settings payload/lock must be a regular file; symlinks are refused"
        );
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
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.file_type().is_file(),
        "Settings payload/lock must be a regular file"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Settings payload/lock must not be a reparse point"
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
        // Stable Rust1.99 MetadataExt file_index/volume_serial_number are
        // nightly-only. Query FILE_ID_INFO using the documented Windows ABI:
        // https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info
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
        // SAFETY: File owns a live handle throughout this synchronous call.
        // FileIdInfo(0x12) fills the repr(C),24-byte FILE_ID_INFO output; BOOL
        // is i32 and DWORD is u32. The buffer is initialized and remains valid.
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
                .context("Windows file identity unavailable; settings write refused");
        }
        Ok(FileIdentity {
            volume: info.volume,
            id: info.id,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        bail!("Native file identity unavailable; settings write refused")
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
    let directory = options.open(path)?;
    ensure!(
        directory.metadata()?.file_type().is_dir(),
        "Settings parent must remain a directory"
    );
    Ok(directory)
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
        let metadata = file.metadata()?;
        Self::with_metadata(file, &metadata)
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

enum Baseline {
    Missing,
    Present {
        bytes: Vec<u8>,
        signature: Signature,
        permissions: Permissions,
        handle: File,
    },
}
impl Baseline {
    fn read(path: &Path) -> Result<Self> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error.into()),
            Ok(metadata) => ensure!(
                metadata.file_type().is_file(),
                "Settings target must be a regular file; symlinks are refused"
            ),
        }
        let mut file = open_regular(path, false)?;
        let metadata = file.metadata()?;
        ensure!(metadata.len() <= FILE_BYTES as u64, "Settings exceed 1 MiB");
        let signature = Signature::with_metadata(&file, &metadata)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= FILE_BYTES, "Settings exceed 1 MiB");
        ensure!(
            Signature::of(&file)? == signature,
            "Settings changed while being read"
        );
        let named = open_regular(path, false)?;
        ensure!(
            Signature::of(&named)? == signature,
            "Settings target changed while being read"
        );
        Ok(Self::Present {
            bytes,
            signature,
            permissions: metadata.permissions(),
            handle: file,
        })
    }
    fn input(&self) -> &[u8] {
        match self {
            Self::Missing => b"{}\n",
            Self::Present { bytes, .. } => bytes,
        }
    }
    fn hash(&self) -> Option<[u8; 32]> {
        match self {
            Self::Missing => None,
            Self::Present { bytes, .. } => Some(Sha256::digest(bytes).into()),
        }
    }
    fn matches(&self, other: &Self) -> Result<bool> {
        Ok(match (self, other) {
            (Self::Missing, Self::Missing) => true,
            (
                Self::Present {
                    bytes,
                    signature,
                    handle,
                    ..
                },
                Self::Present {
                    bytes: current,
                    signature: current_signature,
                    ..
                },
            ) => {
                // Keep the original opened file alive throughout preparation:
                // its native ID cannot be reused after an unlink/replacement.
                bytes == current
                    && signature == current_signature
                    && Signature::of(handle)? == *signature
            }
            _ => false,
        })
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
                    let file = open_regular(&canonical, false)?;
                    let identity = file_identity(&file)?;
                    Ok(canonical == target
                        || matches!(baseline,
                            Baseline::Present { signature, .. } if signature.identity == identity))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let parent = path.parent().filter(|path| !path.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new("."));
                    let canonical = fs::canonicalize(parent)?.join(path.file_name().context("Missing model filename")?);
                    Ok(canonical == target)
                }
                Err(error) => Err(error.into()),
            }
        })().with_context(|| format!(
            "Cannot resolve settings model {id} at {}; close the unresolved model before retrying",
            path.display()
        ))?;
        if matches {
            matching.push(*id);
        }
    }
    Ok(matching)
}
fn work(
    batch: &Batch,
    events: &SyncSender<WorkerEvent>,
    decisions: &Receiver<Decision>,
    authorization: Duration,
) -> Result<Outcome> {
    let requested_parent = batch
        .path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(requested_parent)?;
    let parent = fs::canonicalize(requested_parent)?;
    let parent_handle = open_directory(&parent)?;
    let parent_identity = file_identity(&parent_handle)?;
    let path = parent.join(
        batch
            .path
            .file_name()
            .context("Missing settings filename")?,
    );
    validate_path(&path)?;
    let mut lock_name = path.file_name().unwrap().to_os_string();
    lock_name.push(".vscli-write.lock");
    let lock_path = parent.join(lock_name);
    validate_path(&lock_path)?;
    let lock = open_regular(&lock_path, true)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => bail!("Settings lock unavailable: {error}"),
        }
    }
    let lock_guard = LockedFile(lock);
    let lock_identity = file_identity(&lock_guard.0)?;
    let baseline = Baseline::read(&path)?;
    let matching_models = matching_models(&batch.models, &path, &baseline)?;
    if let Baseline::Present { permissions, .. } = &baseline {
        ensure!(!permissions.readonly(), "Settings file is read-only");
    }
    let mut bytes = baseline.input().to_vec();
    for (key, value) in &batch.patches {
        bytes = crate::settings_write::patch(&bytes, key, value)?;
    }
    ensure!(bytes.len() <= FILE_BYTES, "Patched settings exceed 1 MiB");
    let mut temporary = tempfile::NamedTempFile::new_in(&parent)?;
    temporary.write_all(&bytes)?;
    if let Baseline::Present { permissions, .. } = &baseline {
        temporary.as_file().set_permissions(permissions.clone())?;
    }
    temporary.as_file().sync_all()?;
    let expires_at = Instant::now() + authorization;
    let info = PreparedInfo {
        id: batch.id,
        path: batch.path.clone(),
        canonical_path: path.clone(),
        profile: batch.profile,
        baseline_sha256: baseline.hash(),
        expires_at,
        matching_models: matching_models.clone(),
    };
    if events.send(WorkerEvent::Prepared(info)).is_err() {
        return Ok(Outcome::Rejected);
    }
    match decisions.recv_timeout(authorization) {
        Ok(Decision::Reject) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Ok(Outcome::Rejected);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => return Ok(Outcome::Expired),
        Ok(Decision::Authorize) => ensure!(
            Instant::now() < expires_at,
            "Settings authorization expired"
        ),
    }
    let current = Baseline::read(&path)?;
    ensure!(
        baseline.matches(&current)?,
        "Settings changed externally after preparation; existing bytes preserved"
    );
    ensure!(
        self::matching_models(&batch.models, &path, &current)? == matching_models,
        "Settings model aliases changed after preparation; existing bytes preserved"
    );
    ensure!(
        fs::canonicalize(requested_parent)? == parent,
        "Settings requested parent changed after preparation"
    );
    let current_parent = open_directory(&parent)?;
    ensure!(
        file_identity(&current_parent)? == parent_identity,
        "Settings parent directory identity changed after preparation"
    );
    let current_lock = open_regular(&lock_path, false)
        .context("Settings lock pathname disappeared or became invalid")?;
    ensure!(
        file_identity(&current_lock)? == lock_identity,
        "Settings lock pathname no longer names the owned lock"
    );
    match baseline {
        Baseline::Missing => {
            temporary
                .persist_noclobber(&path)
                .map_err(|error| error.error)?;
        }
        Baseline::Present { .. } => {
            temporary.persist(&path).map_err(|error| error.error)?;
        }
    }
    #[cfg(unix)]
    let durability_warning = File::open(&parent)
        .and_then(|directory| directory.sync_all())
        .err()
        .map(message);
    #[cfg(not(unix))]
    let durability_warning = None;
    Ok(Outcome::Committed {
        path,
        sha256: Sha256::digest(&bytes).into(),
        bytes: bytes.len(),
        durability_warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn intent(path: &Path, profile: u64, key: &str, value: Value) -> Intent {
        Intent::new(path.into(), profile, key.into(), value).unwrap()
    }
    fn event(writer: &mut Writer) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = writer.poll() {
                return event;
            }
            assert!(Instant::now() < deadline, "settings worker did not settle");
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn prepared(writer: &mut Writer, id: u64) -> PreparedInfo {
        match event(writer) {
            Event::Prepared(info) => {
                assert_eq!(info.id, id);
                info
            }
            Event::Finished { result, .. } => panic!("unexpected finish: {result:?}"),
        }
    }
    fn finished(writer: &mut Writer, id: u64) -> std::result::Result<Outcome, String> {
        match event(writer) {
            Event::Finished { id: actual, result } => {
                assert_eq!(actual, id);
                result
            }
            Event::Prepared(info) => panic!("unexpected preparation: {info:?}"),
        }
    }
    fn assert_unlocked(path: &Path) {
        let mut name = path.file_name().unwrap().to_os_string();
        name.push(".vscli-write.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_file_name(name))
            .unwrap();
        lock.try_lock().unwrap();
        lock.unlock().unwrap();
    }
    #[test]
    fn explicit_authorization_preserves_every_other_jsonc_crlf_byte_and_permissions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let original = b"{ // note\r\n  \"breadcrumbs.enabled\": true,\r\n  \"other\": 7\r\n}\r\n";
        fs::write(&path, original).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, Permissions::from_mode(0o640)).unwrap();
        }
        let mut writer = Writer::default();
        let id = writer
            .request(intent(&path, 1, "breadcrumbs.enabled", json!(false)))
            .unwrap();
        let info = prepared(&mut writer, id);
        assert_eq!(info.profile, 1);
        assert_eq!(info.canonical_path, fs::canonicalize(&path).unwrap());
        assert_eq!(info.baseline_sha256, Some(Sha256::digest(original).into()));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(writer.awaiting_authorization(id));
        writer.authorize(id).unwrap();
        assert!(!writer.cancel(id));
        assert!(matches!(
            finished(&mut writer, id).unwrap(),
            Outcome::Committed { .. }
        ));
        assert_eq!(
            fs::read(&path).unwrap(),
            b"{ // note\r\n  \"breadcrumbs.enabled\": false,\r\n  \"other\": 7\r\n}\r\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        assert_unlocked(&path);
        assert!(!writer.busy());
    }
    #[test]
    fn rejection_cleans_adjacent_temporary_and_explicitly_unlocks_without_changing_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\r\n").unwrap();
        let mut writer = Writer::default();
        let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
        prepared(&mut writer, id);
        assert!(writer.reject(id));
        assert!(matches!(
            finished(&mut writer, id).unwrap(),
            Outcome::Rejected
        ));
        assert_eq!(fs::read(&path).unwrap(), b"{}\r\n");
        assert_unlocked(&path);
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            2,
            "only payload and stable lock may remain"
        );
    }
    #[test]
    fn external_change_or_deletion_after_prepare_is_rejected_without_overwrite() {
        for delete in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("settings.json");
            fs::write(&path, b"{\"x\":0}\n").unwrap();
            let mut writer = Writer::default();
            let id = writer.request(intent(&path, 1, "x", json!(1))).unwrap();
            prepared(&mut writer, id);
            if delete {
                fs::remove_file(&path).unwrap();
            } else {
                fs::write(&path, b"{\"external\":99}\n").unwrap();
            }
            writer.authorize(id).unwrap();
            assert!(
                finished(&mut writer, id)
                    .unwrap_err()
                    .contains("changed externally")
            );
            if delete {
                assert!(!path.exists());
            } else {
                assert_eq!(fs::read(&path).unwrap(), b"{\"external\":99}\n");
            }
            assert_unlocked(&path);
        }
    }
    #[test]
    fn missing_file_is_created_only_after_authorization_and_external_creation_wins() {
        for external in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("new/settings.json");
            let mut writer = Writer::default();
            let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
            assert!(prepared(&mut writer, id).baseline_sha256.is_none());
            assert!(!path.exists());
            if external {
                fs::write(&path, b"{\"external\":1}\r\n").unwrap();
            }
            writer.authorize(id).unwrap();
            let result = finished(&mut writer, id);
            if external {
                assert!(result.is_err());
                assert_eq!(fs::read(&path).unwrap(), b"{\"external\":1}\r\n");
            } else {
                assert!(matches!(result.unwrap(), Outcome::Committed { .. }));
                assert_eq!(
                    crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap()).unwrap()["x"],
                    json!(true)
                );
            }
            assert_unlocked(&path);
        }
    }
    #[test]
    fn malformed_oversize_and_nonregular_payloads_remain_unchanged() {
        for input in [b"{malformed".to_vec(), vec![b' '; FILE_BYTES + 1]] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("settings.json");
            fs::write(&path, &input).unwrap();
            let mut writer = Writer::default();
            let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
            assert!(finished(&mut writer, id).is_err());
            assert_eq!(fs::read(&path).unwrap(), input);
            assert_unlocked(&path);
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::create_dir(&path).unwrap();
        let mut writer = Writer::default();
        let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
        assert!(finished(&mut writer, id).is_err());
        assert!(path.is_dir());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_payload_and_lock_never_redirect_the_write() {
        use std::os::unix::fs::symlink;
        for payload in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("settings.json");
            let other = root.path().join("other.json");
            fs::write(&other, b"{}\n").unwrap();
            if payload {
                symlink(&other, &path).unwrap();
            } else {
                fs::write(&path, b"{}\n").unwrap();
                symlink(&other, root.path().join("settings.json.vscli-write.lock")).unwrap();
            }
            let mut writer = Writer::default();
            let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
            assert!(finished(&mut writer, id).is_err());
            assert_eq!(fs::read(&other).unwrap(), b"{}\n");
            assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        }
    }
    #[test]
    fn latest_same_target_batch_keeps_distinct_unapproved_keys_and_latest_same_key() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let first = writer
            .request(intent(&path, 7, "breadcrumbs.enabled", json!(false)))
            .unwrap();
        prepared(&mut writer, first);
        writer
            .request(intent(&path, 7, "files.autoSave", json!("afterDelay")))
            .unwrap();
        let latest = writer
            .request(intent(&path, 7, "breadcrumbs.enabled", json!(true)))
            .unwrap();
        assert!(writer.authorize(first).is_err());
        assert!(matches!(
            finished(&mut writer, first).unwrap(),
            Outcome::Rejected
        ));
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        prepared(&mut writer, latest);
        writer.authorize(latest).unwrap();
        finished(&mut writer, latest).unwrap();
        let result = crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            result,
            json!({"breadcrumbs.enabled":true,"files.autoSave":"afterDelay"})
        );
    }
    #[test]
    fn profile_round_trip_cannot_revive_retired_uncommitted_keys() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let first = writer
            .request(intent(&path, 1, "old", json!(false)))
            .unwrap();
        prepared(&mut writer, first);
        writer
            .request(intent(&path, 2, "differentProfile", json!(true)))
            .unwrap();
        let latest = writer
            .request(intent(&path, 1, "fresh", json!("on")))
            .unwrap();
        finished(&mut writer, first).unwrap();
        prepared(&mut writer, latest);
        writer.authorize(latest).unwrap();
        finished(&mut writer, latest).unwrap();
        assert_eq!(
            crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap()).unwrap(),
            json!({"fresh":"on"})
        );
    }
    #[test]
    fn validated_queue_bounds_leave_previous_batch_and_identity_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let first = writer.request(intent(&path, 1, "key0", json!(0))).unwrap();
        prepared(&mut writer, first);
        for index in 1..PATCHES {
            writer
                .request(intent(&path, 1, &format!("key{index}"), json!(index)))
                .unwrap();
        }
        let latest = writer.next_id;
        assert!(
            writer
                .request(intent(&path, 1, "overflow", json!(true)))
                .is_err()
        );
        assert_eq!(writer.next_id, latest);
        assert_eq!(writer.queued.as_ref().unwrap().patches.len(), PATCHES);
        finished(&mut writer, first).unwrap();
        prepared(&mut writer, latest);
        writer.authorize(latest).unwrap();
        finished(&mut writer, latest).unwrap();
        assert_eq!(
            crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap())
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            PATCHES
        );
        writer.next_id = u64::MAX;
        assert!(
            writer
                .request(intent(&path, 1, "new", json!(true)))
                .is_err()
        );
        assert!(!writer.busy());
    }
    #[test]
    fn cumulative_serialized_values_reject_before_superseding_valid_authorization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let id = writer
            .request(intent(&path, 1, "first", json!("a".repeat(40 * 1024))))
            .unwrap();
        prepared(&mut writer, id);
        assert!(
            writer
                .request(intent(&path, 1, "second", json!("b".repeat(40 * 1024))))
                .is_err()
        );
        assert!(writer.awaiting_authorization(id));
        assert!(writer.queued.is_none());
        writer.authorize(id).unwrap();
        finished(&mut writer, id).unwrap();
        let result = crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(result.get("second").is_none());
        assert_eq!(result["first"].as_str().unwrap().len(), 40 * 1024);
        assert!(Intent::new(path, 1, "x".into(), json!("\n".repeat(VALUE_BYTES / 2))).is_err());
    }
    #[test]
    fn request_between_actual_settlement_and_queued_dispatch_consumes_latest_batch_once() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let first = writer
            .request(intent(&path, 1, "first", json!(true)))
            .unwrap();
        prepared(&mut writer, first);
        writer
            .request(intent(&path, 1, "second", json!(true)))
            .unwrap();
        finished(&mut writer, first).unwrap();
        assert!(writer.pending.is_none());
        assert!(writer.queued.is_some());
        let latest = writer
            .request(intent(&path, 1, "third", json!(true)))
            .unwrap();
        assert!(writer.queued.is_none());
        prepared(&mut writer, latest);
        writer.authorize(latest).unwrap();
        finished(&mut writer, latest).unwrap();
        assert!(!writer.busy());
        assert_eq!(
            crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap()).unwrap(),
            json!({"first":true,"second":true,"third":true})
        );
    }
    #[test]
    fn authorization_expiry_finishes_before_releasing_capacity_and_cleans_worker_resources() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let batch = Batch {
            id: 1,
            path: path.clone(),
            profile: 1,
            patches: vec![("x".into(), json!(true))],
            models: Vec::new(),
        };
        let mut writer = Writer {
            next_id: 1,
            pending: Some(spawn_with_timeout(batch, Duration::from_millis(1)).unwrap()),
            queued: None,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !writer.pending.as_ref().unwrap().thread.is_finished() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(writer.busy());
        assert!(matches!(
            finished(&mut writer, 1).unwrap(),
            Outcome::Expired
        ));
        assert!(!writer.busy());
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        assert_unlocked(&path);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn replacement_with_identical_bytes_retires_the_prepared_baseline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{\"x\":0}\n").unwrap();
        let mut writer = Writer::default();
        let id = writer.request(intent(&path, 1, "x", json!(1))).unwrap();
        prepared(&mut writer, id);
        let original_handle = open_regular(&path, false).unwrap();
        let original_identity = file_identity(&original_handle).unwrap();
        let mut replacement = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        replacement.write_all(b"{\"x\":0}\n").unwrap();
        replacement.persist(&path).unwrap();
        let replacement_handle = open_regular(&path, false).unwrap();
        assert!(file_identity(&replacement_handle).unwrap() != original_identity);
        assert!(file_identity(&original_handle).unwrap() == original_identity);
        writer.authorize(id).unwrap();
        assert!(finished(&mut writer, id).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{\"x\":0}\n");
        assert_unlocked(&path);
    }
    #[cfg(unix)]
    #[test]
    fn retargeted_requested_parent_refuses_both_original_and_new_settings() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let first_path = first.join("settings.json");
        let second_path = second.join("settings.json");
        fs::write(&first_path, b"{\"source\":1}\n").unwrap();
        fs::write(&second_path, b"{\"source\":2}\n").unwrap();
        let alias = root.path().join("alias");
        symlink(&first, &alias).unwrap();
        let mut writer = Writer::default();
        let id = writer
            .request(intent(&alias.join("settings.json"), 1, "x", json!(true)))
            .unwrap();
        prepared(&mut writer, id);
        fs::remove_file(&alias).unwrap();
        symlink(&second, &alias).unwrap();
        writer.authorize(id).unwrap();
        assert!(
            finished(&mut writer, id)
                .unwrap_err()
                .contains("requested parent changed")
        );
        assert_eq!(fs::read(&first_path).unwrap(), b"{\"source\":1}\n");
        assert_eq!(fs::read(&second_path).unwrap(), b"{\"source\":2}\n");
        assert_unlocked(&first_path);
        assert!(!second.join("settings.json.vscli-write.lock").exists());
    }
    #[cfg(unix)]
    #[test]
    fn replaced_parent_identity_rejects_even_identical_hardlinked_payload_and_lock() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("settings");
        let moved = root.path().join("moved");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
        prepared(&mut writer, id);
        fs::rename(&parent, &moved).unwrap();
        fs::create_dir(&parent).unwrap();
        // Keep the exact payload/lock inodes so only the directory lifetime
        // changed. Matching pathname and payload bytes are insufficient.
        fs::hard_link(moved.join("settings.json"), &path).unwrap();
        fs::hard_link(
            moved.join("settings.json.vscli-write.lock"),
            parent.join("settings.json.vscli-write.lock"),
        )
        .unwrap();
        writer.authorize(id).unwrap();
        assert!(
            finished(&mut writer, id)
                .unwrap_err()
                .contains("directory identity changed")
        );
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        assert_eq!(fs::read(moved.join("settings.json")).unwrap(), b"{}\n");
        assert_unlocked(&path);
    }
    #[cfg(unix)]
    #[test]
    fn replaced_lock_path_refuses_overwrite_while_replacement_lock_is_owned_elsewhere() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let lock_path = root.path().join("settings.json.vscli-write.lock");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
        prepared(&mut writer, id);
        fs::remove_file(&lock_path).unwrap();
        let replacement = open_regular(&lock_path, true).unwrap();
        // A second cooperating writer could acquire this new lock despite
        // the first worker still holding the old, unlinked lock inode.
        replacement.try_lock().unwrap();
        writer.authorize(id).unwrap();
        assert!(
            finished(&mut writer, id)
                .unwrap_err()
                .contains("owned lock")
        );
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        replacement.unlock().unwrap();
        assert_unlocked(&path);
    }
    #[cfg(windows)]
    fn windows_symlink_capability(error: std::io::Error) -> bool {
        if error.raw_os_error() == Some(1314) {
            eprintln!("Windows symlink qualification skipped: privilege unavailable (1314)");
            false
        } else {
            panic!("Windows symlink setup failed unexpectedly: {error}");
        }
    }
    #[cfg(windows)]
    #[test]
    fn windows_symlink_payload_and_lock_are_refused_when_creation_is_permitted() {
        use std::os::windows::fs::symlink_file;
        for payload in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("settings.json");
            let other = root.path().join("other.json");
            fs::write(&other, b"{}\n").unwrap();
            let link = if payload {
                path.clone()
            } else {
                fs::write(&path, b"{}\n").unwrap();
                root.path().join("settings.json.vscli-write.lock")
            };
            if let Err(error) = symlink_file(&other, &link) {
                assert!(!windows_symlink_capability(error));
                return;
            }
            let mut writer = Writer::default();
            let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
            assert!(finished(&mut writer, id).is_err());
            assert_eq!(fs::read(&other).unwrap(), b"{}\n");
            assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        }
    }
    #[cfg(windows)]
    #[test]
    fn windows_retargeted_parent_preserves_both_files_when_symlinks_are_permitted() {
        use std::os::windows::fs::symlink_dir;
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let first_path = first.join("settings.json");
        let second_path = second.join("settings.json");
        fs::write(&first_path, b"{\"source\":1}\n").unwrap();
        fs::write(&second_path, b"{\"source\":2}\n").unwrap();
        let alias = root.path().join("alias");
        if let Err(error) = symlink_dir(&first, &alias) {
            assert!(!windows_symlink_capability(error));
            return;
        }
        let mut writer = Writer::default();
        let id = writer
            .request(intent(&alias.join("settings.json"), 1, "x", json!(true)))
            .unwrap();
        prepared(&mut writer, id);
        fs::remove_dir(&alias).unwrap();
        symlink_dir(&second, &alias).unwrap();
        writer.authorize(id).unwrap();
        assert!(
            finished(&mut writer, id)
                .unwrap_err()
                .contains("requested parent changed")
        );
        assert_eq!(fs::read(&first_path).unwrap(), b"{\"source\":1}\n");
        assert_eq!(fs::read(&second_path).unwrap(), b"{\"source\":2}\n");
        assert_unlocked(&first_path);
    }
    #[test]
    fn read_only_settings_are_not_replaced_even_when_parent_is_writable() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let mut writer = Writer::default();
        let id = writer.request(intent(&path, 1, "x", json!(true))).unwrap();
        assert!(finished(&mut writer, id).unwrap_err().contains("read-only"));
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        assert!(fs::metadata(&path).unwrap().permissions().readonly());
        assert_unlocked(&path);
        fs::set_permissions(&path, permissions).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn model_inventory_detects_third_parent_alias_and_hardlink_without_reading_candidate_bytes() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("real");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let alias_one = root.path().join("alias-one");
        let alias_three = root.path().join("alias-three");
        symlink(&directory, &alias_one).unwrap();
        symlink(&directory, &alias_three).unwrap();
        let hardlink = root.path().join("hardlink.json");
        fs::hard_link(&path, &hardlink).unwrap();
        let other = root.path().join("other.json");
        // Candidate content is deliberately not JSON and exceeds the settings
        // read budget: this inventory resolution must only inspect identity.
        fs::write(&other, vec![b'!'; FILE_BYTES + 1]).unwrap();
        let request = intent(&alias_one.join("settings.json"), 1, "x", json!(true))
            .with_models(vec![
                (31, alias_three.join("settings.json")),
                (7, hardlink.clone()),
                (8, other),
            ])
            .unwrap();
        let mut writer = Writer::default();
        let id = writer.request(request).unwrap();
        assert_eq!(prepared(&mut writer, id).matching_models, vec![31, 7]);
        writer.reject(id);
        assert!(matches!(
            finished(&mut writer, id).unwrap(),
            Outcome::Rejected
        ));
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        assert_eq!(fs::read(&hardlink).unwrap(), b"{}\n");
    }
    #[test]
    fn missing_target_inventory_uses_canonical_existing_parent_and_filename() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let canonical = fs::canonicalize(root.path()).unwrap();
        let request = intent(&path, 1, "x", json!(true))
            .with_models(vec![
                (4, canonical.join("settings.json")),
                (5, canonical.join("other.json")),
            ])
            .unwrap();
        let mut writer = Writer::default();
        let id = writer.request(request).unwrap();
        assert_eq!(prepared(&mut writer, id).matching_models, vec![4]);
        assert!(!path.exists());
        writer.reject(id);
        assert!(matches!(
            finished(&mut writer, id).unwrap(),
            Outcome::Rejected
        ));
        assert!(!path.exists());
    }
    #[test]
    fn model_inventory_validation_rolls_back_and_latest_snapshot_replaces_carried_inventory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let mut writer = Writer::default();
        let first = writer
            .request(
                intent(&path, 1, "first", json!(true))
                    .with_models(vec![(1, path.clone())])
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(prepared(&mut writer, first).matching_models, vec![1]);
        for models in [
            (0..=MODELS).map(|id| (id as u64, path.clone())).collect(),
            vec![(1, path.clone()), (1, path.clone())],
            vec![(1, PathBuf::from("x".repeat(PATH_BYTES + 1)))],
            vec![(1, PathBuf::from("bad\0path"))],
        ] {
            assert!(
                intent(&path, 1, "invalid", json!(true))
                    .with_models(models)
                    .is_err()
            );
            assert!(writer.awaiting_authorization(first));
            assert!(writer.queued.is_none());
            assert_eq!(writer.next_id, first);
        }
        // The public inventory boundary admits its exact byte/count limits
        // without performing filesystem work on those synthetic path strings.
        assert!(
            intent(&path, 1, "bounded", json!(true))
                .with_models(
                    (0..MODELS)
                        .map(|id| (id as u64, PathBuf::from("x".repeat(PATH_BYTES))))
                        .collect()
                )
                .is_ok()
        );
        writer
            .request(
                intent(&path, 1, "second", json!(true))
                    .with_models(vec![(2, path.clone())])
                    .unwrap(),
            )
            .unwrap();
        let latest = writer
            .request(
                intent(&path, 1, "third", json!(true))
                    .with_models(vec![(3, path.clone())])
                    .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            finished(&mut writer, first).unwrap(),
            Outcome::Rejected
        ));
        assert_eq!(prepared(&mut writer, latest).matching_models, vec![3]);
        writer.authorize(latest).unwrap();
        assert!(matches!(
            finished(&mut writer, latest).unwrap(),
            Outcome::Committed { .. }
        ));
        assert_eq!(
            crate::jsonc::parse::<Value>(&fs::read_to_string(&path).unwrap()).unwrap(),
            json!({"first": true, "second": true, "third": true})
        );
    }
    #[test]
    fn unresolved_model_parent_refuses_preparation_and_preserves_target_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        fs::write(&path, b"{}\n").unwrap();
        let request = intent(&path, 1, "x", json!(true))
            .with_models(vec![(
                19,
                root.path().join("deleted-parent").join("old.json"),
            )])
            .unwrap();
        let mut writer = Writer::default();
        let id = writer.request(request).unwrap();
        let error = finished(&mut writer, id).unwrap_err();
        assert!(error.contains("model 19"));
        assert!(error.contains("close the unresolved model"));
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        assert_unlocked(&path);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[cfg(unix)]
    #[test]
    fn candidate_alias_retarget_after_preparation_refuses_authorized_commit() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let other = root.path().join("other.json");
        let alias = root.path().join("model-alias.json");
        fs::write(&path, b"{}\n").unwrap();
        fs::write(&other, b"{\"other\":true}\n").unwrap();
        symlink(&other, &alias).unwrap();
        let request = intent(&path, 1, "x", json!(true))
            .with_models(vec![(27, alias.clone())])
            .unwrap();
        let mut writer = Writer::default();
        let id = writer.request(request).unwrap();
        assert!(prepared(&mut writer, id).matching_models.is_empty());
        fs::remove_file(&alias).unwrap();
        symlink(&path, &alias).unwrap();
        writer.authorize(id).unwrap();
        assert!(
            finished(&mut writer, id)
                .unwrap_err()
                .contains("model aliases changed")
        );
        assert_eq!(fs::read(&path).unwrap(), b"{}\n");
        assert_eq!(fs::read(&other).unwrap(), b"{\"other\":true}\n");
        assert_unlocked(&path);
    }
}
