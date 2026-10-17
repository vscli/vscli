//! Native file discovery keeps its actual worker and terminal reply separate.
use super::*;
use crate::{
    extensions::services::{NativeService, Operation},
    workspace_file_search::{Output, Worker},
};
use std::sync::atomic::Ordering;
struct Job {
    token: u64,
    service: NativeService,
    invalid: Option<String>,
}
struct Completed {
    service: NativeService,
    result: Result<Output, String>,
}
#[derive(Default)]
pub(super) struct State {
    pub(crate) worker: Worker,
    pending: Option<Job>,
    completed: Option<Completed>,
}
impl State {
    fn occupied(&self) -> bool {
        self.worker.occupied() || self.pending.is_some() || self.completed.is_some()
    }
}
impl App {
    pub(super) fn file_search_pending(&self) -> bool {
        self.extension_file_search.occupied()
    }
    pub(super) fn cancel_extension_file_search(&mut self) {
        if let Some(job) = &self.extension_file_search.pending {
            self.extension_file_search.worker.cancel(job.token);
        }
    }
    pub(super) fn settle_extension_file_search(&mut self) {
        self.extension_file_search.worker.settle();
        self.extension_file_search.pending = None;
        self.extension_file_search.completed = None;
    }
    fn file_search_current(
        &self,
        host: &crate::extensions::Client,
        service: &NativeService,
    ) -> Result<()> {
        host.service_valid(service)?;
        let Operation::FindFiles(capture, _) = &service.operation else {
            anyhow::bail!("Invalid native file search operation");
        };
        if self.settings_error.is_some()
            || !capture.current(
                &self.workspace.root,
                self.settings.extension_layers(),
                self.settings_profile_generation(),
            )
        {
            anyhow::bail!("Workspace or settings changed during file discovery");
        }
        Ok(())
    }
    pub(super) fn start_extension_file_search(&mut self, service: NativeService) -> Result<()> {
        anyhow::ensure!(
            !self.extension_file_search.occupied(),
            "Native file discovery is actually occupied"
        );
        let Operation::FindFiles(capture, cancelled) = &service.operation else {
            anyhow::bail!("Invalid native file discovery operation");
        };
        anyhow::ensure!(
            self.settings_error.is_none()
                && capture.current(
                    &self.workspace.root,
                    self.settings.extension_layers(),
                    self.settings_profile_generation()
                ),
            "File discovery context changed before dispatch"
        );
        if cancelled.load(Ordering::Acquire) {
            self.extension_file_search.completed = Some(Completed {
                service,
                result: Ok(Output {
                    uris: Vec::new(),
                    cancelled: true,
                }),
            });
            return Ok(());
        }
        let token = self
            .extension_file_search
            .worker
            .start(capture.clone(), service.caller_retired.clone())?;
        self.extension_file_search.pending = Some(Job {
            token,
            service,
            invalid: None,
        });
        Ok(())
    }
    pub(super) fn poll_extension_file_search(
        &mut self,
        mut host: Option<&mut crate::extensions::Client>,
    ) -> bool {
        if let Some(job) = &self.extension_file_search.pending {
            let invalid = match host.as_deref() {
                Some(host) => self
                    .file_search_current(host, &job.service)
                    .err()
                    .map(|error| crate::extensions::services::bounded_error(&error)),
                None => Some("Original file discovery host retired".into()),
            };
            let explicitly_cancelled = matches!(&job.service.operation,Operation::FindFiles(_,cancelled) if cancelled.load(Ordering::Acquire));
            if invalid.is_some() || explicitly_cancelled {
                self.extension_file_search.worker.cancel(job.token);
                if let Some(job) = &mut self.extension_file_search.pending
                    && job.invalid.is_none()
                {
                    job.invalid = invalid;
                }
            }
        }
        let mut changed = false;
        if let Some((token, result)) = self.extension_file_search.worker.poll() {
            changed = true;
            if self
                .extension_file_search
                .pending
                .as_ref()
                .is_some_and(|job| job.token == token)
            {
                let job = self
                    .extension_file_search
                    .pending
                    .take()
                    .expect("validated native discovery token");
                self.extension_file_search.completed = Some(Completed {
                    service: job.service,
                    result: match job.invalid {
                        Some(error) => Err(error),
                        None => result,
                    },
                });
            } else {
                self.message = "Native file discovery identity mismatch; reply retained".into();
            }
        }
        let Some(done) = self.extension_file_search.completed.as_ref() else {
            return changed;
        };
        let service = done.service.clone();
        let Some(host) = host.as_mut() else {
            self.extension_file_search.completed = None;
            return true;
        };
        let exact = host.service().is_some_and(|current| {
            current.id == service.id
                && current.request.session == service.request.session
                && current.request.owner == service.request.owner
                && matches!(current.operation, Operation::FindFiles(..))
        });
        if !exact {
            if service.request.session != host.session
                || service.caller_retired.load(Ordering::Acquire)
            {
                self.extension_file_search.completed = None;
                return true;
            }
            return changed;
        }
        let result = self
            .file_search_current(host, &service)
            .and_then(|()| match &done.result {
                Ok(output) => {
                    Ok(serde_json::json!({"uris":output.uris,"cancelled":output.cancelled}))
                }
                Err(error) => Err(anyhow::anyhow!(error.clone())),
            });
        // Discovery is read-only; no document mirror or UI snapshot authorizes it.
        match host.answer_service(&service, result) {
            Ok(()) => {
                self.extension_file_search.completed = None;
                true
            }
            Err(error) => {
                self.message = format!("Native file discovery reply retained: {error:#}");
                changed
            }
        }
    }
}
