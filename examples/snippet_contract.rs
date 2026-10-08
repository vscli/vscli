//! Emit native document/session observations for the pinned editor reference.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vscli::{document::Document, snippet::Template};

fn observe(document: &Document, action: &str) -> Value {
    let selections: Vec<_> = document
        .selections()
        .iter()
        .map(|s| json!({"anchor":s.anchor.unwrap_or(s.cursor), "cursor":s.cursor}))
        .collect();
    json!({"action": action, "text":document.text.to_string(), "selections":selections})
}

fn main() -> Result<()> {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../tests/vscode-reference/snippet-cases.json"))?;
    let variables = BTreeMap::from([
        ("TM_LINE_NUMBER".into(), "1".into()),
        ("TM_LINE_INDEX".into(), "0".into()),
        ("CURSOR_INDEX".into(), "0".into()),
        ("CURSOR_NUMBER".into(), "1".into()),
    ]);
    let mut observations = Vec::new();
    for fixture in cases {
        let mut doc = Document::default();
        let parse = if fixture["entry"] == "command" {
            Template::parse_user
        } else {
            Template::parse
        };
        doc.insert_snippet(&parse(fixture["body"].as_str().unwrap())?, &variables)?;
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
                "undo" => doc.undo(),
                "redo" => doc.redo(),
                _ => anyhow::bail!("Unknown fixture action {action}"),
            }
            trace.push(observe(&doc, action));
        }
        observations.push(json!({"name": fixture["name"], "body": fixture["body"],
            "observations": trace}));
    }
    if let Some(reference) = std::env::args_os().nth(1) {
        let reference: Vec<Value> = serde_json::from_slice(&std::fs::read(reference)?)?;
        ensure!(
            reference.len() == observations.len(),
            "Snippet reference cases disappeared"
        );
        for (actual, expected) in observations.iter().zip(reference) {
            ensure!(
                actual["name"] == expected["name"] && actual["body"] == expected["body"],
                "Snippet fixtures differ"
            );
            ensure!(
                actual["observations"] == expected["observations"],
                "Snippet trace differs for {}: native={}, reference={}",
                actual["name"],
                actual["observations"],
                expected["observations"]
            );
        }
    }
    println!("{}", serde_json::to_string_pretty(&observations)?);
    Ok(())
}
