# Task RV-030: Reject Dangling Task References in DAG Validation

## Subsystem
`domain` (DAG Validation & Dependency Resolution)

## Summary
In workflow definitions, tasks declare dependencies via `depends_on`. If the DAG validator does not verify that all referenced dependency names correspond to tasks defined within the same workflow, definitions referencing non-existent tasks (e.g. `depends_on: ["ghost"]`) pass validation. When executed, the dependent tasks can never satisfy their dependencies, leaving runs permanently stuck in `Queued` or `Running`.

## Expected Behavior
1. In `src/domain/dag.rs`, `topo_order` must verify that every task name in `depends_on` exists in the definition's task set.
2. An unresolvable reference must return `Err(DomainError::UnknownDependency(dep, task))`.

## Files Affected
- `src/domain/dag.rs`

## Verification
Run:
```bash
cargo test --lib domain::dag::tests
```
Assert that definitions with dangling task dependencies fail validation with `DomainError::UnknownDependency`.
