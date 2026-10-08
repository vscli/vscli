//! Emit native document/session observations for the pinned editor reference.
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vscli::{
    document::{Document, Selection},
    snippet::Template,
};

fn observe(document: &Document, action: &str) -> Value {
    let selections: Vec<_> = document
        .selections()
        .iter()
        .map(|s| json!({"anchor":s.anchor.unwrap_or(s.cursor), "cursor":s.cursor}))
        .collect();
    json!({"action": action, "text":document.text.to_string(), "selections":selections})
}

pub fn trace(cases: &[Value]) -> Result<Vec<Value>> {
    let variables = BTreeMap::from([
        ("TM_LINE_NUMBER".into(), "1".into()),
        ("TM_LINE_INDEX".into(), "0".into()),
        ("CURSOR_INDEX".into(), "0".into()),
        ("CURSOR_NUMBER".into(), "1".into()),
    ]);
    let mut observations = Vec::new();
    for fixture in cases {
        let mut doc = Document::from_text(fixture["text"].as_str().unwrap_or(""));
        doc.set_indentation(
            fixture["tabSize"].as_u64().unwrap_or(4) as usize,
            fixture["insertSpaces"].as_bool().unwrap_or(true),
        );
        if let Some(selections) = fixture["selections"].as_array() {
            doc.set_selections(
                selections
                    .iter()
                    .map(|s| Selection {
                        anchor: Some(s["anchor"].as_u64().unwrap() as usize),
                        cursor: s["cursor"].as_u64().unwrap() as usize,
                        desired_column: None,
                    })
                    .collect(),
            );
        }
        let parse = if fixture["entry"] == "command" {
            Template::parse_user
        } else {
            Template::parse
        };
        let template = parse(fixture["body"].as_str().unwrap())?;
        if fixture["resolveVariables"] == true {
            let environment = vscli::snippet::variables::Environment {
                workspace: std::env::current_dir()?,
                clipboard: None,
                language: fixture["language"].as_str().unwrap_or("plaintext").into(),
                timestamp: chrono::Local::now().fixed_offset(),
            };
            let count = doc.selections().len();
            doc.insert_snippet_command_resolved(
                &template,
                |doc, selection, index, name, indent| {
                    environment.resolve(
                        vscli::snippet::variables::Cursor {
                            document: doc,
                            selection,
                            index,
                            count,
                        },
                        name,
                        indent,
                    )
                },
            )?;
        } else if fixture["entry"] == "command" {
            doc.insert_snippet_command(&template, &variables)?;
        } else {
            doc.insert_snippet(&template, &variables)?;
        }
        let mut trace = vec![observe(&doc, "insert")];
        for step in fixture["steps"].as_array().unwrap() {
            let action = step["command"].as_str().unwrap_or("type");
            match action {
                "type" => doc.insert(step["type"].as_str().unwrap(), true),
                "jumpToNextSnippetPlaceholder" => {
                    doc.step_snippet(false)?;
                }
                "jumpToPrevSnippetPlaceholder" => {
                    doc.step_snippet(true)?;
                }
                "leaveSnippet" => doc.leave_snippet(),
                "cursorLeft" | "cursorRight" => {
                    doc.navigate_cursors(action, false, 20);
                }
                "undo" => doc.undo(),
                "redo" => doc.redo(),
                _ => anyhow::bail!("Unknown fixture action {action}"),
            }
            trace.push(observe(&doc, action));
        }
        observations.push(json!({"name": fixture["name"], "body": fixture["body"],
            "observations": trace}));
    }
    Ok(observations)
}
