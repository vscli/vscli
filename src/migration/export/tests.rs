use super::*;
use crate::keys::Profile;
use serde_json::json;
use std::sync::atomic::AtomicBool;

fn save(root: &Path, document: &Value) -> PathBuf {
    let path = root.join("setup.code-profile");
    fs::write(&path, serde_json::to_vec_pretty(document).unwrap()).unwrap();
    path
}
fn document() -> Value {
    json!({
        "name":"Personal λ🙂", "icon":"code", "unknownFuture":{"retain":true},
        "settings":json!({"settings":"{// original\r\n\"editor.tabSize\":2,\"unknown.value\":true,}\r\n", "future":true}).to_string(),
        "keybindings":json!({"platform":2,"keybindings":"[// comments\r\n{\"key\":\"f6\",\"command\":\"type\",\"args\":{\"text\":\" imported\"}},]\r\n"}).to_string(),
        "tasks":json!({"tasks":"{\"version\":\"2.0.0\",\"tasks\":[]}"}).to_string(),
        "snippets":json!({"snippets":{"global.code-snippets":"{// retained\r\n\"Lambda\":{\"prefix\":\"lam\",\"body\":\"λ🙂$0\"}}"}}).to_string(),
        "globalState":json!({"storage":{"unknown":"retained"}}).to_string(),
        "extensions":"[]"
    })
}
fn package(root: &Path, id: &str, version: &str, metadata: Value) -> Value {
    let (publisher, name) = id.split_once('.').unwrap();
    let directory = format!(
        "{id}-{version}-{}",
        metadata
            .get("targetPlatform")
            .and_then(Value::as_str)
            .unwrap_or("universal")
    );
    let path = root.join(&directory);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("package.json"), json!({"publisher":publisher,"name":name,"version":version,"main":"main.cjs","engines":{"vscode":"^1.95.0"}}).to_string()).unwrap();
    fs::write(
        path.join("main.cjs"),
        "throw new Error('migration must never execute source code')",
    )
    .unwrap();
    json!({"identifier":{"id":id},"version":version,"relativeLocation":directory,"metadata":metadata})
}
fn index(root: &Path, rows: &[Value]) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("extensions.json"),
        serde_json::to_vec(rows).unwrap(),
    )
    .unwrap();
}
fn apply(preview: Preview, root: &Path, packages: bool) -> Result<Report> {
    preview.apply_controlled(
        root,
        ApplyOptions {
            import_extensions: packages,
            activate_extensions: false,
        },
        &AtomicBool::new(false),
        &mut |_, _, _| {},
    )
}

#[test]
fn configuration_bytes_unknown_fields_and_original_export_survive_atomic_apply_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let document = document();
    let path = save(root.path(), &document);
    let original = fs::read(&path).unwrap();
    let native = root.path().join("native");
    let preview = preview(
        &path,
        Profile::Linux,
        &root.path().join("missing-extensions"),
        None,
    )
    .unwrap();
    assert!(!native.exists());
    assert!(preview.report.unresolved_extensions.is_empty());
    assert_eq!(
        preview.report.profile.as_ref().unwrap().name,
        "Personal λ🙂"
    );
    assert!(
        preview
            .report
            .notices
            .iter()
            .any(|notice| notice.contains("globalState") && notice.contains("not applied"))
    );
    let report = apply(preview, &native, true).unwrap();
    let active = report.activated_profile.unwrap();
    assert_eq!(report.extensions_execution_enabled, 0);
    for resource in ["settings", "keybindings", "tasks"] {
        let wrapper: Value = serde_json::from_str(document[resource].as_str().unwrap()).unwrap();
        assert_eq!(
            fs::read(active.join(format!("{resource}.json"))).unwrap(),
            wrapper[resource].as_str().unwrap().as_bytes()
        );
    }
    let wrapper: Value = serde_json::from_str(document["snippets"].as_str().unwrap()).unwrap();
    assert_eq!(
        fs::read(active.join("snippets/global.code-snippets")).unwrap(),
        wrapper["snippets"]["global.code-snippets"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
    assert_eq!(
        fs::read(active.join("source.code-profile")).unwrap(),
        original
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!active.join("globalState").exists());
    assert_eq!(
        rollback(&native).unwrap(),
        fs::canonicalize(&native).unwrap()
    );
    assert!(active.is_dir());
}

#[test]
fn exact_pinned_versions_release_channels_and_disabled_state_override_unrelated_local_selection() {
    let root = tempfile::tempdir().unwrap();
    let extensions = root.path().join("extensions");
    let rows = [
        package(&extensions, "fixture.pinned", "1.0.0", json!({})),
        package(&extensions, "fixture.pinned", "2.0.0", json!({})),
        package(
            &extensions,
            "fixture.channel",
            "3.0.0",
            json!({"preRelease":false}),
        ),
        package(
            &extensions,
            "fixture.channel",
            "4.0.0",
            json!({"preRelease":true,"isPreReleaseVersion":true}),
        ),
        package(&extensions, "fixture.unselected", "1.0.0", json!({})),
    ];
    index(&extensions, &rows);
    let mut document = document();
    document["extensions"] = json!([
        {"identifier":{"id":"FIXTURE.PINNED"},"version":"1.0.0","disabled":true,"applicationScoped":true},
        {"identifier":{"id":"fixture.channel"},"preRelease":true}
    ]).to_string().into();
    let path = save(root.path(), &document);
    let preview = preview(&path, Profile::Linux, &extensions, None).unwrap();
    let inventory = preview.report.extensions.as_ref().unwrap();
    assert_eq!(inventory.selected.len(), 2);
    assert!(preview.report.unresolved_extensions.is_empty());
    assert_eq!(inventory.selected[0].version, "4.0.0");
    assert_eq!(inventory.selected[1].version, "1.0.0");
    assert!(!inventory.selected[1].enabled);
    assert!(
        preview
            .report
            .notices
            .iter()
            .any(|notice| notice.contains("application-scoped"))
    );
    let native = root.path().join("native");
    let report = apply(preview, &native, true).unwrap();
    assert_eq!(report.extensions_imported, 2);
    assert_eq!(report.extensions_execution_enabled, 0);
    let store =
        crate::extension_store::Store::new(report.activated_profile.unwrap().join("extensions"));
    assert_eq!(store.get("fixture.pinned").unwrap().version, "1.0.0");
    assert!(store.get("fixture.unselected").is_err());
    document["extensions"] = json!([{"identifier":{"id":"fixture.channel"}}])
        .to_string()
        .into();
    let path = save(root.path(), &document);
    let stable = preview_source(
        &discovery::Source {
            user: path.clone(),
            default_user: path,
            extensions: extensions.clone(),
            application: None,
            id: "export".into(),
            name: "ignored".into(),
            product: "VS Code export".into(),
            use_default: Default::default(),
        },
        Profile::Linux,
    )
    .unwrap();
    assert_eq!(
        stable.report.extensions.unwrap().selected[0].version,
        "3.0.0"
    );
}

#[test]
fn all_missing_versions_platforms_and_dependencies_are_reported_before_any_destination_write() {
    let root = tempfile::tempdir().unwrap();
    let extensions = root.path().join("extensions");
    let rows = [
        package(&extensions, "fixture.pinned", "2.0.0", json!({})),
        package(
            &extensions,
            "fixture.platform",
            "1.0.0",
            json!({"targetPlatform":"unknown-platform"}),
        ),
        package(&extensions, "fixture.dependent", "1.0.0", json!({})),
    ];
    let manifest = extensions
        .join(rows[2]["relativeLocation"].as_str().unwrap())
        .join("package.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["extensionDependencies"] = json!(["fixture.dependency"]);
    fs::write(manifest, value.to_string()).unwrap();
    index(&extensions, &rows);
    let mut document = document();
    document["extensions"] = json!([
        {"identifier":{"id":"fixture.missing"}},
        {"identifier":{"id":"fixture.pinned"},"version":"1.0.0"},
        {"identifier":{"id":"fixture.platform"}},
        {"identifier":{"id":"fixture.dependent"}}
    ])
    .to_string()
    .into();
    let path = save(root.path(), &document);
    let native = root.path().join("new-native");
    let previewed = preview(&path, Profile::Linux, &extensions, None).unwrap();
    let unresolved = previewed.report.unresolved_extensions.join("\n");
    for requirement in [
        "fixture.missing",
        "fixture.pinned@1.0.0",
        "platform",
        "fixture.dependency",
    ] {
        assert!(unresolved.contains(requirement), "{unresolved}");
    }
    let error = apply(previewed, &native, true).err().unwrap().to_string();
    assert!(error.contains("no profile was activated"));
    assert!(!native.exists());
    let configuration = apply(
        preview(&path, Profile::Linux, &extensions, None).unwrap(),
        &native,
        false,
    )
    .unwrap();
    let pointer = fs::read(native.join("active-profile.json")).unwrap();
    assert_eq!(configuration.extensions_imported, 0);
    assert!(
        apply(
            preview(&path, Profile::Linux, &extensions, None).unwrap(),
            &native,
            true
        )
        .is_err()
    );
    assert_eq!(
        fs::read(native.join("active-profile.json")).unwrap(),
        pointer
    );
}

#[test]
fn malformed_resources_and_nonportable_or_case_colliding_snippets_fail_during_preview() {
    let root = tempfile::tempdir().unwrap();
    let extensions = root.path().join("missing");
    for name in [
        "../escape.json",
        "/absolute.json",
        "nested/file.json",
        "a\\b.json",
        "CON.json",
        "LPT1.json",
        "file.json ",
        "file.txt",
        "a:b.json",
    ] {
        let mut document = document();
        document["snippets"] = json!({"snippets":{name:"{}"}}).to_string().into();
        let path = save(root.path(), &document);
        assert!(
            preview(&path, Profile::Linux, &extensions, None).is_err(),
            "{name}"
        );
    }
    for (field, value) in [
        ("settings", json!("{broken")),
        ("settings", json!("{\"settings\":{}}")),
        ("settings", json!("{\"settings\":\"[]\"}")),
        ("keybindings", json!("{\"keybindings\":\"{}\"}")),
        ("tasks", json!("[]")),
        (
            "snippets",
            json!("{\"snippets\":{\"a.json\":{},\"b.json\":\"{}\"}}"),
        ),
        (
            "snippets",
            json!("{\"snippets\":{\"A.json\":\"{}\",\"a.json\":\"{}\"}}"),
        ),
        (
            "extensions",
            json!("[{\"identifier\":{\"id\":\"fixture.a\"},\"disabled\":\"no\"}]"),
        ),
        (
            "extensions",
            json!(
                "[{\"identifier\":{\"id\":\"fixture.a\"}},{\"identifier\":{\"id\":\"FIXTURE.A\"}}]"
            ),
        ),
        ("globalState", json!({"not":"a resource string"})),
    ] {
        let mut document = document();
        document[field] = value;
        let path = save(root.path(), &document);
        assert!(
            preview(&path, Profile::Linux, &extensions, None).is_err(),
            "{field}: {}",
            document[field]
        );
    }
    assert!(!root.path().join("escape.json").exists());
}

#[test]
fn null_and_absent_resources_remain_absent_and_cross_platform_keys_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let document = json!({"name":"Empty","settings":"{\"settings\":null}","keybindings":"{\"keybindings\":null,\"platform\":3}","tasks":"{\"tasks\":null}"});
    let path = save(root.path(), &document);
    let report = preview(&path, Profile::Linux, &root.path().join("missing"), None)
        .unwrap()
        .apply(&root.path().join("native"))
        .unwrap();
    let active = report.activated_profile.unwrap();
    assert!(!active.join("settings.json").exists());
    assert!(!active.join("keybindings.json").exists());
    let path = save(root.path(), &super::tests::document());
    let preview = preview(&path, Profile::Windows, &root.path().join("missing"), None).unwrap();
    assert!(
        preview
            .report
            .notices
            .iter()
            .any(|notice| notice.contains("keybinding platform differs"))
    );
}

#[test]
fn export_size_and_decoded_file_budgets_reject_before_activation() {
    let root = tempfile::tempdir().unwrap();
    let mut document = document();
    document["settings"] =
        json!({"settings":format!("{{\"large\":\"{}\"}}", "x".repeat(MAX_FILE))})
            .to_string()
            .into();
    let path = save(root.path(), &document);
    assert!(preview(&path, Profile::Linux, &root.path().join("missing"), None).is_err());
    fs::write(&path, vec![b' '; MAX_TOTAL + 1]).unwrap();
    assert!(preview(&path, Profile::Linux, &root.path().join("missing"), None).is_err());
}

#[test]
fn destination_cannot_modify_source_packages_and_changed_manifest_prevents_activation() {
    let root = tempfile::tempdir().unwrap();
    let extensions = root.path().join("extensions");
    let row = package(&extensions, "fixture.a", "1.0.0", json!({}));
    index(&extensions, std::slice::from_ref(&row));
    let mut document = document();
    document["extensions"] = json!([{"identifier":{"id":"fixture.a"}}])
        .to_string()
        .into();
    let path = save(root.path(), &document);
    assert!(
        apply(
            preview(&path, Profile::Linux, &extensions, None).unwrap(),
            &extensions.join("destination"),
            true
        )
        .is_err()
    );
    assert!(!extensions.join("destination").exists());
    let preview = preview(&path, Profile::Linux, &extensions, None).unwrap();
    fs::write(
        extensions
            .join(row["relativeLocation"].as_str().unwrap())
            .join("package.json"),
        "{}",
    )
    .unwrap();
    let native = root.path().join("native");
    assert!(apply(preview, &native, true).is_err());
    assert!(!native.join("active-profile.json").exists());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        serde_json::to_string_pretty(&document).unwrap()
    );
}

#[test]
fn exported_builtin_disablements_and_pins_apply_without_omitting_enabled_application_defaults() {
    let root = tempfile::tempdir().unwrap();
    let app_root = root.path().join("application/resources/app");
    fs::create_dir_all(app_root.join("extensions")).unwrap();
    fs::write(app_root.join("package.json"), "{\"version\":\"1.118.0\"}").unwrap();
    fs::write(
        app_root.join("product.json"),
        "{\"applicationName\":\"code\"}",
    )
    .unwrap();
    package(
        &app_root.join("extensions"),
        "fixture.default",
        "1.0.0",
        json!({}),
    );
    package(
        &app_root.join("extensions"),
        "fixture.disabled",
        "1.0.0",
        json!({}),
    );
    package(
        &app_root.join("extensions"),
        "fixture.pinned",
        "2.0.0",
        json!({}),
    );
    let extensions = root.path().join("extensions");
    let row = package(&extensions, "fixture.pinned", "1.0.0", json!({}));
    index(&extensions, &[row]);
    let mut document = document();
    document["extensions"] = json!([
        {"identifier":{"id":"fixture.disabled"},"disabled":true},
        {"identifier":{"id":"fixture.pinned"},"version":"1.0.0"}
    ])
    .to_string()
    .into();
    let path = save(root.path(), &document);
    let preview = preview(
        &path,
        Profile::Linux,
        &extensions,
        Some(application::Application::open(&app_root).unwrap()),
    )
    .unwrap();
    assert!(preview.report.unresolved_extensions.is_empty());
    assert_eq!(
        preview
            .report
            .extensions
            .as_ref()
            .unwrap()
            .built_in_packages,
        3
    );
    let packages = &preview.report.extensions.as_ref().unwrap().selected;
    assert_eq!(packages.len(), 3);
    assert!(packages[0].built_in && packages[0].enabled);
    assert!(packages[1].built_in && !packages[1].enabled);
    assert_eq!(packages[2].version, "1.0.0");
    assert!(!packages[2].built_in);
    let native = root.path().join("native");
    let cancel = AtomicBool::new(true);
    assert!(
        preview
            .apply_controlled(&native, ApplyOptions::default(), &cancel, &mut |_, _, _| {})
            .is_err()
    );
    assert!(!native.join("active-profile.json").exists());
}
