//! File-backed append-only JSONL implementation of [`AuditLogger`].
//!
//! Audit events are serialized to JSON Lines format, with each record terminated
//! by a newline and immediately flushed for durability.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;

use super::{AuditError, AuditFilter, AuditLogger, AuditRecord};

/// Thread-safe file-backed audit logger storing records as JSON Lines.
pub struct FileAuditLogger {
    path: PathBuf,
    writer: Mutex<BufWriter<File>>,
}

impl std::fmt::Debug for FileAuditLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileAuditLogger")
            .field("path", &self.path)
            .finish()
    }
}

impl FileAuditLogger {
    /// Opens or creates the audit log file at `path` for appending.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AuditError> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path_buf)?;

        Ok(Self {
            path: path_buf,
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Returns the filesystem path to the audit log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_all_matching(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut matching = Vec::new();

        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<AuditRecord>(trimmed) {
                if filter.matches(&record) {
                    matching.push(record);
                }
            }
        }

        // Audit queries return newest records first
        matching.reverse();
        Ok(matching)
    }
}

impl AuditLogger for FileAuditLogger {
    fn record(&self, record: AuditRecord) -> Result<(), AuditError> {
        let serialized = serde_json::to_string(&record)?;
        let mut writer = self.writer.lock();
        writer.write_all(serialized.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let matching = self.read_all_matching(filter)?;
        let paged = matching
            .into_iter()
            .skip(filter.offset)
            .take(filter.limit)
            .collect();
        Ok(paged)
    }

    fn count(&self, filter: &AuditFilter) -> Result<usize, AuditError> {
        let matching = self.read_all_matching(filter)?;
        Ok(matching.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditAction, AuditActor, AuditOutcome};
    use crate::domain::ids::{generate_id, AuditRecordId};

    fn make_record(tenant: Option<&str>, action: &str, timestamp: i64) -> AuditRecord {
        let mut rec = AuditRecord::new(
            AuditRecordId::parse(&generate_id("au_")).unwrap(),
            timestamp,
            AuditActor::User {
                username: "alice".into(),
                role: "admin".into(),
            },
            AuditAction::Custom {
                name: action.into(),
            },
            AuditOutcome::Success,
            "system_config",
        );
        if let Some(t) = tenant {
            rec = rec.with_tenant(t);
        }
        rec
    }

    #[test]
    fn file_logger_persists_and_queries_records() {
        let dir = std::env::temp_dir().join(format!("runvane_audit_test_{}", generate_id("au_")));
        let file_path = dir.join("audit.jsonl");

        let logger = FileAuditLogger::open(&file_path).expect("open logger");
        logger
            .record(make_record(Some("tenant_a"), "op1", 1000))
            .unwrap();
        logger
            .record(make_record(Some("tenant_b"), "op2", 2000))
            .unwrap();
        logger
            .record(make_record(Some("tenant_a"), "op3", 3000))
            .unwrap();

        // Query across all
        let all = logger.query(&AuditFilter::new()).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].timestamp_ms, 3000);
        assert_eq!(all[1].timestamp_ms, 2000);
        assert_eq!(all[2].timestamp_ms, 1000);

        // Filter by tenant
        let filter = AuditFilter::new().with_tenant("tenant_a");
        let tenant_a_records = logger.query(&filter).unwrap();
        assert_eq!(tenant_a_records.len(), 2);
        assert_eq!(tenant_a_records[0].timestamp_ms, 3000);
        assert_eq!(tenant_a_records[1].timestamp_ms, 1000);

        // Re-open existing file and verify persistence
        let logger2 = FileAuditLogger::open(&file_path).expect("reopen logger");
        assert_eq!(logger2.count(&AuditFilter::new()).unwrap(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
