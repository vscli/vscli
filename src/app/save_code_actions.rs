//! Origin-owned edit-only source actions. Actual capacity belongs to the LSP client.
use super::*;
use crate::{
    lsp::Request,
    save_action_edits::{self, Origin},
    settings::{SaveActionPlan, SaveActionPolicy, SaveActionReason},
};
use anyhow::{Context, ensure};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_millis(1500);
const REPLACEMENTS: usize = 4 * 1024 * 1024;
#[cfg(test)]
mod tests;

struct Proof {
    intent: u64,
    document: u64,
    path: PathBuf,
    uri: String,
    revision: u64,
    epoch: u64,
    saved_revision: u64,
    save_generation: u64,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    profile: u64,
    workspace: PathBuf,
    server: Arc<()>,
}
enum Phase {
    Discovery,
    Resolve,
}
struct Pending {
    token: u64,
    phase: Phase,
}
struct Run {
    proof: Proof,
    plan: SaveActionPlan,
    family: usize,
    started: Instant,
    pending: Option<Pending>,
    origin: Option<Request>,
    candidates: VecDeque<Value>,
    resolves: usize,
    raw_remaining: usize,
    expanded_remaining: usize,
    source_failed: bool,
}
#[derive(Default)]
pub(super) struct State {
    active: Option<Run>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.active.is_some()
    }
    pub(super) fn document_pending(&self, id: u64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|run| run.proof.document == id)
    }
}
impl App {
    fn action_save_notice(&mut self, intent: u64, notice: impl Into<String>) {
        if let Some(latest) = &mut self.saving.latest
            && latest.id == intent
            && latest.stage == saving::Stage::Actions
        {
            let notice = notice.into().chars().take(500).collect::<String>();
            latest.add_notice(format!("Code actions skipped: {notice}"));
        }
    }
    fn finish_save_actions(&mut self, intent: u64) {
        if let Some(latest) = &mut self.saving.latest
            && latest.id == intent
            && latest.stage == saving::Stage::Actions
        {
            latest.stage = saving::Stage::Formatting;
        }
    }
    fn action_save_current(&self, run: &Run) -> bool {
        let proof = &run.proof;
        self.settings_error.is_none()
            && self.settings_profile_generation() == proof.profile
            && Arc::ptr_eq(self.settings.extension_layers(), &proof.settings)
            && self.workspace.root == proof.workspace
            && self.saving.latest.as_ref().is_some_and(|intent| {
                intent.id == proof.intent
                    && intent.document == proof.document
                    && intent.stage == saving::Stage::Actions
                    && intent.destination.is_none()
                    && intent.automatic.is_none()
            })
            && self.documents.iter().any(|doc| {
                doc.id == proof.document
                    && doc.path.as_ref() == Some(&proof.path)
                    && doc.revision == proof.revision
                    && doc.text_epoch() == proof.epoch
                    && doc.saved_revision == proof.saved_revision
                    && doc.save_generation() == proof.save_generation
            })
            && self.lsp.as_ref().is_some_and(|client| {
                client.ready && Arc::ptr_eq(&client.identity(), &proof.server)
            })
    }
    fn cancel_action_run(&mut self, run: &Run) {
        if let Some(pending) = &run.pending
            && let Some(client) = &mut self.lsp
            && Arc::ptr_eq(&client.identity(), &run.proof.server)
            && let Err(error) = client.cancel_action_request(pending.token)
        {
            self.message = format!("Save action cancellation failed: {error:#}");
        }
    }
    pub(super) fn cancel_save_code_actions(&mut self) {
        if let Some(run) = self.saving.actions.active.take() {
            self.cancel_action_run(&run);
        }
    }
    pub(super) fn retire_save_code_actions(&mut self, reason: &str) -> bool {
        let Some(run) = self.saving.actions.active.take() else {
            return false;
        };
        self.cancel_action_run(&run);
        if self
            .saving
            .latest
            .as_ref()
            .is_some_and(|latest| latest.id == run.proof.intent)
        {
            self.saving.latest = None;
            self.message = format!("Save retired; unsaved work retained: {reason}");
        }
        true
    }
    pub(super) fn escape_save_code_actions(&mut self) -> bool {
        if !self.retire_save_code_actions("code actions canceled") {
            // Before ordinary polling admits source actions, Escape may cancel
            // only an opted-in action save. A queued plain save behind actual
            // filesystem work belongs to the save/close controller instead.
            if self.native_save_worker_busy() || self.settings_error.is_some() {
                return false;
            }
            let Some(intent) = self.saving.latest.as_ref() else {
                return false;
            };
            if intent.stage != saving::Stage::Actions
                || intent.automatic.is_some()
                || intent.destination.is_some()
            {
                return false;
            }
            let Some(path) = self
                .documents
                .iter()
                .find(|doc| doc.id == intent.document)
                .and_then(|doc| doc.path.as_deref())
            else {
                return false;
            };
            if !matches!(
                self.settings.save_code_actions(
                    crate::languages::language(path),
                    SaveActionReason::Explicit
                ),
                SaveActionPolicy::Native(plan) if !plan.families.is_empty()
            ) {
                return false;
            }
            let id = intent.id;
            if !self
                .saving
                .latest
                .as_ref()
                .is_some_and(|latest| latest.id == id && latest.stage == saving::Stage::Actions)
            {
                return false;
            }
            self.saving.latest = None;
            self.message = "Save retired; unsaved work retained: code actions canceled".into();
        }
        self.cancel_save_continuations();
        true
    }
    pub(super) fn begin_save_code_actions(&mut self, intent: &saving::Intent) -> Result<bool> {
        if intent.stage != saving::Stage::Actions {
            return Ok(false);
        }
        let Some(doc) = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == intent.document)
        else {
            if self.saving.latest.as_ref().is_some_and(|latest| {
                latest.id == intent.id && latest.stage == saving::Stage::Actions
            }) {
                self.saving.latest = None;
                self.message = "Save retired; unsaved work retained: save model was closed".into();
            }
            return Ok(false);
        };
        let language = doc
            .path
            .as_deref()
            .map_or("plaintext", crate::languages::language);
        let reason = if intent.automatic.is_some() {
            SaveActionReason::AfterDelay
        } else {
            SaveActionReason::Explicit
        };
        let plan = match self.settings.save_code_actions(language, reason) {
            SaveActionPolicy::Off => {
                self.finish_save_actions(intent.id);
                return Ok(false);
            }
            SaveActionPolicy::Unavailable(notice) => {
                self.action_save_notice(intent.id, notice);
                self.finish_save_actions(intent.id);
                return Ok(false);
            }
            SaveActionPolicy::Native(plan) => plan,
        };
        let skip = if self.settings_error.is_some() {
            Some("settings failed to load")
        } else if intent.automatic.is_some()
            || intent.destination.is_some()
            || doc.path.is_none()
            || !self.documents.iter().any(|visible| visible.id == doc.id)
        {
            Some("first save, Save As, after-delay and hidden targets are not supported")
        } else {
            match &self.lsp {
                None => Some("no native language server"),
                Some(client) if !client.ready => Some("native language server is not ready"),
                Some(client)
                    if !matches!(
                        client.capabilities.get("codeActionProvider"),
                        Some(Value::Object(_)) | Some(Value::Bool(true))
                    ) =>
                {
                    Some("native server has no code-action provider")
                }
                Some(client) if !client.action_available() => {
                    Some("native action work remains occupied until its actual reply")
                }
                _ => None,
            }
        };
        if let Some(notice) = skip {
            self.action_save_notice(intent.id, notice);
            self.finish_save_actions(intent.id);
            return Ok(false);
        }
        ensure!(
            doc.secondary.len() < 16_384,
            "Save origin exceeds 16,384 selections"
        );
        let path = doc.path.clone().expect("named origin checked");
        let uri = crate::lsp::file_uri(&path)?;
        ensure!(uri.len() <= 8192, "Save action URI exceeds 8 KiB");
        let proof = Proof {
            intent: intent.id,
            document: doc.id,
            path,
            uri,
            revision: doc.revision,
            epoch: doc.text_epoch(),
            saved_revision: doc.saved_revision,
            save_generation: doc.save_generation(),
            settings: self.settings.extension_layers().clone(),
            profile: self.settings_profile_generation(),
            workspace: self.workspace.root.clone(),
            server: self.lsp.as_ref().expect("server checked").identity(),
        };
        let mut run = Run {
            proof,
            plan,
            family: 0,
            started: Instant::now(),
            pending: None,
            origin: None,
            candidates: VecDeque::new(),
            resolves: 0,
            raw_remaining: REPLACEMENTS,
            expanded_remaining: REPLACEMENTS,
            source_failed: false,
        };
        for notice in &run.plan.notices {
            self.action_save_notice(intent.id, notice.clone());
        }
        if let Err(error) = self.advance_save_actions(&mut run) {
            self.action_save_notice(intent.id, format!("{error:#}"));
            self.finish_save_actions(intent.id);
            return Ok(false);
        }
        if run.family >= run.plan.families.len() && run.pending.is_none() {
            self.finish_save_actions(intent.id);
            return Ok(false);
        }
        self.saving.actions.active = Some(run);
        self.message = "Applying source actions before save… Escape cancels".into();
        Ok(true)
    }
    fn next_save_action_family(run: &mut Run) {
        run.family += 1;
        run.origin = None;
        run.candidates.clear();
        run.resolves = 0;
    }
    fn stage_save_action(
        &mut self,
        run: &mut Run,
        request: &Request,
        item: &Value,
    ) -> Result<bool> {
        ensure!(
            self.action_save_current(run),
            "Save action ownership changed"
        );
        ensure!(
            self.lsp
                .as_ref()
                .is_some_and(|client| client.request_current(request)),
            "Native action version changed"
        );
        let doc = self
            .documents
            .iter()
            .find(|doc| doc.id == run.proof.document)
            .context("Save model was closed")?;
        let staged = save_action_edits::stage(
            doc,
            &item["edit"],
            Origin {
                uri: &run.proof.uri,
                version: request.sync_version,
            },
            run.raw_remaining.min(run.expanded_remaining),
        )?;
        ensure!(
            self.action_save_current(run),
            "Save action ownership changed before mutation"
        );
        run.raw_remaining -= staged.raw_bytes;
        run.expanded_remaining -= staged.expanded_bytes;
        if staged.changes.is_empty() {
            return Ok(false);
        }
        let doc = self
            .documents
            .iter_mut()
            .find(|doc| doc.id == run.proof.document)
            .context("Save model was closed")?;
        doc.apply_changes(staged.changes);
        run.proof.revision = doc.revision;
        run.proof.epoch = doc.text_epoch();
        self.preview_edit_barrier();
        // The next family and same-poll formatter must see the owned edit. A
        // failed publication revokes this pipeline rather than saving it raw.
        if let Err(error) = self
            .lsp
            .as_mut()
            .context("Native server disconnected")?
            .sync(&self.documents)
        {
            run.source_failed = true;
            return Err(error);
        }
        Ok(true)
    }
    fn advance_save_actions(&mut self, run: &mut Run) -> Result<()> {
        ensure!(
            !run.source_failed,
            "Post-action native synchronization failed"
        );
        while run.pending.is_none() && run.family < run.plan.families.len() {
            ensure!(
                self.action_save_current(run),
                "Save action ownership changed"
            );
            if run.started.elapsed() >= DEADLINE {
                self.action_save_notice(run.proof.intent, "1500 ms participant deadline elapsed");
                run.family = run.plan.families.len();
                break;
            }
            let family = run.plan.families[run.family];
            if run.origin.is_none() {
                let client = self.lsp.as_mut().context("Native server disconnected")?;
                if !client.action_available() {
                    self.action_save_notice(run.proof.intent, "native action lane is busy");
                    run.family = run.plan.families.len();
                    break;
                }
                client.sync(&self.documents)?;
                let doc = self
                    .documents
                    .iter()
                    .find(|doc| doc.id == run.proof.document)
                    .context("Save model was closed")?;
                let diagnostics = self
                    .diagnostics
                    .get(&run.proof.path)
                    .filter(|publication| {
                        client.diagnostic_current(publication, doc)
                            && publication.items.len() <= 128
                            && publication.serialized_bytes <= 128 * 1024
                    })
                    .map_or_else(Vec::new, |publication| {
                        publication
                            .items
                            .iter()
                            .map(|item| {
                                serde_json::to_value(item).expect("native diagnostic serialization")
                            })
                            .collect()
                    });
                let token = client.request_code_actions(doc, json!({
                    "range":{"start":{"line":0,"character":0},"end":crate::lsp::position(doc,doc.len())},
                    "context":{"diagnostics":diagnostics,"only":[family.kind()],"triggerKind":2}
                }), None)?;
                run.pending = Some(Pending {
                    token,
                    phase: Phase::Discovery,
                });
                break;
            }
            let Some(item) = run.candidates.pop_front() else {
                Self::next_save_action_family(run);
                continue;
            };
            let Some(kind) = item.get("kind").and_then(Value::as_str) else {
                continue;
            };
            if !run.plan.allows(family, kind) || item.get("disabled").is_some() {
                continue;
            }
            if item.get("command").is_some() {
                self.action_save_notice(
                    run.proof.intent,
                    "command-bearing source action requires a separate qualified command lifetime",
                );
                continue;
            }
            if item.get("edit").is_some() {
                // Move the captured proof around mutable staging rather than
                // copying up to 10,000 selections for every candidate.
                let request = run.origin.take().expect("provide request captured");
                let staged = self.stage_save_action(run, &request, &item);
                run.origin = Some(request);
                match staged {
                    Ok(true) => Self::next_save_action_family(run),
                    Ok(false) => {}
                    Err(error) if run.source_failed => return Err(error),
                    Err(error) => self.action_save_notice(run.proof.intent, format!("{error:#}")),
                }
                continue;
            }
            if run.resolves >= 8 {
                self.action_save_notice(run.proof.intent, "eight lazy resolves per family reached");
                continue;
            }
            let client = self.lsp.as_mut().context("Native server disconnected")?;
            if client.capabilities["codeActionProvider"]["resolveProvider"] != true {
                continue;
            }
            let request = run.origin.as_ref().expect("provide request captured");
            match client.resolve_code_action(request, item) {
                Ok(token) => {
                    run.resolves += 1;
                    run.pending = Some(Pending {
                        token,
                        phase: Phase::Resolve,
                    });
                }
                Err(error) => self.action_save_notice(run.proof.intent, format!("{error:#}")),
            }
        }
        Ok(())
    }
    pub(super) fn save_code_actions_owned(&self, request: &Request) -> bool {
        self.saving.actions.active.as_ref().is_some_and(|run| {
            run.pending
                .as_ref()
                .is_some_and(|pending| pending.token == request.token)
                && Arc::ptr_eq(&run.proof.server, &request.server)
        })
    }
    pub(super) fn save_code_actions_result(
        &mut self,
        request: Request,
        result: Result<Value, String>,
    ) {
        if !self.save_code_actions_owned(&request) {
            return;
        }
        let mut run = self.saving.actions.active.take().expect("owned run");
        if !self.action_save_current(&run)
            || request.document_id != run.proof.document
            || request.revision != run.proof.revision
            || request.text_epoch != run.proof.epoch
            || !self
                .lsp
                .as_ref()
                .is_some_and(|client| client.request_current(&request))
        {
            self.cancel_action_run(&run);
            if self
                .saving
                .latest
                .as_ref()
                .is_some_and(|intent| intent.id == run.proof.intent)
            {
                self.saving.latest = None;
            }
            self.message = "Save retired; unsaved work retained: action ownership changed".into();
            return;
        }
        let phase = run.pending.take().expect("owned pending").phase;
        if run.started.elapsed() >= DEADLINE {
            self.action_save_notice(
                run.proof.intent,
                "1500 ms deadline elapsed; late result discarded after actual settlement",
            );
            self.finish_save_actions(run.proof.intent);
            return;
        }
        match (phase, result) {
            (Phase::Discovery, Ok(Value::Array(items))) => {
                run.origin = Some(request);
                run.candidates = items.into();
            }
            (Phase::Discovery, Ok(Value::Null)) => Self::next_save_action_family(&mut run),
            (Phase::Discovery, Ok(_)) => {
                self.action_save_notice(run.proof.intent, "invalid source action list");
                Self::next_save_action_family(&mut run);
            }
            (Phase::Discovery, Err(error)) => {
                self.action_save_notice(run.proof.intent, error);
                Self::next_save_action_family(&mut run);
            }
            (Phase::Resolve, Err(error)) => self.action_save_notice(run.proof.intent, error),
            (Phase::Resolve, Ok(item)) => {
                let family = run.plan.families[run.family];
                if item.get("command").is_some() {
                    self.action_save_notice(
                        run.proof.intent,
                        "resolved command-bearing action skipped before mutation",
                    );
                } else if item.get("disabled").is_none()
                    && item
                        .get("kind")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| run.plan.allows(family, kind))
                    && item.get("edit").is_some()
                {
                    match self.stage_save_action(&mut run, &request, &item) {
                        Ok(true) => Self::next_save_action_family(&mut run),
                        Ok(false) => {}
                        Err(error) => {
                            self.action_save_notice(run.proof.intent, format!("{error:#}"))
                        }
                    }
                }
            }
        }
        self.resume_save_actions(run);
    }
    fn resume_save_actions(&mut self, mut run: Run) {
        if let Err(error) = self.advance_save_actions(&mut run) {
            if run.source_failed || !self.action_save_current(&run) {
                self.cancel_action_run(&run);
                if self
                    .saving
                    .latest
                    .as_ref()
                    .is_some_and(|intent| intent.id == run.proof.intent)
                {
                    self.saving.latest = None;
                }
                self.message =
                    "Save retired; unsaved work retained: action ownership changed".into();
                return;
            }
            self.action_save_notice(run.proof.intent, format!("{error:#}"));
            self.finish_save_actions(run.proof.intent);
        } else if run.family >= run.plan.families.len() && run.pending.is_none() {
            self.finish_save_actions(run.proof.intent);
        } else {
            self.saving.actions.active = Some(run);
        }
    }
    pub(super) fn poll_save_code_actions(&mut self, now: Instant) -> bool {
        let Some(run) = self.saving.actions.active.as_ref() else {
            return false;
        };
        if !self.action_save_current(run)
            || run.pending.as_ref().is_some_and(|pending| {
                !self
                    .lsp
                    .as_ref()
                    .is_some_and(|client| client.action_request_current(pending.token))
            })
        {
            return self.retire_save_code_actions(
                "model, policy or native action synchronization changed",
            );
        }
        if now.saturating_duration_since(run.started) >= DEADLINE {
            let run = self.saving.actions.active.take().expect("pending checked");
            self.cancel_action_run(&run);
            self.action_save_notice(run.proof.intent,"1500 ms deadline elapsed; canceled native work remains occupied until its actual reply");
            self.finish_save_actions(run.proof.intent);
            return true;
        }
        false
    }
}
