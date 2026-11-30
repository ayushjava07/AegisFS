//! State-machine engine and run/task machines.
//!
//! * [`machine`] — the generic transition-table core.
//! * [`run_fsm`] — the workflow-run machine (the authority on legal run
//!   transitions).
//! * [`task_fsm`] — the task-run machine.
//! * [`invariant`] — cross-entity consistency checks.
//! * [`snapshots`] — point-in-time run aggregates for observability.

pub mod invariant;
pub mod machine;
pub mod run_fsm;
pub mod snapshots;
pub mod task_fsm;
