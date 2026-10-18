//! Worker-prepared model restoration; filesystem/App authority stays separate.
use super::*;
use crate::document::workspace::{ResourceBody, WorkspaceTextPrepared};

pub struct ResourceModelPrepared {
    target: ResourceModelSource,
    text: WorkspaceTextPrepared,
    disk: Rope,
    eol: String,
    language: Option<String>,
    hint: Option<String>,
    dirty: bool,
    deadline: Instant,
}
impl ResourceModelPrepared {
    pub fn document(&self) -> u64 {
        self.target.proof.document
    }
    pub fn bytes(&self) -> usize {
        self.text.bytes()
    }
}
/// An exclusive native destination lease. Preparation may reserve capacity but
/// changes no document text, identity, baseline or history before publish.
pub struct ResourceModelCommit<'a> {
    document: &'a mut Document,
    body: ResourceBody,
    disk: Rope,
    language: Option<String>,
    hint: Option<String>,
    language_epoch: u64,
    save_generation: u64,
    saved_revision: u64,
    dirty: bool,
}
impl ResourceModelCommit<'_> {
    pub fn metadata_bytes(&self) -> usize {
        self.body.metadata + 512
    }
    /// All original-source checks, clocks and allocations precede this move.
    pub fn publish(self) {
        let document = self.document;
        self.body.publish(document);
        if document.language_epoch != self.language_epoch {
            document.retire_typing_pairs();
            document.retire_folding_policy();
        }
        document.language_override = self.language;
        document.language_hint = self.hint;
        document.language_epoch = self.language_epoch;
        document.disk_content = Some(self.disk);
        document.save_generation = self.save_generation;
        document.saved_revision = self.saved_revision;
        document.resource_dirty = self.dirty;
        document.break_group();
    }
}
impl Document {
    pub fn prepare_resource_model_commit<'a>(
        &'a mut self,
        prepared: ResourceModelPrepared,
        control: &Control<'_>,
        available_metadata: usize,
    ) -> Result<ResourceModelCommit<'a>> {
        control.check()?;
        ensure!(
            available_metadata <= crate::document::workspace::MAX_WORKSPACE_METADATA,
            "Invalid native resource metadata budget"
        );
        ensure!(
            control.deadline() == prepared.deadline,
            "Original restoration deadline changed"
        );
        prepared.target.check_current(self)?;
        let save_generation = self
            .save_generation
            .checked_add(1)
            .context("Resource baseline lifetime exhausted")?;
        let language_epoch =
            if self.language_override != prepared.language || self.language_hint != prepared.hint {
                self.language_epoch
                    .checked_add(1)
                    .context("Resource language lifetime exhausted")?
            } else {
                self.language_epoch
            };
        let body = self.prepare_resource_body(
            prepared.text,
            prepared.eol,
            available_metadata
                .checked_sub(512)
                .context("Resource metadata budget exhausted")?,
        )?;
        let saved_revision = if prepared.dirty {
            self.saved_revision
        } else {
            body.revision()
        };
        control.check()?;
        Ok(ResourceModelCommit {
            document: self,
            body,
            disk: prepared.disk,
            language: prepared.language,
            hint: prepared.hint,
            language_epoch,
            save_generation,
            saved_revision,
            dirty: prepared.dirty,
        })
    }
}

impl ResourceModelSource {
    /// CREATE-overwrite/reload of an existing original target. Actual postimage
    /// reads and effect receipts belong to the native filesystem coordinator.
    pub fn prepare_reload(
        self,
        disk: Rope,
        control: &Control<'_>,
    ) -> Result<ResourceModelPrepared> {
        let eol = infer_eol(&disk, self.source.eol()).to_owned();
        let language = self.language.clone();
        let hint = self.language_hint.clone();
        prepare(
            self,
            disk.clone(),
            disk,
            eol,
            language,
            hint,
            false,
            control,
        )
    }
    /// MOVE/COPY into an original surviving target keeps that target's document
    /// and history. Dirty source contents restore; clean source contents reload.
    pub fn prepare_transfer(
        &self,
        target: ResourceModelSource,
        disk: Rope,
        control: &Control<'_>,
    ) -> Result<ResourceModelPrepared> {
        let (text, eol) = if self.dirty {
            (self.source.text().clone(), self.source.eol().to_owned())
        } else {
            (
                disk.clone(),
                infer_eol(&disk, target.source.eol()).to_owned(),
            )
        };
        let (language, hint) = self.restored_language(
            &target.resource,
            target.language.clone(),
            target.language_hint.clone(),
        );
        prepare(target, text, disk, eol, language, hint, self.dirty, control)
    }
    /// A non-surviving destination gets an actual fresh native identity. It
    /// never receives the source model's version/history/identity. App must still
    /// authorize original absence/membership and the real filesystem receipt.
    pub fn restore_new(
        &self,
        path: PathBuf,
        disk: Rope,
        control: &Control<'_>,
    ) -> Result<Document> {
        control.check()?;
        validate_snapshot_path(&path, true)?;
        let text = if self.dirty {
            self.source.text().clone()
        } else {
            disk.clone()
        };
        validate(&text, &disk, control)?;
        let (language, hint) = self.restored_language(&path, None, None);
        let mut document = Document::from_rope(text);
        document.path = Some(path);
        document.disk_content = Some(disk);
        if self.dirty {
            document.eol = self.source.eol().to_owned();
            document.resource_dirty = true;
            // This fresh dirty model has never had a saved alternative version.
            document.saved_revision = u64::MAX;
        }
        document.language_override = language;
        document.language_hint = hint;
        control.check()?;
        Ok(document)
    }
    fn restored_language(
        &self,
        target: &Path,
        mut language: Option<String>,
        mut hint: Option<String>,
    ) -> (Option<String>, Option<String>) {
        let source = self
            .language
            .as_deref()
            .or(self.language_hint.as_deref())
            .unwrap_or_else(|| crate::lsp::language(&self.resource));
        if source != "plaintext" {
            if self.language.is_some() {
                language = Some(source.to_owned());
                hint = None;
            } else if language
                .as_deref()
                .or(hint.as_deref())
                .unwrap_or_else(|| crate::lsp::language(target))
                == "plaintext"
                && target
                    .extension()
                    .is_none_or(|extension| extension != "txt")
            {
                hint = Some(source.to_owned());
            }
        }
        (language, hint)
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "private original destination payload"
)]
fn prepare(
    target: ResourceModelSource,
    text: Rope,
    disk: Rope,
    eol: String,
    language: Option<String>,
    hint: Option<String>,
    dirty: bool,
    control: &Control<'_>,
) -> Result<ResourceModelPrepared> {
    validate(&text, &disk, control)?;
    let original = target.source.text();
    let mut prefix = 0;
    while prefix < original.len_lines().min(text.len_lines())
        && same_line(original.line(prefix), text.line(prefix), control)?
    {
        prefix += 1;
    }
    let remaining = original.len_lines().min(text.len_lines()) - prefix;
    let mut suffix = 0;
    while suffix < remaining
        && same_line(
            original.line(original.len_lines() - suffix - 1),
            text.line(text.len_lines() - suffix - 1),
            control,
        )?
    {
        suffix += 1;
    }
    let edits = if prefix == original.len_lines() && prefix == text.len_lines() {
        Vec::new()
    } else {
        // Pinned ModelService._computeEdits uses whole-line common prefix/suffix
        // and includes the last common line's boundary when there is no suffix.
        // Raw EOL differences remain real native edits rather than lost bytes.
        let (old_range, new_range) = if suffix > 0 {
            (
                original.line_to_char(prefix)..original.line_to_char(original.len_lines() - suffix),
                text.line_to_char(prefix)..text.line_to_char(text.len_lines() - suffix),
            )
        } else if prefix > 0 {
            (
                line_end(original, prefix - 1)..original.len_chars(),
                line_end(&text, prefix - 1)..text.len_chars(),
            )
        } else {
            (0..original.len_chars(), 0..text.len_chars())
        };
        vec![(old_range, text.slice(new_range).to_string())]
    };
    let mut prepared =
        target
            .source
            .clone()
            .prepare(edits, control.cancel_flag(), control.deadline())?;
    prepared.force_resource_markers();
    // Retain the original restored backing after calculating native edit maps.
    // Clean model/baseline publication can then share this allocation as well.
    prepared.use_resource_text(text);
    control.check()?;
    Ok(ResourceModelPrepared {
        target,
        text: prepared,
        disk,
        eol,
        language,
        hint,
        dirty,
        deadline: control.deadline(),
    })
}
fn line_end(text: &Rope, line: usize) -> usize {
    let value = text.line(line);
    let trailing = value
        .chars_at(value.len_chars())
        .reversed()
        .take_while(|character| matches!(character, '\r' | '\n'))
        .count();
    text.line_to_char(line) + value.len_chars() - trailing
}
fn same_line(
    a: ropey::RopeSlice<'_>,
    b: ropey::RopeSlice<'_>,
    control: &Control<'_>,
) -> Result<bool> {
    control.check()?;
    if a.len_bytes() != b.len_bytes() {
        return Ok(false);
    }
    for (index, (a, b)) in a
        .chunks()
        .flat_map(str::bytes)
        .zip(b.chunks().flat_map(str::bytes))
        .enumerate()
    {
        if index % 4096 == 0 {
            control.check()?;
        }
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn validate(text: &Rope, disk: &Rope, control: &Control<'_>) -> Result<()> {
    control.check()?;
    // A clean reload shares one immutable Rope for model and baseline. Charge
    // that backing once; independent dirty contents/baselines remain additive.
    let baseline_bytes = if text.is_instance(disk) {
        0
    } else {
        disk.len_bytes()
    };
    ensure!(
        text.len_bytes()
            .checked_add(baseline_bytes)
            .is_some_and(|bytes| bytes <= crate::resource_journal::MAX_BYTES),
        "Restored text/baseline exceed shared resource budget"
    );
    for chunk in text.chunks().chain(disk.chunks()) {
        control.check()?;
        ensure!(
            !chunk.contains('\0'),
            "Binary resource cannot restore a native text model"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
