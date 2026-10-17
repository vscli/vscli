//! Session-owned native extension surfaces. No document contents or arbitrary
//! extension command arguments are stored in this presentation model.
use super::*;
use ropey::Rope;
use std::collections::BTreeMap;

const MAX_CHANNELS: usize = 32;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_CHANNEL_BYTES: usize = 256 * 1024;
const MAX_CHANNEL_LINES: usize = 4096;
const MAX_OUTPUT_LINES: usize = 16384;
const MAX_UPDATE_BYTES: usize = 65536;
const MAX_STATUS: usize = 64;
const MAX_VIEWS: usize = 8;
const MAX_TREE_BYTES: usize = 512 * 1024;
const MAX_NODES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SurfaceKey {
    pub owner: String,
    pub id: String,
}
impl SurfaceKey {
    fn parse(params: &Value) -> Result<Self> {
        Ok(Self {
            owner: string(params, "owner", 201, false)?.into(),
            id: string(params, "id", 256, false)?.into(),
        })
    }
}
pub struct OutputChannel {
    pub key: SurfaceKey,
    pub name: String,
    pub text: Rope,
    updated: u64,
}
#[derive(Clone)]
pub struct StatusItem {
    pub key: SurfaceKey,
    pub name: String,
    pub text: String,
    pub tooltip: String,
    pub alignment: u64,
    pub priority: f64,
    pub visible: bool,
    pub has_command: bool,
    pub generation: u64,
    pub color: String,
    pub background: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TreeNode {
    pub node: String,
    pub label: String,
    pub description: String,
    pub tooltip: String,
    pub collapsible: u8,
    pub has_command: bool,
}
impl TreeNode {
    fn bytes(&self) -> usize {
        self.node.len() + self.label.len() + self.description.len() + self.tooltip.len()
    }
    fn validate(&self) -> Result<()> {
        if self.node.is_empty()
            || self.node.len() > 128
            || self.label.is_empty()
            || self.label.len() > 1024
            || self.description.len() > 2048
            || self.tooltip.len() > 4096
            || self.collapsible > 2
        {
            bail!("Invalid native tree item");
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct TreeView {
    pub key: SurfaceKey,
    pub title: String,
    pub description: String,
    pub message: String,
    pub generation: u64,
    pub nodes: BTreeMap<String, TreeNode>,
    pub children: BTreeMap<Option<String>, Vec<String>>,
    parents: BTreeMap<String, Option<String>>,
}
impl TreeView {
    fn bytes(&self) -> usize {
        self.nodes.values().map(TreeNode::bytes).sum()
    }
    fn depth(&self, node: &str) -> Result<usize> {
        let mut depth = 0;
        let mut current = Some(node);
        while let Some(id) = current {
            depth += 1;
            if depth > 32 {
                bail!("Native tree depth exceeds 32");
            }
            current = self
                .parents
                .get(id)
                .context("Unknown tree parent")?
                .as_deref();
        }
        Ok(depth)
    }
    fn replace_children(&mut self, parent: Option<String>, items: Vec<TreeNode>) -> Result<()> {
        if let Some(parent) = &parent
            && self.depth(parent)? >= 32
        {
            bail!("Native tree depth exceeds 32");
        }
        let mut remove = self.children.remove(&parent).unwrap_or_default();
        while let Some(node) = remove.pop() {
            self.nodes.remove(&node);
            self.parents.remove(&node);
            remove.extend(self.children.remove(&Some(node)).unwrap_or_default());
        }
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            item.validate()?;
            if self.nodes.contains_key(&item.node) {
                bail!("Duplicate or cyclic native tree node");
            }
            self.parents.insert(item.node.clone(), parent.clone());
            ids.push(item.node.clone());
            self.nodes.insert(item.node.clone(), item);
        }
        if self.nodes.len() > MAX_NODES {
            bail!("Native tree exceeds 1024 retained nodes");
        }
        self.children.insert(parent, ids);
        Ok(())
    }
}
#[derive(Clone)]
pub enum SurfaceReveal {
    Output(SurfaceKey, bool),
    HideOutput(SurfaceKey),
}
struct TreePending {
    id: u64,
    key: SurfaceKey,
    generation: u64,
    parent: Option<String>,
}
#[derive(Clone, Default)]
pub struct SurfaceDeclarations(BTreeMap<SurfaceKey, String>);

#[derive(Default)]
pub struct SurfaceState {
    pub generation: u64,
    pub channels: BTreeMap<SurfaceKey, OutputChannel>,
    pub statuses: BTreeMap<SurfaceKey, StatusItem>,
    pub trees: BTreeMap<SurfaceKey, TreeView>,
    pub reveal: std::collections::VecDeque<SurfaceReveal>,
    declarations: BTreeMap<SurfaceKey, String>,
    pending_tree: Option<TreePending>,
}
fn string<'a>(value: &'a Value, key: &str, max: usize, empty: bool) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| s.len() <= max && (empty || !s.is_empty()))
        .with_context(|| format!("Invalid surface {key}"))
}
fn boolean(value: &Value, key: &str) -> Result<bool> {
    value[key]
        .as_bool()
        .with_context(|| format!("Invalid surface {key}"))
}
fn generation(value: &Value) -> Result<u64> {
    value["generation"]
        .as_u64()
        .context("Invalid surface generation")
}
fn keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .context("Surface update must be an object")?;
    if object.keys().any(|key| {
        !["session", "owner", "id", "op", "command", "commandOwner"].contains(&key.as_str())
            && !allowed.contains(&key.as_str())
    }) {
        bail!("Unsupported surface update field");
    }
    Ok(())
}
fn trim_output(text: &mut Rope, bytes: usize) {
    if bytes == 0 {
        return;
    }
    if bytes >= text.len_bytes() {
        *text = Rope::new();
        return;
    }
    let mut chars = text.byte_to_char(bytes);
    if text.char_to_byte(chars) < bytes {
        chars += 1;
    }
    text.remove(..chars);
}
impl SurfaceState {
    pub fn prepare_declarations(packages: &[Package]) -> Result<SurfaceDeclarations> {
        if packages.len() > 8 {
            bail!("Native surface declarations exceed 8 selected packages");
        }
        let mut state = Self::default();
        for package in packages {
            let (_, manifest) = read_manifest(&package.path)?;
            if manifest["version"].as_str() != Some(&package.version)
                || format!(
                    "{}.{}",
                    manifest["publisher"].as_str().unwrap_or(""),
                    manifest["name"].as_str().unwrap_or("")
                )
                .to_ascii_lowercase()
                    != package.id
            {
                bail!("Extension identity changed while loading views");
            }
            let Some(groups) = manifest["contributes"]["views"].as_object() else {
                if !manifest["contributes"]["views"].is_null() {
                    bail!("Invalid contributed views");
                }
                continue;
            };
            let mut count = 0;
            for group in groups.values() {
                for view in group.as_array().context("Invalid contributed views")? {
                    if view["type"].as_str().is_some_and(|kind| kind != "tree") {
                        continue;
                    }
                    count += 1;
                    if count > MAX_VIEWS {
                        bail!("Extension contributes more than 8 native tree views");
                    }
                    let id = string(view, "id", 256, false)?;
                    let name = string(view, "name", 256, false)?;
                    if state
                        .declarations
                        .insert(
                            SurfaceKey {
                                owner: package.id.clone(),
                                id: id.into(),
                            },
                            name.into(),
                        )
                        .is_some()
                    {
                        bail!("Duplicate contributed view ID");
                    }
                }
            }
        }
        Ok(SurfaceDeclarations(state.declarations))
    }
    pub(super) fn stage_declarations(
        &self,
        declarations: &SurfaceDeclarations,
    ) -> Result<SurfaceDeclarations> {
        let additional = declarations
            .0
            .keys()
            .filter(|key| !self.declarations.contains_key(*key))
            .count();
        if self.declarations.len() + additional > 64 {
            bail!("Native surface declarations exceed 64 views");
        }
        if declarations.0.iter().any(|(key, name)| {
            self.declarations
                .get(key)
                .is_some_and(|previous| previous != name)
        }) {
            bail!("Existing surface declaration changed during admission");
        }
        let mut staged = self.declarations.clone();
        staged.extend(declarations.0.clone());
        Ok(SurfaceDeclarations(staged))
    }
    pub(super) fn commit_declarations(&mut self, declarations: SurfaceDeclarations) {
        self.declarations = declarations.0;
    }
    pub fn merge_declarations(&mut self, declarations: SurfaceDeclarations) -> Result<()> {
        let staged = self.stage_declarations(&declarations)?;
        self.commit_declarations(staged);
        Ok(())
    }
    pub(super) fn for_packages(packages: &[Package]) -> Result<Self> {
        let mut state = Self::default();
        state.merge_declarations(Self::prepare_declarations(packages)?)?;
        Ok(state)
    }
    fn apply(&mut self, params: &Value) -> Result<()> {
        let key = SurfaceKey::parse(params)?;
        let op = string(params, "op", 64, false)?;
        match op {
            "outputCreate" => {
                keys(params, &["name"])?;
                let name = string(params, "name", 256, false)?.to_owned();
                if self.channels.contains_key(&key) || self.channels.len() >= MAX_CHANNELS {
                    bail!("Duplicate or excessive output channel");
                }
                self.channels.insert(
                    key.clone(),
                    OutputChannel {
                        key,
                        name,
                        text: Rope::new(),
                        updated: self.generation,
                    },
                );
            }
            "outputAppend" | "outputReplace" => {
                keys(params, &["text"])?;
                let text = string(params, "text", MAX_UPDATE_BYTES, true)?;
                let channel = self
                    .channels
                    .get_mut(&key)
                    .context("Unknown output channel")?;
                if op == "outputReplace" {
                    channel.text = Rope::from_str(text);
                } else {
                    channel.text.insert(channel.text.len_chars(), text);
                }
                channel.updated = self.generation;
                let overflow = channel.text.len_bytes().saturating_sub(MAX_CHANNEL_BYTES);
                trim_output(&mut channel.text, overflow);
                let extra_lines = channel.text.len_lines().saturating_sub(MAX_CHANNEL_LINES);
                if extra_lines > 0 {
                    let end = channel.text.line_to_char(extra_lines);
                    channel.text.remove(..end);
                }
                loop {
                    let bytes: usize = self.channels.values().map(|c| c.text.len_bytes()).sum();
                    let lines: usize = self
                        .channels
                        .values()
                        .map(|c| {
                            if c.text.len_bytes() == 0 {
                                0
                            } else {
                                c.text.len_lines()
                            }
                        })
                        .sum();
                    if bytes <= MAX_OUTPUT_BYTES && lines <= MAX_OUTPUT_LINES {
                        break;
                    }
                    let oldest = self
                        .channels
                        .iter()
                        .filter(|(_, c)| c.text.len_bytes() != 0)
                        .min_by_key(|(_, c)| c.updated)
                        .map(|(key, _)| key.clone())
                        .unwrap();
                    let text = &mut self.channels.get_mut(&oldest).unwrap().text;
                    trim_output(text, bytes.saturating_sub(MAX_OUTPUT_BYTES));
                    let remove = lines.saturating_sub(MAX_OUTPUT_LINES);
                    if remove >= text.len_lines() {
                        *text = Rope::new();
                    } else if remove > 0 {
                        let end = text.line_to_char(remove);
                        text.remove(..end);
                    }
                }
            }
            "outputClear" => {
                keys(params, &[])?;
                self.channels
                    .get_mut(&key)
                    .context("Unknown output channel")?
                    .text = Rope::new();
            }
            "outputShow" => {
                keys(params, &["preserveFocus"])?;
                let preserve = boolean(params, "preserveFocus")?;
                if !self.channels.contains_key(&key) {
                    bail!("Unknown output channel");
                }
                if self.reveal.len() >= 64 {
                    bail!("Native output reveal queue exceeds 64 operations");
                }
                self.reveal.push_back(SurfaceReveal::Output(key, preserve));
            }
            "outputHide" => {
                keys(params, &[])?;
                if self.reveal.len() >= 64 {
                    bail!("Native output reveal queue exceeds 64 operations");
                }
                self.reveal.push_back(SurfaceReveal::HideOutput(key));
            }
            "outputDispose" => {
                keys(params, &[])?;
                self.channels.remove(&key);
            }
            "status" => {
                keys(
                    params,
                    &[
                        "name",
                        "text",
                        "tooltip",
                        "alignment",
                        "priority",
                        "visible",
                        "hasCommand",
                        "generation",
                        "color",
                        "background",
                    ],
                )?;
                let next = StatusItem {
                    key: key.clone(),
                    name: string(params, "name", 256, true)?.into(),
                    text: string(params, "text", 1024, true)?.into(),
                    tooltip: string(params, "tooltip", 4096, true)?.into(),
                    alignment: params["alignment"]
                        .as_u64()
                        .context("Invalid status alignment")?,
                    priority: params["priority"]
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .context("Invalid status priority")?,
                    visible: boolean(params, "visible")?,
                    has_command: boolean(params, "hasCommand")?,
                    generation: generation(params)?,
                    color: string(params, "color", 256, true)?.into(),
                    background: string(params, "background", 256, true)?.into(),
                };
                if ![1, 2].contains(&next.alignment)
                    || ![
                        "",
                        "statusBarItem.errorBackground",
                        "statusBarItem.warningBackground",
                    ]
                    .contains(&next.background.as_str())
                {
                    bail!("Unsupported status appearance");
                }
                if self
                    .statuses
                    .get(&key)
                    .is_some_and(|old| old.generation >= next.generation)
                {
                    bail!("Outdated status update");
                }
                if !self.statuses.contains_key(&key) && self.statuses.len() >= MAX_STATUS {
                    bail!("Status item limit reached");
                }
                self.statuses.insert(key, next);
            }
            "statusDispose" => {
                keys(params, &[])?;
                self.statuses.remove(&key);
            }
            "tree" => {
                keys(params, &["title", "description", "message", "generation"])?;
                if !self.declarations.contains_key(&key) {
                    bail!("Tree view is not contributed by its owner");
                }
                let title = string(params, "title", 256, true)?.to_owned();
                let description = string(params, "description", 256, true)?.to_owned();
                let message = string(params, "message", 4096, true)?.to_owned();
                let generation = generation(params)?;
                if let Some(old) = self.trees.get(&key) {
                    if generation < old.generation {
                        bail!("Outdated tree update");
                    }
                } else if self.trees.len() >= MAX_VIEWS
                    || self.trees.keys().any(|other| other.id == key.id)
                {
                    bail!("Duplicate or excessive native tree view");
                }
                let view = self.trees.entry(key.clone()).or_insert_with(|| TreeView {
                    key,
                    title: String::new(),
                    description: String::new(),
                    message: String::new(),
                    generation,
                    nodes: BTreeMap::new(),
                    children: BTreeMap::new(),
                    parents: BTreeMap::new(),
                });
                if generation != view.generation {
                    view.nodes.clear();
                    view.children.clear();
                    view.parents.clear();
                }
                view.title = title;
                view.description = description;
                view.message = message;
                view.generation = generation;
            }
            "treeDispose" => {
                keys(params, &[])?;
                self.trees.remove(&key);
            }
            _ => bail!("Unsupported native surface operation: {op}"),
        }
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
    /// Remove presentation for a retired/failed owner without releasing an
    /// outstanding request slot. The eventual reply is rejected against the
    /// removed view; unrelated owners and selected declarations remain intact.
    pub fn remove_owner(&mut self, owner: &str) {
        self.channels.retain(|key, _| key.owner != owner);
        self.statuses.retain(|key, _| key.owner != owner);
        self.trees.retain(|key, _| key.owner != owner);
        self.reveal.retain(|reveal| match reveal {
            SurfaceReveal::Output(key, _) | SurfaceReveal::HideOutput(key) => key.owner != owner,
        });
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn tree_busy(&self) -> bool {
        self.pending_tree.is_some()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeReply {
    session: u64,
    owner: String,
    id: String,
    generation: u64,
    items: Vec<TreeNode>,
}
impl Client {
    pub(super) fn surface_update(&mut self, params: Value) -> Result<()> {
        let session = params["session"]
            .as_u64()
            .context("Invalid surface session")?;
        let owner = string(&params, "owner", 201, false)?;
        let command = if params["command"].is_null() {
            None
        } else {
            Some(
                params["command"]
                    .as_u64()
                    .context("Invalid surface origin")?,
            )
        };
        let command_owner = if params["commandOwner"].is_null() {
            None
        } else {
            Some(
                params["commandOwner"]
                    .as_str()
                    .context("Invalid surface origin owner")?,
            )
        };
        self.validate_native_origin(session, owner, command, command_owner)?;
        self.surfaces.apply(&params)
    }
    pub fn surface_tree_children(
        &mut self,
        key: &SurfaceKey,
        parent: Option<String>,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        self.surface_tree_children_with_hidden(key, parent, documents, &[], active, settings)
    }
    pub fn surface_tree_children_with_hidden(
        &mut self,
        key: &SurfaceKey,
        parent: Option<String>,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        if !self.owner_active(&key.owner) {
            bail!("Extension surface owner is not active");
        }
        if self.surfaces.pending_tree.is_some() {
            bail!("A native tree request is still running");
        }
        let view = self.surfaces.trees.get(key).context("Tree view closed")?;
        if let Some(parent) = &parent
            && view.depth(parent)? >= 32
        {
            bail!("Native tree depth exceeds 32");
        }
        let generation = view.generation;
        self.sync_with_hidden(documents, hidden, active)?;
        self.sync_configuration(settings)?;
        let mut params =
            json!({"session":self.session,"owner":key.owner,"id":key.id,"generation":generation});
        if let Some(node) = &parent {
            params["node"] = json!(node);
        }
        self.request("treeChildren", params)?;
        self.surfaces.pending_tree = Some(TreePending {
            id: self.next_id,
            key: key.clone(),
            generation,
            parent,
        });
        Ok(())
    }
    pub(super) fn surface_tree_reply(&mut self, id: u64, message: &mut Value) -> Result<()> {
        let Some(pending) = self.surfaces.pending_tree.take() else {
            return Ok(());
        };
        if pending.id != id {
            self.surfaces.pending_tree = Some(pending);
            return Ok(());
        }
        if !message["error"].is_null() {
            bail!(
                "Native tree provider failed: {}",
                message["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error")
            );
        }
        if message["result"]["items"]
            .as_array()
            .is_some_and(|items| items.len() > 256)
        {
            bail!("Native tree reply exceeds 256 items");
        }
        let reply: TreeReply = serde_json::from_value(message["result"].take())?;
        if reply.session != self.session
            || reply.owner != pending.key.owner
            || reply.id != pending.key.id
            || reply.generation != pending.generation
            || reply.items.len() > 256
        {
            bail!("Invalid or outdated native tree reply");
        }
        let previous = self
            .surfaces
            .trees
            .get(&pending.key)
            .context("Tree view closed while loading")?;
        if previous.generation != pending.generation {
            bail!("Tree view changed while loading");
        }
        let previous_bytes = previous.bytes();
        let mut next = previous.clone();
        next.replace_children(pending.parent, reply.items)?;
        let total: usize = self.surfaces.trees.values().map(TreeView::bytes).sum();
        if total - previous_bytes + next.bytes() > MAX_TREE_BYTES {
            bail!("Native tree text exceeds 512 KiB session budget");
        }
        self.surfaces.trees.insert(pending.key, next);
        self.surfaces.generation = self.surfaces.generation.wrapping_add(1);
        Ok(())
    }
    pub fn surface_action(
        &mut self,
        key: &SurfaceKey,
        generation: u64,
        node: Option<&str>,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        self.surface_action_with_hidden(key, generation, node, documents, &[], active, settings)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn surface_action_with_hidden(
        &mut self,
        key: &SurfaceKey,
        generation: u64,
        node: Option<&str>,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        if !self.owner_active(&key.owner) {
            bail!("Extension surface owner is not active");
        }
        if let Some(node) = node {
            let tree = self.surfaces.trees.get(key).context("Tree view closed")?;
            if tree.generation != generation
                || !tree.nodes.get(node).is_some_and(|item| item.has_command)
            {
                bail!("Tree action changed");
            }
        } else {
            let status = self
                .surfaces
                .statuses
                .get(key)
                .context("Status item closed")?;
            if status.generation != generation || !status.visible || !status.has_command {
                bail!("Status action changed");
            }
        }
        self.sync_with_hidden(documents, hidden, active)?;
        self.sync_configuration(settings)?;
        let mut params = json!({"session":self.session,"owner":key.owner,"id":key.id,"generation":generation,"kind":if node.is_some(){"tree"}else{"status"}});
        if let Some(node) = node {
            params["node"] = json!(node);
        }
        self.request("surfaceAction", params)
    }
    pub fn surface_tree_event(
        &self,
        key: &SurfaceKey,
        generation: u64,
        event: &str,
        node: Option<&str>,
        visible: bool,
    ) -> Result<()> {
        if !self.owner_active(&key.owner) {
            bail!("Extension surface owner is not active");
        }
        let mut params = json!({"session":self.session,"owner":key.owner,"id":key.id,"generation":generation,"event":event,"visible":visible});
        if let Some(node) = node {
            params["node"] = json!(node);
        }
        self.process
            .send(json!({"method":"treeEvent","params":params}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn update(state: &mut SurfaceState, id: &str, op: &str, values: Value) -> Result<()> {
        let mut value = json!({"owner":"fixture.surfaces","id":id,"op":op});
        value
            .as_object_mut()
            .unwrap()
            .extend(values.as_object().unwrap().clone());
        state.apply(&value)
    }
    #[test]
    fn rolling_output_caps_utf8_and_aggregate_without_document_storage() {
        let mut state = SurfaceState::default();
        for index in 0..8 {
            let id = format!("output-{index}");
            update(&mut state, &id, "outputCreate", json!({"name":"log"})).unwrap();
            for _ in 0..8 {
                update(
                    &mut state,
                    &id,
                    "outputAppend",
                    json!({"text":"猫🙂\r\n".repeat(7000)}),
                )
                .unwrap();
            }
            assert!(
                state
                    .channels
                    .values()
                    .all(|c| c.text.len_bytes() <= MAX_CHANNEL_BYTES)
            );
            assert!(
                state
                    .channels
                    .values()
                    .map(|c| c.text.len_bytes())
                    .sum::<usize>()
                    <= MAX_OUTPUT_BYTES
            );
        }
        assert!(
            state
                .channels
                .values()
                .all(|c| c.text.len_lines() <= MAX_CHANNEL_LINES)
        );
        assert!(
            state
                .channels
                .values()
                .filter(|c| c.text.len_bytes() > 0)
                .map(|c| c.text.len_lines())
                .sum::<usize>()
                <= MAX_OUTPUT_LINES
        );
        let before = state
            .channels
            .values()
            .map(|c| c.text.to_string())
            .collect::<Vec<_>>();
        assert!(
            update(
                &mut state,
                "output-7",
                "outputAppend",
                json!({"text":"X".repeat(65537)})
            )
            .is_err()
        );
        assert_eq!(
            before,
            state
                .channels
                .values()
                .map(|c| c.text.to_string())
                .collect::<Vec<_>>()
        );
        update(&mut state, "output-7", "outputClear", json!({})).unwrap();
        assert_eq!(state.channels.values().last().unwrap().text.len_bytes(), 0);
    }
    #[test]
    fn newline_free_output_obeys_aggregate_byte_budget_and_staged_declarations_preserve_caches() {
        let mut state = SurfaceState::default();
        for index in 0..8 {
            let id = format!("output-{index}");
            update(&mut state, &id, "outputCreate", json!({"name":"log"})).unwrap();
            for _ in 0..5 {
                update(
                    &mut state,
                    &id,
                    "outputAppend",
                    json!({"text":"猫".repeat(21000)}),
                )
                .unwrap();
            }
        }
        let bytes: usize = state.channels.values().map(|c| c.text.len_bytes()).sum();
        assert!(bytes <= MAX_OUTPUT_BYTES && bytes > MAX_OUTPUT_BYTES - 3);
        assert!(
            state
                .channels
                .values()
                .all(|c| c.text.len_bytes() <= MAX_CHANNEL_BYTES)
        );
        let key = SurfaceKey {
            owner: "fixture.surfaces".into(),
            id: "tree".into(),
        };
        state
            .merge_declarations(SurfaceDeclarations(BTreeMap::from([(
                key.clone(),
                "original".into(),
            )])))
            .unwrap();
        let before: Vec<_> = state
            .channels
            .values()
            .map(|c| c.text.to_string())
            .collect();
        assert!(
            state
                .merge_declarations(SurfaceDeclarations(BTreeMap::from([(
                    key.clone(),
                    "changed".into()
                )])))
                .is_err()
        );
        assert_eq!(state.declarations[&key], "original");
        assert_eq!(
            before,
            state
                .channels
                .values()
                .map(|c| c.text.to_string())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn owner_retirement_keeps_other_channels_and_retains_pending_request_slot() {
        let mut state = SurfaceState::default();
        update(&mut state, "one", "outputCreate", json!({"name":"one"})).unwrap();
        state
            .apply(&json!({"owner":"another.owner","id":"two","op":"outputCreate","name":"two"}))
            .unwrap();
        update(
            &mut state,
            "one",
            "outputShow",
            json!({"preserveFocus":false}),
        )
        .unwrap();
        state.pending_tree = Some(TreePending {
            id: 1,
            key: SurfaceKey {
                owner: "fixture.surfaces".into(),
                id: "tree".into(),
            },
            generation: 1,
            parent: None,
        });
        state.remove_owner("fixture.surfaces");
        assert!(state.tree_busy());
        assert!(state.reveal.is_empty());
        assert_eq!(state.channels.len(), 1);
        assert_eq!(
            state.channels.values().next().unwrap().key.owner,
            "another.owner"
        );
    }
    #[test]
    fn status_and_tree_updates_reject_stale_or_foreign_data_before_mutation() {
        let mut state = SurfaceState::default();
        let status = json!({"generation":4,"name":"status","text":"ready","tooltip":"plain","alignment":1,"priority":4.0,"visible":true,"hasCommand":true,"color":"","background":""});
        update(&mut state, "status-1", "status", status.clone()).unwrap();
        assert!(update(&mut state, "status-1", "status", status).is_err());
        assert_eq!(state.statuses.values().next().unwrap().generation, 4);
        let tree = json!({"generation":2,"title":"Tree","description":"","message":""});
        assert!(update(&mut state, "undeclared", "tree", tree.clone()).is_err());
        let key = SurfaceKey {
            owner: "fixture.surfaces".into(),
            id: "declared".into(),
        };
        state.declarations.insert(key.clone(), "Tree".into());
        update(&mut state, "declared", "tree", tree).unwrap();
        let node = TreeNode {
            node: "n1".into(),
            label: "猫".into(),
            description: String::new(),
            tooltip: String::new(),
            collapsible: 1,
            has_command: false,
        };
        state
            .trees
            .get_mut(&key)
            .unwrap()
            .replace_children(None, vec![node.clone()])
            .unwrap();
        let mut candidate = state.trees[&key].clone();
        assert!(
            candidate
                .replace_children(Some("n1".into()), vec![node])
                .is_err()
        );
        assert_eq!(state.trees[&key].nodes.len(), 1);
        update(
            &mut state,
            "declared",
            "tree",
            json!({"generation":3,"title":"Refreshed","description":"","message":""}),
        )
        .unwrap();
        assert!(state.trees[&key].nodes.is_empty());
        assert!(
            update(
                &mut state,
                "declared",
                "tree",
                json!({"generation":2,"title":"Stale","description":"","message":""})
            )
            .is_err()
        );
        assert_eq!(state.trees[&key].title, "Refreshed");
    }
}
