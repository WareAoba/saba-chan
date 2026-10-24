use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

/// GET /api/config/bot - 봇 설정 조회 (전체 JSON + `revision`)
pub async fn get_bot_config(State(state): State<IPCServer>) -> impl IntoResponse {
    let mut json = state.config_store.get_bot_config_json().await;
    json["revision"] = json!(crate::config_store::patch::revision(&json));
    (StatusCode::OK, Json(json)).into_response()
}

/// PUT /api/config/bot - 봇 설정 저장 (최상위 키 단위로 얹는다)
pub async fn save_bot_config(State(state): State<IPCServer>, Json(config): Json<Value>) -> impl IntoResponse {
    write_bot_config(&state, config, crate::config_store::Apply::Overlay).await
}

/// PATCH /api/config/bot - 바뀐 칸만 (JSON Merge Patch, `null`은 지우기). `revision`을 주면 그사이 바뀌었을 때 409 `settings_changed`
pub async fn patch_bot_config(State(state): State<IPCServer>, Json(config): Json<Value>) -> impl IntoResponse {
    write_bot_config(&state, config, crate::config_store::Apply::MergePatch).await
}

async fn write_bot_config(state: &IPCServer, mut config: Value, how: crate::config_store::Apply) -> axum::response::Response {
    // PUT은 예전처럼 revision을 보지 않는다 (GET 본문을 그대로 돌려보내는 클라이언트가 있다)
    let revision = crate::config_store::patch::take_revision(&mut config).filter(|_| how == crate::config_store::Apply::MergePatch);
    let old_relay = state.config_store.get_bot_config().await.cloud.relay_url;
    match state.config_store.write_bot(config, how, revision.as_deref()).await {
        Ok((saved, ignored, _)) => {
            // 릴레이 주소가 바뀌면 에이전트 세션을 새 주소로 다시 시작한다 (예전 세션은 예전 주소에서 멈춘다)
            if saved.cloud.relay_url.trim() != old_relay.trim() && state.relay_agent.status().running {
                crate::relay_agent::restart(state).await;
            }
            // hostId는 노드 토큰이 기준이다: 클라이언트가 예전 값을 다시 보내도 토큰에 맞춘다
            let token = super::relay::load_node_token();
            super::relay::sync_host_id(state, token.as_deref()).await;
            // hostId를 토큰에 맞춘 뒤의 값
            let revision = state.config_store.bot_revision().await;
            let mut body = json!({ "success": true, "message": "Bot config saved", "revision": revision });
            // 권한 키는 권한 API로만 바뀐다: 바꾸려던 값은 저장하지 않고 알린다 (L61)
            if !ignored.is_empty() {
                body["ignored"] = json!(ignored);
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(crate::config_store::ConfigStoreError::RevisionConflict(revision)) => (
            StatusCode::CONFLICT,
            Json(json!({ "success": false, "error": "The bot settings were changed elsewhere; read them again", "error_code": "settings_changed", "revision": revision })),
        )
            .into_response(),
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
