//! Atomic edits to retained shared models; transport ownership stays with callers.
use crate::{document::Document, lsp};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{collections::HashSet, path::PathBuf};

const MAX_TARGETS: usize = 128;
const MAX_EDITS: usize = 4096;
const MAX_REPLACEMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_URI_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceVersion {
    Native(i64),
    Extension(u64),
}

#[derive(Clone, Debug)]
pub(super) struct WorkspaceEditTarget {
    pub uri: String,
    pub path: Option<PathBuf>,
    pub document: u64,
    pub revision: u64,
    pub text_epoch: u64,
    pub version: WorkspaceVersion,
}

#[derive(Clone, Copy)]
pub(super) struct WorkspaceEditPolicy {
    pub require_versions: bool,
    /// Optional compatibility mirrors have a smaller aggregate budget than core editing.
    pub max_total_document_bytes: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct WorkspaceEditOutcome {
    pub buffers: usize,
    pub text_edits: usize,
}

struct WireEdit<'a> {
    uri: &'a str,
    version: Option<&'a Value>,
    edits: &'a Value,
}

#[derive(Clone, Copy)]
enum Location {
    Visible(usize),
    Hidden(usize),
}

fn fields(value: &Value, allowed: &[&str], message: &str) -> Result<()> {
    ensure!(
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str()))),
        "{message}"
    );
    Ok(())
}

fn wire_edits(edit: &Value, policy: WorkspaceEditPolicy) -> Result<Vec<WireEdit<'_>>> {
    fields(
        edit,
        &["changes", "documentChanges", "changeAnnotations"],
        "Unsupported workspace edit fields",
    )?;
    if edit
        .get("changeAnnotations")
        .is_some_and(|v| v.as_object().is_none_or(|a| !a.is_empty()))
    {
        bail!("Annotated workspace edits require a review UI and are not supported yet");
    }
    if policy.require_versions
        && (edit.get("changes").is_some() || !edit["documentChanges"].is_array())
    {
        bail!("Command workspace edits require documentChanges with explicit numeric versions");
    }
    let mut result = Vec::new();
    if let Some(changes) = edit.get("documentChanges") {
        ensure!(
            edit.get("changes").is_none(),
            "Ambiguous workspace edit contains both edit forms"
        );
        let changes = changes.as_array().context("Invalid documentChanges")?;
        ensure!(
            changes.len() <= MAX_TARGETS,
            "Code action exceeds 128 edited buffers"
        );
        for change in changes {
            ensure!(
                change.get("kind").is_none(),
                "Workspace file creation, rename and deletion actions are not supported yet"
            );
            fields(
                change,
                &["textDocument", "edits"],
                "Unsupported document edit fields",
            )?;
            let document = change
                .get("textDocument")
                .context("Missing workspace edit document")?;
            fields(
                document,
                &["uri", "version"],
                "Unsupported workspace document fields",
            )?;
            ensure!(
                !policy.require_versions
                    || document["version"].is_i64()
                    || document["version"].is_u64(),
                "Command workspace edits require documentChanges with explicit numeric versions"
            );
            let version = document
                .get("version")
                .context("Missing workspace edit version")?;
            ensure!(
                version.is_null() || version.is_number(),
                "Invalid workspace edit version"
            );
            result.push(WireEdit {
                uri: document["uri"]
                    .as_str()
                    .context("Missing workspace edit URI")?,
                version: (!version.is_null()).then_some(version),
                edits: change
                    .get("edits")
                    .context("Missing workspace text edits")?,
            });
        }
    } else if let Some(changes) = edit.get("changes") {
        let changes = changes
            .as_object()
            .context("Invalid workspace edit changes")?;
        ensure!(
            changes.len() <= MAX_TARGETS,
            "Code action exceeds 128 edited buffers"
        );
        result.extend(changes.iter().map(|(uri, edits)| WireEdit {
            uri,
            version: None,
            edits,
        }));
    } else {
        bail!("No workspace text edits");
    }
    Ok(result)
}

fn position(value: &Value) -> Result<lsp::Position> {
    fields(
        value,
        &["line", "character"],
        "Invalid workspace edit position",
    )?;
    let coordinate = |name: &str| -> Result<usize> {
        let value = value[name]
            .as_u64()
            .context("Invalid workspace edit coordinate")?;
        ensure!(
            value <= i32::MAX as u64,
            "Workspace edit coordinate exceeds LSP bounds"
        );
        Ok(value as usize)
    };
    Ok(lsp::Position {
        line: coordinate("line")?,
        character: coordinate("character")?,
    })
}

fn offset(document: &Document, position: lsp::Position) -> Result<usize> {
    ensure!(
        position.line < document.line_count(),
        "Language server returned an invalid line"
    );
    let line = document.line_slice(position.line);
    ensure!(
        position.character <= line.len_utf16_cu(),
        "Language server returned an invalid column"
    );
    // Ropey's indexed conversion avoids rescanning a long line for each of 4096 edits.
    let character = line.utf16_cu_to_char(position.character);
    ensure!(
        line.char_to_utf16_cu(character) == position.character,
        "Workspace edit splits a UTF-16 surrogate pair"
    );
    Ok(document.line_start(position.line) + character)
}

fn normalized(text: &str, eol: &str, available: usize) -> Result<String> {
    ensure!(
        matches!(eol, "\n" | "\r\n"),
        "Unsupported document line ending"
    );
    if !text.contains(['\r', '\n']) {
        ensure!(
            text.len() <= available,
            "Normalized code action exceeds 4 MiB replacement text"
        );
        return Ok(text.to_owned());
    }
    // Calculate expansion before allocating so even a rejected CRLF expansion
    // cannot double an unbounded String capacity on the input path.
    let mut normalized_bytes = 0usize;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let bytes = if character == '\r' || character == '\n' {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            eol.len()
        } else {
            character.len_utf8()
        };
        ensure!(
            bytes <= available.saturating_sub(normalized_bytes),
            "Normalized code action exceeds 4 MiB replacement text"
        );
        normalized_bytes += bytes;
    }
    let mut result = String::with_capacity(normalized_bytes);
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' || character == '\n' {
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
            result.push_str(eol);
        } else {
            result.push(character);
        }
    }
    Ok(result)
}

fn version_current(version: Option<&Value>, expected: WorkspaceVersion) -> Result<bool> {
    let Some(version) = version else {
        return Ok(true);
    };
    Ok(match expected {
        WorkspaceVersion::Native(expected) => {
            version.as_i64().context("Invalid workspace edit version")? == expected
        }
        WorkspaceVersion::Extension(expected) => {
            let version = version.as_u64().context("Invalid workspace edit version")?;
            ensure!(
                version < (1u64 << 53),
                "Extension workspace edit version exceeds safe integer bounds"
            );
            version == expected
        }
    })
}

/// Callers first qualify request origin and editor context. This additionally checks
/// every target's live model and transport version, then stages ALL changes before
/// touching any document. No target is opened, promoted, saved, or replaced.
pub(super) fn apply_workspace_edit(
    documents: &mut [Document],
    hidden: &mut [Document],
    edit: &Value,
    targets: &[WorkspaceEditTarget],
    policy: WorkspaceEditPolicy,
    target_current: impl Fn(&WorkspaceEditTarget, &Document) -> bool,
) -> Result<WorkspaceEditOutcome> {
    ensure!(
        targets.len() <= MAX_TARGETS,
        "Code actions support at most 128 captured buffers"
    );
    let wire = wire_edits(edit, policy)?;
    let mut seen = HashSet::new();
    let mut staged = Vec::new();
    let mut count_edits = 0usize;
    let mut raw_bytes = 0usize;
    let mut expanded_bytes = 0usize;
    let mut total_bytes =
        documents
            .iter()
            .chain(hidden.iter())
            .try_fold(0usize, |total, document| {
                total
                    .checked_add(document.text.len_bytes())
                    .context("Workspace document byte count overflow")
            })?;
    for change in wire {
        ensure!(
            change.uri.len() <= MAX_URI_BYTES,
            "Workspace edit URI exceeds 8 KiB"
        );
        // Decode file URIs only lexically. Exact captured untitled URIs are supported.
        let decoded = url::Url::parse(change.uri)?.to_file_path().ok();
        let mut matches = targets.iter().filter(|target| {
            target.uri == change.uri
                || decoded
                    .as_ref()
                    .is_some_and(|path| target.path.as_ref() == Some(path))
        });
        let target = matches.next().context(
            "Code action touches a file outside the synchronized open buffers; open it and retry",
        )?;
        ensure!(matches.next().is_none(), "Ambiguous workspace edit target");
        ensure!(
            seen.insert(target.document),
            "Repeated workspace edit target is not supported"
        );
        let found = documents
            .iter()
            .enumerate()
            .find(|(_, doc)| doc.id == target.document)
            .map(|(index, doc)| (Location::Visible(index), doc))
            .or_else(|| {
                hidden
                    .iter()
                    .enumerate()
                    .find(|(_, doc)| doc.id == target.document)
                    .map(|(index, doc)| (Location::Hidden(index), doc))
            })
            .context("Workspace document was closed or replaced; request code actions again")?;
        let (location, document) = found;
        ensure!(
            document.path == target.path
                && document.revision == target.revision
                && document.text_epoch() == target.text_epoch
                && target_current(target, document)
                && version_current(change.version, target.version)?,
            "Workspace document changed since this request; request code actions again"
        );
        let values = change
            .edits
            .as_array()
            .context("Invalid workspace text edits")?;
        count_edits = count_edits
            .checked_add(values.len())
            .context("Workspace text edit count overflow")?;
        ensure!(
            count_edits <= MAX_EDITS,
            "Code action exceeds 4096 text edits"
        );
        let mut changes = Vec::with_capacity(values.len());
        for value in values {
            ensure!(
                value.get("annotationId").is_none(),
                "Annotated workspace text edits are not supported yet"
            );
            fields(
                value,
                &["range", "newText"],
                "Unsupported workspace text edit fields",
            )?;
            let text = value["newText"]
                .as_str()
                .context("Invalid replacement text")?;
            raw_bytes = raw_bytes
                .checked_add(text.len())
                .context("Workspace replacement byte count overflow")?;
            ensure!(
                raw_bytes <= MAX_REPLACEMENT_BYTES,
                "Code action exceeds 4 MiB replacement text"
            );
            let range = value.get("range").context("Missing workspace edit range")?;
            fields(range, &["start", "end"], "Invalid workspace edit range")?;
            let start = offset(document, position(&range["start"])?)?;
            let end = offset(document, position(&range["end"])?)?;
            ensure!(start <= end, "Language server edit has a reversed range");
            let text = normalized(text, &document.eol, MAX_REPLACEMENT_BYTES - expanded_bytes)?;
            expanded_bytes += text.len();
            changes.push((start..end, text));
        }
        changes.sort_by_key(|(range, _)| (range.start, range.end));
        ensure!(
            changes
                .windows(2)
                .all(|pair| pair[0].0.end <= pair[1].0.start && pair[0].0.start != pair[1].0.start),
            "Language server returned overlapping edits"
        );
        let removed: usize = changes
            .iter()
            .map(|(range, _)| document.text.slice(range.clone()).len_bytes())
            .sum();
        let added: usize = changes.iter().map(|(_, text)| text.len()).sum();
        let final_bytes = document
            .text
            .len_bytes()
            .checked_sub(removed)
            .and_then(|bytes| bytes.checked_add(added))
            .context("Workspace document byte count overflow")?;
        ensure!(
            final_bytes as u64 <= crate::document::MAX_FILE_BYTES,
            "Language server edit exceeds document size limit"
        );
        total_bytes = total_bytes
            .checked_sub(document.text.len_bytes())
            .and_then(|bytes| bytes.checked_add(final_bytes))
            .context("Workspace document byte count overflow")?;
        // Empty edit arrays never create an Undo step or count as a modified buffer.
        if !changes.is_empty() {
            staged.push((location, changes));
        }
    }
    ensure!(
        policy
            .max_total_document_bytes
            .is_none_or(|maximum| total_bytes <= maximum),
        "Extension edit exceeds the 4 MiB mirror budget"
    );
    let outcome = WorkspaceEditOutcome {
        buffers: staged.len(),
        text_edits: count_edits,
    };
    for (location, changes) in staged {
        let document = match location {
            Location::Visible(index) => &mut documents[index],
            Location::Hidden(index) => &mut hidden[index],
        };
        document.apply_changes(changes);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Selection;
    use serde_json::json;

    fn target(document: &Document) -> WorkspaceEditTarget {
        WorkspaceEditTarget {
            uri: document.path.as_deref().map_or_else(
                || format!("untitled:vscli-{}", document.id),
                |path| lsp::file_uri(path).unwrap(),
            ),
            path: document.path.clone(),
            document: document.id,
            revision: document.revision,
            text_epoch: document.text_epoch(),
            version: WorkspaceVersion::Extension(7),
        }
    }

    fn text_edit(start: usize, end: usize, text: &str) -> Value {
        json!({"range":{"start":{"line":0,"character":start},
            "end":{"line":0,"character":end}},"newText":text})
    }

    fn workspace(targets: &[WorkspaceEditTarget], edits: Vec<Vec<Value>>) -> Value {
        json!({"documentChanges":targets.iter().zip(edits).map(|(target,edits)| {
            let version = match target.version {
                WorkspaceVersion::Native(version) => json!(version),
                WorkspaceVersion::Extension(version) => json!(version),
            };
            json!({"textDocument":{"uri":target.uri,"version":version},"edits":edits})
        }).collect::<Vec<_>>()})
    }

    fn apply(
        documents: &mut [Document],
        hidden: &mut [Document],
        edit: &Value,
        targets: &[WorkspaceEditTarget],
    ) -> Result<WorkspaceEditOutcome> {
        apply_workspace_edit(
            documents,
            hidden,
            edit,
            targets,
            WorkspaceEditPolicy {
                require_versions: false,
                max_total_document_bytes: Some(MAX_REPLACEMENT_BYTES),
            },
            |_, _| true,
        )
    }

    fn state(document: &Document) -> Value {
        json!({"id":document.id,"text":document.text.to_string(),"revision":document.revision,
            "epoch":document.text_epoch(),"path":document.path,"selections":document.selections(),
            "dirty":document.dirty(),"saved":document.saved_revision,"eol":document.eol,
            "saveGeneration":document.save_generation()})
    }

    #[test]
    fn unicode_eol_edits_preserve_dirty_visible_hidden_identity_and_one_undo_per_file() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main.cpp");
        let other = root.path().join("other.cpp");
        std::fs::write(&main, "猫🙂 :) emoji\r\n").unwrap();
        std::fs::write(&other, "猫🙂 :) emoji\n").unwrap();
        let mut documents = vec![Document::open(&main).unwrap()];
        let mut hidden = vec![Document::open(&other).unwrap()];
        for document in documents.iter_mut().chain(&mut hidden) {
            document.insert("dirty ", false);
        }
        let targets = vec![target(&documents[0]), target(&hidden[0])];
        let edit = workspace(
            &targets,
            vec![
                vec![text_edit(10, 12, "😺\nextra\r\nend\r")],
                vec![text_edit(10, 12, "😀\r\nextra\rend\n")],
            ],
        );
        assert_eq!(
            apply(&mut documents, &mut hidden, &edit, &targets).unwrap(),
            WorkspaceEditOutcome {
                buffers: 2,
                text_edits: 2
            }
        );
        let expected = [
            "dirty 猫🙂 😺\r\nextra\r\nend\r\n emoji\r\n",
            "dirty 猫🙂 😀\nextra\nend\n emoji\n",
        ];
        for (index, document) in documents.iter_mut().chain(&mut hidden).enumerate() {
            assert_eq!(document.id, targets[index].document);
            assert_eq!(document.text.to_string(), expected[index]);
            document.undo();
            let eol = if index == 0 { "\r\n" } else { "\n" };
            assert_eq!(
                document.text.to_string(),
                format!("dirty 猫🙂 :) emoji{eol}")
            );
            document.undo();
            assert_eq!(document.text.to_string(), format!("猫🙂 :) emoji{eol}"));
            document.redo();
            assert_eq!(
                document.text.to_string(),
                format!("dirty 猫🙂 :) emoji{eol}")
            );
            document.redo();
            assert_eq!(document.text.to_string(), expected[index]);
        }
        assert_eq!(documents.len(), 1);
        assert_eq!(hidden.len(), 1);
        assert_eq!(std::fs::read(main).unwrap(), "猫🙂 :) emoji\r\n".as_bytes());
        assert_eq!(std::fs::read(other).unwrap(), "猫🙂 :) emoji\n".as_bytes());
    }

    #[test]
    fn invalid_second_hidden_target_preserves_every_model_and_redo_without_disk_writes() {
        for variant in 0..5 {
            let root = tempfile::tempdir().unwrap();
            let main = root.path().join("main.cpp");
            let other = root.path().join("other.cpp");
            for path in [&main, &other] {
                std::fs::write(path, "猫🙂 tail\r\n").unwrap();
            }
            let mut documents = vec![Document::open(&main).unwrap()];
            let mut hidden = vec![Document::open(&other).unwrap()];
            for document in documents.iter_mut().chain(&mut hidden) {
                document.move_to(0, false);
                document.insert("dirty ", false);
                document.move_to(0, false);
                document.insert("future ", false);
                document.undo();
                document.move_to(2, false);
                document.move_to(4, true);
                document.secondary.push(Selection::caret(6));
            }
            let before = [state(&documents[0]), state(&hidden[0])];
            let targets = vec![target(&documents[0]), target(&hidden[0])];
            let second = match variant {
                0 => vec![text_edit(7, 8, "split")], // Middle of 🙂 after the dirty prefix.
                1 => vec![text_edit(0, 3, "a"), text_edit(2, 4, "b")],
                2 => {
                    let mut edit = text_edit(0, 0, "a");
                    edit["annotationId"] = json!("review");
                    vec![edit]
                }
                3 => vec![text_edit(0, 0, "a")],
                _ => {
                    let mut edit = text_edit(0, 0, "a");
                    edit["range"]["start"]["line"] = json!(i32::MAX as u64 + 1);
                    vec![edit]
                }
            };
            let mut edit = workspace(&targets, vec![vec![text_edit(0, 0, "changed ")], second]);
            if variant == 3 {
                edit["documentChanges"][1]["textDocument"]["version"] = json!(6);
            }
            assert!(apply(&mut documents, &mut hidden, &edit, &targets).is_err());
            assert_eq!([state(&documents[0]), state(&hidden[0])], before);
            for document in documents.iter_mut().chain(&mut hidden) {
                document.redo();
                assert_eq!(document.text.to_string(), "future dirty 猫🙂 tail\r\n");
                document.undo();
                assert_eq!(document.text.to_string(), "dirty 猫🙂 tail\r\n");
            }
            for path in [&main, &other] {
                assert_eq!(std::fs::read(path).unwrap(), "猫🙂 tail\r\n".as_bytes());
            }
        }
    }

    #[test]
    fn hidden_edit_undo_revision_reuse_cannot_revive_a_captured_target() {
        let mut documents = vec![Document::from_text("visible\r\n")];
        let mut hidden = vec![Document::from_text("hidden🙂\r\n")];
        let targets = vec![target(&documents[0]), target(&hidden[0])];
        let revision = hidden[0].revision;
        hidden[0].insert("transient ", false);
        hidden[0].undo();
        assert_eq!(hidden[0].revision, revision);
        assert_ne!(hidden[0].text_epoch(), targets[1].text_epoch);
        let before = [state(&documents[0]), state(&hidden[0])];
        let edit = workspace(
            &targets,
            vec![
                vec![text_edit(0, 0, "first ")],
                vec![text_edit(0, 0, "second ")],
            ],
        );
        assert!(apply(&mut documents, &mut hidden, &edit, &targets).is_err());
        assert_eq!([state(&documents[0]), state(&hidden[0])], before);
        hidden[0].redo();
        assert_eq!(hidden[0].text.to_string(), "transient hidden🙂\r\n");
    }

    #[test]
    fn paths_replaced_models_aliases_and_distinct_untitled_models_keep_exact_identity() {
        let root = tempfile::tempdir().unwrap();
        let original_path = root.path().join("main.cpp");
        let mut document = Document::from_text("original");
        document.path = Some(original_path.clone());
        let captured = target(&document);
        let edit = workspace(
            std::slice::from_ref(&captured),
            vec![vec![text_edit(0, 0, "changed ")]],
        );
        document.path = Some(root.path().join("renamed.cpp"));
        assert!(
            apply(
                std::slice::from_mut(&mut document),
                &mut [],
                &edit,
                std::slice::from_ref(&captured)
            )
            .is_err()
        );
        assert_eq!(document.text.to_string(), "original");
        let mut replacement = Document::from_text("replacement");
        replacement.path = Some(original_path.clone());
        assert!(
            apply(
                std::slice::from_mut(&mut replacement),
                &mut [],
                &edit,
                std::slice::from_ref(&captured)
            )
            .is_err()
        );
        assert_eq!(replacement.text.to_string(), "replacement");
        document.path = Some(original_path);
        let mut duplicate = edit.clone();
        let mut alias = duplicate["documentChanges"][0].clone();
        alias["textDocument"]["uri"] = json!(captured.uri.replace("main.cpp", "%6dain.cpp"));
        duplicate["documentChanges"]
            .as_array_mut()
            .unwrap()
            .push(alias);
        assert!(
            apply(
                std::slice::from_mut(&mut document),
                &mut [],
                &duplicate,
                std::slice::from_ref(&captured)
            )
            .is_err()
        );
        assert_eq!(document.text.to_string(), "original");
        let mut documents = vec![Document::from_text("first")];
        let mut hidden = vec![Document::from_text("second")];
        let targets = vec![target(&documents[0]), target(&hidden[0])];
        let edit = workspace(&targets[1..], vec![vec![text_edit(0, 6, "changed second")]]);
        apply(&mut documents, &mut hidden, &edit, &targets).unwrap();
        assert_eq!(documents[0].text.to_string(), "first");
        assert_eq!(hidden[0].id, targets[1].document);
        assert_eq!(hidden[0].text.to_string(), "changed second");
        assert_eq!(hidden.len(), 1);
    }

    #[test]
    fn shared_views_map_independent_selections_and_share_a_single_undo() {
        let mut document = Document::from_text("猫🙂 :) emoji\r\n");
        document.activate_view(11);
        document.move_to(9, false);
        document.move_to(11, true);
        document.secondary.push(Selection::caret(6));
        document.activate_view(22);
        document.move_to(1, false);
        document.move_to(3, true);
        let target = target(&document);
        let id = document.id;
        let edit = workspace(
            std::slice::from_ref(&target),
            vec![vec![text_edit(4, 6, "😀")]],
        );
        apply(
            std::slice::from_mut(&mut document),
            &mut [],
            &edit,
            &[target],
        )
        .unwrap();
        assert_eq!(document.id, id);
        assert_eq!(document.text.to_string(), "猫🙂 😀 emoji\r\n");
        assert_eq!(
            (
                document.view_state(Some(11)).cursor,
                document.view_state(Some(11)).anchor
            ),
            (10, Some(8))
        );
        assert_eq!(document.view_state(Some(11)).secondary[0].cursor, 5);
        // A caret at the replacement start follows the inserted text, consistently
        // with Document's existing shared-view mapping policy.
        assert_eq!((document.cursor, document.anchor), (4, Some(1)));
        document.activate_view(11);
        document.undo();
        assert_eq!(document.text.to_string(), "猫🙂 :) emoji\r\n");
        assert_eq!((document.cursor, document.anchor), (11, Some(9)));
        document.redo();
        assert_eq!(document.text.to_string(), "猫🙂 😀 emoji\r\n");
    }

    #[test]
    fn normalized_and_final_aggregate_budgets_include_hidden_unedited_models() {
        let mut document = Document::from_text("x\r\n");
        let target = target(&document);
        let before = state(&document);
        let text = "\n".repeat(MAX_REPLACEMENT_BYTES / 2 + 1);
        let edit = workspace(
            std::slice::from_ref(&target),
            vec![vec![text_edit(0, 0, &text)]],
        );
        let error = apply(
            std::slice::from_mut(&mut document),
            &mut [],
            &edit,
            std::slice::from_ref(&target),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Normalized"));
        assert_eq!(state(&document), before);
        let mut hidden = vec![Document::from_text(&"x".repeat(MAX_REPLACEMENT_BYTES - 3))];
        let edit = workspace(
            std::slice::from_ref(&target),
            vec![vec![text_edit(0, 0, "more")]],
        );
        assert!(
            apply(
                std::slice::from_mut(&mut document),
                &mut hidden,
                &edit,
                std::slice::from_ref(&target)
            )
            .is_err()
        );
        assert_eq!(state(&document), before);
        assert_eq!(hidden[0].text.len_bytes(), MAX_REPLACEMENT_BYTES - 3);
        // Raw bytes and array cardinality are checked before cloning/staging
        // attacker-controlled replacement strings or individual edits.
        let raw = "x".repeat(MAX_REPLACEMENT_BYTES + 1);
        let edit = workspace(
            std::slice::from_ref(&target),
            vec![vec![text_edit(0, 0, &raw)]],
        );
        assert!(
            apply(
                std::slice::from_mut(&mut document),
                &mut [],
                &edit,
                std::slice::from_ref(&target)
            )
            .unwrap_err()
            .to_string()
            .contains("4 MiB replacement")
        );
        assert_eq!(state(&document), before);
        let edit = workspace(
            std::slice::from_ref(&target),
            vec![vec![text_edit(0, 0, ""); MAX_EDITS + 1]],
        );
        assert!(
            apply(
                std::slice::from_mut(&mut document),
                &mut [],
                &edit,
                std::slice::from_ref(&target)
            )
            .unwrap_err()
            .to_string()
            .contains("4096")
        );
        assert_eq!(state(&document), before);
        let mut edit = workspace(std::slice::from_ref(&target), vec![vec![]]);
        let one = edit["documentChanges"][0].clone();
        edit["documentChanges"] = json!(vec![one; MAX_TARGETS + 1]);
        assert!(
            apply(
                std::slice::from_mut(&mut document),
                &mut [],
                &edit,
                std::slice::from_ref(&target)
            )
            .unwrap_err()
            .to_string()
            .contains("128")
        );
        assert_eq!(state(&document), before);
    }

    #[test]
    fn command_versions_remain_explicit_and_transport_versions_cannot_go_stale() {
        let mut document = Document::from_text("same text");
        let mut target = target(&document);
        target.version = WorkspaceVersion::Native(42);
        let direct = json!({"changes":{target.uri.clone():[text_edit(0,0,"prefix ")]}});
        let mut numbered = workspace(
            std::slice::from_ref(&target),
            vec![vec![text_edit(0, 0, "prefix ")]],
        );
        let before = state(&document);
        for edit in [&direct, &{
            let mut edit = numbered.clone();
            edit["documentChanges"][0]["textDocument"]["version"] = Value::Null;
            edit
        }] {
            assert!(
                apply_workspace_edit(
                    std::slice::from_mut(&mut document),
                    &mut [],
                    edit,
                    std::slice::from_ref(&target),
                    WorkspaceEditPolicy {
                        require_versions: true,
                        max_total_document_bytes: None
                    },
                    |_, _| true
                )
                .is_err()
            );
            assert_eq!(state(&document), before);
        }
        numbered["documentChanges"][0]["textDocument"]
            .as_object_mut()
            .unwrap()
            .remove("version");
        assert!(
            apply_workspace_edit(
                std::slice::from_mut(&mut document),
                &mut [],
                &numbered,
                std::slice::from_ref(&target),
                WorkspaceEditPolicy {
                    require_versions: true,
                    max_total_document_bytes: None
                },
                |_, _| true
            )
            .is_err()
        );
        // The model stamp is equal, but the caller observes a newer transport lifetime.
        assert!(
            apply_workspace_edit(
                std::slice::from_mut(&mut document),
                &mut [],
                &direct,
                std::slice::from_ref(&target),
                WorkspaceEditPolicy {
                    require_versions: false,
                    max_total_document_bytes: None
                },
                |_, _| false
            )
            .is_err()
        );
        assert_eq!(state(&document), before);
        apply_workspace_edit(
            std::slice::from_mut(&mut document),
            &mut [],
            &direct,
            &[target],
            WorkspaceEditPolicy {
                require_versions: false,
                max_total_document_bytes: None,
            },
            |_, _| true,
        )
        .unwrap();
        assert_eq!(document.text.to_string(), "prefix same text");
        document.undo();
        assert_eq!(document.text.to_string(), "same text");
    }

    #[test]
    fn empty_target_edits_never_create_undo_or_discard_redo() {
        let mut document = Document::from_text("original");
        document.insert("future ", false);
        document.undo();
        let target = target(&document);
        let before = state(&document);
        let edit = workspace(std::slice::from_ref(&target), vec![vec![]]);
        assert_eq!(
            apply(
                std::slice::from_mut(&mut document),
                &mut [],
                &edit,
                &[target]
            )
            .unwrap(),
            WorkspaceEditOutcome {
                buffers: 0,
                text_edits: 0
            }
        );
        assert_eq!(state(&document), before);
        document.redo();
        assert_eq!(document.text.to_string(), "future original");
        document.undo();
        assert_eq!(document.text.to_string(), "original");
    }
}
