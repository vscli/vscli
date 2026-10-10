//! Bounded editor-group membership. Documents and dirty-close decisions belong to App.
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;

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
}
impl Tab {
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
}
impl Group {
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Groups {
    groups: Vec<Group>,
    active: Option<GroupId>,
    generation: u64,
    next_group: u64,
    next_tab: u64,
}
impl Default for Groups {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            active: None,
            generation: 0,
            next_group: 1,
            next_tab: 1,
        }
    }
}
fn increment(value: u64, what: &str) -> Result<u64> {
    value
        .checked_add(1)
        .with_context(|| format!("Editor-group {what} exhausted"))
}
impl Groups {
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
    pub fn can_open(&self, document: u64) -> Result<()> {
        ensure!(document != 0, "Invalid document identity");
        if let Some(group) = self.active {
            return self.can_open_in_group(group, document);
        }
        increment(self.generation, "interaction generation")?;
        increment(self.next_group, "group identities")?;
        increment(self.next_tab, "tab identities")?;
        Ok(())
    }
    pub fn can_open_in_group(&self, id: GroupId, document: u64) -> Result<()> {
        ensure!(document != 0, "Invalid document identity");
        let group = self.group(id).context("Editor group was closed")?;
        if let Some(tab) = group.tabs.iter().find(|tab| tab.document == document) {
            if self.active != Some(id) || group.active != tab.id {
                increment(self.generation, "interaction generation")?;
            }
            return Ok(());
        }
        ensure!(
            group.tabs.len() < MAX_TABS_PER_GROUP,
            "Editor group exceeds 128 tabs"
        );
        increment(self.generation, "interaction generation")?;
        increment(self.next_tab, "tab identities")?;
        increment(group.membership_generation, "membership generation")?;
        Ok(())
    }
    pub fn open(&mut self, document: u64) -> Result<Change> {
        if let Some(group) = self.active {
            return self.open_in_group(group, document);
        }
        self.can_open(document)?;
        let group = GroupId(self.next_group);
        let tab = TabId(self.next_tab);
        let member = Membership {
            group,
            tab,
            document,
        };
        self.groups.push(Group {
            id: group,
            tabs: vec![Tab { id: tab, document }],
            active: tab,
            recent: vec![tab],
            membership_generation: 1,
        });
        self.active = Some(group);
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
    pub fn open_in_group(&mut self, id: GroupId, document: u64) -> Result<Change> {
        self.can_open_in_group(id, document)?;
        let index = self.groups.iter().position(|group| group.id == id).unwrap();
        if let Some(tab) = self.groups[index]
            .tabs
            .iter()
            .find(|tab| tab.document == document)
        {
            return self.focus(self.groups[index].membership(tab));
        }
        let previous = self.active_membership();
        let tab = TabId(self.next_tab);
        let group = &mut self.groups[index];
        let position = group
            .tabs
            .iter()
            .position(|tab| tab.id == group.active)
            .unwrap()
            + 1;
        group.tabs.insert(position, Tab { id: tab, document });
        group.active = tab;
        group.recent.insert(0, tab);
        group.membership_generation += 1;
        self.next_tab += 1;
        self.generation += 1;
        self.active = Some(id);
        let member = Membership {
            group: id,
            tab,
            document,
        };
        Ok(Change {
            changed: true,
            previous,
            active: Some(member),
            inserted: vec![member],
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
        group.recent.retain(|id| *id != member.tab);
        group.recent.insert(0, member.tab);
        self.active = Some(member.group);
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
                }],
                active: tab,
                recent: vec![tab],
                membership_generation: 1,
            },
        );
        self.active = Some(group);
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
            group.tabs.remove(tab);
            group.recent.retain(|id| *id != member.tab);
            if group.active == member.tab {
                group.active = group.recent[0];
            }
            group.membership_generation = membership_generation.unwrap();
        }
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
            });
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(group.recent.first(), Some(&group.active));
            assert_eq!(group.recent.len(), group.tabs.len());
            let recent: HashSet<_> = group.recent.iter().copied().collect();
            assert_eq!(recent.len(), group.tabs.len());
            assert!(group.tabs.iter().all(|tab| recent.contains(&tab.id)));
            assert!(group.active().is_some());
            total += group.tabs.len();
        }
        assert!(total <= MAX_MEMBERSHIPS);
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
}
