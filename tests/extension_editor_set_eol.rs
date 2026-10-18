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
fn ready(root: &Path, original: &str) -> App {
    let file = root.join("eol.txt");
    std::fs::write(&file, original).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.open(&file).unwrap();
    app.execute("workbench.action.splitEditor", Value::Null);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-set-eol-extension");
    app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    app
}
fn save(app: &mut App, root: &Path, expected: &str) {
    app.execute("workbench.action.files.save", Value::Null);
    until(app, |app| !app.saves_pending());
    assert_eq!(
        std::fs::read(root.join("eol.txt")).unwrap(),
        expected.as_bytes()
    );
}
#[test]
fn public_mixed_editor_eol_notifies_before_text_and_ack_and_preserves_separate_native_undo_redo_bytes()
 {
    let root = tempfile::tempdir().unwrap();
    let original = "λ🙂 original\nsecond\n";
    let converted = "λ🙂 original\r\nsecond\r\n";
    let final_text = "λ🙂\r\nX original\r\nsecond\r\n";
    let mut app = ready(root.path(), original);
    let id = app.doc().id;
    app.execute("fixture.editorSetEol.mixed", Value::Null);
    until(&mut app, |app| app.message == "Editor EOL and text applied");
    save(&mut app, root.path(), final_text);
    for (command, expected) in [
        ("undo", converted),
        ("undo", original),
        ("redo", converted),
        ("redo", final_text),
    ] {
        app.execute(command, Value::Null);
        assert_eq!(app.doc().text.to_string(), expected);
        assert_eq!(app.doc().id, id);
        save(&mut app, root.path(), expected);
    }
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_eol_only_last_request_wins_same_eol_preserves_redo_and_real_disk_baseline() {
    let root = tempfile::tempdir().unwrap();
    let original = "λ🙂 original\r\nsecond\r\n";
    let mut app = ready(root.path(), original);
    let id = app.doc().id;
    app.execute("fixture.editorSetEol.only", Value::Null);
    until(&mut app, |app| app.message == "Editor EOL only applied");
    let expected = "λ🙂 original\nsecond\n";
    assert_eq!(app.doc().text.to_string(), expected);
    save(&mut app, root.path(), expected);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    app.execute("fixture.editorSetEol.same", Value::Null);
    until(&mut app, |app| app.message == "Editor EOL unchanged");
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().text.to_string(), expected);
    assert_eq!(app.doc().id, id);
    app.execute("vscli.extensions.stop", Value::Null);
}

#[test]
fn public_plain_editor_edit_cannot_overtake_an_earlier_eol_preparation_from_the_same_source_version()
 {
    let root = tempfile::tempdir().unwrap();
    let original = "λ🙂 original\nsecond\n";
    let mut app = ready(root.path(), original);
    let id = app.doc().id;
    app.execute("fixture.editorSetEol.concurrent", Value::Null);
    until(&mut app, |app| {
        app.message == "Concurrent editor EOL order preserved"
    });
    save(&mut app, root.path(), "λ🙂 original\r\nsecond\r\n");
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), original);
    assert_eq!(app.doc().id, id);
    save(&mut app, root.path(), original);
    app.execute("vscli.extensions.stop", Value::Null);
}
