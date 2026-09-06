# Task RV-006: Prevent Scheduler Starvation from Zero-Worker Configuration

## Subsystem
`scheduler` / `cli` (Capacity Validation & Resource Management)

## Summary
When the platform is configured with `workers = 0` (via `--workers 0` or configuration file), the system initializes with an empty thread pool. The dispatcher loop still activates and aggressively claims ready runs from the store, but submits them to a pool with zero worker threads. These jobs sit unexecuted while holding exclusive claim leases. When `shutdown()` is called, the scheduler hangs indefinitely waiting on worker join handles that do not exist.

## Expected Behavior
1. Worker pool initialization must require `size >= 1`.
2. CLI configuration parsing and `serve` boot must validate that `workers > 0` before starting background services.
3. If zero is supplied, return a descriptive configuration error (`"worker count must be at least 1"`).

## Files Affected
- `src/config.rs`
- `src/scheduler/pool.rs`

## Verification
Run:
```bash
cargo test --lib
```
Assert that zero-worker configurations are rejected on startup.
