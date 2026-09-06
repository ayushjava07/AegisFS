//! Workflow versioning, alias resolution, canary traffic shifting, and compatibility analysis.
//!
//! In production platforms, workflow definitions undergo continuous updates.
//! This module provides primitives for semantic version tracking, aliases (`latest`, `prod`, `canary`),
//! weighted traffic splitting, and static backward-compatibility validation between workflow versions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::workflow::WorkflowDef;

/// An alias pointing to a target workflow version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowAlias {
    /// Name of the alias (e.g. `prod`, `staging`, `canary`, `v1`).
    pub name: String,
    /// Owning tenant.
    pub tenant: String,
    /// Workflow name.
    pub workflow_name: String,
    /// Target version assigned to this alias.
    pub target_version: u32,
    /// Epoch timestamp when the alias was created or updated.
    pub updated_at_ms: i64,
}

impl WorkflowAlias {
    /// Creates a new alias record.
    pub fn new(
        name: impl Into<String>,
        tenant: impl Into<String>,
        workflow_name: impl Into<String>,
        target_version: u32,
        updated_at_ms: i64,
    ) -> Self {
        Self {
            name: name.into(),
            tenant: tenant.into(),
            workflow_name: workflow_name.into(),
            target_version,
            updated_at_ms,
        }
    }
}

/// A canary deployment rule that probabilistically or deterministically routes
/// runs between a baseline version and a canary version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanaryRoutingPolicy {
    /// Baseline version (e.g. current stable production version).
    pub baseline_version: u32,
    /// Candidate canary version being tested.
    pub canary_version: u32,
    /// Percentage of traffic directed to the canary version (0 to 100).
    pub canary_weight_pct: u8,
}

impl CanaryRoutingPolicy {
    /// Creates a new canary policy with a clamped percentage (0..=100).
    pub fn new(baseline_version: u32, canary_version: u32, canary_weight_pct: u8) -> Self {
        Self {
            baseline_version,
            canary_version,
            canary_weight_pct: canary_weight_pct.min(100),
        }
    }

    /// Selects either the baseline or canary version given a uniform pseudo-random number in `0..100`.
    pub fn select_version_by_sample(&self, sample_0_to_99: u8) -> u32 {
        if sample_0_to_99 < self.canary_weight_pct {
            self.canary_version
        } else {
            self.baseline_version
        }
    }

    /// Selects version deterministically based on a discriminator string (e.g. tenant or run key hash).
    pub fn select_version_by_key(&self, key: &str) -> u32 {
        if self.canary_weight_pct == 0 {
            return self.baseline_version;
        }
        if self.canary_weight_pct >= 100 {
            return self.canary_version;
        }
        // Simple FNV-1a hash mod 100 for deterministic bucketing
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in key.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let bucket = (hash % 100) as u8;
        self.select_version_by_sample(bucket)
    }
}

/// Category of compatibility issue discovered between two workflow definition versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompatibilitySeverity {
    /// An informational divergence that does not prevent migration.
    Info,
    /// A potential concern (e.g. reduced timeout or changed retry policy).
    Warning,
    /// A breaking incompatibility (e.g. deleted task, added required dependency).
    Breaking,
}

/// A specific compatibility issue identified during definition comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityIssue {
    /// Severity level of this issue.
    pub severity: CompatibilitySeverity,
    /// Name of the affected task, if localized to a specific task.
    pub task_name: Option<String>,
    /// Human-readable explanation of the issue.
    pub message: String,
}

impl CompatibilityIssue {
    /// Creates a breaking change issue.
    pub fn breaking(task_name: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: CompatibilitySeverity::Breaking,
            task_name: task_name.map(str::to_owned),
            message: message.into(),
        }
    }

    /// Creates a warning issue.
    pub fn warning(task_name: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity: CompatibilitySeverity::Warning,
            task_name: task_name.map(str::to_owned),
            message: message.into(),
        }
    }
}

/// The outcome of evaluating backward compatibility between an old definition and a new definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// True if the new definition introduces no breaking changes relative to the old definition.
    pub is_backward_compatible: bool,
    /// Detailed list of all detected issues.
    pub issues: Vec<CompatibilityIssue>,
}

impl CompatibilityReport {
    /// Evaluates compatibility between `old_def` and `new_def`.
    pub fn analyze(old_def: &WorkflowDef, new_def: &WorkflowDef) -> Self {
        let mut issues = Vec::new();

        let old_tasks: BTreeMap<&str, &super::workflow::TaskSpec> =
            old_def.tasks.iter().map(|t| (t.name.as_str(), t)).collect();
        let new_tasks: BTreeMap<&str, &super::workflow::TaskSpec> =
            new_def.tasks.iter().map(|t| (t.name.as_str(), t)).collect();

        // Check for removed tasks (breaking for in-flight steps that might resume)
        for &old_name in old_tasks.keys() {
            if !new_tasks.contains_key(old_name) {
                issues.push(CompatibilityIssue::breaking(
                    Some(old_name),
                    format!(
                        "Task '{old_name}' was removed in version {}",
                        new_def.version
                    ),
                ));
            }
        }

        // Check for changed task handlers or modified dependencies
        for (&new_name, &new_spec) in &new_tasks {
            if let Some(&old_spec) = old_tasks.get(new_name) {
                if old_spec.handler != new_spec.handler {
                    issues.push(CompatibilityIssue::breaking(
                        Some(new_name),
                        format!(
                            "Task '{new_name}' changed handler from '{}' to '{}'",
                            old_spec.handler, new_spec.handler
                        ),
                    ));
                }

                let old_deps: BTreeSet<&str> =
                    old_spec.depends_on.iter().map(String::as_str).collect();
                let new_deps: BTreeSet<&str> =
                    new_spec.depends_on.iter().map(String::as_str).collect();

                let added_deps: Vec<&&str> = new_deps.difference(&old_deps).collect();
                if !added_deps.is_empty() {
                    issues.push(CompatibilityIssue::warning(
                        Some(new_name),
                        format!("Task '{new_name}' added new dependencies: {:?}", added_deps),
                    ));
                }
            }
        }

        // Check for reduced timeout
        if new_def.timeout_ms < old_def.timeout_ms {
            issues.push(CompatibilityIssue::warning(
                None,
                format!(
                    "Overall workflow timeout decreased from {}ms to {}ms",
                    old_def.timeout_ms, new_def.timeout_ms
                ),
            ));
        }

        let is_backward_compatible = !issues
            .iter()
            .any(|i| i.severity == CompatibilitySeverity::Breaking);

        Self {
            is_backward_compatible,
            issues,
        }
    }
}

impl fmt::Display for CompatibilityReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_backward_compatible {
            write!(
                f,
                "COMPATIBLE: {} non-breaking observations",
                self.issues.len()
            )
        } else {
            let breaking_count = self
                .issues
                .iter()
                .filter(|i| i.severity == CompatibilitySeverity::Breaking)
                .count();
            write!(f, "INCOMPATIBLE: {breaking_count} breaking change(s)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{HandlerId, WorkflowId};
    use crate::domain::retry_policy::RetryPolicy;
    use crate::domain::status::Priority;
    use crate::domain::workflow::{Hooks, TaskSpec, WorkflowDef, SPEC_VERSION};

    fn make_def(version: u32, tasks: Vec<(&str, &str, Vec<&str>)>, timeout: u64) -> WorkflowDef {
        let task_specs = tasks
            .into_iter()
            .map(|(name, handler, deps)| TaskSpec {
                name: name.to_owned(),
                handler: HandlerId::parse(handler).unwrap(),
                input: serde_json::json!({}),
                depends_on: deps.into_iter().map(str::to_owned).collect(),
                timeout_ms: None,
                retry: None,
                meta: BTreeMap::new(),
            })
            .collect();

        WorkflowDef {
            id: WorkflowId::parse("wf_000000000000000000000001").unwrap(),
            tenant: "acme".to_owned(),
            name: "pipeline".to_owned(),
            version,
            description: "test pipeline".to_owned(),
            tasks: task_specs,
            timeout_ms: timeout,
            retry: RetryPolicy::default(),
            default_priority: Priority::Normal,
            hooks: Hooks::default(),
            tags: BTreeMap::new(),
            spec_version: SPEC_VERSION,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    #[test]
    fn alias_constructs_and_serializes() {
        let alias = WorkflowAlias::new("prod", "acme", "pipeline", 3, 1000);
        assert_eq!(alias.target_version, 3);
        assert_eq!(alias.name, "prod");
        let json = serde_json::to_string(&alias).unwrap();
        let back: WorkflowAlias = serde_json::from_str(&json).unwrap();
        assert_eq!(back, alias);
    }

    #[test]
    fn canary_policy_selects_by_weight_and_key() {
        let policy = CanaryRoutingPolicy::new(1, 2, 25);
        assert_eq!(policy.select_version_by_sample(10), 2);
        assert_eq!(policy.select_version_by_sample(24), 2);
        assert_eq!(policy.select_version_by_sample(25), 1);
        assert_eq!(policy.select_version_by_sample(99), 1);

        // Deterministic by key
        let v_a = policy.select_version_by_key("tenant_123");
        let v_b = policy.select_version_by_key("tenant_123");
        assert_eq!(v_a, v_b);

        // Edge weights
        let all_baseline = CanaryRoutingPolicy::new(1, 2, 0);
        assert_eq!(all_baseline.select_version_by_key("any"), 1);
        let all_canary = CanaryRoutingPolicy::new(1, 2, 100);
        assert_eq!(all_canary.select_version_by_key("any"), 2);
    }

    #[test]
    fn compatibility_analyzer_detects_clean_evolution() {
        let v1 = make_def(1, vec![("step1", "runvane.echo", vec![])], 60_000);
        let v2 = make_def(
            2,
            vec![
                ("step1", "runvane.echo", vec![]),
                ("step2", "runvane.echo", vec!["step1"]),
            ],
            60_000,
        );

        let report = CompatibilityReport::analyze(&v1, &v2);
        assert!(report.is_backward_compatible);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn compatibility_analyzer_detects_breaking_changes() {
        let v1 = make_def(
            1,
            vec![
                ("step1", "runvane.echo", vec![]),
                ("step2", "runvane.echo", vec!["step1"]),
            ],
            60_000,
        );
        // v2 removes step2 and changes step1 handler
        let v2 = make_def(2, vec![("step1", "runvane.noop", vec![])], 30_000);

        let report = CompatibilityReport::analyze(&v1, &v2);
        assert!(!report.is_backward_compatible);
        assert_eq!(report.issues.len(), 3); // 2 breaking + 1 timeout warning
        assert!(report
            .issues
            .iter()
            .any(|i| i.severity == CompatibilitySeverity::Breaking
                && i.task_name.as_deref() == Some("step2")));
        assert!(report
            .issues
            .iter()
            .any(|i| i.severity == CompatibilitySeverity::Breaking
                && i.task_name.as_deref() == Some("step1")));
    }
}
