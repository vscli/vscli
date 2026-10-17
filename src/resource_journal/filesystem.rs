use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::Read,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn open(path: &Path, directory: bool) -> Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Resource journal refuses payload symlinks"
    );
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
        options.custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 });
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "Resource journal requires ordinary file/directory payloads"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Resource journal refuses reparse points"
        );
    }
    Ok(file)
}
// Reopen the retained object, never a pathname, when Windows requires write
// access to flush metadata. Inspection handles remain read-only and no-follow.
pub(super) fn durability_handle(file: &File) -> Result<File> {
    #[cfg(windows)]
    {
        use std::{
            ffi::c_void,
            os::windows::io::{AsRawHandle, FromRawHandle},
        };
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ReOpenFile(
                original: *mut c_void,
                access: u32,
                share: u32,
                flags: u32,
            ) -> *mut c_void;
        }
        // GENERIC_READ | GENERIC_WRITE; share read/write/delete; retain both
        // backup semantics (directories) and OPEN_REPARSE_POINT (no traversal).
        // SAFETY: the live File owns original. A successful return is a newly
        // owned handle and is immediately put under File's RAII ownership.
        let raw = unsafe { ReOpenFile(file.as_raw_handle(), 0xc0000000, 7, 0x02200000) };
        ensure!(
            raw as isize != -1,
            "Resource journal flush access unavailable: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: ReOpenFile returned a new, valid handle, owned only here.
        let reopened = unsafe { File::from_raw_handle(raw) };
        ensure!(
            identity(&reopened)? == identity(file)?,
            "Resource journal flush identity changed"
        );
        Ok(reopened)
    }
    #[cfg(not(windows))]
    {
        Ok(file.try_clone()?)
    }
}
pub(super) fn sync(file: &File) -> Result<()> {
    durability_handle(file)?.sync_all()?;
    Ok(())
}

pub(super) fn identity(file: &File) -> Result<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(Identity::Unix {
            device: metadata.dev(),
            inode: metadata.ino(),
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
        // SAFETY: File owns the live handle and FileIdInfo fills this initialized
        // repr(C) 24-byte output, matching existing native persistence guards.
        let success = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                0x12,
                (&mut info as *mut IdInfo).cast(),
                std::mem::size_of::<IdInfo>() as u32,
            )
        };
        ensure!(
            success != 0,
            "Windows resource journal identity unavailable: {}",
            std::io::Error::last_os_error()
        );
        Ok(Identity::Windows {
            volume: info.volume,
            file_id: info.id,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        anyhow::bail!("Resource journal identity unsupported on this platform")
    }
}
pub(super) fn permissions(metadata: &Metadata) -> Permissions {
    #[cfg(unix)]
    let unix_mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    };
    #[cfg(not(unix))]
    let unix_mode = None;
    Permissions {
        readonly: metadata.permissions().readonly(),
        unix_mode,
    }
}
fn timestamp(time: SystemTime) -> Option<Timestamp> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => Some(Timestamp {
            seconds: duration.as_secs().try_into().ok()?,
            nanos: duration.subsec_nanos(),
        }),
        Err(error) => {
            let duration = error.duration();
            let seconds: i64 = duration.as_secs().try_into().ok()?;
            Some(if duration.subsec_nanos() == 0 {
                Timestamp {
                    seconds: -seconds,
                    nanos: 0,
                }
            } else {
                Timestamp {
                    seconds: seconds.checked_neg()?.checked_sub(1)?,
                    nanos: 1_000_000_000 - duration.subsec_nanos(),
                }
            })
        }
    }
}
pub(super) fn modified(metadata: &Metadata) -> Option<Timestamp> {
    metadata.modified().ok().and_then(timestamp)
}
pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub(super) fn read(
    file: &mut File,
    path: &Path,
    limit: usize,
    control: &Control<'_>,
) -> Result<Vec<u8>> {
    control.check()?;
    let before = file.metadata()?;
    ensure!(
        before.is_file() && before.len() <= limit as u64,
        "Resource journal file exceeds remaining byte budget"
    );
    let original = identity(file)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(before.len() as usize)
        .context("Resource journal byte reservation refused")?;
    let mut buffer = [0u8; 65536];
    loop {
        control.check()?;
        let count = match file.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        ensure!(
            count <= limit.saturating_sub(bytes.len()),
            "Resource journal file grew beyond byte budget"
        );
        bytes.extend_from_slice(&buffer[..count]);
    }
    let after = file.metadata()?;
    ensure!(
        identity(file)? == original
            && bytes.len() as u64 == before.len()
            && after.len() == before.len()
            && modified(&before) == modified(&after)
            && permissions(&before) == permissions(&after)
            && identity(&open(path, false)?)? == original,
        "Resource journal source changed while capturing"
    );
    control.check()?;
    Ok(bytes)
}
pub(super) fn children(
    path: &Path,
    limit: usize,
    byte_limit: usize,
    control: &Control<'_>,
) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut bytes = 0;
    for entry in fs::read_dir(path)? {
        control.check()?;
        ensure!(
            names.len() < limit,
            "Resource journal directory exceeds node budget"
        );
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Resource journal refuses non-UTF-8 tree names"))?;
        ensure!(
            !name.is_empty() && name.len() <= MAX_PATH_BYTES && !name.contains('\0'),
            "Invalid resource journal tree name"
        );
        add(&mut bytes, name.len() + 16, byte_limit, "tree names")?;
        names
            .try_reserve(1)
            .context("Resource journal child reservation refused")?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}
pub(super) fn resolve(
    requested: &Path,
    metadata_limit: usize,
    control: &Control<'_>,
) -> Result<(ResourceImage, File)> {
    path(requested)?;
    control.check()?;
    let mut cursor = requested
        .parent()
        .context("Resource journal operand requires a parent")?
        .to_owned();
    let mut missing = Vec::new();
    let mut scratch_bytes = requested.as_os_str().len();
    ensure!(
        scratch_bytes <= metadata_limit,
        "Resource journal ancestry metadata exceeds budget"
    );
    loop {
        control.check()?;
        match fs::symlink_metadata(&cursor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor
                    .file_name()
                    .context("Resource journal missing parent cannot be resolved")?
                    .to_owned();
                ensure!(
                    name != "." && name != "..",
                    "Resource journal refuses unresolved parent navigation"
                );
                ensure!(
                    missing.len() < 4096,
                    "Resource journal missing ancestor count exceeds budget"
                );
                add(
                    &mut scratch_bytes,
                    name.len() + 24,
                    metadata_limit,
                    "missing parent names",
                )?;
                missing
                    .try_reserve(1)
                    .context("Resource journal ancestry reservation refused")?;
                missing.push(name);
                cursor = cursor
                    .parent()
                    .context("Resource journal parent cannot be resolved")?
                    .to_owned();
            }
            Err(error) => return Err(error.into()),
        }
    }
    // Parent aliases are canonicalized, as in persistence::destination. The
    // FINAL payload is never canonicalized/followed through a symlink.
    let ancestor = fs::canonicalize(&cursor)?;
    let handle = open(&ancestor, true)?;
    let ancestor_identity = identity(&handle)?;
    let mut canonical = ancestor.clone();
    let mut missing_ancestors = Vec::new();
    let mut image_bytes =
        requested.as_os_str().len() + cursor.as_os_str().len() + ancestor.as_os_str().len() + 256;
    ensure!(
        image_bytes <= metadata_limit,
        "Resource journal ancestry metadata exceeds budget"
    );
    for name in missing.into_iter().rev() {
        canonical.push(name);
        path(&canonical)?;
        add(
            &mut image_bytes,
            canonical.as_os_str().len() + 24,
            metadata_limit,
            "missing ancestor prefixes",
        )?;
        missing_ancestors
            .try_reserve(1)
            .context("Resource journal ancestry reservation refused")?;
        missing_ancestors.push(canonical.clone());
    }
    canonical.push(
        requested
            .file_name()
            .context("Resource journal operand requires a filename")?,
    );
    path(&canonical)?;
    add(
        &mut image_bytes,
        canonical.as_os_str().len(),
        metadata_limit,
        "canonical resource",
    )?;
    let present = match fs::symlink_metadata(&canonical) {
        Ok(metadata) => {
            ensure!(
                !metadata.file_type().is_symlink(),
                "Resource journal refuses payload symlinks"
            );
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    Ok((
        ResourceImage {
            requested: requested.into(),
            canonical,
            ancestor_requested: cursor,
            ancestor,
            ancestor_identity,
            missing_ancestors,
            present,
        },
        handle,
    ))
}
