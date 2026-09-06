# Task RV-033: Retention Maintenance for In-Memory Store

## Subsystem
`persistence` / `cli` (Retention Cleanup & Memory Leaks)

## Summary
When running `runvane serve` with in-memory persistence (`--memory` or without SQLite configuration), terminal workflow runs (`Succeeded`, `Failed`, `TimedOut`, `Cancelled`) remain indefinitely in `MemoryStore.runs`. If retention sweeps are inactive or not configured, memory consumption grows linearly with total executed runs.

## Expected Behavior
1. `reap_finished_runs(before_ms)` must remove all terminal runs whose `finished_at_ms` is older than `before_ms`.
2. Non-terminal runs (`Queued`, `Running`) and recent terminal runs must remain untouched.

## Files Affected
- `src/persistence/memory.rs`
- `src/cli/serve.rs`

## Verification
Run:
```bash
cargo test --lib persistence::tests
```
Assert that `reap_finished_runs` successfully purges expired terminal runs from the store.
