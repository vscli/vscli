use super::*;
use std::{
    collections::BTreeSet,
    fs::{self, File},
};

#[derive(Debug)]
pub(super) struct World {
    pub nodes: BTreeMap<PathBuf, NodeImage>,
    pub known: BTreeSet<PathBuf>,
    stable: Vec<ResourceImage>,
    pub handles: BTreeMap<PathBuf, File>,
    limits: Limits,
}
impl World {
    pub fn originals(capture: &Capture) -> Result<Self> {
        let mut stable = Vec::new();
        let mut known = BTreeSet::new();
        for resource in &capture.resources {
            known.insert(resource.canonical.clone());
            known.extend(resource.missing_ancestors.iter().cloned());
            if !capture.nodes.contains_key(&resource.ancestor) {
                if !stable.iter().any(|old: &ResourceImage| {
                    old.ancestor_requested == resource.ancestor_requested
                }) {
                    stable.push(resource.clone());
                }
            } else {
                ensure!(
                    resource.ancestor_requested == resource.ancestor,
                    "Execution refuses parent aliases inside resource trees"
                );
            }
        }
        known.extend(capture.nodes.keys().cloned());
        Ok(Self {
            nodes: capture.nodes.clone(),
            known,
            stable,
            handles: BTreeMap::new(),
            limits: capture.limits,
        })
    }
    pub fn verify(&self, control: &Control<'_>) -> Result<()> {
        self.budget()?;
        for anchor in &self.stable {
            control.check()?;
            ensure!(
                fs::canonicalize(&anchor.ancestor_requested)? == anchor.ancestor
                    && filesystem::identity(&filesystem::open(&anchor.ancestor, true)?)?
                        == anchor.ancestor_identity,
                "Execution parent alias/identity changed"
            );
        }
        for node in self.nodes.values() {
            control.check()?;
            if let Some(handle) = self.handles.get(&node.path) {
                ensure!(
                    filesystem::identity(handle)? == node.identity,
                    "Execution held identity changed"
                );
            }
            capture::verify_node(node, self.limits, control)?;
        }
        for path in &self.known {
            if !self.nodes.contains_key(path) {
                control.check()?;
                ensure!(
                    matches!(fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
                    "Execution absent path appeared externally: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }
    pub fn bind(&mut self, control: &Control<'_>) -> Result<()> {
        self.verify(control)?;
        for node in self.nodes.values() {
            self.handles.insert(
                node.path.clone(),
                filesystem::open(&node.path, node.kind == NodeKind::Directory)?,
            );
        }
        self.verify(control)
    }
    pub fn subtree(&self, root: &Path) -> Vec<NodeImage> {
        self.nodes
            .values()
            .filter(|node| node.path.starts_with(root))
            .cloned()
            .collect()
    }
    fn edge(&mut self, path: &Path, present: bool) -> Result<()> {
        if let Some(parent) = path.parent().and_then(|parent| self.nodes.get_mut(parent)) {
            ensure!(
                parent.kind == NodeKind::Directory,
                "Execution parent is not a directory"
            );
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("Invalid execution name")?
                .to_owned();
            if present {
                parent.children.push(name);
                parent.children.sort();
                parent.children.dedup();
            } else {
                parent.children.retain(|child| child != &name);
            }
        }
        Ok(())
    }
    pub fn add_nodes(&mut self, nodes: &[NodeImage]) -> Result<()> {
        for node in nodes {
            path(&node.path)?;
            ensure!(
                !self.nodes.contains_key(&node.path),
                "Execution node already exists"
            );
            self.known.insert(node.path.clone());
            self.nodes.insert(node.path.clone(), node.clone());
        }
        if let Some(root) = nodes
            .iter()
            .min_by_key(|node| node.path.components().count())
        {
            self.edge(&root.path, true)?;
        }
        self.budget()
    }
    pub fn remove_nodes(&mut self, root: &Path) -> Result<()> {
        let nodes = self.subtree(root);
        ensure!(!nodes.is_empty(), "Execution source proof missing");
        for node in nodes {
            self.nodes.remove(&node.path);
            self.handles.remove(&node.path);
        }
        self.edge(root, false)
    }
    pub fn moved(&mut self, source: &Path, target: &Path) -> Result<()> {
        ensure!(
            !self.nodes.contains_key(target),
            "Execution target must be absent"
        );
        let old = self.subtree(source);
        ensure!(!old.is_empty(), "Execution move source missing");
        let mut moved = Vec::new();
        for mut node in old {
            let new_path = relocated(&node.path, source, target)?;
            if let Some(handle) = self.handles.remove(&node.path) {
                self.handles.insert(new_path.clone(), handle);
            }
            self.nodes.remove(&node.path);
            node.path = new_path;
            moved.push(node);
        }
        self.edge(source, false)?;
        self.add_nodes(&moved)
    }
    pub fn budget(&self) -> Result<()> {
        ensure!(
            self.nodes.len() <= self.limits.max_nodes
                && self.known.len() <= self.limits.max_nodes * 2 + self.limits.max_resources,
            "Execution world exceeds node/path budget"
        );
        let mut bytes = 0;
        let mut metadata = 0;
        for node in self.nodes.values() {
            add(
                &mut bytes,
                node.bytes,
                self.limits.max_bytes,
                "execution live bytes",
            )?;
            add(
                &mut metadata,
                node.path.as_os_str().len() + 256,
                self.limits.max_manifest_bytes,
                "execution metadata",
            )?;
            for name in &node.children {
                add(
                    &mut metadata,
                    name.len() + 16,
                    self.limits.max_manifest_bytes,
                    "execution names",
                )?;
            }
        }
        for path in &self.known {
            add(
                &mut metadata,
                path.as_os_str().len() + 24,
                self.limits.max_manifest_bytes,
                "execution known paths",
            )?;
        }
        Ok(())
    }
    fn fork(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            known: self.known.clone(),
            stable: self.stable.clone(),
            handles: BTreeMap::new(),
            limits: self.limits,
        }
    }
    pub fn preview_move(&self, source: &Path, target: &Path) -> Result<()> {
        self.fork().moved(source, target)
    }
    pub fn preview_copy(&self, source: &Path, target: &Path) -> Result<()> {
        let mut nodes = self.subtree(source);
        for node in &mut nodes {
            node.path = relocated(&node.path, source, target)?;
        }
        // Only count/path/byte planning; these original identities are never
        // persisted or accepted as identities of the future copied nodes.
        self.fork().add_nodes(&nodes)
    }
    pub fn reserve_new(&self, count: usize, bytes: usize, path: &Path) -> Result<()> {
        ensure!(
            count <= self.limits.max_nodes.saturating_sub(self.nodes.len()),
            "Execution new node budget exceeded"
        );
        let mut live = 0;
        let mut metadata = 0;
        for node in self.nodes.values() {
            add(
                &mut live,
                node.bytes,
                self.limits.max_bytes,
                "execution bytes",
            )?;
            add(
                &mut metadata,
                node.path.as_os_str().len() + 256,
                self.limits.max_manifest_bytes,
                "execution metadata",
            )?;
            for name in &node.children {
                add(
                    &mut metadata,
                    name.len() + 16,
                    self.limits.max_manifest_bytes,
                    "execution names",
                )?;
            }
        }
        for known in &self.known {
            add(
                &mut metadata,
                known.as_os_str().len() + 24,
                self.limits.max_manifest_bytes,
                "execution paths",
            )?;
        }
        add(
            &mut live,
            bytes,
            self.limits.max_bytes,
            "execution new bytes",
        )?;
        add(
            &mut metadata,
            path.as_os_str().len() * 2 + 512,
            self.limits.max_manifest_bytes,
            "execution new metadata",
        )?;
        Ok(())
    }
}
