//! Bounded native completion items and filtering, independent of any runtime.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{self, Write};

pub const MAX_ITEMS: usize = 300;
pub const MAX_TEXT: usize = 4096;
pub const MAX_PREFIX: usize = 1024;
pub const MAX_DOCUMENTATION: usize = 32 * 1024;
const MAX_ITEM_BYTES: usize = 64 * 1024;
const MAX_BYTES: usize = 2 * 1024 * 1024;

pub struct Item {
    pub label: String,
    pub detail: String,
    pub documentation: String,
    pub value: Value,
    pub resolved: bool,
    filter: String,
    sort: String,
    boundaries: Vec<bool>,
    bytes: usize,
}
pub struct Model {
    items: Vec<Item>,
    visible: Vec<usize>,
    selected: usize,
    deliberate_selection: bool,
    bytes: usize,
    pub incomplete: bool,
}
struct Budget(usize);
impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::other("Completion item exceeds its byte budget"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Model {
    pub fn parse(mut response: Value, prefix: &str) -> Result<Self> {
        if response
            .get("itemDefaults")
            .is_some_and(|defaults| !defaults.is_null())
        {
            bail!("Completion-list item defaults are not implemented");
        }
        let incomplete = response["isIncomplete"].as_bool().unwrap_or(false);
        let values = if response.is_array() {
            response.take()
        } else if response.is_object() {
            response["items"].take()
        } else {
            bail!("Invalid completion list");
        };
        let values = values.as_array().context("Invalid completion list")?;
        if values.len() > MAX_ITEMS {
            bail!("Completion list exceeds 300 entries");
        }
        let mut retained = 0;
        let mut items = Vec::with_capacity(values.len());
        for value in values {
            let item = parse_item(value.clone())?;
            retained += item.bytes;
            if retained > MAX_BYTES {
                bail!("Completion list exceeds 2 MiB");
            }
            items.push(item);
        }
        items.sort_by(|a, b| a.sort.cmp(&b.sort).then(a.label.cmp(&b.label)));
        let mut model = Self {
            items,
            visible: Vec::new(),
            selected: 0,
            deliberate_selection: false,
            bytes: retained,
            incomplete,
        };
        model.filter(prefix)?;
        if let Some(index) = model
            .visible
            .iter()
            .position(|index| model.items[*index].value["preselect"].as_bool() == Some(true))
        {
            model.selected = index;
        }
        Ok(model)
    }
    pub fn filter(&mut self, prefix: &str) -> Result<()> {
        if prefix.len() > MAX_PREFIX {
            bail!("Completion prefix exceeds 1 KiB");
        }
        let selected = self.visible.get(self.selected).copied();
        let prefix = prefix.to_lowercase();
        let mut ranked: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match_score(item, &prefix).map(|score| (score, index)))
            .collect();
        ranked.sort_by_key(|&(score, index)| (score, index));
        self.visible = ranked.into_iter().map(|(_, index)| index).collect();
        self.selected = selected
            .filter(|_| self.deliberate_selection)
            .and_then(|selected| self.visible.iter().position(|index| *index == selected))
            .unwrap_or(0);
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.visible.len()
    }
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn item(&self, index: usize) -> Option<&Item> {
        self.visible.get(index).map(|index| &self.items[*index])
    }
    pub fn selected_item(&self) -> Option<&Item> {
        self.item(self.selected)
    }
    pub fn step(&mut self, delta: isize) {
        self.deliberate_selection = true;
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.visible.len().saturating_sub(1));
    }
    /// Resolution cannot change the identity, filtering or primary insertion of
    /// the item the user selected. A failed merge retains the original model.
    pub fn replace_selected_resolved(&mut self, original: &Value, resolved: Value) -> Result<()> {
        let index = *self
            .visible
            .get(self.selected)
            .context("No selected completion")?;
        if &self.items[index].value != original {
            bail!("Selected completion changed during resolution");
        }
        validate_resolution(original, &resolved)?;
        let mut item = parse_item(resolved)?;
        let bytes = self.bytes - self.items[index].bytes + item.bytes;
        if bytes > MAX_BYTES {
            bail!("Resolved completion list exceeds 2 MiB");
        }
        item.resolved = true;
        self.items[index] = item;
        self.bytes = bytes;
        Ok(())
    }
}

pub fn validate_resolution(original: &Value, resolved: &Value) -> Result<()> {
    let original = original
        .as_object()
        .context("Invalid original completion")?;
    let resolved = resolved
        .as_object()
        .context("Invalid resolved completion")?;
    for key in original.keys().chain(resolved.keys()) {
        if !matches!(
            key.as_str(),
            "detail" | "documentation" | "additionalTextEdits"
        ) && original.get(key) != resolved.get(key)
        {
            bail!("Completion resolver changed immutable property {key}");
        }
    }
    Ok(())
}

fn parse_item(value: Value) -> Result<Item> {
    let mut budget = Budget(MAX_ITEM_BYTES);
    serde_json::to_writer(&mut budget, &value)?;
    let label = value["label"]
        .as_str()
        .context("Completion has no string label")?;
    let detail = value["detail"].as_str().unwrap_or("");
    let filter = value["filterText"].as_str().unwrap_or(label);
    let sort = value["sortText"].as_str().unwrap_or(label);
    if label.is_empty()
        || [label, detail, filter, sort]
            .iter()
            .any(|s| s.len() > MAX_TEXT)
    {
        bail!("Completion text is empty or exceeds 4 KiB");
    }
    let documentation = match value.get("documentation") {
        None | Some(Value::Null) => "",
        Some(Value::String(text)) => text,
        Some(Value::Object(object)) => {
            if object
                .get("isTrusted")
                .is_some_and(|v| v != &Value::Bool(false))
                || object
                    .get("supportHtml")
                    .is_some_and(|v| v != &Value::Bool(false))
            {
                bail!("Trusted or HTML completion documentation is not supported");
            }
            if let Some(kind) = object.get("kind")
                && !matches!(kind.as_str(), Some("plaintext" | "markdown"))
            {
                bail!("Invalid completion documentation kind");
            }
            object
                .get("value")
                .and_then(Value::as_str)
                .context("Invalid completion documentation")?
        }
        _ => bail!("Invalid completion documentation"),
    };
    if documentation.len() > MAX_DOCUMENTATION {
        bail!("Completion documentation exceeds 32 KiB");
    }
    let mut boundaries = Vec::new();
    let mut previous: Option<char> = None;
    for character in filter.chars() {
        let boundary = previous.is_none_or(|p| {
            !p.is_alphanumeric() || p == '_' || p.is_lowercase() && character.is_uppercase()
        });
        for (index, _) in character.to_lowercase().enumerate() {
            boundaries.push(boundary && index == 0);
        }
        previous = Some(character);
    }
    Ok(Item {
        label: label.to_owned(),
        detail: detail.to_owned(),
        documentation: documentation.to_owned(),
        filter: filter.to_lowercase(),
        sort: sort.to_owned(),
        boundaries,
        bytes: MAX_ITEM_BYTES - budget.0,
        value,
        resolved: false,
    })
}

/// A linear scan per bounded item: exact/prefix matches, then token-boundary
/// abbreviations, then other subsequences. Server sortText breaks equal scores.
fn match_score(item: &Item, prefix: &str) -> Option<(u8, usize, usize)> {
    if prefix.is_empty() {
        return Some((0, 0, 0));
    }
    if item.filter == prefix {
        return Some((0, 0, 0));
    }
    if item.filter.starts_with(prefix) {
        return Some((1, 0, 0));
    }
    let mut wanted = prefix.chars();
    let mut next = wanted.next()?;
    let (mut start, mut misses, mut matched) = (0, 0, 0);
    for (index, character) in item.filter.chars().enumerate() {
        if character != next {
            continue;
        }
        if matched == 0 {
            start = index;
        }
        matched += 1;
        misses += usize::from(!item.boundaries[index]);
        match wanted.next() {
            Some(character) => next = character,
            None => {
                return Some((
                    if misses == 0 { 2 } else { 3 },
                    index - start + 1 - matched,
                    start,
                ));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn filters_unicode_labels_without_changing_edit_identity_and_keeps_selection() {
        let edit = json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"猫Alpha"});
        let mut model = Model::parse(
            json!({"isIncomplete":true,"items":[
                {"label":"Other","sortText":"0"},
                {"label":"猫Alpha","sortText":"2","textEdit":edit},
                {"label":"猫Another","sortText":"1","preselect":true},
            ]}),
            "猫a",
        )
        .unwrap();
        assert!(model.incomplete);
        assert_eq!(model.len(), 2);
        assert_eq!(model.selected_item().unwrap().label, "猫Another");
        model.step(1);
        assert_eq!(model.selected_item().unwrap().value["textEdit"], edit);
        model.filter("猫Al").unwrap();
        assert_eq!(model.len(), 1);
        assert_eq!(model.selected_item().unwrap().label, "猫Alpha");
        model.filter("missing").unwrap();
        model.step(-1);
        assert!(model.selected_item().is_none());
    }
    #[test]
    fn rejects_unbounded_items_text_prefix_and_total_payload_before_retaining_a_model() {
        assert!(Model::parse(json!(vec![json!({"label":"valid"}); 301]), "").is_err());
        assert!(Model::parse(json!(false), "").is_err());
        assert!(
            Model::parse(
                json!({"items":[{"label":"valid"}],"itemDefaults":{"editRange":{}}}),
                ""
            )
            .is_err()
        );
        assert!(Model::parse(json!([{"label":"x".repeat(MAX_TEXT+1)}]), "").is_err());
        assert!(
            Model::parse(
                json!([{"label":"valid","data":"x".repeat(MAX_ITEM_BYTES)}]),
                ""
            )
            .is_err()
        );
        assert!(Model::parse(json!([{"label":"valid"}]), &"x".repeat(MAX_PREFIX + 1)).is_err());
        let values: Vec<_> = (0..40)
            .map(|index| json!({"label":format!("item{index}"),"data":"x".repeat(60*1024)}))
            .collect();
        assert!(Model::parse(json!(values), "").is_err());
    }
    #[test]
    fn ranks_exact_prefix_and_boundary_abbreviations_without_rewriting_provider_edits() {
        let edit = json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"getClient()"});
        let mut model = Model::parse(
            json!([
                {"label":"generic_call", "sortText":"a"},
                {"label":"getClient", "sortText":"b", "textEdit":edit},
                {"label":"gcPrefix", "sortText":"c"},
                {"label":"gc", "sortText":"z"},
                {"label":"unrelated"}
            ]),
            "gc",
        )
        .unwrap();
        let labels: Vec<_> = (0..model.len())
            .map(|i| model.item(i).unwrap().label.as_str())
            .collect();
        assert_eq!(labels, ["gc", "gcPrefix", "getClient", "generic_call"]);
        model.step(2);
        assert_eq!(model.selected_item().unwrap().value["textEdit"], edit);
        model.filter("gC").unwrap();
        assert_eq!(model.selected_item().unwrap().label, "getClient");
        model.filter("getC").unwrap();
        assert_eq!(model.len(), 1);
        assert_eq!(model.selected_item().unwrap().value["textEdit"], edit);
        assert!(
            Model::parse(json!([{"label":"猫Alpha"}]), "猫a")
                .unwrap()
                .len()
                == 1
        );
    }
    #[test]
    fn resolves_only_selected_identity_and_mutable_fields_with_atomic_budget_checks() {
        let original =
            json!({"label":"answer", "insertText":"answer", "sortText":"0", "data":{"id":7}});
        let mut model = Model::parse(json!([original, {"label":"another"}]), "a").unwrap();
        let mut resolved = original.clone();
        resolved["detail"] = json!("int answer");
        resolved["documentation"] = json!({"kind":"markdown","value":"Returns **a value**.\n猫🙂"});
        resolved["additionalTextEdits"] = json!([]);
        model
            .replace_selected_resolved(&original, resolved.clone())
            .unwrap();
        let item = model.selected_item().unwrap();
        assert!(item.resolved);
        assert_eq!(item.documentation, "Returns **a value**.\n猫🙂");
        assert_eq!(item.value["data"], original["data"]);
        for property in ["label", "insertText", "sortText", "data", "command"] {
            let mut invalid = resolved.clone();
            invalid[property] = json!("changed");
            assert!(model.replace_selected_resolved(&resolved, invalid).is_err());
            assert_eq!(model.selected_item().unwrap().value, resolved);
        }
        let mut oversized = resolved.clone();
        oversized["documentation"] = json!("x".repeat(MAX_DOCUMENTATION + 1));
        assert!(
            model
                .replace_selected_resolved(&resolved, oversized)
                .is_err()
        );
        assert_eq!(model.selected_item().unwrap().value, resolved);
        model.step(1);
        assert!(
            model
                .replace_selected_resolved(&resolved, resolved.clone())
                .is_err()
        );
        assert_eq!(model.selected_item().unwrap().label, "another");
        assert!(
            Model::parse(
                json!([{"label":"a","documentation":{"isTrusted":true,"value":"unsafe"}}]),
                ""
            )
            .is_err()
        );
    }
}
