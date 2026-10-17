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
fn public_editor_multiline_insert_normalizes_to_original_eol_and_preserves_exact_save_undo_bytes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("native.txt");
    let original = "λ🙂 original\r\n";
    std::fs::write(&file, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&file).unwrap();
    let identity = app.doc().id;
    assert_eq!(app.doc().eol, "\r\n");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-eol-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    app.execute("fixture.editorEol.normalize", Value::Null);
    until(&mut app, |app| {
        app.message == "Multiline editor edit normalized"
    });
    let expected = format!("α\r\nβ\r\nγ\r\n{original}");
    assert_eq!(app.doc().text.to_string(), expected);
    assert_eq!(app.doc().id, identity);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(std::fs::read(&file).unwrap(), expected.as_bytes());
    app.execute("undo", Value::Null);
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(std::fs::read(&file).unwrap(), original.as_bytes());
    assert_eq!(app.doc().id, identity);
    app.execute("vscli.extensions.stop", Value::Null);
}
