use crate::document::{Document, Selection};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct SavedDocument {
    path: Option<PathBuf>,
    text: String,
    disk_content: Option<String>,
    cursor: usize,
    #[serde(default)]
    selections: Vec<Selection>,
}
#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    documents: Vec<SavedDocument>,
}

pub struct Recovery {
    pub directory: PathBuf,
    path: PathBuf,
    lock_path: PathBuf,
    _lock: File,
}
impl Drop for Recovery {
    fn drop(&mut self) {
        // A concurrent fork can briefly inherit this open file description.
        // Explicit unlock releases the session even before that child execs.
        let _ = self._lock.unlock();
    }
}
impl Recovery {
    pub fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = directory.join(format!("{}-{stamp}.json", std::process::id()));
        let lock_path = path.with_extension("lock");
        let lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        lock.lock()?;
        Ok(Self {
            directory,
            path,
            lock_path,
            _lock: lock,
        })
    }
    pub fn restore(&self) -> Result<(Vec<Document>, Vec<PathBuf>)> {
        let mut docs = Vec::new();
        let mut consumed = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") || path == self.path {
                continue;
            }
            let lock = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path.with_extension("lock"))?;
            if lock.try_lock().is_err() {
                continue;
            }
            let data = fs::read_to_string(&path)?;
            let Ok(session) = serde_json::from_str::<Session>(&data) else {
                continue;
            };
            if session.version != 1 {
                continue;
            }
            for saved in session.documents {
                let baseline = saved.disk_content.as_deref().map(ropey::Rope::from_str);
                let mut doc = Document::from_rope(baseline.clone().unwrap_or_default());
                doc.path = saved.path;
                doc.disk_content = baseline;
                doc.move_to(saved.cursor.min(doc.len()), false);
                // Undo recovered edits returns to the actual saved baseline, not a clean
                // snapshot containing unsaved text. Empty recovered files matter too.
                doc.select_all();
                doc.insert(&saved.text, false);
                doc.move_to(saved.cursor.min(doc.len()), false);
                if !saved.selections.is_empty() {
                    doc.set_selections(saved.selections);
                }
                docs.push(doc);
            }
            consumed.push(path);
        }
        Ok((docs, consumed))
    }
    pub fn persist(&self, documents: &[Document]) -> Result<()> {
        let session = Session {
            version: 1,
            documents: documents
                .iter()
                .filter(|d| d.dirty())
                .map(|d| SavedDocument {
                    path: d.path.clone(),
                    text: d.text.to_string(),
                    disk_content: d.disk_content.as_ref().map(ToString::to_string),
                    cursor: d.cursor,
                    selections: d.selections(),
                })
                .collect(),
        };
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        serde_json::to_writer(&mut temp, &session)?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|e| e.error)
            .context("Could not save recovery snapshot")?;
        Ok(())
    }
    pub fn consume(paths: &[PathBuf]) {
        for path in paths {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(path.with_extension("lock"));
        }
    }
    pub fn finish(self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        self._lock.unlock()?;
        fs::remove_file(&self.lock_path)?;
        Ok(())
    }
}
pub fn default_directory() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "vscli", "vscli")
        .map(|d| d.state_dir().unwrap_or(d.data_local_dir()).join("recovery"))
}
pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "vscli", "vscli")
        .map(|d| d.config_dir().join("keybindings.json"))
}
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace(|c: char| c.is_control(), "�")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_sessions_are_skipped_and_crashes_restore_unsaved_text() {
        let dir = tempfile::tempdir().unwrap();
        let a = Recovery::new(dir.path().into()).unwrap();
        let mut d = Document::default();
        d.insert("unsaved🙂", false);
        a.persist(&[d]).unwrap();
        let b = Recovery::new(dir.path().into()).unwrap();
        assert!(b.restore().unwrap().0.is_empty());
        drop(a);
        let (docs, paths) = b.restore().unwrap();
        assert_eq!(docs[0].text.to_string(), "unsaved🙂");
        assert!(docs[0].dirty());
        b.persist(&docs).unwrap();
        Recovery::consume(&paths);
        b.finish().unwrap();
    }

    #[test]
    fn recovering_a_deleted_file_buffer_preserves_dirty_state_and_real_undo_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, "original").unwrap();
        let a = Recovery::new(dir.path().join("state")).unwrap();
        let mut d = Document::open(&file).unwrap();
        d.select_all();
        d.insert("", false);
        a.persist(&[d]).unwrap();
        drop(a);
        let b = Recovery::new(dir.path().join("state")).unwrap();
        let (mut docs, old) = b.restore().unwrap();
        assert!(docs[0].is_empty());
        assert!(docs[0].dirty());
        docs[0].undo();
        assert_eq!(docs[0].text.to_string(), "original");
        assert!(!docs[0].dirty());
        b.persist(&docs).unwrap();
        Recovery::consume(&old);
        b.finish().unwrap();
    }
}
