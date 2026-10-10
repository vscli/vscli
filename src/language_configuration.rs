//! Bounded installed language-configuration data. Loading never activates code.
//! Callers load this catalog on a worker, never in an input or rendering path.
use crate::{extension_activation::Preferences, extension_store::Installed};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const PACKAGES: usize = 128;
const REFERENCES: usize = 256;
const FILE_BYTES: usize = 64 * 1024;
const READ_BYTES: usize = 4 * 1024 * 1024;
const PAIRS: usize = 64;
const WARNINGS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub owner: String,
    pub version: String,
    pub package_path: PathBuf,
    pub archive_sha256: String,
    pub configuration_path: PathBuf,
    pub content_sha256: String,
    pub composition_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub open: char,
    pub close: char,
    pub not_string: bool,
    pub not_comment: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delimiter {
    pub open: char,
    pub close: char,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comments {
    pub line_comment: Option<String>,
    pub block_comment: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    pub identity: Identity,
    pub language: String,
    pub auto_closing_pairs: Option<Vec<Pair>>,
    pub surrounding_pairs: Option<Vec<Delimiter>>,
    pub auto_close_before: Option<String>,
    pub brackets: Option<Vec<Delimiter>>,
    pub comments: Option<Comments>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub profiles: BTreeMap<String, Arc<Configuration>>,
    pub warnings: Vec<String>,
    loaded_packages: BTreeMap<String, LoadedPackage>,
}

#[derive(Clone, Debug)]
struct LoadedPackage {
    version: String,
    path: PathBuf,
    archive_sha256: String,
}

impl Catalog {
    /// Read-only source proof; it performs no filesystem work or allocation.
    pub fn has_installed_configuration(&self, installed: &Installed) -> bool {
        self.loaded_packages
            .get(&installed.id)
            .is_some_and(|source| {
                source.version == installed.version
                    && source.path == installed.path
                    && source.archive_sha256 == installed.sha256
            })
    }
    pub fn for_language(&self, language: &str) -> Option<Arc<Configuration>> {
        self.profiles.get(language).map(Arc::clone)
    }

    fn warn(&mut self, message: impl Into<String>) {
        if self.warnings.len() < WARNINGS - 1 {
            let mut message = message.into();
            if message.len() > 4096 {
                let mut end = 4096;
                while !message.is_char_boundary(end) {
                    end -= 1
                }
                message.truncate(end);
                message.push('…');
            }
            self.warnings.push(message);
        } else if self.warnings.len() == WARNINGS - 1 {
            self.warnings.push(
                "Language configuration warnings exceed 511 entries; further warnings omitted"
                    .into(),
            );
        }
    }

    pub fn load(installed: &[Installed], global: &Preferences, workspace: &Preferences) -> Self {
        let mut catalog = Self::default();
        let mut packages = installed.iter().take(PACKAGES).collect::<Vec<_>>();
        packages.sort_by(|a, b| a.id.cmp(&b.id));
        if installed.len() > PACKAGES {
            catalog.warn("Language configuration catalog exceeds 128 installed packages; remaining packages omitted");
        }
        let mut references = 0usize;
        let mut remaining = READ_BYTES;
        for package in packages {
            if !workspace
                .extensions
                .get(&package.id)
                .or_else(|| global.extensions.get(&package.id))
                .copied()
                .unwrap_or(true)
            {
                continue;
            }
            if let Err(error) = validate_package(package) {
                catalog.warn(format!("Language configuration {}: {error:#}", package.id));
                continue;
            }
            let Some(contributions) = package.manifest.pointer("/contributes/languages") else {
                continue;
            };
            let Some(contributions) = contributions.as_array() else {
                catalog.warn(format!(
                    "{}: contributes.languages must be an array",
                    package.id
                ));
                continue;
            };
            for contribution in contributions {
                if references == REFERENCES {
                    catalog.warn("Language configuration catalog exceeds 256 language contributions; remaining contributions omitted");
                    return catalog;
                }
                references += 1;
                let Some(relative) = contribution.get("configuration") else {
                    continue;
                };
                let result = (|| -> Result<Configuration> {
                    let language = contribution
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|value| bounded_string(value, 128, false))
                        .context(
                            "Language contribution requires a nonempty id of at most 128 bytes",
                        )?;
                    let relative = relative
                        .as_str()
                        .context("Language configuration path must be a string")?;
                    let (root, path, bytes) =
                        read_configuration(&package.path, relative, &mut remaining)?;
                    let text = std::str::from_utf8(&bytes)
                        .context("Language configuration must be UTF-8")?;
                    let value: Value = crate::jsonc::parse(text)
                        .context("Invalid language configuration JSONC")?;
                    let object = value
                        .as_object()
                        .context("Language configuration must be an object")?;
                    let identity = Identity {
                        owner: package.id.clone(),
                        version: package.version.clone(),
                        package_path: root,
                        archive_sha256: package.sha256.clone(),
                        configuration_path: path,
                        content_sha256: format!("{:x}", Sha256::digest(&bytes)),
                        composition_sha256: String::new(),
                    };
                    Ok(catalog.parse(language, identity, object))
                })();
                match result {
                    Ok(mut configuration) => {
                        catalog.loaded_packages.insert(
                            package.id.clone(),
                            LoadedPackage {
                                version: package.version.clone(),
                                path: package.path.clone(),
                                archive_sha256: package.sha256.clone(),
                            },
                        );
                        let previous = catalog.profiles.get(&configuration.language).cloned();
                        if let Some(previous) = &previous {
                            catalog.warn(format!("{}: language {:?} configurations compose fieldwise; package-ID order then manifest order applies", package.id, configuration.language));
                            inherit(&mut configuration, previous);
                        }
                        configuration.identity.composition_sha256 =
                            composition(&configuration, previous.as_deref());
                        catalog
                            .profiles
                            .insert(configuration.language.clone(), Arc::new(configuration));
                    }
                    Err(error) => {
                        catalog.warn(format!("Language configuration {}: {error:#}", package.id))
                    }
                }
            }
        }
        catalog
    }

    fn parse(
        &mut self,
        language: &str,
        identity: Identity,
        object: &serde_json::Map<String, Value>,
    ) -> Configuration {
        let label = format!("{} language {language:?}", identity.owner);
        for key in ["indentationRules", "onEnterRules", "wordPattern", "folding"] {
            if object.contains_key(key) {
                self.warn(format!(
                    "{label}: {key} is unsupported by the native declarative configuration loader"
                ));
            }
        }
        let auto_closing_pairs = self.table(
            object.get("autoClosingPairs"),
            &label,
            "autoClosingPairs",
            parse_pair,
        );
        let surrounding_pairs = self.table(
            object.get("surroundingPairs"),
            &label,
            "surroundingPairs",
            parse_delimiter,
        );
        let brackets = self.table(object.get("brackets"), &label, "brackets", |value| {
            if !value.is_array() {
                bail!("bracket pair must be a two-string array")
            }
            parse_delimiter(value)
        });
        let auto_close_before = object.get("autoCloseBefore").and_then(|value| {
            match value.as_str() {
                Some("") => None,
                Some(value) if bounded_string(value, 256, true) && value.chars().count() <= 64 => Some(value.to_owned()),
                Some(_) => {
                    self.warn(format!("{label}: invalid autoCloseBefore; native auto-closing disabled before other characters"));
                    Some(String::new())
                }
                None => {
                    self.warn(format!("{label}: autoCloseBefore must be a string; field omitted"));
                    None
                }
            }
        });
        let comments = object
            .get("comments")
            .and_then(|value| match parse_comments(value) {
                Ok((comments, issues)) => {
                    for issue in issues {
                        self.warn(format!("{label}: {issue}; comment field omitted"));
                    }
                    comments
                }
                Err(error) => {
                    self.warn(format!(
                        "{label}: invalid comments ({error:#}); comments omitted"
                    ));
                    None
                }
            });
        Configuration {
            identity,
            language: language.to_owned(),
            auto_closing_pairs,
            surrounding_pairs,
            auto_close_before,
            brackets,
            comments,
        }
    }

    fn table<T>(
        &mut self,
        value: Option<&Value>,
        label: &str,
        name: &str,
        parse: impl Fn(&Value) -> Result<T>,
    ) -> Option<Vec<T>> {
        let value = value?;
        let Some(array) = value.as_array() else {
            self.warn(format!("{label}: {name} must be an array; field omitted"));
            return None;
        };
        if array.len() > PAIRS {
            self.warn(format!(
                "{label}: {name} exceeds 64 pairs; native table disabled"
            ));
            return Some(Vec::new());
        }
        let mut result = Vec::new();
        let mut unsupported = false;
        for (index, value) in array.iter().enumerate() {
            match parse(value) {
                Ok(value) => result.push(value),
                Err(error) => {
                    unsupported |= error.downcast_ref::<Unsupported>().is_some();
                    self.warn(format!("{label}: {name}[{index}] omitted ({error:#})"));
                }
            }
        }
        if result.is_empty() && !unsupported {
            None
        } else {
            Some(result)
        }
    }
}

fn scalar(value: &Value) -> Result<char> {
    let string = value.as_str().context("delimiter must be a string")?;
    let mut chars = string.chars();
    let ch = chars
        .next()
        .ok_or(Unsupported("empty delimiters are unsupported natively"))?;
    if matches!(ch, '\0' | '\r' | '\n') || chars.next().is_some() {
        return Err(Unsupported(
            "only non-NUL, non-line-break single-scalar delimiters are supported",
        )
        .into());
    }
    Ok(ch)
}

fn parse_delimiter(value: &Value) -> Result<Delimiter> {
    if let Some(array) = value.as_array() {
        if array.len() != 2 {
            bail!("delimiter pair must have exactly two entries")
        }
        Ok(Delimiter {
            open: scalar(&array[0])?,
            close: scalar(&array[1])?,
        })
    } else if let Some(object) = value.as_object() {
        Ok(Delimiter {
            open: scalar(object.get("open").context("pair requires open")?)?,
            close: scalar(object.get("close").context("pair requires close")?)?,
        })
    } else {
        bail!("delimiter pair must be an array or an object")
    }
}

fn parse_pair(value: &Value) -> Result<Pair> {
    let delimiter = parse_delimiter(value)?;
    let mut pair = Pair {
        open: delimiter.open,
        close: delimiter.close,
        not_string: false,
        not_comment: false,
    };
    if let Some(guards) = value.get("notIn") {
        let guards = guards.as_array().context("notIn must be an array")?;
        if guards.len() > 64 {
            return Err(Unsupported("notIn exceeds 64 entries").into());
        }
        for guard in guards {
            match guard.as_str() {
                Some("string") => pair.not_string = true,
                Some("comment") => pair.not_comment = true,
                _ => return Err(Unsupported("unsupported notIn guard").into()),
            }
        }
    }
    Ok(pair)
}

fn parse_comments(value: &Value) -> Result<(Option<Comments>, Vec<&'static str>)> {
    let object = value.as_object().context("comments must be an object")?;
    let mut issues = Vec::new();
    let line_comment = object.get("lineComment").and_then(|value| {
        let result = value
            .as_str()
            .filter(|value| bounded_string(value, 256, false) && !value.contains(['\r', '\n']))
            .map(str::to_owned);
        if result.is_none() {
            issues.push("lineComment requires nonempty single-line text up to 256 bytes");
        }
        result
    });
    let block_comment = object.get("blockComment").and_then(|value| {
        let result = value
            .as_array()
            .filter(|array| array.len() == 2)
            .and_then(|array| {
                let open = array[0].as_str().filter(|value| {
                    bounded_string(value, 256, false) && !value.contains(['\r', '\n'])
                })?;
                let close = array[1].as_str().filter(|value| {
                    bounded_string(value, 256, false) && !value.contains(['\r', '\n'])
                })?;
                Some((open.to_owned(), close.to_owned()))
            });
        if result.is_none() {
            issues.push(
                "blockComment requires two nonempty single-line strings up to 256 bytes each",
            );
        }
        result
    });
    if line_comment.is_none() && block_comment.is_none() {
        return Ok((None, issues));
    }
    Ok((
        Some(Comments {
            line_comment,
            block_comment,
        }),
        issues,
    ))
}

#[derive(Debug)]
struct Unsupported(&'static str);
impl std::fmt::Display for Unsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for Unsupported {}

fn inherit(configuration: &mut Configuration, previous: &Configuration) {
    if configuration.auto_closing_pairs.is_none() {
        configuration
            .auto_closing_pairs
            .clone_from(&previous.auto_closing_pairs)
    }
    if configuration.surrounding_pairs.is_none() {
        configuration
            .surrounding_pairs
            .clone_from(&previous.surrounding_pairs)
    }
    if configuration.auto_close_before.is_none() {
        configuration
            .auto_close_before
            .clone_from(&previous.auto_close_before)
    }
    if configuration.brackets.is_none() {
        configuration.brackets.clone_from(&previous.brackets)
    }
    if configuration.comments.is_none() {
        configuration.comments.clone_from(&previous.comments)
    }
}

fn composition(configuration: &Configuration, previous: Option<&Configuration>) -> String {
    let mut digest = Sha256::new();
    let identity = &configuration.identity;
    for bytes in [
        previous
            .map(|p| p.identity.composition_sha256.as_bytes())
            .unwrap_or_default(),
        identity.owner.as_bytes(),
        identity.version.as_bytes(),
        identity.package_path.as_os_str().as_encoded_bytes(),
        identity.archive_sha256.as_bytes(),
        identity.configuration_path.as_os_str().as_encoded_bytes(),
        identity.content_sha256.as_bytes(),
        configuration.language.as_bytes(),
    ] {
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    // These bounded typed values have deterministic Debug representations;
    // lengths frame the source identity and distinguish all effective fields.
    digest.update(
        format!(
            "{:?}{:?}{:?}{:?}{:?}",
            configuration.auto_closing_pairs,
            configuration.surrounding_pairs,
            configuration.auto_close_before,
            configuration.brackets,
            configuration.comments
        )
        .as_bytes(),
    );
    format!("{:x}", digest.finalize())
}

fn bounded_string(value: &str, bytes: usize, empty: bool) -> bool {
    (empty || !value.is_empty()) && value.len() <= bytes && !value.contains('\0')
}

fn validate_package(package: &Installed) -> Result<()> {
    crate::extension_activation::validate_id(&package.id)?;
    if !bounded_string(&package.version, 256, false) || !bounded_string(&package.sha256, 256, true)
    {
        bail!("Installed version/archive identity exceeds its metadata bounds");
    }
    Ok(())
}

fn read_configuration(
    root: &Path,
    relative: &str,
    remaining: &mut usize,
) -> Result<(PathBuf, PathBuf, Vec<u8>)> {
    let relative_path = Path::new(relative);
    if !bounded_string(relative, 1024, false)
        || relative.contains(['\\', ':'])
        || relative_path.is_absolute()
        || relative_path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        bail!("Configuration path must stay inside its installed package");
    }
    let root = fs::canonicalize(root).context("Cannot resolve installed package")?;
    let path = fs::canonicalize(root.join(relative_path))
        .context("Cannot resolve language configuration")?;
    if !path.starts_with(&root) || !fs::symlink_metadata(&path)?.is_file() {
        bail!("Configuration escapes its installed package or is not a regular file");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&path)
        .context("Cannot open language configuration")?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("Language configuration is not a regular file")
    }
    if metadata.len() > FILE_BYTES as u64 {
        bail!("Language configuration exceeds 64 KiB")
    }
    if *remaining == 0 {
        bail!("Language configuration catalog exceeds 4 MiB actual reads")
    }
    let limit = (FILE_BYTES + 1).min(*remaining);
    let mut bytes = Vec::new();
    let result = file.take(limit as u64).read_to_end(&mut bytes);
    *remaining -= bytes.len();
    result.context("Cannot read language configuration")?;
    if bytes.len() > FILE_BYTES {
        bail!("Language configuration exceeds 64 KiB")
    }
    if bytes.len() == limit && limit <= FILE_BYTES {
        bail!(
            "Language configuration catalog exhausted its 4 MiB read budget before a complete file proof"
        );
    }
    let relative = relative_path
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect::<PathBuf>();
    Ok((root, relative, bytes))
}
