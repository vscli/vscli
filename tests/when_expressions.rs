use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use vscli::keys::{Keymap, Profile, Resolution, evaluate};
#[test]
fn native_conditions_match_pinned_upstream_parser_for_boolean_membership_numeric_and_regex_cases() {
    let expressions = [
        "flag",
        "!flag",
        "!unknown",
        "value == true",
        "value == false",
        "value != true",
        "value != false",
        "value == 'true'",
        "value == 'false'",
        "value === 1",
        "value !== 0",
        "value == ''",
        "value == undefined",
        "value == null",
        "count > 2",
        "count >= 2.5",
        "count < 8",
        "count <= '3px'",
        "count >= 0x10",
        "count > invalid",
        "item in items",
        "item not in items",
        "item in lookup",
        "item not in lookup",
        "!(item in items)",
        "flag && count >= 2 || !flag",
        "!(flag && unknown)",
        "name =~ /rust/i",
        "name =~ /^rust$/",
        "name =~ /^rust$/giy",
        "name =~ /a.b/s",
        "name =~ /\\d+/",
        "name =~ /\\w+/",
        "name =~ /\\bRust\\b/",
        "name =~ /[a-z]+/i",
        "name =~ /^(rust|cpp)$/i",
        "value =~ /^1$/",
        "count>2",
        "value ==",
        "value == && flag",
    ];
    let mut contexts = vec![
        json!({}),
        json!({"flag":false,"value":0,"count":0,"name":"RUST","item":"cpp","items":["cpp"],"lookup":{"cpp":false}}),
        json!({"flag":true,"value":true,"count":"3px","name":"rust","item":0,"items":[false,0],"lookup":{"0":true}}),
        json!({"flag":true,"value":false,"count":2.5,"name":"a\nb","item":false,"items":[false],"lookup":{"false":true}}),
    ];
    for value in [
        json!(null),
        json!(""),
        json!("true"),
        json!("false"),
        json!("0"),
        json!(1.0),
        json!(-0.0),
        json!([]),
        json!([1]),
        json!({}),
        json!([null]),
    ] {
        contexts.push(json!({"flag":true,"value":value,"count":"  +2.5e1tail","name":"λRust λ12","item":"file:///C:/A","items":["file:///c:/a"],"lookup":{"file:///c:/a":null}}));
    }
    let cases: Vec<_> = contexts
        .iter()
        .flat_map(|context| {
            expressions
                .iter()
                .map(move |source| json!({"source":source,"context":context}))
        })
        .collect();
    let runner =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upstream/context-key-oracle/run.cjs");
    let mut child = Command::new("node")
        .arg(runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&cases).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(original.len(), cases.len());
    for (case, original) in cases.iter().zip(original) {
        let context: HashMap<String, Value> =
            serde_json::from_value(case["context"].clone()).unwrap();
        let source = case["source"].as_str().unwrap();
        let native = evaluate(source, &context);
        assert_eq!(
            native.is_ok(),
            original["valid"] == true,
            "{case}: native={native:?}; original={original}"
        );
        if let Ok(native) = native {
            assert_eq!(
                native,
                original["value"].as_bool().unwrap(),
                "{case}; original={original}"
            );
        }
    }
    println!(
        "Compared {} exact upstream/native context evaluations",
        cases.len()
    );
}
#[test]
fn imported_complex_rules_preserve_source_priority_removals_and_unsupported_regex_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("keybindings.json");
    let source = r#"[// original comments
 {"key":"f6","command":"type","args":{"text":"ready"},"when":"enabled && mode in modes && count >= 2 && name =~ /^Rust$/i"},
 {"key":"f7","command":"type","args":{"text":"bad"},"when":"name =~ /(?=unsafe)/"},
 {"key":"ctrl+s","command":"-workbench.action.files.save"},
 {"key":"ctrl+s","command":"type","args":{"text":"override"},"when":"mode not in blocked"}
 ]"#;
    std::fs::write(&path, source).unwrap();
    let mut map = Keymap::new(Profile::Linux);
    let (count, notices) = map.load_imported(&path).unwrap();
    assert_eq!(count, 3);
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("regular expression"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    let mut context = HashMap::from([
        ("enabled".into(), json!(true)),
        ("mode".into(), json!("cpp")),
        ("modes".into(), json!(["cpp", "rust"])),
        ("count".into(), json!(2)),
        ("name".into(), json!("RUST")),
        ("blocked".into(), json!({"cpp":false})),
    ]);
    assert!(matches!(map.resolve("f6",&context),Resolution::Command(id,_)if id=="type"));
    assert_eq!(map.resolve("ctrl+s", &context), Resolution::None);
    context.insert("blocked".into(), json!({}));
    assert!(matches!(map.resolve("ctrl+s",&context),Resolution::Command(id,_)if id=="type"));
    context.insert("enabled".into(), json!(0));
    assert_eq!(map.resolve("f6", &context), Resolution::None);
}
