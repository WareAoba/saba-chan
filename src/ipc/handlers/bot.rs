use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

/// GET /api/config/bot - 봇 설정 조회 (전체 JSON 반환)
///
/// 자가치유: bot-config의 `cloud.hostId`가 비어 있어도 페어링 토큰(`.node_token`)이
/// 살아 있으면 여전히 페어링된 상태다 (데몬은 토큰만으로 릴레이에 연결한다).
/// 과거 빈 설정 저장으로 hostId가 유실된 경우에도 재페어링 없이 복구되도록
/// 토큰에서 hostId를 도출해 응답에 주입한다.
pub async fn get_bot_config(State(state): State<IPCServer>) -> impl IntoResponse {
    let mut json = state.config_store.get_bot_config_json().await;

    let host_id_empty = json
        .pointer("/cloud/hostId")
        .and_then(Value::as_str)
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    if host_id_empty {
        let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
        if let Some(token) = crate::cloud_agent::auth::load_node_token_from(&data_dir) {
            if let Some(cloud) = json.get_mut("cloud").and_then(Value::as_object_mut) {
                cloud.insert("hostId".to_string(), json!(token.node_id));
                tracing::info!(
                    "[BotConfig] hostId가 비어 있어 .node_token에서 복원 (node_id={})",
                    token.node_id
                );
            }
        }
    }

    (StatusCode::OK, Json(json)).into_response()
}

/// PUT /api/config/bot - 봇 설정 저장 (전체 JSON 수용)
pub async fn save_bot_config(
    State(state): State<IPCServer>,
    Json(config): Json<Value>,
) -> impl IntoResponse {
    match state.config_store.set_bot_config_from_json(config).await {
        Ok(saved) => {
            // 인터페이스는 저장만 요청한다. prefix/별명을 릴레이 봇에 반영하는
            // 전파는 데몬이 책임진다 (클라우드 미연결이면 no-op).
            // module.toml 기본 별명과 병합해야 하므로 supervisor를 함께 전달한다.
            let supervisor = state.supervisor.clone();
            tokio::spawn(async move {
                super::relay::push_bot_config_to_relay(&saved, &supervisor).await;
            });
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "Bot config saved"
                })),
            ).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("{}", e)
            })),
        ).into_response(),
    }
}
