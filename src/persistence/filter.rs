//! Store query predicates.

use std::collections::BTreeMap;

use crate::domain::run::Run;
use crate::domain::status::RunStatus;

/// Retrieved-run filters. All fields are ANDed; `None`/empty means "match
/// everything" for that dimension.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunFilter {
    /// Match the workflow definition name exactly.
    pub name: Option<String>,
    /// Match the run status exactly.
    pub status: Option<RunStatus>,
    /// Match the tenant exactly.
    pub tenant: Option<String>,
    /// Tag equality filters (every entry must match).
    pub tags: BTreeMap<String, String>,
    /// Only a run submitted at or after this timestamp.
    pub from_ms: Option<i64>,
    /// Only a run submitted before this timestamp.
    pub to_ms: Option<i64>,
    /// Maximum rows to return.
    pub limit: Option<usize>,
}

impl RunFilter {
    /// Whether `run` satisfies every set predicate.
    pub fn matches(&self, run: &crate::domain::run::Run) -> bool {
        if let Some(name) = &self.name {
            if run.def_name != *name {
                return false;
            }
        }
        if let Some(status) = &self.status {
            if run.status != *status {
                return false;
            }
        }
        if let Some(tenant) = &self.tenant {
            if run.tenant != *tenant {
                return false;
            }
        }
        for (key, value) in &self.tags {
            if run.tags.get(key) != Some(value) {
                return false;
            }
        }
        if let Some(from) = self.from_ms {
            if run.created_at_ms < from {
                return false;
            }
        }
        if let Some(to) = self.to_ms {
            if run.created_at_ms >= to {
                return false;
            }
        }
        true
    }

    /// Applies `limit` to a matched slice, newest-first ordering by
    /// `created_at_ms` descending.
    pub fn apply_order(&self, mut runs: Vec<&Run>) -> Vec<Run> {
        runs.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
        let mut out = Vec::new();
        for run in runs {
            out.push(run.clone());
            if let Some(limit) = self.limit {
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::RunId;
    use crate::domain::run::Run;
    use serde_json::json;

    fn run(created: i64, tenant: &str, status: RunStatus, tag_env: &str) -> Run {
        Run {
            id: RunId::from_validated("rn_x".into()),
            tenant: tenant.into(),
            def_name: "pipeline".into(),
            def_version: 1,
            input: json!({}),
            status,
            attempts: 0,
            next_attempt_at_ms: None,
            deadline_at_ms: None,
            started_at_ms: None,
            finished_at_ms: None,
            error: None,
            output: None,
            tags: BTreeMap::from([("env".to_owned(), tag_env.to_owned())]),
            created_at_ms: created,
            run_number: 1,
        }
    }

    #[test]
    fn empty_filter_matches_everything() {
        let f = RunFilter::default();
        assert!(f.matches(&run(0, "acme", RunStatus::Queued, "prod")));
    }

    #[test]
    fn dimensions_are_anded() {
        let f = RunFilter {
            tenant: Some("acme".into()),
            status: Some(RunStatus::Failed),
            tags: BTreeMap::from([("env".into(), "prod".into())]),
            name: Some("pipeline".into()),
            from_ms: Some(10),
            to_ms: Some(100),
            ..Default::default()
        };
        assert!(f.matches(&run(50, "acme", RunStatus::Failed, "prod")));
        assert!(!f.matches(&run(5, "acme", RunStatus::Failed, "prod"))); // from_ms
        assert!(!f.matches(&run(50, "acme", RunStatus::Failed, "staging"))); // tag
        assert!(!f.matches(&run(50, "beta", RunStatus::Failed, "prod"))); // tenant
        assert!(!f.matches(&run(50, "acme", RunStatus::Queued, "prod"))); // status
    }

    #[test]
    fn limit_applied_after_newest_first() {
        let r1 = run(1, "acme", RunStatus::Queued, "prod");
        let r2 = run(2, "acme", RunStatus::Queued, "prod");
        let r3 = run(3, "acme", RunStatus::Queued, "prod");
        let f = RunFilter { limit: Some(2), ..Default::default() };
        let got = f.apply_order(vec![&r1, &r2, &r3]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].created_at_ms, 3);
        assert_eq!(got[1].created_at_ms, 2);
    }
}