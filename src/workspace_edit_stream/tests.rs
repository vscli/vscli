use super::*;
use serde_json::json;

const A: &str = "file:///tmp/original.cpp";
const B: &str = "file:///tmp/renamed.cpp";
fn text(uri: &str, value: &str) -> Value {
    json!({"resource":uri,"textEdit":{"text":value,"range":{"startLineNumber":1,"startColumn":1,"endLineNumber":1,"endColumn":1}}})
}

#[test]
fn create_edit_rename_edit_delete_preserves_all_original_phase_boundaries() {
    let dto = json!({"edits":[
        {"newResource":A,"options":{"contents":{"type":"base64","value":"AP8="}}},
        text(A,"first\0猫🙂\r\n"),
        {"oldResource":A,"newResource":B,"options":{}},
        text(B,"second"),
        {"oldResource":B,"options":{"recursive":true}},
    ]});
    let stream = Stream::parse(&dto).unwrap();
    assert_eq!(
        stream.groups(),
        &[
            Group {
                kind: Kind::File,
                entries: 0..1
            },
            Group {
                kind: Kind::Text,
                entries: 1..2
            },
            Group {
                kind: Kind::File,
                entries: 2..3
            },
            Group {
                kind: Kind::Text,
                entries: 3..4
            },
            Group {
                kind: Kind::File,
                entries: 4..5
            },
        ]
    );
    let Entry::Text(first) = stream.entry(1).unwrap() else {
        panic!("original text phase")
    };
    assert_eq!(first.edit["text"], "first\0猫🙂\r\n");
    let Entry::File(create) = stream.entry(0).unwrap() else {
        panic!("original creation phase")
    };
    assert_eq!(
        create.option("contents").unwrap(),
        &json!({"type":"base64","value":"AP8="})
    );
    assert!(std::ptr::eq(
        create.options.unwrap(),
        dto["edits"][0]["options"].as_object().unwrap()
    ));
}

#[test]
fn file_subbatches_keep_contiguous_kinds_and_folder_options_without_global_reordering() {
    let dto = json!({"edits":[
        {"newResource":A,"options":{"contents":{"type":"base64","value":""}}},
        {"newResource":B,"options":{"folder":true}},
        {"oldResource":A,"newResource":B,"options":{}},
        {"oldResource":B,"newResource":A,"options":{"copy":true}},
        {"oldResource":A,"options":{}},
        {"newResource":A,"options":{}},
    ]});
    let stream = Stream::parse(&dto).unwrap();
    assert_eq!(
        stream.groups(),
        &[Group {
            kind: Kind::File,
            entries: 0..6
        }]
    );
    assert_eq!(
        stream.file_batches(&stream.groups()[0]).unwrap(),
        vec![
            FileBatch {
                kind: FileKind::Create,
                entries: 0..2
            },
            FileBatch {
                kind: FileKind::Rename,
                entries: 2..3
            },
            FileBatch {
                kind: FileKind::Copy,
                entries: 3..4
            },
            FileBatch {
                kind: FileKind::Delete,
                entries: 4..5
            },
            FileBatch {
                kind: FileKind::Create,
                entries: 5..6
            },
        ]
    );
    let Entry::File(first) = stream.entry(0).unwrap() else {
        panic!("file before folder")
    };
    let Entry::File(second) = stream.entry(1).unwrap() else {
        panic!("folder operand")
    };
    assert_eq!(first.option("contents").unwrap()["value"], "");
    assert_eq!(second.option("folder"), Some(&Value::Bool(true)));
    assert!(
        stream
            .file_batches(&Group {
                kind: Kind::File,
                entries: 1..3
            })
            .is_err()
    );
}

#[test]
fn defined_false_or_null_overwrite_prevents_ignore_existing_short_circuit() {
    for value in [
        json!(false),
        json!(null),
        json!(0),
        json!(""),
        json!(true),
        json!([]),
    ] {
        let dto = json!({"edits":[{"newResource":A,"options":{"overwrite":value,"ignoreIfExists":true}}]});
        let stream = Stream::parse(&dto).unwrap();
        let Entry::File(file) = stream.entry(0).unwrap() else {
            panic!("file")
        };
        assert_eq!(file.overwrite(), Some(truthy(&value)));
        assert!(!file.ignore_existing());
    }
    let dto = json!({"edits":[{"newResource":A,"options":{"ignoreIfExists":"yes","recursive":true,"skipTrashBin":false,"maxSize":-1,"custom":{"source":"preserved"}}}]});
    let stream = Stream::parse(&dto).unwrap();
    let Entry::File(file) = stream.entry(0).unwrap() else {
        panic!("file")
    };
    assert_eq!(file.overwrite(), None);
    assert!(file.ignore_existing());
    assert_eq!(file.option("custom").unwrap()["source"], "preserved");
    assert_eq!(file.option("maxSize"), Some(&json!(-1)));
}

#[test]
fn later_create_suppresses_only_its_text_version_without_rewriting_original_records() {
    let mut first = text(A, "before create");
    first["versionId"] = json!(7);
    let mut unrelated = text(B, "unrelated");
    unrelated["versionId"] = json!(8);
    let dto =
        json!({"edits":[first,unrelated,{"newResource":A,"options":{"ignoreIfExists":true}}]});
    let stream = Stream::parse(&dto).unwrap();
    let Entry::Text(first) = stream.entry(0).unwrap() else {
        panic!("first")
    };
    let Entry::Text(unrelated) = stream.entry(1).unwrap() else {
        panic!("unrelated")
    };
    assert_eq!(first.requested_version, Some(7));
    assert_eq!(stream.effective_version(&first), None);
    assert_eq!(stream.effective_version(&unrelated), Some(8));
    assert_eq!(dto["edits"][0]["versionId"], 7);
}

#[test]
fn notebook_and_metadata_boundaries_remain_distinct_from_text_resource_grouping() {
    let dto = json!({"edits":[
        text(A,"first"),
        {"resource":A,"cellEdit":{"editType":5,"metadata":{"source":"notebook"}},"metadata":{"label":"confirm","needsConfirmation":true}},
        text(A,"second"),
    ]});
    let stream = Stream::parse(&dto).unwrap();
    assert_eq!(
        stream.groups(),
        &[
            Group {
                kind: Kind::Text,
                entries: 0..1
            },
            Group {
                kind: Kind::Notebook,
                entries: 1..2
            },
            Group {
                kind: Kind::Text,
                entries: 2..3
            },
        ]
    );
    let Entry::Notebook(notebook) = stream.entry(1).unwrap() else {
        panic!("notebook")
    };
    assert_eq!(notebook.edit["metadata"]["source"], "notebook");
    assert_eq!(notebook.metadata.unwrap()["needsConfirmation"], true);
}

#[test]
fn absence_empty_bytes_and_data_transfer_contents_remain_different_original_inputs() {
    let dto = json!({"edits":[
        {"newResource":A,"options":{}},
        {"newResource":A,"options":{"contents":{"type":"base64","value":""}}},
        {"newResource":A,"options":{"contents":{"type":"dataTransferItem","id":"real-item"}}},
    ]});
    let stream = Stream::parse(&dto).unwrap();
    let values: Vec<_> = (0..3)
        .map(|index| {
            let Entry::File(file) = stream.entry(index).unwrap() else {
                panic!("file")
            };
            file.option("contents")
        })
        .collect();
    assert_eq!(values[0], None);
    assert_ne!(values[1], values[2]);
    assert_eq!(values[2].unwrap()["id"], "real-item");
}

#[test]
fn malformed_late_entry_refuses_the_entire_stream_without_any_effect_or_input_mutation() {
    for bad in [
        json!({"oldResource":null,"newResource":null}),
        json!({"newResource":A,"textEdit":{}}),
        json!({"resource":A,"textEdit":{},"cellEdit":{}}),
        json!({"oldResource":A,"surprise":true}),
        json!({"newResource":"not a URI"}),
        json!({"newResource":A,"options":[]}),
    ] {
        let dto = json!({"edits":[text(A,"must never apply"),bad]});
        let before = dto.clone();
        assert!(Stream::parse(&dto).is_err());
        assert_eq!(dto, before);
    }
}

#[test]
fn original_resource_entry_and_aggregate_bounds_apply_before_retention() {
    let empty = json!({"edits":[]});
    assert!(Stream::parse(&empty).unwrap().is_empty());
    let large = json!({"edits":vec![text(A,"");4097]});
    assert!(Stream::parse(&large).is_err());
    let at_limit = json!({"edits":vec![text(A,"");4096]});
    assert_eq!(Stream::parse(&at_limit).unwrap().len(), 4096);
    let resources = json!({"edits":(0..129).map(|id| text(&format!("file:///tmp/{id}"),"")).collect::<Vec<_>>()});
    assert!(Stream::parse(&resources).is_err());
    let at_resource_limit = json!({"edits":(0..128).map(|id| text(&format!("file:///tmp/{id}"),"")).collect::<Vec<_>>()});
    assert_eq!(Stream::parse(&at_resource_limit).unwrap().len(), 128);
    let escaped = json!({"edits":[text(A,&"\0".repeat(400_000))]});
    assert!(Stream::parse(&escaped).is_err());
    let mut deep = json!(true);
    for _ in 0..97 {
        deep = json!({"nested":deep});
    }
    let nested = json!({"edits":[{"newResource":A,"options":{"custom":deep}}]});
    assert!(Stream::parse(&nested).is_err());
    assert!(Stream::parse(&empty).unwrap().entry(0).is_err());
}
