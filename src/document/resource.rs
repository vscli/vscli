//! Native live-model authority and worker-only resource journal materialization.
//! Serialized numeric clocks are recovery evidence, never live model authority.
use super::*;
use crate::resource_journal::{Control, ModelArchive, ModelProof};
use anyhow::ensure;
use serde::ser::SerializeSeq;
mod restore;
pub use restore::{ResourceModelCommit, ResourceModelPrepared};

/// A COW snapshot of an actual native file document. Keep this owner through
/// native authorization; neither a URI nor a recovered ModelProof replaces it.
/// History head identities remain live-only and cannot be rebuilt from the wire.
pub struct ResourceModelSource {
    source: WorkspaceSource,
    proof: ModelProof,
    resource: PathBuf,
    disk: Option<Rope>,
    language: Option<String>,
    language_hint: Option<String>,
    dirty: bool,
}
impl ResourceModelSource {
    pub fn proof(&self) -> &ModelProof {
        &self.proof
    }
    pub fn resource(&self) -> &Path {
        &self.resource
    }
    /// Display/caret movement does not retire source authority. Text/history,
    /// path, language and accepted saved-baseline publications do, including ABA.
    pub fn check_current(&self, document: &Document) -> Result<()> {
        ensure!(
            self.source.current(document)
                && document.language_epoch == self.proof.language_epoch
                && document.language_override == self.language
                && document.language_hint == self.language_hint,
            "Original resource model source changed"
        );
        Ok(())
    }
    /// Call on the retained filesystem worker, using its original control and
    /// remaining aggregate budgets. No disk access and no document mutation.
    /// Full native Undo/Redo serialization is a separate controller obligation.
    pub fn archive(
        &self,
        control: &Control<'_>,
        remaining_bytes: usize,
        remaining_metadata: usize,
    ) -> Result<ModelArchive> {
        control.check()?;
        ensure!(
            remaining_bytes <= crate::resource_journal::MAX_BYTES
                && remaining_metadata <= 8 * 1024 * 1024,
            "Invalid native resource archive bounds"
        );
        let text = self.source.text();
        ensure!(
            text.len_bytes() <= remaining_bytes,
            "Native resource archive text exceeds remaining budget"
        );
        let mut metadata = Bounded {
            bytes: Vec::new(),
            limit: remaining_metadata.min(remaining_bytes - text.len_bytes()),
            control,
        };
        serde_json::to_writer(
            &mut metadata,
            &Metadata {
                version: 1,
                proof: &self.proof,
                eol: self.source.eol(),
                encoding: "utf8",
                language_override: self.language.as_deref(),
                language_hint: self.language_hint.as_deref(),
                dirty: self.dirty,
                disk_content: self.disk.as_ref().map(Chunks),
            },
        )?;
        let mut bytes = Bounded {
            bytes: Vec::new(),
            limit: text.len_bytes(),
            control,
        };
        for chunk in text.chunks() {
            bytes.write_all(chunk.as_bytes())?;
        }
        control.check()?;
        Ok(ModelArchive {
            proof: self.proof.clone(),
            resource: self.resource.clone(),
            text: bytes.bytes,
            metadata: metadata.bytes,
        })
    }
}

impl Document {
    /// Captures only bounded native metadata and COW ropes on the input thread.
    /// Flattening/JSON encoding belong to ResourceModelSource::archive on a worker.
    pub fn capture_resource_model(&self) -> Result<ResourceModelSource> {
        let path = self
            .path
            .as_ref()
            .context("Resource model requires a file path")?;
        ensure!(
            path.is_absolute() && path.to_str().is_some() && path.as_os_str().len() <= 4096,
            "Invalid native resource model path"
        );
        ensure!(
            self.disk_content
                .as_ref()
                .is_none_or(|disk| disk.len_bytes() as u64 <= MAX_FILE_BYTES),
            "Native resource baseline exceeds document budget"
        );
        Ok(ResourceModelSource {
            source: self.capture_workspace_source()?,
            proof: ModelProof {
                document: self.id,
                revision: self.revision,
                text_epoch: self.text_epoch,
                language_epoch: self.language_epoch,
                resource_epoch: self.resource_epoch,
                saved_revision: self.saved_revision,
                save_generation: self.save_generation,
            },
            resource: path.clone(),
            disk: self.disk_content.clone(),
            language: self.language_override.clone(),
            language_hint: self.language_hint.clone(),
            dirty: self.dirty(),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata<'a> {
    version: u32,
    proof: &'a ModelProof,
    eol: &'a str,
    encoding: &'a str,
    language_override: Option<&'a str>,
    language_hint: Option<&'a str>,
    dirty: bool,
    disk_content: Option<Chunks<'a>>,
}
// Preserve exact raw baseline bytes, including mixed EOLs, without allocating a
// second baseline-sized String. Chunk boundaries carry no semantic authority.
struct Chunks<'a>(&'a Rope);
impl Serialize for Chunks<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        for chunk in self.0.chunks() {
            sequence.serialize_element(chunk)?;
        }
        sequence.end()
    }
}
struct Bounded<'a, 'b> {
    bytes: Vec<u8>,
    limit: usize,
    control: &'a Control<'b>,
}
impl Write for Bounded<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.control.check().map_err(std::io::Error::other)?;
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other(
                "Native resource archive budget exceeded",
            ));
        }
        let required = self.bytes.len() + bytes.len();
        if required > self.bytes.capacity() {
            let capacity = required
                .max(self.bytes.capacity().max(64).saturating_mul(2))
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.control.check().map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod tests;
