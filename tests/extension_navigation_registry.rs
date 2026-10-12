//! Real public/native queries share the stateful pinned registry ordering.
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};

const SOURCE: &str = "猫🙂 source\r\nnext\r\n";
const TARGET: &str = "α🙂 target\r\n";

fn until(app: &mut App, phase: &str, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut terminal = Terminal::new(TestBackend::new(150, 35)).unwrap();
        terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Registry phase {phase}: {}",
            app.message
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn command(app: &mut App, command: &str, message: &str) {
    app.execute(&format!("navigationRegistry.{command}"), Value::Null);
    until(app, message, |app| app.message == message);
}
fn query(app: &mut App, candidate: &str, winner: &str) {
    let suffix = match winner {
        "early" => "Early",
        "wildcard" => "Wildcard",
        "third" => "Third",
        "null" => "None",
        _ => unreachable!(),
    };
    let name = if candidate == "cpp" { "Cpp" } else { "Rust" };
    command(
        app,
        &format!("query{name}{suffix}"),
        &format!("API registry query;{candidate}={winner};inactive/noEdit=true"),
    );
}
fn native(app: &mut App, root: &Path, winner: &str) {
    command(app, "source", "API registry source;retained=true");
    app.execute("editor.action.revealDefinition", Value::Null);
    let target = root.join(format!("{winner}.cpp"));
    until(app, &format!("native winner {winner}"), |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(target.as_path()))
            && app.modal.is_none()
    });
    let suffix = match winner {
        "early" => "Early",
        "wildcard" => "Wildcard",
        "third" => "Third",
        _ => unreachable!(),
    };
    command(
        app,
        &format!("native{suffix}"),
        &format!("API registry native;winner={winner};collapsed/shared=true"),
    );
    assert_eq!(app.doc().cursor, if winner == "wildcard" { 7 } else { 1 });
    assert_eq!(app.doc().anchor, None);
    assert!(app.doc().secondary.is_empty());
}

#[test]
fn public_and_native_definition_share_exclusive_order_across_candidates_cached_queries_registration_and_disposal()
 {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::write(root.join("source.cpp"), SOURCE).unwrap();
    std::fs::write(root.join("candidate.rs"), "猫🙂 rust\r\n").unwrap();
    for name in ["early", "wildcard", "third"] {
        std::fs::write(root.join(format!("{name}.cpp")), TARGET).unwrap();
    }
    let settings = root.join("settings.json");
    std::fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "vscli.languageServer.enabled":false,"breadcrumbs.enabled":false,
            "files.autoSave":"off","editor.formatOnSave":false,"editor.codeActionsOnSave":{}
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(root.clone(), Profile::Linux);
    app.configure_settings(Some(settings)).unwrap();
    app.open(&root.join("source.cpp")).unwrap();
    until(&mut app, "original source open", |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_deref() == Some(root.join("source.cpp").as_path()))
    });
    app.doc_mut().insert("dirty ", false);
    let source_id = app.doc().id;
    let source_membership = app.editor_groups().active_membership().unwrap();
    let source_proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    let source_selections = app.doc().selections();
    app.extension_node = std::env::var("VSCLI_EXTENSION_NODE").unwrap_or_else(|_| "node".into());
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/navigation-registry-extension");
    app.start_extension_packages(vec![Package::read(&package).unwrap()])
        .unwrap();
    until(&mut app, "selected fixture ready", |app| {
        app.extension_host.as_ref().is_some_and(|host| {
            host.ready
                && host
                    .commands
                    .iter()
                    .any(|(_, id)| id == "navigationRegistry.prepare")
        })
    });
    command(
        &mut app,
        "prepare",
        "API registry prepared;shared/dirty=true",
    );
    let wildcard_id = app
        .documents
        .iter()
        .find(|doc| doc.path.as_deref() == Some(root.join("wildcard.cpp").as_path()))
        .unwrap()
        .id;
    let original_wildcard_memberships: Vec<_> =
        app.editor_groups().memberships(wildcard_id).collect();
    assert_eq!(original_wildcard_memberships.len(), 2);
    assert!(
        original_wildcard_memberships
            .iter()
            .all(|member| member.group != source_membership.group)
    );

    // Initial exclusive selection scans registration order before score sorting:
    // older cpp(score 10) wins over newer wildcard(score 5).
    query(&mut app, "cpp", "early");
    native(&mut app, &root, "early");
    // The inactive public rust query moves wildcard to the front of the SAME
    // registry. A separate native cache would incorrectly keep choosing early.
    query(&mut app, "rust", "wildcard");
    native(&mut app, &root, "wildcard");
    let revealed_wildcard_membership = app.editor_groups().active_membership().unwrap();
    assert_eq!(revealed_wildcard_membership.document, wildcard_id);
    assert_eq!(revealed_wildcard_membership.group, source_membership.group);
    assert!(!original_wildcard_memberships.contains(&revealed_wildcard_membership));
    query(&mut app, "cpp", "wildcard");
    native(&mut app, &root, "wildcard");
    assert_eq!(app.doc().id, wildcard_id);
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "targetClean",
        "API registry target Undo;exact=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "targetDirty",
        "API registry target Redo;exact=true",
    );
    assert_eq!(app.doc().id, wildcard_id);

    // Registration appends to the current ordered array, invalidating only the
    // candidate cache. Sorting the exclusive winner moves newer zero-score
    // third ahead of early; removing wildcard therefore exposes third.
    command(
        &mut app,
        "registerThird",
        "API registry third registered;source=retained",
    );
    query(&mut app, "cpp", "wildcard");
    native(&mut app, &root, "wildcard");
    command(
        &mut app,
        "disposeWildcard",
        "API registry wildcard disposed;source=retained",
    );
    native(&mut app, &root, "third");
    query(&mut app, "cpp", "third");
    query(&mut app, "rust", "null");
    native(&mut app, &root, "third");
    command(
        &mut app,
        "disposeThird",
        "API registry third disposed;source=retained",
    );
    native(&mut app, &root, "early");
    query(&mut app, "cpp", "early");

    command(&mut app, "source", "API registry source;retained=true");
    assert_eq!(app.doc().id, source_id);
    assert_eq!(
        app.editor_groups().active_membership(),
        Some(source_membership)
    );
    assert_eq!(app.doc().selections(), source_selections);
    let proof = app.doc().capture_save(root.join("source.cpp")).unwrap();
    assert_eq!(proof.text_epoch(), source_proof.text_epoch());
    assert_eq!(proof.save_generation(), source_proof.save_generation());
    let final_wildcard_memberships: Vec<_> = app.editor_groups().memberships(wildcard_id).collect();
    assert_eq!(final_wildcard_memberships.len(), 3);
    for member in original_wildcard_memberships
        .into_iter()
        .chain([revealed_wildcard_membership])
    {
        assert!(app.editor_groups().membership_current(member));
        assert!(final_wildcard_memberships.contains(&member));
    }
    assert_eq!(
        app.documents
            .iter()
            .find(|doc| doc.id == wildcard_id)
            .unwrap()
            .text
            .to_string(),
        format!("dirty {TARGET}")
    );
    app.execute("undo", Value::Null);
    command(
        &mut app,
        "sourceClean",
        "API registry source Undo;exact=true",
    );
    app.execute("redo", Value::Null);
    command(
        &mut app,
        "sourceDirty",
        "API registry source Redo;exact=true",
    );
    assert_eq!(app.doc().id, source_id);
    assert_eq!(
        std::fs::read(root.join("source.cpp")).unwrap(),
        SOURCE.as_bytes()
    );
    assert_eq!(
        std::fs::read(root.join("candidate.rs")).unwrap(),
        "猫🙂 rust\r\n".as_bytes()
    );
    for name in ["early", "wildcard", "third"] {
        assert_eq!(
            std::fs::read(root.join(format!("{name}.cpp"))).unwrap(),
            TARGET.as_bytes()
        );
    }
    app.execute("vscli.extensions.stop", Value::Null);
}
