//! Native TextMate-style snippet templates. Parsing/expansion is separate from
//! editor sessions so every edit can be staged before changing a document.
use anyhow::{Result, bail, ensure};
use std::{collections::BTreeMap, ops::Range};

pub mod catalog;
pub mod variables;

const MAX_SOURCE: usize = 64 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_MARKERS: usize = 10_000;
const MAX_EXPANSION: usize = 1024 * 1024;

#[derive(Clone, Debug)]
enum Node {
    Text(String),
    Marker {
        key: Key,
        children: Vec<Node>,
        choices: Vec<String>,
        transform: Option<Transform>,
    },
}

#[derive(Clone, Debug)]
enum Key {
    Stop(u32),
    Variable(String),
}

#[derive(Clone, Debug)]
pub struct Transform {
    pub pattern: String,
    pub format: String,
    pub flags: String,
}

#[derive(Clone, Debug)]
pub struct Placeholder {
    pub index: u32,
    /// Unicode scalar offsets within the expanded text.
    pub range: Range<usize>,
    pub choices: Vec<String>,
    pub transform: Option<Transform>,
    /// Enclosing placeholder occurrences, not their numeric tab-stop indices.
    pub parents: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Expansion {
    pub text: String,
    pub placeholders: Vec<Placeholder>,
    pub(crate) model_offsets: Option<ModelOffsets>,
}

#[derive(Clone, Debug)]
pub(crate) struct ModelOffsets {
    pub ranges: Vec<Range<usize>>,
    pub text_len: usize,
}

impl Expansion {
    /// Completion fields track their actual normalized text, unlike the pinned
    /// extension insertion API's historical UTF-16 decoration offsets.
    fn normalize_completion_eol(&mut self, eol: &str) -> Result<()> {
        let normalized = self
            .text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', eol);
        ensure!(
            normalized.len() <= MAX_EXPANSION,
            "Snippet expansion exceeds 1 MiB"
        );
        let mut positions = vec![0];
        let mut offset = 0;
        let mut chars = self.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                offset += eol.chars().count();
                positions.push(offset);
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    positions.push(offset);
                }
            } else {
                offset += if ch == '\n' { eol.chars().count() } else { 1 };
                positions.push(offset);
            }
        }
        for marker in &mut self.placeholders {
            marker.range = positions[marker.range.start]..positions[marker.range.end];
        }
        self.text = normalized;
        self.model_offsets = None;
        Ok(())
    }

    fn normalize_model_eol(&mut self, eol: &str) -> Result<()> {
        let normalized = self
            .text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', eol);
        ensure!(
            normalized.len() <= MAX_EXPANSION,
            "Snippet expansion exceeds 1 MiB"
        );
        if normalized == self.text {
            return Ok(());
        }
        // The pinned controller creates decorations from template UTF-16
        // offsets after the model normalizes EOLs. Preserve that observable
        // behavior even for nested API defaults/choices skipped by adjustment.
        let mut raw_offsets = vec![0];
        for ch in self.text.chars() {
            raw_offsets.push(raw_offsets.last().unwrap() + ch.len_utf16());
        }
        let mut positions = vec![0];
        let mut previous = None;
        for (index, ch) in normalized.chars().enumerate() {
            if previous == Some('\r') && ch == '\n' {
                *positions.last_mut().unwrap() = index - 1;
            }
            positions.extend(std::iter::repeat_n(index + 1, ch.len_utf16()));
            previous = Some(ch);
        }
        let ranges: Vec<_> = self
            .placeholders
            .iter()
            .map(|marker| raw_offsets[marker.range.start]..raw_offsets[marker.range.end])
            .collect();
        for (marker, range) in self.placeholders.iter_mut().zip(&ranges) {
            marker.range = positions[range.start.min(positions.len() - 1)]
                ..positions[range.end.min(positions.len() - 1)];
        }
        self.model_offsets = Some(ModelOffsets {
            ranges,
            text_len: *raw_offsets.last().unwrap(),
        });
        self.text = normalized;
        Ok(())
    }

    pub fn first_selections(&self) -> Vec<Range<usize>> {
        let first = self
            .placeholders
            .iter()
            .map(|p| p.index)
            .min_by_key(|index| {
                if *index == 0 {
                    u64::MAX
                } else {
                    u64::from(*index)
                }
            });
        match first {
            Some(index) => self
                .placeholders
                .iter()
                .filter(|p| p.index == index)
                .map(|p| p.range.clone())
                .collect(),
            None => {
                let end = self.text.chars().count();
                std::iter::once(end..end).collect()
            }
        }
    }
}

/// Resolve each occurrence after template indentation. The optional preceding
/// indent belongs to the last expanded text node's final line.
pub trait VariableResolver {
    fn resolve(&mut self, name: &str, preceding_indent: Option<&str>) -> Result<Option<String>>;
}

impl<F> VariableResolver for F
where
    F: FnMut(&str, Option<&str>) -> Result<Option<String>>,
{
    fn resolve(&mut self, name: &str, preceding_indent: Option<&str>) -> Result<Option<String>> {
        self(name, preceding_indent)
    }
}

#[derive(Clone, Debug)]
pub struct Template {
    nodes: Vec<Node>,
}

/// Document-specific indentation, applied to template text before resolving
/// variables. Keeping this separate avoids modifying regexes or variable values.
pub struct Whitespace<'a> {
    pub leading: &'a str,
    pub eol: &'a str,
    pub tab_size: usize,
    pub insert_spaces: bool,
    /// The pinned extension API adjusts only fragment-level text. The user
    /// command adjusts text inside placeholders too.
    pub fragment_only: bool,
}

impl Whitespace<'_> {
    fn normalize(&self, text: &str) -> Result<String> {
        let tab = self.tab_size.clamp(1, 16);
        let mut columns = 0;
        let mut bytes = 0;
        for ch in text.chars().take_while(|ch| matches!(ch, ' ' | '\t')) {
            columns += if ch == '\t' { tab - columns % tab } else { 1 };
            bytes += ch.len_utf8();
        }
        ensure!(
            columns.saturating_add(text.len() - bytes) <= MAX_EXPANSION,
            "Snippet indentation exceeds 1 MiB"
        );
        let mut result = if self.insert_spaces {
            " ".repeat(columns)
        } else {
            format!(
                "{}{}",
                "\t".repeat(columns / tab),
                " ".repeat(columns % tab)
            )
        };
        result.push_str(&text[bytes..]);
        Ok(result)
    }

    fn adjust(
        &self,
        nodes: &mut [Node],
        previous: &mut Option<char>,
        choice: bool,
        bytes: &mut usize,
    ) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(text) => {
                    if !choice {
                        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                        let mut result = String::new();
                        for (i, line) in normalized.split('\n').enumerate() {
                            let line = if i > 0 || matches!(previous, Some('\r' | '\n')) {
                                self.normalize(&format!("{}{line}", self.leading))?
                            } else if previous.is_none() {
                                self.normalize(line)?
                            } else {
                                line.to_owned()
                            };
                            if i > 0 {
                                result.push_str(self.eol);
                            }
                            ensure!(
                                result.len().saturating_add(line.len()) <= MAX_EXPANSION,
                                "Snippet indentation exceeds 1 MiB"
                            );
                            result.push_str(&line);
                        }
                        *text = result;
                    }
                    *bytes = bytes.saturating_add(text.len());
                    ensure!(*bytes <= MAX_EXPANSION, "Snippet indentation exceeds 1 MiB");
                    if let Some(last) = text.chars().next_back() {
                        *previous = Some(last);
                    }
                }
                Node::Marker {
                    children, choices, ..
                } => {
                    self.adjust(
                        children,
                        previous,
                        choice || self.fragment_only || !choices.is_empty(),
                        bytes,
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl Template {
    pub fn uses_variable(&self, name: &str) -> bool {
        fn contains(nodes: &[Node], name: &str) -> bool {
            nodes.iter().any(|node| match node {
                Node::Marker { key, children, .. } => {
                    matches!(key, Key::Variable(value) if value == name) || contains(children, name)
                }
                Node::Text(_) => false,
            })
        }
        contains(&self.nodes, name)
    }

    /// User snippets historically reinterpret unknown bare variables as editable
    /// placeholders. The extension API deliberately keeps the raw semantics.
    pub fn parse_user(source: &str) -> Result<Self> {
        let mut template = Self::parse(source)?;
        fn maximum(nodes: &[Node]) -> u32 {
            nodes
                .iter()
                .map(|n| match n {
                    Node::Text(_) => 0,
                    Node::Marker { key, children, .. } => maximum(children).max(match key {
                        Key::Stop(i) => *i,
                        _ => 0,
                    }),
                })
                .max()
                .unwrap_or(0)
        }
        let mut next = maximum(&template.nodes);
        let mut names = BTreeMap::new();
        let mut queue: std::collections::VecDeque<_> = template.nodes.iter().collect();
        while let Some(node) = queue.pop_front() {
            if let Node::Marker { key, children, .. } = node {
                if let Key::Variable(name) = key {
                    if children.is_empty() && !known_variable(name) && !names.contains_key(name) {
                        next = next.checked_add(1).ok_or_else(|| {
                            anyhow::anyhow!("Snippet synthetic index exceeds u32")
                        })?;
                        names.insert(name.clone(), next);
                    }
                } else {
                    queue.extend(children);
                }
            }
        }
        fn replace(nodes: &mut [Node], names: &BTreeMap<String, u32>) {
            for node in nodes {
                if let Node::Marker {
                    key,
                    children,
                    choices,
                    transform,
                } = node
                {
                    if let Key::Variable(name) = key {
                        if children.is_empty()
                            && let Some(index) = names.get(name)
                        {
                            children.push(Node::Text(name.clone()));
                            *key = Key::Stop(*index);
                            choices.clear();
                            *transform = None;
                        }
                    } else {
                        replace(children, names);
                    }
                }
            }
        }
        replace(&mut template.nodes, &names);
        Ok(template)
    }

    pub fn parse(source: &str) -> Result<Self> {
        ensure!(
            source.len() <= MAX_SOURCE,
            "Snippet exceeds the 64 KiB source limit"
        );
        let mut parser = Parser {
            source,
            offset: 0,
            markers: 0,
        };
        let (nodes, _) = parser.nodes(0, false)?;
        Ok(Self { nodes })
    }

    /// Resolve variables supplied by the caller. Unknown variables retain their
    /// default children or expand to empty, matching TextEditor.insertSnippet.
    pub fn expand(&self, variables: &BTreeMap<String, String>) -> Result<Expansion> {
        let mut defaults = BTreeMap::new();
        collect_defaults(&self.nodes, &mut defaults);
        self.expand_nodes(
            &mut |name: &str, _: Option<&str>| Ok(variables.get(name).cloned()),
            defaults,
        )
    }

    pub fn expand_with_whitespace(
        &self,
        variables: &BTreeMap<String, String>,
        whitespace: &Whitespace<'_>,
    ) -> Result<Expansion> {
        self.expand_with_resolver(
            &mut |name: &str, _: Option<&str>| Ok(variables.get(name).cloned()),
            whitespace,
        )
    }

    pub fn expand_with_resolver(
        &self,
        variables: &mut dyn VariableResolver,
        whitespace: &Whitespace<'_>,
    ) -> Result<Expansion> {
        self.expand_with_resolver_inner(variables, whitespace, false)
    }

    pub(crate) fn expand_for_completion(
        &self,
        variables: &mut dyn VariableResolver,
        whitespace: &Whitespace<'_>,
    ) -> Result<Expansion> {
        self.expand_with_resolver_inner(variables, whitespace, true)
    }

    fn expand_with_resolver_inner(
        &self,
        variables: &mut dyn VariableResolver,
        whitespace: &Whitespace<'_>,
        completion: bool,
    ) -> Result<Expansion> {
        ensure!(
            whitespace.leading.len() <= MAX_EXPANSION,
            "Snippet indentation exceeds 1 MiB"
        );
        let mut defaults = BTreeMap::new();
        collect_defaults(&self.nodes, &mut defaults);
        // Materialize each occurrence before indentation. A mirrored multiline
        // default can appear at a different column from its defining occurrence.
        let mut budget = (0usize, 0usize);
        let mut choices = BTreeMap::new();
        collect_default_choices(&self.nodes, &mut choices);
        let mut nodes = materialize(
            &self.nodes,
            &defaults,
            &choices,
            &mut Vec::new(),
            0,
            &mut budget,
        )?;
        whitespace.adjust(&mut nodes, &mut None, false, &mut 0)?;
        let mut result = Self { nodes }.expand_nodes(variables, BTreeMap::new())?;
        if completion {
            result.normalize_completion_eol(whitespace.eol)?;
        } else {
            result.normalize_model_eol(whitespace.eol)?;
        }
        Ok(result)
    }

    fn expand_nodes(
        &self,
        variables: &mut dyn VariableResolver,
        defaults: BTreeMap<u32, Vec<Node>>,
    ) -> Result<Expansion> {
        let mut builder = Builder {
            result: Expansion::default(),
            chars: 0,
            defaults,
            variables,
            preceding_indent: None,
            stack: Vec::new(),
            parents: Vec::new(),
        };
        builder.nodes(&self.nodes, 0)?;
        if !builder.result.placeholders.is_empty()
            && !builder.result.placeholders.iter().any(|p| p.index == 0)
        {
            builder.result.placeholders.push(Placeholder {
                index: 0,
                range: builder.chars..builder.chars,
                choices: Vec::new(),
                transform: None,
                parents: Vec::new(),
            });
        }
        Ok(builder.result)
    }
}

fn collect_default_choices(nodes: &[Node], result: &mut BTreeMap<u32, Vec<String>>) {
    for node in nodes {
        if let Node::Marker {
            key,
            children,
            choices,
            ..
        } = node
        {
            if let Key::Stop(index) = key
                && *index != 0
                && (!children.is_empty() || !choices.is_empty())
            {
                result.entry(*index).or_insert_with(|| choices.clone());
            }
            collect_default_choices(children, result);
        }
    }
}

fn materialize(
    nodes: &[Node],
    defaults: &BTreeMap<u32, Vec<Node>>,
    default_choices: &BTreeMap<u32, Vec<String>>,
    stack: &mut Vec<u32>,
    depth: usize,
    budget: &mut (usize, usize),
) -> Result<Vec<Node>> {
    ensure!(depth <= MAX_DEPTH, "Snippet expansion exceeds 64 levels");
    let mut result = Vec::new();
    for node in nodes {
        let node = match node {
            Node::Text(text) => {
                budget.1 = budget.1.saturating_add(text.len());
                ensure!(budget.1 <= MAX_EXPANSION, "Snippet expansion exceeds 1 MiB");
                Node::Text(text.clone())
            }
            Node::Marker {
                key,
                children,
                choices,
                transform,
            } => {
                budget.0 += 1;
                ensure!(
                    budget.0 <= MAX_MARKERS,
                    "Snippet expansion has too many markers"
                );
                let children = if let Key::Stop(index) = key {
                    if stack.contains(index) {
                        Vec::new()
                    } else {
                        stack.push(*index);
                        let children = materialize(
                            defaults.get(index).unwrap_or(children),
                            defaults,
                            default_choices,
                            stack,
                            depth + 1,
                            budget,
                        )?;
                        stack.pop();
                        children
                    }
                } else {
                    materialize(
                        children,
                        defaults,
                        default_choices,
                        stack,
                        depth + 1,
                        budget,
                    )?
                };
                Node::Marker {
                    key: key.clone(),
                    children,
                    choices: match key {
                        Key::Stop(index) => default_choices.get(index).unwrap_or(choices).clone(),
                        _ => choices.clone(),
                    },
                    transform: transform.clone(),
                }
            }
        };
        result.push(node);
    }
    Ok(result)
}

fn known_variable(name: &str) -> bool {
    matches!(
        name,
        "CURRENT_YEAR"
            | "CURRENT_YEAR_SHORT"
            | "CURRENT_MONTH"
            | "CURRENT_DATE"
            | "CURRENT_HOUR"
            | "CURRENT_MINUTE"
            | "CURRENT_SECOND"
            | "CURRENT_DAY_NAME"
            | "CURRENT_DAY_NAME_SHORT"
            | "CURRENT_MONTH_NAME"
            | "CURRENT_MONTH_NAME_SHORT"
            | "CURRENT_SECONDS_UNIX"
            | "CURRENT_TIMEZONE_OFFSET"
            | "SELECTION"
            | "CLIPBOARD"
            | "TM_SELECTED_TEXT"
            | "TM_CURRENT_LINE"
            | "TM_CURRENT_WORD"
            | "TM_LINE_INDEX"
            | "TM_LINE_NUMBER"
            | "TM_FILENAME"
            | "TM_FILENAME_BASE"
            | "TM_DIRECTORY"
            | "TM_FILEPATH"
            | "CURSOR_INDEX"
            | "CURSOR_NUMBER"
            | "RELATIVE_FILEPATH"
            | "BLOCK_COMMENT_START"
            | "BLOCK_COMMENT_END"
            | "LINE_COMMENT"
            | "WORKSPACE_NAME"
            | "WORKSPACE_FOLDER"
            | "RANDOM"
            | "RANDOM_HEX"
            | "UUID"
    )
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
    markers: usize,
}
impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.offset += ch.len_utf8();
        Some(ch)
    }
    fn eat(&mut self, ch: char) -> bool {
        if self.peek() == Some(ch) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn nodes(&mut self, depth: usize, closing: bool) -> Result<(Vec<Node>, bool)> {
        ensure!(depth <= MAX_DEPTH, "Snippet nesting exceeds 64 levels");
        let mut nodes = Vec::new();
        while let Some(ch) = self.peek() {
            if closing && ch == '}' {
                self.bump();
                return Ok((nodes, true));
            }
            if ch == '$' {
                let start = self.offset;
                if let Some(marker) = self.marker(depth)? {
                    nodes.extend(marker);
                    continue;
                }
                self.offset = start;
            }
            self.bump();
            let ch = if ch == '\\' && self.peek().is_some_and(|c| matches!(c, '$' | '}' | '\\')) {
                self.bump().unwrap()
            } else {
                ch
            };
            if let Some(Node::Text(text)) = nodes.last_mut() {
                text.push(ch);
            } else {
                nodes.push(Node::Text(ch.to_string()));
            }
        }
        Ok((nodes, false))
    }
    fn marker(&mut self, depth: usize) -> Result<Option<Vec<Node>>> {
        self.bump(); // $
        let braced = self.eat('{');
        let start = self.offset;
        let numeric = self.peek().is_some_and(|c| c.is_ascii_digit());
        if numeric {
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        } else if self
            .peek()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            self.bump();
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                self.bump();
            }
        }
        if start == self.offset {
            return Ok(None);
        }
        let name = &self.source[start..self.offset];
        let key = if numeric {
            Key::Stop(
                name.parse()
                    .map_err(|_| anyhow::anyhow!("Snippet tab-stop index exceeds u32"))?,
            )
        } else {
            Key::Variable(name.into())
        };
        self.markers += 1;
        ensure!(self.markers <= MAX_MARKERS, "Snippet has too many markers");
        let mut children = Vec::new();
        let mut choices = Vec::new();
        let mut transform = None;
        if braced {
            if self.eat(':') {
                let (parsed, closed) = self.nodes(depth + 1, true)?;
                if !closed {
                    let mut fallback = vec![Node::Text(format!("${{{name}:"))];
                    fallback.extend(parsed);
                    return Ok(Some(fallback));
                }
                children = parsed;
            } else if matches!(key, Key::Stop(index) if index > 0) && self.eat('|') {
                let mut choice = String::new();
                loop {
                    match self.bump() {
                        Some(',') | Some('|') if choice.is_empty() => return Ok(None),
                        Some(',') => choices.push(std::mem::take(&mut choice)),
                        Some('|') => {
                            choices.push(choice);
                            if !self.eat('}') {
                                return Ok(None);
                            }
                            break;
                        }
                        Some('\\')
                            if self.peek().is_some_and(|c| matches!(c, ',' | '|' | '\\')) =>
                        {
                            choice.push(self.bump().unwrap())
                        }
                        Some(ch) => choice.push(ch),
                        None => return Ok(None),
                    }
                }
            } else if self.eat('/') {
                let Some(pattern) = self.transform_part(false) else {
                    return Ok(None);
                };
                let Some(format) = self.transform_part(true) else {
                    return Ok(None);
                };
                let start = self.offset;
                while self.peek().is_some_and(|c| c != '}') {
                    self.bump();
                }
                let flags = self.source[start..self.offset].to_string();
                if !self.eat('}') {
                    return Ok(None);
                }
                transform = Some(Transform {
                    pattern,
                    format,
                    flags,
                });
            } else if !self.eat('}') {
                return Ok(None);
            }
        }
        Ok(Some(vec![Node::Marker {
            key,
            children,
            choices,
            transform,
        }]))
    }
    fn transform_part(&mut self, format: bool) -> Option<String> {
        let mut output = String::new();
        let mut braces: usize = 0;
        while let Some(ch) = self.bump() {
            if ch == '/' && braces == 0 {
                return Some(output);
            }
            if ch == '\\' && self.peek() == Some('/') {
                output.push(self.bump()?);
                continue;
            }
            if format {
                if ch == '{' {
                    braces += 1;
                }
                if ch == '}' {
                    braces = braces.saturating_sub(1);
                }
            }
            output.push(ch);
        }
        None
    }
}

fn collect_defaults(nodes: &[Node], defaults: &mut BTreeMap<u32, Vec<Node>>) {
    for node in nodes {
        if let Node::Marker {
            key,
            children,
            choices,
            ..
        } = node
        {
            if let Key::Stop(index) = key
                && *index != 0
                && (!children.is_empty() || !choices.is_empty())
            {
                defaults.entry(*index).or_insert_with(|| {
                    if choices.is_empty() {
                        children.clone()
                    } else {
                        vec![Node::Text(choices[0].clone())]
                    }
                });
            }
            collect_defaults(children, defaults);
        }
    }
}

struct Builder<'a> {
    result: Expansion,
    chars: usize,
    defaults: BTreeMap<u32, Vec<Node>>,
    variables: &'a mut dyn VariableResolver,
    preceding_indent: Option<String>,
    stack: Vec<u32>,
    parents: Vec<usize>,
}
impl Builder<'_> {
    fn text(&mut self, text: &str) -> Result<()> {
        ensure!(
            self.result.text.len().saturating_add(text.len()) <= MAX_EXPANSION,
            "Snippet expansion exceeds 1 MiB"
        );
        self.preceding_indent = Some(
            text.rsplit(['\r', '\n'])
                .next()
                .unwrap_or("")
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t'))
                .collect(),
        );
        self.chars += text.chars().count();
        self.result.text.push_str(text);
        Ok(())
    }
    fn nodes(&mut self, nodes: &[Node], depth: usize) -> Result<()> {
        ensure!(depth <= MAX_DEPTH, "Snippet expansion exceeds 64 levels");
        for node in nodes {
            match node {
                Node::Text(text) => self.text(text)?,
                Node::Marker {
                    key: Key::Variable(name),
                    children,
                    transform,
                    ..
                } => {
                    if let Some(value) = self
                        .variables
                        .resolve(name, self.preceding_indent.as_deref())?
                    {
                        if let Some(transform) = transform {
                            self.text(&transform.apply(&value)?)?;
                        } else {
                            self.text(&value)?;
                        }
                    } else if let Some(transform) = transform {
                        self.text(&transform.apply("")?)?;
                    } else {
                        self.nodes(children, depth + 1)?;
                    }
                }
                Node::Marker {
                    key: Key::Stop(index),
                    children,
                    choices,
                    transform,
                } => {
                    ensure!(
                        self.result.placeholders.len() < MAX_MARKERS,
                        "Snippet expansion has too many placeholders"
                    );
                    let id = self.result.placeholders.len();
                    self.result.placeholders.push(Placeholder {
                        index: *index,
                        range: self.chars..self.chars,
                        choices: choices.clone(),
                        transform: transform.clone(),
                        parents: self.parents.clone(),
                    });
                    if !self.stack.contains(index) {
                        self.stack.push(*index);
                        self.parents.push(id);
                        let content = self
                            .defaults
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| children.clone());
                        self.nodes(&content, depth + 1)?;
                        self.parents.pop();
                        self.stack.pop();
                    }
                    self.result.placeholders[id].range.end = self.chars;
                }
            }
        }
        Ok(())
    }
}

impl Transform {
    /// Rust's bounded regular-expression engine currently supports the common
    /// linear-time subset. Unsupported ECMAScript constructs fail explicitly.
    pub fn apply(&self, value: &str) -> Result<String> {
        ensure!(
            self.pattern.len() <= MAX_SOURCE && self.format.len() <= MAX_SOURCE,
            "Snippet transform exceeds 64 KiB"
        );
        ensure!(
            value.len() <= MAX_EXPANSION,
            "Snippet transform input exceeds 1 MiB"
        );
        ensure!(
            self.flags
                .chars()
                .all(|c| matches!(c, 'g' | 'i' | 'm' | 's' | 'u')),
            "Unsupported snippet regex flags"
        );
        let regex = regex::RegexBuilder::new(&self.pattern)
            .case_insensitive(self.flags.contains('i'))
            .multi_line(self.flags.contains('m'))
            .dot_matches_new_line(self.flags.contains('s'))
            .size_limit(MAX_EXPANSION)
            .build()?;
        let mut output = String::new();
        let mut previous = 0;
        let mut matched = false;
        for capture in regex.captures_iter(value) {
            matched = true;
            let found = capture.get(0).unwrap();
            output.push_str(&value[previous..found.start()]);
            output.push_str(&format_capture(&self.format, Some(&capture))?.0);
            ensure!(
                output.len() <= MAX_EXPANSION,
                "Snippet transform output exceeds 1 MiB"
            );
            previous = found.end();
            if !self.flags.contains('g') {
                break;
            }
        }
        if !matched {
            let (fallback, has_else) = format_capture(&self.format, None)?;
            if has_else {
                return Ok(fallback);
            }
        }
        output.push_str(&value[previous..]);
        ensure!(
            output.len() <= MAX_EXPANSION,
            "Snippet transform output exceeds 1 MiB"
        );
        Ok(output)
    }
}

fn format_capture(format: &str, captures: Option<&regex::Captures<'_>>) -> Result<(String, bool)> {
    let mut output = String::new();
    let mut has_else = false;
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut index = String::new();
        while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            index.push(chars.next().unwrap());
        }
        if index.is_empty() {
            output.push('$');
            if braced {
                output.push('{');
            }
            continue;
        }
        let index = index.parse::<usize>()?;
        let value = captures
            .and_then(|c| c.get(index))
            .map_or("", |m| m.as_str());
        if !braced {
            output.push_str(value);
            continue;
        }
        match chars.next() {
            Some('}') => output.push_str(value),
            Some(':') => {
                let mut modifier = String::new();
                for ch in chars.by_ref() {
                    if ch == '}' {
                        break;
                    }
                    modifier.push(ch);
                }
                let replacement = if let Some(case) = modifier.strip_prefix('/') {
                    match case {
                        "upcase" => value.to_uppercase(),
                        "downcase" => value.to_lowercase(),
                        "capitalize" => {
                            let mut c = value.chars();
                            c.next().map_or_else(String::new, |first| {
                                // VS Code capitalizes the first UTF-16 code unit.
                                if first.len_utf16() == 2 {
                                    value.into()
                                } else {
                                    first.to_uppercase().collect::<String>() + c.as_str()
                                }
                            })
                        }
                        "camelcase" | "pascalcase" => {
                            let words: Vec<_> = value
                                .split(|c: char| !c.is_ascii_alphanumeric())
                                .filter(|word| !word.is_empty())
                                .collect();
                            if words.is_empty() {
                                value.into()
                            } else {
                                words
                                    .iter()
                                    .enumerate()
                                    .map(|(i, word)| {
                                        let first = &word[..1];
                                        (if i == 0 && case == "camelcase" {
                                            first.to_ascii_lowercase()
                                        } else {
                                            first.to_ascii_uppercase()
                                        }) + &word[1..]
                                    })
                                    .collect()
                            }
                        }
                        _ => value.into(),
                    }
                } else if let Some(yes) = modifier.strip_prefix('+') {
                    if value.is_empty() {
                        String::new()
                    } else {
                        yes.into()
                    }
                } else if let Some(condition) = modifier.strip_prefix('?') {
                    let (yes, no) = condition.split_once(':').unwrap_or((condition, ""));
                    has_else |= !no.is_empty();
                    if value.is_empty() {
                        no.into()
                    } else {
                        yes.into()
                    }
                } else {
                    let fallback = modifier.strip_prefix('-').unwrap_or(&modifier);
                    has_else |= !fallback.is_empty();
                    if value.is_empty() {
                        fallback.into()
                    } else {
                        value.into()
                    }
                };
                output.push_str(&replacement);
            }
            _ => bail!("Invalid snippet capture format"),
        }
        ensure!(
            output.len() <= MAX_EXPANSION,
            "Snippet capture output exceeds 1 MiB"
        );
    }
    Ok((output, has_else))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_ranges_follow_normalized_variable_text_and_astral_fields() {
        let template = Template::parse_user("${1:$TM_SELECTED_TEXT}-$1-${2:🙂}$0").unwrap();
        for eol in ["\n", "\r\n"] {
            let expansion = template
                .expand_for_completion(
                    &mut |name: &str, _: Option<&str>| {
                        Ok((name == "TM_SELECTED_TEXT").then(|| "α🙂\n猫".into()))
                    },
                    &Whitespace {
                        leading: "",
                        eol,
                        tab_size: 4,
                        insert_spaces: true,
                        fragment_only: false,
                    },
                )
                .unwrap();
            let value = format!("α🙂{eol}猫");
            assert_eq!(expansion.text, format!("{value}-{value}-🙂"));
            let rope = ropey::Rope::from_str(&expansion.text);
            for marker in &expansion.placeholders {
                let expected = match marker.index {
                    1 => value.as_str(),
                    2 => "🙂",
                    0 => "",
                    _ => unreachable!(),
                };
                assert_eq!(rope.slice(marker.range.clone()).to_string(), expected);
            }
            assert!(expansion.model_offsets.is_none());
        }
    }
    #[test]
    fn limits_reject_deep_sources_and_amplified_defaults_without_editing() {
        assert!(Template::parse(&"x".repeat(MAX_SOURCE + 1)).is_err());
        assert!(Template::parse(&format!("{}x{}", "${1:".repeat(100), "}".repeat(100))).is_err());
        let mut source = String::new();
        for index in 1..30 {
            source += &format!("${{{index}:${}${}}}", index + 1, index + 1);
        }
        source += "${30:large}";
        assert!(
            Template::parse(&source)
                .unwrap()
                .expand(&BTreeMap::new())
                .is_err()
        );
    }
    #[test]
    fn nested_mirrors_keep_scalar_ranges_and_parent_occurrences() {
        let expanded = Template::parse("${1:a${2:猫}b}-$1-$2$0")
            .unwrap()
            .expand(&BTreeMap::new())
            .unwrap();
        assert_eq!(expanded.text, "a猫b-a猫b-猫");
        assert_eq!(expanded.first_selections(), vec![0..3, 4..7]);
        let nested: Vec<_> = expanded
            .placeholders
            .iter()
            .filter(|p| p.index == 2)
            .collect();
        assert_eq!(
            nested.iter().map(|p| p.range.clone()).collect::<Vec<_>>(),
            vec![1..2, 5..6, 8..9]
        );
        assert_eq!(nested[0].parents, vec![0]);
        assert_eq!(nested[1].parents, vec![2]);
        assert!(nested[2].parents.is_empty());
    }

    #[test]
    fn transforms_cover_captures_conditionals_case_and_no_match_fallback() {
        for (input, pattern, format, flags, expected) in [
            (
                "snake_case-name",
                "(.*)",
                "${1:/camelcase}",
                "",
                "snakeCaseName",
            ),
            (
                "snake_case-name",
                "(.*)",
                "${1:/pascalcase}",
                "",
                "SnakeCaseName",
            ),
            ("fox", "(.*)", "${1:/upcase}", "", "FOX"),
            ("CAT", "(.*)", "${1:/downcase}", "", "cat"),
            ("cat", "(cat)|(dog)", "${1:+yes}${2:-no}", "", "yesno"),
            ("dog", "(cat)", "${1:?yes:no}", "", "no"),
            ("dog", "(cat)", "$1", "", "dog"),
            ("aa", "a", "b", "g", "bb"),
            ("aa", "a", "b", "", "ba"),
            ("fox", "(.*)", "${1:/unknown}", "", "fox"),
        ] {
            let transform = Transform {
                pattern: pattern.into(),
                format: format.into(),
                flags: flags.into(),
            };
            assert_eq!(transform.apply(input).unwrap(), expected);
        }
        let unsupported = Transform {
            pattern: "(?=x)".into(),
            format: "y".into(),
            flags: String::new(),
        };
        assert!(unsupported.apply("x").is_err());
    }
}
