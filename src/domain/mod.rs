//! Domain layer: the vocabulary of Runvane.
//!
//! This module owns the data model and the validation rules that govern it.
//! It has no I/O and no knowledge of transports, storage, or scheduling —
//! everything else in the platform consumes these types and relies on their
//! invariants.
//!
//! Sub-modules:
//! * [`ids`] — typed identifiers for every entity;
//! * [`status`] — run and task status enumerations;
//! * [`retry_policy`] — retry/backoff configuration;
//! * [`workflow`] — workflow definitions and task specs;
//! * [`run`] — run and task-run records;
//! * [`dag`] — task-graph ordering and cycle detection;
//! * [`error`] — domain error types;
//! * [`validation`] — structural validation of definitions and inputs.

pub mod dag;
pub mod error;
pub mod ids;
pub mod retry_policy;
pub mod run;
pub mod status;
pub mod validation;
pub mod workflow;

/// Re-export of the most frequently used domain surface for downstream
/// convenience. Prefer importing from `runvane::domain::*` in crate code.
pub mod prelude {
    pub use super::error::DomainError;
    pub use super::ids::{HandlerId, RunId, TaskRunId, TenantId, WorkflowId};
    pub use super::retry_policy::{BackoffKind, JitterKind, RetryPolicy};
    pub use super::run::{Run, RunError, TaskRun};
    pub use super::status::{FailureKind, Priority, RunStatus, TaskStatus};
    pub use super::workflow::{HookSpec, Hooks, TaskSpec, WorkflowDef};
}