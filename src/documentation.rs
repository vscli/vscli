//! Bounded, inert terminal documentation projection. This is a Markdown subset,
//! not a browser renderer. Provider/source bytes and their kind remain retained.
use anyhow::{Context, Result, bail};
use serde_json::Value;

pub const MAX_HOVER_BYTES: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Plaintext,
    Markdown,
    Code,
}
#[derive(Clone, Debug)]
pub struct Part {
    pub kind: Kind,
    pub source: String,
    pub display: String,
}
#[derive(Clone, Debug, Default)]
pub struct Document {
    pub parts: Vec<Part>,
    pub display: String,
}
impl Part {
    pub fn parse(value: Option<&Value>, default: Kind, maximum: usize) -> Result<Self> {
        let (kind, source) = match value {
            None | Some(Value::Null) => (default, ""),
            Some(Value::String(source)) => (default, source.as_str()),
            Some(Value::Object(object)) => {
                for field in ["isTrusted", "supportHtml"] {
                    if object.get(field).is_some_and(|v| v != &Value::Bool(false)) {
                        bail!("Trusted or HTML documentation is not supported");
                    }
                }
                let kind = match object.get("kind").and_then(Value::as_str) {
                    Some("plaintext") => Kind::Plaintext,
                    Some("markdown") => Kind::Markdown,
                    None if object.get("kind").is_none() => default,
                    _ => bail!("Invalid documentation kind"),
                };
                (
                    kind,
                    object
                        .get("value")
                        .and_then(Value::as_str)
                        .context("Invalid documentation value")?,
                )
            }
            _ => bail!("Invalid documentation"),
        };
        Self::new(kind, source, maximum)
    }
    pub fn new(kind: Kind, source: &str, maximum: usize) -> Result<Self> {
        if maximum > MAX_HOVER_BYTES {
            bail!("Documentation limit exceeds native maximum");
        }
        if source.len() > maximum {
            bail!("Documentation exceeds its byte budget");
        }
        let display = if kind == Kind::Markdown {
            project(source, maximum)?
        } else {
            source.to_owned()
        };
        Ok(Self {
            kind,
            source: source.to_owned(),
            display,
        })
    }
}
impl Document {
    pub fn from_parts(parts: Vec<Part>, separator: &str, maximum: usize) -> Result<Self> {
        if maximum > MAX_HOVER_BYTES {
            bail!("Documentation limit exceeds native maximum");
        }
        if parts.len() > 32 {
            bail!("Documentation exceeds 32 parts");
        }
        let raw = parts.iter().try_fold(0usize, |n, p| {
            n.checked_add(p.source.len())
                .context("Documentation size overflow")
        })?;
        let projected = parts.iter().try_fold(0usize, |n, p| {
            n.checked_add(p.display.len())
                .context("Documentation size overflow")
        })?;
        let separators = parts
            .len()
            .saturating_sub(1)
            .checked_mul(separator.len())
            .context("Documentation size overflow")?;
        if raw > maximum
            || projected
                .checked_add(separators)
                .context("Documentation size overflow")?
                > maximum
        {
            bail!("Documentation exceeds aggregate byte budget");
        }
        let mut display = String::with_capacity(projected + separators);
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                display.push_str(separator);
            }
            display.push_str(&part.display);
        }
        Ok(Self { parts, display })
    }
    /// Legacy hover strings are Markdown; language-marked code is literal.
    /// LSP MarkupContent preserves its explicit Markdown/plaintext kind.
    pub fn hover(value: &Value) -> Result<Self> {
        fn part(value: &Value) -> Result<Part> {
            if let Some(object) = value.as_object()
                && let Some(language) = object.get("language")
            {
                let language = language.as_str().context("Invalid hover code language")?;
                if language.len() > 128 {
                    bail!("Hover code language exceeds 128 bytes");
                }
                for field in ["isTrusted", "supportHtml"] {
                    if object.get(field).is_some_and(|v| v != &Value::Bool(false)) {
                        bail!("Trusted or HTML documentation is not supported");
                    }
                }
                let source = object
                    .get("value")
                    .and_then(Value::as_str)
                    .context("Invalid hover code value")?;
                return Part::new(Kind::Code, source, MAX_HOVER_BYTES);
            }
            Part::parse(Some(value), Kind::Markdown, MAX_HOVER_BYTES)
        }
        let parts = match value {
            Value::Array(values) => {
                if values.len() > 32 {
                    bail!("Hover exceeds 32 contents");
                }
                let raw = values.iter().try_fold(0usize, |total, v| {
                    let text = v
                        .as_str()
                        .or_else(|| v.get("value").and_then(Value::as_str))
                        .context("Invalid hover content")?;
                    total.checked_add(text.len()).context("Hover size overflow")
                })?;
                if raw > MAX_HOVER_BYTES {
                    bail!("Hover exceeds aggregate byte budget");
                }
                values.iter().map(part).collect::<Result<Vec<_>>>()?
            }
            Value::Null => Vec::new(),
            value => vec![part(value)?],
        };
        Self::from_parts(parts, "\n\n", MAX_HOVER_BYTES)
    }
}
struct Output {
    text: String,
    maximum: usize,
    work: usize,
}
impl Output {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.work = self
            .work
            .checked_sub(bytes)
            .context("Markdown projection work budget exceeded")?;
        Ok(())
    }
    fn push(&mut self, text: &str) -> Result<()> {
        if text.len() > self.maximum.saturating_sub(self.text.len()) {
            bail!("Markdown projection byte budget exceeded");
        }
        self.text.push_str(text);
        Ok(())
    }
}
fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = *trimmed.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let count = trimmed.bytes().take_while(|b| *b == marker).count();
    (count >= 3).then(|| (marker, count, &trimmed[count..]))
}
fn entity(text: &str) -> Option<(usize, char)> {
    let end = text.as_bytes().iter().take(34).position(|b| *b == b';')?;
    let token = text.get(1..end)?;
    let value = match token {
        "nbsp" => ' ',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let number = if let Some(hex) = token
                .strip_prefix("#x")
                .or_else(|| token.strip_prefix("#X"))
            {
                if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                u32::from_str_radix(hex, 16).ok()?
            } else {
                let decimal = token.strip_prefix('#')?;
                if decimal.is_empty() || !decimal.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                decimal.parse().ok()?
            };
            // Never manufacture terminal control sequences from numeric entities.
            let character = char::from_u32(number)
                .filter(|c| !c.is_control())
                .unwrap_or('\u{fffd}');
            if character == '\u{a0}' {
                ' '
            } else {
                character
            }
        }
    };
    Some((end + 1, value))
}
fn closing_ticks(text: &str, count: usize, output: &mut Output) -> Result<Option<usize>> {
    let mut index = 0;
    while index < text.len() {
        let character = text[index..].chars().next().unwrap();
        if character == '`' {
            let run = text[index..].bytes().take_while(|b| *b == b'`').count();
            output.charge(run)?;
            if run == count {
                return Ok(Some(index));
            }
            index += run;
        } else {
            output.charge(character.len_utf8())?;
            index += character.len_utf8();
        }
    }
    Ok(None)
}
fn link_end(text: &str, output: &mut Output) -> Result<Option<(usize, usize)>> {
    let mut depth = 1usize;
    let mut index = 1;
    let mut label = None;
    while index < text.len() {
        let character = text[index..].chars().next().unwrap();
        output.charge(character.len_utf8())?;
        if character == '\\' {
            index += 1;
            if index < text.len() {
                let next = text[index..].chars().next().unwrap();
                output.charge(next.len_utf8())?;
                index += next.len_utf8();
            }
            continue;
        }
        match character {
            '[' if label.is_none() => {
                depth += 1;
                if depth > 16 {
                    bail!("Markdown link nesting exceeds 16");
                }
            }
            ']' if label.is_none() => {
                depth -= 1;
                if depth == 0 {
                    if !text[index + 1..].starts_with('(') {
                        return Ok(None);
                    }
                    label = Some(index);
                    depth = 1;
                    index += 1;
                }
            }
            '(' if label.is_some() => {
                depth += 1;
                if depth > 16 {
                    bail!("Markdown link nesting exceeds 16");
                }
            }
            ')' if label.is_some() => {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some((label.unwrap(), index + 1)));
                }
            }
            _ => {}
        }
        index += character.len_utf8();
    }
    Ok(None)
}
fn inline(text: &str, output: &mut Output, depth: usize) -> Result<()> {
    if depth > 16 {
        bail!("Markdown inline nesting exceeds 16");
    }
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let character = rest.chars().next().unwrap();
        output.charge(character.len_utf8())?;
        if character == '\\'
            && let Some(next) = rest[1..].chars().next()
            && next.is_ascii_punctuation()
        {
            output.push(&rest[1..1 + next.len_utf8()])?;
            index += 1 + next.len_utf8();
            continue;
        }
        if character == '&' {
            // Charge even unsuccessful bounded entity lookahead before searching.
            output.charge(rest.len().min(34))?;
            if let Some((bytes, decoded)) = entity(rest) {
                output.charge(bytes)?;
                output.push(decoded.encode_utf8(&mut [0; 4]))?;
                index += bytes;
                continue;
            }
        }
        if character == '`' {
            let count = rest.bytes().take_while(|b| *b == b'`').count();
            if let Some(end) = closing_ticks(&rest[count..], count, output)? {
                output.push(&rest[count..count + end])?;
                index += count + end + count;
                continue;
            }
            // Unmatched spans remain literal, without repeated suffix searches.
            output.push(rest)?;
            break;
        }
        let image = rest.starts_with("![");
        if character == '[' || image {
            let opening = usize::from(image);
            if let Some((label, end)) = link_end(&rest[opening..], output)? {
                inline(&rest[opening + 1..opening + label], output, depth + 1)?;
                index += opening + end;
                continue;
            }
        }
        if character == '*' || character == '_' || rest.starts_with("~~") {
            // Underscores inside words are literal; this is a terminal subset.
            let inside_word = character == '_'
                && index > 0
                && text[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
                && rest[1..].chars().next().is_some_and(char::is_alphanumeric);
            if !inside_word {
                let size = if rest.as_bytes().get(1) == Some(&rest.as_bytes()[0]) {
                    2
                } else {
                    1
                };
                if character != '~' || size == 2 {
                    let marker = &rest[..size];
                    // Charge the whole suffix before its bounded search.
                    output.charge(rest.len() - size)?;
                    if let Some(end) = rest[size..].find(marker)
                        && end > 0
                    {
                        inline(&rest[size..size + end], output, depth + 1)?;
                        index += size + end + size;
                        continue;
                    }
                }
            }
        }
        output.push(&rest[..character.len_utf8()])?;
        index += character.len_utf8();
    }
    Ok(())
}
fn project(source: &str, maximum: usize) -> Result<String> {
    let mut output = Output {
        text: String::with_capacity(source.len()),
        maximum,
        work: source
            .len()
            .checked_mul(8)
            .and_then(|n| n.checked_add(4096))
            .context("Markdown budget overflow")?,
    };
    let mut code_fence = None;
    let mut paragraph = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        output.charge(line.len())?;
        if let Some((marker, count)) = code_fence {
            if fence(line).is_some_and(|(m, n, tail)| {
                m == marker && n >= count && tail.trim_matches([' ', '\t', '\r', '\n']).is_empty()
            }) {
                code_fence = None;
            } else {
                output.push(line)?;
            }
            continue;
        }
        if let Some((marker, count, tail)) = fence(line)
            && (marker != b'`' || !tail.contains('`'))
        {
            inline(&paragraph, &mut output, 0)?;
            paragraph.clear();
            code_fence = Some((marker, count));
            continue;
        }
        // Conservatively preserve code-looking indented lines without decoding.
        if line.starts_with("    ") || line.starts_with('\t') {
            inline(&paragraph, &mut output, 0)?;
            paragraph.clear();
            output.push(line)?;
            continue;
        }
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
        let content = if indent <= 3
            && (1..=6).contains(&hashes)
            && trimmed
                .as_bytes()
                .get(hashes)
                .is_some_and(u8::is_ascii_whitespace)
        {
            trimmed[hashes..].trim_start_matches([' ', '\t'])
        } else if indent <= 3 && trimmed.starts_with("> ") {
            &trimmed[2..]
        } else {
            line
        };
        paragraph.push_str(content);
    }
    inline(&paragraph, &mut output, 0)?;
    Ok(output.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn plaintext_never_decodes_markdown_or_entities() {
        let source = "&nbsp; \\*literal\\* **literal** `code` [run](command:test)\r\n猫🙂";
        let part = Part::new(Kind::Plaintext, source, MAX_HOVER_BYTES).unwrap();
        assert_eq!(part.source, source);
        assert_eq!(part.display, source);
        assert_eq!(part.kind, Kind::Plaintext);
    }
    #[test]
    fn source_append_text_spaces_tabs_tokens_and_crlf_project_once() {
        let raw = "spaces&nbsp;&nbsp;tab&nbsp;tilde&nbsp;\\~x\\~\r\n\n\\-&nbsp;literal&nbsp;猫🙂";
        let part = Part::new(Kind::Markdown, raw, MAX_HOVER_BYTES).unwrap();
        assert_eq!(part.source, raw);
        assert_eq!(part.display, "spaces  tab tilde ~x~\r\n\n- literal 猫🙂");
    }
    #[test]
    fn fenced_and_inline_code_preserve_literal_entities_escapes_and_links() {
        let raw = "intro\n````cpp\n猫\t&nbsp; \\* [x](command:test)\r\n```inner\n````\n`&nbsp; \\* [run](command:x)`";
        let part = Part::new(Kind::Markdown, raw, MAX_HOVER_BYTES).unwrap();
        assert_eq!(
            part.display,
            "intro\n猫\t&nbsp; \\* [x](command:test)\r\n```inner\n&nbsp; \\* [run](command:x)"
        );
    }
    #[test]
    fn multiline_inline_code_and_unclosed_fences_remain_literal() {
        assert_eq!(
            Part::new(Kind::Markdown, "`first\n&nbsp; \\*`", 1024)
                .unwrap()
                .display,
            "first\n&nbsp; \\*"
        );
        assert_eq!(
            Part::new(Kind::Markdown, "```cpp\n&nbsp; \\*\n", 1024)
                .unwrap()
                .display,
            "&nbsp; \\*\n"
        );
    }
    #[test]
    fn indented_code_and_non_ascii_fence_suffix_do_not_decode_or_disappear() {
        assert_eq!(
            Part::new(
                Kind::Markdown,
                "    &nbsp; \\*
\t&amp;",
                1024
            )
            .unwrap()
            .display,
            "    &nbsp; \\*
\t&amp;"
        );
        assert_eq!(
            Part::new(
                Kind::Markdown,
                "```cpp
```\u{a0}
&nbsp;
```
",
                1024
            )
            .unwrap()
            .display,
            "```\u{a0}
&nbsp;
"
        );
    }
    #[test]
    fn link_destinations_and_formatting_are_inert_text() {
        let raw = "# Heading\n**bold** _em_ ~~gone~~ [run](command:danger?x=1) ![alt](https://example.invalid/image)";
        let part = Part::new(Kind::Markdown, raw, 1024).unwrap();
        assert_eq!(part.display, "Heading\nbold em gone run alt");
        assert!(!part.display.contains("command:"));
        assert!(!part.display.contains("https:"));
    }
    #[test]
    fn entities_do_not_manufacture_terminal_controls_or_decode_recursively() {
        let part = Part::new(
            Kind::Markdown,
            "&amp;nbsp; &#x1F642; &#27; &#xD800; &not-a-known-entity;",
            1024,
        )
        .unwrap();
        assert_eq!(part.display, "&nbsp; 🙂 � � &not-a-known-entity;");
    }
    #[test]
    fn mixed_hover_parts_preserve_each_kind_and_raw_bytes() {
        let doc=Document::hover(&json!(["**marked**",{"kind":"plaintext","value":"&nbsp; \\*"},{"language":"cpp","value":"猫\r\n&nbsp;"}])).unwrap();
        assert_eq!(
            doc.parts.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![Kind::Markdown, Kind::Plaintext, Kind::Code]
        );
        assert_eq!(doc.display, "marked\n\n&nbsp; \\*\n\n猫\r\n&nbsp;");
    }
    #[test]
    fn bad_metadata_and_hover_admission_fail_before_publication() {
        for value in [
            json!({"kind":"other","value":"x"}),
            json!({"kind":17,"value":"x"}),
            json!({"value":17}),
            json!({"value":"x","isTrusted":true}),
            json!({"value":"x","supportHtml":true}),
        ] {
            assert!(Part::parse(Some(&value), Kind::Markdown, 8192).is_err());
        }
        assert!(Document::hover(&json!([vec!["x"]])).is_err());
        assert!(Document::hover(&json!(vec!["x"; 33])).is_err());
        assert!(Document::hover(&json!(["x".repeat(40000), "x".repeat(40000)])).is_err());
        assert!(Part::new(Kind::Plaintext, &"x".repeat(8193), 8192).is_err());
    }
    #[test]
    fn hostile_suffix_and_recursive_input_have_bounded_work() {
        assert!(Part::new(Kind::Markdown, &"[".repeat(20000), MAX_HOVER_BYTES).is_err());
        let deep = format!("{}x{}", "[".repeat(17), "](x)".repeat(17));
        assert!(Part::new(Kind::Markdown, &deep, 1024).is_err());
        let unicode = "猫🙂".repeat(1000);
        assert_eq!(
            Part::new(Kind::Markdown, &unicode, 8192).unwrap().display,
            unicode
        );
    }
}
