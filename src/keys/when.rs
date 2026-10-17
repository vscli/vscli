//! Native context expressions following the pinned VS Code parser/evaluator.
//! Reference: microsoft/vscode 2a59476c9bfcb90b3ddc372c36762471b7dfad1c,
//! src/vs/platform/contextkey/common/{scanner,contextkey}.ts (MIT).
use super::*;
use regex::{Regex, RegexBuilder};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
const BYTES: usize = 8 * 1024;
const TOKENS: usize = 1024;
const DEPTH: usize = 64;
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Pattern(String, String),
    Not,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Match,
    L,
    R,
}
#[derive(Debug)]
pub(super) enum Expr {
    Constant(bool),
    Defined(String),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Equal(String, String, bool),
    Compare(String, f64, Comparison),
    Member(String, String, bool),
    Matches(String, Regex),
}
#[derive(Debug)]
pub(super) enum Comparison {
    Lt,
    Le,
    Gt,
    Ge,
}
fn truth(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(v)) => *v,
        Some(Value::Number(v)) => v.as_f64().is_some_and(|v| v != 0.0),
        Some(Value::String(v)) => !v.is_empty(),
        _ => true,
    }
}
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(v)) => v.clone(),
        Some(Value::Bool(v)) => v.to_string(),
        Some(Value::Number(v)) => {
            let value = v.as_f64().unwrap_or(0.0);
            if value == 0.0 {
                "0".into()
            } else {
                value.to_string()
            }
        }
        Some(Value::Object(_)) => "[object Object]".into(),
        Some(Value::Array(v)) => v
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_string(Some(v))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
    }
}
fn number(value: &str) -> Option<f64> {
    let value = value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if value.is_empty() {
        return Some(0.0);
    }
    if let Some(v) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u64::from_str_radix(v, 16).ok().map(|v| v as f64);
    }
    if let Some(v) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        return u64::from_str_radix(v, 2).ok().map(|v| v as f64);
    }
    if let Some(v) = value
        .strip_prefix("0o")
        .or_else(|| value.strip_prefix("0O"))
    {
        return u64::from_str_radix(v, 8).ok().map(|v| v as f64);
    }
    value.parse::<f64>().ok().filter(|v| !v.is_nan())
}
fn float_prefix(value: &str) -> f64 {
    let value = value.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    if value[index..].starts_with("Infinity") {
        return if bytes.first() == Some(&b'-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
    }
    if !bytes[start..index].iter().any(u8::is_ascii_digit) {
        return f64::NAN;
    }
    let exponent = index;
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let digits = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == digits {
            index = exponent;
        }
    }
    value[..index].parse().unwrap_or(f64::NAN)
}
fn loose_equal(value: Option<&Value>, right: &str) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(value)) => value == right,
        Some(Value::Number(value)) => {
            number(right).is_some_and(|right| value.as_f64() == Some(right))
        }
        Some(Value::Bool(value)) => number(right) == Some(if *value { 1.0 } else { 0.0 }),
        Some(_) => js_string(value) == right,
    }
}
impl Expr {
    pub(super) fn evaluate(&self, context: &HashMap<String, Value>) -> bool {
        match self {
            Self::Constant(v) => *v,
            Self::Defined(key) => truth(context.get(key)),
            Self::Not(v) => !v.evaluate(context),
            Self::And(v) => v.iter().all(|v| v.evaluate(context)),
            Self::Or(v) => v.iter().any(|v| v.evaluate(context)),
            Self::Equal(key, right, equal) => loose_equal(context.get(key), right) == *equal,
            Self::Compare(key, right, op) => {
                let left = float_prefix(&js_string(context.get(key)));
                match op {
                    Comparison::Lt => left < *right,
                    Comparison::Le => left <= *right,
                    Comparison::Gt => left > *right,
                    Comparison::Ge => left >= *right,
                }
            }
            Self::Matches(key, regex) => regex.is_match(&js_string(context.get(key))),
            Self::Member(key, source, positive) => {
                let item = context.get(key);
                let found = match context.get(source) {
                    Some(Value::Array(values)) => item.is_some_and(|item| {
                        values.iter().any(|v| strict_equal(v, item))
                            || (cfg!(windows)
                                && item.as_str().is_some_and(|item| {
                                    item.starts_with("file:///")
                                        && values.iter().any(|v| {
                                            v.as_str().is_some_and(|v| {
                                                v.to_lowercase() == item.to_lowercase()
                                            })
                                        })
                                }))
                    }),
                    Some(Value::Object(values)) => {
                        item.and_then(Value::as_str).is_some_and(|item| {
                            values.contains_key(item)
                                || (cfg!(windows)
                                    && item.starts_with("file:///")
                                    && values
                                        .keys()
                                        .any(|key| key.to_lowercase() == item.to_lowercase()))
                        })
                    }
                    _ => false,
                };
                found == *positive
            }
        }
    }
}
fn strict_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(_) | Value::Object(_), _) | (_, Value::Array(_) | Value::Object(_)) => false,
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => a == b,
    }
}
fn bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || c.is_alphabetic() || "_<>-./\\:*?+[]^,#@;\"%$".contains(c)
}
fn lex(source: &str) -> Result<Vec<Token>> {
    if source.len() > BYTES {
        bail!("When expression exceeds 8 KiB");
    }
    let chars: Vec<_> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if out.len() >= TOKENS {
            bail!("When expression exceeds 1024 tokens");
        }
        let c = chars[i];
        i += 1;
        out.push(match c {
            '(' => Token::L,
            ')' => Token::R,
            '!' => {
                if chars.get(i) == Some(&'=') {
                    i += 1;
                    if chars.get(i) == Some(&'=') {
                        i += 1;
                    }
                    Token::Ne
                } else {
                    Token::Not
                }
            }
            '=' => {
                if chars.get(i) == Some(&'=') {
                    i += 1;
                    if chars.get(i) == Some(&'=') {
                        i += 1;
                    }
                    Token::Eq
                } else if chars.get(i) == Some(&'~') {
                    i += 1;
                    Token::Match
                } else {
                    bail!("Expected == or =~")
                }
            }
            '<' => {
                if chars.get(i) == Some(&'=') {
                    i += 1;
                    Token::Le
                } else {
                    Token::Lt
                }
            }
            '>' => {
                if chars.get(i) == Some(&'=') {
                    i += 1;
                    Token::Ge
                } else {
                    Token::Gt
                }
            }
            '&' | '|' => {
                if chars.get(i) != Some(&c) {
                    bail!("Expected && or ||");
                }
                i += 1;
                if c == '&' { Token::And } else { Token::Or }
            }
            '\'' => {
                let start = i;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                if i == chars.len() {
                    bail!("Unclosed string");
                }
                let value = chars[start..i].iter().collect();
                i += 1;
                Token::Quoted(value)
            }
            '/' => {
                let start = i;
                let mut escaped = false;
                let mut class = false;
                while i < chars.len() {
                    let current = chars[i];
                    if !escaped {
                        if current == '/' && !class {
                            break;
                        }
                        if current == '[' {
                            class = true;
                        }
                        if current == ']' {
                            class = false;
                        }
                    }
                    escaped = current == '\\' && !escaped;
                    i += 1;
                }
                if i == chars.len() {
                    bail!("Unclosed regular expression");
                }
                let pattern = chars[start..i].iter().collect();
                i += 1;
                let start = i;
                while chars.get(i).is_some_and(char::is_ascii_alphabetic) {
                    i += 1;
                }
                Token::Pattern(pattern, chars[start..i].iter().collect())
            }
            c if bare(c) => {
                let start = i - 1;
                while chars.get(i).is_some_and(|c| bare(*c)) {
                    i += 1;
                }
                Token::Word(chars[start..i].iter().collect())
            }
            _ => bail!("Invalid when-expression character"),
        });
    }
    Ok(out)
}
fn regular(pattern: &str, flags: &str) -> Result<Regex> {
    let mut insensitive = false;
    let mut multiline = false;
    let mut dot = false;
    let mut used = std::collections::HashSet::new();
    for flag in flags.chars() {
        if matches!(flag, 'g' | 'y') {
            continue;
        }
        if !used.insert(flag) {
            bail!("Duplicate regular-expression flag");
        }
        match flag {
            'i' => insensitive = true,
            'm' => multiline = true,
            's' => dot = true,
            'u' | 'd' => {}
            _ => bail!("Regular-expression flag is not supported by the native engine"),
        }
    }
    // JavaScript's digit/word shorthands are ASCII; Rust's defaults are Unicode.
    let mut adapted = String::new();
    let mut chars = pattern.chars();
    let mut class = false;
    while let Some(c) = chars.next() {
        if c == '[' {
            class = true;
        } else if c == ']' {
            class = false;
        }
        if c == '\\' {
            let next = chars.next().context("Trailing regular-expression escape")?;
            match next {
                'd' => adapted.push_str("[0-9]"),
                'D' => adapted.push_str("[^0-9]"),
                'w' => adapted.push_str("[A-Za-z0-9_]"),
                'W' => adapted.push_str("[^A-Za-z0-9_]"),
                'b' if !class => adapted.push_str("(?-u:\\b)"),
                'B' if !class => adapted.push_str("(?-u:\\B)"),
                'b' => adapted.push_str("\\x08"),
                _ => {
                    adapted.push('\\');
                    adapted.push(next);
                }
            }
        } else {
            adapted.push(c);
        }
    }
    RegexBuilder::new(&adapted)
        .case_insensitive(insensitive)
        .multi_line(multiline)
        .dot_matches_new_line(dot)
        .size_limit(64 * 1024)
        .dfa_size_limit(64 * 1024)
        .build()
        .context("Unsupported native regular expression (lookaround/backreferences remain open)")
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}
impl Parser {
    fn or(&mut self) -> Result<Expr> {
        let mut values = vec![self.and()?];
        while self.peek() == Some(&Token::Or) {
            self.at += 1;
            values.push(self.and()?);
        }
        Ok(if values.len() == 1 {
            values.pop().unwrap()
        } else {
            Expr::Or(values)
        })
    }
    fn and(&mut self) -> Result<Expr> {
        let mut values = vec![self.term()?];
        while self.peek() == Some(&Token::And) {
            self.at += 1;
            values.push(self.term()?);
        }
        Ok(if values.len() == 1 {
            values.pop().unwrap()
        } else {
            Expr::And(values)
        })
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }
    fn group(&mut self) -> Result<Expr> {
        if self.depth >= DEPTH {
            bail!("When expression exceeds 64 nested groups or negations");
        }
        self.depth += 1;
        let result = self.or()?;
        self.depth -= 1;
        if self.peek() != Some(&Token::R) {
            bail!("Missing )");
        }
        self.at += 1;
        Ok(result)
    }
    fn term(&mut self) -> Result<Expr> {
        let token = self.peek().cloned().context("Expected context key")?;
        self.at += 1;
        match token {
            Token::Not => {
                let token = self.peek().cloned().context("Expected key after !")?;
                self.at += 1;
                let expr = match token {
                    Token::L => self.group()?,
                    Token::Word(key) if key == "true" => Expr::Constant(true),
                    Token::Word(key) if key == "false" => Expr::Constant(false),
                    Token::Word(key) if key != "in" && key != "not" => Expr::Defined(key),
                    _ => bail!("Expected key, boolean or parenthesized expression after !"),
                };
                Ok(Expr::Not(Box::new(expr)))
            }
            Token::L => self.group(),
            Token::Word(key) if key == "true" => Ok(Expr::Constant(true)),
            Token::Word(key) if key == "false" => Ok(Expr::Constant(false)),
            Token::Word(key) if key != "in" && key != "not" => {
                let operator = self.peek().cloned();
                if operator == Some(Token::Match) {
                    self.at += 1;
                    let Some(Token::Pattern(pattern, flags)) = self.peek().cloned() else {
                        bail!("Expected /regular expression/ after =~");
                    };
                    self.at += 1;
                    return Ok(Expr::Matches(key, regular(&pattern, &flags)?));
                }
                if matches!(&operator,Some(Token::Word(word))if word=="in"||word=="not") {
                    self.at += 1;
                    let positive = matches!(&operator,Some(Token::Word(word))if word=="in");
                    if !positive {
                        if self.peek() != Some(&Token::Word("in".into())) {
                            bail!("Expected in after not");
                        }
                        self.at += 1;
                    }
                    let (source, _) = self.value();
                    return Ok(Expr::Member(key, source, positive));
                }
                if matches!(
                    operator,
                    Some(Token::Eq | Token::Ne | Token::Lt | Token::Le | Token::Gt | Token::Ge)
                ) {
                    self.at += 1;
                    let (right, quoted) = self.value();
                    let operator = operator.unwrap();
                    if matches!(operator, Token::Eq | Token::Ne) {
                        let equal = operator == Token::Eq;
                        if !quoted && matches!(right.as_str(), "true" | "false") {
                            let positive = (right == "true") == equal;
                            let defined = Expr::Defined(key);
                            return Ok(if positive {
                                defined
                            } else {
                                Expr::Not(Box::new(defined))
                            });
                        }
                        return Ok(Expr::Equal(key, right, equal));
                    }
                    let value = float_prefix(&right);
                    if value.is_nan() {
                        return Ok(Expr::Constant(false));
                    }
                    return Ok(Expr::Compare(
                        key,
                        value,
                        match operator {
                            Token::Lt => Comparison::Lt,
                            Token::Le => Comparison::Le,
                            Token::Gt => Comparison::Gt,
                            _ => Comparison::Ge,
                        },
                    ));
                }
                Ok(Expr::Defined(key))
            }
            _ => bail!("Expected context key"),
        }
    }
    fn value(&mut self) -> (String, bool) {
        match self.peek() {
            Some(Token::Quoted(value)) => {
                let result = (value.clone(), true);
                self.at += 1;
                result
            }
            Some(Token::Word(value)) if value != "not" => {
                let result = (value.clone(), false);
                self.at += 1;
                result
            }
            _ => (String::new(), false),
        }
    }
}
fn parse(source: &str) -> Result<Arc<Expr>> {
    let mut parser = Parser {
        tokens: lex(source)?,
        at: 0,
        depth: 0,
    };
    let value = parser.or()?;
    if parser.at != parser.tokens.len() {
        bail!("Unexpected expression token");
    }
    Ok(Arc::new(value))
}
#[derive(Default)]
struct Cache {
    order: VecDeque<String>,
    values: HashMap<String, std::result::Result<Arc<Expr>, String>>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
pub(super) fn compile(source: &str) -> Result<Arc<Expr>> {
    let cache = CACHE.get_or_init(Default::default);
    if let Some(value) = cache
        .lock()
        .unwrap_or_else(|v| v.into_inner())
        .values
        .get(source)
        .cloned()
    {
        return value.map_err(anyhow::Error::msg);
    }
    let result = parse(source).map_err(|error| format!("{error:#}"));
    if source.len() <= BYTES {
        let mut cache = cache.lock().unwrap_or_else(|v| v.into_inner());
        if !cache.values.contains_key(source) {
            while cache.order.len() >= 2048 {
                if let Some(old) = cache.order.pop_front() {
                    cache.values.remove(&old);
                }
            }
            cache.order.push_back(source.into());
            cache.values.insert(source.into(), result.clone());
        }
    }
    result.map_err(anyhow::Error::msg)
}
