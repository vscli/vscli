use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{
    app::App,
    extensions::{ActivationRequest, Client, Package},
    keys::Profile,
};
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "Timed out: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn package(root: &Path, name: &str, source: &str, dependencies: &[&str]) -> Package {
    let path = root.join(name);
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("package.json"), serde_json::to_vec(&json!({"publisher":"test", "name":name,"version":"1","main":"main.cjs","extensionDependencies":dependencies})).unwrap()).unwrap();
    std::fs::write(path.join("main.cjs"), source).unwrap();
    Package::read(&path).unwrap()
}
fn activate(app: &mut App, request: ActivationRequest) {
    app.extension_host
        .as_mut()
        .unwrap()
        .activate(&request, &app.documents, app.active, &app.settings)
        .unwrap();
    until(app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| !host.activating())
    });
}
#[test]
fn lazy_selected_snapshot_and_append_share_exports_native_identity_versions_and_undo() {
    let root = tempfile::tempdir().unwrap();
    let a = package(
        root.path(),
        "a",
        r#"
      const vscode = require('vscode'); let count = 0;
      exports.activate = context => {
        context.subscriptions.push(vscode.commands.registerCommand('a.count', () => vscode.window.showInformationMessage(`count=${count}`)));
        return { count: ++count };
      };
    "#,
        &[],
    );
    let b = package(
        root.path(),
        "b",
        r#"
      const vscode = require('vscode');
      exports.activate = context => {
        const a = vscode.extensions.getExtension('test.a');
        context.subscriptions.push(vscode.commands.registerCommand('b.edit', async () => {
          const editor = vscode.window.activeTextEditor;
          const applied = await editor.edit(edit => edit.insert(new vscode.Position(0, 0), `shared${a.exports.count}:`));
          await vscode.window.showInformationMessage(`applied=${applied}`);
        }));
      };
    "#,
        &["test.a"],
    );
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("猫\r\n🙂", false);
    let identity = app.doc().id;
    let prepared = Client::prepare(&app.documents, app.active, &app.settings)
        .unwrap()
        .with_activation(vec![])
        .unwrap();
    app.extension_host =
        Some(Client::start_many_prepared("node", &[a], root.path(), prepared).unwrap());
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    assert!(app.extension_host.as_ref().unwrap().commands.is_empty());
    assert_eq!(
        app.extension_host.as_ref().unwrap().activation_states["test.a"],
        "dormant"
    );
    activate(
        &mut app,
        ActivationRequest {
            additions: vec![],
            targets: vec!["test.a".into()],
            owner: Some("test.a".into()),
        },
    );
    activate(
        &mut app,
        ActivationRequest {
            additions: vec![b],
            targets: vec!["test.b".into()],
            owner: Some("test.b".into()),
        },
    );
    assert_eq!(
        app.extension_host.as_ref().unwrap().activation_states["test.b"],
        "active",
        "{}",
        app.message
    );
    app.execute("b.edit", Value::Null);
    until(&mut app, |app| app.message == "applied=true");
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), "shared1:猫\r\n🙂");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "猫\r\n🙂");
    activate(
        &mut app,
        ActivationRequest {
            additions: vec![],
            targets: vec!["test.a".into()],
            owner: None,
        },
    );
    app.execute("a.count", Value::Null);
    until(&mut app, |app| app.message == "count=1");
}
#[test]
fn failed_lazy_activation_keeps_prior_commands_and_discards_partial_owner_registrations() {
    let root = tempfile::tempdir().unwrap();
    let a = package(
        root.path(),
        "a",
        r#"const vscode = require('vscode'); exports.activate = context => {
      context.subscriptions.push(vscode.commands.registerCommand('a.retained', () => vscode.window.showInformationMessage('retained')));
    };"#,
        &[],
    );
    let b = package(
        root.path(),
        "b",
        r#"const vscode = require('vscode'); exports.activate = () => {
      vscode.commands.registerCommand('b.partial', () => {}); throw new Error('qualified activation failure');
    };"#,
        &[],
    );
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.extension_host = Some(
        Client::start_many(
            "node",
            &[a],
            root.path(),
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap(),
    );
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    activate(
        &mut app,
        ActivationRequest {
            additions: vec![b],
            targets: vec!["test.b".into()],
            owner: None,
        },
    );
    let host = app.extension_host.as_ref().unwrap();
    assert_eq!(host.activation_states["test.b"], "failed");
    assert!(!host.commands.iter().any(|(_, id)| id == "b.partial"));
    app.execute("a.retained", Value::Null);
    until(&mut app, |app| app.message == "retained");
}
