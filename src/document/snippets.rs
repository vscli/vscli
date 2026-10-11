use super::*;
use crate::snippet::{Expansion, Placeholder, Template, Whitespace};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct Session {
    markers: Vec<Option<Placeholder>>,
    active: u32,
    edited: bool,
}

impl Session {
    pub(super) fn workspace_metadata(&self) -> Result<usize> {
        anyhow::ensure!(
            self.markers.len() <= 10_000,
            "Workspace snippet marker limit"
        );
        let mut bytes = self.markers.len() * std::mem::size_of::<Option<Placeholder>>();
        for marker in self.markers.iter().flatten() {
            anyhow::ensure!(
                marker.parents.len() <= 64 && marker.choices.len() <= 10_000,
                "Workspace snippet metadata count limit"
            );
            bytes = bytes
                .checked_add(
                    marker.parents.len() * std::mem::size_of::<usize>()
                        + marker.choices.len() * std::mem::size_of::<String>(),
                )
                .context("Workspace snippet payload overflow")?;
            for choice in &marker.choices {
                bytes = bytes
                    .checked_add(choice.len())
                    .context("Workspace choice overflow")?;
            }
            if let Some(transform) = &marker.transform {
                for field in [&transform.pattern, &transform.format, &transform.flags] {
                    bytes = bytes
                        .checked_add(field.len())
                        .context("Workspace transform overflow")?;
                }
            }
            anyhow::ensure!(
                bytes <= 8 * 1024 * 1024,
                "Workspace snippet metadata exceeds budget"
            );
        }
        Ok(bytes)
    }
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct SnippetOptions {
    pub undo_stop_before: bool,
    pub undo_stop_after: bool,
    pub keep_whitespace: bool,
}
impl Default for SnippetOptions {
    fn default() -> Self {
        Self {
            undo_stop_before: true,
            undo_stop_after: true,
            keep_whitespace: false,
        }
    }
}
struct PreparedInsertion {
    changes: Vec<(Range<usize>, String)>,
    markers: Vec<Option<Placeholder>>,
    final_cursors: Vec<Selection>,
}

impl Document {
    /// Validate and expand a completion against the original model, then commit
    /// its primary replacement and imports as one undoable transaction.
    pub(crate) fn apply_completion_edits(
        &mut self,
        primary: Range<usize>,
        mut additional: Vec<(Range<usize>, String)>,
        expand: impl FnOnce(&Document, &Whitespace<'_>) -> Result<Expansion>,
    ) -> Result<()> {
        self.ensure_writable()?;

        if additional.len() >= 4096 || primary.start > primary.end || primary.end > self.len() {
            bail!("Invalid completion range or edit count exceeds 4096");
        }
        let row = self.text.char_to_line(primary.start);
        let leading: String = self
            .text
            .slice(self.line_start(row)..primary.start)
            .chars()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .take(1024 * 1024 + 1)
            .collect();
        let expanded = expand(
            self,
            &Whitespace {
                leading: &leading,
                eol: &self.eol,
                tab_size: self.tab_size,
                insert_spaces: self.insert_spaces,
                fragment_only: false,
            },
        )?;
        let mut bytes = expanded.text.len();
        for (range, text) in &additional {
            if range.start > range.end || range.end > self.len() {
                bail!("Invalid additional completion edit range");
            }
            bytes = bytes.saturating_add(text.len());
        }
        if bytes > 1024 * 1024 || expanded.placeholders.len() > 10_000 {
            bail!("Completion exceeds 1 MiB or 10,000 markers");
        }
        let inserted = expanded.text.chars().count();
        let mut ranges: Vec<_> = additional.iter().map(|(range, _)| range.clone()).collect();
        ranges.push(primary.clone());
        ranges.sort_by_key(|range| (range.start, range.end));
        for pair in ranges.windows(2) {
            if pair[0].end > pair[1].start || pair[0].start == pair[1].start {
                bail!("Completion edits overlap");
            }
        }
        let removed: usize = ranges
            .iter()
            .map(|range| self.text.slice(range.clone()).len_bytes())
            .sum();
        if (self.text.len_bytes() - removed + bytes) as u64 > MAX_FILE_BYTES {
            bail!("Completion exceeds document size limit");
        }
        let mut start = primary.start;
        for (range, text) in &additional {
            if range.end <= primary.start {
                start = start
                    .saturating_add_signed(text.chars().count() as isize - range.len() as isize);
            }
        }
        let markers = expanded
            .placeholders
            .into_iter()
            .map(|mut marker| {
                marker.range.start += start;
                marker.range.end += start;
                Some(marker)
            })
            .collect();
        additional.push((primary, expanded.text));
        let original = self.snapshot();
        self.cancel_snippet();
        self.snippet_generation += 1;
        self.apply_changes(additional);
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
            self.set_selections(vec![Selection::caret(start + inserted)]);
        }
        Ok(())
    }

    pub fn cancel_snippet(&mut self) {
        if self.snippet.take().is_some() {
            self.snippet_generation += 1;
        }
    }

    pub(super) fn cancel_invalid_snippet(&mut self) {
        if !self.in_snippet() {
            self.cancel_snippet();
        }
    }

    pub fn leave_snippet(&mut self) {
        let typing = self.typing;
        self.cancel_snippet();
        let primary = self.selections().remove(0);
        self.set_selections(vec![primary]);
        self.typing = typing.map(|(time, _)| (time, self.cursor));
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
        self.insert_snippet_inner(false, |_, _, _, whitespace| {
            template.expand_with_whitespace(variables, whitespace)
        })
    }

    /// User-command insertion preserves the primary cursor and adjusts nested
    /// template indentation; the pinned extension API has different semantics.
    pub fn insert_snippet_command(
        &mut self,
        template: &Template,
        variables: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.insert_snippet_inner(true, |_, _, _, whitespace| {
            template.expand_with_whitespace(variables, whitespace)
        })
    }

    /// Resolve each variable occurrence against its original insertion cursor.
    /// All resolutions and expansions succeed before any buffer mutation.
    pub fn insert_snippet_command_resolved(
        &mut self,
        template: &Template,
        mut resolve: impl FnMut(
            &Document,
            &Selection,
            usize,
            &str,
            Option<&str>,
        ) -> Result<Option<String>>,
    ) -> Result<()> {
        self.insert_snippet_inner(true, |doc, selection, index, whitespace| {
            template.expand_with_resolver(
                &mut |name: &str, indent: Option<&str>| {
                    resolve(doc, selection, index, name, indent)
                },
                whitespace,
            )
        })
    }

    /// Public array insertion replaces an existing snippet session. Preparation
    /// borrows the original source; no view or history changes precede admission.
    #[allow(clippy::too_many_arguments)] // Separate source, mirror, and selection admission limits.
    pub(crate) fn insert_editor_snippet_resolved(
        &mut self,
        proof: &EditorViewProof,
        template: &Template,
        selections: Vec<Selection>,
        options: SnippetOptions,
        max_bytes: usize,
        max_selections: usize,
        mut resolve: impl FnMut(
            &Document,
            &Selection,
            usize,
            &str,
            Option<&str>,
        ) -> Result<Option<String>>,
    ) -> Result<()> {
        self.ensure_writable()?;
        anyhow::ensure!(
            self.editor_view_current(proof),
            "Snippet editor view retired"
        );
        anyhow::ensure!(
            !selections.is_empty() && selections.len() <= 4096,
            "Snippet selection count exceeds bounds"
        );
        anyhow::ensure!(
            selections
                .iter()
                .all(|s| s.cursor <= self.len() && s.anchor.is_none_or(|a| a <= self.len())),
            "Invalid snippet selection"
        );
        let mut fragments = crate::snippet::ApiFragments::default();
        let PreparedInsertion {
            changes,
            markers,
            final_cursors,
        } = self.prepare_snippet_insertion(
            false,
            selections,
            true,
            max_bytes,
            &mut |doc, selection, index, whitespace| {
                let mut resolver =
                    |name: &str, indent: Option<&str>| resolve(doc, selection, index, name, indent);
                template.expand_api_fragment(
                    &mut resolver,
                    whitespace,
                    options.keep_whitespace,
                    &mut fragments,
                )
            },
        )?;
        let mut session = Session {
            markers,
            active: 0,
            edited: false,
        };
        session.workspace_metadata()?;
        let groups = session.groups();
        let limit = max_selections.min(4096);
        let mut counts = BTreeMap::new();
        for marker in session.markers.iter().flatten() {
            let count = counts.entry(marker.index).or_insert(0usize);
            *count += 1;
            anyhow::ensure!(
                *count <= limit,
                "Snippet placeholder selection budget exceeded"
            );
        }
        let output = if let Some(first) = groups.first() {
            session.active = *first;
            session.selections()
        } else {
            final_cursors
        };
        anyhow::ensure!(
            !output.is_empty() && output.len() <= limit,
            "Snippet output selection budget exceeded"
        );
        self.next_revision
            .checked_add(1)
            .context("Snippet revision exhausted")?;
        self.text_epoch
            .checked_add(changes.len() as u64)
            .context("Snippet text epoch exhausted")?;
        let target = if proof.id == self.active_view {
            &self.view
        } else {
            self.other_views
                .get(&proof.id)
                .context("Snippet editor view retired")?
        };
        anyhow::ensure!(
            target.secondary.len() < 4096 && self.secondary.len() < 4096,
            "Snippet retained view selection budget exceeded"
        );
        let next_generation = target
            .snippet_generation
            .checked_add(1)
            .context("Snippet session generation exhausted")?;
        let mut original = self.snapshot();
        original.view_id = proof.id;
        original.cursor = target.cursor;
        original.anchor = target.anchor;
        original.secondary = target.secondary.clone();
        original.api_owner = target.editor_owner.clone();
        original.snippet = target.snippet.clone();
        original.snippet_generation = target.snippet_generation;
        original.pairs = target.pairs.clone();
        let join = !options.undo_stop_before
            && self.undo.last().is_some_and(|s| {
                !s.api_closed && self.api_history_current(s, &original.api_owner, proof.id)
            });
        let group = if join {
            self.undo
                .last()
                .and_then(|s| s.api_history.clone())
                .unwrap_or_else(|| std::sync::Arc::new(()))
        } else {
            std::sync::Arc::new(())
        };
        if join {
            anyhow::ensure!(
                self.api_group_changes(self.undo.last().and_then(|s| s.api_history.as_ref()))
                    .checked_add(changes.len())
                    .is_some_and(|n| n <= 4096),
                "Snippet Undo group change budget exceeded"
            );
        }
        self.undo
            .try_reserve(1)
            .context("Snippet Undo allocation failed")?;
        original
            .changes
            .try_reserve(changes.len())
            .context("Snippet history allocation failed")?;
        self.byte_changes
            .try_reserve(changes.len().min(257))
            .context("Snippet journal allocation failed")?;
        // Every fallible operation is above this point. The captured retained
        // view is activated without a user navigation/history boundary.
        let changed_source = !changes.is_empty();
        let previous_boundary = self.undo.last().map(|s| (s.api_open, s.api_closed));
        self.display_view(proof.id);
        self.typing = None;
        self.snippet = None;
        self.snippet_generation = next_generation;
        if changed_source {
            if join {
                if let Some(previous) = self.undo.last_mut() {
                    previous.api_history = Some(group.clone());
                }
            } else if let Some(previous) = self.undo.last_mut() {
                previous.api_open = false;
                previous.api_closed = true;
            }
            self.undo.push(original);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.apply_ordered_changes_without_checkpoint(
                changes.into_iter().map(|(r, text)| (r, text, false)),
            );
        }
        self.set_selections(output);
        if !groups.is_empty() && session.active != 0 {
            self.snippet = Some(session);
        }
        if changed_source && let Some(snapshot) = self.undo.last_mut() {
            snapshot.api_history = Some(group);
            snapshot.api_open = !options.undo_stop_after;
            snapshot.api_closed = options.undo_stop_after;
        } else if let Some(snapshot) = self.undo.last_mut() {
            if options.undo_stop_before || options.undo_stop_after {
                snapshot.api_open = false;
                snapshot.api_closed = true;
            } else if let Some((open, closed)) = previous_boundary {
                snapshot.api_open = open;
                snapshot.api_closed = closed;
            }
        }
        Ok(())
    }

    fn insert_snippet_inner(
        &mut self,
        user_command: bool,
        mut expand: impl FnMut(&Document, &Selection, usize, &Whitespace<'_>) -> Result<Expansion>,
    ) -> Result<()> {
        self.ensure_writable()?;

        let PreparedInsertion {
            changes,
            markers,
            final_cursors,
        } = self.prepare_snippet_insertion(
            user_command,
            self.selections(),
            false,
            MAX_FILE_BYTES as usize,
            &mut expand,
        )?;
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

    fn prepare_snippet_insertion(
        &self,
        user_command: bool,
        selections: Vec<Selection>,
        strict: bool,
        maximum: usize,
        expand: &mut impl FnMut(&Document, &Selection, usize, &Whitespace<'_>) -> Result<Expansion>,
    ) -> Result<PreparedInsertion> {
        let mut selections: Vec<_> = selections.into_iter().enumerate().collect();
        selections.sort_by_key(|(_, s)| s.range().start);
        if !user_command && !strict {
            for (index, (ordinal, _)) in selections.iter_mut().enumerate() {
                *ordinal = index;
            }
        }
        // Precompute the maximum admissible expansion from the original
        // non-overlapping ranges, before resolving any variable/template.
        let mut previous_end = 0;
        let mut removed = 0usize;
        for (_, selection) in &selections {
            let range = selection.range();
            anyhow::ensure!(
                range.start >= previous_end && range.end <= self.len(),
                "Overlapping or invalid snippet selections"
            );
            previous_end = range.end;
            removed = removed
                .checked_add(self.text.slice(range).len_bytes())
                .context("Snippet source byte overflow")?;
        }
        let expansion_budget = maximum
            .min(MAX_FILE_BYTES as usize)
            .checked_add(removed)
            .and_then(|n| n.checked_sub(self.text.len_bytes()))
            .context("Snippet final document exceeds byte budget")?
            .min(MAX_FILE_BYTES as usize);
        let mut end = 0;
        let mut shift = 0isize;
        let mut changes = Vec::new();
        let mut marker_groups = vec![Vec::new(); selections.len()];
        let mut final_cursors = vec![Selection::caret(0); selections.len()];
        let mut total_bytes = 0usize;
        let mut total_markers = 0usize;
        let mut model_offsets = Vec::new();
        let mut api_offset_delta = 0isize;
        for (file_index, (ordinal, selection)) in selections.into_iter().enumerate() {
            let output_ordinal = if strict { file_index } else { ordinal };
            let range = selection.range();
            if range.start < end || range.end > self.len() {
                bail!("Overlapping or invalid snippet selections");
            }
            end = range.end;
            let row = self.text.char_to_line(range.start);
            let leading: String = self
                .text
                .slice(self.line_start(row)..range.start)
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t'))
                .take(1024 * 1024 + 1)
                .collect();
            let expanded = expand(
                self,
                &selection,
                ordinal,
                &Whitespace {
                    leading: &leading,
                    eol: &self.eol,
                    tab_size: if strict {
                        self.indent_size()
                    } else {
                        self.tab_size
                    },
                    insert_spaces: self.insert_spaces,
                    fragment_only: !user_command,
                },
            )?;
            total_bytes = total_bytes.saturating_add(expanded.text.len());
            if total_bytes > expansion_budget {
                bail!("Combined snippet expansion exceeds admitted final byte budget");
            }
            let start = range.start.saturating_add_signed(shift);
            let inserted = expanded.text.chars().count();
            let actual_utf16 = expanded.text.encode_utf16().count();
            let raw_utf16 = expanded
                .model_offsets
                .as_ref()
                .map_or(actual_utf16, |offsets| offsets.text_len);
            if expanded.model_offsets.is_some() || api_offset_delta != 0 {
                let offsets = expanded.model_offsets.map_or_else(
                    || {
                        let text = Rope::from_str(&expanded.text);
                        expanded
                            .placeholders
                            .iter()
                            .map(|p| {
                                text.char_to_utf16_cu(p.range.start)
                                    ..text.char_to_utf16_cu(p.range.end)
                            })
                            .collect()
                    },
                    |offsets| offsets.ranges,
                );
                model_offsets.push((output_ordinal, start, api_offset_delta, offsets));
            }
            if !user_command {
                api_offset_delta += raw_utf16 as isize - actual_utf16 as isize;
            }
            for mut marker in expanded.placeholders {
                marker.range.start += start;
                marker.range.end += start;
                marker_groups[output_ordinal].push(marker);
                total_markers += 1;
            }
            if total_markers > 10_000 {
                bail!("Combined snippet expansion exceeds 10,000 markers");
            }
            final_cursors[output_ordinal] = Selection::caret(start + inserted);
            shift += inserted as isize - range.len() as isize;
            if !range.is_empty() || !expanded.text.is_empty() {
                changes.push((range, expanded.text));
            }
        }
        let removed = changes
            .iter()
            .map(|(r, _)| self.text.slice(r.clone()).len_bytes())
            .sum::<usize>();
        let final_bytes = self
            .text
            .len_bytes()
            .checked_sub(removed)
            .and_then(|n| n.checked_add(total_bytes))
            .context("Snippet result byte overflow")?;
        anyhow::ensure!(
            final_bytes <= maximum.min(MAX_FILE_BYTES as usize),
            "Snippet final document exceeds byte budget"
        );
        if !model_offsets.is_empty() {
            // Resolve the reference's post-normalization offsets against the
            // complete staged model, including unchanged text after a snippet.
            // The shared rope keeps this atomic without copying the whole file.
            let mut staged = self.text.clone();
            for (range, text) in changes.iter().rev() {
                staged.remove(range.clone());
                staged.insert(range.start, text);
            }
            for (ordinal, start, delta, offsets) in model_offsets {
                let base = staged.char_to_utf16_cu(start).saturating_add_signed(delta);
                for (marker, range) in marker_groups[ordinal].iter_mut().zip(offsets) {
                    marker.range = model_scalar_position(&staged, base + range.start)
                        ..model_scalar_position(&staged, base + range.end);
                }
            }
        }
        // Public array sessions follow file order. User commands preserve
        // their original primary selection independently of resolver ordinals.
        let mut markers = Vec::with_capacity(total_markers);
        for group in marker_groups {
            let base_id = markers.len();
            for mut marker in group {
                for parent in &mut marker.parents {
                    *parent += base_id;
                }
                markers.push(Some(marker));
            }
        }
        if strict && !markers.is_empty() && !markers.iter().flatten().any(|m| m.index == 0) {
            let end = final_cursors
                .last()
                .context("Snippet requires a target")?
                .cursor;
            anyhow::ensure!(
                markers.len() < 10_000,
                "Combined snippet expansion exceeds 10,000 markers"
            );
            markers.push(Some(Placeholder {
                index: 0,
                range: end..end,
                choices: Vec::new(),
                transform: None,
                parents: Vec::new(),
            }));
        }
        Ok(PreparedInsertion {
            changes,
            markers,
            final_cursors,
        })
    }

    pub fn step_snippet(&mut self, backwards: bool) -> Result<bool> {
        self.ensure_writable()?;

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
        let typing = self.typing;
        self.set_selections(selections);
        // Tab navigation doesn't end the typing undo transaction in VS Code.
        // Retain its age, but move the endpoint to the newly selected field.
        self.typing = typing.map(|(time, _)| (time, self.cursor));
        if next == 0 {
            self.cancel_snippet();
        }
        Ok(true)
    }
}

fn model_scalar_position(text: &Rope, offset: usize) -> usize {
    let offset = offset.min(text.len_utf16_cu());
    let mut scalar = text.utf16_cu_to_char(offset);
    if scalar > 0
        && scalar < text.len_chars()
        && text.char(scalar - 1) == '\r'
        && text.char(scalar) == '\n'
    {
        scalar -= 1;
    } else if text.char_to_utf16_cu(scalar) < offset {
        // The differential observer counts an isolated leading surrogate as
        // one scalar when an upstream position lands inside a surrogate pair.
        scalar += 1;
    }
    scalar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_insertion_saves_crlf_and_restores_both_views_on_undo() {
        let original = "  a\r\n\tb\r\nTAIL";
        for command in [false, true] {
            let mut doc = Document::from_text(original);
            doc.activate_view(2);
            doc.move_to(doc.len(), false);
            doc.activate_view(1);
            doc.set_selections(vec![
                Selection {
                    cursor: 7,
                    anchor: Some(6),
                    desired_column: None,
                },
                Selection {
                    cursor: 3,
                    anchor: Some(2),
                    desired_column: None,
                },
            ]);
            let selections = doc.selections();
            let template = Template::parse("${1:猫\n\t🙂}-$1\n$0").unwrap();
            if command {
                doc.insert_snippet_command(&template, &BTreeMap::new())
                    .unwrap();
            } else {
                doc.insert_snippet(&template, &BTreeMap::new()).unwrap();
            }
            let inserted = doc.text.to_string();
            assert!(!inserted.replace("\r\n", "").contains(['\r', '\n']));
            let after = doc.selections();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("multiline.txt");
            doc.save_to(&path, false).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), inserted);
            doc.undo();
            assert_eq!(doc.text.to_string(), original);
            assert_eq!(doc.selections(), selections);
            doc.activate_view(2);
            assert_eq!(doc.cursor, original.chars().count());
            doc.activate_view(1);
            doc.redo();
            assert_eq!(doc.text.to_string(), inserted);
            assert_eq!(doc.selections(), after);
        }
    }

    #[test]
    fn indentation_amplification_fails_before_mutating_document_or_history() {
        let mut doc = Document::from_text(&" ".repeat(64 * 1024));
        doc.move_to(doc.len(), false);
        doc.insert("x", false);
        let before = doc.text.clone();
        let selections = doc.selections();
        let revision = doc.revision;
        let template = Template::parse(&format!("{}$1", "\n".repeat(32))).unwrap();
        assert!(
            doc.insert_snippet_command(&template, &BTreeMap::new())
                .is_err()
        );
        assert_eq!(doc.text, before);
        assert_eq!(doc.selections(), selections);
        assert_eq!(doc.revision, revision);
        doc.undo();
        assert_eq!(doc.text.to_string(), " ".repeat(64 * 1024));
    }

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

#[cfg(test)]
mod editor_snippet_tests {
    use super::*;

    fn insert(
        doc: &mut Document,
        template: &str,
        selections: Vec<Selection>,
        options: SnippetOptions,
    ) -> Result<()> {
        let proof = doc.editor_view_identity();
        doc.insert_editor_snippet_resolved(
            &proof,
            &Template::parse(template)?,
            selections,
            options,
            4 * 1024 * 1024,
            4096,
            |_, _, _, _, _| Ok(None),
        )
    }

    #[test]
    fn explicit_ranges_link_unicode_fields_and_keep_shared_crlf_views() {
        let mut doc = Document::from_text("cat 猫\r\ncat 🙂\r\n");
        doc.activate_view(1);
        doc.move_to(doc.len(), false);
        let sibling = doc.selections();
        doc.activate_view(2);
        doc.display_view(1);
        let proof = doc.editor_view_identity_for(2).unwrap();
        let second = doc.line_start(1);
        let id = doc.id;
        doc.insert_editor_snippet_resolved(
            &proof,
            &Template::parse("${1:犬}-$1-${2:🙂}$0").unwrap(),
            vec![
                Selection {
                    cursor: 3,
                    anchor: Some(0),
                    desired_column: None,
                },
                Selection {
                    cursor: second,
                    anchor: Some(second + 3),
                    desired_column: None,
                },
            ],
            SnippetOptions::default(),
            4096,
            4096,
            |_, _, _, _, _| Ok(None),
        )
        .unwrap();
        assert_eq!(doc.active_view, 2);
        assert_eq!(doc.id, id);
        assert_eq!(doc.text.to_string(), "犬-犬-🙂 猫\r\n犬-犬-🙂 🙂\r\n");
        assert_eq!(doc.selections().len(), 4);
        doc.insert("x", true);
        assert_eq!(doc.text.to_string(), "x-x-🙂 猫\r\nx-x-🙂 🙂\r\n");
        doc.step_snippet(false).unwrap();
        assert_eq!(doc.selections().len(), 2);
        doc.display_view(1);
        assert!(!doc.in_snippet());
        assert_eq!(doc.cursor, doc.len());
        doc.display_view(2);
        doc.undo();
        doc.undo();
        assert_eq!(doc.text.to_string(), "cat 猫\r\ncat 🙂\r\n");
        doc.display_view(1);
        assert_eq!(doc.selections(), sibling);
        doc.redo();
        doc.redo();
        assert_eq!(doc.text.to_string(), "x-x-🙂 猫\r\nx-x-🙂 🙂\r\n");
    }

    #[test]
    fn keep_whitespace_preserves_fragment_and_model_eol() {
        for (keep, expected) in [
            (false, "  first\r\n    childtail\r\n"),
            (true, "  first\r\n  childtail\r\n"),
        ] {
            let mut doc = Document::from_text("  tail\r\n");
            doc.activate_view(1);
            insert(
                &mut doc,
                "first\n  child$0",
                vec![Selection::caret(2)],
                SnippetOptions {
                    keep_whitespace: keep,
                    ..SnippetOptions::default()
                },
            )
            .unwrap();
            assert_eq!(doc.text.to_string(), expected);
            assert_eq!(doc.cursor, if keep { 16 } else { 18 });
            doc.undo();
            assert_eq!(doc.text.to_string(), "  tail\r\n");
            doc.redo();
            assert_eq!(doc.text.to_string(), expected);
        }
    }

    #[test]
    fn all_undo_stop_combinations_have_explicit_source_owned_boundaries() {
        for before in [false, true] {
            for after in [false, true] {
                let mut doc = Document::from_text("base");
                doc.activate_view(1);
                doc.move_to(4, false);
                doc.insert("P", true);
                insert(
                    &mut doc,
                    "A",
                    vec![Selection::caret(5)],
                    SnippetOptions {
                        undo_stop_before: before,
                        undo_stop_after: after,
                        keep_whitespace: false,
                    },
                )
                .unwrap();
                doc.insert("B", true);
                assert_eq!(doc.text.to_string(), "basePAB");
                assert_eq!(doc.cursor, 7);
                doc.undo();
                let expected = match (before, after) {
                    (_, true) => "basePA",
                    (true, false) => "baseP",
                    (false, false) => "base",
                };
                assert_eq!(doc.text.to_string(), expected);
                doc.redo();
                assert_eq!(doc.text.to_string(), "basePAB");
                assert_eq!(doc.cursor, 7);
            }
        }
    }

    #[test]
    fn adjacent_open_api_group_and_native_typing_redo_restores_actual_final_caret() {
        let mut doc = Document::from_text("  tail\r\n");
        doc.activate_view(1);
        doc.move_to(2, false);
        doc.activate_view(2);
        doc.move_to(doc.len(), false);
        doc.activate_view(1);
        let options = SnippetOptions {
            undo_stop_before: false,
            undo_stop_after: false,
            keep_whitespace: false,
        };
        insert(&mut doc, "A", vec![Selection::caret(2)], options).unwrap();
        insert(&mut doc, "B", vec![Selection::caret(3)], options).unwrap();
        doc.insert("C", true);
        assert_eq!(doc.text.to_string(), "  ABCtail\r\n");
        assert_eq!(doc.cursor, 5);
        doc.display_view(2);
        doc.undo();
        assert_eq!(doc.text.to_string(), "  tail\r\n");
        assert_eq!(doc.active_view, 2);
        assert_eq!(doc.cursor, doc.len());
        doc.redo();
        assert_eq!(doc.text.to_string(), "  ABCtail\r\n");
        assert_eq!(doc.cursor, doc.len());
        doc.display_view(1);
        assert_eq!(doc.cursor, 5);
    }

    #[test]
    fn late_resolution_budget_overlap_and_retired_view_refuse_without_effects() {
        let mut doc = Document::from_text("猫🙂\r\nline\r\n");
        doc.activate_view(1);
        doc.move_to(1, true);
        doc.insert("!", false);
        doc.undo();
        doc.activate_view(2);
        let proof = doc.editor_view_identity_for(1).unwrap();
        let state = (
            doc.text.clone(),
            doc.selections(),
            doc.revision,
            doc.text_epoch,
            doc.save_generation,
            doc.undo.len(),
            doc.redo.len(),
            doc.active_view,
        );
        let selections = vec![Selection::caret(0), Selection::caret(3)];
        assert!(
            doc.insert_editor_snippet_resolved(
                &proof,
                &Template::parse("$VALUE").unwrap(),
                selections.clone(),
                SnippetOptions::default(),
                4096,
                4096,
                |_, _, index, _, _| {
                    if index == 1 {
                        bail!("late failure")
                    }
                    Ok(Some("x".into()))
                }
            )
            .is_err()
        );
        assert!(
            doc.insert_editor_snippet_resolved(
                &proof,
                &Template::parse("longlong").unwrap(),
                selections.clone(),
                SnippetOptions::default(),
                4,
                4096,
                |_, _, _, _, _| Ok(None)
            )
            .is_err()
        );
        assert!(
            doc.insert_editor_snippet_resolved(
                &proof,
                &Template::parse("$1-$1-$2-$2").unwrap(),
                selections,
                SnippetOptions::default(),
                4096,
                2,
                |_, _, _, _, _| Ok(None)
            )
            .is_err()
        );
        assert!(
            doc.insert_editor_snippet_resolved(
                &proof,
                &Template::parse("x").unwrap(),
                vec![
                    Selection {
                        cursor: 3,
                        anchor: Some(0),
                        desired_column: None
                    },
                    Selection {
                        cursor: 4,
                        anchor: Some(2),
                        desired_column: None
                    }
                ],
                SnippetOptions::default(),
                4096,
                4096,
                |_, _, _, _, _| Ok(None)
            )
            .is_err()
        );
        assert_eq!(
            (
                doc.text.clone(),
                doc.selections(),
                doc.revision,
                doc.text_epoch,
                doc.save_generation,
                doc.undo.len(),
                doc.redo.len(),
                doc.active_view
            ),
            state
        );
        doc.redo();
        assert!(doc.text.to_string().contains('!'));
        doc.remove_view(1);
        doc.display_view(1);
        assert!(
            doc.insert_editor_snippet_resolved(
                &proof,
                &Template::parse("x").unwrap(),
                vec![Selection::caret(0)],
                SnippetOptions::default(),
                4096,
                4096,
                |_, _, _, _, _| Ok(None)
            )
            .is_err()
        );
        let mut virtual_doc = Document::from_virtual("readonly:test".into(), "猫").unwrap();
        virtual_doc.activate_view(1);
        assert!(
            insert(
                &mut virtual_doc,
                "x",
                vec![Selection::caret(0)],
                SnippetOptions::default()
            )
            .is_err()
        );
        assert_eq!(virtual_doc.text.to_string(), "猫");
    }

    #[test]
    fn duplicate_empty_insertions_and_original_variable_ordinals_are_admitted() {
        let mut doc = Document::from_text("abcd");
        doc.activate_view(1);
        let proof = doc.editor_view_identity();
        doc.insert_editor_snippet_resolved(
            &proof,
            &Template::parse("$INDEX").unwrap(),
            vec![
                Selection::caret(3),
                Selection::caret(0),
                Selection::caret(0),
            ],
            SnippetOptions::default(),
            4096,
            4096,
            |_, _, index, _, _| Ok(Some(index.to_string())),
        )
        .unwrap();
        assert_eq!(doc.text.to_string(), "12abc0d");
        assert_eq!(
            doc.selections(),
            vec![
                Selection::caret(1),
                Selection::caret(2),
                Selection::caret(6)
            ]
        );
        doc.undo();
        assert_eq!(doc.text.to_string(), "abcd");
    }

    #[test]
    fn reversed_input_resolves_original_indices_but_shares_file_first_defaults() {
        let mut doc = Document::from_text("abcd");
        doc.activate_view(1);
        let proof = doc.editor_view_identity();
        doc.insert_editor_snippet_resolved(
            &proof,
            &Template::parse("${1:$CURSOR_INDEX}-$CURSOR_INDEX").unwrap(),
            vec![Selection::caret(3), Selection::caret(0)],
            SnippetOptions::default(),
            4096,
            4096,
            |_, _, index, _, _| Ok(Some(index.to_string())),
        )
        .unwrap();
        assert_eq!(doc.text.to_string(), "1-1abc1-0d");
        assert_eq!(
            doc.selections(),
            vec![
                Selection {
                    cursor: 1,
                    anchor: Some(0),
                    desired_column: None
                },
                Selection {
                    cursor: 7,
                    anchor: Some(6),
                    desired_column: None
                }
            ]
        );
        doc.step_snippet(false).unwrap();
        // Array fragments form one snippet, with a single implicit final stop.
        assert_eq!(
            doc.selections(),
            vec![Selection {
                cursor: 9,
                anchor: Some(9),
                desired_column: None
            }]
        );
        doc.undo();
        assert_eq!(doc.text.to_string(), "abcd");
        doc.redo();
        assert_eq!(doc.text.to_string(), "1-1abc1-0d");
    }

    #[test]
    fn language_save_and_view_boundaries_do_not_join_prior_history() {
        for boundary in 0..3 {
            let mut doc = Document::from_text("base");
            doc.activate_view(1);
            doc.move_to(4, false);
            let options = SnippetOptions {
                undo_stop_before: false,
                undo_stop_after: false,
                keep_whitespace: false,
            };
            insert(&mut doc, "A", vec![Selection::caret(4)], options).unwrap();
            match boundary {
                0 => {
                    doc.set_language_override(Some("cpp".into())).unwrap();
                }
                1 => {
                    doc.save_generation += 1;
                }
                _ => {
                    doc.activate_view(2);
                    doc.activate_view(1);
                }
            }
            insert(&mut doc, "B", vec![Selection::caret(5)], options).unwrap();
            doc.undo();
            assert_eq!(doc.text.to_string(), "baseA");
            doc.undo();
            assert_eq!(doc.text.to_string(), "base");
        }
    }
}

#[cfg(test)]
mod editor_snippet_admission_tests {
    use super::*;
    fn api(doc: &mut Document, text: &str, options: SnippetOptions) -> Result<()> {
        let proof = doc.editor_view_identity();
        let selections = doc.selections();
        doc.insert_editor_snippet_resolved(
            &proof,
            &Template::parse(text)?,
            selections,
            options,
            4096,
            4096,
            |_, _, _, _, _| Ok(None),
        )
    }
    #[test]
    fn empty_snippet_applies_undo_stops_without_creating_history() {
        for stops in [false, true] {
            let mut doc = Document::from_text("base");
            doc.activate_view(1);
            doc.move_to(4, false);
            let open = SnippetOptions {
                undo_stop_before: false,
                undo_stop_after: false,
                keep_whitespace: false,
            };
            api(&mut doc, "A", open).unwrap();
            let epoch = doc.text_epoch;
            let revision = doc.revision;
            let count = doc.undo.len();
            api(
                &mut doc,
                "",
                SnippetOptions {
                    undo_stop_before: stops,
                    undo_stop_after: stops,
                    keep_whitespace: false,
                },
            )
            .unwrap();
            assert_eq!(
                (doc.text_epoch, doc.revision, doc.undo.len()),
                (epoch, revision, count)
            );
            doc.insert("B", true);
            doc.undo();
            assert_eq!(doc.text.to_string(), if stops { "baseA" } else { "base" });
        }
    }
    #[test]
    fn expanded_byte_admission_stops_before_next_variable_resolution() {
        let mut doc = Document::from_text("abcd");
        doc.activate_view(1);
        let proof = doc.editor_view_identity();
        let original = (
            doc.text.clone(),
            doc.selections(),
            doc.revision,
            doc.text_epoch,
            doc.undo.len(),
        );
        let mut resolved = 0;
        assert!(
            doc.insert_editor_snippet_resolved(
                &proof,
                &Template::parse("$VALUE").unwrap(),
                vec![Selection::caret(0), Selection::caret(4)],
                SnippetOptions::default(),
                5,
                4096,
                |_, _, _, _, _| {
                    resolved += 1;
                    Ok(Some("xx".into()))
                }
            )
            .is_err()
        );
        assert_eq!(resolved, 1);
        assert_eq!(
            (
                doc.text.clone(),
                doc.selections(),
                doc.revision,
                doc.text_epoch,
                doc.undo.len()
            ),
            original
        );
    }
    #[test]
    fn counter_exhaustion_refuses_before_source_view_or_history_publication() {
        for counter in 0..3 {
            let mut doc = Document::from_text("猫\r\n");
            doc.activate_view(1);
            doc.activate_view(2);
            let proof = doc.editor_view_identity_for(1).unwrap();
            match counter {
                0 => doc.next_revision = u64::MAX,
                1 => doc.text_epoch = u64::MAX,
                _ => doc.other_views.get_mut(&1).unwrap().snippet_generation = u64::MAX,
            }
            let original = (
                doc.text.clone(),
                doc.selections(),
                doc.revision,
                doc.text_epoch,
                doc.save_generation,
                doc.active_view,
                doc.undo.len(),
                doc.redo.len(),
            );
            assert!(
                doc.insert_editor_snippet_resolved(
                    &proof,
                    &Template::parse("${1:x}").unwrap(),
                    vec![Selection::caret(0)],
                    SnippetOptions::default(),
                    4096,
                    4096,
                    |_, _, _, _, _| Ok(None)
                )
                .is_err()
            );
            assert_eq!(
                (
                    doc.text.clone(),
                    doc.selections(),
                    doc.revision,
                    doc.text_epoch,
                    doc.save_generation,
                    doc.active_view,
                    doc.undo.len(),
                    doc.redo.len()
                ),
                original
            );
        }
    }
    #[test]
    fn public_array_insert_replaces_previous_session_without_nested_merge() {
        let mut doc = Document::default();
        doc.activate_view(1);
        api(&mut doc, "${1:outer}-${2:end}$0", SnippetOptions::default()).unwrap();
        api(
            &mut doc,
            "${1:inner}-${2:next}$0",
            SnippetOptions::default(),
        )
        .unwrap();
        assert_eq!(doc.text.to_string(), "inner-next-end");
        doc.step_snippet(false).unwrap();
        assert_eq!(doc.selected_text().as_deref(), Some("next"));
        doc.step_snippet(false).unwrap();
        assert_eq!(doc.selection(), None);
    }
}
