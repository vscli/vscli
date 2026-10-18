//! Profile-indexed local packages and enablement. No package code is executed.
use super::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Map;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_PACKAGES: usize = 2048;
const MAX_DATABASE: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    pub version: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub manifest_sha256: String,
    pub code: bool,
    #[serde(default)]
    pub runtime_blockers: Vec<String>,
    #[serde(default)]
    pub built_in: bool,
    pub dependencies: Vec<String>,
    pub contributions: Vec<String>,
    pub target_platform: Option<String>,
    pub metadata: Map<String, Value>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub index: PathBuf,
    pub registered_versions: usize,
    #[serde(default)]
    pub built_in_packages: usize,
    pub selected: Vec<Package>,
    pub notices: Vec<String>,
}

pub fn inventory(source: &discovery::Source) -> Result<Inventory> {
    inventory_selected(source, None)
}

pub(super) fn inventory_selected(
    source: &discovery::Source,
    requests: Option<&[super::export::Extension]>,
) -> Result<Inventory> {
    let index = source.extension_index();
    let rows: Vec<Value> = if index.exists() {
        serde_json::from_slice(&read(&index, 8 * MAX_FILE)?)
            .context("Invalid VS Code extension profile index")?
    } else {
        Vec::new()
    };
    if rows.len() > MAX_PACKAGES {
        bail!("Extension profile exceeds {MAX_PACKAGES} registered versions");
    }
    let disabled = disabled(source)?;
    let mut selected: BTreeMap<String, Package> = BTreeMap::new();
    let mut result = Inventory {
        index,
        registered_versions: rows.len(),
        ..Inventory::default()
    };
    if !result.index.exists() {
        result.notices.push("No registered user extension index was found; unregistered user folders are not silently selected".into());
    }
    let mut metadata_bytes = 0usize;
    let mut metadata_nodes = 0usize;
    for row in rows {
        let id = row
            .pointer("/identifier/id")
            .and_then(Value::as_str)
            .context("Extension index entry has no identifier")?
            .to_ascii_lowercase();
        crate::extension_activation::validate_id(&id)?;
        let version = row
            .get("version")
            .and_then(Value::as_str)
            .context("Extension index entry has no version")?;
        let parsed =
            semver::Version::parse(version).context("Invalid registered extension version")?;
        if let Some(requests) = requests
            && !requests
                .iter()
                .any(|request| request.matches(&id, version, row.get("metadata"), false))
        {
            continue;
        }
        let path = location(&row, &source.extensions)?;
        let manifest_bytes = read(&path.join("package.json"), MAX_FILE)?;
        let manifest: Value = serde_json::from_slice(&manifest_bytes)
            .context("Invalid registered package manifest")?;
        let manifest_id = format!(
            "{}.{}",
            manifest["publisher"]
                .as_str()
                .context("Package has no publisher")?,
            manifest["name"].as_str().context("Package has no name")?
        )
        .to_ascii_lowercase();
        if manifest_id != id || manifest["version"].as_str() != Some(version) {
            bail!("Registered extension identity/version differs from its package: {id}");
        }
        let metadata = row
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let target_platform = metadata
            .get("targetPlatform")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !platform_matches(target_platform.as_deref()) {
            result.notices.push(format!("{id}@{version}: package platform {:?} differs from this editor; native binaries remain unqualified",target_platform));
        }
        metadata_budget(
            &manifest_bytes,
            &manifest,
            &mut metadata_bytes,
            &mut metadata_nodes,
        )?;
        let package = package(
            path,
            &manifest_bytes,
            &manifest,
            id.clone(),
            version.into(),
            metadata,
            &disabled,
            false,
        )?;
        let replace = match selected.get(&id) {
            None => true,
            Some(old) => {
                let old_platform = platform_matches(old.target_platform.as_deref());
                let new_platform = platform_matches(package.target_platform.as_deref());
                (new_platform && !old_platform)
                    || (new_platform == old_platform
                        && parsed > semver::Version::parse(&old.version)?)
            }
        };
        if replace {
            selected.insert(id, package);
        }
    }
    if requests.is_none() && result.registered_versions > selected.len() {
        result.notices.push(format!("{} registered versions resolve to {} IDs; the highest registered version for this platform is selected and other variants remain in the original index",result.registered_versions,selected.len()));
    }
    if let Some(application) = &source.application {
        let root = fs::canonicalize(application.extensions())?;
        let mut entries = fs::read_dir(&root)?
            .take(MAX_PACKAGES + 1)
            .collect::<std::io::Result<Vec<_>>>()?;
        if entries.len() > MAX_PACKAGES {
            bail!("Built-in package directory exceeds {MAX_PACKAGES} entries");
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = fs::canonicalize(entry.path())?;
            if !path.starts_with(&root) {
                bail!("Built-in package escapes source application");
            }
            let manifest_path = path.join("package.json");
            if !manifest_path.is_file() {
                continue;
            }
            let bytes = read(&manifest_path, MAX_FILE)?;
            let manifest: Value =
                serde_json::from_slice(&bytes).context("Invalid built-in package manifest")?;
            metadata_budget(&bytes, &manifest, &mut metadata_bytes, &mut metadata_nodes)?;
            let id = format!(
                "{}.{}",
                manifest["publisher"]
                    .as_str()
                    .context("Built-in package has no publisher")?,
                manifest["name"]
                    .as_str()
                    .context("Built-in package has no name")?
            )
            .to_ascii_lowercase();
            crate::extension_activation::validate_id(&id)?;
            let version = manifest["version"]
                .as_str()
                .context("Built-in package has no version")?;
            let parsed = semver::Version::parse(version)?;
            result.built_in_packages += 1;
            if let Some(request) =
                requests.and_then(|requests| requests.iter().find(|request| request.id == id))
                && !request.matches(&id, version, None, true)
            {
                continue;
            }
            let package = package(
                path,
                &bytes,
                &manifest,
                id.clone(),
                version.into(),
                Map::new(),
                &disabled,
                true,
            )?;
            // Pinned upstream scanner prefers a newer version and the built-in
            // on an equal version. Source engine validity remains unqualified.
            if selected.get(&id).is_none_or(|old| {
                semver::Version::parse(&old.version).is_ok_and(|old| parsed >= old)
            }) {
                selected.insert(id, package);
            }
        }
        if selected.len() > MAX_PACKAGES {
            bail!("Combined source exceeds {MAX_PACKAGES} distinct packages");
        }
        result.notices.push(format!("Source application {} supplies {} built-in packages; engine/native backend compatibility remains unqualified", application.version, result.built_in_packages));
    } else {
        result.notices.push(
            "Source application was not selected; built-in packages are absent from this preview"
                .into(),
        );
    }
    for package in selected.values() {
        if package.code && !package.runtime_blockers.is_empty() {
            result.notices.push(format!(
                "{}: code activation blocked: {}. Package copying remains available; native contribution support is separate.",
                package.id,
                package.runtime_blockers.join("; ")
            ));
        }
        for dependency in &package.dependencies {
            if !selected.contains_key(dependency) {
                result.notices.push(format!("{} requires {dependency}, which is absent from this profile; activation remains unqualified",package.id));
            } else if !selected[dependency].enabled && package.enabled {
                result.notices.push(format!("{} has disabled dependency {dependency}; migration retains that disabled state",package.id));
            }
        }
    }
    result.selected = selected.into_values().collect();
    Ok(result)
}

fn metadata_budget(
    bytes: &[u8],
    manifest: &Value,
    total: &mut usize,
    nodes: &mut usize,
) -> Result<()> {
    *total = total.saturating_add(bytes.len());
    let mut remaining = vec![manifest];
    while let Some(value) = remaining.pop() {
        *nodes = nodes.saturating_add(1);
        if *total > 8 * MAX_FILE || *nodes > 250_000 {
            bail!("Source extension manifests exceed 8 MiB / 250,000 JSON-node metadata budget");
        }
        match value {
            Value::Object(value) => remaining.extend(value.values()),
            Value::Array(value) => remaining.extend(value),
            _ => {}
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn package(
    path: PathBuf,
    manifest_bytes: &[u8],
    manifest: &Value,
    id: String,
    version: String,
    metadata: Map<String, Value>,
    disabled: &BTreeSet<String>,
    built_in: bool,
) -> Result<Package> {
    let target_platform = metadata
        .get("targetPlatform")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let dependencies = manifest
        .get("extensionDependencies")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    let id = value
                        .as_str()
                        .context("Invalid extension dependency")?
                        .to_ascii_lowercase();
                    crate::extension_activation::validate_id(&id)?;
                    Ok(id)
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Package {
        id: id.clone(),
        version,
        path,
        enabled: !disabled.contains(&id),
        manifest_sha256: format!("{:x}", Sha256::digest(manifest_bytes)),
        code: manifest.get("main").is_some() || manifest.get("browser").is_some(),
        runtime_blockers: if manifest.get("enabledApiProposals").is_some_and(|value| {
            value.as_array().is_some_and(|values| !values.is_empty())
                || value.as_str().is_some_and(|value| !value.is_empty())
        }) {
            vec!["Requested proposed APIs are rejected by the current compatibility host".into()]
        } else {
            Vec::new()
        },
        built_in,
        dependencies,
        contributions: manifest
            .get("contributes")
            .and_then(Value::as_object)
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default(),
        target_platform,
        metadata,
    })
}

pub(super) fn platform_matches(platform: Option<&str>) -> bool {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "arm") => "linux-armhf",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        ("windows", "x86_64") => "win32-x64",
        ("windows", "aarch64") => "win32-arm64",
        _ => "unknown",
    };
    platform.is_none_or(|platform| {
        matches!(platform, "undefined" | "universal" | "web") || platform == target
    })
}
fn location(row: &Value, extensions: &Path) -> Result<PathBuf> {
    let path = if let Some(relative) = row
        .get("relativeLocation")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        if Path::new(relative)
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!("Registered package has an unsafe relative location");
        }
        extensions.join(relative)
    } else {
        let location = row
            .get("location")
            .context("Registered package has no location")?;
        if location.get("scheme").and_then(Value::as_str) != Some("file") {
            bail!("Registered package must use a local file URI");
        }
        let path = location
            .get("path")
            .and_then(Value::as_str)
            .context("Registered package has no URI path")?;
        let authority = location
            .get("authority")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !authority.is_empty() {
            bail!("Registered package has a remote file authority");
        }
        #[cfg(windows)]
        let path = path
            .strip_prefix('/')
            .filter(|path| path.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(path);
        PathBuf::from(path)
    };
    let path = fs::canonicalize(path).context("Registered extension directory is missing")?;
    if !path.is_dir() {
        bail!("Registered extension is not a directory");
    }
    Ok(path)
}

fn disabled(source: &discovery::Source) -> Result<BTreeSet<String>> {
    let database = source
        .resource_directory("globalState")
        .join("globalStorage/state.vscdb");
    if !database.exists() {
        return Ok(BTreeSet::new());
    }
    // Opening the original SQLite database can create WAL/SHM files. Recover a
    // bounded private snapshot instead, and query only extension enablement.
    let temporary = tempfile::tempdir()?;
    let snapshot = temporary.path().join("state.vscdb");
    let paths = [database.clone(), database.with_file_name("state.vscdb-wal")];
    let before: Vec<_> = paths
        .iter()
        .map(|path| fingerprint(path))
        .collect::<Result<_>>()?;
    for (path, stamp) in paths.iter().zip(&before) {
        if stamp.is_none() {
            continue;
        }
        let mut input = fs::File::open(path)?;
        let mut output = fs::File::create(temporary.path().join(path.file_name().unwrap()))?;
        if std::io::copy(&mut (&mut input).take(MAX_DATABASE + 1), &mut output)? > MAX_DATABASE {
            bail!("VS Code state database snapshot exceeds 256 MiB");
        }
    }
    let after: Vec<_> = paths
        .iter()
        .map(|path| fingerprint(path))
        .collect::<Result<_>>()?;
    if before != after {
        bail!("VS Code state changed while capturing enablement; retry the import");
    }
    let connection = Connection::open_with_flags(
        snapshot,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let row: Option<(i64,String)>=connection.query_row(
        "SELECT length(value), CAST(substr(value,1,1048577) AS TEXT) FROM ItemTable WHERE key = ?1",
        ["extensionsIdentifiers/disabled"],|row| Ok((row.get(0)?,row.get(1)?))
    ).optional()?;
    let Some((length, text)) = row else {
        return Ok(BTreeSet::new());
    };
    if length > MAX_FILE as i64 || text.len() > MAX_FILE {
        bail!("Extension disablement exceeds 1 MiB");
    }
    let values: Vec<Value> =
        serde_json::from_str(&text).context("Invalid VS Code extension disablement state")?;
    if values.len() > MAX_PACKAGES {
        bail!("Extension disablement exceeds {MAX_PACKAGES} entries");
    }
    values
        .iter()
        .map(|value| {
            let id = value
                .as_str()
                .or_else(|| value.get("id").and_then(Value::as_str))
                .context("Invalid disabled extension identifier")?
                .to_ascii_lowercase();
            crate::extension_activation::validate_id(&id)?;
            Ok(id)
        })
        .collect()
}
fn fingerprint(path: &Path) -> Result<Option<(u64, std::time::SystemTime)>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            if !metadata.is_file() {
                bail!("VS Code state database must be a regular file");
            }
            Ok(Some((metadata.len(), metadata.modified()?)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(root: &Path) -> discovery::Source {
        let user = root.join("User");
        fs::create_dir_all(user.join("globalStorage")).unwrap();
        fs::create_dir_all(root.join("extensions")).unwrap();
        discovery::installation("Fixture", &user, &root.join("extensions"))
            .unwrap()
            .remove(0)
    }
    fn package(source: &discovery::Source, version: &str) -> Value {
        let relative = format!("fixture.native-{version}");
        let root = source.extensions.join(&relative);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("package.json"),serde_json::to_vec(&json!({"publisher":"fixture","name":"native","version":version,"main":"main.cjs","contributes":{"commands":[]}})).unwrap()).unwrap();
        json!({"identifier":{"id":"Fixture.Native"},"version":version,"relativeLocation":relative,"location":{"scheme":"file","path":root},"metadata":{"targetPlatform":"universal"}})
    }
    #[test]
    fn registered_variants_resolve_latest_and_disabled_state_is_captured_without_source_writes() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path());
        let rows = vec![package(&source, "1.0.0"), package(&source, "2.0.0")];
        fs::write(source.extension_index(), serde_json::to_vec(&rows).unwrap()).unwrap();
        let db = source.user.join("globalStorage/state.vscdb");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY,value BLOB)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable VALUES (?1,?2)",
                [
                    "extensionsIdentifiers/disabled",
                    r#"[{"id":"FIXTURE.NATIVE"}]"#,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable VALUES (?1,?2)",
                ["unrelated.secret", "must never be selected"],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&db).unwrap();
        let filenames: Vec<_> = fs::read_dir(db.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let inventory = inventory(&source).unwrap();
        assert_eq!(inventory.registered_versions, 2);
        assert_eq!(inventory.selected.len(), 1);
        assert_eq!(inventory.selected[0].version, "2.0.0");
        assert!(!inventory.selected[0].enabled);
        assert_eq!(fs::read(&db).unwrap(), before);
        let after: Vec<_> = fs::read_dir(db.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(filenames, after);
    }
    #[test]
    fn active_wal_disablements_are_read_from_private_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path());
        let db = source.user.join("globalStorage/state.vscdb");
        let connection = Connection::open(&db).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .execute(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY,value BLOB)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable VALUES (?1,?2)",
                [
                    "extensionsIdentifiers/disabled",
                    r#"[{"id":"fixture.native"}]"#,
                ],
            )
            .unwrap();
        assert!(db.with_file_name("state.vscdb-wal").exists());
        assert!(disabled(&source).unwrap().contains("fixture.native"));
        drop(connection);
    }
    #[test]
    fn tampered_manifest_or_corrupt_disablements_fail_instead_of_enabling_everything() {
        let root = tempfile::tempdir().unwrap();
        let source = fixture(root.path());
        let row = package(&source, "1.0.0");
        fs::write(
            source.extension_index(),
            serde_json::to_vec(&vec![row]).unwrap(),
        )
        .unwrap();
        fs::write(
            source.user.join("globalStorage/state.vscdb"),
            "broken database",
        )
        .unwrap();
        assert!(inventory(&source).is_err());
        fs::remove_file(source.user.join("globalStorage/state.vscdb")).unwrap();
        fs::write(
            source.extensions.join("fixture.native-1.0.0/package.json"),
            r#"{"publisher":"other","name":"native","version":"1.0.0"}"#,
        )
        .unwrap();
        assert!(inventory(&source).is_err());
    }
}
