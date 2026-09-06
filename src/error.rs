//! Top-level error type for the Runvane platform.
//!
//! Layer-specific error types stay in their own modules ([`domain::error`],
//! `persistence`, `config`, ...) and are flattened into [`RunvaneError`] at
//! service boundaries. Keeping one outward-facing error helps the API layer
//! map failures to responses consistently and lets the CLI render a single
//! kind of failure story.

use crate::domain::error::DomainError;
use crate::domain::ids::IdError;

/// Errors surfaced by the platform as a whole.
#[derive(Debug, thiserror::Error)]
pub enum RunvaneError {
    /// A domain-level error (validation, state machine, not-found).
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// An identifier failed to parse/validate.
    #[error(transparent)]
    Id(#[from] IdError),

    /// A persistence/backing-store failure.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// A scheduler/queue operation failed.
    #[error("scheduler error: {0}")]
    Scheduler(String),

    /// A configuration problem (bad file, precedence conflict, missing flag).
    #[error("config error: {0}")]
    Config(String),

    /// An HTTP/runtime wiring error.
    #[error("server error: {0}")]
    Server(String),

    /// A plugin/handler failure.
    #[error("handler error: {0}")]
    Handler(String),

    /// An unexpected internal condition (bug). Carries context for the
    /// operator regardless of `RUST_BACKTRACE`.
    #[error("internal error: {0}")]
    Internal(String),
}

/// A structured persist-layer failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// A unique-constraint collision (e.g. duplicate workflow version).
    #[error("conflict: {0}")]
    Conflict(String),

    /// The requested record does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// An optimistic-concurrency guard fired (version mismatch).
    #[error("concurrent modification of {0}")]
    ConcurrentModification(String),

    /// A schema/migration concern.
    #[error("schema error: {0}")]
    Schema(String),

    /// Underlying backend failure (SQLite error, lock failure, ...).
    #[error("storage backend error: {0}")]
    Backend(String),

    /// Queue-claim guard failed (entry already claimed by another worker).
    #[error("queue claim lost for run {0}")]
    ClaimLost(String),
}

impl StorageError {
    /// Whether the error represents a missing record (maps to 404).
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl RunvaneError {
    /// HTTP-style status code for the error's category.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::Domain(e) => e.http_status(),
            Self::Storage(e) => match e {
                StorageError::NotFound(_) => 404,
                StorageError::Conflict(_) | StorageError::ConcurrentModification(_) => 409,
                _ => 500,
            },
            Self::Id(_) | Self::Config(_) => 400,
            Self::Handler(_) => 502,
            Self::Scheduler(_) | Self::Server(_) | Self::Internal(_) => 500,
        }
    }

    /// A stable machine-readable category for logs and API error bodies.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Domain(e) => e.category(),
            Self::Id(_) => "validation",
            Self::Storage(e) => match e {
                StorageError::NotFound(_) => "not_found",
                StorageError::Conflict(_) | StorageError::ConcurrentModification(_) => "conflict",
                _ => "storage",
            },
            Self::Scheduler(_) => "scheduler",
            Self::Config(_) => "config",
            Self::Server(_) => "server",
            Self::Handler(_) => "handler",
            Self::Internal(_) => "internal",
        }
    }

    /// Whether the error is a "not found" for any layer.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Domain(DomainError::NotFound { .. }))
            || matches!(self, Self::Storage(e) if e.is_not_found())
    }

    /// Convenience constructor for domain not-found errors.
    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::Domain(DomainError::NotFound {
            entity,
            id: id.into(),
        })
    }
}

impl From<String> for StorageError {
    fn from(message: String) -> Self {
        Self::Backend(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_helpers() {
        let e = RunvaneError::not_found("run", "rn_x");
        assert!(e.is_not_found());
        assert_eq!(e.http_status(), 404);
        assert_eq!(e.category(), "not_found");
    }

    #[test]
    fn domain_errors_flow_through() {
        let e = RunvaneError::from(DomainError::CycleDetected);
        assert_eq!(e.http_status(), 400);
        assert_eq!(e.category(), "validation");
    }

    #[test]
    // [P2P] RV-019/001 witness (error→status mapping holds in both states).
    fn id_errors_map_to_bad_request() {
        let e = RunvaneError::from(IdError::Malformed {
            kind: "run",
            id: "rn_x".into(),
            reason: "bad".into(),
        });
        assert_eq!(e.http_status(), 400);
    }

    #[test]
    fn storage_categories() {
        let not_found = RunvaneError::from(StorageError::NotFound("run".into()));
        assert_eq!(not_found.category(), "not_found");
        let conflict = RunvaneError::from(StorageError::Conflict("dup".into()));
        assert_eq!(conflict.category(), "conflict");
        assert_eq!(conflict.http_status(), 409);
    }
}
