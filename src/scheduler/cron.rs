//! Cron expression parser and periodic workflow trigger engine.
//!
//! Provides parsing and next-tick scheduling for standard 5-field cron specifications
//! (`minute hour dom month dow`), supporting wildcards (`*`), step intervals (`*/5`),
//! inclusive ranges (`1-5`), and lists (`1,15,30`).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// Errors encountered when parsing or evaluating cron schedules.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CronError {
    /// Expression did not contain exactly 5 space-delimited fields.
    #[error("invalid field count: expected 5, found {0}")]
    InvalidFieldCount(usize),

    /// A field value was outside its legal integer bounds.
    #[error("field '{field}' value {value} out of range ({min}..={max})")]
    ValueOutOfRange {
        /// Name of the field.
        field: &'static str,
        /// Provided value.
        value: i64,
        /// Minimum allowable value.
        min: i64,
        /// Maximum allowable value.
        max: i64,
    },

    /// A field expression could not be parsed.
    #[error("malformed cron field '{field}': {reason}")]
    MalformedField {
        /// Name of the field.
        field: &'static str,
        /// Detailed syntax failure reason.
        reason: String,
    },
}

/// A parsed set of allowed integer values for a single cron field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronField {
    allowed: Vec<u32>,
}

impl CronField {
    /// Returns whether the candidate value matches this field.
    pub fn contains(&self, val: u32) -> bool {
        self.allowed.binary_search(&val).is_ok()
    }

    /// Parses a single field expression (e.g. `*`, `*/15`, `1-5`, `0,30`).
    pub fn parse(
        expr: &str,
        field_name: &'static str,
        min: u32,
        max: u32,
    ) -> Result<Self, CronError> {
        let mut allowed = Vec::new();

        for part in expr.split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err(CronError::MalformedField {
                    field: field_name,
                    reason: "empty sub-expression in list".into(),
                });
            }

            if part == "*" {
                for v in min..=max {
                    allowed.push(v);
                }
            } else if let Some(step_str) = part.strip_prefix("*/") {
                let step = step_str
                    .parse::<u32>()
                    .map_err(|e| CronError::MalformedField {
                        field: field_name,
                        reason: format!("invalid step: {e}"),
                    })?;
                if step == 0 {
                    return Err(CronError::MalformedField {
                        field: field_name,
                        reason: "step must be greater than 0".into(),
                    });
                }
                let mut v = min;
                while v <= max {
                    allowed.push(v);
                    v = match v.checked_add(step) {
                        Some(next) => next,
                        None => break,
                    };
                }
            } else if part.contains('-') {
                let mut bounds = part.split('-');
                let start_str = bounds.next().unwrap();
                let end_str = bounds.next().ok_or_else(|| CronError::MalformedField {
                    field: field_name,
                    reason: "missing range upper bound".into(),
                })?;
                let start = start_str
                    .parse::<u32>()
                    .map_err(|e| CronError::MalformedField {
                        field: field_name,
                        reason: format!("invalid range start: {e}"),
                    })?;
                let end = end_str
                    .parse::<u32>()
                    .map_err(|e| CronError::MalformedField {
                        field: field_name,
                        reason: format!("invalid range end: {e}"),
                    })?;

                if start < min || start > max {
                    return Err(CronError::ValueOutOfRange {
                        field: field_name,
                        value: i64::from(start),
                        min: i64::from(min),
                        max: i64::from(max),
                    });
                }
                if end < min || end > max {
                    return Err(CronError::ValueOutOfRange {
                        field: field_name,
                        value: i64::from(end),
                        min: i64::from(min),
                        max: i64::from(max),
                    });
                }
                if start > end {
                    return Err(CronError::MalformedField {
                        field: field_name,
                        reason: format!("range start {start} exceeds end {end}"),
                    });
                }

                for v in start..=end {
                    allowed.push(v);
                }
            } else {
                let val = part.parse::<u32>().map_err(|e| CronError::MalformedField {
                    field: field_name,
                    reason: format!("invalid integer: {e}"),
                })?;
                if val < min || val > max {
                    return Err(CronError::ValueOutOfRange {
                        field: field_name,
                        value: i64::from(val),
                        min: i64::from(min),
                        max: i64::from(max),
                    });
                }
                allowed.push(val);
            }
        }

        allowed.sort_unstable();
        allowed.dedup();

        Ok(Self { allowed })
    }
}

/// A parsed 5-field cron schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    raw: String,
    minutes: CronField,
    hours: CronField,
    days_of_month: CronField,
    months: CronField,
    days_of_week: CronField,
}

impl fmt::Display for CronSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl FromStr for CronSchedule {
    type Err = CronError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl CronSchedule {
    /// Parses a 5-field cron string (`"min hour dom month dow"`).
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronError::InvalidFieldCount(fields.len()));
        }

        let minutes = CronField::parse(fields[0], "minute", 0, 59)?;
        let hours = CronField::parse(fields[1], "hour", 0, 23)?;
        let days_of_month = CronField::parse(fields[2], "day_of_month", 1, 31)?;
        let months = CronField::parse(fields[3], "month", 1, 12)?;
        let days_of_week = CronField::parse(fields[4], "day_of_week", 0, 6)?;

        Ok(Self {
            raw: expr.trim().to_owned(),
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
        })
    }

    /// Returns whether the given UTC date/time satisfies all 5 schedule fields.
    pub fn matches_utc(&self, dt: &DateTime<Utc>) -> bool {
        let minute = dt.minute();
        let hour = dt.hour();
        let day = dt.day();
        let month = dt.month();
        // chrono: Mon=0..Sun=6 via num_days_from_sunday
        let dow = dt.weekday().num_days_from_sunday();

        self.minutes.contains(minute)
            && self.hours.contains(hour)
            && self.days_of_month.contains(day)
            && self.months.contains(month)
            && self.days_of_week.contains(dow)
    }

    /// Computes the next tick timestamp in epoch milliseconds strictly after `after_epoch_ms`.
    ///
    /// Scans up to 5 years (2,628,000 minutes) into the future before giving up.
    pub fn next_tick_ms(&self, after_epoch_ms: i64) -> Option<i64> {
        let mut dt = Utc.timestamp_millis_opt(after_epoch_ms).single()?;

        // Advance to the start of the next whole minute
        let remainder_sec = dt.second();
        let remainder_nano = dt.nanosecond();
        dt = dt + Duration::minutes(1)
            - Duration::seconds(i64::from(remainder_sec))
            - Duration::nanoseconds(i64::from(remainder_nano));

        // Scan minute by minute
        for _ in 0..2_628_000 {
            if self.matches_utc(&dt) {
                return Some(dt.timestamp_millis());
            }
            dt += Duration::minutes(1);
        }

        None
    }
}

/// A registered periodic trigger configured to launch workflow runs on a schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTrigger {
    /// Unique trigger identifier.
    pub id: String,
    /// Owning tenant.
    pub tenant: String,
    /// Target workflow name.
    pub workflow_name: String,
    /// Raw cron schedule string.
    pub cron_expr: String,
    /// Default input payload supplied to submitted runs.
    pub input: serde_json::Value,
    /// Whether the schedule is actively running.
    pub enabled: bool,
    /// Timestamp (epoch ms) of the most recent triggered execution.
    pub last_triggered_at_ms: Option<i64>,
    /// Timestamp (epoch ms) when the trigger next fires.
    pub next_trigger_at_ms: Option<i64>,
}

impl ScheduledTrigger {
    /// Creates a new active trigger, computing its initial `next_trigger_at_ms`.
    pub fn new(
        id: impl Into<String>,
        tenant: impl Into<String>,
        workflow_name: impl Into<String>,
        cron_expr: impl Into<String>,
        input: serde_json::Value,
        now_ms: i64,
    ) -> Result<Self, CronError> {
        let cron_str = cron_expr.into();
        let schedule = CronSchedule::parse(&cron_str)?;
        let next_trigger = schedule.next_tick_ms(now_ms);

        Ok(Self {
            id: id.into(),
            tenant: tenant.into(),
            workflow_name: workflow_name.into(),
            cron_expr: cron_str,
            input,
            enabled: true,
            last_triggered_at_ms: None,
            next_trigger_at_ms: next_trigger,
        })
    }

    /// Advances the trigger state after firing, computing the next tick.
    pub fn advance_after_fire(&mut self, fired_at_ms: i64) -> Result<Option<i64>, CronError> {
        let schedule = CronSchedule::parse(&self.cron_expr)?;
        let next = schedule.next_tick_ms(fired_at_ms);
        self.last_triggered_at_ms = Some(fired_at_ms);
        self.next_trigger_at_ms = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_cron_expressions() {
        assert!(CronSchedule::parse("* * * * *").is_ok());
        assert!(CronSchedule::parse("*/5 * * * *").is_ok());
        assert!(CronSchedule::parse("0 12 * * 1-5").is_ok());
        assert!(CronSchedule::parse("30 8 1,15 * *").is_ok());
    }

    #[test]
    fn reject_invalid_cron_expressions() {
        // Too few fields
        assert!(matches!(
            CronSchedule::parse("* * * *").unwrap_err(),
            CronError::InvalidFieldCount(4)
        ));

        // Value out of range
        assert!(matches!(
            CronSchedule::parse("60 * * * *").unwrap_err(),
            CronError::ValueOutOfRange {
                field: "minute",
                ..
            }
        ));

        // Malformed range
        assert!(matches!(
            CronSchedule::parse("10-5 * * * *").unwrap_err(),
            CronError::MalformedField { .. }
        ));
    }

    #[test]
    fn schedule_calculates_next_tick() {
        // Every 15 minutes: */15 * * * *
        let sched = CronSchedule::parse("*/15 * * * *").unwrap();

        // 2026-09-06 12:05:30 UTC
        let base_dt = Utc.with_ymd_and_hms(2026, 9, 6, 12, 5, 30).unwrap();
        let next_ms = sched.next_tick_ms(base_dt.timestamp_millis()).unwrap();
        let next_dt = Utc.timestamp_millis_opt(next_ms).unwrap();

        assert_eq!(next_dt.minute(), 15);
        assert_eq!(next_dt.hour(), 12);
        assert_eq!(next_dt.second(), 0);
    }

    #[test]
    fn scheduled_trigger_lifecycle() {
        let now = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let mut trigger = ScheduledTrigger::new(
            "trig_1",
            "acme",
            "backup",
            "0 2 * * *", // 2am every day
            serde_json::json!({ "mode": "full" }),
            now,
        )
        .unwrap();

        assert!(trigger.enabled);
        let next = trigger.next_trigger_at_ms.unwrap();
        let next_dt = Utc.timestamp_millis_opt(next).unwrap();
        assert_eq!(next_dt.hour(), 2);
        assert_eq!(next_dt.minute(), 0);

        // Simulate firing
        let fired_at = next;
        let next2 = trigger.advance_after_fire(fired_at).unwrap().unwrap();
        assert_eq!(trigger.last_triggered_at_ms, Some(fired_at));
        let next2_dt = Utc.timestamp_millis_opt(next2).unwrap();
        assert_eq!(next2_dt.day(), next_dt.day() + 1);
        assert_eq!(next2_dt.hour(), 2);
    }
}
