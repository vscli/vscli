//! Bounded, native Open VSX reads and explicit, identity-checked package installation.
//! Call only in background workers (or the noninteractive CLI), never while rendering.
use crate::extension_store::{Installed, Store};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use url::Url;

pub const DEFAULT_URL: &str = "https://open-vsx.org";
pub const MAX_RESULTS: usize = 20;
const MAX_METADATA: u64 = 1024 * 1024;
const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub license: String,
    pub platform: String,
    pub download: String,
    pub checksum: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Updates {
    pub items: Vec<Entry>,
    pub notices: Vec<String>,
}
#[derive(Clone)]
pub struct Registry {
    base: Url,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new(DEFAULT_URL).expect("valid built-in registry URL")
    }
}
fn validate_url(url: &Url) -> Result<()> {
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        bail!("Registry URLs require HTTPS (HTTP is supported only on numeric loopback)");
    }
    Ok(())
}
fn id_parts(id: &str) -> Result<(&str, &str)> {
    let (namespace, name) = id
        .split_once('.')
        .context("Extension ID must be publisher.name")?;
    if [namespace, name].iter().any(|p| {
        p.is_empty()
            || p.len() > 100
            || !p
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }) {
        bail!("Extension ID must be publisher.name");
    }
    Ok((namespace, name))
}
fn field(value: &serde_json::Value, key: &str, max: usize) -> Result<String> {
    let s = value[key].as_str().unwrap_or("");
    if s.len() > max {
        bail!("Registry field {key} exceeds its display budget");
    }
    Ok(s.into())
}
pub fn target_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if cfg!(target_env = "musl") => "alpine-x64",
        ("linux", "aarch64") if cfg!(target_env = "musl") => "alpine-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "arm") => "linux-armhf",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        ("windows", "x86_64") => "win32-x64",
        ("windows", "aarch64") => "win32-arm64",
        ("windows", "x86") => "win32-ia32",
        _ => "universal",
    }
}
impl Entry {
    fn parse(value: &serde_json::Value) -> Result<Self> {
        if value["error"].is_string() {
            bail!("Registry: {}", field(value, "error", 1024)?);
        }
        let namespace = field(value, "namespace", 100)?;
        let name = field(value, "name", 100)?;
        let id = format!("{namespace}.{name}").to_ascii_lowercase();
        id_parts(&id)?;
        let version = field(value, "version", 100)?;
        let parsed =
            semver::Version::parse(&version).context("Invalid registry extension version")?;
        if !parsed.pre.is_empty() || value["preRelease"] == true {
            bail!("Prerelease packages are not selected automatically");
        }
        let platform = match value["targetPlatform"].as_str() {
            None => bail!("Registry metadata lacks targetPlatform"),
            Some(s) => s,
        };
        if platform.len() > 32 {
            bail!("Registry platform exceeds its field budget");
        }
        if platform != "universal" && platform != target_platform() {
            bail!("Package target {platform} does not match this machine");
        }
        let download = value["files"]["download"]
            .as_str()
            .context("Registry package has no download URL")?;
        if download.len() > 4096 {
            bail!("Download URL exceeds its budget");
        }
        validate_url(&Url::parse(download)?)?;
        let checksum = value["files"]["sha256"].as_str().map(str::to_owned);
        if let Some(s) = &checksum {
            if s.len() > 4096 {
                bail!("Checksum URL exceeds its budget");
            }
            validate_url(&Url::parse(s)?)?;
        }
        Ok(Self {
            id,
            version,
            name: field(value, "displayName", 512)?,
            description: field(value, "description", 4096)?,
            license: field(value, "license", 256)?,
            platform: platform.into(),
            download: download.into(),
            checksum,
        })
    }
}
fn canceled(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Registry operation canceled");
    }
    if Instant::now() >= deadline {
        bail!("Registry operation deadline exceeded");
    }
    Ok(())
}
impl Registry {
    pub fn new(base: &str) -> Result<Self> {
        if base.len() > 4096 {
            bail!("Registry base URL exceeds 4 KiB");
        }
        let mut base = Url::parse(base)?;
        validate_url(&base)?;
        if base.query().is_some() {
            bail!("Registry base URL cannot contain a query");
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        Ok(Self { base })
    }
    fn request(
        &self,
        mut url: Url,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        for _ in 0..=5 {
            canceled(cancel, deadline)?;
            validate_url(&url)?;
            if self.base.scheme() == "https" && url.scheme() != "https" {
                bail!("HTTPS registry requests cannot downgrade to HTTP");
            }
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(deadline.saturating_duration_since(Instant::now())))
                .timeout_connect(Some(Duration::from_secs(5)))
                .timeout_resolve(Some(Duration::from_secs(5)))
                .timeout_recv_body(Some(Duration::from_secs(10)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into();
            let response = agent
                .get(url.as_str())
                .header("User-Agent", "VSCLI/0.1")
                .header("Accept-Encoding", "identity")
                .call()?;
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                let next = response
                    .headers()
                    .get("location")
                    .context("Registry redirect without location")?
                    .to_str()?;
                url = url.join(next)?;
                continue;
            }
            return Ok(response);
        }
        bail!("Registry redirect limit exceeded")
    }
    fn bytes(
        &self,
        url: Url,
        limit: u64,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Vec<u8>> {
        self.read_bytes(
            self.request(url, cancel, deadline)?,
            limit,
            cancel,
            deadline,
        )
    }
    fn read_bytes(
        &self,
        mut response: ureq::http::Response<ureq::Body>,
        limit: u64,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Vec<u8>> {
        if !response.status().is_success() {
            bail!("Registry HTTP {}", response.status().as_u16());
        }
        if response
            .body()
            .content_length()
            .is_some_and(|size| size > limit)
        {
            bail!("Registry response exceeds its byte budget");
        }
        let mut data = Vec::new();
        let mut reader = response.body_mut().as_reader().take(limit + 1);
        let mut buffer = [0; 16384];
        loop {
            canceled(cancel, deadline)?;
            let size = reader.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..size]);
            if data.len() as u64 > limit {
                bail!("Registry response exceeds its byte budget");
            }
        }
        Ok(data)
    }
    pub fn search(&self, query: &str, cancel: &AtomicBool) -> Result<Vec<Entry>> {
        if query.len() > 1024 {
            bail!("Extension search query exceeds 1 KiB");
        }
        let mut url = self.base.join("api/-/search")?;
        url.query_pairs_mut()
            .append_pair("query", query)
            .append_pair("size", "20");
        let deadline = Instant::now() + DEADLINE;
        let value: serde_json::Value =
            serde_json::from_slice(&self.bytes(url, MAX_METADATA, cancel, deadline)?)?;
        if value["error"].is_string() {
            bail!("Registry: {}", field(&value, "error", 1024)?);
        }
        let values = value["extensions"]
            .as_array()
            .context("Registry search lacks extensions")?;
        if values.len() > MAX_RESULTS {
            bail!("Registry search exceeds 20 results");
        }
        let mut entries = Vec::new();
        for value in values {
            let entry = if value["targetPlatform"].is_string() {
                Entry::parse(value)?
            } else {
                // Search summaries omit target platform and license. Never infer
                // a platform from an arbitrary summary download URL.
                let id = format!(
                    "{}.{}",
                    field(value, "namespace", 100)?,
                    field(value, "name", 100)?
                )
                .to_ascii_lowercase();
                let version = field(value, "version", 100)?;
                match self.version_until(&id, &version, cancel, deadline)? {
                    Some(entry) => entry,
                    None => continue,
                }
            };
            entries.push(entry);
        }
        Ok(entries)
    }
    fn version_until(
        &self,
        id: &str,
        version: &str,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Option<Entry>> {
        if version != "latest" {
            semver::Version::parse(version)?;
        }

        let (namespace, name) = id_parts(id)?;
        // Prefer the exact host build; the universal route is a deliberate fallback.
        for platform in [target_platform(), "universal"] {
            let url = self
                .base
                .join(&format!("api/{namespace}/{name}/{platform}/{version}"))?;
            let response = self.request(url.clone(), cancel, deadline)?;
            if response.status().as_u16() == 404 {
                continue;
            }
            let value = serde_json::from_slice(&self.read_bytes(
                response,
                MAX_METADATA,
                cancel,
                deadline,
            )?)?;
            let entry = Entry::parse(&value)?;
            if !entry.id.eq_ignore_ascii_case(id) {
                bail!("Registry metadata returned a different package identity");
            }
            if version != "latest" && entry.version != version {
                bail!("Registry metadata returned a different package version");
            }
            return Ok(Some(entry));
        }
        Ok(None)
    }
    fn latest_until(&self, id: &str, cancel: &AtomicBool, deadline: Instant) -> Result<Entry> {
        self.version_until(id, "latest", cancel, deadline)?
            .with_context(|| format!("No compatible stable package found for {id}"))
    }
    pub fn latest(&self, id: &str, cancel: &AtomicBool) -> Result<Entry> {
        self.latest_until(id, cancel, Instant::now() + DEADLINE)
    }
    pub fn updates(&self, installed: &[Installed], cancel: &AtomicBool) -> Result<Updates> {
        if installed.len() > 128 {
            bail!("Check at most 128 installed packages per request");
        }
        let deadline = Instant::now() + DEADLINE;
        let mut report = Updates {
            items: Vec::new(),
            notices: Vec::new(),
        };
        for item in installed {
            canceled(cancel, deadline)?;
            let check = (|| -> Result<Option<Entry>> {
                let current = semver::Version::parse(&item.version)?;
                let entry = self.latest_until(&item.id, cancel, deadline)?;
                Ok((semver::Version::parse(&entry.version)? > current).then_some(entry))
            })();
            match check {
                Ok(Some(entry)) => report.items.push(entry),
                Ok(None) => {}
                Err(error) => report.notices.push(
                    format!("{}: {error:#}", item.id)
                        .chars()
                        .take(512)
                        .collect(),
                ),
            }
        }
        Ok(report)
    }
    pub fn install(&self, entry: &Entry, store: &Store, cancel: &AtomicBool) -> Result<Installed> {
        id_parts(&entry.id)?;
        let deadline = Instant::now() + DEADLINE;
        let checksum = match &entry.checksum {
            None => None,
            Some(url) => {
                let data = self.bytes(Url::parse(url)?, 4096, cancel, deadline)?;
                let value = std::str::from_utf8(&data)?
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!("Invalid registry SHA-256 checksum");
                }
                Some(value.to_ascii_lowercase())
            }
        };
        let mut response = self.request(Url::parse(&entry.download)?, cancel, deadline)?;
        if !response.status().is_success() {
            bail!("Registry download HTTP {}", response.status().as_u16());
        }
        if response
            .body()
            .content_length()
            .is_some_and(|size| size > MAX_DOWNLOAD)
        {
            bail!("Registry download exceeds 128 MiB");
        }
        let mut archive = tempfile::NamedTempFile::new()?;
        let mut reader = response.body_mut().as_reader().take(MAX_DOWNLOAD + 1);
        let mut buffer = [0; 65536];
        let mut digest = Sha256::new();
        let mut total = 0;
        loop {
            canceled(cancel, deadline)?;
            let size = reader.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            total += size as u64;
            if total > MAX_DOWNLOAD {
                bail!("Registry download exceeds 128 MiB");
            }
            archive.write_all(&buffer[..size])?;
            digest.update(&buffer[..size]);
        }
        if let Some(expected) = checksum
            && format!("{:x}", digest.finalize()) != expected
        {
            bail!("Registry download checksum mismatch; installation retained");
        }
        archive.flush()?;
        canceled(cancel, deadline)?;
        store.install_registry(archive.path(), &entry.id, &entry.version, &entry.download)
    }
}
