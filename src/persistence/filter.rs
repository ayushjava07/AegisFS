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
    /// Match definition name by prefix.
    pub name_prefix: Option<String>,
    /// Match the run status exactly.
    pub status: Option<RunStatus>,
    /// Match any of the given run statuses.
    pub status_in: Vec<RunStatus>,
    /// Match the tenant exactly.
    pub tenant: Option<String>,
    /// Tag equality filters (every entry must match).
    pub tags: BTreeMap<String, String>,
    /// Tag key presence filters (run must contain each named key).
    pub has_tag_keys: Vec<String>,
    /// Only a run submitted at or after this timestamp.
    pub from_ms: Option<i64>,
    /// Only a run submitted before this timestamp.
    pub to_ms: Option<i64>,
    /// Only a run completed at or after this timestamp.
    pub finished_from_ms: Option<i64>,
    /// Only a run completed at or before this timestamp.
    pub finished_to_ms: Option<i64>,
    /// Minimum run duration (finished_at - started_at) in milliseconds.
    pub min_duration_ms: Option<i64>,
    /// Maximum rows to return.
    pub limit: Option<usize>,
    /// Number of rows to skip (pagination offset).
    pub offset: usize,
}

impl RunFilter {
    /// Creates a new empty filter matching everything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an exact workflow definition name requirement.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets a workflow definition prefix requirement.
    pub fn with_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.name_prefix = Some(prefix.into());
        self
    }

    /// Sets an exact run status filter.
    pub fn with_status(mut self, status: RunStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Admits any run whose status is within `statuses`.
    pub fn with_status_in(mut self, statuses: impl IntoIterator<Item = RunStatus>) -> Self {
        self.status_in = statuses.into_iter().collect();
        self
    }

    /// Sets an exact tenant requirement.
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Adds a tag key-value requirement.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Adds a tag key presence requirement.
    pub fn with_has_tag_key(mut self, key: impl Into<String>) -> Self {
        self.has_tag_keys.push(key.into());
        self
    }

    /// Sets the created timestamp window [from_ms, to_ms).
    pub fn with_time_range(mut self, from_ms: Option<i64>, to_ms: Option<i64>) -> Self {
        self.from_ms = from_ms;
        self.to_ms = to_ms;
        self
    }

    /// Sets the finished timestamp window [from_ms, to_ms].
    pub fn with_finished_range(mut self, from_ms: Option<i64>, to_ms: Option<i64>) -> Self {
        self.finished_from_ms = from_ms;
        self.finished_to_ms = to_ms;
        self
    }

    /// Sets minimum execution duration requirement.
    pub fn with_min_duration(mut self, duration_ms: i64) -> Self {
        self.min_duration_ms = Some(duration_ms);
        self
    }

    /// Sets maximum result limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets pagination offset.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    /// Whether `run` satisfies every set predicate.
    pub fn matches(&self, run: &crate::domain::run::Run) -> bool {
        if let Some(name) = &self.name {
            if run.def_name != *name {
                return false;
            }
        }
        if let Some(prefix) = &self.name_prefix {
            if !run.def_name.starts_with(prefix) {
                return false;
            }
        }
        if let Some(status) = &self.status {
            if run.status != *status {
                return false;
            }
        }
        if !self.status_in.is_empty() && !self.status_in.contains(&run.status) {
            return false;
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
        for key in &self.has_tag_keys {
            if !run.tags.contains_key(key) {
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
        if let Some(f_from) = self.finished_from_ms {
            match run.finished_at_ms {
                Some(finished) if finished >= f_from => {}
                _ => return false,
            }
        }
        if let Some(f_to) = self.finished_to_ms {
            match run.finished_at_ms {
                Some(finished) if finished <= f_to => {}
                _ => return false,
            }
        }
        if let Some(min_dur) = self.min_duration_ms {
            match (run.started_at_ms, run.finished_at_ms) {
                (Some(s), Some(f)) if f.saturating_sub(s) >= min_dur => {}
                _ => return false,
            }
        }
        true
    }

    /// Applies pagination and newest-first ordering by `created_at_ms` descending.
    pub fn apply_order(&self, mut runs: Vec<&Run>) -> Vec<Run> {
        runs.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
        let iter = runs.into_iter().skip(self.offset);
        let mut out = Vec::new();
        for run in iter {
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
        let f = RunFilter {
            limit: Some(2),
            ..Default::default()
        };
        let got = f.apply_order(vec![&r1, &r2, &r3]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].created_at_ms, 3);
        assert_eq!(got[1].created_at_ms, 2);
    }

    #[test]
    fn prefix_and_status_in_filtering() {
        let r = run(100, "acme", RunStatus::Failed, "prod");
        assert!(RunFilter::new().with_name_prefix("pipe").matches(&r));
        assert!(!RunFilter::new().with_name_prefix("batch").matches(&r));

        let f = RunFilter::new().with_status_in([RunStatus::Failed, RunStatus::TimedOut]);
        assert!(f.matches(&r));

        let f_miss = RunFilter::new().with_status_in([RunStatus::Queued, RunStatus::Succeeded]);
        assert!(!f_miss.matches(&r));
    }

    #[test]
    fn tag_keys_presence_filtering() {
        let mut r = run(100, "acme", RunStatus::Succeeded, "prod");
        r.tags.insert("region".into(), "us-east-1".into());

        assert!(RunFilter::new().with_has_tag_key("env").matches(&r));
        assert!(RunFilter::new().with_has_tag_key("region").matches(&r));
        assert!(!RunFilter::new().with_has_tag_key("missing_key").matches(&r));
    }

    #[test]
    fn finished_range_and_duration_filtering() {
        let mut r = run(100, "acme", RunStatus::Succeeded, "prod");
        r.started_at_ms = Some(150);
        r.finished_at_ms = Some(450); // duration = 300ms

        assert!(RunFilter::new()
            .with_finished_range(Some(400), Some(500))
            .matches(&r));
        assert!(!RunFilter::new()
            .with_finished_range(Some(500), None)
            .matches(&r));
        assert!(RunFilter::new().with_min_duration(250).matches(&r));
        assert!(!RunFilter::new().with_min_duration(350).matches(&r));
    }

    #[test]
    fn pagination_offset_skips_entries() {
        let r1 = run(1, "acme", RunStatus::Queued, "prod");
        let r2 = run(2, "acme", RunStatus::Queued, "prod");
        let r3 = run(3, "acme", RunStatus::Queued, "prod");
        let r4 = run(4, "acme", RunStatus::Queued, "prod");

        let f = RunFilter::new().with_offset(1).with_limit(2);
        let got = f.apply_order(vec![&r1, &r2, &r3, &r4]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].created_at_ms, 3);
        assert_eq!(got[1].created_at_ms, 2);
    }
}
