//! Workflow definitions and task specifications.
//!
//! Everything an operator can declare about *how* a piece of work should run:
//! the task graph, handler wiring, timeouts, retry policy, completion hooks,
//! and free-form tags. Definitions are versioned (`tenant`, `name`, `version`)
//! and immutable once stored; a modified definition is a new version.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::error::DomainError;
use super::ids::{HandlerId, WorkflowId};
use super::retry_policy::RetryPolicy;
use super::status::Priority;

/// Version of the definition document JSON contract.
pub const SPEC_VERSION: u32 = 1;

/// Latest format version field prefix used when serializing defs.
pub const MAX_TASK_NAME_LEN: usize = 64;
/// Maximum number of tasks in a workflow.
pub const MAX_TASKS: usize = 256;
/// Maximum size of a definition's static task input, in bytes.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;

/// A complete workflow definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// Surrogate record id (auto-assigned on creation).
    pub id: WorkflowId,
    /// Owning tenant; part of the natural key (`tenant`, `name`, `version`).
    pub tenant: String,
    /// Definition name; unique per tenant across versions.
    pub name: String,
    /// Monotonic version, bumped on every mutation.
    pub version: u32,
    /// Free-form description for operators.
    pub description: String,
    /// The ordered task graph. Order is only significant for display; the
    /// dependency edges in `TaskSpec::depends_on` decide execution order.
    pub tasks: Vec<TaskSpec>,
    /// Whole-workflow deadline measured from first dispatch.
    pub timeout_ms: u64,
    /// Default retry policy applied to tasks without their own override.
    pub retry: RetryPolicy,
    /// Default submission priority for new runs.
    pub default_priority: Priority,
    /// Completion hooks fired at run lifecycle transitions.
    pub hooks: Hooks,
    /// Operator tags; also used by filter queries.
    pub tags: BTreeMap<String, String>,
    /// Serialization contract version of the document.
    pub spec_version: u32,
    /// Creation timestamp, epoch milliseconds.
    pub created_at_ms: i64,
    /// Last-update timestamp, epoch milliseconds.
    pub updated_at_ms: i64,
}

impl WorkflowDef {
    /// A cheap structural sanity check usable before persisting: name/task
    /// counts within platform limits.
    pub fn is_minimally_valid(&self) -> bool {
        !self.name.is_empty() && !self.tasks.is_empty() && self.tasks.len() <= MAX_TASKS
    }
}

/// Specification of a single task inside a workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Task name; unique within the workflow. Reference key for `depends_on`.
    pub name: String,
    /// The handler plugin responsible for executing the task.
    pub handler: HandlerId,
    /// Static input passed to the handler; may be a templated JSON value.
    pub input: Json,
    /// Task names that must succeed before this one dispatches.
    pub depends_on: Vec<String>,
    /// Per-task timeout override. Absent means "inherit from the run".
    pub timeout_ms: Option<u64>,
    /// Per-task retry override. Absent means "use the workflow default".
    pub retry: Option<RetryPolicy>,
    /// Free-form metadata (annotations, per-handler options).
    pub meta: BTreeMap<String, Json>,
}

impl TaskSpec {
    /// Returns the effective retry policy for this task.
    pub fn effective_retry<'a>(&'a self, workflow_default: &'a RetryPolicy) -> &'a RetryPolicy {
        self.retry.as_ref().unwrap_or(workflow_default)
    }

    /// Returns the effective timeout in milliseconds for this task.
    pub fn effective_timeout_ms(&self, workflow_default_ms: u64) -> u64 {
        self.timeout_ms.unwrap_or(workflow_default_ms)
    }
}

/// Completion hooks attached to a definition, fired by the event subsystem.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hooks {
    /// Fired when a run transitions to `Running`.
    pub on_start: Vec<HookSpec>,
    /// Fired when a run reaches `Succeeded`.
    pub on_success: Vec<HookSpec>,
    /// Fired when a run reaches `Failed` or `TimedOut`.
    pub on_failure: Vec<HookSpec>,
    /// Fired when a run is cancelled.
    pub on_cancel: Vec<HookSpec>,
}

/// A single delivery endpoint/hook binding.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HookSpec {
    /// Webhook URL to POST the event to. Optional when paired with internal
    /// subscribers.
    pub webhook_url: Option<String>,
    /// Event name selector; empty matches every event kind.
    pub event_filter: Option<String>,
    /// Additional static headers on the delivery request.
    pub headers: BTreeMap<String, String>,
}

impl Hooks {
    /// Enumerates every hook spec regardless of trigger, in a stable order
    /// (start, success, failure, cancel).
    pub fn all(&self) -> impl Iterator<Item = &HookSpec> {
        self.on_start
            .iter()
            .chain(self.on_success.iter())
            .chain(self.on_failure.iter())
            .chain(self.on_cancel.iter())
    }
}

impl Default for WorkflowDef {
    fn default() -> Self {
        Self {
            id: WorkflowId::from_validated("wf_default".to_owned()),
            tenant: String::new(),
            name: String::new(),
            version: 1,
            description: String::new(),
            tasks: Vec::new(),
            timeout_ms: 3_600_000,
            retry: RetryPolicy::default(),
            default_priority: Priority::default(),
            hooks: Hooks::default(),
            tags: BTreeMap::new(),
            spec_version: SPEC_VERSION,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }
}

/// Common redaction of untrusted values when echoing a definition back.
pub(crate) fn sanitize_description(description: &str) -> Result<(), DomainError> {
    if description.chars().count() > 512 {
        return Err(DomainError::DescriptionTooLong);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, handler: &str, depends: &[&str]) -> TaskSpec {
        TaskSpec {
            name: name.to_owned(),
            handler: HandlerId::parse(handler).unwrap(),
            input: Json::Null,
            depends_on: depends.iter().map(|s| s.to_string()).collect(),
            timeout_ms: None,
            retry: None,
            meta: BTreeMap::new(),
        }
    }

    #[test]
    fn effective_retry_falls_back_to_workflow_default() {
        let default = RetryPolicy::fixed(2, 1_000);
        let mut task = spec("t", "runvane.http.call", &[]);
        let over = RetryPolicy::fixed(7, 1_000);
        task.retry = Some(over.clone());

        assert_eq!(task.effective_retry(&default), &over);
        task.retry = None;
        assert_eq!(task.effective_retry(&default), &default);
    }

    #[test]
    fn effective_timeout_override_beats_default() {
        let task = spec("t", "runvane.http.call", &[]);
        assert_eq!(task.effective_timeout_ms(5_000), 5_000);
        let mut with = task.clone();
        with.timeout_ms = Some(42);
        assert_eq!(with.effective_timeout_ms(5_000), 42);
    }

    #[test]
    fn hooks_iterate_in_stable_order() {
        let hooks = Hooks {
            on_start: vec![HookSpec {
                webhook_url: Some("u1".into()),
                ..Default::default()
            }],
            on_success: vec![HookSpec {
                webhook_url: Some("u2".into()),
                ..Default::default()
            }],
            on_failure: vec![HookSpec {
                webhook_url: Some("u3".into()),
                ..Default::default()
            }],
            on_cancel: vec![HookSpec {
                webhook_url: Some("u4".into()),
                ..Default::default()
            }],
        };
        let urls: Vec<_> = hooks
            .all()
            .filter_map(|h| h.webhook_url.as_deref())
            .collect();
        assert_eq!(urls, vec!["u1", "u2", "u3", "u4"]);
    }

    #[test]
    fn minimal_validity_guards() {
        let mut def = WorkflowDef {
            id: WorkflowId::from_validated("wf_x".to_owned()),
            tenant: "acme".to_owned(),
            name: "nightly".to_owned(),
            tasks: vec![spec("a", "runvane.echo", &[])],
            ..Default::default()
        };
        assert!(def.is_minimally_valid());
        def.name.clear();
        assert!(!def.is_minimally_valid());
    }

    #[test]
    fn description_length_capped() {
        assert!(sanitize_description(&"x".repeat(512)).is_ok());
        assert_eq!(
            sanitize_description(&"x".repeat(513)),
            Err(DomainError::DescriptionTooLong)
        );
    }
}
