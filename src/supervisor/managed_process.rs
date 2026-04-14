//! Managed Process - Direct process spawning with stdio capture
//!
//! The core daemon can directly manage server processes with:
//! - Real-time stdout/stderr capture and log buffering
//! - stdin command injection
//! - Configurable log level parsing via module log_pattern
//! - Process lifecycle tracking with running state watch

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{broadcast, mpsc, Mutex, watch};
use serde::{Serialize, Deserialize};
use anyhow::Result;
use regex::Regex;

/// Console log file name within the instance's logs directory.
const CONSOLE_LOG_FILENAME: &str = "console.log";
/// Maximum console log file size before rotation (10 MB).
const CONSOLE_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// Number of rotated files to keep.
const CONSOLE_LOG_ROTATIONS: usize = 2;

/// Default maximum number of log lines to keep in the ring buffer.
const DEFAULT_LOG_BUFFER: usize = 10_000;

// ─── Log Types ───────────────────────────────────────────────

/// A single line of console output from the managed process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    /// Sequential ID for polling (`GET /console?since=<id>`)
    pub id: u64,
    /// Unix timestamp (seconds)
    pub timestamp: u64,
    /// Where the line came from
    pub source: LogSource,
    /// Raw text content
    pub content: String,
    /// Parsed severity level
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    Stdout,
    Stderr,
    /// System messages from saba-chan itself
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

// ─── Log Buffer ──────────────────────────────────────────────

/// Ring buffer that stores recent log lines with sequential IDs.
/// Optionally persists every line to a disk log file via an async channel.
struct LogBuffer {
    lines: VecDeque<LogLine>,
    next_id: u64,
    max_size: usize,
    /// Async sender for disk persistence (None = no persistence)
    file_tx: Option<mpsc::UnboundedSender<String>>,
}

impl LogBuffer {
    fn new() -> Self {
        Self::with_capacity(DEFAULT_LOG_BUFFER)
    }

    fn with_capacity(max_size: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_size),
            next_id: 0,
            max_size,
            file_tx: None,
        }
    }

    /// Push a new log line and return the created `LogLine`.
    fn push(&mut self, source: LogSource, content: String, level: LogLevel) -> LogLine {
        let line = LogLine {
            id: self.next_id,
            timestamp: current_timestamp(),
            source,
            content,
            level,
        };
        self.next_id += 1;

        if self.lines.len() >= self.max_size {
            self.lines.pop_front();
        }
        self.lines.push_back(line.clone());

        // Persist to disk — writer 실패 시 시스템 경고를 콘솔 버퍼에 삽입
        if let Some(ref tx) = self.file_tx {
            let disk_line = format_log_line_for_disk(&line);
            if tx.send(disk_line).is_err() {
                // Writer task가 죽었으므로 더 이상 디스크에 쓰지 않음
                self.file_tx = None;
                tracing::warn!("Log disk writer channel closed — disk logging disabled");
                // GUI에 표시될 시스템 경고 라인 삽입
                let warn_line = LogLine {
                    id: self.next_id,
                    timestamp: current_timestamp(),
                    source: LogSource::System,
                    content: "⚠ Log file writer stopped — disk logs may be incomplete. Check disk space.".to_string(),
                    level: LogLevel::Warn,
                };
                self.next_id += 1;
                if self.lines.len() >= self.max_size {
                    self.lines.pop_front();
                }
                self.lines.push_back(warn_line);
            }
        }

        line
    }

    /// Get all lines with id >= `since_id` (for polling).
    fn get_since(&self, since_id: u64) -> Vec<LogLine> {
        self.lines.iter()
            .filter(|l| l.id >= since_id)
            .cloned()
            .collect()
    }

    /// Get the most recent `count` lines.
    fn get_recent(&self, count: usize) -> Vec<LogLine> {
        self.lines.iter().rev().take(count).rev().cloned().collect()
    }
}

// ─── Managed Process ─────────────────────────────────────────

/// A server process managed directly by the core daemon.
///
/// Provides:
/// - Async stdin command injection via `send_command()`
/// - Buffered console output via `get_console_since()` / `get_recent_console()`
/// - Real-time log broadcast via `subscribe()`
/// - Running state monitoring via `is_running()`
pub struct ManagedProcess {
    /// Channel to send commands to stdin
    stdin_tx: mpsc::Sender<String>,
    /// Log buffer for recent console output
    log_buffer: Arc<Mutex<LogBuffer>>,
    /// Broadcast channel for real-time log events
    #[allow(dead_code)]
    log_broadcast: broadcast::Sender<LogLine>,
    /// Process PID
    pub pid: u32,
    /// Watch channel for running state
    #[allow(dead_code)]
    running_tx: Arc<watch::Sender<bool>>,
    running_rx: watch::Receiver<bool>,
    /// Abort handle for the background poller task (reattached stubs only)
    poller_handle: Option<tokio::task::AbortHandle>,
    /// Whether stdin is available (false for reattached stubs)
    stdin_available: bool,
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if let Some(handle) = self.poller_handle.take() {
            handle.abort();
        }
    }
}

impl ManagedProcess {
    /// Spawn a new managed process.
    ///
    /// # Arguments
    /// * `program` - Executable to run (e.g., `"java"`)
    /// * `args` - Command-line arguments
    /// * `working_dir` - Working directory
    /// * `env_vars` - Extra environment variables
    /// * `log_pattern` - Optional regex pattern for extracting log level from output lines.
    ///   The pattern should have a named capture group `level` matching
    ///   INFO, WARN, ERROR, DEBUG etc. If None, all lines default to Info.
    /// * `instance_dir` - Optional instance directory for console log persistence.
    ///   If provided, all console output is also written to `{instance_dir}/logs/console.log`.
    pub async fn spawn(
        program: &str,
        args: &[String],
        working_dir: &str,
        env_vars: Vec<(String, String)>,
        log_pattern: Option<&str>,
        instance_dir: Option<&Path>,
    ) -> Result<Self> {
        let mut cmd = TokioCommand::new(program);
        cmd.args(args)
            .current_dir(working_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(false);

        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        // Windows: hide console window
        crate::utils::apply_creation_flags(&mut cmd);

        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", program, e))?;

        let pid = child.id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get PID of spawned process"))?;

        // Channels
        let (stdin_tx, stdin_rx) = mpsc::channel::<String>(256);
        let (log_tx, _) = broadcast::channel::<LogLine>(2048);
        let (running_tx, running_rx) = watch::channel(true);

        let mut buf = LogBuffer::new();
        // Attach disk writer if instance_dir is provided
        if let Some(dir) = instance_dir {
            let log_path = console_log_path(dir);
            buf.file_tx = Some(spawn_disk_writer(log_path));
        }
        let log_buffer = Arc::new(Mutex::new(buf));
        let running_tx = Arc::new(running_tx);

        // Compile log pattern regex (shared across stdout/stderr readers)
        let log_regex = log_pattern.and_then(|pat| {
            match Regex::new(pat) {
                Ok(re) => Some(Arc::new(re)),
                Err(e) => {
                    tracing::warn!("Invalid log_pattern '{}': {}, falling back to default", pat, e);
                    None
                }
            }
        });

        // Take ownership of stdio handles
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();

        // ── stdout reader ────────────────────────────────────
        if let Some(stdout) = stdout {
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            let re = log_regex.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let level = parse_log_level(&line, re.as_deref());
                    let log_line = buf.lock().await.push(LogSource::Stdout, line, level);
                    let _ = bc.send(log_line);
                }
            });
        }

        // ── stderr reader ────────────────────────────────────
        if let Some(stderr) = stderr {
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            let re = log_regex.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let level = parse_log_level(&line, re.as_deref());
                    // stderr lines default to at least Warn
                    let effective = if level == LogLevel::Info { LogLevel::Warn } else { level };
                    let log_line = buf.lock().await.push(LogSource::Stderr, line, effective);
                    let _ = bc.send(log_line);
                }
            });
        }

        // ── stdin writer ─────────────────────────────────────
        if let Some(mut stdin_handle) = stdin {
            let mut rx = stdin_rx;
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    let data = if cmd.ends_with('\n') { cmd } else { format!("{}\n", cmd) };
                    if stdin_handle.write_all(data.as_bytes()).await.is_err() {
                        break;
                    }
                    if stdin_handle.flush().await.is_err() {
                        break;
                    }
                }
            });
        }

        // ── process waiter ───────────────────────────────────
        {
            let running = running_tx.clone();
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            tokio::spawn(async move {
                let exit_msg = match child.wait().await {
                    Ok(status) => format!("Process exited with {}", status),
                    Err(e) => format!("Failed to wait for process: {}", e),
                };
                tracing::info!("{}", exit_msg);
                let log_line = buf.lock().await.push(LogSource::System, exit_msg, LogLevel::Info);
                let _ = bc.send(log_line);
                let _ = running.send(false);
            });
        }

        // System log entry
        {
            let msg = format!("Process started with PID {}", pid);
            let log_line = log_buffer.lock().await.push(LogSource::System, msg, LogLevel::Info);
            let _ = log_tx.send(log_line);
        }

        Ok(Self {
            stdin_tx,
            log_buffer,
            log_broadcast: log_tx,
            pid,
            running_tx,
            running_rx,
            poller_handle: None,
            stdin_available: true,
        })
    }

    /// Send a command string to the process's stdin.
    pub async fn send_command(&self, command: &str) -> Result<()> {
        self.stdin_tx.send(command.to_string()).await
            .map_err(|e| anyhow::anyhow!("stdin channel closed: {}", e))
    }

    /// Spawn a log follower that streams an extension-provided command's output
    /// into the standard LogBuffer/broadcast infrastructure.
    ///
    /// This allows extension-managed processes (e.g. containers) to share
    /// the same console API (`GET /api/instance/:id/console?since=N`) as native
    /// managed processes.
    ///
    /// # Arguments
    /// * `program`       - The executable to run (e.g. container runtime, "wsl")
    /// * `args`          - Arguments for the command
    /// * `working_dir`   - Working directory for the command
    /// * `description`   - Human-readable label for log messages
    /// * `log_pattern`   - Optional regex for log level parsing
    /// * `strip_prefix`  - Optional separator to strip from each line (e.g. " | " for compose logs)
    /// * `instance_dir`  - Optional instance directory for console log persistence
    pub async fn spawn_log_follower(
        program: &str,
        args: &[String],
        working_dir: &Path,
        description: &str,
        log_pattern: Option<&str>,
        strip_prefix: Option<&str>,
        instance_dir: Option<&Path>,
    ) -> Result<Self> {
        let mut cmd = TokioCommand::new(program);
        cmd.args(args);

        cmd.current_dir(working_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        crate::utils::apply_creation_flags(&mut cmd);

        let description_owned = description.to_string();
        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!(
                "Failed to spawn log follower '{}': {}",
                description_owned, e
            ))?;

        let pid = child.id().unwrap_or(0);

        // Channels — stdin is not used for container log followers, but we keep the
        // interface compatible with ManagedProcess.
        let (stdin_tx, _stdin_rx) = mpsc::channel::<String>(1);
        let (log_tx, _) = broadcast::channel::<LogLine>(2048);
        let (running_tx, running_rx) = watch::channel(true);

        let mut buf = LogBuffer::new();
        if let Some(dir) = instance_dir {
            let log_path = console_log_path(dir);
            buf.file_tx = Some(spawn_disk_writer(log_path));
        }
        let log_buffer = Arc::new(Mutex::new(buf));
        let running_tx = Arc::new(running_tx);

        let log_regex = log_pattern.and_then(|pat| {
            match regex::Regex::new(pat) {
                Ok(re) => Some(Arc::new(re)),
                Err(e) => {
                    tracing::warn!("Invalid log_pattern '{}': {}", pat, e);
                    None
                }
            }
        });

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // ── stdout reader ──
        if let Some(stdout) = stdout {
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            let re = log_regex.clone();
            let prefix = strip_prefix.map(String::from);
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let content = if let Some(ref sep) = prefix {
                        if let Some(pos) = line.find(sep.as_str()) {
                            line[pos + sep.len()..].to_string()
                        } else {
                            line
                        }
                    } else {
                        line
                    };
                    let level = parse_log_level(&content, re.as_deref());
                    let log_line = buf.lock().await.push(LogSource::Stdout, content, level);
                    let _ = bc.send(log_line);
                }
            });
        }

        // ── stderr reader ──
        if let Some(stderr) = stderr {
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            let re = log_regex;
            let prefix = strip_prefix.map(String::from);
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let content = if let Some(ref sep) = prefix {
                        if let Some(pos) = line.find(sep.as_str()) {
                            line[pos + sep.len()..].to_string()
                        } else {
                            line
                        }
                    } else {
                        line
                    };
                    let level = parse_log_level(&content, re.as_deref());
                    let effective = if level == LogLevel::Info { LogLevel::Warn } else { level };
                    let log_line = buf.lock().await.push(LogSource::Stderr, content, effective);
                    let _ = bc.send(log_line);
                }
            });
        }

        // ── process waiter ──
        {
            let running = running_tx.clone();
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            let desc = description_owned.clone();
            tokio::spawn(async move {
                let exit_msg = match child.wait().await {
                    Ok(status) => format!("Log follower '{}' exited with {}", desc, status),
                    Err(e) => format!("Log follower '{}' error: {}", desc, e),
                };
                tracing::info!("{}", exit_msg);
                let log_line = buf.lock().await.push(LogSource::System, exit_msg, LogLevel::Info);
                let _ = bc.send(log_line);
                let _ = running.send(false);
            });
        }

        // System log
        {
            let msg = format!("Log streaming started: '{}'", description_owned);
            tracing::info!("{}", msg);
            let log_line = log_buffer.lock().await.push(LogSource::System, msg, LogLevel::Info);
            let _ = log_tx.send(log_line);
        }

        Ok(Self {
            stdin_tx,
            log_buffer,
            log_broadcast: log_tx,
            pid,
            running_tx,
            running_rx,
            poller_handle: None,
            stdin_available: false, // log follower has no stdin
        })
    }

    /// Get all log lines with `id >= since_id`.
    pub async fn get_console_since(&self, since_id: u64) -> Vec<LogLine> {
        self.log_buffer.lock().await.get_since(since_id)
    }

    /// Get the most recent `count` log lines.
    pub async fn get_recent_console(&self, count: usize) -> Vec<LogLine> {
        self.log_buffer.lock().await.get_recent(count)
    }

    /// Subscribe to real-time log events.
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.log_broadcast.subscribe()
    }

    /// Whether the process is still running.
    pub fn is_running(&self) -> bool {
        *self.running_rx.borrow()
    }

    /// Whether stdin commands can be sent to this process.
    pub fn is_stdin_available(&self) -> bool {
        self.stdin_available
    }

    /// Wait until the process exits.
    #[allow(dead_code)]
    pub async fn wait_for_exit(&mut self) {
        while self.is_running() {
            if self.running_rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// Create a stub ManagedProcess for an orphaned server process.
    ///
    /// This is used when the daemon restarts and finds a still-running server.
    /// The stub:
    /// - Loads console history from the disk log file
    /// - Monitors the process liveness via periodic PID polling
    /// - Cannot send stdin commands (stdio pipes are lost)
    /// - Continues writing new log lines to disk
    pub async fn create_reattached_stub(
        pid: u32,
        instance_dir: &Path,
    ) -> Self {
        let (stdin_tx, _stdin_rx) = mpsc::channel::<String>(1);
        let (log_tx, _) = broadcast::channel::<LogLine>(2048);
        let (running_tx, running_rx) = watch::channel(true);
        let running_tx = Arc::new(running_tx);

        // Load history from disk (blocking I/O → spawn_blocking)
        let dir_owned = instance_dir.to_path_buf();
        let history = tokio::task::spawn_blocking(move || {
            read_console_log_from_disk(&dir_owned, DEFAULT_LOG_BUFFER)
        }).await.unwrap_or_default();
        let next_id = history.last().map(|l| l.id + 1).unwrap_or(0);

        let mut log_buf = LogBuffer::with_capacity(DEFAULT_LOG_BUFFER);
        log_buf.next_id = next_id;
        for line in history {
            log_buf.lines.push_back(line);
        }

        // Attach disk writer for any new lines
        let log_path = console_log_path(instance_dir);
        log_buf.file_tx = Some(spawn_disk_writer(log_path));

        // Add system message about reattachment
        let sys_msg = format!(
            "── Daemon restarted. Reattached to running process (PID {}). \
             stdin is unavailable until next restart. ──",
            pid
        );
        log_buf.push(LogSource::System, sys_msg, LogLevel::Warn);

        let log_buffer = Arc::new(Mutex::new(log_buf));

        // Spawn a background task to monitor process liveness
        let poller_handle = {
            let running = running_tx.clone();
            let buf = log_buffer.clone();
            let bc = log_tx.clone();
            let handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    if !super::process::is_process_alive(pid) {
                        let exit_msg = format!("Process (PID {}) exited", pid);
                        tracing::info!("{}", exit_msg);
                        let log_line = buf.lock().await.push(
                            LogSource::System, exit_msg, LogLevel::Info
                        );
                        let _ = bc.send(log_line);
                        let _ = running.send(false);
                        break;
                    }
                }
            });
            handle.abort_handle()
        };

        Self {
            stdin_tx,
            log_buffer,
            log_broadcast: log_tx,
            pid,
            running_tx,
            running_rx,
            poller_handle: Some(poller_handle),
            stdin_available: false, // reattached stub — stdio pipes lost
        }
    }
}

// ─── Managed Process Store ───────────────────────────────────

/// Central store for all managed processes. Thread-safe.
pub struct ManagedProcessStore {
    processes: Mutex<HashMap<String, Arc<ManagedProcess>>>,
}

impl ManagedProcessStore {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }

    /// Register a managed process under an instance ID.
    pub async fn insert(&self, instance_id: &str, process: ManagedProcess) {
        let mut map = self.processes.lock().await;
        map.insert(instance_id.to_string(), Arc::new(process));
    }

    /// Get a managed process by instance ID.
    pub async fn get(&self, instance_id: &str) -> Option<Arc<ManagedProcess>> {
        let map = self.processes.lock().await;
        map.get(instance_id).cloned()
    }

    /// Remove a managed process (e.g., after it exits).
    #[allow(dead_code)]
    pub async fn remove(&self, instance_id: &str) -> Option<Arc<ManagedProcess>> {
        let mut map = self.processes.lock().await;
        map.remove(instance_id)
    }

    /// 현재 실행 중인 인스턴스 ID 목록 반환
    pub async fn running_instance_ids(&self) -> Vec<String> {
        let map = self.processes.lock().await;
        map.iter()
            .filter(|(_, proc)| proc.is_running())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Clean up processes that are no longer running.
    /// Returns the list of instance IDs that were removed.
    pub async fn cleanup_dead(&self) -> Vec<String> {
        let mut map = self.processes.lock().await;
        let mut removed = Vec::new();
        map.retain(|id, proc| {
            if !proc.is_running() {
                tracing::info!("Cleaning up dead managed process for instance '{}'", id);
                removed.push(id.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}

impl Default for ManagedProcessStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ─────────────────────────────────────────────────

/// Parse the log level from a server log line using an optional regex pattern.
///
/// If a pattern is provided, it should contain a named capture group `level`
/// that matches level keywords (INFO, WARN, ERROR, DEBUG, etc.).
/// If no pattern is provided, defaults to Info.
///
/// Example patterns:
///   Minecraft: `/(?P<level>INFO|WARN|ERROR|DEBUG|FATAL)\]`
///   Generic:   `(?P<level>INFO|WARN|ERROR|DEBUG|TRACE|FATAL)`
fn parse_log_level(line: &str, pattern: Option<&Regex>) -> LogLevel {
    if let Some(re) = pattern {
        if let Some(caps) = re.captures(line) {
            if let Some(level_match) = caps.name("level") {
                return match level_match.as_str().to_uppercase().as_str() {
                    "ERROR" | "FATAL" => LogLevel::Error,
                    "WARN" | "WARNING" => LogLevel::Warn,
                    "DEBUG" | "TRACE" => LogLevel::Debug,
                    _ => LogLevel::Info,
                };
            }
        }
    }
    LogLevel::Info
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ─── Disk Persistence Helpers ────────────────────────────────

/// Format a log line for disk output:
/// `[2026-03-05 12:34:56] [STDOUT/INFO] actual content`
fn format_log_line_for_disk(line: &LogLine) -> String {
    let dt = format_unix_timestamp(line.timestamp);
    let source = match line.source {
        LogSource::Stdout => "STDOUT",
        LogSource::Stderr => "STDERR",
        LogSource::System => "SYSTEM",
    };
    let level = match line.level {
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
        LogLevel::Debug => "DEBUG",
    };
    format!("[{}] [{}/{}] {}", dt, source, level, line.content)
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM:SS` (UTC).
fn format_unix_timestamp(ts: u64) -> String {
    let secs = ts;
    // Days since epoch
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Civil date from days since 1970-01-01 (Rata Die algorithm)
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hours, minutes, seconds)
}

/// Parse `YYYY-MM-DD HH:MM:SS` back to Unix timestamp.
fn parse_datetime_to_unix(s: &str) -> Option<u64> {
    // Expected: "2026-03-05 12:34:56"
    if s.len() < 19 { return None; }
    let y: i64 = s[0..4].parse().ok()?;
    let m: u64 = s[5..7].parse().ok()?;
    let d: u64 = s[8..10].parse().ok()?;
    let hh: u64 = s[11..13].parse().ok()?;
    let mm: u64 = s[14..16].parse().ok()?;
    let ss: u64 = s[17..19].parse().ok()?;

    // Inverse of civil date → days since epoch
    let y_adj = if m <= 2 { y - 1 } else { y };
    let m_adj = if m > 2 { m - 3 } else { m + 9 };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64;
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as u64 * 146097 + doe - 719468;

    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

/// Resolve the console log path for an instance:
/// `{instance_dir}/logs/console.log`
pub fn console_log_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join("logs").join(CONSOLE_LOG_FILENAME)
}

/// Spawn a background task that writes log lines to a file.
/// Returns an unbounded sender; dropping it cleanly stops the writer.
fn spawn_disk_writer(log_path: PathBuf) -> mpsc::UnboundedSender<String> {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    tokio::spawn(async move {
        use tokio::fs::{self, OpenOptions};
        use tokio::io::AsyncWriteExt as _;

        // Ensure parent directory exists
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent).await;
        }

        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
        {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to open console log '{}': {}", log_path.display(), e);
                return;
            }
        };

        let mut bytes_written: u64 = file.metadata().await.map(|m| m.len()).unwrap_or(0);

        while let Some(line) = rx.recv().await {
            let data = format!("{}\n", line);
            let data_len = data.len() as u64;
            if let Err(e) = file.write_all(data.as_bytes()).await {
                tracing::warn!("Disk writer for '{}' I/O error: {} — stopping log persistence", log_path.display(), e);
                break;
            }
            bytes_written += data_len;

            // Rotate if needed
            if bytes_written >= CONSOLE_LOG_MAX_BYTES {
                let _ = file.flush().await;
                rotate_log_files(&log_path).await;
                match OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                    .await
                {
                    Ok(f) => {
                        file = f;
                        bytes_written = 0;
                    }
                    Err(e) => {
                        tracing::warn!("Disk writer for '{}' rotation failed: {} — stopping log persistence", log_path.display(), e);
                        break;
                    }
                }
            }
        }

        let _ = file.flush().await;
    });

    tx
}

/// Rotate `console.log` → `console.log.1`, `console.log.1` → `console.log.2`, etc.
async fn rotate_log_files(log_path: &Path) {
    use tokio::fs;
    let base = log_path.to_string_lossy().to_string();
    // Delete the oldest rotated file first (Windows rename fails if dst exists)
    let oldest = format!("{}.{}", base, CONSOLE_LOG_ROTATIONS);
    let _ = fs::remove_file(&oldest).await;
    // Shift older files: .1 → .2, current → .1
    for i in (1..=CONSOLE_LOG_ROTATIONS).rev() {
        let src = if i == 1 {
            base.clone()
        } else {
            format!("{}.{}", base, i - 1)
        };
        let dst = format!("{}.{}", base, i);
        let _ = fs::rename(&src, &dst).await;
    }
}

/// Read the most recent `count` lines from the console log on disk.
/// Used to restore console history when the daemon restarts.
pub fn read_console_log_from_disk(instance_dir: &Path, count: usize) -> Vec<LogLine> {
    let log_path = console_log_path(instance_dir);
    let content = match std::fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let total_lines: Vec<&str> = content.lines().collect();
    let start = total_lines.len().saturating_sub(count);
    total_lines[start..]
        .iter()
        .enumerate()
        .map(|(i, raw)| parse_disk_log_line(i as u64, raw))
        .collect()
}

/// Parse a disk log line back into a `LogLine`.
/// Format: `[2026-03-05 12:34:56] [STDOUT/INFO] actual content`
fn parse_disk_log_line(id: u64, raw: &str) -> LogLine {
    // Try to parse the structured format
    let (timestamp, source, level, content) =
        if raw.starts_with('[') {
            parse_structured_disk_line(raw)
        } else {
            (current_timestamp(), LogSource::Stdout, LogLevel::Info, raw.to_string())
        };

    LogLine { id, timestamp, source, content, level }
}

/// Parse `[datetime] [SOURCE/LEVEL] content`
fn parse_structured_disk_line(raw: &str) -> (u64, LogSource, LogLevel, String) {
    // Find first `]` for datetime
    let dt_end = raw.find(']').unwrap_or(0);
    let after_dt = &raw[dt_end + 1..];

    // Find `[SOURCE/LEVEL]`
    let trimmed = after_dt.trim_start();
    if let Some(bracket_start) = trimmed.find('[') {
        if let Some(bracket_end) = trimmed[bracket_start..].find(']') {
            let tag = &trimmed[bracket_start + 1..bracket_start + bracket_end];
            let content = trimmed[bracket_start + bracket_end + 1..].trim_start().to_string();

            let (source, level) = if let Some(slash) = tag.find('/') {
                let src = match &tag[..slash] {
                    "STDERR" => LogSource::Stderr,
                    "SYSTEM" => LogSource::System,
                    _ => LogSource::Stdout,
                };
                let lvl = match &tag[slash + 1..] {
                    "WARN" => LogLevel::Warn,
                    "ERROR" => LogLevel::Error,
                    "DEBUG" => LogLevel::Debug,
                    _ => LogLevel::Info,
                };
                (src, lvl)
            } else {
                (LogSource::Stdout, LogLevel::Info)
            };

            // Try parsing timestamp
            let dt_str = &raw[1..dt_end];
            let timestamp = parse_datetime_to_unix(dt_str)
                .unwrap_or_else(current_timestamp);

            return (timestamp, source, level, content);
        }
    }

    (current_timestamp(), LogSource::Stdout, LogLevel::Info, raw.to_string())
}

// ─── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ═══════════════════════════════════════════════════════
    // LogBuffer 단위 테스트
    // ═══════════════════════════════════════════════════════

    #[test]
    fn test_log_buffer_push_and_query() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogSource::Stdout, "line 0".into(), LogLevel::Info);
        buffer.push(LogSource::Stdout, "line 1".into(), LogLevel::Info);
        buffer.push(LogSource::Stderr, "err 0".into(), LogLevel::Error);

        assert_eq!(buffer.lines.len(), 3);
        // get_since(0) → id >= 0 → 전체 3줄 반환
        assert_eq!(buffer.get_since(0).len(), 3);
        // get_since(1) → id >= 1 → 2줄 반환 (id=0 제외)
        assert_eq!(buffer.get_since(1).len(), 2);
        assert_eq!(buffer.get_recent(2).len(), 2);
        assert_eq!(buffer.get_recent(100).len(), 3);
    }

    #[test]
    fn test_log_buffer_ring() {
        let mut buffer = LogBuffer::new();
        for i in 0..(DEFAULT_LOG_BUFFER + 100) {
            buffer.push(LogSource::Stdout, format!("line {}", i), LogLevel::Info);
        }
        assert_eq!(buffer.lines.len(), DEFAULT_LOG_BUFFER);
        assert!(buffer.lines.front().unwrap().id > 0);
    }

    #[test]
    fn test_log_buffer_custom_capacity() {
        let mut buffer = LogBuffer::with_capacity(5);
        for i in 0..10 {
            buffer.push(LogSource::Stdout, format!("line {}", i), LogLevel::Info);
        }
        assert_eq!(buffer.lines.len(), 5);
        // 가장 오래된 것이 evict되었으므로 첫 라인은 "line 5"
        assert_eq!(buffer.lines.front().unwrap().content, "line 5");
        assert_eq!(buffer.lines.back().unwrap().content, "line 9");
    }

    #[test]
    fn test_log_buffer_ids_are_monotonic() {
        let mut buffer = LogBuffer::new();
        let l1 = buffer.push(LogSource::Stdout, "a".into(), LogLevel::Info);
        let l2 = buffer.push(LogSource::Stdout, "b".into(), LogLevel::Info);
        let l3 = buffer.push(LogSource::Stdout, "c".into(), LogLevel::Info);
        assert!(l1.id < l2.id, "IDs must be monotonically increasing");
        assert!(l2.id < l3.id, "IDs must be monotonically increasing");
    }

    #[test]
    fn test_log_buffer_get_since_returns_from_id_inclusive() {
        let mut buffer = LogBuffer::new();
        let l1 = buffer.push(LogSource::Stdout, "first".into(), LogLevel::Info);    // id=0
        let _l2 = buffer.push(LogSource::Stdout, "second".into(), LogLevel::Info);  // id=1
        let _l3 = buffer.push(LogSource::Stdout, "third".into(), LogLevel::Info);   // id=2

        // get_since(0) → id >= 0 → 모든 라인 반환
        let since = buffer.get_since(l1.id);
        assert_eq!(since.len(), 3, "Should return all 3 lines from id {}", l1.id);
        assert_eq!(since[0].content, "first");
        assert_eq!(since[1].content, "second");
        assert_eq!(since[2].content, "third");

        // get_since(1) → id >= 1 → "first" 제외
        let since_1 = buffer.get_since(1);
        assert_eq!(since_1.len(), 2);
        assert_eq!(since_1[0].content, "second");
        assert_eq!(since_1[1].content, "third");
    }

    /// GUI 폴링 시퀀스 시뮬레이션:
    /// 1. 초기 폴링: since=0 → 전체 반환
    /// 2. 이후 폴링: since=last_id+1 → 새 라인만 반환, 누락 없음
    #[test]
    fn test_log_buffer_polling_sequence_no_missing_lines() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogSource::Stdout, "line0".into(), LogLevel::Info); // id=0
        buffer.push(LogSource::Stdout, "line1".into(), LogLevel::Info); // id=1
        buffer.push(LogSource::Stdout, "line2".into(), LogLevel::Info); // id=2

        // Poll 1: since=0 → id >= 0 → 전체 반환 (id=0 포함)
        let poll1 = buffer.get_since(0);
        assert_eq!(poll1.len(), 3);
        assert_eq!(poll1[0].content, "line0");
        let since_id = poll1.last().unwrap().id + 1; // GUI: sinceId = 2 + 1 = 3

        // 새 라인 추가
        buffer.push(LogSource::Stdout, "line3".into(), LogLevel::Info); // id=3
        buffer.push(LogSource::Stdout, "line4".into(), LogLevel::Info); // id=4

        // Poll 2: since=3 → id >= 3 → line3, line4 반환 (line3 누락 없음)
        let poll2 = buffer.get_since(since_id);
        assert_eq!(poll2.len(), 2);
        assert_eq!(poll2[0].content, "line3");
        assert_eq!(poll2[1].content, "line4");
    }

    #[test]
    fn test_log_buffer_get_since_future_id_returns_empty() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogSource::Stdout, "a".into(), LogLevel::Info);
        let since = buffer.get_since(999999);
        assert!(since.is_empty(), "Future ID should return no results");
    }

    #[test]
    fn test_log_buffer_get_recent_zero() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogSource::Stdout, "a".into(), LogLevel::Info);
        let recent = buffer.get_recent(0);
        assert!(recent.is_empty());
    }

    #[test]
    fn test_log_buffer_empty_queries() {
        let buffer = LogBuffer::new();
        assert!(buffer.get_since(0).is_empty());
        assert!(buffer.get_recent(10).is_empty());
    }

    #[test]
    fn test_log_buffer_source_and_level_preserved() {
        let mut buffer = LogBuffer::new();
        let l1 = buffer.push(LogSource::Stdout, "out".into(), LogLevel::Info);
        let l2 = buffer.push(LogSource::Stderr, "err".into(), LogLevel::Error);
        let l3 = buffer.push(LogSource::System, "sys".into(), LogLevel::Warn);

        assert!(matches!(l1.source, LogSource::Stdout));
        assert!(matches!(l2.source, LogSource::Stderr));
        assert!(matches!(l3.source, LogSource::System));
        assert_eq!(l1.level, LogLevel::Info);
        assert_eq!(l2.level, LogLevel::Error);
        assert_eq!(l3.level, LogLevel::Warn);
    }

    #[test]
    fn test_log_buffer_timestamp_is_set() {
        let mut buffer = LogBuffer::new();
        let line = buffer.push(LogSource::Stdout, "hello".into(), LogLevel::Info);
        assert!(line.timestamp > 0, "Timestamp should be valid Unix epoch");
    }

    // ═══════════════════════════════════════════════════════
    // LogLevel 파싱 테스트
    // ═══════════════════════════════════════════════════════

    #[test]
    fn test_parse_log_level_with_pattern() {
        let mc_pattern = Regex::new(r"/(?P<level>INFO|WARN|ERROR|DEBUG|FATAL)\]").unwrap();
        assert_eq!(
            parse_log_level("[12:00:00] [Server thread/INFO]: Done (5.123s)!", Some(&mc_pattern)),
            LogLevel::Info
        );
        assert_eq!(
            parse_log_level("[12:00:00] [Server thread/WARN]: Can't keep up!", Some(&mc_pattern)),
            LogLevel::Warn
        );
        assert_eq!(
            parse_log_level("[12:00:00] [Server thread/ERROR]: Encountered an unexpected exception", Some(&mc_pattern)),
            LogLevel::Error
        );
        assert_eq!(
            parse_log_level("[12:00:00] [Server thread/DEBUG]: Reloading ResourceManager", Some(&mc_pattern)),
            LogLevel::Debug
        );
        assert_eq!(
            parse_log_level("Some random output", Some(&mc_pattern)),
            LogLevel::Info
        );
    }

    #[test]
    fn test_parse_log_level_fatal_maps_to_error() {
        let pattern = Regex::new(r"(?P<level>INFO|WARN|ERROR|DEBUG|FATAL)").unwrap();
        assert_eq!(
            parse_log_level("FATAL: out of memory", Some(&pattern)),
            LogLevel::Error
        );
    }

    #[test]
    fn test_parse_log_level_warning_variant() {
        let pattern = Regex::new(r"(?P<level>INFO|WARNING|ERROR|DEBUG)").unwrap();
        assert_eq!(
            parse_log_level("WARNING: disk nearly full", Some(&pattern)),
            LogLevel::Warn
        );
    }

    #[test]
    fn test_parse_log_level_trace_maps_to_debug() {
        let pattern = Regex::new(r"(?P<level>INFO|WARN|ERROR|DEBUG|TRACE)").unwrap();
        assert_eq!(
            parse_log_level("TRACE: entering function", Some(&pattern)),
            LogLevel::Debug
        );
    }

    #[test]
    fn test_parse_log_level_without_pattern() {
        assert_eq!(parse_log_level("[12:00:00] [Server thread/ERROR]: err", None), LogLevel::Info);
        assert_eq!(parse_log_level("Some random output", None), LogLevel::Info);
    }

    // ═══════════════════════════════════════════════════════
    // ManagedProcessStore 테스트
    // ═══════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_managed_process_store_empty() {
        let store = ManagedProcessStore::new();
        assert!(store.get("test").await.is_none());
        assert!(store.running_instance_ids().await.is_empty());
    }

    #[tokio::test]
    async fn test_managed_process_store_remove_returns_process() {
        let store = ManagedProcessStore::new();
        // 빈 상태에서 remove → None
        assert!(store.remove("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_managed_process_store_cleanup_dead_on_empty() {
        let store = ManagedProcessStore::new();
        // 빈 상태에서 cleanup → 패닉 없이 정상
        store.cleanup_dead().await;
        assert!(store.running_instance_ids().await.is_empty());
    }

    #[test]
    fn test_current_timestamp_is_reasonable() {
        let ts = current_timestamp();
        // 2024-01-01 이후 (Unix epoch 1704067200)
        assert!(ts > 1_704_067_200, "Timestamp seems too old: {}", ts);
        // 2050-01-01 이전 (Unix epoch 2524608000)
        assert!(ts < 2_524_608_000, "Timestamp seems too new: {}", ts);
    }
}
