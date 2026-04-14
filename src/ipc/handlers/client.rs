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
    let token = state.shutdown_token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        token.cancel();
    });
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

    let client_id = state.client_registry.register(kind.clone()).await;
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

/// POST /api/client/:id/heartbeat — TTL 갱신
pub async fn client_heartbeat(
    Path(client_id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let bot_pid = payload
        .get("bot_pid")
        .and_then(|v| v.as_u64())
        .map(|p| p as u32);

    if state.client_registry.heartbeat(&client_id, bot_pid).await {
        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Client not registered"})),
        )
            .into_response()
    }
}

/// DELETE /api/client/:id/unregister — 클라이언트 명시적 해제 + 봇 정리
/// Query: ?shutdown=true → 봇 프로세스 종료 + 마지막 클라이언트 해제 후 데몬도 종료
///        ?shutdown=false → 봇 프로세스 유지, 데몬도 유지 (인터페이스만 종료)
pub async fn client_unregister(
    Path(client_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let wants_shutdown = params.get("shutdown").map(|v| v == "true").unwrap_or(false);

    if let Some(client) = state.client_registry.unregister(&client_id).await {
        // shutdown=true(완전 종료)일 때만 봇 프로세스 정리
        if wants_shutdown {
            if let Some(pid) = client.bot_pid {
                kill_bot_pid(pid);
            }
        }
        let count = state.client_registry.count().await;
        tracing::info!("[Heartbeat] Active clients after unregister: {}", count);

        // shutdown=true && 남은 클라이언트 0 → 데몬 자체 종료
        if wants_shutdown && count == 0 {
            tracing::info!("[Shutdown] Voluntary shutdown requested via unregister — initiating graceful shutdown");
            // 별도 태스크에서 짧은 딜레이 후 종료 (응답을 먼저 보내기 위해)
            let token = state.shutdown_token.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                token.cancel();
            });
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

/// 특정 PID의 봇 프로세스를 종료
pub fn kill_bot_pid(pid: u32) {
    tracing::info!("[Heartbeat] Killing bot process PID: {}", pid);

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// 백그라운드 태스크에서 호출 — 만료 클라이언트 정리 및 고아 봇 프로세스 종료
pub async fn reap_expired_clients(registry: &ClientRegistry) {
    let timeout = std::time::Duration::from_secs(90);
    let expired = registry.reap_expired(timeout).await;

    for (id, client) in &expired {
        tracing::warn!(
            "[Heartbeat] Cleaning up expired client: {} ({:?})",
            id,
            client.kind
        );
        if let Some(pid) = client.bot_pid {
            kill_bot_pid(pid);
        }
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
        })),
    )
        .into_response()
}
