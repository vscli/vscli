use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

fn fixture(root: &Path) -> discovery::Source {
    let user = root.join("User");
    let extensions = root.join("source-extensions");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(extensions.join("fixture.native-1.0.0")).unwrap();
    fs::write(
        user.join("settings.json"),
        "{// original\n\"editor.tabSize\":2,}",
    )
    .unwrap();
    fs::write(
        user.join("keybindings.json"),
        r#"[{"key":"f6","command":"type","args":{"text":"imported"}}]"#,
    )
    .unwrap();
    fs::write(extensions.join("fixture.native-1.0.0/package.json"),r#"{"publisher":"fixture","name":"native","version":"1.0.0","main":"main.cjs","engines":{"vscode":"^1.95.0"}}"#).unwrap();
    fs::write(
        extensions.join("fixture.native-1.0.0/main.cjs"),
        "throw new Error('code must not execute during migration')",
    )
    .unwrap();
    fs::write(extensions.join("extensions.json"),serde_json::to_vec(&json!([{"identifier":{"id":"fixture.native"},"version":"1.0.0","relativeLocation":"fixture.native-1.0.0","location":{"scheme":"file","path":extensions.join("fixture.native-1.0.0")}}])).unwrap()).unwrap();
    discovery::default_source("Fixture", &user, &extensions).unwrap()
}
#[test]
fn proposed_api_requirements_are_visible_and_block_grants_before_any_destination_write() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let manifest_path = source.extensions.join("fixture.native-1.0.0/package.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["enabledApiProposals"] = json!(["fixtureProposal"]);
    let original = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(&manifest_path, &original).unwrap();
    let native = root.path().join("native");
    let inspect = || preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let preview = inspect();
    let package = &preview.report.extensions.as_ref().unwrap().selected[0];
    assert!(package.code && package.enabled);
    assert_eq!(package.runtime_blockers.len(), 1);
    assert!(
        preview
            .report
            .extensions
            .as_ref()
            .unwrap()
            .notices
            .iter()
            .any(
                |notice| notice.contains("fixture.native") && notice.contains("activation blocked")
            )
    );
    let options = ApplyOptions {
        import_extensions: true,
        activate_extensions: true,
    };
    let failure = preview
        .apply_controlled(&native, options, &AtomicBool::new(false), &mut |_, _, _| {})
        .err()
        .unwrap()
        .to_string();
    assert!(failure.contains("fixture.native") && failure.contains("proposed APIs"));
    assert!(!native.exists());
    let copied = inspect()
        .apply_controlled(
            &native,
            ApplyOptions {
                activate_extensions: false,
                ..options
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    assert_eq!(copied.extensions_imported, 1);
    assert_eq!(copied.extensions_execution_enabled, 0);
    let pointer = fs::read(native.join("active-profile.json")).unwrap();
    assert!(
        inspect()
            .apply_controlled(&native, options, &AtomicBool::new(false), &mut |_, _, _| {})
            .is_err()
    );
    assert_eq!(
        fs::read(native.join("active-profile.json")).unwrap(),
        pointer
    );
    assert_eq!(fs::read(manifest_path).unwrap(), original);
}

#[test]
fn source_disabled_proposed_packages_copy_without_execution_grants_when_activation_is_requested() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let manifest = source.extensions.join("fixture.native-1.0.0/package.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["enabledApiProposals"] = json!(["fixtureProposal"]);
    fs::write(&manifest, value.to_string()).unwrap();
    fs::create_dir_all(source.user.join("globalStorage")).unwrap();
    let database_path = source.user.join("globalStorage/state.vscdb");
    let database = rusqlite::Connection::open(&database_path).unwrap();
    database.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB); INSERT INTO ItemTable VALUES ('extensionsIdentifiers/disabled', '[{\"id\":\"fixture.native\"}]');").unwrap();
    drop(database);
    let before = fs::read(&database_path).unwrap();
    let preview = preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let package = &preview.report.extensions.as_ref().unwrap().selected[0];
    assert!(!package.enabled && !package.runtime_blockers.is_empty());
    let report = preview
        .apply_controlled(
            &root.path().join("native"),
            ApplyOptions {
                import_extensions: true,
                activate_extensions: true,
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    assert_eq!(report.extensions_imported, 1);
    assert_eq!(report.extensions_execution_enabled, 0);
    assert_eq!(fs::read(database_path).unwrap(), before);
    let prefs: Value = serde_json::from_slice(
        &fs::read(
            report
                .activated_profile
                .unwrap()
                .join("extensions-enabled.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        prefs["extensions"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value != true)
    );
}
#[test]
fn config_and_exact_packages_publish_together_with_explicit_grants_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let native = root.path().join("native");
    fs::create_dir(&native).unwrap();
    fs::write(native.join("settings.json"), "native original").unwrap();
    let options = ApplyOptions {
        import_extensions: true,
        activate_extensions: false,
    };
    let first = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(&native, options, &AtomicBool::new(false), &mut |_, _, _| {})
        .unwrap();
    let directory = first.activated_profile.unwrap();
    assert_eq!(first.extensions_imported, 1);
    assert_eq!(first.extensions_execution_enabled, 0);
    assert_eq!(
        fs::read(directory.join("settings.json")).unwrap(),
        fs::read(source.user.join("settings.json")).unwrap()
    );
    let store =
        crate::extension_store::Store::new(imported_extensions_directory(&directory).unwrap());
    assert_eq!(store.list().unwrap()[0].version, "1.0.0");
    assert!(
        !crate::extension_activation::state::read(&directory.join("extensions-enabled.json"))
            .unwrap()
            .extensions
            .get("fixture.native")
            .copied()
            .unwrap_or(false)
    );
    let second = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(
            &native,
            ApplyOptions {
                activate_extensions: true,
                ..options
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    assert_eq!(second.extensions_execution_enabled, 1);
    assert_eq!(rollback(&native).unwrap(), directory);
    assert_eq!(active_directory(&native).unwrap(), directory);
    assert!(second.activated_profile.unwrap().exists());
    assert_eq!(
        rollback(&native).unwrap(),
        fs::canonicalize(&native).unwrap()
    );
    assert_eq!(
        fs::read_to_string(native.join("settings.json")).unwrap(),
        "native original"
    );
    assert!(store.get("fixture.native").is_ok());
}
#[test]
fn cancellation_after_copy_does_not_activate_or_remove_existing_profile() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let native = root.path().join("native");
    let first = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply(&native)
        .unwrap();
    let pointer = fs::read(native.join("active-profile.json")).unwrap();
    let cancel = AtomicBool::new(false);
    let result = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(
            &native,
            ApplyOptions {
                import_extensions: true,
                activate_extensions: true,
            },
            &cancel,
            &mut |_, _, _| cancel.store(true, Ordering::Release),
        );
    assert!(result.is_err());
    assert_eq!(
        fs::read(native.join("active-profile.json")).unwrap(),
        pointer
    );
    assert!(first.activated_profile.unwrap().is_dir());
    assert!(fs::read_dir(native.join("imports")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".staging-")
    }));
}
#[test]
fn named_profile_resource_inheritance_copies_correct_bytes_without_modifying_sources() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path());
    let named = source.user.join("profiles/named");
    fs::create_dir_all(&named).unwrap();
    fs::write(named.join("keybindings.json"), "[]").unwrap();
    source.user = named;
    source.id = "named".into();
    source
        .use_default
        .insert("settings".into(), Value::Bool(true));
    source
        .use_default
        .insert("extensions".into(), Value::Bool(true));
    let preview = preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let report = preview.apply(&root.path().join("native")).unwrap();
    let active = report.activated_profile.unwrap();
    assert_eq!(
        fs::read_to_string(active.join("keybindings.json")).unwrap(),
        "[]"
    );
    assert_eq!(
        fs::read(active.join("settings.json")).unwrap(),
        fs::read(source.default_user.join("settings.json")).unwrap()
    );
    assert_eq!(report.extensions.unwrap().selected.len(), 1);
}

fn source_application(root: &Path) -> application::Application {
    let app = root.join("source-app/resources/app");
    fs::create_dir_all(app.join("extensions")).unwrap();
    fs::write(app.join("package.json"), r#"{"version":"1.118.0"}"#).unwrap();
    fs::write(app.join("product.json"), r#"{"applicationName":"code"}"#).unwrap();
    application::Application::open(&app).unwrap()
}
fn built_in(
    app: &application::Application,
    name: &str,
    version: &str,
    contributions: Value,
) -> PathBuf {
    let path = app.extensions().join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("package.json"), serde_json::to_vec(&json!({"publisher":"fixture","name":name,"version":version,"contributes":contributions})).unwrap()).unwrap();
    path
}
#[test]
fn built_ins_reconcile_with_registered_versions_and_disabled_data_without_code_grants() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path());
    let app = source_application(root.path());
    // Equal-version built-in wins; a newer registered version wins next preview.
    built_in(&app, "native", "1.0.0", json!({}));
    let data = built_in(
        &app,
        "data",
        "1.0.0",
        json!({"snippets":[{"language":"cpp","path":"snippets.json"}],"languages":[{"id":"cpp","configuration":"typing.json"}],"themes":[{"id":"Builtin Dark","label":"Builtin Dark","uiTheme":"vs-dark","path":"theme.json"}]}),
    );
    let manifest_path = data.join("package.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["main"] = json!("main.cjs");
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::write(
        data.join("main.cjs"),
        "throw new Error('ungranted built-in code must not execute')",
    )
    .unwrap();
    fs::write(
        data.join("snippets.json"),
        r#"{"From built-in":{"prefix":"builtin","body":"BUILTIN_DATA"}}"#,
    )
    .unwrap();
    fs::write(
        data.join("typing.json"),
        r#"{"autoClosingPairs":[["<",">"]]}"#,
    )
    .unwrap();
    fs::write(
        data.join("theme.json"),
        r##"{"colors":{"editor.background":"#123456"}}"##,
    )
    .unwrap();
    let disabled = built_in(
        &app,
        "disabled",
        "1.0.0",
        json!({"snippets":[{"language":"cpp","path":"snippets.json"}],"languages":[{"id":"disabled-language","configuration":"typing.json"}],"themes":[{"id":"Disabled Theme","label":"Disabled Theme","path":"theme.json"}]}),
    );
    fs::write(
        disabled.join("snippets.json"),
        r#"{"Must be hidden":{"prefix":"disabled","body":"DISABLED_DATA"}}"#,
    )
    .unwrap();
    fs::write(
        disabled.join("typing.json"),
        r#"{"autoClosingPairs":[["<",">"]]}"#,
    )
    .unwrap();
    fs::write(
        disabled.join("theme.json"),
        r##"{"colors":{"editor.background":"#abcdef"}}"##,
    )
    .unwrap();
    fs::create_dir_all(source.user.join("globalStorage")).unwrap();
    {
        let database =
            rusqlite::Connection::open(source.user.join("globalStorage/state.vscdb")).unwrap();
        database
            .execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB)")
            .unwrap();
        database
            .execute(
                "INSERT INTO ItemTable VALUES (?1, ?2)",
                rusqlite::params![
                    "extensionsIdentifiers/disabled",
                    r#"[{"id":"fixture.disabled"}]"#
                ],
            )
            .unwrap();
    }
    fs::write(
        source.user.join("settings.json"),
        r#"{"workbench.colorTheme":"Builtin Dark"}"#,
    )
    .unwrap();
    source.application = Some(app.clone());
    let preview = preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let inventory = preview.report.extensions.as_ref().unwrap();
    assert_eq!(inventory.built_in_packages, 3);
    assert_eq!(inventory.selected.len(), 3);
    assert!(
        inventory
            .selected
            .iter()
            .find(|p| p.id == "fixture.native")
            .unwrap()
            .built_in
    );
    assert!(
        preview.theme.is_some(),
        "configuration-only import must snapshot built-in themes"
    );
    let report = preview
        .apply_controlled(
            &root.path().join("native"),
            ApplyOptions {
                import_extensions: true,
                activate_extensions: false,
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    assert_eq!(report.extensions_imported, 3);
    assert_eq!(report.extensions_execution_enabled, 0);
    let profile = report.activated_profile.unwrap();
    let installed = crate::extension_store::Store::new(profile.join("extensions"))
        .list()
        .unwrap();
    assert!(
        !installed
            .iter()
            .find(|p| p.id == "fixture.disabled")
            .unwrap()
            .enabled
    );
    let snippets = crate::snippet::catalog::Catalog::load_with_extensions(
        None,
        root.path(),
        "cpp",
        Some(&profile.join("extensions")),
    );
    assert_eq!(
        snippets
            .entries
            .iter()
            .map(|e| e.body.as_str())
            .collect::<Vec<_>>(),
        ["BUILTIN_DATA"]
    );
    let grants =
        crate::extension_activation::state::read(&profile.join("extensions-enabled.json")).unwrap();
    assert!(grants.extensions.is_empty());
    let language =
        crate::language_configuration::Catalog::load(&installed, &grants, &Default::default());
    assert!(language.for_language("cpp").is_some());
    assert!(language.for_language("disabled-language").is_none());
    assert!(!source.user.join("globalStorage/state.vscdb-wal").exists());
    assert!(!source.user.join("globalStorage/state.vscdb-shm").exists());
    // Reconcile the registered generation against the built-in after an upgrade.
    let path = source.extensions.join("fixture.native-1.0.0/package.json");
    let manifest = fs::read_to_string(&path).unwrap().replace("1.0.0", "2.0.0");
    fs::write(&path, manifest).unwrap();
    let index = source.extension_index();
    let bytes = fs::read_to_string(&index)
        .unwrap()
        .replace("\"version\":\"1.0.0\"", "\"version\":\"2.0.0\"");
    fs::write(index, bytes).unwrap();
    let next = packages::inventory(&source).unwrap();
    let selected = next
        .selected
        .iter()
        .find(|p| p.id == "fixture.native")
        .unwrap();
    assert_eq!(selected.version, "2.0.0");
    assert!(!selected.built_in);
}

#[test]
fn built_in_only_profile_and_catalog_above_execution_limit_are_usable_and_bounded() {
    let root = tempfile::tempdir().unwrap();
    let mut source = fixture(root.path());
    fs::remove_file(source.extension_index()).unwrap();
    let app = source_application(root.path());
    for index in 0..184 {
        built_in(&app, &format!("data-{index:03}"), "1.0.0", json!({}));
    }
    source.application = Some(app);
    let inventory = packages::inventory(&source).unwrap();
    assert_eq!(inventory.selected.len(), 184);
    let report = preview_profile(&source, crate::keys::Profile::Linux)
        .unwrap()
        .apply_controlled(
            &root.path().join("native"),
            ApplyOptions {
                import_extensions: true,
                activate_extensions: false,
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    let profile = report.activated_profile.unwrap();
    let installed = crate::extension_store::Store::new(profile.join("extensions"))
        .list()
        .unwrap();
    assert_eq!(installed.len(), 184);
    let catalog = installed
        .iter()
        .map(|p| {
            Ok((
                p.id.clone(),
                crate::extension_activation::Metadata::installed(p)?,
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()
        .unwrap();
    assert!(
        crate::extension_activation::plan(&catalog, &Default::default(), &[], &Default::default())
            .unwrap()
            .ordered
            .is_empty()
    );
    let overflow = (0..129).map(|i| format!("fixture.data-{i:03}")).collect();
    assert!(crate::extension_activation::plan(&catalog, &overflow, &[], &overflow).is_err());
    assert_eq!(crate::extension_activation::MAX_SELECTED, 128);
}

#[test]
fn unpacked_theme_without_user_index_remains_data_only_even_when_package_copy_is_requested() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    fs::remove_file(source.extension_index()).unwrap();
    let theme = source.extensions.join("unpacked-theme");
    fs::create_dir(&theme).unwrap();
    fs::write(theme.join("package.json"), r#"{"main":"main.cjs","contributes":{"themes":[{"id":"unpacked.theme","label":"Unpacked Theme","path":"theme.json"}]}}"#).unwrap();
    fs::write(
        theme.join("main.cjs"),
        "throw new Error('unregistered code must not be imported')",
    )
    .unwrap();
    fs::write(
        theme.join("theme.json"),
        r##"{"colors":{"editor.foreground":"#123456"}}"##,
    )
    .unwrap();
    fs::write(
        source.user.join("settings.json"),
        r#"{"workbench.colorTheme":"unpacked.theme"}"#,
    )
    .unwrap();
    let preview = preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let theme_file = preview.theme.as_ref().unwrap().1.clone();
    assert!(
        preview
            .report
            .extensions
            .as_ref()
            .unwrap()
            .selected
            .is_empty()
    );
    let report = preview
        .apply_controlled(
            &root.path().join("native"),
            ApplyOptions {
                import_extensions: true,
                activate_extensions: false,
            },
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap();
    assert_eq!(report.extensions_imported, 0);
    assert_eq!(report.extensions_execution_enabled, 0);
    let profile = report.activated_profile.unwrap();
    assert!(
        crate::extension_store::Store::new(profile.join("extensions"))
            .list()
            .unwrap()
            .is_empty()
    );
    let copied_theme = crate::theme::Theme::load(&profile.join(theme_file)).unwrap();
    assert_eq!(
        copied_theme.colors.foreground,
        ratatui::style::Color::Rgb(18, 52, 86)
    );
    assert_eq!(
        fs::read_to_string(theme.join("main.cjs")).unwrap(),
        "throw new Error('unregistered code must not be imported')"
    );
}

#[test]
fn packaged_default_theme_label_without_id_matches_imported_setting_and_preserves_source_bytes() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture(root.path());
    let package = source.extensions.join("fixture.native-1.0.0");
    let manifest=br#"{"publisher":"fixture","name":"native","version":"1.0.0","main":"main.cjs","contributes":{"themes":[{"label":"%themeLabel%","path":"theme.json"}]}}"#;
    let translations =
        br#"{"themeLabel":{"message":"Default translated theme","comment":["Display label"]}}"#;
    let settings =
        b"{// preserved comment\n\"workbench.colorTheme\":\"Default translated theme\",}";
    fs::write(package.join("package.json"), manifest).unwrap();
    fs::write(package.join("package.nls.json"), translations).unwrap();
    fs::write(
        package.join("theme.json"),
        r##"{"colors":{"editor.foreground":"#123456"}}"##,
    )
    .unwrap();
    fs::write(source.user.join("settings.json"), settings).unwrap();
    let code = fs::read(package.join("main.cjs")).unwrap();
    let preview = preview_profile(&source, crate::keys::Profile::Linux).unwrap();
    let theme_file = preview.theme.as_ref().unwrap().1.clone();
    let report = preview.apply(&root.path().join("native")).unwrap();
    let profile = report.activated_profile.unwrap();
    assert_eq!(report.extensions_execution_enabled, 0);
    assert_eq!(fs::read(profile.join("settings.json")).unwrap(), settings);
    assert_eq!(
        crate::theme::Theme::load(&profile.join(theme_file))
            .unwrap()
            .colors
            .foreground,
        ratatui::style::Color::Rgb(18, 52, 86)
    );
    assert_eq!(fs::read(package.join("package.json")).unwrap(), manifest);
    assert_eq!(
        fs::read(package.join("package.nls.json")).unwrap(),
        translations
    );
    assert_eq!(fs::read(package.join("main.cjs")).unwrap(), code);
    assert_eq!(
        fs::read(source.user.join("settings.json")).unwrap(),
        settings
    );
}
