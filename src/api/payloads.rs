//! Versioned request/response payloads for the HTTP surface.
//!
//! Wire bodies deliberately mirror the canonical domain documents (`data`
//! fields serialize the same `WorkflowDef`/`Run` the store returns) so clients
//! can round-trip them without a second translation layer. Request payloads
//! are the *subset* an operator supplies; server-managed fields (ids,
//! versions, timestamps) are stamped during conversion and never accepted
//! from the wire.

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

use crate::domain::ids::{HandlerId, TenantId, WorkflowId, generate_id};
use crate::domain::retry_policy::RetryPolicy;
use crate::domain::status::{Priority, RunStatus};
use crate::domain::validation;
use crate::domain::workflow::{Hooks, TaskSpec, WorkflowDef, SPEC_VERSION};
use crate::error::RunvaneError;
use crate::persistence::RunFilter;

use super::error::ApiError;

/// Envelope wrapping every successful response body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<T: Serialize> {
    /// Wire-format version of the envelope contract.
    pub spec_version: u8,
    /// The operation's payload.
    pub data: T,
}

impl<T: Serialize> Envelope<T> {
    /// Wraps `data` in a v1 envelope.
    pub fn of(data: T) -> Self {
        Self {
            spec_version: 1,
            data,
        }
    }
}

/// Definition payload accepted by `POST /v1/workflows`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSpec {
    /// Owning tenant.
    pub tenant: String,
    /// Definition name (unique per tenant).
    pub name: String,
    /// Optional free-form description.
    pub description: Option<String>,
    /// The task graph; must not be empty.
    pub tasks: Vec<TaskSpecPayload>,
    /// Whole-workflow deadline measured from first dispatch.
    pub timeout_ms: Option<u64>,
    /// Default retry policy for tasks without their own override.
    pub retry: Option<RetryPolicy>,
    /// Default submission priority for new runs.
    pub default_priority: Option<Priority>,
    /// Completion hooks fired at run-lifecycle transitions.
    pub hooks: Option<Hooks>,
    /// Operator tags.
    pub tags: Option<BTreeMap<String, String>>,
}

impl WorkflowSpec {
    /// Converts into a validated, server-stamped definition at `now_ms`.
    pub fn into_definition(self, now_ms: i64) -> Result<WorkflowDef, RunvaneError> {
        let tenant = TenantId::parse(&self.tenant)?;
        let tasks = self
            .tasks
            .into_iter()
            .map(TaskSpecPayload::into_task)
            .collect::<Result<Vec<_>, RunvaneError>>()?;
        let def = WorkflowDef {
            id: WorkflowId::from_validated(generate_id("wf_")),
            tenant: tenant.to_string(),
            name: self.name,
            version: 1,
            description: self.description.unwrap_or_default(),
            tasks,
            timeout_ms: self.timeout_ms.unwrap_or(60_000),
            retry: self.retry.unwrap_or_default(),
            default_priority: self.default_priority.unwrap_or_default(),
            hooks: self.hooks.unwrap_or_default(),
            tags: self.tags.unwrap_or_default(),
            spec_version: SPEC_VERSION,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        validation::validate_definition(&def)?;
        validation::validate_workflow_policy(&def)?;
        Ok(def)
    }
}

/// Single task description inside a [`WorkflowSpec`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpecPayload {
    /// Task name; unique within the workflow.
    pub name: String,
    /// Handler plugin reference.
    pub handler: String,
    /// Static input passed to the handler.
    pub input: Option<Json>,
    /// Task names that must succeed first.
    pub depends_on: Option<Vec<String>>,
    /// Per-task timeout override.
    pub timeout_ms: Option<u64>,
    /// Per-task retry override.
    pub retry: Option<RetryPolicy>,
    /// Free-form metadata.
    pub meta: Option<BTreeMap<String, Json>>,
}

impl TaskSpecPayload {
    /// Converts into a domain task, parsing the handler reference.
    pub fn into_task(self) -> Result<TaskSpec, RunvaneError> {
        Ok(TaskSpec {
            name: self.name,
            handler: HandlerId::parse(&self.handler)?,
            input: self.input.unwrap_or_else(|| json!({})),
            depends_on: self.depends_on.unwrap_or_default(),
            timeout_ms: self.timeout_ms,
            retry: self.retry,
            meta: self.meta.unwrap_or_default(),
        })
    }
}

/// Body accepted by `POST /v1/workflows/{tenant}/{name}/runs`.
///
/// An omitted body is equivalent to `{ "input": {} }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitRunRequest {
    /// Run input; size-capped by the platform.
    pub input: Option<Json>,
    /// Operator tags captured at submission.
    pub tags: Option<BTreeMap<String, String>>,
}

/// Hard ceiling on runs returned by a single list query.
pub const MAX_LIST_LIMIT: usize = 500;

/// Query-string parameters for `GET /v1/runs`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunQuery {
    /// Match the tenant exactly.
    pub tenant: Option<String>,
    /// Match the workflow definition name exactly.
    pub name: Option<String>,
    /// Match the run status exactly.
    pub status: Option<String>,
    /// Maximum rows to return (1..=500).
    pub limit: Option<usize>,
}

impl RunQuery {
    /// Converts into a store filter, rejecting malformed dimensions.
    pub fn into_filter(self) -> Result<RunFilter, ApiError> {
        let status = match self.status {
            None => None,
            Some(raw) => Some(
                RunStatus::from_str(&raw)
                    .map_err(|_| ApiError::bad_request(format!("invalid status {raw:?}")))?,
            ),
        };
        let limit = match self.limit {
            None => None,
            Some(n) if n == 0 || n > MAX_LIST_LIMIT => {
                return Err(ApiError::bad_request(format!(
                    "limit must be within 1..={MAX_LIST_LIMIT}"
                )));
            }
            Some(n) => Some(n),
        };
        Ok(RunFilter {
            tenant: self.tenant,
            name: self.name,
            status,
            limit,
            ..RunFilter::default()
        })
    }
}

/// Readiness payload for `GET /v1/health`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthView {
    /// Literal `ok` while the server can serve.
    pub status: &'static str,
    /// Server boot timestamp, epoch milliseconds.
    pub booted_at_ms: i64,
    /// Current platform time, epoch milliseconds.
    pub now_ms: i64,
    /// Depth of the unclaimed run queue.
    pub queue_depth: usize,
    /// Envelope version the server speaks.
    pub spec_version: u8,
}