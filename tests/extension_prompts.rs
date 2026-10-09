use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use vscli::{
    app::{App, PromptKind},
    extensions::Package,
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
fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn extension_prompt(app: &App) -> bool {
    matches!(
        app.prompt.as_ref().map(|p| &p.kind),
        Some(PromptKind::Extension(_))
    )
}
fn fixture(root: &std::path::Path) -> App {
    let folder = root.join("extension");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(
        folder.join("package.json"),
        r#"{"publisher":"fixture","name":"prompts","version":"1.0.0","main":"extension.cjs"}"#,
    )
    .unwrap();
    std::fs::write(folder.join("extension.cjs"), r#"
const vscode = require('vscode');
exports.activate = context => {
  const register = (id, callback) => context.subscriptions.push(vscode.commands.registerCommand('prompts.' + id, callback));
  register('edit', async () => {
    const item = await vscode.window.showQuickPick([{label:'one',description:'first',detail:'first detail'}, {label:'two 😀',description:'second',detail:'find this',data:'SELECTED'}], {title:'Choose item',matchOnDetail:true});
    if (!item) return vscode.window.showInformationMessage('pick canceled');
    const text = await vscode.window.showInputBox({title:'Enter input',prompt:'Input prompt',placeHolder:'Input hint',value:'default'});
    if (text === undefined) return vscode.window.showInformationMessage('input canceled');
    const editor = vscode.window.activeTextEditor;
    const applied = await editor.edit(edit => edit.insert(new vscode.Position(0,0), item.data + text));
    await vscode.window.showInformationMessage('applied=' + applied);
  });
  register('queue', async () => {
    const values = await Promise.all([vscode.window.showInputBox({title:'First'}),vscode.window.showInputBox({title:'Second'})]);
    await vscode.window.showInformationMessage('queue=' + JSON.stringify(values));
  });
  register('crash', () => process.exit(7));
};
"#).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    app.execute("workbench.action.files.newUntitledFile", Value::Null);
    app.doc_mut().insert("original", false);
    app.start_extension_packages(vec![Package::read(&folder).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    app
}
#[test]
fn quick_pick_input_unicode_edits_cancel_and_undo_preserve_native_identity() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    let id = app.doc().id;
    app.focus = vscli::app::Focus::Explorer;
    app.execute("prompts.edit", Value::Null);
    until(&mut app, extension_prompt);
    app.event(Event::Paste("find this".into()));
    key(&mut app, KeyCode::Enter);
    until(
        &mut app,
        |app| matches!(app.prompt.as_ref().map(|p| &p.kind), Some(PromptKind::Extension(request)) if request.spec.title == "Enter input"),
    );
    app.event(Event::Paste("猫".into()));
    key(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.message == "applied=true");
    assert_eq!(app.doc().id, id);
    assert!(app.focus == vscli::app::Focus::Explorer);
    assert_eq!(app.doc().text.to_string(), "SELECTED猫original");
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "original");
    app.execute("prompts.edit", Value::Null);
    until(&mut app, extension_prompt);
    key(&mut app, KeyCode::Esc);
    until(&mut app, |app| app.message == "pick canceled");
    assert_eq!(app.doc().text.to_string(), "original");
    app.execute("prompts.edit", Value::Null);
    until(&mut app, extension_prompt);
    key(&mut app, KeyCode::Enter);
    until(
        &mut app,
        |app| matches!(app.prompt.as_ref().map(|p| &p.kind), Some(PromptKind::Extension(request)) if request.spec.title == "Enter input"),
    );
    key(&mut app, KeyCode::Esc);
    until(&mut app, |app| app.message == "input canceled");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "original");
}
#[test]
fn native_prompt_priority_fifo_replacement_and_host_failure_do_not_revive_ui() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    app.execute("workbench.action.showCommands", Value::Null);
    app.prompt.as_mut().unwrap().text = "native query".into();
    app.execute("prompts.queue", Value::Null);
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.prompt().is_some())
    });
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Palette
    ));
    assert_eq!(app.prompt.as_ref().unwrap().text, "native query");
    key(&mut app, KeyCode::Esc);
    until(&mut app, extension_prompt);
    let old = app
        .extension_host
        .as_ref()
        .unwrap()
        .prompt()
        .unwrap()
        .clone();
    app.execute("workbench.action.showCommands", Value::Null);
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Palette
    ));
    key(&mut app, KeyCode::Esc);
    until(&mut app, extension_prompt);
    assert_eq!(
        app.extension_host
            .as_ref()
            .unwrap()
            .prompt()
            .unwrap()
            .spec
            .title,
        "Second"
    );
    assert!(
        !app.extension_host
            .as_mut()
            .unwrap()
            .answer_prompt(old.spec.session, &old.spec.owner, &old.id, json!("late"))
            .unwrap()
    );
    app.event(Event::Paste("done".into()));
    key(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.message == "queue=[null,\"done\"]");
    app.execute("prompts.queue", Value::Null);
    until(&mut app, extension_prompt);
    app.execute("prompts.crash", Value::Null);
    until(&mut app, |app| app.extension_host.is_none());
    assert!(app.prompt.is_none());
    assert_eq!(app.doc().text.to_string(), "original");
    app.doc_mut().insert("!", false);
    assert!(app.doc().text.to_string().contains('!'));
}
#[test]
fn stop_and_replacement_clear_prompt_session_while_native_prompt_survives() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    app.execute("prompts.queue", Value::Null);
    until(&mut app, extension_prompt);
    let old = app
        .extension_host
        .as_ref()
        .unwrap()
        .prompt()
        .unwrap()
        .clone();
    app.execute("vscli.extensions.restart", Value::Null);
    assert!(app.prompt.is_none());
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    assert!(
        !app.extension_host
            .as_mut()
            .unwrap()
            .answer_prompt(old.spec.session, &old.spec.owner, &old.id, json!("late"))
            .unwrap()
    );
    app.execute("prompts.queue", Value::Null);
    until(&mut app, extension_prompt);
    app.execute("vscli.extensions.stop", Value::Null);
    assert!(app.prompt.is_none());
    app.execute("workbench.action.showCommands", Value::Null);
    for _ in 0..10 {
        app.poll();
    }
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Palette
    ));
    assert_eq!(app.doc().text.to_string(), "original");
}

#[test]
#[ignore = "requires the pinned unchanged Lorem Ipsum package in VSCLI_TEST_LOREM_IPSUM"]
fn upstream_lorem_ipsum_quick_pick_inserts_paragraphs_and_native_save_undo() {
    let directory = tempfile::tempdir().unwrap();
    let extension = std::path::PathBuf::from(std::env::var("VSCLI_TEST_LOREM_IPSUM").unwrap());
    let path = directory.path().join("paragraphs.txt");
    std::fs::write(&path, "tail").unwrap();
    let mut app = App::new(directory.path().into(), Profile::Linux);
    app.open(&path).unwrap();
    let id = app.doc().id;
    app.start_extension_packages(vec![Package::read(&extension).unwrap()])
        .unwrap();
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    app.execute("lorem-ipsum.multipleParagraphs", Value::Null);
    until(&mut app, extension_prompt);
    key(&mut app, KeyCode::Esc);
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| !host.busy())
    });
    assert_eq!(app.doc().text.to_string(), "tail");
    app.execute("lorem-ipsum.multipleParagraphs", Value::Null);
    until(&mut app, extension_prompt);
    app.event(Event::Paste("2".into()));
    key(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.doc().text.len_bytes() > 4);
    let text = app.doc().text.to_string();
    assert_eq!(app.doc().id, id);
    assert!(app.doc().dirty());
    assert!(text.ends_with("tail"));
    let generated = text.strip_suffix("tail").unwrap();
    assert_eq!(generated.split("\n\n").count(), 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "tail");
    app.execute("workbench.action.files.save", Value::Null);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    app.doc_mut().undo();
    assert_eq!(app.doc().text.to_string(), "tail");
    assert!(app.extension_host.as_ref().is_some_and(|host| host.ready));
}

#[test]
fn native_extension_prompt_dismisses_parameter_hints_and_retains_shared_text() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    let path = directory.path().join("main.cpp");
    std::fs::write(&path, "sum(1, 2)\r\n").unwrap();
    app.open(&path).unwrap();
    app.doc_mut().move_to(7, false);
    let id = app.doc().id;
    let args = vec![
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/signature_server.py")
            .to_string_lossy()
            .into_owned(),
    ];
    app.lsp =
        Some(vscli::lsp::Client::start("python3", &args, directory.path(), "cpp".into()).unwrap());
    until(&mut app, |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    app.execute("editor.action.triggerParameterHints", Value::Null);
    until(&mut app, |app| app.signature_help().is_some());
    // A host-initiated request reaches the native prompt layer without executing
    // another App command that would already dismiss parameter hints.
    app.extension_host
        .as_mut()
        .unwrap()
        .execute(
            "prompts.queue",
            None,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap();
    until(&mut app, extension_prompt);
    assert!(app.signature_help().is_none());
    key(&mut app, KeyCode::Esc);
    until(&mut app, extension_prompt);
    key(&mut app, KeyCode::Esc);
    until(&mut app, |app| app.message == "queue=[null,null]");
    assert!(app.signature_help().is_none());
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().text.to_string(), "sum(1, 2)\r\n");
    assert!(!app.doc().dirty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "sum(1, 2)\r\n");
}

#[test]
fn symbol_picker_preserves_extension_fifo_and_cancels_replaced_prompt_without_edits() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = fixture(directory.path());
    let path = directory.path().join("main.cpp");
    std::fs::write(&path, "x 😀foo\r\n").unwrap();
    app.open(&path).unwrap();
    app.doc_mut().insert("unsaved", false);
    let id = app.doc().id;
    let revision = app.doc().revision;
    let text = app.doc().text.to_string();
    let args = vec![
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/symbol_server.py")
            .to_string_lossy()
            .into_owned(),
    ];
    app.lsp =
        Some(vscli::lsp::Client::start("python3", &args, directory.path(), "cpp".into()).unwrap());
    until(&mut app, |app| {
        app.lsp.as_ref().is_some_and(|client| client.ready)
    });
    app.execute("workbench.action.gotoSymbol", Value::Null);
    until(&mut app, |app| !app.symbol_items("").is_empty());
    // A host request queues behind a native symbol picker without executing
    // an App command that would intentionally cancel the picker.
    app.extension_host
        .as_mut()
        .unwrap()
        .execute(
            "prompts.queue",
            None,
            &app.documents,
            app.active,
            &app.settings,
        )
        .unwrap();
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.prompt().is_some())
    });
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Symbols
    ));
    app.event(Event::Paste("x".repeat(1024)));
    app.event(Event::Paste("overflow".into()));
    assert_eq!(app.prompt.as_ref().unwrap().text.len(), 1024);
    assert_eq!(app.message, "Symbol query exceeds 1 KiB");
    key(&mut app, KeyCode::Esc);
    until(&mut app, extension_prompt);
    let old = app
        .extension_host
        .as_ref()
        .unwrap()
        .prompt()
        .unwrap()
        .clone();
    assert_eq!(old.spec.title, "First");
    // Replacing the visible extension input cancels it once; its second
    // queued input remains behind the new native symbol picker.
    app.execute("workbench.action.gotoSymbol", Value::Null);
    until(&mut app, |app| !app.symbol_items("").is_empty());
    assert!(matches!(
        app.prompt.as_ref().unwrap().kind,
        PromptKind::Symbols
    ));
    assert!(
        !app.extension_host
            .as_mut()
            .unwrap()
            .answer_prompt(old.spec.session, &old.spec.owner, &old.id, json!("late"))
            .unwrap()
    );
    key(&mut app, KeyCode::Esc);
    until(&mut app, extension_prompt);
    assert_eq!(
        app.extension_host
            .as_ref()
            .unwrap()
            .prompt()
            .unwrap()
            .spec
            .title,
        "Second"
    );
    app.event(Event::Paste("done".into()));
    key(&mut app, KeyCode::Enter);
    until(&mut app, |app| app.message == "queue=[null,\"done\"]");
    assert_eq!(app.doc().id, id);
    assert_eq!(app.doc().revision, revision);
    assert_eq!(app.doc().text.to_string(), text);
    assert!(app.prompt.is_none());
    app.execute("undo", Value::Null);
    assert_eq!(app.doc().text.to_string(), "x 😀foo\r\n");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "x 😀foo\r\n");
}
