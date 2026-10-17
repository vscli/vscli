//! Default package.nls.json data, following the pinned MIT extensionNls.ts algorithm.
//! Copyright (c) Microsoft Corporation. The full MIT notice is preserved in
//! extension-host/upstream-manifest-nls.cjs alongside the reused upstream code.
//! Preferred-language packs remain a separate qualification requirement.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{fs, io::Read, path::Path};
const MAX_FILE: usize = 1024 * 1024;
const MAX_TOTAL: usize = 8 * MAX_FILE;
#[derive(Default)]
pub(crate) struct Defaults {
    bytes: usize,
    pub warnings: Vec<String>,
}
impl Defaults {
    pub fn localize(&mut self, root: &Path, manifest: &Value) -> Value {
        match self.try_localize(root, manifest) {
            Ok(value) => value,
            Err(error) => {
                if self.warnings.len() < 128 {
                    self.warnings.push(format!(
                        "Manifest localization unavailable; original labels retained: {error:#}"
                    ));
                }
                manifest.clone()
            }
        }
    }
    fn try_localize(&mut self, root: &Path, manifest: &Value) -> Result<Value> {
        let file = root.join("package.nls.json");
        let metadata = match fs::symlink_metadata(&file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(manifest.clone());
            }
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE as u64 {
            bail!("package.nls.json must be a regular file at most 1 MiB");
        }
        let root = fs::canonicalize(root)?;
        if !fs::canonicalize(&file)?.starts_with(&root) {
            bail!("package.nls.json escapes its package");
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let mut input = options.open(file)?;
        let before = input.metadata()?;
        if !before.is_file() || before.len() > MAX_FILE as u64 {
            bail!("package.nls.json changed type or exceeds 1 MiB");
        }
        let remaining = MAX_TOTAL.saturating_sub(self.bytes);
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut input)
            .take((MAX_FILE.min(remaining) + 1) as u64)
            .read_to_end(&mut bytes)?;
        self.bytes = self.bytes.saturating_add(bytes.len());
        let after = input.metadata()?;
        if bytes.len() > remaining || bytes.len() > MAX_FILE {
            bail!("Manifest translations exceed the 8 MiB batch / 1 MiB file budget");
        }
        if bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified()? != after.modified()?
        {
            bail!("package.nls.json changed during capture");
        }
        let messages: Value = serde_json::from_slice(&bytes).context("Invalid package.nls.json")?;
        let messages = messages
            .as_object()
            .context("package.nls.json must contain an object")?;
        if messages.len() > 16384 {
            bail!("Manifest translations exceed 16384 keys");
        }
        let mut value = manifest.clone();
        let mut size = serde_json::to_vec(&value)?.len();
        if size > MAX_FILE {
            bail!("Localized manifest exceeds 1 MiB");
        }
        replace(&mut value, messages, &mut size, 0)?;
        Ok(value)
    }
}
fn replace(
    value: &mut Value,
    messages: &serde_json::Map<String, Value>,
    bytes: &mut usize,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        bail!("Manifest localization exceeds 64 levels");
    }
    match value {
        Value::String(text) => {
            if text.len() > 1 && text.starts_with('%') && text.ends_with('%') {
                let entry = messages.get(&text[1..text.len() - 1]);
                let message = entry.and_then(|entry| {
                    entry
                        .as_str()
                        .or_else(|| entry.get("message").and_then(Value::as_str))
                });
                if let Some(message) = message.filter(|message| !message.is_empty()) {
                    let size = bytes
                        .saturating_sub(serde_json::to_vec(text)?.len())
                        .saturating_add(serde_json::to_vec(message)?.len());
                    if size > MAX_FILE {
                        bail!("Localized manifest exceeds 1 MiB");
                    }
                    *bytes = size;
                    *text = message.into();
                }
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace(value, messages, bytes, depth + 1)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                replace(value, messages, bytes, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn default_strings_comment_objects_exact_placeholders_and_fallback_preserve_source() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("package.nls.json");
        let bytes = br#"{"title":"Run default","label":{"message":"Default label","comment":["context"]},"empty":""}"#;
        fs::write(&path, bytes).unwrap();
        let original = json!({"displayName":"%label%","contributes":{"commands":[{"title":"%title%","category":"%label%"}],"themes":[{"label":"%label%"}]},"unknown":"%unknown%","empty":"%empty%","embedded":"before %title%","percent":"%%"});
        let mut reader = Defaults::default();
        let localized = reader.localize(root.path(), &original);
        assert!(reader.warnings.is_empty());
        assert_eq!(
            localized["contributes"]["commands"][0]["title"],
            "Run default"
        );
        assert_eq!(localized["displayName"], "Default label");
        assert_eq!(
            localized["contributes"]["themes"][0]["label"],
            "Default label"
        );
        for key in ["unknown", "empty", "embedded", "percent"] {
            assert_eq!(localized[key], original[key]);
        }
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(original["displayName"], "%label%");
    }
    #[test]
    fn malformed_expanding_and_exhausted_translations_keep_the_whole_original_manifest() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("package.nls.json");
        let original = json!({"a":"%long%","b":"%long%"});
        for bytes in [
            b"not JSON".to_vec(),
            serde_json::to_vec(&json!({"long":"x".repeat(600*1024)})).unwrap(),
        ] {
            fs::write(&file, &bytes).unwrap();
            let mut reader = Defaults::default();
            assert_eq!(reader.localize(root.path(), &original), original);
            assert_eq!(reader.warnings.len(), 1);
            assert_eq!(fs::read(&file).unwrap(), bytes);
        }
        fs::write(&file, br#"{"long":"valid"}"#).unwrap();
        let mut reader = Defaults {
            bytes: MAX_TOTAL,
            warnings: Vec::new(),
        };
        assert_eq!(reader.localize(root.path(), &original), original);
        assert_eq!(reader.warnings.len(), 1);
    }
    #[test]
    fn native_default_normalization_matches_the_reused_upstream_algorithm() {
        let root = tempfile::tempdir().unwrap();
        let messages = serde_json::json!({"title":"Run default","label":{"message":"Default label","comment":["context"]},"empty":""});
        fs::write(
            root.path().join("package.nls.json"),
            serde_json::to_vec(&messages).unwrap(),
        )
        .unwrap();
        let original = serde_json::json!({"name":"fixture","displayName":"%label%","contributes":{"commands":[{"title":"%title%","category":"%label%"}],"themes":[{"label":"%label%"}]},"unknown":"%unknown%","empty":"%empty%","embedded":"before %title%","percent":"%%","nested":["%title%",null,true,{"key":"%label%"}]});
        let upstream = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("extension-host/upstream-manifest-nls.cjs");
        let output=std::process::Command::new("node").arg("-e").arg("const module=require(process.argv[1]),input=JSON.parse(process.argv[2]);process.stdout.write(JSON.stringify(module.localizeManifest({warn(){},error(error){throw Error(error);}},input.manifest,input.messages)));").arg(upstream).arg(serde_json::to_string(&serde_json::json!({"manifest":original,"messages":messages})).unwrap()).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: Value = serde_json::from_slice(&output.stdout).unwrap();
        let mut defaults = Defaults::default();
        assert_eq!(defaults.localize(root.path(), &original), expected);
        assert!(defaults.warnings.is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_and_fifos_do_not_open_outside_or_block_the_loader() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let file = root.path().join("package.nls.json");
        symlink(external.path(), &file).unwrap();
        let original = json!({"label":"%label%"});
        let mut reader = Defaults::default();
        assert_eq!(reader.localize(root.path(), &original), original);
        assert_eq!(reader.warnings.len(), 1);
        fs::remove_file(&file).unwrap();
        let name = std::ffi::CString::new(file.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(reader.localize(root.path(), &original), original);
        assert_eq!(reader.warnings.len(), 2);
    }
}
