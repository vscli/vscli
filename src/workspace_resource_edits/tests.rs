use super::*;
use serde_json::json;
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn control(canceled: &AtomicBool) -> Control<'_> {
    Control::new(canceled, Instant::now() + Duration::from_secs(10))
}
fn prepare(dto: &Value) -> Result<PreparedGroup> {
    let canceled = AtomicBool::new(false);
    let stream = Stream::parse(dto)?;
    prepare_group(&stream, &stream.groups()[0], &control(&canceled))
}
fn uri(path: &std::path::Path) -> String {
    url::Url::from_file_path(path).unwrap().to_string()
}

#[test]
fn original_file_groups_and_indices_do_not_cross_text_or_notebook_boundaries() {
    let dto = json!({"edits":[
        {"newResource":"file:///tmp/a"},
        {"resource":"file:///tmp/a","textEdit":{"text":"text"}},
        {"oldResource":"file:///tmp/a","newResource":"file:///tmp/b"},
        {"resource":"file:///tmp/b","cellEdit":{"editType":1}}
    ]});
    let stream = Stream::parse(&dto).unwrap();
    let canceled = AtomicBool::new(false);
    let groups = prepare_groups(&stream, &control(&canceled)).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].original.entries, 0..1);
    assert_eq!(groups[1].original.entries, 2..3);
    let first = prepare_group(&stream, &stream.groups()[0], &control(&canceled)).unwrap();
    assert_eq!(first.edits.len(), 1);
    assert_eq!(first.edits[0].original_index, 0);
    assert!(matches!(
        first.edits[0].operation,
        Some(Operation::Create { .. })
    ));
    assert!(prepare_group(&stream, &stream.groups()[1], &control(&canceled)).is_err());
    let third = prepare_group(&stream, &stream.groups()[2], &control(&canceled)).unwrap();
    assert_eq!(third.edits[0].original_index, 2);
    assert!(matches!(
        third.edits[0].operation,
        Some(Operation::Move { .. })
    ));
    assert!(
        prepare_group(
            &stream,
            &Group {
                kind: crate::workspace_edit_stream::Kind::File,
                entries: 0..3
            },
            &control(&canceled)
        )
        .is_err()
    );
}

#[test]
fn options_preserve_absent_null_false_and_truthy_effects_with_original_annotations() {
    let dto = json!({"edits":[
        {"newResource":"file:///tmp/a","options":{"ignoreIfExists":true}},
        {"newResource":"file:///tmp/b","options":{"overwrite":null,"ignoreIfExists":true}},
        {"oldResource":"file:///tmp/b","newResource":"file:///tmp/c","options":{"overwrite":false,"copy":true,"recursive":0,"folder":"","skipTrashBin":{},"maxSize":12},"metadata":{"label":"Original","needsConfirmation":true}}
    ]});
    let group = prepare(&dto).unwrap();
    assert_eq!(
        group.edits[0]
            .operation
            .as_ref()
            .unwrap()
            .options()
            .overwrite,
        Presence::Absent
    );
    assert_eq!(
        group.edits[1]
            .operation
            .as_ref()
            .unwrap()
            .options()
            .overwrite,
        Presence::Null
    );
    let options = group.edits[2].operation.as_ref().unwrap().options();
    assert_eq!(options.overwrite, Presence::Value(false));
    assert_eq!(options.copy, Presence::Value(true));
    assert_eq!(options.recursive, Presence::Value(false));
    assert_eq!(options.folder, Presence::Value(false));
    assert_eq!(options.skip_trash_bin, Presence::Value(true));
    assert_eq!(options.max_size, Presence::Value(12.0));
    assert_eq!(
        group.edits[2].metadata.as_ref(),
        Some(&dto["edits"][2]["metadata"])
    );
}

#[test]
fn absent_contents_zero_bytes_and_binary_contents_remain_distinct() {
    let dto = json!({"edits":[
        {"newResource":"file:///tmp/a"},
        {"newResource":"file:///tmp/b","options":{"contents":{"type":"base64","value":""}}},
        {"newResource":"file:///tmp/c","options":{"contents":{"type":"base64","value":"AP+A"}}}
    ]});
    let group = prepare(&dto).unwrap();
    let bytes: Vec<_> = group
        .edits
        .iter()
        .map(|edit| match edit.operation.as_ref().unwrap() {
            Operation::Create { contents, .. } => contents.clone(),
            _ => panic!("create"),
        })
        .collect();
    assert_eq!(bytes, vec![None, Some(Vec::new()), Some(vec![0, 255, 128])]);
}

#[test]
fn local_uri_decoding_is_io_free_and_refuses_unsupported_or_qualified_resources() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("absent/猫🙂\n.bin");
    let group = prepare(&json!({"edits":[{"newResource":uri(&target)}]})).unwrap();
    assert_eq!(
        group.edits[0].operation.as_ref().unwrap().paths(),
        vec![target.as_path()]
    );
    assert!(!target.exists());
    for resource in [
        "mem:/target",
        "file://foreign.invalid/target",
        "file:///tmp/a?query",
        "file:///tmp/a#fragment",
        "file:///tmp/a%00b",
    ] {
        assert!(
            prepare(&json!({"edits":[{"newResource":resource}]})).is_err(),
            "{resource}"
        );
    }
}

#[test]
fn late_corrupt_contents_or_transfer_resolver_refuses_whole_preparation() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    std::fs::write(&original, [0, 255, 128]).unwrap();
    let created = root.path().join("created");
    for invalid in [
        json!({"contents":{"type":"base64","value":"AP?="}}),
        json!({"contents":{"type":"dataTransferItem","id":"opaque"}}),
    ] {
        let dto = json!({"edits":[{"newResource":uri(&created),"options":{"contents":{"type":"base64","value":"AP+A"}}},{"oldResource":uri(&original),"options":invalid}]});
        assert!(prepare(&dto).is_err());
        assert!(!created.exists());
        assert_eq!(std::fs::read(&original).unwrap(), [0, 255, 128]);
    }
    assert!(contents(Some(&json!({"type":"base64","value":"AP+A"})), &mut 2).is_err());
}

#[test]
fn original_cancel_and_deadline_refuse_preparation_without_path_resolution() {
    let dto = json!({"edits":[{"newResource":"file:///missing/a"}]});
    let stream = Stream::parse(&dto).unwrap();
    assert!(
        prepare_group(
            &stream,
            &stream.groups()[0],
            &control(&AtomicBool::new(true))
        )
        .is_err()
    );
    assert!(
        prepare_group(
            &stream,
            &stream.groups()[0],
            &Control::new(
                &AtomicBool::new(false),
                Instant::now() - Duration::from_secs(1)
            )
        )
        .is_err()
    );
}

#[test]
fn untitled_create_skips_after_whole_stream_revival_without_losing_original_row() {
    let dto = json!({"edits":[{"newResource":"untitled:future","metadata":{"label":"Original"}},
        {"resource":"untitled:future","textEdit":{"text":"new text"},"versionId":99},
        {"newResource":"file:///tmp/later","options":{"contents":{"type":"base64","value":"AA=="}}}]});
    let group = prepare(&dto).unwrap();
    assert_eq!(group.edits.len(), 1);
    assert!(group.edits[0].operation.is_none());
    assert_eq!(
        group.edits[0].skipped_untitled_create.as_deref(),
        Some("untitled:future")
    );
    assert_eq!(
        group.edits[0].metadata.as_ref(),
        Some(&dto["edits"][0]["metadata"])
    );
    let stream = Stream::parse(&dto).unwrap();
    let Entry::Text(text) = stream.entry(1).unwrap() else {
        panic!("text")
    };
    assert_eq!(stream.effective_version(&text), None);
    let mut bad = dto;
    bad["edits"][2]["options"]["contents"] = json!({"type":"dataTransferItem","id":"late"});
    assert!(
        prepare(&bad).is_err(),
        "later descriptors are revived before first file group"
    );
    let group = prepare(&json!({"edits":[
        {"oldResource":"file:///tmp/a","newResource":"file:///tmp/b"},
        {"newResource":"untitled:barrier"},
        {"oldResource":"file:///tmp/b","newResource":"file:///tmp/c"}
    ]}))
    .unwrap();
    assert_eq!(
        group.batches.len(),
        3,
        "a skipped CREATE still separates rename batches"
    );
    assert_eq!(
        group
            .edits
            .iter()
            .filter(|edit| edit.operation.is_some())
            .count(),
        2
    );
}

#[test]
fn source_ignored_options_and_nonnumeric_max_size_are_retained_without_physical_effects() {
    for maximum in [json!("1"), json!(false), json!({"future":true})] {
        let dto = json!({"edits":[{"oldResource":"file:///tmp/a","options":{"maxSize":maximum,"futureOption":{"nested":"retained"}}}]});
        let group = prepare(&dto).unwrap();
        assert_eq!(
            group.edits[0]
                .operation
                .as_ref()
                .unwrap()
                .options()
                .max_size,
            Presence::Absent
        );
        assert_eq!(
            group.edits[0].original_options.as_ref(),
            Some(dto["edits"][0]["options"].as_object().unwrap())
        );
    }
    for number in [-1.0, 0.0, 0.5] {
        let group = prepare(
            &json!({"edits":[{"oldResource":"file:///tmp/a","options":{"maxSize":number}}]}),
        )
        .unwrap();
        assert_eq!(
            group.edits[0]
                .operation
                .as_ref()
                .unwrap()
                .options()
                .max_size,
            Presence::Value(number)
        );
    }
    assert_eq!(
        contents(Some(&json!({"type":"base64","value":"AA=="})), &mut 1).unwrap(),
        Some(vec![0])
    );
}

#[test]
fn whole_stream_raw_budget_counts_text_in_later_phases_and_exact_padding_bytes() {
    let mut dto = json!({"edits":[{"newResource":"file:///tmp/a","options":{"contents":{"type":"base64","value":"AA=="}}},
        {"resource":"file:///tmp/a","textEdit":{"text":"x".repeat(1024*1024-1)}}]});
    assert!(prepare(&dto).is_ok());
    dto["edits"][1]["textEdit"]["text"] = json!("x".repeat(1024 * 1024));
    assert!(prepare(&dto).is_err());
    assert!(prepare(&json!({"edits":[{"oldResource":"untitled:/target"}]})).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn original_dto_reaches_actual_joined_resource_worker_only_after_exact_authorization() {
    use crate::{
        resource_journal::{
            Capture, ExecutionLimits, ExecutionPolicy, ExecutionState, Executor, Limits,
            StagedJournal,
        },
        resource_worker::{Event, Outcome, Preparation, Transaction, Worker},
    };
    struct Job {
        dto: Value,
        journal: PathBuf,
        gate: std::sync::mpsc::Receiver<()>,
    }
    struct Prepared {
        journal: StagedJournal,
        gate: std::sync::mpsc::Receiver<()>,
        deadline: Instant,
    }
    impl Transaction for Job {
        type Prepared = Prepared;
        type Info = PathBuf;
        type Receipt = Executor;
        fn prepare(self, preparation: &Preparation) -> Result<(Prepared, PathBuf)> {
            let control = Control::new(preparation.cancel_flag(), preparation.deadline());
            let stream = Stream::parse(&self.dto)?;
            let group = prepare_group(&stream, &stream.groups()[0], &control)?;
            ensure!(
                group.edits.iter().all(|edit| edit.metadata.is_none()),
                "Test has no confirmation authority"
            );
            let capture = Capture::prepare(
                group
                    .edits
                    .into_iter()
                    .filter_map(|edit| edit.operation)
                    .collect(),
                Vec::new(),
                Limits::default(),
                &control,
            )?;
            let staged = StagedJournal::persist(capture, &self.journal, &control)?;
            let path = staged.path().to_owned();
            Ok((
                Prepared {
                    journal: staged,
                    gate: self.gate,
                    deadline: preparation.deadline(),
                },
                path,
            ))
        }
        fn commit(prepared: Prepared) -> Result<Executor> {
            prepared.gate.recv_timeout(Duration::from_secs(10))?;
            let canceled = AtomicBool::new(false); // owner retirement cannot revoke accepted effects
            let control = Control::new(&canceled, prepared.deadline);
            let mut executor = Executor::authorize(
                prepared.journal,
                ExecutionPolicy::default(),
                ExecutionLimits::default(),
                &control,
            )?;
            let outcome = executor.execute_authorized(&control);
            ensure!(
                outcome.state == ExecutionState::Completed,
                "{:?}",
                outcome.error
            );
            Ok(executor)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let target = root.path().join("猫🙂.bin");
    let dto = json!({"edits":[{"newResource":uri(&source),"options":{"contents":{"type":"base64","value":"AP+A"}}},{"oldResource":uri(&source),"newResource":uri(&target)}]});
    let (release, gate) = std::sync::mpsc::channel();
    let mut worker = Worker::<Job>::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    worker
        .start(
            1,
            Job {
                dto,
                journal: root.path().join("journal"),
                gate,
            },
            deadline,
        )
        .unwrap();
    loop {
        if let Some(Event::Prepared { id, .. }) = worker.poll() {
            assert_eq!(id, 1);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!source.exists() && !target.exists());
    assert!(worker.authorize(2).is_err());
    worker.authorize(1).unwrap();
    worker.retire(1);
    release.send(()).unwrap();
    let executor = loop {
        if let Some(Event::Finished {
            id,
            retired,
            authorized,
            result,
        }) = worker.poll()
        {
            assert_eq!(id, 1);
            assert!(retired && authorized);
            let Outcome::Committed(executor) = result.unwrap() else {
                panic!("exact authorized commit")
            };
            break executor;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(!worker.busy());
    assert!(!source.exists());
    assert_eq!(std::fs::read(&target).unwrap(), [0, 255, 128]);
    assert_eq!(executor.outcome().steps.len(), 2);
    let recovered = crate::resource_journal::ExecutionRecovery::load(
        executor.path(),
        Limits::default(),
        &control(&AtomicBool::new(false)),
    )
    .unwrap();
    recovered
        .verify_current(&control(&AtomicBool::new(false)))
        .unwrap();
}
