# Task RV-002: Isolate Monotonic Run Numbers Across Tenants

## Subsystem
`persistence` (Identity & Multi-Tenancy Isolation)

## Summary
The `Store::next_run_number` method allocates monotonically increasing human-readable run numbers (1, 2, 3...) for workflow runs. However, the internal sequence table keys counters by `workflow_name` rather than `(tenant, workflow_name)`. If tenant `alpha` and tenant `beta` both register a workflow named `etl-pipeline`, submissions from tenant `beta` receive non-sequential numbers incremented by tenant `alpha`'s runs, leaking run activity and violating multi-tenancy invariants.

## Expected Behavior
1. `next_run_number` must take tenant context or store implementations must partition the sequence generator by `(tenant, workflow_name)`.
2. Tenant `alpha` running `etl-pipeline` gets run numbers `1, 2, 3...`.
3. Tenant `beta` running `etl-pipeline` simultaneously gets run numbers `1, 2, 3...` independently.

## Files Affected
- `src/persistence/mod.rs`
- `src/persistence/memory.rs`
- `src/persistence/sqlite.rs`

## Verification
Run:
```bash
cargo test --lib persistence
```
Ensure multi-tenant run sequence isolation tests pass across memory and SQLite stores.
