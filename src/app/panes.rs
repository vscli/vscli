use super::*;
use crate::editor_groups::{Change, Membership, Navigate};
impl App {
    pub fn editor_groups(&self) -> &crate::editor_groups::Groups {
        &self.editor_groups
    }
    pub(super) fn active_tab_membership(&self) -> Option<Membership> {
        (!self.group_fallback)
            .then(|| self.editor_groups.active_membership())
            .flatten()
    }
    pub(super) fn project_editor_groups(&mut self) {
        self.panes = self
            .editor_groups
            .groups()
            .iter()
            .filter_map(|group| {
                group.active().map(|tab| Pane {
                    id: group.id().value(),
                    document: tab.document(),
                })
            })
            .collect();
        self.active_pane = self
            .editor_groups
            .active_group()
            .and_then(|group| self.panes.iter().position(|pane| pane.id == group.value()))
            .unwrap_or(0);
        if let Some(member) = self.editor_groups.active_membership()
            && let Some(index) = self
                .documents
                .iter()
                .position(|doc| doc.id == member.document)
        {
            self.active = index;
            self.documents[index].activate_view(member.group.value());
        }
        if self.panes.is_empty() {
            self.active = 0;
            self.active_pane = 0;
            self.pane_areas.clear();
            self.editor_area = Rect::default();
        }
    }
    pub(super) fn apply_group_change(&mut self, change: Change) {
        for member in &change.inserted {
            let split_copy = change
                .previous
                .is_some_and(|source| source.document == member.document)
                && change.created_groups.contains(&member.group);
            if !split_copy
                && self.editor_groups.memberships(member.document).count() > 1
                && let Some(doc) = self
                    .documents
                    .iter_mut()
                    .find(|doc| doc.id == member.document)
            {
                // An ordinary new membership starts at the origin; splitting
                // alone copies the source group's current view.
                doc.activate_view(member.group.value());
                doc.clear_secondary();
                doc.move_to(0, false);
                doc.top = 0;
                doc.left = 0;
            }
        }
        for member in &change.removed {
            if let Some(doc) = self
                .documents
                .iter_mut()
                .find(|doc| doc.id == member.document)
            {
                doc.remove_view(member.group.value());
            }
        }
        // Retiring a group retires all its historical views, including views
        // retained by models that no longer have a tab in that group.
        for group in change.removed_groups {
            for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                doc.remove_view(group.value());
            }
        }
        self.project_editor_groups();
        if change.changed {
            if matches!(self.modal, Some(Modal::Confirm(AfterSave::Close)))
                && self
                    .close_membership
                    .is_some_and(|member| Some(member) != self.active_tab_membership())
            {
                self.modal = None;
                self.cancel_save_continuations();
                self.message =
                    "Close dialog retired: the active tab changed; buffers retained".into();
            }

            if let Some(client) = &mut self.lsp
                && let Err(error) = client.invalidate_editor_views()
            {
                self.message = format!("Native editor-view ownership retired: {error:#}");
            }
            self.tab_hits.clear();
            self.observe_outline();
            self.observe_breadcrumbs();
        }
    }
    pub(super) fn can_admit_editor(&self, document: u64) -> Result<()> {
        if self.group_fallback {
            return Ok(());
        }
        self.editor_groups.can_open(document)
    }
    pub fn sync_pane(&mut self) {
        if self.group_fallback {
            self.sync_legacy_pane();
            return;
        }
        let desired = self.active_document().map(|doc| doc.id);
        // Recovery and public fixture construction can install models directly.
        // Admit every retained model; a display limit cannot discard recovery.
        let unassigned_count = self
            .documents
            .iter()
            .filter(|doc| self.editor_groups.memberships(doc.id).next().is_none())
            .count();
        let occupied = self
            .editor_groups
            .active_group()
            .and_then(|id| self.editor_groups.group(id))
            .map_or(0, |group| group.tabs().len());
        if unassigned_count > crate::editor_groups::MAX_TABS_PER_GROUP.saturating_sub(occupied)
            || self.documents.len() > crate::editor_groups::MAX_MEMBERSHIPS
        {
            self.group_fallback = true;
            self.message = "Editor-group layout unavailable: recovered buffers exceed the tab limit; all buffers retained".into();
            self.next_pane_id = self
                .panes
                .iter()
                .map(|pane| pane.id)
                .max()
                .unwrap_or(1)
                .saturating_add(1);
            self.sync_legacy_pane();
            return;
        }
        let removed: Vec<_> = self
            .editor_groups
            .groups()
            .iter()
            .flat_map(|group| {
                group
                    .tabs()
                    .iter()
                    .filter(|tab| !self.documents.iter().any(|doc| doc.id == tab.document()))
                    .map(|tab| Membership {
                        group: group.id(),
                        tab: tab.id(),
                        document: tab.document(),
                    })
            })
            .collect();
        for member in removed {
            match self.editor_groups.close(member) {
                Ok(change) => self.apply_group_change(change),
                Err(error) => {
                    self.message = format!("Editor-group update rejected: {error:#}");
                    return;
                }
            }
        }
        let unassigned: Vec<_> = self
            .documents
            .iter()
            .map(|doc| doc.id)
            .filter(|id| self.editor_groups.memberships(*id).next().is_none())
            .collect();
        for id in unassigned {
            match self.editor_groups.open(id) {
                Ok(change) => self.apply_group_change(change),
                Err(error) => {
                    self.message =
                        format!("Editor-group admission rejected; buffers retained: {error:#}");
                    return;
                }
            }
        }
        if let Some(id) = desired {
            match self.editor_groups.open(id) {
                Ok(change) => self.apply_group_change(change),
                Err(error) => {
                    self.message =
                        format!("Editor-group admission rejected; buffers retained: {error:#}");
                }
            }
        } else {
            self.project_editor_groups();
        }
    }
    fn sync_legacy_pane(&mut self) {
        let Some(document) = self.active_document().map(|doc| doc.id) else {
            self.panes.clear();
            self.pane_areas.clear();
            self.active_pane = 0;
            self.editor_area = Rect::default();
            return;
        };
        if self.panes.is_empty() {
            let Some(next) = self.next_pane_id.checked_add(1) else {
                self.message = "Editor view identifiers exhausted; buffers retained".into();
                return;
            };
            self.panes.push(Pane {
                id: self.next_pane_id,
                document,
            });
            self.next_pane_id = next;
            self.active_pane = 0;
        }
        self.active_pane = self.active_pane.min(self.panes.len() - 1);
        self.panes[self.active_pane].document = document;
        for pane in &mut self.panes {
            if !self.documents.iter().any(|doc| doc.id == pane.document) {
                pane.document = document;
            }
        }
        let view = self.panes[self.active_pane].id;
        self.doc_mut().activate_view(view);
    }
    pub(super) fn focus_tab(&mut self, member: Membership) -> Result<()> {
        self.breadcrumbs_ui_command("vscli.focusTab");
        self.observe_navigation(navigation_history::Reason::Ordinary);
        let change = self.editor_groups.focus(member)?;
        self.apply_group_change(change);
        self.focus = Focus::Editor;
        self.observe_navigation(navigation_history::Reason::EditorChange);
        Ok(())
    }
    pub fn focus_pane(&mut self, index: usize) {
        if index >= self.panes.len() {
            return;
        }
        self.breadcrumbs_ui_command("vscli.focusPane");
        self.observe_navigation(navigation_history::Reason::Ordinary);
        self.sync_pane();
        if index >= self.panes.len() {
            return;
        }
        if self.group_fallback {
            self.active_pane = index;
            self.active = self
                .documents
                .iter()
                .position(|doc| doc.id == self.panes[index].document)
                .unwrap_or(self.active);
            let view = self.panes[index].id;
            self.doc_mut().activate_view(view);
        } else {
            let Some(group) = self
                .editor_groups
                .groups()
                .get(index)
                .map(|group| group.id())
            else {
                return;
            };
            match self.editor_groups.focus_group(group) {
                Ok(change) => self.apply_group_change(change),
                Err(error) => {
                    self.message = format!("Editor group rejected: {error:#}");
                    return;
                }
            }
        }
        self.focus = Focus::Editor;
        self.observe_navigation(navigation_history::Reason::EditorChange);
        self.observe_outline();
        self.observe_breadcrumbs();
    }
    pub(super) fn navigate_editor_tabs(&mut self, direction: Navigate) {
        self.sync_pane();
        if self.group_fallback {
            if self.documents.is_empty() {
                return;
            }
            self.active = if matches!(direction, Navigate::Previous | Navigate::PreviousInGroup) {
                (self.active + self.documents.len() - 1) % self.documents.len()
            } else {
                (self.active + 1) % self.documents.len()
            };
            self.sync_legacy_pane();
        } else {
            match self.editor_groups.navigate(direction) {
                Ok(change) => self.apply_group_change(change),
                Err(error) => self.message = format!("Editor navigation rejected: {error:#}"),
            }
        }
        self.focus = Focus::Editor;
    }
    pub(super) fn split_editor(&mut self, horizontal: bool) {
        if self.active_document().is_none() {
            return;
        }
        self.sync_pane();
        if self.group_fallback {
            self.message =
                "Group splits are unavailable while recovered buffers exceed the tab limit".into();
            return;
        }
        match self.editor_groups.split_active() {
            Ok(change) => {
                self.horizontal_split = horizontal;
                self.apply_group_change(change);
                self.focus = Focus::Editor;
            }
            Err(error) => self.message = format!("Editor group split rejected: {error:#}"),
        }
    }
    pub(super) fn close_tab_membership(&mut self, member: Membership) -> Result<()> {
        if !self.editor_groups.membership_current(member) {
            anyhow::bail!("Original editor tab was closed or replaced");
        }
        let change = self.editor_groups.close(member)?;
        self.record_closed_tab(member);
        self.apply_group_change(change);
        if self
            .editor_groups
            .memberships(member.document)
            .next()
            .is_none()
            && let Some(index) = self
                .documents
                .iter()
                .position(|doc| doc.id == member.document)
        {
            self.documents.remove(index);
            self.project_editor_groups();
        }
        if self.documents.is_empty() {
            self.session_closed_all();
        }
        Ok(())
    }
    pub(super) fn close_pane(&mut self) {
        self.begin_close_editor_group();
    }
    fn begin_close_editor_group(&mut self) {
        self.sync_pane();
        let Some(group) = self.editor_groups.active_group() else {
            return;
        };
        match self.editor_groups.group_proof(group) {
            Ok(proof) => {
                self.closing_group = Some(ClosingGroup { proof });
                self.advance_close_editor_group();
            }
            Err(error) => self.message = format!("Close group rejected: {error:#}"),
        }
    }
    pub(super) fn advance_close_editor_group(&mut self) {
        let Some(closing) = self.closing_group.take() else {
            return;
        };
        if !self.editor_groups.group_proof_current(&closing.proof) {
            self.message = "Close group retired: its tabs changed; buffers retained".into();
            return;
        }
        let dirty = closing.proof.members().iter().copied().find(|member| {
            self.editor_groups.memberships(member.document).count() == 1
                && (self.document_save_pending(member.document)
                    || self
                        .documents
                        .iter()
                        .any(|doc| doc.id == member.document && doc.dirty()))
        });
        if let Some(member) = dirty {
            match self.focus_tab(member) {
                Ok(()) => {
                    self.closing_group = Some(closing);
                    self.close_membership = Some(member);
                    self.request_close(AfterSave::Close);
                }
                Err(error) => {
                    self.message = format!("Close group retired; buffers retained: {error:#}")
                }
            }
        } else {
            match self.editor_groups.close_group(&closing.proof) {
                Ok(change) => {
                    for member in closing.proof.members() {
                        self.record_closed_tab(*member);
                    }
                    let removed: Vec<_> = change
                        .removed
                        .iter()
                        .map(|member| member.document)
                        .collect();
                    self.apply_group_change(change);
                    self.documents.retain(|doc| {
                        !removed.contains(&doc.id)
                            || self.editor_groups.memberships(doc.id).next().is_some()
                    });
                    self.project_editor_groups();
                    if self.documents.is_empty() {
                        self.session_closed_all();
                    }
                }
                Err(error) => {
                    self.message = format!("Close group retired; buffers retained: {error:#}")
                }
            }
        }
    }
    pub(super) fn refresh_closing_group(&mut self) {
        if let Some(closing) = &mut self.closing_group {
            match self.editor_groups.group_proof(closing.proof.group()) {
                Ok(proof) => closing.proof = proof,
                Err(_) => self.closing_group = None,
            }
        }
        self.advance_close_editor_group();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panes_share_edits_and_close_without_losing_the_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().into(), Profile::Linux);
        app.execute(
            "workbench.action.files.newUntitledFile",
            serde_json::Value::Null,
        );
        app.doc_mut().insert("abc\nxyz", false);
        app.doc_mut().move_to(1, false);
        app.execute("workbench.action.splitEditor", Value::Null);
        app.doc_mut().move_to(5, false);
        app.execute("workbench.action.focusFirstEditorGroup", Value::Null);
        assert_eq!(app.doc().cursor, 1);
        app.doc_mut().insert("!", false);
        app.execute("workbench.action.focusSecondEditorGroup", Value::Null);
        assert_eq!(app.doc().cursor, 6);
        assert_eq!(app.documents.len(), 1);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        assert_eq!(app.panes.len(), 1);
        assert!(app.modal.is_none());
        assert!(app.doc().dirty());
        assert_eq!(app.doc().text.to_string(), "a!bc\nxyz");
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        assert!(matches!(app.modal, Some(Modal::Confirm(_))));
    }
}

#[cfg(test)]
mod group_tabs_tests;
