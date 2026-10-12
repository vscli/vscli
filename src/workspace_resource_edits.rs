//! Worker-only adaptation of one ORIGINAL file group to native resource edits.
//! This performs no filesystem IO, model authorization, or editor publication.
use crate::{
    resource_journal::{Control, FileOptions, MAX_BYTES, Operation, Presence},
    workspace_edit_stream::{Entry, File, FileBatch, Group, Kind, Stream},
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value};
use std::path::PathBuf;

#[derive(Debug)]
pub struct PreparedEdit {
    pub original_index: usize,
    pub operation: Option<Operation>,
    pub skipped_untitled_create: Option<String>,
    pub original_options: Option<Map<String, Value>>,
    /// Retain annotations for the controller; preparation does not confirm them.
    pub metadata: Option<Value>,
}
#[derive(Debug)]
pub struct PreparedGroup {
    pub original: Group,
    /// Preserve kind boundaries even when a batch contains only skipped creates.
    pub batches: Vec<FileBatch>,
    pub edits: Vec<PreparedEdit>,
}

/// Prepare all file groups once, before the controller starts any phase effects.
pub fn prepare_groups(stream: &Stream<'_>, control: &Control<'_>) -> Result<Vec<PreparedGroup>> {
    validate_stream_contents(stream, control)?;
    let mut groups = Vec::new();
    groups
        .try_reserve_exact(
            stream
                .groups()
                .iter()
                .filter(|group| group.kind == Kind::File)
                .count(),
        )
        .context("File-group allocation refused")?;
    for group in stream
        .groups()
        .iter()
        .filter(|group| group.kind == Kind::File)
    {
        groups.push(build_group(stream, group, control)?);
    }
    control.check()?;
    Ok(groups)
}

/// File/text/notebook boundaries are never flattened. The caller retains its
/// complete original DTO and model/configuration proofs through authorization.
pub fn prepare_group(
    stream: &Stream<'_>,
    group: &Group,
    control: &Control<'_>,
) -> Result<PreparedGroup> {
    control.check()?;
    stream.file_batches(group)?; // Reject fabricated ranges and non-file groups.
    validate_stream_contents(stream, control)?;
    build_group(stream, group, control)
}

fn build_group(stream: &Stream<'_>, group: &Group, control: &Control<'_>) -> Result<PreparedGroup> {
    let batches = stream.file_batches(group)?;
    let mut edits = Vec::new();
    edits
        .try_reserve_exact(group.entries.len())
        .context("Resource edit allocation refused")?;
    let mut remaining = MAX_BYTES;
    for index in group.entries.clone() {
        control.check()?;
        let Entry::File(file) = stream.entry(index)? else {
            unreachable!("original file group")
        };
        let options = options(&file)?;
        // Revival precedes choosing the file-operation backend. In particular a
        // data-transfer descriptor must not disappear because rename ignores bytes.
        let contents = contents(file.option("contents"), &mut remaining)?;
        if file.old_resource.is_none()
            && let Some(new) = file.new_resource
            && new
                .split_once(':')
                .is_some_and(|(scheme, _)| scheme == "untitled")
        {
            edits.push(PreparedEdit {
                original_index: index,
                operation: None,
                skipped_untitled_create: Some(new.to_owned()),
                original_options: file.options.cloned(),
                metadata: file.metadata.cloned(),
            });
            continue;
        }
        let operation = match (file.old_resource, file.new_resource) {
            (None, Some(new)) => Operation::Create {
                path: local_path(new)?,
                options,
                contents,
            },
            (Some(old), Some(new)) => Operation::Move {
                source: local_path(old)?,
                destination: local_path(new)?,
                options,
            },
            (Some(old), None) => Operation::Delete {
                path: local_path(old)?,
                options,
            },
            (None, None) => unreachable!("validated operands"),
        };
        edits.push(PreparedEdit {
            original_index: index,
            operation: Some(operation),
            skipped_untitled_create: None,
            original_options: file.options.cloned(),
            metadata: file.metadata.cloned(),
        });
    }
    control.check()?;
    Ok(PreparedGroup {
        original: group.clone(),
        batches,
        edits,
    })
}

/// Public apply revives every descriptor before entering any execution phase.
/// Bound file bytes plus scalar text payloads over the whole original stream.
fn validate_stream_contents(stream: &Stream<'_>, control: &Control<'_>) -> Result<()> {
    let mut remaining = 1024 * 1024;
    for group in stream.groups() {
        for index in group.entries.clone() {
            control.check()?;
            match stream.entry(index)? {
                Entry::File(file) => {
                    contents(file.option("contents"), &mut remaining)?;
                }
                Entry::Text(text) => {
                    if let Some(value) = text.edit.get("text").and_then(Value::as_str) {
                        remaining = remaining
                            .checked_sub(value.len())
                            .context("Workspace raw payload exceeds 1 MiB")?;
                    }
                }
                Entry::Notebook(_) => {}
            }
        }
    }
    Ok(())
}

fn local_path(resource: &str) -> Result<PathBuf> {
    let uri = url::Url::parse(resource).context("Malformed resource URI")?;
    ensure!(
        uri.scheme() == "file"
            && uri.query().is_none()
            && uri.fragment().is_none()
            && uri.username().is_empty()
            && uri.password().is_none()
            && uri.port().is_none(),
        "Resource edit requires an unqualified local file URI"
    );
    let path = uri
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Resource URI has no native local path"))?;
    ensure!(
        path.is_absolute()
            && path.to_str().is_some()
            && path.as_os_str().len() <= 4096
            && !path.as_os_str().as_encoded_bytes().contains(&0),
        "Resource path exceeds native bounds"
    );
    Ok(path)
}

fn options(file: &File<'_>) -> Result<FileOptions> {
    fn flag(file: &File<'_>, name: &str) -> Presence<bool> {
        match file.option(name) {
            None => Presence::Absent,
            Some(Value::Null) => Presence::Null,
            Some(value) => Presence::Value(truthy(value)),
        }
    }
    let max_size = match file.option("maxSize") {
        None => Presence::Absent,
        Some(Value::Null) => Presence::Null,
        Some(value) => value.as_f64().map_or(Presence::Absent, Presence::Value),
    };
    Ok(FileOptions {
        overwrite: flag(file, "overwrite"),
        ignore_if_exists: flag(file, "ignoreIfExists"),
        ignore_if_not_exists: flag(file, "ignoreIfNotExists"),
        recursive: flag(file, "recursive"),
        copy: flag(file, "copy"),
        folder: flag(file, "folder"),
        skip_trash_bin: flag(file, "skipTrashBin"),
        max_size,
    })
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn contents(value: Option<&Value>, remaining: &mut usize) -> Result<Option<Vec<u8>>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .context("Invalid resource contents descriptor")?;
    if object.get("type").and_then(Value::as_str) == Some("dataTransferItem") {
        bail!("Resource contents require a data-transfer resolver");
    }
    ensure!(
        object.len() == 2 && object.get("type").and_then(Value::as_str) == Some("base64"),
        "Invalid resource contents descriptor"
    );
    let encoded = object
        .get("value")
        .and_then(Value::as_str)
        .context("Resource contents must be base64 text")?;
    // Bound allocation before invoking the decoder. Canonical source DTOs are
    // padded standard base64; malformed/noncanonical data never reaches effects.
    ensure!(
        encoded.len().is_multiple_of(4),
        "Invalid resource contents base64 length"
    );
    let padding = encoded
        .as_bytes()
        .iter()
        .rev()
        .take(2)
        .take_while(|byte| **byte == b'=')
        .count();
    let capacity = (encoded.len() / 4)
        .checked_mul(3)
        .and_then(|n| n.checked_sub(padding))
        .context("Resource contents length overflow")?;
    ensure!(
        capacity <= *remaining,
        "Resource contents exceed byte budget"
    );
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .context("Resource contents allocation refused")?;
    bytes.resize(capacity, 0);
    let length = STANDARD
        .decode_slice(encoded, &mut bytes)
        .context("Invalid resource contents base64")?;
    bytes.truncate(length);
    *remaining -= bytes.len();
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests;
