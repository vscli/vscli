//! Bounded native typing decisions and one transaction per compound gesture.
use super::*;
use crate::editing_profile::{
    AutoClosing, ContextProof, LexicalContext, PairHandling, ProfileId, Surround, TypingOptions,
    before, quote, word,
};
use std::sync::Arc;

const SCAN_BYTES: usize = 64 * 1024;
const CHECKPOINTS: usize = 512;
const STRIDE: usize = 64 * 1024;
const CURSORS: usize = 10_000;
const PAIRS: usize = 4096;
const EDIT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
struct Mark {
    open: usize,
    close: usize,
    left: char,
    right: char,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Pairs {
    // Distinct closing positions, sorted ascending. Mapping surviving marks
    // through a text edit preserves this order; only new marks require sorting.
    marks: Arc<Vec<Mark>>,
    pub(super) generation: u64,
    options: Option<TypingOptions>,
}
impl Pairs {
    pub(super) fn map(&mut self, range: &Range<usize>, added: usize) {
        if self.marks.is_empty() {
            return;
        }
        let marks = Arc::make_mut(&mut self.marks);
        marks.retain_mut(|mark| {
            if !range.is_empty() && (range.contains(&mark.open) || range.contains(&mark.close)) {
                return false;
            }
            mark.open = map_position(mark.open, range, added);
            mark.close = map_position(mark.close, range, added);
            mark.open < mark.close
        });
    }
    fn retire(&mut self) {
        self.marks = Arc::default();
        self.generation = self.generation.wrapping_add(1);
        self.options = None;
    }
}

#[derive(Clone, Copy, Debug)]
enum State {
    Code(bool), // previous character can start a C++ R" raw literal
    Slash,
    Line(bool, bool), // continued line comment / CR of a continued CRLF
    Block(bool),
    String(char, bool, bool), // delimiter, escape, escaped CR
    RawStart([u8; 16], u8),
    Raw([u8; 16], u8, u8), // delimiter / matched closing prefix
    Unknown,
}
impl State {
    fn context(self) -> LexicalContext {
        match self {
            Self::Code(_) | Self::Slash => LexicalContext::Code,
            Self::Line(..) | Self::Block(_) => LexicalContext::Comment,
            Self::String(..) | Self::RawStart(..) | Self::Raw(..) => LexicalContext::String,
            Self::Unknown => LexicalContext::Unknown,
        }
    }
    fn next(self, ch: char, profile: ProfileId) -> Self {
        match self {
            Self::Code(raw) => match ch {
                '/' => Self::Slash,
                '"' if raw && profile == ProfileId::Cpp => Self::RawStart([0; 16], 0),
                '\'' | '"' if profile == ProfileId::Cpp || ch == '"' => {
                    Self::String(ch, false, false)
                }
                // These are recognized by the JSON configuration as pairs but
                // their tokenization in malformed JSON is deliberately unknown.
                '\'' | '`' => Self::Unknown,
                _ => Self::Code(ch == 'R'),
            },
            Self::Slash => match ch {
                '/' => Self::Line(false, false),
                '*' => Self::Block(false),
                _ => Self::Code(false).next(ch, profile),
            },
            Self::Line(continued, cr) => match ch {
                '\n' if continued || cr => Self::Line(false, false),
                '\r' if continued => Self::Line(false, true),
                '\n' | '\r' => Self::Code(false),
                _ => Self::Line(ch == '\\', false),
            },
            Self::Block(star) => {
                if star && ch == '/' {
                    Self::Code(false)
                } else {
                    Self::Block(ch == '*')
                }
            }
            Self::String(delimiter, escaped, cr) => {
                if cr && ch == '\n' {
                    Self::String(delimiter, false, false)
                } else if escaped {
                    Self::String(delimiter, false, ch == '\r')
                } else if ch == delimiter {
                    Self::Code(false)
                } else if matches!(ch, '\n' | '\r') {
                    Self::Unknown
                } else {
                    Self::String(delimiter, ch == '\\', false)
                }
            }
            Self::RawStart(mut delimiter, len) => {
                if ch == '(' {
                    Self::Raw(delimiter, len, 0)
                } else if len < 16
                    && ch.is_ascii()
                    && !ch.is_ascii_whitespace()
                    && !matches!(ch, ')' | '\\')
                {
                    delimiter[usize::from(len)] = ch as u8;
                    Self::RawStart(delimiter, len + 1)
                } else {
                    Self::Unknown
                }
            }
            Self::Raw(delimiter, len, matched) => {
                let expected = if matched == 0 {
                    ')'
                } else if matched <= len {
                    delimiter[usize::from(matched - 1)] as char
                } else {
                    '"'
                };
                if ch == expected {
                    if matched == len + 1 {
                        Self::Code(false)
                    } else {
                        Self::Raw(delimiter, len, matched + 1)
                    }
                } else {
                    Self::Raw(delimiter, len, u8::from(ch == ')'))
                }
            }
            Self::Unknown => Self::Unknown,
        }
    }
}
#[derive(Clone, Copy)]
struct Checkpoint {
    byte: usize,
    state: State,
}
#[derive(Default)]
pub(super) struct ContextCache {
    profile: Option<ProfileId>,
    epoch: u64,
    checkpoints: Vec<Checkpoint>,
}
impl ContextCache {
    fn reset(&mut self, profile: ProfileId, epoch: u64) {
        self.profile = Some(profile);
        self.epoch = epoch;
        self.checkpoints.clear();
        self.checkpoints.push(Checkpoint {
            byte: 0,
            state: State::Code(false),
        });
    }
    fn checkpoint(&mut self, checkpoint: Checkpoint) {
        match self
            .checkpoints
            .binary_search_by_key(&checkpoint.byte, |point| point.byte)
        {
            Ok(index) => self.checkpoints[index] = checkpoint,
            Err(index) => {
                if self.checkpoints.len() == CHECKPOINTS {
                    // Retain the initial guaranteed state and the latest useful
                    // region. Every retained point remains an exact prefix proof.
                    self.checkpoints.remove(1);
                    let index = self
                        .checkpoints
                        .partition_point(|p| p.byte < checkpoint.byte);
                    self.checkpoints.insert(index, checkpoint);
                } else {
                    self.checkpoints.insert(index, checkpoint);
                }
            }
        }
    }
}

struct Plan {
    changes: Vec<(Range<usize>, String)>,
    selections: Vec<Selection>,
    marks: Vec<Mark>,
}
impl Document {
    /// Current editing policy is deliberately outside Undo snapshots. A real
    /// source/configuration transition retires ownership in every shared view.
    pub fn set_language_configuration(
        &mut self,
        configuration: Option<Arc<crate::language_configuration::Configuration>>,
    ) -> Result<()> {
        if let Some(configuration) = configuration.as_ref() {
            validate_configuration(configuration)?;
        }
        if self.language_configuration.as_deref() == configuration.as_deref() {
            return Ok(());
        }
        self.retire_typing_pairs();
        self.retire_folding_policy();
        self.typing_context = ContextCache::default();
        self.language_configuration = configuration;
        Ok(())
    }
    pub fn language_configuration(&self) -> Option<&crate::language_configuration::Configuration> {
        self.language_configuration.as_deref()
    }
    fn typing_pair(&self, profile: ProfileId, open: char) -> Option<crate::editing_profile::Pair> {
        if let Some(pairs) = self
            .language_configuration
            .as_ref()
            .and_then(|c| c.auto_closing_pairs.as_ref())
        {
            return pairs.iter().find(|pair| pair.open == open).map(|pair| {
                crate::editing_profile::Pair {
                    open: pair.open,
                    close: pair.close,
                    not_string: pair.not_string,
                    not_comment: pair.not_comment,
                }
            });
        }
        // C++ and JSON inherit explicit bundled auto-closing tables. Languages
        // without such a table derive unguarded auto pairs from their brackets.
        if matches!(profile, ProfileId::Plaintext | ProfileId::Unsupported)
            && let Some(brackets) = self
                .language_configuration
                .as_ref()
                .and_then(|c| c.brackets.as_ref())
        {
            return brackets.iter().find(|pair| pair.open == open).map(|pair| {
                crate::editing_profile::Pair {
                    open: pair.open,
                    close: pair.close,
                    not_string: false,
                    not_comment: false,
                }
            });
        }
        profile.pair(open)
    }
    fn typing_surround(&self, profile: ProfileId, open: char) -> Option<char> {
        if let Some(configuration) = self.language_configuration.as_ref() {
            if let Some(pairs) = configuration.surrounding_pairs.as_ref() {
                return pairs
                    .iter()
                    .find(|pair| pair.open == open)
                    .map(|pair| pair.close);
            }
            // C++ has an explicit bundled surrounding table, which remains a
            // lower field contribution when an installed override omits it.
            if profile == ProfileId::Cpp {
                return profile.surround(open);
            }
            if let Some(pairs) = configuration.auto_closing_pairs.as_ref() {
                return pairs
                    .iter()
                    .find(|pair| pair.open == open)
                    .map(|pair| pair.close);
            }
            if matches!(profile, ProfileId::Plaintext | ProfileId::Unsupported)
                && let Some(pairs) = configuration.brackets.as_ref()
            {
                return pairs
                    .iter()
                    .find(|pair| pair.open == open)
                    .map(|pair| pair.close);
            }
        }
        profile.surround(open)
    }
    fn typing_closing(&self, profile: ProfileId, close: char) -> bool {
        if let Some(pairs) = self
            .language_configuration
            .as_ref()
            .and_then(|c| c.auto_closing_pairs.as_ref())
        {
            return pairs.iter().any(|pair| pair.close == close);
        }
        if matches!(profile, ProfileId::Plaintext | ProfileId::Unsupported)
            && let Some(pairs) = self
                .language_configuration
                .as_ref()
                .and_then(|c| c.brackets.as_ref())
        {
            return pairs.iter().any(|pair| pair.close == close);
        }
        profile.closing(close)
    }
    fn typing_before(&self, ch: char, policy: AutoClosing, for_quotes: bool) -> bool {
        // A physical line ending is not a following content character. The
        // declarative set may exclude whitespace without disabling EOL pairs.
        if matches!(ch, '\r' | '\n') && policy != AutoClosing::Never {
            return true;
        }
        if policy == AutoClosing::LanguageDefined
            && let Some(before) = self
                .language_configuration
                .as_ref()
                .and_then(|c| c.auto_close_before.as_ref())
        {
            return before.contains(ch);
        }
        if policy == AutoClosing::LanguageDefined && !for_quotes && quote(ch) {
            return true;
        }
        before(ch, policy)
    }
    pub(super) fn typing_indent_pair(
        &self,
        profile: ProfileId,
        open: char,
        close: Option<char>,
    ) -> bool {
        self.language_configuration
            .as_ref()
            .and_then(|c| c.brackets.as_ref())
            .map_or_else(
                || profile.indent_pair(open, close),
                |pairs| {
                    pairs.iter().any(|pair| {
                        pair.open == open && close.is_none_or(|close| pair.close == close)
                    })
                },
            )
    }
    // Prefix checkpoints certify exact current-Rope state, independently of
    // asynchronous syntax highlighting. Both planners share this invalidation.
    fn take_typing_cache(&mut self, profile: ProfileId) -> ContextCache {
        let mut cache = std::mem::take(&mut self.typing_context);
        if cache.profile != Some(profile) {
            cache.reset(profile, self.text_epoch);
        } else if cache.epoch != self.text_epoch {
            if let Some(changes) = self.byte_changes_since(cache.epoch) {
                if let Some(first) = changes.map(|change| change.range.start).min() {
                    cache.checkpoints.retain(|point| point.byte <= first);
                }
                cache.epoch = self.text_epoch;
            } else {
                cache.reset(profile, self.text_epoch);
            }
        }
        cache
    }
    pub(super) fn scan_typing_line(
        &mut self,
        row: usize,
        profile: ProfileId,
        budget: &mut usize,
    ) -> Option<super::indentation::Line> {
        if !matches!(profile, ProfileId::Cpp | ProfileId::Json) || row >= self.line_count() {
            return None;
        }
        let mut cache = self.take_typing_cache(profile);
        let start = self.line_start(row);
        let target = self.text.char_to_byte(start);
        let point = cache.checkpoints[cache.checkpoints.partition_point(|p| p.byte <= target) - 1];
        let mut state = point.state;
        let mut byte = point.byte;
        let mut position = self.text.byte_to_char(byte);
        let mut chars = Vec::new();
        let mut code = Vec::new();
        let end = self.line_end(row);
        for ch in self.text.chars_at(position) {
            if position == end || ch.len_utf8() > *budget {
                break;
            }
            *budget -= ch.len_utf8();
            if position >= start {
                chars.push(ch);
                code.push(state.context() == LexicalContext::Code);
            }
            state = state.next(ch, profile);
            byte += ch.len_utf8();
            position += 1;
            if position == start {
                cache.checkpoint(Checkpoint { byte, state });
            }
        }
        cache.checkpoint(Checkpoint { byte, state });
        self.typing_context = cache;
        (position == end && state.context() != LexicalContext::Unknown).then_some(
            super::indentation::Line {
                chars,
                code,
                end_context: state.context(),
            },
        )
    }
    pub fn typing_contexts(
        &mut self,
        positions: &[usize],
        profile: ProfileId,
    ) -> Vec<ContextProof> {
        let mut cache = self.take_typing_cache(profile);
        let mut order = positions
            .iter()
            .enumerate()
            .take(CURSORS)
            .collect::<Vec<_>>();
        order.sort_by_key(|(_, position)| **position);
        let mut proofs = positions
            .iter()
            .take(CURSORS)
            .map(|position| ContextProof {
                document: self.id,
                text_epoch: self.text_epoch,
                profile,
                byte: self.text.char_to_byte((*position).min(self.len())),
                context: LexicalContext::Unknown,
            })
            .collect::<Vec<_>>();
        let mut scanned = 0;
        for (index, position) in order {
            if *position > self.len() || !matches!(profile, ProfileId::Cpp | ProfileId::Json) {
                continue;
            }
            let target = self.text.char_to_byte(*position);
            let checkpoint =
                cache.checkpoints[cache.checkpoints.partition_point(|p| p.byte <= target) - 1];
            let mut state = checkpoint.state;
            let mut byte = checkpoint.byte;
            let mut next_checkpoint = byte.saturating_add(STRIDE);
            for ch in self.text.chars_at(self.text.byte_to_char(byte)) {
                if byte == target || scanned + ch.len_utf8() > SCAN_BYTES {
                    break;
                }
                state = state.next(ch, profile);
                byte += ch.len_utf8();
                scanned += ch.len_utf8();
                if byte >= next_checkpoint {
                    cache.checkpoint(Checkpoint { byte, state });
                    next_checkpoint = byte.saturating_add(STRIDE);
                }
            }
            cache.checkpoint(Checkpoint { byte, state });
            if byte == target {
                proofs[index].context = if matches!(state, State::Slash)
                    && *position < self.len()
                    && matches!(self.text.char(*position), '/' | '*')
                {
                    // At a token's opening boundary the next character belongs
                    // to the current document, not to cached prefix state.
                    LexicalContext::Comment
                } else {
                    state.context()
                };
            }
        }
        self.typing_context = cache;
        proofs
    }
    pub fn retire_typing_pairs(&mut self) {
        self.break_group();
        self.view.pairs.retire();
        for view in self.other_views.values_mut() {
            view.pairs.retire();
        }
    }
    pub(super) fn retire_outside_typing_pairs(&mut self) {
        if self.view.pairs.marks.is_empty() {
            return;
        }
        let mut carets = std::iter::once(self.cursor)
            .chain(self.secondary.iter().map(|selection| selection.cursor))
            .collect::<Vec<_>>();
        carets.sort_unstable();
        let occupied = |mark: &Mark| {
            let after_open = carets.partition_point(|cursor| *cursor <= mark.open);
            carets
                .get(after_open)
                .is_some_and(|cursor| *cursor <= mark.close)
        };
        if !self.view.pairs.marks.iter().all(occupied) {
            Arc::make_mut(&mut self.view.pairs.marks).retain(occupied);
            // Explicit cursor departure must not resurrect retired ownership
            // through an older Undo snapshot.
            self.view.pairs.generation = self.view.pairs.generation.wrapping_add(1);
        }
    }
    pub(super) fn smart_selections(&self) -> Result<Vec<Selection>> {
        // The public secondary list may be malformed; enforce the bound before
        // cloning it, rather than allocating an arbitrarily large staging list.
        if self.secondary.len() >= CURSORS {
            bail!("Typing supports at most 10,000 cursors");
        }
        let selections = self.selections();
        let mut ranges = Vec::with_capacity(selections.len());
        for selection in &selections {
            let range = selection.range();
            if range.end > self.len() {
                bail!("Invalid typing selection");
            }
            ranges.push(range);
        }
        ranges.sort_by_key(|range| (range.start, range.end));
        if ranges.windows(2).any(|pair| {
            pair[0].end > pair[1].start
                || (pair[0].end == pair[1].start && (pair[0].is_empty() || pair[1].is_empty()))
        }) {
            bail!("Typing selections overlap; normalize selections first");
        }
        Ok(selections)
    }
    fn prepare_typing_options(&mut self, options: TypingOptions) {
        let mut changed = false;
        for view in std::iter::once(&mut self.view).chain(self.other_views.values_mut()) {
            if view
                .pairs
                .options
                .is_some_and(|previous| previous != options)
            {
                view.pairs.retire();
                changed = true;
            }
            view.pairs.options = Some(options);
        }
        if changed {
            self.break_group();
        }
    }
    fn live_mark(&self, cursor: usize, close: char, options: TypingOptions) -> Option<&Mark> {
        if self.pairs.options != Some(options) {
            return None;
        }
        let index = self
            .pairs
            .marks
            .binary_search_by_key(&cursor, |mark| mark.close)
            .ok()?;
        let mark = &self.pairs.marks[index];
        (mark.right == close
            && mark.close < self.len()
            && self.text.char(mark.open) == mark.left
            && self.text.char(mark.close) == mark.right)
            .then_some(mark)
    }
    pub fn type_character(
        &mut self,
        ch: char,
        options: TypingOptions,
        grouped: bool,
    ) -> Result<()> {
        if ch == '\0' {
            bail!("NUL-containing text is unsupported");
        }
        if ch == '\n' {
            return self.newline_with_options(options);
        }
        let selections = self.smart_selections()?;
        if self.in_snippet() {
            self.validate_typing_size(
                &selections
                    .iter()
                    .map(|selection| (selection.range(), ch.to_string()))
                    .collect::<Vec<_>>(),
            )?;
            self.prepare_typing_options(options);
            self.insert(&ch.to_string(), grouped);
            return Ok(());
        }
        if options.overtype != PairHandling::Never
            && selections.iter().all(|selection| {
                let cursor = selection.cursor;
                selection.range().is_empty()
                    && cursor < self.len()
                    && self.text.char(cursor) == ch
                    && (self.live_mark(cursor, ch, options).is_some()
                        || (options.overtype == PairHandling::Always
                            && self.typing_closing(options.profile, ch)))
                    && !(quote(ch) && cursor > 0 && self.text.char(cursor - 1) == '\\')
            })
        {
            self.prepare_typing_options(options);
            let mut positions = selections
                .iter()
                .map(|selection| selection.cursor)
                .collect::<Vec<_>>();
            positions.sort_unstable();
            Arc::make_mut(&mut self.view.pairs.marks)
                .retain(|mark| positions.binary_search(&mark.close).is_err());
            self.assign_selections(
                selections
                    .iter()
                    .map(|selection| Selection::caret(selection.cursor + 1))
                    .collect(),
            );
            self.break_group();
            return Ok(());
        }
        if let Some(close) = self.typing_surround(options.profile, ch)
            && options.surround != Surround::Never
            && (options.surround != Surround::Quotes || quote(ch))
            && (options.surround != Surround::Brackets || !quote(ch))
        {
            let mut budget = SCAN_BYTES;
            let surround = selections.iter().all(|selection| {
                let range = selection.range();
                if range.is_empty()
                    || (quote(ch) && range.len() == 1 && quote(self.text.char(range.start)))
                {
                    return false;
                }
                for selected in self.text.slice(range).chars() {
                    if selected.len_utf8() > budget {
                        return false;
                    }
                    budget -= selected.len_utf8();
                    if !matches!(selected, ' ' | '\t' | '\r' | '\n') {
                        return true;
                    }
                }
                false
            });
            if surround {
                return self.surround_typing(selections, ch, close, options);
            }
        }
        if let Some(pair) = self.typing_pair(options.profile, ch) {
            let policy = if quote(ch) {
                options.quotes
            } else {
                options.brackets
            };
            if policy != AutoClosing::Never
                && selections.iter().all(|selection| {
                    let cursor = selection.cursor;
                    selection.range().is_empty()
                        && (cursor == self.len()
                            || self.typing_before(self.text.char(cursor), policy, quote(ch))
                            || (!quote(self.text.char(cursor))
                                && self.typing_closing(options.profile, self.text.char(cursor))))
                        && (!quote(ch)
                            || policy == AutoClosing::Always
                            || cursor == 0
                            || !word(self.text.char(cursor - 1)))
                        && (!quote(ch) || cursor == 0 || self.text.char(cursor - 1) != '\\')
                })
            {
                let contextual = pair.not_string || pair.not_comment;
                let allowed = !contextual
                    || self
                        .typing_contexts(
                            &selections
                                .iter()
                                .map(|selection| selection.cursor)
                                .collect::<Vec<_>>(),
                            options.profile,
                        )
                        .iter()
                        .all(|proof| match proof.context {
                            LexicalContext::Code => true,
                            LexicalContext::String => !pair.not_string,
                            LexicalContext::Comment => !pair.not_comment,
                            LexicalContext::Unknown => false,
                        });
                let retained = if self.pairs.options == Some(options) {
                    self.pairs.marks.len()
                } else {
                    0
                };
                if allowed && retained + selections.len() <= PAIRS {
                    let mut plan = Plan {
                        changes: Vec::new(),
                        selections: Vec::new(),
                        marks: Vec::new(),
                    };
                    let shifts = shifts(&selections, |_| 2);
                    for (index, selection) in selections.iter().enumerate() {
                        let start = selection.cursor.saturating_add_signed(shifts[index]);
                        plan.changes.push((
                            selection.cursor..selection.cursor,
                            format!("{}{}", pair.open, pair.close),
                        ));
                        plan.selections.push(Selection::caret(start + 1));
                        plan.marks.push(Mark {
                            open: start,
                            close: start + 1,
                            left: pair.open,
                            right: pair.close,
                        });
                    }
                    self.commit_typing(plan, options)?;
                    self.typing = grouped.then(|| (Instant::now(), self.cursor));
                    return Ok(());
                }
            }
        }
        if let Some(edits) =
            super::indentation::Session::new().closing(self, &selections, ch, options)
        {
            let group = grouped
                && self.view.pairs.options == Some(options)
                && self.typing.is_some_and(|(time, end)| {
                    time.elapsed() < Duration::from_millis(700) && end == self.cursor
                });
            self.commit_indentation(edits, options)?;
            if group && let Some(last) = self.undo.pop() {
                if let Some(previous) = self.undo.last_mut() {
                    previous.changes.extend(last.changes);
                    previous.after_selections = None;
                } else {
                    self.undo.push(last);
                }
            }
            self.break_group();
            return Ok(());
        }
        let inserted = ch.to_string();
        self.validate_typing_size(
            &selections
                .iter()
                .map(|selection| (selection.range(), inserted.clone()))
                .collect::<Vec<_>>(),
        )?;
        self.prepare_typing_options(options);
        self.insert(&inserted, grouped);
        Ok(())
    }
    fn surround_typing(
        &mut self,
        selections: Vec<Selection>,
        open: char,
        close: char,
        options: TypingOptions,
    ) -> Result<()> {
        let mut boundaries = std::collections::BTreeMap::<usize, (String, String)>::new();
        for selection in &selections {
            let range = selection.range();
            boundaries.entry(range.start).or_default().1.push(open);
            boundaries.entry(range.end).or_default().0.push(close);
        }
        let changes = boundaries
            .into_iter()
            .map(|(position, (close, open))| (position..position, close + &open))
            .collect::<Vec<_>>();
        let shifts = shifts(&selections, |index| selections[index].range().len() + 2);
        let final_selections = selections
            .iter()
            .enumerate()
            .map(|(index, selection)| {
                let range = selection.range();
                Selection {
                    anchor: Some(range.start.saturating_add_signed(shifts[index]) + 1),
                    cursor: range.end.saturating_add_signed(shifts[index]) + 1,
                    desired_column: None,
                }
            })
            .collect();
        self.commit_typing(
            Plan {
                changes,
                selections: final_selections,
                marks: Vec::new(),
            },
            options,
        )
    }
    pub(super) fn validate_typing_size(&self, changes: &[(Range<usize>, String)]) -> Result<()> {
        let mut bytes = 0usize;
        let mut resulting = self.text.len_bytes();
        let mut previous = None;
        let mut ordered = changes.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|(range, _)| range.start);
        for (range, text) in ordered {
            if range.start > range.end
                || range.end > self.len()
                || previous.is_some_and(|end| range.start < end)
            {
                bail!("Invalid or overlapping typing edits");
            }
            previous = Some(range.end);
            bytes = bytes.saturating_add(text.len());
            resulting = resulting
                .checked_sub(self.text.slice(range.clone()).len_bytes())
                .and_then(|size| size.checked_add(text.len()))
                .context("Typing size overflow")?;
        }
        if bytes > EDIT_BYTES || resulting > MAX_FILE_BYTES as usize {
            bail!("Typing exceeds 4 MiB replacement or 32 MiB document limit");
        }
        Ok(())
    }
    fn commit_typing(&mut self, mut plan: Plan, options: TypingOptions) -> Result<()> {
        plan.changes.sort_by_key(|(range, _)| range.start);
        self.validate_typing_size(&plan.changes)?;
        if plan.selections.is_empty() || plan.selections.len() > CURSORS {
            bail!("Invalid typing cursor count");
        }
        self.prepare_typing_options(options);
        let original = self.snapshot();
        self.secondary.clear();
        self.apply_changes(plan.changes);
        self.replace_undo_snapshot(original);
        self.assign_selections(plan.selections);
        if !plan.marks.is_empty() {
            let marks = Arc::make_mut(&mut self.view.pairs.marks);
            marks.extend(plan.marks);
            marks.sort_unstable_by_key(|mark| mark.close);
        }
        Ok(())
    }
    pub fn newline_with_options(&mut self, options: TypingOptions) -> Result<()> {
        self.typing_newline(options, false)
    }
    pub fn line_break_with_options(&mut self, options: TypingOptions) -> Result<()> {
        self.typing_newline(options, true)
    }
    fn commit_indentation(
        &mut self,
        edits: Vec<super::indentation::Edit>,
        options: TypingOptions,
    ) -> Result<()> {
        let mut order = (0..edits.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| edits[*index].range.start);
        let mut selections = vec![Selection::caret(0); edits.len()];
        let mut shift = 0isize;
        for index in order {
            let edit = &edits[index];
            selections[index] =
                Selection::caret(edit.range.start.saturating_add_signed(shift) + edit.offset);
            shift += edit.text.chars().count() as isize - edit.range.len() as isize;
        }
        self.commit_typing(
            Plan {
                changes: edits
                    .into_iter()
                    .map(|edit| (edit.range, edit.text))
                    .collect(),
                selections,
                marks: Vec::new(),
            },
            options,
        )
    }
    fn typing_newline(&mut self, options: TypingOptions, keep_position: bool) -> Result<()> {
        let selections = self.smart_selections()?;
        let mut session = super::indentation::Session::new();
        // Reserve ordinary leading-indent work before optional lexical scans.
        // Exhausting optional work must never turn a later caret into an error.
        let bases = selections
            .iter()
            .map(|selection| session.base(self, &selection.range(), options.indent))
            .collect::<Result<Vec<_>>>()?;
        let snippet = self.in_snippet();
        let edits = selections
            .iter()
            .zip(bases)
            .map(|(selection, base)| {
                session.enter(self, selection, options, base, keep_position, snippet)
            })
            .collect::<Result<Vec<_>>>()?;
        self.commit_indentation(edits, options)?;
        if !keep_position {
            self.typing = Some((Instant::now(), self.cursor));
        }
        Ok(())
    }
    pub fn backspace_with_options(&mut self, options: TypingOptions, word: bool) -> Result<()> {
        let selections = self.smart_selections()?;
        if word || self.in_snippet() || options.delete == PairHandling::Never {
            self.prepare_typing_options(options);
            self.backspace(word);
            return Ok(());
        }
        let paired = selections.iter().all(|selection| {
            let cursor = selection.cursor;
            selection.range().is_empty()
                && cursor > 0
                && cursor < self.len()
                && self
                    .typing_pair(options.profile, self.text.char(cursor - 1))
                    .is_some_and(|pair| pair.close == self.text.char(cursor))
                && (options.delete == PairHandling::Always
                    || self
                        .live_mark(cursor, self.text.char(cursor), options)
                        .is_some_and(|mark| mark.open + 1 == cursor))
        });
        if !paired {
            self.prepare_typing_options(options);
            self.backspace(false);
            return Ok(());
        }
        let removed = selections
            .iter()
            .map(|selection| Selection {
                cursor: selection.cursor + 1,
                anchor: Some(selection.cursor - 1),
                desired_column: None,
            })
            .collect::<Vec<_>>();
        let shifts = shifts(&removed, |_| 0);
        let cursors = removed
            .iter()
            .enumerate()
            .map(|(index, selection)| {
                Selection::caret(selection.range().start.saturating_add_signed(shifts[index]))
            })
            .collect();
        self.commit_typing(
            Plan {
                changes: removed
                    .iter()
                    .map(|selection| (selection.range(), String::new()))
                    .collect(),
                selections: cursors,
                marks: Vec::new(),
            },
            options,
        )
    }
}

// Preserve caller order while computing independent replacement endpoints.
fn validate_configuration(
    configuration: &crate::language_configuration::Configuration,
) -> Result<()> {
    let text = |value: &str, max: usize, allow_empty: bool| {
        (allow_empty || !value.is_empty()) && value.len() <= max && !value.contains('\0')
    };
    if !text(&configuration.language, 128, false)
        || !text(&configuration.identity.owner, 256, false)
        || !text(&configuration.identity.version, 256, false)
        || !text(&configuration.identity.archive_sha256, 256, true)
        || !text(&configuration.identity.content_sha256, 64, false)
        || !text(&configuration.identity.composition_sha256, 64, false)
        || configuration.identity.package_path.as_os_str().len() > 8192
        || configuration.identity.configuration_path.as_os_str().len() > 1024
    {
        bail!("Language configuration identity exceeds native limits");
    }
    let delimiter = |ch: char| !matches!(ch, '\0' | '\n' | '\r');
    if configuration
        .auto_closing_pairs
        .as_ref()
        .is_some_and(|pairs| {
            pairs.len() > 64
                || pairs
                    .iter()
                    .any(|pair| !delimiter(pair.open) || !delimiter(pair.close))
        })
        || [&configuration.surrounding_pairs, &configuration.brackets]
            .into_iter()
            .any(|pairs| {
                pairs.as_ref().is_some_and(|pairs| {
                    pairs.len() > 64
                        || pairs
                            .iter()
                            .any(|pair| !delimiter(pair.open) || !delimiter(pair.close))
                })
            })
    {
        bail!("Language configuration pairs require at most 64 non-line-break scalar delimiters");
    }
    if configuration
        .auto_close_before
        .as_ref()
        .is_some_and(|value| !text(value, 256, true) || value.chars().count() > 64)
    {
        bail!("Language configuration autoCloseBefore exceeds native limits");
    }
    if let Some(comments) = configuration.comments.as_ref() {
        let valid = |value: &str| text(value, 256, false) && !value.contains(['\r', '\n']);
        if comments
            .line_comment
            .as_ref()
            .is_some_and(|value| !valid(value))
            || comments
                .block_comment
                .as_ref()
                .is_some_and(|(open, close)| !valid(open) || !valid(close))
        {
            bail!(
                "Language configuration comment delimiters require nonempty single-line text up to 256 bytes"
            );
        }
    }
    Ok(())
}

fn shifts(selections: &[Selection], added: impl Fn(usize) -> usize) -> Vec<isize> {
    let mut order = (0..selections.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| selections[*index].range().start);
    let mut result = vec![0; selections.len()];
    let mut shift = 0isize;
    for index in order {
        result[index] = shift;
        shift += added(index) as isize - selections[index].range().len() as isize;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_marks_stay_sorted_through_generic_mapping_consumption_and_undo() {
        let options = TypingOptions {
            profile: ProfileId::Cpp,
            ..TypingOptions::default()
        };
        let mut doc = Document::from_text("\r\n");
        for ch in ['(', '[', '{'] {
            doc.type_character(ch, options, false).unwrap();
            assert!(
                doc.pairs
                    .marks
                    .windows(2)
                    .all(|marks| marks[0].close < marks[1].close)
            );
        }
        doc.apply_changes(vec![(0..0, "猫🙂 ".into())]);
        assert_eq!(doc.text.to_string(), "猫🙂 ([{}])\r\n");
        assert!(
            doc.pairs
                .marks
                .windows(2)
                .all(|marks| marks[0].close < marks[1].close)
        );
        let revision = doc.revision;
        for ch in ['}', ']', ')'] {
            doc.type_character(ch, options, false).unwrap();
            assert_eq!(doc.revision, revision);
        }
        assert!(doc.pairs.marks.is_empty());
        assert_eq!(doc.cursor, 9);
        doc.undo();
        assert_eq!(doc.text.to_string(), "([{}])\r\n");
        assert!(
            doc.pairs
                .marks
                .windows(2)
                .all(|marks| marks[0].close < marks[1].close)
        );
        doc.move_to(3, false);
        doc.backspace_with_options(options, false).unwrap();
        assert_eq!(doc.text.to_string(), "([])\r\n");
    }

    #[test]
    fn context_metadata_and_public_probe_count_stay_bounded() {
        let mut doc = Document::from_text(&" ".repeat(1024));
        let positions = (0..1024).collect::<Vec<_>>();
        let proofs = doc.typing_contexts(&positions, ProfileId::Cpp);
        assert!(
            proofs
                .iter()
                .all(|proof| proof.context == LexicalContext::Code)
        );
        assert!(doc.typing_context.checkpoints.len() <= CHECKPOINTS);
        assert!(std::mem::size_of::<Checkpoint>() * CHECKPOINTS <= 64 * 1024);
        assert_eq!(doc.typing_context.checkpoints[0].byte, 0);
        let positions = vec![0; CURSORS + 1];
        assert_eq!(
            doc.typing_contexts(&positions, ProfileId::Cpp).len(),
            CURSORS
        );
    }
}
