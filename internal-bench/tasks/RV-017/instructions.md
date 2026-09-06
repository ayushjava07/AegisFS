# Task RV-017: Enforce RunQuery Limit Boundaries

## Subsystem
`api` (HTTP Query Parsing & Validation)

## Summary
When an operator passes query parameters to `GET /v1/runs`, `RunQuery.into_filter()` converts the query into a domain `RunFilter`. If `limit` is unvalidated or parsed without range checking, requests with `limit=0` or `limit > MAX_LIST_LIMIT` (e.g. 10,000) are forwarded to the storage layer, causing unbounded query execution or incorrect pagination.

## Expected Behavior
1. `RunQuery::into_filter` must reject `limit: Some(0)` and `limit: Some(n)` where `n > MAX_LIST_LIMIT` with `ApiError::bad_request`.
2. Valid limits (1..=500) must be accepted and passed into `RunFilter.limit`.

## Files Affected
- `src/api/payloads.rs`

## Verification
Run:
```bash
cargo test --lib api::payloads::tests
```
Assert that out-of-bounds limits in `RunQuery` return a `bad_request` error.
