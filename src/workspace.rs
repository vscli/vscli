use ignore::WalkBuilder;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

pub struct Workspace {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    pub indexing: bool,
    receiver: Receiver<Vec<PathBuf>>,
    replacement: Option<Vec<PathBuf>>,
}
impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        let receiver = Self::scan(root.clone());
        Self {
            root,
            files: Vec::new(),
            indexing: true,
            receiver,
            replacement: None,
        }
    }
    fn scan(scan_root: PathBuf) -> Receiver<Vec<PathBuf>> {
        let (tx, receiver) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let mut batch = Vec::new();
            let mut count = 0;
            for item in WalkBuilder::new(&scan_root)
                .hidden(false)
                .require_git(false)
                .filter_entry(|e| {
                    !matches!(
                        e.file_name().to_str(),
                        Some(".git" | "target" | "node_modules" | ".venv" | "__pycache__")
                    )
                })
                .build()
                .flatten()
            {
                if item.file_type().is_some_and(|t| t.is_file()) {
                    batch.push(item.path().to_path_buf());
                    count += 1;
                    if batch.len() >= 256 && tx.send(std::mem::take(&mut batch)).is_err() {
                        return;
                    }
                    if count >= 100_000 {
                        break;
                    }
                }
            }
            if !batch.is_empty() {
                let _ = tx.send(batch);
            }
        });
        receiver
    }
    pub fn refresh(&mut self) {
        self.receiver = Self::scan(self.root.clone());
        self.replacement = Some(Vec::new());
        self.indexing = true;
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.receiver.try_recv() {
                Ok(batch) => {
                    self.replacement
                        .as_mut()
                        .unwrap_or(&mut self.files)
                        .extend(batch);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(files) = self.replacement.take() {
                        self.files = files;
                    }
                    if self.indexing {
                        changed = true;
                    }
                    self.indexing = false;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        changed
    }
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }
    pub fn matches(&self, query: &str) -> Vec<PathBuf> {
        let mut found: Vec<_> = self
            .files
            .iter()
            .filter_map(|p| score(&self.relative(p), query).map(|s| (s, p)))
            .collect();
        found.sort_by(|(a, pa), (b, pb)| b.cmp(a).then_with(|| pa.cmp(pb)));
        found
            .into_iter()
            .take(100)
            .map(|(_, p)| p.clone())
            .collect()
    }
}

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub directory: bool,
}
pub fn directory_entries(path: &Path) -> Vec<Entry> {
    let mut entries: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name() != ".git")
        .map(|e| Entry {
            directory: e.path().is_dir(),
            path: e.path(),
        })
        .collect();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.cmp(&b.path))
    });
    entries
}
pub fn score(candidate: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(-(candidate.len() as i64));
    }
    let candidate = candidate.to_lowercase();
    let query = query.to_lowercase();
    let mut want = query.chars();
    let mut next = want.next();
    let mut score = 0i64;
    let mut last = None;
    for (i, c) in candidate.chars().enumerate() {
        if next == Some(c) {
            score += 10;
            if last == Some(i.saturating_sub(1)) {
                score += 15;
            }
            if i == 0 {
                score += 20;
            }
            last = Some(i);
            next = want.next();
            if next.is_none() {
                return Some(score - candidate.len() as i64);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fuzzy_paths() {
        assert!(score("src/main.rs", "smr").is_some());
        assert!(score("Cargo.toml", "xyz").is_none());
    }
}
