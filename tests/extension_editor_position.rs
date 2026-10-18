use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
fn until(app: &mut App, check: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn public_position_replacements_clamp_and_keep_shared_unicode_crlf_save_undo_redo() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("position.txt");
    let original = "λ🙂 original\r\n";
    std::fs::write(&file, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&file).unwrap();
    app.execute("workbench.action.splitEditor", Value::Null);
    let identity = app.doc().id;
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-position-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    app.execute("fixture.editorPosition.replace", Value::Null);
    until(&mut app, |app| {
        app.message == "Position replacement applied"
    });
    let expected = "λ🙂α\r\n original\r\ntail";
    assert_eq!(app.doc().text.to_string(), expected);
    assert_eq!(app.doc().eol, "\r\n");
    for (command, bytes) in [
        (None, expected),
        (Some("undo"), original),
        (Some("redo"), expected),
        (Some("undo"), original),
    ] {
        if let Some(command) = command {
            app.execute(command, Value::Null);
        }
        app.execute("workbench.action.files.save", Value::Null);
        until(&mut app, |app| !app.saves_pending());
        assert_eq!(std::fs::read(&file).unwrap(), bytes.as_bytes());
        assert_eq!(app.doc().id, identity);
    }
    app.execute("vscli.extensions.stop", Value::Null);
}
