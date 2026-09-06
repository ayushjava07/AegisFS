//! Unit and integration tests for content-addressable artifact storage.

use std::fs;

use super::artifacts::*;

#[test]
fn memory_artifact_store_put_get_describe_and_deduplicate() {
    let store = MemoryArtifactStore::new();
    let data = b"sample output report content";

    let desc1 = store
        .put("tenant_a", "report.txt", data, "text/plain", 1000)
        .expect("put artifact");
    assert_eq!(desc1.size_bytes, data.len());

    // Deduplication: same content generates identical SHA256 ID
    let desc2 = store
        .put("tenant_a", "report_copy.txt", data, "text/plain", 1001)
        .expect("put duplicate artifact");
    assert_eq!(desc1.id, desc2.id);

    // Get content back
    let fetched = store.get(&desc1.id).expect("get").expect("exists");
    assert_eq!(fetched, data);

    // Describe
    let desc = store.describe(&desc1.id).unwrap().unwrap();
    assert_eq!(desc.id, desc1.id);
    assert_eq!(desc.tenant, "tenant_a");

    // List by tenant
    let list = store.list_by_tenant("tenant_a").unwrap();
    assert_eq!(list.len(), 1);

    // Delete
    assert!(store.delete(&desc1.id).unwrap());
    assert_eq!(store.get(&desc1.id).unwrap(), None);
}

#[test]
fn disk_artifact_store_sharding_and_integrity_verification() {
    let dir = std::env::temp_dir().join(format!("runvane_artifact_test_{}", std::process::id()));
    let store = DiskArtifactStore::open(&dir).expect("open disk store");

    let payload = b"critical workflow payload data 12345";
    let desc = store
        .put(
            "acme",
            "payload.bin",
            payload,
            "application/octet-stream",
            2000,
        )
        .expect("put disk artifact");

    // Fetch and verify content
    let retrieved = store.get(&desc.id).expect("get").expect("exists");
    assert_eq!(retrieved, payload);

    // Verify file exists on disk with sharding
    let (prefix1, remainder) = desc.id.split_at(2);
    let (prefix2, _) = remainder.split_at(2);
    let expected_file = dir.join(prefix1).join(prefix2).join(&desc.id);
    assert!(expected_file.exists());

    // Tamper with file to test integrity detection
    fs::write(&expected_file, b"corrupted bytes").unwrap();
    let err = store.get(&desc.id).unwrap_err();
    assert!(matches!(err, ArtifactError::IntegrityMismatch { .. }));

    let _ = fs::remove_dir_all(&dir);
}
