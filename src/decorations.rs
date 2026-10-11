//! Bounded native decoration DTOs and terminal style projection.
use anyhow::{Result, ensure};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) const MAX_SET: usize = 4096;
pub(crate) const MAX_MODEL: usize = 16384;
pub(crate) const MAX_MODEL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct DecorationRenderOptions {
    pub raw: Value,
    pub range_behavior: u8,
}
impl DecorationRenderOptions {
    pub(crate) fn from_wire(raw: Value) -> Result<Self> {
        ensure!(raw.is_object(), "Decoration options must be an object");
        json_budget(&raw, 16 * 1024)?;
        validate_options(&raw, 0)?;
        let behavior = raw
            .get("rangeBehavior")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        ensure!(behavior <= 3, "Invalid decoration rangeBehavior");
        Ok(Self {
            raw,
            range_behavior: behavior as u8,
        })
    }
    pub(crate) fn style(&self, theme: &crate::theme::Theme) -> Style {
        let mut style = style_fields(&self.raw, theme);
        let variant = if is_light(theme) { "light" } else { "dark" };
        if let Some(value) = self.raw.get(variant) {
            style = style.patch(style_fields(value, theme));
        }
        style
    }
    pub(crate) fn whole_line(&self) -> bool {
        self.raw
            .get("isWholeLine")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}
#[derive(Debug)]
pub(crate) struct DecorationType {
    pub key: String,
    pub owner: String,
    pub options: DecorationRenderOptions,
    pub live: Arc<AtomicBool>,
}
impl DecorationType {
    pub(crate) fn new(key: String, owner: String, options: Value) -> Result<Arc<Self>> {
        ensure!(
            !key.is_empty() && key.len() <= 128 && key.is_ascii() && !key.contains('\0'),
            "Invalid decoration key"
        );
        ensure!(
            !owner.is_empty() && owner.len() <= 512 && !owner.contains('\0'),
            "Invalid decoration owner"
        );
        Ok(Arc::new(Self {
            key,
            owner,
            options: DecorationRenderOptions::from_wire(options)?,
            live: Arc::new(AtomicBool::new(true)),
        }))
    }
    pub(crate) fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DecorationInput {
    pub range: crate::lsp::Range,
    #[serde(default)]
    pub hover_message: Option<Value>,
    #[serde(default)]
    pub render_options: Option<Value>,
}

/// Validate before cloning/serializing caller-controlled nested render/hover data.
fn escaped_bytes(s: &str) -> usize {
    2 + s
        .chars()
        .map(|c| match c {
            '"' | '\\' => 2,
            '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if c <= '\u{1f}' => 6,
            c => c.len_utf8(),
        })
        .sum::<usize>()
}
pub(crate) fn json_budget(value: &Value, maximum: usize) -> Result<usize> {
    fn visit(
        value: &Value,
        depth: usize,
        nodes: &mut usize,
        bytes: &mut usize,
        max: usize,
    ) -> Result<()> {
        *nodes += 1;
        *bytes = bytes.saturating_add(8);
        ensure!(
            depth <= 12 && *nodes <= 4096 && *bytes <= max,
            "Decoration JSON exceeds bounds"
        );
        match value {
            Value::String(s) => {
                *bytes = bytes.saturating_add(escaped_bytes(s));
                ensure!(!s.contains('\0'), "Decoration text contains NUL");
            }
            Value::Array(v) => {
                ensure!(v.len() <= 4096 - *nodes, "Decoration array exceeds bounds");
                for x in v {
                    visit(x, depth + 1, nodes, bytes, max)?;
                }
            }
            Value::Object(v) => {
                ensure!(v.len() <= 4096 - *nodes, "Decoration object exceeds bounds");
                for (k, x) in v {
                    *bytes = bytes.saturating_add(escaped_bytes(k));
                    visit(x, depth + 1, nodes, bytes, max)?;
                }
            }
            Value::Number(_) => {
                *bytes = bytes.saturating_add(32);
            }
            _ => {}
        }
        ensure!(*bytes <= max, "Decoration JSON exceeds byte budget");
        Ok(())
    }
    let (mut nodes, mut bytes) = (0, 0);
    visit(value, 0, &mut nodes, &mut bytes, maximum)?;
    Ok(bytes)
}
fn validate_options(value: &Value, depth: usize) -> Result<()> {
    ensure!(
        depth <= 4 && value.is_object(),
        "Invalid decoration render options"
    );
    for (key, v) in value.as_object().unwrap() {
        match key.as_str() {
            "color" | "backgroundColor" | "borderColor" | "outlineColor" | "overviewRulerColor" => {
                ensure!(
                    v.is_string()
                        || v.as_object().is_some_and(
                            |o| o.len() == 1 && o.get("id").is_some_and(Value::is_string)
                        ),
                    "Invalid decoration color"
                )
            }
            "fontStyle" | "fontWeight" | "textDecoration" | "border" | "borderRadius"
            | "borderSpacing" | "borderStyle" | "borderWidth" | "outline" | "outlineStyle"
            | "outlineWidth" | "cursor" | "letterSpacing" | "opacity" | "gutterIconSize" => {
                ensure!(v.is_string(), "Invalid decoration string option")
            }
            "isWholeLine" => ensure!(v.is_boolean(), "Invalid whole-line option"),
            "rangeBehavior" => ensure!(
                v.as_u64().is_some_and(|n| n <= 3),
                "Invalid decoration rangeBehavior"
            ),
            "light" | "dark" => validate_options(v, depth + 1)?,
            // Rich attachment, URI and ruler properties are retained as bounded
            // JSON for future rendering; they do not pretend to paint today.
            _ => {}
        }
    }
    Ok(())
}
fn is_light(theme: &crate::theme::Theme) -> bool {
    matches!(theme.colors.background, Color::Rgb(r,g,b) if u16::from(r)+u16::from(g)+u16::from(b) > 382)
}
fn color(value: &Value, theme: &crate::theme::Theme) -> Option<Color> {
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        return match id {
            "editor.foreground" => Some(theme.colors.foreground),
            "editor.background" => Some(theme.colors.background),
            "editor.selectionBackground" => Some(theme.colors.selection),
            "editor.lineHighlightBackground" => Some(theme.colors.current_line),
            "editorLineNumber.foreground" | "descriptionForeground" => Some(theme.colors.muted),
            "focusBorder" => Some(theme.colors.accent),
            _ => None,
        };
    }
    let s = value.as_str()?;
    if let Some(hex) = s.strip_prefix('#') {
        let rgb = match hex.len() {
            3 | 4 => {
                let mut c = hex.chars();
                let r = c.next()?.to_digit(16)? as u8;
                let g = c.next()?.to_digit(16)? as u8;
                let b = c.next()?.to_digit(16)? as u8;
                (r * 17, g * 17, b * 17)
            }
            6 | 8 if hex.is_ascii() => (
                u8::from_str_radix(&hex[..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ),
            _ => return None,
        };
        return Some(Color::Rgb(rgb.0, rgb.1, rgb.2));
    }
    match s {
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "blue" => Some(Color::Blue),
        "white" => Some(Color::White),
        "black" => Some(Color::Black),
        "yellow" => Some(Color::Yellow),
        _ => None,
    }
}
fn style_fields(value: &Value, theme: &crate::theme::Theme) -> Style {
    let mut s = Style::default();
    if let Some(c) = value.get("color").and_then(|v| color(v, theme)) {
        s = s.fg(c);
    }
    if let Some(c) = value.get("backgroundColor").and_then(|v| color(v, theme)) {
        s = s.bg(c);
    }
    for (key, on, modifier) in [
        ("fontWeight", "bold", Modifier::BOLD),
        ("fontStyle", "italic", Modifier::ITALIC),
    ] {
        if let Some(v) = value.get(key).and_then(Value::as_str) {
            if v == on {
                s = s.add_modifier(modifier);
            } else if v == "normal" {
                s = s.remove_modifier(modifier);
            }
        }
    }
    if let Some(v) = value.get("textDecoration").and_then(Value::as_str) {
        if v == "none" {
            s = s.remove_modifier(Modifier::UNDERLINED | Modifier::CROSSED_OUT);
        } else {
            if v.split_whitespace().any(|t| t == "underline") {
                s = s.add_modifier(Modifier::UNDERLINED);
            }
            if v.split_whitespace().any(|t| t == "line-through") {
                s = s.add_modifier(Modifier::CROSSED_OUT);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn variants_override_without_losing_preserved_rich_fields() {
        let o=DecorationRenderOptions::from_wire(json!({"color":"#123456","fontWeight":"bold","before":{"contentText":"rich"},"dark":{"color":"#ff0000","fontStyle":"italic"},"light":{"color":{"id":"editor.foreground"}}})).unwrap();
        assert_eq!(
            o.style(&crate::theme::Theme::default()).fg,
            Some(Color::Rgb(255, 0, 0))
        );
        assert!(
            o.style(&crate::theme::Theme::default())
                .add_modifier
                .contains(Modifier::BOLD | Modifier::ITALIC)
        );
        assert_eq!(
            o.style(&crate::theme::Theme::light()).fg,
            Some(crate::theme::Theme::light().colors.foreground)
        );
        assert_eq!(o.raw["before"]["contentText"], "rich");
        assert!(DecorationRenderOptions::from_wire(json!({"rangeBehavior":4})).is_err());
        assert!(DecorationRenderOptions::from_wire(json!({"color":true})).is_err());
        assert!(
            DecorationRenderOptions::from_wire(json!({"before":{"contentText":"x".repeat(16384)}}))
                .is_err()
        );
    }
}
