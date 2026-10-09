use super::*;
use crate::{
    extension_activation::{
        self as lifecycle, Event as ActivationEvent, Metadata, Preferences, Scope, state::Paths,
    },
    extension_store::Store,
    extensions::{ActivationRequest, Package},
};
use anyhow::Context;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc,
        mpsc::{Receiver, sync_channel},
    },
};
const MAX_COMMANDS: usize = 8;
#[derive(Default)]
pub(super) struct State {
    configured: bool,
    config_root: Option<PathBuf>,
    generation: u64,
    control: u64,
    loaded: bool,
    refresh: bool,
    paused: bool,
    global: Preferences,
    workspace: Preferences,
    catalog: Arc<BTreeMap<String, Metadata>>,
    commands: Vec<(String, String)>,
    notices: Vec<String>,
    host_bindings: Vec<(String, Value)>,
    pub(super) owners: BTreeMap<String, String>,
    automatic: BTreeSet<String>,
    attempted: BTreeSet<String>,
    languages: BTreeSet<String>,
    pending: VecDeque<Command>,
    next_command: u64,
    waiting: Option<(String, u64)>,
    last_session: Option<u64>,
    starting: bool,
    job: Option<Job>,
    change: Option<Change>,
}
struct Change {
    id: String,
    enabled: bool,
    scope: Scope,
    config: Option<PathBuf>,
    workspace: PathBuf,
    directory: PathBuf,
    control: u64,
    generation: u64,
}
#[derive(PartialEq)]
struct CommandContext {
    workspace: PathBuf,
    document: Option<(u64, u64, Option<PathBuf>)>,
    selections: Vec<crate::document::Selection>,
    pane: Option<u64>,
    focus: Focus,
}
impl CommandContext {
    fn capture(app: &App) -> Result<Self> {
        if app
            .active_document()
            .is_some_and(|doc| doc.secondary.len() >= 4096)
        {
            anyhow::bail!("Pending extension commands support at most 4096 selections");
        }
        Ok(Self {
            workspace: app.workspace.root.clone(),
            document: app
                .active_document()
                .map(|doc| (doc.id, doc.revision, doc.path.clone())),
            selections: app
                .active_document()
                .map_or_else(Vec::new, Document::selections),
            pane: app.panes.get(app.active_pane).map(|pane| pane.id),
            focus: app.focus.clone(),
        })
    }
}
struct Command {
    token: u64,
    context: CommandContext,
    id: String,
    args: Option<Value>,
}
struct Job {
    command: Option<u64>,
    generation: u64,
    control: u64,
    extension_epoch: Option<u64>,
    receiver: Receiver<Result<Output, String>>,
}
struct Snapshot {
    global: Preferences,
    workspace: Preferences,
    catalog: BTreeMap<String, Metadata>,
    matched: BTreeSet<String>,
    notices: Vec<String>,
}
enum Output {
    Loaded(Box<Snapshot>, Option<(String, bool)>),
    Planned(lifecycle::Plan, String),
}
fn load(
    config: Option<&Path>,
    workspace: &Path,
    directory: Option<&Path>,
    retained: &[Package],
) -> Result<Snapshot> {
    let paths = Paths::new(config, workspace)?;
    let (global, local) = paths.read()?;
    let enabled = Preferences::effective(&global, &local);
    let mut catalog = BTreeMap::new();
    let mut notices = Vec::new();
    if let Some(directory) = directory {
        for installed in Store::new(directory.into()).list()? {
            if catalog.len() >= lifecycle::MAX_ENABLED {
                anyhow::bail!("Extension activation catalog exceeds 128 installed packages");
            }
            match Metadata::installed(&installed) {
                Ok(metadata) => {
                    if enabled.contains(&installed.id) {
                        if let Some(reason) = &metadata.blocked {
                            notices.push(format!("{}: {reason}", installed.id));
                        }
                        if let Some(event) = metadata.unsupported_events.first() {
                            notices.push(format!(
                                "{}: unsupported activation event {event}",
                                installed.id
                            ));
                        }
                    }
                    catalog.insert(installed.id, metadata);
                }
                Err(error) => {
                    if notices.len() < 128 {
                        notices.push(format!("{}: {error:#}", installed.id));
                    }
                }
            }
        }
    }
    // Running/selected generations keep their manifest semantics through registry upgrades.
    for package in retained {
        catalog.insert(package.id.clone(), Metadata::selected(package)?);
    }
    if catalog.len() > lifecycle::MAX_ENABLED {
        anyhow::bail!(
            "Extension activation catalog exceeds 128 packages including retained snapshots"
        );
    }
    let matches = lifecycle::workspace::scan(workspace, &catalog, &enabled);
    notices.extend(matches.notices);
    notices.truncate(128);
    Ok(Snapshot {
        global,
        workspace: local,
        catalog,
        matched: matches.owners,
        notices,
    })
}
impl App {
    /// Always use the native user root, never a repository file or copied import profile.
    pub fn configure_extension_activation(&mut self, config_root: Option<&Path>) {
        self.activation.configured = true;
        self.activation.config_root = config_root.map(Path::to_owned);
        self.refresh_extension_catalog();
    }
    pub fn extension_enabled(&self, id: &str) -> bool {
        self.activation.loaded
            && Preferences::enabled(
                &self.activation.global,
                &self.activation.workspace,
                &id.to_ascii_lowercase(),
            )
    }
    pub fn extension_activation_notices(&self) -> &[String] {
        &self.activation.notices
    }
    pub fn extension_activation_status(&self, id: &str) -> String {
        if let Some(state) = self
            .extension_host
            .as_ref()
            .and_then(|host| host.activation_states.get(id))
        {
            return state.clone();
        }
        if !self.activation.loaded {
            return "enablement loading or unavailable".into();
        }
        if self.extension_enabled(id) {
            "enabled; waiting for a supported event".into()
        } else {
            "disabled".into()
        }
    }
    pub fn refresh_extension_catalog(&mut self) {
        if !self.activation.configured {
            return;
        }
        self.activation.generation += 1;
        self.activation.refresh = true;
    }
    pub fn set_extension_enabled(&mut self, id: &str, enabled: bool, scope: Scope) -> Result<()> {
        let id = id.to_ascii_lowercase();
        lifecycle::validate_id(&id)?;
        if !self.activation.configured {
            anyhow::bail!("Native extension configuration has not been configured");
        }
        if self.activation.config_root.is_none() {
            anyhow::bail!(
                "Native user configuration is unavailable; remembered enablement was not changed"
            );
        }
        if self.activation.change.is_some() {
            anyhow::bail!("An extension enablement intent is already queued");
        }
        let directory = self
            .extensions_directory
            .clone()
            .context("No extension storage directory available")?;
        self.activation.control += 1;
        self.activation.change = Some(Change {
            id: id.clone(),
            enabled,
            scope,
            config: self.activation.config_root.clone(),
            workspace: self.workspace.root.clone(),
            directory,
            control: self.activation.control,
            generation: self.activation.generation,
        });
        self.message = format!(
            "Saving {} enablement for {id}…",
            if enabled { "enabled" } else { "disabled" }
        );
        Ok(())
    }
    fn start_enablement_change(&mut self, change: Change) {
        let Change {
            id,
            enabled,
            scope,
            config,
            workspace,
            directory,
            control,
            generation,
        } = change;
        let retained = self.extension_host.as_ref().map_or_else(
            || self.extension_packages.clone(),
            |host| host.packages.clone(),
        );
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| {
                Store::new(directory.clone()).get(&id)?;
                let paths = Paths::new(config.as_deref(), &workspace)?;
                lifecycle::state::change(paths.path(scope)?, &id, enabled)?;
                load(config.as_deref(), &workspace, Some(&directory), &retained)
                    .map(|snapshot| Output::Loaded(Box::new(snapshot), Some((id, enabled))))
                    .map_err(|error| anyhow::anyhow!("Enablement saved; refresh failed and automatic activation is unavailable: {error:#}"))
            })().map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        self.activation.job = Some(Job {
            command: None,
            generation,
            control,
            extension_epoch: None,
            receiver,
        });
    }
    pub(super) fn pause_automatic_extensions(&mut self) {
        self.activation.control += 1;
        self.activation.paused = true;
        self.activation.pending.clear();
        self.activation.automatic.clear();
        self.activation.waiting = None;
        self.activation.starting = false;
        self.keymap.clear_extension_bindings();
    }
    pub(super) fn resume_automatic_extensions(&mut self) {
        self.activation.pending.clear();
        self.activation.waiting = None;
        self.activation.automatic.clear();
        self.activation.control += 1;
        self.activation.paused = false;
        self.activation.attempted.clear();
        self.activation.languages.clear();
        self.activation.last_session = None;
        self.activation.starting = self.extension_job.as_ref().is_some_and(|job| job.startup());
        if self.activation.configured {
            self.refresh_extension_catalog();
        }
    }
    pub(super) fn queue_extension_command(&mut self, command: &str, args: &Option<Value>) -> bool {
        if !self.activation.configured
            || (!self.activation.loaded
                && self
                    .extension_host
                    .as_ref()
                    .is_some_and(|host| host.ready && host.command_owner(command).is_some()))
            || (self.activation.loaded && !self.activation.owners.contains_key(command))
        {
            return false;
        }
        if self.activation.paused {
            self.message =
                "Automatic extension activation is paused; restart or explicitly enable a package"
                    .into();
            return true;
        }
        if self.activation.pending.len() >= MAX_COMMANDS {
            self.message = "Extension activation command queue limit reached (8)".into();
            return true;
        }
        let size = serde_json::to_vec(args).map_or(usize::MAX, |value| value.len());
        let retained: usize = self
            .activation
            .pending
            .iter()
            .map(|command| {
                serde_json::to_vec(&command.args).map_or(usize::MAX, |value| value.len())
            })
            .sum();
        if size > 64 * 1024 || retained.saturating_add(size) > 256 * 1024 {
            self.message =
                "Extension activation command arguments exceed 64 KiB each / 256 KiB queued".into();
            return true;
        }
        let context = match CommandContext::capture(self) {
            Ok(context) => context,
            Err(error) => {
                self.message = error.to_string();
                return true;
            }
        };
        let Some(token) = self.activation.next_command.checked_add(1) else {
            self.message = "Extension command ticket limit reached".into();
            return true;
        };
        self.activation.next_command = token;
        self.activation.pending.push_back(Command {
            token,
            context,
            id: command.into(),
            args: args.clone(),
        });
        self.message = format!("Waiting to activate extension command: {command}");
        true
    }
    pub(super) fn invalidate_pending_extension_commands(&mut self) {
        if self.activation.pending.is_empty() {
            return;
        }
        let current = CommandContext::capture(self);
        let before = self.activation.pending.len();
        self.activation.pending.retain(|command| {
            current
                .as_ref()
                .is_ok_and(|context| context == &command.context)
        });
        if self.activation.waiting.as_ref().is_some_and(|(_, token)| {
            !self
                .activation
                .pending
                .iter()
                .any(|command| command.token == *token)
        }) {
            self.activation.waiting = None;
        }
        if before != self.activation.pending.len() {
            self.message =
                "Pending extension command canceled because its native editor context changed"
                    .into();
        }
    }
    pub(super) fn extension_dormant_commands(&self) -> &[(String, String)] {
        if self.activation.paused || !self.activation.loaded {
            &[]
        } else {
            &self.activation.commands
        }
    }
    fn publish_activation_catalog(&mut self, snapshot: Snapshot) {
        self.activation.global = snapshot.global;
        self.activation.workspace = snapshot.workspace;
        self.activation.catalog = Arc::new(snapshot.catalog);
        self.activation.loaded = true;
        self.activation.automatic = snapshot.matched;
        self.activation.attempted.clear();
        self.activation.languages.clear();
        let enabled = Preferences::effective(&self.activation.global, &self.activation.workspace);
        let mut commands = BTreeMap::new();
        let mut owners = BTreeMap::new();
        let mut bytes = 0;
        let mut limited = false;
        let reserved = native_command_ids();
        for (owner, item) in self.activation.catalog.iter() {
            if !enabled.contains(owner) {
                continue;
            }
            if item.matches(&ActivationEvent::StartupFinished) {
                self.activation.automatic.insert(owner.clone());
            }
            for event in &item.events {
                if let ActivationEvent::Command(id) = event {
                    if reserved.contains(id) || id.starts_with("cursor") || owners.contains_key(id)
                    {
                        continue;
                    }
                    if owners.len() >= 1024 || bytes + id.len() + owner.len() > 256 * 1024 {
                        limited = true;
                        continue;
                    }
                    bytes += id.len() + owner.len();
                    owners.insert(id.clone(), owner.clone());
                }
            }
            for (id, title) in &item.commands {
                if owners.get(id) != Some(owner)
                    || reserved.contains(id)
                    || id.starts_with("cursor")
                    || commands.contains_key(id)
                {
                    continue;
                }
                let title: String = title.chars().take(128).collect();
                let label = format!("Extension: {title} [{owner}]");
                if commands.len() >= 1024 || bytes + label.len() + id.len() > 256 * 1024 {
                    limited = true;
                    break;
                }
                bytes += label.len() + id.len();
                commands.insert(id.clone(), (label, owner.clone()));
            }
        }
        self.activation.commands = commands
            .iter()
            .map(|(id, (label, _))| (label.clone(), id.clone()))
            .collect();
        self.activation.owners = owners;
        self.activation.notices = snapshot.notices;
        if limited && self.activation.notices.len() < 128 {
            self.activation.notices.push("Dormant command catalog reached its 1024-command / 256 KiB budget; excess commands are unavailable".into());
        }
        self.refresh_dormant_bindings();
        if let Some(notice) = self.activation.notices.first() {
            self.message = format!("Extension activation notice: {notice}");
        }
    }
    pub(super) fn retain_extension_bindings(&mut self, sets: Vec<(String, Value)>) -> bool {
        if !self.activation.configured {
            return false;
        }
        self.activation.host_bindings = sets;
        self.refresh_dormant_bindings();
        true
    }
    pub(super) fn clear_active_extension_bindings(&mut self) {
        self.activation.host_bindings.clear();
    }
    pub(super) fn refresh_dormant_bindings(&mut self) {
        if self.activation.paused {
            return;
        }
        let mut sets: BTreeMap<_, _> = self.activation.host_bindings.iter().cloned().collect();
        let size = |value: &Value| {
            value
                .as_array()
                .map_or(usize::from(value.is_object()), Vec::len)
        };
        let mut count: usize = sets.values().map(size).sum();
        for (owner, item) in self.activation.catalog.iter() {
            if !self.extension_enabled(owner) || sets.contains_key(owner) {
                continue;
            }
            let additional = size(&item.keybindings);
            if count + additional > 1024 {
                continue;
            }
            count += additional;
            sets.insert(owner.clone(), item.keybindings.clone());
        }
        match self
            .keymap
            .set_extension_binding_sets(sets.into_iter().collect())
        {
            Ok(errors) if !errors.is_empty() => {
                self.message = format!(
                    "Dormant extension keybindings rejected: {}",
                    errors.join("; ")
                );
            }
            Err(error) => {
                self.message = format!("Dormant extension keybindings rejected: {error:#}");
            }
            _ => {}
        }
    }
    fn start_activation_read(&mut self) {
        let config = self.activation.config_root.clone();
        let root = self.workspace.root.clone();
        let directory = self.extensions_directory.clone();
        let retained = self.extension_host.as_ref().map_or_else(
            || self.extension_packages.clone(),
            |host| host.packages.clone(),
        );
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || {
            let _ = sender.send(
                load(config.as_deref(), &root, directory.as_deref(), &retained)
                    .map(|s| Output::Loaded(Box::new(s), None))
                    .map_err(|e| format!("{e:#}")),
            );
        });
        self.activation.refresh = false;
        self.activation.job = Some(Job {
            command: None,
            generation: self.activation.generation,
            control: self.activation.control,
            extension_epoch: None,
            receiver,
        });
    }
    pub(super) fn poll_extension_activation(&mut self) -> bool {
        self.invalidate_pending_extension_commands();
        if !self.activation.configured {
            return false;
        }
        let mut changed = false;
        if let Some(job) = &self.activation.job {
            match job.receiver.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                result => {
                    let job = self.activation.job.take().unwrap();
                    if job.generation == self.activation.generation {
                        changed = true;
                        match result.unwrap_or_else(|_| {
                            Err("Extension activation worker disconnected".into())
                        }) {
                            Err(error) => {
                                if job.extension_epoch.is_none() {
                                    self.activation.loaded = false;
                                    self.activation.pending.clear();
                                } else if job.command.as_ref().is_some_and(|id| {
                                    self.activation
                                        .pending
                                        .front()
                                        .is_some_and(|command| &command.token == id)
                                }) {
                                    self.activation.pending.pop_front();
                                }
                                self.message = format!("Extension activation failed: {error}");
                                self.activation.waiting = None;
                            }
                            Ok(Output::Loaded(snapshot, grant)) => {
                                self.publish_activation_catalog(*snapshot);
                                if let Some((id, enabled)) = grant {
                                    let resume = job.control == self.activation.control;
                                    if !enabled
                                        && !self.extension_enabled(&id)
                                        && self.extension_packages.iter().any(|p| p.id == id)
                                    {
                                        self.stop_extension_host();
                                        self.extension_packages.clear();
                                        self.activation.last_session = None;
                                    }
                                    if resume {
                                        self.activation.paused = false;
                                        self.refresh_dormant_bindings();
                                    }
                                    self.message = format!(
                                        "{} {id}; workspace enablement overrides global state",
                                        if enabled { "Enabled" } else { "Disabled" }
                                    );
                                }
                            }
                            Ok(Output::Planned(plan, owner)) => {
                                if job.extension_epoch == Some(self.extension_epoch)
                                    && job.control == self.activation.control
                                    && !self.activation.paused
                                {
                                    let command = job.command.as_ref().is_some_and(|id| {
                                        self.activation
                                            .pending
                                            .front()
                                            .is_some_and(|command| &command.token == id)
                                    });
                                    if let Err(error) =
                                        self.apply_activation_plan(plan, &owner, command)
                                    {
                                        self.message =
                                            format!("Cannot activate {owner}: {error:#}");
                                        if command {
                                            self.activation.pending.pop_front();
                                        }
                                        self.activation.waiting = None;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if self.activation.job.is_none()
            && let Some(change) = self.activation.change.take()
        {
            self.start_enablement_change(change);
            return true;
        }
        if self.activation.job.is_none() && self.activation.refresh {
            self.start_activation_read();
            return true;
        }
        if !self.activation.loaded || self.activation.paused {
            return changed;
        }
        if let Some(host) = &self.extension_host {
            self.activation.last_session = Some(host.session);
            if host.ready {
                self.activation.starting = false;
            }
        } else if (self.activation.last_session.is_some() || self.activation.starting)
            && !self.extension_job.as_ref().is_some_and(|job| job.startup())
        {
            self.pause_automatic_extensions();
            self.message =
                "Extension session stopped; automatic retry is paused until restart or enable"
                    .into();
            return true;
        }
        if self.extension_job.is_some()
            || self.extension_retirement.is_some()
            || self.activation.job.is_some()
            || self
                .extension_host
                .as_ref()
                .is_some_and(|host| !host.ready || host.activating())
        {
            return changed;
        }
        if let Some((owner, token)) = self.activation.waiting.take() {
            if !self
                .activation
                .pending
                .front()
                .is_some_and(|command| command.token == token)
            {
                return true;
            }
            if let Some(command) = self.activation.pending.pop_front() {
                if self
                    .extension_host
                    .as_ref()
                    .and_then(|host| host.activation_states.get(&owner))
                    .is_some_and(|state| state == "active")
                {
                    self.execute_extension_now(&command.id, command.args);
                } else {
                    self.message = format!(
                        "Extension {owner} failed to activate; restart explicitly to retry"
                    );
                }
            }
            changed = true;
        }
        let languages: BTreeSet<_> = self
            .documents
            .iter()
            .map(|doc| {
                doc.path
                    .as_deref()
                    .map_or("plaintext", crate::lsp::language)
                    .to_owned()
            })
            .collect();
        if languages != self.activation.languages {
            self.activation.languages = languages.clone();
            for (id, item) in self.activation.catalog.iter() {
                if self.extension_enabled(id)
                    && languages
                        .iter()
                        .any(|language| item.matches(&ActivationEvent::Language(language.clone())))
                {
                    self.activation.automatic.insert(id.clone());
                }
            }
        }
        let owner = if let Some(command) = self.activation.pending.front() {
            if let Some(owner) = self.activation.owners.get(&command.id) {
                owner.clone()
            } else {
                let command = self.activation.pending.pop_front().unwrap();
                self.execute_extension_now(&command.id, command.args);
                return true;
            }
        } else {
            let next = self
                .activation
                .automatic
                .iter()
                .find(|id| !self.activation.attempted.contains(*id))
                .cloned();
            let Some(next) = next else {
                return changed;
            };
            next
        };
        if let Some(host) = &self.extension_host
            && let Some(state) = host.activation_states.get(&owner)
        {
            if state == "active" {
                self.activation.attempted.insert(owner);
                if let Some(command) = self.activation.pending.pop_front() {
                    self.execute_extension_now(&command.id, command.args);
                }
                return true;
            }
            if state == "failed" {
                self.activation.attempted.insert(owner.clone());
                self.activation.pending.pop_front();
                self.message = format!("Extension {owner} failed; restart explicitly to retry");
                return true;
            }
        }
        self.activation.attempted.insert(owner.clone());
        let catalog = self.activation.catalog.clone();
        let packages = self.extension_host.as_ref().map_or_else(
            || self.extension_packages.clone(),
            |host| host.packages.clone(),
        );
        let mut enabled =
            Preferences::effective(&self.activation.global, &self.activation.workspace);
        enabled.extend(packages.iter().map(|package| package.id.clone()));
        let root = owner.clone();
        let (sender, receiver) = sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| {
                let retained: Vec<_> = packages
                    .iter()
                    .map(Metadata::selected)
                    .collect::<Result<_>>()?;
                lifecycle::plan(
                    &catalog,
                    &enabled,
                    &retained,
                    &BTreeSet::from([root.clone()]),
                )
                .map(|plan| Output::Planned(plan, root))
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        self.activation.job = Some(Job {
            command: self.activation.pending.front().map(|command| command.token),
            generation: self.activation.generation,
            control: self.activation.control,
            extension_epoch: Some(self.extension_epoch),
            receiver,
        });
        self.message = format!("Planning activation for {owner}…");
        true
    }
    fn apply_activation_plan(
        &mut self,
        plan: lifecycle::Plan,
        owner: &str,
        command: bool,
    ) -> Result<()> {
        if !plan.needs_code() {
            self.message = format!("Declarative extension {owner} needs no JavaScript runtime");
            if command {
                self.activation.pending.pop_front();
                self.message =
                    format!("Declarative extension {owner} has no implemented command handler");
            }
            return Ok(());
        }
        let packages: Vec<_> = plan
            .ordered
            .iter()
            .map(|item| item.package.clone())
            .collect();
        let targets = vec![owner.to_owned()];
        if let Some(host) = &mut self.extension_host {
            let additions = packages
                .iter()
                .filter(|package| !host.packages.iter().any(|old| old.id == package.id))
                .cloned()
                .collect();
            let request = ActivationRequest {
                additions,
                targets,
                owner: command.then(|| owner.to_owned()),
            };
            host.activate(&request, &self.documents, self.active, &self.settings)?;
            self.extension_packages = host.packages.clone();
        } else {
            self.start_extension_packages_lazy(packages, Some(targets))?;
            self.activation.starting = true;
        }
        self.activation.waiting = if command {
            self.activation
                .pending
                .front()
                .map(|command| (owner.to_owned(), command.token))
        } else {
            None
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata(id: &str) -> Metadata {
        Metadata {
            package: crate::extensions::Package {
                id: id.into(),
                version: "1".into(),
                path: "/unused/snapshot".into(),
                sha256: None,
            },
            dependencies: BTreeSet::new(),
            events: BTreeSet::new(),
            unsupported_events: BTreeSet::new(),
            commands: BTreeMap::new(),
            keybindings: Value::Null,
            code: true,
            blocked: None,
        }
    }
    fn app(root: &Path) -> App {
        let mut app = App::new(root.into(), Profile::Linux);
        app.execute("workbench.action.files.newUntitledFile", Value::Null);
        app.doc_mut().insert("unsaved 猫", false);
        app.activation.configured = true;
        app.activation.loaded = true;
        app
    }
    #[test]
    fn stopped_plan_cannot_start_a_host_or_consume_a_later_native_document() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let identity = app.doc().id;
        let (sender, receiver) = sync_channel(1);
        app.activation.job = Some(Job {
            command: None,
            generation: 0,
            control: 0,
            extension_epoch: Some(app.extension_epoch),
            receiver,
        });
        sender
            .send(Ok(Output::Planned(
                lifecycle::Plan {
                    ordered: vec![metadata("test.a")],
                },
                "test.a".into(),
            )))
            .unwrap();
        app.stop_extension_host();
        app.poll_extension_activation();
        assert!(app.activation.job.is_none());
        assert!(app.extension_host.is_none());
        assert!(app.extension_packages.is_empty());
        assert!(app.activation.paused);
        assert_eq!(app.doc().id, identity);
        assert_eq!(app.doc().text.to_string(), "unsaved 猫");
    }
    #[test]
    fn older_grant_completion_publishes_consent_without_overriding_a_later_stop() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let mut global = Preferences::default();
        global.set("test.a", true).unwrap();
        let snapshot = Snapshot {
            global,
            workspace: Preferences::default(),
            catalog: BTreeMap::from([("test.a".into(), metadata("test.a"))]),
            matched: BTreeSet::from(["test.a".into()]),
            notices: Vec::new(),
        };
        let (sender, receiver) = sync_channel(1);
        app.activation.job = Some(Job {
            command: None,
            generation: 0,
            control: 0,
            extension_epoch: None,
            receiver,
        });
        sender
            .send(Ok(Output::Loaded(
                Box::new(snapshot),
                Some(("test.a".into(), true)),
            )))
            .unwrap();
        app.stop_extension_host();
        app.poll_extension_activation();
        assert!(app.extension_enabled("test.a"));
        assert!(app.activation.paused);
        assert!(app.extension_host.is_none());
        assert!(app.activation.job.is_none());
        assert_eq!(app.doc().text.to_string(), "unsaved 猫");
    }
    #[test]
    fn commandless_plan_does_not_consume_or_run_a_queued_command_from_another_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let mut a = metadata("test.a");
        a.code = false;
        let b = metadata("test.b");
        app.activation.catalog = Arc::new(BTreeMap::from([("test.b".into(), b)]));
        app.activation.global.set("test.b", true).unwrap();
        app.activation
            .owners
            .insert("b.run".into(), "test.b".into());
        let context = CommandContext::capture(&app).unwrap();
        app.activation.pending.push_back(Command {
            token: 1,
            context,
            id: "b.run".into(),
            args: Some(json!([1, 2])),
        });
        let (sender, receiver) = sync_channel(1);
        app.activation.job = Some(Job {
            command: None,
            generation: 0,
            control: 0,
            extension_epoch: Some(app.extension_epoch),
            receiver,
        });
        sender
            .send(Ok(Output::Planned(
                lifecycle::Plan { ordered: vec![a] },
                "test.a".into(),
            )))
            .unwrap();
        app.poll_extension_activation();
        assert_eq!(app.activation.pending.len(), 1);
        assert_eq!(app.activation.pending.front().unwrap().id, "b.run");
        assert_eq!(
            app.activation.pending.front().unwrap().args,
            Some(json!([1, 2]))
        );
        assert!(app.activation.waiting.is_none());
        assert!(app.extension_host.is_none());
    }
    #[test]
    fn queued_commands_and_argument_bytes_remain_bounded() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        app.activation
            .owners
            .insert("a.run".into(), "test.a".into());
        for _ in 0..8 {
            assert!(app.queue_extension_command("a.run", &Some(json!(["data"]))));
        }
        assert!(app.queue_extension_command("a.run", &None));
        assert_eq!(app.activation.pending.len(), 8);
        app.activation.pending.clear();
        assert!(app.queue_extension_command("a.run", &Some(json!("x".repeat(65 * 1024)))));
        assert!(app.activation.pending.is_empty());
        for _ in 0..8 {
            app.queue_extension_command("a.run", &Some(json!("x".repeat(63 * 1024))));
        }
        assert_eq!(app.activation.pending.len(), 4);
    }
    #[test]
    fn deferred_command_context_cancels_on_selection_revision_and_focus_away_then_back() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        app.activation
            .owners
            .insert("a.run".into(), "test.a".into());
        assert!(app.queue_extension_command("a.run", &None));
        app.execute("cursorLeft", Value::Null);
        assert!(app.activation.pending.is_empty());
        assert!(app.queue_extension_command("a.run", &None));
        app.doc_mut().insert("native", false);
        app.invalidate_pending_extension_commands();
        assert!(app.activation.pending.is_empty());
        assert!(app.queue_extension_command("a.run", &None));
        app.focus = Focus::Explorer;
        app.invalidate_pending_extension_commands();
        app.focus = Focus::Editor;
        app.invalidate_pending_extension_commands();
        assert!(app.activation.pending.is_empty());
        let selection = app.doc().selections()[0].clone();
        app.doc_mut().secondary = vec![selection; 4096];
        assert!(app.queue_extension_command("a.run", &None));
        assert!(app.activation.pending.is_empty());
        assert!(app.message.contains("at most 4096 selections"));
    }
    #[test]
    fn a_completed_old_plan_cannot_consume_a_new_ticket_with_the_same_command_id() {
        let root = tempfile::tempdir().unwrap();
        let mut app = app(root.path());
        let mut a = metadata("test.a");
        a.code = false;
        app.activation.catalog = Arc::new(BTreeMap::from([("test.a".into(), a.clone())]));
        app.activation.global.set("test.a", true).unwrap();
        app.activation
            .owners
            .insert("a.run".into(), "test.a".into());
        app.queue_extension_command("a.run", &Some(json!(["old"])));
        let old = app.activation.pending.front().unwrap().token;
        let (sender, receiver) = sync_channel(1);
        app.activation.job = Some(Job {
            command: Some(old),
            generation: 0,
            control: 0,
            extension_epoch: Some(app.extension_epoch),
            receiver,
        });
        app.execute("cursorLeft", Value::Null);
        app.queue_extension_command("a.run", &Some(json!(["new"])));
        let new = app.activation.pending.front().unwrap().token;
        assert_ne!(old, new);
        sender
            .send(Ok(Output::Planned(
                lifecycle::Plan { ordered: vec![a] },
                "test.a".into(),
            )))
            .unwrap();
        app.poll_extension_activation();
        assert_eq!(app.activation.pending.front().unwrap().token, new);
        assert_eq!(
            app.activation.pending.front().unwrap().args,
            Some(json!(["new"]))
        );
        assert!(app.activation.waiting.is_none());
        assert!(app.extension_host.is_none());
    }
}
