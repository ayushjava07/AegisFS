# Task RV-015: Normalize max_attempts 0 Boundary in Retry Policy

## Subsystem
`retry` / `api` (Retry Policy Boundary Handling)

## Summary
When an operator configures a task retry policy with `max_attempts: 0` or specifies zero attempts via the CLI / API, differing interpretations occur across transports. In one transport path 0 is interpreted as disabled/infinite while in domain execution it evaluates as immediate exhaustion or underflow. Retry policy parsing must enforce canonical validation rejecting or normalizing 0 attempts uniformly across all ingestion surfaces.

## Expected Behavior
1. `max_attempts: 0` must be rejected during policy validation across all transports with `PolicyError::BadAttempts(0)` or `DomainError::Validation`.
2. Boundary test asserting `max_attempts` behavior across 0, 1, and `MAX_ATTEMPTS` must verify uniform semantics.

## Files Affected
- `src/domain/retry_policy.rs`
- `src/api/grpc.rs`
- `tests/boundary.rs`

## Verification
Run:
```bash
cargo test --test boundary retry
```
Assert that `max_attempts: 0` consistently fails validation with a clear error.
