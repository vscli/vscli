//! Read-only discovery of the source application's built-in extension directory.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Application {
    pub root: PathBuf,
    pub version: String,
    pub name: String,
}
impl Application {
    /// Accept an application resources/app root, installation root or executable.
    /// Inspection never launches the source application.
    pub fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path).context("Cannot find VS Code application")?;
        let base = if path.is_file() {
            path.parent().unwrap()
        } else {
            &path
        };
        for ancestor in base.ancestors().take(5) {
            for root in [
                ancestor.to_path_buf(),
                ancestor.join("resources/app"),
                ancestor.join("Contents/Resources/app"),
            ] {
                if root.join("package.json").is_file()
                    && root.join("product.json").is_file()
                    && root.join("extensions").is_dir()
                {
                    let root = fs::canonicalize(root)?;
                    let manifest: Value =
                        serde_json::from_slice(&read(&root.join("package.json"), MAX_FILE)?)?;
                    let product: Value =
                        serde_json::from_slice(&read(&root.join("product.json"), MAX_FILE)?)?;
                    let version = manifest["version"]
                        .as_str()
                        .context("Source application has no version")?;
                    semver::Version::parse(version)
                        .context("Invalid source application version")?;
                    let name = product["applicationName"]
                        .as_str()
                        .context("Source application has no product identity")?;
                    if name.len() > 256 {
                        bail!("Source application identity exceeds bounds");
                    }
                    return Ok(Self {
                        root,
                        version: version.into(),
                        name: name.into(),
                    });
                }
            }
        }
        bail!(
            "No VS Code resources/app with product metadata and built-in extensions at {}",
            path.display()
        )
    }
    pub fn extensions(&self) -> PathBuf {
        self.root.join("extensions")
    }
}

pub fn discover(product: &str) -> (Option<Application>, Vec<String>) {
    let (executable, folder) = match product {
        "VS Code" => ("code", "Microsoft VS Code"),
        "VS Code Insiders" => ("code-insiders", "Microsoft VS Code Insiders"),
        "VSCodium" => ("codium", "VSCodium"),
        "Code OSS" => ("code-oss", "Code - OSS"),
        _ => return (None, Vec::new()),
    };
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path).take(256) {
            candidates.push(directory.join(executable));
            #[cfg(windows)]
            {
                candidates.push(directory.join(format!("{executable}.cmd")));
                candidates.push(directory.join(format!("{executable}.exe")));
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from(format!("/usr/share/{executable}")));
        candidates.push(PathBuf::from(format!("/usr/lib/{executable}")));
        if executable == "code" {
            candidates.push("/opt/visual-studio-code".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = match executable {
            "code" => "Visual Studio Code.app",
            "code-insiders" => "Visual Studio Code - Insiders.app",
            "codium" => "VSCodium.app",
            _ => "Code - OSS.app",
        };
        candidates.push(PathBuf::from("/Applications").join(bundle));
        if let Some(base) = directories::BaseDirs::new() {
            candidates.push(base.home_dir().join("Applications").join(bundle));
        }
    }
    #[cfg(windows)]
    {
        for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                candidates.push(root.join(folder));
                candidates.push(root.join("Programs").join(folder));
            }
        }
    }
    let _ = folder;
    let mut seen = BTreeSet::new();
    let mut applications = Vec::new();
    for path in candidates {
        if path.exists()
            && let Ok(application) = Application::open(&path)
            && application.name == executable
            && seen.insert(application.root.clone())
        {
            applications.push(application);
        }
    }
    match applications.len() {
        0 => (
            None,
            vec![format!(
                "{product}: source application not found; built-in packages need an application path"
            )],
        ),
        1 => (applications.pop(), Vec::new()),
        _ => (
            None,
            vec![format!(
                "{product}: multiple source applications found; choose an application path to import built-ins"
            )],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_installation_and_executable_resolve_without_running_source_code() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("resources/app");
        fs::create_dir_all(app.join("extensions")).unwrap();
        fs::create_dir_all(root.path().join("bin")).unwrap();
        fs::write(root.path().join("bin/code"), "source code must not execute").unwrap();
        fs::write(app.join("package.json"), r#"{"version":"1.118.0"}"#).unwrap();
        fs::write(app.join("product.json"), r#"{"applicationName":"code"}"#).unwrap();
        let canonical = fs::canonicalize(&app).unwrap();
        for path in [&app, root.path(), &root.path().join("bin/code")] {
            let found = Application::open(path).unwrap();
            assert_eq!(found.root, canonical);
            assert_eq!(found.version, "1.118.0");
            assert_eq!(found.extensions(), canonical.join("extensions"));
        }
        fs::write(app.join("package.json"), r#"{"version":"invalid"}"#).unwrap();
        assert!(Application::open(&app).is_err());
        assert!(Application::open(&root.path().join("absent")).is_err());
    }
}
