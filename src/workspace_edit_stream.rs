//! Original-order resource/text/notebook planning for the native bulk-edit
//! coordinator. This parses the source converter DTO, not query reconstruction.
//! Parsing neither authorizes filesystem effects nor opens/mutates a document.
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::{collections::HashSet, ops::Range};

const MAX_EDITS: usize = 4096;
const MAX_RESOURCES: usize = 128;
const MAX_WIRE: usize = 2 * 1024 * 1024;
const MAX_NODES: usize = 131_072;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Text,
    Notebook,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Create,
    Rename,
    Copy,
    Delete,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub kind: Kind,
    pub entries: Range<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileBatch {
    pub kind: FileKind,
    pub entries: Range<usize>,
}

pub struct File<'a> {
    pub old_resource: Option<&'a str>,
    pub new_resource: Option<&'a str>,
    pub options: Option<&'a Map<String, Value>>,
    pub metadata: Option<&'a Value>,
}
impl<'a> File<'a> {
    pub fn kind(&self) -> FileKind {
        match (self.old_resource, self.new_resource) {
            (None, Some(_)) => FileKind::Create,
            (Some(_), None) => FileKind::Delete,
            (Some(_), Some(_)) if self.option("copy").is_some_and(truthy) => FileKind::Copy,
            (Some(_), Some(_)) => FileKind::Rename,
            (None, None) => unreachable!("stream validates a resource operand"),
        }
    }
    pub fn option(&self, name: &str) -> Option<&'a Value> {
        self.options.and_then(|options| options.get(name))
    }
    /// Preserve the distinction between an absent property and a defined false
    /// (including null) before computing its execution effect.
    pub fn overwrite(&self) -> Option<bool> {
        self.option("overwrite").map(truthy)
    }
    pub fn ignore_existing(&self) -> bool {
        self.overwrite().is_none() && self.option("ignoreIfExists").is_some_and(truthy)
    }
}
pub struct Text<'a> {
    pub resource: &'a str,
    pub edit: &'a Map<String, Value>,
    /// Explicit source version, independent of create suppression.
    pub requested_version: Option<i64>,
    pub metadata: Option<&'a Value>,
}
pub struct Notebook<'a> {
    pub resource: &'a str,
    pub edit: &'a Map<String, Value>,
    pub metadata: Option<&'a Value>,
}
pub enum Entry<'a> {
    File(File<'a>),
    Text(Text<'a>),
    Notebook(Notebook<'a>),
}

pub struct Stream<'a> {
    entries: &'a [Value],
    groups: Vec<Group>,
    creates: HashSet<&'a str>,
}
impl<'a> Stream<'a> {
    /// The private original DTO must stay alive throughout preparation. No
    /// extension getter, public document object, URI override or JSON hook runs.
    pub fn parse(dto: &'a Value) -> Result<Self> {
        budget(dto)?;
        let object = dto
            .as_object()
            .context("Workspace stream must be an object")?;
        ensure!(
            object.len() == 1 && object.contains_key("edits"),
            "Invalid workspace stream fields"
        );
        let entries = object["edits"]
            .as_array()
            .context("Workspace stream edits must be an array")?;
        ensure!(
            entries.len() <= MAX_EDITS,
            "Workspace stream exceeds 4096 edits"
        );
        let mut groups: Vec<Group> = Vec::new();
        let mut creates = HashSet::new();
        let mut resources = HashSet::new();
        for (index, value) in entries.iter().enumerate() {
            let entry = parse_entry(value)?;
            let kind = match entry {
                Entry::File(file) => {
                    if let Some(old) = file.old_resource {
                        resources.insert(old);
                    }
                    if let Some(new) = file.new_resource {
                        resources.insert(new);
                        if file.old_resource.is_none() {
                            creates.insert(new);
                        }
                    }
                    Kind::File
                }
                Entry::Text(text) => {
                    resources.insert(text.resource);
                    Kind::Text
                }
                Entry::Notebook(notebook) => {
                    resources.insert(notebook.resource);
                    Kind::Notebook
                }
            };
            ensure!(
                resources.len() <= MAX_RESOURCES,
                "Workspace stream exceeds 128 resources"
            );
            match groups.last_mut() {
                Some(group) if group.kind == kind => group.entries.end = index + 1,
                _ => groups.push(Group {
                    kind,
                    entries: index..index + 1,
                }),
            }
        }
        Ok(Self {
            entries,
            groups,
            creates,
        })
    }
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn entry(&self, index: usize) -> Result<Entry<'a>> {
        parse_entry(
            self.entries
                .get(index)
                .context("Workspace stream entry outside captured bounds")?,
        )
    }
    /// Source conversion suppresses a text version for ANY create resource in
    /// the whole edit, including a later/ignored create. Native caller/model
    /// proofs remain independent and must still be checked by the coordinator.
    pub fn effective_version(&self, text: &Text<'_>) -> Option<i64> {
        if self.creates.contains(text.resource) {
            None
        } else {
            text.requested_version
        }
    }
    pub fn file_batches(&self, group: &Group) -> Result<Vec<FileBatch>> {
        ensure!(
            group.kind == Kind::File && self.groups.contains(group),
            "File batch requires an original file group"
        );
        let mut batches: Vec<FileBatch> = Vec::new();
        for index in group.entries.clone() {
            let Entry::File(file) = self.entry(index)? else {
                unreachable!("original file group")
            };
            let kind = file.kind();
            match batches.last_mut() {
                Some(batch) if batch.kind == kind => batch.entries.end = index + 1,
                _ => batches.push(FileBatch {
                    kind,
                    entries: index..index + 1,
                }),
            }
        }
        Ok(batches)
    }
}
fn resource(value: Option<&Value>) -> Result<Option<&str>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let uri = value
        .as_str()
        .context("Workspace resource must be a URI string")?;
    ensure!(
        !uri.is_empty() && uri.len() <= 4096 && !uri.contains('\0'),
        "Workspace resource URI exceeds native bounds"
    );
    url::Url::parse(uri).context("Malformed workspace resource URI")?;
    Ok(Some(uri))
}
fn parse_entry(value: &Value) -> Result<Entry<'_>> {
    let object = value
        .as_object()
        .context("Workspace edit must be an object")?;
    let file = object.contains_key("oldResource") || object.contains_key("newResource");
    let text = object.contains_key("textEdit");
    let notebook = object.contains_key("cellEdit");
    ensure!(
        usize::from(file) + usize::from(text) + usize::from(notebook) == 1,
        "Ambiguous or unknown workspace edit kind"
    );
    let metadata = object.get("metadata");
    if file {
        ensure!(
            object
                .keys()
                .all(|key| ["oldResource", "newResource", "options", "metadata"]
                    .contains(&key.as_str())),
            "Invalid file edit fields"
        );
        let old_resource = resource(object.get("oldResource"))?;
        let new_resource = resource(object.get("newResource"))?;
        ensure!(
            old_resource.is_some() || new_resource.is_some(),
            "File edit requires a resource operand"
        );
        let options = object
            .get("options")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_object()
                    .context("File edit options must be an object")
            })
            .transpose()?;
        return Ok(Entry::File(File {
            old_resource,
            new_resource,
            options,
            metadata,
        }));
    }
    let uri = resource(object.get("resource"))?.context("Workspace edit resource missing")?;
    if text {
        ensure!(
            object.keys().all(
                |key| ["resource", "textEdit", "versionId", "metadata"].contains(&key.as_str())
            ),
            "Invalid text edit fields"
        );
        let edit = object["textEdit"]
            .as_object()
            .context("Workspace text edit must be an object")?;
        let requested_version = object
            .get("versionId")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_i64()
                    .context("Workspace text version must be an integer")
            })
            .transpose()?;
        return Ok(Entry::Text(Text {
            resource: uri,
            edit,
            requested_version,
            metadata,
        }));
    }
    ensure!(
        object.keys().all(
            |key| ["resource", "cellEdit", "notebookVersionId", "metadata"].contains(&key.as_str())
        ),
        "Invalid notebook edit fields"
    );
    let edit = object["cellEdit"]
        .as_object()
        .context("Notebook edit must be an object")?;
    Ok(Entry::Notebook(Notebook {
        resource: uri,
        edit,
        metadata,
    }))
}
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}
fn budget(value: &Value) -> Result<()> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize, bytes: &mut usize) -> Result<()> {
        *nodes += 1;
        ensure!(
            depth <= 96 && *nodes <= MAX_NODES,
            "Workspace stream exceeds depth/node bounds"
        );
        *bytes += 8;
        match value {
            Value::String(value) => {
                for byte in value.bytes() {
                    *bytes += match byte {
                        b'"' | b'\\' => 2,
                        0..=31 => 6,
                        _ => 1,
                    };
                    ensure!(*bytes <= MAX_WIRE, "Workspace stream exceeds 2 MiB");
                }
            }
            Value::Array(values) => {
                ensure!(
                    values.len() <= MAX_NODES - *nodes,
                    "Workspace stream array exceeds node bounds"
                );
                for value in values {
                    visit(value, depth + 1, nodes, bytes)?;
                }
            }
            Value::Object(values) => {
                ensure!(
                    values.len() <= MAX_NODES - *nodes,
                    "Workspace stream object exceeds node bounds"
                );
                for (key, value) in values {
                    ensure!(
                        key.len() <= MAX_WIRE - (*bytes).min(MAX_WIRE),
                        "Workspace stream key exceeds byte bounds"
                    );
                    for byte in key.bytes() {
                        *bytes += if byte <= 31 { 6 } else { 2 };
                    }
                    ensure!(*bytes <= MAX_WIRE, "Workspace stream exceeds 2 MiB");
                    visit(value, depth + 1, nodes, bytes)?;
                }
            }
            Value::Number(_) => *bytes += 32,
            Value::Bool(_) | Value::Null => {}
        }
        ensure!(*bytes <= MAX_WIRE, "Workspace stream exceeds 2 MiB");
        Ok(())
    }
    visit(value, 0, &mut 0, &mut 0)
}

#[cfg(test)]
mod tests;
