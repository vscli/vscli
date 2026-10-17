//! Actual output-queue saturation: completed native save receipts cannot be
//! lost or readmitted while the real optional-host writer is occupied.
use super::*;
use crate::app::saving::document_interests::Receipt;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);
const REQUEST: &str = "save-receipt-original";
const UNTITLED: &str = "猫🙂 unsaved\r\nsecond\r\n";
const ACTIVE: &str = "β🙂 active\r\n";

struct WriterGate {
    entered: Receiver<()>,
    release: Option<SyncSender<()>>,
}
impl WriterGate {
    fn resume(&mut self) {
        self.release.take().unwrap().try_send(()).unwrap();
    }
}
impl Drop for WriterGate {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.try_send(());
        }
    }
}
fn until(app: &mut App, predicate: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", app.message);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture(root: &Path) -> PathBuf {
    let extension = root.join("receipt-extension");
    std::fs::create_dir(&extension).unwrap();
    std::fs::write(extension.join("package.json"), r#"{"publisher":"test","name":"save-receipt","version":"1.0.0","main":"index.cjs","contributes":{"commands":[{"command":"receipt.issue","title":"Issue exact internal save"}]}}"#).unwrap();
    std::fs::write(extension.join("index.cjs"), r#"
'use strict';
const vscode = require('vscode'), fs = require('node:fs'), path = require('node:path');
exports.activate = context => {
  const root = vscode.workspace.rootPath, replies = []; let buffer = Buffer.alloc(0);
  // Supplemental protocol observer only: the host remains the actual parser
  // and authoritative selected owner. Public TextDocument.save is not wired.
  const observe = chunk => {
    if (buffer.length + chunk.length > 128 * 1024) throw new Error('Receipt observer input budget');
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length) {
      const header = buffer.indexOf('\r\n\r\n'); if (header < 0) break;
      const match = /^Content-Length: (\d+)$/i.exec(buffer.subarray(0, header).toString('ascii'));
      if (!match) throw new Error('Receipt observer framing');
      const length = Number(match[1]); if (length > 64 * 1024) throw new Error('Receipt observer body budget');
      if (buffer.length < header + 4 + length) break;
      const message = JSON.parse(buffer.subarray(header + 4, header + 4 + length));
      buffer = buffer.subarray(header + 4 + length);
      if (message.id === 'save-receipt-original') {
        if (replies.length >= 8) throw new Error('Receipt observer reply budget');
        replies.push(message);
        const temporary = path.join(root, 'receipt-observed.tmp'), destination = path.join(root, 'receipt-observed.json');
        fs.writeFileSync(temporary, JSON.stringify(replies)); fs.renameSync(temporary, destination);
      }
    }
  };
  process.stdin.on('data', observe);
  context.subscriptions.push({ dispose() { process.stdin.removeListener('data', observe); } });
  context.subscriptions.push(vscode.commands.registerCommand('receipt.issue', proof => {
    const body = Buffer.from(JSON.stringify({ id:'save-receipt-original', method:'nativeDocumentSave',
      params:{session:proof.session,owner:'test.save-receipt',args:{document:proof.document,version:1}} }));
    process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`); process.stdout.write(body);
  }));
};
"#).unwrap();
    extension
}
#[test]
fn canceled_untitled_save_keeps_exact_terminal_receipt_during_real_output_saturation() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let active_path = root.join("active.txt");
    std::fs::write(&active_path, ACTIVE).unwrap();
    let mut app = App::new(root.clone(), Profile::Linux);
    app.settings = crate::settings::Settings::from_values(json!({
        "vscli.languageServer.enabled":false, "files.autoSave":"off", "editor.formatOnSave":false,
        "editor.codeActionsOnSave":{}, "breadcrumbs.enabled":false,
    }).as_object().unwrap().clone(), "save receipt saturation").unwrap();
    app.open(&active_path).unwrap();
    until(&mut app, |app| {
        app.active_document()
            .is_some_and(|doc| doc.path.as_ref() == Some(&active_path))
    });
    let active_id = app.doc().id;
    let mut hidden = Document::from_text(UNTITLED);
    hidden.insert("captured λ ", false);
    let document = hidden.id;
    app.hidden_documents.push(hidden);
    let package = crate::extensions::Package::read(&fixture(&root)).unwrap();
    let prepared = crate::extensions::Client::prepare_with_hidden(
        &app.documents,
        &app.hidden_documents,
        app.active,
        &app.settings,
    )
    .unwrap();
    app.extension_host = Some(
        crate::extensions::Client::start_many_prepared("node", &[package], &root, prepared)
            .unwrap(),
    );
    until(&mut app, |app| {
        app.extension_host.as_ref().is_some_and(|host| host.ready)
    });
    let session = app.extension_host.as_ref().unwrap().session;
    app.execute(
        "receipt.issue",
        json!({"session":session,"document":document}),
    );
    until(&mut app, |app| {
        app.prompt
            .as_ref()
            .is_some_and(|prompt| matches!(prompt.kind, PromptKind::SaveAs))
    });
    assert_eq!(app.doc().id, active_id);
    let original = app
        .extension_host
        .as_ref()
        .unwrap()
        .service()
        .unwrap()
        .clone();
    assert_eq!(original.id, REQUEST);
    assert_eq!(original.request.owner, "test.save-receipt");
    let hidden = app
        .hidden_documents
        .iter()
        .find(|doc| doc.id == document)
        .unwrap();
    let text = hidden.text.clone();
    let epoch = hidden.text_epoch();
    let revision = hidden.revision;
    let selection = hidden.selections();
    let active_selection = app.doc().selections();
    let generation = hidden.save_generation();

    let (entered, release) = app
        .extension_host
        .as_ref()
        .unwrap()
        .fixture_gate_native_writer()
        .unwrap();
    let mut gate = WriterGate {
        entered,
        release: Some(release),
    };
    app.extension_host
        .as_ref()
        .unwrap()
        .fixture_native_output_probe()
        .unwrap();
    gate.entered
        .recv_timeout(WAIT)
        .expect("actual writer reached after-dequeue boundary");
    for _ in 0..64 {
        app.extension_host
            .as_ref()
            .unwrap()
            .fixture_native_output_probe()
            .unwrap();
    }
    let error = app
        .extension_host
        .as_ref()
        .unwrap()
        .fixture_native_output_probe()
        .unwrap_err();
    assert!(format!("{error:#}").contains("Protocol output unavailable"));
    // Original Escape cancels only its path interest. The writer cannot dequeue
    // any of the 64 existing real frames until our positive release.
    app.event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(app.prompt.is_none());
    assert!(!app.native_save_worker_busy());
    let completion = app
        .document_save_completion()
        .expect("native path cancellation receipt");
    assert_eq!(completion.caller.request, REQUEST);
    assert_eq!(completion.caller.session, session);
    assert!(completion.retired);
    assert_eq!(completion.receipt, Receipt::NotSaved);
    let interest = completion.interest;
    for _ in 0..3 {
        assert!(app.poll_extension_services());
        let pending = app.extension_host.as_ref().unwrap().service().unwrap();
        assert_eq!(pending.id, REQUEST);
        assert_eq!(pending.started, original.started);
        let completion = app
            .document_save_completion()
            .expect("terminal receipt retained until transport admission");
        assert_eq!(completion.interest, interest);
        assert_eq!(completion.receipt, Receipt::NotSaved);
        assert!(app.message.contains("Protocol output unavailable"));
        assert!(app.prompt.is_none());
        assert!(!app.native_save_worker_busy());
        assert!(!app.saves_pending());
    }
    gate.resume();
    let observed = root.join("receipt-observed.json");
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.service().is_none())
            && app.document_save_completion().is_none()
            && observed.is_file()
    });
    let replies: Value = serde_json::from_slice(&std::fs::read(&observed).unwrap()).unwrap();
    assert_eq!(replies, json!([{"id":REQUEST,"result":false}]));
    for _ in 0..16 {
        app.poll();
        assert!(app.prompt.is_none());
        assert!(!app.saves_pending());
    }
    app.extension_host
        .as_mut()
        .unwrap()
        .fixture_native_output_barrier()
        .unwrap();
    until(&mut app, |app| {
        app.extension_host
            .as_ref()
            .is_some_and(|host| host.ready && !host.busy())
    });
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&observed).unwrap()).unwrap(),
        replies
    );
    assert_eq!(app.doc().id, active_id);
    assert_eq!(app.doc().selections(), active_selection);
    assert_eq!(app.doc().text.to_string(), ACTIVE);
    assert_eq!(std::fs::read(&active_path).unwrap(), ACTIVE.as_bytes());
    assert!(!root.join("untitled.txt").exists());
    let hidden = app
        .hidden_documents
        .iter()
        .find(|doc| doc.id == document)
        .unwrap();
    assert!(hidden.path.is_none());
    assert_eq!(hidden.text, text);
    assert_eq!(hidden.text_epoch(), epoch);
    assert_eq!(hidden.revision, revision);
    assert_eq!(hidden.save_generation(), generation);
    assert_eq!(hidden.selections(), selection);
    assert!(app.extension_host.as_ref().unwrap().service().is_none());
    let hidden = app
        .hidden_documents
        .iter_mut()
        .find(|doc| doc.id == document)
        .unwrap();
    hidden.undo();
    assert_eq!(hidden.text.to_string(), UNTITLED);
    hidden.redo();
    assert_eq!(hidden.text, text);
    assert!(!root.join("untitled.txt").exists());
    assert_eq!(std::fs::read(&active_path).unwrap(), ACTIVE.as_bytes());
    app.stop_extension_host();
}
