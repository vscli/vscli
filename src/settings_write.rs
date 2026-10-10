//! Pure, bounded root-setting patches. Filesystem persistence belongs to a worker.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{io::Write, ops::Range};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_KEY_BYTES: usize = 1024;

struct Root {
    target: Option<Range<usize>>,
    first: Option<(usize, String)>,
    last: Option<(usize, bool)>,
    close: usize,
}

/// Replace one scalar root value, or insert a new scalar root property.
/// Existing composite target values and duplicate target keys are rejected.
/// Every original byte outside the replaced value is retained. Insertion adds
/// a separating comma before trailing comments, leaving those comments intact.
pub fn patch(input: &[u8], key: &str, value: &Value) -> Result<Vec<u8>> {
    ensure!(input.len() <= MAX_BYTES, "Settings exceed 1 MiB");
    ensure!(
        !key.is_empty() && key.len() <= MAX_KEY_BYTES,
        "Setting key must contain 1–1024 bytes"
    );
    ensure!(
        !value.is_array() && !value.is_object(),
        "Setting patches require a scalar value"
    );
    let text = std::str::from_utf8(input).context("Settings must contain UTF-8")?;
    let parsed: Value = crate::jsonc::parse(text).context("Invalid settings JSON5")?;
    let object = parsed
        .as_object()
        .context("Settings must be a root object")?;
    let encoded = encode(value)?;
    let root = properties(text, key)?;
    let close = root.close;
    let mut edits = Vec::<(usize, usize, Vec<u8>)>::new();
    if let Some(target) = root.target {
        let current = object
            .get(key)
            .context("Setting key span did not match parsed input")?;
        ensure!(
            !current.is_array() && !current.is_object(),
            "Existing setting target must be scalar"
        );
        edits.push((target.start, target.end, encoded));
    } else {
        if let Some((end, false)) = root.last {
            edits.push((end, end, b",".to_vec()));
        }
        let quoted = serde_json::to_string(key)?;
        let separator = root
            .first
            .as_ref()
            .map_or(": ", |(_, separator)| separator.as_str());
        let trailing_comma = root.last.is_some_and(|(_, comma)| comma);
        let mut addition = Vec::new();
        let multiline = line_prefix(text, close);
        let insertion = if let Some((start, closing_indent)) = multiline {
            let indent = root
                .first
                .as_ref()
                .and_then(|(key_start, _)| line_prefix(text, *key_start))
                .map(|(_, indent)| indent.to_owned())
                .unwrap_or_else(|| format!("{closing_indent}  "));
            addition.extend_from_slice(indent.as_bytes());
            start
        } else {
            // Inline comments can end immediately before the closing brace;
            // one space keeps the inserted property separated from that trivia.
            if close > 0 && !input[close - 1].is_ascii_whitespace() && input[close - 1] != b'{' {
                addition.push(b' ');
            }
            close
        };
        addition.extend_from_slice(quoted.as_bytes());
        addition.extend_from_slice(separator.as_bytes());
        addition.extend_from_slice(&encoded);
        if trailing_comma {
            addition.push(b',');
        }
        if multiline.is_some() {
            addition.extend_from_slice(line_ending(text).as_bytes());
        }
        edits.push((insertion, insertion, addition));
    }
    edits.sort_by_key(|edit| edit.0);
    let removed = edits
        .iter()
        .map(|(start, end, _)| end - start)
        .sum::<usize>();
    let added = edits.iter().map(|(_, _, bytes)| bytes.len()).sum::<usize>();
    let size = input
        .len()
        .checked_sub(removed)
        .and_then(|size| size.checked_add(added))
        .context("Settings patch size overflow")?;
    ensure!(size <= MAX_BYTES, "Patched settings exceed 1 MiB");
    let mut output = Vec::with_capacity(size);
    let mut previous = 0;
    for (start, end, replacement) in edits {
        ensure!(
            previous <= start && start <= end && end <= input.len(),
            "Overlapping setting spans"
        );
        output.extend_from_slice(&input[previous..start]);
        output.extend_from_slice(&replacement);
        previous = end;
    }
    output.extend_from_slice(&input[previous..]);
    let checked: Value = crate::jsonc::parse(std::str::from_utf8(&output)?)
        .context("Patched settings are invalid JSON5")?;
    ensure!(
        checked.get(key) == Some(value),
        "Patched setting does not match requested value"
    );
    Ok(output)
}

fn encode(value: &Value) -> Result<Vec<u8>> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("Scalar setting exceeds 1 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut result = Bounded(Vec::new());
    serde_json::to_writer(&mut result, value)?;
    Ok(result.0)
}

fn properties(text: &str, target: &str) -> Result<Root> {
    let bytes = text.as_bytes();
    let mut index = trivia(text, 0)?;
    ensure!(
        bytes.get(index) == Some(&b'{'),
        "Settings must be a root object"
    );
    index += 1;
    let mut result = Root {
        target: None,
        first: None,
        last: None,
        close: 0,
    };
    loop {
        index = trivia(text, index)?;
        if bytes.get(index) == Some(&b'}') {
            ensure!(
                trivia(text, index + 1)? == bytes.len(),
                "Unexpected settings suffix"
            );
            result.close = index;
            return Ok(result);
        }
        let key_start = index;
        if matches!(bytes.get(index), Some(b'\'' | b'"')) {
            index = string_end(bytes, index)?;
        } else {
            while index < bytes.len() && bytes[index] != b':' && !is_trivia(text, index) {
                index += text[index..]
                    .chars()
                    .next()
                    .context("Missing setting key")?
                    .len_utf8();
            }
        }
        ensure!(index > key_start, "Missing setting key");
        let raw = &text[key_start..index];
        let decoded: Value =
            crate::jsonc::parse(&format!("{{{raw}:null}}")).context("Invalid root setting key")?;
        let key = decoded
            .as_object()
            .and_then(|object| object.keys().next())
            .context("Missing decoded setting key")?;
        index = trivia(text, index)?;
        ensure!(
            bytes.get(index) == Some(&b':'),
            "Setting key requires a colon"
        );
        let colon = index;
        index = trivia(text, index + 1)?;
        let value_start = index;
        index = value_end(text, index)?;
        let value_end = index;
        index = trivia(text, index)?;
        let comma = if bytes.get(index) == Some(&b',') {
            let comma = index;
            index += 1;
            Some(comma)
        } else {
            None
        };
        ensure!(
            comma.is_some() || bytes.get(index) == Some(&b'}'),
            "Setting requires a separator"
        );
        if key == target {
            ensure!(
                result.target.is_none(),
                "Duplicate target setting keys are ambiguous"
            );
            result.target = Some(value_start..value_end);
        }
        if result.first.is_none() {
            let spacing = &text[colon + 1..value_start];
            let separator = if spacing.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
                format!(":{spacing}")
            } else {
                ": ".into()
            };
            result.first = Some((key_start, separator));
        }
        result.last = Some((value_end, comma.is_some()));
    }
}

fn value_end(text: &str, start: usize) -> Result<usize> {
    let bytes = text.as_bytes();
    if matches!(bytes.get(start), Some(b'\'' | b'"')) {
        return string_end(bytes, start);
    }
    let mut index = start;
    if matches!(bytes.get(index), Some(b'{' | b'[')) {
        let mut stack = Vec::new();
        loop {
            index = trivia(text, index)?;
            match bytes.get(index) {
                Some(b'\'' | b'"') => {
                    index = string_end(bytes, index)?;
                    continue;
                }
                Some(b'{' | b'[') => {
                    ensure!(stack.len() < 64, "Settings nesting exceeds 64 levels");
                    stack.push(bytes[index]);
                }
                Some(b'}' | b']') => {
                    let expected = if bytes[index] == b'}' { b'{' } else { b'[' };
                    ensure!(stack.pop() == Some(expected), "Unmatched setting delimiter");
                    if stack.is_empty() {
                        return Ok(index + 1);
                    }
                }
                Some(_) => (),
                None => bail!("Unclosed setting value"),
            }
            index += text[index..].chars().next().unwrap().len_utf8();
        }
    }
    while index < bytes.len()
        && !matches!(bytes[index], b',' | b'}' | b']')
        && !is_trivia(text, index)
    {
        index += text[index..].chars().next().unwrap().len_utf8();
    }
    ensure!(index > start, "Missing setting value");
    Ok(index)
}

fn string_end(bytes: &[u8], start: usize) -> Result<usize> {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == quote {
            return Ok(index + 1);
        }
        if bytes[index] == b'\\' {
            index += 1;
            ensure!(index < bytes.len(), "Unclosed setting escape");
            if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
        }
        index += 1;
    }
    bail!("Unclosed setting string")
}

fn is_trivia(text: &str, index: usize) -> bool {
    text[index..].starts_with("//")
        || text[index..].starts_with("/*")
        || text[index..]
            .chars()
            .next()
            .is_some_and(|character| character.is_whitespace() || character == '\u{feff}')
}

fn trivia(text: &str, mut index: usize) -> Result<usize> {
    while index < text.len() {
        let suffix = &text[index..];
        let character = suffix.chars().next().unwrap();
        if character.is_whitespace() || character == '\u{feff}' {
            index += character.len_utf8();
        } else if suffix.starts_with("//") {
            index += 2;
            while index < text.len() {
                let character = text[index..].chars().next().unwrap();
                if matches!(character, '\r' | '\n' | '\u{2028}' | '\u{2029}') {
                    break;
                }
                index += character.len_utf8();
            }
        } else if suffix.starts_with("/*") {
            index += suffix.find("*/").context("Unclosed settings comment")? + 2;
        } else {
            break;
        }
    }
    Ok(index)
}

fn line_prefix(text: &str, index: usize) -> Option<(usize, &str)> {
    let previous = text[..index].rfind(['\r', '\n'])?;
    let start = previous + 1;
    let indent = &text[start..index];
    indent
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some((start, indent))
}

fn line_ending(text: &str) -> &'static str {
    let Some(index) = text.find(['\r', '\n']) else {
        return "\n";
    };
    match text.as_bytes()[index] {
        b'\r' if text.as_bytes().get(index + 1) == Some(&b'\n') => "\r\n",
        b'\r' => "\r",
        _ => "\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replacing_a_root_scalar_preserves_every_other_unicode_crlf_byte() {
        let input = "{\r\n  // 猫🙂 says breadcrumbs.enabled is false\r\n  'breadcrumbs.enabled' /* key */ : /* before */ false /* after */,\r\n  '[cpp]': {'breadcrumbs.enabled': false,},\r\n  extension: { note: 'false } //', list: [false, {deep:true}], },\r\n}\r\n// trailing 界\r\n";
        let value_start = input.find("false /* after */").unwrap();
        let result = patch(input.as_bytes(), "breadcrumbs.enabled", &json!(true)).unwrap();
        assert_eq!(&result[..value_start], &input.as_bytes()[..value_start]);
        assert_eq!(&result[value_start..value_start + 4], b"true");
        assert_eq!(
            &result[value_start + 4..],
            &input.as_bytes()[value_start + 5..]
        );
        let value: Value = crate::jsonc::parse(std::str::from_utf8(&result).unwrap()).unwrap();
        assert_eq!(value["[cpp]"]["breadcrumbs.enabled"], false);
        assert_eq!(value["extension"]["list"], json!([false,{"deep":true}]));
    }

    #[test]
    fn scalar_literals_and_escaped_keys_are_located_without_matching_comments_or_nested_keys() {
        for literal in [
            "null",
            "true",
            "false",
            ".5",
            "+42",
            "0xF",
            "'escaped \\' quote and /* comment */'",
            "'line\\\r\ncontinuation'",
        ] {
            let input = format!(
                "{{ /* target: fake */ 'tar\\u0067et': {literal} /* keep */, nested: {{target: {literal}}}, }}"
            );
            let expected = format!(
                "{{ /* target: fake */ 'tar\\u0067et': \"猫🙂\" /* keep */, nested: {{target: {literal}}}, }}"
            );
            assert_eq!(
                patch(input.as_bytes(), "target", &json!("猫🙂")).unwrap(),
                expected.as_bytes(),
                "{literal}"
            );
        }
        let input = "{ target:false, // target:true }\u{2028} nested:{target:true}, text:'\\\"target\\\":true', }";
        let expected = "{ target:null, // target:true }\u{2028} nested:{target:true}, text:'\\\"target\\\":true', }";
        assert_eq!(
            patch(input.as_bytes(), "target", &Value::Null).unwrap(),
            expected.as_bytes()
        );
    }

    #[test]
    fn insertion_preserves_empty_root_comments_and_existing_crlf_indentation() {
        for (input, expected) in [
            ("{}", "{\"enabled\": true}"),
            (
                "{ /* keep */ } // after",
                "{ /* keep */ \"enabled\": true} // after",
            ),
            ("{\n}", "{\n  \"enabled\": true\n}"),
            (
                "{\r\n\t// empty 猫\r\n}",
                "{\r\n\t// empty 猫\r\n  \"enabled\": true\r\n}",
            ),
            (
                "{\n  value: 1 // keep last\n}\n",
                "{\n  value: 1, // keep last\n  \"enabled\": true\n}\n",
            ),
            (
                "{\r\n\tvalue:\t1, // trailing\r\n}\r\n",
                "{\r\n\tvalue:\t1, // trailing\r\n\t\"enabled\":\ttrue,\r\n}\r\n",
            ),
            (
                "{value:1 /* keep */}",
                "{value:1, /* keep */ \"enabled\":true}",
            ),
            (
                "{value:1,/* keep */}",
                "{value:1,/* keep */ \"enabled\":true,}",
            ),
        ] {
            assert_eq!(
                patch(input.as_bytes(), "enabled", &json!(true)).unwrap(),
                expected.as_bytes(),
                "{input:?}"
            );
        }
    }

    #[test]
    fn insertion_does_not_rewrite_existing_language_or_extension_properties() {
        let input =
            b"{ '[cpp]': {target: false}, targetText:'target:true', extension:{'target':null}, }";
        let result = patch(input, "target", &json!(false)).unwrap();
        assert_eq!(&result[..input.len() - 1], &input[..input.len() - 1]);
        assert_eq!(&result[input.len() - 1..], b"\"target\": false,}");
        let parsed: Value = crate::jsonc::parse(std::str::from_utf8(&result).unwrap()).unwrap();
        assert_eq!(parsed["[cpp]"]["target"], false);
        assert_eq!(parsed["extension"]["target"], Value::Null);
        assert_eq!(parsed["targetText"], "target:true");
        let key = "quoted \"猫\"\\key";
        let added = patch(b"{}", key, &json!("escaped\n🙂")).unwrap();
        let replaced = patch(&added, key, &json!(false)).unwrap();
        let parsed: Value = crate::jsonc::parse(std::str::from_utf8(&replaced).unwrap()).unwrap();
        assert_eq!(parsed[key], false);
    }

    #[test]
    fn ambiguous_duplicate_root_targets_and_composite_targets_are_rejected() {
        for input in [
            r#"{target:1,'target':2}"#,
            r#"{'tar\u0067et':1,target:2}"#,
            "{'tar\\\r\nget':1,target:2}",
        ] {
            assert!(
                patch(input.as_bytes(), "target", &json!(true))
                    .unwrap_err()
                    .to_string()
                    .contains("Duplicate"),
                "{input:?}"
            );
        }
        for input in [b"{target:{child:1}}".as_slice(), b"{target:[1,2]}"] {
            assert!(
                patch(input, "target", &json!(true))
                    .unwrap_err()
                    .to_string()
                    .contains("scalar")
            );
        }
        // Ambiguity belongs to the selected root target, not an unrelated field.
        assert_eq!(
            patch(b"{other:1,other:2,target:false}", "target", &json!(true)).unwrap(),
            b"{other:1,other:2,target:true}"
        );
    }

    #[test]
    fn malformed_non_object_and_non_scalar_updates_leave_input_untouched() {
        for input in [
            "{target:}",
            "{target:1",
            "{target:1}garbage",
            "{target:'unclosed}",
            "{target:/* missing}",
            "[]",
            "null",
        ] {
            let original = input.as_bytes().to_vec();
            assert!(patch(&original, "target", &json!(true)).is_err());
            assert_eq!(original, input.as_bytes());
        }
        for value in [json!([]), json!({"nested":true})] {
            assert!(patch(b"{}", "target", &value).is_err());
        }
        assert!(patch(&[b'{', 0xff, b'}'], "target", &json!(true)).is_err());
        assert!(patch(b"{}", "", &json!(true)).is_err());
        assert!(patch(b"{}", &"x".repeat(MAX_KEY_BYTES + 1), &json!(true)).is_err());
    }

    #[test]
    fn input_output_escaped_scalar_and_nested_work_are_bounded() {
        let (prefix, suffix) = (b"{target:false/*", b"*/}");
        let mut input = prefix.to_vec();
        input.resize(MAX_BYTES - suffix.len(), b' ');
        input.extend_from_slice(suffix);
        assert_eq!(input.len(), MAX_BYTES);
        let output = patch(&input, "target", &json!(true)).unwrap();
        assert_eq!(output.len(), MAX_BYTES - 1);
        input.push(b' ');
        assert!(patch(&input, "target", &json!(true)).is_err());
        assert!(patch(b"{}", "target", &json!("x".repeat(MAX_BYTES - 2))).is_err());
        assert!(patch(b"{}", "target", &json!("\0".repeat(MAX_BYTES / 6 + 1))).is_err());
        let boundary = format!(
            "{{target:false,deep:{}0{}}}",
            "[".repeat(63),
            "]".repeat(63)
        );
        assert!(patch(boundary.as_bytes(), "target", &json!(true)).is_ok());
        let too_deep = format!(
            "{{target:false,deep:{}0{}}}",
            "[".repeat(64),
            "]".repeat(64)
        );
        assert!(patch(too_deep.as_bytes(), "target", &json!(true)).is_err());
    }
}
