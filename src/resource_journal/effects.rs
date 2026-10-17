use super::*;
use std::{
    fs::{self, File},
    io::Write,
};

struct Parent {
    path: PathBuf,
    handle: File,
    identity: Identity,
}
impl Parent {
    fn open(path: &Path) -> Result<Self> {
        let parent = path.parent().context("Effect requires parent")?;
        ensure!(
            fs::canonicalize(parent)? == parent,
            "Effect parent changed to an alias"
        );
        let handle = filesystem::open(parent, true)?;
        Ok(Self {
            path: parent.into(),
            identity: filesystem::identity(&handle)?,
            handle,
        })
    }
    fn check(&self) -> Result<()> {
        ensure!(
            fs::canonicalize(&self.path)? == self.path
                && filesystem::identity(&self.handle)? == self.identity
                && filesystem::identity(&filesystem::open(&self.path, true)?)? == self.identity,
            "Effect parent identity changed"
        );
        Ok(())
    }
    fn sync(&self) -> Result<()> {
        self.handle
            .sync_all()
            .context("Effect parent durability unavailable")?;
        self.check()
    }
}
fn missing(path: &Path) -> Result<()> {
    ensure!(
        matches!(fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "Effect requires absent destination: {}",
        path.display()
    );
    Ok(())
}
#[cfg(unix)]
fn name(path: &Path) -> Result<std::ffi::CString> {
    std::ffi::CString::new(
        path.file_name()
            .context("Effect requires filename")?
            .as_encoded_bytes(),
    )
    .context("Invalid effect filename")
}
fn ordinary_file(path: &Path, parent: &Parent) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name = name(path)?;
        // SAFETY: held directory fd and NUL-free filename are live for this
        // synchronous call; O_EXCL prevents replacing a foreign leaf.
        let fd = unsafe {
            libc::openat(
                parent.handle.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                0o666,
            )
        };
        ensure!(
            fd >= 0,
            "Effect create failed: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: successful openat returns one owned fd.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        Ok(options.open(path)?)
    }
}
pub(super) fn create_file(
    path: &Path,
    bytes: &[u8],
    limit: usize,
    control: &Control<'_>,
) -> Result<NodeImage> {
    path_check(path)?;
    control.check()?;
    ensure!(bytes.len() <= limit, "Effect contents exceed bound");
    let parent = Parent::open(path)?;
    parent.check()?;
    missing(path)?;
    let mut file = ordinary_file(path, &parent)?;
    let identity = filesystem::identity(&file)?;
    for chunk in bytes.chunks(65536) {
        control.check()?;
        file.write_all(chunk)?;
    }
    file.sync_all()
        .context("Effect file durability unavailable")?;
    parent.sync()?;
    ensure!(
        filesystem::identity(&filesystem::open(path, false)?)? == identity,
        "Created effect identity changed"
    );
    let metadata = file.metadata()?;
    Ok(NodeImage {
        path: path.into(),
        kind: NodeKind::File,
        identity,
        permissions: filesystem::permissions(&metadata),
        modified: filesystem::modified(&metadata),
        bytes: bytes.len(),
        sha256: Some(filesystem::digest(bytes)),
        children: Vec::new(),
    })
}
pub(super) fn create_directory(path: &Path, control: &Control<'_>) -> Result<NodeImage> {
    path_check(path)?;
    control.check()?;
    let parent = Parent::open(path)?;
    parent.check()?;
    missing(path)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let name = name(path)?;
        // SAFETY: held parent fd and validated filename; mkdirat never replaces
        // an existing leaf and anchors creation to the original parent.
        let status = unsafe { libc::mkdirat(parent.handle.as_raw_fd(), name.as_ptr(), 0o777) };
        ensure!(
            status == 0,
            "Effect mkdir failed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(unix))]
    fs::create_dir(path)?;
    parent.sync()?;
    let handle = filesystem::open(path, true)?;
    handle.sync_all()?;
    let metadata = handle.metadata()?;
    Ok(NodeImage {
        path: path.into(),
        kind: NodeKind::Directory,
        identity: filesystem::identity(&handle)?,
        permissions: filesystem::permissions(&metadata),
        modified: filesystem::modified(&metadata),
        bytes: 0,
        sha256: None,
        children: Vec::new(),
    })
}
pub(super) fn rename(
    source: &Path,
    target: &Path,
    preimage: &[NodeImage],
    limits: Limits,
    control: &Control<'_>,
) -> Result<Vec<NodeImage>> {
    control.check()?;
    path_check(source)?;
    path_check(target)?;
    ensure!(
        !source.starts_with(target) && !target.starts_with(source),
        "Effect refuses overlapping move trees"
    );
    ensure!(!preimage.is_empty(), "Effect source proof missing");
    let from = Parent::open(source)?;
    let to = Parent::open(target)?;
    from.check()?;
    to.check()?;
    missing(target)?;
    ensure!(
        same_volume(&preimage[0].identity, &to.identity),
        "Cross-volume rename/parking has no qualified backend"
    );
    for node in preimage {
        capture::verify_node(node, limits, control)?;
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let old = name(source)?;
        let new = name(target)?;
        // SAFETY: both held directory fds and NUL-free names are live; exclusive
        // rename refuses any foreign destination appearing after validation.
        let status = unsafe {
            libc::renameat2(
                from.handle.as_raw_fd(),
                old.as_ptr(),
                to.handle.as_raw_fd(),
                new.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        ensure!(
            status == 0,
            "Effect rename failed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Exclusive guarded rename backend is currently qualified only on Linux");
    }
    from.sync()?;
    to.sync()?;
    missing(source)?;
    let mut postimage = Vec::new();
    for node in preimage {
        let mut image = node.clone();
        image.path = relocated(&node.path, source, target)?;
        capture::verify_node(&image, limits, control)?;
        postimage.push(image);
    }
    Ok(postimage)
}
pub(super) fn remove_created(
    node: &NodeImage,
    limits: Limits,
    control: &Control<'_>,
) -> Result<()> {
    control.check()?;
    capture::verify_node(node, limits, control)?;
    ensure!(
        node.kind == NodeKind::File || node.children.is_empty(),
        "Rollback directory must remain empty"
    );
    let parent = Parent::open(&node.path)?;
    parent.check()?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let name = name(&node.path)?;
        // SAFETY: held original parent fd/name; directories use AT_REMOVEDIR,
        // which refuses newly appearing children instead of deleting them.
        let status = unsafe {
            libc::unlinkat(
                parent.handle.as_raw_fd(),
                name.as_ptr(),
                if node.kind == NodeKind::Directory {
                    libc::AT_REMOVEDIR
                } else {
                    0
                },
            )
        };
        ensure!(
            status == 0,
            "Rollback unlink failed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(unix))]
    {
        if node.kind == NodeKind::Directory {
            fs::remove_dir(&node.path)?;
        } else {
            fs::remove_file(&node.path)?;
        }
    }
    parent.sync()?;
    missing(&node.path)
}
pub(super) fn same_volume(left: &Identity, right: &Identity) -> bool {
    match (left, right) {
        (Identity::Unix { device: a, .. }, Identity::Unix { device: b, .. }) => a == b,
        (Identity::Windows { volume: a, .. }, Identity::Windows { volume: b, .. }) => a == b,
        _ => false,
    }
}
pub(super) fn copy_tree(
    source: &Path,
    scratch: &Path,
    nodes: &[NodeImage],
    limits: Limits,
    control: &Control<'_>,
) -> Result<Vec<NodeImage>> {
    ensure!(!nodes.is_empty(), "Copy source is absent");
    missing(scratch)?;
    let mut post = Vec::new();
    for original in nodes {
        control.check()?;
        capture::verify_node(original, limits, control)?;
        let target = relocated(&original.path, source, scratch)?;
        path_check(&target)?;
        let image = if original.kind == NodeKind::Directory {
            create_directory(&target, control)?
        } else {
            let mut file = filesystem::open(&original.path, false)?;
            let bytes = filesystem::read(&mut file, &original.path, original.bytes, control)?;
            ensure!(
                Some(filesystem::digest(&bytes)) == original.sha256,
                "Copy source bytes changed"
            );
            create_file(&target, &bytes, limits.max_bytes, control)?
        };
        post.push(image);
    }
    // Apply directory permissions after children, so read-only source trees can
    // be copied without silently omitting their entries.
    for (image, original) in post.iter_mut().zip(nodes).rev() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &image.path,
                fs::Permissions::from_mode(
                    original
                        .permissions
                        .unix_mode
                        .context("Unix permissions missing")?,
                ),
            )?;
        }
        #[cfg(not(unix))]
        {
            let mut permissions = fs::metadata(&image.path)?.permissions();
            permissions.set_readonly(original.permissions.readonly);
            fs::set_permissions(&image.path, permissions)?;
        }
        let handle = filesystem::open(&image.path, image.kind == NodeKind::Directory)?;
        handle.sync_all()?;
        image.permissions = filesystem::permissions(&handle.metadata()?);
        image.modified = filesystem::modified(&handle.metadata()?);
        image.children = original.children.clone();
    }
    for original in nodes {
        capture::verify_node(original, limits, control)?;
    }
    Ok(post)
}
fn path_check(value: &Path) -> Result<()> {
    super::path(value)
}
