use super::*;
use super::{
    execution_records::{Envelope, Header, Record},
    world::World,
};
use std::{collections::BTreeSet, fs};

/// Worker-only owner. The UI/coordinator must authorize its original model and
/// filesystem captures before calling authorize; this object does not forge App
/// proof authority or spawn a worker. Effects remain inspectable after retirement.
#[derive(Debug)]
pub struct Executor {
    staged: StagedJournal,
    world: World,
    policy: ExecutionPolicy,
    limits: ExecutionLimits,
    state: ExecutionState,
    steps: Vec<StepReceipt>,
    operations: Vec<OperationReceipt>,
    rolled_back: usize,
    sequence: usize,
    previous: [u8; 32],
    record_bytes: usize,
    rollback_reserved: usize,
    batch_ignored: BTreeSet<usize>,
    prepared_batch: Option<std::ops::Range<usize>>,
    private_bytes: usize,
    error: Option<String>,
}
impl Executor {
    pub fn authorize(
        staged: StagedJournal,
        policy: ExecutionPolicy,
        limits: ExecutionLimits,
        control: &Control<'_>,
    ) -> Result<Self> {
        limits.validate()?;
        control.check()?;
        ensure!(
            !staged.capture.legacy_archive_proofs,
            "Legacy model archive proof is inspection-only"
        );
        let recovered = staged.recovery(control)?;
        ensure!(
            !recovered.capture().has_legacy_archive_proofs(),
            "Legacy model archive proof is inspection-only"
        );
        drop(recovered);
        validate_policy(&staged.capture, &policy)?;
        ensure!(
            cfg!(target_os = "linux"),
            "Native exclusive resource execution currently has a Linux backend only"
        );
        let mut world = World::originals(&staged.capture)?;
        world.bind(control)?;
        let root = read(
            &staged.directory.join("manifest.json"),
            staged.capture.limits.max_manifest_bytes,
            control,
        )?;
        let header = Header {
            version: 2,
            original_sha256: filesystem::digest(&root),
            journal: staged.directory.clone(),
            identity: staged.identity.clone(),
            policy: policy.clone(),
            limits,
        };
        let bytes = staging::encode(&header, staged.capture.limits.max_manifest_bytes)?;
        ensure!(
            bytes.len() + TERMINAL_RESERVE <= limits.max_record_bytes,
            "Execution header/terminal exceed record budget"
        );
        staged.verify_directory(control)?;
        staging::write(
            &staged.directory.join("execution.json"),
            &bytes,
            staged.capture.limits.max_manifest_bytes,
            control,
        )?;
        Ok(Self {
            staged,
            world,
            policy,
            limits,
            state: ExecutionState::Executing,
            steps: Vec::new(),
            operations: Vec::new(),
            rolled_back: 0,
            sequence: 0,
            previous: filesystem::digest(&bytes),
            record_bytes: bytes.len(),
            rollback_reserved: 0,
            batch_ignored: BTreeSet::new(),
            prepared_batch: None,
            private_bytes: 0,
            error: None,
        })
    }
    pub fn path(&self) -> &Path {
        self.staged.path()
    }
    pub fn originals(&self) -> &Capture {
        self.staged.capture()
    }
    pub fn state(&self) -> ExecutionState {
        self.state
    }
    pub fn verify_current(&self, control: &Control<'_>) -> Result<()> {
        ensure!(
            self.state != ExecutionState::Ambiguous,
            "Interrupted intent has no automatic recovery authority"
        );
        self.staged.verify_directory(control)?;
        self.world.verify(control)
    }
    pub fn outcome(&self) -> ExecutionOutcome {
        ExecutionOutcome {
            state: self.state,
            steps: self.steps.clone(),
            operations: self.operations.clone(),
            error: self.error.clone(),
        }
    }
    /// Executes actual original operation order. A stopped aggregate may contain
    /// applied physical steps; an ambiguous aggregate may additionally contain an
    /// unacknowledged effect. Neither is reported as a no-effect rejection.
    pub fn execute_authorized(&mut self, control: &Control<'_>) -> ExecutionOutcome {
        if self.state != ExecutionState::Executing
            || !self.steps.is_empty()
            || !self.operations.is_empty()
        {
            self.error = Some("Execution cannot be replayed or resumed implicitly".into());
            return self.outcome();
        }
        for index in execution_schedule(&self.staged.capture.operations) {
            if let Err(error) = self.prepare_batch(index) {
                self.state = ExecutionState::Stopped;
                self.error = Some(reason(error));
                self.terminal(control);
                return self.outcome();
            }
            let operation = self.staged.capture.operations[index].clone();
            if let Err(error) = self.run_operation(index, &operation, control) {
                if self.state != ExecutionState::Ambiguous {
                    self.state = ExecutionState::Stopped;
                }
                self.error = Some(reason(error));
                self.terminal(control);
                return self.outcome();
            }
        }
        self.state = ExecutionState::Completed;
        self.terminal(control);
        self.outcome()
    }
    fn prepare_batch(&mut self, operation: usize) -> Result<()> {
        if self
            .prepared_batch
            .as_ref()
            .is_some_and(|batch| batch.contains(&operation))
        {
            return Ok(());
        }
        let kind = batch_kind(&self.staged.capture.operations[operation]);
        let mut start = operation;
        while start > 0 && batch_kind(&self.staged.capture.operations[start - 1]) == kind {
            start -= 1;
        }
        self.batch_ignored.clear();
        let mut end = start;
        while end < self.staged.capture.operations.len()
            && batch_kind(&self.staged.capture.operations[end]) == kind
        {
            end += 1;
        }
        for index in start..end {
            let input = &self.staged.capture.operations[index];
            if batch_kind(input) != kind {
                break;
            }
            let options = input.options();
            let target = self.canonical(input.paths().last().unwrap())?;
            match input {
                Operation::Create { .. } => {
                    if !options.overwrite.is_present()
                        && truth(&options.ignore_if_exists)
                        && self.world.nodes.contains_key(&target)
                    {
                        self.batch_ignored.insert(index);
                    }
                }
                Operation::Move { source, .. } => {
                    let source = self.canonical(source)?;
                    if source == target
                        || (!options.overwrite.is_present()
                            && truth(&options.ignore_if_exists)
                            && self.world.nodes.contains_key(&target))
                    {
                        self.batch_ignored.insert(index);
                    } else {
                        let source_node = self
                            .world
                            .nodes
                            .get(&source)
                            .context("Move/copy batch source is absent")?;
                        ensure!(
                            !source.starts_with(&target) && !target.starts_with(&source),
                            "Move/copy batch trees overlap"
                        );
                        ensure!(
                            truth(&options.copy) || !source_node.permissions.readonly,
                            "Readonly move batch source"
                        );
                        if let Some(target_node) = self.world.nodes.get(&target) {
                            ensure!(
                                truth(&options.overwrite) && !target_node.permissions.readonly,
                                "Move/copy batch overwrite is not authorized"
                            );
                        }
                    }
                }
                Operation::Delete { .. } => {}
            }
        }
        self.prepared_batch = Some(start..end);
        Ok(())
    }
    fn canonical(&self, requested: &Path) -> Result<PathBuf> {
        self.staged
            .capture
            .resources
            .iter()
            .find(|resource| resource.requested == requested)
            .map(|resource| resource.canonical.clone())
            .context("Original operation resource proof missing")
    }
    fn run_operation(
        &mut self,
        index: usize,
        operation: &Operation,
        control: &Control<'_>,
    ) -> Result<()> {
        self.verify_current(control)?;
        control.check()?;
        let options = operation.options();
        let mut skipped = false;
        let mut delete_inverse_suppressed = false;
        match operation {
            Operation::Create {
                path: requested,
                contents,
                ..
            } => {
                let target = self.canonical(requested)?;
                if self.batch_ignored.contains(&index) {
                    skipped = true;
                } else if truth(&options.folder) {
                    if let Some(node) = self.world.nodes.get(&target) {
                        ensure!(
                            node.kind == NodeKind::Directory,
                            "Folder create target is a file"
                        );
                    } else {
                        self.parents(index, &target, control)?;
                        self.mkdir(index, target, control)?;
                    }
                } else {
                    let bytes = match contents {
                        Some(bytes) => bytes.clone(),
                        None => self
                            .policy
                            .encoded_empty
                            .iter()
                            .find(|empty| empty.requested == *requested)
                            .context("Absent contents have no authorized empty-file encoding")?
                            .bytes
                            .clone(),
                    };
                    if let Some(node) = self.world.nodes.get(&target) {
                        ensure!(
                            truth(&options.overwrite) && node.kind == NodeKind::File,
                            "Create overwrite is not authorized for this destination"
                        );
                        self.park(index, target.clone(), RenameRole::Displace, control)?;
                    }
                    self.parents(index, &target, control)?;
                    self.create(index, target, &bytes, control)?;
                }
            }
            Operation::Move {
                source: requested_source,
                destination: requested_target,
                ..
            } => {
                let source = self.canonical(requested_source)?;
                let target = self.canonical(requested_target)?;
                if self.batch_ignored.contains(&index) {
                    skipped = true;
                } else {
                    ensure!(
                        !source.starts_with(&target) && !target.starts_with(&source),
                        "Move/copy trees overlap"
                    );
                    let original = self
                        .world
                        .nodes
                        .get(&source)
                        .context("Move/copy source is absent")?;
                    ensure!(
                        truth(&options.copy) || !original.permissions.readonly,
                        "Readonly move source"
                    );
                    if self.world.nodes.contains_key(&target) {
                        ensure!(
                            truth(&options.overwrite),
                            "Move/copy overwrite is not authorized"
                        );
                    }
                    self.ensure_park_volume(&source)?;
                    self.ensure_park_volume_for_target(&target)?;
                    let scratch = if truth(&options.copy) {
                        let scratch = self.staged.directory.join(format!("scratch-{index}"));
                        let preimage = self.world.subtree(&source);
                        self.private_charge(&preimage)?;
                        let bytes = preimage.iter().map(|node| node.bytes).sum();
                        self.reserve_new(preimage.len(), bytes, &scratch)?;
                        self.world.preview_copy(&source, &scratch)?;
                        self.staged.verify_directory(control)?;
                        let copied = effects::copy_tree(
                            &source,
                            &scratch,
                            &preimage,
                            self.staged.capture.limits,
                            control,
                        )?;
                        self.world.add_nodes(&copied)?;
                        self.world.bind(control)?;
                        Some(scratch)
                    } else {
                        None
                    };
                    if self.world.nodes.contains_key(&target) {
                        self.park(index, target.clone(), RenameRole::Displace, control)?;
                    }
                    self.parents(index, &target, control)?;
                    self.move_nodes(
                        index,
                        scratch.unwrap_or(source),
                        target,
                        if truth(&options.copy) {
                            RenameRole::PublishCopy
                        } else {
                            RenameRole::Source
                        },
                        control,
                    )?;
                }
            }
            Operation::Delete {
                path: requested, ..
            } => {
                let target = self.canonical(requested)?;
                if let Some(node) = self.world.nodes.get(&target) {
                    ensure!(!node.permissions.readonly, "Readonly delete target");
                    ensure!(
                        node.kind != NodeKind::Directory
                            || node.children.is_empty()
                            || truth(&options.recursive),
                        "Delete of non-empty directory requires recursive"
                    );
                    let stat = filesystem::open(&target, node.kind == NodeKind::Directory)?;
                    ensure!(
                        filesystem::identity(&stat)? == node.identity,
                        "Delete stat identity changed"
                    );
                    let stat_size = stat.metadata()?.len();
                    delete_inverse_suppressed = !truth(&options.folder)
                        && options
                            .max_size
                            .value()
                            .is_some_and(|maximum| stat_size as f64 > *maximum);
                    self.park(index, target, RenameRole::Delete, control)?;
                } else {
                    ensure!(
                        truth(&options.ignore_if_not_exists),
                        "Delete target is absent"
                    );
                    skipped = true;
                }
            }
        }
        let receipt = OperationReceipt {
            operation: index,
            skipped,
            delete_inverse_suppressed,
        };
        self.append(Record::Operation(receipt.clone()), control)?;
        self.operations.push(receipt);
        Ok(())
    }
    fn ensure_park_volume(&self, source: &Path) -> Result<()> {
        let node = self
            .world
            .nodes
            .get(source)
            .context("Resource source missing")?;
        ensure!(
            effects::same_volume(&node.identity, &self.staged.identity),
            "Cross-volume parking/copy has no qualified backend"
        );
        Ok(())
    }
    fn ensure_park_volume_for_target(&self, target: &Path) -> Result<()> {
        let mut parent = target.parent().context("Target parent missing")?;
        while !parent.exists() {
            parent = parent.parent().context("Target ancestor missing")?;
        }
        let id = filesystem::identity(&filesystem::open(parent, true)?)?;
        ensure!(
            effects::same_volume(&id, &self.staged.identity),
            "Cross-volume target has no qualified backend"
        );
        Ok(())
    }
    fn private_charge(&mut self, nodes: &[NodeImage]) -> Result<()> {
        for node in nodes {
            add(
                &mut self.private_bytes,
                node.bytes,
                self.limits.max_private_bytes,
                "private retained bytes",
            )?;
        }
        Ok(())
    }
    fn reserve_new(&self, count: usize, bytes: usize, path: &Path) -> Result<()> {
        super::path(path)?;
        self.world.reserve_new(count, bytes, path)
    }
    fn parents(&mut self, operation: usize, target: &Path, control: &Control<'_>) -> Result<()> {
        let mut missing = Vec::new();
        let mut metadata_bytes = 0;
        let mut parent = target
            .parent()
            .context("Target requires parent")?
            .to_owned();
        loop {
            control.check()?;
            match fs::symlink_metadata(&parent) {
                Ok(stat) => {
                    ensure!(
                        stat.is_dir() && !stat.file_type().is_symlink(),
                        "Target parent is not ordinary directory"
                    );
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    super::path(&parent)?;
                    ensure!(
                        missing.len() < self.staged.capture.limits.max_nodes,
                        "Parent creation node budget exceeded"
                    );
                    add(
                        &mut metadata_bytes,
                        parent.as_os_str().len() + 24,
                        self.staged.capture.limits.max_manifest_bytes,
                        "created ancestor paths",
                    )?;
                    missing.push(parent.clone());
                    parent = parent
                        .parent()
                        .context("Target ancestor missing")?
                        .to_owned();
                }
                Err(error) => return Err(error.into()),
            }
        }
        self.ensure_park_volume_for_target(target)?;
        for parent in missing.into_iter().rev() {
            self.mkdir(operation, parent, control)?;
        }
        Ok(())
    }
    fn mkdir(&mut self, operation: usize, path: PathBuf, control: &Control<'_>) -> Result<()> {
        self.reserve_new(1, 0, &path)?;
        let action = PhysicalAction::CreateDirectory { path: path.clone() };
        self.intent(operation, &action, control)?;
        let result = effects::create_directory(&path, control);
        self.finish_effect(operation, action, result.map(|node| vec![node]), control)
    }
    fn create(
        &mut self,
        operation: usize,
        path: PathBuf,
        bytes: &[u8],
        control: &Control<'_>,
    ) -> Result<()> {
        self.reserve_new(1, bytes.len(), &path)?;
        let action = PhysicalAction::CreateFile {
            path: path.clone(),
            bytes: bytes.len(),
            sha256: filesystem::digest(bytes),
        };
        self.intent(operation, &action, control)?;
        let result =
            effects::create_file(&path, bytes, self.staged.capture.limits.max_bytes, control);
        self.finish_effect(operation, action, result.map(|node| vec![node]), control)
    }
    fn park(
        &mut self,
        operation: usize,
        source: PathBuf,
        role: RenameRole,
        control: &Control<'_>,
    ) -> Result<()> {
        self.ensure_park_volume(&source)?;
        ensure!(
            !self
                .world
                .nodes
                .get(&source)
                .context("Park source missing")?
                .permissions
                .readonly,
            "Readonly displaced/deleted target"
        );
        let nodes = self.world.subtree(&source);
        self.private_charge(&nodes)?;
        let target = self
            .staged
            .directory
            .join(format!("parked-{}", self.steps.len()));
        self.move_nodes(operation, source, target, role, control)
    }
    fn move_nodes(
        &mut self,
        operation: usize,
        source: PathBuf,
        target: PathBuf,
        role: RenameRole,
        control: &Control<'_>,
    ) -> Result<()> {
        self.world.preview_move(&source, &target)?;
        let preimage = self.world.subtree(&source);
        let action = PhysicalAction::Rename {
            source: source.clone(),
            target: target.clone(),
            role,
            preimage: preimage.clone(),
        };
        self.intent(operation, &action, control)?;
        let result = effects::rename(
            &source,
            &target,
            &preimage,
            self.staged.capture.limits,
            control,
        );
        self.finish_effect(operation, action, result, control)
    }
    fn intent(
        &mut self,
        operation: usize,
        action: &PhysicalAction,
        control: &Control<'_>,
    ) -> Result<()> {
        ensure!(
            self.steps.len() < self.limits.max_steps,
            "Physical step budget exceeded"
        );
        self.verify_current(control)?;
        let action_bytes = staging::encode(action, self.staged.capture.limits.max_manifest_bytes)?;
        let extra = match action {
            PhysicalAction::Rename { preimage, .. } => preimage.len() * 512,
            _ => 8192,
        };
        let reserve = action_bytes
            .len()
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(extra + 8192))
            .context("Execution receipt budget overflow")?;
        let remaining = self
            .limits
            .max_record_bytes
            .saturating_sub(self.record_bytes)
            .saturating_sub(self.rollback_reserved)
            .saturating_sub(TERMINAL_RESERVE);
        ensure!(
            reserve + ROLLBACK_RESERVE <= remaining,
            "Execution intent/receipt/rollback records exceed remaining budget"
        );
        ensure!(
            action_bytes.len() * 3 + extra <= self.staged.capture.limits.max_manifest_bytes,
            "Execution applied receipt exceeds single-record budget"
        );
        self.append(
            Record::Intent {
                index: self.steps.len(),
                operation,
                action: action.clone(),
            },
            control,
        )?;
        self.state = ExecutionState::Ambiguous;
        Ok(())
    }
    fn finish_effect(
        &mut self,
        operation: usize,
        action: PhysicalAction,
        result: Result<Vec<NodeImage>>,
        control: &Control<'_>,
    ) -> Result<()> {
        let postimage = result?;
        match &action {
            PhysicalAction::Rename { source, target, .. } => self.world.moved(source, target)?,
            _ => self.world.add_nodes(&postimage)?,
        }
        self.world.bind(control)?;
        let receipt = StepReceipt {
            index: self.steps.len(),
            operation,
            action,
            postimage,
        };
        self.append(Record::Applied(receipt.clone()), control)?;
        self.steps.push(receipt);
        self.rollback_reserved += ROLLBACK_RESERVE;
        self.state = ExecutionState::Executing;
        Ok(())
    }
    fn append(&mut self, record: Record, control: &Control<'_>) -> Result<()> {
        let envelope = Envelope {
            version: 2,
            sequence: self.sequence,
            previous_sha256: self.previous,
            record,
        };
        let bytes = staging::encode(&envelope, self.staged.capture.limits.max_manifest_bytes)?;
        ensure!(
            bytes.len()
                <= self
                    .limits
                    .max_record_bytes
                    .saturating_sub(self.record_bytes),
            "Execution record retention exceeds budget"
        );
        self.staged.verify_directory(control)?;
        staging::write(
            &self
                .staged
                .directory
                .join(format!("record-{}.json", self.sequence)),
            &bytes,
            self.staged.capture.limits.max_manifest_bytes,
            control,
        )?;
        self.previous = filesystem::digest(&bytes);
        self.sequence += 1;
        self.record_bytes += bytes.len();
        Ok(())
    }
    fn terminal(&mut self, control: &Control<'_>) {
        if self.state == ExecutionState::Ambiguous {
            return;
        }
        if let Err(error) = self.append(
            Record::Terminal {
                state: self.state,
                error: self.error.clone(),
            },
            control,
        ) {
            self.error = Some(reason(error));
            self.state = ExecutionState::Ambiguous;
        }
    }
    /// Internal recovery rollback, newest PHYSICAL effect first. This is not the
    /// pinned public WorkspaceEdit Undo grouping/inverse-loss policy.
    pub fn rollback_authorized(&mut self, control: &Control<'_>) -> ExecutionOutcome {
        if let Err(error) = self.verify_current(control) {
            self.error = Some(reason(error));
            return self.outcome();
        }
        if !matches!(
            self.state,
            ExecutionState::Completed | ExecutionState::Stopped | ExecutionState::RollingBack
        ) {
            self.error = Some("Journal has no acknowledged rollback authority".into());
            return self.outcome();
        }
        self.state = ExecutionState::RollingBack;
        while self.rolled_back < self.steps.len() {
            let index = self.steps.len() - self.rolled_back - 1;
            let receipt = self.steps[index].clone();
            let result = (|| -> Result<()> {
                self.staged.verify_directory(control)?;
                self.world.verify(control)?;
                self.append(Record::RollbackIntent { index }, control)?;
                self.state = ExecutionState::Ambiguous;
                match &receipt.action {
                    PhysicalAction::Rename { source, target, .. } => {
                        let nodes = self.world.subtree(target);
                        effects::rename(
                            target,
                            source,
                            &nodes,
                            self.staged.capture.limits,
                            control,
                        )?;
                        self.world.moved(target, source)?;
                    }
                    PhysicalAction::CreateFile { path, .. }
                    | PhysicalAction::CreateDirectory { path } => {
                        let node = self
                            .world
                            .nodes
                            .get(path)
                            .context("Rollback created node missing")?
                            .clone();
                        effects::remove_created(&node, self.staged.capture.limits, control)?;
                        self.world.remove_nodes(path)?;
                    }
                }
                self.world.bind(control)?;
                self.append(Record::RolledBack { index }, control)?;
                self.rolled_back += 1;
                self.rollback_reserved = self.rollback_reserved.saturating_sub(ROLLBACK_RESERVE);
                self.state = ExecutionState::RollingBack;
                Ok(())
            })();
            if let Err(error) = result {
                self.error = Some(reason(error));
                return self.outcome();
            }
        }
        self.state = ExecutionState::RolledBack;
        self.error = None;
        self.terminal(control);
        self.outcome()
    }
}
const TERMINAL_RESERVE: usize = 32768;
const ROLLBACK_RESERVE: usize = 1024;
/// Source CREATE batches execute folders first, preserving original order inside
/// each partition and all surrounding operation-kind boundaries. The caller must
/// split filesystem phases at text/notebook boundaries before constructing Capture.
fn batch_kind(operation: &Operation) -> u8 {
    match operation {
        Operation::Create { .. } => 0,
        Operation::Move { options, .. } if truth(&options.copy) => 1,
        Operation::Move { .. } => 2,
        Operation::Delete { .. } => 3,
    }
}
fn execution_schedule(operations: &[Operation]) -> Vec<usize> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < operations.len() {
        if matches!(operations[index], Operation::Create { .. }) {
            let start = index;
            while index < operations.len() && matches!(operations[index], Operation::Create { .. })
            {
                index += 1;
            }
            output
                .extend((start..index).filter(|index| truth(&operations[*index].options().folder)));
            output.extend(
                (start..index).filter(|index| !truth(&operations[*index].options().folder)),
            );
        } else {
            output.push(index);
            index += 1;
        }
    }
    output
}
fn truth(value: &Presence<bool>) -> bool {
    matches!(value, Presence::Value(true))
}
fn validate_policy(capture: &Capture, policy: &ExecutionPolicy) -> Result<()> {
    ensure!(
        policy.encoded_empty.len() <= capture.limits.max_resources,
        "Empty-encoding inventory exceeds budget"
    );
    let mut seen = BTreeSet::new();
    for empty in &policy.encoded_empty {
        path(&empty.requested)?;
        ensure!(
            seen.insert(empty.requested.clone()) && empty.bytes.len() <= 4,
            "Invalid/duplicate encoded empty file"
        );
        ensure!(capture.operations.iter().any(|operation|matches!(operation,Operation::Create{path,contents:None,..} if path==&empty.requested)),"Encoded empty resource has no original absent-contents create");
    }
    for operation in &capture.operations {
        if matches!(operation, Operation::Delete { .. }) {
            ensure!(
                !policy.use_trash || truth(&operation.options().skip_trash_bin),
                "OS trash backend is unavailable"
            );
        }
        if let Operation::Create {
            path,
            contents: None,
            options,
        } = operation
        {
            ensure!(
                truth(&options.folder)
                    || policy
                        .encoded_empty
                        .iter()
                        .any(|empty| empty.requested == *path),
                "Absent create contents require captured empty-file encoding"
            );
        }
    }
    Ok(())
}
fn reason(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].into()
}
fn read(path: &Path, limit: usize, control: &Control<'_>) -> Result<Vec<u8>> {
    let mut file = filesystem::open(path, false)?;
    filesystem::read(&mut file, path, limit, control)
}

/// Closed, bounded inspection of an execution journal. A pending intent is
/// ALWAYS Ambiguous, even if current paths happen to resemble its preimage.
/// Reading evidence never automatically authorizes retries or recovery writes.
#[derive(Debug)]
pub struct ExecutionRecovery {
    executor: Executor,
    pending: bool,
}
impl ExecutionRecovery {
    pub fn load(directory: &Path, limits: Limits, control: &Control<'_>) -> Result<Self> {
        control.check()?;
        let stored = RecoveryAuthority::load_stored(directory, limits, control, false)?;
        let header_bytes = read(
            &directory.join("execution.json"),
            limits.max_manifest_bytes,
            control,
        )?;
        let header: Header =
            serde_json::from_slice(&header_bytes).context("Malformed execution header")?;
        ensure!(
            header.version == 2
                && header.journal == stored.directory
                && header.identity == stored.identity,
            "Execution header binding/version mismatch"
        );
        header.limits.validate()?;
        ensure!(
            !stored.capture.legacy_archive_proofs,
            "Legacy archives cannot authorize execution recovery"
        );
        validate_policy(&stored.capture, &header.policy)?;
        let root = read(
            &directory.join("manifest.json"),
            limits.max_manifest_bytes,
            control,
        )?;
        ensure!(
            filesystem::digest(&root) == header.original_sha256,
            "Original manifest changed after execution admission"
        );
        let staged = StagedJournal {
            directory: stored.directory,
            identity: stored.identity,
            directory_handle: stored.directory_handle,
            location: stored.location,
            parent_handle: stored.parent_handle,
            capture: stored.capture,
        };
        let world = World::originals(&staged.capture)?;
        let mut executor = Executor {
            staged,
            world,
            policy: header.policy,
            limits: header.limits,
            state: ExecutionState::NotStarted,
            steps: Vec::new(),
            operations: Vec::new(),
            rolled_back: 0,
            sequence: 0,
            previous: filesystem::digest(&header_bytes),
            record_bytes: header_bytes.len(),
            rollback_reserved: 0,
            batch_ignored: BTreeSet::new(),
            prepared_batch: None,
            private_bytes: 0,
            error: None,
        };
        let mut records = BTreeMap::new();
        let mut count = 0;
        let mut name_bytes = 0;
        for entry in fs::read_dir(directory)? {
            control.check()?;
            count += 1;
            ensure!(
                count
                    <= executor.limits.max_steps * 4
                        + limits.max_operations
                        + limits.max_nodes
                        + limits.max_archives * 2
                        + 16,
                "Execution directory entry budget exceeded"
            );
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Non-UTF8 journal entry"))?;
            add(
                &mut name_bytes,
                name.len() + 24,
                limits.max_manifest_bytes,
                "execution directory names",
            )?;
            if let Some(number) = name
                .strip_prefix("record-")
                .and_then(|number| number.strip_suffix(".json"))
            {
                let index: usize = number.parse().context("Malformed execution record name")?;
                ensure!(
                    name == format!("record-{index}.json")
                        && records.insert(index, entry.path()).is_none(),
                    "Duplicate/noncanonical execution record name"
                );
            }
        }
        let mut pending: Option<(usize, usize, PhysicalAction)> = None;
        let mut undo_pending: Option<usize> = None;
        let mut terminal = false;
        for (sequence, path) in records {
            ensure!(sequence == executor.sequence, "Execution record gap");
            let bytes = read(&path, limits.max_manifest_bytes, control)?;
            add(
                &mut executor.record_bytes,
                bytes.len(),
                executor.limits.max_record_bytes,
                "execution records",
            )?;
            let envelope: Envelope =
                serde_json::from_slice(&bytes).context("Malformed execution record")?;
            ensure!(
                envelope.version == 2
                    && envelope.sequence == sequence
                    && envelope.previous_sha256 == executor.previous,
                "Execution record chain mismatch"
            );
            match envelope.record {
                Record::Intent {
                    index,
                    operation,
                    action,
                } => {
                    ensure!(
                        !terminal
                            && pending.is_none()
                            && undo_pending.is_none()
                            && executor.rolled_back == 0
                            && index == executor.steps.len()
                            && index < executor.limits.max_steps
                            && execution_schedule(&executor.staged.capture.operations)
                                .get(executor.operations.len())
                                == Some(&operation)
                            && operation < executor.staged.capture.operations.len(),
                        "Out-of-order execution intent"
                    );
                    executor.prepare_batch(operation)?;
                    validate_action(&executor, operation, index, &action)?;
                    pending = Some((index, operation, action));
                    executor.state = ExecutionState::Ambiguous;
                }
                Record::Applied(receipt) => {
                    let (index, operation, action) = pending
                        .take()
                        .context("Applied receipt without original intent")?;
                    ensure!(
                        receipt.index == index
                            && receipt.operation == operation
                            && receipt.action == action,
                        "Applied receipt changed original intent"
                    );
                    replay_applied(&mut executor, &receipt)?;
                    executor.steps.push(receipt);
                    executor.rollback_reserved += ROLLBACK_RESERVE;
                    executor.state = ExecutionState::Stopped;
                }
                Record::Operation(receipt) => {
                    ensure!(
                        !terminal
                            && pending.is_none()
                            && undo_pending.is_none()
                            && executor.rolled_back == 0
                            && execution_schedule(&executor.staged.capture.operations)
                                .get(executor.operations.len())
                                == Some(&receipt.operation)
                            && receipt.operation < executor.staged.capture.operations.len(),
                        "Out-of-order operation receipt"
                    );
                    executor.prepare_batch(receipt.operation)?;
                    executor.operations.push(receipt);
                    executor.state = ExecutionState::Stopped;
                }
                Record::RollbackIntent { index } => {
                    ensure!(
                        pending.is_none()
                            && undo_pending.is_none()
                            && executor.rolled_back < executor.steps.len()
                            && index == executor.steps.len() - executor.rolled_back - 1,
                        "Out-of-order rollback intent"
                    );
                    terminal = false;
                    undo_pending = Some(index);
                    executor.state = ExecutionState::Ambiguous;
                }
                Record::RolledBack { index } => {
                    ensure!(
                        undo_pending.take() == Some(index),
                        "Rollback receipt without matching intent"
                    );
                    let receipt = executor.steps[index].clone();
                    replay_rollback(&mut executor.world, &receipt)?;
                    executor.rolled_back += 1;
                    executor.rollback_reserved =
                        executor.rollback_reserved.saturating_sub(ROLLBACK_RESERVE);
                    executor.state = ExecutionState::RollingBack;
                }
                Record::Terminal { state, error } => {
                    ensure!(
                        pending.is_none() && undo_pending.is_none() && !terminal,
                        "Terminal record cannot dismiss an unacknowledged intent"
                    );
                    ensure!(
                        matches!(
                            state,
                            ExecutionState::Completed
                                | ExecutionState::Stopped
                                | ExecutionState::RolledBack
                        ),
                        "Invalid terminal state"
                    );
                    ensure!(
                        state != ExecutionState::Completed
                            || executor.operations.len()
                                == executor.staged.capture.operations.len(),
                        "Incomplete completed execution"
                    );
                    ensure!(
                        state != ExecutionState::RolledBack
                            || executor.rolled_back == executor.steps.len(),
                        "Incomplete acknowledged rollback"
                    );
                    ensure!(
                        error.as_ref().is_none_or(|value| value.len() <= 4096),
                        "Oversized execution error"
                    );
                    executor.state = state;
                    executor.error = error;
                    terminal = true;
                }
            }
            executor.previous = filesystem::digest(&bytes);
            executor.sequence += 1;
        }
        executor.staged.verify_directory(control)?;
        let pending = pending.is_some() || undo_pending.is_some();
        if pending {
            executor.state = ExecutionState::Ambiguous;
        }
        Ok(Self { executor, pending })
    }
    pub fn state(&self) -> ExecutionState {
        self.executor.state
    }
    pub fn outcome(&self) -> ExecutionOutcome {
        self.executor.outcome()
    }
    pub fn originals(&self) -> &Capture {
        self.executor.originals()
    }
    pub fn has_unacknowledged_intent(&self) -> bool {
        self.pending
    }
    pub fn verify_current(&self, control: &Control<'_>) -> Result<()> {
        self.executor.verify_current(control)
    }
    /// Requires a fresh explicit native recovery authorization from the caller;
    /// only acknowledged states with unchanged owned postimages are admitted.
    pub fn into_rollback_authorized(mut self, control: &Control<'_>) -> Result<Executor> {
        ensure!(
            !self.pending
                && matches!(
                    self.executor.state,
                    ExecutionState::Completed
                        | ExecutionState::Stopped
                        | ExecutionState::RollingBack
                ),
            "Recovery cannot automatically settle an ambiguous/not-started journal"
        );
        self.executor.verify_current(control)?;
        self.executor.world.bind(control)?;
        Ok(self.executor)
    }
}
fn validate_action(
    executor: &Executor,
    operation: usize,
    index: usize,
    action: &PhysicalAction,
) -> Result<()> {
    let input = &executor.staged.capture.operations[operation];
    let endpoints = input
        .paths()
        .into_iter()
        .map(|path| executor.canonical(path))
        .collect::<Result<Vec<_>>>()?;
    let parent_target = match input {
        Operation::Create { .. } => Some(&endpoints[0]),
        Operation::Move { .. } => Some(&endpoints[1]),
        Operation::Delete { .. } => None,
    };
    match action {
        PhysicalAction::CreateDirectory { path } => {
            super::path(path)?;
            ensure!(
                parent_target.is_some_and(|target| target.starts_with(path))
                    && !executor.world.nodes.contains_key(path),
                "Unrelated/existing mkdir intent"
            );
        }
        PhysicalAction::CreateFile {
            path,
            bytes,
            sha256,
        } => {
            super::path(path)?;
            let Operation::Create {
                path: requested,
                contents,
                options,
            } = input
            else {
                anyhow::bail!("File creation intent has wrong original operation")
            };
            ensure!(
                !truth(&options.folder)
                    && path == &endpoints[0]
                    && !executor.world.nodes.contains_key(path),
                "File create intent target mismatch"
            );
            let contents = contents
                .as_deref()
                .or_else(|| {
                    executor
                        .policy
                        .encoded_empty
                        .iter()
                        .find(|empty| empty.requested == *requested)
                        .map(|empty| empty.bytes.as_slice())
                })
                .context("Original create contents missing")?;
            ensure!(
                *bytes == contents.len() && *sha256 == filesystem::digest(contents),
                "Create intent contents changed"
            );
        }
        PhysicalAction::Rename {
            source,
            target,
            role,
            preimage,
        } => {
            super::path(source)?;
            super::path(target)?;
            ensure!(
                !source.starts_with(target)
                    && !target.starts_with(source)
                    && !executor.world.nodes.contains_key(target),
                "Invalid rename intent paths"
            );
            let parked = executor.staged.directory.join(format!("parked-{index}"));
            match role {
                RenameRole::Source => ensure!(
                    matches!(input,Operation::Move{options,..} if !truth(&options.copy))
                        && source == &endpoints[0]
                        && target == &endpoints[1],
                    "Source move intent differs from original operation"
                ),
                RenameRole::Displace => ensure!(
                    matches!(input,Operation::Create{options,..}|Operation::Move{options,..} if truth(&options.overwrite))
                        && source == endpoints.last().unwrap()
                        && *target == parked,
                    "Displacement differs from original operation"
                ),
                RenameRole::Delete => ensure!(
                    matches!(input, Operation::Delete { .. })
                        && source == &endpoints[0]
                        && *target == parked,
                    "Delete intent differs from original operation"
                ),
                RenameRole::PublishCopy => {
                    ensure!(
                        matches!(input,Operation::Move{options,..} if truth(&options.copy))
                            && *source
                                == executor
                                    .staged
                                    .directory
                                    .join(format!("scratch-{operation}"))
                            && target == &endpoints[1],
                        "Copy publication intent differs from original operation"
                    );
                    let originals = executor.world.subtree(&endpoints[0]);
                    ensure!(
                        originals.len() == preimage.len(),
                        "Copy tree inventory changed"
                    );
                    for (original, copy) in originals.iter().zip(preimage) {
                        ensure!(
                            copy.path == relocated(&original.path, &endpoints[0], source)?
                                && copy.kind == original.kind
                                && copy.bytes == original.bytes
                                && copy.sha256 == original.sha256
                                && copy.children == original.children
                                && copy.permissions == original.permissions,
                            "Copy tree differs from authorized source"
                        );
                    }
                }
            }
            ensure!(
                !preimage.is_empty() && preimage.len() <= executor.staged.capture.limits.max_nodes,
                "Invalid rename preimage bound"
            );
            if *role != RenameRole::PublishCopy {
                ensure!(
                    *preimage == executor.world.subtree(source),
                    "Rename preimage changed original world"
                );
            }
            for node in preimage {
                super::path(&node.path)?;
                ensure!(
                    node.path.starts_with(source),
                    "Rename preimage outside source"
                );
            }
        }
    }
    Ok(())
}
fn replay_applied(executor: &mut Executor, receipt: &StepReceipt) -> Result<()> {
    match &receipt.action {
        PhysicalAction::Rename {
            source,
            target,
            role,
            preimage,
        } => {
            let expected = preimage
                .iter()
                .map(|node| {
                    let mut node = node.clone();
                    node.path = relocated(&node.path, source, target)?;
                    Ok(node)
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                receipt.postimage == expected,
                "Rename receipt is not the original owned identity/postimage"
            );
            if *role == RenameRole::PublishCopy {
                executor.world.add_nodes(preimage)?;
            }
            executor.world.moved(source, target)?;
        }
        PhysicalAction::CreateFile {
            path,
            bytes,
            sha256,
        } => {
            ensure!(
                receipt.postimage.len() == 1,
                "File receipt inventory invalid"
            );
            let node = &receipt.postimage[0];
            ensure!(
                node.path == *path
                    && node.kind == NodeKind::File
                    && node.bytes == *bytes
                    && node.sha256 == Some(*sha256)
                    && node.children.is_empty(),
                "File receipt postimage mismatch"
            );
            executor.world.add_nodes(&receipt.postimage)?;
        }
        PhysicalAction::CreateDirectory { path } => {
            ensure!(
                receipt.postimage.len() == 1,
                "Directory receipt inventory invalid"
            );
            let node = &receipt.postimage[0];
            ensure!(
                node.path == *path
                    && node.kind == NodeKind::Directory
                    && node.bytes == 0
                    && node.sha256.is_none()
                    && node.children.is_empty(),
                "Directory receipt postimage mismatch"
            );
            executor.world.add_nodes(&receipt.postimage)?;
        }
    }
    executor.world.budget()
}
fn replay_rollback(world: &mut World, receipt: &StepReceipt) -> Result<()> {
    match &receipt.action {
        PhysicalAction::Rename { source, target, .. } => world.moved(target, source),
        PhysicalAction::CreateFile { path, .. } | PhysicalAction::CreateDirectory { path } => {
            world.remove_nodes(path)
        }
    }
}
