use super::*;
impl App {
    pub fn sync_pane(&mut self) {
        let Some(document) = self.active_document().map(|doc| doc.id) else {
            self.panes.clear();
            self.pane_areas.clear();
            self.active_pane = 0;
            self.editor_area = Rect::default();
            return;
        };
        if self.panes.is_empty() {
            self.panes.push(Pane {
                id: self.next_pane_id,
                document,
            });
            self.next_pane_id += 1;
            self.active_pane = 0;
        }
        self.panes[self.active_pane].document = document;
        for pane in &mut self.panes {
            if !self.documents.iter().any(|doc| doc.id == pane.document) {
                pane.document = document;
            }
        }
        let view = self.panes[self.active_pane].id;
        self.doc_mut().activate_view(view);
    }
    pub fn focus_pane(&mut self, index: usize) {
        if index >= self.panes.len() {
            return;
        }
        self.breadcrumbs_ui_command("vscli.focusPane");
        self.observe_navigation(navigation_history::Reason::Ordinary);
        self.sync_pane();
        self.active_pane = index;
        self.active = self
            .documents
            .iter()
            .position(|doc| doc.id == self.panes[index].document)
            .unwrap_or(self.active);
        let view = self.panes[index].id;
        self.doc_mut().activate_view(view);
        self.focus = Focus::Editor;
        self.observe_navigation(navigation_history::Reason::EditorChange);
        self.observe_outline();
        self.observe_breadcrumbs();
    }
    pub(super) fn split_editor(&mut self, horizontal: bool) {
        if self.active_document().is_none() {
            return;
        }
        if self.panes.len() >= 4 {
            self.message = "Editor group limit reached (4)".into();
            return;
        }
        self.sync_pane();
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        self.panes.insert(
            self.active_pane + 1,
            Pane {
                id,
                document: self.doc().id,
            },
        );
        self.horizontal_split = horizontal;
        self.focus_pane(self.active_pane + 1);
    }
    pub(super) fn close_pane(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        if self.panes.len() == 1 {
            self.request_close(AfterSave::Close);
            return;
        }
        self.record_closed();
        let pane = self.panes.remove(self.active_pane);
        for doc in &mut self.documents {
            doc.remove_view(pane.id);
        }
        self.active_pane = self.active_pane.min(self.panes.len() - 1);
        self.active = self
            .documents
            .iter()
            .position(|doc| doc.id == self.panes[self.active_pane].document)
            .unwrap_or(0);
        let id = self.panes[self.active_pane].id;
        self.doc_mut().activate_view(id);
        self.focus = Focus::Editor;
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
