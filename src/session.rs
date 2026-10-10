//! Clean-file layout metadata. Each process owns a leased slot; all I/O runs on one worker.
use crate::document::{Document, Selection};
use crate::editor_layout::{Axis, SavedNode, validate_saved_layout};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub const MAX_DOCUMENTS: usize = 32;
pub const MAX_VIEWS: usize = 4;
pub const MAX_SELECTIONS: usize = 128;
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_READ_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SLOTS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedSelection {
    pub cursor: Position,
    pub anchor: Option<Position>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub selections: Vec<SavedSelection>,
    pub top: usize,
    pub left: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedFile {
    pub path: PathBuf,
    pub view: View,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    pub file: usize,
    pub view: View,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub file: usize,
    pub view: View,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub recent: Vec<usize>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub files: Vec<SavedFile>,
    pub panes: Vec<Pane>,
    pub active_file: usize,
    pub active_pane: usize,
    pub horizontal: bool,
    #[serde(default)]
    pub groups: Option<Vec<Group>>,
    #[serde(default)]
    pub active_group: usize,
    /// Normalized metadata only; schema-1 and schema-2 DTOs never accept these fields.
    #[serde(default)]
    pub tree: Option<SavedNode>,
    #[serde(default)]
    pub sticky: Option<Vec<Vec<bool>>>,
}
impl Layout {
    pub fn validate(&self) -> Result<()> {
        self.validate_structure()?;
        if self.groups.is_some() {
            // Future publication admission is distinct from legacy read
            // admission: an already bounded old slot remains recoverable even
            // if adding schema-3 mode/tree fields would exceed the write cap.
            serde_json::to_writer(MetadataBudget(0), &NestedLayoutRef::new(self)?)
                .context("Session metadata exceeds 1 MiB")?;
        }
        Ok(())
    }
    fn validate_structure(&self) -> Result<()> {
        if self.files.len() > MAX_DOCUMENTS || self.panes.len() > MAX_VIEWS {
            bail!("Session exceeds 32 files or four panes");
        }
        let mut paths = std::collections::HashSet::new();
        for file in &self.files {
            if !file.path.is_absolute()
                || file.path.as_os_str().len() > 4096
                || file.path.as_os_str().as_encoded_bytes().contains(&0)
                || !paths.insert(&file.path)
            {
                bail!("Invalid or duplicate session file path");
            }
            file.view.validate()?;
        }
        if self.files.is_empty() {
            if !self.panes.is_empty() || self.active_file != 0 || self.active_pane != 0 {
                bail!("Invalid empty session");
            }
        } else if self.active_file >= self.files.len()
            || self.panes.is_empty()
            || self.active_pane >= self.panes.len()
            || self.panes[self.active_pane].file != self.active_file
        {
            bail!("Invalid active session file or pane");
        }
        for pane in &self.panes {
            if pane.file >= self.files.len() {
                bail!("Invalid session pane file");
            }
            pane.view.validate()?;
        }
        if let Some(groups) = &self.groups {
            validate_groups(&self.files, groups, self.active_group)?;
            if self.active_pane != self.active_group
                || self.panes.len() != groups.len()
                || self.panes.iter().zip(groups).any(|(pane, group)| {
                    let tab = &group.tabs[group.active];
                    pane.file != tab.file || pane.view != tab.view
                })
            {
                bail!("Session group compatibility projection changed");
            }
        } else if self.active_group != 0 {
            bail!("Legacy session has no active group");
        }
        match &self.groups {
            Some(groups) => {
                if self.tree.is_some() {
                    validate_saved_layout(self.tree.as_ref(), groups.len())?;
                }
                if let Some(sticky) = &self.sticky {
                    validate_sticky(groups, sticky)?;
                }
            }
            None if self.tree.is_some() || self.sticky.is_some() => {
                bail!("Legacy session has unexpected layout mode metadata");
            }
            None => {}
        }
        Ok(())
    }
    /// Upgrade legacy inventory in memory. Reading never rewrites its source slot.
    pub fn normalized(&self) -> Result<Self> {
        self.validate_structure()?;
        if self.groups.is_some() {
            let mut normalized = self.clone();
            normalized.fill_layout_metadata();
            normalized.project_file_views();
            normalized.validate_structure()?;
            return Ok(normalized);
        }
        let mut groups = Vec::new();
        if let Some(first) = self.panes.first() {
            let tabs = self
                .files
                .iter()
                .enumerate()
                .map(|(file, saved)| Tab {
                    file,
                    view: if file == first.file {
                        first.view.clone()
                    } else {
                        saved.view.clone()
                    },
                })
                .collect::<Vec<_>>();
            let recent = std::iter::once(first.file)
                .chain((0..tabs.len()).filter(|file| *file != first.file))
                .collect();
            groups.push(Group {
                tabs,
                active: first.file,
                recent,
            });
            groups.extend(self.panes.iter().skip(1).map(|pane| Group {
                tabs: vec![Tab {
                    file: pane.file,
                    view: pane.view.clone(),
                }],
                active: 0,
                recent: vec![0],
            }));
        }
        let mut normalized = self.clone();
        normalized.groups = Some(groups);
        normalized.active_group = self.active_pane;
        normalized.fill_layout_metadata();
        normalized.project_file_views();
        normalized.validate_structure()?;
        Ok(normalized)
    }
    fn fill_layout_metadata(&mut self) {
        let groups = self.groups.as_ref().expect("normalized groups");
        if self.tree.is_none() {
            self.tree = flat_saved_layout(
                groups.len(),
                if self.horizontal {
                    Axis::Rows
                } else {
                    Axis::Columns
                },
            );
        }
        if self.sticky.is_none() {
            self.sticky = Some(
                groups
                    .iter()
                    .map(|group| vec![false; group.tabs.len()])
                    .collect(),
            );
        }
    }
    fn project_file_views(&mut self) {
        let mut assigned = [false; MAX_DOCUMENTS];
        for group in self.groups.as_ref().expect("normalized groups") {
            for tab in &group.tabs {
                if !assigned[tab.file] {
                    self.files[tab.file].view = tab.view.clone();
                    assigned[tab.file] = true;
                }
            }
        }
    }
}
// Direct wire indices avoid creating artificial runtime GroupIds while reading
// old metadata. Equal same-axis leaves retain the legacy flat layout semantics.
fn flat_saved_layout(count: usize, axis: Axis) -> Option<SavedNode> {
    fn build(index: usize, count: usize, axis: Axis) -> SavedNode {
        if count == 1 {
            SavedNode::Leaf { group: index }
        } else {
            SavedNode::Split {
                axis,
                first_weight: 1,
                second_weight: (count - 1) as u32,
                first: Box::new(SavedNode::Leaf { group: index }),
                second: Box::new(build(index + 1, count - 1, axis)),
            }
        }
    }
    (count > 0).then(|| build(0, count, axis))
}
fn validate_sticky(groups: &[Group], sticky: &[Vec<bool>]) -> Result<()> {
    if sticky.len() != groups.len() {
        bail!("Session sticky group inventory changed");
    }
    for (group, flags) in groups.iter().zip(sticky) {
        if flags.len() != group.tabs.len() || flags.windows(2).any(|pair| !pair[0] && pair[1]) {
            bail!("Invalid session sticky prefix");
        }
    }
    Ok(())
}
#[derive(Serialize)]
struct TabV3Ref<'a> {
    file: usize,
    view: &'a View,
    sticky: bool,
}
#[derive(Serialize)]
struct GroupV3Ref<'a> {
    tabs: Vec<TabV3Ref<'a>>,
    active: usize,
    recent: &'a [usize],
}
#[derive(Serialize)]
struct NestedLayoutRef<'a> {
    files: Vec<&'a Path>,
    groups: Vec<GroupV3Ref<'a>>,
    active_group: usize,
    horizontal: bool,
    tree: Option<&'a SavedNode>,
}
impl<'a> NestedLayoutRef<'a> {
    fn new(layout: &'a Layout) -> Result<Self> {
        let groups = layout
            .groups
            .as_ref()
            .context("Normalized session groups missing")?;
        Ok(Self {
            files: layout
                .files
                .iter()
                .map(|file| file.path.as_path())
                .collect(),
            groups: groups
                .iter()
                .enumerate()
                .map(|(index, group)| GroupV3Ref {
                    tabs: group
                        .tabs
                        .iter()
                        .enumerate()
                        .map(|(tab, saved)| TabV3Ref {
                            file: saved.file,
                            view: &saved.view,
                            sticky: layout
                                .sticky
                                .as_ref()
                                .is_some_and(|flags| flags[index][tab]),
                        })
                        .collect(),
                    active: group.active,
                    recent: &group.recent,
                })
                .collect(),
            active_group: layout.active_group,
            horizontal: layout.horizontal,
            tree: layout.tree.as_ref(),
        })
    }
}
struct MetadataBudget(usize);
impl Write for MetadataBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > (MAX_BYTES as usize).saturating_sub(self.0) {
            return Err(std::io::Error::other("Session metadata exceeds 1 MiB"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn validate_groups(files: &[SavedFile], groups: &[Group], active: usize) -> Result<()> {
    if groups.len() > MAX_VIEWS
        || (groups.is_empty() && (!files.is_empty() || active != 0))
        || (!groups.is_empty() && active >= groups.len())
    {
        bail!("Invalid session group count or active group");
    }
    let mut referenced = [false; MAX_DOCUMENTS];
    for group in groups {
        if group.tabs.is_empty()
            || group.tabs.len() > MAX_DOCUMENTS
            || group.active >= group.tabs.len()
            || group.recent.len() != group.tabs.len()
            || group.recent.first() != Some(&group.active)
        {
            bail!("Invalid session group tabs, active tab or recent order");
        }
        let mut membership = [false; MAX_DOCUMENTS];
        let mut recent = [false; MAX_DOCUMENTS];
        for tab in &group.tabs {
            if tab.file >= files.len() || membership[tab.file] {
                bail!("Invalid or duplicate session group file");
            }
            membership[tab.file] = true;
            referenced[tab.file] = true;
            tab.view.validate()?;
        }
        for &tab in &group.recent {
            if tab >= group.tabs.len() || recent[tab] {
                bail!("Invalid session recent-tab permutation");
            }
            recent[tab] = true;
        }
    }
    if referenced[..files.len()].iter().any(|used| !used) {
        bail!("Session file has no group membership");
    }
    Ok(())
}
impl View {
    pub fn capture(doc: &Document, view: &crate::document::ViewState) -> Result<Self> {
        if view.secondary.len() >= MAX_SELECTIONS {
            bail!("Session view exceeds 128 selections");
        }
        let position = |value: usize| {
            let value = value.min(doc.len());
            let line = doc.text.char_to_line(value);
            Position {
                line,
                character: value - doc.text.line_to_char(line),
            }
        };
        let mut selections = vec![SavedSelection {
            cursor: position(view.cursor),
            anchor: view.anchor.map(position),
        }];
        selections.extend(view.secondary.iter().map(|s| SavedSelection {
            cursor: position(s.cursor),
            anchor: s.anchor.map(position),
        }));
        let saved = Self {
            selections,
            top: view.top,
            left: view.left,
        };
        saved.validate()?;
        Ok(saved)
    }
    fn validate(&self) -> Result<()> {
        if self.selections.is_empty() || self.selections.len() > MAX_SELECTIONS {
            bail!("Session view exceeds 128 selections or has no cursor");
        }
        Ok(())
    }
    pub fn apply(&self, doc: &mut Document) {
        let position = |p: &Position| {
            let row = p.line.min(doc.line_count() - 1);
            doc.line_start(row) + p.character.min(doc.line_end(row) - doc.line_start(row))
        };
        let selections = self
            .selections
            .iter()
            .map(|s| Selection {
                cursor: position(&s.cursor),
                anchor: s.anchor.as_ref().map(position),
                desired_column: None,
            })
            .collect();
        doc.set_selections(selections);
        doc.top = self.top.min(doc.line_count() - 1);
        doc.left = self.left.min(crate::document::MAX_FILE_BYTES as usize);
    }
}
struct Saved {
    workspace: PathBuf,
    stamp: u64,
    layout: Layout,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyLayout {
    files: Vec<SavedFile>,
    panes: Vec<Pane>,
    active_file: usize,
    active_pane: usize,
    horizontal: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedV1 {
    schema: u32,
    workspace: PathBuf,
    stamp: u64,
    layout: LegacyLayout,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupLayout {
    files: Vec<PathBuf>,
    groups: Vec<Group>,
    active_group: usize,
    horizontal: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedV2 {
    schema: u32,
    workspace: PathBuf,
    stamp: u64,
    layout: GroupLayout,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TabV3 {
    file: usize,
    view: View,
    sticky: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupV3 {
    tabs: Vec<TabV3>,
    active: usize,
    recent: Vec<usize>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NestedGroupLayoutV3 {
    files: Vec<PathBuf>,
    groups: Vec<GroupV3>,
    active_group: usize,
    horizontal: bool,
    #[serde(deserialize_with = "deserialize_saved_tree")]
    tree: Option<SavedNode>,
}
fn deserialize_saved_tree<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<SavedNode>, D::Error> {
    Option::<SavedNode>::deserialize(deserializer)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedV3 {
    schema: u32,
    workspace: PathBuf,
    stamp: u64,
    layout: NestedGroupLayoutV3,
}
impl NestedGroupLayoutV3 {
    fn from_layout(layout: &Layout) -> Result<Self> {
        let layout = layout.normalized()?;
        let groups = layout.groups.expect("normalized groups");
        let flags = layout.sticky.expect("normalized sticky flags");
        Ok(Self {
            files: layout.files.into_iter().map(|file| file.path).collect(),
            groups: groups
                .into_iter()
                .zip(flags)
                .map(|(group, flags)| GroupV3 {
                    tabs: group
                        .tabs
                        .into_iter()
                        .zip(flags)
                        .map(|(tab, sticky)| TabV3 {
                            file: tab.file,
                            view: tab.view,
                            sticky,
                        })
                        .collect(),
                    active: group.active,
                    recent: group.recent,
                })
                .collect(),
            active_group: layout.active_group,
            horizontal: layout.horizontal,
            tree: layout.tree,
        })
    }
    fn into_layout(self) -> Result<Layout> {
        if self.files.len() > MAX_DOCUMENTS || self.groups.len() > MAX_VIEWS {
            bail!("Session exceeds 32 files or four groups");
        }
        // Validate all tree/mode metadata before building compatibility state or
        // opening/configuring any files. No lower-version parser is involved.
        validate_saved_layout(self.tree.as_ref(), self.groups.len())?;
        let mut sticky = Vec::with_capacity(self.groups.len());
        let mut groups = Vec::with_capacity(self.groups.len());
        for group in self.groups {
            if group.tabs.is_empty() || group.tabs.len() > MAX_DOCUMENTS {
                bail!("Invalid session group tabs, active tab or recent order");
            }
            let flags = group.tabs.iter().map(|tab| tab.sticky).collect::<Vec<_>>();
            if flags.windows(2).any(|pair| !pair[0] && pair[1]) {
                bail!("Invalid session sticky prefix");
            }
            sticky.push(flags);
            groups.push(Group {
                tabs: group
                    .tabs
                    .into_iter()
                    .map(|tab| Tab {
                        file: tab.file,
                        view: tab.view,
                    })
                    .collect(),
                active: group.active,
                recent: group.recent,
            });
        }
        let mut layout = GroupLayout {
            files: self.files,
            groups,
            active_group: self.active_group,
            horizontal: self.horizontal,
        }
        .into_layout()?;
        layout.tree = self.tree;
        layout.sticky = Some(sticky);
        layout.validate()?;
        Ok(layout)
    }
}
impl GroupLayout {
    #[cfg(test)]
    fn from_layout(layout: &Layout) -> Result<Self> {
        let layout = layout.normalized()?;
        Ok(Self {
            files: layout.files.into_iter().map(|file| file.path).collect(),
            groups: layout.groups.expect("normalized groups"),
            active_group: layout.active_group,
            horizontal: layout.horizontal,
        })
    }
    fn into_layout(self) -> Result<Layout> {
        // Check counts before indexing/allocating derived compatibility state.
        if self.files.len() > MAX_DOCUMENTS || self.groups.len() > MAX_VIEWS {
            bail!("Session exceeds 32 files or four groups");
        }
        let files = self
            .files
            .into_iter()
            .map(|path| SavedFile {
                path,
                view: View {
                    selections: vec![SavedSelection {
                        cursor: Position {
                            line: 0,
                            character: 0,
                        },
                        anchor: None,
                    }],
                    top: 0,
                    left: 0,
                },
            })
            .collect::<Vec<_>>();
        validate_groups(&files, &self.groups, self.active_group)?;
        let mut layout = Layout {
            files,
            panes: Vec::new(),
            active_file: 0,
            active_pane: self.active_group,
            horizontal: self.horizontal,
            groups: Some(self.groups),
            active_group: self.active_group,
            tree: None,
            sticky: None,
        };
        let groups = layout.groups.as_ref().unwrap();
        let mut assigned = [false; MAX_DOCUMENTS];
        for group in groups {
            for tab in &group.tabs {
                if !assigned[tab.file] {
                    layout.files[tab.file].view = tab.view.clone();
                    assigned[tab.file] = true;
                }
            }
            let tab = &group.tabs[group.active];
            layout.panes.push(Pane {
                file: tab.file,
                view: tab.view.clone(),
            });
        }
        layout.active_file = layout
            .panes
            .get(layout.active_pane)
            .map_or(0, |pane| pane.file);
        layout.normalized()
    }
}
fn regular_open(path: &Path, create: bool) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => bail!("Session state must be a regular file"),
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(create)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Session state must be a regular file");
    }
    Ok(file)
}
fn lock(file: &File) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => bail!("Session metadata lock unavailable: {error}"),
        }
    }
}
fn read(path: &Path, workspace: &Path) -> Result<Saved> {
    let mut bytes = Vec::new();
    regular_open(path, false)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Session metadata exceeds 1 MiB");
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("Invalid session metadata")?;
    let saved = match value.get("schema").and_then(serde_json::Value::as_u64) {
        Some(1) => {
            let saved: SavedV1 =
                serde_json::from_slice(&bytes).context("Invalid schema-1 session")?;
            if saved.schema != 1 {
                bail!("Session schema mismatch");
            }
            let legacy = saved.layout;
            Saved {
                workspace: saved.workspace,
                stamp: saved.stamp,
                layout: Layout {
                    files: legacy.files,
                    panes: legacy.panes,
                    active_file: legacy.active_file,
                    active_pane: legacy.active_pane,
                    horizontal: legacy.horizontal,
                    groups: None,
                    active_group: 0,
                    tree: None,
                    sticky: None,
                }
                .normalized()?,
            }
        }
        Some(2) => {
            let saved: SavedV2 =
                serde_json::from_slice(&bytes).context("Invalid schema-2 session")?;
            if saved.schema != 2 {
                bail!("Session schema mismatch");
            }
            Saved {
                workspace: saved.workspace,
                stamp: saved.stamp,
                layout: saved.layout.into_layout()?,
            }
        }
        Some(3) => {
            let saved: SavedV3 =
                serde_json::from_slice(&bytes).context("Invalid schema-3 session")?;
            if saved.schema != 3 {
                bail!("Session schema mismatch");
            }
            Saved {
                workspace: saved.workspace,
                stamp: saved.stamp,
                layout: saved.layout.into_layout()?,
            }
        }
        _ => bail!("Unknown session schema"),
    };
    if saved.workspace != workspace {
        bail!("Session schema/workspace mismatch");
    }
    saved.layout.validate_structure()?;
    Ok(saved)
}
struct Store {
    directory: PathBuf,
    workspace: PathBuf,
    path: PathBuf,
    lease_path: PathBuf,
    lease: File,
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = self.lease.unlock();
    }
}
impl Store {
    fn new(config: &Path, workspace: &Path) -> Result<(Self, Option<Layout>)> {
        let workspace = fs::canonicalize(workspace)?;
        let digest = Sha256::digest(workspace.as_os_str().as_encoded_bytes());
        let directory = config.join("state/sessions").join(format!("{digest:x}"));
        fs::create_dir_all(&directory)?;
        let registry = regular_open(&directory.join("registry.lock"), true)?;
        lock(&registry)?;
        let mut slots = Vec::new();
        for (count, entry) in fs::read_dir(&directory)?.enumerate() {
            if count >= 64 {
                bail!("Session directory entry budget exceeded");
            }
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "lock")
                && path.file_name().is_some_and(|n| n != "registry.lock")
            {
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    bail!("Invalid session slot");
                };
                if uuid::Uuid::parse_str(stem).is_err() {
                    bail!("Invalid session slot");
                }
                let lease = regular_open(&path, false)?;
                // Windows requires write access for an exclusive file lock.
                drop(lease);
                let lease = regular_open(&path, true)?;
                match lease.try_lock() {
                    Ok(()) => {
                        let saved_path = path.with_extension("json");
                        let saved = if saved_path.exists() {
                            Some(read(&saved_path, &workspace)?)
                        } else {
                            None
                        };
                        slots.push((path, Some(lease), saved));
                    }
                    Err(fs::TryLockError::WouldBlock) => slots.push((path, None, None)),
                    Err(error) => return Err(error.into()),
                }
            }
        }
        slots.sort_by_key(|(_, _, saved)| saved.as_ref().map_or(0, |s| s.stamp));
        let previous = slots
            .iter()
            .rev()
            .find_map(|(_, _, saved)| saved.as_ref().map(|s| s.layout.clone()));
        if slots.len() > MAX_SLOTS {
            bail!("Session slot budget exceeded");
        }
        let newest = slots.iter().rposition(|(_, _, saved)| saved.is_some());
        if slots.len() >= MAX_SLOTS {
            let Some(index) = slots
                .iter()
                .enumerate()
                .position(|(index, (_, lease, _))| lease.is_some() && Some(index) != newest)
            else {
                bail!("All eight session slots are in use");
            };
            let (path, lease, _) = slots.remove(index);
            match fs::remove_file(path.with_extension("json")) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            drop(lease);
            fs::remove_file(path)?;
        }
        let lease_path = directory.join(format!("{}.lock", uuid::Uuid::new_v4()));
        let lease = regular_open(&lease_path, true)?;
        lease.try_lock()?;
        let path = lease_path.with_extension("json");
        Ok((
            Self {
                directory,
                workspace,
                path,
                lease_path,
                lease,
            },
            previous,
        ))
    }
    fn publish(&self, layout: &Layout) -> Result<()> {
        self.publish_with(layout, || Ok(()))
    }
    fn publish_with(
        &self,
        layout: &Layout,
        before_publish: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        layout.validate()?;
        let saved = SavedV3 {
            schema: 3,
            workspace: self.workspace.clone(),
            stamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_micros()
                .min(u64::MAX as u128) as u64,
            layout: NestedGroupLayoutV3::from_layout(layout)?,
        };
        // Count the complete envelope including workspace, tree and per-tab
        // sticky flags before allocating bytes or touching the published slot.
        serde_json::to_writer(MetadataBudget(0), &saved)
            .context("Session metadata exceeds 1 MiB")?;
        let bytes = serde_json::to_vec(&saved)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("Session metadata exceeds 1 MiB");
        }
        if self.path.exists() {
            let _ = regular_open(&self.path, false)?;
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        before_publish()?;
        temp.persist(&self.path).map_err(|e| e.error)?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
    fn finish(self) -> Result<()> {
        if !self.path.exists() {
            let registry = regular_open(&self.directory.join("registry.lock"), true)?;
            lock(&registry)?;
            fs::remove_file(&self.lease_path)?;
        }
        Ok(())
    }
}

pub struct Restored {
    pub layout: Layout,
    pub documents: Vec<Document>,
}
fn restore(layout: &Layout, skip: &[PathBuf]) -> Result<Restored> {
    restore_with_budget(layout, skip, MAX_READ_BYTES)
}
fn restore_with_budget(layout: &Layout, skip: &[PathBuf], mut remaining: u64) -> Result<Restored> {
    let mut layout = layout.normalized()?;
    validate_skip(skip)?;
    let identities = skip
        .iter()
        .map(|path| {
            (
                path,
                fs::canonicalize(path).unwrap_or_else(|_| path.clone()),
            )
        })
        .collect::<Vec<_>>();
    let mut documents = Vec::new();
    let mut paths = std::collections::HashSet::new();
    for file in &mut layout.files {
        let canonical = fs::canonicalize(&file.path);
        let identity = canonical.as_ref().unwrap_or(&file.path);
        if !paths.insert(identity.clone()) {
            bail!("Session contains aliases of the same file");
        }
        if let Some((path, _)) = identities
            .iter()
            .find(|(path, resolved)| **path == file.path || resolved == identity)
        {
            file.path = path.to_path_buf();
            continue;
        }
        canonical?;
        let doc = Document::open_existing_bounded(&file.path, remaining)?;
        remaining = remaining
            .checked_sub(doc.text.len_bytes() as u64)
            .context("Session files exceed 128 MiB")?;
        file.path = doc.path.clone().context("Restored file has no path")?;
        documents.push(doc);
    }
    layout.validate_structure()?;
    Ok(Restored { layout, documents })
}
fn validate_skip(skip: &[PathBuf]) -> Result<()> {
    if skip.len() > 128
        || skip.iter().any(|path| {
            !path.is_absolute()
                || path.as_os_str().len() > 4096
                || path.as_os_str().as_encoded_bytes().contains(&0)
        })
    {
        bail!("Session existing-model paths exceed native bounds");
    }
    Ok(())
}
enum Request {
    Save(Layout),
    Restore(Vec<PathBuf>),
    Finish(Option<Layout>),
}
pub enum Event {
    Ready(bool),
    Saved,
    Restored(Restored),
    Failed(String),
}
pub struct Worker {
    sender: SyncSender<Request>,
    receiver: Receiver<Event>,
    pending: bool,
    queued: Option<Layout>,
    saved: Option<Layout>,
    inflight: Option<Layout>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}
impl Worker {
    pub fn start(config: PathBuf, workspace: PathBuf) -> Result<Self> {
        let (sender, requests) = mpsc::sync_channel(1);
        let (events, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("vscli-session".into())
            .spawn(move || {
                let (store, previous) = match Store::new(&config, &workspace) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = events.send(Event::Failed(format!("{error:#}")));
                        return Err(error);
                    }
                };
                if events.send(Event::Ready(previous.is_some())).is_err() {
                    return store.finish();
                }
                while let Ok(request) = requests.recv() {
                    let result = match request {
                        Request::Save(layout) => store.publish(&layout).map(|()| Event::Saved),
                        Request::Restore(skip) => previous
                            .as_ref()
                            .context("No previous clean-file session")
                            .and_then(|layout| restore(layout, &skip))
                            .map(Event::Restored),
                        Request::Finish(layout) => {
                            if let Some(layout) = layout {
                                store.publish(&layout)?;
                            }
                            return store.finish();
                        }
                    };
                    if events
                        .send(result.unwrap_or_else(|e| Event::Failed(format!("{e:#}"))))
                        .is_err()
                    {
                        break;
                    }
                }
                store.finish()
            })?;
        Ok(Self {
            sender,
            receiver,
            pending: true,
            queued: None,
            saved: None,
            inflight: None,
            thread: Some(thread),
        })
    }
    pub fn poll(&mut self) -> Option<Event> {
        let event = match self.receiver.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) if self.pending => {
                Event::Failed("Session worker stopped".into())
            }
            Err(_) => return None,
        };
        self.pending = false;
        if matches!(event, Event::Saved) {
            self.saved = self.inflight.take();
        } else if matches!(event, Event::Failed(_))
            && let Some(layout) = self.inflight.take()
            && self.queued.is_none()
        {
            self.queued = Some(layout);
        }
        Some(event)
    }
    pub fn save(&mut self, layout: Layout) -> Result<()> {
        layout.validate()?;
        if !self.pending && self.saved.as_ref() == Some(&layout) && self.queued.is_none() {
            return Ok(());
        }
        self.queued = Some(layout);
        self.flush()
    }
    pub fn flush(&mut self) -> Result<()> {
        if !self.pending
            && let Some(layout) = self.queued.take()
        {
            if let Err(error) = self.sender.try_send(Request::Save(layout.clone())) {
                self.queued = Some(layout);
                return Err(error).context("Session worker unavailable");
            }
            self.inflight = Some(layout);
            self.pending = true;
        }
        Ok(())
    }
    pub fn restore(&mut self, skip: Vec<PathBuf>) -> Result<()> {
        validate_skip(&skip)?;
        if self.pending {
            bail!("Session worker is busy; retry shortly");
        }
        self.sender
            .try_send(Request::Restore(skip))
            .context("Session worker unavailable")?;
        self.pending = true;
        Ok(())
    }
    /// A final snapshot supersedes queued state. None deliberately suppresses
    /// unsent publication after failed/stale restoration; issued I/O may finish.
    pub fn finish(self, layout: Option<Layout>) -> Result<()> {
        self.finish_with_timeout(layout, Duration::from_secs(5))
    }
    fn finish_with_timeout(mut self, layout: Option<Layout>, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut request = Some(Request::Finish(layout));
        loop {
            // Drain replies even when input stopped before the initial Ready event.
            while self.receiver.try_recv().is_ok() {}
            if let Some(next) = request.take() {
                match self.sender.try_send(next) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(next)) => request = Some(next),
                    Err(mpsc::TrySendError::Disconnected(_)) => {}
                }
            }
            if self.thread.as_ref().unwrap().is_finished() {
                return self
                    .thread
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| anyhow::anyhow!("Session worker panicked"))?;
            }
            if Instant::now() >= deadline {
                // Dropping channels cancels further replies/work. A blocked operation
                // keeps the lease until the sole detached worker actually exits.
                bail!("Session shutdown timed out; last published metadata retained");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(path: &Path) -> Layout {
        let view = View {
            selections: vec![SavedSelection {
                cursor: Position {
                    line: 0,
                    character: 2,
                },
                anchor: None,
            }],
            top: 0,
            left: 0,
        };
        Layout {
            files: vec![SavedFile {
                path: path.to_owned(),
                view: view.clone(),
            }],
            panes: vec![Pane { file: 0, view }],
            ..Layout::default()
        }
    }
    fn await_event(worker: &mut Worker) -> Event {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(event) = worker.poll() {
                return event;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn leases_isolate_live_instances_workspaces_and_config_roots() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let other = root.path().join("other");
        fs::create_dir(&other).unwrap();
        let file = root.path().join("file.txt");
        fs::write(&file, "disk text").unwrap();
        let saved = layout(&file);
        let (first, prior) = Store::new(&config, root.path()).unwrap();
        assert!(prior.is_none());
        first.publish(&saved).unwrap();
        let (second, prior) = Store::new(&config, root.path()).unwrap();
        assert!(prior.is_none(), "A live instance must not be restored");
        assert_ne!(first.path, second.path);
        drop(first);
        let (third, prior) = Store::new(&config, root.path()).unwrap();
        assert_eq!(prior, Some(saved.normalized().unwrap()));
        assert!(
            Store::new(&root.path().join("other-config"), root.path())
                .unwrap()
                .1
                .is_none()
        );
        assert!(Store::new(&config, &other).unwrap().1.is_none());
        second.finish().unwrap();
        third.finish().unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "disk text");
    }
    #[test]
    fn capacity_never_evicts_live_leases_or_the_only_retryable_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let missing = root.path().join("missing.txt");
        let saved = layout(&missing);
        let (previous, _) = Store::new(&config, root.path()).unwrap();
        previous.publish(&saved).unwrap();
        let path = previous.path.clone();
        let bytes = fs::read(&path).unwrap();
        drop(previous);
        let mut live = Vec::new();
        for _ in 0..7 {
            live.push(Store::new(&config, root.path()).unwrap().0);
        }
        assert!(Store::new(&config, root.path()).is_err());
        assert!(restore(&saved, &[]).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
        for store in live {
            assert!(store.lease_path.exists());
            store.finish().unwrap();
        }
    }
    #[test]
    fn malformed_oversized_and_failed_publication_retain_previous_bytes() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let workspace = fs::canonicalize(root.path()).unwrap();
        assert_eq!(store.workspace, workspace);
        let saved = layout(&workspace.join("file.txt"));
        store.publish(&saved).unwrap();
        let before = fs::read(&store.path).unwrap();
        let mut bad = saved.clone();
        bad.panes[0].file = 90;
        assert!(store.publish(&bad).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert!(
            store
                .publish_with(&Layout::default(), || bail!(
                    "injected failure after sync before rename"
                ))
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert_eq!(
            read(&store.path, &workspace).unwrap().layout,
            saved.normalized().unwrap()
        );
        assert!(read(&store.path, &workspace.join("different-workspace")).is_err());
        for bytes in [b"malformed".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            fs::write(&store.path, &bytes).unwrap();
            assert!(read(&store.path, &workspace).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
        }
    }
    #[test]
    fn restore_bounds_combined_reads_and_never_creates_missing_files() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.txt");
        let second = root.path().join("second.txt");
        fs::write(&first, "123456").unwrap();
        fs::write(&second, "abcdef").unwrap();
        let mut saved = layout(&first);
        saved.files.push(SavedFile {
            path: second.clone(),
            view: saved.files[0].view.clone(),
        });
        assert!(restore_with_budget(&saved, &[], 11).is_err());
        assert_eq!(
            restore_with_budget(&saved, &[], 12)
                .unwrap()
                .documents
                .len(),
            2
        );
        assert_eq!(
            restore_with_budget(&saved, std::slice::from_ref(&first), 6)
                .unwrap()
                .documents
                .len(),
            1
        );
        fs::remove_file(&second).unwrap();
        assert!(restore(&saved, &[]).is_err());
        assert!(!second.exists());
        let mut bad = saved.clone();
        bad.files = vec![saved.files[0].clone(); 33];
        assert!(bad.validate().is_err());
        bad = saved.clone();
        bad.panes = vec![saved.panes[0].clone(); 5];
        assert!(bad.validate().is_err());
        bad = saved.clone();
        bad.files[0].view.selections = vec![saved.files[0].view.selections[0].clone(); 129];
        assert!(bad.validate().is_err());
        assert_eq!(fs::read_to_string(first).unwrap(), "123456");
    }
    #[test]
    fn queued_snapshots_coalesce_and_shutdown_drains_initial_and_save_replies() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let saved = layout(&root.path().join("first.txt"));
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        worker.save(saved.clone()).unwrap(); // Ready has not been polled.
        worker.finish(Some(saved.clone())).unwrap();
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(true)));
        worker.save(Layout::default()).unwrap();
        worker.save(saved.clone()).unwrap();
        worker.finish(Some(saved.clone())).unwrap(); // Saved may still be queued.
        let (_, previous) = Store::new(&config, root.path()).unwrap();
        assert_eq!(previous, Some(saved.normalized().unwrap()));
    }
    #[test]
    fn returning_to_acknowledged_state_while_another_write_runs_preserves_latest_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let a = layout(&root.path().join("a.txt"));
        let b = layout(&root.path().join("b.txt"));
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(false)));
        worker.save(a.clone()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.save(b).unwrap();
        worker.save(a.clone()).unwrap();
        assert_eq!(worker.queued.as_ref(), Some(&a));
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.flush().unwrap();
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.finish(None).unwrap();
        assert_eq!(
            Store::new(&config, root.path()).unwrap().1,
            Some(a.normalized().unwrap())
        );
    }
    fn groups_layout(paths: &[PathBuf]) -> Layout {
        let mut saved = layout(&paths[0]);
        let base_view = saved.files[0].view.clone();
        saved.files.extend(
            paths
                .iter()
                .skip(1)
                .map(|path| SavedFile {
                    path: path.clone(),
                    view: base_view.clone(),
                })
                .collect::<Vec<_>>(),
        );
        let mut first = saved.files[0].view.clone();
        first.selections[0].cursor.character = 1;
        let mut historical = first.clone();
        historical.selections[0].cursor.character = 3;
        historical.selections[0].anchor = Some(Position {
            line: 0,
            character: 0,
        });
        let mut shared = first.clone();
        shared.selections[0].cursor.character = 5;
        shared.top = 4;
        saved.groups = Some(vec![
            Group {
                tabs: vec![
                    Tab {
                        file: 0,
                        view: first.clone(),
                    },
                    Tab {
                        file: 1,
                        view: historical,
                    },
                ],
                active: 0,
                recent: vec![0, 1],
            },
            Group {
                tabs: vec![Tab {
                    file: 0,
                    view: shared.clone(),
                }],
                active: 0,
                recent: vec![0],
            },
        ]);
        saved.panes = vec![
            Pane {
                file: 0,
                view: first,
            },
            Pane {
                file: 0,
                view: shared,
            },
        ];
        saved.active_group = 1;
        saved.active_pane = 1;
        saved.horizontal = true;
        saved.normalized().unwrap()
    }
    #[test]
    fn schema_three_worker_roundtrip_retains_inactive_views_shared_files_and_recent_order() {
        let root = tempfile::tempdir().unwrap();
        let canonical = fs::canonicalize(root.path()).unwrap();
        let paths = [canonical.join("a.txt"), canonical.join("b.txt")];
        for path in &paths {
            fs::write(path, "猫🙂 alpha\r\nsecond\r\n").unwrap();
        }
        let saved = groups_layout(&paths);
        let config = root.path().join("config");
        let mut worker = Worker::start(config.clone(), root.path().into()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(false)));
        worker.save(saved.clone()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.finish(None).unwrap();
        let mut worker = Worker::start(config, root.path().into()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(true)));
        worker.restore(Vec::new()).unwrap();
        let Event::Restored(restored) = await_event(&mut worker) else {
            panic!("restore failed");
        };
        assert_eq!(restored.layout, saved);
        assert_eq!(restored.documents.len(), 2);
        assert!(restored.documents.iter().all(|doc| !doc.dirty()));
        assert_eq!(
            restored.layout.groups.as_ref().unwrap()[0].tabs[1]
                .view
                .selections[0]
                .anchor,
            Some(Position {
                line: 0,
                character: 0
            })
        );
        worker.finish(None).unwrap();
        for path in &paths {
            assert_eq!(
                fs::read(path).unwrap(),
                "猫🙂 alpha\r\nsecond\r\n".as_bytes()
            );
        }
    }
    #[test]
    fn schema_one_migration_retains_original_bytes_until_successful_schema_three_publish() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let mut legacy = layout(&root.path().join("a.txt"));
        legacy.files.push(SavedFile {
            path: root.path().join("b.txt"),
            view: legacy.files[0].view.clone(),
        });
        legacy.files.push(SavedFile {
            path: root.path().join("c.txt"),
            view: legacy.files[0].view.clone(),
        });
        legacy.panes[0].file = 1;
        legacy.panes.push(Pane {
            file: 0,
            view: legacy.files[0].view.clone(),
        });
        legacy.active_file = 0;
        legacy.active_pane = 1;
        legacy.panes[0].view.selections[0].cursor.character = 7;
        let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"workspace":store.workspace,"stamp":12,"layout":LegacyLayout {
            files: legacy.files.clone(), panes: legacy.panes.clone(), active_file: legacy.active_file,
            active_pane: legacy.active_pane, horizontal: true,
        }})).unwrap();
        fs::write(&store.path, &bytes).unwrap();
        let migrated = read(&store.path, &store.workspace).unwrap().layout;
        let groups = migrated.groups.as_ref().unwrap();
        assert_eq!(
            groups[0]
                .tabs
                .iter()
                .map(|tab| tab.file)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(groups[0].active, 1);
        assert_eq!(groups[0].recent, vec![1, 0, 2]);
        assert_eq!(groups[0].tabs[1].view.selections[0].cursor.character, 7);
        assert_eq!(groups[1].tabs.len(), 1);
        assert_eq!(migrated.active_group, 1);
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        assert!(
            store
                .publish_with(&migrated, || bail!(
                    "injected migration publication failure"
                ))
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        store.publish(&migrated).unwrap();
        let wire: serde_json::Value =
            serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        assert_eq!(wire["schema"], 3);
        assert!(wire["layout"].get("panes").is_none());
        assert!(wire["layout"].get("active_file").is_none());
        assert!(wire["layout"]["files"][0].is_string());
        assert_eq!(
            read(&store.path, &store.workspace).unwrap().layout,
            migrated
        );
    }
    #[test]
    fn late_invalid_group_reference_unknown_fields_and_versions_fail_without_rewriting() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let saved = groups_layout(&[root.path().join("a"), root.path().join("b")]);
        store.publish(&saved).unwrap();
        let prior = fs::read(&store.path).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&prior).unwrap();
        for mode in 0..6 {
            let mut bad = wire.clone();
            match mode {
                0 => bad["layout"]["groups"][1]["tabs"][0]["file"] = serde_json::json!(32),
                1 => bad["layout"]["groups"][0]["recent"] = serde_json::json!([0, 0]),
                2 => bad["layout"]["groups"][0]["tabs"][1]["file"] = serde_json::json!(0),
                3 => bad["layout"]["groups"][1]["unexpected"] = serde_json::json!(true),
                4 => bad["layout"]["panes"] = serde_json::json!([]),
                _ => bad["schema"] = serde_json::json!(99),
            }
            let malformed = serde_json::to_vec(&bad).unwrap();
            fs::write(&store.path, &malformed).unwrap();
            assert!(read(&store.path, &store.workspace).is_err(), "mode {mode}");
            assert_eq!(fs::read(&store.path).unwrap(), malformed);
        }
        fs::write(&store.path, &prior).unwrap();
        let mut bad = saved.clone();
        bad.groups.as_mut().unwrap()[1].tabs[0].file = 32;
        assert!(store.publish(&bad).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), prior);
    }
    #[test]
    fn metadata_budget_and_bad_latest_snapshot_preserve_acknowledged_publication() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let saved = groups_layout(&[root.path().join("a"), root.path().join("b")]);
        store.publish(&saved).unwrap();
        let prior = fs::read(&store.path).unwrap();
        let mut huge = saved.clone();
        huge.files = (0..MAX_DOCUMENTS)
            .map(|index| SavedFile {
                path: root.path().join(format!("file{index}")),
                view: saved.files[0].view.clone(),
            })
            .collect();
        let view = View {
            selections: vec![
                SavedSelection {
                    cursor: Position {
                        line: usize::MAX,
                        character: usize::MAX
                    },
                    anchor: Some(Position {
                        line: usize::MAX,
                        character: usize::MAX
                    })
                };
                MAX_SELECTIONS
            ],
            top: usize::MAX,
            left: usize::MAX,
        };
        let group = Group {
            tabs: (0..MAX_DOCUMENTS)
                .map(|file| Tab {
                    file,
                    view: view.clone(),
                })
                .collect(),
            active: 0,
            recent: (0..MAX_DOCUMENTS).collect(),
        };
        huge.groups = Some(vec![group; MAX_VIEWS]);
        huge.active_group = 0;
        huge.active_pane = 0;
        huge.panes = vec![Pane { file: 0, view }; MAX_VIEWS];
        huge.tree = flat_saved_layout(MAX_VIEWS, Axis::Rows);
        huge.sticky = Some(vec![vec![false; MAX_DOCUMENTS]; MAX_VIEWS]);
        assert!(huge.validate().unwrap_err().to_string().contains("1 MiB"));
        assert!(store.publish(&huge).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), prior);
        let mut worker =
            Worker::start(root.path().join("other-config"), root.path().into()).unwrap();
        worker.save(saved.clone()).unwrap();
        assert!(worker.save(huge).is_err());
        assert_eq!(worker.queued, Some(saved.clone()));
        worker.finish(Some(saved)).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn aliases_and_late_missing_files_reject_whole_restore_but_recovered_deleted_models_survive() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        let alias = root.path().join("alias");
        fs::write(&file, "clean 猫🙂\r\n").unwrap();
        symlink(&file, &alias).unwrap();
        let saved = groups_layout(&[file.clone(), alias]);
        assert!(restore(&saved, &[]).is_err());
        let missing = root.path().join("missing");
        let saved = groups_layout(&[file.clone(), missing.clone()]);
        assert!(restore(&saved, &[]).is_err());
        assert!(!missing.exists());
        let restored = restore(&saved, std::slice::from_ref(&missing)).unwrap();
        assert_eq!(restored.documents.len(), 1);
        assert_eq!(restored.layout.files[1].path, missing);
        assert_eq!(fs::read(&file).unwrap(), "clean 猫🙂\r\n".as_bytes());
    }
    #[test]
    fn shutdown_deadline_does_not_join_a_blocked_worker() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (events, receiver) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            wait.recv().unwrap();
            let _ = events.send(Event::Ready(false));
            drop(requests);
            Ok(())
        });
        let worker = Worker {
            sender,
            receiver,
            pending: true,
            queued: None,
            saved: None,
            inflight: None,
            thread: Some(thread),
        };
        let started = Instant::now();
        assert!(
            worker
                .finish_with_timeout(None, Duration::from_millis(30))
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn fifo_state_and_targets_are_rejected_without_opening_a_blocking_reader() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("fifo");
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(regular_open(&fifo, false).is_err());
        assert!(restore(&layout(&fifo), &[]).is_err());
    }
    fn nested_v3_layout(paths: &[PathBuf; 3]) -> Layout {
        let mut saved = groups_layout(&paths[..2]);
        let view = saved.files[0].view.clone();
        saved.files.push(SavedFile {
            path: paths[2].clone(),
            view: view.clone(),
        });
        saved.groups.as_mut().unwrap().push(Group {
            tabs: vec![
                Tab {
                    file: 2,
                    view: view.clone(),
                },
                Tab {
                    file: 1,
                    view: view.clone(),
                },
            ],
            active: 0,
            recent: vec![0, 1],
        });
        saved.panes.push(Pane { file: 2, view });
        saved.active_group = 2;
        saved.active_pane = 2;
        saved.active_file = 2;
        saved.tree = Some(SavedNode::Split {
            axis: Axis::Columns,
            first_weight: 3,
            second_weight: 7,
            first: Box::new(SavedNode::Leaf { group: 0 }),
            second: Box::new(SavedNode::Split {
                axis: Axis::Rows,
                first_weight: 2,
                second_weight: 5,
                first: Box::new(SavedNode::Leaf { group: 1 }),
                second: Box::new(SavedNode::Leaf { group: 2 }),
            }),
        });
        saved.sticky = Some(vec![vec![true, false], vec![false], vec![true, false]]);
        saved.normalized().unwrap()
    }

    #[test]
    fn schema_three_worker_retains_nested_weights_sticky_prefixes_and_shared_views() {
        let root = tempfile::tempdir().unwrap();
        let workspace = fs::canonicalize(root.path()).unwrap();
        let paths = [
            workspace.join("a.txt"),
            workspace.join("b.txt"),
            workspace.join("c.txt"),
        ];
        let original = "猫🙂 alpha\r\nsecond\r\n";
        for path in &paths {
            fs::write(path, original).unwrap();
        }
        let saved = nested_v3_layout(&paths);
        let config = root.path().join("config");
        let mut worker = Worker::start(config.clone(), workspace.clone()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(false)));
        worker.save(saved.clone()).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Saved));
        worker.finish(None).unwrap();
        let (store, prior) = Store::new(&config, &workspace).unwrap();
        assert_eq!(prior, Some(saved.clone()));
        let directory = store.directory.clone();
        let wire = fs::read_dir(directory)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().is_some_and(|ext| ext == "json")).then_some(path)
            })
            .map(|path| fs::read(path).unwrap())
            .next()
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&wire).unwrap();
        assert_eq!(value["schema"], 3);
        assert_eq!(
            value["layout"]["tree"],
            serde_json::to_value(saved.tree.as_ref().unwrap()).unwrap()
        );
        assert_eq!(value["layout"]["groups"][0]["tabs"][0]["sticky"], true);
        assert!(
            value["layout"]["groups"][0]["tabs"][0]
                .get("preview")
                .is_none()
        );
        store.finish().unwrap();
        let mut worker = Worker::start(config, workspace).unwrap();
        assert!(matches!(await_event(&mut worker), Event::Ready(true)));
        worker.restore(Vec::new()).unwrap();
        let Event::Restored(restored) = await_event(&mut worker) else {
            panic!("restore failed")
        };
        assert_eq!(restored.layout, saved);
        assert_eq!(restored.documents.len(), 3);
        assert!(restored.documents.iter().all(|doc| !doc.dirty()));
        worker.finish(None).unwrap();
        for path in paths {
            assert_eq!(fs::read(path).unwrap(), original.as_bytes());
        }
    }

    #[test]
    fn strict_schema_two_migrates_without_rewriting_or_accepting_schema_three_fields() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let saved = groups_layout(&[root.path().join("a.txt"), root.path().join("b.txt")]);
        let old = SavedV2 {
            schema: 2,
            workspace: store.workspace.clone(),
            stamp: 12,
            layout: GroupLayout::from_layout(&saved).unwrap(),
        };
        let original = serde_json::to_vec(&old).unwrap();
        fs::write(&store.path, &original).unwrap();
        let migrated = read(&store.path, &store.workspace).unwrap().layout;
        assert_eq!(migrated, saved);
        assert_eq!(migrated.tree, flat_saved_layout(2, Axis::Rows));
        assert_eq!(migrated.sticky, Some(vec![vec![false, false], vec![false]]));
        assert_eq!(fs::read(&store.path).unwrap(), original);
        for mode in 0..3 {
            let mut bad: serde_json::Value = serde_json::from_slice(&original).unwrap();
            match mode {
                0 => {
                    bad["layout"]["tree"] =
                        serde_json::to_value(migrated.tree.as_ref().unwrap()).unwrap()
                }
                1 => bad["layout"]["groups"][0]["tabs"][0]["sticky"] = serde_json::json!(false),
                _ => bad["layout"]["groups"][0]["tabs"][0]["preview"] = serde_json::json!(false),
            }
            let bytes = serde_json::to_vec(&bad).unwrap();
            fs::write(&store.path, &bytes).unwrap();
            assert_eq!(
                read(&store.path, &store.workspace)
                    .err()
                    .unwrap()
                    .to_string(),
                "Invalid schema-2 session"
            );
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
        }
        fs::write(&store.path, &original).unwrap();
        assert!(
            store
                .publish_with(&migrated, || bail!(
                    "injected schema-3 migration write failure"
                ))
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
        store.publish(&migrated).unwrap();
        assert_eq!(
            read(&store.path, &store.workspace).unwrap().layout,
            migrated
        );
        let wire: serde_json::Value =
            serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        assert_eq!(wire["schema"], 3);
    }

    #[test]
    fn schema_three_rejects_late_tree_sticky_and_view_errors_before_any_file_load() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let paths = [
            root.path().join("missing-a"),
            root.path().join("missing-b"),
            root.path().join("missing-c"),
        ];
        let saved = nested_v3_layout(&paths);
        // A positive metadata read succeeds without any referenced file existing.
        store.publish(&saved).unwrap();
        let original = fs::read(&store.path).unwrap();
        assert_eq!(read(&store.path, &store.workspace).unwrap().layout, saved);
        let wire: serde_json::Value = serde_json::from_slice(&original).unwrap();
        for mode in 0..8 {
            let mut bad = wire.clone();
            let expected = match mode {
                0 => {
                    bad["layout"]["tree"]["second"]["second"]["group"] = serde_json::json!(0);
                    "Invalid or duplicate layout group index"
                }
                1 => {
                    bad["layout"]["tree"]["second"]["second_weight"] = serde_json::json!(0);
                    "Invalid editor layout weight"
                }
                2 => {
                    bad["layout"]["groups"][2]["tabs"][0]["sticky"] = serde_json::json!(false);
                    bad["layout"]["groups"][2]["tabs"][1]["sticky"] = serde_json::json!(true);
                    "Invalid session sticky prefix"
                }
                3 => {
                    bad["layout"]["groups"][2]["recent"] = serde_json::json!([0, 0]);
                    "Invalid session recent-tab permutation"
                }
                4 => {
                    bad["layout"]["groups"][2]["tabs"][1]["file"] = serde_json::json!(32);
                    "Invalid or duplicate session group file"
                }
                5 => {
                    bad["layout"]["groups"][2]["tabs"][0]["preview"] = serde_json::json!(false);
                    "Invalid schema-3 session"
                }
                6 => {
                    bad["layout"].as_object_mut().unwrap().remove("tree");
                    "Invalid schema-3 session"
                }
                _ => {
                    bad["layout"]["groups"][2]["tabs"][1]["view"]["selections"] =
                        serde_json::json!([]);
                    "Session view exceeds 128 selections or has no cursor"
                }
            };
            let bytes = serde_json::to_vec(&bad).unwrap();
            fs::write(&store.path, &bytes).unwrap();
            assert_eq!(
                read(&store.path, &store.workspace)
                    .err()
                    .unwrap()
                    .to_string(),
                expected,
                "mode {mode}"
            );
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
            assert!(paths.iter().all(|path| !path.exists()));
        }
        fs::write(&store.path, &original).unwrap();
        assert_eq!(read(&store.path, &store.workspace).unwrap().layout, saved);
    }

    #[test]
    fn schema_three_empty_wire_is_explicit_and_failed_replace_preserves_lease_and_last_bytes() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let empty = Layout::default().normalized().unwrap();
        store.publish(&empty).unwrap();
        let original = fs::read(&store.path).unwrap();
        assert_eq!(read(&store.path, &store.workspace).unwrap().layout, empty);
        let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert!(value["layout"]["tree"].is_null());
        assert_eq!(value["layout"]["groups"], serde_json::json!([]));
        let mut missing = value.clone();
        missing["layout"].as_object_mut().unwrap().remove("tree");
        fs::write(&store.path, serde_json::to_vec(&missing).unwrap()).unwrap();
        assert_eq!(
            read(&store.path, &store.workspace)
                .err()
                .unwrap()
                .to_string(),
            "Invalid schema-3 session"
        );
        fs::write(&store.path, &original).unwrap();
        let next = nested_v3_layout(&[
            root.path().join("a"),
            root.path().join("b"),
            root.path().join("c"),
        ]);
        assert!(
            store
                .publish_with(&next, || bail!("actual prepared-file publication denied"))
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
        assert!(store.lease_path.exists());
        let lease = regular_open(&store.lease_path, true).unwrap();
        assert!(matches!(
            lease.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        assert!(fs::read_dir(&store.directory).unwrap().all(|entry| {
            let path = entry.unwrap().path();
            path.extension()
                .is_some_and(|extension| extension == "json" || extension == "lock")
        }));
    }
    #[test]
    fn bounded_near_budget_schema_two_remains_readable_when_schema_three_write_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (store, _) = Store::new(&root.path().join("config"), root.path()).unwrap();
        let paths = (0..MAX_DOCUMENTS)
            .map(|index| root.path().join(format!("legacy-{index}")))
            .collect::<Vec<_>>();
        for path in &paths {
            fs::write(path, "猫🙂\r\n").unwrap();
        }
        let selection = SavedSelection {
            cursor: Position {
                line: usize::MAX,
                character: usize::MAX,
            },
            anchor: Some(Position {
                line: usize::MAX,
                character: usize::MAX,
            }),
        };
        let view = View {
            selections: vec![selection.clone()],
            top: usize::MAX,
            left: usize::MAX,
        };
        let mut old = SavedV2 {
            schema: 2,
            workspace: store.workspace.clone(),
            stamp: 12,
            layout: GroupLayout {
                files: paths.clone(),
                groups: (0..MAX_VIEWS)
                    .map(|_| Group {
                        tabs: (0..MAX_DOCUMENTS)
                            .map(|file| Tab {
                                file,
                                view: view.clone(),
                            })
                            .collect(),
                        active: 0,
                        recent: (0..MAX_DOCUMENTS).collect(),
                    })
                    .collect(),
                active_group: 0,
                horizontal: false,
            },
        };
        // Find a valid old encoding just below the cap using a bounded binary
        // search over legal selection counts, never padding unknown fields.
        let members = MAX_VIEWS * MAX_DOCUMENTS;
        let mut encode = |total: usize| {
            for (index, tab) in old
                .layout
                .groups
                .iter_mut()
                .flat_map(|group| &mut group.tabs)
                .enumerate()
            {
                let count = total / members + usize::from(index < total % members);
                tab.view.selections = vec![selection.clone(); count];
            }
            serde_json::to_vec(&old).unwrap()
        };
        let mut low = members;
        let mut high = members * MAX_SELECTIONS;
        let limit = MAX_BYTES as usize - 512;
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            if encode(middle).len() <= limit {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let original = encode(low);
        assert!(original.len() <= limit);
        assert!(original.len() > MAX_BYTES as usize - 1024);
        fs::write(&store.path, &original).unwrap();
        let saved = read(&store.path, &store.workspace).unwrap().layout;
        assert_eq!(saved.files.len(), MAX_DOCUMENTS);
        assert_eq!(saved.groups.as_ref().unwrap().len(), MAX_VIEWS);
        assert!(
            saved
                .sticky
                .as_ref()
                .unwrap()
                .iter()
                .flatten()
                .all(|flag| !flag)
        );
        let restored = restore(&saved, &[]).unwrap();
        assert_eq!(restored.documents.len(), MAX_DOCUMENTS);
        assert!(restored.documents.iter().all(|doc| !doc.dirty()));
        assert_eq!(fs::read(&store.path).unwrap(), original);
        assert_eq!(
            store.publish(&saved).unwrap_err().to_string(),
            "Session metadata exceeds 1 MiB"
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
        let lease = regular_open(&store.lease_path, true).unwrap();
        assert!(matches!(
            lease.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        assert!(fs::read_dir(&store.directory).unwrap().all(|entry| {
            let path = entry.unwrap().path();
            path.extension()
                .is_some_and(|extension| extension == "json" || extension == "lock")
        }));
        for path in paths {
            assert_eq!(fs::read(path).unwrap(), "猫🙂\r\n".as_bytes());
        }
    }
}
