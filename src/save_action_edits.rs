//! Pure origin-only admission for native source actions before saving.
use crate::document::Document;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::ops::Range;

const BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Origin<'a> {
    pub uri: &'a str,
    pub version: i64,
}
pub(crate) struct Staged {
    pub changes: Vec<(Range<usize>, String)>,
    pub raw_bytes: usize,
    pub expanded_bytes: usize,
}

/// Borrow the only admitted origin array. No path lookup or model mutation occurs.
pub(crate) fn origin_text_edits<'a>(
    edit: &'a Value,
    origin: Origin<'_>,
) -> Result<Option<&'a Value>> {
    ensure!(origin.uri.len() <= 8192, "Save action URI exceeds 8 KiB");
    ensure!(
        (0..=i64::from(i32::MAX)).contains(&origin.version),
        "Invalid save action origin version"
    );
    let object = edit.as_object().context("Invalid source WorkspaceEdit")?;
    ensure!(
        object.len() <= 3
            && object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "changes" | "documentChanges" | "changeAnnotations"
                )
            }),
        "Unsupported source WorkspaceEdit fields"
    );
    ensure!(
        !object.contains_key("changes") || !object.contains_key("documentChanges"),
        "Ambiguous source WorkspaceEdit forms"
    );
    ensure!(
        object.get("changeAnnotations").is_none_or(|value| {
            value
                .as_object()
                .is_some_and(|annotations| annotations.is_empty())
        }),
        "Save actions cannot apply annotated edits without review"
    );
    if let Some(changes) = object.get("changes") {
        let changes = changes.as_object().context("Invalid source changes")?;
        ensure!(
            changes.len() <= 1,
            "Save action must edit only its origin document"
        );
        let Some((uri, edits)) = changes.iter().next() else {
            return Ok(None);
        };
        ensure!(
            uri.len() <= 8192 && uri == origin.uri,
            "Save action targets a different URI"
        );
        ensure!(edits.is_array(), "Source edits must be an array");
        return Ok(Some(edits));
    }
    if let Some(changes) = object.get("documentChanges") {
        let changes = changes
            .as_array()
            .context("Invalid source documentChanges")?;
        ensure!(
            changes.len() <= 1,
            "Save action contains multiple document edits"
        );
        let Some(change) = changes.first() else {
            return Ok(None);
        };
        let change = change.as_object().context("Invalid source document edit")?;
        ensure!(
            change.len() == 2
                && change.contains_key("textDocument")
                && change.contains_key("edits"),
            "Save actions cannot create, rename, delete or annotate resources"
        );
        let document = change["textDocument"]
            .as_object()
            .context("Invalid source textDocument")?;
        ensure!(
            document.len() == 2 && document.contains_key("uri") && document.contains_key("version"),
            "Source textDocument requires uri and version"
        );
        ensure!(
            document["uri"]
                .as_str()
                .is_some_and(|uri| uri.len() <= 8192 && uri == origin.uri),
            "Save action targets a different URI"
        );
        let version = &document["version"];
        ensure!(
            version.is_null() || version.as_i64() == Some(origin.version),
            "Source action document version changed or is invalid"
        );
        ensure!(change["edits"].is_array(), "Source edits must be an array");
        return Ok(Some(&change["edits"]));
    }
    Ok(None)
}

/// Validate everything before returning scalar edits, with cumulative caller budget.
pub(crate) fn stage(
    doc: &Document,
    edit: &Value,
    origin: Origin<'_>,
    replacement_budget: usize,
) -> Result<Staged> {
    let Some(edits) = origin_text_edits(edit, origin)? else {
        return Ok(Staged {
            changes: Vec::new(),
            raw_bytes: 0,
            expanded_bytes: 0,
        });
    };
    let values = edits.as_array().expect("array admitted above");
    ensure!(values.len() <= 4096, "Save action exceeds 4096 edits");
    ensure!(
        matches!(doc.eol.as_str(), "\n" | "\r\n"),
        "Unsupported origin EOL"
    );
    let budget = replacement_budget.min(BYTES);
    let mut raw_bytes = 0usize;
    let mut expanded_bytes = 0usize;
    for value in values {
        let text = value
            .get("newText")
            .and_then(Value::as_str)
            .context("Source newText must be a string")?;
        raw_bytes = raw_bytes
            .checked_add(text.len())
            .context("Source replacement overflow")?;
        ensure!(
            raw_bytes <= budget,
            "Save actions exceed cumulative raw replacement budget"
        );
        let mut bytes = text.bytes().peekable();
        while let Some(byte) = bytes.next() {
            let added = if byte == b'\r' || byte == b'\n' {
                if byte == b'\r' && bytes.peek() == Some(&b'\n') {
                    bytes.next();
                }
                doc.eol.len()
            } else {
                1
            };
            expanded_bytes = expanded_bytes
                .checked_add(added)
                .context("Source EOL expansion overflow")?;
            ensure!(
                expanded_bytes <= budget,
                "Save actions exceed cumulative expanded replacement budget"
            );
        }
    }
    let changes = crate::save_formatting_edits::stage(doc, edits)?;
    Ok(Staged {
        changes,
        raw_bytes,
        expanded_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Selection;
    use serde_json::json;
    const URI: &str = "file:///tmp/origin.cpp";
    fn origin() -> Origin<'static> {
        Origin {
            uri: URI,
            version: 7,
        }
    }
    fn text_edit(start: usize, end: usize, text: &str) -> Value {
        json!({"range":{"start":{"line":0,"character":start},"end":{"line":0,"character":end}},"newText":text})
    }
    fn edit(values: Value) -> Value {
        json!({"changes":{URI:values}})
    }
    #[test]
    fn supported_forms_and_empty_envelopes_are_borrowed_and_versions_are_exact() {
        let doc = Document::from_text("猫🙂 x\r\n");
        let values = json!([text_edit(1, 3, "λ")]);
        let wire = edit(values.clone());
        assert!(std::ptr::eq(
            origin_text_edits(&wire, origin()).unwrap().unwrap(),
            &wire["changes"][URI]
        ));
        assert_eq!(
            stage(&doc, &wire, origin(), BYTES).unwrap().changes,
            vec![(1..2, "λ".into())]
        );
        for version in [json!(7), Value::Null] {
            let wire = json!({"documentChanges":[{"textDocument":{"uri":URI,"version":version},"edits":values}]});
            assert_eq!(
                stage(&doc, &wire, origin(), BYTES).unwrap().changes.len(),
                1
            );
        }
        for empty in [
            json!({}),
            json!({"changes":{}}),
            json!({"documentChanges":[]}),
        ] {
            assert!(stage(&doc, &empty, origin(), 0).unwrap().changes.is_empty());
        }
        for version in [json!(6), json!(-1), json!(7.0), json!(2147483648u64)] {
            assert!(stage(&doc, &json!({"documentChanges":[{"textDocument":{"uri":URI,"version":version},"edits":values}]}), origin(), BYTES).is_err());
        }
    }
    #[test]
    fn unrelated_duplicate_resource_annotation_and_malformed_envelopes_reject() {
        let doc = Document::from_text("猫🙂 x\r\n");
        let values = json!([text_edit(1, 3, "λ")]);
        let single = json!({"textDocument":{"uri":URI,"version":7},"edits":values});
        for wire in [
            Value::Null,
            json!({"changes":{URI:values},"documentChanges":[]}),
            json!({"changes":{URI:values,"file:///tmp/other.cpp":[]}}),
            json!({"documentChanges":[single.clone(),single.clone()]}),
            json!({"documentChanges":[{"kind":"delete","uri":URI}]}),
            json!({"documentChanges":[{"textDocument":{"uri":URI},"edits":[]}]}),
            json!({"changes":{"file:///tmp/../tmp/origin.cpp":values}}),
            json!({"changeAnnotations":{"x":{"label":"review"}}}),
            json!({"changes":{URI:values},"unknown":true}),
        ] {
            assert!(stage(&doc, &wire, origin(), BYTES).is_err(), "{wire}");
        }
    }
    #[test]
    fn malformed_last_range_and_overlap_preserve_redo_and_reversed_selections() {
        let mut doc = Document::from_text("猫🙂 x\r\n");
        doc.insert("dirty", false);
        doc.undo();
        doc.set_selections(vec![
            Selection {
                cursor: 3,
                anchor: Some(0),
                desired_column: None,
            },
            Selection::caret(1),
        ]);
        let before = doc.text.to_string();
        let selection = doc.selections();
        let epoch = doc.text_epoch();
        for values in [
            json!([text_edit(1, 3, "λ"), text_edit(2, 3, "bad")]),
            json!([text_edit(1, 3, "λ"), text_edit(5, 6, "bad")]),
            json!([text_edit(0, 0, "x"), text_edit(0, 0, "y")]),
        ] {
            assert!(stage(&doc, &edit(values), origin(), BYTES).is_err());
            assert_eq!(doc.text.to_string(), before);
            assert_eq!(doc.selections(), selection);
            assert_eq!(doc.text_epoch(), epoch);
        }
        doc.redo();
        assert_eq!(doc.text.to_string(), "dirty猫🙂 x\r\n");
    }
    #[test]
    fn normalization_and_noop_preserve_epoch_redo_and_cumulative_budget() {
        let mut doc = Document::from_text("猫🙂 x\r\n");
        doc.insert("dirty", false);
        doc.undo();
        let epoch = doc.text_epoch();
        let no_op = stage(&doc, &edit(json!([text_edit(1, 3, "🙂")])), origin(), BYTES).unwrap();
        assert!(no_op.changes.is_empty());
        doc.apply_changes(no_op.changes);
        assert_eq!(doc.text_epoch(), epoch);
        doc.redo();
        assert_eq!(doc.text.to_string(), "dirty猫🙂 x\r\n");
        let doc = Document::from_text("\r\n");
        let wire = edit(json!([text_edit(0, 0, "\n\n")]));
        assert!(stage(&doc, &wire, origin(), 3).is_err());
        let staged = stage(&doc, &wire, origin(), 4).unwrap();
        assert_eq!((staged.raw_bytes, staged.expanded_bytes), (2, 4));
        assert_eq!(staged.changes, vec![(0..0, "\r\n\r\n".into())]);
    }
    #[test]
    fn one_atomic_application_restores_unicode_crlf_and_all_original_selections() {
        let mut doc = Document::from_text("猫🙂 x\r\n");
        let selections = vec![
            Selection {
                cursor: 2,
                anchor: Some(0),
                desired_column: None,
            },
            Selection::caret(4),
        ];
        doc.set_selections(selections.clone());
        let staged = stage(
            &doc,
            &edit(json!([text_edit(0, 0, "use z;\n"), text_edit(4, 5, "λ")])),
            origin(),
            BYTES,
        )
        .unwrap();
        doc.apply_changes(staged.changes);
        assert_eq!(doc.text.to_string(), "use z;\r\n猫🙂 λ\r\n");
        doc.undo();
        assert_eq!(doc.text.to_string(), "猫🙂 x\r\n");
        assert_eq!(doc.selections(), selections);
        doc.redo();
        assert_eq!(doc.text.to_string(), "use z;\r\n猫🙂 λ\r\n");
    }
}
