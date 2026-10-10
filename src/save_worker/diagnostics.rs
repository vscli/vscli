//! Test-only actual worker progress. No I/O, control messages or persistence decisions.
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};

const CAPACITY: usize = 64;

macro_rules! stages {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub(crate) enum Stage { $($variant),+ }
        impl Stage {
            fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
            fn from_code(code: u8) -> Option<Self> {
                $(if code == Self::$variant as u8 { return Some(Self::$variant); })+
                None
            }
        }
    };
}
stages! {
    WorkerEntered => "worker_entered",
    PreparedSending => "prepared_sending",
    PreparedSent => "prepared_sent",
    AuthorizationReceiving => "authorization_receiving",
    AuthorizationReceived => "authorization_received",
    AuthorizationRejected => "authorization_rejected",
    AuthorizationExpired => "authorization_expired",
    CommitCalling => "commit_calling",
    CommitCallReturned => "commit_call_returned",
    RecheckBegin => "recheck_begin",
    RecheckReturned => "recheck_returned",
    NoClobberBegin => "noclobber_api_begin",
    NoClobberReturned => "noclobber_api_returned",
    ReplaceBegin => "replace_api_begin",
    ReplaceReturned => "replace_api_returned",
    TemporaryCleanupBegin => "temporary_cleanup_begin",
    TemporaryCleanupReturned => "temporary_cleanup_returned",
    LockUnlockBegin => "lock_unlock_begin",
    LockUnlockReturned => "lock_unlock_returned",
    WorkCommitted => "work_committed",
    WorkRejected => "work_rejected",
    WorkExpired => "work_expired",
    WorkError => "work_error",
    TerminalSending => "terminal_sending",
    TerminalSent => "terminal_send_returned",
    WorkerScopeExit => "worker_scope_exit",
}

struct Slot {
    ready: AtomicBool,
    stage: AtomicU8,
    micros: AtomicU64,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            ready: AtomicBool::new(false),
            stage: AtomicU8::new(0),
            micros: AtomicU64::new(0),
        }
    }
}
pub(super) struct Trace {
    started: Instant,
    used: AtomicUsize,
    truncated: AtomicBool,
    slots: [Slot; CAPACITY],
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            used: AtomicUsize::new(0),
            truncated: AtomicBool::new(false),
            slots: std::array::from_fn(|_| Slot::default()),
        }
    }
}
impl Trace {
    fn record(&self, stage: Stage) {
        self.record_at(
            stage,
            self.started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
    }
    fn record_at(&self, stage: Stage, micros: u64) {
        let Ok(index) = self
            .used
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                (used < CAPACITY).then_some(used + 1)
            })
        else {
            self.truncated.store(true, Ordering::Relaxed);
            return;
        };
        let slot = &self.slots[index];
        slot.stage.store(stage as u8, Ordering::Relaxed);
        slot.micros.store(micros, Ordering::Relaxed);
        // A reader never interprets a reserved but incompletely published slot.
        slot.ready.store(true, Ordering::Release);
    }
    pub(super) fn snapshot(&self) -> Vec<(Stage, u64)> {
        self.slots[..self.used.load(Ordering::Relaxed).min(CAPACITY)]
            .iter()
            .filter_map(|slot| {
                if !slot.ready.load(Ordering::Acquire) {
                    return None;
                }
                Stage::from_code(slot.stage.load(Ordering::Relaxed))
                    .map(|stage| (stage, slot.micros.load(Ordering::Relaxed)))
            })
            .collect()
    }
    pub(super) fn status(&self) -> String {
        let records = self
            .snapshot()
            .into_iter()
            .map(|(stage, micros)| format!("{}@{}us", stage.name(), micros))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "progress[{records}]; truncated={}",
            self.truncated.load(Ordering::Relaxed)
        )
    }
}

std::thread_local! {
    static CURRENT: RefCell<Option<Arc<Trace>>> = const { RefCell::new(None) };
}

pub(super) struct Scope {
    previous: Option<Arc<Trace>>,
    installed: bool,
}
impl Scope {
    pub(super) fn enter(trace: Arc<Trace>) -> Self {
        let mut previous = None;
        let installed = CURRENT
            .try_with(|cell| {
                cell.try_borrow_mut().is_ok_and(|mut current| {
                    previous = current.replace(trace);
                    true
                })
            })
            .unwrap_or(false);
        if installed {
            mark(Stage::WorkerEntered);
        }
        Self {
            previous,
            installed,
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if !self.installed {
            return;
        }
        mark(Stage::WorkerScopeExit);
        // Never panic during an existing worker unwind or TLS teardown.
        let _ = CURRENT.try_with(|cell| {
            if let Ok(mut current) = cell.try_borrow_mut() {
                *current = self.previous.take();
            }
        });
    }
}

pub(crate) fn mark(stage: Stage) {
    let _ = CURRENT.try_with(|cell| {
        if let Ok(current) = cell.try_borrow()
            && let Some(trace) = current.as_ref()
        {
            trace.record(stage);
        }
    });
}

/// End means the bracket returned or unwound, not that the operation succeeded.
pub(crate) struct Span(Stage);
impl Span {
    pub(crate) fn enter(begin: Stage, end: Stage) -> Self {
        mark(begin);
        Self(end)
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        mark(self.0);
    }
}

#[test]
fn overflow_is_bounded_without_replacing_original_records() {
    let trace = Trace::default();
    for index in 0..CAPACITY + 7 {
        trace.record_at(Stage::NoClobberBegin, index as u64);
    }
    let actual = trace.snapshot();
    assert_eq!(actual.len(), CAPACITY);
    assert_eq!(actual.first(), Some(&(Stage::NoClobberBegin, 0)));
    assert_eq!(
        actual.last(),
        Some(&(Stage::NoClobberBegin, (CAPACITY - 1) as u64))
    );
    assert_eq!(trace.used.load(Ordering::Relaxed), CAPACITY);
    assert!(trace.truncated.load(Ordering::Relaxed));
}

#[test]
fn reserved_incomplete_slot_is_not_an_observed_operation() {
    let trace = Trace::default();
    trace.used.store(1, Ordering::Relaxed);
    trace.slots[0]
        .stage
        .store(Stage::NoClobberReturned as u8, Ordering::Relaxed);
    assert!(trace.snapshot().is_empty());
    trace.slots[0].micros.store(23, Ordering::Relaxed);
    trace.slots[0].ready.store(true, Ordering::Release);
    assert_eq!(trace.snapshot(), [(Stage::NoClobberReturned, 23)]);
}

#[test]
fn actual_threads_are_isolated_and_unwind_restores_the_previous_trace() {
    let first = Arc::new(Trace::default());
    let second = Arc::new(Trace::default());
    let worker = {
        let first = first.clone();
        std::thread::spawn(move || {
            let _scope = Scope::enter(first);
            mark(Stage::AuthorizationReceived);
            let result = std::panic::catch_unwind(|| {
                let _span = Span::enter(Stage::NoClobberBegin, Stage::NoClobberReturned);
                panic!("diagnostic unwind fixture");
            });
            assert!(result.is_err());
        })
    };
    {
        let _scope = Scope::enter(second.clone());
        mark(Stage::WorkRejected);
    }
    worker.join().unwrap();
    let first_stages = first
        .snapshot()
        .into_iter()
        .map(|r| r.0)
        .collect::<Vec<_>>();
    assert_eq!(
        first_stages,
        [
            Stage::WorkerEntered,
            Stage::AuthorizationReceived,
            Stage::NoClobberBegin,
            Stage::NoClobberReturned,
            Stage::WorkerScopeExit
        ]
    );
    let second_before = second.snapshot();
    assert_eq!(
        second_before.iter().map(|r| r.0).collect::<Vec<_>>(),
        [
            Stage::WorkerEntered,
            Stage::WorkRejected,
            Stage::WorkerScopeExit
        ]
    );
    mark(Stage::WorkError);
    assert_eq!(second.snapshot(), second_before);
}

#[test]
fn nested_scope_unwind_restores_original_worker_ownership() {
    let original = Arc::new(Trace::default());
    let nested = Arc::new(Trace::default());
    {
        let _scope = Scope::enter(original.clone());
        assert!(
            std::panic::catch_unwind(|| {
                let _nested = Scope::enter(nested.clone());
                let _span = Span::enter(Stage::ReplaceBegin, Stage::ReplaceReturned);
                panic!("nested trace unwind");
            })
            .is_err()
        );
        mark(Stage::WorkCommitted);
    }
    assert_eq!(
        original.snapshot().iter().map(|r| r.0).collect::<Vec<_>>(),
        [
            Stage::WorkerEntered,
            Stage::WorkCommitted,
            Stage::WorkerScopeExit
        ]
    );
    assert_eq!(
        nested.snapshot().iter().map(|r| r.0).collect::<Vec<_>>(),
        [
            Stage::WorkerEntered,
            Stage::ReplaceBegin,
            Stage::ReplaceReturned,
            Stage::WorkerScopeExit
        ]
    );
}
