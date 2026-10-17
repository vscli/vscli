//! Actual rendered viewport and source-owned public editor presentation operations.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
fn original() -> String {
    let mut text = String::from("猫🙂 header\r\n");
    for line in 1..=96 {
        text.push_str(&format!("    line {line:02} 猫🙂\r\n"));
    }
    text
}
fn draw(app: &mut App, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
}
fn until(app: &mut App, size: Option<(u16, u16)>, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some((width, height)) = size {
            draw(app, width, height);
        }
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Editor presentation fixture timeout: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, id: &str, receipt: &str, size: Option<(u16, u16)>) {
    if let Some((width, height)) = size {
        draw(app, width, height);
        app.poll();
    }
    app.execute(id, Value::Null);
    until(app, size, |app| app.message == receipt);
}
fn ready(root: &Path) -> App {
    std::fs::write(root.join("a.cpp"), original()).unwrap();
    std::fs::write(root.join("b.txt"), b"other\r\n").unwrap();
    let settings = root.join("settings.json");
    std::fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,"files.autoSave":"off",
            "editor.formatOnSave":false,"editor.codeActionsOnSave":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("a.cpp")).unwrap();
    until(&mut app, None, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("a.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    let extension = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/editor-presentation-extension");
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![Package::read(&extension).unwrap()])
        .unwrap();
    until(&mut app, None, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "editorPresentation.shared")
        })
    });
    app
}
fn unchanged(root: &Path) {
    assert_eq!(
        std::fs::read(root.join("a.cpp")).unwrap(),
        original().as_bytes()
    );
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"other\r\n");
}
#[test]
fn public_editor_options_shared_indentation_per_view_styles_auto_alias_and_native_tab_preserve_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    let size = Some((120, 32));
    command(
        &mut app,
        "editorPresentation.shared",
        "API presentation shared;identity=true;focus=first",
        size,
    );
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes.iter().all(|pane| pane.document == id));
    assert_eq!(app.active_pane, 0);
    command(
        &mut app,
        "editorPresentation.options",
        "API presentation options;sharedIndent=true;perPane=true;Interval=3",
        size,
    );
    command(
        &mut app,
        "editorPresentation.auto",
        "API presentation auto;detected=4;tabSizeAlias=true",
        size,
    );
    command(
        &mut app,
        "editorPresentation.explicit",
        "API presentation explicit;tab=4;indent=2;spaces=true",
        size,
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    app.event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    command(
        &mut app,
        "editorPresentation.tab",
        "API presentation native Tab;indent=2;sharedText=true",
        size,
    );
    assert_eq!(
        app.doc().text.to_string(),
        format!("dirty   {}", original())
    );
    assert_eq!(app.doc().id, id);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorPresentation.dirty",
        "API presentation undo;dirtyBaseline=preserved",
        size,
    );
    assert_eq!(app.doc().text.to_string(), format!("dirty {}", original()));
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorPresentation.tab",
        "API presentation native Tab;indent=2;sharedText=true",
        size,
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorPresentation.dirty",
        "API presentation undo;dirtyBaseline=preserved",
        size,
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorPresentation.clean",
        "API presentation original Undo;clean=true",
        size,
    );
    assert_eq!(app.doc().id, id);
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorPresentation.dirty",
        "API presentation undo;dirtyBaseline=preserved",
        size,
    );
    assert_eq!(app.doc().text.to_string(), format!("dirty {}", original()));
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_visible_ranges_actual_draw_inactive_reveal_all_types_resize_and_retirement_preserve_selection()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let cursor = app.doc().cursor;
    let anchor = app.doc().anchor;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    command(
        &mut app,
        "editorPresentation.unrendered",
        "API presentation unrendered;ranges=0",
        None,
    );
    let size = Some((120, 32));
    command(
        &mut app,
        "editorPresentation.rendered",
        "API presentation rendered;ranges=actual;UTF16=true",
        size,
    );
    command(
        &mut app,
        "editorPresentation.shared",
        "API presentation shared;identity=true;focus=first",
        size,
    );
    let first = app.panes[0].id;
    let second = app.panes[1].id;
    let first_top = app.doc().view_state(Some(first)).top;
    let second_cursor = app.doc().view_state(Some(second)).cursor;
    let second_anchor = app.doc().view_state(Some(second)).anchor;
    command(
        &mut app,
        "editorPresentation.revealTop",
        "API presentation reveal top;event=true;inactive=true;selections=preserved",
        size,
    );
    assert_eq!(app.doc().view_state(Some(second)).top, 60);
    assert_eq!(app.doc().view_state(Some(first)).top, first_top);
    command(
        &mut app,
        "editorPresentation.revealCenter",
        "API presentation reveal center;event=true;inactive=true;selections=preserved",
        size,
    );
    let centered = app.doc().view_state(Some(second)).top;
    assert!(centered < 20);
    command(
        &mut app,
        "editorPresentation.revealInside",
        "API presentation reveal inside;quiet=true;selections=preserved",
        size,
    );
    assert_eq!(app.doc().view_state(Some(second)).top, centered);
    command(
        &mut app,
        "editorPresentation.revealDefault",
        "API presentation reveal default;event=true;inactive=true;selections=preserved",
        size,
    );
    assert_eq!(app.doc().view_state(Some(first)).top, first_top);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().anchor, anchor);
    assert_eq!(app.doc().view_state(Some(second)).cursor, second_cursor);
    assert_eq!(app.doc().view_state(Some(second)).anchor, second_anchor);
    command(
        &mut app,
        "editorPresentation.armResize",
        "API presentation resize armed",
        size,
    );
    app.event(Event::Resize(120, 24));
    until(&mut app, Some((120, 24)), |app| {
        app.message == "API presentation resized;event=true;selections=preserved"
    });
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().cursor, cursor);
    assert_eq!(app.doc().anchor, anchor);
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    command(
        &mut app,
        "editorPresentation.retire",
        "API presentation retired;options/reveal=refused;replacement=untouched",
        Some((120, 24)),
    );
    assert_eq!(app.active_pane, 1);
    assert_eq!(app.doc().text.to_string(), "other\r\n");
    let original_doc = app.documents.iter().find(|doc| doc.id == id).unwrap();
    assert_eq!(
        original_doc.text.to_string(),
        format!("dirty {}", original())
    );
    assert_eq!(original_doc.revision, revision);
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
