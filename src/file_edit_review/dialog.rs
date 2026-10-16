//! Interactive review of immutable proposed edits. Selection never rewrites DTOs.
use super::{Choice, Initial, Overview};
use anyhow::{Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ask,
    Preview,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Apply,
    Preview,
    Skip,
    Remember,
    Rows,
    Details,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Remember(bool),
    Apply {
        indices: Vec<usize>,
        remember: Option<bool>,
    },
    Skip,
}
pub struct Dialog {
    id: u64,
    overview: Arc<Overview>,
    names: Vec<String>,
    phase: Phase,
    focus: Focus,
    requires_preview: bool,
    remember: bool,
    selected: Vec<bool>,
    cursor: usize,
    details: Vec<String>,
    details_offset: usize,
    preview: Option<Arc<super::preview::Preview>>,
}
impl Dialog {
    pub fn new(
        id: u64,
        overview: Arc<Overview>,
        names: Vec<String>,
        initial: Initial,
    ) -> Result<Self> {
        ensure!(
            id > 0 && !overview.rows().is_empty(),
            "No proposed file edits to review"
        );
        ensure!(
            names.len() <= 128 && names.iter().all(|name| name.len() <= 4096),
            "Contributor names exceed review bounds"
        );
        let (phase, requires_preview) = match initial {
            Initial::Ask { requires_preview } => (Phase::Ask, requires_preview),
            Initial::Apply { show_preview: true } => (Phase::Preview, false),
            _ => anyhow::bail!("This file-edit decision does not require a dialog"),
        };
        let selected = vec![true; overview.rows().len()];
        let mut dialog = Self {
            id,
            overview,
            names,
            phase,
            focus: if phase == Phase::Preview {
                Focus::Rows
            } else if requires_preview {
                Focus::Preview
            } else {
                Focus::Apply
            },
            requires_preview,
            remember: false,
            selected,
            cursor: 0,
            details: Vec::new(),
            details_offset: 0,
            preview: None,
        };
        dialog.refresh_details();
        Ok(dialog)
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub(crate) fn with_preview(mut self, preview: Arc<super::preview::Preview>) -> Self {
        self.preview = Some(preview);
        self.refresh_details();
        self
    }
    pub fn overview(&self) -> &Arc<Overview> {
        &self.overview
    }
    pub fn contributors(&self) -> &[String] {
        &self.names
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn focus(&self) -> Focus {
        self.focus
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn row_selected(&self, index: usize) -> bool {
        self.selected.get(index).copied().unwrap_or(false)
    }
    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|selected| **selected).count()
    }
    pub fn remember_checked(&self) -> bool {
        self.remember
    }
    pub fn requires_preview(&self) -> bool {
        self.requires_preview
    }
    pub fn details(&self) -> &[String] {
        &self.details
    }
    pub fn details_offset(&self) -> usize {
        self.details_offset
    }
    pub fn selected_indices(&self) -> Vec<usize> {
        self.overview
            .rows()
            .iter()
            .zip(&self.selected)
            .filter_map(|(row, selected)| selected.then_some(row.index))
            .collect()
    }
    fn focuses(&self) -> &'static [Focus] {
        if self.phase == Phase::Preview {
            &[Focus::Rows, Focus::Details, Focus::Apply, Focus::Skip]
        } else if self.requires_preview {
            &[Focus::Preview, Focus::Skip]
        } else {
            &[Focus::Apply, Focus::Preview, Focus::Skip, Focus::Remember]
        }
    }
    fn cycle(&mut self, reverse: bool) {
        let focuses = self.focuses();
        let current = focuses
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next = if reverse {
            (current + focuses.len() - 1) % focuses.len()
        } else {
            (current + 1) % focuses.len()
        };
        self.focus = focuses[next];
    }
    pub fn key(&mut self, key: KeyEvent) -> Option<Event> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        match key.code {
            KeyCode::Esc => return Some(Event::Skip),
            KeyCode::Tab => self.cycle(key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::BackTab => self.cycle(true),
            KeyCode::Left => self.cycle(true),
            KeyCode::Right => self.cycle(false),
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
                if self.phase == Phase::Preview =>
            {
                let down = matches!(key.code, KeyCode::Down | KeyCode::PageDown | KeyCode::End);
                let page = matches!(key.code, KeyCode::PageUp | KeyCode::PageDown);
                let end = matches!(key.code, KeyCode::Home | KeyCode::End);
                if self.focus == Focus::Details {
                    let max = self.details.len().saturating_sub(1);
                    self.details_offset = moved(self.details_offset, max, down, page, end);
                } else if self.focus == Focus::Rows {
                    let max = self.selected.len().saturating_sub(1);
                    self.cursor = moved(self.cursor, max, down, page, end);
                    self.refresh_details();
                } else {
                    self.cycle(!down);
                }
            }
            KeyCode::Up => self.cycle(true),
            KeyCode::Down => self.cycle(false),
            KeyCode::Enter | KeyCode::Char(' ') => return self.activate(),
            _ => {}
        }
        None
    }
    fn activate(&mut self) -> Option<Event> {
        match self.focus {
            Focus::Skip => Some(Event::Skip),
            Focus::Remember => {
                self.remember = !self.remember;
                None
            }
            Focus::Rows => {
                self.selected[self.cursor] = !self.selected[self.cursor];
                None
            }
            Focus::Details => None,
            Focus::Preview => {
                let remember = super::choose(
                    Initial::Ask {
                        requires_preview: self.requires_preview,
                    },
                    Choice::Preview,
                    self.remember,
                )
                .ok()?;
                self.phase = Phase::Preview;
                self.focus = Focus::Rows;
                match remember {
                    super::Decision::Apply {
                        remember: Some(value),
                        ..
                    } => Some(Event::Remember(value)),
                    _ => None,
                }
            }
            Focus::Apply if self.selected_count() > 0 => {
                let remember = if self.phase == Phase::Ask {
                    match super::choose(
                        Initial::Ask {
                            requires_preview: self.requires_preview,
                        },
                        Choice::Apply,
                        self.remember,
                    )
                    .ok()?
                    {
                        super::Decision::Apply { remember, .. } => remember,
                        _ => None,
                    }
                } else {
                    None
                };
                Some(Event::Apply {
                    indices: self.selected_indices(),
                    remember,
                })
            }
            Focus::Apply => None,
        }
    }
    fn refresh_details(&mut self) {
        self.details_offset = 0;
        let row = &self.overview.rows()[self.cursor];
        let mut lines = vec!["Proposed edit details; baseline diff is not loaded.".into()];
        if let Some(preview) = self
            .preview
            .as_ref()
            .and_then(|preview| preview.get(&row.index))
        {
            lines = preview.clone();
        }
        if let Some(destination) = &row.destination {
            lines.push(format!("From: {}", plain(&row.resource, 240)));
            lines.push(format!("To: {}", plain(destination, 240)));
        }
        if let Some(entry) = self.overview.original()["edits"].get(row.index) {
            if matches!(row.change, super::Change::Snippet) {
                lines.push("Snippet source below; placeholders have not been expanded.".into());
            }
            if let Some(eol) = entry.get("textEdit").and_then(|edit| edit.get("eol")) {
                lines.push(format!(
                    "Proposed line endings: {}",
                    match eol.as_u64() {
                        Some(0) => "LF",
                        Some(1) => "CRLF",
                        _ => "original EOL value below",
                    }
                ));
            }
            details_value("entry", entry, 0, &mut lines);
        }
        self.details = lines;
    }
}
fn moved(current: usize, max: usize, down: bool, page: bool, end: bool) -> usize {
    if end {
        if down { max } else { 0 }
    } else if down {
        current.saturating_add(if page { 10 } else { 1 }).min(max)
    } else {
        current.saturating_sub(if page { 10 } else { 1 })
    }
}
/// Display only: control and terminal/bidi formatting characters cannot execute.
pub fn plain(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut output = String::new();
    for ch in chars.by_ref().take(max_chars) {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control()
                || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                output.push('�')
            }
            ch => output.push(ch),
        }
    }
    if chars.next().is_some() {
        output.push('…');
    }
    output
}
fn details_value(label: &str, value: &Value, depth: usize, lines: &mut Vec<String>) {
    const MAX_LINES: usize = 256;
    if lines.len() >= MAX_LINES || depth > 12 {
        if lines
            .last()
            .is_none_or(|line| line != "Display clipped; complete original edit is retained.")
        {
            lines.push("Display clipped; complete original edit is retained.".into());
        }
        return;
    }
    let label = format!("{}{}", "  ".repeat(depth), plain(label, 80));
    match value {
        Value::Object(fields) => {
            lines.push(format!("{label}:"));
            for (name, value) in fields {
                details_value(name, value, depth + 1, lines);
                if lines.len() >= MAX_LINES {
                    break;
                }
            }
        }
        Value::Array(values) => {
            lines.push(format!("{label}: [{} items]", values.len()));
            for (index, value) in values.iter().enumerate() {
                details_value(&format!("[{index}]"), value, depth + 1, lines);
                if lines.len() >= MAX_LINES {
                    break;
                }
            }
        }
        Value::String(value) => {
            let clipped = value.chars().take(241).count() > 240;
            lines.push(format!("{label}: {}", plain(value, 240)));
            if clipped {
                lines.push("Display clipped; complete original edit is retained.".into());
            }
        }
        _ => lines.push(format!("{label}: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn dialog(required: bool) -> Dialog {
        let overview = Arc::new(
            Overview::prepare(Arc::new(json!({"edits":[
                {"newResource":"file:///a","options":{"contents":{"type":"base64","value":"AAH/"}}},
                {"resource":"file:///b","textEdit":{"text":"猫🙂\r\n","insertAsSnippet":true}},
                {"resource":"notebook:///c","cellEdit":{"editType":5}}
            ]})))
            .unwrap(),
        );
        Dialog::new(
            1,
            overview,
            vec!["Contributor".into()],
            Initial::Ask {
                requires_preview: required,
            },
        )
        .unwrap()
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn direct_apply_preserves_all_original_indices_and_remember_policy() {
        let mut dialog = dialog(false);
        let original = dialog.overview().original().clone();
        dialog.key(key(KeyCode::BackTab));
        dialog.key(key(KeyCode::Char(' ')));
        dialog.key(key(KeyCode::Tab));
        assert_eq!(
            dialog.key(key(KeyCode::Enter)),
            Some(Event::Apply {
                indices: vec![0, 1, 2],
                remember: Some(false)
            })
        );
        assert!(Arc::ptr_eq(dialog.overview().original(), &original));
        assert_eq!(original["edits"][0]["options"]["contents"]["value"], "AAH/");
    }
    #[test]
    fn preview_persists_choice_before_cancel_and_selection_keeps_original_indices() {
        let mut dialog = dialog(false);
        dialog.key(key(KeyCode::BackTab));
        dialog.key(key(KeyCode::Char(' ')));
        dialog.key(key(KeyCode::Tab));
        dialog.key(key(KeyCode::Tab));
        assert_eq!(dialog.key(key(KeyCode::Enter)), Some(Event::Remember(true)));
        dialog.key(key(KeyCode::Down));
        dialog.key(key(KeyCode::Char(' ')));
        assert_eq!(dialog.selected_indices(), vec![0, 2]);
        assert_eq!(dialog.overview().rows().len(), 3);
        assert_eq!(dialog.key(key(KeyCode::Esc)), Some(Event::Skip));
    }
    #[test]
    fn required_confirmation_offers_preview_without_remember_or_direct_apply() {
        let mut dialog = dialog(true);
        assert_eq!(dialog.focus(), Focus::Preview);
        assert_eq!(dialog.key(key(KeyCode::Enter)), None);
        assert_eq!(dialog.phase(), Phase::Preview);
        dialog.key(key(KeyCode::Tab));
        dialog.key(key(KeyCode::Tab));
        assert_eq!(
            dialog.key(key(KeyCode::Enter)),
            Some(Event::Apply {
                indices: vec![0, 1, 2],
                remember: None
            })
        );
    }
    #[test]
    fn preview_navigation_empty_selection_and_release_keys_are_safe() {
        let mut dialog = dialog(true);
        dialog.key(key(KeyCode::Enter));
        for _ in 0..3 {
            dialog.key(key(KeyCode::Char(' ')));
            dialog.key(key(KeyCode::Down));
        }
        assert_eq!(dialog.selected_count(), 0);
        dialog.key(key(KeyCode::Tab));
        dialog.key(key(KeyCode::End));
        assert!(dialog.details_offset() > 0);
        dialog.key(key(KeyCode::Tab));
        assert_eq!(dialog.key(key(KeyCode::Enter)), None);
        let mut released = key(KeyCode::Esc);
        released.kind = KeyEventKind::Release;
        assert_eq!(dialog.key(released), None);
    }
    #[test]
    fn hostile_and_long_details_are_bounded_without_altering_original() {
        let text = format!("\u{001b}[31m\u{202e}{}", "猫".repeat(100_000));
        let original =
            Arc::new(json!({"edits":[{"resource":"file:///a","textEdit":{"text":text}}]}));
        let overview = Arc::new(Overview::prepare(original.clone()).unwrap());
        let dialog =
            Dialog::new(1, overview, vec![], Initial::Apply { show_preview: true }).unwrap();
        assert!(dialog.details().len() <= 257);
        assert!(
            dialog
                .details()
                .iter()
                .all(|line| !line.contains('\u{001b}') && !line.contains('\u{202e}'))
        );
        assert!(
            dialog
                .details()
                .iter()
                .any(|line| line.contains("Display clipped"))
        );
        assert!(Arc::ptr_eq(dialog.overview().original(), &original));
        assert_eq!(
            original["edits"][0]["textEdit"]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            100_006
        );
    }
    #[test]
    fn eol_details_use_internal_zero_based_sequence_and_keep_original_value() {
        for (eol, label) in [(0, "LF"), (1, "CRLF")] {
            let original = Arc::new(json!({"edits":[
                {"resource":"file:///a","textEdit":{"text":"","eol":eol}}
            ]}));
            let overview = Arc::new(Overview::prepare(original.clone()).unwrap());
            let dialog =
                Dialog::new(1, overview, vec![], Initial::Apply { show_preview: true }).unwrap();
            assert!(
                dialog
                    .details()
                    .iter()
                    .any(|line| line == &format!("Proposed line endings: {label}"))
            );
            assert_eq!(original["edits"][0]["textEdit"]["eol"], eol);
        }
    }
}
