use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn preferred_extension_language_loads_before_module_evaluation_and_survives_restart_and_native_undo()
 {
    for (language, prefix, notice) in [
        ("en", "Hello 猫🙂 ", "Ready/Applied true;language=en"),
        ("fr", "Bonjour 猫🙂 ", "Prêt/Appliqué true;language=fr"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native.txt");
        let original = "λ🙂 original\r\n";
        std::fs::write(&path, original).unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.configure_locale(language).unwrap();
        app.open(&path).unwrap();
        let identity = app.doc().id;
        let package = Package::read(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/localization-extension"),
        )
        .unwrap();
        app.start_extension_packages(vec![package]).unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        let initial_session = app.extension_host.as_ref().unwrap().session;
        assert!(app.configure_locale("de").is_err());
        for restart in [false, true] {
            if restart {
                app.execute("vscli.extensions.restart", Value::Null);
                until(&mut app, |app| {
                    app.extension_host
                        .as_ref()
                        .is_some_and(|host| host.ready && host.session != initial_session)
                });
            }
            app.execute("fixture.localized", Value::Null);
            until(&mut app, |app| app.message == notice);
            assert_eq!(app.doc().text.to_string(), format!("{prefix}{original}"));
            assert_eq!(app.doc().id, identity);
            app.execute("workbench.action.files.save", Value::Null);
            until(&mut app, |app| !app.saves_pending());
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                format!("{prefix}{original}")
            );
            app.execute("undo", Value::Null);
            until(&mut app, |app| app.doc().text == original);
            app.execute("workbench.action.files.save", Value::Null);
            until(&mut app, |app| !app.saves_pending());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert_eq!(app.doc().id, identity);
        }
    }
}
