use serde_json::{Value, json};
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
fn public_editor_all_undo_stop_pairs_bridge_native_typing_and_preserve_shared_crlf_save_history() {
    for before in [false, true] {
        for after in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join("undo.txt");
            let original = "λ🙂base\r\n";
            std::fs::write(&file, original).unwrap();
            let mut app = App::new(root.path().into(), Profile::Linux);
            app.open(&file).unwrap();
            app.execute("workbench.action.splitEditor", Value::Null);
            let identity = app.doc().id;
            let fixture =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor-undo-extension");
            app.start_extension_packages(vec![Package::read(&fixture).unwrap()])
                .unwrap();
            until(&mut app, |app| {
                app.extension_host.as_ref().is_some_and(|host| host.ready)
            });
            app.doc_mut().move_to(6, false);
            app.doc_mut().insert("P", true);
            app.execute(
                "fixture.editorUndo.append",
                json!({"undoStopBefore":before,"undoStopAfter":after}),
            );
            until(&mut app, |app| app.message == "Editor undo edit applied");
            app.doc_mut().insert("B", true);
            assert_eq!(app.doc().text.to_string(), "λ🙂basePAB\r\n");
            app.execute("undo", Value::Null);
            let expected = match (before, after) {
                (_, true) => "λ🙂basePA\r\n",
                (true, false) => "λ🙂baseP\r\n",
                (false, false) => original,
            };
            assert_eq!(
                app.doc().text.to_string(),
                expected,
                "before={before},after={after}"
            );
            app.execute("workbench.action.files.save", Value::Null);
            until(&mut app, |app| !app.saves_pending());
            assert_eq!(std::fs::read(&file).unwrap(), expected.as_bytes());
            app.execute("redo", Value::Null);
            assert_eq!(app.doc().text.to_string(), "λ🙂basePAB\r\n");
            assert_eq!(app.doc().id, identity);
            app.execute("vscli.extensions.stop", Value::Null);
        }
    }
}
