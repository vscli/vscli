//! Compare visible navigation through public App inputs, not history internals.
use anyhow::{Context, Result, bail, ensure};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, keys::Profile};

const CASES: &str = include_str!("../tests/vscode-reference/navigation-history-cases.json");
const DEFAULT_REFERENCE: &str =
    "tests/vscode-reference/baselines/1.95.0/navigation-history/linux.json";

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn snapshot(app: &App, root: &Path, action: &str) -> Result<Value> {
    let document = app
        .active_document()
        .context("Navigation lost the active editor")?;
    let resource = document
        .path
        .as_deref()
        .context("Navigation produced an untitled resource")?
        .strip_prefix(root)
        .context("Active resource escaped the fixture")?
        .to_str()
        .context("Fixture resource is not UTF-8")?
        .replace('\\', "/");
    Ok(
        json!({"action":action,"resource":resource,"text":document.text.to_string(),
        "primary":{"anchor":document.anchor.unwrap_or(document.cursor),"cursor":document.cursor},
        "dirty":document.dirty()}),
    )
}

fn settle(app: &mut App, root: &Path, action: &str) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut previous = None;
    let mut unchanged = Instant::now();
    loop {
        app.poll();
        let observed = snapshot(app, root, action)?;
        if previous.as_ref() != Some(&observed) {
            previous = Some(observed.clone());
            unchanged = Instant::now();
        }
        if unchanged.elapsed() >= Duration::from_millis(100) {
            return Ok(observed);
        }
        ensure!(
            Instant::now() < deadline,
            "Independent native editor settlement exceeded two seconds"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn open(app: &mut App, path: &Path) -> Result<()> {
    app.open(path)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        app.poll();
        if app
            .active_document()
            .and_then(|document| document.path.as_deref())
            == Some(path)
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Public open did not complete its requested source path"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn trace(fixture: &Value) -> Result<Value> {
    let directory = tempfile::tempdir()?;
    let root = std::fs::canonicalize(directory.path())?;
    let files = fixture["files"]
        .as_array()
        .context("Missing resource fixtures")?;
    for file in files {
        let name = file["name"].as_str().context("Missing resource name")?;
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.')),
            "Fixture name must be one safe path component"
        );
        std::fs::write(
            root.join(name),
            file["text"].as_str().context("Missing resource text")?,
        )?;
    }
    let settings_path = root.join("fixture-settings.json");
    std::fs::write(&settings_path, br#"{"editor.tabSize":4,"editor.insertSpaces":true,"editor.autoClosingQuotes":"never","editor.autoClosingBrackets":"never"}"#)?;
    let mut app = App::new(root.clone(), Profile::Linux);
    app.settings = vscli::settings::Settings::load(&[settings_path])?;
    ensure!(
        app.settings.warnings.is_empty(),
        "Invalid native fixture settings: {:?}",
        app.settings.warnings
    );
    let first = fixture["setup"]["open"]
        .as_str()
        .context("Missing setup resource")?;
    open(&mut app, &root.join(first))?;
    if let Some(selection) = fixture["setup"].get("selection") {
        let anchor = usize::try_from(
            selection["anchor"]
                .as_u64()
                .context("Missing setup anchor")?,
        )?;
        let cursor = usize::try_from(
            selection["cursor"]
                .as_u64()
                .context("Missing setup cursor")?,
        )?;
        ensure!(
            anchor <= app.doc().len() && cursor <= app.doc().len(),
            "Invalid setup selection"
        );
        app.doc_mut().move_to(anchor, false);
        app.doc_mut().move_to(cursor, true);
        app.poll();
    }
    // Reference token preparation changes no editor/document state and has no
    // native command analogue. Native bracket matching inspects its own Rope.
    let mut observations = vec![settle(&mut app, &root, "initial")?];
    for step in fixture["steps"]
        .as_array()
        .context("Missing target gestures")?
    {
        let action = if let Some(resource) = step["open"].as_str() {
            ensure!(
                files.iter().any(|file| file["name"] == resource),
                "Unknown open resource"
            );
            open(&mut app, &root.join(resource))?;
            "open"
        } else if let Some(line) = step["gotoLine"].as_u64() {
            app.execute("workbench.action.gotoLine", Value::Null);
            ensure!(
                app.prompt.is_some(),
                "Go to Line did not acquire its input control"
            );
            app.event(Event::Paste(line.to_string()));
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            ensure!(app.prompt.is_none(), "Go to Line did not accept its input");
            "gotoLine"
        } else if let Some(text) = step["type"].as_str() {
            let mut characters = text.chars();
            let character = characters.next().context("Missing typed scalar")?;
            ensure!(
                characters.next().is_none(),
                "Typing gesture must be one scalar"
            );
            app.event(Event::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )));
            "type"
        } else {
            let command = step["command"].as_str().context("Missing target command")?;
            app.execute(command, step.get("args").cloned().unwrap_or(Value::Null));
            command
        };
        observations.push(settle(&mut app, &root, action)?);
    }
    for file in files {
        ensure!(
            std::fs::read_to_string(root.join(file["name"].as_str().unwrap()))?
                == file["text"].as_str().unwrap(),
            "Navigation/edit/Undo changed fixture disk bytes"
        );
    }
    Ok(json!({"name":fixture["name"],"observations":observations}))
}

fn validate_reference(
    bytes: &[u8],
    evidence_bytes: &[u8],
    provenance: &Value,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let cases: Vec<Value> = serde_json::from_str(CASES)?;
    let reference: Vec<Value> = serde_json::from_slice(bytes)?;
    let evidence: Vec<Value> = serde_json::from_slice(evidence_bytes)?;
    ensure!(
        provenance["version"] == "1.95.0"
            && provenance["commit"] == "912bb683695358a54ae0c670461738984cbb5b95"
            && matches!(
                provenance["platform"].as_str(),
                Some("linux" | "darwin" | "win32")
            ),
        "Navigation reference product or platform differs"
    );
    ensure!(
        provenance["traceSha256"] == hash(bytes),
        "Navigation reference trace hash differs"
    );
    ensure!(
        provenance["evidenceSha256"] == hash(evidence_bytes),
        "Navigation evidence hash differs"
    );
    ensure!(
        provenance["casesSha256"] == hash(CASES.as_bytes()),
        "Navigation corpus hash differs"
    );
    ensure!(
        cases.len() == 10
            && reference.len() == cases.len()
            && evidence.len() == cases.len()
            && provenance["caseCount"] == 10
            && provenance["snapshotCount"] == 85,
        "Navigation cases or snapshots disappeared"
    );
    let sources: &[(&str, &[u8])] = &[
        (
            "observerSha256",
            include_bytes!("../tests/vscode-reference/navigation-history.cjs"),
        ),
        ("casesSha256", CASES.as_bytes()),
        (
            "suiteSha256",
            include_bytes!("../tests/vscode-reference/navigation-history-suite.cjs"),
        ),
        (
            "runnerSha256",
            include_bytes!("../tests/vscode-reference/navigation-history-run.cjs"),
        ),
        (
            "workerSha256",
            include_bytes!("../tests/vscode-reference/navigation-history-worker.cjs"),
        ),
        (
            "supervisorSha256",
            include_bytes!("../tests/vscode-reference/supervisor.cjs"),
        ),
        (
            "testElectronLockSha256",
            include_bytes!("../tests/vscode-reference/package-lock.json"),
        ),
        (
            "harnessManifestSha256",
            include_bytes!("../tests/vscode-reference/package.json"),
        ),
        (
            "harnessExtensionSha256",
            include_bytes!("../tests/vscode-reference/extension.cjs"),
        ),
    ];
    let runs = provenance["runs"]
        .as_array()
        .context("Missing actual runs")?;
    ensure!(runs.len() == cases.len(), "Actual run inventory differs");
    let mut resource_inventory = Vec::new();
    let mut count = 0;
    for (((fixture, expected), raw), run) in cases.iter().zip(&reference).zip(&evidence).zip(runs) {
        ensure!(
            fixture["name"] == expected["name"]
                && fixture["name"] == raw["name"]
                && fixture["name"] == run["name"],
            "Navigation fixture ordering differs"
        );
        ensure!(
            run["version"] == provenance["version"]
                && run["commit"] == provenance["commit"]
                && run["platform"] == provenance["platform"]
                && run["architecture"] == provenance["architecture"],
            "Actual run product identity differs"
        );
        for (field, source) in sources {
            ensure!(
                run[*field] == hash(source),
                "Captured source differs: {field}"
            );
        }
        for field in ["productSha256", "evidenceSha256", "profileSettingsSha256"] {
            ensure!(
                run[field].as_str().is_some_and(|text| text.len() == 64
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())),
                "Invalid actual run digest: {field}"
            );
        }
        let files = fixture["files"]
            .as_array()
            .context("Missing compiled fixtures")?;
        for file in files {
            let name = file["name"].as_str().context("Missing compiled resource")?;
            ensure!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
                    && name != "."
                    && name != "..",
                "Compiled resource is unsafe"
            );
            resource_inventory.push(json!({"case":fixture["name"],"resource":name,
                "sha256":hash(file["text"].as_str().context("Missing compiled text")?.as_bytes())}));
        }
        let observations = expected["observations"]
            .as_array()
            .context("Missing reference observations")?;
        let raw_observations = raw["observations"]
            .as_array()
            .context("Missing raw observations")?;
        let steps = fixture["steps"]
            .as_array()
            .context("Missing compiled steps")?;
        ensure!(
            observations.len() == steps.len() + 1 && raw_observations.len() == observations.len(),
            "Navigation gesture inventory differs"
        );
        count += observations.len();
        for (index, (observation, raw_observation)) in
            observations.iter().zip(raw_observations).enumerate()
        {
            let action = if index == 0 {
                "initial"
            } else {
                let step = &steps[index - 1];
                if step.get("open").is_some() {
                    "open"
                } else if step.get("gotoLine").is_some() {
                    "gotoLine"
                } else if step.get("type").is_some() {
                    "type"
                } else {
                    step["command"]
                        .as_str()
                        .context("Missing compiled command")?
                }
            };
            ensure!(
                observation["action"] == action,
                "Navigation original gesture differs"
            );
            let projected = json!({"action":raw_observation["action"],"resource":raw_observation["resource"],
                "text":raw_observation["text"],"primary":raw_observation["primary"],"dirty":raw_observation["dirty"]});
            ensure!(
                *observation == projected,
                "Navigation raw evidence projection differs"
            );
            ensure!(
                files
                    .iter()
                    .any(|file| file["name"] == observation["resource"]),
                "Reference active resource is outside the compiled inventory"
            );
            let text = observation["text"]
                .as_str()
                .context("Missing observed text")?;
            let length = text.chars().count() as u64;
            for field in ["anchor", "cursor"] {
                ensure!(
                    observation["primary"][field]
                        .as_u64()
                        .is_some_and(|position| position <= length),
                    "Observed selection is outside text"
                );
            }
            ensure!(
                observation["dirty"].is_boolean()
                    && raw_observation["selections"][0] == observation["primary"],
                "Observed primary selection or dirty state differs"
            );
            let documents = raw_observation["documents"]
                .as_array()
                .context("Missing resource evidence")?;
            ensure!(
                documents.len() == files.len(),
                "Raw resource inventory differs"
            );
            for (document, file) in documents.iter().zip(files) {
                ensure!(
                    document["resource"] == file["name"] && document["disk"] == file["text"],
                    "Fixture disk bytes or resource inventory differs"
                );
            }
        }
        let setup = raw["setup"]
            .as_array()
            .context("Missing independent setup evidence")?;
        for operation in setup {
            if operation["action"] == "public.forceRetokenize" {
                ensure!(
                    operation["before"] == operation["after"],
                    "Bracket preparation changed editor state"
                );
            }
        }
        ensure!(
            setup
                .iter()
                .filter(|operation| operation["action"] == "public.forceRetokenize")
                .count()
                == usize::from(fixture["setup"]["prepareTokens"] == true),
            "Bracket preparation inventory differs"
        );
    }
    ensure!(count == 85, "Navigation snapshot inventory differs");
    ensure!(
        provenance["files"] == Value::Array(resource_inventory),
        "Resource provenance inventory differs"
    );
    Ok((cases, reference))
}

fn main() -> Result<()> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        arguments.len() <= 2,
        "Expected optional trace and provenance paths"
    );
    let reference = arguments
        .first()
        .cloned()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_REFERENCE.into());
    let provenance = arguments
        .get(1)
        .cloned()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            reference.with_file_name(
                if reference
                    .file_name()
                    .is_some_and(|name| name == "linux.json")
                {
                    "linux-provenance.json"
                } else {
                    "navigation-history-provenance.json"
                },
            )
        });
    let evidence_path = reference.with_file_name(
        if reference
            .file_name()
            .is_some_and(|name| name == "linux.json")
        {
            "linux-evidence.json"
        } else {
            "navigation-history-evidence.json"
        },
    );
    let bytes = std::fs::read(reference)?;
    let evidence_bytes = std::fs::read(evidence_path)?;
    let provenance: Value = serde_json::from_slice(&std::fs::read(provenance)?)?;
    // Complete provenance and evidence preflight precedes every fixture write.
    let (cases, reference) = validate_reference(&bytes, &evidence_bytes, &provenance)?;
    let mut observed = Vec::new();
    let mut differences = Vec::new();
    for (fixture, expected) in cases.iter().zip(&reference) {
        ensure!(
            fixture["name"] == expected["name"],
            "Navigation fixture ordering differs"
        );
        let actual = trace(fixture)?;
        if actual != *expected {
            differences.push(format!(
                "{}: native={}, reference={}",
                fixture["name"], actual["observations"], expected["observations"]
            ));
        }
        observed.push(actual);
    }
    if !differences.is_empty() {
        bail!("Navigation trace differences:\n{}", differences.join("\n"));
    }
    println!("{}", serde_json::to_string_pretty(&observed)?);
    eprintln!("Compared 10 navigation cases / 85 visible snapshots through public App inputs");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const TRACE: &[u8] =
        include_bytes!("../tests/vscode-reference/baselines/1.95.0/navigation-history/linux.json");
    const EVIDENCE: &[u8] = include_bytes!(
        "../tests/vscode-reference/baselines/1.95.0/navigation-history/linux-evidence.json"
    );
    const PROVENANCE: &str = include_str!(
        "../tests/vscode-reference/baselines/1.95.0/navigation-history/linux-provenance.json"
    );

    fn reject_at_guard(expected_error: &str, change: impl FnOnce(&mut Value)) {
        let mut provenance: Value = serde_json::from_str(PROVENANCE).unwrap();
        validate_reference(TRACE, EVIDENCE, &provenance)
            .expect("Unmodified baseline must pass before testing malformed metadata");
        change(&mut provenance);
        let error = validate_reference(TRACE, EVIDENCE, &provenance)
            .expect_err("Malformed metadata must fail at its intended guard");
        assert_eq!(error.to_string(), expected_error);
    }

    #[test]
    fn accepts_unmodified_pinned_baseline() {
        let provenance: Value = serde_json::from_str(PROVENANCE).unwrap();
        let (cases, observations) = validate_reference(TRACE, EVIDENCE, &provenance).unwrap();
        assert_eq!(cases.len(), 10);
        assert_eq!(observations.len(), 10);
        assert_eq!(
            observations
                .iter()
                .map(|case| case["observations"].as_array().unwrap().len())
                .sum::<usize>(),
            85
        );
    }

    #[test]
    fn rejects_different_observer_source_before_fixture_creation() {
        reject_at_guard("Captured source differs: observerSha256", |provenance| {
            provenance["runs"][0]["observerSha256"] = json!("0".repeat(64))
        });
    }

    #[test]
    fn rejects_duplicate_resource_even_with_complete_count() {
        reject_at_guard("Resource provenance inventory differs", |provenance| {
            provenance["files"][1] = provenance["files"][0].clone()
        });
    }

    #[test]
    fn rejects_missing_resource_before_fixture_creation() {
        reject_at_guard("Resource provenance inventory differs", |provenance| {
            provenance["files"].as_array_mut().unwrap().pop();
        });
    }

    #[test]
    fn rejects_hostile_reference_resource_before_fixture_creation() {
        reject_at_guard("Resource provenance inventory differs", |provenance| {
            provenance["files"][0]["resource"] = json!("../../outside.cpp")
        });
    }
}
