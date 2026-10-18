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
    match state.config_store.set_bot_config_from_json(config).await {
        Ok(_) => {
            // hostId는 노드 토큰이 기준이다: 클라이언트가 예전 값을 다시 보내도 토큰에 맞춘다
            let token_path = saba_chan_updater_lib::constants::resolve_data_dir().join(".node_token");
            let token = std::fs::read_to_string(token_path).ok();
            super::relay::sync_host_id(&state, token.as_deref()).await;
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "Bot config saved"
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("{}", e)
            })),
        ).into_response(),
    }
}
