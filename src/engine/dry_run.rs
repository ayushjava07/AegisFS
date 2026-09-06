//! Static simulation and dry-run analysis engine for workflow definitions.
//!
//! Provides compile-time and pre-flight validation of workflow DAGs:
//! - Wave/stage decomposition (parallel dispatch tiers)
//! - Critical path calculation and parallelism bounds
//! - Template variable reference analysis and dependency reachability
//! - Human-readable ASCII stage visualization and JSON-serializable reports

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::domain::dag::topo_order;
use crate::domain::error::DomainError;
use crate::domain::workflow::WorkflowDef;

/// A parallel execution tier in the simulated workflow run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunStage {
    /// Zero-based stage index.
    pub stage_index: usize,
    /// Task names that can dispatch concurrently in this stage.
    pub tasks: Vec<String>,
}

/// Comprehensive report produced by workflow dry-run analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunReport {
    /// Tenant identifier.
    pub tenant: String,
    /// Workflow definition name.
    pub workflow_name: String,
    /// Definition version.
    pub version: u32,
    /// Total number of tasks.
    pub total_tasks: usize,
    /// Maximum number of tasks that can execute concurrently.
    pub max_parallelism: usize,
    /// Execution stages grouped by dependency wave.
    pub stages: Vec<DryRunStage>,
    /// Task names forming the longest sequential critical path.
    pub critical_path: Vec<String>,
    /// Discovered template expression placeholders across all task inputs.
    pub variable_references: Vec<String>,
    /// Static analysis warnings (e.g. forward references, loose dependencies).
    pub warnings: Vec<String>,
}

impl DryRunReport {
    /// Formats the dry-run report as a human-readable terminal overview.
    pub fn format_text(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "=== Workflow Dry-Run Report: {}/{} (v{}) ===",
            self.tenant, self.workflow_name, self.version
        );
        let _ = writeln!(
            out,
            "Tasks: {} | Max Parallelism: {} | Stages: {}",
            self.total_tasks,
            self.max_parallelism,
            self.stages.len()
        );
        let _ = writeln!(out, "\nExecution Stages (Topological Waves):");
        for stage in &self.stages {
            let _ = writeln!(
                out,
                "  Stage {:02} (parallel): [{}]",
                stage.stage_index,
                stage.tasks.join(", ")
            );
        }
        let _ = writeln!(out, "\nCritical Path: {}", self.critical_path.join(" -> "));
        if !self.variable_references.is_empty() {
            let _ = writeln!(
                out,
                "\nReferenced Variables: [{}]",
                self.variable_references.join(", ")
            );
        }
        if !self.warnings.is_empty() {
            let _ = writeln!(out, "\nWarnings:");
            for w in &self.warnings {
                let _ = writeln!(out, "  - [WARN] {w}");
            }
        }
        out
    }
}

/// Simulates workflow execution and generates a [`DryRunReport`].
pub fn simulate_workflow(def: &WorkflowDef) -> Result<DryRunReport, DomainError> {
    // 1. Verify topological ordering and cycle freedom.
    let _ordered = topo_order(&def.tasks)?;

    let mut task_map = BTreeMap::new();
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for task in &def.tasks {
        task_map.insert(task.name.clone(), task);
        in_degree.insert(task.name.clone(), task.depends_on.len());
        dependents.entry(task.name.clone()).or_default();
        for dep in &task.depends_on {
            dependents
                .entry(dep.clone())
                .or_default()
                .push(task.name.clone());
        }
    }

    // 2. Wave decomposition into parallel execution stages.
    let mut stages = Vec::new();
    let mut remaining_in_degree = in_degree.clone();
    let mut stage_idx = 0;

    let mut ready: Vec<String> = remaining_in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(name, _)| name.clone())
        .collect();
    ready.sort();

    let mut task_stage_map = BTreeMap::new();

    while !ready.is_empty() {
        for t in &ready {
            task_stage_map.insert(t.clone(), stage_idx);
        }
        stages.push(DryRunStage {
            stage_index: stage_idx,
            tasks: ready.clone(),
        });
        stage_idx += 1;

        let mut next_ready = Vec::new();
        for task_name in ready {
            if let Some(children) = dependents.get(&task_name) {
                for child in children {
                    if let Some(deg) = remaining_in_degree.get_mut(child) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            next_ready.push(child.clone());
                        }
                    }
                }
            }
        }
        next_ready.sort();
        ready = next_ready;
    }

    let max_parallelism = stages.iter().map(|s| s.tasks.len()).max().unwrap_or(0);

    // 3. Compute Critical Path (longest path through DAG).
    let mut dist: BTreeMap<String, usize> = BTreeMap::new();
    let mut prev: BTreeMap<String, Option<String>> = BTreeMap::new();

    for task in &def.tasks {
        dist.insert(task.name.clone(), 1);
        prev.insert(task.name.clone(), None);
    }

    for stage in &stages {
        for task_name in &stage.tasks {
            let current_dist = *dist.get(task_name).unwrap_or(&1);
            if let Some(children) = dependents.get(task_name) {
                for child in children {
                    let child_dist = dist.get_mut(child).unwrap();
                    if current_dist + 1 > *child_dist {
                        *child_dist = current_dist + 1;
                        prev.insert(child.clone(), Some(task_name.clone()));
                    }
                }
            }
        }
    }

    let mut end_task = def
        .tasks
        .first()
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let mut max_dist = 0;
    for (name, &d) in &dist {
        if d > max_dist {
            max_dist = d;
            end_task = name.clone();
        }
    }

    let mut critical_path = Vec::new();
    let mut curr = Some(end_task);
    while let Some(node) = curr {
        critical_path.push(node.clone());
        curr = prev.get(&node).cloned().flatten();
    }
    critical_path.reverse();

    // 4. Template variable extraction & static reachability warnings.
    let mut variable_references = BTreeSet::new();
    let mut warnings = Vec::new();

    for task in &def.tasks {
        let extracted = extract_placeholders(&task.input);
        for var in extracted {
            variable_references.insert(var.clone());
            // If variable references another task, check that the task is an upstream dependency.
            if var.starts_with("tasks.") {
                let parts: Vec<&str> = var.split('.').collect();
                if parts.len() >= 2 {
                    let ref_task = parts[1];
                    if ref_task == task.name {
                        warnings.push(format!(
                            "task '{}' references its own output via '${{{}}}'",
                            task.name, var
                        ));
                    } else if !is_transitive_dependency(ref_task, &task.name, &task_map) {
                        warnings.push(format!(
                            "task '{}' references '${{{}}}' but '{}' is not in its dependency chain",
                            task.name, var, ref_task
                        ));
                    }
                }
            }
        }
    }

    Ok(DryRunReport {
        tenant: def.tenant.clone(),
        workflow_name: def.name.clone(),
        version: def.version,
        total_tasks: def.tasks.len(),
        max_parallelism,
        stages,
        critical_path,
        variable_references: variable_references.into_iter().collect(),
        warnings,
    })
}

fn is_transitive_dependency(
    target: &str,
    from_task: &str,
    task_map: &BTreeMap<String, &crate::domain::workflow::TaskSpec>,
) -> bool {
    let mut visited = BTreeSet::new();
    let mut queue = vec![from_task];

    while let Some(current) = queue.pop() {
        if let Some(spec) = task_map.get(current) {
            for dep in &spec.depends_on {
                if dep == target {
                    return true;
                }
                if visited.insert(dep.as_str()) {
                    queue.push(dep.as_str());
                }
            }
        }
    }
    false
}

fn extract_placeholders(val: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    match val {
        serde_json::Value::String(s) => {
            let mut cursor = 0;
            while let Some(start) = s[cursor..].find("${") {
                let abs_start = cursor + start;
                if let Some(end) = s[abs_start..].find('}') {
                    let abs_end = abs_start + end;
                    let placeholder = &s[abs_start + 2..abs_end].trim();
                    if !placeholder.is_empty() {
                        out.push(placeholder.to_string());
                    }
                    cursor = abs_end + 1;
                } else {
                    break;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                out.extend(extract_placeholders(item));
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                out.extend(extract_placeholders(v));
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{HandlerId, WorkflowId};
    use crate::domain::retry_policy::RetryPolicy;
    use crate::domain::status::Priority;
    use crate::domain::workflow::{Hooks, TaskSpec, SPEC_VERSION};
    use serde_json::json;

    fn make_def(tasks: Vec<TaskSpec>) -> WorkflowDef {
        WorkflowDef {
            id: WorkflowId::from_validated("wf_dryrun_test".into()),
            tenant: "acme".into(),
            name: "pipeline".into(),
            version: 1,
            description: "dry run test workflow".into(),
            tasks,
            timeout_ms: 60_000,
            retry: RetryPolicy::default(),
            default_priority: Priority::default(),
            hooks: Hooks::default(),
            tags: BTreeMap::new(),
            spec_version: SPEC_VERSION,
            created_at_ms: 100,
            updated_at_ms: 100,
        }
    }

    fn task_with_input(name: &str, depends: &[&str], input: serde_json::Value) -> TaskSpec {
        TaskSpec {
            name: name.to_owned(),
            handler: HandlerId::parse("runvane.echo").unwrap(),
            input,
            depends_on: depends.iter().map(|s| s.to_string()).collect(),
            timeout_ms: None,
            retry: None,
            meta: BTreeMap::new(),
        }
    }

    #[test]
    fn dry_run_linear_pipeline_stages() {
        let tasks = vec![
            task_with_input("step1", &[], json!({"src": "s3://data"})),
            task_with_input(
                "step2",
                &["step1"],
                json!({"prev": "${tasks.step1.output}"}),
            ),
            task_with_input("step3", &["step2"], json!({})),
        ];
        let def = make_def(tasks);
        let report = simulate_workflow(&def).unwrap();

        assert_eq!(report.total_tasks, 3);
        assert_eq!(report.max_parallelism, 1);
        assert_eq!(report.stages.len(), 3);
        assert_eq!(report.critical_path, vec!["step1", "step2", "step3"]);
        assert_eq!(report.variable_references, vec!["tasks.step1.output"]);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn dry_run_diamond_dag_parallelism() {
        let tasks = vec![
            task_with_input("start", &[], json!({})),
            task_with_input("branch_a", &["start"], json!({})),
            task_with_input("branch_b", &["start"], json!({})),
            task_with_input("join", &["branch_a", "branch_b"], json!({})),
        ];
        let def = make_def(tasks);
        let report = simulate_workflow(&def).unwrap();

        assert_eq!(report.total_tasks, 4);
        assert_eq!(report.max_parallelism, 2);
        assert_eq!(report.stages.len(), 3);
        assert_eq!(report.stages[1].tasks, vec!["branch_a", "branch_b"]);
    }

    #[test]
    fn dry_run_detects_unreachable_variable_dependency() {
        let tasks = vec![
            task_with_input("a", &[], json!({})),
            task_with_input("b", &[], json!({"unlinked": "${tasks.a.output.val}"})),
        ];
        let def = make_def(tasks);
        let report = simulate_workflow(&def).unwrap();

        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("not in its dependency chain"));
        assert!(report.format_text().contains("Warnings:"));
    }
}
