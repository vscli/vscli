use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
};

#[derive(Clone)]
pub enum Action {
    CreateFile(PathBuf),
    CreateFolder(PathBuf),
    Rename { from: PathBuf, to: PathBuf },
    Trash(PathBuf),
}
pub struct Job {
    receiver: Receiver<std::result::Result<Action, String>>,
    thread: Option<JoinHandle<()>>,
    terminal: Option<std::result::Result<Action, String>>,
}
impl Job {
    pub fn start(action: Action) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let mut action = action;
            match &mut action {
                Action::Rename { from: path, .. } | Action::Trash(path) => {
                    if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_symlink())
                        && let Ok(canonical) = std::fs::canonicalize(&path)
                    {
                        *path = canonical;
                    }
                }
                _ => {}
            }
            let result = perform(&action)
                .map(|_| action)
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        Self {
            receiver,
            thread: Some(thread),
            terminal: None,
        }
    }
    pub fn poll(&mut self) -> Option<std::result::Result<Action, String>> {
        if self.terminal.is_none() {
            match self.receiver.try_recv() {
                Ok(result) => self.terminal = Some(result),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(_) => {
                    self.terminal = Some(Err(
                        "File operation worker disconnected; outcome unknown".into()
                    ))
                }
            }
        }
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            return None;
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            self.terminal = Some(Err("File operation worker panicked; outcome unknown".into()));
        }
        self.terminal.take()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn perform(action: &Action) -> Result<()> {
    match action {
        Action::CreateFile(path) => {
            fs::File::create_new(path)
                .with_context(|| format!("Cannot create {}", path.display()))?;
        }
        Action::CreateFolder(path) => {
            fs::create_dir(path)
                .with_context(|| format!("Cannot create folder {}", path.display()))?;
        }
        Action::Rename { from, to } => {
            if from == to {
                return Ok(());
            }
            if fs::symlink_metadata(to).is_ok() {
                bail!("Destination already exists; rename refused");
            }
            fs::rename(from, to).context("Rename failed")?;
        }
        Action::Trash(path) => {
            trash::delete(path).context(
                "Could not move item to system trash; nothing is permanently deleted as a fallback",
            )?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_and_rename_protect_existing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original");
        let destination = dir.path().join("destination");
        perform(&Action::CreateFile(original.clone())).unwrap();
        fs::write(&original, "preserve me").unwrap();
        assert!(perform(&Action::CreateFile(original.clone())).is_err());
        perform(&Action::CreateFile(destination.clone())).unwrap();
        assert!(
            perform(&Action::Rename {
                from: original.clone(),
                to: destination
            })
            .is_err()
        );
        let new = dir.path().join("new");
        perform(&Action::Rename {
            from: original,
            to: new.clone(),
        })
        .unwrap();
        assert_eq!(fs::read_to_string(new).unwrap(), "preserve me");
    }
    #[test]
    fn terminal_reply_does_not_release_worker_before_actual_exit_and_join() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            sender
                .send(Ok(Action::CreateFolder(std::env::temp_dir())))
                .unwrap();
            gate.recv().unwrap();
        });
        let mut job = Job {
            receiver,
            thread: Some(thread),
            terminal: None,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while job.terminal.is_none() {
            assert!(job.poll().is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(job.thread.is_some());
        release.send(()).unwrap();
        let action = loop {
            if let Some(action) = job.poll() {
                break action;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert!(matches!(action, Ok(Action::CreateFolder(_))));
        assert!(job.thread.is_none());
    }
}
