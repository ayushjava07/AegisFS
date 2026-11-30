//! Shared builders for tests and benchmarks.
//!
//! Constructing a legal [`Run`] requires filling a dozen fields; these helpers
//! make that painless and keep fixture shapes consistent across the suite, so
//! a defect in one fixture cannot silently hide a behavioral difference.

use std::collections::BTreeMap;

use serde_json::json;

use crate::domain::ids::{RunId, TaskRunId};
use crate::domain::run::{Run, TaskRun};
use crate::domain::status::{RunStatus, TaskStatus};

/// A synthetic run with minimal fixed fields.
pub fn run(id: &str, tenant: &str, def_name: &str, status: RunStatus, at_ms: i64) -> Run {
    Run {
        id: RunId::from_validated(id.to_owned()),
        tenant: tenant.to_owned(),
        def_name: def_name.to_owned(),
        def_version: 1,
        input: json!({"probe": true}),
        status,
        attempts: if status == RunStatus::Queued { 0 } else { 1 },
        next_attempt_at_ms: None,
        deadline_at_ms: None,
        started_at_ms: None,
        finished_at_ms: None,
        error: None,
        output: None,
        tags: BTreeMap::from([("env".to_owned(), "test".to_owned())]),
        created_at_ms: at_ms,
        run_number: 1,
    }
}

/// A task-run fixture for a given run.
///
/// The id body is derived from the run's body + task name so every produced id
/// stays valid base32hex (`0-9a-v`) and round-trips through strict parsing.
pub fn task(run_id: &RunId, name: &str, status: TaskStatus) -> TaskRun {
    let run_body = run_id.as_str().strip_prefix("rn_").unwrap_or("");
    TaskRun {
        id: TaskRunId::from_validated(format!("tr_{run_body}{name}")),
        run_id: run_id.clone(),
        task_name: name.to_owned(),
        status,
        attempts: if status == TaskStatus::Pending { 0 } else { 1 },
        last_error: None,
        started_at_ms: None,
        finished_at_ms: None,
        output: None,
    }
}
