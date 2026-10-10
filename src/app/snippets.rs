use super::*;
use crate::{
    document::Selection,
    snippet::{
        Template,
        variables::{Cursor, Environment},
    },
};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct Pending {
    receiver: Receiver<Option<String>>,
    template: Template,
    document: u64,
    revision: u64,
    pane: u64,
    selections: Vec<Selection>,
}

impl App {
    pub(super) fn insert_snippet(&mut self, args: &Value) {
        if self.active_document().is_none() {
            self.message = "Open a file or create a new file first".into();
            return;
        }
        if self.snippet_pending.is_some() {
            self.message = "A snippet is waiting for the clipboard".into();
            return;
        }
        let Some(body) = args.get("snippet").and_then(Value::as_str) else {
            self.load_snippet_catalog(args);
            return;
        };
        let template = match Template::parse_user(body) {
            Ok(template) => template,
            Err(error) => {
                self.message = format!("Snippet insertion failed: {error}");
                return;
            }
        };
        if template.uses_variable("CLIPBOARD") {
            let (sender, receiver) = mpsc::sync_channel(1);
            let pending = Pending {
                receiver,
                template,
                document: self.doc().id,
                revision: self.doc().revision,
                pane: self.panes[self.active_pane].id,
                selections: self.doc().selections(),
            };
            std::thread::spawn(move || {
                let _ = sender.send(clipboard_read());
            });
            self.snippet_pending = Some(pending);
            self.message = "Reading clipboard for snippet…".into();
        } else {
            self.apply_snippet(template, None);
        }
    }

    pub(super) fn poll_snippet(&mut self) -> bool {
        let Some(pending) = &self.snippet_pending else {
            return false;
        };
        let clipboard = match pending.receiver.try_recv() {
            Ok(text) => text,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => None,
        };
        let pending = self.snippet_pending.take().unwrap();
        if self.active_document().is_none()
            || self.doc().id != pending.document
            || self.doc().revision != pending.revision
            || self.panes[self.active_pane].id != pending.pane
            || self.doc().selections() != pending.selections
            || self.focus != Focus::Editor
            || self.prompt.is_some()
            || self.modal.is_some()
        {
            self.message =
                "Snippet canceled because its editor context changed while reading the clipboard"
                    .into();
            return true;
        }
        self.apply_snippet(
            pending.template,
            clipboard.or_else(|| Some(self.clipboard.clone())),
        );
        true
    }

    fn apply_snippet(&mut self, template: Template, clipboard: Option<String>) {
        let environment = Environment {
            workspace: self.workspace.root.clone(),
            clipboard,
            language: self.language().into(),
            timestamp: chrono::Local::now().fixed_offset(),
        };
        let count = self.doc().selections().len();
        let result = self.doc_mut().insert_snippet_command_resolved(
            &template,
            |document, selection, index, name, indent| {
                environment.resolve(
                    Cursor {
                        document,
                        selection,
                        index,
                        count,
                    },
                    name,
                    indent,
                )
            },
        );
        match result {
            Ok(()) => {
                self.preview_edit_barrier();
                self.focus = Focus::Editor;
                self.message = if self.doc().in_snippet() {
                    "Snippet · Tab next · Shift+Tab previous · Esc leave"
                } else {
                    "Snippet inserted"
                }
                .into();
            }
            Err(error) => self.message = format!("Snippet insertion failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        app.event(Event::Key(KeyEvent::new(code, modifiers)));
    }
    #[test]
    fn snippet_keys_traverse_cancel_preserve_undo_and_respect_focus_and_overrides() {
        for profile in [Profile::Linux, Profile::Macos, Profile::Windows] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = App::new(directory.path().into(), profile);
            app.execute(
                "workbench.action.files.newUntitledFile",
                serde_json::Value::Null,
            );
            app.execute(
                "editor.action.insertSnippet",
                json!({"snippet":"${1:cat}-$1-${2:dog}$0"}),
            );
            assert!(app.doc().in_snippet());
            key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(app.doc().selected_text().as_deref(), Some("dog"));
            key(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
            assert_eq!(app.doc().selected_text().as_deref(), Some("x"));
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            assert!(!app.doc().in_snippet());
            app.execute("undo", Value::Null);
            assert_eq!(app.doc().text.to_string(), "cat-cat-dog");
            app.execute("redo", Value::Null);
            assert_eq!(app.doc().text.to_string(), "x-x-y");

            app.doc_mut().select_all();
            app.execute(
                "editor.action.insertSnippet",
                json!({"snippet":"${1:cat}-$1$0"}),
            );
            key(&mut app, KeyCode::Esc, KeyModifiers::SHIFT);
            assert!(!app.doc().in_snippet());
            assert_eq!(app.doc().selected_text().as_deref(), Some("cat"));
            assert!(app.doc().secondary.is_empty());
            key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
            assert_eq!(app.doc().text.to_string(), "z-cat");

            app.doc_mut().select_all();
            app.execute(
                "editor.action.insertSnippet",
                json!({"snippet":"${1:a}${2:b}$0"}),
            );
            app.focus = Focus::Explorer;
            let before = app.doc().selections();
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(app.doc().selections(), before);
            app.focus = Focus::Editor;
            let bindings = directory.path().join("keys.json");
            std::fs::write(
                &bindings,
                r#"[{"key":"tab","command":"type","args":{"text":"override"}}]"#,
            )
            .unwrap();
            app.keymap.load(&bindings).unwrap();
            key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(app.doc().text.to_string(), "overrideb");
        }
    }

    #[test]
    fn clipboard_reply_after_last_editor_closed_does_not_recreate_a_buffer() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        let (sender, receiver) = mpsc::sync_channel(1);
        app.snippet_pending = Some(Pending {
            receiver,
            template: Template::parse_user("$CLIPBOARD").unwrap(),
            document: app.doc().id,
            revision: app.doc().revision,
            pane: app.panes[app.active_pane].id,
            selections: app.doc().selections(),
        });
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        sender.send(Some("late".into())).unwrap();
        assert!(app.poll_snippet());
        assert!(app.documents.is_empty());
        assert!(app.snippet_pending.is_none());
        assert!(app.message.contains("context changed"));
    }

    #[test]
    fn clipboard_reply_requires_unchanged_document_view_selection_and_focus() {
        for change in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let mut app = App::new(directory.path().into(), Profile::Linux);
            app.execute(
                "workbench.action.files.newUntitledFile",
                serde_json::Value::Null,
            );
            app.doc_mut().insert("seed", false);
            let (sender, receiver) = mpsc::sync_channel(1);
            app.snippet_pending = Some(Pending {
                receiver,
                template: Template::parse_user("$CLIPBOARD${1:x}$0").unwrap(),
                document: app.doc().id,
                revision: app.doc().revision,
                pane: app.panes[app.active_pane].id,
                selections: app.doc().selections(),
            });
            app.insert_snippet(&json!({"snippet":"should not queue"}));
            assert!(!app.poll_snippet());
            match change {
                0 => app.doc_mut().insert("!", true),
                1 => app.doc_mut().move_to(0, false),
                2 => app.execute("workbench.action.splitEditorRight", Value::Null),
                3 => app.focus = Focus::Explorer,
                _ => {}
            }
            let before = app.doc().text.clone();
            sender.send(Some("clip".into())).unwrap();
            assert!(app.poll_snippet());
            assert!(app.snippet_pending.is_none());
            if change == 4 {
                assert_eq!(app.doc().text.to_string(), "seedclipx");
                app.doc_mut().undo();
                assert_eq!(app.doc().text, before);
            } else {
                assert_eq!(app.doc().text, before);
                assert!(app.message.contains("context changed"));
            }
        }
    }
}
