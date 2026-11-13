//! 릴레이 에이전트 핸들러

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use super::super::IPCServer;

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
            Json(json!({ "success": false, "error": e.message, "error_code": e.code })),
        )
            .into_response(),
    }
}

/// POST /api/relay/agent/stop
pub async fn agent_stop(State(state): State<IPCServer>) -> impl IntoResponse {
    state.relay_agent.stop();
    Json(json!({ "success": true, "status": state.relay_agent.status() }))
}
