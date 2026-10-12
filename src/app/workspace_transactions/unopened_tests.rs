use super::*;
use serde_json::json;

const ORIGINAL: &str = "α😀 A\r\n";
const UNOPENED: &str = "猫 B\r\n";
const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    _directory: tempfile::TempDir,
    app: App,
    retained: PathBuf,
    unopened: PathBuf,
    id: u64,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let retained = root.join("retained.txt");
        let unopened = root.join("unopened.txt");
        std::fs::write(&retained, ORIGINAL).unwrap();
        std::fs::write(&unopened, UNOPENED).unwrap();
        let mut app = App::new(root, crate::keys::Profile::Linux);
        app.open(&retained).unwrap();
        let deadline = Instant::now() + WAIT;
        while app.doc().path.as_ref() != Some(&retained) {
            app.poll();
            assert!(Instant::now() < deadline, "Open failed: {}", app.message);
            std::thread::yield_now();
        }
        let id = app.doc().id;
        Self {
            _directory: directory,
            app,
            retained,
            unopened,
            id,
        }
    }
    fn inventory(&self, version: u64) -> Vec<MirrorSnapshot> {
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
                resource_epoch: doc.resource_epoch(),
                language_epoch: doc.language_epoch(),
                version,
            })
            .collect()
    }
    fn capture(&self, entries: Value) -> Arc<Capture> {
        self.app
            .capture_workspace_transaction(
                Origin::new(
                    1,
                    "unopened",
                    "fixture.extension",
                    Arc::new(AtomicBool::new(false)),
                )
                .unwrap(),
                &json!({"entries": entries, "metadata":{"isRefactoring":false}}),
                &self.inventory(1),
                Instant::now(),
            )
            .unwrap()
    }
    fn entries(&self) -> Value {
        json!([
            {"uri":crate::lsp::file_uri(&self.retained).unwrap(),"edits":[edit(4,5,"X")]},
            {"uri":crate::lsp::file_uri(&self.unopened).unwrap(),"edits":[edit(2,3,"Y")]}
        ])
    }
    fn finish(&mut self, token: u64) -> Completion {
        let deadline = Instant::now() + WAIT;
        loop {
            self.app.poll_workspace_transactions();
            if let Some(done) = self.app.take_workspace_transaction_completion(token) {
                return done;
            }
            assert!(Instant::now() < deadline, "Worker failed to join");
            std::thread::yield_now();
        }
    }
    fn apply(&mut self, capture: Arc<Capture>) -> Completion {
        let token = self.app.start_workspace_transaction(capture).unwrap();
        self.finish(token)
    }
    fn hidden(&self) -> &Document {
        self.app
            .hidden_documents
            .iter()
            .find(|doc| doc.path.as_ref() == Some(&self.unopened))
            .unwrap()
    }
    fn unchanged_disks(&self) {
        assert_eq!(std::fs::read_to_string(&self.retained).unwrap(), ORIGINAL);
        assert_eq!(std::fs::read_to_string(&self.unopened).unwrap(), UNOPENED);
    }
}
fn edit(start: usize, end: usize, text: &str) -> Value {
    json!({"range":{"start":{"line":0,"character":start},"end":{"line":0,"character":end}},"newText":text})
}

#[test]
fn unopened_file_is_a_real_hidden_dirty_model_in_atomic_grouped_history() {
    let mut fixture = Fixture::new();
    let capture = fixture.capture(fixture.entries());
    assert!(
        fixture.app.hidden_documents.is_empty(),
        "Capture must perform no disk loading/admission"
    );
    let result = fixture.apply(capture);
    assert!(result.applied, "{:?}", result.reason);
    assert_eq!(fixture.app.doc().id, fixture.id);
    assert_eq!(fixture.app.doc().text.to_string(), "α😀 X\r\n");
    assert_eq!(fixture.hidden().text.to_string(), "猫 Y\r\n");
    assert_ne!(fixture.hidden().id, fixture.id);
    assert_ne!(fixture.hidden().revision, fixture.hidden().saved_revision);
    let hidden_id = fixture.hidden().id;
    fixture.unchanged_disks();
    fixture.app.undo_editor();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
    assert_eq!(fixture.hidden().text.to_string(), UNOPENED);
    assert_eq!(fixture.hidden().id, hidden_id);
    fixture.app.redo_editor();
    assert_eq!(fixture.app.doc().text.to_string(), "α😀 X\r\n");
    assert_eq!(fixture.hidden().text.to_string(), "猫 Y\r\n");
    assert_eq!(fixture.hidden().id, hidden_id);
    fixture.unchanged_disks();
}

#[test]
fn late_retained_source_change_refuses_new_model_and_keeps_native_history() {
    let mut fixture = Fixture::new();
    let capture = fixture.capture(fixture.entries());
    let (worker, entered, release) = fixture_worker(vec![GatePoint::AfterReply]);
    fixture.app.workspace_transactions.worker = worker;
    let token = fixture.app.start_workspace_transaction(capture).unwrap();
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    fixture.app.doc_mut().insert("prior ", false);
    let source = fixture.app.doc().capture_workspace_source().unwrap();
    release.send(()).unwrap();
    assert!(!fixture.finish(token).applied);
    assert!(source.current(fixture.app.doc()));
    assert!(fixture.app.hidden_documents.is_empty());
    fixture.app.undo_editor();
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
    fixture.unchanged_disks();
}

#[test]
fn unopened_cancellation_keeps_actual_slot_until_worker_reply_and_exit_join() {
    let mut fixture = Fixture::new();
    let capture = fixture.capture(fixture.entries());
    let (worker, entered, release) =
        fixture_worker(vec![GatePoint::BeforePrepare, GatePoint::AfterReply]);
    fixture.app.workspace_transactions.worker = worker;
    let token = fixture
        .app
        .start_workspace_transaction(capture.clone())
        .unwrap();
    assert_eq!(
        entered.recv_timeout(WAIT).unwrap(),
        GatePoint::BeforePrepare
    );
    fixture.app.cancel_workspace_transaction(token);
    assert!(
        fixture
            .app
            .start_workspace_transaction(capture.clone())
            .is_err()
    );
    release.send(()).unwrap();
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    assert!(!fixture.app.poll_workspace_transactions());
    assert!(fixture.app.workspace_transactions_pending());
    assert!(fixture.app.hidden_documents.is_empty());
    release.send(()).unwrap();
    assert!(!fixture.finish(token).applied);
    assert!(fixture.app.workspace_transactions.worker.active.is_none());
    assert!(fixture.apply(fixture.capture(fixture.entries())).applied);
    fixture.unchanged_disks();
}

#[cfg(unix)]
#[test]
fn canonical_alias_uses_original_dirty_hidden_identity_and_refuses_duplicate_groups() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new();
    let mut hidden = Document::open_existing(&fixture.unopened).unwrap();
    hidden.insert("dirty ", false);
    let hidden_id = hidden.id;
    fixture.app.hidden_documents.push(hidden);
    let alias = fixture.unopened.with_file_name("alias.txt");
    symlink(&fixture.unopened, &alias).unwrap();
    let entry = json!({"uri":crate::lsp::file_uri(&alias).unwrap(),"edits":[edit(8,9,"Y")]});
    let done = fixture.apply(fixture.capture(json!([entry.clone()])));
    assert!(done.applied, "{:?}", done.reason);
    assert_eq!(fixture.hidden().id, hidden_id);
    assert_eq!(fixture.hidden().text.to_string(), "dirty 猫 Y\r\n");
    assert_eq!(fixture.app.hidden_documents.len(), 1);
    let source = fixture.hidden().capture_workspace_source().unwrap();
    let duplicate = fixture.capture(json!([entry, {
        "uri":crate::lsp::file_uri(&fixture.unopened).unwrap(),"edits":[edit(8,9,"Z")]
    }]));
    assert!(!fixture.apply(duplicate).applied);
    assert!(source.current(fixture.hidden()));
    assert_eq!(fixture.app.hidden_documents.len(), 1);
    fixture.unchanged_disks();
}

#[test]
fn bad_unopened_targets_refuse_every_target_before_any_publication() {
    for kind in ["missing", "directory", "binary", "utf8", "oversize"] {
        let mut fixture = Fixture::new();
        match kind {
            "missing" => std::fs::remove_file(&fixture.unopened).unwrap(),
            "directory" => {
                std::fs::remove_file(&fixture.unopened).unwrap();
                std::fs::create_dir(&fixture.unopened).unwrap();
            }
            "binary" => std::fs::write(&fixture.unopened, b"bad\0file").unwrap(),
            "utf8" => std::fs::write(&fixture.unopened, [0xff, 0xfe]).unwrap(),
            "oversize" => std::fs::write(&fixture.unopened, vec![b'x'; MAX_DOCUMENTS + 1]).unwrap(),
            _ => unreachable!(),
        }
        let source = fixture.app.doc().capture_workspace_source().unwrap();
        let capture = fixture.capture(fixture.entries());
        assert!(!fixture.apply(capture).applied, "{kind}");
        assert!(source.current(fixture.app.doc()), "{kind}");
        assert!(fixture.app.hidden_documents.is_empty(), "{kind}");
        assert_eq!(
            std::fs::read_to_string(&fixture.retained).unwrap(),
            ORIGINAL
        );
    }
}

#[test]
fn native_unknown_wire_versions_are_never_admitted_for_public_extension_origins() {
    let fixture = Fixture::new();
    let args = json!({"entries":[{"uri":crate::lsp::file_uri(&fixture.retained).unwrap(),"edits":[edit(4,5,"X")]}],
        "metadata":{"isRefactoring":false}});
    let lifetime = Arc::new(AtomicBool::new(false));
    let native = Origin::native("rename", lifetime.clone()).unwrap();
    assert!(native.is_native());
    assert!(
        fixture
            .app
            .capture_workspace_transaction(
                native.clone(),
                &args,
                &fixture.inventory(0),
                Instant::now()
            )
            .is_ok()
    );
    let public = Origin::new(1, "extension", "fixture.extension", lifetime.clone()).unwrap();
    assert!(!public.is_native());
    assert!(
        fixture
            .app
            .capture_workspace_transaction(public, &args, &fixture.inventory(0), Instant::now())
            .is_err()
    );
    lifetime.store(true, Ordering::Release);
    assert!(!native.alive());
}

#[test]
fn optional_identity_and_version_fields_cannot_launder_unopened_or_stale_targets() {
    let fixture = Fixture::new();
    for (uri, extra) in [
        (&fixture.unopened, json!({"document":fixture.id})),
        (&fixture.unopened, json!({"version":1})),
        (&fixture.retained, json!({"document":fixture.id + 999})),
        (&fixture.retained, json!({"version":2})),
        (&fixture.retained, json!({"version":0})),
    ] {
        let mut entry = json!({"uri":crate::lsp::file_uri(uri).unwrap(),"edits":[edit(0,0,"X")]});
        entry
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let args = json!({"entries":[entry],"metadata":{"isRefactoring":false}});
        let origin = Origin::new(
            1,
            "bounds",
            "fixture.extension",
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(
            fixture
                .app
                .capture_workspace_transaction(origin, &args, &fixture.inventory(1), Instant::now())
                .is_err()
        );
    }
}

#[test]
fn non_boolean_refactoring_setting_is_strictly_disabled_while_missing_defaults_enabled() {
    let fixture = Fixture::new();
    for value in [
        Value::Null,
        json!(0),
        json!("true"),
        json!({}),
        Value::Bool(false),
    ] {
        let mut layer = Map::new();
        layer.insert("files.refactoring.autoSave".into(), value);
        let context =
            CaptureContext::new(Arc::new(vec![layer]), 0, fixture.app.workspace.root.clone())
                .unwrap();
        assert!(!context.refactoring_auto_save);
    }
    let default =
        CaptureContext::new(Arc::new(Vec::new()), 0, fixture.app.workspace.root.clone()).unwrap();
    assert!(default.refactoring_auto_save);
}

#[test]
fn unopened_result_aggregate_budget_refuses_hidden_admission_and_retained_prefix() {
    let mut fixture = Fixture::new();
    std::fs::write(
        &fixture.unopened,
        vec![b'x'; MAX_DOCUMENTS - ORIGINAL.len()],
    )
    .unwrap();
    let source = fixture.app.doc().capture_workspace_source().unwrap();
    let capture = fixture.capture(json!([
        {"uri":crate::lsp::file_uri(&fixture.retained).unwrap(),"edits":[edit(0,0,"extra")]},
        {"uri":crate::lsp::file_uri(&fixture.unopened).unwrap(),"edits":[edit(0,1,"y")]}
    ]));
    let completion = fixture.apply(capture);
    assert!(!completion.applied);
    assert!(completion.reason.unwrap().contains("4 MiB"));
    assert!(source.current(fixture.app.doc()));
    assert!(fixture.app.hidden_documents.is_empty());
    assert_eq!(
        std::fs::read_to_string(&fixture.retained).unwrap(),
        ORIGINAL
    );
}

#[test]
fn reopening_target_while_loader_has_replied_refuses_duplicate_model_publication() {
    let mut fixture = Fixture::new();
    let capture = fixture.capture(fixture.entries());
    let (worker, entered, release) = fixture_worker(vec![GatePoint::AfterReply]);
    fixture.app.workspace_transactions.worker = worker;
    let token = fixture.app.start_workspace_transaction(capture).unwrap();
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    let mut actual = Document::open_existing(&fixture.unopened).unwrap();
    actual.insert("independent ", false);
    let source = actual.capture_workspace_source().unwrap();
    fixture.app.hidden_documents.push(actual);
    release.send(()).unwrap();
    let completion = fixture.finish(token);
    assert!(!completion.applied);
    assert!(completion.reason.unwrap().contains("inventory changed"));
    assert_eq!(fixture.app.hidden_documents.len(), 1);
    assert!(source.current(fixture.hidden()));
    assert_eq!(fixture.app.doc().text.to_string(), ORIGINAL);
    fixture.unchanged_disks();
}

fn receipt_capture(fixture: &Fixture, entries: Value, flag: Value) -> Result<Arc<Capture>> {
    fixture.app.capture_workspace_transaction(
        Origin::new(
            1,
            "target-receipts",
            "fixture.extension",
            Arc::new(AtomicBool::new(false)),
        )?,
        &json!({"entries":entries,"metadata":{"isRefactoring":false},"targetReceipts":flag}),
        &fixture.inventory(1),
        Instant::now(),
    )
}

#[cfg(unix)]
#[test]
fn trusted_alias_receipt_maps_original_uri_to_dirty_retained_canonical_identity() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new();
    let mut hidden = Document::open_existing(&fixture.unopened).unwrap();
    hidden.insert("dirty ", false);
    let id = hidden.id;
    fixture.app.hidden_documents.push(hidden);
    let alias = fixture.unopened.with_file_name("dirty-alias.txt");
    symlink(&fixture.unopened, &alias).unwrap();
    let requested = crate::lsp::file_uri(&alias).unwrap();
    let capture = receipt_capture(
        &fixture,
        json!([
            {"uri":requested,"edits":[edit(8,9,"Y")]}
        ]),
        Value::Bool(true),
    )
    .unwrap();
    let completion = fixture.apply(capture);
    assert!(completion.applied, "{:?}", completion.reason);
    assert!(completion.target_receipts);
    assert_eq!(completion.targets.len(), 1);
    assert_eq!(completion.targets[0].uri, requested);
    assert_eq!(completion.targets[0].document, id);
    assert_eq!(fixture.hidden().id, id);
    assert_eq!(fixture.hidden().path.as_ref(), Some(&fixture.unopened));
    assert_eq!(fixture.hidden().text.to_string(), "dirty 猫 Y\r\n");
    assert_eq!(fixture.app.hidden_documents.len(), 1);
    fixture.unchanged_disks();
}

#[cfg(unix)]
#[test]
fn actual_loaded_alias_receipt_keeps_request_uri_and_real_hidden_model_id() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new();
    let alias = fixture.unopened.with_file_name("new-alias.txt");
    symlink(&fixture.unopened, &alias).unwrap();
    let requested = crate::lsp::file_uri(&alias).unwrap();
    let capture = receipt_capture(
        &fixture,
        json!([
            {"uri":requested,"edits":[edit(2,3,"Y")]}
        ]),
        Value::Bool(true),
    )
    .unwrap();
    let completion = fixture.apply(capture);
    assert!(completion.applied, "{:?}", completion.reason);
    assert!(completion.target_receipts);
    assert_eq!(completion.targets.len(), 1);
    assert_eq!(completion.targets[0].uri, requested);
    assert_eq!(completion.targets[0].document, fixture.hidden().id);
    assert_ne!(completion.targets[0].document, fixture.id);
    assert_eq!(fixture.hidden().path.as_ref(), Some(&fixture.unopened));
    assert_eq!(fixture.hidden().text.to_string(), "猫 Y\r\n");
    fixture.unchanged_disks();
}

#[test]
fn receipts_include_noop_loaded_participants_without_inventing_history_or_versions() {
    let mut fixture = Fixture::new();
    let requested = crate::lsp::file_uri(&fixture.unopened).unwrap();
    let capture = receipt_capture(
        &fixture,
        json!([
            {"uri":requested,"edits":[edit(2,3,"B")]}
        ]),
        Value::Bool(true),
    )
    .unwrap();
    let completion = fixture.apply(capture);
    assert!(completion.applied, "{:?}", completion.reason);
    assert!(completion.target_receipts);
    assert_eq!(completion.targets.len(), 1);
    assert_eq!(completion.targets[0].document, fixture.hidden().id);
    assert_eq!(completion.targets[0].uri, requested);
    assert_eq!(fixture.hidden().revision, fixture.hidden().saved_revision);
    assert!(
        fixture
            .hidden()
            .workspace_history(WorkspaceHistoryDirection::Undo)
            .is_none()
    );
    let serialized = serde_json::to_value(&completion.targets).unwrap();
    assert_eq!(
        serialized,
        json!([{"uri":requested,"document":fixture.hidden().id}])
    );
    fixture.unchanged_disks();
}

#[test]
fn failed_joined_transaction_has_no_identity_receipts_and_negotiation_flag_is_boolean() {
    let mut fixture = Fixture::new();
    assert!(receipt_capture(&fixture, fixture.entries(), json!("true")).is_err());
    let unnegotiated = fixture.capture(fixture.entries());
    assert!(!unnegotiated.target_receipts);
    let capture = receipt_capture(&fixture, fixture.entries(), Value::Bool(true)).unwrap();
    let (worker, entered, release) = fixture_worker(vec![GatePoint::AfterReply]);
    fixture.app.workspace_transactions.worker = worker;
    let token = fixture.app.start_workspace_transaction(capture).unwrap();
    assert_eq!(entered.recv_timeout(WAIT).unwrap(), GatePoint::AfterReply);
    fixture.app.cancel_workspace_transaction(token);
    assert!(!fixture.app.poll_workspace_transactions());
    assert!(fixture.app.workspace_transactions_pending());
    release.send(()).unwrap();
    let completion = fixture.finish(token);
    assert!(!completion.applied);
    assert!(completion.targets.is_empty());
    assert!(fixture.app.workspace_transactions.worker.active.is_none());
    assert!(fixture.app.hidden_documents.is_empty());
    fixture.unchanged_disks();
}
