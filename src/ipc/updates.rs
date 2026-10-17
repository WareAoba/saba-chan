//! 데몬 내장 업데이트 API — `/api/updates/*`
//!
//! 업데이트 확인(check)과 다운로드(download)를 데몬 프로세스 내에서 수행합니다.
//! 파일 적용(apply)은 별도 업데이터 프로세스에서 처리합니다.
//!
//! ## 엔드포인트
//! - `GET  /api/updates/status`              — 캐시된 업데이트 상태 조회
//! - `POST /api/updates/check`               — 업데이트 확인 (Relay `/v1/updates/check`)
//! - `POST /api/updates/download`            — 선택 컴포넌트 다운로드
//! - `POST /api/updates/apply`               — 업데이터 exe 스폰하여 적용
//! - `GET  /api/updates/config`              — 업데이트 설정 조회
//! - `PUT  /api/updates/config`              — 업데이트 설정 변경
//!
//! 모든 라우트는 다른 `/api/*`와 같이 IPC 토큰 인증을 거친다
//! (`routes::build_router`가 인증 레이어를 이 라우터까지 덮도록 머지한다).

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post, put},
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
    /// 모듈/익스텐션 업데이트 적용 후 핫로드를 위한 참조
    pub supervisor: Option<Arc<RwLock<crate::supervisor::Supervisor>>>,
    pub extension_manager: Option<Arc<RwLock<crate::extension::ExtensionManager>>>,
}

impl UpdateState {
    /// 내장 기본값으로 업데이트 매니저 생성
    pub fn new() -> Self {
        let cfg = load_updater_config();
        let modules_dir = resolve_modules_dir();
        let mgr = UpdateManager::new(cfg, &modules_dir);
        let progress = mgr.download_progress.clone();
        let manager = Arc::new(RwLock::new(mgr));
        Self {
            manager,
            download_progress: progress,
            supervisor: None,
            extension_manager: None,
        }
    }

    /// 핫로드를 위한 supervisor/extension_manager 참조 주입
    pub fn with_hot_reload(
        mut self,
        supervisor: Arc<RwLock<crate::supervisor::Supervisor>>,
        extension_manager: Arc<RwLock<crate::extension::ExtensionManager>>,
    ) -> Self {
        self.supervisor = Some(supervisor);
        self.extension_manager = Some(extension_manager);
        self
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
        .route("/api/updates/config", get(get_config))
        .route("/api/updates/config", put(set_config))
        .with_state(state)
}

// ═══════════════════════════════════════════════════════
// 핸들러
// ═══════════════════════════════════════════════════════

/// GET /api/updates/status — 캐시된 상태 반환 (Relay 호출 없음)
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

/// POST /api/updates/check — 릴레이를 호출하여 최신 릴리스 확인
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
    let prog = state
        .download_progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
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
    // Updater → 모듈/익스텐션/Locales → CoreDaemon → 인터페이스
    targets.sort_by_key(|comp| match comp {
        Component::Updater => 0u8,
        Component::Module(_) | Component::Extension(_) | Component::Locales => 1,
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

    // 적용된 모듈/익스텐션이 있으면 핫로드: 메모리 캐시를 디스크에서 다시 로드
    let has_module_updates = targets.iter().any(|c| matches!(c, Component::Module(_)));
    let has_ext_updates = targets.iter().any(|c| matches!(c, Component::Extension(_)));

    // 업데이터 exe가 필요한 컴포넌트가 있으면:
    // 1) pending manifest 재저장 (데몬이 적용한 것들은 downloaded=false로 제외됨)
    // 2) apply-targets.json에 업데이터가 적용해야 할 정확한 컴포넌트 목록 저장
    //    (GUI가 CLI 인자로 일부만 전달해도 업데이터가 정확한 목록을 알 수 있도록)
    //
    // 영속화에 **둘 중 하나라도 실패하면** 업데이터 위임을 포기한다.
    // 부분 기록 상태에서 업데이터가 실행되면 잘못된 컴포넌트가 적용되거나
    // manifest를 로드하지 못해 no-op 종료하는 침묵 오동작이 발생한다.
    let mut persistence_ok = true;
    if !needs_updater.is_empty() {
        if let Err(e) = mgr.save_pending_manifest() {
            tracing::error!("[Updates] Failed to save pending manifest for updater: {}", e);
            errors.push(format!("save pending manifest: {}", e));
            persistence_ok = false;
        }
        if let Err(e) = mgr.save_updater_apply_targets(&needs_updater) {
            tracing::error!("[Updates] Failed to save apply targets for updater: {}", e);
            errors.push(format!("save apply targets: {}", e));
            persistence_ok = false;
        }
    }

    // UpdateManager lock 해제 (supervisor/extension_manager lock과 교차 잠금 방지)
    drop(mgr);

    // 업데이터가 필요하면 스폰 인증 티켓 발급 — 클라이언트만 토큰을 받아 업데이터를 실행할 수 있다.
    // 영속화 실패 시에는 티켓 발급도 생략한다.
    let launch_token: Option<String> = if !needs_updater.is_empty() && persistence_ok {
        match saba_chan_updater_lib::launch_ticket::issue(&["gui", "cli"]) {
            Ok(tok) => Some(tok),
            Err(e) => {
                tracing::error!("[Updates] Failed to issue launch ticket: {}", e);
                errors.push(format!("launch ticket: {}", e));
                None
            }
        }
    } else {
        None
    };

    // 업데이터 위임 실패 여부 — 토큰이 없거나 영속화 실패면 클라이언트가 스폰하지 못하도록 false.
    let requires_updater = !needs_updater.is_empty() && persistence_ok && launch_token.is_some();

    // 데몬이 직접 적용한 경우에만 핫로드 (업데이터 위임 시에는 불필요)
    if !requires_updater {
        if has_module_updates {
            if let Some(ref sup) = state.supervisor {
                let sup = sup.read().await;
                match sup.refresh_modules() {
                    Ok(modules) => {
                        tracing::info!("[Updates] Hot-reloaded {} module(s) after update", modules.len());
                    }
                    Err(e) => {
                        tracing::warn!("[Updates] Failed to hot-reload modules after update: {}", e);
                    }
                }
            }
        }

        if has_ext_updates {
            if let Some(ref ext_mgr) = state.extension_manager {
                let mut ext_mgr = ext_mgr.write().await;
                match ext_mgr.rescan() {
                    Ok(newly_found) => {
                        tracing::info!("[Updates] Hot-reloaded extensions after update ({} new)", newly_found.len());
                    }
                    Err(e) => {
                        tracing::warn!("[Updates] Failed to hot-reload extensions after update: {}", e);
                    }
                }
            }
        }
    }

    Json(json!({
        "ok": errors.is_empty(),
        "applied": applied,
        "needs_updater": needs_updater,
        "requires_updater": requires_updater,
        "launch_token": launch_token,
        "errors": errors,
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
    // 범위를 벗어난 값은 조용히 잘라 저장하지 않고 거부한다.
    let check_interval = match body.get("check_interval_hours") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) => Some(n),
            None => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({ "ok": false, "error": "check_interval_hours must be an integer between 0 and 4294967295" })),
                )
                    .into_response();
            }
        },
    };

    let mut mgr = state.manager.write().await;
    let mut cfg = mgr.get_config();

    // 부분 업데이트
    if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = body.get("relay_base_url").and_then(|v| v.as_str()) {
        cfg.relay_base_url = v.to_string();
    }
    if let Some(v) = body.get("channel").and_then(|v| v.as_str()) {
        cfg.channel = v.to_string();
    }
    if let Some(v) = body.get("platform").and_then(|v| v.as_str()) {
        cfg.platform = v.to_string();
    }
    if let Some(v) = body.get("client_id").and_then(|v| v.as_str()) {
        cfg.client_id = v.to_string();
    }
    if let Some(v) = check_interval {
        cfg.check_interval_hours = v;
    }
    if let Some(v) = body.get("auto_download").and_then(|v| v.as_bool()) {
        cfg.auto_download = v;
    }
    if let Some(v) = body.get("auto_apply").and_then(|v| v.as_bool()) {
        cfg.auto_apply = v;
    }
    if let Some(v) = body.get("install_root").and_then(|v| v.as_str()) {
        cfg.install_root = Some(v.to_string());
    }

    mgr.update_config(cfg.clone());

    Json(json!({
        "ok": true,
        "config": cfg,
    }))
    .into_response()
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


