use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
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
}
impl Job {
    pub fn start(action: Action) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
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
        Self { receiver }
    }
    pub fn poll(&mut self) -> Option<std::result::Result<Action, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("File operation worker disconnected".into())),
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
}
