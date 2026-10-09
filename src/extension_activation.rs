//! Native activation planning. Reading manifests and starting code are separate operations.
use crate::{extension_store::Installed, extensions::Package};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ENABLED: usize = 128;
pub const MAX_SELECTED: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Workspace,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub schema: u32,
    pub extensions: BTreeMap<String, bool>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema: 1,
            extensions: BTreeMap::new(),
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 || self.extensions.len() > MAX_ENABLED {
            bail!("Extension enablement requires schema 1 and at most 128 entries");
        }
        for id in self.extensions.keys() {
            validate_id(id)?;
        }
        Ok(())
    }
    pub fn set(&mut self, id: &str, enabled: bool) -> Result<()> {
        let id = id.to_ascii_lowercase();
        validate_id(&id)?;
        if !self.extensions.contains_key(&id) && self.extensions.len() >= MAX_ENABLED {
            bail!("Extension enablement supports at most 128 entries");
        }
        self.extensions.insert(id, enabled);
        Ok(())
    }
    pub fn enabled(global: &Self, workspace: &Self, id: &str) -> bool {
        workspace
            .extensions
            .get(id)
            .or_else(|| global.extensions.get(id))
            .copied()
            .unwrap_or(false)
    }
    pub fn effective(global: &Self, workspace: &Self) -> BTreeSet<String> {
        global
            .extensions
            .keys()
            .chain(workspace.extensions.keys())
            .filter(|id| Self::enabled(global, workspace, id))
            .cloned()
            .collect()
    }
}

pub fn validate_id(id: &str) -> Result<()> {
    if id != id.to_ascii_lowercase()
        || id.split('.').count() != 2
        || id.split('.').any(|part| {
            part.is_empty()
                || part.len() > 100
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    {
        bail!("Extension ID must be lowercase publisher.name: {id}");
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Event {
    Always,
    StartupFinished,
    Command(String),
    Language(String),
    WorkspaceContains(String),
}
impl Event {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "*" => Some(Self::Always),
            "onStartupFinished" => Some(Self::StartupFinished),
            _ => {
                let (prefix, value) = value.split_once(':')?;
                if value.is_empty() {
                    return None;
                }
                match prefix {
                    "onCommand" => Some(Self::Command(value.into())),
                    "onLanguage" => Some(Self::Language(value.into())),
                    "workspaceContains" => Some(Self::WorkspaceContains(value.into())),
                    _ => None,
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub package: Package,
    pub dependencies: BTreeSet<String>,
    pub events: BTreeSet<Event>,
    pub unsupported_events: BTreeSet<String>,
    pub commands: BTreeMap<String, String>,
    pub keybindings: Value,
    pub code: bool,
    pub blocked: Option<String>,
}
fn strings(value: &Value, limit: usize, name: &str) -> Result<Vec<String>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .with_context(|| format!("Invalid extension {name}"))?;
    if values.len() > limit {
        bail!("Extension {name} exceeds {limit} entries");
    }
    values
        .iter()
        .map(|v| {
            let value = v
                .as_str()
                .with_context(|| format!("Invalid extension {name} entry"))?;
            if value.len() > 1024 {
                bail!("Extension {name} entry exceeds 1 KiB");
            }
            Ok(value.to_owned())
        })
        .collect()
}
impl Metadata {
    pub fn installed(item: &Installed) -> Result<Self> {
        validate_id(&item.id)?;
        let manifest = &item.manifest;
        let mut dependencies = BTreeSet::new();
        for id in strings(
            &manifest["extensionDependencies"],
            MAX_SELECTED,
            "dependencies",
        )? {
            let id = id.to_ascii_lowercase();
            validate_id(&id)?;
            dependencies.insert(id);
        }
        let mut events = BTreeSet::new();
        let mut unsupported_events = BTreeSet::new();
        for event in strings(&manifest["activationEvents"], 128, "activation events")? {
            if let Some(event) = Event::parse(&event) {
                events.insert(event);
            } else {
                unsupported_events.insert(event);
            }
        }
        let mut commands = BTreeMap::new();
        let contribution = &manifest["contributes"]["commands"];
        if !contribution.is_null() {
            let values = contribution
                .as_array()
                .context("Invalid command contributions")?;
            if values.len() > 1024 {
                bail!("Extension commands exceed 1024 entries");
            }
            for value in values {
                let id = value["command"]
                    .as_str()
                    .context("Missing contributed command ID")?;
                let title = value["title"]
                    .as_str()
                    .context("Missing contributed command title")?;
                if id.is_empty() || id.len() > 1024 || title.len() > 4096 {
                    bail!("Contributed command text exceeds activation limits");
                }
                if commands.insert(id.into(), title.into()).is_some() {
                    bail!("Duplicate contributed command: {id}");
                }
                events.insert(Event::Command(id.into()));
            }
        }
        let languages = &manifest["contributes"]["languages"];
        if !languages.is_null() {
            let values = languages
                .as_array()
                .context("Invalid language contributions")?;
            if values.len() > 128 {
                bail!("Extension languages exceed 128 entries");
            }
            for value in values {
                let id = value["id"]
                    .as_str()
                    .context("Missing contributed language ID")?;
                if id.is_empty() || id.len() > 1024 {
                    bail!("Invalid contributed language ID");
                }
                events.insert(Event::Language(id.into()));
            }
        }
        let event_bytes: usize = events
            .iter()
            .map(|e| match e {
                Event::Command(s) | Event::Language(s) | Event::WorkspaceContains(s) => s.len(),
                _ => 1,
            })
            .sum();
        let unsupported_bytes: usize = unsupported_events.iter().map(String::len).sum();
        if events.len() > 1024 || event_bytes + unsupported_bytes > 64 * 1024 {
            bail!("Extension activation metadata exceeds 1024 events / 64 KiB");
        }
        let code = manifest["main"].is_string();
        let blocked = if manifest["enabledApiProposals"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            Some("Proposed extension APIs are unsupported".into())
        } else if !code && !manifest["browser"].is_null() {
            Some("Browser extension runtime is unsupported".into())
        } else {
            None
        };
        Ok(Self {
            package: Package::installed(item),
            dependencies,
            events,
            unsupported_events,
            commands,
            keybindings: manifest["contributes"]["keybindings"].clone(),
            code,
            blocked,
        })
    }
    pub fn matches(&self, event: &Event) -> bool {
        self.events.contains(&Event::Always) || self.events.contains(event)
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    /// Dependencies precede dependants; independent siblings use package-ID order.
    pub ordered: Vec<Metadata>,
}
impl Plan {
    pub fn needs_code(&self) -> bool {
        self.ordered.iter().any(|m| m.code)
    }
}
pub fn plan(
    catalog: &BTreeMap<String, Metadata>,
    enabled: &BTreeSet<String>,
    retained: &[Metadata],
    roots: &BTreeSet<String>,
) -> Result<Plan> {
    if catalog.len() > MAX_ENABLED || enabled.len() > MAX_ENABLED || retained.len() > MAX_SELECTED {
        bail!("Extension activation catalog budget exceeded");
    }
    let mut available = catalog.clone();
    // Selected generations remain immutable through upgrades and rollback.
    let mut retained_ids = BTreeSet::new();
    for item in retained {
        if !retained_ids.insert(item.package.id.clone()) {
            bail!("Duplicate selected extension: {}", item.package.id);
        }
        available.insert(item.package.id.clone(), item.clone());
    }
    let mut ordered = Vec::new();
    let mut visiting = Vec::new();
    let mut finished = BTreeSet::new();
    for id in retained
        .iter()
        .map(|m| m.package.id.clone())
        .collect::<BTreeSet<_>>()
        .iter()
        .chain(roots.iter())
    {
        visit(
            id,
            &available,
            enabled,
            &mut visiting,
            &mut finished,
            &mut ordered,
        )?;
    }
    Ok(Plan { ordered })
}
fn visit(
    id: &str,
    available: &BTreeMap<String, Metadata>,
    enabled: &BTreeSet<String>,
    visiting: &mut Vec<String>,
    finished: &mut BTreeSet<String>,
    ordered: &mut Vec<Metadata>,
) -> Result<()> {
    if finished.contains(id) {
        return Ok(());
    }
    if visiting.iter().any(|v| v == id) {
        bail!(
            "Extension dependency cycle: {} -> {id}",
            visiting.join(" -> ")
        );
    }
    if !enabled.contains(id) {
        bail!("Extension {id} is disabled; enable it explicitly before activation");
    }
    let metadata = available
        .get(id)
        .with_context(|| format!("Required extension {id} is not installed"))?;
    if let Some(reason) = &metadata.blocked {
        bail!("Extension {id} cannot activate: {reason}");
    }
    if visiting.len() >= MAX_SELECTED {
        bail!("Extension dependency closure exceeds eight selected packages");
    }
    visiting.push(id.into());
    for dependency in &metadata.dependencies {
        visit(dependency, available, enabled, visiting, finished, ordered)?;
    }
    visiting.pop();
    if ordered.len() >= MAX_SELECTED {
        bail!(
            "Extension cohort exceeds eight selected packages; stop a cohort before selecting more"
        );
    }
    finished.insert(id.into());
    ordered.push(metadata.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn metadata(id: &str, dependencies: &[&str]) -> Metadata {
        let (publisher, name) = id.split_once('.').unwrap();
        Metadata::installed(&Installed { id: id.into(), version: "1.0.0".into(),
            path: format!("/snapshot/{id}").into(), source: "fixture".into(), sha256: "hash".into(), compatibility: "fixture".into(),
            manifest: json!({"publisher":publisher,"name":name,"main":"main.cjs","extensionDependencies":dependencies}) }).unwrap()
    }
    #[test]
    fn consent_defaults_disabled_and_workspace_override_does_not_change_global_state() {
        let mut global = Preferences::default();
        let mut workspace = Preferences::default();
        assert!(!Preferences::enabled(&global, &workspace, "test.a"));
        global.set("TEST.A", true).unwrap();
        workspace.set("test.a", false).unwrap();
        assert!(!Preferences::enabled(&global, &workspace, "test.a"));
        assert!(global.extensions["test.a"]);
        workspace.set("test.b", true).unwrap();
        assert_eq!(
            Preferences::effective(&global, &workspace),
            BTreeSet::from(["test.b".into()])
        );
        global.extensions.insert("bad/id".into(), true);
        assert!(global.validate().is_err());
    }
    #[test]
    fn dependencies_activate_once_in_stable_order_and_keep_selected_versions() {
        let a = metadata("test.a", &["test.b", "test.c"]);
        let b = metadata("test.b", &["test.c"]);
        let c = metadata("test.c", &[]);
        let mut catalog = BTreeMap::from([
            (a.package.id.clone(), a),
            (b.package.id.clone(), b),
            (c.package.id.clone(), c.clone()),
        ]);
        let enabled = catalog.keys().cloned().collect();
        catalog.get_mut("test.c").unwrap().package.version = "2.0.0".into();
        let result = plan(&catalog, &enabled, &[c], &BTreeSet::from(["test.a".into()])).unwrap();
        assert_eq!(
            result
                .ordered
                .iter()
                .map(|m| m.package.id.as_str())
                .collect::<Vec<_>>(),
            ["test.c", "test.b", "test.a"]
        );
        assert_eq!(result.ordered[0].package.version, "1.0.0");
    }
    #[test]
    fn contribution_events_are_inferred_and_unsupported_events_remain_explicit() {
        let mut item = Installed {
            id: "test.events".into(),
            version: "1.0.0".into(),
            path: "/snapshot".into(),
            source: "fixture".into(),
            sha256: "hash".into(),
            compatibility: "fixture".into(),
            manifest: json!({"main":"main.cjs","activationEvents":["onStartupFinished","workspaceContains:**/*.toml","onDebug:fixture"],
                "contributes":{"commands":[{"command":"events.run","title":"Run"}],"languages":[{"id":"cpp"}]}}),
        };
        let result = Metadata::installed(&item).unwrap();
        assert!(result.matches(&Event::StartupFinished));
        assert!(result.matches(&Event::Command("events.run".into())));
        assert!(result.matches(&Event::Language("cpp".into())));
        assert!(!result.matches(&Event::Language("python".into())));
        assert_eq!(
            result.unsupported_events,
            BTreeSet::from(["onDebug:fixture".into()])
        );
        item.manifest["activationEvents"] = json!(["*", "onLanguage:rust"]);
        assert!(
            Metadata::installed(&item)
                .unwrap()
                .matches(&Event::Language("python".into()))
        );
        item.manifest["activationEvents"] = json!("onCommand:events.run");
        assert!(Metadata::installed(&item).is_err());
    }
    #[test]
    fn disabled_missing_cyclic_and_oversized_dependency_graphs_do_not_create_a_plan() {
        let mut catalog = BTreeMap::from([("test.a".into(), metadata("test.a", &["test.b"]))]);
        let roots = BTreeSet::from(["test.a".into()]);
        let mut enabled = roots.clone();
        assert!(
            plan(&catalog, &enabled, &[], &roots)
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
        enabled.insert("test.b".into());
        assert!(
            plan(&catalog, &enabled, &[], &roots)
                .unwrap_err()
                .to_string()
                .contains("not installed")
        );
        catalog.insert("test.b".into(), metadata("test.b", &["test.a"]));
        assert!(
            plan(&catalog, &enabled, &[], &roots)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        let catalog = (0..9)
            .map(|i| {
                let id = format!("test.p{i}");
                (id.clone(), metadata(&id, &[]))
            })
            .collect::<BTreeMap<_, _>>();
        let enabled = catalog.keys().cloned().collect();
        assert!(
            plan(&catalog, &enabled, &[], &enabled)
                .unwrap_err()
                .to_string()
                .contains("eight")
        );
    }
}
