//! Explicit Git operations in a worker. Paths are literal arguments, never shell code.
use anyhow::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub original: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
}
pub struct Status {
    pub root: PathBuf,
    pub branch: String,
    pub entries: Vec<Entry>,
}
pub enum Action {
    Status,
    Diff { path: PathBuf, staged: bool },
    Stage(PathBuf),
    Unstage(PathBuf),
    Log,
    Commit(String),
}
pub enum Output {
    Status(Status),
    Text { title: String, text: String },
    Changed(String),
}
pub struct Job {
    receiver: Receiver<std::result::Result<Output, String>>,
    cancel: Arc<AtomicBool>,
}
impl Job {
    pub fn start(root: PathBuf, action: Action) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        std::thread::spawn(move || {
            let result = execute(&root, action, &cancelled).map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        Self { receiver, cancel }
    }
    pub fn poll(&mut self) -> Option<std::result::Result<Output, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Git worker disconnected".into())),
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn os_path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(std::str::from_utf8(bytes)?))
    }
}
pub fn parse_status(root: PathBuf, bytes: &[u8]) -> Result<Status> {
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut branch = String::new();
    let mut entries = Vec::new();
    while let Some(record) = records.next() {
        if let Some(name) = record.strip_prefix(b"## ") {
            branch = String::from_utf8_lossy(name).into_owned();
            continue;
        }
        if record.len() < 4 || record[2] != b' ' {
            bail!("Malformed Git status record");
        }
        let path = os_path(&record[3..])?;
        let original = if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C') {
            Some(os_path(records.next().context("Missing rename source")?)?)
        } else {
            None
        };
        entries.push(Entry {
            path,
            original,
            index: record[0] as char,
            worktree: record[1] as char,
        });
    }
    Ok(Status {
        root,
        branch,
        entries,
    })
}
fn run(
    root: &Path,
    args: &[OsString],
    cancel: &AtomicBool,
    allow_diff_exit: bool,
) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .args([
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot launch git")?;
    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    for (is_error, pipe) in [
        (false, Box::new(stdout) as Box<dyn Read + Send>),
        (true, Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        let sender = sender.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send((is_error, result));
        });
    }
    drop(sender);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Git operation canceled or timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    for _ in 0..2 {
        let (is_error, bytes) = receiver
            .recv_timeout(Duration::from_secs(1))
            .context("Git output stream did not close")?;
        let bytes = bytes?;
        if bytes.len() > 8 * 1024 * 1024 {
            bail!("Git output exceeds 8 MiB; narrow the operation");
        }
        if is_error {
            stderr = bytes;
        } else {
            stdout = bytes;
        }
    }
    if !status.success() && !(allow_diff_exit && status.code() == Some(1)) {
        bail!("{}", String::from_utf8_lossy(&stderr));
    }
    Ok(stdout)
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn execute(workspace: &Path, action: Action, cancel: &AtomicBool) -> Result<Output> {
    let root = run(
        workspace,
        &args(&["rev-parse", "--show-toplevel"]),
        cancel,
        false,
    )?;
    let root = os_path(root.strip_suffix(b"\n").unwrap_or(&root))?;
    match action {
        Action::Status => {
            let bytes = run(
                &root,
                &args(&[
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--branch",
                    "--untracked-files=all",
                ]),
                cancel,
                false,
            )?;
            Ok(Output::Status(parse_status(root, &bytes)?))
        }
        Action::Diff { path, staged } => {
            let mut command = args(&["diff", "--no-ext-diff", "--no-textconv"]);
            if staged {
                command.push("--cached".into());
            }
            command.push("--".into());
            command.push(path.as_os_str().to_owned());
            let bytes = run(&root, &command, cancel, false)?;
            let text = if bytes.is_empty() {
                "No tracked diff for this selection. Untracked files can be opened from Source Control.".into()
            } else {
                String::from_utf8_lossy(&bytes).into_owned()
            };
            Ok(Output::Text {
                title: format!(
                    " {} diff: {} · Esc closes ",
                    if staged { "Staged" } else { "Working tree" },
                    path.display()
                ),
                text,
            })
        }
        Action::Stage(path) => {
            run(
                &root,
                &[OsString::from("add"), "--".into(), path.into_os_string()],
                cancel,
                false,
            )?;
            Ok(Output::Changed("Staged saved file contents".into()))
        }
        Action::Unstage(path) => {
            let has_head = run(
                &root,
                &args(&["rev-parse", "--verify", "HEAD"]),
                cancel,
                false,
            )
            .is_ok();
            let mut command = if has_head {
                args(&["reset", "-q", "HEAD", "--"])
            } else {
                args(&["rm", "--cached", "--ignore-unmatch", "--"])
            };
            command.push(path.into_os_string());
            run(&root, &command, cancel, false)?;
            Ok(Output::Changed(
                "Unstaged file; working tree retained".into(),
            ))
        }
        Action::Log => {
            let bytes = run(
                &root,
                &args(&["log", "-100", "--date=short", "--format=%h %ad %s (%an)"]),
                cancel,
                false,
            )?;
            Ok(Output::Text {
                title: " Git History · newest 100 · Esc closes ".into(),
                text: String::from_utf8_lossy(&bytes).into_owned(),
            })
        }
        Action::Commit(message) => {
            if message.trim().is_empty() {
                bail!("Commit message cannot be empty");
            }
            let bytes = run(
                &root,
                &[
                    OsString::from("commit"),
                    OsStr::new("-m").to_owned(),
                    message.into(),
                ],
                cancel,
                false,
            )?;
            Ok(Output::Changed(
                String::from_utf8_lossy(&bytes).into_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn porcelain_renames_and_unusual_names_are_not_split_on_whitespace() {
        let status = parse_status(
            PathBuf::from("/tmp/repo"),
            b"## main\0R  new name\0old\nname\0?? :(glob)*.rs\0",
        )
        .unwrap();
        assert_eq!(status.entries.len(), 2);
        assert_eq!(status.entries[0].original, Some(PathBuf::from("old\nname")));
        assert_eq!(status.entries[1].path, PathBuf::from(":(glob)*.rs"));
    }
    #[test]
    fn git_stage_unstage_diff_and_commit_preserve_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cancel = AtomicBool::new(false);
        run(root, &args(&["init", "-q"]), &cancel, false).unwrap();
        run(
            root,
            &args(&["config", "user.name", "VSCLI Tests"]),
            &cancel,
            false,
        )
        .unwrap();
        run(
            root,
            &args(&["config", "user.email", "tests@example.invalid"]),
            &cancel,
            false,
        )
        .unwrap();
        run(
            root,
            &args(&["config", "commit.gpgsign", "false"]),
            &cancel,
            false,
        )
        .unwrap();
        let name = PathBuf::from("space name.txt");
        std::fs::write(root.join(&name), "first\n").unwrap();
        execute(root, Action::Stage(name.clone()), &cancel).unwrap();
        execute(root, Action::Unstage(name.clone()), &cancel).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(&name)).unwrap(),
            "first\n"
        );
        execute(root, Action::Stage(name.clone()), &cancel).unwrap();
        execute(root, Action::Commit("fixture".into()), &cancel).unwrap();
        std::fs::write(root.join(&name), "second\n").unwrap();
        let Output::Text { text, .. } = execute(
            root,
            Action::Diff {
                path: name.clone(),
                staged: false,
            },
            &cancel,
        )
        .unwrap() else {
            panic!()
        };
        assert!(text.contains("+second"));
        execute(root, Action::Stage(name.clone()), &cancel).unwrap();
        execute(root, Action::Unstage(name.clone()), &cancel).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(name)).unwrap(),
            "second\n"
        );
    }
}
