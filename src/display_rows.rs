//! Immutable logical/display-row mapping. No view mutation or background work.
//!
//! Wrapping is deliberately unavailable in this foundation. Cell mapping uses
//! the native chunked grapheme iterator, without flattening a logical line.
use crate::{
    document::{MAX_FILE_BYTES, grapheme_width, graphemes},
    folding::{self, Region, RowMap},
};
use anyhow::{Result, ensure};
use ropey::Rope;
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Off,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Before,
    After,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub wrap: Wrap,
    pub width: u16,
    pub tab_size: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowAnchor {
    pub logical_line: usize,
    pub segment: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRow {
    pub anchor: RowAnchor,
    /// Absolute scalar offsets excluding the physical CR/LF line ending.
    pub characters: Range<usize>,
    pub continuation: bool,
    pub indent_cells: u16,
    /// Absolute scalar span hidden after this header; never an editable hit.
    pub folded_body: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Located {
    pub row: RowAnchor,
    /// Absolute logical-line terminal column for the initial Wrap::Off path.
    pub cell: usize,
    /// Hidden positions project to the visible header's end and require reveal.
    pub hidden: bool,
}

/// Immutable preparation identity. Clones preserve identity, a fresh preparation
/// has a different identity even when its text/options are numerically equal.
#[derive(Clone, Debug)]
pub struct DisplayRows {
    text: Rope,
    options: Options,
    folded: Option<Arc<RowMap>>,
    identity: Arc<()>,
}

impl DisplayRows {
    /// The ordinary editor path remains available beyond folding's scan limits.
    pub fn identity(text: Rope, options: Options) -> Result<Self> {
        validate_options(options)?;
        ensure!(
            text.len_bytes() as u64 <= MAX_FILE_BYTES,
            "Display source exceeds the native 32 MiB document limit"
        );
        Ok(Self {
            text,
            options,
            folded: None,
            identity: Arc::new(()),
        })
    }

    /// Entire folded preparation is validated before any result is published.
    /// This function is pure; its eventual caller chooses a bounded worker.
    pub fn prepare_folded(text: Rope, options: Options, collapsed: &[Region]) -> Result<Self> {
        validate_options(options)?;
        ensure!(
            text.len_bytes() <= folding::MAX_BYTES && text.len_lines() <= folding::MAX_LINES,
            "Folded display preparation exceeds 2 MiB or 100,000 lines"
        );
        let map = RowMap::new(&text, collapsed)?;
        Ok(Self {
            text,
            options,
            folded: Some(Arc::new(map)),
            identity: Arc::new(()),
        })
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    pub fn options(&self) -> Options {
        self.options
    }

    pub fn same_projection(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }

    pub fn row_count(&self) -> usize {
        self.folded
            .as_ref()
            .map_or_else(|| self.text.len_lines(), |map| map.visible_count())
    }

    /// Hidden rows and nonexistent continuation segments have no visible anchor.
    pub fn ordinal(&self, anchor: RowAnchor) -> Option<usize> {
        if anchor.segment != 0 || anchor.logical_line >= self.text.len_lines() {
            return None;
        }
        match &self.folded {
            Some(map) if map.hidden_header(anchor.logical_line).is_some() => None,
            Some(map) => map.logical_to_visible(anchor.logical_line),
            None => Some(anchor.logical_line),
        }
    }

    pub fn anchor(&self, ordinal: usize) -> Option<RowAnchor> {
        if ordinal >= self.row_count() {
            return None;
        }
        Some(RowAnchor {
            logical_line: self
                .folded
                .as_ref()
                .map_or(Some(ordinal), |map| map.visible_to_logical(ordinal))?,
            segment: 0,
        })
    }

    pub fn row(&self, anchor: RowAnchor) -> Option<DisplayRow> {
        self.ordinal(anchor)?;
        let characters = line_characters(&self.text, anchor.logical_line);
        let folded_body = self
            .folded
            .as_ref()
            .and_then(|map| map.hidden_body(anchor.logical_line))
            .map(|body| {
                self.text.line_to_char(body.start)..if body.end == self.text.len_lines() {
                    self.text.len_chars()
                } else {
                    self.text.line_to_char(body.end)
                }
            });
        Some(DisplayRow {
            anchor,
            characters,
            continuation: false,
            indent_cells: 0,
            folded_body,
        })
    }

    /// Clamp an accepted visible movement to the first/last visible row.
    pub fn advance(&self, anchor: RowAnchor, amount: isize) -> Option<RowAnchor> {
        let ordinal = self.ordinal(anchor)?;
        self.anchor(
            ordinal
                .saturating_add_signed(amount)
                .min(self.row_count() - 1),
        )
    }

    /// Protocol offsets inside a cluster/EOL remain logical offsets; only their
    /// display projection uses affinity. No caller selection is modified here.
    pub fn locate(&self, scalar: usize, affinity: Affinity) -> Option<Located> {
        if scalar > self.text.len_chars() || self.options.width == 0 {
            return None;
        }
        let logical_line = self.text.char_to_line(scalar);
        let header = self
            .folded
            .as_ref()
            .and_then(|map| map.hidden_header(logical_line));
        let row = RowAnchor {
            logical_line: header.unwrap_or(logical_line),
            segment: 0,
        };
        let characters = line_characters(&self.text, row.logical_line);
        let position = if header.is_some() {
            characters.end
        } else {
            scalar.min(characters.end)
        };
        let mut start = characters.start;
        let mut cell = 0;
        for cluster in graphemes::Graphemes::new(self.text.slice(characters)) {
            let end = start + cluster.len_chars();
            if position <= start {
                break;
            }
            let next = cell + self.cluster_width(cluster, cell);
            if position < end {
                if affinity == Affinity::After {
                    cell = next;
                }
                break;
            }
            start = end;
            cell = next;
        }
        Some(Located {
            row,
            cell,
            hidden: header.is_some(),
        })
    }

    /// A cell beyond line content clamps to that line's end. `cell` is the
    /// absolute logical-line column, so horizontal scrolling is caller-owned.
    /// A zero-width editor and hidden row never produce actionable positions.
    pub fn hit(&self, anchor: RowAnchor, cell: usize, affinity: Affinity) -> Option<usize> {
        if self.options.width == 0 {
            return None;
        }
        let row = self.row(anchor)?;
        let mut start = row.characters.start;
        let mut column = 0;
        for cluster in graphemes::Graphemes::new(self.text.slice(row.characters.clone())) {
            if cell <= column {
                return Some(start);
            }
            let end = start + cluster.len_chars();
            let next = column + self.cluster_width(cluster, column);
            if cell < next {
                return Some(if affinity == Affinity::Before {
                    start
                } else {
                    end
                });
            }
            start = end;
            column = next;
        }
        Some(row.characters.end)
    }

    fn cluster_width(&self, cluster: ropey::RopeSlice<'_>, column: usize) -> usize {
        if cluster.len_chars() == 1 && cluster.char(0) == '\t' {
            let tab = usize::from(self.options.tab_size);
            tab - column % tab
        } else {
            grapheme_width(&graphemes::as_text(cluster), column)
        }
    }
}

fn validate_options(options: Options) -> Result<()> {
    ensure!(
        options.wrap == Wrap::Off,
        "Wrapped display rows are not implemented"
    );
    ensure!(
        (1..=16).contains(&options.tab_size),
        "Display tab size must be between 1 and 16"
    );
    Ok(())
}

fn line_characters(text: &Rope, line: usize) -> Range<usize> {
    let slice = text.line(line);
    let start = text.line_to_char(line);
    let trailing = slice
        .chars_at(slice.len_chars())
        .reversed()
        .take_while(|ch| matches!(ch, '\r' | '\n'))
        .count();
    start..start + slice.len_chars() - trailing
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    fn options() -> Options {
        Options {
            wrap: Wrap::Off,
            width: 80,
            tab_size: 4,
        }
    }

    fn anchor(line: usize) -> RowAnchor {
        RowAnchor {
            logical_line: line,
            segment: 0,
        }
    }

    #[test]
    fn identity_rows_empty_crlf_unicode_and_final_line_preserve_source() {
        for source in ["", "\r\n", "猫🙂\r\nnext\r\n", "last", "\t\n\n"] {
            let rows = DisplayRows::identity(Rope::from_str(source), options()).unwrap();
            let expected = source.split('\n').collect::<Vec<_>>();
            assert_eq!(rows.row_count(), expected.len());
            let mut offset = 0;
            for (line, physical) in expected.iter().enumerate() {
                let content = physical.trim_end_matches('\r');
                let row = rows.row(anchor(line)).unwrap();
                assert_eq!(rows.ordinal(row.anchor), Some(line));
                assert_eq!(rows.anchor(line), Some(row.anchor));
                assert_eq!(row.characters, offset..offset + content.chars().count());
                assert_eq!(rows.text().slice(row.characters).to_string(), content);
                assert!(!row.continuation);
                assert_eq!(row.indent_cells, 0);
                assert!(row.folded_body.is_none());
                offset += physical.chars().count() + usize::from(line + 1 < expected.len());
            }
            assert_eq!(rows.text().to_string(), source);
            assert!(rows.anchor(rows.row_count()).is_none());
            assert!(rows.row(anchor(rows.row_count())).is_none());
            assert!(
                rows.ordinal(RowAnchor {
                    logical_line: 0,
                    segment: 1
                })
                .is_none()
            );
            assert!(
                rows.locate(rows.text().len_chars() + 1, Affinity::Before)
                    .is_none()
            );
            assert_eq!(rows.advance(anchor(0), isize::MIN), Some(anchor(0)));
            assert_eq!(
                rows.advance(anchor(0), isize::MAX),
                Some(anchor(rows.row_count() - 1))
            );
        }
    }

    #[test]
    fn prepared_nested_visibility_matches_independent_interval_oracle() {
        let source = "header猫\r\n inner🙂\r\n  leaf\r\n peer\r\nafter\r\n branch\r\nend\r\n";
        let text = Rope::from_str(source);
        let intervals = [(0, 3), (1, 2), (4, 5)];
        let regions =
            intervals.map(|(first, last)| Region::from_lines(&text, first, last).unwrap());
        for mask in 0..1 << regions.len() {
            let selected = regions
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, region)| region.clone())
                .collect::<Vec<_>>();
            let rows = DisplayRows::prepare_folded(text.clone(), options(), &selected).unwrap();
            let visible = (0..text.len_lines())
                .filter(|line| {
                    !intervals.iter().enumerate().any(|(index, (first, last))| {
                        mask & (1 << index) != 0 && line > first && line <= last
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(rows.row_count(), visible.len());
            for (ordinal, logical) in visible.iter().copied().enumerate() {
                assert_eq!(rows.anchor(ordinal), Some(anchor(logical)));
                assert_eq!(rows.ordinal(anchor(logical)), Some(ordinal));
                let projected = rows.row(anchor(logical)).unwrap();
                let body_end = intervals
                    .iter()
                    .enumerate()
                    .filter(|(index, (first, _))| mask & (1 << index) != 0 && *first == logical)
                    .map(|(_, (_, last))| text.line_to_char(last + 1))
                    .max();
                assert_eq!(
                    projected.folded_body,
                    body_end.map(|end| text.line_to_char(logical + 1)..end)
                );
            }
            for logical in 0..text.len_lines() {
                let located = rows
                    .locate(text.line_to_char(logical), Affinity::Before)
                    .unwrap();
                if visible.contains(&logical) {
                    assert!(!located.hidden);
                    assert_eq!(located.row, anchor(logical));
                } else {
                    assert!(rows.row(anchor(logical)).is_none());
                    assert!(rows.hit(anchor(logical), 0, Affinity::Before).is_none());
                    let header = visible
                        .iter()
                        .copied()
                        .take_while(|line| *line < logical)
                        .last()
                        .unwrap();
                    assert!(located.hidden);
                    assert_eq!(located.row, anchor(header));
                    let header_end = rows.row(anchor(header)).unwrap().characters.end;
                    assert_eq!(
                        rows.hit(located.row, located.cell, Affinity::Before),
                        Some(header_end)
                    );
                }
            }
            assert_eq!(rows.text().to_string(), source);
        }
    }

    // Independent contiguous-string cell boundaries: production walks Rope
    // chunks and obtains a RopeSlice for each cluster instead of this table.
    fn oracle(content: &str, tab: usize) -> Vec<(Range<usize>, Range<usize>)> {
        let mut scalar = 0;
        let mut cell = 0;
        content
            .graphemes(true)
            .map(|cluster| {
                let end = scalar + cluster.chars().count();
                let width = if cluster == "\t" {
                    tab - cell % tab
                } else if cluster.chars().any(char::is_control) {
                    1
                } else {
                    UnicodeWidthStr::width(cluster).max(1)
                };
                let next = cell + width;
                let item = (scalar..end, cell..next);
                scalar = end;
                cell = next;
                item
            })
            .collect()
    }

    #[test]
    fn hit_and_locate_affinity_match_contiguous_unicode_tab_oracle() {
        for tab in [1, 2, 4, 8, 16] {
            for content in [
                "",
                "ab\t猫🙂z",
                "e\u{301} 👩\u{200d}💻 🇬🇧 x",
                "\0\t\u{301}a",
            ] {
                let source = format!("{content}\r\nnext");
                let rows = DisplayRows::identity(
                    Rope::from_str(&source),
                    Options {
                        tab_size: tab,
                        ..options()
                    },
                )
                .unwrap();
                let boundaries = oracle(content, usize::from(tab));
                let length = content.chars().count();
                let width = boundaries.last().map_or(0, |(_, cells)| cells.end);
                for position in 0..=length + 1 {
                    let position_in_content = position.min(length);
                    let inside = boundaries.iter().find(|(chars, _)| {
                        chars.start < position_in_content && position_in_content < chars.end
                    });
                    let exact = boundaries
                        .iter()
                        .find(|(chars, _)| chars.start == position_in_content)
                        .map_or(width, |(_, cells)| cells.start);
                    for affinity in [Affinity::Before, Affinity::After] {
                        let expected = inside.map_or(exact, |(_, cells)| {
                            if affinity == Affinity::Before {
                                cells.start
                            } else {
                                cells.end
                            }
                        });
                        let located = rows.locate(position, affinity).unwrap();
                        assert_eq!(located.row, anchor(0));
                        assert!(!located.hidden);
                        assert_eq!(
                            located.cell, expected,
                            "{content:?}, tab {tab}, scalar {position}, {affinity:?}"
                        );
                    }
                }
                for cell in 0..=width + 2 {
                    for affinity in [Affinity::Before, Affinity::After] {
                        let expected = boundaries
                            .iter()
                            .find(|(_, cells)| cell < cells.end)
                            .map_or(length, |(chars, cells)| {
                                if cell == cells.start || affinity == Affinity::Before {
                                    chars.start
                                } else {
                                    chars.end
                                }
                            });
                        assert_eq!(rows.hit(anchor(0), cell, affinity), Some(expected));
                    }
                }
                assert_eq!(
                    rows.hit(anchor(0), usize::MAX, Affinity::After),
                    Some(length)
                );
                assert_eq!(rows.text().to_string(), source);
            }
        }
    }

    #[test]
    fn chunked_graphemes_and_long_cross_chunk_cluster_keep_scalar_boundaries() {
        let content = format!(
            "{}a{}\t👩\u{200d}💻",
            "猫🙂e\u{301} ".repeat(600),
            "\u{301}".repeat(4000)
        );
        let mut text = Rope::new();
        for ch in content.chars() {
            text.insert_char(text.len_chars(), ch);
        }
        assert!(text.chunks().count() > 1);
        let rows = DisplayRows::identity(text, options()).unwrap();
        let boundaries = oracle(&content, 4);
        let stride = boundaries.len().div_ceil(32);
        for (index, (chars, cells)) in
            boundaries.iter().enumerate().filter(|(index, (chars, _))| {
                index % stride == 0 || index + 1 == boundaries.len() || chars.len() > 1000
            })
        {
            assert_eq!(
                rows.locate(chars.start, Affinity::Before).unwrap().cell,
                cells.start,
                "Cluster {index}"
            );
            assert_eq!(
                rows.locate(chars.end, Affinity::After).unwrap().cell,
                cells.end
            );
            assert_eq!(
                rows.hit(anchor(0), cells.start, Affinity::Before),
                Some(chars.start)
            );
            if chars.len() > 1 {
                assert_eq!(
                    rows.locate(chars.start + 1, Affinity::Before).unwrap().cell,
                    cells.start
                );
                assert_eq!(
                    rows.locate(chars.start + 1, Affinity::After).unwrap().cell,
                    cells.end
                );
            }
        }
        assert_eq!(rows.text().to_string(), content);
    }

    #[test]
    fn hidden_last_line_body_is_exact_and_eof_blank_line_remains_visible() {
        for source in ["head\r\n body猫", "head\r\n body猫\r\n"] {
            let text = Rope::from_str(source);
            let region = Region::from_lines(&text, 0, 1).unwrap();
            let rows = DisplayRows::prepare_folded(text.clone(), options(), &[region]).unwrap();
            assert_eq!(
                rows.row(anchor(0)).unwrap().folded_body,
                Some(text.line_to_char(1)..text.len_chars())
            );
            let hidden = rows.locate(text.line_to_char(1), Affinity::After).unwrap();
            assert!(hidden.hidden);
            assert_eq!(hidden.row, anchor(0));
            let eof = rows.locate(text.len_chars(), Affinity::Before).unwrap();
            assert_eq!(eof.hidden, !source.ends_with('\n'));
            assert_eq!(rows.row_count(), if source.ends_with('\n') { 2 } else { 1 });
        }
    }

    #[test]
    fn invalid_late_regions_and_options_do_not_replace_retained_projection() {
        let source = "a\nb\nc\nd\ne\nf\n";
        let text = Rope::from_str(source);
        let first = Region::from_lines(&text, 0, 3).unwrap();
        let valid =
            DisplayRows::prepare_folded(text.clone(), options(), std::slice::from_ref(&first))
                .unwrap();
        let retained = valid.clone();
        let crossing = Region::from_lines(&text, 2, 4).unwrap();
        assert!(
            DisplayRows::prepare_folded(text.clone(), options(), &[first.clone(), crossing])
                .is_err()
        );
        assert!(
            DisplayRows::prepare_folded(text.clone(), options(), &[first.clone(), first.clone()])
                .is_err()
        );
        assert!(
            DisplayRows::prepare_folded(
                Rope::from_str("short"),
                options(),
                std::slice::from_ref(&first)
            )
            .is_err()
        );
        assert!(
            DisplayRows::prepare_folded(
                text.clone(),
                options(),
                &vec![first; folding::MAX_REGIONS + 1]
            )
            .is_err()
        );
        for bad in [
            Options {
                tab_size: 0,
                ..options()
            },
            Options {
                tab_size: 17,
                ..options()
            },
            Options {
                wrap: Wrap::On,
                ..options()
            },
        ] {
            assert!(DisplayRows::identity(text.clone(), bad).is_err());
            assert!(DisplayRows::prepare_folded(text.clone(), bad, &[]).is_err());
        }
        assert!(valid.same_projection(&retained));
        assert_eq!(retained.row_count(), 4);
        assert_eq!(retained.text().to_string(), source);
    }

    #[test]
    fn zero_width_keeps_row_navigation_and_rejects_source_cell_hits() {
        let text = Rope::from_str("head\n body\nafter");
        let region = Region::from_lines(&text, 0, 1).unwrap();
        for rows in [
            DisplayRows::identity(
                text.clone(),
                Options {
                    width: 0,
                    ..options()
                },
            )
            .unwrap(),
            DisplayRows::prepare_folded(
                text,
                Options {
                    width: 0,
                    ..options()
                },
                &[region],
            )
            .unwrap(),
        ] {
            assert!(rows.row(anchor(0)).is_some());
            assert!(rows.anchor(0).is_some());
            assert!(rows.advance(anchor(0), 1).is_some());
            assert!(rows.hit(anchor(0), 0, Affinity::Before).is_none());
            assert!(rows.locate(0, Affinity::After).is_none());
        }
        for width in [1, 2] {
            let rows = DisplayRows::identity(Rope::from_str("猫"), Options { width, ..options() })
                .unwrap();
            assert_eq!(rows.hit(anchor(0), 1, Affinity::Before), Some(0));
            assert_eq!(rows.hit(anchor(0), 1, Affinity::After), Some(1));
        }
    }

    #[test]
    fn folding_admission_is_separate_from_core_document_identity_limits() {
        let large = Rope::from_str(&"x".repeat(folding::MAX_BYTES + 1));
        assert!(DisplayRows::prepare_folded(large.clone(), options(), &[]).is_err());
        let rows = DisplayRows::identity(large, options()).unwrap();
        assert_eq!(rows.row_count(), 1);
        let many = Rope::from_str(&"\n".repeat(folding::MAX_LINES));
        assert!(DisplayRows::prepare_folded(many.clone(), options(), &[]).is_err());
        assert_eq!(
            DisplayRows::identity(many, options()).unwrap().row_count(),
            folding::MAX_LINES + 1
        );
        let mut maximum = Rope::from_str(&"x".repeat(MAX_FILE_BYTES as usize));
        assert!(DisplayRows::identity(maximum.clone(), options()).is_ok());
        maximum.insert_char(maximum.len_chars(), 'x');
        assert!(DisplayRows::identity(maximum, options()).is_err());
    }

    #[test]
    fn immutable_snapshot_and_private_identity_do_not_revive_equal_preparations() {
        let mut text = Rope::from_str("猫🙂\r\n");
        let original = text.to_string();
        let rows = DisplayRows::identity(text.clone(), options()).unwrap();
        assert!(rows.same_projection(&rows.clone()));
        let equal = DisplayRows::identity(text.clone(), options()).unwrap();
        assert!(!rows.same_projection(&equal));
        text.insert(0, "new");
        let changed = DisplayRows::identity(text.clone(), options()).unwrap();
        text.remove(0..3);
        let reverted = DisplayRows::identity(text, options()).unwrap();
        assert_eq!(rows.text().to_string(), original);
        assert_eq!(reverted.text().to_string(), original);
        assert!(!rows.same_projection(&changed));
        assert!(!rows.same_projection(&reverted));
    }
}
