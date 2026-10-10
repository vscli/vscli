use super::*;
use crate::{document::Selection, lsp::formatting_tests, settings::Settings};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

const ORIGINAL: &str = "猫🙂 A\r\nlast\r\n";
const B: &str = "B λ unchanged\r\n";

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    a: PathBuf,
    b: PathBuf,
    a_id: u64,
    b_id: u64,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let a = root.join("a.cpp");
        let b = root.join("b.cpp");
        std::fs::write(&a, ORIGINAL).unwrap();
        std::fs::write(&b, B).unwrap();
        let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
        app.settings = Settings::from_values(
            json!({"editor.formatOnSave":true,"files.autoSave":"off",
                "breadcrumbs.enabled":false,"vscli.languageServer.enabled":false})
            .as_object()
            .unwrap()
            .clone(),
            "save formatting fixture",
        )
        .unwrap();
        app.open(&b).unwrap();
        until(&mut app, "open B", |app| {
            app.doc().path.as_ref() == Some(&b)
        });
        let b_id = app.doc().id;
        app.open(&a).unwrap();
        until(&mut app, "open A", |app| {
            app.doc().path.as_ref() == Some(&a)
        });
        let a_id = app.doc().id;
        app.lsp = Some(formatting_tests::start(&root, app.doc()));
        Self {
            _directory: directory,
            app,
            a,
            b,
            a_id,
            b_id,
        }
    }
    fn model(&self, id: u64) -> &Document {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .find(|doc| doc.id == id)
            .unwrap()
    }
    fn seed(&mut self) {
        self.app.doc_mut().move_to(0, false);
        self.app.doc_mut().insert("dirty ", false);
    }
    fn save(&mut self) -> u64 {
        self.app
            .lsp
            .as_mut()
            .unwrap()
            .sync(&self.app.documents)
            .unwrap();
        self.app.execute("workbench.action.files.save", Value::Null);
        until(&mut self.app, "submit synchronized formatter", |app| {
            app.saving.formatting.active.is_some()
        });
        let token = self
            .app
            .saving
            .formatting
            .active
            .as_ref()
            .unwrap_or_else(|| panic!("save formatter not started: {}", self.app.message))
            .token;
        formatting_tests::held(self.app.lsp.as_mut().unwrap(), token);
        token
    }
    fn release(&mut self, token: u64, result: Value) {
        self.app
            .lsp
            .as_ref()
            .unwrap()
            .fixture_notify("fixture/release", json!({"id":token,"result":result}))
            .unwrap();
    }
    fn saved(&mut self) {
        until(&mut self.app, "save receipt", |app| !app.saves_pending());
    }
    fn release_retired(&mut self, token: u64) {
        self.release(token, prefix());
        until(&mut self.app, "actual formatter release", |app| {
            app.lsp.as_ref().unwrap().formatting_available()
        });
    }
}
fn until(app: &mut App, phase: &str, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll();
        if ready(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn prefix() -> Value {
    json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"fmt λ\n"}])
}
fn escape(app: &mut App) {
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
}

#[test]
fn formatted_save_preserves_unicode_crlf_and_is_one_undo_before_seed_history() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let seeded = fixture.app.doc().text.clone();
    let original_selection = fixture.app.doc().selections();
    let token = fixture.save();
    fixture.release(token, prefix());
    fixture.saved();
    let formatted = format!("fmt λ\r\ndirty {ORIGINAL}");
    assert_eq!(fixture.app.doc().id, fixture.a_id);
    assert_eq!(fixture.app.doc().text.to_string(), formatted);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), formatted.as_bytes());
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert!(!fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text, seeded);
    assert_eq!(fixture.app.doc().selections(), original_selection);
    assert!(fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, seeded);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), formatted);
    assert!(!fixture.app.doc().dirty());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B.as_bytes());
}

#[test]
fn unsynchronized_typing_and_save_in_one_input_batch_submits_current_epoch_on_poll() {
    let mut fixture = Fixture::new();
    // The fixture server initially knows only ORIGINAL. This models typing and
    // Ctrl+S before the CLI's next poll; no manual client.sync is permitted here.
    fixture.seed();
    let epoch = fixture.app.doc().text_epoch();
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    assert!(fixture.app.saves_pending());
    assert!(fixture.app.saving.formatting.active.is_none());
    until(
        &mut fixture.app,
        "synchronize then submit formatter",
        |app| app.saving.formatting.active.is_some(),
    );
    let pending = fixture.app.saving.formatting.active.as_ref().unwrap();
    assert_eq!(pending.epoch, epoch);
    assert_eq!(pending.document, fixture.a_id);
    let token = pending.token;
    assert!(
        fixture
            .app
            .lsp
            .as_ref()
            .unwrap()
            .formatting_request_current(token)
    );
    formatting_tests::held(fixture.app.lsp.as_mut().unwrap(), token);
    fixture.release(token, prefix());
    fixture.saved();
    let formatted = format!("fmt λ\r\ndirty {ORIGINAL}");
    assert_eq!(fixture.app.doc().text.to_string(), formatted);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), formatted.as_bytes());
    assert!(!fixture.app.doc().dirty());
    fixture.app.doc_mut().undo();
    assert_eq!(
        fixture.app.doc().text.to_string(),
        format!("dirty {ORIGINAL}")
    );
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text.to_string(), formatted);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn held_format_edit_undo_retires_save_without_reviving_equal_revision_or_destroying_redo() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let seeded = fixture.app.doc().text.clone();
    let token = fixture.save();
    fixture.app.doc_mut().insert("newer ", false);
    let newer = fixture.app.doc().text.clone();
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text, seeded);
    fixture.app.poll();
    assert!(!fixture.app.saves_pending());
    assert!(!fixture.app.lsp.as_ref().unwrap().formatting_available());
    fixture.release_retired(token);
    assert_eq!(fixture.app.doc().text, seeded);
    assert_eq!(fixture.app.doc().save_generation(), 0);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL.as_bytes());
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, newer);
    assert!(fixture.app.doc().dirty());
}

#[test]
fn held_format_survives_switch_and_edits_other_model_without_saving_it() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let token = fixture.save();
    fixture.app.open(&fixture.b).unwrap();
    until(&mut fixture.app, "switch B", |app| {
        app.doc().id == fixture.b_id
    });
    fixture.app.doc_mut().insert("new B ", false);
    let b_text = fixture.app.doc().text.clone();
    let b_epoch = fixture.app.doc().text_epoch();
    let b_selections = fixture.app.doc().selections();
    fixture.release(token, prefix());
    fixture.saved();
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    assert_eq!(fixture.app.doc().text, b_text);
    assert_eq!(fixture.app.doc().text_epoch(), b_epoch);
    assert_eq!(fixture.app.doc().selections(), b_selections);
    assert!(fixture.app.doc().dirty());
    assert_eq!(fixture.app.doc().save_generation(), 0);
    assert_eq!(
        fixture.model(fixture.a_id).text.to_string(),
        format!("fmt λ\r\ndirty {ORIGINAL}")
    );
    assert!(!fixture.model(fixture.a_id).dirty());
    assert_eq!(std::fs::read(&fixture.b).unwrap(), B.as_bytes());
}

#[test]
fn format_maps_primary_secondary_and_shared_view_carets_without_focus_fencing() {
    let mut fixture = Fixture::new();
    fixture.seed();
    fixture
        .app
        .doc_mut()
        .set_selections(vec![Selection::caret(8), Selection::caret(10)]);
    let first = fixture.app.panes[fixture.app.active_pane].id;
    fixture
        .app
        .execute("workbench.action.splitEditor", Value::Null);
    let second = fixture.app.panes[fixture.app.active_pane].id;
    fixture
        .app
        .doc_mut()
        .set_selections(vec![Selection::caret(3)]);
    let token = fixture.save();
    fixture.app.doc_mut().move_to(4, false);
    fixture.release(token, prefix());
    fixture.saved();
    assert_eq!(fixture.app.panes[fixture.app.active_pane].id, second);
    assert_eq!(fixture.app.doc().cursor, 11);
    let other = fixture.app.doc().view_state(Some(first));
    assert_eq!(other.cursor, 15);
    assert_eq!(other.secondary.len(), 1);
    assert_eq!(other.secondary[0].cursor, 17);
    assert_eq!(fixture.app.doc().id, fixture.a_id);
}

#[test]
fn malformed_formatter_batches_save_raw_with_notice_and_preserve_redo() {
    for edits in [
        json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"bad"},
               {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"overlap"}]),
        json!([{"range":{"start":{"line":0,"character":7},"end":{"line":0,"character":8}},"newText":"split surrogate"}]),
        json!({"not":"edits"}),
    ] {
        let mut fixture = Fixture::new();
        fixture.seed();
        let seeded = fixture.app.doc().text.clone();
        fixture.app.doc_mut().insert("redo ", false);
        let redo = fixture.app.doc().text.clone();
        fixture.app.doc_mut().undo();
        let epoch = fixture.app.doc().text_epoch();
        let selections = fixture.app.doc().selections();
        let token = fixture.save();
        fixture.release(token, edits);
        fixture.saved();
        assert!(
            fixture.app.message.contains("Formatting skipped"),
            "{}",
            fixture.app.message
        );
        assert_eq!(fixture.app.doc().text, seeded);
        assert_eq!(fixture.app.doc().text_epoch(), epoch);
        assert_eq!(fixture.app.doc().selections(), selections);
        assert_eq!(
            std::fs::read(&fixture.a).unwrap(),
            seeded.to_string().as_bytes()
        );
        fixture.app.doc_mut().redo();
        assert_eq!(fixture.app.doc().text, redo);
        assert!(fixture.app.doc().dirty());
    }
}

#[test]
fn save_deadline_saves_raw_but_retains_actual_callback_and_late_edits_stay_inert() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let seeded = fixture.app.doc().text.clone();
    let token = fixture.save();
    fixture
        .app
        .saving
        .formatting
        .active
        .as_mut()
        .unwrap()
        .started = Instant::now() - DEADLINE;
    assert!(fixture.app.poll_save_formatting(Instant::now()));
    fixture.saved();
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        seeded.to_string().as_bytes()
    );
    assert!(
        fixture.app.message.contains("1500 ms"),
        "{}",
        fixture.app.message
    );
    assert!(!fixture.app.lsp.as_ref().unwrap().formatting_available());
    let epoch = fixture.app.doc().text_epoch();
    fixture.release_retired(token);
    assert_eq!(fixture.app.doc().text, seeded);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().save_generation(), 1);
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, seeded);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn positively_settled_response_after_save_deadline_saves_raw_without_late_edit_or_undo_entry() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let seeded = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    let selections = fixture.app.doc().selections();
    let token = fixture.save();
    fixture.release(token, prefix());
    // Capture the actual framed response before App delivery. Aging only the
    // participant clock makes the response path deterministic: the ordinary
    // timer poll cannot cancel the participant before it handles this reply.
    let events = formatting_tests::until(fixture.app.lsp.as_mut().unwrap(), |_, events| {
        events.iter().any(|event| {
            matches!(event, crate::lsp::Event::FormattingResult(request, _) if request.token == token)
        })
    });
    let (request, result) = events
        .into_iter()
        .find_map(|event| match event {
            crate::lsp::Event::FormattingResult(request, result) if request.token == token => {
                Some((request, result))
            }
            _ => None,
        })
        .unwrap();
    assert!(fixture.app.lsp.as_ref().unwrap().formatting_available());
    fixture
        .app
        .saving
        .formatting
        .active
        .as_mut()
        .unwrap()
        .started = Instant::now() - DEADLINE - Duration::from_millis(1);
    fixture.app.save_formatting_result(request, result);
    fixture.saved();
    assert_eq!(fixture.app.doc().text, seeded);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().selections(), selections);
    assert_eq!(fixture.app.doc().save_generation(), 1);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        seeded.to_string().as_bytes()
    );
    assert!(
        fixture.app.message.contains("1500 ms"),
        "{}",
        fixture.app.message
    );
    assert!(
        fixture.app.message.contains("settlement"),
        "{}",
        fixture.app.message
    );
    assert!(fixture.app.lsp.as_ref().unwrap().formatting_available());
    fixture.app.doc_mut().undo();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
    fixture.app.doc_mut().redo();
    assert_eq!(fixture.app.doc().text, seeded);
    assert!(!fixture.app.doc().dirty());
}

#[test]
fn synchronized_close_reopen_cannot_revalidate_timeout_raw_save_at_equal_model_epoch() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let seeded = fixture.app.doc().text.clone();
    let epoch = fixture.app.doc().text_epoch();
    let token = fixture.save();
    let client = fixture.app.lsp.as_mut().unwrap();
    client.sync(&[]).unwrap();
    client.sync(&fixture.app.documents).unwrap();
    assert!(!client.formatting_request_current(token));
    fixture
        .app
        .saving
        .formatting
        .active
        .as_mut()
        .unwrap()
        .started = Instant::now() - DEADLINE;
    assert!(fixture.app.poll_save_formatting(Instant::now()));
    assert!(!fixture.app.saves_pending());
    fixture.release_retired(token);
    assert_eq!(fixture.app.doc().text, seeded);
    assert_eq!(fixture.app.doc().text_epoch(), epoch);
    assert_eq!(fixture.app.doc().save_generation(), 0);
    assert!(fixture.app.doc().dirty());
    assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL.as_bytes());
}

#[test]
fn escape_settings_aba_and_shutdown_retire_unapproved_formatting_without_persisting() {
    for retirement in ["escape", "settings", "shutdown"] {
        let mut fixture = Fixture::new();
        fixture.seed();
        let seeded = fixture.app.doc().text.clone();
        let token = fixture.save();
        match retirement {
            "escape" => escape(&mut fixture.app),
            "settings" => {
                let original = fixture.app.settings.clone();
                fixture.app.invalidate_settings_profile().unwrap();
                fixture.app.settings = Settings::default();
                fixture.app.invalidate_settings_profile().unwrap();
                fixture.app.settings = original;
            }
            "shutdown" => fixture.app.settle_persistence(),
            _ => unreachable!(),
        }
        assert!(!fixture.app.saves_pending());
        assert!(fixture.app.saving.formatting.active.is_none());
        fixture.release_retired(token);
        assert_eq!(fixture.app.doc().text, seeded);
        assert_eq!(fixture.app.doc().save_generation(), 0);
        assert!(fixture.app.doc().dirty());
        assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL.as_bytes());
        fixture.app.doc_mut().undo();
        assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
        fixture.app.doc_mut().redo();
        assert_eq!(fixture.app.doc().text, seeded);
    }
}

#[test]
fn superseded_formatter_retains_actual_lane_and_new_save_recaptures_raw_latest_text() {
    let mut fixture = Fixture::new();
    fixture.seed();
    let token = fixture.save();
    fixture.app.doc_mut().insert("latest ", false);
    let latest = fixture.app.doc().text.clone();
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    fixture.saved();
    assert_eq!(fixture.app.doc().text, latest);
    assert_eq!(
        std::fs::read(&fixture.a).unwrap(),
        latest.to_string().as_bytes()
    );
    assert!(
        fixture.app.message.contains("callback"),
        "{}",
        fixture.app.message
    );
    assert!(!fixture.app.lsp.as_ref().unwrap().formatting_available());
    fixture.release_retired(token);
    assert_eq!(fixture.app.doc().text, latest);
    assert_eq!(fixture.app.doc().save_generation(), 1);
}

#[test]
fn deferred_close_and_quit_wait_for_formatted_receipt_and_escape_preserves_source() {
    for command in [
        "workbench.action.closeActiveEditor",
        "workbench.action.quit",
    ] {
        for cancel in [false, true] {
            let mut fixture = Fixture::new();
            fixture.seed();
            let seeded = fixture.app.doc().text.clone();
            let token = fixture.save();
            fixture.app.execute(command, Value::Null);
            assert!(fixture.app.modal.is_none(), "{}", fixture.app.message);
            assert!(fixture.app.running);
            assert!(
                fixture
                    .app
                    .documents
                    .iter()
                    .any(|doc| doc.id == fixture.a_id)
            );
            if cancel {
                escape(&mut fixture.app);
                fixture.release_retired(token);
                assert!(fixture.app.running);
                assert_eq!(fixture.app.doc().id, fixture.a_id);
                assert_eq!(fixture.app.doc().text, seeded);
                assert_eq!(std::fs::read(&fixture.a).unwrap(), ORIGINAL.as_bytes());
            } else {
                fixture.release(token, prefix());
                fixture.saved();
                assert_eq!(
                    std::fs::read(&fixture.a).unwrap(),
                    format!("fmt λ\r\ndirty {ORIGINAL}").as_bytes()
                );
                if command == "workbench.action.quit" {
                    assert!(!fixture.app.running);
                } else {
                    assert!(
                        !fixture
                            .app
                            .documents
                            .iter()
                            .any(|doc| doc.id == fixture.a_id)
                    );
                    assert_eq!(fixture.app.doc().id, fixture.b_id);
                }
            }
            assert_eq!(std::fs::read(&fixture.b).unwrap(), B.as_bytes());
        }
    }
}
