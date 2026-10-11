use super::*;

fn prepare(doc: &Document, edits: Vec<(Range<usize>, String)>) -> WorkspaceTextPrepared {
    doc.capture_workspace_source()
        .unwrap()
        .prepare(
            edits,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap()
}
fn apply(doc: &mut Document, edits: Vec<(Range<usize>, String)>) -> Arc<WorkspaceHistoryGroup> {
    let item = prepare(doc, edits);
    let group = WorkspaceHistoryGroup::new(1, std::slice::from_ref(&item)).unwrap();
    doc.prepare_workspace_commit(item, group.clone(), MAX_WORKSPACE_METADATA)
        .unwrap()
        .publish();
    group
}

#[test]
fn prequeue_edit_undo_and_accepted_baseline_each_retire_original_source() {
    let mut doc = Document::from_text("α😀 original\r\n");
    let before = doc.capture_workspace_source().unwrap();
    let item = prepare(&doc, vec![(0..1, "猫".into())]);
    let group = WorkspaceHistoryGroup::new(1, std::slice::from_ref(&item)).unwrap();
    doc.insert("later", false);
    doc.undo();
    assert_eq!(doc.text.to_string(), "α😀 original\r\n");
    assert!(!before.current(&doc));
    assert!(
        doc.prepare_workspace_commit(item, group, MAX_WORKSPACE_METADATA)
            .is_err()
    );
    doc.redo();
    assert_eq!(doc.text.to_string(), "laterα😀 original\r\n");
    let before = doc.capture_workspace_source().unwrap();
    doc.saved_revision = doc.revision;
    doc.save_generation += 1;
    assert!(!before.current(&doc));
}

#[test]
fn source_capture_survives_navigation_but_maps_each_shared_reversed_selection() {
    let mut doc = Document::from_text("α😀 xyz\r\nlast\r\n");
    doc.activate_view(10);
    doc.set_selections(vec![Selection {
        cursor: 2,
        anchor: Some(6),
        desired_column: None,
    }]);
    doc.activate_view(20);
    doc.set_selections(vec![Selection {
        cursor: 6,
        anchor: Some(2),
        desired_column: None,
    }]);
    let source = doc.capture_workspace_source().unwrap();
    doc.move_to(5, false);
    doc.activate_view(10);
    assert!(source.current(&doc));
    let group = apply(&mut doc, vec![(0..0, "Q".into())]);
    assert_eq!(doc.view_state(Some(10)).cursor, 3);
    assert_eq!(doc.view_state(Some(10)).anchor, Some(7));
    assert_eq!(doc.view_state(Some(20)).cursor, 6);
    assert_eq!(doc.view_state(Some(20)).anchor, None);
    assert_eq!(doc.id, group.members[0].document);
    assert!(doc.dirty());
    doc.prepare_workspace_history(
        &group,
        WorkspaceHistoryDirection::Undo,
        MAX_WORKSPACE_METADATA,
    )
    .unwrap()
    .publish();
    assert_eq!(doc.text.to_string(), "α😀 xyz\r\nlast\r\n");
    assert_eq!(doc.view_state(Some(10)).cursor, 2);
    assert_eq!(doc.view_state(Some(10)).anchor, Some(6));
    assert_eq!(doc.view_state(Some(20)).cursor, 5);
    doc.prepare_workspace_history(
        &group,
        WorkspaceHistoryDirection::Redo,
        MAX_WORKSPACE_METADATA,
    )
    .unwrap()
    .publish();
    assert_eq!(doc.text.to_string(), "Qα😀 xyz\r\nlast\r\n");
}

#[test]
fn grouped_checkpoint_is_not_silently_consumed_by_direct_document_undo() {
    let mut doc = Document::from_text("before");
    let group = apply(&mut doc, vec![(0..6, "after".into())]);
    let epoch = doc.text_epoch();
    doc.undo();
    assert_eq!(doc.text.to_string(), "after");
    assert_eq!(doc.text_epoch(), epoch);
    doc.prepare_workspace_history(
        &group,
        WorkspaceHistoryDirection::Undo,
        MAX_WORKSPACE_METADATA,
    )
    .unwrap()
    .publish();
    let epoch = doc.text_epoch();
    doc.redo();
    assert_eq!(doc.text.to_string(), "before");
    assert_eq!(doc.text_epoch(), epoch);
}

#[test]
fn no_op_keeps_redo_epoch_selection_and_baseline() {
    let mut doc = Document::from_text("a😀\r\n");
    doc.insert("x", false);
    doc.undo();
    let epoch = doc.text_epoch();
    let saved = doc.saved_revision;
    let redo = doc.redo.last().unwrap().history_identity.clone();
    let item = prepare(&doc, vec![(0..1, "a".into())]);
    assert!(!item.changed());
    assert_eq!(doc.text_epoch(), epoch);
    assert_eq!(doc.saved_revision, saved);
    assert!(Arc::ptr_eq(
        &redo,
        &doc.redo.last().unwrap().history_identity
    ));
    doc.redo();
    assert_eq!(doc.text.to_string(), "xa😀\r\n");
}

#[test]
fn prepared_lease_refuses_exhausted_counters_and_metadata_without_text_publication() {
    let mut doc = Document::from_text("original");
    let item = prepare(&doc, vec![(0..0, "x".into())]);
    let group = WorkspaceHistoryGroup::new(1, std::slice::from_ref(&item)).unwrap();
    assert!(doc.prepare_workspace_commit(item, group, 1).is_err());
    assert_eq!(doc.text.to_string(), "original");
    assert!(doc.undo.is_empty());
    doc.text_epoch = u64::MAX;
    assert!(
        doc.capture_workspace_source()
            .unwrap()
            .prepare(
                vec![(0..0, "x".into())],
                &AtomicBool::new(false),
                Instant::now() + Duration::from_secs(5)
            )
            .is_err()
    );
    assert_eq!(doc.text.to_string(), "original");
    assert!(doc.undo.is_empty());
}

#[test]
fn dropping_a_prepared_commit_preserves_model_history_and_owned_save_baseline() {
    let mut doc = Document::from_text("original\r\n");
    let before = doc.capture_workspace_source().unwrap();
    let item = prepare(&doc, vec![(0..0, "x".into())]);
    let group = WorkspaceHistoryGroup::new(1, std::slice::from_ref(&item)).unwrap();
    drop(
        doc.prepare_workspace_commit(item, group, MAX_WORKSPACE_METADATA)
            .unwrap(),
    );
    assert!(before.current(&doc));
    assert_eq!(doc.text.to_string(), "original\r\n");
    assert!(doc.undo.is_empty());
}
