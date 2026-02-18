//! 익스텐션 관리 API 핸들러
//!
//! GET  /api/extensions              → 익스텐션 목록
//! POST /api/extensions/:id/enable   → 활성화
//! POST /api/extensions/:id/disable  → 비활성화
//! GET  /api/extensions/:id/gui      → GUI 번들 서빙
//! GET  /api/extensions/:id/gui/styles → CSS 서빙
//! GET  /api/extensions/:id/i18n/:locale → i18n JSON

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::extension::ExtensionError;
use crate::ipc::IPCServer;

/// anyhow::Error 에서 ExtensionError를 추출하여 구조화된 응답을 반환.
/// ExtensionError가 아닌 경우 일반 500 INTERNAL_SERVER_ERROR.
fn extension_err_response(
    err: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Some(ext_err) = err.downcast_ref::<ExtensionError>() {
        let status = match ext_err.error_code.as_str() {
            "not_found" | "not_mounted" | "manifest_not_found" => StatusCode::NOT_FOUND,
            "dependency_missing" | "dependency_not_enabled" => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            "has_dependents" | "in_use" => StatusCode::CONFLICT,
            "id_mismatch" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(json!({
                "success": false,
                "error": ext_err.message,
                "error_code": ext_err.error_code,
                "related": ext_err.related,
            })),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error": err.to_string(),
                "error_code": "internal",
            })),
        )
    }
}

/// GET /api/extensions — 발견된 전체 익스텐션 목록
pub async fn list_extensions(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    let mgr = state.extension_manager.read().await;
    let list = mgr.list();
    Json(json!({ "extensions": list }))
}

/// POST /api/extensions/:id/enable — 익스텐션 활성화
pub async fn enable_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut mgr = state.extension_manager.write().await;
    match mgr.enable(&ext_id) {
        Ok(()) => Ok(Json(json!({ "success": true, "id": ext_id }))),
        Err(e) => Err(extension_err_response(&e)),
    }
}

/// POST /api/extensions/:id/disable — 익스텐션 비활성화
pub async fn disable_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // 인스턴스 목록에서 extension_data 추출
    let active_ext_data = {
        let sup = state.supervisor.read().await;
        sup.instance_store
            .list()
            .iter()
            .map(|inst| (inst.name.clone(), inst.extension_data.clone()))
            .collect::<Vec<_>>()
    };
    let refs: Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> =
        active_ext_data
            .iter()
            .map(|(n, d)| (n.as_str(), d))
            .collect();

    let mut mgr = state.extension_manager.write().await;
    match mgr.disable(&ext_id, &refs) {
        Ok(()) => Ok(Json(json!({ "success": true, "id": ext_id }))),
        Err(e) => Err(extension_err_response(&e)),
    }
}

/// POST /api/extensions/rescan — 런타임 중 익스텐션 디렉토리 재스캔
pub async fn rescan_extensions(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    let mut mgr = state.extension_manager.write().await;
    match mgr.rescan() {
        Ok(newly_found) => Json(json!({
            "success": true,
            "newly_found": newly_found,
        })),
        Err(e) => Json(json!({
            "success": false,
            "error": e.to_string(),
        })),
    }
}

/// POST /api/extensions/:id/mount — 익스텐션 핫 마운트
pub async fn mount_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut mgr = state.extension_manager.write().await;
    match mgr.mount(&ext_id) {
        Ok(()) => Ok(Json(json!({ "success": true, "id": ext_id }))),
        Err(e) => Err(extension_err_response(&e)),
    }
}

/// POST /api/extensions/:id/unmount — 익스텐션 핫 언마운트
pub async fn unmount_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // 인스턴스 목록에서 extension_data 추출
    let active_ext_data = {
        let sup = state.supervisor.read().await;
        sup.instance_store
            .list()
            .iter()
            .map(|inst| (inst.name.clone(), inst.extension_data.clone()))
            .collect::<Vec<_>>()
    };
    let refs: Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> =
        active_ext_data
            .iter()
            .map(|(n, d)| (n.as_str(), d))
            .collect();

    let mut mgr = state.extension_manager.write().await;
    match mgr.unmount(&ext_id, &refs) {
        Ok(()) => Ok(Json(json!({ "success": true, "id": ext_id }))),
        Err(e) => Err(extension_err_response(&e)),
    }
}

/// GET /api/extensions/:id/gui — GUI UMD 번들 서빙
pub async fn serve_gui_bundle(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Response, StatusCode> {
    let mgr = state.extension_manager.read().await;

    // GUI manifest에서 번들 경로 가져오기
    let manifests = mgr.gui_manifests();
    let gui = manifests
        .iter()
        .find(|(id, _)| *id == ext_id)
        .map(|(_, g)| g);

    let gui = match gui {
        Some(g) => g,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let bundle_path = match mgr.extension_file_path(&ext_id, &gui.bundle) {
        Some(p) => p,
        None => return Err(StatusCode::NOT_FOUND),
    };

    serve_static_file(&bundle_path, "application/javascript").await
}

/// GET /api/extensions/:id/gui/styles — CSS 서빙
pub async fn serve_gui_styles(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Response, StatusCode> {
    let mgr = state.extension_manager.read().await;

    let manifests = mgr.gui_manifests();
    let gui = manifests
        .iter()
        .find(|(id, _)| *id == ext_id)
        .map(|(_, g)| g);

    let gui = match gui {
        Some(g) => g,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let styles_path = match gui.styles.as_deref() {
        Some(s) => match mgr.extension_file_path(&ext_id, s) {
            Some(p) => p,
            None => return Err(StatusCode::NOT_FOUND),
        },
        None => return Err(StatusCode::NOT_FOUND),
    };

    serve_static_file(&styles_path, "text/css").await
}

/// GET /api/extensions/:id/i18n/:locale — i18n JSON 서빙
pub async fn serve_i18n(
    State(state): State<IPCServer>,
    Path((ext_id, locale)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mgr = state.extension_manager.read().await;
    match mgr.load_i18n(&ext_id, &locale) {
        Some(val) => Ok(Json(val)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// 파일을 읽어서 HTTP 응답으로 반환
async fn serve_static_file(
    path: &std::path::Path,
    content_type: &str,
) -> Result<Response, StatusCode> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok((
            [(header::CONTENT_TYPE, content_type.to_string())],
            bytes,
        )
            .into_response()),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
