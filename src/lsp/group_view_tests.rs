//! Real framed replies distinguish UI view ownership from model-owned save work.
use super::formatting_tests::{held, start, until};
use super::*;

const ORIGINAL: &str = "猫🙂 original\r\nlast\r\n";

fn fixture() -> (tempfile::TempDir, Document, Client) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.cpp");
    std::fs::write(&path, ORIGINAL).unwrap();
    let document = Document::open(&path).unwrap();
    let client = start(directory.path(), &document);
    (directory, document, client)
}
fn formatting(client: &mut Client, doc: &Document, view: Option<u64>) -> u64 {
    let token = client.next_id;
    client
        .request_in_view(
            "textDocument/formatting",
            doc,
            json!({"options":{"tabSize":4,"insertSpaces":true}}),
            view,
        )
        .unwrap();
    held(client, token);
    token
}
fn release(client: &mut Client, token: u64) -> Request {
    client
        .notify("fixture/release", json!({"id":token,"result":[]}))
        .unwrap();
    let events = until(client, |client, _| client.formatting_available());
    events
        .into_iter()
        .find_map(|event| match event {
            Event::FormattingResult(request, Ok(value)) if request.token == token => {
                assert_eq!(value, json!([]));
                Some(request)
            }
            _ => None,
        })
        .expect("The actual formatter response must release its original slot")
}
fn peer_state(client: &mut Client) -> Value {
    client.notify("fixture/state", json!({})).unwrap();
    until(client, |_, events| {
        events.iter().any(
            |event| matches!(event,Event::Message(text) if text.starts_with("formatter-state:")),
        )
    })
    .into_iter()
    .find_map(|event| match event {
        Event::Message(text) => text
            .strip_prefix("formatter-state:")
            .map(|text| serde_json::from_str(text).unwrap()),
        _ => None,
    })
    .unwrap()
}
fn hover_reply(client: &mut Client, token: u64) -> Request {
    until(client, |_, events| {
        events
            .iter()
            .any(|event| matches!(event,Event::Response(request,_) if request.token==token))
    })
    .into_iter()
    .find_map(|event| match event {
        Event::Response(request, value) if request.token == token => {
            assert!(value.is_null());
            Some(request)
        }
        _ => None,
    })
    .unwrap()
}

#[test]
fn identical_document_after_group_round_trip_rejects_ui_reply_but_releases_actual_slot() {
    let (directory, mut document, mut client) = fixture();
    document.insert("x", false);
    document.undo();
    document.set_selections(vec![
        Selection {
            cursor: 2,
            anchor: Some(0),
            desired_column: None,
        },
        Selection::caret(6),
    ]);
    client.sync(std::slice::from_ref(&document)).unwrap();
    let token = formatting(&mut client, &document, Some(17));
    let original = client.pending[&token].clone();
    assert!(client.request_current(&original));
    let proof = (
        document.id,
        document.revision,
        document.text_epoch(),
        document.save_generation(),
        document.saved_revision,
    );
    let selections = document.selections();
    client.invalidate_editor_views().unwrap();
    client.invalidate_editor_views().unwrap();
    assert_eq!(client.editor_view_epoch, Some(2));
    assert!(!client.request_current(&original));
    assert!(!client.formatting_request_current(token));
    assert!(!client.formatting_available());
    assert!(client.pending.contains_key(&token));
    let state = peer_state(&mut client);
    assert_eq!(state["calls"].as_array().unwrap().len(), 1);
    assert_eq!(state["cancellations"], json!([]));
    assert_eq!(
        (
            document.id,
            document.revision,
            document.text_epoch(),
            document.save_generation(),
            document.saved_revision
        ),
        proof
    );
    assert_eq!(document.selections(), selections);
    assert_eq!(document.text.to_string(), ORIGINAL);
    assert_eq!(
        std::fs::read(directory.path().join("main.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
    let stale = release(&mut client, token);
    assert_eq!(stale.editor_view_epoch, Some(0));
    assert!(!client.request_current(&stale));
    assert!(client.formatting_available());
    let fresh = formatting(&mut client, &document, Some(17));
    let current = release(&mut client, fresh);
    assert_eq!(current.editor_view_epoch, Some(2));
    assert!(client.request_current(&current));
    document.redo();
    assert_eq!(document.text.to_string(), "x".to_owned() + ORIGINAL);
    assert_eq!(
        std::fs::read(directory.path().join("main.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
}

#[test]
fn model_owned_save_formatter_survives_view_round_trip_without_capacity_or_proof_changes() {
    let (directory, mut document, mut client) = fixture();
    document.insert("RAW ", false);
    client.sync(std::slice::from_ref(&document)).unwrap();
    let token = client.request_document_formatting(&document).unwrap();
    held(&mut client, token);
    let request = client.pending[&token].clone();
    assert!(request.view.is_none());
    assert!(request.editor_view_epoch.is_none());
    let epoch = document.text_epoch();
    let selections = document.selections();
    client.invalidate_editor_views().unwrap();
    client.invalidate_editor_views().unwrap();
    assert!(client.request_current(&request));
    assert!(client.formatting_request_current(token));
    assert!(!client.formatting_available());
    let next_id = client.next_id;
    assert!(client.request_document_formatting(&document).is_err());
    assert_eq!(client.next_id, next_id);
    let state = peer_state(&mut client);
    assert_eq!(state["calls"].as_array().unwrap().len(), 1);
    assert_eq!(state["cancellations"], json!([]));
    let reply = release(&mut client, token);
    assert!(client.request_current(&reply));
    assert_eq!(reply.editor_view_epoch, None);
    assert_eq!(document.text_epoch(), epoch);
    assert_eq!(document.selections(), selections);
    assert_eq!(document.text.to_string(), "RAW ".to_owned() + ORIGINAL);
    assert!(document.dirty());
    assert_eq!(
        std::fs::read(directory.path().join("main.cpp")).unwrap(),
        ORIGINAL.as_bytes()
    );
}

#[test]
fn follow_up_keeps_original_view_proof_and_stale_follow_up_consumes_no_request_id() {
    let (_directory, document, mut client) = fixture();
    let token = client.next_id;
    client
        .request_in_view("textDocument/hover", &document, json!({}), Some(31))
        .unwrap();
    let original = hover_reply(&mut client, token);
    assert!(client.request_current(&original));
    let follow_up = client.next_id;
    client
        .follow_up(&original, "textDocument/hover", json!({}))
        .unwrap();
    assert_eq!(
        client.pending[&follow_up].editor_view_epoch,
        original.editor_view_epoch
    );
    client.invalidate_editor_views().unwrap();
    client.invalidate_editor_views().unwrap();
    let next_id = client.next_id;
    let pending = client.pending.len();
    assert!(
        client
            .follow_up(&original, "textDocument/hover", json!({}))
            .unwrap_err()
            .to_string()
            .contains("Original editor view changed")
    );
    assert_eq!(client.next_id, next_id);
    assert_eq!(client.pending.len(), pending);
    let stale = hover_reply(&mut client, follow_up);
    assert_eq!(stale.editor_view_epoch, Some(0));
    assert!(!client.request_current(&stale));
    let mut stripped = original;
    stripped.view = None;
    assert!(!client.request_current(&stripped));
}

#[test]
fn exhausted_view_epoch_retires_ui_without_allocating_ids_or_blocking_model_requests() {
    let (_directory, document, mut client) = fixture();
    client.editor_view_epoch = Some(u64::MAX);
    let token = formatting(&mut client, &document, Some(41));
    let original = client.pending[&token].clone();
    assert!(client.request_current(&original));
    assert!(
        client
            .invalidate_editor_views()
            .unwrap_err()
            .to_string()
            .contains("generation exhausted")
    );
    assert_eq!(client.editor_view_epoch, None);
    assert!(
        client
            .invalidate_editor_views()
            .unwrap_err()
            .to_string()
            .contains("ownership is retired")
    );
    let next_id = client.next_id;
    let pending = client.pending.len();
    assert!(
        client
            .request_in_view("textDocument/hover", &document, json!({}), Some(41))
            .unwrap_err()
            .to_string()
            .contains("ownership is retired")
    );
    assert!(
        client
            .follow_up(&original, "textDocument/hover", json!({}))
            .is_err()
    );
    assert_eq!(client.next_id, next_id);
    assert_eq!(client.pending.len(), pending);
    assert!(!client.request_current(&original));
    assert!(!client.formatting_available());
    assert_eq!(peer_state(&mut client)["cancellations"], json!([]));
    let hover = client.next_id;
    client
        .request("textDocument/hover", &document, json!({}))
        .unwrap();
    let model_reply = hover_reply(&mut client, hover);
    assert_eq!(model_reply.editor_view_epoch, None);
    assert!(client.request_current(&model_reply));
    let stale = release(&mut client, token);
    assert!(!client.request_current(&stale));
    let save = client.request_document_formatting(&document).unwrap();
    held(&mut client, save);
    assert!(client.request_current(&client.pending[&save]));
    let saved = release(&mut client, save);
    assert!(client.request_current(&saved));
    assert_eq!(client.editor_view_epoch, None);
}
