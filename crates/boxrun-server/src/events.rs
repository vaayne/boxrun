use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

/// An event published through the EventBus.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: i64,
    pub event_type: String, // "log" | "exit"
    pub data: String,
    pub stream: Option<String>, // "stdout" | "stderr"
}

/// In-memory pub/sub for SSE streaming of exec events.
#[derive(Clone)]
pub struct EventBus {
    #[allow(clippy::type_complexity)]
    subscribers: Arc<Mutex<HashMap<String, Vec<mpsc::Sender<Option<Event>>>>>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Publish an event to all subscribers of the given exec_id.
    pub async fn publish(&self, exec_id: &str, event: Event) {
        let subs = self.subscribers.lock().await;
        if let Some(senders) = subs.get(exec_id) {
            for sender in senders {
                let _ = sender.send(Some(event.clone())).await;
            }
        }
    }

    /// Signal all subscribers that streaming is done for the given exec_id.
    pub async fn finish(&self, exec_id: &str) {
        let mut subs = self.subscribers.lock().await;
        if let Some(senders) = subs.remove(exec_id) {
            for sender in &senders {
                let _ = sender.send(None).await;
            }
        }
    }

    /// Subscribe to events for the given exec_id. Returns a receiver.
    pub async fn subscribe(&self, exec_id: &str) -> Subscription {
        let (tx, rx) = mpsc::channel(256);
        let mut subs = self.subscribers.lock().await;
        subs.entry(exec_id.to_string()).or_default().push(tx);
        Subscription {
            bus: self.clone(),
            exec_id: exec_id.to_string(),
            rx,
        }
    }
}

/// A subscription to events for a specific exec_id.
pub struct Subscription {
    bus: EventBus,
    exec_id: String,
    rx: mpsc::Receiver<Option<Event>>,
}

impl Subscription {
    /// Receive the next event. Returns None when the stream is finished.
    pub async fn recv(&mut self) -> Option<Event> {
        match self.rx.recv().await {
            Some(Some(event)) => Some(event),
            _ => None, // None or channel closed
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // Best-effort cleanup: try to remove sender from subscriber list.
        // We can't do async in Drop, so we use try_lock.
        if let Ok(mut subs) = self.bus.subscribers.try_lock() {
            if let Some(senders) = subs.get_mut(&self.exec_id) {
                // Remove closed senders (this sender's tx is dropped, so it will be closed)
                senders.retain(|s| !s.is_closed());
                if senders.is_empty() {
                    subs.remove(&self.exec_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_subscribe() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe("exec_1").await;

        let event = Event {
            seq: 0,
            event_type: "log".into(),
            data: "hello".into(),
            stream: Some("stdout".into()),
        };
        bus.publish("exec_1", event).await;

        let received = sub.recv().await.unwrap();
        assert_eq!(received.seq, 0);
        assert_eq!(received.data, "hello");
        assert_eq!(received.stream, Some("stdout".into()));
    }

    #[tokio::test]
    async fn test_finish_stream() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe("exec_2").await;

        bus.publish(
            "exec_2",
            Event {
                seq: 0,
                event_type: "log".into(),
                data: "data".into(),
                stream: Some("stdout".into()),
            },
        )
        .await;
        bus.finish("exec_2").await;

        let event = sub.recv().await.unwrap();
        assert_eq!(event.data, "data");

        let end = sub.recv().await;
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = EventBus::new();
        let mut sub1 = bus.subscribe("exec_3").await;
        let mut sub2 = bus.subscribe("exec_3").await;

        bus.publish(
            "exec_3",
            Event {
                seq: 0,
                event_type: "log".into(),
                data: "shared".into(),
                stream: None,
            },
        )
        .await;
        bus.finish("exec_3").await;

        let e1 = sub1.recv().await.unwrap();
        let e2 = sub2.recv().await.unwrap();
        assert_eq!(e1.data, "shared");
        assert_eq!(e2.data, "shared");
    }

    #[tokio::test]
    async fn test_no_subscribers() {
        let bus = EventBus::new();
        // Should not panic
        bus.publish(
            "nonexistent",
            Event {
                seq: 0,
                event_type: "log".into(),
                data: "dropped".into(),
                stream: None,
            },
        )
        .await;
        bus.finish("nonexistent").await;
    }

    #[tokio::test]
    async fn test_subscription_cleanup() {
        let bus = EventBus::new();
        {
            let _sub = bus.subscribe("exec_4").await;
            // sub is dropped here
        }
        // Should not panic
        bus.publish(
            "exec_4",
            Event {
                seq: 0,
                event_type: "log".into(),
                data: "after_drop".into(),
                stream: None,
            },
        )
        .await;
    }
}
