//! Bounded overload data shared by native and optional signature providers.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::ops::Range;
struct ResultBudget(usize);
impl std::io::Write for ResultBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::other("Signature help exceeds 256 KiB"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub enabled: bool,
    pub cycle: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: true,
            cycle: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Hint {
    pub label: String,
    pub parameter: Option<Range<usize>>,
    pub documentation: String,
    pub signature: usize,
    pub count: usize,
}

/// Keeps every qualified overload, so navigation never invokes a provider.
#[derive(Clone, Debug)]
pub struct Model {
    help: Value,
    hints: Vec<Hint>,
    selected: usize,
}
impl Model {
    pub fn parse(value: &Value) -> Result<Option<Self>> {
        if value.is_null() {
            return Ok(None);
        }
        serde_json::to_writer(&mut ResultBudget(256 * 1024), value)?;
        // Active indices remain protocol fields even when the selected
        // overload has no parameters, or no overload is currently selected.
        let requested = index(value.get("activeSignature"))?;
        index(value.get("activeParameter"))?;
        let signatures = value["signatures"]
            .as_array()
            .context("Invalid signature help")?;
        if signatures.is_empty() {
            return Ok(None);
        }
        if signatures.len() > 32 {
            bail!("Signature help exceeds 32 signatures");
        }
        let mut bytes = 0usize;
        let mut hints = Vec::with_capacity(signatures.len());
        for (signature, item) in signatures.iter().enumerate() {
            index(item.get("activeParameter"))?;
            let label = item["label"].as_str().context("Missing signature label")?;
            bytes = bytes.saturating_add(label.len());
            if label.len() > 8192 || bytes > 65536 {
                bail!("Signature labels exceed display budget");
            }
            // Validate all documentation and labels, including unselected data.
            documentation(item.get("documentation"))?;
            if let Some(parameters) = item.get("parameters") {
                let parameters = parameters
                    .as_array()
                    .context("Invalid signature parameters")?;
                if parameters.len() > 128 {
                    bail!("Signature exceeds 128 parameters");
                }
                for parameter in parameters {
                    documentation(parameter.get("documentation"))?;
                    if parameter["label"]
                        .as_str()
                        .is_some_and(|text| text.len() > 8192)
                    {
                        bail!("Parameter label exceeds 8 KiB");
                    }
                    parameter_range(label, &parameter["label"])?;
                }
            }
            hints.push(Hint::selected(
                signatures,
                signature,
                value.get("activeParameter"),
            )?);
        }
        let selected = if requested < hints.len() {
            requested
        } else {
            0
        };
        let mut model = Self {
            help: value.clone(),
            hints,
            selected,
        };
        model.update_context();
        Ok(Some(model))
    }
    pub fn hint(&self) -> &Hint {
        &self.hints[self.selected]
    }
    pub fn context(&self) -> Value {
        self.help.clone()
    }
    /// Returns false when navigation should dismiss at a non-cycling boundary.
    pub fn cycle(&mut self, forward: bool, wrap: bool) -> bool {
        if !wrap
            && ((forward && self.selected + 1 == self.hints.len())
                || (!forward && self.selected == 0))
        {
            return false;
        }
        self.selected = if forward {
            (self.selected + 1) % self.hints.len()
        } else {
            (self.selected + self.hints.len() - 1) % self.hints.len()
        };
        self.update_context();
        true
    }
    fn update_context(&mut self) {
        self.help["activeSignature"] = Value::from(self.selected);
        // Preserve the help-level parameter while overload-specific overrides
        // remain on their original signatures, matching the provider protocol.
        if self.help.get("activeParameter").is_none() {
            self.help["activeParameter"] = Value::from(0);
        }
    }
}
fn index(value: Option<&Value>) -> Result<usize> {
    value.map_or(Ok(0), |v| {
        v.as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .context("Invalid active signature or parameter")
    })
}
fn documentation(value: Option<&Value>) -> Result<String> {
    let Some(value) = value else {
        return Ok(String::new());
    };
    let text = value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))
        .context("Invalid signature documentation")?;
    if text.len() > 8192 {
        bail!("Signature documentation exceeds 8 KiB")
    }
    Ok(text.into())
}
fn byte_offset(text: &str, units: usize) -> Result<usize> {
    let mut offset = 0;
    for (byte, ch) in text.char_indices() {
        if offset == units {
            return Ok(byte);
        }
        offset += ch.len_utf16();
        if offset > units {
            bail!("Parameter label splits a UTF-16 character")
        }
    }
    if offset == units {
        Ok(text.len())
    } else {
        bail!("Parameter label is outside the signature")
    }
}
// VS Code 1.95 getParameterLabelOffsets matches an escaped literal between
// ECMAScript ASCII non-word boundaries, returning an empty span on no match.
fn string_parameter_range(label: &str, parameter: &str) -> Range<usize> {
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    for start in label
        .char_indices()
        .map(|(start, _)| start)
        .chain(std::iter::once(label.len()))
    {
        if start > 0 && word(label.as_bytes()[start - 1]) {
            continue;
        }
        if !label[start..].starts_with(parameter) {
            continue;
        }
        let end = start + parameter.len();
        if end == label.len() || !word(label.as_bytes()[end]) {
            return start..end;
        }
    }
    0..0
}
fn parameter_range(label: &str, value: &Value) -> Result<Range<usize>> {
    let range = if let Some(text) = value.as_str() {
        string_parameter_range(label, text)
    } else {
        let pair = value
            .as_array()
            .filter(|pair| pair.len() == 2)
            .context("Invalid parameter label")?;
        byte_offset(label, index(pair.first())?)?..byte_offset(label, index(pair.get(1))?)?
    };
    if range.start > range.end {
        bail!("Reversed parameter label");
    }
    Ok(range)
}
impl Hint {
    pub fn parse(value: &Value) -> Result<Option<Self>> {
        Ok(Model::parse(value)?.map(|model| model.hint().clone()))
    }
    fn selected(
        signatures: &[Value],
        signature: usize,
        active_parameter: Option<&Value>,
    ) -> Result<Self> {
        let selected = &signatures[signature];
        let label = selected["label"]
            .as_str()
            .context("Missing signature label")?;
        let mut docs = documentation(selected.get("documentation"))?;
        let mut parameter = None;
        if let Some(parameters) = selected["parameters"]
            .as_array()
            .filter(|parameters| !parameters.is_empty())
        {
            let requested = index(selected.get("activeParameter").or(active_parameter))?;
            let selected = &parameters[if requested < parameters.len() {
                requested
            } else {
                0
            }];
            parameter = Some(parameter_range(label, &selected["label"])?);
            let detail = documentation(selected.get("documentation"))?;
            if !detail.is_empty() {
                if !docs.is_empty() {
                    docs.push('\n');
                }
                docs.push_str(&detail);
            }
        }
        Ok(Self {
            label: label.into(),
            parameter,
            documentation: docs,
            signature,
            count: signatures.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn overload_navigation_is_local_and_preserves_provider_context_handle() {
        let value = json!({"_vscliSignatureHelpHandle":17,"activeSignature":0,"activeParameter":1,"signatures":[
            {"label":"f(猫🙂 left, int right)","parameters":[{"label":"猫🙂 left"},{"label":"int right","documentation":"integer argument"}],"documentation":"first"},
            {"label":"f(🙂 item, double count)","activeParameter":0,"parameters":[{"label":[2,9]},{"label":"double count"}],"documentation":"second"}
        ]});
        let mut model = Model::parse(&value).unwrap().unwrap();
        assert_eq!(
            &model.hint().label[model.hint().parameter.clone().unwrap()],
            "int right"
        );
        assert!(model.cycle(true, true));
        assert_eq!(model.hint().documentation, "second");
        assert_eq!(
            &model.hint().label[model.hint().parameter.clone().unwrap()],
            "🙂 item"
        );
        assert_eq!(model.context()["activeSignature"], 1);
        assert_eq!(model.context()["activeParameter"], 1);
        assert_eq!(model.context()["_vscliSignatureHelpHandle"], 17);
        assert!(!model.cycle(true, false));
        assert!(model.cycle(true, true));
        assert_eq!(model.hint().signature, 0);
        assert!(!model.cycle(false, false));
        assert!(model.cycle(false, true));
        assert_eq!(model.hint().signature, 1);
    }
    #[test]
    fn every_overload_and_total_result_are_qualified_before_display() {
        for invalid in [
            json!({"label":"f(🙂)","parameters":[{"label":[3,4]}]}),
            json!({"label":"f()","documentation":"x".repeat(8193)}),
            json!({"label":"f()","parameters":[{"label":"x","documentation":"x".repeat(8193)}]}),
        ] {
            assert!(
                Model::parse(
                    &json!({"activeSignature":0,"signatures":[{"label":"valid()"},invalid]})
                )
                .is_err()
            );
        }
        let signatures = vec![
            json!({"label":"f(int x)","documentation":"x".repeat(8192),"parameters":[{"label":"int x","documentation":"x".repeat(8192)}]});
            17
        ];
        assert!(
            Model::parse(&json!({"signatures":signatures}))
                .unwrap_err()
                .to_string()
                .contains("256 KiB")
        );
    }
    #[test]
    fn invalid_active_indices_reject_even_with_empty_or_unselected_parameters() {
        for malformed in [json!(-1), json!(1.5), json!("0"), json!(false), Value::Null] {
            for signatures in [
                json!([]),
                json!([{"label":"f()"}]),
                json!([{"label":"f()","parameters":[]}]),
            ] {
                assert!(
                    Model::parse(&json!({"activeParameter":malformed,"signatures":signatures}))
                        .is_err()
                );
            }
            for parameters in [None, Some(json!([]))] {
                let mut unselected = json!({"label":"other()","activeParameter":malformed});
                if let Some(parameters) = parameters {
                    unselected["parameters"] = parameters;
                }
                assert!(Model::parse(&json!({"activeSignature":0,"signatures":[{"label":"selected()"},unselected]})).is_err());
            }
        }
        assert!(Model::parse(&json!({"activeSignature":-1,"signatures":[]})).is_err());
        assert!(
            Model::parse(
                &json!({"activeParameter":0,"signatures":[{"label":"f()","activeParameter":0}]})
            )
            .unwrap()
            .is_some()
        );
    }
    #[test]
    fn utf16_parameter_ranges_and_selected_signature_override_are_preserved() {
        let hint = Hint::parse(&json!({"activeSignature":1,"activeParameter":0,"signatures":[{"label":"other()"},{"label":"f(😀, int count)","activeParameter":1,"parameters":[{"label":[2,4]},{"label":[6,15],"documentation":"count docs"}],"documentation":{"kind":"markdown","value":"function docs"}}]})).unwrap().unwrap();
        assert_eq!(&hint.label[hint.parameter.unwrap()], "int count");
        assert_eq!(hint.signature, 1);
        assert_eq!(hint.documentation, "function docs\ncount docs");
    }
    #[test]
    fn string_labels_follow_pinned_ascii_boundaries_and_literal_matching() {
        for (label, parameter, expected) in [
            ("print(value: int)", "int", 13..16),
            ("f(intValue, int)", "int", 12..15),
            ("f(_int, int_)", "int", 0..0),
            ("f(λintλ)", "int", 4..7),
            ("f(value: T[])", "T[]", 9..12),
            ("xa-a-a", "a-a", 3..6),
            ("f(int)", "missing", 0..0),
            ("f()", "", 2..2),
        ] {
            let hint = Hint::parse(
                &json!({"signatures":[{"label":label,"parameters":[{"label":parameter}]}]}),
            )
            .unwrap()
            .unwrap();
            assert_eq!(hint.parameter, Some(expected), "{label}: {parameter}");
        }
    }
    #[test]
    fn malformed_ranges_and_budgets_reject_without_renderable_data() {
        for parameter in [json!([3, 4]), json!([4, 2]), json!([0, 999])] {
            assert!(
                Hint::parse(
                    &json!({"signatures":[{"label":"f(😀)","parameters":[{"label":parameter}]}]})
                )
                .is_err()
            );
        }
        assert!(Hint::parse(&json!({"signatures":vec![json!({"label":"f()"});33]})).is_err());
        assert!(Hint::parse(&json!({"signatures":[{"label":"x".repeat(8193)}]})).is_err());
        assert!(
            Hint::parse(
                &json!({"signatures":[{"label":"f()","parameters":vec![json!({"label":""});129]}]})
            )
            .is_err()
        );
        assert!(
            Hint::parse(&json!({"signatures":[{"label":"f()","documentation":"x".repeat(8193)}]}))
                .is_err()
        );
        assert!(Hint::parse(&Value::Null).unwrap().is_none());
        let hint=Hint::parse(&json!({"activeSignature":9,"activeParameter":9,"signatures":[{"label":"f(int x)","parameters":[{"label":"int x"}]}]})).unwrap().unwrap();
        assert_eq!(&hint.label[hint.parameter.unwrap()], "int x");
    }
}
