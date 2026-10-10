//! Pure, bounded folding foundations. No editor commands or view mutation live here.
use anyhow::{Result, bail};
use ropey::Rope;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LINES: usize = 100_000;
pub const MAX_REGIONS: usize = 5_000;
pub const MAX_DEPTH: usize = 256;

/// Character anchors include the visible header and complete hidden body lines.
/// Construction and mapping never alter selections, text, or document identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    start: usize,
    end: usize,
}
impl Region {
    pub fn from_lines(text: &Rope, first: usize, last: usize) -> Result<Self> {
        if first >= last || last >= text.len_lines() {
            bail!("A folding region needs a header and at least one body line");
        }
        let region = Self {
            start: text.line_to_char(first),
            end: if last + 1 < text.len_lines() {
                text.line_to_char(last + 1)
            } else {
                text.len_chars()
            },
        };
        // An empty EOF line has no character anchor; do not silently include it.
        if region.end == text.line_to_char(last) {
            bail!("An empty final line cannot end a folding region");
        }
        region.lines(text)?;
        Ok(region)
    }
    pub fn characters(&self) -> Range<usize> {
        self.start..self.end
    }
    fn lines(&self, text: &Rope) -> Result<Range<usize>> {
        if self.start >= self.end || self.end > text.len_chars() {
            bail!("Folding anchors are outside the current text");
        }
        let first = text.char_to_line(self.start);
        let last = text.char_to_line(self.end - 1);
        if first >= last
            || text.line_to_char(first) != self.start
            || (self.end != text.len_chars() && text.line_to_char(last + 1) != self.end)
        {
            bail!("Folding anchors must enclose complete header and body lines");
        }
        Ok(first..last + 1)
    }
    /// Retain only strictly disjoint edits. Boundary edits can join lines, so
    /// they conservatively invalidate the region along with intersecting edits.
    /// The caller must validate the edit against the old text and invalidate its
    /// row index after any edit, including retained/mapped regions.
    pub fn map_edit(&self, replaced: Range<usize>, inserted_chars: usize) -> Option<Self> {
        if replaced.start > replaced.end {
            return None;
        }
        if replaced.end < self.start {
            Some(Self {
                start: self
                    .start
                    .checked_sub(replaced.len())?
                    .checked_add(inserted_chars)?,
                end: self
                    .end
                    .checked_sub(replaced.len())?
                    .checked_add(inserted_chars)?,
            })
        } else if replaced.start > self.end {
            Some(self.clone())
        } else {
            None
        }
    }
    /// Conservative policy: a collapsed region may not conceal a caret, either
    /// endpoint, or any selected text. The caller retains the original selections.
    pub fn can_collapse(&self, text: &Rope, selections: &[Range<usize>]) -> Result<bool> {
        let lines = self.lines(text)?;
        let hidden = text.line_to_char(lines.start + 1)..self.end;
        for selection in selections {
            if selection.start > selection.end || selection.end > text.len_chars() {
                bail!("Selection is outside the current text");
            }
            if hidden.contains(&selection.start)
                || hidden.contains(&selection.end)
                || (!selection.is_empty()
                    && selection.start < hidden.end
                    && selection.end > hidden.start)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn check(cancelled: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("Folding scan cancelled");
    }
    if Instant::now() >= deadline {
        bail!("Folding scan deadline exceeded");
    }
    Ok(())
}

/// Scan an immutable rope snapshot on the eventual caller's worker. Results are
/// all-or-nothing; exceeding a limit never returns a truncated folding model.
/// There is deliberately no thread/queue allocation in this foundation.
pub fn discover(
    text: &Rope,
    tab_size: usize,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<Vec<Region>> {
    discover_checked(text, tab_size, || check(cancelled, deadline))
}

fn discover_checked(
    text: &Rope,
    tab_size: usize,
    mut guard: impl FnMut() -> Result<()>,
) -> Result<Vec<Region>> {
    guard()?;
    if text.len_bytes() > MAX_BYTES || text.len_lines() > MAX_LINES {
        bail!("Folding scan exceeds 2 MiB or 100,000 lines");
    }
    if !(1..=16).contains(&tab_size) {
        bail!("Folding tab size must be between 1 and 16");
    }
    let mut result = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut previous = None;
    let mut offset = 0;
    let mut last_end = 0;
    for line in text.lines() {
        guard()?;
        let mut indent = 0;
        let mut blank = true;
        for (index, c) in line.chars().enumerate() {
            if index % 1024 == 0 {
                guard()?;
            }
            match c {
                ' ' => indent += 1,
                '\t' => indent += tab_size - indent % tab_size,
                '\r' | '\n' => {}
                _ => {
                    blank = false;
                    break;
                }
            }
        }
        if !blank {
            while stack.last().is_some_and(|(_, level)| *level >= indent) {
                let (start, _) = stack.pop().unwrap();
                result.push(Region {
                    start,
                    end: last_end,
                });
            }
            if let Some((start, level)) = previous
                && indent > level
            {
                if stack.len() >= MAX_DEPTH {
                    bail!("Folding scan exceeds 256 nested regions");
                }
                if result.len() + stack.len() >= MAX_REGIONS {
                    bail!("Folding scan exceeds 5,000 regions");
                }
                stack.push((start, level));
            }
            previous = Some((offset, indent));
            last_end = offset + line.len_chars();
        }
        offset += line.len_chars();
    }
    result.extend(stack.into_iter().map(|(start, _)| Region {
        start,
        end: last_end,
    }));
    guard()?;
    result.sort_unstable_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    guard()?;
    Ok(result)
}

#[derive(Clone, Debug)]
struct Hidden {
    rows: Range<usize>,
    before: usize,
}

/// Immutable row index prepared outside rendering. Hidden logical rows map to
/// their visible header; out-of-document ordinals return None instead of clamping.
#[derive(Clone, Debug)]
pub struct RowMap {
    hidden: Vec<Hidden>,
    logical_count: usize,
    visible_count: usize,
}
impl RowMap {
    pub fn new(text: &Rope, collapsed: &[Region]) -> Result<Self> {
        if collapsed.len() > MAX_REGIONS {
            bail!("Folding row map exceeds 5,000 regions");
        }
        let mut rows = collapsed
            .iter()
            .map(|r| r.lines(text))
            .collect::<Result<Vec<_>>>()?;
        rows.sort_unstable_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
        let mut parents: Vec<Range<usize>> = Vec::new();
        let mut hidden: Vec<Hidden> = Vec::new();
        let mut count = 0;
        for row in rows {
            while parents.last().is_some_and(|p| p.end <= row.start) {
                parents.pop();
            }
            if let Some(parent) = parents.last()
                && (row.end > parent.end || row == *parent)
            {
                bail!("Folding regions cross or duplicate one another");
            }
            if parents.len() >= MAX_DEPTH {
                bail!("Folding row map exceeds 256 nested regions");
            }
            if parents.is_empty() {
                let body = row.start + 1..row.end;
                hidden.push(Hidden {
                    rows: body.clone(),
                    before: count,
                });
                count += body.len();
            }
            parents.push(row);
        }
        Ok(Self {
            hidden,
            logical_count: text.len_lines(),
            visible_count: text.len_lines() - count,
        })
    }
    pub(crate) fn allocated_payload(&self) -> usize {
        std::mem::size_of::<Self>() + self.hidden.capacity() * std::mem::size_of::<Hidden>()
    }
    pub fn visible_count(&self) -> usize {
        self.visible_count
    }
    /// Complete hidden logical lines belonging to an outermost visible header.
    /// Nested collapsed regions are already combined by this immutable index.
    pub fn hidden_body(&self, header: usize) -> Option<Range<usize>> {
        let start = header.checked_add(1)?;
        let index = partition(&self.hidden, |span| span.rows.start < start);
        self.hidden
            .get(index)
            .filter(|span| span.rows.start == start)
            .map(|span| span.rows.clone())
    }
    pub fn logical_to_visible(&self, row: usize) -> Option<usize> {
        if row >= self.logical_count {
            return None;
        }
        let i = partition(&self.hidden, |span| span.rows.start <= row);
        Some(i.checked_sub(1).map_or(row, |i| {
            let span = &self.hidden[i];
            row - span.before - (row + 1 - span.rows.start).min(span.rows.len())
        }))
    }
    pub fn visible_to_logical(&self, row: usize) -> Option<usize> {
        if row >= self.visible_count {
            return None;
        }
        let i = partition(&self.hidden, |span| span.rows.start - span.before <= row);
        Some(i.checked_sub(1).map_or(row, |i| {
            let span = &self.hidden[i];
            row + span.before + span.rows.len()
        }))
    }
    pub fn hidden_header(&self, row: usize) -> Option<usize> {
        let i = partition(&self.hidden, |span| span.rows.start <= row);
        i.checked_sub(1).and_then(|i| {
            self.hidden[i]
                .rows
                .contains(&row)
                .then_some(self.hidden[i].rows.start - 1)
        })
    }
}

fn partition<T>(items: &[T], mut predicate: impl FnMut(&T) -> bool) -> usize {
    let mut low = 0;
    let mut high = items.len();
    while low < high {
        let mid = low + (high - low) / 2;
        if predicate(&items[mid]) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scan(text: &Rope) -> Result<Vec<Region>> {
        discover(
            text,
            4,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10),
        )
    }

    #[test]
    fn indentation_tabs_blank_lines_unicode_and_crlf_preserve_source() {
        let original = "root猫\r\n\tinner🙂\r\n\t\tleaf e\u{301}\r\n\r\n\tsibling\r\nafter\r\n\r\n";
        let text = Rope::from_str(original);
        let ranges = scan(&text).unwrap();
        assert_eq!(
            ranges
                .iter()
                .map(|r| r.lines(&text).unwrap())
                .collect::<Vec<_>>(),
            vec![0..5, 1..3]
        );
        let map = RowMap::new(&text, &ranges).unwrap();
        assert_eq!(
            (0..map.visible_count())
                .map(|r| map.visible_to_logical(r).unwrap())
                .collect::<Vec<_>>(),
            vec![0, 5, 6, 7]
        );
        assert_eq!(text.to_string(), original);
        // Changing configured tab stops changes which mixed-indentation rows nest.
        let mixed = Rope::from_str("root\n\tchild\n   peer\nafter");
        let two = discover(
            &mixed,
            2,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(scan(&mixed).unwrap().len(), 1);
    }

    #[test]
    fn exact_resource_limits_reject_without_partial_results() {
        assert!(scan(&Rope::from_str(&"x".repeat(MAX_BYTES))).is_ok());
        assert!(
            scan(&Rope::from_str(&"x".repeat(MAX_BYTES + 1)))
                .unwrap_err()
                .to_string()
                .contains("2 MiB")
        );
        assert!(scan(&Rope::from_str(&"\n".repeat(MAX_LINES - 1))).is_ok());
        assert!(
            scan(&Rope::from_str(&"\n".repeat(MAX_LINES)))
                .unwrap_err()
                .to_string()
                .contains("lines")
        );
        let exact = Rope::from_str(&"root\n child\n".repeat(MAX_REGIONS));
        assert_eq!(scan(&exact).unwrap().len(), MAX_REGIONS);
        let excessive = Rope::from_str(&"root\n child\n".repeat(MAX_REGIONS + 1));
        assert!(scan(&excessive).unwrap_err().to_string().contains("5,000"));
        let nested = |depth| {
            Rope::from_str(
                &(0..=depth)
                    .map(|n| format!("{}x\n", " ".repeat(n)))
                    .collect::<String>(),
            )
        };
        assert_eq!(scan(&nested(MAX_DEPTH)).unwrap().len(), MAX_DEPTH);
        assert!(
            scan(&nested(MAX_DEPTH + 1))
                .unwrap_err()
                .to_string()
                .contains("256")
        );
        assert!(
            discover(
                &exact,
                0,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            discover(
                &exact,
                17,
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(
            discover(
                &exact,
                4,
                &AtomicBool::new(true),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap_err()
            .to_string()
            .contains("cancelled")
        );
        assert!(
            discover(&exact, 4, &AtomicBool::new(false), Instant::now())
                .unwrap_err()
                .to_string()
                .contains("deadline")
        );
    }

    #[test]
    fn scan_interrupts_long_whitespace_and_discards_already_found_regions() {
        // Deterministically cancel inside the second line's prefix, without
        // scheduler-dependent sleep/thread assertions.
        let text = Rope::from_str(&format!("root\n{}body\n", " ".repeat(50_000)));
        let mut checks = 0;
        let result = discover_checked(&text, 4, || {
            checks += 1;
            if checks == 10 {
                bail!("cancelled during prefix");
            }
            Ok(())
        });
        assert_eq!(result.unwrap_err().to_string(), "cancelled during prefix");
        assert_eq!(checks, 10);
        let text = Rope::from_str(&"root\n body\n".repeat(10));
        let mut checks = 0;
        let result = discover_checked(&text, 4, || {
            checks += 1;
            if checks == 25 {
                bail!("deadline after completed ranges");
            }
            Ok(())
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "deadline after completed ranges"
        );
    }

    #[test]
    fn nested_adjacent_and_disjoint_maps_match_naive_visibility() {
        let text = Rope::from_str(&"x\n".repeat(20));
        let ranges = [(0, 8), (1, 5), (2, 3), (9, 11), (13, 17)]
            .map(|(a, b)| Region::from_lines(&text, a, b).unwrap());
        // Every subset exercises parents expanded while children stay collapsed.
        for mask in 0..1 << ranges.len() {
            let subset = ranges
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, r)| r.clone())
                .collect::<Vec<_>>();
            let map = RowMap::new(&text, &subset).unwrap();
            let visible = (0..text.len_lines())
                .filter(|row| {
                    !subset.iter().any(|r| {
                        let lines = r.lines(&text).unwrap();
                        *row > lines.start && *row < lines.end
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(map.visible_count(), visible.len());
            for (ordinal, row) in visible.iter().copied().enumerate() {
                assert_eq!(map.visible_to_logical(ordinal), Some(row));
                assert_eq!(map.logical_to_visible(row), Some(ordinal));
                assert_eq!(map.hidden_header(row), None);
            }
            for row in 0..text.len_lines() {
                let ordinal = visible.partition_point(|r| *r <= row) - 1;
                assert_eq!(map.logical_to_visible(row), Some(ordinal));
                if !visible.contains(&row) {
                    assert_eq!(map.hidden_header(row), Some(visible[ordinal]));
                }
            }
            assert_eq!(map.visible_to_logical(visible.len()), None);
            assert_eq!(map.logical_to_visible(text.len_lines()), None);
            assert_eq!(map.logical_to_visible(usize::MAX), None);
        }
        assert_eq!(
            RowMap::new(&Rope::new(), &[])
                .unwrap()
                .visible_to_logical(0),
            Some(0)
        );
    }

    #[test]
    fn malformed_crossing_duplicate_and_stale_anchors_are_rejected() {
        let text = Rope::from_str("a\nb\nc\nd\ne\nf\n");
        assert!(Region::from_lines(&text, 0, 0).is_err());
        assert!(Region::from_lines(&text, 0, 6).is_err()); // empty final line
        let a = Region::from_lines(&text, 0, 3).unwrap();
        let b = Region::from_lines(&text, 2, 4).unwrap();
        assert!(RowMap::new(&text, &[a.clone(), b]).is_err());
        assert!(RowMap::new(&text, &[a.clone(), a.clone()]).is_err());
        assert!(RowMap::new(&text, &vec![a.clone(); MAX_REGIONS + 1]).is_err());
        assert!(RowMap::new(&Rope::from_str("short"), &[a]).is_err());
        assert!(RowMap::new(&text, &[Region { start: 1, end: 5 }]).is_err());
        // Valid nesting with an identical header keeps only the outer hidden span.
        let same_header = [
            Region::from_lines(&text, 0, 4).unwrap(),
            Region::from_lines(&text, 0, 2).unwrap(),
        ];
        assert_eq!(RowMap::new(&text, &same_header).unwrap().visible_count(), 3);
        let deep_text = Rope::from_str(&"x\n".repeat(MAX_DEPTH + 3));
        let deep = (1..=MAX_DEPTH + 1)
            .map(|last| Region::from_lines(&deep_text, 0, last).unwrap())
            .collect::<Vec<_>>();
        assert!(
            RowMap::new(&deep_text, &deep)
                .unwrap_err()
                .to_string()
                .contains("256")
        );
    }

    #[test]
    fn selection_policy_retains_anchors_and_secondary_ranges() {
        let text = Rope::from_str("head\n body🙂\n tail\nafter");
        let region = Region::from_lines(&text, 0, 2).unwrap();
        let hidden = text.line_to_char(1);
        let after = text.line_to_char(3);
        let selections = vec![0..0, hidden + 1..hidden + 3, after..after];
        let original = selections.clone();
        assert!(!region.can_collapse(&text, &selections).unwrap());
        assert_eq!(selections, original);
        for selection in [hidden..hidden, 0..hidden, 0..after, hidden - 1..after + 1] {
            assert!(!region.can_collapse(&text, &[selection]).unwrap());
        }
        assert!(region.can_collapse(&text, &[0..4, after..after]).unwrap());
        assert!(
            region
                .can_collapse(&text, std::slice::from_ref(&(0..text.len_chars() + 1)))
                .is_err()
        );
    }

    #[test]
    fn edit_mapping_is_conservative_unicode_safe_and_invertible_when_disjoint() {
        let mut text = Rope::from_str("prefix\r\nhead猫\r\n body🙂\r\nafter\r\n");
        let original = text.clone();
        let region = Region::from_lines(&text, 1, 2).unwrap();
        let inserted = "e\u{301}\r\n";
        let mapped = region.map_edit(0..0, inserted.chars().count()).unwrap();
        text.insert(0, inserted);
        assert_eq!(mapped.lines(&text).unwrap(), 2..4);
        assert_eq!(
            mapped.map_edit(0..inserted.chars().count(), 0),
            Some(region.clone())
        );
        text.remove(0..inserted.chars().count());
        assert_eq!(text, original);
        let span = region.characters();
        for edit in [
            span.start..span.start,
            span.end..span.end,
            span.start - 1..span.start,
            span.start + 1..span.end - 1,
            0..span.end + 1,
        ] {
            assert_eq!(region.map_edit(edit, 0), None);
        }
        assert_eq!(
            region.map_edit(span.end + 1..span.end + 1, 2),
            Some(region.clone())
        );
        assert_eq!(region.map_edit(0..0, usize::MAX), None);
    }

    #[test]
    fn maximum_region_viewport_uses_logarithmic_mapping() {
        let text = Rope::from_str(&"header\n body\n".repeat(MAX_REGIONS));
        let ranges = scan(&text).unwrap();
        let map = RowMap::new(&text, &ranges).unwrap();
        assert_eq!(map.visible_count(), MAX_REGIONS + 1);
        for ordinal in 0..map.visible_count() {
            assert_eq!(map.visible_to_logical(ordinal), Some(ordinal * 2));
        }
        // Count actual comparisons in the same binary-search primitive used by
        // each mapping call, rather than asserting machine-dependent wall time.
        for row in [0, 1, 100, MAX_REGIONS / 2, MAX_REGIONS] {
            let mut comparisons = 0;
            let index = partition(&map.hidden, |span| {
                comparisons += 1;
                span.rows.start - span.before <= row
            });
            assert_eq!(index, row);
            assert!(
                comparisons <= 13,
                "{comparisons} comparisons for {MAX_REGIONS} folds"
            );
        }
    }
}
