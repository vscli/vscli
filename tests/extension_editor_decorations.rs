//! Actual public decorations render per native view and preserve shared source history.
use crossterm::event::Event;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::{Buffer, Cell},
    layout::Rect,
    style::{Color, Modifier},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
const ORIGINAL: &str = "猫🙂 alpha\r\nsecond beta\r\n";
const DIRTY: &str = "dirty 猫🙂 alpha\r\nsecond beta\r\n";
const GREEN: Color = Color::Rgb(0x12, 0xab, 0x34);
const BLUE: Color = Color::Rgb(0x23, 0x45, 0xab);
const ORANGE: Color = Color::Rgb(0xd9, 0x87, 0x12);
const WHOLE: Color = Color::Rgb(0x12, 0x23, 0x34);
const PURPLE: Color = Color::Rgb(0xa1, 0x42, 0xf5);
fn draw(app: &mut App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(140, 32)).unwrap();
    terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        draw(app);
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Editor decorations fixture timeout: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, id: &str, receipt: &str) {
    app.execute(id, Value::Null);
    until(app, |app| app.message == receipt);
}
fn ready(root: &Path) -> App {
    std::fs::write(root.join("a.cpp"), ORIGINAL).unwrap();
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
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("a.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    let extension = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/editor-decorations-extension");
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    app.start_extension_packages(vec![Package::read(&extension).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "editorDecorations.shared")
        })
    });
    app
}
fn word(buffer: &Buffer, area: Rect, word: &str) -> (u16, Vec<Cell>) {
    assert!(word.is_ascii());
    let width = u16::try_from(word.len()).unwrap();
    for y in area.y..area.bottom() {
        for x in area.x..=area.right().saturating_sub(width) {
            if word.bytes().enumerate().all(|(offset, byte)| {
                buffer
                    .cell((x + offset as u16, y))
                    .unwrap()
                    .symbol()
                    .as_bytes()
                    == [byte]
            }) {
                return (
                    y,
                    (0..width)
                        .map(|offset| buffer.cell((x + offset, y)).unwrap().clone())
                        .collect(),
                );
            }
        }
    }
    panic!("Rendered native word {word:?} absent from pane {area:?}");
}
fn no_foreground(buffer: &Buffer, area: Rect, color: Color) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            assert_ne!(
                buffer.cell((x, y)).unwrap().fg,
                color,
                "Unexpected stale decoration at {x},{y}"
            );
        }
    }
}
fn unchanged(root: &Path) {
    assert_eq!(
        std::fs::read(root.join("a.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
    assert_eq!(std::fs::read(root.join("b.txt")).unwrap(), b"other\r\n");
}
fn shared_styles(app: &mut App) {
    command(
        app,
        "editorDecorations.shared",
        "API decorations shared;views=independent;source=unchanged",
    );
    let buffer = draw(app);
    assert_eq!(app.panes.len(), 2);
    let (_, first) = word(&buffer, app.pane_areas[0], "alpha");
    let (_, second) = word(&buffer, app.pane_areas[1], "alpha");
    for cell in first {
        assert_eq!(cell.fg, GREEN);
        assert_eq!(cell.bg, BLUE);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }
    for cell in second {
        assert_eq!(cell.fg, ORANGE);
        assert!(
            cell.modifier
                .contains(Modifier::ITALIC | Modifier::UNDERLINED | Modifier::CROSSED_OUT)
        );
    }
    let (row, line) = word(&buffer, app.pane_areas[0], "second");
    assert!(line.iter().all(|cell| cell.bg == WHOLE));
    assert_eq!(
        buffer
            .cell((app.pane_areas[0].right() - 1, row))
            .unwrap()
            .bg,
        WHOLE,
        "whole-line background extends beyond source glyphs"
    );
}
#[test]
fn public_decoration_range_and_options_per_view_render_replace_clear_dispose_preserve_shared_history()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let proof = app.doc().capture_save(root.join("a.cpp")).unwrap();
    shared_styles(&mut app);
    command(
        &mut app,
        "editorDecorations.replace",
        "API decorations replacement;oldRange=removed;second=retained",
    );
    let buffer = draw(&mut app);
    let (_, first) = word(&buffer, app.pane_areas[0], "alpha");
    assert!(first[..2].iter().all(|cell| cell.fg == GREEN));
    assert!(first[2..].iter().all(|cell| cell.fg != GREEN));
    assert!(
        word(&buffer, app.pane_areas[1], "alpha")
            .1
            .iter()
            .all(|cell| cell.fg == ORANGE)
    );
    command(
        &mut app,
        "editorDecorations.clear",
        "API decorations cleared;first=empty;second=retained",
    );
    let buffer = draw(&mut app);
    no_foreground(&buffer, app.pane_areas[0], GREEN);
    assert!(
        word(&buffer, app.pane_areas[1], "alpha")
            .1
            .iter()
            .all(|cell| cell.fg == ORANGE)
    );
    command(
        &mut app,
        "editorDecorations.dispose",
        "API decorations disposed;idempotent=true;forgery=refused",
    );
    let buffer = draw(&mut app);
    for area in &app.pane_areas {
        no_foreground(&buffer, *area, GREEN);
        no_foreground(&buffer, *area, ORANGE);
    }
    assert!(
        word(&buffer, app.pane_areas[0], "second")
            .1
            .iter()
            .all(|cell| cell.bg != WHOLE)
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    let current = app.doc().capture_save(root.join("a.cpp")).unwrap();
    assert_eq!(current.text_epoch(), proof.text_epoch());
    assert_eq!(current.save_generation(), proof.save_generation());
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorDecorations.clean",
        "API decorations original undo;clean=true",
    );
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorDecorations.dirty",
        "API decorations undo;dirtyBaseline=preserved",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    unchanged(&root);
    app.execute("vscli.extensions.stop", Value::Null);
}
#[test]
fn public_decoration_stickiness_native_edit_history_retired_pane_stale_getter_and_host_stop_preserve_source()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut app = ready(&root);
    let id = app.doc().id;
    shared_styles(&mut app);
    command(
        &mut app,
        "editorDecorations.armStart",
        "API decorations native start armed",
    );
    app.event(Event::Paste("Z".into()));
    command(
        &mut app,
        "editorDecorations.inserted",
        "API decorations native insertion;source=true;shared=true",
    );
    let buffer = draw(&mut app);
    let (_, first) = word(&buffer, app.pane_areas[0], "Zalpha");
    let (_, second) = word(&buffer, app.pane_areas[1], "Zalpha");
    assert!(first.iter().all(|cell| cell.fg == GREEN));
    assert_ne!(second[0].fg, ORANGE);
    assert!(second[1..].iter().all(|cell| cell.fg == ORANGE));
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorDecorations.dirty",
        "API decorations undo;dirtyBaseline=preserved",
    );
    let buffer = draw(&mut app);
    assert!(
        word(&buffer, app.pane_areas[0], "alpha")
            .1
            .iter()
            .all(|cell| cell.fg == GREEN)
    );
    assert!(
        word(&buffer, app.pane_areas[1], "alpha")
            .1
            .iter()
            .all(|cell| cell.fg == ORANGE)
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "editorDecorations.inserted",
        "API decorations native insertion;source=true;shared=true",
    );
    let buffer = draw(&mut app);
    assert!(
        word(&buffer, app.pane_areas[0], "Zalpha")
            .1
            .iter()
            .all(|cell| cell.fg == GREEN)
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorDecorations.dirty",
        "API decorations undo;dirtyBaseline=preserved",
    );
    command(
        &mut app,
        "editorDecorations.forcedInsert",
        "API decorations forced insert;edit=true;shared=true",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(
        app.doc().text.to_string(),
        "dirty 猫🙂 Zalpha\r\nsecond beta\r\n"
    );
    let buffer = draw(&mut app);
    let (_, first) = word(&buffer, app.pane_areas[0], "Zalpha");
    let (_, second) = word(&buffer, app.pane_areas[1], "Zalpha");
    // Public TextEditorEdit.insert forces markers past its insertion, even OpenOpen.
    // This differs from the native keyboard insertion qualified above.
    assert_ne!(first[0].fg, GREEN);
    assert!(first[1..].iter().all(|cell| cell.fg == GREEN));
    assert_ne!(second[0].fg, ORANGE);
    assert!(second[1..].iter().all(|cell| cell.fg == ORANGE));
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorDecorations.dirty",
        "API decorations undo;dirtyBaseline=preserved",
    );
    let buffer = draw(&mut app);
    assert!(
        word(&buffer, app.pane_areas[0], "alpha")
            .1
            .iter()
            .all(|cell| cell.fg == GREEN)
    );
    assert!(
        word(&buffer, app.pane_areas[1], "alpha")
            .1
            .iter()
            .all(|cell| cell.fg == ORANGE)
    );
    command(
        &mut app,
        "editorDecorations.reentrant",
        "API decorations reentrant edit;staleRequest=issued;edit=true",
    );
    assert_eq!(app.doc().text.to_string(), format!("stale {DIRTY}"));
    let buffer = draw(&mut app);
    for area in &app.pane_areas {
        no_foreground(&buffer, *area, PURPLE);
    }
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "editorDecorations.reentrantUndo",
        "API decorations reentrant undo;original=retained",
    );
    command(
        &mut app,
        "editorDecorations.retire",
        "API decorations retired;setter=refused;replacement=untouched",
    );
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert_eq!(app.active_pane, 0);
    app.execute("vscli.extensions.stop", Value::Null);
    until(&mut app, |app| app.extension_host.is_none());
    let buffer = draw(&mut app);
    for area in &app.pane_areas {
        no_foreground(&buffer, *area, GREEN);
        no_foreground(&buffer, *area, ORANGE);
        no_foreground(&buffer, *area, PURPLE);
    }
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), ORIGINAL);
    assert!(!app.doc().dirty());
    app.execute("redo", Value::Null);
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), DIRTY);
    assert!(app.doc().dirty());
    unchanged(&root);
}
