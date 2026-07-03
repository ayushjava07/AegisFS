use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::future::BoxFuture;
use futures::FutureExt;
use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tracing::info;

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::{JournalHandler, JournalStore};
use crate::core::types::*;

fn compute_entry_checksum(kind: &JournalEntryKind, data: &[u8]) -> AegisResult<HashValue> {
    let kind_bytes = bincode::serialize(kind)
        .map_err(|e| AegisError::SerializationError(e.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(&kind_bytes);
    hasher.update(data);
    Ok(HashValue::from_bytes(hasher.finalize().into()))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyncMode {
    Sync,
    #[default]
    Async,
    Batch,
}

#[derive(Debug, Clone)]
pub struct JournalConfig {
    pub max_entries: usize,
    pub flush_interval: Duration,
    pub sync_mode: SyncMode,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            max_entries: 10000,
            flush_interval: Duration::from_millis(100),
            sync_mode: SyncMode::default(),
        }
    }
}

impl JournalConfig {
    pub fn new(max_entries: usize, flush_interval: Duration, sync_mode: SyncMode) -> Self {
        Self {
            max_entries,
            flush_interval,
            sync_mode,
        }
    }
}

struct MemoryJournalInner {
    entries: Vec<JournalEntry>,
    next_sequence: u64,
}

pub struct MemoryJournal {
    inner: Arc<RwLock<MemoryJournalInner>>,
}

impl MemoryJournal {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryJournalInner {
                entries: Vec::new(),
                next_sequence: 1,
            })),
        }
    }

    pub fn new_with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryJournalInner {
                entries: Vec::with_capacity(capacity),
                next_sequence: 1,
            })),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().entries.is_empty()
    }

    pub fn entries(&self) -> Vec<JournalEntry> {
        self.inner.read().entries.clone()
    }
}

impl Default for MemoryJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl JournalStore for MemoryJournal {
    fn append(&self, mut entry: JournalEntry) -> BoxFuture<'_, AegisResult<u64>> {
        let result = {
            let mut inner = self.inner.write();
            let seq = inner.next_sequence;
            entry.sequence = seq;
            entry.timestamp = Utc::now();
            let checksum = match compute_entry_checksum(&entry.kind, &entry.data) {
                Ok(c) => c,
                Err(e) => return async move { Err(e) }.boxed(),
            };
            entry.checksum = checksum;
            inner.entries.push(entry);
            inner.next_sequence += 1;
            Ok(seq)
        };
        async move { result }.boxed()
    }

    fn read_after(&self, sequence: u64, limit: usize) -> BoxFuture<'_, AegisResult<Vec<JournalEntry>>> {
        let entries = {
            let inner = self.inner.read();
            inner
                .entries
                .iter()
                .filter(|e| e.sequence > sequence)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>()
        };
        async move { Ok(entries) }.boxed()
    }

    fn latest_sequence(&self) -> BoxFuture<'_, AegisResult<u64>> {
        let seq = {
            let inner = self.inner.read();
            inner
                .entries
                .last()
                .map(|e| e.sequence)
                .unwrap_or(0)
        };
        async move { Ok(seq) }.boxed()
    }

    fn truncate(&self, before_sequence: u64) -> BoxFuture<'_, AegisResult<()>> {
        let mut inner = self.inner.write();
        inner.entries.retain(|e| e.sequence >= before_sequence);
        async move { Ok(()) }.boxed()
    }

    fn replay(&self, handler: Box<dyn JournalHandler + Send>) -> BoxFuture<'_, AegisResult<u64>> {
        let entries = {
            let inner = self.inner.read();
            inner.entries.clone()
        };
        async move {
            let mut h = handler;
            for entry in &entries {
                h.handle(entry).map_err(|e| {
                    AegisError::JournalReplayFailed {
                        sequence: entry.sequence,
                        message: e.to_string(),
                    }
                })?;
            }
            Ok(entries.len() as u64)
        }.boxed()
    }
}

pub fn append_entry_to<S: JournalStore + ?Sized>(
    store: &S,
    kind: JournalEntryKind,
    data: Vec<u8>,
) -> BoxFuture<'_, AegisResult<u64>> {
    let entry = JournalEntry {
        sequence: 0,
        timestamp: Utc::now(),
        kind,
        data,
        checksum: HashValue::nil(),
    };
    store.append(entry)
}

pub fn replay_entries(
    entries: &[JournalEntry],
    handler: &mut impl JournalHandler,
) -> AegisResult<u64> {
    for entry in entries {
        handler.handle(entry).map_err(|e| {
            AegisError::JournalReplayFailed {
                sequence: entry.sequence,
                message: e.to_string(),
            }
        })?;
    }
    Ok(entries.last().map(|e| e.sequence).unwrap_or(0))
}

pub struct JournalPlayer<S: JournalStore> {
    store: Arc<S>,
}

impl<S: JournalStore> JournalPlayer<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    pub fn play<H: JournalHandler + Send + 'static>(
        &self,
        handler: H,
    ) -> BoxFuture<'_, AegisResult<u64>> {
        self.store.replay(Box::new(handler))
    }

    pub fn store_ref(&self) -> &Arc<S> {
        &self.store
    }
}

pub fn verify_entry_checksum(entry: &JournalEntry) -> AegisResult<()> {
    let computed = compute_entry_checksum(&entry.kind, &entry.data)?;
    if computed != entry.checksum {
        return Err(AegisError::ChecksumMismatch {
            expected: entry.checksum.to_hex(),
            actual: computed.to_hex(),
        });
    }
    Ok(())
}

pub fn verify_entries_checksums(entries: &[JournalEntry]) -> AegisResult<()> {
    for entry in entries {
        verify_entry_checksum(entry)?;
    }
    Ok(())
}

pub struct WalWriter<S: JournalStore> {
    store: Arc<S>,
    buffer: Vec<JournalEntry>,
    config: JournalConfig,
    current_sequence: u64,
}

impl<S: JournalStore> WalWriter<S> {
    pub fn new(store: Arc<S>, config: JournalConfig) -> Self {
        let capacity = config.max_entries.clamp(64, 4096);
        Self {
            store,
            buffer: Vec::with_capacity(capacity),
            config,
            current_sequence: 0,
        }
    }

    pub fn with_capacity(store: Arc<S>, config: JournalConfig, capacity: usize) -> Self {
        Self {
            store,
            buffer: Vec::with_capacity(capacity),
            config,
            current_sequence: 0,
        }
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn current_sequence(&self) -> u64 {
        self.current_sequence
    }

    pub fn config(&self) -> &JournalConfig {
        &self.config
    }

    pub async fn append(&mut self, kind: JournalEntryKind, data: Vec<u8>) -> AegisResult<()> {
        let entry = JournalEntry {
            sequence: 0,
            timestamp: Utc::now(),
            kind,
            data,
            checksum: HashValue::nil(),
        };
        self.buffer.push(entry);
        if self.buffer.len() >= self.config.max_entries {
            self.flush().await?;
        }
        Ok(())
    }

    pub async fn append_entry(&mut self, entry: JournalEntry) -> AegisResult<()> {
        self.buffer.push(entry);
        if self.buffer.len() >= self.config.max_entries {
            self.flush().await?;
        }
        Ok(())
    }

    pub async fn flush(&mut self) -> AegisResult<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let entries = std::mem::take(&mut self.buffer);
        let batch_size = entries.len();
        for entry in entries {
            let seq = self.store.append(entry).await?;
            self.current_sequence = seq;
        }
        if batch_size > 0 {
            info!("Flushed {} journal entries (seq={})", batch_size, self.current_sequence);
        }
        Ok(())
    }

    pub async fn flush_and_sync(&mut self) -> AegisResult<()> {
        let result = self.flush().await;
        info!("Journal sync complete at sequence {}", self.current_sequence);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CollectingHandler {
        entries: Vec<JournalEntry>,
        fail_on: Option<u64>,
    }

    impl CollectingHandler {
        fn new() -> Self {
            Self::default()
        }

        fn with_failure(seq: u64) -> Self {
            Self {
                entries: Vec::new(),
                fail_on: Some(seq),
            }
        }
    }

    impl JournalHandler for CollectingHandler {
        fn handle(&mut self, entry: &JournalEntry) -> AegisResult<()> {
            if let Some(fail_seq) = self.fail_on {
                if entry.sequence == fail_seq {
                    return Err(AegisError::JournalError("test failure".into()));
                }
            }
            self.entries.push(entry.clone());
            Ok(())
        }
    }

    fn create_entry(kind: JournalEntryKind, data: Vec<u8>) -> JournalEntry {
        JournalEntry {
            sequence: 0,
            timestamp: Utc::now(),
            kind,
            data,
            checksum: HashValue::nil(),
        }
    }

    #[tokio::test]
    async fn test_basic_append_and_read() {
        let journal = MemoryJournal::new();
        let seq1 = journal
            .append(create_entry(JournalEntryKind::CreateNode, b"node1".to_vec()))
            .await
            .unwrap();
        assert_eq!(seq1, 1);

        let seq2 = journal
            .append(create_entry(JournalEntryKind::DeleteNode, b"node2".to_vec()))
            .await
            .unwrap();
        assert_eq!(seq2, 2);

        let entries = journal.read_after(0, 10).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[1].sequence, 2);
        assert_eq!(entries[0].kind, JournalEntryKind::CreateNode);
        assert_eq!(entries[1].kind, JournalEntryKind::DeleteNode);
        assert_eq!(entries[0].data, b"node1");
        assert_eq!(entries[1].data, b"node2");
    }

    #[tokio::test]
    async fn test_sequence_number_ordering() {
        let journal = MemoryJournal::new();
        for i in 1..=10 {
            let seq = journal
                .append(create_entry(JournalEntryKind::Checkpoint, vec![i as u8]))
                .await
                .unwrap();
            assert_eq!(seq, i as u64);
        }

        let entries = journal.read_after(0, 100).await.unwrap();
        assert_eq!(entries.len(), 10);
        for (idx, entry) in entries.iter().enumerate() {
            assert_eq!(entry.sequence, (idx + 1) as u64);
        }
    }

    #[tokio::test]
    async fn test_truncation() {
        let journal = MemoryJournal::new();
        for i in 1..=5 {
            journal
                .append(create_entry(JournalEntryKind::Checkpoint, vec![i]))
                .await
                .unwrap();
        }

        assert_eq!(journal.len(), 5);

        journal.truncate(3).await.unwrap();
        assert_eq!(journal.len(), 3);

        let entries = journal.read_after(0, 10).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sequence, 3);
        assert_eq!(entries[1].sequence, 4);
        assert_eq!(entries[2].sequence, 5);
    }

    #[tokio::test]
    async fn test_truncate_all() {
        let journal = MemoryJournal::new();
        for i in 1..=3 {
            journal
                .append(create_entry(JournalEntryKind::Checkpoint, vec![i]))
                .await
                .unwrap();
        }

        journal.truncate(10).await.unwrap();
        assert!(journal.is_empty());

        let entries = journal.read_after(0, 10).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_replay() {
        let journal = Arc::new(MemoryJournal::new());
        for i in 1..=5 {
            journal
                .append(create_entry(JournalEntryKind::CreateNode, vec![i]))
                .await
                .unwrap();
        }            let handler = CollectingHandler::new();
        let last_seq = journal.replay(Box::new(handler)).await.unwrap();
        assert_eq!(last_seq, 5);
    }

    #[tokio::test]
    async fn test_replay_via_player() {
        let journal = MemoryJournal::new();
        for i in 1..=3 {
            journal
                .append(create_entry(JournalEntryKind::Checkpoint, vec![i]))
                .await
                .unwrap();
        }

        let player = JournalPlayer::new(Arc::new(journal));
        let handler = CollectingHandler::new();
        let last_seq = player.play(handler).await.unwrap();
        assert_eq!(last_seq, 3);
    }

    #[tokio::test]
    async fn test_replay_with_failure() {
        let journal = MemoryJournal::new();
        for i in 1..=5 {
            journal
                .append(create_entry(JournalEntryKind::CreateNode, vec![i]))
                .await
                .unwrap();
        }

        let handler = CollectingHandler::with_failure(3);
        let result = journal.replay(Box::new(handler)).await;
        assert!(result.is_err());
        match result {
            Err(AegisError::JournalReplayFailed { sequence, .. }) => {
                assert_eq!(sequence, 3);
            }
            _ => panic!("Expected JournalReplayFailed error"),
        }
    }

    #[tokio::test]
    async fn test_checksum_verification() {
        let journal = MemoryJournal::new();
        let seq = journal
            .append(create_entry(JournalEntryKind::CreateNode, b"data".to_vec()))
            .await
            .unwrap();

        let entries = journal.read_after(0, 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sequence, seq);

        verify_entry_checksum(&entries[0]).unwrap();

        let mut corrupted = entries[0].clone();
        corrupted.checksum = HashValue::nil();
        assert!(verify_entry_checksum(&corrupted).is_err());
    }

    #[tokio::test]
    async fn test_empty_journal() {
        let journal = MemoryJournal::new();
        assert!(journal.is_empty());

        let entries = journal.read_after(0, 10).await.unwrap();
        assert!(entries.is_empty());

        let seq = journal.latest_sequence().await.unwrap();
        assert_eq!(seq, 0);

        let handler = CollectingHandler::new();
        let last_seq = journal.replay(Box::new(handler)).await.unwrap();
        assert_eq!(last_seq, 0);
    }

    #[tokio::test]
    async fn test_large_batch() {
        let journal = MemoryJournal::new_with_capacity(1000);
        let count = 500;
        for i in 0..count {
            let data = vec![(i % 256) as u8; 64];
            let seq = journal
                .append(create_entry(JournalEntryKind::Checkpoint, data))
                .await
                .unwrap();
            assert_eq!(seq, (i + 1) as u64);
        }

        assert_eq!(journal.len(), count);

        let entries = journal.read_after(0, count).await.unwrap();
        assert_eq!(entries.len(), count);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[count - 1].sequence, count as u64);
    }

    #[tokio::test]
    async fn test_truncate_followed_by_append() {
        let journal = MemoryJournal::new();
        for i in 1..=3 {
            journal
                .append(create_entry(JournalEntryKind::CreateNode, vec![i]))
                .await
                .unwrap();
        }

        journal.truncate(2).await.unwrap();
        assert_eq!(journal.len(), 2);

        let seq = journal
            .append(create_entry(JournalEntryKind::DeleteNode, b"new".to_vec()))
            .await
            .unwrap();
        assert_eq!(seq, 4);

        let entries = journal.read_after(0, 10).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sequence, 2);
        assert_eq!(entries[1].sequence, 3);
        assert_eq!(entries[2].sequence, 4);
        assert_eq!(entries[2].kind, JournalEntryKind::DeleteNode);
    }

    #[tokio::test]
    async fn test_read_after_limit() {
        let journal = MemoryJournal::new();
        for i in 1..=10 {
            journal
                .append(create_entry(JournalEntryKind::Checkpoint, vec![i]))
                .await
                .unwrap();
        }

        let entries = journal.read_after(5, 3).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sequence, 6);
        assert_eq!(entries[1].sequence, 7);
        assert_eq!(entries[2].sequence, 8);
    }

    #[tokio::test]
    async fn test_replay_entries_fn() {
        let entries = vec![
            JournalEntry {
                sequence: 1,
                timestamp: Utc::now(),
                kind: JournalEntryKind::CreateNode,
                data: b"a".to_vec(),
                checksum: HashValue::nil(),
            },
            JournalEntry {
                sequence: 2,
                timestamp: Utc::now(),
                kind: JournalEntryKind::DeleteNode,
                data: b"b".to_vec(),
                checksum: HashValue::nil(),
            },
        ];

        let mut handler = CollectingHandler::new();
        let last_seq = replay_entries(&entries, &mut handler).unwrap();
        assert_eq!(last_seq, 2);
        assert_eq!(handler.entries.len(), 2);
    }

    #[tokio::test]
    async fn test_wal_writer() {
        let store = Arc::new(MemoryJournal::new());
        let config = JournalConfig::new(5, Duration::from_millis(10), SyncMode::Batch);
        let mut writer = WalWriter::new(store.clone(), config);

        assert_eq!(writer.buffer_len(), 0);
        assert_eq!(writer.current_sequence(), 0);

        writer
            .append(JournalEntryKind::CreateNode, b"test".to_vec())
            .await
            .unwrap();
        assert_eq!(writer.buffer_len(), 1);
        assert_eq!(store.len(), 0);

        writer.flush().await.unwrap();
        assert_eq!(writer.buffer_len(), 0);
        assert_eq!(store.len(), 1);
        assert!(writer.current_sequence() > 0);
    }

    #[tokio::test]
    async fn test_wal_writer_auto_flush() {
        let store = Arc::new(MemoryJournal::new());
        let config = JournalConfig::new(3, Duration::from_millis(10), SyncMode::Batch);
        let mut writer = WalWriter::new(store.clone(), config);

        writer
            .append(JournalEntryKind::Checkpoint, b"1".to_vec())
            .await
            .unwrap();
        writer
            .append(JournalEntryKind::Checkpoint, b"2".to_vec())
            .await
            .unwrap();
        assert_eq!(store.len(), 0);

        writer
            .append(JournalEntryKind::Checkpoint, b"3".to_vec())
            .await
            .unwrap();
        assert_eq!(store.len(), 3);

        assert_eq!(store.read_after(0, 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_latest_sequence() {
        let journal = MemoryJournal::new();
        assert_eq!(journal.latest_sequence().await.unwrap(), 0);

        journal
            .append(create_entry(JournalEntryKind::Checkpoint, b"a".to_vec()))
            .await
            .unwrap();
        assert_eq!(journal.latest_sequence().await.unwrap(), 1);

        journal
            .append(create_entry(JournalEntryKind::Checkpoint, b"b".to_vec()))
            .await
            .unwrap();
        assert_eq!(journal.latest_sequence().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_append_entry_helper() {
        let journal = MemoryJournal::new();
        let seq = append_entry_to(
            &journal,
            JournalEntryKind::CreateSnapshot,
            b"snap1".to_vec(),
        )
        .await
        .unwrap();
        assert_eq!(seq, 1);

        let entries = journal.read_after(0, 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, JournalEntryKind::CreateSnapshot);
        assert_eq!(entries[0].data, b"snap1");
    }

    #[tokio::test]
    async fn test_verify_multiple_entries() {
        let journal = MemoryJournal::new();
        for i in 0..5 {
            journal
                .append(create_entry(
                    JournalEntryKind::ConfigChange,
                    format!("config{}", i).into_bytes(),
                ))
                .await
                .unwrap();
        }

        let entries = journal.read_after(0, 10).await.unwrap();
        verify_entries_checksums(&entries).unwrap();
    }

    #[tokio::test]
    async fn test_journal_player_store_ref() {
        let journal = MemoryJournal::new();
        let _seq = journal
            .append(create_entry(JournalEntryKind::Checkpoint, vec![]))
            .await
            .unwrap();

        let player = JournalPlayer::new(Arc::new(journal));

        let handler = CollectingHandler::new();
        let _ = player.play(handler).await.unwrap();

        assert!(player.store_ref().latest_sequence().await.unwrap() >= 1);
    }
}
