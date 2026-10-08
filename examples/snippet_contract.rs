//! Emit initial snippet expansion observations for the pinned editor reference.
//! This does not yet qualify interactive placeholder sessions.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vscli::snippet::Template;

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
        let expanded = Template::parse(fixture["body"].as_str().unwrap())?.expand(&variables)?;
        let selections: Vec<_> = expanded
            .first_selections()
            .into_iter()
            .map(|range| json!({ "anchor": range.start, "cursor": range.end }))
            .collect();
        observations.push(json!({"name": fixture["name"], "body": fixture["body"],
            "observation": {"action": "insert", "text": expanded.text, "selections": selections}}));
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
                actual["observation"] == expected["observations"][0],
                "Initial snippet expansion differs for {}: native={}, reference={}",
                actual["name"],
                actual["observation"],
                expected["observations"][0]
            );
        }
    }
    println!("{}", serde_json::to_string_pretty(&observations)?);
    Ok(())
}
