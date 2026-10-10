use super::*;
use crate::{
    document::Selection,
    lsp,
    snippet::{
        Expansion, Template,
        variables::{Cursor, Environment},
    },
};
use anyhow::{Context, bail, ensure};

const MAX_EDIT_BYTES: usize = 1024 * 1024;

fn replacement_range(doc: &Document, edit: &Value) -> Result<std::ops::Range<usize>> {
    ensure!(edit.is_object(), "Completion edit must be an object");
    ensure!(
        edit.get("insert").is_none() && edit.get("replace").is_none(),
        "Insert/replace completion ranges are not supported"
    );
    let range: lsp::Range = serde_json::from_value(
        edit.get("range")
            .context("Completion edit missing range")?
            .clone(),
    )?;
    let start = lsp::offset(doc, range.start)?;
    let end = lsp::offset(doc, range.end)?;
    ensure!(start <= end, "Completion edit has a reversed range");
    Ok(start..end)
}

fn normalized(text: &str, eol: &str) -> Result<String> {
    ensure!(
        text.len() <= MAX_EDIT_BYTES,
        "Completion edit exceeds 1 MiB"
    );
    let result = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', eol);
    ensure!(
        result.len() <= MAX_EDIT_BYTES,
        "Normalized completion edit exceeds 1 MiB"
    );
    Ok(result)
}

impl App {
    /// Request ownership and version checks belong to the caller. This method
    /// stages every range, variable, expansion and import before changing text.
    pub(super) fn apply_completion_item(&mut self, item: &Value, cursor: usize) -> Result<()> {
        ensure!(item.is_object(), "Completion item must be an object");
        ensure!(cursor <= self.doc().len(), "Completion cursor is invalid");
        ensure!(
            item.get("command").is_none(),
            "Completion follow-up commands are not supported"
        );
        if let Some(mode) = item.get("insertTextMode") {
            ensure!(
                mode.as_u64() == Some(1),
                "Completion insertTextMode is unsupported"
            );
        }
        let format = match item.get("insertTextFormat") {
            None => 1,
            Some(value) => value
                .as_u64()
                .context("Invalid completion insertTextFormat")?,
        };
        ensure!(
            matches!(format, 1 | 2),
            "Invalid completion insertTextFormat"
        );
        let (primary, text) = if let Some(edit) = item.get("textEdit") {
            (
                replacement_range(self.doc(), edit)?,
                edit.get("newText")
                    .and_then(Value::as_str)
                    .context("Completion edit missing text")?,
            )
        } else {
            let mut start = cursor;
            let mut scanned = 0;
            while start > 0 {
                let ch = self.doc().text.char(start - 1);
                if !ch.is_alphanumeric() && ch != '_' {
                    break;
                }
                scanned += ch.len_utf8();
                ensure!(scanned <= 1024, "Completion word prefix exceeds 1 KiB");
                start -= 1;
            }
            (
                start..cursor,
                item.get("insertText")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("label").and_then(Value::as_str))
                    .context("Completion missing text")?,
            )
        };
        ensure!(
            text.len() <= MAX_EDIT_BYTES,
            "Completion text exceeds 1 MiB"
        );
        let mut additional = Vec::new();
        let mut raw_bytes = text.len();
        let mut normalized_bytes = 0usize;
        if let Some(edits) = item.get("additionalTextEdits") {
            let edits = edits
                .as_array()
                .context("Completion additionalTextEdits must be an array")?;
            ensure!(edits.len() < 4096, "Completion edit count exceeds 4096");
            for edit in edits {
                let range = replacement_range(self.doc(), edit)?;
                let text = edit
                    .get("newText")
                    .and_then(Value::as_str)
                    .context("Additional completion edit missing text")?;
                raw_bytes = raw_bytes.saturating_add(text.len());
                ensure!(raw_bytes <= MAX_EDIT_BYTES, "Completion edits exceed 1 MiB");
                let text = normalized(text, &self.doc().eol)?;
                normalized_bytes = normalized_bytes.saturating_add(text.len());
                ensure!(
                    normalized_bytes <= MAX_EDIT_BYTES,
                    "Normalized completion edits exceed 1 MiB"
                );
                additional.push((range, text));
            }
        }
        let template = if format == 2 {
            let template = Template::parse_user(text)?;
            if template.uses_variable("CLIPBOARD") {
                bail!("Clipboard variables in completion snippets are not supported");
            }
            Some(template)
        } else {
            None
        };
        let environment = Environment {
            workspace: self.workspace.root.clone(),
            clipboard: None,
            language: self.language().into(),
            timestamp: chrono::Local::now().fixed_offset(),
        };
        let selection = Selection {
            cursor: primary.end,
            anchor: Some(primary.start),
            desired_column: None,
        };
        self.doc_mut()
            .apply_completion_edits(primary, additional, |document, whitespace| {
                if let Some(template) = template {
                    template.expand_for_completion(
                        &mut |name: &str, indent: Option<&str>| {
                            environment.resolve(
                                Cursor {
                                    document,
                                    selection: &selection,
                                    index: 0,
                                    count: 1,
                                },
                                name,
                                indent,
                            )
                        },
                        whitespace,
                    )
                } else {
                    Ok(Expansion {
                        text: normalized(text, &document.eol)?,
                        ..Expansion::default()
                    })
                }
            })?;
        self.preview_edit_barrier();
        self.message = if self.doc().in_snippet() {
            "Completion applied · Tab next · Shift+Tab previous · Esc leave"
        } else {
            "Completion applied"
        }
        .into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(root: &Path, text: &str) -> App {
        let path = root.join("main.cpp");
        std::fs::write(&path, text).unwrap();
        let mut app = App::new(root.into(), Profile::Linux);
        app.open(&path).unwrap();
        app
    }

    #[test]
    fn imports_and_unicode_crlf_fields_share_one_undo_and_preserve_other_views() {
        let directory = tempfile::tempdir().unwrap();
        let original = "α🙂 pre\r\nTAIL\r\n";
        let mut app = app(directory.path(), original);
        let id = app.doc().id;
        app.doc_mut().activate_view(2);
        let other_cursor = app.doc().len();
        app.doc_mut().move_to(other_cursor, false);
        app.doc_mut().activate_view(1);
        app.doc_mut()
            .set_selections(vec![Selection::caret(6), Selection::caret(0)]);
        let before = app.doc().selections();
        let epoch = app.doc().text_epoch();
        let item = json!({"label":"completion", "insertTextFormat":2,
            "textEdit":{"range":{"start":{"line":0,"character":4},"end":{"line":0,"character":7}},
                "newText":"${1:猫}\n$1 ${2:🙂}$0"},
            "additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},
                "newText":"#include <例>\n"}]});
        app.apply_completion_item(&item, 6).unwrap();
        let inserted = "#include <例>\r\nα🙂 猫\r\n猫 🙂\r\nTAIL\r\n";
        assert_eq!(app.doc().text.to_string(), inserted);
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        assert!(app.doc().text_epoch() > epoch);
        assert_eq!(app.doc().selected_text().as_deref(), Some("猫"));
        assert_eq!(app.doc().selections().len(), 2);
        let after = app.doc().selections();
        app.doc_mut().activate_view(2);
        assert_eq!(app.doc().cursor, inserted.chars().count());
        app.doc_mut().activate_view(1);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), original);
        assert_eq!(app.doc().selections(), before);
        assert!(!app.doc().in_snippet());
        app.doc_mut().activate_view(2);
        assert_eq!(app.doc().cursor, other_cursor);
        app.doc_mut().activate_view(1);
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), inserted);
        assert_eq!(app.doc().selections(), after);
        // Undo/redo terminates the live snippet session, as with native snippet
        // insertion. Start a fresh acceptance to exercise linked field edits.
        app.doc_mut().undo();
        app.apply_completion_item(&item, 6).unwrap();
        assert!(app.doc().in_snippet());
        app.doc_mut().insert("犬", false);
        assert_eq!(app.doc().text.to_string(), inserted.replace("猫", "犬"));
        app.doc_mut().step_snippet(false).unwrap();
        assert_eq!(app.doc().selected_text().as_deref(), Some("🙂"));
        app.doc_mut().save().unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("main.cpp")).unwrap(),
            inserted.replace("猫", "犬")
        );
    }

    #[test]
    fn malformed_or_unsupported_completion_preserves_text_selections_and_redo() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(directory.path(), "α🙂 pre\r\nTAIL\r\n");
        app.doc_mut().move_to(6, false);
        app.doc_mut().insert("!", false);
        let future = app.doc().text.clone();
        app.doc_mut().undo();
        app.doc_mut()
            .set_selections(vec![Selection::caret(6), Selection::caret(0)]);
        let original = app.doc().text.clone();
        let selections = app.doc().selections();
        let revision = app.doc().revision;
        let epoch = app.doc().text_epoch();
        let id = app.doc().id;
        let range = json!({"start":{"line":0,"character":4},"end":{"line":0,"character":7}});
        let base = json!({"label":"completion", "insertTextFormat":2,
            "textEdit":{"range":range,"newText":"${1:cat}-$1$0"}});
        let mut invalid = Vec::new();
        for edit in [
            json!({"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":2}},"newText":"bad surrogate"}),
            json!({"range":{"start":{"line":0,"character":6},"end":{"line":0,"character":7}},"newText":"overlap"}),
            json!({"range":{"start":{"line":99,"character":0},"end":{"line":99,"character":0}},"newText":"bad line"}),
            json!({"range":{"start":{"line":0,"character":7},"end":{"line":0,"character":4}},"newText":"reversed"}),
        ] {
            let mut item = base.clone();
            item["additionalTextEdits"] = json!([edit]);
            invalid.push(item);
        }
        for (key, value) in [
            ("command", json!({"command":"unsafe"})),
            ("insertTextFormat", json!(3)),
            ("insertTextMode", json!(2)),
            ("additionalTextEdits", json!({})),
        ] {
            let mut item = base.clone();
            item[key] = value;
            invalid.push(item);
        }
        for text in ["$CLIPBOARD", &"x".repeat(64 * 1024 + 1)] {
            let mut item = base.clone();
            item["textEdit"]["newText"] = json!(text);
            invalid.push(item);
        }
        let mut pair = base.clone();
        pair["textEdit"]["insert"] = range;
        invalid.push(pair);
        let mut deep = base.clone();
        deep["textEdit"]["newText"] = json!(format!("{}x{}", "${1:".repeat(65), "}".repeat(65)));
        invalid.push(deep);
        for item in invalid {
            assert!(app.apply_completion_item(&item, 6).is_err(), "{item}");
            assert_eq!(app.doc().text, original);
            assert_eq!(app.doc().selections(), selections);
            assert_eq!(app.doc().revision, revision);
            assert_eq!(app.doc().text_epoch(), epoch);
            assert_eq!(app.doc().id, id);
            assert!(!app.doc().in_snippet());
        }
        app.doc_mut().redo();
        assert_eq!(app.doc().text, future);
        app.doc_mut().undo();
        assert_eq!(app.doc().text, original);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("main.cpp")).unwrap(),
            original.to_string()
        );
    }

    #[test]
    fn completion_variables_normalize_multiline_selection_without_moving_fields() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(directory.path(), "α🙂\r\n猫\r\nTAIL\r\n");
        app.doc_mut().move_to(5, false);
        let item = json!({"label":"selection", "insertTextFormat":2,
            "textEdit":{"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":1}},
                "newText":"${1:$TM_SELECTED_TEXT}-$1-${2:🙂}$0"}});
        app.apply_completion_item(&item, 5).unwrap();
        assert_eq!(app.doc().selected_text().as_deref(), Some("α🙂\r\n猫"));
        assert_eq!(
            app.doc().text.to_string(),
            "α🙂\r\n猫-α🙂\r\n猫-🙂\r\nTAIL\r\n"
        );
        app.doc_mut().insert("犬", false);
        assert_eq!(app.doc().text.to_string(), "犬-犬-🙂\r\nTAIL\r\n");
        app.doc_mut().step_snippet(false).unwrap();
        assert_eq!(app.doc().selected_text().as_deref(), Some("🙂"));
        app.doc_mut().undo();
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "α🙂\r\n猫\r\nTAIL\r\n");
    }

    #[test]
    fn plain_completion_and_imports_restore_original_multi_selection_in_one_undo() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(directory.path(), "pre\r\nTAIL\r\n");
        app.doc_mut()
            .set_selections(vec![Selection::caret(3), Selection::caret(5)]);
        let selections = app.doc().selections();
        app.apply_completion_item(&json!({"label":"prefix", "insertText":"prefix\nline",
            "additionalTextEdits":[{"range":{"start":{"line":1,"character":4},"end":{"line":1,"character":4}},"newText":"!"}]}), 3).unwrap();
        assert_eq!(app.doc().text.to_string(), "prefix\r\nline\r\nTAIL!\r\n");
        assert_eq!(app.doc().cursor, "prefix\r\nline".chars().count());
        assert_eq!(app.doc().selections().len(), 1);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "pre\r\nTAIL\r\n");
        assert_eq!(app.doc().selections(), selections);
    }

    #[test]
    fn aggregate_edit_budget_and_oversized_expansion_reject_before_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(directory.path(), "pre\r\nTAIL\r\n");
        app.doc_mut().move_to(3, false);
        let before = app.doc().text.clone();
        let selections = app.doc().selections();
        let item = json!({"label":"prefix", "insertText":"prefix",
            "additionalTextEdits":[{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":0}},"newText":"x".repeat(MAX_EDIT_BYTES)}]});
        assert!(app.apply_completion_item(&item, 3).is_err());
        let item = json!({"label":"prefix", "additionalTextEdits": vec![json!({}); 4096]});
        assert!(app.apply_completion_item(&item, 3).is_err());
        assert_eq!(app.doc().text, before);
        assert_eq!(app.doc().selections(), selections);
        app.doc_mut().undo();
        assert_eq!(app.doc().text, before);

        let indented_directory = tempfile::tempdir().unwrap();
        let indented = format!("{}pre\r\n", " ".repeat(64 * 1024));
        let mut app = self::app(indented_directory.path(), &indented);
        let cursor = 64 * 1024 + 3;
        app.doc_mut().move_to(cursor, false);
        let epoch = app.doc().text_epoch();
        let selections = app.doc().selections();
        let item = json!({"label":"expansion", "insertTextFormat":2,
            "insertText": "${1:x}\n".repeat(32)});
        assert!(app.apply_completion_item(&item, cursor).is_err());
        assert_eq!(app.doc().text.to_string(), indented);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text_epoch(), epoch);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), indented);
    }

    #[test]
    fn rejected_completion_retains_live_linked_fields_and_original_undo() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app(directory.path(), "pre\r\n");
        app.doc_mut().move_to(3, false);
        app.apply_completion_item(
            &json!({"label":"snippet", "insertTextFormat":2,
            "insertText":"${1:猫}-$1-${2:🙂}$0"}),
            3,
        )
        .unwrap();
        let before = app.doc().text.clone();
        let selections = app.doc().selections();
        let epoch = app.doc().text_epoch();
        let item = json!({"label":"other", "insertTextFormat":2,
            "insertText":"${1:bad}$0", "additionalTextEdits":[{
                "range":{"start":{"line":99,"character":0},"end":{"line":99,"character":0}}, "newText":"bad"}]});
        let cursor = app.doc().cursor;
        assert!(app.apply_completion_item(&item, cursor).is_err());
        assert_eq!(app.doc().text, before);
        assert_eq!(app.doc().selections(), selections);
        assert_eq!(app.doc().text_epoch(), epoch);
        assert!(app.doc().in_snippet());
        app.doc_mut().insert("犬", false);
        assert_eq!(app.doc().text.to_string(), "犬-犬-🙂\r\n");
        app.doc_mut().undo();
        assert_eq!(app.doc().text, before);
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), "pre\r\n");
    }
}
