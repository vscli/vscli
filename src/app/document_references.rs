//! Native customer references and separate model/editor/history ownership.
use super::*;
use crate::document_references::{ModelKey, References, ResourceIdentity, ResourceKey};
use anyhow::{Context, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

mod discard;
use discard::Discard;

#[derive(Default)]
pub(super) struct State {
    ledger: Option<References>,
    // Only models genuinely acquired through the native API participate in
    // resolver cleanup. Native transaction/other hidden models have other owners.
    managed: BTreeSet<ModelKey>,
    lengths: BTreeMap<ModelKey, Length>,
    discard: Option<Discard>,
}
struct Length {
    epoch: u64,
    eol: String,
    units: u64,
}

fn model_key(doc: &Document) -> ModelKey {
    ModelKey {
        document: doc.id,
        resource_epoch: doc.resource_epoch(),
    }
}
fn resource_key(doc: &Document) -> Result<ResourceKey> {
    let original = crate::extensions::document_uri(doc)?;
    let uri = url::Url::parse(&original)?;
    Ok(ResourceKey::new(
        &original,
        ResourceIdentity {
            scheme: uri.scheme().into(),
            authority: uri[url::Position::BeforeUsername..url::Position::AfterPort].into(),
            path: uri.path().into(),
            query: uri.query().unwrap_or_default().into(),
            fragment: uri.fragment().unwrap_or_default().into(),
        },
    )?)
}
// The public model joins logical lines with its EOL. Mixed raw newline bytes
// remain native-owned, but cannot inflate/shrink the captured model length.
fn value_length(doc: &Document) -> u64 {
    let mut units = doc.text.len_utf16_cu() as u64;
    let mut newline_units = 0u64;
    let mut breaks = 0u64;
    let mut cr = false;
    for character in doc.text.chars() {
        match character {
            '\r' => {
                newline_units += 1;
                breaks += 1;
                cr = true;
            }
            '\n' => {
                newline_units += 1;
                if !cr {
                    breaks += 1;
                }
                cr = false;
            }
            _ => cr = false,
        }
    }
    units -= newline_units;
    units + breaks * doc.eol.len() as u64
}

impl App {
    pub(super) fn remove_api_operation_references(
        &mut self,
        action: &crate::files::Action,
    ) -> Result<()> {
        let (path, moved) = match action {
            crate::files::Action::Rename { from, .. } => (from, true),
            crate::files::Action::Trash(path) => (path, false),
            _ => return Ok(()),
        };
        let Some(ledger) = self.api_documents.ledger.as_mut() else {
            return Ok(());
        };
        let uri = url::Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("Invalid completed resource URI"))?;
        let key = ResourceKey::new(
            uri.as_str(),
            ResourceIdentity {
                scheme: uri.scheme().into(),
                authority: uri[url::Position::BeforeUsername..url::Position::AfterPort].into(),
                path: uri.path().into(),
                query: uri.query().unwrap_or_default().into(),
                fragment: uri.fragment().unwrap_or_default().into(),
            },
        )?;
        if moved {
            ledger.remove_move_source(ledger.customer_epoch(), &key)?;
        } else {
            ledger.remove_deleted_resource(ledger.customer_epoch(), &key)?;
        }
        Ok(())
    }
    pub(super) fn bind_api_document_customer(&mut self, session: u64) -> Result<()> {
        match &mut self.api_documents.ledger {
            Some(ledger) if ledger.customer_epoch() != session => {
                ledger.replace_customer(ledger.customer_epoch(), session)?;
            }
            Some(ledger) => {
                ledger.cached_open(
                    session,
                    ModelKey {
                        document: 1,
                        resource_epoch: 0,
                    },
                )?;
            }
            None => self.api_documents.ledger = Some(References::new(session)?),
        }
        Ok(())
    }
    pub(super) fn acquire_api_document(
        &mut self,
        session: u64,
        document: u64,
        acquired_at: Instant,
    ) -> Result<()> {
        self.bind_api_document_customer(session)?;
        let doc = self
            .documents
            .iter()
            .chain(&self.hidden_documents)
            .find(|doc| doc.id == document)
            .context("API acquisition model retired")?;
        ensure!(
            self.documents
                .len()
                .saturating_add(self.hidden_documents.len())
                <= 128,
            "API model ownership exceeds 128 retained models"
        );
        ensure!(
            doc.text.len_bytes() <= 4 * 1024 * 1024,
            "API acquisition text exceeds mirror budget"
        );
        let model = model_key(doc);
        let resource = resource_key(doc)?;
        // A genuine alias open may acquire again, but unchanged original text
        // need not be rescanned. This is native metadata, independent of public
        // document getters and of already captured acquisition lengths.
        let length = self
            .api_documents
            .lengths
            .get(&model)
            .filter(|length| length.epoch == doc.text_epoch() && length.eol == doc.eol)
            .map_or_else(|| value_length(doc), |length| length.units);
        let proof = Length {
            epoch: doc.text_epoch(),
            eol: doc.eol.clone(),
            units: length,
        };
        self.api_documents.managed.retain(|key| {
            self.documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|doc| model_key(doc) == *key)
        });
        self.api_documents
            .lengths
            .retain(|key, _| self.api_documents.managed.contains(key));
        self.api_documents
            .ledger
            .as_mut()
            .expect("bound native customer")
            .acquire(session, model, resource, length, acquired_at)?;
        self.api_documents.managed.insert(model);
        self.api_documents.lengths.insert(model, proof);
        Ok(())
    }
    pub(super) fn retire_api_document_customer(&mut self, session: u64) {
        if let Some(ledger) = &mut self.api_documents.ledger
            && ledger.customer_epoch() == session
        {
            let _ = ledger.retire_customer(session);
        }
    }
    fn api_model_referenced(&self, doc: &Document) -> bool {
        self.api_documents
            .ledger
            .as_ref()
            .is_some_and(|ledger| ledger.retention(model_key(doc)).acquisitions != 0)
    }
    pub(super) fn prepare_api_tab_retention(&mut self, document: u64) -> Result<()> {
        if self
            .documents
            .iter()
            .find(|doc| doc.id == document)
            .is_some_and(|doc| self.api_model_referenced(doc))
        {
            self.hidden_documents
                .try_reserve(1)
                .context("Cannot reserve retained API model storage")?;
        }
        Ok(())
    }
    pub(super) fn remove_or_retain_api_document(&mut self, index: usize) {
        // Callers reserve before changing memberships. Moving existing storage
        // does not acquire another reference or enlarge model/text admission.
        let retain = self.api_model_referenced(&self.documents[index]);
        let doc = self.documents.remove(index);
        if retain {
            self.hidden_documents.push(doc);
        } else {
            self.api_documents.managed.remove(&model_key(&doc));
            self.api_documents.lengths.remove(&model_key(&doc));
        }
    }
    pub(super) fn poll_api_document_references(
        &mut self,
        session: Option<u64>,
        now: Instant,
    ) -> bool {
        if let Some(session) = session {
            if let Err(error) = self.bind_api_document_customer(session) {
                self.message = format!("Native API customer refused: {error:#}");
                return false;
            }
            if let Some(ledger) = &mut self.api_documents.ledger
                && ledger.next_expiry().is_some_and(|deadline| now >= deadline)
            {
                let _ = ledger.expire_due(session, now);
            }
        } else if let Some(ledger) = &mut self.api_documents.ledger {
            let _ = ledger.retire_customer(ledger.customer_epoch());
        }
        // Queue/worker owners outlive API timers. Ordinary extension retirement
        // or interaction cancellation cannot free an actually occupied model.
        let pending = self.persistence_pending()
            || self.workspace_transactions_pending()
            || self.virtual_documents_pending()
            || self.api_documents.discard.is_some()
            || self.extension_services_pending()
            || self
                .extension_host
                .as_ref()
                .is_some_and(|host| host.busy() || host.queued_services().next().is_some());
        let mut changed = false;
        let mut index = 0;
        while index < self.hidden_documents.len() {
            let doc = &self.hidden_documents[index];
            let key = model_key(doc);
            let eligible = self.api_documents.managed.contains(&key)
                && !self.api_model_referenced(doc)
                && !pending
                && !doc.dirty()
                && !doc.retains_text_history()
                && !self.save_as_document_pending(doc.id)
                && !self.document_save_pending(doc.id)
                && !self.panes.iter().any(|pane| pane.document == doc.id)
                && self.editor_groups.memberships(doc.id).next().is_none();
            if eligible {
                self.hidden_documents.remove(index);
                self.api_documents.managed.remove(&key);
                changed = true;
            } else {
                index += 1;
            }
        }
        // Retired bindings cannot grant ownership to a new resource epoch or
        // keep metadata after an explicitly authorized close. At most 128 live
        // model keys remain; stale acquisition timers keep their original IDs.
        self.api_documents.managed.retain(|key| {
            self.documents
                .iter()
                .chain(&self.hidden_documents)
                .any(|doc| model_key(doc) == *key)
        });
        self.api_documents
            .lengths
            .retain(|key, _| self.api_documents.managed.contains(key));
        changed
    }
}

#[cfg(test)]
mod tests;
