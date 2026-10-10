//! Native merge publication and actual persistence ownership, independent byte oracles.
use super::*;
use crate::{
    document::Selection,
    editor_groups::{GroupId, Membership},
    save_worker::GatePoint,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::mpsc::{Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};

const A: &str = "猫🙂 A baseline\r\nlast\r\n";
const B: &str = "B λ baseline\r\nlast\r\n";
const C: &str = "C e\u{301} baseline\r\nlast\r\n";
const MERGE: &str = "workbench.action.closeGroup";
const SPLIT: &str = "workbench.action.splitEditor";
const CLOSE: &str = "workbench.action.closeActiveEditor";
const WAIT: Duration = Duration::from_secs(5);
struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    paths: [PathBuf; 3],
    ids: [u64; 3],
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let paths = [root.join("a.cpp"), root.join("b.cpp"), root.join("c.cpp")];
        for (path, text) in paths.iter().zip([A, B, C]) {
            std::fs::write(path, text).unwrap();
        }
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.extension_node = "vscli-merge-node-missing".into();
        app.sidebar = false;
        app.settings = crate::settings::Settings::from_values(
            json!({
                "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,
                "files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}
            })
            .as_object()
            .unwrap()
            .clone(),
            "native merge fixture",
        )
        .unwrap();
        let mut ids = [0; 3];
        for (index, path) in paths.iter().enumerate() {
            app.open(path).unwrap();
            until(&mut app, "open", |app| {
                app.active_document()
                    .is_some_and(|doc| doc.path.as_ref() == Some(path))
            });
            ids[index] = app.doc().id;
        }
        let member = app.editor_groups.memberships(ids[0]).next().unwrap();
        app.focus_tab(member).unwrap();
        Self {
            _directory: directory,
            app,
            paths,
            ids,
        }
    }
    fn member(&self, index: usize) -> Membership {
        self.app
            .editor_groups
            .memberships(self.ids[index])
            .next()
            .unwrap()
    }
    fn focus(&mut self, member: Membership) {
        self.app.focus_tab(member).unwrap();
    }
    fn model(&self, index: usize) -> &Document {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .find(|doc| doc.id == self.ids[index])
            .unwrap()
    }
    fn model_mut(&mut self, index: usize) -> &mut Document {
        self.app
            .documents
            .iter_mut()
            .chain(&mut self.app.hidden_documents)
            .find(|doc| doc.id == self.ids[index])
            .unwrap()
    }
    fn disks(&self) {
        for (path, text) in self.paths.iter().zip([A, B, C]) {
            assert_eq!(std::fs::read(path).unwrap(), text.as_bytes());
        }
    }
    fn split(&mut self) -> (Membership, Membership) {
        let source = self.app.active_tab_membership().unwrap();
        command(&mut self.app, SPLIT);
        (source, self.app.active_tab_membership().unwrap())
    }
    fn gates(&mut self, points: Vec<GatePoint>) -> Gates {
        assert!(!self.app.saves_pending());
        let (worker, entered, release) = Worker::fixture_gated(points);
        self.app.saving.worker = worker;
        Gates { entered, release }
    }
}
struct Gates {
    entered: Receiver<GatePoint>,
    release: SyncSender<()>,
}
impl Gates {
    fn reach(&self, app: &mut App, expected: GatePoint) {
        let deadline = Instant::now() + WAIT;
        loop {
            app.poll();
            match self.entered.try_recv() {
                Ok(point) => {
                    assert_eq!(point, expected);
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    panic!("{expected:?} disconnected: {}", app.save_fixture_status())
                }
                Err(TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "{expected:?}: {}",
                app.save_fixture_status()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn release(&self) {
        self.release.try_send(()).unwrap();
    }
}
impl Drop for Gates {
    fn drop(&mut self) {
        let _ = self.release.try_send(());
    }
}
fn until(app: &mut App, phase: &str, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if ready(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{phase}: {}",
            app.save_fixture_status()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn command(app: &mut App, id: &str) {
    app.execute(id, Value::Null);
}
fn answer(app: &mut App, key: char) {
    app.event(Event::Key(KeyEvent::new(
        KeyCode::Char(key),
        KeyModifiers::NONE,
    )));
}
fn policy(app: &mut App, key: &str, value: Value) {
    let mut values = app.settings.extension_layers()[0].clone();
    values.insert(key.into(), value);
    app.settings = crate::settings::Settings::from_values(values, "merge policy fixture").unwrap();
}
type View = (usize, Option<usize>, Vec<Selection>, usize, usize);
fn view(doc: &Document, group: GroupId) -> View {
    let v = doc.view_state(Some(group.value()));
    (v.cursor, v.anchor, v.secondary.clone(), v.top, v.left)
}
fn docs(app: &App, group: GroupId) -> Vec<u64> {
    app.editor_groups
        .group(group)
        .unwrap()
        .tabs()
        .iter()
        .map(|tab| tab.document())
        .collect()
}
fn draw(app: &mut App) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
}
fn selections() -> Vec<Selection> {
    vec![
        Selection {
            cursor: 2,
            anchor: Some(5),
            desired_column: None,
        },
        Selection::caret(12),
    ]
}

#[test]
fn multiple_inactive_documents_keep_exact_target_snippet_and_copy_only_active_source_with_redo() {
    let mut f = Fixture::new();
    let (source, target) = f.split();
    f.app.doc_mut().move_to(0, false);
    f.app
        .doc_mut()
        .insert_snippet(
            &crate::snippet::Template::parse("${1:x}$0").unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(f.app.doc().in_snippet());
    let kept = view(f.app.doc(), target.group);
    let a_text = f.app.doc().text.to_string();
    let b = f.member(1);
    f.focus(b);
    f.app.doc_mut().insert("redo ", false);
    f.app.doc_mut().undo();
    f.app.doc_mut().set_selections(selections());
    f.app.doc_mut().top = 1;
    f.app.doc_mut().left = 2;
    let epoch = f.app.doc().text_epoch();
    let ids: Vec<_> = f.app.documents.iter().map(|doc| doc.id).collect();
    draw(&mut f.app);
    let frame = f.app.editor_presentation.proof().unwrap().clone();
    let copied = view(f.app.doc(), source.group);
    command(&mut f.app, MERGE);
    assert!(
        f.app.message.starts_with("Group merged"),
        "{}",
        f.app.message
    );
    assert_eq!(f.app.editor_groups.groups().len(), 1);
    assert_eq!(docs(&f.app, target.group), f.ids);
    assert_eq!(f.app.doc().id, f.ids[1]);
    assert_eq!(view(f.app.doc(), target.group), copied);
    assert_eq!(view(f.model(0), target.group), kept);
    assert_eq!(view(f.model(2), target.group), (0, None, vec![], 0, 0));
    assert_eq!(f.app.doc().text_epoch(), epoch);
    assert_eq!(
        f.app.documents.iter().map(|doc| doc.id).collect::<Vec<_>>(),
        ids
    );
    assert!(!f.app.editor_groups.membership_current(source));
    assert!(!f.app.editor_presentation.current(
        &frame,
        f.app.editor_layout(),
        f.app.editor_groups()
    ));
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), format!("redo {B}"));
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), B);
    f.focus(target);
    assert!(f.app.doc().in_snippet());
    assert_eq!(f.app.doc().text.to_string(), a_text);
    assert!(f.app.doc_mut().step_snippet(false).unwrap());
    f.disks();
}

#[test]
fn active_duplicate_copy_retires_target_private_pair_but_preserves_other_group_and_undo() {
    let mut f = Fixture::new();
    let (source, target) = f.split();
    let end = f.app.doc().len();
    f.app.doc_mut().move_to(end, false);
    let options = crate::editing_profile::TypingOptions {
        profile: crate::editing_profile::ProfileId::Cpp,
        ..Default::default()
    };
    f.app.doc_mut().type_character('(', options, false).unwrap();
    let pair_text = f.app.doc().text.to_string();
    assert_eq!(pair_text, format!("{A}()"));
    command(&mut f.app, "workbench.action.splitEditorDown");
    let other = f.app.active_tab_membership().unwrap();
    f.app.doc_mut().move_to(8, false);
    let unrelated = view(f.app.doc(), other.group);
    // Make target most-recent other group; source-visible geometry supersedes it.
    f.focus(target);
    f.focus(source);
    f.app.doc_mut().move_to(end + 1, false);
    let copied = view(f.app.doc(), source.group);
    command(&mut f.app, MERGE);
    assert_eq!(f.app.active_tab_membership(), Some(target));
    assert_eq!(view(f.app.doc(), target.group), copied);
    assert_eq!(view(f.app.doc(), other.group), unrelated);
    f.app.doc_mut().type_character(')', options, false).unwrap();
    assert_eq!(f.app.doc().text.to_string(), format!("{A}())")); // Literal: old target marker retired.
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), pair_text);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), format!("{A}())"));
    f.disks();
}

#[test]
fn inactive_duplicate_pair_survives_merge_and_text_neutral_skip_preserves_redo() {
    let mut f = Fixture::new();
    let (_, target) = f.split();
    let end = f.app.doc().len();
    f.app.doc_mut().move_to(end, false);
    let options = crate::editing_profile::TypingOptions {
        profile: crate::editing_profile::ProfileId::Cpp,
        ..Default::default()
    };
    f.app.doc_mut().type_character('(', options, false).unwrap();
    f.app.doc_mut().type_character('x', options, false).unwrap();
    let with_x = f.app.doc().text.to_string();
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), format!("{A}()"));
    f.focus(f.member(1));
    command(&mut f.app, MERGE);
    f.focus(target);
    f.app.doc_mut().type_character(')', options, false).unwrap();
    assert_eq!(f.app.doc().text.to_string(), format!("{A}()"));
    assert_eq!(f.app.doc().cursor, end + 2);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), with_x);
    f.disks();
}

#[test]
fn changed_policy_and_late_document_or_layout_refusal_publish_nothing_and_keep_redo() {
    for failure in 0..3 {
        let mut f = Fixture::new();
        let (source, target) = f.split();
        f.focus(source);
        f.app.doc_mut().insert("redo ", false);
        f.app.doc_mut().undo();
        if failure == 0 {
            policy(
                &mut f.app,
                "workbench.editor.openPositioning",
                json!("left"),
            );
        }
        if failure == 1 {
            f.model_mut(2).secondary = vec![Selection::caret(0); 10_000];
        }
        if failure == 2 {
            let reversed = f
                .app
                .editor_groups
                .groups()
                .iter()
                .rev()
                .map(|group| group.id())
                .collect::<Vec<_>>();
            let plan = f
                .app
                .editor_layout
                .prepare_flat(&reversed, crate::editor_layout::Axis::Rows)
                .unwrap();
            f.app.editor_layout.commit(plan).unwrap();
        }
        let groups = f.app.editor_groups.clone();
        let source_view = view(f.model(0), source.group);
        let target_view = view(f.model(0), target.group);
        let epoch = f.model(0).text_epoch();
        let geometry = f.app.editor_layout.clone();
        command(&mut f.app, MERGE);
        assert!(
            f.app.message.starts_with("Close Group rejected"),
            "{}",
            f.app.message
        );
        assert_eq!(f.app.editor_groups, groups);
        assert_eq!(view(f.model(0), source.group), source_view);
        assert_eq!(view(f.model(0), target.group), target_view);
        assert_eq!(f.model(0).text_epoch(), epoch);
        assert_eq!(f.app.editor_layout, geometry);
        f.app.doc_mut().redo();
        assert_eq!(f.app.doc().text.to_string(), format!("redo {A}"));
        f.disks();
    }
}

#[test]
fn explicit_merge_removes_source_with_close_empty_false_and_sole_group_is_inert() {
    let mut f = Fixture::new();
    policy(
        &mut f.app,
        "workbench.editor.openPositioning",
        json!("left"),
    );
    let original = f.app.editor_groups.clone();
    command(&mut f.app, MERGE);
    assert_eq!(f.app.editor_groups, original);
    policy(
        &mut f.app,
        "workbench.editor.openPositioning",
        json!("right"),
    );
    policy(
        &mut f.app,
        "workbench.editor.closeEmptyGroups",
        json!(false),
    );
    let (source, target) = f.split();
    f.focus(source);
    command(&mut f.app, MERGE);
    assert_eq!(f.app.editor_groups.groups().len(), 1);
    assert_eq!(f.app.active_tab_membership(), Some(target));
    assert!(f.app.modal.is_none());
    f.disks();
    let groups = f.app.editor_groups.clone();
    let selected = f.app.doc().selections();
    command(&mut f.app, MERGE);
    assert_eq!(f.app.editor_groups, groups);
    assert_eq!(f.app.doc().selections(), selected);
}

#[test]
fn approved_source_save_close_publishes_real_receipt_but_never_closes_reused_destination() {
    for newer in [false, true] {
        let mut f = Fixture::new();
        let source = f.member(0);
        f.app.doc_mut().insert("captured λ ", false);
        let captured = f.app.doc().text.to_string();
        let gates = f.gates(vec![GatePoint::BeforeCommit, GatePoint::BeforeFinish]);
        command(&mut f.app, CLOSE);
        answer(&mut f.app, 's');
        gates.reach(&mut f.app, GatePoint::BeforeCommit);
        command(&mut f.app, SPLIT);
        let target = f.app.active_tab_membership().unwrap();
        f.focus(source);
        command(&mut f.app, MERGE);
        assert_eq!(f.app.active_tab_membership(), Some(target));
        assert!(f.app.saving.active.as_ref().unwrap().authorized);
        assert!(f.app.saving.active.as_ref().unwrap().continuation.is_some());
        gates.release();
        gates.reach(&mut f.app, GatePoint::BeforeFinish);
        assert_eq!(std::fs::read(&f.paths[0]).unwrap(), captured.as_bytes());
        assert_eq!(f.app.doc().save_generation(), 0);
        let expected = if newer {
            f.app.doc_mut().insert("newer 🙂 ", false);
            f.app.doc().text.to_string()
        } else {
            captured.clone()
        };
        gates.release();
        until(&mut f.app, "source saved receipt", |app| {
            !app.saves_pending()
        });
        assert!(f.app.editor_groups.membership_current(target));
        assert!(!f.app.editor_groups.membership_current(source));
        assert_eq!(f.app.doc().id, f.ids[0]);
        assert_eq!(f.app.doc().save_generation(), 1);
        assert_eq!(f.app.doc().dirty(), newer);
        assert_eq!(f.app.doc().text.to_string(), expected);
        assert_eq!(std::fs::read(&f.paths[0]).unwrap(), captured.as_bytes());
        f.app.doc_mut().undo();
        assert_eq!(
            f.app.doc().text.to_string(),
            if newer { captured } else { A.into() }
        );
        f.app.doc_mut().redo();
        assert_eq!(f.app.doc().text.to_string(), expected);
        assert_eq!(std::fs::read(&f.paths[1]).unwrap(), B.as_bytes());
        assert_eq!(std::fs::read(&f.paths[2]).unwrap(), C.as_bytes());
    }
}

#[test]
fn approved_save_as_retains_destination_and_identity_after_source_group_collapse() {
    let mut f = Fixture::new();
    let (source, target) = f.split();
    f.focus(source);
    f.app.doc_mut().insert("SaveAs 🙂 ", false);
    let captured = f.app.doc().text.to_string();
    let destination = f.paths[0].parent().unwrap().join("renamed.cpp");
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    command(&mut f.app, "workbench.action.files.saveAs");
    f.app.prompt.as_mut().unwrap().text = destination.to_string_lossy().into_owned();
    f.app.event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    command(&mut f.app, MERGE);
    assert_eq!(f.app.active_tab_membership(), Some(target));
    gates.release();
    until(&mut f.app, "SaveAs merge receipt", |app| {
        !app.saves_pending()
    });
    assert_eq!(f.app.doc().path.as_ref(), Some(&destination));
    assert_eq!(f.app.doc().id, f.ids[0]);
    assert_eq!(f.app.doc().save_generation(), 1);
    assert_eq!(std::fs::read(&destination).unwrap(), captured.as_bytes());
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), A);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), captured);
    f.disks();
}

#[test]
fn unrelated_approved_close_still_finishes_while_remaining_batch_and_source_dialog_retire() {
    let mut f = Fixture::new();
    f.focus(f.member(2));
    let (_, target) = f.split();
    let a = f.member(0);
    f.focus(a);
    f.app.doc_mut().insert("approved λ ", false);
    let captured = f.app.doc().text.to_string();
    let gates = f.gates(vec![GatePoint::BeforeCommit]);
    command(&mut f.app, "workbench.action.closeEditorsInGroup");
    answer(&mut f.app, 's');
    gates.reach(&mut f.app, GatePoint::BeforeCommit);
    // Merge the other group into the original: original approved A membership remains.
    f.focus(target);
    command(&mut f.app, MERGE);
    assert!(f.app.closing_group.is_none());
    assert!(f.app.saving.active.as_ref().unwrap().continuation.is_some());
    assert!(f.app.editor_groups.membership_current(a));
    gates.release();
    until(&mut f.app, "independent original close", |app| {
        !app.saves_pending()
    });
    assert!(!f.app.editor_groups.membership_current(a));
    assert_eq!(std::fs::read(&f.paths[0]).unwrap(), captured.as_bytes());
    assert!(f.app.editor_groups.memberships(f.ids[1]).next().is_some());
    assert!(f.app.editor_groups.memberships(f.ids[2]).next().is_some());
    assert_eq!(std::fs::read(&f.paths[1]).unwrap(), B.as_bytes());
    assert_eq!(std::fs::read(&f.paths[2]).unwrap(), C.as_bytes());
}

#[test]
fn unapproved_source_modal_retires_without_global_save_generation_or_dirty_history_change() {
    let mut f = Fixture::new();
    let (source, target) = f.split();
    f.focus(f.member(1));
    f.app.doc_mut().insert("dirty 🙂 ", false);
    let text = f.app.doc().text.to_string();
    command(&mut f.app, CLOSE);
    assert!(matches!(
        f.app.modal,
        Some(Modal::Confirm(AfterSave::Close))
    ));
    let generation = f.app.saving.close_generation;
    command(&mut f.app, MERGE);
    assert!(f.app.modal.is_none());
    assert!(f.app.close_membership.is_none());
    assert_eq!(f.app.saving.close_generation, generation);
    assert!(!f.app.editor_groups.membership_current(source));
    assert_eq!(f.app.active_tab_membership().unwrap().group, target.group);
    assert_eq!(f.app.doc().text.to_string(), text);
    assert!(f.app.doc().dirty());
    f.app.doc_mut().undo();
    assert_eq!(f.app.doc().text.to_string(), B);
    f.app.doc_mut().redo();
    assert_eq!(f.app.doc().text.to_string(), text);
    f.disks();
}

#[test]
fn original_titles_are_distinct_and_current_group_only_arguments_refuse_without_mutation() {
    assert!(COMMANDS.contains(&("View: Close Group", MERGE)));
    assert!(COMMANDS.contains(&(
        "View: Close All Editors in Group",
        "workbench.action.closeEditorsInGroup"
    )));
    assert!(
        !COMMANDS
            .iter()
            .any(|(_, id)| *id == "workbench.action.closeEditorsAndGroup")
    );
    let mut f = Fixture::new();
    let _ = f.split();
    let groups = f.app.editor_groups.clone();
    f.app.execute(MERGE, json!({"groupId":1}));
    assert_eq!(f.app.editor_groups, groups);
    assert!(f.app.message.contains("only the current group"));
    f.disks();
}

#[test]
fn existing_tab_without_exact_target_view_uses_fresh_origin_instead_of_fallback_geometry() {
    let mut f = Fixture::new();
    let (source, target) = f.split();
    f.app.doc_mut().move_to(7, false);
    f.app.doc_mut().remove_view(target.group.value());
    f.app.doc_mut().activate_view(source.group.value());
    f.app.doc_mut().move_to(4, false);
    assert!(
        f.app
            .doc()
            .retained_view_state(target.group.value())
            .is_none()
    );
    f.focus(f.member(1));
    command(&mut f.app, MERGE);
    assert!(f.app.editor_groups.membership_current(target));
    f.focus(target);
    assert_eq!(f.app.doc().cursor, 0);
    assert!(f.app.doc().secondary.is_empty());
    assert_eq!(f.app.doc().text.to_string(), A);
    f.disks();
}
