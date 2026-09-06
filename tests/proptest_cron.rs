//! Property-based tests for cron expression parsing and schedule tick calculations.

use chrono::{TimeZone, Utc};
use proptest::prelude::*;
use runvane::scheduler::cron::{CronError, CronSchedule};

proptest! {
    #[test]
    fn cron_next_tick_is_strictly_future_and_aligned_to_minute(
        minute_step in 1u32..30,
        epoch_sec in 1_600_000_000i64..1_800_000_000i64,
    ) {
        let expr = format!("*/{} * * * *", minute_step);
        let schedule = CronSchedule::parse(&expr).expect("valid cron expression");

        let after_ms = epoch_sec * 1000;
        let next_ms = schedule.next_tick_ms(after_ms).expect("must find a tick within 5 years");

        // Next tick must be strictly in the future
        prop_assert!(next_ms > after_ms, "next_ms {} must be > after_ms {}", next_ms, after_ms);

        // Next tick must be aligned to the start of a UTC minute (00 seconds, 000 millis)
        prop_assert_eq!(next_ms % 60_000, 0, "next_ms {} not aligned to minute boundary", next_ms);

        // Next tick must satisfy matches_utc
        let dt = Utc.timestamp_millis_opt(next_ms).single().unwrap();
        prop_assert!(schedule.matches_utc(&dt), "schedule must match at the calculated tick");
    }

    #[test]
    fn cron_ticks_are_strictly_monotonic(
        minute_step in 5u32..25,
        epoch_sec in 1_700_000_000i64..1_750_000_000i64,
    ) {
        let expr = format!("*/{} * * * *", minute_step);
        let schedule = CronSchedule::parse(&expr).expect("valid cron expression");

        let mut current_ms = epoch_sec * 1000;
        for _ in 0..5 {
            let next_ms = schedule.next_tick_ms(current_ms).expect("tick exists");
            prop_assert!(next_ms > current_ms, "tick {} must be > {}", next_ms, current_ms);
            current_ms = next_ms;
        }
    }

    #[test]
    fn wildcard_cron_always_fires_at_next_minute(
        epoch_sec in 1_700_000_000i64..1_750_000_000i64,
        extra_ms in 0i64..59_999,
    ) {
        let schedule = CronSchedule::parse("* * * * *").expect("standard wildcard");
        let now_ms = epoch_sec * 1000 + extra_ms;
        let expected_next_ms = (now_ms / 60_000 + 1) * 60_000;

        let next_ms = schedule.next_tick_ms(now_ms).expect("next tick");
        prop_assert_eq!(next_ms, expected_next_ms);
    }

    #[test]
    fn invalid_field_counts_are_rejected(fields in proptest::collection::vec("[a-z0-9*]+", 0..10)) {
        prop_assume!(fields.len() != 5);
        let expr = fields.join(" ");
        let err = CronSchedule::parse(&expr).unwrap_err();
        prop_assert!(matches!(err, CronError::InvalidFieldCount(count) if count == fields.len()));
    }

    #[test]
    fn minute_out_of_range_is_rejected(min in 60u32..200) {
        let expr = format!("{} * * * *", min);
        let err = CronSchedule::parse(&expr).unwrap_err();
        let is_range_err = matches!(err, CronError::ValueOutOfRange { field: "minute", .. });
        prop_assert!(is_range_err);
    }

    #[test]
    fn hour_out_of_range_is_rejected(hour in 24u32..100) {
        let expr = format!("* {} * * *", hour);
        let err = CronSchedule::parse(&expr).unwrap_err();
        let is_range_err = matches!(err, CronError::ValueOutOfRange { field: "hour", .. });
        prop_assert!(is_range_err);
    }
}
