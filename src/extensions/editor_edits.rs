//! One actual EOL preparer with bounded intent and retained native receipts.
use super::*;
use crate::document::{EditorViewProof, WorkspaceSource, workspace::EditorEolPrepared};
use anyhow::ensure;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

const LIFETIME: Duration = Duration::from_secs(5);
const QUEUE_BYTES: usize = 8 * 1024 * 1024;
struct Job {
    id: Value,
    edit: Edit,
    source: WorkspaceSource,
    proof: EditorViewProof,
    language_epoch: u64,
    owner_retired: Arc<AtomicBool>,
    deadline: Instant,
    bytes: usize,
    available: usize,
}
struct Actual {
    job: Job,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<std::result::Result<EditorEolPrepared, String>>,
    thread: JoinHandle<()>,
}
#[cfg(test)]
struct Gate {
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
}
#[derive(Default)]
pub(super) struct State {
    queued: VecDeque<Job>,
    actual: Option<Actual>,
    #[cfg(test)]
    gate: Option<Gate>,
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.actual.is_some() || !self.queued.is_empty()
    }
    fn enqueue(&mut self, job: Job) -> Result<()> {
        ensure!(
            self.queued.len() + usize::from(self.actual.is_some()) < 8,
            "Editor EOL queue is full"
        );
        ensure!(
            !self.queued.iter().any(|old| old.id == job.id)
                && self.actual.as_ref().is_none_or(|old| old.job.id != job.id),
            "Duplicate editor EOL request"
        );
        let retained = self.queued.iter().map(|job| job.bytes).sum::<usize>()
            + self.actual.as_ref().map_or(0, |actual| actual.job.bytes);
        ensure!(
            retained.saturating_add(job.bytes) <= QUEUE_BYTES,
            "Editor EOL queued payload exceeds 8 MiB"
        );
        self.queued.push_back(job);
        Ok(())
    }
    fn start(&mut self, mut job: Job) -> Result<()> {
        ensure!(
            self.actual.is_none(),
            "Editor EOL preparation is actually occupied"
        );
        let source = job.source.clone();
        let edits = std::mem::take(&mut job.edit.edits)
            .into_iter()
            .map(|edit| (edit.edit, edit.force_move_markers))
            .collect();
        let eol = if job.edit.set_end_of_line == Some(2)
            || (job.edit.set_end_of_line.is_none() && source.eol() == "\r\n")
        {
            "\r\n"
        } else {
            "\n"
        };
        let deadline = job.deadline;
        let available = job.available;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        #[cfg(test)]
        let gate = self.gate.take();
        let thread = thread::Builder::new()
            .name("vscli-editor-eol".into())
            .spawn(move || {
                let result =
                    EditorEolPrepared::prepare(source, edits, eol, available, &flag, deadline)
                        .map_err(|error| format!("{error:#}").chars().take(4096).collect());
                let _ = sender.send(result);
                #[cfg(test)]
                if let Some(gate) = gate {
                    let _ = gate.entered.try_send(());
                    let _ = gate.release.recv_timeout(Duration::from_secs(6));
                }
            })
            .context("Cannot start editor EOL preparation")?;
        self.actual = Some(Actual {
            job,
            cancel,
            receiver,
            thread,
        });
        Ok(())
    }
    fn ready(&mut self) -> Option<(Job, Result<EditorEolPrepared>)> {
        if !self.actual.as_ref()?.thread.is_finished() {
            return None;
        }
        let actual = self.actual.take()?;
        let result = if actual.thread.join().is_err() {
            Err(anyhow::anyhow!("Editor EOL worker panicked"))
        } else {
            actual
                .receiver
                .try_recv()
                .map_err(|_| anyhow::anyhow!("Editor EOL worker exited without a result"))
                .and_then(|result| result.map_err(anyhow::Error::msg))
        };
        Some((actual.job, result))
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(actual) = &self.actual {
            actual.cancel.store(true, Ordering::Release);
        }
    }
}

impl Client {
    pub(super) fn queue_editor_eol(
        &mut self,
        id: Value,
        params: Value,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<()> {
        ensure!(
            id.as_str()
                .is_some_and(|id| !id.is_empty() && id.len() <= 128),
            "Invalid editor EOL request ID"
        );
        ensure!(
            params["edits"]
                .as_array()
                .is_some_and(|edits| edits.len() <= 4096),
            "Invalid editor EOL edit count"
        );
        let edit: Edit = serde_json::from_value(params)?;
        ensure!(
            matches!(edit.set_end_of_line, None | Some(1 | 2)),
            "Invalid editor endOfLine"
        );
        self.validate_selected_owner(edit.session, &edit.owner)?;
        let owner_retired = self.owner_lifetimes.capture(&edit.owner)?;
        let refs = documents.iter().chain(hidden).collect::<Vec<_>>();
        check_mirror_budget(&refs)?;
        let doc = refs
            .iter()
            .copied()
            .find(|doc| doc.id == edit.document)
            .context("Editor EOL document closed")?;
        if doc.is_read_only()
            || !self.service_document_current(doc, edit.version)
            || self.owner_retired(&edit.owner)
        {
            return self
                .process
                .send(json!({"id":id,"result":{"applied":false}}));
        }
        let proof = match &edit.editor {
            Some(editor) => {
                ensure!(editor.len() <= 128, "Invalid editor EOL handle");
                self.editor_view_source(editor, doc)?
            }
            None => {
                ensure!(
                    !self.editor_inventory_explicit(),
                    "Editor EOL requires original visible editor"
                );
                doc.editor_view_identity()
            }
        };
        let text_bytes = edit.edits.iter().try_fold(0usize, |bytes, edit| {
            bytes
                .checked_add(edit.edit.new_text.len())
                .context("Editor EOL payload overflow")
        })?;
        ensure!(
            text_bytes <= MAX_DOCUMENT_BYTES,
            "Editor EOL text exceeds 4 MiB"
        );
        let bytes = text_bytes + edit.edits.len() * 128;
        let available = MAX_DOCUMENT_BYTES
            - (refs.iter().map(|doc| doc.text.len_bytes()).sum::<usize>() - doc.text.len_bytes());
        self.editor_edits.enqueue(Job {
            id,
            edit,
            source: doc.capture_workspace_source()?,
            proof,
            language_epoch: doc.language_epoch(),
            owner_retired,
            deadline: Instant::now() + LIFETIME,
            bytes,
            available,
        })
    }
    fn editor_eol_job_current(
        &self,
        job: &Job,
        documents: &[Document],
        hidden: &[Document],
    ) -> bool {
        job.deadline > Instant::now()
            && !job.owner_retired.load(Ordering::Acquire)
            && self
                .validate_selected_owner(job.edit.session, &job.edit.owner)
                .is_ok()
            && !self.owner_retired(&job.edit.owner)
            && documents
                .iter()
                .chain(hidden)
                .find(|doc| doc.id == job.edit.document)
                .is_some_and(|doc| {
                    job.source.current(doc)
                        && doc.editor_view_current(&job.proof)
                        && doc.language_epoch() == job.language_epoch
                        && self.service_document_current(doc, job.edit.version)
                        && job
                            .edit
                            .editor
                            .as_ref()
                            .is_none_or(|editor| self.editor_view_live(editor, doc))
                })
    }
    pub(super) fn poll_editor_eol(
        &mut self,
        documents: &mut [Document],
        hidden: &mut [Document],
        active: usize,
    ) -> Result<()> {
        if let Some((job, result)) = self.editor_edits.ready() {
            if !self.editor_eol_job_current(&job, documents, hidden) {
                self.process
                    .send(json!({"id":job.id,"result":{"applied":false}}))?;
            } else {
                let result = result.and_then(|prepared| {
                    self.publish_editor_eol(&job, prepared, documents, hidden, active)
                });
                if let Err(error) = result {
                    self.process
                        .send(json!({"id":job.id,"error":{"message":error.to_string()}}))?;
                }
            }
        }
        if self.editor_edits.actual.is_none() {
            for _ in 0..8 {
                let Some(job) = self.editor_edits.queued.pop_front() else {
                    break;
                };
                if !self.editor_eol_job_current(&job, documents, hidden) {
                    self.process
                        .send(json!({"id":job.id,"result":{"applied":false}}))?;
                    continue;
                }
                let id = job.id.clone();
                if let Err(error) = self.editor_edits.start(job) {
                    self.process
                        .send(json!({"id":id,"error":{"message":error.to_string()}}))?;
                }
                break;
            }
        }
        Ok(())
    }
    fn publish_editor_eol(
        &mut self,
        job: &Job,
        prepared: EditorEolPrepared,
        documents: &mut [Document],
        hidden: &mut [Document],
        active: usize,
    ) -> Result<()> {
        let doc = documents
            .iter_mut()
            .chain(hidden.iter_mut())
            .find(|doc| doc.id == job.edit.document)
            .context("Editor EOL model retired")?;
        let commit = doc.prepare_editor_eol_commit(
            prepared,
            &job.proof,
            job.edit.undo_stop_before,
            job.edit.undo_stop_after,
        )?;
        let mut mirror = self.mirror.clone();
        let mut messages = Vec::new();
        for replacement in commit
            .intermediate()
            .into_iter()
            .chain(std::iter::once(commit.final_document()))
        {
            let refs = documents
                .iter()
                .chain(hidden.iter())
                .map(|doc| {
                    if doc.id == job.edit.document {
                        replacement
                    } else {
                        doc
                    }
                })
                .collect::<Vec<_>>();
            if MirrorState::stamp_refs(&refs, refs.get(active).copied()) == mirror.last_stamp {
                continue;
            }
            let (next, state) =
                self.editor_eol_mirror(&mirror, &refs, documents.get(active).map(|doc| doc.id))?;
            messages.push(json!({"method":"state","params":state}));
            mirror = next;
        }
        messages.push(json!({"id":job.id,"result":{"applied":true}}));
        // One bounded queue admission covers both states and the ACK. Every
        // model/history preparation above was private; refusal has no effects.
        self.process.send_batch(messages)?;
        let doc = documents
            .iter_mut()
            .chain(hidden.iter_mut())
            .find(|doc| doc.id == job.edit.document)
            .expect("retained editor EOL model");
        commit.publish(doc);
        self.mirror = mirror;
        Ok(())
    }
    fn editor_eol_mirror(
        &self,
        mirror: &MirrorState,
        refs: &[&Document],
        active: Option<u64>,
    ) -> Result<(MirrorState, Value)> {
        check_mirror_budget(refs)?;
        let editor_stamp = self
            .editor_inventory
            .as_ref()
            .map(|inventory| inventory.stamp(refs))
            .transpose()?;
        let tabs_stamp = self
            .editor_tabs
            .as_ref()
            .map(editor_tabs::Inventory::wire)
            .transpose()?;
        let active = refs.iter().copied().find(|doc| Some(doc.id) == active);
        let (mut next, mut state) = mirror.next_refs_with_inventory_stamps(
            refs,
            active,
            editor_stamp.as_ref(),
            tabs_stamp.as_ref(),
        )?;
        if let Some(inventory) = &self.editor_inventory {
            state["editors"] = inventory.wire(refs, &next.api_editor_selections)?;
            state["activeEditor"] = json!(inventory.active);
            next.editors = inventory.frames.clone();
            next.api_editor_selections.clear();
        }
        if let Some(stamp) = tabs_stamp {
            state["tabs"] = stamp;
            next.tabs = self.editor_tabs.clone();
        }
        Ok((next, state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_eol_preparer_retains_capacity_after_reply_and_late_edit_undo_preserves_redo() {
        let directory = tempfile::tempdir().unwrap();
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut docs = vec![Document::from_text("λ🙂\n")];
        let mut client = Client::start(
            "node",
            &fixture,
            directory.path(),
            &docs,
            0,
            &Settings::default(),
        )
        .unwrap();
        let (entered, observed) = mpsc::sync_channel(1);
        let (release, resume) = mpsc::sync_channel(1);
        client.editor_edits.gate = Some(Gate {
            entered,
            release: resume,
        });
        let params = json!({"session":client.session,"owner":client.packages[0].id,"document":docs[0].id,"version":1,"edits":[],"setEndOfLine":2});
        client
            .queue_editor_eol(json!("eol-original"), params.clone(), &docs, &[])
            .unwrap();
        client.poll_editor_eol(&mut docs, &mut [], 0).unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            client.editor_edits.ready().is_none(),
            "reply alone must not release actual worker capacity"
        );
        assert!(
            client
                .queue_editor_eol(json!("eol-original"), params.clone(), &docs, &[])
                .is_err()
        );
        for index in 0..7 {
            client
                .queue_editor_eol(
                    json!(format!("eol-queued-{index}")),
                    params.clone(),
                    &docs,
                    &[],
                )
                .unwrap();
        }
        assert!(
            client
                .queue_editor_eol(json!("eol-ninth"), params, &docs, &[])
                .is_err()
        );
        docs[0].insert("retained", false);
        docs[0].undo();
        client.poll_editor_eol(&mut docs, &mut [], 0).unwrap();
        assert!(client.editor_edits.actual.is_some());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.editor_edits.actual.is_some() || !client.editor_edits.queued.is_empty() {
            client.poll_editor_eol(&mut docs, &mut [], 0).unwrap();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(docs[0].text.to_string(), "λ🙂\n");
        assert_eq!(docs[0].eol, "\n");
        docs[0].redo();
        assert_eq!(docs[0].text.to_string(), "retainedλ🙂\n");
        assert_eq!(docs[0].eol, "\n");
    }
}
