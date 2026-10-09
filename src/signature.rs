//! Bounded native rendering data for explicit LSP signature help.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::ops::Range;

#[derive(Debug)]
pub struct Hint {
    pub label: String,
    pub parameter: Option<Range<usize>>,
    pub documentation: String,
    pub signature: usize,
    pub count: usize,
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
impl Hint {
    pub fn parse(value: &Value) -> Result<Option<Self>> {
        if value.is_null() {
            return Ok(None);
        }
        let signatures = value["signatures"]
            .as_array()
            .context("Invalid signature help")?;
        if signatures.is_empty() {
            return Ok(None);
        }
        if signatures.len() > 32 {
            bail!("Signature help exceeds 32 signatures")
        }
        let mut bytes = 0;
        for signature in signatures {
            let label = signature["label"]
                .as_str()
                .context("Missing signature label")?;
            bytes += label.len();
            if label.len() > 8192 || bytes > 65536 {
                bail!("Signature labels exceed display budget")
            }
            if let Some(parameters) = signature.get("parameters")
                && parameters
                    .as_array()
                    .context("Invalid signature parameters")?
                    .len()
                    > 128
            {
                bail!("Signature exceeds 128 parameters")
            }
        }
        let requested = index(value.get("activeSignature"))?;
        let signature = if requested < signatures.len() {
            requested
        } else {
            0
        };
        let selected = &signatures[signature];
        let label = selected["label"].as_str().unwrap();
        let mut docs = documentation(selected.get("documentation"))?;
        let mut parameter = None;
        if let Some(parameters) = selected["parameters"].as_array().filter(|p| !p.is_empty()) {
            let requested = index(
                selected
                    .get("activeParameter")
                    .or_else(|| value.get("activeParameter")),
            )?;
            let selected = &parameters[if requested < parameters.len() {
                requested
            } else {
                0
            }];
            let range = if let Some(text) = selected["label"].as_str() {
                let start = label
                    .find(text)
                    .context("Parameter label is absent from signature")?;
                start..start + text.len()
            } else {
                let pair = selected["label"]
                    .as_array()
                    .filter(|p| p.len() == 2)
                    .context("Invalid parameter label")?;
                byte_offset(label, index(pair.first())?)?..byte_offset(label, index(pair.get(1))?)?
            };
            if range.start > range.end {
                bail!("Reversed parameter label")
            }
            parameter = Some(range);
            let detail = documentation(selected.get("documentation"))?;
            if !detail.is_empty() {
                if !docs.is_empty() {
                    docs.push('\n');
                }
                docs.push_str(&detail);
            }
        }
        Ok(Some(Self {
            label: label.into(),
            parameter,
            documentation: docs,
            signature,
            count: signatures.len(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn utf16_parameter_ranges_and_selected_signature_override_are_preserved() {
        let hint = Hint::parse(&json!({"activeSignature":1,"activeParameter":0,"signatures":[{"label":"other()"},{"label":"f(😀, int count)","activeParameter":1,"parameters":[{"label":[2,4]},{"label":[6,15],"documentation":"count docs"}],"documentation":{"kind":"markdown","value":"function docs"}}]})).unwrap().unwrap();
        assert_eq!(&hint.label[hint.parameter.unwrap()], "int count");
        assert_eq!(hint.signature, 1);
        assert_eq!(hint.documentation, "function docs\ncount docs");
    }
    #[test]
    fn malformed_ranges_and_budgets_reject_without_renderable_data() {
        for parameter in [
            json!([3, 4]),
            json!([4, 2]),
            json!([0, 999]),
            json!("absent"),
        ] {
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
