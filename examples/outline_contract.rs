//! Narrow actual provider-geometry and confirmed Outline reveal qualification.
//! The complete 20 desktop snapshots are retained, not all compared to native UI.
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Focus, OutlineStatus},
    keys::Profile,
    lsp::Client,
};

const CASES: &str = include_str!("../tests/vscode-reference/outline-cases.json");
const DEFAULT_REFERENCE: &str = "tests/vscode-reference/baselines/1.95.0/outline/linux.json";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Independent reference coordinate check; no native parser participates in
// artifact qualification or malformed-reference tests.
fn reference_offset(text: &str, position: &Value) -> Result<usize> {
    let line = usize::try_from(
        position["line"]
            .as_u64()
            .context("Missing reference line")?,
    )?;
    let column = usize::try_from(
        position["character"]
            .as_u64()
            .context("Missing reference UTF-16 column")?,
    )?;
    let lines = text.split('\n').collect::<Vec<_>>();
    let content = lines
        .get(line)
        .context("Reference symbol position is outside text")?
        .trim_end_matches('\r');
    let start = lines
        .iter()
        .take(line)
        .map(|line| line.chars().count() + 1)
        .sum::<usize>();
    let mut units = 0;
    for (index, character) in content.chars().enumerate() {
        if units == column {
            return Ok(start + index);
        }
        units += character.len_utf16();
        ensure!(
            units <= column,
            "Reference symbol position splits a UTF-16 scalar"
        );
    }
    ensure!(units == column, "Reference symbol position is outside text");
    Ok(start + content.chars().count())
}
fn geometry(
    symbols: &[Value],
    text: &str,
    parent: Option<usize>,
    depth: usize,
    out: &mut Vec<Value>,
) -> Result<()> {
    ensure!(depth <= 16, "Reference symbol hierarchy exceeds 16 levels");
    for symbol in symbols {
        ensure!(
            out.len() < 512,
            "Reference symbol inventory exceeds 512 nodes"
        );
        let name = symbol["name"]
            .as_str()
            .context("Missing reference symbol name")?;
        ensure!(
            !name.is_empty() && name.len() <= 4096,
            "Invalid reference symbol name"
        );
        let kind = symbol["kind"]
            .as_u64()
            .filter(|kind| (1..=26).contains(kind))
            .context("Invalid reference symbol kind")?;
        let range = &symbol["range"];
        let selection = &symbol["selectionRange"];
        let start = reference_offset(text, &range["start"])?;
        let end = reference_offset(text, &range["end"])?;
        let selected_start = reference_offset(text, &selection["start"])?;
        let selected_end = reference_offset(text, &selection["end"])?;
        ensure!(
            start <= selected_start && selected_start <= selected_end && selected_end <= end,
            "Reference symbol selection lies outside its range"
        );
        if let Some(parent) = parent {
            let outer = &out[parent]["range"];
            ensure!(
                reference_offset(text, &outer["start"])? <= start
                    && end <= reference_offset(text, &outer["end"])?,
                "Reference symbol child lies outside parent"
            );
        }
        let index = out.len();
        out.push(json!({"name":name,"kind":kind,"range":range,"selectionRange":selection,"parent":parent,"depth":depth}));
        geometry(
            symbol["children"]
                .as_array()
                .context("Missing reference symbol children")?,
            text,
            Some(index),
            depth + 1,
            out,
        )?;
    }
    Ok(())
}
fn validate_reference(
    trace_bytes: &[u8],
    evidence_bytes: &[u8],
    provenance: &Value,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let cases: Vec<Value> = serde_json::from_str(CASES)?;
    let trace: Vec<Value> = serde_json::from_slice(trace_bytes)?;
    let evidence: Vec<Value> = serde_json::from_slice(evidence_bytes)?;
    ensure!(
        provenance["version"] == "1.95.0"
            && provenance["commit"] == "912bb683695358a54ae0c670461738984cbb5b95",
        "Outline reference product differs"
    );
    ensure!(
        provenance["traceSha256"] == hash(trace_bytes),
        "Outline reference trace hash differs"
    );
    ensure!(
        provenance["evidenceSha256"] == hash(evidence_bytes),
        "Outline reference evidence hash differs"
    );
    ensure!(
        provenance["casesSha256"] == hash(CASES.as_bytes()),
        "Outline reference cases hash differs"
    );
    ensure!(
        cases.len() == 2
            && trace.len() == 2
            && evidence.len() == 2
            && provenance["caseCount"] == 2
            && provenance["snapshotCount"] == 20,
        "Outline reference case inventory differs"
    );
    let sources: &[(&str, &[u8])] = &[
        (
            "outline.cjs",
            include_bytes!("../tests/vscode-reference/outline.cjs"),
        ),
        ("outline-cases.json", CASES.as_bytes()),
        (
            "outline-suite.cjs",
            include_bytes!("../tests/vscode-reference/outline-suite.cjs"),
        ),
        (
            "outline-run.cjs",
            include_bytes!("../tests/vscode-reference/outline-run.cjs"),
        ),
        (
            "outline-worker.cjs",
            include_bytes!("../tests/vscode-reference/outline-worker.cjs"),
        ),
        (
            "supervisor.cjs",
            include_bytes!("../tests/vscode-reference/supervisor.cjs"),
        ),
        (
            "package-lock.json",
            include_bytes!("../tests/vscode-reference/package-lock.json"),
        ),
        (
            "package.json",
            include_bytes!("../tests/vscode-reference/package.json"),
        ),
        (
            "extension.cjs",
            include_bytes!("../tests/vscode-reference/extension.cjs"),
        ),
    ];
    let runs = provenance["runs"]
        .as_array()
        .context("Missing reference runs")?;
    ensure!(
        runs.len() == cases.len(),
        "Outline reference run inventory differs"
    );
    let mut count = 0;
    for (((fixture, projected), raw), run) in cases.iter().zip(&trace).zip(&evidence).zip(runs) {
        ensure!(
            fixture["name"] == projected["name"]
                && fixture["name"] == raw["name"]
                && fixture["name"] == run["name"],
            "Outline reference run identity differs"
        );
        ensure!(
            run["version"] == provenance["version"]
                && run["commit"] == provenance["commit"]
                && run["platform"] == provenance["platform"]
                && run["architecture"] == provenance["architecture"],
            "Outline reference run product differs"
        );
        let recorded = run["sources"]
            .as_object()
            .context("Missing captured sources")?;
        ensure!(
            recorded.len() == sources.len(),
            "Outline source inventory differs"
        );
        for (name, bytes) in sources {
            ensure!(
                recorded.get(*name) == Some(&json!(hash(bytes))),
                "Captured Outline source differs: {name}"
            );
        }
        let text = fixture["text"].as_str().context("Missing compiled text")?;
        ensure!(
            raw["setup"]["shape"] == fixture["shape"],
            "Outline API shape differs"
        );
        if fixture["setupOutlineFocus"] == true {
            ensure!(
                raw["setup"]["outlineFocusBeforeRegistration"] == true
                    && raw["setup"]["callbacksBeforeApiOrTargets"]
                        .as_u64()
                        .is_some_and(|count| count > 0),
                "Outline UI readiness proof is missing"
            );
        }
        let inventory = raw["commandInventory"]
            .as_array()
            .context("Missing original command inventory")?;
        let unique = inventory
            .iter()
            .filter_map(Value::as_str)
            .collect::<std::collections::HashSet<_>>();
        ensure!(
            unique.len() == inventory.len(),
            "Outline original command inventory contains duplicates"
        );
        let commands = fixture["commands"]
            .as_array()
            .context("Missing compiled original commands")?;
        for command in commands {
            ensure!(
                unique.contains(command.as_str().context("Invalid compiled command")?),
                "Outline original command is missing"
            );
        }
        let requests = raw["requests"]
            .as_array()
            .context("Missing provider callback observations")?;
        ensure!(
            !requests.is_empty()
                && requests.len() <= 128
                && requests
                    .iter()
                    .all(|request| request["sameDocument"] == true
                        && request["version"] == 1
                        && request["cancelledAtInvocation"] == false),
            "Outline callback identity differs"
        );
        ensure!(
            projected["actualSymbols"] == raw["actualSymbols"],
            "Outline API geometry projection differs"
        );
        let mut nodes = Vec::new();
        geometry(
            projected["actualSymbols"]
                .as_array()
                .context("Missing observed API symbols")?,
            text,
            None,
            0,
            &mut nodes,
        )?;
        ensure!(!nodes.is_empty(), "Outline API geometry disappeared");
        let observations = projected["observations"]
            .as_array()
            .context("Missing projected observations")?;
        let raw_observations = raw["observations"]
            .as_array()
            .context("Missing raw observations")?;
        let steps = raw["steps"]
            .as_array()
            .context("Missing original gesture evidence")?;
        ensure!(
            observations.len() == commands.len() + 1
                && raw_observations.len() == observations.len()
                && steps.len() == commands.len(),
            "Outline gesture inventory differs"
        );
        count += observations.len();
        for (index, (observed, raw_observed)) in
            observations.iter().zip(raw_observations).enumerate()
        {
            let action = if index == 0 {
                &json!("initial")
            } else {
                &commands[index - 1]
            };
            ensure!(
                observed["action"] == *action,
                "Outline original gesture differs"
            );
            if index > 0 {
                ensure!(
                    steps[index - 1]["command"] == *action,
                    "Outline original step evidence differs"
                );
            }
            let projection = json!({"action":raw_observed["action"],"resource":raw_observed["resource"],
                "text":raw_observed["text"],"primary":raw_observed["primary"],"dirty":raw_observed["dirty"]});
            ensure!(
                *observed == projection,
                "Outline raw editor projection differs"
            );
            ensure!(
                observed["resource"] == "main.cpp"
                    && observed["text"] == text
                    && observed["dirty"] == false,
                "Outline reference changed resource or text"
            );
            let length = text.chars().count() as u64;
            ensure!(
                ["anchor", "cursor"]
                    .iter()
                    .all(|key| observed["primary"][*key]
                        .as_u64()
                        .is_some_and(|offset| offset <= length))
                    && raw_observed["selections"][0] == observed["primary"],
                "Outline reference selection is invalid"
            );
        }
        // Confirmed reveals are evidence-derived symbol starts. Initial no-op
        // gestures remain in the complete artifact, not silently discarded.
        if fixture["shape"] == "hierarchical" {
            for name in ["main()", "render(int value)"] {
                let symbol = nodes
                    .iter()
                    .find(|symbol| symbol["name"] == name)
                    .context("Confirmed reveal symbol disappeared")?;
                let offset = reference_offset(text, &symbol["selectionRange"]["start"])?;
                ensure!(
                    observations
                        .iter()
                        .any(|observation| observation["action"] == "list.select"
                            && observation["primary"] == json!({"anchor":offset,"cursor":offset})),
                    "Confirmed collapsed reveal disappeared"
                );
            }
        }
    }
    ensure!(count == 20, "Outline snapshot inventory differs");
    Ok((cases, trace))
}

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn native_case(fixture: &Value, actual: &Value) -> Result<Value> {
    let directory = tempfile::tempdir()?;
    let root = std::fs::canonicalize(directory.path())?;
    let source = root.join("main.cpp");
    let text = fixture["text"].as_str().context("Missing compiled text")?;
    std::fs::write(&source, text)?;
    let mut app = App::new(root.clone(), Profile::Linux);
    app.open(&source)?;
    until(&mut app, "Native fixture open", |app| {
        app.active_document()
            .is_some_and(|document| document.path.as_ref() == Some(&source))
    })?;
    let identity = app.doc().id;
    let mut arguments = vec![
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/outline_server.py")
            .to_string_lossy()
            .into_owned(),
        root.join("peer.jsonl").to_string_lossy().into_owned(),
        root.join("release").to_string_lossy().into_owned(),
    ];
    if fixture["shape"] == "flat" {
        arguments.push("--flat".into());
    }
    app.lsp = Some(Client::start(
        if cfg!(windows) { "python" } else { "python3" },
        &arguments,
        &root,
        "cpp".into(),
    )?);
    until(&mut app, "Native provider ready", |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    })?;
    app.execute("outline.focus", Value::Null);
    until(&mut app, "Current Outline tree", |app| {
        app.outline_view().status == OutlineStatus::Ready && app.outline_view().actionable
    })?;
    let mut expected = Vec::new();
    geometry(
        actual["actualSymbols"].as_array().unwrap(),
        text,
        None,
        0,
        &mut expected,
    )?;
    let tree = app
        .outline_view()
        .tree
        .context("Current Outline lost its tree")?;
    let observed = tree.nodes.iter().map(|node|json!({"name":node.name,"kind":node.kind,
        "range":node.range,"selectionRange":node.selection_range,"parent":node.parent,"depth":node.depth})).collect::<Vec<_>>();
    ensure!(
        observed == expected,
        "Native provider geometry differs: observed={observed:?}; actual={expected:?}"
    );
    let mut reveals = Vec::new();
    if fixture["shape"] == "hierarchical" {
        // Exactly one accepted native keyboard gesture per confirmed target;
        // no target is retried or polled until an expected cursor appears.
        for name in ["main()", "render(int value)"] {
            app.execute("outline.focus", Value::Null);
            app.execute("outline.expand", Value::Null);
            key(&mut app, KeyCode::Home);
            let view = app.outline_view();
            let index = view
                .tree
                .unwrap()
                .nodes
                .iter()
                .position(|node| node.name == name)
                .context("Confirmed target missing")?;
            let row = view
                .visible
                .iter()
                .position(|visible| *visible == index)
                .context("Confirmed target hidden")?;
            for _ in 0..row {
                key(&mut app, KeyCode::Down);
            }
            key(&mut app, KeyCode::Enter);
            let offset = reference_offset(text, &expected[index]["selectionRange"]["start"])?;
            ensure!(
                app.focus == Focus::Editor
                    && app.doc().cursor == offset
                    && app.doc().anchor.is_none(),
                "Confirmed collapsed identifier-start reveal differs"
            );
            ensure!(
                app.doc().id == identity
                    && app.doc().text == text
                    && !app.doc().dirty()
                    && std::fs::read(&source)? == text.as_bytes(),
                "Readonly Outline reveal changed document or disk"
            );
            reveals.push(json!({"name":name,"anchor":offset,"cursor":offset}));
        }
        key(&mut app, KeyCode::Char('猫'));
        let edited = app.doc().text.to_string();
        ensure!(
            app.doc().dirty()
                && app.doc().id == identity
                && std::fs::read(&source)? == text.as_bytes(),
            "Outline target typing saved or replaced shared document"
        );
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, "Explicit edited save", |_| {
            std::fs::read(&source).ok().as_deref() == Some(edited.as_bytes())
        })?;
        app.execute("undo", Value::Null);
        ensure!(
            app.doc().text == text && app.doc().id == identity,
            "Undo after Outline reveal changed original document"
        );
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, "Explicit Undo save", |_| {
            std::fs::read(&source).ok().as_deref() == Some(text.as_bytes())
        })?;
    }
    Ok(
        json!({"name":fixture["name"],"geometry":observed,"confirmedCollapsedReveals":reveals,
        "scope":"Provider geometry plus confirmed main/render collapsed reveals; initial desktop no-op sequence retained only as raw qualification boundary. Container metadata and whole sidebar/history sequence are not compared."}),
    )
}
fn main() -> Result<()> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        arguments.len() <= 2,
        "Expected optional Outline trace and provenance paths"
    );
    let reference = arguments
        .first()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_REFERENCE.into());
    let tracked = reference
        .file_name()
        .is_some_and(|name| name == "linux.json");
    let provenance_path = arguments
        .get(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            reference.with_file_name(if tracked {
                "linux-provenance.json"
            } else {
                "outline-provenance.json"
            })
        });
    let evidence_path = reference.with_file_name(if tracked {
        "linux-evidence.json"
    } else {
        "outline-evidence.json"
    });
    let bytes = std::fs::read(reference)?;
    let evidence = std::fs::read(evidence_path)?;
    let provenance: Value = serde_json::from_slice(&std::fs::read(provenance_path)?)?;
    // Every artifact/source/geometry check completes before native fixtures write.
    let (cases, trace) = validate_reference(&bytes, &evidence, &provenance)?;
    let output = cases
        .iter()
        .zip(&trace)
        .map(|(fixture, actual)| native_case(fixture, actual))
        .collect::<Result<Vec<_>>>()?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    eprintln!(
        "Compared 2 actual provider geometry cases and 2 confirmed collapsed reveals; preserved 20 raw snapshots, no whole-sidebar parity claim"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const TRACE: &[u8] =
        include_bytes!("../tests/vscode-reference/baselines/1.95.0/outline/linux.json");
    const EVIDENCE: &[u8] =
        include_bytes!("../tests/vscode-reference/baselines/1.95.0/outline/linux-evidence.json");
    const PROVENANCE: &str =
        include_str!("../tests/vscode-reference/baselines/1.95.0/outline/linux-provenance.json");
    fn baseline() -> Value {
        serde_json::from_str(PROVENANCE).unwrap()
    }
    #[test]
    fn accepts_unmodified_actual_baseline() {
        let (cases, trace) = validate_reference(TRACE, EVIDENCE, &baseline()).unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(trace.len(), 2);
    }
    #[test]
    fn rejects_wrong_observer_after_positive_baseline() {
        let mut provenance = baseline();
        validate_reference(TRACE, EVIDENCE, &provenance).unwrap();
        provenance["runs"][0]["sources"]["outline.cjs"] = json!("0".repeat(64));
        assert_eq!(
            validate_reference(TRACE, EVIDENCE, &provenance)
                .unwrap_err()
                .to_string(),
            "Captured Outline source differs: outline.cjs"
        );
    }
    #[test]
    fn rejects_invalid_actual_geometry_even_with_updated_projection_digests() {
        let mut provenance = baseline();
        validate_reference(TRACE, EVIDENCE, &provenance).unwrap();
        let mut trace: Value = serde_json::from_slice(TRACE).unwrap();
        let mut evidence: Value = serde_json::from_slice(EVIDENCE).unwrap();
        trace[0]["actualSymbols"][0]["selectionRange"]["start"]["line"] = json!(999);
        evidence[0]["actualSymbols"] = trace[0]["actualSymbols"].clone();
        let trace = serde_json::to_vec(&trace).unwrap();
        let evidence = serde_json::to_vec(&evidence).unwrap();
        provenance["traceSha256"] = json!(hash(&trace));
        provenance["evidenceSha256"] = json!(hash(&evidence));
        assert_eq!(
            validate_reference(&trace, &evidence, &provenance)
                .unwrap_err()
                .to_string(),
            "Reference symbol position is outside text"
        );
    }
    #[test]
    fn rejects_missing_original_command_even_with_updated_evidence_digest() {
        let mut provenance = baseline();
        validate_reference(TRACE, EVIDENCE, &provenance).unwrap();
        let mut evidence: Value = serde_json::from_slice(EVIDENCE).unwrap();
        evidence[0]["commandInventory"]
            .as_array_mut()
            .unwrap()
            .retain(|command| command != "list.select");
        let evidence = serde_json::to_vec(&evidence).unwrap();
        provenance["evidenceSha256"] = json!(hash(&evidence));
        assert_eq!(
            validate_reference(TRACE, &evidence, &provenance)
                .unwrap_err()
                .to_string(),
            "Outline original command is missing"
        );
    }
}
