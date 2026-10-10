//! Real App/render/worker qualification; register only under app::saving cfg(test).
use super::*;
use crate::{document::Selection, editor_groups::GroupId, editor_presentation::GeometryProof};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::{Position, Rect},
};
use serde_json::Value;
use std::{
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

const ORIGINAL: &str = "猫🙂 first\r\nbody λ\r\nlast\r\n";
const WAIT: Duration = Duration::from_secs(5);
struct Fixture {
    _root: tempfile::TempDir,
    path: PathBuf,
    app: App,
    document: u64,
}
impl Fixture {
    fn new(text: &str) -> Self {
        Self::named(text, "shared.txt")
    }
    fn named(text: &str, filename: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let path = root.join(filename);
        std::fs::write(&path, text).unwrap();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.extension_node = "vscli-fixture-node-not-installed".into();
        app.configure_language_services(None, true).unwrap();
        app.sidebar = false;
        app.open(&path).unwrap();
        until(&mut app, "initial editor", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&path))
        });
        let document = app.doc().id;
        Self {
            _root: directory,
            path,
            app,
            document,
        }
    }
    fn nested(&mut self) -> [GroupId; 3] {
        self.app.doc_mut().move_to(1, false);
        let left = self.app.editor_groups.active_group().unwrap();
        self.app
            .execute("workbench.action.splitEditorRight", Value::Null);
        self.app.doc_mut().move_to(12, false);
        let upper = self.app.editor_groups.active_group().unwrap();
        self.app
            .execute("workbench.action.splitEditorDown", Value::Null);
        self.app.doc_mut().move_to(20, false);
        let lower = self.app.editor_groups.active_group().unwrap();
        assert_eq!(self.app.documents.len(), 1);
        assert_eq!(self.app.editor_groups.groups().len(), 3);
        [left, upper, lower]
    }
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{phase}: {}",
            app.save_fixture_status()
        );
        std::thread::yield_now();
    }
}
fn draw(app: &mut App, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
}
fn proof(app: &App) -> GeometryProof {
    let proof = app.editor_presentation.proof().unwrap().clone();
    assert!(
        app.editor_presentation
            .current(&proof, app.editor_layout(), app.editor_groups())
    );
    proof
}
fn click(app: &mut App, point: Position) {
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: point.x,
        row: point.y,
        modifiers: KeyModifiers::NONE,
    }));
}
fn text_point(app: &App) -> Position {
    let group = app.editor_groups.active_group().unwrap();
    let pane = app
        .editor_presentation
        .panes()
        .iter()
        .flatten()
        .find(|pane| pane.group == group)
        .unwrap();
    assert!(pane.text.width >= 3 && pane.text.height >= 1);
    Position::new(pane.text.x + 2, pane.text.y)
}
fn assert_retained(f: &Fixture, text: &str) {
    assert_eq!(f.app.documents.len(), 1);
    assert_eq!(f.app.doc().id, f.document);
    assert_eq!(f.app.doc().text.to_string(), text);
    assert_eq!(std::fs::read(&f.path).unwrap(), ORIGINAL.as_bytes());
    assert!(f.app.editor_groups.groups().iter().all(|group| {
        group
            .active()
            .is_some_and(|tab| tab.document() == f.document)
    }));
}

#[test]
fn nested_actual_drawing_retains_distinct_unicode_crlf_carets_secondary_history_and_dirty_identity()
{
    let mut f = Fixture::new(ORIGINAL);
    let groups = f.nested();
    f.app.doc_mut().secondary = vec![Selection::caret(12)];
    let views: Vec<_> = groups
        .iter()
        .map(|group| f.app.doc().view_state(Some(group.value())).clone())
        .collect();
    let epoch = f.app.doc().text_epoch();
    let revision = f.app.doc().revision;
    let generation = f.app.doc().save_generation();
    let layout_generation = f.app.editor_layout().generation();
    for (width, height) in [(120, 36), (34, 12), (20, 6), (120, 36)] {
        draw(&mut f.app, width, height);
        assert_retained(&f, ORIGINAL);
        assert_eq!(f.app.doc().revision, revision);
        assert_eq!(f.app.doc().text_epoch(), epoch);
        assert_eq!(f.app.doc().save_generation(), generation);
        assert_eq!(f.app.editor_layout().generation(), layout_generation);
        for (group, expected) in groups.iter().zip(&views) {
            let actual = f.app.doc().view_state(Some(group.value()));
            assert_eq!(actual.cursor, expected.cursor);
            assert_eq!(actual.anchor, expected.anchor);
            assert_eq!(actual.secondary, expected.secondary);
        }
        proof(&f.app);
    }
    f.app.event(Event::Paste("X".into()));
    let changed = "猫🙂 first\r\nboXdy λ\r\nlaXst\r\n";
    assert_retained(&f, changed);
    assert!(f.app.doc().dirty());
    assert!(f.app.editor_presentation.proof().is_none());
    f.app.execute("undo", Value::Null);
    assert_retained(&f, ORIGINAL);
    f.app.execute("redo", Value::Null);
    assert_retained(&f, changed);
    draw(&mut f.app, 120, 36);
    let view = f.app.doc().selections();
    let id = f.app.doc().id;
    f.app
        .execute("workbench.action.increaseViewHeight", Value::Null);
    assert_retained(&f, changed);
    assert_eq!(f.app.doc().selections(), view);
    assert_eq!(f.app.doc().id, id);
    f.app.execute("undo", Value::Null);
    assert_retained(&f, ORIGINAL);
}

#[test]
fn tiny_active_only_and_whole_screen_fallback_preserve_tree_group_ids_and_shared_models() {
    let mut f = Fixture::new(ORIGINAL);
    let groups = f.nested();
    let ids = f.app.editor_layout().groups();
    let saved = f.app.editor_layout().export(&ids).unwrap();
    let cursor = f.app.doc().cursor;
    // With only five editor rows, nested allocation remains bounded and may
    // use hard minima. Direct 1x1 projection proves active-only without changing
    // model state; whole-screen tiny UI intentionally authorizes no input.
    let geometry = f.app.project_editor_layout(Rect::new(4, 3, 1, 1)).unwrap();
    assert!(geometry.active_only());
    assert_eq!(
        geometry
            .placements()
            .iter()
            .flatten()
            .filter(|placement| placement.outer.width != 0 && placement.outer.height != 0)
            .count(),
        1
    );
    assert!(geometry.dividers().iter().all(Option::is_none));
    draw(&mut f.app, 120, 36);
    let old = proof(&f.app);
    draw(&mut f.app, 19, 5);
    assert!(f.app.editor_presentation.proof().is_none());
    assert!(f.app.tab_hits.is_empty());
    assert!(f.app.pane_areas.is_empty());
    assert_eq!(f.app.editor_layout().groups(), ids);
    assert_eq!(f.app.editor_layout().export(&ids).unwrap(), saved);
    assert_eq!(f.app.doc().cursor, cursor);
    assert_retained(&f, ORIGINAL);
    draw(&mut f.app, 120, 36);
    assert_ne!(proof(&f.app), old);
    assert_eq!(
        f.app
            .editor_groups
            .groups()
            .iter()
            .map(|group| group.id())
            .collect::<Vec<_>>(),
        groups
    );
    assert_eq!(f.app.editor_layout().export(&ids).unwrap(), saved);
}

#[test]
fn resize_sidebar_panel_and_inspector_aba_retire_original_hit_authorization_before_repaint() {
    let mut f = Fixture::new(ORIGINAL);
    f.nested();
    draw(&mut f.app, 120, 36);
    let old = proof(&f.app);
    let point = text_point(&f.app);
    let selections = f.app.doc().selections();
    f.app.event(Event::Resize(100, 30));
    f.app.event(Event::Resize(120, 36));
    assert!(f.app.editor_presentation.proof().is_none());
    click(&mut f.app, point);
    assert_eq!(f.app.doc().selections(), selections);
    draw(&mut f.app, 120, 36);
    assert_ne!(proof(&f.app), old);
    for command in [
        "workbench.action.toggleSidebarVisibility",
        "workbench.action.togglePanel",
    ] {
        let original = proof(&f.app);
        let point = text_point(&f.app);
        // Panel visibility is an actual App accepted state mutation, but no
        // shell process is needed to qualify presentation invalidation.
        if command.ends_with("togglePanel") {
            f.app.terminal_visible = true;
            f.app.event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }));
            f.app.terminal_visible = false;
            f.app.event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }));
        } else {
            f.app.execute(command, Value::Null);
            f.app.execute(command, Value::Null);
        }
        assert!(f.app.editor_presentation.proof().is_none());
        click(&mut f.app, point);
        assert_eq!(f.app.doc().selections(), selections);
        draw(&mut f.app, 120, 36);
        assert_ne!(proof(&f.app), original);
    }
    let original = proof(&f.app);
    f.app.execute("vscli.keyboardInspector", Value::Null);
    assert!(matches!(f.app.modal, Some(Modal::Inspector)));
    draw(&mut f.app, 120, 36);
    assert!(f.app.editor_presentation.proof().is_none());
    f.app
        .event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(f.app.modal.is_none());
    click(&mut f.app, point);
    assert_eq!(f.app.doc().selections(), selections);
    draw(&mut f.app, 120, 36);
    assert_ne!(proof(&f.app), original);
    assert_retained(&f, ORIGINAL);
}

#[test]
fn actual_gutter_digit_change_and_same_batch_edit_retire_old_source_mapping_without_losing_undo() {
    let original = "x\r\n".repeat(998);
    let mut f = Fixture::new(&original);
    draw(&mut f.app, 100, 28);
    let old = proof(&f.app);
    let group = f.app.editor_groups.active_group().unwrap();
    let old_text = f
        .app
        .editor_presentation
        .panes()
        .iter()
        .flatten()
        .find(|pane| pane.group == group)
        .unwrap()
        .text;
    let point = Position::new(old_text.x + 2, old_text.y);
    f.app.event(Event::Paste("α\r\nβ\r\n".into()));
    assert!(f.app.doc().line_count() >= 1000);
    let after_edit = f.app.doc().selections();
    let edited = f.app.doc().text.to_string();
    assert!(f.app.editor_presentation.proof().is_none());
    click(&mut f.app, point);
    assert_eq!(f.app.doc().selections(), after_edit);
    assert_eq!(f.app.doc().text.to_string(), edited);
    draw(&mut f.app, 100, 28);
    let new = proof(&f.app);
    assert_ne!(new, old);
    let new_text = f
        .app
        .editor_presentation
        .panes()
        .iter()
        .flatten()
        .find(|pane| pane.group == group)
        .unwrap()
        .text;
    assert_eq!(new_text.x, old_text.x + 1);
    f.app.execute("undo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), original);
    f.app.execute("redo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), edited);
    // An edit that does not change any rectangle also retires the old source
    // map in the same drained input batch, rather than interpreting stale rows.
    draw(&mut f.app, 100, 28);
    let point = text_point(&f.app);
    f.app.event(Event::Paste("Z".into()));
    let after = f.app.doc().selections();
    let latest = f.app.doc().text.to_string();
    assert!(f.app.editor_presentation.proof().is_none());
    click(&mut f.app, point);
    assert_eq!(f.app.doc().selections(), after);
    assert_eq!(f.app.doc().text.to_string(), latest);
    assert_eq!(std::fs::read(&f.path).unwrap(), original.as_bytes());
}

struct Gate {
    entered: Receiver<crate::save_worker::GatePoint>,
    release: SyncSender<()>,
}
impl Gate {
    fn reach(&self, app: &mut App) {
        let deadline = Instant::now() + WAIT;
        loop {
            app.poll();
            match self.entered.try_recv() {
                Ok(point) => {
                    assert_eq!(point, crate::save_worker::GatePoint::BeforeCommit);
                    return;
                }
                Err(TryRecvError::Disconnected) => panic!(
                    "actual save gate disconnected: {}",
                    app.save_fixture_status()
                ),
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "actual save gate: {}",
                app.save_fixture_status()
            );
            std::thread::yield_now();
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.release.try_send(());
    }
}
#[test]
fn actual_ratio_resize_preserves_authorized_save_and_native_original_view_request_proof() {
    // The genuine framed peer is a C++ server; the opened fixture must belong
    // to its real synchronized language pool before requesting hover.
    let mut f = Fixture::named(ORIGINAL, "shared.cpp");
    f.nested();
    f.app.event(Event::Paste("X".into()));
    let captured = f.app.doc().text.to_string();
    let selections = f.app.doc().selections();
    let text_epoch = f.app.doc().text_epoch();
    let revision = f.app.doc().revision;
    let save_generation = f.app.doc().save_generation();
    let groups = f.app.editor_groups.proof();
    // A genuine framed native peer yields an original source/view-owned Request.
    // Its proof must stay usable across a ratio-only change; group changes would
    // retire it through Client::invalidate_editor_views.
    let mut client = crate::lsp::outline_tests::start(f._root.path(), f.app.doc());
    let view = f.app.editor_groups.active_group().unwrap().value();
    client
        .request_in_view("textDocument/hover", f.app.doc(), Value::Null, Some(view))
        .unwrap();
    let responses = crate::lsp::outline_tests::until(&mut client, |_, events| {
        events.iter().any(|event|matches!(event,crate::lsp::Event::Response(request,_) if request.method=="textDocument/hover"))
    });
    let request = responses
        .into_iter()
        .find_map(|event| match event {
            crate::lsp::Event::Response(request, _) => Some(request),
            _ => None,
        })
        .unwrap();
    assert!(client.request_current(&request));
    f.app.lsp = Some(client);
    let (worker, entered, release) =
        Worker::fixture_gated(vec![crate::save_worker::GatePoint::BeforeCommit]);
    f.app.replace_save_worker_fixture(worker);
    let gate = Gate { entered, release };
    f.app.execute("workbench.action.files.save", Value::Null);
    gate.reach(&mut f.app);
    assert!(f.app.native_save_worker_busy());
    assert_eq!(std::fs::read(&f.path).unwrap(), ORIGINAL.as_bytes());
    draw(&mut f.app, 120, 36);
    let before = f.app.editor_layout().generation();
    let geometry = f.app.editor_presentation.geometry().unwrap().clone();
    let group = f.app.editor_groups.active_group().unwrap();
    let old_height = geometry.placement(group).unwrap().outer.height;
    f.app
        .execute("workbench.action.increaseViewHeight", Value::Null);
    assert!(
        f.app.editor_layout().generation() > before,
        "{}",
        f.app.message
    );
    assert!(f.app.editor_presentation.proof().is_none());
    assert_eq!(f.app.editor_groups.proof(), groups);
    assert_eq!(f.app.doc().id, f.document);
    assert_eq!(f.app.doc().text_epoch(), text_epoch);
    assert_eq!(f.app.doc().revision, revision);
    assert_eq!(f.app.doc().selections(), selections);
    assert!(f.app.lsp.as_ref().unwrap().request_current(&request));
    assert!(f.app.native_save_worker_busy());
    draw(&mut f.app, 120, 36);
    assert_eq!(
        f.app
            .editor_presentation
            .geometry()
            .unwrap()
            .placement(group)
            .unwrap()
            .outer
            .height,
        old_height + 2
    );
    gate.release.try_send(()).unwrap();
    until(
        &mut f.app,
        "authorized save receipt after ratio resize",
        |app| !app.saves_pending(),
    );
    assert_eq!(f.app.doc().save_generation(), save_generation + 1);
    assert_eq!(f.app.doc().text_epoch(), text_epoch);
    assert_eq!(f.app.doc().text.to_string(), captured);
    assert_eq!(std::fs::read(&f.path).unwrap(), captured.as_bytes());
    assert!(!f.app.doc().dirty());
    f.app.execute("undo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), ORIGINAL);
    assert!(f.app.doc().dirty());
    f.app.execute("redo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), captured);
    assert!(!f.app.doc().dirty());
    assert_eq!(std::fs::read(&f.path).unwrap(), captured.as_bytes());
}

#[test]
fn recovered_overflow_renders_and_edits_authoritative_model_without_fake_hits_and_respects_save_inventory_bound()
 {
    let mut f = Fixture::new(ORIGINAL);
    // Actual recovery/model import is a setup action, outside observational draw.
    f.app
        .documents
        .extend((0..128).map(|index| Document::from_text(&format!("recovered {index}"))));
    f.app.sync_pane();
    assert!(f.app.editor_group_overflow());
    assert_eq!(f.app.documents.len(), 129);
    let ids: Vec<_> = f.app.documents.iter().map(|doc| doc.id).collect();
    let views = f.app.doc().selections();
    let revision = f.app.doc().revision;
    let epoch = f.app.doc().text_epoch();
    let groups = f.app.editor_groups.proof();
    let layout = f.app.editor_layout().generation();
    let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
    terminal
        .draw(|frame| crate::ui::draw(frame, &mut f.app))
        .unwrap();
    let visible: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(visible.contains("Recovery"));
    assert!(visible.contains("猫"));
    assert!(visible.contains("🙂"));
    assert!(visible.contains("first"));
    assert!(!visible.contains("Editor layout unavailable"));
    assert!(f.app.editor_presentation.proof().is_none());
    assert!(f.app.tab_hits.is_empty());
    assert!(f.app.pane_areas.is_empty());
    assert_eq!(
        f.app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(f.app.doc().selections(), views);
    assert_eq!(f.app.doc().revision, revision);
    assert_eq!(f.app.doc().text_epoch(), epoch);
    assert_eq!(f.app.editor_groups.proof(), groups);
    assert_eq!(f.app.editor_layout().generation(), layout);
    let area = f.app.editor_area;
    assert!(area.width > 4 && area.height > 0);
    click(&mut f.app, Position::new(area.x + 3, area.y));
    assert_eq!(f.app.doc().selections(), views);
    f.app.event(Event::Paste("Z".into()));
    let changed = format!("Z{ORIGINAL}");
    assert_eq!(f.app.doc().id, f.document);
    assert_eq!(f.app.doc().text.to_string(), changed);
    f.app.execute("undo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), ORIGINAL);
    f.app.execute("redo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), changed);
    // More than128 retained models is a real pre-existing no-clobber inventory
    // bound. Refusal is explicit; no dropped model or synchronous bypass.
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    assert!(!f.app.saves_pending());
    assert!(f.app.message.contains("128"), "{}", f.app.message);
    assert_eq!(std::fs::read(&f.path).unwrap(), ORIGINAL.as_bytes());
    assert_eq!(
        f.app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
        ids
    );
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::PageDown,
        KeyModifiers::CONTROL,
    )));
    let explicitly_closed = f.app.doc().id;
    assert_ne!(explicitly_closed, f.document);
    assert!(!f.app.doc().dirty());
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    assert!(f.app.modal.is_none());
    assert_eq!(f.app.documents.len(), 128);
    assert!(
        !f.app
            .documents
            .iter()
            .any(|doc| doc.id == explicitly_closed)
    );
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::PageUp,
        KeyModifiers::CONTROL,
    )));
    assert_eq!(f.app.doc().id, f.document);
    assert_eq!(f.app.doc().text.to_string(), changed);
    assert!(f.app.editor_group_overflow());
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    until(&mut f.app, "bounded fallback save receipt", |app| {
        !app.saves_pending()
    });
    assert_eq!(std::fs::read(&f.path).unwrap(), changed.as_bytes());
    assert!(!f.app.doc().dirty());
    f.app.execute("undo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), ORIGINAL);
    assert!(f.app.doc().dirty());
    f.app.execute("redo", Value::Null);
    assert_eq!(f.app.doc().text.to_string(), changed);
    assert!(!f.app.doc().dirty());
    assert_eq!(
        f.app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
        ids.into_iter()
            .filter(|id| *id != explicitly_closed)
            .collect::<Vec<_>>()
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), changed.as_bytes());
}
