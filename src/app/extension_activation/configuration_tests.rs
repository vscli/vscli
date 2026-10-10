//! Deterministic publication and late-reply checks, including hidden models.
use super::*;
use crate::editing_profile::{ProfileId, TypingOptions};

fn installed(root: &Path) -> crate::extension_store::Installed {
    crate::extension_store::Installed {
        id: "test.native".into(),
        version: "1.0.0".into(),
        path: root.into(),
        manifest: json!({"contributes":{"languages":[{"id":"cpp","configuration":"typing.json"}]}}),
        source: "native publication fixture".into(),
        sha256: "fixture-archive".into(),
        compatibility: "native data".into(),
    }
}
fn catalog(root: &Path) -> crate::language_configuration::Catalog {
    std::fs::write(
        root.join("typing.json"),
        r#"{"autoClosingPairs":[["<",">"]]}"#,
    )
    .unwrap();
    let catalog = crate::language_configuration::Catalog::load(
        &[installed(root)],
        &Preferences::default(),
        &Preferences::default(),
    );
    assert!(catalog.warnings.is_empty(), "{:?}", catalog.warnings);
    assert!(catalog.for_language("cpp").is_some());
    catalog
}
fn snapshot(catalog: crate::language_configuration::Catalog) -> Snapshot {
    Snapshot {
        global: Preferences::default(),
        workspace: Preferences::default(),
        catalog: BTreeMap::new(),
        matched: BTreeSet::new(),
        notices: Vec::new(),
        language_configurations: catalog,
    }
}
fn app(root: &Path) -> App {
    let mut app = App::new(root.into(), Profile::Linux);
    app.activation.configured = true;
    app.activation.loaded = true;
    app.activation.config_root = Some(root.join("config"));
    app.extensions_directory = Some(root.join("store"));
    for (name, hidden) in [("visible.cpp", false), ("hidden.cpp", true)] {
        let mut document = Document::from_text("猫🙂 ");
        document.path = Some(root.join(name));
        if hidden {
            app.hidden_documents.push(document);
        } else {
            app.documents.push(document);
        }
    }
    app
}
fn options() -> TypingOptions {
    TypingOptions {
        profile: ProfileId::Cpp,
        ..TypingOptions::default()
    }
}
fn job(app: &mut App) -> std::sync::mpsc::SyncSender<Result<Output, String>> {
    let (sender, receiver) = sync_channel(1);
    app.activation.job = Some(Job {
        command: None,
        generation: app.activation.generation,
        control: app.activation.control,
        extension_epoch: None,
        receiver,
    });
    sender
}

#[test]
fn publication_updates_hidden_models_and_a_b_a_cannot_revive_owned_closers() {
    let root = tempfile::tempdir().unwrap();
    let catalog = catalog(root.path());
    let mut app = app(root.path());
    let ids = [app.documents[0].id, app.hidden_documents[0].id];
    app.publish_activation_catalog(snapshot(catalog.clone()));
    for document in app.documents.iter_mut().chain(&mut app.hidden_documents) {
        assert!(document.language_configuration().is_some());
        document.move_to(3, false);
        document.type_character('<', options(), false).unwrap();
        assert_eq!(document.text.to_string(), "猫🙂 <>");
    }
    let epochs = [
        app.documents[0].text_epoch(),
        app.hidden_documents[0].text_epoch(),
    ];
    app.publish_activation_catalog(snapshot(Default::default()));
    app.publish_activation_catalog(snapshot(catalog));
    assert_eq!(
        [
            app.documents[0].text_epoch(),
            app.hidden_documents[0].text_epoch()
        ],
        epochs
    );
    for document in app.documents.iter_mut().chain(&mut app.hidden_documents) {
        document.undo();
        document.redo();
        document.type_character('>', options(), false).unwrap();
        assert_eq!(document.text.to_string(), "猫🙂 <>>");
    }
    assert_eq!([app.documents[0].id, app.hidden_documents[0].id], ids);
    assert!(app.extension_host.is_none());
}

#[test]
fn newer_enablement_intent_rejects_a_held_older_catalog() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let sender = job(&mut app);
    app.set_extension_enabled("test.native", false, Scope::Global)
        .unwrap();
    sender
        .send(Ok(Output::Loaded(
            Box::new(snapshot(catalog(root.path()))),
            None,
        )))
        .unwrap();
    assert!(app.poll_extension_activation());
    assert!(app.language_configuration("cpp").is_none());
    assert!(app.activation.job.is_none());
    assert!(app.activation.change.is_some());
    assert!(app.activation.refresh);
    assert_eq!(
        app.language_configuration_catalog_status(),
        "language configuration loading"
    );
    assert_eq!(app.doc().text.to_string(), "猫🙂 ");
}

#[test]
fn superseded_generation_cannot_publish_a_held_catalog() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let sender = job(&mut app);
    app.activation.generation += 1;
    sender
        .send(Ok(Output::Loaded(
            Box::new(snapshot(catalog(root.path()))),
            None,
        )))
        .unwrap();
    app.poll_extension_activation();
    assert!(app.language_configuration("cpp").is_none());
    assert!(app.activation.job.is_none());
    assert_eq!(app.doc().text.to_string(), "猫🙂 ");
}

#[test]
fn failed_catalog_refresh_retires_configuration_without_losing_hidden_text_or_history() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.publish_activation_catalog(snapshot(catalog(root.path())));
    app.hidden_documents[0].move_to(3, false);
    app.hidden_documents[0]
        .type_character('<', options(), false)
        .unwrap();
    let id = app.hidden_documents[0].id;
    let sender = job(&mut app);
    sender
        .send(Err("Fixture registry no longer readable".into()))
        .unwrap();
    app.poll_extension_activation();
    assert!(app.hidden_documents[0].language_configuration().is_none());
    assert_eq!(
        app.language_configuration_catalog_status(),
        "language configuration unavailable"
    );
    assert_eq!(app.hidden_documents[0].text.to_string(), "猫🙂 <>");
    app.hidden_documents[0].undo();
    assert_eq!(app.hidden_documents[0].text.to_string(), "猫🙂 ");
    app.hidden_documents[0].redo();
    assert_eq!(app.hidden_documents[0].text.to_string(), "猫🙂 <>");
    assert_eq!(app.hidden_documents[0].id, id);
}

#[test]
fn extension_rows_distinguish_loaded_data_from_an_update_waiting_for_publication() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    app.publish_activation_catalog(snapshot(catalog(root.path())));
    let mut installed = installed(root.path());
    assert_eq!(
        app.installed_extension_activation_status(&installed),
        "language configuration loaded; code disabled"
    );
    assert_eq!(
        app.language_configuration_catalog_status(),
        "language configuration ready"
    );
    installed.version = "2.0.0".into();
    installed.sha256 = "updated-archive".into();
    assert_eq!(
        app.installed_extension_activation_status(&installed),
        "language configuration loading"
    );
    let updated = crate::language_configuration::Catalog::load(
        &[installed.clone()],
        &Preferences::default(),
        &Preferences::default(),
    );
    app.publish_activation_catalog(snapshot(updated));
    assert_eq!(
        app.installed_extension_activation_status(&installed),
        "language configuration loaded; code disabled"
    );
    assert!(app.extension_host.is_none());
}
