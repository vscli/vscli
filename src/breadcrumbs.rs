//! Pure bounded Breadcrumbs projection. No filesystem or language requests.
use crate::outline::Tree;
pub use crate::settings::{BreadcrumbPath as PathMode, Breadcrumbs as Options};
use anyhow::{Result, bail};
use std::path::{Component, Path};

const FILE_SEGMENTS: usize = 64;
const LABEL_BYTES: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
    Folder,
    File,
    Symbol,
    RootSymbols,
    Ellipsis,
}
#[derive(Clone, Debug)]
pub struct Element {
    pub label: String,
    pub kind: ElementKind,
    pub node: Option<usize>,
}
pub fn file_trail(path: &Path, workspace: &Path, mode: PathMode) -> Result<Vec<Element>> {
    if path.as_os_str().len() > LABEL_BYTES || workspace.as_os_str().len() > LABEL_BYTES {
        bail!("Breadcrumbs supports paths up to 4 KiB");
    }
    if mode == PathMode::Off {
        return Ok(Vec::new());
    }
    let relative = path.strip_prefix(workspace).unwrap_or(path);
    let count = relative.components().count();
    let skip = if mode == PathMode::Last {
        count.saturating_sub(1)
    } else {
        count.saturating_sub(FILE_SEGMENTS)
    };
    let mut output = Vec::with_capacity((count - skip).min(FILE_SEGMENTS) + 1);
    let mut bytes = 0;
    if skip > 0 && mode == PathMode::On {
        output.push(Element {
            label: "…".into(),
            kind: ElementKind::Ellipsis,
            node: None,
        });
        bytes = 3;
    }
    for (index, part) in relative.components().enumerate().skip(skip) {
        let label = match part {
            Component::RootDir => std::path::MAIN_SEPARATOR.to_string(),
            _ => part.as_os_str().to_string_lossy().into_owned(),
        };
        bytes += label.len();
        if bytes > LABEL_BYTES {
            bail!("Breadcrumbs file labels exceed 4 KiB");
        }
        output.push(Element {
            label,
            kind: if index + 1 == count {
                ElementKind::File
            } else {
                ElementKind::Folder
            },
            node: None,
        });
    }
    Ok(output)
}
pub fn symbol_trail(tree: &Tree, active: Option<usize>, mode: PathMode) -> Result<Vec<Element>> {
    if mode == PathMode::Off {
        return Ok(Vec::new());
    }
    if tree.nodes.len() > 512 {
        bail!("Breadcrumbs symbol tree exceeds 512 nodes");
    }
    let Some(mut index) = active else {
        return Ok(if tree.nodes.is_empty() {
            Vec::new()
        } else {
            vec![Element {
                label: "…".into(),
                kind: ElementKind::RootSymbols,
                node: None,
            }]
        });
    };
    let mut chain = Vec::with_capacity(17);
    loop {
        let node = tree
            .nodes
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Invalid Breadcrumbs symbol index"))?;
        if chain.len() >= 17 || node.name.len() > 4096 {
            bail!("Breadcrumbs symbol ancestry exceeds bounds");
        }
        chain.push(index);
        if mode == PathMode::Last || !tree.hierarchical {
            break;
        }
        let Some(parent) = node.parent else {
            break;
        };
        if parent >= index {
            bail!("Invalid Breadcrumbs symbol ancestry");
        }
        index = parent;
    }
    chain.reverse();
    Ok(chain
        .into_iter()
        .map(|index| Element {
            label: tree.nodes[index].name.clone(),
            kind: ElementKind::Symbol,
            node: Some(index),
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::parse_document;
    use serde_json::json;
    #[test]
    fn path_modes_unicode_and_deep_components_are_bounded_without_io() {
        let root = Path::new("/project");
        let path = Path::new("/project/src/猫/main.cpp");
        let all = file_trail(path, root, PathMode::On).unwrap();
        assert_eq!(
            all.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(),
            ["src", "猫", "main.cpp"]
        );
        assert_eq!(all[2].kind, ElementKind::File);
        assert_eq!(
            file_trail(path, root, PathMode::Last).unwrap()[0].label,
            "main.cpp"
        );
        assert!(file_trail(path, root, PathMode::Off).unwrap().is_empty());
        let deep = Path::new("/project").join("a/".repeat(100)).join("end.cpp");
        let trail = file_trail(&deep, root, PathMode::On).unwrap();
        assert_eq!(trail.len(), 65);
        assert_eq!(trail[0].kind, ElementKind::Ellipsis);
        assert_eq!(trail.last().unwrap().label, "end.cpp");
        assert!(file_trail(Path::new(&"x".repeat(4097)), root, PathMode::On).is_err());
    }
    #[test]
    fn true_symbol_ancestry_and_flat_qualifiers_never_invent_hierarchy() {
        let range = json!({"start":{"line":0,"character":0},"end":{"line":3,"character":0}});
        let child = json!({"name":"method","kind":6,"range":range,"selectionRange":range});
        let parent = json!({"name":"Class","kind":5,"range":range,"selectionRange":range,"children":[child]});
        let tree = parse_document(&json!([parent]), "untitled:vscli-1").unwrap();
        assert_eq!(
            symbol_trail(&tree, Some(1), PathMode::On)
                .unwrap()
                .iter()
                .map(|e| e.label.as_str())
                .collect::<Vec<_>>(),
            ["Class", "method"]
        );
        assert_eq!(
            symbol_trail(&tree, Some(1), PathMode::Last).unwrap()[0].label,
            "method"
        );
        assert_eq!(
            symbol_trail(&tree, None, PathMode::On).unwrap()[0].kind,
            ElementKind::RootSymbols
        );
        let flat=parse_document(&json!([{"name":"method","kind":6,"containerName":"Class","location":{"uri":"untitled:vscli-1","range":range}}]),"untitled:vscli-1").unwrap();
        assert_eq!(symbol_trail(&flat, Some(0), PathMode::On).unwrap().len(), 1);
        let mut invalid = tree.clone();
        invalid.nodes[1].parent = Some(1);
        assert!(symbol_trail(&invalid, Some(1), PathMode::On).is_err());
    }
}
