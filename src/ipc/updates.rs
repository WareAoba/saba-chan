//! 데몬 내장 업데이트 API — `/api/updates/*`
//!
//! 업데이트 확인(check)과 다운로드(download)를 데몬 프로세스 내에서 수행합니다.
//! 파일 적용(apply)은 별도 업데이터 프로세스에서 처리합니다.
//!
//! 주기 확인은 데몬이 한다(U6, `spawn_update_scheduler`): `check_interval_hours`마다 확인하고,
//! `auto_download`이면 내려받아 둔다. GUI·CLI는 `/api/updates/status`를 읽어 표시만 한다.
//!
//! ## 엔드포인트
//! - `GET  /api/updates/status`              — 캐시된 업데이트 상태 조회
//! - `POST /api/updates/check`               — 업데이트 확인 (릴레이의 서명된 manifest)
//! - `POST /api/updates/download`            — 선택 컴포넌트 다운로드 (본문 없으면 전부)
//! - `POST /api/updates/apply`               — 모듈 등은 데몬이 바로 적용, 재시작이 필요한 것은
//!   `requires_updater`로 돌려주고 실제 적용은 `launch-updater`가 한다 (본문 없으면 전부)
//! - `POST /api/updates/launch-updater`      — 데몬이 업데이터를 띄우고 스스로 종료 (U7)
//! - `POST /api/updates/install`             — 확인 → 전부 받기 → 적용 → 필요하면 업데이터 띄우기를 한 번에 (액션 `update.apply`, 디스코드 `/사바쨩 업데이트`)
//! - `GET  /api/updates/last-result`         — 마지막 업데이트 적용 결과 (DELETE로 확인 처리)
//! - `GET  /api/updates/config`              — 업데이트 설정 조회
//! - `PUT  /api/updates/config`              — 업데이트 설정 변경

use super::handlers::LooseJson;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use saba_chan_updater_lib::config_file::ConfigFile;
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
    /// 업데이터를 띄운 뒤 데몬을 끝내는 신호 (IPCServer::stop_requested)
    pub stop_requested: Option<tokio_util::sync::CancellationToken>,
    /// 업데이터를 띄울 때 끌 GUI를 찾는 클라이언트 등록부 (IPCServer::client_registry)
    pub clients: Option<crate::ipc::ClientRegistry>,
    /// 업데이터를 이미 띄웠는가. 다시 띄우면 표를 새로 끊어 먼저 띄운 업데이터가 적용하지 못한다
    updater_launched: Arc<std::sync::atomic::AtomicBool>,
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
            stop_requested: None,
            clients: None,
            updater_launched: Arc::default(),
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

    pub fn with_stop_signal(mut self, stop_requested: tokio_util::sync::CancellationToken) -> Self {
        self.stop_requested = Some(stop_requested);
        self
    }

    /// 업데이터를 띄웠다 (데몬이 곧 끝나고 업데이터가 실행 파일을 바꾼다)
    pub fn updater_launched(&self) -> bool {
        self.updater_launched.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn with_clients(mut self, clients: crate::ipc::ClientRegistry) -> Self {
        self.clients = Some(clients);
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
        .route("/api/updates/launch-updater", post(launch_updater))
        .route("/api/updates/install", post(install_updates))
        .route("/api/updates/last-result", get(get_last_result).delete(clear_last_result))
        .route("/api/updates/integrity", get(check_integrity))
        .route("/api/updates/config", get(get_config))
        .route("/api/updates/config", put(set_config))
        .with_state(state)
}

// ═══════════════════════════════════════════════════════
// 핸들러
// ═══════════════════════════════════════════════════════

/// GET /api/updates/status — 캐시된 상태 반환 (네트워크 없음)
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
        "profile": saba_chan_updater_lib::profile::Profile::load().as_str(),
    }))
}

/// POST /api/updates/check — 릴레이에서 최신 릴리즈 확인
///
/// Locales 컴포넌트는 응답에서 제외하고, 업데이트가 있으면
/// 백그라운드에서 자동 다운로드+적용합니다 (사용자 비표시).
async fn check_updates(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    match check_and_follow_up(&state).await {
        Ok(status) => {

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

/// 확인 한 번과 그 뒤처리. 수동 확인(`POST /check`)과 주기 확인이 같은 길을 쓴다.
/// - Locales 업데이트는 사용자에게 보이지 않게 바로 적용
/// - `auto_download`이면 나머지 업데이트를 내려받아 두고 pending manifest 저장
///   (적용은 언제나 사용자가 고른다)
async fn check_and_follow_up(state: &UpdateState) -> anyhow::Result<saba_chan_updater_lib::UpdateStatus> {
    let (status, auto_download) = {
        let mut mgr = state.manager.write().await;
        let status = mgr.check_for_updates().await?;
        (status, mgr.get_config().auto_download)
    };
    announce_new_updates(&status);

    let has_locale_update = status.components.iter()
        .any(|c| matches!(c.component, Component::Locales) && c.update_available);
    if has_locale_update {
        let mgr_clone = state.manager.clone();
        tokio::spawn(async move {
            silent_apply_locales(&mgr_clone).await;
        });
    }

    let has_visible_update = status.components.iter()
        .any(|c| c.update_available && !c.downloaded && !matches!(c.component, Component::Locales));
    if auto_download && has_visible_update {
        let manager = state.manager.clone();
        tokio::spawn(async move {
            let mut mgr = manager.write().await;
            match mgr.download_available_updates().await {
                Ok(downloaded) => {
                    tracing::info!("[Updates] Auto-downloaded {} update(s)", downloaded.len());
                    if let Err(e) = mgr.save_pending_manifest() {
                        tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("[Updates] Auto-download failed: {}", e);
                    // 실패 전에 받은 것은 적용할 수 있게 남긴다
                    if let Err(e) = mgr.save_pending_manifest() {
                        tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
                    }
                }
            }
        });
    }
    Ok(status)
}

/// 데몬 기동 후 업데이트를 주기적으로 확인한다 (U6).
/// 설정은 매 회 다시 읽는다. 꺼져 있으면(`enabled=false`) 확인하지 않고 설정만 다시 본다.
pub fn spawn_update_scheduler(state: UpdateState, shutdown: tokio_util::sync::CancellationToken) {
    /// 기동 직후에는 다른 초기화가 먼저 끝나도록 잠깐 기다린다
    const FIRST_CHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(30);
    /// 꺼져 있거나 실패했을 때 다시 볼 간격
    const RETRY: std::time::Duration = std::time::Duration::from_secs(15 * 60);

    tokio::spawn(async move {
        let mut wait = FIRST_CHECK_DELAY;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(wait) => {}
            }
            let cfg = state.manager.read().await.get_config();
            if !cfg.enabled {
                wait = RETRY;
                continue;
            }
            wait = match check_and_follow_up(&state).await {
                Ok(_) => std::time::Duration::from_secs(u64::from(cfg.check_interval_hours.max(1)) * 3600),
                Err(e) => {
                    tracing::warn!("[Updates] Scheduled check failed: {}", e);
                    RETRY
                }
            };
        }
    });
}

/// 사용자에게 보이는 업데이트가 지난번 알린 것과 달라졌을 때만 `update.available` (P4)
fn announce_new_updates(status: &saba_chan_updater_lib::UpdateStatus) {
    // 같은 구성요소라도 더 새 버전이 나오면 다시 알린다
    static LAST: std::sync::Mutex<Vec<(String, Option<String>)>> = std::sync::Mutex::new(Vec::new());
    let visible: Vec<_> = status.components.iter()
        .filter(|c| c.update_available && !matches!(c.component, Component::Locales))
        .collect();
    let found: Vec<(String, Option<String>)> = visible.iter()
        .map(|c| (c.component.manifest_key(), c.latest_version.clone()))
        .collect();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if !found.is_empty() && *last != found {
        // `/api/updates/check`의 `update_names`와 같은 표시 이름
        let names: Vec<String> = visible.iter().map(|c| c.component.display_name()).collect();
        crate::events::emit("update.available", json!({ "components": names }));
    }
    *last = found;
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
/// 본문이 없거나 비면 기본값 (CLI는 본문 없이 POST한다). 있는데 깨졌으면 400:
/// 잘못된 본문을 "전부"로 읽어 모든 컴포넌트를 건드리지 않게
fn body_or_default<T: serde::de::DeserializeOwned + Default>(bytes: &[u8]) -> Result<T, String> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON body: {}", e))
}

fn bad_body(error: String) -> axum::response::Response {
    (axum::http::StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": error, "error_code": "invalid_body" }))).into_response()
}

#[derive(Deserialize, Default)]
struct DownloadRequest {
    /// 다운로드할 컴포넌트 키 목록. 비어있으면 전체.
    #[serde(default)]
    components: Vec<String>,
}

async fn download_components(State(state): State<UpdateState>, raw: axum::body::Bytes) -> axum::response::Response {
    match body_or_default::<DownloadRequest>(&raw) {
        Ok(body) => download_components_with(state, body).await.into_response(),
        Err(e) => bad_body(e),
    }
}

async fn download_components_with(state: UpdateState, body: DownloadRequest) -> impl IntoResponse {
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
        // 전체 다운로드. 하나가 실패해도 받은 것은 적용할 수 있게 남기고, 선택 다운로드처럼 받은 것과 실패한 것을 함께 알린다
        match mgr.download_available_updates_report().await {
            Ok((downloaded, failed)) => {
                // 업데이터 --apply 모드를 위해 매니페스트 저장
                if let Err(e) = mgr.save_pending_manifest() {
                    tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
                }
                let errors: Vec<Value> = failed.iter().map(|(k, e)| json!({ "component": k, "error": e })).collect();
                Json(json!({
                    "ok": errors.is_empty(),
                    "downloaded": downloaded,
                    "count": downloaded.len(),
                    "errors": errors,
                }))
            }
            Err(e) => Json(json!({
                "ok": false,
                "error": e.to_string(),
            })),
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
    let prog = state.download_progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
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
/// - 데몬/GUI/CLI·업데이터: 적용하지 않고 `requires_updater: true`로 돌려준다. 실제 적용은 `POST /api/updates/launch-updater`
///
/// Body: `{ "components": ["module-minecraft", "saba-core"] }` (선택 적용, 비어있으면 전체)
#[derive(Deserialize, Default)]
struct ApplyRequest {
    #[serde(default)]
    components: Vec<String>,
}

async fn apply_updates(State(state): State<UpdateState>, raw: axum::body::Bytes) -> axum::response::Response {
    match body_or_default::<ApplyRequest>(&raw) {
        Ok(body) => apply_updates_with(state, body).await.into_response(),
        Err(e) => bad_body(e),
    }
}

async fn apply_updates_with(state: UpdateState, body: ApplyRequest) -> impl IntoResponse {
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

    // 업데이터와 같은 적용 순서: Updater → CoreDaemon → 모듈/익스텐션/Locales → 인터페이스, 같은 묶음의 requires가 먼저
    let order = mgr.apply_order(&targets.iter().map(|c| c.manifest_key()).collect::<Vec<_>>());
    targets.sort_by_key(|c| order.iter().position(|k| *k == c.manifest_key()));

    let mut applied = Vec::new();
    let mut applied_keys = Vec::new();
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
            // 같은 모듈을 받아 설치·제거하는 요청과 겹치면 서로의 파일을 지운다 (`module_install_guard`)
            let _installing = match comp {
                Component::Module(name) => match crate::ipc::handlers::server::module_install_guard(name) {
                    Some(guard) => Some(guard),
                    None => {
                        errors.push(format!("{}: Module '{}' is already being installed or removed", comp.display_name(), name));
                        continue;
                    }
                },
                _ => None,
            };
            match mgr.apply_single_component(comp).await {
                Ok(result) if result.success => {
                    applied.push(comp.display_name());
                    applied_keys.push(comp.manifest_key());
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
    // 둘 중 하나라도 못 쓰면 업데이터에 넘기지 않는다: 목록 없이 뜬 업데이터는 받아 둔 것을 모두 적용한다
    if !needs_updater.is_empty() {
        let saved = mgr.save_pending_manifest().and_then(|_| mgr.save_updater_apply_targets(&needs_updater));
        if let Err(e) = saved {
            tracing::warn!("[Updates] Cannot hand the update to the updater: {}", e);
            errors.push(format!("Could not prepare the update for the updater: {}", e));
            needs_updater.clear();
        }
    }

    // 데몬이 바로 적용했으면 결과와 패치노트를 남긴다 (업데이터에 넘기면 업데이터가 남긴다). CLI만으로 적용해도 `update last`로 본다
    if needs_updater.is_empty() && !targets.is_empty() {
        record_daemon_apply(&saba_chan_updater_lib::UpdateCompletionMarker::path(), &applied_keys, &errors, mgr.applying_release());
    }

    // UpdateManager lock 해제 (supervisor/extension_manager lock과 교차 잠금 방지)
    drop(mgr);

    // 데몬이 직접 적용한 경우에만 핫로드 (업데이터 위임 시에는 불필요)
    if needs_updater.is_empty() {
        if has_module_updates {
            if let Some(ref sup) = state.supervisor {
                let mut sup = sup.write().await;
                match sup.refresh_modules() {
                    Ok(modules) => {
                        tracing::info!("[Updates] Hot-reloaded {} module(s) after update", modules.len());
                        sup.migrate_list_settings();
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
        "requires_updater": !needs_updater.is_empty(),
        "errors": errors,
    }))
}

/// POST /api/updates/install — 확인 → 받을 것 전부 받기 → 적용. 데몬·GUI처럼 재시작이 필요한 것이 있으면 업데이터를 띄운다
/// (그 뒤 데몬은 꺼지고 업데이터가 GUI나 데몬을 다시 띄운다). GUI가 없는 원격(디스코드)에서도 끝까지 가게 하는 경로다.
/// 답: apply의 답(`ok, applied, requires_updater, errors`)에 `downloaded`, 받지 못한 것 `download_errors`, 업데이터를 띄웠으면 `updater_launched: true`
async fn install_updates(State(state): State<UpdateState>) -> axum::response::Response {
    use crate::ipc::internal::into_internal;
    if let Err(e) = check_and_follow_up(&state).await {
        return Json(json!({ "ok": false, "error": format!("Update check failed: {}", e) })).into_response();
    }
    let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest::default()).await).await.body;
    if downloaded.get("ok").and_then(Value::as_bool) == Some(false) && downloaded.get("downloaded").is_none() {
        return Json(downloaded).into_response();
    }
    let mut applied = into_internal(apply_updates_with(state.clone(), ApplyRequest::default()).await).await.body;
    applied["downloaded"] = downloaded.get("downloaded").cloned().unwrap_or(Value::Null);
    // 일부만 받았으면 받은 것은 적용하고 받지 못한 것을 함께 알린다
    if let Some(failed) = downloaded.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
        applied["ok"] = json!(false);
        applied["download_errors"] = json!(failed);
    }
    if applied.get("requires_updater").and_then(Value::as_bool) == Some(true) {
        let launched = into_internal(launch_updater(State(state), LooseJson(None)).await).await.body;
        if launched.get("ok").and_then(Value::as_bool) == Some(false) {
            applied["ok"] = json!(false);
            applied["error"] = launched.get("error").cloned().unwrap_or(Value::Null);
        } else {
            applied["updater_launched"] = json!(true);
        }
    }
    Json(applied).into_response()
}

/// POST /api/updates/launch-updater — 데몬이 업데이터를 띄우고 스스로 종료한다 (U7)
///
/// `apply`가 `requires_updater: true`를 돌려준 뒤 부른다. 어느 클라이언트가 떠 있든
/// 적용 경로가 하나가 되도록 업데이터 실행은 데몬만 한다.
/// Body: `{ "relaunch": ["<exe>", "추가 인자"...] }` — 적용 후 다시 띄울 프로그램(생략하면 업데이터가 GUI를 찾는다)
#[derive(Deserialize, Default)]
struct LaunchUpdaterRequest {
    #[serde(default)]
    relaunch: Vec<String>,
}

async fn launch_updater(
    State(state): State<UpdateState>,
    LooseJson(body): LooseJson<LaunchUpdaterRequest>,
) -> axum::response::Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let Some(updater) = updater_binary() else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "saba-chan-updater not found next to the daemon" })),
        )
            .into_response();
    };
    if state.manager.read().await.get_pending_components().is_empty() {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "No downloaded updates to apply" })),
        )
            .into_response();
    }
    use std::sync::atomic::Ordering;
    if state.updater_launched.swap(true, Ordering::SeqCst) {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "The updater is already launching", "error_code": "updater_already_launched" })),
        )
            .into_response();
    }

    // 이번 적용에만 쓰는 실행 표 (업데이터는 표 없이 --apply를 거절한다)
    let ticket = match saba_chan_updater_lib::launch_ticket::issue(&saba_chan_updater_lib::constants::resolve_staging_dir()) {
        Ok(t) => t,
        Err(e) => {
            state.updater_launched.store(false, Ordering::SeqCst);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("Failed to prepare updater launch: {}", e) })),
            )
                .into_response()
        }
    };
    // 떠 있는 GUI는 실행 파일을 쥐고 있어 바꿀 수 없다. GUI가 부른 게 아니면(CLI·자동 업데이트) 데몬이 끄고
    // 업데이터가 같은 GUI를 다시 띄우게 한다(GUI가 뜨면서 데몬도 띄운다). GUI가 불렀으면 그 GUI는 스스로 끝난다
    let (guis, resident) = match &state.clients {
        Some(clients) => {
            let list = clients.list().await;
            (running_guis(&list), only_resident_guis(&list))
        }
        None => (Vec::new(), false),
    };
    let relaunch = relaunch_target(body.relaunch, &guis, resident);
    let args = updater_args(ticket, saba_chan_updater_lib::profile::Profile::load(), relaunch);
    if let Err(e) = spawn_detached(&updater, &args) {
        state.updater_launched.store(false, Ordering::SeqCst);
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("Failed to start updater: {}", e) })),
        )
            .into_response();
    }
    tracing::info!("[Updates] Launched updater {} {:?}; daemon is shutting down", updater.display(), args);

    if let Some(stop) = state.stop_requested.clone() {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await; // 응답이 나가도록
            close_guis(&guis).await;
            stop.cancel();
        });
    }
    Json(json!({ "ok": true, "updater_launched": true, "daemon_stopping": true })).into_response()
}

/// 업데이터 실행 인자. 헤드리스 설치는 창 없이 적용하게 `--headless`를 붙이고,
/// 다시 띄울 것을 받지 않았으면 데몬 자신을 다시 띄우게 한다 (GUI가 없어 데몬을 띄워 줄 쪽이 없다)
fn updater_args(ticket: String, profile: saba_chan_updater_lib::profile::Profile, mut relaunch: Vec<String>) -> Vec<String> {
    let mut args = vec![
        "--apply".to_string(),
        "--wait-pid".to_string(),
        std::process::id().to_string(),
        "--launch-token".to_string(),
        ticket,
    ];
    if profile == saba_chan_updater_lib::profile::Profile::Headless {
        args.push("--headless".to_string());
        if relaunch.is_empty() {
            if let Ok(me) = std::env::current_exe() {
                relaunch = vec![me.to_string_lossy().into_owned(), "--spawned".to_string()];
            }
        }
    }
    if !relaunch.is_empty() {
        args.push("--relaunch".to_string());
        args.extend(relaunch);
    }
    args
}

/// 등록된 GUI 중 지금도 GUI 실행 파일로 떠 있는 것의 PID와 실행 파일 (PID 재사용 대비)
fn running_guis(clients: &[(String, crate::ipc::RegisteredClient)]) -> Vec<(u32, std::path::PathBuf)> {
    let mut pids: Vec<u32> = clients
        .iter()
        .filter(|(_, c)| c.kind == crate::ipc::ClientKind::Gui)
        .filter_map(|(_, c)| c.pid)
        .filter(|&pid| pid != std::process::id())
        .collect();
    pids.sort_unstable();
    pids.dedup();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    pids.into_iter()
        .filter_map(|pid| {
            let proc = sys.process(sysinfo::Pid::from_u32(pid))?;
            let exe = proc.exe()?.to_path_buf();
            let name = exe.file_name()?.to_string_lossy().into_owned();
            saba_chan_updater_lib::GUI_EXE_NAMES.iter().any(|g| g.eq_ignore_ascii_case(&name)).then_some((pid, exe))
        })
        .collect()
}

/// 등록된 GUI가 모두 창 없이 트레이만 들고 있다 (다시 띄울 때도 창을 열지 않는다)
fn only_resident_guis(clients: &[(String, crate::ipc::RegisteredClient)]) -> bool {
    let mut guis = clients.iter().filter(|(_, c)| c.kind == crate::ipc::ClientKind::Gui).peekable();
    guis.peek().is_some() && guis.all(|(_, c)| c.resident)
}

/// 업데이터가 다시 띄울 것: 떠 있던 GUI가 있으면 그 GUI(요청이 이미 GUI면 그대로), 없으면 요청대로.
/// 떠 있던 GUI가 트레이만 들고 있었으면(`resident`) 트레이 모드로 다시 띄운다
fn relaunch_target(requested: Vec<String>, guis: &[(u32, std::path::PathBuf)], resident: bool) -> Vec<String> {
    let is_gui = |exe: &str| {
        let name = std::path::Path::new(exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        saba_chan_updater_lib::GUI_EXE_NAMES.iter().any(|g| g.eq_ignore_ascii_case(&name))
    };
    match guis.first() {
        Some((_, exe)) if !requested.first().is_some_and(|r| is_gui(r)) => {
            let mut line = vec![exe.to_string_lossy().into_owned()];
            if resident {
                line.push(crate::tray_host::TRAY_ARG.to_string());
            }
            line
        }
        _ => requested,
    }
}

/// GUI가 스스로 끝날 틈(업데이트를 부른 GUI는 곧 끝난다)을 준 뒤 남은 것을 끈다
async fn close_guis(guis: &[(u32, std::path::PathBuf)]) {
    use crate::supervisor::process::{force_kill_pid, is_process_alive};
    let alive = |pids: &[(u32, std::path::PathBuf)]| pids.iter().filter(|(pid, _)| is_process_alive(*pid)).cloned().collect::<Vec<_>>();
    for _ in 0..10 {
        if alive(guis).is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    let left = running_guis_among(&alive(guis));
    for (pid, exe) in &left {
        tracing::info!("[Updates] Closing GUI (PID {}, {}) so the updater can replace it", pid, exe.display());
        if let Err(e) = force_kill_pid(*pid) {
            tracing::warn!("[Updates] Could not close GUI PID {}: {}", pid, e);
        }
    }
    for _ in 0..20 {
        if alive(&left).is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// 기다리는 동안 PID가 다른 프로그램에 다시 쓰였으면 빼고, 같은 실행 파일인 것만 남긴다
fn running_guis_among(guis: &[(u32, std::path::PathBuf)]) -> Vec<(u32, std::path::PathBuf)> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    guis.iter()
        .filter(|(pid, exe)| sys.process(sysinfo::Pid::from_u32(*pid)).and_then(|p| p.exe()) == Some(exe.as_path()))
        .cloned()
        .collect()
}

/// 업데이터 실행 파일. `SABA_UPDATER_PATH`가 있으면 그것, 없으면 데몬 실행 파일 옆(또는 한 단계 위).
fn updater_binary() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SABA_UPDATER_PATH").map(std::path::PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let exe = std::env::current_exe().ok()?;
    let name = format!("saba-chan-updater{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors().skip(1).take(2).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

/// 데몬이 끝나도 살아남게 띄운다
fn spawn_detached(program: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // 데몬을 띄운 쪽의 Job Object가 닫혀도 업데이터가 살아남게 빠져나온다. 허용되지 않으면 그냥 띄운다.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(5) => {
                cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                cmd.spawn().map(|_| ())
            }
            other => other.map(|_| ()),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        cmd.spawn().map(|_| ())
    }
}

/// GET /api/updates/last-result — 마지막 적용 결과 (업데이터나 데몬이 남긴 완료 마커). 없으면 `result: null`.
/// `?lang=ko`면 그 언어의 패치노트를 `notes`에 고른다 (없으면 영어·한국어 순). 언어별 원문은 `result.release.notes`.
/// 클라이언트가 확인한(DELETE) 결과는 `?include_seen=true`일 때만 `acknowledged: true`와 함께 준다
async fn get_last_result(Query(q): Query<LastResultQuery>) -> impl IntoResponse {
    let path = saba_chan_updater_lib::UpdateCompletionMarker::path();
    Json(last_result_at(&path, q.include_seen, q.lang.as_deref().unwrap_or("en")))
}

#[derive(Deserialize, Default)]
struct LastResultQuery {
    lang: Option<String>,
    #[serde(default)]
    include_seen: bool,
}

fn last_result_at(path: &std::path::Path, include_seen: bool, lang: &str) -> Value {
    let marker = saba_chan_updater_lib::UpdateCompletionMarker::load_from(path).filter(|m| include_seen || !m.acknowledged);
    let acknowledged = marker.as_ref().is_some_and(|m| m.acknowledged);
    let mut body = last_result_body(marker, lang);
    body["acknowledged"] = json!(acknowledged);
    body
}

/// 데몬이 바로 적용한 결과를 완료 마커로 남긴다 (업데이터가 적용할 때와 같은 모양). 클라이언트가 결과·패치노트를 본다
fn record_daemon_apply(path: &std::path::Path, applied: &[String], errors: &[String], release: Option<saba_chan_updater_lib::marker::ReleaseSummary>) {
    let marker = saba_chan_updater_lib::UpdateCompletionMarker {
        message: Some(if errors.is_empty() {
            format!("{} updates applied: {}", applied.len(), applied.join(", "))
        } else {
            format!("{} applied, {} failed", applied.len(), errors.len())
        }),
        success: errors.is_empty(),
        failed: errors.to_vec(),
        release,
        ..saba_chan_updater_lib::UpdateCompletionMarker::success(applied.to_vec())
    };
    if let Err(e) = marker.save_to(path) {
        tracing::warn!("[Updates] Cannot record the update result: {}", e);
    }
}

fn last_result_body(marker: Option<saba_chan_updater_lib::UpdateCompletionMarker>, lang: &str) -> Value {
    let notes = marker.as_ref().and_then(|m| m.release.as_ref()).and_then(|r| r.notes_for(lang)).map(str::to_string);
    let version = marker.as_ref().and_then(|m| m.release.as_ref()).map(|r| r.version.clone());
    json!({ "ok": true, "result": marker, "version": version, "notes": notes })
}

/// DELETE /api/updates/last-result — 사용자가 결과를 확인했다. 결과는 지우지 않고 확인했다고만 적는다
/// (다른 클라이언트도 `?include_seen=true`로 패치노트를 다시 본다)
async fn clear_last_result() -> impl IntoResponse {
    match saba_chan_updater_lib::UpdateCompletionMarker::acknowledge_at(&saba_chan_updater_lib::UpdateCompletionMarker::path()) {
        Ok(()) => Json(json!({ "ok": true })),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

/// GET /api/updates/integrity — 업데이터가 설치하며 기록한 파일 해시와 지금 파일을 비교
async fn check_integrity(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let report = state.manager.read().await.verify_integrity();

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
        // 이 데몬이 모의 릴레이(scripts/mock-release-server.js)의 서명을 믿는 개발 빌드인가.
        // 아니면 GUI는 MOCK 출처를 감추거나 막는다
        "mock_relay": cfg!(feature = "mock-relay"),
    }))
}

/// 앱에서 고를 수 있는 업데이트 채널 (위 채널은 아래 채널 릴리즈도 받는다, updater `relay::CHANNELS`)
const SELECTABLE_CHANNELS: [&str; 3] = ["stable", "rc", "beta"];

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
    if let Some(v) = body.get("check_interval_hours").and_then(|v| v.as_u64()) {
        cfg.check_interval_hours = v.clamp(1, 8) as u32;
    }
    if let Some(v) = body.get("auto_download").and_then(|v| v.as_bool()) {
        cfg.auto_download = v;
    }
    if let Some(v) = body.get("install_root").and_then(|v| v.as_str()) {
        cfg.install_root = Some(v.to_string());
    }
    // 업데이트 출처는 앱에서 바꾸지 않는다 (D7): config.json의 updates.relay_url로만.
    // 모의 릴레이를 믿는 개발 빌드만 앱(개발 옵션)에서 바꿀 수 있다. 비우기(기본 릴레이로 되돌리기)는 늘 된다
    if let Some(v) = body.get("relay_url").and_then(|v| v.as_str()) {
        if !v.is_empty() && !cfg!(feature = "mock-relay") {
            return (
                axum::http::StatusCode::FORBIDDEN,
                Json(json!({
                    "ok": false,
                    "error": "The update source can only be changed in config.json (updates.relay_url)",
                    "error_code": "relay_url_not_settable",
                })),
            )
                .into_response();
        }
        cfg.relay_url = if v.is_empty() { None } else { Some(v.to_string()) };
    }
    // 채널은 정식(stable)·미리보기(rc)·베타(beta)만 고른다 (nightly는 쓰지 않는다)
    if let Some(v) = body.get("channel").and_then(|v| v.as_str()) {
        if !SELECTABLE_CHANNELS.contains(&v) {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "error": format!("Unknown update channel '{}' (stable, rc, beta)", v),
                    "error_code": "invalid_channel",
                })),
            )
                .into_response();
        }
        cfg.channel = v.to_string();
    }

    mgr.update_config(cfg.clone());
    if let Err(e) = save_updater_config(&cfg) {
        tracing::warn!("[Updates] Failed to persist update config: {}", e);
    }

    Json(json!({
        "ok": true,
        "config": cfg,
    }))
    .into_response()
}

// ═══════════════════════════════════════════════════════
// 설정 유틸리티
// ═══════════════════════════════════════════════════════

/// 저장된 업데이트 설정 (`config.json`의 `updates`). 없거나 깨졌으면 기본값.
pub(crate) fn load_updater_config() -> UpdateConfig {
    load_updater_config_from(&ConfigFile::default_location())
}

fn load_updater_config_from(file: &ConfigFile) -> UpdateConfig {
    match file.section("updates") {
        serde_json::Value::Null => UpdateConfig::default(),
        mut v => {
            // 0.1.1 이름(`relay_base_url`)과 새 이름이 같이 있으면 별칭이 겹쳐 설정 전체가 깨진다: 새 이름을 쓴다
            if let Some(obj) = v.as_object_mut().filter(|o| o.contains_key("relay_url")) {
                obj.remove("relay_base_url");
            }
            serde_json::from_value(v).unwrap_or_else(|e| {
                tracing::warn!("[Updates] Ignoring invalid 'updates' config: {}", e);
                UpdateConfig::default()
            })
        }
    }
}

fn save_updater_config(cfg: &UpdateConfig) -> std::io::Result<()> {
    save_updater_config_to(&ConfigFile::default_location(), cfg)
}

fn save_updater_config_to(file: &ConfigFile, cfg: &UpdateConfig) -> std::io::Result<()> {
    file.set_section("updates", serde_json::to_value(cfg).map_err(std::io::Error::other)?)
}

fn resolve_modules_dir() -> String {
    let dir = saba_chan_updater_lib::constants::resolve_modules_dir();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir.to_string_lossy().to_string()
}



#[cfg(test)]
mod tests {
    /// 등록된 GUI 중 GUI 실행 파일로 떠 있는 것만 고르고, 업데이터를 띄울 때 끈다
    #[cfg(unix)]
    #[tokio::test]
    async fn launching_the_updater_closes_only_registered_gui_processes() {
        let tmp = tempfile::tempdir().unwrap();
        let gui_exe = tmp.path().join("saba-chan-gui");
        std::fs::copy("/bin/sleep", &gui_exe).unwrap();
        let spawn = |exe: &std::path::Path| {
            // 방금 복사한 파일은 다른 테스트가 fork하는 동안 쓰기 핸들이 남아 ETXTBSY가 날 수 있다
            let mut child = (0..50)
                .find_map(|_| match std::process::Command::new(exe).arg("30").spawn() {
                    Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        None
                    }
                    other => Some(other.unwrap()),
                })
                .expect("executable stayed busy");
            let pid = child.id();
            std::thread::spawn(move || child.wait()); // 끝나면 바로 거둬 좀비로 남지 않게
            pid
        };
        let gui = spawn(&gui_exe);
        let other = spawn(std::path::Path::new("/bin/sleep"));
        let registry = crate::ipc::ClientRegistry::new();
        registry.register(crate::ipc::ClientKind::Gui, Some(gui), None).await;
        registry.register(crate::ipc::ClientKind::Gui, Some(other), None).await; // PID가 남의 프로그램에 다시 쓰인 경우
        registry.register(crate::ipc::ClientKind::Cli, Some(other), None).await;

        let guis = super::running_guis(&registry.list().await);
        assert_eq!(guis, vec![(gui, std::fs::canonicalize(&gui_exe).unwrap())]);

        super::close_guis(&guis).await;
        assert!(!crate::supervisor::process::is_process_alive(gui), "the registered GUI is closed");
        assert!(crate::supervisor::process::is_process_alive(other), "other programs are left alone");
        let _ = crate::supervisor::process::force_kill_pid(other);
    }

    /// 업데이터를 띄운 뒤 다시 부르면 표를 새로 끊지 않고 409로 거절한다.
    /// 표를 새로 끊으면 먼저 띄운 업데이터의 표가 무효가 되어 둘 다 아무것도 적용하지 못한다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_second_launch_is_refused_while_one_is_pending() {
        use axum::extract::State;
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        std::fs::create_dir_all(&staging).unwrap();
        let staged = staging.join("saba-chan-gui-linux-x64.zip");
        std::fs::write(&staged, b"zip").unwrap();
        let pending = json!([{
            "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
            "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-linux-x64.zip",
            "release_notes": null, "published_at": null, "downloaded": true,
            "downloaded_path": staged.to_string_lossy(), "installed": true
        }]);
        std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();
        std::env::set_var("SABA_UPDATER_PATH", "/bin/true");

        let state = UpdateState::new();
        state.manager.write().await.load_pending_manifest().unwrap();
        let first = super::launch_updater(State(state.clone()), crate::ipc::handlers::LooseJson(None)).await;
        assert_eq!(first.status(), axum::http::StatusCode::OK);
        let ticket = std::fs::read_to_string(staging.join("apply-ticket")).unwrap();

        let second = super::launch_updater(State(state.clone()), crate::ipc::handlers::LooseJson(None)).await;
        assert_eq!(second.status(), axum::http::StatusCode::CONFLICT);
        let body = axum::body::to_bytes(second.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error_code"], "updater_already_launched");
        assert_eq!(std::fs::read_to_string(staging.join("apply-ticket")).unwrap(), ticket, "the first ticket stays valid");
    }

    /// CLI가 데몬을 다시 띄우라고 해도 GUI가 떠 있었으면 GUI를 띄운다. GUI가 부른 요청은 그대로
    #[test]
    fn the_updater_relaunches_the_gui_that_was_open() {
        let gui = std::path::PathBuf::from("/opt/saba/saba-chan-gui");
        let guis = vec![(42, gui.clone())];
        let daemon = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
        assert_eq!(super::relaunch_target(daemon.clone(), &guis, false), vec![gui.to_string_lossy().into_owned()]);
        assert_eq!(super::relaunch_target(Vec::new(), &guis, false), vec![gui.to_string_lossy().into_owned()]);
        assert_eq!(super::relaunch_target(daemon.clone(), &[], false), daemon);
        let own = vec![format!("/other/{}", saba_chan_updater_lib::GUI_EXE_NAMES[0])];
        assert_eq!(super::relaunch_target(own.clone(), &guis, false), own);
    }

    /// 트레이만 든 GUI는 업데이트 뒤에도 창 없이 트레이로 다시 뜬다 (자동 업데이트마다 창이 튀어나오지 않게)
    #[tokio::test]
    async fn a_tray_only_gui_is_relaunched_as_a_tray() {
        let gui = std::path::PathBuf::from("/opt/saba/saba-chan-gui");
        let guis = vec![(42, gui.clone())];
        let daemon = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
        assert_eq!(
            super::relaunch_target(daemon, &guis, true),
            vec![gui.to_string_lossy().into_owned(), "--tray".to_string()]
        );

        let registry = crate::ipc::ClientRegistry::new();
        assert!(!super::only_resident_guis(&registry.list().await), "no GUI at all is not a tray");
        let tray = registry.register(crate::ipc::ClientKind::Gui, Some(1), None).await;
        registry.register(crate::ipc::ClientKind::Cli, Some(2), None).await;
        assert!(!super::only_resident_guis(&registry.list().await));
        registry.set_resident(&tray, true).await;
        assert!(super::only_resident_guis(&registry.list().await), "CLIs do not count");
        registry.register(crate::ipc::ClientKind::Gui, Some(3), None).await;
        assert!(!super::only_resident_guis(&registry.list().await), "a GUI with a window is relaunched with its window");
    }

    /// 헤드리스 설치는 창 없이 적용하고, 다시 띄울 것을 받지 않았으면 데몬 자신을 다시 띄운다. 데스크톱은 그대로
    #[test]
    fn a_headless_install_applies_without_a_window_and_relaunches_the_daemon() {
        use saba_chan_updater_lib::profile::Profile;
        let me = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let args = super::updater_args("t".into(), Profile::Headless, Vec::new());
        let relaunch = args.iter().position(|a| a == "--relaunch").expect("relaunches something");
        assert!(args[..relaunch].contains(&"--headless".to_string()), "{args:?}");
        assert_eq!(args[relaunch + 1..], [me, "--spawned".to_string()]);

        let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
        let args = super::updater_args("t".into(), Profile::Headless, cli.clone());
        assert_eq!(args[args.len() - 2..], cli[..]);

        let args = super::updater_args("t".into(), Profile::Desktop, Vec::new());
        assert!(!args.iter().any(|a| a == "--headless" || a == "--relaunch"), "{args:?}");
    }

    /// 데몬이 바로 적용한 업데이트(재시작 없는 모듈·익스텐션·설치기)도 결과와 패치노트를 남긴다.
    /// 한 클라이언트가 확인해도 결과를 지우지 않아 `?include_seen=true`로 다시 볼 수 있다(CLI `update last`).
    /// 매개변수 없는 조회는 예전처럼 확인한 결과를 보지 않는다(확인 뒤 GUI가 다시 띄우지 않게)
    #[test]
    fn applied_updates_keep_their_notes_after_a_client_acknowledges_them() {
        use saba_chan_updater_lib::marker::ReleaseSummary;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("update-complete.json");
        let release = ReleaseSummary { version: "0.1.8".into(), released_at: None, notes: [("ko".to_string(), "고침".to_string())].into() };
        super::record_daemon_apply(&path, &["module-minecraft".into()], &["ext-steamcmd: busy".into()], Some(release));

        let first = super::last_result_at(&path, false, "ko");
        assert_eq!((first["version"].as_str(), first["notes"].as_str()), (Some("0.1.8"), Some("고침")), "{first}");
        assert_eq!(first["result"]["updated_components"], json!(["module-minecraft"]));
        assert_eq!(first["result"]["failed"], json!(["ext-steamcmd: busy"]));
        assert_eq!(first["result"]["success"], false);
        assert_eq!(first["acknowledged"], false);

        saba_chan_updater_lib::UpdateCompletionMarker::acknowledge_at(&path).unwrap();
        assert!(super::last_result_at(&path, false, "ko")["result"].is_null(), "an acknowledged result is not shown as new");
        let again = super::last_result_at(&path, true, "ko");
        assert_eq!(again["notes"], "고침", "the notes stay readable after a client acknowledged them");
        assert_eq!(again["acknowledged"], true);
    }

    #[test]
    fn apply_and_download_accept_a_missing_body_but_not_a_broken_one() {
        let empty: super::ApplyRequest = super::body_or_default(b"").ok().unwrap();
        assert!(empty.components.is_empty());
        let blank: super::DownloadRequest = super::body_or_default(b" \n").ok().unwrap();
        assert!(blank.components.is_empty());
        let some: super::ApplyRequest = super::body_or_default(br#"{"components":["gui"]}"#).ok().unwrap();
        assert_eq!(some.components, vec!["gui".to_string()]);
        assert!(super::body_or_default::<super::ApplyRequest>(b"{oops").is_err(), "a broken body must not mean 'everything'");
    }

    use super::*;

    /// D7: 릴리즈 빌드에서는 앱이 업데이트 출처를 바꿀 수 없다 (거부는 저장 전에 끝난다)
    #[cfg(not(feature = "mock-relay"))]
    #[tokio::test]
    async fn the_update_source_cannot_be_changed_from_the_app() {
        let state = UpdateState::new();
        let before = state.manager.read().await.get_config().relay_url;
        let resp = set_config(State(state.clone()), Json(json!({ "relay_url": "https://evil.example" })))
            .await
            .into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(state.manager.read().await.get_config().relay_url, before);
    }

    /// 모르는 채널은 저장하지 않는다
    #[tokio::test]
    async fn an_unknown_update_channel_is_refused() {
        let state = UpdateState::new();
        let before = state.manager.read().await.get_config().channel;
        let resp = set_config(State(state.clone()), Json(json!({ "channel": "nightly" }))).await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(state.manager.read().await.get_config().channel, before);
        assert!(SELECTABLE_CHANNELS.iter().all(|c| saba_chan_updater_lib::relay::CHANNELS.contains(c)));
    }

    /// A5: 적용 결과에 릴리즈 버전과 고른 언어의 패치노트가 붙는다. 예전 마커(릴리즈 없음)는 null
    #[test]
    fn last_result_carries_the_version_and_notes_in_the_asked_language() {
        use saba_chan_updater_lib::marker::ReleaseSummary;
        let mut marker = saba_chan_updater_lib::UpdateCompletionMarker::success(vec!["saba-core".into()]);
        marker.release = Some(ReleaseSummary {
            version: "0.1.3".into(),
            released_at: None,
            notes: [("ko".to_string(), "고침".to_string()), ("en".to_string(), "Fixes".to_string())].into(),
        });
        let body = last_result_body(Some(marker.clone()), "ko");
        assert_eq!((body["version"].as_str(), body["notes"].as_str()), (Some("0.1.3"), Some("고침")));
        assert_eq!(last_result_body(Some(marker.clone()), "ja")["notes"], "Fixes");
        assert_eq!(body["result"]["release"]["notes"]["en"], "Fixes");

        marker.release = None;
        let old = last_result_body(Some(marker), "ko");
        assert!(old["notes"].is_null() && old["version"].is_null());
        assert!(last_result_body(None, "ko")["result"].is_null());
    }

    #[test]
    fn updater_config_survives_restart() {
        let mut cfg = UpdateConfig::default();
        cfg.enabled = false;
        cfg.check_interval_hours = 6;
        cfg.auto_download = true;
        let dir = tempfile::tempdir().unwrap();
        save_updater_config_to(&ConfigFile::at(dir.path()), &cfg).unwrap();

        let loaded = load_updater_config_from(&ConfigFile::at(dir.path()));
        assert!(!loaded.enabled);
        assert_eq!(loaded.check_interval_hours, 6);
        assert!(loaded.auto_download);
    }

    /// 0.1.1이 남긴 `updates` 설정: 채널과 릴레이 주소가 업데이트 뒤에도 남아야 한다
    #[test]
    fn settings_written_by_0_1_1_keep_their_channel_and_relay() {
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        file.set_section(
            "updates",
            json!({
                "enabled": true, "check_interval_hours": 3, "auto_download": false, "auto_apply": false,
                "install_root": null, "relay_base_url": "http://127.0.0.1:9876", "channel": "beta",
                "platform": "windows-x64", "client_id": "abcdefgh12345678"
            }),
        )
        .unwrap();
        let loaded = load_updater_config_from(&file);
        assert_eq!(loaded.channel, "beta");
        assert_eq!(loaded.relay_url.as_deref(), Some("http://127.0.0.1:9876"));

        file.set_section("updates", json!({ "relay_base_url": "", "channel": "beta" })).unwrap();
        assert_eq!(load_updater_config_from(&file).relay_url, None, "blank means the default relay");

        // 둘 다 있으면 새 이름을 쓴다 (설정 전체가 기본값으로 떨어지지 않는다)
        file.set_section("updates", json!({ "relay_base_url": "http://old", "relay_url": "http://new", "channel": "beta" })).unwrap();
        let both = load_updater_config_from(&file);
        assert_eq!((both.relay_url.as_deref(), both.channel.as_str()), (Some("http://new"), "beta"));
    }
}
