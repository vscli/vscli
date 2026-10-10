//! One bounded native folding controller lane. App must still validate live
//! membership/UI ownership before and after the borrowed Document proof bridge.
use crate::{
    display_rows::{Options, Wrap},
    document::{FoldAction, FoldSnapshot, Selection},
    editor_groups::Membership,
    folding::{self, Region},
    folding_worker::{self, DocumentWork, Prepared, Work, Worker},
};
use anyhow::{Context, Result, ensure};
use ropey::Rope;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Lifetime(Arc<()>);
impl Default for Lifetime {
    fn default() -> Self {
        Self(Arc::new(()))
    }
}
impl PartialEq for Lifetime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Lifetime {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelProof {
    pub document: u64,
    pub text_epoch: u64,
    pub options: Lifetime,
    pub tab_size: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewProof {
    pub model: ModelProof,
    pub membership: Membership,
    pub lifetime: Lifetime,
    pub generation: u64,
    pub selection_generation: u64,
    pub interaction: u64,
    pub primary: Selection,
    secondary: Vec<Selection>,
}
/// Borrowed live cohort: no polling/input cohort clone.
pub struct CurrentView<'a> {
    pub membership: Membership,
    pub lifetime: &'a Lifetime,
    pub generation: u64,
    pub selection_generation: u64,
    pub interaction: u64,
    pub primary: &'a Selection,
    pub secondary: &'a [Selection],
}
pub struct Current<'a> {
    pub model: &'a ModelProof,
    pub view: Option<CurrentView<'a>>,
}
impl ViewProof {
    pub fn capture(model: ModelProof, view: CurrentView<'_>) -> Result<Self> {
        validate_model(&model)?;
        ensure!(
            view.membership.document == model.document,
            "Folding membership belongs to another model"
        );
        ensure!(
            view.secondary.len() < folding_worker::MAX_SELECTIONS,
            "Folding action exceeds 10,000 selections"
        );
        Ok(Self {
            model,
            membership: view.membership,
            lifetime: view.lifetime.clone(),
            generation: view.generation,
            selection_generation: view.selection_generation,
            interaction: view.interaction,
            primary: view.primary.clone(),
            secondary: view.secondary.to_vec(),
        })
    }
    fn current(&self, current: &Current<'_>) -> bool {
        self.model == *current.model
            && current.view.as_ref().is_some_and(|view| {
                self.membership == view.membership
                    && self.lifetime == *view.lifetime
                    && self.generation == view.generation
                    && self.selection_generation == view.selection_generation
                    && self.interaction == view.interaction
                    && self.primary == *view.primary
                    && self.secondary.as_slice() == view.secondary
            })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Discover(ModelProof),
    Prepare {
        view: ViewProof,
        options: Options,
        collapsed: Arc<[Region]>,
    },
    DiscoverDocument {
        view: ViewProof,
        options: Options,
        action: FoldAction,
    },
    DocumentPrepare {
        view: ViewProof,
        options: Options,
        operation: DocumentWork,
    },
}
impl Intent {
    pub fn model(&self) -> &ModelProof {
        match self {
            Self::Discover(model) => model,
            Self::Prepare { view, .. }
            | Self::DiscoverDocument { view, .. }
            | Self::DocumentPrepare { view, .. } => &view.model,
        }
    }
    pub fn is_current(&self, current: &Current<'_>) -> bool {
        match self {
            Self::Discover(model) => model == current.model,
            Self::Prepare { view, .. }
            | Self::DiscoverDocument { view, .. }
            | Self::DocumentPrepare { view, .. } => view.current(current),
        }
    }
    fn work(&self) -> Result<Work> {
        Ok(match self {
            Self::Discover(model) => Work::Discover {
                tab_size: model.tab_size,
            },
            Self::Prepare {
                view,
                options,
                collapsed,
            } => Work::Prepare {
                options: *options,
                collapsed: collapsed.clone(),
                protected: std::iter::once(&view.primary)
                    .chain(&view.secondary)
                    .map(Selection::range)
                    .collect::<Vec<_>>()
                    .into(),
            },
            _ => anyhow::bail!("Document folding requires owned snapshot dispatch"),
        })
    }
    fn document_work(&self) -> Result<(Options, DocumentWork)> {
        Ok(match self {
            Self::DiscoverDocument { options, .. } => (*options, DocumentWork::Discover),
            Self::DocumentPrepare {
                options, operation, ..
            } => (*options, operation.clone()),
            _ => anyhow::bail!("Not a document folding intent"),
        })
    }
}
struct Desired {
    intent: Intent,
    expires: Instant,
    grant: Lifetime,
    snapshot: Option<FoldSnapshot>,
}
struct Running {
    token: u64,
    intent: Intent,
    expires: Instant,
    grant: Lifetime,
}
pub struct Outcome {
    pub intent: Intent,
    pub result: std::result::Result<Prepared, String>,
    phase: Option<DocumentPhase>,
}
/// Private consume-once phase authority. A caller cannot fabricate a catalog
/// result, extend the original deadline, or advance after retirement/replacement.
pub struct DocumentPhase {
    intent: Intent,
    expires: Instant,
    grant: Lifetime,
    snapshot: FoldSnapshot,
}
impl Outcome {
    pub fn document_phase(self) -> Result<DocumentPhase> {
        self.phase
            .context("Not an accepted document discovery outcome")
    }
}
#[derive(Default)]
pub struct State {
    worker: Worker,
    desired: Option<Desired>,
    running: Option<Running>,
    phase: Option<Lifetime>,
    stopped: bool,
    notice: Option<&'static str>,
}
impl State {
    #[cfg(test)]
    pub(crate) fn hold_worker(&mut self, gates: folding_worker::Gates) {
        self.worker.hold(gates);
    }
    #[cfg(test)]
    pub(crate) fn last_worker_token(&self) -> u64 {
        self.worker.last_token()
    }
    pub fn occupied(&self) -> bool {
        self.worker.occupied()
    }
    pub fn desired(&self) -> Option<&Intent> {
        self.desired.as_ref().map(|d| &d.intent)
    }
    /// Poll dispatcher can avoid recapturing a discovery phase's owned source.
    pub fn document_snapshot_ready(&self) -> bool {
        self.desired
            .as_ref()
            .is_some_and(|desired| desired.snapshot.is_some())
    }
    pub fn interest(&self) -> Option<&Intent> {
        self.running
            .as_ref()
            .map(|r| &r.intent)
            .or_else(|| self.desired())
    }
    pub fn take_notice(&mut self) -> Option<&'static str> {
        self.notice.take()
    }
    pub fn resolving(&self) -> bool {
        self.desired.is_some() || self.running.is_some()
    }
    pub fn request(&mut self, intent: Intent, now: Instant) -> Result<bool> {
        ensure!(!self.stopped, "Folding controller stopped");
        validate_intent(&intent)?;
        let expires = now
            .checked_add(folding_worker::DEADLINE)
            .context("Folding action deadline overflow")?;
        if self.desired.as_ref().is_some_and(|d| d.intent == intent)
            || self.running.as_ref().is_some_and(|r| r.intent == intent)
        {
            return Ok(false);
        }
        self.cancel_running();
        self.phase = None;
        self.notice = None;
        self.desired = Some(Desired {
            intent,
            expires,
            grant: Lifetime::default(),
            snapshot: None,
        });
        Ok(true)
    }
    /// Accepted catalog -> preparation is not a new request/deadline. Stale
    /// phase admission never discards a newer unrelated owned intent.
    pub fn advance_document_phase(
        &mut self,
        phase: DocumentPhase,
        current: &Current<'_>,
        now: Instant,
    ) -> Result<bool> {
        ensure!(!self.stopped, "Folding controller stopped");
        ensure!(
            self.phase.as_ref() == Some(&phase.grant)
                && self.desired.is_none()
                && self.running.is_none(),
            "Folding discovery phase retired or replaced"
        );
        if now >= phase.expires
            || !phase.intent.is_current(current)
            || !phase.snapshot.is_current(current)
        {
            self.phase = None;
            self.notice = Some(if now >= phase.expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            return Ok(false);
        }
        validate_intent(&phase.intent)?;
        self.phase = None;
        self.desired = Some(Desired {
            intent: phase.intent,
            expires: phase.expires,
            grant: phase.grant,
            snapshot: Some(phase.snapshot),
        });
        Ok(true)
    }
    fn admitted(&mut self, current: &Current<'_>, now: Instant) -> bool {
        if self.stopped || !self.worker.available() {
            return false;
        }
        let Some(d) = &self.desired else { return false };
        if now >= d.expires || !d.intent.is_current(current) {
            self.notice = Some(if now >= d.expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            self.desired = None;
            self.phase = None;
            return false;
        }
        true
    }
    fn started(&mut self, token: u64) {
        let d = self.desired.take().expect("one owned intent");
        self.running = Some(Running {
            token,
            intent: d.intent,
            expires: d.expires,
            grant: d.grant,
        });
    }
    pub fn dispatch(&mut self, current: &Current<'_>, text: Rope, now: Instant) -> Result<bool> {
        if !self.admitted(current, now) {
            return Ok(false);
        }
        let work = self.desired.as_ref().unwrap().intent.work()?;
        let deadline = self.desired.as_ref().unwrap().expires;
        let token = match self.worker.start_until(text, work, now, deadline) {
            Ok(token) => token,
            Err(error) => {
                self.desired = None;
                return Err(error);
            }
        };
        self.started(token);
        Ok(true)
    }
    /// Only Document::capture_folding can supply the originating owned source.
    /// Capacity is checked before any snapshot/proof operation in this method;
    /// caller must likewise check capacity before allocating a fresh capture.
    pub fn dispatch_document(
        &mut self,
        current: &Current<'_>,
        snapshot: FoldSnapshot,
        now: Instant,
    ) -> Result<bool> {
        if !self.admitted(current, now) {
            return Ok(false);
        }
        let desired = self.desired.as_ref().unwrap();
        ensure!(
            desired.snapshot.is_none(),
            "Discovery phase owns its original snapshot; use dispatch_ready_document"
        );
        let (options, operation) = desired.intent.document_work()?;
        ensure!(
            snapshot.is_current(current) && snapshot.options() == options,
            "Folding snapshot does not match owned intent/current view"
        );
        let token = match self
            .worker
            .start_document(snapshot, operation, now, desired.expires)
        {
            Ok(token) => token,
            Err(error) => {
                self.desired = None;
                return Err(error);
            }
        };
        self.started(token);
        Ok(true)
    }
    /// Preparation following discovery consumes the original snapshot, never a
    /// recaptured or caller-substituted Rope. One latest intent, same actual lane.
    pub fn dispatch_ready_document(&mut self, current: &Current<'_>, now: Instant) -> Result<bool> {
        if !self.admitted(current, now) {
            return Ok(false);
        }
        let desired = self.desired.as_ref().unwrap();
        let (options, operation) = desired.intent.document_work()?;
        let snapshot = desired
            .snapshot
            .as_ref()
            .context("No prepared discovery phase")?;
        ensure!(
            snapshot.is_current(current) && snapshot.options() == options,
            "Discovery source/view changed before preparation"
        );
        let deadline = desired.expires;
        let snapshot = self.desired.as_mut().unwrap().snapshot.take().unwrap();
        let token = match self
            .worker
            .start_document(snapshot, operation, now, deadline)
        {
            Ok(token) => token,
            Err(error) => {
                self.desired = None;
                return Err(error);
            }
        };
        self.started(token);
        Ok(true)
    }
    pub fn poll(&mut self, current: &Current<'_>, now: Instant) -> Option<Outcome> {
        if self
            .desired
            .as_ref()
            .is_some_and(|d| now >= d.expires || !d.intent.is_current(current))
        {
            self.notice = Some(if now >= self.desired.as_ref().unwrap().expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            self.desired = None;
            self.phase = None;
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| now >= r.expires || !r.intent.is_current(current))
        {
            self.notice = Some(if now >= self.running.as_ref().unwrap().expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            self.cancel_running();
            self.phase = None;
        }
        let reply = self.worker.poll()?;
        let running = self.running.take()?;
        if reply.token != running.token
            || !running.intent.is_current(current)
            || now >= running.expires
        {
            return None;
        }
        let (result, phase) = match (reply.result, &running.intent) {
            (
                Ok(Prepared::DocumentCatalog { snapshot, catalog }),
                Intent::DiscoverDocument {
                    view,
                    options,
                    action,
                },
            ) => {
                self.phase = Some(running.grant.clone());
                let phase = DocumentPhase {
                    intent: Intent::DocumentPrepare {
                        view: view.clone(),
                        options: *options,
                        operation: DocumentWork::Action {
                            action: *action,
                            catalog: catalog.clone(),
                        },
                    },
                    expires: running.expires,
                    grant: running.grant,
                    snapshot,
                };
                (Ok(Prepared::Catalog(catalog)), Some(phase))
            }
            (result, _) => (result, None),
        };
        Some(Outcome {
            intent: running.intent,
            result,
            phase,
        })
    }

    pub fn retire(&mut self) {
        self.desired = None;
        self.phase = None;
        self.cancel_running()
    }
    pub fn poll_retired(&mut self) -> bool {
        self.running.is_none() && self.worker.poll().is_some()
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<()> {
        self.stopped = true;
        self.retire();
        self.worker.shutdown(timeout)
    }
    fn cancel_running(&mut self) {
        if let Some(r) = self.running.take() {
            self.worker.cancel(r.token)
        }
    }
}
fn validate_model(model: &ModelProof) -> Result<()> {
    ensure!(
        model.document != 0 && (1..=16).contains(&model.tab_size),
        "Invalid folding model proof"
    );
    Ok(())
}
fn validate_intent(intent: &Intent) -> Result<()> {
    validate_model(intent.model())?;
    let (view, options) = match intent {
        Intent::Discover(_) => return Ok(()),
        Intent::Prepare {
            view,
            options,
            collapsed,
        } => {
            ensure!(
                collapsed.len() <= folding::MAX_REGIONS,
                "Folding action exceeds 5,000 regions"
            );
            (view, options)
        }
        Intent::DiscoverDocument { view, options, .. } => (view, options),
        Intent::DocumentPrepare {
            view,
            options,
            operation,
        } => {
            if let DocumentWork::Collapsed(regions)
            | DocumentWork::Action {
                catalog: regions, ..
            } = operation
            {
                ensure!(
                    regions.len() <= folding::MAX_REGIONS,
                    "Folding action exceeds 5,000 regions"
                );
            }
            ensure!(
                !matches!(operation, DocumentWork::Discover),
                "Use owned document discovery intent"
            );
            (view, options)
        }
    };
    ensure!(
        view.secondary.len() < folding_worker::MAX_SELECTIONS,
        "Folding action exceeds 10,000 selections"
    );
    ensure!(
        view.membership.document == view.model.document && options.tab_size == view.model.tab_size,
        "Folding action model/options mismatch"
    );
    ensure!(
        options.wrap == Wrap::Off,
        "Wrapping is not part of native folding"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editor_groups::Groups,
        folding_worker::{Gate, Gates},
    };
    fn model() -> ModelProof {
        ModelProof {
            document: 1,
            text_epoch: 0,
            options: Lifetime::default(),
            tab_size: 4,
        }
    }
    #[test]
    fn closed_last_model_drains_retired_worker_without_fabricating_current_source() {
        let mut state = State::default();
        let original = model();
        let gates = Gates {
            before: Gate::held(),
            after: Gate::held(),
        };
        state.worker.hold(gates.clone());
        state
            .request(Intent::Discover(original.clone()), Instant::now())
            .unwrap();
        state
            .dispatch(
                &Current {
                    model: &original,
                    view: None,
                },
                Rope::from_str("closed\n body"),
                Instant::now(),
            )
            .unwrap();
        assert!(
            !state.poll_retired(),
            "Live interest cannot be discarded by the drain"
        );
        state.retire();
        assert!(state.interest().is_none());
        assert!(!state.resolving());
        assert!(!state.poll_retired());
        assert!(state.occupied());
        gates.before.release();
        gates.after.release();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !state.poll_retired() {
            assert!(
                Instant::now() < deadline,
                "Retired worker never actually settled"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!state.occupied());
        assert!(!state.poll_retired());
        let mut next = model();
        next.document = 2;
        state
            .request(Intent::Discover(next.clone()), Instant::now())
            .unwrap();
        let current = Current {
            model: &next,
            view: None,
        };
        assert!(
            state
                .dispatch(&current, Rope::from_str("new\n child"), Instant::now())
                .unwrap()
        );
        assert!(matches!(
            until_settled(&mut state, &current).unwrap().result,
            Ok(Prepared::Catalog(_))
        ));
    }
    fn until_settled(state: &mut State, current: &Current<'_>) -> Option<Outcome> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let outcome = state.poll(current, Instant::now());
            if !state.occupied() {
                return outcome;
            }
            assert!(
                Instant::now() < deadline,
                "Folding slot never actually settled"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn held_discovery_edit_undo_epoch_and_options_aba_never_publish_stale_result() {
        for mode in 0..2 {
            let original = model();
            let mut current_model = original.clone();
            let mut state = State::default();
            let gates = Gates {
                before: Gate::held(),
                after: Gate::held(),
            };
            state.worker.hold(gates.clone());
            let source = Rope::from_str("root\n body\nafter");
            state
                .request(Intent::Discover(original.clone()), Instant::now())
                .unwrap();
            state
                .dispatch(
                    &Current {
                        model: &original,
                        view: None,
                    },
                    source.clone(),
                    Instant::now(),
                )
                .unwrap();
            if mode == 0 {
                current_model.text_epoch = 1;
            } else {
                current_model.options = Lifetime::default();
            }
            assert!(
                state
                    .poll(
                        &Current {
                            model: &current_model,
                            view: None
                        },
                        Instant::now()
                    )
                    .is_none()
            );
            assert!(state.occupied());
            // Equal bytes/revision or returning to the old source proof cannot
            // restore the retired controller interest after the observed change.
            if mode == 0 {
                current_model.text_epoch = 2;
            } else {
                current_model = original.clone();
            }
            gates.before.release();
            gates.after.release();
            assert!(
                until_settled(
                    &mut state,
                    &Current {
                        model: &current_model,
                        view: None
                    }
                )
                .is_none()
            );
            assert_eq!(source.to_string(), "root\n body\nafter");
            assert!(!state.resolving());
        }
    }
    #[test]
    fn latest_intent_waits_actual_cancel_release_and_repetition_is_bounded() {
        let mut state = State::default();
        let original = model();
        let gates = Gates {
            before: Gate::held(),
            after: Gate::held(),
        };
        state.worker.hold(gates.clone());
        state
            .request(Intent::Discover(original.clone()), Instant::now())
            .unwrap();
        state
            .dispatch(
                &Current {
                    model: &original,
                    view: None,
                },
                Rope::from_str("root\n body"),
                Instant::now(),
            )
            .unwrap();
        let mut latest = original.clone();
        latest.text_epoch = 1;
        assert!(
            state
                .request(Intent::Discover(latest.clone()), Instant::now())
                .unwrap()
        );
        for _ in 0..64 {
            assert!(
                !state
                    .request(Intent::Discover(latest.clone()), Instant::now())
                    .unwrap()
            );
        }
        let current = Current {
            model: &latest,
            view: None,
        };
        assert!(
            !state
                .dispatch(&current, Rope::new(), Instant::now())
                .unwrap()
        );
        gates.before.release();
        gates.after.release();
        assert!(until_settled(&mut state, &current).is_none());
        assert!(state.desired().is_some());
        assert!(
            state
                .dispatch(&current, Rope::from_str("new\n child"), Instant::now())
                .unwrap()
        );
        let result = until_settled(&mut state, &current).unwrap();
        assert!(matches!(result.result, Ok(Prepared::Catalog(_))));
        assert!(!state.resolving());
    }
    #[test]
    fn held_preparation_reopened_view_or_focus_aba_cannot_collapse_same_model() {
        let mut groups = Groups::default();
        groups.open(1).unwrap();
        let member = groups.active_membership().unwrap();
        let model = model();
        let primary = Selection::caret(0);
        let source = Rope::from_str("root\n body\nafter");
        let original_life = Lifetime::default();
        for mode in 0..2 {
            let mut life = original_life.clone();
            let mut interaction = 0;
            let original = CurrentView {
                membership: member,
                lifetime: &original_life,
                generation: 0,
                selection_generation: 0,
                interaction: 0,
                primary: &primary,
                secondary: &[],
            };
            let proof = ViewProof::capture(model.clone(), original).unwrap();
            let intent = Intent::Prepare {
                view: proof,
                options: Options {
                    wrap: crate::display_rows::Wrap::Off,
                    width: 80,
                    tab_size: 4,
                },
                collapsed: vec![Region::from_lines(&source, 0, 1).unwrap()].into(),
            };
            let mut state = State::default();
            let gates = Gates {
                before: Gate::held(),
                after: Gate::held(),
            };
            state.worker.hold(gates.clone());
            state.request(intent, Instant::now()).unwrap();
            state
                .dispatch(
                    &Current {
                        model: &model,
                        view: Some(CurrentView {
                            membership: member,
                            lifetime: &life,
                            generation: 0,
                            selection_generation: 0,
                            interaction,
                            primary: &primary,
                            secondary: &[],
                        }),
                    },
                    source.clone(),
                    Instant::now(),
                )
                .unwrap();
            if mode == 0 {
                life = Lifetime::default();
            } else {
                interaction = 1;
            }
            assert!(
                state
                    .poll(
                        &Current {
                            model: &model,
                            view: Some(CurrentView {
                                membership: member,
                                lifetime: &life,
                                generation: 0,
                                selection_generation: 0,
                                interaction,
                                primary: &primary,
                                secondary: &[]
                            })
                        },
                        Instant::now()
                    )
                    .is_none()
            );
            interaction = 2;
            gates.before.release();
            gates.after.release();
            assert!(
                until_settled(
                    &mut state,
                    &Current {
                        model: &model,
                        view: Some(CurrentView {
                            membership: member,
                            lifetime: &life,
                            generation: 0,
                            selection_generation: 0,
                            interaction,
                            primary: &primary,
                            secondary: &[]
                        })
                    }
                )
                .is_none()
            );
            assert_eq!(source.to_string(), "root\n body\nafter");
        }
    }
    #[test]
    fn logical_deadline_and_shutdown_keep_actual_slot_but_drop_queued_intent() {
        let mut state = State::default();
        let model = model();
        let now = Instant::now();
        let gates = Gates {
            before: Gate::held(),
            after: Gate::held(),
        };
        state.worker.hold(gates.clone());
        state.request(Intent::Discover(model.clone()), now).unwrap();
        let current = Current {
            model: &model,
            view: None,
        };
        state
            .dispatch(&current, Rope::from_str("root\n body"), now)
            .unwrap();
        assert!(
            state
                .poll(&current, now + folding_worker::DEADLINE)
                .is_none()
        );
        assert_eq!(
            state.take_notice(),
            Some("Folding request expired; text retained")
        );
        assert!(
            state
                .poll(&current, now + folding_worker::DEADLINE)
                .is_none()
        );
        assert!(state.take_notice().is_none());
        assert!(state.occupied());
        assert!(!state.resolving());
        let mut latest = model.clone();
        latest.text_epoch = 1;
        state.request(Intent::Discover(latest), now).unwrap();
        assert!(state.shutdown(Duration::ZERO).is_err());
        assert!(state.desired().is_none());
        assert!(state.request(Intent::Discover(model.clone()), now).is_err());
        gates.before.release();
        gates.after.release();
        assert!(until_settled(&mut state, &current).is_none());
        assert!(state.shutdown(Duration::from_secs(1)).is_ok());
    }
    #[test]
    fn invalid_admission_is_inert_and_bounded_cohort_capture_preserves_interest() {
        let mut state = State::default();
        let model = model();
        state
            .request(Intent::Discover(model.clone()), Instant::now())
            .unwrap();
        let current = Current {
            model: &model,
            view: None,
        };
        let oversized = Rope::from_str(&"x".repeat(folding::MAX_BYTES + 1));
        assert!(state.dispatch(&current, oversized, Instant::now()).is_err());
        assert!(!state.occupied());
        assert!(state.desired().is_none());
        assert!(
            !state
                .dispatch(&current, Rope::new(), Instant::now())
                .unwrap()
        );
        state
            .request(Intent::Discover(model.clone()), Instant::now())
            .unwrap();
        let mut groups = Groups::default();
        groups.open(1).unwrap();
        let life = Lifetime::default();
        let primary = Selection::caret(0);
        let secondary = vec![primary.clone(); folding_worker::MAX_SELECTIONS];
        assert!(
            ViewProof::capture(
                model,
                CurrentView {
                    membership: groups.active_membership().unwrap(),
                    lifetime: &life,
                    generation: 0,
                    selection_generation: 0,
                    interaction: 0,
                    primary: &primary,
                    secondary: &secondary
                }
            )
            .is_err()
        );
        assert!(state.desired().is_some());
        assert!(!state.occupied());
    }
    const DOCUMENT_SOURCE: &str = "prefix\r\nhead猫\r\n body🙂\r\n tail\r\nafter\r\n";
    fn document_fixture() -> (crate::document::Document, Membership, Options) {
        let mut doc = crate::document::Document::from_text(DOCUMENT_SOURCE);
        let mut groups = Groups::default();
        let member = groups.open(doc.id).unwrap().active.unwrap();
        doc.prepare_folding_view(member.group.value(), None)
            .unwrap()
            .publish();
        doc.activate_view(member.group.value());
        let options = Options {
            wrap: Wrap::Off,
            width: 80,
            tab_size: doc.tab_size as u8,
        };
        (doc, member, options)
    }
    fn document_intent(
        doc: &crate::document::Document,
        member: Membership,
        options: Options,
        action: FoldAction,
    ) -> Intent {
        let view = doc
            .with_folding_current(member, 17, |current| {
                ViewProof::capture(current.model.clone(), current.view.unwrap())
            })
            .unwrap()
            .unwrap();
        Intent::DiscoverDocument {
            view,
            options,
            action,
        }
    }
    fn dispatch_document(
        state: &mut State,
        doc: &crate::document::Document,
        member: Membership,
        options: Options,
        now: Instant,
    ) -> bool {
        let snapshot = doc.capture_folding(member.group.value(), options).unwrap();
        doc.with_folding_current(member, 17, |current| {
            state.dispatch_document(&current, snapshot, now)
        })
        .unwrap()
        .unwrap()
    }
    fn settle_document(
        state: &mut State,
        doc: &crate::document::Document,
        member: Membership,
    ) -> Option<Outcome> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let result = doc
                .with_folding_current(member, 17, |current| state.poll(&current, Instant::now()))
                .unwrap();
            if !state.occupied() {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "Document worker failed to settle"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn start_discovery(
        state: &mut State,
        doc: &crate::document::Document,
        member: Membership,
        options: Options,
        now: Instant,
    ) -> DocumentPhase {
        state
            .request(
                document_intent(doc, member, options, FoldAction::FoldAll),
                now,
            )
            .unwrap();
        assert!(dispatch_document(state, doc, member, options, now));
        settle_document(state, doc, member)
            .unwrap()
            .document_phase()
            .unwrap()
    }
    #[test]
    fn original_snapshot_discover_prepare_publish_keeps_deadline_and_exact_source() {
        let (mut doc, member, options) = document_fixture();
        let mut state = State::default();
        let now = Instant::now();
        let original = (
            doc.id,
            doc.revision,
            doc.text_epoch(),
            doc.save_generation(),
            doc.selections(),
        );
        let phase = start_discovery(&mut state, &doc, member, options, now);
        assert_eq!(phase.snapshot.text().to_string(), DOCUMENT_SOURCE);
        assert_eq!(phase.expires, now + folding_worker::DEADLINE);
        assert!(
            doc.with_folding_current(member, 17, |current| state.advance_document_phase(
                phase,
                &current,
                Instant::now()
            ))
            .unwrap()
            .unwrap()
        );
        let snapshot = doc.capture_folding(member.group.value(), options).unwrap();
        assert!(
            doc.with_folding_current(member, 17, |current| state.dispatch_document(
                &current,
                snapshot,
                Instant::now()
            ))
            .unwrap()
            .is_err(),
            "Recapture cannot substitute for original phase source"
        );
        assert!(
            doc.with_folding_current(member, 17, |current| state
                .dispatch_ready_document(&current, Instant::now()))
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            state.running.as_ref().unwrap().expires,
            now + folding_worker::DEADLINE
        );
        let result = settle_document(&mut state, &doc, member).unwrap();
        let Prepared::Document(prepared) = result.result.unwrap() else {
            panic!("Wrong result kind")
        };
        doc.prepare_fold_publication(prepared).unwrap().publish();
        let rows = doc.current_folding_rows(member.group.value()).unwrap();
        let anchors: Vec<_> = (0..rows.row_count())
            .map(|r| rows.anchor(r).unwrap().logical_line)
            .collect();
        assert_eq!(anchors, [0, 1, 4, 5]);
        assert_eq!(doc.text.to_string(), DOCUMENT_SOURCE);
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.text_epoch(),
                doc.save_generation(),
                doc.selections()
            ),
            original
        );
        assert!(!state.occupied());
        assert!(!state.resolving());
    }
    #[test]
    fn stale_phase_cannot_replace_latest_intent_or_extend_original_expiry() {
        let (doc, member, options) = document_fixture();
        for replaced in [false, true] {
            let mut state = State::default();
            let now = Instant::now();
            let phase = start_discovery(&mut state, &doc, member, options, now);
            let settled_token = state.worker.last_token();
            if replaced {
                let latest = document_intent(&doc, member, options, FoldAction::Fold);
                state.request(latest.clone(), Instant::now()).unwrap();
                assert!(
                    doc.with_folding_current(member, 17, |current| state.advance_document_phase(
                        phase,
                        &current,
                        Instant::now()
                    ))
                    .unwrap()
                    .is_err()
                );
                assert_eq!(state.desired(), Some(&latest));
                assert!(dispatch_document(
                    &mut state,
                    &doc,
                    member,
                    options,
                    Instant::now()
                ));
                assert!(settle_document(&mut state, &doc, member).is_some());
            } else {
                assert!(
                    !doc.with_folding_current(member, 17, |current| state.advance_document_phase(
                        phase,
                        &current,
                        now + folding_worker::DEADLINE
                    ))
                    .unwrap()
                    .unwrap()
                );
                assert!(state.desired().is_none());
                assert_eq!(state.worker.last_token(), settled_token);
                assert!(state.worker.available());
                assert_eq!(
                    state.take_notice(),
                    Some("Folding request expired; text retained")
                );
            }
        }
    }
    #[test]
    fn held_document_result_retires_for_edit_undo_selection_options_clear_and_deadline() {
        for mode in 0..5 {
            let (mut doc, member, options) = document_fixture();
            let mut state = State::default();
            let now = Instant::now();
            let phase = start_discovery(&mut state, &doc, member, options, now);
            doc.with_folding_current(member, 17, |current| {
                state.advance_document_phase(phase, &current, Instant::now())
            })
            .unwrap()
            .unwrap();
            let gates = Gates {
                before: Gate::held(),
                after: Gate::held(),
            };
            state.worker.hold(gates.clone());
            doc.with_folding_current(member, 17, |current| {
                state.dispatch_ready_document(&current, Instant::now())
            })
            .unwrap()
            .unwrap();
            match mode {
                0 => {
                    doc.insert("é", false);
                    doc.undo();
                }
                1 => {
                    doc.move_to(1, false);
                    doc.move_to(0, false);
                }
                2 => {
                    doc.set_indentation(2, true);
                    doc.set_indentation(4, true);
                }
                3 => {
                    doc.prepare_clear_folding(member.group.value())
                        .unwrap()
                        .publish();
                }
                _ => {}
            }
            let poll_time = if mode == 4 {
                now + folding_worker::DEADLINE
            } else {
                Instant::now()
            };
            assert!(
                doc.with_folding_current(member, 17, |current| state.poll(&current, poll_time))
                    .unwrap()
                    .is_none()
            );
            assert!(!state.resolving());
            assert!(
                state.occupied(),
                "Logical retirement must not free actual worker"
            );
            gates.before.release();
            gates.after.wait_reached();
            assert!(
                doc.with_folding_current(member, 17, |current| state
                    .poll(&current, Instant::now()))
                    .unwrap()
                    .is_none()
            );
            assert!(state.occupied());
            gates.after.release();
            assert!(settle_document(&mut state, &doc, member).is_none());
            assert!(!state.occupied());
            assert!(doc.current_folding_rows(member.group.value()).is_none());
            assert_eq!(doc.text.to_string(), DOCUMENT_SOURCE);
            if mode == 0 {
                doc.redo();
                assert_eq!(doc.text.to_string(), format!("é{DOCUMENT_SOURCE}"));
                doc.undo();
                assert_eq!(doc.text.to_string(), DOCUMENT_SOURCE);
            }
        }
    }
    #[test]
    fn refused_foreign_snapshot_preserves_owned_intent_and_phase_source() {
        let (doc, member, options) = document_fixture();
        let mut state = State::default();
        let now = Instant::now();
        let intent = document_intent(&doc, member, options, FoldAction::FoldAll);
        state.request(intent.clone(), now).unwrap();
        let foreign = crate::document::Document::from_text(DOCUMENT_SOURCE);
        let snapshot = foreign.capture_folding(0, options).unwrap();
        assert!(
            doc.with_folding_current(member, 17, |current| state
                .dispatch_document(&current, snapshot, now))
                .unwrap()
                .is_err()
        );
        assert_eq!(state.desired(), Some(&intent));
        assert!(state.worker.available());
        assert_eq!(state.worker.last_token(), 0);
        assert!(dispatch_document(&mut state, &doc, member, options, now));
        let phase = settle_document(&mut state, &doc, member)
            .unwrap()
            .document_phase()
            .unwrap();
        state.retire();
        assert!(
            doc.with_folding_current(member, 17, |current| state.advance_document_phase(
                phase,
                &current,
                Instant::now()
            ))
            .unwrap()
            .is_err()
        );
        assert!(!state.resolving());
        assert_eq!(doc.text.to_string(), DOCUMENT_SOURCE);
    }
    #[test]
    fn editable_public_outcome_fields_cannot_redirect_private_discovery_authority() {
        let (doc, member, options) = document_fixture();
        let (foreign, other, foreign_options) = document_fixture();
        let mut state = State::default();
        let now = Instant::now();
        state
            .request(
                document_intent(&doc, member, options, FoldAction::FoldAll),
                now,
            )
            .unwrap();
        assert!(dispatch_document(&mut state, &doc, member, options, now));
        let mut outcome = settle_document(&mut state, &doc, member).unwrap();
        outcome.intent = document_intent(&foreign, other, foreign_options, FoldAction::Unfold);
        outcome.result = Err("Caller changed the public result".into());
        let phase = outcome.document_phase().unwrap();
        assert_eq!(phase.snapshot.document_id(), doc.id);
        assert_eq!(phase.intent.model().document, doc.id);
        assert!(matches!(
            &phase.intent,
            Intent::DocumentPrepare {
                operation: DocumentWork::Action {
                    action: FoldAction::FoldAll,
                    ..
                },
                ..
            }
        ));
        assert!(
            doc.with_folding_current(member, 17, |current| state.advance_document_phase(
                phase,
                &current,
                Instant::now()
            ))
            .unwrap()
            .unwrap()
        );
        assert!(
            doc.with_folding_current(member, 17, |current| state
                .dispatch_ready_document(&current, Instant::now()))
                .unwrap()
                .unwrap()
        );
        let result = settle_document(&mut state, &doc, member).unwrap();
        let Prepared::Document(prepared) = result.result.unwrap() else {
            panic!("Wrong result kind")
        };
        assert_eq!(prepared.rows().text().to_string(), DOCUMENT_SOURCE);
        assert_eq!(prepared.desired_count(), 1);
    }
}
