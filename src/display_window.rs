//! Bounded immutable Wrap-Off viewport preparation, shared by paint and hits.
//! No App/source ownership is granted here. Shared projection/Rope allocations
//! are not window-owned payload; App must separately account for retained maps.
use crate::{
    display_rows::{Affinity, DisplayRow, DisplayRows, RowAnchor},
    document::{grapheme_width, graphemes},
};
use anyhow::{Context, Result, ensure};
use std::{mem::size_of, ops::Range, sync::Arc};

pub const MAX_ROWS: usize = 4096;
pub const MAX_CELLS: usize = 262_144;
pub const MAX_LINE_BYTES: usize = 65_536;
pub const MAX_PREPARED_BYTES: usize = 262_144;
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub row: usize,
    pub column: usize,
}

/// Full source cluster and its clipped viewport cells. Partial clusters must
/// be painted conservatively (e.g. spaces); never slice their source string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub characters: Range<usize>,
    pub bytes: Range<usize>,
    pub cells: Range<usize>,
    pub viewport: Range<usize>,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug)]
struct CompactRun {
    start: u32,
    end: u32,
    byte_start: u32,
    byte_end: u32,
    cell_start: u32,
    cell_end: u32,
}

#[derive(Debug)]
struct PreparedRow {
    descriptor: DisplayRow,
    bytes: Range<usize>,
    runs: Vec<CompactRun>,
    end_cell: usize,
}

#[derive(Debug)]
struct Data {
    projection: DisplayRows,
    top: RowAnchor,
    left: usize,
    right: usize,
    width: u16,
    height: u16,
    rows: Vec<PreparedRow>,
    prepared_bytes: usize,
    allocated_payload: usize,
    preparation_scratch_capacity: usize,
}

/// Clones share both immutable preparation and identity. Fresh equal windows
/// have different identity; exact source projection identity is also retained.
#[derive(Clone, Debug)]
pub struct Window(Arc<Data>);

impl Window {
    pub fn prepare(
        projection: DisplayRows,
        top: RowAnchor,
        left: usize,
        width: u16,
        height: u16,
    ) -> Result<Self> {
        ensure!(
            width == projection.options().width,
            "Window width differs from projection"
        );
        ensure!(usize::from(height) <= MAX_ROWS, "Window exceeds row budget");
        let cells = usize::from(width)
            .checked_mul(usize::from(height))
            .context("Window cell overflow")?;
        ensure!(cells <= MAX_CELLS, "Window exceeds cell budget");
        let right = left
            .checked_add(usize::from(width))
            .context("Window horizontal overflow")?;
        let first = projection
            .ordinal(top)
            .context("Window top is hidden or invalid")?;
        let count = if width == 0 {
            0
        } else {
            usize::from(height).min(projection.row_count() - first)
        };
        let mut rows = Vec::<PreparedRow>::new();
        rows.try_reserve_exact(count)
            .context("Window row allocation failed")?;
        let mut allocated =
            size_of::<Data>() + 2 * size_of::<usize>() + rows.capacity() * size_of::<PreparedRow>();
        ensure!(
            allocated <= MAX_PAYLOAD_BYTES,
            "Window exceeds payload budget"
        );
        let mut prepared_bytes = 0usize;
        let mut scratch = String::new();
        for offset in 0..count {
            let anchor = projection
                .anchor(first + offset)
                .context("Missing visible row")?;
            let text = projection.text();
            // Inspect the full physical line length BEFORE any segmentation.
            ensure!(
                text.line(anchor.logical_line).len_bytes() <= MAX_LINE_BYTES,
                "Visible line exceeds 64 KiB preparation budget"
            );
            prepared_bytes = prepared_bytes
                .checked_add(text.line(anchor.logical_line).len_bytes())
                .context("Window scan overflow")?;
            ensure!(
                prepared_bytes <= MAX_PREPARED_BYTES,
                "Window exceeds aggregate preparation budget"
            );
            let descriptor = projection
                .row(anchor)
                .context("Missing visible descriptor")?;
            let bytes = text.char_to_byte(descriptor.characters.start)
                ..text.char_to_byte(descriptor.characters.end);
            let capacity = descriptor.characters.len();
            let reservation = capacity
                .checked_mul(size_of::<CompactRun>())
                .context("Window payload overflow")?;
            ensure!(
                allocated
                    .checked_add(reservation)
                    .is_some_and(|n| n <= MAX_PAYLOAD_BYTES),
                "Window exceeds payload budget"
            );
            let mut runs = Vec::<CompactRun>::new();
            runs.try_reserve_exact(capacity)
                .context("Window run allocation failed")?;
            allocated = allocated
                .checked_add(
                    runs.capacity()
                        .checked_mul(size_of::<CompactRun>())
                        .context("Window payload overflow")?,
                )
                .context("Window payload overflow")?;
            ensure!(
                allocated <= MAX_PAYLOAD_BYTES,
                "Window exceeds payload budget"
            );
            ensure!(
                allocated
                    .checked_add(scratch.capacity())
                    .is_some_and(|n| n <= MAX_PAYLOAD_BYTES),
                "Window preparation exceeds payload budget"
            );
            let mut scalar = descriptor.characters.start;
            let mut byte = bytes.start;
            let mut cell = 0usize;
            for cluster in graphemes::Graphemes::new(text.slice(descriptor.characters.clone())) {
                let end = scalar
                    .checked_add(cluster.len_chars())
                    .context("Window scalar overflow")?;
                let byte_end = byte
                    .checked_add(cluster.len_bytes())
                    .context("Window byte overflow")?;
                let width = if cluster.len_chars() == 1 && cluster.char(0) == '\t' {
                    let tab = usize::from(projection.options().tab_size);
                    tab - cell % tab
                } else {
                    if let Some(content) = cluster.as_str() {
                        grapheme_width(content, cell)
                    } else {
                        scratch.clear();
                        if scratch.capacity() < cluster.len_bytes() {
                            ensure!(
                                allocated
                                    .checked_add(cluster.len_bytes())
                                    .is_some_and(|n| n <= MAX_PAYLOAD_BYTES),
                                "Window preparation exceeds payload budget"
                            );
                            scratch
                                .try_reserve_exact(cluster.len_bytes())
                                .context("Window cluster allocation failed")?;
                            ensure!(
                                scratch.capacity() <= MAX_LINE_BYTES
                                    && allocated
                                        .checked_add(scratch.capacity())
                                        .is_some_and(|n| n <= MAX_PAYLOAD_BYTES),
                                "Window cluster exceeds payload budget"
                            );
                        }
                        for chunk in cluster.chunks() {
                            scratch.push_str(chunk);
                        }
                        grapheme_width(&scratch, cell)
                    }
                };
                let next = cell.checked_add(width).context("Window column overflow")?;
                runs.push(CompactRun {
                    start: u32::try_from(scalar)?,
                    end: u32::try_from(end)?,
                    byte_start: u32::try_from(byte)?,
                    byte_end: u32::try_from(byte_end)?,
                    cell_start: u32::try_from(cell)?,
                    cell_end: u32::try_from(next)?,
                });
                scalar = end;
                byte = byte_end;
                cell = next;
            }
            rows.push(PreparedRow {
                descriptor,
                bytes,
                runs,
                end_cell: cell,
            });
        }
        Ok(Self(Arc::new(Data {
            projection,
            top,
            left,
            right,
            width,
            height,
            rows,
            prepared_bytes,
            allocated_payload: allocated,
            preparation_scratch_capacity: scratch.capacity(),
        })))
    }

    pub fn same_window(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn same_projection(&self, projection: &DisplayRows) -> bool {
        self.0.projection.same_projection(projection)
    }
    pub fn projection(&self) -> &DisplayRows {
        &self.0.projection
    }
    pub fn top(&self) -> RowAnchor {
        self.0.top
    }
    pub fn left(&self) -> usize {
        self.0.left
    }
    pub fn width(&self) -> u16 {
        self.0.width
    }
    pub fn height(&self) -> u16 {
        self.0.height
    }
    pub fn row_count(&self) -> usize {
        self.0.rows.len()
    }
    pub fn row(&self, index: usize) -> Option<&DisplayRow> {
        self.0.rows.get(index).map(|r| &r.descriptor)
    }
    pub fn row_bytes(&self, index: usize) -> Option<Range<usize>> {
        Some(self.0.rows.get(index)?.bytes.clone())
    }
    pub fn prepared_bytes(&self) -> usize {
        self.0.prepared_bytes
    }
    /// Exact Rust object/vector-capacity payload plus Arc's two counters. Does
    /// not purport to measure allocator headers or retained shared Rope/RowMap.
    pub fn allocated_payload(&self) -> usize {
        self.0.allocated_payload
    }
    /// Capacity of the reusable temporary cross-chunk cluster string, dropped
    /// before publication. Retained payload plus this is checked against 8 MiB.
    pub fn preparation_scratch_capacity(&self) -> usize {
        self.0.preparation_scratch_capacity
    }

    pub fn runs(&self, row: usize) -> impl Iterator<Item = Run> + '_ {
        let runs = self.0.rows.get(row).map_or(&[][..], |r| r.runs.as_slice());
        let first = runs.partition_point(|r| r.cell_end as usize <= self.0.left);
        runs[first..]
            .iter()
            .take_while(|r| (r.cell_start as usize) < self.0.right)
            .map(|r| {
                let cells = r.cell_start as usize..r.cell_end as usize;
                Run {
                    characters: r.start as usize..r.end as usize,
                    bytes: r.byte_start as usize..r.byte_end as usize,
                    viewport: cells.start.max(self.0.left) - self.0.left
                        ..cells.end.min(self.0.right) - self.0.left,
                    complete: cells.start >= self.0.left && cells.end <= self.0.right,
                    cells,
                }
            })
    }

    /// Editable SOURCE cells only. Decorations/fold markers must use a separate
    /// hit region; this API never maps a marker or folded body.
    pub fn hit(&self, row: usize, column: usize, affinity: Affinity) -> Option<usize> {
        if column >= usize::from(self.0.width) || self.0.height == 0 {
            return None;
        }
        let prepared = self.0.rows.get(row)?;
        let cell = self.0.left.checked_add(column)?;
        let index = prepared
            .runs
            .partition_point(|r| r.cell_end as usize <= cell);
        let Some(run) = prepared.runs.get(index) else {
            return Some(prepared.descriptor.characters.end);
        };
        Some(
            if cell <= run.cell_start as usize || affinity == Affinity::Before {
                run.start as usize
            } else {
                run.end as usize
            },
        )
    }

    /// Only visible, horizontally in-window carets are returned. Hidden body,
    /// off-window rows and right-edge/offscreen columns return None, not guesses.
    pub fn locate(&self, scalar: usize, affinity: Affinity) -> Option<Caret> {
        if self.0.width == 0 || scalar > self.0.projection.text().len_chars() {
            return None;
        }
        let line = self.0.projection.text().char_to_line(scalar);
        let index = self
            .0
            .rows
            .binary_search_by_key(&line, |r| r.descriptor.anchor.logical_line)
            .ok()?;
        let row = &self.0.rows[index];
        let scalar = scalar.min(row.descriptor.characters.end);
        let run_index = row.runs.partition_point(|r| r.end as usize <= scalar);
        let cell = row.runs.get(run_index).map_or(row.end_cell, |run| {
            if scalar <= run.start as usize || affinity == Affinity::Before {
                run.cell_start as usize
            } else {
                run.cell_end as usize
            }
        });
        if !(self.0.left..self.0.right).contains(&cell) {
            return None;
        }
        Some(Caret {
            row: index,
            column: cell - self.0.left,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        display_rows::{Options, Wrap},
        folding::Region,
    };
    use ropey::Rope;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    fn anchor(line: usize) -> RowAnchor {
        RowAnchor {
            logical_line: line,
            segment: 0,
        }
    }
    fn projection(source: &str, width: u16, tab: u8) -> DisplayRows {
        DisplayRows::identity(
            Rope::from_str(source),
            Options {
                wrap: Wrap::Off,
                width,
                tab_size: tab,
            },
        )
        .unwrap()
    }
    // Independent contiguous-string oracle, not DisplayRows hit/locate or native
    // chunked segmentation. Test tabs follow absolute columns, not viewport x.
    fn oracle(content: &str, scalar_start: usize, byte_start: usize, tab: usize) -> Vec<Run> {
        let mut scalar = scalar_start;
        let mut byte = byte_start;
        let mut cell = 0;
        content
            .graphemes(true)
            .map(|g| {
                let width = if g == "\t" {
                    tab - cell % tab
                } else if g.chars().any(char::is_control) {
                    1
                } else {
                    UnicodeWidthStr::width(g).max(1)
                };
                let run = Run {
                    characters: scalar..scalar + g.chars().count(),
                    bytes: byte..byte + g.len(),
                    cells: cell..cell + width,
                    viewport: 0..0,
                    complete: false,
                };
                scalar = run.characters.end;
                byte = run.bytes.end;
                cell = run.cells.end;
                run
            })
            .collect()
    }
    fn expected_hit(runs: &[Run], end: usize, cell: usize, affinity: Affinity) -> usize {
        for run in runs {
            if cell <= run.cells.start {
                return run.characters.start;
            }
            if cell < run.cells.end {
                return if affinity == Affinity::Before {
                    run.characters.start
                } else {
                    run.characters.end
                };
            }
        }
        end
    }
    fn expected_cell(runs: &[Run], scalar: usize, affinity: Affinity) -> usize {
        let mut cell = 0;
        for run in runs {
            if scalar <= run.characters.start {
                break;
            }
            if scalar < run.characters.end {
                if affinity == Affinity::After {
                    cell = run.cells.end;
                }
                break;
            }
            cell = run.cells.end;
        }
        cell
    }

    #[test]
    fn unicode_crlf_cells_hits_and_carets_match_independent_oracle_for_every_offset() {
        let lines = ["a\t猫e\u{301}👩\u{200d}💻🇬🇧z", "\t🙂\t\u{301}\u{1}Q", ""];
        let source = format!("{}\r\n{}\r\n{}", lines[0], lines[1], lines[2]);
        for tab in [1, 4, 16] {
            for left in 0..26 {
                let rows = projection(&source, 7, tab);
                let window = Window::prepare(rows.clone(), anchor(0), left, 7, 3).unwrap();
                let mut scalar_start = 0;
                let mut byte_start = 0;
                for (row, line) in lines.iter().enumerate() {
                    let oracle = oracle(line, scalar_start, byte_start, usize::from(tab));
                    let end = scalar_start + line.chars().count();
                    let actual = window.runs(row).collect::<Vec<_>>();
                    let expected = oracle
                        .iter()
                        .filter(|r| r.cells.end > left && r.cells.start < left + 7)
                        .map(|r| {
                            let mut r = r.clone();
                            r.viewport =
                                r.cells.start.max(left) - left..r.cells.end.min(left + 7) - left;
                            r.complete = r.cells.start >= left && r.cells.end <= left + 7;
                            r
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(actual, expected, "tab {tab} left {left} row {row}");
                    assert_eq!(
                        window.row_bytes(row),
                        Some(byte_start..byte_start + line.len())
                    );
                    for x in 0..7 {
                        for affinity in [Affinity::Before, Affinity::After] {
                            assert_eq!(
                                window.hit(row, x, affinity),
                                Some(expected_hit(&oracle, end, left + x, affinity))
                            );
                        }
                    }
                    for scalar in scalar_start..=end {
                        for affinity in [Affinity::Before, Affinity::After] {
                            let cell = expected_cell(&oracle, scalar, affinity);
                            let expected = (left..left + 7).contains(&cell).then(|| Caret {
                                row,
                                column: cell - left,
                            });
                            assert_eq!(
                                window.locate(scalar, affinity),
                                expected,
                                "scalar {scalar} affinity {affinity:?}"
                            );
                        }
                    }
                    if row < 2 {
                        // Both CR and LF positions project to the content EOL.
                        for scalar in end..end + 2 {
                            assert_eq!(
                                window.locate(scalar, Affinity::Before),
                                window.locate(end, Affinity::Before)
                            );
                        }
                        scalar_start = end + 2;
                        byte_start += line.len() + 2;
                    }
                }
                assert_eq!(window.projection().text().to_string(), source);
                assert!(window.hit(3, 0, Affinity::Before).is_none());
                assert!(window.hit(0, 7, Affinity::Before).is_none());
            }
        }
    }

    #[test]
    fn partial_clusters_and_blank_eol_use_full_cluster_affinities() {
        let window = Window::prepare(projection("猫🙂\r\n", 2, 4), anchor(0), 1, 2, 1).unwrap();
        let runs = window.runs(0).collect::<Vec<_>>();
        assert_eq!(
            runs.iter()
                .map(|r| (r.viewport.clone(), r.complete))
                .collect::<Vec<_>>(),
            vec![(0..1, false), (1..2, false)]
        );
        assert_eq!(window.hit(0, 0, Affinity::Before), Some(0));
        assert_eq!(window.hit(0, 0, Affinity::After), Some(1));
        assert_eq!(window.hit(0, 1, Affinity::After), Some(1)); // exact boundary
        assert_eq!(
            window.locate(1, Affinity::Before),
            Some(Caret { row: 0, column: 1 })
        );
        let blank = Window::prepare(projection("猫🙂", 3, 4), anchor(0), 4, 3, 1).unwrap();
        assert_eq!(blank.runs(0).count(), 0);
        for x in 0..3 {
            assert_eq!(blank.hit(0, x, Affinity::After), Some(2));
        }
        assert_eq!(
            blank.locate(2, Affinity::After),
            Some(Caret { row: 0, column: 0 })
        );
    }

    #[test]
    fn folded_headers_retain_metadata_but_body_and_offwindow_positions_are_inert() {
        let source = Rope::from_str("head猫\r\n hidden🙂\r\n leaf\r\nafter\r\nend");
        let region = Region::from_lines(&source, 0, 2).unwrap();
        let rows = DisplayRows::prepare_folded(
            source.clone(),
            Options {
                wrap: Wrap::Off,
                width: 12,
                tab_size: 4,
            },
            &[region],
        )
        .unwrap();
        let window = Window::prepare(rows, anchor(0), 0, 12, 2).unwrap();
        assert_eq!(window.row(0).unwrap().anchor, anchor(0));
        assert_eq!(window.row(1).unwrap().anchor, anchor(3));
        assert_eq!(
            window.row(0).unwrap().folded_body,
            Some(source.line_to_char(1)..source.line_to_char(3))
        );
        for scalar in source.line_to_char(1)..source.line_to_char(3) {
            assert!(window.locate(scalar, Affinity::After).is_none());
        }
        assert!(
            window
                .locate(source.line_to_char(4), Affinity::Before)
                .is_none()
        );
        for x in 0..12 {
            let hit = window.hit(0, x, Affinity::After).unwrap();
            assert!(hit <= window.row(0).unwrap().characters.end);
        }
        assert!(Window::prepare(window.projection().clone(), anchor(1), 0, 12, 1).is_err());
    }

    #[test]
    fn source_and_window_identities_do_not_revive_from_equal_preparations() {
        let rows = projection("equal", 4, 4);
        let first = Window::prepare(rows.clone(), anchor(0), 0, 4, 1).unwrap();
        let clone = first.clone();
        let fresh = Window::prepare(rows.clone(), anchor(0), 0, 4, 1).unwrap();
        let other = projection("equal", 4, 4);
        assert!(first.same_window(&clone));
        assert!(!first.same_window(&fresh));
        assert!(first.same_projection(&rows));
        assert!(!first.same_projection(&other));
        assert!(first.same_projection(clone.projection()));
    }

    #[test]
    fn zero_dimensions_invalid_anchors_and_overflow_never_authorize_hits() {
        for (width, height) in [(0, 10), (4, 0)] {
            let w =
                Window::prepare(projection("x", width, 4), anchor(0), 0, width, height).unwrap();
            assert_eq!(w.row_count(), 0);
            assert!(w.hit(0, 0, Affinity::Before).is_none());
            assert!(w.locate(0, Affinity::Before).is_none());
            assert_eq!(w.runs(0).count(), 0);
        }
        assert!(Window::prepare(projection("x", 2, 4), anchor(0), usize::MAX, 2, 1).is_err());
        assert!(Window::prepare(projection("x", 2, 4), anchor(0), 0, 1, 1).is_err());
        assert!(
            Window::prepare(
                projection("x", 2, 4),
                RowAnchor {
                    logical_line: 0,
                    segment: 1
                },
                0,
                2,
                1
            )
            .is_err()
        );
        assert!(Window::prepare(projection("x", 2, 4), anchor(5), 0, 2, 1).is_err());
        assert!(Window::prepare(projection("x", 1, 4), anchor(0), 0, 1, 4097).is_err());
        assert!(Window::prepare(projection("x", 65, 4), anchor(0), 0, 65, 4096).is_err());
    }

    #[test]
    fn physical_line_limit_precedes_segmentation_and_failed_windows_preserve_old_authority() {
        // A single >64KiB cross-chunk combining cluster must be rejected whole.
        let long = format!("a{}\r\n", "\u{301}".repeat(MAX_LINE_BYTES / 2));
        let rows = projection(&format!("ok\r\n{long}"), 20, 4);
        let old = Window::prepare(rows.clone(), anchor(0), 0, 20, 1).unwrap();
        let retained = old.clone();
        assert!(Window::prepare(rows.clone(), anchor(0), 0, 20, 2).is_err());
        assert!(old.same_window(&retained));
        assert!(old.same_projection(&rows));
        assert_eq!(old.hit(0, 1, Affinity::Before), Some(1));
    }

    #[test]
    fn maximal_scan_and_cell_cohorts_report_exact_vector_capacity_payload() {
        let source = format!("{}\n", "x".repeat(MAX_LINE_BYTES - 1)).repeat(4);
        let rows = projection(&source, 64, 4);
        let window = Window::prepare(rows.clone(), anchor(0), 0, 64, 4).unwrap();
        assert_eq!(window.prepared_bytes(), MAX_PREPARED_BYTES);
        let actual = size_of::<Data>()
            + 2 * size_of::<usize>()
            + window.0.rows.capacity() * size_of::<PreparedRow>()
            + window
                .0
                .rows
                .iter()
                .map(|r| r.runs.capacity() * size_of::<CompactRun>())
                .sum::<usize>();
        assert_eq!(window.allocated_payload(), actual);
        assert!(actual <= MAX_PAYLOAD_BYTES);
        let too_many = projection(&(source + "z\n"), 64, 4);
        assert!(Window::prepare(too_many, anchor(0), 0, 64, 5).is_err());
        let short = projection(&"\n".repeat(MAX_ROWS), 64, 4);
        let max = Window::prepare(short, anchor(0), 0, 64, MAX_ROWS as u16).unwrap();
        assert_eq!(max.row_count(), MAX_ROWS);
        assert_eq!(
            max.hit(MAX_ROWS - 1, 63, Affinity::After),
            Some(MAX_ROWS - 1)
        );
    }

    #[test]
    fn cross_chunk_graphemes_keep_absolute_byte_ranges_without_flattened_prefixes() {
        let source = format!(
            "prefix\r\n{}e{}👩\u{200d}💻suffix",
            "x".repeat(3000),
            "\u{301}".repeat(2000)
        );
        let window = Window::prepare(projection(&source, 12, 4), anchor(1), 2998, 12, 1).unwrap();
        let line = source.split("\r\n").nth(1).unwrap();
        let expected = oracle(line, 8, 8, 4);
        for run in window.runs(0) {
            let original = expected
                .iter()
                .find(|r| r.characters == run.characters)
                .unwrap();
            assert_eq!(run.bytes, original.bytes);
            assert_eq!(run.cells, original.cells);
            assert!(std::str::from_utf8(&source.as_bytes()[run.bytes.clone()]).is_ok());
        }
        assert!(window.preparation_scratch_capacity() > 0);
        assert!(window.preparation_scratch_capacity() <= MAX_LINE_BYTES);
        assert!(
            window.allocated_payload() + window.preparation_scratch_capacity() <= MAX_PAYLOAD_BYTES
        );
        let inside = 8 + 3001;
        assert_eq!(
            window.locate(inside, Affinity::Before),
            Some(Caret { row: 0, column: 2 })
        );
        assert_eq!(
            window.locate(inside, Affinity::After),
            Some(Caret { row: 0, column: 3 })
        );
    }
    #[test]
    fn bounded_visible_work_does_not_inspect_offwindow_long_lines_or_all_document_rows() {
        let source = format!(
            "{}small猫\r\n{}",
            "x\n".repeat(100_001),
            "z".repeat(MAX_LINE_BYTES + 1)
        );
        let rows = projection(&source, 12, 4);
        let window = Window::prepare(rows.clone(), anchor(100_001), 0, 12, 1).unwrap();
        assert_eq!(window.row_count(), 1);
        assert_eq!(window.prepared_bytes(), "small猫\r\n".len());
        assert_eq!(window.row(0).unwrap().anchor, anchor(100_001));
        assert_eq!(window.hit(0, 6, Affinity::After), Some(200_008));
        assert!(window.locate(0, Affinity::Before).is_none());
        assert!(Window::prepare(rows, anchor(100_001), 0, 12, 2).is_err());
    }
}
