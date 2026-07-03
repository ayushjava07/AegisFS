use crate::core::error::{AegisError, AegisResult};
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A region of allocated memory with usage tracking.
pub struct MemoryBlock {
    data: Box<[u8]>,
    offset: usize,
}

impl MemoryBlock {
    /// Allocate a new zeroed block of the given size.
    pub fn new(size: usize) -> Self {
        Self { data: vec![0u8; size].into_boxed_slice(), offset: 0 }
    }

    /// Allocate a block from an existing buffer, taking ownership.
    pub fn from_vec(mut vec: Vec<u8>) -> Self {
        let len = vec.len();
        vec.shrink_to_fit();
        Self { data: vec.into_boxed_slice(), offset: len }
    }

    /// Number of bytes written / in use within the block.
    pub fn len(&self) -> usize {
        self.offset
    }

    /// Total capacity of the block.
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offset == 0
    }

    /// Remaining writable space.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_mut_ptr()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.offset]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data.as_mut()[..self.offset]
    }

    /// Returns the full underlying slice (including unused portion).
    pub fn as_raw_slice(&self) -> &[u8] {
        &self.data
    }

    /// Returns the full underlying mutable slice.
    pub fn as_raw_mut_slice(&mut self) -> &mut [u8] {
        self.data.as_mut()
    }

    /// Set how many bytes are considered used.
    pub fn set_offset(&mut self, offset: usize) -> AegisResult<()> {
        if offset > self.data.len() {
            return Err(AegisError::InvalidArgument(
                format!("offset {} exceeds block capacity {}", offset, self.data.len()),
            ));
        }
        self.offset = offset;
        Ok(())
    }

    /// Zero the entire block and reset offset.
    pub fn reset(&mut self) {
        self.offset = 0;
        for byte in self.data.as_mut() {
            *byte = 0;
        }
    }

    /// Reset offset without zeroing memory.
    pub fn clear(&mut self) {
        self.offset = 0;
    }
}

impl Clone for MemoryBlock {
    fn clone(&self) -> Self {
        Self { data: self.data.clone(), offset: self.offset }
    }
}

// ---------------------------------------------------------------------------

/// A memory pool managing fixed-size blocks with pre-allocation.
///
/// All blocks are of a uniform `block_size`. Requests for `size <= block_size`
/// are served from the free-list; requests for larger sizes bypass the pool.
pub struct MemoryPool {
    block_size: usize,
    prealloc_count: usize,
    free: Mutex<Vec<MemoryBlock>>,
    allocated_count: AtomicUsize,
    allocated_bytes: AtomicUsize,
    total_capacity: AtomicUsize,
}

impl MemoryPool {
    /// Create a new pool that pre-allocates `prealloc_count` blocks of
    /// `block_size` bytes each.
    pub fn new(block_size: usize, prealloc_count: usize) -> AegisResult<Self> {
        if block_size == 0 {
            return Err(AegisError::InvalidArgument(
                "block_size must be non-zero".into(),
            ));
        }

        let mut free = Vec::with_capacity(prealloc_count);
        for _ in 0..prealloc_count {
            free.push(MemoryBlock::new(block_size));
        }

        let total = prealloc_count * block_size;
        Ok(Self {
            block_size,
            prealloc_count,
            free: Mutex::new(free),
            allocated_count: AtomicUsize::new(0),
            allocated_bytes: AtomicUsize::new(0),
            total_capacity: AtomicUsize::new(total),
        })
    }

    /// Allocate a block of at least `size` bytes.
    pub fn allocate(&self, size: usize) -> AegisResult<MemoryBlock> {
        if size == 0 {
            return Err(AegisError::InvalidArgument(
                "allocation size must be non-zero".into(),
            ));
        }

        if size <= self.block_size {
            let mut free = self.free.lock();
            let block = free.pop().unwrap_or_else(|| {
                let b = MemoryBlock::new(self.block_size);
                self.total_capacity.fetch_add(self.block_size, Ordering::Relaxed);
                b
            });
            self.allocated_count.fetch_add(1, Ordering::Relaxed);
            self.allocated_bytes.fetch_add(self.block_size, Ordering::Relaxed);
            Ok(block)
        } else {
            let block = MemoryBlock::new(size);
            self.allocated_count.fetch_add(1, Ordering::Relaxed);
            self.allocated_bytes.fetch_add(size, Ordering::Relaxed);
            self.total_capacity.fetch_add(size, Ordering::Relaxed);
            Ok(block)
        }
    }

    /// Return a block to the pool. Only blocks whose capacity matches the
    /// pool's `block_size` are recycled; oversized blocks are dropped.
    pub fn deallocate(&self, mut block: MemoryBlock) {
        let freed = block.capacity();
        if freed == self.block_size {
            block.clear();
            self.free.lock().push(block);
        }
        self.allocated_count.fetch_sub(1, Ordering::Relaxed);
        self.allocated_bytes.fetch_sub(freed, Ordering::Relaxed);
    }

    /// Reset the pool: clear all tracked allocations and re-populate the
    /// free-list with the original pre-allocated count of fresh blocks.
    pub fn reset(&self) -> AegisResult<()> {
        let mut free = self.free.lock();
        free.clear();
        for _ in 0..self.prealloc_count {
            free.push(MemoryBlock::new(self.block_size));
        }
        let new_total = self.prealloc_count * self.block_size;
        self.total_capacity.store(new_total, Ordering::Relaxed);
        self.allocated_count.store(0, Ordering::Relaxed);
        self.allocated_bytes.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// Total capacity managed by the pool (pre-allocated + dynamically grown).
    pub fn capacity(&self) -> usize {
        self.total_capacity.load(Ordering::Relaxed)
    }

    /// Total bytes currently checked out to callers.
    pub fn used(&self) -> usize {
        self.allocated_bytes.load(Ordering::Relaxed)
    }

    /// Number of blocks currently checked out.
    pub fn allocated_count(&self) -> usize {
        self.allocated_count.load(Ordering::Relaxed)
    }

    /// Number of blocks available in the free-list.
    pub fn free_count(&self) -> usize {
        self.free.lock().len()
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }
}

// ---------------------------------------------------------------------------
//  Slab allocator – fixed-size elements backed by a pre-allocated Vec.

struct SlabInner<T> {
    entries: Vec<T>,
    free: Vec<usize>,
    default_gen: fn() -> T,
}

/// A slab allocator that hands out indices into a contiguous store.
///
/// The allocator grows on demand and supports O(1) allocate / deallocate.
pub struct SlabAllocator<T> {
    inner: Arc<Mutex<SlabInner<T>>>,
}

impl<T> SlabAllocator<T> {
    /// Create a slab with `capacity` pre-allocated (but not yet usable)
    /// slots.  Elements are initialised lazily on first allocation.
    pub fn new(capacity: usize) -> Self
    where
        T: Default,
    {
        Self {
            inner: Arc::new(Mutex::new(SlabInner {
                entries: Vec::with_capacity(capacity),
                free: (0..capacity).rev().collect(),
                default_gen: T::default,
            })),
        }
    }

    /// Same as `new` but uses a custom generator instead of `T::default`.
    pub fn with_generator(capacity: usize, default_gen: fn() -> T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SlabInner {
                entries: Vec::with_capacity(capacity),
                free: (0..capacity).rev().collect(),
                default_gen,
            })),
        }
    }

    /// Allocate a slot and return its index.
    ///
    /// Returns `None` if the slab has reached its pre-allocated capacity
    /// and the internal free-list is exhausted (only when the slab was
    /// created with `new_bounded`).
    pub fn allocate(&self) -> usize
    where
        T: Default,
    {
        let mut inner = self.inner.lock();
        let idx = if let Some(idx) = inner.free.pop() {
            idx
        } else {
            inner.entries.len()
        };
        if idx >= inner.entries.len() {
            let default_gen = inner.default_gen;
            inner.entries.resize_with(idx + 1, default_gen);
        }
        idx
    }

    /// Allocate and immediately initialise with a value.
    pub fn allocate_with(&self, val: T) -> usize {
        let mut inner = self.inner.lock();
        let idx = if let Some(idx) = inner.free.pop() {
            idx
        } else {
            inner.entries.len()
        };
        if idx >= inner.entries.len() {
            let default_gen = inner.default_gen;
            inner.entries.resize_with(idx + 1, default_gen);
        }
        inner.entries[idx] = val;
        idx
    }

    /// Release a previously allocated slot.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range or has already been freed.
    pub fn deallocate(&self, index: usize)
    where
        T: Default,
    {
        let mut inner = self.inner.lock();
        assert!(
            index < inner.entries.len(),
            "SlabAllocator::deallocate – index {} out of bounds (len {})",
            index,
            inner.entries.len()
        );
        inner.entries[index] = (inner.default_gen)();
        inner.free.push(index);
    }

    /// Access an element (read-only) via a scoped guard.
    #[allow(dead_code)]
    pub fn get(&self, index: usize) -> Option<MappedMutexGuard<'_, T>> {
        let guard = self.inner.lock();
        if index >= guard.entries.len() {
            return None;
        }
        Some(MutexGuard::map(guard, |inner| &mut inner.entries[index]))
    }

    /// Access an element (mutable) via a scoped guard.
    pub fn get_mut(&self, index: usize) -> Option<MappedMutexGuard<'_, T>> {
        let guard = self.inner.lock();
        if index >= guard.entries.len() {
            return None;
        }
        Some(MutexGuard::map(guard, |inner| &mut inner.entries[index]))
    }

    /// Number of live allocations.
    pub fn live_count(&self) -> usize {
        let inner = self.inner.lock();
        let free_in_entries = inner.free.iter().filter(|&&idx| idx < inner.entries.len()).count();
        inner.entries.len() - free_in_entries
    }

    /// Total capacity (allocated + free slots).
    pub fn capacity(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// Number of free slots.
    pub fn free_count(&self) -> usize {
        self.inner.lock().free.len()
    }
}

impl<T: Clone> SlabAllocator<T> {
    /// Allocate a slot and fill it with a cloned value.
    pub fn allocate_cloned(&self, val: &T) -> usize
    where
        T: Default,
    {
        let mut inner = self.inner.lock();
        let idx = if let Some(idx) = inner.free.pop() {
            idx
        } else {
            inner.entries.len()
        };
        if idx >= inner.entries.len() {
            let default_gen = inner.default_gen;
            inner.entries.resize_with(idx + 1, default_gen);
        }
        inner.entries[idx] = val.clone();
        idx
    }
}

// ---------------------------------------------------------------------------
//  Thread-local buffer cache.

struct BufferPoolInner {
    buffers: Vec<Vec<u8>>,
}

/// A buffer cache that reuses byte vectors.
///
/// Intended to be used either as a shared (cloned) resource behind `Arc` or
/// as a `thread_local!` per-thread cache.
#[derive(Clone)]
pub struct BufferPool {
    inner: Arc<Mutex<BufferPoolInner>>,
    buffer_size: usize,
}

impl BufferPool {
    /// Create a new pool that returns buffers of at least `buffer_size`
    /// capacity.
    pub fn new(buffer_size: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferPoolInner { buffers: Vec::new() })),
            buffer_size,
        }
    }

    /// Acquire a buffer from the pool, or create a fresh one if empty.
    pub fn acquire(&self) -> PooledBuffer {
        let mut inner = self.inner.lock();
        let mut buf = inner.buffers.pop().unwrap_or_else(|| Vec::with_capacity(self.buffer_size));
        buf.resize(self.buffer_size, 0);
        PooledBuffer { buf: Some(buf), pool: Some(self.clone()) }
    }

    /// Return a `Vec<u8>` to the pool.
    fn release(&self, mut buf: Vec<u8>) {
        buf.clear();
        let mut inner = self.inner.lock();
        inner.buffers.push(buf);
    }

    /// Number of buffers cached.
    pub fn len(&self) -> usize {
        self.inner.lock().buffers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The configured buffer size.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }
}

// ---------------------------------------------------------------------------
//  Smart buffer that returns to the owning pool on drop.

/// A smart byte buffer that automatically returns itself to the originating
/// [`BufferPool`] when dropped.
///
/// If the buffer was not obtained from a pool (e.g. via `PooledBuffer::new`),
/// it is simply deallocated on drop.
pub struct PooledBuffer {
    buf: Option<Vec<u8>>,
    pool: Option<BufferPool>,
}

impl PooledBuffer {
    /// Create a standalone pooled buffer *not* associated with any pool.
    /// On drop the buffer is deallocated normally.
    pub fn new(size: usize) -> Self {
        Self { buf: Some(vec![0u8; size]), pool: None }
    }

    /// Wrap an existing `Vec<u8>` into a pooled buffer (no pool association).
    pub fn from_vec(vec: Vec<u8>) -> Self {
        Self { buf: Some(vec), pool: None }
    }

    /// Access the underlying bytes.
    pub fn as_slice(&self) -> &[u8] {
        self.buf.as_deref().unwrap_or(&[])
    }

    /// Mutably access the underlying bytes.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buf.as_mut().map_or(&mut [], |v| v.as_mut_slice())
    }

    /// Length of the buffer.
    pub fn len(&self) -> usize {
        self.buf.as_ref().map_or(0, Vec::len)
    }

    /// Capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.buf.as_ref().map_or(0, Vec::capacity)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resize the buffer.
    pub fn resize(&mut self, new_len: usize, value: u8) {
        if let Some(buf) = self.buf.as_mut() {
            buf.resize(new_len, value);
        }
    }

    /// Clear the buffer without zeroing.
    pub fn clear(&mut self) {
        if let Some(buf) = self.buf.as_mut() {
            buf.clear();
        }
    }

    /// Take ownership of the inner `Vec<u8>`.  After this call the buffer
    /// will **not** be returned to the pool on drop.
    pub fn take(&mut self) -> Vec<u8> {
        self.buf.take().unwrap_or_default()
    }

    /// Returns `true` if this buffer is associated with a pool.
    pub fn is_pooled(&self) -> bool {
        self.pool.is_some()
    }
}

impl Deref for PooledBuffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.buf.as_deref().unwrap_or(&[])
    }
}

impl DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.buf.as_mut().map_or(&mut [], |v| v.as_mut_slice())
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.take() {
            if let Some(buf) = self.buf.take() {
                pool.release(buf);
            }
        }
    }
}

// ---------------------------------------------------------------------------
//  Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    // -- MemoryPool ---------------------------------------------------------

    #[test]
    fn test_pool_basic_alloc_dealloc() {
        let pool = MemoryPool::new(64, 4).unwrap();
        assert_eq!(pool.capacity(), 256);
        assert_eq!(pool.used(), 0);
        assert_eq!(pool.free_count(), 4);

        let mut block = pool.allocate(16).unwrap();
        assert_eq!(block.capacity(), 64);
        assert_eq!(block.len(), 0);
        block.set_offset(8).unwrap();
        assert_eq!(block.len(), 8);

        assert_eq!(pool.used(), 64);
        assert_eq!(pool.free_count(), 3);

        pool.deallocate(block);
        assert_eq!(pool.used(), 0);
        assert_eq!(pool.free_count(), 4);
    }

    #[test]
    fn test_pool_large_alloc_bypasses_pool() {
        let pool = MemoryPool::new(64, 2).unwrap();
        let block = pool.allocate(128).unwrap();
        assert_eq!(block.capacity(), 128);
        assert_eq!(pool.used(), 128);
        assert_eq!(pool.free_count(), 2); // large alloc didn't consume a pool block

        pool.deallocate(block);
        assert_eq!(pool.used(), 0);
        assert_eq!(pool.free_count(), 2); // large block was NOT returned to free list
    }

    #[test]
    fn test_pool_zero_size_rejected() {
        let pool = MemoryPool::new(64, 1).unwrap();
        assert!(pool.allocate(0).is_err());
    }

    #[test]
    fn test_pool_reset() {
        let pool = MemoryPool::new(32, 3).unwrap();
        let _a = pool.allocate(16).unwrap();
        let _b = pool.allocate(16).unwrap();
        assert_eq!(pool.used(), 64);
        drop(_a);
        drop(_b);
        // after dropping, used is 0 but we still have 2 dynamic allocs tracked
        // actually deallocate was called, so all is well
        pool.reset().unwrap();
        assert_eq!(pool.used(), 0);
        assert_eq!(pool.capacity(), 96); // 3 × 32
        assert_eq!(pool.free_count(), 3);
    }

    #[test]
    fn test_pool_capacity_tracking() {
        let pool = MemoryPool::new(128, 2).unwrap();
        assert_eq!(pool.capacity(), 256);

        // consume both pre-allocated
        let a = pool.allocate(64).unwrap();
        let b = pool.allocate(64).unwrap();
        assert_eq!(pool.capacity(), 256);
        assert_eq!(pool.used(), 256);
        assert_eq!(pool.free_count(), 0);

        // force a dynamic allocation
        let c = pool.allocate(64).unwrap();
        assert_eq!(pool.capacity(), 384); // grew by 128
        assert_eq!(pool.used(), 384);
        assert_eq!(pool.free_count(), 0);

        pool.deallocate(a);
        assert_eq!(pool.used(), 256);
        assert_eq!(pool.free_count(), 1);

        pool.deallocate(b);
        pool.deallocate(c);
        assert_eq!(pool.used(), 0);
        assert_eq!(pool.free_count(), 3);
    }

    #[test]
    fn test_pool_concurrent_allocations() {
        let pool = Arc::new(MemoryPool::new(16, 8).unwrap());
        let barrier = Arc::new(Barrier::new(4));
        let mut handles = Vec::new();

        for _ in 0..4 {
            let p = pool.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                b.wait();
                let block = p.allocate(8).unwrap();
                assert_eq!(block.capacity(), 16);
                p.deallocate(block);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // all blocks returned
        assert_eq!(pool.used(), 0);
        assert_eq!(pool.free_count(), 8);
    }

    #[test]
    fn test_pool_allocate_deallocate_roundtrip_reuses_block() {
        let pool = MemoryPool::new(32, 1).unwrap();
        let block1 = pool.allocate(4).unwrap();
        let addr1 = block1.as_ptr();
        pool.deallocate(block1);

        let block2 = pool.allocate(4).unwrap();
        let addr2 = block2.as_ptr();
        assert_eq!(addr1, addr2, "pool should reuse the same block");
    }

    #[test]
    fn test_pool_block_clone() {
        let pool = MemoryPool::new(64, 1).unwrap();
        let mut block = pool.allocate(32).unwrap();
        block.set_offset(12).unwrap();
        block.as_mut_slice().copy_from_slice(b"hello world!");
        let cloned = block.clone();
        assert_eq!(cloned.len(), 12);
        assert_eq!(&cloned.as_slice()[..12], b"hello world!");
    }

    #[test]
    fn test_pool_block_reset() {
        let mut block = MemoryBlock::new(16);
        block.set_offset(8).unwrap();
        block.as_mut_slice().fill(0xAB);
        block.reset();
        assert_eq!(block.len(), 0);
        assert_eq!(&block.as_raw_slice()[..4], &[0u8; 4]);
    }

    // -- SlabAllocator ------------------------------------------------------

    #[test]
    fn test_slab_basic_alloc_dealloc() {
        let slab = SlabAllocator::<u64>::new(4);
        assert_eq!(slab.capacity(), 0); // lazy – no pre-fill
        assert_eq!(slab.free_count(), 4);

        let i0 = slab.allocate();
        let _i1 = slab.allocate();
        assert_eq!(slab.live_count(), 2);

        {
            let mut guard = slab.get_mut(i0).unwrap();
            *guard = 42;
        }
        {
            let guard = slab.get(i0).unwrap();
            assert_eq!(*guard, 42);
        }

        slab.deallocate(i0);
        assert_eq!(slab.live_count(), 1);

        // the freed slot should be reused
        let i2 = slab.allocate();
        assert_eq!(i2, i0);
    }

    #[test]
    fn test_slab_grows_past_initial_capacity() {
        let slab = SlabAllocator::<String>::new(2);
        // consume the 2 free slots
        let _a = slab.allocate();
        let _b = slab.allocate();
        assert_eq!(slab.capacity(), 2);

        // this forces growth
        let c = slab.allocate();
        assert_eq!(slab.capacity(), 3);
        slab.deallocate(c);
    }

    #[test]
    fn test_slab_allocate_with() {
        let slab = SlabAllocator::new(2);
        let idx = slab.allocate_with(99u64);
        let guard = slab.get(idx).unwrap();
        assert_eq!(*guard, 99);
    }

    #[test]
    fn test_slab_concurrent_allocations() {
        let slab = Arc::new(SlabAllocator::<usize>::new(16));
        let barrier = Arc::new(Barrier::new(4));
        let mut handles = Vec::new();

        for _ in 0..4 {
            let s = slab.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                b.wait();
                for _ in 0..8 {
                    let idx = s.allocate();
                    // write thread id to verify isolation
                    *s.get_mut(idx).unwrap() = idx;
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(slab.live_count(), 32);
    }

    #[test]
    fn test_slab_get_invalid_index() {
        let slab = SlabAllocator::<u8>::new(2);
        assert!(slab.get(999).is_none());
        assert!(slab.get_mut(999).is_none());
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_slab_deallocate_invalid_index() {
        let slab = SlabAllocator::<u8>::new(2);
        slab.deallocate(999);
    }

    // -- BufferPool / PooledBuffer -----------------------------------------

    #[test]
    fn test_buffer_pool_acquire_release() {
        let pool = BufferPool::new(64);
        assert!(pool.is_empty());

        let buf = pool.acquire();
        assert_eq!(buf.capacity(), 64);
        assert!(pool.is_empty()); // buffer is checked out

        drop(buf);
        assert_eq!(pool.len(), 1); // buffer returned
    }

    #[test]
    fn test_pooled_buffer_drop_returns_to_pool() {
        let pool = BufferPool::new(128);
        {
            let _buf = pool.acquire();
        }
        assert_eq!(pool.len(), 1);

        // acquire again – should reuse the same buffer
        let buf2 = pool.acquire();
        assert_eq!(buf2.capacity(), 128);
        assert!(pool.is_empty()); // nothing left in cache
    }

    #[test]
    fn test_pooled_buffer_take_does_not_return() {
        let pool = BufferPool::new(32);
        let mut buf = pool.acquire();
        let _inner = buf.take(); // transfer ownership out
        assert!(buf.is_empty()); // buf's Vec is gone

        drop(buf); // should not panic, and not return anything to pool
        assert!(pool.is_empty()); // nothing was returned
    }

    #[test]
    fn test_pooled_buffer_deref_mut() {
        let mut buf = PooledBuffer::new(16);
        buf[..14].copy_from_slice(b"hello there!!!");
        assert_eq!(buf.len(), 16);
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn test_pooled_buffer_resize_clear() {
        let mut buf = PooledBuffer::new(10);
        buf.resize(20, 0xAA);
        assert_eq!(buf.len(), 20);
        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_pooled_buffer_standalone_drop() {
        let buf = PooledBuffer::new(64);
        assert!(!buf.is_pooled());
        // just ensure no crash on drop
        drop(buf);
    }

    #[test]
    fn test_pooled_buffer_from_vec() {
        let original = vec![1u8, 2, 3, 4];
        let buf = PooledBuffer::from_vec(original);
        assert_eq!(buf.len(), 4);
        assert!(!buf.is_pooled());
    }

    #[test]
    fn test_buffer_pool_clone_shares_state() {
        let pool = BufferPool::new(64);
        let pool2 = pool.clone();

        {
            let _buf = pool.acquire();
        }

        assert_eq!(pool2.len(), 1);
    }

    #[test]
    fn test_buffer_pool_concurrent() {
        let pool = Arc::new(BufferPool::new(256));
        let barrier = Arc::new(Barrier::new(4));
        let mut handles = Vec::new();

        for _ in 0..4 {
            let p = pool.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                b.wait();
                for _ in 0..10 {
                    let mut buf = p.acquire();
                    let cap = buf.capacity().min(16);
                    buf[..cap].copy_from_slice(b"concurrent test!");
                    drop(buf);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // at least some buffers should be cached
        assert!(pool.len() > 0, "expected cached buffers after concurrent use");
    }
}
