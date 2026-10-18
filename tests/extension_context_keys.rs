use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    app::App,
    extensions::Package,
    keys::{Profile, Resolution},
};
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
fn module_context_at_welcome_controls_shortcuts_then_edits_save_undo_and_restart_keep_shared_identity()
 {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("native.txt");
    let original = "λ🙂 original\r\n";
    std::fs::write(&path, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    let package = Package::read(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/context-keys-extension"),
    )
    .unwrap();
    app.start_extension_packages(vec![package]).unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|h| h.ready)
    });
    assert!(app.documents.is_empty());
    let context = app.context();
    assert_eq!(context["fixture.module"], true);
    assert_eq!(context["fixture.zero"], 0);
    assert_eq!(context["fixture.null"], Value::Null);
    assert_eq!(context["editorTextFocus"], false);
    app.open(&path).unwrap();
    let identity = app.doc().id;
    for restart in [false, true] {
        if restart {
            let session = app.extension_host.as_ref().unwrap().session;
            app.execute("vscli.extensions.restart", Value::Null);
            until(&mut app, |app| {
                app.extension_host
                    .as_ref()
                    .is_some_and(|h| h.ready && h.session != session)
            });
        }
        assert_eq!(app.context()["fixture.enabled"], true);
        assert_eq!(
            app.context()["editorTextFocus"],
            true,
            "focused native facts take precedence"
        );
        assert!(
            matches!(app.keymap.resolve("f8",&app.context()),Resolution::Command(id,_)if id=="fixture.context.edit")
        );
        app.execute("fixture.context.invalid", Value::Null);
        until(&mut app, |app| app.message == "Invalid context refused");
        assert_eq!(app.context()["fixture.enabled"], true);
        app.execute("fixture.context.edit", Value::Null);
        until(&mut app, |app| {
            app.message == "Context edit applied and shortcut disabled"
        });
        assert_eq!(app.context()["fixture.enabled"], false);
        assert_eq!(app.doc().text.to_string(), format!("context {original}"));
        assert_eq!(app.doc().id, identity);
        assert!(
            !matches!(app.keymap.resolve("f8",&app.context()),Resolution::Command(id,_)if id=="fixture.context.edit")
        );
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, |app| !app.saves_pending());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("context {original}")
        );
        app.execute("undo", Value::Null);
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, |app| !app.saves_pending());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        app.execute("fixture.context.enable", json!(null));
        until(&mut app, |app| app.message == "Context shortcut enabled");
        assert_eq!(app.context()["fixture.enabled"], true);
        assert!(!app.context().contains_key("fixture.removed"));
        assert_eq!(app.doc().id, identity);
    }
    app.execute("vscli.extensions.stop", Value::Null);
    assert!(!app.context().contains_key("fixture.enabled"));
    assert_eq!(app.doc().text, original);
    assert_eq!(app.doc().id, identity);
}
