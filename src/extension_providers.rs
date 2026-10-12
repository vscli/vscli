//! Bounded owner-scoped metadata for optional extension language providers.
//! This registry does not invoke JavaScript or mutate native documents.
use crate::{document::Document, lsp};
use anyhow::{Result, bail};
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

pub const MAX_PROVIDERS: usize = 128;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    CodeAction,
    Completion,
    Hover,
    Definition,
    Implementation,
    #[serde(rename = "typeDefinition")]
    TypeDefinition,
    Declaration,
    References,
    Formatting,
    Symbols,
    #[serde(rename = "workspaceSymbols")]
    WorkspaceSymbols,
    Signature,
}
impl Kind {
    pub fn is_navigation(self) -> bool {
        matches!(
            self,
            Self::Definition
                | Self::Implementation
                | Self::TypeDefinition
                | Self::Declaration
                | Self::References
        )
    }
    pub fn method(self) -> &'static str {
        match self {
            Self::CodeAction => "textDocument/codeAction",
            Self::Completion => "textDocument/completion",
            Self::Hover => "textDocument/hover",
            Self::Definition => "textDocument/definition",
            Self::Implementation => "textDocument/implementation",
            Self::TypeDefinition => "textDocument/typeDefinition",
            Self::Declaration => "textDocument/declaration",
            Self::References => "textDocument/references",
            Self::Formatting => "textDocument/formatting",
            Self::Symbols => "textDocument/documentSymbol",
            Self::WorkspaceSymbols => "workspace/symbol",
            Self::Signature => "textDocument/signatureHelp",
        }
    }
    pub fn from_method(method: &str) -> Option<Self> {
        [
            Self::CodeAction,
            Self::Completion,
            Self::Hover,
            Self::Definition,
            Self::Implementation,
            Self::TypeDefinition,
            Self::Declaration,
            Self::References,
            Self::Formatting,
            Self::Symbols,
            Self::WorkspaceSymbols,
            Self::Signature,
        ]
        .into_iter()
        .find(|kind| kind.method() == method)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub language: Option<String>,
    pub scheme: Option<String>,
    #[serde(default)]
    pub pattern: Option<Pattern>,
    #[serde(default)]
    pub exclusive: bool,
    #[serde(skip)]
    matcher: Option<Arc<GlobMatcher>>,
    #[serde(skip)]
    base: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Pattern {
    Glob(String),
    Relative(RelativePattern),
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RelativePattern {
    pub base_uri: String,
    pub pattern: String,
}

impl Filter {
    fn score(&self, language: &str, scheme: &str, path: &str) -> u8 {
        let mut score = 0;
        for (expected, actual) in [(&self.language, language), (&self.scheme, scheme)] {
            match expected.as_deref().filter(|value| !value.is_empty()) {
                None => (),
                Some("*") => score = score.max(5),
                Some(value) if value == actual => score = 10,
                Some(_) => return 0,
            }
        }
        if let Some(matcher) = &self.matcher {
            let path = if let Some(base) = &self.base {
                let Some(prefix) = path.get(..base.len()) else {
                    return 0;
                };
                let equal = if cfg!(target_os = "linux") {
                    prefix == base
                } else {
                    prefix.to_lowercase() == base.to_lowercase()
                };
                let Some(relative) = equal.then(|| &path[base.len()..]) else {
                    return 0;
                };
                if !relative.is_empty() && !relative.starts_with('/') {
                    return 0;
                }
                relative.trim_start_matches('/')
            } else {
                path
            };
            if !matcher.is_match(path) {
                return 0;
            }
            score = 10;
        }
        score
    }
    fn prepare(&mut self) -> Result<()> {
        let (pattern, base) = match &self.pattern {
            None => return Ok(()),
            Some(Pattern::Glob(pattern)) if pattern.is_empty() => return Ok(()),
            Some(Pattern::Glob(pattern)) => (pattern, None),
            Some(Pattern::Relative(relative)) => {
                if relative.base_uri.len() > 4096 || relative.base_uri.contains('\0') {
                    bail!("Provider pattern base exceeds bounds");
                }
                let base = uri_fs_path(&relative.base_uri)
                    .ok_or_else(|| anyhow::anyhow!("Invalid provider pattern base URI"))?;
                (&relative.pattern, Some(normalize_base(&base)))
            }
        };
        if pattern.len() > 4096 || pattern.contains('\0') {
            bail!("Provider pattern exceeds bounds");
        }
        let pattern = pattern.replace('\\', "/");
        let matcher = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .backslash_escape(false)
            .build()?
            .compile_matcher();
        self.matcher = Some(Arc::new(matcher));
        self.base = base;
        Ok(())
    }
    fn valid(&self) -> bool {
        [&self.language, &self.scheme]
            .into_iter()
            .flatten()
            .all(|value| value.len() <= 128)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: u64,
    pub owner: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub selector: Vec<Filter>,
    pub triggers: Vec<String>,
    #[serde(default)]
    pub retriggers: Vec<String>,
    #[serde(default)]
    pub resolves: bool,
    #[serde(default, rename = "actionKinds")]
    pub action_kinds: Option<Vec<String>>,
}
impl Provider {
    pub fn score(&self, document: &Document) -> u8 {
        let language = document
            .language_override()
            .unwrap_or_else(|| document.path.as_deref().map_or("plaintext", lsp::language));
        let scheme = document
            .virtual_uri()
            .or(document.untitled_uri())
            .and_then(|uri| uri.split_once(':').map(|(scheme, _)| scheme))
            .unwrap_or(if document.path.is_some() {
                "file"
            } else {
                "untitled"
            });
        let path = if let Some(path) = &document.path {
            if path.as_os_str().as_encoded_bytes().len() > 4096 {
                return 0;
            }
            path.to_string_lossy().replace('\\', "/")
        } else if let Some(uri) = document.virtual_uri().or(document.untitled_uri()) {
            let Some(path) = uri_fs_path(uri) else {
                return 0;
            };
            path
        } else {
            format!("vscli-{}", document.id)
        };
        self.selector
            .iter()
            .map(|filter| filter.score(language, scheme, &path))
            .max()
            .unwrap_or(0)
    }
}
fn uri_fs_path(uri: &str) -> Option<String> {
    if uri.len() > 4096 || uri.contains('\0') {
        return None;
    }
    let url = url::Url::parse(uri).ok()?;
    if url.scheme() == "file" {
        return Some(
            url.to_file_path()
                .ok()?
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    let mut bytes = Vec::with_capacity(url.path().len());
    let mut input = url.path().as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    Some(String::from_utf8(bytes).ok()?.replace('\\', "/"))
}
fn normalize_base(base: &str) -> String {
    let absolute = base.starts_with('/');
    let mut parts = Vec::new();
    for part in base.split('/') {
        match part {
            "" | "." => (),
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("{}{}", if absolute { "/" } else { "" }, parts.join("/"))
        .trim_end_matches('/')
        .to_owned()
}
#[derive(Default)]
pub struct Registry {
    providers: Vec<Provider>,
    epoch: u64,
}
impl Registry {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn entries(&self) -> &[Provider] {
        &self.providers
    }
    /// Stage and validate the whole update; invalid metadata preserves prior entries.
    pub fn replace(&mut self, value: serde_json::Value, owners: &[&str]) -> Result<()> {
        let entries = value
            .as_array()
            .filter(|entries| entries.len() <= MAX_PROVIDERS)
            .ok_or_else(|| {
                anyhow::anyhow!("Extension language provider metadata exceeds its budget")
            })?;
        let mut bytes = 2usize;
        for entry in entries {
            bytes = bytes
                .checked_add(
                    crate::decorations::json_budget(entry, 2 * 1024 * 1024).map_err(|error| {
                        anyhow::anyhow!("Provider registration exceeds bounds: {error}")
                    })?,
                )
                .ok_or_else(|| anyhow::anyhow!("Provider registration byte overflow"))?;
            if bytes > 2 * 1024 * 1024 {
                bail!("Extension language provider metadata exceeds its budget");
            }
        }
        let mut next: Vec<Provider> = serde_json::from_value(value)?;
        let mut ids = HashSet::new();
        for provider in &next {
            if provider.id == 0
                || provider.id > 9_007_199_254_740_991
                || !ids.insert(provider.id)
                || !owners.contains(&provider.owner.as_str())
                || (provider.resolves
                    && !matches!(
                        provider.kind,
                        Kind::Completion | Kind::CodeAction | Kind::WorkspaceSymbols
                    ))
                || (provider.kind == Kind::WorkspaceSymbols && !provider.selector.is_empty())
                || (provider.kind != Kind::CodeAction && provider.action_kinds.is_some())
                || (provider.kind != Kind::Signature && !provider.retriggers.is_empty())
                || provider.action_kinds.as_ref().is_some_and(|kinds| {
                    kinds.len() > 32 || kinds.iter().any(|kind| kind.len() > 128)
                })
                || provider.selector.len() > 32
                || !provider.selector.iter().all(Filter::valid)
                || provider.triggers.len() > 16
                || provider.retriggers.len() > 16
                || provider
                    .triggers
                    .iter()
                    .chain(&provider.retriggers)
                    .any(|value| value.chars().count() != 1 || value.len() > 4)
            {
                bail!("Invalid extension language provider registration");
            }
        }
        for provider in &mut next {
            for filter in &mut provider.selector {
                filter.prepare()?;
            }
        }
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Provider registry identities exhausted"))?;
        self.providers = next;
        self.epoch = epoch;
        Ok(())
    }
    /// Prefer exact filters, then the most recently registered matching provider.
    pub fn selected(&self, kind: Kind, document: &Document) -> Option<&Provider> {
        self.providers
            .iter()
            .filter(|provider| provider.kind == kind)
            .filter_map(|provider| {
                let score = provider.score(document);
                (score > 0).then_some((score, provider.id, provider))
            })
            .max_by_key(|(score, id, _)| (*score, *id))
            .map(|(_, _, provider)| provider)
    }
    pub fn current(&self, provider: &Provider, epoch: u64) -> bool {
        self.epoch == epoch
            && self.providers.iter().any(|entry| {
                entry.id == provider.id
                    && entry.owner == provider.owner
                    && entry.kind == provider.kind
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn registration(id: u64, language: &str) -> serde_json::Value {
        json!({"id":id,"owner":"test.provider","type":"completion","selector":[{"language":language,"scheme":"file"}],"triggers":["."]})
    }
    #[test]
    fn effective_language_virtual_scheme_and_full_path_patterns_match_without_io() {
        let mut registry = Registry::default();
        let owners = ["test.provider"];
        let entries = json!([
            {"id":1,"owner":"test.provider","type":"implementation","selector":[{"language":"cpp","scheme":"custom","pattern":"**/*.cpp"}],"triggers":[]},
            {"id":2,"owner":"test.provider","type":"typeDefinition","selector":[{"language":"python","scheme":"custom","pattern":{"baseUri":"custom:/work/./src","pattern":"**/*.cpp"}}],"triggers":[]},
            {"id":3,"owner":"test.provider","type":"declaration","selector":[{"language":"","scheme":"","pattern":"**/*.cpp","exclusive":true}],"triggers":[]}
        ]);
        registry.replace(entries, &owners).unwrap();
        let mut doc = Document::from_virtual("custom:/work/src/猫.cpp".into(), "α🙂\r\n").unwrap();
        doc.set_language_override(Some("cpp".into())).unwrap();
        assert_eq!(registry.entries()[0].score(&doc), 10);
        assert_eq!(registry.entries()[1].score(&doc), 0);
        assert!(registry.entries()[2].selector[0].exclusive);
        let text_epoch = doc.text_epoch();
        let saved = doc.save_generation();
        doc.set_language_override(Some("python".into())).unwrap();
        assert_eq!(registry.entries()[0].score(&doc), 0);
        assert_eq!(registry.entries()[1].score(&doc), 10);
        let mut outside =
            Document::from_virtual("custom:/work/srcOther/猫.cpp".into(), "α🙂\r\n").unwrap();
        outside
            .set_language_override(Some("python".into()))
            .unwrap();
        assert_eq!(registry.entries()[1].score(&outside), 0);
        assert_eq!(
            (doc.text_epoch(), doc.save_generation()),
            (text_epoch, saved)
        );
        assert_eq!(doc.text.to_string(), "α🙂\n");
        assert!(doc.is_read_only());
    }
    #[test]
    fn relative_pattern_equal_base_matches_empty_suffix_and_preserves_exclusive_metadata() {
        let mut registry = Registry::default();
        let entries = json!([
            {"id":1,"owner":"test.provider","type":"definition","selector":[{"pattern":{"baseUri":"custom:/work/猫.cpp","pattern":"*"}}],"triggers":[]},
            {"id":2,"owner":"test.provider","type":"definition","selector":[{"language":"cpp","exclusive":true},{"scheme":"custom","exclusive":false}],"triggers":[]},
            {"id":3,"owner":"test.provider","type":"definition","selector":[{"language":"cpp","exclusive":true},{"scheme":"custom","exclusive":true}],"triggers":[]}
        ]);
        registry.replace(entries, &["test.provider"]).unwrap();
        let mut doc = Document::from_virtual("custom:/work/猫.cpp".into(), "猫🙂\r\n").unwrap();
        doc.set_language_override(Some("cpp".into())).unwrap();
        assert_eq!(
            registry.entries()[0].score(&doc),
            10,
            "Equal base has an empty matching suffix"
        );
        assert_eq!(registry.entries()[1].score(&doc), 10);
        assert!(
            !registry.entries()[1]
                .selector
                .iter()
                .all(|filter| filter.exclusive),
            "Preserve the ordinary filter for host authority"
        );
        assert!(
            registry.entries()[2]
                .selector
                .iter()
                .all(|filter| filter.exclusive)
        );
        let other = Document::from_virtual("custom:/work/猫.cppOther".into(), "猫🙂").unwrap();
        assert_eq!(
            registry.entries()[0].score(&other),
            0,
            "A longer same-prefix sibling is outside base"
        );
        assert_eq!(doc.text.to_string(), "猫🙂\n");
        assert!(!doc.dirty());
    }
    #[test]
    fn late_invalid_selector_and_epoch_exhaustion_preserve_original_registration() {
        let mut registry = Registry::default();
        let owner = ["test.provider"];
        registry
            .replace(json!([registration(1, "cpp")]), &owner)
            .unwrap();
        let epoch = registry.epoch();
        for pattern in [
            json!("["),
            json!("x".repeat(4097)),
            json!({"baseUri":"bad uri","pattern":"*"}),
        ] {
            let mut invalid = registration(2, "cpp");
            invalid["selector"][0]["pattern"] = pattern;
            assert!(
                registry
                    .replace(json!([registration(3, "cpp"), invalid]), &owner)
                    .is_err()
            );
            assert_eq!(registry.epoch(), epoch);
            assert_eq!(registry.entries()[0].id, 1);
        }
        registry.epoch = u64::MAX;
        assert!(
            registry
                .replace(json!([registration(2, "cpp")]), &owner)
                .is_err()
        );
        assert_eq!(registry.epoch(), u64::MAX);
        assert_eq!(registry.entries()[0].id, 1);
        assert_eq!(
            Kind::from_method("textDocument/typeDefinition"),
            Some(Kind::TypeDefinition)
        );
        assert_eq!(
            serde_json::to_value(Kind::TypeDefinition).unwrap(),
            json!("typeDefinition")
        );
    }
    #[test]
    fn signature_metadata_preserves_distinct_triggers_and_rejects_atomic_invalid_updates() {
        let mut registry = Registry::default();
        let owners = ["test.provider"];
        let mut signature = registration(1, "cpp");
        signature["type"] = json!("signature");
        signature["triggers"] = json!(["("]);
        signature["retriggers"] = json!([",", ")"]);
        registry
            .replace(json!([signature.clone()]), &owners)
            .unwrap();
        assert_eq!(registry.entries()[0].triggers, ["("]);
        assert_eq!(registry.entries()[0].retriggers, [",", ")"]);
        let epoch = registry.epoch();
        for field in [json!(["ab"]), json!(vec![","; 17]), json!([0])] {
            let mut invalid = signature.clone();
            invalid["retriggers"] = field;
            assert!(registry.replace(json!([invalid]), &owners).is_err());
            assert_eq!(registry.epoch(), epoch);
        }
        signature["type"] = json!("completion");
        assert!(registry.replace(json!([signature]), &owners).is_err());
        assert_eq!(registry.epoch(), epoch);
    }
    #[test]
    fn metadata_updates_are_atomic_and_do_not_adopt_unknown_owners_or_selector_fields() {
        let mut registry = Registry::default();
        let owner = ["test.provider"];
        registry
            .replace(json!([registration(1, "cpp")]), &owner)
            .unwrap();
        let epoch = registry.epoch();
        let mut invalid = registration(2, "cpp");
        invalid["selector"][0]["unsupportedField"] = json!("**/*.cpp");
        let mut wrong_owner = registration(2, "cpp");
        wrong_owner["owner"] = json!("other.provider");
        for update in [
            json!([registration(1, "cpp"), registration(1, "cpp")]),
            json!([registration(0, "cpp")]),
            json!([invalid]),
            json!([wrong_owner]),
            json!([registration(2, &"x".repeat(129))]),
            json!(vec![registration(2, "cpp"); 129]),
        ] {
            assert!(registry.replace(update, &owner).is_err());
            assert_eq!(registry.entries()[0].id, 1);
            assert_eq!(registry.epoch(), epoch);
        }
    }
    #[test]
    fn selection_prefers_exact_languages_then_registration_order_without_mutating_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.cpp");
        std::fs::write(&path, "猫🙂\r\n").unwrap();
        let doc = Document::open(&path).unwrap();
        let mut registry = Registry::default();
        // The wildcard has no exact scheme, so its score is five rather than ten.
        let mut wildcard = registration(100, "*");
        wildcard["selector"][0]
            .as_object_mut()
            .unwrap()
            .remove("scheme");
        registry
            .replace(
                json!([registration(1, "cpp"), wildcard, registration(2, "cpp")]),
                &["test.provider"],
            )
            .unwrap();
        let selected = registry.selected(Kind::Completion, &doc).unwrap().clone();
        assert_eq!(selected.id, 2);
        assert!(registry.selected(Kind::Formatting, &doc).is_none());
        assert_eq!(doc.text.to_string(), "猫🙂\r\n");
        let epoch = registry.epoch();
        registry.replace(json!([]), &["test.provider"]).unwrap();
        assert!(!registry.current(&selected, epoch));
    }
    #[test]
    fn selectors_distinguish_untitled_from_file_and_reject_malformed_triggers() {
        let doc = Document::from_text("");
        let mut entry = registration(1, "*");
        entry["selector"][0]["scheme"] = json!("untitled");
        let mut registry = Registry::default();
        registry
            .replace(json!([entry.clone()]), &["test.provider"])
            .unwrap();
        assert_eq!(registry.selected(Kind::Completion, &doc).unwrap().id, 1);
        entry["triggers"] = json!(["ab"]);
        assert!(
            registry
                .replace(json!([entry]), &["test.provider"])
                .is_err()
        );
        for kind in [
            Kind::Completion,
            Kind::Hover,
            Kind::Definition,
            Kind::References,
            Kind::Formatting,
            Kind::Symbols,
            Kind::Signature,
        ] {
            assert_eq!(Kind::from_method(kind.method()), Some(kind));
        }
        assert_eq!(Kind::from_method("workspace/executeCommand"), None);
    }
}
