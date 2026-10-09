//! Native user, workspace and installed-extension snippets, loaded off the input loop.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 128;
const MAX_ENTRIES: usize = 4096;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * MAX_FILE_BYTES;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub prefixes: Vec<String>,
    pub description: String,
    pub body: String,
    pub source: PathBuf,
    pub is_file_template: bool,
}

#[derive(Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub warnings: Vec<String>,
}

impl Catalog {
    pub fn load(user: Option<&Path>, workspace: &Path, language: &str) -> Self {
        Self::load_with_extensions(user, workspace, language, None)
    }

    pub fn load_with_extensions(
        user: Option<&Path>,
        workspace: &Path,
        language: &str,
        extensions: Option<&Path>,
    ) -> Self {
        let mut result = Self::default();
        let mut files = Vec::new();
        let mut language_files = HashSet::new();
        for (directory, is_user) in user
            .into_iter()
            .map(|p| (p.to_path_buf(), true))
            .chain(std::iter::once((workspace.join(".vscode"), false)))
        {
            let listing = match fs::read_dir(&directory) {
                Ok(listing) => listing,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    result
                        .warnings
                        .push(format!("{}: {error}", directory.display()));
                    continue;
                }
            };
            let mut batch = Vec::new();
            for (index, item) in listing.take(MAX_ENTRIES + 1).enumerate() {
                if index == MAX_ENTRIES {
                    result.warnings.push(format!(
                        "{}: directory exceeds 4096 entries; remaining entries skipped",
                        directory.display()
                    ));
                    break;
                }
                let Ok(item) = item else { continue };
                let path = item.path();
                let extension = path.extension().and_then(|v| v.to_str());
                if (extension == Some("code-snippets")
                    || (is_user
                        && extension == Some("json")
                        && path.file_stem().and_then(|v| v.to_str()) == Some(language)))
                    && item.file_type().is_ok_and(|kind| kind.is_file())
                {
                    batch.push(path);
                }
                if files.len() + batch.len() > MAX_FILES {
                    break;
                }
            }
            batch.sort();
            files.extend(batch);
        }
        if let Some(directory) = extensions {
            match crate::extension_store::Store::new(directory.into()).list() {
                Ok(packages) => {
                    result.extension_files(&packages, language, &mut files, &mut language_files)
                }
                Err(error) => result
                    .warnings
                    .push(format!("Installed snippets unavailable: {error:#}")),
            }
        }
        if files.len() > MAX_FILES {
            result
                .warnings
                .push("Snippet catalog exceeds 128 files; extra files skipped".into());
            files.truncate(MAX_FILES);
        }
        let mut total = 0;
        for path in files {
            if total >= MAX_TOTAL_BYTES {
                result.warnings.push(
                    "Snippet catalog reached its 16 MiB read budget; remaining files skipped"
                        .into(),
                );
                break;
            }
            match read_file(&path, &mut total).and_then(|text| {
                result.parse(&path, &text, language, !language_files.contains(&path))
            }) {
                Ok(()) => {}
                Err(error) => result
                    .warnings
                    .push(format!("{}: {error:#}", path.display())),
            }
        }
        result
    }

    fn extension_files(
        &mut self,
        packages: &[crate::extension_store::Installed],
        language: &str,
        files: &mut Vec<PathBuf>,
        language_files: &mut HashSet<PathBuf>,
    ) {
        let mut inspected = 0;
        for package in packages {
            let Some(contributions) = package.manifest.pointer("/contributes/snippets") else {
                continue;
            };
            let Some(contributions) = contributions.as_array() else {
                self.warnings.push(format!(
                    "{}: contributes.snippets must be an array",
                    package.id
                ));
                continue;
            };
            for contribution in contributions {
                if inspected == MAX_ENTRIES {
                    self.warnings.push("Installed snippet catalog exceeds 4096 contributions; remaining contributions skipped".into());
                    return;
                }
                inspected += 1;
                let result = (|| -> Result<Option<PathBuf>> {
                    let contributed_language = match contribution.get("language") {
                        None => None,
                        Some(value) => Some(
                            value
                                .as_str()
                                .context("Snippet language must be a string")?,
                        ),
                    }
                    .filter(|value| !value.trim().is_empty());
                    let relative = contribution
                        .get("path")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .context("Snippet contribution requires a relative path")?;
                    if contributed_language.is_none() && !relative.ends_with(".code-snippets") {
                        bail!(
                            "Snippet contribution without a language requires a .code-snippets file"
                        );
                    }
                    if contributed_language.is_some_and(|scope| {
                        language != scope
                            && !language
                                .strip_prefix(scope)
                                .is_some_and(|tail| tail.starts_with('.'))
                    }) {
                        return Ok(None);
                    }
                    let path = Path::new(relative);
                    if path.is_absolute()
                        || relative.contains(['\\', ':'])
                        || path.components().any(|component| {
                            !matches!(
                                component,
                                std::path::Component::Normal(_) | std::path::Component::CurDir
                            )
                        })
                    {
                        bail!("Snippet contribution path must stay inside its package");
                    }
                    let root = fs::canonicalize(&package.path)
                        .context("Cannot resolve snippet package")?;
                    let path = fs::canonicalize(root.join(path))
                        .context("Cannot resolve contributed snippet file")?;
                    if !path.starts_with(&root) || !fs::metadata(&path)?.is_file() {
                        bail!(
                            "Snippet contribution path escapes its package or is not a regular file"
                        );
                    }
                    if contributed_language.is_some() {
                        language_files.insert(path.clone());
                    }
                    Ok(Some(path))
                })();
                match result {
                    Ok(Some(path)) if !files.contains(&path) => {
                        files.push(path);
                        if files.len() > MAX_FILES {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => self.warnings.push(format!("{}: {error:#}", package.id)),
                }
            }
        }
    }

    fn parse(
        &mut self,
        path: &Path,
        text: &str,
        language: &str,
        filter_scopes: bool,
    ) -> Result<()> {
        let value: Value = crate::jsonc::parse(text).context("Invalid snippet JSONC")?;
        let object = value
            .as_object()
            .context("Snippet file must contain an object")?;
        for (name, value) in object {
            if value.get("body").is_some() {
                self.add(path, name, value, language, filter_scopes)?;
            } else if let Some(group) = value.as_object() {
                for (name, value) in group {
                    self.add(path, name, value, language, filter_scopes)?;
                }
            }
        }
        Ok(())
    }

    fn add(
        &mut self,
        path: &Path,
        name: &str,
        value: &Value,
        language: &str,
        filter_scopes: bool,
    ) -> Result<()> {
        if self.entries.len() >= MAX_ENTRIES {
            bail!("Snippet catalog exceeds 4096 entries");
        }
        if filter_scopes && path.extension().and_then(|v| v.to_str()) == Some("code-snippets") {
            let scopes: Vec<_> = value
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if !scopes.is_empty()
                && !scopes.iter().any(|scope| {
                    language == *scope
                        || language
                            .strip_prefix(scope)
                            .is_some_and(|tail| tail.starts_with('.'))
                })
            {
                return Ok(());
            }
        }
        let Some(body_value) = value.get("body") else {
            return Ok(());
        };
        let body = lines(body_value).with_context(|| {
            format!("Snippet {name:?} body must be a string or an array of strings")
        })?;
        if body.len() > 64 * 1024 {
            bail!("Snippet {name:?} exceeds 64 KiB");
        }
        let prefixes = match value.get("prefix") {
            Some(Value::String(prefix)) => vec![prefix.clone()],
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        self.entries.push(Entry {
            name: name.into(),
            prefixes,
            description: value.get("description").and_then(lines).unwrap_or_default(),
            body,
            source: path.into(),
            is_file_template: value
                .get("isFileTemplate")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
        Ok(())
    }
}

fn lines(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(values) => values
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|lines| lines.join("\n")),
        _ => None,
    }
}

fn read_file(path: &Path, total: &mut usize) -> Result<String> {
    let mut bytes = Vec::new();
    let remaining = MAX_TOTAL_BYTES.saturating_sub(*total);
    if remaining == 0 {
        bail!("Snippet catalog exhausted its 16 MiB read budget");
    }
    fs::File::open(path)?
        .take((MAX_FILE_BYTES + 1).min(remaining) as u64)
        .read_to_end(&mut bytes)?;
    *total += bytes.len();
    if bytes.len() > MAX_FILE_BYTES {
        bail!("Snippet file exceeds 1 MiB");
    }
    if bytes.len() == remaining {
        bail!("Snippet catalog exceeds 16 MiB");
    }
    String::from_utf8(bytes).context("Snippet file is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deeply_nested_file_is_reported_without_hiding_other_snippets() {
        let root = tempfile::tempdir().unwrap();
        let nested = format!("{}0{}", "[".repeat(50_000), "]".repeat(50_000));
        fs::write(root.path().join("a.code-snippets"), nested).unwrap();
        fs::write(
            root.path().join("b.code-snippets"),
            r#"{"Good":{"body":"works"}}"#,
        )
        .unwrap();
        let catalog = Catalog::load(Some(root.path()), root.path(), "rust");
        assert_eq!(catalog.warnings.len(), 1);
        assert!(catalog.warnings[0].contains("nesting"));
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].name, "Good");
    }

    #[test]
    fn language_global_workspace_scopes_groups_and_no_prefix() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("snippets");
        fs::create_dir(&user).unwrap();
        fs::create_dir(root.path().join(".vscode")).unwrap();
        fs::write(user.join("rust.json"), r#"{// language file overrides scope
          "First":{"scope":"python","prefix":["one","two"],"body":["${1:x}","$1$0"],"description":["a","b"],},
          "Group":{"No prefix":{"body":"literal"}}
        }"#).unwrap();
        fs::write(
            user.join("python.json"),
            r#"{"Excluded":{"body":"python"}}"#,
        )
        .unwrap();
        fs::write(user.join("global.code-snippets"), r#"{"Global":{"body":"all"},"Scoped":{"scope":" python, rust ","body":"rs"},"Excluded":{"scope":"python","body":"py"}}"#).unwrap();
        fs::write(
            root.path().join(".vscode/local.code-snippets"),
            r#"{"Workspace":{"body":"local","isFileTemplate":true}}"#,
        )
        .unwrap();
        let catalog = Catalog::load(Some(&user), root.path(), "rust");
        assert!(catalog.warnings.is_empty());
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|v| v.name.as_str())
                .collect::<Vec<_>>(),
            ["Global", "Scoped", "First", "No prefix", "Workspace"]
        );
        let first = &catalog.entries[2];
        assert_eq!(first.prefixes, ["one", "two"]);
        assert_eq!(first.body, "${1:x}\n$1$0");
        assert_eq!(first.description, "a\nb");
        assert!(catalog.entries[4].is_file_template);
    }
    #[test]
    fn read_budget_counts_invalid_utf8_and_stops_before_opening_more_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("invalid.code-snippets");
        fs::write(&path, [0xff; 8]).unwrap();
        let mut total = MAX_TOTAL_BYTES - 12;
        assert!(read_file(&path, &mut total).is_err());
        assert_eq!(total, MAX_TOTAL_BYTES - 4);
        assert!(read_file(&path, &mut total).is_err());
        assert_eq!(total, MAX_TOTAL_BYTES);
        fs::remove_file(&path).unwrap();
        assert!(
            read_file(&path, &mut total)
                .unwrap_err()
                .to_string()
                .contains("read budget")
        );
    }
    #[test]
    fn limits_report_truncation_and_keep_catalog_bounded() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_FILES + 1 {
            fs::write(root.path().join(format!("{index}.code-snippets")), "{}").unwrap();
        }
        let catalog = Catalog::load(Some(root.path()), root.path(), "plaintext");
        assert!(
            catalog
                .warnings
                .iter()
                .any(|warning| warning.contains("128 files"))
        );
        let directory = root.path().join("many");
        fs::create_dir(&directory).unwrap();
        for index in 0..MAX_ENTRIES + 1 {
            fs::write(directory.join(index.to_string()), "").unwrap();
        }
        let catalog = Catalog::load(Some(&directory), root.path(), "plaintext");
        assert!(
            catalog
                .warnings
                .iter()
                .any(|warning| warning.contains("4096 entries"))
        );
    }
    #[test]
    fn malformed_and_oversized_files_do_not_hide_valid_files() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.code-snippets"), "invalid {").unwrap();
        fs::write(
            root.path().join("b.code-snippets"),
            " ".repeat(MAX_FILE_BYTES + 1),
        )
        .unwrap();
        fs::write(
            root.path().join("c.code-snippets"),
            r#"{"Works":{"body":"yes"}}"#,
        )
        .unwrap();
        let catalog = Catalog::load(Some(root.path()), root.path(), "rust");
        assert_eq!(catalog.warnings.len(), 2);
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].name, "Works");
    }
}
