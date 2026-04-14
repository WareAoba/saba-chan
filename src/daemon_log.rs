//! Daemon Log Buffer — Captures tracing output into a shared ring buffer
//!
//! Provides:
//! - `DaemonLogBuffer`: Thread-safe ring buffer of daemon log entries
//! - `DaemonLogLayer`: tracing Layer that pipes events into the buffer
//! - `daemon_terminal_loop`: daemon-only mode console output loop

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

/// Runs a polling loop that prints process console logs to stderr.
/// Intended for daemon-only mode where no GUI/CLI is attached.
///
/// 데몬 자체 tracing 로그는 이미 stderr로 출력되므로 여기서는
/// 관리형 프로세스 콘솔 로그만 표시합니다.
/// 데몬 로그 버퍼는 커서 동기화용으로만 보관합니다 (API용).
pub async fn daemon_terminal_loop(
    _daemon_buf: DaemonLogBuffer,
    supervisor: Arc<tokio::sync::RwLock<crate::supervisor::Supervisor>>,
    shutdown: tokio_util::sync::CancellationToken,
) {
    use std::io::Write;

    // instance_id → last seen log id
    let mut process_cursors: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                eprintln!("\n[DaemonOnly] Terminal loop shutting down");
                break;
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(500)) => {}
        }

        // ── Collect process console logs (async 구간) ──
        // stderr lock은 Send가 아니므로 await 경계를 넘기지 않는다.
        let mut output_lines: Vec<String> = Vec::new();

        let sup = supervisor.read().await;
        let instances = sup.instance_store.list();
        for inst in instances {
            let cursor = process_cursors.get(&inst.id).copied();
            if let Ok(result) = sup.get_console_output(&inst.id, cursor, Some(200)).await {
                if let Some(lines) = result.get("lines").and_then(|v| v.as_array()) {
                    for line in lines {
                        let id = line.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let content = line.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        let level = line.get("level").and_then(|v| v.as_str()).unwrap_or("info");
                        let level_tag = match level {
                            "error" => "\x1b[31mERROR\x1b[0m",
                            "warn" => "\x1b[33m WARN\x1b[0m",
                            _ => " INFO",
                        };
                        output_lines.push(format!(
                            " {} \x1b[35m[{}]\x1b[0m {}",
                            level_tag,
                            inst.name,
                            content
                        ));
                        process_cursors.insert(inst.id.clone(), id + 1);
                    }
                }
            }
        }
        drop(sup); // RwLock 해제

        // ── Write to stderr (sync 구간) ──
        if !output_lines.is_empty() {
            let stderr = std::io::stderr();
            let mut out = stderr.lock();
            for line in &output_lines {
                let _ = writeln!(out, "{}", line);
            }
        }
    }
}

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
