use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{ClientKind, ClientRegistry, IPCServer};

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
pub async fn client_unregister(
    Path(client_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if let Some(client) = state.client_registry.unregister(&client_id).await {
        // 해당 클라이언트가 관리하던 봇 프로세스 정리
        if let Some(pid) = client.bot_pid {
            kill_bot_pid(pid);
        }
        let count = state.client_registry.count().await;
        tracing::info!("[Heartbeat] Active clients after unregister: {}", count);
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
