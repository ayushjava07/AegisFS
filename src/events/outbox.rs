//! Durable webhook retry outbox and dead-letter queue (DLQ).
//!
//! While the default [`super::dispatch::WebhookSink`] is best-effort for low-latency
//! loopback notifications, the outbox provides at-least-once delivery semantics with
//! exponential backoff and eventual dead-letter isolation for failing receivers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::clock::Clock;
use crate::events::dispatch::{Delivery, WebhookSink};

/// Status of an outbox delivery item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    /// Scheduled for initial or retry delivery.
    Pending,
    /// Successfully acknowledged by the receiver.
    Delivered,
    /// Exhausted all retry attempts and parked in the dead-letter queue.
    DeadLetter,
}

/// A queued webhook delivery tracking retry state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxItem {
    /// Stable delivery id (sha256 of run + event + tenant).
    pub id: String,
    /// The outbound HTTP delivery specification.
    pub delivery: Delivery,
    /// Number of delivery attempts made so far.
    pub attempts: u32,
    /// Maximum permitted attempts before dead-lettering.
    pub max_attempts: u32,
    /// Earliest timestamp (epoch ms) for next attempt.
    pub next_attempt_at_ms: i64,
    /// Details of the most recent delivery failure.
    pub last_error: Option<String>,
    /// Current delivery lifecycle status.
    pub status: OutboxStatus,
}

/// Statistics covering the outbox buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboxStats {
    /// Number of items awaiting delivery.
    pub pending: usize,
    /// Successfully completed deliveries.
    pub delivered: usize,
    /// Permanently failed items parked in DLQ.
    pub dead_letter: usize,
}

/// Thread-safe in-memory outbox buffer for webhook deliveries.
#[derive(Debug, Clone, Default)]
pub struct WebhookOutbox {
    items: Arc<Mutex<HashMap<String, OutboxItem>>>,
    delivered_count: Arc<AtomicUsize>,
    dead_letter_count: Arc<AtomicUsize>,
}

impl WebhookOutbox {
    /// Creates an empty outbox.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueues a delivery. Idempotent: if `delivery.id` is already pending or delivered,
    /// the existing item is preserved.
    pub fn enqueue(&self, delivery: Delivery, max_attempts: u32, now_ms: i64) {
        let mut guard = self.items.lock().unwrap();
        if guard.contains_key(&delivery.id) {
            return;
        }
        guard.insert(
            delivery.id.clone(),
            OutboxItem {
                id: delivery.id.clone(),
                delivery,
                attempts: 0,
                max_attempts: max_attempts.max(1),
                next_attempt_at_ms: now_ms,
                last_error: None,
                status: OutboxStatus::Pending,
            },
        );
    }

    /// Returns all items that are ready for attempt at `now_ms`.
    pub fn ready_items(&self, now_ms: i64) -> Vec<OutboxItem> {
        let guard = self.items.lock().unwrap();
        let mut ready: Vec<OutboxItem> = guard
            .values()
            .filter(|item| {
                item.status == OutboxStatus::Pending && item.next_attempt_at_ms <= now_ms
            })
            .cloned()
            .collect();
        ready.sort_by_key(|i| (i.next_attempt_at_ms, i.attempts));
        ready
    }

    /// Marks an item successfully delivered.
    pub fn mark_success(&self, id: &str) -> bool {
        let mut guard = self.items.lock().unwrap();
        if let Some(item) = guard.get_mut(id) {
            if item.status == OutboxStatus::Pending {
                item.status = OutboxStatus::Delivered;
                self.delivered_count.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Records a failed attempt. If attempts reach `max_attempts`, parks the item in DeadLetter.
    /// Otherwise, calculates the next retry timestamp.
    pub fn mark_failure(
        &self,
        id: &str,
        error: String,
        now_ms: i64,
        backoff_ms: i64,
    ) -> OutboxStatus {
        let mut guard = self.items.lock().unwrap();
        if let Some(item) = guard.get_mut(id) {
            item.attempts += 1;
            item.last_error = Some(error);
            if item.attempts >= item.max_attempts {
                item.status = OutboxStatus::DeadLetter;
                self.dead_letter_count.fetch_add(1, Ordering::Relaxed);
            } else {
                item.next_attempt_at_ms = now_ms + backoff_ms;
            }
            return item.status;
        }
        OutboxStatus::Pending
    }

    /// Returns items currently in the dead-letter queue.
    pub fn dead_letter_items(&self) -> Vec<OutboxItem> {
        let guard = self.items.lock().unwrap();
        guard
            .values()
            .filter(|item| item.status == OutboxStatus::DeadLetter)
            .cloned()
            .collect()
    }

    /// Retrieves current aggregate stats.
    pub fn stats(&self) -> OutboxStats {
        let guard = self.items.lock().unwrap();
        let pending = guard
            .values()
            .filter(|item| item.status == OutboxStatus::Pending)
            .count();
        OutboxStats {
            pending,
            delivered: self.delivered_count.load(Ordering::Relaxed),
            dead_letter: self.dead_letter_count.load(Ordering::Relaxed),
        }
    }
}

/// Driver loop executing pending outbox deliveries through a [`WebhookSink`].
pub struct OutboxProcessor<'a> {
    outbox: &'a WebhookOutbox,
    sink: &'a dyn WebhookSink,
    clock: &'a dyn Clock,
    base_backoff_ms: i64,
}

impl<'a> OutboxProcessor<'a> {
    /// Creates a new outbox processor.
    pub fn new(
        outbox: &'a WebhookOutbox,
        sink: &'a dyn WebhookSink,
        clock: &'a dyn Clock,
        base_backoff_ms: i64,
    ) -> Self {
        Self {
            outbox,
            sink,
            clock,
            base_backoff_ms: base_backoff_ms.max(10),
        }
    }

    /// Executes one drain step over currently ready deliveries. Returns count processed.
    pub fn step(&self) -> usize {
        let now = self.clock.now_ms();
        let ready = self.outbox.ready_items(now);
        let count = ready.len();

        for item in ready {
            match self.sink.deliver(&item.delivery) {
                Ok(()) => {
                    self.outbox.mark_success(&item.id);
                }
                Err(err) => {
                    let backoff = self
                        .base_backoff_ms
                        .saturating_mul(1 << item.attempts.min(10));
                    self.outbox.mark_failure(&item.id, err, now, backoff);
                }
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::events::dispatch::RecordingSink;

    fn sample_delivery(id: &str) -> Delivery {
        Delivery {
            id: id.to_owned(),
            url: "http://127.0.0.1:8080/hook".to_owned(),
            headers: std::collections::BTreeMap::new(),
            body_json: r#"{"status":"succeeded"}"#.to_owned(),
        }
    }

    #[test]
    fn outbox_enqueue_and_success_flow() {
        let outbox = WebhookOutbox::new();
        let sink = RecordingSink::default();
        let clock = ManualClock::at(1_000);

        outbox.enqueue(sample_delivery("dlv_1"), 3, clock.now_ms());
        assert_eq!(outbox.ready_items(clock.now_ms()).len(), 1);

        let processor = OutboxProcessor::new(&outbox, &sink, &clock, 100);
        let processed = processor.step();
        assert_eq!(processed, 1);
        assert_eq!(sink.count(), 1);

        let stats = outbox.stats();
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.delivered, 1);
        assert_eq!(stats.dead_letter, 0);
    }

    #[test]
    fn outbox_retries_with_backoff_and_moves_to_dlq() {
        let outbox = WebhookOutbox::new();
        struct FlakySink {
            fails_remaining: AtomicUsize,
        }
        impl WebhookSink for FlakySink {
            fn deliver(&self, _delivery: &Delivery) -> Result<(), String> {
                let prev = self.fails_remaining.fetch_sub(1, Ordering::SeqCst);
                if prev > 0 {
                    Err("network timeout".into())
                } else {
                    Ok(())
                }
            }
        }

        let sink = FlakySink {
            fails_remaining: AtomicUsize::new(2),
        };
        let clock = ManualClock::at(10_000);
        let processor = OutboxProcessor::new(&outbox, &sink, &clock, 500);

        // Enqueue with max 2 attempts.
        outbox.enqueue(sample_delivery("dlv_flaky"), 2, clock.now_ms());

        // Attempt 1: fails, scheduled for backoff (+500ms -> 10_500ms).
        assert_eq!(processor.step(), 1);
        assert_eq!(outbox.ready_items(10_100).len(), 0);

        // Advance to 10_500: Attempt 2 fails, reaches max attempts (2) -> DeadLetter.
        clock.advance(500);
        assert_eq!(processor.step(), 1);

        let dlq = outbox.dead_letter_items();
        assert_eq!(dlq.len(), 1);
        assert_eq!(dlq[0].id, "dlv_flaky");
        assert_eq!(dlq[0].status, OutboxStatus::DeadLetter);
        assert_eq!(outbox.stats().dead_letter, 1);
    }
}
