use std::collections::HashSet;

use chrono::Utc;

use crate::core::error::AegisResult;
use crate::core::traits::PolicyEngine;
use crate::core::types::*;

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub max_snapshots: usize,
    pub min_age_days: u64,
    pub tags_keep: Vec<String>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_snapshots: 30,
            min_age_days: 7,
            tags_keep: vec!["weekly".into(), "monthly".into(), "yearly".into()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct GcPolicy {
    pub min_chunk_age_hours: u64,
    pub max_unreferenced_chunks: u64,
    pub aggressive: bool,
}

impl Default for GcPolicy {
    fn default() -> Self {
        Self {
            min_chunk_age_hours: 24,
            max_unreferenced_chunks: 10000,
            aggressive: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub retention: RetentionPolicy,
    pub gc: GcPolicy,
    pub compression_min_savings_percent: f64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            retention: RetentionPolicy::default(),
            gc: GcPolicy::default(),
            compression_min_savings_percent: 10.0,
        }
    }
}

pub struct PolicyEngineImpl {
    config: PolicyConfig,
}

impl PolicyEngineImpl {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: PolicyConfig) {
        self.config = config;
    }
}

impl PolicyEngine for PolicyEngineImpl {
    fn evaluate_retention(&self, snapshots: &[Snapshot]) -> AegisResult<Vec<SnapshotId>> {
        let mut eligible: Vec<SnapshotId> = Vec::new();

        if snapshots.len() <= self.config.retention.max_snapshots {
            return Ok(eligible);
        }

        let keep_count = self.config.retention.max_snapshots;
        let now = Utc::now();

        let mut sorted: Vec<&Snapshot> = snapshots.iter().collect();
        sorted.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        let keep_set: HashSet<SnapshotId> = sorted
            .iter()
            .take(keep_count)
            .map(|s| s.id)
            .collect();

        let tagged_keep: HashSet<SnapshotId> = sorted
            .iter()
            .filter(|s| {
                s.labels
                    .values()
                    .any(|v| self.config.retention.tags_keep.contains(v))
            })
            .map(|s| s.id)
            .collect();

        for snapshot in &sorted {
            let age = now - snapshot.timestamp;
            if keep_set.contains(&snapshot.id) || tagged_keep.contains(&snapshot.id) {
                continue;
            }
            if age.num_days() < self.config.retention.min_age_days as i64 {
                continue;
            }
            eligible.push(snapshot.id);
        }

        Ok(eligible)
    }

    fn evaluate_gc(&self, chunks: &[ChunkId], referenced: &[ChunkId]) -> AegisResult<Vec<ChunkId>> {
        let ref_set: HashSet<&ChunkId> = referenced.iter().collect();

        let mut orphaned: Vec<ChunkId> = chunks
            .iter()
            .filter(|id| !ref_set.contains(id))
            .copied()
            .collect();

        if self.config.gc.aggressive {
            return Ok(orphaned);
        }

        if orphaned.len() as u64 > self.config.gc.max_unreferenced_chunks {
            orphaned.truncate(self.config.gc.max_unreferenced_chunks as usize);
        }

        Ok(orphaned)
    }

    fn meets_requirements(&self, archive: &Archive) -> AegisResult<bool> {
        if archive.sealed {
            return Ok(true);
        }

        if let crate::core::types::CompressionAlgorithm::Zstd(level) = archive.compression {
            if level < 1 {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_snapshot(
        id: &str,
        days_ago: i64,
        labels: Vec<(&str, &str)>,
    ) -> Snapshot {
        let sid = SnapshotId::from_uuid(
            uuid::Uuid::parse_str(id).unwrap_or(uuid::Uuid::nil()),
        );
        let mut label_map = HashMap::new();
        for (k, v) in labels {
            label_map.insert(k.to_string(), v.to_string());
        }
        Snapshot {
            id: sid,
            parent: None,
            archive_id: ArchiveId::nil(),
            manifest: crate::core::types::ManifestRef {
                id: ManifestId::nil(),
                root_node: NodeId::nil(),
                chunk_count: 0,
                total_size: 0,
                created_at: Utc::now(),
            },
            timestamp: Utc::now() - chrono::Duration::days(days_ago),
            labels: label_map,
            incremental: false,
        }
    }

    #[test]
    fn retention_under_limit_keeps_all() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let snapshots = vec![
            make_snapshot("00000000-0000-0000-0000-000000000001", 1, vec![]),
            make_snapshot("00000000-0000-0000-0000-000000000002", 2, vec![]),
        ];
        let eligible = engine.evaluate_retention(&snapshots).unwrap();
        assert!(eligible.is_empty());
    }

    #[test]
    fn retention_removes_oldest_beyond_limit() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let mut snapshots: Vec<Snapshot> = (0..35)
            .map(|i| {
                make_snapshot(
                    &format!("00000000-0000-0000-0000-{:012x}", i),
                    (i + 30) as i64,
                    vec![],
                )
            })
            .collect();
        let eligible = engine.evaluate_retention(&snapshots).unwrap();
        assert_eq!(eligible.len(), 5);
    }

    #[test]
    fn retention_tags_protected() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let snapshots = vec![
            make_snapshot("00000000-0000-0000-0000-000000000001", 100, vec![("tag", "weekly")]),
            make_snapshot("00000000-0000-0000-0000-000000000002", 50, vec![]),
            make_snapshot("00000000-0000-0000-0000-000000000003", 60, vec![]),
            make_snapshot("00000000-0000-0000-0000-000000000004", 70, vec![]),
            make_snapshot("00000000-0000-0000-0000-000000000005", 80, vec![]),
        ];
        let eligible = engine.evaluate_retention(&snapshots).unwrap();
        assert!(!eligible.contains(
            &SnapshotId::from_uuid(
                uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
            )
        ));
    }

    #[test]
    fn retention_min_age_protects_recent() {
        let config = PolicyConfig {
            retention: RetentionPolicy {
                max_snapshots: 1,
                min_age_days: 30,
                tags_keep: vec![],
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngineImpl::new(config);
        let snapshots = vec![
            make_snapshot("00000000-0000-0000-0000-000000000001", 5, vec![]),
            make_snapshot("00000000-0000-0000-0000-000000000002", 10, vec![]),
        ];
        let eligible = engine.evaluate_retention(&snapshots).unwrap();
        assert!(eligible.is_empty());
    }

    #[test]
    fn gc_unreferenced_chunks_identified() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let all_chunks: Vec<ChunkId> = (0..10)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i;
                ChunkId::from_bytes(bytes)
            })
            .collect();
        let referenced: Vec<ChunkId> = all_chunks.iter().take(3).copied().collect();
        let orphaned = engine.evaluate_gc(&all_chunks, &referenced).unwrap();
        assert_eq!(orphaned.len(), 7);
    }

    #[test]
    fn gc_all_referenced_none_orphaned() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let chunks: Vec<ChunkId> = (0..5)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i;
                ChunkId::from_bytes(bytes)
            })
            .collect();
        let orphaned = engine.evaluate_gc(&chunks, &chunks).unwrap();
        assert!(orphaned.is_empty());
    }

    #[test]
    fn gc_aggressive_mode_returns_all() {
        let config = PolicyConfig {
            gc: GcPolicy {
                aggressive: true,
                ..GcPolicy::default()
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngineImpl::new(config);
        let all_chunks: Vec<ChunkId> = (0..10)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i;
                ChunkId::from_bytes(bytes)
            })
            .collect();
        let referenced: Vec<ChunkId> = all_chunks.iter().take(1).copied().collect();
        let orphaned = engine.evaluate_gc(&all_chunks, &referenced).unwrap();
        assert_eq!(orphaned.len(), 9);
    }

    #[test]
    fn gc_respects_max_unreferenced() {
        let config = PolicyConfig {
            gc: GcPolicy {
                aggressive: false,
                max_unreferenced_chunks: 3,
                ..GcPolicy::default()
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngineImpl::new(config);
        let all_chunks: Vec<ChunkId> = (0..20)
            .map(|i| {
                let mut bytes = [0u8; 32];
                bytes[0] = i;
                ChunkId::from_bytes(bytes)
            })
            .collect();
        let referenced: Vec<ChunkId> = vec![];
        let orphaned = engine.evaluate_gc(&all_chunks, &referenced).unwrap();
        assert_eq!(orphaned.len(), 3);
    }

    #[test]
    fn requirements_sealed_archive_passes() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let archive = Archive {
            id: ArchiveId::nil(),
            name: "test".into(),
            manifest: ManifestRef {
                id: ManifestId::nil(),
                root_node: NodeId::nil(),
                chunk_count: 0,
                total_size: 0,
                created_at: Utc::now(),
            },
            encrypted: false,
            compression: CompressionAlgorithm::None,
            created_at: Utc::now(),
            sealed: true,
        };
        assert!(engine.meets_requirements(&archive).unwrap());
    }

    #[test]
    fn requirements_unsealed_compression_none_ok() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let archive = Archive {
            id: ArchiveId::nil(),
            name: "test".into(),
            manifest: ManifestRef {
                id: ManifestId::nil(),
                root_node: NodeId::nil(),
                chunk_count: 0,
                total_size: 0,
                created_at: Utc::now(),
            },
            encrypted: false,
            compression: CompressionAlgorithm::None,
            created_at: Utc::now(),
            sealed: false,
        };
        assert!(engine.meets_requirements(&archive).unwrap());
    }

    #[test]
    fn requirements_unsealed_zstd_zero_fails() {
        let engine = PolicyEngineImpl::new(PolicyConfig::default());
        let archive = Archive {
            id: ArchiveId::nil(),
            name: "test".into(),
            manifest: ManifestRef {
                id: ManifestId::nil(),
                root_node: NodeId::nil(),
                chunk_count: 0,
                total_size: 0,
                created_at: Utc::now(),
            },
            encrypted: false,
            compression: CompressionAlgorithm::Zstd(0),
            created_at: Utc::now(),
            sealed: false,
        };
        assert!(!engine.meets_requirements(&archive).unwrap());
    }

    #[test]
    fn config_defaults_are_sane() {
        let config = PolicyConfig::default();
        assert_eq!(config.retention.max_snapshots, 30);
        assert_eq!(config.retention.min_age_days, 7);
        assert_eq!(config.gc.min_chunk_age_hours, 24);
        assert!(!config.gc.aggressive);
        assert!((config.compression_min_savings_percent - 10.0).abs() < 1e-9);
    }
}
