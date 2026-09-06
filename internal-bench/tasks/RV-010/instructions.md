# Task RV-010: Prevent Cache Invalidation Overwrite in LRU Store

## Subsystem
`persistence` (LRU Caching & Concurrency Invalidation)

## Summary
`LruStore` caches workflow definitions and runs to reduce SQLite disk load. When `get_workflow` experiences a cache miss, it fetches the definition from the underlying store and calls `cache.put(key, val)`. If an operator updates or deletes the workflow definition concurrently while the fetch is in flight, the invalidation clears the cache entry first, but the stale fetch subsequently writes its cached copy back into the LRU, resurrecting stale or deleted workflow records.

## Expected Behavior
1. Check version tag or check the cache state under lock when inserting following a miss.
2. Invalidate entries immediately on write/delete.
3. If an entry was invalidated while a fetch was running, do not re-insert the stale pre-invalidation record.

## Files Affected
- `src/persistence/lru_store.rs`

## Verification
Run:
```bash
cargo test --lib persistence::lru_store::tests
```
Assert that concurrent writes during cache misses do not leave stale entries in the LRU.
