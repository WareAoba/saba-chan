//! 데몬 내장 업데이트 API — `/api/updates/*`
//!
//! 업데이트 확인(check)과 다운로드(download)를 데몬 프로세스 내에서 수행합니다.
//! 파일 적용(apply)은 별도 업데이터 프로세스에서 처리합니다.
//!
//! ## 엔드포인트
//! - `GET  /api/updates/status`              — 캐시된 업데이트 상태 조회
//! - `POST /api/updates/check`               — 업데이트 확인 (GitHub API 호출)
//! - `POST /api/updates/download`            — 선택 컴포넌트 다운로드
//! - `POST /api/updates/apply`               — 업데이터 exe 스폰하여 적용
//! - `GET  /api/updates/config`              — 업데이트 설정 조회
//! - `PUT  /api/updates/config`              — 업데이트 설정 변경

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

use saba_chan_updater_lib::{
    Component, DownloadProgress, UpdateConfig, UpdateManager,
};

// ═══════════════════════════════════════════════════════
// 공유 상태
// ═══════════════════════════════════════════════════════

/// 데몬 업데이트 매니저 — Arc<RwLock>으로 공유
#[derive(Clone)]
pub struct UpdateState {
    pub manager: Arc<RwLock<UpdateManager>>,
    /// 다운로드 진행률 (Manager 잠금 없이 폴링 가능)
    pub download_progress: Arc<std::sync::Mutex<DownloadProgress>>,
}

impl UpdateState {
    /// 내장 기본값으로 업데이트 매니저 생성
    pub fn new() -> Self {
        let cfg = load_updater_config();
        let modules_dir = resolve_modules_dir();
        let mgr = UpdateManager::new(cfg, &modules_dir);
        let progress = mgr.download_progress.clone();
        let manager = Arc::new(RwLock::new(mgr));
        Self { manager, download_progress: progress }
    }
}

impl Default for UpdateState {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════
// 라우터
// ═══════════════════════════════════════════════════════

/// `/api/updates/*` 라우트를 포함하는 axum Router 생성
pub fn updates_router(state: UpdateState) -> Router {
    Router::new()
        .route("/api/updates/status", get(get_status))
        .route("/api/updates/check", post(check_updates))
        .route("/api/updates/download", post(download_components))
        .route("/api/updates/download/progress", get(get_download_progress))
        .route("/api/updates/apply", post(apply_updates))
        .route("/api/updates/integrity", get(check_integrity))
        .route("/api/updates/config", get(get_config))
        .route("/api/updates/config", post(set_config))
        .with_state(state)
}

// ═══════════════════════════════════════════════════════
// 핸들러
// ═══════════════════════════════════════════════════════

/// GET /api/updates/status — 캐시된 상태 반환 (GitHub API 호출 없음)
///
/// Locales 컴포넌트는 사용자에게 비표시 — 백그라운드 자동 다운로드/적용 대상이므로
/// 응답의 `components`, `updates_available` 에서 제외합니다.
async fn get_status(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let status = mgr.get_status();

    // Locales는 UI에 표시하지 않음 — 백그라운드 자동 적용 대상
    let components: Vec<Value> = status.components.iter()
        .filter(|c| !matches!(c.component, Component::Locales))
        .map(|c| {
            json!({
                "component": c.component.manifest_key(),
                "display_name": c.component.display_name(),
                "current_version": c.current_version,
                "latest_version": c.latest_version,
                "update_available": c.update_available,
                "downloaded": c.downloaded,
                "installed": c.installed,
            })
        }).collect();

    let visible_update_count = status.components.iter()
        .filter(|c| c.update_available && !matches!(c.component, Component::Locales))
        .count();

    Json(json!({
        "ok": true,
        "last_check": status.last_check,
        "next_check": status.next_check,
        "checking": status.checking,
        "error": status.error,
        "updates_available": visible_update_count,
        "components": components,
    }))
}

/// POST /api/updates/check — GitHub API를 호출하여 최신 릴리스 확인
///
/// Locales 컴포넌트는 응답에서 제외하고, 업데이트가 있으면
/// 백그라운드에서 자동 다운로드+적용합니다 (사용자 비표시).
async fn check_updates(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let result = {
        let mut mgr = state.manager.write().await;
        mgr.check_for_updates().await
    };

    match result {
        Ok(status) => {
            // ── Locales 자동 적용: 사용자 비표시 ──
            // Locales는 작고 재시작이 불필요하므로, 체크 직후 백그라운드에서 자동 다운로드+적용
            let has_locale_update = status.components.iter()
                .any(|c| matches!(c.component, Component::Locales) && c.update_available);
            if has_locale_update {
                let mgr_clone = state.manager.clone();
                tokio::spawn(async move {
                    silent_apply_locales(&mgr_clone).await;
                });
            }

            // Locales를 제외한 "사용자에게 보이는" 컴포넌트 목록
            let components: Vec<Value> = status.components.iter()
                .filter(|c| !matches!(c.component, Component::Locales))
                .map(|c| {
                    json!({
                        "component": c.component.manifest_key(),
                        "display_name": c.component.display_name(),
                        "current_version": c.current_version,
                        "latest_version": c.latest_version,
                        "update_available": c.update_available,
                        "downloaded": c.downloaded,
                        "installed": c.installed,
                        "release_notes": c.release_notes,
                    })
                }).collect();

            let update_names: Vec<String> = status.components.iter()
                .filter(|c| c.update_available && !matches!(c.component, Component::Locales))
                .map(|c| c.component.display_name())
                .collect();

            Json(json!({
                "ok": true,
                "updates_available": update_names.len(),
                "update_names": update_names,
                "components": components,
                "last_check": status.last_check,
            }))
        }
        Err(e) => {
            Json(json!({
                "ok": false,
                "error": e.to_string(),
            }))
        }
    }
}

/// Locales 컴포넌트를 사용자 비표시로 다운로드+적용합니다.
///
/// 실패 시에도 에러만 로깅 — 사용자 흐름에 영향 없음.
async fn silent_apply_locales(manager: &Arc<RwLock<UpdateManager>>) {
    // 1. 다운로드
    let download_ok = {
        let mut mgr = manager.write().await;
        match mgr.download_component(&Component::Locales).await {
            Ok(_) => {
                tracing::info!("[Updates] Locales silently downloaded");
                true
            }
            Err(e) => {
                tracing::warn!("[Updates] Silent locales download failed: {}", e);
                false
            }
        }
    };

    if !download_ok {
        return;
    }

    // 2. 적용
    let mut mgr = manager.write().await;
    match mgr.apply_single_component(&Component::Locales).await {
        Ok(result) if result.success => {
            tracing::info!("[Updates] Locales silently applied");
        }
        Ok(result) => {
            tracing::warn!("[Updates] Silent locales apply returned failure: {}", result.message);
        }
        Err(e) => {
            tracing::warn!("[Updates] Silent locales apply failed: {}", e);
        }
    }
}

/// POST /api/updates/download — 선택한 컴포넌트 다운로드
///
/// Body: `{ "components": ["gui", "cli", "module-minecraft"] }`
/// 비어있으면 업데이트 가능한 전체를 다운로드
#[derive(Deserialize)]
struct DownloadRequest {
    /// 다운로드할 컴포넌트 키 목록. 비어있으면 전체.
    #[serde(default)]
    components: Vec<String>,
}

async fn download_components(
    State(state): State<UpdateState>,
    Json(body): Json<DownloadRequest>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;

    // 아직 체크하지 않았으면 먼저 체크
    if mgr.get_status().components.is_empty() {
        if let Err(e) = mgr.check_for_updates().await {
            return Json(json!({
                "ok": false,
                "error": format!("Auto-check failed: {}", e),
            }));
        }
    }

    if body.components.is_empty() {
        // 전체 다운로드
        match mgr.download_available_updates().await {
            Ok(downloaded) => {
                // 업데이터 --apply 모드를 위해 매니페스트 저장
                if let Err(e) = mgr.save_pending_manifest() {
                    tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
                }
                Json(json!({
                    "ok": true,
                    "downloaded": downloaded,
                    "count": downloaded.len(),
                }))
            }
            Err(e) => {
                Json(json!({
                    "ok": false,
                    "error": e.to_string(),
                }))
            }
        }
    } else {
        // 선택 컴포넌트만 다운로드
        let mut downloaded = Vec::new();
        let mut errors = Vec::new();

        for key in &body.components {
            let component = Component::from_manifest_key(key);
            match mgr.download_component(&component).await {
                Ok(asset) => {
                    downloaded.push(json!({
                        "component": key,
                        "asset": asset,
                    }));
                }
                Err(e) => {
                    errors.push(json!({
                        "component": key,
                        "error": e.to_string(),
                    }));
                }
            }
        }

        // 업데이터 --apply 모드를 위해 매니페스트 저장
        if !downloaded.is_empty() {
            if let Err(e) = mgr.save_pending_manifest() {
                tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
            }
        }

        Json(json!({
            "ok": errors.is_empty(),
            "downloaded": downloaded,
            "count": downloaded.len(),
            "errors": errors,
        }))
    }
}

/// GET /api/updates/download/progress — 현재 다운로드 진행률 조회
///
/// Manager의 RwLock과 독립된 `std::sync::Mutex`를 사용하므로
/// 다운로드 중에도 블로킹 없이 폴링 가능.
async fn get_download_progress(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let prog = state.download_progress.lock().unwrap().clone();
    Json(json!({
        "ok": true,
        "component": prog.component,
        "bytes_received": prog.bytes_received,
        "total_bytes": prog.total_bytes,
        "active": prog.active,
    }))
}

/// POST /api/updates/apply — 다운로드된 업데이트 적용
///
/// - 모듈: 데몬이 직접 적용 (파일 교체)
/// - 데몬/GUI/CLI: 업데이터 exe를 스폰하여 적용 (응답에 requires_updater: true)
///
/// Body: `{ "components": ["module-minecraft", "saba-core"] }` (선택 적용, 비어있으면 전체)
#[derive(Deserialize)]
struct ApplyRequest {
    #[serde(default)]
    components: Vec<String>,
}

async fn apply_updates(
    State(state): State<UpdateState>,
    Json(body): Json<ApplyRequest>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;

    // 적용 대상 분류
    let pending = mgr.get_pending_components();
    let mut targets: Vec<Component> = if body.components.is_empty() {
        pending.iter().map(|c| c.component.clone()).collect()
    } else {
        body.components.iter()
            .map(|k| Component::from_manifest_key(k))
            .filter(|c| pending.iter().any(|p| p.component == *c))
            .collect()
    };

    // 적용 우선순위에 따라 정렬:
    // Updater → 모듈/익스텐션/Locales → DiscordBot → CoreDaemon → 인터페이스
    targets.sort_by_key(|comp| match comp {
        Component::Updater => 0u8,
        Component::Module(_) | Component::Extension(_) | Component::Locales => 1,
        Component::DiscordBot => 2,
        Component::CoreDaemon => 3,
        Component::Gui => 4,
        Component::Cli => 4,
    });

    let mut applied = Vec::new();
    let mut errors = Vec::new();
    let mut needs_updater: Vec<String> = Vec::new();

    // GUI/Updater/CoreDaemon이 포함되면 GUI+데몬이 종료되어야 하므로,
    // 데몬이 일부만 적용하다 중간에 죽는 문제를 방지하기 위해
    // **모든** 컴포넌트를 업데이터 exe에 위임한다.
    // 업데이터가 프로세스 종료 후 올바른 순서로 일괄 적용.
    let any_needs_restart = targets.iter().any(|c| matches!(c,
        Component::Gui | Component::Updater | Component::CoreDaemon
    ));

    if any_needs_restart {
        // 전부 업데이터에 위임 — 데몬은 아무것도 직접 적용하지 않음
        needs_updater = targets.iter().map(|c| c.manifest_key()).collect();
    } else {
        // GUI/데몬 재시작 불필요 → 데몬이 직접 적용
        for comp in &targets {
            match mgr.apply_single_component(comp).await {
                Ok(result) if result.success => {
                    applied.push(comp.display_name());
                }
                Ok(result) => {
                    errors.push(format!("{}: {}", comp.display_name(), result.message));
                }
                Err(e) => {
                    errors.push(format!("{}: {}", comp.display_name(), e));
                }
            }
        }
    }

    // 업데이터 exe가 필요한 컴포넌트가 있으면:
    // 1) pending manifest 재저장 (데몬이 적용한 것들은 downloaded=false로 제외됨)
    // 2) apply-targets.json에 업데이터가 적용해야 할 정확한 컴포넌트 목록 저장
    //    (GUI가 CLI 인자로 일부만 전달해도 업데이터가 정확한 목록을 알 수 있도록)
    if !needs_updater.is_empty() {
        if let Err(e) = mgr.save_pending_manifest() {
            tracing::warn!("[Updates] Failed to save pending manifest for updater: {}", e);
        }
        if let Err(e) = mgr.save_updater_apply_targets(&needs_updater) {
            tracing::warn!("[Updates] Failed to save apply targets for updater: {}", e);
        }
    }

    Json(json!({
        "ok": errors.is_empty(),
        "applied": applied,
        "needs_updater": needs_updater,
        "requires_updater": !needs_updater.is_empty(),
        "errors": errors,
    }))
}

/// GET /api/updates/integrity — 서버에서 매니페스트를 가져와 SHA256 무결성 검증
async fn check_integrity(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;

    let report = match mgr.verify_integrity().await {
        Ok(r) => r,
        Err(e) => {
            return Json(json!({
                "ok": false,
                "error": format!("Integrity check failed: {}", e),
            }));
        }
    };

    let components: Vec<Value> = report.components.iter().map(|c| {
        json!({
            "component": c.component,
            "display_name": c.display_name,
            "status": format!("{:?}", c.status),
            "expected_hash": c.expected_hash,
            "actual_hash": c.actual_hash,
            "file_path": c.file_path,
            "message": c.message,
        })
    }).collect();

    Json(json!({
        "ok": true,
        "checked_at": report.checked_at,
        "overall": format!("{:?}", report.overall),
        "total": report.total,
        "verified": report.verified,
        "failed": report.failed,
        "skipped": report.skipped,
        "components": components,
    }))
}

/// GET /api/updates/config
async fn get_config(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let config = mgr.get_config();
    Json(json!({
        "ok": true,
        "config": config,
    }))
}

/// PUT /api/updates/config
async fn set_config(
    State(state): State<UpdateState>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;
    let mut cfg = mgr.get_config();

    // 부분 업데이트
    if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = body.get("github_owner").and_then(|v| v.as_str()) {
        cfg.github_owner = v.to_string();
    }
    if let Some(v) = body.get("github_repo").and_then(|v| v.as_str()) {
        cfg.github_repo = v.to_string();
    }
    if let Some(v) = body.get("check_interval_hours").and_then(|v| v.as_u64()) {
        cfg.check_interval_hours = v as u32;
    }
    if let Some(v) = body.get("auto_download").and_then(|v| v.as_bool()) {
        cfg.auto_download = v;
    }
    if let Some(v) = body.get("auto_apply").and_then(|v| v.as_bool()) {
        cfg.auto_apply = v;
    }
    if let Some(v) = body.get("include_prerelease").and_then(|v| v.as_bool()) {
        cfg.include_prerelease = v;
    }
    if let Some(v) = body.get("install_root").and_then(|v| v.as_str()) {
        cfg.install_root = Some(v.to_string());
    }
    if let Some(v) = body.get("api_base_url").and_then(|v| v.as_str()) {
        cfg.api_base_url = Some(v.to_string());
    }

    mgr.update_config(cfg.clone());

    Json(json!({
        "ok": true,
        "config": cfg,
    }))
}

// ═══════════════════════════════════════════════════════
// 설정 유틸리티
// ═══════════════════════════════════════════════════════

/// 업데이터 설정 로드 — 하드코딩 기본값 사용
fn load_updater_config() -> UpdateConfig {
    UpdateConfig::default()
}

fn resolve_modules_dir() -> String {
    let dir = saba_chan_updater_lib::constants::resolve_modules_dir();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir.to_string_lossy().to_string()
}


