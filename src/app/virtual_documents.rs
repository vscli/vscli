//! Joined content preparation and retained receipts for virtual and untitled resources.
use super::*;
use crate::{
    document::PreparedVirtualContent,
    extensions::{
        services::{NativeService, Operation},
        text_content::Capture,
    },
};
use anyhow::{Context, ensure};
use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

enum Prepared {
    Open(Box<Document>),
    Change(PreparedVirtualContent),
    Untitled(Box<Document>),
}
struct Job {
    service: NativeService,
    thread: JoinHandle<Result<Prepared, String>>,
    invalid: Option<String>,
}
struct Completed {
    service: NativeService,
    prepared: Option<Result<Prepared, String>>,
    receipt: Option<Result<Value, String>>,
}
#[derive(Default)]
pub(super) struct State {
    active: Option<Job>,
    completed: Option<Completed>,
}
fn capture(service: &NativeService) -> Result<&Arc<Capture>> {
    match &service.operation {
        Operation::VirtualOpen(capture) | Operation::VirtualChange(capture) => Ok(capture),
        _ => anyhow::bail!("Invalid virtual document service"),
    }
}
impl App {
    pub(super) fn virtual_documents_pending(&self) -> bool {
        self.virtual_documents.active.is_some() || self.virtual_documents.completed.is_some()
    }
    pub(super) fn cancel_virtual_documents(&mut self) {
        if let Some(job) = &mut self.virtual_documents.active {
            job.invalid
                .get_or_insert_with(|| "Original virtual document host retired".into());
        }
    }
    pub(super) fn settle_virtual_documents(&mut self) {
        if let Some(job) = self.virtual_documents.active.take() {
            let _ = job.thread.join();
        }
        self.virtual_documents.completed = None;
    }
    fn virtual_current(
        &self,
        host: &crate::extensions::Client,
        service: &NativeService,
    ) -> Result<()> {
        host.service_valid(service)?;
        if let Operation::UntitledOpen(input) = &service.operation {
            ensure!(
                self.settings_error.is_none()
                    && input.current(
                        &self.workspace.root,
                        self.settings.extension_layers(),
                        self.settings_profile_generation()
                    ),
                "Untitled creation settings/workspace changed"
            );
            Ok(())
        } else {
            host.text_content_current(capture(service)?, &self.documents, &self.hidden_documents)
        }
    }
    pub(super) fn start_virtual_document(
        &mut self,
        host: &crate::extensions::Client,
        service: NativeService,
    ) -> Result<()> {
        ensure!(
            !self.virtual_documents_pending(),
            "Virtual document worker is actually occupied"
        );
        self.virtual_current(host, &service)?;
        if let Operation::UntitledOpen(input) = &service.operation
            && input.requested_uri
            && let Some(id) = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find(|doc| {
                    doc.untitled_uri().is_some() && doc.untitled_uri() == input.uri.as_deref()
                })
                .map(|doc| doc.id)
        {
            self.acquire_api_document(service.request.session, id, std::time::Instant::now())?;
            self.virtual_documents.completed = Some(Completed {
                service: service.clone(),
                prepared: None,
                receipt: Some(Ok(json!({"document":id}))),
            });
            return Ok(());
        }
        let operation = service.operation.clone();
        let thread = thread::Builder::new()
            .name("vscli-content-document".into())
            .spawn(move || {
                (|| -> Result<Prepared> {
                    match operation {
                        Operation::UntitledOpen(input) => {
                            Ok(Prepared::Untitled(Box::new(input.prepare()?)))
                        }
                        Operation::VirtualChange(input) => Ok(Prepared::Change(
                            Document::prepare_virtual_content(&input.content)?,
                        )),
                        Operation::VirtualOpen(input) => {
                            let mut doc =
                                Document::from_virtual(input.uri.clone(), &input.content)?;
                            let uri = url::Url::parse(&input.uri)?;
                            doc.set_language_override(Some(
                                crate::lsp::language(std::path::Path::new(uri.path())).into(),
                            ))?;
                            Ok(Prepared::Open(Box::new(doc)))
                        }
                        _ => anyhow::bail!("Invalid generated document operation"),
                    }
                })()
                .map_err(|error| crate::extensions::services::bounded_error(&error))
            })?;
        self.virtual_documents.active = Some(Job {
            service,
            thread,
            invalid: None,
        });
        Ok(())
    }
    fn publish_virtual_document(
        &mut self,
        service: &NativeService,
        output: Prepared,
    ) -> Result<Value> {
        let total: usize = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .map(|doc| doc.text.len_bytes())
            .sum();
        let id = match output {
            Prepared::Untitled(mut doc) => {
                if let Some(old) = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|old| {
                        old.untitled_uri().is_some() && old.untitled_uri() == doc.untitled_uri()
                    })
                {
                    ensure!(
                        matches!(&service.operation, Operation::UntitledOpen(input) if input.requested_uri),
                        "Fresh untitled resource collided with a retained document"
                    );
                    old.id
                } else {
                    ensure!(
                        self.hidden_documents.len() < 16
                            && total.saturating_add(doc.text.len_bytes()) <= 4 * 1024 * 1024,
                        "Untitled creation exceeds hidden model or mirror budget"
                    );
                    self.settings.apply(&mut doc);
                    self.configure_document_language(&mut doc)?;
                    let id = doc.id;
                    self.hidden_documents.push(*doc);
                    id
                }
            }
            Prepared::Open(mut doc) => {
                let input = capture(service)?;
                if let Some(old) = self
                    .documents
                    .iter()
                    .chain(&self.hidden_documents)
                    .find(|doc| doc.virtual_uri() == Some(input.uri.as_str()))
                {
                    old.id
                } else {
                    ensure!(
                        self.hidden_documents.len() < 16
                            && total.saturating_add(doc.text.len_bytes()) <= 4 * 1024 * 1024,
                        "Virtual document exceeds hidden model or mirror budget"
                    );
                    self.settings.apply(&mut doc);
                    self.configure_document_language(&mut doc)?;
                    let id = doc.id;
                    self.hidden_documents.push(*doc);
                    id
                }
            }
            Prepared::Change(content) => {
                let input = capture(service)?;
                let id = input.document.context("Missing virtual refresh document")?;
                let doc = self
                    .documents
                    .iter_mut()
                    .chain(&mut self.hidden_documents)
                    .find(|doc| doc.id == id)
                    .context("Virtual refresh document closed")?;
                ensure!(
                    total
                        .saturating_sub(doc.text.len_bytes())
                        .saturating_add(content.len_bytes())
                        <= 4 * 1024 * 1024,
                    "Virtual refresh exceeds mirror budget"
                );
                doc.refresh_virtual_prepared(content)?;
                id
            }
        };
        if matches!(
            service.operation,
            Operation::UntitledOpen(_) | Operation::VirtualOpen(_)
        ) {
            self.acquire_api_document(service.request.session, id, std::time::Instant::now())?;
        }
        Ok(json!({"document":id}))
    }
    pub(super) fn poll_virtual_documents(
        &mut self,
        mut host: Option<&mut crate::extensions::Client>,
    ) -> bool {
        if let Some(job) = &self.virtual_documents.active {
            let invalid = match host.as_deref() {
                Some(host) => self
                    .virtual_current(host, &job.service)
                    .err()
                    .map(|error| crate::extensions::services::bounded_error(&error)),
                None => Some("Original virtual document host retired".into()),
            };
            if let Some(reason) = invalid
                && let Some(job) = &mut self.virtual_documents.active
            {
                job.invalid.get_or_insert(reason);
            }
        }
        let finished = self
            .virtual_documents
            .active
            .as_ref()
            .is_some_and(|job| job.thread.is_finished());
        if finished {
            let job = self
                .virtual_documents
                .active
                .take()
                .expect("finished virtual worker");
            // The actual thread exits and joins before its occupancy is released.
            let prepared = match job.thread.join() {
                Ok(result) => match job.invalid {
                    Some(error) => Err(error),
                    None => result,
                },
                Err(_) => Err("Virtual content worker panicked".into()),
            };
            self.virtual_documents.completed = Some(Completed {
                service: job.service,
                prepared: Some(prepared),
                receipt: None,
            });
        }
        let Some(mut done) = self.virtual_documents.completed.take() else {
            return false;
        };
        let Some(host) = host.as_mut() else {
            return true;
        };
        let exact = host.service().is_some_and(|service| {
            service.id == done.service.id
                && service.request.session == done.service.request.session
                && service.request.owner == done.service.request.owner
        });
        if !exact {
            if done.service.request.session == host.session {
                self.virtual_documents.completed = Some(done);
            }
            return finished;
        }
        if done.receipt.is_none() {
            let receipt = self.virtual_current(host, &done.service).and_then(|()| {
                match done
                    .prepared
                    .take()
                    .context("Virtual prepared receipt missing")?
                {
                    Ok(output) => self.publish_virtual_document(&done.service, output),
                    Err(error) => anyhow::bail!(error),
                }
            });
            done.receipt =
                Some(receipt.map_err(|error| crate::extensions::services::bounded_error(&error)));
        }
        // Never repeat publication or settle the promise before the native mirror.
        if let Err(error) =
            host.sync_with_hidden(&self.documents, &self.hidden_documents, self.active)
        {
            self.message = format!("Virtual document mirror retry retained: {error:#}");
            self.virtual_documents.completed = Some(done);
            return finished;
        }
        let receipt = match done.receipt.as_ref().expect("published virtual receipt") {
            Ok(value) => host
                .service_valid(&done.service)
                .and_then(|()| {
                    if let Operation::UntitledOpen(input) = &done.service.operation {
                        ensure!(
                            self.settings_error.is_none()
                                && input.current(
                                    &self.workspace.root,
                                    self.settings.extension_layers(),
                                    self.settings_profile_generation()
                                ),
                            "Untitled creation context retired before acknowledgement"
                        );
                        Ok(())
                    } else {
                        host.text_content_provider_current(capture(&done.service)?)
                    }
                })
                .map(|()| value.clone()),
            Err(error) => Err(anyhow::anyhow!(error.clone())),
        };
        if let Err(error) = host.answer_service(&done.service, receipt) {
            self.message = format!("Virtual document reply retained: {error:#}");
            self.virtual_documents.completed = Some(done);
            return finished;
        }
        true
    }
}
