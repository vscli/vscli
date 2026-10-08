//! Recorded native/API expansion comparisons; CI also reruns the actual pinned
//! editor on each platform. Complete captured document/session traces are gated.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vscli::{document::Document, snippet::Template};

#[test]
fn initial_expansion_matches_pinned_editor_observations() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("vscode-reference/snippet-cases.json")).unwrap();
    let expected: Vec<Value> = serde_json::from_str(include_str!(
        "vscode-reference/baselines/1.95.0/snippets/linux.json"
    ))
    .unwrap();
    assert_eq!(fixtures.len(), 34);
    assert_eq!(expected.len(), fixtures.len());
    let variables = BTreeMap::from([
        ("TM_LINE_NUMBER".into(), "1".into()),
        ("TM_LINE_INDEX".into(), "0".into()),
        ("CURSOR_INDEX".into(), "0".into()),
        ("CURSOR_NUMBER".into(), "1".into()),
    ]);
    for (fixture, expected) in fixtures.iter().zip(expected) {
        assert_eq!(fixture["name"], expected["name"]);
        assert_eq!(fixture["body"], expected["body"]);
        let parse = if fixture["entry"] == "command" {
            Template::parse_user
        } else {
            Template::parse
        };
        let result = parse(fixture["body"].as_str().unwrap())
            .unwrap()
            .expand(&variables)
            .unwrap();
        let selections: Vec<_> = result
            .first_selections()
            .into_iter()
            .map(|r| json!({"anchor":r.start,"cursor":r.end}))
            .collect();
        assert_eq!(
            json!({"action":"insert", "text":result.text,"selections":selections}),
            expected["observations"][0],
            "{}",
            fixture["name"]
        );
    }
}

#[test]
fn document_sessions_match_complete_reference_traces() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("vscode-reference/snippet-cases.json")).unwrap();
    let expected: Vec<Value> = serde_json::from_str(include_str!(
        "vscode-reference/baselines/1.95.0/snippets/linux.json"
    ))
    .unwrap();
    let variables = BTreeMap::from([
        ("TM_LINE_NUMBER".into(), "1".into()),
        ("TM_LINE_INDEX".into(), "0".into()),
        ("CURSOR_INDEX".into(), "0".into()),
        ("CURSOR_NUMBER".into(), "1".into()),
    ]);
    fn observe(doc: &Document, action: &str) -> Value {
        let selections: Vec<_> = doc
            .selections()
            .iter()
            .map(|s| json!({"anchor":s.anchor.unwrap_or(s.cursor), "cursor":s.cursor}))
            .collect();
        json!({"action":action, "text":doc.text.to_string(), "selections":selections})
    }
    assert_eq!(cases.len(), expected.len());
    for (case, expected) in cases.iter().zip(expected) {
        let parse = if case["entry"] == "command" {
            Template::parse_user
        } else {
            Template::parse
        };
        let mut doc = Document::default();
        doc.insert_snippet(&parse(case["body"].as_str().unwrap()).unwrap(), &variables)
            .unwrap();
        let mut trace = vec![observe(&doc, "insert")];
        for step in case["steps"].as_array().unwrap() {
            let action = step["command"].as_str().unwrap_or("type");
            match action {
                "type" => doc.insert(step["type"].as_str().unwrap(), true),
                "jumpToNextSnippetPlaceholder" => {
                    doc.step_snippet(false).unwrap();
                }
                "jumpToPrevSnippetPlaceholder" => {
                    doc.step_snippet(true).unwrap();
                }
                "undo" => doc.undo(),
                "redo" => doc.redo(),
                _ => panic!("Unexpected action {action}"),
            }
            trace.push(observe(&doc, action));
        }
        assert_eq!(json!(trace), expected["observations"], "{}", case["name"]);
    }
}
