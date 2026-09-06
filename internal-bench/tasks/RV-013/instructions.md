# Task RV-013: Guarantee Multi-Tenant Isolation in LRU Cache Keys

## Subsystem
`persistence` (LRU Caching & Tenant Isolation)

## Summary
In `LruStore`, cached workflow definitions must be partitioned by tenant. If any internal method or helper keys the cache map solely by `workflow_name` rather than the compound key `(Tenant, WorkflowName)`, a cache warm from tenant `alpha` will mistakenly be returned to tenant `beta` when tenant `beta` queries or submits a workflow with the same name.

## Expected Behavior
1. All workflow cache keys in `LruStore` must be structured as `(String, String)` representing `(tenant, name)`.
2. Cache lookups from tenant `B` must never hit entries stored by tenant `A`.
3. Invalidation must similarly target the compound key.

## Files Affected
- `src/persistence/lru_store.rs`

## Verification
Run:
```bash
cargo test --lib persistence::lru_store::tests
```
Assert that identically named workflows across different tenants never collide in the LRU cache.
