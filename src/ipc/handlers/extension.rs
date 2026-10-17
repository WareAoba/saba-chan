//! 익스텐션 관리 API 핸들러
//!
//! GET  /api/extensions              → 익스텐션 목록
//! POST /api/extensions/:id/enable   → 활성화
//! POST /api/extensions/:id/disable  → 비활성화
//! GET  /api/extensions/:id/gui      → GUI 번들 서빙
//! GET  /api/extensions/:id/gui/styles → CSS 서빙
//! GET  /api/extensions/:id/icon     → 아이콘 (icon.png) 서빙
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

/// 실행 중인 인스턴스의 extension_data를 수집하여 참조 벡터로 반환.
/// disable / remove 에서 사용 중인 익스텐션 검사에 공통으로 사용.
async fn collect_active_ext_data(
    state: &IPCServer,
) -> Vec<(String, std::collections::HashMap<String, serde_json::Value>)> {
    let sup = state.supervisor.read().await;
    let running_ids = sup.managed_store.running_instance_ids().await;
    sup.instance_store
        .list()
        .iter()
        .filter(|inst| running_ids.contains(&inst.id))
        .map(|inst| (inst.name.clone(), inst.extension_data.clone()))
        .collect()
}

fn active_ext_refs(
    data: &[(String, std::collections::HashMap<String, serde_json::Value>)],
) -> Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> {
    data.iter().map(|(n, d)| (n.as_str(), d)).collect()
}

/// anyhow::Error 에서 ExtensionError를 추출하여 구조화된 응답을 반환.
/// ExtensionError가 아닌 경우 일반 500 INTERNAL_SERVER_ERROR.
fn extension_err_response(
    err: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Some(ext_err) = err.downcast_ref::<ExtensionError>() {
        let status = match ext_err.error_code.as_str() {
            "not_found" | "not_mounted" | "manifest_not_found" => StatusCode::NOT_FOUND,
            "dependency_missing" | "dependency_not_enabled" | "component_version_unsatisfied" => {
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
///
/// 활성화 직후 해당 익스텐션에 daemon.startup 훅이 있으면 비동기로 디스패치하여
/// npm install 등 환경 구축을 즉시 수행합니다. 재시작 없이 바로 사용 가능하도록.
pub async fn enable_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    {
        let mut mgr = state.extension_manager.write().await;
        if let Err(e) = mgr.enable(&ext_id) {
            return Err(extension_err_response(&e));
        }
    }

    // 활성화 성공 → 이 익스텐션의 daemon.startup 훅만 비동기 디스패치 (락 밖에서 실행)
    let plan = state
        .extension_manager
        .read()
        .await
        .hook_plan("daemon.startup")
        .only(&ext_id);
    let has_startup_hook = !plan.is_empty();

    if has_startup_hook {
        let init_tracker = state.extension_init_tracker.clone();
        tokio::spawn(async move {
            run_startup_hooks(plan, init_tracker).await;
        });
    }

    Ok(Json(json!({ "success": true, "id": ext_id, "startup_dispatched": has_startup_hook })))
}

/// `daemon.startup` 훅 계획을 실행하고 결과를 초기화 추적기에 기록한다.
/// 데몬 기동 시와 익스텐션 활성화 직후에 공통으로 사용한다.
pub async fn run_startup_hooks(
    plan: crate::extension::HookPlan,
    init_tracker: crate::ipc::ExtensionInitTracker,
) {
    if plan.is_empty() {
        return;
    }
    for ext_id in plan.ext_ids() {
        init_tracker.mark_started(&ext_id, "Initializing...").await;
    }
    for (ext_id, result) in plan.dispatch(json!({})).await {
        match result {
            Ok(val) => {
                let success = val.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
                let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("OK");
                if success {
                    tracing::info!("Extension '{}' startup complete: {}", ext_id, msg);
                } else {
                    let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                    tracing::warn!("Extension '{}' startup failed: {}", ext_id, err);
                }
                init_tracker.mark_finished(&ext_id, success, msg).await;
            }
            Err(e) => {
                tracing::error!("Extension '{}' startup error: {}", ext_id, e);
                init_tracker.mark_finished(&ext_id, false, &e.to_string()).await;
            }
        }
    }
}

/// POST /api/extensions/:id/disable — 익스텐션 비활성화
pub async fn disable_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // 실행 중인 인스턴스만 검사 (정지된 인스턴스는 비활성화를 차단하지 않음)
    let active_ext_data = collect_active_ext_data(&state).await;
    let refs = active_ext_refs(&active_ext_data);

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
//  매니페스트 & 버전관리 핸들러
// ══════════════════════════════════════════════════════════════

/// GET /api/extensions/manifest — 원격 매니페스트에서 가용 익스텐션 목록 페치
///
/// 응답:
/// ```json
/// {
///   "extensions": [ { "id": "...", "version": "...", ... } ],
///   "updates": [ { "id": "...", "installed_version": "...", "latest_version": "...", ... } ]
/// }
/// ```
pub async fn fetch_manifest(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    use saba_chan_updater_lib::{UpdateConfig, UpdateManager};

    // ① Relay 서명 매니페스트에서 ext-* 키 수집 (Ed25519 검증 포함)
    let cfg = UpdateConfig::default();
    let modules_dir = saba_chan_updater_lib::constants::resolve_modules_dir();
    let mut mgr = UpdateManager::new(cfg, &modules_dir.to_string_lossy());

    match mgr.fetch_remote_manifest().await {
        Ok(manifest) => {
            let remote = remote_extension_entries(&manifest);
            // ② 설치된 익스텐션 대비 업데이트 가능 목록 계산 (ExtensionManager 로컬 상태 재사용)
            let ext_mgr = state.extension_manager.read().await;
            // serde_json::Value[] 를 Vec<RemoteExtensionInfo> 로 역직렬화
            let remote_typed = typed_remote_entries(&remote);
            let updates = ext_mgr.check_updates_against(&remote_typed);
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

/// 서명 매니페스트의 `ext-*` 컴포넌트를 GUI가 쓰는 익스텐션 항목 JSON으로 변환한다.
fn remote_extension_entries(manifest: &saba_chan_updater_lib::ReleaseManifest) -> Vec<serde_json::Value> {
    manifest
        .components
        .iter()
        .filter_map(|(key, info)| {
            let id = key.strip_prefix("ext-")?;
            Some(json!({
                "id": id,
                "name": info.display_name.clone().unwrap_or_else(|| id.to_string()),
                "version": info.version,
                "description": info.description.clone().unwrap_or_default(),
                "download_url": info.url.clone().unwrap_or_default(),
                "sha256": info.sha256.clone(),
                "asset": info.asset.clone(),
                "install_dir": info.install_dir.clone(),
                "dependencies": info.dependencies.clone().unwrap_or_default(),
                "min_app_version": info.min_app_version.clone().unwrap_or_default(),
            }))
        })
        .collect()
}

fn typed_remote_entries(entries: &[serde_json::Value]) -> Vec<crate::extension::RemoteExtensionInfo> {
    entries
        .iter()
        .filter_map(|e| serde_json::from_value(e.clone()).ok())
        .collect()
}

/// POST /api/extensions/:id/install — 매니페스트에서 익스텐션 다운로드 & 설치
///
/// Request body (optional):
/// ```json
/// { "download_url": "...", "sha256": "..." }
/// ```
/// body가 없으면 매니페스트에서 URL을 조회합니다.
pub async fn install_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
    _body: Option<Json<serde_json::Value>>,
) -> Json<serde_json::Value> {
    use saba_chan_updater_lib::{Component, UpdateConfig, UpdateManager};

    // 경로 탐색 방지
    if !crate::utils::is_single_path_component(&ext_id) {
        return Json(json!({
            "success": false,
            "error": "Invalid extension ID",
            "error_code": "invalid_id",
        }));
    }

    let cfg = UpdateConfig::default();
    let modules_dir = saba_chan_updater_lib::constants::resolve_modules_dir();
    let mut mgr = UpdateManager::new(cfg, &modules_dir.to_string_lossy());

    // Relay 서명 매니페스트 → R2 다운로드 + SHA256 검증 + 압축 해제
    match mgr
        .install_component(&Component::Extension(ext_id.clone()))
        .await
    {
        Ok(_install_path) => {
            // 설치 후 디스크 재스캔으로 새 매니페스트 등록/갱신
            let mut ext_mgr = state.extension_manager.write().await;
            if let Err(e) = ext_mgr.rescan() {
                tracing::warn!("Extension '{}' installed but rescan failed: {}", ext_id, e);
            }
            Json(json!({ "success": true, "id": ext_id }))
        }
        Err(e) => Json(json!({
            "success": false,
            "error": e.to_string(),
            "error_code": "install_failed",
        })),
    }
}

/// GET /api/extensions/updates — 설치된 익스텐션 업데이트 체크 (Relay 서명 매니페스트 기반)
pub async fn check_extension_updates(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    use saba_chan_updater_lib::{UpdateConfig, UpdateManager};

    let cfg = UpdateConfig::default();
    let modules_dir = saba_chan_updater_lib::constants::resolve_modules_dir();
    let mut mgr = UpdateManager::new(cfg, &modules_dir.to_string_lossy());

    match mgr.fetch_remote_manifest().await {
        Ok(manifest) => {
            let remote = typed_remote_entries(&remote_extension_entries(&manifest));
            let ext_mgr = state.extension_manager.read().await;
            let updates = ext_mgr.check_updates_against(&remote);            let ext_mgr = state.extension_manager.read().await;
            let updates = ext_mgr.check_updates_against(&remote);
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
    let active_ext_data = collect_active_ext_data(&state).await;
    let refs = active_ext_refs(&active_ext_data);

    let mut mgr = state.extension_manager.write().await;
    match mgr.remove(&ext_id, &refs) {
        Ok(()) => Ok(Json(serde_json::json!({ "success": true, "id": ext_id }))),
        Err(e) => Err(extension_err_response(&e)),
    }
}

/// GET /api/extensions/init-status — 익스텐션 초기화 진행 상태
pub async fn extension_init_status(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    Json(state.extension_init_tracker.snapshot().await)
}

/// GET /api/extensions/:id/icon — 익스텐션 아이콘 (icon.png) 서빙
pub async fn serve_icon(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Response, StatusCode> {
    let mgr = state.extension_manager.read().await;

    let icon_path = match mgr.extension_file_path(&ext_id, "icon.png") {
        Some(p) if p.is_file() => p,
        _ => return Err(StatusCode::NOT_FOUND),
    };

    serve_static_file(&icon_path, "image/png").await
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

/// GET /api/extensions/:id/config — 익스텐션 글로벌 설정 조회
///
/// 해당 익스텐션의 `extensionConfig.json` 저장 값을 반환합니다.
pub async fn get_extension_config(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Json<serde_json::Value> {
    let mgr = state.extension_manager.read().await;
    let config = mgr.get_extension_config(&ext_id);
    Json(json!({ "config": config }))
}

/// PUT /api/extensions/:id/config — 익스텐션 글로벌 설정 저장
///
/// 요청 body: `{ "key1": value1, "key2": value2, ... }`
/// 기존 키를 머지(덮어쓰기)합니다. 키를 제거하려면 null을 전달하세요.
pub async fn save_extension_config(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let values: std::collections::HashMap<String, serde_json::Value> = match body.as_object() {
        Some(obj) => obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "error": "Request body must be a JSON object",
                })),
            ));
        }
    };

    let mut mgr = state.extension_manager.write().await;
    mgr.set_extension_config(&ext_id, values);
    let updated = mgr.get_extension_config(&ext_id);
    Ok(Json(json!({ "success": true, "config": updated })))
}
