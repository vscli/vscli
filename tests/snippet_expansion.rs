//! Recorded native/API expansion comparisons; CI also reruns the actual pinned
//! editor on each platform. Interactive traces are recorded but not yet gated.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use vscli::snippet::Template;

#[test]
fn initial_expansion_matches_thirty_pinned_editor_observations() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("vscode-reference/snippet-cases.json")).unwrap();
    let expected: Vec<Value> = serde_json::from_str(include_str!(
        "vscode-reference/baselines/1.95.0/snippets/linux.json"
    ))
    .unwrap();
    assert_eq!(fixtures.len(), 30);
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
        let result = Template::parse(fixture["body"].as_str().unwrap())
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
