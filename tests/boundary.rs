//! Comprehensive boundary-condition test suite covering input caps, length limits,
//! recursion ceilings, and pagination boundaries.

use runvane::api::payloads::{RunQuery, MAX_LIST_LIMIT};
use runvane::domain::error::DomainError;
use runvane::domain::ids::{HandlerId, WorkflowId};
use runvane::domain::retry_policy::RetryPolicy;
use runvane::domain::status::Priority;
use runvane::domain::validation::{
    json_depth, validate_definition, validate_run_input, MAX_JSON_DEPTH, MAX_RUN_INPUT_BYTES,
};
use runvane::domain::workflow::{Hooks, TaskSpec, WorkflowDef, MAX_TASKS, MAX_TASK_NAME_LEN};
use serde_json::json;
use std::collections::BTreeMap;

fn make_wf(tasks: Vec<TaskSpec>) -> WorkflowDef {
    WorkflowDef {
        id: WorkflowId::from_validated("wf_boundary001".to_string()),
        tenant: "tenant-test".to_string(),
        name: "wf-boundary".to_string(),
        version: 1,
        description: "boundary testing workflow".to_string(),
        tasks,
        timeout_ms: 10_000,
        retry: RetryPolicy::fixed(1, 100),
        default_priority: Priority::Normal,
        hooks: Hooks::default(),
        tags: BTreeMap::new(),
        spec_version: 1,
        created_at_ms: 1_000_000,
        updated_at_ms: 1_000_000,
    }
}

fn make_task(name: &str) -> TaskSpec {
    TaskSpec {
        name: name.to_string(),
        handler: HandlerId::from_validated("testhandler".to_string()),
        input: json!({}),
        depends_on: vec![],
        timeout_ms: Some(5_000),
        retry: None,
        meta: BTreeMap::new(),
    }
}

#[test]
fn empty_task_list_boundary() {
    let wf = make_wf(vec![]);
    let err = validate_definition(&wf).unwrap_err();
    assert!(matches!(err, DomainError::EmptyTasks));
}

#[test]
fn max_tasks_ceiling_boundary() {
    // Exactly MAX_TASKS (100) must succeed
    let tasks_100: Vec<TaskSpec> = (0..MAX_TASKS)
        .map(|i| make_task(&format!("task_{i:03}")))
        .collect();
    let wf_100 = make_wf(tasks_100);
    assert!(validate_definition(&wf_100).is_ok());

    // MAX_TASKS + 1 (101) must fail
    let tasks_101: Vec<TaskSpec> = (0..=MAX_TASKS)
        .map(|i| make_task(&format!("task_{i:03}")))
        .collect();
    let wf_101 = make_wf(tasks_101);
    let err = validate_definition(&wf_101).unwrap_err();
    assert!(matches!(err, DomainError::TooManyTasks));
}

#[test]
fn task_name_length_boundary() {
    // Exactly MAX_TASK_NAME_LEN (64) characters must pass
    let name_64 = "a".repeat(MAX_TASK_NAME_LEN);
    let wf_valid = make_wf(vec![make_task(&name_64)]);
    assert!(validate_definition(&wf_valid).is_ok());

    // MAX_TASK_NAME_LEN + 1 (65) characters must fail
    let name_65 = "a".repeat(MAX_TASK_NAME_LEN + 1);
    let wf_invalid = make_wf(vec![make_task(&name_65)]);
    let err = validate_definition(&wf_invalid).unwrap_err();
    assert!(matches!(err, DomainError::InvalidDefinition(_)));
}

#[test]
fn timeout_zero_boundary() {
    let mut wf = make_wf(vec![make_task("step1")]);
    wf.timeout_ms = 0;
    let err = validate_definition(&wf).unwrap_err();
    assert!(matches!(err, DomainError::ZeroTimeout));

    let mut task_zero_timeout = make_task("step_zero");
    task_zero_timeout.timeout_ms = Some(0);
    let wf_task_zero = make_wf(vec![task_zero_timeout]);
    let err_task = validate_definition(&wf_task_zero).unwrap_err();
    assert!(matches!(err_task, DomainError::ZeroTimeout));
}

#[test]
fn input_payload_size_boundary() {
    // Payload under limit passes
    let payload_ok = json!({ "blob": "a".repeat(10_000) });
    assert!(validate_run_input(&payload_ok).is_ok());

    // Payload exceeding MAX_RUN_INPUT_BYTES fails
    let payload_too_large = json!({ "blob": "a".repeat(MAX_RUN_INPUT_BYTES + 10) });
    let err = validate_run_input(&payload_too_large).unwrap_err();
    assert!(matches!(err, DomainError::InputTooLarge { .. }));
}

#[test]
fn json_nesting_depth_boundary() {
    // Depth exactly MAX_JSON_DEPTH passes
    let mut nested = json!("leaf");
    for _ in 0..MAX_JSON_DEPTH {
        nested = json!([nested]);
    }
    assert_eq!(json_depth(&nested, 0), MAX_JSON_DEPTH);
    assert!(validate_run_input(&nested).is_ok());

    // Depth exceeding MAX_JSON_DEPTH fails
    let nested_too_deep = json!([nested]);
    assert_eq!(json_depth(&nested_too_deep, 0), MAX_JSON_DEPTH + 1);
    let err = validate_run_input(&nested_too_deep).unwrap_err();
    assert!(matches!(err, DomainError::InputTooDeep { .. }));
}

#[test]
fn pagination_limit_boundary() {
    // Limit 1 passes
    let q1 = RunQuery {
        limit: Some(1),
        ..RunQuery::default()
    };
    assert_eq!(q1.into_filter().unwrap().limit, Some(1));

    // Limit MAX_LIST_LIMIT passes
    let q_max = RunQuery {
        limit: Some(MAX_LIST_LIMIT),
        ..RunQuery::default()
    };
    assert_eq!(q_max.into_filter().unwrap().limit, Some(MAX_LIST_LIMIT));

    // Limit 0 fails
    let q_zero = RunQuery {
        limit: Some(0),
        ..RunQuery::default()
    };
    assert!(q_zero.into_filter().is_err());

    // Limit MAX_LIST_LIMIT + 1 fails
    let q_overflow = RunQuery {
        limit: Some(MAX_LIST_LIMIT + 1),
        ..RunQuery::default()
    };
    assert!(q_overflow.into_filter().is_err());
}
