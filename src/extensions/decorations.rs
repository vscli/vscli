//! Native-owned decoration type identity and irreversible retirement.
use super::*;
use crate::decorations::DecorationType;
use anyhow::ensure;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub(super) struct Registry {
    entries: HashMap<String, Arc<DecorationType>>,
    seen: HashSet<String>,
    bytes: usize,
    sizes: HashMap<String, usize>,
    generation: u64,
}
impl Registry {
    pub(super) fn get(&self, key: &str) -> Option<Arc<DecorationType>> {
        self.entries
            .get(key)
            .filter(|kind| kind.live.load(Ordering::Acquire))
            .cloned()
    }
    fn register(&mut self, key: String, owner: String, options: Value) -> Result<()> {
        ensure!(
            !key.is_empty()
                && key.len() <= 128
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "Invalid decoration type key"
        );
        ensure!(
            !self.seen.contains(&key),
            "Decoration type key has already been used"
        );
        ensure!(
            self.entries.len() < 256 && self.seen.len() < 4096,
            "Decoration type registry limit exceeded"
        );
        crate::decorations::json_budget(&options, 128 * 1024)?;
        let bytes = serde_json::to_vec(&options)?.len();
        ensure!(
            bytes <= 16 * 1024 && self.bytes.saturating_add(bytes) <= 1024 * 1024,
            "Decoration type options budget exceeded"
        );
        let generation = self
            .generation
            .checked_add(1)
            .context("Decoration generation exhausted")?;
        let kind = DecorationType::new(key.clone(), owner, options)?;
        self.seen.insert(key.clone());
        self.sizes.insert(key.clone(), bytes);
        self.entries.insert(key, kind);
        self.bytes += bytes;
        self.generation = generation;
        Ok(())
    }
    fn dispose(&mut self, key: &str, owner: &str) -> Result<()> {
        if let Some(kind) = self.entries.get(key) {
            ensure!(
                kind.owner == owner,
                "Decoration type belongs to another owner"
            );
            kind.live.store(false, Ordering::Release);
            self.entries.remove(key);
            self.bytes = self
                .bytes
                .saturating_sub(self.sizes.remove(key).unwrap_or(0));
            self.generation = self.generation.saturating_add(1);
        }
        Ok(())
    }
    pub(super) fn remove_owner(&mut self, owner: &str) {
        let keys = self
            .entries
            .iter()
            .filter(|(_, kind)| kind.owner == owner)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            let _ = self.dispose(&key, owner);
        }
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        for kind in self.entries.values() {
            kind.live.store(false, Ordering::Release);
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Update {
    session: u64,
    owner: String,
    key: String,
    action: String,
    options: Option<Value>,
    command: Option<u64>,
    command_owner: Option<String>,
}
impl Client {
    pub(crate) fn decorations_generation(&self) -> u64 {
        self.decorations.generation
    }
    pub(super) fn decoration_update(&mut self, params: Value) -> Result<()> {
        crate::decorations::json_budget(&params, 128 * 1024)?;
        ensure!(
            serde_json::to_vec(&params)?.len() <= 20 * 1024,
            "Decoration type notification budget exceeded"
        );
        let update: Update = serde_json::from_value(params)?;
        self.validate_native_origin(
            update.session,
            &update.owner,
            update.command,
            update.command_owner.as_deref(),
        )?;
        self.owner_lifetimes.capture(&update.owner)?;
        ensure!(
            self.editor_inventory_explicit(),
            "Native decoration inventory is unavailable"
        );
        match update.action.as_str() {
            "create" => self.decorations.register(
                update.key,
                update.owner,
                update.options.context("Decoration options missing")?,
            ),
            "dispose" => {
                ensure!(
                    update.options.is_none(),
                    "Dispose cannot change decoration options"
                );
                self.decorations.dispose(&update.key, &update.owner)
            }
            _ => Err(anyhow::anyhow!("Unknown decoration type operation")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_registry_identity_retires_on_disposal_owner_failure_and_host_drop() {
        let mut registry = Registry::default();
        registry
            .register(
                "decoration-1".into(),
                "owner.a".into(),
                json!({"color":"#123456"}),
            )
            .unwrap();
        let original = registry.get("decoration-1").unwrap();
        assert!(registry.dispose("decoration-1", "owner.b").is_err());
        assert!(Arc::ptr_eq(
            &original,
            &registry.get("decoration-1").unwrap()
        ));
        registry.dispose("decoration-1", "owner.a").unwrap();
        assert!(!original.live.load(Ordering::Acquire));
        assert!(registry.get("decoration-1").is_none());
        assert!(
            registry
                .register("decoration-1".into(), "owner.a".into(), json!({}))
                .is_err()
        );
        registry
            .register("decoration-2".into(), "owner.a".into(), json!({}))
            .unwrap();
        let failed = registry.get("decoration-2").unwrap();
        registry.remove_owner("owner.a");
        assert!(!failed.live.load(Ordering::Acquire));
        registry
            .register("decoration-3".into(), "owner.b".into(), json!({}))
            .unwrap();
        let stopped = registry.get("decoration-3").unwrap();
        drop(registry);
        assert!(!stopped.live.load(Ordering::Acquire));
    }
    #[test]
    fn registry_refuses_invalid_or_oversized_type_before_publication() {
        let mut registry = Registry::default();
        for (key, options) in [
            ("bad/key", json!({})),
            ("decoration-1", json!({"color":12})),
            (
                "decoration-2",
                json!({"before":{"contentText":"x".repeat(20 * 1024)}}),
            ),
        ] {
            assert!(
                registry
                    .register(key.into(), "owner.a".into(), options)
                    .is_err()
            );
            assert!(registry.entries.is_empty());
            assert!(registry.seen.is_empty());
            assert_eq!((registry.bytes, registry.generation), (0, 0));
        }
        for i in 0..256 {
            registry
                .register(format!("decoration-{i}"), "owner.a".into(), json!({}))
                .unwrap();
        }
        let before = registry.generation;
        assert!(
            registry
                .register("decoration-overflow".into(), "owner.a".into(), json!({}))
                .is_err()
        );
        assert_eq!(registry.generation, before);
        assert_eq!(registry.entries.len(), 256);
    }
}
