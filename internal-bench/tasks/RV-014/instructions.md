# Task RV-014: Invalidate Cached Definitions on Version Updates

## Subsystem
`persistence` (LRU Caching & Version Updates)

## Summary
When an operator calls `put_workflow` with an incremented `version` or when bulk migration tools bump workflow definitions, `LruStore` must invalidate existing cache slots corresponding to `(tenant, name)`. If invalidation is missed or deferred, `get_workflow` continues to return the previously cached version, causing newly submitted runs to run tasks according to an obsolete workflow DAG.

## Expected Behavior
1. Any write operation (`put_workflow`, `delete_workflow`) must explicitly invalidate the cached entry in `LruStore`.
2. Immediate subsequent calls to `get_workflow` must hit the inner store and return the newly bumped definition.
3. Cached entries must store and compare definition versions to guarantee freshness.

## Files Affected
- `src/persistence/lru_store.rs`

## Verification
Run:
```bash
cargo test --lib persistence::lru_store::tests
```
Assert that updating a workflow definition version causes subsequent reads to serve the updated definition.
