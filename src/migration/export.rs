//! Native .code-profile decoding into the existing atomic migration pipeline.
//! Format reference: VS Code 1.95.0 userDataProfileImportExportService and resource classes.
use super::*;
use std::collections::BTreeSet;

#[derive(Debug)]
pub(super) struct Extension {
    pub id: String,
    version: Option<String>,
    pre_release: bool,
    disabled: bool,
    application_scoped: bool,
}
impl Extension {
    pub(super) fn matches(
        &self,
        id: &str,
        version: &str,
        metadata: Option<&Value>,
        built_in: bool,
    ) -> bool {
        self.id == id
            && self.version.as_ref().map_or_else(
                || {
                    built_in
                        || self.pre_release
                            == metadata
                                .and_then(|value| value.get("preRelease"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                },
                |expected| expected == version,
            )
    }
    fn description(&self) -> String {
        format!(
            "{}{}",
            self.id,
            self.version.as_ref().map_or_else(
                || if self.pre_release {
                    " (pre-release)".into()
                } else {
                    String::new()
                },
                |version| format!("@{version}"),
            )
        )
    }
}

pub fn preview(
    path: &Path,
    profile: crate::keys::Profile,
    extensions: &Path,
    application: Option<application::Application>,
) -> Result<Preview> {
    preview_source(
        &discovery::Source {
            id: "export".into(),
            name: "Exported profile".into(),
            product: "VS Code export".into(),
            user: path.into(),
            default_user: path.into(),
            extensions: extensions.into(),
            use_default: Default::default(),
            application,
        },
        profile,
    )
}

pub(super) fn preview_source(
    source: &discovery::Source,
    profile: crate::keys::Profile,
) -> Result<Preview> {
    let path = fs::canonicalize(&source.user).context("Cannot open exported VS Code profile")?;
    let original = read(&path, MAX_TOTAL)?;
    let document: Value =
        serde_json::from_slice(&original).context("Invalid .code-profile JSON")?;
    let object = document
        .as_object()
        .context("Exported profile must contain an object")?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty() && name.len() <= 1024)
        .context("Exported profile needs a nonempty name of at most 1024 bytes")?;
    for field in [
        "icon",
        "settings",
        "keybindings",
        "tasks",
        "snippets",
        "globalState",
        "extensions",
    ] {
        if object.get(field).is_some_and(|value| !value.is_string()) {
            bail!("Exported profile {field} must be a JSON string when present");
        }
    }
    let requests = extension_requests(&document)?;
    let temporary = tempfile::tempdir()?;
    let mut notices = Vec::new();
    let mut total = original.len();
    for resource in ["settings", "keybindings", "tasks"] {
        if let Some(wrapper) = resource_value(&document, resource)? {
            let text = wrapper.get(resource).context(format!(
                "Exported {resource} wrapper is missing its content"
            ))?;
            if text.is_null() {
                continue;
            }
            let text = text
                .as_str()
                .context(format!("Exported {resource} must be a string or null"))?;
            add_file(
                temporary.path(),
                Path::new(&format!("{resource}.json")),
                text,
                &mut total,
            )?;
            if resource == "keybindings" {
                let expected = match profile {
                    crate::keys::Profile::Macos => 1,
                    crate::keys::Profile::Linux => 2,
                    crate::keys::Profile::Windows => 3,
                };
                if wrapper.get("platform").and_then(Value::as_u64) != Some(expected) {
                    notices.push("Exported keybinding platform differs from the selected keyboard profile or is absent; rules are preserved without translating physical keys".into());
                }
            }
        }
    }
    if let Some(wrapper) = resource_value(&document, "snippets")? {
        let snippets = wrapper
            .get("snippets")
            .and_then(Value::as_object)
            .context("Exported snippets wrapper must contain a snippets object")?;
        if snippets.len() > 128 {
            bail!("Import exceeds 128 snippet files");
        }
        let mut names = BTreeSet::new();
        for (name, text) in snippets {
            snippet_name(name)?;
            if !names.insert(name.to_lowercase()) {
                bail!("Exported snippets contain case-colliding filenames");
            }
            let text = text
                .as_str()
                .context("Exported snippet content must be a string")?;
            add_file(
                temporary.path(),
                &Path::new("snippets").join(name),
                text,
                &mut total,
            )?;
        }
    }
    // Only this private resource view is temporary. Preview owns all configuration
    // bytes; package roots and the original export remain the source identities.
    let layout = discovery::Source {
        user: temporary.path().into(),
        default_user: temporary.path().into(),
        ..source.clone()
    };
    let mut inventory = packages::inventory_selected(&layout, Some(&requests))?;
    let mut unresolved = Vec::new();
    inventory.selected.retain(|package| {
        if !packages::platform_matches(package.target_platform.as_deref()) {
            unresolved.push(format!(
                "{}@{}: local package platform does not match this editor",
                package.id, package.version
            ));
            false
        } else {
            true
        }
    });
    for request in &requests {
        if let Some(package) = inventory
            .selected
            .iter_mut()
            .find(|package| package.id == request.id)
        {
            package.enabled = !request.disabled;
            if request.application_scoped {
                notices.push(format!("{}: application-scoped intent is retained in source.code-profile; native package scope is this imported profile", request.id));
            }
        } else {
            unresolved.push(format!(
                "{}: matching registered local package or selected built-in was not found",
                request.description()
            ));
        }
    }
    for package in inventory.selected.iter().filter(|package| package.enabled) {
        for dependency in &package.dependencies {
            if !inventory
                .selected
                .iter()
                .any(|package| package.id == *dependency && package.enabled)
            {
                unresolved.push(format!(
                    "{}: required dependency {dependency} is missing or disabled",
                    package.id
                ));
            }
        }
    }
    unresolved.sort();
    unresolved.dedup();
    let mut result = preview_layout(
        temporary.path(),
        profile,
        Some(&source.extensions),
        Some(layout),
        Some(&inventory),
    )?;
    // The inventory index in an export is the original file, not the discarded view.
    inventory.index = path.clone();
    result.report.extensions = Some(inventory);
    result.report.unresolved_extensions = unresolved;
    result.report.notices.extend(notices);
    if document.get("globalState").is_some() {
        result.report.notices.push("Exported globalState is preserved in source.code-profile; workbench state, accounts and extension storage are not applied".into());
    }
    if !result.report.unresolved_extensions.is_empty() {
        result.report.notices.push(format!("{} unresolved extension requirements: package import is blocked until resolved; configuration-only import remains available", result.report.unresolved_extensions.len()));
    }
    result.report.notices.push("The complete original export, including unknown fields, is retained as source.code-profile. Enabled built-ins are normally absent from an export; choose a source application to include them. Exported files may omit machine-specific settings and do not establish full setup fidelity.".into());
    let mut original_source = source.clone();
    original_source.user = path.clone();
    original_source.default_user = path.clone();
    original_source.name = name.into();
    result.report.profile = Some(original_source);
    result.report.source = path.clone();
    result.report.bytes += original.len();
    if result.report.bytes > MAX_TOTAL {
        bail!("Import exceeds 16 MiB including original export and selected theme");
    }
    let archive = PathBuf::from("source.code-profile");
    result.report.files.push(archive.clone());
    result.files.push((archive, original));
    result.source_roots = vec![path];
    result.source_roots.extend(
        result
            .report
            .extensions
            .as_ref()
            .unwrap()
            .selected
            .iter()
            .map(|package| package.path.clone()),
    );
    if source.extensions.exists() {
        result
            .source_roots
            .push(fs::canonicalize(&source.extensions)?);
    }
    if let Some(application) = &source.application {
        result.source_roots.push(application.root.clone());
    }
    Ok(result)
}

fn resource_value(document: &Value, key: &str) -> Result<Option<Value>> {
    document
        .get(key)
        .and_then(Value::as_str)
        .map(|text| {
            let value: Value = serde_json::from_str(text)
                .with_context(|| format!("Invalid exported {key} resource JSON"))?;
            if !value.is_object() {
                bail!("Exported {key} resource must contain an object");
            }
            Ok(value)
        })
        .transpose()
}

fn add_file(root: &Path, relative: &Path, text: &str, total: &mut usize) -> Result<()> {
    *total = total.saturating_add(text.len());
    if text.len() > MAX_FILE || *total > MAX_TOTAL {
        bail!(
            "Export exceeds 1 MiB per configuration file or 16 MiB including the original export"
        );
    }
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap())?;
    write_new(&path, text.as_bytes())
}

fn snippet_name(name: &str) -> Result<()> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "/\\:*?\"<>|".contains(ch))
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
        || !matches!(
            Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("json" | "code-snippets")
        )
    {
        bail!("Exported snippet filename is not a portable direct JSON/snippet filename: {name:?}");
    }
    Ok(())
}

fn extension_requests(document: &Value) -> Result<Vec<Extension>> {
    let rows: Vec<Value> = document
        .get("extensions")
        .and_then(Value::as_str)
        .map(serde_json::from_str)
        .transpose()
        .context("Invalid exported extensions array")?
        .unwrap_or_default();
    if rows.len() > 2048 {
        bail!("Export exceeds 2048 extension requests");
    }
    let mut ids = BTreeSet::new();
    rows.iter()
        .map(|row| {
            let id = row
                .pointer("/identifier/id")
                .and_then(Value::as_str)
                .context("Exported extension has no identifier")?
                .to_ascii_lowercase();
            crate::extension_activation::validate_id(&id)?;
            if !ids.insert(id.clone()) {
                bail!("Duplicate exported extension identifier: {id}");
            }
            let version = row
                .get("version")
                .map(|value| {
                    let version = value
                        .as_str()
                        .context("Exported extension version must be a string")?;
                    semver::Version::parse(version)
                        .context("Invalid exported extension version")?;
                    Ok::<_, anyhow::Error>(version.to_owned())
                })
                .transpose()?;
            let flag = |key: &str| -> Result<bool> {
                row.get(key)
                    .map(|value| {
                        value
                            .as_bool()
                            .context(format!("Exported extension {key} must be a boolean"))
                    })
                    .transpose()
                    .map(|value| value.unwrap_or(false))
            };
            Ok(Extension {
                id,
                version,
                pre_release: flag("preRelease")?,
                disabled: flag("disabled")?,
                application_scoped: flag("applicationScoped")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
