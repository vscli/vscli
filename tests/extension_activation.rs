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

fn install(
    root: &Path,
    store: &vscli::extension_store::Store,
    name: &str,
    source: Option<&str>,
    extra: Value,
) {
    use std::io::Write;
    let mut manifest = json!({"publisher":"test","name":name,"version":"1.0.0"});
    if source.is_some() {
        manifest["main"] = "main.cjs".into();
    }
    for (key, value) in extra.as_object().unwrap() {
        manifest[key] = value.clone();
    }
    let path = root.join(format!("{name}.vsix"));
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    writer
        .start_file(
            "extension/package.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer
        .write_all(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    if let Some(source) = source {
        writer
            .start_file(
                "extension/main.cjs",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(source.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
    store.install(&path).unwrap();
}
fn configured(root: &Path, config: &Path, store: &vscli::extension_store::Store) -> App {
    let mut app = App::new(root.into(), Profile::Linux);
    app.extensions_directory = Some(store.root().into());
    app.configure_extension_activation(Some(config));
    until(&mut app, |app| {
        app.extension_activation_status("test.a") == "disabled"
    });
    app
}
fn enable(app: &mut App, id: &str, scope: vscli::extension_activation::Scope) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        match app.set_extension_enabled(id, true, scope) {
            Ok(()) => break,
            Err(error) if error.to_string().contains("worker is already running") => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("{error:#}"),
        }
    }
    until(app, |app| app.extension_enabled(id));
}
#[test]
fn installed_command_enablement_is_dormant_without_node_and_failed_start_preserves_native_edits() {
    use vscli::extension_activation::Scope;
    let root = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let store = vscli::extension_store::Store::new(root.path().join("store"));
    install(
        root.path(),
        &store,
        "a",
        Some("exports.activate=()=>{}"),
        json!({"contributes":{"commands":[{"command":"a.run","title":"Dormant command"}],"keybindings":[{"key":"f8","command":"a.run"}]}}),
    );
    let mut app = configured(root.path(), config.path(), &store);
    app.extension_node = "vscli-test-node-is-unavailable".into();
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("native 猫", false);
    let identity = app.doc().id;
    assert!(
        !app.palette_items("Dormant command")
            .iter()
            .any(|(_, id)| *id == "a.run")
    );
    enable(&mut app, "test.a", Scope::Global);
    assert!(
        app.palette_items("Dormant command")
            .iter()
            .any(|(_, id)| *id == "a.run")
    );
    assert_eq!(app.keymap.shortcut("a.run"), "f8");
    assert!(app.extension_host.is_none());
    app.execute("a.run", Value::Null);
    until(&mut app, |app| {
        app.message.contains("automatic retry is paused")
    });
    assert!(app.extension_host.is_none());
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), "native 猫");
    app.doc_mut().insert("!", false);
    assert_eq!(app.doc().text.to_string(), "native 猫!");
}
#[test]
fn remembered_dependencies_require_separate_grants_and_shared_commands_keep_args_and_undo() {
    use vscli::extension_activation::Scope;
    let root = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let store = vscli::extension_store::Store::new(root.path().join("store"));
    install(
        root.path(),
        &store,
        "a",
        Some(
            r#"const vscode=require('vscode'); exports.activate=context=>{
      const dependency=vscode.extensions.getExtension('test.b');
      context.subscriptions.push(vscode.commands.registerCommand('a.run',async args=>{
        const editor=vscode.window.activeTextEditor;
        const ok=await editor.edit(edit=>edit.insert(new vscode.Position(0,0),`${dependency.exports.prefix}:${JSON.stringify(args)}:`));
        await vscode.window.showInformationMessage(`automatic edit=${ok}`);
      }));
    };"#,
        ),
        json!({"extensionDependencies":["test.b"],"contributes":{"commands":[{"command":"a.run","title":"Remembered edit"}]}}),
    );
    install(
        root.path(),
        &store,
        "b",
        Some("exports.activate=()=>({prefix:'shared'})"),
        json!({}),
    );
    let mut app = configured(root.path(), config.path(), &store);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("猫🙂", false);
    let identity = app.doc().id;
    enable(&mut app, "test.a", Scope::Workspace);
    app.execute("a.run", json!(["x", 2]));
    until(&mut app, |app| app.message.contains("test.b is disabled"));
    assert!(app.extension_host.is_none());
    assert_eq!(app.doc().text.to_string(), "猫🙂");
    enable(&mut app, "test.b", Scope::Global);
    app.execute("a.run", json!(["x", 2]));
    until(&mut app, |app| app.message == "automatic edit=true");
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), "shared:[\"x\",2]:猫🙂");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "猫🙂");
    let session = app.extension_host.as_ref().unwrap().session;
    app.execute("a.run", Value::Null);
    until(&mut app, |app| {
        app.doc().text.to_string().starts_with("shared:undefined:")
    });
    assert_eq!(app.extension_host.as_ref().unwrap().session, session);
    app.execute("vscli.extensions.disableWorkspace", json!({"id":"test.a"}));
    until(&mut app, |app| {
        !app.extension_enabled("test.a") && app.extension_host.is_none()
    });
    assert!(
        !app.palette_items("Remembered edit")
            .iter()
            .any(|(_, id)| *id == "a.run")
    );
    drop(app);
    let mut reopened = App::new(root.path().into(), Profile::Linux);
    reopened.extensions_directory = Some(store.root().into());
    reopened.configure_extension_activation(Some(config.path()));
    until(&mut reopened, |app| {
        app.extension_activation_status("test.a") == "disabled"
    });
    assert!(!reopened.extension_enabled("test.a"));
    assert!(reopened.extension_enabled("test.b"));
    assert!(reopened.extension_host.is_none());
}
#[test]
fn language_and_workspace_events_activate_granted_code_while_declarative_events_need_no_node() {
    use vscli::extension_activation::{Scope, state::Paths};
    let root = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let store = vscli::extension_store::Store::new(root.path().join("store"));
    install(
        root.path(),
        &store,
        "a",
        Some("exports.activate=()=>require('node:fs').writeFileSync('language-activated','yes')"),
        json!({"activationEvents":["onLanguage:rust"]}),
    );
    install(
        root.path(),
        &store,
        "b",
        None,
        json!({"activationEvents":["onStartupFinished"]}),
    );
    let mut app = configured(root.path(), config.path(), &store);
    app.extension_node = "vscli-test-node-is-unavailable".into();
    enable(&mut app, "test.b", Scope::Global);
    until(&mut app, |app| {
        app.message
            .contains("Declarative extension test.b needs no JavaScript")
    });
    assert!(app.extension_host.is_none());
    enable(&mut app, "test.a", Scope::Global);
    assert!(app.extension_host.is_none());
    app.extension_node = "node".into();
    let file = root.path().join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    app.open(&file).unwrap();
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.owner_active("test.a"))
    });
    assert_eq!(
        std::fs::read_to_string(root.path().join("language-activated")).unwrap(),
        "yes"
    );
    drop(app);
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("marker.cfg"), "").unwrap();
    install(
        root.path(),
        &store,
        "c",
        Some("exports.activate=()=>require('node:fs').writeFileSync('workspace-activated','yes')"),
        json!({"activationEvents":["workspaceContains:**/*.cfg"]}),
    );
    let paths = Paths::new(Some(config.path()), other.path()).unwrap();
    vscli::extension_activation::state::change(
        paths.path(Scope::Workspace).unwrap(),
        "test.c",
        true,
    )
    .unwrap();
    let mut reopened = App::new(other.path().into(), Profile::Linux);
    reopened.extensions_directory = Some(store.root().into());
    reopened.configure_extension_activation(Some(config.path()));
    until(&mut reopened, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.owner_active("test.c"))
    });
    assert_eq!(
        std::fs::read_to_string(other.path().join("workspace-activated")).unwrap(),
        "yes"
    );
}

#[test]
fn held_activation_cannot_dispatch_a_command_into_a_later_native_document() {
    use vscli::extension_activation::Scope;
    let root = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let store = vscli::extension_store::Store::new(root.path().join("store"));
    install(
        root.path(),
        &store,
        "a",
        Some(
            r#"const vscode=require('vscode'), fs=require('node:fs');
exports.activate=async context=>{
 fs.writeFileSync('activation-started','yes');
 await new Promise(resolve=>{const timer=setInterval(()=>{if(fs.existsSync('activation-release')){clearInterval(timer);resolve();}},5);});
 context.subscriptions.push(vscode.commands.registerCommand('a.run',async()=>{
  const applied=await vscode.window.activeTextEditor.edit(edit=>edit.insert(new vscode.Position(0,0),'extension:'));
  await vscode.window.showInformationMessage('context edit='+applied);
 }));
};"#,
        ),
        json!({"activationEvents":["onCommand:a.run"]}),
    );
    let mut app = configured(root.path(), config.path(), &store);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("first 猫", false);
    let first = app.doc().id;
    enable(&mut app, "test.a", Scope::Global);
    app.execute("a.run", Value::Null);
    until(&mut app, |_| {
        root.path().join("activation-started").exists()
    });
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("second 🙂", false);
    let second = app.doc().id;
    assert_ne!(first, second);
    std::fs::write(root.path().join("activation-release"), "yes").unwrap();
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.owner_active("test.a"))
    });
    for _ in 0..10 {
        app.poll();
    }
    assert_eq!(
        app.documents
            .iter()
            .find(|doc| doc.id == first)
            .unwrap()
            .text
            .to_string(),
        "first 猫"
    );
    assert_eq!(app.doc().id, second);
    assert_eq!(app.doc().text.to_string(), "second 🙂");
    app.execute("a.run", Value::Null);
    until(&mut app, |app| app.message == "context edit=true");
    assert_eq!(app.doc().id, second);
    assert_eq!(app.doc().text.to_string(), "extension:second 🙂");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "second 🙂");
}
