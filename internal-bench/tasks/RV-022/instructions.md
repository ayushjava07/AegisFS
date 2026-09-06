# Task RV-022: Reject Out-of-Bounds Delay Values in gRPC Converter

## Subsystem
`api` (gRPC Wire Deserialization & Validation)

## Summary
The gRPC wire protocol defines `base_delay_ms` and `max_delay_ms` as `uint64`. If the gRPC converter converts these values without enforcing ceiling validation (`policy.validate()` or magnitude check), extreme values like `u64::MAX` bypass domain bounds and corrupt scheduler duration calculations.

## Expected Behavior
1. In `src/api/grpc.rs`, `retry_from` must validate delay bounds via `policy.validate()`.
2. Any retry policy with `base_delay_ms` or `max_delay_ms` exceeding platform limits (`MAX_BACKOFF_MS`) must return `tonic::Status::invalid_argument`.

## Files Affected
- `src/api/grpc.rs`

## Verification
Run:
```bash
cargo test --lib api::grpc::tests
```
Assert that `retry_from` rejects oversized uint64 delay configurations.
