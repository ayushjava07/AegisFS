//! Ready-set selection for task dispatch.
//!
//! Given the definition's task graph and the current status of every task in
//! the run, which previously-pending tasks may dispatch now? Deterministic and
//! pure, so the executor can be tested exhaustively without a store.

use std::collections::BTreeMap;

use crate::domain::status::TaskStatus;
use crate::domain::workflow::WorkflowDef;

/// Names of tasks that may dispatch at this step:
///
/// * still `Pending` with every dependency `Succeeded` (a skipped dependency
///   means the chain already aborted, so the task must be skipped, not run);
///
/// * `Failed` tasks, which the retry path re-runs on a later attempt (the task
///   machine allows `Failed -> Running`. The caller filters out tasks whose
///   attempt budget is already exhausted; topology alone cannot see budgets.
pub fn ready_tasks(def: &WorkflowDef, states: &BTreeMap<String, TaskStatus>) -> Vec<String> {
    let mut ready = Vec::new();
    for task in &def.tasks {
        let name = &task.name;
        let status = states.get(name).copied().unwrap_or(TaskStatus::Pending);
        match status {
            TaskStatus::Failed => ready.push(name.clone()),
            TaskStatus::Pending => {
                let deps_ok = task.depends_on.iter().all(|dep| {
                    matches!(states.get(dep).copied(), Some(TaskStatus::Succeeded))
                });
                if deps_ok {
                    ready.push(name.clone());
                }
            }
            _ => {}
        }
    }
    // Deterministic dispatch order: definition order equals lexical order in
    // practice; sort to be explicit for arbitrarily-ordered definitions.
    ready.sort();
    ready
}

/// Marks tasks whose dispatch can never proceed: `Pending` tasks with at least
/// one dependency that is not `Succeeded` (failed or transitively skipped) are
/// themselves skipped. Iterates until a fixpoint so chains collapse entirely.
/// Returns the new status map; the caller persists the changes.
pub fn pending_to_skip(
    def: &WorkflowDef,
    states: &BTreeMap<String, TaskStatus>,
) -> BTreeMap<String, TaskStatus> {
    let mut next = states.clone();
    loop {
        let mut changed = false;
        for task in &def.tasks {
            let status = next.get(&task.name).copied().unwrap_or(TaskStatus::Pending);
            if status != TaskStatus::Pending {
                continue;
            }
            let dep_blocked = task.depends_on.iter().any(|dep| {
                !matches!(next.get(dep).copied(), Some(TaskStatus::Succeeded))
            });
            if dep_blocked {
                next.insert(task.name.clone(), TaskStatus::Skipped);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{HandlerId, WorkflowId};
    use crate::domain::retry_policy::RetryPolicy;
    use crate::domain::workflow::{Hooks, TaskSpec};
    use serde_json::json;

    fn def(tasks: Vec<(String, Vec<String>)>) -> WorkflowDef {
        let tasks: Vec<TaskSpec> = tasks
            .into_iter()
            .map(|(name, depends_on)| TaskSpec {
                name,
                handler: HandlerId::from_validated("runvane.noop".into()),
                input: json!({}),
                depends_on,
                timeout_ms: None,
                retry: None,
                meta: Default::default(),
            })
            .collect();
        WorkflowDef {
            id: WorkflowId::from_validated("wf_x".into()),
            tenant: "acme".into(),
            name: "pipeline".into(),
            version: 1,
            description: String::new(),
            tasks,
            timeout_ms: 60_000,
            retry: RetryPolicy::default(),
            default_priority: Default::default(),
            hooks: Hooks::default(),
            tags: Default::default(),
            spec_version: 1,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn states(pairs: &[(&str, TaskStatus)]) -> BTreeMap<String, TaskStatus> {
        pairs.iter().map(|(n, s)| (n.to_string(), *s)).collect()
    }

    #[test]
    fn linear_chain_ready_set() {
        let d = def(vec![
            ("a".into(), vec![]),
            ("b".into(), vec!["a".into()]),
            ("c".into(), vec!["b".into()]),
        ]);
        // Nothing started: only `a` is ready.
        let s = states(&[]);
        assert_eq!(ready_tasks(&d, &s), vec!["a".to_string()]);
        // a done -> b ready; c still blocked on b.
        let s = states(&[("a", TaskStatus::Succeeded)]);
        assert_eq!(ready_tasks(&d, &s), vec!["b".to_string()]);
        // a,b done -> c ready.
        let s = states(&[("a", TaskStatus::Succeeded), ("b", TaskStatus::Succeeded)]);
        assert_eq!(ready_tasks(&d, &s), vec!["c".to_string()]);
    }

    #[test]
    fn depends_ready_only_after_all_deps() {
        let d = def(vec![
            ("left".into(), vec![]),
            ("right".into(), vec![]),
            ("joint".into(), vec!["left".into(), "right".into()]),
        ]);
        let s = states(&[("left", TaskStatus::Succeeded)]);
        assert_eq!(ready_tasks(&d, &s), vec!["right".to_string()]);
        let s2 = states(&[("left", TaskStatus::Succeeded), ("right", TaskStatus::Succeeded)]);
        assert_eq!(ready_tasks(&d, &s2), vec!["joint".to_string()]);
    }

    #[test]
    fn failed_dep_skips_pending_tasks() {
        let d = def(vec![
            ("a".into(), vec![]),
            ("b".into(), vec!["a".into()]),
            ("c".into(), vec!["b".into()]),
        ]);
        let s = states(&[("a", TaskStatus::Failed)]);
        let next = pending_to_skip(&d, &s);
        assert_eq!(next["b"], TaskStatus::Skipped);
        // Skipped b blocks c transitively: c is skipped, never dispatched.
        assert_eq!(next["c"], TaskStatus::Skipped);
    }

    #[test]
    fn skipped_dep_blocks_rather_than_runs() {
        let d = def(vec![
            ("a".into(), vec![]),
            ("b".into(), vec!["a".into()]),
        ]);
        // b is Skipped -> it must not be selectable as ready for any consumer.
        let s = states(&[("a", TaskStatus::Succeeded), ("b", TaskStatus::Skipped)]);
        // a already done, b skipped: nothing more to dispatch.
        assert!(ready_tasks(&d, &s).is_empty());
    }

    #[test]
    fn running_tasks_are_not_ready() {
        let d = def(vec![("a".into(), vec![]), ("b".into(), vec!["a".into()])]);
        let s = states(&[("a", TaskStatus::Running)]);
        assert!(ready_tasks(&d, &s).is_empty());
    }

    #[test]
    fn failed_tasks_are_selected_for_retry() {
        let d = def(vec![
            ("a".into(), vec![]),
            ("b".into(), vec!["a".into()]),
        ]);
        let s = states(&[("a", TaskStatus::Failed), ("b", TaskStatus::Succeeded)]);
        assert_eq!(ready_tasks(&d, &s), vec!["a".to_string()]);
    }
}