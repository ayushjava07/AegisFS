# Task RV-012: Enforce Monotonic Clock Source Propagation in Lease Bookkeeping

## Subsystem
`scheduler` (Clock Injection & Lease Monotonicity)

## Summary
The distributed lease engine relies on monotonic timestamps to calculate lease expiration windows (`lease_until_ms = now_ms + lease_ms`). If certain scheduler operations (such as heartbeat renewal or queue scanning) read from an un-injected system clock while the main dispatcher or test harness operates on a simulated or stepped clock (`ManualClock`), the mismatched timestamps trigger premature lease expiry or prevent rightful lease renewals.

## Expected Behavior
1. All timestamped store operations (`claim`, `renew_lease`, `release`, `scan_ready`) must receive `now_ms` derived from the injected `&dyn Clock`.
2. Do not invoke `Utc::now()` or host system clocks directly in scheduler execution loops.
3. Under stepped or frozen manual clocks, lease expiration logic behaves 100% deterministically.

## Files Affected
- `src/scheduler/executor.rs`
- `src/scheduler/pool.rs`

## Verification
Run:
```bash
cargo test --lib scheduler
```
Assert that stepped clock scenarios maintain accurate and monotonic lease states.
