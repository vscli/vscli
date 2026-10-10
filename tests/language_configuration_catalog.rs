use serde_json::json;
use std::{fs, path::Path};
use vscli::{
    extension_activation::Preferences, extension_store::Installed, language_configuration::Catalog,
};

fn package(root: &Path, id: &str, text: &str) -> Installed {
    let directory = root.join(id);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("language.json"), text).unwrap();
    Installed {
        id: id.into(),
        version: "1.2.3".into(),
        path: directory,
        manifest: json!({"contributes":{"languages":[{"id":"cpp","configuration":"./language.json"}]}}),
        source: "fixture".into(),
        sha256: "archive-sha".into(),
        compatibility: "browser-only; JavaScript execution unavailable".into(),
    }
}
fn load(packages: &[Installed]) -> Catalog {
    Catalog::load(packages, &Preferences::default(), &Preferences::default())
}

#[test]
fn native_data_defaults_enabled_without_granting_javascript_execution() {
    let root = tempfile::tempdir().unwrap();
    let installed = package(
        root.path(),
        "fixture.native",
        r#"{
        // native metadata, not extension code
        "autoClosingPairs": [{"open":"猫","close":"🙂","notIn":["string","comment"]}],
        "surroundingPairs": [["[","]"]], "brackets": [["{","}"]],
        "autoCloseBefore": "; )\n", "comments": {"lineComment":"//","blockComment":["/*","*/"]},
    }"#,
    );
    let global = Preferences::default();
    let workspace = Preferences::default();
    assert!(!Preferences::enabled(&global, &workspace, &installed.id));
    let catalog = Catalog::load(&[installed], &global, &workspace);
    assert!(catalog.warnings.is_empty(), "{:?}", catalog.warnings);
    let configuration = catalog.for_language("cpp").unwrap();
    let pair = configuration.auto_closing_pairs.as_ref().unwrap()[0];
    assert_eq!((pair.open, pair.close), ('猫', '🙂'));
    assert!(pair.not_string && pair.not_comment);
    assert_eq!(configuration.auto_close_before.as_deref(), Some("; )\n"));
    assert_eq!(
        configuration
            .comments
            .as_ref()
            .unwrap()
            .block_comment
            .as_ref()
            .unwrap(),
        &("/*".into(), "*/".into())
    );
    assert!(catalog.for_language("rust").is_none());
}

#[test]
fn explicit_workspace_preference_overrides_global_and_uninstall_has_no_retained_source() {
    let root = tempfile::tempdir().unwrap();
    let installed = package(root.path(), "fixture.native", "{}");
    let mut global = Preferences::default();
    let mut workspace = Preferences::default();
    global.set(&installed.id, false).unwrap();
    assert!(
        Catalog::load(std::slice::from_ref(&installed), &global, &workspace)
            .profiles
            .is_empty()
    );
    workspace.set(&installed.id, true).unwrap();
    assert!(
        Catalog::load(std::slice::from_ref(&installed), &global, &workspace)
            .for_language("cpp")
            .is_some()
    );
    global.set(&installed.id, true).unwrap();
    workspace.set(&installed.id, false).unwrap();
    assert!(
        Catalog::load(&[installed], &global, &workspace)
            .profiles
            .is_empty()
    );
    assert!(load(&[]).profiles.is_empty());
}

#[test]
fn identity_is_stable_on_reload_and_changes_for_content_owner_and_generation() {
    let root = tempfile::tempdir().unwrap();
    let mut installed = package(root.path(), "fixture.native", "{brackets:[['{','}']]}");
    let first = load(std::slice::from_ref(&installed))
        .for_language("cpp")
        .unwrap();
    let catalog = load(std::slice::from_ref(&installed));
    assert!(catalog.has_installed_configuration(&installed));
    for changed in ["version", "path", "archive", "owner"] {
        let mut other = installed.clone();
        match changed {
            "version" => other.version.push_str("-other"),
            "path" => other.path.push("other-generation"),
            "archive" => other.sha256.push_str("-other"),
            _ => other.id = "another.owner".into(),
        }
        assert!(!catalog.has_installed_configuration(&other), "{changed}");
    }
    let again = load(std::slice::from_ref(&installed))
        .for_language("cpp")
        .unwrap();
    assert_eq!(first.identity, again.identity);
    fs::write(
        installed.path.join("language.json"),
        "{brackets:[['[',']']]}",
    )
    .unwrap();
    let changed = load(std::slice::from_ref(&installed))
        .for_language("cpp")
        .unwrap();
    assert_ne!(first.identity, changed.identity);
    assert_eq!(
        first.identity.archive_sha256,
        changed.identity.archive_sha256
    );
    installed.sha256 = "new-generation".into();
    let generation = load(std::slice::from_ref(&installed))
        .for_language("cpp")
        .unwrap();
    assert_ne!(generation.identity, changed.identity);
    installed.version = "1.2.4".into();
    assert_ne!(
        generation.identity,
        load(&[installed]).for_language("cpp").unwrap().identity
    );
    // Published immutable snapshots retain the earlier source's actual data.
    assert_eq!(first.brackets.as_ref().unwrap()[0].open, '{');
}

#[test]
fn sorted_owner_order_and_manifest_order_compose_fields_with_source_proof() {
    let root = tempfile::tempdir().unwrap();
    let earlier = package(
        root.path(),
        "a.native",
        "{autoClosingPairs:[['{','}']], comments:{lineComment:'//',blockComment:['/*','*/']}}",
    );
    let later = package(
        root.path(),
        "z.native",
        "{surroundingPairs:[['[',']']],comments:{lineComment:'#'}}",
    );
    let one = load(&[later.clone(), earlier.clone()]);
    let two = load(&[earlier.clone(), later.clone()]);
    let result = one.for_language("cpp").unwrap();
    assert_eq!(result, two.for_language("cpp").unwrap());
    assert_eq!(result.identity.owner, "z.native");
    assert_eq!(result.auto_closing_pairs.as_ref().unwrap()[0].open, '{');
    assert_eq!(result.surrounding_pairs.as_ref().unwrap()[0].open, '[');
    assert!(result.comments.as_ref().unwrap().block_comment.is_none());
    assert!(
        one.warnings
            .iter()
            .any(|warning| warning.contains("compose fieldwise"))
    );
    fs::write(
        earlier.path.join("language.json"),
        "{autoClosingPairs:[['(',')']]}",
    )
    .unwrap();
    let changed = load(&[earlier, later]).for_language("cpp").unwrap();
    assert_eq!(
        result.identity.content_sha256,
        changed.identity.content_sha256
    );
    assert_ne!(
        result.identity.composition_sha256,
        changed.identity.composition_sha256
    );

    let mut installed = package(root.path(), "fixture.order", "{brackets:[['{','}']]}");
    fs::write(installed.path.join("later.json"), "{brackets:[['[',']']]}").unwrap();
    installed.manifest["contributes"]["languages"] = json!([
        {"id":"cpp","configuration":"language.json"},{"id":"cpp","configuration":"later.json"}]);
    let result = load(&[installed]).for_language("cpp").unwrap();
    assert_eq!(result.brackets.as_ref().unwrap()[0].open, '[');
    assert_eq!(result.identity.configuration_path, Path::new("later.json"));
}

#[test]
fn installed_empty_fields_are_omitted_and_partial_comments_replace_lower_field() {
    let root = tempfile::tempdir().unwrap();
    let earlier = package(
        root.path(),
        "a.native",
        "{autoClosingPairs:[['{','}']], surroundingPairs:[['(',')']], brackets:[['[',']']],autoCloseBefore:' ',comments:{lineComment:'//',blockComment:['/*','*/']}}",
    );
    let later = package(
        root.path(),
        "z.native",
        "{autoClosingPairs:[],surroundingPairs:[],brackets:[],autoCloseBefore:'',comments:{}}",
    );
    let result = load(&[earlier, later]).for_language("cpp").unwrap();
    assert_eq!(result.auto_closing_pairs.as_ref().unwrap()[0].open, '{');
    assert_eq!(result.auto_close_before.as_deref(), Some(" "));
    assert_eq!(
        result.comments.as_ref().unwrap().line_comment.as_deref(),
        Some("//")
    );
}

#[test]
fn unsupported_and_malformed_fields_have_visible_conservative_fallbacks() {
    let root = tempfile::tempdir().unwrap();
    let installed = package(
        root.path(),
        "fixture.native",
        "{autoClosingPairs:[{open:'{',close:'}',notIn:['regex']}],surroundingPairs:[['/*','*/']],brackets:7,autoCloseBefore:[],comments:{blockComment:['/*']},indentationRules:{increaseIndentPattern:'x'},onEnterRules:[]}",
    );
    let catalog = load(&[installed]);
    let result = catalog.for_language("cpp").unwrap();
    assert_eq!(result.auto_closing_pairs, Some(vec![]));
    assert_eq!(result.surrounding_pairs, Some(vec![]));
    assert_eq!(result.brackets, None);
    assert_eq!(result.auto_close_before, None);
    assert!(result.comments.is_none());
    assert_eq!(catalog.warnings.len(), 7);
}

#[test]
fn installed_invalid_items_filter_individually_and_all_invalid_fields_inherit() {
    let root = tempfile::tempdir().unwrap();
    let lower = package(
        root.path(),
        "a.native",
        "{autoClosingPairs:[['{','}']],brackets:[['[',']']],comments:{lineComment:'//',blockComment:['/*','*/']}}",
    );
    let upper = package(
        root.path(),
        "z.native",
        "{autoClosingPairs:[7,{open:'(',close:')'}],brackets:[7,{open:'<',close:'>'}],comments:{lineComment:'#',blockComment:7},autoCloseBefore:7}",
    );
    let catalog = load(&[lower, upper]);
    let result = catalog.for_language("cpp").unwrap();
    assert_eq!(result.auto_closing_pairs.as_ref().unwrap().len(), 1);
    assert_eq!(result.auto_closing_pairs.as_ref().unwrap()[0].open, '(');
    assert_eq!(result.brackets.as_ref().unwrap()[0].open, '[');
    assert_eq!(
        result.comments.as_ref().unwrap().line_comment.as_deref(),
        Some("#")
    );
    assert!(result.comments.as_ref().unwrap().block_comment.is_none());
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("omitted"))
    );
}

#[test]
fn confinement_rejects_escape_absolute_backslash_and_nonregular_sources() {
    let root = tempfile::tempdir().unwrap();
    let mut installed = package(root.path(), "fixture.native", "{}");
    fs::create_dir(installed.path.join("directory")).unwrap();
    for relative in [
        "../outside.json",
        "/tmp/outside.json",
        "..\\outside.json",
        "C:outside.json",
        "directory",
        "",
    ] {
        installed.manifest["contributes"]["languages"][0]["configuration"] = json!(relative);
        let catalog = load(std::slice::from_ref(&installed));
        assert!(catalog.profiles.is_empty(), "{relative}");
        assert!(!catalog.warnings.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn symlink_escape_and_fifo_are_rejected_without_blocking() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let mut installed = package(root.path(), "fixture.native", "{}");
    let outside = root.path().join("outside.json");
    fs::write(&outside, "{}").unwrap();
    symlink(&outside, installed.path.join("link.json")).unwrap();
    installed.manifest["contributes"]["languages"][0]["configuration"] = json!("link.json");
    assert!(load(std::slice::from_ref(&installed)).profiles.is_empty());
    let fifo = installed.path.join("fifo");
    let path =
        std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(fifo.as_os_str())).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    installed.manifest["contributes"]["languages"][0]["configuration"] = json!("fifo");
    let start = std::time::Instant::now();
    let catalog = load(&[installed]);
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    assert!(catalog.profiles.is_empty());
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("regular file"))
    );
}

#[test]
fn json_depth_pair_file_package_and_reference_work_limits_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let installed = package(
        root.path(),
        "fixture.native",
        &format!("{{brackets:{}0{}}}", "[".repeat(65), "]".repeat(65)),
    );
    assert!(
        load(std::slice::from_ref(&installed))
            .warnings
            .iter()
            .any(|warning| warning.contains("nesting"))
    );
    fs::write(
        installed.path.join("language.json"),
        " ".repeat(64 * 1024 + 1),
    )
    .unwrap();
    assert!(
        load(std::slice::from_ref(&installed))
            .warnings
            .iter()
            .any(|warning| warning.contains("64 KiB"))
    );
    fs::write(
        installed.path.join("language.json"),
        json!({"brackets":vec![json!(["{","}"]);65]}).to_string(),
    )
    .unwrap();
    assert!(
        load(std::slice::from_ref(&installed))
            .for_language("cpp")
            .unwrap()
            .brackets
            .as_ref()
            .unwrap()
            .is_empty()
    );
    let mut many = installed.clone();
    many.manifest["contributes"]["languages"] = json!(
        (0..300)
            .map(|n| json!({"id":format!("lang{n}"),"configuration":"language.json"}))
            .collect::<Vec<_>>()
    );
    let catalog = load(&[many]);
    assert_eq!(catalog.profiles.len(), 256);
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("256 language"))
    );
    let catalog = load(&vec![installed; 129]);
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("128 installed"))
    );
}

#[test]
fn actual_read_quota_counts_repeated_and_invalid_configuration_files() {
    let root = tempfile::tempdir().unwrap();
    // Every invalid file is 64 KiB, so failed parsing still consumes the budget.
    let mut installed = package(root.path(), "fixture.native", &"x".repeat(64 * 1024));
    installed.manifest["contributes"]["languages"] = json!(
        (0..100)
            .map(|n| json!({"id":format!("lang{n}"),"configuration":"language.json"}))
            .collect::<Vec<_>>()
    );
    let catalog = load(&[installed]);
    assert!(catalog.profiles.is_empty());
    assert!(
        catalog
            .warnings
            .iter()
            .any(|warning| warning.contains("4 MiB"))
    );
    assert!(catalog.warnings.len() <= 512);
}
