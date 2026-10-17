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
        let (application, notices) = super::application::discover(&product);
        for source in result.profiles.iter_mut().filter(|s| s.product == product) {
            source.application = application.clone();
        }
        result.notices.extend(notices);
    }
    Ok(result)
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
