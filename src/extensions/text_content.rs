//! Native receipt authority for bounded provider-owned, read-only resources.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PROVIDERS: usize = 128;
const MAX_SAFE: u64 = (1 << 53) - 1;
const MAX_CONTENT: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct Registry {
    generation: u64,
    highwater: u64,
    providers: Vec<Arc<Registration>>,
}
struct Registration {
    scheme: String,
    owner: String,
    handle: u64,
    retired: Arc<AtomicBool>,
    owner_retired: Arc<AtomicBool>,
}

pub(crate) struct Capture {
    pub(crate) uri: String,
    pub(crate) content: String,
    pub(crate) document: Option<u64>,
    session: u64,
    provider: Arc<Registration>,
    source: Option<Source>,
}
struct Source {
    revision: u64,
    text_epoch: u64,
    version: u64,
}
struct Retained<'a> {
    documents: &'a [Document],
    hidden: &'a [Document],
    mirrors: &'a HashMap<u64, Mirror>,
}
impl Retained<'_> {
    fn document(&self, id: u64) -> Result<&Document> {
        let mut matching = self
            .documents
            .iter()
            .chain(self.hidden)
            .filter(|d| d.id == id);
        let document = matching
            .next()
            .context("Virtual change document is not retained")?;
        if matching.next().is_some() {
            bail!("Duplicate virtual document identity");
        }
        Ok(document)
    }
    fn source(&self, id: u64, uri: &str, version: u64) -> Result<Source> {
        let document = self.document(id)?;
        let mirror = self
            .mirrors
            .get(&id)
            .context("Virtual document is not mirrored")?;
        if !document.is_read_only()
            || document.path.is_some()
            || document.virtual_uri() != Some(uri)
            || mirror.uri != uri
            || mirror.version != version
            || mirror.revision != document.revision
            || mirror.text_epoch != document.text_epoch()
            || mirror.eol != document.eol
        {
            bail!("Virtual change source changed before publication");
        }
        Ok(Source {
            revision: document.revision,
            text_epoch: document.text_epoch(),
            version,
        })
    }
}
fn fields<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    let object = value.as_object().context("Invalid text content object")?;
    if object.len() != allowed.len() || object.keys().any(|key| !allowed.contains(&key.as_str())) {
        bail!("Invalid text content object fields");
    }
    Ok(object)
}
fn integer(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .filter(|n| (1..=MAX_SAFE).contains(n))
        .context("Invalid text content safe integer")
}
fn text(value: &Value, maximum: usize) -> Result<&str> {
    value
        .as_str()
        .filter(|s| s.len() <= maximum && !s.contains('\0'))
        .context("Invalid text content string")
}
fn scheme(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphabetic()
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c))
        || value.eq_ignore_ascii_case("file")
        || value.eq_ignore_ascii_case("untitled")
    {
        bail!("Text content requires a bounded custom URI scheme");
    }
    Ok(())
}
fn uri_scheme(uri: &str) -> Result<&str> {
    let value = uri
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .context("Invalid virtual document URI")?;
    scheme(value)?;
    Ok(value)
}
impl Registry {
    fn register(
        &mut self,
        session: u64,
        value: &Value,
        owners: &owner_lifetimes::OwnerLifetimes,
    ) -> Result<()> {
        let update = fields(value, &["session", "generation", "providers"])?;
        if integer(&update["session"])? != session {
            bail!("Text content registry session changed");
        }
        let generation = integer(&update["generation"])?;
        if generation <= self.generation {
            bail!("Text content registry generation must increase");
        }
        let providers = update["providers"]
            .as_array()
            .filter(|p| p.len() <= MAX_PROVIDERS)
            .context("Text content provider limit exceeded")?;
        let mut next = Vec::with_capacity(providers.len());
        let mut handles = HashSet::with_capacity(providers.len());
        let mut highwater = self.highwater;
        for item in providers {
            let object = fields(item, &["scheme", "owner", "handle"])?;
            let scheme_name = text(&object["scheme"], 128)?;
            scheme(scheme_name)?;
            let owner = text(&object["owner"], 201)?;
            let owner_retired = owners.capture(owner)?;
            let handle = integer(&object["handle"])?;
            if !handles.insert(handle) {
                bail!("Duplicate text content provider handle");
            }
            if let Some(existing) = self.providers.iter().find(|p| p.handle == handle) {
                if existing.scheme != scheme_name
                    || existing.owner != owner
                    || !Arc::ptr_eq(&existing.owner_retired, &owner_retired)
                    || existing.retired.load(Ordering::Acquire)
                {
                    bail!("Text content provider handle changed authority");
                }
                next.push(existing.clone());
            } else {
                if handle <= self.highwater {
                    bail!("Text content provider handle cannot be reused");
                }
                highwater = highwater.max(handle);
                next.push(Arc::new(Registration {
                    scheme: scheme_name.into(),
                    owner: owner.into(),
                    handle,
                    retired: Arc::new(AtomicBool::new(false)),
                    owner_retired,
                }));
            }
        }
        // No previously captured authorization retires until the WHOLE update validates.
        for previous in &self.providers {
            if !handles.contains(&previous.handle) {
                previous.retired.store(true, Ordering::Release);
            }
        }
        self.providers = next;
        self.generation = generation;
        self.highwater = highwater;
        Ok(())
    }
    fn provider(
        &self,
        owner: &str,
        handle: u64,
        scheme: &str,
        owners: &owner_lifetimes::OwnerLifetimes,
    ) -> Result<Arc<Registration>> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.handle == handle && p.owner == owner && p.scheme == scheme)
            .context("Text content provider registration is no longer current")?;
        self.provider_current(provider, owners)?;
        Ok(provider.clone())
    }
    fn provider_current(
        &self,
        provider: &Arc<Registration>,
        owners: &owner_lifetimes::OwnerLifetimes,
    ) -> Result<()> {
        if provider.retired.load(Ordering::Acquire)
            || provider.owner_retired.load(Ordering::Acquire)
            || !self.providers.iter().any(|p| Arc::ptr_eq(p, provider))
            || !Arc::ptr_eq(&owners.capture(&provider.owner)?, &provider.owner_retired)
        {
            bail!("Text content provider authorization retired");
        }
        Ok(())
    }
    fn capture(
        &self,
        session: u64,
        args: &Value,
        change: bool,
        retained: Retained<'_>,
        owners: &owner_lifetimes::OwnerLifetimes,
    ) -> Result<Arc<Capture>> {
        let object = fields(
            args,
            if change {
                &[
                    "uri",
                    "content",
                    "providerOwner",
                    "providerHandle",
                    "document",
                    "version",
                ]
            } else {
                &["uri", "content", "providerOwner", "providerHandle"]
            },
        )?;
        let uri = text(&object["uri"], 4096)?;
        let scheme = uri_scheme(uri)?;
        let content = text(&object["content"], MAX_CONTENT)?;
        let owner = text(&object["providerOwner"], 201)?;
        let handle = integer(&object["providerHandle"])?;
        let provider = self.provider(owner, handle, scheme, owners)?;
        let (document, source) = if change {
            let document = integer(&object["document"])?;
            let version = integer(&object["version"])?;
            (
                Some(document),
                Some(retained.source(document, uri, version)?),
            )
        } else {
            (None, None)
        };
        Ok(Arc::new(Capture {
            uri: uri.into(),
            content: content.into(),
            document,
            session,
            provider,
            source,
        }))
    }
    fn current(
        &self,
        capture: &Capture,
        session: u64,
        retained: Retained<'_>,
        owners: &owner_lifetimes::OwnerLifetimes,
    ) -> Result<()> {
        if capture.session != session {
            bail!("Text content receipt session changed");
        }
        self.provider_current(&capture.provider, owners)?;
        if let (Some(document), Some(source)) = (capture.document, capture.source.as_ref()) {
            let now = retained.source(document, &capture.uri, source.version)?;
            if now.revision != source.revision || now.text_epoch != source.text_epoch {
                bail!("Virtual document changed after native receipt");
            }
        }
        Ok(())
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        for provider in &self.providers {
            provider.retired.store(true, Ordering::Release);
        }
    }
}
impl Client {
    pub(super) fn register_text_content_providers(&mut self, value: Value) -> Result<()> {
        self.text_content_registry
            .register(self.session, &value, &self.owner_lifetimes)
    }
    pub(crate) fn capture_text_content(
        &self,
        args: &Value,
        change: bool,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<Arc<Capture>> {
        self.text_content_registry.capture(
            self.session,
            args,
            change,
            Retained {
                documents,
                hidden,
                mirrors: &self.mirror.mirrors,
            },
            &self.owner_lifetimes,
        )
    }
    pub(crate) fn text_content_provider_current(&self, capture: &Capture) -> Result<()> {
        if capture.session != self.session {
            bail!("Text content receipt session changed");
        }
        self.text_content_registry
            .provider_current(&capture.provider, &self.owner_lifetimes)
    }
    pub(crate) fn text_content_current(
        &self,
        capture: &Capture,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<()> {
        self.text_content_registry.current(
            capture,
            self.session,
            Retained {
                documents,
                hidden,
                mirrors: &self.mirror.mirrors,
            },
            &self.owner_lifetimes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owners() -> owner_lifetimes::OwnerLifetimes {
        owner_lifetimes::OwnerLifetimes::new(&["fixture.a", "fixture.b"].map(|id| Package {
            id: id.into(),
            version: "1".into(),
            path: PathBuf::from(id),
            sha256: None,
        }))
    }
    fn update(generation: u64, providers: Value) -> Value {
        json!({"session":1,"generation":generation,"providers":providers})
    }
    fn provider(handle: u64, owner: &str) -> Value {
        json!({"scheme":"preview-test","owner":owner,"handle":handle})
    }
    fn args(content: &str) -> Value {
        json!({"uri":"preview-test:/sample.cpp","content":content,"providerOwner":"fixture.a","providerHandle":1})
    }
    fn empty<'a>(mirrors: &'a HashMap<u64, Mirror>) -> Retained<'a> {
        Retained {
            documents: &[],
            hidden: &[],
            mirrors,
        }
    }
    fn hidden<'a>(documents: &'a [Document], mirrors: &'a HashMap<u64, Mirror>) -> Retained<'a> {
        Retained {
            documents: &[],
            hidden: documents,
            mirrors,
        }
    }
    #[test]
    fn whole_registry_validation_preserves_original_capture_until_real_disposal_and_rejects_aba() {
        let owners = owners();
        let mut registry = Registry::default();
        let mirrors = HashMap::new();
        registry
            .register(1, &update(1, json!([provider(1, "fixture.a")])), &owners)
            .unwrap();
        let first = registry
            .capture(1, &args("猫🙂"), false, empty(&mirrors), &owners)
            .unwrap();
        assert_eq!(first.content, "猫🙂");
        registry
            .register(
                1,
                &update(
                    2,
                    json!([provider(1, "fixture.a"), provider(2, "fixture.b")]),
                ),
                &owners,
            )
            .unwrap();
        registry
            .current(&first, 1, empty(&mirrors), &owners)
            .unwrap();
        let highwater = registry.highwater;
        assert!(
            registry
                .register(
                    1,
                    &update(
                        3,
                        json!([provider(3, "fixture.a"), provider(3, "fixture.b")])
                    ),
                    &owners
                )
                .is_err()
        );
        assert_eq!(registry.generation, 2);
        assert_eq!(registry.highwater, highwater);
        registry
            .current(&first, 1, empty(&mirrors), &owners)
            .unwrap();
        assert!(
            registry
                .register(1, &update(3, json!([provider(1, "fixture.b")])), &owners)
                .is_err()
        );
        registry
            .current(&first, 1, empty(&mirrors), &owners)
            .unwrap();
        registry
            .register(1, &update(3, json!([provider(2, "fixture.b")])), &owners)
            .unwrap();
        assert!(
            registry
                .current(&first, 1, empty(&mirrors), &owners)
                .is_err()
        );
        assert!(
            registry
                .register(1, &update(4, json!([provider(1, "fixture.a")])), &owners)
                .is_err()
        );
        registry
            .register(1, &update(4, json!([provider(4, "fixture.a")])), &owners)
            .unwrap();
        assert!(
            registry
                .current(&first, 1, empty(&mirrors), &owners)
                .is_err()
        );
    }
    #[test]
    fn owner_retirement_and_reactivation_never_reauthorize_received_virtual_content() {
        let mut owners = owners();
        let mut registry = Registry::default();
        let mirrors = HashMap::new();
        registry
            .register(1, &update(1, json!([provider(1, "fixture.a")])), &owners)
            .unwrap();
        let first = registry
            .capture(1, &args("old"), false, empty(&mirrors), &owners)
            .unwrap();
        owners.publish(&[("fixture.a".into(), "disposed".into())].into());
        assert!(
            registry
                .current(&first, 1, empty(&mirrors), &owners)
                .is_err()
        );
        owners.publish(&[("fixture.a".into(), "active".into())].into());
        assert!(
            registry
                .register(1, &update(2, json!([provider(1, "fixture.a")])), &owners)
                .is_err()
        );
        registry
            .register(1, &update(2, json!([provider(2, "fixture.a")])), &owners)
            .unwrap();
        assert!(
            registry
                .current(&first, 1, empty(&mirrors), &owners)
                .is_err()
        );
    }
    #[test]
    fn change_receipt_proves_original_readonly_hidden_model_version_epoch_and_uri() {
        let owners = owners();
        let mut registry = Registry::default();
        registry
            .register(1, &update(1, json!([provider(1, "fixture.a")])), &owners)
            .unwrap();
        let mut docs =
            [Document::from_virtual("preview-test:/sample.cpp".into(), "猫🙂\r\n").unwrap()];
        let id = docs[0].id;
        let mut mirrors = HashMap::new();
        mirrors.insert(
            id,
            Mirror {
                revision: docs[0].revision,
                text_epoch: docs[0].text_epoch(),
                language_epoch: docs[0].language_epoch(),
                editor_view: docs[0].editor_view_identity(),
                save_generation: 0,
                uri: "preview-test:/sample.cpp".into(),
                eol: docs[0].eol.clone(),
                version: 1,
            },
        );
        let mut raw = args("latest");
        raw["document"] = id.into();
        raw["version"] = 1.into();
        let first = registry
            .capture(1, &raw, true, hidden(&docs, &mirrors), &owners)
            .unwrap();
        assert_eq!(first.document, Some(id));
        registry
            .current(&first, 1, hidden(&docs, &mirrors), &owners)
            .unwrap();
        raw["version"] = 2.into();
        assert!(
            registry
                .capture(1, &raw, true, hidden(&docs, &mirrors), &owners)
                .is_err()
        );
        raw["version"] = 1.into();
        raw["uri"] = "preview-test:/other.cpp".into();
        assert!(
            registry
                .capture(1, &raw, true, hidden(&docs, &mirrors), &owners)
                .is_err()
        );
        docs[0].refresh_virtual_content("new content").unwrap();
        assert!(
            registry
                .current(&first, 1, hidden(&docs, &mirrors), &owners)
                .is_err()
        );
        assert_eq!(docs[0].id, id);
        assert_eq!(docs[0].text.to_string(), "new content");
        assert!(
            registry
                .current(&first, 1, empty(&mirrors), &owners)
                .is_err()
        );
    }
    #[test]
    fn strict_budgets_session_integer_and_unknown_fields_refuse_without_retiring_valid_provider() {
        let owners = owners();
        let mut registry = Registry::default();
        let mirrors = HashMap::new();
        registry
            .register(1, &update(1, json!([provider(1, "fixture.a")])), &owners)
            .unwrap();
        let first = registry
            .capture(1, &args("original"), false, empty(&mirrors), &owners)
            .unwrap();
        for bad in [
            update(1, json!([])),
            update(MAX_SAFE + 1, json!([])),
            json!({"session":2,"generation":2,"providers":[]}),
            update(2, json!([{"scheme":"file","owner":"fixture.a","handle":2}])),
            update(
                2,
                json!([{"scheme":"preview-test","owner":"unselected","handle":2}]),
            ),
            update(
                2,
                Value::Array(
                    (2..=130)
                        .map(|handle| provider(handle, "fixture.a"))
                        .collect(),
                ),
            ),
            json!({"session":1,"generation":2,"providers":[],"unexpected":true}),
        ] {
            assert!(registry.register(1, &bad, &owners).is_err());
            registry
                .current(&first, 1, empty(&mirrors), &owners)
                .unwrap();
        }
        for (key, value) in [
            ("content", Value::String("x".repeat(MAX_CONTENT + 1))),
            ("content", Value::String("bad\0".into())),
            ("uri", Value::String("file:/sample.cpp".into())),
            (
                "uri",
                Value::String(format!("preview-test:{}", "x".repeat(4096))),
            ),
            ("providerHandle", json!(MAX_SAFE + 1)),
            ("providerOwner", json!("fixture.b")),
            ("extra", json!(true)),
        ] {
            let mut raw = args("original");
            raw[key] = value;
            assert!(
                registry
                    .capture(1, &raw, false, empty(&mirrors), &owners)
                    .is_err()
            );
        }
        assert!(
            registry
                .current(&first, 2, empty(&mirrors), &owners)
                .is_err()
        );
        registry
            .current(&first, 1, empty(&mirrors), &owners)
            .unwrap();
    }
    #[test]
    fn registry_drop_irreversibly_retires_capture_while_another_session_keeps_its_own_provider() {
        let owners = owners();
        let mirrors = HashMap::new();
        let mut registry = Registry::default();
        registry
            .register(1, &update(1, json!([provider(1, "fixture.a")])), &owners)
            .unwrap();
        let first = registry
            .capture(1, &args("original"), false, empty(&mirrors), &owners)
            .unwrap();
        let mut other = Registry::default();
        other
            .register(1, &update(1, json!([provider(1, "fixture.a")])), &owners)
            .unwrap();
        drop(registry);
        assert!(first.provider.retired.load(Ordering::Acquire));
        assert!(other.current(&first, 1, empty(&mirrors), &owners).is_err());
        let live = other
            .capture(1, &args("other"), false, empty(&mirrors), &owners)
            .unwrap();
        other.current(&live, 1, empty(&mirrors), &owners).unwrap();
    }
}
