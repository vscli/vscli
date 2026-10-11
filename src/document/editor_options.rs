//! Native resolved model options and per-view presentation overrides.
//! Indentation detection follows Microsoft's MIT-licensed indentationGuesser.ts
//! at VS Code 2a59476c9bfcb90b3ddc372c36762471b7dfad1c, with a bounded native scan.
// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT License:
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
// of the Software, and to permit persons to whom the Software is furnished to do
// so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
use super::*;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) enum AutoValue {
    #[serde(rename = "auto")]
    Auto,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum SizeOption {
    Number(usize),
    Auto(AutoValue),
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum TabSizeValue {
    #[serde(rename = "tabSize")]
    TabSize,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum IndentOption {
    Number(usize),
    TabSize(TabSizeValue),
}
impl Default for IndentOption {
    fn default() -> Self {
        Self::TabSize(TabSizeValue::TabSize)
    }
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum SpaceOption {
    Bool(bool),
    Auto(AutoValue),
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EditorOptionsPatch {
    pub tab_size: Option<SizeOption>,
    pub indent_size: Option<IndentOption>,
    pub insert_spaces: Option<SpaceOption>,
    pub cursor_style: Option<u8>,
    pub line_numbers: Option<u8>,
}
#[derive(Clone, Copy)]
pub(super) struct Defaults {
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub cursor_style: u8,
}
impl Default for Defaults {
    fn default() -> Self {
        Self {
            tab_size: 4,
            insert_spaces: true,
            cursor_style: 1,
        }
    }
}
fn width(value: usize) -> Result<usize> {
    if !(1..=16).contains(&value) {
        bail!("Native tabSize and indentSize require widths 1–16");
    }
    Ok(value)
}
impl IndentOption {
    pub(super) fn resolved(self, tab: usize) -> usize {
        match self {
            Self::Number(n) => n,
            Self::TabSize(_) => tab,
        }
    }
    fn wire(self) -> Value {
        match self {
            Self::Number(n) => json!(n),
            Self::TabSize(_) => json!("tabSize"),
        }
    }
}
impl Document {
    pub fn indent_size(&self) -> usize {
        self.indent_size.resolved(self.tab_size).clamp(1, 16)
    }
    pub fn editor_line_numbers(&self) -> crate::settings::LineNumbers {
        self.view.line_numbers_override.unwrap_or(self.line_numbers)
    }
    pub fn editor_cursor_style(&self) -> u8 {
        self.view
            .cursor_style_override
            .unwrap_or(self.editor_defaults.cursor_style)
    }
    pub fn editor_cursor_following(&self) -> bool {
        !self.view.explicit_viewport
    }
    pub fn resume_editor_cursor_follow(&mut self) {
        self.view.explicit_viewport = false;
    }
    pub(crate) fn editor_options_for(&self, id: u64) -> Option<Value> {
        let view = if id == self.active_view {
            Some(&self.view)
        } else {
            self.other_views.get(&id)
        }?;
        let numbers = view.line_numbers_override.unwrap_or(self.line_numbers);
        let numbers = match numbers {
            crate::settings::LineNumbers::Off => 0,
            crate::settings::LineNumbers::On => 1,
            crate::settings::LineNumbers::Relative => 2,
            crate::settings::LineNumbers::Interval => 3,
        };
        Some(
            json!({"tabSize":self.tab_size,"indentSize":self.indent_size(),
            "originalIndentSize":self.indent_size.wire(),"insertSpaces":self.insert_spaces,
            "cursorStyle":view.cursor_style_override.unwrap_or(self.editor_defaults.cursor_style),
            "lineNumbers":numbers}),
        )
    }
    pub(crate) fn apply_editor_options(
        &mut self,
        proof: &EditorViewProof,
        patch: &EditorOptionsPatch,
    ) -> Result<()> {
        if !self.editor_view_current(proof) {
            bail!("Original editor view retired");
        }
        if let Some(SizeOption::Number(n)) = patch.tab_size {
            width(n)?;
        }
        if let Some(IndentOption::Number(n)) = patch.indent_size {
            width(n)?;
        }
        if patch
            .cursor_style
            .is_some_and(|value| !(1..=6).contains(&value))
            || patch.line_numbers.is_some_and(|value| value > 3)
        {
            bail!("Invalid cursorStyle or lineNumbers");
        }
        let automatic = matches!(patch.tab_size, Some(SizeOption::Auto(_)))
            || matches!(patch.insert_spaces, Some(SpaceOption::Auto(_)));
        let mut tab = match patch.tab_size {
            Some(SizeOption::Number(n)) => n,
            _ if automatic => self.editor_defaults.tab_size,
            _ => self.tab_size,
        };
        let mut spaces = match patch.insert_spaces {
            Some(SpaceOption::Bool(value)) => value,
            _ if automatic => self.editor_defaults.insert_spaces,
            _ => self.insert_spaces,
        };
        let indent = if automatic {
            (tab, spaces) = guess_indentation(self, tab, spaces)?;
            // mainThreadEditor ignores explicit indentSize in its auto branch;
            // TextModel.detectIndentation publishes the detected numeric width.
            IndentOption::Number(tab)
        } else {
            patch.indent_size.unwrap_or(self.indent_size)
        };
        self.set_indentation(tab, spaces);
        self.indent_size = indent;
        let view = if proof.id == self.active_view {
            &mut self.view
        } else {
            self.other_views
                .get_mut(&proof.id)
                .expect("validated retained view")
        };
        if let Some(style) = patch.cursor_style {
            view.cursor_style_override = Some(style);
        }
        if let Some(numbers) = patch.line_numbers {
            view.line_numbers_override = Some(match numbers {
                0 => crate::settings::LineNumbers::Off,
                1 => crate::settings::LineNumbers::On,
                2 => crate::settings::LineNumbers::Relative,
                _ => crate::settings::LineNumbers::Interval,
            });
        }
        Ok(())
    }
    pub(crate) fn set_editor_defaults(
        &mut self,
        tab: usize,
        indent: IndentOption,
        spaces: bool,
        numbers: crate::settings::LineNumbers,
        style: u8,
    ) {
        self.editor_defaults = Defaults {
            tab_size: tab.clamp(1, 16),
            insert_spaces: spaces,
            cursor_style: style,
        };
        self.set_indentation(tab, spaces);
        self.indent_size = indent;
        self.line_numbers = numbers;
    }
    pub(crate) fn reveal_editor_view(
        &mut self,
        proof: &EditorViewProof,
        top: usize,
        left: usize,
    ) -> Result<()> {
        if !self.editor_view_current(proof) {
            bail!("Original editor view retired");
        }
        // Geometry is supplied by the App after exact source/view validation;
        // protect arithmetic even for malformed internal callers.
        if left > MAX_FILE_BYTES as usize * 16 {
            bail!("Native reveal column exceeds bounds");
        }
        let top = top.min(self.line_count() - 1);
        let view = if proof.id == self.active_view {
            &mut self.view
        } else {
            self.other_views
                .get_mut(&proof.id)
                .expect("validated retained view")
        };
        view.top = top;
        view.left = left;
        view.explicit_viewport = true;
        Ok(())
    }
}

fn spaces_diff(a: &[u16], a_indent: usize, b: &[u16], b_indent: usize) -> (usize, bool) {
    let common = a
        .iter()
        .take(a_indent)
        .zip(b.iter().take(b_indent))
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = |line: &[u16], indent: usize| {
        let spaces = line[common..indent].iter().filter(|ch| **ch == 32).count();
        (spaces, indent - common - spaces)
    };
    let (a_spaces, a_tabs) = suffix(a, a_indent);
    let (b_spaces, b_tabs) = suffix(b, b_indent);
    if a_spaces > 0 && a_tabs > 0 || b_spaces > 0 && b_tabs > 0 {
        return (0, false);
    }
    let tabs = a_tabs.abs_diff(b_tabs);
    let spaces = a_spaces.abs_diff(b_spaces);
    if tabs == 0 {
        let alignment = spaces > 0
            && b_spaces > 0
            && b_spaces - 1 < a.len()
            && b_spaces < b.len()
            && b[b_spaces] != 32
            && a[b_spaces - 1] == 32
            && a.last() == Some(&44);
        (spaces, alignment)
    } else if spaces.is_multiple_of(tabs) {
        (spaces / tabs, false)
    } else {
        (0, false)
    }
}
fn guess_indentation(
    doc: &Document,
    default_tab: usize,
    default_spaces: bool,
) -> Result<(usize, bool)> {
    let mut previous = Vec::<u16>::new();
    let mut previous_indent = 0;
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    let mut scores = [0usize; 9];
    let mut charged = 0usize;
    for row in 0..doc.line_count().min(10_000) {
        let line = doc.line_slice(row);
        charged = charged.saturating_add(line.len_bytes());
        if charged > 256 * 1024 {
            bail!("Auto indentation exceeds 256 KiB scan budget");
        }
        let current = line
            .chars()
            .flat_map(|ch| {
                let mut units = [0; 2];
                let length = ch.encode_utf16(&mut units).len();
                units.into_iter().take(length)
            })
            .collect::<Vec<u16>>();
        let indent = current
            .iter()
            .take_while(|ch| matches!(**ch, 9 | 32))
            .count();
        if indent == current.len() {
            continue;
        }
        let line_tabs = current[..indent].iter().filter(|ch| **ch == 9).count();
        if line_tabs > 0 {
            tabs += 1;
        } else if indent > 1 {
            spaces += 1;
        }
        let (difference, alignment) = spaces_diff(&previous, previous_indent, &current, indent);
        if alignment && !(default_spaces && default_tab == difference) {
            continue;
        }
        if difference <= 8 {
            scores[difference] += 1;
        }
        previous = current;
        previous_indent = indent;
    }
    let insert_spaces = if tabs == spaces {
        default_spaces
    } else {
        tabs < spaces
    };
    let mut tab = default_tab;
    if insert_spaces {
        let mut score = 0;
        for candidate in [2, 4, 6, 8, 3, 5, 7] {
            if scores[candidate] > score {
                score = scores[candidate];
                tab = candidate;
            }
        }
        if tab == 4 && scores[4] > 0 && scores[2] > 0 && scores[2] * 3 >= scores[4] * 2 {
            tab = 2;
        }
    }
    Ok((tab, insert_spaces))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn patch(value: Value) -> EditorOptionsPatch {
        serde_json::from_value(value).unwrap()
    }
    fn source(doc: &Document) -> (u64, String, u64, u64, u64, u64, bool) {
        (
            doc.id,
            doc.text.to_string(),
            doc.revision,
            doc.text_epoch(),
            doc.saved_revision,
            doc.save_generation(),
            doc.dirty(),
        )
    }
    #[test]
    fn options_are_shared_model_and_exact_per_view_without_changing_directional_history() {
        let mut doc = Document::from_text("α😀\r\n\t猫\r\n");
        doc.activate_view(1);
        doc.insert("!", false);
        doc.undo();
        doc.set_selections(vec![
            Selection {
                anchor: Some(2),
                cursor: 0,
                desired_column: Some(7),
            },
            Selection::caret(5),
        ]);
        let first = doc.editor_view_identity();
        let selections = doc.selections();
        doc.activate_view(2);
        doc.move_to(6, false);
        let active = doc.selections();
        let before = source(&doc);
        doc.apply_editor_options(
            &first,
            &patch(json!({"tabSize":4,"indentSize":2,"insertSpaces":true,
            "lineNumbers":2,"cursorStyle":5})),
        )
        .unwrap();
        assert_eq!(doc.editor_view_stamp().0, 2);
        assert_eq!(doc.selections(), active);
        assert_eq!(doc.view_state(Some(1)).anchor, selections[0].anchor);
        assert_eq!(doc.view_state(Some(1)).cursor, selections[0].cursor);
        assert_eq!(doc.view_state(Some(1)).desired_column, Some(7));
        assert_eq!(
            doc.editor_options_for(1).unwrap(),
            json!({"tabSize":4,"indentSize":2,
            "originalIndentSize":2,"insertSpaces":true,"lineNumbers":2,"cursorStyle":5})
        );
        assert_eq!(doc.editor_options_for(2).unwrap()["cursorStyle"], 1);
        assert_eq!(doc.editor_options_for(2).unwrap()["lineNumbers"], 1);
        assert_eq!(doc.indentation(), "  ");
        assert_eq!(doc.display_width("\t猫"), 6);
        assert_eq!(source(&doc), before);
        doc.redo();
        assert_eq!(doc.text.to_string(), format!("!{}", before.1));
        doc.undo();
        assert_eq!(doc.text.to_string(), before.1);
        assert_eq!(doc.editor_options_for(1).unwrap()["indentSize"], 2);
    }
    #[test]
    fn resolved_indent_size_controls_actual_tab_outdent_and_bracket_enter() {
        let mut doc = Document::from_text("if (x) {");
        let view = doc.editor_view_identity();
        doc.apply_editor_options(
            &view,
            &patch(json!({"tabSize":8,"indentSize":2,"insertSpaces":true})),
        )
        .unwrap();
        doc.move_to(doc.len(), false);
        doc.newline_with_options(crate::editing_profile::TypingOptions {
            profile: crate::editing_profile::ProfileId::Cpp,
            indent: crate::editing_profile::AutoIndent::Brackets,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc.text.to_string(), format!("if (x) {{{}  ", doc.eol));
        doc.undo();
        assert_eq!(doc.text.to_string(), "if (x) {");
        let mut lines = Document::from_text("α😀\r\n猫\r\n");
        let view = lines.editor_view_identity();
        lines
            .apply_editor_options(
                &view,
                &patch(json!({"tabSize":4,"indentSize":2,"insertSpaces":true})),
            )
            .unwrap();
        lines.select_all();
        lines.transform_lines(false, None);
        assert_eq!(lines.text.to_string(), "  α😀\r\n  猫\r\n");
        lines.transform_lines(true, None);
        assert_eq!(lines.text.to_string(), "α😀\r\n猫\r\n");
        lines.undo();
        assert_eq!(lines.text.to_string(), "  α😀\r\n  猫\r\n");
        lines.undo();
        assert_eq!(lines.text.to_string(), "α😀\r\n猫\r\n");
        lines
            .apply_editor_options(&view, &patch(json!({"insertSpaces":false})))
            .unwrap();
        assert_eq!(lines.indentation(), "  ");
        lines
            .apply_editor_options(&view, &patch(json!({"indentSize":"tabSize"})))
            .unwrap();
        assert_eq!(lines.indentation(), "\t");
        lines
            .apply_editor_options(&view, &patch(json!({"tabSize":8})))
            .unwrap();
        assert_eq!(lines.editor_options_for(0).unwrap()["indentSize"], 8);
    }
    #[test]
    fn line_comments_use_indent_size_without_changing_tab_display_width_or_history() {
        let original = "  α😀\r\n    猫\r\n";
        let mut doc = Document::from_text(original);
        doc.set_language_override(Some("cpp".into())).unwrap();
        let view = doc.editor_view_identity();
        doc.apply_editor_options(&view, &patch(json!({"tabSize":4,"indentSize":2})))
            .unwrap();
        doc.select_all();
        doc.comment_lines(super::super::comments::CommentOperation::Add)
            .unwrap();
        let commented = "  // α😀\r\n  //   猫\r\n";
        assert_eq!(doc.text.to_string(), commented);
        assert_eq!(doc.display_width("\t猫"), 6);
        doc.undo();
        assert_eq!(doc.text.to_string(), original);
        doc.redo();
        assert_eq!(doc.text.to_string(), commented);
    }
    #[test]
    fn auto_jointly_detects_from_content_with_creation_defaults_and_refuses_before_mutation() {
        let mut doc = Document::from_text("root\r\n  α😀\r\n    猫\r\n  next\r\n");
        let view = doc.editor_view_identity();
        doc.apply_editor_options(
            &view,
            &patch(json!({"tabSize":8,"indentSize":6,"insertSpaces":false})),
        )
        .unwrap();
        doc.apply_editor_options(&view, &patch(json!({"tabSize":"auto","indentSize":7})))
            .unwrap();
        let options = doc.editor_options_for(0).unwrap();
        assert_eq!(options["tabSize"], 2);
        assert_eq!(options["indentSize"], 2);
        assert_eq!(options["originalIndentSize"], 2);
        assert_eq!(options["insertSpaces"], true);
        let mut tabs = Document::from_text("root\r\n\tα😀\r\n\t\t猫\r\n");
        let view = tabs.editor_view_identity();
        tabs.apply_editor_options(&view, &patch(json!({"insertSpaces":"auto","tabSize":8})))
            .unwrap();
        assert_eq!(tabs.tab_size, 8);
        assert!(!tabs.insert_spaces);
        let mut large = Document::from_text(&"x".repeat(256 * 1024 + 1));
        large.insert("!", false);
        large.undo();
        let view = large.editor_view_identity();
        let before = (
            source(&large),
            large.editor_options_for(0),
            large.selections(),
        );
        let error = large
            .apply_editor_options(&view, &patch(json!({"tabSize":"auto","lineNumbers":0})))
            .unwrap_err();
        assert!(error.to_string().contains("256 KiB"));
        assert_eq!(
            (
                source(&large),
                large.editor_options_for(0),
                large.selections()
            ),
            before
        );
        large.redo();
        assert_eq!(large.text.char(0), '!');
    }
    #[test]
    fn invalid_or_retired_option_patch_is_atomic_and_readonly_metadata_is_allowed() {
        let mut doc = Document::from_virtual("fixture:/readonly".into(), "α😀\r\n猫").unwrap();
        doc.activate_view(1);
        let old = doc.editor_view_identity();
        let before = (source(&doc), doc.editor_options_for(1), doc.selections());
        for value in [
            json!({"tabSize":17,"cursorStyle":2}),
            json!({"indentSize":0,"lineNumbers":0}),
            json!({"tabSize":2,"cursorStyle":7}),
            json!({"lineNumbers":4,"insertSpaces":false}),
        ] {
            assert!(doc.apply_editor_options(&old, &patch(value)).is_err());
            assert_eq!(
                (source(&doc), doc.editor_options_for(1), doc.selections()),
                before
            );
        }
        doc.activate_view(2);
        doc.remove_view(1);
        doc.display_view(1);
        assert!(
            doc.apply_editor_options(&old, &patch(json!({"tabSize":2})))
                .is_err()
        );
        let current = doc.editor_view_identity();
        doc.apply_editor_options(
            &current,
            &patch(json!({"tabSize":2,"lineNumbers":3,"cursorStyle":6})),
        )
        .unwrap();
        assert_eq!(doc.tab_size, 2);
        assert!(doc.is_read_only());
        assert_eq!(source(&doc), before.0);
        assert_eq!(doc.editor_options_for(1).unwrap()["lineNumbers"], 3);
        for malformed in [
            json!({"tabSize":"other"}),
            json!({"indentSize":"auto"}),
            json!({"extra":true}),
        ] {
            assert!(serde_json::from_value::<EditorOptionsPatch>(malformed).is_err());
        }
    }
    #[test]
    fn inactive_reveal_preserves_private_selections_and_follow_resumes_on_real_caret_aba() {
        let mut doc = Document::from_text("α😀\r\n\t猫\r\nthird\r\nfourth");
        doc.activate_view(1);
        doc.insert("!", false);
        doc.undo();
        doc.set_selections(vec![Selection {
            cursor: 1,
            anchor: Some(2),
            desired_column: Some(11),
        }]);
        let view = doc.editor_view_identity();
        let selection = doc.selections();
        doc.activate_view(2);
        doc.move_to(5, false);
        let sibling = doc.selections();
        let before = source(&doc);
        doc.reveal_editor_view(&view, 3, 10).unwrap();
        assert_eq!(doc.selections(), sibling);
        assert_eq!(doc.editor_view_stamp().0, 2);
        doc.display_view(1);
        assert_eq!(doc.selections(), selection);
        assert_eq!((doc.top, doc.left), (3, 10));
        assert!(!doc.editor_cursor_following());
        doc.display_view(2);
        doc.display_view(1);
        assert!(!doc.editor_cursor_following());
        let caret = doc.cursor;
        doc.move_to(0, false);
        doc.move_to(caret, false);
        assert!(doc.editor_cursor_following());
        assert_eq!(source(&doc), before);
        doc.redo();
        assert_eq!(doc.text.to_string(), format!("!{}", before.1));
        doc.undo();
        assert_eq!(doc.text.to_string(), before.1);
        doc.activate_view(2);
        doc.remove_view(1);
        doc.display_view(1);
        let before = (doc.top, doc.left, doc.selections());
        assert!(doc.reveal_editor_view(&view, 0, 0).is_err());
        assert_eq!((doc.top, doc.left, doc.selections()), before);
    }
}
