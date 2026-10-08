use super::*;
use crate::snippet::{Placeholder, Template};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct Session {
    markers: Vec<Option<Placeholder>>,
    active: u32,
    edited: bool,
}

impl Session {
    pub(super) fn retain_removed_from(&mut self, current: &Self) {
        for (previous, current) in self.markers.iter_mut().zip(&current.markers) {
            if current.is_none() {
                *previous = None;
            }
        }
    }
    fn groups(&self) -> Vec<u32> {
        let mut groups: Vec<_> = self
            .markers
            .iter()
            .flatten()
            .map(|p| p.index)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        groups.sort_by_key(|index| {
            if *index == 0 {
                u64::MAX
            } else {
                u64::from(*index)
            }
        });
        groups
    }
    fn selections(&self) -> Vec<Selection> {
        self.markers
            .iter()
            .flatten()
            .filter(|p| p.index == self.active)
            .map(|p| Selection {
                cursor: p.range.end,
                anchor: Some(p.range.start),
                desired_column: None,
            })
            .collect()
    }
    fn contains(&self, range: &Range<usize>) -> bool {
        self.markers.iter().flatten().any(|p| {
            p.index == self.active && p.range.start <= range.start && range.end <= p.range.end
        })
    }
    fn change(&mut self, range: &Range<usize>, added: usize) -> bool {
        let owners: Vec<_> = self
            .markers
            .iter()
            .enumerate()
            .filter_map(|(id, p)| {
                p.as_ref()
                    .filter(|p| {
                        p.index == self.active
                            && p.range.start <= range.start
                            && range.end <= p.range.end
                    })
                    .map(|_| id)
            })
            .collect();
        if owners.is_empty() {
            return false;
        }
        let ancestors: BTreeSet<_> = owners
            .iter()
            .flat_map(|id| self.markers[*id].as_ref().unwrap().parents.iter().copied())
            .collect();
        let shift = added as isize - range.len() as isize;
        for (id, marker) in self.markers.iter_mut().enumerate() {
            let Some(p) = marker else {
                continue;
            };
            if owners.contains(&id) || ancestors.contains(&id) {
                if p.range.start > range.start {
                    p.range.start = range.start;
                }
                p.range.end = p.range.end.saturating_add_signed(shift);
            } else if !range.is_empty()
                && p.parents.iter().any(|parent| owners.contains(parent))
                && range.start <= p.range.start
                && p.range.end <= range.end
            {
                *marker = None;
            } else if p.range.start >= range.end {
                p.range.start = p.range.start.saturating_add_signed(shift);
                p.range.end = p.range.end.saturating_add_signed(shift);
            } else if p.range.end > range.start {
                // An edit crossing a foreign placeholder boundary cannot retain
                // a sound linked range; terminate this session atomically.
                return false;
            }
        }
        self.edited = true;
        true
    }
}

impl ViewState {
    pub(super) fn map_snippet(&mut self, range: &Range<usize>, added: usize) {
        if let Some(session) = &mut self.snippet
            && !session.change(range, added)
        {
            self.snippet = None;
            self.snippet_generation += 1;
        }
    }
}

impl Document {
    pub fn cancel_snippet(&mut self) {
        if self.snippet.take().is_some() {
            self.snippet_generation += 1;
        }
    }

    pub fn in_snippet(&self) -> bool {
        self.snippet.as_ref().is_some_and(|s| {
            self.selections()
                .iter()
                .all(|selection| s.contains(&selection.range()))
        })
    }

    pub fn has_snippet_step(&self, backwards: bool) -> bool {
        if !self.in_snippet() {
            return false;
        }
        let session = self.snippet.as_ref().unwrap();
        let groups = session.groups();
        let Some(index) = groups.iter().position(|index| *index == session.active) else {
            return false;
        };
        if backwards {
            index > 0
        } else {
            index + 1 < groups.len()
        }
    }

    /// Stage all expansions before changing any buffer or selection. The caller
    /// supplies resolved variables; this entry point follows API insertion.
    pub fn insert_snippet(
        &mut self,
        template: &Template,
        variables: &BTreeMap<String, String>,
    ) -> Result<()> {
        let mut selections = self.selections();
        selections.sort_by_key(|s| s.range().start);
        let mut end = 0;
        let mut shift = 0isize;
        let mut changes = Vec::new();
        let mut markers = Vec::new();
        let mut final_cursors = Vec::new();
        let mut total_bytes = 0usize;
        for selection in selections {
            let range = selection.range();
            if range.start < end || range.end > self.len() {
                bail!("Overlapping or invalid snippet selections");
            }
            end = range.end;
            let expanded = template.expand(variables)?;
            total_bytes = total_bytes.saturating_add(expanded.text.len());
            if total_bytes > MAX_FILE_BYTES as usize {
                bail!("Combined snippet expansion exceeds 32 MiB");
            }
            let start = range.start.saturating_add_signed(shift);
            let inserted = expanded.text.chars().count();
            let base_id = markers.len();
            for mut marker in expanded.placeholders {
                marker.range.start += start;
                marker.range.end += start;
                for parent in &mut marker.parents {
                    *parent += base_id;
                }
                markers.push(Some(marker));
            }
            if markers.len() > 10_000 {
                bail!("Combined snippet expansion exceeds 10,000 markers");
            }
            final_cursors.push(Selection::caret(start + inserted));
            shift += inserted as isize - range.len() as isize;
            if !range.is_empty() || !expanded.text.is_empty() {
                changes.push((range, expanded.text));
            }
        }
        let original = self.snapshot();
        self.cancel_snippet();
        self.snippet_generation += 1;
        self.apply_changes(changes);
        if self.revision != original.revision {
            self.replace_undo_snapshot(original);
        }
        let mut session = Session {
            markers,
            active: 0,
            edited: false,
        };
        if let Some(first) = session.groups().first().copied() {
            session.active = first;
            self.set_selections(session.selections());
            if first != 0 {
                self.snippet = Some(session);
            }
        } else {
            self.set_selections(final_cursors);
        }
        Ok(())
    }

    pub fn step_snippet(&mut self, backwards: bool) -> Result<bool> {
        if !self.in_snippet() {
            self.cancel_snippet();
            return Ok(false);
        }
        let session = self.snippet.as_ref().unwrap();
        let groups = session.groups();
        let index = groups.iter().position(|i| *i == session.active).unwrap();
        let next = if backwards {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|i| *i < groups.len())
        };
        let Some(next) = next else {
            return Ok(false);
        };
        let next = groups[next];
        let merge_undo = session.edited
            && self.undo.last().is_some_and(|snapshot| {
                snapshot.view_id == self.active_view
                    && snapshot.snippet_generation == self.snippet_generation
            });
        let has_transform = session
            .markers
            .iter()
            .flatten()
            .any(|p| p.index == session.active && p.transform.is_some());
        let mut transaction_selections = self.selections();
        let mut changes = Vec::new();
        for p in session
            .markers
            .iter()
            .flatten()
            .filter(|p| p.index == session.active)
        {
            if let Some(transform) = &p.transform {
                let current = self.text.slice(p.range.clone()).to_string();
                let replacement = transform.apply(&current)?;
                if current != replacement {
                    changes.push((p.range.clone(), replacement));
                }
            }
        }
        let changed = !changes.is_empty();
        self.apply_changes(changes);
        if has_transform && let Some(session) = &self.snippet {
            transaction_selections = session.selections();
        }
        if changed
            && merge_undo
            && let Some(last) = self.undo.pop()
        {
            if let Some(previous) = self.undo.last_mut() {
                previous.changes.extend(last.changes);
            } else {
                self.undo.push(last);
            }
        }
        if (merge_undo || changed)
            && let Some(snapshot) = self.undo.last_mut()
        {
            snapshot.after_selections = Some(transaction_selections);
        }
        let Some(session) = &mut self.snippet else {
            return Ok(false);
        };
        session.active = next;
        session.edited = false;
        let selections = session.selections();
        self.set_selections(selections);
        if next == 0 {
            self.cancel_snippet();
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_insertions_share_document_identity_and_preserve_crlf_through_save_and_undo() {
        let original = "cat 猫\r\ncat 🐶\r\n";
        let mut doc = Document::from_text(original);
        let id = doc.id;
        doc.activate_view(1);
        doc.activate_view(2);
        doc.move_to(doc.len(), false);
        doc.activate_view(1);
        let second = doc.line_start(1);
        doc.set_selections(vec![
            Selection {
                cursor: 3,
                anchor: Some(0),
                desired_column: None,
            },
            Selection {
                cursor: second + 3,
                anchor: Some(second),
                desired_column: None,
            },
        ]);
        doc.insert_snippet(
            &Template::parse("${1:name} = ${2:value};$0").unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(doc.in_snippet());
        doc.activate_view(2);
        assert!(!doc.in_snippet());
        assert_eq!(doc.id, id);
        doc.activate_view(1);
        doc.insert("e\u{301}猫", true);
        doc.step_snippet(false).unwrap();
        doc.insert("🙂", true);
        doc.step_snippet(false).unwrap();
        assert!(!doc.in_snippet());
        let expected = "e\u{301}猫 = 🙂; 猫\r\ne\u{301}猫 = 🙂; 🐶\r\n";
        assert_eq!(doc.text.to_string(), expected);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.txt");
        doc.save_to(&path, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
        doc.undo();
        doc.undo();
        doc.undo();
        assert_eq!(doc.text.to_string(), original);
        doc.activate_view(2);
        assert_eq!(doc.cursor, original.chars().count());
        doc.save().unwrap();
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    }

    #[test]
    fn expansion_and_transform_failures_preserve_text_selections_revision_and_undo() {
        let mut doc = Document::default();
        doc.insert("seed", false);
        let mut variables = BTreeMap::new();
        variables.insert("BIG".into(), "x".repeat(1024 * 1024 + 1));
        assert!(
            doc.insert_snippet(&Template::parse("$BIG").unwrap(), &variables)
                .is_err()
        );
        assert_eq!(doc.text.to_string(), "seed");
        doc.undo();
        assert!(doc.is_empty());
        doc.insert_snippet(
            &Template::parse("${1:x} ${1/x/y/} ${1/(?=x)/z/}$0").unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        doc.insert("x", true);
        let text = doc.text.clone();
        let selections = doc.selections();
        let revision = doc.revision;
        assert!(doc.step_snippet(false).is_err());
        assert_eq!(doc.text, text);
        assert_eq!(doc.selections(), selections);
        assert_eq!(doc.revision, revision);
        assert!(doc.in_snippet());
    }

    #[test]
    fn editing_outside_a_session_cancels_it_without_reviving_it_on_undo() {
        let mut doc = Document::default();
        doc.insert_snippet(
            &Template::parse("${1:cat} end$0").unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        doc.move_to(doc.len(), false);
        assert!(!doc.in_snippet());
        doc.insert("!", true);
        doc.undo();
        assert!(!doc.in_snippet());
        assert_eq!(doc.text.to_string(), "cat end");
    }

    #[test]
    fn transformations_do_not_merge_another_views_edit_into_their_undo() {
        let mut doc = Document::default();
        doc.activate_view(1);
        doc.insert_snippet(
            &Template::parse("${1:x} ${1/(.*)/${1:/upcase}/}$0").unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        doc.insert("cat", true);
        doc.activate_view(2);
        doc.set_selections(vec![Selection::caret(1)]);
        doc.insert("!", true);
        doc.activate_view(1);
        doc.step_snippet(false).unwrap();
        assert_eq!(doc.text.to_string(), "c!at CAT");
        doc.undo();
        assert_eq!(doc.text.to_string(), "c!at cat");
        doc.undo();
        assert_eq!(doc.text.to_string(), "cat cat");
        doc.undo();
        assert_eq!(doc.text.to_string(), "x x");
    }
}
