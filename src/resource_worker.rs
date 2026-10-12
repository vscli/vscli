//! Two-phase native filesystem work. Preparation holds its original deadline;
//! only explicit UI authorization admits effects. An authorized worker always
//! settles and joins, even after its caller retires. No JavaScript is involved.
use anyhow::{Context, Result, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

/// Implementations must bound their captured inventory, preparation reads,
/// descriptors and receipts. Both methods execute exclusively on the worker.
/// `prepare` must retain original filesystem proofs and check its control during
/// bounded reads; `commit` must recheck those proofs before filesystem effects.
pub trait Transaction: Send + 'static {
    type Prepared;
    type Info: Send + 'static;
    type Receipt: Send + 'static;

    fn prepare(self, control: &Preparation) -> Result<(Self::Prepared, Self::Info)>;
    fn commit(prepared: Self::Prepared) -> Result<Self::Receipt>;
}

pub struct Preparation {
    canceled: Arc<AtomicBool>,
    deadline: Instant,
}
impl Preparation {
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.canceled.load(Ordering::Acquire) && Instant::now() < self.deadline,
            "Resource preparation retired or expired"
        );
        Ok(())
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
    /// Native coordinators may share this retirement flag with bounded journal
    /// reads. It is a one-way cancellation signal, never a renewable lease.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.canceled
    }
}

#[derive(Debug)]
pub enum Outcome<R> {
    Rejected,
    Expired,
    Committed(R),
}
#[derive(Debug)]
pub enum Event<I, R> {
    Prepared {
        id: u64,
        info: I,
        expires_at: Instant,
    },
    Finished {
        id: u64,
        /// The logical owner retired; this never means an authorized write
        /// was rolled back, nor that a reported failure had no physical effects.
        retired: bool,
        authorized: bool,
        result: std::result::Result<Outcome<R>, String>,
    },
}
enum Message<I, R> {
    Prepared(I),
    Finished {
        authorized: bool,
        result: std::result::Result<Outcome<R>, String>,
    },
}
enum Decision {
    Authorize,
    Reject,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Awaiting,
    Committing,
    Retiring,
    Finishing,
}
struct Terminal<R> {
    authorized: bool,
    result: std::result::Result<Outcome<R>, String>,
}
struct Pending<I, R> {
    id: u64,
    deadline: Instant,
    phase: Phase,
    retired: bool,
    authorization_issued: bool,
    canceled: Arc<AtomicBool>,
    decisions: SyncSender<Decision>,
    receiver: Receiver<Message<I, R>>,
    thread: JoinHandle<()>,
    terminal: Option<Terminal<R>>,
}

pub struct Worker<T: Transaction> {
    last_id: u64,
    pending: Option<Pending<T::Info, T::Receipt>>,
    #[cfg(test)]
    after_reply: Option<TestGate>,
}
impl<T: Transaction> Default for Worker<T> {
    fn default() -> Self {
        Self {
            last_id: 0,
            pending: None,
            #[cfg(test)]
            after_reply: None,
        }
    }
}
fn reason(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
impl<T: Transaction> Worker<T> {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn awaiting_authorization(&self, id: u64) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.id == id
                && pending.phase == Phase::Awaiting
                && !pending.retired
                && Instant::now() < pending.deadline
        })
    }
    /// The deadline comes from original receipt admission. FIFO waiting or an
    /// observation timeout must never replace it with a fresh authorization.
    pub fn start(&mut self, id: u64, transaction: T, deadline: Instant) -> Result<()> {
        ensure!(!self.busy(), "Resource worker is actually occupied");
        ensure!(id > self.last_id, "Resource worker identity must increase");
        ensure!(
            Instant::now() < deadline,
            "Original resource deadline expired"
        );
        let (sender, receiver) = mpsc::sync_channel(2);
        let (decisions, control) = mpsc::sync_channel(1);
        let canceled = Arc::new(AtomicBool::new(false));
        let preparation = Preparation {
            canceled: canceled.clone(),
            deadline,
        };
        #[cfg(test)]
        let after_reply = self.after_reply.take();
        let worker = thread::Builder::new()
            .name("vscli-resource-operation".into())
            .spawn(move || {
                let mut authorized = false;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    preparation.check()?;
                    let (prepared, info) = transaction.prepare(&preparation)?;
                    if preparation.canceled.load(Ordering::Acquire) {
                        return Ok(Outcome::Rejected);
                    }
                    if Instant::now() >= deadline {
                        return Ok(Outcome::Expired);
                    }
                    if sender.send(Message::Prepared(info)).is_err() {
                        return Ok(Outcome::Rejected);
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match control.recv_timeout(remaining) {
                        Ok(Decision::Authorize) => {
                            // An accepted decision is irrevocable. Owner retirement
                            // after authorization must not silently cancel a commit.
                            authorized = true;
                            T::commit(prepared).map(Outcome::Committed)
                        }
                        Ok(Decision::Reject) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            Ok(Outcome::Rejected)
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => Ok(Outcome::Expired),
                    }
                }))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("Resource worker panicked; outcome unknown"))
                })
                .map_err(|error| reason(format!("{error:#}")));
                let _ = sender.send(Message::Finished { authorized, result });
                #[cfg(test)]
                if let Some(gate) = after_reply {
                    let _ = gate.started.send(());
                    let _ = gate.release.recv();
                }
            })
            .context("Cannot start resource operation worker")?;
        self.last_id = id;
        self.pending = Some(Pending {
            id,
            deadline,
            phase: Phase::Preparing,
            retired: false,
            authorization_issued: false,
            canceled,
            decisions,
            receiver,
            thread: worker,
            terminal: None,
        });
        Ok(())
    }
    /// Call only after validating the exact original native caller/model proofs.
    /// This method cannot infer authorization from a public URI or request ID.
    pub fn authorize(&mut self, id: u64) -> Result<()> {
        ensure!(
            self.awaiting_authorization(id),
            "Resource authorization is stale, expired or not awaiting a decision"
        );
        let pending = self.pending.as_mut().expect("awaiting exact resource");
        pending
            .decisions
            .try_send(Decision::Authorize)
            .context("Resource authorization channel unavailable")?;
        pending.phase = Phase::Committing;
        pending.authorization_issued = true;
        Ok(())
    }
    /// Logical retirement retains the occupied slot through actual exit/join.
    /// Already-authorized effects continue toward their real terminal receipt.
    pub fn retire(&mut self, id: u64) -> bool {
        let Some(pending) = self.pending.as_mut().filter(|pending| pending.id == id) else {
            return false;
        };
        pending.retired = true;
        if matches!(pending.phase, Phase::Preparing | Phase::Awaiting) {
            pending.canceled.store(true, Ordering::Release);
            let _ = pending.decisions.try_send(Decision::Reject);
            pending.phase = Phase::Retiring;
        }
        true
    }
    pub fn poll(&mut self) -> Option<Event<T::Info, T::Receipt>> {
        let pending = self.pending.as_mut()?;
        if matches!(pending.phase, Phase::Preparing | Phase::Awaiting)
            && Instant::now() >= pending.deadline
        {
            pending.canceled.store(true, Ordering::Release);
            let _ = pending.decisions.try_send(Decision::Reject);
            pending.phase = Phase::Retiring;
        }
        // At most two bounded messages exist for this one original worker.
        for _ in 0..2 {
            match pending.receiver.try_recv() {
                Ok(Message::Prepared(info)) => {
                    if pending.phase == Phase::Preparing && Instant::now() < pending.deadline {
                        pending.phase = Phase::Awaiting;
                        return Some(Event::Prepared {
                            id: pending.id,
                            info,
                            expires_at: pending.deadline,
                        });
                    }
                }
                Ok(Message::Finished { authorized, result }) => {
                    pending.phase = Phase::Finishing;
                    pending.terminal = Some(Terminal { authorized, result });
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        if !pending.thread.is_finished() {
            return None;
        }
        // Joining a positively finished handle never waits on filesystem work.
        // The slot remains occupied until this exact terminal receipt is taken.
        let mut pending = self.pending.take().expect("finished original resource");
        let joined = pending.thread.join();
        for _ in 0..2 {
            if let Ok(Message::Finished { authorized, result }) = pending.receiver.try_recv() {
                pending.terminal = Some(Terminal { authorized, result });
            }
        }
        Some(terminal_event(
            pending.id,
            pending.retired,
            pending.authorization_issued,
            pending.terminal,
            joined.is_ok(),
        ))
    }
    /// Shutdown-only join. Never call from rendering/input or on advisory timeout.
    pub fn settle(&mut self) -> Option<Event<T::Info, T::Receipt>> {
        let id = self.pending.as_ref()?.id;
        self.retire(id);
        let mut pending = self.pending.take().expect("original resource settling");
        let joined = pending.thread.join();
        for _ in 0..2 {
            if let Ok(Message::Finished { authorized, result }) = pending.receiver.try_recv() {
                pending.terminal = Some(Terminal { authorized, result });
            }
        }
        Some(terminal_event(
            pending.id,
            pending.retired,
            pending.authorization_issued,
            pending.terminal,
            joined.is_ok(),
        ))
    }
}
fn terminal_event<I, R>(
    id: u64,
    retired: bool,
    authorization_issued: bool,
    terminal: Option<Terminal<R>>,
    joined: bool,
) -> Event<I, R> {
    let terminal = terminal.unwrap_or(Terminal {
        authorized: authorization_issued,
        result: Err(
            "Resource worker disconnected without its terminal receipt; outcome unknown".into(),
        ),
    });
    Event::Finished {
        id,
        retired,
        authorized: terminal.authorized,
        result: if joined {
            terminal.result
        } else {
            Err("Resource worker could not join; outcome unknown".into())
        },
    }
}
impl<T: Transaction> Drop for Worker<T> {
    fn drop(&mut self) {
        let _ = self.settle();
    }
}

#[cfg(test)]
struct TestGate {
    started: SyncSender<()>,
    release: Receiver<()>,
}

#[cfg(test)]
mod tests;
