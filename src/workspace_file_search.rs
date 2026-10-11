//! Native workspace.findFiles traversal. No editor model or UI authorization.
//! Captures borrow immutable native settings; all glob compilation/filesystem
//! work runs in one retained worker. Cancellation never releases actual capacity.
use anyhow::{Context as _, Result, ensure};
use globset::{GlobBuilder, GlobMatcher};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) const LIFETIME: Duration = Duration::from_secs(10);
const MAX_RESULTS: usize = 4096;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_VISITED_PATH_BYTES: usize = 16 * 1024 * 1024;
const MAX_PATH: usize = 4096;
const MAX_PATTERNS: usize = 128;
const MAX_PATTERN: usize = 1024;
const MAX_PATTERN_BYTES: usize = 32 * 1024;
const DEFAULT_EXCLUDES: [&str; 6] = [
    "**/.git",
    "**/.svn",
    "**/.hg",
    "**/CVS",
    "**/.DS_Store",
    "**/Thumbs.db",
];

#[derive(Clone)]
pub(crate) struct Context {
    root: PathBuf,
    settings: Arc<Vec<Map<String, Value>>>,
    profile: u64,
}
impl Context {
    pub(crate) fn new(
        root: PathBuf,
        settings: Arc<Vec<Map<String, Value>>>,
        profile: u64,
    ) -> Result<Self> {
        check_path(&root)?;
        ensure!(
            settings.len() <= 32,
            "findFiles settings layer budget exceeded"
        );
        Ok(Self {
            root,
            settings,
            profile,
        })
    }
    pub(crate) fn current(
        &self,
        root: &Path,
        settings: &Arc<Vec<Map<String, Value>>>,
        profile: u64,
    ) -> bool {
        self.root == root && self.profile == profile && Arc::ptr_eq(&self.settings, settings)
    }
}

fn check_exclude_inputs(context: &Context) -> Result<()> {
    // Bounds before any contributed pattern/value clone. Different layers
    // are merged on the worker, preserving later false overrides.
    let (mut entries, mut bytes) = (0usize, 0usize);
    for layer in context.settings.iter() {
        if let Some(value) = layer.get("files.exclude") {
            let table = value.as_object().context("Invalid files.exclude object")?;
            entries = entries
                .checked_add(table.len())
                .context("Exclude count overflow")?;
            ensure!(entries <= 256, "findFiles exclude input budget exceeded");
            for (pattern, value) in table {
                check_pattern(pattern)?;
                bytes = bytes
                    .checked_add(pattern.len())
                    .context("Exclude bytes overflow")?;
                if value.is_boolean() {
                    continue;
                }
                let object = fields(value, &["when"])?;
                let when = object["when"]
                    .as_str()
                    .context("Invalid conditional file exclude")?;
                ensure!(
                    !when.is_empty()
                        && when.len() <= 256
                        && !when.contains(['/', '\\', '\0'])
                        && !when.replacen("$(basename)", "name", 1).contains('$'),
                    "Unsupported conditional files.exclude sibling"
                );
                bytes = bytes
                    .checked_add(when.len())
                    .context("Exclude bytes overflow")?;
            }
        }
    }
    ensure!(
        bytes <= MAX_PATTERN_BYTES,
        "findFiles exclude byte budget exceeded"
    );
    Ok(())
}

pub(crate) struct Capture {
    context: Context,
    base: PathBuf,
    include: Option<String>,
    exclude: Option<String>,
    default_excludes: bool,
    max_results: Option<usize>,
    deadline: Instant,
}
impl Capture {
    pub(crate) fn new(context: Context, args: &Value, started: Instant) -> Result<Arc<Self>> {
        let object = fields(args, &["include", "exclude", "maxResults"])?;
        let include = fields(&object["include"], &["pattern", "baseUri"])?;
        let pattern = nullable_pattern(&include["pattern"])?;
        let base = if include["baseUri"].is_null() {
            context.root.clone()
        } else {
            let uri = include["baseUri"]
                .as_str()
                .context("Invalid relative-pattern base URI")?;
            ensure!(
                uri.len() <= MAX_PATH * 3 && !uri.contains('\0'),
                "Relative base URI budget exceeded"
            );
            let parsed = url::Url::parse(uri)?;
            ensure!(
                parsed.scheme() == "file"
                    && parsed.query().is_none()
                    && parsed.fragment().is_none(),
                "findFiles supports local file RelativePattern bases only"
            );
            let path = parsed
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("Unsupported file URI authority"))?;
            check_path(&path)?;
            path
        };
        let exclude = fields(&object["exclude"], &["mode", "pattern"])?;
        let mode = exclude["mode"]
            .as_str()
            .context("Invalid findFiles exclude mode")?;
        let exclude_pattern = nullable_pattern(&exclude["pattern"])?;
        let default_excludes = match mode {
            "default" | "none" => {
                ensure!(
                    exclude_pattern.is_none(),
                    "Unexpected exclude pattern for mode"
                );
                mode == "default"
            }
            "pattern" => {
                ensure!(
                    exclude_pattern.is_some(),
                    "Missing explicit exclude pattern"
                );
                true
            }
            _ => anyhow::bail!("Invalid findFiles exclude mode"),
        };
        if default_excludes {
            check_exclude_inputs(&context)?;
        }
        let max_results = if object["maxResults"].is_null() {
            None
        } else {
            let value = object["maxResults"]
                .as_u64()
                .context("Invalid findFiles maxResults")?;
            ensure!(
                value <= MAX_RESULTS as u64,
                "findFiles maxResults exceeds 4096"
            );
            Some(value as usize)
        };
        let deadline = started
            .checked_add(LIFETIME)
            .context("findFiles deadline overflow")?;
        ensure!(
            Instant::now() < deadline,
            "findFiles expired before receipt"
        );
        Ok(Arc::new(Self {
            context,
            base,
            include: pattern,
            exclude: exclude_pattern,
            default_excludes,
            max_results,
            deadline,
        }))
    }
    pub(crate) fn current(
        &self,
        root: &Path,
        settings: &Arc<Vec<Map<String, Value>>>,
        profile: u64,
    ) -> bool {
        self.context.current(root, settings, profile)
    }
}
fn fields<'a>(value: &'a Value, names: &[&str]) -> Result<&'a Map<String, Value>> {
    let object = value
        .as_object()
        .context("findFiles arguments must be objects")?;
    ensure!(
        object.len() == names.len() && names.iter().all(|name| object.contains_key(*name)),
        "Unsupported or missing findFiles fields"
    );
    Ok(object)
}
fn check_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && path.as_os_str().len() <= MAX_PATH
            && !path.to_string_lossy().contains('\0'),
        "findFiles path must be absolute and at most 4096 bytes"
    );
    Ok(())
}
fn nullable_pattern(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let text = value.as_str().context("Invalid findFiles glob")?;
    check_pattern(text)?;
    Ok(Some(text.to_owned()))
}
fn check_pattern(text: &str) -> Result<()> {
    ensure!(
        text.len() <= MAX_PATTERN
            && !text.contains(['\0', '\\'])
            && !text.starts_with('/')
            && !text.split('/').any(|part| part == ".."),
        "Unsupported or oversized findFiles glob"
    );
    // Globset has no caller-configured regex compilation budget. Restrict
    // recursive brace expansion before compiling rather than permitting a tiny
    // input to expand into an unbounded expression.
    let (mut braces, mut alternatives, mut wildcards) = (0usize, 0usize, 0usize);
    for ch in text.chars() {
        match ch {
            '{' => {
                braces += 1;
                ensure!(braces <= 1, "Nested glob braces are unsupported");
            }
            '}' => {
                ensure!(braces == 1, "Unbalanced glob brace");
                braces = 0;
            }
            ',' if braces == 1 => alternatives += 1,
            '*' | '?' | '[' => wildcards += 1,
            _ => {}
        }
    }
    ensure!(
        braces == 0 && alternatives <= 32 && wildcards <= 128,
        "findFiles glob expansion budget exceeded"
    );
    Ok(())
}
fn compile(pattern: &str) -> Result<GlobMatcher> {
    check_pattern(pattern)?;
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()?
        .compile_matcher())
}
#[derive(Debug)]
pub(crate) struct Output {
    pub uris: Vec<String>,
    pub cancelled: bool,
}
impl Output {
    fn cancelled() -> Self {
        Self {
            uris: Vec::new(),
            cancelled: true,
        }
    }
}
struct Actual {
    token: u64,
    cancel: Arc<AtomicBool>,
    retired: Arc<AtomicBool>,
    deadline: Instant,
    receiver: Receiver<Result<Output, String>>,
    thread: JoinHandle<()>,
}
#[derive(Default)]
pub(crate) struct Worker {
    next: u64,
    active: Option<Actual>,
    #[cfg(test)]
    gates: Option<Arc<Gates>>,
}
impl Worker {
    pub(crate) fn occupied(&self) -> bool {
        self.active.is_some()
    }
    pub(crate) fn start(
        &mut self,
        capture: Arc<Capture>,
        caller_retired: Arc<AtomicBool>,
    ) -> Result<u64> {
        ensure!(!self.occupied(), "findFiles actual worker is occupied");
        ensure!(
            !caller_retired.load(Ordering::Acquire),
            "findFiles caller retired"
        );
        ensure!(
            Instant::now() < capture.deadline,
            "findFiles expired before dispatch"
        );
        let token = self
            .next
            .checked_add(1)
            .context("findFiles request counter exhausted")?;
        let deadline = capture.deadline;
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let retired = caller_retired.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        #[cfg(test)]
        let gates = self.gates.clone();
        let thread = thread::Builder::new()
            .name("vscli-find-files".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(gates) = &gates {
                    gates.wait(GatePoint::BeforeWalk);
                }
                let result = walk(&capture, &flag, &retired).map_err(|error| format!("{error:#}"));
                let _ = sender.send(result);
                #[cfg(test)]
                if let Some(gates) = &gates {
                    gates.wait(GatePoint::AfterReply);
                }
            })?;
        self.next = token;
        self.active = Some(Actual {
            token,
            cancel,
            retired: caller_retired,
            deadline,
            receiver,
            thread,
        });
        Ok(token)
    }
    pub(crate) fn cancel(&mut self, token: u64) {
        if let Some(actual) = &self.active
            && actual.token == token
        {
            actual.cancel.store(true, Ordering::Release);
        }
    }
    pub(crate) fn poll(&mut self) -> Option<(u64, Result<Output, String>)> {
        if !self.active.as_ref()?.thread.is_finished() {
            return None;
        }
        let actual = self.active.take()?;
        // Cancellation/retirement observed AFTER actual exit and join also
        // covers a reply already sent while the worker still held its slot.
        let result = if actual.thread.join().is_err() {
            Err("findFiles worker panicked".into())
        } else if actual.retired.load(Ordering::Acquire) {
            Err("findFiles caller retired".into())
        } else if actual.cancel.load(Ordering::Acquire) {
            Ok(Output::cancelled())
        } else if Instant::now() >= actual.deadline {
            Err("findFiles deadline expired".into())
        } else {
            actual
                .receiver
                .try_recv()
                .unwrap_or_else(|_| Err("findFiles worker exited without result".into()))
        };
        Some((actual.token, result))
    }
    pub(crate) fn settle(&mut self) {
        if let Some(actual) = self.active.take() {
            actual.cancel.store(true, Ordering::Release);
            let _ = actual.thread.join();
        }
    }
    #[cfg(test)]
    pub(crate) fn with_gates(gates: Arc<Gates>) -> Self {
        Self {
            next: 0,
            active: None,
            gates: Some(gates),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.settle();
    }
}

struct Rule {
    matcher: GlobMatcher,
    sibling: Option<String>,
}
fn value<'a>(context: &'a Context, key: &str) -> Option<&'a Value> {
    context
        .settings
        .iter()
        .rev()
        .find_map(|layer| layer.get(key))
}
fn boolean(context: &Context, key: &str, default: bool) -> Result<bool> {
    value(context, key).map_or(Ok(default), |value| {
        value.as_bool().context("Invalid findFiles boolean setting")
    })
}
fn rules(capture: &Capture) -> Result<Vec<Rule>> {
    let mut result = Vec::new();
    if capture.default_excludes {
        let mut effective: BTreeMap<String, Value> = DEFAULT_EXCLUDES
            .iter()
            .map(|key| ((*key).into(), Value::Bool(true)))
            .collect();
        for layer in capture.context.settings.iter() {
            if let Some(table) = layer.get("files.exclude") {
                for (pattern, value) in table.as_object().context("Invalid files.exclude object")? {
                    effective.insert(pattern.clone(), value.clone());
                    ensure!(
                        effective.len() <= MAX_PATTERNS,
                        "findFiles effective exclude budget exceeded"
                    );
                }
            }
        }
        for (pattern, value) in effective {
            if value == Value::Bool(false) {
                continue;
            }
            let sibling = if value == Value::Bool(true) {
                None
            } else {
                Some(
                    value["when"]
                        .as_str()
                        .context("Invalid conditional file exclude")?
                        .to_owned(),
                )
            };
            result.push(Rule {
                matcher: compile(&pattern)?,
                sibling,
            });
        }
    }
    if let Some(pattern) = &capture.exclude {
        result.push(Rule {
            matcher: compile(pattern)?,
            sibling: None,
        });
    }
    Ok(result)
}
fn excluded(path: &Path, relative: &Path, rules: &[Rule]) -> Result<bool> {
    for rule in rules {
        if !rule.matcher.is_match(relative) {
            continue;
        }
        let Some(sibling) = &rule.sibling else {
            return Ok(true);
        };
        let Some(stem) = path.file_stem().and_then(|part| part.to_str()) else {
            continue;
        };
        let name = sibling.replacen("$(basename)", stem, 1);
        ensure!(
            name.len() <= MAX_PATH,
            "Conditional exclude sibling exceeds path budget"
        );
        if let Some(parent) = path.parent()
            && parent
                .join(name)
                .try_exists()
                .context("Conditional exclude sibling lookup failed")?
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn guard(capture: &Capture, cancel: &AtomicBool, retired: &AtomicBool) -> Result<bool> {
    ensure!(!retired.load(Ordering::Acquire), "findFiles caller retired");
    ensure!(
        Instant::now() < capture.deadline,
        "findFiles deadline expired"
    );
    Ok(cancel.load(Ordering::Acquire))
}
struct IgnoreBudget {
    bytes: usize,
    lines: usize,
}
fn local_ignores(directory: &Path, budget: &mut IgnoreBudget) -> Result<Option<Gitignore>> {
    let mut builder = GitignoreBuilder::new(directory);
    let mut loaded = false;
    // Same native ignore parser as workspace/search, with bounded file reads.
    // .ignore has precedence over .gitignore; local .gitignore overrides
    // the repository's info/exclude entries.
    for relative in [".git/info/exclude", ".gitignore", ".ignore"] {
        let path = directory.join(relative);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("findFiles ignore metadata failed"),
        };
        ensure!(
            metadata.is_file(),
            "findFiles ignore file must be an ordinary file"
        );
        ensure!(
            metadata.len() <= 64 * 1024,
            "findFiles ignore file exceeds 64 KiB"
        );
        let file = std::fs::File::open(&path)?;
        ensure!(
            file.metadata()?.is_file(),
            "findFiles ignore file changed type"
        );
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 64 * 1024,
            "findFiles ignore file grew beyond 64 KiB"
        );
        budget.bytes = budget
            .bytes
            .checked_add(bytes.len())
            .context("Ignore byte overflow")?;
        ensure!(
            budget.bytes <= 1024 * 1024,
            "findFiles cumulative ignore input exceeds 1 MiB"
        );
        let text = String::from_utf8(bytes).context("findFiles ignore file is not UTF-8")?;
        for line in text.lines() {
            budget.lines += 1;
            ensure!(
                budget.lines <= 8192 && line.len() <= MAX_PATTERN,
                "findFiles ignore pattern budget exceeded"
            );
            builder.add_line(Some(path.clone()), line)?;
        }
        loaded = true;
    }
    if loaded {
        Ok(Some(builder.build()?))
    } else {
        Ok(None)
    }
}
struct Directory {
    canonical: PathBuf,
    entries: std::fs::ReadDir,
    ignores: Option<Gitignore>,
}
fn directory(path: PathBuf, local: bool, budget: &mut IgnoreBudget) -> Result<Directory> {
    check_path(&path)?;
    let canonical = std::fs::canonicalize(&path).context("findFiles directory unavailable")?;
    check_path(&canonical)?;
    let entries = std::fs::read_dir(&path).context("findFiles directory cannot be read")?;
    let ignores = if local {
        local_ignores(&path, budget)?
    } else {
        None
    };
    Ok(Directory {
        canonical,
        entries,
        ignores,
    })
}
fn is_ignored(path: &Path, is_directory: bool, stack: &[Directory], parents: &[Gitignore]) -> bool {
    // A descendant whitelist can override a parent rule, but a pruned ignored
    // parent is never opened. This preserves the native parser's negations.
    for matcher in stack
        .iter()
        .rev()
        .filter_map(|item| item.ignores.as_ref())
        .chain(parents.iter().rev())
    {
        let matched = matcher.matched(path, is_directory);
        if matched.is_whitelist() {
            return false;
        }
        if matched.is_ignore() {
            return true;
        }
    }
    false
}
fn walk(capture: &Capture, cancel: &AtomicBool, retired: &AtomicBool) -> Result<Output> {
    if guard(capture, cancel, retired)? {
        return Ok(Output::cancelled());
    }
    if capture.max_results == Some(0) {
        return Ok(Output {
            uris: Vec::new(),
            cancelled: false,
        });
    }
    let include = capture
        .include
        .as_deref()
        .filter(|text| !text.is_empty())
        .map(compile)
        .transpose()?;
    let exclusions = rules(capture)?;
    let local = capture.default_excludes
        && boolean(
            &capture.context,
            "search.experimental.useIgnoreFilesInFindFiles",
            false,
        )?
        && boolean(&capture.context, "search.useIgnoreFiles", true)?;
    // Git global excludes require Git configuration/provider behavior beyond
    // this native parser. Refuse an opted-in unsupported policy explicitly,
    // rather than silently report a complete result with different exclusions.
    ensure!(
        !local || !boolean(&capture.context, "search.useGlobalIgnoreFiles", false)?,
        "findFiles global ignore configuration is not supported by the native backend"
    );
    let use_parents = local && boolean(&capture.context, "search.useParentIgnoreFiles", false)?;
    let follow = boolean(&capture.context, "search.followSymlinks", true)?;
    let mut ignore_budget = IgnoreBudget { bytes: 0, lines: 0 };
    let mut parent_ignores = Vec::new();
    if use_parents {
        let mut paths = capture
            .base
            .ancestors()
            .skip(1)
            .take(65)
            .collect::<Vec<_>>();
        ensure!(
            paths.len() <= 64,
            "findFiles ignore ancestor budget exceeded"
        );
        paths.reverse();
        for path in paths {
            if guard(capture, cancel, retired)? {
                return Ok(Output::cancelled());
            }
            if let Some(matcher) = local_ignores(path, &mut ignore_budget)? {
                parent_ignores.push(matcher);
            }
        }
    }
    let mut stack = vec![directory(capture.base.clone(), local, &mut ignore_budget)?];
    // Streaming DFS retains at most 65 ReadDir handles and no materialized
    // directory listing. Every enumerated entry, including an excluded one,
    // reaches our cancellation/deadline and entry/path budget checks.
    let mut output = Vec::new();
    let (mut count, mut paths, mut encoded) = (0usize, 0usize, 64usize);
    while !stack.is_empty() {
        if guard(capture, cancel, retired)? {
            return Ok(Output::cancelled());
        }
        let Some(entry) = stack.last_mut().unwrap().entries.next() else {
            stack.pop();
            continue;
        };
        let entry = entry.context("findFiles traversal failed")?;
        count += 1;
        ensure!(
            count <= MAX_ENTRIES,
            "findFiles traversal exceeds 100000 entries"
        );
        let path = entry.path();
        ensure!(
            path.as_os_str().len() <= MAX_PATH,
            "findFiles encountered oversized path"
        );
        paths = paths
            .checked_add(path.as_os_str().len())
            .context("findFiles path bytes overflow")?;
        ensure!(
            paths <= MAX_VISITED_PATH_BYTES,
            "findFiles traversal path budget exceeded"
        );
        let file_type = entry.file_type()?;
        if file_type.is_symlink() && !follow {
            continue;
        }
        // DirEntry::metadata does not follow symlinks on every platform. Use
        // the explicit following metadata call only for opted-in links.
        let metadata = if file_type.is_symlink() {
            Some(std::fs::metadata(&path)?)
        } else {
            None
        };
        let is_directory = metadata.as_ref().map_or(file_type.is_dir(), |m| m.is_dir());
        let is_file = metadata
            .as_ref()
            .map_or(file_type.is_file(), |m| m.is_file());
        let relative = path.strip_prefix(&capture.base)?;
        if excluded(&path, relative, &exclusions)?
            || is_ignored(&path, is_directory, &stack, &parent_ignores)
        {
            continue;
        }
        if is_directory {
            ensure!(
                stack.len() < 65,
                "findFiles traversal exceeds 64 directory levels"
            );
            let next = directory(path, local, &mut ignore_budget)?;
            ensure!(
                !stack.iter().any(|item| item.canonical == next.canonical),
                "findFiles symlink directory cycle"
            );
            stack
                .try_reserve(1)
                .context("findFiles directory reservation refused")?;
            stack.push(next);
            continue;
        }
        if !is_file
            || include
                .as_ref()
                .is_some_and(|matcher| !matcher.is_match(relative))
        {
            continue;
        }
        if capture.max_results.is_none() && output.len() == MAX_RESULTS {
            anyhow::bail!("findFiles result exceeds native 4096-result budget");
        }
        let uri = crate::lsp::file_uri(&path)?;
        encoded = encoded
            .checked_add(serde_json::to_string(&uri)?.len() + 1)
            .context("findFiles output bytes overflow")?;
        ensure!(
            encoded <= MAX_OUTPUT,
            "findFiles result exceeds 2 MiB encoded output budget"
        );
        output
            .try_reserve(1)
            .context("findFiles result reservation refused")?;
        output.push(uri);
        if capture.max_results == Some(output.len()) {
            break;
        }
    }
    if guard(capture, cancel, retired)? {
        return Ok(Output::cancelled());
    }
    output.sort();
    output.dedup();
    Ok(Output {
        uris: output,
        cancelled: false,
    })
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GatePoint {
    BeforeWalk,
    AfterReply,
}
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Gates {
    state: std::sync::Mutex<[(bool, bool); 2]>,
    wake: std::sync::Condvar,
}
#[cfg(test)]
impl Gates {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn index(point: GatePoint) -> usize {
        match point {
            GatePoint::BeforeWalk => 0,
            GatePoint::AfterReply => 1,
        }
    }
    pub(crate) fn reached(&self, point: GatePoint) -> bool {
        self.state.lock().unwrap()[Self::index(point)].0
    }
    pub(crate) fn release(&self, point: GatePoint) {
        self.state.lock().unwrap()[Self::index(point)].1 = true;
        self.wake.notify_all();
    }
    fn wait(&self, point: GatePoint) {
        let index = Self::index(point);
        let mut state = self.state.lock().unwrap();
        state[index].0 = true;
        self.wake.notify_all();
        let end = Instant::now() + Duration::from_secs(8);
        while !state[index].1 {
            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            state = self.wake.wait_timeout(state, remaining).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn capture(
        root: &Path,
        layers: Vec<Map<String, Value>>,
        mode: &str,
        pattern: Option<&str>,
        max: Option<usize>,
    ) -> Arc<Capture> {
        Capture::new(Context::new(root.to_owned(), Arc::new(layers), 1).unwrap(),
            &json!({"include":{"pattern":pattern,"baseUri":null},
                "exclude":{"mode":mode,"pattern":if mode == "pattern" { Some("**/*.tmp") } else { None }},
                "maxResults":max}), Instant::now()).unwrap()
    }
    fn files(root: &Path, names: &[&str]) {
        for name in names {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"unchanged\n").unwrap();
        }
    }
    fn run(capture: Arc<Capture>) -> Vec<String> {
        walk(&capture, &AtomicBool::new(false), &AtomicBool::new(false))
            .unwrap()
            .uris
    }
    fn names(root: &Path, uris: Vec<String>) -> Vec<String> {
        uris.into_iter()
            .map(|uri| {
                url::Url::parse(&uri)
                    .unwrap()
                    .to_file_path()
                    .unwrap()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }
    fn until(mut check: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(8);
        while !check() {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn default_null_explicit_exclusion_and_false_layer_override_are_distinct() {
        let dir = tempfile::tempdir().unwrap();
        files(
            dir.path(),
            &[
                "a.rs",
                ".git/fixture",
                "node_modules/package.json",
                "target/build.rs",
                "src/猫.rs",
                "src/a.tmp",
            ],
        );
        let layer = json!({"files.exclude":{"**/*.tmp":true, "**/.git":false}, "search.exclude":{"**/*.rs":true}})
            .as_object().unwrap().clone();
        let found = names(
            dir.path(),
            run(capture(
                dir.path(),
                vec![layer.clone()],
                "default",
                None,
                None,
            )),
        );
        assert!(found.contains(&".git/fixture".into()));
        assert!(found.contains(&"node_modules/package.json".into()));
        assert!(found.contains(&"target/build.rs".into()));
        assert!(found.contains(&"src/猫.rs".into()));
        assert!(!found.contains(&"src/a.tmp".into()));
        let all = names(
            dir.path(),
            run(capture(dir.path(), vec![layer], "none", None, None)),
        );
        assert!(all.contains(&"src/a.tmp".into()));
        let explicit = names(
            dir.path(),
            run(capture(dir.path(), Vec::new(), "pattern", None, None)),
        );
        assert!(!explicit.contains(&".git/fixture".into()));
        assert!(!explicit.contains(&"src/a.tmp".into()));
        assert_eq!(
            std::fs::read(dir.path().join("src/猫.rs")).unwrap(),
            b"unchanged\n"
        );
    }
    #[test]
    fn conditional_sibling_excludes_and_brace_class_include() {
        let dir = tempfile::tempdir().unwrap();
        files(
            dir.path(),
            &["src/a.js", "src/a.ts", "src/b.js", "src/c.txt"],
        );
        let layer = json!({"files.exclude":{"**/*.js":{"when":"$(basename).ts"}}})
            .as_object()
            .unwrap()
            .clone();
        let found = names(
            dir.path(),
            run(capture(
                dir.path(),
                vec![layer],
                "default",
                Some("**/*.{[ab]js,js,ts}"),
                None,
            )),
        );
        assert_eq!(found, ["src/a.ts", "src/b.js"]);
    }
    #[test]
    fn legacy_ignore_opt_in_and_null_escape_hatch() {
        let dir = tempfile::tempdir().unwrap();
        files(dir.path(), &["ignored/a.rs", "kept.rs"]);
        std::fs::write(dir.path().join(".gitignore"), "ignored/\n").unwrap();
        assert!(
            names(
                dir.path(),
                run(capture(
                    dir.path(),
                    Vec::new(),
                    "default",
                    Some("**/*.rs"),
                    None
                ))
            )
            .contains(&"ignored/a.rs".into())
        );
        let layer = json!({"search.experimental.useIgnoreFilesInFindFiles":true})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            names(
                dir.path(),
                run(capture(
                    dir.path(),
                    vec![layer.clone()],
                    "default",
                    Some("**/*.rs"),
                    None
                ))
            ),
            ["kept.rs"]
        );
        assert!(
            names(
                dir.path(),
                run(capture(
                    dir.path(),
                    vec![layer],
                    "none",
                    Some("**/*.rs"),
                    None
                ))
            )
            .contains(&"ignored/a.rs".into())
        );
    }
    #[test]
    fn outside_relative_base_is_owned_and_context_never_retargets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        files(outside.path(), &["猫.txt"]);
        let settings = Arc::new(Vec::new());
        let context = Context::new(root.path().to_owned(), settings.clone(), 4).unwrap();
        assert!(context.current(root.path(), &settings, 4));
        assert!(!context.current(root.path(), &Arc::new(Vec::new()), 4));
        assert!(!context.current(root.path(), &settings, 5));
        let capture = Capture::new(context, &json!({
            "include":{"pattern":"**/*.txt","baseUri":crate::lsp::file_uri(outside.path()).unwrap()},
            "exclude":{"mode":"none","pattern":null},"maxResults":null}), Instant::now()).unwrap();
        assert!(capture.current(root.path(), &settings, 4));
        assert_eq!(names(outside.path(), run(capture)), ["猫.txt"]);
    }
    #[test]
    fn strict_admission_and_default_overflow_never_report_partial_success() {
        let dir = tempfile::tempdir().unwrap();
        files(dir.path(), &["a.rs", "b.rs"]);
        assert!(run(capture(dir.path(), Vec::new(), "none", None, Some(0))).is_empty());
        assert_eq!(
            run(capture(dir.path(), Vec::new(), "none", None, Some(1))).len(),
            1
        );
        for pattern in ["../*", "{{a,b},c}", "/absolute", "a\\b"] {
            assert!(check_pattern(pattern).is_err());
        }
        let layer = json!({"files.exclude":{"**/*.js":{"when":"../secret"}}})
            .as_object()
            .unwrap()
            .clone();
        let context = Context::new(dir.path().to_owned(), Arc::new(vec![layer]), 1).unwrap();
        assert!(
            Capture::new(
                context,
                &json!({"include":{"pattern":null,"baseUri":null},
            "exclude":{"mode":"default","pattern":null},"maxResults":null}),
                Instant::now()
            )
            .is_err()
        );
        // 4097 independent files prove undefined upper bound refuses the full
        // request; no partial result is returned as a completed search.
        for n in 0..4095 {
            files(dir.path(), &[&format!("file-{n}")]);
        }
        let captured = capture(dir.path(), Vec::new(), "none", None, None);
        assert!(
            walk(&captured, &AtomicBool::new(false), &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("4096")
        );
        assert_eq!(
            run(capture(dir.path(), Vec::new(), "none", None, Some(4096))).len(),
            4096
        );
    }
    #[test]
    fn null_exclude_bypasses_unused_malformed_defaults_and_context_identity_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        files(dir.path(), &["a"]);
        let settings = Arc::new(vec![
            json!({"files.exclude":"malformed"})
                .as_object()
                .unwrap()
                .clone(),
        ]);
        let context = Context::new(dir.path().to_owned(), settings.clone(), 17).unwrap();
        let args = json!({"include":{"pattern":null,"baseUri":null},
            "exclude":{"mode":"none","pattern":null},"maxResults":null});
        let captured = Capture::new(context.clone(), &args, Instant::now()).unwrap();
        assert!(captured.current(dir.path(), &settings, 17));
        assert!(!captured.current(dir.path(), &settings, 18));
        assert_eq!(names(dir.path(), run(captured)), ["a"]);
        let mut defaults = args;
        defaults["exclude"]["mode"] = json!("default");
        assert!(Capture::new(context, &defaults, Instant::now()).is_err());
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"unchanged\n");
    }
    #[test]
    fn real_reply_before_exit_exact_cancel_keeps_capacity_until_join() {
        let dir = tempfile::tempdir().unwrap();
        files(dir.path(), &["original.txt"]);
        let captured = capture(dir.path(), Vec::new(), "none", None, None);
        let gates = Gates::new();
        gates.release(GatePoint::BeforeWalk);
        let mut worker = Worker::with_gates(gates.clone());
        let retired = Arc::new(AtomicBool::new(false));
        let token = worker.start(captured.clone(), retired.clone()).unwrap();
        until(|| gates.reached(GatePoint::AfterReply));
        worker.cancel(token + 1);
        assert!(worker.occupied());
        assert!(worker.poll().is_none());
        assert!(worker.start(captured.clone(), retired.clone()).is_err());
        worker.cancel(token);
        assert!(worker.poll().is_none());
        gates.release(GatePoint::AfterReply);
        let mut result = None;
        until(|| {
            result = worker.poll();
            result.is_some()
        });
        let (actual_token, result) = result.unwrap();
        assert_eq!(actual_token, token);
        let result = result.unwrap();
        assert!(result.cancelled && result.uris.is_empty());
        assert!(!worker.occupied());
        let next = worker.start(captured, retired).unwrap();
        until(|| worker.poll().is_some());
        assert_eq!(next, token + 1);
        assert_eq!(
            std::fs::read(dir.path().join("original.txt")).unwrap(),
            b"unchanged\n"
        );
    }
    #[test]
    fn owner_aba_and_deadline_after_reply_refuse_original_result() {
        let dir = tempfile::tempdir().unwrap();
        files(dir.path(), &["a"]);
        let gates = Gates::new();
        gates.release(GatePoint::BeforeWalk);
        let mut worker = Worker::with_gates(gates.clone());
        let original = Arc::new(AtomicBool::new(false));
        worker
            .start(
                capture(dir.path(), Vec::new(), "none", None, None),
                original.clone(),
            )
            .unwrap();
        until(|| gates.reached(GatePoint::AfterReply));
        original.store(true, Ordering::Release);
        let replacement = Arc::new(AtomicBool::new(false));
        gates.release(GatePoint::AfterReply);
        let mut result = None;
        until(|| {
            result = worker.poll();
            result.is_some()
        });
        assert!(result.unwrap().1.unwrap_err().contains("retired"));
        worker
            .start(
                capture(dir.path(), Vec::new(), "none", None, None),
                replacement,
            )
            .unwrap();
        until(|| worker.poll().is_some());
        // Age the exact actual operation clock, with a real peer reply already
        // held, rather than changing production timeout or forging a result.
        let gates = Gates::new();
        gates.release(GatePoint::BeforeWalk);
        worker.gates = Some(gates.clone());
        worker
            .start(
                capture(dir.path(), Vec::new(), "none", None, None),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        until(|| gates.reached(GatePoint::AfterReply));
        worker.active.as_mut().unwrap().deadline = Instant::now() - Duration::from_millis(1);
        gates.release(GatePoint::AfterReply);
        let mut result = None;
        until(|| {
            result = worker.poll();
            result.is_some()
        });
        assert!(result.unwrap().1.unwrap_err().contains("deadline"));
    }
    #[test]
    fn exhausted_token_and_shutdown_join_preserve_files() {
        let dir = tempfile::tempdir().unwrap();
        files(dir.path(), &["a"]);
        let mut worker = Worker {
            next: u64::MAX,
            active: None,
            gates: None,
        };
        assert!(
            worker
                .start(
                    capture(dir.path(), Vec::new(), "none", None, None),
                    Arc::new(AtomicBool::new(false))
                )
                .is_err()
        );
        assert_eq!(worker.next, u64::MAX);
        assert!(!worker.occupied());
        let gates = Gates::new();
        let release = gates.clone();
        let mut worker = Worker::with_gates(gates.clone());
        worker
            .start(
                capture(dir.path(), Vec::new(), "none", None, None),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        until(|| gates.reached(GatePoint::BeforeWalk));
        let thread = thread::spawn(move || release.release(GatePoint::BeforeWalk));
        gates.release(GatePoint::AfterReply);
        worker.settle();
        thread.join().unwrap();
        assert!(!worker.occupied());
        assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"unchanged\n");
    }
}
