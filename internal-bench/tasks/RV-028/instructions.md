# Task RV-028: Enforce Input Payload Size Ceiling

## Subsystem
`domain` / `validation` (Input Payload Bounds)

## Summary
When clients submit workflow runs with JSON input payloads via HTTP or gRPC, `validate_run_input` validates the payload structure. If the payload size ceiling check (`MAX_RUN_INPUT_BYTES`) is missing or omitted, multi-megabyte documents are accepted into SQLite/Memory storage, leading to heap exhaustion and severe listing latency.

## Expected Behavior
1. `validate_run_input` must measure the serialized size of the input payload.
2. Inputs exceeding `MAX_RUN_INPUT_BYTES` must return `Err(DomainError::InputTooLarge { size, cap })`.

## Files Affected
- `src/domain/validation.rs`

## Verification
Run:
```bash
cargo test --lib domain::validation::tests
```
Assert that oversized JSON payloads are rejected with `InputTooLarge`.
