use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use crate::core::error::AegisResult;
use crate::core::traits::{EventBus, EventReceiver};
use crate::core::types::{Event, EventKind, EventSeverity};

static NEXT_RECEIVER_ID: AtomicU64 = AtomicU64::new(1);

fn next_receiver_id() -> u64 {
    NEXT_RECEIVER_ID.fetch_add(1, Ordering::Relaxed)
}

type SubscriberMap = DashMap<EventKind, Vec<(u64, UnboundedSender<Event>)>>;

pub struct InMemoryEventBus {
    subscribers: SubscriberMap,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self {
            subscribers: SubscriberMap::new(),
        }
    }

    pub fn publish_sync(&self, event: Event) {
        if let Some(mut subscribers) = self.subscribers.get_mut(&event.kind) {
            subscribers.retain(|(_, sender)| sender.send(event.clone()).is_ok());
        }
    }

    pub fn subscribe_sync(&self, kind: EventKind) -> ChannelEventReceiver {
        let id = next_receiver_id();
        let (tx, rx) = unbounded_channel();
        self.subscribers.entry(kind).or_default().push((id, tx));
        ChannelEventReceiver {
            kind,
            receiver_id: id,
            inner: rx,
        }
    }

    pub fn unsubscribe_sync(&self, kind: EventKind, receiver_id: u64) {
        if let Some(mut subscribers) = self.subscribers.get_mut(&kind) {
            subscribers.retain(|(id, _)| *id != receiver_id);
        }
    }
}

impl Default for InMemoryEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus for InMemoryEventBus {
    fn publish(&self, event: Event) -> crate::core::traits::BoxFuture<'_, AegisResult<()>> {
        let bus = self.subscribers.clone();
        Box::pin(async move {
            if let Some(mut subscribers) = bus.get_mut(&event.kind) {
                subscribers.retain(|(_, sender)| sender.send(event.clone()).is_ok());
            }
            Ok(())
        })
    }

    fn subscribe(
        &self,
        kind: EventKind,
    ) -> crate::core::traits::BoxFuture<'_, AegisResult<Box<dyn crate::core::traits::EventReceiver>>>
    {
        let id = next_receiver_id();
        let (tx, rx) = unbounded_channel();
        self.subscribers.entry(kind).or_default().push((id, tx));
        Box::pin(async move {
            let receiver: Box<dyn crate::core::traits::EventReceiver> =
                Box::new(ChannelEventReceiver {
                    kind,
                    receiver_id: id,
                    inner: rx,
                });
            Ok(receiver)
        })
    }

    fn unsubscribe(
        &self,
        kind: EventKind,
        receiver_id: Uuid,
    ) -> crate::core::traits::BoxFuture<'_, AegisResult<()>> {
        let rid = receiver_id.as_u128() as u64;
        Box::pin(async move {
            if let Some(mut subscribers) = self.subscribers.get_mut(&kind) {
                subscribers.retain(|(id, _)| *id != rid);
            }
            Ok(())
        })
    }
}

pub struct ChannelEventReceiver {
    kind: EventKind,
    receiver_id: u64,
    inner: UnboundedReceiver<Event>,
}

impl ChannelEventReceiver {
    pub fn kind(&self) -> EventKind {
        self.kind
    }

    pub fn receiver_id(&self) -> u64 {
        self.receiver_id
    }

    pub async fn recv(&mut self) -> Option<Event> {
        self.inner.recv().await
    }

    pub fn try_recv(&mut self) -> Option<Event> {
        self.inner.try_recv().ok()
    }
}

impl EventReceiver for ChannelEventReceiver {
    fn recv(&mut self) -> crate::core::traits::BoxFuture<'_, Option<Event>> {
        Box::pin(async move { self.inner.recv().await })
    }

    fn try_recv(&mut self) -> Option<Event> {
        self.inner.try_recv().ok()
    }
}

pub struct EventBuilder {
    kind: EventKind,
    source: String,
    payload: Vec<u8>,
    severity: EventSeverity,
}

impl EventBuilder {
    pub fn new(kind: EventKind, source: &str) -> Self {
        Self {
            kind,
            source: source.to_string(),
            payload: Vec::new(),
            severity: EventSeverity::Info,
        }
    }

    pub fn with_payload(mut self, data: Vec<u8>) -> Self {
        self.payload = data;
        self
    }

    pub fn with_severity(mut self, severity: EventSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn build(self) -> Event {
        Event {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            kind: self.kind,
            source: self.source,
            payload: self.payload,
            severity: self.severity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_builder_default_severity() {
        let event = EventBuilder::new(EventKind::ArchiveCreated, "test").build();
        assert_eq!(event.kind, EventKind::ArchiveCreated);
        assert_eq!(event.source, "test");
        assert_eq!(event.severity, EventSeverity::Info);
        assert!(event.payload.is_empty());
    }

    #[test]
    fn test_event_builder_full() {
        let payload = b"hello".to_vec();
        let event = EventBuilder::new(EventKind::ChunkStored, "worker")
            .with_payload(payload.clone())
            .with_severity(EventSeverity::Debug)
            .build();

        assert_eq!(event.kind, EventKind::ChunkStored);
        assert_eq!(event.source, "worker");
        assert_eq!(event.severity, EventSeverity::Debug);
        assert_eq!(event.payload, payload);
    }

    #[test]
    fn test_event_builder_unique_ids() {
        let e1 = EventBuilder::new(EventKind::Error, "src1").build();
        let e2 = EventBuilder::new(EventKind::Error, "src2").build();
        assert_ne!(e1.id, e2.id);
    }

    #[test]
    fn test_event_builder_timestamp() {
        let event = EventBuilder::new(EventKind::ConfigChanged, "system").build();
        let now = Utc::now();
        let diff = now - event.timestamp;
        assert!(diff.num_seconds() < 2);
    }

    #[test]
    fn test_event_builder_severity_chain() {
        let event = EventBuilder::new(EventKind::ChunkDeleted, "gc")
            .with_severity(EventSeverity::Warning)
            .with_payload(vec![1, 2, 3])
            .build();

        assert_eq!(event.severity, EventSeverity::Warning);
        assert_eq!(event.payload, vec![1, 2, 3]);
    }

    #[test]
    fn test_publish_subscribe_sync() {
        let bus = InMemoryEventBus::new();
        let mut receiver = bus.subscribe_sync(EventKind::ChunkStored);

        let event = EventBuilder::new(EventKind::ChunkStored, "test")
            .with_payload(b"data".to_vec())
            .build();

        let event_clone = event.clone();
        bus.publish_sync(event);

        let received = receiver.try_recv().unwrap();
        assert_eq!(received.kind, EventKind::ChunkStored);
        assert_eq!(received.source, "test");
        assert_eq!(received.payload, b"data".to_vec());
        assert_eq!(received.id, event_clone.id);
    }

    #[test]
    fn test_multiple_receivers() {
        let bus = InMemoryEventBus::new();

        let mut rx1 = bus.subscribe_sync(EventKind::ChunkStored);
        let mut rx2 = bus.subscribe_sync(EventKind::ChunkStored);

        let event = EventBuilder::new(EventKind::ChunkStored, "multi").build();
        bus.publish_sync(event);

        let received1 = rx1.try_recv().unwrap();
        let received2 = rx2.try_recv().unwrap();
        assert_eq!(received1.id, received2.id);
    }

    #[test]
    fn test_unsubscribe() {
        let bus = InMemoryEventBus::new();

        let receiver = bus.subscribe_sync(EventKind::ChunkStored);
        let receiver_id = receiver.receiver_id();
        drop(receiver);

        bus.unsubscribe_sync(EventKind::ChunkStored, receiver_id);

        let event = EventBuilder::new(EventKind::ChunkStored, "unsub").build();
        bus.publish_sync(event);

        assert!(bus
            .subscribers
            .get(&EventKind::ChunkStored)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_different_kinds_isolated() {
        let bus = InMemoryEventBus::new();

        let mut rx_stored = bus.subscribe_sync(EventKind::ChunkStored);
        let mut rx_deleted = bus.subscribe_sync(EventKind::ChunkDeleted);

        let stored_event = EventBuilder::new(EventKind::ChunkStored, "s").build();
        let deleted_event = EventBuilder::new(EventKind::ChunkDeleted, "d").build();

        bus.publish_sync(stored_event);
        bus.publish_sync(deleted_event);

        assert!(rx_stored.try_recv().is_some());
        assert!(rx_deleted.try_recv().is_some());
    }

    #[test]
    fn test_publish_no_subscribers() {
        let bus = InMemoryEventBus::new();
        let event = EventBuilder::new(EventKind::SyncCompleted, "test").build();
        bus.publish_sync(event);
    }

    #[test]
    fn test_stale_receiver_removed() {
        let bus = InMemoryEventBus::new();

        let receiver = bus.subscribe_sync(EventKind::ChunkStored);
        let receiver_id = receiver.receiver_id();
        drop(receiver);

        let event = EventBuilder::new(EventKind::ChunkStored, "stale").build();

        {
            let mut subscribers = bus.subscribers.get_mut(&EventKind::ChunkStored).unwrap();
            subscribers.retain(|(id, sender)| {
                if *id == receiver_id {
                    sender.send(event.clone()).is_ok()
                } else {
                    true
                }
            });
        }

        let event2 = EventBuilder::new(EventKind::ChunkStored, "after").build();
        bus.publish_sync(event2);

        let subscribers = bus.subscribers.get(&EventKind::ChunkStored).unwrap();
        assert!(subscribers.iter().all(|(_, sender)| !sender.is_closed()));
    }

    #[tokio::test]
    async fn test_async_publish_subscribe() {
        let bus = InMemoryEventBus::new();

        let mut receiver = {
            let fut = bus.subscribe(EventKind::ArchiveCreated);
            let result: Box<dyn crate::core::traits::EventReceiver> = fut.await.unwrap();
            result
        };

        let event = EventBuilder::new(EventKind::ArchiveCreated, "async_test")
            .with_payload(b"async".to_vec())
            .build();

        bus.publish(event.clone()).await.unwrap();

        let received = receiver.recv().await.unwrap();
        assert_eq!(received.kind, EventKind::ArchiveCreated);
        assert_eq!(received.payload, b"async".to_vec());
    }

    #[tokio::test]
    async fn test_async_unsubscribe() {
        let bus = InMemoryEventBus::new();

        let receiver = bus.subscribe_sync(EventKind::ChunkStored);
        let receiver_id = Uuid::from_u128(receiver.receiver_id() as u128);
        drop(receiver);

        bus.unsubscribe_sync(EventKind::ChunkStored, receiver_id.as_u128() as u64);

        let subscribers = bus.subscribers.get(&EventKind::ChunkStored);
        assert!(subscribers.is_none() || subscribers.unwrap().is_empty());
    }

    #[test]
    fn test_channel_receiver_kind() {
        let bus = InMemoryEventBus::new();
        let receiver = bus.subscribe_sync(EventKind::ChunkStored);
        assert_eq!(receiver.kind(), EventKind::ChunkStored);
    }

    #[test]
    fn test_channel_receiver_unique_ids() {
        let bus = InMemoryEventBus::new();
        let r1 = bus.subscribe_sync(EventKind::ChunkStored);
        let r2 = bus.subscribe_sync(EventKind::ChunkStored);
        assert_ne!(r1.receiver_id(), r2.receiver_id());
    }

    #[test]
    fn test_try_recv_empty() {
        let bus = InMemoryEventBus::new();
        let mut receiver = bus.subscribe_sync(EventKind::ChunkStored);
        assert!(receiver.try_recv().is_none());
    }
}
