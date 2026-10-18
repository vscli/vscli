use super::*;

fn state(client: &mut Client) -> Value {
    client.fixture_notify("fixture/state", json!({})).unwrap();
    formatting_tests::until(client, |_, events| {
        events.iter().any(
            |event| matches!(event, Event::Message(value) if value.starts_with("formatter-state:")),
        )
    })
    .into_iter()
    .find_map(|event| match event {
        Event::Message(value) => value
            .strip_prefix("formatter-state:")
            .map(|text| serde_json::from_str(text).unwrap()),
        _ => None,
    })
    .unwrap()
}

#[test]
fn retained_hidden_sync_preserves_original_formatter_identity_version_and_actual_cancel_slot() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let visible_path = root.join("visible.cpp");
    let hidden_path = root.join("hidden.cpp");
    std::fs::write(&visible_path, "visible\n").unwrap();
    std::fs::write(&hidden_path, "hidden 猫🙂\r\n").unwrap();
    let visible = Document::open_existing(&visible_path).unwrap();
    let mut hidden = Document::open_existing(&hidden_path).unwrap();
    hidden.insert("dirty ", false);
    let mut client = formatting_tests::start(&root, &visible);
    client
        .sync_retained(
            std::slice::from_ref(&visible),
            std::slice::from_ref(&hidden),
        )
        .unwrap();
    let token = client.request_document_formatting(&hidden).unwrap();
    assert_eq!(client.pending[&token].document_id, hidden.id);
    assert_eq!(client.pending[&token].text_epoch, hidden.text_epoch());
    let version = client.pending[&token].sync_version;
    formatting_tests::held(&mut client, token);
    client
        .sync_retained(
            std::slice::from_ref(&visible),
            std::slice::from_ref(&hidden),
        )
        .unwrap();
    let observed = state(&mut client);
    let uri = file_uri(&hidden_path).unwrap();
    let opens: Vec<_> = observed["lifecycle"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            row["method"] == "textDocument/didOpen" && row["params"]["textDocument"]["uri"] == uri
        })
        .collect();
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0]["params"]["textDocument"]["version"], version);
    assert_eq!(
        opens[0]["params"]["textDocument"]["text"],
        hidden.text.to_string()
    );
    assert!(
        !observed["lifecycle"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["method"] == "textDocument/didClose"
                && row["params"]["textDocument"]["uri"] == uri)
    );
    hidden.insert("independent ", false);
    hidden.undo();
    client
        .sync_retained(
            std::slice::from_ref(&visible),
            std::slice::from_ref(&hidden),
        )
        .unwrap();
    assert!(!client.request_current(&client.pending[&token]));
    client.cancel_formatting_request(token).unwrap();
    assert!(!client.formatting_available());
    assert!(client.request_document_formatting(&hidden).is_err());
    client
        .fixture_notify("fixture/release", json!({"id":token,"result":[]}))
        .unwrap();
    formatting_tests::until(&mut client, |client, _| client.formatting_available());
    client
        .sync_retained(std::slice::from_ref(&visible), &[])
        .unwrap();
    let observed = state(&mut client);
    assert!(
        observed["lifecycle"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["method"] == "textDocument/didClose"
                && row["params"]["textDocument"]["uri"] == uri)
    );
}
