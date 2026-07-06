//! Domain-level error types.
//!
//! Everything that can go wrong *while interpreting a workflow document* is
//! a `DomainError`. Storage, scheduling, and transport failures live in their
//! own modules; this module deliberately stays free of I/O concerns so it can
//! be reused from the validation layer, the state machine, and the API
//! mappers without coupling.

use super::status::{RunStatus, TaskStatus};
use super::workflow::MAX_TASKS;
use crate::domain::ids::HandlerId;

/// Errors produced by domain validation and state-machine work.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    /// A requested transition is not part of the legal state machine.
    #[error("illegal state transition {0:?} -> {1:?}")]
    IllegalTransition(RunStatus, RunStatus),

    /// A task-level transition is not part of the legal task machine.
    #[error("illegal task state transition {0:?} -> {1:?}")]
    IllegalTaskTransition(TaskStatus, TaskStatus),

    /// The workflow definition document is structurally invalid.
    #[error("invalid workflow definition: {0}")]
    InvalidDefinition(String),

    /// A named dependency does not reference a known task.
    #[error("unknown dependency {0:?} in task {1:?}")]
    UnknownDependency(String, String),

    /// The task graph contains at least one cycle.
    #[error("task dependency cycle detected")]
    CycleDetected,

    /// Duplicate task name within a single definition.
    #[error("duplicate task name {0:?}")]
    DuplicateTaskName(String),

    /// The definition name does not satisfy the naming rules.
    #[error("invalid definition name {0:?}")]
    InvalidName(String),

    /// A task list may not be empty.
    #[error("workflow definition has no tasks")]
    EmptyTasks,

    /// Too many tasks in a single definition.
    #[error("workflow defines more than {MAX_TASKS} tasks")]
    TooManyTasks,

    /// Input payload exceeded the size cap.
    #[error("input payload of {size} bytes exceeds cap of {cap} bytes")]
    InputTooLarge {
        /// Measured payload size in bytes.
        size: usize,
        /// Platform/definition cap in bytes.
        cap: usize,
    },

    /// A referenced handler id is not registered with the platform.
    #[error("unknown handler {0:?}")]
    UnknownHandler(HandlerId),

    /// The retry policy failed structural validation.
    #[error("invalid retry policy: {0}")]
    InvalidPolicy(String),

    /// A timeout value of zero is not allowed where a deadline is required.
    #[error("timeout must be positive")]
    ZeroTimeout,

    /// Description exceeded the length cap.
    #[error("description exceeds 512 characters")]
    DescriptionTooLong,

    /// JSON template expansion failed.
    #[error("input template expansion failed: {0}")]
    TemplateError(String),

    /// Attempted to run an operation on an entity that is already in a
    /// terminal state.
    #[error("operation not allowed on terminal entity")]
    TerminalState,

    /// Unknown entity (workflow/run/task) referenced by an id.
    #[error("{entity} {id} not found")]
    NotFound {
        /// Type of the missing entity, for the message.
        entity: &'static str,
        /// The id that did not resolve.
        id: String,
    },
}

impl DomainError {
    /// A concise machine-readable category used by API error mapping.
    pub fn category(&self) -> &'static str {
        match self {
            Self::IllegalTransition(..) | Self::IllegalTaskTransition(..) => "state_machine",
            Self::InvalidDefinition(_)
            | Self::UnknownDependency(..)
            | Self::CycleDetected
            | Self::DuplicateTaskName(_)
            | Self::InvalidName(_)
            | Self::EmptyTasks
            | Self::TooManyTasks
            | Self::InputTooLarge { .. }
            | Self::InvalidPolicy(_)
            | Self::ZeroTimeout
            | Self::DescriptionTooLong
            | Self::TemplateError(_) => "validation",
            Self::UnknownHandler(_) => "handler",
            Self::TerminalState => "state_machine",
            Self::NotFound { .. } => "not_found",
        }
    }

    /// HTTP-style status code for the category.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::NotFound { .. } => 404,
            Self::IllegalTransition(..) | Self::IllegalTaskTransition(..) => 409,
            Self::TerminalState => 409,
            _ => 400,
        }
    }
}

/// Convienience alias for fallible domain operations.
pub type Result<T> = std::result::Result<T, DomainError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::status::{RunStatus, TaskStatus};

    #[test]
    fn transition_error_category_and_status() {
        let e = DomainError::IllegalTransition(RunStatus::Succeeded, RunStatus::Queued);
        assert_eq!(e.category(), "state_machine");
        assert_eq!(e.http_status(), 409);
    }

    #[test]
    fn validation_errors_map_to_400() {
        let e = DomainError::InvalidName("bad".into());
        assert_eq!(e.category(), "validation");
        assert_eq!(e.http_status(), 400);
    }

    #[test]
    fn not_found_maps_to_404() {
        let e = DomainError::NotFound {
            entity: "run",
            id: "rn_x".into(),
        };
        assert_eq!(e.http_status(), 404);
        assert_eq!(e.category(), "not_found");
    }

    #[test]
    fn task_transition_is_distinct() {
        let e = DomainError::IllegalTaskTransition(TaskStatus::Succeeded, TaskStatus::Running);
        assert_eq!(e.category(), "state_machine");
    }
}