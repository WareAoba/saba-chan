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
    /// 버퍼 세대. 번호는 데몬을 켤 때마다 1부터라, 클라이언트는 세대가 바뀌면 이어 읽던 번호를 버린다 (L55)
    epoch: u64,
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
            epoch: crate::supervisor::managed_process::next_epoch(),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Push a new log entry into the buffer, evicting the oldest if at capacity.
    pub fn push(&self, level: String, target: String, message: String) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // 번호는 잠금 안에서 매긴다: 버퍼 순서와 번호 순서가 같아야 since 폴링이 줄을 건너뛰지 않는다 (D-S14)
        let mut buf = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = DaemonLogEntry {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            timestamp,
            level,
            target,
            message,
        };
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

    /// `keep`을 만족하는 최근 항목을 최대 `count`개, 오래된 것부터
    pub fn recent_where(&self, count: usize, keep: impl Fn(&DaemonLogEntry) -> bool) -> Vec<DaemonLogEntry> {
        let buf = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut found: Vec<DaemonLogEntry> = buf.iter().rev().filter(|e| keep(e)).take(count).cloned().collect();
        found.reverse();
        found
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
                let lines = live_console_lines(&result);
                if lines.is_empty() && result.get("source").is_some() {
                    // 꺼졌다: 다시 켜면 줄 번호가 처음부터라 커서를 버린다
                    process_cursors.remove(&inst.id);
                }
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

/// 콘솔 조회 결과 중 터미널에 찍을 줄. 꺼진 인스턴스의 디스크 로그(`source: "disk"`)는 지난 기록이고
/// `since`를 무시하므로 찍지 않는다 (찍으면 같은 줄을 매번 다시 찍는다)
fn live_console_lines(result: &serde_json::Value) -> &[serde_json::Value] {
    if result.get("source").and_then(|v| v.as_str()) == Some("disk") {
        return &[];
    }
    result.get("lines").and_then(|v| v.as_array()).map_or(&[], Vec::as_slice)
}

// ─── Tests ───────────────────────────────────────────────────

// ─── Log File ────────────────────────────────────────────────

/// 데몬 자신의 로그 파일 (`<data>/logs/daemon.log`). GUI가 띄운 데몬의 stderr는 GUI의 파이프라
/// GUI가 사라지면 로그가 아무 데도 남지 않는다. `cap` 바이트를 넘으면 `.1`로 넘기고 새로 쓴다
pub struct LogFile {
    path: std::path::PathBuf,
    cap: u64,
    file: Mutex<Option<(std::fs::File, u64)>>,
}

impl LogFile {
    pub fn open(path: &std::path::Path, cap: u64) -> Arc<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = Self::append(path);
        Arc::new(Self { path: path.to_path_buf(), cap, file: Mutex::new(file) })
    }

    fn append(path: &std::path::Path) -> Option<(std::fs::File, u64)> {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Some((file, len))
    }

    /// tracing `with_writer`에 넘길 쓰기 핸들
    pub fn writer(self: &Arc<Self>) -> LogFileWriter {
        LogFileWriter(self.clone())
    }

    /// 패닉 훅에서 부른다: tracing을 거치지 않고 바로 쓴다
    pub fn write_panic(&self, message: &str) {
        let line = format!("{} PANIC {}\n", chrono::Utc::now().to_rfc3339(), message);
        self.write_line(line.as_bytes());
    }

    /// 쓰기 실패는 삼킨다: 로그 때문에 데몬이 멈추거나 죽지 않게
    fn write_line(&self, buf: &[u8]) {
        let Ok(mut guard) = self.file.lock() else { return };
        if guard.as_ref().is_some_and(|(_, len)| *len > 0 && len + buf.len() as u64 > self.cap) {
            *guard = None;
            let mut rolled = self.path.as_os_str().to_os_string();
            rolled.push(".1");
            if std::fs::rename(&self.path, rolled).is_err() {
                // 넘기지 못하면 비우고 새로 쓴다: 상한을 지키는 쪽이 낫다
                let _ = std::fs::OpenOptions::new().write(true).truncate(true).open(&self.path);
            }
        }
        if guard.is_none() {
            *guard = Self::append(&self.path);
        }
        if let Some((file, len)) = guard.as_mut() {
            if std::io::Write::write_all(file, buf).is_ok() {
                *len += buf.len() as u64;
            }
        }
    }
}

pub struct LogFileWriter(Arc<LogFile>);

impl std::io::Write for LogFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write_line(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 데몬 자신의 로그 파일: 크기 상한을 넘으면 `.1`로 넘기고, 패닉도 같은 파일에 남는다
    #[test]
    fn the_log_file_rolls_over_and_keeps_panics() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("logs").join("daemon.log");
        let log = LogFile::open(&path, 32);
        let mut w = log.writer();
        std::io::Write::write_all(&mut w, b"first line that is long enough to fill the cap..\n").unwrap();
        std::io::Write::write_all(&mut w, b"second\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
        assert!(std::fs::read_to_string(path.with_extension("log.1")).unwrap().starts_with("first line"));
        log.write_panic("boom at src/x.rs:1");
        assert!(std::fs::read_to_string(&path).unwrap().contains("PANIC boom at src/x.rs:1"));
    }

    /// `.1`로 넘기지 못해도(이름 바꾸기 실패) 파일이 끝없이 커지지 않는다
    #[test]
    fn the_log_file_stays_capped_when_it_cannot_roll_over() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("daemon.log");
        let blocker = path.with_extension("log.1");
        std::fs::create_dir_all(blocker.join("busy")).unwrap();
        let log = LogFile::open(&path, 32);
        let mut w = log.writer();
        for _ in 0..20 {
            std::io::Write::write_all(&mut w, b"a line of about twenty\n").unwrap();
        }
        assert!(std::fs::metadata(&path).unwrap().len() <= 32, "the file must not grow past the cap");
    }

    // 백로그 L6: 꺼진 인스턴스는 디스크 로그로 답하는데, 그 답은 since를 무시해 같은 200줄을 0.5초마다 다시 찍었다
    #[test]
    fn the_terminal_shows_only_live_console_lines() {
        let line = serde_json::json!({ "id": 3, "content": "hi" });
        let live = serde_json::json!({ "success": true, "lines": [line.clone()], "running": true });
        assert_eq!(live_console_lines(&live), &[line.clone()][..]);
        let disk = serde_json::json!({ "success": true, "lines": [line], "running": false, "source": "disk" });
        assert!(live_console_lines(&disk).is_empty(), "past log on disk is not live output");
    }

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

    // D-S14: 번호를 잠금 밖에서 매기면 늦은 번호가 먼저 들어가, since 폴링이 이른 번호 한 줄을 건너뛴다
    #[test]
    fn ids_follow_buffer_order_when_threads_log_at_once() {
        let buf = DaemonLogBuffer::new();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let buf = buf.clone();
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        buf.push("info".into(), "t".into(), "x".into());
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let ids: Vec<u64> = buf.get_since(Some(1), Some(10_000)).iter().map(|e| e.id).collect();
        assert_eq!(ids, (1..=8000).collect::<Vec<_>>(), "ids must rise in the order lines enter the buffer");
    }

    #[test]
    fn recent_where_keeps_the_latest_matches_oldest_first() {
        let buf = DaemonLogBuffer::new();
        for (level, msg) in [("warn", "a"), ("info", "b"), ("error", "c"), ("warn", "d")] {
            buf.push(level.into(), "t".into(), msg.into());
        }
        let got: Vec<_> = buf.recent_where(2, |e| e.level != "info").into_iter().map(|e| e.message).collect();
        assert_eq!(got, vec!["c", "d"]);
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

        let entries = buf.get_since(None, None);
        assert_eq!(entries.len(), 5);
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
