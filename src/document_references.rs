//! Customer-wide API model references, independent of editor/model ownership.
//!
//! There are no timers or filesystem calls here. The native caller supplies the
//! original acquisition instant and polls deadlines; releases are evidence for
//! ownership arbitration, never instructions to destroy a document.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const MAX_AGE: Duration = Duration::from_secs(180);
pub const MAX_LENGTH_UNITS: u64 = 80 * 1024 * 1024;
pub const COUNT_TRIGGER: usize = 60;
pub const COUNT_AFTER_EVICTION: usize = 50;
pub const MAX_RESOURCE_BYTES: usize = 8192;

/// URI identity components already normalized by the native resource authority.
/// `path` uses '/' boundaries; provider case/UNC normalization belongs to the
/// caller. Query and fragment remain part of identity, including for files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceIdentity {
    pub scheme: String,
    pub authority: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceKey {
    uri: String,
    identity: ResourceIdentity,
}
impl ResourceKey {
    pub fn new(uri: &str, identity: ResourceIdentity) -> Result<Self, Error> {
        let parts = [
            uri,
            &identity.scheme,
            &identity.authority,
            &identity.path,
            &identity.query,
            &identity.fragment,
        ];
        let length = parts
            .iter()
            .try_fold(0usize, |total, part| total.checked_add(part.len()));
        if identity.scheme.is_empty()
            || identity.path.is_empty()
            || parts.iter().any(|part| part.contains('\0'))
            || length.is_none_or(|length| length > MAX_RESOURCE_BYTES)
        {
            return Err(Error::InvalidResource);
        }
        // Rebuild validated strings instead of retaining arbitrary spare capacity
        // from a caller's path/URI builder.
        let identity = ResourceIdentity {
            scheme: identity.scheme.as_str().to_owned(),
            authority: identity.authority.as_str().to_owned(),
            path: identity.path.as_str().to_owned(),
            query: identity.query.as_str().to_owned(),
            fragment: identity.fragment.as_str().to_owned(),
        };
        Ok(Self {
            uri: uri.to_owned(),
            identity,
        })
    }
    pub fn uri(&self) -> &str {
        &self.uri
    }
    pub fn identity(&self) -> &ResourceIdentity {
        &self.identity
    }
    pub fn same_resource(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
    pub fn equal_or_descendant_of(&self, parent: &Self) -> bool {
        let resource = &self.identity;
        let parent = &parent.identity;
        if resource.scheme != parent.scheme
            || resource.authority != parent.authority
            || resource.query != parent.query
            || resource.fragment != parent.fragment
        {
            return false;
        }
        if resource.path == parent.path {
            return true;
        }
        let prefix = parent.path.trim_end_matches('/');
        resource
            .path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

/// Both fields come from the actual retained Document at acquisition time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelKey {
    pub document: u64,
    pub resource_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcquisitionId {
    pub customer_epoch: u64,
    pub sequence: u64,
}

#[derive(Debug)]
pub struct Acquisition {
    pub id: AcquisitionId,
    pub model: ModelKey,
    pub resource: ResourceKey,
    pub acquired_at: Instant,
    pub expires_at: Instant,
    /// Captured getValueLength units, not UTF-8 bytes or current model length.
    pub captured_length_units: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseCause {
    Expired,
    LengthLimit,
    CountLimit,
    MoveSource,
    DeletedResource,
    UntitledRevert,
    CustomerRetired,
}

#[derive(Debug)]
pub struct Release {
    pub acquisition: Acquisition,
    pub cause: ReleaseCause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    pub model: ModelKey,
    pub acquisitions: usize,
    pub captured_length_units: u64,
}

#[derive(Debug)]
pub struct Decision {
    /// An admitted acquisition can itself be immediately evicted by length.
    pub acquired: Option<AcquisitionId>,
    pub released: Vec<Release>,
    /// Complete current reference ownership, grouped by ORIGINAL model epoch.
    pub retained: Vec<Retention>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidCustomer,
    StaleCustomer,
    RetiredCustomer,
    InvalidModel,
    InvalidResource,
    NotUntitled,
    IdentityExhausted,
    DeadlineOverflow,
    Capacity,
}
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Document reference admission: {self:?}")
    }
}
impl std::error::Error for Error {}

pub struct References {
    customer_epoch: u64,
    retired: bool,
    last_sequence: u64,
    entries: VecDeque<Acquisition>,
    length_units: u64,
}
impl References {
    pub fn new(customer_epoch: u64) -> Result<Self, Error> {
        if customer_epoch == 0 {
            return Err(Error::InvalidCustomer);
        }
        Ok(Self {
            customer_epoch,
            retired: false,
            last_sequence: 0,
            entries: VecDeque::new(),
            length_units: 0,
        })
    }
    pub fn customer_epoch(&self) -> u64 {
        self.customer_epoch
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn captured_length_units(&self) -> u64 {
        self.length_units
    }
    pub fn next_expiry(&self) -> Option<Instant> {
        self.entries.iter().map(|entry| entry.expires_at).min()
    }
    pub fn contains(&self, id: AcquisitionId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
    pub fn retention(&self, model: ModelKey) -> Retention {
        self.entries
            .iter()
            .filter(|entry| entry.model == model)
            .fold(
                Retention {
                    model,
                    acquisitions: 0,
                    captured_length_units: 0,
                },
                |mut result, entry| {
                    result.acquisitions += 1;
                    result.captured_length_units += entry.captured_length_units;
                    result
                },
            )
    }
    /// A public mirrored cache hit is an observation, with no lease/clock change.
    pub fn cached_open(&self, customer_epoch: u64, model: ModelKey) -> Result<Retention, Error> {
        self.check_customer(customer_epoch)?;
        Ok(self.retention(model))
    }
    /// Call only after a genuine native model-reference acquisition succeeds.
    /// Admission does not implicitly poll expiry, matching independent timers.
    pub fn acquire(
        &mut self,
        customer_epoch: u64,
        model: ModelKey,
        resource: ResourceKey,
        captured_length_units: u64,
        acquired_at: Instant,
    ) -> Result<Decision, Error> {
        self.check_customer(customer_epoch)?;
        if model.document == 0 {
            return Err(Error::InvalidModel);
        }
        let sequence = self
            .last_sequence
            .checked_add(1)
            .ok_or(Error::IdentityExhausted)?;
        let expires_at = acquired_at
            .checked_add(MAX_AGE)
            .ok_or(Error::DeadlineOverflow)?;
        self.entries.try_reserve(1).map_err(|_| Error::Capacity)?;
        let mut released = Vec::new();
        released
            .try_reserve(COUNT_TRIGGER)
            .map_err(|_| Error::Capacity)?;
        let id = AcquisitionId {
            customer_epoch,
            sequence,
        };
        // Wider temporary total also handles a single oversized acquisition
        // faithfully: release older entries and then that acquisition itself.
        let mut total = u128::from(self.length_units) + u128::from(captured_length_units);
        self.last_sequence = sequence;
        self.entries.push_back(Acquisition {
            id,
            model,
            resource,
            acquired_at,
            expires_at,
            captured_length_units,
        });
        while total > u128::from(MAX_LENGTH_UNITS) {
            let entry = self
                .entries
                .pop_front()
                .expect("positive length has an owner");
            total -= u128::from(entry.captured_length_units);
            released.push(Release {
                acquisition: entry,
                cause: ReleaseCause::LengthLimit,
            });
        }
        self.length_units = total as u64;
        if self.entries.len() >= COUNT_TRIGGER {
            for _ in 0..COUNT_TRIGGER - COUNT_AFTER_EVICTION {
                let entry = self
                    .entries
                    .pop_front()
                    .expect("count eviction has an owner");
                self.length_units -= entry.captured_length_units;
                released.push(Release {
                    acquisition: entry,
                    cause: ReleaseCause::CountLimit,
                });
            }
        }
        Ok(self.decision(Some(id), released))
    }
    pub fn expire_due(&mut self, customer_epoch: u64, now: Instant) -> Result<Decision, Error> {
        self.check_customer(customer_epoch)?;
        Ok(self.remove_matching(ReleaseCause::Expired, |entry| entry.expires_at <= now))
    }
    /// A canceled/stale timer is harmless, even after a new customer/model epoch.
    pub fn expire_acquisition(&mut self, id: AcquisitionId, now: Instant) -> Decision {
        self.remove_matching(ReleaseCause::Expired, |entry| {
            entry.id == id && entry.expires_at <= now
        })
    }
    /// Invoke for completed working-copy MOVE sources, not watcher/internal
    /// overwrite deletion. Destination references are deliberately unaffected.
    pub fn remove_move_source(
        &mut self,
        customer_epoch: u64,
        source: &ResourceKey,
    ) -> Result<Decision, Error> {
        self.check_customer(customer_epoch)?;
        Ok(self.remove_matching(ReleaseCause::MoveSource, |entry| {
            entry.resource.equal_or_descendant_of(source)
        }))
    }
    pub fn remove_deleted_resource(
        &mut self,
        customer_epoch: u64,
        target: &ResourceKey,
    ) -> Result<Decision, Error> {
        self.check_customer(customer_epoch)?;
        Ok(
            self.remove_matching(ReleaseCause::DeletedResource, |entry| {
                entry.resource.equal_or_descendant_of(target)
            }),
        )
    }
    pub fn untitled_revert(
        &mut self,
        customer_epoch: u64,
        model: ModelKey,
        resource: &ResourceKey,
    ) -> Result<Decision, Error> {
        self.check_customer(customer_epoch)?;
        if resource.identity.scheme != "untitled" {
            return Err(Error::NotUntitled);
        }
        Ok(self.remove_matching(ReleaseCause::UntitledRevert, |entry| {
            entry.model == model && entry.resource.same_resource(resource)
        }))
    }
    pub fn retire_customer(&mut self, customer_epoch: u64) -> Result<Decision, Error> {
        if customer_epoch != self.customer_epoch {
            return Err(Error::StaleCustomer);
        }
        self.retired = true;
        // Own the complete snapshot; never splice the collection being iterated.
        let snapshot = std::mem::take(&mut self.entries);
        self.length_units = 0;
        Ok(self.decision(
            None,
            snapshot
                .into_iter()
                .map(|acquisition| Release {
                    acquisition,
                    cause: ReleaseCause::CustomerRetired,
                })
                .collect(),
        ))
    }
    pub fn replace_customer(
        &mut self,
        expected_epoch: u64,
        next_epoch: u64,
    ) -> Result<Decision, Error> {
        if expected_epoch != self.customer_epoch {
            return Err(Error::StaleCustomer);
        }
        if next_epoch <= expected_epoch {
            return Err(Error::InvalidCustomer);
        }
        let decision = self.retire_customer(expected_epoch)?;
        self.customer_epoch = next_epoch;
        self.retired = false;
        // The sequence never resets across customer replacements.
        Ok(decision)
    }
    fn check_customer(&self, customer_epoch: u64) -> Result<(), Error> {
        if customer_epoch != self.customer_epoch {
            Err(Error::StaleCustomer)
        } else if self.retired {
            Err(Error::RetiredCustomer)
        } else {
            Ok(())
        }
    }
    fn remove_matching(
        &mut self,
        cause: ReleaseCause,
        matches: impl Fn(&Acquisition) -> bool,
    ) -> Decision {
        let mut released = Vec::new();
        let mut retained = VecDeque::new();
        for entry in std::mem::take(&mut self.entries) {
            if matches(&entry) {
                self.length_units -= entry.captured_length_units;
                released.push(Release {
                    acquisition: entry,
                    cause,
                });
            } else {
                retained.push_back(entry);
            }
        }
        self.entries = retained;
        self.decision(None, released)
    }
    fn decision(&self, acquired: Option<AcquisitionId>, released: Vec<Release>) -> Decision {
        let mut retained: Vec<Retention> = Vec::new();
        for entry in &self.entries {
            if let Some(model) = retained.iter_mut().find(|model| model.model == entry.model) {
                model.acquisitions += 1;
                model.captured_length_units += entry.captured_length_units;
            } else {
                retained.push(Retention {
                    model: entry.model,
                    acquisitions: 1,
                    captured_length_units: entry.captured_length_units,
                });
            }
        }
        Decision {
            acquired,
            released,
            retained,
        }
    }
}

#[cfg(test)]
#[path = "document_references/tests.rs"]
mod tests;
