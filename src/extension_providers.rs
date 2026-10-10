//! Bounded owner-scoped metadata for optional extension language providers.
//! This registry does not invoke JavaScript or mutate native documents.
use crate::{document::Document, lsp};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_PROVIDERS: usize = 128;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    CodeAction,
    Completion,
    Hover,
    Definition,
    References,
    Formatting,
    Symbols,
    Signature,
}
impl Kind {
    pub fn method(self) -> &'static str {
        match self {
            Self::CodeAction => "textDocument/codeAction",
            Self::Completion => "textDocument/completion",
            Self::Hover => "textDocument/hover",
            Self::Definition => "textDocument/definition",
            Self::References => "textDocument/references",
            Self::Formatting => "textDocument/formatting",
            Self::Symbols => "textDocument/documentSymbol",
            Self::Signature => "textDocument/signatureHelp",
        }
    }
    pub fn from_method(method: &str) -> Option<Self> {
        [
            Self::CodeAction,
            Self::Completion,
            Self::Hover,
            Self::Definition,
            Self::References,
            Self::Formatting,
            Self::Symbols,
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
}
impl Filter {
    fn score(&self, language: &str, scheme: &str) -> u8 {
        let mut score = 0;
        for (expected, actual) in [(&self.language, language), (&self.scheme, scheme)] {
            match expected.as_deref() {
                None => (),
                Some("*") => score = score.max(5),
                Some(value) if value == actual => score = 10,
                Some(_) => return 0,
            }
        }
        score
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
    pub resolves: bool,
    #[serde(default, rename = "actionKinds")]
    pub action_kinds: Option<Vec<String>>,
}
impl Provider {
    pub fn score(&self, document: &Document) -> u8 {
        let language = document.path.as_deref().map_or("plaintext", lsp::language);
        let scheme = if document.path.is_some() {
            "file"
        } else {
            "untitled"
        };
        self.selector
            .iter()
            .map(|filter| filter.score(language, scheme))
            .max()
            .unwrap_or(0)
    }
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
        if value
            .as_array()
            .is_none_or(|values| values.len() > MAX_PROVIDERS)
            || serde_json::to_vec(&value)?.len() > 2 * 1024 * 1024
        {
            bail!("Extension language provider metadata exceeds its budget");
        }
        let next: Vec<Provider> = serde_json::from_value(value)?;
        let mut ids = HashSet::new();
        for provider in &next {
            if provider.id == 0
                || provider.id > 9_007_199_254_740_991
                || !ids.insert(provider.id)
                || !owners.contains(&provider.owner.as_str())
                || (provider.resolves
                    && !matches!(provider.kind, Kind::Completion | Kind::CodeAction))
                || (provider.kind != Kind::CodeAction && provider.action_kinds.is_some())
                || provider.action_kinds.as_ref().is_some_and(|kinds| {
                    kinds.len() > 32 || kinds.iter().any(|kind| kind.len() > 128)
                })
                || provider.selector.len() > 32
                || !provider.selector.iter().all(Filter::valid)
                || provider.triggers.len() > 16
                || provider
                    .triggers
                    .iter()
                    .any(|value| value.chars().count() != 1 || value.len() > 4)
            {
                bail!("Invalid extension language provider registration");
            }
        }
        self.providers = next;
        self.epoch = self.epoch.wrapping_add(1);
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
    fn metadata_updates_are_atomic_and_do_not_adopt_unknown_owners_or_selector_fields() {
        let mut registry = Registry::default();
        let owner = ["test.provider"];
        registry
            .replace(json!([registration(1, "cpp")]), &owner)
            .unwrap();
        let epoch = registry.epoch();
        let mut invalid = registration(2, "cpp");
        invalid["selector"][0]["pattern"] = json!("**/*.cpp");
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
