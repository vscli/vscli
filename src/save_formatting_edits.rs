//! Pure admission for one native document's save-time formatting edits.
//!
//! This module stages text only. Its caller owns asynchronous source/document
//! proofs and decides whether to apply the entire result as one transaction.
use crate::document::Document;
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::ops::Range;

const MAX_EDITS: usize = 4096;
const MAX_REPLACEMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;

/// Validate every edit before returning changes; never mutate the document.
///
/// Ranges use strict logical-line UTF-16 coordinates. Returned edits are sorted
/// and disjoint, with document EOLs. Byte-identical edits are omitted only after
/// all input ranges, overlap and budgets have passed admission.
pub fn stage(doc: &Document, edits: &Value) -> Result<Vec<(Range<usize>, String)>> {
    if edits.is_null() {
        return Ok(Vec::new());
    }
    let edits = edits
        .as_array()
        .context("Save formatting edits must be an array or null")?;
    ensure!(
        edits.len() <= MAX_EDITS,
        "Save formatting exceeds 4096 edits"
    );
    if edits.is_empty() {
        return Ok(Vec::new());
    }
    let mut changes = Vec::with_capacity(edits.len());
    let mut raw_bytes = 0usize;
    let mut expanded_bytes = 0usize;
    for edit in edits {
        let edit = fields(
            edit,
            &["range", "newText"],
            "Invalid save formatting TextEdit",
        )?;
        let text = edit["newText"]
            .as_str()
            .context("Save formatting newText must be a string")?;
        raw_bytes = raw_bytes
            .checked_add(text.len())
            .context("Save formatting raw byte count overflow")?;
        ensure!(
            raw_bytes <= MAX_REPLACEMENT_BYTES,
            "Save formatting exceeds 4 MiB raw replacement text"
        );
        let range = fields(
            &edit["range"],
            &["start", "end"],
            "Invalid save formatting range",
        )?;
        let start = offset(doc, &range["start"])?;
        let end = offset(doc, &range["end"])?;
        ensure!(start <= end, "Save formatting range is reversed");
        let text = normalized(text, &doc.eol, MAX_REPLACEMENT_BYTES - expanded_bytes)?;
        expanded_bytes += text.len();
        changes.push((start..end, text));
    }
    changes.sort_by_key(|(range, _)| (range.start, range.end));
    ensure!(
        changes
            .windows(2)
            .all(|pair| { pair[0].0.end <= pair[1].0.start && pair[0].0.start != pair[1].0.start }),
        "Save formatting edits overlap or have ambiguous equal starts"
    );
    let removed = changes.iter().try_fold(0usize, |bytes, (range, _)| {
        bytes
            .checked_add(doc.text.slice(range.clone()).len_bytes())
            .context("Save formatting removed byte count overflow")
    })?;
    let final_bytes = doc
        .text
        .len_bytes()
        .checked_sub(removed)
        .and_then(|bytes| bytes.checked_add(expanded_bytes))
        .context("Save formatting final byte count overflow")?;
    ensure!(
        final_bytes <= MAX_DOCUMENT_BYTES,
        "Save formatting result exceeds 32 MiB document limit"
    );
    changes.retain(|(range, text)| {
        let original = doc.text.slice(range.clone());
        original.len_bytes() != text.len()
            || !original.chunks().flat_map(str::bytes).eq(text.bytes())
    });
    Ok(changes)
}

fn fields<'a>(value: &'a Value, names: &[&str], error: &str) -> Result<&'a Map<String, Value>> {
    let object = value.as_object().context(error.to_owned())?;
    // Reject oversized arbitrary metadata before traversing any of its values.
    ensure!(
        object.len() == names.len() && names.iter().all(|name| object.contains_key(*name)),
        "{error}"
    );
    Ok(object)
}

fn offset(doc: &Document, value: &Value) -> Result<usize> {
    let position = fields(
        value,
        &["line", "character"],
        "Invalid save formatting position",
    )?;
    let coordinate = |name| -> Result<usize> {
        let value = position[name]
            .as_u64()
            .context("Save formatting coordinates must be nonnegative integers")?;
        ensure!(
            value <= i32::MAX as u64,
            "Save formatting coordinate exceeds LSP bounds"
        );
        Ok(value as usize)
    };
    let line_number = coordinate("line")?;
    let units = coordinate("character")?;
    ensure!(
        line_number < doc.line_count(),
        "Invalid save formatting line"
    );
    let line = doc.line_slice(line_number);
    ensure!(
        units <= line.len_utf16_cu(),
        "Invalid save formatting column"
    );
    // Ropey's indexed conversion avoids walking a long line for every edit.
    // The inverse check rejects its otherwise permissive surrogate rounding.
    let scalar = line.utf16_cu_to_char(units);
    ensure!(
        line.char_to_utf16_cu(scalar) == units,
        "Save formatting range splits a UTF-16 surrogate pair"
    );
    Ok(doc.line_start(line_number) + scalar)
}

fn normalized(text: &str, eol: &str, available: usize) -> Result<String> {
    ensure!(
        matches!(eol, "\n" | "\r\n"),
        "Unsupported document line ending"
    );
    if !text.contains(['\r', '\n']) {
        ensure!(
            text.len() <= available,
            "Save formatting exceeds 4 MiB normalized replacement text"
        );
        return Ok(text.to_owned());
    }
    // Compute expansion before allocation; all raw input already passed its
    // aggregate cap, and rejected CRLF expansion allocates no oversized String.
    let mut size = 0usize;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let added = if matches!(character, '\r' | '\n') {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            eol.len()
        } else {
            character.len_utf8()
        };
        ensure!(
            added <= available.saturating_sub(size),
            "Save formatting exceeds 4 MiB normalized replacement text"
        );
        size += added;
    }
    let mut output = String::with_capacity(size);
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if matches!(character, '\r' | '\n') {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            output.push_str(eol);
        } else {
            output.push(character);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Selection;
    use serde_json::json;

    fn edit(start: (usize, usize), end: (usize, usize), text: &str) -> Value {
        json!({"range":{"start":{"line":start.0,"character":start.1},
            "end":{"line":end.0,"character":end.1}},"newText":text})
    }
    fn view(doc: &Document, id: u64) -> (usize, Option<usize>, Vec<Selection>) {
        let view = doc.view_state(Some(id));
        (view.cursor, view.anchor, view.secondary.clone())
    }

    #[test]
    fn unicode_crlf_and_eof_changes_apply_once_preserving_shared_reversed_views() {
        let original = "猫🙂x\r\nλ y\r\n";
        let expected = "猫😎\r\nx\r\nλ Y\r\ntail\r\n";
        let mut doc = Document::from_text(original);
        doc.activate_view(1);
        doc.set_selections(vec![
            Selection {
                cursor: 0,
                anchor: Some(3),
                desired_column: None,
            },
            Selection::caret(6),
        ]);
        doc.activate_view(2);
        doc.set_selections(vec![
            Selection {
                cursor: 8,
                anchor: Some(5),
                desired_column: None,
            },
            Selection::caret(0),
        ]);
        let first = view(&doc, 1);
        let second = view(&doc, 2);
        let id = doc.id;
        let epoch = doc.text_epoch();
        let changes = stage(
            &doc,
            &json!([
                edit((2, 0), (2, 0), "tail\n"),
                edit((1, 2), (1, 3), "Y"),
                edit((0, 1), (0, 3), "😎\n")
            ]),
        )
        .unwrap();
        assert_eq!(doc.text.to_string(), original);
        assert_eq!(doc.text_epoch(), epoch);
        assert_eq!(view(&doc, 1), first);
        assert_eq!(view(&doc, 2), second);
        assert_eq!(
            changes
                .iter()
                .map(|(range, _)| range.clone())
                .collect::<Vec<_>>(),
            vec![1..2, 7..8, 10..10]
        );
        doc.apply_changes(changes);
        assert_eq!(doc.id, id);
        assert_eq!(doc.text.to_string(), expected);
        assert!(doc.dirty());
        assert!(doc.text_epoch() > epoch);
        assert!(doc.view_state(Some(1)).anchor.unwrap() > doc.view_state(Some(1)).cursor);
        assert!(doc.view_state(Some(2)).anchor.unwrap() < doc.view_state(Some(2)).cursor);
        let formatted_first = view(&doc, 1);
        let formatted_second = view(&doc, 2);
        doc.undo();
        assert_eq!(doc.text.to_string(), original);
        assert_eq!(view(&doc, 1), first);
        assert_eq!(view(&doc, 2), second);
        assert!(!doc.dirty());
        doc.undo();
        assert_eq!(doc.text.to_string(), original);
        doc.redo();
        assert_eq!(doc.text.to_string(), expected);
        assert_eq!(view(&doc, 1), formatted_first);
        assert_eq!(view(&doc, 2), formatted_second);
    }

    #[test]
    fn invalid_late_edit_preserves_bytes_views_baseline_and_existing_redo() {
        let mut doc = Document::from_text("猫🙂x\r\nλ y\r\n");
        doc.activate_view(1);
        doc.insert("pending ", false);
        let redone = doc.text.clone();
        doc.undo();
        doc.set_selections(vec![
            Selection {
                cursor: 1,
                anchor: Some(3),
                desired_column: None,
            },
            Selection::caret(7),
        ]);
        doc.activate_view(2);
        doc.move_to(5, false);
        let text = doc.text.clone();
        let first = view(&doc, 1);
        let second = view(&doc, 2);
        let identity = (
            doc.id,
            doc.revision,
            doc.saved_revision,
            doc.save_generation(),
            doc.text_epoch(),
        );
        assert!(
            stage(
                &doc,
                &json!([
                    edit((0, 0), (0, 1), "valid"),
                    edit((0, 2), (0, 3), "invalid surrogate")
                ])
            )
            .unwrap_err()
            .to_string()
            .contains("surrogate")
        );
        assert_eq!(doc.text, text);
        assert_eq!(view(&doc, 1), first);
        assert_eq!(view(&doc, 2), second);
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.saved_revision,
                doc.save_generation(),
                doc.text_epoch()
            ),
            identity
        );
        assert!(doc.disk_content.is_none());
        assert!(!doc.dirty());
        doc.redo();
        assert_eq!(doc.text, redone);
    }

    #[test]
    fn null_empty_and_byte_identical_results_do_not_consume_redo_or_change_epoch() {
        let mut doc = Document::from_text("x\r\n");
        doc.insert("pending ", false);
        let pending = doc.text.clone();
        doc.undo();
        let epoch = doc.text_epoch();
        let selection = doc.selections();
        for edits in [
            Value::Null,
            json!([]),
            json!([edit((0, 0), (1, 0), "x\n")]),
            json!([edit((1, 0), (1, 0), "")]),
        ] {
            let changes = stage(&doc, &edits).unwrap();
            assert!(changes.is_empty());
            doc.apply_changes(changes);
            assert_eq!(doc.text_epoch(), epoch);
            assert_eq!(doc.selections(), selection);
            assert!(!doc.dirty());
        }
        doc.redo();
        assert_eq!(doc.text, pending);
    }

    #[test]
    fn replacement_line_endings_normalize_cr_lf_and_crlf_without_rewriting_source() {
        for eol in ["\n", "\r\n"] {
            let mut doc = Document::from_text("猫");
            doc.eol = eol.into();
            let expected = ["a", "b", "c", "d"].join(eol);
            let changes = stage(&doc, &json!([edit((0, 1), (0, 1), "a\rb\r\nc\nd")])).unwrap();
            assert_eq!(changes, vec![(1..1, expected)]);
            assert_eq!(doc.text.to_string(), "猫");
            assert_eq!(doc.text_epoch(), 0);
        }
    }

    #[test]
    fn strict_coordinates_exclude_surrogates_eol_and_out_of_range_lines() {
        let doc = Document::from_text("猫🙂x\r\nλ\r\n");
        for (line, column, message) in [
            (0, 2, "surrogate"),
            (0, 5, "column"),
            (1, 2, "column"),
            (2, 1, "column"),
            (3, 0, "line"),
        ] {
            assert!(
                stage(&doc, &json!([edit((line, column), (line, column), "!")]))
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
        assert_eq!(
            stage(&doc, &json!([edit((0, 4), (0, 4), "!")])).unwrap()[0].0,
            3..3
        );
        assert!(
            stage(&doc, &json!([edit((1, 0), (0, 0), "")]))
                .unwrap_err()
                .to_string()
                .contains("reversed")
        );
        for value in [
            json!(-1),
            json!(1.5),
            json!(true),
            json!(i32::MAX as u64 + 1),
        ] {
            let mut bad = edit((0, 0), (0, 0), "!");
            bad["range"]["start"]["character"] = value;
            assert!(stage(&doc, &json!([bad])).is_err());
        }
        assert_eq!(
            stage(
                &Document::from_text(""),
                &json!([edit((0, 0), (0, 0), "猫")])
            )
            .unwrap()[0]
                .0,
            0..0
        );
    }

    #[test]
    fn malformed_metadata_and_ambiguous_overlaps_are_rejected_before_noop_removal() {
        let doc = Document::from_text("abc\n");
        for invalid in [
            json!({}),
            json!([null]),
            json!([{"range":{},"newText":"x"}]),
            json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":7}]),
        ] {
            assert!(stage(&doc, &invalid).is_err());
        }
        for (path, value) in [
            (vec!["annotationId"], json!("a")),
            (vec!["range", "extra"], json!(null)),
            (vec!["range", "start", "extra"], json!(null)),
        ] {
            let mut invalid = edit((0, 0), (0, 0), "!");
            let mut cursor = &mut invalid;
            for key in &path[..path.len() - 1] {
                cursor = &mut cursor[*key];
            }
            cursor[path[path.len() - 1]] = value;
            assert!(stage(&doc, &json!([invalid])).is_err());
        }
        for edits in [
            json!([edit((0, 0), (0, 2), "x"), edit((0, 1), (0, 3), "y")]),
            json!([edit((0, 0), (0, 0), ""), edit((0, 0), (0, 1), "a")]),
            json!([edit((0, 0), (0, 0), "x"), edit((0, 0), (0, 0), "y")]),
        ] {
            assert!(
                stage(&doc, &edits)
                    .unwrap_err()
                    .to_string()
                    .contains("overlap")
            );
        }
        assert_eq!(
            stage(
                &doc,
                &json!([edit((0, 1), (0, 2), "Y"), edit((0, 0), (0, 1), "X")])
            )
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn aggregate_raw_expanded_edit_count_and_final_document_limits_are_enforced() {
        let doc = Document::from_text("x\r\n");
        assert!(
            stage(
                &doc,
                &json!([edit((0, 0), (0, 0), &"x".repeat(MAX_REPLACEMENT_BYTES + 1))])
            )
            .unwrap_err()
            .to_string()
            .contains("raw")
        );
        assert!(
            stage(
                &doc,
                &json!([edit(
                    (0, 0),
                    (0, 0),
                    &"\n".repeat(MAX_REPLACEMENT_BYTES / 2 + 1)
                )])
            )
            .unwrap_err()
            .to_string()
            .contains("normalized")
        );
        let half = "x".repeat(MAX_REPLACEMENT_BYTES / 2 + 1);
        assert!(
            stage(
                &doc,
                &json!([edit((0, 0), (0, 0), &half), edit((0, 1), (0, 1), &half)])
            )
            .unwrap_err()
            .to_string()
            .contains("raw")
        );
        assert!(
            stage(&doc, &json!(vec![edit((0, 0), (0, 0), ""); MAX_EDITS + 1]))
                .unwrap_err()
                .to_string()
                .contains("4096")
        );
        let maximum = Document::from_text(&"x".repeat(MAX_DOCUMENT_BYTES));
        assert!(
            stage(&maximum, &json!([edit((0, 0), (0, 0), "猫")]))
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        assert_eq!(
            stage(&maximum, &json!([edit((0, 0), (0, 3), "猫")]))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn many_coordinates_on_a_long_line_use_exact_indexed_offsets() {
        let doc = Document::from_text(&format!("{}🙂z\r\n", "x".repeat(1024 * 1024)));
        let edits: Vec<_> = (0..MAX_EDITS)
            .rev()
            .map(|index| {
                let offset = index * 256;
                edit((0, offset), (0, offset), ".")
            })
            .collect();
        let changes = stage(&doc, &json!(edits)).unwrap();
        assert_eq!(changes.len(), MAX_EDITS);
        for (index, (range, text)) in changes.iter().enumerate() {
            assert_eq!(range, &(index * 256..index * 256));
            assert_eq!(text, ".");
        }
        assert_eq!(
            stage(
                &doc,
                &json!([edit((0, 1024 * 1024 + 2), (0, 1024 * 1024 + 3), "Z")])
            )
            .unwrap()[0]
                .0,
            1024 * 1024 + 1..1024 * 1024 + 2
        );
    }
}
