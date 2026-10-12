use super::*;

/// Caller-selected empty-file encoding. The journal never guesses a BOM or
/// drops absent contents into an arbitrary encoding. A UTF-8 empty file is [].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncodedEmpty {
    pub requested: PathBuf,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    /// True requests real OS trash. No trash backend exists here: refuse before
    /// effects unless the individual delete explicitly skips trash.
    pub use_trash: bool,
    pub encoded_empty: Vec<EncodedEmpty>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub max_steps: usize,
    pub max_record_bytes: usize,
    /// Additional private parked/staged trees, on top of original payloads.
    pub max_private_bytes: usize,
}
impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_steps: 16384,
            max_record_bytes: 16 * 1024 * 1024,
            max_private_bytes: MAX_BYTES,
        }
    }
}
impl ExecutionLimits {
    pub(super) fn validate(self) -> Result<()> {
        ensure!(
            self.max_steps > 0
                && self.max_steps <= 16384
                && self.max_record_bytes > 0
                && self.max_record_bytes <= 16 * 1024 * 1024
                && self.max_private_bytes <= MAX_BYTES,
            "Invalid execution bounds"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenameRole {
    Source,
    Displace,
    Delete,
    PublishCopy,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PhysicalAction {
    CreateFile {
        path: PathBuf,
        bytes: usize,
        sha256: [u8; 32],
    },
    CreateDirectory {
        path: PathBuf,
    },
    Rename {
        source: PathBuf,
        target: PathBuf,
        role: RenameRole,
        preimage: Vec<NodeImage>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepReceipt {
    pub index: usize,
    pub operation: usize,
    pub action: PhysicalAction,
    pub postimage: Vec<NodeImage>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationReceipt {
    pub operation: usize,
    pub skipped: bool,
    /// Pinned delete maxSize can suppress its public inverse; internal durable
    /// preimages and rollback authority are retained regardless.
    pub delete_inverse_suppressed: bool,
}
#[derive(Clone, Debug)]
pub struct ExecutionOutcome {
    pub state: ExecutionState,
    pub steps: Vec<StepReceipt>,
    pub operations: Vec<OperationReceipt>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    pub version: u32,
    pub original_sha256: [u8; 32],
    pub journal: PathBuf,
    pub identity: Identity,
    pub policy: ExecutionPolicy,
    pub limits: ExecutionLimits,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Record {
    Intent {
        index: usize,
        operation: usize,
        action: PhysicalAction,
    },
    Applied(StepReceipt),
    Operation(OperationReceipt),
    RollbackIntent {
        index: usize,
    },
    RolledBack {
        index: usize,
    },
    Terminal {
        state: ExecutionState,
        error: Option<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub version: u32,
    pub sequence: usize,
    pub previous_sha256: [u8; 32],
    pub record: Record,
}
