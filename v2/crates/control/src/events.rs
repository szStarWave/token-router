//! SSE 事件总线（P4-081，技术方案 §4.9）。
//!
//! 契约：
//! - `GET /v2/admin/events`（`text/event-stream`）；`Last-Event-ID` 断线续传；
//! - 保留最近 1024 条事件，超出后客户端应重新拉取（`retry-after-overflow`）；
//! - 心跳 15s（`event: ping`）；
//! - 事件不含 prompt/响应正文，只含 ID 与元数据。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

/// ring 容量。
pub const RING_CAPACITY: usize = 1024;

/// 事件类型。
#[derive(Debug, Clone)]
pub struct Event {
    pub id: i64,
    pub kind: EventKind,
    /// JSON payload（不含正文）。
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    RequestCompleted,
    ConfigApplied,
    BreakerOpened,
    ApprovalRequired,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestCompleted => "request.completed",
            Self::ConfigApplied => "config.applied",
            Self::BreakerOpened => "breaker.opened",
            Self::ApprovalRequired => "approval.required",
        }
    }
}

/// 全局事件总线（控制面与 pipeline 共享）。
#[derive(Default)]
pub struct EventBus {
    ring: Mutex<VecDeque<Event>>,
    next_id: AtomicI64,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发布事件（seq 单调递增）。
    pub fn publish(&self, kind: EventKind, data: serde_json::Value) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let mut ring = self
            .ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(Event { id, kind, data });
    }

    /// `Last-Event-ID` 之后的全部事件（最多一页；超出提示重拉）。
    pub fn after(&self, last_event_id: i64) -> (Vec<Event>, bool) {
        let ring = self
            .ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let events: Vec<Event> = ring
            .iter()
            .filter(|event| event.id > last_event_id)
            .cloned()
            .collect();
        // 若客户端落后超过 ring 容量（ring 里最老的事件都比 last_event_id 新），
        // 返回 overflow 标记。
        let overflow = ring
            .front()
            .map(|oldest| oldest.id > last_event_id + 1)
            .unwrap_or(false);
        (events, overflow)
    }

    /// 最新事件 id（无事件返回 0）。
    pub fn latest_id(&self) -> i64 {
        self.next_id.load(Ordering::SeqCst)
    }
}

impl Event {
    /// 渲染为 SSE 帧（`id:` + `event:` + `data:`）。
    pub fn to_sse(&self) -> String {
        format!(
            "id: {}\nevent: {}\ndata: {}\n\n",
            self.id,
            self.kind.as_str(),
            self.data
        )
    }
}

/// 渲染心跳帧。
pub fn ping_frame() -> &'static str {
    "event: ping\ndata: {}\n\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_and_replay_after() {
        let bus = EventBus::new();
        bus.publish(
            EventKind::RequestCompleted,
            serde_json::json!({"outcome": "success"}),
        );
        bus.publish(
            EventKind::ConfigApplied,
            serde_json::json!({"revision": "abc"}),
        );
        let (events, overflow) = bus.after(0);
        assert_eq!(events.len(), 2);
        assert!(!overflow);
        assert_eq!(events[0].kind.as_str(), "request.completed");
        assert_eq!(events[1].id, events[0].id + 1);
    }

    #[test]
    fn last_event_id_skips_delivered() {
        let bus = EventBus::new();
        bus.publish(EventKind::RequestCompleted, serde_json::json!({}));
        let first = bus.after(0).0[0].id;
        bus.publish(EventKind::BreakerOpened, serde_json::json!({}));
        let (events, _) = bus.after(first);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind.as_str(), "breaker.opened");
    }

    #[test]
    fn ring_overflow_flags_resync() {
        let bus = EventBus::new();
        for index in 0..(RING_CAPACITY as i64 + 10) {
            bus.publish(EventKind::RequestCompleted, serde_json::json!({"i": index}));
        }
        // 客户端停在 id=0：ring 已滚动，最新 1024 条全部 > 0 → overflow。
        let (events, overflow) = bus.after(0);
        assert_eq!(events.len(), RING_CAPACITY);
        assert!(overflow, "client is behind the ring; should resync");
        // 跟上的客户端没有 overflow。
        let latest = bus.latest_id();
        let (_, overflow2) = bus.after(latest - 1);
        assert!(!overflow2);
    }

    #[test]
    fn sse_frame_shape() {
        let bus = EventBus::new();
        bus.publish(
            EventKind::ConfigApplied,
            serde_json::json!({"revision": "6f2c"}),
        );
        let (events, _) = bus.after(0);
        let frame = events[0].to_sse();
        assert!(frame.starts_with("id: 1\nevent: config.applied\ndata: {"));
        assert!(frame.ends_with("\n\n"));
        assert_eq!(ping_frame(), "event: ping\ndata: {}\n\n");
    }
}
