//! Task-graph (DAG) utilities.
//!
//! Tasks reference each other through `TaskSpec::depends_on`. The platform
//! needs two things from that graph: a validation pass that rejects unknown
//! dependencies and cycles, and an execution order for dispatch planning.
//! Both are pure functions over task names so they can be exhaustively
//! unit- and property-tested.

use std::collections::{BTreeMap, BTreeSet};

use super::{error::DomainError, workflow::TaskSpec};

/// Topological ordering of task names (dependency-first). Returns names in an
/// order where every task appears after all of its dependencies.
///
/// The order is deterministic: ties are broken lexicographically so the same
/// definition always produces the same plan.
pub fn topo_order(tasks: &[TaskSpec]) -> Result<Vec<String>, DomainError> {
    // name -> set of task names it depends on
    let mut deps: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for task in tasks {
        deps.entry(task.name.as_str()).or_default();
        for dep in &task.depends_on {
            deps.get_mut(task.name.as_str())
                .expect("just inserted")
                .insert(dep.as_str());
        }
    }

    // Sanity: every depended-on name must be a real task. Guarded here so
    // topo_order stays total on malformed input.
    for (name, deps_of_name) in &deps {
        for dep in deps_of_name {
            if !deps.contains_key(*dep) {
                return Err(DomainError::UnknownDependency(
                    (*dep).to_owned(),
                    (*name).to_owned(),
                ));
            }
        }
    }

    // Kahn's algorithm with a lexicographic frontier for determinism.
    let mut ready: BTreeSet<String> = deps
        .iter()
        .filter(|(_, d)| d.is_empty())
        .map(|(k, _)| (*k).to_owned())
        .collect();
    let mut remaining: BTreeMap<&str, BTreeSet<&str>> = deps;
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::with_capacity(tasks.len());

    while let Some(next) = ready.iter().next().cloned() {
        ready.remove(&next);
        emitted.insert(next.clone());
        order.push(next.clone());

        // Drop `next` from dependent sets; sets left empty become ready.
        let draining: Vec<String> = remaining
            .iter_mut()
            .filter_map(|(name, d)| {
                if d.remove(next.as_str()) && d.is_empty() {
                    Some((*name).to_owned())
                } else {
                    None
                }
            })
            .collect();
        for name in draining {
            if !emitted.contains(&name) {
                ready.insert(name);
            }
        }
    }

    if order.len() != remaining.len() {
        return Err(DomainError::CycleDetected);
    }
    Ok(order)
}

/// Whether the task graph is acyclic and fully resolvable.
pub fn is_acyclic(tasks: &[TaskSpec]) -> bool {
    topo_order(tasks).is_ok()
}

/// Validates that dependency edges only reference known tasks and never form
/// a cycle. Returns the first error encountered.
pub fn validate_edges(tasks: &[TaskSpec]) -> Result<(), DomainError> {
    topo_order(tasks).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::HandlerId;
    use crate::domain::workflow::TaskSpec;

    fn spec(name: &str, depends: &[&str]) -> TaskSpec {
        TaskSpec {
            name: name.to_owned(),
            handler: HandlerId::from_validated("runvane.echo".to_owned()),
            input: serde_json::Value::Null,
            depends_on: depends.iter().map(|s| s.to_string()).collect(),
            timeout_ms: None,
            retry: None,
            meta: Default::default(),
        }
    }

    #[test]
    fn empty_graph_is_ordered() {
        assert_eq!(topo_order(&[]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn independent_tasks_are_lexicographic() {
        let tasks = vec![spec("b", &[]), spec("a", &[]), spec("c", &[])];
        assert_eq!(topo_order(&tasks).unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn diamond_resolution() {
        let tasks = vec![
            spec("root", &[]),
            spec("l", &["root"]),
            spec("r", &["root"]),
            spec("leaf", &["l", "r"]),
        ];
        let order = topo_order(&tasks).unwrap();
        assert_eq!(order.first().unwrap(), "root");
        assert_eq!(order.last().unwrap(), "leaf");
        // l before leaf, r before leaf, root before both.
        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(pos("root") < pos("l"));
        assert!(pos("root") < pos("r"));
        assert!(pos("l") < pos("leaf"));
        assert!(pos("r") < pos("leaf"));
    }

    #[test]
    fn cycle_is_detected() {
        let tasks = vec![spec("a", &["b"]), spec("b", &["a"])];
        assert_eq!(topo_order(&tasks).unwrap_err(), DomainError::CycleDetected);
        assert!(!is_acyclic(&tasks));
    }

    #[test]
    fn self_dependency_is_a_cycle() {
        let tasks = vec![spec("a", &["a"])];
        assert_eq!(topo_order(&tasks).unwrap_err(), DomainError::CycleDetected);
    }

    #[test]
    fn unknown_dependency_is_reported() {
        let tasks = vec![spec("a", &["ghost"])];
        assert_eq!(
            topo_order(&tasks).unwrap_err(),
            DomainError::UnknownDependency("ghost".into(), "a".into())
        );
    }

    #[test]
    fn chain_orders_start_to_finish() {
        let tasks = vec![
            spec("one", &[]),
            spec("two", &["one"]),
            spec("three", &["two"]),
        ];
        assert_eq!(topo_order(&tasks).unwrap(), vec!["one", "two", "three"]);
    }
}