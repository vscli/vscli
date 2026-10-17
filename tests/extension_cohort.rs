//! Large selected sessions must retain native editing and explicit admission bounds.
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use vscli::{
    app::App,
    extensions::{Client, Package},
    keys::Profile,
};

fn until(app: &mut App, condition: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if condition(app) {
            return;
        }
        assert!(Instant::now() < deadline, "Timed out: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn ninety_code_packages_and_eighteen_hundred_commands_share_native_documents_and_undo() {
    let root = tempfile::tempdir().unwrap();
    let packages: Vec<_> = (0..90).map(|index| {
        let name = format!("p{index:03}");
        let path = root.path().join(&name);
        std::fs::create_dir(&path).unwrap();
        let dependencies: Vec<_> = if index < 89 { vec![format!("cohort.p{:03}", index+1)] } else { Vec::new() };
        let commands: Vec<_> = (0..20).map(|command| json!({"command":format!("cohort.{name}.c{command}"),"title":format!("Cohort {index} command {command}")})).collect();
        // Exceeds the old cumulative node budget without approaching the byte limit.
        let manifest = json!({"publisher":"cohort","name":name,"version":"1.0.0","main":"main.cjs","extensionDependencies":dependencies,"padding":vec![0;600],"contributes":{"commands":commands}});
        std::fs::write(path.join("package.json"), manifest.to_string()).unwrap();
        std::fs::write(path.join("main.cjs"), format!(r#"const v=require('vscode');
exports.activate=context=>{{for(let i=0;i<20;i++) context.subscriptions.push(v.commands.registerCommand('cohort.{name}.c'+i,async()=>{{
const editor=v.window.activeTextEditor;
const ok=await editor.edit(edit=>edit.insert(new v.Position(0,0),'cohort '+v.extensions.all.length+' '));
await v.window.showInformationMessage('large cohort edit='+ok);
}}));return {{id:'cohort.{name}'}};}};"#)).unwrap();
        Package::read(&path).unwrap()
    }).collect();
    let file = root.path().join("native.txt");
    let original = "λ🙂 original\r\n";
    std::fs::write(&file, original).unwrap();
    let mut app = App::new(root.path().into(), Profile::Linux);
    app.open(&file).unwrap();
    let identity = app.doc().id;
    app.extension_host = Some(
        Client::start_many(
            "node",
            &packages,
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
    assert_eq!(app.extension_host.as_ref().unwrap().packages.len(), 90);
    assert_eq!(app.extension_host.as_ref().unwrap().commands.len(), 1800);
    app.execute("cohort.p089.c19", Value::Null);
    until(&mut app, |app| app.message == "large cohort edit=true");
    assert_eq!(app.doc().id, identity);
    assert_eq!(app.doc().text.to_string(), format!("cohort 90 {original}"));
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("cohort 90 {original}")
    );
    app.doc_mut().undo();
    app.execute("workbench.action.files.save", Value::Null);
    until(&mut app, |app| !app.saves_pending());
    assert_eq!(app.doc().id, identity);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}

#[test]
fn package_and_manifest_limits_reject_before_starting_a_runtime() {
    let root = tempfile::tempdir().unwrap();
    let mut packages = Vec::new();
    for index in 0..129 {
        let path = root.path().join(format!("p{index}"));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("package.json"), json!({"publisher":"limit","name":format!("p{index}"),"version":"1.0.0","main":"main.cjs"}).to_string()).unwrap();
        packages.push(Package::read(&path).unwrap());
    }
    let app = App::new(root.path().into(), Profile::Linux);
    let error = Client::start_many(
        "missing-node",
        &packages,
        root.path(),
        &app.documents,
        app.active,
        &app.settings,
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("128 code extensions"));
    for package in &packages[..2] {
        let path = package.path.join("package.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["padding"] = json!(vec![0; 130_000]);
        std::fs::write(path, manifest.to_string()).unwrap();
    }
    let error = Client::start_many(
        "missing-node",
        &packages[..2],
        root.path(),
        &app.documents,
        app.active,
        &app.settings,
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("250,000 nodes"));
}
