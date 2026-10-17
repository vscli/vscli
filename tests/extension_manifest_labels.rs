use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{
    app::App,
    extension_activation::Metadata,
    extension_store::{DirectorySource, Store},
    extensions::Package,
    keys::Profile,
};
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/default-manifest-extension")
}
fn until(app: &mut App, test: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if test(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn installed_theme_after_128_packages_and_dormant_command_labels_use_default_nls_without_executing_code()
 {
    let root = tempfile::tempdir().unwrap();
    let source = fixture();
    let original = std::fs::read(source.join("package.json")).unwrap();
    let nls = std::fs::read(source.join("package.nls.json")).unwrap();
    use sha2::{Digest, Sha256};
    let sources = [DirectorySource {
        enabled: true,
        path: source.clone(),
        id: "fixture.default-manifest".into(),
        version: "1.0.0".into(),
        manifest_sha256: format!("{:x}", Sha256::digest(&original)),
    }];
    let installed = Store::new(root.path().join("store"))
        .import_directories(
            &sources,
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap()
        .remove(0);
    let metadata = Metadata::installed(&installed).unwrap();
    assert!(metadata.manifest_warnings.is_empty());
    assert_eq!(
        metadata.commands["fixture.defaultManifest.edit"],
        "Apply localized edit"
    );
    let mut packages = (0..183)
        .map(|_| (root.path().join("unused"), serde_json::json!({})))
        .collect::<Vec<_>>();
    packages.push((installed.path.clone(), installed.manifest.clone()));
    let (choices, warnings) = vscli::theme::contributions(&packages);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].label, "Default translated theme");
    assert_eq!(choices[0].id, "Default translated theme");
    let theme = vscli::theme::Theme::load(&choices[0].path).unwrap();
    assert_eq!(
        theme.colors.background,
        ratatui::style::Color::Rgb(35, 69, 103)
    );
    assert_eq!(
        std::fs::read(source.join("package.json")).unwrap(),
        original
    );
    assert_eq!(std::fs::read(source.join("package.nls.json")).unwrap(), nls);
    assert_eq!(
        std::fs::read(installed.path.join("package.json")).unwrap(),
        original
    );
}
#[test]
fn public_package_json_and_running_command_labels_are_localized_before_actual_edit_save_undo() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native.txt");
    let original = "λ🙂 original\r\n";
    std::fs::write(&path, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let identity = app.doc().id;
    app.start_extension_packages(vec![Package::read(&fixture()).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    assert!(
        app.palette_items("Apply localized edit")
            .iter()
            .any(|(label, _)| label.contains("Apply localized edit"))
    );
    app.execute("fixture.defaultManifest.edit", Value::Null);
    until(&mut app, |app| {
        app.message == "Default manifest edit applied"
    });
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), format!("localized {original}"));
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        format!("localized {original}").as_bytes()
    );
    app.execute("undo", Value::Null);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    assert_eq!(app.doc().id, identity);
    app.execute("vscli.extensions.stop", Value::Null);
}
