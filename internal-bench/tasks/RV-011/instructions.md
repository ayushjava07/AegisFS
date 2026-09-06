# Task RV-011: Deduplicate Terminal Webhook Deliveries in Event Watcher

## Subsystem
`events` (Watcher Differential Polling & Event Deduplication)

## Summary
The `Watcher` periodically queries the store for run state changes by diffing previous snapshots against current records. If a run transitions rapidly or a database query returns overlapping snapshots during concurrent status updates, the watcher can observe the same terminal state (`Succeeded`, `Failed`, etc.) in successive polls. This leads to duplicate webhook deliveries to customer HTTP endpoints, violating at-most-once/exactly-once event notifications.

## Expected Behavior
1. The watcher or event dispatcher must track the set of emitted `(RunId, EventKind)` pairs.
2. If `(run_id, kind)` has already been emitted for a terminal status, subsequent polls must suppress the duplicate event.
3. Every terminal state produces exactly one outgoing webhook delivery per configured hook.

## Files Affected
- `src/events/watcher.rs`
- `src/events/dispatch.rs`

## Verification
Run:
```bash
cargo test --lib events::watcher::tests
```
Assert that multiple polls on terminal runs fire events exactly once.
