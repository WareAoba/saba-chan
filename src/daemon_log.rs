//! Daemon Log Buffer — Captures tracing output into a shared ring buffer
//!
//! Provides:
//! - `DaemonLogBuffer`: Thread-safe ring buffer of daemon log entries
//! - `DaemonLogLayer`: tracing Layer that pipes events into the buffer

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Maximum number of log entries retained in the ring buffer.
const DEFAULT_DAEMON_LOG_CAPACITY: usize = 10_000;

// ─── Types ───────────────────────────────────────────────────

/// A single daemon log entry.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonLogEntry {
    /// Sequential ID for incremental polling (`?since=<id>`)
    pub id: u64,
    /// Unix timestamp in milliseconds
    pub timestamp: u64,
    /// Log level (info, warn, error, debug, trace)
    pub level: String,
    /// tracing target (module path)
    pub target: String,
    /// Formatted message
    pub message: String,
}

/// Shared daemon log buffer accessible from multiple threads.
#[derive(Clone)]
pub struct DaemonLogBuffer {
    inner: Arc<Mutex<VecDeque<DaemonLogEntry>>>,
    next_id: Arc<AtomicU64>,
    capacity: usize,
}

impl DaemonLogBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_DAEMON_LOG_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity.min(1024)))),
            next_id: Arc::new(AtomicU64::new(1)),
            capacity,
        }
    }

    /// Push a new log entry into the buffer, evicting the oldest if at capacity.
    pub fn push(&self, level: String, target: String, message: String) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let entry = DaemonLogEntry {
            id,
            timestamp,
            level,
            target,
            message,
        };

        let mut buf = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(entry);
    }

    /// Retrieve entries with `id >= since`. Returns at most `count` entries.
    pub fn get_since(&self, since: Option<u64>, count: Option<usize>) -> Vec<DaemonLogEntry> {
        let buf = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let count = count.unwrap_or(200);

        match since {
            Some(since_id) => buf
                .iter()
                .filter(|e| e.id >= since_id)
                .take(count)
                .cloned()
                .collect(),
            None => buf
                .iter()
                .rev()
                .take(count)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .cloned()
                .collect(),
        }
    }

    /// Number of entries currently in the buffer.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
}

impl Default for DaemonLogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tracing Layer ───────────────────────────────────────────

/// tracing Layer that captures formatted events into a `DaemonLogBuffer`.
pub struct DaemonLogLayer {
    buffer: DaemonLogBuffer,
}

impl DaemonLogLayer {
    pub fn new(buffer: DaemonLogBuffer) -> Self {
        Self { buffer }
    }
}

/// Visitor that collects the `message` field from tracing events.
struct MessageVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Vec::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else {
            self.fields.push((field.name().to_string(), format!("{:?}", value)));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push((field.name().to_string(), value.to_string()));
        }
    }
}

impl<S: Subscriber> Layer<S> for DaemonLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level().to_string().to_lowercase();
        let target = metadata.target().to_string();

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        // Build final message: "message field1=val1 field2=val2"
        let mut msg = visitor.message;
        for (k, v) in &visitor.fields {
            msg.push(' ');
            msg.push_str(k);
            msg.push('=');
            msg.push_str(v);
        }

        self.buffer.push(level, target, msg);
    }
}

// ─── Daemon-Only Terminal Loop ───────────────────────────────
//
// 데몬 온리 모드에서는 데몬 자체의 tracing 로그가 이미 stderr로 출력되므로
// 별도의 터미널 폴링 루프가 필요 없다. 인스턴스(관리형 프로세스/컨테이너)의
// 콘솔 로그를 보고 싶다면 GUI/CLI를 띄워 IPC API(`/api/instance/:id/console`)를
// 통해 조회한다.
//
// 과거에는 여기서 `daemon_terminal_loop`을 띄워 모든 인스턴스의 콘솔 로그를
// stderr로 끌어올렸으나, 다음 이유로 제거했다:
//   1. 종료된 인스턴스의 디스크 콘솔 로그가 디스크 폴백 경로에서 since_id를
//      무시하기 때문에(`Supervisor::get_console_output` 참고), 무한 재출력 루프가
//      발생했다.
//   2. 데몬 온리 모드는 헤드리스 운영용이며, 인스턴스 콘솔이 필요한 사용자는
//      UI를 기동시킨다.

// ─── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_push_and_get() {
        let buf = DaemonLogBuffer::new();
        buf.push("info".into(), "test::module".into(), "hello world".into());
        buf.push("warn".into(), "test::module".into(), "something wrong".into());

        let entries = buf.get_since(None, None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, 1);
        assert_eq!(entries[0].level, "info");
        assert_eq!(entries[0].message, "hello world");
        assert_eq!(entries[1].id, 2);
        assert_eq!(entries[1].level, "warn");
    }

    #[test]
    fn test_buffer_since_filter() {
        let buf = DaemonLogBuffer::new();
        for i in 0..10 {
            buf.push("info".into(), "test".into(), format!("line {}", i));
        }

        let entries = buf.get_since(Some(5), None);
        assert_eq!(entries.len(), 6); // ids 5,6,7,8,9,10
        assert_eq!(entries[0].id, 5);
    }

    #[test]
    fn test_buffer_capacity_eviction() {
        let buf = DaemonLogBuffer::with_capacity(5);
        for i in 0..10 {
            buf.push("info".into(), "test".into(), format!("line {}", i));
        }

        assert_eq!(buf.len(), 5);
        let entries = buf.get_since(None, None);
        // IDs 6-10 remain (oldest evicted)
        assert_eq!(entries[0].id, 6);
        assert_eq!(entries[4].id, 10);
    }

    #[test]
    fn test_buffer_count_limit() {
        let buf = DaemonLogBuffer::new();
        for i in 0..100 {
            buf.push("info".into(), "test".into(), format!("line {}", i));
        }

        let entries = buf.get_since(Some(1), Some(3));
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_buffer_empty() {
        let buf = DaemonLogBuffer::new();
        let entries = buf.get_since(None, None);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_buffer_none_since_returns_latest() {
        let buf = DaemonLogBuffer::new();
        for i in 0..500 {
            buf.push("info".into(), "test".into(), format!("line {}", i));
        }

        // No `since` → returns last `count` entries
        let entries = buf.get_since(None, Some(10));
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[9].id, 500); // latest
        assert_eq!(entries[0].id, 491);
    }

    #[test]
    fn test_entry_serialization() {
        let buf = DaemonLogBuffer::new();
        buf.push("error".into(), "saba_chan::ipc".into(), "test error".into());

        let entries = buf.get_since(None, None);
        let json = serde_json::to_value(&entries[0]).unwrap();
        assert_eq!(json["level"], "error");
        assert_eq!(json["target"], "saba_chan::ipc");
        assert_eq!(json["message"], "test error");
        assert!(json["id"].as_u64().is_some());
        assert!(json["timestamp"].as_u64().is_some());
    }

    #[test]
    fn test_layer_captures_event() {
        use tracing_subscriber::prelude::*;

        let buf = DaemonLogBuffer::new();
        let layer = DaemonLogLayer::new(buf.clone());

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(target: "test_target", "hello from tracing");
        tracing::warn!(target: "test_target", "a warning");

        let entries = buf.get_since(None, None);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, "info");
        assert!(entries[0].message.contains("hello from tracing"));
        assert_eq!(entries[1].level, "warn");
    }

    #[test]
    fn test_layer_captures_fields() {
        use tracing_subscriber::prelude::*;

        let buf = DaemonLogBuffer::new();
        let layer = DaemonLogLayer::new(buf.clone());

        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(port = 8080, "server started");

        let entries = buf.get_since(None, None);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].message.contains("server started"));
        assert!(entries[0].message.contains("port=8080"));
    }
}
