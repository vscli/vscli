//! Typed, source-owned language/selection updates independent of editor focus.
use super::*;
use crate::document::{EditorViewProof, Selection};
use anyhow::{Context as _, ensure};

#[derive(Clone)]
pub(crate) struct Context {
    root: PathBuf,
    settings: Arc<Vec<serde_json::Map<String, Value>>>,
    profile: u64,
}
impl Context {
    pub(crate) fn new(
        root: PathBuf,
        settings: Arc<Vec<serde_json::Map<String, Value>>>,
        profile: u64,
    ) -> Result<Self> {
        ensure!(
            root.is_absolute() && root.as_os_str().len() <= 4096,
            "Editor state workspace exceeds bounds"
        );
        Ok(Self {
            root,
            settings,
            profile,
        })
    }
    pub(crate) fn current(
        &self,
        root: &Path,
        settings: &Arc<Vec<serde_json::Map<String, Value>>>,
        profile: u64,
    ) -> bool {
        self.root == root && Arc::ptr_eq(&self.settings, settings) && self.profile == profile
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditorSelection {
    anchor: EditorPosition,
    active: EditorPosition,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditorPosition {
    line: usize,
    character: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageArgs {
    document: u64,
    version: u64,
    language: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionArgs {
    document: u64,
    version: u64,
    selections: Vec<EditorSelection>,
}
pub(crate) enum Change {
    Language(String),
    Selections(Vec<EditorSelection>),
}
pub(crate) struct Capture {
    pub(crate) document: u64,
    pub(crate) change: Change,
    context: Context,
    uri: String,
    revision: u64,
    epoch: u64,
    language_epoch: u64,
    eol: String,
    view: EditorViewProof,
}
impl Capture {
    pub(crate) fn current(
        &self,
        doc: &Document,
        root: &Path,
        settings: &Arc<Vec<serde_json::Map<String, Value>>>,
        profile: u64,
    ) -> bool {
        self.context.current(root, settings, profile)
            && doc.id == self.document
            && doc.revision == self.revision
            && doc.text_epoch() == self.epoch
            && doc.language_epoch() == self.language_epoch
            && doc.eol == self.eol
            && document_uri(doc).ok().as_ref() == Some(&self.uri)
            && (!matches!(self.change, Change::Selections(_))
                || doc.editor_view_active_current(&self.view))
    }
    pub(crate) fn apply_selections(&self, doc: &mut Document) -> Result<bool> {
        let Change::Selections(requested) = &self.change else {
            bail!("Not a selection operation");
        };
        let mut points = Vec::with_capacity(requested.len() * 2);
        for (index, selection) in requested.iter().enumerate() {
            points.push((selection.anchor, index * 2));
            points.push((selection.active, index * 2 + 1));
        }
        points.sort_by_key(|(point, _)| (point.line, point.character));
        let mut mapped = vec![0; points.len()];
        let mut charged = 0usize;
        let mut begin = 0;
        while begin < points.len() {
            let row = points[begin].0.line;
            let end = begin
                + points[begin..]
                    .iter()
                    .take_while(|(point, _)| point.line == row)
                    .count();
            if row >= doc.line_count() {
                for (_, index) in &points[begin..end] {
                    mapped[*index] = doc.len();
                }
            } else {
                let start = doc.line_start(row);
                let last = doc.line_end(row);
                let line = doc.text.slice(start..last);
                let bytes = line.len_bytes();
                let mut units = 0usize;
                let mut cursor = start;
                let mut chars = line.chars().peekable();
                for (point, index) in &points[begin..end] {
                    if point.character >= bytes {
                        mapped[*index] = last;
                        continue;
                    }
                    while let Some(ch) = chars.peek() {
                        if units + ch.len_utf16() > point.character {
                            break;
                        }
                        charged = charged.saturating_add(ch.len_utf8());
                        ensure!(
                            charged <= 256 * 1024,
                            "Editor selection UTF16 mapping exceeds 256 KiB scan budget"
                        );
                        units += ch.len_utf16();
                        cursor += 1;
                        chars.next();
                    }
                    mapped[*index] = cursor;
                }
            }
            begin = end;
        }
        let selections = (0..requested.len())
            .map(|index| Selection {
                cursor: mapped[index * 2 + 1],
                anchor: Some(mapped[index * 2]),
                desired_column: None,
            })
            .collect::<Vec<_>>();
        // Publication remains atomic: mapping/refusal precedes view mutation.
        // The returned changed bit is sufficient for redraw; mirrors compare
        // exact vectors, including inactive retained views.
        doc.set_editor_selections(&self.view, selections)?;
        Ok(true)
    }
}
impl Client {
    pub(crate) fn set_editor_state_context(&mut self, context: Option<Context>) {
        self.editor_state_context = context;
    }
    pub(super) fn capture_editor_state(
        &self,
        args: &Value,
        language: bool,
        documents: &[Document],
        hidden: &[Document],
    ) -> Result<Arc<Capture>> {
        let context = self
            .editor_state_context
            .clone()
            .context("Editor state context unavailable")?;
        let (document, version, change) = if language {
            let args: LanguageArgs = serde_json::from_value(args.clone())?;
            ensure!(
                !args.language.is_empty()
                    && args.language.len() <= 128
                    && !args.language.contains('\0'),
                "Invalid document language identifier"
            );
            (args.document, args.version, Change::Language(args.language))
        } else {
            let array = args
                .get("selections")
                .and_then(Value::as_array)
                .context("Invalid selection array")?;
            ensure!(
                !array.is_empty() && array.len() <= 4096,
                "Editor selections require 1–4096 entries"
            );
            let args: SelectionArgs = serde_json::from_value(args.clone())?;
            (
                args.document,
                args.version,
                Change::Selections(args.selections),
            )
        };
        let doc = documents
            .iter()
            .chain(hidden)
            .find(|doc| doc.id == document)
            .context("Editor state document is not retained")?;
        ensure!(
            version != 0 && self.service_document_current(doc, version),
            "Editor state source changed before receipt"
        );
        let mirror = self
            .mirror
            .mirrors
            .get(&document)
            .context("Editor model is not mirrored")?;
        ensure!(
            mirror.language_epoch == doc.language_epoch()
                && (language || doc.editor_view_active_current(&mirror.editor_view)),
            "Original mirrored editor view changed before receipt"
        );
        let uri = document_uri(doc)?;
        ensure!(uri.len() <= 4096, "Editor state URI exceeds 4096 bytes");
        Ok(Arc::new(Capture {
            document,
            change,
            context,
            uri,
            revision: doc.revision,
            epoch: doc.text_epoch(),
            language_epoch: doc.language_epoch(),
            eol: doc.eol.clone(),
            view: mirror.editor_view.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(doc: &Document, change: Change) -> Capture {
        Capture {
            document: doc.id,
            change,
            context: Context::new(PathBuf::from("/workspace"), Arc::default(), 1).unwrap(),
            uri: document_uri(doc).unwrap(),
            revision: doc.revision,
            epoch: doc.text_epoch(),
            language_epoch: doc.language_epoch(),
            eol: doc.eol.clone(),
            view: doc.editor_view_identity(),
        }
    }
    fn selections(value: Value) -> Change {
        let args: SelectionArgs = serde_json::from_value(value).unwrap();
        Change::Selections(args.selections)
    }
    #[test]
    fn utf16_clamp_keeps_reversed_primary_and_unicode_crlf_source_history() {
        let mut doc = Document::from_text("α😀x\r\n猫z\r\n");
        doc.insert("!", false);
        doc.undo();
        let original = doc.text.to_string();
        let proof = (
            doc.id,
            doc.revision,
            doc.text_epoch(),
            doc.save_generation(),
            doc.dirty(),
        );
        let request = capture(
            &doc,
            selections(json!({"document":doc.id,"version":1,"selections":[
                {"anchor":{"line":1,"character":999999},"active":{"line":1,"character":0}},
                {"anchor":{"line":0,"character":2},"active":{"line":0,"character":3}},
                {"anchor":{"line":999999,"character":999999},"active":{"line":999999,"character":0}}
            ]})),
        );
        request.apply_selections(&mut doc).unwrap();
        assert_eq!(
            doc.selections(),
            vec![
                Selection {
                    anchor: Some(7),
                    cursor: 5,
                    desired_column: None
                },
                Selection {
                    anchor: Some(1),
                    cursor: 2,
                    desired_column: None
                },
                Selection {
                    anchor: Some(9),
                    cursor: 9,
                    desired_column: None
                }
            ]
        );
        assert_eq!(doc.text.to_string(), original);
        assert_eq!(
            (
                doc.id,
                doc.revision,
                doc.text_epoch(),
                doc.save_generation(),
                doc.dirty()
            ),
            proof
        );
        doc.redo();
        assert_eq!(doc.text.to_string(), format!("!{original}"));
        doc.undo();
        assert_eq!(doc.text.to_string(), original);
    }
    #[test]
    fn source_undo_language_and_view_aba_retire_original_capture() {
        let mut doc = Document::from_text("α😀\r\n");
        doc.activate_view(1);
        let old = capture(
            &doc,
            selections(json!({"document":doc.id,"version":1,"selections":[
                {"anchor":{"line":0,"character":0},"active":{"line":0,"character":0}}
            ]})),
        );
        let current =
            |doc: &Document| old.current(doc, Path::new("/workspace"), &old.context.settings, 1);
        assert!(current(&doc));
        doc.insert("x", false);
        doc.undo();
        assert_eq!(doc.revision, old.revision);
        assert!(!current(&doc));
        let language = capture(&doc, Change::Language("cpp".into()));
        doc.set_language_override(Some("cpp".into())).unwrap();
        doc.set_language_override(Some("python".into())).unwrap();
        doc.set_language_override(Some("cpp".into())).unwrap();
        assert!(!language.current(&doc, Path::new("/workspace"), &language.context.settings, 1));
        let view = capture(
            &doc,
            selections(json!({"document":doc.id,"version":1,"selections":[
                {"anchor":{"line":0,"character":0},"active":{"line":0,"character":0}}
            ]})),
        );
        doc.activate_view(2);
        doc.remove_view(1);
        doc.display_view(1);
        assert!(!view.current(&doc, Path::new("/workspace"), &view.context.settings, 1));
        assert!(view.apply_selections(&mut doc).is_err());
    }
    #[test]
    fn mapping_budget_refusal_preserves_view_and_redo_before_publication() {
        let mut doc = Document::from_text(&"é".repeat(140_000));
        doc.insert("!", false);
        doc.undo();
        doc.move_to(2, false);
        let before = (
            doc.selections(),
            doc.revision,
            doc.text_epoch(),
            doc.dirty(),
            doc.save_generation(),
        );
        let request = capture(
            &doc,
            selections(json!({"document":doc.id,"version":1,"selections":[
                {"anchor":{"line":0,"character":139999},"active":{"line":0,"character":139999}}
            ]})),
        );
        let error = request.apply_selections(&mut doc).unwrap_err();
        assert!(error.to_string().contains("256 KiB"));
        assert_eq!(
            (
                doc.selections(),
                doc.revision,
                doc.text_epoch(),
                doc.dirty(),
                doc.save_generation()
            ),
            before
        );
        doc.redo();
        assert_eq!(doc.text.char(0), '!');
    }
    #[test]
    fn strict_selection_dto_refuses_fractional_negative_and_unknown_coordinates() {
        for point in [
            json!({"line":-1,"character":0}),
            json!({"line":0,"character":1.5}),
            json!({"line":0,"character":0,"hidden":true}),
        ] {
            let value = json!({"document":1,"version":1,"selections":[{"anchor":point,"active":{"line":0,"character":0}}]});
            assert!(serde_json::from_value::<SelectionArgs>(value).is_err());
        }
        assert!(
            serde_json::from_value::<LanguageArgs>(
                json!({"document":1,"version":1,"language":"cpp","extra":1})
            )
            .is_err()
        );
    }
    #[test]
    fn scope_identity_refuses_equal_content_settings_and_profile_aba() {
        let doc = Document::from_text("source");
        let old = capture(&doc, Change::Language("cpp".into()));
        assert!(old.current(&doc, Path::new("/workspace"), &old.context.settings, 1));
        assert!(!old.current(&doc, Path::new("/workspace"), &Arc::default(), 1));
        assert!(!old.current(&doc, Path::new("/workspace"), &old.context.settings, 3));
        assert!(!old.current(&doc, Path::new("/other"), &old.context.settings, 1));
    }
}
