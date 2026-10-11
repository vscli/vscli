//! Native visible-editor lifetimes, separate from shared document identities.
use super::*;
use crate::document::EditorViewProof;

#[derive(Clone)]
pub(crate) struct Frame {
    id: String,
    document: u64,
    view: u64,
    column: usize,
    uri: String,
    stamp: (u64, usize),
    proof: EditorViewProof,
    visible_ranges: Value,
}

#[derive(Clone, Default)]
pub(crate) struct Inventory {
    next: u64,
    pub(super) frames: Vec<Frame>,
    pub(super) active: Option<String>,
    invalid: bool,
}

impl Inventory {
    pub(crate) fn identities(&self) -> Vec<(u64, u64, String)> {
        self.frames
            .iter()
            .map(|frame| (frame.document, frame.view, frame.id.clone()))
            .collect()
    }
    pub(crate) fn id_for_view(&self, document: u64, view: u64) -> Option<String> {
        self.frames
            .iter()
            .find(|frame| frame.document == document && frame.view == view)
            .map(|frame| frame.id.clone())
    }
    pub(crate) fn set_visible_ranges(&mut self, ranges: &[(String, Value)]) {
        for frame in &mut self.frames {
            frame.visible_ranges = ranges
                .iter()
                .find(|(id, _)| *id == frame.id)
                .map_or_else(|| json!([]), |(_, ranges)| ranges.clone());
        }
    }
    pub(crate) fn invalidate(&mut self) {
        self.frames.clear();
        self.active = None;
        self.invalid = true;
    }

    /// Observe every native membership transition, including ones coalesced
    /// before transport publication. Removed handles never regain their IDs.
    pub(crate) fn observe(
        &mut self,
        documents: &[&Document],
        views: &[(u64, u64, usize)],
        active: Option<(u64, u64)>,
    ) -> Result<()> {
        anyhow::ensure!(
            views.len() <= 4,
            "Visible editor inventory exceeds four groups"
        );
        let mut frames = Vec::with_capacity(views.len());
        let mut next = self.next;
        for &(document, view, column) in views {
            anyhow::ensure!((1..=4).contains(&column), "Invalid editor view column");
            anyhow::ensure!(
                !frames.iter().any(|frame: &Frame| frame.view == view),
                "Duplicate native editor group"
            );
            let doc = documents
                .iter()
                .find(|doc| doc.id == document)
                .context("Visible editor document is absent")?;
            let proof = doc
                .editor_view_identity_for(view)
                .context("Visible editor view is absent")?;
            let stamp = doc
                .editor_view_stamp_for(view)
                .context("Visible editor view is absent")?;
            let uri = document_uri(doc)?;
            anyhow::ensure!(uri.len() <= 4096, "Editor resource exceeds 4096 bytes");
            let old = self.frames.iter().find(|frame| {
                frame.document == document
                    && frame.view == view
                    && frame.uri == uri
                    && frame.stamp == stamp
                    && doc.editor_view_current(&frame.proof)
            });
            let id = if let Some(old) = old {
                old.id.clone()
            } else {
                next = next
                    .checked_add(1)
                    .context("Native editor identifiers exhausted")?;
                format!("editor-{next}")
            };
            frames.push(Frame {
                id,
                document,
                view,
                column,
                uri,
                stamp,
                proof,
                visible_ranges: old.map_or_else(|| json!([]), |frame| frame.visible_ranges.clone()),
            });
        }
        let active = active.and_then(|(document, view)| {
            frames
                .iter()
                .find(|frame| frame.document == document && frame.view == view)
                .map(|frame| frame.id.clone())
        });
        self.next = next;
        self.frames = frames;
        self.active = active;
        self.invalid = false;
        Ok(())
    }

    pub(super) fn validate(&self, documents: &[&Document]) -> Result<()> {
        anyhow::ensure!(!self.invalid, "Native visible editor inventory is invalid");
        let mut count = documents
            .iter()
            .map(|doc| doc.secondary.len() + 1)
            .sum::<usize>();
        for frame in &self.frames {
            let doc = documents
                .iter()
                .find(|doc| doc.id == frame.document)
                .context("Visible editor model retired")?;
            anyhow::ensure!(
                doc.editor_view_current(&frame.proof) && document_uri(doc)? == frame.uri,
                "Visible editor resource or view retired"
            );
            let view = doc.view_state(Some(frame.view));
            anyhow::ensure!(
                view.secondary.len() < 10_000,
                "Visible editor selection limit exceeded"
            );
            count = count.saturating_add(view.secondary.len() + 1);
            anyhow::ensure!(
                count <= 16_384,
                "Document/editor mirrors exceed 16,384 selections"
            );
        }
        Ok(())
    }

    pub(super) fn wire(&self, documents: &[&Document], kinds: &HashSet<String>) -> Result<Value> {
        self.validate(documents)?;
        self.frames
            .iter()
            .map(|frame| {
                let doc = documents
                    .iter()
                    .find(|doc| doc.id == frame.document)
                    .context("Visible model retired")?;
                let mut value = json!({"id":frame.id,"document":frame.document,
                "viewColumn":frame.column,"selections":selections_for(doc,Some(frame.view)),
                "options":doc.editor_options_for(frame.view).context("Visible editor options unavailable")?,
                "visibleRanges":frame.visible_ranges});
                if kinds.contains(&frame.id) {
                    value["selectionKind"] = 3.into();
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()
            .map(Value::Array)
    }

    pub(super) fn stamp(&self, documents: &[&Document]) -> Result<Value> {
        Ok(json!({"editors": self.wire(documents,&HashSet::new())?, "activeEditor":self.active}))
    }

    fn current(&self, id: &str, doc: &Document) -> Option<&Frame> {
        (!self.invalid).then_some(())?;
        self.frames.iter().find(|frame| {
            frame.id == id
                && frame.document == doc.id
                && doc.editor_view_current(&frame.proof)
                && document_uri(doc).ok().as_ref() == Some(&frame.uri)
        })
    }
}

impl Prepared {
    pub(crate) fn with_editor_inventory(
        mut self,
        inventory: Inventory,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<Self> {
        let refs = documents.iter().chain(hidden).collect::<Vec<_>>();
        inventory.validate(&refs)?;
        self.state["editors"] = inventory.wire(&refs, &HashSet::new())?;
        self.state["activeEditor"] = json!(inventory.active);
        self.mirror.editor_stamp = inventory.stamp(&refs)?;
        self.mirror.editors = inventory.frames.clone();
        self.editor_inventory = Some(inventory);
        Ok(self)
    }
}

impl Client {
    pub(crate) fn completed_show_owner_valid(
        &self,
        service: &services::NativeService,
    ) -> Result<()> {
        anyhow::ensure!(
            !service
                .caller_retired
                .load(std::sync::atomic::Ordering::Acquire),
            "Original show caller retired"
        );
        anyhow::ensure!(
            matches!(service.operation, services::Operation::Show(_))
                && self
                    .service()
                    .is_some_and(|current| current.id == service.id
                        && current.request.session == service.request.session
                        && current.request.owner == service.request.owner),
            "Original show service is no longer pending"
        );
        self.validate_selected_owner(service.request.session, &service.request.owner)
    }
    pub(crate) fn set_editor_inventory(&mut self, inventory: Inventory) {
        if self.editor_inventory.is_some() {
            self.editor_inventory = Some(inventory);
        }
    }
    pub(crate) fn editor_inventory_explicit(&self) -> bool {
        self.editor_inventory.is_some()
    }
    pub(crate) fn editor_view_source(&self, id: &str, doc: &Document) -> Result<EditorViewProof> {
        let frame = self
            .mirror
            .editors
            .iter()
            .find(|frame| frame.id == id && frame.document == doc.id)
            .context("Original editor is no longer published")?;
        let current = self
            .editor_inventory
            .as_ref()
            .and_then(|inventory| inventory.current(id, doc))
            .context("Original visible editor retired")?;
        anyhow::ensure!(
            current.stamp == frame.stamp && current.uri == frame.uri,
            "Original editor resource or view changed"
        );
        Ok(frame.proof.clone())
    }
    pub(crate) fn editor_view_live(&self, id: &str, doc: &Document) -> bool {
        self.editor_view_source(id, doc).is_ok()
    }
    pub(crate) fn editor_id_for_document(&self, document: u64, view: u64) -> Option<String> {
        self.mirror
            .editors
            .iter()
            .find(|frame| frame.document == document && frame.view == view)
            .map(|frame| frame.id.clone())
    }
    pub(crate) fn mark_api_editor_selections(&mut self, editor: &str) {
        self.mirror.api_editor_selections.insert(editor.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        let mut doc = Document::from_text("猫🙂 A\r\nlast\r\n");
        doc.activate_view(1);
        doc.move_to(0, false);
        doc.insert("dirty ", false);
        doc.activate_view(2);
        doc.move_to(3, false);
        doc.activate_view(1);
        doc
    }

    #[test]
    fn independent_visible_editors_share_source_and_keep_ids_through_focus_language_and_columns() {
        let mut doc = document();
        let id = doc.id;
        let epoch = doc.text_epoch();
        let revision = doc.revision;
        let mut inventory = Inventory::default();
        inventory
            .observe(&[&doc], &[(id, 1, 1), (id, 2, 2)], Some((id, 1)))
            .unwrap();
        let first = inventory.frames[0].id.clone();
        let second = inventory.frames[1].id.clone();
        assert_ne!(first, second);
        let wire = inventory.wire(&[&doc], &HashSet::new()).unwrap();
        assert_eq!(wire[0]["selections"][0]["active"]["character"], 6);
        assert_eq!(wire[1]["selections"][0]["active"]["character"], 3);
        doc.set_language_override(Some("python".into())).unwrap();
        doc.activate_view(2);
        inventory
            .observe(&[&doc], &[(id, 2, 1), (id, 1, 2)], Some((id, 2)))
            .unwrap();
        assert_eq!(inventory.frames[0].id, second);
        assert_eq!(inventory.frames[1].id, first);
        assert_eq!(inventory.active.as_ref(), Some(&second));
        assert_eq!(
            (doc.id, doc.revision, doc.text_epoch()),
            (id, revision, epoch)
        );
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂 A\r\nlast\r\n");
        doc.redo();
        assert_eq!(doc.text.to_string(), "dirty 猫🙂 A\r\nlast\r\n");
    }

    #[test]
    fn coalesced_visibility_and_native_view_aba_never_reuse_retired_editor_id() {
        let mut doc = document();
        let id = doc.id;
        let mut inventory = Inventory::default();
        inventory
            .observe(&[&doc], &[(id, 1, 1)], Some((id, 1)))
            .unwrap();
        let retired = inventory.frames[0].id.clone();
        inventory.observe(&[&doc], &[], None).unwrap();
        inventory
            .observe(&[&doc], &[(id, 1, 1)], Some((id, 1)))
            .unwrap();
        assert_ne!(inventory.frames[0].id, retired);
        assert!(inventory.current(&retired, &doc).is_none());
        let retired = inventory.frames[0].id.clone();
        doc.remove_view(1);
        doc.activate_view(1);
        inventory
            .observe(&[&doc], &[(id, 1, 1)], Some((id, 1)))
            .unwrap();
        assert_ne!(inventory.frames[0].id, retired);
        assert!(inventory.current(&retired, &doc).is_none());
    }

    #[test]
    fn resource_transition_retires_editor_without_retargeting_shared_document_history() {
        let mut doc = document();
        let id = doc.id;
        let epoch = doc.text_epoch();
        let mut inventory = Inventory::default();
        inventory
            .observe(&[&doc], &[(id, 1, 1)], Some((id, 1)))
            .unwrap();
        let old = inventory.frames[0].id.clone();
        doc.path = Some(std::env::temp_dir().join("vscli-editor-inventory-resource.cpp"));
        assert!(inventory.current(&old, &doc).is_none());
        inventory
            .observe(&[&doc], &[(id, 1, 1)], Some((id, 1)))
            .unwrap();
        assert_ne!(inventory.frames[0].id, old);
        assert_eq!((doc.id, doc.text_epoch()), (id, epoch));
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂 A\r\nlast\r\n");
    }

    #[test]
    fn exhausted_inventory_clock_and_selection_budget_refuse_before_publication() {
        let mut doc = document();
        let id = doc.id;
        let mut inventory = Inventory::default();
        inventory
            .observe(&[&doc], &[(id, 1, 1)], Some((id, 1)))
            .unwrap();
        let previous = inventory.frames[0].id.clone();
        inventory.next = u64::MAX;
        assert!(
            inventory
                .observe(&[&doc], &[(id, 1, 1), (id, 2, 2)], Some((id, 1)))
                .is_err()
        );
        assert_eq!(inventory.frames.len(), 1);
        assert_eq!(inventory.frames[0].id, previous);
        doc.secondary = vec![crate::document::Selection::caret(0); 8192];
        assert!(inventory.wire(&[&doc], &HashSet::new()).is_err());
        assert_eq!(inventory.frames[0].id, previous);
        assert!(doc.dirty());
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂 A\r\nlast\r\n");
    }
}
