//! Complete bounded native tab inventory, separate from visible editor handles.
use super::document_uri;
use crate::{
    document::Document,
    editor_groups::{
        GroupId, GroupProof, Groups, MAX_GROUPS, MAX_MEMBERSHIPS, MAX_TABS_PER_GROUP, Membership,
    },
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

const MAX_WIRE: usize = 2 * 1024 * 1024;
#[derive(Clone)]
struct TabFrame {
    id: String,
    member: Membership,
    lease: Arc<()>,
    uri: String,
    label: String,
    active: bool,
    dirty: bool,
    pinned: bool,
    preview: bool,
}
#[derive(Clone)]
struct GroupFrame {
    id: String,
    group: GroupId,
    proof: GroupProof,
    lease: Arc<()>,
    column: usize,
    active: bool,
    tabs: Vec<TabFrame>,
}
#[derive(Clone, Default)]
pub(crate) struct Inventory {
    next: u64,
    engine: Option<Arc<()>>,
    groups: Vec<GroupFrame>,
    invalid: bool,
}

#[derive(Clone)]
pub(crate) struct Capture {
    kind: String,
    ids: Vec<String>,
    engine: Arc<()>,
    members: Vec<Membership>,
    group_ids: Vec<GroupId>,
    group_proofs: Vec<GroupProof>,
    group_leases: Vec<(String, Arc<()>)>,
    tab_leases: Vec<(String, Arc<()>)>,
}
impl Capture {
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }
    pub(crate) fn ids(&self) -> &[String] {
        &self.ids
    }
    pub(crate) fn memberships(&self) -> &[Membership] {
        &self.members
    }
    pub(crate) fn group_ids(&self) -> &[GroupId] {
        &self.group_ids
    }
    pub(crate) fn groups(&self) -> &[GroupProof] {
        &self.group_proofs
    }
    /// An admitted multi-step close validates remaining original memberships
    /// itself. Its own earlier closes or SaveAs do not acquire a new identity.
    pub(crate) fn engine_current(&self, groups: &Groups) -> bool {
        Arc::ptr_eq(&self.engine, &groups.lifetime())
    }
    pub(crate) fn current(&self, inventory: &Inventory, groups: &Groups) -> bool {
        self.engine_current(groups)
            && !inventory.invalid
            && inventory
                .engine
                .as_ref()
                .is_some_and(|engine| Arc::ptr_eq(engine, &self.engine))
            && self
                .members
                .iter()
                .all(|member| groups.membership_current(*member))
            && self
                .group_proofs
                .iter()
                .all(|proof| groups.group_proof_current(proof))
            && self.group_leases.iter().all(|(id, lease)| {
                inventory
                    .groups
                    .iter()
                    .any(|group| group.id == *id && Arc::ptr_eq(lease, &group.lease))
            })
            && self.tab_leases.iter().all(|(id, lease)| {
                inventory
                    .groups
                    .iter()
                    .flat_map(|group| &group.tabs)
                    .any(|tab| tab.id == *id && Arc::ptr_eq(lease, &tab.lease))
            })
    }
}
impl Inventory {
    pub(crate) fn invalidate(&mut self) {
        self.groups.clear();
        self.engine = None;
        self.invalid = true;
    }
    /// Called at every engine transition, including transitions coalesced before
    /// a host mirror. The engine lifetime prevents numeric ID ABA after reset.
    pub(crate) fn observe(&mut self, groups: &Groups, documents: &[&Document]) -> Result<()> {
        ensure!(
            !groups.groups().is_empty() && groups.groups().len() <= MAX_GROUPS,
            "Native tab groups require 1–4 actual groups"
        );
        ensure!(
            documents.len() <= 128,
            "Native tab inventory model limit exceeded"
        );
        let engine = groups.lifetime();
        let reuse = !self.invalid
            && self
                .engine
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, &engine));
        let mut staged = Vec::with_capacity(groups.groups().len());
        let mut next = self.next;
        let mut members = 0usize;
        let mut wire_bytes = 32usize;
        let mut seen_groups = HashSet::new();
        let mut seen_members = HashSet::new();
        for (index, group) in groups.groups().iter().enumerate() {
            ensure!(seen_groups.insert(group.id()), "Duplicate native tab group");
            ensure!(
                group.tabs().len() <= MAX_TABS_PER_GROUP,
                "Native tab group exceeds 128 tabs"
            );
            members = members
                .checked_add(group.tabs().len())
                .context("Tab inventory count overflow")?;
            ensure!(
                members <= MAX_MEMBERSHIPS,
                "Native tabs exceed 512 memberships"
            );
            let old = reuse
                .then(|| self.groups.iter().find(|old| old.group == group.id()))
                .flatten();
            let (id, lease) = if let Some(old) = old {
                (old.id.clone(), old.lease.clone())
            } else {
                (allocate(&mut next, "group")?, Arc::new(()))
            };
            let mut tabs = Vec::with_capacity(group.tabs().len());
            wire_bytes += 128 + id.len();
            for tab in group.tabs() {
                let member = Membership {
                    group: group.id(),
                    tab: tab.id(),
                    document: tab.document(),
                };
                ensure!(
                    seen_members.insert((member.group, member.tab)),
                    "Duplicate native tab identity"
                );
                let doc = documents
                    .iter()
                    .find(|doc| doc.id == tab.document())
                    .context("Native tab document is absent")?;
                ensure!(
                    doc.path
                        .as_ref()
                        .is_none_or(|path| path.as_os_str().as_encoded_bytes().len() <= 4096),
                    "Tab resource path exceeds bounds"
                );
                let uri = document_uri(doc)?;
                let label = doc.name();
                ensure!(
                    uri.len() <= 4096
                        && label.len() <= 1024
                        && !uri.contains('\0')
                        && !label.contains('\0'),
                    "Tab metadata exceeds bounds"
                );
                let previous = old.and_then(|group| {
                    group
                        .tabs
                        .iter()
                        .find(|old| old.member == member && old.uri == uri)
                });
                let (id, lease) = if let Some(old) = previous {
                    (old.id.clone(), old.lease.clone())
                } else {
                    (allocate(&mut next, "tab")?, Arc::new(()))
                };
                wire_bytes = wire_bytes
                    .checked_add(192 + id.len() + string_bytes(&uri) + string_bytes(&label))
                    .context("Tab mirror byte overflow")?;
                ensure!(wire_bytes <= MAX_WIRE, "Native tab mirror exceeds 2 MiB");
                tabs.push(TabFrame {
                    id,
                    member,
                    lease,
                    uri,
                    label,
                    active: group.active().is_some_and(|active| active.id() == tab.id()),
                    dirty: doc.dirty(),
                    pinned: tab.is_sticky(),
                    preview: tab.is_preview(),
                });
            }
            staged.push(GroupFrame {
                id,
                group: group.id(),
                proof: groups.group_proof(group.id())?,
                lease,
                column: index + 1,
                active: groups.active_group() == Some(group.id()),
                tabs,
            });
        }
        ensure!(
            staged.iter().filter(|group| group.active).count() == 1,
            "Native tab inventory needs one active group"
        );
        self.next = next;
        self.engine = Some(engine);
        self.groups = staged;
        self.invalid = false;
        Ok(())
    }
    pub(crate) fn validate(&self, groups: &Groups, documents: &[&Document]) -> Result<()> {
        ensure!(
            documents.len() <= 128,
            "Native tab inventory model limit exceeded"
        );
        ensure!(
            !self.invalid
                && self
                    .engine
                    .as_ref()
                    .is_some_and(|engine| Arc::ptr_eq(engine, &groups.lifetime())),
            "Native tab engine retired"
        );
        ensure!(
            self.groups.len() == groups.groups().len(),
            "Native tab groups changed"
        );
        for (frame, group) in self.groups.iter().zip(groups.groups()) {
            ensure!(
                frame.group == group.id()
                    && frame.active == (groups.active_group() == Some(group.id()))
                    && frame.tabs.len() == group.tabs().len(),
                "Native tab group changed"
            );
            for (frame, tab) in frame.tabs.iter().zip(group.tabs()) {
                let doc = documents
                    .iter()
                    .find(|doc| doc.id == tab.document())
                    .context("Native tab model retired")?;
                ensure!(
                    frame.member
                        == Membership {
                            group: group.id(),
                            tab: tab.id(),
                            document: tab.document()
                        }
                        && frame.uri == document_uri(doc)?
                        && frame.label == doc.name()
                        && frame.dirty == doc.dirty()
                        && frame.pinned == tab.is_sticky()
                        && frame.preview == tab.is_preview()
                        && frame.active
                            == group.active().is_some_and(|active| active.id() == tab.id()),
                    "Native tab metadata changed"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn wire(&self) -> Result<Value> {
        ensure!(
            !self.invalid && self.engine.is_some() && !self.groups.is_empty(),
            "Native tab inventory unavailable"
        );
        Ok(
            json!({"groups":self.groups.iter().map(|group|json!({"id":group.id,"viewColumn":group.column,"isActive":group.active,"tabs":group.tabs.iter().map(|tab|json!({"id":tab.id,"label":tab.label,"isActive":tab.active,"isDirty":tab.dirty,"isPinned":tab.pinned,"isPreview":tab.preview,"input":{"kind":"text","uri":tab.uri}})).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
        )
    }
    /// Capture only the last successfully observed inventory. Live App state
    /// must validate this original cohort before beginning any close effects.
    pub(crate) fn capture_published(&self, kind: &str, ids: &[String]) -> Result<Capture> {
        ensure!(
            matches!(kind, "tabs" | "groups") && ids.len() <= MAX_MEMBERSHIPS,
            "Invalid tab close target kind or count"
        );
        ensure!(
            ids.iter().all(|id| id.len() <= 128 && id.is_ascii()),
            "Invalid native tab identifier"
        );
        let engine = self
            .engine
            .as_ref()
            .context("Native tab inventory unavailable")?;
        ensure!(!self.invalid, "Native tab inventory retired");
        let mut capture = Capture {
            kind: kind.into(),
            ids: ids.to_vec(),
            engine: engine.clone(),
            members: Vec::new(),
            group_ids: Vec::new(),
            group_proofs: Vec::new(),
            group_leases: Vec::new(),
            tab_leases: Vec::new(),
        };
        let mut seen = HashSet::new();
        for id in ids {
            ensure!(
                id.len() <= 128 && id.is_ascii(),
                "Invalid native tab identifier"
            );
            if !seen.insert(id) {
                continue;
            }
            if kind == "tabs" {
                let tab = self
                    .groups
                    .iter()
                    .flat_map(|group| &group.tabs)
                    .find(|tab| tab.id == *id)
                    .context("Native tab retired")?;
                capture.members.push(tab.member);
                capture.tab_leases.push((id.clone(), tab.lease.clone()));
            } else {
                let group = self
                    .groups
                    .iter()
                    .find(|group| group.id == *id)
                    .context("Native tab group retired")?;
                let proof = group.proof.clone();
                capture.members.extend_from_slice(proof.members());
                capture.group_ids.push(group.group);
                capture.group_proofs.push(proof);
                capture.group_leases.push((id.clone(), group.lease.clone()));
            }
        }
        Ok(capture)
    }
}
fn allocate(next: &mut u64, kind: &str) -> Result<String> {
    *next = next
        .checked_add(1)
        .context("Native tab identifiers exhausted")?;
    Ok(format!("tabs-{kind}-{next}"))
}
fn string_bytes(value: &str) -> usize {
    2 + value
        .chars()
        .map(|ch| match ch {
            '"' | '\\' => 2,
            '\u{0}'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        })
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(inventory: &Inventory) -> Vec<String> {
        inventory
            .groups
            .iter()
            .flat_map(|group| group.tabs.iter().map(|tab| tab.id.clone()))
            .collect()
    }
    #[test]
    fn actual_welcome_and_final_close_preserve_group_handle_without_fake_tab() {
        let mut groups = Groups::with_empty_group();
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[]).unwrap();
        let id = inventory.groups[0].id.clone();
        let wire = inventory.wire().unwrap();
        assert_eq!(wire["groups"][0]["tabs"], json!([]));
        assert_eq!(wire["groups"][0]["isActive"], true);
        let capture = inventory
            .capture_published("groups", std::slice::from_ref(&id))
            .unwrap();
        assert!(capture.current(&inventory, &groups));
        assert!(capture.memberships().is_empty());
        assert_eq!(capture.group_ids(), &[groups.active_group().unwrap()]);
        let doc = Document::from_text("猫🙂\r\n");
        let member = groups.open(doc.id).unwrap().active.unwrap();
        inventory.observe(&groups, &[&doc]).unwrap();
        assert_eq!(inventory.groups[0].id, id);
        assert!(!capture.current(&inventory, &groups));
        groups.close(member).unwrap();
        inventory.observe(&groups, &[]).unwrap();
        assert_eq!(inventory.groups[0].id, id);
        assert!(inventory.groups[0].tabs.is_empty());
    }
    #[test]
    fn inactive_memberships_shared_dirty_preview_and_sticky_have_separate_handles() {
        let mut a = Document::from_text("猫🙂\r\n");
        let b = Document::from_text("second\r\n");
        let mut groups = Groups::with_empty_group();
        let first = groups.open(a.id).unwrap().active.unwrap();
        groups
            .open_mode(b.id, crate::editor_groups::OpenMode::Preview, None)
            .unwrap();
        groups.focus(first).unwrap();
        let second = groups.split_active().unwrap().active.unwrap();
        groups.set_sticky(first, true).unwrap();
        a.insert("!", false);
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[&a, &b]).unwrap();
        let wire = inventory.wire().unwrap();
        assert_eq!(wire["groups"][0]["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(wire["groups"][0]["tabs"][0]["isPinned"], true);
        assert_eq!(wire["groups"][0]["tabs"][1]["isPreview"], true);
        assert_eq!(wire["groups"][0]["tabs"][0]["isDirty"], true);
        assert_eq!(wire["groups"][1]["tabs"][0]["isDirty"], true);
        assert_ne!(
            wire["groups"][0]["tabs"][0]["id"],
            wire["groups"][1]["tabs"][0]["id"]
        );
        let initial = ids(&inventory);
        let inactive_id = inventory.groups[0].tabs[1].id.clone();
        let capture = inventory
            .capture_published("tabs", std::slice::from_ref(&inactive_id))
            .unwrap();
        groups.focus(second).unwrap();
        inventory.observe(&groups, &[&a, &b]).unwrap();
        assert_eq!(ids(&inventory), initial);
        assert!(capture.current(&inventory, &groups));
        assert_eq!(capture.kind(), "tabs");
        assert_eq!(capture.ids(), &[inactive_id]);
        a.undo();
        inventory.observe(&groups, &[&a, &b]).unwrap();
        assert_eq!(ids(&inventory), initial);
        assert!(capture.current(&inventory, &groups));
        assert_eq!(a.text.to_string(), "猫🙂\r\n");
    }
    #[test]
    fn removed_reopened_input_and_foreign_engine_cannot_borrow_original_close_handle() {
        let doc = Document::from_text("猫\r\n");
        let mut groups = Groups::with_empty_group();
        let first = groups.open(doc.id).unwrap().active.unwrap();
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[&doc]).unwrap();
        let id = ids(&inventory)[0].clone();
        let capture = inventory
            .capture_published("tabs", std::slice::from_ref(&id))
            .unwrap();
        groups.close(first).unwrap();
        inventory.observe(&groups, &[]).unwrap();
        groups.open(doc.id).unwrap();
        inventory.observe(&groups, &[&doc]).unwrap();
        assert_ne!(ids(&inventory)[0], id);
        assert!(!capture.current(&inventory, &groups));
        assert!(capture.engine_current(&groups));
        let mut foreign = Groups::with_empty_group();
        foreign.open(doc.id).unwrap();
        assert!(!capture.engine_current(&foreign));
        inventory.observe(&foreign, &[&doc]).unwrap();
        assert!(!capture.current(&inventory, &foreign));
    }
    #[test]
    fn group_capture_rejects_new_members_but_owned_steps_keep_engine_authority() {
        let a = Document::from_text("a");
        let b = Document::from_text("b");
        let mut groups = Groups::with_empty_group();
        let first = groups.open(a.id).unwrap().active.unwrap();
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[&a, &b]).unwrap();
        let id = inventory.groups[0].id.clone();
        let capture = inventory
            .capture_published("groups", &[id.clone(), id.clone()])
            .unwrap();
        assert_eq!(capture.ids().len(), 2);
        assert_eq!(capture.groups().len(), 1);
        groups.open(b.id).unwrap();
        inventory.observe(&groups, &[&a, &b]).unwrap();
        assert!(!capture.current(&inventory, &groups));
        assert!(capture.engine_current(&groups));
        groups.close(first).unwrap();
        inventory.observe(&groups, &[&b]).unwrap();
        assert!(!capture.current(&inventory, &groups));
        assert!(capture.engine_current(&groups));
        assert_eq!(capture.memberships(), &[first]);
    }
    #[test]
    fn published_capture_never_resnapshots_live_membership_before_admission() {
        let a = Document::from_text("猫🙂\r\n");
        let b = Document::from_text("second\r\n");
        let mut groups = Groups::with_empty_group();
        let original = groups.open(a.id).unwrap().active.unwrap();
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[&a, &b]).unwrap();
        let group_id = inventory.groups[0].id.clone();
        let tab_id = inventory.groups[0].tabs[0].id.clone();
        let wire = inventory.wire().unwrap();
        // Client admission still sees the old successful mirror, even if App
        // state changed before that queued operation is dispatched.
        groups.open(b.id).unwrap();
        let group_capture = inventory.capture_published("groups", &[group_id]).unwrap();
        assert_eq!(group_capture.memberships(), &[original]);
        assert_eq!(group_capture.groups()[0].members(), &[original]);
        assert!(!group_capture.current(&inventory, &groups));
        assert_eq!(inventory.wire().unwrap(), wire);
        // A tab operation owns only its original membership, not new siblings.
        let tab_capture = inventory.capture_published("tabs", &[tab_id]).unwrap();
        assert!(tab_capture.current(&inventory, &groups));
        groups.close(original).unwrap();
        assert!(!tab_capture.current(&inventory, &groups));
        assert!(tab_capture.engine_current(&groups));
        assert_eq!(a.text.to_string(), "猫🙂\r\n");
    }
    #[test]
    fn resource_change_rotates_tab_but_preserves_group_and_owned_membership() {
        let mut doc = Document::from_text("猫\r\n");
        let mut groups = Groups::with_empty_group();
        groups.open(doc.id).unwrap();
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[&doc]).unwrap();
        let group = inventory.groups[0].id.clone();
        let tab = ids(&inventory)[0].clone();
        let capture = inventory
            .capture_published("tabs", std::slice::from_ref(&tab))
            .unwrap();
        // Resource metadata changes after an owned SaveAs without replacing the model.
        doc.path = Some(std::env::temp_dir().join("vscli-tab-inventory-new-name.txt"));
        assert!(inventory.validate(&groups, &[&doc]).is_err());
        inventory.observe(&groups, &[&doc]).unwrap();
        assert_eq!(inventory.groups[0].id, group);
        assert_ne!(ids(&inventory)[0], tab);
        assert!(!capture.current(&inventory, &groups));
        assert!(capture.engine_current(&groups));
        assert!(groups.membership_current(capture.memberships()[0]));
    }
    #[test]
    fn tab_labels_admit_1024_bytes_and_refuse_oversize_before_inventory_publication() {
        let mut doc = Document::from_text("猫🙂\r\n");
        doc.insert("!", false);
        let mut groups = Groups::with_empty_group();
        groups.open(doc.id).unwrap();
        let mut inventory = Inventory::default();
        doc.path = Some(std::env::temp_dir().join("a".repeat(1024)));
        inventory.observe(&groups, &[&doc]).unwrap();
        assert_eq!(inventory.groups[0].tabs[0].label.len(), 1024);
        let old = inventory.wire().unwrap();
        let next = inventory.next;
        let id = doc.id;
        let revision = doc.revision;
        let epoch = doc.text_epoch();
        let save = doc.save_generation();
        // Unicode is measured in UTF-8 bytes, matching the host bound. Its
        // encoded resource remains within the separate 4096-byte URI budget.
        doc.path = Some(std::env::temp_dir().join("猫".repeat(342)));
        assert!(document_uri(&doc).unwrap().len() <= 4096);
        assert_eq!(doc.name().len(), 1026);
        assert!(inventory.observe(&groups, &[&doc]).is_err());
        assert_eq!(inventory.wire().unwrap(), old);
        assert_eq!(inventory.next, next);
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.text_epoch(),
                doc.save_generation()
            ),
            (id, revision, epoch, save)
        );
        assert!(doc.dirty());
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂\r\n");
        doc.redo();
        assert_eq!(doc.text.to_string(), "!猫🙂\r\n");
    }
    #[test]
    fn late_missing_model_counter_exhaustion_and_invalid_targets_preserve_inventory() {
        let a = Document::from_text("a");
        let b = Document::from_text("b");
        let mut groups = Groups::with_empty_group();
        groups.open(a.id).unwrap();
        let mut inventory = Inventory::default();
        inventory.observe(&groups, &[&a]).unwrap();
        let old = inventory.wire().unwrap();
        let next = inventory.next;
        groups.open(b.id).unwrap();
        assert!(inventory.observe(&groups, &[&a]).is_err());
        assert_eq!(inventory.wire().unwrap(), old);
        assert_eq!(inventory.next, next);
        inventory.next = u64::MAX;
        assert!(inventory.observe(&groups, &[&a, &b]).is_err());
        assert_eq!(inventory.wire().unwrap(), old);
        assert_eq!(inventory.next, u64::MAX);
        assert!(
            inventory
                .capture_published("tabs", &["missing".into()])
                .is_err()
        );
        assert!(inventory.capture_published("wrong", &[]).is_err());
        inventory.invalidate();
        assert!(inventory.wire().is_err());
    }
}
