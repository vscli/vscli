//! Bounded editor geometry. Memberships, documents and filesystem work belong to callers.
mod spatial;
use crate::editor_groups::GroupId;
use anyhow::{Context, Result, ensure};
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_LEAVES: usize = 4;
pub const MAX_NODES: usize = 7;
pub const MAX_DEPTH: usize = 3;
pub const MAX_WEIGHT: u32 = 1_000_000;
pub const PREFERRED_COLUMNS: u16 = 16;
pub const PREFERRED_ROWS: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Columns,
    Rows,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
impl Direction {
    fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Columns,
            Self::Up | Self::Down => Axis::Rows,
        }
    }
    fn before(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

/// Persistence describes group indices, never runtime group/divider identities.
/// The caller must bound encoded metadata before deserialization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SavedNode {
    Leaf {
        group: usize,
    },
    Split {
        axis: Axis,
        first_weight: u32,
        second_weight: u32,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitId(u64);
impl SplitId {
    pub fn value(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Debug, Eq)]
pub struct DividerProof {
    identity: Arc<()>,
    generation: u64,
    split: SplitId,
}
impl DividerProof {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn split(&self) -> SplitId {
        self.split
    }
}
impl PartialEq for DividerProof {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
            && self.generation == other.generation
            && self.split == other.split
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Leaf(GroupId),
    Split {
        id: SplitId,
        axis: Axis,
        weights: (u32, u32),
        first: Box<Node>,
        second: Box<Node>,
    },
}
impl Node {
    fn contains(&self, group: GroupId) -> bool {
        match self {
            Self::Leaf(id) => *id == group,
            Self::Split { first, second, .. } => first.contains(group) || second.contains(group),
        }
    }
    fn has_divider(&self, wanted: SplitId) -> bool {
        match self {
            Self::Split {
                id, first, second, ..
            } => *id == wanted || first.has_divider(wanted) || second.has_divider(wanted),
            Self::Leaf(_) => false,
        }
    }
    fn visit(&self, visitor: &mut impl FnMut(GroupId)) {
        match self {
            Self::Leaf(id) => visitor(*id),
            Self::Split { first, second, .. } => {
                first.visit(visitor);
                second.visit(visitor);
            }
        }
    }
    fn units(&self, axis: Axis) -> u32 {
        match self {
            Self::Split {
                axis: current,
                first,
                second,
                ..
            } if *current == axis => first.units(axis) + second.units(axis),
            _ => 1,
        }
    }
    fn minimum(&self, preferred: bool) -> (u16, u16) {
        match self {
            Self::Leaf(_) => {
                if preferred {
                    (PREFERRED_COLUMNS, PREFERRED_ROWS)
                } else {
                    (1, 1)
                }
            }
            Self::Split {
                axis,
                first,
                second,
                ..
            } => {
                let (aw, ah) = first.minimum(preferred);
                let (bw, bh) = second.minimum(preferred);
                match axis {
                    Axis::Columns => (aw + bw + 1, ah.max(bh)),
                    Axis::Rows => (aw.max(bw), ah + bh + 1),
                }
            }
        }
    }
    fn split(&mut self, group: GroupId, new: GroupId, direction: Direction, id: SplitId) {
        match self {
            Self::Leaf(old) if *old == group => {
                let (first, second) = if direction.before() {
                    (new, *old)
                } else {
                    (*old, new)
                };
                *self = Self::Split {
                    id,
                    axis: direction.axis(),
                    weights: (1, 1),
                    first: Box::new(Self::Leaf(first)),
                    second: Box::new(Self::Leaf(second)),
                };
            }
            Self::Split { first, second, .. } => {
                if first.contains(group) {
                    first.split(group, new, direction, id);
                } else {
                    second.split(group, new, direction, id);
                }
            }
            _ => unreachable!("split was preflighted"),
        }
    }
    fn remove(self, group: GroupId) -> Option<Self> {
        match self {
            Self::Leaf(id) => (id != group).then_some(Self::Leaf(id)),
            Self::Split {
                id,
                axis,
                weights,
                first,
                second,
            } => match (first.remove(group), second.remove(group)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    id,
                    axis,
                    weights,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (first, second) => first.or(second),
            },
        }
    }
    fn reset(&mut self) {
        if let Self::Split {
            axis,
            weights,
            first,
            second,
            ..
        } = self
        {
            *weights = reduced(first.units(*axis), second.units(*axis));
            first.reset();
            second.reset();
        }
    }
    fn divider_mut(&mut self, wanted: SplitId) -> Option<&mut Self> {
        match self {
            Self::Split { id, .. } if *id == wanted => Some(self),
            Self::Split { first, second, .. } => first
                .divider_mut(wanted)
                .or_else(|| second.divider_mut(wanted)),
            _ => None,
        }
    }
    fn nearest(&self, group: GroupId, axis: Axis) -> Option<(SplitId, bool)> {
        match self {
            Self::Leaf(_) => None,
            Self::Split {
                id,
                axis: current,
                first,
                second,
                ..
            } => {
                let before = first.contains(group);
                let child = if before { first } else { second };
                child
                    .nearest(group, axis)
                    .or_else(|| (*current == axis).then_some((*id, before)))
            }
        }
    }
    fn saved(&self, groups: &[GroupId]) -> SavedNode {
        match self {
            Self::Leaf(group) => SavedNode::Leaf {
                group: groups.iter().position(|id| id == group).unwrap(),
            },
            Self::Split {
                axis,
                weights,
                first,
                second,
                ..
            } => SavedNode::Split {
                axis: *axis,
                first_weight: weights.0,
                second_weight: weights.1,
                first: Box::new(first.saved(groups)),
                second: Box::new(second.saved(groups)),
            },
        }
    }
    fn matches_saved(&self, saved: &SavedNode, groups: &[GroupId]) -> bool {
        match (self, saved) {
            (Self::Leaf(id), SavedNode::Leaf { group }) => groups[*group] == *id,
            (
                Self::Split {
                    axis,
                    weights,
                    first,
                    second,
                    ..
                },
                SavedNode::Split {
                    axis: other,
                    first_weight,
                    second_weight,
                    first: a,
                    second: b,
                },
            ) => {
                axis == other
                    && *weights == reduced(*first_weight, *second_weight)
                    && first.matches_saved(a, groups)
                    && second.matches_saved(b, groups)
            }
            _ => false,
        }
    }
}

fn reduced(a: u32, b: u32) -> (u32, u32) {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    (a / x, b / x)
}
fn checked_next(value: u64, what: &str) -> Result<u64> {
    value
        .checked_add(1)
        .with_context(|| format!("Editor layout {what} exhausted"))
}
fn check_groups(groups: &[GroupId]) -> Result<()> {
    ensure!(
        groups.len() <= MAX_LEAVES,
        "Editor layout exceeds four groups"
    );
    for (index, group) in groups.iter().enumerate() {
        ensure!(
            !groups[..index].contains(group),
            "Duplicate editor layout group"
        );
    }
    Ok(())
}
/// Validate persisted layout metadata without allocating runtime group identities.
/// Callers must bound encoded metadata before deserialization. This checks the
/// complete tree, including exact appearance order, before model loading.
pub fn validate_saved_layout(saved: Option<&SavedNode>, group_count: usize) -> Result<()> {
    validate_saved_wire(saved, group_count).map(|_| ())
}

fn validate_saved(saved: Option<&SavedNode>, groups: &[GroupId]) -> Result<usize> {
    check_groups(groups)?;
    validate_saved_wire(saved, groups.len())
}

fn validate_saved_wire(saved: Option<&SavedNode>, groups: usize) -> Result<usize> {
    ensure!(groups <= MAX_LEAVES, "Editor layout exceeds four groups");
    if groups == 0 {
        ensure!(saved.is_none(), "Empty groups require an empty layout");
        return Ok(0);
    }
    let saved = saved.context("Nonempty groups require an editor layout")?;
    fn walk(
        node: &SavedNode,
        depth: usize,
        count: &mut usize,
        seen: &mut Vec<usize>,
        groups: usize,
    ) -> Result<()> {
        ensure!(
            depth <= MAX_DEPTH,
            "Editor layout exceeds three branch levels"
        );
        *count += 1;
        ensure!(*count <= MAX_NODES, "Editor layout exceeds seven nodes");
        match node {
            SavedNode::Leaf { group } => {
                ensure!(
                    *group < groups && !seen.contains(group),
                    "Invalid or duplicate layout group index"
                );
                seen.push(*group);
            }
            SavedNode::Split {
                first_weight,
                second_weight,
                first,
                second,
                ..
            } => {
                ensure!(
                    (1..=MAX_WEIGHT).contains(first_weight)
                        && (1..=MAX_WEIGHT).contains(second_weight),
                    "Invalid editor layout weight"
                );
                walk(first, depth + 1, count, seen, groups)?;
                walk(second, depth + 1, count, seen, groups)?;
            }
        }
        Ok(())
    }
    let mut count = 0;
    let mut seen = Vec::with_capacity(MAX_LEAVES);
    walk(saved, 0, &mut count, &mut seen, groups)?;
    ensure!(seen.len() == groups, "Editor layout omits a group");
    ensure!(
        seen.iter().copied().eq(0..groups),
        "Layout group order differs from appearance order"
    );
    Ok((count - 1) / 2)
}
fn from_saved(saved: &SavedNode, groups: &[GroupId], next: &mut u64) -> Node {
    match saved {
        SavedNode::Leaf { group } => Node::Leaf(groups[*group]),
        SavedNode::Split {
            axis,
            first_weight,
            second_weight,
            first,
            second,
        } => {
            let id = SplitId(*next);
            *next += 1; // The complete allocation range was checked before construction.
            Node::Split {
                id,
                axis: *axis,
                weights: reduced(*first_weight, *second_weight),
                first: Box::new(from_saved(first, groups, next)),
                second: Box::new(from_saved(second, groups, next)),
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    identity: Arc<()>,
    root: Option<Node>,
    generation: u64,
    next_split: u64,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            identity: Arc::new(()),
            root: None,
            generation: 0,
            next_split: 1,
        }
    }
}
/// A fully staged replacement. Callers can preflight membership changes before commit.
#[derive(Debug)]
pub struct Plan {
    identity: Arc<()>,
    origin: u64,
    next: Layout,
}
impl Plan {
    pub fn changed(&self) -> bool {
        self.origin != self.next.generation
    }
    pub fn projected(&self) -> &Layout {
        &self.next
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub group: GroupId,
    pub outer: Rect,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divider {
    pub proof: DividerProof,
    pub axis: Axis,
    pub rect: Rect,
    pub parent: Rect,
    pub first_cells: u16,
    pub usable_cells: u16,
    minimum_first: u16,
    minimum_second: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Geometry {
    identity: Arc<()>,
    generation: u64,
    area: Rect,
    placements: [Option<Placement>; MAX_LEAVES],
    dividers: [Option<Divider>; MAX_LEAVES - 1],
    active_only: bool,
}
impl Geometry {
    /// Numeric generations alone cannot distinguish independently staged forks.
    pub fn same_revision(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity) && self.generation == other.generation
    }
    pub fn area(&self) -> Rect {
        self.area
    }
    pub fn placements(&self) -> &[Option<Placement>; MAX_LEAVES] {
        &self.placements
    }
    pub fn dividers(&self) -> &[Option<Divider>; MAX_LEAVES - 1] {
        &self.dividers
    }
    pub fn active_only(&self) -> bool {
        self.active_only
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn placement(&self, group: GroupId) -> Option<&Placement> {
        self.placements.iter().flatten().find(|p| p.group == group)
    }
}
impl Layout {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn geometry_current(&self, geometry: &Geometry) -> bool {
        Arc::ptr_eq(&self.identity, &geometry.identity) && self.generation == geometry.generation
    }
    pub fn divider_current(&self, proof: &DividerProof) -> bool {
        Arc::ptr_eq(&proof.identity, &self.identity)
            && proof.generation == self.generation
            && self
                .root
                .as_ref()
                .is_some_and(|root| root.has_divider(proof.split))
    }
    pub fn groups(&self) -> Vec<GroupId> {
        let mut groups = Vec::with_capacity(MAX_LEAVES);
        if let Some(root) = &self.root {
            root.visit(&mut |id| groups.push(id));
        }
        groups
    }
    pub fn validate(&self, groups: &[GroupId]) -> Result<()> {
        check_groups(groups)?;
        ensure!(
            self.groups() == groups,
            "Editor layout and membership group order differ"
        );
        Ok(())
    }
    fn plan(&self, mut next: Self) -> Result<Plan> {
        if next.root != self.root {
            next.generation = checked_next(self.generation, "generation")?;
            // A staged clone can diverge at the same numeric generation. A
            // fresh immutable revision identity fences those forks as well.
            next.identity = Arc::new(());
        }
        Ok(Plan {
            identity: self.identity.clone(),
            origin: self.generation,
            next,
        })
    }
    pub fn commit(&mut self, plan: Plan) -> Result<bool> {
        ensure!(
            Arc::ptr_eq(&plan.identity, &self.identity) && plan.origin == self.generation,
            "Editor layout plan was retired"
        );
        let changed = plan.changed();
        *self = plan.next;
        Ok(changed)
    }
    pub fn prepare_import(&self, groups: &[GroupId], saved: Option<&SavedNode>) -> Result<Plan> {
        let branches = validate_saved(saved, groups)?;
        let same = match (&self.root, saved) {
            (None, None) => true,
            (Some(root), Some(saved)) => root.matches_saved(saved, groups),
            _ => false,
        };
        if same {
            return self.plan(self.clone());
        }
        checked_next(self.generation, "generation")?;
        self.next_split
            .checked_add(branches as u64)
            .context("Editor layout divider identities exhausted")?;
        let mut next = self.clone();
        next.root = saved.map(|saved| from_saved(saved, groups, &mut next.next_split));
        self.plan(next)
    }
    pub fn export(&self, groups: &[GroupId]) -> Result<Option<SavedNode>> {
        self.validate(groups)?;
        Ok(self.root.as_ref().map(|root| root.saved(groups)))
    }
    pub fn prepare_flat(&self, groups: &[GroupId], axis: Axis) -> Result<Plan> {
        check_groups(groups)?;
        fn flat(index: usize, count: usize, axis: Axis) -> SavedNode {
            if count == 1 {
                SavedNode::Leaf { group: index }
            } else {
                SavedNode::Split {
                    axis,
                    first_weight: 1,
                    second_weight: (count - 1) as u32,
                    first: Box::new(SavedNode::Leaf { group: index }),
                    second: Box::new(flat(index + 1, count - 1, axis)),
                }
            }
        }
        let saved = (!groups.is_empty()).then(|| flat(0, groups.len(), axis));
        self.prepare_import(groups, saved.as_ref())
    }
    pub fn prepare_split(
        &self,
        source: GroupId,
        new_group: GroupId,
        direction: Direction,
    ) -> Result<Plan> {
        let root = self.root.as_ref().context("No editor group to split")?;
        ensure!(root.contains(source), "Source editor group was closed");
        ensure!(!root.contains(new_group), "New editor group already exists");
        ensure!(
            self.groups().len() < MAX_LEAVES,
            "Editor group limit reached (4)"
        );
        checked_next(self.generation, "generation")?;
        let next_id = checked_next(self.next_split, "divider identities")?;
        let mut next = self.clone();
        next.root
            .as_mut()
            .unwrap()
            .split(source, new_group, direction, SplitId(self.next_split));
        next.next_split = next_id;
        self.plan(next)
    }
    pub fn prepare_remove(&self, group: GroupId) -> Result<Plan> {
        ensure!(
            self.root.as_ref().is_some_and(|root| root.contains(group)),
            "Editor group was closed"
        );
        checked_next(self.generation, "generation")?;
        let mut next = self.clone();
        next.root = next.root.take().and_then(|root| root.remove(group));
        self.plan(next)
    }
    pub fn prepare_reset(&self) -> Result<Plan> {
        let mut next = self.clone();
        if let Some(root) = &mut next.root {
            root.reset();
        }
        self.plan(next)
    }
    pub fn prepare_resize(
        &self,
        proof: &DividerProof,
        first_cells: i32,
        geometry: &Geometry,
    ) -> Result<Plan> {
        ensure!(
            Arc::ptr_eq(&proof.identity, &self.identity)
                && Arc::ptr_eq(&geometry.identity, &self.identity)
                && proof.generation == self.generation
                && geometry.generation == self.generation,
            "Editor divider geometry was retired"
        );
        let divider = geometry
            .dividers
            .iter()
            .flatten()
            .find(|d| &d.proof == proof)
            .context("Editor divider is not visible")?;
        let first = first_cells.clamp(
            i32::from(divider.minimum_first),
            i32::from(divider.usable_cells - divider.minimum_second),
        ) as u16;
        if first == divider.first_cells {
            return self.plan(self.clone());
        }
        let mut next = self.clone();
        let Node::Split { weights, .. } = next
            .root
            .as_mut()
            .and_then(|root| root.divider_mut(proof.split))
            .context("Editor divider was closed")?
        else {
            unreachable!()
        };
        *weights = reduced(u32::from(first), u32::from(divider.usable_cells - first));
        self.plan(next)
    }
    /// Callers choose documented terminal cell increments; no CSS pixel conversion occurs here.
    pub fn prepare_resize_group(
        &self,
        group: GroupId,
        axis: Axis,
        delta: i16,
        geometry: &Geometry,
    ) -> Result<Plan> {
        let root = self.root.as_ref().context("No editor group to resize")?;
        ensure!(root.contains(group), "Editor group was closed");
        ensure!(
            Arc::ptr_eq(&geometry.identity, &self.identity)
                && geometry.generation == self.generation,
            "Editor geometry was retired"
        );
        let Some((id, first)) = root.nearest(group, axis) else {
            return self.plan(self.clone());
        };
        let proof = DividerProof {
            identity: self.identity.clone(),
            generation: self.generation,
            split: id,
        };
        let divider = geometry
            .dividers
            .iter()
            .flatten()
            .find(|d| d.proof == proof)
            .context("Editor divider is not visible")?;
        let delta = if first {
            i32::from(delta)
        } else {
            -i32::from(delta)
        };
        self.prepare_resize(&proof, i32::from(divider.first_cells) + delta, geometry)
    }
    pub fn project(&self, area: Rect, active: Option<GroupId>) -> Result<Geometry> {
        ensure!(
            area.x.checked_add(area.width).is_some() && area.y.checked_add(area.height).is_some(),
            "Editor geometry coordinate overflow"
        );
        let mut geometry = Geometry {
            identity: self.identity.clone(),
            generation: self.generation,
            area,
            placements: [None; MAX_LEAVES],
            dividers: std::array::from_fn(|_| None),
            active_only: false,
        };
        let Some(root) = &self.root else {
            ensure!(active.is_none(), "Empty layout has an active group");
            return Ok(geometry);
        };
        let active = active.context("Editor layout requires an active group")?;
        ensure!(root.contains(active), "Active editor group was closed");
        let (width, height) = root.minimum(false);
        if area.width < width || area.height < height {
            geometry.active_only = true;
            let mut index = 0;
            root.visit(&mut |group| {
                geometry.placements[index] = Some(Placement {
                    group,
                    outer: if group == active {
                        area
                    } else {
                        Rect::new(area.x, area.y, 0, 0)
                    },
                });
                index += 1;
            });
        } else {
            allocate(root, area, &mut geometry, &mut 0, &mut 0);
        }
        Ok(geometry)
    }
}

fn allocate(
    node: &Node,
    area: Rect,
    geometry: &mut Geometry,
    leaf: &mut usize,
    branch: &mut usize,
) {
    match node {
        Node::Leaf(group) => {
            geometry.placements[*leaf] = Some(Placement {
                group: *group,
                outer: area,
            });
            *leaf += 1;
        }
        Node::Split {
            id,
            axis,
            weights,
            first,
            second,
        } => {
            let length = match axis {
                Axis::Columns => area.width,
                Axis::Rows => area.height,
            };
            let usable = length - 1;
            let dimension = |node: &Node, preferred| {
                let (width, height) = node.minimum(preferred);
                match axis {
                    Axis::Columns => width,
                    Axis::Rows => height,
                }
            };
            let preferred = dimension(first, true) + dimension(second, true) <= usable;
            let minimum_first = dimension(first, preferred);
            let minimum_second = dimension(second, preferred);
            let weighted = (u64::from(usable) * u64::from(weights.0)
                / (u64::from(weights.0) + u64::from(weights.1))) as u16;
            let first_cells = weighted.clamp(minimum_first, usable - minimum_second);
            let second_cells = usable - first_cells;
            let (a, divider, b) = match axis {
                Axis::Columns => (
                    Rect::new(area.x, area.y, first_cells, area.height),
                    Rect::new(area.x + first_cells, area.y, 1, area.height),
                    Rect::new(area.x + first_cells + 1, area.y, second_cells, area.height),
                ),
                Axis::Rows => (
                    Rect::new(area.x, area.y, area.width, first_cells),
                    Rect::new(area.x, area.y + first_cells, area.width, 1),
                    Rect::new(area.x, area.y + first_cells + 1, area.width, second_cells),
                ),
            };
            geometry.dividers[*branch] = Some(Divider {
                proof: DividerProof {
                    identity: geometry.identity.clone(),
                    generation: geometry.generation,
                    split: *id,
                },
                axis: *axis,
                rect: divider,
                parent: area,
                first_cells,
                usable_cells: usable,
                minimum_first,
                minimum_second,
            });
            *branch += 1;
            allocate(first, a, geometry, leaf, branch);
            allocate(second, b, geometry, leaf, branch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_groups::Groups;

    fn ids(count: usize) -> Vec<GroupId> {
        let mut groups = Groups::default();
        if count > 0 {
            groups.open(1).unwrap();
            for _ in 1..count {
                groups.split_active().unwrap();
            }
        }
        groups.groups().iter().map(|g| g.id()).collect()
    }
    fn flat(groups: &[GroupId], axis: Axis) -> Layout {
        let mut layout = Layout::default();
        layout
            .commit(layout.prepare_flat(groups, axis).unwrap())
            .unwrap();
        layout
    }
    fn nested(groups: &[GroupId]) -> Layout {
        let mut layout = flat(&groups[..1], Axis::Columns);
        layout
            .commit(
                layout
                    .prepare_split(groups[0], groups[1], Direction::Right)
                    .unwrap(),
            )
            .unwrap();
        layout
            .commit(
                layout
                    .prepare_split(groups[1], groups[2], Direction::Down)
                    .unwrap(),
            )
            .unwrap();
        layout
    }
    fn rect(geometry: &Geometry, group: GroupId) -> Rect {
        geometry.placement(group).unwrap().outer
    }
    fn assert_partition(geometry: &Geometry) {
        let regions: Vec<_> = geometry
            .placements
            .iter()
            .flatten()
            .map(|p| p.outer)
            .chain(geometry.dividers.iter().flatten().map(|d| d.rect))
            .collect();
        let mut cells = 0u64;
        for (index, region) in regions.iter().enumerate() {
            assert!(region.x >= geometry.area.x && region.y >= geometry.area.y);
            assert!(
                u32::from(region.x) + u32::from(region.width)
                    <= u32::from(geometry.area.x) + u32::from(geometry.area.width)
            );
            assert!(
                u32::from(region.y) + u32::from(region.height)
                    <= u32::from(geometry.area.y) + u32::from(geometry.area.height)
            );
            cells += u64::from(region.width) * u64::from(region.height);
            for previous in &regions[..index] {
                let overlap = region.intersection(*previous);
                assert!(
                    overlap.width == 0 || overlap.height == 0,
                    "overlapping hit regions: {region:?}, {previous:?}"
                );
            }
        }
        assert_eq!(
            cells,
            u64::from(geometry.area.width) * u64::from(geometry.area.height)
        );
    }

    #[test]
    fn single_leaf_and_tiny_geometry_reject_equal_generation_foreign_revisions() {
        let groups = ids(2);
        let origin = Layout::default();
        let mut first = origin.clone();
        let mut foreign = origin.clone();
        first
            .commit(first.prepare_flat(&groups[..1], Axis::Columns).unwrap())
            .unwrap();
        foreign
            .commit(foreign.prepare_flat(&groups[..1], Axis::Columns).unwrap())
            .unwrap();
        let a = first
            .project(Rect::new(0, 0, 1, 1), Some(groups[0]))
            .unwrap();
        let b = foreign
            .project(Rect::new(0, 0, 1, 1), Some(groups[0]))
            .unwrap();
        assert_eq!(a.generation(), b.generation());
        assert_eq!(a.placements(), b.placements());
        assert!(a.dividers().iter().all(Option::is_none));
        assert!(first.geometry_current(&a));
        assert!(!first.geometry_current(&b));
        assert!(!a.same_revision(&b));
        assert!(a.same_revision(&a.clone()));
        first
            .commit(
                first
                    .prepare_split(groups[0], groups[1], Direction::Right)
                    .unwrap(),
            )
            .unwrap();
        let tiny = first
            .project(Rect::new(0, 0, 1, 1), Some(groups[1]))
            .unwrap();
        assert!(tiny.active_only());
        assert!(first.geometry_current(&tiny));
        assert!(!first.geometry_current(&a));
    }

    #[test]
    fn right_then_down_is_local_and_collapse_retains_the_surviving_subtree() {
        let groups = ids(3);
        let mut layout = nested(&groups);
        let area = Rect::new(3, 5, 101, 21);
        let geometry = layout.project(area, Some(groups[2])).unwrap();
        assert_eq!(rect(&geometry, groups[0]), Rect::new(3, 5, 50, 21));
        assert_eq!(rect(&geometry, groups[1]), Rect::new(54, 5, 50, 10));
        assert_eq!(rect(&geometry, groups[2]), Rect::new(54, 16, 50, 10));
        assert_eq!(layout.groups(), groups);
        let survivor = geometry.dividers[1].as_ref().unwrap().proof.split;
        let old_proof = geometry.dividers[0].as_ref().unwrap().proof.clone();
        let plan = layout.prepare_remove(groups[0]).unwrap();
        assert_eq!(layout.groups(), groups, "preparation is observational");
        assert_eq!(plan.projected().groups(), groups[1..]);
        layout.commit(plan).unwrap();
        let collapsed = layout.project(area, Some(groups[2])).unwrap();
        assert_eq!(
            collapsed.dividers[0].as_ref().unwrap().proof.split,
            survivor
        );
        assert_eq!(collapsed.dividers[0].as_ref().unwrap().axis, Axis::Rows);
        assert!(layout.prepare_resize(&old_proof, 20, &geometry).is_err());
        assert_partition(&collapsed);
    }

    #[test]
    fn left_and_up_preserve_ids_and_reorder_only_appearance() {
        let groups = ids(3);
        let mut layout = flat(&groups[..1], Axis::Columns);
        layout
            .commit(
                layout
                    .prepare_split(groups[0], groups[1], Direction::Left)
                    .unwrap(),
            )
            .unwrap();
        layout
            .commit(
                layout
                    .prepare_split(groups[0], groups[2], Direction::Up)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(layout.groups(), vec![groups[1], groups[2], groups[0]]);
        let geometry = layout
            .project(Rect::new(0, 0, 101, 21), Some(groups[0]))
            .unwrap();
        assert_eq!(rect(&geometry, groups[1]), Rect::new(0, 0, 50, 21));
        assert_eq!(rect(&geometry, groups[2]), Rect::new(51, 0, 50, 10));
        assert_partition(&geometry);
        assert!(
            layout.export(&groups).is_err(),
            "serialized order cannot silently disagree"
        );
        let order = layout.groups();
        let saved = layout.export(&order).unwrap();
        let mut imported = Layout::default();
        imported
            .commit(imported.prepare_import(&order, saved.as_ref()).unwrap())
            .unwrap();
        assert_eq!(imported.export(&order).unwrap(), saved);
    }

    #[test]
    fn all_four_leaf_split_shapes_partition_small_and_extreme_areas() {
        let groups = ids(4);
        for first in [Direction::Right, Direction::Down] {
            for second in [Direction::Right, Direction::Down] {
                for last in [0, 1, 2] {
                    let mut layout = flat(&groups[..1], Axis::Columns);
                    layout
                        .commit(layout.prepare_split(groups[0], groups[1], first).unwrap())
                        .unwrap();
                    layout
                        .commit(layout.prepare_split(groups[1], groups[2], second).unwrap())
                        .unwrap();
                    layout
                        .commit(
                            layout
                                .prepare_split(groups[last], groups[3], Direction::Down)
                                .unwrap(),
                        )
                        .unwrap();
                    for width in 0..=40 {
                        for height in 0..=15 {
                            assert_partition(
                                &layout
                                    .project(Rect::new(7, 9, width, height), Some(groups[3]))
                                    .unwrap(),
                            );
                        }
                    }
                    assert_partition(
                        &layout
                            .project(Rect::new(0, 0, u16::MAX, u16::MAX), Some(groups[0]))
                            .unwrap(),
                    );
                    // Rect::new clamps overflowing dimensions. The public fields
                    // can still carry malformed input, which projection must reject.
                    for malformed in [
                        Rect {
                            x: u16::MAX,
                            y: 0,
                            width: 1,
                            height: 1,
                        },
                        Rect {
                            x: 0,
                            y: u16::MAX,
                            width: 1,
                            height: 1,
                        },
                    ] {
                        assert!(layout.project(malformed, Some(groups[0])).is_err());
                    }
                    assert_partition(
                        &layout
                            .project(Rect::new(u16::MAX, 0, 1, 1), Some(groups[0]))
                            .unwrap(),
                    );
                }
            }
        }
    }

    #[test]
    fn preferred_minimum_clamps_and_tiny_fallback_never_changes_layout() {
        let groups = ids(2);
        let mut layout = flat(&groups, Axis::Columns);
        let before = layout.clone();
        let geometry = layout
            .project(Rect::new(0, 0, 101, 12), Some(groups[0]))
            .unwrap();
        let divider = geometry.dividers[0].as_ref().unwrap();
        layout
            .commit(
                layout
                    .prepare_resize(&divider.proof, i32::MIN, &geometry)
                    .unwrap(),
            )
            .unwrap();
        let resized = layout.project(geometry.area, Some(groups[0])).unwrap();
        assert_eq!(rect(&resized, groups[0]).width, PREFERRED_COLUMNS);
        let divider = resized.dividers[0].as_ref().unwrap();
        layout
            .commit(
                layout
                    .prepare_resize(&divider.proof, i32::MAX, &resized)
                    .unwrap(),
            )
            .unwrap();
        let maximum = layout.project(geometry.area, Some(groups[0])).unwrap();
        assert_eq!(rect(&maximum, groups[1]).width, PREFERRED_COLUMNS);
        let unchanged = layout.clone();
        let tiny = layout
            .project(Rect::new(4, 8, 2, 1), Some(groups[1]))
            .unwrap();
        assert!(tiny.active_only);
        assert_eq!(rect(&tiny, groups[1]), tiny.area);
        assert_eq!(rect(&tiny, groups[0]), Rect::new(4, 8, 0, 0));
        assert!(tiny.dividers.iter().all(Option::is_none));
        assert!(
            layout
                .prepare_resize(&maximum.dividers[0].as_ref().unwrap().proof, 1, &tiny)
                .is_err()
        );
        assert_eq!(layout, unchanged);
        assert_ne!(layout, before);
        assert_partition(&tiny);
    }

    #[test]
    fn flat_migration_and_reset_preserve_equal_columns_not_equal_leaf_area() {
        let groups = ids(4);
        for axis in [Axis::Columns, Axis::Rows] {
            let layout = flat(&groups, axis);
            let area = match axis {
                Axis::Columns => Rect::new(0, 0, 83, 12),
                Axis::Rows => Rect::new(0, 0, 60, 43),
            };
            let geometry = layout.project(area, Some(groups[0])).unwrap();
            let expected = match axis {
                Axis::Columns => 20,
                Axis::Rows => 10,
            };
            for group in &groups {
                let r = rect(&geometry, *group);
                assert_eq!(
                    match axis {
                        Axis::Columns => r.width,
                        Axis::Rows => r.height,
                    },
                    expected
                );
            }
            let saved = layout.export(&groups).unwrap();
            let mut restored = Layout::default();
            restored
                .commit(restored.prepare_import(&groups, saved.as_ref()).unwrap())
                .unwrap();
            assert_eq!(restored.export(&groups).unwrap(), saved);
        }
        let mut layout = nested(&groups[..3]);
        let area = Rect::new(0, 0, 101, 21);
        let geometry = layout.project(area, Some(groups[0])).unwrap();
        layout
            .commit(
                layout
                    .prepare_resize(&geometry.dividers[0].as_ref().unwrap().proof, 30, &geometry)
                    .unwrap(),
            )
            .unwrap();
        layout.commit(layout.prepare_reset().unwrap()).unwrap();
        let reset = layout.project(area, Some(groups[0])).unwrap();
        assert_eq!(rect(&reset, groups[0]).width, 50);
        assert_eq!(rect(&reset, groups[1]).width, 50);
        assert_eq!(rect(&reset, groups[2]).width, 50);
        assert!(!layout.prepare_reset().unwrap().changed());
    }

    #[test]
    fn resize_group_is_axis_local_and_preserves_other_branch_weights() {
        let groups = ids(3);
        let mut layout = nested(&groups);
        let area = Rect::new(0, 0, 101, 31);
        let geometry = layout.project(area, Some(groups[2])).unwrap();
        layout
            .commit(
                layout
                    .prepare_resize_group(groups[2], Axis::Rows, 2, &geometry)
                    .unwrap(),
            )
            .unwrap();
        let resized = layout.project(area, Some(groups[2])).unwrap();
        assert_eq!(rect(&resized, groups[0]), rect(&geometry, groups[0]));
        assert_eq!(
            rect(&resized, groups[2]).height,
            rect(&geometry, groups[2]).height + 2
        );
        assert_eq!(
            rect(&resized, groups[1]).height,
            rect(&geometry, groups[1]).height - 2
        );
        assert_partition(&resized);
    }

    #[test]
    fn stale_divider_aba_cross_instance_and_staged_plan_are_rejected() {
        let groups = ids(2);
        let mut layout = flat(&groups, Axis::Columns);
        let area = Rect::new(0, 0, 101, 12);
        let initial = layout.project(area, Some(groups[0])).unwrap();
        let old_proof = initial.dividers[0].as_ref().unwrap().proof.clone();
        let old_plan = layout.prepare_resize(&old_proof, 30, &initial).unwrap();
        layout
            .commit(layout.prepare_resize(&old_proof, 20, &initial).unwrap())
            .unwrap();
        let changed = layout.project(area, Some(groups[0])).unwrap();
        layout
            .commit(
                layout
                    .prepare_resize(&changed.dividers[0].as_ref().unwrap().proof, 50, &changed)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            rect(&layout.project(area, Some(groups[0])).unwrap(), groups[0]),
            rect(&initial, groups[0])
        );
        assert!(layout.prepare_resize(&old_proof, 30, &initial).is_err());
        assert!(layout.commit(old_plan).is_err());
        let other = flat(&groups, Axis::Columns);
        let foreign = other.project(area, Some(groups[0])).unwrap();
        assert!(
            layout
                .prepare_resize(&foreign.dividers[0].as_ref().unwrap().proof, 30, &foreign)
                .is_err()
        );
        let mut empty = Layout::default();
        assert!(
            empty
                .commit(
                    Layout::default()
                        .prepare_flat(&groups, Axis::Columns)
                        .unwrap()
                )
                .is_err()
        );

        // Independently staged clones can have equal numeric generations.
        // Their immutable revision identities must still fence each other.
        let mut a = flat(&groups, Axis::Columns);
        let mut b = a.clone();
        let shared = a.project(area, Some(groups[0])).unwrap();
        let proof = &shared.dividers[0].as_ref().unwrap().proof;
        a.commit(a.prepare_resize(proof, 20, &shared).unwrap())
            .unwrap();
        b.commit(b.prepare_resize(proof, 30, &shared).unwrap())
            .unwrap();
        assert_eq!(a.generation(), b.generation());
        let ga = a.project(area, Some(groups[0])).unwrap();
        let gb = b.project(area, Some(groups[0])).unwrap();
        let foreign = &ga.dividers[0].as_ref().unwrap().proof;
        assert!(!b.divider_current(foreign));
        assert!(b.prepare_resize(foreign, 40, &ga).is_err());
        assert!(
            b.commit(a.prepare_resize(foreign, 40, &ga).unwrap())
                .is_err()
        );
        assert!(b.divider_current(&gb.dividers[0].as_ref().unwrap().proof));
    }

    #[test]
    fn fifth_actual_group_id_and_equivalent_wire_do_not_change_capacity_or_proofs() {
        let mut engine = Groups::default();
        engine.open(1).unwrap();
        for _ in 1..4 {
            engine.split_active().unwrap();
        }
        let groups: Vec<_> = engine.groups().iter().map(|group| group.id()).collect();
        engine
            .close_group(&engine.group_proof(groups[3]).unwrap())
            .unwrap();
        let fresh = engine.split_active().unwrap().created_groups[0];
        let layout = flat(&groups, Axis::Columns);
        let before = layout.clone();
        assert!(
            layout
                .prepare_split(groups[0], fresh, Direction::Down)
                .is_err()
        );
        let mut too_many = groups.clone();
        too_many.push(fresh);
        assert!(layout.prepare_flat(&too_many, Axis::Columns).is_err());
        assert_eq!(layout, before);
        let mut two = flat(&groups[..2], Axis::Columns);
        let geometry = two
            .project(Rect::new(0, 0, 101, 12), Some(groups[0]))
            .unwrap();
        let mut saved = two.export(&groups[..2]).unwrap().unwrap();
        let SavedNode::Split {
            first_weight,
            second_weight,
            ..
        } = &mut saved
        else {
            unreachable!()
        };
        *first_weight = 100;
        *second_weight = 100;
        let plan = two.prepare_import(&groups[..2], Some(&saved)).unwrap();
        assert!(!plan.changed());
        assert!(!two.commit(plan).unwrap());
        assert!(two.divider_current(&geometry.dividers[0].as_ref().unwrap().proof));
    }

    #[test]
    fn malformed_late_wire_and_exhausted_counters_preserve_all_prior_state() {
        let groups = ids(4);
        let mut layout = nested(&groups[..3]);
        let original = layout.clone();
        let mut late = flat(&groups, Axis::Columns)
            .export(&groups)
            .unwrap()
            .unwrap();
        let SavedNode::Split { second, .. } = &mut late else {
            unreachable!()
        };
        let SavedNode::Split { second, .. } = second.as_mut() else {
            unreachable!()
        };
        let SavedNode::Split { second, .. } = second.as_mut() else {
            unreachable!()
        };
        **second = SavedNode::Leaf { group: 0 };
        assert!(layout.prepare_import(&groups, Some(&late)).is_err());
        assert_eq!(layout, original);
        for weight in [0, MAX_WEIGHT + 1, u32::MAX] {
            let mut invalid = flat(&groups[..2], Axis::Columns)
                .export(&groups[..2])
                .unwrap()
                .unwrap();
            let SavedNode::Split { first_weight, .. } = &mut invalid else {
                unreachable!()
            };
            *first_weight = weight;
            assert!(layout.prepare_import(&groups[..2], Some(&invalid)).is_err());
            assert_eq!(layout, original);
        }
        assert!(layout.prepare_import(&groups, None).is_err());
        assert!(
            layout
                .prepare_flat(&[groups[0], groups[0]], Axis::Columns)
                .is_err()
        );
        let mut deep = SavedNode::Leaf { group: 0 };
        for _ in 0..8 {
            deep = SavedNode::Split {
                axis: Axis::Rows,
                first_weight: 1,
                second_weight: 1,
                first: Box::new(deep),
                second: Box::new(SavedNode::Leaf { group: 1 }),
            };
        }
        assert!(
            layout
                .prepare_import(&groups, Some(&deep))
                .unwrap_err()
                .to_string()
                .contains("three branch levels")
        );
        assert_eq!(layout, original);
        assert!(
            serde_json::from_str::<SavedNode>(r#"{"kind":"leaf","group":0,"runtime_id":1}"#)
                .is_err()
        );
        layout.generation = u64::MAX;
        let before = layout.clone();
        assert!(layout.prepare_remove(groups[0]).is_err());
        assert!(
            layout
                .prepare_split(groups[0], groups[3], Direction::Right)
                .is_err()
        );
        assert_eq!(layout, before);
        layout.generation = original.generation;
        layout.next_split = u64::MAX;
        let before = layout.clone();
        assert!(
            layout
                .prepare_split(groups[0], groups[3], Direction::Right)
                .is_err()
        );
        assert!(layout.prepare_flat(&groups, Axis::Rows).is_err());
        assert_eq!(layout, before);
    }

    #[test]
    fn removing_final_group_and_importing_empty_are_exact_noop_or_empty_states() {
        let groups = ids(1);
        let mut layout = flat(&groups, Axis::Columns);
        layout
            .commit(layout.prepare_remove(groups[0]).unwrap())
            .unwrap();
        assert!(layout.groups().is_empty());
        assert_eq!(layout.export(&[]).unwrap(), None);
        assert!(!layout.prepare_import(&[], None).unwrap().changed());
        assert!(
            layout
                .prepare_import(&[], Some(&SavedNode::Leaf { group: 0 }))
                .is_err()
        );
        assert!(
            layout
                .project(Rect::new(0, 0, 3, 3), Some(groups[0]))
                .is_err()
        );
        let geometry = layout.project(Rect::new(0, 0, 3, 3), None).unwrap();
        assert!(geometry.placements.iter().all(Option::is_none));
        assert!(geometry.dividers.iter().all(Option::is_none));
    }

    fn wire_split(first: SavedNode, second: SavedNode) -> SavedNode {
        SavedNode::Split {
            axis: Axis::Columns,
            first_weight: 1,
            second_weight: MAX_WEIGHT,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    #[test]
    fn count_only_wire_accepts_complete_ordered_metadata_without_runtime_ids() {
        let wire = wire_split(
            SavedNode::Leaf { group: 0 },
            wire_split(
                SavedNode::Leaf { group: 1 },
                wire_split(SavedNode::Leaf { group: 2 }, SavedNode::Leaf { group: 3 }),
            ),
        );
        let before = wire.clone();
        validate_saved_layout(Some(&wire), 4).unwrap();
        assert_eq!(wire, before);
        validate_saved_layout(Some(&SavedNode::Leaf { group: 0 }), 1).unwrap();
    }

    #[test]
    fn count_only_wire_rejects_late_duplicate_and_nested_weight_without_mutation() {
        let valid = wire_split(
            SavedNode::Leaf { group: 0 },
            wire_split(SavedNode::Leaf { group: 1 }, SavedNode::Leaf { group: 2 }),
        );
        validate_saved_layout(Some(&valid), 3).unwrap();
        let mut duplicate = valid.clone();
        let SavedNode::Split { second, .. } = &mut duplicate else {
            unreachable!()
        };
        let SavedNode::Split { second, .. } = second.as_mut() else {
            unreachable!()
        };
        **second = SavedNode::Leaf { group: 0 };
        let before = duplicate.clone();
        assert_eq!(
            validate_saved_layout(Some(&duplicate), 3)
                .unwrap_err()
                .to_string(),
            "Invalid or duplicate layout group index"
        );
        assert_eq!(duplicate, before);
        let mut invalid_weight = valid.clone();
        let SavedNode::Split { second, .. } = &mut invalid_weight else {
            unreachable!()
        };
        let SavedNode::Split { second_weight, .. } = second.as_mut() else {
            unreachable!()
        };
        *second_weight = 0;
        let before = invalid_weight.clone();
        assert_eq!(
            validate_saved_layout(Some(&invalid_weight), 3)
                .unwrap_err()
                .to_string(),
            "Invalid editor layout weight"
        );
        assert_eq!(invalid_weight, before);
        validate_saved_layout(Some(&valid), 3).unwrap();
    }

    #[test]
    fn count_only_wire_requires_exact_empty_inventory_and_complete_appearance_order() {
        validate_saved_layout(None, 0).unwrap();
        let leaf = SavedNode::Leaf { group: 0 };
        assert_eq!(
            validate_saved_layout(Some(&leaf), 0)
                .unwrap_err()
                .to_string(),
            "Empty groups require an empty layout"
        );
        assert_eq!(
            validate_saved_layout(None, 1).unwrap_err().to_string(),
            "Nonempty groups require an editor layout"
        );
        assert_eq!(
            validate_saved_layout(Some(&leaf), 2)
                .unwrap_err()
                .to_string(),
            "Editor layout omits a group"
        );
        let reversed = wire_split(SavedNode::Leaf { group: 1 }, leaf);
        assert_eq!(
            validate_saved_layout(Some(&reversed), 2)
                .unwrap_err()
                .to_string(),
            "Layout group order differs from appearance order"
        );
        let out_of_range = wire_split(SavedNode::Leaf { group: 0 }, SavedNode::Leaf { group: 2 });
        assert_eq!(
            validate_saved_layout(Some(&out_of_range), 2)
                .unwrap_err()
                .to_string(),
            "Invalid or duplicate layout group index"
        );
    }

    #[test]
    fn count_only_wire_bounds_admission_before_traversing_or_loading_models() {
        for count in [MAX_LEAVES + 1, usize::MAX] {
            assert_eq!(
                validate_saved_layout(None, count).unwrap_err().to_string(),
                "Editor layout exceeds four groups"
            );
        }
        let mut deep = SavedNode::Leaf { group: 0 };
        for _ in 0..MAX_DEPTH + 1 {
            deep = wire_split(deep, SavedNode::Leaf { group: 1 });
        }
        assert_eq!(
            validate_saved_layout(Some(&deep), 4)
                .unwrap_err()
                .to_string(),
            "Editor layout exceeds three branch levels"
        );
        let four = wire_split(
            wire_split(SavedNode::Leaf { group: 0 }, SavedNode::Leaf { group: 1 }),
            wire_split(SavedNode::Leaf { group: 2 }, SavedNode::Leaf { group: 3 }),
        );
        validate_saved_layout(Some(&four), 4).unwrap();
        let too_many_nodes = wire_split(four, SavedNode::Leaf { group: 0 });
        assert_eq!(
            validate_saved_layout(Some(&too_many_nodes), 4)
                .unwrap_err()
                .to_string(),
            "Editor layout exceeds seven nodes"
        );
    }
}
