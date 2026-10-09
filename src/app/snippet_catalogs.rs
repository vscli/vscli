use super::*;
use crate::{
    document::Selection,
    snippet::catalog::{Catalog, Entry},
};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone)]
struct Context {
    document: u64,
    revision: u64,
    pane: u64,
    selections: Vec<Selection>,
    language: String,
}
impl Context {
    fn capture(app: &App) -> Option<Self> {
        let doc = app.active_document()?;
        let pane = app.panes.get(app.active_pane)?;
        Some(Self {
            document: doc.id,
            revision: doc.revision,
            pane: pane.id,
            selections: doc.selections(),
            language: app.language().into(),
        })
    }
    fn valid(&self, app: &App) -> bool {
        let Some(doc) = app.active_document() else {
            return false;
        };
        self.document == doc.id
            && self.revision == doc.revision
            && app
                .panes
                .get(app.active_pane)
                .is_some_and(|pane| self.pane == pane.id)
            && self.selections == doc.selections()
            && self.language == app.language()
            && app.focus == Focus::Editor
            && app.modal.is_none()
    }
}
struct Pending {
    receiver: Receiver<Catalog>,
    context: Context,
    name: Option<String>,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    entries: Vec<Entry>,
    context: Option<Context>,
}
impl App {
    pub(super) fn load_snippet_catalog(&mut self, args: &Value) {
        if self.snippet_catalog.pending.is_some() {
            self.message = "Snippet catalog is still loading".into();
            return;
        }
        let language = args
            .get("langId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.language())
            .to_owned();
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let Some(context) = Context::capture(self) else {
            self.message = "Open a file or create a new file first".into();
            return;
        };
        // An explicit Insert Snippet command targets the active editor, including
        // when invoked from the palette while Explorer previously held focus.
        self.focus = Focus::Editor;
        let user = self
            .settings_user
            .as_ref()
            .and_then(|path| path.parent())
            .map(|path| path.join("snippets"));
        let workspace = self.workspace.root.clone();
        let extensions = self.extensions_directory.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.snippet_catalog.entries.clear();
        self.snippet_catalog.context = None;
        if name.is_none() {
            self.start_prompt(PromptKind::Snippet, String::new());
        }
        self.snippet_catalog.pending = Some(Pending {
            receiver,
            context,
            name,
        });
        std::thread::spawn(move || {
            let _ = sender.send(Catalog::load_with_extensions(
                user.as_deref(),
                &workspace,
                &language,
                extensions.as_deref(),
            ));
        });
        self.message = "Loading snippets…".into();
    }

    pub(super) fn poll_snippet_catalog(&mut self) -> bool {
        let Some(pending) = &self.snippet_catalog.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(catalog) => catalog,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.snippet_catalog.pending = None;
                self.message = "Snippet catalog worker stopped".into();
                return true;
            }
        };
        let pending = self.snippet_catalog.pending.take().unwrap();
        let expected_prompt = if pending.name.is_none() {
            matches!(
                self.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::Snippet)
            )
        } else {
            self.prompt.is_none()
        };
        if !pending.context.valid(self) || !expected_prompt {
            if matches!(
                self.prompt.as_ref().map(|prompt| &prompt.kind),
                Some(PromptKind::Snippet)
            ) {
                self.prompt = None;
            }
            self.message =
                "Snippet canceled because its editor context changed while loading".into();
            return true;
        }
        let notices = if result.warnings.is_empty() {
            String::new()
        } else {
            format!(
                " · {} catalog warning(s): {}",
                result.warnings.len(),
                result.warnings[0]
            )
        };
        if let Some(name) = pending.name {
            if let Some(entry) = result.entries.iter().find(|entry| entry.name == name) {
                self.insert_snippet(&json!({"snippet": entry.body}));
                self.message.push_str(&notices);
            } else {
                self.message = format!("Snippet {name:?} not found{notices}");
            }
        } else {
            self.message = format!(
                "{} snippets · Enter inserts · Esc cancels{notices}",
                result.entries.len()
            );
            self.snippet_catalog.entries = result.entries;
            self.snippet_catalog.context = Some(pending.context);
        }
        true
    }

    pub fn snippet_items(&self, query: &str) -> Vec<&Entry> {
        let mut entries: Vec<_> = self
            .snippet_catalog
            .entries
            .iter()
            .filter_map(|entry| {
                let rank = std::iter::once(entry.name.as_str())
                    .chain(entry.prefixes.iter().map(String::as_str))
                    .chain(std::iter::once(entry.description.as_str()))
                    .filter_map(|text| score(text, query))
                    .max()?;
                Some((rank, entry))
            })
            .collect();
        entries.sort_by(|(a, left), (b, right)| b.cmp(a).then_with(|| left.name.cmp(&right.name)));
        entries
            .into_iter()
            .take(100)
            .map(|(_, entry)| entry)
            .collect()
    }

    pub(super) fn accept_snippet(&mut self, query: &str, selected: usize) {
        if self.snippet_catalog.pending.is_some() {
            self.start_prompt(PromptKind::Snippet, query.into());
            self.message = "Snippet catalog is still loading".into();
            return;
        }
        if !self
            .snippet_catalog
            .context
            .as_ref()
            .is_some_and(|context| context.valid(self))
        {
            self.message = "Snippet canceled because its editor context changed".into();
            return;
        }
        let items = self.snippet_items(query);
        let body = items
            .get(selected.min(items.len().saturating_sub(1)))
            .map(|entry| entry.body.clone());
        self.snippet_catalog.context = None;
        self.snippet_catalog.entries.clear();
        if let Some(body) = body {
            self.insert_snippet(&json!({"snippet": body}));
        } else {
            self.message = "No matching snippets".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Catalog {
        Catalog {
            entries: vec![Entry {
                name: "Named".into(),
                prefixes: vec!["prefix".into()],
                description: "description".into(),
                body: "${1:cat}-$1$0".into(),
                source: PathBuf::from("test.code-snippets"),
                is_file_template: false,
            }],
            warnings: Vec::new(),
        }
    }
    fn pending(app: &mut App, name: Option<&str>) -> mpsc::SyncSender<Catalog> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let context = Context::capture(app).expect("test editor");
        if name.is_none() {
            app.start_prompt(PromptKind::Snippet, String::new());
        }
        app.snippet_catalog.pending = Some(Pending {
            receiver,
            context,
            name: name.map(str::to_owned),
        });
        sender
    }
    #[test]
    fn empty_workbench_and_closed_editor_cancel_catalog_work_without_new_documents() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        assert!(Context::capture(&app).is_none());
        app.load_snippet_catalog(&Value::Null);
        assert!(app.snippet_catalog.pending.is_none());
        assert!(app.prompt.is_none());
        app.insert_snippet(&json!({"snippet":"must not create a buffer"}));
        assert!(app.documents.is_empty());
        for named in [false, true] {
            app.execute("workbench.action.files.newUntitledFile", Value::Null);
            let sender = pending(&mut app, named.then_some("Named"));
            app.execute("workbench.action.closeActiveEditor", Value::Null);
            sender.send(fixture()).unwrap();
            assert!(app.poll_snippet_catalog());
            assert!(app.documents.is_empty());
            assert!(app.panes.is_empty());
            assert!(app.prompt.is_none());
            assert!(app.snippet_catalog.context.is_none());
            assert!(app.message.contains("context changed"));
        }
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        pending(&mut app, None).send(fixture()).unwrap();
        assert!(app.poll_snippet_catalog());
        assert_eq!(app.snippet_items("").len(), 1);
        app.execute("workbench.action.closeActiveEditor", Value::Null);
        app.accept_prompt();
        assert!(app.documents.is_empty());
        assert!(app.message.contains("context changed"));
    }

    #[test]
    fn catalog_replies_and_picker_accept_require_unchanged_editor_context() {
        for named in [false, true] {
            for change in 0..7 {
                let root = tempfile::tempdir().unwrap();
                let mut app = App::new(root.path().into(), Profile::Linux);
                app.execute("workbench.action.files.newUntitledFile", Value::Null);
                app.doc_mut().insert("seed", false);
                app.doc_mut().select_all();
                let sender = pending(&mut app, named.then_some("Named"));
                app.load_snippet_catalog(&Value::Null);
                assert_eq!(app.message, "Snippet catalog is still loading");
                assert!(!app.poll_snippet_catalog());
                match change {
                    0 => app.doc_mut().insert("!", true),
                    1 => app.doc_mut().move_to(0, false),
                    2 => app.execute("workbench.action.splitEditorRight", Value::Null),
                    3 => app.focus = Focus::Explorer,
                    4 => app.start_prompt(PromptKind::Open, String::new()),
                    5 if !named => app.prompt = None,
                    _ => {}
                }
                let before = app.doc().text.clone();
                sender.send(fixture()).unwrap();
                assert!(app.poll_snippet_catalog());
                if change == 6 || (change == 5 && named) {
                    if !named {
                        assert_eq!(app.snippet_items("prefix").len(), 1);
                        assert_eq!(app.snippet_items("description").len(), 1);
                        app.accept_prompt();
                    }
                    assert_eq!(app.doc().text.to_string(), "cat-cat");
                    app.execute("undo", Value::Null);
                    assert_eq!(app.doc().text, before);
                } else {
                    assert_eq!(app.doc().text, before);
                    assert!(app.message.contains("context changed"));
                    assert!(app.snippet_catalog.context.is_none());
                }
            }
        }
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        pending(&mut app, None).send(fixture()).unwrap();
        assert!(app.poll_snippet_catalog());
        app.doc_mut().insert("new work", false);
        app.accept_prompt();
        assert_eq!(app.doc().text.to_string(), "new work");
        assert!(app.message.contains("context changed"));
    }

    #[test]
    fn named_lookup_language_override_and_fresh_loads_preserve_undo() {
        let root = tempfile::tempdir().unwrap();
        let snippets = root.path().join("user/snippets");
        std::fs::create_dir_all(&snippets).unwrap();
        std::fs::write(
            snippets.join("rust.json"),
            r#"{"Named":{"body":"${1:rust}-$1$0"}}"#,
        )
        .unwrap();
        let mut app = App::new(root.path().into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        let imported_settings = root.path().join("user/settings.json");
        std::fs::write(&imported_settings, "{}").unwrap();
        app.configure_settings(Some(imported_settings)).unwrap();
        app.doc_mut().insert("original", false);
        app.doc_mut().select_all();
        let poll = |app: &mut App| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !app.poll_snippet_catalog() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        app.focus = Focus::Explorer;
        app.insert_snippet(&json!({"name":"Named", "langId":"rust"}));
        assert!(app.focus == Focus::Editor);
        poll(&mut app);
        assert_eq!(app.doc().text.to_string(), "rust-rust");
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), "original");
        std::fs::write(
            snippets.join("rust.json"),
            r#"{"Named":{"body":"changed"}}"#,
        )
        .unwrap();
        app.insert_snippet(&json!({"name":"Named", "langId":"rust"}));
        poll(&mut app);
        assert_eq!(app.doc().text.to_string(), "changed");
        app.execute("undo", Value::Null);
        app.insert_snippet(&json!({"name":"Named"}));
        poll(&mut app);
        assert!(app.message.contains("not found"));
        assert_eq!(app.doc().text.to_string(), "original");
        app.focus = Focus::Explorer;
        app.insert_snippet(&json!({"langId":"rust"}));
        poll(&mut app);
        assert!(app.focus == Focus::Editor);
        assert!(matches!(
            app.prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::Snippet)
        ));
        app.accept_prompt();
        assert_eq!(app.doc().text.to_string(), "changed");
        app.execute("undo", Value::Null);
        assert_eq!(app.doc().text.to_string(), "original");
    }
}
