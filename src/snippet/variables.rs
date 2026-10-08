//! Native snippet variables. Resolved lazily per occurrence against the original
//! document/cursor, before the insertion transaction mutates any text.
use crate::document::{Document, Selection};
use anyhow::{Result, ensure};
use chrono::{DateTime, FixedOffset};
use std::{path::PathBuf, sync::OnceLock};

pub struct Cursor<'a> {
    pub document: &'a Document,
    pub selection: &'a Selection,
    pub index: usize,
    pub count: usize,
}

pub struct Environment {
    pub workspace: PathBuf,
    pub clipboard: Option<String>,
    pub language: String,
    pub timestamp: DateTime<FixedOffset>,
}

impl Environment {
    pub fn resolve(
        &self,
        cursor: Cursor<'_>,
        name: &str,
        preceding_indent: Option<&str>,
    ) -> Result<Option<String>> {
        let Cursor {
            document: doc,
            selection,
            index,
            count,
        } = cursor;
        let row = doc.text.char_to_line(selection.cursor);
        let path = doc.path.as_deref();
        let value = match name {
            "CURSOR_INDEX" => Some(index.to_string()),
            "CURSOR_NUMBER" => Some((index + 1).to_string()),
            "TM_LINE_INDEX" => Some(row.to_string()),
            "TM_LINE_NUMBER" => Some((row + 1).to_string()),
            "TM_CURRENT_LINE" => Some(bounded_text(doc.line_slice(row))?),
            "TM_CURRENT_WORD" => {
                let line = bounded_text(doc.line_slice(row))?;
                let position = doc
                    .text
                    .slice(doc.line_start(row)..selection.cursor)
                    .len_bytes();
                word_at(&line, position)
            }
            "TM_SELECTED_TEXT" | "SELECTION" => {
                let range = selection.range();
                if range.is_empty() {
                    None
                } else {
                    let text = bounded_text(doc.text.slice(range.clone()))?;
                    if !text.contains(['\r', '\n']) {
                        return Ok(Some(text));
                    }
                    let start = doc.line_start(doc.text.char_to_line(range.start));
                    let leading: String = doc
                        .text
                        .slice(start..range.start)
                        .chars()
                        .take_while(|c| matches!(c, ' ' | '\t'))
                        .take(super::MAX_EXPANSION + 1)
                        .collect();
                    ensure!(
                        leading.len() <= super::MAX_EXPANSION,
                        "Snippet selection indentation exceeds 1 MiB"
                    );
                    let indent = preceding_indent.unwrap_or(&leading);
                    let common = leading
                        .bytes()
                        .zip(indent.bytes())
                        .take_while(|(a, b)| a == b)
                        .count();
                    Some(indent_selected(&text, &indent[common..])?)
                }
            }
            "TM_FILENAME" => Some(doc.name()),
            "TM_FILENAME_BASE" => {
                let name = doc.name();
                let end = name
                    .rfind('.')
                    .filter(|index| *index > 0)
                    .unwrap_or(name.len());
                Some(name[..end].to_owned())
            }
            "TM_FILEPATH" => Some(path.map_or_else(|| doc.name(), display_path)),
            "TM_DIRECTORY" => Some(
                path.and_then(|p| p.parent())
                    .map_or_else(String::new, display_path),
            ),
            "RELATIVE_FILEPATH" => Some(path.map_or_else(
                || doc.name(),
                |p| display_path(p.strip_prefix(&self.workspace).unwrap_or(p)),
            )),
            "WORKSPACE_NAME" => self
                .workspace
                .file_name()
                .map(|s| s.to_string_lossy().into_owned()),
            "WORKSPACE_FOLDER" => Some(display_path(&self.workspace)),
            "CLIPBOARD" => self
                .clipboard
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|text| {
                    let lines: Vec<_> = text
                        .split(['\r', '\n'])
                        .filter(|line| !line.trim().is_empty())
                        .collect();
                    if lines.len() == count {
                        lines[index].to_owned()
                    } else {
                        text.to_owned()
                    }
                }),
            "LINE_COMMENT" => comment_tokens(&self.language).0.map(str::to_owned),
            "BLOCK_COMMENT_START" => comment_tokens(&self.language).1.map(str::to_owned),
            "BLOCK_COMMENT_END" => comment_tokens(&self.language).2.map(str::to_owned),
            "CURRENT_YEAR" => Some(self.timestamp.format("%Y").to_string()),
            "CURRENT_YEAR_SHORT" => Some(self.timestamp.format("%y").to_string()),
            "CURRENT_MONTH" => Some(self.timestamp.format("%m").to_string()),
            "CURRENT_DATE" => Some(self.timestamp.format("%d").to_string()),
            "CURRENT_HOUR" => Some(self.timestamp.format("%H").to_string()),
            "CURRENT_MINUTE" => Some(self.timestamp.format("%M").to_string()),
            "CURRENT_SECOND" => Some(self.timestamp.format("%S").to_string()),
            "CURRENT_DAY_NAME" => Some(self.timestamp.format("%A").to_string()),
            "CURRENT_DAY_NAME_SHORT" => Some(self.timestamp.format("%a").to_string()),
            "CURRENT_MONTH_NAME" => Some(self.timestamp.format("%B").to_string()),
            "CURRENT_MONTH_NAME_SHORT" => Some(self.timestamp.format("%b").to_string()),
            "CURRENT_SECONDS_UNIX" => Some(self.timestamp.timestamp().to_string()),
            "CURRENT_TIMEZONE_OFFSET" => Some(self.timestamp.format("%:z").to_string()),
            "RANDOM" => Some(format!("{:06}", uuid::Uuid::new_v4().as_u128() % 1_000_000)),
            "RANDOM_HEX" => Some(format!(
                "{:06x}",
                uuid::Uuid::new_v4().as_u128() & 0xff_ffff
            )),
            "UUID" => Some(uuid::Uuid::new_v4().to_string()),
            _ => None,
        };
        if let Some(value) = &value {
            ensure!(
                value.len() <= super::MAX_EXPANSION,
                "Snippet variable {name} exceeds 1 MiB"
            );
        }
        Ok(value)
    }
}

fn display_path(path: &std::path::Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else {
        path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
    }
}

fn bounded_text(text: ropey::RopeSlice<'_>) -> Result<String> {
    ensure!(
        text.len_bytes() <= super::MAX_EXPANSION,
        "Snippet variable exceeds 1 MiB"
    );
    Ok(text.to_string())
}

fn indent_selected(text: &str, extra: &str) -> Result<String> {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        output.push(ch);
        if ch == '\r' && chars.peek() == Some(&'\n') {
            output.push(chars.next().unwrap());
        }
        if matches!(ch, '\r' | '\n') {
            output.push_str(extra);
        }
        ensure!(
            output.len() <= super::MAX_EXPANSION,
            "Selected snippet text exceeds 1 MiB"
        );
    }
    Ok(output)
}

fn word_at(line: &str, position: usize) -> Option<String> {
    static WORD: OnceLock<regex::Regex> = OnceLock::new();
    let word = WORD.get_or_init(|| {
        let separators = regex::escape("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?");
        regex::Regex::new(&format!(
            r"(-?[0-9]*\.[0-9][A-Za-z0-9_]*)|([^{separators}\s]+)"
        ))
        .unwrap()
    });
    word.find_iter(line)
        .take_while(|word| word.start() <= position)
        .find(|word| word.start() <= position && position <= word.end())
        .map(|word| word.as_str().to_owned())
}

fn comment_tokens(
    language: &str,
) -> (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
) {
    match language {
        "rust" | "c" | "cpp" | "javascript" | "typescript" | "go" | "json" | "jsonc" => {
            (Some("//"), Some("/*"), Some("*/"))
        }
        "python" => (Some("#"), Some("\"\"\""), Some("\"\"\"")),
        "shellscript" => (Some("#"), None, None),
        "css" => (None, Some("/*"), Some("*/")),
        "html" | "markdown" => (None, Some("<!--"), Some("-->")),
        _ => (None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_clock_random_and_clipboard_variables_are_native_and_bounded() {
        let workspace = std::env::current_dir().unwrap();
        let mut doc = Document::from_text("one\ntwo\n");
        doc.path = Some(workspace.join("src").join("hello.world.rs"));
        let selection = Selection::caret(4);
        let mut environment = Environment {
            workspace: workspace.clone(),
            clipboard: Some(" red\r\n\n blue \n".into()),
            language: "rust".into(),
            timestamp: DateTime::parse_from_rfc3339("2024-02-29T13:05:07+05:30").unwrap(),
        };
        let read = |environment: &Environment, name: &str| {
            environment
                .resolve(
                    Cursor {
                        document: &doc,
                        selection: &selection,
                        index: 1,
                        count: 2,
                    },
                    name,
                    None,
                )
                .unwrap()
                .unwrap()
        };
        for (name, expected) in [
            ("TM_FILENAME", "hello.world.rs"),
            ("TM_FILENAME_BASE", "hello.world"),
            ("TM_LINE_NUMBER", "2"),
            ("CURSOR_NUMBER", "2"),
            ("CLIPBOARD", " blue "),
            ("CURRENT_YEAR", "2024"),
            ("CURRENT_YEAR_SHORT", "24"),
            ("CURRENT_MONTH", "02"),
            ("CURRENT_DATE", "29"),
            ("CURRENT_HOUR", "13"),
            ("CURRENT_MINUTE", "05"),
            ("CURRENT_SECOND", "07"),
            ("CURRENT_DAY_NAME", "Thursday"),
            ("CURRENT_DAY_NAME_SHORT", "Thu"),
            ("CURRENT_MONTH_NAME", "February"),
            ("CURRENT_MONTH_NAME_SHORT", "Feb"),
            ("CURRENT_TIMEZONE_OFFSET", "+05:30"),
        ] {
            assert_eq!(read(&environment, name), expected, "{name}");
        }
        assert_eq!(
            read(&environment, "TM_FILEPATH"),
            display_path(doc.path.as_ref().unwrap())
        );
        assert_eq!(
            read(&environment, "RELATIVE_FILEPATH"),
            display_path(&PathBuf::from("src").join("hello.world.rs"))
        );
        assert_eq!(
            read(&environment, "CURRENT_SECONDS_UNIX"),
            environment.timestamp.timestamp().to_string()
        );
        let decimal = read(&environment, "RANDOM");
        assert_eq!(decimal.len(), 6);
        assert!(decimal.bytes().all(|c| c.is_ascii_digit()));
        let hex = read(&environment, "RANDOM_HEX");
        assert_eq!(hex.len(), 6);
        assert!(hex.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            uuid::Uuid::parse_str(&read(&environment, "UUID"))
                .unwrap()
                .get_version_num(),
            4
        );
        environment.clipboard = Some("x".repeat(super::super::MAX_EXPANSION + 1));
        assert!(
            environment
                .resolve(
                    Cursor {
                        document: &doc,
                        selection: &selection,
                        index: 0,
                        count: 1
                    },
                    "CLIPBOARD",
                    None
                )
                .is_err()
        );
    }

    #[test]
    fn occurrence_resolution_and_late_failure_are_atomic() {
        let template =
            crate::snippet::Template::parse_user("${1:$UUID} / $UUID / ${2:next}$0").unwrap();
        let mut doc = Document::from_text("cat cat");
        doc.set_selections(vec![
            Selection {
                cursor: 3,
                anchor: Some(0),
                desired_column: None,
            },
            Selection {
                cursor: 7,
                anchor: Some(4),
                desired_column: None,
            },
        ]);
        let mut calls = 0;
        let original = doc.text.clone();
        let selections = doc.selections();
        let revision = doc.revision;
        let result = doc.insert_snippet_command_resolved(&template, |_, _, _, name, _| {
            assert_eq!(name, "UUID");
            calls += 1;
            anyhow::ensure!(calls < 4, "fixture resolver failure");
            Ok(Some(calls.to_string()))
        });
        assert!(result.is_err());
        assert_eq!(calls, 4);
        assert_eq!(doc.text, original);
        assert_eq!(doc.selections(), selections);
        assert_eq!(doc.revision, revision);
        doc.undo();
        assert_eq!(doc.text, original);
        calls = 0;
        doc.insert_snippet_command_resolved(&template, |_, _, _, _, _| {
            calls += 1;
            Ok(Some(calls.to_string()))
        })
        .unwrap();
        assert_eq!(doc.text.to_string(), "1 / 2 / next 3 / 4 / next");
        doc.undo();
        assert_eq!(doc.text, original);
        assert_eq!(doc.selections(), selections);
    }
}
