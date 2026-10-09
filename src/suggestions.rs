//! Bounded native completion items and filtering, independent of any runtime.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{self, Write};

pub const MAX_ITEMS: usize = 300;
pub const MAX_TEXT: usize = 4096;
pub const MAX_PREFIX: usize = 1024;
const MAX_ITEM_BYTES: usize = 64 * 1024;
const MAX_BYTES: usize = 2 * 1024 * 1024;

pub struct Item {
    pub label: String,
    pub detail: String,
    pub value: Value,
    filter: String,
    sort: String,
}
pub struct Model {
    items: Vec<Item>,
    visible: Vec<usize>,
    selected: usize,
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
            let mut budget = Budget(MAX_ITEM_BYTES);
            serde_json::to_writer(&mut budget, value)?;
            retained += MAX_ITEM_BYTES - budget.0;
            if retained > MAX_BYTES {
                bail!("Completion list exceeds 2 MiB");
            }
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
            items.push(Item {
                label: label.to_owned(),
                detail: detail.to_owned(),
                value: value.clone(),
                filter: filter.to_lowercase(),
                sort: sort.to_owned(),
            });
        }
        items.sort_by(|a, b| a.sort.cmp(&b.sort).then(a.label.cmp(&b.label)));
        let mut model = Self {
            items,
            visible: Vec::new(),
            selected: 0,
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
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.filter.starts_with(&prefix).then_some(index))
            .collect();
        self.selected = selected
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
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.visible.len().saturating_sub(1));
    }
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
}
