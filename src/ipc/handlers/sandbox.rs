//! 확인되지 않은 구성요소를 격리 없이 돌리기 동의 (`config.json` `sandbox.unisolated`)
//!
//! 격리 준비가 실패하면 실행이 `action_required: sandbox_unavailable`과 `component`를 돌려준다.
//! GUI·CLI가 사용자에게 물은 뒤 이 라우트로 기록한다. 기록은 지금 파일의 지문에 묶여, 파일이 바뀌면 다시 묻는다.

use axum::{extract::Path, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

async fn result(component: String, allow: bool) -> impl IntoResponse {
    let key = component.clone();
    let saved = tokio::task::spawn_blocking(move || crate::sandbox::set_unisolated(&key, allow))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)));
    match saved {
        Ok(fingerprint) => (StatusCode::OK, Json(json!({ "success": true, "component": component, "fingerprint": fingerprint }))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Unknown component: {}", component), "error_code": "unknown_component" })),
        ),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// GET /api/sandbox/unisolated — 동의 목록
pub async fn list_unisolated() -> impl IntoResponse {
    let list = tokio::task::spawn_blocking(crate::sandbox::unisolated_list).await.unwrap_or_default();
    Json(json!({ "components": list }))
}

/// POST /api/sandbox/unisolated/:component
pub async fn allow_unisolated(Path(component): Path<String>) -> impl IntoResponse {
    result(component, true).await
}

/// DELETE /api/sandbox/unisolated/:component
pub async fn revoke_unisolated(Path(component): Path<String>) -> impl IntoResponse {
    result(component, false).await
}
