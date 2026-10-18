use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
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
        assert!(
            Instant::now() < deadline,
            "Progress fixture timeout: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 32)).unwrap();
    terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn progress(app: &App, text: &str) -> bool {
    app.extension_host.as_ref().is_some_and(|host| {
        host.surfaces
            .statuses
            .values()
            .any(|item| item.visible && item.text == text)
    })
}
#[test]
fn actual_window_progress_receipts_results_failures_and_stop_preserve_shared_dirty_history_and_empty_workbench()
 {
    for empty in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let path = root.join("a.txt");
        std::fs::write(&path, "猫🙂 original\r\n").unwrap();
        let settings = root.join("settings.json");
        std::fs::write(&settings,serde_json::to_vec(&json!({"vscli.languageServer.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}})).unwrap()).unwrap();
        let mut app = App::new(root.into(), Profile::Linux);
        app.configure_settings(Some(settings)).unwrap();
        if !empty {
            app.open(&path).unwrap();
            app.execute("workbench.action.splitEditor", Value::Null);
            app.doc_mut().insert("dirty ", false);
        }
        let before = app
            .active_document()
            .map(|doc| (doc.id, doc.revision, doc.text.to_string()));
        let extension = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/window-progress-extension");
        app.start_extension_packages(vec![Package::read(&extension).unwrap()])
            .unwrap();
        until(&mut app, |app| {
            app.extension_host.as_ref().is_some_and(|host| host.ready)
        });
        app.execute("progress.begin", Value::Null);
        until(&mut app, |app| progress(app, "⟳ Profile"));
        assert!(screen(&mut app).contains("⟳ Profile"));
        app.execute("progress.step", Value::Null);
        until(&mut app, |app| progress(app, "⟳ Profile: phase 猫"));
        assert!(screen(&mut app).contains("⟳ Profile: phase 猫"));
        app.execute("progress.finish", Value::Null);
        until(&mut app, |app| {
            app.message == "Progress value preserved"
                && app
                    .extension_host
                    .as_ref()
                    .is_some_and(|host| !host.busy() && host.surfaces.statuses.is_empty())
        });
        assert!(!screen(&mut app).contains("⟳ Profile"));
        app.execute("progress.fail", Value::Null);
        until(&mut app, |app| app.message == "Progress failure preserved");
        assert!(
            app.extension_host
                .as_ref()
                .unwrap()
                .surfaces
                .statuses
                .is_empty()
        );
        app.execute("progress.begin", Value::Null);
        until(&mut app, |app| progress(app, "⟳ Profile"));
        app.execute("vscli.extensions.stop", Value::Null);
        assert!(app.extension_host.is_none());
        assert!(!screen(&mut app).contains("⟳ Profile"));
        assert_eq!(
            app.active_document()
                .map(|doc| (doc.id, doc.revision, doc.text.to_string())),
            before
        );
        if let Some((id, _, _)) = before {
            assert!(app.panes.iter().all(|pane| pane.document == id));
            app.doc_mut().undo();
            assert_eq!(app.doc().text.to_string(), "猫🙂 original\r\n");
        } else {
            assert!(app.documents.is_empty());
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            "猫🙂 original\r\n".as_bytes()
        );
    }
}
