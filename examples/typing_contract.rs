//! Compare native typing gestures with observations from the pinned executable.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use vscli::document::{Document, Selection};

fn snapshot(document: &Document, action: &str) -> Value {
    json!({"action":action, "text":document.text.to_string(),
        "selections":document.selections().iter().map(|selection| json!({
            "anchor":selection.anchor.unwrap_or(selection.cursor),
            "cursor":selection.cursor,
        })).collect::<Vec<_>>()})
}

fn trace(cases: &[Value]) -> Result<Vec<Value>> {
    let mut traces = Vec::new();
    for fixture in cases {
        let mut document = Document::from_text(fixture["text"].as_str().context("Missing text")?);
        document.set_indentation(4, true);
        let options = vscli::settings::Settings::default()
            .typing(fixture["language"].as_str().context("Missing language")?);
        if let Some(selections) = fixture["selections"].as_array() {
            let mut values = Vec::new();
            for selection in selections {
                let cursor =
                    usize::try_from(selection["cursor"].as_u64().context("Missing cursor")?)?;
                let anchor =
                    usize::try_from(selection["anchor"].as_u64().context("Missing anchor")?)?;
                ensure!(
                    cursor <= document.text.len_chars() && anchor <= document.text.len_chars(),
                    "Selection outside fixture"
                );
                values.push(Selection {
                    cursor,
                    anchor: (anchor != cursor).then_some(anchor),
                    desired_column: None,
                });
            }
            let primary = values.first().context("No primary selection")?;
            document.cursor = primary.cursor;
            document.anchor = primary.anchor;
            document.secondary = values.into_iter().skip(1).collect();
        }
        let mut observations = vec![snapshot(&document, "initial")];
        for step in fixture["steps"].as_array().context("Missing steps")? {
            let action = if let Some(text) = step["type"].as_str() {
                if text == "\n" {
                    document.newline_with_options(options)?;
                } else {
                    let mut characters = text.chars();
                    let character = characters.next().context("Empty typed gesture")?;
                    ensure!(
                        characters.next().is_none(),
                        "Typing fixture must use one scalar gesture"
                    );
                    document.type_character(character, options, false)?;
                }
                "type"
            } else {
                let command = step["command"].as_str().context("Missing command")?;
                match command {
                    "undo" => document.undo(),
                    "redo" => document.redo(),
                    "deleteLeft" => document.backspace_with_options(options, false)?,
                    "lineBreakInsert" => document.line_break_with_options(options)?,
                    _ => bail!("Unsupported typing reference command: {command}"),
                }
                command
            };
            observations.push(snapshot(&document, action));
        }
        traces.push(json!({"name":fixture["name"],"observations":observations}));
    }
    Ok(traces)
}

fn main() -> Result<()> {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../tests/vscode-reference/typing-cases.json"))?;
    let observations = trace(&cases)?;
    if let Some(reference) = std::env::args_os().nth(1) {
        let reference: Vec<Value> = serde_json::from_slice(&std::fs::read(reference)?)?;
        ensure!(
            reference.len() == observations.len(),
            "Typing reference cases disappeared"
        );
        for (actual, expected) in observations.iter().zip(reference) {
            ensure!(
                actual["name"] == expected["name"],
                "Typing fixture ordering differs"
            );
            ensure!(
                actual["observations"] == expected["observations"],
                "Typing trace differs for {}: native={}, reference={}",
                actual["name"],
                actual["observations"],
                expected["observations"]
            );
        }
    }
    println!("{}", serde_json::to_string_pretty(&observations)?);
    Ok(())
}
