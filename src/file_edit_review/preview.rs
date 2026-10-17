//! Worker-prepared removed/replacement ranges. Display never loads a document.
use super::dialog::plain;
use crate::{
    document::Document,
    resource_journal::Control,
    workspace_edit_stream::{Entry, Stream},
};
use anyhow::{Context, Result, ensure};
use ropey::Rope;
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) type Preview = BTreeMap<usize, Vec<String>>;
pub(crate) fn prepare(
    original: &Value,
    mut sources: Vec<(String, Rope)>,
    control: &Control<'_>,
) -> Result<Preview> {
    let stream = Stream::parse(original)?;
    let mut bytes = sources
        .iter()
        .map(|(_, text)| text.len_bytes())
        .sum::<usize>();
    ensure!(
        sources.len() <= 128 && bytes <= 4 * 1024 * 1024,
        "Preview source budget exceeded"
    );
    let mut result = Preview::new();
    let mut resolved: BTreeMap<String, usize> = sources
        .iter()
        .enumerate()
        .map(|(index, (uri, _))| (uri.clone(), index))
        .collect();
    let mut display_bytes = 0;
    for index in 0..stream.len() {
        control.check()?;
        let Entry::Text(text) = stream.entry(index)? else {
            continue;
        };
        let Some(range) = text.edit.get("range") else {
            continue;
        };
        let preview = (|| -> Result<Vec<String>> {
            let position = if let Some(position) = resolved.get(text.resource) {
                *position
            } else {
                let path = crate::workspace_resource_edits::local_path(text.resource)?;
                let canonical = std::fs::canonicalize(&path)?;
                control.check()?;
                // Resolve a URI alias to its retained dirty source before disk.
                let mut alias = None;
                for (index, (uri, _)) in sources.iter().enumerate() {
                    control.check()?;
                    if crate::workspace_resource_edits::local_path(uri).is_ok_and(|path| {
                        std::fs::canonicalize(path).is_ok_and(|path| path == canonical)
                    }) {
                        alias = Some(index);
                        break;
                    }
                }
                let position = if let Some(position) = alias {
                    position
                } else {
                    ensure!(
                        sources.len() < 128 && bytes < 4 * 1024 * 1024,
                        "Preview source budget exceeded"
                    );
                    let document = Document::open_existing_bounded(
                        &canonical,
                        (4 * 1024 * 1024 - bytes) as u64,
                    )?;
                    bytes += document.text.len_bytes();
                    sources.push((text.resource.to_owned(), document.text));
                    sources.len() - 1
                };
                resolved.insert(text.resource.to_owned(), position);
                position
            };
            let source = &sources[position].1;
            let start = offset(source, range, "startLineNumber", "startColumn")?;
            let end = offset(source, range, "endLineNumber", "endColumn")?;
            ensure!(start <= end, "Preview range is reversed");
            let mut lines = vec!["Original range (-) and proposed replacement (+):".into()];
            let mut clipped = false;
            for (prefix, content) in [
                ('-', source.slice(start..end)),
                (
                    '+',
                    ropey::RopeSlice::from(
                        text.edit
                            .get("text")
                            .and_then(Value::as_str)
                            .context("Missing replacement")?,
                    ),
                ),
            ] {
                for (line, value) in content.lines().enumerate() {
                    control.check()?;
                    if line >= 48 || display_bytes >= 1024 * 1024 {
                        clipped = true;
                        break;
                    }
                    let rendered = plain(&value.chars().take(241).collect::<String>(), 240);
                    clipped |= value.len_chars() > 240;
                    let rendered = format!("{prefix} {rendered}");
                    display_bytes += rendered.len();
                    lines.push(rendered);
                }
            }
            if clipped {
                lines.push("Preview clipped; complete original edit is retained.".into());
            }
            Ok(lines)
        })();
        result.insert(
            index,
            preview.unwrap_or_else(|error| {
                vec![format!(
                    "Range preview unavailable: {}",
                    plain(&error.to_string(), 200)
                )]
            }),
        );
    }
    control.check()?;
    Ok(result)
}
fn offset(text: &Rope, range: &Value, line: &str, column: &str) -> Result<usize> {
    let read = |name| -> Result<usize> {
        let value = range
            .get(name)
            .and_then(Value::as_u64)
            .context("Invalid preview coordinate")?;
        ensure!(
            value > 0 && value <= i32::MAX as u64,
            "Preview coordinate outside bounds"
        );
        Ok(value as usize - 1)
    };
    let line = read(line)?;
    let column = read(column)?;
    ensure!(line < text.len_lines(), "Preview line outside document");
    let value = text.line(line);
    let end = value.len_chars()
        - value
            .chars_at(value.len_chars())
            .reversed()
            .take_while(|ch| matches!(ch, '\r' | '\n'))
            .count();
    let logical = value.slice(..end);
    ensure!(
        column <= logical.len_utf16_cu(),
        "Preview column crosses logical EOL"
    );
    let offset = logical.utf16_cu_to_char(column);
    ensure!(
        logical.char_to_utf16_cu(offset) == column,
        "Preview range splits UTF-16 surrogate"
    );
    Ok(text.line_to_char(line) + offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };
    #[test]
    fn retained_dirty_utf16_range_is_previewed_instead_of_disk_with_bounded_inert_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.cpp");
        std::fs::write(&path, "disk text\r\n").unwrap();
        let uri = url::Url::from_file_path(&path).unwrap().to_string();
        let original = json!({"edits":[{"resource":uri,"textEdit":{
            "range":{"startLineNumber":1,"startColumn":1,"endLineNumber":1,"endColumn":5},
            "text":"replacement \u{001b}[31m\r\n"}}]});
        let canceled = AtomicBool::new(false);
        let preview = prepare(
            &original,
            vec![(uri, Rope::from_str("猫🙂x dirty\r\n"))],
            &Control::new(&canceled, Instant::now() + Duration::from_secs(2)),
        )
        .unwrap();
        assert!(preview[&0].iter().any(|line| line == "- 猫🙂x"));
        assert!(
            preview[&0]
                .iter()
                .any(|line| line.contains("+ replacement �[31m\\r\\n"))
        );
        assert!(!preview[&0].iter().any(|line| line.contains('\u{001b}')));
        assert_eq!(std::fs::read(path).unwrap(), b"disk text\r\n");
    }
    #[test]
    fn invalid_surrogate_boundary_and_excess_display_do_not_manufacture_a_complete_preview() {
        let uri = "file:///missing/source.cpp";
        let original = json!({"edits":[{"resource":uri,"textEdit":{
            "range":{"startLineNumber":1,"startColumn":2,"endLineNumber":1,"endColumn":3},"text":"replacement"}},
            {"resource":uri,"textEdit":{"range":{"startLineNumber":1,"startColumn":1,"endLineNumber":1,"endColumn":3},"text":"large\n".repeat(100)}}]});
        let canceled = AtomicBool::new(false);
        let preview = prepare(
            &original,
            vec![(uri.into(), Rope::from_str("🙂\r\n"))],
            &Control::new(&canceled, Instant::now() + Duration::from_secs(2)),
        )
        .unwrap();
        assert!(preview[&0][0].contains("splits UTF-16 surrogate"));
        assert!(preview[&1].last().unwrap().contains("Preview clipped"));
        assert!(preview[&1].len() <= 100);
    }
}
