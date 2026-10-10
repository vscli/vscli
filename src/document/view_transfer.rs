//! Bounded current view projection staged before joint group/layout publication.
use super::{Document, ViewState};
use anyhow::{Result, ensure};

const MAX_SELECTIONS: usize = 10_000;

/// Exclusive lease; preparation changes no logical view or text/history state.
/// The caller must publish its checked group/layout transaction before this lease.
pub struct ViewTransfer<'a> {
    document: &'a mut Document,
    source: u64,
    target: u64,
    state: ViewState,
    source_snippet_generation: u64,
}
impl Document {
    pub fn prepare_view_transfer(&mut self, source: u64, target: u64) -> Result<ViewTransfer<'_>> {
        ensure!(
            source == self.active_view,
            "Transfer source view is not active"
        );
        ensure!(
            source != 0 && target != 0 && source != target,
            "Invalid transfer view identity"
        );
        ensure!(
            self.view.secondary.len() < MAX_SELECTIONS,
            "Transfer exceeds 10,000 selections"
        );
        let len = self.len();
        ensure!(
            self.view.cursor <= len
                && self.view.anchor.is_none_or(|position| position <= len)
                && self
                    .view
                    .secondary
                    .iter()
                    .all(|selection| selection.cursor <= len
                        && selection.anchor.is_none_or(|position| position <= len)),
            "Transfer selection is outside current source"
        );
        let source_snippet_generation = self
            .view
            .snippet_generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Transfer source snippet generation exhausted"))?;
        let generation = self
            .other_views
            .get(&target)
            .map_or(0, |view| view.snippet_generation)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Transfer snippet generation exhausted"))?;
        // Copy only current public projection, never cursor history, snippets,
        // generated pair ownership or an unrelated target's selection.
        let state = ViewState {
            cursor: self.view.cursor,
            anchor: self.view.anchor,
            secondary: self.view.secondary.clone(),
            top: self.view.top,
            left: self.view.left,
            desired_column: self.view.desired_column,
            snippet_generation: generation,
            ..ViewState::default()
        };
        self.other_views.try_reserve(1)?;
        Ok(ViewTransfer {
            document: self,
            source,
            target,
            state,
            source_snippet_generation,
        })
    }
}
impl ViewTransfer<'_> {
    /// Infallible after preparation. Focus remains the caller's responsibility.
    pub fn publish(self) {
        self.document.view.snippet = None;
        self.document.view.snippet_generation = self.source_snippet_generation;
        self.document.view.pairs = super::typing::Pairs::default();
        self.document.view.cursor_history.clear();
        self.document.remove_view(self.source);
        self.document.other_views.insert(self.target, self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Selection;
    const SOURCE: &str = "猫🙂 first\r\nsecond e\u{301}\r\n";
    fn source() -> Document {
        let mut document = Document::from_text(SOURCE);
        document.activate_view(1);
        document.move_to(2, false);
        document.top = 1;
        document.left = 3;
        document.set_selections(vec![Selection::caret(2), Selection::caret(12)]);
        document
    }
    #[test]
    fn dropped_projection_preserves_views_dirty_history_and_pending_redo() {
        let mut document = source();
        document.insert("λ", false);
        document.undo();
        let original = document.selections();
        let epoch = document.text_epoch();
        drop(document.prepare_view_transfer(1, 2).unwrap());
        assert_eq!(document.active_view, 1);
        assert!(!document.other_views.contains_key(&2));
        assert_eq!(document.selections(), original);
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(document.text.to_string(), SOURCE);
        document.redo();
        assert!(document.text.to_string().contains('λ'));
    }
    #[test]
    fn new_target_preserves_unicode_crlf_projection_and_full_saved_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let mut document = source();
        let selections = document.selections();
        let epoch = document.text_epoch();
        let id = document.id;
        let path = directory.path().join("target.txt");
        let receipt = document.capture_save(path.clone()).unwrap();
        document.prepare_view_transfer(1, 2).unwrap().publish();
        document.activate_view(2);
        assert_eq!(document.selections(), selections);
        assert_eq!((document.top, document.left), (1, 3));
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(document.id, id);
        assert!(!document.other_views.contains_key(&1));
        document.check_save_snapshot(&receipt).unwrap();
        document.save_to(&path, false).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), SOURCE.as_bytes());
        assert_eq!(receipt.text().to_string(), SOURCE);
    }
    #[test]
    fn existing_target_is_replaced_without_changing_unrelated_view_or_text_redo() {
        let mut document = source();
        document.activate_view(3);
        document.move_to(4, false);
        document.top = 4;
        document.activate_view(2);
        document.move_to(7, false);
        document.activate_view(1);
        document.insert("x", false);
        document.undo();
        let selections = document.selections();
        document.prepare_view_transfer(1, 2).unwrap().publish();
        document.activate_view(2);
        assert_eq!(document.selections(), selections);
        document.redo();
        assert!(document.text.to_string().contains('x'));
        document.activate_view(3);
        assert_eq!(document.top, 4);
        // Source edits map unrelated selections according to normal text history.
        assert_eq!(document.cursor, 5);
    }
    #[test]
    fn invalid_cohort_identity_and_exhaustion_refuse_without_mutating_views_or_redo() {
        let mut document = source();
        document.insert("x", false);
        document.undo();
        let selection = document.selections();
        let epoch = document.text_epoch();
        for (source, target) in [(9, 2), (1, 1), (1, 0)] {
            assert!(document.prepare_view_transfer(source, target).is_err());
            assert_eq!(document.selections(), selection);
            assert_eq!(document.text_epoch(), epoch);
        }
        document.secondary = vec![Selection::caret(0); MAX_SELECTIONS];
        assert!(document.prepare_view_transfer(1, 2).is_err());
        document.secondary.clear();
        document.cursor = document.len() + 1;
        assert!(document.prepare_view_transfer(1, 2).is_err());
        document.cursor = 2;
        document.other_views.insert(
            2,
            ViewState {
                snippet_generation: u64::MAX,
                ..ViewState::default()
            },
        );
        assert!(document.prepare_view_transfer(1, 2).is_err());
        assert_eq!(document.active_view, 1);
        assert_eq!(document.text.to_string(), SOURCE);
        document.redo();
    }
}
