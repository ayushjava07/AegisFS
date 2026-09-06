# Task RV-032: Cap Retained Retry History

## Subsystem
`retry` / `scheduler` (Retry State & Memory Bounds)

## Summary
When workflow runs encounter intermittent or transient failures across repeated backoff attempts, the retry subsystem records attempt metadata. If historical attempts are accumulated without an eviction ceiling or capacity bound, long-running flapping jobs retain unbounded attempt collections in memory, causing heap leaks and bloated serialization payloads.

## Expected Behavior
1. Retained retry history per run must be capped to a bounded limit (e.g. keeping the most recent attempts).
2. The retry planner and attempt tracking must enforce this boundary under high attempt counts.

## Files Affected
- `src/retry/mod.rs`

## Verification
Run:
```bash
cargo test --lib retry::tests
```
Assert that attempt planning and tracking remain bounded and memory-safe under extensive retries.
