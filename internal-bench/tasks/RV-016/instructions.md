# Task RV-016: Saturating Arithmetic for Lease Deadlines

## Subsystem
`scheduler` / `persistence` (Lease Calculation)

## Summary
When claiming or renewing a run lease, the scheduler and store compute `deadline = now_ms + lease_ms`. If an operator configures an extremely large `lease_ms` or a value close to `i64::MAX`, standard addition overflows in debug builds (panic) or wraps to a negative timestamp in release builds. This causes the lease to be treated as immediately expired, causing rapid claim-release thrashing.

## Expected Behavior
1. All lease expiration calculations (`now_ms + lease_ms`) must use `now_ms.saturating_add(lease_ms)`.
2. Large `lease_ms` values (e.g. `i64::MAX`) must produce a saturated deadline without overflowing or panicking.

## Files Affected
- `src/persistence/memory.rs`
- `src/persistence/sqlite.rs`
- `tests/boundary.rs`

## Verification
Run:
```bash
cargo test --test boundary lease
```
Assert that a claim with `lease_ms = i64::MAX` sets a saturated positive deadline and does not panic or wrap.
