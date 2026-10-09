//! Shared bounded Content-Length framing for native LSP and DAP clients.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;
pub(crate) fn read_message(reader: &mut impl BufRead) -> Result<Value> {
    let mut header = Vec::new();
    // Read headers with a hard bound before allocating the body.
    loop {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
        if header.len() > 8192 {
            bail!("Protocol header exceeds 8 KiB");
        }
    }
    let mut length = None;
    for line in std::str::from_utf8(&header)?.split("\r\n") {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            if length.is_some() {
                bail!("Duplicate Protocol Content-Length");
            }
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.context("Protocol Content-Length missing")?;
    if length > MAX_MESSAGE {
        bail!("Protocol message exceeds 16 MiB");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

pub struct Process {
    child: Child,
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
    sender: SyncSender<Vec<u8>>,
    receiver: Receiver<std::result::Result<Value, String>>,
    queued_bytes: Arc<AtomicUsize>,
    stderr: Arc<Mutex<Vec<u8>>>,
}
impl Process {
    pub fn start(program: &str, args: &[String], root: &Path) -> Result<Self> {
        Self::start_with_isolation(program, args, root, false)
    }
    /// Extensions retire their Unix process group; ordinary LSP/DAP children keep their semantics.
    pub fn start_isolated(program: &str, args: &[String], root: &Path) -> Result<Self> {
        Self::start_with_isolation(program, args, root, true)
    }
    fn start_with_isolation(
        program: &str,
        args: &[String],
        root: &Path,
        isolated: bool,
    ) -> Result<Self> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        if isolated {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(not(unix))]
        let _ = isolated;
        let mut child = command
            .spawn()
            .with_context(|| format!("Cannot start {program}"))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (sender, outgoing) = mpsc::sync_channel::<Vec<u8>>(64);
        let (incoming, receiver) = mpsc::sync_channel(4);
        let errors = incoming.clone();
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let pending = queued_bytes.clone();
        std::thread::spawn(move || {
            for body in outgoing {
                let result = (|| -> Result<()> {
                    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
                    stdin.write_all(&body)?;
                    stdin.flush()?;
                    Ok(())
                })();
                pending.fetch_sub(body.len(), Ordering::Relaxed);
                if let Err(e) = result {
                    let _ = errors.send(Err(format!("Protocol write failed: {e}")));
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let message =
                    read_message(&mut reader).map_err(|e| format!("Protocol stream ended: {e}"));
                let stop = message.is_err();
                if incoming.send(message).is_err() || stop {
                    break;
                }
            }
        });
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        if let Ok(mut tail) = tail.lock() {
                            tail.extend_from_slice(&buffer[..size]);
                            let remove = tail.len().saturating_sub(8192);
                            tail.drain(..remove);
                        }
                    }
                }
            }
        });
        #[cfg(unix)]
        let process_group = isolated.then_some(child.id() as libc::pid_t);
        Ok(Self {
            child,
            #[cfg(unix)]
            process_group,
            sender,
            receiver,
            queued_bytes,
            stderr: stderr_tail,
        })
    }
    pub fn send(&self, message: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&message)?;
        let size = bytes.len();
        if size > MAX_MESSAGE {
            bail!("Protocol message exceeds 16 MiB output limit");
        }
        let mut current = self.queued_bytes.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(size)
                .filter(|n| *n <= MAX_QUEUED_BYTES)
                .context("Protocol output byte budget exhausted")?;
            match self.queued_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        if let Err(e) = self.sender.try_send(bytes) {
            self.queued_bytes.fetch_sub(size, Ordering::Relaxed);
            bail!("Protocol output unavailable: {e}");
        }
        Ok(())
    }
    pub fn receive(&self) -> Result<Option<Value>> {
        match self.receiver.try_recv() {
            Ok(Ok(message)) => Ok(Some(message)),
            Ok(Err(error)) => {
                let stderr = self.stderr_tail();
                bail!("{error}\n{stderr}");
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => bail!("Protocol process disconnected"),
        }
    }
    pub fn exited(&mut self) -> bool {
        #[cfg(unix)]
        if let Some(group) = self.process_group {
            // Keep the group leader unreaped until Drop signals the group. Reaping it here
            // would allow its PID/PGID to be reused before retirement.
            let mut status = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
            // SAFETY: status points to writable siginfo_t storage; WNOWAIT preserves
            // ownership of this child, and WNOHANG keeps status checks nonblocking.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    group as libc::id_t,
                    status.as_mut_ptr(),
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            // SAFETY: waitid initialized the zeroed output on success.
            return result == 0 && unsafe { status.assume_init().si_pid() } == group;
        }
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
    pub fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.process_group.take() {
            // SAFETY: only successful isolated spawns store a positive leader PID.
            // The leader remains unreaped, reserving this PGID until after signaling.
            let _ = unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        // Also handle a failed group signal and preserve non-isolated semantics.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn isolated_exit_observation_preserves_leader_until_group_retirement() {
        let directory = tempfile::tempdir().unwrap();
        let mut process = Process::start_isolated(
            "sh",
            &["-c".into(), "sleep 60 & exit 7".into()],
            directory.path(),
        )
        .unwrap();
        let leader = process.child.id() as libc::pid_t;
        let stderr = process.stderr.clone();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !process.exited() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            process.exited(),
            "repeated status checks must not reap the leader"
        );
        // SAFETY: signal zero only checks that our still-owned leader PID exists.
        assert_eq!(unsafe { libc::kill(leader, 0) }, 0);
        drop(process);
        // The descendant inherited stderr. Its termination closes that pipe and lets
        // the transport reader relinquish its Arc, including after leader failure.
        while Arc::strong_count(&stderr) != 1 {
            assert!(Instant::now() < deadline, "inherited stderr was not closed");
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut status = 0;
        // SAFETY: status is writable; only our former child PID is queried.
        assert_eq!(
            unsafe { libc::waitpid(leader, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
