//! In-memory thread-safe implementation of [`AuditLogger`].
//!
//! Stores audit records in a bounded ring-buffer, suitable for test harnesses,
//! ephemeral deployments, and local development.

use std::collections::VecDeque;

use parking_lot::RwLock;

use super::{AuditError, AuditFilter, AuditLogger, AuditRecord};

/// Thread-safe in-memory audit logger bounded by a maximum capacity.
#[derive(Debug)]
pub struct MemoryAuditLogger {
    max_capacity: usize,
    records: RwLock<VecDeque<AuditRecord>>,
}

impl Default for MemoryAuditLogger {
    fn default() -> Self {
        Self::new(10_000)
    }
}

impl MemoryAuditLogger {
    /// Creates a new in-memory logger retaining up to `max_capacity` recent records.
    pub fn new(max_capacity: usize) -> Self {
        Self {
            max_capacity: max_capacity.max(1),
            records: RwLock::new(VecDeque::with_capacity(max_capacity.min(1024))),
        }
    }

    /// Clears all recorded audit entries.
    pub fn clear(&self) {
        self.records.write().clear();
    }

    /// Returns the current number of retained records.
    pub fn len(&self) -> usize {
        self.records.read().len()
    }

    /// Returns `true` if no records are currently retained.
    pub fn is_empty(&self) -> bool {
        self.records.read().is_empty()
    }
}

impl AuditLogger for MemoryAuditLogger {
    fn record(&self, record: AuditRecord) -> Result<(), AuditError> {
        let mut lock = self.records.write();
        if lock.len() >= self.max_capacity {
            lock.pop_back();
        }
        lock.push_front(record);
        Ok(())
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let lock = self.records.read();
        let matched = lock
            .iter()
            .filter(|rec| filter.matches(rec))
            .skip(filter.offset)
            .take(filter.limit)
            .cloned()
            .collect();
        Ok(matched)
    }

    fn count(&self, filter: &AuditFilter) -> Result<usize, AuditError> {
        let lock = self.records.read();
        let count = lock.iter().filter(|rec| filter.matches(rec)).count();
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditAction, AuditActor, AuditOutcome};
    use crate::domain::ids::{generate_id, AuditRecordId};

    fn make_record(tenant: Option<&str>, action_name: &str, timestamp: i64) -> AuditRecord {
        let mut rec = AuditRecord::new(
            AuditRecordId::parse(&generate_id("au_")).unwrap(),
            timestamp,
            AuditActor::System {
                component: "test".into(),
            },
            AuditAction::Custom {
                name: action_name.into(),
            },
            AuditOutcome::Success,
            "test_resource",
        );
        if let Some(t) = tenant {
            rec = rec.with_tenant(t);
        }
        rec
    }

    #[test]
    fn memory_logger_records_and_queries_newest_first() {
        let logger = MemoryAuditLogger::new(10);
        logger.record(make_record(Some("t1"), "act1", 100)).unwrap();
        logger.record(make_record(Some("t1"), "act2", 200)).unwrap();
        logger.record(make_record(Some("t2"), "act3", 300)).unwrap();

        let all = logger.query(&AuditFilter::new()).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].timestamp_ms, 300);
        assert_eq!(all[1].timestamp_ms, 200);
        assert_eq!(all[2].timestamp_ms, 100);

        let t1_filter = AuditFilter::new().with_tenant("t1");
        let t1_res = logger.query(&t1_filter).unwrap();
        assert_eq!(t1_res.len(), 2);
        assert_eq!(logger.count(&t1_filter).unwrap(), 2);
    }

    #[test]
    fn memory_logger_respects_capacity_ring_buffer() {
        let logger = MemoryAuditLogger::new(3);
        logger.record(make_record(None, "1", 1)).unwrap();
        logger.record(make_record(None, "2", 2)).unwrap();
        logger.record(make_record(None, "3", 3)).unwrap();
        logger.record(make_record(None, "4", 4)).unwrap();

        assert_eq!(logger.len(), 3);
        let entries = logger.query(&AuditFilter::new()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].timestamp_ms, 4);
        assert_eq!(entries[1].timestamp_ms, 3);
        assert_eq!(entries[2].timestamp_ms, 2);
    }

    #[test]
    fn memory_logger_pagination_offset_and_limit() {
        let logger = MemoryAuditLogger::new(10);
        for i in 1..=5 {
            logger.record(make_record(None, "act", i * 10)).unwrap();
        }

        let filter = AuditFilter::new().with_pagination(2, 1);
        let page = logger.query(&filter).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].timestamp_ms, 40);
        assert_eq!(page[1].timestamp_ms, 30);
    }
}
