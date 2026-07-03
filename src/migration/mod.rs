use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::error::AegisResult;

type MigrationFn = Box<dyn Fn() -> AegisResult<()> + Send + Sync>;

pub struct MigrationManager {
    migrations: HashMap<(String, String), MigrationFn>,
    current_version: String,
}

impl MigrationManager {
    pub fn new() -> Self {
        Self {
            migrations: HashMap::new(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn register(&mut self, from_version: &str, to_version: &str, migration: MigrationFn) {
        self.migrations.insert(
            (from_version.to_string(), to_version.to_string()),
            migration,
        );
    }

    pub fn migrate(&self, from: &str, to: &str) -> AegisResult<()> {
        if from == to {
            return Ok(());
        }

        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for (f, t) in self.migrations.keys() {
            adj.entry(f.as_str()).or_default().push(t.as_str());
        }

        let mut visited: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut parent: HashMap<&str, &str> = HashMap::new();

        visited.insert(from);
        queue.push_back(from);

        while let Some(current) = queue.pop_front() {
            if current == to {
                break;
            }
            if let Some(neighbors) = adj.get(current) {
                for next in neighbors {
                    if visited.insert(next) {
                        parent.insert(next, current);
                        queue.push_back(next);
                    }
                }
            }
        }

        if !parent.contains_key(to) {
            return Err(crate::core::error::AegisError::NotSupported(format!(
                "no migration path from {} to {}",
                from, to
            )));
        }

        let mut path = Vec::new();
        let mut current = to;
        while current != from {
            let prev = parent.get(current).ok_or_else(|| {
                crate::core::error::AegisError::Internal("path reconstruction failed".into())
            })?;
            path.push((*prev, current));
            current = prev;
        }
        path.reverse();

        for (f, t) in &path {
            let key = (f.to_string(), t.to_string());
            if let Some(migration) = self.migrations.get(&key) {
                migration()?;
            }
        }

        Ok(())
    }

    pub fn current_version(&self) -> &str {
        &self.current_version
    }
}

impl Default for MigrationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_direct_migration() {
        let mut manager = MigrationManager::new();
        manager.register("1.0.0", "2.0.0", Box::new(|| Ok(())));

        assert!(manager.migrate("1.0.0", "2.0.0").is_ok());
    }

    #[test]
    fn test_no_migration_path() {
        let manager = MigrationManager::new();
        let result = manager.migrate("1.0.0", "2.0.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_current_version_not_empty() {
        let manager = MigrationManager::new();
        assert!(!manager.current_version().is_empty());
    }

    #[test]
    fn test_same_version_no_op() {
        let manager = MigrationManager::new();
        assert!(manager.migrate("1.0.0", "1.0.0").is_ok());
    }

    #[test]
    fn test_chained_migration() {
        let mut manager = MigrationManager::new();
        manager.register("1.0.0", "2.0.0", Box::new(|| Ok(())));
        manager.register("2.0.0", "3.0.0", Box::new(|| Ok(())));

        assert!(manager.migrate("1.0.0", "3.0.0").is_ok());
    }
}
