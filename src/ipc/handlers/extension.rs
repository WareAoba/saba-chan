//! 익스텐션 관리 API 핸들러
//!
//! GET  /api/extensions              → 익스텐션 목록
//! POST /api/extensions/:id/enable   → 활성화
//! POST /api/extensions/:id/disable  → 비활성화
//! GET  /api/extensions/:id/gui      → GUI 번들 서빙 (확인된 것이거나 사용자가 허락한 것만)
//! GET  /api/extensions/:id/gui/styles → CSS 서빙 (번들과 같은 조건)
//! GET  /api/extensions/:id/icon     → 아이콘 (icon.png) 서빙
//! GET  /api/extensions/:id/audit    → 코드가 쓰는 위험한 능력과 manifest `permissions` 비교
//! GET  /api/extensions/:id/i18n/:locale → i18n JSON
//! DELETE /api/extensions/:id         → 제거 (비활성화 + 디렉토리 삭제)

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::extension::{ExtensionError, ExtensionManager};
use crate::ipc::IPCServer;

/// anyhow::Error 에서 ExtensionError를 추출하여 구조화된 응답을 반환.
/// ExtensionError가 아닌 경우 일반 500 INTERNAL_SERVER_ERROR.
fn extension_err_response(
    err: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Some(ext_err) = err.downcast_ref::<ExtensionError>() {
        let status = match ext_err.error_code.as_str() {
            "not_found" | "not_mounted" | "manifest_not_found" => StatusCode::NOT_FOUND,
            "dependency_missing" | "dependency_not_enabled" | "component_version_unsatisfied"
            | "app_version_unsatisfied" => {
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

#[derive(serde::Deserialize, Default)]
pub struct LangQuery {
    #[serde(default)]
    lang: Option<String>,
}

/// GET /api/extensions?lang= — 발견된 전체 익스텐션 목록. 설명은 `lang`(없으면 GUI 언어)으로
pub async fn list_extensions(
    State(state): State<IPCServer>,
    axum::extract::Query(q): axum::extract::Query<LangQuery>,
) -> Json<serde_json::Value> {
    let language = match q.lang.filter(|l| !l.is_empty()) {
        Some(l) => l,
        None => state.config_store.ui_language().await,
    };
    let mgr = state.extension_manager.read().await;
    let mut list = mgr.list();
    // 설명은 GUI 언어로 (없으면 "ko-KR" → "ko", 그래도 없으면 manifest의 영어 description)
    let base = language.split(['-', '_']).next().unwrap_or("").to_string();
    for item in &mut list {
        if let Some(text) = item.descriptions.get(&language).or_else(|| item.descriptions.get(&base)) {
            item.description = text.clone();
        }
    }
    let failed = mgr.load_errors();
    if failed.is_empty() {
        Json(json!({ "extensions": list }))
    } else {
        Json(json!({ "extensions": list, "failed": failed }))
    }
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

    // 활성화 성공 → daemon.startup 훅이 있으면 비동기 디스패치 (npm install 등)
    let has_startup_hook = {
        let mgr = state.extension_manager.read().await;
        mgr.hooks_for("daemon.startup")
            .iter()
            .any(|(ext, _)| ext.manifest.id == ext_id)
    };

    if has_startup_hook {
        let ext_mgr = state.extension_manager.clone();
        let init_tracker = state.extension_init_tracker.clone();
        let ext_id_clone = ext_id.clone();
        tokio::spawn(async move {
            init_tracker.mark_started(&ext_id_clone, "Initializing...").await;
            // 방금 켠 익스텐션에만 보낸다 (다른 익스텐션의 startup을 다시 돌리지 않게)
            let result = crate::extension::run_hook_to(&ext_mgr, &ext_id_clone, "daemon.startup", serde_json::json!({})).await;
            let rid = ext_id_clone;
            match result {
                Some(Ok(val)) => {
                    let success = val.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
                    let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("OK");
                    if success {
                        tracing::info!("Extension '{}' post-enable startup complete: {}", rid, msg);
                    } else {
                        let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                        tracing::warn!("Extension '{}' post-enable startup failed: {}", rid, err);
                    }
                    init_tracker.mark_finished(&rid, success, msg).await;
                }
                Some(Err(e)) => {
                    tracing::error!("Extension '{}' post-enable startup error: {}", rid, e);
                    init_tracker.mark_finished(&rid, false, &e.to_string()).await;
                }
                // 조건이 맞지 않아 실행하지 않았다
                None => init_tracker.mark_finished(&rid, true, "OK").await,
            }
        });
    }

    // 미검증·변경된 익스텐션을 켜면 알림만 싣는다 (실행은 막지 않는다)
    let trust = state.extension_manager.read().await.discovered(&ext_id).map(|ext| {
        saba_chan_updater_lib::integrity::trust_of(&format!("ext-{}", ext_id), &ext.dir)
    });
    let warnings = trust.map(|t| super::server::trust_warnings("extension", &ext_id, t)).unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "id": ext_id,
        "startup_dispatched": has_startup_hook,
        "trust": trust,
        "warnings": warnings,
    })))
}

/// POST /api/extensions/:id/disable — 익스텐션 비활성화
pub async fn disable_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // 실행 중인 인스턴스만 검사 (정지된 인스턴스는 비활성화를 차단하지 않음)
    let active_ext_data = instance_ext_data(&state, true).await;
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

    let dir = mgr.discovered(&ext_id).map(|ext| ext.dir.clone()).ok_or(StatusCode::NOT_FOUND)?;
    let bundle_path = gui_file(&dir, bundle_rel).ok_or(StatusCode::NOT_FOUND)?;
    drop(mgr);
    if let Some(refused) = unapproved_gui(&ext_id, dir).await {
        return Ok(refused);
    }

    serve_static_file(&bundle_path, "application/javascript").await
}

/// manifest의 GUI 파일 경로를 익스텐션 폴더 안의 실제 파일로 푼다. 허락은 폴더 안 파일의 지문에 묶이므로
/// 폴더 밖(`..`·절대 경로)이나 링크(파일이든 거쳐 가는 폴더든)를 가리키면 서빙하지 않는다
fn gui_file(dir: &std::path::Path, relative: &str) -> Option<std::path::PathBuf> {
    let rel = std::path::Path::new(relative);
    if relative.is_empty() || !rel.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    let mut path = dir.to_path_buf();
    for part in rel.components() {
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
    }
    std::fs::symlink_metadata(&path).ok()?.is_file().then_some(path)
}

/// 번들은 GUI 웹뷰에서 데몬 API 전체를 부를 수 있다. 확인된 익스텐션이 아니면 사용자가 이 PC에서
/// 지금 파일 그대로 허락했을 때만 서빙한다. 아니면 403 `extension_gui_unapproved`
async fn unapproved_gui(ext_id: &str, dir: std::path::PathBuf) -> Option<Response> {
    let key = format!("{}{}", saba_chan_updater_lib::relay::EXTENSION_PREFIX, ext_id);
    let check = key.clone();
    let allowed = tokio::task::spawn_blocking(move || crate::extension::gui_allowed(&check, &dir)).await.unwrap_or(false);
    if allowed {
        return None;
    }
    let body = json!({
        "error": format!("The GUI of unverified extension '{}' needs your approval on this PC", ext_id),
        "error_code": "extension_gui_unapproved",
        "component": key,
    });
    Some((StatusCode::FORBIDDEN, Json(body)).into_response())
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

    let dir = mgr.discovered(&ext_id).map(|ext| ext.dir.clone()).ok_or(StatusCode::NOT_FOUND)?;
    let styles_path = gui.styles.as_deref().and_then(|s| gui_file(&dir, s)).ok_or(StatusCode::NOT_FOUND)?;
    drop(mgr);
    if let Some(refused) = unapproved_gui(&ext_id, dir).await {
        return Ok(refused);
    }

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
    // 네트워크 요청은 매니저 락 밖에서 (C12)
    match ExtensionManager::fetch_remote().await {
        Ok(remote) => {
            // 설치된 익스텐션 중 업데이트 가능한 항목 계산
            let updates = state.extension_manager.read().await.check_updates_against(&remote);
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

/// 인스턴스 이름과 extension_data (`running_only`면 실행 중인 것만)
async fn instance_ext_data(state: &IPCServer, running_only: bool) -> Vec<(String, std::collections::HashMap<String, serde_json::Value>)> {
    let sup = state.supervisor.read().await;
    let running_ids = sup.managed_store.running_instance_ids().await;
    sup.instance_store
        .list()
        .iter()
        .filter(|inst| !running_only || running_ids.contains(&inst.id))
        .map(|inst| (inst.name.clone(), inst.extension_data.clone()))
        .collect()
}

/// 받을 주소와 sha256을 정한다. 주소를 주지 않으면 서명된 카탈로그의 것을 쓴다.
/// 주소만 주면 카탈로그의 주소와 같을 때 카탈로그 해시를 쓰고, 다르면 해시 없이 받지 않는다 (`sha256_required`).
/// 주소와 해시를 함께 주면 그대로 쓴다 (카탈로그와 다르면 공식 설치로 기록하지 않는다).
fn install_source(
    id: &str,
    url: Option<String>,
    sha: Option<String>,
    remote: &anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>>,
) -> Result<(String, Option<String>), (String, &'static str)> {
    let entry = remote.as_ref().ok().and_then(|r| r.iter().find(|e| e.id == id));
    let sha = sha.filter(|s| !s.is_empty());
    match (url.filter(|u| !u.is_empty()), sha) {
        (Some(url), Some(sha)) => Ok((url, Some(sha))),
        (Some(url), None) => match entry.filter(|e| e.download_url == url).and_then(|e| e.sha256.clone()) {
            Some(sha) => Ok((url, Some(sha))),
            None => Err((format!("Extension '{}': a download_url outside the catalog needs a sha256", id), "sha256_required")),
        },
        (None, _) => match (entry, remote) {
            (Some(e), _) => Ok((e.download_url.clone(), e.sha256.clone())),
            (None, Ok(_)) => Err((format!("Extension '{}' not found in manifest", id), "not_in_manifest")),
            (None, Err(e)) => Err((format!("Manifest fetch failed: {}", e), "manifest_fetch_failed")),
        },
    }
}

/// POST /api/extensions/:id/install — 익스텐션 설치·업데이트 (`{download_url?, sha256?}`, `install_source` 참고)
///
/// 받기·검사·압축 해제는 락 없이 하고, 폴더 교체만 write 락 안에서 한다.
/// 실행 중인 인스턴스가 쓰는 익스텐션은 교체하지 않는다. 의존 익스텐션이 없으면 원격 목록에서 같이 설치한다.
pub async fn install_extension(
    State(state): State<IPCServer>,
    Path(ext_id): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> Json<serde_json::Value> {
    let field = |k: &str| body.as_ref().and_then(|Json(b)| b.get(k)).and_then(|v| v.as_str()).map(str::to_string);
    let (explicit_url, explicit_sha) = (field("download_url"), field("sha256"));

    let extensions_dir = state.extension_manager.read().await.extensions_dir().to_path_buf();
    let remote = ExtensionManager::fetch_remote().await;
    let fail = |error: String, code: &str| Json(json!({ "success": false, "error": error, "error_code": code }));

    let mut queue = vec![ext_id.clone()];
    let mut installed: Vec<String> = Vec::new();
    while let Some(id) = queue.pop() {
        if installed.contains(&id) {
            continue;
        }
        let entry = remote.as_ref().ok().and_then(|r| r.iter().find(|e| e.id == id).cloned());
        let explicit = if id == ext_id { (explicit_url.clone(), explicit_sha.clone()) } else { (None, None) };
        let (url, sha) = match install_source(&id, explicit.0, explicit.1, &remote) {
            Ok(source) => source,
            Err((error, code)) => return fail(error, code),
        };

        // 카탈로그가 설치된 것보다 낮은 버전을 주면 받지 않는다
        if let Some(e) = entry.as_ref().filter(|e| e.download_url == url) {
            let installed = state.extension_manager.read().await.discovered(&id).map(|d| d.manifest.version.clone());
            if let Some(installed) = installed.filter(|v| saba_chan_updater_lib::version::is_downgrade(v, &e.version)) {
                return fail(format!("Extension '{}' {} is older than the installed {}", id, e.version, installed), "older_than_installed");
            }
        }

        let staged = match ExtensionManager::download_package(&url).await.and_then(|bytes| {
            ExtensionManager::stage_package(&extensions_dir, &id, &bytes, sha.as_deref())
        }) {
            Ok(p) => p,
            Err(e) => return fail(format!("{:#}", e), "install_failed"),
        };

        let running = instance_ext_data(&state, true).await;
        let refs: Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> =
            running.iter().map(|(n, d)| (n.as_str(), d)).collect();
        let mut mgr = state.extension_manager.write().await;
        // 카탈로그의 주소와 해시로 받았을 때만 공식 설치다
        let official = entry.as_ref().is_some_and(|e| e.download_url == url && e.sha256.is_some() && e.sha256 == sha);
        if let Err(e) = mgr.install_staged(&id, &staged, &refs, official) {
            let (_, Json(body)) = extension_err_response(&e);
            if body.get("error_code").and_then(|c| c.as_str()) == Some("internal") {
                return fail(e.to_string(), "install_failed");
            }
            return Json(body);
        }
        installed.push(id.clone());
        if let Ok(r) = &remote {
            queue.extend(mgr.missing_dependencies(&id, r));
        }
    }
    Json(json!({ "success": true, "id": ext_id, "installed": installed }))
}

/// GET /api/extensions/updates — 설치된 익스텐션 업데이트 체크 (매니페스트 비교)
pub async fn check_extension_updates(
    State(state): State<IPCServer>,
) -> Json<serde_json::Value> {
    match ExtensionManager::fetch_remote().await {
        Ok(remote) => {
            let updates = state.extension_manager.read().await.check_updates_against(&remote);
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
    // 지우면 되돌릴 수 없으므로 정지된 인스턴스도 검사한다 (disable은 실행 중인 것만)
    let active_ext_data = instance_ext_data(&state, false).await;
    let refs: Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> =
        active_ext_data
            .iter()
            .map(|(n, d)| (n.as_str(), d))
            .collect();

    let mut mgr = state.extension_manager.write().await;
    match mgr.remove(&ext_id, &refs) {
        Ok(()) => {
            let _ = saba_chan_updater_lib::integrity::set_official(&format!("ext-{}", ext_id), false);
            Ok(Json(serde_json::json!({ "success": true, "id": ext_id })))
        }
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

/// GET /api/extensions/:id/audit — `GET /api/module/:name/audit`와 같은 모양
pub async fn audit_extension(State(state): State<IPCServer>, Path(ext_id): Path<String>) -> Response {
    let found = {
        let mgr = state.extension_manager.read().await;
        mgr.discovered(&ext_id).map(|e| (e.dir.clone(), e.manifest.permissions.clone()))
    };
    let Some((dir, declared)) = found else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Extension '{}' not found", ext_id), "error_code": "not_found" }))).into_response();
    };
    let scanned = match crate::python_env::get_python_path().await {
        Ok(python) => crate::module_audit::scan(&python, &dir).await,
        Err(e) => Err(e),
    };
    match scanned {
        Ok(findings) => {
            let undeclared = crate::module_audit::undeclared(&declared, &findings);
            Json(json!({ "success": true, "declared": declared, "findings": findings, "undeclared": undeclared })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{:#}", e), "error_code": "audit_failed" }))).into_response(),
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
    if mgr.discovered(&ext_id).is_none() {
        return Err(extension_err_response(&ExtensionError::not_found(&ext_id).into()));
    }
    mgr.set_extension_config(&ext_id, values);
    let updated = mgr.get_extension_config(&ext_id);
    Ok(Json(json!({ "success": true, "config": updated })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[test]
    fn an_install_url_without_a_hash_takes_the_catalog_hash_or_is_refused() {
        let remote: anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>> = Ok(serde_json::from_value(json!([
            {"id": "box", "name": "B", "version": "0.2.0", "download_url": "https://cdn/box.zip", "sha256": "ab"}
        ])).unwrap());
        let s = |v: &str| Some(v.to_string());
        let source = |url: Option<String>, sha: Option<String>| install_source("box", url, sha, &remote).map_err(|(_, code)| code);
        assert_eq!(source(None, None), Ok(("https://cdn/box.zip".into(), s("ab"))));
        assert_eq!(source(s("https://cdn/box.zip"), None), Ok(("https://cdn/box.zip".into(), s("ab"))));
        assert_eq!(source(s("https://cdn/box.zip"), s("")), Ok(("https://cdn/box.zip".into(), s("ab"))));
        assert_eq!(source(s("https://elsewhere/box.zip"), s("cd")), Ok(("https://elsewhere/box.zip".into(), s("cd"))));
        assert_eq!(source(s("https://elsewhere/box.zip"), None), Err("sha256_required"));
        let offline: anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>> = Err(anyhow::anyhow!("offline"));
        assert_eq!(install_source("box", s("https://cdn/box.zip"), None, &offline).map_err(|(_, c)| c), Err("sha256_required"));
        assert_eq!(install_source("nope", None, None, &remote).map_err(|(_, c)| c), Err("not_in_manifest"));
    }

    // GUI #282 리뷰: 설치되지 않은 id로 설정을 저장하면 받아서 남겼다 (다른 익스텐션 경로처럼 없다고 답한다)
    #[tokio::test]
    async fn config_for_an_extension_that_is_not_installed_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let ghost = "not-installed-ext-282".to_string();
        let err = save_extension_config(State(state.clone()), Path(ghost.clone()), Json(json!({ "k": 1 }))).await.unwrap_err();
        assert_eq!((err.0, err.1["error_code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
        assert_eq!(state.extension_manager.read().await.get_extension_config(&ghost), Default::default(), "nothing is saved");
    }

    #[tokio::test]
    async fn an_unverified_extension_gui_is_served_only_after_local_approval() {
        let tmp = tempfile::tempdir().unwrap();
        // 동의는 데몬 익스텐션 폴더의 구성요소에 기록된다: 그 아래에 겹치지 않는 이름으로 만든다
        let ext = tempfile::Builder::new().prefix("guitrust").tempdir_in(crate::plugin::resolve_extensions_dir()).unwrap();
        let id = ext.path().file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            ext.path().join("manifest.json"),
            json!({ "id": id, "name": "t", "version": "0.1.0", "gui": { "bundle": "gui.js", "styles": "gui.css" } }).to_string(),
        )
        .unwrap();
        std::fs::write(ext.path().join("gui.js"), "window.x = 1").unwrap();
        std::fs::write(ext.path().join("gui.css"), "body{}").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        {
            let mut mgr = state.extension_manager.write().await;
            mgr.discover().unwrap();
            mgr.enable(&id).unwrap();
        }
        let key = format!("ext-{}", id);
        let status = |r: Result<Response, StatusCode>| r.map(|r| r.status()).unwrap_or_else(|s| s);
        let listed = |state: IPCServer, id: String| async move {
            let mgr = state.extension_manager.read().await;
            mgr.list().into_iter().find(|e| e.id == id).and_then(|e| e.gui_allowed)
        };

        assert_eq!(status(serve_gui_bundle(State(state.clone()), Path(id.clone())).await), StatusCode::FORBIDDEN);
        assert_eq!(status(serve_gui_styles(State(state.clone()), Path(id.clone())).await), StatusCode::FORBIDDEN);
        assert_eq!(listed(state.clone(), id.clone()).await, Some(false));

        // 원격에서는 허락할 수 없다
        let remote = crate::audit::with_origin(
            crate::audit::Origin::new("discord"),
            super::super::sandbox::allow_extension_gui(Path(key.clone())),
        )
        .await
        .into_response();
        assert_eq!(remote.status(), StatusCode::FORBIDDEN);

        let ok = super::super::sandbox::allow_extension_gui(Path(key.clone())).await.into_response();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(status(serve_gui_bundle(State(state.clone()), Path(id.clone())).await), StatusCode::OK);
        assert_eq!(status(serve_gui_styles(State(state.clone()), Path(id.clone())).await), StatusCode::OK);
        assert_eq!(listed(state.clone(), id.clone()).await, Some(true));

        // 파일이 바뀌면 다시 묻는다
        std::fs::write(ext.path().join("gui.js"), "window.x = 2").unwrap();
        assert_eq!(status(serve_gui_bundle(State(state.clone()), Path(id.clone())).await), StatusCode::FORBIDDEN);
        let _ = super::super::sandbox::revoke_extension_gui(Path(key)).await;
    }

    #[test]
    fn gui_files_must_be_plain_files_inside_the_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("ext");
        std::fs::create_dir_all(dir.join("gui")).unwrap();
        std::fs::write(dir.join("gui").join("a.js"), "x").unwrap();
        std::fs::write(tmp.path().join("outside.js"), "x").unwrap();
        assert_eq!(gui_file(&dir, "gui/a.js"), Some(dir.join("gui").join("a.js")));
        let outside = tmp.path().join("outside.js");
        for bad in ["../outside.js", "gui/../../outside.js", "", "gui", "missing.js", outside.to_str().unwrap()] {
            assert_eq!(gui_file(&dir, bad), None, "{}", bad);
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join("outside.js"), dir.join("link.js")).unwrap();
            std::os::unix::fs::symlink(tmp.path(), dir.join("linked")).unwrap();
            assert_eq!(gui_file(&dir, "link.js"), None);
            assert_eq!(gui_file(&dir, "linked/outside.js"), None);
        }
    }
}
