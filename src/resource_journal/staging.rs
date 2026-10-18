use super::*;
use crate::persistence::{Expected, Options, ParentPolicy, Target};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Write,
};

const VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Blob {
    name: String,
    bytes: usize,
    sha256: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum StoredOperation {
    Create {
        path: PathBuf,
        options: FileOptions,
        contents: Option<Blob>,
    },
    Move {
        source: PathBuf,
        destination: PathBuf,
        options: FileOptions,
    },
    Delete {
        path: PathBuf,
        options: FileOptions,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredNode {
    image: NodeImage,
    contents: Option<Blob>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArchive {
    proof: ModelProof,
    resource: PathBuf,
    text: Blob,
    metadata: Blob,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    state: ExecutionState,
    journal: PathBuf,
    journal_identity: Identity,
    journal_location: ResourceImage,
    resources: Vec<ResourceImage>,
    operations: Vec<StoredOperation>,
    nodes: Vec<StoredNode>,
    archives: Vec<StoredArchive>,
}
/// Durable ORIGINAL preimages, not an execution receipt. Dropping this owner
/// intentionally leaves the journal for recovery inspection; no resource effect
/// or postimage is implied by successful persistence.
#[derive(Debug)]
pub struct StagedJournal {
    pub(super) directory: PathBuf,
    pub(super) identity: Identity,
    pub(super) directory_handle: File,
    pub(super) location: ResourceImage,
    pub(super) parent_handle: File,
    pub(super) capture: Capture,
}
impl StagedJournal {
    pub fn persist(
        capture: Capture,
        journal_directory: &Path,
        control: &Control<'_>,
    ) -> Result<Self> {
        control.check()?;
        capture.verify_originals(control)?;
        path(journal_directory)?;
        let (mut location, parent_handle) = filesystem::resolve(
            journal_directory,
            capture.limits.max_manifest_bytes,
            control,
        )?;
        ensure!(
            !location.present && location.missing_ancestors.is_empty(),
            "Journal requires a fresh directory with an existing parent"
        );
        outside_operands(&location.canonical, &capture.resources)?;
        ensure!(
            !capture
                .archives
                .iter()
                .any(|archive| archive.resource.starts_with(&location.canonical)),
            "Journal cannot contain archived resources"
        );
        capture::verify_resource(&location, control)?;
        ensure!(
            filesystem::identity(&parent_handle)? == location.ancestor_identity,
            "Journal retained parent identity changed before creation"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700).create(&location.canonical)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&location.canonical)?;
        location.present = true;
        capture::verify_resource(&location, control)?;
        ensure!(
            filesystem::identity(&parent_handle)? == location.ancestor_identity,
            "Journal retained parent identity changed during creation"
        );
        filesystem::sync(&parent_handle)
            .context("Journal parent creation durability unavailable")?;
        let directory_handle = filesystem::open(&location.canonical, true)?;
        let identity = filesystem::identity(&directory_handle)?;
        let staged = Self {
            directory: location.canonical.clone(),
            identity,
            directory_handle,
            location,
            parent_handle,
            capture,
        };
        staged.persist_manifest(control)?;
        staged.verify_originals(control)?;
        Ok(staged)
    }
    fn persist_manifest(&self, control: &Control<'_>) -> Result<()> {
        let mut next = 0;
        let mut store = |bytes: &[u8]| -> Result<Blob> {
            control.check()?;
            self.verify_directory(control)?;
            let name = format!("payload-{next}.bin");
            next += 1;
            write(
                &self.directory.join(&name),
                bytes,
                self.capture.limits.max_bytes,
                control,
            )?;
            Ok(Blob {
                name,
                bytes: bytes.len(),
                sha256: filesystem::digest(bytes),
            })
        };
        let mut operations = Vec::new();
        operations
            .try_reserve_exact(self.capture.operations.len())
            .context("Journal operation reservation refused")?;
        for operation in &self.capture.operations {
            operations.push(match operation {
                Operation::Create {
                    path,
                    options,
                    contents,
                } => StoredOperation::Create {
                    path: path.clone(),
                    options: options.clone(),
                    contents: contents.as_ref().map(|value| store(value)).transpose()?,
                },
                Operation::Move {
                    source,
                    destination,
                    options,
                } => StoredOperation::Move {
                    source: source.clone(),
                    destination: destination.clone(),
                    options: options.clone(),
                },
                Operation::Delete { path, options } => StoredOperation::Delete {
                    path: path.clone(),
                    options: options.clone(),
                },
            });
        }
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(self.capture.nodes.len())
            .context("Journal node reservation refused")?;
        for image in self.capture.nodes.values() {
            nodes.push(StoredNode {
                image: image.clone(),
                contents: self
                    .capture
                    .contents
                    .get(&image.path)
                    .map(|bytes| store(bytes))
                    .transpose()?,
            });
        }
        let mut archives = Vec::new();
        archives
            .try_reserve_exact(self.capture.archives.len())
            .context("Journal archive reservation refused")?;
        for archive in &self.capture.archives {
            archives.push(StoredArchive {
                proof: archive.proof.clone(),
                resource: archive.resource.clone(),
                text: store(&archive.text)?,
                metadata: store(&archive.metadata)?,
            });
        }
        let manifest = Manifest {
            version: VERSION,
            state: ExecutionState::NotStarted,
            journal: self.directory.clone(),
            journal_identity: self.identity.clone(),
            journal_location: self.location.clone(),
            operations,
            resources: self.capture.resources.clone(),
            nodes,
            archives,
        };
        let encoded = encode(&manifest, self.capture.limits.max_manifest_bytes)?;
        control.check()?;
        self.verify_directory(control)?;
        self.capture.verify_originals(control)?;
        write(
            &self.directory.join(MANIFEST),
            &encoded,
            self.capture.limits.max_manifest_bytes,
            control,
        )?;
        filesystem::sync(&self.directory_handle)
            .context("Journal directory durability unavailable")?;
        filesystem::sync(&self.parent_handle).context("Journal parent durability unavailable")?;
        self.verify_directory(control)?;
        Ok(())
    }
    pub(super) fn verify_directory(&self, control: &Control<'_>) -> Result<()> {
        capture::verify_resource(&self.location, control)?;
        ensure!(
            filesystem::identity(&self.parent_handle)? == self.location.ancestor_identity,
            "Journal retained parent identity changed"
        );
        ensure!(
            filesystem::identity(&self.directory_handle)? == self.identity
                && filesystem::identity(&filesystem::open(&self.directory, true)?)?
                    == self.identity,
            "Journal directory identity changed"
        );
        Ok(())
    }
    pub fn path(&self) -> &Path {
        &self.directory
    }
    pub fn state(&self) -> ExecutionState {
        ExecutionState::NotStarted
    }
    pub fn capture(&self) -> &Capture {
        &self.capture
    }
    pub fn verify_originals(&self, control: &Control<'_>) -> Result<()> {
        self.verify_directory(control)?;
        self.capture.verify_originals(control)
    }
    pub fn recovery(&self, control: &Control<'_>) -> Result<RecoveryAuthority> {
        self.verify_directory(control)?;
        let recovery = RecoveryAuthority::load(&self.directory, self.capture.limits, control)?;
        ensure!(
            recovery.capture.operations == self.capture.operations
                && recovery.capture.resources == self.capture.resources
                && recovery.capture.nodes == self.capture.nodes
                && recovery.capture.contents == self.capture.contents
                && recovery.capture.archives == self.capture.archives,
            "Durable journal no longer matches retained original capture"
        );
        Ok(recovery)
    }
}
fn outside_operands(directory: &Path, resources: &[ResourceImage]) -> Result<()> {
    ensure!(
        !resources
            .iter()
            .any(|resource| directory.starts_with(&resource.canonical)
                || resource.canonical.starts_with(directory)),
        "Journal must be outside every operand/descendant, including canonical aliases"
    );
    Ok(())
}
pub(super) fn write(
    path: &Path,
    bytes: &[u8],
    maximum: usize,
    control: &Control<'_>,
) -> Result<()> {
    control.check()?;
    let target = Target::open(
        path,
        Options {
            max_bytes: maximum.clamp(1, MAX_BYTES),
            parent: ParentPolicy::Existing,
            expected: Expected::Missing,
            models: Vec::new(),
        },
    )?;
    let staged = target.stage_bytes(bytes)?;
    control.check()?;
    let receipt = staged.commit()?;
    ensure!(
        receipt.durability_warning.is_none(),
        "Journal durability warning: {:?}",
        receipt.durability_warning
    );
    Ok(())
}
pub(super) fn encode<T: Serialize>(manifest: &T, limit: usize) -> Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other(
                    "Resource journal manifest exceeds budget",
                ));
            }
            self.bytes
                .try_reserve(bytes.len())
                .map_err(std::io::Error::other)?;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, manifest)?;
    Ok(output.bytes)
}

/// Inspection/recovery AUTHORITY for a durable NotStarted journal. Loading
/// validates all schema and binary preimages and refuses any foreign current
/// source/parent state. It deliberately exposes no restore/apply/Undo method.
#[derive(Debug)]
pub struct RecoveryAuthority {
    pub(super) directory: PathBuf,
    pub(super) directory_handle: File,
    pub(super) identity: Identity,
    pub(super) location: ResourceImage,
    pub(super) parent_handle: File,
    pub(super) capture: Capture,
}
impl RecoveryAuthority {
    pub fn load(journal_directory: &Path, limits: Limits, control: &Control<'_>) -> Result<Self> {
        ensure!(
            matches!(std::fs::symlink_metadata(journal_directory.join("execution.json")),Err(error) if error.kind()==std::io::ErrorKind::NotFound),
            "Execution journal requires ExecutionRecovery inspection"
        );
        Self::load_stored(journal_directory, limits, control, true)
    }
    pub(super) fn load_stored(
        journal_directory: &Path,
        limits: Limits,
        control: &Control<'_>,
        verify_live: bool,
    ) -> Result<Self> {
        limits.validate()?;
        control.check()?;
        path(journal_directory)?;
        // A recovery path itself may not be a symlink, even when an operand's
        // requested parent was admitted as an explicitly captured alias.
        let handle = filesystem::open(journal_directory, true)?;
        let identity = filesystem::identity(&handle)?;
        let directory = fs::canonicalize(journal_directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                handle.metadata()?.permissions().mode() & 0o077 == 0,
                "Journal directory is not private"
            );
        }
        let manifest_path = directory.join(MANIFEST);
        let mut file = filesystem::open(&manifest_path, false)?;
        let encoded = filesystem::read(
            &mut file,
            &manifest_path,
            limits.max_manifest_bytes,
            control,
        )?;
        let shape: serde_json::Value =
            serde_json::from_slice(&encoded).context("Malformed resource journal manifest")?;
        let legacy_archive_proofs = shape
            .get("archives")
            .and_then(|archives| archives.as_array())
            .is_some_and(|archives| {
                archives.iter().any(|archive| {
                    archive
                        .get("proof")
                        .and_then(|proof| proof.get("resource_epoch"))
                        .is_none()
                })
            });
        let manifest: Manifest =
            serde_json::from_value(shape).context("Malformed resource journal manifest")?;
        ensure!(
            manifest.version == VERSION && manifest.state == ExecutionState::NotStarted,
            "Unknown resource journal version/execution state"
        );
        ensure!(
            manifest.journal == directory && manifest.journal_identity == identity,
            "Journal directory replacement refuses recovery"
        );
        ensure!(
            !manifest.operations.is_empty()
                && manifest.operations.len() <= limits.max_operations
                && !manifest.resources.is_empty()
                && manifest.resources.len() <= limits.max_resources
                && manifest.nodes.len() <= limits.max_nodes
                && manifest.archives.len() <= limits.max_archives,
            "Resource journal durable inventory exceeds bounds"
        );
        ensure!(
            manifest.journal_location.canonical == directory
                && manifest.journal_location.present
                && manifest.journal_location.missing_ancestors.is_empty(),
            "Invalid journal parent authority"
        );
        capture::verify_resource(&manifest.journal_location, control)?;
        let parent_handle = filesystem::open(&manifest.journal_location.ancestor, true)?;
        validate_images(&manifest, limits, control, verify_live)?;
        outside_operands(&directory, &manifest.resources)?;
        let mut bytes = 0;
        let mut blob_names = BTreeSet::new();
        let mut load = |blob: &Blob| -> Result<Vec<u8>> {
            control.check()?;
            ensure!(
                valid_blob_name(&blob.name) && blob_names.insert(blob.name.clone()),
                "Malformed/duplicate journal payload name"
            );
            add(&mut bytes, blob.bytes, limits.max_bytes, "recovery payload")?;
            let path = directory.join(&blob.name);
            let mut file = filesystem::open(&path, false)?;
            let content = filesystem::read(&mut file, &path, blob.bytes, control)?;
            ensure!(
                content.len() == blob.bytes && filesystem::digest(&content) == blob.sha256,
                "Journal payload digest/length mismatch"
            );
            Ok(content)
        };
        let mut operations = Vec::new();
        operations
            .try_reserve_exact(manifest.operations.len())
            .context("Recovery operation reservation refused")?;
        for operation in manifest.operations {
            operations.push(match operation {
                StoredOperation::Create {
                    path,
                    options,
                    contents,
                } => Operation::Create {
                    path,
                    options,
                    contents: contents.as_ref().map(&mut load).transpose()?,
                },
                StoredOperation::Move {
                    source,
                    destination,
                    options,
                } => Operation::Move {
                    source,
                    destination,
                    options,
                },
                StoredOperation::Delete { path, options } => Operation::Delete { path, options },
            });
        }
        let mut nodes = BTreeMap::new();
        let mut contents = BTreeMap::new();
        for node in manifest.nodes {
            if let Some(blob) = &node.contents {
                ensure!(
                    node.image.kind == NodeKind::File
                        && node.image.sha256 == Some(blob.sha256)
                        && node.image.bytes == blob.bytes,
                    "Journal preimage descriptor mismatch"
                );
                contents.insert(node.image.path.clone(), load(blob)?);
            } else {
                ensure!(
                    node.image.kind == NodeKind::Directory,
                    "Journal file preimage missing"
                );
            }
            ensure!(
                nodes.insert(node.image.path.clone(), node.image).is_none(),
                "Duplicate journal node"
            );
        }
        let mut archives = Vec::new();
        let mut documents = BTreeSet::new();
        let mut metadata = 0;
        for archive in manifest.archives {
            ensure!(
                archive.proof.document != 0
                    && archive.proof.document < (1u64 << 53)
                    && documents.insert(archive.proof.document),
                "Invalid/duplicate journal model archive"
            );
            add(
                &mut metadata,
                archive.metadata.bytes,
                limits.max_metadata_bytes,
                "model metadata",
            )?;
            archives.push(ModelArchive {
                proof: archive.proof,
                resource: archive.resource,
                text: load(&archive.text)?,
                metadata: load(&archive.metadata)?,
            });
        }
        let mut anchors = BTreeMap::new();
        for resource in &manifest.resources {
            if !verify_live {
                continue;
            }
            capture::verify_resource(resource, control)?;
            anchors
                .entry(resource.ancestor.clone())
                .or_insert(filesystem::open(&resource.ancestor, true)?);
        }
        let mut handles = BTreeMap::new();
        for node in nodes.values() {
            if !verify_live {
                continue;
            }
            capture::verify_node(node, limits, control)?;
            handles.insert(
                node.path.clone(),
                filesystem::open(&node.path, node.kind == NodeKind::Directory)?,
            );
        }
        let capture = Capture {
            operations,
            resources: manifest.resources,
            nodes,
            contents,
            archives,
            anchors,
            handles,
            limits,
            legacy_archive_proofs,
        };
        if verify_live {
            capture.verify_originals(control)?;
        }
        let recovery = Self {
            directory,
            directory_handle: handle,
            identity,
            location: manifest.journal_location,
            parent_handle,
            capture,
        };
        if verify_live {
            recovery.verify_originals(control)?;
        }
        Ok(recovery)
    }
    pub fn path(&self) -> &Path {
        &self.directory
    }
    pub fn state(&self) -> ExecutionState {
        ExecutionState::NotStarted
    }
    pub fn capture(&self) -> &Capture {
        &self.capture
    }
    pub fn verify_originals(&self, control: &Control<'_>) -> Result<()> {
        capture::verify_resource(&self.location, control)?;
        ensure!(
            filesystem::identity(&self.parent_handle)? == self.location.ancestor_identity,
            "Recovery journal parent identity changed"
        );
        ensure!(
            filesystem::identity(&self.directory_handle)? == self.identity
                && filesystem::identity(&filesystem::open(&self.directory, true)?)?
                    == self.identity,
            "Recovery journal identity changed"
        );
        self.capture.verify_originals(control)
    }
}
fn valid_blob_name(name: &str) -> bool {
    name.strip_prefix("payload-")
        .and_then(|value| value.strip_suffix(".bin"))
        .is_some_and(|number| {
            !number.is_empty()
                && number.len() <= 10
                && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}
fn validate_images(
    manifest: &Manifest,
    limits: Limits,
    control: &Control<'_>,
    verify_live: bool,
) -> Result<()> {
    let mut requested = BTreeSet::new();
    let mut resource_metadata = 0;
    let mut nodes = BTreeMap::new();
    for resource in &manifest.resources {
        add(
            &mut resource_metadata,
            resource.metadata_bytes(),
            limits.max_manifest_bytes,
            "resource ancestry",
        )?;
        if verify_live {
            capture::verify_resource(resource, control)?;
        }
        for value in [
            &resource.requested,
            &resource.canonical,
            &resource.ancestor_requested,
            &resource.ancestor,
        ] {
            // Existing anchors can be filesystem roots; operands cannot.
            if value == &resource.ancestor || value == &resource.ancestor_requested {
                ensure!(
                    value.is_absolute()
                        && value.to_str().is_some()
                        && value.as_os_str().len() <= MAX_PATH_BYTES,
                    "Invalid journal anchor path"
                );
            } else {
                path(value)?;
            }
        }
        ensure!(
            requested.insert(resource.requested.clone())
                && resource.canonical.starts_with(&resource.ancestor)
                && resource.canonical != resource.ancestor,
            "Invalid resource journal canonical inventory"
        );
        for missing in &resource.missing_ancestors {
            path(missing)?;
            ensure!(
                missing.starts_with(&resource.ancestor)
                    && resource.canonical.starts_with(missing)
                    && missing != &resource.ancestor
                    && missing != &resource.canonical,
                "Invalid journal missing ancestor"
            );
        }
    }
    for node in &manifest.nodes {
        path(&node.image.path)?;
        ensure!(
            nodes.insert(node.image.path.clone(), &node.image).is_none(),
            "Duplicate journal node path"
        );
        ensure!(
            manifest.resources.iter().any(
                |resource| resource.present && node.image.path.starts_with(&resource.canonical)
            ),
            "Journal node outside resource trees"
        );
        ensure!(
            node.image
                .modified
                .as_ref()
                .is_none_or(|stamp| stamp.nanos < 1_000_000_000),
            "Invalid journal timestamp"
        );
        ensure!(
            node.image.children.len() <= limits.max_nodes
                && node.image.children.windows(2).all(|pair| pair[0] < pair[1]),
            "Invalid journal tree ordering"
        );
        for name in &node.image.children {
            ensure!(
                !name.is_empty()
                    && name.len() <= MAX_PATH_BYTES
                    && !name.contains(['/', '\0'])
                    && (!cfg!(windows) || !name.contains('\\'))
                    && name != "."
                    && name != "..",
                "Malformed journal tree child"
            );
        }
        match node.image.kind {
            NodeKind::File => ensure!(
                node.image.children.is_empty()
                    && node.image.bytes <= limits.max_bytes
                    && node.image.sha256.is_some()
                    && node.contents.is_some(),
                "Invalid journal file descriptor"
            ),
            NodeKind::Directory => ensure!(
                node.image.bytes == 0 && node.image.sha256.is_none() && node.contents.is_none(),
                "Invalid journal directory descriptor"
            ),
        }
    }
    for resource in &manifest.resources {
        ensure!(
            nodes.contains_key(&resource.canonical) == resource.present,
            "Journal operand preimage missing/unexpected"
        );
    }
    for node in nodes.values() {
        if !manifest
            .resources
            .iter()
            .any(|resource| resource.canonical == node.path)
        {
            let parent = node
                .path
                .parent()
                .and_then(|value| nodes.get(value))
                .context("Journal tree parent preimage missing")?;
            let name = node
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .context("Invalid journal tree name")?;
            ensure!(
                parent.kind == NodeKind::Directory
                    && parent.children.iter().any(|child| child == name),
                "Journal tree node has no declared original parent edge"
            );
        }
        for name in &node.children {
            ensure!(
                nodes.contains_key(&node.path.join(name)),
                "Journal tree child preimage missing"
            );
        }
    }
    let mut operands = BTreeSet::new();
    for operation in &manifest.operations {
        let (paths, options) = match operation {
            StoredOperation::Create { path, options, .. }
            | StoredOperation::Delete { path, options } => (vec![path], options),
            StoredOperation::Move {
                source,
                destination,
                options,
            } => (vec![source, destination], options),
        };
        ensure!(
            options
                .max_size
                .value()
                .is_none_or(|value| value.is_finite()),
            "Invalid journal maxSize"
        );
        for value in paths {
            path(value)?;
            operands.insert(value.to_owned());
            ensure!(
                requested.contains(value),
                "Journal operation has no original resource proof"
            );
        }
    }
    ensure!(
        operands == requested,
        "Journal inventory does not equal original operation operands"
    );
    for archive in &manifest.archives {
        path(&archive.resource)?;
        // Capture itself checked canonical archive inclusion. Recovery repeats it
        // against live original parents, never trusts an arbitrary archive path.
        if !verify_live {
            continue;
        }
        let (image, _) =
            filesystem::resolve(&archive.resource, limits.max_manifest_bytes, control)?;
        ensure!(
            manifest
                .resources
                .iter()
                .any(|resource| image.canonical == resource.canonical
                    || (nodes
                        .get(&resource.canonical)
                        .is_some_and(|node| node.kind == NodeKind::Directory)
                        && image.canonical.starts_with(&resource.canonical))),
            "Journal archive outside operand trees"
        );
    }
    Ok(())
}
