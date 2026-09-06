//! Structured audit logging subsystem for administrative and operational actions.
//!
//! Runvane records security-relevant and state-altering operations (such as workflow
//! registrations, run cancellations, authentication failures, lease reaping, and retention sweeps)
//! into durable, queryable audit trails.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::domain::ids::AuditRecordId;

pub mod file;
pub mod memory;

#[cfg(test)]
mod tests;

/// Category of actor initiating the audited event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditActor {
    /// Internal automated system background worker or maintenance sweep.
    System {
        /// Component or worker name (e.g. `scheduler::reaper`, `events::watcher`).
        component: String,
    },
    /// Interactive authenticated user identity.
    User {
        /// User name or principal identifier.
        username: String,
        /// Role assigned to the user (e.g. `admin`, `operator`, `viewer`).
        role: String,
    },
    /// Machine API token or bearer token client.
    Token {
        /// Token identifier.
        token_id: String,
        /// Associated tenant identifier.
        tenant_id: String,
        /// Role authorized by the token.
        role: String,
    },
    /// Unauthenticated caller or anonymous network peer.
    Anonymous {
        /// Client peer IP address, if known.
        client_ip: Option<String>,
    },
}

impl AuditActor {
    /// Short string classifier for filtering purposes.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::System { .. } => "system",
            Self::User { .. } => "user",
            Self::Token { .. } => "token",
            Self::Anonymous { .. } => "anonymous",
        }
    }
}

/// The specific administrative or operational action performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AuditAction {
    /// A new workflow definition was registered.
    WorkflowCreated {
        /// Name of the workflow.
        name: String,
        /// Version assigned to the registered definition.
        version: u32,
    },
    /// An existing workflow definition was updated with a new version.
    WorkflowUpdated {
        /// Name of the workflow.
        name: String,
        /// New version registered.
        version: u32,
    },
    /// A workflow definition was marked for deletion or tombstoned.
    WorkflowDeleted {
        /// Name of the deleted workflow.
        name: String,
    },
    /// A workflow run was submitted.
    RunSubmitted {
        /// ID of the submitted run.
        run_id: String,
        /// Definition name.
        workflow_name: String,
    },
    /// An active or pending run was cancelled by an operator.
    RunCancelled {
        /// Target run identifier.
        run_id: String,
        /// Reason supplied for cancellation.
        reason: String,
    },
    /// A failed or dead-lettered run was manually re-enqueued for retry.
    RunRetried {
        /// Target run identifier.
        run_id: String,
        /// New attempt counter.
        attempt: u32,
    },
    /// An authentication or token verification check failed.
    AuthFailed {
        /// Reason for failure (e.g. invalid signature, expired, unknown token).
        reason: String,
        /// Remote caller IP address.
        client_ip: Option<String>,
    },
    /// An API token or credential was provisioned.
    TokenCreated {
        /// Generated token identifier.
        token_id: String,
        /// Role assigned to the token.
        role: String,
    },
    /// An API token was revoked or deleted.
    TokenRevoked {
        /// Revoked token identifier.
        token_id: String,
    },
    /// The scheduler watchdog reaped expired leases.
    LeaseReaped {
        /// Number of lapsed leases recovered.
        count: usize,
    },
    /// The retention cleaner purged old terminal runs.
    RetentionPurged {
        /// Number of purged run records.
        count: usize,
    },
    /// Custom administrative action.
    Custom {
        /// Action identifier.
        name: String,
    },
}

impl AuditAction {
    /// Returns a short string classifier for filtering.
    pub fn action_name(&self) -> &'static str {
        match self {
            Self::WorkflowCreated { .. } => "workflow_created",
            Self::WorkflowUpdated { .. } => "workflow_updated",
            Self::WorkflowDeleted { .. } => "workflow_deleted",
            Self::RunSubmitted { .. } => "run_submitted",
            Self::RunCancelled { .. } => "run_cancelled",
            Self::RunRetried { .. } => "run_retried",
            Self::AuthFailed { .. } => "auth_failed",
            Self::TokenCreated { .. } => "token_created",
            Self::TokenRevoked { .. } => "token_revoked",
            Self::LeaseReaped { .. } => "lease_reaped",
            Self::RetentionPurged { .. } => "retention_purged",
            Self::Custom { .. } => "custom",
        }
    }
}

/// The outcome of the audited action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuditOutcome {
    /// The action completed successfully.
    Success,
    /// The action was rejected or unauthorized.
    Denied {
        /// Detailed policy or authorization denial reason.
        reason: String,
    },
    /// The action failed due to an error.
    Error {
        /// Human-readable error message.
        message: String,
    },
}

impl AuditOutcome {
    /// Returns whether the outcome indicates successful completion.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns a short string representation of the outcome status.
    pub fn status_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied { .. } => "denied",
            Self::Error { .. } => "error",
        }
    }
}

/// A complete, structured audit record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Unique identifier for this audit event.
    pub id: AuditRecordId,
    /// Timestamp in epoch milliseconds when the event was recorded.
    pub timestamp_ms: i64,
    /// Tenant identifier associated with the event, if scoped.
    pub tenant: Option<String>,
    /// Actor who initiated the operation.
    pub actor: AuditActor,
    /// The specific action performed.
    pub action: AuditAction,
    /// Outcome of the action.
    pub outcome: AuditOutcome,
    /// Type of resource affected (e.g. `workflow`, `run`, `token`, `system`).
    pub resource_type: String,
    /// Resource identifier, if applicable.
    pub resource_id: Option<String>,
    /// Additional contextual attributes and metadata.
    pub metadata: BTreeMap<String, Json>,
}

impl AuditRecord {
    /// Constructs a new audit record builder with required fields.
    pub fn new(
        id: AuditRecordId,
        timestamp_ms: i64,
        actor: AuditActor,
        action: AuditAction,
        outcome: AuditOutcome,
        resource_type: impl Into<String>,
    ) -> Self {
        Self {
            id,
            timestamp_ms,
            tenant: None,
            actor,
            action,
            outcome,
            resource_type: resource_type.into(),
            resource_id: None,
            metadata: BTreeMap::new(),
        }
    }

    /// Sets the tenant for the audit record.
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Sets the affected resource ID.
    pub fn with_resource_id(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    /// Appends a metadata attribute.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<Json>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Query filter for searching audit records.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFilter {
    /// Filter by tenant identifier.
    pub tenant: Option<String>,
    /// Filter by actor category (e.g. `system`, `user`, `token`, `anonymous`).
    pub actor_kind: Option<String>,
    /// Filter by action name (e.g. `run_submitted`, `workflow_created`).
    pub action_name: Option<String>,
    /// Filter by outcome status (`success`, `denied`, `error`).
    pub status: Option<String>,
    /// Filter by resource type (e.g. `workflow`, `run`).
    pub resource_type: Option<String>,
    /// Filter by specific resource ID.
    pub resource_id: Option<String>,
    /// Include events recorded on or after this timestamp (epoch ms).
    pub since_ms: Option<i64>,
    /// Include events recorded on or before this timestamp (epoch ms).
    pub until_ms: Option<i64>,
    /// Maximum number of records to return.
    pub limit: usize,
    /// Number of matching records to skip (for pagination).
    pub offset: usize,
}

impl AuditFilter {
    /// Creates a default filter with no constraints and standard limit of 50.
    pub fn new() -> Self {
        Self {
            limit: 50,
            ..Default::default()
        }
    }

    /// Sets the tenant filter.
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Sets the actor category filter.
    pub fn with_actor_kind(mut self, kind: impl Into<String>) -> Self {
        self.actor_kind = Some(kind.into());
        self
    }

    /// Sets the action name filter.
    pub fn with_action_name(mut self, action: impl Into<String>) -> Self {
        self.action_name = Some(action.into());
        self
    }

    /// Sets the outcome status filter.
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    /// Sets the resource type filter.
    pub fn with_resource_type(mut self, rtype: impl Into<String>) -> Self {
        self.resource_type = Some(rtype.into());
        self
    }

    /// Sets the time window.
    pub fn with_time_range(mut self, since: Option<i64>, until: Option<i64>) -> Self {
        self.since_ms = since;
        self.until_ms = until;
        self
    }

    /// Sets the pagination parameters.
    pub fn with_pagination(mut self, limit: usize, offset: usize) -> Self {
        self.limit = limit.clamp(1, 1000);
        self.offset = offset;
        self
    }

    /// Returns `true` if the candidate record satisfies all active filter conditions.
    pub fn matches(&self, record: &AuditRecord) -> bool {
        if let Some(expected_tenant) = &self.tenant {
            match &record.tenant {
                Some(actual) if actual == expected_tenant => {}
                _ => return false,
            }
        }

        if let Some(expected_actor) = &self.actor_kind {
            if record.actor.kind_str() != expected_actor.as_str() {
                return false;
            }
        }

        if let Some(expected_action) = &self.action_name {
            if record.action.action_name() != expected_action.as_str() {
                return false;
            }
        }

        if let Some(expected_status) = &self.status {
            if record.outcome.status_str() != expected_status.as_str() {
                return false;
            }
        }

        if let Some(expected_rtype) = &self.resource_type {
            if &record.resource_type != expected_rtype {
                return false;
            }
        }

        if let Some(expected_rid) = &self.resource_id {
            match &record.resource_id {
                Some(actual) if actual == expected_rid => {}
                _ => return false,
            }
        }

        if let Some(since) = self.since_ms {
            if record.timestamp_ms < since {
                return false;
            }
        }

        if let Some(until) = self.until_ms {
            if record.timestamp_ms > until {
                return false;
            }
        }

        true
    }
}

/// Errors arising in audit logger backends.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// An I/O error occurred while appending or reading from storage.
    #[error("audit I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON or binary serialization error occurred.
    #[error("audit serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// An internal storage backend failure.
    #[error("audit storage failure: {0}")]
    Storage(String),
}

/// Interface for recording and querying structured audit events.
pub trait AuditLogger: Send + Sync {
    /// Appends a new audit record to the persistent log.
    fn record(&self, record: AuditRecord) -> Result<(), AuditError>;

    /// Queries audit records matching the provided filter, ordered newest-first.
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError>;

    /// Counts total audit records matching the filter without pagination.
    fn count(&self, filter: &AuditFilter) -> Result<usize, AuditError>;
}

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.action_name())
    }
}
