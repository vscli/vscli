//! Per-view fold intent and checked immutable publication. Text history never
//! owns this state. Preparation methods are worker-only, with no filesystem IO.
use super::{Document, Selection, ViewState};
use crate::{
    display_rows::{DisplayRows, Options, Wrap},
    editor_groups::Membership,
    folding::{self, Region},
    folding_controller::{Current, CurrentView, Lifetime, ModelProof},
};
use anyhow::{Result, ensure};
use ropey::Rope;
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

const MAX_VIEWS: usize = 5; // Baseline plus the four native group views.
const MAX_SELECTIONS: usize = 10_000;
const MAX_JOURNAL: usize = 256;
const MAX_DOCUMENT_ANCHORS: usize = 20_000;

#[derive(Clone, Debug)]
struct Ready {
    epoch: u64,
    policy: Lifetime,
    rows: Arc<DisplayRows>,
    hidden: Arc<[Range<usize>]>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct View {
    lifetime: Lifetime,
    generation: u64,
    selection_generation: u64,
    blocked: bool,
    desired: Arc<[Region]>,
    desired_epoch: u64,
    ready: Option<Ready>,
}
impl View {
    fn invalidate(&mut self) {
        self.ready = None;
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.blocked = true;
            self.desired = Arc::from([]);
        }
    }
    fn fresh_copy(&self) -> Self {
        Self {
            lifetime: Lifetime::default(),
            generation: 0,
            selection_generation: 0,
            blocked: false,
            ..self.clone()
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Change {
    epoch: u64,
    range: Range<usize>,
    added: usize,
}
#[derive(Clone, Debug)]
struct Proof {
    document: u64,
    view: u64,
    lifetime: Lifetime,
    policy: Lifetime,
    epoch: u64,
    generation: u64,
    selection_generation: u64,
    options: Options,
    primary: Selection,
    secondary: Vec<Selection>,
}
/// Immutable source and exact originating view. Constructed only by Document;
/// preparation may scan bounded data and belongs on the actual folding worker.
#[derive(Clone, Debug)]
pub struct FoldSnapshot {
    proof: Proof,
    text: Rope,
    desired: Arc<[Region]>,
    changes: Vec<Change>,
}
/// Unforgeable checked worker result; publication still verifies live ownership.
#[derive(Debug)]
pub struct FoldPrepared {
    proof: Proof,
    desired: Arc<[Region]>,
    rows: Arc<DisplayRows>,
    hidden: Arc<[Range<usize>]>,
    skipped: usize,
}
/// Exclusive publication lease. Dropping it leaves all document state intact.
pub struct FoldCommit<'a> {
    document: &'a mut Document,
    prepared: FoldPrepared,
    next_generation: u64,
}
/// Native finite worker actions. No original command arguments or syntax ranges
/// are implied. Full selection protection is part of preparation, not relocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldAction {
    Fold,
    Unfold,
    FoldAll,
}

/// A scan-free clear lease, including generation-exhaustion recovery. Clearing
/// always creates a fresh private lifetime, so a no-op cannot revive old replies.
pub struct FoldClear<'a> {
    document: &'a mut Document,
    view: u64,
    state: View,
}

/// Explicit split or unseen shared-view insertion, staged without activating it.
/// A copied split receives a fresh private lifetime. Ordinary insertion expands.
pub struct FoldViewInsertion<'a> {
    document: &'a mut Document,
    target: u64,
    state: ViewState,
}

fn primary(view: &ViewState) -> Selection {
    Selection {
        cursor: view.cursor,
        anchor: view.anchor,
        desired_column: view.desired_column,
    }
}
fn valid_selection(selection: &Selection, len: usize) -> bool {
    selection.cursor <= len && selection.anchor.is_none_or(|p| p <= len)
}
fn protected(hidden: &[Range<usize>], selected: Range<usize>) -> bool {
    // Selection endpoints are inclusive; hidden scalar spans are half open.
    let index = hidden.partition_point(|span| span.end <= selected.start);
    hidden
        .get(index)
        .is_some_and(|span| span.start <= selected.end)
}
fn view_safe(view: &ViewState, hidden: &[Range<usize>], len: usize) -> bool {
    if view.secondary.len() >= MAX_SELECTIONS {
        return false;
    }
    let first = primary(view);
    valid_selection(&first, len)
        && !protected(hidden, first.range())
        && view
            .secondary
            .iter()
            .all(|s| valid_selection(s, len) && !protected(hidden, s.range()))
}
fn guard(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "Folding preparation cancelled"
    );
    ensure!(
        Instant::now() < deadline,
        "Folding preparation deadline exceeded"
    );
    Ok(())
}

impl FoldSnapshot {
    pub fn document_id(&self) -> u64 {
        self.proof.document
    }
    pub fn view_id(&self) -> u64 {
        self.proof.view
    }
    pub fn text_epoch(&self) -> u64 {
        self.proof.epoch
    }
    pub fn options(&self) -> Options {
        self.proof.options
    }
    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// Replay exact scalar changes and remove touched or selection-hidden intent.
    /// Undo is another forward epoch/change and can never resurrect removed intent.
    pub fn prepare_existing(self, cancel: &AtomicBool, deadline: Instant) -> Result<FoldPrepared> {
        let desired = self.map_desired(cancel, deadline)?;
        self.finish(desired, true, cancel, deadline)
    }
    pub fn is_current(&self, current: &Current<'_>) -> bool {
        self.proof.document == current.model.document
            && self.proof.epoch == current.model.text_epoch
            && self.proof.policy == current.model.options
            && self.proof.options.tab_size == current.model.tab_size
            && current.view.as_ref().is_some_and(|view| {
                view.membership.document == self.proof.document
                    && view.membership.group.value() == self.proof.view
                    && *view.lifetime == self.proof.lifetime
                    && view.generation == self.proof.generation
                    && view.selection_generation == self.proof.selection_generation
                    && *view.primary == self.proof.primary
                    && view.secondary == self.proof.secondary.as_slice()
            })
    }
    pub(crate) fn validate_worker(&self) -> Result<()> {
        ensure!(
            self.text.len_bytes() <= folding::MAX_BYTES
                && self.text.len_lines() <= folding::MAX_LINES,
            "Folding source exceeds worker limits"
        );
        ensure!(
            self.proof.options.wrap == Wrap::Off && (1..=16).contains(&self.proof.options.tab_size),
            "Folding snapshot options are invalid"
        );
        ensure!(
            self.proof.secondary.len() < MAX_SELECTIONS
                && self.desired.len() <= folding::MAX_REGIONS
                && self.changes.len() <= MAX_JOURNAL,
            "Folding snapshot metadata exceeds worker limits"
        );
        Ok(())
    }
    fn map_desired(&self, cancel: &AtomicBool, deadline: Instant) -> Result<Vec<Region>> {
        guard(cancel, deadline)?;
        let mut desired = self.desired.to_vec();
        for change in &self.changes {
            guard(cancel, deadline)?;
            let mut mapped = Vec::with_capacity(desired.len());
            for region in desired {
                guard(cancel, deadline)?;
                if let Some(region) = region.map_edit(change.range.clone(), change.added) {
                    mapped.push(region);
                }
            }
            desired = mapped;
        }
        Ok(desired)
    }
    /// Catalog targeting runs on the retained worker. Per-selection nesting is
    /// capped at the foundation's depth; no regions × selections Cartesian walk.
    /// Fold/Unfold operate simultaneously on the original collapsed mask.
    pub fn prepare_action(
        self,
        action: FoldAction,
        catalog: Arc<[Region]>,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<FoldPrepared> {
        guard(cancel, deadline)?;
        ensure!(
            catalog.len() <= folding::MAX_REGIONS,
            "Folding catalog exceeds 5,000 regions"
        );
        folding::RowMap::new(&self.text, &catalog)?;
        if action == FoldAction::FoldAll {
            return self.finish(catalog.to_vec(), true, cancel, deadline);
        }
        let mapped = self.map_desired(cancel, deadline)?;
        let existing = self.clone().finish(mapped, true, cancel, deadline)?;
        let mut catalog = catalog.to_vec();
        catalog.sort_unstable_by_key(|r| {
            (r.characters().start, std::cmp::Reverse(r.characters().end))
        });
        let keys: Vec<_> = catalog
            .iter()
            .map(|r| {
                let c = r.characters();
                (c.start, std::cmp::Reverse(c.end))
            })
            .collect();
        let mut mask = vec![false; catalog.len()];
        for region in existing.desired.iter() {
            guard(cancel, deadline)?;
            let c = region.characters();
            if let Ok(index) = keys.binary_search(&(c.start, std::cmp::Reverse(c.end))) {
                mask[index] = true;
            }
        }
        let mut positions: Vec<_> = std::iter::once(&self.proof.primary)
            .chain(&self.proof.secondary)
            .map(|s| {
                self.text
                    .line_to_char(self.text.char_to_line(s.range().start))
            })
            .collect();
        positions.sort_unstable();
        positions.dedup();
        let mut changed = vec![false; catalog.len()];
        let mut stack: Vec<usize> = Vec::new();
        let mut next = 0;
        for position in positions {
            guard(cancel, deadline)?;
            while next < catalog.len() && catalog[next].characters().start <= position {
                let start = catalog[next].characters().start;
                while stack
                    .last()
                    .is_some_and(|&index| catalog[index].characters().end <= start)
                {
                    stack.pop();
                }
                ensure!(
                    stack.len() < folding::MAX_DEPTH,
                    "Folding catalog nesting exceeds limit"
                );
                stack.push(next);
                next += 1;
            }
            while stack
                .last()
                .is_some_and(|&index| catalog[index].characters().end <= position)
            {
                stack.pop();
            }
            for &index in stack.iter().rev() {
                guard(cancel, deadline)?;
                if mask[index] == (action == FoldAction::Unfold) {
                    changed[index] = true;
                    break;
                }
            }
        }
        // Preserve valid mapped intent absent from the current indentation catalog;
        // target only the catalog members named by this finite action.
        let mut desired = existing.desired.to_vec();
        if action == FoldAction::Fold {
            for (index, region) in catalog.into_iter().enumerate() {
                guard(cancel, deadline)?;
                if changed[index] {
                    desired.push(region);
                }
            }
        } else {
            let mut retained = Vec::with_capacity(desired.len());
            for region in desired {
                guard(cancel, deadline)?;
                let c = region.characters();
                let keep = match keys.binary_search(&(c.start, std::cmp::Reverse(c.end))) {
                    Ok(index) => !changed[index],
                    Err(_) => true,
                };
                if keep {
                    retained.push(region);
                }
            }
            desired = retained;
        }
        self.finish(desired, false, cancel, deadline)
    }

    /// Explicit collapse is all-or-nothing, including malformed late regions and
    /// protected selections. No caller cursor is relocated to make it succeed.
    pub fn prepare_collapsed(
        self,
        desired: Arc<[Region]>,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<FoldPrepared> {
        ensure!(
            desired.len() <= folding::MAX_REGIONS,
            "Folding exceeds 5,000 regions"
        );
        guard(cancel, deadline)?;
        self.finish(desired.to_vec(), false, cancel, deadline)
    }
    fn finish(
        self,
        mut desired: Vec<Region>,
        reveal: bool,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<FoldPrepared> {
        guard(cancel, deadline)?;
        // Validate the ENTIRE proposed tree before dropping any protected region.
        folding::RowMap::new(&self.text, &desired)?;
        desired.sort_unstable_by_key(|r| {
            (r.characters().start, std::cmp::Reverse(r.characters().end))
        });
        let mut selections = Vec::with_capacity(self.proof.secondary.len() + 1);
        selections.push(self.proof.primary.range());
        selections.extend(self.proof.secondary.iter().map(Selection::range));
        selections.sort_unstable_by_key(|r| (r.start, r.end));
        let mut selected: Vec<Range<usize>> = Vec::with_capacity(selections.len());
        for range in selections {
            guard(cancel, deadline)?;
            if let Some(previous) = selected.last_mut()
                && range.start <= previous.end
            {
                previous.end = previous.end.max(range.end);
            } else {
                selected.push(range);
            }
        }
        let proposed_count = desired.len();
        let mut accepted = Vec::with_capacity(desired.len());
        let mut hidden: Vec<Range<usize>> = Vec::with_capacity(desired.len());
        for region in desired {
            guard(cancel, deadline)?;
            let chars = region.characters();
            let span = self
                .text
                .line_to_char(self.text.char_to_line(chars.start) + 1)
                ..chars.end;
            let index = selected.partition_point(|r| r.end < span.start);
            let conceals = selected.get(index).is_some_and(|r| r.start < span.end);
            if conceals {
                ensure!(reveal, "Cannot collapse a region containing a selection");
                continue;
            }
            if let Some(previous) = hidden.last_mut()
                && span.start <= previous.end
            {
                previous.end = previous.end.max(span.end);
            } else {
                hidden.push(span);
            }
            accepted.push(region);
        }
        guard(cancel, deadline)?;
        let rows = DisplayRows::prepare_folded(self.text, self.proof.options, &accepted)?;
        guard(cancel, deadline)?;
        Ok(FoldPrepared {
            proof: self.proof,
            skipped: proposed_count - accepted.len(),
            desired: accepted.into(),
            rows: Arc::new(rows),
            hidden: hidden.into(),
        })
    }
}

impl FoldPrepared {
    pub fn skipped_regions(&self) -> usize {
        self.skipped
    }
    pub fn rows(&self) -> &DisplayRows {
        &self.rows
    }
    pub fn desired_count(&self) -> usize {
        self.desired.len()
    }
}
impl Document {
    /// This validates model/view ownership, not live Groups membership. App
    /// validates Groups/UI before and after using this borrowed bridge.
    pub fn with_folding_current<T>(
        &self,
        membership: Membership,
        interaction: u64,
        inspect: impl FnOnce(Current<'_>) -> T,
    ) -> Result<T> {
        ensure!(
            membership.document == self.id,
            "Folding membership belongs to another document"
        );
        let view = self
            .exact_fold_view(membership.group.value())
            .ok_or_else(|| anyhow::anyhow!("Folding view retired"))?;
        ensure!(
            !view.folding.blocked && view.secondary.len() < MAX_SELECTIONS,
            "Folding view is blocked or exceeds selection limits"
        );
        ensure!(
            (1..=16).contains(&self.tab_size),
            "Folding tab size is invalid"
        );
        let primary = primary(view);
        let model = ModelProof {
            document: self.id,
            text_epoch: self.text_epoch,
            options: self.folding_policy.clone(),
            tab_size: self.tab_size as u8,
        };
        Ok(inspect(Current {
            model: &model,
            view: Some(CurrentView {
                membership,
                lifetime: &view.folding.lifetime,
                generation: view.folding.generation,
                selection_generation: view.folding.selection_generation,
                interaction,
                primary: &primary,
                secondary: &view.secondary,
            }),
        }))
    }
    pub fn check_folding_snapshot(&self, snapshot: &FoldSnapshot) -> Result<()> {
        let proof = &snapshot.proof;
        ensure!(
            self.id == proof.document
                && self.text_epoch == proof.epoch
                && self.folding_policy == proof.policy
                && self.tab_size == usize::from(proof.options.tab_size),
            "Folding source changed"
        );
        let view = self
            .exact_fold_view(proof.view)
            .ok_or_else(|| anyhow::anyhow!("Folding view retired"))?;
        ensure!(
            !view.folding.blocked
                && view.folding.lifetime == proof.lifetime
                && view.folding.generation == proof.generation
                && view.folding.selection_generation == proof.selection_generation
                && primary(view) == proof.primary
                && view.secondary == proof.secondary,
            "Folding view changed"
        );
        Ok(())
    }
    /// No source-byte, region or selection traversal. Always renew lifetime even
    /// if already expanded, and recover an exhausted fold clock with a fresh one.
    pub fn prepare_clear_folding(&mut self, id: u64) -> Result<FoldClear<'_>> {
        let view = self
            .exact_fold_view(id)
            .ok_or_else(|| anyhow::anyhow!("Folding view retired"))?;
        let state = View {
            generation: view.folding.generation.checked_add(1).unwrap_or(0),
            desired_epoch: self.text_epoch,
            ..View::default()
        };
        Ok(FoldClear {
            document: self,
            view: id,
            state,
        })
    }
    fn exact_fold_view(&self, id: u64) -> Option<&ViewState> {
        if id == self.active_view {
            Some(&self.view)
        } else {
            self.other_views.get(&id)
        }
    }
    fn exact_fold_view_mut(&mut self, id: u64) -> Option<&mut ViewState> {
        if id == self.active_view {
            Some(&mut self.view)
        } else {
            self.other_views.get_mut(&id)
        }
    }
    pub fn folding_policy_lifetime(&self) -> &Lifetime {
        &self.folding_policy
    }
    pub fn folding_view_lifetime(&self, id: u64) -> Option<&Lifetime> {
        Some(&self.exact_fold_view(id)?.folding.lifetime)
    }
    pub fn folding_generation(&self, id: u64) -> Option<u64> {
        Some(self.exact_fold_view(id)?.folding.generation)
    }
    /// Intent count only, for later App-wide accounting; not a current index.
    pub fn folding_desired_count(&self, id: u64) -> Option<usize> {
        Some(self.exact_fold_view(id)?.folding.desired.len())
    }
    pub fn folding_selection_generation(&self, id: u64) -> Option<u64> {
        Some(self.exact_fold_view(id)?.folding.selection_generation)
    }
    /// Cheap current map check. Direct public selection assignments are guarded
    /// here too; unproved maps are never usable for rendering or pointer hits.
    pub fn current_folding_rows(&self, id: u64) -> Option<&DisplayRows> {
        let view = self.exact_fold_view(id)?;
        let ready = view.folding.ready.as_ref()?;
        if view.folding.blocked
            || ready.epoch != self.text_epoch
            || ready.policy != self.folding_policy
            || usize::from(ready.rows.options().tab_size) != self.tab_size
            || !view_safe(view, &ready.hidden, self.len())
        {
            return None;
        }
        Some(&ready.rows)
    }
    /// Explicit caller hook for completed logical selection changes. It is not
    /// called during intermediate multi-cursor movement or render activation.
    pub fn observe_folding_selection(&mut self) {
        let len = self.len();
        let view = &mut self.view;
        if let Some(next) = view.folding.selection_generation.checked_add(1) {
            view.folding.selection_generation = next;
        } else {
            view.folding.blocked = true;
            view.folding.invalidate();
        }
        if view
            .folding
            .ready
            .as_ref()
            .is_some_and(|r| !view_safe(view, &r.hidden, len))
        {
            view.folding.invalidate();
        }
    }
    /// Effective source/options transition, not an Undo operation. The caller
    /// invokes this only for actual changed policies, including source ABA.
    pub fn retire_folding_policy(&mut self) {
        self.folding_policy = Lifetime::default();
        self.folding_changes.clear();
        for view in std::iter::once(&mut self.view).chain(self.other_views.values_mut()) {
            view.folding.invalidate();
            view.folding.desired = Arc::from([]);
            view.folding.desired_epoch = self.text_epoch;
        }
    }
    pub(super) fn record_folding_change(&mut self, range: Range<usize>, added: usize) {
        let wanted = std::iter::once(&self.view)
            .chain(self.other_views.values())
            .any(|view| !view.folding.desired.is_empty());
        for view in std::iter::once(&mut self.view).chain(self.other_views.values_mut()) {
            view.folding.invalidate();
        }
        if !wanted {
            self.folding_changes.clear();
            return;
        }
        self.folding_changes.push_back(Change {
            epoch: self.text_epoch,
            range,
            added,
        });
        if self.folding_changes.len() > MAX_JOURNAL {
            self.folding_changes.pop_front();
        }
        if let Some(first) = self.folding_changes.front() {
            for view in std::iter::once(&mut self.view).chain(self.other_views.values_mut()) {
                if !view.folding.desired.is_empty() && view.folding.desired_epoch < first.epoch - 1
                {
                    view.folding.desired = Arc::from([]);
                    view.folding.desired_epoch = self.text_epoch;
                }
            }
        }
    }
    pub fn capture_folding(&self, id: u64, options: Options) -> Result<FoldSnapshot> {
        ensure!(
            self.text.len_bytes() <= folding::MAX_BYTES && self.line_count() <= folding::MAX_LINES,
            "Folding exceeds 2 MiB or 100,000 lines"
        );
        ensure!(
            options.wrap == Wrap::Off
                && (1..=16).contains(&options.tab_size)
                && usize::from(options.tab_size) == self.tab_size,
            "Folding options are not current"
        );
        ensure!(
            self.other_views.len() < MAX_VIEWS,
            "Folding exceeds five retained views"
        );
        let view = self
            .exact_fold_view(id)
            .ok_or_else(|| anyhow::anyhow!("Folding view retired"))?;
        ensure!(!view.folding.blocked, "Folding view generation exhausted");
        ensure!(
            view.secondary.len() < MAX_SELECTIONS,
            "Folding exceeds 10,000 selections"
        );
        let first = primary(view);
        ensure!(
            valid_selection(&first, self.len())
                && view
                    .secondary
                    .iter()
                    .all(|s| valid_selection(s, self.len())),
            "Folding selection is outside current source"
        );
        ensure!(
            view.folding.desired.len() <= folding::MAX_REGIONS,
            "Folding exceeds 5,000 regions"
        );
        let changes = if view.folding.desired.is_empty() {
            Vec::new()
        } else {
            let baseline = view.folding.desired_epoch;
            ensure!(
                baseline <= self.text_epoch,
                "Folding anchor epoch is invalid"
            );
            if baseline != self.text_epoch {
                ensure!(
                    self.folding_changes
                        .front()
                        .is_some_and(|c| baseline >= c.epoch - 1)
                        && self
                            .folding_changes
                            .back()
                            .is_some_and(|c| c.epoch == self.text_epoch),
                    "Folding edit journal is no longer complete"
                );
            }
            self.folding_changes
                .iter()
                .filter(|c| c.epoch > baseline)
                .cloned()
                .collect()
        };
        Ok(FoldSnapshot {
            proof: Proof {
                document: self.id,
                view: id,
                lifetime: view.folding.lifetime.clone(),
                policy: self.folding_policy.clone(),
                epoch: self.text_epoch,
                generation: view.folding.generation,
                selection_generation: view.folding.selection_generation,
                options,
                primary: first,
                secondary: view.secondary.clone(),
            },
            text: self.text.clone(),
            desired: view.folding.desired.clone(),
            changes,
        })
    }
    pub fn prepare_fold_publication(&mut self, prepared: FoldPrepared) -> Result<FoldCommit<'_>> {
        let proof = &prepared.proof;
        ensure!(
            self.other_views.len() < MAX_VIEWS,
            "Folding exceeds five retained views"
        );
        ensure!(
            proof.document == self.id
                && proof.epoch == self.text_epoch
                && proof.policy == self.folding_policy
                && usize::from(proof.options.tab_size) == self.tab_size,
            "Folding source changed"
        );
        let view = self
            .exact_fold_view(proof.view)
            .ok_or_else(|| anyhow::anyhow!("Folding view retired"))?;
        ensure!(
            !view.folding.blocked
                && view.folding.lifetime == proof.lifetime
                && view.folding.generation == proof.generation
                && view.folding.selection_generation == proof.selection_generation
                && primary(view) == proof.primary
                && view.secondary == proof.secondary,
            "Folding view changed"
        );
        ensure!(
            view_safe(view, &prepared.hidden, self.len()),
            "Folding selection requires reveal"
        );
        let next_generation = view
            .folding
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Folding view generation exhausted"))?;
        let anchors = std::iter::once((self.active_view, &self.view))
            .chain(self.other_views.iter().map(|(id, v)| (*id, v)))
            .filter(|(id, _)| *id != proof.view)
            .try_fold(prepared.desired.len(), |n, (_, v)| {
                n.checked_add(v.folding.desired.len())
                    .ok_or_else(|| anyhow::anyhow!("Folding anchor budget overflow"))
            })?;
        ensure!(
            anchors <= MAX_DOCUMENT_ANCHORS,
            "Folding exceeds 20,000 per-document anchors"
        );
        Ok(FoldCommit {
            document: self,
            prepared,
            next_generation,
        })
    }
    pub fn prepare_folding_view(
        &mut self,
        target: u64,
        copy_from: Option<u64>,
    ) -> Result<FoldViewInsertion<'_>> {
        ensure!(
            self.exact_fold_view(target).is_none(),
            "Folding view already exists"
        );
        ensure!(
            self.other_views.len() + 1 < MAX_VIEWS,
            "Folding exceeds five retained views"
        );
        let origin = self
            .exact_fold_view(copy_from.unwrap_or(self.active_view))
            .ok_or_else(|| anyhow::anyhow!("Folding source view retired"))?;
        ensure!(
            origin.secondary.len() < MAX_SELECTIONS,
            "Folding exceeds 10,000 selections"
        );
        // Do not clone cursor history, snippet state or generated pair tables
        // merely to throw them away: the bounded live cohort is all we copy.
        let state = ViewState {
            cursor: origin.cursor,
            anchor: origin.anchor,
            secondary: origin.secondary.clone(),
            top: origin.top,
            left: origin.left,
            desired_column: origin.desired_column,
            cursor_history: Vec::new(),
            snippet: None,
            snippet_generation: origin.snippet_generation,
            pairs: super::typing::Pairs::default(),
            folding: if copy_from.is_some() {
                origin.folding.fresh_copy()
            } else {
                View::default()
            },
        };
        let anchors = std::iter::once(&self.view)
            .chain(self.other_views.values())
            .try_fold(state.folding.desired.len(), |n, v| {
                n.checked_add(v.folding.desired.len())
                    .ok_or_else(|| anyhow::anyhow!("Folding anchor budget overflow"))
            })?;
        ensure!(
            anchors <= MAX_DOCUMENT_ANCHORS,
            "Folding exceeds 20,000 per-document anchors"
        );
        // Reserve before the exclusive publication lease; publication cannot fail.
        self.other_views.try_reserve(1)?;
        Ok(FoldViewInsertion {
            document: self,
            target,
            state,
        })
    }
}
impl FoldClear<'_> {
    pub fn publish(self) {
        self.document
            .exact_fold_view_mut(self.view)
            .expect("exclusive folding clear lease")
            .folding = self.state;
    }
}
impl FoldCommit<'_> {
    pub fn publish(self) {
        let proof = &self.prepared.proof;
        let view = self
            .document
            .exact_fold_view_mut(proof.view)
            .expect("exclusive folding publication lease");
        view.folding.desired = self.prepared.desired;
        view.folding.desired_epoch = proof.epoch;
        view.folding.generation = self.next_generation;
        view.folding.ready = Some(Ready {
            epoch: proof.epoch,
            policy: proof.policy.clone(),
            rows: self.prepared.rows,
            hidden: self.prepared.hidden,
        });
    }
}
impl FoldViewInsertion<'_> {
    pub fn publish(self) {
        self.document.other_views.insert(self.target, self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_rows::RowAnchor;
    use std::{fs, time::Duration};

    const SOURCE: &str = "prefix\r\nhead猫\r\n body🙂\r\n tail\r\nafter\r\n";
    fn options(doc: &Document) -> Options {
        Options {
            wrap: Wrap::Off,
            width: 80,
            tab_size: doc.tab_size as u8,
        }
    }
    fn region(doc: &Document) -> Region {
        Region::from_lines(&doc.text, 1, 3).unwrap()
    }
    fn collapsed(doc: &Document, id: u64, regions: Vec<Region>) -> FoldPrepared {
        doc.capture_folding(id, options(doc))
            .unwrap()
            .prepare_collapsed(
                regions.into(),
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap()
    }
    fn existing(doc: &Document, id: u64) -> FoldPrepared {
        doc.capture_folding(id, options(doc))
            .unwrap()
            .prepare_existing(
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap()
    }
    fn publish(doc: &mut Document, id: u64) {
        let prepared = collapsed(doc, id, vec![region(doc)]);
        doc.prepare_fold_publication(prepared).unwrap().publish();
    }
    fn visible(doc: &Document, id: u64) -> Vec<usize> {
        let rows = doc.current_folding_rows(id).expect("current prepared rows");
        (0..rows.row_count())
            .map(|i| rows.anchor(i).unwrap().logical_line)
            .collect()
    }
    fn bytes(doc: &Document) -> Vec<u8> {
        doc.text.to_string().into_bytes()
    }

    #[test]
    fn publication_is_view_only_and_dropped_lease_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fold.cpp");
        fs::write(&path, SOURCE).unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.move_to(0, false);
        doc.insert("é🙂", false);
        let current = bytes(&doc);
        // Reversed primary and a second caret are outside the hidden body.
        doc.set_selections(vec![
            Selection {
                cursor: 0,
                anchor: Some(2),
                desired_column: None,
            },
            Selection::caret(doc.line_start(4)),
        ]);
        let selected = doc.selections();
        let proof = (
            doc.id,
            doc.revision,
            doc.saved_revision,
            doc.text_epoch,
            doc.save_generation,
            doc.dirty(),
        );
        let prepared = collapsed(&doc, 0, vec![region(&doc)]);
        drop(doc.prepare_fold_publication(prepared).unwrap());
        assert!(doc.current_folding_rows(0).is_none());
        publish(&mut doc, 0);
        assert_eq!(visible(&doc, 0), [0, 1, 4, 5]);
        assert_eq!(bytes(&doc), current);
        assert_eq!(doc.selections(), selected);
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.saved_revision,
                doc.text_epoch,
                doc.save_generation,
                doc.dirty()
            ),
            proof
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE.as_bytes());
        doc.undo();
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        assert!(doc.current_folding_rows(0).is_none());
        doc.redo();
        assert_eq!(bytes(&doc), current);
        assert!(doc.current_folding_rows(0).is_none());
        doc.save().unwrap();
        assert_eq!(fs::read(&path).unwrap(), current);
    }

    #[test]
    fn held_snapshot_cannot_revive_after_edit_undo_equal_bytes() {
        let mut doc = Document::from_text(SOURCE);
        let old = collapsed(&doc, 0, vec![region(&doc)]);
        let revision = doc.revision;
        let initial_epoch = doc.text_epoch;
        doc.insert("x", false);
        doc.undo();
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        assert_eq!(doc.revision, revision);
        assert!(doc.text_epoch > initial_epoch);
        assert!(doc.prepare_fold_publication(old).is_err());
        assert!(doc.current_folding_rows(0).is_none());
        doc.redo();
        assert_eq!(bytes(&doc), format!("x{SOURCE}").as_bytes());
    }

    #[test]
    fn disjoint_anchor_mapping_survives_but_touched_undo_never_restores_intent() {
        let mut doc = Document::from_text(SOURCE);
        publish(&mut doc, 0);
        doc.insert("é", false); // Strictly before the header.
        assert!(doc.current_folding_rows(0).is_none());
        let prepared = existing(&doc, 0);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        assert_eq!(visible(&doc, 0), [0, 1, 4, 5]);
        doc.undo();
        let prepared = existing(&doc, 0);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        assert_eq!(visible(&doc, 0), [0, 1, 4, 5]);
        let original = bytes(&doc);
        doc.move_to(doc.line_start(2) + 2, false);
        doc.insert("x", false);
        doc.undo();
        doc.move_to(0, false);
        assert_eq!(bytes(&doc), original);
        let prepared = existing(&doc, 0);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        assert_eq!(visible(&doc, 0), [0, 1, 2, 3, 4, 5]);
        assert!(doc.view.folding.desired.is_empty());
        doc.redo();
        assert_eq!(doc.text.line(2).to_string(), " bxody🙂\r\n");
    }

    #[test]
    fn explicit_split_copies_intent_with_fresh_lifetime_ordinary_shared_view_expands() {
        let mut doc = Document::from_text(SOURCE);
        publish(&mut doc, 0);
        let origin = doc.folding_view_lifetime(0).unwrap().clone();
        doc.prepare_folding_view(1, Some(0)).unwrap().publish();
        assert_ne!(doc.folding_view_lifetime(1), Some(&origin));
        assert_eq!(visible(&doc, 1), [0, 1, 4, 5]);
        doc.prepare_folding_view(2, None).unwrap().publish();
        assert!(doc.current_folding_rows(2).is_none());
        assert!(doc.other_views[&2].folding.desired.is_empty());
        doc.activate_view(1);
        doc.move_to(doc.line_start(4), false);
        doc.insert("x", false);
        assert!(doc.current_folding_rows(0).is_none());
        assert!(doc.current_folding_rows(1).is_none());
        doc.undo();
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        let prepared = existing(&doc, 0);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        let prepared = existing(&doc, 1);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        // A boundary edit conservatively retires the region in both views.
        assert_eq!(visible(&doc, 0), [0, 1, 2, 3, 4, 5]);
        assert_eq!(visible(&doc, 1), [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn removed_view_equal_numeric_id_cannot_accept_old_prepared_reply() {
        let mut doc = Document::from_text(SOURCE);
        doc.prepare_folding_view(7, None).unwrap().publish();
        let old = collapsed(&doc, 7, vec![region(&doc)]);
        let old_life = doc.folding_view_lifetime(7).unwrap().clone();
        doc.remove_view(7);
        doc.prepare_folding_view(7, None).unwrap().publish();
        assert_ne!(doc.folding_view_lifetime(7), Some(&old_life));
        assert!(doc.prepare_fold_publication(old).is_err());
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        assert!(!doc.dirty());
    }

    #[test]
    fn selection_aba_fences_reply_but_visible_movement_keeps_current_map() {
        let mut doc = Document::from_text(SOURCE);
        publish(&mut doc, 0);
        let prepared = existing(&doc, 0);
        let projection = doc.current_folding_rows(0).unwrap().clone();
        doc.move_to(1, false);
        doc.move_to(0, false);
        assert!(
            doc.current_folding_rows(0)
                .unwrap()
                .same_projection(&projection)
        );
        assert!(doc.prepare_fold_publication(prepared).is_err());
        // Direct assignments cannot reveal concealed source using a stale map.
        doc.cursor = doc.line_start(2) + 2;
        assert!(doc.current_folding_rows(0).is_none());
        doc.observe_folding_selection();
        let selected = doc.cursor;
        let prepared = existing(&doc, 0);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        assert_eq!(doc.cursor, selected);
        assert_eq!(visible(&doc, 0), [0, 1, 2, 3, 4, 5]);
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
    }

    #[test]
    fn entire_secondary_selected_interval_is_protected_and_batch_failure_is_atomic() {
        let mut doc = Document::from_text(SOURCE);
        publish(&mut doc, 0);
        let initial_generation = doc.folding_generation(0);
        let projection = doc.current_folding_rows(0).unwrap().clone();
        let later = Region::from_lines(&doc.text, 3, 4).unwrap();
        // Both individually valid, but crossing ancestry fails the whole batch.
        let result = doc
            .capture_folding(0, options(&doc))
            .unwrap()
            .prepare_collapsed(
                vec![region(&doc), later].into(),
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5),
            );
        assert!(result.is_err());
        assert_eq!(doc.folding_generation(0), initial_generation);
        assert!(
            doc.current_folding_rows(0)
                .unwrap()
                .same_projection(&projection)
        );
        doc.set_selections(vec![
            Selection::caret(0),
            Selection {
                cursor: doc.line_start(4),
                anchor: Some(doc.line_start(1)),
                desired_column: None,
            },
        ]);
        assert!(doc.current_folding_rows(0).is_none());
        assert!(
            doc.capture_folding(0, options(&doc))
                .unwrap()
                .prepare_collapsed(
                    vec![region(&doc)].into(),
                    &AtomicBool::new(false),
                    Instant::now() + Duration::from_secs(5)
                )
                .is_err()
        );
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        assert!(!doc.dirty());
    }

    #[test]
    fn journal_rollover_discards_unprovable_anchors_without_resurrecting_undo() {
        let mut doc = Document::from_text(SOURCE);
        publish(&mut doc, 0);
        doc.move_to(doc.line_start(2), false);
        doc.insert("x", false);
        doc.move_to(0, false);
        for _ in 0..MAX_JOURNAL {
            doc.insert("a", false);
        }
        assert!(doc.view.folding.desired.is_empty());
        assert_eq!(doc.folding_changes.len(), MAX_JOURNAL);
        for _ in 0..=MAX_JOURNAL {
            doc.undo();
        }
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        let prepared = existing(&doc, 0);
        doc.prepare_fold_publication(prepared).unwrap().publish();
        assert_eq!(visible(&doc, 0), [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn options_aba_retirement_is_not_undo_state_and_counter_failure_keeps_editing() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Document::from_text(SOURCE);
        let held = collapsed(&doc, 0, vec![region(&doc)]);
        let old_policy = doc.folding_policy_lifetime().clone();
        doc.set_indentation(2, true);
        doc.set_indentation(4, true);
        assert_ne!(doc.folding_policy_lifetime(), &old_policy);
        assert!(doc.prepare_fold_publication(held).is_err());
        doc.view.folding.generation = u64::MAX;
        let prepared = collapsed(&doc, 0, vec![region(&doc)]);
        assert!(doc.prepare_fold_publication(prepared).is_err());
        let original = bytes(&doc);
        doc.insert("é🙂", false);
        assert!(doc.view.folding.blocked);
        assert!(doc.capture_folding(0, options(&doc)).is_err());
        doc.undo();
        assert_eq!(bytes(&doc), original);
        doc.redo();
        assert_eq!(bytes(&doc), format!("é🙂{SOURCE}").as_bytes());
        let path = dir.path().join("counter.cpp");
        doc.save_to(&path, false).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes(&doc));
    }

    #[test]
    fn oversized_selection_inventory_refuses_before_cloning_and_preserves_redo() {
        let mut doc = Document::from_text(SOURCE);
        doc.insert("x", false);
        doc.undo();
        doc.secondary = vec![Selection::caret(0); MAX_SELECTIONS];
        let selected = doc.secondary.clone();
        let epoch = doc.text_epoch;
        assert!(doc.capture_folding(0, options(&doc)).is_err());
        assert_eq!(doc.secondary, selected);
        assert_eq!(doc.text_epoch, epoch);
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        doc.secondary.clear();
        doc.redo();
        assert_eq!(bytes(&doc), format!("x{SOURCE}").as_bytes());
    }

    #[test]
    fn malformed_tree_and_stale_publication_preserve_live_redo_and_saved_state() {
        let mut doc = Document::from_text(SOURCE);
        doc.insert("x", false);
        doc.undo();
        publish(&mut doc, 0);
        let projection = doc.current_folding_rows(0).unwrap().clone();
        let proof = (
            doc.id,
            doc.revision,
            doc.saved_revision,
            doc.text_epoch,
            doc.save_generation,
            doc.dirty(),
            doc.folding_generation(0),
        );
        let crossing = Region::from_lines(&doc.text, 3, 4).unwrap();
        assert!(
            doc.capture_folding(0, options(&doc))
                .unwrap()
                .prepare_collapsed(
                    vec![region(&doc), crossing].into(),
                    &AtomicBool::new(false),
                    Instant::now() + Duration::from_secs(5)
                )
                .is_err()
        );
        let held = existing(&doc, 0);
        doc.move_to(1, false);
        doc.move_to(0, false);
        assert!(doc.prepare_fold_publication(held).is_err());
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.saved_revision,
                doc.text_epoch,
                doc.save_generation,
                doc.dirty(),
                doc.folding_generation(0)
            ),
            proof
        );
        assert!(
            doc.current_folding_rows(0)
                .unwrap()
                .same_projection(&projection)
        );
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
        doc.redo();
        assert_eq!(bytes(&doc), format!("x{SOURCE}").as_bytes());
        doc.undo();
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
    }

    #[test]
    fn cancellation_and_deadline_refuse_without_publishing_any_partial_rows() {
        let mut doc = Document::from_text(SOURCE);
        publish(&mut doc, 0);
        let projection = doc.current_folding_rows(0).unwrap().clone();
        let snapshot = doc.capture_folding(0, options(&doc)).unwrap();
        assert!(
            snapshot
                .prepare_existing(
                    &AtomicBool::new(true),
                    Instant::now() + Duration::from_secs(5)
                )
                .is_err()
        );
        let snapshot = doc.capture_folding(0, options(&doc)).unwrap();
        assert!(
            snapshot
                .prepare_existing(&AtomicBool::new(false), Instant::now())
                .is_err()
        );
        assert!(
            doc.current_folding_rows(0)
                .unwrap()
                .same_projection(&projection)
        );
        assert_eq!(
            doc.current_folding_rows(0)
                .unwrap()
                .row(RowAnchor {
                    logical_line: 1,
                    segment: 0
                })
                .unwrap()
                .characters,
            doc.line_start(1)..doc.line_end(1)
        );
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
    }
    #[test]
    fn clear_renews_even_expanded_authority_without_touching_source_or_redo() {
        let mut doc = Document::from_text(SOURCE);
        doc.insert("é🙂", false);
        doc.undo();
        doc.move_to(0, false);
        let original = (
            doc.id,
            doc.revision,
            doc.saved_revision,
            doc.text_epoch,
            doc.save_generation,
            doc.dirty(),
            doc.selections(),
            bytes(&doc),
        );
        let snapshot = doc.capture_folding(0, options(&doc)).unwrap();
        let prepared = snapshot
            .clone()
            .prepare_collapsed(
                vec![region(&doc)].into(),
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let lifetime = doc.folding_view_lifetime(0).unwrap().clone();
        drop(doc.prepare_clear_folding(0).unwrap());
        assert_eq!(doc.folding_view_lifetime(0).unwrap(), &lifetime);
        assert!(doc.check_folding_snapshot(&snapshot).is_ok());
        doc.prepare_clear_folding(0).unwrap().publish();
        assert_ne!(doc.folding_view_lifetime(0).unwrap(), &lifetime);
        assert!(doc.check_folding_snapshot(&snapshot).is_err());
        assert!(doc.prepare_fold_publication(prepared).is_err());
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.saved_revision,
                doc.text_epoch,
                doc.save_generation,
                doc.dirty(),
                doc.selections(),
                bytes(&doc)
            ),
            original
        );
        let next = doc.folding_view_lifetime(0).unwrap().clone();
        doc.prepare_clear_folding(0).unwrap().publish();
        assert_ne!(
            doc.folding_view_lifetime(0).unwrap(),
            &next,
            "No-op clear retires pending interest"
        );
        doc.redo();
        assert_eq!(doc.text.to_string(), format!("é🙂{SOURCE}"));
        doc.undo();
        assert_eq!(bytes(&doc), SOURCE.as_bytes());
    }
    #[test]
    fn clear_survives_preparation_budget_and_clock_exhaustion_without_blocking_editing() {
        let mut doc = Document::from_text(&"x".repeat(folding::MAX_BYTES + 1));
        doc.secondary = vec![Selection::caret(0); MAX_SELECTIONS];
        assert!(doc.capture_folding(0, options(&doc)).is_err());
        let before = (
            doc.id,
            doc.revision,
            doc.text_epoch,
            doc.save_generation,
            doc.selections(),
        );
        let old = doc.view.folding.lifetime.clone();
        doc.view.folding.generation = u64::MAX;
        doc.view.folding.selection_generation = u64::MAX;
        doc.view.folding.blocked = true;
        doc.prepare_clear_folding(0).unwrap().publish();
        assert_ne!(doc.view.folding.lifetime, old);
        assert!(!doc.view.folding.blocked);
        assert_eq!(doc.view.folding.generation, 0);
        assert_eq!(doc.view.folding.selection_generation, 0);
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.text_epoch,
                doc.save_generation,
                doc.selections()
            ),
            before
        );
        assert_eq!(doc.len(), folding::MAX_BYTES + 1);
        doc.set_selections(vec![Selection::caret(0)]);
        doc.insert("猫", false);
        assert_eq!(doc.text.char(0), '猫');
        doc.undo();
        assert_eq!(doc.len(), folding::MAX_BYTES + 1);
        doc.redo();
        assert_eq!(doc.text.char(0), '猫');
    }
    #[test]
    fn borrowed_bridge_checks_exact_view_not_groups_liveness_and_noop_clear_retires_proof() {
        let mut doc = Document::from_text(SOURCE);
        let mut groups = crate::editor_groups::Groups::default();
        let member = groups.open(doc.id).unwrap().active.unwrap();
        doc.prepare_folding_view(member.group.value(), None)
            .unwrap()
            .publish();
        doc.activate_view(member.group.value());
        let snapshot = doc
            .capture_folding(member.group.value(), options(&doc))
            .unwrap();
        let proof = doc
            .with_folding_current(member, 7, |current| {
                crate::folding_controller::ViewProof::capture(
                    current.model.clone(),
                    current.view.unwrap(),
                )
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            proof.selection_generation,
            doc.folding_selection_generation(member.group.value())
                .unwrap()
        );
        assert!(
            doc.with_folding_current(member, 7, |current| snapshot.is_current(&current))
                .unwrap()
        );
        let other = groups.open(doc.id + 1).unwrap().active.unwrap();
        assert!(doc.with_folding_current(other, 7, |_| ()).is_err());
        // Removing the membership alone is deliberately App's check, not a
        // fabricated claim that Document knows Groups validity.
        groups.close(member).unwrap();
        assert!(
            doc.with_folding_current(member, 7, |current| snapshot.is_current(&current))
                .unwrap()
        );
        doc.prepare_clear_folding(member.group.value())
            .unwrap()
            .publish();
        assert!(
            !doc.with_folding_current(member, 7, |current| snapshot.is_current(&current))
                .unwrap()
        );
        doc.remove_view(member.group.value());
        assert!(doc.with_folding_current(member, 7, |_| ()).is_err());
    }
    #[test]
    fn finite_actions_protect_full_ranges_and_duplicate_carets_use_original_mask() {
        let mut doc = Document::from_text("root\r\n child猫\r\n  body🙂\r\n tail\r\nafter\r\n");
        let outer = Region::from_lines(&doc.text, 0, 3).unwrap();
        let inner = Region::from_lines(&doc.text, 1, 2).unwrap();
        let catalog: Arc<[Region]> = vec![outer.clone(), inner.clone()].into();
        let deadline = Instant::now() + Duration::from_secs(5);
        let cancel = AtomicBool::new(false);
        doc.set_selections(vec![Selection {
            cursor: doc.len(),
            anchor: Some(0),
            desired_column: None,
        }]);
        let protected = doc
            .capture_folding(0, options(&doc))
            .unwrap()
            .prepare_action(FoldAction::FoldAll, catalog.clone(), &cancel, deadline)
            .unwrap();
        assert_eq!(protected.desired_count(), 0);
        assert_eq!(protected.skipped_regions(), 2);
        assert!(
            doc.capture_folding(0, options(&doc))
                .unwrap()
                .prepare_action(FoldAction::Fold, catalog.clone(), &cancel, deadline)
                .is_err()
        );
        let header = doc.line_start(1);
        doc.set_selections(vec![Selection::caret(header)]);
        doc.secondary.push(Selection::caret(header));
        let folded = doc
            .capture_folding(0, options(&doc))
            .unwrap()
            .prepare_action(FoldAction::Fold, catalog.clone(), &cancel, deadline)
            .unwrap();
        assert_eq!(folded.desired_count(), 1);
        doc.prepare_fold_publication(folded).unwrap().publish();
        assert_eq!(doc.folding_desired_count(0), Some(1));
        let unfolded = doc
            .capture_folding(0, options(&doc))
            .unwrap()
            .prepare_action(FoldAction::Unfold, catalog, &cancel, deadline)
            .unwrap();
        assert_eq!(unfolded.desired_count(), 0);
        doc.prepare_fold_publication(unfolded).unwrap().publish();
        assert_eq!(
            bytes(&doc),
            "root\r\n child猫\r\n  body🙂\r\n tail\r\nafter\r\n".as_bytes()
        );
    }
}
