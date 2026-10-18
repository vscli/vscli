//! Local VS Code installations and registered profiles; discovery never changes them.
use super::*;
use serde_json::Map;

const MAX_PROFILES: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub product: String,
    pub user: PathBuf,
    pub default_user: PathBuf,
    pub extensions: PathBuf,
    #[serde(default)]
    pub use_default: Map<String, Value>,
    #[serde(default)]
    pub application: Option<super::application::Application>,
}
impl Source {
    pub fn resource_directory(&self, kind: &str) -> &Path {
        if self.use_default.get(kind).and_then(Value::as_bool) == Some(true) {
            &self.default_user
        } else {
            &self.user
        }
    }
    pub fn extension_index(&self) -> PathBuf {
        let user = self.resource_directory("extensions");
        if user == self.default_user {
            let current = self.extensions.join("extensions.json");
            if current.is_file() {
                return current;
            }
        }
        user.join("extensions.json")
    }
}

#[derive(Default, Serialize)]
pub struct Discovery {
    pub profiles: Vec<Source>,
    pub notices: Vec<String>,
}

pub fn discover() -> Result<Discovery> {
    let base = directories::BaseDirs::new()
        .context("No local user directories; pass a VS Code User directory explicitly")?;
    let mut installs = vec![
        (
            "VS Code".to_owned(),
            base.config_dir().join("Code/User"),
            base.home_dir().join(".vscode/extensions"),
        ),
        (
            "VS Code Insiders".to_owned(),
            base.config_dir().join("Code - Insiders/User"),
            base.home_dir().join(".vscode-insiders/extensions"),
        ),
        (
            "VSCodium".to_owned(),
            base.config_dir().join("VSCodium/User"),
            base.home_dir().join(".vscode-oss/extensions"),
        ),
        (
            "Code OSS".to_owned(),
            base.config_dir().join("Code - OSS/User"),
            base.home_dir().join(".vscode-oss/extensions"),
        ),
    ];
    if let Some(portable) = std::env::var_os("VSCODE_PORTABLE") {
        let root = PathBuf::from(portable);
        installs.push((
            "VS Code Portable".into(),
            root.join("user-data/User"),
            root.join("extensions"),
        ));
    }
    let mut result = discover_installations(&installs)?;
    let products: std::collections::BTreeSet<_> =
        result.profiles.iter().map(|s| s.product.clone()).collect();
    for product in products {
        let (application, notices) = application_for(&product, &result.profiles);
        for source in result.profiles.iter_mut().filter(|s| s.product == product) {
            source.application = application.clone();
        }
        result.notices.extend(notices);
    }
    Ok(result)
}

fn application_for(
    product: &str,
    sources: &[Source],
) -> (Option<super::application::Application>, Vec<String>) {
    if product != "VS Code Portable" {
        return super::application::discover(product);
    }
    let mut found = std::collections::BTreeMap::new();
    let mut inspected = std::collections::BTreeSet::new();
    for source in sources.iter().filter(|source| source.product == product) {
        if !inspected.insert(&source.default_user) {
            continue;
        }
        let mut candidates = vec![source.default_user.clone()];
        // macOS keeps the portable data directory beside the application bundle.
        if let Some(parent) = source.default_user.ancestors().nth(3) {
            let data = source
                .default_user
                .ancestors()
                .nth(2)
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let bundles: &[&str] = match data {
                Some("code-portable-data") => &["Visual Studio Code.app"],
                Some("code-insiders-portable-data") => &["Visual Studio Code - Insiders.app"],
                _ => &[
                    "Visual Studio Code.app",
                    "Visual Studio Code - Insiders.app",
                    "VSCodium.app",
                    "Code - OSS.app",
                ],
            };
            candidates.extend(bundles.iter().map(|bundle| parent.join(bundle)));
        }
        for path in candidates {
            if let Ok(application) = super::application::Application::open(&path)
                && matches!(
                    application.name.as_str(),
                    "code" | "code-insiders" | "codium" | "code-oss"
                )
            {
                found.insert(application.root.clone(), application);
            }
        }
    }
    match found.len() {
        1 => (found.into_values().next(), Vec::new()),
        0 => (None, vec!["VS Code Portable: source application not found beside its data; choose an application path to import built-ins".into()]),
        _ => (None, vec!["VS Code Portable: multiple adjacent applications found; choose an application path to import built-ins".into()]),
    }
}

pub fn discover_installations(installs: &[(String, PathBuf, PathBuf)]) -> Result<Discovery> {
    let mut result = Discovery::default();
    for (product, user, extensions) in installs {
        if !user.is_dir() {
            continue;
        }
        match installation(product, user, extensions) {
            Ok(mut sources) => result.profiles.append(&mut sources),
            Err(error) => {
                result.notices.push(format!(
                    "{product}: registered profiles could not be read: {error:#}"
                ));
                if let Ok(default) = default_source(product, user, extensions) {
                    result.profiles.push(default);
                }
            }
        }
    }
    Ok(result)
}

pub fn default_source(product: &str, user: &Path, extensions: &Path) -> Result<Source> {
    let user = fs::canonicalize(user).context("Cannot open VS Code User directory")?;
    Ok(Source {
        id: "default".into(),
        name: "Default".into(),
        product: product.into(),
        user: user.clone(),
        default_user: user.clone(),
        extensions: crate::document::absolute_path(extensions)?,
        use_default: Map::new(),
        application: None,
    })
}

pub fn installation(product: &str, user: &Path, extensions: &Path) -> Result<Vec<Source>> {
    let default = default_source(product, user, extensions)?;
    let user = default.user.clone();
    let mut sources = vec![default.clone()];
    let metadata = user.join("globalStorage/storage.json");
    if !metadata.exists() {
        return Ok(sources);
    }
    let value: Value = serde_json::from_slice(&read(&metadata, 4 * MAX_FILE)?)
        .context("Invalid VS Code profile metadata")?;
    let Some(profiles) = value.get("userDataProfiles").and_then(Value::as_array) else {
        return Ok(sources);
    };
    if profiles.len() > MAX_PROFILES {
        bail!("Profile metadata exceeds {MAX_PROFILES} registered profiles");
    }
    for profile in profiles {
        let name = profile
            .get("name")
            .and_then(Value::as_str)
            .context("Registered profile has no name")?;
        let id = profile
            .get("location")
            .and_then(Value::as_str)
            .context("Registered profile has no relative location")?;
        if id.is_empty()
            || id.len() > 256
            || name.len() > 1024
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            bail!("Invalid registered profile location or name");
        }
        let path = user.join("profiles").join(id);
        if !path.is_dir() {
            bail!("Registered profile directory is missing: {id}");
        }
        let path = fs::canonicalize(path)?;
        if !path.starts_with(user.join("profiles")) {
            bail!("Registered profile escapes the VS Code User directory");
        }
        let use_default = profile
            .get("useDefaultFlags")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        sources.push(Source {
            id: id.into(),
            name: name.into(),
            user: path,
            use_default,
            ..default.clone()
        });
    }
    Ok(sources)
}

pub fn select(profiles: &[Source], selector: Option<&str>) -> Result<Source> {
    let matches: Vec<_> = profiles
        .iter()
        .filter(|source| match selector {
            Some(selector) => {
                source.id == selector
                    || source.name == selector
                    || format!("{}:{}", source.product, source.id) == selector
            }
            None => source.id == "default",
        })
        .collect();
    match matches.as_slice() {
        [source] => Ok((*source).clone()),
        [] => bail!(
            "No matching local VS Code profile; use --list-vscode-profiles or pass a User directory"
        ),
        _ => bail!(
            "More than one VS Code profile matches; select product:id with --vscode-profile (see --list-vscode-profiles)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn portable_applications_follow_adjacent_installations_and_channel_specific_macos_bundles() {
        for (data, bundle, product) in [
            ("data", "resources/app", "code"),
            (
                "code-portable-data",
                "Visual Studio Code.app/Contents/Resources/app",
                "code",
            ),
            (
                "code-insiders-portable-data",
                "Visual Studio Code - Insiders.app/Contents/Resources/app",
                "code-insiders",
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let user = root.path().join(data).join("user-data/User");
            fs::create_dir_all(&user).unwrap();
            let app = root.path().join(bundle);
            fs::create_dir_all(app.join("extensions")).unwrap();
            fs::write(app.join("package.json"), r#"{"version":"1.118.0"}"#).unwrap();
            fs::write(
                app.join("product.json"),
                serde_json::to_vec(&json!({"applicationName":product})).unwrap(),
            )
            .unwrap();
            let sources = installation(
                "VS Code Portable",
                &user,
                &root.path().join(data).join("extensions"),
            )
            .unwrap();
            let (found, notices) = application_for("VS Code Portable", &sources);
            assert!(notices.is_empty(), "{notices:?}");
            let found = found.unwrap();
            assert_eq!(found.root, fs::canonicalize(app).unwrap());
            assert_eq!(found.version, "1.118.0");
            assert_eq!(found.name, product);
        }
    }
    #[test]
    fn portable_unknown_or_ambiguous_installations_require_an_explicit_application() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("custom-data/user-data/User");
        fs::create_dir_all(&user).unwrap();
        let sources =
            installation("VS Code Portable", &user, &root.path().join("extensions")).unwrap();
        let (found, notices) = application_for("VS Code Portable", &sources);
        assert!(found.is_none());
        assert!(notices[0].contains("not found"));
        for (bundle, product) in [
            ("Visual Studio Code.app", "code"),
            ("Visual Studio Code - Insiders.app", "code-insiders"),
        ] {
            let app = root.path().join(bundle).join("Contents/Resources/app");
            fs::create_dir_all(app.join("extensions")).unwrap();
            fs::write(app.join("package.json"), r#"{"version":"1.118.0"}"#).unwrap();
            fs::write(
                app.join("product.json"),
                serde_json::to_vec(&json!({"applicationName":product})).unwrap(),
            )
            .unwrap();
        }
        let (found, notices) = application_for("VS Code Portable", &sources);
        assert!(found.is_none());
        assert!(notices[0].contains("multiple"));
    }
    #[test]
    fn registered_profiles_keep_inheritance_and_do_not_discover_unregistered_folders() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("Code/User");
        fs::create_dir_all(user.join("globalStorage")).unwrap();
        fs::create_dir_all(user.join("profiles/named")).unwrap();
        fs::create_dir_all(user.join("profiles/unregistered")).unwrap();
        fs::write(user.join("globalStorage/storage.json"),serde_json::to_vec(&json!({"userDataProfiles":[{"name":"C++ λ","location":"named","useDefaultFlags":{"settings":true,"extensions":true}}]})).unwrap()).unwrap();
        let extensions = root.path().join("extensions");
        fs::create_dir_all(&extensions).unwrap();
        fs::write(extensions.join("extensions.json"), "[]").unwrap();
        let profiles = installation("VS Code", &user, &extensions).unwrap();
        assert_eq!(profiles.len(), 2);
        let selected = select(&profiles, Some("C++ λ")).unwrap();
        assert_eq!(
            selected.resource_directory("settings"),
            fs::canonicalize(&user).unwrap()
        );
        assert_eq!(
            selected.resource_directory("keybindings"),
            fs::canonicalize(user.join("profiles/named")).unwrap()
        );
        assert_eq!(
            selected.extension_index(),
            fs::canonicalize(extensions.join("extensions.json")).unwrap()
        );
        assert_eq!(select(&profiles, None).unwrap().id, "default");
    }
    #[test]
    fn ambiguous_sources_require_explicit_product_and_traversal_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("User");
        fs::create_dir_all(user.join("globalStorage")).unwrap();
        let mut profiles = installation("Stable", &user, root.path()).unwrap();
        profiles.extend(installation("Insiders", &user, root.path()).unwrap());
        assert!(select(&profiles, None).is_err());
        assert_eq!(
            select(&profiles, Some("Insiders:default")).unwrap().product,
            "Insiders"
        );
        fs::write(
            user.join("globalStorage/storage.json"),
            r#"{"userDataProfiles":[{"name":"Escape","location":"../../outside"}]}"#,
        )
        .unwrap();
        assert!(installation("Stable", &user, root.path()).is_err());
    }
}
