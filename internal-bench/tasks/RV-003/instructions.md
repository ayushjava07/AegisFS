# Task RV-003: Prevent Hot-Looping on Past Deadline Timed-Out Runs

## Subsystem
`scheduler` (State Transition & Timing)

## Summary
When a workflow run exceeds its absolute deadline (`deadline_at_ms`) or when an attempt exhausts its retry budget, the scheduler decides the terminal queue action. If `next_attempt_at_ms` was previously computed at a timestamp in the past, releasing or re-evaluating the entry without clamping produces negative duration intervals (`due_at_ms < now_ms`). The dispatcher continuously re-claims and re-scans the run on every tick, causing 100% CPU starvation in the scheduler loop.

## Expected Behavior
1. Timed-out runs must transition immediately to terminal `TimedOut` status.
2. The run's queue entry must be acknowledged (`RunAction::Ack`) and removed from the active ready queue, rather than re-queued with a past timestamp.
3. If an intermediate re-queue is required, `release_at_ms` must be clamped: `release_at_ms >= now_ms`.

## Files Affected
- `src/scheduler/executor.rs`

## Verification
Run:
```bash
cargo test --lib scheduler
```
Assert that `timed_out_run_parks_cleanly` acknowledges the queue entry without hot-looping.
