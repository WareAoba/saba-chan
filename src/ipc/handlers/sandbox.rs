//! 확인되지 않은 구성요소에 대한 사용자 동의 (`config.json` `sandbox.<종류>`)
//!
//! - 격리 없이 돌리기(`unisolated`): 격리 준비가 실패하면 실행이 `action_required: sandbox_unavailable`과 `component`를 돌려준다.
//! - 익스텐션 GUI 번들 싣기(`extension-gui`): 확인되지 않은 익스텐션의 번들은 동의 전에는 서빙하지 않는다(403 `extension_gui_unapproved`).
//!
//! GUI·CLI가 사용자에게 물은 뒤 이 라우트로 기록한다. 기록은 지금 파일의 지문에 묶여, 파일이 바뀌면 다시 묻는다.
//! 동의는 이 PC에서만 받는다 (원격 요청은 403 `consent_local_only`).

use axum::{extract::Path, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::sandbox::Consent;

async fn result(kind: Consent, component: String, allow: bool) -> (StatusCode, Json<serde_json::Value>) {
    if crate::audit::current().source == "discord" {
        let error = json!({ "success": false, "error": "This can only be allowed on this PC", "error_code": "consent_local_only" });
        return (StatusCode::FORBIDDEN, Json(error));
    }
    let key = component.clone();
    let saved = tokio::task::spawn_blocking(move || crate::sandbox::set_consent(kind, &key, allow))
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

async fn list(kind: Consent) -> Json<serde_json::Value> {
    let list = tokio::task::spawn_blocking(move || crate::sandbox::consent_list(kind)).await.unwrap_or_default();
    Json(json!({ "components": list }))
}

/// GET /api/sandbox/unisolated — 동의 목록
pub async fn list_unisolated() -> impl IntoResponse {
    list(Consent::Unisolated).await
}

/// POST /api/sandbox/unisolated/:component
pub async fn allow_unisolated(Path(component): Path<String>) -> impl IntoResponse {
    result(Consent::Unisolated, component, true).await
}

/// DELETE /api/sandbox/unisolated/:component
pub async fn revoke_unisolated(Path(component): Path<String>) -> impl IntoResponse {
    result(Consent::Unisolated, component, false).await
}

/// GET /api/sandbox/extension-gui — GUI 번들 싣기 동의 목록
pub async fn list_extension_gui() -> impl IntoResponse {
    list(Consent::ExtensionGui).await
}

/// POST /api/sandbox/extension-gui/:component (`ext-<id>`)
pub async fn allow_extension_gui(Path(component): Path<String>) -> impl IntoResponse {
    result(Consent::ExtensionGui, component, true).await
}

/// DELETE /api/sandbox/extension-gui/:component
pub async fn revoke_extension_gui(Path(component): Path<String>) -> impl IntoResponse {
    result(Consent::ExtensionGui, component, false).await
}
