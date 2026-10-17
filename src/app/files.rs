use super::*;
use crate::files::{Action, Job};
impl App {
    pub(super) fn start_file_job(&mut self, action: Action) {
        if self.workspace_transactions_pending()
            || self.resource_models_pending()
            || self.api_discard_occupied()
        {
            self.message = "Wait for pending document operations before changing file paths".into();
            return;
        }
        if self.saves_pending() {
            self.message = "Wait for pending saves before changing file paths".into();
            return;
        }
        if self.file_job.is_some() {
            self.message = "A file operation is already running".into();
            return;
        }
        if self.file_participation_pending() {
            self.message = "A file operation is awaiting extension edits".into();
            return;
        }
        if self
            .extension_host
            .as_ref()
            .is_some_and(|host| !host.file_operation_capacity())
        {
            self.message = "Wait for pending extension file-operation notifications".into();
            return;
        }
        if matches!(action, Action::Rename { .. } | Action::Trash(_))
            && let Some(error) = self
                .documents
                .iter()
                .chain(&self.hidden_documents)
                .find_map(|doc| doc.check_resource_change().err())
        {
            self.message = format!("File operation refused before disk changes: {error:#}");
            return;
        }
        if let Err(error) = self.begin_native_file_participation(action) {
            self.message = format!("File operation could not start: {error:#}");
        }
    }
    pub(super) fn refresh_files(&mut self) {
        self.entries = directory_entries(&self.explorer_dir);
        self.explorer_selected = self
            .explorer_selected
            .min(self.entries.len().saturating_sub(1));
        self.workspace.refresh();
        self.message = "Explorer refreshed; rebuilding file index".into();
    }
    pub(super) fn poll_files(&mut self) -> bool {
        let Some(result) = self.file_job.as_mut().and_then(Job::poll) else {
            return false;
        };
        self.file_job = None;
        let participant_notice = self.take_file_participation_notice();
        match result {
            Err(error) => self.message = format!("File operation failed: {error}"),
            Ok(action) => {
                let mut publication_error = None;
                if let Err(error) = self.remove_api_operation_references(&action) {
                    publication_error = Some(format!(
                        "File operation completed; reference cleanup failed: {error:#}"
                    ));
                }
                if let Some(host) = &mut self.extension_host
                    && let Err(error) = host.queue_file_operation(&action)
                {
                    publication_error = Some(format!(
                        "File operation completed; extension event could not be retained: {error:#}"
                    ));
                }
                self.refresh_files();
                match action {
                    Action::CreateFile(path) => {
                        if let Err(e) = self.open(&path) {
                            self.message = format!("Created file, but open failed: {e:#}");
                        }
                    }
                    Action::CreateFolder(path) => {
                        self.message = format!("Created folder {}", path.display())
                    }
                    Action::Rename { from, to } => {
                        let to = std::fs::canonicalize(&to).unwrap_or(to);
                        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                            if let Some(path) = &doc.path
                                && let Ok(relative) = path.strip_prefix(&from)
                            {
                                let path = Some(if relative.as_os_str().is_empty() {
                                    to.clone()
                                } else {
                                    to.join(relative)
                                });
                                // Native saves are excluded while this job runs;
                                // resource-clock capacity was checked before IO.
                                doc.set_resource_path(path)
                                    .expect("file job resource capacity was preflighted");
                            }
                        }
                        self.diagnostics.clear();
                        self.message = format!("Renamed to {}", to.display());
                    }
                    Action::Trash(path) => {
                        let mut retained = 0;
                        for doc in self.documents.iter_mut().chain(&mut self.hidden_documents) {
                            if doc.path.as_ref().is_some_and(|p| p.starts_with(&path)) {
                                doc.set_resource_path(None)
                                    .expect("file job resource capacity was preflighted");
                                doc.disk_content = None;
                                doc.saved_revision = u64::MAX;
                                retained += 1;
                            }
                        }
                        self.diagnostics.clear();
                        self.message = format!(
                            "Moved to system trash; retained {retained} open buffer(s) as unsaved copies"
                        );
                    }
                }
                if let Some(error) = publication_error {
                    self.message = error;
                }
                if let Some(notice) = participant_notice {
                    self.message = format!("{} · {notice}", self.message);
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_extension_receives_joined_native_create_and_rename_after_document_publication() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let from = root.join("a.cpp");
        let to = root.join("b.cpp");
        let created = root.join("created.cpp");
        let log = root.join("events.jsonl");
        std::fs::write(&from, "original 猫🙂\r\n").unwrap();
        let extension = root.join("extension");
        std::fs::create_dir(&extension).unwrap();
        std::fs::write(extension.join("package.json"),serde_json::to_vec(&json!({"publisher":"fixture","name":"file-events","version":"1.0.0","main":"main.cjs","engines":{"vscode":"*"},"activationEvents":["*"]})).unwrap()).unwrap();
        std::fs::write(extension.join("main.cjs"),r#"
const vscode=require('vscode'),fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
exports.activate=context=>{
 const root=vscode.workspace.workspaceFolders[0].uri.fsPath;
 const original=vscode.workspace.textDocuments.find(document=>document.uri.fsPath===path.join(root,'a.cpp'));
 const record=(kind,event)=>{
  assert.equal(Object.isFrozen(event),true);assert.equal(Object.isFrozen(event.files),false);
  if(kind==='rename'){assert.equal(original.isClosed,true);assert.ok(vscode.workspace.textDocuments.some(document=>document.uri.fsPath===event.files[0].newUri.fsPath));}
  if(kind==='create')assert.ok(vscode.workspace.textDocuments.some(document=>document.uri.fsPath===event.files[0].fsPath));
  fs.appendFileSync(path.join(root,'events.jsonl'),JSON.stringify({kind,files:event.files.map(file=>file.oldUri?{old:file.oldUri.fsPath,new:file.newUri.fsPath}:file.fsPath)})+'\n');
 };
 context.subscriptions.push(vscode.workspace.onDidCreateFiles(event=>record('create',event)),vscode.workspace.onDidRenameFiles(event=>record('rename',event)));
};
"#).unwrap();
        let mut app = App::new(root.clone(), Profile::Linux);
        app.open(&from).unwrap();
        app.doc_mut().insert("dirty ", false);
        app.start_extension_packages(vec![crate::extensions::Package::read(&extension).unwrap()])
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.extension_host.as_ref().is_some_and(|host| host.ready) {
            app.poll();
            assert!(std::time::Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.start_file_job(Action::CreateFile(created.clone()));
        while std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count()
            < 1
        {
            app.poll();
            assert!(std::time::Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.open(&from).unwrap();
        let id = app.doc().id;
        app.start_file_job(Action::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        while std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count()
            < 2
        {
            app.poll();
            assert!(std::time::Instant::now() < deadline, "{}", app.message);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let events: Vec<Value> = std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events[0], json!({"kind":"create","files":[created]}));
        assert_eq!(
            events[1],
            json!({"kind":"rename","files":[{"old":from,"new":to}]})
        );
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        assert_eq!(app.doc().text.to_string(), "dirty original 猫🙂\r\n");
        assert_eq!(std::fs::read(to).unwrap(), "original 猫🙂\r\n".as_bytes());
    }
    #[test]
    fn actual_file_rename_aba_retires_original_save_and_workspace_proofs() {
        let root = tempfile::tempdir().unwrap();
        let from = root.path().join("before.cpp");
        let to = root.path().join("after.cpp");
        std::fs::write(&from, "猫🙂 disk\r\n").unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.open(&from).unwrap();
        app.doc_mut().insert("dirty λ ", false);
        let path = app.doc().path.clone().unwrap();
        let save = app.doc().capture_save(path.clone()).unwrap();
        let workspace = app.doc().capture_workspace_source().unwrap();
        let source =
            saving::document_interests::NativeSaveSource::capture(&app, app.doc().id).unwrap();
        let text = app.doc().text.clone();
        let baseline = app.doc().disk_content.clone();
        let proof = (
            app.doc().id,
            app.doc().revision,
            app.doc().text_epoch(),
            app.doc().save_generation(),
        );
        for (from, to) in [(from.clone(), to.clone()), (to, from.clone())] {
            app.start_file_job(Action::Rename { from, to });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while app.file_job.is_some() {
                app.poll_files();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        assert_eq!(app.doc().path.as_ref(), Some(&path));
        assert_eq!(app.doc().resource_epoch(), save.resource_epoch() + 2);
        assert!(app.doc().check_save_snapshot(&save).is_err());
        assert!(!workspace.current(app.doc()));
        assert!(source.current(&app).is_err());
        assert_eq!(app.doc().text, text);
        assert_eq!(app.doc().disk_content, baseline);
        assert_eq!(
            (
                app.doc().id,
                app.doc().revision,
                app.doc().text_epoch(),
                app.doc().save_generation()
            ),
            proof
        );
        assert_eq!(std::fs::read(&from).unwrap(), "猫🙂 disk\r\n".as_bytes());
        app.doc_mut().save().unwrap();
        assert_eq!(std::fs::read(from).unwrap(), text.to_string().as_bytes());
    }
    #[test]
    fn rename_keeps_unsaved_buffer_and_save_targets_the_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("before.txt");
        let to = dir.path().join("after.txt");
        std::fs::write(&from, "original").unwrap();
        let mut app = App::new(dir.path().into(), Profile::Linux);
        app.open(&from).unwrap();
        app.doc_mut().insert("unsaved ", false);
        let id = app.doc().id;
        app.start_file_job(Action::Rename {
            from: from.clone(),
            to: to.clone(),
        });
        let start = std::time::Instant::now();
        while app.file_job.is_some() {
            app.poll();
            assert!(start.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            app.doc().path.as_ref(),
            Some(&std::fs::canonicalize(&to).unwrap())
        );
        assert_eq!(app.doc().id, id);
        assert!(app.doc().dirty());
        app.doc_mut().save().unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(to).unwrap(), "unsaved original");
    }
}
