//! A bounded worker-only workspaceContains subset; no filesystem work in input/rendering.
use super::{Event, Metadata};
use globset::{GlobBuilder, GlobSetBuilder};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    time::{Duration, Instant},
};
const MAX_PATTERNS: usize = 128;
const MAX_ENTRIES: usize = 10_000;
const MAX_PATH_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 16;
#[derive(Default)]
pub struct Matches {
    pub owners: BTreeSet<String>,
    pub notices: Vec<String>,
}
impl Matches {
    fn notice(&mut self, text: String) {
        if self.notices.len() < 128 {
            self.notices.push(text.chars().take(128).collect());
        }
    }
}
pub fn scan(
    root: &Path,
    catalog: &BTreeMap<String, Metadata>,
    enabled: &BTreeSet<String>,
) -> Matches {
    let mut result = Matches::default();
    let mut builder = GlobSetBuilder::new();
    let mut owners = Vec::new();
    let mut checked = 0;
    for (id, item) in catalog {
        if !enabled.contains(id) {
            continue;
        }
        for event in &item.events {
            let Event::WorkspaceContains(pattern) = event else {
                continue;
            };
            if checked >= MAX_PATTERNS {
                result.notice(
                    "workspaceContains exceeds 128 patterns; excess patterns were not checked"
                        .into(),
                );
                break;
            }
            checked += 1;
            // Advanced glob parsing/expansion and parent traversal are deliberately excluded.
            if pattern.starts_with('/')
                || pattern.split('/').any(|part| part == "..")
                || pattern.contains(['\\', '[', ']', '{', '}', ':'])
            {
                result.notice(format!(
                    "{id}: unsupported workspaceContains pattern: {pattern}"
                ));
                continue;
            }
            match GlobBuilder::new(pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
            {
                Ok(glob) => {
                    builder.add(glob);
                    owners.push(id.clone());
                }
                Err(error) => result
                    .notices
                    .push(format!("{id}: invalid workspaceContains pattern: {error}")),
            }
        }
    }
    if owners.is_empty() {
        return result;
    }
    let Ok(matcher) = builder.build() else {
        result
            .notices
            .push("workspaceContains glob compilation limit reached".into());
        return result;
    };
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut queue = VecDeque::from([(root.to_owned(), 0)]);
    let (mut entries, mut path_bytes) = (0, 0);
    let mut limited = false;
    'walk: while let Some((directory, depth)) = queue.pop_front() {
        let listing = match std::fs::read_dir(directory) {
            Ok(listing) => listing,
            Err(error) => {
                result.notice(format!(
                    "workspaceContains directory could not be read: {error}"
                ));
                continue;
            }
        };
        for entry in listing {
            if entries >= MAX_ENTRIES || Instant::now() >= deadline {
                limited = true;
                break 'walk;
            }
            entries += 1;
            let Ok(entry) = entry else {
                continue;
            };
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let relative = relative
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            path_bytes += relative.len();
            if relative.len() > 4096 || path_bytes > MAX_PATH_BYTES {
                limited = true;
                break 'walk;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    queue.push_back((path, depth + 1));
                } else {
                    limited = true;
                }
            } else if kind.is_file() {
                for index in matcher.matches(&relative) {
                    result.owners.insert(owners[index].clone());
                }
            }
        }
    }
    if limited {
        result.notice("workspaceContains stopped at its 10,000-entry / depth-16 / 4 MiB path / 500 ms budget; unmatched patterns remain unqualified".into());
    }

    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extension_activation::Metadata, extension_store::Installed};
    use serde_json::json;
    fn metadata(events: &[&str]) -> Metadata {
        Metadata::installed(&Installed {
            id: "test.scan".into(),
            version: "1".into(),
            path: "/snapshot".into(),
            source: "fixture".into(),
            sha256: "hash".into(),
            compatibility: String::new(),
            manifest: json!({"activationEvents":events}),
        })
        .unwrap()
    }
    #[test]
    fn root_and_nested_globs_match_regular_files_only_when_granted() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("package.json"), "{}").unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/code.rs"), "").unwrap();
        let catalog = BTreeMap::from([(
            "test.scan".into(),
            metadata(&[
                "workspaceContains:**/*.rs",
                "workspaceContains:package.json",
            ]),
        )]);
        assert!(
            scan(root.path(), &catalog, &BTreeSet::new())
                .owners
                .is_empty()
        );
        let granted = BTreeSet::from(["test.scan".into()]);
        assert_eq!(scan(root.path(), &catalog, &granted).owners, granted);
        let catalog = BTreeMap::from([(
            "test.scan".into(),
            metadata(&["workspaceContains:../secret", "workspaceContains:{a,b}"]),
        )]);
        let matches = scan(root.path(), &catalog, &granted);
        assert!(matches.owners.is_empty());
        assert_eq!(matches.notices.len(), 2);
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_directories_and_fifo_names_do_not_activate_packages() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escaped")).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.path().join("fake.rs"))
                .status()
                .unwrap()
                .success()
        );
        let catalog =
            BTreeMap::from([("test.scan".into(), metadata(&["workspaceContains:**/*.rs"]))]);
        assert!(
            scan(root.path(), &catalog, &BTreeSet::from(["test.scan".into()]))
                .owners
                .is_empty()
        );
    }
}
