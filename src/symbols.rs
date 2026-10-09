//! Bounded LSP symbol data, with no filesystem access while parsing or rendering.
use crate::lsp::{Position, Range};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const LIMIT: usize = 512;
const KINDS: [&str; 26] = [
    "File",
    "Module",
    "Namespace",
    "Package",
    "Class",
    "Method",
    "Property",
    "Field",
    "Constructor",
    "Enum",
    "Interface",
    "Function",
    "Variable",
    "Constant",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Key",
    "Null",
    "Enum Member",
    "Struct",
    "Event",
    "Operator",
    "Type Parameter",
];
#[derive(Clone, Debug)]
pub struct Symbol {
    pub label: String,
    pub path: PathBuf,
    pub range: Range,
}
fn position(value: &Value) -> Result<Position> {
    let position: Position = serde_json::from_value(value.clone())?;
    if position.line > i32::MAX as usize || position.character > i32::MAX as usize {
        bail!("Symbol position exceeds LSP integer limits");
    }
    Ok(position)
}
fn range(value: &Value) -> Result<Range> {
    let start = position(&value["start"])?;
    let end = position(&value["end"])?;
    if (start.line, start.character) > (end.line, end.character) {
        bail!("Reversed symbol range");
    }
    Ok(Range { start, end })
}
fn path(value: &Value) -> Result<PathBuf> {
    let uri = value.as_str().context("Missing symbol URI")?;
    let url = url::Url::parse(uri)?;
    if url.query().is_some() || url.fragment().is_some() {
        bail!("Symbol URI queries and fragments are unsupported");
    }
    url.to_file_path()
        .map_err(|_| anyhow::anyhow!("Only local file symbol URIs are supported"))
}
fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    let value = value
        .as_str()
        .with_context(|| format!("Invalid symbol {field}"))?;
    if value.len() > 4096 {
        bail!("Symbol {field} exceeds 4 KiB");
    }
    Ok(value)
}
pub fn parse(value: &Value, document: Option<&Path>) -> Result<Vec<Symbol>> {
    let mut output = Vec::new();
    let mut bytes = 0;
    if value.is_null() {
        return Ok(output);
    }
    parse_level(
        value.as_array().context("Invalid symbol response")?,
        document,
        "",
        0,
        &mut bytes,
        &mut output,
    )?;
    Ok(output)
}
fn parse_level(
    values: &[Value],
    document: Option<&Path>,
    parent: &str,
    depth: usize,
    bytes: &mut usize,
    output: &mut Vec<Symbol>,
) -> Result<()> {
    if depth > 16 {
        bail!("Symbol hierarchy exceeds 16 levels");
    }
    for value in values {
        if output.len() >= LIMIT {
            bail!("Symbol response exceeds 512 entries; narrow the workspace query");
        }
        let name = text(&value["name"], "name")?;
        let kind = value["kind"]
            .as_u64()
            .filter(|k| (1..=26).contains(k))
            .context("Invalid symbol kind")?;
        let kind = KINDS[kind as usize - 1];
        let container = value
            .get("containerName")
            .map(|v| text(v, "containerName"))
            .transpose()?
            .unwrap_or(parent);
        let detail = value
            .get("detail")
            .map(|v| text(v, "detail"))
            .transpose()?
            .unwrap_or("");
        let (path, selection) = if let Some(location) = value.get("location") {
            if location.get("range").is_none() {
                bail!(
                    "Unresolved WorkspaceSymbol locations are unsupported; this client does not request symbol resolve"
                );
            }
            if value.get("children").is_some() {
                bail!("Location-based symbol children are unsupported");
            }
            (path(&location["uri"])?, range(&location["range"])?)
        } else {
            let path = document
                .context("Workspace symbol is missing its location")?
                .to_owned();
            let outer = range(&value["range"])?;
            let selected = range(&value["selectionRange"])?;
            if (selected.start.line, selected.start.character)
                < (outer.start.line, outer.start.character)
                || (selected.end.line, selected.end.character)
                    > (outer.end.line, outer.end.character)
            {
                bail!("Symbol selectionRange lies outside its range");
            }
            (path, selected)
        };
        if let Some(document) = document {
            // SymbolInformation returned for a document must not redirect to another file.
            if path != document && crate::lsp::file_uri(&path)? != crate::lsp::file_uri(document)? {
                bail!("Document symbol refers to a different file");
            }
        }
        if path.as_os_str().len() > 4096 {
            bail!("Symbol path exceeds 4 KiB");
        }
        let label = format!(
            "{name}  [{kind}]  {container}  {detail}  {}:{}",
            path.display(),
            selection.start.line + 1
        );
        *bytes += label.len();
        if *bytes > 65536 {
            bail!("Symbol display metadata exceeds 64 KiB");
        }
        output.push(Symbol {
            label,
            path,
            range: selection,
        });
        if let Some(children) = value.get("children") {
            let container = if container.is_empty() {
                name.to_owned()
            } else {
                format!("{container} › {name}")
            };
            parse_level(
                children.as_array().context("Invalid symbol children")?,
                document,
                &container,
                depth + 1,
                bytes,
                output,
            )?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn symbol() -> Value {
        json!({"name":"猫","kind":12,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":8}},"selectionRange":{"start":{"line":0,"character":2},"end":{"line":0,"character":4}}})
    }
    #[test]
    fn hierarchy_flat_locations_and_unicode_metadata_are_bounded() {
        let path = std::env::temp_dir().join("symbols.cpp");
        let mut parent = symbol();
        parent["children"] = json!([symbol()]);
        let items = parse(&json!([parent]), Some(&path)).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[1].label.contains("猫"));
        let location = json!({"name":"flat","kind":12,"location":{"uri":crate::lsp::file_uri(&path).unwrap(),"range":symbol()["selectionRange"]}});
        assert_eq!(parse(&json!([location]), None).unwrap().len(), 1);
        assert!(parse(&json!(vec![symbol(); 513]), Some(&path)).is_err());
        let mut deep = symbol();
        for _ in 0..17 {
            let mut next = symbol();
            next["children"] = json!([deep]);
            deep = next;
        }
        assert!(parse(&json!([deep]), Some(&path)).is_err());
        let mut wide = symbol();
        wide["name"] = json!("x".repeat(4096));
        assert!(parse(&json!(vec![wide; 17]), Some(&path)).is_err());
    }
    #[test]
    fn malformed_unsupported_and_reversed_locations_are_explicit_errors() {
        let path = std::env::temp_dir().join("symbols.cpp");
        for value in [
            json!({"name":"x","kind":12,"location":{"uri":"file:///x"}}),
            json!({"name":"x","kind":12,"location":{"uri":"untitled:x","range":symbol()["range"]}}),
        ] {
            assert!(parse(&json!([value]), None).is_err());
        }
        let mut outside = symbol();
        outside["selectionRange"]["end"]["character"] = json!(99);
        assert!(parse(&json!([outside]), Some(&path)).is_err());
        let mut reversed = symbol();
        reversed["selectionRange"]["start"]["character"] = json!(7);
        assert!(parse(&json!([reversed]), Some(&path)).is_err());
        assert!(parse(&json!({}), None).is_err());
    }
}
