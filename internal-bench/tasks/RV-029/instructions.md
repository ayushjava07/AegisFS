# Task RV-029: Enforce Description Character Limit in gRPC Converter

## Subsystem
`api` (gRPC Validation & Transport Parity)

## Summary
When an operator creates a workflow via the HTTP API, descriptions longer than 512 characters are rejected with `DomainError::DescriptionTooLong`. In `src/api/grpc.rs`, `create_workflow` converts the wire description without validating or sanitizing length against `sanitize_description`. As a result, gRPC allows creating workflow definitions with arbitrarily long descriptions, breaking parity with HTTP and ballooning persistence metadata.

## Expected Behavior
1. In `src/api/grpc.rs`, workflow descriptions must be validated using `sanitize_description` before persisting.
2. If `description` exceeds 512 characters, `create_workflow` must return `tonic::Status::invalid_argument`.

## Files Affected
- `src/api/grpc.rs`

## Verification
Run:
```bash
cargo test --lib api::grpc::tests
```
Assert that `create_workflow` rejects specifications with descriptions exceeding 512 characters.
