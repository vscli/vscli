//! Native will-file-operation coordination. Original returned DTOs survive
//! collection, review and worker preparation; publication uses native model proofs.
use super::*;
use crate::{
    extensions::FileParticipationTicket,
    file_edit_review::{
        Initial, Overview,
        dialog::{Dialog, Event as ReviewEvent},
    },
    files::{Action, Job, Preflight},
    resource_worker::{Event as WorkerEvent, Outcome, Preparation, Transaction, Worker},
    workspace_edit_stream::{Entry, Stream},
};
use anyhow::{Context, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

enum Phase {
    Preflight,
    Callback,
    PreparingReview,
    Review(Arc<Overview>),
    PreparingApply,
    Applying(u64),
    Ready,
}

#[cfg(test)]
mod tests;
struct Run {
    id: u64,
    preparation: u64,
    action: Action,
    root: PathBuf,
    profile: u64,
    session: u64,
    ticket: Option<FileParticipationTicket>,
    preflight: Option<Arc<Preflight>>,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    timeout: Duration,
    lifetime: Arc<AtomicBool>,
    names: Vec<String>,
    phase: Phase,
}
#[derive(Default)]
pub(super) struct State {
    next: u64,
    preparation_next: u64,
    run: Option<Run>,
    worker: Worker<Work>,
    retired_transaction: Option<u64>,
    notice: Option<String>,
}
struct PrepareReview {
    original: Arc<Value>,
    store: Option<crate::file_edit_review::preferences::Store>,
    indices: Option<Vec<usize>>,
    preflight: Arc<Preflight>,
    sources: Vec<(String, ropey::Rope)>,
}
enum Work {
    Preflight(Action),
    Review(PrepareReview),
}
enum PreparedReview {
    Preflight(Arc<Preflight>),
    Review {
        overview: Arc<Overview>,
        remembered: Option<bool>,
        preview: Arc<crate::file_edit_review::preview::Preview>,
    },
    Apply(Value),
}
impl Transaction for Work {
    type Prepared = PreparedReview;
    type Info = ();
    type Receipt = PreparedReview;
    fn prepare(self, control: &Preparation) -> Result<(PreparedReview, ())> {
        control.check()?;
        let self_ = match self {
            Self::Preflight(action) => {
                let original = crate::resource_journal::Control::new(
                    control.cancel_flag(),
                    control.deadline(),
                );
                return Ok((
                    PreparedReview::Preflight(Arc::new(Preflight::prepare(action, &original)?)),
                    (),
                ));
            }
            Self::Review(work) => work,
        };
        let original =
            crate::resource_journal::Control::new(control.cancel_flag(), control.deadline());
        self_.preflight.verify(&original)?;
        let prepared = if let Some(indices) = self_.indices {
            PreparedReview::Apply(text_arguments(&self_.original, &indices)?)
        } else {
            let remembered = self_
                .store
                .map(|store| store.load(control))
                .transpose()?
                .flatten();
            PreparedReview::Review {
                preview: Arc::new(crate::file_edit_review::preview::prepare(
                    &self_.original,
                    self_.sources,
                    &original,
                )?),
                overview: Arc::new(Overview::prepare(self_.original)?),
                remembered,
            }
        };
        control.check()?;
        Ok((prepared, ()))
    }
    fn commit(prepared: PreparedReview) -> Result<PreparedReview> {
        Ok(prepared)
    }
}

/// An apply backend consumes explicit original indices. Unsupported selected
/// entries fail the entire additional edit before native text publication.
fn text_arguments(original: &Value, indices: &[usize]) -> Result<Value> {
    let stream = Stream::parse(original)?;
    ensure!(
        !indices.is_empty() && indices.len() <= stream.len(),
        "Empty/oversized edit selection"
    );
    let mut previous = None;
    let mut entries: Vec<Value> = Vec::new();
    for &index in indices {
        ensure!(
            previous.is_none_or(|old| old < index),
            "Selection must retain original edit order"
        );
        previous = Some(index);
        let Entry::Text(text) = stream.entry(index)? else {
            anyhow::bail!("Selected resource/notebook edits require their native apply backend");
        };
        ensure!(
            text.edit
                .keys()
                .all(|key| matches!(key.as_str(), "range" | "text" | "insertAsSnippet")),
            "Selected EOL/compound text edits require their native apply backend"
        );
        ensure!(
            !text
                .edit
                .get("insertAsSnippet")
                .is_some_and(crate::workspace_edit_stream::truthy),
            "Selected snippet edits require their native apply backend"
        );
        let range = text
            .edit
            .get("range")
            .and_then(Value::as_object)
            .context("Text edit range is required")?;
        ensure!(range.len() == 4, "Unsupported text edit range fields");
        let coordinate = |name: &str| -> Result<u64> {
            let value = range
                .get(name)
                .and_then(Value::as_u64)
                .context("Invalid text edit coordinate")?;
            ensure!(
                value > 0 && value <= i32::MAX as u64,
                "Text edit coordinate exceeds source bounds"
            );
            Ok(value - 1)
        };
        let edit = json!({"range":{"start":{"line":coordinate("startLineNumber")?,"character":coordinate("startColumn")?},
            "end":{"line":coordinate("endLineNumber")?,"character":coordinate("endColumn")?}},
            "newText":text.edit.get("text").and_then(Value::as_str).context("Text replacement is required")?});
        let version = text
            .requested_version
            .map(|version| {
                ensure!(
                    version > 0 && version < (1i64 << 53),
                    "Invalid returned text version"
                );
                Ok::<_, anyhow::Error>(version as u64)
            })
            .transpose()?;
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry["uri"].as_str() == Some(text.resource))
        {
            ensure!(
                entry.get("version").and_then(Value::as_u64) == version,
                "Conflicting returned versions for one resource"
            );
            entry["edits"]
                .as_array_mut()
                .expect("owned edit array")
                .push(edit);
        } else {
            let mut entry = json!({"uri":text.resource,"edits":[edit]});
            if let Some(version) = version {
                entry["version"] = json!(version);
            }
            entries.push(entry);
        }
    }
    Ok(json!({"entries":entries,"metadata":{"isRefactoring":false}}))
}

impl App {
    pub(super) fn escape_file_participation(&mut self) -> bool {
        let Some(run) = self
            .file_participation
            .run
            .as_ref()
            .filter(|run| matches!(run.phase, Phase::Callback))
        else {
            return false;
        };
        let ticket = run
            .ticket
            .as_ref()
            .expect("dispatched participant ticket")
            .clone();
        if let Some(host) = &mut self.extension_host {
            host.cancel_file_participation(&ticket);
            let _ = host.take_file_participation_reply(
                &ticket,
                &self.documents,
                &self.hidden_documents,
            );
        }
        self.skip_additional_file_edits(
            "Additional file edits canceled; original operation continues".into(),
        );
        true
    }
    pub(super) fn file_participation_pending(&self) -> bool {
        self.file_participation.run.is_some()
            || self.file_participation.worker.busy()
            || self.file_participation.retired_transaction.is_some()
    }
    pub(super) fn take_file_participation_notice(&mut self) -> Option<String> {
        self.file_participation.notice.take()
    }
    pub(super) fn begin_native_file_participation(&mut self, action: Action) -> Result<()> {
        self.file_participation.notice = None;
        let id = self
            .file_participation
            .next
            .checked_add(1)
            .context("File participation identities exhausted")?;
        let timeout = self.settings.file_participant_timeout();
        let Some(host) = self
            .extension_host
            .as_mut()
            .filter(|host| host.file_participation_enabled() && timeout.is_some())
        else {
            self.file_job = Some(Job::start(action));
            self.message = "File operation running…".into();
            return Ok(());
        };
        ensure!(
            host.file_participation_available(),
            "File participant callback is still occupied or starting"
        );
        ensure!(
            self.settings_error.is_none(),
            "File participant settings are unavailable"
        );
        let session = host.session;
        let preparation = self
            .file_participation
            .preparation_next
            .checked_add(1)
            .context("File preparation identities exhausted")?;
        self.file_participation.worker.start(
            preparation,
            Work::Preflight(action.clone()),
            Instant::now() + Duration::from_secs(5),
        )?;
        self.file_participation.preparation_next = preparation;
        self.file_participation.next = id;
        self.file_participation.run = Some(Run {
            id,
            preparation,
            action,
            root: self.workspace.root.clone(),
            profile: self.settings_profile_generation(),
            session,
            ticket: None,
            preflight: None,
            settings: self.settings.extension_layers().clone(),
            timeout: timeout.expect("enabled participant policy"),
            lifetime: Arc::new(AtomicBool::new(false)),
            names: Vec::new(),
            phase: Phase::Preflight,
        });
        self.message = "Checking original file operation…".into();
        Ok(())
    }
    fn file_participation_context_current(&self, run: &Run) -> bool {
        self.file_participation_context_with_host(run, self.extension_host.as_ref())
    }
    fn file_participation_context_with_host(
        &self,
        run: &Run,
        host: Option<&crate::extensions::Client>,
    ) -> bool {
        run.root == self.workspace.root
            && run.profile == self.settings_profile_generation()
            && self.settings_error.is_none()
            && Arc::ptr_eq(&run.settings, self.settings.extension_layers())
            && host.is_some_and(|host| host.session == run.session)
    }
    fn file_participation_edits_current(&self, run: &Run) -> bool {
        self.file_participation_edits_with_host(run, self.extension_host.as_ref())
    }
    fn file_participation_edits_with_host(
        &self,
        run: &Run,
        host: Option<&crate::extensions::Client>,
    ) -> bool {
        self.file_participation_context_with_host(run, host)
            && run.ticket.as_ref().is_none_or(|ticket| {
                host.is_some_and(|host| {
                    host.file_participation_current(ticket, &self.documents, &self.hidden_documents)
                })
            })
    }
    /// Called before the shared transaction worker can publish. A replaced
    /// session or changed returned target cannot acquire a new source authority.
    pub(super) fn guard_file_participation(&self, host: Option<&crate::extensions::Client>) {
        if let Some(run) = &self.file_participation.run
            && matches!(run.phase, Phase::Applying(_))
            && !matches!(run.phase, Phase::Applying(token) if self.workspace_transaction_completion().is_some_and(|done| done.token == token))
            && !self.file_participation_edits_with_host(run, host)
        {
            run.lifetime.store(true, Ordering::Release);
        }
    }
    pub(super) fn retire_file_participation(&mut self) {
        let Some(run) = self.file_participation.run.take() else {
            return;
        };
        run.lifetime.store(true, Ordering::Release);
        self.file_participation.worker.retire(run.preparation);
        if let Some(host) = &mut self.extension_host
            && let Some(ticket) = &run.ticket
        {
            host.cancel_file_participation(ticket);
            let _ =
                host.take_file_participation_reply(ticket, &self.documents, &self.hidden_documents);
        }
        if let Phase::Applying(token) = run.phase {
            self.cancel_workspace_transaction(token);
            self.file_participation.retired_transaction = Some(token);
        }
        if matches!(&self.modal, Some(Modal::FileEditReview(dialog)) if dialog.id() == run.id) {
            self.modal = None;
        }
    }
    fn skip_additional_file_edits(&mut self, notice: String) {
        if let Some(run) = &mut self.file_participation.run {
            run.phase = Phase::Ready;
        }
        self.file_participation.notice = Some(notice.clone());
        self.message = notice;
    }
    fn prepare_file_edit_selection(
        &mut self,
        overview: Arc<Overview>,
        indices: Vec<usize>,
    ) -> Result<()> {
        let run = self
            .file_participation
            .run
            .as_ref()
            .context("Original file operation retired")?;
        ensure!(
            self.file_participation_edits_current(run),
            "Returned edit source changed before apply"
        );
        self.start_file_review_preparation(PrepareReview {
            original: overview.original().clone(),
            store: None,
            indices: Some(indices),
            preflight: run
                .preflight
                .as_ref()
                .context("Original file preflight missing")?
                .clone(),
            sources: Vec::new(),
        })?;
        self.file_participation.run.as_mut().unwrap().phase = Phase::PreparingApply;
        self.message = "Preparing additional file edits…".into();
        Ok(())
    }
    fn start_file_review_preparation(&mut self, work: PrepareReview) -> Result<()> {
        let id = self
            .file_participation
            .preparation_next
            .checked_add(1)
            .context("File review worker identities exhausted")?;
        ensure!(
            self.file_participation.run.is_some(),
            "Original file operation retired"
        );
        self.file_participation.worker.start(
            id,
            Work::Review(work),
            Instant::now() + Duration::from_secs(6),
        )?;
        self.file_participation.preparation_next = id;
        self.file_participation.run.as_mut().unwrap().preparation = id;
        Ok(())
    }
    pub(super) fn file_edit_review_key(&mut self, key: KeyEvent, mut dialog: Dialog) {
        let current = self.file_participation.run.as_ref().is_some_and(|run| {
            run.id == dialog.id()
                && matches!(run.phase, Phase::Review(_))
                && self.file_participation_edits_current(run)
        });
        if !current {
            self.retire_file_participation();
            self.message =
                "File edit review expired; original operation canceled and work retained".into();
            return;
        }
        match dialog.key(key) {
            Some(ReviewEvent::Skip) => {
                self.skip_additional_file_edits("Additional edits skipped".into())
            }
            Some(ReviewEvent::Remember(choice)) => {
                if let Err(error) = self.write_file_participant_choice(Some(choice)) {
                    self.message = format!("Could not remember preview choice: {error:#}");
                }
                self.modal = Some(Modal::FileEditReview(dialog));
            }
            Some(ReviewEvent::Apply { indices, remember }) => {
                if let Some(choice) = remember
                    && let Err(error) = self.write_file_participant_choice(Some(choice))
                {
                    self.file_participation.notice =
                        Some(format!("Could not remember choice: {error:#}"));
                }
                if let Err(error) =
                    self.prepare_file_edit_selection(dialog.overview().clone(), indices)
                {
                    self.skip_additional_file_edits(format!("Additional edits refused: {error:#}"));
                }
            }
            None => self.modal = Some(Modal::FileEditReview(dialog)),
        }
    }
    fn file_edit_review_ready(
        &mut self,
        overview: Arc<Overview>,
        remembered: Option<bool>,
        preview: Arc<crate::file_edit_review::preview::Preview>,
    ) -> Result<()> {
        let remembered = self.cached_file_participant_choice().unwrap_or(remembered);
        let initial = overview.initial(remembered);
        if initial == Initial::NoChanges {
            self.file_participation.run.as_mut().unwrap().phase = Phase::Ready;
        } else if initial
            == (Initial::Apply {
                show_preview: false,
            })
            && !overview.needs_confirmation()
        {
            let indices = overview.rows().iter().map(|row| row.index).collect();
            self.prepare_file_edit_selection(overview, indices)?;
        } else {
            // Bulk-edit annotations require preview independently of the outer
            // remembered file-participant dialog policy.
            let initial = if initial
                == (Initial::Apply {
                    show_preview: false,
                }) {
                Initial::Apply { show_preview: true }
            } else {
                initial
            };
            let run = self.file_participation.run.as_mut().unwrap();
            let dialog = Dialog::new(run.id, overview.clone(), run.names.clone(), initial)?
                .with_preview(preview);
            run.phase = Phase::Review(overview);
            self.modal = Some(Modal::FileEditReview(dialog));
        }
        Ok(())
    }
    pub(super) fn poll_native_file_participation(&mut self) -> bool {
        self.guard_file_participation(self.extension_host.as_ref());
        let mut changed = false;
        if let Some(token) = self.file_participation.retired_transaction
            && self.take_workspace_transaction_completion(token).is_some()
        {
            self.file_participation.retired_transaction = None;
            changed = true;
        }
        if let Some(event) = self.file_participation.worker.poll() {
            changed = true;
            match event {
                WorkerEvent::Prepared { id, .. } => {
                    let current = self.file_participation.run.as_ref().is_some_and(|run| {
                        run.preparation == id && self.file_participation_edits_current(run)
                    });
                    if !current || self.file_participation.worker.authorize(id).is_err() {
                        self.file_participation.worker.retire(id);
                    }
                }
                WorkerEvent::Finished { id, result, .. } => {
                    if self
                        .file_participation
                        .run
                        .as_ref()
                        .is_some_and(|run| run.preparation == id)
                    {
                        let result = (|| -> Result<()> {
                            let run = self.file_participation.run.as_ref().unwrap();
                            ensure!(
                                self.file_participation_edits_current(run),
                                "Returned source changed during preparation"
                            );
                            match result {
                                Ok(Outcome::Committed(PreparedReview::Preflight(preflight))) => {
                                    let host = self
                                        .extension_host
                                        .as_mut()
                                        .context("Original extension host retired")?;
                                    host.sync_with_hidden(
                                        &self.documents,
                                        &self.hidden_documents,
                                        self.active,
                                    )?;
                                    let run = self.file_participation.run.as_mut().unwrap();
                                    run.action = preflight.action.clone();
                                    let ticket = host.start_file_participation(
                                        &run.action,
                                        &self.documents,
                                        &self.hidden_documents,
                                        run.timeout,
                                    )?;
                                    run.ticket = Some(ticket);
                                    run.preflight = Some(preflight);
                                    run.phase = Phase::Callback;
                                    self.message =
                                        "Running file participants… Esc skips additional edits"
                                            .into();
                                    Ok(())
                                }
                                Ok(Outcome::Committed(PreparedReview::Review {
                                    overview,
                                    remembered,
                                    preview,
                                })) => self.file_edit_review_ready(overview, remembered, preview),
                                Ok(Outcome::Committed(PreparedReview::Apply(args))) => {
                                    let host = self
                                        .extension_host
                                        .as_mut()
                                        .context("Extension host retired")?;
                                    host.sync_with_hidden(
                                        &self.documents,
                                        &self.hidden_documents,
                                        self.active,
                                    )?;
                                    let inventory = host.mirrored_snapshotset(
                                        &self.documents,
                                        &self.hidden_documents,
                                    )?;
                                    let run = self.file_participation.run.as_ref().unwrap();
                                    ensure!(
                                        self.file_participation_edits_current(run),
                                        "Returned source changed before transaction admission"
                                    );
                                    let origin = super::workspace_transactions::Origin::native(
                                        "native-file-participant",
                                        run.lifetime.clone(),
                                    )?;
                                    let capture = self.capture_workspace_transaction(
                                        origin,
                                        &args,
                                        &inventory,
                                        Instant::now(),
                                    )?;
                                    let token = self.start_workspace_transaction(capture)?;
                                    self.file_participation.run.as_mut().unwrap().phase =
                                        Phase::Applying(token);
                                    Ok(())
                                }
                                Ok(_) => {
                                    anyhow::bail!("Additional edit preparation canceled or expired")
                                }
                                Err(error) => anyhow::bail!(error),
                            }
                        })();
                        if let Err(error) = result {
                            if matches!(
                                self.file_participation.run.as_ref().map(|run| &run.phase),
                                Some(Phase::Preflight)
                            ) {
                                self.retire_file_participation();
                                self.message = format!(
                                    "File operation refused before callbacks or disk changes: {error:#}"
                                );
                            } else {
                                self.skip_additional_file_edits(format!(
                                    "Additional edits refused: {error:#}"
                                ));
                            }
                        }
                    }
                }
            }
        }
        let Some(run) = self.file_participation.run.as_ref() else {
            return changed;
        };
        if !self.file_participation_context_current(run) {
            self.retire_file_participation();
            self.message = "File operation context changed; operation canceled".into();
            return true;
        }
        match run.phase {
            Phase::Callback => {
                let ticket = run
                    .ticket
                    .as_ref()
                    .expect("dispatched participant ticket")
                    .clone();
                let reply = self
                    .extension_host
                    .as_mut()
                    .unwrap()
                    .take_file_participation_reply(
                        &ticket,
                        &self.documents,
                        &self.hidden_documents,
                    );
                if let Some(reply) = reply {
                    changed = true;
                    let result = (|| -> Result<()> {
                        match reply.map_err(anyhow::Error::msg)? {
                            None => {
                                self.file_participation.run.as_mut().unwrap().phase = Phase::Ready
                            }
                            Some(value) => {
                                let original = Arc::new(
                                    value
                                        .get("edit")
                                        .context("Missing participant edit")?
                                        .clone(),
                                );
                                let names = value
                                    .get("extensionNames")
                                    .and_then(Value::as_array)
                                    .context("Missing contributor names")?
                                    .iter()
                                    .map(|name| {
                                        name.as_str()
                                            .map(str::to_owned)
                                            .context("Invalid contributor name")
                                    })
                                    .collect::<Result<Vec<_>>>()?;
                                let store = self.participant_preference_store()?;
                                let snapshots =
                                    self.extension_host.as_ref().unwrap().mirrored_snapshotset(
                                        &self.documents,
                                        &self.hidden_documents,
                                    )?;
                                let sources = snapshots
                                    .iter()
                                    .map(|snapshot| {
                                        let document = self
                                            .documents
                                            .iter()
                                            .chain(&self.hidden_documents)
                                            .find(|document| document.id == snapshot.document)
                                            .context("Original preview source closed")?;
                                        Ok((snapshot.uri.clone(), document.text.clone()))
                                    })
                                    .collect::<Result<Vec<_>>>()?;
                                self.file_participation.run.as_mut().unwrap().names = names;
                                self.start_file_review_preparation(PrepareReview {
                                    original,
                                    store: Some(store),
                                    indices: None,
                                    preflight: self
                                        .file_participation
                                        .run
                                        .as_ref()
                                        .unwrap()
                                        .preflight
                                        .as_ref()
                                        .context("Original file preflight missing")?
                                        .clone(),
                                    sources,
                                })?;
                                self.file_participation.run.as_mut().unwrap().phase =
                                    Phase::PreparingReview;
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.skip_additional_file_edits(format!(
                            "Extension file edits unavailable: {error:#}"
                        ));
                    }
                }
            }
            Phase::Applying(token) => {
                if let Some(done) = self.take_workspace_transaction_completion(token) {
                    if done.applied {
                        self.file_participation.run.as_mut().unwrap().phase = Phase::Ready;
                    } else {
                        self.skip_additional_file_edits(format!(
                            "Additional edits refused: {}",
                            done.reason.unwrap_or_default()
                        ));
                    }
                    changed = true;
                }
            }
            Phase::Review(ref overview) => {
                let id = run.id;
                let _ = overview; // Retained whole payload outlives its modal.
                if !self.file_participation_edits_current(run) {
                    self.retire_file_participation();
                    self.message = "Reviewed edit source changed; file operation canceled".into();
                    changed = true;
                } else if !matches!(&self.modal, Some(Modal::FileEditReview(dialog)) if dialog.id() == id)
                {
                    self.skip_additional_file_edits("Additional edit review canceled".into());
                    changed = true;
                }
            }
            Phase::Ready if !self.saves_pending() => {
                let run = self.file_participation.run.take().unwrap();
                // Recheck native path-clock capacity at actual effect admission.
                if matches!(run.action, Action::Rename { .. } | Action::Trash(_))
                    && let Some(error) = self
                        .documents
                        .iter()
                        .chain(&self.hidden_documents)
                        .find_map(|doc| doc.check_resource_change().err())
                {
                    self.message = format!("File operation refused before disk changes: {error:#}");
                } else {
                    self.file_job = Some(Job::start_preflighted(
                        run.preflight.expect("original file preflight"),
                    ));
                    self.message = "File operation running…".into();
                }
                changed = true;
            }
            _ => {}
        }
        changed
    }
}
