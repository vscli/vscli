use super::*;
use crate::{
    document::Selection,
    save_worker::{GatePoint as SaveGate, Worker as SaveWorker},
};
use serde_json::json;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
const A: &str = "α😀 A\r\nnext\r\n";
const B: &str = "猫 B\r\n";
const WAIT: Duration = Duration::from_secs(5);
struct Fixture {
    _root: tempfile::TempDir,
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
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        std::fs::write(&a, A).unwrap();
        std::fs::write(&b, B).unwrap();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.open(&a).unwrap();
        until(&mut app, "open A", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&a))
        });
        let a_id = app.doc().id;
        app.open(&b).unwrap();
        until(&mut app, "open B", |app| {
            app.active_document()
                .is_some_and(|doc| doc.path.as_ref() == Some(&b))
        });
        let b_id = app.doc().id;
        app.open(&a).unwrap();
        until(&mut app, "focus A", |app| app.doc().id == a_id);
        Self {
            _root: directory,
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
    fn model_mut(&mut self, id: u64) -> &mut Document {
        self.app
            .documents
            .iter_mut()
            .chain(&mut self.app.hidden_documents)
            .find(|doc| doc.id == id)
            .unwrap()
    }
    fn mirrors(&self) -> Vec<MirrorSnapshot> {
        self.app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .map(|doc| MirrorSnapshot {
                document: doc.id,
                uri: doc
                    .path
                    .as_ref()
                    .map(|path| crate::lsp::file_uri(path).unwrap())
                    .unwrap_or_else(|| format!("untitled:vscli-{}", doc.id)),
                path: doc.path.clone(),
                revision: doc.revision,
                text_epoch: doc.text_epoch(),
                language_epoch: doc.language_epoch(),
                version: doc.text_epoch() + 1,
            })
            .collect()
    }
    fn capture(&self, edits: Vec<(u64, Value)>) -> Arc<Capture> {
        let mirrors = self.mirrors();
        let entries: Vec<_> = edits
            .into_iter()
            .map(|(id, edits)| {
                let mirror = mirrors.iter().find(|mirror| mirror.document == id).unwrap();
                json!({"document":id,"version":mirror.version,"uri":mirror.uri,"edits":edits})
            })
            .collect();
        let origin = Origin::new(
            1,
            "workspace-test",
            "fixture.extension",
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        self.app
            .capture_workspace_transaction(
                origin,
                &json!({"entries":entries,"metadata":{"isRefactoring":false}}),
                &mirrors,
                Instant::now(),
            )
            .unwrap()
    }
    fn standard(&self) -> Arc<Capture> {
        self.capture(vec![
            (self.a_id, json!([edit(0, 4, 0, 5, "X")])),
            (self.b_id, json!([edit(0, 2, 0, 3, "Y")])),
        ])
    }
    fn start(&mut self, capture: Arc<Capture>) -> u64 {
        self.app.start_workspace_transaction(capture).unwrap()
    }
    fn finish(&mut self, token: u64) -> Completion {
        let deadline = Instant::now() + WAIT;
        loop {
            self.app.poll_workspace_transactions();
            if self.app.workspace_transaction_completion().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "Workspace worker did not join");
            std::thread::yield_now();
        }
        self.app
            .take_workspace_transaction_completion(token)
            .unwrap()
    }
    fn apply(&mut self, capture: Arc<Capture>) -> Completion {
        let token = self.start(capture);
        self.finish(token)
    }
    fn gates(&mut self, points: Vec<GatePoint>) -> (Receiver<GatePoint>, SyncSender<()>) {
        let (worker, entered, release) = fixture_worker(points);
        self.app.workspace_transactions.worker = worker;
        (entered, release)
    }
    fn focus(&mut self, id: u64) {
        let path = self.model(id).path.clone().unwrap();
        self.app.open(&path).unwrap();
        until(&mut self.app, "focus origin", |app| app.doc().id == id);
    }
    fn unchanged_disks(&self) {
        assert_eq!(std::fs::read_to_string(&self.a).unwrap(), A);
        assert_eq!(std::fs::read_to_string(&self.b).unwrap(), B);
    }
}
fn edit(sl: usize, sc: usize, el: usize, ec: usize, text: &str) -> Value {
    json!({"range":{"start":{"line":sl,"character":sc},"end":{"line":el,"character":ec}},"newText":text})
}
fn until(app: &mut App, phase: &str, condition: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition(app) {
        app.poll();
        assert!(Instant::now() < deadline, "{phase}: {}", app.message);
        std::thread::yield_now();
    }
}
fn save_gate(app: &mut App, receiver: &Receiver<SaveGate>, expected: SaveGate) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        match receiver.try_recv() {
            Ok(phase) => {
                assert_eq!(phase, expected);
                return;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => panic!("Save gate disappeared"),
        };
        assert!(
            Instant::now() < deadline,
            "Missing {expected:?}: {}",
            app.message
        );
        std::thread::yield_now();
    }
}

#[test]
fn real_worker_grouped_unicode_crlf_undo_redo_from_either_peer_preserves_disks() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    let id = fixture.app.doc().id;
    let baseline = fixture
        .model(a)
        .capture_save(fixture.a.clone())
        .unwrap()
        .save_generation();
    let capture = fixture.standard();
    assert!(fixture.apply(capture).applied);
    assert_eq!(fixture.app.doc().id, id);
    assert_eq!(fixture.model(a).text.to_string(), "α😀 X\r\nnext\r\n");
    assert_eq!(fixture.model(b).text.to_string(), "猫 Y\r\n");
    fixture.unchanged_disks();
    assert_eq!(fixture.model(a).save_generation(), baseline);
    fixture.focus(b);
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(b).text.to_string(), B);
    fixture.unchanged_disks();
    fixture.focus(a);
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), "α😀 X\r\nnext\r\n");
    assert_eq!(fixture.model(b).text.to_string(), "猫 Y\r\n");
    fixture.unchanged_disks();
}

#[test]
fn receipt_capture_is_not_rebuilt_after_second_target_edit_undo_or_focus_aba() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    let capture = fixture.standard();
    let (entered, release) = fixture.gates(vec![GatePoint::BeforePrepare]);
    let token = fixture.start(capture.clone());
    assert_eq!(
        entered.recv_timeout(WAIT).unwrap(),
        GatePoint::BeforePrepare
    );
    fixture.model_mut(b).insert("later", false);
    fixture.model_mut(b).undo();
    fixture.focus(b);
    fixture.focus(a);
    assert_eq!(fixture.model(b).text.to_string(), B);
    release.send(()).unwrap();
    let completion = fixture.finish(token);
    assert!(!completion.applied);
    assert!(completion.reason.unwrap().contains("changed"));
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert!(
        fixture
            .model(a)
            .workspace_history(WorkspaceHistoryDirection::Undo)
            .is_none()
    );
    fixture.model_mut(b).redo();
    assert_eq!(fixture.model(b).text.to_string(), "later猫 B\r\n");
    fixture.unchanged_disks();
    assert!(fixture.app.start_workspace_transaction(capture).is_err());
}

#[test]
fn same_position_insert_order_and_hidden_untitled_targets_are_atomic_and_do_not_focus() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let active = fixture.app.doc().id;
    let mut hidden = Document::from_text("hidden😀\r\n");
    hidden.move_to(1, false);
    let hidden_id = hidden.id;
    fixture.app.hidden_documents.push(hidden);
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 0, 0, 0, "1"), edit(0, 0, 0, 0, "2")])),
        (hidden_id, json!([edit(0, 0, 0, 0, "猫\n")])),
    ]);
    assert!(fixture.apply(capture).applied);
    assert_eq!(fixture.app.doc().id, active);
    assert_eq!(fixture.model(a).text.to_string(), format!("12{A}"));
    assert_eq!(
        fixture.model(hidden_id).text.to_string(),
        "猫\r\nhidden😀\r\n"
    );
    assert_eq!(fixture.model(hidden_id).cursor, 4);
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(hidden_id).text.to_string(), "hidden😀\r\n");
    assert!(
        fixture
            .app
            .hidden_documents
            .iter()
            .any(|doc| doc.id == hidden_id)
    );
    fixture.unchanged_disks();
}

#[test]
fn second_target_bad_surrogate_eol_or_overlap_never_publishes_first_target() {
    for invalid in [
        json!([edit(0, 1, 0, 1, "x")]),
        json!([edit(0, 6, 0, 6, "x")]),
        json!([edit(0, 0, 0, 2, "x"), edit(0, 1, 0, 3, "y")]),
    ] {
        let mut fixture = Fixture::new();
        let a = fixture.a_id;
        let b = fixture.b_id;
        // Put an astral character in target B without changing the captured
        // proof afterwards. UTF16 index1 is the surrogate interior.
        fixture.model_mut(b).move_to(0, false);
        fixture.model_mut(b).insert("😀", false);
        let before = fixture.model(b).text.to_string();
        let epoch = fixture.model(a).text_epoch();
        let capture = fixture.capture(vec![(a, json!([edit(0, 4, 0, 5, "X")])), (b, invalid)]);
        assert!(!fixture.apply(capture).applied);
        assert_eq!(fixture.model(a).text.to_string(), A);
        assert_eq!(fixture.model(a).text_epoch(), epoch);
        assert_eq!(fixture.model(b).text.to_string(), before);
        fixture.unchanged_disks();
    }
}

#[test]
fn no_op_keeps_native_redo_and_all_model_epochs() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(a).insert("prior", false);
    fixture.model_mut(a).undo();
    let epoch = fixture.model(a).text_epoch();
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 4, 0, 5, "A")])),
        (b, json!([edit(0, 2, 0, 3, "B")])),
    ]);
    assert!(fixture.apply(capture).applied);
    assert_eq!(fixture.model(a).text_epoch(), epoch);
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), format!("prior{A}"));
    fixture.unchanged_disks();
}

#[test]
fn independent_peer_edit_blocks_whole_group_until_that_edit_is_undone() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    let capture = fixture.standard();
    assert!(fixture.apply(capture).applied);
    fixture.model_mut(b).insert("independent", false);
    let a_epoch = fixture.model(a).text_epoch();
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), "α😀 X\r\nnext\r\n");
    assert_eq!(fixture.model(a).text_epoch(), a_epoch);
    assert_eq!(fixture.model(b).text.to_string(), "independent猫 Y\r\n");
    fixture.model_mut(b).undo();
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(b).text.to_string(), B);
    // A new independent edit clears only B's group Redo. A's Redo must refuse
    // the complete cohort, rather than overwrite that independent B edit.
    fixture.model_mut(b).insert("new", false);
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(b).text.to_string(), "new猫 B\r\n");
    fixture.unchanged_disks();
}

#[test]
fn worker_reply_cannot_release_actual_capacity_until_join_and_owner_retirement_is_inert() {
    let mut fixture = Fixture::new();
    let capture = fixture.standard();
    let retired = capture.origin.retired.clone();
    let (entered, release) = fixture.gates(vec![GatePoint::AfterReply]);
    let token = fixture.start(capture.clone());
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    assert!(!fixture.app.poll_workspace_transactions());
    assert!(fixture.app.workspace_transaction_completion().is_none());
    assert!(fixture.app.start_workspace_transaction(capture).is_err());
    retired.store(true, Ordering::Release);
    release.send(()).unwrap();
    assert!(!fixture.finish(token).applied);
    fixture.unchanged_disks();
    assert_eq!(fixture.model(fixture.a_id).text.to_string(), A);
    assert_eq!(fixture.model(fixture.b_id).text.to_string(), B);
    let fresh = fixture.standard();
    assert!(fixture.apply(fresh).applied);
}

#[test]
fn late_second_commit_lease_refusal_keeps_first_text_epoch_and_redo() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(a).insert("prior", false);
    fixture.model_mut(a).undo();
    // Source history remains legal; live-view publication work is too large.
    fixture.model_mut(b).move_to(0, false);
    fixture.model_mut(b).insert(&"z".repeat(10_000), false);
    let before_b = fixture.model(b).text.to_string();
    fixture.model_mut(b).set_selections(
        (0..10_000)
            .map(|cursor| Selection {
                cursor,
                anchor: None,
                desired_column: None,
            })
            .collect(),
    );
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 4, 0, 5, "X")])),
        (
            b,
            Value::Array(
                (1..=8)
                    .map(|column| edit(0, column * 10, 0, column * 10, "Z"))
                    .collect(),
            ),
        ),
    ]);
    let epoch = fixture.model(a).text_epoch();
    assert!(!fixture.apply(capture).applied);
    assert_eq!(fixture.model(a).text_epoch(), epoch);
    assert_eq!(fixture.model(a).text.to_string(), A);
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), format!("prior{A}"));
    assert_eq!(fixture.model(b).text.to_string(), before_b);
    fixture.unchanged_disks();
}

#[test]
fn shared_split_views_reversed_selections_survive_group_history_and_document_identity() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    fixture
        .app
        .execute("workbench.action.splitEditor", Value::Null);
    let second = fixture.app.panes.last().unwrap().id;
    let first = fixture.app.panes.first().unwrap().id;
    fixture.model_mut(a).activate_view(first);
    fixture.model_mut(a).set_selections(vec![Selection {
        cursor: 2,
        anchor: Some(4),
        desired_column: None,
    }]);
    fixture.model_mut(a).activate_view(second);
    fixture.model_mut(a).set_selections(vec![Selection {
        cursor: 4,
        anchor: Some(2),
        desired_column: None,
    }]);
    let capture = fixture.capture(vec![(a, json!([edit(0, 0, 0, 0, "Q")]))]);
    assert!(fixture.apply(capture).applied);
    assert_eq!(fixture.model(a).view_state(Some(first)).cursor, 3);
    assert_eq!(fixture.model(a).view_state(Some(first)).anchor, Some(5));
    assert_eq!(fixture.model(a).view_state(Some(second)).cursor, 5);
    assert_eq!(fixture.model(a).view_state(Some(second)).anchor, Some(3));
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(a).id, a);
    assert_eq!(fixture.model(a).view_state(Some(first)).cursor, 2);
    assert_eq!(fixture.model(a).view_state(Some(second)).cursor, 4);
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), format!("Q{A}"));
    fixture.unchanged_disks();
}

#[test]
fn authorized_real_save_persists_old_snapshot_and_preserves_newer_group_undo_redo() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(a).move_to(0, false);
    fixture.model_mut(a).insert("saved ", false);
    let saved_text = fixture.model(a).text.to_string();
    let generation = fixture.model(a).save_generation();
    let (worker, entered, release) = SaveWorker::fixture_gated(vec![SaveGate::BeforeCommit]);
    fixture.app.replace_save_worker_fixture(worker);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    save_gate(&mut fixture.app, &entered, SaveGate::BeforeCommit);
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 0, 0, 0, "new ")])),
        (b, json!([edit(0, 2, 0, 3, "Y")])),
    ]);
    assert!(fixture.apply(capture).applied);
    release.send(()).unwrap();
    until(&mut fixture.app, "authorized save receipt", |app| {
        !app.saves_pending()
    });
    assert_eq!(std::fs::read_to_string(&fixture.a).unwrap(), saved_text);
    assert_eq!(
        fixture.model(a).text.to_string(),
        format!("new {saved_text}")
    );
    assert!(fixture.model(a).dirty());
    assert_eq!(fixture.model(a).save_generation(), generation + 1);
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), saved_text);
    assert_eq!(fixture.model(b).text.to_string(), B);
    assert!(!fixture.model(a).dirty());
    fixture.app.execute("redo", Value::Null);
    assert_eq!(
        fixture.model(a).text.to_string(),
        format!("new {saved_text}")
    );
    assert_eq!(fixture.model(b).text.to_string(), "猫 Y\r\n");
    assert!(fixture.model(a).dirty());
    assert_eq!(std::fs::read_to_string(&fixture.a).unwrap(), saved_text);
    assert_eq!(std::fs::read_to_string(&fixture.b).unwrap(), B);
}

#[test]
fn real_unapproved_save_snapshot_is_rejected_after_group_publication() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(a).insert("dirty ", false);
    let (worker, entered, release) = SaveWorker::fixture_gated(vec![SaveGate::BeforePrepared]);
    fixture.app.replace_save_worker_fixture(worker);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    save_gate(&mut fixture.app, &entered, SaveGate::BeforePrepared);
    let generation = fixture.model(a).save_generation();
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 0, 0, 0, "new ")])),
        (b, json!([edit(0, 2, 0, 3, "Y")])),
    ]);
    assert!(fixture.apply(capture).applied);
    release.send(()).unwrap();
    until(&mut fixture.app, "rejected preauthorization save", |app| {
        !app.saves_pending()
    });
    fixture.unchanged_disks();
    assert_eq!(fixture.model(a).save_generation(), generation);
    assert!(fixture.model(a).dirty());
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), format!("dirty {A}"));
    assert_eq!(fixture.model(b).text.to_string(), B);
    fixture.unchanged_disks();
}

#[test]
fn captured_settings_profile_and_workspace_aba_cannot_be_reauthorized_at_dispatch() {
    let mut fixture = Fixture::new();
    let capture = fixture.standard();
    let old = fixture.app.settings.clone();
    let (entered, release) = fixture.gates(vec![GatePoint::AfterReply]);
    let token = fixture.start(capture.clone());
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    fixture.app.settings = crate::settings::Settings::default();
    release.send(()).unwrap();
    assert!(!fixture.finish(token).applied);
    fixture.app.settings = old;
    // Restoring the original Arc does not revive a capture if native profile
    // generation advanced; private root integration must always advance it.
    let mut changed = CaptureContext::new(
        fixture.app.settings.extension_layers().clone(),
        fixture
            .app
            .settings_profile_generation()
            .checked_add(1)
            .unwrap(),
        fixture.app.workspace.root.clone(),
    )
    .unwrap();
    changed.workspace = fixture.app.workspace.root.join("different");
    let mirrors = fixture.mirrors();
    let args = json!({"entries":[{"document":fixture.a_id,"version":mirrors.iter().find(|m|m.document==fixture.a_id).unwrap().version,"uri":crate::lsp::file_uri(&fixture.a).unwrap(),"edits":[edit(0,4,0,5,"X")]}],"metadata":{"isRefactoring":false}});
    let origin = Origin::new(
        1,
        "context-test",
        "fixture.extension",
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let capture = capture_workspace_transaction(
        &fixture.app.documents,
        &fixture.app.hidden_documents,
        &mirrors,
        origin,
        &args,
        changed,
        Instant::now(),
    )
    .unwrap();
    assert!(fixture.app.start_workspace_transaction(capture).is_err());
    fixture.unchanged_disks();
}

#[test]
fn grouped_restore_budget_refusal_preserves_other_retained_work_and_all_history_heads() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(b).move_to(0, false);
    fixture
        .model_mut(b)
        .insert(&"b".repeat(2 * 1024 * 1024), false);
    let before_b = fixture.model(b).text.to_string();
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 4, 0, 5, "X")])),
        (b, json!([edit(0, 0, 0, 2 * 1024 * 1024, "")])),
    ]);
    assert!(fixture.apply(capture).applied);
    let hidden = Document::from_text(&"h".repeat(3 * 1024 * 1024));
    let hidden_id = hidden.id;
    fixture.app.hidden_documents.push(hidden);
    let epoch = fixture.model(a).text_epoch();
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), "α😀 X\r\nnext\r\n");
    assert_eq!(fixture.model(a).text_epoch(), epoch);
    assert_eq!(fixture.model(b).text.to_string(), B);
    assert_eq!(fixture.model(hidden_id).text.len_bytes(), 3 * 1024 * 1024);
    fixture
        .app
        .hidden_documents
        .retain(|doc| doc.id != hidden_id);
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(b).text.to_string(), before_b);
    fixture.unchanged_disks();
}

#[test]
fn terminal_receipt_retains_logical_capacity_and_exact_origin_until_transport_admission() {
    let mut fixture = Fixture::new();
    let capture = fixture.standard();
    let token = fixture.start(capture.clone());
    let deadline = Instant::now() + WAIT;
    loop {
        fixture.app.poll_workspace_transactions();
        if fixture.app.workspace_transaction_completion().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let receipt = fixture.app.workspace_transaction_completion().unwrap();
    assert_eq!(receipt.origin.session, 1);
    assert_eq!(receipt.origin.owner, "fixture.extension");
    assert_eq!(receipt.origin.request, "workspace-test");
    assert!(receipt.applied);
    assert!(fixture.app.workspace_transactions_pending());
    assert!(fixture.app.start_workspace_transaction(capture).is_err());
    assert!(
        fixture
            .app
            .take_workspace_transaction_completion(token + 1)
            .is_none()
    );
    assert!(fixture.app.workspace_transactions_pending());
    assert!(
        fixture
            .app
            .take_workspace_transaction_completion(token)
            .unwrap()
            .applied
    );
    assert!(!fixture.app.workspace_transactions_pending());
}

#[test]
fn accepted_save_baseline_retires_prequeue_workspace_proof_without_losing_peer_redo() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(a).insert("save ", false);
    fixture.model_mut(b).insert("peer ", false);
    fixture.model_mut(b).undo();
    let capture = fixture.capture(vec![
        (a, json!([edit(0, 0, 0, 0, "new ")])),
        (b, json!([edit(0, 2, 0, 3, "Y")])),
    ]);
    let generation = fixture.model(a).save_generation();
    let saved_text = fixture.model(a).text.to_string();
    let (worker, save_entered, save_release) =
        SaveWorker::fixture_gated(vec![SaveGate::BeforeFinish]);
    fixture.app.replace_save_worker_fixture(worker);
    fixture
        .app
        .execute("workbench.action.files.save", Value::Null);
    save_gate(&mut fixture.app, &save_entered, SaveGate::BeforeFinish);
    let (entered, release) = fixture.gates(vec![GatePoint::AfterReply]);
    let token = fixture.start(capture);
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    assert_eq!(std::fs::read_to_string(&fixture.a).unwrap(), saved_text);
    assert_eq!(fixture.model(a).save_generation(), generation);
    save_release.send(()).unwrap();
    until(
        &mut fixture.app,
        "terminal save baseline publication",
        |app| !app.saves_pending(),
    );
    assert_eq!(fixture.model(a).save_generation(), generation + 1);
    release.send(()).unwrap();
    assert!(!fixture.finish(token).applied);
    assert_eq!(fixture.model(a).text.to_string(), saved_text);
    assert!(!fixture.model(a).dirty());
    assert_eq!(fixture.model(b).text.to_string(), B);
    fixture.model_mut(b).redo();
    assert_eq!(fixture.model(b).text.to_string(), format!("peer {B}"));
    assert_eq!(std::fs::read_to_string(&fixture.b).unwrap(), B);
}

#[test]
fn exact_cancellation_retains_actual_worker_until_join_without_publishing_text() {
    let mut fixture = Fixture::new();
    let capture = fixture.standard();
    let (entered, release) = fixture.gates(vec![GatePoint::BeforePrepare]);
    let token = fixture.start(capture);
    assert_eq!(
        entered.recv_timeout(WAIT).unwrap(),
        GatePoint::BeforePrepare
    );
    fixture.app.cancel_workspace_transaction(token + 1);
    assert!(
        !fixture
            .app
            .workspace_transactions
            .worker
            .active
            .as_ref()
            .unwrap()
            .cancel
            .load(Ordering::Acquire)
    );
    fixture.app.cancel_workspace_transaction(token);
    assert!(fixture.app.workspace_transactions_pending());
    assert!(fixture.app.workspace_transaction_completion().is_none());
    release.send(()).unwrap();
    assert!(!fixture.finish(token).applied);
    assert!(!fixture.app.workspace_transactions_pending());
    assert_eq!(fixture.model(fixture.a_id).text.to_string(), A);
    assert_eq!(fixture.model(fixture.b_id).text.to_string(), B);
    fixture.unchanged_disks();
}

// Preparation has already sent its successful result at this actual gate.
#[test]
fn exact_cancel_after_real_reply_cannot_publish_or_destroy_prior_redo() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    fixture.model_mut(a).insert("prior ", false);
    fixture.model_mut(a).undo();
    let a_source = fixture.model(a).capture_workspace_source().unwrap();
    let b_source = fixture.model(b).capture_workspace_source().unwrap();
    let a_selection = fixture.model(a).selections();
    let b_selection = fixture.model(b).selections();
    let active = fixture.app.doc().id;
    let capture = fixture.standard();
    let (entered, release) = fixture.gates(vec![GatePoint::AfterReply]);
    let token = fixture.start(capture.clone());
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    fixture.app.cancel_workspace_transaction(token + 1);
    assert!(
        !fixture
            .app
            .workspace_transactions
            .worker
            .active
            .as_ref()
            .unwrap()
            .cancel
            .load(Ordering::Acquire)
    );
    fixture.app.cancel_workspace_transaction(token);
    assert!(
        fixture
            .app
            .workspace_transactions
            .worker
            .active
            .as_ref()
            .unwrap()
            .cancel
            .load(Ordering::Acquire)
    );
    assert!(
        !fixture.app.poll_workspace_transactions(),
        "A sent result has not joined"
    );
    assert!(fixture.app.workspace_transactions_pending());
    assert!(fixture.app.workspace_transaction_completion().is_none());
    assert!(fixture.app.start_workspace_transaction(capture).is_err());
    release.send(()).unwrap();
    let completion = fixture.finish(token);
    assert!(
        !completion.applied,
        "An exact canceled successful reply must remain inert"
    );
    assert!(
        completion
            .reason
            .unwrap()
            .to_ascii_lowercase()
            .contains("cancel")
    );
    assert!(!fixture.app.workspace_transactions_pending());
    assert!(a_source.current(fixture.model(a)));
    assert!(b_source.current(fixture.model(b)));
    assert_eq!(fixture.app.doc().id, active);
    assert_eq!(fixture.model(a).selections(), a_selection);
    assert_eq!(fixture.model(b).selections(), b_selection);
    fixture.unchanged_disks();
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), format!("prior {A}"));
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    let fresh = fixture.standard();
    assert!(
        fixture.apply(fresh).applied,
        "Joined cancellation must allow fresh work"
    );
    assert_eq!(fixture.model(a).text.to_string(), "α😀 X\r\nnext\r\n");
    assert_eq!(fixture.model(b).text.to_string(), "猫 Y\r\n");
    fixture.unchanged_disks();
}

fn fill_workspace_inventory_to_129(fixture: &mut Fixture) -> Vec<WorkspaceSource> {
    assert_eq!(
        fixture.app.documents.len() + fixture.app.hidden_documents.len(),
        2
    );
    for ordinal in 0..127 {
        // Independent retained models with real native Undo state to preserve.
        let mut document = Document::from_text("unrelated 猫🙂\r\n");
        document.insert(&format!("{ordinal} "), false);
        fixture.app.hidden_documents.push(document);
    }
    assert_eq!(
        fixture.app.documents.len() + fixture.app.hidden_documents.len(),
        129
    );
    fixture
        .app
        .hidden_documents
        .iter()
        .map(|document| document.capture_workspace_source().unwrap())
        .collect()
}
fn assert_workspace_unrelated_sources(fixture: &Fixture, sources: &[WorkspaceSource]) {
    for source in sources {
        assert!(
            source.current(fixture.model(source.document())),
            "Refusal/history preserves unrelated model {}, baseline and Undo/Redo heads",
            source.document()
        );
    }
}

#[test]
fn publication_refuses_late_129_model_inventory_after_actual_reply_atomically() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    let a_source = fixture.model(a).capture_workspace_source().unwrap();
    let b_source = fixture.model(b).capture_workspace_source().unwrap();
    let capture = fixture.standard();
    let (entered, release) = fixture.gates(vec![GatePoint::AfterReply]);
    let token = fixture.start(capture);
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    let unrelated = fill_workspace_inventory_to_129(&mut fixture);
    let active = fixture.app.doc().id;
    let selections = fixture.app.doc().selections();
    release.send(()).unwrap();
    let completion = fixture.finish(token);
    assert!(
        !completion.applied,
        "Admission bounds must be rechecked at publication"
    );
    assert!(
        completion.reason.unwrap().contains("128"),
        "Exact inventory refusal required"
    );
    assert!(a_source.current(fixture.model(a)));
    assert!(b_source.current(fixture.model(b)));
    assert_eq!(fixture.app.doc().id, active);
    assert_eq!(fixture.app.doc().selections(), selections);
    assert_workspace_unrelated_sources(&fixture, &unrelated);
    fixture.unchanged_disks();
    // Returning to 128 preserves peers and permits new work, not poisoned state.
    let removed = fixture.app.hidden_documents.pop().unwrap();
    let fresh = fixture.standard();
    assert!(fixture.apply(fresh).applied);
    assert_workspace_unrelated_sources(&fixture, &unrelated[..unrelated.len() - 1]);
    assert_eq!(removed.text.to_string(), "126 unrelated 猫🙂\r\n");
    fixture.unchanged_disks();
}

#[test]
fn grouped_undo_and_redo_refuse_129_models_without_consuming_any_peer_head() {
    let mut fixture = Fixture::new();
    let a = fixture.a_id;
    let b = fixture.b_id;
    let capture = fixture.standard();
    assert!(fixture.apply(capture).applied);
    let unrelated = fill_workspace_inventory_to_129(&mut fixture);
    let a_after = fixture.model(a).capture_workspace_source().unwrap();
    let b_after = fixture.model(b).capture_workspace_source().unwrap();
    let undo_group = fixture
        .model(a)
        .workspace_history(WorkspaceHistoryDirection::Undo)
        .unwrap();
    let active = fixture.app.doc().id;
    let selection = fixture.app.doc().selections();
    fixture.app.execute("undo", Value::Null);
    assert!(
        fixture.app.message.contains("128"),
        "{}",
        fixture.app.message
    );
    assert!(a_after.current(fixture.model(a)));
    assert!(b_after.current(fixture.model(b)));
    assert!(Arc::ptr_eq(
        &undo_group,
        &fixture
            .model(a)
            .workspace_history(WorkspaceHistoryDirection::Undo)
            .unwrap()
    ));
    assert_eq!(fixture.app.doc().id, active);
    assert_eq!(fixture.app.doc().selections(), selection);
    assert_workspace_unrelated_sources(&fixture, &unrelated);
    fixture.unchanged_disks();
    let removed = fixture.app.hidden_documents.pop().unwrap();
    fixture.app.execute("undo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), A);
    assert_eq!(fixture.model(b).text.to_string(), B);
    let a_before = fixture.model(a).capture_workspace_source().unwrap();
    let b_before = fixture.model(b).capture_workspace_source().unwrap();
    let redo_group = fixture
        .model(a)
        .workspace_history(WorkspaceHistoryDirection::Redo)
        .unwrap();
    let undone_selection = fixture.app.doc().selections();
    fixture.app.hidden_documents.push(removed);
    fixture.app.execute("redo", Value::Null);
    assert!(
        fixture.app.message.contains("128"),
        "{}",
        fixture.app.message
    );
    assert!(a_before.current(fixture.model(a)));
    assert!(b_before.current(fixture.model(b)));
    assert!(Arc::ptr_eq(
        &redo_group,
        &fixture
            .model(a)
            .workspace_history(WorkspaceHistoryDirection::Redo)
            .unwrap()
    ));
    assert_eq!(fixture.app.doc().id, active);
    assert_eq!(fixture.app.doc().selections(), undone_selection);
    assert_workspace_unrelated_sources(&fixture, &unrelated);
    fixture.unchanged_disks();
    fixture.app.hidden_documents.pop();
    fixture.app.execute("redo", Value::Null);
    assert_eq!(fixture.model(a).text.to_string(), "α😀 X\r\nnext\r\n");
    assert_eq!(fixture.model(b).text.to_string(), "猫 Y\r\n");
    assert_workspace_unrelated_sources(&fixture, &unrelated[..unrelated.len() - 1]);
    fixture.unchanged_disks();
}
