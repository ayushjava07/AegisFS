# Task RV-009: Eliminate Dispatcher Time-of-Check to Time-of-Use (TOCTOU) Race

## Subsystem
`scheduler` (Concurrency & State Synchronization)

## Summary
In `Dispatcher::step()`, the dispatcher scans ready queue entries with `store.scan_ready(...)` and then claims each entry with `store.claim(...)`. Between the initial scan and the `mark_running` transition, a client or external watcher might cancel the run or update its status. If `mark_running` relies on stale pre-claim run state without re-querying the database under the held lease token, it transitions an already-cancelled or terminal run into `Running`, violating state machine invariants.

## Expected Behavior
1. In `Dispatcher::mark_running`, re-fetch the latest `Run` record from the store *after* successfully acquiring the lease token.
2. Validate that `run.status == RunStatus::Queued` (or already `Running` under idempotent double-claim).
3. If the run was mutated to any other status (e.g. `Cancelled`), abort the submission, return `Err`, and drop/failclaim the lease immediately so that other workers and the queue stay synchronized.

## Files Affected
- `src/scheduler/pool.rs`

## Verification
Run:
```bash
cargo test --lib scheduler
```
Assert that concurrent cancellations during the claim window cleanly abort dispatch and release the claim token.
