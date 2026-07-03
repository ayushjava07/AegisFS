use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::error::AegisResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEventKind {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

struct FileState {
    mtime: SystemTime,
}

pub struct DirectoryWatcher {
    path: PathBuf,
    files: HashMap<PathBuf, FileState>,
}

impl DirectoryWatcher {
    pub fn new() -> Self {
        Self {
            path: PathBuf::new(),
            files: HashMap::new(),
        }
    }

    pub fn watch(&mut self, path: &Path) -> AegisResult<()> {
        self.path = path.to_path_buf();
        self.files.clear();
        self.scan()
    }

    pub fn poll(&mut self) -> AegisResult<Vec<WatchEvent>> {
        let mut events = Vec::new();
        let mut current = HashMap::new();
        let dir = match self.path.read_dir() {
            Ok(d) => d,
            Err(_) => return Ok(events),
        };

        for entry in dir.flatten() {
            let path = entry.path();
            let mtime = match entry.metadata().and_then(|m| m.modified()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            current.insert(path.clone(), FileState { mtime });
        }

        for (path, state) in &current {
            match self.files.get(path) {
                None => {
                    events.push(WatchEvent {
                        path: path.clone(),
                        kind: WatchEventKind::Created,
                    });
                }
                Some(old) if old.mtime != state.mtime => {
                    events.push(WatchEvent {
                        path: path.clone(),
                        kind: WatchEventKind::Modified,
                    });
                }
                _ => {}
            }
        }

        for path in self.files.keys() {
            if !current.contains_key(path) {
                events.push(WatchEvent {
                    path: path.clone(),
                    kind: WatchEventKind::Deleted,
                });
            }
        }

        self.files = current;
        Ok(events)
    }

    fn scan(&mut self) -> AegisResult<()> {
        let dir = match self.path.read_dir() {
            Ok(d) => d,
            Err(e) => return Err(crate::core::error::AegisError::Io(e)),
        };

        for entry in dir.flatten() {
            let path = entry.path();
            let mtime = match entry.metadata().and_then(|m| m.modified()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            self.files.insert(path, FileState { mtime });
        }

        Ok(())
    }
}

impl Default for DirectoryWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_watch_create() {
        let dir = TempDir::new().unwrap();
        let mut watcher = DirectoryWatcher::new();
        watcher.watch(dir.path()).unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello").unwrap();
        let events = watcher.poll().unwrap();
        assert!(events.iter().any(|e| e.kind == WatchEventKind::Created));
    }

    #[test]
    fn test_watch_modify() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello").unwrap();
        let mut watcher = DirectoryWatcher::new();
        watcher.watch(dir.path()).unwrap();
        fs::write(&path, "world").unwrap();
        let events = watcher.poll().unwrap();
        assert!(events.iter().any(|e| e.kind == WatchEventKind::Modified));
    }

    #[test]
    fn test_watch_delete() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello").unwrap();
        let mut watcher = DirectoryWatcher::new();
        watcher.watch(dir.path()).unwrap();
        fs::remove_file(&path).unwrap();
        let events = watcher.poll().unwrap();
        assert!(events.iter().any(|e| e.kind == WatchEventKind::Deleted));
    }
}
