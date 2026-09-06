# Task RV-004: Guard Worker Execution Against Interleaved Cancellations

## Subsystem
`state` (Run FSM & Concurrency Interleaving)

## Summary
When an operator calls `POST /v1/runs/{id}/cancel`, the store flips the run status to `Cancelled` and removes the pending queue entry. If a worker claimed the run just prior to the cancellation request arriving, the worker holds a valid claim token. Without re-verifying the authoritative run state prior to invoking plugins, the worker proceeds to execute tasks on behalf of an already-cancelled run, potentially invoking external side-effects (HTTP webhooks, scripts).

## Expected Behavior
1. In `RunExecutor::attempt_with_token`, the executor must re-fetch the latest run state from the store.
2. If `run.status == RunStatus::Cancelled` or `run.is_terminal()`, the executor must immediately abort task dispatch.
3. The executor returns `RunAction::Ack` and preserves the `Cancelled` status without executing further tasks.

## Files Affected
- `src/scheduler/executor.rs`

## Verification
Run:
```bash
cargo test --lib scheduler::tests::in_flight_cancellation_aborts_run_cleanly
```
Assert that cancelled runs never trigger task handler invocations.
