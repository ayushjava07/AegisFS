# Task RV-026: Reject Negative Worker Count in Environment Overrides

## Subsystem
`config` (Environment Configuration & Parsing)

## Summary
When an operator sets `RUNVANE_WORKERS` to a negative number like `-1` in the environment, parsing it as a signed integer followed by an unchecked `as usize` cast produces `usize::MAX`. Because `self.workers` becomes non-zero, validation passes, and the server attempts to spawn an enormous thread pool, crashing the host or exhausting memory.

## Expected Behavior
1. `apply_env_map` must reject negative values for `RUNVANE_WORKERS` with a configuration error.
2. Worker count must parse strictly into non-negative values within reasonable bounds.

## Files Affected
- `src/config.rs`

## Verification
Run:
```bash
cargo test --lib config::tests
```
Assert that setting `RUNVANE_WORKERS=-1` produces an error and is not cast to `usize::MAX`.
