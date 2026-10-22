use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

/// GET /api/config/bot - 봇 설정 조회 (전체 JSON 반환)
pub async fn get_bot_config(State(state): State<IPCServer>) -> impl IntoResponse {
    let json = state.config_store.get_bot_config_json().await;
    (StatusCode::OK, Json(json)).into_response()
}

/// PUT /api/config/bot - 봇 설정 저장 (전체 JSON 수용)
pub async fn save_bot_config(
    State(state): State<IPCServer>,
    Json(config): Json<Value>,
) -> impl IntoResponse {
    let old_relay = state.config_store.get_bot_config().await.cloud.relay_url;
    match state.config_store.save_bot_config_json(config).await {
        Ok((saved, ignored)) => {
            // 릴레이 주소가 바뀌면 에이전트 세션을 새 주소로 다시 시작한다 (예전 세션은 예전 주소에서 멈춘다)
            if saved.cloud.relay_url.trim() != old_relay.trim() && state.relay_agent.status().running {
                crate::relay_agent::restart(&state).await;
            }
            // hostId는 노드 토큰이 기준이다: 클라이언트가 예전 값을 다시 보내도 토큰에 맞춘다
            let token = std::fs::read_to_string(super::relay::node_token_path()).ok();
            super::relay::sync_host_id(&state, token.as_deref()).await;
            let mut body = json!({ "success": true, "message": "Bot config saved" });
            // 권한 키는 권한 API로만 바뀐다: 바꾸려던 값은 저장하지 않고 알린다 (L61)
            if !ignored.is_empty() {
                body["ignored"] = json!(ignored);
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(crate::config_store::ConfigStoreError::InvalidRelayUrl(msg)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": msg, "error_code": "invalid_relay_url" })),
        )
            .into_response(),
        Err(crate::config_store::ConfigStoreError::InvalidData(msg)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": msg, "error_code": "invalid_request" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("{}", e)
            })),
        ).into_response(),
    }
}
