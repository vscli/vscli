//! One actually occupied native folding worker. No App or view mutation.
use crate::{
    display_rows::{DisplayRows, Options},
    folding::{self, Region},
};
use anyhow::{Context, Result, bail, ensure};
use ropey::Rope;
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const MAX_SELECTIONS: usize = 10_000;
pub const DEADLINE: Duration = Duration::from_secs(6);

pub enum Work {
    Discover {
        tab_size: u8,
    },
    Prepare {
        options: Options,
        collapsed: Arc<[Region]>,
        protected: Arc<[Range<usize>]>,
    },
}
pub enum Prepared {
    Catalog(Arc<[Region]>),
    Rows(DisplayRows),
}
pub struct Reply {
    pub token: u64,
    pub result: std::result::Result<Prepared, String>,
}
struct Active {
    token: u64,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<std::result::Result<Prepared, String>>,
    thread: JoinHandle<()>,
}
#[derive(Default)]
pub struct Worker {
    token: u64,
    active: Option<Active>,
    stopping: bool,
    #[cfg(test)]
    gates: Option<Gates>,
}
impl Worker {
    pub fn available(&self) -> bool {
        !self.stopping && self.active.is_none()
    }
    pub fn occupied(&self) -> bool {
        self.active.is_some()
    }

    /// All validation and ID checks precede spawning or live slot mutation.
    pub fn start(&mut self, text: Rope, work: Work, now: Instant) -> Result<u64> {
        ensure!(
            self.available(),
            "Folding worker is actually occupied or stopped"
        );
        validate(&text, &work)?;
        let token = self
            .token
            .checked_add(1)
            .context("Folding request ID exhausted")?;
        let deadline = now
            .checked_add(DEADLINE)
            .context("Folding deadline overflow")?;
        let cancel = Arc::new(AtomicBool::new(false));
        let own_cancel = cancel.clone();
        let (sender, result) = mpsc::sync_channel(1);
        #[cfg(test)]
        let gates = self.gates.take();
        let thread = thread::Builder::new()
            .name("vscli-folding".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(gates) = &gates {
                    gates.before.wait();
                }
                let prepared = prepare(text, work, &own_cancel, deadline)
                    .map_err(|error| format!("{error:#}"));
                let _ = sender.send(prepared);
                // A received message alone does NOT acknowledge thread settlement.
                #[cfg(test)]
                if let Some(gates) = gates {
                    gates.after.wait();
                }
            })
            .context("Cannot start folding worker")?;
        self.token = token;
        self.active = Some(Active {
            token,
            cancel,
            result,
            thread,
        });
        Ok(token)
    }
    pub fn cancel(&mut self, token: u64) {
        if let Some(active) = &self.active
            && active.token == token
        {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn poll(&mut self) -> Option<Reply> {
        if !self.active.as_ref()?.thread.is_finished() {
            return None;
        }
        let active = self.active.take()?;
        let result = match active.thread.join() {
            Ok(()) => active
                .result
                .try_recv()
                .unwrap_or_else(|_| Err("Folding worker exited without a result".into())),
            Err(_) => Err("Folding worker panicked".into()),
        };
        Some(Reply {
            token: active.token,
            result,
        })
    }
    /// Explicit shutdown only. A timed-out detached thread retains its own Rope
    /// and cancellation state; no replacement work can be launched on this worker.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<()> {
        self.stopping = true;
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .context("Folding shutdown deadline overflow")?;
        while self.occupied() {
            if self.poll().is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "Folding shutdown timed out; canceled worker remains owned until actual exit"
                );
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Drop never blocks an input/render path; explicit shutdown joins first.
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn validate(text: &Rope, work: &Work) -> Result<()> {
    ensure!(
        text.len_bytes() <= folding::MAX_BYTES && text.len_lines() <= folding::MAX_LINES,
        "Folding work exceeds 2 MiB or 100,000 lines"
    );
    match work {
        Work::Discover { tab_size } => {
            ensure!((1..=16).contains(tab_size), "Invalid folding tab size")
        }
        Work::Prepare {
            options,
            collapsed,
            protected,
        } => {
            ensure!(
                options.wrap == crate::display_rows::Wrap::Off
                    && (1..=16).contains(&options.tab_size),
                "Invalid native folding options"
            );
            ensure!(
                collapsed.len() <= folding::MAX_REGIONS,
                "Folding work exceeds 5,000 regions"
            );
            ensure!(
                protected.len() <= MAX_SELECTIONS,
                "Folding work exceeds 10,000 selections"
            );
            ensure!(
                protected
                    .iter()
                    .all(|range| range.start <= range.end && range.end <= text.len_chars()),
                "Folding selections are outside the source"
            );
        }
    }
    Ok(())
}
fn guard(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "Folding work canceled");
    ensure!(Instant::now() < deadline, "Folding work deadline exceeded");
    Ok(())
}
fn prepare(text: Rope, work: Work, cancel: &AtomicBool, deadline: Instant) -> Result<Prepared> {
    guard(cancel, deadline)?;
    match work {
        Work::Discover { tab_size } => Ok(Prepared::Catalog(
            folding::discover(&text, usize::from(tab_size), cancel, deadline)?.into(),
        )),
        Work::Prepare {
            options,
            collapsed,
            protected,
        } => {
            let rows = DisplayRows::prepare_folded(text.clone(), options, &collapsed)?;
            guard(cancel, deadline)?;
            // Closed selection intervals preserve the conservative foundation's
            // endpoint protection. Merge once; never regions × all selections.
            let mut selections = protected.to_vec();
            selections.sort_unstable_by_key(|range| (range.start, range.end));
            let mut merged: Vec<Range<usize>> = Vec::with_capacity(selections.len());
            for (index, range) in selections.into_iter().enumerate() {
                if index % 512 == 0 {
                    guard(cancel, deadline)?;
                }
                if let Some(previous) = merged.last_mut()
                    && range.start <= previous.end
                {
                    previous.end = previous.end.max(range.end);
                } else {
                    merged.push(range);
                }
            }
            for region in collapsed.iter() {
                guard(cancel, deadline)?;
                let chars = region.characters();
                let header = text.char_to_line(chars.start);
                let hidden = text.line_to_char(header + 1)..chars.end;
                let index = merged.partition_point(|range| range.start < hidden.end);
                ensure!(
                    index == 0 || merged[index - 1].end < hidden.start,
                    "Collapsed region would hide a protected selection"
                );
            }
            guard(cancel, deadline)?;
            Ok(Prepared::Rows(rows))
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct Gate(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
#[cfg(test)]
impl Gate {
    pub(crate) fn held() -> Self {
        Self(Arc::new((
            std::sync::Mutex::new(false),
            std::sync::Condvar::new(),
        )))
    }
    pub(crate) fn release(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
    fn wait(&self) {
        let (released, _) = self
            .0
            .1
            .wait_timeout_while(
                self.0.0.lock().unwrap(),
                Duration::from_secs(8),
                |released| !*released,
            )
            .unwrap();
        assert!(*released, "Actual folding worker gate was not released");
    }
}
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct Gates {
    pub(crate) before: Gate,
    pub(crate) after: Gate,
}
#[cfg(test)]
impl Worker {
    pub(crate) fn hold(&mut self, gates: Gates) {
        self.gates = Some(gates);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_rows::Wrap;
    pub(crate) fn settle(worker: &mut Worker) -> Reply {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(reply) = worker.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "Actual worker did not settle");
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn canceled_or_reply_sent_thread_retains_actual_slot_until_join() {
        let mut worker = Worker::default();
        let gates = Gates {
            before: Gate::held(),
            after: Gate::held(),
        };
        worker.hold(gates.clone());
        let token = worker
            .start(
                Rope::from_str("root\n body\nafter"),
                Work::Discover { tab_size: 4 },
                Instant::now(),
            )
            .unwrap();
        worker.cancel(token);
        assert!(worker.occupied());
        assert!(!worker.available());
        assert!(
            worker
                .start(Rope::new(), Work::Discover { tab_size: 4 }, Instant::now())
                .is_err()
        );
        gates.before.release();
        let deadline = Instant::now() + Duration::from_secs(8);
        while worker.active.as_ref().unwrap().result.try_recv().is_err() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(worker.poll().is_none());
        assert!(!worker.available());
        gates.after.release();
        // The test deliberately consumed the result above; actual-slot release
        // still occurs only after join, with an honest missing-result error.
        assert!(settle(&mut worker).result.is_err());
        assert!(worker.available());
    }
    #[test]
    fn admission_failure_and_exhausted_ids_preserve_slot_and_source() {
        let mut worker = Worker {
            token: u64::MAX,
            active: None,
            stopping: false,
            gates: None,
        };
        assert!(
            worker
                .start(Rope::new(), Work::Discover { tab_size: 4 }, Instant::now())
                .is_err()
        );
        assert!(worker.available());
        assert_eq!(worker.token, u64::MAX);
        worker.token = 3;
        assert!(
            worker
                .start(
                    Rope::from_str(&"x".repeat(folding::MAX_BYTES + 1)),
                    Work::Discover { tab_size: 4 },
                    Instant::now()
                )
                .is_err()
        );
        assert!(worker.available());
        assert_eq!(worker.token, 3);
        assert!(
            worker
                .start(Rope::new(), Work::Discover { tab_size: 0 }, Instant::now())
                .is_err()
        );
        assert_eq!(worker.token, 3);
    }
    #[test]
    fn merged_selection_guard_matches_independent_foundation_policy() {
        let text = Rope::from_str("head\n body猫\n tail\nafter");
        let region = Region::from_lines(&text, 0, 2).unwrap();
        for selections in [
            std::iter::once(0..0).collect(),
            std::iter::once(5..5).collect(),
            std::iter::once(0..5).collect(),
            std::iter::once(0..text.len_chars()).collect(),
            vec![20..20, 0..0],
        ] {
            let expected = region.can_collapse(&text, &selections).unwrap();
            let work = Work::Prepare {
                options: Options {
                    wrap: Wrap::Off,
                    width: 80,
                    tab_size: 4,
                },
                collapsed: vec![region.clone()].into(),
                protected: selections.into(),
            };
            let actual = prepare(
                text.clone(),
                work,
                &AtomicBool::new(false),
                Instant::now() + DEADLINE,
            );
            assert_eq!(actual.is_ok(), expected);
        }
        assert_eq!(text.to_string(), "head\n body猫\n tail\nafter");
    }
    #[test]
    fn shutdown_deadline_does_not_release_or_accept_replacement_work() {
        let mut worker = Worker::default();
        let gates = Gates {
            before: Gate::held(),
            after: Gate::held(),
        };
        worker.hold(gates.clone());
        worker
            .start(
                Rope::from_str("root\n body"),
                Work::Discover { tab_size: 4 },
                Instant::now(),
            )
            .unwrap();
        assert!(worker.shutdown(Duration::ZERO).is_err());
        assert!(worker.occupied());
        assert!(!worker.available());
        gates.before.release();
        gates.after.release();
        assert!(settle(&mut worker).result.is_err());
        assert!(!worker.available());
        worker.shutdown(Duration::from_secs(1)).unwrap();
    }
}
