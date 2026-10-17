//! 채팅 명령 / 릴레이 에이전트 핸들러

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use super::super::IPCServer;
use crate::chat::ChatRequest;

/// POST /api/chat/command — 텍스트 명령 처리 ({text, guild_id?, user_id?} → {reply})
pub async fn chat_command(State(state): State<IPCServer>, Json(req): Json<ChatRequest>) -> impl IntoResponse {
    // 감사 로그 (A10): 요청한 클라이언트에 본문의 사용자·길드를 붙인다
    let origin = crate::audit::current().requested_by(&req);
    let reply = crate::audit::with_origin(origin, crate::chat::api::run_with_daemon_settings(&state, &req)).await;
    // `code`는 입력을 알아보지 못했을 때만 (UNKNOWN_COMMAND·INSTANCE_NOT_FOUND)
    Json(json!({ "reply": reply.text, "code": reply.code, "args": reply.args }))
}

/// GET /api/relay/agent — 에이전트 상태
pub async fn agent_status(State(state): State<IPCServer>) -> impl IntoResponse {
    Json(state.relay_agent.status())
}

/// POST /api/relay/agent/start
pub async fn agent_start(State(state): State<IPCServer>) -> impl IntoResponse {
    match state.relay_agent.start(state.clone()).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "success": true, "status": state.relay_agent.status() }))).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(json!({ "success": false, "error": e, "error_code": "node_token_missing" })),
        )
            .into_response(),
    }
}

/// POST /api/relay/agent/stop
pub async fn agent_stop(State(state): State<IPCServer>) -> impl IntoResponse {
    state.relay_agent.stop();
    Json(json!({ "success": true, "status": state.relay_agent.status() }))
}
