//! Current-Rope indentation decisions. No syntax-worker result is trusted here.
//! One gesture shares 64 KiB of scan/evaluation work, 128 lines and 4096 bracket
//! tokens. Unproved context falls back to ordinary insertion/base indentation.
use super::*;
use crate::editing_profile::{AutoIndent, LexicalContext, ProfileId, TypingOptions};
use std::{collections::BTreeMap, sync::Arc};

const BYTES: usize = 64 * 1024;
const LINES: usize = 128;
const TOKENS: usize = 4096;

pub(super) struct Line {
    pub(super) chars: Vec<char>,
    pub(super) code: Vec<bool>,
    pub(super) end_context: LexicalContext,
}
impl Line {
    fn processed(&self, range: Range<usize>) -> String {
        self.chars[range.clone()]
            .iter()
            .enumerate()
            .filter_map(|(index, ch)| {
                (self.code[range.start + index] || !"{}[]()".contains(*ch)).then_some(*ch)
            })
            .collect()
    }
    fn indent_len(&self) -> usize {
        self.chars
            .iter()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .count()
    }
    fn indent(&self) -> String {
        self.chars[..self.indent_len()].iter().collect()
    }
    fn code_at(&self, column: usize) -> bool {
        self.code
            .get(column)
            .copied()
            .unwrap_or(self.end_context == LexicalContext::Code)
    }
}

pub(super) struct Edit {
    pub(super) range: Range<usize>,
    pub(super) text: String,
    pub(super) offset: usize,
}
pub(super) struct Session {
    budget: usize,
    lines: BTreeMap<usize, Arc<Line>>,
}
impl Session {
    pub(super) fn new() -> Self {
        Self {
            budget: BYTES,
            lines: BTreeMap::new(),
        }
    }
    fn line(&mut self, doc: &mut Document, row: usize, profile: ProfileId) -> Option<Arc<Line>> {
        if let Some(line) = self.lines.get(&row) {
            return Some(Arc::clone(line));
        }
        if self.lines.len() == LINES {
            return None;
        }
        let line = Arc::new(doc.scan_typing_line(row, profile, &mut self.budget)?);
        self.lines.insert(row, Arc::clone(&line));
        Some(line)
    }
    fn evaluate(&mut self, line: &Line) -> bool {
        // Reusing cached lexical evidence must not permit 10,000 unbounded
        // evaluations of the same long line within one keyboard gesture.
        let cost = line.chars.len();
        if cost > self.budget {
            return false;
        }
        self.budget -= cost;
        true
    }
    pub(super) fn base(
        &mut self,
        doc: &Document,
        range: &Range<usize>,
        mode: AutoIndent,
    ) -> Result<String> {
        if mode == AutoIndent::None {
            return Ok(String::new());
        }
        let mut indent = String::new();
        for ch in doc
            .text
            .slice(doc.line_start(doc.text.char_to_line(range.start))..range.start)
            .chars()
        {
            if !matches!(ch, ' ' | '\t') {
                break;
            }
            if self.budget == 0 {
                bail!("Typing indentation inspection exceeds 64 KiB");
            }
            self.budget -= 1;
            indent.push(ch);
        }
        Ok(normalize(doc, &indent))
    }
    pub(super) fn enter(
        &mut self,
        doc: &mut Document,
        selection: &Selection,
        options: TypingOptions,
        base: String,
        keep_position: bool,
        snippet: bool,
    ) -> Result<Edit> {
        let mut range = selection.range();
        let mut text = format!("{}{}", doc.eol, base);
        let mut offset = text.chars().count();
        if snippet || matches!(options.indent, AutoIndent::None | AutoIndent::Keep) {
            return Ok(Edit {
                range,
                text,
                offset: if keep_position { 0 } else { offset },
            });
        }
        let row = doc.text.char_to_line(range.start);
        // Previous-line textual rules need exactly the physical previous line,
        // including comments. A blank line is never skipped for this rule.
        let previous = if row > 0
            && options.profile == ProfileId::Cpp
            && matches!(options.indent, AutoIndent::Advanced | AutoIndent::Full)
        {
            self.line(doc, row - 1, options.profile)
        } else {
            None
        };
        let Some(line) = self.line(doc, row, options.profile) else {
            return Ok(Edit {
                range,
                text,
                offset: if keep_position { 0 } else { offset },
            });
        };
        if !self.evaluate(&line) {
            return Ok(Edit {
                range,
                text,
                offset: if keep_position { 0 } else { offset },
            });
        }
        let column = range.start - doc.line_start(row);
        if column > line.chars.len() {
            return Ok(Edit {
                range,
                text,
                offset: if keep_position { 0 } else { offset },
            });
        }
        let before = line.processed(0..column);
        let end_row = doc.text.char_to_line(range.end);
        let after_line = if end_row == row {
            Some(Arc::clone(&line))
        } else {
            self.line(doc, end_row, options.profile)
        };
        if end_row != row && after_line.as_ref().is_some_and(|line| !self.evaluate(line)) {
            return Ok(Edit {
                range,
                text,
                offset: if keep_position { 0 } else { offset },
            });
        }
        let after = after_line.as_ref().and_then(|line| {
            let column = range.end - doc.line_start(end_row);
            (column <= line.chars.len()).then(|| line.processed(column..line.chars.len()))
        });
        if after.is_none() {
            return Ok(Edit {
                range,
                text,
                offset: if keep_position { 0 } else { offset },
            });
        }
        let outdent = previous.as_ref().is_some_and(|previous| {
            self.evaluate(previous)
                && cpp_header(&previous.processed(0..previous.chars.len()))
                && cpp_body(&before)
        });
        if outdent {
            text = format!("{}{}", doc.eol, unshift(doc, &base));
            offset = text.chars().count();
        } else if let Some(open) = before.trim_end_matches(js_whitespace).chars().last()
            && doc.typing_indent_pair(options.profile, open, None)
        {
            text = format!("{}{}", doc.eol, shift(doc, &base));
            offset = text.chars().count();
            if after
                .as_ref()
                .and_then(|after| after.trim_start_matches(js_whitespace).chars().next())
                .is_some_and(|close| doc.typing_indent_pair(options.profile, open, Some(close)))
            {
                text.push_str(&doc.eol);
                text.push_str(&base);
            }
        } else if options.profile == ProfileId::Json && options.indent == AutoIndent::Full {
            let indent = if !before.trim_matches(js_whitespace).is_empty() {
                if increase(&before) {
                    shift(doc, &base)
                } else {
                    base.clone()
                }
            } else {
                self.inherit(doc, row, options.profile, true)
                    .map(|(indent, increased)| {
                        if increased {
                            shift(doc, &indent)
                        } else {
                            indent
                        }
                    })
                    .unwrap_or(base.clone())
            };
            let indent = if after.as_ref().is_some_and(|after| decrease(after)) {
                unshift(doc, &indent)
            } else {
                indent
            };
            if let Some(after_line) = after_line {
                range.end = range
                    .end
                    .max(doc.line_start(end_row) + after_line.indent_len());
            }
            text = format!("{}{}", doc.eol, indent);
            offset = text.chars().count();
        }
        Ok(Edit {
            range,
            text,
            offset: if keep_position { 0 } else { offset },
        })
    }
    fn inherit(
        &mut self,
        doc: &mut Document,
        row: usize,
        profile: ProfileId,
        intentional: bool,
    ) -> Option<(String, bool)> {
        let mut nearest = None;
        for prior in (row.saturating_sub(LINES)..row).rev() {
            let line = self.line(doc, prior, profile)?;
            if !self.evaluate(&line) {
                return None;
            }
            let processed = line.processed(0..line.chars.len());
            if processed.trim_matches(js_whitespace).is_empty() {
                continue;
            }
            let indent = normalize(doc, &line.indent());
            if increase(&processed) {
                return Some((indent, true));
            }
            if decrease(&processed) || intentional {
                return Some((indent, false));
            }
            nearest = Some((indent, false));
        }
        // Reaching the real start is a complete inheritance proof. Reaching
        // the scan window's boundary is not permission to invent a baseline.
        (row < LINES).then_some(nearest).flatten()
    }
    pub(super) fn closing(
        &mut self,
        doc: &mut Document,
        selections: &[Selection],
        ch: char,
        options: TypingOptions,
    ) -> Option<Vec<Edit>> {
        if !matches!(ch, ')' | ']' | '}') || selections.iter().any(|s| !s.range().is_empty()) {
            return None;
        }
        if options.profile == ProfileId::Json && options.indent == AutoIndent::Full {
            let edits = selections
                .iter()
                .map(|selection| self.json_close(doc, selection.cursor, ch, options.profile))
                .collect::<Option<Vec<_>>>();
            if edits.is_some() {
                return edits;
            }
        }
        if selections.len() != 1 {
            return None;
        }
        self.electric(doc, selections[0].cursor, ch, options.profile)
            .map(|edit| vec![edit])
    }
    fn json_close(
        &mut self,
        doc: &mut Document,
        cursor: usize,
        ch: char,
        profile: ProfileId,
    ) -> Option<Edit> {
        if !matches!(ch, ']' | '}') {
            return None;
        }
        let row = doc.text.char_to_line(cursor);
        let line = self.line(doc, row, profile)?;
        if !self.evaluate(&line) {
            return None;
        }
        let column = cursor - doc.line_start(row);
        if column > line.chars.len() || !line.code_at(column) {
            return None;
        }
        let before = line.processed(0..column);
        let after = line.processed(column..line.chars.len());
        if decrease(&(before.clone() + &after)) || !decrease(&format!("{before}{ch}{after}")) {
            return None;
        }
        let (indent, increased) = self.inherit(doc, row, profile, false)?;
        let indent = if increased {
            indent
        } else {
            unshift(doc, &indent)
        };
        if line.indent() == indent {
            return None;
        }
        let content: String = line.chars[line.indent_len().min(column)..column]
            .iter()
            .collect();
        let text = format!("{indent}{content}{ch}");
        Some(Edit {
            range: doc.line_start(row)..cursor,
            offset: text.chars().count(),
            text,
        })
    }
    fn electric(
        &mut self,
        doc: &mut Document,
        cursor: usize,
        close: char,
        profile: ProfileId,
    ) -> Option<Edit> {
        let open = matching(close)?;
        if !doc.typing_indent_pair(profile, open, Some(close)) {
            return None;
        }
        let row = doc.text.char_to_line(cursor);
        let first = row.saturating_sub(LINES - 1);
        let mut tokens = Vec::new();
        let mut current = None;
        for prior in first..=row {
            let line = self.line(doc, prior, profile)?;
            if !self.evaluate(&line) {
                return None;
            }
            let limit = if prior == row {
                cursor - doc.line_start(row)
            } else {
                line.chars.len()
            };
            if limit > line.chars.len() {
                return None;
            }
            for (column, ch) in line.chars[..limit].iter().enumerate() {
                if line.code[column]
                    && (doc.typing_indent_pair(profile, *ch, None)
                        || matching(*ch)
                            .is_some_and(|open| doc.typing_indent_pair(profile, open, Some(*ch))))
                {
                    if tokens.len() == TOKENS {
                        return None;
                    }
                    tokens.push((prior, *ch));
                }
            }
            if prior == row {
                current = Some(line);
            }
        }
        let line = current?;
        let column = cursor - doc.line_start(row);
        if column > line.indent_len() || !line.code_at(column) {
            return None;
        }
        let mut stack = vec![open];
        for (prior, ch) in tokens.into_iter().rev() {
            if let Some(open) = matching(ch) {
                stack.push(open);
            } else if stack.pop() != Some(ch) {
                return None;
            } else if stack.is_empty() {
                if prior == row {
                    return None;
                }
                let opener = self.line(doc, prior, profile)?;
                let indent = normalize(doc, &opener.indent());
                let text = format!("{indent}{close}");
                return Some(Edit {
                    range: doc.line_start(row)..cursor,
                    offset: text.chars().count(),
                    text,
                });
            }
        }
        None
    }
}
fn matching(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}
fn width(doc: &Document, indent: &str) -> usize {
    let tab_size = doc.tab_size.clamp(1, 16);
    indent.chars().fold(0, |width, ch| {
        if ch == '\t' {
            width + tab_size - width % tab_size
        } else {
            width + 1
        }
    })
}
fn render(doc: &Document, columns: usize) -> String {
    let tab_size = doc.tab_size.clamp(1, 16);
    if doc.insert_spaces {
        " ".repeat(columns)
    } else {
        "\t".repeat(columns / tab_size) + &" ".repeat(columns % tab_size)
    }
}
fn normalize(doc: &Document, indent: &str) -> String {
    render(doc, width(doc, indent))
}
fn shift(doc: &Document, indent: &str) -> String {
    let tab_size = doc.tab_size.clamp(1, 16);
    render(doc, (width(doc, indent) / tab_size + 1) * tab_size)
}
fn unshift(doc: &Document, indent: &str) -> String {
    let tab_size = doc.tab_size.clamp(1, 16);
    render(
        doc,
        width(doc, indent).saturating_sub(1) / tab_size * tab_size,
    )
}
fn cpp_header(text: &str) -> bool {
    let text = text.trim_matches(js_whitespace);
    if text == "else" {
        return true;
    }
    ["if", "elseif", "else if", "for", "while"]
        .iter()
        .any(|keyword| {
            text.strip_prefix(keyword).is_some_and(|rest| {
                let rest = rest.trim_start_matches(js_whitespace);
                rest.starts_with('(') && rest.ends_with(')')
            })
        })
}
fn cpp_body(text: &str) -> bool {
    if !text.chars().next().is_some_and(js_whitespace) {
        return false;
    }
    let body = text.trim_start_matches(js_whitespace);
    if body.is_empty() || body.starts_with('{') {
        return false;
    }
    !body.strip_prefix("if").is_some_and(|rest| {
        rest.chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
    })
}
fn decrease(text: &str) -> bool {
    let text = text.trim_matches(js_whitespace);
    matches!(text, "}" | "}," | "]" | "],")
}
fn js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
fn increase(text: &str) -> bool {
    // Bracket characters in strings/comments have already been removed. For
    // each bracket kind the bundled JSON rule tests the last such character.
    let curly = text.chars().rev().find(|ch| matches!(ch, '{' | '}'));
    let square = text.chars().rev().find(|ch| matches!(ch, '[' | ']'));
    curly == Some('{') || square == Some('[')
}
