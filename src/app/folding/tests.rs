//! Native App/window/worker integrity regressions for the integrated candidate.
use super::*;
use crate::folding_worker::{Gate, Gates};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, thread};

const SOURCE: &str = "prefix\r\nhead猫\r\n body🙂\r\n tail\r\nafter\r\n";
struct Fixture {
    _root: tempfile::TempDir,
    path: PathBuf,
    app: App,
}
impl Fixture {
    fn new(text: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("fold.txt");
        fs::write(&path, text).unwrap();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.extension_node = "vscli-fixture-missing-node".into();
        app.configure_language_services(None, true).unwrap();
        app.sidebar = false;
        app.open(&path).unwrap();
        until(&mut app, "open", |a| {
            a.active_document()
                .is_some_and(|d| d.path.as_ref() == Some(&path))
        });
        app.doc_mut().move_to(0, false);
        Self {
            _root: directory,
            path,
            app,
        }
    }
    fn folds(&mut self) {
        self.app.execute("editor.foldAll", Value::Null);
        until(&mut self.app, "Fold All", |a| {
            a.active_tab_membership()
                .is_some_and(|m| a.doc().current_folding_rows(m.group.value()).is_some())
                && !a.folding.lane.resolving()
                && !a.folding.lane.occupied()
        });
        assert_eq!(
            self.app
                .doc()
                .folding_desired_count(self.app.active_tab_membership().unwrap().group.value()),
            Some(1)
        );
    }
}
fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        thread::yield_now();
    }
}
fn draw(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(104, 30)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..30 {
        for x in 0..104 {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}
fn source(app: &App) -> String {
    app.doc().text.to_string()
}
fn identity(app: &App) -> (u64, u64, u64, u64, bool) {
    (
        app.doc().id,
        app.doc().revision,
        app.doc().text_epoch(),
        app.doc().save_generation(),
        app.doc().dirty(),
    )
}
fn click(app: &mut App, x: u16, y: u16) {
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
}
struct Held(Gates);
impl std::ops::Deref for Held {
    type Target = Gates;
    fn deref(&self) -> &Gates {
        &self.0
    }
}
impl Drop for Held {
    fn drop(&mut self) {
        self.before.release();
        self.after.release();
    }
}
fn gates(app: &mut App) -> Held {
    let gates = Gates {
        before: Gate::held(),
        after: Gate::held(),
    };
    app.folding.lane.hold_worker(gates.clone());
    Held(gates)
}
fn dispatch(app: &mut App) {
    app.execute("editor.foldAll", Value::Null);
    app.poll_folding();
    assert!(app.folding.lane.occupied());
}

#[test]
fn painted_rows_caret_mouse_and_vertical_movement_use_one_unicode_crlf_projection() {
    let mut f = Fixture::new(SOURCE);
    let before = identity(&f.app);
    f.folds();
    let screen = draw(&mut f.app);
    assert!(
        screen.contains("head猫") && screen.contains("after"),
        "{screen}"
    );
    assert!(
        !screen.contains("body🙂") && !screen.contains("tail"),
        "{screen}"
    );
    assert!(f.app.folding.sealed);
    let pane = f.app.folding_window().unwrap().clone();
    let visible: Vec<_> = (0..pane.window.row_count())
        .map(|i| pane.window.row(i).unwrap().anchor.logical_line)
        .collect();
    assert_eq!(visible, vec![0, 1, 4, 5]);
    click(&mut f.app, pane.text.x, pane.text.y + 2);
    assert_eq!(f.app.doc().row(), 4);
    assert_eq!(f.app.doc().cursor, f.app.doc().text.line_to_char(4));
    draw(&mut f.app);
    let caret = f.app.folded_caret().unwrap();
    assert_eq!(caret, (pane.text.x, pane.text.y + 2));
    f.app.execute("cursorUp", Value::Null);
    assert_eq!(f.app.doc().row(), 1);
    f.app.execute("cursorDown", Value::Null);
    assert_eq!(f.app.doc().row(), 4);
    assert_eq!(source(&f.app), SOURCE);
    assert_eq!(identity(&f.app), before);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn gutter_unfold_does_not_move_caret_or_primary_secondary_selection() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    f.app.doc_mut().set_selections(vec![
        Selection {
            cursor: 0,
            anchor: Some(2),
            desired_column: None,
        },
        Selection::caret(SOURCE.chars().count() - 2),
    ]);
    f.app.observe_folding();
    draw(&mut f.app);
    // Selection changes need a fresh safe prepared map; no hidden-range authority is reused.
    until(&mut f.app, "safe selection refresh", |a| {
        a.doc()
            .current_folding_rows(a.active_tab_membership().unwrap().group.value())
            .is_some()
    });
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap().clone();
    let selections = f.app.doc().selections();
    let before = identity(&f.app);
    click(&mut f.app, pane.text.x - 2, pane.text.y + 1);
    until(&mut f.app, "header unfold", |a| {
        a.doc()
            .folding_desired_count(a.active_tab_membership().unwrap().group.value())
            == Some(0)
            && !a.folding.lane.occupied()
    });
    assert_eq!(f.app.doc().selections(), selections);
    assert_eq!(identity(&f.app), before);
    assert_eq!(source(&f.app), SOURCE);
    assert!(draw(&mut f.app).contains("body🙂"));
}

#[test]
fn source_edit_before_second_batched_mouse_rejects_old_rows_and_preserves_redo() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap().clone();
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
    let cursor = f.app.doc().cursor;
    click(&mut f.app, pane.text.x, pane.text.y + 2);
    assert_eq!(f.app.doc().cursor, cursor);
    assert!(
        f.app.message.contains("viewport changed"),
        "{}",
        f.app.message
    );
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn held_edit_undo_reply_never_revives_original_map_and_reservation_lives_until_join() {
    let mut f = Fixture::new(SOURCE);
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    assert!(f.app.folding.reserved);
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    f.app.doc_mut().undo();
    let proof = identity(&f.app);
    f.app.poll_folding();
    assert!(f.app.folding.lane.occupied() && f.app.folding.reserved);
    assert!(
        f.app
            .doc()
            .current_folding_rows(f.app.active_tab_membership().unwrap().group.value())
            .is_none()
    );
    hold.before.release();
    hold.after.wait_reached();
    f.app.poll_folding();
    assert!(f.app.folding.lane.occupied() && f.app.folding.reserved);
    hold.after.release();
    until(&mut f.app, "retired actual join", |a| {
        !a.folding.lane.occupied()
    });
    assert!(!f.app.folding.reserved);
    assert_eq!(identity(&f.app), proof);
    assert!(
        f.app
            .doc()
            .current_folding_rows(f.app.active_tab_membership().unwrap().group.value())
            .is_none()
    );
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn unfolded_clear_after_held_command_retires_reply_without_cancelling_text_or_history() {
    let mut f = Fixture::new(SOURCE);
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    f.app.doc_mut().undo();
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    f.app.execute("editor.unfoldAll", Value::Null);
    let before = identity(&f.app);
    assert!(f.app.folding.lane.occupied());
    hold.before.release();
    hold.after.release();
    until(&mut f.app, "clear retires actual", |a| {
        !a.folding.lane.occupied()
    });
    assert_eq!(identity(&f.app), before);
    assert_eq!(
        f.app
            .doc()
            .folding_desired_count(f.app.active_tab_membership().unwrap().group.value()),
        Some(0)
    );
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
}

#[test]
fn shared_split_copies_intent_with_fresh_authority_and_clear_changes_only_destination() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    let document = f.app.doc().id;
    let original = f.app.active_tab_membership().unwrap();
    f.app
        .execute("workbench.action.splitEditorRight", Value::Null);
    let destination = f.app.active_tab_membership().unwrap();
    assert_ne!(original.group, destination.group);
    assert_eq!(f.app.documents.len(), 1);
    assert_eq!(f.app.doc().id, document);
    assert_ne!(
        f.app
            .doc()
            .folding_view_stamp(original.group.value())
            .unwrap()
            .0,
        f.app
            .doc()
            .folding_view_stamp(destination.group.value())
            .unwrap()
            .0
    );
    until(&mut f.app, "split refreshed", |a| {
        a.doc()
            .current_folding_rows(destination.group.value())
            .is_some()
    });
    f.app.execute("editor.unfoldAll", Value::Null);
    assert_eq!(
        f.app.doc().folding_desired_count(destination.group.value()),
        Some(0)
    );
    assert_eq!(
        f.app.doc().folding_desired_count(original.group.value()),
        Some(1)
    );
    f.app.focus_tab(original).unwrap();
    until(&mut f.app, "source map", |a| {
        a.doc()
            .current_folding_rows(original.group.value())
            .is_some()
    });
    assert!(draw(&mut f.app).contains("head猫"));
    assert_eq!(identity(&f.app).0, document);
    assert_eq!(source(&f.app), SOURCE);
}

#[test]
fn protected_range_is_never_hidden_or_relocated_by_fold_all() {
    let mut f = Fixture::new(SOURCE);
    let start = f.app.doc().text.line_to_char(1);
    let end = f.app.doc().text.line_to_char(4);
    f.app.doc_mut().set_selections(vec![Selection {
        cursor: start,
        anchor: Some(end),
        desired_column: None,
    }]);
    let selections = f.app.doc().selections();
    let before = identity(&f.app);
    f.app.execute("editor.foldAll", Value::Null);
    until(&mut f.app, "protected fold", |a| {
        !a.folding.lane.resolving() && !a.folding.lane.occupied()
    });
    assert_eq!(
        f.app
            .doc()
            .folding_desired_count(f.app.active_tab_membership().unwrap().group.value()),
        Some(0)
    );
    assert_eq!(f.app.doc().selections(), selections);
    assert_eq!(identity(&f.app), before);
    assert!(draw(&mut f.app).contains("body🙂"));
}

#[test]
fn fold_and_unfold_target_selected_header_without_editing_or_new_text_undo() {
    let mut f = Fixture::new(SOURCE);
    let header = f.app.doc().text.line_to_char(1);
    f.app.doc_mut().move_to(header, false);
    let before = identity(&f.app);
    f.app.execute("editor.fold", Value::Null);
    until(&mut f.app, "Fold", |a| {
        a.doc()
            .folding_desired_count(a.active_tab_membership().unwrap().group.value())
            == Some(1)
            && !a.folding.lane.occupied()
    });
    f.app.execute("editor.unfold", Value::Null);
    until(&mut f.app, "Unfold", |a| {
        a.doc()
            .folding_desired_count(a.active_tab_membership().unwrap().group.value())
            == Some(0)
            && !a.folding.lane.occupied()
    });
    assert_eq!(identity(&f.app), before);
    assert_eq!(source(&f.app), SOURCE);
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
}

#[test]
fn oversize_fold_refusal_stays_native_editable_and_scan_free_clear_is_available() {
    let text = format!("{}\r\n", "x".repeat(crate::folding::MAX_BYTES + 1));
    let mut f = Fixture::new(&text);
    f.app.execute("editor.foldAll", Value::Null);
    f.app.poll_folding();
    assert!(!f.app.folding.lane.occupied());
    assert!(f.app.message.contains("exceeds"), "{}", f.app.message);
    f.app.doc_mut().insert("猫", false);
    let edited = source(&f.app);
    f.app.execute("editor.unfoldAll", Value::Null);
    assert_eq!(source(&f.app), edited);
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), text);
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
    assert_eq!(fs::read(&f.path).unwrap(), text.as_bytes());
}

#[test]
fn unsupported_command_arguments_preserve_exact_model_selection_redo_and_disk() {
    let mut f = Fixture::new(SOURCE);
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    f.app.doc_mut().undo();
    let before = identity(&f.app);
    let selected = f.app.doc().selections();
    f.app
        .execute("editor.fold", serde_json::json!({"levels":2}));
    assert!(f.app.message.contains("no-argument"));
    assert!(!f.app.folding.lane.occupied());
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selected);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
}

#[test]
fn shutdown_waits_for_positive_worker_exit_and_drops_latest_intent_without_publication() {
    let mut f = Fixture::new(SOURCE);
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    f.app.execute("editor.fold", Value::Null);
    assert!(f.app.folding.lane.resolving());
    assert!(f.app.settle_folding().is_err());
    assert!(f.app.folding.lane.occupied() && f.app.folding.reserved);
    hold.before.release();
    hold.after.release();
    let deadline = Instant::now() + Duration::from_secs(8);
    while f.app.folding.lane.occupied() {
        f.app.folding.lane.poll_retired();
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    f.app.settle_folding().unwrap();
    assert!(!f.app.folding.reserved);
    assert_eq!(source(&f.app), SOURCE);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
    assert!(
        f.app
            .doc()
            .current_folding_rows(f.app.active_tab_membership().unwrap().group.value())
            .is_none()
    );
}

#[test]
fn literal_input_after_folding_preserves_bytes_and_single_original_text_undo_redo() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    draw(&mut f.app);
    let before = identity(&f.app);
    f.app.event(Event::Paste("é🙂".into()));
    let edited = format!("é🙂{SOURCE}");
    assert_eq!(source(&f.app), edited);
    assert_eq!(identity(&f.app).0, before.0);
    assert!(
        f.app
            .doc()
            .current_folding_rows(f.app.active_tab_membership().unwrap().group.value())
            .is_none()
    );
    draw(&mut f.app);
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::Char('z'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(source(&f.app), SOURCE);
    f.app.execute("redo", Value::Null);
    assert_eq!(source(&f.app), edited);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn observed_enabled_policy_aba_cannot_reauthorize_held_origin() {
    let mut f = Fixture::new(SOURCE);
    let user = f._root.path().join("settings.json");
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    fs::write(&user, r#"{"editor.folding":false}"#).unwrap();
    f.app.settings = crate::settings::Settings::load(std::slice::from_ref(&user)).unwrap();
    f.app.observe_folding();
    assert!(!f.app.folding_enabled());
    fs::write(&user, r#"{"editor.folding":true}"#).unwrap();
    f.app.settings = crate::settings::Settings::load(std::slice::from_ref(&user)).unwrap();
    f.app.observe_folding();
    assert!(f.app.folding_enabled());
    hold.before.release();
    hold.after.release();
    until(&mut f.app, "policy retired actual", |a| {
        !a.folding.lane.occupied()
    });
    assert!(
        f.app
            .doc()
            .current_folding_rows(f.app.active_tab_membership().unwrap().group.value())
            .is_none()
    );
    assert_eq!(source(&f.app), SOURCE);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn retained_group_proof_rejects_held_focus_aba_without_reviving_fold_authority() {
    let mut f = Fixture::new(SOURCE);
    let source_member = f.app.active_tab_membership().unwrap();
    f.app
        .execute("workbench.action.splitEditorRight", Value::Null);
    let destination = f.app.active_tab_membership().unwrap();
    f.app.focus_tab(source_member).unwrap();
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    f.app.focus_tab(destination).unwrap();
    f.app.focus_tab(source_member).unwrap();
    f.app.poll_folding();
    assert!(f.app.folding.lane.occupied() && f.app.folding.reserved);
    hold.before.release();
    hold.after.release();
    until(&mut f.app, "group retired actual", |a| {
        !a.folding.lane.occupied()
    });
    assert_eq!(f.app.documents.len(), 1);
    assert_eq!(f.app.doc().id, source_member.document);
    assert!(
        f.app
            .doc()
            .current_folding_rows(source_member.group.value())
            .is_none()
    );
    assert_eq!(source(&f.app), SOURCE);
}

#[test]
fn five_previously_folded_documents_retire_offscreen_maps_but_keep_bounded_intent() {
    let mut f = Fixture::new(SOURCE);
    let first = f.app.doc().id;
    let mut ids = vec![first];
    f.folds();
    draw(&mut f.app);
    for index in 1..5 {
        let path = f._root.path().join(format!("fold{index}.txt"));
        fs::write(&path, SOURCE).unwrap();
        f.app.open(&path).unwrap();
        until(&mut f.app, "switch", |a| {
            a.doc().path.as_ref() == Some(&path)
        });
        ids.push(f.app.doc().id);
        f.folds();
        assert!(f.app.folding_resources().1 <= 4);
        assert!(f.app.folding_source_bytes() <= SOURCE_RETENTION);
        draw(&mut f.app);
    }
    assert_eq!(f.app.documents.len(), 5);
    let group = f.app.active_tab_membership().unwrap().group.value();
    for id in &ids[..4] {
        let doc = f.app.documents.iter().find(|d| d.id == *id).unwrap();
        assert_eq!(doc.folding_desired_count(group), Some(1));
        assert!(doc.current_folding_rows(group).is_none());
    }
    let old = f.app.folding_window().unwrap().window.clone();
    let old_source = old.projection().text().len_bytes();
    f.app.event(Event::Paste("é".into()));
    assert!(!f.app.folding.sealed);
    assert!(f.app.folding_source_bytes() >= old_source);
    assert!(f.app.folding_resources().1 <= 4);
    assert_eq!(old.projection().text().to_string(), SOURCE);
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
    draw(&mut f.app);
    assert!(f.app.folding_source_bytes() <= SOURCE_RETENTION);
    for id in ids {
        assert_eq!(
            f.app
                .documents
                .iter()
                .find(|d| d.id == id)
                .unwrap()
                .text
                .to_string(),
            SOURCE
        );
    }
}

#[test]
fn repeated_commands_keep_one_actual_worker_and_one_latest_two_phase_intent() {
    let mut f = Fixture::new(SOURCE);
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    let token = f.app.folding.lane.last_worker_token();
    let before = identity(&f.app);
    for _ in 0..32 {
        f.app.execute("editor.foldAll", Value::Null);
        f.app.poll_folding();
        assert_eq!(f.app.folding.lane.last_worker_token(), token);
        assert!(f.app.folding.lane.occupied() && f.app.folding.reserved);
    }
    hold.before.release();
    hold.after.release();
    until(&mut f.app, "latest two-phase action", |a| {
        a.doc()
            .current_folding_rows(a.active_tab_membership().unwrap().group.value())
            .is_some()
            && !a.folding.lane.occupied()
            && !a.folding.lane.resolving()
    });
    assert_eq!(f.app.folding.lane.last_worker_token(), token + 2);
    assert_eq!(identity(&f.app), before);
    assert_eq!(source(&f.app), SOURCE);
    assert!(!f.app.folding.reserved);
}

#[test]
fn refused_pane_window_budget_expands_only_display_and_preserves_original_text_redo() {
    let mut f = Fixture::new(SOURCE);
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    f.app.doc_mut().undo();
    f.folds();
    let before = identity(&f.app);
    let selected = f.app.doc().selections();
    let member = f.app.active_tab_membership().unwrap();
    f.app.begin_folding_frame();
    f.app
        .prepare_folding_window(
            member,
            Rect::new(0, 0, 4096, 4096),
            Rect::new(5, 0, 4091, 4096),
        )
        .unwrap();
    assert!(
        f.app.message.contains("Folding expanded"),
        "{}",
        f.app.message
    );
    assert!(f.app.folding_window().is_none());
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selected);
    assert_eq!(source(&f.app), SOURCE);
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn folding_display_changes_preserve_an_actual_authorized_save_receipt_and_text_undo() {
    use crate::save_worker::{GatePoint, Worker};
    use std::sync::mpsc::{SyncSender, TryRecvError};
    struct Release(SyncSender<()>);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = self.0.try_send(());
        }
    }
    let mut f = Fixture::new(SOURCE);
    f.app.doc_mut().insert("é🙂", false);
    let captured = source(&f.app);
    let epoch = f.app.doc().text_epoch();
    let generation = f.app.doc().save_generation();
    f.folds();
    let (worker, entered, release) = Worker::fixture_gated(vec![GatePoint::BeforeCommit]);
    let release = Release(release);
    f.app.replace_save_worker_fixture(worker);
    f.app.execute("workbench.action.files.save", Value::Null);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        f.app.poll();
        match entered.try_recv() {
            Ok(point) => {
                assert_eq!(point, GatePoint::BeforeCommit);
                break;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => panic!("{}", f.app.message),
        }
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(f.app.native_save_worker_busy());
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
    f.app.execute("editor.unfoldAll", Value::Null);
    f.folds();
    draw(&mut f.app);
    f.app.execute("editor.unfoldAll", Value::Null);
    assert!(f.app.native_save_worker_busy());
    assert_eq!(source(&f.app), captured);
    assert_eq!(f.app.doc().text_epoch(), epoch);
    release.0.try_send(()).unwrap();
    until(&mut f.app, "real save receipt", |a| !a.saves_pending());
    assert_eq!(fs::read(&f.path).unwrap(), captured.as_bytes());
    assert_eq!(f.app.doc().save_generation(), generation + 1);
    assert!(!f.app.doc().dirty());
    assert_eq!(f.app.doc().text_epoch(), epoch);
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
    assert!(f.app.doc().dirty());
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), captured);
    assert!(!f.app.doc().dirty());
}

#[test]
fn observed_tab_width_aba_and_logical_reveal_retire_old_source_hits() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap().clone();
    let before = identity(&f.app);
    f.app.doc_mut().set_indentation(2, true);
    f.app.observe_folding();
    f.app.doc_mut().set_indentation(4, true);
    f.app.observe_folding();
    let selections = f.app.doc().selections();
    click(&mut f.app, pane.text.x, pane.text.y + 2);
    assert_eq!(f.app.doc().selections(), selections);
    assert_eq!(identity(&f.app), before);
    f.folds();
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap().clone();
    let body = f.app.doc().text.line_to_char(2) + 1;
    f.app.doc_mut().move_to(body, false);
    f.app.observe_folding();
    click(&mut f.app, pane.text.x, pane.text.y + 2);
    assert_eq!(f.app.doc().cursor, body);
    assert!(f.app.message.contains("viewport changed"));
    assert!(draw(&mut f.app).contains("body🙂"));
    assert_eq!(source(&f.app), SOURCE);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn automatic_fold_refresh_preserves_an_actual_save_receipt_message_and_history() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    f.app.event(Event::Paste("é🙂".into()));
    let captured = source(&f.app);
    let epoch = f.app.doc().text_epoch();
    let generation = f.app.doc().save_generation();
    f.app.execute("workbench.action.files.save", Value::Null);
    let held = gates(&mut f.app);
    until(&mut f.app, "automatic refresh dispatch", |app| {
        app.folding.lane.occupied()
    });
    held.before.wait_reached();
    until(&mut f.app, "actual save while refresh is held", |app| {
        !app.saves_pending()
    });
    assert_eq!(fs::read(&f.path).unwrap(), captured.as_bytes());
    assert_eq!(f.app.doc().save_generation(), generation + 1);
    assert!(!f.app.doc().dirty());
    let saved_message = f.app.message.clone();
    assert!(saved_message.contains("Saved"), "{saved_message}");
    held.before.release();
    held.after.release();
    until(&mut f.app, "automatic refresh after real save", |app| {
        !app.folding.lane.occupied()
            && !app.folding.lane.resolving()
            && app.active_tab_membership().is_some_and(|member| {
                app.doc()
                    .current_folding_rows(member.group.value())
                    .is_some()
            })
    });
    assert_eq!(f.app.message, saved_message);
    assert_eq!(source(&f.app), captured);
    assert_eq!(f.app.doc().text_epoch(), epoch);
    assert_eq!(f.app.doc().save_generation(), generation + 1);
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
    assert!(f.app.doc().dirty());
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), captured);
    assert!(!f.app.doc().dirty());
    assert_eq!(fs::read(&f.path).unwrap(), captured.as_bytes());
}

#[test]
fn painted_grapheme_tab_hits_and_secondary_caret_follow_exact_folded_source_cells() {
    let text = "prefix\r\nhead猫e\u{301}🙂\tend\r\n hidden\r\nafter\r\n";
    let mut f = Fixture::new(text);
    f.folds();
    draw(&mut f.app);
    let header = f.app.doc().text.line_to_char(1);
    let after = f.app.doc().text.line_to_char(3);
    for (cell, scalar) in [(5, 4), (6, 5), (8, 7), (10, 8)] {
        draw(&mut f.app);
        let pane = f.app.folding_window().unwrap().clone();
        click(&mut f.app, pane.text.x + cell, pane.text.y + 1);
        assert_eq!(f.app.doc().cursor, header + scalar);
    }
    f.app
        .doc_mut()
        .set_selections(vec![Selection::caret(header + 7), Selection::caret(after)]);
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap();
    let primary = pane.window.locate(header + 7, Affinity::Before).unwrap();
    let secondary = pane.window.locate(after, Affinity::Before).unwrap();
    assert_eq!((primary.row, primary.column), (1, 7));
    assert_eq!((secondary.row, secondary.column), (2, 0));
    assert_eq!(
        f.app.folded_caret(),
        Some((pane.text.x + 7, pane.text.y + 1))
    );
    assert_eq!(source(&f.app), text);
    assert_eq!(fs::read(&f.path).unwrap(), text.as_bytes());
}

#[test]
fn unchanged_oversize_refresh_error_is_inert_until_new_epoch_then_recovers_current_source() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    let token = f.app.folding.lane.last_worker_token();
    let end = f.app.doc().len();
    f.app.doc_mut().move_to(end, false);
    f.app
        .doc_mut()
        .insert(&"x".repeat(crate::folding::MAX_BYTES), false);
    let edited = source(&f.app);
    let proof = identity(&f.app);
    f.app.poll_folding();
    f.app.poll_folding();
    assert!(f.app.message.contains("exceeds"), "{}", f.app.message);
    for _ in 0..64 {
        f.app.poll_folding();
        assert!(!f.app.folding.lane.occupied());
        assert!(!f.app.folding.lane.resolving());
        assert_eq!(f.app.folding.lane.last_worker_token(), token);
    }
    assert_eq!(source(&f.app), edited);
    assert_eq!(identity(&f.app), proof);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
    until(&mut f.app, "new epoch recovery", |a| {
        a.doc()
            .current_folding_rows(a.active_tab_membership().unwrap().group.value())
            .is_some()
            && !a.folding.lane.occupied()
    });
    assert!(f.app.folding.lane.last_worker_token() > token);
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
}

#[test]
fn shared_long_header_prefixes_and_failed_windows_share_one_frame_scan_allowance() {
    let text = format!(
        "prefix\r\nhead{}\r\n hidden\r\nafter\r\n",
        "x".repeat(48 * 1024)
    );
    let mut f = Fixture::new(&text);
    let header = f.app.doc().text.line_to_char(1);
    f.app.doc_mut().move_to(header, false);
    f.folds();
    for _ in 0..3 {
        f.app.execute("workbench.action.splitEditor", Value::Null);
    }
    until(&mut f.app, "four shared prepared maps", |a| {
        a.editor_groups.groups().len() == 4
            && a.editor_groups.groups().iter().all(|group| {
                group.active().is_some_and(|tab| {
                    a.documents
                        .iter()
                        .find(|doc| doc.id == tab.document())
                        .is_some_and(|doc| doc.current_folding_rows(group.id().value()).is_some())
                })
            })
            && !a.folding.lane.occupied()
    });
    let before = identity(&f.app);
    let selections = f.app.doc().selections();
    draw(&mut f.app);
    assert!(f.app.folding.frame_scan_bytes <= crate::display_window::MAX_PREPARED_BYTES);
    assert!(
        f.app.folding.windows.iter().flatten().count() < 4,
        "Each header and viewport needs48KiB; four cannot fit one256KiB allowance"
    );
    assert!(f.app.folding.windows.iter().flatten().count() > 0);
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selections);
    assert_eq!(source(&f.app), text);
    assert_eq!(fs::read(&f.path).unwrap(), text.as_bytes());
}

#[test]
fn split_refuses_before_view_copy_when_actual_pending_and_old_frame_exhaust_metadata() {
    let text = format!(
        "prefix\r\n{}\r\n{}\r\n{}\r\n{}",
        "P".repeat(50_000),
        "Q".repeat(50_000),
        "R".repeat(50_000),
        "head\r\n body\r\n".repeat(5_000)
    );
    let mut f = Fixture::new(&text);
    f.app.doc_mut().insert("é", false);
    f.app.doc_mut().undo();
    f.app.execute("editor.foldAll", Value::Null);
    until(&mut f.app, "large actual prepared map", |a| {
        a.doc()
            .current_folding_rows(a.active_tab_membership().unwrap().group.value())
            .is_some()
            && !a.folding.lane.occupied()
            && !a.folding.lane.resolving()
    });
    assert_eq!(
        f.app
            .doc()
            .folding_desired_count(f.app.active_tab_membership().unwrap().group.value()),
        Some(5_000)
    );
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap().clone();
    assert!(pane.window.allocated_payload() > 3_500_000);
    let hold = gates(&mut f.app);
    dispatch(&mut f.app);
    hold.before.wait_reached();
    let before = identity(&f.app);
    let selected = f.app.doc().selections();
    let groups = f.app.editor_groups.proof();
    let resources = f.app.folding_resources();
    let retained = f.app.folding_source_bytes();
    let geometry = f
        .app
        .editor_layout
        .project(
            Rect::default(),
            f.app
                .editor_groups
                .active_membership()
                .map(|member| member.group),
        )
        .unwrap();
    let error = f
        .app
        .split_editor_layout(crate::editor_layout::Direction::Right)
        .unwrap_err();
    assert!(error.to_string().contains("metadata"), "{error:#}");
    assert!(f.app.editor_groups.proof_current(&groups));
    assert!(f.app.editor_layout.geometry_current(&geometry));
    assert_eq!(f.app.editor_groups.groups().len(), 1);
    assert_eq!(f.app.folding_resources(), resources);
    assert_eq!(f.app.folding_source_bytes(), retained);
    assert!(
        f.app
            .folding_window()
            .unwrap()
            .window
            .same_window(&pane.window)
    );
    assert!(f.app.folding.lane.occupied() && f.app.folding.reserved);
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selected);
    assert_eq!(source(&f.app), text);
    assert_eq!(fs::read(&f.path).unwrap(), text.as_bytes());
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), format!("é{text}"));
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), text);
    f.app.execute("editor.unfoldAll", Value::Null);
    hold.before.release();
    hold.after.release();
    until(&mut f.app, "retired actual join after refused split", |a| {
        !a.folding.lane.occupied() && !a.folding.lane.resolving()
    });
    assert!(!f.app.folding.reserved);
    assert_eq!(source(&f.app), text);
    assert_eq!(f.app.doc().save_generation(), before.3);
}

fn add_overflow_retained_models(app: &mut App) {
    for _ in 0..128 {
        let mut doc = Document::from_text("retained");
        doc.insert("é", false);
        app.hidden_documents.push(doc);
    }
    assert_eq!(app.documents.len() + app.hidden_documents.len(), 129);
}
#[test]
fn oversized_inventory_retires_invalidated_interest_once_without_queue_or_redraw_spin() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    let before = identity(&f.app);
    let selected = f.app.doc().selections();
    let token = f.app.folding.lane.last_worker_token();
    add_overflow_retained_models(&mut f.app);
    let retained: Vec<_> = f
        .app
        .hidden_documents
        .iter()
        .map(|doc| {
            (
                doc.id,
                doc.text_epoch(),
                doc.revision,
                doc.save_generation(),
                doc.dirty(),
                doc.cursor,
            )
        })
        .collect();
    assert!(f.app.poll_folding());
    assert!(f.app.message.contains("inventory"));
    assert!(!f.app.folding_enabled());
    for _ in 0..64 {
        assert!(!f.app.poll_folding());
        assert!(!f.app.folding.lane.resolving());
        assert!(!f.app.folding.lane.occupied());
        assert!(!f.app.folding.reserved);
        assert_eq!(f.app.folding.lane.last_worker_token(), token);
    }
    assert!(!f.app.move_folded(1, false, false));
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selected);
    assert_eq!(source(&f.app), edited);
    f.app.execute("editor.unfoldAll", Value::Null);
    assert_eq!(
        f.app
            .doc()
            .folding_desired_count(f.app.active_tab_membership().unwrap().group.value()),
        Some(0)
    );
    f.app.doc_mut().undo();
    assert_eq!(source(&f.app), SOURCE);
    f.app.doc_mut().redo();
    assert_eq!(source(&f.app), edited);
    assert_eq!(
        f.app
            .hidden_documents
            .iter()
            .map(|doc| (
                doc.id,
                doc.text_epoch(),
                doc.revision,
                doc.save_generation(),
                doc.dirty(),
                doc.cursor
            ))
            .collect::<Vec<_>>(),
        retained
    );
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}
#[test]
fn oversized_inventory_never_seals_ready_rows_and_readmission_refreshes_private_authority() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    draw(&mut f.app);
    let pane = f.app.folding_window().unwrap().clone();
    let member = f.app.active_tab_membership().unwrap();
    let lifetime = f
        .app
        .doc()
        .folding_view_stamp(member.group.value())
        .unwrap();
    let before = identity(&f.app);
    let selected = f.app.doc().selections();
    add_overflow_retained_models(&mut f.app);
    assert!(f.app.observe_folding());
    f.app.begin_folding_frame();
    f.app
        .prepare_folding_window(
            member,
            Rect::new(
                pane.gutter.x,
                pane.text.y,
                pane.gutter.width + pane.text.width,
                pane.text.height,
            ),
            pane.text,
        )
        .unwrap();
    f.app.seal_folding_frame();
    assert!(f.app.folding_window().is_none());
    assert!(!f.app.folding.sealed);
    assert!(!f.app.move_folded(1, false, false));
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selected);
    f.app.hidden_documents.pop();
    assert!(f.app.observe_folding());
    assert!(
        f.app
            .doc()
            .current_folding_rows(member.group.value())
            .is_none(),
        "Readmission must not revive the pre-refusal Ready projection"
    );
    assert_ne!(
        f.app
            .doc()
            .folding_view_stamp(member.group.value())
            .unwrap()
            .1,
        lifetime.1
    );
    assert_eq!(
        f.app.doc().folding_desired_count(member.group.value()),
        Some(1)
    );
    until(&mut f.app, "fresh readmitted projection", |a| {
        a.doc().current_folding_rows(member.group.value()).is_some() && !a.folding.lane.occupied()
    });
    draw(&mut f.app);
    assert!(
        !f.app
            .folding_window()
            .unwrap()
            .window
            .same_projection(pane.window.projection())
    );
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selected);
    assert_eq!(source(&f.app), SOURCE);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}

#[test]
fn folding_disabled_split_with_129_retained_models_preserves_native_editor_and_history() {
    let mut f = Fixture::new(SOURCE);
    f.folds();
    let original = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().insert("é", false);
    let edited = source(&f.app);
    let before = identity(&f.app);
    let selection = f.app.doc().selections();
    add_overflow_retained_models(&mut f.app);
    let hidden: Vec<_> = f
        .app
        .hidden_documents
        .iter()
        .map(|doc| {
            (
                doc.id,
                doc.text.to_string(),
                doc.text_epoch(),
                doc.revision,
                doc.save_generation(),
                doc.dirty(),
            )
        })
        .collect();
    let user = f._root.path().join("settings.json");
    fs::write(&user, r#"{"editor.folding":false}"#).unwrap();
    f.app.settings = crate::settings::Settings::load(std::slice::from_ref(&user)).unwrap();
    assert!(!f.app.copy_split_folding_intent(original));
    f.app
        .execute("workbench.action.splitEditorRight", Value::Null);
    let target = f.app.active_tab_membership().unwrap();
    assert_ne!(target.group, original.group);
    assert_eq!(target.document, original.document);
    assert_eq!(f.app.editor_groups.groups().len(), 2);
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selection);
    assert_eq!(source(&f.app), edited);
    assert_eq!(
        f.app.doc().folding_desired_count(target.group.value()),
        Some(0)
    );
    assert!(
        f.app
            .doc()
            .current_folding_rows(target.group.value())
            .is_none()
    );
    f.app.poll_folding(); // settle the one inventory refusal observation
    let next = f.app.folding.next;
    for _ in 0..32 {
        assert!(!f.app.poll_folding());
        assert!(!f.app.folding.lane.occupied());
        assert!(!f.app.folding.lane.resolving());
        assert!(!f.app.folding.reserved);
    }
    assert_eq!(f.app.folding.next, next);
    assert_eq!(
        f.app
            .hidden_documents
            .iter()
            .map(|doc| {
                (
                    doc.id,
                    doc.text.to_string(),
                    doc.text_epoch(),
                    doc.revision,
                    doc.save_generation(),
                    doc.dirty(),
                )
            })
            .collect::<Vec<_>>(),
        hidden
    );
    f.app.execute("undo", Value::Null);
    assert_eq!(source(&f.app), SOURCE);
    f.app.execute("redo", Value::Null);
    assert_eq!(source(&f.app), edited);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
    assert_eq!(identity(&f.app).0, before.0);
    assert_eq!(identity(&f.app).3, before.3);
}

#[test]
fn ordinary_no_intent_split_remains_usable_above_folding_inventory_cap() {
    let mut f = Fixture::new(SOURCE);
    let original = f.app.active_tab_membership().unwrap();
    assert!(f.app.settings.folding(f.app.language()));
    add_overflow_retained_models(&mut f.app);
    let before = identity(&f.app);
    let selection = f.app.doc().selections();
    f.app
        .execute("workbench.action.splitEditorDown", Value::Null);
    let target = f.app.active_tab_membership().unwrap();
    assert_ne!(target.group, original.group);
    assert_eq!(target.document, original.document);
    assert_eq!(f.app.editor_groups.groups().len(), 2);
    assert_eq!(identity(&f.app), before);
    assert_eq!(f.app.doc().selections(), selection);
    for member in [original, target] {
        assert_eq!(
            f.app.doc().folding_desired_count(member.group.value()),
            Some(0)
        );
        assert!(
            f.app
                .doc()
                .current_folding_rows(member.group.value())
                .is_none()
        );
    }
    f.app.poll_folding();
    let next = f.app.folding.next;
    for _ in 0..32 {
        assert!(!f.app.poll_folding());
        assert!(!f.app.folding.lane.occupied());
        assert!(!f.app.folding.lane.resolving());
        assert!(!f.app.folding.reserved);
    }
    assert_eq!(f.app.folding.next, next);
    assert_eq!(source(&f.app), SOURCE);
    assert_eq!(fs::read(&f.path).unwrap(), SOURCE.as_bytes());
}
