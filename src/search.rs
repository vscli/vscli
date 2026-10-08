//! Bounded, cancellable workspace search. Workers never access live editor state.
use anyhow::Result;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

pub fn line_hash(line: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut state = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut state);
    state.finish()
}
const MAX_RESULTS: usize = 5000;
#[derive(Clone, Default)]
pub struct Options {
    pub case_sensitive: bool,
    pub regex: bool,
    pub whole_word: bool,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub row: usize,
    pub column: usize,
    pub length: usize,
    pub line: String,
    pub line_hash: u64,
}
enum Update {
    Hits(Vec<Hit>),
    Done { skipped: usize, truncated: bool },
}
pub struct Search {
    pub query: String,
    pub hits: Vec<Hit>,
    pub selected: usize,
    pub running: bool,
    pub skipped: usize,
    pub truncated: bool,
    receiver: Receiver<Update>,
    cancel: Arc<AtomicBool>,
}
impl Search {
    pub fn start(
        root: PathBuf,
        query: String,
        options: Options,
        mut overlays: HashMap<PathBuf, String>,
    ) -> Result<Self> {
        let pattern = if options.regex {
            query.clone()
        } else {
            regex::escape(&query)
        };
        let pattern = if options.whole_word {
            format!(r"\b(?:{pattern})\b")
        } else {
            pattern
        };
        let matcher = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case_sensitive)
            .size_limit(4 * 1024 * 1024)
            .build()?;
        let (sender, receiver) = mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        std::thread::spawn(move || {
            let mut count = 0;
            let mut skipped = 0;
            let mut batch = Vec::new();
            let walker = WalkBuilder::new(&root)
                .hidden(false)
                .require_git(false)
                .filter_entry(|e| {
                    !matches!(
                        e.file_name().to_str(),
                        Some(".git" | "target" | "node_modules" | ".venv" | "__pycache__")
                    )
                })
                .build();
            let mut paths = Vec::new();
            for entry in walker {
                if cancelled.load(Ordering::Relaxed) {
                    return;
                }
                match entry {
                    Ok(entry) if entry.file_type().is_some_and(|t| t.is_file()) => {
                        paths.push(entry.into_path())
                    }
                    Err(_) => skipped += 1,
                    _ => {}
                }
                if paths.len() >= 100_000 {
                    break;
                }
            }
            // Include unsaved new files inside this workspace, including ignored open files.
            paths.extend(overlays.keys().filter(|p| p.starts_with(&root)).cloned());
            paths.sort();
            paths.dedup();
            let file_limit = paths.len() >= 100_000;
            'files: for path in paths {
                if cancelled.load(Ordering::Relaxed) {
                    return;
                }
                let text = if let Some(text) = overlays.remove(&path) {
                    text
                } else {
                    let read = (|| -> std::io::Result<String> {
                        let file = std::fs::File::open(&path)?;
                        if file.metadata()?.len() > crate::document::MAX_FILE_BYTES {
                            return Err(std::io::Error::other("file too large"));
                        }
                        let mut bytes = Vec::new();
                        file.take(crate::document::MAX_FILE_BYTES + 1)
                            .read_to_end(&mut bytes)?;
                        if bytes.len() as u64 > crate::document::MAX_FILE_BYTES
                            || bytes.contains(&0)
                        {
                            return Err(std::io::Error::other("not a supported text file"));
                        }
                        String::from_utf8(bytes).map_err(std::io::Error::other)
                    })();
                    match read {
                        Ok(text) => text,
                        Err(_) => {
                            skipped += 1;
                            continue;
                        }
                    }
                };
                for (row, line) in text.lines().enumerate() {
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    for found in matcher.find_iter(line) {
                        batch.push(Hit {
                            path: path.clone(),
                            row,
                            column: line[..found.start()].chars().count(),
                            length: found.as_str().chars().count(),
                            line: line.chars().take(300).collect(),
                            line_hash: line_hash(line),
                        });
                        count += 1;
                        if batch.len() == 64
                            && sender
                                .send(Update::Hits(std::mem::take(&mut batch)))
                                .is_err()
                        {
                            return;
                        }
                        if count >= MAX_RESULTS {
                            break 'files;
                        }
                    }
                }
            }
            if !batch.is_empty() && sender.send(Update::Hits(batch)).is_err() {
                return;
            }
            let _ = sender.send(Update::Done {
                skipped,
                truncated: count >= MAX_RESULTS || file_limit,
            });
        });
        Ok(Self {
            query,
            hits: Vec::new(),
            selected: 0,
            running: true,
            skipped: 0,
            truncated: false,
            receiver,
            cancel,
        })
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..16 {
            match self.receiver.try_recv() {
                Ok(Update::Hits(hits)) => {
                    self.hits.extend(hits);
                    changed = true;
                }
                Ok(Update::Done { skipped, truncated }) => {
                    self.skipped = skipped;
                    self.truncated = truncated;
                    self.running = false;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    changed |= self.running;
                    self.running = false;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        changed
    }
}
impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(search: &mut Search) {
        let start = std::time::Instant::now();
        while search.running {
            search.poll();
            assert!(start.elapsed().as_secs() < 5);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn unicode_regex_ignore_and_unsaved_overlay() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".ignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "cat").unwrap();
        std::fs::write(dir.path().join("a.txt"), "stale").unwrap();
        std::fs::write(dir.path().join("binary"), b"cat\0cat").unwrap();
        let overlays = HashMap::from([(dir.path().join("a.txt"), "猫 CAT catapult\ncat".into())]);
        let mut search = Search::start(
            dir.path().into(),
            "cat".into(),
            Options {
                whole_word: true,
                ..Options::default()
            },
            overlays,
        )
        .unwrap();
        finish(&mut search);
        assert_eq!(search.hits.len(), 2);
        assert_eq!((search.hits[0].column, search.hits[0].length), (2, 3));
        assert_eq!(search.skipped, 1);
        assert!(
            Search::start(
                dir.path().into(),
                "[".into(),
                Options {
                    regex: true,
                    ..Options::default()
                },
                HashMap::new()
            )
            .is_err()
        );
    }
    #[test]
    fn result_limit_and_cancellation_do_not_block_the_editor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("many.txt"), "x\n".repeat(10_000)).unwrap();
        let mut search = Search::start(
            dir.path().into(),
            "x".into(),
            Options::default(),
            HashMap::new(),
        )
        .unwrap();
        finish(&mut search);
        assert_eq!(search.hits.len(), MAX_RESULTS);
        assert!(search.truncated);
        let search = Search::start(
            dir.path().into(),
            "x".into(),
            Options::default(),
            HashMap::new(),
        )
        .unwrap();
        drop(search);
    }
}
