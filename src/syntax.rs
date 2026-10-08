//! Bounded background grammar highlighting. Results belong to one document revision.
use crate::document::Document;
use anyhow::{Result, bail};
use ropey::Rope;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

pub const NAMES: &[&str] = &[
    "comment",
    "string",
    "number",
    "constant",
    "keyword",
    "function",
    "type",
    "variable",
    "property",
    "operator",
    "punctuation",
    "attribute",
    "constructor",
    "escape",
];
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    id: u64,
    revision: u64,
    language: &'static str,
}
impl Key {
    fn for_doc(doc: &Document) -> Option<Self> {
        if doc.text.len_bytes() > MAX_BYTES {
            return None;
        }
        Some(Self {
            id: doc.id,
            revision: doc.revision,
            language: language(doc.path.as_deref()?)?,
        })
    }
}
pub fn language(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "json" => Some("json"),
        _ => None,
    }
}
fn configuration(language: &str) -> Result<HighlightConfiguration> {
    let (grammar, query) = match language {
        "rust" => (
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY.to_string(),
        ),
        "python" => (
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY.to_string(),
        ),
        "json" => (
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY.to_string(),
        ),
        "javascript" => (
            tree_sitter_javascript::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
        ),
        "typescript" | "tsx" => (
            if language == "tsx" {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            },
            format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                if language == "tsx" {
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                } else {
                    ""
                }
            ),
        ),
        _ => bail!("No grammar for {language}"),
    };
    let mut config = HighlightConfiguration::new(grammar, language, &query, "", "")?;
    config.configure(NAMES);
    Ok(config)
}
#[derive(Clone, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: usize,
}
pub struct Highlights {
    key: Key,
    spans: Vec<Span>,
}
impl Highlights {
    pub fn style_at(&self, byte: usize) -> Option<usize> {
        let index = self.spans.partition_point(|s| s.end <= byte);
        self.spans
            .get(index)
            .filter(|s| s.start <= byte)
            .map(|s| s.style)
    }
}
fn highlight(
    config: &HighlightConfiguration,
    text: &str,
    cancel: &AtomicUsize,
) -> Result<Vec<Span>> {
    let mut highlighter = Highlighter::new();
    let mut stack = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    for event in highlighter.highlight(config, text.as_bytes(), None, Some(cancel), |_| None)? {
        match event? {
            HighlightEvent::HighlightStart(style) => stack.push(style.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if let Some(&style) = stack.last() {
                    if let Some(last) = spans
                        .last_mut()
                        .filter(|s| s.style == style && s.end == start)
                    {
                        last.end = end;
                    } else {
                        spans.push(Span { start, end, style });
                    }
                    if spans.len() > 500_000 {
                        bail!("Syntax span budget exceeded");
                    }
                }
            }
        }
    }
    Ok(spans)
}
struct Request {
    key: Key,
    text: Rope,
    cancel: Arc<AtomicUsize>,
}
struct Response {
    key: Key,
    result: Result<Vec<Span>, String>,
}
struct InFlight {
    key: Key,
    cancel: Arc<AtomicUsize>,
    started: Instant,
}
pub struct Engine {
    sender: SyncSender<Request>,
    receiver: Receiver<Response>,
    pending: Option<InFlight>,
    cache: HashMap<u64, Highlights>,
}
impl Default for Engine {
    fn default() -> Self {
        let (sender, requests) = mpsc::sync_channel::<Request>(1);
        let (responses, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut configurations = HashMap::new();
            for request in requests {
                let result = (|| -> Result<Vec<Span>> {
                    if !configurations.contains_key(request.key.language) {
                        configurations
                            .insert(request.key.language, configuration(request.key.language)?);
                    }
                    highlight(
                        &configurations[request.key.language],
                        &request.text.to_string(),
                        &request.cancel,
                    )
                })()
                .map_err(|e| e.to_string());
                if responses
                    .send(Response {
                        key: request.key,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            sender,
            receiver,
            pending: None,
            cache: HashMap::new(),
        }
    }
}
impl Engine {
    pub fn get(&self, doc: &Document) -> Option<&Highlights> {
        let key = Key::for_doc(doc)?;
        self.cache.get(&doc.id).filter(|c| c.key == key)
    }
    pub fn poll(&mut self, documents: &[&Document]) -> (bool, Option<String>) {
        let keys: Vec<_> = documents.iter().filter_map(|d| Key::for_doc(d)).collect();
        self.cache.retain(|id, _| keys.iter().any(|k| k.id == *id));
        let mut changed = false;
        let mut error = None;
        if let Some(pending) = &self.pending
            && (!keys.contains(&pending.key)
                || pending.started.elapsed() > Duration::from_millis(500))
        {
            pending.cancel.store(1, Ordering::Relaxed);
        }
        if let Ok(response) = self.receiver.try_recv() {
            self.pending = None;
            if keys.contains(&response.key) {
                let spans = match response.result {
                    Ok(spans) => spans,
                    Err(reason) => {
                        error = Some(format!(
                            "Syntax highlighting unavailable for this revision: {reason}"
                        ));
                        Vec::new()
                    }
                };
                self.cache.insert(
                    response.key.id,
                    Highlights {
                        key: response.key,
                        spans,
                    },
                );
                changed = true;
            }
        }
        if self.pending.is_none()
            && let Some(key) = keys
                .into_iter()
                .find(|k| self.cache.get(&k.id).is_none_or(|c| c.key != *k))
            && let Some(doc) = documents.iter().find(|d| d.id == key.id)
        {
            let cancel = Arc::new(AtomicUsize::new(0));
            if self
                .sender
                .try_send(Request {
                    key: key.clone(),
                    text: doc.text.clone(),
                    cancel: cancel.clone(),
                })
                .is_ok()
            {
                self.pending = Some(InFlight {
                    key,
                    cancel,
                    started: Instant::now(),
                });
            }
        }
        (changed, error)
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grammars_cover_multiline_strings_comments_unicode_and_tsx() {
        let fixtures = [
            (
                "rust",
                "/* first\nsecond */\nfn main() { let x = \"🌍\"; }",
                "second",
                "comment",
            ),
            (
                "python",
                "text = \"\"\"first\n🌍 second\"\"\"\nprint(text)",
                "second",
                "string",
            ),
            (
                "javascript",
                "/* first\nsecond */\nconst x = <div>ok</div>;",
                "second",
                "comment",
            ),
            ("typescript", "const value: number = 42;", "number", "type"),
            (
                "tsx",
                "const X = () => <div title=\"hello\" />;",
                "hello",
                "string",
            ),
            ("json", "{\"value\": true}", "true", "constant"),
        ];
        for (language, text, token, expected) in fixtures {
            let spans = highlight(
                &configuration(language).unwrap(),
                text,
                &AtomicUsize::new(0),
            )
            .unwrap();
            let byte = text.find(token).unwrap();
            let style = spans
                .iter()
                .find(|s| s.start <= byte && s.end > byte)
                .unwrap();
            assert_eq!(NAMES[style.style], expected, "{language}");
        }
    }
    #[test]
    fn worker_discards_stale_revisions_and_retains_current_colors() {
        let mut engine = Engine::default();
        let mut doc = Document::from_text("fn before() {}\n");
        doc.path = Some("test.rs".into());
        engine.poll(&[&doc]);
        doc.insert("// comment\n", false);
        assert!(engine.get(&doc).is_none());
        let started = Instant::now();
        while engine.get(&doc).is_none() {
            let (_, error) = engine.poll(&[&doc]);
            assert!(error.is_none(), "{error:?}");
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(engine.get(&doc).unwrap().style_at(3), Some(0));
    }
}
