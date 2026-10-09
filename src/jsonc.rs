//! Bound nesting before the recursive JSON5 parser sees configuration input.
use anyhow::{Result, bail};
use serde::de::DeserializeOwned;

const MAX_DEPTH: usize = 64;

pub fn parse<T: DeserializeOwned>(text: &str) -> Result<T> {
    check_depth(text)?;
    Ok(json5::from_str(text)?)
}

fn check_depth(text: &str) -> Result<()> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            quote @ (b'\'' | b'"') => {
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == quote {
                        index += 1;
                        break;
                    } else {
                        index += 1;
                    }
                }
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                // JSON5 also ends single-line comments at Unicode line/paragraph separators.
                while index < bytes.len()
                    && !matches!(bytes[index], b'\n' | b'\r')
                    && !bytes[index..].starts_with(&[0xe2, 0x80, 0xa8])
                    && !bytes[index..].starts_with(&[0xe2, 0x80, 0xa9])
                {
                    index += 1;
                }
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < bytes.len() && !bytes[index..].starts_with(b"*/") {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    bail!("Configuration nesting exceeds {MAX_DEPTH} levels");
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }
    // Syntax validation (including mismatched or unclosed delimiters) belongs to JSON5.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn nesting_is_bounded_before_recursive_parsing() {
        let at_limit = format!("{}0{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse::<Value>(&at_limit).is_ok());
        let excessive = format!("{}0{}", "[".repeat(50_000), "]".repeat(50_000));
        assert!(
            parse::<Value>(&excessive)
                .unwrap_err()
                .to_string()
                .contains("nesting")
        );
        let objects = format!(
            "{}0{}",
            "{a:".repeat(MAX_DEPTH + 1),
            "}".repeat(MAX_DEPTH + 1)
        );
        assert!(
            parse::<Value>(&objects)
                .unwrap_err()
                .to_string()
                .contains("nesting")
        );
    }

    #[test]
    fn quotes_escapes_and_comments_do_not_count_as_structure() {
        let text = format!(
            "/* {} */ {{ body: 'escaped \\' and {}', other: \"escaped \\\" [\", // {}\n value: 1,}}",
            "[".repeat(1000),
            "{".repeat(1000),
            "{".repeat(1000),
        );
        let parsed: Value = parse(&text).unwrap();
        assert_eq!(parsed["value"], 1);
        for end in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
            let input = format!("// comment{end}{}0{}", "[".repeat(65), "]".repeat(65));
            assert!(
                parse::<Value>(&input)
                    .unwrap_err()
                    .to_string()
                    .contains("nesting")
            );
        }
        assert!(parse::<Value>("{broken: [}").is_err());
        assert!(parse::<Value>("{quote:'unterminated}").is_err());
    }
}
