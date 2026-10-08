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
    sender: SyncSender<Vec<u8>>,
    receiver: Receiver<std::result::Result<Value, String>>,
    queued_bytes: Arc<AtomicUsize>,
    stderr: Arc<Mutex<Vec<u8>>>,
}
impl Process {
    pub fn start(program: &str, args: &[String], root: &Path) -> Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
        Ok(Self {
            child,
            sender,
            receiver,
            queued_bytes,
            stderr: stderr_tail,
        })
    }
    pub fn send(&self, message: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&message)?;
        let size = bytes.len();
        if size > MAX_QUEUED_BYTES {
            bail!("Protocol message exceeds output limit");
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
                let stderr = self
                    .stderr
                    .lock()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                bail!("{error}\n{stderr}");
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => bail!("Protocol process disconnected"),
        }
    }
    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
