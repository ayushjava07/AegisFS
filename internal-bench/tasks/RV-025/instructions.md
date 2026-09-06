# Task RV-025: Decouple Event Watcher Cadence From Lease Reaping

## Subsystem
`events` / `scheduler` (Polling Cadence & Decoupled Scheduling)

## Summary
In `src/cli/serve.rs`, the scheduler loop executes lease reaping, dispatching, and event watcher polling sequentially, sleeping for `reap_ms` at each iteration. When an operator tunes `reap_interval_ms` to high values (e.g. 10–30s to avoid DB load on long-lived leases), event watcher polling and webhook deliveries are delayed proportionally, violating SLA requirements for prompt event notifications.

## Expected Behavior
1. Event notification polling must operate on an independent cadence from lease reaping.
2. Changes to `reap_interval_ms` must not delay watcher polling cadence or starve webhook delivery.

## Files Affected
- `src/cli/serve.rs`
- `src/events/watcher.rs`

## Verification
Run:
```bash
cargo test --lib events::watcher::tests
```
Assert that watcher polling frequency is decoupled from lease expiration sweeps.
