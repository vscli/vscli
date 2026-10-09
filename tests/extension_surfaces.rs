use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    app::{App, Focus, Modal, PromptKind},
    extensions::Package,
    keys::Profile,
};
fn until(app: &mut App, test: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if test(app) {
            return;
        }
        assert!(Instant::now() < deadline, "Timed out: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn key(app: &mut App, key: KeyCode) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
    app.event(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)));
}
fn fixture(root: &Path) -> App {
    let mut app = App::new(root.into(), Profile::Linux);
    app.start_extension_packages(vec![
        Package::read(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extension-surfaces"),
        )
        .unwrap(),
    ])
    .unwrap();
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| {
            h.ready && h.surfaces.statuses.len() == 1 && h.surfaces.trees.len() == 1
        })
    });
    app
}
#[test]
fn output_focus_read_only_status_opaque_action_shared_undo_and_save_integrity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file.txt");
    std::fs::write(&path, "base猫\r\n").unwrap();
    let mut app = fixture(root.path());
    app.open(&path).unwrap();
    let id = app.doc().id;
    app.execute("workbench.action.splitEditor", Value::Null);
    app.doc_mut().insert("dirty", false);
    let before = app.doc().text.to_string();
    let revision = app.doc().revision;
    app.execute("surfaces.show", Value::Null);
    until(&mut app, |a| a.focus == Focus::Output);
    app.event(Event::Paste("DO NOT INSERT".into()));
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc().text.to_string(), before);
    assert_eq!(app.doc().revision, revision);
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), before);
    key(&mut app, KeyCode::Esc);
    assert!(app.focus == Focus::Editor);
    app.execute("surfaces.preserve", Value::Null);
    until(&mut app, |a| a.extension_surfaces.output.is_some());
    assert!(app.focus == Focus::Editor);
    app.execute("vscli.extensions.status", Value::Null);
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| a.message == "Surface applied=true");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.documents.len(), 1);
    assert!(app.panes.iter().all(|p| p.document == id));
    assert_eq!(app.doc().text.to_string(), format!("INSERTED猫{before}"));
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), before);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| vscli::ui::draw(frame, &mut app))
        .unwrap();
    let hit = app.extension_surfaces.status_hits[0].0;
    app.event(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: hit.x,
        row: hit.y,
        modifiers: KeyModifiers::NONE,
    }));
    until(&mut app, |a| {
        a.doc().text == format!("INSERTED猫{before}").as_str()
    });
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), before);
    app.execute("workbench.action.files.save", Value::Null);
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}
#[test]
fn tree_lazy_expand_refresh_cancel_action_and_host_retirement_are_session_owned() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path());
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("original", false);
    let id = app.doc().id;
    app.execute("vscli.extensions.trees", Value::Null);
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| a.surface_tree_rows().len() == 1);
    key(&mut app, KeyCode::Right);
    until(&mut app, |a| a.surface_tree_rows().len() == 2);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    until(&mut app, |a| a.message == "Surface applied=true");
    assert_eq!(app.doc().text.to_string(), "INSERTED猫original");
    app.doc_mut().undo();
    app.execute("surfaces.slow", Value::Null);
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| !h.busy())
    });
    app.execute("vscli.extensions.trees", Value::Null);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    until(&mut app, |a| {
        a.extension_host
            .as_ref()
            .is_some_and(|h| !h.surfaces.tree_busy())
    });
    assert!(app.modal.is_none());
    app.execute("surfaces.refresh", Value::Null);
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| !h.busy())
    });
    app.execute("vscli.extensions.trees", Value::Null);
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| a.surface_tree_rows().len() == 1);
    assert!(
        app.extension_host
            .as_ref()
            .unwrap()
            .surfaces
            .trees
            .values()
            .next()
            .unwrap()
            .nodes
            .values()
            .all(|n| n.label.contains("refreshed"))
    );
    key(&mut app, KeyCode::Esc);
    app.execute("surfaces.crash", Value::Null);
    until(&mut app, |a| a.extension_host.is_none());
    assert!(app.extension_surfaces.tree.is_none());
    assert!(app.extension_surfaces.output.is_none());
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "original");
}
#[test]
fn welcome_surface_actions_and_native_prompt_priority_never_fabricate_documents() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path());
    app.execute("vscli.extensions.status", Value::Null);
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| {
        a.message == "No active editor; no document created"
    });
    assert!(app.documents.is_empty());
    app.execute("workbench.action.showCommands", Value::Null);
    app.prompt.as_mut().unwrap().text = "retained query".into();
    app.execute("surfaces.show", Value::Null);
    until(&mut app, |a| a.extension_surfaces.output.is_some());
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Palette
    ));
    assert_eq!(app.prompt.as_ref().unwrap().text, "retained query");
    assert!(app.focus == Focus::Editor);
    key(&mut app, KeyCode::Esc);
    app.execute("surfaces.prompt", Value::Null);
    until(&mut app, |a| {
        matches!(
            a.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Extension(_))
        )
    });
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| a.message == "Surface input=answer");
    assert!(app.documents.is_empty());
    app.execute("vscli.extensions.trees", Value::Null);
    assert!(matches!(app.modal, Some(Modal::ExtensionSurfaces(_))));
    app.execute("surfaces.dispose", Value::Null);
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| {
            h.surfaces.channels.is_empty()
                && h.surfaces.statuses.is_empty()
                && h.surfaces.trees.is_empty()
        })
    });
    key(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert!(app.documents.is_empty());
}

#[test]
fn stale_status_picker_and_closed_tree_reply_cannot_invoke_newer_actions() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path());
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("retained", false);
    app.execute("vscli.extensions.status", Value::Null);
    // A renderer/picker retains the exact version it presented. A newer update
    // must not turn the user's older selection into a different command.
    app.extension_host
        .as_mut()
        .unwrap()
        .surfaces
        .statuses
        .values_mut()
        .next()
        .unwrap()
        .generation += 1;
    key(&mut app, KeyCode::Enter);
    assert!(app.message.contains("Status action changed"));
    assert_eq!(app.doc().text.to_string(), "retained");
    app.execute("surfaces.slow", Value::Null);
    until(&mut app, |a| {
        a.extension_host.as_ref().is_some_and(|h| !h.busy())
    });
    app.execute("vscli.extensions.trees", Value::Null);
    key(&mut app, KeyCode::Enter);
    assert!(app.extension_host.as_ref().unwrap().surfaces.tree_busy());
    key(&mut app, KeyCode::Esc);
    app.execute("workbench.action.showCommands", Value::Null);
    app.prompt.as_mut().unwrap().text = "newer UI".into();
    until(&mut app, |a| {
        a.extension_host
            .as_ref()
            .is_some_and(|h| !h.surfaces.tree_busy())
    });
    assert!(app.modal.is_none());
    assert_eq!(app.prompt.as_ref().unwrap().text, "newer UI");
    assert_eq!(app.doc().text.to_string(), "retained");
}

#[test]
fn tree_actions_require_the_same_generation_and_rows_that_were_drawn() {
    let root = tempfile::tempdir().unwrap();
    let mut app = fixture(root.path());
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("safe", false);
    app.execute("vscli.extensions.trees", Value::Null);
    key(&mut app, KeyCode::Enter);
    until(&mut app, |a| a.surface_tree_rows().len() == 1);
    key(&mut app, KeyCode::Right);
    until(&mut app, |a| a.surface_tree_rows().len() == 2);
    key(&mut app, KeyCode::Down);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|frame| vscli::ui::draw(frame, &mut app))
        .unwrap();
    app.extension_host
        .as_mut()
        .unwrap()
        .surfaces
        .trees
        .values_mut()
        .next()
        .unwrap()
        .generation += 1;
    // A notification can land between painting and the next input. Do not draw
    // again here: Enter still refers to the row the user actually saw.
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert_eq!(app.message, "Tree view changed; review the refreshed items");
    assert!(matches!(app.modal, Some(Modal::ExtensionTree)));
    assert_eq!(app.doc().text.to_string(), "safe");
}
