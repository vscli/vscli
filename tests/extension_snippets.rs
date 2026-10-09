use serde_json::{Value, json};
use std::{fs, io::Write, path::Path};
use vscli::{extension_store::Store, snippet::catalog::Catalog};
use zip::{ZipWriter, write::SimpleFileOptions};

fn install(
    store: &Store,
    archive: &Path,
    version: &str,
    snippets: Value,
    files: &[(&str, &str)],
) -> vscli::extension_store::Installed {
    let mut zip = ZipWriter::new(fs::File::create(archive).unwrap());
    zip.start_file("extension/package.json", SimpleFileOptions::default())
        .unwrap();
    write!(zip, "{}", json!({"publisher":"test", "name":"snippets", "version":version,"contributes":{"snippets":snippets}})).unwrap();
    for (name, text) in files {
        zip.start_file(format!("extension/{name}"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    store.install(archive).unwrap()
}
fn load(store: &Store, workspace: &Path, language: &str) -> Catalog {
    Catalog::load_with_extensions(None, workspace, language, Some(store.root()))
}
fn local(workspace: &Path) {
    fs::create_dir_all(workspace.join(".vscode")).unwrap();
    fs::write(
        workspace.join(".vscode/local.code-snippets"),
        r#"{"Native":{"body":"kept"}}"#,
    )
    .unwrap();
}

#[test]
fn installed_languages_global_scopes_and_refresh_use_data_without_a_runtime() {
    let root = tempfile::tempdir().unwrap();
    local(root.path());
    let store = Store::new(root.path().join("store"));
    let archive = root.path().join("snippets.vsix");
    let contributions = json!([
        {"language":"cpp","path":"./snippets/cpp.code-snippets"},
        {"language":"javascript","path":"snippets/js.json"},
        {"path":"snippets/global.code-snippets"}
    ]);
    let files = [
        (
            "snippets/cpp.code-snippets",
            r#"{"C++":{"scope":"python", "prefix":"cpp", "body":"${1:class}-$1$0"}}"#,
        ),
        (
            "snippets/js.json",
            r#"{"JavaScript":{"body":"const ${1:name} = $0;"}}"#,
        ),
        (
            "snippets/global.code-snippets",
            r#"{"Global":{"body":"global"},"Python":{"scope":"python", "body":"python"}}"#,
        ),
    ];
    let installed = install(&store, &archive, "1.0.0", contributions.clone(), &files);
    assert!(installed.compatibility.contains("no Node"));
    for (language, expected) in [
        ("cpp", vec!["Native", "C++", "Global"]),
        ("javascript", vec!["Native", "JavaScript", "Global"]),
        ("python", vec!["Native", "Global", "Python"]),
    ] {
        let catalog = load(&store, root.path(), language);
        assert!(catalog.warnings.is_empty(), "{:?}", catalog.warnings);
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    install(
        &store,
        &archive,
        "2.0.0",
        json!([{"language":"cpp","path":"new.json"}]),
        &[("new.json", r#"{"Updated":{"body":"updated"}}"#)],
    );
    assert_eq!(load(&store, root.path(), "cpp").entries[1].name, "Updated");
    store.rollback("test.snippets").unwrap();
    assert_eq!(load(&store, root.path(), "cpp").entries[1].name, "C++");
    store.uninstall("test.snippets").unwrap();
    assert_eq!(load(&store, root.path(), "cpp").entries.len(), 1);
}

#[test]
fn bad_contributions_and_registry_preserve_native_catalogs() {
    let root = tempfile::tempdir().unwrap();
    local(root.path());
    let store = Store::new(root.path().join("store"));
    install(
        &store,
        &root.path().join("bad.vsix"),
        "1.0.0",
        json!([
            {"language":"cpp","path":"../outside.json"},
            {"language":"cpp","path":"/outside.json"},
            {"language":"cpp","path":"C:\\outside.json"},
            {"language":42,"path":"good.json"},
            {"path":"good.json"},
            {"language":"cpp","path":"broken.json"},
            {"language":"cpp","path":"bad-body.json"},
            {"language":"cpp","path":"good.json"}
        ]),
        &[
            ("broken.json", "{ invalid"),
            ("bad-body.json", r#"{"Bad":{"body":42}}"#),
            ("good.json", r#"{"Good":{"body":"works"}}"#),
        ],
    );
    let catalog = load(&store, root.path(), "cpp");
    assert_eq!(catalog.entries.len(), 2);
    assert_eq!(catalog.entries[1].name, "Good");
    assert_eq!(catalog.warnings.len(), 7);
    install(
        &store,
        &root.path().join("bad.vsix"),
        "2.0.0",
        json!({"wrong":"shape"}),
        &[],
    );
    let catalog = load(&store, root.path(), "cpp");
    assert_eq!(catalog.entries.len(), 1);
    assert!(catalog.warnings[0].contains("must be an array"));
    fs::write(store.root().join("registry.json"), "invalid").unwrap();
    let catalog = load(&store, root.path(), "cpp");
    assert_eq!(catalog.entries.len(), 1);
    assert!(catalog.warnings[0].contains("Installed snippets unavailable"));
}

#[cfg(unix)]
#[test]
fn canonical_contribution_paths_cannot_follow_a_link_outside_the_package() {
    let root = tempfile::tempdir().unwrap();
    local(root.path());
    let store = Store::new(root.path().join("store"));
    let installed = install(
        &store,
        &root.path().join("package.vsix"),
        "1.0.0",
        json!([{"language":"cpp","path":"snippets.json"}]),
        &[("snippets.json", "{}")],
    );
    let outside = root.path().join("outside.json");
    fs::write(&outside, r#"{"Outside":{"body":"must not load"}}"#).unwrap();
    fs::remove_file(installed.path.join("snippets.json")).unwrap();
    std::os::unix::fs::symlink(outside, installed.path.join("snippets.json")).unwrap();
    let catalog = load(&store, root.path(), "cpp");
    assert_eq!(catalog.entries.len(), 1);
    assert!(catalog.warnings[0].contains("escapes its package"));
}

#[test]
fn native_and_extension_files_share_file_entry_and_byte_budgets() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    local(&workspace);
    let store = Store::new(root.path().join("store"));
    let body = "x".repeat(1024);
    let snippet = json!({"Extension":{"body":body}}).to_string();
    install(
        &store,
        &root.path().join("package.vsix"),
        "1.0.0",
        json!([{"language":"cpp","path":"snippets.json"}]),
        &[("snippets.json", &snippet)],
    );
    for index in 0..127 {
        fs::write(
            workspace.join(format!(".vscode/{index:03}.code-snippets")),
            "{}",
        )
        .unwrap();
    }
    let catalog = load(&store, &workspace, "cpp");
    assert_eq!(catalog.entries.len(), 1);
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("128 files"))
    );
    for index in 0..127 {
        fs::remove_file(workspace.join(format!(".vscode/{index:03}.code-snippets"))).unwrap();
    }
    let entries: serde_json::Map<_, _> = (0..4095)
        .map(|i| (format!("entry{i}"), json!({"body":"x"})))
        .collect();
    fs::write(
        workspace.join(".vscode/entries.code-snippets"),
        Value::Object(entries).to_string(),
    )
    .unwrap();
    let catalog = load(&store, &workspace, "cpp");
    assert_eq!(catalog.entries.len(), 4096);
    assert!(
        !catalog
            .entries
            .iter()
            .any(|entry| entry.name == "Extension")
    );
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("4096 entries"))
    );
    fs::remove_file(workspace.join(".vscode/entries.code-snippets")).unwrap();
    let padding = format!("/*{}*/{{}}", " ".repeat(1024 * 1024 - 22));
    for index in 0..16 {
        fs::write(
            workspace.join(format!(".vscode/z{index:03}.code-snippets")),
            &padding,
        )
        .unwrap();
    }
    let catalog = load(&store, &workspace, "cpp");
    assert_eq!(catalog.entries.len(), 1);
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("16 MiB"))
    );
}
