//! AI 에이전트용 통합 명령 진입점.
//!
//! 외부 AI 에이전트(MCP 래퍼 등)가 "마인크래프트 실행" 같은 자연어 의도를
//! 단일 엔드포인트로 보낼 수 있도록 하는 얇은 디스패처다. 인스턴스를
//! id 또는 이름(대소문자 무시)으로 해석한 뒤, 기존 도메인 핸들러를 그대로
//! 호출한다. 새 비즈니스 로직을 만들지 않고 기존 핸들러에 위임하여 동작이
//! 어긋나지 않도록 한다.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::super::{IPCServer, ServerStartRequest, ServerStopRequest};
use super::{command, managed, server};

/// 에이전트 통합 명령 요청 본문.
#[derive(Debug, Deserialize)]
pub struct AgentCommandRequest {
    /// 대상 인스턴스 — UUID 또는 이름(대소문자 무시).
    pub target: String,
    /// 수행할 동작. start/stop/restart/status/console/command/rcon.
    pub action: String,
    /// `command`/`rcon` 동작에서 서버에 전달할 텍스트 명령 원문.
    #[serde(default)]
    pub command: Option<String>,
    /// `stop`/`restart` 시 강제 종료 여부.
    #[serde(default)]
    pub force: bool,
}

/// 대상 문자열을 (id, name, module_name) 으로 해석.
///
/// 1) UUID 직접 일치 → 2) 이름 대소문자 무시 일치.
async fn resolve_target(state: &IPCServer, target: &str) -> Option<(String, String, String)> {
    let supervisor = state.supervisor.read().await;
    let instances = supervisor.instance_store.list();
    if let Some(inst) = instances.iter().find(|i| i.id == target) {
        return Some((inst.id.clone(), inst.name.clone(), inst.module_name.clone()));
    }
    let lower = target.to_lowercase();
    instances
        .iter()
        .find(|i| i.name.to_lowercase() == lower)
        .map(|i| (i.id.clone(), i.name.clone(), i.module_name.clone()))
}

fn not_found(target: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": format!("instance not found: {}", target),
            "error_code": "instance_not_found",
        })),
    )
        .into_response()
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": message,
            "error_code": "bad_request",
        })),
    )
        .into_response()
}

/// POST /api/agent/command — 에이전트 통합 명령 진입점.
pub async fn agent_command(
    State(state): State<IPCServer>,
    Json(req): Json<AgentCommandRequest>,
) -> impl IntoResponse {
    let (id, name, module) = match resolve_target(&state, &req.target).await {
        Some(v) => v,
        None => return not_found(&req.target),
    };

    match req.action.to_lowercase().as_str() {
        "start" => server::start_server_handler(
            axum::extract::Path(name),
            State(state.clone()),
            Json(ServerStartRequest {
                module,
                config: Value::Null,
            }),
        )
        .await
        .into_response(),
        "stop" => server::stop_server_handler(
            axum::extract::Path(name),
            State(state.clone()),
            Json(ServerStopRequest { force: req.force }),
        )
        .await
        .into_response(),
        "restart" => {
            let _ = server::stop_server_handler(
                axum::extract::Path(name.clone()),
                State(state.clone()),
                Json(ServerStopRequest { force: req.force }),
            )
            .await
            .into_response();
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            server::start_server_handler(
                axum::extract::Path(name),
                State(state.clone()),
                Json(ServerStartRequest {
                    module,
                    config: Value::Null,
                }),
            )
            .await
            .into_response()
        }
        "status" => {
            server::get_server_status(axum::extract::Path(name), State(state.clone()))
                .await
                .into_response()
        }
        "console" => managed::get_console_handler(
            axum::extract::Path(id),
            State(state.clone()),
            Query(HashMap::new()),
        )
        .await
        .into_response(),
        "command" | "exec" | "stdin" => {
            let cmd = match req.command.as_deref() {
                Some(c) if !c.is_empty() => c,
                _ => return bad_request("'command' field is required for action 'command'"),
            };
            managed::send_stdin_handler(
                axum::extract::Path(id),
                State(state.clone()),
                Json(json!({ "command": cmd })),
            )
            .await
            .into_response()
        }
        "rcon" => {
            let cmd = match req.command.as_deref() {
                Some(c) if !c.is_empty() => c,
                _ => return bad_request("'command' field is required for action 'rcon'"),
            };
            command::execute_rcon_command(
                axum::extract::Path(id),
                State(state.clone()),
                Json(json!({ "command": cmd })),
            )
            .await
            .into_response()
        }
        other => bad_request(&format!("unknown action: {}", other)),
    }
}
