use super::*;

impl Document {
    pub fn selections(&self) -> Vec<Selection> {
        let mut selections = vec![Selection {
            cursor: self.cursor,
            anchor: self.anchor,
            desired_column: self.desired_column,
        }];
        selections.extend(self.secondary.clone());
        selections
    }
    pub(super) fn assign_selections(&mut self, mut selections: Vec<Selection>) {
        if selections.is_empty() {
            selections.push(Selection::caret(0));
        }
        let first = selections.remove(0);
        self.cursor = first.cursor.min(self.len());
        self.anchor = first.anchor.map(|p| p.min(self.len()));
        self.desired_column = first.desired_column;
        self.secondary = selections;
    }
    pub fn set_selections(&mut self, selections: Vec<Selection>) {
        self.break_group();
        self.assign_selections(selections);
        self.normalize_selections();
        self.retire_outside_typing_pairs();
    }
    fn record_cursors(&mut self) {
        let selections = self.selections();
        self.cursor_history.push(selections);
        if self.cursor_history.len() > 100 {
            self.cursor_history.remove(0);
        }
        self.break_group();
    }
    pub fn undo_cursor(&mut self) {
        if let Some(selections) = self.cursor_history.pop() {
            self.set_selections(selections);
        }
    }
    pub fn clear_secondary(&mut self) {
        self.secondary.clear();
        self.anchor = None;
        self.break_group();
    }
    pub fn normalize_selections(&mut self) {
        let mut selections: Vec<_> = self
            .selections()
            .into_iter()
            .enumerate()
            .map(|(i, mut s)| {
                s.cursor = s.cursor.min(self.len());
                s.anchor = s.anchor.map(|p| p.min(self.len()));
                (s, i)
            })
            .collect();
        selections.sort_by_key(|(s, _)| (s.range().start, s.range().end));
        let mut merged: Vec<(Selection, usize)> = Vec::new();
        for (s, order) in selections {
            if let Some((last, last_order)) = merged.last_mut() {
                let a = last.range();
                let b = s.range();
                if b.start < a.end || (b.start == a.end && (a.is_empty() || b.is_empty())) {
                    let forward = if order < *last_order {
                        s.cursor >= s.anchor.unwrap_or(s.cursor)
                    } else {
                        last.cursor >= last.anchor.unwrap_or(last.cursor)
                    };
                    let start = a.start.min(b.start);
                    let end = a.end.max(b.end);
                    *last = Selection {
                        cursor: if forward { end } else { start },
                        anchor: if start == end {
                            None
                        } else {
                            Some(if forward { start } else { end })
                        },
                        desired_column: None,
                    };
                    *last_order = (*last_order).min(order);
                    continue;
                }
            }
            merged.push((s, order));
        }
        // Sorting is needed to merge overlapping ranges, but selection identity
        // follows caller order (including secondaries), not file position.
        merged.sort_by_key(|(_, order)| *order);
        self.assign_selections(merged.into_iter().map(|(s, _)| s).collect());
        self.cancel_invalid_snippet();
    }
    pub fn replace_cursors(&mut self, text: impl Fn(&Self, &Selection) -> String) {
        self.normalize_selections();
        let replacements: Vec<_> = self
            .selections()
            .iter()
            .map(|s| (s.range(), text(self, s)))
            .collect();
        // Each caret belongs to its own replacement. Generic right-biased mapping
        // would attach a boundary caret to the next adjacent selection's edit.
        let mut order: Vec<_> = (0..replacements.len()).collect();
        order.sort_by_key(|i| replacements[*i].0.start);
        let mut positions = vec![0; replacements.len()];
        let mut shift = 0isize;
        for index in order {
            let (range, inserted) = &replacements[index];
            let added = inserted.chars().count();
            positions[index] = range.start.saturating_add_signed(shift) + added;
            shift += added as isize - (range.end - range.start) as isize;
        }
        let cursors = positions.into_iter().map(Selection::caret).collect();
        let original = self.snapshot();
        let revision = self.revision;
        // All cursor positions were computed together; avoid quadratic remapping.
        self.secondary.clear();
        self.apply_changes(
            replacements
                .into_iter()
                .filter(|(r, s)| !r.is_empty() || !s.is_empty())
                .collect(),
        );
        if self.revision != revision {
            self.replace_undo_snapshot(original);
        }
        self.assign_selections(cursors);
        self.normalize_selections();
    }
    pub(super) fn delete_cursors(&mut self, right: bool, word: bool) {
        let original = self.snapshot();
        let selections = self.selections();
        let mut expanded = Vec::new();
        for mut selection in selections {
            if selection.range().is_empty() {
                self.cursor = selection.cursor;
                let end = if right {
                    if word {
                        self.word_right()
                    } else {
                        self.next(self.cursor)
                    }
                } else if word {
                    self.word_left()
                } else {
                    self.previous(self.cursor)
                };
                selection.anchor = Some(end);
            }
            expanded.push(selection);
        }
        self.assign_selections(expanded);
        self.normalize_selections();
        let old_revision = self.revision;
        self.replace_cursors(|_, _| String::new());
        if self.revision != old_revision {
            self.replace_undo_snapshot(original);
        }
    }
    pub fn navigate_cursors(&mut self, command: &str, select: bool, page: isize) -> bool {
        if !matches!(
            command,
            "cursorLeft"
                | "cursorRight"
                | "cursorWordLeft"
                | "cursorWordRight"
                | "cursorUp"
                | "cursorDown"
                | "cursorPageUp"
                | "cursorPageDown"
                | "cursorHome"
                | "cursorEnd"
                | "cursorTop"
                | "cursorBottom"
        ) {
            return false;
        }
        self.record_cursors();
        let selections = self.selections();
        self.secondary.clear();
        let mut moved = Vec::new();
        for selection in selections {
            self.cursor = selection.cursor;
            self.anchor = selection.anchor;
            self.desired_column = selection.desired_column;
            match command {
                "cursorLeft" => self.horizontal_inner(false, select, false),
                "cursorRight" => self.horizontal_inner(true, select, false),
                "cursorWordLeft" => self.horizontal_inner(false, select, true),
                "cursorWordRight" => self.horizontal_inner(true, select, true),
                "cursorUp" => self.vertical_inner(-1, select),
                "cursorDown" => self.vertical_inner(1, select),
                "cursorPageUp" => self.vertical_inner(-page, select),
                "cursorPageDown" => self.vertical_inner(page, select),
                "cursorHome" => self.home_inner(select),
                "cursorEnd" => self.move_to_inner(self.line_end(self.row()), select),
                "cursorTop" => self.move_to_inner(0, select),
                "cursorBottom" => self.move_to_inner(self.len(), select),
                _ => return false,
            }
            moved.push(Selection {
                cursor: self.cursor,
                anchor: self.anchor,
                desired_column: self.desired_column,
            });
        }
        self.assign_selections(moved);
        self.normalize_selections();
        // Intermediate primary positions are not the completed multi-cursor
        // movement. Retire ownership only after every caret has been moved.
        self.retire_outside_typing_pairs();
        true
    }
    pub fn add_cursor(&mut self, pos: usize) {
        if self.secondary.len() + 1 >= 10_000 {
            return;
        }
        self.record_cursors();
        let mut selections = self.selections();
        selections.insert(0, Selection::caret(pos.min(self.len())));
        self.set_selections(selections);
    }
    pub fn add_cursor_vertical(&mut self, down: bool) {
        let row = self.row();
        let target = if down { row + 1 } else { row.saturating_sub(1) };
        if target >= self.line_count() || target == row {
            return;
        }
        self.add_cursor(self.position_at(target, self.visual_column()));
    }
    fn word_range_at(&self, pos: usize) -> Range<usize> {
        let mut start = pos;
        let mut end = pos;
        while start > 0 && is_word(self.text.char(start - 1)) {
            start = self.previous(start);
        }
        while end < self.len() && is_word(self.text.char(end)) {
            end = self.next(end);
        }
        start..end
    }
    pub fn select_next_occurrence(&mut self, all: bool) -> bool {
        self.record_cursors();
        let range = match self.selection() {
            Some(r) => r,
            None => {
                let r = self.word_range_at(self.cursor);
                if r.is_empty() {
                    return false;
                }
                self.anchor = Some(r.start);
                self.cursor = r.end;
                if !all {
                    return true;
                }
                r
            }
        };
        let needle = self.text.slice(range.clone()).to_string();
        let text = self.text.to_string();
        let existing: Vec<_> = self.selections().iter().map(Selection::range).collect();
        if all {
            let mut matches: Vec<_> = text
                .match_indices(&needle)
                .take(10_001)
                .map(|(b, _)| self.text.byte_to_char(b)..self.text.byte_to_char(b + needle.len()))
                .collect();
            if matches.len() > 10_000 {
                return false;
            }
            let primary = matches.iter().position(|r| *r == range).unwrap_or(0);
            matches.rotate_left(primary);
            self.set_selections(
                matches
                    .into_iter()
                    .map(|r| Selection {
                        cursor: r.end,
                        anchor: Some(r.start),
                        desired_column: None,
                    })
                    .collect(),
            );
            return true;
        }
        if existing.len() >= 10_000 {
            return false;
        }
        let start = self.text.char_to_byte(range.end);
        let mut matches = text[start..]
            .match_indices(&needle)
            .map(|(b, _)| b + start)
            .chain(text[..start].match_indices(&needle).map(|(b, _)| b))
            .map(|b| self.text.byte_to_char(b)..self.text.byte_to_char(b + needle.len()));
        if let Some(next) =
            matches.find(|r| !existing.iter().any(|e| e.start < r.end && r.start < e.end))
        {
            let mut selections = self.selections();
            selections.insert(
                0,
                Selection {
                    cursor: next.end,
                    anchor: Some(next.start),
                    desired_column: None,
                },
            );
            self.set_selections(selections);
            true
        } else {
            false
        }
    }
    pub fn cursors_at_line_ends(&mut self) {
        self.record_cursors();
        self.set_selections(
            self.all_selected_rows()
                .into_iter()
                .map(|row| Selection::caret(self.line_end(row)))
                .collect(),
        );
    }
    pub fn insert_line(&mut self, above: bool) {
        let original = self.snapshot();
        let rows = self.all_selected_rows();
        let mut cursors = Vec::new();
        let mut changes = Vec::new();
        for row in rows {
            let line = self.line(row);
            let indent: String = line
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t'))
                .collect();
            let pos = if above {
                self.line_start(row)
            } else {
                self.line_end(row)
            };
            let text = if above {
                format!("{indent}{}", self.eol)
            } else {
                format!("{}{indent}", self.eol)
            };
            cursors.push(Selection::caret(pos));
            changes.push((pos..pos, text));
        }
        self.assign_selections(cursors);
        self.apply_changes(changes);
        if above {
            let eol_len = self.eol.chars().count();
            self.cursor = self.cursor.saturating_sub(eol_len);
            for s in &mut self.secondary {
                s.cursor = s.cursor.saturating_sub(eol_len);
            }
        }
        self.replace_undo_snapshot(original);
    }
    fn line_groups(&self) -> Vec<(usize, usize)> {
        let mut groups: Vec<(usize, usize)> = Vec::new();
        for row in self.all_selected_rows() {
            if let Some(last) = groups.last_mut()
                && last.1 + 1 == row
            {
                last.1 = row;
            } else {
                groups.push((row, row));
            }
        }
        groups
    }
    pub fn copy_lines(&mut self, down: bool) {
        let original = self.snapshot();
        let selections = self.selections();
        let mut changes = Vec::new();
        let mut shifts = Vec::new();
        let mut prior = 0;
        for (first, last) in self.line_groups() {
            let start = self.line_start(first);
            let end = if last + 1 < self.line_count() {
                self.line_start(last + 1)
            } else {
                self.len()
            };
            let mut text = self.text.slice(start..end).to_string();
            let shift = if down {
                end - start
                    + if text.ends_with('\n') {
                        0
                    } else {
                        self.eol.chars().count()
                    }
            } else {
                0
            };
            if !text.ends_with('\n') {
                text = if down {
                    format!("{}{text}", self.eol)
                } else {
                    format!("{text}{}", self.eol)
                };
            }
            let pos = if down { end } else { start };
            shifts.push((first, last, prior + shift));
            prior += text.chars().count();
            changes.push((pos..pos, text));
        }
        let selections = selections
            .into_iter()
            .map(|s| {
                let row = self.text.char_to_line(s.range().start);
                let shift = shifts
                    .iter()
                    .find(|(first, last, _)| *first <= row && row <= *last)
                    .map_or(0, |(_, _, shift)| *shift);
                Selection {
                    cursor: s.cursor + shift,
                    anchor: s.anchor.map(|p| p + shift),
                    desired_column: None,
                }
            })
            .collect();
        self.secondary.clear();
        self.apply_changes(changes);
        self.replace_undo_snapshot(original);
        self.set_selections(selections);
    }
    pub fn move_lines(&mut self, down: bool) {
        let original = self.snapshot();
        let groups = self.line_groups();
        let targets: Vec<_> = self
            .selections()
            .into_iter()
            .map(|s| {
                let row = self.text.char_to_line(s.range().start);
                let (first, last) = groups
                    .iter()
                    .find(|(first, last)| *first <= row && row <= *last)
                    .copied()
                    .unwrap();
                let shift = if (!down && first == 0) || (down && last + 1 >= self.line_count()) {
                    0
                } else if down {
                    1
                } else {
                    -1
                };
                let coordinate = |p| {
                    let row = self.text.char_to_line(p);
                    (row.saturating_add_signed(shift), p - self.line_start(row))
                };
                (coordinate(s.cursor), s.anchor.map(coordinate))
            })
            .collect();
        let mut changes = Vec::new();
        for (first, last) in groups {
            if (!down && first == 0) || (down && last + 1 >= self.line_count()) {
                continue;
            }
            let low = if down { first } else { first - 1 };
            let high = if down { last + 1 } else { last };
            let start = self.line_start(low);
            let end = if high + 1 < self.line_count() {
                self.line_start(high + 1)
            } else {
                self.len()
            };
            let mut lines: Vec<_> = (low..=high).map(|row| self.line(row)).collect();
            let endings: Vec<_> = (low..=high)
                .map(|row| {
                    let raw = self.text.line(row).to_string();
                    raw[self.line(row).len()..].to_string()
                })
                .collect();
            if down {
                lines.rotate_right(1);
            } else {
                lines.rotate_left(1);
            }
            let replacement: String = lines
                .iter()
                .zip(endings)
                .map(|(line, eol)| format!("{line}{eol}"))
                .collect();
            changes.push((start..end, replacement));
        }
        if changes.is_empty() {
            return;
        }
        self.secondary.clear();
        self.apply_changes(changes);
        let position = |(row, col): (usize, usize)| {
            if row >= self.line_count() {
                self.len()
            } else {
                (self.line_start(row) + col).min(self.line_end(row))
            }
        };
        let selections = targets
            .into_iter()
            .map(|(cursor, anchor)| Selection {
                cursor: position(cursor),
                anchor: anchor.map(position),
                desired_column: None,
            })
            .collect();
        self.replace_undo_snapshot(original);
        self.set_selections(selections);
    }
    pub fn jump_bracket(&mut self) -> bool {
        let candidate = [self.cursor, self.cursor.saturating_sub(1)]
            .into_iter()
            .find(|p| *p < self.len() && "()[]{}".contains(self.text.char(*p)));
        let Some(start) = candidate else {
            return false;
        };
        let ch = self.text.char(start);
        let (partner, forward) = match ch {
            '(' => (')', true),
            '[' => (']', true),
            '{' => ('}', true),
            ')' => ('(', false),
            ']' => ('[', false),
            '}' => ('{', false),
            _ => unreachable!(),
        };
        let mut depth = 0;
        if forward {
            for pos in start + 1..self.len() {
                let c = self.text.char(pos);
                if c == ch {
                    depth += 1;
                } else if c == partner {
                    if depth == 0 {
                        self.move_to(pos, false);
                        self.secondary.clear();
                        return true;
                    }
                    depth -= 1;
                }
            }
        } else {
            for pos in (0..start).rev() {
                let c = self.text.char(pos);
                if c == ch {
                    depth += 1;
                } else if c == partner {
                    if depth == 0 {
                        self.move_to(pos, false);
                        self.secondary.clear();
                        return true;
                    }
                    depth -= 1;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_order_survives_normalization_typing_and_undo() {
        let mut doc = Document::from_text("a\r\nb\r\nc\r\nd\r\n");
        let original: Vec<_> = [9, 3, 0, 6]
            .into_iter()
            .map(|start| Selection {
                cursor: start + 1,
                anchor: Some(start),
                desired_column: None,
            })
            .collect();
        doc.set_selections(original.clone());
        assert_eq!(doc.selections(), original);
        doc.insert("猫🙂", true);
        assert_eq!(doc.text.to_string(), "猫🙂\r\n猫🙂\r\n猫🙂\r\n猫🙂\r\n");
        assert_eq!(
            doc.selections()
                .iter()
                .map(|s| s.cursor)
                .collect::<Vec<_>>(),
            [14, 6, 2, 10]
        );
        let edited = doc.selections();
        doc.undo();
        assert_eq!(doc.selections(), original);
        doc.redo();
        assert_eq!(doc.selections(), edited);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ordered.txt");
        doc.save_to(&path, false).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), doc.text.to_string());

        // The earliest original selection owns a merged range and direction.
        doc.set_selections(vec![
            Selection::caret(10),
            Selection {
                cursor: 2,
                anchor: Some(5),
                desired_column: None,
            },
            Selection {
                cursor: 6,
                anchor: Some(4),
                desired_column: None,
            },
        ]);
        assert_eq!(doc.selections()[0].cursor, 10);
        assert_eq!(doc.selections()[1].cursor, 2);
        assert_eq!(doc.selections()[1].anchor, Some(6));
    }

    #[test]
    fn multicursor_typing_maps_offsets_and_groups_undo() {
        let mut d = Document::from_text("cat cat cat");
        d.select_next_occurrence(true);
        assert_eq!(d.selections().len(), 3);
        d.insert("猫", true);
        assert_eq!(d.text.to_string(), "猫 猫 猫");
        d.insert("!", true);
        d.insert("?", true);
        assert_eq!(d.text.to_string(), "猫!? 猫!? 猫!?");
        d.undo();
        assert_eq!(d.text.to_string(), "cat cat cat");
        assert_eq!(d.selections().len(), 3);
        d.redo();
        assert_eq!(d.text.to_string(), "猫!? 猫!? 猫!?");
    }
    #[test]
    fn multicursor_delete_preserves_original_selections_on_undo() {
        let mut d = Document::from_text("a🙂 b🙂");
        d.set_selections(vec![Selection::caret(2), Selection::caret(5)]);
        d.backspace(false);
        assert_eq!(d.text.to_string(), "a b");
        d.undo();
        assert_eq!(d.cursor, 2);
        assert_eq!(d.secondary[0].cursor, 5);
        assert_eq!(d.anchor, None);
    }
    #[test]
    fn split_views_share_text_but_keep_cursors_and_map_undo_redo() {
        let mut doc = Document::from_text("abc\nxyz");
        doc.activate_view(1);
        doc.move_to(1, false);
        doc.activate_view(2);
        doc.move_to(5, false);
        doc.top = 1;
        doc.activate_view(1);
        doc.insert("!", true);
        doc.insert("?", true);
        doc.activate_view(2);
        assert_eq!(doc.cursor, 7);
        assert_eq!(doc.top, 1);
        doc.undo();
        assert_eq!(doc.text.to_string(), "abc\nxyz");
        assert_eq!(doc.cursor, 5);
        doc.redo();
        assert_eq!(doc.cursor, 7);
        doc.activate_view(1);
        assert_eq!(doc.cursor, 3);
        doc.undo();
        assert_eq!(doc.cursor, 1);
    }
    #[test]
    fn split_views_map_disjoint_edits_and_grouped_multicursor_undo() {
        let mut doc = Document::from_text("a b c d");
        doc.activate_view(1);
        doc.set_selections(vec![Selection::caret(1), Selection::caret(5)]);
        doc.activate_view(2);
        doc.clear_secondary();
        doc.move_to(3, false);
        doc.activate_view(1);
        doc.insert("x", true);
        doc.insert("y", true);
        doc.activate_view(2);
        assert_eq!(doc.cursor, 5);
        doc.undo();
        assert_eq!(doc.cursor, 3);
        assert_eq!(doc.text.to_string(), "a b c d");
        doc.redo();
        assert_eq!(doc.cursor, 5);
        doc.activate_view(1);
        assert_eq!(doc.selections().len(), 2);
        assert_eq!(doc.cursor, 3);
    }
    #[test]
    fn adjacent_replacements_retain_distinct_carets() {
        let mut doc = Document::from_text("catcat");
        doc.set_selections(vec![
            Selection {
                cursor: 3,
                anchor: Some(0),
                desired_column: None,
            },
            Selection {
                cursor: 6,
                anchor: Some(3),
                desired_column: None,
            },
        ]);
        doc.insert("x", false);
        assert_eq!(doc.text.to_string(), "xx");
        assert_eq!(doc.cursor, 1);
        assert_eq!(doc.secondary[0].cursor, 2);
        doc.insert("!", false);
        assert_eq!(doc.text.to_string(), "x!x!");
        doc.undo();
        doc.undo();
        assert_eq!(doc.text.to_string(), "catcat");
        assert_eq!(doc.selections().len(), 2);
    }
    #[test]
    fn overlapping_selections_are_merged_before_editing() {
        let mut d = Document::from_text("abcdef");
        d.set_selections(vec![
            Selection {
                cursor: 4,
                anchor: Some(1),
                desired_column: None,
            },
            Selection {
                cursor: 5,
                anchor: Some(3),
                desired_column: None,
            },
        ]);
        assert_eq!(d.selections().len(), 1);
        d.insert("x", false);
        assert_eq!(d.text.to_string(), "axf");
    }
    #[test]
    fn move_and_duplicate_preserve_unterminated_last_line() {
        let mut d = Document::from_text("one\r\ntwo");
        d.move_lines(true);
        assert_eq!(d.text.to_string(), "two\r\none");
        d.undo();
        assert_eq!(d.text.to_string(), "one\r\ntwo");
        d.move_to(d.len(), false);
        d.copy_lines(true);
        assert_eq!(d.text.to_string(), "one\r\ntwo\r\ntwo");
        d.undo();
        d.insert_line(true);
        assert_eq!(d.text.to_string(), "one\r\n\r\ntwo");
    }
    #[test]
    fn disjoint_line_moves_and_copies_keep_every_cursor_and_one_undo() {
        let mut doc = Document::from_text("a\nbb\nc\ndd");
        doc.set_selections(vec![Selection::caret(0), Selection::caret(5)]);
        doc.move_lines(true);
        assert_eq!(doc.text.to_string(), "bb\na\ndd\nc");
        assert_eq!(doc.cursor, 3);
        assert_eq!(doc.secondary[0].cursor, 8);
        doc.undo();
        assert_eq!(doc.text.to_string(), "a\nbb\nc\ndd");
        doc.copy_lines(true);
        assert_eq!(doc.text.to_string(), "a\na\nbb\nc\nc\ndd");
        assert_eq!(doc.cursor, 2);
        assert_eq!(doc.secondary[0].cursor, 9);
        doc.undo();
        assert_eq!(doc.secondary[0].cursor, 5);
    }
    #[test]
    fn independent_vertical_columns_and_cursor_undo() {
        let mut d = Document::from_text("abcdef\nx\nabcdef");
        d.move_to(4, false);
        d.add_cursor_vertical(true);
        d.navigate_cursors("cursorDown", false, 20);
        assert_eq!(d.selections().len(), 2);
        d.undo_cursor();
        assert_eq!(d.cursor, 8);
        assert_eq!(d.secondary[0].cursor, 4);
    }
    #[test]
    fn next_occurrence_wraps_and_escape_can_collapse() {
        let mut d = Document::from_text("hello hello hello");
        assert!(d.select_next_occurrence(false));
        assert_eq!(d.selections().len(), 1);
        assert!(d.select_next_occurrence(false));
        assert!(d.select_next_occurrence(false));
        assert!(!d.select_next_occurrence(false));
        assert_eq!(d.selections().len(), 3);
        d.clear_secondary();
        assert_eq!(d.selections().len(), 1);
    }
}
