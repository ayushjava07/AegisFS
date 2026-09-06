# Task RV-001: Enforce Strict Limit Boundaries on gRPC ListRuns

## Subsystem
`api/grpc` (Type Safety & Wire Validation)

## Summary
The gRPC `ListRuns` RPC accepts a `limit` field in `ListRunsRequest`. When clients pass `0` or negative values (represented as signed integers on the wire), the control plane improperly accepts the value without validating it against `1..=500` bounds, causing divergences between HTTP and gRPC query semantics.

## Expected Behavior
1. `ListRunsRequest.limit == 0` must be rejected with `tonic::Status::invalid_argument("limit must be between 1 and 500")`.
2. Negative `limit` values must be rejected with `tonic::Status::invalid_argument`.
3. Valid limits between `1` and `500` must be converted to `usize` and passed to `RunFilter.limit`.
4. Values exceeding `500` must be rejected with `invalid_argument`.

## Files Affected
- `src/api/grpc.rs`

## Verification
Run:
```bash
cargo test --lib api::grpc::tests
```
All tests including `[F2P] api::grpc::tests::list_runs_rejects_invalid_limit` must pass.
