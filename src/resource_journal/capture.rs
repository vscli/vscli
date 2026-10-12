use super::*;
use std::{collections::BTreeSet, fs};

pub(super) fn prepare(
    operations: Vec<Operation>,
    archives: Vec<ModelArchive>,
    limits: Limits,
    control: &Control<'_>,
) -> Result<Capture> {
    limits.validate()?;
    control.check()?;
    ensure!(
        !operations.is_empty() && operations.len() <= limits.max_operations,
        "Resource journal operation budget exceeded"
    );
    ensure!(
        archives.len() <= limits.max_archives,
        "Resource journal archive budget exceeded"
    );
    let mut bytes = 0;
    let mut metadata_bytes = 0;
    let mut requested = BTreeSet::new();
    for operation in &operations {
        control.check()?;
        if let Presence::Value(maximum) = operation.options().max_size {
            ensure!(
                maximum.is_finite(),
                "Resource journal maxSize must be finite"
            );
        }
        for operand in operation.paths() {
            path(operand)?;
            add(
                &mut metadata_bytes,
                operand.as_os_str().len(),
                limits.max_manifest_bytes,
                "path metadata",
            )?;
            requested.insert(operand.to_owned());
            ensure!(
                requested.len() <= limits.max_resources,
                "Resource journal resource budget exceeded"
            );
        }
        if let Operation::Create {
            contents: Some(contents),
            ..
        } = operation
        {
            add(&mut bytes, contents.len(), limits.max_bytes, "raw contents")?;
        }
    }
    let mut documents = BTreeSet::new();
    let mut archive_metadata = 0;
    for archive in &archives {
        control.check()?;
        path(&archive.resource)?;
        add(
            &mut metadata_bytes,
            archive.resource.as_os_str().len() + 128,
            limits.max_manifest_bytes,
            "archive proof",
        )?;
        ensure!(
            archive.proof.document != 0
                && archive.proof.document < (1u64 << 53)
                && documents.insert(archive.proof.document),
            "Invalid or duplicate resource journal model archive"
        );
        add(
            &mut archive_metadata,
            archive.metadata.len(),
            limits.max_metadata_bytes,
            "model metadata",
        )?;
        add(
            &mut bytes,
            archive.text.len(),
            limits.max_bytes,
            "model text",
        )?;
        add(
            &mut bytes,
            archive.metadata.len(),
            limits.max_bytes,
            "model archive",
        )?;
    }
    let mut capture = Capture {
        operations,
        resources: Vec::new(),
        nodes: BTreeMap::new(),
        contents: BTreeMap::new(),
        archives,
        anchors: BTreeMap::new(),
        handles: BTreeMap::new(),
        limits,
        legacy_archive_proofs: false,
    };
    capture
        .resources
        .try_reserve_exact(requested.len())
        .context("Resource journal inventory reservation refused")?;
    for requested in requested {
        let (image, handle) = filesystem::resolve(
            &requested,
            limits.max_manifest_bytes.saturating_sub(metadata_bytes),
            control,
        )?;
        add(
            &mut metadata_bytes,
            image.metadata_bytes(),
            limits.max_manifest_bytes,
            "resource ancestry",
        )?;
        if let Some(old) = capture.anchors.get(&image.ancestor) {
            ensure!(
                filesystem::identity(old)? == image.ancestor_identity,
                "Resource journal parent changed during capture"
            );
        } else {
            capture.anchors.insert(image.ancestor.clone(), handle);
        }
        if image.present {
            tree(
                &mut capture,
                &image.canonical,
                &mut bytes,
                &mut metadata_bytes,
                control,
            )?;
        }
        capture.resources.push(image);
    }
    // Model archive paths must belong to a real operation endpoint or its tree.
    // This prevents an unrelated dirty model from being silently filed away.
    for archive in &capture.archives {
        let (resolved, _) =
            filesystem::resolve(&archive.resource, limits.max_manifest_bytes, control)?;
        ensure!(
            capture
                .resources
                .iter()
                .any(|resource| resolved.canonical == resource.canonical
                    || (capture
                        .nodes
                        .get(&resource.canonical)
                        .is_some_and(|node| node.kind == NodeKind::Directory)
                        && resolved.canonical.starts_with(&resource.canonical))),
            "Model archive is outside resource operands"
        );
    }
    capture.verify_originals(control)?;
    Ok(capture)
}
fn tree(
    capture: &mut Capture,
    root: &Path,
    bytes: &mut usize,
    metadata_bytes: &mut usize,
    control: &Control<'_>,
) -> Result<()> {
    let mut pending = vec![root.to_owned()];
    while let Some(current) = pending.pop() {
        control.check()?;
        if capture.nodes.contains_key(&current) {
            continue;
        }
        ensure!(
            capture.nodes.len() < capture.limits.max_nodes,
            "Resource journal node budget exceeded"
        );
        path(&current)?;
        add(
            metadata_bytes,
            current.as_os_str().len() + 256,
            capture.limits.max_manifest_bytes,
            "node metadata",
        )?;
        let stat = fs::symlink_metadata(&current)?;
        ensure!(
            !stat.file_type().is_symlink() && (stat.is_file() || stat.is_dir()),
            "Resource journal refuses symlink/special tree entries"
        );
        let mut file = filesystem::open(&current, stat.is_dir())?;
        let identity = filesystem::identity(&file)?;
        let metadata = file.metadata()?;
        let (kind, length, sha256, children) = if stat.is_dir() {
            let children = filesystem::children(
                &current,
                capture.limits.max_nodes,
                capture
                    .limits
                    .max_manifest_bytes
                    .saturating_sub(*metadata_bytes),
                control,
            )?;
            ensure!(
                children.len()
                    <= capture
                        .limits
                        .max_nodes
                        .saturating_sub(capture.nodes.len() + pending.len() + 1),
                "Resource journal tree exceeds node budget"
            );
            for child in &children {
                add(
                    metadata_bytes,
                    child.len() + 16,
                    capture.limits.max_manifest_bytes,
                    "tree names",
                )?;
                let next = current.join(child);
                path(&next)?;
                pending
                    .try_reserve(1)
                    .context("Resource journal traversal reservation refused")?;
                pending.push(next);
            }
            (NodeKind::Directory, 0, None, children)
        } else {
            let content = filesystem::read(
                &mut file,
                &current,
                capture.limits.max_bytes - *bytes,
                control,
            )?;
            add(
                bytes,
                content.len(),
                capture.limits.max_bytes,
                "preimage bytes",
            )?;
            let length = content.len();
            let digest = filesystem::digest(&content);
            capture.contents.insert(current.clone(), content);
            (NodeKind::File, length, Some(digest), Vec::new())
        };
        ensure!(
            filesystem::identity(&filesystem::open(&current, stat.is_dir())?)? == identity,
            "Resource journal tree changed during capture"
        );
        capture.nodes.insert(
            current.clone(),
            NodeImage {
                path: current.clone(),
                kind,
                identity,
                permissions: filesystem::permissions(&metadata),
                modified: filesystem::modified(&metadata),
                bytes: length,
                sha256,
                children,
            },
        );
        capture.handles.insert(current, file);
    }
    Ok(())
}

pub(super) fn verify(capture: &Capture, control: &Control<'_>) -> Result<()> {
    for resource in &capture.resources {
        control.check()?;
        let anchor = capture
            .anchors
            .get(&resource.ancestor)
            .context("Resource journal parent handle missing")?;
        ensure!(
            filesystem::identity(anchor)? == resource.ancestor_identity,
            "Resource journal retained parent identity changed"
        );
        verify_resource(resource, control)?;
    }
    for node in capture.nodes.values() {
        control.check()?;
        let held = capture
            .handles
            .get(&node.path)
            .context("Resource journal source handle missing")?;
        ensure!(
            filesystem::identity(held)? == node.identity,
            "Resource journal retained source identity changed"
        );
        verify_node(node, capture.limits, control)?;
    }
    Ok(())
}
pub(super) fn verify_resource(resource: &ResourceImage, control: &Control<'_>) -> Result<()> {
    control.check()?;
    let (resolved, _) = filesystem::resolve(&resource.requested, 2 * 1024 * 1024, control)?;
    ensure!(
        resolved == *resource,
        "Resource journal requested/canonical/ancestry proof changed"
    );
    ensure!(
        fs::canonicalize(&resource.ancestor_requested)? == resource.ancestor,
        "Resource journal requested parent alias changed"
    );
    ensure!(
        filesystem::identity(&filesystem::open(&resource.ancestor, true)?)?
            == resource.ancestor_identity,
        "Resource journal parent identity changed (including ABA)"
    );
    for missing in &resource.missing_ancestors {
        ensure!(
            matches!(fs::symlink_metadata(missing), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
            "Resource journal originally absent parent appeared"
        );
    }
    let current = fs::symlink_metadata(&resource.canonical);
    if resource.present {
        let stat = current?;
        ensure!(
            !stat.file_type().is_symlink() && (stat.is_dir() || stat.is_file()),
            "Resource journal payload was replaced by a symlink/special node"
        );
    } else {
        ensure!(
            matches!(current, Err(error) if error.kind() == std::io::ErrorKind::NotFound),
            "Resource journal missing operand appeared externally"
        );
    }
    Ok(())
}
pub(super) fn verify_node(node: &NodeImage, limits: Limits, control: &Control<'_>) -> Result<()> {
    let directory = node.kind == NodeKind::Directory;
    let mut file = filesystem::open(&node.path, directory)?;
    let metadata = file.metadata()?;
    ensure!(
        filesystem::identity(&file)? == node.identity
            && filesystem::permissions(&metadata) == node.permissions,
        "Resource journal source identity/permissions changed"
    );
    if directory {
        ensure!(
            filesystem::children(
                &node.path,
                limits.max_nodes,
                limits.max_manifest_bytes,
                control
            )? == node.children,
            "Resource journal directory children changed"
        );
    } else {
        ensure!(
            filesystem::modified(&metadata) == node.modified && metadata.len() == node.bytes as u64,
            "Resource journal file signature changed"
        );
        let content = filesystem::read(&mut file, &node.path, node.bytes, control)?;
        ensure!(
            Some(filesystem::digest(&content)) == node.sha256,
            "Resource journal foreign bytes refuse recovery"
        );
    }
    control.check()?;
    Ok(())
}
