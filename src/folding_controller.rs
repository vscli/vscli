//! Prepared native folding controller core; not yet connected to App/Document.
//! Callers supply borrowed current proofs and publish a checked outcome atomically.
use crate::{
    display_rows::Options,
    document::Selection,
    editor_groups::Membership,
    folding::{self, Region},
    folding_worker::{self, Prepared, Work, Worker},
};
use anyhow::{Context, Result, ensure};
use ropey::Rope;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// A view lifetime or effective options lifetime, distinct from numeric IDs.
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
    pub interaction: u64,
    pub primary: Selection,
    secondary: Vec<Selection>,
}
/// Constructed without a cohort allocation on an ordinary poll/input path.
pub struct CurrentView<'a> {
    pub membership: Membership,
    pub lifetime: &'a Lifetime,
    pub generation: u64,
    pub interaction: u64,
    pub primary: &'a Selection,
    pub secondary: &'a [Selection],
}
pub struct Current<'a> {
    pub model: &'a ModelProof,
    pub view: Option<CurrentView<'a>>,
}
impl ViewProof {
    /// Explicit action admission only; validate bounds BEFORE cloning selections.
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
}
impl Intent {
    pub fn model(&self) -> &ModelProof {
        match self {
            Self::Discover(model) => model,
            Self::Prepare { view, .. } => &view.model,
        }
    }
    pub fn is_current(&self, current: &Current<'_>) -> bool {
        match self {
            Self::Discover(model) => model == current.model,
            Self::Prepare { view, .. } => view.current(current),
        }
    }
    fn work(&self) -> Work {
        match self {
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
        }
    }
}
struct Desired {
    intent: Intent,
    expires: Instant,
}
struct Running {
    token: u64,
    intent: Intent,
    expires: Instant,
}
pub struct Outcome {
    pub intent: Intent,
    pub result: std::result::Result<Prepared, String>,
}
#[derive(Default)]
pub struct State {
    worker: Worker,
    desired: Option<Desired>,
    running: Option<Running>,
    stopped: bool,
    notice: Option<&'static str>,
}
impl State {
    pub fn occupied(&self) -> bool {
        self.worker.occupied()
    }
    pub fn desired(&self) -> Option<&Intent> {
        self.desired.as_ref().map(|desired| &desired.intent)
    }
    /// Resolve current proofs from this retained origin, not whichever editor
    /// happens to be active after the worker was submitted.
    pub fn interest(&self) -> Option<&Intent> {
        self.running
            .as_ref()
            .map(|running| &running.intent)
            .or_else(|| self.desired())
    }
    pub fn take_notice(&mut self) -> Option<&'static str> {
        self.notice.take()
    }
    pub fn resolving(&self) -> bool {
        self.desired.is_some() || self.running.is_some()
    }
    /// One latest metadata intent. Repetition coalesces without extending the
    /// first action deadline; replacement cancels logically, not physically.
    pub fn request(&mut self, intent: Intent, now: Instant) -> Result<bool> {
        ensure!(!self.stopped, "Folding controller stopped");
        validate_model(intent.model())?;
        if let Intent::Prepare {
            view,
            options,
            collapsed,
        } = &intent
        {
            ensure!(
                collapsed.len() <= folding::MAX_REGIONS,
                "Folding action exceeds 5,000 regions"
            );
            ensure!(
                view.secondary.len() < folding_worker::MAX_SELECTIONS,
                "Folding action exceeds 10,000 selections"
            );
            ensure!(
                view.membership.document == view.model.document
                    && options.tab_size == view.model.tab_size,
                "Folding action model/options mismatch"
            );
            ensure!(
                options.wrap == crate::display_rows::Wrap::Off,
                "Wrapping is not part of native folding"
            );
        }
        let expires = now
            .checked_add(folding_worker::DEADLINE)
            .context("Folding action deadline overflow")?;
        if self
            .desired
            .as_ref()
            .is_some_and(|desired| desired.intent == intent)
            || self
                .running
                .as_ref()
                .is_some_and(|running| running.intent == intent)
        {
            return Ok(false);
        }
        self.cancel_running();
        self.notice = None;
        self.desired = Some(Desired { intent, expires });
        Ok(true)
    }
    /// Called in App.poll only, after checking actual capacity and recapturing
    /// the selected retained model's immutable current Rope. No scans run here.
    pub fn dispatch(&mut self, current: &Current<'_>, text: Rope, now: Instant) -> Result<bool> {
        if self.stopped || !self.worker.available() {
            return Ok(false);
        }
        let Some(desired) = &self.desired else {
            return Ok(false);
        };
        if now >= desired.expires || !desired.intent.is_current(current) {
            self.notice = Some(if now >= desired.expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            self.desired = None;
            return Ok(false);
        }
        let token = match self.worker.start(text, desired.intent.work(), now) {
            Ok(token) => token,
            Err(error) => {
                // A malformed/over-budget source or spawn error is inert until
                // another explicit demand, not an automatic per-poll retry loop.
                self.desired = None;
                return Err(error);
            }
        };
        let desired = self.desired.take().expect("one owned intent");
        self.running = Some(Running {
            token,
            intent: desired.intent,
            expires: desired.expires,
        });
        Ok(true)
    }
    /// Check borrowed ownership before publishing, including edit→Undo and view
    /// or options A→B→A. The result is returned only after actual worker join.
    pub fn poll(&mut self, current: &Current<'_>, now: Instant) -> Option<Outcome> {
        if self
            .desired
            .as_ref()
            .is_some_and(|desired| now >= desired.expires || !desired.intent.is_current(current))
        {
            self.notice = Some(if now >= self.desired.as_ref().unwrap().expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            self.desired = None;
        }
        if self
            .running
            .as_ref()
            .is_some_and(|running| now >= running.expires || !running.intent.is_current(current))
        {
            self.notice = Some(if now >= self.running.as_ref().unwrap().expires {
                "Folding request expired; text retained"
            } else {
                "Folding request retired: its source or view changed"
            });
            self.cancel_running();
        }
        let reply = self.worker.poll()?;
        let running = self.running.take()?;
        if reply.token != running.token
            || !running.intent.is_current(current)
            || now >= running.expires
        {
            return None;
        }
        Some(Outcome {
            intent: running.intent,
            result: reply.result,
        })
    }
    pub fn retire(&mut self) {
        self.desired = None;
        self.cancel_running();
    }
    /// Drain a logically retired job even after its last model/view was removed.
    /// No current source proof is needed because its result cannot be published.
    /// Capacity is released only after the worker has actually exited and joined.
    pub fn poll_retired(&mut self) -> bool {
        self.running.is_none() && self.worker.poll().is_some()
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<()> {
        self.stopped = true;
        self.retire();
        self.worker.shutdown(timeout)
    }
    fn cancel_running(&mut self) {
        if let Some(running) = self.running.take() {
            self.worker.cancel(running.token);
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
}
