//! Optional isolated CommonJS extension host. Rust owns document transactions.
mod decorations;
mod diagnostics;
pub(crate) mod editor_inventory;
pub(crate) mod editor_state;
pub(crate) mod editor_tabs;
mod owner_lifetimes;
mod prompts;
pub(crate) mod providers;
pub(crate) mod services;
mod surfaces;
pub(crate) mod text_content;
pub(crate) mod untitled;
use crate::{document::Document, lsp, settings::Settings, transport::Process};
use anyhow::{Context, Result, bail};
pub use prompts::{MAX_PROMPT_TEXT, NativePrompt, PromptType};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
pub use surfaces::{
    OutputChannel, StatusItem, SurfaceDeclarations, SurfaceKey, SurfaceReveal, SurfaceState,
    TreeNode, TreeView,
};

const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PACKAGES: usize = 8;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub version: String,
    pub path: PathBuf,
    pub sha256: Option<String>,
}
impl Package {
    pub fn read(path: &Path) -> Result<Self> {
        let path = std::fs::canonicalize(path).context("Cannot open extension directory")?;
        let (_, manifest) = read_manifest(&path)?;
        let publisher = manifest["publisher"]
            .as_str()
            .context("Extension manifest has no publisher")?;
        let name = manifest["name"]
            .as_str()
            .context("Extension manifest has no name")?;
        let id = format!("{publisher}.{name}").to_ascii_lowercase();
        if id.split('.').count() != 2
            || id.split('.').any(|part| {
                part.is_empty()
                    || part.len() > 100
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        {
            bail!("Extension ID must be publisher.name");
        }
        let version = manifest["version"]
            .as_str()
            .context("Extension manifest has no version")?
            .to_owned();
        if version.is_empty() || version.len() > 100 {
            bail!("Invalid extension version");
        }
        Ok(Self {
            id,
            version,
            path,
            sha256: None,
        })
    }
    pub(crate) fn manifest(&self) -> Result<Value> {
        let (_, manifest) = read_manifest(&self.path)?;
        let id = format!(
            "{}.{}",
            manifest["publisher"].as_str().unwrap_or_default(),
            manifest["name"].as_str().unwrap_or_default()
        )
        .to_ascii_lowercase();
        if id != self.id || manifest["version"].as_str() != Some(self.version.as_str()) {
            bail!("Selected extension snapshot identity changed");
        }
        Ok(manifest)
    }
    pub fn installed(item: &crate::extension_store::Installed) -> Self {
        Self {
            id: item.id.clone(),
            version: item.version.clone(),
            path: item.path.clone(),
            sha256: Some(item.sha256.clone()),
        }
    }
}
fn read_manifest(path: &Path) -> Result<(usize, Value)> {
    let manifest = path.join("package.json");
    if !std::fs::metadata(&manifest)?.is_file() {
        bail!("Extension manifest must be a regular file");
    }
    let mut bytes = Vec::new();
    std::fs::File::open(manifest)?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        bail!("Extension manifest exceeds 1 MiB");
    }
    Ok((bytes.len(), serde_json::from_slice(&bytes)?))
}
fn manifest_nodes(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(manifest_nodes).sum::<usize>(),
        Value::Object(values) => 1 + values.values().map(manifest_nodes).sum::<usize>(),
        _ => 1,
    }
}
fn validate_packages(packages: &[Package]) -> Result<Vec<Package>> {
    if packages.is_empty() || packages.len() > MAX_PACKAGES {
        bail!("Select between one and eight code extensions");
    }
    let mut selected = packages.to_vec();
    selected.sort_by(|a, b| a.id.cmp(&b.id));
    let mut ids = HashSet::new();
    let (mut bytes, mut nodes, mut bindings, mut commands) = (0, 0, 0, 0);
    for package in &selected {
        if !ids.insert(package.id.clone()) {
            bail!("Duplicate extension: {}", package.id);
        }
        let (size, manifest) = read_manifest(&package.path)?;
        bytes += size;
        nodes += manifest_nodes(&manifest);
        if bytes > 8 * 1024 * 1024 || nodes > 50_000 {
            bail!("Extension session manifest budget exceeded (8 MiB / 50,000 nodes)");
        }
        let id = format!(
            "{}.{}",
            manifest["publisher"].as_str().unwrap_or(""),
            manifest["name"].as_str().unwrap_or("")
        )
        .to_ascii_lowercase();
        if id != package.id || manifest["version"].as_str() != Some(package.version.as_str()) {
            bail!("Extension identity changed: {}", package.id);
        }
        let count = |value: &Value| match value {
            Value::Null => 0,
            Value::Array(v) => v.len(),
            _ => 1,
        };
        bindings += count(&manifest["contributes"]["keybindings"]);
        commands += count(&manifest["contributes"]["commands"]);
        if bindings > 1024 || commands > 1024 {
            bail!("Extension session contribution limit exceeded");
        }
    }
    Ok(selected)
}
#[derive(Deserialize)]
struct Registration {
    id: String,
    owner: String,
}

#[derive(Clone)]
struct Mirror {
    language_epoch: u64,
    editor_view: crate::document::EditorViewProof,
    revision: u64,
    text_epoch: u64,
    resource_epoch: u64,
    save_generation: u64,
    uri: String,
    eol: String,
    version: u64,
}
struct Pending {
    owner: Option<String>,
    method: String,
    started: Instant,
}
#[derive(Deserialize)]
struct NativeTextEdit {
    #[serde(flatten)]
    edit: lsp::TextEdit,
    #[serde(default, rename = "forceMoveMarkers")]
    force_move_markers: bool,
}
#[derive(Deserialize)]
struct Edit {
    #[serde(default)]
    editor: Option<String>,
    session: u64,
    owner: String,
    document: u64,
    version: u64,
    edits: Vec<NativeTextEdit>,
}
pub struct Prepared {
    storage_root: Option<PathBuf>,
    editor_inventory: Option<editor_inventory::Inventory>,
    editor_tabs: Option<editor_tabs::Inventory>,
    mirror: MirrorState,
    state: Value,
    configuration: Arc<Vec<serde_json::Map<String, Value>>>,
    activation: Option<Vec<String>>,
}
impl Prepared {
    pub(crate) fn with_editor_tabs(mut self, inventory: editor_tabs::Inventory) -> Result<Self> {
        let wire = inventory.wire()?;
        self.state["tabs"] = wire.clone();
        self.mirror.tabs_stamp = wire;
        self.mirror.tabs = Some(inventory.clone());
        self.editor_tabs = Some(inventory);
        Ok(self)
    }
    /// Select lazy roots; omitted roots preserve explicit eager startup.
    pub fn with_activation(mut self, ids: Vec<String>) -> Result<Self> {
        if ids.len() > MAX_PACKAGES {
            bail!("Extension activation target limit exceeded");
        }
        for id in &ids {
            crate::extension_activation::validate_id(id)?;
        }
        self.activation = Some(ids);
        Ok(self)
    }
}
pub struct ActivationRequest {
    /// Worker-validated registered languages admitted with this selected cohort.
    pub registered_languages: HashSet<String>,
    /// Prepared in the activation worker, never from the native input path.
    pub surface_declarations: surfaces::SurfaceDeclarations,
    pub additions: Vec<Package>,
    pub targets: Vec<String>,
    pub owner: Option<String>,
}
pub struct Client {
    editor_inventory: Option<editor_inventory::Inventory>,
    editor_tabs: Option<editor_tabs::Inventory>,
    decorations: decorations::Registry,
    diagnostics: diagnostics::State,
    providers: providers::State,
    services: std::collections::VecDeque<services::NativeService>,
    workspace_edit_context: Option<crate::app::workspace_transactions::CaptureContext>,
    file_search_context: Option<crate::workspace_file_search::Context>,
    owner_lifetimes: owner_lifetimes::OwnerLifetimes,
    text_content_registry: text_content::Registry,
    untitled_context: Option<untitled::CreationContext>,
    next_untitled_name: u64,
    editor_state_context: Option<editor_state::Context>,
    selected_languages: HashSet<String>,
    state_store: Option<crate::extension_state::Store>,
    // Drop the process before deleting its embedded runtime files on Windows.
    process: Process,
    _runtime: tempfile::TempDir,
    mirror: MirrorState,
    mirror_suspended: bool,
    configuration: Arc<Vec<serde_json::Map<String, Value>>>,
    pending: HashMap<u64, Pending>,
    prompts: std::collections::VecDeque<NativePrompt>,
    pub surfaces: SurfaceState,
    next_id: u64,
    heartbeat: Instant,
    pub keybindings: Option<Value>,
    pub ready: bool,
    pub commands: Vec<(String, String)>,
    pub identity: String,
    pub session: u64,
    pub packages: Vec<Package>,
    pub binding_sets: Option<Vec<(String, Value)>>,
    pub activation_states: std::collections::BTreeMap<String, String>,
    command_owners: HashMap<String, String>,
    titles: HashMap<(String, String), String>,
}
impl Client {
    pub(crate) fn set_editor_tabs(&mut self, inventory: editor_tabs::Inventory) {
        if self.editor_tabs.is_some() {
            self.editor_tabs = Some(inventory);
        }
    }
    /// Admission uses the last accepted mirror, never a newer unpublished cohort.
    pub(crate) fn editor_tabs(&self) -> Option<&editor_tabs::Inventory> {
        self.mirror.tabs.as_ref()
    }
    pub fn start(
        node: &str,
        extension: &Path,
        root: &Path,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Self> {
        Self::start_many(
            node,
            &[Package::read(extension)?],
            root,
            documents,
            active,
            settings,
        )
    }
    pub fn start_many(
        node: &str,
        packages: &[Package],
        root: &Path,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Self> {
        Self::start_many_prepared(
            node,
            packages,
            root,
            Self::prepare(documents, active, settings)?,
        )
    }
    pub fn prepare(documents: &[Document], active: usize, settings: &Settings) -> Result<Prepared> {
        Self::prepare_with_hidden(documents, &[], active, settings)
    }
    pub fn prepare_with_hidden(
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Prepared> {
        let refs: Vec<_> = documents.iter().chain(hidden).collect();
        let (mirror, state) = MirrorState::default().next_refs(&refs, documents.get(active))?;
        Ok(Prepared {
            storage_root: None,
            editor_inventory: None,
            editor_tabs: None,
            mirror,
            state,
            configuration: settings.extension_layers().clone(),
            activation: None,
        })
    }
    pub fn start_prepared(
        node: &str,
        extension: &Path,
        root: &Path,
        prepared: Prepared,
    ) -> Result<Self> {
        Self::start_many_prepared(node, &[Package::read(extension)?], root, prepared)
    }
    pub fn start_many_prepared(
        node: &str,
        packages: &[Package],
        root: &Path,
        prepared: Prepared,
    ) -> Result<Self> {
        let packages = validate_packages(packages)?;
        let state_store = prepared
            .storage_root
            .as_ref()
            .map(|root_path| crate::extension_state::Store::new(root_path, root))
            .transpose()?;
        let extension_state = services::initial_state(state_store.as_ref(), &packages)?;
        let surfaces = SurfaceState::for_packages(&packages)?;
        let selected_languages = selected_languages(&packages)?;
        static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let session = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let runtime = tempfile::tempdir()?;
        for (name, source) in [
            (
                "editor-presentation.cjs",
                include_str!("../extension-host/editor-presentation.cjs"),
            ),
            (
                "api-types.cjs",
                include_str!("../extension-host/api-types.cjs"),
            ),
            ("api.cjs", include_str!("../extension-host/api.cjs")),
            (
                "caller-context.cjs",
                include_str!("../extension-host/caller-context.cjs"),
            ),
            (
                "es5-class-compat.cjs",
                include_str!("../extension-host/es5-class-compat.cjs"),
            ),
            (
                "edit-types.cjs",
                include_str!("../extension-host/edit-types.cjs"),
            ),
            (
                "edit-values.cjs",
                include_str!("../extension-host/edit-values.cjs"),
            ),
            ("uri.cjs", include_str!("../extension-host/uri.cjs")),
            (
                "workspace-scope.cjs",
                include_str!("../extension-host/workspace-scope.cjs"),
            ),
            (
                "document-types.cjs",
                include_str!("../extension-host/document-types.cjs"),
            ),
            (
                "document-types.LICENSE.txt",
                include_str!("../extension-host/document-types.LICENSE.txt"),
            ),
            (
                "filesystem.cjs",
                include_str!("../extension-host/filesystem.cjs"),
            ),
            (
                "filesystem.LICENSE.txt",
                include_str!("../extension-host/filesystem.LICENSE.txt"),
            ),
            (
                "coordinates.cjs",
                include_str!("../extension-host/coordinates.cjs"),
            ),
            (
                "code-action-types.cjs",
                include_str!("../extension-host/code-action-types.cjs"),
            ),
            (
                "code-actions.cjs",
                include_str!("../extension-host/code-actions.cjs"),
            ),
            (
                "diagnostic-types.cjs",
                include_str!("../extension-host/diagnostic-types.cjs"),
            ),
            (
                "diagnostics.cjs",
                include_str!("../extension-host/diagnostics.cjs"),
            ),
            (
                "cancellation.cjs",
                include_str!("../extension-host/cancellation.cjs"),
            ),
            (
                "provider-types.cjs",
                include_str!("../extension-host/provider-types.cjs"),
            ),
            (
                "markdown-string.cjs",
                include_str!("../extension-host/markdown-string.cjs"),
            ),
            (
                "snippet-string.cjs",
                include_str!("../extension-host/snippet-string.cjs"),
            ),
            (
                "providers.cjs",
                include_str!("../extension-host/providers.cjs"),
            ),
            (
                "navigation-providers.cjs",
                include_str!("../extension-host/navigation-providers.cjs"),
            ),
            (
                "language-selector.cjs",
                include_str!("../extension-host/language-selector.cjs"),
            ),
            (
                "provider-order.cjs",
                include_str!("../extension-host/provider-order.cjs"),
            ),
            (
                "workspace-symbols.cjs",
                include_str!("../extension-host/workspace-symbols.cjs"),
            ),
            (
                "rename-providers.cjs",
                include_str!("../extension-host/rename-providers.cjs"),
            ),
            (
                "workspace-edit-values.cjs",
                include_str!("../extension-host/workspace-edit-values.cjs"),
            ),
            (
                "signatures.cjs",
                include_str!("../extension-host/signatures.cjs"),
            ),
            ("symbols.cjs", include_str!("../extension-host/symbols.cjs")),
            ("prompts.cjs", include_str!("../extension-host/prompts.cjs")),
            (
                "document-services.cjs",
                include_str!("../extension-host/document-services.cjs"),
            ),
            (
                "editor-decorations.cjs",
                include_str!("../extension-host/editor-decorations.cjs"),
            ),
            (
                "editor-snippets.cjs",
                include_str!("../extension-host/editor-snippets.cjs"),
            ),
            (
                "editor-tabs.cjs",
                include_str!("../extension-host/editor-tabs.cjs"),
            ),
            (
                "tab-inputs.cjs",
                include_str!("../extension-host/tab-inputs.cjs"),
            ),
            ("memento.cjs", include_str!("../extension-host/memento.cjs")),
            (
                "surfaces.cjs",
                include_str!("../extension-host/surfaces.cjs"),
            ),
            (
                "configuration.cjs",
                include_str!("../extension-host/configuration.cjs"),
            ),
            (
                "activation.cjs",
                include_str!("../extension-host/activation.cjs"),
            ),
            (
                "native-requests.cjs",
                include_str!("../extension-host/native-requests.cjs"),
            ),
            (
                "workspace-edits.cjs",
                include_str!("../extension-host/workspace-edits.cjs"),
            ),
            (
                "workspace-files.cjs",
                include_str!("../extension-host/workspace-files.cjs"),
            ),
            (
                "file-watchers.cjs",
                include_str!("../extension-host/file-watchers.cjs"),
            ),
            (
                "watcher-glob.cjs",
                include_str!("../extension-host/watcher-glob.cjs"),
            ),
            (
                "local-file-watcher.cjs",
                include_str!("../extension-host/local-file-watcher.cjs"),
            ),
            (
                "text-content-providers.cjs",
                include_str!("../extension-host/text-content-providers.cjs"),
            ),
            ("host.cjs", include_str!("../extension-host/host.cjs")),
        ] {
            std::fs::write(runtime.path().join(name), source)?;
        }
        let process = Process::start_isolated(
            node,
            &[
                "--max-old-space-size=256".into(),
                runtime
                    .path()
                    .join("host.cjs")
                    .to_string_lossy()
                    .into_owned(),
            ],
            root,
        )?;
        let mut client = Self {
            editor_inventory: prepared.editor_inventory,
            editor_tabs: prepared.editor_tabs,
            diagnostics: diagnostics::State::default(),
            decorations: decorations::Registry::default(),
            providers: providers::State::default(),
            services: services::empty_queue(),
            workspace_edit_context: None,
            file_search_context: None,
            text_content_registry: text_content::Registry::default(),
            untitled_context: None,
            next_untitled_name: 0,
            editor_state_context: None,
            selected_languages,
            owner_lifetimes: owner_lifetimes::OwnerLifetimes::new(&packages),
            state_store,
            process,
            _runtime: runtime,
            mirror: prepared.mirror,
            mirror_suspended: false,
            configuration: prepared.configuration,
            pending: HashMap::new(),
            prompts: std::collections::VecDeque::new(),
            surfaces,
            next_id: 0,
            heartbeat: Instant::now(),
            keybindings: None,
            ready: false,
            commands: Vec::new(),
            identity: packages
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            session,
            packages,
            binding_sets: None,
            activation_states: std::collections::BTreeMap::new(),
            command_owners: HashMap::new(),
            titles: HashMap::new(),
        };
        client.request(
            "initialize",
            json!({"protocol":4, "session": session, "extensions": client.packages,
                "languageProviders": true, "workspaceEdits": true, "workspaceUnopenedEdits": true, "workspaceRefactoringSaves": true, "findFiles": true, "virtualDocuments": true, "untitledDocuments": true, "documentLanguage": true, "editorSelections": true, "editorOptions":client.editor_inventory_explicit(), "editorReveal":client.editor_inventory_explicit(), "editorDecorations":client.editor_inventory_explicit(), "editorSnippets":client.editor_inventory_explicit(), "editorTabs":client.editor_tabs.is_some(), "reservedCommands": crate::app::native_command_ids(), "root": root, "state": prepared.state,
                "configuration": client.configuration.as_ref(), "activate": prepared.activation, "extensionState": extension_state}),
        )?;
        Ok(client)
    }
    #[cfg(test)]
    pub(crate) fn fixture_gate_native_writer(
        &self,
    ) -> Result<(
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::SyncSender<()>,
    )> {
        self.process.fixture_gate_writer()
    }
    #[cfg(test)]
    pub(crate) fn fixture_native_output_barrier(&mut self) -> Result<()> {
        self.request("ping", json!({"session":self.session}))
    }
    #[cfg(test)]
    pub(crate) fn fixture_native_output_probe(&self) -> Result<()> {
        // Real harmless notifications fill the existing transport queue. They
        // carry no request id, register no callback, and cannot mutate models.
        self.process
            .send(json!({"method":"cancelLanguageProvider", "params":{
                "session":self.session, "owner":self.packages[0].id, "request":0,
            }}))
    }
    fn request(&mut self, method: &str, params: Value) -> Result<()> {
        if self.pending.len() >= 64 {
            bail!("Extension request limit reached");
        }
        self.next_id += 1;
        self.process
            .send(json!({"id":self.next_id, "method":method, "params":params}))?;
        self.pending.insert(
            self.next_id,
            Pending {
                owner: params["owner"].as_str().map(str::to_owned),
                method: method.into(),
                started: Instant::now(),
            },
        );
        Ok(())
    }
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(crate) fn validate_selected_owner(&self, session: u64, owner: &str) -> Result<()> {
        if session != self.session || !self.packages.iter().any(|p| p.id == owner) {
            bail!("Outdated extension native request owner/session");
        }
        Ok(())
    }
    fn owner_retired(&self, owner: &str) -> bool {
        self.activation_states
            .get(owner)
            .is_some_and(|state| matches!(state.as_str(), "failed" | "disposed"))
    }
    pub(crate) fn validate_native_origin(
        &self,
        session: u64,
        owner: &str,
        command: Option<u64>,
        command_owner: Option<&str>,
    ) -> Result<()> {
        self.validate_selected_owner(session, owner)?;
        if self.owner_retired(owner) {
            bail!("Extension native request owner has failed or stopped");
        }
        let Some(command) = command else {
            if command_owner.is_some() {
                bail!("Extension native request has an owner without a command origin");
            }
            return Ok(());
        };
        let pending = self
            .pending
            .get(&command)
            .context("Outdated extension native request command")?;
        if !self
            .packages
            .iter()
            .any(|p| Some(p.id.as_str()) == command_owner)
        {
            bail!("Invalid extension native request command owner");
        }
        let owned = pending.owner.as_deref() == command_owner;
        if !(pending.method == "initialize"
            || matches!(
                pending.method.as_str(),
                "execute"
                    | "provideLanguage"
                    | "provideWorkspaceSymbols"
                    | "resolveLanguageWorkspaceSymbol"
                    | "prepareLanguageRename"
                    | "provideLanguageRename"
                    | "resolveLanguageCompletion"
                    | "resolveLanguageCodeAction"
                    | "treeChildren"
                    | "surfaceAction"
            ) && owned
            || pending.method == "activate" && (pending.owner.is_none() || owned))
        {
            bail!("Invalid extension native request command origin");
        }
        Ok(())
    }
    pub(crate) fn sync_configuration(&mut self, settings: &Settings) -> Result<()> {
        if !Arc::ptr_eq(&self.configuration, settings.extension_layers()) {
            self.process.send(
                json!({"method":"configuration", "params":settings.extension_layers().as_ref()}),
            )?;
            self.configuration = settings.extension_layers().clone();
        }
        Ok(())
    }
    /// Admit worker-validated immutable descriptors and activate selected roots.
    /// Disk validation belongs to the planning worker and the optional host.
    pub fn activate(
        &mut self,
        request: &ActivationRequest,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        self.activate_prepared_with_hidden(request, json!({}), documents, &[], active, settings)
    }
    /// State is staged by the activation worker before admitting new packages.
    pub(crate) fn activate_prepared_with_hidden(
        &mut self,
        request: &ActivationRequest,
        extension_state: Value,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        let ActivationRequest {
            registered_languages,
            surface_declarations,
            additions,
            targets,
            owner,
        } = request;
        let owner = owner.as_deref();
        if !self.ready || self.activating() {
            bail!("Extension host is not ready for another activation batch");
        }
        let mut packages = self.packages.clone();
        packages.extend_from_slice(additions);
        packages.sort_by(|a, b| a.id.cmp(&b.id));
        if packages.len() > MAX_PACKAGES
            || targets.len() > MAX_PACKAGES
            || packages.windows(2).any(|pair| pair[0].id == pair[1].id)
        {
            bail!("Invalid extension cohort or activation target count");
        }
        for package in &packages {
            crate::extension_activation::validate_id(&package.id)?;
        }
        if targets
            .iter()
            .any(|id| !packages.iter().any(|package| &package.id == id))
            || owner.is_some_and(|id| !packages.iter().any(|package| package.id == id))
        {
            bail!("Activation targets and owner must be selected");
        }
        if registered_languages.len() > MAX_PACKAGES * 128
            || registered_languages
                .iter()
                .any(|id| id.is_empty() || id.len() > 128 || id.contains('\0'))
        {
            bail!("Registered language contribution bounds exceeded");
        }
        let declarations = self.surfaces.stage_declarations(surface_declarations)?;
        self.sync_with_hidden(documents, hidden, active)?;
        self.sync_configuration(settings)?;
        self.request(
            "activate",
            json!({"session":self.session, "owner":owner,
            "extensions":additions, "activate":targets, "extensionState":extension_state}),
        )?;
        self.surfaces.commit_declarations(declarations);
        self.selected_languages
            .extend(registered_languages.iter().cloned());
        self.owner_lifetimes.admit_selected(&packages);
        self.packages = packages;
        self.identity = self
            .packages
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(())
    }
    pub fn owner_active(&self, owner: &str) -> bool {
        self.ready
            && self
                .activation_states
                .get(owner)
                .is_some_and(|state| state == "active")
    }
    pub(crate) fn command_owner(&self, command: &str) -> Option<&str> {
        self.command_owners.get(command).map(String::as_str)
    }
    pub fn activating(&self) -> bool {
        self.pending
            .values()
            .any(|pending| matches!(pending.method.as_str(), "initialize" | "activate"))
    }
    fn register_activation(&mut self, value: &Value) -> Result<()> {
        let items = value
            .as_array()
            .context("Extension activation status missing")?;
        if items.len() != self.packages.len() {
            bail!("Extension activation status count changed");
        }
        let mut states = std::collections::BTreeMap::new();
        for item in items {
            let id = item["id"]
                .as_str()
                .context("Extension activation owner missing")?;
            let state = item["state"]
                .as_str()
                .context("Extension activation state missing")?;
            if !self.packages.iter().any(|p| p.id == id)
                || !matches!(
                    state,
                    "dormant" | "activating" | "active" | "failed" | "disposed"
                )
                || states.insert(id.to_owned(), state.to_owned()).is_some()
            {
                bail!("Invalid extension activation registry");
            }
        }
        self.owner_lifetimes.publish(&states);
        for (owner, state) in &states {
            if matches!(state.as_str(), "failed" | "disposed") {
                self.surfaces.remove_owner(owner);
                self.decorations.remove_owner(owner);
                self.diagnostics.remove_owner(owner);
            }
        }
        self.activation_states = states;
        Ok(())
    }
    pub fn execute(
        &mut self,
        command: &str,
        args: Option<Value>,
        documents: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        self.execute_with_hidden(command, args, documents, &[], active, settings)
    }
    pub fn execute_with_hidden(
        &mut self,
        command: &str,
        args: Option<Value>,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
        settings: &Settings,
    ) -> Result<()> {
        if !self.ready {
            bail!("Extension host is not ready");
        }
        if !self.commands.iter().any(|(_, id)| id == command) {
            bail!("Extension command is not registered: {command}");
        }
        self.sync_with_hidden(documents, hidden, active)?;
        self.sync_configuration(settings)?;
        let args: Vec<_> = args.into_iter().collect();
        self.request("execute", json!({"session":self.session, "owner":self.command_owners[command], "command":command, "args":args}))
    }
    pub(crate) fn selected_language_known(&self, language: &str) -> bool {
        self.selected_languages.contains(language)
    }
    pub(crate) fn mark_api_selections(&mut self, document: u64) {
        self.mirror.api_selections.insert(document);
    }
    pub(crate) fn sync_with_hidden(
        &mut self,
        documents: &[Document],
        hidden: &[Document],
        active: usize,
    ) -> Result<()> {
        let refs: Vec<_> = documents.iter().chain(hidden).collect();
        let active = documents.get(active);
        check_mirror_budget(&refs)?;
        let editor_stamp = self
            .editor_inventory
            .as_ref()
            .map(|inventory| inventory.stamp(&refs))
            .transpose()?;
        let tabs_stamp = self
            .editor_tabs
            .as_ref()
            .map(editor_tabs::Inventory::wire)
            .transpose()?;
        if MirrorState::stamp_refs(&refs, active) != self.mirror.last_stamp
            || !self.mirror.api_selections.is_empty()
            || !self.mirror.api_editor_selections.is_empty()
            || editor_stamp
                .as_ref()
                .is_some_and(|stamp| *stamp != self.mirror.editor_stamp)
            || tabs_stamp
                .as_ref()
                .is_some_and(|stamp| *stamp != self.mirror.tabs_stamp)
        {
            let (mut next, mut state) = self.mirror.next_refs_with_inventory_stamps(
                &refs,
                active,
                editor_stamp.as_ref(),
                tabs_stamp.as_ref(),
            )?;
            if let Some(inventory) = &self.editor_inventory {
                state["editors"] = inventory.wire(&refs, &next.api_editor_selections)?;
                state["activeEditor"] = json!(inventory.active);
                next.editors = inventory.frames.clone();
                next.api_editor_selections.clear();
            }
            if let Some(stamp) = tabs_stamp {
                state["tabs"] = stamp;
                next.tabs = self.editor_tabs.clone();
            }
            self.process
                .send(json!({"method":"state", "params":state}))?;
            self.mirror = next;
        }
        Ok(())
    }
    #[cfg(test)]
    fn apply_edit(&self, edit: Edit, documents: &mut [Document]) -> Result<bool> {
        self.apply_edit_with_hidden(edit, documents, &mut [])
    }
    fn apply_edit_with_hidden(
        &self,
        mut edit: Edit,
        documents: &mut [Document],
        hidden: &mut [Document],
    ) -> Result<bool> {
        if self
            .validate_selected_owner(edit.session, &edit.owner)
            .is_err()
            || self.owner_retired(&edit.owner)
        {
            return Ok(false);
        }
        let Some(mirror) = self.mirror.mirrors.get(&edit.document) else {
            return Ok(false);
        };
        let total: usize = documents
            .iter()
            .chain(hidden.iter())
            .map(|d| d.text.len_bytes())
            .sum();
        let Some(doc) = documents
            .iter_mut()
            .chain(hidden.iter_mut())
            .find(|d| d.id == edit.document)
        else {
            return Ok(false);
        };
        if doc.is_read_only()
            || match &edit.editor {
                Some(editor) => editor.len() > 128 || !self.editor_view_live(editor, doc),
                None => self.editor_inventory_explicit(),
            }
            || edit.version != mirror.version
            || doc.revision != mirror.revision
            || doc.text_epoch() != mirror.text_epoch
            || doc.resource_epoch() != mirror.resource_epoch
            || doc.eol != mirror.eol
            || document_uri(doc)? != mirror.uri
        {
            return Ok(false);
        }
        // Keep per-edit marker semantics paired through native range sorting.
        edit.edits.sort_by_key(|item| {
            (
                item.edit.range.start.line,
                item.edit.range.start.character,
                item.edit.range.end.line,
                item.edit.range.end.character,
            )
        });
        let force = edit
            .edits
            .iter()
            .map(|item| item.force_move_markers)
            .collect::<Vec<_>>();
        let changes = lsp::edits(doc, edit.edits.into_iter().map(|item| item.edit).collect())?;
        let removed: usize = changes
            .iter()
            .map(|(range, _)| doc.text.slice(range.clone()).len_bytes())
            .sum();
        let added: usize = changes.iter().map(|(_, text)| text.len()).sum();
        if total - removed + added > MAX_DOCUMENT_BYTES {
            bail!("Extension edit exceeds the 4 MiB mirror budget");
        }
        doc.apply_changes_with_marker_semantics(
            changes
                .into_iter()
                .zip(force)
                .map(|((range, text), force)| (range, text, force))
                .collect(),
        );
        Ok(true)
    }
    fn register_commands(&mut self, value: Value) -> Result<()> {
        let registrations: Vec<Registration> = serde_json::from_value(value)?;
        if registrations.len() > 1024 {
            bail!("Extension command limit exceeded");
        }
        let mut commands = Vec::new();
        let mut owners = HashMap::new();
        let reserved = crate::app::native_command_ids();
        for registration in registrations {
            if registration.id.is_empty()
                || registration.id.len() > 1024
                || registration.id.starts_with("cursor")
                || reserved.contains(&registration.id)
                || !self.packages.iter().any(|p| p.id == registration.owner)
                || self.owner_retired(&registration.owner)
                || owners
                    .insert(registration.id.clone(), registration.owner.clone())
                    .is_some()
            {
                bail!("Invalid extension command ownership: {}", registration.id);
            }
            let title = self
                .titles
                .get(&(registration.owner.clone(), registration.id.clone()))
                .cloned()
                .unwrap_or_else(|| format!("{} [{}]", registration.id, registration.owner));
            commands.push((title, registration.id));
        }
        commands.sort_by(|a, b| a.1.cmp(&b.1));
        self.commands = commands;
        self.command_owners = owners;
        Ok(())
    }
    pub fn poll(
        &mut self,
        documents: &mut [Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Vec<String>> {
        self.poll_with_hidden(documents, &mut [], active, settings)
    }
    pub fn poll_with_hidden(
        &mut self,
        documents: &mut [Document],
        hidden: &mut [Document],
        active: usize,
        settings: &Settings,
    ) -> Result<Vec<String>> {
        let mut messages = Vec::new();
        let refs = documents.iter().chain(hidden.iter()).collect::<Vec<_>>();
        match check_mirror_budget(&refs).and_then(|()| {
            self.editor_inventory
                .as_ref()
                .map_or(Ok(()), |inventory| inventory.validate(&refs))
        }) {
            Ok(()) => {
                self.sync_with_hidden(documents, hidden, active)?;
                if self.mirror_suspended {
                    messages.push("Extension document mirrors resumed".into());
                }
                self.mirror_suspended = false;
            }
            Err(error) => {
                if !self.mirror_suspended {
                    messages.push(format!("Extension document mirrors paused: {error}"));
                }
                self.mirror_suspended = true;
            }
        }
        self.sync_configuration(settings)?;
        if self.ready
            && self.pending.is_empty()
            && self.heartbeat.elapsed() >= Duration::from_secs(5)
        {
            self.request("ping", json!({"session":self.session}))?;
            self.heartbeat = Instant::now();
        }
        for _ in 0..16 {
            let Some(mut message) = self.process.receive()? else {
                break;
            };
            if let Some(method) = message["method"].as_str() {
                match method {
                    "activation" => {
                        if message["params"]["session"].as_u64() != Some(self.session) {
                            bail!("Outdated activation registry");
                        }
                        self.register_activation(&message["params"]["activation"])?;
                    }
                    "nativeDocumentOpen"
                    | "nativeDocumentShow"
                    | "nativeCommand"
                    | "nativeStateWrite"
                    | "nativeDocumentSave"
                    | "nativeWorkspaceApplyEdit"
                    | "nativeFindFiles"
                    | "nativeVirtualDocumentOpen"
                    | "nativeUntitledDocumentOpen"
                    | "nativeVirtualDocumentChange"
                    | "nativeDocumentLanguage"
                    | "nativeEditorSelections"
                    | "nativeEditorOptions"
                    | "nativeEditorReveal"
                    | "nativeEditorDecorations"
                    | "nativeEditorInsertSnippet"
                    | "nativeTabsClose" => {
                        let method = method.to_owned();
                        let id = message["id"].clone();
                        if let Err(error) = self.queue_service_with_models(
                            &method,
                            id.clone(),
                            message["params"].take(),
                            documents,
                            hidden,
                        ) {
                            self.process
                                .send(json!({"id":id,"error":{"message":services::bounded_error(&error)}}))?;
                        }
                    }
                    "textContentProviders" => {
                        if let Err(error) =
                            self.register_text_content_providers(message["params"].take())
                        {
                            messages.push(format!(
                                "Text content provider registry rejected: {error:#}"
                            ));
                        }
                    }
                    "cancelNativeFindFiles" => {
                        if let Err(error) = self.cancel_native_file_search(message["params"].take())
                        {
                            messages
                                .push(format!("File discovery cancellation rejected: {error:#}"));
                        }
                    }
                    "diagnosticCollections" => {
                        if let Err(error) =
                            self.register_diagnostics(message["params"].take(), documents, hidden)
                        {
                            messages.push(format!("Extension diagnostics rejected: {error:#}"));
                        }
                    }
                    "nativeSurface" => {
                        if let Err(error) = self.surface_update(message["params"].take()) {
                            messages.push(format!("Extension surface rejected: {error:#}"));
                        }
                    }
                    "decorationType" => {
                        if let Err(error) = self.decoration_update(message["params"].take()) {
                            messages.push(format!("Extension decoration type rejected: {error:#}"));
                        }
                    }
                    "prompt" => {
                        let id = message["id"].clone();
                        if let Err(error) = self.queue_prompt(id.clone(), message["params"].take())
                        {
                            self.process
                                .send(json!({"id":id, "error":{"message":error.to_string()}}))?;
                        }
                    }
                    "edit" => {
                        let result = if message["params"]["edits"]
                            .as_array()
                            .is_some_and(|edits| edits.len() > 4096)
                        {
                            Err(anyhow::anyhow!("Extension edit count limit exceeded"))
                        } else {
                            serde_json::from_value::<Edit>(message["params"].take())
                                .map_err(anyhow::Error::from)
                                .and_then(|edit| {
                                    self.apply_edit_with_hidden(edit, documents, hidden)
                                })
                        };
                        match result {
                            Ok(applied) => {
                                // Ordered notifications precede the acknowledgement.
                                self.sync_with_hidden(documents, hidden, active)?;
                                self.process.send(
                                    json!({"id":message["id"], "result":{"applied":applied}}),
                                )?;
                            }
                            Err(error) => self.process.send(
                                json!({"id":message["id"], "error":{"message":error.to_string()}}),
                            )?,
                        }
                    }
                    "signatureReleased" => {
                        self.signature_released(message["params"].take())?;
                    }
                    "symbolsReleased" => {
                        self.symbols_released(message["params"].take())?;
                    }
                    "languageProviderReleased" => {
                        self.language_provider_released(message["params"].take())?;
                    }
                    "languageProviders" => {
                        if message["params"]["session"].as_u64() != Some(self.session) {
                            continue;
                        }
                        if !self.ready {
                            bail!("Language providers arrived before activation completed");
                        }
                        self.register_language_providers(message["params"]["providers"].take())?;
                    }
                    "commands" => {
                        if message["params"]["session"].as_u64() != Some(self.session) {
                            continue;
                        }
                        if !self.ready {
                            bail!("Extension commands arrived before session activation completed");
                        }
                        self.register_commands(message["params"]["commands"].clone())?;
                    }
                    "message" => {
                        let params = &message["params"];
                        if params["session"].as_u64() == Some(self.session)
                            && self
                                .packages
                                .iter()
                                .any(|p| params["owner"].as_str() == Some(&p.id))
                        {
                            messages.push(
                                params["text"]
                                    .as_str()
                                    .unwrap_or("Extension message")
                                    .chars()
                                    .take(2048)
                                    .collect(),
                            );
                        }
                    }
                    _ => bail!("Unknown extension request: {method}"),
                }
            } else if let Some(id) = message["id"].as_u64()
                && let Some(pending) = self.pending.remove(&id)
            {
                if pending.method == "rankLanguageProviders" {
                    self.provider_rank_response(id, message)?;
                    continue;
                }
                if matches!(
                    pending.method.as_str(),
                    "provideWorkspaceSymbols" | "resolveLanguageWorkspaceSymbol"
                ) {
                    self.workspace_symbol_response(id, message)?;
                    continue;
                }
                if matches!(
                    pending.method.as_str(),
                    "provideLanguage"
                        | "prepareLanguageRename"
                        | "provideLanguageRename"
                        | "resolveLanguageCompletion"
                        | "resolveLanguageCodeAction"
                ) {
                    self.provider_response(id, message)?;
                    continue;
                }
                if pending.method == "treeChildren" {
                    if let Err(error) = self.surface_tree_reply(id, &mut message) {
                        messages.push(format!("Extension tree: {error:#}"));
                    }
                    continue;
                }
                if !message["error"].is_null() {
                    let error = message["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown extension error");
                    if pending.method == "initialize" || pending.method == "ping" {
                        bail!(
                            "Extension {} failed: {error}",
                            if pending.method == "initialize" {
                                "activation"
                            } else {
                                "heartbeat"
                            }
                        );
                    }
                    messages.push(format!("Extension command failed: {error}"));
                    continue;
                }
                if pending.method == "initialize" || pending.method == "activate" {
                    let result = &message["result"];
                    if result["protocol"] != 4 || result["session"].as_u64() != Some(self.session) {
                        bail!("Unsupported or outdated extension session protocol");
                    }
                    let extensions = result["extensions"]
                        .as_array()
                        .context("Extension session packages missing")?;
                    if extensions.len() != self.packages.len() {
                        bail!("Extension session package count changed");
                    }
                    let mut bindings = Vec::new();
                    let mut owners = HashSet::new();
                    for item in extensions {
                        let owner = item["id"]
                            .as_str()
                            .context("Extension session identity missing")?;
                        if !owners.insert(owner)
                            || !self.packages.iter().any(|p| {
                                p.id == owner
                                    && Some(p.version.as_str()) == item["version"].as_str()
                            })
                        {
                            bail!("Extension session identity changed");
                        }
                        bindings.push((owner.to_owned(), item["keybindings"].clone()));
                        if let Some(contributions) = item["contributions"].as_array() {
                            if contributions.len() > 1024 {
                                bail!("Extension command contribution limit exceeded");
                            }
                            for contribution in contributions {
                                if let Some(id) = contribution["command"].as_str() {
                                    let title: String = contribution["title"]
                                        .as_str()
                                        .unwrap_or(id)
                                        .chars()
                                        .take(256)
                                        .collect();
                                    self.titles.insert(
                                        (owner.into(), id.into()),
                                        format!("Extension: {title} [{owner}]"),
                                    );
                                }
                            }
                        }
                    }
                    bindings.sort_by(|a, b| a.0.cmp(&b.0));
                    // Keep single-package callers' contribution access intact.
                    self.keybindings = (bindings.len() == 1).then(|| bindings[0].1.clone());
                    self.binding_sets = Some(bindings);
                    self.register_commands(result["commands"].clone())?;
                    self.register_activation(&result["activation"])?;
                    self.register_language_providers(
                        result["languageProviders"]
                            .as_array()
                            .map_or_else(|| json!([]), |v| json!(v)),
                    )?;
                    self.ready = true;
                    messages.push(format!(
                        "Extension ready: {} ({} commands)",
                        self.identity,
                        self.commands.len()
                    ));
                }
            }
        }
        self.expire_prompts()?;
        self.expire_document_save_path_waits();
        self.expire_language_providers()?;
        if self.process.exited() {
            bail!("Extension process exited\n{}", self.process.stderr_tail());
        }
        if self.pending.iter().any(|(id, p)| {
            p.started.elapsed() > Duration::from_secs(30)
                && !self.document_save_command_waiting(*id)
                && !self
                    .prompts
                    .iter()
                    .any(|prompt| prompt.spec.command == Some(*id))
        }) {
            bail!("Extension host timed out; native editing remains available");
        }
        Ok(messages)
    }
}
#[derive(Clone, Default)]
struct MirrorState {
    tabs_stamp: Value,
    tabs: Option<editor_tabs::Inventory>,
    editor_stamp: Value,
    editors: Vec<editor_inventory::Frame>,
    api_editor_selections: HashSet<String>,
    mirrors: HashMap<u64, Mirror>,
    generation: u64,
    // A measured viewport does not change the source or editor identity used by
    // showTextDocument. Retain only the contiguous generation interval for that
    // case; source, selections, options, and membership changes reset it.
    show_generation_floor: u64,
    last_stamp: Value,
    api_selections: HashSet<u64>,
}
impl MirrorState {
    fn stamp_refs(documents: &[&Document], active: Option<&Document>) -> Value {
        if check_mirror_budget(documents).is_err() {
            return Value::Null;
        }
        json!({"documents": documents.iter().map(|d| json!([d.id, d.revision, d.text_epoch(), d.resource_epoch(), d.save_generation(), d.path, d.virtual_uri(), d.untitled_uri(), d.language_override(), d.language_epoch(), d.dirty(), d.eol, selections(d), d.editor_view_stamp()])).collect::<Vec<_>>(),
            "active": active.map(|d| d.id), "selections": active.map(selections)})
    }
    #[cfg(test)]
    fn next(&self, documents: &[Document], active: usize) -> Result<(Self, Value)> {
        self.next_refs(&documents.iter().collect::<Vec<_>>(), documents.get(active))
    }
    fn next_refs(
        &self,
        documents: &[&Document],
        active: Option<&Document>,
    ) -> Result<(Self, Value)> {
        self.next_refs_with_editor_stamp(documents, active, None)
    }
    fn next_refs_with_editor_stamp(
        &self,
        documents: &[&Document],
        active: Option<&Document>,
        editor_stamp: Option<&Value>,
    ) -> Result<(Self, Value)> {
        self.next_refs_with_inventory_stamps(documents, active, editor_stamp, None)
    }
    fn next_refs_with_inventory_stamps(
        &self,
        documents: &[&Document],
        active: Option<&Document>,
        editor_stamp: Option<&Value>,
        tabs_stamp: Option<&Value>,
    ) -> Result<(Self, Value)> {
        check_mirror_budget(documents)?;
        let tabs_unchanged = tabs_stamp.map_or(self.tabs_stamp.is_null(), |stamp| {
            Self::show_tabs_stamp(stamp) == Self::show_tabs_stamp(&self.tabs_stamp)
        });
        let viewport_only = tabs_unchanged
            && editor_stamp.is_some_and(|stamp| {
                !self.editor_stamp.is_null()
                    && Self::stamp_refs(documents, active) == self.last_stamp
                    && self.api_selections.is_empty()
                    && self.api_editor_selections.is_empty()
                    && Self::show_editor_stamp(stamp) == Self::show_editor_stamp(&self.editor_stamp)
            });
        // Only publish the new baseline after the transport accepts the update.
        let mut next = self.clone();
        let state = next.state(documents, active)?;
        if viewport_only {
            next.show_generation_floor = self.show_generation_floor;
        }
        if let Some(stamp) = editor_stamp {
            next.editor_stamp = stamp.clone();
        }
        if let Some(stamp) = tabs_stamp {
            next.tabs_stamp = stamp.clone();
        }
        Ok((next, state))
    }
    fn show_editor_stamp(stamp: &Value) -> Value {
        let mut stamp = stamp.clone();
        if let Some(editors) = stamp["editors"].as_array_mut() {
            for editor in editors {
                if let Some(editor) = editor.as_object_mut() {
                    editor.remove("visibleRanges");
                }
            }
        }
        stamp
    }
    /// Presentation flags do not retarget showTextDocument. Keep exact group
    /// and tab lifetimes, resource identity, order and active membership; only
    /// independent labels/dirty/pin/preview metadata may span this interval.
    fn show_tabs_stamp(stamp: &Value) -> Value {
        let mut stamp = stamp.clone();
        if let Some(groups) = stamp["groups"].as_array_mut() {
            for group in groups {
                if let Some(tabs) = group["tabs"].as_array_mut() {
                    for tab in tabs {
                        if let Some(tab) = tab.as_object_mut() {
                            for field in ["label", "isDirty", "isPinned", "isPreview"] {
                                tab.remove(field);
                            }
                        }
                    }
                }
            }
        }
        stamp
    }
    fn show_generation_current(&self, generation: Option<u64>) -> bool {
        generation.is_some_and(|generation| {
            generation >= self.show_generation_floor && generation <= self.generation
        })
    }
    fn state(&mut self, documents: &[&Document], active: Option<&Document>) -> Result<Value> {
        if documents.iter().map(|d| d.text.len_bytes()).sum::<usize>() > MAX_DOCUMENT_BYTES {
            bail!(
                "Extension document mirrors currently support at most 4 MiB total; native editing remains available"
            );
        }
        let mut snapshots = Vec::new();
        for doc in documents {
            let uri = document_uri(doc)?;
            let needs_text = self.mirrors.get(&doc.id).is_none_or(|old| {
                old.revision != doc.revision
                    || old.text_epoch != doc.text_epoch()
                    || old.resource_epoch != doc.resource_epoch()
                    || old.eol != doc.eol
            });
            let mirror = self.mirrors.entry(doc.id).or_insert_with(|| Mirror {
                editor_view: doc.editor_view_identity(),
                language_epoch: doc.language_epoch(),
                revision: doc.revision,
                text_epoch: doc.text_epoch(),
                resource_epoch: doc.resource_epoch(),
                save_generation: doc.save_generation(),
                uri: uri.clone(),
                eol: doc.eol.clone(),
                version: 1,
            });
            if mirror.revision != doc.revision
                || mirror.text_epoch != doc.text_epoch()
                || mirror.resource_epoch != doc.resource_epoch()
                || mirror.uri != uri
                || mirror.eol != doc.eol
            {
                mirror.version += 1;
                mirror.revision = doc.revision;
                mirror.text_epoch = doc.text_epoch();
                mirror.resource_epoch = doc.resource_epoch();
                mirror.uri = uri.clone();
                mirror.eol = doc.eol.clone();
            }
            mirror.editor_view = doc.editor_view_identity();
            mirror.language_epoch = doc.language_epoch();
            mirror.save_generation = doc.save_generation();
            let mut snapshot = json!({"savedGeneration": mirror.save_generation, "id":doc.id, "uri":uri, "version":mirror.version,
                "languageEpoch":doc.language_epoch(), "selections":selections(doc), "languageId":doc.language_override().unwrap_or_else(||doc.path.as_deref().map_or("plaintext", lsp::language)), "isDirty":doc.api_dirty(), "isReadonly":doc.is_read_only(),
                "eol":if doc.eol == "\r\n" {2} else {1}, "encoding":"utf8"});
            if self.api_selections.contains(&doc.id) {
                snapshot["selectionKind"] = 3.into();
            }
            if needs_text {
                snapshot["text"] = doc.text.to_string().into();
            }
            snapshots.push(snapshot);
        }
        self.api_selections.clear();
        let ids: HashSet<_> = documents.iter().map(|d| d.id).collect();
        self.mirrors.retain(|id, _| ids.contains(id));
        self.generation += 1;
        self.show_generation_floor = self.generation;
        self.last_stamp = Self::stamp_refs(documents, active);
        Ok(
            json!({"generation":self.generation, "documents":snapshots, "active":active.map(|d|d.id),
            "selections":active.map(selections).unwrap_or_default()}),
        )
    }
}
pub(crate) fn document_uri(doc: &Document) -> Result<String> {
    if let Some(uri) = doc.virtual_uri() {
        return Ok(uri.to_owned());
    }
    if let Some(uri) = doc.untitled_uri() {
        return Ok(uri.to_owned());
    }
    doc.path
        .as_deref()
        .map_or_else(|| Ok(format!("untitled:vscli-{}", doc.id)), lsp::file_uri)
}
fn selected_languages(packages: &[Package]) -> Result<HashSet<String>> {
    let mut languages = HashSet::new();
    for package in packages {
        let manifest = package.manifest()?;
        let declared = &manifest["contributes"]["languages"];
        if !declared.is_null() {
            let values = declared
                .as_array()
                .context("Invalid selected language contribution table")?;
            if values.len() > 128 {
                bail!("Selected language contributions exceed 128 entries");
            }
            for value in values {
                let id = value["id"]
                    .as_str()
                    .context("Invalid selected contributed language")?;
                if id.is_empty() || id.len() > 128 || id.contains('\0') {
                    bail!("Contributed language identifier exceeds bounds");
                }
                languages.insert(id.to_owned());
            }
        }
    }
    Ok(languages)
}
fn check_mirror_budget(documents: &[&Document]) -> Result<()> {
    if documents
        .iter()
        .map(|doc| doc.text.len_bytes())
        .sum::<usize>()
        > MAX_DOCUMENT_BYTES
    {
        bail!("Extension document mirrors admit at most 4 MiB total");
    }
    if documents.len() > 128
        || documents.iter().any(|doc| doc.secondary.len() >= 10_000)
        || documents
            .iter()
            .map(|doc| doc.secondary.len() + 1)
            .sum::<usize>()
            > 16_384
    {
        bail!(
            "Document mirrors admit 128 models, 10,000 selections/model and 16,384 total selections"
        );
    }
    Ok(())
}
fn selections(doc: &Document) -> Vec<Value> {
    selections_for(doc, None)
}
fn selections_for(doc: &Document, view: Option<u64>) -> Vec<Value> {
    // Scan each requested physical line prefix once, retaining caller order.
    // Aggregate source admission precedes calls from transport preparation.
    let view = doc.view_state(view);
    let selections = std::iter::once((view.anchor.unwrap_or(view.cursor), view.cursor))
        .chain(view.secondary.iter().map(|selection| {
            (
                selection.anchor.unwrap_or(selection.cursor),
                selection.cursor,
            )
        }))
        .collect::<Vec<_>>();
    let mut points = selections
        .iter()
        .flat_map(|(anchor, active)| [*anchor, *active])
        .enumerate()
        .map(|(index, offset)| (offset.min(doc.len()), index))
        .collect::<Vec<_>>();
    points.sort_by_key(|(offset, _)| *offset);
    let mut positions = vec![
        lsp::Position {
            line: 0,
            character: 0
        };
        points.len()
    ];
    let mut row = usize::MAX;
    let mut cursor = 0;
    let mut units = 0;
    for (offset, index) in points {
        let next_row = doc.text.char_to_line(offset);
        if row != next_row {
            row = next_row;
            cursor = doc.line_start(row);
            units = 0;
        }
        units += doc
            .text
            .slice(cursor..offset)
            .chars()
            .map(char::len_utf16)
            .sum::<usize>();
        cursor = offset;
        positions[index] = lsp::Position {
            line: row,
            character: units,
        };
    }
    (0..selections.len())
        .map(|index| json!({"anchor":positions[index*2],"active":positions[index*2+1]}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn show_generation_tolerates_only_contiguous_measured_viewport_updates() {
        let mut docs = [Document::from_text("dirty 猫🙂\r\n")];
        let stamp = json!({"activeEditor":"editor-1","editors":[{
            "id":"editor-1","document":docs[0].id,"viewColumn":1,
            "selections":[],"options":{"tabSize":4},"visibleRanges":[]}]});
        let (first, _) = MirrorState::default()
            .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&stamp))
            .unwrap();
        let mut measured = stamp.clone();
        measured["editors"][0]["visibleRanges"] =
            json!([{"start":{"line":0,"character":0},"end":{"line":1,"character":0}}]);
        let (second, _) = first
            .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&measured))
            .unwrap();
        assert!(second.show_generation_current(Some(first.generation)));
        measured["editors"][0]["visibleRanges"] = json!([]);
        let (third, _) = second
            .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&measured))
            .unwrap();
        assert!(third.show_generation_current(Some(first.generation)));
        assert!(!third.show_generation_current(None));
        assert!(!third.show_generation_current(Some(0)));
        assert!(!third.show_generation_current(Some(third.generation + 1)));
        for (field, value) in [
            ("id", json!("editor-replaced")),
            ("viewColumn", json!(2)),
            ("options", json!({"tabSize":8})),
            (
                "selections",
                json!([{"anchor":{"line":0,"character":0},"active":{"line":0,"character":1}}]),
            ),
        ] {
            let mut changed = measured.clone();
            changed["editors"][0][field] = value;
            let (next, _) = third
                .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&changed))
                .unwrap();
            assert!(
                !next.show_generation_current(Some(third.generation)),
                "{field}"
            );
        }
        let mut changed = measured.clone();
        changed["activeEditor"] = Value::Null;
        let (next, _) = third
            .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&changed))
            .unwrap();
        assert!(!next.show_generation_current(Some(third.generation)));
        let mut api = third.clone();
        api.api_editor_selections.insert("editor-1".into());
        let (next, _) = api
            .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&measured))
            .unwrap();
        assert!(!next.show_generation_current(Some(third.generation)));
        docs[0].insert("X", false);
        docs[0].undo();
        let (next, _) = third
            .next_refs_with_editor_stamp(&[&docs[0]], docs.first(), Some(&measured))
            .unwrap();
        assert!(
            !next.show_generation_current(Some(third.generation)),
            "source ABA"
        );
    }
    #[test]
    fn tab_metadata_preserves_show_interval_but_actual_membership_changes_retire_it() {
        let docs = [Document::from_text("猫🙂\r\n")];
        let mut groups = crate::editor_groups::Groups::with_empty_group();
        let member = groups.open(docs[0].id).unwrap().active.unwrap();
        let mut inventory = editor_tabs::Inventory::default();
        inventory.observe(&groups, &[&docs[0]]).unwrap();
        let tabs = inventory.wire().unwrap();
        let stamp = json!({"activeEditor":"editor-1","editors":[{
            "id":"editor-1","document":docs[0].id,"viewColumn":1,
            "selections":[],"options":{"tabSize":4},"visibleRanges":[]}]});
        let (first, _) = MirrorState::default()
            .next_refs_with_inventory_stamps(&[&docs[0]], docs.first(), Some(&stamp), Some(&tabs))
            .unwrap();
        let mut measured = stamp.clone();
        measured["editors"][0]["visibleRanges"] =
            json!([{"start":{"line":0,"character":0},"end":{"line":1,"character":0}}]);
        let (second, _) = first
            .next_refs_with_inventory_stamps(
                &[&docs[0]],
                docs.first(),
                Some(&measured),
                Some(&tabs),
            )
            .unwrap();
        assert!(second.show_generation_current(Some(first.generation)));
        groups.set_sticky(member, true).unwrap();
        inventory.observe(&groups, &[&docs[0]]).unwrap();
        let changed_tabs = inventory.wire().unwrap();
        assert_ne!(changed_tabs, tabs);
        let (third, state) = second
            .next_refs_with_inventory_stamps(
                &[&docs[0]],
                docs.first(),
                Some(&stamp),
                Some(&changed_tabs),
            )
            .unwrap();
        assert!(third.show_generation_current(Some(first.generation)));
        assert!(third.show_generation_current(Some(second.generation)));
        assert_eq!(third.tabs_stamp, changed_tabs);
        // A new shared membership retires old Show requests even if the test's
        // visible-editor/document stamp has not advanced yet.
        groups.split_active().unwrap();
        inventory.observe(&groups, &[&docs[0]]).unwrap();
        let topology = inventory.wire().unwrap();
        let (fourth, _) = third
            .next_refs_with_inventory_stamps(
                &[&docs[0]],
                docs.first(),
                Some(&measured),
                Some(&topology),
            )
            .unwrap();
        assert!(!fourth.show_generation_current(Some(third.generation)));
        assert!(!fourth.show_generation_current(Some(first.generation)));
        assert_eq!(state["documents"][0]["version"], 1);
        assert_eq!(docs[0].text.to_string(), "猫🙂\r\n");
        assert!(!docs[0].dirty());
    }
    #[test]
    fn prepared_tab_inventory_contains_real_empty_group_and_original_published_proof() {
        let groups = crate::editor_groups::Groups::with_empty_group();
        let mut inventory = editor_tabs::Inventory::default();
        inventory.observe(&groups, &[]).unwrap();
        let expected = inventory.wire().unwrap();
        let prepared = Client::prepare(&[], 0, &Settings::default())
            .unwrap()
            .with_editor_tabs(inventory)
            .unwrap();
        assert_eq!(prepared.state["tabs"], expected);
        assert_eq!(prepared.mirror.tabs_stamp, expected);
        assert_eq!(prepared.state["tabs"]["groups"][0]["tabs"], json!([]));
        let id = expected["groups"][0]["id"].as_str().unwrap().to_owned();
        let capture = prepared
            .mirror
            .tabs
            .as_ref()
            .unwrap()
            .capture_published("groups", &[id])
            .unwrap();
        assert!(capture.current(prepared.editor_tabs.as_ref().unwrap(), &groups));
    }
    #[test]
    fn editor_state_mirrors_inactive_selections_language_and_command_kind_without_text_version() {
        let mut documents = vec![Document::from_text("α😀\r\n"), Document::from_text("other")];
        let (mut first, _) = MirrorState::default().next(&documents, 1).unwrap();
        documents[0].set_selections(vec![crate::document::Selection {
            cursor: 2,
            anchor: Some(0),
            desired_column: None,
        }]);
        documents[0]
            .set_language_override(Some("python".into()))
            .unwrap();
        first.api_selections.insert(documents[0].id);
        let (next, state) = first.next(&documents, 1).unwrap();
        assert_eq!(state["active"], documents[1].id);
        assert_eq!(state["documents"][0]["version"], 1);
        assert_eq!(state["documents"][0]["languageId"], "python");
        assert_eq!(state["documents"][0]["languageEpoch"], 1);
        assert_eq!(state["documents"][0]["selectionKind"], 3);
        assert_eq!(
            state["documents"][0]["selections"],
            json!([{"anchor":{"line":0,"character":0},"active":{"line":0,"character":3}}])
        );
        assert!(state["documents"][0].get("text").is_none());
        assert!(!first.api_selections.is_empty());
        assert!(next.api_selections.is_empty());
        let (_, later) = next.next(&documents, 1).unwrap();
        assert!(later["documents"][0].get("selectionKind").is_none());
    }
    #[test]
    fn editor_state_selected_languages_validate_actual_manifest_identity_and_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        fs::write(&path, serde_json::to_vec(&json!({"publisher":"fixture","name":"language","version":"1.0.0","contributes":{"languages":[{"id":"fixture-language"}]}})).unwrap()).unwrap();
        let package = Package::read(dir.path()).unwrap();
        assert!(
            selected_languages(std::slice::from_ref(&package))
                .unwrap()
                .contains("fixture-language")
        );
        fs::write(&path, serde_json::to_vec(&json!({"publisher":"fixture","name":"language","version":"2.0.0","contributes":{"languages":[{"id":"fixture-language"}]}})).unwrap()).unwrap();
        assert!(selected_languages(std::slice::from_ref(&package)).is_err());
        fs::write(&path, serde_json::to_vec(&json!({"publisher":"fixture","name":"language","version":"1.0.0","contributes":{"languages":{}}})).unwrap()).unwrap();
        assert!(selected_languages(std::slice::from_ref(&package)).is_err());
    }
    #[test]
    fn hidden_models_have_no_active_editor_and_keep_versions_when_shown() {
        let hidden = vec![Document::from_text("hidden α\r\n")];
        let id = hidden[0].id;
        let initial = Client::prepare_with_hidden(&[], &hidden, 0, &Settings::default()).unwrap();
        assert!(initial.state["active"].is_null());
        assert_eq!(initial.state["selections"], json!([]));
        assert_eq!(initial.state["documents"][0]["id"], id);
        let (shown, state) = initial.mirror.next(&hidden, 0).unwrap();
        assert_eq!(state["active"], id);
        assert_eq!(state["documents"][0]["version"], 1);
        assert!(state["documents"][0].get("text").is_none());
        assert_eq!(shown.mirrors.len(), 1);
        let huge = vec![Document::from_text(&"x".repeat(MAX_DOCUMENT_BYTES))];
        assert!(Client::prepare_with_hidden(&huge, &hidden, 0, &Settings::default()).is_err());
    }
    #[test]
    fn cursor_updates_are_bounded_metadata_and_edits_only_send_changed_documents() {
        let mut docs = vec![
            Document::from_text(&"0123456789abcdef\n".repeat(100_000)),
            Document::from_text("other"),
        ];
        let baseline = MirrorState::default();
        let (first, initial) = baseline.next(&docs, 0).unwrap();
        assert!(serde_json::to_vec(&initial).unwrap().len() > 1_700_000);
        // Encoding an update that never gets queued cannot advance the baseline.
        assert_eq!(baseline.next(&docs, 0).unwrap().1, initial);
        docs[0].move_to(20, false);
        let (second, cursor) = first.next(&docs, 0).unwrap();
        assert!(serde_json::to_vec(&cursor).unwrap().len() < 1024);
        assert!(
            cursor["documents"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| d.get("text").is_none())
        );
        assert_eq!(cursor["documents"][0]["version"], 1);
        docs[1].insert("!", false);
        let (third, edited) = second.next(&docs, 1).unwrap();
        assert!(edited["documents"][0].get("text").is_none());
        assert_eq!(edited["documents"][1]["text"], "!other");
        assert_eq!(edited["documents"][1]["version"], 2);
        docs[1].undo();
        let (_, undone) = third.next(&docs, 1).unwrap();
        assert_eq!(undone["documents"][1]["version"], 3);
        assert_eq!(undone["documents"][1]["text"], "other");
    }
    #[test]
    fn untitled_mirror_dirty_state_survives_undo_without_changing_tab_cleanliness() {
        let mut docs = vec![Document::from_text("")];
        let (first, initial) = MirrorState::default().next(&docs, 0).unwrap();
        assert_eq!(initial["documents"][0]["isDirty"], false);
        docs[0].insert("猫🙂\r\n", false);
        let (second, edited) = first.next(&docs, 0).unwrap();
        assert_eq!(edited["documents"][0]["isDirty"], true);
        docs[0].undo();
        let (third, undone) = second.next(&docs, 0).unwrap();
        assert_eq!(undone["documents"][0]["isDirty"], true);
        assert_eq!(undone["documents"][0]["text"], "");
        assert_eq!(undone["documents"][0]["version"], 3);
        assert!(!docs[0].dirty());
        docs[0].redo();
        let (_, redone) = third.next(&docs, 0).unwrap();
        assert_eq!(redone["documents"][0]["text"], "猫🙂\r\n");
        assert_eq!(redone["documents"][0]["isDirty"], true);
        assert_eq!(redone["documents"][0]["version"], 4);
    }
    #[test]
    fn mirror_eol_metadata_is_versioned_without_modifying_document_or_disk() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("eol.txt");
        let original = "猫🙂\r\nsecond\nthird";
        std::fs::write(&file, original).unwrap();
        let mut docs = vec![Document::open_existing(&file).unwrap()];
        let (first, initial) = MirrorState::default().next(&docs, 0).unwrap();
        assert_eq!(initial["documents"][0]["eol"], 2);
        assert_eq!(initial["documents"][0]["encoding"], "utf8");
        let stamp = MirrorState::stamp_refs(&[&docs[0]], Some(&docs[0]));
        let revision = docs[0].revision;
        docs[0].eol = "\n".into();
        assert_ne!(stamp, MirrorState::stamp_refs(&[&docs[0]], Some(&docs[0])));
        let (_, updated) = first.next(&docs, 0).unwrap();
        assert_eq!(updated["documents"][0]["version"], 2);
        assert_eq!(updated["documents"][0]["eol"], 1);
        assert_eq!(updated["documents"][0]["text"], original);
        assert_eq!(updated["documents"][0]["isDirty"], false);
        assert_eq!(docs[0].revision, revision);
        assert_eq!(docs[0].text.to_string(), original);
        assert_eq!(std::fs::read_to_string(file).unwrap(), original);
    }
    #[test]
    fn path_dirty_and_lifecycle_updates_preserve_content_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut docs = vec![Document::from_text("original")];
        let (first, _) = MirrorState::default().next(&docs, 0).unwrap();
        docs[0].path = Some(directory.path().join("renamed.rs"));
        let (second, renamed) = first.next(&docs, 0).unwrap();
        assert_eq!(renamed["documents"][0]["version"], 2);
        assert_eq!(renamed["documents"][0]["languageId"], "rust");
        assert!(renamed["documents"][0].get("text").is_none());
        docs[0].saved_revision = u64::MAX;
        let (third, dirty) = second.next(&docs, 0).unwrap();
        assert_eq!(dirty["documents"][0]["version"], 2);
        assert_eq!(dirty["documents"][0]["isDirty"], true);
        assert!(dirty["documents"][0].get("text").is_none());
        let (closed, update) = third.next(&[], 0).unwrap();
        assert_eq!(update["documents"], json!([]));
        assert!(update["active"].is_null());
        let (_, reopened) = closed.next(&docs, 0).unwrap();
        assert_eq!(reopened["documents"][0]["text"], "original");
    }
    #[test]
    fn mirror_text_epoch_advances_version_after_edit_undo_revision_reuse() {
        let mut docs = vec![Document::from_text("猫\r\n🙂")];
        let revision = docs[0].revision;
        let (first, _) = MirrorState::default().next(&docs, 0).unwrap();
        docs[0].insert("transient", false);
        docs[0].undo();
        assert_eq!(docs[0].revision, revision);
        assert_ne!(
            MirrorState::stamp_refs(&docs.iter().collect::<Vec<_>>(), docs.first()),
            first.last_stamp
        );
        let (second, restored) = first.next(&docs, 0).unwrap();
        assert_eq!(restored["documents"][0]["version"], 2);
        assert_eq!(restored["documents"][0]["text"], "猫\r\n🙂");
        docs[0].move_to(1, false);
        let (_, cursor) = second.next(&docs, 0).unwrap();
        assert_eq!(cursor["documents"][0]["version"], 2);
        assert!(cursor["documents"][0].get("text").is_none());
    }
    #[test]
    fn outdated_owner_edits_and_expired_heartbeat_cannot_mutate_native_documents() {
        let directory = tempfile::tempdir().unwrap();
        let extension =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut documents = vec![Document::from_text("original")];
        let settings = Settings::default();
        let mut client = Client::start(
            "node",
            &extension,
            directory.path(),
            &documents,
            0,
            &settings,
        )
        .unwrap();
        let edit = |session, owner| Edit {
            editor: None,
            session,
            owner,
            document: documents[0].id,
            version: 1,
            edits: Vec::new(),
        };
        assert!(
            !client
                .apply_edit(
                    edit(client.session + 1, client.packages[0].id.clone()),
                    &mut [Document::from_text("unrelated")]
                )
                .unwrap()
        );
        let edit = Edit {
            editor: None,
            session: client.session,
            owner: "unknown.owner".into(),
            document: documents[0].id,
            version: 1,
            edits: Vec::new(),
        };
        assert!(!client.apply_edit(edit, &mut documents).unwrap());
        let original_eol = documents[0].eol.clone();
        documents[0].eol = "\r\n".into();
        let stale_eol_edit = Edit {
            editor: None,
            session: client.session,
            owner: client.packages[0].id.clone(),
            document: documents[0].id,
            version: 1,
            edits: vec![
                serde_json::from_value(json!({
                    "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},
                    "newText":"stale eol"
                }))
                .unwrap(),
            ],
        };
        assert!(!client.service_document_current(&documents[0], 1));
        assert!(!client.apply_edit(stale_eol_edit, &mut documents).unwrap());
        assert_eq!(documents[0].text.to_string(), "original");
        documents[0].eol = original_eol;
        assert!(client.service_document_current(&documents[0], 1));
        documents[0].insert("transient", false);
        documents[0].undo();
        let edit = Edit {
            editor: None,
            session: client.session,
            owner: client.packages[0].id.clone(),
            document: documents[0].id,
            version: 1,
            edits: vec![
                serde_json::from_value(json!({
                    "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},
                    "newText":"stale"
                }))
                .unwrap(),
            ],
        };
        // Reject even before a state sync advances the exposed host version.
        assert!(!client.apply_edit(edit, &mut documents).unwrap());
        assert_eq!(documents[0].text.to_string(), "original");
        client.pending.insert(
            u64::MAX,
            Pending {
                owner: None,
                method: "ping".into(),
                started: Instant::now() - Duration::from_secs(31),
            },
        );
        assert!(
            client
                .poll(&mut documents, 0, &settings)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert_eq!(documents[0].text.to_string(), "original");
    }
}
