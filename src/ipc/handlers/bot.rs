use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{BotConfig, IPCServer};

/// GET /api/config/bot - 봇 설정 조회
pub async fn get_bot_config(State(_state): State<IPCServer>) -> impl IntoResponse {
    match std::fs::read_to_string(crate::supervisor::get_discord_bot_config_path()) {
        Ok(content) => match serde_json::from_str::<BotConfig>(&content) {
            Ok(config) => (StatusCode::OK, Json(config)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to parse bot config: {}", e)
                })),
            )
                .into_response(),
        },
        Err(_) => (
            StatusCode::OK,
            Json(BotConfig {
                prefix: "!saba".to_string(),
                module_aliases: Default::default(),
                command_aliases: Default::default(),
            }),
        )
            .into_response(),
    }
}

/// PUT /api/config/bot - 봇 설정 저장
pub async fn save_bot_config(
    State(_state): State<IPCServer>,
    Json(config): Json<BotConfig>,
) -> impl IntoResponse {
    let config_path = crate::supervisor::get_discord_bot_config_path();

    // 파일 경로의 부모 디렉토리 생성
    if let Some(parent) = std::path::Path::new(&config_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to create config directory: {}", e)
                })),
            )
                .into_response();
        }
    }

    // 설정을 JSON으로 저장
    match serde_json::to_string_pretty(&config) {
        Ok(json_str) => match std::fs::write(&config_path, json_str) {
            Ok(_) => (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "Bot config saved"
                })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to write bot config: {}", e)
                })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("Failed to serialize bot config: {}", e)
            })),
        )
            .into_response(),
    }
}
