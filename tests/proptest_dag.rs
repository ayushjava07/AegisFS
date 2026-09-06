//! Property-based tests for DAG validation, cycle detection, and topological sorting.

use proptest::prelude::*;
use runvane::domain::dag::topo_order;
use runvane::domain::error::DomainError;
use runvane::domain::ids::HandlerId;
use runvane::domain::workflow::TaskSpec;
use std::collections::{BTreeMap, HashMap};

fn make_task(name: &str, deps: Vec<String>) -> TaskSpec {
    TaskSpec {
        name: name.to_string(),
        handler: HandlerId::from_validated("testhandler".to_string()),
        input: serde_json::json!({}),
        depends_on: deps,
        timeout_ms: None,
        retry: None,
        meta: BTreeMap::new(),
    }
}

proptest! {
    #[test]
    fn linear_chains_are_always_valid_and_ordered(n in 1usize..30) {
        let mut tasks = Vec::with_capacity(n);
        for i in 0..n {
            let name = format!("task_{i:03}");
            let deps = if i == 0 {
                vec![]
            } else {
                vec![format!("task_{:03}", i - 1)]
            };
            tasks.push(make_task(&name, deps));
        }

        let order = topo_order(&tasks).expect("linear chain must be valid");
        prop_assert_eq!(order.len(), n);
        for (i, name) in order.iter().enumerate() {
            prop_assert_eq!(name, &format!("task_{i:03}"));
        }
    }

    #[test]
    fn back_edge_cycle_is_always_detected(n in 2usize..25) {
        let mut tasks = Vec::with_capacity(n);
        for i in 0..n {
            let name = format!("task_{i:03}");
            let mut deps = if i == 0 {
                vec![]
            } else {
                vec![format!("task_{:03}", i - 1)]
            };
            if i == 0 {
                // Introduce a cycle: task_0 depends on the last task
                deps.push(format!("task_{:03}", n - 1));
            }
            tasks.push(make_task(&name, deps));
        }

        let err = topo_order(&tasks).unwrap_err();
        prop_assert!(matches!(err, DomainError::CycleDetected));
    }

    #[test]
    fn self_dependency_is_always_rejected(name_idx in 0usize..100) {
        let name = format!("task_{name_idx}");
        let task = make_task(&name, vec![name.clone()]);
        let err = topo_order(&[task]).unwrap_err();
        prop_assert!(matches!(err, DomainError::CycleDetected));
    }

    #[test]
    fn unknown_dependency_is_always_rejected(name_idx in 0usize..50) {
        let name = format!("task_{name_idx}");
        let task = make_task(&name, vec!["nonexistent_task".to_string()]);
        let err = topo_order(&[task]).unwrap_err();
        prop_assert!(matches!(err, DomainError::UnknownDependency(..)));
    }

    #[test]
    fn arbitrary_forward_dag_produces_valid_topological_sort(n in 2usize..20) {
        let mut tasks = Vec::with_capacity(n);
        for i in 0..n {
            let name = format!("t_{i}");
            // Forward-only edges: each task i can only depend on a subset of tasks j < i
            let mut deps = Vec::new();
            for j in 0..i {
                // Deterministic pseudo-selection of edges
                if (i + j) % 2 == 0 {
                    deps.push(format!("t_{j}"));
                }
            }
            tasks.push(make_task(&name, deps));
        }

        let order = topo_order(&tasks).expect("arbitrary forward DAG must be acyclic");
        prop_assert_eq!(order.len(), n);

        // Map each task name to its position in the topological order
        let positions: HashMap<&str, usize> = order
            .iter()
            .enumerate()
            .map(|(pos, name)| (name.as_str(), pos))
            .collect();

        // In a valid topological sort, dependencies must precede the dependent task
        for task in &tasks {
            let task_pos = positions[task.name.as_str()];
            for dep in &task.depends_on {
                let dep_pos = positions[dep.as_str()];
                prop_assert!(
                    dep_pos < task_pos,
                    "dep {} at {} must appear before task {} at {}",
                    dep,
                    dep_pos,
                    task.name,
                    task_pos
                );
            }
        }
    }
}
