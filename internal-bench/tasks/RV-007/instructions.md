# Task RV-007: Guarantee Terminal Status Persistence Under Expired Lease Contention

## Subsystem
`scheduler` / `persistence` (Lease Expiry & Status Commit)

## Summary
During long execution attempts where a lease expires and the reaper thread resets the queue entry, a worker completing the task attempt calls `apply_action(store, &job, outcome.action)`. If `store.ack` or `store.release` encounters `StorageError::NotFound` because the lease was reaped or stolen, the worker path must NOT abort without persisting the final `RunStatus` (e.g. `Succeeded` or `Failed`). If the status write is bypassed, the run record remains permanently in `Running`, the event watcher never emits terminal webhooks, and retention never reaps the run.

## Expected Behavior
1. In `WorkerPool` and `RunExecutor`, final `Run` record updates must be committed to the store regardless of queue lease ack/release status.
2. Even if `store.ack` returns `NotFound` or `ClaimLost`, `store.put_run` with the final status must still execute.
3. The run status definitively reaches terminal `Succeeded` or `Failed`.

## Files Affected
- `src/scheduler/pool.rs`
- `src/scheduler/executor.rs`

## Verification
Run:
```bash
cargo test --lib scheduler
```
Assert that runs whose queue entries were contended or expired still record their terminal status in the store.
