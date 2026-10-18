//! Bounded optional clipboard work. A canceled request never releases an
//! occupied worker merely because its native receipt has retired.
use anyhow::{Result, bail};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};
#[cfg(windows)]
#[path = "snippet_clipboard/windows.rs"]
mod windows;

#[cfg(any(not(windows), test))]
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) const MAX_BYTES: usize = 1024 * 1024;
static BUSY: OnceLock<Arc<AtomicBool>> = OnceLock::new();
struct Slot(Arc<AtomicBool>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
pub(crate) struct Worker {
    receiver: Receiver<Result<Option<String>, String>>,
    thread: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    _slot: Arc<Slot>,
}
impl Worker {
    pub(crate) fn start() -> Result<Self> {
        Self::start_with(
            BUSY.get_or_init(|| Arc::new(AtomicBool::new(false)))
                .clone(),
            |sender, cancel, slot| {
                let _ = sender.send(read_platform(&cancel, &slot));
            },
        )
    }
    pub(crate) fn occupied() -> bool {
        BUSY.get().is_some_and(|busy| busy.load(Ordering::Acquire))
    }
    pub(crate) fn start_write(text: Arc<str>) -> Result<Self> {
        if text.len() > MAX_BYTES {
            bail!("Clipboard text exceeds 1 MiB");
        }
        Self::start_with(
            BUSY.get_or_init(|| Arc::new(AtomicBool::new(false)))
                .clone(),
            move |sender, cancel, slot| {
                let _ = sender.send(write_platform(&cancel, &slot, &text));
            },
        )
    }
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    #[cfg(test)]
    pub(crate) fn start_commands(
        busy: Arc<AtomicBool>,
        commands: Vec<(String, Vec<String>)>,
        input: Option<Arc<str>>,
    ) -> Result<Self> {
        Self::start_with(busy, move |sender, cancel, slot| {
            let arguments: Vec<_> = commands
                .iter()
                .map(|(_, args)| args.iter().map(String::as_str).collect::<Vec<_>>())
                .collect();
            let candidates: Vec<_> = commands
                .iter()
                .zip(&arguments)
                .map(|((name, _), args)| (name.as_str(), args.as_slice()))
                .collect();
            let _ = sender.send(transfer_commands(
                &candidates,
                &cancel,
                &slot,
                input.as_deref(),
            ));
        })
    }
    fn start_with(
        busy: Arc<AtomicBool>,
        read: impl FnOnce(mpsc::SyncSender<Result<Option<String>, String>>, Arc<AtomicBool>, Arc<Slot>)
        + Send
        + 'static,
    ) -> Result<Self> {
        if busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            bail!("A snippet clipboard worker is still occupied");
        }
        let slot = Arc::new(Slot(busy));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(1);
        let flag = cancel.clone();
        let worker_slot = slot.clone();
        let handle = thread::Builder::new()
            .name("snippet-clipboard".into())
            .spawn(move || read(sender, flag, worker_slot))?;
        Ok(Self {
            receiver,
            thread: Some(handle),
            cancel,
            _slot: slot,
        })
    }
    pub(crate) fn try_finish(&mut self) -> Option<Result<Option<String>>> {
        if !self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            return None;
        }
        let handle = self.thread.take().expect("finished clipboard worker");
        if handle.join().is_err() {
            return Some(Err(anyhow::anyhow!("Snippet clipboard worker failed")));
        }
        Some(
            self.receiver
                .try_recv()
                .map_err(|_| anyhow::anyhow!("Snippet clipboard worker did not publish a result"))
                .and_then(|result| result.map_err(anyhow::Error::msg)),
        )
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
fn read_platform(cancel: &AtomicBool, slot: &Arc<Slot>) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let _ = slot;
        windows::read(cancel)
            .map(Some)
            .map_err(|error| format!("Windows clipboard: {error:#}"))
    }
    #[cfg(not(windows))]
    {
        let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
            &[("pbpaste", &[])]
        } else {
            &[
                ("wl-paste", &["--no-newline"]),
                ("xclip", &["-selection", "clipboard", "-out"]),
            ]
        };
        read_commands(candidates, cancel, slot)
    }
}
fn write_platform(
    cancel: &AtomicBool,
    slot: &Arc<Slot>,
    text: &str,
) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let _ = slot;
        windows::write(text, cancel)
            .map(|()| Some(String::new()))
            .map_err(|error| format!("Windows clipboard: {error:#}"))
    }
    #[cfg(not(windows))]
    {
        let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
            &[("pbcopy", &[])]
        } else {
            &[
                ("wl-copy", &[]),
                ("xclip", &["-selection", "clipboard", "-in"]),
            ]
        };
        transfer_commands(candidates, cancel, slot, Some(text))
    }
}
#[cfg(not(windows))]
fn read_commands(
    candidates: &[(&str, &[&str])],
    cancel: &AtomicBool,
    slot: &Arc<Slot>,
) -> Result<Option<String>, String> {
    transfer_commands(candidates, cancel, slot, None)
}
#[cfg(any(not(windows), test))]
fn transfer_commands(
    candidates: &[(&str, &[&str])],
    cancel: &AtomicBool,
    slot: &Arc<Slot>,
    input: Option<&str>,
) -> Result<Option<String>, String> {
    if input.is_some_and(|text| text.len() > MAX_BYTES) {
        return Err("Clipboard text exceeds 1 MiB".into());
    }
    for (name, args) in candidates {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let mut child = match Command::new(name)
            .args(*args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(if input.is_none() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => continue,
        };
        let stdout = child.stdout.take();
        let stdin = child.stdin.take();
        let payload = input.map(str::to_owned);
        let (sender, receiver) = mpsc::sync_channel(1);
        let pipe_slot = slot.clone();
        let reader = thread::Builder::new()
            .name("snippet-clipboard-pipe".into())
            .spawn(move || {
                let _slot = pipe_slot;
                let result = if let Some(payload) = payload {
                    stdin
                        .expect("piped clipboard input")
                        .write_all(payload.as_bytes())
                        .map(|()| String::new())
                        .map_err(|error| error.to_string())
                } else {
                    let mut bytes = Vec::new();
                    stdout
                        .expect("piped clipboard output")
                        .take(MAX_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())
                        .and_then(|_| {
                            if bytes.len() > MAX_BYTES {
                                return Err("Snippet clipboard exceeds 1 MiB".into());
                            }
                            String::from_utf8(bytes)
                                .map_err(|_| "Snippet clipboard is not UTF-8".into())
                        })
                };
                let _ = sender.send(result);
            });
        let reader = match reader {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        };
        let started = Instant::now();
        let success = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None)
                    if !cancel.load(Ordering::Acquire)
                        && started.elapsed() < Duration::from_millis(300) =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        let mut output = receiver.recv_timeout(Duration::from_millis(50));
        while !reader.is_finished()
            && !cancel.load(Ordering::Acquire)
            && started.elapsed() < Duration::from_millis(350)
        {
            thread::sleep(Duration::from_millis(1));
        }
        if reader.is_finished() {
            let _ = reader.join();
            if output.is_err() {
                output = receiver
                    .try_recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected);
            }
        } else {
            // A descendant holding the pipe must retain the global permit too.
            // Refuse rather than spawning replacements around that live reader.
            return Err("Snippet clipboard pipe has not retired".into());
        }
        if let Ok(Err(error)) = &output {
            return Err(error.clone());
        }
        if success {
            return output
                .map(Some)
                .map_err(|_| "Snippet clipboard result missing".to_owned())
                .and_then(|result| result.transpose());
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_clipboard_snippets_refuse_occupied_and_canceled_public_workers() {
        let busy = BUSY
            .get_or_init(|| Arc::new(AtomicBool::new(false)))
            .clone();
        let (release, gate) = mpsc::sync_channel(1);
        let mut worker = Some(
            Worker::start_with(busy.clone(), move |_, _, _slot| {
                gate.recv().unwrap();
            })
            .unwrap(),
        );
        let directory = tempfile::tempdir().unwrap();
        let mut app = crate::app::App::new(directory.path().into(), crate::keys::Profile::Linux);
        app.execute(
            "workbench.action.files.newUntitledFile",
            serde_json::Value::Null,
        );
        app.doc_mut().insert("protected 猫🙂\r\n", false);
        let original = (
            app.doc().id,
            app.doc().text.to_string(),
            app.doc().revision,
            app.doc().text_epoch(),
            app.doc().save_generation(),
        );
        for canceled in [false, true] {
            if canceled {
                drop(worker.take());
            }
            app.execute(
                "editor.action.insertSnippet",
                serde_json::json!({"snippet":"$CLIPBOARD"}),
            );
            assert_eq!(app.message, "A snippet clipboard worker is still occupied");
            assert_eq!(
                (
                    app.doc().id,
                    app.doc().text.to_string(),
                    app.doc().revision,
                    app.doc().text_epoch(),
                    app.doc().save_generation()
                ),
                original
            );
        }
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        while busy.load(Ordering::Acquire) {
            assert!(Instant::now() < until);
            thread::yield_now();
        }
        app.doc_mut().undo();
        assert!(app.doc().text.to_string().is_empty());
        app.doc_mut().redo();
        assert_eq!(app.doc().text.to_string(), original.1);
    }
    #[test]
    fn reply_before_exit_and_cancellation_do_not_release_occupied_clipboard_capacity() {
        let busy = Arc::new(AtomicBool::new(false));
        let (release, gate) = mpsc::sync_channel(1);
        let (published, notice) = mpsc::sync_channel(1);
        let mut worker = Worker::start_with(busy.clone(), move |sender, cancel, _slot| {
            sender.send(Ok(Some("猫🙂\r\n".into()))).unwrap();
            published.send(cancel).unwrap();
            gate.recv().unwrap();
        })
        .unwrap();
        let flag = notice.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(worker.try_finish().is_none());
        assert!(Worker::start_with(busy.clone(), |_, _, _| {}).is_err());
        drop(worker);
        assert!(flag.load(Ordering::Acquire));
        assert!(Worker::start_with(busy.clone(), |_, _, _| {}).is_err());
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        while busy.load(Ordering::Acquire) {
            assert!(Instant::now() < until);
            thread::yield_now();
        }
        let mut fresh = Worker::start_with(busy, |sender, _, _| {
            sender.send(Ok(None)).unwrap();
        })
        .unwrap();
        while fresh.try_finish().is_none() {
            assert!(Instant::now() < until);
            thread::yield_now();
        }
    }
    #[cfg(unix)]
    #[test]
    fn actual_clipboard_commands_preserve_utf8_crlf_and_bound_output_and_deadlines() {
        let root = tempfile::tempdir().unwrap();
        let value = root.path().join("clipboard");
        std::fs::write(&value, "猫🙂\r\nsecond\r\n").unwrap();
        let run = |args: Vec<String>| {
            let mut worker = Worker::start_with(
                Arc::new(AtomicBool::new(false)),
                move |sender, cancel, slot| {
                    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
                    sender
                        .send(read_commands(&[("/bin/sh", &refs)], &cancel, &slot))
                        .unwrap();
                },
            )
            .unwrap();
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(result) = worker.try_finish() {
                    break result;
                }
                assert!(Instant::now() < until);
                thread::sleep(Duration::from_millis(1));
            }
        };
        assert_eq!(
            run(vec![
                "-c".into(),
                "cat \"$1\"".into(),
                "fixture".into(),
                value.display().to_string()
            ])
            .unwrap()
            .as_deref(),
            Some("猫🙂\r\nsecond\r\n")
        );
        // The helper leader can exit before its descendant publishes the pipe.
        // A pre-join receive timeout must not discard the joined reader's reply.
        assert_eq!(
            run(vec![
                "-c".into(),
                "(sleep 0.1; printf late) & exit 0".into()
            ])
            .unwrap()
            .as_deref(),
            Some("late")
        );
        std::fs::write(&value, vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(
            run(vec![
                "-c".into(),
                "cat \"$1\"".into(),
                "fixture".into(),
                value.display().to_string()
            ])
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
        );
        let started = Instant::now();
        assert_eq!(run(vec!["-c".into(), "exec sleep 2".into()]).unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[cfg(unix)]
    #[test]
    fn real_clipboard_write_transfers_exact_bytes_and_a_live_descendant_pipe_keeps_the_original_permit()
     {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("clipboard");
        let text = "猫🙂\r\nsecond\r\n";
        let busy = Arc::new(AtomicBool::new(false));
        let mut worker = Worker::start_commands(
            busy.clone(),
            vec![(
                "/bin/sh".into(),
                vec![
                    "-c".into(),
                    "cat > \"$1\"".into(),
                    "fixture".into(),
                    path.to_str().unwrap().into(),
                ],
            )],
            Some(Arc::from(text)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Some(result) = worker.try_finish() {
                break result;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(result.unwrap().as_deref(), Some(""));
        assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
        drop(worker);
        assert!(!busy.load(Ordering::Acquire));
        assert!(Worker::start_write(Arc::from("x".repeat(MAX_BYTES + 1))).is_err());
        let release = root.path().join("release");
        // The child inherits the actual pipe directly, independently of shell
        // background-stdin rules. It cannot drain it until the test releases it.
        // Removing the temporary root during unwinding also releases the child.
        let helper = "import os,pathlib,sys,time\nif os.fork(): os._exit(0)\nrelease,root=map(pathlib.Path,sys.argv[1:])\nwhile root.is_dir() and not release.exists(): time.sleep(0.01)\nsys.stdin.buffer.read()\nos._exit(0)\n";
        let mut worker = Worker::start_commands(
            busy.clone(),
            vec![(
                "python3".into(),
                vec![
                    "-c".into(),
                    helper.into(),
                    release.to_str().unwrap().into(),
                    root.path().to_str().unwrap().into(),
                ],
            )],
            Some(Arc::from("x".repeat(MAX_BYTES))),
        )
        .unwrap();
        let result = loop {
            if let Some(result) = worker.try_finish() {
                break result;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        };
        let error = result.unwrap_err().to_string();
        assert!(error.contains("pipe has not retired"), "{error}");
        drop(worker);
        assert!(busy.load(Ordering::Acquire));
        assert!(Worker::start_with(busy.clone(), |_, _, _| {}).is_err());
        std::fs::write(release, []).unwrap();
        while busy.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(all(test, windows))]
mod windows_ci {
    use super::*;
    #[test]
    #[ignore = "requires an explicitly isolated Windows CI clipboard; never runs on a personal desktop by default"]
    fn isolated_windows_clipboard_preserves_unicode_crlf_empty_text_and_canceled_write() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        assert_eq!(
            std::env::var("VSCLI_TEST_WINDOWS_CLIPBOARD").as_deref(),
            Ok("1")
        );
        fn finish(mut worker: Worker) -> Result<Option<String>> {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(result) = worker.try_finish() {
                    return result;
                }
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
        }
        let text = "VSCLI isolated CI clipboard 猫🙂\r\nsecond\r\n";
        assert_eq!(
            finish(Worker::start_write(Arc::from(text)).unwrap())
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert_eq!(
            finish(Worker::start().unwrap()).unwrap().as_deref(),
            Some(text)
        );
        // A null-window holder did not block reads in runner observations,
        // including across processes. Hold a real window in another process.
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let release = directory.path().join("release");
        struct Holder(std::process::Child, std::path::PathBuf);
        impl Drop for Holder {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.1, []);
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut held = Holder(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "snippet_clipboard::windows_ci::hold_for_parent",
                    "--ignored",
                    "--nocapture",
                ])
                .env("VSCLI_TEST_CLIPBOARD_READY", &ready)
                .env("VSCLI_TEST_CLIPBOARD_RELEASE", &release)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
            release.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                held.0.try_wait().unwrap().is_none(),
                "Clipboard holder exited before admission"
            );
            assert!(Instant::now() < deadline, "Clipboard holder did not start");
            thread::sleep(Duration::from_millis(1));
        }
        let error = finish(Worker::start().unwrap()).unwrap_err().to_string();
        assert!(error.contains("Cannot open Windows clipboard"), "{error}");
        std::fs::write(&release, []).unwrap();
        loop {
            if let Some(status) = held.0.try_wait().unwrap() {
                assert!(status.success(), "Clipboard holder failed: {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Clipboard holder did not release"
            );
            thread::sleep(Duration::from_millis(1));
        }
        drop(held);
        let canceled = AtomicBool::new(true);
        assert!(windows::write("forbidden", &canceled).is_err());
        assert_eq!(
            finish(Worker::start().unwrap()).unwrap().as_deref(),
            Some(text)
        );
        let boundary = "x".repeat(MAX_BYTES);
        assert_eq!(
            finish(Worker::start_write(Arc::from(boundary.as_str())).unwrap())
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert!(
            finish(Worker::start().unwrap()).unwrap().as_deref() == Some(boundary.as_str()),
            "Windows clipboard boundary string changed"
        );
        assert_eq!(
            finish(Worker::start_write(Arc::from("")).unwrap())
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert_eq!(
            finish(Worker::start().unwrap()).unwrap().as_deref(),
            Some("")
        );
    }

    #[test]
    #[ignore = "private isolated Windows clipboard holder; invoked only by the parent CI control"]
    fn hold_for_parent() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        assert_eq!(
            std::env::var("VSCLI_TEST_WINDOWS_CLIPBOARD").as_deref(),
            Ok("1")
        );
        let ready = std::path::PathBuf::from(
            std::env::var_os("VSCLI_TEST_CLIPBOARD_READY").expect("parent ready path"),
        );
        let release = std::path::PathBuf::from(
            std::env::var_os("VSCLI_TEST_CLIPBOARD_RELEASE").expect("parent release path"),
        );
        let _held = windows::hold_open_for_test().unwrap();
        std::fs::write(&ready, []).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !release.exists() && release.parent().unwrap().exists() {
            assert!(
                Instant::now() < deadline,
                "Clipboard parent did not release"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
}
