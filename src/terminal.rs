use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender},
};

enum Update {
    Output(Vec<u8>),
    Exit(String),
    Error(String),
}
#[derive(Default)]
pub struct Responses {
    bytes: Vec<u8>,
}
impl vt100::Callbacks for Responses {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        intermediate1: Option<u8>,
        intermediate2: Option<u8>,
        params: &[&[u16]],
        action: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let reply = match (intermediate1, intermediate2, action, first) {
            (None, None, 'n', 5) => "\x1b[0n".into(),
            (None, None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                format!("\x1b[{};{}R", row + 1, col + 1)
            }
            (None, None, 'c', 0) => "\x1b[?1;2c".into(),
            _ => String::new(),
        };
        if self.bytes.len() + reply.len() <= 8192 {
            self.bytes.extend_from_slice(reply.as_bytes());
        }
    }
}
pub struct Session {
    master: Box<dyn MasterPty + Send>,
    stop: SyncSender<()>,
    sender: SyncSender<Vec<u8>>,
    receiver: Receiver<Update>,
    pub parser: vt100::Parser<Responses>,
    pub title: String,
    pub exited: bool,
    pub status: String,
}
impl Session {
    pub fn shell(root: &Path, rows: u16, cols: u16) -> Result<Self> {
        let shell = std::env::var_os(if cfg!(windows) { "COMSPEC" } else { "SHELL" })
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "cmd.exe".into()
                } else {
                    "/bin/sh".into()
                }
            });
        let mut command = CommandBuilder::new(&shell);
        command.cwd(root);
        Self::spawn(command, shell.to_string_lossy().into_owned(), rows, cols)
    }
    pub fn spawn(mut command: CommandBuilder, title: String, rows: u16, cols: u16) -> Result<Self> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        command.env("TERM", "xterm-256color");
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let mut child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let (sender, input) = mpsc::sync_channel::<Vec<u8>>(32);
        let (output, receiver) = mpsc::sync_channel(32);
        let errors = output.clone();
        std::thread::spawn(move || {
            for bytes in input {
                if let Err(e) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                    let _ = errors.send(Update::Error(format!("Terminal input failed: {e}")));
                    break;
                }
            }
        });
        let exit = output.clone();
        let (stop, control) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let _ = exit.send(Update::Exit(status.to_string()));
                        break;
                    }
                    Err(e) => {
                        let _ = exit.send(Update::Exit(e.to_string()));
                        break;
                    }
                    Ok(None) => {}
                }
                match control.recv_timeout(std::time::Duration::from_millis(20)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => {
                        // The same worker owns signalling and reaping, avoiding recycled-PID races.
                        let _ = child.kill();
                        let status = child
                            .wait()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|e| e.to_string());
                        let _ = exit.send(Update::Exit(status));
                        break;
                    }
                }
            }
        });
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if output.send(Update::Output(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break, // Unix PTYs commonly report EIO at child exit.
                }
            }
        });
        Ok(Self {
            master: pair.master,
            stop,
            sender,
            receiver,
            parser: vt100::Parser::new_with_callbacks(rows, cols, 5000, Responses::default()),
            title,
            exited: false,
            status: "running".into(),
        })
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let size = (rows.max(1), cols.max(1));
        if self.parser.screen().size() != size {
            self.master.resize(PtySize {
                rows: size.0,
                cols: size.1,
                pixel_width: 0,
                pixel_height: 0,
            })?;
            self.parser.screen_mut().set_size(size.0, size.1);
        }
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..16 {
            match self.receiver.try_recv() {
                Ok(Update::Output(bytes)) => {
                    self.parser.process(&bytes);
                    changed = true;
                }
                Ok(Update::Exit(status)) => {
                    self.exited = true;
                    self.status = status;
                    changed = true;
                }
                Ok(Update::Error(error)) => {
                    self.status = error;
                    changed = true;
                }
                Err(_) => break,
            }
        }
        let responses = std::mem::take(&mut self.parser.callbacks_mut().bytes);
        if !responses.is_empty()
            && let Err(e) = self.write(responses)
        {
            self.status = e.to_string();
        }
        changed
    }
    pub fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        if self.exited {
            bail!("Terminal process has exited");
        }
        if bytes.len() > 1024 * 1024 {
            bail!("Terminal paste exceeds 1 MiB");
        }
        self.sender
            .try_send(bytes)
            .map_err(|e| anyhow::anyhow!("Terminal input queue unavailable: {e}"))?;
        self.parser.screen_mut().set_scrollback(0);
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result<()> {
        let bytes = if self.parser.screen().bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        self.write(bytes)
    }
    pub fn key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers == KeyModifiers::SHIFT
            && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
        {
            let offset = self.parser.screen().scrollback();
            let page = self.parser.screen().size().0 as usize;
            self.parser
                .screen_mut()
                .set_scrollback(if key.code == KeyCode::PageUp {
                    offset.saturating_add(page)
                } else {
                    offset.saturating_sub(page)
                });
            return Ok(());
        }
        let bytes = encode_key(key, self.parser.screen().application_cursor());
        if bytes.is_empty() {
            bail!("This key cannot be represented by the embedded terminal protocol");
        }
        self.write(bytes)
    }
    pub fn kill(&mut self) -> Result<()> {
        match self.stop.try_send(()) {
            Ok(())
            | Err(mpsc::TrySendError::Disconnected(_))
            | Err(mpsc::TrySendError::Full(_)) => Ok(()),
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
    }
}

fn encode_key(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let mods = key.modifiers;
    let modifier = 1
        + u8::from(mods.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(mods.contains(KeyModifiers::ALT))
        + 4 * u8::from(mods.contains(KeyModifiers::CONTROL));
    let mut prefix = Vec::new();
    let bytes = match key.code {
        KeyCode::Char(ch) => {
            if mods.intersects(KeyModifiers::SUPER | KeyModifiers::META) {
                return Vec::new();
            }
            if mods.contains(KeyModifiers::ALT) {
                prefix.push(0x1b);
            }
            if mods.contains(KeyModifiers::CONTROL) {
                match ch.to_ascii_lowercase() {
                    'a'..='z' => vec![ch.to_ascii_lowercase() as u8 - b'a' + 1],
                    ' ' | '@' => vec![0],
                    '[' => vec![27],
                    '\\' => vec![28],
                    ']' => vec![29],
                    '^' => vec![30],
                    '_' => vec![31],
                    '?' => vec![127],
                    _ => return Vec::new(),
                }
            } else {
                ch.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![27],
        KeyCode::Backspace => {
            if mods.contains(KeyModifiers::ALT) {
                prefix.push(27);
            }
            vec![127]
        }
        KeyCode::Null => vec![0],
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Right
        | KeyCode::Left
        | KeyCode::Home
        | KeyCode::End => {
            let tail = match key.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            if modifier > 1 {
                format!("\x1b[1;{modifier}{tail}").into_bytes()
            } else {
                format!("\x1b{}{tail}", if application_cursor { 'O' } else { '[' }).into_bytes()
            }
        }
        KeyCode::Insert
        | KeyCode::Delete
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::F(5..=12) => {
            let code = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                KeyCode::PageDown => 6,
                KeyCode::F(n) => [15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize],
                _ => unreachable!(),
            };
            if modifier > 1 {
                format!("\x1b[{code};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{code}~").into_bytes()
            }
        }
        KeyCode::F(n @ 1..=4) => {
            let tail = (b'P' + n - 1) as char;
            if modifier > 1 {
                format!("\x1b[1;{modifier}{tail}").into_bytes()
            } else {
                format!("\x1bO{tail}").into_bytes()
            }
        }
        _ => Vec::new(),
    };
    prefix.extend(bytes);
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_encoding_and_parser_queries() {
        assert_eq!(
            encode_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false
            ),
            vec![3]
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT), true),
            b"\x1b[1;3A"
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 100, Responses::default());
        parser.process(b"\x1b[3;4H\x1b[6n");
        assert_eq!(parser.callbacks().bytes, b"\x1b[3;4R");
    }
    #[cfg(unix)]
    #[test]
    fn native_pty_roundtrip_resize_and_exit() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = CommandBuilder::new("/bin/sh");
        command.cwd(dir.path());
        let mut terminal = Session::spawn(command, "test".into(), 10, 60).unwrap();
        terminal.resize(12, 70).unwrap();
        terminal
            .write(b"printf '\\033[31mPTY_OK\\033[0m\\n'; stty size; exit 0\r".to_vec())
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            terminal.poll();
            if terminal.exited && terminal.parser.screen().contents().contains("12 70") {
                break;
            }
            assert!(
                start.elapsed().as_secs() < 5,
                "{}",
                terminal.parser.screen().contents()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(terminal.parser.screen().contents().contains("PTY_OK"));
    }
}
