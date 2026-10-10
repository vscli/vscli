use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    document::Document,
    lsp::{Client, DiagnosticPublication, Event, Request},
};

fn client(root: &Path) -> Client {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics_server.py");
    let mut client = Client::start(
        if cfg!(windows) { "python" } else { "python3" },
        &[
            fixture.to_string_lossy().into(),
            root.to_string_lossy().into(),
        ],
        root,
        "rust".into(),
    )
    .unwrap();
    collect_until(&mut client, |events| {
        events.iter().any(|event| matches!(event, Event::Ready))
    });
    client
}
fn document(root: &Path) -> Document {
    let path = root.join("input.rs");
    std::fs::write(&path, "a 🙂\r\n").unwrap();
    Document::open(&path).unwrap()
}
fn collect_until(client: &mut Client, predicate: impl Fn(&[Event]) -> bool) -> Vec<Event> {
    let started = Instant::now();
    let mut events = Vec::new();
    loop {
        events.extend(client.poll().unwrap());
        if predicate(&events) {
            return events;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{}",
            client.debug_summary()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn publication(client: &mut Client, version: i64) -> DiagnosticPublication {
    let mut events = collect_until(client, |events| {
        events.iter().any(
            |event| matches!(event, Event::Diagnostics(batch) if batch.snapshot.version == version),
        )
    });
    events
        .drain(..)
        .find_map(|event| match event {
            Event::Diagnostics(batch) if batch.snapshot.version == version => Some(batch),
            _ => None,
        })
        .unwrap()
}
fn manual_publication(client: &mut Client, doc: &Document, options: Value) -> Vec<Event> {
    client
        .request("fixture/publish", doc, json!({"publication":options}))
        .unwrap();
    collect_until(client, |events| {
        events.iter().any(|event| matches!(event, Event::Response(request, _) if request.method == "fixture/publish"))
    })
}
fn action_request(client: &mut Client, doc: &Document) -> Request {
    client
        .request("textDocument/codeAction", doc, json!({}))
        .unwrap();
    collect_until(client, |events| events.iter().any(|event| matches!(event, Event::ActionResult(request, Ok(_)) if request.method == "textDocument/codeAction")))
        .into_iter().find_map(|event| match event { Event::ActionResult(request, Ok(_)) => Some(request), _ => None }).unwrap()
}

#[test]
fn unsynchronized_edit_undo_invalidates_publication_and_advances_protocol_version() {
    let root = tempfile::tempdir().unwrap();
    let mut doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let original = publication(&mut client, 1);
    assert!(client.diagnostic_current(&original, &doc));
    let revision = doc.revision;
    doc.insert("x", false);
    doc.undo();
    assert_eq!(doc.revision, revision);
    assert_eq!(doc.text.to_string(), "a 🙂\r\n");
    assert!(!client.diagnostic_current(&original, &doc));
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let fresh = publication(&mut client, 2);
    assert!(fresh.snapshot.text_epoch > original.snapshot.text_epoch);
    assert!(client.diagnostic_current(&fresh, &doc));
    assert!(!client.diagnostic_current(&original, &doc));
    let journal = std::fs::read_to_string(root.path().join("sync.jsonl")).unwrap();
    assert!(journal.lines().any(
        |line| serde_json::from_str::<Value>(line).unwrap()["method"] == "textDocument/didChange"
    ));
    assert_eq!(
        std::fs::read(root.path().join("input.rs")).unwrap(),
        "a 🙂\r\n".as_bytes()
    );
    doc.redo();
    assert_eq!(doc.text.to_string(), "xa 🙂\r\n");
}

#[test]
fn held_old_version_and_stale_empty_publication_cannot_replace_current_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("hold-1"), "").unwrap();
    let mut doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    doc.insert("x", false);
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let fresh = publication(&mut client, 2);
    std::fs::remove_file(root.path().join("hold-1")).unwrap();
    let events = collect_until(&mut client, |events| {
        events
            .iter()
            .any(|event| matches!(event, Event::Message(message) if message == "published-1"))
    });
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Diagnostics(_)))
    );
    let events = manual_publication(&mut client, &doc, json!({"version":1,"empty":true}));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Diagnostics(_)))
    );
    assert!(client.diagnostic_current(&fresh, &doc));
    let events = manual_publication(&mut client, &doc, json!({"empty":true}));
    assert!(events.iter().any(|event| matches!(event, Event::Diagnostics(batch) if batch.items.is_empty() && client.diagnostic_current(batch,&doc))));
}

#[test]
fn malformed_present_versions_and_diagnostic_shapes_never_become_empty_clears() {
    let root = tempfile::tempdir().unwrap();
    let doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let original = publication(&mut client, 1);
    for value in [
        Value::Null,
        json!("1"),
        json!(1.5),
        json!({}),
        json!([1]),
        json!(i64::MAX),
    ] {
        let events = manual_publication(&mut client, &doc, json!({"version":value,"empty":true}));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Diagnostics(_)))
        );
    }
    for field in ["malformed_diagnostics", "malformed_item"] {
        let events = manual_publication(&mut client, &doc, json!({field:true}));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Diagnostics(_)))
        );
    }
    assert!(client.diagnostic_current(&original, &doc));
}

#[test]
fn unversioned_publication_is_explicitly_marked_and_never_reuses_a_prior_text_epoch() {
    let root = tempfile::tempdir().unwrap();
    let mut doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    publication(&mut client, 1);
    let events = manual_publication(&mut client, &doc, json!({"unversioned":true}));
    let unversioned = events
        .into_iter()
        .find_map(|event| match event {
            Event::Diagnostics(batch) => Some(batch),
            _ => None,
        })
        .unwrap();
    assert!(!unversioned.versioned);
    assert!(client.diagnostic_current(&unversioned, &doc));
    doc.insert("x", false);
    doc.undo();
    assert!(!client.diagnostic_current(&unversioned, &doc));
    // This does not claim provenance for a delayed versionless notification
    // arriving after a later synchronization; the protocol carries no version.
}

#[test]
fn replacement_client_equal_versions_and_reopened_document_identity_reject_old_batches() {
    let root = tempfile::tempdir().unwrap();
    let doc = document(root.path());
    let mut first = client(root.path());
    first.sync(std::slice::from_ref(&doc)).unwrap();
    let old = publication(&mut first, 1);
    let old_request = action_request(&mut first, &doc);
    let mut replacement = client(root.path());
    replacement.sync(std::slice::from_ref(&doc)).unwrap();
    let fresh = publication(&mut replacement, 1);
    assert_eq!(old.snapshot, fresh.snapshot);
    assert!(!replacement.diagnostic_current(&old, &doc));
    assert!(replacement.diagnostic_current(&fresh, &doc));
    assert!(!replacement.request_current(&old_request));
    replacement.sync(&[]).unwrap();
    assert!(!replacement.diagnostic_current(&fresh, &doc));
    let reopened = Document::open(doc.path.as_ref().unwrap()).unwrap();
    assert_ne!(reopened.id, doc.id);
    replacement.sync(std::slice::from_ref(&reopened)).unwrap();
    let next = publication(&mut replacement, 2);
    assert!(replacement.diagnostic_current(&next, &reopened));
    assert!(!replacement.diagnostic_current(&fresh, &reopened));
}

#[test]
fn moving_same_model_out_of_visible_sync_scope_requires_new_publication_when_returned() {
    let root = tempfile::tempdir().unwrap();
    let doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let visible = publication(&mut client, 1);
    client.sync(&[]).unwrap();
    assert!(!client.diagnostic_current(&visible, &doc));
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let returned = publication(&mut client, 2);
    assert_eq!(returned.snapshot.id, visible.snapshot.id);
    assert_eq!(returned.snapshot.text_epoch, visible.snapshot.text_epoch);
    assert!(client.diagnostic_current(&returned, &doc));
    assert!(!client.diagnostic_current(&visible, &doc));
}

#[test]
fn action_workspace_snapshot_tracks_text_epochs_and_current_server_origin() {
    let root = tempfile::tempdir().unwrap();
    let mut doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    publication(&mut client, 1);
    let old = action_request(&mut client, &doc);
    assert!(client.request_current(&old));
    doc.insert("x", false);
    doc.undo();
    client.sync(std::slice::from_ref(&doc)).unwrap();
    publication(&mut client, 2);
    assert!(!client.request_current(&old));
    let fresh = action_request(&mut client, &doc);
    assert_eq!(
        fresh.text_epoch,
        fresh.workspace[doc.path.as_ref().unwrap()].text_epoch
    );
    assert!(fresh.text_epoch > old.text_epoch);
    assert!(client.request_current(&fresh));
}

#[test]
fn opened_lexical_alias_keeps_canonical_identity_without_accepting_unknown_publication_uri() {
    let root = tempfile::tempdir().unwrap();
    let doc = document(root.path());
    let alias = root.path().join(".").join("input.rs");
    let alias_doc = Document::open(&alias).unwrap();
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&alias_doc)).unwrap();
    let batch = publication(&mut client, 1);
    assert_eq!(batch.path, *doc.path.as_ref().unwrap());
    assert!(client.diagnostic_current(&batch, &alias_doc));
    let unknown_uri = vscli::lsp::file_uri(&root.path().join("unknown.rs")).unwrap();
    let events = manual_publication(&mut client, &alias_doc, json!({"uri":unknown_uri}));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Diagnostics(_)))
    );
}

#[test]
fn oversized_or_invalid_publications_are_rejected_without_clearing_prior_items() {
    let root = tempfile::tempdir().unwrap();
    let doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let original = publication(&mut client, 1);
    for options in [
        json!({"message_bytes":4098}),
        json!({"data_bytes":2*1024*1024}),
        json!({"count":5001}),
        json!({"reversed":true}),
        json!({"negative":true}),
        json!({"coordinate":u64::MAX}),
        json!({"coordinate":i64::from(i32::MAX)+1}),
        json!({"severity":0}),
        json!({"severity":5}),
        json!({"severity":null}),
    ] {
        let events = manual_publication(&mut client, &doc, options);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::Diagnostics(_)))
        );
        assert!(events.iter().any(|event| matches!(event, Event::Message(message) if message.starts_with("Diagnostic publication rejected:"))));
        assert!(client.diagnostic_current(&original, &doc));
        assert_eq!(original.items[0].message, "version-1");
    }
    let events = manual_publication(&mut client, &doc, json!({"message_bytes":4096}));
    assert!(events.iter().any(
        |event| matches!(event, Event::Diagnostics(batch) if batch.items[0].message.len()==4096)
    ));
}

#[test]
fn nonaction_request_cannot_cross_close_reopen_of_the_same_unchanged_model() {
    let root = tempfile::tempdir().unwrap();
    let doc = document(root.path());
    let mut client = client(root.path());
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let initial = publication(&mut client, 1);
    let old = manual_publication(&mut client, &doc, json!({}))
        .into_iter()
        .find_map(|event| match event {
            Event::Response(request, _) => Some(request),
            _ => None,
        })
        .unwrap();
    assert!(old.workspace.is_empty());
    assert!(client.request_current(&old));
    client.sync(&[]).unwrap();
    assert!(!client.request_current(&old));
    client.sync(std::slice::from_ref(&doc)).unwrap();
    let reopened = publication(&mut client, 2);
    assert_eq!(initial.snapshot.id, reopened.snapshot.id);
    assert_eq!(initial.snapshot.revision, reopened.snapshot.revision);
    assert_eq!(initial.snapshot.text_epoch, reopened.snapshot.text_epoch);
    assert!(!client.request_current(&old));
    let fresh = manual_publication(&mut client, &doc, json!({}))
        .into_iter()
        .find_map(|event| match event {
            Event::Response(request, _) => Some(request),
            _ => None,
        })
        .unwrap();
    assert!(client.request_current(&fresh));
    assert_eq!(doc.text.to_string(), "a 🙂\r\n");
    assert_eq!(
        std::fs::read(root.path().join("input.rs")).unwrap(),
        "a 🙂\r\n".as_bytes()
    );
}
