//! Atomic native comment commands with bounded inspection and no document copy.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const INSPECT: usize = 64 * 1024;
const ROWS: usize = 10_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentOperation {
    Toggle,
    Add,
    Remove,
}
#[derive(Clone, Copy)]
struct Point {
    position: usize,
    right: bool,
    extra: usize,
}
impl Point {
    fn at(position: usize, right: bool) -> Self {
        Self {
            position,
            right,
            extra: 0,
        }
    }
}
struct Target {
    anchor: Option<Point>,
    cursor: Point,
}
struct Prefix {
    indent: Vec<char>,
    blank: bool,
    commented: usize,
}
struct Boundary {
    text: String,
    bytes: Vec<usize>,
    start: usize,
}
impl Boundary {
    fn len(&self) -> usize {
        self.bytes.len() - 1
    }
    fn part(&self, range: Range<usize>) -> &str {
        &self.text[self.bytes[range.start]..self.bytes[range.end]]
    }
    fn scalar(&self, byte: usize) -> usize {
        self.bytes
            .binary_search(&byte)
            .expect("UTF-8 token boundary")
    }
    fn char(&self, at: usize) -> Option<char> {
        (at < self.len()).then(|| self.part(at..at + 1).chars().next().unwrap())
    }
}
impl Document {
    fn comment_delimiters(&self) -> (Option<&str>, Option<(&str, &str)>) {
        if let Some(comments) = self
            .language_configuration
            .as_ref()
            .and_then(|c| c.comments.as_ref())
        {
            return (
                comments.line_comment.as_deref(),
                comments
                    .block_comment
                    .as_ref()
                    .map(|(open, close)| (open.as_str(), close.as_str())),
            );
        }
        let extension = self
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|p| p.to_str());
        let line = match extension {
            Some("py" | "sh" | "bash" | "toml" | "yaml" | "yml" | "rb") => "#",
            Some("sql" | "lua") => "--",
            _ => "//",
        };
        let block = match extension {
            Some("py" | "sh" | "bash" | "toml" | "yaml" | "yml" | "rb") => None,
            Some("html") => Some(("<!--", "-->")),
            Some("lua") => Some(("--[[", "]]")),
            _ => Some(("/*", "*/")),
        };
        (Some(line), block)
    }
    fn comment_prefix(&self, row: usize, token: &str, budget: &mut usize) -> Result<Prefix> {
        let start = self.line_start(row);
        let end = self.line_end(row);
        let mut indent = Vec::new();
        for ch in self.text.slice(start..end).chars() {
            if !matches!(ch, ' ' | '\t') {
                break;
            }
            charge(budget, ch.len_utf8())?;
            indent.push(ch);
        }
        let at = start + indent.len();
        let mut commented = 0;
        if at < end {
            let mut characters = self.text.chars_at(at);
            let mut matched = true;
            for wanted in token.chars() {
                let actual = characters.next().filter(|_| at + commented < end);
                charge(budget, wanted.len_utf8())?;
                if actual.is_none_or(|actual| !actual.eq_ignore_ascii_case(&wanted)) {
                    matched = false;
                    break;
                }
                commented += 1;
            }
            if !matched {
                commented = 0;
            } else if at + commented < end && self.text.char(at + commented) == ' ' {
                commented += 1;
            }
        }
        Ok(Prefix {
            blank: at == end,
            indent,
            commented,
        })
    }
    pub fn comment_lines(&mut self, operation: CommentOperation) -> Result<()> {
        let Some(token) = self.comment_delimiters().0 else {
            // The pinned line command delegates to block commenting when the
            // language has only block delimiters, irrespective of toggle mode.
            return self.block_comments(true);
        };
        let token = token.to_owned();
        let mut selections = self.smart_selections()?;
        // The line command returns document-order selections. Its Undo
        // snapshot still records the original primary and caller order.
        selections.sort_by_key(|selection| selection.range().start);
        let mut order = (0..selections.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| selections[*index].range().start);
        let mut seen = BTreeSet::new();
        let mut prefixes = BTreeMap::new();
        let mut budget = INSPECT;
        let mut changes = Vec::new();
        for index in order {
            let selection = &selections[index];
            let range = selection.range();
            let first = self.text.char_to_line(range.start);
            let last = self.text.char_to_line(if range.is_empty() {
                range.end
            } else {
                range.end - 1
            });
            if last - first >= ROWS {
                bail!("Comment commands support at most 10,000 selected rows");
            }
            let mut rows = Vec::new();
            for row in first..=last {
                if !seen.insert(row) {
                    continue;
                }
                if seen.len() > ROWS {
                    bail!("Comment commands support at most 10,000 selected rows");
                }
                prefixes.insert(row, self.comment_prefix(row, &token, &mut budget)?);
                rows.push(row);
            }
            let all_blank = rows.iter().all(|row| prefixes[row].blank);
            let remove = operation == CommentOperation::Remove
                || (operation == CommentOperation::Toggle
                    && !all_blank
                    && rows
                        .iter()
                        .filter(|row| !prefixes[*row].blank)
                        .all(|row| prefixes[row].commented > 0));
            let active = rows
                .iter()
                .copied()
                .filter(|row| {
                    !prefixes[row].blank || (operation == CommentOperation::Toggle && all_blank)
                })
                .collect::<Vec<_>>();
            let tab = self.tab_size.clamp(1, 16);
            let common = active
                .iter()
                .map(|row| visible(&prefixes[row].indent, tab))
                .min()
                .unwrap_or(0)
                / tab
                * tab;
            for row in active {
                let prefix = &prefixes[&row];
                let start = self.line_start(row);
                if remove {
                    if prefix.commented > 0 {
                        let at = start + prefix.indent.len();
                        changes.push((at..at + prefix.commented, String::new()));
                    }
                } else {
                    let mut column = 0;
                    let mut count = 0;
                    for ch in &prefix.indent {
                        let next = advance(column, *ch, tab);
                        if next > common {
                            break;
                        }
                        column = next;
                        count += 1;
                        if column == common {
                            break;
                        }
                    }
                    let at = start + count;
                    changes.push((at..at, format!("{token} ")));
                }
            }
        }
        let targets = selections
            .iter()
            .map(|selection| Target {
                anchor: selection.anchor.map(|position| Point::at(position, true)),
                cursor: Point::at(selection.cursor, true),
            })
            .collect();
        self.commit_comments(changes, targets)
    }
    fn comment_boundary(&self, row: usize, budget: &mut usize) -> Result<Boundary> {
        let slice = self.line_slice(row);
        charge(budget, slice.len_bytes())?;
        let text = slice.to_string();
        let bytes = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text.len()))
            .collect();
        Ok(Boundary {
            text,
            bytes,
            start: self.line_start(row),
        })
    }
    pub fn toggle_block_comment(&mut self) -> Result<()> {
        self.block_comments(false)
    }
    fn block_comments(&mut self, line_mode: bool) -> Result<()> {
        let Some((open, close)) = self.comment_delimiters().1 else {
            return Ok(());
        };
        let (open, close) = (open.to_owned(), close.to_owned());
        let opening = open.chars().count();
        let closing = close.chars().count();
        let selections = self.smart_selections()?;
        let mut cache = BTreeMap::<usize, Boundary>::new();
        let mut budget = INSPECT;
        let mut changes = Vec::new();
        let mut insertions = BTreeMap::<usize, (String, String)>::new();
        let mut targets = Vec::new();
        for selection in selections {
            let original_range = selection.range();
            let mut range = original_range.clone();
            if line_mode && !range.is_empty() {
                let last = self.text.char_to_line(range.end - 1);
                range.end = range.end.min(self.line_end(last));
            }
            let first = self.text.char_to_line(range.start);
            let last = self.text.char_to_line(range.end);
            for row in [first, last] {
                if cache.len() == 128 && !cache.contains_key(&row) {
                    bail!("Block comments support at most 128 boundary lines per gesture");
                }
                if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(row) {
                    entry.insert(self.comment_boundary(row, &mut budget)?);
                }
            }
            let start_line = &cache[&first];
            let end_line = &cache[&last];
            let start = range.start - start_line.start;
            let end = range.end - end_line.start;
            if start > start_line.len() || end > end_line.len() {
                bail!("Comment boundary falls inside a line ending");
            }
            // Charge repeated searches even when the same bounded line has
            // already been materialized for another selection.
            charge(
                &mut budget,
                start_line.text.len()
                    + if first == last {
                        0
                    } else {
                        end_line.text.len()
                    },
            )?;
            let allowed = (start + opening * 2).min(start_line.len());
            let opening_at = start_line
                .part(0..allowed)
                .rfind(&open)
                .map(|byte| start_line.scalar(byte));
            let from = end.saturating_sub(closing);
            let closing_at = end_line
                .part(from..end_line.len())
                .find(&close)
                .map(|byte| end_line.scalar(end_line.bytes[from] + byte));
            let existing = opening_at.zip(closing_at).filter(|(left, right)| {
                if first == last {
                    left + opening <= *right
                        && !start_line.part(left + opening..*right).contains(&close)
                } else {
                    !start_line
                        .part(left + opening..start_line.len())
                        .contains(&close)
                        && !end_line.part(0..*right).contains(&close)
                }
            });
            if let Some((left, right)) = existing {
                let left_end =
                    left + opening + usize::from(start_line.char(left + opening) == Some(' '));
                let right_start = right.saturating_sub(usize::from(
                    right > 0 && end_line.char(right - 1) == Some(' '),
                ));
                let left = start_line.start + left;
                let content_start = start_line.start + left_end;
                let content_end = end_line.start + right_start;
                let right_end = end_line.start + right + closing;
                if content_start >= content_end {
                    changes.push((left..right_end, String::new()));
                } else {
                    changes.push((left..content_start, String::new()));
                    changes.push((content_end..right_end, String::new()));
                }
                targets.push(if line_mode {
                    Target {
                        anchor: selection.anchor.map(|position| Point::at(position, false)),
                        cursor: Point::at(selection.cursor, false),
                    }
                } else {
                    Target {
                        anchor: (content_start < content_end)
                            .then_some(Point::at(content_start, false)),
                        cursor: Point::at(content_end, false),
                    }
                });
                continue;
            }
            if line_mode {
                let first_content = start_line
                    .text
                    .chars()
                    .take_while(|ch| matches!(ch, ' ' | '\t'))
                    .count();
                range = start_line.start + first_content..end_line.start + end_line.len();
            }
            if range.is_empty() {
                changes.push((range.clone(), format!("{open}  {close}")));
                targets.push(Target {
                    anchor: None,
                    cursor: Point {
                        position: if line_mode {
                            selection.cursor
                        } else {
                            range.start
                        },
                        right: false,
                        extra: opening + 1,
                    },
                });
            } else {
                insertions
                    .entry(range.start)
                    .or_default()
                    .1
                    .push_str(&format!("{open} "));
                insertions
                    .entry(range.end)
                    .or_default()
                    .0
                    .push_str(&format!(" {close}"));
                targets.push(if line_mode {
                    Target {
                        anchor: selection.anchor.map(|position| Point::at(position, false)),
                        cursor: Point::at(selection.cursor, false),
                    }
                } else {
                    Target {
                        anchor: Some(Point::at(range.start, true)),
                        cursor: Point::at(range.end, false),
                    }
                });
            }
        }
        changes.extend(
            insertions
                .into_iter()
                .map(|(position, (close, open))| (position..position, close + &open)),
        );
        self.commit_comments(changes, targets)
    }
    fn commit_comments(
        &mut self,
        mut changes: Vec<(Range<usize>, String)>,
        targets: Vec<Target>,
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        changes.sort_by_key(|(range, _)| range.start);
        self.validate_typing_size(&changes)?;
        let mut shifts = Vec::with_capacity(changes.len() + 1);
        shifts.push(0isize);
        for (range, text) in &changes {
            shifts.push(
                shifts.last().unwrap() + text.chars().count() as isize - range.len() as isize,
            );
        }
        let map = |point: Point| {
            let index = changes.partition_point(|(range, _)| {
                range.end < point.position
                    || (range.end == point.position && (!range.is_empty() || point.right))
            });
            let position = if let Some((range, text)) = changes.get(index)
                && range.start <= point.position
                && range.end > point.position
            {
                range.start.saturating_add_signed(shifts[index])
                    + if point.right { text.chars().count() } else { 0 }
            } else {
                point.position.saturating_add_signed(shifts[index])
            };
            position + point.extra
        };
        let selections = targets
            .into_iter()
            .map(|target| Selection {
                anchor: target.anchor.map(map),
                cursor: map(target.cursor),
                desired_column: None,
            })
            .collect::<Vec<_>>();
        let resulting = self.len().saturating_add_signed(*shifts.last().unwrap());
        if selections
            .iter()
            .any(|s| s.cursor > resulting || s.anchor.is_some_and(|a| a > resulting))
        {
            bail!("Invalid comment selection endpoint");
        }
        let original = self.snapshot();
        self.secondary.clear();
        self.apply_changes(changes);
        self.replace_undo_snapshot(original);
        self.assign_selections(selections);
        Ok(())
    }
}
fn charge(budget: &mut usize, bytes: usize) -> Result<()> {
    *budget = budget
        .checked_sub(bytes)
        .context("Comment inspection exceeds 64 KiB per gesture")?;
    Ok(())
}
fn advance(column: usize, ch: char, tab: usize) -> usize {
    if ch == '\t' {
        column + tab - column % tab
    } else {
        column + 1
    }
}
fn visible(indent: &[char], tab: usize) -> usize {
    indent
        .iter()
        .fold(0, |column, ch| advance(column, *ch, tab))
}
