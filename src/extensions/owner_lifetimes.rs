//! Native-owned selected caller lifetimes, independent of command correlation.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct OwnerLifetimes {
    owners: HashMap<String, Arc<AtomicBool>>,
}

impl OwnerLifetimes {
    pub(super) fn new(packages: &[Package]) -> Self {
        Self {
            owners: packages
                .iter()
                .map(|package| (package.id.clone(), Arc::new(AtomicBool::new(false))))
                .collect(),
        }
    }

    pub(super) fn capture(&self, owner: &str) -> Result<Arc<AtomicBool>> {
        let lifetime = self
            .owners
            .get(owner)
            .context("Native caller is not selected")?;
        if lifetime.load(Ordering::Acquire) {
            bail!("Native caller lifetime has retired");
        }
        Ok(lifetime.clone())
    }

    // Only a completely validated native activation registry reaches this method.
    // A new activation can acquire a fresh lifetime but never renew an old one.
    pub(super) fn publish(&mut self, states: &std::collections::BTreeMap<String, String>) {
        for (owner, state) in states {
            let Some(lifetime) = self.owners.get_mut(owner) else {
                continue;
            };
            if matches!(state.as_str(), "failed" | "disposed") {
                lifetime.store(true, Ordering::Release);
            } else if matches!(state.as_str(), "activating" | "active")
                && lifetime.load(Ordering::Acquire)
            {
                *lifetime = Arc::new(AtomicBool::new(false));
            }
        }
    }
}

impl Drop for OwnerLifetimes {
    fn drop(&mut self) {
        for lifetime in self.owners.values() {
            lifetime.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packages() -> Vec<Package> {
        ["caller.a", "caller.b"]
            .into_iter()
            .map(|id| Package {
                id: id.into(),
                version: "1.0.0".into(),
                path: PathBuf::from(id),
                sha256: None,
            })
            .collect()
    }

    fn states(a: &str, b: &str) -> std::collections::BTreeMap<String, String> {
        [("caller.a".into(), a.into()), ("caller.b".into(), b.into())].into()
    }

    #[test]
    fn retirement_and_reactivation_never_renew_original_native_caller() {
        let mut registry = OwnerLifetimes::new(&packages());
        let first = registry.capture("caller.a").unwrap();
        let other = registry.capture("caller.b").unwrap();
        registry.publish(&states("active", "active"));
        assert!(Arc::ptr_eq(&first, &registry.capture("caller.a").unwrap()));
        registry.publish(&states("failed", "active"));
        assert!(first.load(Ordering::Acquire));
        assert!(!other.load(Ordering::Acquire));
        assert!(registry.capture("caller.a").is_err());
        registry.publish(&states("dormant", "active"));
        assert!(registry.capture("caller.a").is_err());
        registry.publish(&states("activating", "active"));
        let replacement = registry.capture("caller.a").unwrap();
        assert!(!replacement.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&first, &replacement));
        registry.publish(&states("active", "active"));
        assert!(Arc::ptr_eq(
            &replacement,
            &registry.capture("caller.a").unwrap()
        ));
        assert!(first.load(Ordering::Acquire));
        registry.publish(&states("disposed", "active"));
        assert!(replacement.load(Ordering::Acquire));
        assert!(!other.load(Ordering::Acquire));
    }

    #[test]
    fn dropping_registry_retires_every_selected_lifetime_and_session_isolation_holds() {
        let registry = OwnerLifetimes::new(&packages());
        let first = registry.capture("caller.a").unwrap();
        let other = registry.capture("caller.b").unwrap();
        let next = OwnerLifetimes::new(&packages());
        let replacement = next.capture("caller.a").unwrap();
        assert!(registry.capture("unselected.c").is_err());
        drop(registry);
        assert!(first.load(Ordering::Acquire));
        assert!(other.load(Ordering::Acquire));
        assert!(!replacement.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&first, &replacement));
    }
}
