//! Bounded, post-publication refactoring saves through real native interests.
//! Save failure never turns a published workspace edit into an unapplied edit.
use super::saving::document_interests::{Admission, Interest, NativeSaveSource, Receipt};
use super::workspace_transactions::{Completion, Origin, TargetReceipt};
use super::*;
use anyhow::{Context as _, ensure};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const QUEUE_WAIT: Duration = Duration::from_secs(30);
struct Member {
    source: NativeSaveSource,
    resource: String,
}
struct Attempt {
    interest: Interest,
    document: u64,
    resource: String,
    needs_path: bool,
    prompted: bool,
}
pub(super) struct Cohort {
    token: u64,
    origin: Origin,
    targets: Vec<TargetReceipt>,
    target_receipts: bool,
    todo: VecDeque<Member>,
    current: Option<Attempt>,
    retired: Arc<AtomicBool>,
    warning: String,
    queued: Instant,
}
fn resource(doc: &Document) -> Result<String> {
    doc.path
        .as_ref()
        .map(|path| crate::lsp::file_uri(path))
        .unwrap_or_else(|| {
            Ok(doc
                .untitled_uri()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("untitled:vscli-{}", doc.id)))
        })
}
impl Cohort {
    pub(super) fn capture(
        app: &App,
        token: u64,
        origin: Origin,
        targets: &[TargetReceipt],
        target_receipts: bool,
    ) -> Result<Self> {
        ensure!(
            token != 0 && targets.len() <= 128,
            "Refactoring save cohort exceeds bounds"
        );
        let mut cohort = Self {
            token,
            origin,
            targets: targets.to_vec(),
            target_receipts,
            todo: VecDeque::new(),
            current: None,
            retired: Arc::new(AtomicBool::new(false)),
            warning: String::new(),
            queued: Instant::now(),
        };
        cohort
            .todo
            .try_reserve(cohort.targets.len())
            .context("Refactoring save cohort reservation refused")?;
        // Capture ALL dirty participants now. Later queue admission consumes
        // these original proofs rather than saving a newer version by accident.
        for target in &cohort.targets {
            let doc = app
                .documents
                .iter()
                .chain(&app.hidden_documents)
                .find(|doc| doc.id == target.document)
                .context("Refactoring participant disappeared after publication")?;
            if doc.dirty() {
                cohort.todo.push_back(Member {
                    source: NativeSaveSource::capture(app, doc.id)?,
                    resource: resource(doc)?,
                });
            }
        }
        Ok(cohort)
    }
    pub(super) fn token(&self) -> u64 {
        self.token
    }
    pub(super) fn retire(&mut self) {
        self.retired.store(true, Ordering::Release);
    }
    fn warn(&mut self, text: &str) {
        if self.warning.len() >= 4096 {
            return;
        }
        if !self.warning.is_empty() {
            self.warning.push_str(" · ");
        }
        let available = 4096 - self.warning.len();
        let mut end = text.len().min(available);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.warning.push_str(&text[..end]);
    }
    fn receipt(&mut self, app: &App, attempt: &Attempt, receipt: Receipt) {
        match receipt {
            Receipt::Committed {
                document,
                save_generation,
            } => {
                let current = app
                    .documents
                    .iter()
                    .chain(&app.hidden_documents)
                    .find(|doc| doc.id == document);
                if let Some(doc) = current
                    && document == attempt.document
                    && doc.save_generation() == save_generation
                {
                    match resource(doc) {
                        Ok(uri) if uri != attempt.resource => {
                            // Only an actual native Save As receipt authorizes a
                            // public mirror's old-URI close/new-URI open transition.
                            for target in &mut self.targets {
                                if target.document == document {
                                    target.saved_resource = Some(uri.clone());
                                    target.saved_generation = Some(save_generation);
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            self.warn(&format!("Saved participant receipt unavailable: {error:#}"))
                        }
                    }
                } else {
                    self.warn("Saved participant mirror changed before refactoring receipt");
                }
            }
            Receipt::NotSaved => self.warn(
                "An automatic refactoring save was canceled or retired; its edits remain applied",
            ),
            Receipt::Failed(error) => self.warn(&format!(
                "Automatic refactoring save failed; edits remain applied: {error}"
            )),
            Receipt::CommittedButUnpublished(error) => self.warn(&format!(
                "Refactoring save committed but its model receipt failed: {error}"
            )),
        }
    }
    fn complete(self) -> Completion {
        Completion {
            token: self.token,
            origin: self.origin,
            applied: true,
            targets: self.targets,
            target_receipts: self.target_receipts,
            reason: (!self.warning.is_empty()).then_some(self.warning),
        }
    }
}
impl App {
    pub(super) fn workspace_transaction_published(&self, token: u64) -> bool {
        self.workspace_transactions
            .refactoring
            .as_ref()
            .is_some_and(|cohort| cohort.token() == token)
            || self
                .workspace_transaction_completion()
                .is_some_and(|done| done.token == token && done.applied)
    }
    pub(super) fn poll_refactoring_saves(&mut self) -> bool {
        let Some(mut cohort) = self.workspace_transactions.refactoring.take() else {
            return false;
        };
        let mut changed = false;
        if !cohort.origin.alive() || cohort.retired.load(Ordering::Acquire) {
            cohort.retire();
            if !cohort.todo.is_empty() {
                cohort.todo.clear();
                cohort
                    .warn("Refactoring save ownership retired; unsaved applied edits are retained");
                changed = true;
            }
            if let Some(attempt) = &cohort.current {
                changed |= self.retire_document_save_interest(attempt.interest);
            }
        }
        if let Some(attempt) = cohort.current.take() {
            if self.refactoring_save_completion(attempt.interest).is_some() {
                let done = self
                    .take_refactoring_save_completion(attempt.interest)
                    .expect("exact native receipt present");
                self.retire_document_save_prompt(attempt.interest);
                cohort.receipt(self, &attempt, done.receipt);
                cohort.queued = Instant::now();
                changed = true;
            } else {
                let mut attempt = attempt;
                if attempt.needs_path
                    && !attempt.prompted
                    && self.prompt.is_none()
                    && self.modal.is_none()
                    && self.pending.is_none()
                {
                    match self.begin_document_save_path_prompt(attempt.interest, attempt.document) {
                        Ok(()) => {
                            attempt.prompted = true;
                            changed = true;
                        }
                        Err(error) => {
                            cohort.warn(&format!("Refactoring Save As could not start: {error:#}"));
                            changed |= self.retire_document_save_interest(attempt.interest);
                        }
                    }
                }
                cohort.current = Some(attempt);
            }
        }
        if cohort.current.is_none() && !cohort.todo.is_empty() {
            if let Err(error) = cohort
                .todo
                .front()
                .expect("nonempty cohort")
                .source
                .current(self)
            {
                cohort.todo.pop_front();
                cohort.warn(&format!(
                    "Deferred refactoring save source changed; edits remain applied: {error:#}"
                ));
                changed = true;
            } else if cohort.queued.elapsed() >= QUEUE_WAIT {
                cohort.todo.clear();
                cohort.warn("Automatic refactoring save queue expired; edits remain applied");
                changed = true;
            } else if self.refactoring_save_lane_available() && self.refactoring_save_capacity() {
                let member = cohort.todo.pop_front().expect("nonempty cohort");
                let document = member.source.document();
                let admitted = self.request_refactoring_save_interest(
                    cohort.token,
                    cohort.origin.lifetime(),
                    cohort.retired.clone(),
                    member.source,
                );
                match admitted {
                    Ok(admission) => {
                        let (interest, needs_path) = match admission {
                            Admission::Enqueued(id) => (id, false),
                            Admission::NeedsPath(id) => (id, true),
                        };
                        cohort.current = Some(Attempt {
                            interest,
                            document,
                            resource: member.resource,
                            needs_path,
                            prompted: false,
                        });
                    }
                    Err(error) => cohort.warn(&format!(
                        "Automatic refactoring save refused; edits remain applied: {error:#}"
                    )),
                }
                changed = true;
            }
        }
        if cohort.current.is_none() && cohort.todo.is_empty() {
            self.workspace_transactions.completed = Some(cohort.complete());
            return true;
        }
        self.workspace_transactions.refactoring = Some(cohort);
        changed
    }
    pub(super) fn retire_refactoring_saves(&mut self) {
        if let Some(cohort) = &mut self.workspace_transactions.refactoring {
            cohort.retire();
        }
    }
}

#[cfg(test)]
mod tests;
