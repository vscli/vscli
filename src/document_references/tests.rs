use super::*;

fn model(document: u64, resource_epoch: u64) -> ModelKey {
    ModelKey {
        document,
        resource_epoch,
    }
}
fn resource(scheme: &str, path: &str) -> ResourceKey {
    ResourceKey::new(
        &format!("{scheme}://{path}"),
        ResourceIdentity {
            scheme: scheme.into(),
            authority: String::new(),
            path: path.into(),
            query: String::new(),
            fragment: String::new(),
        },
    )
    .unwrap()
}
fn acquire(
    ledger: &mut References,
    key: ModelKey,
    path: &str,
    length: u64,
    at: Instant,
) -> Decision {
    ledger
        .acquire(
            ledger.customer_epoch(),
            key,
            resource("file", path),
            length,
            at,
        )
        .unwrap()
}

#[test]
fn genuine_duplicate_acquisitions_have_independent_original_deadlines() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let key = model(7, 3);
    let first = acquire(&mut ledger, key, "/a", 4, now).acquired.unwrap();
    let second = acquire(&mut ledger, key, "/a", 9, now + Duration::from_secs(30))
        .acquired
        .unwrap();
    assert!(second.sequence > first.sequence);
    assert_eq!(ledger.retention(key).acquisitions, 2);
    let receipt = ledger.expire_due(1, now + MAX_AGE).unwrap();
    assert_eq!(receipt.released.len(), 1);
    let original = &receipt.released[0].acquisition;
    assert_eq!(original.id, first);
    assert_eq!(original.model, key);
    assert_eq!(original.acquired_at, now);
    assert_eq!(original.captured_length_units, 4);
    assert_eq!(receipt.retained[0].captured_length_units, 9);
    assert!(ledger.contains(second));
    assert_eq!(
        ledger.next_expiry(),
        Some(now + MAX_AGE + Duration::from_secs(30))
    );
}

#[test]
fn cached_opens_never_acquire_or_extend_expiry_or_capture_later_length() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let key = model(1, 0);
    acquire(&mut ledger, key, "/a", 3, now);
    for _ in 0..100 {
        assert_eq!(ledger.cached_open(1, key).unwrap().acquisitions, 1);
    }
    assert_eq!(ledger.captured_length_units(), 3);
    assert_eq!(ledger.next_expiry(), Some(now + MAX_AGE));
    assert_eq!(ledger.cached_open(1, model(2, 0)).unwrap().acquisitions, 0);
    assert_eq!(ledger.len(), 1);
    assert!(
        ledger
            .expire_due(1, now + MAX_AGE)
            .unwrap()
            .retained
            .is_empty()
    );
}

#[test]
fn exact_deadline_and_stale_timer_do_not_release_newer_acquisition() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let old = acquire(&mut ledger, model(1, 0), "/a", 1, now)
        .acquired
        .unwrap();
    assert!(
        ledger
            .expire_acquisition(old, now + MAX_AGE - Duration::from_nanos(1))
            .released
            .is_empty()
    );
    ledger
        .remove_move_source(1, &resource("file", "/a"))
        .unwrap();
    let new = acquire(&mut ledger, model(1, 2), "/a", 2, now)
        .acquired
        .unwrap();
    assert!(
        ledger
            .expire_acquisition(old, now + MAX_AGE)
            .released
            .is_empty()
    );
    assert!(ledger.contains(new));
    let release = ledger.expire_acquisition(new, now + MAX_AGE);
    assert_eq!(release.released[0].acquisition.model, model(1, 2));
}

#[test]
fn expiry_scans_deadlines_not_insertion_order() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let later = acquire(
        &mut ledger,
        model(1, 0),
        "/a",
        1,
        now + Duration::from_secs(10),
    )
    .acquired
    .unwrap();
    let earlier = acquire(&mut ledger, model(2, 0), "/b", 1, now)
        .acquired
        .unwrap();
    assert_eq!(ledger.next_expiry(), Some(now + MAX_AGE));
    let release = ledger.expire_due(1, now + MAX_AGE).unwrap();
    assert_eq!(release.released[0].acquisition.id, earlier);
    assert!(ledger.contains(later));
}

#[test]
fn length_limit_is_strict_and_oldest_acquisition_is_evicted() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let first = acquire(&mut ledger, model(1, 0), "/a", MAX_LENGTH_UNITS - 1, now)
        .acquired
        .unwrap();
    let second = acquire(&mut ledger, model(2, 0), "/b", 1, now)
        .acquired
        .unwrap();
    assert_eq!(ledger.captured_length_units(), MAX_LENGTH_UNITS);
    let receipt = acquire(&mut ledger, model(3, 0), "/c", 1, now);
    assert_eq!(receipt.released.len(), 1);
    assert_eq!(receipt.released[0].acquisition.id, first);
    assert_eq!(receipt.released[0].cause, ReleaseCause::LengthLimit);
    assert!(ledger.contains(second));
    assert_eq!(ledger.captured_length_units(), 2);
}

#[test]
fn oversized_length_releases_all_older_entries_and_itself_without_overflow() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    acquire(&mut ledger, model(1, 0), "/zero", 0, now);
    acquire(&mut ledger, model(2, 0), "/a", 10, now);
    let receipt = acquire(&mut ledger, model(3, 0), "/huge", u64::MAX, now);
    assert_eq!(receipt.released.len(), 3);
    assert_eq!(
        receipt.released[2].acquisition.id,
        receipt.acquired.unwrap()
    );
    assert!(receipt.retained.is_empty());
    assert!(ledger.is_empty());
    assert_eq!(ledger.captured_length_units(), 0);
}

#[test]
fn count_sixty_evicts_ten_and_allows_fifty_one_through_fifty_nine() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    for id in 1..60 {
        assert!(
            acquire(&mut ledger, model(id, 0), "/same", 0, now)
                .released
                .is_empty()
        );
    }
    assert_eq!(ledger.len(), 59);
    let receipt = acquire(&mut ledger, model(60, 0), "/same", 0, now);
    assert_eq!(ledger.len(), 50);
    assert_eq!(receipt.released.len(), 10);
    assert_eq!(
        receipt
            .released
            .iter()
            .map(|release| release.acquisition.model.document)
            .collect::<Vec<_>>(),
        (1..=10).collect::<Vec<_>>()
    );
    assert!(
        receipt
            .released
            .iter()
            .all(|release| release.cause == ReleaseCause::CountLimit)
    );
}

#[test]
fn length_eviction_precedes_count_trigger() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    acquire(&mut ledger, model(1, 0), "/a", MAX_LENGTH_UNITS, now);
    for id in 2..60 {
        acquire(&mut ledger, model(id, 0), "/zero", 0, now);
    }
    let receipt = acquire(&mut ledger, model(60, 0), "/one", 1, now);
    assert_eq!(receipt.released.len(), 1);
    assert_eq!(receipt.released[0].cause, ReleaseCause::LengthLimit);
    assert_eq!(ledger.len(), 59);
}

#[test]
fn move_removes_source_descendants_and_duplicates_but_keeps_destination_and_siblings() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    for (id, path) in [
        (1, "/src"),
        (2, "/src/a"),
        (3, "/src/a"),
        (4, "/src-other/a"),
        (5, "/dst/a"),
    ] {
        acquire(&mut ledger, model(id, id), path, id, now);
    }
    let receipt = ledger
        .remove_move_source(1, &resource("file", "/src"))
        .unwrap();
    assert_eq!(receipt.released.len(), 3);
    assert!(
        receipt
            .released
            .iter()
            .all(|release| release.cause == ReleaseCause::MoveSource)
    );
    assert_eq!(
        receipt
            .retained
            .iter()
            .map(|entry| entry.model.document)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    assert_eq!(ledger.captured_length_units(), 9);
}

#[test]
fn delete_matches_alias_identity_root_and_exact_uri_namespace() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let canonical = resource("file", "/root/a");
    let alias = ResourceKey::new("file:///root/./a", canonical.identity().clone()).unwrap();
    ledger.acquire(1, model(1, 0), alias, 1, now).unwrap();
    let mut queried = canonical.identity().clone();
    queried.query = "revision=1".into();
    ledger
        .acquire(
            1,
            model(2, 0),
            ResourceKey::new("file:///root/a?revision=1", queried).unwrap(),
            1,
            now,
        )
        .unwrap();
    acquire(&mut ledger, model(3, 0), "/root/a", 1, now);
    let receipt = ledger.remove_deleted_resource(1, &canonical).unwrap();
    assert_eq!(receipt.released.len(), 2);
    assert_eq!(
        receipt.released[0].acquisition.resource.uri(),
        "file:///root/./a"
    );
    assert_eq!(ledger.len(), 1);
    assert!(
        ledger
            .remove_deleted_resource(1, &resource("file", "/"))
            .unwrap()
            .released
            .is_empty()
    );
    let query_root = ResourceKey::new(
        "file:///?revision=1",
        ResourceIdentity {
            query: "revision=1".into(),
            ..resource("file", "/").identity().clone()
        },
    )
    .unwrap();
    assert_eq!(
        ledger
            .remove_deleted_resource(1, &query_root)
            .unwrap()
            .released
            .len(),
        1
    );
}

#[test]
fn untitled_revert_is_exact_and_does_not_remove_descendants_or_files() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    for (id, scheme, path) in [
        (1, "untitled", "Untitled-1"),
        (2, "untitled", "Untitled-1/child"),
        (3, "file", "Untitled-1"),
    ] {
        ledger
            .acquire(1, model(id, 0), resource(scheme, path), 1, now)
            .unwrap();
    }
    let receipt = ledger
        .untitled_revert(1, model(1, 0), &resource("untitled", "Untitled-1"))
        .unwrap();
    assert_eq!(receipt.released.len(), 1);
    assert_eq!(receipt.released[0].cause, ReleaseCause::UntitledRevert);
    assert_eq!(ledger.len(), 2);
    assert_eq!(
        ledger
            .untitled_revert(1, model(3, 0), &resource("file", "Untitled-1"))
            .unwrap_err(),
        Error::NotUntitled
    );
    assert_eq!(ledger.len(), 2);
}

#[test]
fn old_untitled_revert_cannot_release_new_binding_with_same_uri_or_native_id() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    let uri = resource("untitled", "Untitled-1");
    ledger.acquire(1, model(8, 0), uri.clone(), 4, now).unwrap();
    ledger.acquire(1, model(8, 2), uri.clone(), 5, now).unwrap();
    let receipt = ledger.untitled_revert(1, model(8, 0), &uri).unwrap();
    assert_eq!(receipt.released.len(), 1);
    assert_eq!(receipt.released[0].acquisition.model, model(8, 0));
    assert_eq!(receipt.retained[0].model, model(8, 2));
    assert!(
        ledger
            .untitled_revert(1, model(8, 0), &uri)
            .unwrap()
            .released
            .is_empty()
    );
}

#[test]
fn teardown_drains_every_reference_and_replacement_preserves_monotonic_ids() {
    let now = Instant::now();
    let mut ledger = References::new(7).unwrap();
    let mut last = None;
    for id in 1..=5 {
        last = acquire(&mut ledger, model(id, 0), "/a", id, now).acquired;
    }
    let receipt = ledger.retire_customer(7).unwrap();
    assert_eq!(receipt.released.len(), 5);
    assert!(receipt.retained.is_empty());
    assert_eq!(ledger.captured_length_units(), 0);
    assert!(ledger.next_expiry().is_none());
    assert_eq!(
        ledger.cached_open(7, model(1, 0)).unwrap_err(),
        Error::RetiredCustomer
    );
    assert!(ledger.retire_customer(7).unwrap().released.is_empty());
    ledger.replace_customer(7, 8).unwrap();
    let new = acquire(&mut ledger, model(1, 2), "/a", 9, now)
        .acquired
        .unwrap();
    assert!(new.sequence > last.unwrap().sequence);
    assert_eq!(new.customer_epoch, 8);
    assert!(
        ledger
            .expire_acquisition(last.unwrap(), now + MAX_AGE)
            .released
            .is_empty()
    );
    assert_eq!(ledger.retire_customer(7).unwrap_err(), Error::StaleCustomer);
    assert!(ledger.contains(new));
}

#[test]
fn same_native_id_different_resource_epochs_have_separate_retention_evidence() {
    let now = Instant::now();
    let mut ledger = References::new(1).unwrap();
    acquire(&mut ledger, model(9, 0), "/before", 4, now);
    acquire(&mut ledger, model(9, 2), "/after", 6, now);
    let receipt = ledger
        .remove_deleted_resource(1, &resource("file", "/before"))
        .unwrap();
    assert_eq!(receipt.released[0].acquisition.model, model(9, 0));
    assert_eq!(receipt.retained[0].model, model(9, 2));
    assert_eq!(ledger.retention(model(9, 0)).acquisitions, 0);
    assert_eq!(ledger.retention(model(9, 2)).acquisitions, 1);
}

#[test]
fn refused_admissions_and_stale_customer_actions_preserve_original_inventory() {
    let now = Instant::now();
    assert!(matches!(References::new(0), Err(Error::InvalidCustomer)));
    let mut ledger = References::new(1).unwrap();
    let original = acquire(&mut ledger, model(1, 0), "/a", 5, now)
        .acquired
        .unwrap();
    assert_eq!(
        ledger
            .acquire(2, model(2, 0), resource("file", "/b"), 1, now)
            .unwrap_err(),
        Error::StaleCustomer
    );
    assert_eq!(
        ledger
            .acquire(1, model(0, 0), resource("file", "/b"), 1, now)
            .unwrap_err(),
        Error::InvalidModel
    );
    assert_eq!(
        ledger.replace_customer(1, 1).unwrap_err(),
        Error::InvalidCustomer
    );
    assert_eq!(
        ledger.expire_due(2, now + MAX_AGE).unwrap_err(),
        Error::StaleCustomer
    );
    assert_eq!(
        ledger
            .remove_deleted_resource(2, &resource("file", "/a"))
            .unwrap_err(),
        Error::StaleCustomer
    );
    ledger.last_sequence = u64::MAX;
    assert_eq!(
        ledger
            .acquire(1, model(2, 0), resource("file", "/b"), 1, now)
            .unwrap_err(),
        Error::IdentityExhausted
    );
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.captured_length_units(), 5);
    assert!(ledger.contains(original));
}

#[test]
fn resource_storage_is_bounded_and_namespace_fields_are_preserved() {
    let key = resource("file", "/a");
    assert_eq!(
        ResourceKey::new(&"a".repeat(MAX_RESOURCE_BYTES + 1), key.identity().clone()).unwrap_err(),
        Error::InvalidResource
    );
    let mut invalid = key.identity().clone();
    invalid.path = "a\0b".into();
    assert_eq!(
        ResourceKey::new("file:///a", invalid).unwrap_err(),
        Error::InvalidResource
    );
    let mut other = key.identity().clone();
    other.authority = "remote".into();
    assert!(!key.equal_or_descendant_of(&ResourceKey::new("file://remote/a", other).unwrap()));
    let mut other = key.identity().clone();
    other.fragment = "x".into();
    assert!(!key.same_resource(&ResourceKey::new("file:///a#x", other).unwrap()));
}
