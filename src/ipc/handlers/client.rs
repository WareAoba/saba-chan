use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{ClientKind, ClientRegistry, IPCServer};

/// POST /api/daemon/shutdown — 데몬 강제 종료 (CLI 등 외부에서 호출)
pub async fn daemon_shutdown(
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    tracing::info!("[Shutdown] Daemon shutdown requested via API");
    state.request_shutdown();
    (StatusCode::OK, Json(json!({"ok": true, "message": "Shutdown initiated"}))).into_response()
}

/// POST /api/client/register — 클라이언트(GUI/CLI) 등록
pub async fn client_register(
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let kind_str = payload
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("gui");
    let kind = match kind_str {
        "cli" => ClientKind::Cli,
        _ => ClientKind::Gui,
    };

    let pid = payload.get("pid").and_then(|v| v.as_u64()).and_then(|p| u32::try_from(p).ok());
    let install_root = payload
        .get("installRoot")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let client_id = state.client_registry.register(kind.clone(), pid, install_root).await;
    let count = state.client_registry.count().await;
    tracing::info!("[Heartbeat] Active clients: {}", count);

    (
        StatusCode::OK,
        Json(json!({
            "client_id": client_id,
            "kind": kind_str,
            "heartbeat_interval_ms": 30000,
            "timeout_ms": 90000
        })),
    )
        .into_response()
}

/// GET /api/clients — 지금 등록된 클라이언트. 인스톨러가 이 설치의 GUI(임시 폴더에 풀린 포터블 GUI 포함)를 찾는다
pub async fn client_list(State(state): State<IPCServer>) -> impl IntoResponse {
    let clients: Vec<_> = state
        .client_registry
        .list()
        .await
        .into_iter()
        .map(|(id, c)| json!({"id": id, "kind": c.kind, "pid": c.pid, "installRoot": c.install_root}))
        .collect();
    Json(json!({"clients": clients}))
}

/// POST /api/client/:id/heartbeat — TTL 갱신 (본문은 무시)
pub async fn client_heartbeat(
    Path(client_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if state.client_registry.heartbeat(&client_id).await {
        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Client not registered"})),
        )
            .into_response()
    }
}

/// DELETE /api/client/:id/unregister — 클라이언트 명시적 해제
/// Query: ?shutdown=true → 마지막 클라이언트 해제 후 데몬도 종료
///        ?shutdown=false → 데몬 유지 (인터페이스만 종료)
pub async fn client_unregister(
    Path(client_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let wants_shutdown = params.get("shutdown").map(|v| v == "true").unwrap_or(false);

    if state.client_registry.unregister(&client_id).await.is_some() {
        let count = state.client_registry.count().await;
        tracing::info!("[Heartbeat] Active clients after unregister: {}", count);

        // shutdown=true && 남은 클라이언트 0 → 데몬 자체 종료
        if wants_shutdown && count == 0 {
            tracing::info!("[Shutdown] Voluntary shutdown requested via unregister — initiating graceful shutdown");
            state.request_shutdown();
        }

        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Client not registered"})),
        )
            .into_response()
    }
}

/// 백그라운드 태스크에서 호출 — 만료 클라이언트 정리
pub async fn reap_expired_clients(registry: &ClientRegistry) {
    let timeout = std::time::Duration::from_secs(90);
    let expired = registry.reap_expired(timeout).await;

    for (id, client) in &expired {
        tracing::warn!(
            "[Heartbeat] Cleaning up expired client: {} ({:?})",
            id,
            client.kind
        );
    }

    if !expired.is_empty() {
        let remaining = registry.count().await;
        tracing::info!(
            "[Heartbeat] Reap complete. Cleaned: {}, remaining clients: {}",
            expired.len(),
            remaining
        );
    }
}

/// GET /api/daemon/console?since=0&count=200 — 데몬 자체 tracing 로그 조회
pub async fn daemon_console(
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let since_id = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let count = params.get("count").and_then(|c| c.parse::<usize>().ok());

    let lines = state.daemon_log_buffer.get_since(since_id, count);

    (
        StatusCode::OK,
        Json(json!({
            "lines": lines,
            "count": lines.len(),
            "epoch": state.daemon_log_buffer.epoch(),
        })),
    )
        .into_response()
}

/// `POST /api/clients/launch` 본문에서 CLI 실행 인자를 만든다 (I4 "터미널로 열기").
/// 실행 줄은 여기서 정한 모양뿐이다: 본문 문자열을 인자로 그대로 넘기지 않는다. 틀리면 `(field, reason)`
/// CLI 인자와 그 줄이 가리키는 인스턴스 id
type LaunchLine = (Vec<String>, Option<String>);

fn launch_args(body: &serde_json::Value) -> Result<LaunchLine, (&'static str, &'static str)> {
    match body.get("kind").and_then(|v| v.as_str()) {
        Some("cli") => {}
        Some(_) => return Err(("kind", "only \"cli\" can be launched")),
        None => return Err(("kind", "required")),
    }
    let instance = match body.get("instance") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(v.as_str().filter(|s| !s.is_empty()).ok_or(("instance", "must be an instance id"))?.to_string()),
    };
    match (body.get("view").and_then(|v| v.as_str()), instance) {
        // `--` 뒤라 id가 옵션으로 읽히지 않는다
        (Some("console"), Some(id)) => Ok((vec!["console".into(), "--".into(), id.clone()], Some(id))),
        (Some("console"), None) => Err(("instance", "required for the console view")),
        (Some("shell"), None) => Ok((Vec::new(), None)),
        (Some("shell"), Some(_)) => Err(("instance", "the shell view takes no instance")),
        _ => Err(("view", "must be \"console\" or \"shell\"")),
    }
}

/// POST /api/clients/launch — 새 터미널 창에 CLI를 띄운다 (I4). 로컬 전용: 액션 카탈로그에 없다
pub async fn client_launch(State(state): State<IPCServer>, Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let (args, instance) = match launch_args(&body) {
        Ok(v) => v,
        Err((field, reason)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "error": format!("Invalid input '{}': {}", field, reason), "error_code": "invalid_input", "field": field, "reason": reason })),
            )
                .into_response()
        }
    };
    if let Some(id) = &instance {
        if state.supervisor.read().await.instance_store.get(id).is_none() {
            return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response();
        }
    }
    if !cfg!(windows) {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({ "success": false, "error": "Opening a terminal is only supported on Windows for now", "error_code": "unsupported_platform" })),
        )
            .into_response();
    }
    let Some(cli) = crate::boot_selector::find_interface_binary("saba-chan-cli") else {
        return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": "saba-chan-cli was not found next to the daemon", "error_code": "cli_not_found" }))).into_response();
    };
    match spawn_in_new_console(&cli, &args) {
        Ok(pid) => {
            tracing::info!("[Launch] Opened CLI {:?} (pid {})", args, pid);
            (StatusCode::OK, Json(json!({ "success": true, "pid": pid }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": format!("Could not start saba-chan-cli: {}", e), "error_code": "launch_failed" }))).into_response(),
    }
}

#[cfg(windows)]
fn spawn_in_new_console(cli: &std::path::Path, args: &[String]) -> std::io::Result<u32> {
    crate::boot_selector::spawn_cli_in_new_console(cli, args)
}

#[cfg(not(windows))]
fn spawn_in_new_console(_cli: &std::path::Path, _args: &[String]) -> std::io::Result<u32> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Windows only"))
}

#[cfg(test)]
mod launch_tests {
    use super::launch_args;
    use serde_json::json;

    #[test]
    fn only_fixed_command_lines_are_built() {
        let console = launch_args(&json!({ "kind": "cli", "view": "console", "instance": "abc" })).unwrap();
        assert_eq!(console, (vec!["console".to_string(), "--".to_string(), "abc".to_string()], Some("abc".to_string())));
        assert_eq!(launch_args(&json!({ "kind": "cli", "view": "shell" })).unwrap(), (vec![], None));

        let field = |b| launch_args(&b).unwrap_err().0;
        assert_eq!(field(json!({ "kind": "gui", "view": "shell" })), "kind");
        assert_eq!(field(json!({ "view": "shell" })), "kind");
        assert_eq!(field(json!({ "kind": "cli", "view": "console" })), "instance");
        assert_eq!(field(json!({ "kind": "cli", "view": "console", "instance": 5 })), "instance");
        assert_eq!(field(json!({ "kind": "cli", "view": "shell", "instance": "abc" })), "instance");
        assert_eq!(field(json!({ "kind": "cli", "view": "run", "instance": "abc" })), "view");
    }
}

#[cfg(test)]
mod console_tests {
    use super::*;

    /// 데몬 로그 번호는 데몬을 켤 때마다 1부터 다시 센다: 답에 버퍼 세대 `epoch`를 실어,
    /// 이어 읽던 클라이언트가 바뀐 것을 알고 번호를 버리게 한다 (L55)
    #[tokio::test]
    async fn the_daemon_console_carries_an_epoch_that_changes_with_the_buffer() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let read = |buffer: crate::daemon_log::DaemonLogBuffer| {
            let state = IPCServer::new(sup.clone(), "127.0.0.1:0", buffer);
            async move {
                let res = daemon_console(State(state), axum::extract::Query(Default::default())).await.into_response();
                crate::ipc::internal::into_internal(res).await.body
            }
        };
        let first = read(crate::daemon_log::DaemonLogBuffer::new()).await;
        let again = read(crate::daemon_log::DaemonLogBuffer::new()).await;
        let (a, b) = (first["epoch"].as_u64().expect("epoch"), again["epoch"].as_u64().expect("epoch"));
        assert!(b > a, "a restarted daemon's buffer has a newer epoch: {a} then {b}");
    }
}
