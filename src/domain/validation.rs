//! Structural validation of workflow definitions and run inputs.
//!
//! The API and CLI both funnel definitions through [`validate_definition`]
//! before persist; runs through [`validate_run_input`] at submission. Keeping
//! the rules here, away from the transport layers, is what makes the state
//! machine safe to trust downstream: a definition that survived validation
//! has a resolvable task graph, registered handlers, and sane policies.

use std::collections::BTreeSet;

use serde_json::Value as Json;

use super::dag;
use super::error::DomainError;
use super::workflow::{TaskSpec, WorkflowDef, MAX_TASKS};

/// Character pattern for definition names.
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Byte-size cap for a single run input payload.
pub const MAX_RUN_INPUT_BYTES: usize = 1 << 20;

/// Validates a full workflow definition, returning the first problem found.
///
/// Checks performed, in order:
/// 1. name rules;
/// 2. task list emptiness and count ceiling;
/// 3. unique task names;
/// 4. handler id format;
/// 5. per-task retry policy validity;
/// 6. dependency edges resolvable and acyclic;
/// 7. workflow timeout and ceilings;
/// 8. definition-specified hooks point at parseable URLs when present.
pub fn validate_definition(def: &WorkflowDef) -> Result<(), DomainError> {
    if !is_valid_name(&def.name) {
        return Err(DomainError::InvalidName(def.name.clone()));
    }
    super::workflow::sanitize_description(&def.description)?;
    if def.tasks.is_empty() {
        return Err(DomainError::EmptyTasks);
    }
    if def.tasks.len() > MAX_TASKS {
        return Err(DomainError::TooManyTasks);
    }

    // Unique task names.
    let mut seen = BTreeSet::new();
    for task in &def.tasks {
        if !seen.insert(task.name.as_str()) {
            return Err(DomainError::DuplicateTaskName(task.name.clone()));
        }
    }

    // Handler and policy sanity, plus timeout bounds.
    for task in &def.tasks {
        validate_task_basics(task)?;
    }

    // Graph edges (also covers cycle detection).
    dag::validate_edges(&def.tasks)?;

    // Whole-workflow timeout must be positive.
    if def.timeout_ms == 0 {
        return Err(DomainError::ZeroTimeout);
    }

    // Webhook URLs must at least parse as absolute http(s) URLs.
    for hook in def.hooks.all() {
        if let Some(url) = &hook.webhook_url {
            validate_webhook_url(url).map_err(DomainError::InvalidDefinition)?;
        }
    }

    Ok(())
}

fn validate_task_basics(task: &TaskSpec) -> Result<(), DomainError> {
    if task.timeout_ms == Some(0) {
        return Err(DomainError::ZeroTimeout);
    }
    if task.name.len() > super::workflow::MAX_TASK_NAME_LEN {
        return Err(DomainError::InvalidDefinition(format!(
            "task name {0:?} exceeds {1} characters",
            task.name,
            super::workflow::MAX_TASK_NAME_LEN
        )));
    }
    if let Some(retry) = &task.retry {
        retry
            .validate()
            .map_err(|e| DomainError::InvalidPolicy(e.to_string()))?;
    }
    Ok(())
}

/// Runs the workflow-level default retry policy through validation too.
pub fn validate_workflow_policy(def: &WorkflowDef) -> Result<(), DomainError> {
    def.retry
        .validate()
        .map_err(|e| DomainError::InvalidPolicy(e.to_string()))
}

/// Middleware-free URL validation: scheme must be `http:` or `https:` and a
/// host component must be present. No network access is performed.
pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("webhook url is not parseable: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "webhook url scheme must be http/https, got {other:?}"
            ))
        }
    }
    if parsed.host_str().is_none() {
        return Err("webhook url has no host".to_owned());
    }
    Ok(())
}

/// Validates a run submission payload by size.
pub fn validate_run_input(input: &Json) -> Result<(), DomainError> {
    let size = json_size(input);
    if size > MAX_RUN_INPUT_BYTES {
        return Err(DomainError::InputTooLarge {
            size,
            cap: MAX_RUN_INPUT_BYTES,
        });
    }
    Ok(())
}

/// Returns the serialized byte length of a JSON value.
pub fn json_size(value: &Json) -> usize {
    // Avoiding the allocation of `to_vec` for a measure keeps this O(1) for
    // already-constructed values; the cap check in validate_run_input is what
    // actually guards the wire.
    match value {
        Json::Null => 4,
        Json::Bool(_) => 4,
        Json::Number(n) => n.to_string().len(),
        Json::String(s) => s.len() + 2,
        Json::Array(items) => items.iter().map(json_size).sum(),
        Json::Object(map) => map
            .iter()
            .map(|(k, v)| k.len() + json_size(v))
            .sum::<usize>(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::HandlerId;
    use crate::domain::retry_policy::RetryPolicy;
    use crate::domain::workflow::{HookSpec, Hooks, TaskSpec};
    use std::collections::BTreeMap;

    fn base_def() -> WorkflowDef {
        let task = TaskSpec {
            name: "t1".to_owned(),
            handler: HandlerId::from_validated("runvane.echo".to_owned()),
            input: Json::Null,
            depends_on: vec![],
            timeout_ms: None,
            retry: None,
            meta: BTreeMap::new(),
        };
        WorkflowDef {
            id: crate::domain::ids::WorkflowId::from_validated("wf_x".to_owned()),
            tenant: "acme".to_owned(),
            name: "nightly".to_owned(),
            version: 1,
            description: String::new(),
            tasks: vec![task],
            timeout_ms: 60_000,
            retry: RetryPolicy::fixed(3, 1_000),
            default_priority: super::super::status::Priority::default(),
            hooks: Hooks::default(),
            tags: BTreeMap::new(),
            spec_version: 1,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn valid_def_passes() {
        assert!(validate_definition(&base_def()).is_ok());
    }

    #[test]
    fn invalid_name_rejected() {
        let mut def = base_def();
        def.name = "Nightly".to_owned();
        assert!(matches!(
            validate_definition(&def),
            Err(DomainError::InvalidName(_))
        ));
    }

    #[test]
    // [P2P] RV-030 witness (empty task graph stays rejected on broken and fixed).
    fn empty_tasks_rejected() {
        let mut def = base_def();
        def.tasks.clear();
        assert_eq!(validate_definition(&def), Err(DomainError::EmptyTasks));
    }

    #[test]
    // [P2P] RV-030 witness (duplicate/dangling names are rejected both ways).
    fn duplicate_task_names_rejected() {
        let mut def = base_def();
        let dup = def.tasks[0].clone();
        def.tasks.push(dup);
        assert!(matches!(
            validate_definition(&def),
            Err(DomainError::DuplicateTaskName(_))
        ));
    }

    #[test]
    // [P2P] RV-030 witness (graph cycle detection is stable across fixes).
    fn cycle_rejected() {
        let mut def = base_def();
        def.tasks.push(TaskSpec {
            name: "t2".to_owned(),
            handler: HandlerId::from_validated("runvane.echo".to_owned()),
            input: Json::Null,
            depends_on: vec!["t1".to_owned()],
            timeout_ms: None,
            retry: None,
            meta: BTreeMap::new(),
        });
        def.tasks[0].depends_on = vec!["t2".to_owned()];
        assert_eq!(validate_definition(&def), Err(DomainError::CycleDetected));
    }

    #[test]
    fn unknown_dependency_rejected() {
        let mut def = base_def();
        def.tasks[0].depends_on = vec!["ghost".to_owned()];
        assert!(matches!(
            validate_definition(&def),
            Err(DomainError::UnknownDependency(..))
        ));
    }

    #[test]
    fn zero_timeout_rejected() {
        let mut def = base_def();
        def.timeout_ms = 0;
        assert_eq!(validate_definition(&def), Err(DomainError::ZeroTimeout));
    }

    #[test]
    fn bad_retry_policy_rejected() {
        let mut def = base_def();
        def.tasks[0].retry = Some(RetryPolicy {
            max_attempts: 0,
            ..RetryPolicy::default()
        });
        assert!(matches!(
            validate_definition(&def),
            Err(DomainError::InvalidPolicy(_))
        ));
    }

    #[test]
    fn bad_hook_url_rejected() {
        let mut def = base_def();
        def.hooks.on_success = vec![HookSpec {
            webhook_url: Some("not-a-url".to_owned()),
            ..HookSpec {
                webhook_url: None,
                event_filter: None,
                headers: BTreeMap::new(),
            }
        }];
        assert!(validate_definition(&def).is_err());
    }

    #[test]
    fn name_pattern_edges() {
        assert!(is_valid_name("a"));
        assert!(is_valid_name("a-b-c"));
        assert!(is_valid_name("a1"));
        assert!(!is_valid_name("1a"));
        assert!(!is_valid_name("a b"));
        assert!(!is_valid_name("a_b"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name(&"a".repeat(65)));
    }

    #[test]
    fn webhook_url_validation_edges() {
        assert!(validate_webhook_url("https://hooks.example.com/endpoint").is_ok());
        assert!(validate_webhook_url("http://localhost:8080/x").is_ok());
        assert!(validate_webhook_url("ftp://x").is_err());
        assert!(validate_webhook_url("https://").is_err());
        assert!(validate_webhook_url("nonsense").is_err());
    }

    #[test]
    fn run_input_size_guard() {
        let big = serde_json::json!({ "blob": "x".repeat(MAX_RUN_INPUT_BYTES + 1) });
        assert!(matches!(
            validate_run_input(&big),
            Err(DomainError::InputTooLarge { .. })
        ));
        assert!(validate_run_input(&Json::Null).is_ok());
    }
}
