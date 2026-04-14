//! 범용 익스텐션 프로세스 매니저
//!
//! 이름 기반으로 외부 프로세스(Discord 봇, 음악 봇 등)를 관리한다.
//! API: /api/ext-process/:name/{start,stop,status,console,stdin}
//!
//! 프로세스 실행에 필요한 command, args, cwd, env는 클라이언트가 조립하여 POST body로 전달.
//! 데몬은 특정 익스텐션의 내부 로직을 모른다.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::super::IPCServer;

// ═══════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════

/// 콘솔 한 줄 (ID 포함, 폴링용)
#[derive(Clone, serde::Serialize)]
struct ConsoleEntry {
    id: u64,
    line: String,
}

/// 단일 프로세스의 런타임 상태
struct ProcessEntry {
    pid: Option<u32>,
    console_buffer: VecDeque<ConsoleEntry>,
    next_id: u64,
    stdin_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// 프로세스 시작 시 전달된 메타데이터 (클라이언트가 자유롭게 설정)
    meta: Value,
}

impl ProcessEntry {
    fn new() -> Self {
        Self {
            pid: None,
            console_buffer: VecDeque::with_capacity(4096),
            next_id: 0,
            stdin_tx: None,
            meta: Value::Null,
        }
    }

    fn push_log(&mut self, line: String) {
        if self.console_buffer.len() >= 100_000 {
            self.console_buffer.pop_front();
        }
        let entry = ConsoleEntry {
            id: self.next_id,
            line,
        };
        self.next_id += 1;
        self.console_buffer.push_back(entry);
    }

    fn is_running(&self) -> bool {
        self.pid.is_some()
    }
}

/// 이름 → 프로세스 상태 매핑 (여러 익스텐션 동시 관리)
pub struct ExtProcessManager {
    processes: HashMap<String, ProcessEntry>,
}

impl ExtProcessManager {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    /// 데몬 종료 시 모든 실행 중 프로세스에 shutdown 명령 전송 후 강제 종료.
    /// 클라이언트 등록 없이도 (daemon-only 모드) 봇 등 ext 프로세스를 정리한다.
    pub async fn shutdown_all(&mut self) {
        let running: Vec<(String, Option<u32>)> = self
            .processes
            .iter()
            .filter(|(_, e)| e.is_running())
            .map(|(name, e)| (name.clone(), e.pid))
            .collect();

        for (name, pid) in &running {
            tracing::info!("[Shutdown] Stopping ext-process '{}'", name);

            // stdin graceful shutdown 시도
            if let Some(entry) = self.processes.get(&*name) {
                if let Some(tx) = &entry.stdin_tx {
                    let _ = tx.send(
                        serde_json::to_string(&serde_json::json!({"type": "shutdown"}))
                            .unwrap_or_default(),
                    ).await;
                }
            }

            // force kill
            if let Some(pid) = pid {
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/F", "/T"])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
                #[cfg(not(windows))]
                {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
    }
}

pub type SharedExtProcessManager = Arc<Mutex<ExtProcessManager>>;

pub fn new_ext_process_manager() -> SharedExtProcessManager {
    Arc::new(Mutex::new(ExtProcessManager::new()))
}

// ═══════════════════════════════════════════════════════
// Request types
// ═══════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct StartProcessRequest {
    /// 실행할 명령어 (예: "node", "python", 절대경로 등)
    pub command: String,
    /// 명령어 인자
    #[serde(default)]
    pub args: Vec<String>,
    /// 작업 디렉토리 (없으면 데몬 cwd 사용)
    pub cwd: Option<String>,
    /// 환경변수 (기존 env에 merge)
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 클라이언트가 자유롭게 저장하는 메타데이터 (예: mode, token 등)
    #[serde(default)]
    pub meta: Value,
}

// ═══════════════════════════════════════════════════════
// Internal API (데몬 내부에서도 호출 가능)
// ═══════════════════════════════════════════════════════

/// 프로세스를 시작하고 PID를 반환합니다.
/// HTTP 핸들러와 데몬 내부 자동 시작 모두에서 사용됩니다.
pub async fn start_process_internal(
    mgr: &SharedExtProcessManager,
    name: String,
    req: StartProcessRequest,
) -> Result<u32, String> {
    let mut mgr_lock = mgr.lock().await;

    let entry = mgr_lock.processes.entry(name.clone()).or_insert_with(ProcessEntry::new);

    if entry.is_running() {
        return Err(format!("Process '{}' is already running", name));
    }

    // 프로세스 구성
    let mut cmd = Command::new(&req.command);
    cmd.args(&req.args)
        .envs(&req.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(cwd) = &req.cwd {
        cmd.current_dir(cwd);
    }

    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    match cmd.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            entry.pid = pid;
            entry.console_buffer.clear();
            entry.next_id = 0;
            entry.meta = req.meta;

            // stdin 채널
            let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<String>(64);
            if let Some(mut stdin) = child.stdin.take() {
                tokio::spawn(async move {
                    while let Some(line) = stdin_rx.recv().await {
                        if stdin.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                        if stdin.write_all(b"\n").await.is_err() {
                            break;
                        }
                    }
                });
            }
            entry.stdin_tx = Some(stdin_tx);

            // stdout 캡처
            let manager_ref = mgr.clone();
            let name_stdout = name.clone();
            if let Some(stdout) = child.stdout.take() {
                tokio::spawn(async move {
                    let reader = BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        // __IPC__: 접두사는 내부 프로토콜 메시지이므로 터미널에 표시하지 않음
                        if !line.starts_with("__IPC__:") {
                            eprintln!("[ext:{}] {}", name_stdout, line);
                        }
                        let mut mgr = manager_ref.lock().await;
                        if let Some(entry) = mgr.processes.get_mut(&name_stdout) {
                            entry.push_log(format!("[stdout] {}", line));
                        }
                    }
                });
            }

            // stderr 캡처
            let manager_ref = mgr.clone();
            let name_stderr = name.clone();
            if let Some(stderr) = child.stderr.take() {
                tokio::spawn(async move {
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        eprintln!("[ext:{}] [stderr] {}", name_stderr, line);
                        let mut mgr = manager_ref.lock().await;
                        if let Some(entry) = mgr.processes.get_mut(&name_stderr) {
                            entry.push_log(format!("[stderr] {}", line));
                        }
                    }
                });
            }

            // 프로세스 종료 감시
            let manager_ref = mgr.clone();
            let name_wait = name.clone();
            tokio::spawn(async move {
                let status = child.wait().await;
                let exit_info = match &status {
                    Ok(s) => format!("exit code: {:?}", s.code()),
                    Err(e) => format!("wait error: {}", e),
                };
                eprintln!("[ext:{}] Process exited ({})", name_wait, exit_info);
                let mut mgr = manager_ref.lock().await;
                if let Some(entry) = mgr.processes.get_mut(&name_wait) {
                    entry.pid = None;
                    entry.stdin_tx = None;
                    entry.push_log(format!("[system] Process exited ({})", exit_info));
                }
            });

            let actual_pid = pid.unwrap_or(0);
            Ok(actual_pid)
        }
        Err(e) => Err(format!("Failed to spawn process '{}': {}", name, e)),
    }
}

// ═══════════════════════════════════════════════════════
// Handlers
// ═══════════════════════════════════════════════════════

/// POST /api/ext-process/:name/start
pub async fn start_process(
    State(state): State<IPCServer>,
    Path(name): Path<String>,
    Json(req): Json<StartProcessRequest>,
) -> impl IntoResponse {
    match start_process_internal(&state.ext_process_manager, name.clone(), req).await {
        Ok(pid) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "pid": pid, "name": name })),
        )
            .into_response(),
        Err(e) if e.contains("already running") => (
            StatusCode::CONFLICT,
            Json(json!({ "error": e })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /api/ext-process/:name/stop
pub async fn stop_process(
    State(state): State<IPCServer>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let mgr = &state.ext_process_manager;
    let mut mgr_lock = mgr.lock().await;

    let entry = match mgr_lock.processes.get_mut(&name) {
        Some(e) => e,
        None => {
            return (
                StatusCode::OK,
                Json(json!({ "ok": true, "message": format!("Process '{}' not registered", name) })),
            )
                .into_response();
        }
    };

    if !entry.is_running() {
        return (
            StatusCode::OK,
            Json(json!({ "ok": true, "message": format!("Process '{}' is not running", name) })),
        )
            .into_response();
    }

    // graceful shutdown: stdin에 shutdown 명령
    if let Some(tx) = &entry.stdin_tx {
        let _ = tx
            .send(serde_json::to_string(&json!({"type": "shutdown"})).unwrap_or_default())
            .await;
    }

    // 1초 후 강제 종료
    if let Some(pid) = entry.pid {
        let manager_ref = state.ext_process_manager.clone();
        let name_kill = name.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let mgr = manager_ref.lock().await;
            if let Some(entry) = mgr.processes.get(&name_kill) {
                if entry.pid == Some(pid) {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/F", "/T"])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = std::process::Command::new("kill")
                            .args(["-9", &pid.to_string()])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                }
            }
        });
    }

    entry.push_log("[system] Stop requested".to_string());

    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// GET /api/ext-process/:name/status
pub async fn get_process_status(
    State(state): State<IPCServer>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let mgr = state.ext_process_manager.lock().await;
    match mgr.processes.get(&name) {
        Some(entry) => {
            let status = if entry.is_running() { "running" } else { "stopped" };
            (
                StatusCode::OK,
                Json(json!({
                    "name": name,
                    "status": status,
                    "pid": entry.pid,
                    "meta": entry.meta,
                })),
            )
                .into_response()
        }
        None => (
            StatusCode::OK,
            Json(json!({
                "name": name,
                "status": "unknown",
                "pid": null,
            })),
        )
            .into_response(),
    }
}

/// GET /api/ext-process/:name/console?since=0&count=200
pub async fn get_process_console(
    State(state): State<IPCServer>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let since_id = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let count = params
        .get("count")
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(200);

    let mgr = state.ext_process_manager.lock().await;
    match mgr.processes.get(&name) {
        Some(entry) => {
            let lines: Vec<&ConsoleEntry> = if let Some(since) = since_id {
                entry
                    .console_buffer
                    .iter()
                    .filter(|e| e.id >= since)
                    .collect()
            } else {
                entry
                    .console_buffer
                    .iter()
                    .rev()
                    .take(count)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            };
            (
                StatusCode::OK,
                Json(json!({
                    "name": name,
                    "lines": lines,
                    "running": entry.is_running(),
                })),
            )
                .into_response()
        }
        None => (
            StatusCode::OK,
            Json(json!({ "name": name, "lines": [], "running": false })),
        )
            .into_response(),
    }
}

/// POST /api/ext-process/:name/stdin
pub async fn send_process_stdin(
    State(state): State<IPCServer>,
    Path(name): Path<String>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mgr = state.ext_process_manager.lock().await;
    match mgr.processes.get(&name) {
        Some(entry) if entry.stdin_tx.is_some() => {
            let tx = entry.stdin_tx.as_ref().unwrap();
            match tx.send(message.to_string()).await {
                Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("Failed to send stdin: {}", e) })),
                )
                    .into_response(),
            }
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Process '{}' is not running or stdin not available", name) })),
        )
            .into_response(),
    }
}

/// GET /api/ext-processes — 전체 프로세스 목록
pub async fn list_processes(State(state): State<IPCServer>) -> impl IntoResponse {
    let mgr = state.ext_process_manager.lock().await;
    let list: Vec<Value> = mgr
        .processes
        .iter()
        .map(|(name, entry)| {
            json!({
                "name": name,
                "status": if entry.is_running() { "running" } else { "stopped" },
                "pid": entry.pid,
                "meta": entry.meta,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({ "processes": list })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_all_empty() {
        let mut mgr = ExtProcessManager::new();
        // 빈 상태에서 shutdown_all → 패닉 없이 정상 실행
        mgr.shutdown_all().await;
        assert!(mgr.processes.is_empty());
    }

    #[tokio::test]
    async fn test_shutdown_all_clears_running_processes() {
        let mut mgr = ExtProcessManager::new();
        // 가짜 stopped 프로세스 추가 (pid=None → is_running()=false)
        mgr.processes.insert("stopped-bot".to_string(), ProcessEntry::new());
        // shutdown_all은 running만 대상 → stopped는 건드리지 않음
        mgr.shutdown_all().await;
        assert!(mgr.processes.contains_key("stopped-bot"));
    }
}
