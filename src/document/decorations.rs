//! Editor-scoped overlays. Current sets map through real edit streams; Undo
//! never restores an obsolete extension publication from a history snapshot.
use super::*;
use crate::decorations::{
    DecorationInput, DecorationRenderOptions, DecorationType, MAX_MODEL, MAX_MODEL_BYTES, MAX_SET,
    json_budget,
};
use ratatui::style::{Color, Modifier, Style};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
struct Entry {
    // VS Code markers use UTF-16 units, including replacement commonLength.
    // Native scalar ranges are projected from the current Rope metadata.
    units: Range<usize>,
    render: Option<Arc<DecorationRenderOptions>>,
    hover: Option<Arc<serde_json::Value>>,
}
#[derive(Clone, Debug)]
struct Set {
    kind: Arc<DecorationType>,
    entries: Arc<Vec<Entry>>,
    bytes: usize,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Decorations {
    sets: Vec<Set>,
}
#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub start: usize,
    pub deleted: usize,
    pub added: usize,
    pub force_move: bool,
}
impl Edit {
    pub(super) fn inverse(&self) -> Self {
        Self {
            start: self.start,
            deleted: self.added,
            added: self.deleted,
            force_move: false,
        }
    }
}

// Adapted from Microsoft VS Code intervalTree.ts nodeAcceptEdit (MIT).
// Copyright (c) Microsoft Corporation. All rights reserved.
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
// of the Software, and to permit persons to whom the Software is furnished to do
// so, subject to the following conditions: The above copyright notice and this
// permission notice shall be included in all copies or substantial portions of
// the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
// EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO
// EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
// OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
// ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.
fn marker(p: usize, previous: bool, edit: &Edit) -> usize {
    let end = edit.start + edit.deleted;
    let common = edit.deleted.min(edit.added);
    let before =
        |at, stay: Option<bool>| p < at || p == at && !edit.force_move && stay.unwrap_or(previous);
    if before(edit.start, if edit.deleted > 0 { Some(true) } else { None }) {
        return p;
    }
    if common > 0
        && !edit.force_move
        && before(
            edit.start + common,
            if edit.deleted > edit.added {
                Some(true)
            } else {
                None
            },
        )
    {
        return p;
    }
    if before(end, None) {
        return edit.start + edit.added;
    }
    if edit.added >= edit.deleted {
        p.saturating_add(edit.added - edit.deleted)
    } else {
        p.saturating_sub(edit.deleted - edit.added)
    }
}
impl Decorations {
    pub(super) fn map(&mut self, edit: &Edit) {
        self.sets.retain(|set| set.kind.is_live());
        for set in &mut self.sets {
            let behavior = set.kind.options.range_behavior;
            for entry in Arc::make_mut(&mut set.entries) {
                let start = marker(entry.units.start, matches!(behavior, 0 | 2), edit);
                let end = marker(entry.units.end, matches!(behavior, 1 | 2), edit);
                entry.units = start..end.max(start);
            }
        }
    }
    fn count(&self) -> usize {
        self.sets
            .iter()
            .filter(|s| s.kind.is_live())
            .map(|s| s.entries.len())
            .sum()
    }
    fn bytes(&self) -> usize {
        self.sets
            .iter()
            .filter(|s| s.kind.is_live())
            .map(|s| s.bytes)
            .sum()
    }
}
impl Document {
    pub(crate) fn apply_editor_decorations(
        &mut self,
        proof: &EditorViewProof,
        kind: Arc<DecorationType>,
        entries: Vec<DecorationInput>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.editor_view_current(proof) && kind.is_live(),
            "Decoration editor or type retired"
        );
        anyhow::ensure!(
            entries.len() <= MAX_SET,
            "Decoration set exceeds 4096 ranges"
        );
        let target = if proof.id == self.active_view {
            &self.view
        } else {
            self.other_views.get(&proof.id).unwrap()
        };
        let replaced = target
            .decorations
            .sets
            .iter()
            .find(|s| Arc::ptr_eq(&s.kind, &kind));
        let count = self
            .editor_decoration_count()
            .saturating_sub(replaced.map_or(0, |s| s.entries.len()));
        anyhow::ensure!(
            count + entries.len() <= MAX_MODEL,
            "Document decorations exceed 16384 ranges"
        );
        let existing = std::iter::once(&self.view)
            .chain(self.other_views.values())
            .map(|v| v.decorations.bytes())
            .sum::<usize>()
            .saturating_sub(replaced.map_or(0, |s| s.bytes));
        anyhow::ensure!(
            entries.len() * std::mem::size_of::<Entry>()
                <= (2 * 1024 * 1024usize).min(MAX_MODEL_BYTES.saturating_sub(existing)),
            "Decoration metadata reservation exceeds bounds"
        );
        let mut prepared = Vec::new();
        prepared.try_reserve_exact(entries.len())?;
        let mut bytes = prepared.capacity() * std::mem::size_of::<Entry>();
        for input in entries {
            let start = clamp(self, input.range.start);
            let end = clamp(self, input.range.end);
            let range = start.min(end)..start.max(end);
            let render = match input.render_options {
                Some(raw) => {
                    bytes = bytes.saturating_add(json_budget(&raw, 16 * 1024)?);
                    Some(Arc::new(DecorationRenderOptions::from_wire(raw)?))
                }
                None => None,
            };
            let hover = match input.hover_message {
                Some(raw) => {
                    bytes = bytes.saturating_add(json_budget(&raw, 16 * 1024)?);
                    Some(Arc::new(raw))
                }
                None => None,
            };
            anyhow::ensure!(
                bytes <= 2 * 1024 * 1024 && existing + bytes <= MAX_MODEL_BYTES,
                "Decoration metadata budget exceeded"
            );
            prepared.push(Entry {
                units: range,
                render,
                hover,
            });
        }
        self.remove_decoration_types(&[]);
        let view = if proof.id == self.active_view {
            &mut self.view
        } else {
            self.other_views.get_mut(&proof.id).unwrap()
        };
        view.decorations.sets.retain(|s| s.kind.is_live());
        if let Some(i) = view
            .decorations
            .sets
            .iter()
            .position(|s| Arc::ptr_eq(&s.kind, &kind))
        {
            if prepared.is_empty() {
                view.decorations.sets.remove(i);
            } else {
                view.decorations.sets[i] = Set {
                    kind,
                    entries: Arc::new(prepared),
                    bytes,
                };
            }
        } else if !prepared.is_empty() {
            view.decorations.sets.try_reserve(1)?;
            view.decorations.sets.push(Set {
                kind,
                entries: Arc::new(prepared),
                bytes,
            });
        }
        Ok(())
    }
    pub(crate) fn editor_decoration_count(&self) -> usize {
        std::iter::once(&self.view)
            .chain(self.other_views.values())
            .map(|v| v.decorations.count())
            .sum()
    }
    pub(crate) fn clear_editor_decorations_for(&mut self, view: u64) {
        if view == self.active_view {
            self.view.decorations = Decorations::default();
        } else if let Some(v) = self.other_views.get_mut(&view) {
            v.decorations = Decorations::default();
        }
    }
    pub(crate) fn remove_decoration_types(&mut self, types: &[Arc<DecorationType>]) {
        for view in std::iter::once(&mut self.view).chain(self.other_views.values_mut()) {
            view.decorations.sets.retain(|s| {
                s.kind.is_live() && !types.iter().any(|kind| Arc::ptr_eq(&s.kind, kind))
            });
        }
    }
    pub(crate) fn decoration_overlay(
        &self,
        range: Range<usize>,
        theme: &crate::theme::Theme,
    ) -> Overlay {
        let mut events = Vec::new();
        let mut whole_events = Vec::new();
        let mut priority = 0;
        for set in &self.view.decorations.sets {
            if !set.kind.is_live() {
                continue;
            }
            for entry in set.entries.iter() {
                priority += 1;
                let _retained_hover = &entry.hover;
                let mut span = self
                    .text
                    .utf16_cu_to_char(entry.units.start.min(self.text.len_utf16_cu()))
                    ..self
                        .text
                        .utf16_cu_to_char(entry.units.end.min(self.text.len_utf16_cu()));
                let whole = entry
                    .render
                    .as_ref()
                    .map_or(set.kind.options.whole_line(), |o| {
                        o.raw
                            .get("isWholeLine")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(set.kind.options.whole_line())
                    });
                if whole {
                    let first = self.text.char_to_line(span.start);
                    let last = self.text.char_to_line(span.end);
                    span = self.line_start(first)..if last + 1 < self.line_count() {
                        self.line_start(last + 1)
                    } else {
                        self.len() + 1
                    };
                }
                span = span.start.max(range.start)..span.end.min(range.end);
                if span.is_empty() {
                    continue;
                }
                let mut style = set.kind.options.style(theme);
                if let Some(options) = &entry.render {
                    style = style.patch(options.style(theme));
                }
                if whole {
                    whole_events.push(Event {
                        at: span.start,
                        priority,
                        style,
                        add: true,
                    });
                    whole_events.push(Event {
                        at: span.end,
                        priority,
                        style,
                        add: false,
                    });
                }
                events.push(Event {
                    at: span.start,
                    priority,
                    style,
                    add: true,
                });
                events.push(Event {
                    at: span.end,
                    priority,
                    style,
                    add: false,
                });
            }
        }
        Overlay {
            runs: project(events, range.start),
            whole_runs: project(whole_events, range.start),
        }
    }
}
fn clamp(doc: &Document, position: crate::lsp::Position) -> usize {
    if position.line >= doc.line_count() {
        return doc.text.len_utf16_cu();
    }
    let start = doc.line_start(position.line);
    let end = doc.line_end(position.line);
    doc.text.char_to_utf16_cu(start)
        + position
            .character
            .min(doc.text.char_to_utf16_cu(end) - doc.text.char_to_utf16_cu(start))
}
struct Event {
    at: usize,
    priority: usize,
    style: Style,
    add: bool,
}
struct Run {
    range: Range<usize>,
    style: Style,
}
fn project(mut events: Vec<Event>, start: usize) -> Vec<Run> {
    events.sort_unstable_by_key(|e| (e.at, e.add, e.priority));
    let mut active = Active::default();
    let mut runs = Vec::new();
    let mut previous = start;
    for event in events {
        if event.at > previous {
            let style = active.style();
            if style != Style::default() {
                runs.push(Run {
                    range: previous..event.at,
                    style,
                });
            }
            previous = event.at;
        }
        active.update(&event);
    }
    runs
}
pub(crate) struct Overlay {
    runs: Vec<Run>,
    whole_runs: Vec<Run>,
}
impl Overlay {
    pub(crate) fn style_at(&self, position: usize) -> Style {
        lookup(&self.runs, position)
    }
    pub(crate) fn whole_style_at(&self, position: usize) -> Style {
        lookup(&self.whole_runs, position)
    }
}
fn lookup(runs: &[Run], position: usize) -> Style {
    let i = runs.partition_point(|r| r.range.end <= position);
    runs.get(i)
        .filter(|r| r.range.contains(&position))
        .map_or(Style::default(), |r| r.style)
}
#[derive(Default)]
struct Active {
    fg: BTreeMap<usize, Color>,
    bg: BTreeMap<usize, Color>,
    bold: BTreeMap<usize, bool>,
    italic: BTreeMap<usize, bool>,
    underline: BTreeMap<usize, bool>,
    strike: BTreeMap<usize, bool>,
}
impl Active {
    fn update(&mut self, e: &Event) {
        if let Some(c) = e.style.fg {
            if e.add {
                self.fg.insert(e.priority, c);
            } else {
                self.fg.remove(&e.priority);
            }
        }
        if let Some(c) = e.style.bg {
            if e.add {
                self.bg.insert(e.priority, c);
            } else {
                self.bg.remove(&e.priority);
            }
        }
        for (map, m) in [
            (&mut self.bold, Modifier::BOLD),
            (&mut self.italic, Modifier::ITALIC),
            (&mut self.underline, Modifier::UNDERLINED),
            (&mut self.strike, Modifier::CROSSED_OUT),
        ] {
            if e.style.add_modifier.contains(m) || e.style.sub_modifier.contains(m) {
                if e.add {
                    map.insert(e.priority, e.style.add_modifier.contains(m));
                } else {
                    map.remove(&e.priority);
                }
            }
        }
    }
    fn style(&self) -> Style {
        let mut s = Style::default();
        if let Some((_, c)) = self.fg.last_key_value() {
            s = s.fg(*c);
        }
        if let Some((_, c)) = self.bg.last_key_value() {
            s = s.bg(*c);
        }
        for (map, m) in [
            (&self.bold, Modifier::BOLD),
            (&self.italic, Modifier::ITALIC),
            (&self.underline, Modifier::UNDERLINED),
            (&self.strike, Modifier::CROSSED_OUT),
        ] {
            if let Some((_, v)) = map.last_key_value() {
                s = if *v {
                    s.add_modifier(m)
                } else {
                    s.remove_modifier(m)
                };
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::Ordering;
    fn kind(behavior: u8) -> Arc<DecorationType> {
        DecorationType::new(
            format!("decoration-{behavior}"),
            "fixture.owner".into(),
            json!({"rangeBehavior":behavior,"color":"#ff0000"}),
        )
        .unwrap()
    }
    fn input(start: usize, end: usize) -> DecorationInput {
        serde_json::from_value(json!({"range":{"start":{"line":0,"character":start},"end":{"line":0,"character":end}}})).unwrap()
    }
    fn ranges(doc: &Document, view: u64) -> Vec<Range<usize>> {
        let view = doc.view_state(Some(view));
        view.decorations
            .sets
            .iter()
            .flat_map(|s| s.entries.iter().map(|e| e.units.clone()))
            .collect()
    }
    fn source(doc: &Document) -> (u64, String, u64, u64, u64, u64, bool, Vec<Selection>) {
        (
            doc.id,
            doc.text.to_string(),
            doc.revision,
            doc.text_epoch(),
            doc.saved_revision,
            doc.save_generation(),
            doc.dirty(),
            doc.selections(),
        )
    }
    #[test]
    fn all_four_endpoint_behaviors_and_empty_ranges_use_original_marker_semantics() {
        for (position, expected) in [(1, [1..4, 2..4, 1..4, 2..4]), (3, [1..4, 1..3, 1..3, 1..4])] {
            for (behavior, expected) in expected.into_iter().enumerate() {
                let mut doc = Document::from_text("abcd");
                let proof = doc.editor_view_identity();
                doc.apply_editor_decorations(&proof, kind(behavior as u8), vec![input(1, 3)])
                    .unwrap();
                doc.move_to(position, false);
                doc.insert("X", false);
                assert_eq!(
                    ranges(&doc, 0),
                    vec![expected],
                    "behavior {behavior}, insertion {position}"
                );
                doc.undo();
                assert_eq!(doc.text.to_string(), "abcd");
                assert_eq!(ranges(&doc, 0), vec![1..3]);
                doc.redo();
                assert_eq!(
                    doc.text.to_string(),
                    format!("{}X{}", &"abcd"[..position], &"abcd"[position..])
                );
            }
        }
        for (behavior, expected) in [2..3, 3..3, 2..2, 3..3].into_iter().enumerate() {
            let mut doc = Document::from_text("abcd");
            let proof = doc.editor_view_identity();
            doc.apply_editor_decorations(&proof, kind(behavior as u8), vec![input(2, 2)])
                .unwrap();
            doc.move_to(2, false);
            doc.insert("x", false);
            assert_eq!(ranges(&doc, 0), vec![expected]);
        }
    }
    #[test]
    fn replacement_common_length_is_utf16_and_current_api_sets_survive_history() {
        let mut doc = Document::from_text("a😀b\r\n猫");
        doc.activate_view(11);
        let proof = doc.editor_view_identity();
        doc.apply_editor_decorations(&proof, kind(0), vec![input(3, 4)])
            .unwrap();
        doc.apply_changes(vec![(1..2, "XY".into())]);
        assert_eq!(doc.text.to_string(), "aXYb\r\n猫");
        assert_eq!(ranges(&doc, 11), vec![3..4]);
        // A new publication while an older edit is in history must remain the
        // authority after Undo; the old range must not be resurrected.
        doc.apply_editor_decorations(&proof, kind(1), vec![input(0, 1)])
            .unwrap();
        doc.undo();
        assert_eq!(doc.text.to_string(), "a😀b\r\n猫");
        assert_eq!(ranges(&doc, 11), vec![3..4, 0..1]);
        doc.redo();
        assert_eq!(ranges(&doc, 11), vec![3..4, 0..1]);
        for behavior in 0..4 {
            let mut doc = Document::from_text("abcd");
            let proof = doc.editor_view_identity();
            doc.apply_editor_decorations(&proof, kind(behavior), vec![input(1, 3)])
                .unwrap();
            doc.apply_changes(vec![(0..2, "Q".into())]);
            assert_eq!(ranges(&doc, 0), vec![1..2]);
            doc.undo();
            // Marker mapping is not an inverse restoration of lost ranges.
            // The longer inverse replacement retains only open start markers
            // at commonLength; closed starts follow the inserted suffix.
            let restored = if matches!(behavior, 0 | 2) {
                1..3
            } else {
                2..3
            };
            assert_eq!(ranges(&doc, 0), vec![restored]);
        }
    }
    #[test]
    fn clamps_utf16_ranges_and_inactive_sets_without_touching_source_or_undo() {
        let mut doc = Document::from_text("α😀\r\n猫");
        doc.activate_view(11);
        doc.insert("!", false);
        doc.undo();
        let proof = doc.editor_view_identity();
        doc.activate_view(12);
        doc.move_to(4, false);
        let before = source(&doc);
        let k = kind(0);
        doc.apply_editor_decorations(&proof, k.clone(), vec![input(2, 99)])
            .unwrap();
        assert_eq!(ranges(&doc, 11), vec![2..3]);
        assert!(ranges(&doc, 12).is_empty());
        assert_eq!(source(&doc), before);
        doc.display_view(11);
        assert_eq!(
            doc.decoration_overlay(0..doc.len(), &crate::theme::Theme::default())
                .style_at(1)
                .fg,
            Some(Color::Rgb(255, 0, 0))
        );
        doc.apply_editor_decorations(&proof, k, vec![]).unwrap();
        assert!(ranges(&doc, 11).is_empty());
        doc.redo();
        assert_eq!(doc.text.to_string(), "!α😀\r\n猫");
        doc.undo();
        assert_eq!(doc.text.to_string(), before.1);
    }
    #[test]
    fn retired_type_and_recreated_editor_cannot_restore_overlay_and_refusals_are_atomic() {
        let mut doc = Document::from_text("abc");
        doc.activate_view(11);
        let proof = doc.editor_view_identity();
        let k = kind(0);
        doc.apply_editor_decorations(&proof, k.clone(), vec![input(0, 2)])
            .unwrap();
        let before = source(&doc);
        assert!(
            doc.apply_editor_decorations(&proof, k.clone(), vec![input(0, 1); MAX_SET + 1])
                .is_err()
        );
        assert_eq!(ranges(&doc, 11), vec![0..2]);
        assert_eq!(source(&doc), before);
        k.live.store(false, Ordering::Release);
        assert_eq!(
            doc.decoration_overlay(0..3, &crate::theme::Theme::default())
                .style_at(0),
            Style::default()
        );
        assert!(doc.apply_editor_decorations(&proof, k, vec![]).is_err());
        doc.activate_view(12);
        doc.remove_view(11);
        doc.activate_view(11);
        assert!(
            doc.apply_editor_decorations(&proof, kind(0), vec![input(0, 2)])
                .is_err()
        );
        assert!(ranges(&doc, 11).is_empty());
        assert_eq!(doc.text.to_string(), before.1);
    }
    #[test]
    fn model_range_and_json_budgets_reject_late_without_replacing_live_sets() {
        let mut doc = Document::from_text("abc");
        let proof = doc.editor_view_identity();
        for i in 0..4 {
            let k = DecorationType::new(format!("decoration-{i}"), "fixture".into(), json!({}))
                .unwrap();
            doc.apply_editor_decorations(&proof, k, vec![input(0, 1); MAX_SET])
                .unwrap();
        }
        let before = source(&doc);
        assert_eq!(doc.editor_decoration_count(), MAX_MODEL);
        assert!(
            doc.apply_editor_decorations(&proof, kind(0), vec![input(1, 2)])
                .is_err()
        );
        assert_eq!(doc.editor_decoration_count(), MAX_MODEL);
        assert_eq!(source(&doc), before);
        doc.clear_editor_decorations_for(0);
        assert_eq!(doc.editor_decoration_count(), 0);
        let mut invalid = input(1, 2);
        invalid.render_options = Some(json!({"color":true}));
        assert!(
            doc.apply_editor_decorations(&proof, kind(0), vec![input(0, 1), invalid])
                .is_err()
        );
        assert_eq!(doc.editor_decoration_count(), 0);
        assert_eq!(source(&doc), before);
    }
    #[test]
    fn relaxed_decoration_admission_preserves_surrogate_interior_and_clamped_utf16_units() {
        let mut doc = Document::from_text("α😀b\r\n猫");
        let proof = doc.editor_view_identity();
        doc.apply_editor_decorations(
            &proof,
            kind(0),
            vec![input(2, 2), input(0, 2), input(2, 99)],
        )
        .unwrap();
        // Decorations use relaxed validation unlike edits: even collapsed
        // markers inside a surrogate remain exact UTF16 positions.
        assert_eq!(ranges(&doc, 0), vec![2..2, 0..2, 2..4]);
        doc.apply_changes(vec![(1..2, "XY".into())]);
        assert_eq!(ranges(&doc, 0), vec![2..2, 0..2, 2..4]);
        doc.undo();
        assert_eq!(doc.text.to_string(), "α😀b\r\n猫");
        assert_eq!(ranges(&doc, 0), vec![2..2, 0..2, 2..4]);
        doc.apply_editor_decorations(&proof, kind(1), vec![serde_json::from_value(json!({"range":{"start":{"line":usize::MAX,"character":usize::MAX},"end":{"line":0,"character":2}}})).unwrap()]).unwrap();
        assert_eq!(ranges(&doc, 0).last(), Some(&(2..7)));
    }
    #[test]
    fn forced_insert_delete_replacement_uses_explicit_flags_once_and_undo_is_marker_defined() {
        for behavior in 0..4 {
            for (position, expected) in [(1, 2..4), (3, 1..4)] {
                let mut doc = Document::from_text("abcd");
                let proof = doc.editor_view_identity();
                doc.apply_editor_decorations(&proof, kind(behavior), vec![input(1, 3)])
                    .unwrap();
                doc.apply_changes_with_marker_semantics(vec![(
                    position..position,
                    "X".into(),
                    true,
                )]);
                assert_eq!(ranges(&doc, 0), vec![expected]);
                doc.undo();
                assert_eq!(doc.text.to_string(), "abcd");
                assert_eq!(ranges(&doc, 0), vec![1..3]);
                doc.redo();
                assert_eq!(
                    doc.text.to_string(),
                    format!("{}X{}", &"abcd"[..position], &"abcd"[position..])
                );
            }
            let mut doc = Document::from_text("abcd");
            let proof = doc.editor_view_identity();
            doc.apply_editor_decorations(&proof, kind(behavior), vec![input(1, 3)])
                .unwrap();
            doc.apply_changes_with_marker_semantics(vec![(0..4, "X".into(), true)]);
            assert_eq!(ranges(&doc, 0), vec![1..1]);
            doc.undo();
            assert_eq!(doc.text.to_string(), "abcd");
            let expected = [1..4, 4..4, 1..1, 4..4][behavior as usize].clone();
            assert_eq!(ranges(&doc, 0), vec![expected]);
            doc.apply_changes_with_marker_semantics(vec![(0..4, String::new(), true)]);
            assert_eq!(doc.text.to_string(), "");
            assert_eq!(ranges(&doc, 0), vec![0..0]);
            doc.undo();
            assert_eq!(doc.text.to_string(), "abcd");
        }
    }
    #[test]
    fn ordinary_and_explicit_view_copies_never_inherit_editor_scoped_registrations() {
        let mut doc = Document::from_text("abc");
        doc.activate_view(11);
        let proof = doc.editor_view_identity();
        doc.apply_editor_decorations(&proof, kind(0), vec![input(0, 2)])
            .unwrap();
        let before = source(&doc);
        doc.prepare_folding_view(12, Some(11)).unwrap().publish();
        doc.display_view(12);
        assert!(ranges(&doc, 12).is_empty());
        doc.activate_view(13);
        assert!(ranges(&doc, 13).is_empty());
        doc.display_view(11);
        assert_eq!(ranges(&doc, 11), vec![0..2]);
        assert_eq!(source(&doc), before);
    }

    #[test]
    fn whole_line_and_overlapping_field_priority_project_without_per_cell_scans() {
        let mut doc = Document::from_text("abc\r\n猫");
        let proof = doc.editor_view_identity();
        let whole = DecorationType::new(
            "decoration-whole".into(),
            "fixture".into(),
            json!({"isWholeLine":true,"backgroundColor":"#123456","fontWeight":"bold"}),
        )
        .unwrap();
        doc.apply_editor_decorations(&proof, whole, vec![input(1, 2)])
            .unwrap();
        let mut child = input(1, 2);
        child.render_options = Some(
            json!({"fontWeight":"normal","fontStyle":"italic","textDecoration":"underline line-through"}),
        );
        doc.apply_editor_decorations(&proof, kind(0), vec![child])
            .unwrap();
        let o = doc.decoration_overlay(0..doc.len() + 1, &crate::theme::Theme::default());
        assert_eq!(o.whole_style_at(0).bg, Some(Color::Rgb(0x12, 0x34, 0x56)));
        assert_eq!(o.style_at(1).fg, Some(Color::Rgb(255, 0, 0)));
        assert!(o.style_at(1).sub_modifier.contains(Modifier::BOLD));
        assert!(
            o.style_at(1)
                .add_modifier
                .contains(Modifier::ITALIC | Modifier::UNDERLINED | Modifier::CROSSED_OUT)
        );
        assert!(o.style_at(2).add_modifier.contains(Modifier::BOLD));
        assert_eq!(o.style_at(5), Style::default());
    }
}
