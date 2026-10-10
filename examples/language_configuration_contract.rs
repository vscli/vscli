//! Compare actual installed declarations through the native public catalog.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
use vscli::{
    document::{CommentOperation, Document, Selection},
    extension_activation::Preferences,
    extension_store::Installed,
    language_configuration::{Catalog, Configuration},
};

const CASES: &str = include_str!("../tests/vscode-reference/language-configuration-cases.json");
const DEFAULT_REFERENCE: &str =
    "tests/vscode-reference/baselines/1.95.0/language-configuration/linux.json";
const COMMIT: &str = "912bb683695358a54ae0c670461738984cbb5b95";
const EXCLUDED: [&str; 2] = [
    "unknown-brackets-brackets-fallback-pair",
    "unknown-brackets-brackets-fallback-surround",
];

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn captured_bytes(value: &Value, expected: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    ensure!(
        Some(hash(&bytes).as_str()) == expected.as_str(),
        "Captured installed declaration bytes do not match their provenance"
    );
    Ok(bytes)
}

fn configurations(
    provenance: &Value,
    directory: &Path,
    corpus: &Value,
) -> Result<BTreeMap<String, Arc<Configuration>>> {
    let mut configurations = BTreeMap::new();
    // Validate the entire source set before creating even the first package.
    for fixture in validate_fixtures(provenance, corpus)? {
        let variant = fixture["variant"].as_str().context("Missing variant")?;
        let manifest = &fixture["manifest"];
        let root = directory.join(variant);
        std::fs::create_dir(&root)?;
        std::fs::write(
            root.join("package.json"),
            captured_bytes(manifest, &fixture["manifestSha256"])?,
        )?;
        std::fs::write(
            root.join("language-configuration.json"),
            captured_bytes(&fixture["configuration"], &fixture["configurationSha256"])?,
        )?;
        let package = Installed {
            id: format!(
                "{}.{}",
                manifest["publisher"]
                    .as_str()
                    .context("Missing publisher")?,
                manifest["name"].as_str().context("Missing package name")?
            ),
            version: manifest["version"]
                .as_str()
                .context("Missing version")?
                .into(),
            path: root,
            manifest: manifest.clone(),
            source: "Pinned executable's generated declarative package".into(),
            // The fixture is an unpacked development package, not a VSIX.
            sha256: fixture["configurationSha256"]
                .as_str()
                .context("Missing configuration digest")?
                .into(),
            compatibility: "Native declarative data projection".into(),
        };
        let catalog = Catalog::load(&[package], &Preferences::default(), &Preferences::default());
        let language = manifest["contributes"]["languages"][0]["id"]
            .as_str()
            .context("Missing language")?;
        let configuration = catalog
            .for_language(language)
            .with_context(|| format!("Catalog dropped installed fixture {variant}"))?;
        ensure!(
            configurations
                .insert(variant.into(), configuration)
                .is_none(),
            "Duplicate fixture variant"
        );
    }
    Ok(configurations)
}

fn validate_fixtures<'a>(provenance: &'a Value, corpus: &Value) -> Result<&'a [Value]> {
    let mut expected = BTreeMap::new();
    for variant in corpus["variants"]
        .as_array()
        .context("Missing compiled variants")?
    {
        let name = variant["id"]
            .as_str()
            .context("Missing compiled variant name")?;
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "Compiled variant must be a safe single directory component"
        );
        let language = variant["language"]
            .as_str()
            .context("Missing compiled language")?;
        let mut configuration = variant["configuration"].clone();
        configuration
            .as_object_mut()
            .context("Missing compiled configuration object")?
            .insert(
                "onEnterRules".into(),
                json!([{
                    "beforeText":"^VSCLI_CONFIG_READY$",
                    "action":{"indent":"none","appendText":"!"},
                }]),
            );
        ensure!(
            expected.insert(name, (language, configuration)).is_none(),
            "Duplicate compiled variant"
        );
    }
    ensure!(
        expected.len() == 10,
        "Compiled declaration variants disappeared"
    );
    let fixtures = provenance["fixtures"]
        .as_array()
        .context("Missing packages")?;
    ensure!(
        fixtures.len() == expected.len(),
        "Captured source variants disappeared"
    );
    let mut seen = BTreeSet::new();
    for fixture in fixtures {
        let variant = fixture["variant"].as_str().context("Missing variant")?;
        let (language, configuration) = expected
            .get(variant)
            .context("Unknown captured variant; absolute and traversal paths are forbidden")?;
        ensure!(seen.insert(variant), "Duplicate captured variant");
        let manifest = json!({
            "name":format!("language-configuration-{variant}"),
            "publisher":"vscli-test","version":"0.0.0","private":true,
            "license":"MIT","engines":{"vscode":"^1.95.0"},
            "contributes":{"languages":[{"id":language,"configuration":"./language-configuration.json"}]},
        });
        ensure!(
            fixture["manifest"] == manifest,
            "Captured manifest shape/language differs for {variant}"
        );
        ensure!(
            fixture["configuration"] == *configuration,
            "Captured configuration differs for {variant}"
        );
        captured_bytes(&fixture["manifest"], &fixture["manifestSha256"])?;
        captured_bytes(&fixture["configuration"], &fixture["configurationSha256"])?;
    }
    ensure!(
        seen.len() == expected.len(),
        "Captured source set is incomplete"
    );
    Ok(fixtures)
}

fn snapshot(document: &Document, action: &str) -> Value {
    json!({"action":action,"text":document.text.to_string(),
        "selections":document.selections().iter().map(|selection| json!({
            "anchor":selection.anchor.unwrap_or(selection.cursor),
            "cursor":selection.cursor,
        })).collect::<Vec<_>>()})
}

fn bundled_configuration() -> Result<Arc<Configuration>> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vscode-cpp-configuration");
    let provenance: Value = serde_json::from_slice(&std::fs::read(root.join("provenance.json"))?)?;
    ensure!(
        provenance["referenceVersion"] == "1.95.0"
            && provenance["referenceCommit"] == COMMIT
            && provenance["extension"] == "vscode.cpp"
            && provenance["license"] == "MIT",
        "Bundled configuration identity differs"
    );
    for file in provenance["files"]
        .as_array()
        .context("Missing source files")?
    {
        let name = file["file"].as_str().context("Missing source filename")?;
        ensure!(
            matches!(
                name,
                "package.json" | "language-configuration.json" | "LICENSE"
            ),
            "Unexpected bundled source file"
        );
        ensure!(
            file["sha256"] == hash(&std::fs::read(root.join(name))?),
            "Bundled source digest differs for {name}"
        );
    }
    let package = Installed {
        id: "vscode.cpp".into(),
        version: "1.0.0".into(),
        path: root.clone(),
        manifest: serde_json::from_slice(&std::fs::read(root.join("package.json"))?)?,
        source: "Exact unchanged pinned bundled declarative input".into(),
        sha256: hash(&std::fs::read(root.join("language-configuration.json"))?),
        compatibility: "Native single-character declarative projection".into(),
    };
    ensure!(
        provenance["referenceInputInventorySha256"] == package.sha256,
        "Bundled configuration differs from the executable's inventoried input"
    );
    let catalog = Catalog::load(&[package], &Preferences::default(), &Preferences::default());
    ensure!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("onEnterRules"))
            && catalog
                .warnings
                .iter()
                .any(|warning| warning.contains("delimiters")),
        "Unqualified regex/multi-character input was not reported"
    );
    eprintln!(
        "Bundled vscode.cpp input projection: {} warnings explicitly retain unsupported regex, folding/word rules and multi-character pairs outside this comparison",
        catalog.warnings.len()
    );
    catalog
        .for_language("cpp")
        .context("Bundled CPP declaration disappeared")
}

fn trace(
    fixture: &Value,
    configuration: Arc<Configuration>,
    options: vscli::editing_profile::TypingOptions,
) -> Result<Value> {
    let mut document = Document::from_text(fixture["text"].as_str().context("Missing text")?);
    document.path = Some("reference.cpp".into());
    document.set_indentation(4, true);
    document.set_language_configuration(Some(configuration))?;
    let mut selections = Vec::new();
    for selection in fixture["selections"]
        .as_array()
        .context("Missing selections")?
    {
        let cursor = usize::try_from(selection["cursor"].as_u64().context("Missing cursor")?)?;
        let anchor = usize::try_from(selection["anchor"].as_u64().context("Missing anchor")?)?;
        ensure!(
            cursor <= document.text.len_chars() && anchor <= document.text.len_chars(),
            "Selection outside fixture"
        );
        selections.push(Selection {
            cursor,
            anchor: (anchor != cursor).then_some(anchor),
            desired_column: None,
        });
    }
    let primary = selections.first().context("Missing primary selection")?;
    document.cursor = primary.cursor;
    document.anchor = primary.anchor;
    document.secondary = selections.into_iter().skip(1).collect();
    let mut observations = vec![snapshot(&document, "initial")];
    for step in fixture["steps"].as_array().context("Missing steps")? {
        let action = if let Some(text) = step["type"].as_str() {
            let mut characters = text.chars();
            let character = characters.next().context("Missing typed character")?;
            ensure!(characters.next().is_none(), "Gesture must be one scalar");
            document.type_character(character, options, true)?;
            "type"
        } else {
            let command = step["command"].as_str().context("Missing command")?;
            match command {
                "editor.action.commentLine" => document.comment_lines(CommentOperation::Toggle)?,
                "editor.action.addCommentLine" => document.comment_lines(CommentOperation::Add)?,
                "editor.action.removeCommentLine" => {
                    document.comment_lines(CommentOperation::Remove)?
                }
                "editor.action.blockComment" => document.toggle_block_comment()?,
                "undo" => document.undo(),
                "redo" => document.redo(),
                _ => bail!("Unsupported captured command {command}"),
            }
            command
        };
        observations.push(snapshot(&document, action));
    }
    Ok(json!({"name":fixture["name"],"observations":observations}))
}

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let bundled = arguments.iter().any(|argument| argument == "--bundled");
    arguments.retain(|argument| argument != "--bundled");
    ensure!(
        arguments.len() <= 2,
        "Expected optional trace and provenance paths, and --bundled"
    );
    let reference = arguments
        .first()
        .cloned()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_REFERENCE.into());
    let provenance = arguments.get(1).cloned().map_or_else(
        || {
            reference.with_file_name(
                if reference
                    .file_name()
                    .is_some_and(|name| name == "linux.json")
                {
                    "linux-provenance.json"
                } else {
                    "language-configuration-provenance.json"
                },
            )
        },
        std::path::PathBuf::from,
    );
    let reference_bytes = std::fs::read(&reference)?;
    let reference: Vec<Value> = serde_json::from_slice(&reference_bytes)?;
    let provenance: Value = serde_json::from_slice(&std::fs::read(provenance)?)?;
    ensure!(
        provenance["version"] == "1.95.0" && provenance["commit"] == COMMIT,
        "Reference product identity differs"
    );
    ensure!(
        provenance["traceSha256"] == hash(&reference_bytes),
        "Reference trace hash differs"
    );
    ensure!(
        provenance["casesSha256"] == hash(CASES.as_bytes()),
        "Reference corpus hash differs"
    );
    let corpus: Value = serde_json::from_str(CASES)?;
    let cases = corpus["cases"].as_array().context("Missing corpus")?;
    let languages = corpus["variants"]
        .as_array()
        .context("Missing variants")?
        .iter()
        .map(|variant| {
            Ok((
                variant["id"].as_str().context("Missing variant identity")?,
                variant["language"]
                    .as_str()
                    .context("Missing variant language")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    ensure!(
        cases.len() == 52 && reference.len() == cases.len(),
        "Captured cases disappeared"
    );
    let directory = tempfile::tempdir()?;
    let configurations = configurations(&provenance, directory.path(), &corpus)?;
    let settings = directory.path().join("settings.json");
    std::fs::write(&settings, br#"{"editor.autoIndent":"full","editor.tabSize":4,"editor.insertSpaces":true,"editor.autoClosingQuotes":"languageDefined","editor.autoClosingBrackets":"languageDefined","editor.autoSurround":"languageDefined"}"#)?;
    let settings = vscli::settings::Settings::load(&[settings])?;
    ensure!(settings.warnings.is_empty(), "Invalid comparison settings");
    let mut native = Vec::new();
    let mut excluded = Vec::new();
    let mut failures = Vec::new();
    let bundled_configuration = bundled.then(bundled_configuration).transpose()?;
    let mut bundled_traces = Vec::new();
    for (fixture, expected) in cases.iter().zip(&reference) {
        ensure!(
            fixture["name"] == expected["name"],
            "Captured case ordering differs"
        );
        let variant = fixture["variant"]
            .as_str()
            .context("Missing case variant")?;
        if languages.get(variant).context("Unknown case variant")? != &"cpp" {
            excluded.push(fixture["name"].as_str().context("Missing case name")?);
            continue;
        }
        let configuration = configurations
            .get(variant)
            .context("Missing captured declaration")?;
        let actual = trace(fixture, Arc::clone(configuration), settings.typing("cpp"))?;
        if actual != *expected {
            failures.push(format!(
                "{}: native={}, reference={}",
                fixture["name"], actual["observations"], expected["observations"]
            ));
        }
        if variant == "omitted"
            && let Some(configuration) = &bundled_configuration
        {
            let actual = trace(fixture, Arc::clone(configuration), settings.typing("cpp"))?;
            if actual != *expected {
                failures.push(format!(
                    "Bundled input {}: native={}, reference={}",
                    fixture["name"], actual["observations"], expected["observations"]
                ));
            }
            bundled_traces.push(actual);
        }
        native.push(actual);
    }
    ensure!(excluded == EXCLUDED, "Raw-only exclusion identity changed");
    ensure!(native.len() == 50, "Qualified native cases disappeared");
    if bundled {
        ensure!(
            bundled_traces.len() == 21,
            "Bundled-input workflow cases disappeared"
        );
        eprintln!(
            "Compared 21 additional unchanged bundled CPP declaration workflows / 84 snapshots; no full extension execution claim"
        );
    }
    eprintln!(
        "Compared 50 C++ cases / 200 snapshots; preserved 2 explicitly raw-only unknown-language cases / 8 snapshots: {}",
        excluded.join(", ")
    );
    ensure!(
        failures.is_empty(),
        "Native declaration traces differ:\n{}",
        failures.join("\n")
    );
    let output = if bundled {
        json!({"native":native,"bundled":bundled_traces})
    } else {
        json!(native)
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> Value {
        serde_json::from_str(include_str!(
            "../tests/vscode-reference/baselines/1.95.0/language-configuration/linux-provenance.json"
        )).unwrap()
    }

    fn rejects_before_writes(provenance: &Value, scratch: &Path) {
        let corpus = serde_json::from_str(CASES).unwrap();
        assert!(configurations(provenance, scratch, &corpus).is_err());
        assert_eq!(std::fs::read_dir(scratch).unwrap().count(), 0);
    }

    #[test]
    fn hostile_absolute_and_traversal_variants_cannot_write_outside_scratch() {
        let parent = tempfile::tempdir().unwrap();
        let scratch = parent.path().join("scratch");
        std::fs::create_dir(&scratch).unwrap();
        let outside = parent.path().join("outside");
        for name in [outside.to_str().unwrap(), "../outside"] {
            let mut provenance = provenance();
            provenance["fixtures"][0]["variant"] = json!(name);
            rejects_before_writes(&provenance, &scratch);
            assert!(!outside.exists());
        }
    }

    #[test]
    fn duplicate_variant_rejects_entire_source_set_before_writes() {
        let scratch = tempfile::tempdir().unwrap();
        let mut provenance = provenance();
        let duplicate = provenance["fixtures"][0].clone();
        provenance["fixtures"][9] = duplicate;
        rejects_before_writes(&provenance, scratch.path());
    }

    #[test]
    fn missing_raw_only_fixture_rejects_before_writes() {
        let scratch = tempfile::tempdir().unwrap();
        let mut provenance = provenance();
        let removed = provenance["fixtures"]
            .as_array_mut()
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(removed["variant"], "unknown-brackets");
        rejects_before_writes(&provenance, scratch.path());
    }

    #[test]
    fn wrong_declared_language_rejects_before_writes_even_with_matching_hash() {
        let scratch = tempfile::tempdir().unwrap();
        let mut provenance = provenance();
        provenance["fixtures"][0]["manifest"]["contributes"]["languages"][0]["id"] =
            json!("plaintext");
        let mut bytes = serde_json::to_vec_pretty(&provenance["fixtures"][0]["manifest"]).unwrap();
        bytes.push(b'\n');
        provenance["fixtures"][0]["manifestSha256"] = json!(hash(&bytes));
        rejects_before_writes(&provenance, scratch.path());
    }
}
