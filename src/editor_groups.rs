//! Bounded editor-group membership. Documents and dirty-close decisions belong to App.
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_RECENT_RESERVATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    Committed,
    Preview,
}

pub const MAX_GROUPS: usize = 4;
pub const MAX_TABS_PER_GROUP: usize = 128;
pub const MAX_MEMBERSHIPS: usize = MAX_GROUPS * MAX_TABS_PER_GROUP;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GroupId(u64);
impl GroupId {
    pub fn value(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);
impl TabId {
    pub fn value(self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub group: GroupId,
    pub tab: TabId,
    pub document: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    id: TabId,
    document: u64,
    preview: bool,
    sticky: bool,
}
impl Tab {
    pub fn is_preview(&self) -> bool {
        self.preview
    }
    pub fn is_sticky(&self) -> bool {
        self.sticky
    }
    pub fn id(&self) -> TabId {
        self.id
    }
    pub fn document(&self) -> u64 {
        self.document
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    id: GroupId,
    tabs: Vec<Tab>,
    active: TabId,
    recent: Vec<TabId>,
    membership_generation: u64,
    preview: Option<TabId>,
    sticky_count: usize,
}
impl Group {
    pub fn sticky_count(&self) -> usize {
        self.sticky_count
    }
    pub fn preview(&self) -> Option<Membership> {
        let preview = self.preview?;
        self.tabs
            .iter()
            .find(|tab| tab.id == preview)
            .map(|tab| self.membership(tab))
    }
    pub fn id(&self) -> GroupId {
        self.id
    }
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }
    pub fn active(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == self.active)
    }
    pub fn recent(&self) -> &[TabId] {
        &self.recent
    }
    pub fn membership_generation(&self) -> u64 {
        self.membership_generation
    }
    fn membership(&self, tab: &Tab) -> Membership {
        Membership {
            group: self.id,
            tab: tab.id,
            document: tab.document,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiProof {
    generation: u64,
    active: Option<Membership>,
}
impl UiProof {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn active(&self) -> Option<Membership> {
        self.active
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupProof {
    group: GroupId,
    generation: u64,
    members: Vec<Membership>,
}
impl GroupProof {
    pub fn group(&self) -> GroupId {
        self.group
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn members(&self) -> &[Membership] {
        &self.members
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreGroup {
    pub documents: Vec<u64>,
    pub active: usize,
    pub recent: Vec<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigate {
    Next,
    Previous,
    NextInGroup,
    PreviousInGroup,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub changed: bool,
    pub previous: Option<Membership>,
    pub active: Option<Membership>,
    pub inserted: Vec<Membership>,
    pub removed: Vec<Membership>,
    pub created_groups: Vec<GroupId>,
    pub removed_groups: Vec<GroupId>,
    pub promoted: Vec<Membership>,
    pub sticky_changed: Vec<Membership>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Groups {
    groups: Vec<Group>,
    active: Option<GroupId>,
    generation: u64,
    next_group: u64,
    next_tab: u64,
    recent_groups: Vec<GroupId>,
    recent_memberships: Vec<Membership>,
}
impl Default for Groups {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            active: None,
            generation: 0,
            next_group: 1,
            next_tab: 1,
            recent_groups: Vec::new(),
            recent_memberships: Vec::new(),
        }
    }
}
struct Admission {
    group: usize,
    existing: Option<usize>,
    replacement: Option<usize>,
    promote: Option<usize>,
    generation: u64,
    membership_generation: Option<u64>,
    next_tab: Option<u64>,
}

fn increment(value: u64, what: &str) -> Result<u64> {
    value
        .checked_add(1)
        .with_context(|| format!("Editor-group {what} exhausted"))
}
impl Groups {
    /// Actual activation order, independent of sequential layout order.
    pub fn group_mru(&self) -> &[GroupId] {
        &self.recent_groups
    }
    pub fn membership_mru(&self) -> &[Membership] {
        &self.recent_memberships
    }
    /// Deterministic allocator-refusal oracle at actual admission, after logical
    /// preflight. Thread local so concurrent fixture engines cannot interfere.
    #[cfg(test)]
    pub(crate) fn fail_next_recent_reservation(&self) {
        FAIL_NEXT_RECENT_RESERVATION.with(|fail| fail.set(true));
    }

    fn reserve_recent(&mut self, members: usize, groups: usize) -> Result<()> {
        ensure!(
            self.recent_memberships
                .len()
                .checked_add(members)
                .is_some_and(|length| length <= MAX_MEMBERSHIPS),
            "Editor MRU exceeds 512 memberships"
        );
        ensure!(
            self.recent_groups
                .len()
                .checked_add(groups)
                .is_some_and(|length| length <= MAX_GROUPS),
            "Editor group MRU exceeds four groups"
        );
        #[cfg(test)]
        if (members != 0 || groups != 0)
            && FAIL_NEXT_RECENT_RESERVATION.with(|fail| fail.replace(false))
        {
            anyhow::bail!("Cannot reserve editor MRU storage (injected allocation refusal)");
        }
        self.recent_memberships
            .try_reserve(members)
            .context("Cannot reserve editor MRU storage")?;
        self.recent_groups
            .try_reserve(groups)
            .context("Cannot reserve group MRU storage")?;
        Ok(())
    }
    /// Called only after validated open/focus/close/split publication. Front
    /// no-ops do no allocation or shifting; mode-only changes never call it.
    fn touch_active(&mut self) {
        let Some(member) = self.active_membership() else {
            return;
        };
        if self.recent_groups.first() != Some(&member.group) {
            self.recent_groups.retain(|group| *group != member.group);
            self.recent_groups.insert(0, member.group);
        }
        if self.recent_memberships.first() != Some(&member) {
            self.recent_memberships.retain(|current| *current != member);
            self.recent_memberships.insert(0, member);
        }
    }
    /// Same-group protection takes precedence over cross-group global MRU.
    /// Unknown/removed groups have no eligible target.
    pub fn next_nonsticky_recent(&self, id: GroupId) -> Option<Membership> {
        let group = self.group(id)?;
        group.recent.iter().find_map(|id| {
            let tab = group.tabs.iter().find(|tab| tab.id == *id)?;
            (!tab.sticky).then(|| group.membership(tab))
        })
    }
    pub fn next_nonsticky_recent_any_group(&self) -> Option<Membership> {
        self.recent_memberships.iter().find_map(|member| {
            let group = self.group(member.group)?;
            if group.sticky_count == group.tabs.len() {
                return None;
            }
            let tab = group
                .tabs
                .iter()
                .find(|tab| tab.id == member.tab && tab.document == member.document)?;
            (!tab.sticky).then_some(*member)
        })
    }
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }
    pub fn active_group(&self) -> Option<GroupId> {
        self.active
    }
    pub fn active_membership(&self) -> Option<Membership> {
        let group = self.group(self.active?)?;
        Some(group.membership(group.active()?))
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn proof(&self) -> UiProof {
        UiProof {
            generation: self.generation,
            active: self.active_membership(),
        }
    }
    pub fn proof_current(&self, proof: &UiProof) -> bool {
        self.generation == proof.generation && self.active_membership() == proof.active
    }
    pub fn group_proof(&self, id: GroupId) -> Result<GroupProof> {
        let group = self.group(id).context("Editor group was closed")?;
        Ok(GroupProof {
            group: id,
            generation: group.membership_generation,
            members: group.tabs.iter().map(|tab| group.membership(tab)).collect(),
        })
    }
    pub fn group_proof_current(&self, proof: &GroupProof) -> bool {
        self.group(proof.group).is_some_and(|group| {
            group.membership_generation == proof.generation
                && group.tabs.len() == proof.members.len()
                && group
                    .tabs
                    .iter()
                    .zip(&proof.members)
                    .all(|(tab, member)| group.membership(tab) == *member)
        })
    }
    pub fn membership_current(&self, member: Membership) -> bool {
        self.locate(member).is_ok()
    }
    pub fn memberships(&self, document: u64) -> impl Iterator<Item = Membership> + '_ {
        self.groups.iter().flat_map(move |group| {
            group
                .tabs
                .iter()
                .filter(move |tab| tab.document == document)
                .map(move |tab| group.membership(tab))
        })
    }
    fn locate(&self, member: Membership) -> Result<(usize, usize)> {
        let group = self
            .groups
            .iter()
            .position(|group| group.id == member.group)
            .context("Editor group was closed")?;
        let tab = self.groups[group]
            .tabs
            .iter()
            .position(|tab| tab.id == member.tab && tab.document == member.document)
            .context("Editor tab membership changed")?;
        Ok((group, tab))
    }
    fn unchanged(&self) -> Change {
        let active = self.active_membership();
        Change {
            previous: active,
            active,
            ..Change::default()
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture_exhaust_membership_generation(&mut self, id: GroupId) {
        self.groups
            .iter_mut()
            .find(|group| group.id == id)
            .unwrap()
            .membership_generation = u64::MAX;
    }
    pub fn can_open(&self, document: u64) -> Result<()> {
        self.can_open_mode(document, OpenMode::Committed, None)
    }
    pub fn can_open_in_group(&self, id: GroupId, document: u64) -> Result<()> {
        self.can_open_in_group_mode(id, document, OpenMode::Committed, None)
    }
    pub fn can_open_mode(
        &self,
        document: u64,
        mode: OpenMode,
        replacement: Option<Membership>,
    ) -> Result<()> {
        ensure!(document != 0, "Invalid document identity");
        if let Some(group) = self.active {
            return self.can_open_in_group_mode(group, document, mode, replacement);
        }
        ensure!(replacement.is_none(), "No editor preview to replace");
        increment(self.generation, "interaction generation")?;
        increment(self.next_group, "group identities")?;
        increment(self.next_tab, "tab identities")?;
        Ok(())
    }
    pub fn can_open_in_group_mode(
        &self,
        id: GroupId,
        document: u64,
        mode: OpenMode,
        replacement: Option<Membership>,
    ) -> Result<()> {
        self.admission(id, document, mode, replacement).map(|_| ())
    }
    fn admission(
        &self,
        id: GroupId,
        document: u64,
        mode: OpenMode,
        replacement: Option<Membership>,
    ) -> Result<Admission> {
        ensure!(document != 0, "Invalid document identity");
        let index = self
            .groups
            .iter()
            .position(|group| group.id == id)
            .context("Editor group was closed")?;
        let group = &self.groups[index];
        let existing = group.tabs.iter().position(|tab| tab.document == document);
        let replaced = if let Some(member) = replacement {
            ensure!(
                mode == OpenMode::Preview && existing.is_none(),
                "Replacement requires a new preview target"
            );
            ensure!(
                member.group == id,
                "Replacement belongs to another editor group"
            );
            let (_, position) = self.locate(member)?;
            let tab = &group.tabs[position];
            ensure!(
                tab.preview && group.preview == Some(tab.id),
                "Replacement editor is not a preview"
            );
            Some(position)
        } else {
            None
        };
        let promote = match existing {
            Some(position) if mode == OpenMode::Committed && group.tabs[position].preview => {
                Some(position)
            }
            None if mode == OpenMode::Preview && replaced.is_none() => group
                .preview
                .and_then(|preview| group.tabs.iter().position(|tab| tab.id == preview)),
            _ => None,
        };
        if existing.is_none() {
            ensure!(
                group.tabs.len() - usize::from(replaced.is_some()) < MAX_TABS_PER_GROUP,
                "Editor group exceeds 128 tabs"
            );
        }
        let changed = existing.is_none()
            || promote.is_some()
            || self.active != Some(id)
            || existing.is_some_and(|position| group.active != group.tabs[position].id);
        let generation = if changed {
            increment(self.generation, "interaction generation")?
        } else {
            self.generation
        };
        let membership_generation = if existing.is_none() || promote.is_some() {
            Some(increment(
                group.membership_generation,
                "membership generation",
            )?)
        } else {
            None
        };
        let next_tab = if existing.is_none() {
            Some(increment(self.next_tab, "tab identities")?)
        } else {
            None
        };
        Ok(Admission {
            group: index,
            existing,
            replacement: replaced,
            promote,
            generation,
            membership_generation,
            next_tab,
        })
    }
    pub fn open(&mut self, document: u64) -> Result<Change> {
        self.open_mode(document, OpenMode::Committed, None)
    }
    pub fn open_in_group(&mut self, id: GroupId, document: u64) -> Result<Change> {
        self.open_in_group_mode(id, document, OpenMode::Committed, None)
    }
    pub fn open_mode(
        &mut self,
        document: u64,
        mode: OpenMode,
        replacement: Option<Membership>,
    ) -> Result<Change> {
        if let Some(group) = self.active {
            return self.open_in_group_mode(group, document, mode, replacement);
        }
        self.can_open_mode(document, mode, replacement)?;
        self.reserve_recent(1, 1)?;
        let group = GroupId(self.next_group);
        let tab = TabId(self.next_tab);
        let member = Membership {
            group,
            tab,
            document,
        };
        self.groups.push(Group {
            id: group,
            tabs: vec![Tab {
                id: tab,
                document,
                preview: mode == OpenMode::Preview,
                sticky: false,
            }],
            active: tab,
            recent: vec![tab],
            membership_generation: 1,
            preview: (mode == OpenMode::Preview).then_some(tab),
            sticky_count: 0,
        });
        self.active = Some(group);
        self.touch_active();
        self.next_group += 1;
        self.next_tab += 1;
        self.generation += 1;
        Ok(Change {
            changed: true,
            active: Some(member),
            inserted: vec![member],
            created_groups: vec![group],
            ..Change::default()
        })
    }
    pub fn open_in_group_mode(
        &mut self,
        id: GroupId,
        document: u64,
        mode: OpenMode,
        replacement: Option<Membership>,
    ) -> Result<Change> {
        let admission = self.admission(id, document, mode, replacement)?;
        if admission.generation == self.generation {
            return Ok(self.unchanged());
        }
        self.reserve_recent(
            usize::from(admission.existing.is_none() && admission.replacement.is_none()),
            0,
        )?;
        let previous = self.active_membership();
        let mut promoted = Vec::new();
        let mut removed = Vec::new();
        let mut inserted = Vec::new();
        let fresh_tab = TabId(self.next_tab);
        let group = &mut self.groups[admission.group];
        if let Some(position) = admission.promote {
            let member = group.membership(&group.tabs[position]);
            group.tabs[position].preview = false;
            group.preview = None;
            promoted.push(member);
        }
        let tab = if let Some(position) = admission.existing {
            group.tabs[position].id
        } else {
            // Compute right-of-active position in the original group, then
            // account for removal. Replacing the active preview preserves its
            // slot rather than transiently focusing a different MRU editor.
            let mut position = group
                .tabs
                .iter()
                .position(|tab| tab.id == group.active)
                .unwrap()
                + 1;
            position = position.max(group.sticky_count);
            if let Some(old) = admission.replacement {
                let member = group.membership(&group.tabs[old]);
                let removed_tab = group.tabs.remove(old);
                group.recent.retain(|tab| *tab != removed_tab.id);
                group.preview = None;
                if old < position {
                    position -= 1;
                }
                removed.push(member);
            }
            let member = Membership {
                group: id,
                tab: fresh_tab,
                document,
            };
            group.tabs.insert(
                position,
                Tab {
                    id: fresh_tab,
                    document,
                    preview: mode == OpenMode::Preview,
                    sticky: false,
                },
            );
            if mode == OpenMode::Preview {
                group.preview = Some(fresh_tab);
            }
            inserted.push(member);
            fresh_tab
        };
        group.active = tab;
        if group.recent.first() != Some(&tab) {
            group.recent.retain(|current| *current != tab);
            group.recent.insert(0, tab);
        }
        if let Some(generation) = admission.membership_generation {
            group.membership_generation = generation;
        }
        if let Some(next_tab) = admission.next_tab {
            self.next_tab = next_tab;
        }
        self.generation = admission.generation;
        self.active = Some(id);
        for member in &removed {
            self.recent_memberships.retain(|current| current != member);
        }
        self.touch_active();
        Ok(Change {
            changed: true,
            previous,
            active: Some(Membership {
                group: id,
                tab,
                document,
            }),
            inserted,
            removed,
            promoted,
            ..Change::default()
        })
    }
    /// Sticky mode is membership-local and always committed. Reordering does
    /// not focus an inactive target, edit its document or change either MRU.
    pub fn set_sticky(&mut self, member: Membership, sticky: bool) -> Result<Change> {
        let (index, position) = self.locate(member)?;
        if self.groups[index].tabs[position].sticky == sticky {
            return Ok(self.unchanged());
        }
        let generation = increment(self.generation, "interaction generation")?;
        let membership_generation = increment(
            self.groups[index].membership_generation,
            "membership generation",
        )?;
        let previous = self.active_membership();
        let group = &mut self.groups[index];
        let destination = if sticky {
            group.sticky_count
        } else {
            group.sticky_count - 1
        };
        let mut tab = group.tabs.remove(position);
        let promoted = if tab.preview {
            vec![member]
        } else {
            Vec::new()
        };
        if tab.preview {
            tab.preview = false;
            group.preview = None;
        }
        tab.sticky = sticky;
        group.tabs.insert(destination, tab);
        if sticky {
            group.sticky_count += 1;
        } else {
            group.sticky_count -= 1;
        }
        group.membership_generation = membership_generation;
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            promoted,
            sticky_changed: vec![member],
            ..Change::default()
        })
    }
    /// Promote in place; mode changes retire UI and captured group batch proofs.
    pub fn keep(&mut self, member: Membership) -> Result<Change> {
        let (index, tab) = self.locate(member)?;
        if !self.groups[index].tabs[tab].preview {
            return Ok(self.unchanged());
        }
        let generation = increment(self.generation, "interaction generation")?;
        let membership_generation = increment(
            self.groups[index].membership_generation,
            "membership generation",
        )?;
        let previous = self.active_membership();
        self.groups[index].tabs[tab].preview = false;
        self.groups[index].preview = None;
        self.groups[index].membership_generation = membership_generation;
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            promoted: vec![member],
            ..Change::default()
        })
    }
    /// Dirty-state observation is supplied by App; Undo never demotes a tab.
    pub fn promote_document(&mut self, document: u64) -> Result<Change> {
        ensure!(document != 0, "Invalid document identity");
        let mut targets = Vec::with_capacity(MAX_GROUPS);
        for (index, group) in self.groups.iter().enumerate() {
            if let Some(member) = group.preview().filter(|member| member.document == document) {
                let tab = group
                    .tabs
                    .iter()
                    .position(|tab| tab.id == member.tab)
                    .unwrap();
                let next = increment(group.membership_generation, "membership generation")?;
                targets.push((index, tab, member, next));
            }
        }
        if targets.is_empty() {
            return Ok(self.unchanged());
        }
        let generation = increment(self.generation, "interaction generation")?;
        let previous = self.active_membership();
        let mut promoted = Vec::with_capacity(targets.len());
        for (index, tab, member, next) in targets {
            self.groups[index].tabs[tab].preview = false;
            self.groups[index].preview = None;
            self.groups[index].membership_generation = next;
            promoted.push(member);
        }
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            promoted,
            ..Change::default()
        })
    }
    pub fn focus_group(&mut self, id: GroupId) -> Result<Change> {
        let group = self.group(id).context("Editor group was closed")?;
        self.focus(group.membership(group.active().expect("nonempty group")))
    }
    pub fn focus(&mut self, member: Membership) -> Result<Change> {
        let (index, _) = self.locate(member)?;
        if self.active == Some(member.group) && self.groups[index].active == member.tab {
            return Ok(self.unchanged());
        }
        let generation = increment(self.generation, "interaction generation")?;
        let previous = self.active_membership();
        let group = &mut self.groups[index];
        group.active = member.tab;
        if group.recent.first() != Some(&member.tab) {
            group.recent.retain(|id| *id != member.tab);
            group.recent.insert(0, member.tab);
        }
        self.active = Some(member.group);
        self.touch_active();
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: Some(member),
            ..Change::default()
        })
    }
    pub fn navigate(&mut self, direction: Navigate) -> Result<Change> {
        let Some(active) = self.active_membership() else {
            return Ok(self.unchanged());
        };
        let (mut group, mut tab) = self.locate(active)?;
        match direction {
            Navigate::NextInGroup => tab = (tab + 1) % self.groups[group].tabs.len(),
            Navigate::PreviousInGroup => {
                tab = (tab + self.groups[group].tabs.len() - 1) % self.groups[group].tabs.len();
            }
            Navigate::Next if tab + 1 < self.groups[group].tabs.len() => tab += 1,
            Navigate::Previous if tab > 0 => tab -= 1,
            Navigate::Next => {
                group = (group + 1) % self.groups.len();
                tab = 0;
            }
            Navigate::Previous => {
                group = (group + self.groups.len() - 1) % self.groups.len();
                tab = self.groups[group].tabs.len() - 1;
            }
        }
        self.focus(self.groups[group].membership(&self.groups[group].tabs[tab]))
    }
    pub fn split_active(&mut self) -> Result<Change> {
        let Some(previous) = self.active_membership() else {
            return Ok(self.unchanged());
        };
        ensure!(
            self.groups.len() < MAX_GROUPS,
            "Editor group limit reached (4)"
        );
        let generation = increment(self.generation, "interaction generation")?;
        let next_group = increment(self.next_group, "group identities")?;
        let next_tab = increment(self.next_tab, "tab identities")?;
        let (index, _) = self.locate(previous)?;
        self.reserve_recent(1, 1)?;
        let group = GroupId(self.next_group);
        let tab = TabId(self.next_tab);
        let member = Membership {
            group,
            tab,
            document: previous.document,
        };
        self.groups.insert(
            index + 1,
            Group {
                id: group,
                tabs: vec![Tab {
                    id: tab,
                    document: member.document,
                    preview: false,
                    sticky: false,
                }],
                active: tab,
                recent: vec![tab],
                membership_generation: 1,
                preview: None,
                sticky_count: 0,
            },
        );
        self.active = Some(group);
        self.touch_active();
        self.generation = generation;
        self.next_group = next_group;
        self.next_tab = next_tab;
        Ok(Change {
            changed: true,
            previous: Some(previous),
            active: Some(member),
            inserted: vec![member],
            created_groups: vec![group],
            ..Change::default()
        })
    }
    fn remove_group(&mut self, index: usize) -> Group {
        let removed = self.groups.remove(index);
        self.recent_groups.retain(|group| *group != removed.id);
        self.recent_memberships
            .retain(|member| member.group != removed.id);
        if self.active == Some(removed.id) {
            self.active = self
                .groups
                .get(index.min(self.groups.len().saturating_sub(1)))
                .map(|g| g.id);
        }
        removed
    }
    pub fn close(&mut self, member: Membership) -> Result<Change> {
        let (index, tab) = self.locate(member)?;
        let generation = increment(self.generation, "interaction generation")?;
        let remove_group = self.groups[index].tabs.len() == 1;
        let membership_generation = if remove_group {
            None
        } else {
            Some(increment(
                self.groups[index].membership_generation,
                "membership generation",
            )?)
        };
        let previous = self.active_membership();
        if remove_group {
            self.remove_group(index);
        } else {
            let group = &mut self.groups[index];
            let removed = group.tabs.remove(tab);
            group.sticky_count -= usize::from(removed.sticky);
            if group.preview == Some(removed.id) {
                group.preview = None;
            }
            group.recent.retain(|id| *id != member.tab);
            if group.active == member.tab {
                group.active = group.recent[0];
            }
            group.membership_generation = membership_generation.unwrap();
        }
        self.recent_memberships.retain(|current| *current != member);
        self.touch_active();
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            removed: vec![member],
            removed_groups: if remove_group {
                vec![member.group]
            } else {
                Vec::new()
            },
            ..Change::default()
        })
    }
    pub fn close_group(&mut self, proof: &GroupProof) -> Result<Change> {
        ensure!(
            self.group_proof_current(proof),
            "Editor group membership changed"
        );
        let generation = increment(self.generation, "interaction generation")?;
        let previous = self.active_membership();
        let index = self
            .groups
            .iter()
            .position(|group| group.id == proof.group)
            .unwrap();
        let removed = self.remove_group(index);
        self.touch_active();
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            removed: removed
                .tabs
                .iter()
                .map(|tab| removed.membership(tab))
                .collect(),
            removed_groups: vec![removed.id],
            ..Change::default()
        })
    }
    /// Close an exact nonsticky subset under the complete ordered group proof.
    /// App owns dirty review/save continuations; this cannot close protected
    /// sticky peers. Removed entries are reported in original visual order.
    pub fn close_memberships(
        &mut self,
        proof: &GroupProof,
        targets: &[Membership],
    ) -> Result<Change> {
        ensure!(
            self.group_proof_current(proof),
            "Editor group membership changed"
        );
        ensure!(
            targets.len() <= MAX_TABS_PER_GROUP,
            "Editor close subset exceeds 128 tabs"
        );
        let index = self
            .groups
            .iter()
            .position(|group| group.id == proof.group)
            .unwrap();
        let mut selected = HashSet::new();
        for member in targets {
            ensure!(
                member.group == proof.group,
                "Close target belongs to another editor group"
            );
            let (group, tab) = self.locate(*member)?;
            ensure!(
                group == index && !self.groups[group].tabs[tab].sticky,
                "Close subset cannot remove sticky editors"
            );
            ensure!(selected.insert(member.tab), "Duplicate editor close target");
        }
        if selected.is_empty() {
            return Ok(self.unchanged());
        }
        let generation = increment(self.generation, "interaction generation")?;
        let remove_group = selected.len() == self.groups[index].tabs.len();
        let membership_generation = if remove_group {
            None
        } else {
            Some(increment(
                self.groups[index].membership_generation,
                "membership generation",
            )?)
        };
        let removed: Vec<_> = self.groups[index]
            .tabs
            .iter()
            .filter(|tab| selected.contains(&tab.id))
            .map(|tab| self.groups[index].membership(tab))
            .collect();
        let previous = self.active_membership();
        if remove_group {
            self.remove_group(index);
        } else {
            let group = &mut self.groups[index];
            group.tabs.retain(|tab| !selected.contains(&tab.id));
            group.recent.retain(|tab| !selected.contains(tab));
            if group.preview.is_some_and(|tab| selected.contains(&tab)) {
                group.preview = None;
            }
            if selected.contains(&group.active) {
                group.active = group.recent[0];
            }
            group.membership_generation = membership_generation.unwrap();
            self.recent_memberships
                .retain(|member| member.group != proof.group || !selected.contains(&member.tab));
        }
        self.touch_active();
        self.generation = generation;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            removed,
            removed_groups: if remove_group {
                vec![proof.group]
            } else {
                Vec::new()
            },
            ..Change::default()
        })
    }
    /// Install a validated ordered layout with fresh live IDs. No model/view mutation occurs here.
    pub fn import(&mut self, restored: &[RestoreGroup], active_group: usize) -> Result<Change> {
        ensure!(
            restored.len() <= MAX_GROUPS,
            "Editor group limit reached (4)"
        );
        if restored.is_empty() {
            ensure!(active_group == 0, "Invalid empty editor-group layout");
            if self.groups.is_empty() {
                return Ok(self.unchanged());
            }
        } else {
            ensure!(active_group < restored.len(), "Invalid active editor group");
        }
        let mut count = 0;
        for group in restored {
            ensure!(
                !group.documents.is_empty() && group.documents.len() <= MAX_TABS_PER_GROUP,
                "Editor group requires 1–128 tabs"
            );
            let mut documents = HashSet::new();
            ensure!(
                group
                    .documents
                    .iter()
                    .all(|id| *id != 0 && documents.insert(*id)),
                "Invalid or duplicate group document identity"
            );
            ensure!(
                group.active < group.documents.len(),
                "Invalid active editor tab"
            );
            let mut recent = HashSet::new();
            ensure!(
                group.recent.len() == group.documents.len()
                    && group.recent.first() == Some(&group.active)
                    && group
                        .recent
                        .iter()
                        .all(|index| *index < group.documents.len() && recent.insert(*index)),
                "Invalid recent editor-tab order"
            );
            count += group.documents.len();
        }
        ensure!(
            count <= MAX_MEMBERSHIPS,
            "Editor layout exceeds 512 memberships"
        );
        let generation = increment(self.generation, "interaction generation")?;
        let next_group = self
            .next_group
            .checked_add(restored.len() as u64)
            .context("Editor-group group identities exhausted")?;
        let next_tab = self
            .next_tab
            .checked_add(count as u64)
            .context("Editor-group tab identities exhausted")?;
        let mut tab_id = self.next_tab;
        let mut groups = Vec::with_capacity(restored.len());
        for (index, restored) in restored.iter().enumerate() {
            // The complete allocation range was checked before construction.
            let id = GroupId(self.next_group + index as u64);
            let tabs: Vec<Tab> = restored
                .documents
                .iter()
                .map(|document| {
                    let tab = Tab {
                        id: TabId(tab_id),
                        document: *document,
                        preview: false,
                        sticky: false,
                    };
                    tab_id += 1;
                    tab
                })
                .collect();
            let active = tabs[restored.active].id;
            let recent = restored
                .recent
                .iter()
                .map(|index| tabs[*index].id)
                .collect();
            groups.push(Group {
                id,
                tabs,
                active,
                recent,
                membership_generation: 1,
                preview: None,
                sticky_count: 0,
            });
        }
        let mut recent_groups = Vec::new();
        let mut recent_memberships = Vec::new();
        recent_groups
            .try_reserve(groups.len())
            .context("Cannot reserve group MRU storage")?;
        #[cfg(test)]
        if count != 0 && FAIL_NEXT_RECENT_RESERVATION.with(|fail| fail.replace(false)) {
            anyhow::bail!("Cannot reserve editor MRU storage (injected allocation refusal)");
        }
        recent_memberships
            .try_reserve(count)
            .context("Cannot reserve editor MRU storage")?;
        if !groups.is_empty() {
            recent_groups.push(groups[active_group].id);
            recent_groups.extend(
                groups
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != active_group)
                    .map(|(_, group)| group.id),
            );
            for id in &recent_groups {
                let group = groups.iter().find(|group| group.id == *id).unwrap();
                for tab in &group.recent {
                    recent_memberships.push(
                        group.membership(group.tabs.iter().find(|item| item.id == *tab).unwrap()),
                    );
                }
            }
        }
        let previous = self.active_membership();
        let removed_groups = self.groups.iter().map(|group| group.id).collect();
        let removed = self
            .groups
            .iter()
            .flat_map(|group| group.tabs.iter().map(|tab| group.membership(tab)))
            .collect();
        self.active = groups.get(active_group).map(|group| group.id);
        self.groups = groups;
        self.recent_groups = recent_groups;
        self.recent_memberships = recent_memberships;
        self.generation = generation;
        self.next_group = next_group;
        self.next_tab = next_tab;
        Ok(Change {
            changed: true,
            previous,
            active: self.active_membership(),
            inserted: self
                .groups
                .iter()
                .flat_map(|group| group.tabs.iter().map(|tab| group.membership(tab)))
                .collect(),
            removed,
            created_groups: self.groups.iter().map(|group| group.id).collect(),
            removed_groups,
            promoted: Vec::new(),
            sticky_changed: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_recent_reservation_refusal_preserves_engine_counters_memberships_and_mru() {
        let mut groups = Groups::default();
        groups.open(1).unwrap();
        let before = groups.clone();
        groups.can_open(2).unwrap();
        groups.fail_next_recent_reservation();
        assert!(
            groups
                .open(2)
                .unwrap_err()
                .to_string()
                .contains("injected allocation refusal")
        );
        assert_eq!(groups, before);
        groups.open(2).unwrap();
        assert_eq!(
            groups.active_membership().unwrap().tab.value(),
            before.next_tab
        );
        assert_eq!(groups.membership_mru()[0].document, 2);
        let before = groups.clone();
        groups.fail_next_recent_reservation();
        assert!(
            groups
                .split_active()
                .unwrap_err()
                .to_string()
                .contains("injected allocation refusal")
        );
        assert_eq!(groups, before);
        let split = groups.split_active().unwrap();
        assert_eq!(split.active.unwrap().group.value(), before.next_group);
        assert_eq!(split.active.unwrap().tab.value(), before.next_tab);
    }

    fn restored(documents: &[u64], active: usize) -> RestoreGroup {
        RestoreGroup {
            documents: documents.to_vec(),
            active,
            recent: std::iter::once(active)
                .chain((0..documents.len()).filter(|index| *index != active))
                .collect(),
        }
    }
    fn member(groups: &Groups, group: usize, document: u64) -> Membership {
        let group = &groups.groups()[group];
        group.membership(
            group
                .tabs()
                .iter()
                .find(|tab| tab.document() == document)
                .unwrap(),
        )
    }
    fn documents(groups: &Groups, group: usize) -> Vec<u64> {
        groups.groups()[group]
            .tabs()
            .iter()
            .map(Tab::document)
            .collect()
    }
    fn invariant(groups: &Groups) {
        assert!(groups.groups.len() <= MAX_GROUPS);
        let mut group_ids = HashSet::new();
        let mut tab_ids = HashSet::new();
        let mut total = 0;
        for group in &groups.groups {
            assert!(group.id.value() > 0 && group_ids.insert(group.id));
            assert!(!group.tabs.is_empty() && group.tabs.len() <= MAX_TABS_PER_GROUP);
            let mut document_ids = HashSet::new();
            for tab in &group.tabs {
                assert!(tab.id.value() > 0 && tab_ids.insert(tab.id));
                assert!(tab.document > 0 && document_ids.insert(tab.document));
            }
            let previews: Vec<_> = group.tabs.iter().filter(|tab| tab.is_preview()).collect();
            assert!(previews.len() <= 1);
            assert_eq!(group.preview, previews.first().map(|tab| tab.id));
            assert!(group.sticky_count <= group.tabs.len());
            for (index, tab) in group.tabs.iter().enumerate() {
                assert_eq!(tab.is_sticky(), index < group.sticky_count);
                assert!(!tab.is_sticky() || !tab.is_preview());
            }
            assert_eq!(group.preview().map(|member| member.tab), group.preview);
            assert_eq!(group.recent.first(), Some(&group.active));
            assert_eq!(group.recent.len(), group.tabs.len());
            let recent: HashSet<_> = group.recent.iter().copied().collect();
            assert_eq!(recent.len(), group.tabs.len());
            assert!(group.tabs.iter().all(|tab| recent.contains(&tab.id)));
            assert!(group.active().is_some());
            total += group.tabs.len();
        }
        assert!(total <= MAX_MEMBERSHIPS);
        assert_eq!(groups.recent_groups.len(), groups.groups.len());
        let recent_groups: HashSet<_> = groups.recent_groups.iter().copied().collect();
        assert_eq!(recent_groups.len(), groups.groups.len());
        assert!(
            groups
                .groups
                .iter()
                .all(|group| recent_groups.contains(&group.id))
        );
        assert_eq!(groups.recent_groups.first().copied(), groups.active);
        assert_eq!(groups.recent_memberships.len(), total);
        let recent_tabs: HashSet<_> = groups
            .recent_memberships
            .iter()
            .map(|member| member.tab)
            .collect();
        assert_eq!(recent_tabs.len(), total);
        assert!(
            groups
                .recent_memberships
                .iter()
                .all(|member| groups.membership_current(*member))
        );
        assert_eq!(
            groups.recent_memberships.first().copied(),
            groups.active_membership()
        );
        assert_eq!(groups.groups.is_empty(), groups.active.is_none());
        if let Some(active) = groups.active_membership() {
            assert!(groups.membership_current(active));
        } else {
            assert!(groups.groups.is_empty());
        }
    }

    #[test]
    fn opening_inserts_right_of_active_without_duplicate_models_or_identity_changes() {
        let mut groups = Groups::default();
        groups.can_open(1).unwrap();
        let a = groups.open(1).unwrap().active.unwrap();
        groups.open(2).unwrap();
        groups.focus(a).unwrap();
        groups.open(3).unwrap();
        assert_eq!(documents(&groups, 0), [1, 3, 2]);
        let generation = groups.generation();
        let change = groups.open(1).unwrap();
        assert_eq!(change.active, Some(a));
        assert!(change.inserted.is_empty());
        assert_eq!(groups.generation(), generation + 1);
        assert_eq!(documents(&groups, 0), [1, 3, 2]);
        let before = groups.clone();
        assert!(!groups.open(1).unwrap().changed);
        assert_eq!(groups, before);
        assert_eq!(groups.group(a.group).unwrap().recent()[0], a.tab);
        invariant(&groups);
    }

    #[test]
    fn split_copies_only_active_membership_and_tabs_remain_independent() {
        let mut groups = Groups::default();
        let a = groups.open(1).unwrap().active.unwrap();
        groups.open(2).unwrap();
        groups.focus(a).unwrap();
        let split = groups.split_active().unwrap();
        let shared = split.active.unwrap();
        assert_eq!(split.previous, Some(a));
        assert_eq!(split.created_groups, [shared.group]);
        assert_eq!(shared.document, a.document);
        assert_ne!(shared.tab, a.tab);
        groups.open(3).unwrap();
        groups.open(4).unwrap();
        assert_eq!(documents(&groups, 0), [1, 2]);
        assert_eq!(documents(&groups, 1), [1, 3, 4]);
        assert_eq!(groups.memberships(1).collect::<Vec<_>>(), [a, shared]);
        groups.focus_group(a.group).unwrap();
        assert_eq!(groups.active_membership(), Some(a));
        invariant(&groups);
    }

    #[test]
    fn sequential_navigation_crosses_groups_while_in_group_navigation_wraps() {
        let mut groups = Groups::default();
        groups
            .import(&[restored(&[1, 2], 0), restored(&[2, 3], 0)], 0)
            .unwrap();
        let a = member(&groups, 0, 1);
        let left_b = member(&groups, 0, 2);
        let right_b = member(&groups, 1, 2);
        let c = member(&groups, 1, 3);
        for expected in [left_b, right_b, c, a] {
            assert_eq!(
                groups.navigate(Navigate::Next).unwrap().active,
                Some(expected)
            );
        }
        for expected in [c, right_b, left_b, a] {
            assert_eq!(
                groups.navigate(Navigate::Previous).unwrap().active,
                Some(expected)
            );
        }
        for expected in [left_b, a, left_b] {
            assert_eq!(
                groups.navigate(Navigate::NextInGroup).unwrap().active,
                Some(expected)
            );
            assert_eq!(groups.active_group(), Some(a.group));
        }
        for expected in [a, left_b, a] {
            assert_eq!(
                groups.navigate(Navigate::PreviousInGroup).unwrap().active,
                Some(expected)
            );
        }
        let mut single = Groups::default();
        single.open(8).unwrap();
        let before = single.clone();
        assert!(!single.navigate(Navigate::Next).unwrap().changed);
        assert!(!single.navigate(Navigate::PreviousInGroup).unwrap().changed);
        assert_eq!(single, before);
        invariant(&groups);
    }

    #[test]
    fn closing_active_uses_mru_and_closing_inactive_group_does_not_steal_focus() {
        let mut groups = Groups::default();
        let a = groups.open(1).unwrap().active.unwrap();
        groups.open(2).unwrap();
        let c = groups.open(3).unwrap().active.unwrap();
        groups.focus(a).unwrap();
        let close = groups.close(a).unwrap();
        assert_eq!(close.removed, [a]);
        assert_eq!(close.active, Some(c)); // MRU, rather than adjacent B.
        assert_eq!(documents(&groups, 0), [2, 3]);
        let left = groups.group_proof(c.group).unwrap();
        let right = groups.split_active().unwrap().active.unwrap();
        let before = groups.active_membership();
        let removed = groups.close_group(&left).unwrap();
        assert_eq!(removed.removed, left.members());
        assert_eq!(removed.removed_groups, [c.group]);
        assert_eq!(groups.active_membership(), before);
        assert_eq!(groups.memberships(3).collect::<Vec<_>>(), [right]);
        invariant(&groups);
    }

    #[test]
    fn shared_close_keeps_other_membership_and_last_close_returns_true_empty_state() {
        let mut groups = Groups::default();
        let first = groups.open(1).unwrap().active.unwrap();
        let second = groups.split_active().unwrap().active.unwrap();
        groups.focus(first).unwrap();
        let closed = groups.close(first).unwrap();
        assert_eq!(closed.removed, [first]);
        assert_eq!(closed.active, Some(second));
        assert_eq!(groups.memberships(1).collect::<Vec<_>>(), [second]);
        assert!(groups.membership_current(second));
        let empty = groups.close(second).unwrap();
        assert_eq!(empty.removed, [second]);
        assert!(empty.active.is_none());
        assert!(groups.groups().is_empty());
        let before = groups.clone();
        assert!(!groups.split_active().unwrap().changed);
        assert!(!groups.navigate(Navigate::Next).unwrap().changed);
        assert_eq!(groups, before);
        let reopened = groups.open(1).unwrap().active.unwrap();
        assert_ne!(reopened.group, first.group);
        assert_ne!(reopened.tab, first.tab);
        invariant(&groups);
    }

    #[test]
    fn ui_aba_and_close_reopen_retire_old_proofs_without_retiring_surviving_models() {
        let mut groups = Groups::default();
        let a = groups.open(1).unwrap().active.unwrap();
        let b = groups.open(2).unwrap().active.unwrap();
        groups.focus(a).unwrap();
        let proof = groups.proof();
        let structural = groups.group_proof(a.group).unwrap();
        groups.focus(b).unwrap();
        groups.focus(a).unwrap();
        assert_eq!(groups.active_membership(), proof.active());
        assert!(!groups.proof_current(&proof));
        assert!(groups.group_proof_current(&structural));
        let shared = groups.split_active().unwrap().active.unwrap();
        groups.close(a).unwrap();
        assert!(groups.membership_current(shared));
        let replacement = groups.open_in_group(b.group, 1).unwrap().active.unwrap();
        assert_eq!(replacement.group, a.group);
        assert_eq!(replacement.document, a.document);
        assert_ne!(replacement.tab, a.tab);
        assert!(!groups.membership_current(a));
        assert!(!groups.group_proof_current(&structural));
        let before = groups.clone();
        assert!(
            groups
                .close(a)
                .unwrap_err()
                .to_string()
                .contains("membership changed")
        );
        assert!(
            groups
                .close_group(&structural)
                .unwrap_err()
                .to_string()
                .contains("membership changed")
        );
        assert_eq!(groups, before);
        assert!(groups.membership_current(replacement));
        invariant(&groups);
    }

    #[test]
    fn complete_capacity_preflight_preserves_state_and_existing_tab_focus_still_works() {
        let mut groups = Groups::default();
        let full = restored(&(1..=MAX_TABS_PER_GROUP as u64).collect::<Vec<_>>(), 127);
        groups.import(&vec![full; MAX_GROUPS], 3).unwrap();
        assert_eq!(
            groups
                .groups
                .iter()
                .map(|group| group.tabs.len())
                .sum::<usize>(),
            MAX_MEMBERSHIPS
        );
        let before = groups.clone();
        assert!(
            groups
                .can_open(129)
                .unwrap_err()
                .to_string()
                .contains("128 tabs")
        );
        assert!(
            groups
                .open(129)
                .unwrap_err()
                .to_string()
                .contains("128 tabs")
        );
        assert!(
            groups
                .split_active()
                .unwrap_err()
                .to_string()
                .contains("limit reached")
        );
        assert_eq!(groups, before);
        groups.can_open(1).unwrap();
        assert_eq!(groups.open(1).unwrap().active.unwrap().document, 1);
        invariant(&groups);
    }

    #[test]
    fn restore_is_atomic_for_late_malformed_groups_and_does_not_consume_id_ranges() {
        let mut groups = Groups::default();
        groups.open(10).unwrap();
        let before = groups.clone();
        let valid = restored(&[1, 2, 3], 1);
        let mut invalid = restored(&[4, 5], 0);
        invalid.documents[1] = 4;
        assert!(
            groups
                .import(&[valid.clone(), invalid], 0)
                .unwrap_err()
                .to_string()
                .contains("duplicate group document")
        );
        assert_eq!(groups, before);
        let mut invalid = restored(&[4, 5], 0);
        invalid.recent = vec![0, 0];
        assert!(
            groups
                .import(&[valid.clone(), invalid], 0)
                .unwrap_err()
                .to_string()
                .contains("recent editor-tab order")
        );
        assert_eq!(groups, before);
        let mut invalid = restored(&[4, 5], 0);
        invalid.active = 2;
        assert!(
            groups
                .import(&[valid.clone(), invalid], 0)
                .unwrap_err()
                .to_string()
                .contains("active editor tab")
        );
        assert_eq!(groups, before);
        assert!(
            groups
                .import(&[valid], 1)
                .unwrap_err()
                .to_string()
                .contains("active editor group")
        );
        assert!(
            groups
                .import(&[], 1)
                .unwrap_err()
                .to_string()
                .contains("empty editor-group layout")
        );
        assert_eq!(groups, before);
        invariant(&groups);
    }

    #[test]
    fn import_restores_shared_inventory_and_mru_with_fresh_ids_then_clears_atomically() {
        let mut groups = Groups::default();
        let old = groups.open(7).unwrap().active.unwrap();
        let mut right = restored(&[1, 2, 3], 2);
        right.recent = vec![2, 1, 0];
        let change = groups.import(&[restored(&[1, 4], 1), right], 1).unwrap();
        assert_eq!(change.removed, [old]);
        assert_eq!(change.removed_groups, [old.group]);
        assert!(!groups.membership_current(old));
        assert!(
            change
                .inserted
                .iter()
                .all(|new| new.group != old.group && new.tab != old.tab)
        );
        assert_eq!(documents(&groups, 0), [1, 4]);
        assert_eq!(documents(&groups, 1), [1, 2, 3]);
        assert_eq!(groups.active_membership().unwrap().document, 3);
        assert_eq!(groups.memberships(1).count(), 2);
        let close = groups.close(groups.active_membership().unwrap()).unwrap();
        assert_eq!(close.active.unwrap().document, 2);
        let old_members = groups
            .groups
            .iter()
            .flat_map(|group| group.tabs.iter().map(|tab| group.membership(tab)))
            .collect::<Vec<_>>();
        let cleared = groups.import(&[], 0).unwrap();
        assert_eq!(cleared.removed, old_members);
        assert!(cleared.active.is_none());
        invariant(&groups);
        let before = groups.clone();
        assert!(!groups.import(&[], 0).unwrap().changed);
        assert_eq!(groups, before);
    }

    #[test]
    fn identity_and_generation_exhaustion_never_partially_mutate_or_wrap() {
        for kind in 0..4 {
            let mut groups = Groups::default();
            let active = groups.open(1).unwrap().active.unwrap();
            match kind {
                0 => groups.generation = u64::MAX,
                1 => groups.next_tab = u64::MAX,
                2 => groups.groups[0].membership_generation = u64::MAX,
                _ => groups.next_group = u64::MAX,
            }
            let before = groups.clone();
            let error = if kind == 3 {
                groups.split_active()
            } else {
                groups.open(2)
            }
            .unwrap_err();
            assert!(error.to_string().contains("exhausted"));
            assert_eq!(groups, before);
            assert!(!groups.focus(active).unwrap().changed);
            assert_eq!(groups, before);
        }
        let mut groups = Groups::default();
        groups.open(1).unwrap();
        groups.next_tab = u64::MAX - 1;
        let before = groups.clone();
        assert!(
            groups
                .import(&[restored(&[2, 3], 0)], 0)
                .unwrap_err()
                .to_string()
                .contains("tab identities exhausted")
        );
        assert_eq!(groups, before);
        groups.next_tab = 2;
        groups.next_group = u64::MAX - 1;
        let before = groups.clone();
        assert!(
            groups
                .import(&[restored(&[2], 0), restored(&[3], 0)], 0)
                .unwrap_err()
                .to_string()
                .contains("group identities exhausted")
        );
        assert_eq!(groups, before);
    }

    #[test]
    fn forged_membership_cannot_focus_or_close_a_different_model_or_group() {
        let mut groups = Groups::default();
        let first = groups.open(1).unwrap().active.unwrap();
        let second = groups.split_active().unwrap().active.unwrap();
        let before = groups.clone();
        let wrong_document = Membership {
            document: 2,
            ..first
        };
        let wrong_group = Membership {
            group: second.group,
            ..first
        };
        for forged in [wrong_document, wrong_group] {
            assert!(!groups.membership_current(forged));
            assert!(groups.focus(forged).is_err());
            assert!(groups.close(forged).is_err());
            assert_eq!(groups, before);
        }
        assert!(
            groups
                .open(0)
                .unwrap_err()
                .to_string()
                .contains("document identity")
        );
        assert_eq!(groups, before);
        invariant(&groups);
    }

    fn is_preview(groups: &Groups, member: Membership) -> bool {
        let (group, tab) = groups.locate(member).unwrap();
        groups.groups[group].tabs[tab].is_preview()
    }

    #[test]
    fn preview_replacement_at_capacity_is_exact_and_atomic_when_ineligible() {
        let mut groups = Groups::default();
        let preview = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        for document in 2..=MAX_TABS_PER_GROUP as u64 {
            groups.open(document).unwrap();
        }
        let current = groups.active_membership().unwrap();
        let before = groups.clone();
        // Withholding authorization cannot promote/discard an in-flight model at capacity.
        assert!(groups.can_open_mode(129, OpenMode::Preview, None).is_err());
        assert!(groups.open_mode(129, OpenMode::Preview, None).is_err());
        assert_eq!(groups, before);
        assert!(is_preview(&groups, preview));
        let group_proof = groups.group_proof(preview.group).unwrap();
        let ui = groups.proof();
        groups
            .can_open_mode(129, OpenMode::Preview, Some(preview))
            .unwrap();
        assert_eq!(groups, before);
        let change = groups
            .open_mode(129, OpenMode::Preview, Some(preview))
            .unwrap();
        let replacement = change.active.unwrap();
        assert_eq!(replacement.group, preview.group);
        assert_ne!(replacement.tab, preview.tab);
        assert_eq!(change.removed, [preview]);
        assert_eq!(change.inserted, [replacement]);
        assert!(change.promoted.is_empty());
        assert_eq!(groups.groups()[0].tabs().len(), MAX_TABS_PER_GROUP);
        assert_eq!(documents(&groups, 0).last(), Some(&129));
        assert_eq!(
            groups.groups()[0].recent()[..2],
            [replacement.tab, current.tab]
        );
        assert!(!groups.membership_current(preview));
        assert!(!groups.group_proof_current(&group_proof));
        assert!(!groups.proof_current(&ui));
        invariant(&groups);
    }

    #[test]
    fn preview_replacement_uses_current_active_insertion_not_old_preview_index() {
        let mut groups = Groups::default();
        let b = groups
            .open_mode(2, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let c = groups.open(3).unwrap().active.unwrap();
        assert_eq!(documents(&groups, 0), [2, 3]);
        assert_eq!(groups.group(c.group).unwrap().preview(), Some(b));
        let d = groups
            .open_mode(4, OpenMode::Preview, Some(b))
            .unwrap()
            .active
            .unwrap();
        assert_eq!(documents(&groups, 0), [3, 4]);
        groups.open_mode(3, OpenMode::Preview, None).unwrap();
        assert!(!is_preview(&groups, c));
        let a = groups
            .open_mode(1, OpenMode::Preview, Some(d))
            .unwrap()
            .active
            .unwrap();
        assert_eq!(documents(&groups, 0), [3, 1]);
        assert_eq!(groups.group(c.group).unwrap().preview(), Some(a));
        // Replacement of an active first/only preview preserves its visible slot.
        groups.focus(a).unwrap();
        let replacement = groups
            .open_mode(5, OpenMode::Preview, Some(a))
            .unwrap()
            .active
            .unwrap();
        assert_eq!(documents(&groups, 0), [3, 5]);
        assert_eq!(groups.groups()[0].recent(), &[replacement.tab, c.tab]);
        invariant(&groups);
    }

    #[test]
    fn unsafe_preview_is_kept_before_new_preview_only_after_admission_succeeds() {
        let mut groups = Groups::default();
        let old = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let proof = groups.group_proof(old.group).unwrap();
        let change = groups.open_mode(2, OpenMode::Preview, None).unwrap();
        assert_eq!(change.promoted, [old]);
        assert!(change.removed.is_empty());
        assert_eq!(documents(&groups, 0), [1, 2]);
        assert!(!is_preview(&groups, old));
        assert!(groups.membership_current(old));
        assert!(!groups.group_proof_current(&proof));
        let preview = change.active.unwrap();
        assert_eq!(groups.group(old.group).unwrap().preview(), Some(preview));
        invariant(&groups);
    }

    #[test]
    fn keep_and_explicit_committed_open_preserve_identity_and_never_demote() {
        let mut groups = Groups::default();
        let a = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let before = groups.clone();
        assert!(
            !groups
                .open_mode(1, OpenMode::Preview, None)
                .unwrap()
                .changed
        );
        assert_eq!(groups, before);
        let ui = groups.proof();
        let proof = groups.group_proof(a.group).unwrap();
        let change = groups.keep(a).unwrap();
        assert_eq!(change.active, Some(a));
        assert_eq!(change.promoted, [a]);
        assert!(change.inserted.is_empty() && change.removed.is_empty());
        assert!(!groups.proof_current(&ui));
        assert!(!groups.group_proof_current(&proof));
        let committed = groups.clone();
        assert!(!groups.keep(a).unwrap().changed);
        assert!(
            !groups
                .open_mode(1, OpenMode::Preview, None)
                .unwrap()
                .changed
        );
        assert_eq!(groups, committed);
        let b = groups
            .open_mode(2, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let change = groups.open(2).unwrap();
        assert_eq!(change.promoted, [b]);
        assert_eq!(change.active, Some(b));
        assert!(groups.group(b.group).unwrap().preview().is_none());
        assert!(!groups.groups()[0].tabs().iter().any(Tab::is_sticky));
        invariant(&groups);
    }

    #[test]
    fn split_commits_destination_only_and_document_edit_undo_cannot_demote() {
        let mut document = crate::document::Document::from_text("猫🙂\r\n");
        document.path = Some(std::path::PathBuf::from("preview.cpp"));
        let mut groups = Groups::default();
        let source = groups
            .open_mode(document.id, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let destination = groups.split_active().unwrap().active.unwrap();
        assert!(is_preview(&groups, source));
        assert!(!is_preview(&groups, destination));
        assert_ne!(source.tab, destination.tab);
        assert_eq!(source.document, destination.document);
        let ui = groups.proof();
        let source_proof = groups.group_proof(source.group).unwrap();
        let destination_proof = groups.group_proof(destination.group).unwrap();
        let before_text = document.text.to_string();
        document.insert("x", false);
        assert!(document.dirty());
        let change = groups.promote_document(document.id).unwrap();
        assert_eq!(change.promoted, [source]);
        assert_eq!(change.active, Some(destination));
        assert!(!groups.proof_current(&ui));
        assert!(!groups.group_proof_current(&source_proof));
        assert!(groups.group_proof_current(&destination_proof));
        document.undo();
        assert_eq!(document.text.to_string(), before_text);
        assert!(!document.dirty());
        let committed = groups.clone();
        assert!(!groups.promote_document(document.id).unwrap().changed);
        groups
            .open_mode(document.id, OpenMode::Preview, None)
            .unwrap();
        assert_eq!(groups, committed);
        document.redo();
        assert_eq!(document.text.to_string(), format!("x{before_text}"));
        assert!(document.dirty());
        assert!(!is_preview(&groups, source));
        assert!(!is_preview(&groups, destination));
        invariant(&groups);
    }

    #[test]
    fn shared_previews_promote_together_without_focus_order_or_mru_changes() {
        let mut groups = Groups::default();
        let left = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let right = groups.split_active().unwrap().active.unwrap();
        groups.close(right).unwrap();
        let seed = groups.split_active().unwrap().active.unwrap();
        groups.open(2).unwrap();
        groups.close(seed).unwrap();
        let right_preview = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let left_proof = groups.group_proof(left.group).unwrap();
        let right_proof = groups.group_proof(right_preview.group).unwrap();
        let order: Vec<_> = groups
            .groups()
            .iter()
            .enumerate()
            .map(|(index, group)| {
                (
                    group.id(),
                    documents(&groups, index),
                    group.recent().to_vec(),
                )
            })
            .collect();
        let change = groups.promote_document(1).unwrap();
        assert_eq!(change.promoted, [left, right_preview]);
        assert_eq!(change.active, Some(right_preview));
        assert!(!groups.group_proof_current(&left_proof));
        assert!(!groups.group_proof_current(&right_proof));
        for (index, (id, documents_before, recent)) in order.iter().enumerate() {
            assert_eq!(groups.groups()[index].id(), *id);
            assert_eq!(documents(&groups, index), *documents_before);
            assert_eq!(groups.groups()[index].recent(), recent);
            assert!(groups.groups()[index].preview().is_none());
        }
        invariant(&groups);
    }

    #[test]
    fn stale_wrong_group_committed_or_existing_target_replacement_is_atomic() {
        let mut groups = Groups::default();
        let old = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let committed = groups.open(2).unwrap().active.unwrap();
        let fresh = groups
            .open_mode(3, OpenMode::Preview, Some(old))
            .unwrap()
            .active
            .unwrap();
        let other = groups.split_active().unwrap().active.unwrap();
        let before = groups.clone();
        for (group, document, mode, replacement) in [
            (fresh.group, 4, OpenMode::Preview, old),
            (fresh.group, 4, OpenMode::Preview, committed),
            (other.group, 4, OpenMode::Preview, fresh),
            (fresh.group, 4, OpenMode::Committed, fresh),
            (fresh.group, 2, OpenMode::Preview, fresh),
            (fresh.group, 3, OpenMode::Preview, fresh),
        ] {
            assert!(
                groups
                    .can_open_in_group_mode(group, document, mode, Some(replacement))
                    .is_err()
            );
            assert!(
                groups
                    .open_in_group_mode(group, document, mode, Some(replacement))
                    .is_err()
            );
            assert_eq!(groups, before);
        }
        groups.close(fresh).unwrap();
        let reopened = groups
            .open_in_group_mode(fresh.group, 3, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        assert_ne!(reopened.tab, fresh.tab);
        let reopened_state = groups.clone();
        assert!(
            groups
                .open_in_group_mode(fresh.group, 4, OpenMode::Preview, Some(fresh))
                .is_err()
        );
        assert_eq!(groups, reopened_state);
        invariant(&groups);
    }

    #[test]
    fn preview_counter_exhaustion_preflights_all_promotions_and_replacements() {
        for counter in 0..3 {
            let mut groups = Groups::default();
            let a = groups
                .open_mode(1, OpenMode::Preview, None)
                .unwrap()
                .active
                .unwrap();
            match counter {
                0 => groups.generation = u64::MAX,
                1 => groups.groups[0].membership_generation = u64::MAX,
                _ => groups.next_tab = u64::MAX,
            }
            let before = groups.clone();
            for replacement in [None, Some(a)] {
                assert!(
                    groups
                        .can_open_mode(2, OpenMode::Preview, replacement)
                        .is_err()
                );
                assert!(groups.open_mode(2, OpenMode::Preview, replacement).is_err());
                assert_eq!(groups, before);
            }
        }
        let mut groups = Groups::default();
        let left = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        groups.split_active().unwrap();
        groups.open(2).unwrap();
        let copy = member(&groups, 1, 1);
        groups.close(copy).unwrap();
        let right = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let index = groups
            .groups
            .iter()
            .position(|group| group.id == right.group)
            .unwrap();
        groups.groups[index].membership_generation = u64::MAX;
        let before = groups.clone();
        assert!(groups.promote_document(1).is_err());
        assert_eq!(groups, before);
        assert!(is_preview(&groups, left) && is_preview(&groups, right));
        assert!(groups.keep(right).is_err());
        assert_eq!(groups, before);
        invariant(&groups);
    }

    #[test]
    fn close_and_restore_clear_preview_pointer_without_resurrecting_modes() {
        let mut groups = Groups::default();
        let preview = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let kept = groups.open(2).unwrap().active.unwrap();
        groups.close(preview).unwrap();
        assert_eq!(groups.active_membership(), Some(kept));
        assert!(groups.group(kept.group).unwrap().preview().is_none());
        groups.open_mode(3, OpenMode::Preview, None).unwrap();
        groups.import(&[restored(&[1, 2, 3], 1)], 0).unwrap();
        assert!(groups.groups()[0].preview().is_none());
        assert!(
            groups.groups()[0]
                .tabs()
                .iter()
                .all(|tab| !tab.is_preview() && !tab.is_sticky())
        );
        invariant(&groups);
    }

    #[test]
    fn replacing_shared_preview_preserves_other_group_membership_and_close_ownership() {
        let mut groups = Groups::default();
        let preview = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let committed = groups.split_active().unwrap().active.unwrap();
        let destination_proof = groups.group_proof(committed.group).unwrap();
        groups.focus(preview).unwrap();
        let replacement = groups
            .open_mode(2, OpenMode::Preview, Some(preview))
            .unwrap()
            .active
            .unwrap();
        assert_eq!(groups.memberships(1).collect::<Vec<_>>(), [committed]);
        assert!(groups.membership_current(committed));
        assert!(groups.group_proof_current(&destination_proof));
        let before = groups.clone();
        assert!(groups.close(preview).is_err());
        assert!(groups.keep(preview).is_err());
        assert_eq!(groups, before);
        groups.close(committed).unwrap();
        assert_eq!(groups.active_membership(), Some(replacement));
        assert_eq!(groups.groups().len(), 1);
        assert_eq!(
            groups.group(replacement.group).unwrap().preview(),
            Some(replacement)
        );
        invariant(&groups);
    }

    fn sticky(groups: &Groups, member: Membership) -> bool {
        let (group, tab) = groups.locate(member).unwrap();
        groups.groups[group].tabs[tab].is_sticky()
    }

    #[test]
    fn pin_preview_commits_exact_membership_without_focusing_or_changing_mru() {
        let mut groups = Groups::default();
        let preview = groups
            .open_mode(1, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let active = groups.open(2).unwrap().active.unwrap();
        let ui = groups.proof();
        let proof = groups.group_proof(preview.group).unwrap();
        let group_recent = groups.group(preview.group).unwrap().recent().to_vec();
        let global_recent = groups.membership_mru().to_vec();
        let group_mru = groups.group_mru().to_vec();
        let next_tab = groups.next_tab;
        let change = groups.set_sticky(preview, true).unwrap();
        assert_eq!(change.promoted, [preview]);
        assert_eq!(change.sticky_changed, [preview]);
        assert_eq!(change.active, Some(active));
        assert!(change.inserted.is_empty() && change.removed.is_empty());
        assert_eq!(groups.active_membership(), Some(active));
        assert_eq!(groups.next_tab, next_tab);
        assert!(sticky(&groups, preview));
        assert!(!is_preview(&groups, preview));
        assert_eq!(groups.group(preview.group).unwrap().preview(), None);
        assert_eq!(groups.group(preview.group).unwrap().recent(), group_recent);
        assert_eq!(groups.membership_mru(), global_recent);
        assert_eq!(groups.group_mru(), group_mru);
        assert!(!groups.proof_current(&ui));
        assert!(!groups.group_proof_current(&proof));
        let before = groups.clone();
        assert!(!groups.keep(preview).unwrap().changed);
        assert!(!groups.set_sticky(preview, true).unwrap().changed);
        assert_eq!(groups, before);
        invariant(&groups);
    }

    #[test]
    fn first_middle_and_last_sticky_unpin_move_to_first_nonsticky_slot() {
        for (target, expected) in [(2, [4, 1, 2, 3]), (4, [2, 1, 4, 3]), (1, [2, 4, 1, 3])] {
            let mut groups = Groups::default();
            groups
                .import(&[restored(&[1, 2, 3, 4], 2), restored(&[9], 0)], 0)
                .unwrap();
            let unaffected = groups.group_proof(groups.groups()[1].id()).unwrap();
            for document in [2, 4, 1] {
                groups
                    .set_sticky(member(&groups, 0, document), true)
                    .unwrap();
            }
            assert_eq!(documents(&groups, 0), [2, 4, 1, 3]);
            assert_eq!(groups.groups()[0].sticky_count(), 3);
            let active = groups.active_membership();
            let recent = groups.membership_mru().to_vec();
            let local = groups.groups()[0].recent().to_vec();
            let tabs: Vec<_> = groups.groups()[0].tabs().iter().map(Tab::id).collect();
            let target = member(&groups, 0, target);
            let change = groups.set_sticky(target, false).unwrap();
            assert_eq!(change.sticky_changed, [target]);
            assert!(change.promoted.is_empty());
            assert_eq!(documents(&groups, 0), expected);
            assert_eq!(groups.groups()[0].sticky_count(), 2);
            assert_eq!(groups.active_membership(), active);
            assert_eq!(groups.membership_mru(), recent);
            assert_eq!(groups.groups()[0].recent(), local);
            assert_eq!(
                groups.groups()[0]
                    .tabs()
                    .iter()
                    .map(Tab::id)
                    .collect::<HashSet<_>>(),
                tabs.into_iter().collect()
            );
            assert!(groups.group_proof_current(&unaffected));
            assert!(!sticky(&groups, target) && !is_preview(&groups, target));
            invariant(&groups);
        }
    }

    #[test]
    fn right_of_active_insert_and_preview_replace_never_split_sticky_prefix() {
        let mut groups = Groups::default();
        groups.import(&[restored(&[1, 2, 3, 4], 3)], 0).unwrap();
        let first = member(&groups, 0, 1);
        groups.set_sticky(first, true).unwrap();
        groups.set_sticky(member(&groups, 0, 2), true).unwrap();
        let preview = groups
            .open_mode(5, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        groups.focus(first).unwrap();
        let fresh = groups
            .open_mode(6, OpenMode::Preview, Some(preview))
            .unwrap();
        assert_eq!(fresh.removed, [preview]);
        assert_eq!(documents(&groups, 0), [1, 2, 6, 3, 4]);
        assert_ne!(fresh.active.unwrap().tab, preview.tab);
        assert_eq!(groups.groups()[0].sticky_count(), 2);
        assert!(groups.membership_current(first));
        groups.focus(first).unwrap();
        groups.open(7).unwrap();
        assert_eq!(documents(&groups, 0), [1, 2, 7, 6, 3, 4]);
        invariant(&groups);
    }

    #[test]
    fn split_sticky_source_is_shared_committed_nonsticky_at_destination() {
        let mut groups = Groups::default();
        let source = groups.open(1).unwrap().active.unwrap();
        groups.set_sticky(source, true).unwrap();
        let destination = groups.split_active().unwrap().active.unwrap();
        assert_eq!(destination.document, source.document);
        assert_ne!(destination.tab, source.tab);
        assert!(sticky(&groups, source));
        assert!(!sticky(&groups, destination) && !is_preview(&groups, destination));
        assert_eq!(groups.groups()[1].sticky_count(), 0);
        groups.close(destination).unwrap();
        assert_eq!(groups.memberships(1).collect::<Vec<_>>(), [source]);
        assert!(groups.membership_current(source));
        invariant(&groups);
    }

    #[test]
    fn global_editor_mru_cannot_be_reconstructed_from_group_mru() {
        let mut groups = Groups::default();
        groups
            .import(&[restored(&[1, 2], 0), restored(&[3, 4], 0)], 0)
            .unwrap();
        let left_sticky = member(&groups, 0, 1);
        let left_plain = member(&groups, 0, 2);
        let right_sticky = member(&groups, 1, 3);
        let right_plain = member(&groups, 1, 4);
        groups.set_sticky(left_sticky, true).unwrap();
        groups.set_sticky(right_sticky, true).unwrap();
        groups.focus(left_plain).unwrap();
        groups.focus(right_plain).unwrap();
        groups.focus(left_sticky).unwrap();
        assert_eq!(groups.group_mru()[0], left_sticky.group);
        assert_eq!(
            groups.next_nonsticky_recent(left_sticky.group),
            Some(left_plain)
        );
        // Concatenating per-group MRUs in group-MRU order incorrectly yields
        // left_plain. Actual interleaved editor activation last visited right_plain.
        assert_eq!(groups.next_nonsticky_recent_any_group(), Some(right_plain));
        assert_eq!(
            groups.membership_mru()[..3],
            [left_sticky, right_plain, left_plain]
        );
        let before = groups.clone();
        assert!(!groups.focus(left_sticky).unwrap().changed);
        assert!(
            !groups
                .open_in_group_mode(left_sticky.group, 1, OpenMode::Preview, None)
                .unwrap()
                .changed
        );
        assert_eq!(groups, before);
        groups.set_sticky(left_plain, true).unwrap();
        assert_eq!(groups.next_nonsticky_recent(left_sticky.group), None);
        assert_eq!(groups.next_nonsticky_recent_any_group(), Some(right_plain));
        groups.set_sticky(right_plain, true).unwrap();
        assert_eq!(groups.next_nonsticky_recent_any_group(), None);
        invariant(&groups);
    }

    #[test]
    fn exact_nonsticky_subset_preserves_sticky_shared_memberships_and_mru_fallback() {
        let mut groups = Groups::default();
        groups
            .import(&[restored(&[1, 2, 3, 4], 3), restored(&[3], 0)], 0)
            .unwrap();
        let protected = member(&groups, 0, 1);
        groups.set_sticky(protected, true).unwrap();
        let two = member(&groups, 0, 2);
        let three = member(&groups, 0, 3);
        let four = member(&groups, 0, 4);
        let shared = member(&groups, 1, 3);
        let unaffected = groups.group_proof(shared.group).unwrap();
        groups.focus(two).unwrap();
        groups.focus(four).unwrap();
        let proof = groups.group_proof(protected.group).unwrap();
        let change = groups.close_memberships(&proof, &[four, three]).unwrap();
        assert_eq!(change.removed, [three, four]);
        assert_eq!(documents(&groups, 0), [1, 2]);
        assert_eq!(change.active, Some(two));
        assert_eq!(groups.memberships(3).collect::<Vec<_>>(), [shared]);
        assert!(groups.group_proof_current(&unaffected));
        assert!(sticky(&groups, protected));
        assert!(!groups.group_proof_current(&proof));
        invariant(&groups);
    }

    #[test]
    fn subset_close_clears_preview_and_all_plain_group_removal_preserves_other_groups() {
        let mut groups = Groups::default();
        let kept = groups.open(1).unwrap().active.unwrap();
        groups.set_sticky(kept, true).unwrap();
        let preview = groups
            .open_mode(2, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        let shared = groups.split_active().unwrap().active.unwrap();
        let proof = groups.group_proof(preview.group).unwrap();
        groups.close_memberships(&proof, &[preview]).unwrap();
        assert_eq!(groups.group(kept.group).unwrap().preview(), None);
        assert_eq!(groups.memberships(2).collect::<Vec<_>>(), [shared]);
        let proof = groups.group_proof(shared.group).unwrap();
        let removed = groups.close_memberships(&proof, &[shared]).unwrap();
        assert_eq!(removed.removed_groups, [shared.group]);
        assert_eq!(groups.active_membership(), Some(kept));
        invariant(&groups);
    }

    #[test]
    fn subset_close_rejects_foreign_duplicate_sticky_wrong_document_and_stale_aba_atomically() {
        let mut groups = Groups::default();
        groups
            .import(&[restored(&[1, 2], 0), restored(&[3], 0)], 0)
            .unwrap();
        let protected = member(&groups, 0, 1);
        let plain = member(&groups, 0, 2);
        let foreign = member(&groups, 1, 3);
        groups.set_sticky(protected, true).unwrap();
        let proof = groups.group_proof(plain.group).unwrap();
        for targets in [
            vec![protected],
            vec![plain, plain],
            vec![foreign],
            vec![Membership {
                document: 99,
                ..plain
            }],
            vec![plain; MAX_TABS_PER_GROUP + 1],
        ] {
            let before = groups.clone();
            assert!(groups.close_memberships(&proof, &targets).is_err());
            assert_eq!(groups, before);
        }
        groups.set_sticky(plain, true).unwrap();
        groups.set_sticky(plain, false).unwrap();
        assert_eq!(
            proof.members(),
            groups.group_proof(plain.group).unwrap().members()
        );
        let before = groups.clone();
        assert!(groups.close_memberships(&proof, &[plain]).is_err());
        assert_eq!(groups, before);
        let proof = groups.group_proof(plain.group).unwrap();
        groups.close(plain).unwrap();
        let replacement = groups
            .open_in_group(protected.group, 2)
            .unwrap()
            .active
            .unwrap();
        assert_ne!(replacement.tab, plain.tab);
        let before = groups.clone();
        assert!(groups.set_sticky(plain, true).is_err());
        assert!(groups.close_memberships(&proof, &[replacement]).is_err());
        assert_eq!(groups, before);
        invariant(&groups);
    }

    #[test]
    fn sticky_and_subset_counter_exhaustion_fail_before_identity_mode_or_order_changes() {
        for exhausted in 0..2 {
            let mut groups = Groups::default();
            groups.import(&[restored(&[1, 2, 3], 0)], 0).unwrap();
            let one = member(&groups, 0, 1);
            groups.set_sticky(one, true).unwrap();
            let two = member(&groups, 0, 2);
            if exhausted == 0 {
                groups.generation = u64::MAX;
            } else {
                groups.groups[0].membership_generation = u64::MAX;
            }
            let proof = groups.group_proof(one.group).unwrap();
            let before = groups.clone();
            assert!(groups.set_sticky(two, true).is_err());
            assert!(groups.set_sticky(one, false).is_err());
            assert!(groups.close_memberships(&proof, &[two]).is_err());
            assert_eq!(groups, before);
            assert!(!groups.set_sticky(one, true).unwrap().changed);
            assert!(!groups.close_memberships(&proof, &[]).unwrap().changed);
            assert_eq!(groups, before);
            invariant(&groups);
        }
    }

    #[test]
    fn individual_sticky_close_maintains_prefix_and_prunes_global_mru() {
        for document in [1, 2, 3] {
            let mut groups = Groups::default();
            groups.import(&[restored(&[1, 2, 3, 4], 3)], 0).unwrap();
            for value in [1, 2, 3] {
                groups.set_sticky(member(&groups, 0, value), true).unwrap();
            }
            let removed = member(&groups, 0, document);
            let change = groups.close(removed).unwrap();
            assert_eq!(change.removed, [removed]);
            assert_eq!(groups.groups()[0].sticky_count(), 2);
            assert!(!groups.membership_mru().contains(&removed));
            assert_eq!(
                documents(&groups, 0),
                (1..=4)
                    .filter(|value| *value != document)
                    .collect::<Vec<_>>()
            );
            invariant(&groups);
        }
    }

    #[test]
    fn full_group_preview_replacement_preserves_prefix_and_live_mru_bounds() {
        let mut groups = Groups::default();
        let first = groups.open(1).unwrap().active.unwrap();
        for document in 2..MAX_TABS_PER_GROUP as u64 {
            groups.open(document).unwrap();
        }
        let old = groups
            .open_mode(MAX_TABS_PER_GROUP as u64, OpenMode::Preview, None)
            .unwrap()
            .active
            .unwrap();
        groups.set_sticky(first, true).unwrap();
        groups.focus(first).unwrap();
        let inserted = groups
            .open_mode(1000, OpenMode::Preview, Some(old))
            .unwrap()
            .active
            .unwrap();
        assert_eq!(groups.groups()[0].tabs().len(), MAX_TABS_PER_GROUP);
        assert_eq!(groups.membership_mru().len(), MAX_TABS_PER_GROUP);
        assert_eq!(groups.groups()[0].tabs()[0].document(), 1);
        assert_eq!(groups.groups()[0].tabs()[1].document(), 1000);
        assert_ne!(inserted.tab, old.tab);
        assert!(!groups.membership_current(old));
        invariant(&groups);
    }

    #[test]
    fn restore_retires_sticky_modes_old_memberships_and_activation_inventory() {
        let mut groups = Groups::default();
        let old = groups.open(1).unwrap().active.unwrap();
        groups.set_sticky(old, true).unwrap();
        groups.split_active().unwrap();
        groups.import(&[restored(&[1, 2], 1)], 0).unwrap();
        assert!(!groups.membership_current(old));
        assert_eq!(groups.groups()[0].sticky_count(), 0);
        assert!(
            groups.groups()[0]
                .tabs()
                .iter()
                .all(|tab| !tab.is_sticky() && !tab.is_preview())
        );
        assert_eq!(
            groups
                .membership_mru()
                .iter()
                .map(|member| member.document)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        invariant(&groups);
    }

    #[test]
    fn captured_subset_survives_focus_but_cannot_redirect_to_another_group() {
        let mut groups = Groups::default();
        groups
            .import(&[restored(&[1, 2], 0), restored(&[3, 4], 0)], 0)
            .unwrap();
        let protected = member(&groups, 0, 1);
        groups.set_sticky(protected, true).unwrap();
        let target = member(&groups, 0, 2);
        let proof = groups.group_proof(target.group).unwrap();
        let ui = groups.proof();
        let other = member(&groups, 1, 4);
        groups.focus(other).unwrap();
        assert!(!groups.proof_current(&ui));
        assert!(groups.group_proof_current(&proof));
        let recent = groups.membership_mru().to_vec();
        let change = groups.close_memberships(&proof, &[target]).unwrap();
        assert_eq!(change.removed, [target]);
        assert_eq!(change.active, Some(other));
        assert_eq!(documents(&groups, 1), [3, 4]);
        assert_eq!(
            groups.membership_mru(),
            recent
                .into_iter()
                .filter(|member| *member != target)
                .collect::<Vec<_>>()
        );
        invariant(&groups);
    }

    #[test]
    fn global_mru_is_bounded_for_512_distinct_live_memberships_in_four_groups() {
        let mut groups = Groups::default();
        let restored: Vec<_> = (0..MAX_GROUPS)
            .map(|index| {
                let docs: Vec<_> = (1..=MAX_TABS_PER_GROUP as u64)
                    .map(|doc| doc + (index * MAX_TABS_PER_GROUP) as u64)
                    .collect();
                restored(&docs, 0)
            })
            .collect();
        groups.import(&restored, 0).unwrap();
        assert_eq!(groups.membership_mru().len(), MAX_MEMBERSHIPS);
        for index in [3, 0, 2, 1, 3] {
            let target = member(&groups, index, ((index + 1) * MAX_TABS_PER_GROUP) as u64);
            groups.focus(target).unwrap();
            groups.set_sticky(target, true).unwrap();
            assert_eq!(groups.membership_mru()[0], target);
            assert_eq!(groups.group_mru()[0], target.group);
            invariant(&groups);
        }
        let before = groups.clone();
        assert!(groups.open(10000).is_err());
        assert_eq!(groups, before);
        let target = groups.active_membership().unwrap();
        groups.close(target).unwrap();
        groups.open(10000).unwrap();
        assert_eq!(groups.membership_mru().len(), MAX_MEMBERSHIPS);
        assert_eq!(groups.membership_mru()[0].document, 10000);
        invariant(&groups);
    }
}
