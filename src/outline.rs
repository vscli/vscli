//! Bounded document-symbol trees; parsing and position checks perform no I/O.
use crate::{
    document::Document,
    lsp::{Position, Range},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{self, Write};

pub const LIMIT: usize = 512;
const DEPTH: usize = 16;
const LABEL_BYTES: usize = 65_536;
const JSON_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub detail: String,
    pub container: String,
    pub kind: u8,
    pub deprecated: bool,
    pub range: Range,
    pub selection_range: Range,
    pub parent: Option<usize>,
    pub depth: u8,
}
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub hierarchical: bool,
}
struct Budget(usize);
impl Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > JSON_BYTES.saturating_sub(self.0) {
            return Err(io::Error::other("Outline response exceeds 2 MiB"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn point(value: &Value) -> Result<Position> {
    let position: Position = serde_json::from_value(value.clone())?;
    if position.line > i32::MAX as usize || position.character > i32::MAX as usize {
        bail!("Outline position exceeds LSP integer limits");
    }
    Ok(position)
}
fn range(value: &Value) -> Result<Range> {
    let result = Range {
        start: point(&value["start"])?,
        end: point(&value["end"])?,
    };
    if coordinates(result.start) > coordinates(result.end) {
        bail!("Outline range is reversed");
    }
    Ok(result)
}
fn coordinates(position: Position) -> (usize, usize) {
    (position.line, position.character)
}
fn text(value: &Value, optional: bool) -> Result<&str> {
    if value.is_null() && optional {
        return Ok("");
    }
    let text = value.as_str().context("Invalid outline label")?;
    if text.len() > 4096 {
        bail!("Outline label exceeds 4 KiB");
    }
    Ok(text)
}
fn resource(uri: &str) -> Result<url::Url> {
    if uri.len() > 4096 {
        bail!("Outline resource URI exceeds 4 KiB");
    }
    let uri = url::Url::parse(uri)?;
    if !matches!(uri.scheme(), "file" | "untitled")
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        bail!("Outline only supports local file and untitled resource URIs");
    }
    if uri.scheme() == "file" && uri.to_file_path().is_err() {
        bail!("Invalid local outline URI");
    }
    Ok(uri)
}
fn json_shape(value: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
    if depth > 64 || *nodes >= JSON_BYTES {
        bail!("Outline JSON exceeds bounded nesting or value count");
    }
    *nodes += 1;
    match value {
        Value::Array(values) => {
            for value in values {
                json_shape(value, depth + 1, nodes)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                json_shape(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn parse_document(value: &Value, resource_uri: &str) -> Result<Tree> {
    let uri = resource(resource_uri)?;
    // Public callers can construct deeper Values than the framed JSON decoder
    // accepts. Reject before the serializer's recursive traversal.
    json_shape(value, 0, &mut 0)?;
    serde_json::to_writer(&mut Budget(0), value).context("Outline JSON budget exceeded")?;
    if value.is_null() {
        return Ok(Tree::default());
    }
    let values = value
        .as_array()
        .context("Invalid document-symbol response")?;
    let hierarchical = values.first().is_none_or(|v| v.get("location").is_none());
    let mut tree = Tree {
        nodes: Vec::new(),
        hierarchical,
    };
    parse_level(values, &uri, None, 0, &mut 0, &mut tree)?;
    Ok(tree)
}
fn parse_level(
    values: &[Value],
    uri: &url::Url,
    parent: Option<usize>,
    depth: usize,
    bytes: &mut usize,
    tree: &mut Tree,
) -> Result<()> {
    if depth > DEPTH {
        bail!("Outline hierarchy exceeds 16 levels");
    }
    for value in values {
        if tree.nodes.len() >= LIMIT {
            bail!("Outline exceeds 512 symbols");
        }
        if !value.is_object() {
            bail!("Invalid outline symbol");
        }
        let name = text(&value["name"], false)?;
        if name.trim().is_empty() {
            bail!("Outline symbol name is empty");
        }
        let detail = text(&value["detail"], true)?;
        let container = text(&value["containerName"], true)?;
        *bytes += name.len() + detail.len() + container.len();
        if *bytes > LABEL_BYTES {
            bail!("Outline display metadata exceeds 64 KiB");
        }
        let kind = value["kind"]
            .as_u64()
            .filter(|k| (1..=26).contains(k))
            .context("Invalid outline symbol kind")? as u8;
        let deprecated = value
            .get("deprecated")
            .map(|v| v.as_bool().context("Invalid deprecated flag"))
            .transpose()?
            .unwrap_or(false);
        let tags = value
            .get("tags")
            .map(|v| v.as_array().context("Invalid outline tags"))
            .transpose()?;
        if tags.is_some_and(|tags| tags.len() > 64) {
            bail!("Too many outline tags");
        }
        let deprecated =
            deprecated || tags.is_some_and(|tags| tags.iter().any(|tag| tag.as_u64() == Some(1)));
        let (outer, selected) = if let Some(location) = value.get("location") {
            if tree.hierarchical || parent.is_some() || value.get("children").is_some() {
                bail!("Mixed or nested flat outline symbols are unsupported");
            }
            if resource(
                location["uri"]
                    .as_str()
                    .context("Missing outline symbol URI")?,
            )? != *uri
            {
                bail!("Document outline symbol redirects to a different resource");
            }
            let range = range(&location["range"])?;
            (range, range)
        } else {
            if !tree.hierarchical {
                bail!("Mixed document-symbol response shapes");
            }
            let outer = range(&value["range"])?;
            let selected = range(&value["selectionRange"])?;
            if coordinates(selected.start) < coordinates(outer.start)
                || coordinates(selected.end) > coordinates(outer.end)
            {
                bail!("Outline selectionRange lies outside its range");
            }
            if let Some(parent) = parent {
                let parent = tree.nodes[parent].range;
                if coordinates(outer.start) < coordinates(parent.start)
                    || coordinates(outer.end) > coordinates(parent.end)
                {
                    bail!("Outline child range lies outside its parent");
                }
            }
            (outer, selected)
        };
        let index = tree.nodes.len();
        tree.nodes.push(Node {
            name: name.into(),
            detail: detail.into(),
            container: container.into(),
            kind,
            deprecated,
            range: outer,
            selection_range: selected,
            parent,
            depth: depth as u8,
        });
        if let Some(children) = value.get("children") {
            parse_level(
                children.as_array().context("Invalid outline children")?,
                uri,
                Some(index),
                depth + 1,
                bytes,
                tree,
            )?;
        }
    }
    Ok(())
}
pub fn strict_offset(doc: &Document, position: Position) -> Result<usize> {
    if position.line > i32::MAX as usize
        || position.character > i32::MAX as usize
        || position.line >= doc.line_count()
    {
        bail!("Outline returned an invalid line or UTF-16 position");
    }
    let start = doc.line_start(position.line);
    let end = doc.line_end(position.line);
    let first = doc.text.char_to_utf16_cu(start);
    let last = doc.text.char_to_utf16_cu(end);
    if position.character > last - first {
        bail!("Outline returned an invalid column");
    }
    let units = first + position.character;
    let cursor = doc.text.utf16_cu_to_char(units);
    if doc.text.char_to_utf16_cu(cursor) != units {
        bail!("Outline position splits a UTF-16 scalar");
    }
    Ok(cursor)
}
impl Tree {
    pub fn validate_document(&self, doc: &Document) -> Result<()> {
        for node in &self.nodes {
            for point in [
                node.range.start,
                node.range.end,
                node.selection_range.start,
                node.selection_range.end,
            ] {
                strict_offset(doc, point)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn symbol() -> Value {
        json!({"name":"猫🙂", "kind":5,"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":4}},"selectionRange":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}}})
    }
    #[test]
    fn hierarchy_preserves_true_parent_selection_and_deprecation() {
        let mut parent = symbol();
        let mut child = symbol();
        child["name"] = json!("method");
        child["kind"] = json!(6);
        child["tags"] = json!([1, 999]);
        child["range"]["start"] = json!({"line":1,"character":0});
        child["selectionRange"] =
            json!({"start":{"line":1,"character":0},"end":{"line":1,"character":4}});
        parent["children"] = json!([child]);
        let tree = parse_document(&json!([parent]), "untitled:Untitled-1").unwrap();
        assert!(tree.hierarchical);
        assert_eq!(tree.nodes.len(), 2);
        assert_eq!(tree.nodes[1].parent, Some(0));
        assert_eq!(tree.nodes[1].depth, 1);
        assert!(tree.nodes[1].deprecated);
        tree.validate_document(&Document::from_text("猫🙂\r\nbody\r\n"))
            .unwrap();
    }
    #[test]
    fn flat_symbols_never_invent_parents_or_redirect_documents() {
        let uri = "untitled:Untitled-1";
        let flat = json!({"name":"child","kind":6,"containerName":"Parent","location":{"uri":uri,"range":symbol()["selectionRange"]}});
        let tree = parse_document(&json!([flat.clone(), flat.clone()]), uri).unwrap();
        assert!(!tree.hierarchical);
        assert!(
            tree.nodes
                .iter()
                .all(|node| node.parent.is_none() && node.depth == 0)
        );
        assert_eq!(tree.nodes[0].container, "Parent");
        let mut redirected = flat.clone();
        redirected["location"]["uri"] = json!("untitled:Untitled-2");
        assert!(parse_document(&json!([redirected]), uri).is_err());
        assert!(parse_document(&json!([flat.clone(), symbol()]), uri).is_err());
        assert!(parse_document(&json!([symbol(), flat]), uri).is_err());
        for uri in [
            "https://example.com/code",
            "untitled:x?query=1",
            "untitled:x#fragment",
        ] {
            assert!(parse_document(&Value::Null, uri).is_err());
        }
    }
    #[test]
    fn malformed_ranges_labels_children_and_integer_limits_reject_whole_tree() {
        for mutation in 0..8 {
            let mut bad = symbol();
            match mutation {
                0 => bad["selectionRange"]["end"] = json!({"line":2,"character":0}),
                1 => bad["range"]["start"] = json!({"line":3,"character":0}),
                2 => bad["range"]["end"]["character"] = json!((i32::MAX as u64) + 1),
                3 => bad["name"] = json!("   "),
                4 => bad["children"] = json!({}),
                5 => bad["kind"] = json!(0),
                6 => bad["deprecated"] = json!("false"),
                _ => bad["tags"] = json!(vec![1; 65]),
            }
            assert!(parse_document(&json!([symbol(), bad]), "untitled:main").is_err());
        }
        let mut parent = symbol();
        let mut child = symbol();
        child["range"]["end"] = json!({"line":2,"character":0});
        parent["children"] = json!([child]);
        assert!(parse_document(&json!([parent]), "untitled:main").is_err());
    }
    #[test]
    fn node_depth_label_and_opaque_json_budgets_reject_without_partial_publication() {
        assert!(parse_document(&json!(vec![symbol(); 513]), "untitled:main").is_err());
        let mut deep = symbol();
        for _ in 0..17 {
            let mut parent = symbol();
            parent["children"] = json!([deep]);
            deep = parent;
        }
        assert!(parse_document(&json!([deep]), "untitled:main").is_err());
        let mut wide = symbol();
        wide["name"] = json!("x".repeat(4096));
        assert!(parse_document(&json!(vec![wide; 17]), "untitled:main").is_err());
        let mut opaque = symbol();
        opaque["data"] = json!("x".repeat(JSON_BYTES));
        assert!(parse_document(&json!([opaque]), "untitled:main").is_err());
        let mut unknown = Value::Null;
        for _ in 0..65 {
            unknown = json!([unknown]);
        }
        let mut opaque = symbol();
        opaque["data"] = unknown;
        assert!(parse_document(&json!([opaque]), "untitled:main").is_err());
        assert!(parse_document(&json!(vec![symbol(); 512]), "untitled:main").is_ok());
    }
    #[test]
    fn strict_utf16_crlf_and_long_line_positions_preserve_document_and_history() {
        let mut doc = Document::from_text("猫🙂\r\nnext\r\n");
        doc.insert("seed", false);
        doc.undo();
        let before = doc.text.to_string();
        let epoch = doc.text_epoch();
        let selections = doc.selections();
        assert_eq!(
            strict_offset(
                &doc,
                Position {
                    line: 0,
                    character: 3
                }
            )
            .unwrap(),
            2
        );
        assert!(
            strict_offset(
                &doc,
                Position {
                    line: 0,
                    character: 2
                }
            )
            .is_err()
        );
        assert!(
            strict_offset(
                &doc,
                Position {
                    line: 0,
                    character: 4
                }
            )
            .is_err()
        );
        assert!(
            strict_offset(
                &doc,
                Position {
                    line: 3,
                    character: 0
                }
            )
            .is_err()
        );
        let tree = parse_document(&json!([symbol()]), "untitled:main").unwrap();
        tree.validate_document(&doc).unwrap();
        assert_eq!(doc.text_epoch(), epoch);
        assert_eq!(doc.selections(), selections);
        assert_eq!(doc.text.to_string(), before);
        doc.redo();
        assert_eq!(doc.text.to_string(), "seed".to_owned() + &before);
        let long = Document::from_text(&("x".repeat(1024 * 1024) + "🙂\r\n"));
        assert_eq!(
            strict_offset(
                &long,
                Position {
                    line: 0,
                    character: 1024 * 1024 + 2
                }
            )
            .unwrap(),
            1024 * 1024 + 1
        );
        assert!(
            strict_offset(
                &long,
                Position {
                    line: 0,
                    character: 1024 * 1024 + 1
                }
            )
            .is_err()
        );
    }
}
