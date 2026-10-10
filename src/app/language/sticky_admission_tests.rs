//! Actual diagnostic/rename publication must stay atomic on recoverable
//! native group-MRU admission failure. Intended app::language child.
use super::*;
use crate::{document::Selection, editor_groups::UiProof, keys::Profile};
use serde_json::json;
use std::time::{Duration, Instant};

const A_DISK: &str = "λ🙂 A baseline\r\nlast\r\n";
const B_DISK: &str = "猫🙂 B baseline\r\nlast\r\n";

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    a: PathBuf,
    b: PathBuf,
    b_id: u64,
    b_edited: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let a = root.join("a.cpp");
        let b = root.join("b.cpp");
        std::fs::write(&a, A_DISK).unwrap();
        std::fs::write(&b, B_DISK).unwrap();
        let mut app = App::new(root, Profile::Linux);
        app.settings = crate::settings::Settings::from_values(
            json!({
                "vscli.languageServer.enabled":false,
                "breadcrumbs.enabled":false,
                "files.autoSave":"off",
                "editor.formatOnSave":false,
                "editor.codeActionsOnSave":{}
            })
            .as_object()
            .unwrap()
            .clone(),
            "actual admission rejection fixture",
        )
        .unwrap();
        app.open(&b).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .active_document()
            .is_none_or(|doc| doc.path.as_ref() != Some(&b))
        {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(Duration::from_millis(1));
        }
        let b_id = app.doc().id;
        app.doc_mut().move_to(0, false);
        app.execute("type", json!({"text":"edit "}));
        let b_edited = app.doc().text.to_string();
        app.doc_mut().set_selections(vec![Selection {
            cursor: 0,
            anchor: Some(3),
            desired_column: Some(7),
        }]);
        Self {
            _directory: directory,
            app,
            a,
            b,
            b_id,
            b_edited,
        }
    }
    fn assert_disks(&self) {
        assert_eq!(std::fs::read(&self.a).unwrap(), A_DISK.as_bytes());
        assert_eq!(std::fs::read(&self.b).unwrap(), B_DISK.as_bytes());
    }
    fn assert_b_undo_redo(&mut self) {
        assert_eq!(self.app.doc().id, self.b_id);
        assert_eq!(self.app.doc().text.to_string(), self.b_edited);
        self.app.doc_mut().undo();
        assert_eq!(self.app.doc().text.to_string(), B_DISK);
        self.app.doc_mut().redo();
        assert_eq!(self.app.doc().text.to_string(), self.b_edited);
        assert_eq!(self.app.doc().save_generation(), 0);
        assert!(self.app.doc().dirty());
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Model {
    id: u64,
    path: Option<PathBuf>,
    text: String,
    revision: u64,
    saved_revision: u64,
    epoch: u64,
    save_generation: u64,
    dirty: bool,
    selections: Vec<Selection>,
}
impl Model {
    fn capture(doc: &Document) -> Self {
        Self {
            id: doc.id,
            path: doc.path.clone(),
            text: doc.text.to_string(),
            revision: doc.revision,
            saved_revision: doc.saved_revision,
            epoch: doc.text_epoch(),
            save_generation: doc.save_generation(),
            dirty: doc.dirty(),
            selections: doc.selections(),
        }
    }
}
struct Before {
    visible: Vec<Model>,
    hidden: Vec<Model>,
    groups: UiProof,
    active: usize,
    active_pane: usize,
    panes: Vec<(u64, u64)>,
    focus: Focus,
}
impl Before {
    fn capture(app: &App) -> Self {
        Self {
            visible: app.documents.iter().map(Model::capture).collect(),
            hidden: app.hidden_documents.iter().map(Model::capture).collect(),
            groups: app.editor_groups.proof(),
            active: app.active,
            active_pane: app.active_pane,
            panes: app
                .panes
                .iter()
                .map(|pane| (pane.id, pane.document))
                .collect(),
            focus: app.focus.clone(),
        }
    }
    fn assert_retained(&self, app: &App) {
        assert_eq!(
            app.documents.iter().map(Model::capture).collect::<Vec<_>>(),
            self.visible
        );
        assert_eq!(
            app.hidden_documents
                .iter()
                .map(Model::capture)
                .collect::<Vec<_>>(),
            self.hidden
        );
        assert!(app.editor_groups.proof_current(&self.groups));
        assert_eq!(app.active, self.active);
        assert_eq!(app.active_pane, self.active_pane);
        assert_eq!(
            app.panes
                .iter()
                .map(|pane| (pane.id, pane.document))
                .collect::<Vec<_>>(),
            self.panes
        );
        assert!(app.focus == self.focus, "Rejected admission changed focus");
        assert!(!app.saves_pending());
    }
}

#[test]
fn hidden_diagnostic_jump_rejects_real_mru_reservation_without_model_focus_or_caret_edits() {
    let mut fixture = Fixture::new();
    let mut hidden = Document::open(&fixture.a).unwrap();
    hidden.move_to(1, false);
    hidden.insert("dirty λ🙂 ", false);
    hidden.set_selections(vec![Selection {
        cursor: 1,
        anchor: Some(4),
        desired_column: Some(2),
    }]);
    let hidden_id = hidden.id;
    let hidden_epoch = hidden.text_epoch();
    let hidden_text = hidden.text.to_string();
    fixture.app.hidden_documents.push(hidden);
    let before = Before::capture(&fixture.app);
    fixture.app.editor_groups.fail_next_recent_reservation();
    let error = fixture
        .app
        .language_action(&LanguageAction::DiagnosticLocation {
            document: hidden_id,
            text_epoch: hidden_epoch,
            range: lsp::Range {
                start: lsp::Position {
                    line: 0,
                    character: 0,
                },
                end: lsp::Position {
                    line: 0,
                    character: 1,
                },
            },
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("MRU"), "{error:#}");
    before.assert_retained(&fixture.app);
    assert!(
        fixture
            .app
            .editor_groups
            .memberships(hidden_id)
            .next()
            .is_none()
    );
    fixture.assert_disks();
    fixture.assert_b_undo_redo();
    let hidden = fixture
        .app
        .hidden_documents
        .iter_mut()
        .find(|doc| doc.id == hidden_id)
        .unwrap();
    hidden.undo();
    assert_eq!(hidden.text.to_string(), A_DISK);
    hidden.redo();
    assert_eq!(hidden.text.to_string(), hidden_text);
    assert!(hidden.dirty());
    fixture.assert_disks();
    // A failed reveal must also release navigation-observation suspension.
    // Qualify the subsequent accepted jump through original Back, rather than
    // inspecting private history-stack entries or suspension counters.
    fixture.app.doc_mut().move_to(0, false);
    fixture
        .app
        .observe_navigation(navigation_history::Reason::Ordinary);
    let current_epoch = fixture.app.doc().text_epoch();
    fixture
        .app
        .language_action(&LanguageAction::DiagnosticLocation {
            document: fixture.b_id,
            text_epoch: current_epoch,
            range: lsp::Range {
                start: lsp::Position {
                    line: 1,
                    character: 0,
                },
                end: lsp::Position {
                    line: 1,
                    character: 0,
                },
            },
        })
        .unwrap();
    assert_eq!(fixture.app.doc().row(), 1);
    assert!(fixture.app.can_navigate_back());
    fixture
        .app
        .execute("workbench.action.navigateBack", Value::Null);
    assert_eq!(fixture.app.doc().id, fixture.b_id);
    assert_eq!(fixture.app.doc().cursor, 0);
    assert_eq!(fixture.app.doc().text.to_string(), fixture.b_edited);
    fixture.assert_disks();
}

#[test]
fn multi_file_native_rename_rejects_real_mru_reservation_before_any_text_or_model_publication() {
    let mut fixture = Fixture::new();
    let a_uri = lsp::file_uri(&fixture.a).unwrap();
    let b_uri = lsp::file_uri(&fixture.b).unwrap();
    let edits = json!({"changes":{
        a_uri:[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"renamed λ"}],
        b_uri:[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":4}},"newText":"renamed"}]
    }});
    let before = Before::capture(&fixture.app);
    fixture.app.editor_groups.fail_next_recent_reservation();
    let error = fixture.app.apply_workspace_edit(edits).unwrap_err();
    assert!(format!("{error:#}").contains("MRU"), "{error:#}");
    before.assert_retained(&fixture.app);
    assert_eq!(fixture.app.documents.len(), 1);
    assert!(fixture.app.hidden_documents.is_empty());
    assert_eq!(fixture.app.editor_groups.groups().len(), 1);
    assert_eq!(fixture.app.editor_groups.groups()[0].tabs().len(), 1);
    fixture.assert_disks();
    fixture.assert_b_undo_redo();
    fixture.assert_disks();
}
