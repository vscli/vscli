//! Bounded background grammar highlighting. Results belong to one document revision.
use crate::document::{ByteChange, Document};
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
    epoch: u64,
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
            epoch: doc.text_epoch(),
            language: language(doc.path.as_deref()?)?,
        })
    }
}
pub fn language(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".hpp.in") || name.ends_with(".h.in") {
        return Some("cpp");
    }
    match path.extension()?.to_str()? {
        "c" | "i" => Some("c"),
        "cpp" | "cppm" | "cc" | "ccm" | "cxx" | "cxxm" | "c++" | "c++m" | "hpp" | "hh" | "hxx"
        | "h++" | "h" | "ii" | "ino" | "inl" | "ipp" | "ixx" | "tpp" | "txx" => Some("cpp"),
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
        "c" => (
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY.to_string(),
        ),
        "cpp" => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            ),
        ),
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
    mapping: Mapping,
    complete: bool,
}
impl Highlights {
    pub fn style_at(&self, byte: usize) -> Option<usize> {
        let byte = self.mapping.source_byte(byte)?;
        let index = self.spans.partition_point(|s| s.end <= byte);
        self.spans
            .get(index)
            .filter(|s| s.start <= byte)
            .map(|s| s.style)
    }
    fn rebase(&mut self, doc: &Document, key: &Key) -> bool {
        if self.key.language != key.language {
            return false;
        }
        let Some(changes) = doc.byte_changes_since(self.key.epoch) else {
            return false;
        };
        for change in changes {
            if !self.mapping.apply(change) {
                return false;
            }
        }
        if self.mapping.length != doc.text.len_bytes() {
            return false;
        }
        self.complete &= self.key == *key;
        self.key = key.clone();
        true
    }
}

#[derive(Clone, Debug)]
struct Piece {
    start: usize,
    end: usize,
    source: usize,
}
/// Only unchanged bytes retain provisional colors. Inserted/replaced bytes have
/// no source mapping; never paint them using an old token's absolute offsets.
struct Mapping {
    pieces: Vec<Piece>,
    length: usize,
}
impl Mapping {
    fn new(length: usize) -> Self {
        Self {
            pieces: if length == 0 {
                Vec::new()
            } else {
                vec![Piece {
                    start: 0,
                    end: length,
                    source: 0,
                }]
            },
            length,
        }
    }
    fn source_byte(&self, byte: usize) -> Option<usize> {
        let index = self.pieces.partition_point(|p| p.end <= byte);
        self.pieces
            .get(index)
            .filter(|p| p.start <= byte)
            .map(|p| p.source + byte - p.start)
    }
    fn apply(&mut self, change: &ByteChange) -> bool {
        let range = &change.range;
        if range.start > range.end || range.end > self.length {
            return false;
        }
        let Some(length) = self
            .length
            .checked_sub(range.len())
            .and_then(|n| n.checked_add(change.added))
        else {
            return false;
        };
        if length > MAX_BYTES {
            return false;
        }
        let shift = |byte: usize| byte - range.len() + change.added;
        let mut next = Vec::<Piece>::with_capacity(self.pieces.len() + 1);
        let mut push = |piece: Piece| {
            if let Some(last) = next.last_mut()
                && last.end == piece.start
                && last.source + last.end - last.start == piece.source
            {
                last.end = piece.end;
            } else {
                next.push(piece);
            }
        };
        for piece in &self.pieces {
            if piece.end <= range.start {
                push(piece.clone());
            } else if piece.start >= range.end {
                push(Piece {
                    start: shift(piece.start),
                    end: shift(piece.end),
                    source: piece.source,
                });
            } else {
                if piece.start < range.start {
                    push(Piece {
                        start: piece.start,
                        end: range.start,
                        source: piece.source,
                    });
                }
                if piece.end > range.end {
                    push(Piece {
                        start: range.start + change.added,
                        end: shift(piece.end),
                        source: piece.source + range.end - piece.start,
                    });
                }
            }
        }
        if next.len() > 512 {
            return false;
        }
        self.pieces = next;
        self.length = length;
        true
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
#[derive(Debug)]
enum Response {
    Parsing {
        key: Key,
        started: Instant,
    },
    Finished {
        key: Key,
        result: Result<Vec<Span>, String>,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cancellation {
    Superseded,
    Deadline,
}
struct InFlight {
    key: Key,
    cancel: Arc<AtomicUsize>,
    started: Instant,
    parsing: Option<Instant>,
    cancellation: Option<Cancellation>,
}
struct Failure {
    key: Key,
    attempts: usize,
    retry: Instant,
}
pub struct Engine {
    sender: SyncSender<Request>,
    receiver: Receiver<Response>,
    pending: Option<InFlight>,
    cache: HashMap<u64, Highlights>,
    failures: HashMap<u64, Failure>,
}
impl Default for Engine {
    fn default() -> Self {
        let (sender, requests) = mpsc::sync_channel::<Request>(1);
        let (responses, receiver) = mpsc::sync_channel(2);
        std::thread::spawn(move || {
            let mut configurations = HashMap::new();
            for request in requests {
                let result = (|| -> Result<Vec<Span>> {
                    if request.cancel.load(Ordering::Relaxed) != 0 {
                        bail!("Syntax request cancelled before setup");
                    }
                    if !configurations.contains_key(request.key.language) {
                        configurations
                            .insert(request.key.language, configuration(request.key.language)?);
                    }
                    if request.cancel.load(Ordering::Relaxed) != 0 {
                        bail!("Syntax request cancelled during setup");
                    }
                    let text = request.text.to_string();
                    // Cold grammar/query construction is bounded background setup,
                    // not part of the unchanged 500 ms parsing deadline.
                    responses.send(Response::Parsing {
                        key: request.key.clone(),
                        started: Instant::now(),
                    })?;
                    highlight(
                        &configurations[request.key.language],
                        &text,
                        &request.cancel,
                    )
                })()
                .map_err(|e| e.to_string());
                if responses
                    .send(Response::Finished {
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
            failures: HashMap::new(),
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
        let mut changed = false;
        let mut error = None;
        self.cache.retain(|id, cached| {
            let Some(key) = keys.iter().find(|key| key.id == *id) else {
                return false;
            };
            let doc = documents.iter().find(|doc| doc.id == *id).unwrap();
            changed |= cached.key != *key;
            cached.rebase(doc, key)
        });
        self.failures
            .retain(|_, failure| keys.contains(&failure.key));
        // At most one request and its two phase/completion messages are live.
        // Cancellation never releases the occupied slot before completion.
        for _ in 0..2 {
            let Ok(response) = self.receiver.try_recv() else {
                break;
            };
            match response {
                Response::Parsing { key, started } => {
                    if let Some(pending) = &mut self.pending
                        && pending.key == key
                    {
                        pending.parsing = Some(started);
                    }
                }
                Response::Finished { key, result } => {
                    if self
                        .pending
                        .as_ref()
                        .is_none_or(|pending| pending.key != key)
                    {
                        continue;
                    }
                    let pending = self.pending.take().unwrap();
                    if !keys.contains(&key) {
                        continue;
                    }
                    if pending.cancel.load(Ordering::Relaxed) != 0 {
                        // Cancellation is terminal acknowledgement of retired work,
                        // not an owned syntax error. Never publish even an Ok result.
                        // Current budget failures still consume the existing bounded
                        // retry policy; visibility supersession does not poison it.
                        if pending.cancellation == Some(Cancellation::Deadline) {
                            self.record_failure(key);
                        }
                        changed = true;
                        continue;
                    }
                    match result {
                        Ok(spans) => {
                            let doc = documents.iter().find(|doc| doc.id == key.id).unwrap();
                            self.failures.remove(&key.id);
                            self.cache.insert(
                                key.id,
                                Highlights {
                                    key,
                                    spans,
                                    mapping: Mapping::new(doc.text.len_bytes()),
                                    complete: true,
                                },
                            );
                        }
                        Err(reason) => {
                            self.record_failure(key);
                            error = Some(format!(
                                "Syntax refresh unavailable; retaining mapped colors where possible: {reason}"
                            ));
                        }
                    }
                    changed = true;
                }
            }
        }
        if let Some(pending) = &mut self.pending {
            if !keys.contains(&pending.key) {
                pending.cancellation = Some(Cancellation::Superseded);
                pending.cancel.store(1, Ordering::Relaxed);
            } else if pending.parsing.map_or_else(
                || pending.started.elapsed() > Duration::from_secs(5),
                |started| started.elapsed() > Duration::from_millis(500),
            ) {
                // Once superseded, becoming visible again cannot revive authority.
                pending.cancellation.get_or_insert(Cancellation::Deadline);
                pending.cancel.store(1, Ordering::Relaxed);
            }
        }
        if self.pending.is_none()
            && let Some(key) = keys.into_iter().find(|k| {
                self.cache.get(&k.id).is_none_or(|c| !c.complete)
                    && self
                        .failures
                        .get(&k.id)
                        .is_none_or(|f| f.attempts < 3 && Instant::now() >= f.retry)
            })
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
                    parsing: None,
                    cancellation: None,
                });
            }
        }
        (changed, error)
    }
    fn record_failure(&mut self, key: Key) {
        let attempts = self.failures.get(&key.id).map_or(1, |f| f.attempts + 1);
        self.failures.insert(
            key.id,
            Failure {
                key,
                attempts,
                retry: Instant::now() + Duration::from_secs(1),
            },
        );
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
    fn controlled(doc: &Document) -> (Engine, Receiver<Request>, SyncSender<Response>) {
        let (sender, requests) = mpsc::sync_channel(1);
        let (responses, receiver) = mpsc::sync_channel(2);
        let key = Key::for_doc(doc).unwrap();
        let spans = highlight(
            &configuration(key.language).unwrap(),
            &doc.text.to_string(),
            &AtomicUsize::new(0),
        )
        .unwrap();
        let mut cache = HashMap::new();
        cache.insert(
            doc.id,
            Highlights {
                key,
                spans,
                mapping: Mapping::new(doc.text.len_bytes()),
                complete: true,
            },
        );
        (
            Engine {
                sender,
                receiver,
                pending: None,
                cache,
                failures: HashMap::new(),
            },
            requests,
            responses,
        )
    }
    fn assert_token(engine: &Engine, doc: &Document, token: &str, name: &str) {
        let byte = doc.text.to_string().find(token).unwrap();
        let style = engine.get(doc).unwrap().style_at(byte).unwrap();
        assert_eq!(NAMES[style], name, "{token}");
    }
    #[test]
    fn mapped_colors_survive_rapid_multicursor_unicode_crlf_undo_and_redo() {
        use crate::document::Selection;
        let original = "// 🌍 heading\r\nconstexpr auto raw = R\"tag(first\r\n🌍 raw text)tag\";\r\n/* first\r\n猫 comment */\r\nint main() { return 42; }\r\n";
        let mut doc = Document::from_text(original);
        doc.path = Some("preview.cpp".into());
        doc.activate_view(1);
        let (mut engine, requests, responses) = controlled(&doc);
        let other = doc.text.byte_to_char(original.find("int main").unwrap());
        doc.set_selections(vec![Selection::caret(0), Selection::caret(other)]);
        let selections = doc.selections();
        let id = doc.id;
        for _ in 0..32 {
            doc.insert("e\u{301}🙂", false);
            engine.poll(&[&doc]);
            assert_token(&engine, &doc, "🌍 raw text", "string");
            assert_token(&engine, &doc, "猫 comment", "comment");
            assert_token(&engine, &doc, "return", "keyword");
            assert!(engine.get(&doc).unwrap().style_at(0).is_none());
        }
        let pending = requests.try_recv().unwrap();
        assert_eq!(pending.cancel.load(Ordering::Relaxed), 1);
        assert!(requests.try_recv().is_err()); // canceled request still owns its slot
        for _ in 0..32 {
            doc.undo();
            engine.poll(&[&doc]);
            assert_token(&engine, &doc, "🌍 raw text", "string");
            assert_token(&engine, &doc, "猫 comment", "comment");
        }
        assert_eq!(doc.id, id);
        assert_eq!(doc.text.to_string(), original);
        assert_eq!(doc.selections(), selections);
        for _ in 0..32 {
            doc.redo();
            engine.poll(&[&doc]);
            assert_token(&engine, &doc, "return", "keyword");
        }
        // Late pre-edit success cannot overwrite the mapped current geometry.
        responses
            .send(Response::Finished {
                key: pending.key,
                result: Ok(Vec::new()),
            })
            .unwrap();
        engine.poll(&[&doc]);
        assert_token(&engine, &doc, "🌍 raw text", "string");
        assert!(!engine.get(&doc).unwrap().complete);
        for _ in 0..32 {
            doc.undo();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.cpp");
        doc.save_to(&path, false).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    }
    #[test]
    fn mapping_drops_replaced_bytes_and_rejects_history_or_piece_budget_gaps() {
        let mut doc = Document::from_text("fn main() { let 猫 = \"🙂\"; return; }\r\n");
        doc.path = Some("mapping.rs".into());
        let (mut engine, _requests, _responses) = controlled(&doc);
        let start = doc.text.to_string().find("猫").unwrap();
        let position = doc.text.byte_to_char(start);
        doc.apply_changes(vec![(position..position + 1, "🌍long".into())]);
        engine.poll(&[&doc]);
        for byte in start..start + "🌍long".len() {
            assert!(engine.get(&doc).unwrap().style_at(byte).is_none());
        }
        assert_token(&engine, &doc, "return", "keyword");
        let before = doc.text_epoch();
        for _ in 0..257 {
            doc.insert("x", false);
        }
        assert!(doc.byte_changes_since(before).is_none());
        engine.poll(&[&doc]);
        assert!(engine.get(&doc).is_none()); // bounded history loss refuses stale coordinates
        let mut mapping = Mapping::new(2_000);
        for at in (1..=511).rev() {
            assert!(mapping.apply(&ByteChange {
                range: at * 2..at * 2,
                added: 1
            }));
        }
        assert!(!mapping.apply(&ByteChange {
            range: 1..1,
            added: 1
        }));
        assert!(!mapping.apply(&ByteChange {
            range: 0..usize::MAX,
            added: 1
        }));
    }
    #[test]
    fn journal_mapping_keeps_exact_source_bytes_through_transactions_and_reload() {
        let original = "// 猫🙂 e\u{301}\r\nfn alpha() { let 世界 = \"🌍\"; }\r\nfn beta() {}\r\n";
        let mut doc = Document::from_text(original);
        let mut mapping = Mapping::new(original.len());
        let mut epoch = doc.text_epoch();
        for step in 0..7 {
            match step {
                0 => doc.apply_changes(vec![(3..5, "changed🙂".into()), (20..24, "x\r\ny".into())]),
                1 => doc.insert("prefix猫\r\n", false),
                2 | 3 => doc.undo(),
                4 | 5 => doc.redo(),
                _ => doc.reload_content(Rope::from_str(&format!("new\r\n{}tail🙂", doc.text))),
            }
            for change in doc.byte_changes_since(epoch).unwrap() {
                assert!(mapping.apply(change));
            }
            epoch = doc.text_epoch();
            let current = doc.text.to_string();
            assert_eq!(mapping.length, current.len());
            for piece in &mapping.pieces {
                assert_eq!(
                    &current[piece.start..piece.end],
                    &original[piece.source..piece.source + piece.end - piece.start]
                );
                assert_eq!(mapping.source_byte(piece.start), Some(piece.source));
                assert_eq!(
                    mapping.source_byte(piece.end - 1),
                    Some(piece.source + piece.end - piece.start - 1)
                );
            }
        }
    }
    #[test]
    fn parse_failures_retain_preview_retry_boundedly_and_never_cache_blank_success() {
        let mut doc = Document::from_text("/* first\n🌍 comment */\nint main() { return 42; }");
        doc.path = Some("failure.cpp".into());
        let (mut engine, requests, responses) = controlled(&doc);
        doc.insert(" ", false);
        engine.poll(&[&doc]);
        for attempt in 1..=3 {
            let request = requests.try_recv().unwrap();
            responses
                .send(Response::Finished {
                    key: request.key,
                    result: Err("cancelled".into()),
                })
                .unwrap();
            let (_, error) = engine.poll(&[&doc]);
            assert!(error.unwrap().contains("retaining mapped colors"));
            assert_token(&engine, &doc, "🌍 comment", "comment");
            assert_eq!(engine.failures[&doc.id].attempts, attempt);
            assert!(requests.try_recv().is_err());
            engine.failures.get_mut(&doc.id).unwrap().retry = Instant::now();
            engine.poll(&[&doc]);
        }
        assert!(engine.pending.is_none());
        assert!(requests.try_recv().is_err());
        doc.insert(" ", false);
        engine.poll(&[&doc]);
        assert!(requests.try_recv().is_ok()); // new revision gets a fresh bounded attempt
    }
    #[test]
    fn cold_setup_has_a_separate_deadline_but_parsing_keeps_500ms_budget() {
        let mut doc = Document::from_text("int main() { return 42; }");
        doc.path = Some("cold.cpp".into());
        let (mut engine, requests, responses) = controlled(&doc);
        doc.insert(" ", false);
        engine.poll(&[&doc]);
        let request = requests.try_recv().unwrap();
        engine.pending.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
        engine.poll(&[&doc]);
        assert_eq!(request.cancel.load(Ordering::Relaxed), 0);
        responses
            .send(Response::Parsing {
                key: request.key.clone(),
                started: Instant::now() - Duration::from_millis(501),
            })
            .unwrap();
        engine.poll(&[&doc]);
        assert_eq!(request.cancel.load(Ordering::Relaxed), 1);
        assert_token(&engine, &doc, "return", "keyword");
        assert!(engine.pending.is_some());
        let pending = engine.pending.as_mut().unwrap();
        pending.cancel.store(0, Ordering::Relaxed);
        pending.parsing = None;
        pending.started = Instant::now() - Duration::from_secs(6);
        engine.poll(&[&doc]);
        assert_eq!(request.cancel.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn rendered_multiline_colors_stay_stable_while_the_worker_is_held() {
        use crate::{app::App, keys::Profile};
        use ratatui::{Terminal, backend::TestBackend};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("render.cpp");
        std::fs::write(&path, "// heading\r\nconstexpr auto value = R\"tag(first\r\nrawStableMarker)tag\";\r\n/* first\r\ncommentStableMarker */\r\n").unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.open(&path).unwrap();
        let (engine, _requests, _responses) = controlled(app.doc());
        app.syntax = engine;
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let tokens = ["rawStableMarker", "commentStableMarker"];
        let colors = [app.theme.token(1), app.theme.token(0)];
        for cycle in 0..8 {
            if cycle > 0 {
                app.doc_mut().insert("e\u{301}🙂", false);
            }
            app.poll();
            terminal
                .draw(|frame| crate::ui::draw(frame, &mut app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            for (token, color) in tokens.iter().zip(colors) {
                let found = (0..30)
                    .find_map(|y| {
                        (0..120 - token.len() as u16).find_map(|x| {
                            token
                                .chars()
                                .enumerate()
                                .all(|(i, ch)| buffer[(x + i as u16, y)].symbol() == ch.to_string())
                                .then_some((x, y))
                        })
                    })
                    .unwrap();
                for i in 0..token.len() as u16 {
                    assert_eq!(
                        buffer[(found.0 + i, found.1)].fg,
                        color,
                        "{token}, frame {cycle}"
                    );
                }
            }
        }
        assert!(!app.syntax.get(app.doc()).unwrap().complete);
    }
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
    fn c_and_cpp_queries_cover_preprocessors_templates_raw_strings_and_unicode() {
        for (language, text, tokens) in [
            (
                "c",
                "#include <stdio.h>\n/* first\n🌍 comment */\nint main(void) { return 42; }",
                vec![
                    ("#include", "keyword"),
                    ("stdio.h", "string"),
                    ("🌍", "comment"),
                    ("int", "type"),
                    ("main", "function"),
                    ("return", "keyword"),
                    ("42", "number"),
                ],
            ),
            (
                "cpp",
                "template<typename T> class Box {};\nconstexpr auto text = R\"tag(first\n🌍 raw)tag\";\nint main() { /* 🌍 comment */ return 42; }",
                vec![
                    ("template", "keyword"),
                    ("typename", "keyword"),
                    ("Box", "type"),
                    ("constexpr", "keyword"),
                    ("auto", "type"),
                    ("🌍 raw", "string"),
                    ("🌍 comment", "comment"),
                    ("main", "function"),
                    ("return", "keyword"),
                    ("42", "number"),
                ],
            ),
        ] {
            let spans = highlight(
                &configuration(language).unwrap(),
                text,
                &AtomicUsize::new(0),
            )
            .unwrap();
            for (token, expected) in tokens {
                let byte = text.find(token).unwrap();
                let span = spans
                    .iter()
                    .find(|s| s.start <= byte && s.end > byte)
                    .unwrap();
                assert_eq!(NAMES[span.style], expected, "{language}: {token}");
            }
            assert!(
                spans
                    .iter()
                    .all(|s| text.is_char_boundary(s.start) && text.is_char_boundary(s.end))
            );
        }
    }
    #[test]
    fn cpp_headers_and_modules_select_the_native_grammar() {
        for name in [
            "main.cpp",
            "module.cppm",
            "module.ixx",
            "file.cc",
            "file.cxx",
            "file.c++",
            "file.hpp",
            "file.h",
            "file.hpp.in",
            "file.h.in",
        ] {
            assert_eq!(language(Path::new(name)), Some("cpp"), "{name}");
        }
        for name in ["main.c", "preprocessed.i"] {
            assert_eq!(language(Path::new(name)), Some("c"));
        }
        for name in ["notes.txt", "config.in", "kernel.cu"] {
            assert_eq!(language(Path::new(name)), None);
        }
    }
    #[test]
    fn cpp_worker_rejects_old_revisions_and_respects_the_document_budget() {
        let mut engine = Engine::default();
        let mut doc = Document::from_text("constexpr int value = 42;\n");
        doc.path = Some("main.cpp".into());
        engine.poll(&[&doc]);
        doc.insert("/* 🌍 */\n", false);
        assert!(engine.get(&doc).is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        while engine.get(&doc).is_none() {
            let (_, error) = engine.poll(&[&doc]);
            assert!(error.is_none(), "{error:?}");
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(engine.get(&doc).unwrap().style_at(3), Some(0));
        let mut oversized = Document::from_text(&"x".repeat(MAX_BYTES + 1));
        oversized.path = Some("large.cpp".into());
        engine.poll(&[&oversized]);
        assert!(engine.get(&oversized).is_none());
        assert!(engine.pending.is_none());
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

    #[test]
    fn superseded_setup_completion_cannot_publish_error_or_success_after_visibility_aba() {
        for successful in [false, true] {
            let mut doc = Document::from_text("// 猫🙂 retained\r\nint value = 42;\r\n");
            doc.path = Some("superseded.cpp".into());
            doc.move_to(doc.len(), false);
            doc.insert("undo pending λ", false);
            let changed_text = doc.text.to_string();
            doc.undo();
            let original = doc.text.to_string();
            let id = doc.id;
            let epoch = doc.text_epoch();
            let revision = doc.revision;
            let cursor = doc.cursor;
            let generation = doc.save_generation();
            let (mut engine, requests, responses) = controlled(&doc);
            let spans = engine.cache[&doc.id].spans.clone();
            engine.cache.get_mut(&doc.id).unwrap().complete = false;
            engine.poll(&[&doc]);
            let old = requests.try_recv().unwrap();
            assert!(requests.try_recv().is_err());
            assert_eq!(engine.poll(&[]).1, None);
            assert_eq!(old.cancel.load(Ordering::Relaxed), 1);
            assert!(engine.pending.is_some());
            assert_eq!(engine.poll(&[&doc]).1, None);
            assert!(requests.try_recv().is_err()); // old actual slot still held
            responses
                .send(Response::Finished {
                    key: old.key,
                    result: if successful {
                        Ok(spans.clone())
                    } else {
                        Err("Syntax request cancelled during setup".into())
                    },
                })
                .unwrap();
            assert_eq!(engine.poll(&[&doc]).1, None);
            assert!(engine.get(&doc).is_none()); // canceled Ok is also unauthorized
            assert!(!engine.failures.contains_key(&doc.id));
            let fresh = requests.try_recv().unwrap();
            assert!(!Arc::ptr_eq(&old.cancel, &fresh.cancel));
            assert_eq!(fresh.cancel.load(Ordering::Relaxed), 0);
            assert!(requests.try_recv().is_err());
            responses
                .send(Response::Finished {
                    key: fresh.key,
                    result: Ok(spans),
                })
                .unwrap();
            assert_eq!(engine.poll(&[&doc]).1, None);
            assert_token(&engine, &doc, "retained", "comment");
            assert_eq!(
                (
                    doc.id,
                    doc.text_epoch(),
                    doc.revision,
                    doc.cursor,
                    doc.save_generation()
                ),
                (id, epoch, revision, cursor, generation)
            );
            assert_eq!(doc.text.to_string(), original);
            doc.redo();
            assert_eq!(doc.text.to_string(), changed_text);
            doc.undo();
            assert_eq!(doc.text.to_string(), original);
        }
    }

    #[test]
    fn canceled_budget_completion_is_quiet_but_retains_three_attempt_backoff() {
        let mut doc = Document::from_text("// 猫🙂 retained\r\nint main() { return 42; }\r\n");
        doc.path = Some("budget.cpp".into());
        let (mut engine, requests, responses) = controlled(&doc);
        doc.insert(" ", false);
        engine.poll(&[&doc]);
        for attempt in 1..=3 {
            let request = requests.try_recv().unwrap();
            engine.pending.as_mut().unwrap().started = Instant::now() - Duration::from_secs(6);
            assert_eq!(engine.poll(&[&doc]).1, None);
            assert_eq!(request.cancel.load(Ordering::Relaxed), 1);
            assert!(requests.try_recv().is_err());
            responses
                .send(Response::Finished {
                    key: request.key,
                    result: Err("Syntax request cancelled during setup".into()),
                })
                .unwrap();
            assert_eq!(engine.poll(&[&doc]).1, None);
            assert_eq!(engine.failures[&doc.id].attempts, attempt);
            assert_token(&engine, &doc, "retained", "comment");
            assert!(requests.try_recv().is_err());
            engine.failures.get_mut(&doc.id).unwrap().retry = Instant::now();
            engine.poll(&[&doc]);
        }
        assert!(engine.pending.is_none());
        assert!(requests.try_recv().is_err());
        doc.insert(" ", false);
        engine.poll(&[&doc]);
        assert!(requests.try_recv().is_ok()); // next document version requalifies
    }

    #[test]
    fn canceled_syntax_setup_keeps_saved_command_notice_but_current_error_still_reports() {
        use crate::{app::App, keys::Profile};
        use serde_json::json;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notice.cpp");
        let disk = "// 猫🙂 saved\r\nint main() { return 42; }\r\n";
        std::fs::write(&path, disk).unwrap();
        let mut app = App::new(directory.path().into(), Profile::Linux);
        app.settings = crate::settings::Settings::from_values(
            json!({
                "vscli.languageServer.enabled":false, "breadcrumbs.enabled":false,
                "files.autoSave":"off", "editor.formatOnSave":false,
                "editor.codeActionsOnSave":{}, "editor.quickSuggestions":false,
                "editor.parameterHints.enabled":false,
            })
            .as_object()
            .unwrap()
            .clone(),
            "syntax cancellation status fixture",
        )
        .unwrap();
        app.extension_node = "vscli-syntax-fixture-node-missing".into();
        app.open(&path).unwrap();
        let (engine, requests, responses) = controlled(app.doc());
        app.syntax = engine;
        app.doc_mut().insert(" ", false);
        let before = app.doc().text.to_string();
        let epoch = app.doc().text_epoch();
        let id = app.doc().id;
        let cursor = app.doc().cursor;
        let generation = app.doc().save_generation();
        app.poll();
        let old = requests.try_recv().unwrap();
        app.syntax.pending.as_mut().unwrap().started = Instant::now() - Duration::from_secs(6);
        app.poll();
        assert_eq!(old.cancel.load(Ordering::Relaxed), 1);
        let notice = "Saved; command-bearing save action skipped in full";
        app.message = notice.into();
        responses
            .send(Response::Finished {
                key: old.key,
                result: Err("Syntax request cancelled during setup".into()),
            })
            .unwrap();
        app.poll();
        assert_eq!(app.message, notice);
        assert_eq!(app.doc().text.to_string(), before);
        assert_eq!(
            (
                app.doc().id,
                app.doc().text_epoch(),
                app.doc().cursor,
                app.doc().save_generation()
            ),
            (id, epoch, cursor, generation)
        );
        assert_eq!(std::fs::read(&path).unwrap(), disk.as_bytes());
        app.syntax.failures.get_mut(&id).unwrap().retry = Instant::now();
        app.poll();
        let current = requests.try_recv().unwrap();
        assert_eq!(current.cancel.load(Ordering::Relaxed), 0);
        responses
            .send(Response::Finished {
                key: current.key,
                result: Err("actual current grammar configuration failure".into()),
            })
            .unwrap();
        app.poll();
        assert!(
            app.message
                .contains("actual current grammar configuration failure")
        );
        assert_eq!(app.syntax.failures[&id].attempts, 2);
        assert_eq!(std::fs::read(&path).unwrap(), disk.as_bytes());
        app.doc_mut().undo();
        assert_eq!(app.doc().text.to_string(), disk);
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), before);
    }
}
