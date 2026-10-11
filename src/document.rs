use anyhow::{Context, Result, bail};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
mod comments;
mod editing;
pub(crate) mod editor_options;
pub(crate) use editor_options::EditorOptionsPatch;
mod folding;
pub use comments::CommentOperation;
pub use folding::{
    FoldAction, FoldClear, FoldCommit, FoldPrepared, FoldSnapshot, FoldViewInsertion,
};
pub(crate) mod graphemes;
mod indentation;
mod snippets;
mod typing;
pub(crate) mod workspace;
pub(crate) use workspace::{
    WorkspaceHistoryDirection, WorkspaceHistoryGroup, WorkspaceSource, WorkspaceTextPrepared,
};

pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const HISTORY_LIMIT: usize = 1000;
const DEFAULT_EOL: &str = if cfg!(windows) { "\r\n" } else { "\n" };

pub fn read_disk(path: &Path) -> Result<Option<Rope>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("Cannot read file"),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("Not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_FILE_BYTES {
        bail!("This alpha supports files up to 32 MiB");
    }
    read_text(file, MAX_FILE_BYTES).map(Some)
}

struct RetryInterrupted<R>(R);
impl<R: Read> Read for RetryInterrupted<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.0.read(buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => return result,
            }
        }
    }
}

fn read_text(reader: impl Read, limit: u64) -> Result<Rope> {
    let reader = std::io::BufReader::with_capacity(64 * 1024, reader.take(limit + 1));
    let content = Rope::from_reader(RetryInterrupted(reader))
        .context("Could not read UTF-8 text (binary and other encodings are not supported yet)")?;
    if content.len_bytes() as u64 > limit {
        bail!("File grew beyond the supported size while reading");
    }
    if content.chunks().any(|chunk| chunk.contains('\0')) {
        bail!("Binary file: refusing to edit NUL-containing content");
    }
    Ok(content)
}

fn infer_eol<'a>(content: &Rope, fallback: &'a str) -> &'a str {
    let mut previous_cr = false;
    let mut has_break = false;
    for chunk in content.chunks() {
        if (previous_cr && chunk.starts_with('\n')) || chunk.contains("\r\n") {
            return "\r\n";
        }
        has_break |= chunk.contains(['\r', '\n']);
        previous_cr = chunk.ends_with('\r');
    }
    if has_break { "\n" } else { fallback }
}

fn reader_matches(mut reader: impl Read, expected: &Rope) -> Result<bool> {
    let mut buffer = [0; 16 * 1024];
    for chunk in expected.chunks() {
        for bytes in chunk.as_bytes().chunks(buffer.len()) {
            let actual = &mut buffer[..bytes.len()];
            match reader.read_exact(actual) {
                Ok(()) if actual == bytes => {}
                Ok(()) => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    // Detect appended content without allocating a second file-sized buffer.
    match reader.read_exact(&mut buffer[..1]) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(true),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn disk_matches(path: &Path, expected: Option<&Rope>) -> Result<bool> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(expected.is_none()),
        Err(error) => return Err(error).context("Cannot check current file before saving"),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("Not a regular file: {}", path.display());
    }
    let Some(expected) = expected else {
        return Ok(false);
    };
    if metadata.len() != expected.len_bytes() as u64 {
        return Ok(false);
    }
    reader_matches(std::io::BufReader::new(file), expected)
        .context("Cannot check current file before saving")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub cursor: usize,
    pub anchor: Option<usize>,
    #[serde(default)]
    pub desired_column: Option<usize>,
}
impl Selection {
    pub fn caret(cursor: usize) -> Self {
        Self {
            cursor,
            anchor: None,
            desired_column: None,
        }
    }
    pub fn range(&self) -> Range<usize> {
        let anchor = self.anchor.unwrap_or(self.cursor);
        anchor.min(self.cursor)..anchor.max(self.cursor)
    }
}

#[derive(Clone)]
struct Snapshot {
    history_identity: std::sync::Arc<()>,
    workspace_group: Option<std::sync::Arc<WorkspaceHistoryGroup>>,
    text: Rope,
    cursor: usize,
    anchor: Option<usize>,
    secondary: Vec<Selection>,
    revision: u64,
    view_id: u64,
    changes: Vec<PositionChange>,
    snippet: Option<snippets::Session>,
    snippet_generation: u64,
    pairs: typing::Pairs,
    // Snippet navigation occurs after the edit. Preserve the edit's endpoint
    // selections instead of turning that later navigation into redo state.
    after_selections: Option<Vec<Selection>>,
}

#[derive(Clone, Debug, Default)]
pub struct ViewState {
    pub cursor: usize,
    pub anchor: Option<usize>,
    pub secondary: Vec<Selection>,
    pub top: usize,
    pub left: usize,
    desired_column: Option<usize>,
    cursor_history: Vec<Vec<Selection>>,
    snippet: Option<snippets::Session>,
    snippet_generation: u64,
    pairs: typing::Pairs,
    folding: folding::View,
    editor_owner: std::sync::Arc<()>,
    line_numbers_override: Option<crate::settings::LineNumbers>,
    cursor_style_override: Option<u8>,
    explicit_viewport: bool,
}
#[derive(Clone)]
pub(crate) struct ByteChange {
    pub range: Range<usize>,
    pub added: usize,
}
impl ByteChange {
    fn inverse(&self) -> Self {
        Self {
            range: self.range.start..self.range.start + self.added,
            added: self.range.len(),
        }
    }
}
#[derive(Clone)]
struct PositionChange {
    range: Range<usize>,
    added: usize,
    bytes: ByteChange,
}
impl PositionChange {
    fn inverse(&self) -> Self {
        Self {
            range: self.range.start..self.range.start + self.added,
            added: self.range.len(),
            bytes: self.bytes.inverse(),
        }
    }
    fn map(&self, view: &mut ViewState) {
        view.explicit_viewport = false;
        view.map_snippet(&self.range, self.added);
        view.pairs.map(&self.range, self.added);
        view.cursor = map_position(view.cursor, &self.range, self.added);
        view.anchor = view
            .anchor
            .map(|p| map_position(p, &self.range, self.added));
        for selection in &mut view.secondary {
            selection.cursor = map_position(selection.cursor, &self.range, self.added);
            selection.anchor = selection
                .anchor
                .map(|p| map_position(p, &self.range, self.added));
            selection.desired_column = None;
        }
        view.desired_column = None;
        view.cursor_history.clear();
    }
}

/// Immutable input and publication proof for a background native save. Cloning
/// shares Rope storage; capture and proof checks perform no filesystem work.
#[derive(Clone)]
pub struct SaveSnapshot {
    document_id: u64,
    source_path: Option<PathBuf>,
    target: PathBuf,
    revision: u64,
    text_epoch: u64,
    saved_revision: u64,
    save_generation: u64,
    next_save_generation: u64,
    text: Rope,
    baseline: Option<Rope>,
}
impl SaveSnapshot {
    pub fn document_id(&self) -> u64 {
        self.document_id
    }
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }
    pub fn target(&self) -> &Path {
        &self.target
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn text_epoch(&self) -> u64 {
        self.text_epoch
    }
    pub fn saved_revision(&self) -> u64 {
        self.saved_revision
    }
    pub fn save_generation(&self) -> u64 {
        self.save_generation
    }
    pub fn text(&self) -> &Rope {
        &self.text
    }
    pub fn baseline(&self) -> Option<&Rope> {
        self.baseline.as_ref()
    }
}

impl std::fmt::Debug for SaveSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SaveSnapshot")
            .field("document_id", &self.document_id)
            .field("source_path", &self.source_path)
            .field("target", &self.target)
            .field("revision", &self.revision)
            .field("text_epoch", &self.text_epoch)
            .field("saved_revision", &self.saved_revision)
            .field("save_generation", &self.save_generation)
            .field("text_bytes", &self.text.len_bytes())
            .field(
                "baseline_bytes",
                &self.baseline.as_ref().map(Rope::len_bytes),
            )
            .finish()
    }
}

fn validate_snapshot_path(path: &Path, canonical: bool) -> Result<()> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() || bytes.len() > 4096 || bytes.contains(&0) || path.file_name().is_none() {
        bail!("Save paths require a filename and 1–4096 bytes without NUL");
    }
    if canonical
        && (!path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            }))
    {
        bail!("Save receipt requires an absolute canonical destination");
    }
    Ok(())
}

/// Bounded, normalized provider content prepared away from input/rendering.
/// Its Rope is private so publication cannot receive an unchecked source.
#[derive(Debug)]
pub struct PreparedVirtualContent {
    text: Rope,
}
impl PreparedVirtualContent {
    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }
}

/// Exact retained editor view authority, independent of focus and numeric-ID ABA.
#[derive(Clone)]
pub(crate) struct EditorViewProof {
    document: u64,
    id: u64,
    owner: std::sync::Arc<()>,
}
impl EditorViewProof {
    pub(crate) fn view_id(&self) -> u64 {
        self.id
    }
}

pub struct Document {
    pub id: u64,
    pub text: Rope,
    view: ViewState,
    active_view: u64,
    other_views: std::collections::HashMap<u64, ViewState>,
    pub path: Option<PathBuf>,
    virtual_uri: Option<String>,
    untitled_uri: Option<String>,
    associated_path: Option<PathBuf>,
    api_created_untitled: bool,
    language_override: Option<String>,
    language_epoch: u64,
    pub revision: u64,
    pub saved_revision: u64,
    pub disk_content: Option<Rope>,
    pub eol: String,
    pub tab_size: usize,
    pub insert_spaces: bool,
    indent_size: editor_options::IndentOption,
    editor_defaults: editor_options::Defaults,
    pub line_numbers: crate::settings::LineNumbers,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    next_revision: u64,
    text_epoch: u64,
    save_generation: u64,
    byte_changes: std::collections::VecDeque<(u64, ByteChange)>,
    folding_policy: crate::folding_controller::Lifetime,
    folding_changes: std::collections::VecDeque<folding::Change>,
    typing: Option<(Instant, usize)>,
    typing_context: typing::ContextCache,
    language_configuration: Option<std::sync::Arc<crate::language_configuration::Configuration>>,
}

impl std::ops::Deref for Document {
    type Target = ViewState;
    fn deref(&self) -> &ViewState {
        &self.view
    }
}
impl std::ops::DerefMut for Document {
    fn deref_mut(&mut self) -> &mut ViewState {
        &mut self.view
    }
}

static NEXT_DOCUMENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Default for Document {
    fn default() -> Self {
        Self::from_text("")
    }
}

impl Document {
    /// Read a view without activating it or changing undo grouping.
    pub fn view_state(&self, id: Option<u64>) -> &ViewState {
        match id {
            Some(id) if id != self.active_view => self.other_views.get(&id).unwrap_or(&self.view),
            _ => &self.view,
        }
    }

    pub(crate) fn open_existing_bounded(path: &Path, limit: u64) -> Result<Self> {
        let path = absolute_path(path)?;
        if !fs::metadata(&path)?.is_file() {
            bail!("Session target is not a regular file");
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = options.open(&path)?;
        let metadata = file.metadata()?;
        let limit = limit.min(MAX_FILE_BYTES);
        if !metadata.is_file() || metadata.len() > limit {
            bail!("Session file exceeds remaining read budget or is not regular");
        }
        let content = read_text(file, limit)?;
        let mut doc = Self::from_rope(content.clone());
        doc.path = Some(path);
        doc.disk_content = Some(content);
        Ok(doc)
    }

    pub fn activate_view(&mut self, id: u64) {
        self.switch_view(id, true);
    }
    pub fn display_view(&mut self, id: u64) {
        self.switch_view(id, false);
    }
    fn switch_view(&mut self, id: u64, user_action: bool) {
        if id == self.active_view {
            return;
        }
        if user_action {
            self.break_group();
        }
        let next = self.other_views.remove(&id).unwrap_or_else(|| {
            let mut view = self.view.clone();
            view.editor_owner = std::sync::Arc::new(());
            view.line_numbers_override = None;
            view.cursor_style_override = None;
            view.explicit_viewport = false;
            view.cursor_history.clear();
            view.snippet = None;
            view.pairs = typing::Pairs::default();
            view.folding = folding::View::default();
            view
        });
        let previous = std::mem::replace(&mut self.view, next);
        self.other_views.insert(self.active_view, previous);
        self.active_view = id;
    }
    pub fn remove_view(&mut self, id: u64) {
        self.other_views.remove(&id);
        if self.active_view == id {
            self.active_view = 0;
            self.view.folding = folding::View::default();
        }
    }
    pub(crate) fn text_epoch(&self) -> u64 {
        self.text_epoch
    }
    /// Counts successful persistence operations, independently of dirty state or Undo.
    pub(crate) fn save_generation(&self) -> u64 {
        self.save_generation
    }
    pub(crate) fn byte_changes_since(
        &self,
        epoch: u64,
    ) -> Option<impl Iterator<Item = &ByteChange>> {
        if epoch > self.text_epoch
            || self
                .byte_changes
                .front()
                .is_some_and(|(first, _)| epoch < first - 1)
        {
            return None;
        }
        Some(
            self.byte_changes
                .iter()
                .filter(move |(e, _)| *e > epoch)
                .map(|(_, c)| c),
        )
    }
    fn record_byte_change(&mut self, change: ByteChange) {
        self.text_epoch += 1;
        self.byte_changes.push_back((self.text_epoch, change));
        if self.byte_changes.len() > 256 {
            self.byte_changes.pop_front();
        }
    }
    fn record_change(&mut self, range: Range<usize>, added: usize, added_bytes: usize) {
        self.view.map_snippet(&range, added);
        self.view.pairs.map(&range, added);
        let bytes = ByteChange {
            range: self.text.char_to_byte(range.start)..self.text.char_to_byte(range.end),
            added: added_bytes,
        };
        self.record_byte_change(bytes.clone());
        self.record_folding_change(range.clone(), added);
        let change = PositionChange {
            range,
            added,
            bytes,
        };
        for view in self.other_views.values_mut() {
            change.map(view);
        }
        if let Some(snapshot) = self.undo.last_mut() {
            snapshot.changes.push(change);
        }
    }
    fn replace_undo_snapshot(&mut self, mut original: Snapshot) {
        if let Some(snapshot) = self.undo.last_mut() {
            original.changes = std::mem::take(&mut snapshot.changes);
            *snapshot = original;
        }
    }
    pub fn from_text(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text))
    }

    pub fn from_rope(text: Rope) -> Self {
        let eol = infer_eol(&text, DEFAULT_EOL).into();
        Self {
            text,
            id: NEXT_DOCUMENT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            view: ViewState::default(),
            active_view: 0,
            other_views: std::collections::HashMap::new(),
            path: None,
            virtual_uri: None,
            untitled_uri: None,
            associated_path: None,
            api_created_untitled: false,
            language_override: None,
            language_epoch: 0,
            revision: 0,
            saved_revision: 0,
            disk_content: None,
            eol,
            tab_size: 4,
            insert_spaces: true,
            indent_size: editor_options::IndentOption::default(),
            editor_defaults: editor_options::Defaults::default(),
            line_numbers: crate::settings::LineNumbers::On,
            undo: Vec::new(),
            redo: Vec::new(),
            next_revision: 1,
            text_epoch: 0,
            save_generation: 0,
            byte_changes: std::collections::VecDeque::new(),
            folding_policy: crate::folding_controller::Lifetime::default(),
            folding_changes: std::collections::VecDeque::new(),
            typing: None,
            typing_context: typing::ContextCache::default(),
            language_configuration: None,
        }
    }

    /// API-created unsaved content, without a synthetic insertion/Undo entry.
    pub fn from_untitled(
        uri: Option<String>,
        content: &str,
        language: Option<String>,
        associated_path: Option<PathBuf>,
    ) -> Result<Self> {
        if let Some(uri) = &uri
            && (uri.len() > 4096
                || uri.contains('\0')
                || !uri
                    .split_once(':')
                    .is_some_and(|(scheme, _)| scheme == "untitled"))
        {
            bail!("Untitled resource requires a bounded untitled URI");
        }
        if let Some(path) = &associated_path {
            validate_snapshot_path(path, false)?;
        }
        if language
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
        {
            bail!("Document language identifier exceeds bounds");
        }
        if content.len() as u64 > MAX_FILE_BYTES || content.contains('\0') {
            bail!("Untitled content exceeds 32 MiB or contains NUL");
        }
        // PieceTreeTextBufferBuilder chooses CRLF only when CR/CRLF account
        // for more than half the line breaks; a tie chooses LF.
        let mut total = 0usize;
        let mut cr = 0usize;
        let mut chars = content.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                total += 1;
                cr += 1;
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            } else if ch == '\n' {
                total += 1;
            }
        }
        let eol = if total == 0 {
            DEFAULT_EOL
        } else if cr > total / 2 {
            "\r\n"
        } else {
            "\n"
        };
        let mut builder = ropey::RopeBuilder::new();
        let mut start = 0;
        let mut breaks = content.char_indices().peekable();
        while let Some((at, ch)) = breaks.next() {
            if ch == '\r' || ch == '\n' {
                builder.append(&content[start..at]);
                builder.append(eol);
                start = at + 1;
                if ch == '\r' && breaks.peek().is_some_and(|(_, next)| *next == '\n') {
                    let (at, _) = breaks.next().expect("peeked LF");
                    start = at + 1;
                }
            }
        }
        builder.append(&content[start..]);
        let text = builder.finish();
        if text.len_bytes() as u64 > MAX_FILE_BYTES {
            bail!("Normalized untitled content exceeds 32 MiB");
        }
        let mut doc = Self::from_rope(text);
        doc.untitled_uri = Some(uri.unwrap_or_else(|| format!("untitled:Untitled-{}", doc.id)));
        doc.associated_path = associated_path;
        doc.api_created_untitled = true;
        doc.revision = 1;
        doc.next_revision = 2;
        doc.language_override = language;
        doc.eol = eol.into();
        Ok(doc)
    }
    pub fn untitled_uri(&self) -> Option<&str> {
        self.untitled_uri.as_deref()
    }
    pub fn associated_path(&self) -> Option<&Path> {
        self.associated_path.as_deref()
    }
    pub(crate) fn api_created_untitled(&self) -> bool {
        self.api_created_untitled
    }

    /// A provider-owned resource. Content uses the provider API's LF line model.
    pub fn from_virtual(uri: String, content: &str) -> Result<Self> {
        let scheme = uri.split_once(':').map(|(scheme, _)| scheme);
        if uri.len() > 4096
            || uri.contains('\0')
            || !scheme.is_some_and(|scheme| {
                !scheme.is_empty()
                    && scheme.as_bytes()[0].is_ascii_alphabetic()
                    && scheme
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || b"+-.".contains(&ch))
                    && !scheme.eq_ignore_ascii_case("file")
                    && !scheme.eq_ignore_ascii_case("untitled")
            })
        {
            bail!("Virtual document requires a bounded non-file resource URI");
        }
        let mut document = Self::from_rope(Self::prepare_virtual_content(content)?.text);
        document.virtual_uri = Some(uri);
        document.eol = "\n".into();
        Ok(document)
    }

    pub fn virtual_uri(&self) -> Option<&str> {
        self.virtual_uri.as_deref()
    }
    pub fn is_read_only(&self) -> bool {
        self.virtual_uri.is_some()
    }
    pub fn language_override(&self) -> Option<&str> {
        self.language_override.as_deref()
    }
    pub(crate) fn check_language_override(&self, language: Option<&str>) -> Result<()> {
        if language
            .is_some_and(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
        {
            bail!("Document language identifier exceeds bounds");
        }
        if self.language_override.as_deref() != language {
            self.language_epoch
                .checked_add(1)
                .context("Document language lifetime exhausted")?;
        }
        Ok(())
    }
    pub fn set_language_override(&mut self, language: Option<String>) -> Result<()> {
        if language
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
        {
            bail!("Document language identifier exceeds bounds");
        }
        if self.language_override != language {
            let epoch = self
                .language_epoch
                .checked_add(1)
                .context("Document language lifetime exhausted")?;
            self.retire_typing_pairs();
            self.retire_folding_policy();
            self.language_override = language;
            self.language_epoch = epoch;
        }
        Ok(())
    }
    pub(crate) fn language_epoch(&self) -> u64 {
        self.language_epoch
    }
    pub(crate) fn editor_view_identity(&self) -> EditorViewProof {
        self.editor_view_identity_for(self.active_view)
            .expect("active view retained")
    }
    pub(crate) fn editor_view_identity_for(&self, id: u64) -> Option<EditorViewProof> {
        let view = if id == self.active_view {
            Some(&self.view)
        } else {
            self.other_views.get(&id)
        }?;
        Some(EditorViewProof {
            document: self.id,
            id,
            owner: view.editor_owner.clone(),
        })
    }
    pub(crate) fn editor_view_stamp_for(&self, id: u64) -> Option<(u64, usize)> {
        let proof = self.editor_view_identity_for(id)?;
        Some((id, std::sync::Arc::as_ptr(&proof.owner) as usize))
    }
    pub(crate) fn editor_selection_count_for(&self, id: u64) -> Option<usize> {
        let view = if id == self.active_view {
            Some(&self.view)
        } else {
            self.other_views.get(&id)
        }?;
        Some(view.secondary.len() + 1)
    }
    pub(crate) fn reserve_editor_view(&mut self, id: u64) -> Result<()> {
        if id != self.active_view && !self.other_views.contains_key(&id) {
            self.other_views
                .try_reserve(1)
                .context("Cannot reserve native editor view")?;
        }
        Ok(())
    }
    pub(crate) fn editor_view_stamp(&self) -> (u64, usize) {
        self.editor_view_stamp_for(self.active_view)
            .expect("active view retained")
    }
    pub(crate) fn editor_view_active_current(&self, proof: &EditorViewProof) -> bool {
        self.active_view == proof.id && self.editor_view_current(proof)
    }
    pub(crate) fn editor_view_current(&self, proof: &EditorViewProof) -> bool {
        if proof.document != self.id {
            return false;
        }
        let view = if proof.id == self.active_view {
            Some(&self.view)
        } else {
            self.other_views.get(&proof.id)
        };
        view.is_some_and(|view| std::sync::Arc::ptr_eq(&view.editor_owner, &proof.owner))
    }
    pub(crate) fn set_editor_selections(
        &mut self,
        proof: &EditorViewProof,
        selections: Vec<Selection>,
    ) -> Result<()> {
        if !self.editor_view_current(proof) || selections.is_empty() || selections.len() > 4096 {
            bail!("Editor view retired or selection bounds exceeded");
        }
        let previous = self.active_view;
        if proof.id != previous {
            // Both views already exist; this swaps ownership without allocating
            // or creating a replacement view, then restores the active one.
            std::mem::swap(
                &mut self.view,
                self.other_views
                    .get_mut(&proof.id)
                    .expect("checked retained view"),
            );
            self.active_view = proof.id;
        }
        self.set_selections(selections);
        if proof.id != previous {
            std::mem::swap(
                &mut self.view,
                self.other_views.get_mut(&proof.id).expect("retained view"),
            );
            self.active_view = previous;
        }
        Ok(())
    }

    fn ensure_writable(&self) -> Result<()> {
        if self.is_read_only() {
            bail!("Virtual documents are read-only");
        }
        Ok(())
    }
    pub fn prepare_virtual_content(content: &str) -> Result<PreparedVirtualContent> {
        if content.len() as u64 > MAX_FILE_BYTES || content.contains('\0') {
            bail!("Virtual content exceeds 32 MiB or contains NUL");
        }
        let mut builder = ropey::RopeBuilder::new();
        let mut parts = content.split('\r');
        builder.append(parts.next().unwrap_or_default());
        for part in parts {
            builder.append("\n");
            builder.append(part.strip_prefix('\n').unwrap_or(part));
        }
        Ok(PreparedVirtualContent {
            text: builder.finish(),
        })
    }

    /// Replace provider content without recording a user edit or save receipt.
    pub fn refresh_virtual_content(&mut self, content: &str) -> Result<bool> {
        if !self.is_read_only() {
            bail!("Resource is not a virtual document");
        }
        self.refresh_virtual_prepared(Self::prepare_virtual_content(content)?)
    }

    /// Publish worker-prepared content against the current retained views.
    pub fn refresh_virtual_prepared(&mut self, prepared: PreparedVirtualContent) -> Result<bool> {
        if !self.is_read_only() {
            bail!("Resource is not a virtual document");
        }
        let next = prepared.text;
        if next == self.text {
            return Ok(false);
        }
        let next_revision = self
            .next_revision
            .checked_add(1)
            .context("Document revision exhausted")?;
        let epoch = self
            .text_epoch
            .checked_add(1)
            .context("Document text epoch exhausted")?;
        let change = ByteChange {
            range: 0..self.text.len_bytes(),
            added: next.len_bytes(),
        };
        self.text = next;
        self.revision = self.next_revision;
        self.next_revision = next_revision;
        self.saved_revision = self.revision;
        self.text_epoch = epoch;
        self.disk_content = None;
        self.eol = "\n".into();
        self.undo.clear();
        self.redo.clear();
        self.typing = None;
        self.typing_context = typing::ContextCache::default();
        self.byte_changes.clear();
        self.byte_changes.push_back((epoch, change));
        self.folding_changes.clear();
        self.folding_policy = crate::folding_controller::Lifetime::default();
        let len = self.len();
        let last_row = self.line_count() - 1;
        for view in std::iter::once(&mut self.view).chain(self.other_views.values_mut()) {
            view.cursor = view.cursor.min(len);
            view.anchor = view.anchor.map(|pos| pos.min(len));
            for selection in &mut view.secondary {
                selection.cursor = selection.cursor.min(len);
                selection.anchor = selection.anchor.map(|pos| pos.min(len));
                selection.desired_column = None;
            }
            view.top = view.top.min(last_row);
            view.desired_column = None;
            view.cursor_history.clear();
            view.snippet = None;
            view.pairs = typing::Pairs::default();
            view.folding = folding::View::default();
        }
        Ok(true)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let path = absolute_path(path)?;
        let Some(content) = read_disk(&path)? else {
            return Ok(Self {
                path: Some(path),
                ..Self::default()
            });
        };
        let mut doc = Self::from_rope(content.clone());
        doc.path = Some(path);
        doc.disk_content = Some(content);
        Ok(doc)
    }

    /// Read an existing regular file for navigation, never creating a missing buffer.
    pub fn open_existing(path: &Path) -> Result<Self> {
        let path = absolute_path(path)?;
        if !fs::metadata(&path)?.is_file() {
            bail!("Not a regular file: {}", path.display());
        }
        let content = read_disk(&path)?.context("Recent file no longer exists")?;
        let mut doc = Self::from_rope(content.clone());
        doc.path = Some(path);
        doc.disk_content = Some(content);
        Ok(doc)
    }

    /// Apply a disk reload as one undoable edit, retaining document and view identity.
    pub fn reload_content(&mut self, next: Rope) {
        if self.is_read_only() {
            return;
        }

        let prefix = self
            .text
            .chars()
            .zip(next.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = self
            .text
            .chars_at(self.text.len_chars())
            .reversed()
            .zip(next.chars_at(next.len_chars()).reversed())
            .take(self.len().min(next.len_chars()).saturating_sub(prefix))
            .take_while(|(a, b)| a == b)
            .count();
        if self.text != next {
            self.apply_changes(vec![(
                prefix..self.len() - suffix,
                next.slice(prefix..next.len_chars() - suffix).to_string(),
            )]);
        }
        self.eol = infer_eol(&next, DEFAULT_EOL).into();
        self.disk_content = Some(next);
        self.saved_revision = self.revision;
        self.break_group();
    }

    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .or(self.associated_path.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                self.virtual_uri
                    .clone()
                    .unwrap_or_else(|| "Untitled".into())
            })
    }
    pub fn dirty(&self) -> bool {
        if self.api_created_untitled {
            self.text_epoch == 0 || !self.is_empty() || self.associated_path.is_some()
        } else {
            self.revision != self.saved_revision
        }
    }
    /// Public document state differs from a clean Untitled tab after Undo:
    /// once edited, its content has still never been persisted to a resource.
    pub fn api_dirty(&self) -> bool {
        self.dirty()
            || (self.path.is_none()
                && self.virtual_uri.is_none()
                && !self.api_created_untitled
                && self.text_epoch != 0)
    }
    pub fn len(&self) -> usize {
        self.text.len_chars()
    }
    pub fn is_empty(&self) -> bool {
        self.text.len_chars() == 0
    }
    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }
    pub fn line(&self, row: usize) -> String {
        self.line_slice(row).to_string()
    }
    pub fn line_slice(&self, row: usize) -> ropey::RopeSlice<'_> {
        let line = self.text.line(row.min(self.line_count() - 1));
        if let Some(text) = line.as_str() {
            // Short lines fit in one rope leaf. Trim the borrowed string
            // without constructing a tree iterator and slicing its metadata.
            return ropey::RopeSlice::from(text.trim_end_matches(['\r', '\n']));
        }
        let trailing = line
            .chars_at(line.len_chars())
            .reversed()
            .take_while(|c| matches!(c, '\r' | '\n'))
            .count();
        line.slice(..line.len_chars() - trailing)
    }
    pub fn row(&self) -> usize {
        self.text.char_to_line(self.cursor.min(self.len()))
    }
    pub fn line_start(&self, row: usize) -> usize {
        self.text.line_to_char(row.min(self.line_count() - 1))
    }
    pub fn line_end(&self, row: usize) -> usize {
        self.line_start(row) + self.line_slice(row).len_chars()
    }
    pub fn column(&self) -> usize {
        self.cursor - self.line_start(self.row())
    }
    pub fn visual_column(&self) -> usize {
        self.display_width_slice(self.text.slice(self.line_start(self.row())..self.cursor))
    }
    pub fn display_width_slice(&self, text: ropey::RopeSlice<'_>) -> usize {
        if let Some(text) = text.as_str() {
            return self.display_width(text);
        }
        graphemes::Graphemes::new(text).fold(0, |column, g| {
            column + self.grapheme_width(&graphemes::as_text(g), column)
        })
    }
    pub fn display_width(&self, text: &str) -> usize {
        text.graphemes(true)
            .fold(0, |col, g| col + self.grapheme_width(g, col))
    }
    pub fn grapheme_width(&self, g: &str, column: usize) -> usize {
        if g == "\t" {
            let tab = self.tab_size.clamp(1, 16);
            tab - column % tab
        } else {
            grapheme_width(g, column)
        }
    }
    pub fn indentation(&self) -> String {
        let indent = self.indent_size();
        if self.insert_spaces {
            " ".repeat(indent)
        } else {
            "\t".repeat(indent / self.tab_size.clamp(1, 16))
                + &" ".repeat(indent % self.tab_size.clamp(1, 16))
        }
    }
    pub fn set_indentation(&mut self, tab_size: usize, insert_spaces: bool) {
        let tab_size = tab_size.clamp(1, 16);
        if self.tab_size != tab_size {
            self.retire_folding_policy();
            self.view.desired_column = None;
            for selection in &mut self.view.secondary {
                selection.desired_column = None;
            }
            for view in self.other_views.values_mut() {
                view.desired_column = None;
                for selection in &mut view.secondary {
                    selection.desired_column = None;
                }
            }
        }
        self.tab_size = tab_size;
        self.insert_spaces = insert_spaces;
    }
    pub fn position_at(&self, row: usize, column: usize) -> usize {
        let row = row.min(self.line_count() - 1);
        let line = self.line_slice(row);
        let mut width = 0;
        let mut chars = 0;
        for g in graphemes::Graphemes::new(line) {
            let w = self.grapheme_width(&graphemes::as_text(g), width);
            if width + w > column {
                break;
            }
            width += w;
            chars += g.len_chars();
        }
        self.line_start(row) + chars
    }
    pub fn selection(&self) -> Option<Range<usize>> {
        self.anchor
            .filter(|a| *a != self.cursor)
            .map(|a| a.min(self.cursor)..a.max(self.cursor))
    }
    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|r| self.text.slice(r).to_string())
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            history_identity: std::sync::Arc::new(()),
            workspace_group: None,
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            secondary: self.secondary.clone(),
            revision: self.revision,
            view_id: self.active_view,
            changes: Vec::new(),
            snippet: self.snippet.clone(),
            snippet_generation: self.snippet_generation,
            pairs: self.pairs.clone(),
            after_selections: None,
        }
    }
    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn changed(&mut self) {
        self.revision = self.next_revision;
        self.next_revision += 1;
        self.desired_column = None;
        self.cursor_history.clear();
    }
    pub fn break_group(&mut self) {
        self.typing = None;
    }
    pub fn insert(&mut self, text: &str, typing: bool) {
        if self.is_read_only() {
            return;
        }

        if !self.secondary.is_empty() {
            let group = typing
                && !text.contains(['\n', '\r'])
                && (self.in_snippet() || self.selections().iter().all(|s| s.range().is_empty()))
                && self.typing.is_some_and(|(time, end)| {
                    time.elapsed() < Duration::from_millis(700) && end == self.cursor
                });
            self.replace_cursors(|_, _| text.to_string());
            if group && let Some(last) = self.undo.pop() {
                if let Some(previous) = self.undo.last_mut() {
                    previous.changes.extend(last.changes);
                    previous.after_selections = None;
                } else {
                    self.undo.push(last);
                }
            }
            self.typing = typing.then(|| (Instant::now(), self.cursor));
            return;
        }
        if text.is_empty() && self.selection().is_none() {
            return;
        }
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        let can_group = typing
            && (self.selection().is_none() || self.in_snippet())
            && !text.contains(['\n', '\r'])
            && self.typing.is_some_and(|(time, end)| {
                time.elapsed() < Duration::from_millis(700) && end == self.cursor
            });
        if !can_group {
            self.checkpoint();
        } else if let Some(snapshot) = self.undo.last_mut() {
            snapshot.after_selections = None;
        }
        self.record_change(range.clone(), text.chars().count(), text.len());
        self.text.remove(range.clone());
        self.text.insert(range.start, text);
        self.cursor = range.start + text.chars().count();
        self.anchor = None;
        self.changed();
        self.typing = if typing {
            Some((Instant::now(), self.cursor))
        } else {
            None
        };
    }
    pub fn newline(&mut self) {
        if self.is_read_only() {
            return;
        }

        if !self.secondary.is_empty() {
            self.replace_cursors(|doc, selection| {
                let row = doc.text.char_to_line(selection.range().start);
                let column = selection.range().start - doc.line_start(row);
                let indent: String = doc
                    .line(row)
                    .chars()
                    .take(column)
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .collect();
                format!("{}{}", doc.eol, indent)
            });
            return;
        }
        let line = self.line(self.row());
        let indent: String = line
            .chars()
            .take(self.column())
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        self.insert(&format!("{}{}", self.eol, indent), false);
    }
    pub fn undo(&mut self) {
        if self.is_read_only() {
            return;
        }

        // Grouped entries require the App to validate every peer first.
        if self
            .workspace_history(WorkspaceHistoryDirection::Undo)
            .is_some()
        {
            return;
        }
        self.break_group();
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.reverse_snapshot(&s));
            self.restore(s);
        }
    }
    pub fn redo(&mut self) {
        if self.is_read_only() {
            return;
        }

        if self
            .workspace_history(WorkspaceHistoryDirection::Redo)
            .is_some()
        {
            return;
        }
        self.break_group();
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.reverse_snapshot(&s));
            self.restore(s);
        }
    }
    fn reverse_snapshot(&self, target: &Snapshot) -> Snapshot {
        let view = if target.view_id == self.active_view {
            &self.view
        } else {
            self.other_views.get(&target.view_id).unwrap_or(&self.view)
        };
        let after = target.after_selections.as_ref();
        let primary = after.and_then(|selections| selections.first());
        Snapshot {
            history_identity: std::sync::Arc::new(()),
            workspace_group: target.workspace_group.clone(),
            text: self.text.clone(),
            cursor: primary.map_or(view.cursor, |s| s.cursor),
            anchor: primary.map_or(view.anchor, |s| s.anchor),
            secondary: after.map_or_else(|| view.secondary.clone(), |s| s[1..].to_vec()),
            revision: self.revision,
            view_id: target.view_id,
            changes: target
                .changes
                .iter()
                .rev()
                .map(PositionChange::inverse)
                .collect(),
            snippet: view.snippet.clone(),
            snippet_generation: view.snippet_generation,
            pairs: view.pairs.clone(),
            after_selections: after.map(|_| {
                let mut selections = vec![Selection {
                    cursor: target.cursor,
                    anchor: target.anchor,
                    desired_column: None,
                }];
                selections.extend(target.secondary.clone());
                selections
            }),
        }
    }
    fn restore(&mut self, mut s: Snapshot) {
        for change in s.changes.iter().rev().map(PositionChange::inverse) {
            self.record_byte_change(change.bytes.clone());
            self.record_folding_change(change.range.clone(), change.added);
            change.map(&mut self.view);
            for view in self.other_views.values_mut() {
                change.map(view);
            }
        }
        self.text = s.text;
        let view = if s.view_id == self.active_view {
            Some(&mut self.view)
        } else {
            self.other_views.get_mut(&s.view_id)
        };
        if let Some(view) = view {
            if view.pairs.generation == s.pairs.generation {
                view.pairs = s.pairs;
            }
            if view.snippet_generation == s.snippet_generation {
                if let (Some(previous), Some(current)) = (&mut s.snippet, &view.snippet) {
                    previous.retain_removed_from(current);
                }
                view.snippet = s.snippet;
            }
            view.cursor = s.cursor;
            view.anchor = s.anchor;
            view.secondary = s.secondary;
            view.desired_column = None;
            view.cursor_history.clear();
        }
        self.revision = s.revision;
        self.observe_folding_selection();
    }
    pub fn move_to(&mut self, pos: usize, select: bool) {
        self.move_to_inner(pos, select);
        self.retire_outside_typing_pairs();
        self.observe_folding_selection();
    }
    fn move_to_inner(&mut self, pos: usize, select: bool) {
        self.resume_editor_cursor_follow();
        self.break_group();
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = pos.min(self.len());
        self.desired_column = None;
        self.cancel_invalid_snippet();
    }
    pub fn previous(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let row = self.text.char_to_line(pos);
        let start = self.line_start(row);
        if pos == start {
            return self.line_end(row - 1);
        }
        start + graphemes::previous_boundary(self.text.slice(start..pos))
    }
    pub fn next(&self, pos: usize) -> usize {
        if pos >= self.len() {
            return self.len();
        }
        let row = self.text.char_to_line(pos);
        let end = self.line_end(row);
        if pos >= end {
            return if row + 1 < self.line_count() {
                self.line_start(row + 1)
            } else {
                self.len()
            };
        }
        pos + graphemes::Graphemes::new(self.text.slice(pos..end))
            .next()
            .map_or(1, |g| g.len_chars())
    }
    pub fn word_left(&self) -> usize {
        let mut p = self.cursor;
        while p > 0 && self.text.char(p - 1).is_whitespace() {
            p = self.previous(p);
        }
        if p == 0 {
            return p;
        }
        let word = is_word(self.text.char(p - 1));
        while p > 0
            && !self.text.char(p - 1).is_whitespace()
            && is_word(self.text.char(p - 1)) == word
        {
            p = self.previous(p);
        }
        p
    }
    pub fn word_right(&self) -> usize {
        let mut p = self.cursor;
        if p < self.len() && !self.text.char(p).is_whitespace() {
            let word = is_word(self.text.char(p));
            while p < self.len()
                && !self.text.char(p).is_whitespace()
                && is_word(self.text.char(p)) == word
            {
                p = self.next(p);
            }
        }
        while p < self.len() && self.text.char(p).is_whitespace() {
            p = self.next(p);
        }
        p
    }
    pub fn horizontal(&mut self, right: bool, select: bool, word: bool) {
        self.horizontal_inner(right, select, word);
        self.retire_outside_typing_pairs();
        self.observe_folding_selection();
    }
    fn horizontal_inner(&mut self, right: bool, select: bool, word: bool) {
        let pos = if !select && !word && self.selection().is_some() {
            let r = self.selection().unwrap();
            if right { r.end } else { r.start }
        } else if word {
            if right {
                self.word_right()
            } else {
                self.word_left()
            }
        } else if right {
            self.next(self.cursor)
        } else {
            self.previous(self.cursor)
        };
        self.move_to_inner(pos, select);
    }
    pub fn vertical(&mut self, amount: isize, select: bool) {
        self.vertical_inner(amount, select);
        self.retire_outside_typing_pairs();
        self.observe_folding_selection();
    }
    fn vertical_inner(&mut self, amount: isize, select: bool) {
        let col = self.desired_column.unwrap_or_else(|| self.visual_column());
        let row = self
            .row()
            .saturating_add_signed(amount)
            .min(self.line_count() - 1);
        self.move_to_inner(self.position_at(row, col), select);
        self.desired_column = Some(col);
    }
    pub fn home(&mut self, select: bool) {
        self.home_inner(select);
        self.retire_outside_typing_pairs();
        self.observe_folding_selection();
    }
    fn home_inner(&mut self, select: bool) {
        let start = self.line_start(self.row());
        let first = start
            + self
                .line(self.row())
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
        self.move_to_inner(if self.cursor == first { start } else { first }, select);
    }
    pub fn backspace(&mut self, word: bool) {
        if self.is_read_only() {
            return;
        }

        if !self.secondary.is_empty() {
            self.delete_cursors(false, word);
            return;
        }
        let range = self.selection().unwrap_or_else(|| {
            let start = if word {
                self.word_left()
            } else {
                self.previous(self.cursor)
            };
            start..self.cursor
        });
        self.delete_range(range);
    }
    pub fn delete(&mut self, word: bool) {
        if self.is_read_only() {
            return;
        }

        if !self.secondary.is_empty() {
            self.delete_cursors(true, word);
            return;
        }
        let range = self.selection().unwrap_or_else(|| {
            let end = if word {
                self.word_right()
            } else {
                self.next(self.cursor)
            };
            self.cursor..end
        });
        self.delete_range(range);
    }
    pub fn delete_range(&mut self, range: Range<usize>) {
        if self.is_read_only() {
            return;
        }

        if range.is_empty() {
            return;
        }
        let start = range.start;
        self.apply_changes(vec![(range, String::new())]);
        self.move_to(start, false);
    }
    pub fn select_all(&mut self) {
        self.secondary.clear();
        self.move_to(self.len(), false);
        self.anchor = Some(0);
    }
    pub fn select_line(&mut self) {
        let start = self
            .selection()
            .map_or(self.line_start(self.row()), |r| r.start);
        let row = self.row();
        let end = if row + 1 < self.line_count() {
            self.line_start(row + 1)
        } else {
            self.len()
        };
        self.move_to(end, false);
        self.anchor = Some(start);
    }
    pub fn line_range(&self) -> Range<usize> {
        let row = self.row();
        self.line_start(row)..if row + 1 < self.line_count() {
            self.line_start(row + 1)
        } else {
            self.len()
        }
    }
    pub fn delete_line(&mut self) {
        if self.is_read_only() {
            return;
        }

        let rows = self.all_selected_rows();
        let columns: Vec<_> = self
            .selections()
            .iter()
            .map(|s| {
                let row = self.text.char_to_line(s.cursor);
                self.display_width(&self.text.slice(self.line_start(row)..s.cursor).to_string())
            })
            .collect();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for row in rows {
            let start = self.line_start(row);
            let end = if row + 1 < self.line_count() {
                self.line_start(row + 1)
            } else {
                self.len()
            };
            if let Some(last) = ranges.last_mut()
                && last.end == start
            {
                last.end = end;
            } else {
                ranges.push(start..end);
            }
        }
        if let Some(last) = ranges.last_mut()
            && last.end == self.len()
            && last.start > 0
        {
            last.start = self.previous(last.start);
        }
        self.apply_changes(
            ranges
                .into_iter()
                .filter(|r| !r.is_empty())
                .map(|r| (r, String::new()))
                .collect(),
        );
        let selections = self
            .selections()
            .iter()
            .zip(columns)
            .map(|(s, col)| {
                Selection::caret(self.position_at(self.text.char_to_line(s.cursor), col))
            })
            .collect();
        self.set_selections(selections);
    }
    pub fn selected_rows(&self) -> (usize, usize) {
        match self.selection() {
            Some(r) => (
                self.text.char_to_line(r.start),
                self.text.char_to_line(r.end.saturating_sub(1)),
            ),
            None => (self.row(), self.row()),
        }
    }
    pub fn all_selected_rows(&self) -> Vec<usize> {
        let mut rows = std::collections::BTreeSet::new();
        for selection in self.selections() {
            let range = selection.range();
            let first = self.text.char_to_line(range.start);
            let last = self.text.char_to_line(if range.is_empty() {
                range.end
            } else {
                range.end - 1
            });
            rows.extend(first..=last);
        }
        rows.into_iter().collect()
    }
    pub fn transform_lines(&mut self, outdent: bool, comment: Option<bool>) {
        if self.is_read_only() {
            return;
        }

        let rows = self.all_selected_rows();
        let prefix = match self
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|x| x.to_str())
        {
            Some("py" | "sh" | "bash" | "toml" | "yaml" | "yml" | "rb") => "#",
            Some("sql" | "lua") => "--",
            _ => "//",
        };
        let all_commented = rows
            .iter()
            .all(|r| self.line(*r).trim_start().starts_with(prefix));
        let remove_comment = comment.is_some_and(|force_remove| force_remove || all_commented);
        let mut changes = Vec::new();
        for row in rows {
            let line = self.line(row);
            let start = self.line_start(row);
            if comment.is_some() {
                let indent = line.chars().take_while(|c| matches!(c, ' ' | '\t')).count();
                if remove_comment {
                    let trimmed = line.trim_start_matches([' ', '\t']);
                    if let Some(rest) = trimmed.strip_prefix(prefix) {
                        let n = prefix.len() + usize::from(rest.starts_with(' '));
                        changes.push((start + indent..start + indent + n, String::new()));
                    }
                } else {
                    changes.push((start + indent..start + indent, format!("{prefix} ")));
                }
            } else if outdent {
                let n = if line.starts_with('\t') {
                    1
                } else {
                    line.chars()
                        .take(self.indent_size())
                        .take_while(|c| *c == ' ')
                        .count()
                };
                if n > 0 {
                    changes.push((start..start + n, String::new()));
                }
            } else {
                changes.push((start..start, self.indentation()));
            }
        }
        self.apply_changes(changes);
    }
    pub fn apply_changes(&mut self, mut changes: Vec<(Range<usize>, String)>) {
        if self.is_read_only() {
            return;
        }

        if changes.is_empty() {
            return;
        }
        changes.sort_by_key(|(r, _)| r.start);
        self.checkpoint();
        self.break_group();
        for (r, text) in changes.into_iter().rev() {
            let added = text.chars().count();
            self.record_change(r.clone(), added, text.len());
            self.cursor = map_position(self.cursor, &r, added);
            self.anchor = self.anchor.map(|p| map_position(p, &r, added));
            for selection in &mut self.secondary {
                selection.cursor = map_position(selection.cursor, &r, added);
                selection.anchor = selection.anchor.map(|p| map_position(p, &r, added));
                selection.desired_column = None;
            }
            self.text.remove(r.clone());
            self.text.insert(r.start, &text);
        }
        self.changed();
    }
    pub fn find(&mut self, query: &str, backwards: bool) -> bool {
        if query.is_empty() {
            return false;
        }
        let text = self.text.to_string();
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        let start = self
            .text
            .char_to_byte(if backwards { range.start } else { range.end });
        let found = if backwards {
            text[..start]
                .rfind(query)
                .or_else(|| text[start..].rfind(query).map(|p| p + start))
        } else {
            text[start..]
                .find(query)
                .map(|p| p + start)
                .or_else(|| text[..start].find(query))
        };
        if let Some(byte) = found {
            self.secondary.clear();
            let pos = self.text.byte_to_char(byte);
            self.move_to(pos + query.chars().count(), false);
            self.anchor = Some(pos);
            true
        } else {
            false
        }
    }
    pub fn replace_all(&mut self, query: &str, replacement: &str) -> usize {
        if self.is_read_only() {
            return 0;
        }

        if query.is_empty() {
            return 0;
        }
        let text = self.text.to_string();
        let changes: Vec<_> = text
            .match_indices(query)
            .map(|(b, _)| {
                (
                    self.text.byte_to_char(b)..self.text.byte_to_char(b + query.len()),
                    replacement.to_string(),
                )
            })
            .collect();
        let count = changes.len();
        self.apply_changes(changes);
        count
    }
    /// Capture an ordinary save or Save As without resolving or reading paths.
    /// The worker must validate aliases, destination baseline and file identity.
    pub fn capture_save(&self, target: PathBuf) -> Result<SaveSnapshot> {
        self.ensure_writable()?;

        validate_snapshot_path(&target, false)?;
        if let Some(path) = &self.path {
            validate_snapshot_path(path, false)?;
        }
        if self.text.len_bytes() as u64 > MAX_FILE_BYTES
            || self
                .disk_content
                .as_ref()
                .is_some_and(|text| text.len_bytes() as u64 > MAX_FILE_BYTES)
        {
            bail!("Save snapshot exceeds the 32 MiB document budget");
        }
        let next_save_generation = self
            .save_generation
            .checked_add(1)
            .context("Document save generation exhausted")?;
        Ok(SaveSnapshot {
            document_id: self.id,
            source_path: self.path.clone(),
            target,
            revision: self.revision,
            text_epoch: self.text_epoch,
            saved_revision: self.saved_revision,
            save_generation: self.save_generation,
            next_save_generation,
            text: self.text.clone(),
            baseline: self.disk_content.clone(),
        })
    }
    /// Authorize only the exact captured model state. Undo cannot restore the
    /// monotonic text epoch, even when its bytes and revision match the capture.
    pub fn check_save_snapshot(&self, snapshot: &SaveSnapshot) -> Result<()> {
        self.ensure_writable()?;

        if self.id != snapshot.document_id
            || self.path != snapshot.source_path
            || self.revision != snapshot.revision
            || self.text_epoch != snapshot.text_epoch
            || self.saved_revision != snapshot.saved_revision
            || self.save_generation != snapshot.save_generation
        {
            bail!("Document changed during save preparation; retry saving");
        }
        Ok(())
    }
    /// Publish only after the worker proves this snapshot was persisted at the
    /// returned canonical destination. No filesystem work or byte comparison
    /// occurs here. Newer edits survive and remain dirty against the saved revision.
    pub fn publish_save(
        &mut self,
        snapshot: &SaveSnapshot,
        canonical_destination: PathBuf,
    ) -> Result<()> {
        self.ensure_writable()?;

        validate_snapshot_path(&canonical_destination, true)?;
        if canonical_destination.file_name() != snapshot.target.file_name() {
            bail!("Save receipt destination does not match the captured filename");
        }
        if self.id != snapshot.document_id
            || self.path != snapshot.source_path
            || self.save_generation != snapshot.save_generation
            || self.saved_revision != snapshot.saved_revision
        {
            bail!(
                "Document path or saved baseline changed after save authorization; persisted snapshot retained on disk"
            );
        }
        if self.path.as_ref() != Some(&canonical_destination) {
            // Save As changes language semantics without changing text history.
            self.retire_typing_pairs();
            self.retire_folding_policy();
        }
        self.untitled_uri = None;
        self.associated_path = None;
        self.api_created_untitled = false;
        self.path = Some(canonical_destination);
        self.disk_content = Some(snapshot.text.clone());
        self.saved_revision = snapshot.revision;
        self.save_generation = snapshot.next_save_generation;
        self.break_group();
        Ok(())
    }
    pub fn save(&mut self) -> Result<()> {
        self.ensure_writable()?;

        let path = self
            .path
            .clone()
            .context("Choose a path with Save As first")?;
        self.save_to(&path, false)
    }
    pub fn save_to(&mut self, path: &Path, overwrite: bool) -> Result<()> {
        self.ensure_writable()?;

        self.save_to_with_before_persist(path, overwrite, |_| {})
    }
    fn save_to_with_before_persist(
        &mut self,
        path: &Path,
        overwrite: bool,
        before_persist: impl FnOnce(&Path),
    ) -> Result<()> {
        self.ensure_writable()?;

        let save_generation = self
            .save_generation
            .checked_add(1)
            .context("Document save generation exhausted")?;
        let path = absolute_path(path)?;
        let same_path = self.path.as_ref() == Some(&path);
        let create_only = !overwrite && (!same_path || self.disk_content.is_none());
        if !overwrite {
            if same_path {
                if !disk_matches(&path, self.disk_content.as_ref())? {
                    bail!(
                        "File changed on disk. Save As a different file, or reload after preserving your edits."
                    );
                }
            } else if path.try_exists()? {
                bail!("File already exists. Choose a different path.");
            }
        }
        let parent = path.parent().context("File has no parent directory")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .context("Cannot create a temporary save file beside the destination")?;
        if let Ok(metadata) = fs::metadata(&path) {
            temp.as_file().set_permissions(metadata.permissions())?;
        }
        {
            let mut writer = BufWriter::new(temp.as_file_mut());
            self.text.write_to(&mut writer)?;
            writer.flush()?;
        }
        temp.as_file().sync_all()?;
        before_persist(&path);
        if create_only {
            temp.persist_noclobber(&path)
                .map_err(|e| e.error)
                .context("Could not create destination without replacing an existing file")?;
        } else {
            temp.persist(&path)
                .map_err(|e| e.error)
                .context("Could not replace the destination file")?;
        }
        if !same_path {
            // Save As changes document semantics independently of Undo. A later
            // return to the old path cannot revive delimiter ownership.
            self.retire_typing_pairs();
            self.retire_folding_policy();
        }
        self.untitled_uri = None;
        self.associated_path = None;
        self.api_created_untitled = false;
        self.path = Some(path);
        self.disk_content = Some(self.text.clone());
        self.saved_revision = self.revision;
        self.save_generation = save_generation;
        self.break_group();
        Ok(())
    }
}

pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("Cannot resolve {}", path.display()));
    }
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = full.parent().context("Missing parent directory")?;
    let name = full.file_name().context("Missing file name")?;
    Ok(fs::canonicalize(parent)
        .context("Parent directory does not exist")?
        .join(name))
}
pub fn grapheme_width(g: &str, column: usize) -> usize {
    if g == "\t" {
        4 - column % 4
    } else if g.chars().any(char::is_control) {
        1
    } else {
        UnicodeWidthStr::width(g).max(1)
    }
}
pub fn display_width(s: &str) -> usize {
    s.graphemes(true)
        .fold(0, |col, g| col + grapheme_width(g, col))
}
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn map_position(p: usize, range: &Range<usize>, added: usize) -> usize {
    if p < range.start {
        p
    } else if p >= range.end {
        p - (range.end - range.start) + added
    } else {
        range.start + added
    }
}

#[cfg(test)]
mod save_snapshot_tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Document, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(directory.path())
            .unwrap()
            .join("source.cpp");
        fs::write(&path, "猫🙂 base\r\n").unwrap();
        let document = Document::open(&path).unwrap();
        (directory, document, path)
    }
    fn view_selections(view: &ViewState) -> Vec<Selection> {
        let mut selections = vec![Selection {
            cursor: view.cursor,
            anchor: view.anchor,
            desired_column: view.desired_column,
        }];
        selections.extend(view.secondary.iter().cloned());
        selections
    }
    fn persist_snapshot(snapshot: &SaveSnapshot) {
        // Simulate the worker's successful receipt with actual persisted bytes.
        // Filesystem/identity/race qualification belongs to the real worker tests.
        let mut file = fs::File::create(snapshot.target()).unwrap();
        snapshot.text().write_to(&mut file).unwrap();
        file.sync_all().unwrap();
    }

    #[test]
    fn save_capture_is_io_free_and_edit_undo_cannot_revive_authorization() {
        let (directory, mut document, path) = fixture();
        let absent = directory.path().join("uncreated/target.cpp");
        let pure = document.capture_save(absent.clone()).unwrap();
        assert_eq!(pure.target(), absent);
        assert!(!absent.parent().unwrap().exists());
        assert_eq!(pure.text(), &document.text);
        assert_eq!(pure.baseline(), document.disk_content.as_ref());
        let snapshot = document.capture_save(path.clone()).unwrap();
        document.check_save_snapshot(&snapshot).unwrap();
        let selections = document.selections();
        document.insert("λ", false);
        document.undo();
        assert_eq!(document.revision, snapshot.revision());
        assert_eq!(document.text, *snapshot.text());
        assert_eq!(document.selections(), selections);
        assert!(!document.dirty());
        assert!(document.text_epoch() > snapshot.text_epoch());
        assert!(
            document
                .check_save_snapshot(&snapshot)
                .unwrap_err()
                .to_string()
                .contains("changed during save preparation")
        );
        assert_eq!(document.save_generation(), 0);
        document.redo();
        assert_eq!(document.text, "λ猫🙂 base\r\n");
        assert_eq!(fs::read(path).unwrap(), "猫🙂 base\r\n".as_bytes());
    }

    #[test]
    fn receipt_publishes_captured_baseline_and_keeps_newer_edits_and_history() {
        let (_directory, mut document, path) = fixture();
        document.insert("captured ", false);
        let snapshot = document.capture_save(path.clone()).unwrap();
        document.check_save_snapshot(&snapshot).unwrap();
        persist_snapshot(&snapshot);
        document.insert("newer ", false);
        let newer = document.text.clone();
        let selections = document.selections();
        let epoch = document.text_epoch();
        let history = (
            document.undo.len(),
            document.redo.len(),
            document.next_revision,
        );
        document.publish_save(&snapshot, path.clone()).unwrap();
        assert_eq!(document.text, newer);
        assert_eq!(document.selections(), selections);
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(
            (
                document.undo.len(),
                document.redo.len(),
                document.next_revision
            ),
            history
        );
        assert_eq!(document.disk_content.as_ref(), Some(snapshot.text()));
        assert_eq!(document.saved_revision, snapshot.revision());
        assert_eq!(document.save_generation(), 1);
        assert!(document.dirty());
        document.undo();
        assert_eq!(document.text, *snapshot.text());
        assert!(!document.dirty());
        document.undo();
        assert_eq!(document.text, "猫🙂 base\r\n");
        assert!(document.dirty());
        document.redo();
        assert_eq!(document.text, *snapshot.text());
        assert!(!document.dirty());
        document.redo();
        assert_eq!(document.text, newer);
        assert!(document.dirty());
        assert_eq!(
            fs::read(path).unwrap(),
            snapshot.text().to_string().as_bytes()
        );
    }

    #[test]
    fn save_as_receipt_preserves_shared_reversed_selections_and_existing_redo() {
        let (directory, mut document, source) = fixture();
        let destination = fs::canonicalize(directory.path()).unwrap().join("new.json");
        let id = document.id;
        document.activate_view(11);
        document.insert("captured ", false);
        document.set_selections(vec![
            Selection {
                cursor: 1,
                anchor: Some(5),
                desired_column: Some(7),
            },
            Selection::caret(8),
        ]);
        let snapshot = document.capture_save(destination.clone()).unwrap();
        document.check_save_snapshot(&snapshot).unwrap();
        persist_snapshot(&snapshot);
        document.activate_view(22);
        document.set_selections(vec![Selection::caret(document.len())]);
        document.insert("later λ\r\n", false);
        let newer = document.text.clone();
        document.insert("redo 猫", false);
        let redo = document.text.clone();
        document.undo();
        assert_eq!(document.text, newer);
        let first_view = view_selections(document.view_state(Some(11)));
        let second_view = view_selections(document.view_state(Some(22)));
        let epoch = document.text_epoch();
        let history = (document.undo.len(), document.redo.len());
        document
            .publish_save(&snapshot, destination.clone())
            .unwrap();
        assert_eq!(document.id, id);
        assert_eq!(document.path.as_ref(), Some(&destination));
        assert_eq!(document.text, newer);
        assert_eq!(document.text_epoch(), epoch);
        assert_eq!(view_selections(document.view_state(Some(11))), first_view);
        assert_eq!(view_selections(document.view_state(Some(22))), second_view);
        assert_eq!((document.undo.len(), document.redo.len()), history);
        assert!(document.dirty());
        document.redo();
        assert_eq!(document.text, redo);
        assert_eq!(document.path.as_ref(), Some(&destination));
        document.undo();
        document.undo();
        assert_eq!(document.text, *snapshot.text());
        assert!(!document.dirty());
        assert_eq!(document.id, id);
        assert_eq!(document.path.as_ref(), Some(&destination));
        assert_eq!(fs::read(source).unwrap(), "猫🙂 base\r\n".as_bytes());
        assert_eq!(
            fs::read(destination).unwrap(),
            snapshot.text().to_string().as_bytes()
        );
    }

    #[test]
    fn moved_path_reloaded_baseline_and_duplicate_receipts_never_mutate_metadata() {
        for change in [0, 1, 2] {
            let (directory, mut document, path) = fixture();
            document.insert("captured ", false);
            let snapshot = document.capture_save(path.clone()).unwrap();
            persist_snapshot(&snapshot);
            match change {
                0 => document.path = Some(directory.path().join("moved.cpp")),
                1 => document.reload_content(Rope::from_str("reloaded λ\r\n")),
                _ => document.publish_save(&snapshot, path.clone()).unwrap(),
            }
            let text = document.text.clone();
            let baseline = document.disk_content.clone();
            let selections = document.selections();
            let state = (
                document.path.clone(),
                document.revision,
                document.saved_revision,
                document.text_epoch(),
                document.save_generation(),
                document.undo.len(),
                document.redo.len(),
            );
            assert!(
                document
                    .publish_save(&snapshot, path.clone())
                    .unwrap_err()
                    .to_string()
                    .contains("path or saved baseline changed")
            );
            assert_eq!(document.text, text);
            assert_eq!(document.disk_content, baseline);
            assert_eq!(document.selections(), selections);
            assert_eq!(
                (
                    document.path.clone(),
                    document.revision,
                    document.saved_revision,
                    document.text_epoch(),
                    document.save_generation(),
                    document.undo.len(),
                    document.redo.len()
                ),
                state
            );
        }
    }

    #[test]
    fn foreign_or_invalid_receipt_and_capture_limits_leave_state_untouched() {
        let (_directory, mut document, path) = fixture();
        let snapshot = document.capture_save(path.clone()).unwrap();
        let mut other = Document::from_text("猫🙂 base\r\n");
        other.path = Some(path.clone());
        assert!(other.check_save_snapshot(&snapshot).is_err());
        assert!(other.publish_save(&snapshot, path.clone()).is_err());
        let before = (
            document.path.clone(),
            document.text_epoch(),
            document.saved_revision,
            document.save_generation(),
        );
        assert!(
            document
                .publish_save(&snapshot, PathBuf::from("source.cpp"))
                .is_err()
        );
        assert!(
            document
                .publish_save(&snapshot, path.with_file_name("other.cpp"))
                .is_err()
        );
        assert!(
            document
                .capture_save(PathBuf::from("x".repeat(4097)))
                .is_err()
        );
        assert!(document.capture_save(PathBuf::from("bad\0.cpp")).is_err());
        assert_eq!(
            (
                document.path.clone(),
                document.text_epoch(),
                document.saved_revision,
                document.save_generation()
            ),
            before
        );
        document.save_generation = u64::MAX;
        assert!(
            document
                .capture_save(path.clone())
                .unwrap_err()
                .to_string()
                .contains("generation exhausted")
        );
        document.save_generation = 0;
        document.text = Rope::from_str(&"x".repeat(MAX_FILE_BYTES as usize + 1));
        assert!(document.capture_save(path.clone()).is_err());
        document.text = Rope::from_str("small");
        document.disk_content = Some(Rope::from_str(&"x".repeat(MAX_FILE_BYTES as usize + 1)));
        assert!(document.capture_save(path).is_err());
        let debug = format!("{snapshot:?}");
        assert!(debug.contains("text_bytes"));
        assert!(
            !debug.contains("猫🙂 base"),
            "Debug must not expose saved text"
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn untitled_api_dirty_survives_undo_while_tab_cleanliness_and_history_remain_exact() {
        let mut doc = super::Document::from_text("");
        let id = doc.id;
        assert!(!doc.dirty());
        assert!(!doc.api_dirty());
        doc.insert("猫🙂\r\n", false);
        assert!(doc.dirty());
        assert!(doc.api_dirty());
        doc.undo();
        assert!(doc.is_empty());
        assert!(!doc.dirty());
        assert!(doc.api_dirty());
        assert_eq!(doc.id, id);
        doc.redo();
        assert_eq!(doc.text.to_string(), "猫🙂\r\n");
        assert!(doc.dirty());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("saved.txt");
        doc.save_to(&path, false).unwrap();
        assert!(!doc.api_dirty());
        assert!(!doc.dirty());
        assert_eq!(std::fs::read(&path).unwrap(), "猫🙂\r\n".as_bytes());
        doc.undo();
        assert!(doc.api_dirty());
        assert!(doc.dirty());
        doc.redo();
        assert!(!doc.api_dirty());
        assert_eq!(doc.id, id);
        assert_eq!(std::fs::read(&path).unwrap(), "猫🙂\r\n".as_bytes());
    }

    use super::*;
    #[test]
    fn foreign_creation_after_save_check_preserves_save_as_and_newborn_model_history() {
        for save_as in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.cpp");
            let destination = root.path().join("destination.cpp");
            let original = "猫🙂 base\r\n";
            fs::write(&source, original).unwrap();
            let mut doc = Document::open(if save_as { &source } else { &destination }).unwrap();
            let initial = doc.text.clone();
            doc.insert("first ", false);
            let once = doc.text.clone();
            doc.insert("second ", false);
            let twice = doc.text.clone();
            doc.undo();
            assert_eq!(doc.text, once);
            doc.set_selections(vec![
                Selection {
                    cursor: 1,
                    anchor: Some(0),
                    desired_column: None,
                },
                Selection::caret(2),
            ]);
            let before = (
                doc.id,
                doc.path.clone(),
                doc.revision,
                doc.saved_revision,
                doc.text_epoch(),
                doc.save_generation(),
                doc.next_revision,
                doc.undo.len(),
                doc.redo.len(),
                doc.dirty(),
            );
            let selections = doc.selections();
            let disk_content = doc.disk_content.clone();
            let eol = doc.eol.clone();
            let foreign = "foreign猫🙂\r\n";
            let mut created = false;
            let error = doc
                .save_to_with_before_persist(&destination, false, |target| {
                    assert_eq!(
                        target,
                        fs::canonicalize(root.path())
                            .unwrap()
                            .join("destination.cpp")
                    );
                    assert!(!target.exists());
                    fs::write(target, foreign).unwrap();
                    created = true;
                })
                .unwrap_err();
            assert!(
                created,
                "conflict must occur after the original preflight check"
            );
            assert!(error.to_string().contains("without replacing"));
            assert_eq!(fs::read(&destination).unwrap(), foreign.as_bytes());
            assert_eq!(fs::read(&source).unwrap(), original.as_bytes());
            assert_eq!(doc.text, once);
            assert_eq!(doc.disk_content, disk_content);
            assert_eq!(doc.eol, eol);
            assert_eq!(doc.selections(), selections);
            assert_eq!(
                (
                    doc.id,
                    doc.path.clone(),
                    doc.revision,
                    doc.saved_revision,
                    doc.text_epoch(),
                    doc.save_generation(),
                    doc.next_revision,
                    doc.undo.len(),
                    doc.redo.len(),
                    doc.dirty()
                ),
                before
            );
            // Preserve both branches, including the redo entry existing at
            // the time persistence failed, without creating a save history edit.
            doc.redo();
            assert_eq!(doc.text, twice);
            doc.undo();
            assert_eq!(doc.text, once);
            doc.undo();
            assert_eq!(doc.text, initial);
            doc.redo();
            assert_eq!(doc.text, once);
            assert_eq!(doc.save_generation(), before.5);
            assert_eq!(fs::read(&destination).unwrap(), foreign.as_bytes());
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
    }
    #[test]
    fn explicit_overwrite_allows_foreign_creation_and_normal_save_replaces_its_baseline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("saved.cpp");
        let mut doc = Document::from_text("猫🙂\r\n");
        doc.insert("first ", false);
        let text = doc.text.clone();
        doc.save_to_with_before_persist(&path, true, |target| {
            fs::write(target, "foreign\r\n").unwrap();
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text.to_string());
        assert_eq!(doc.disk_content.as_ref(), Some(&text));
        assert_eq!(doc.save_generation(), 1);
        assert!(!doc.dirty());
        doc.insert("second ", false);
        let text = doc.text.clone();
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text.to_string());
        assert_eq!(doc.disk_content.as_ref(), Some(&text));
        assert_eq!(doc.save_generation(), 2);
        assert!(!doc.dirty());
        doc.undo();
        assert!(doc.dirty());
        doc.redo();
        assert!(!doc.dirty());
    }
    #[test]
    fn successful_save_generation_is_independent_of_undo_and_failed_persistence() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("save.txt");
        fs::write(&path, "original\r\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.save_generation(), 0);
        doc.insert("edit ", false);
        doc.undo();
        assert!(!doc.dirty());
        assert_eq!(doc.save_generation(), 0);
        doc.save().unwrap();
        assert_eq!(doc.save_generation(), 1);
        assert_eq!(fs::read(&path).unwrap(), b"original\r\n");
        doc.save().unwrap();
        assert_eq!(doc.save_generation(), 2);
        doc.insert("unsaved ", false);
        let before = doc.text.to_string();
        fs::write(&path, "external").unwrap();
        assert!(doc.save().is_err());
        assert_eq!(doc.save_generation(), 2);
        assert_eq!(doc.text.to_string(), before);
        assert!(doc.dirty());
        assert_eq!(fs::read(&path).unwrap(), b"external");
        let alternate = root.path().join("alternate.txt");
        doc.save_to(&alternate, false).unwrap();
        assert_eq!(doc.save_generation(), 3);
        assert_eq!(fs::read_to_string(alternate).unwrap(), before);
        doc.undo();
        doc.redo();
        assert_eq!(doc.save_generation(), 3);
    }
    struct ShortReads<'a> {
        bytes: &'a [u8],
        interrupted: bool,
    }
    impl Read for ShortReads<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.interrupted = !self.interrupted;
            if self.interrupted {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let size = buffer.len().min(7).min(self.bytes.len());
            buffer[..size].copy_from_slice(&self.bytes[..size]);
            self.bytes = &self.bytes[size..];
            Ok(size)
        }
    }

    #[test]
    fn platform_eol_default_applies_only_without_existing_line_breaks() {
        for fallback in ["\n", "\r\n"] {
            assert_eq!(infer_eol(&Rope::new(), fallback), fallback);
            assert_eq!(infer_eol(&Rope::from_str("猫🙂"), fallback), fallback);
            assert_eq!(infer_eol(&Rope::from_str("猫\n🙂"), fallback), "\n");
            assert_eq!(infer_eol(&Rope::from_str("猫\r\n🙂"), fallback), "\r\n");
        }
        for original in ["", "猫🙂", "猫\n🙂", "猫\r\n🙂"] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("eol.txt");
            fs::write(&path, original).unwrap();
            let mut doc = Document::open(&path).unwrap();
            doc.move_to(doc.len(), false);
            let expected = format!("{original}{}", doc.eol);
            doc.newline();
            doc.save().unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
            doc.undo();
            assert_eq!(doc.text.to_string(), original);
            doc.redo();
            assert_eq!(doc.text.to_string(), expected);
        }
        assert_eq!(Document::default().eol, DEFAULT_EOL);
    }

    #[test]
    fn streamed_text_preserves_split_utf8_and_rejects_invalid_or_over_budget_input() {
        let text = "ab🙂c\r\n漢字\n".repeat(5000);
        let reader = || ShortReads {
            bytes: text.as_bytes(),
            interrupted: false,
        };
        let rope = read_text(reader(), text.len() as u64).unwrap();
        assert_eq!(rope, text);
        assert_eq!(infer_eol(&rope, "\n"), "\r\n");
        assert!(reader_matches(reader(), &rope).unwrap());
        assert!(read_text(reader(), text.len() as u64 - 1).is_err());
        assert!(read_text(&b"binary\0text"[..], 100).is_err());
        assert!(read_text(&b"invalid\xff"[..], 100).is_err());
        assert!(read_text(&b"truncated\xf0\x9f"[..], 100).is_err());
        assert_eq!(read_text(&b""[..], 0).unwrap().len_bytes(), 0);
        let error = std::io::Error::other("read failed");
        struct FailedRead(Option<std::io::Error>);
        impl Read for FailedRead {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(self.0.take().unwrap())
            }
        }
        assert!(
            read_text(FailedRead(Some(error)), 100)
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
    }

    #[test]
    fn streamed_comparison_detects_truncation_append_and_late_changes() {
        let text = "line🙂\r\n".repeat(20000);
        let rope = Rope::from_str(&text);
        assert!(reader_matches(text.as_bytes(), &rope).unwrap());
        assert!(!reader_matches(&text.as_bytes()[..text.len() - 1], &rope).unwrap());
        let mut changed = text.clone();
        changed.push('x');
        assert!(!reader_matches(changed.as_bytes(), &rope).unwrap());
        changed = text.clone();
        changed.replace_range(text.len() - 2.., "xx");
        assert!(!reader_matches(changed.as_bytes(), &rope).unwrap());
        assert!(!reader_matches(&b"x"[..], &Rope::new()).unwrap());
        assert!(!reader_matches(&b""[..], &rope).unwrap());
    }

    #[test]
    fn saved_rope_baselines_survive_edits_undo_streamed_save_and_external_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        let original = "first🙂\r\nsecond漢字\r\n".repeat(50000);
        fs::write(&path, &original).unwrap();
        let mut doc = Document::open(&path).unwrap();
        let baseline = doc.disk_content.clone().unwrap();
        doc.insert("edited\r\n", false);
        assert_eq!(doc.disk_content.as_ref().unwrap(), &baseline);
        doc.save().unwrap();
        let saved = format!("edited\r\n{original}");
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        assert_eq!(doc.disk_content.as_ref().unwrap(), &doc.text);
        assert_eq!(baseline, original);
        doc.undo();
        assert_eq!(doc.text, original);
        assert!(doc.dirty());
        let external = format!("X{}", &saved[1..]);
        fs::write(&path, &external).unwrap();
        assert!(doc.save().is_err());
        assert!(doc.dirty());
        assert_eq!(doc.text, original);
        assert_eq!(doc.disk_content.as_ref().unwrap(), &Rope::from_str(&saved));
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
    }

    #[test]
    fn long_line_navigation_matches_contiguous_layout_and_preserves_saved_text() {
        let line = "a\te\u{301} 猫 👩\u{200d}💻 🇬🇧🇺🇸 ".repeat(400);
        let original = format!("{line}\r\nshort\r\n");
        let mut doc = Document::from_text(&original);
        assert_eq!(doc.line(0), line);
        assert_eq!(doc.line_end(0), line.chars().count());
        for column in [0, 1, 2, 7, 79, 501, 1003, 7000, 100000] {
            let mut width = 0;
            let mut chars = 0;
            for g in line.graphemes(true) {
                let next = width + doc.grapheme_width(g, width);
                if next > column {
                    break;
                }
                width = next;
                chars += g.chars().count();
            }
            assert_eq!(doc.position_at(0, column), chars);
            doc.cursor = chars;
            assert_eq!(doc.visual_column(), width);
            let prefix = doc.text.slice(..chars).to_string();
            let suffix = doc.text.slice(chars..doc.line_end(0)).to_string();
            if chars > 0 {
                assert_eq!(
                    doc.previous(chars),
                    chars - prefix.graphemes(true).next_back().unwrap().chars().count()
                );
            }
            if !suffix.is_empty() {
                assert_eq!(
                    doc.next(chars),
                    chars + suffix.graphemes(true).next().unwrap().chars().count()
                );
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.txt");
        doc.cursor = doc.position_at(0, 1003);
        doc.insert("INSERTED🙂", false);
        doc.undo();
        doc.save_to(&path, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    }

    #[test]
    fn reload_preserves_tail_selections_in_all_views_through_undo_redo_and_save() {
        let original = "head OLD\r\nmiddle 猫🙂 tail\r\nlast";
        let next = "head replacement🙂\r\nmiddle 猫🙂 tail\r\nlast";
        let original_tail = original[..original.find("middle").unwrap()].chars().count();
        let next_tail = next[..next.find("middle").unwrap()].chars().count();
        let mut doc = Document::from_text(original);
        let id = doc.id;
        for (view, offset) in [(1, 2), (2, 7)] {
            doc.activate_view(view);
            doc.set_selections(vec![
                Selection {
                    cursor: original_tail + offset + 2,
                    anchor: Some(original_tail + offset),
                    desired_column: None,
                },
                Selection::caret(original.chars().count() - 1),
            ]);
        }
        let verify = |doc: &mut Document, text: &str, tail: usize| {
            assert_eq!(doc.id, id);
            assert_eq!(doc.text.to_string(), text);
            for (view, offset) in [(1, 2), (2, 7)] {
                doc.activate_view(view);
                assert_eq!(doc.cursor, tail + offset + 2, "view {view}");
                assert_eq!(doc.anchor, Some(tail + offset));
                assert_eq!(
                    doc.secondary,
                    vec![Selection::caret(text.chars().count() - 1)]
                );
            }
        };
        doc.activate_view(1);
        doc.reload_content(Rope::from_str(next));
        assert!(!doc.dirty());
        verify(&mut doc, next, next_tail);
        // Undo from the other view must restore both independent selections.
        doc.undo();
        assert!(doc.dirty());
        verify(&mut doc, original, original_tail);
        doc.redo();
        assert!(!doc.dirty());
        verify(&mut doc, next, next_tail);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reloaded.txt");
        doc.save_to(&path, false).unwrap();
        assert_eq!(fs::read(path).unwrap(), next.as_bytes());
    }

    #[test]
    fn reload_handles_insertions_deletions_and_overlapping_equal_ends() {
        for (original, next, cursor, expected) in [
            ("abc tail", "aXYZbc tail", 6, 9),
            ("aXYZbc tail", "abc tail", 9, 6),
            ("aaaa", "aaa", 2, 2),
            ("aaa", "aaaa", 2, 2),
            ("same🙂", "same🙂", 4, 4),
            ("", "猫", 0, 1),
            ("猫", "", 1, 0),
        ] {
            let mut doc = Document::from_text(original);
            doc.cursor = cursor;
            doc.reload_content(Rope::from_str(next));
            assert_eq!(doc.text.to_string(), next);
            assert_eq!(doc.cursor, expected, "{original:?} -> {next:?}");
        }
    }

    #[test]
    fn graphemes_crlf_and_undo() {
        let mut d = Document::from_text("a\u{301}🙂\r\nnext");
        d.horizontal(true, false, false);
        assert_eq!(d.cursor, 2);
        d.horizontal(true, false, false);
        assert_eq!(d.cursor, 3);
        d.horizontal(true, false, false);
        assert_eq!(d.cursor, 5);
        d.backspace(false);
        assert_eq!(d.text.to_string(), "a\u{301}🙂next");
        d.undo();
        assert_eq!(d.text.to_string(), "a\u{301}🙂\r\nnext");
        assert_eq!(d.cursor, 5);
    }
    #[test]
    fn typing_group_selection_and_redo() {
        let mut d = Document::default();
        for c in ["h", "i", "🙂"] {
            d.insert(c, true);
        }
        d.undo();
        assert!(d.is_empty());
        d.redo();
        assert_eq!(d.text.to_string(), "hi🙂");
        d.select_all();
        d.insert("new", false);
        d.undo();
        assert_eq!(d.selected_text().unwrap(), "hi🙂");
    }
    #[test]
    fn save_detects_external_change_and_preserves_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, "a\r\n").unwrap();
        let mut d = Document::open(&p).unwrap();
        d.insert("b", false);
        d.save().unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "ba\r\n");
        d.undo();
        assert!(d.dirty());
        d.redo();
        assert!(!d.dirty());
        fs::write(&p, "external").unwrap();
        d.insert("x", false);
        assert!(d.save().is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "external");
    }
    #[test]
    fn indentation_and_replacement_are_single_transactions() {
        let mut d = Document::from_text("one\ntwo\n");
        d.select_all();
        d.transform_lines(false, None);
        assert_eq!(d.text.to_string(), "    one\n    two\n");
        d.undo();
        assert_eq!(d.text.to_string(), "one\ntwo\n");
        assert_eq!(d.replace_all("o", "🙂"), 2);
        d.undo();
        assert_eq!(d.text.to_string(), "one\ntwo\n");
    }
    #[test]
    fn vertical_movement_preserves_display_column() {
        let mut d = Document::from_text("\t猫a\nx\n\t猫b");
        d.move_to(3, false);
        assert_eq!(d.visual_column(), 7);
        d.vertical(1, false);
        assert_eq!(d.column(), 1);
        d.vertical(1, false);
        assert_eq!(d.column(), 3);
    }
    #[test]
    fn randomized_edit_undo_roundtrip() {
        let mut d = Document::from_text("seed\n🙂");
        let original = d.text.to_string();
        let mut seed = 42u64;
        for _ in 0..300 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            d.move_to((seed as usize) % (d.len() + 1), false);
            d.insert(if seed & 1 == 0 { "é" } else { "x\n" }, false);
        }
        for _ in 0..300 {
            d.undo();
        }
        assert_eq!(d.text.to_string(), original);
        for _ in 0..300 {
            d.redo();
        }
        assert!(d.len() > 300);
    }

    #[test]
    fn undo_deletion_restores_the_original_cursor_and_selection() {
        let mut d = Document::from_text("hello\nworld");
        d.move_to(3, false);
        d.backspace(false);
        d.undo();
        assert_eq!(d.cursor, 3);
        assert_eq!(d.anchor, None);
        d.delete_line();
        d.undo();
        assert_eq!(d.cursor, 3);
        assert_eq!(d.anchor, None);
        assert_eq!(d.text.to_string(), "hello\nworld");
    }

    #[test]
    fn randomized_changes_match_a_reference_string() {
        let mut d = Document::default();
        let mut reference = String::new();
        let mut seed = 913u64;
        for _ in 0..500 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let count = reference.chars().count();
            let pos = (seed as usize) % (count + 1);
            let byte = reference
                .char_indices()
                .nth(pos)
                .map_or(reference.len(), |(b, _)| b);
            d.move_to(pos, false);
            if seed.is_multiple_of(3) && pos < count {
                let end = byte + reference[byte..].chars().next().unwrap().len_utf8();
                reference.replace_range(byte..end, "");
                d.delete(false);
            } else {
                let text = if seed & 1 == 0 { "猫" } else { "x\n" };
                reference.insert_str(byte, text);
                d.insert(text, false);
            }
            assert_eq!(d.text.to_string(), reference);
        }
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_symlinks_preserves_the_link_and_file_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "hello").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        let mut d = Document::open(&link).unwrap();
        d.insert("new ", false);
        d.save().unwrap();
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "new hello");
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}

#[cfg(test)]
mod virtual_document_tests {
    use super::*;
    use crate::editing_profile::TypingOptions;

    fn state(doc: &Document) -> (String, Vec<Selection>, u64, u64, u64, u64, usize, usize) {
        (
            doc.text.to_string(),
            doc.selections(),
            doc.revision,
            doc.saved_revision,
            doc.text_epoch(),
            doc.save_generation(),
            doc.undo.len(),
            doc.redo.len(),
        )
    }

    #[test]
    fn virtual_edits_and_save_refuse_before_source_or_selection_changes() {
        let mut doc = Document::from_virtual("test:α".into(), "α😀\r\nβ\rγ").unwrap();
        assert_eq!(doc.text.to_string(), "α😀\nβ\nγ");
        assert_eq!(doc.virtual_uri(), Some("test:α"));
        assert!(doc.is_read_only());
        assert!(!doc.dirty());
        assert!(!doc.api_dirty());
        assert_eq!(doc.eol, "\n");
        assert_eq!(doc.name(), "test:α");
        doc.set_language_override(Some("cpp".into())).unwrap();
        assert_eq!(doc.language_override(), Some("cpp"));
        doc.set_selections(vec![
            Selection {
                cursor: 1,
                anchor: Some(4),
                desired_column: None,
            },
            Selection::caret(5),
        ]);
        let original = state(&doc);
        doc.insert("bad", false);
        doc.newline();
        doc.backspace(false);
        doc.delete(false);
        doc.delete_range(0..2);
        doc.delete_line();
        doc.transform_lines(false, Some(true));
        doc.apply_changes(vec![(0..1, "bad".into())]);
        assert_eq!(doc.replace_all("α", "bad"), 0);
        doc.replace_cursors(|_, _| panic!("Readonly expansion must not execute"));
        doc.delete_cursors(true, false);
        doc.insert_line(true);
        doc.copy_lines(false);
        doc.move_lines(false);
        doc.reload_content(Rope::from_str("disk"));
        doc.undo();
        doc.redo();
        let options = TypingOptions::default();
        assert!(doc.type_character('(', options, true).is_err());
        assert!(doc.newline_with_options(options).is_err());
        assert!(doc.line_break_with_options(options).is_err());
        assert!(doc.backspace_with_options(options, false).is_err());
        assert!(doc.comment_lines(CommentOperation::Toggle).is_err());
        assert!(doc.toggle_block_comment().is_err());
        let template = crate::snippet::Template::parse("${1:bad}$0").unwrap();
        assert!(doc.insert_snippet(&template, &Default::default()).is_err());
        assert!(
            doc.insert_snippet_command(&template, &Default::default())
                .is_err()
        );
        assert!(
            doc.insert_snippet_command_resolved(&template, |_, _, _, _, _| {
                panic!("Readonly resolver must not execute")
            })
            .is_err()
        );
        assert!(doc.step_snippet(false).is_err());
        assert!(
            doc.apply_completion_edits(0..1, vec![], |_, _| {
                panic!("Readonly completion must not execute")
            })
            .is_err()
        );
        assert!(doc.capture_workspace_source().is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("untouched.txt");
        fs::write(&path, "sentinel").unwrap();
        assert!(doc.save().unwrap_err().to_string().contains("read-only"));
        assert!(doc.save_to(&path, true).is_err());
        assert!(doc.capture_save(path.clone()).is_err());
        // Even a receipt whose public source identity has been forged cannot
        // turn a provider model into a file-backed model.
        let mut ordinary = Document::from_text("ordinary");
        ordinary.id = doc.id;
        let snapshot = ordinary.capture_save(path.clone()).unwrap();
        assert!(doc.check_save_snapshot(&snapshot).is_err());
        assert!(
            doc.publish_save(&snapshot, fs::canonicalize(&path).unwrap())
                .is_err()
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "sentinel");
        assert_eq!(state(&doc), original);
        assert!(doc.path.is_none());
        assert!(doc.disk_content.is_none());
        // Navigation remains available.
        doc.move_to(0, false);
        assert_eq!(doc.cursor, 0);
    }

    #[test]
    fn provider_refresh_retains_shared_identity_and_clamps_all_views_without_undo() {
        let mut doc = Document::from_virtual("test:shared".into(), "α😀\r\nlong line").unwrap();
        doc.activate_view(11);
        doc.set_selections(vec![Selection {
            cursor: doc.len(),
            anchor: Some(3),
            desired_column: Some(20),
        }]);
        doc.activate_view(22);
        doc.set_selections(vec![Selection::caret(8), Selection::caret(5)]);
        doc.top = 99;
        let id = doc.id;
        let old_epoch = doc.text_epoch();
        let generation = doc.save_generation();
        let prepared = Document::prepare_virtual_content("猫\r\n").unwrap();
        // Publication uses the live views, not those at worker submission.
        doc.move_to(9, false);
        assert!(doc.refresh_virtual_prepared(prepared).unwrap());
        assert_eq!(doc.text.to_string(), "猫\n");
        assert_eq!(doc.id, id);
        assert_eq!(doc.active_view, 22);
        assert!(doc.text_epoch() > old_epoch);
        assert_eq!(doc.save_generation(), generation);
        assert_eq!(doc.revision, doc.saved_revision);
        assert!(!doc.api_dirty());
        assert_eq!(doc.view_state(Some(11)).cursor, 2);
        assert_eq!(doc.view_state(Some(11)).anchor, Some(2));
        assert!(
            doc.selections()
                .iter()
                .all(|selection| selection.cursor <= 2)
        );
        assert!(doc.top < doc.line_count());
        assert!(doc.undo.is_empty());
        assert!(doc.redo.is_empty());
        let refreshed = state(&doc);
        assert!(!doc.refresh_virtual_content("猫\n").unwrap());
        doc.undo();
        doc.redo();
        assert_eq!(state(&doc), refreshed);
        assert!(doc.refresh_virtual_content("bad\0").is_err());
        assert_eq!(state(&doc), refreshed);
    }

    #[test]
    fn virtual_refresh_counter_refusal_and_bad_metadata_are_atomic() {
        for counter in [false, true] {
            let mut doc = Document::from_virtual("test:bounded".into(), "old😀").unwrap();
            doc.move_to(3, true);
            if counter {
                doc.text_epoch = u64::MAX;
            } else {
                doc.next_revision = u64::MAX;
            }
            let original = state(&doc);
            assert!(doc.refresh_virtual_content("new").is_err());
            assert_eq!(state(&doc), original);
            assert_eq!(doc.virtual_uri(), Some("test:bounded"));
        }
        for uri in [
            "file:///tmp/test",
            "untitled:test",
            "1bad:value",
            "missing",
            "test:\0",
        ] {
            assert!(Document::from_virtual(uri.into(), "content").is_err());
        }
        let mut doc = Document::from_virtual("test:language".into(), "content").unwrap();
        doc.set_language_override(Some("rust".into())).unwrap();
        assert!(doc.set_language_override(Some("".into())).is_err());
        assert_eq!(doc.language_override(), Some("rust"));
        let mut ordinary = Document::from_text("source");
        assert!(ordinary.refresh_virtual_content("provider").is_err());
        assert_eq!(ordinary.text.to_string(), "source");
    }
}

#[cfg(test)]
mod api_untitled_tests {
    use super::*;

    #[test]
    fn api_untitled_initial_content_is_dirty_without_synthetic_undo_and_saves_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("initial.cpp");
        let initial = "α😀\r\nβ\r\n";
        let mut doc = Document::from_untitled(
            Some("untitled:/initial.cpp".into()),
            initial,
            Some("cpp".into()),
            Some(target.clone()),
        )
        .unwrap();
        let id = doc.id;
        assert_eq!(doc.name(), "initial.cpp");
        assert!(doc.path.is_none());
        assert_eq!(doc.associated_path(), Some(target.as_path()));
        assert_eq!(doc.untitled_uri(), Some("untitled:/initial.cpp"));
        assert_eq!(doc.text.to_string(), initial);
        assert_eq!(doc.eol, "\r\n");
        assert!(doc.dirty() && doc.api_dirty());
        assert!(!doc.is_read_only());
        assert!(doc.undo.is_empty() && doc.redo.is_empty());
        let initial_epoch = doc.text_epoch();
        doc.undo();
        assert_eq!(doc.text_epoch(), initial_epoch);
        assert_eq!(doc.text.to_string(), initial);
        doc.insert("new 猫", false);
        let edited = doc.text.to_string();
        doc.undo();
        assert_eq!(doc.text.to_string(), initial);
        assert!(doc.dirty());
        doc.redo();
        assert_eq!(doc.text.to_string(), edited);
        let snapshot = doc.capture_save(target.clone()).unwrap();
        // A different model's otherwise plausible receipt cannot erase identity.
        let other = Document::from_untitled(None, "other", None, None).unwrap();
        let wrong = other.capture_save(target.clone()).unwrap();
        assert!(doc.publish_save(&wrong, target.clone()).is_err());
        assert_eq!(doc.untitled_uri(), Some("untitled:/initial.cpp"));
        doc.check_save_snapshot(&snapshot).unwrap();
        fs::write(&target, snapshot.text().to_string()).unwrap();
        doc.publish_save(&snapshot, fs::canonicalize(&target).unwrap())
            .unwrap();
        assert_eq!(doc.id, id);
        assert!(!doc.dirty() && !doc.api_dirty());
        assert!(doc.untitled_uri().is_none() && doc.associated_path().is_none());
        assert_eq!(doc.language_override(), Some("cpp"));
        assert_eq!(doc.save_generation(), 1);
        assert_eq!(fs::read_to_string(target).unwrap(), edited);
        doc.undo();
        assert_eq!(doc.text.to_string(), initial);
        assert!(doc.dirty());
        assert!(doc.untitled_uri().is_none());
    }

    #[test]
    fn api_untitled_empty_and_mixed_eol_admission_preserves_source_proofs() {
        let mut empty = Document::from_untitled(None, "", None, None).unwrap();
        assert!(empty.dirty() && empty.api_dirty());
        assert!(
            empty
                .untitled_uri()
                .unwrap()
                .starts_with("untitled:Untitled-")
        );
        empty.undo();
        assert!(empty.is_empty() && empty.dirty());
        empty.insert("", false);
        assert!(empty.dirty());
        empty.insert("x", false);
        assert!(empty.dirty());
        empty.undo();
        assert!(empty.is_empty() && !empty.dirty() && !empty.api_dirty());
        empty.redo();
        assert!(empty.dirty() && empty.api_dirty());
        let mut associated =
            Document::from_untitled(None, "", None, Some(PathBuf::from("target.txt"))).unwrap();
        associated.insert("x", false);
        associated.undo();
        assert!(associated.is_empty() && associated.dirty() && associated.api_dirty());
        for (input, expected, eol) in [
            ("α\r\nβ\rγ\n", "α\r\nβ\r\nγ\r\n", "\r\n"),
            ("α\r\nβ\n", "α\nβ\n", "\n"),
            ("α\rβ\nγ\n", "α\nβ\nγ\n", "\n"),
        ] {
            let mut doc = Document::from_untitled(None, input, None, None).unwrap();
            assert_eq!(doc.text.to_string(), expected);
            assert_eq!(doc.eol, eol);
            let source = doc.capture_workspace_source().unwrap();
            let snapshot = doc.capture_save(PathBuf::from("destination.txt")).unwrap();
            doc.insert("z", false);
            doc.undo();
            assert_eq!(doc.text.to_string(), expected);
            assert!(!source.current(&doc));
            assert!(doc.check_save_snapshot(&snapshot).is_err());
            assert!(doc.dirty());
        }
        for uri in ["file:///tmp/a", "test:a", "untitled:\0"] {
            assert!(Document::from_untitled(Some(uri.into()), "", None, None).is_err());
        }
        assert!(Document::from_untitled(None, "bad\0", None, None).is_err());
        assert!(Document::from_untitled(None, "", Some("".into()), None).is_err());
    }
}

#[cfg(test)]
mod editor_state_tests {
    use super::*;
    #[test]
    fn retained_inactive_selection_update_preserves_active_sibling_and_redo() {
        let mut doc = Document::from_text("α😀\r\n猫\r\n");
        doc.activate_view(1);
        doc.insert("!", false);
        doc.undo();
        doc.set_selections(vec![Selection {
            cursor: 1,
            anchor: Some(2),
            desired_column: None,
        }]);
        let original = doc.editor_view_identity();
        doc.activate_view(2);
        doc.move_to(5, false);
        let sibling = doc.selections();
        let before = (
            doc.id,
            doc.text.to_string(),
            doc.revision,
            doc.text_epoch(),
            doc.saved_revision,
            doc.save_generation(),
            doc.dirty(),
        );
        doc.set_editor_selections(&original, vec![Selection::caret(6)])
            .unwrap();
        assert_eq!(doc.active_view, 2);
        assert_eq!(doc.selections(), sibling);
        assert_eq!(doc.view_state(Some(1)).cursor, 6);
        assert_eq!(
            (
                doc.id,
                doc.text.to_string(),
                doc.revision,
                doc.text_epoch(),
                doc.saved_revision,
                doc.save_generation(),
                doc.dirty()
            ),
            before
        );
        doc.redo();
        assert_eq!(doc.text.to_string(), "!α😀\r\n猫\r\n");
        doc.undo();
        assert_eq!(doc.text.to_string(), before.1);
    }
    #[test]
    fn exact_view_accessors_never_fabricate_missing_authority_or_change_active_view() {
        let mut doc = Document::from_text("α😀\r\nsecond");
        doc.activate_view(1);
        doc.set_selections(vec![Selection::caret(1), Selection::caret(5)]);
        let original = doc.editor_view_identity();
        let stamp = doc.editor_view_stamp();
        doc.activate_view(2);
        doc.move_to(2, false);
        let sibling = doc.selections();
        assert_eq!(doc.editor_view_stamp_for(1), Some(stamp));
        assert_eq!(doc.editor_selection_count_for(1), Some(2));
        assert_eq!(doc.editor_view_identity_for(1).unwrap().view_id(), 1);
        assert!(doc.editor_view_current(&original));
        assert!(doc.editor_view_identity_for(99).is_none());
        assert!(doc.editor_view_stamp_for(99).is_none());
        assert!(doc.editor_selection_count_for(99).is_none());
        assert_eq!(doc.active_view, 2);
        assert_eq!(doc.selections(), sibling);
        doc.remove_view(1);
        assert!(doc.editor_view_stamp_for(1).is_none());
        doc.display_view(1);
        assert_ne!(doc.editor_view_stamp(), stamp);
        assert!(!doc.editor_view_current(&original));
        assert_eq!(doc.text.to_string(), "α😀\r\nsecond");
        assert!(!doc.dirty());
    }
    #[test]
    fn recreated_view_cannot_reuse_selection_authority() {
        let mut doc = Document::from_text("abcd");
        doc.activate_view(5);
        let proof = doc.editor_view_identity();
        doc.activate_view(6);
        doc.remove_view(5);
        doc.display_view(5);
        let current = doc.selections();
        assert!(!doc.editor_view_current(&proof));
        assert!(
            doc.set_editor_selections(&proof, vec![Selection::caret(4)])
                .is_err()
        );
        assert_eq!(doc.selections(), current);
        let new = doc.editor_view_identity();
        doc.set_editor_selections(&new, vec![Selection::caret(4)])
            .unwrap();
        assert_eq!(doc.cursor, 4);
    }
    #[test]
    fn language_clock_aba_noop_and_exhaustion_preserve_text_save_and_redo() {
        let mut doc = Document::from_virtual("sample:/readonly".into(), "猫😀\r\n").unwrap();
        let initial = (
            doc.id,
            doc.text.to_string(),
            doc.revision,
            doc.text_epoch(),
            doc.saved_revision,
            doc.save_generation(),
            doc.dirty(),
        );
        doc.set_language_override(Some("cpp".into())).unwrap();
        let cpp = doc.language_epoch();
        doc.set_language_override(Some("cpp".into())).unwrap();
        assert_eq!(doc.language_epoch(), cpp);
        doc.set_language_override(Some("python".into())).unwrap();
        doc.set_language_override(Some("cpp".into())).unwrap();
        assert!(doc.language_epoch() > cpp);
        assert_eq!(
            (
                doc.id,
                doc.text.to_string(),
                doc.revision,
                doc.text_epoch(),
                doc.saved_revision,
                doc.save_generation(),
                doc.dirty()
            ),
            initial
        );
        doc.language_epoch = u64::MAX;
        assert!(doc.set_language_override(Some("python".into())).is_err());
        assert_eq!(doc.language_override(), Some("cpp"));
        assert!(doc.undo.is_empty() && doc.redo.is_empty());
        let proof = doc.editor_view_identity();
        doc.set_editor_selections(&proof, vec![Selection::caret(1)])
            .unwrap();
        assert_eq!(doc.cursor, 1);
        assert_eq!(doc.text.to_string(), initial.1);
        assert!(doc.is_read_only());
        let mut editable = Document::from_text("α😀\r\n");
        editable.insert("changed", false);
        let edited = editable.text.to_string();
        editable.undo();
        let source = editable.text.to_string();
        let epoch = editable.text_epoch();
        editable.set_language_override(Some("cpp".into())).unwrap();
        editable
            .set_language_override(Some("python".into()))
            .unwrap();
        editable.set_language_override(Some("cpp".into())).unwrap();
        assert_eq!(editable.text_epoch(), epoch);
        editable.redo();
        assert_eq!(editable.text.to_string(), edited);
        editable.undo();
        assert_eq!(editable.text.to_string(), source);
    }
}
