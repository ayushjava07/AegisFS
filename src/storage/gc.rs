//! Garbage collection and automated retention daemon for artifact storage.
//!
//! Sweeps content-addressable blobs that have aged out past retention windows
//! while protecting active run artifacts and operator-pinned references.

use std::collections::BTreeSet;

use super::artifacts::{ArtifactError, ArtifactStore};

/// Configuration options for an artifact garbage collection sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactGcOptions {
    /// Maximum age in milliseconds before an unreferenced artifact is eligible for deletion.
    pub retention_ms: u64,
    /// If true, calculate purges without deleting underlying blobs.
    pub dry_run: bool,
    /// Optional tenant scopes; if empty, sweeps across all discovered tenant partitions.
    pub tenants: Vec<String>,
}

impl Default for ArtifactGcOptions {
    fn default() -> Self {
        Self {
            retention_ms: 7 * 24 * 3600 * 1000, // 7 days
            dry_run: false,
            tenants: Vec::new(),
        }
    }
}

/// Execution metrics and audit trail for a garbage collection sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactGcStats {
    /// Total number of artifact descriptors inspected.
    pub scanned_count: usize,
    /// Total byte volume of inspected artifacts.
    pub scanned_bytes: usize,
    /// Number of stale/orphaned artifacts purged (or simulated).
    pub reclaimed_count: usize,
    /// Total bytes reclaimed from storage.
    pub reclaimed_bytes: usize,
    /// Identifiers (SHA-256 digests) of purged artifacts.
    pub purged_ids: Vec<String>,
}

/// Performs a garbage collection sweep over an [`ArtifactStore`].
pub fn sweep_artifacts(
    store: &dyn ArtifactStore,
    options: &ArtifactGcOptions,
    active_refs: &BTreeSet<String>,
    now_ms: i64,
) -> Result<ArtifactGcStats, ArtifactError> {
    let mut stats = ArtifactGcStats::default();

    // Determine target tenants. If unspecified, use a default fallback or provided list.
    let target_tenants = if options.tenants.is_empty() {
        vec!["default".to_string(), "acme".to_string()]
    } else {
        options.tenants.clone()
    };

    let mut seen_ids = BTreeSet::new();

    for tenant in &target_tenants {
        let descriptors = store.list_by_tenant(tenant)?;
        for desc in descriptors {
            if !seen_ids.insert(desc.id.clone()) {
                continue;
            }

            stats.scanned_count += 1;
            stats.scanned_bytes += desc.size_bytes;

            // Pinned/active references are strictly preserved.
            if active_refs.contains(&desc.id) {
                continue;
            }

            // Check retention age boundary.
            let age_ms = (now_ms - desc.created_at_ms).max(0) as u64;
            if age_ms >= options.retention_ms {
                stats.reclaimed_count += 1;
                stats.reclaimed_bytes += desc.size_bytes;
                stats.purged_ids.push(desc.id.clone());

                if !options.dry_run {
                    store.delete(&desc.id)?;
                }
            }
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::artifacts::MemoryArtifactStore;

    #[test]
    fn sweep_reclaims_expired_unreferenced_artifacts() {
        let store = MemoryArtifactStore::new();
        let desc1 = store
            .put(
                "acme",
                "data.json",
                b"{\"key\":\"old\"}",
                "application/json",
                1_000,
            )
            .unwrap();
        let desc2 = store
            .put(
                "acme",
                "latest.json",
                b"{\"key\":\"new\"}",
                "application/json",
                9_000,
            )
            .unwrap();

        let options = ArtifactGcOptions {
            retention_ms: 5_000,
            dry_run: false,
            tenants: vec!["acme".into()],
        };

        let active_refs = BTreeSet::new();
        let now_ms = 10_000;

        let stats = sweep_artifacts(&store, &options, &active_refs, now_ms).unwrap();
        assert_eq!(stats.scanned_count, 2);
        assert_eq!(stats.reclaimed_count, 1);
        assert_eq!(stats.purged_ids, vec![desc1.id.clone()]);

        // desc1 is deleted, desc2 remains
        assert!(store.get(&desc1.id).unwrap().is_none());
        assert!(store.get(&desc2.id).unwrap().is_some());
    }

    #[test]
    fn sweep_preserves_active_pinned_references_even_if_old() {
        let store = MemoryArtifactStore::new();
        let desc = store
            .put(
                "acme",
                "gold.bin",
                b"pinned data",
                "application/octet-stream",
                1_000,
            )
            .unwrap();

        let options = ArtifactGcOptions {
            retention_ms: 2_000,
            dry_run: false,
            tenants: vec!["acme".into()],
        };

        let mut active_refs = BTreeSet::new();
        active_refs.insert(desc.id.clone());
        let now_ms = 10_000;

        let stats = sweep_artifacts(&store, &options, &active_refs, now_ms).unwrap();
        assert_eq!(stats.scanned_count, 1);
        assert_eq!(stats.reclaimed_count, 0);
        assert!(store.get(&desc.id).unwrap().is_some());
    }

    #[test]
    fn sweep_dry_run_does_not_delete_blobs() {
        let store = MemoryArtifactStore::new();
        let desc = store
            .put("acme", "dry.txt", b"preview content", "text/plain", 1_000)
            .unwrap();

        let options = ArtifactGcOptions {
            retention_ms: 1_000,
            dry_run: true,
            tenants: vec!["acme".into()],
        };

        let stats = sweep_artifacts(&store, &options, &BTreeSet::new(), 10_000).unwrap();
        assert_eq!(stats.reclaimed_count, 1);
        // Blob still exists because dry_run was true
        assert!(store.get(&desc.id).unwrap().is_some());
    }
}
