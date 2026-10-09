use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use vscli::{app::App, extensions::Package, keys::Profile};
fn until(app: &mut App, test: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if test(app) {
            return;
        }
        assert!(Instant::now() < end, "Timed out: {}", app.message);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn key(app: &mut App, key: KeyCode) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| vscli::ui::draw(frame, app)).unwrap();
    app.event(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)));
}
fn fixture(root: &Path, visible: bool) -> App {
    let folder = root.join("extension");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(root.join("hidden.txt"), "hidden猫\r\nretained").unwrap();
    std::fs::write(root.join("visible.txt"), "visible😀\r\n").unwrap();
    std::fs::write(folder.join("package.json"),r#"{"publisher":"fixture","name":"surface-documents","version":"1.0.0","main":"extension.cjs","contributes":{"views":{"explorer":[{"id":"fixture.hiddenTree","name":"Hidden Document Tree"}]}}}"#).unwrap();
    std::fs::write(folder.join("extension.cjs"),r#"
const vscode=require('vscode'),assert=require('node:assert/strict'),path=require('node:path');
exports.activate=async context=>{
 const hidden=await vscode.workspace.openTextDocument(vscode.Uri.file(path.join(vscode.workspace.rootPath,'hidden.txt')));
 let closed=0;context.subscriptions.push(vscode.workspace.onDidCloseTextDocument(doc=>{if(doc===hidden)closed++;}));
 const originalActive=vscode.window.activeTextEditor?.document;
 function check(){assert.equal(closed,0);assert.equal(hidden.isClosed,false);assert(vscode.workspace.textDocuments.includes(hidden));assert.equal(hidden.getText(),'hidden猫\r\nretained');assert.equal(vscode.window.activeTextEditor?.document,originalActive);}
 const out=vscode.window.createOutputChannel('Hidden-preserving output');out.appendLine('ready');
 const status=vscode.window.createStatusBarItem('hidden-check');status.text='Hidden intact';status.command='surfaceDocuments.check';status.show();
 context.subscriptions.push(vscode.commands.registerCommand('surfaceDocuments.check',()=>{check();out.appendLine('checked');return vscode.window.showInformationMessage('hidden surface action intact');}));
 context.subscriptions.push(vscode.commands.registerCommand('surfaceDocuments.output',()=>{check();out.show(true);}));
 context.subscriptions.push(vscode.commands.registerCommand('surfaceDocuments.show',async()=>{check();const editor=await vscode.window.showTextDocument(hidden);assert.equal(editor.document,hidden);assert.equal(closed,0);await vscode.window.showInformationMessage('same hidden document shown');}));
 const leaf={};const tree=vscode.window.createTreeView('fixture.hiddenTree',{treeDataProvider:{getChildren(){check();return [leaf];},getTreeItem(){check();return {label:'Hidden child',command:{command:'surfaceDocuments.check'}};}}});
 context.subscriptions.push(out,status,tree);
};
"#).unwrap();
    let mut app = App::new(root.into(), Profile::Linux);
    if visible {
        app.open(&root.join("visible.txt")).unwrap();
    }
    app.start_extension_packages(vec![Package::read(&folder).unwrap()])
        .unwrap();
    until(&mut app, |a| {
        a.extension_host
            .as_ref()
            .is_some_and(|h| h.ready && h.surfaces.trees.len() == 1)
    });
    app
}
#[test]
fn surface_callbacks_keep_hidden_document_identity_with_and_without_visible_editors() {
    for visible in [false, true] {
        let root = tempfile::Builder::new()
            .prefix("vscli-hidden-document-surface-long-workspace-")
            .tempdir()
            .unwrap();
        let mut app = fixture(root.path(), visible);
        if !visible {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
            terminal
                .draw(|frame| vscli::ui::draw(frame, &mut app))
                .unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("No open editors"));
            assert!(screen.contains("Hidden intact"));
        }

        let original = app
            .active_document()
            .map(|d| (d.id, d.text.to_string(), d.revision));
        app.execute("surfaceDocuments.output", Value::Null);
        until(&mut app, |a| a.extension_surfaces.output.is_some());
        app.execute("vscli.extensions.status", Value::Null);
        key(&mut app, KeyCode::Enter);
        until(&mut app, |a| {
            a.message == "hidden surface action intact"
                && a.extension_host.as_ref().is_some_and(|h| !h.busy())
        });
        app.execute("vscli.extensions.trees", Value::Null);
        key(&mut app, KeyCode::Enter);
        until(&mut app, |a| a.surface_tree_rows().len() == 1);
        key(&mut app, KeyCode::Enter);
        until(&mut app, |a| {
            a.message == "hidden surface action intact"
                && a.extension_host.as_ref().is_some_and(|h| !h.busy())
        });
        assert_eq!(
            app.active_document()
                .map(|d| (d.id, d.text.to_string(), d.revision)),
            original
        );
        assert_eq!(app.documents.len(), usize::from(visible));
        assert_eq!(
            std::fs::read_to_string(root.path().join("hidden.txt")).unwrap(),
            "hidden猫\r\nretained"
        );
        app.execute("surfaceDocuments.show", Value::Null);
        until(&mut app, |a| a.message == "same hidden document shown");
        let id = app.doc().id;
        assert_eq!(app.doc().text.to_string(), "hidden猫\r\nretained");
        app.doc_mut().insert("dirty", false);
        app.doc_mut().undo();
        assert_eq!(app.doc().id, id);
        assert_eq!(app.doc().text.to_string(), "hidden猫\r\nretained");
        app.execute("workbench.action.files.save", Value::Null);
        assert_eq!(
            std::fs::read_to_string(root.path().join("hidden.txt")).unwrap(),
            "hidden猫\r\nretained"
        );
    }
}
