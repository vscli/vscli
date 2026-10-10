//! Native visible folding owns one qualified actual lane. Painted source hits
//! use the same immutable window as cells/carets; stale frames never fall back
//! to logical-y mapping. No JS, filesystem I/O or source discovery here.
use super::*;
use crate::{
    display_rows::{Affinity, Options, Wrap},
    display_window::{Budget, Window},
    document::{FoldAction, Selection},
    editor_groups::{Membership, UiProof},
    folding_controller::{Intent, Lifetime, State as Lane, ViewProof},
    folding_worker::{DocumentWork, Prepared},
};
use anyhow::{Context, ensure};
use ratatui::layout::{Position, Rect};
use std::time::{Duration, Instant};

const ANCHORS: usize = 20_000;
const PAYLOAD: usize = 8 * 1024 * 1024;
const MODELS: usize = 128;
// Conservatively covers original snapshot and returned preparation (each <=1MiB),
// all transient bounded controller/phase selection proofs, catalog and work state.
// Shared Rope content is charged separately below; this is not allocator/RSS accounting.
const WORK_RESERVATION: usize = 4 * 1024 * 1024;
// Four current-map projections, four possibly stale painted projections, one
// actual/phase snapshot. Full source lengths are overcounted even when shared.
const SOURCE_RETENTION: usize = 9 * crate::folding::MAX_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    member: Membership,
    epoch: u64,
    lifetime: Lifetime,
    generation: u64,
    selection: u64,
    top: usize,
    left: usize,
    tab: usize,
    cursor: usize,
    anchor: Option<usize>,
}
#[derive(Clone)]
pub(crate) struct PaneWindow {
    stamp: Stamp,
    groups: UiProof,
    pub text: Rect,
    pub gutter: Rect,
    pub window: Window,
}
#[derive(Default)]
pub(super) struct State {
    lane: Lane,
    owner: Option<UiProof>,
    interaction: u64,
    policies: Vec<(u64, bool)>,
    windows: [Option<PaneWindow>; 4],
    sealed: bool,
    failed: [Option<Stamp>; 4],
    input_blocked: bool,
    next: usize,
    reserved: bool,
    frame_scan_bytes: usize,
    inventory_refused: bool,
}
impl State {
    pub(super) fn retire(&mut self) {
        self.lane.retire();
        self.owner = None;
        self.sealed = false;
    }
}
impl App {
    pub fn folding_enabled(&self) -> bool {
        self.folding_inventory_admitted()
            && self.active_document().is_some()
            && self.settings.folding(self.language())
    }
    pub(super) fn folding_input(&mut self) {
        self.folding.lane.retire();
        self.folding.owner = None;
        if let Some(next) = self.folding.interaction.checked_add(1) {
            self.folding.interaction = next;
        } else {
            self.folding.input_blocked = true;
            self.message =
                "Folding interaction clock exhausted; Unfold All remains available".into();
        }
    }
    fn folding_inventory_admitted(&self) -> bool {
        self.documents.len() + self.hidden_documents.len() <= MODELS
    }
    pub(super) fn observe_folding(&mut self) -> bool {
        let mut changed = false;
        if !self.folding_inventory_admitted() {
            let changed = !self.folding.inventory_refused
                || self.folding.sealed
                || self.folding.lane.resolving();
            self.folding.retire();
            self.folding.inventory_refused = true;
            if changed {
                self.message =
                    "Folding model inventory exceeds128; source expanded and Unfold All remains available".into();
            }
            return changed;
        }
        if self.folding.inventory_refused {
            self.folding.inventory_refused = false;
            // Readmission is not authority to revive pre-refusal Ready maps.
            for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                doc.retain_visible_folding_rows(&[]);
            }
            changed = true;
        }
        self.folding.policies.retain(|(id, _)| {
            self.documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|doc| doc.id == *id)
        });
        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
            let language = doc
                .path
                .as_deref()
                .map_or("plaintext", crate::languages::language);
            let enabled = self.settings.folding(language);
            if let Some((_, previous)) = self
                .folding
                .policies
                .iter_mut()
                .find(|(id, _)| *id == doc.id)
            {
                if *previous != enabled {
                    *previous = enabled;
                    doc.retire_folding_policy();
                    changed = true;
                }
            } else {
                self.folding.policies.push((doc.id, enabled));
            }
            let mut ids = [0; 4];
            let mut count = 0;
            for group in self.editor_groups.groups() {
                if group.active().is_some_and(|tab| tab.document() == doc.id) && count < ids.len() {
                    ids[count] = group.id().value();
                    count += 1;
                }
            }
            doc.retain_visible_folding_rows(&ids[..count]);
        }
        if changed {
            self.folding.retire();
        }
        if self
            .folding
            .windows
            .iter()
            .flatten()
            .any(|pane| !self.folding_pane_current(pane))
        {
            changed |= self.folding.sealed;
            self.folding.sealed = false;
        }
        changed
    }
    fn folding_stamp(&self, member: Membership) -> Option<Stamp> {
        let doc = self
            .documents
            .iter()
            .find(|doc| doc.id == member.document)?;
        let (lifetime, generation, selection) = doc.folding_view_stamp(member.group.value())?;
        // Borrow exact metadata; do not clone a selection/private-state cohort.
        let (top, left) = doc.folding_viewport(member.group.value())?;
        let (cursor, anchor) = doc.folding_primary(member.group.value())?;
        Some(Stamp {
            member,
            epoch: doc.text_epoch(),
            lifetime,
            generation,
            selection,
            top,
            left,
            tab: doc.tab_size,
            cursor,
            anchor,
        })
    }
    fn folding_pane_current(&self, pane: &PaneWindow) -> bool {
        self.editor_groups.proof_current(&pane.groups)
            && self.editor_groups.membership_current(pane.stamp.member)
            && self.folding_stamp(pane.stamp.member).as_ref() == Some(&pane.stamp)
            && self
                .documents
                .iter()
                .find(|doc| doc.id == pane.stamp.member.document)
                .and_then(|doc| doc.current_folding_rows(pane.stamp.member.group.value()))
                .is_some_and(|rows| pane.window.same_projection(rows))
    }
    pub(crate) fn begin_folding_frame(&mut self) {
        self.folding.sealed = false;
        self.folding.windows = std::array::from_fn(|_| None);
        self.folding.frame_scan_bytes = 0;
    }
    pub(crate) fn seal_folding_frame(&mut self) {
        self.folding.sealed = self.folding_inventory_admitted()
            && self.editor_geometry_current()
            && self
                .folding
                .windows
                .iter()
                .flatten()
                .all(|pane| self.folding_pane_current(pane));
    }
    pub(crate) fn folding_window(&self) -> Option<&PaneWindow> {
        let member = self.active_tab_membership()?;
        self.folding
            .windows
            .iter()
            .flatten()
            .find(|pane| pane.stamp.member == member)
    }
    pub(crate) fn folding_window_for(&self, group: u64, document: u64) -> Option<&PaneWindow> {
        self.folding.windows.iter().flatten().find(|pane| {
            pane.stamp.member.group.value() == group && pane.stamp.member.document == document
        })
    }
    pub(crate) fn prepare_folding_window(
        &mut self,
        member: Membership,
        outer: Rect,
        text: Rect,
    ) -> Result<()> {
        if !self.folding_inventory_admitted() {
            self.folding.sealed = false;
            return Ok(());
        }
        if text.width == 0 || text.height == 0 {
            return Ok(());
        }
        let index = self
            .editor_groups
            .groups()
            .iter()
            .position(|group| group.id() == member.group)
            .context("Folding group retired")?;
        let doc_index = self
            .documents
            .iter()
            .position(|doc| doc.id == member.document)
            .context("Folding model retired")?;
        let Some(rows) = self.documents[doc_index]
            .current_folding_rows(member.group.value())
            .cloned()
        else {
            return Ok(());
        };
        if self.documents[doc_index].folding_desired_count(member.group.value()) == Some(0) {
            return Ok(());
        }
        let result = (|| -> Result<Window> {
            let doc = &mut self.documents[doc_index];
            let rows = rows.for_width(text.width)?;
            let (old_top, old_left) = doc
                .folding_viewport(member.group.value())
                .context("Folding view retired")?;
            let caret_bytes = doc.text.line(doc.row()).len_bytes();
            let prefix_bytes = self
                .folding
                .frame_scan_bytes
                .checked_add(caret_bytes)
                .context("Native pane caret prefix reservation exhausted")?;
            ensure!(
                prefix_bytes <= crate::display_window::MAX_PREPARED_BYTES,
                "Native pane caret prefix reservation exhausted"
            );
            ensure!(
                caret_bytes <= crate::display_window::MAX_LINE_BYTES,
                "Folding caret line exceeds window budget"
            );
            self.folding.frame_scan_bytes = prefix_bytes;
            let caret = rows
                .locate(doc.cursor, Affinity::Before)
                .context("Folding caret unavailable")?;
            ensure!(!caret.hidden, "Folding caret requires reveal");
            let caret_row = rows
                .ordinal(caret.row)
                .context("Folding caret row unavailable")?;
            let top_anchor = rows
                .visible_anchor(old_top.min(doc.line_count() - 1))
                .context("Folding top unavailable")?;
            let mut top = rows
                .ordinal(top_anchor)
                .context("Folding top row unavailable")?;
            if caret_row < top {
                top = caret_row;
            }
            if caret_row >= top + usize::from(text.height) {
                top = caret_row + 1 - usize::from(text.height);
            }
            let left = if caret.cell < old_left {
                caret.cell
            } else if caret.cell >= old_left + usize::from(text.width) {
                caret.cell + 1 - usize::from(text.width)
            } else {
                old_left
            };
            let anchor = rows.anchor(top).context("Folding viewport unavailable")?;
            let (anchors, maps, bytes) = self.folding_resources();
            ensure!(
                anchors <= ANCHORS && maps <= 4,
                "Native aggregate folding reservation exceeded"
            );
            let window_bytes: usize = self
                .folding
                .windows
                .iter()
                .flatten()
                .map(|pane| {
                    pane.window.allocated_payload() + pane.window.projection().allocated_payload()
                })
                .sum();
            let window_rows: usize = self
                .folding
                .windows
                .iter()
                .flatten()
                .map(|pane| pane.window.row_count())
                .sum();
            let window_cells: usize = self
                .folding
                .windows
                .iter()
                .flatten()
                .map(|pane| usize::from(pane.window.width()) * usize::from(pane.window.height()))
                .sum();
            ensure!(
                self.folding_source_bytes()
                    + rows.text().len_bytes()
                    + usize::from(self.folding.reserved) * crate::folding::MAX_BYTES
                    <= SOURCE_RETENTION,
                "Native folding retained-source reservation exceeded"
            );
            let budget = Budget {
                rows: crate::display_window::MAX_ROWS
                    .checked_sub(window_rows)
                    .context("Native pane row reservation exhausted")?,
                cells: crate::display_window::MAX_CELLS
                    .checked_sub(window_cells)
                    .context("Native pane cell reservation exhausted")?,
                scan_bytes: crate::display_window::MAX_PREPARED_BYTES
                    .checked_sub(self.folding.frame_scan_bytes)
                    .context("Native pane scan reservation exhausted")?,
                payload: PAYLOAD
                    .checked_sub(
                        bytes
                            + window_bytes
                            + rows.allocated_payload()
                            + usize::from(self.folding.reserved) * WORK_RESERVATION,
                    )
                    .context("Native pane payload reservation exhausted")?,
            };
            ensure!(
                usize::from(text.width) * usize::from(text.height) <= budget.cells,
                "Native pane cell reservation exhausted"
            );
            let count = usize::from(text.height).min(rows.row_count() - top);
            ensure!(
                count <= budget.rows,
                "Native pane row reservation exhausted"
            );
            // Length-only bounded row metadata preflight; no source characters
            // are segmented here. Charge before any failing window allocation.
            let mut scan = 0usize;
            for offset in 0..count {
                let row = rows
                    .anchor(top + offset)
                    .context("Folding viewport row unavailable")?;
                let bytes = rows.text().line(row.logical_line).len_bytes();
                ensure!(
                    bytes <= crate::display_window::MAX_LINE_BYTES,
                    "Folding viewport line exceeds window budget"
                );
                scan = scan
                    .checked_add(bytes)
                    .context("Native pane scan reservation overflow")?;
                ensure!(
                    scan <= budget.scan_bytes,
                    "Native pane scan reservation exhausted"
                );
            }
            self.folding.frame_scan_bytes += scan;
            let window = Window::prepare_with_budget(
                rows,
                anchor,
                left,
                text.width,
                text.height,
                Budget {
                    scan_bytes: scan,
                    ..budget
                },
            )?;
            let doc = &mut self.documents[doc_index];
            doc.top = anchor.logical_line;
            doc.left = left;
            Ok(window)
        })();
        match result {
            Ok(window) => {
                let stamp = self
                    .folding_stamp(member)
                    .context("Folding view changed during window preparation")?;
                self.folding.windows[index] = Some(PaneWindow {
                    stamp,
                    groups: self.editor_groups.proof(),
                    text,
                    gutter: Rect::new(outer.x, text.y, text.x.saturating_sub(outer.x), text.height),
                    window,
                });
                Ok(())
            }
            Err(error) => {
                self.folding.retire();
                self.documents[doc_index]
                    .prepare_clear_folding(member.group.value())?
                    .publish();
                self.message = format!("Folding expanded: {error:#}");
                Ok(())
            }
        }
    }
    fn folding_resources(&self) -> (usize, usize, usize) {
        self.documents
            .iter()
            .chain(&self.hidden_documents)
            .fold((0, 0, 0), |(a, m, b), doc| {
                let (x, y, z) = doc.folding_resources();
                (a + x, m + y, b + z)
            })
    }
    pub(super) fn execute_folding_command(&mut self, command: &str, args: Option<&Value>) -> bool {
        let action = match command {
            "editor.fold" => Some(FoldAction::Fold),
            "editor.unfold" => Some(FoldAction::Unfold),
            "editor.foldAll" => Some(FoldAction::FoldAll),
            "editor.unfoldAll" => None,
            _ => return false,
        };
        let result = (|| -> Result<()> {
            ensure!(
                args.is_none_or(
                    |value| value.is_null() || value.as_object().is_some_and(|map| map.is_empty())
                ),
                "Native folding supports no-argument commands only"
            );
            let member = self
                .active_tab_membership()
                .context("Open an editor to fold")?;
            if action.is_none() {
                self.folding_input();
                self.doc_mut()
                    .prepare_clear_folding(member.group.value())?
                    .publish();
                self.message = "Unfolded all · source unchanged".into();
                return Ok(());
            }
            ensure!(
                !self.folding.input_blocked,
                "Folding interaction clock exhausted"
            );
            ensure!(
                self.documents.len() + self.hidden_documents.len() <= MODELS,
                "Folding model inventory exceeds128"
            );
            ensure!(
                self.folding_enabled(),
                "Folding is disabled for this language"
            );
            let options = Options {
                wrap: Wrap::Off,
                width: self.editor_area.width.max(1),
                tab_size: self.doc().tab_size as u8,
            };
            self.check_folding_reservation()?;
            let interaction = self
                .folding
                .interaction
                .checked_add(1)
                .context("Folding interaction clock exhausted")?;
            let intent = self.doc().with_folding_current(
                member,
                interaction,
                |current| -> Result<Intent> {
                    Ok(Intent::DiscoverDocument {
                        view: ViewProof::capture(
                            current.model.clone(),
                            current.view.context("Folding view missing")?,
                        )?,
                        options,
                        action: action.unwrap(),
                    })
                },
            )??;
            self.folding.lane.request(intent, Instant::now())?;
            self.folding.interaction = interaction;
            self.folding.reserved = true;
            self.folding.owner = Some(self.editor_groups.proof());
            self.folding.failed = std::array::from_fn(|_| None);
            self.message = "Preparing native indentation folds…".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.message = format!("Folding rejected: {error:#}");
        }
        self.observe_folding();
        self.invalidate_editor_presentation();
        true
    }
    fn request_header_unfold(&mut self, member: Membership, header: usize) -> Result<()> {
        ensure!(
            !self.folding.input_blocked && self.folding_enabled(),
            "Folding is unavailable"
        );
        let options = Options {
            wrap: Wrap::Off,
            width: self.editor_area.width.max(1),
            tab_size: self.doc().tab_size as u8,
        };
        self.check_folding_reservation()?;
        let interaction = self
            .folding
            .interaction
            .checked_add(1)
            .context("Folding interaction clock exhausted")?;
        let intent = self.doc().with_folding_current(
            member,
            interaction,
            |current| -> Result<Intent> {
                Ok(Intent::DiscoverDocument {
                    view: ViewProof::capture(
                        current.model.clone(),
                        current.view.context("Folding view missing")?,
                    )?,
                    options,
                    action: FoldAction::UnfoldHeader(header),
                })
            },
        )??;
        self.folding.lane.request(intent, Instant::now())?;
        self.folding.interaction = interaction;
        self.folding.reserved = true;
        self.folding.owner = Some(self.editor_groups.proof());
        self.message = "Preparing native header unfold…".into();
        Ok(())
    }
    fn folding_source_bytes(&self) -> usize {
        self.documents
            .iter()
            .chain(&self.hidden_documents)
            .map(Document::folding_source_bytes)
            .sum::<usize>()
            + self
                .folding
                .windows
                .iter()
                .flatten()
                .map(|pane| pane.window.projection().text().len_bytes())
                .sum::<usize>()
    }
    fn check_folding_reservation(&self) -> Result<()> {
        ensure!(
            self.folding_inventory_admitted(),
            "Folding model inventory exceeds128"
        );
        let (anchors, maps, bytes) = self.folding_resources();
        ensure!(
            anchors <= ANCHORS && maps <= 4,
            "Native aggregate folding reservation exceeded"
        );
        let windows: usize = self
            .folding
            .windows
            .iter()
            .flatten()
            .map(|pane| {
                pane.window.allocated_payload() + pane.window.projection().allocated_payload()
            })
            .sum();
        ensure!(
            bytes + windows + WORK_RESERVATION <= PAYLOAD,
            "Folding worker aggregate reservation exceeded; Unfold All remains available"
        );
        ensure!(
            self.folding_source_bytes() + crate::folding::MAX_BYTES <= SOURCE_RETENTION,
            "Folding retained-source reservation exceeded; Unfold All remains available"
        );
        Ok(())
    }
    /// Folding admission constrains copied fold payload, never ordinary splitting.
    /// A refused/disabled source starts the destination expanded with no intent.
    pub(super) fn copy_split_folding_intent(&self, source: Membership) -> bool {
        self.folding_inventory_admitted()
            && self.editor_groups.membership_current(source)
            && self
                .documents
                .iter()
                .find(|doc| doc.id == source.document)
                .is_some_and(|doc| {
                    let language = doc
                        .path
                        .as_deref()
                        .map_or("plaintext", crate::languages::language);
                    self.settings.folding(language)
                        && doc
                            .folding_desired_count(source.group.value())
                            .is_some_and(|count| count != 0)
                })
    }
    pub(super) fn check_folding_view_copy_reservation(&self, source: Membership) -> Result<()> {
        ensure!(
            self.folding_inventory_admitted(),
            "Split folding model inventory exceeds128"
        );
        ensure!(
            self.editor_groups.membership_current(source),
            "Split folding source membership retired"
        );
        let doc = self
            .documents
            .iter()
            .find(|doc| doc.id == source.document)
            .context("Split folding model retired")?;
        let (extra_anchors, extra_payload) =
            doc.expanded_folding_copy_metadata(source.group.value())?;
        let (anchors, maps, bytes) = self.folding_resources();
        let windows: usize = self
            .folding
            .windows
            .iter()
            .flatten()
            .map(|pane| {
                pane.window.allocated_payload() + pane.window.projection().allocated_payload()
            })
            .sum();
        ensure!(
            anchors + extra_anchors <= ANCHORS && maps <= 4,
            "Split folding aggregate anchor/map budget exceeded"
        );
        ensure!(
            bytes + windows + extra_payload + usize::from(self.folding.reserved) * WORK_RESERVATION
                <= PAYLOAD,
            "Split folding aggregate metadata budget exceeded"
        );
        ensure!(
            self.folding_source_bytes()
                + usize::from(self.folding.reserved) * crate::folding::MAX_BYTES
                <= SOURCE_RETENTION,
            "Split folding retained-source budget exceeded"
        );
        Ok(())
    }
    fn mark_folding_failed(&mut self, member: Membership) {
        if let Some(slot) = self
            .editor_groups
            .groups()
            .iter()
            .position(|group| group.id() == member.group)
        {
            self.folding.failed[slot] = self.folding_stamp(member);
        }
    }
    pub(super) fn poll_folding(&mut self) -> bool {
        let mut changed = self.observe_folding();
        if self
            .folding
            .owner
            .as_ref()
            .is_some_and(|proof| !self.editor_groups.proof_current(proof))
        {
            self.folding.retire();
        }
        let mut observed_member = None;
        let origin = self
            .folding
            .lane
            .interest()
            .map(|intent| intent.model().document);
        if let Some(id) = origin {
            let Some(index) = self.documents.iter().position(|doc| doc.id == id) else {
                self.folding.retire();
                return self.folding.lane.poll_retired() || changed;
            };
            let member = match self.folding.lane.interest() {
                Some(
                    Intent::DiscoverDocument { view, .. } | Intent::DocumentPrepare { view, .. },
                ) => view.membership,
                _ => {
                    self.folding.retire();
                    return self.folding.lane.poll_retired() || changed;
                }
            };
            if !self.editor_groups.membership_current(member) {
                self.folding.retire();
                return self.folding.lane.poll_retired() || changed;
            }
            observed_member = Some(member);
            let interaction = self.folding.interaction;
            let result =
                self.documents[index].with_folding_current(member, interaction, |current| {
                    self.folding.lane.poll(&current, Instant::now())
                });
            match result {
                Err(_) => {
                    self.mark_folding_failed(member);
                    self.folding.retire();
                }
                Ok(Some(outcome)) => {
                    changed = true;
                    let announce = matches!(
                        &outcome.intent,
                        Intent::DocumentPrepare {
                            operation: DocumentWork::Action { .. },
                            ..
                        }
                    );
                    match &outcome.result {
                        Ok(Prepared::Catalog(_)) => {
                            if let Ok(phase) = outcome.document_phase() {
                                let advanced = self.documents[index].with_folding_current(
                                    member,
                                    interaction,
                                    |current| {
                                        self.folding.lane.advance_document_phase(
                                            phase,
                                            &current,
                                            Instant::now(),
                                        )
                                    },
                                );
                                if !matches!(advanced, Ok(Ok(true))) {
                                    self.mark_folding_failed(member);
                                    self.folding.retire();
                                }
                            }
                        }
                        _ => {
                            match outcome.result {
                                Ok(Prepared::Document(prepared)) => {
                                    let (anchors, _, bytes) = self.folding_resources();
                                    let prior = self.documents[index]
                                        .folding_desired_count(member.group.value())
                                        .unwrap_or(0);
                                    let accepted = anchors.saturating_sub(prior)
                                        + prepared.desired_count()
                                        <= ANCHORS
                                        && bytes
                                            + prepared.allocated_payload()
                                            + self
                                                .folding
                                                .windows
                                                .iter()
                                                .flatten()
                                                .map(|pane| {
                                                    pane.window.allocated_payload()
                                                        + pane
                                                            .window
                                                            .projection()
                                                            .allocated_payload()
                                                })
                                                .sum::<usize>()
                                            + usize::from(self.folding.reserved) * WORK_RESERVATION
                                            <= PAYLOAD
                                        && self.folding_source_bytes()
                                            + prepared.rows().text().len_bytes()
                                            + usize::from(self.folding.reserved)
                                                * crate::folding::MAX_BYTES
                                            <= SOURCE_RETENTION;
                                    let skipped = prepared.skipped_regions();
                                    let count = prepared.desired_count();
                                    if accepted {
                                        match self.documents[index]
                                            .prepare_fold_publication(prepared)
                                        {
                                            Ok(lease) => {
                                                lease.publish();
                                                if announce {
                                                    self.message = format!(
                                                        "Native folds ready · {count} collapsed · {skipped} protected regions skipped"
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                self.message =
                                                    format!("Fold result retired: {error:#}");
                                                self.mark_folding_failed(member);
                                            }
                                        }
                                    } else {
                                        self.message = "Native folding aggregate budget exceeded; source unchanged".into();
                                        self.mark_folding_failed(member);
                                    }
                                }
                                Err(error) => {
                                    self.message = format!("Folding unavailable: {error}");
                                    if let Some(slot) = self
                                        .editor_groups
                                        .groups()
                                        .iter()
                                        .position(|group| group.id() == member.group)
                                    {
                                        self.folding.failed[slot] = self.folding_stamp(member);
                                    }
                                }
                                _ => self.message = "Unexpected folding result retired".into(),
                            }
                            self.folding.owner = None;
                        }
                    }
                }
                Ok(None) => {}
            }
            if !self.folding.lane.occupied() && self.folding.lane.desired().is_some() {
                let ready = self.folding.lane.document_snapshot_ready();
                let dispatched = if ready {
                    self.documents[index].with_folding_current(member, interaction, |current| {
                        self.folding
                            .lane
                            .dispatch_ready_document(&current, Instant::now())
                    })
                } else {
                    let options = match self.folding.lane.desired().unwrap() {
                        Intent::DiscoverDocument { options, .. }
                        | Intent::DocumentPrepare { options, .. } => *options,
                        _ => return changed,
                    };
                    match self.documents[index].capture_folding(member.group.value(), options) {
                        Ok(snapshot) => self.documents[index].with_folding_current(
                            member,
                            interaction,
                            |current| {
                                self.folding.lane.dispatch_document(
                                    &current,
                                    snapshot,
                                    Instant::now(),
                                )
                            },
                        ),
                        Err(error) => {
                            self.message = format!("Folding unavailable: {error:#}");
                            self.mark_folding_failed(member);
                            self.folding.retire();
                            return true;
                        }
                    }
                };
                if let Err(error) = dispatched.and_then(|result| result) {
                    self.message = format!("Folding unavailable: {error:#}");
                    self.mark_folding_failed(member);
                    self.folding.retire();
                    changed = true;
                }
            }
        } else {
            changed |= self.folding.lane.poll_retired();
        }
        if let Some(notice) = self.folding.lane.take_notice() {
            if notice.contains("expired")
                && let Some(member) = observed_member
                && let Some(slot) = self
                    .editor_groups
                    .groups()
                    .iter()
                    .position(|group| group.id() == member.group)
            {
                self.folding.failed[slot] = self.folding_stamp(member);
            }
            self.message = notice.into();
            changed = true;
        }
        // A canceled/expired actual worker still owns its original snapshot and
        // possible returned result. Never release the reservation on logical
        // retirement, phase handoff or a result before the thread positively exits.
        if !self.folding.lane.occupied() && !self.folding.lane.resolving() {
            self.folding.reserved = false;
        }
        // Fairly refresh visible per-view intent on the same actual lane.
        // No automatic retries of the same refused source/view proof.
        if self.folding_inventory_admitted()
            && !self.folding.input_blocked
            && !self.folding.lane.resolving()
            && !self.folding.lane.occupied()
        {
            for offset in 0..4 {
                let slot = (self.folding.next + offset) % 4;
                let Some(member) = self.editor_groups.groups().get(slot).and_then(|group| {
                    group.active().map(|tab| Membership {
                        group: group.id(),
                        tab: tab.id(),
                        document: tab.document(),
                    })
                }) else {
                    continue;
                };
                let Some(index) = self
                    .documents
                    .iter()
                    .position(|doc| doc.id == member.document)
                else {
                    continue;
                };
                let doc = &self.documents[index];
                let language = doc
                    .path
                    .as_deref()
                    .map_or("plaintext", crate::languages::language);
                let stamp = self.folding_stamp(member);
                if !self.settings.folding(language)
                    || doc
                        .folding_desired_count(member.group.value())
                        .is_none_or(|n| n == 0)
                    || doc.current_folding_rows(member.group.value()).is_some()
                    || self.folding.failed[slot] == stamp
                {
                    continue;
                }
                if self.check_folding_reservation().is_err() {
                    self.folding.failed[slot] = stamp;
                    self.message =
                        "Folding worker aggregate reservation exceeded; source expanded".into();
                    changed = true;
                    continue;
                }
                let doc = &self.documents[index];
                let options = Options {
                    wrap: Wrap::Off,
                    width: self.editor_area.width.max(1),
                    tab_size: doc.tab_size as u8,
                };
                let interaction = self.folding.interaction;
                if let Ok(Ok(intent)) =
                    doc.with_folding_current(member, interaction, |current| -> Result<Intent> {
                        Ok(Intent::DocumentPrepare {
                            view: ViewProof::capture(
                                current.model.clone(),
                                current.view.context("Folding view missing")?,
                            )?,
                            options,
                            operation: DocumentWork::Existing,
                        })
                    })
                {
                    if self.folding.lane.request(intent, Instant::now()).is_ok() {
                        self.folding.reserved = true;
                        self.folding.owner = Some(self.editor_groups.proof());
                        self.folding.next = (slot + 1) % 4;
                        changed = true;
                        break;
                    } else {
                        self.folding.failed[slot] = stamp;
                        self.message = "Folding preparation unavailable; source expanded".into();
                        changed = true;
                    }
                } else {
                    self.folding.failed[slot] = stamp;
                    self.message = "Folding view unavailable; source expanded".into();
                    changed = true;
                }
            }
        }
        if changed {
            self.invalidate_editor_presentation();
        }
        changed
    }
    pub fn settle_folding(&mut self) -> Result<()> {
        self.folding.retire();
        let result = self.folding.lane.shutdown(Duration::from_secs(1));
        if !self.folding.lane.occupied() {
            self.folding.reserved = false;
        }
        result
    }
    pub(super) fn folding_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> bool {
        let point = Position::new(mouse.column, mouse.row);
        let Some(pane) = self
            .folding
            .windows
            .iter()
            .flatten()
            .find(|pane| pane.text.contains(point) || pane.gutter.contains(point))
            .cloned()
        else {
            return false;
        };
        if !self.folding.sealed
            || !self.editor_geometry_current()
            || !self.folding_pane_current(&pane)
        {
            self.message = "Folded viewport changed; repaint before selecting source".into();
            return true;
        }
        if pane.gutter.contains(point) {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && mouse.column == pane.text.x.saturating_sub(2)
                && let Some(row) = pane.window.row(usize::from(mouse.row - pane.gutter.y))
                && row.folded_body.is_some()
            {
                let header = row.anchor.logical_line;
                if self.editor_groups.active_membership() != Some(pane.stamp.member)
                    && let Err(error) = self.focus_tab(pane.stamp.member)
                {
                    self.message = format!("Fold header focus rejected: {error:#}");
                    return true;
                }
                if let Err(error) = self.request_header_unfold(pane.stamp.member, header) {
                    self.message = format!("Unfold header rejected: {error:#}");
                }
            }
            return true;
        } // A gutter is never a source hit.
        let row = usize::from(mouse.row - pane.text.y);
        let column = usize::from(mouse.column - pane.text.x);
        let hit = pane.window.hit(row, column, Affinity::Before);
        let scroll = match mouse.kind {
            MouseEventKind::ScrollDown => Some(3),
            MouseEventKind::ScrollUp => Some(-3),
            _ => None,
        };
        if let Err(error) = self.focus_tab(pane.stamp.member) {
            self.message = format!("Folded focus rejected: {error:#}");
            return true;
        }
        if let Some(amount) = scroll {
            self.move_folded(amount, false, false);
            return true;
        }
        if let Some(position) = hit
            && matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
            )
        {
            self.focus = Focus::Editor;
            if mouse.modifiers.contains(KeyModifiers::ALT)
                && matches!(mouse.kind, MouseEventKind::Down(_))
            {
                self.doc_mut().add_cursor(position);
            } else {
                self.doc_mut().secondary.clear();
                self.doc_mut().move_to(
                    position,
                    mouse.modifiers.contains(KeyModifiers::SHIFT)
                        || matches!(mouse.kind, MouseEventKind::Drag(_)),
                );
            }
        }
        true
    }
    pub(crate) fn folded_caret(&self) -> Option<(u16, u16)> {
        let pane = self.folding_window()?;
        let caret = pane.window.locate(self.doc().cursor, Affinity::Before)?;
        Some((
            pane.text.x + caret.column as u16,
            pane.text.y + caret.row as u16,
        ))
    }
    pub(super) fn move_folded(&mut self, amount: isize, select: bool, add: bool) -> bool {
        if !self.folding_inventory_admitted() {
            return false;
        }
        let Some(member) = self.active_tab_membership() else {
            return false;
        };
        let Some(rows) = self
            .doc()
            .current_folding_rows(member.group.value())
            .cloned()
        else {
            return false;
        };
        if self.doc().folding_desired_count(member.group.value()) == Some(0) {
            return false;
        }
        let result = (|| -> Result<Vec<Selection>> {
            ensure!(
                self.doc().secondary.len() < crate::folding_worker::MAX_SELECTIONS,
                "Folded movement exceeds selection budget"
            );
            let mut selections = self.doc().selections();
            let mut line_bytes = 0usize;
            for selection in &mut selections {
                let current_row = self.doc().text.char_to_line(selection.cursor);
                let current_line = self.doc().text.line(current_row).len_bytes();
                line_bytes += current_line;
                ensure!(
                    current_line <= crate::display_window::MAX_LINE_BYTES
                        && line_bytes <= crate::display_window::MAX_PREPARED_BYTES,
                    "Folded movement line budget exceeded"
                );
                let at = rows
                    .locate(selection.cursor, Affinity::Before)
                    .context("Folded movement origin invalid")?;
                ensure!(!at.hidden, "Folded movement requires reveal");
                let target = rows
                    .advance(at.row, amount)
                    .context("Folded movement target invalid")?;
                let bytes = self.doc().text.line(target.logical_line).len_bytes();
                line_bytes += bytes;
                ensure!(
                    bytes <= crate::display_window::MAX_LINE_BYTES
                        && line_bytes <= crate::display_window::MAX_PREPARED_BYTES,
                    "Folded movement target budget exceeded"
                );
                let column = selection.desired_column.unwrap_or(at.cell);
                let next = rows
                    .hit(target, column, Affinity::Before)
                    .context("Folded movement target unavailable")?;
                if select && selection.anchor.is_none() {
                    selection.anchor = Some(selection.cursor);
                }
                if !select {
                    selection.anchor = None;
                }
                selection.cursor = next;
                selection.desired_column = Some(column);
            }
            if add {
                let old = self.doc().selections();
                selections.extend(old);
                ensure!(
                    selections.len() <= crate::folding_worker::MAX_SELECTIONS,
                    "Folded cursor admission exceeds selection budget"
                );
            }
            Ok(selections)
        })();
        match result {
            Ok(selections) => self.doc_mut().apply_display_selections(selections),
            Err(error) => self.message = format!("Folded movement rejected: {error:#}"),
        }
        true
    }
}

#[cfg(test)]
mod tests;
