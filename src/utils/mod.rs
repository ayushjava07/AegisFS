use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;
use crate::core::types::*;

pub struct BufferPool {
    pool: Arc<parking_lot::Mutex<Vec<Vec<u8>>>>,
    buffer_size: usize,
    max_pool_size: usize,
}

impl BufferPool {
    pub fn new(buffer_size: usize, max_pool_size: usize) -> Self {
        Self {
            pool: Arc::new(parking_lot::Mutex::new(Vec::with_capacity(max_pool_size))),
            buffer_size,
            max_pool_size,
        }
    }

    pub fn acquire(&self) -> Vec<u8> {
        let mut pool = self.pool.lock();
        pool.pop().unwrap_or_else(|| vec![0u8; self.buffer_size])
    }

    pub fn release(&self, mut buf: Vec<u8>) {
        if buf.len() != self.buffer_size {
            buf.resize(self.buffer_size, 0);
        }
        let mut pool = self.pool.lock();
        if pool.len() < self.max_pool_size {
            pool.push(buf);
        }
    }

    pub fn with_buffer<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = self.acquire();
        let result = f(&mut buf);
        self.release(buf);
        result
    }

    pub fn clear(&self) {
        self.pool.lock().clear();
    }

    pub fn available(&self) -> usize {
        self.pool.lock().len()
    }
}

pub struct RateLimiter {
    tokens: Arc<tokio::sync::Mutex<f64>>,
    capacity: f64,
    rate: f64,
    last_refill: Arc<tokio::sync::Mutex<Instant>>,
}

impl RateLimiter {
    pub fn new(rate: f64, capacity: f64) -> Self {
        Self {
            tokens: Arc::new(tokio::sync::Mutex::new(capacity)),
            capacity,
            rate,
            last_refill: Arc::new(tokio::sync::Mutex::new(Instant::now())),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let mut tokens = self.tokens.lock().await;
            let mut last_refill = self.last_refill.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(*last_refill).as_secs_f64();
            *last_refill = now;

            let mut available = *tokens + elapsed * self.rate;
            if available > self.capacity {
                available = self.capacity;
            }

            if available >= 1.0 {
                *tokens = available - 1.0;
                return;
            }

            let wait = Duration::from_secs_f64((1.0 - available) / self.rate);
            drop(tokens);
            drop(last_refill);
            tokio::time::sleep(wait).await;
        }
    }

    pub async fn try_acquire(&self) -> bool {
        let mut tokens = self.tokens.lock().await;
        let mut last_refill = self.last_refill.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill).as_secs_f64();
        *last_refill = now;

        let mut available = *tokens + elapsed * self.rate;
        if available > self.capacity {
            available = self.capacity;
        }

        if available >= 1.0 {
            *tokens = available - 1.0;
            true
        } else {
            *tokens = available;
            false
        }
    }
}

pub struct Throttle {
    limiter: Arc<RateLimiter>,
}

impl Throttle {
    pub fn new(max_rate: f64) -> Self {
        Self {
            limiter: Arc::new(RateLimiter::new(max_rate, max_rate)),
        }
    }

    pub async fn allow(&self) {
        self.limiter.acquire().await;
    }

    pub async fn try_allow(&self) -> bool {
        self.limiter.try_acquire().await
    }
}

pub struct Backoff {
    initial_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
    jitter: f64,
    attempts: u64,
    max_attempts: u64,
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: 0.1,
            attempts: 0,
            max_attempts: 10,
        }
    }

    pub fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    pub fn with_multiplier(mut self, mult: f64) -> Self {
        self.multiplier = mult;
        self
    }

    pub fn with_jitter(mut self, jitter: f64) -> Self {
        self.jitter = jitter;
        self
    }

    pub fn with_max_attempts(mut self, max: u64) -> Self {
        self.max_attempts = max;
        self
    }

    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempts >= self.max_attempts {
            return None;
        }
        self.attempts += 1;

        let base = self
            .initial_delay
            .as_secs_f64()
            * self.multiplier.powi((self.attempts - 1) as i32);
        let base = base.min(self.max_delay.as_secs_f64());

        let jitter_range = base * self.jitter;
        let jitter = if jitter_range > 0.0 {
            rand::random::<f64>() * jitter_range - jitter_range / 2.0
        } else {
            0.0
        };

        let delay = (base + jitter).max(0.0);
        Some(Duration::from_secs_f64(delay))
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
    }

    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    pub fn max_attempts(&self) -> u64 {
        self.max_attempts
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Retry {
    backoff: Backoff,
}

impl Retry {
    pub fn new(backoff: Backoff) -> Self {
        Self { backoff }
    }

    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> AegisResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = AegisResult<T>>,
    {
        let mut backoff = Backoff::new()
            .with_initial_delay(self.backoff.initial_delay)
            .with_max_delay(self.backoff.max_delay)
            .with_multiplier(self.backoff.multiplier)
            .with_jitter(self.backoff.jitter)
            .with_max_attempts(self.backoff.max_attempts);

        let mut _last_error = None;
        loop {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    _last_error = Some(e);
                    match backoff.next_delay() {
                        Some(delay) => tokio::time::sleep(delay).await,
                        None => break,
                    }
                }
            }
        }
        Err(_last_error.unwrap_or_else(|| AegisError::Internal("retry exhausted".into())))
    }
}

pub struct PathUtil;

impl PathUtil {
    pub fn join(base: &str, name: &str) -> String {
        let base = base.trim_end_matches('/');
        let name = name.trim_start_matches('/');
        if base.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", base, name)
        }
    }

    pub fn normalize(path: &str) -> String {
        let mut segments: Vec<&str> = Vec::new();
        for segment in path.split('/') {
            match segment {
                "." | "" => {}
                ".." => {
                    segments.pop();
                }
                _ => {
                    segments.push(segment);
                }
            }
        }
        let normalized = segments.join("/");
        if path.starts_with('/') {
            format!("/{}", normalized)
        } else {
            normalized
        }
    }

    pub fn parent(path: &str) -> Option<String> {
        if path.is_empty() || path == "/" {
            return None;
        }
        let normalized = Self::normalize(path);
        let trimmed = normalized.trim_end_matches('/');
        if let Some(pos) = trimmed.rfind('/') {
            let parent = if pos == 0 { "/".to_string() } else { trimmed[..pos].to_string() };
            Some(parent)
        } else {
            None
        }
    }

    pub fn filename(path: &str) -> Option<&str> {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/" {
            return None;
        }
        trimmed.rsplit('/').next()
    }

    pub fn extension(path: &str) -> Option<&str> {
        let name = Self::filename(path)?;
        let pos = name.rfind('.')?;
        if pos == 0 || pos == name.len() - 1 {
            None
        } else {
            Some(&name[pos + 1..])
        }
    }
}

pub struct TimeUtil;

impl TimeUtil {
    pub fn format_duration(duration: Duration) -> String {
        let secs = duration.as_secs();
        if secs >= 86400 {
            format!("{}d {}h {}m {}s", secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60, secs % 60)
        } else if secs >= 3600 {
            format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
        } else if secs >= 60 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else if secs > 0 {
            format!("{}s", secs)
        } else {
            format!("{}ms", duration.subsec_millis())
        }
    }

    pub fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
        dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    }

    pub fn iso8601(dt: &chrono::DateTime<chrono::Utc>) -> String {
        dt.to_rfc3339()
    }
}

pub struct IdUtil;

impl IdUtil {
    pub fn new_node_id() -> NodeId { NodeId::new() }
    pub fn new_snapshot_id() -> SnapshotId { SnapshotId::new() }
    pub fn new_archive_id() -> ArchiveId { ArchiveId::new() }
    pub fn new_manifest_id() -> ManifestId { ManifestId::new() }
    pub fn new_task_id() -> TaskId { TaskId::new() }
    pub fn new_session_id() -> SessionId { SessionId::new() }
    pub fn chunk_id_from_data(data: &[u8]) -> ChunkId { ChunkId::from_data(data) }
    pub fn hash_from_data(data: &[u8]) -> HashValue { HashValue::sha256(data) }
}

pub struct MemoryBlock {
    pub data: Vec<u8>,
}

pub struct SimpleMemoryPool {
    pool: parking_lot::Mutex<Vec<MemoryBlock>>,
    block_size: usize,
    max_blocks: usize,
}

impl SimpleMemoryPool {
    pub fn new(block_size: usize, max_blocks: usize) -> Self {
        Self {
            pool: parking_lot::Mutex::new(Vec::with_capacity(max_blocks)),
            block_size,
            max_blocks,
        }
    }
}

use crate::core::types::MemoryBlock as CoreMemoryBlock;

impl MemoryPool for SimpleMemoryPool {
    fn allocate(&self, size: usize) -> AegisResult<CoreMemoryBlock> {
        let mut pool = self.pool.lock();
        if let Some(block) = pool.pop() {
            if block.data.len() >= size {
                return Ok(CoreMemoryBlock {
                    data: block.data.clone(),
                    size: block.data.len(),
                    pool_id: 0,
                });
            }
        }
        Ok(CoreMemoryBlock {
            data: vec![0u8; size],
            size,
            pool_id: 0,
        })
    }

    fn deallocate(&self, block: CoreMemoryBlock) {
        let mut pool = self.pool.lock();
        if pool.len() < self.max_blocks {
            pool.push(MemoryBlock { data: block.data });
        }
    }

    fn reset(&self) { self.pool.lock().clear(); }
    fn capacity(&self) -> usize { self.max_blocks * self.block_size }
    fn used(&self) -> usize { self.pool.lock().len() * self.block_size }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_buffer_pool_acquire_release() {
        let pool = BufferPool::new(64, 10);
        assert_eq!(pool.available(), 0);
        let buf = pool.acquire();
        assert_eq!(buf.len(), 64);
        pool.release(buf);
        assert_eq!(pool.available(), 1);
        let buf2 = pool.acquire();
        assert_eq!(buf2.len(), 64);
        assert_eq!(pool.available(), 0);
    }

    #[tokio::test]
    async fn test_buffer_pool_with_buffer() {
        let pool = BufferPool::new(32, 5);
        let result = pool.with_buffer(|buf| {
            assert_eq!(buf.len(), 32);
            buf[0] = 42;
            99
        });
        assert_eq!(result, 99);
        assert_eq!(pool.available(), 1);
    }

    #[tokio::test]
    async fn test_rate_limiter_try_acquire() {
        let limiter = RateLimiter::new(10.0, 5.0);
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        assert!(!limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_backoff_basic() {
        let mut backoff = Backoff::new()
            .with_initial_delay(Duration::from_millis(10))
            .with_max_delay(Duration::from_millis(100))
            .with_multiplier(2.0)
            .with_jitter(0.0)
            .with_max_attempts(5);

        assert_eq!(backoff.next_delay().unwrap().as_millis(), 10);
        assert_eq!(backoff.next_delay().unwrap().as_millis(), 20);
        assert_eq!(backoff.next_delay().unwrap().as_millis(), 40);
        assert_eq!(backoff.next_delay().unwrap().as_millis(), 80);
        assert_eq!(backoff.next_delay().unwrap().as_millis(), 100);
        assert!(backoff.next_delay().is_none());
    }

    #[tokio::test]
    async fn test_retry_success() {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let retry = Retry::new(Backoff::new().with_initial_delay(Duration::from_millis(1)).with_max_attempts(5));
        let count_clone = count.clone();
        let result = retry
            .execute(move || {
                let c = count_clone.clone();
                async move {
                    let val = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if val < 3 { Err(AegisError::Internal("not yet".into())) }
                    else { Ok(42) }
                }
            })
            .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let retry = Retry::new(Backoff::new().with_initial_delay(Duration::from_millis(1)).with_max_attempts(3));
        let result = retry
            .execute(|| async { Err::<(), _>(AegisError::Internal("fail".into())) })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_buffer_pool_clear() {
        let pool = BufferPool::new(16, 3);
        pool.release(vec![0u8; 16]);
        pool.release(vec![0u8; 16]);
        assert_eq!(pool.available(), 2);
        pool.clear();
        assert_eq!(pool.available(), 0);
    }

    #[tokio::test]
    async fn test_path_util() {
        assert_eq!(PathUtil::join("/a/b", "c"), "/a/b/c");
        assert_eq!(PathUtil::normalize("/a/b/../c"), "/a/c");
        assert_eq!(PathUtil::parent("/a/b/c").unwrap(), "/a/b");
        assert_eq!(PathUtil::filename("/a/b/file.txt"), Some("file.txt"));
        assert_eq!(PathUtil::extension("file.txt"), Some("txt"));
    }

    #[tokio::test]
    async fn test_id_util() {
        assert_ne!(IdUtil::new_node_id(), NodeId::nil());
        assert_ne!(IdUtil::new_snapshot_id(), SnapshotId::nil());
        assert!(!IdUtil::chunk_id_from_data(b"hello").is_nil());
        assert_ne!(IdUtil::hash_from_data(b"world"), HashValue::nil());
    }

    #[tokio::test]
    async fn test_simple_memory_pool() {
        let pool = SimpleMemoryPool::new(1024, 5);
        assert!(pool.capacity() > 0);
        let block = pool.allocate(256).unwrap();
        assert_eq!(block.data.len(), 256);
        pool.deallocate(block);
        pool.reset();
        assert_eq!(pool.used(), 0);
    }
}
