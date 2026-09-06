# Task RV-005: Reject Lease Release on Terminal Runs

## Subsystem
`scheduler` / `persistence` (State Transition & Race Protection)

## Summary
When a worker thread finishes executing a run that succeeded, it calls `store.ack(&run_id, &token)`. In high-concurrency environments with retries, a delayed or stale worker thread might attempt to invoke `store.release(&run_id, &token, next_due)` after the run has already been acknowledged and transitioned to `Succeeded`. If `release` only matches the token against the queue entry without validating the authoritative run status, the run is erroneously flipped from `Succeeded` back to `Queued`, duplicating workflow execution.

## Expected Behavior
1. In `Store::release`, the store backend must check the authoritative `RunStatus`.
2. If `run.is_terminal()` (e.g. `Succeeded`, `Failed`, `Cancelled`, `TimedOut`), the store must reject the release transition or treat it as an idempotent no-op.
3. A terminal run must never be re-inserted into the active queue.

## Files Affected
- `src/persistence/memory.rs`
- `src/persistence/sqlite.rs`

## Verification
Run:
```bash
cargo test --lib persistence
```
Assert that releasing a lease for a terminal run is rejected and the run remains in its terminal state.
