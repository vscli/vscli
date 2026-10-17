use super::*;
use crate::extensions::providers::MirrorSnapshot;
use serde_json::json;

const A: &str = "A 猫🙂\r\n";
const B: &str = "B α🙂\r\n";
const C: &str = "C β🙂\r\n";
struct Fixture {
    _dir: tempfile::TempDir,
    app: App,
    owner: Arc<AtomicBool>,
    a: PathBuf,
    b: PathBuf,
    c: PathBuf,
    first: u64,
    hidden: u64,
    unrelated: u64,
}
impl Fixture {
    fn new(policy: Option<Value>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        let c = root.join("c.txt");
        for (path, text) in [(&a, A), (&b, B), (&c, C)] {
            std::fs::write(path, text).unwrap();
        }
        let config = root.join("settings.json");
        let mut settings = json!({"vscli.languageServer.enabled":false,"files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}});
        if let Some(policy) = policy {
            settings["files.refactoring.autoSave"] = policy;
        }
        std::fs::write(&config, serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut app = App::new(root.clone(), crate::keys::Profile::Linux);
        app.configure_settings(Some(config)).unwrap();
        app.open(&a).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.doc().path.as_ref() != Some(&a) {
            app.poll();
            assert!(Instant::now() < deadline, "{}", app.message);
        }
        let first = app.doc().id;
        app.doc_mut().insert("dirty ", false);
        let mut peer = Document::open(&b).unwrap();
        peer.insert("dirty ", false);
        let hidden = peer.id;
        app.hidden_documents.push(peer);
        let other = root.join("other.txt");
        std::fs::write(&other, "other\r\n").unwrap();
        let mut doc = Document::open(&other).unwrap();
        doc.insert("unrelated ", false);
        let unrelated = doc.id;
        app.hidden_documents.push(doc);
        Self {
            _dir: dir,
            app,
            owner: Arc::new(AtomicBool::new(false)),
            a,
            b,
            c,
            first,
            hidden,
            unrelated,
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
    fn entries(&self) -> Value {
        json!([
            {"uri":crate::lsp::file_uri(&self.a).unwrap(),"edits":[edit(6,7,"AA")]},
            {"uri":crate::lsp::file_uri(&self.b).unwrap(),"edits":[edit(6,7,"BB")]},
            {"uri":crate::lsp::file_uri(&self.c).unwrap(),"edits":[edit(0,1,"CC")]}
        ])
    }
    fn start(&mut self, entries: Value) -> u64 {
        let mirrors: Vec<_> = self
            .app
            .documents
            .iter()
            .chain(&self.app.hidden_documents)
            .map(|doc| MirrorSnapshot {
                document: doc.id,
                uri: resource(doc).unwrap(),
                path: doc.path.clone(),
                revision: doc.revision,
                text_epoch: doc.text_epoch(),
                resource_epoch: doc.resource_epoch(),
                language_epoch: doc.language_epoch(),
                version: 0,
            })
            .collect();
        let origin = Origin::native("refactoring-fixture", self.owner.clone()).unwrap();
        let capture = self
            .app
            .capture_workspace_transaction(
                origin,
                &json!({"entries":entries,"metadata":{"isRefactoring":true},"targetReceipts":true}),
                &mirrors,
                Instant::now(),
            )
            .unwrap();
        self.app.start_workspace_transaction(capture).unwrap()
    }
    fn publish(&mut self, token: u64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.app.poll_workspace_transactions();
            if self.app.workspace_transaction_published(token) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Publication failed: {}",
                self.app.message
            );
            std::thread::yield_now();
        }
    }
    fn finish(&mut self, token: u64) -> Completion {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.app.poll();
            if let Some(done) = self.app.take_workspace_transaction_completion(token) {
                return done;
            }
            assert!(Instant::now() < deadline, "{}", self.app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn unchanged(&self) {
        for (path, text) in [(&self.a, A), (&self.b, B), (&self.c, C)] {
            assert_eq!(std::fs::read(path).unwrap(), text.as_bytes());
        }
    }
    fn unrelated(&self) {
        assert_eq!(self.model(self.unrelated).text, "unrelated other\r\n");
        assert!(self.model(self.unrelated).dirty());
        assert_eq!(
            std::fs::read(self.model(self.unrelated).path.as_ref().unwrap()).unwrap(),
            b"other\r\n"
        );
    }
}
fn edit(start: usize, end: usize, text: &str) -> Value {
    json!({"range":{"start":{"line":0,"character":start},"end":{"line":0,"character":end}},"newText":text})
}
#[test]
fn default_multi_resource_refactoring_saves_actual_dirty_hidden_and_unopened_cohort_after_atomic_publication()
 {
    let mut f = Fixture::new(None);
    let token = f.start(f.entries());
    f.publish(token);
    f.unchanged();
    assert_eq!(f.model(f.first).text, "dirty AA 猫🙂\r\n");
    assert_eq!(f.model(f.hidden).text, "dirty BB α🙂\r\n");
    assert!(f.app.workspace_transactions_pending());
    assert!(f.app.workspace_transaction_completion().is_none());
    let done = f.finish(token);
    assert!(done.applied, "{:?}", done.reason);
    assert!(done.reason.is_none(), "{:?}", done.reason);
    assert_eq!(done.targets.len(), 3);
    assert_eq!(std::fs::read(&f.a).unwrap(), "dirty AA 猫🙂\r\n".as_bytes());
    assert_eq!(std::fs::read(&f.b).unwrap(), "dirty BB α🙂\r\n".as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), "CC β🙂\r\n".as_bytes());
    let new = f
        .app
        .hidden_documents
        .iter()
        .find(|doc| doc.path.as_ref() == Some(&f.c))
        .unwrap()
        .id;
    assert!(!f.model(f.first).dirty());
    assert!(!f.model(f.hidden).dirty());
    assert!(!f.model(new).dirty());
    f.unrelated();
    assert!(f.app.extension_host.is_none());
    f.app.undo_editor();
    assert_eq!(f.model(f.first).text, format!("dirty {A}"));
    assert_eq!(f.model(f.hidden).text, format!("dirty {B}"));
    assert_eq!(f.model(new).text, C);
    assert!(f.model(f.first).dirty());
    assert!(f.model(f.hidden).dirty());
    assert!(f.model(new).dirty());
    assert_eq!(std::fs::read(&f.c).unwrap(), "CC β🙂\r\n".as_bytes());
    f.app.redo_editor();
    assert!(!f.model(f.first).dirty());
    assert!(!f.model(f.hidden).dirty());
    assert!(!f.model(new).dirty());
    f.unrelated();
}
#[test]
fn dirty_noop_peer_is_involved_and_saved_while_unrelated_dirty_copies_are_preserved() {
    let mut f = Fixture::new(Some(json!(true)));
    let entries = json!([
        {"uri":crate::lsp::file_uri(&f.a).unwrap(),"edits":[edit(0,0,"")]},
        {"uri":crate::lsp::file_uri(&f.b).unwrap(),"edits":[edit(0,0,"")]}
    ]);
    let before = f.model(f.first).revision;
    let token = f.start(entries);
    let done = f.finish(token);
    assert!(done.applied);
    assert!(done.reason.is_none(), "{:?}", done.reason);
    assert_eq!(f.model(f.first).revision, before);
    assert_eq!(
        std::fs::read(&f.a).unwrap(),
        format!("dirty {A}").as_bytes()
    );
    assert_eq!(
        std::fs::read(&f.b).unwrap(),
        format!("dirty {B}").as_bytes()
    );
    f.unrelated();
}
#[test]
fn single_resource_and_disabled_or_non_boolean_root_policy_do_not_autosave() {
    for policy in [
        None,
        Some(json!(true)),
        Some(json!(false)),
        Some(json!("true")),
        Some(Value::Null),
    ] {
        let mut f = Fixture::new(policy.clone());
        let entries = if matches!(
            policy,
            Some(Value::Bool(false) | Value::String(_) | Value::Null)
        ) {
            f.entries()
        } else {
            json!([{ "uri":crate::lsp::file_uri(&f.a).unwrap(),"edits":[edit(6,7,"AA")]}])
        };
        let token = f.start(entries);
        let done = f.finish(token);
        assert!(done.applied, "{:?}", done.reason);
        f.unchanged();
        assert!(f.model(f.first).dirty());
        f.unrelated();
        assert!(!f.app.native_save_worker_busy());
    }
}
#[test]
fn save_conflict_keeps_atomic_edits_applied_and_preserves_successful_peer_commits_and_grouped_undo()
{
    let mut f = Fixture::new(None);
    let token = f.start(f.entries());
    f.publish(token);
    std::fs::write(&f.b, "external\r\n").unwrap();
    let done = f.finish(token);
    assert!(done.applied);
    assert!(done.reason.is_some());
    assert_eq!(std::fs::read(&f.b).unwrap(), b"external\r\n");
    assert_eq!(f.model(f.hidden).text, "dirty BB α🙂\r\n");
    assert!(f.model(f.hidden).dirty());
    assert_eq!(std::fs::read(&f.a).unwrap(), "dirty AA 猫🙂\r\n".as_bytes());
    assert_eq!(std::fs::read(&f.c).unwrap(), "CC β🙂\r\n".as_bytes());
    f.app.undo_editor();
    assert_eq!(f.model(f.first).text, format!("dirty {A}"));
    assert_eq!(f.model(f.hidden).text, format!("dirty {B}"));
    assert_eq!(std::fs::read(&f.b).unwrap(), b"external\r\n");
    f.unrelated();
}
#[test]
fn deferred_target_edit_undo_cannot_replace_its_original_post_publication_save_proof() {
    let mut f = Fixture::new(None);
    let token = f.start(f.entries());
    f.publish(token);
    f.model_mut(f.hidden).insert("X", false);
    f.model_mut(f.hidden).undo();
    let done = f.finish(token);
    assert!(done.applied);
    assert!(done.reason.is_some());
    assert_eq!(std::fs::read(&f.b).unwrap(), B.as_bytes());
    assert_eq!(f.model(f.hidden).text, "dirty BB α🙂\r\n");
    f.model_mut(f.hidden).redo();
    assert_eq!(f.model(f.hidden).text, "dirty BBX α🙂\r\n");
    f.unrelated();
}
#[test]
fn post_publication_owner_retirement_keeps_applied_true_without_saving_or_rolling_back() {
    let mut f = Fixture::new(None);
    let token = f.start(f.entries());
    f.publish(token);
    f.owner.store(true, Ordering::Release);
    let done = f.finish(token);
    assert!(done.applied);
    assert!(done.reason.is_some());
    assert_eq!(f.model(f.first).text, "dirty AA 猫🙂\r\n");
    f.unchanged();
    f.unrelated();
    assert!(!f.app.native_save_worker_busy());
}
#[test]
fn native_refactoring_save_as_publishes_real_transition_and_cancel_never_rolls_back() {
    for accept in [true, false] {
        let mut f = Fixture::new(None);
        let mut untitled = Document::default();
        untitled.insert("U 猫🙂\n", false);
        let id = untitled.id;
        let uri = resource(&untitled).unwrap();
        f.app.hidden_documents.push(untitled);
        let entries = json!([{"uri":crate::lsp::file_uri(&f.a).unwrap(),"edits":[edit(6,7,"AA")]},{"uri":uri,"edits":[edit(0,1,"UU")]}]);
        let token = f.start(entries);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            f.app.poll();
            if matches!(
                f.app.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::SaveAs)
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "{}", f.app.message);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(f.app.workspace_transaction_completion().is_none());
        assert_eq!(f.app.doc().id, f.first);
        let path = f.app.workspace.root.join("saved-untitled 猫 # %'(.txt");
        if accept {
            f.app.prompt.as_mut().unwrap().text = path.to_string_lossy().into_owned();
            f.app.prompt.as_mut().unwrap().select_all = false;
            f.app.event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
        } else {
            f.app
                .event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        }
        let done = f.finish(token);
        assert!(done.applied);
        assert_eq!(f.model(id).text, "UU 猫🙂\n");
        assert_eq!(f.app.doc().id, f.first);
        let receipt = done
            .targets
            .iter()
            .find(|target| target.document == id)
            .unwrap();
        if accept {
            assert!(done.reason.is_none(), "{:?}", done.reason);
            assert_eq!(std::fs::read(&path).unwrap(), "UU 猫🙂\n".as_bytes());
            assert!(!f.model(id).dirty());
            assert_eq!(
                receipt.saved_resource,
                Some(crate::extensions::document_uri(f.model(id)).unwrap())
            );
            assert!(receipt.saved_generation.is_some());
        } else {
            assert!(done.reason.is_some());
            assert!(!path.exists());
            assert!(f.model(id).dirty());
            assert!(receipt.saved_resource.is_none());
        }
        f.unrelated();
    }
}

#[test]
fn deferred_target_effective_language_aba_does_not_renew_the_save_source() {
    let mut f = Fixture::new(None);
    let token = f.start(f.entries());
    f.publish(token);
    f.model_mut(f.hidden)
        .set_language_override(Some("rust".into()))
        .unwrap();
    f.model_mut(f.hidden).set_language_override(None).unwrap();
    let done = f.finish(token);
    assert!(done.applied);
    assert!(done.reason.is_some());
    assert_eq!(std::fs::read(&f.b).unwrap(), B.as_bytes());
    assert!(f.model(f.hidden).dirty());
    f.unrelated();
}
