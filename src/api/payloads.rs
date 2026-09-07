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

use crate::domain::ids::{generate_id, HandlerId, TenantId, WorkflowId};
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
    #[serde(alias = "timeout_ms")]
    pub timeout_ms: Option<u64>,
    /// Default retry policy for tasks without their own override.
    pub retry: Option<RetryPolicy>,
    /// Default submission priority for new runs.
    #[serde(alias = "default_priority")]
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
    #[serde(alias = "depends_on")]
    pub depends_on: Option<Vec<String>>,
    /// Per-task timeout override.
    #[serde(alias = "timeout_ms")]
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
    /// Match definition name by prefix.
    pub name_prefix: Option<String>,
    /// Match the run status exactly.
    pub status: Option<String>,
    /// Comma-separated list of run statuses (any match).
    pub status_in: Option<String>,
    /// Only runs submitted at or after this epoch ms.
    pub from_ms: Option<i64>,
    /// Only runs submitted before this epoch ms.
    pub to_ms: Option<i64>,
    /// Only runs completed at or after this epoch ms.
    pub finished_from_ms: Option<i64>,
    /// Only runs completed at or before this epoch ms.
    pub finished_to_ms: Option<i64>,
    /// Minimum run duration in milliseconds.
    pub min_duration_ms: Option<i64>,
    /// Comma-separated tag keys that must be present.
    pub has_tag_keys: Option<String>,
    /// Maximum rows to return (1..=500).
    pub limit: Option<usize>,
    /// Pagination offset (number of rows to skip).
    pub offset: Option<usize>,
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

        let status_in = match self.status_in {
            None => Vec::new(),
            Some(raw) => {
                let mut statuses = Vec::new();
                for piece in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let s = RunStatus::from_str(piece).map_err(|_| {
                        ApiError::bad_request(format!("invalid status_in element {piece:?}"))
                    })?;
                    statuses.push(s);
                }
                statuses
            }
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

        let offset = self.offset.unwrap_or(0);

        if let (Some(from), Some(to)) = (self.from_ms, self.to_ms) {
            if from > to {
                return Err(ApiError::bad_request(format!(
                    "from_ms ({from}) must be <= to_ms ({to})"
                )));
            }
        }

        if let (Some(from), Some(to)) = (self.finished_from_ms, self.finished_to_ms) {
            if from > to {
                return Err(ApiError::bad_request(format!(
                    "finished_from_ms ({from}) must be <= finished_to_ms ({to})"
                )));
            }
        }

        if let Some(dur) = self.min_duration_ms {
            if dur < 0 {
                return Err(ApiError::bad_request(format!(
                    "min_duration_ms ({dur}) must be non-negative"
                )));
            }
        }

        let has_tag_keys = match self.has_tag_keys {
            None => Vec::new(),
            Some(raw) => raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect(),
        };

        Ok(RunFilter {
            tenant: self.tenant,
            name: self.name,
            name_prefix: self.name_prefix,
            status,
            status_in,
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            finished_from_ms: self.finished_from_ms,
            finished_to_ms: self.finished_to_ms,
            min_duration_ms: self.min_duration_ms,
            has_tag_keys,
            limit,
            offset,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_query_defaults_to_empty_filter() {
        let q = RunQuery::default();
        let f = q.into_filter().expect("default query must parse");
        assert_eq!(f, RunFilter::default());
    }

    #[test]
    fn run_query_validates_status_and_status_in() {
        let q = RunQuery {
            status: Some("invalid_status".into()),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());

        let q = RunQuery {
            status: Some("running".into()),
            status_in: Some("queued, succeeded,failed".into()),
            ..RunQuery::default()
        };
        let f = q.into_filter().expect("valid statuses");
        assert_eq!(f.status, Some(RunStatus::Running));
        assert_eq!(
            f.status_in,
            vec![RunStatus::Queued, RunStatus::Succeeded, RunStatus::Failed]
        );

        let q = RunQuery {
            status_in: Some("queued, bogus".into()),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());
    }

    #[test]
    fn run_query_validates_limits_and_ranges() {
        let q = RunQuery {
            limit: Some(0),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());

        let q = RunQuery {
            limit: Some(MAX_LIST_LIMIT + 1),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());

        let q = RunQuery {
            from_ms: Some(200),
            to_ms: Some(100),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());

        let q = RunQuery {
            finished_from_ms: Some(500),
            finished_to_ms: Some(400),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());

        let q = RunQuery {
            min_duration_ms: Some(-10),
            ..RunQuery::default()
        };
        assert!(q.into_filter().is_err());
    }

    #[test]
    fn run_query_parses_tags_and_prefix() {
        let q = RunQuery {
            name_prefix: Some("etl-".into()),
            has_tag_keys: Some("env, team , region".into()),
            offset: Some(25),
            limit: Some(50),
            ..RunQuery::default()
        };
        let f = q.into_filter().expect("valid query");
        assert_eq!(f.name_prefix.as_deref(), Some("etl-"));
        assert_eq!(f.has_tag_keys, vec!["env", "team", "region"]);
        assert_eq!(f.offset, 25);
        assert_eq!(f.limit, Some(50));
    }
}
