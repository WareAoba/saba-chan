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
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{broadcast, mpsc, Mutex, watch};
use serde::{Serialize, Deserialize};
use anyhow::Result;
use regex::Regex;

/// Default maximum number of log lines to keep in the ring buffer.
/// Can be overridden via `log_buffer_size` in config/global.toml.
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
struct LogBuffer {
    lines: VecDeque<LogLine>,
    next_id: u64,
    max_size: usize,
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
        line
    }

    /// Get all lines with id > `since_id` (for polling).
    fn get_since(&self, since_id: u64) -> Vec<LogLine> {
        self.lines.iter()
            .filter(|l| l.id > since_id)
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
    pub async fn spawn(
        program: &str,
        args: &[String],
        working_dir: &str,
        env_vars: Vec<(String, String)>,
        log_pattern: Option<&str>,
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

        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
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
    pub async fn spawn_log_follower(
        program: &str,
        args: &[String],
        working_dir: &Path,
        description: &str,
        log_pattern: Option<&str>,
        strip_prefix: Option<&str>,
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

        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
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
        })
    }

    /// Get all log lines with `id > since_id`.
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

    /// Wait until the process exits.
    #[allow(dead_code)]
    pub async fn wait_for_exit(&mut self) {
        while self.is_running() {
            if self.running_rx.changed().await.is_err() {
                break;
            }
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
    pub async fn cleanup_dead(&self) {
        let mut map = self.processes.lock().await;
        map.retain(|id, proc| {
            if !proc.is_running() {
                tracing::info!("Cleaning up dead managed process for instance '{}'", id);
                false
            } else {
                true
            }
        });
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

// ─── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_push_and_query() {
        let mut buffer = LogBuffer::new();
        buffer.push(LogSource::Stdout, "line 0".into(), LogLevel::Info);
        buffer.push(LogSource::Stdout, "line 1".into(), LogLevel::Info);
        buffer.push(LogSource::Stderr, "err 0".into(), LogLevel::Error);

        assert_eq!(buffer.lines.len(), 3);
        // since_id = 0 → return lines with id > 0
        assert_eq!(buffer.get_since(0).len(), 2);
        assert_eq!(buffer.get_recent(2).len(), 2);
        assert_eq!(buffer.get_recent(100).len(), 3);
    }

    #[test]
    fn test_log_buffer_ring() {
        let mut buffer = LogBuffer::new();
        // Fill beyond capacity
        for i in 0..(DEFAULT_LOG_BUFFER + 100) {
            buffer.push(LogSource::Stdout, format!("line {}", i), LogLevel::Info);
        }
        assert_eq!(buffer.lines.len(), DEFAULT_LOG_BUFFER);
        // First line should have been evicted
        assert!(buffer.lines.front().unwrap().id > 0);
    }

    #[test]
    fn test_parse_log_level_with_pattern() {
        // Minecraft-style log pattern
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
        // No match → default Info
        assert_eq!(
            parse_log_level("Some random output", Some(&mc_pattern)),
            LogLevel::Info
        );
    }

    #[test]
    fn test_parse_log_level_without_pattern() {
        // Without pattern, everything defaults to Info
        assert_eq!(parse_log_level("[12:00:00] [Server thread/ERROR]: err", None), LogLevel::Info);
        assert_eq!(parse_log_level("Some random output", None), LogLevel::Info);
    }

    #[tokio::test]
    async fn test_managed_process_store() {
        let store = ManagedProcessStore::new();
        assert!(store.get("test").await.is_none());
    }
}
