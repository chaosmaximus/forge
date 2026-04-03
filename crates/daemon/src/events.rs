// events.rs — Event bus for real-time streaming to subscribers
//
// Uses tokio::broadcast for fan-out to multiple subscribers (Mac app, CLI, etc.).
// Events are best-effort: if a subscriber is slow, it skips (Lagged).

use serde::{Serialize, Deserialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeEvent {
    pub event: String,     // "extraction" | "consolidation" | "guardrail" | "agent"
    pub data: serde_json::Value,
    pub timestamp: String,
}

pub type EventSender = broadcast::Sender<ForgeEvent>;

pub fn create_event_bus() -> EventSender {
    let (tx, _) = broadcast::channel(256);
    tx
}

/// Helper to emit an event (best-effort, never blocks).
pub fn emit(tx: &EventSender, event: &str, data: serde_json::Value) {
    let _ = tx.send(ForgeEvent {
        event: event.to_string(),
        data,
        timestamp: timestamp_now(),
    });
}

fn timestamp_now() -> String {
    forge_core::time::timestamp_now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_bus() {
        let tx = create_event_bus();
        let mut rx = tx.subscribe();

        emit(&tx, "extraction", serde_json::json!({"title": "test"}));

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "extraction");
        assert_eq!(event.data["title"], "test");
        assert!(!event.timestamp.is_empty());
    }

    #[test]
    fn test_event_bus_multiple() {
        let tx = create_event_bus();
        let mut rx = tx.subscribe();

        emit(&tx, "extraction", serde_json::json!({}));
        emit(&tx, "consolidation", serde_json::json!({}));

        // Both received (no filter at bus level — filtering is in socket handler)
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.event, "extraction");
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.event, "consolidation");
    }

    #[test]
    fn test_event_bus_no_subscriber_no_panic() {
        let tx = create_event_bus();
        // Emit with no subscribers — should not panic
        emit(&tx, "test", serde_json::json!({"ok": true}));
    }

    #[test]
    fn test_event_serde_roundtrip() {
        let event = ForgeEvent {
            event: "extraction".to_string(),
            data: serde_json::json!({"memory_id": "abc123"}),
            timestamp: "1712000000".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: ForgeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.event, "extraction");
        assert_eq!(restored.data["memory_id"], "abc123");
    }
}
