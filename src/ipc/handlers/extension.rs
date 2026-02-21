//! 익스텐션 관리 API 핸들러
//!
//! GET  /api/extensions              → 익스텐션 목록
//! POST /api/extensions/:id/enable   → 활성화
//! POST /api/extensions/:id/disable  → 비활성화
//! GET  /api/extensions/:id/gui      → GUI 번들 서빙
//! GET  /api/extensions/:id/gui/styles → CSS 서빙
//! GET  /api/extensions/:id/i18n/:locale → i18n JSON
//! DELETE /api/extensions/:id         → 제거 (비활성화 + 디렉토리 삭제)

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
    // 실행 중인 인스턴스만 검사 (정지된 인스턴스는 비활성화를 차단하지 않음)
    let active_ext_data = {
        let sup = state.supervisor.read().await;
        let running_ids = sup.managed_store.running_instance_ids().await;
        sup.instance_store
            .list()
            .iter()
            .filter(|inst| running_ids.contains(&inst.id))
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
    // 실행 중인 인스턴스만 검사
    let active_ext_data = {
        let sup = state.supervisor.read().await;
        let running_ids = sup.managed_store.running_instance_ids().await;
        sup.instance_store
            .list()
            .iter()
            .filter(|inst| running_ids.contains(&inst.id))
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

    let bundle_rel = match &gui.bundle {
        Some(b) => b,
        None => return Err(StatusCode::NOT_FOUND), // 내장 익스텐션: 번들 없음
    };

    let bundle_path = match mgr.extension_file_path(&ext_id, bundle_rel) {
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

// ══════════════════════════════════════════════════════════════
//  레지스트리 & 버전관리 핸들러
// ══════════════════════════════════════════════════════════════

/// GET /api/extensions/registry — 원격 레지스트리에서 가용 익스텐션 목록 페치
///
/// 응답:
/// ```json
/// {
///   "extensions": [ { "id": "...", "version": "...", ... } ],
///   "updates": [ { "id": "...", "installed_version": "...", "latest_version": "...", ... } ]
/// }
/// ```
pub async fn fetch_registry(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    let mgr = state.extension_manager.read().await;

    match mgr.fetch_registry().await {
        Ok(remote) => {
            // 설치된 익스텐션 중 업데이트 가능한 항목 계산
            let updates = mgr.check_updates_against(&remote);
            Json(json!({
                "success": true,
                "extensions": remote,
                "updates": updates,
            }))
        }
        Err(e) => Json(json!({
            "success": false,
            "error": e.to_string(),
            "extensions": [],
            "updates": [],
        })),
    }
}

/// POST /api/extensions/:id/install — 레지스트리에서 익스텐션 다운로드 & 설치
///
/// Request body (optional):
/// ```json
/// { "download_url": "...", "sha256": "..." }
/// ```
/// body가 없으면 레지스트리에서 URL을 조회합니다.
pub async fn install_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> Json<serde_json::Value> {
    // body 또는 레지스트리에서 download_url 결정
    let (download_url, sha256) = if let Some(Json(b)) = body {
        let url = b.get("download_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let sha = b.get("sha256")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        (url, sha)
    } else {
        (None, None)
    };

    let download_url = match download_url {
        Some(u) => u,
        None => {
            // 레지스트리에서 해당 익스텐션 URL 조회
            let mgr = state.extension_manager.read().await;
            match mgr.fetch_registry().await {
                Ok(remote) => {
                    match remote.iter().find(|r| r.id == ext_id) {
                        Some(entry) => entry.download_url.clone(),
                        None => return Json(json!({
                            "success": false,
                            "error": format!("Extension '{}' not found in registry", ext_id),
                            "error_code": "not_in_registry",
                        })),
                    }
                }
                Err(e) => return Json(json!({
                    "success": false,
                    "error": format!("Registry fetch failed: {}", e),
                    "error_code": "registry_fetch_failed",
                })),
            }
        }
    };

    let mgr = state.extension_manager.write().await;
    match mgr.install_from_url(&ext_id, &download_url, sha256.as_deref()).await {
        Ok(()) => {
            // 설치 완료 후 마운트 (write lock 재획득 불가하므로 다음 rescan에서 처리됨)
            drop(mgr);
            let mut mgr = state.extension_manager.write().await;
            let _ = mgr.mount(&ext_id);
            Json(json!({ "success": true, "id": ext_id }))
        }
        Err(e) => Json(json!({
            "success": false,
            "error": e.to_string(),
            "error_code": "install_failed",
        })),
    }
}

/// GET /api/extensions/updates — 설치된 익스텐션 업데이트 체크 (레지스트리 비교)
pub async fn check_extension_updates(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    let mgr = state.extension_manager.read().await;

    match mgr.fetch_registry().await {
        Ok(remote) => {
            let updates = mgr.check_updates_against(&remote);
            Json(json!({
                "success": true,
                "updates": updates,
                "count": updates.len(),
            }))
        }
        Err(e) => Json(json!({
            "success": false,
            "error": e.to_string(),
            "updates": [],
            "count": 0,
        })),
    }
}

/// DELETE /api/extensions/:id — 익스텐션 제거 (비활성화 + 디렉토리 삭제)
pub async fn remove_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // 실행 중인 인스턴스만 검사
    let active_ext_data = {
        let sup = state.supervisor.read().await;
        let running_ids = sup.managed_store.running_instance_ids().await;
        sup.instance_store
            .list()
            .iter()
            .filter(|inst| running_ids.contains(&inst.id))
            .map(|inst| (inst.name.clone(), inst.extension_data.clone()))
            .collect::<Vec<_>>()
    };
    let refs: Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> =
        active_ext_data
            .iter()
            .map(|(n, d)| (n.as_str(), d))
            .collect();

    let mut mgr = state.extension_manager.write().await;
    match mgr.remove(&ext_id, &refs) {
        Ok(()) => Ok(Json(serde_json::json!({ "success": true, "id": ext_id }))),
        Err(e) => Err(extension_err_response(&e)),
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
