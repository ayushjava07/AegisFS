# Task RV-019: Translate DomainError to 4xx Bad Request

## Subsystem
`api` (Error Translation & Propagation)

## Summary
When an operator calls `submit_run` with malformed input payloads, invalid parameters, or cycles in ad-hoc workflow definitions, domain validation raises `DomainError`. If `ApiError::from(DomainError)` does not translate the error using the domain HTTP status mapping (returning 400 or 409) and instead falls back to 500 Internal Server Error, malformed client requests appear to monitoring systems as service crashes.

## Expected Behavior
1. `From<DomainError> for ApiError` must map domain errors to their respective 4xx HTTP statuses (e.g. 400 for validation errors, 404 for not found, 409 for conflicts).
2. Submitting an invalid input or trigger payload must yield a 400 Bad Request response, never 500 Internal Server Error.

## Files Affected
- `src/api/error.rs`

## Verification
Run:
```bash
cargo test --lib api::error::tests
```
Assert that `DomainError::from` produces an `ApiError` with HTTP status 400.
