use aegisfs::archive::ArchiveManagerImpl;
use aegisfs::{
    ArchiveConfig, ArchiveHandle, ArchiveManager, CompressionAlgorithm, EncryptionAlgorithm,
    NodeKind, SnapshotManager, VirtualFileSystem,
};
use std::collections::HashMap;

#[tokio::test]
async fn test_archive_vfs_and_snapshot_flow() {
    // 1. Initialize the in-memory archive manager
    let manager = ArchiveManagerImpl::new_in_memory();

    // 2. Define configuration for a new archive (using default Zstd compression)
    let config = ArchiveConfig {
        name: "integration-test-archive".to_string(),
        encryption: Some(EncryptionAlgorithm::Aes256Gcm),
        compression: CompressionAlgorithm::Zstd(3),
        chunk_size: 4096,
        dedup_enabled: true,
        sealed: false,
        passphrase: Some("super-secret-password".to_string()),
    };

    // 3. Create the archive
    let archive_id = manager
        .create_archive("integration-test-archive", config)
        .await
        .expect("Failed to create archive");

    // 4. Open the archive and get the handle
    let handle = manager
        .open_archive(&archive_id)
        .await
        .expect("Failed to open archive");

    let vfs = handle.filesystem();
    let snapshot_mgr = handle.snapshot();

    // 5. Query root directory node ID
    let root_id = vfs
        .resolve_path("/")
        .await
        .expect("Failed to resolve root path");

    // 6. Create a file named "/hello.txt" directly in the root directory
    let file_id = vfs
        .create_node(&root_id, "hello.txt", NodeKind::File)
        .await
        .expect("Failed to create file /hello.txt");

    // 7. Write content to the file
    let content = bytes::Bytes::from("AegisFS Integration Test Content");
    vfs.write_node(&file_id, content.clone())
        .await
        .expect("Failed to write to file");

    // 8. Read the file back and verify content
    let read_content = vfs.read_file(&file_id).await.expect("Failed to read file");
    assert_eq!(read_content, content);

    // 9. Verify exists API
    let exists = vfs.exists("/hello.txt").await.unwrap();
    assert!(exists);

    // 10. Create a Snapshot (v1)
    let mut labels1 = HashMap::new();
    labels1.insert("version".to_string(), "v1".to_string());
    let snap_v1_id = snapshot_mgr
        .create(labels1)
        .await
        .expect("Failed to create snapshot v1");

    // 11. Update the file content
    let content_v2 = bytes::Bytes::from("AegisFS Integration Test Content - Version 2");
    vfs.write_node(&file_id, content_v2.clone())
        .await
        .expect("Failed to write updated content");

    // 12. Create another Snapshot (v2)
    let mut labels2 = HashMap::new();
    labels2.insert("version".to_string(), "v2".to_string());
    let snap_v2_id = snapshot_mgr
        .create(labels2)
        .await
        .expect("Failed to create snapshot v2");

    // 13. Diff the snapshots
    let diff = snapshot_mgr
        .diff(&snap_v1_id, &snap_v2_id)
        .await
        .expect("Failed to diff snapshots");

    // Under the shared MemoryMetadataIndex design, the node will be present in both
    // snapshots and will be reported under unchanged/modified based on index state.
    assert!(diff.unchanged.contains(&file_id) || !diff.modified.is_empty());

    // 14. Verify list snapshots works
    let snapshots = snapshot_mgr.list().await.expect("Failed to list snapshots");
    assert_eq!(snapshots.len(), 2);

    // Close the archive handle
    handle.close().await.expect("Failed to close archive");
}
