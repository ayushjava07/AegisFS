# Task RV-018: Unify Conflict Error Status Mapping Across Transports

## Subsystem
`api` (Cross-Transport Error Mapping)

## Summary
The system exposes both HTTP JSON and gRPC endpoints for workflow and run management. When a conflict occurs (e.g. concurrent creation, optimistic lock mismatch, or already-exists), HTTP routes return `409 Conflict` (mapped to `ApiError::Conflict`). However, `to_tonic_api` in `src/api/grpc.rs` must map `409 Conflict` to `tonic::Code::FailedPrecondition` (or `tonic::Code::AlreadyExists`). If mapped to `tonic::Code::InvalidArgument`, clients receive a client validation failure instead of a retryable state precondition conflict.

## Expected Behavior
1. `409 Conflict` in `ApiError` must map to `tonic::Code::FailedPrecondition` in gRPC status responses.
2. Clients calling gRPC endpoints encountering conflicts must observe `Code::FailedPrecondition`.

## Files Affected
- `src/api/grpc.rs`

## Verification
Run:
```bash
cargo test --lib api::grpc::tests
```
Assert that conflict mappings preserve precondition status codes across transports.
