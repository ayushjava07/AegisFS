# Task RV-020: Pin EventKind Machine-Readable Codes

## Subsystem
`events` (Event Delivery & Serialization)

## Summary
The platform emits webhook and outbox events when runs reach milestones (`run.started`, `run.succeeded`, `run.failed`, `run.timed_out`, `run.cancelled`). If `EventKind::code()` returns unpinned or renamed values (such as `run.timeout` or `run.canceled`), external consumers and hook event filters matching on the canonical codes fail to match, dropping webhook deliveries.

## Expected Behavior
1. `EventKind::code()` must return the pinned constants:
   - `RunStarted`: `"run.started"`
   - `RunSucceeded`: `"run.succeeded"`
   - `RunFailed`: `"run.failed"`
   - `RunTimedOut`: `"run.timed_out"`
   - `RunCancelled`: `"run.cancelled"`
2. A regression test must verify each enum variant matches its exact contract string.

## Files Affected
- `src/events/mod.rs`

## Verification
Run:
```bash
cargo test --lib events::tests
```
Assert that all `EventKind` codes match the contract constants.
