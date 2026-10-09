//! Native user and workspace snippet catalogs, loaded away from the input loop.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
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
        let mut result = Self::default();
        let mut files = Vec::new();
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
            match read_file(&path, &mut total).and_then(|text| result.parse(&path, &text, language))
            {
                Ok(()) => {}
                Err(error) => result
                    .warnings
                    .push(format!("{}: {error:#}", path.display())),
            }
        }
        result
    }

    fn parse(&mut self, path: &Path, text: &str, language: &str) -> Result<()> {
        let value: Value = crate::jsonc::parse(text).context("Invalid snippet JSONC")?;
        let object = value
            .as_object()
            .context("Snippet file must contain an object")?;
        for (name, value) in object {
            if value.get("body").is_some() {
                self.add(path, name, value, language)?;
            } else if let Some(group) = value.as_object() {
                for (name, value) in group {
                    self.add(path, name, value, language)?;
                }
            }
        }
        Ok(())
    }

    fn add(&mut self, path: &Path, name: &str, value: &Value, language: &str) -> Result<()> {
        if self.entries.len() >= MAX_ENTRIES {
            bail!("Snippet catalog exceeds 4096 entries");
        }
        if path.extension().and_then(|v| v.to_str()) == Some("code-snippets") {
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
        let Some(body) = value.get("body").and_then(lines) else {
            return Ok(());
        };
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
