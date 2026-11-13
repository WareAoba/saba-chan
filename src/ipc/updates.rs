//! 데몬 내장 업데이트 API — `/api/updates/*`
//!
//! 업데이트 확인(check)과 다운로드(download)를 데몬 프로세스 내에서 수행합니다.
//! 파일 적용(apply)은 별도 업데이터 프로세스에서 처리합니다.
//!
//! 확인은 데몬이 한다: 릴레이 heartbeat의 판 번호(`latestReleases`)가 바뀌면 확인하고(`on_latest_releases`), 연결과 상관없이
//! 24시간마다 한 번 더 본다(`spawn_update_scheduler`, `check_interval_hours`는 읽기만 한다). GUI·CLI는 `/api/updates/status`를 읽어 표시만 한다.
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
//! - `PUT  /api/updates/config`              — 업데이트 설정 변경 (채널이 바뀌면 곧바로 확인한다)
//! - `POST /api/updates/phase/ack`           — 알약의 `done`을 지운다
//! - `POST /api/updates/restart`             — `ready`에서 "지금 다시 시작": 실패한 판 기록을 지우고 받아 둔 것을 적용한다
//! - `PUT  /api/updates/gui-window`          — GUI 창이 보이는지 (`{open}`)
//! - `GET  /api/updates/addons/{key}/versions` — 모듈·익스텐션의 고를 수 있는 판 (릴레이 `/v1/addons/versions`, 서명 확인)
//! - `POST /api/updates/addons/{key}/pin`    — 고른 판(`{version}`)으로 바꾸고 고정한다
//! - `DELETE /api/updates/addons/{key}/pin`  — 고정을 풀고 가장 새 판으로 바꾼다
//!
//! 앱 업데이트는 묻지 않고 적용한다(`spawn_core_apply`): 켜진 서버가 있어도 코어를 바꾸고, 데몬 작업이나 다시 붙지 못하는
//! 서버가 있을 때만 기다린다. 단계(`phase`)는 `update_phase`가 정한다

use super::handlers::LooseJson;
use axum::{
    extract::{Path, Query, State},
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
    addon_pins, Component, DownloadProgress, IntegrityStatus, UpdateConfig, UpdateManager,
};

use super::update_phase::{self, Decision, LiveState, Phase, PhaseState, ReadyReason};

// ═══════════════════════════════════════════════════════
// 공유 상태
// ═══════════════════════════════════════════════════════

/// heartbeat `latestReleases` 중 이 데몬이 보는 채널별 릴리즈 id
type ReleaseIds = std::collections::BTreeMap<String, u64>;

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
    /// 데몬이 모듈 업데이트를 바로 적용했다: 기동 때 띄운 작업이 그 모듈이 쓰는 익스텐션을 맞춘다 (`prepare_after_update`)
    pub modules_updated: Arc<tokio::sync::Notify>,
    /// 알약 단계·마지막으로 본 릴리즈 id (`<staging>/live-update.json`)
    live: Arc<std::sync::Mutex<LiveState>>,
    live_path: Arc<std::path::PathBuf>,
    /// GUI가 알린 창 상태 (`PUT /api/updates/gui-window`). 알린 적이 없으면 None
    gui_window: Arc<std::sync::Mutex<Option<bool>>>,
    /// 적용 반복을 다시 보게 깨운다 (`begin_core_apply`). 창이 닫히기를 기다리는 반복이 듣는다
    apply_wake: Arc<tokio::sync::Notify>,
    /// 내려가기 직전 릴레이에 "업데이트로 잠깐 끊김"을 알린다
    pub relay: Option<crate::relay_agent::RelayAgent>,
    /// 확인 중인 heartbeat 판 번호. 확인이 성공해야 본 것(`LiveState::seen`)으로 남긴다
    release_check: Arc<std::sync::Mutex<Option<std::collections::BTreeMap<String, u64>>>>,
    /// 확인했지만 응답에 그 번호가 담기지 않은 판 번호와 그때 (`RELEASE_RECHECK` 뒤 한 번 더 본다)
    release_unconfirmed: Arc<std::sync::Mutex<Option<(ReleaseIds, std::time::Instant)>>>,
    /// 앱 업데이트 적용 반복이 도는 중인가 (`spawn_core_apply`, 한 번에 하나)
    core_applying: Arc<std::sync::atomic::AtomicBool>,
    /// 적용 반복이 새로 받아 볼 확인이 있었다: 반복이 다음 차례에 받고, 끝난 뒤에 왔으면 반복을 다시 띄운다
    apply_recheck: Arc<std::sync::atomic::AtomicBool>,
}

impl UpdateState {
    /// 내장 기본값으로 업데이트 매니저 생성
    pub fn new() -> Self {
        let cfg = load_updater_config();
        let modules_dir = saba_chan_updater_lib::constants::ensure_modules_dir().to_string_lossy().to_string();
        let mut mgr = UpdateManager::new(cfg, &modules_dir);
        let live_path = LiveState::path();
        let mut live = LiveState::load_from(&live_path);
        #[cfg(not(test))]
        let marker = std::fs::read_to_string(saba_chan_updater_lib::UpdateCompletionMarker::path())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        // 시험은 이 PC의 실제 완료 마커로 단계나 설정을 바꾸지 않는다
        #[cfg(test)]
        let marker: Option<Value> = None;
        let (changed, rolled_back) = update_phase::adopt_startup_marker(&mut live, marker.as_ref(), || mgr.pending_release().map(|r| r.version));
        if changed {
            live.save_to(&live_path);
        }
        // 되돌린 코어 판은 설치기가 설정에 남긴다. 남기지 못했으면 여기서 남긴다 (같은 코어를 다시 자동 적용하지 않게).
        // 이 마커는 업데이트 실패의 자동 되돌림이라 사용자 고정이 아니다
        if let Some(core) = rolled_back.filter(|_| mgr.config.failed_release.is_none()) {
            let mut cfg = mgr.get_config();
            cfg.failed_release = Some(core);
            cfg.user_held = false;
            if let Err(e) = save_updater_config(&cfg) {
                tracing::warn!("[Updates] Failed to persist update config: {}", e);
            }
            mgr.update_config(cfg);
        }
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
            modules_updated: Arc::default(),
            live: Arc::new(std::sync::Mutex::new(live)),
            live_path: Arc::new(live_path),
            gui_window: Arc::default(),
            apply_wake: Arc::default(),
            relay: None,
            release_check: Arc::default(),
            release_unconfirmed: Arc::default(),
            core_applying: Arc::default(),
            apply_recheck: Arc::default(),
        }
    }

    pub fn with_relay(mut self, relay: crate::relay_agent::RelayAgent) -> Self {
        self.relay = Some(relay);
        self
    }

    /// 지금 단계 (`done`은 하루가 지나면 `idle`)
    pub fn phase(&self) -> Phase {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).phase.visible(chrono::Utc::now())
    }

    /// 단계를 바꾸고 상태 파일에 남긴다. 같으면(기다리는 것까지) 시각을 새로 찍지 않는다
    fn set_phase(&self, phase: Phase) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let same = live.phase.state == phase.state
            && live.phase.version == phase.version
            && live.phase.reason == phase.reason
            && live.phase.waiting_for == phase.waiting_for;
        if same {
            return;
        }
        live.phase = phase;
        live.save_to(&self.live_path);
    }

    /// 코어 업데이트가 데몬 작업만 기다리는 중이다: 새 예약 작업(백업·게임 서버 업데이트)은 업데이트 뒤로 미룬다.
    /// 켜진 서버를 기다리면 며칠이 걸릴 수 있으니 예약 작업은 그대로 돈다
    pub fn core_update_waiting(&self) -> bool {
        let phase = self.phase();
        phase.state == PhaseState::Waiting
            && !phase.waiting_for.is_empty()
            && phase.waiting_for.iter().all(|b| b.kind.is_daemon_job())
    }

    /// GUI 창이 보이는가: 등록된 GUI가 있고, GUI가 알린 창 상태(알린 적이 없으면 트레이 표시가 아님)가 열림이다.
    /// GUI 등록이 사라지면 닫힘으로 본다
    async fn gui_window_open(&self) -> bool {
        let Some(clients) = &self.clients else { return false };
        let list = clients.list().await;
        let any_gui = list.iter().any(|(_, c)| c.kind == crate::ipc::ClientKind::Gui);
        let told = *self.gui_window.lock().unwrap_or_else(|e| e.into_inner());
        any_gui && told.unwrap_or_else(|| !only_resident_guis(&list))
    }

    /// heartbeat로 알고 확인까지 마친 가장 큰 릴리즈 id. heartbeat가 부르지 않은 확인(기동·채널 바꿈·주기·수동)도
    /// 이 값을 릴레이 캐시 키로 붙여, 같은 주소의 오래된 캐시 응답(내린 판이 남은 것)을 덜 받게 한다
    fn known_release_id(&self) -> Option<u64> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).seen.values().copied().max()
    }

    /// heartbeat 판 번호 중 이 데몬이 보는 것이 지난번 확인한 것과 다르고, 지금 확인 중인 것도 아니면 확인할 값.
    /// 응답에 담기지 않았던 번호는 `RELEASE_RECHECK`가 지난 뒤에 다시 본다. 확인을 마치면 `finish_release_check`를 부른다
    fn release_ids_to_check(&self, ids: std::collections::BTreeMap<String, u64>) -> Option<std::collections::BTreeMap<String, u64>> {
        if ids.is_empty() || self.live.lock().unwrap_or_else(|e| e.into_inner()).seen == ids {
            return None;
        }
        let unconfirmed = self.release_unconfirmed.lock().unwrap_or_else(|e| e.into_inner());
        if unconfirmed.as_ref().is_some_and(|(u, at)| *u == ids && at.elapsed() < RELEASE_RECHECK) {
            return None;
        }
        drop(unconfirmed);
        let mut checking = self.release_check.lock().unwrap_or_else(|e| e.into_inner());
        if checking.is_some() {
            return None;
        }
        *checking = Some(ids.clone());
        Some(ids)
    }

    /// 판 번호 확인을 마쳤다. `answered`: 실패·취소·꺼짐이면 None(다음 heartbeat가 다시 확인한다), 확인했으면 `/latest` 응답에
    /// 담긴 가장 큰 릴리즈 id. 그 id가 알린 번호에 못 미치면(캐시된 옛 응답, 이 플랫폼 몫이 아님) 바로 본 것으로 두지 않고
    /// `RELEASE_RECHECK` 뒤 한 번 더 본다. 두 번째도 못 미치면 본 것으로 둔다 (확인은 한 번호에 많아야 두 번)
    fn finish_release_check(&self, ids: std::collections::BTreeMap<String, u64>, answered: Option<Option<u64>>) {
        *self.release_check.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let Some(latest_id) = answered else { return };
        let wanted = ids.values().copied().max().unwrap_or(0);
        let mut unconfirmed = self.release_unconfirmed.lock().unwrap_or_else(|e| e.into_inner());
        let reflected = latest_id.is_some_and(|id| id >= wanted);
        if !reflected && unconfirmed.as_ref().is_none_or(|(u, _)| *u != ids) {
            *unconfirmed = Some((ids, std::time::Instant::now()));
            return;
        }
        *unconfirmed = None;
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.seen = ids;
        live.save_to(&self.live_path);
    }

    /// 적용 반복을 시작해도 되는가. 이미 돌고 있으면 그 반복이 새로 받아 보도록 남긴다
    fn begin_core_apply(&self) -> bool {
        use std::sync::atomic::Ordering;
        // 먼저 남긴다: 반복이 막 끝나는 중이어도 `end_core_apply`가 이것을 본다
        self.apply_recheck.store(true, Ordering::SeqCst);
        self.apply_wake.notify_one();
        !self.core_applying.swap(true, Ordering::SeqCst)
    }

    /// 적용 반복이 이번 차례에 새로 받아야 하는가 (받으면 지운다)
    fn take_apply_recheck(&self) -> bool {
        self.apply_recheck.swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    /// 적용 반복이 끝났다. 마지막 차례 뒤에 확인이 왔으면 true (반복을 다시 띄운다)
    fn end_core_apply(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.core_applying.store(false, Ordering::SeqCst);
        self.apply_recheck.swap(false, Ordering::SeqCst)
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
        .route("/api/updates/rollback", get(get_rollback).post(roll_back))
        .route("/api/updates/versions", get(get_versions))
        .route("/api/updates/install", post(install_updates))
        .route("/api/updates/last-result", get(get_last_result).delete(clear_last_result))
        .route("/api/updates/integrity", get(check_integrity))
        .route("/api/updates/repair", get(get_repair).post(repair))
        .route("/api/updates/config", get(get_config))
        .route("/api/updates/config", put(set_config))
        .route("/api/updates/phase/ack", post(ack_phase))
        .route("/api/updates/restart", post(restart_now))
        .route("/api/updates/gui-window", put(set_gui_window))
        .route("/api/updates/addons/{key}/versions", get(get_addon_versions))
        .route("/api/updates/addons/{key}/pin", post(pin_addon).delete(unpin_addon))
        .route("/api/daemon/update", get(get_daemon_update).post(install_updates).put(put_daemon_update))
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
    // 고른 뒤 다른 판이 깔린 고정은 싣지 않는다 (`AddonChoice::as_installed`). 다음 확인이 푼다
    let pins: std::collections::BTreeMap<String, addon_pins::AddonChoice> = addon_pins::load()
        .into_iter()
        .map(|(key, choice)| {
            let installed = mgr.installed_addon_version(&key).unwrap_or_default();
            (key, choice.as_installed(&installed))
        })
        .collect();

    // Locales는 UI에 표시하지 않음 — 백그라운드 자동 적용 대상
    let components: Vec<Value> = status.components.iter()
        .filter(|c| !matches!(c.component, Component::Locales))
        .map(|c| {
            let key = c.component.manifest_key();
            json!({
                "display_name": c.component.display_name(),
                "current_version": c.current_version,
                "latest_version": c.latest_version,
                "update_available": c.update_available,
                "downloaded": c.downloaded,
                "installed": c.installed,
                // 사용자가 고른 판 `{version, until_newer_than}` (고정하지 않았으면 null)
                "pinned": pins.get(&key).and_then(|p| p.pinned.as_ref()),
                "component": key,
                "kind": c.component.kind(),
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
        "phase": state.phase(),
        "stable_wait": status.stable_wait,
        // 설치된 번역 판 (locales/version). 바뀌면 GUI·TUI가 번역을 다시 읽는다
        "locales_version": crate::chat::i18n::installed_version(),
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
            let pins = addon_pins::load();
            let components: Vec<Value> = status.components.iter()
                .filter(|c| !matches!(c.component, Component::Locales))
                .map(|c| {
                    json!({
                        "pinned": pins.get(&c.component.manifest_key()).and_then(|p| p.pinned.as_ref()),
                        "component": c.component.manifest_key(),
                        "kind": c.component.kind(),
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
/// - 모듈·익스텐션은 바꿔 끼우고(`spawn_addon_replace`), 앱 구성요소는 받아서 적용한다(`spawn_core_apply`)
async fn check_and_follow_up(state: &UpdateState) -> anyhow::Result<saba_chan_updater_lib::UpdateStatus> {
    check_and_follow_up_hinted(state, state.known_release_id()).await.map(|(status, _)| status)
}

/// `check_and_follow_up`에 heartbeat가 알린 릴리즈 id를 릴레이 캐시 키로 붙인다. 확인 결과와 `/latest` 응답에 담긴 가장 큰 릴리즈 id
async fn check_and_follow_up_hinted(state: &UpdateState, hint: Option<u64>) -> anyhow::Result<(saba_chan_updater_lib::UpdateStatus, Option<u64>)> {
    let (status, latest_id) = state.manager.write().await.check_for_updates_hinted(hint).await?;
    announce_new_updates(&status);

    // 모듈·익스텐션은 묻지 않고 바꿔 끼운다 (사용자 결정 2026-11-09). 확인을 끈 설정이면 주기 확인 자체가 돌지 않는다
    if status.components.iter().any(|c| c.update_available && is_addon(&c.component)) {
        spawn_addon_replace(state.clone());
    }

    let has_locale_update = status.components.iter()
        .any(|c| matches!(c.component, Component::Locales) && c.update_available);
    if has_locale_update {
        let mgr_clone = state.manager.clone();
        tokio::spawn(async move {
            silent_apply_locales(&mgr_clone).await;
        });
    }

    // 앱 구성요소는 묻지 않고 적용한다 (켜진 서버가 있어도 코어를 바꾸고, 막는 것이 있을 때만 기다린다).
    // 자동 적용을 껐으면 `spawn_core_apply`가 받기만 하거나(auto_download) 그대로 두고 준비됨으로 둔다
    let app_update = status.components.iter().any(|c| c.update_available && update_phase::is_app_component(&c.component.manifest_key()));
    if app_update && state.manager.read().await.get_config().enabled {
        spawn_core_apply(state.clone());
    } else if !app_update && state.phase().reason == Some(ReadyReason::Manual) {
        state.set_phase(Phase::default());
    }
    Ok((status, latest_id))
}

fn is_addon(c: &Component) -> bool {
    matches!(c, Component::Module(_) | Component::Extension(_))
}

/// 켜진 서버가 쓰고 있어 미룬 모듈·익스텐션 교체를 다시 해 보는 간격
const ADDON_HOLD_RETRY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// 받지 못했거나 바꾸지 못한 모듈·익스텐션 교체를 다시 해 보는 간격 (잠깐의 네트워크 오류로 새 판이 다음 정기 확인까지 밀리지 않게).
/// 다 쓰면 다음 확인에 맡긴다
const ADDON_FAIL_RETRY: [std::time::Duration; 3] =
    [ADDON_HOLD_RETRY, std::time::Duration::from_secs(30 * 60), std::time::Duration::from_secs(30 * 60)];
static ADDON_REPLACING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 교체 한 차례가 남긴 것
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Leftover {
    /// 켜진 서버가 쓰거나 서버 시작·설치 중이라 미뤘다
    held: bool,
    /// 받지 못했거나 바꾸지 못했다
    failed: bool,
}

/// 남긴 것이 있으면 다음 차례까지 기다릴 시간. `failures`는 잇달아 실패한 차례 수
fn addon_retry_wait(left: Leftover, failures: &mut usize) -> Option<std::time::Duration> {
    if !left.failed {
        *failures = 0;
        return left.held.then_some(ADDON_HOLD_RETRY);
    }
    let wait = ADDON_FAIL_RETRY.get(*failures).copied();
    *failures = failures.saturating_add(1);
    if left.held { Some(ADDON_HOLD_RETRY) } else { wait }
}

/// 다시 해 보려고 기다리는 교체를 깨운다: 그 사이의 확인(사용자가 누른 것 포함)이 남은 잠 뒤로 밀리지 않게
static ADDON_RECHECK: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// 모듈·익스텐션 자동 교체: 새 판(지금 코어로 돌아가는 가장 새 판, 릴레이 `/v1/addons/catalog`)을 받아 바로 바꿔 끼운다.
/// 켜진 서버가 쓰는 것은 서버가 꺼질 때까지 5분마다 다시 보고, 받지 못했거나 바꾸지 못한 것은 5분·30분·30분 뒤에 다시 해 본다.
/// 기다리는 사이에 확인이 또 오면 바로 한 차례 돈다.
/// 사용자가 고친 것·직접 넣은 것은 덮어쓰지 않는다. 한 번에 하나만 돈다
fn spawn_addon_replace(state: UpdateState) {
    use std::sync::atomic::Ordering;
    if ADDON_REPLACING.swap(true, Ordering::SeqCst) {
        ADDON_RECHECK.notify_one();
        return;
    }
    tokio::spawn(async move {
        replace_addons_until_done(|| replace_addons_once(&state), &ADDON_RECHECK).await;
        ADDON_REPLACING.store(false, Ordering::SeqCst);
    });
}

/// 남긴 것이 없어질 때까지(또는 실패를 다 쓸 때까지) 차례를 돌린다. 차례 사이의 잠은 `recheck`가 깨운다
async fn replace_addons_until_done<Fut: std::future::Future<Output = Leftover>>(mut once: impl FnMut() -> Fut, recheck: &tokio::sync::Notify) {
    let mut failures = 0;
    while let Some(wait) = addon_retry_wait(once().await, &mut failures) {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = recheck.notified() => {}
        }
    }
}

/// 바꿀 수 있는 것을 바꾸고, 다음 차례로 남긴 것(켜진 서버 때문에 미룬 것, 실패한 것)을 돌려준다
async fn replace_addons_once(state: &UpdateState) -> Leftover {
    // 미뤄 둔 사이 업데이트 확인을 껐으면 멈춘다. 사용자가 고른 판으로 바꾸기(`addon_pins`의 `pending_version`)는 마저 한다
    let enabled = state.manager.read().await.get_config().enabled;
    let chosen: std::collections::HashSet<String> =
        addon_pins::load().into_iter().filter(|(_, c)| c.pending_version.is_some()).map(|(k, _)| k).collect();
    if !enabled && chosen.is_empty() {
        return Leftover::default();
    }
    // 이 코어로 돌아가지 않는 판(릴리즈 manifest에만 있는 더 새 판)은 앱을 올릴 때까지 둔다: 적용이 requires 검사로 거절한다
    let core = std::collections::HashMap::from([("saba-core".to_string(), env!("CARGO_PKG_VERSION").to_string())]);
    let pending: Vec<Component> = state.manager.read().await.get_status().components.into_iter()
        .filter(|c| c.update_available && is_addon(&c.component))
        .filter(|c| enabled || chosen.contains(&c.component.manifest_key()))
        .filter(|c| c.requires.as_ref().is_none_or(|r| {
            let core_only: std::collections::HashMap<String, String> =
                r.iter().filter(|(k, _)| k.as_str() == "saba-core").map(|(k, v)| (k.clone(), v.clone())).collect();
            saba_chan_updater_lib::unmet_requires(&c.component.manifest_key(), &core_only, &core).is_empty()
        }))
        .map(|c| c.component)
        .collect();
    if pending.is_empty() {
        return Leftover::default();
    }
    let (busy_modules, busy_extensions) = busy_addons(state).await;
    let pending: Vec<Component> = pending.into_iter().filter(|c| replaceable_without_asking(&c.manifest_key())).collect();
    let (ready, held) = split_by_busy(pending, &busy_modules, &busy_extensions);
    if !held.is_empty() {
        tracing::info!("[Updates] Servers are using {:?}; replacing them after they stop", held.iter().map(Component::manifest_key).collect::<Vec<_>>());
    }
    if ready.is_empty() {
        return Leftover { held: !held.is_empty(), failed: false };
    }
    let mut failed = false;
    let mut downloaded = Vec::new();
    for comp in ready {
        // 판마다 잠갔다 놓는다: 판 하나를 받는 동안만 상태 조회·업데이트 화면이 기다린다
        let result = state.manager.write().await.download_component(&comp).await;
        match result {
            Ok(_) => downloaded.push(comp),
            Err(e) => {
                failed = true;
                tracing::warn!("[Updates] Could not download {} for replacement: {}", comp.manifest_key(), e);
            }
        }
    }
    // 여기부터 교체가 끝날 때까지 서버 시작을 막는다: 사용 확인과 교체 사이에 켜진 서버가 쓰던 판을 바꾸지 않게
    // 시작·설치가 진행 중이면 기다리지 않고 다음 차례로 미룬다 (받은 파일은 남는다)
    let Some(_starts) = try_hold_starts(state).await else {
        tracing::info!("[Updates] A server is starting or installing; replacing modules and extensions later");
        return Leftover { held: true, failed };
    };
    // 받는 사이에 켠 서버가 쓰는 것은 바꾸지 않고 다음 차례로 미룬다
    let (busy_modules, busy_extensions) = busy_addons(state).await;
    let (ready, started) = split_by_busy(downloaded, &busy_modules, &busy_extensions);
    let held = !held.is_empty() || !started.is_empty();
    // 받는 사이에 사용자가 고치거나 넣은 파일이 있으면 두고 넘어간다: 교체가 폴더를 비운다.
    // 이 확인 뒤(적용이 업데이트 관리자를 기다리는 동안) 생긴 변경은 적용이 백업을 뜬 순간 한 번 더 보고 멈춘다 (`untouched_only`)
    let keys: Vec<String> = ready.iter().map(Component::manifest_key).filter(|k| replaceable_without_asking(k)).collect();
    if !keys.is_empty() {
        tracing::info!("[Updates] Replacing {:?} without asking", keys);
        let answer = crate::ipc::internal::into_internal(apply_updates_with(state.clone(), ApplyRequest { components: keys, untouched_only: true }).await.into_response()).await.body;
        // 바꾸지 못한 것(교체 순간 사용자 파일이 보였거나, 옮기지 못함)은 업데이트 목록에 남아 사용자가 고른다.
        // 다음 차례는 사용자 파일이 있는 것을 받기 전에 거르므로(`replaceable_without_asking`) 옮기지 못한 것만 다시 해 본다
        if let Some(errors) = answer.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
            failed = true;
            tracing::warn!("[Updates] Some modules or extensions were not replaced: {:?}", errors);
        }
    }
    Leftover { held, failed }
}

/// 업데이터가 코어가 낀 묶음에서 새 코어를 기다리게 둔 모듈·익스텐션(완료 마커 `deferred`)을 마저 바꾼다.
/// 사용자가 고른 묶음이므로 자동 교체와 달리 업데이트 확인 설정·사용자 변경과 상관없이 업데이터처럼 바꾸고,
/// 켜진 서버가 쓰는 것은 서버가 꺼질 때까지 5분마다 다시 본다. 기동 때 한 번 띄운다
pub fn spawn_deferred_apply(state: UpdateState) {
    tokio::spawn(async move {
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        while let Some(wait) = apply_deferred_once(&state, &staging).await {
            tokio::time::sleep(wait).await;
        }
    });
}

/// 한 차례. 다음 차례까지 기다릴 시간을 돌려주고, 할 것이 없으면 None
async fn apply_deferred_once(state: &UpdateState, staging: &std::path::Path) -> Option<std::time::Duration> {
    // 업데이터가 이 데몬을 지켜보는 동안은 바꾸지 않는다: 건강하지 않아 코어만 되돌리면 옛 코어가 거절하는 판이 남는다
    const WATCH_POLL: std::time::Duration = std::time::Duration::from_secs(2);
    if saba_chan_updater_lib::live_apply::watch_in_progress(staging) {
        return Some(WATCH_POLL);
    }
    // 되돌린(`rolled_back`) 뒤나 새 판을 그대로 둔(`left_in_place`) 마커는 하지 않는다: 받아 둔 채 다음 앱 업데이트를 기다린다
    let deferred = saba_chan_updater_lib::UpdateCompletionMarker::load().filter(|m| m.result.is_none())?.deferred;
    let pending: Vec<Component> = {
        let mut mgr = state.manager.write().await;
        restore_pending(&mut mgr);
        mgr.get_pending_components().into_iter().map(|c| c.component.clone()).filter(|c| deferred.contains(&c.manifest_key())).collect()
    };
    if pending.is_empty() {
        return None;
    }
    // 교체가 끝날 때까지 서버 시작을 막는다: 사용 확인과 교체 사이에 켜진 서버가 쓰던 판을 바꾸지 않게
    let Some(_starts) = try_hold_starts(state).await else {
        return Some(ADDON_HOLD_RETRY);
    };
    let (busy_modules, busy_extensions) = busy_addons(state).await;
    let (ready, held) = split_by_busy(pending, &busy_modules, &busy_extensions);
    if !ready.is_empty() {
        let keys: Vec<String> = ready.iter().map(Component::manifest_key).collect();
        tracing::info!("[Updates] Applying {:?}, which waited for this core", keys);
        // 결과는 `record_daemon_apply`가 그 묶음의 완료 마커에 더한다. 실패한 것은 받아 둔 채 업데이트 목록에 남는다
        let _ = apply_updates_with(state.clone(), ApplyRequest { components: keys, ..Default::default() }).await;
    }
    if !held.is_empty() {
        tracing::info!("[Updates] Servers are using {:?}; applying them after they stop", held.iter().map(Component::manifest_key).collect::<Vec<_>>());
        return Some(ADDON_HOLD_RETRY);
    }
    None
}

/// 지금 바꿀 것과 켜진 서버가 써서 미룰 것
fn split_by_busy(
    pending: Vec<Component>,
    busy_modules: &std::collections::HashSet<String>,
    busy_extensions: &std::collections::HashSet<String>,
) -> (Vec<Component>, Vec<Component>) {
    pending.into_iter().partition(|c| match c {
        Component::Module(name) => !busy_modules.contains(name),
        Component::Extension(id) => !busy_extensions.contains(id),
        _ => false,
    })
}

/// 카탈로그 그대로인(Verified) 모듈·익스텐션만 묻지 않고 바꾼다. 사용자가 고쳤거나(Modified) 직접 넣은(Unverified) 것,
/// 설치 기록에 없는 파일(사용자가 넣은 설정·데이터)이 있는 것은 둔다: 교체가 폴더를 비워 사용자의 것을 지운다.
/// 그런 것은 업데이트 목록에 남아 사용자가 고를 수 있다
fn replaceable_without_asking(key: &str) -> bool {
    let Some(dir) = crate::sandbox::component_dir(key) else { return false };
    if !saba_chan_updater_lib::integrity::replaceable_in_place(key, &dir) {
        tracing::info!("[Updates] Not replacing {} automatically: its folder is not exactly what the catalog installed", key);
        return false;
    }
    true
}

/// 켜진 서버가 쓰는 모듈 이름과 익스텐션 id
async fn busy_addons(state: &UpdateState) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut modules = std::collections::HashSet::new();
    let mut extensions = std::collections::HashSet::new();
    for user in running_addon_users(state).await {
        modules.insert(user.module);
        extensions.extend(user.extensions);
    }
    (modules, extensions)
}

/// 켜진 서버 하나가 쓰는 모듈과 익스텐션
struct AddonUser {
    name: String,
    module: String,
    extensions: std::collections::HashSet<String>,
}

/// 켜진 서버마다 쓰는 모듈·익스텐션
async fn running_addon_users(state: &UpdateState) -> Vec<AddonUser> {
    let mut users = Vec::new();
    let mut ext_data = Vec::new();
    if let Some(ref sup) = state.supervisor {
        let sup = sup.read().await;
        let running = sup.running_instances().await;
        for instance in sup.instance_store.list() {
            if running.contains(&instance.id) {
                let extensions = sup.effective_required_extensions(instance).into_iter().collect();
                users.push(AddonUser { name: instance.name.clone(), module: instance.module_name.clone(), extensions });
                ext_data.push((instance.id.clone(), instance.extension_data.clone()));
            }
        }
    }
    // 필수 목록에 없어도 켜진 인스턴스가 칸 값으로 쓰는 익스텐션 (손으로 교체할 때의 `in_use` 판정과 같다). 데몬 락을 놓은 뒤에 본다
    if let Some(ref mgr) = state.extension_manager {
        let mgr = mgr.read().await;
        for (user, (id, data)) in users.iter_mut().zip(&ext_data) {
            user.extensions.extend(mgr.extensions_in_use(&[(id.as_str(), data)]));
        }
    }
    users
}

/// 이 데몬의 서버 시작·설치를 막는다(`AddonSwapGate::try_hold_starts`). 감독자가 없으면 막을 시작도 없다
async fn try_hold_starts(state: &UpdateState) -> Option<tokio::sync::OwnedRwLockWriteGuard<()>> {
    let gate = match &state.supervisor {
        Some(sup) => sup.read().await.addon_gate(),
        None => crate::supervisor::AddonSwapGate::default(),
    };
    gate.try_hold_starts()
}

/// 켜진 서버 중 이 모듈·익스텐션을 쓰는 것의 이름
async fn servers_using(state: &UpdateState, component: &Component) -> Vec<String> {
    let uses = |user: &AddonUser| match component {
        Component::Module(name) => user.module == *name,
        Component::Extension(id) => user.extensions.contains(id),
        _ => false,
    };
    running_addon_users(state).await.into_iter().filter(|u| uses(u)).map(|u| u.name).collect()
}

/// 막는 것이 있어 미룬 앱 업데이트 적용을 다시 판정하는 간격
const CORE_HOLD_RETRY: std::time::Duration = std::time::Duration::from_secs(30);
/// 모듈 호출만 막고 있으면(상태 조회처럼 금방 끝나는 것) 이만큼씩 몇 번 더 보고 나서 기다린다고 알린다
const MODULE_CALL_SETTLE: std::time::Duration = std::time::Duration::from_secs(2);
/// 릴레이에 알리는 업데이트로 끊길 시간(초). 릴레이는 여기에 2분을 더해 기다린다
const UPDATING_ETA_SECS: u64 = 90;
/// heartbeat 판 번호를 확인했는데 응답에 담기지 않았으면 다시 보기까지 (릴레이 `/latest`의 캐시 수명)
const RELEASE_RECHECK: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// 앱 업데이트를 묻지 않고 적용한다: 받아 두고, 막는 것이 없으면 적용하고(데몬이 내려가야 하면 업데이터를 띄운다),
/// 막는 것이 있으면 `waiting`으로 두고 30초마다 다시 본다. 자동으로 할 수 없으면 `ready`. 한 번에 하나만 돈다.
/// 도는 중에 다시 불리면(그사이 확인이 더 새 판을 찾았다) 도는 반복이 그 판을 받는다
fn spawn_core_apply(state: UpdateState) {
    if !state.begin_core_apply() {
        return;
    }
    tokio::spawn(async move {
        loop {
            core_apply(&state).await;
            let stop = state.stop_requested.as_ref().is_some_and(|s| s.is_cancelled());
            if !state.end_core_apply() || state.updater_launched() || stop || !state.begin_core_apply() {
                break;
            }
        }
    });
}

/// 확인이 찾은 앱 구성요소 새 판 중 아직 받지 않은 것을 받는다
async fn download_app_updates(state: &UpdateState) {
    use crate::ipc::internal::into_internal;
    let keys: Vec<String> = state.manager.read().await.get_status().components.iter()
        .filter(|c| c.update_available && !c.downloaded && update_phase::is_app_component(&c.component.manifest_key()))
        .map(|c| c.component.manifest_key())
        .collect();
    if !keys.is_empty() {
        let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest { components: keys }).await).await.body;
        if let Some(errors) = downloaded.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
            tracing::warn!("[Updates] Could not download some updates: {:?}", errors);
        }
    }
}

async fn core_apply(state: &UpdateState) {
    use crate::ipc::internal::into_internal;
    let stop = state.stop_requested.clone().unwrap_or_default();
    loop {
        if state.updater_launched() || stop.is_cancelled() {
            return;
        }
        let cfg = state.manager.read().await.get_config();
        // 처음 차례와, 기다리는 사이 확인이 더 새 판을 찾았을 때 받는다. '확인만'(받기·적용 모두 끔)이면 받지 않는다
        if state.take_apply_recheck() && (cfg.auto_apply || cfg.auto_download) {
            download_app_updates(state).await;
        }
        // 자동 적용을 껐다: 사용자가 다시 시작을 누를 때 적용한다 (받아 두지 않았으면 그때 받는다)
        if !cfg.auto_apply {
            hold_for_restart(state).await;
            return;
        }
        let (decision, version, notes) = plan_core_apply(state).await;
        match decision {
            Decision::Nothing => {
                // 기다리던 것이 사라졌으면(받아 둔 것을 적용했거나 지웠다) 걷는다. 되돌림·실패한 판의 준비됨은 사용자가 고를 때까지 둔다
                let phase = state.phase();
                if phase.state == PhaseState::Waiting || phase.reason == Some(ReadyReason::GuiWindowOpen) {
                    state.set_phase(Phase::default());
                }
                return;
            }
            Decision::Held => {
                clear_held_phase(state);
                return;
            }
            Decision::Ready(reason) => {
                state.set_phase(Phase::ready(reason, version, notes));
                // 창을 숨기면(트레이) 이어서 적용한다
                if reason != ReadyReason::GuiWindowOpen {
                    return;
                }
                hold_while_gui_window_open(state, &stop).await;
                continue;
            }
            Decision::Wait(blockers) => {
                tracing::info!("[Updates] Waiting for {:?} before applying {:?}", blockers.iter().map(|b| (b.kind, &b.name)).collect::<Vec<_>>(), version);
                state.set_phase(Phase { waiting_for: blockers, ..Phase::new(PhaseState::Waiting, version, notes) });
            }
            Decision::Apply(targets) => {
                tracing::info!("[Updates] Applying {:?} without asking", targets);
                state.set_phase(Phase::new(PhaseState::Applying, version.clone(), notes.clone()));
                let answer = into_internal(apply_updates_with(state.clone(), ApplyRequest { components: targets, ..Default::default() }).await).await.body;
                if answer.get("requires_updater").and_then(Value::as_bool) == Some(true) {
                    let launched = into_internal(launch_updater(State(state.clone()), LooseJson(None)).await).await.body;
                    if launched.get("ok").and_then(Value::as_bool) != Some(true) {
                        tracing::warn!("[Updates] Could not start the updater: {}", launched);
                        state.set_phase(Phase::default());
                    }
                } else if answer.get("ok").and_then(Value::as_bool) == Some(true) {
                    // 데몬이 내려가지 않고 바꾼 것(CLI·설치기)
                    state.set_phase(Phase::new(PhaseState::Done, version, notes));
                } else {
                    tracing::warn!("[Updates] Applying updates failed: {}", answer);
                    state.set_phase(Phase::default());
                }
                return;
            }
        }
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(CORE_HOLD_RETRY) => {}
        }
    }
}

/// GUI 창이 열려 있어 미룬 적용: 다시 판정할 일이 생길 때까지 기다린다. 창을 며칠 열어 두어도 30초마다
/// 전체 판정(업데이트 관리자 쓰기 잠금, 프로세스 목록 훑기)을 하지 않게, 그동안은 판정을 바꿀 것만 가볍게 본다:
/// 창을 숨겼거나 새 판을 찾았다는 알림(`apply_wake`), GUI 등록이 사라짐, 자동 적용 끔, 받아 둔 것이 없어짐
async fn hold_while_gui_window_open(state: &UpdateState, stop: &tokio_util::sync::CancellationToken) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = state.apply_wake.notified() => {}
            _ = tokio::time::sleep(CORE_HOLD_RETRY) => {}
        }
        // 이 반복을 띄울 때 남은 알림은 지나친다 (다시 볼 일은 `apply_recheck`에 남는다)
        if state.apply_recheck.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let unchanged = {
            let mgr = state.manager.read().await;
            mgr.get_config().auto_apply && !mgr.get_pending_components().is_empty()
        };
        if !unchanged || state.updater_launched() || !state.gui_window_open().await {
            return;
        }
    }
}

/// 확인해 둔 코어 새 판 (없으면 None)
fn core_update(status: &saba_chan_updater_lib::UpdateStatus) -> Option<&str> {
    status.components.iter().find(|c| c.component == Component::CoreDaemon && c.update_available).and_then(|c| c.latest_version.as_deref())
}

/// 사용자가 되돌려 고정한 코어: 앞서 남은 준비됨·기다림을 걷는다 (고정한 판은 알약에 올리지 않는다)
fn clear_held_phase(state: &UpdateState) {
    if matches!(state.phase().state, PhaseState::Ready | PhaseState::Waiting) {
        state.set_phase(Phase::default());
    }
}

/// 자동 적용을 끈 설정: 받을 앱 업데이트가 있으면 `ready`(`manual`)로 두고, 없으면 그 단계를 걷는다.
/// 받을 코어가 사용자가 되돌려 고정한 판이면 준비됨으로 두지 않는다 (`decide`의 `Held`와 같다)
async fn hold_for_restart(state: &UpdateState) {
    let (available, held, release) = {
        let mgr = state.manager.read().await;
        let status = mgr.get_status();
        let available = status.components.iter().any(|c| c.update_available && update_phase::is_app_component(&c.component.manifest_key()));
        let cfg = mgr.get_config();
        let held = cfg.user_held && update_phase::is_held_core(cfg.failed_release.as_deref(), core_update(&status));
        (available, held, mgr.applying_release())
    };
    if held {
        clear_held_phase(state);
    } else if available {
        let version = release.as_ref().map(|r| r.version.clone());
        state.set_phase(Phase::ready(ReadyReason::Manual, version, release.map(|r| r.notes).filter(|n| !n.is_empty())));
    } else if state.phase().reason == Some(ReadyReason::Manual) {
        state.set_phase(Phase::default());
    }
}

/// 받아 둔 앱 업데이트를 지금 적용할 수 있는가 (`update_phase::decide`)와 그 판·패치노트
async fn plan_core_apply(state: &UpdateState) -> (Decision, Option<String>, Option<std::collections::HashMap<String, String>>) {
    let (pending, release, core_version, failed, user_held, reads) = {
        let mut mgr = state.manager.write().await;
        restore_pending(&mut mgr);
        let pending: Vec<String> = mgr.get_pending_components().iter().map(|c| c.component.manifest_key()).collect();
        // 되돌린 기록은 코어 판으로 견준다: 부분 릴리즈는 안 바뀐 코어를 지난 판 번호 그대로 싣는다
        let core_version = mgr.get_pending_components().iter().find(|c| c.component == Component::CoreDaemon).and_then(|c| c.latest_version.clone());
        let release = mgr.applying_release();
        let mut cfg = mgr.get_config();
        // 되돌린 코어보다 새 코어를 받았으면 그 기록은 지운다 (코어가 그대로인 새 릴리즈는 지우지 않는다)
        if let (Some(failed), Some(core)) = (cfg.failed_release.as_deref(), core_version.as_deref()) {
            if update_phase::failed_release_outdated(failed, core) {
                cfg.clear_failed_release();
                if let Err(e) = save_updater_config(&cfg) {
                    tracing::warn!("[Updates] Failed to persist update config: {}", e);
                }
                mgr.update_config(cfg.clone());
            }
        }
        let reads = mgr.core_reads_runner_formats().unwrap_or(saba_chan_updater_lib::relay::DEFAULT_READS_RUNNER_FORMATS);
        (pending, release, core_version, cfg.failed_release, cfg.user_held, reads)
    };
    let version = release.as_ref().map(|r| r.version.clone());
    let notes = release.map(|r| r.notes).filter(|n| !n.is_empty());
    let gui_window_open = state.gui_window_open().await;
    let cli_running = tokio::task::spawn_blocking(saba_chan_updater_lib::ProcessChecker::is_cli_running).await.unwrap_or(false);
    let mut blockers = core_blockers(state, reads).await;
    // 금방 끝나는 모듈 호출(상태 조회 등)만 걸렸으면 잠깐 더 본다
    for _ in 0..3 {
        if blockers.is_empty() || blockers.iter().any(|b| b.kind != crate::supervisor::core_swap::CoreSwapBlockerKind::ModuleCall) {
            break;
        }
        tokio::time::sleep(MODULE_CALL_SETTLE).await;
        blockers = core_blockers(state, reads).await;
    }
    // 이전 데몬이 끝나지 않아 미룬 묶음: 같은 릴리즈면 사용자가 다시 시작할 때까지 둔다
    let phase = state.phase();
    let deferred = (phase.state == PhaseState::Ready && phase.reason == Some(ReadyReason::WaitPidTimeout)).then_some(phase.version).flatten();
    let decision = update_phase::decide(update_phase::PlanInput {
        pending: &pending,
        version: version.as_deref(),
        core_version: core_version.as_deref(),
        failed_release: failed.as_deref(),
        user_held,
        deferred_release: deferred.as_deref(),
        gui_window_open,
        cli_running,
        blockers,
    });
    (decision, version, notes)
}

/// 코어를 바꾸면 끊기는 것: 데몬 작업과 다시 붙지 못하는 켜진 서버 (한 곳에서 모은다)
async fn core_blockers(state: &UpdateState, new_core_reads: (u32, u32)) -> Vec<crate::supervisor::core_swap::CoreSwapBlocker> {
    let Some(ref sup) = state.supervisor else {
        return update_phase::daemon_work(|_| None);
    };
    let sup = sup.read().await;
    let mut blockers = update_phase::daemon_work(|id| sup.instance_store.get(id).map(|i| i.name.clone()));
    blockers.extend(sup.core_swap_blockers(new_core_reads).await);
    blockers
}

/// GET /api/daemon/update — 짝 맺은 PC의 앱 카드가 이 데몬의 판·업데이트를 본다 (`/api/updates/*`는 앱이 있는 PC 전용이라 따로 둔다).
/// `{ok, version, profile, auto, updatesAvailable, latestVersion}` (`latestVersion`: 데몬 구성요소의 새 판, 없으면 null)
async fn get_daemon_update(State(state): State<UpdateState>) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let cfg = mgr.get_config();
    let status = mgr.get_status();
    let visible = status.components.iter().filter(|c| c.update_available && !matches!(c.component, Component::Locales));
    let latest = status.components.iter().find(|c| matches!(c.component, Component::CoreDaemon) && c.update_available).and_then(|c| c.latest_version.clone());
    Json(json!({
        "ok": true,
        "version": crate::relay_agent::AGENT_VERSION,
        "profile": saba_chan_updater_lib::profile::Profile::load().as_str(),
        "auto": cfg.enabled && cfg.auto_apply,
        "updatesAvailable": visible.count(),
        "latestVersion": latest,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DaemonUpdateBody {
    auto: bool,
}

/// PUT /api/daemon/update {auto} — 자동 업데이트(확인·받기·적용)를 켜고 끈다. 켜면 바로 한 번 확인한다
async fn put_daemon_update(State(state): State<UpdateState>, Json(body): Json<DaemonUpdateBody>) -> impl IntoResponse {
    let cfg = {
        let mut mgr = state.manager.write().await;
        let mut cfg = mgr.get_config();
        if body.auto {
            cfg.enabled = true;
            cfg.auto_download = true;
        }
        cfg.auto_apply = body.auto;
        mgr.update_config(cfg.clone());
        cfg
    };
    if let Err(e) = save_updater_config(&cfg) {
        tracing::warn!("[Updates] Failed to persist update config: {}", e);
    }
    if body.auto {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = check_and_follow_up(&state).await {
                tracing::warn!("[Updates] Check after enabling auto-update failed: {}", e);
            }
        });
    }
    Json(json!({ "ok": true, "auto": body.auto }))
}

/// 데몬 기동 후 업데이트를 주기적으로 확인한다 (U6). 24시간마다.
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
            // 연결된 데몬은 heartbeat의 판 번호로 알게 된다. 이것은 연결과 상관없는 안전망이다 (`check_interval_hours`는 쓰지 않는다)
            wait = match check_and_follow_up(&state).await {
                Ok(_) => std::time::Duration::from_secs(u64::from(saba_chan_updater_lib::CHECK_EVERY_HOURS) * 3600),
                Err(e) => {
                    tracing::warn!("[Updates] Scheduled check failed: {}", e);
                    RETRY
                }
            };
        }
    });
}

/// 릴레이의 새 릴리즈 알림(하트비트 응답 `updateHint: { version, channel }`)이 확인할 만한가:
/// 고른 채널이 그 릴리즈 채널을 받고(`channel_includes`), 지금 버전보다 새 버전이어야 한다.
/// 베타 채널을 쓰지 않는 데몬은 베타 릴리즈 알림에 아무것도 하지 않는다
fn hint_wants_check(hint: &Value, channel: &str, current: &str) -> bool {
    use saba_chan_updater_lib::{relay::{channel_includes, CHANNELS}, version::SemVer};
    let (Some(version), Some(release)) = (hint.get("version").and_then(Value::as_str), hint.get("channel").and_then(Value::as_str)) else {
        return false;
    };
    if !CHANNELS.contains(&release) || !channel_includes(channel, release) {
        return false;
    }
    matches!((SemVer::parse(version), SemVer::parse(current)), (Some(v), Some(c)) if v > c)
}

/// 릴레이가 새 릴리즈를 알렸다 (`relay_agent` 하트비트). 대조해서 맞으면 주기 확인과 같은 길(`check_and_follow_up`)로
/// 한 번 확인한다. 업데이트 확인을 꺼 두었으면 하지 않는다.
/// 모든 데몬이 한꺼번에 릴레이에 묻지 않게 0~60초 사이에서 늦춘 뒤 그때의 설정으로 대조한다
pub fn on_update_hint(state: &UpdateState, hint: &Value, shutdown: tokio_util::sync::CancellationToken) {
    let hint = hint.clone();
    let state = state.clone();
    tokio::spawn(async move {
        let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_millis()) as u64 * 60;
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_millis(jitter)) => {}
        }
        // 기다리는 사이 설정을 바꿨을 수 있다 — 기다린 뒤의 설정으로 대조한다
        let cfg = state.manager.read().await.get_config();
        if !cfg.enabled || !hint_wants_check(&hint, &cfg.channel, crate::relay_agent::AGENT_VERSION) {
            return;
        }
        if let Err(e) = check_and_follow_up(&state).await {
            tracing::warn!("[Updates] Check after relay release notice failed: {}", e);
        }
    });
}

/// 릴레이 heartbeat의 채널별 최신 릴리즈 id(`latestReleases`). 이 데몬이 받는 채널(같은 판 베타 규칙 포함)의 값이
/// 지난번 본 것과 다르면 0~60초 사이에서 늦춘 뒤 한 번 확인한다. 같으면 아무것도 묻지 않는다
pub fn on_latest_releases(state: &UpdateState, ids: &Value, shutdown: tokio_util::sync::CancellationToken) {
    let ids = ids.clone();
    let state = state.clone();
    tokio::spawn(async move {
        let channel = state.manager.read().await.get_config().channel;
        let watched = update_phase::watched_release_ids(&ids, &channel, crate::relay_agent::AGENT_VERSION);
        let Some(checking) = state.release_ids_to_check(watched) else {
            return;
        };
        let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_millis()) as u64 * 60;
        let checked = async {
            tokio::select! {
                _ = shutdown.cancelled() => return None,
                _ = tokio::time::sleep(std::time::Duration::from_millis(jitter)) => {}
            }
            if !state.manager.read().await.get_config().enabled {
                return None;
            }
            // 알린 번호를 캐시 키로 붙인다: 번호가 바뀌면 캐시된 옛 응답이 아니라 새 응답을 받는다
            let hint = checking.values().copied().max();
            tokio::select! {
                _ = shutdown.cancelled() => None,
                result = check_and_follow_up_hinted(&state, hint) => match result {
                    Ok((_, latest_id)) => Some(latest_id),
                    Err(e) => {
                        tracing::warn!("[Updates] Check after a new release id failed: {}", e);
                        None
                    }
                },
            }
        }
        .await;
        // 성공한 확인만 본 것으로 남긴다: 실패·취소·꺼짐이면 다음 heartbeat가 같은 번호로 다시 확인한다
        state.finish_release_check(checking, checked);
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
    /// 묻지 않는 교체(`replace_addons_once`): 교체 순간의 폴더가 카탈로그 그대로인 모듈·익스텐션만 바꾼다. API로는 받지 않는다
    #[serde(skip)]
    untouched_only: bool,
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
            // 바꿀 폴더에서 떠 있는 작업자는 끝낸다 (Windows는 작업 폴더로 쓰는 폴더를 옮기지 못한다). 다음 호출이 새 판으로 띄운다.
            // 익스텐션은 다른 구성요소의 작업자가 import해 두었을 수 있어 모든 작업자를 끝낸다 (`utils::swap_dir`와 같다)
            match comp {
                Component::Module(name) => crate::plugin::stop_workers(Some(&crate::plugin::resolve_modules_dir().join(name))),
                Component::Extension(_) => crate::plugin::stop_workers(None),
                _ => {}
            }
            let applying = if body.untouched_only { mgr.apply_untouched_addon(comp).await } else { mgr.apply_single_component(comp).await };
            match applying {
                Ok(result) if result.success => {
                    // 고른 판으로 바꾸기를 기다리던 것이면 기다림을 지운다 (바뀐 판이 무엇이든 그 기다림은 끝났다)
                    if is_addon(comp) {
                        if let Err(e) = addon_pins::update(&comp.manifest_key(), |c| c.pending_version = None) {
                            tracing::warn!("[Updates] Could not clear the chosen version of {}: {}", comp.manifest_key(), e);
                        }
                    }
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
        let target_keys: Vec<String> = targets.iter().map(Component::manifest_key).collect();
        let repair = mgr.is_repair(&target_keys);
        record_daemon_apply(&saba_chan_updater_lib::UpdateCompletionMarker::path(), &target_keys, &applied_keys, &errors, mgr.applying_release(), repair);
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
            // 새 판이 요구하는 익스텐션은 위 결과 마커를 보고 따로 맞춘다 (받는 동안 답을 붙잡지 않는다)
            state.modules_updated.notify_one();
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

/// 데몬을 다시 켠 뒤 아직 확인 전이면(상태가 비어 있음) 지난번에 받아 둔 목록(`pending.json`)을 되살린다.
/// 이미 확인했으면 그 상태를 그대로 쓴다 (확인이 받아 둔 파일을 알아본다)
fn restore_pending(mgr: &mut saba_chan_updater_lib::UpdateManager) {
    if mgr.get_status().components.is_empty() {
        let _ = mgr.load_pending_manifest();
    }
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
/// Body: `{ "relaunch": ["<exe>", "추가 인자"...], "gui_stays": bool }` — 적용 후 다시 띄울 프로그램(생략하면 업데이터가 GUI를 찾는다).
/// `gui_stays`: 부른 GUI가 `gui_restart:false`를 보고 남을 줄 안다 (모르는 옛 GUI는 성공 응답에 늘 끝난다)
#[derive(Deserialize, Default)]
struct LaunchUpdaterRequest {
    #[serde(default)]
    relaunch: Vec<String>,
    #[serde(default)]
    gui_stays: bool,
}

async fn launch_updater(
    State(state): State<UpdateState>,
    LooseJson(body): LooseJson<LaunchUpdaterRequest>,
) -> axum::response::Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let targets = state.manager.read().await.updater_targets();
    if targets.is_empty() {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "No downloaded updates to apply" })),
        )
            .into_response();
    }
    hand_off_to_updater(state, body, targets, false).await
}

/// GET /api/updates/rollback — 되돌릴 수 있는 직전 판 (`live_apply::PreviousVersion`)
async fn get_rollback(State(state): State<UpdateState>) -> impl IntoResponse {
    let previous = saba_chan_updater_lib::live_apply::PreviousVersion::load(&saba_chan_updater_lib::constants::resolve_data_dir());
    let mut body = rollback_body(previous.as_ref(), &saba_chan_updater_lib::relay::core_version());
    if let Some(version) = body["version"].as_str().map(str::to_string) {
        body["needs_newer_core"] = json!(addons_needing_core(&version, installed_addon_requires(&state).await));
    }
    Json(body)
}

/// `{ok, available, current}`, 되돌릴 수 있으면 `version`(돌아갈 판)·`updated_at`(지금 판으로 업데이트한 때)도
fn rollback_body(previous: Option<&saba_chan_updater_lib::live_apply::PreviousVersion>, current: &str) -> Value {
    match kept_previous(previous, current) {
        Some(p) => json!({ "ok": true, "available": true, "version": p.version, "current": current, "updated_at": p.updated_at }),
        None => json!({ "ok": true, "available": false, "current": current }),
    }
}

/// 되돌릴 수 있는 직전 판: 지금 코어(`current`)가 그 직전 판을 바꾼 판일 때만 (그 뒤 설치기로 다시 깔았으면 맞지 않는다)
fn kept_previous<'a>(
    previous: Option<&'a saba_chan_updater_lib::live_apply::PreviousVersion>,
    current: &str,
) -> Option<&'a saba_chan_updater_lib::live_apply::PreviousVersion> {
    previous.filter(|p| p.replaced_by == current)
}

/// GET /api/updates/versions — 앱의 "다른 버전 선택": 지금 판, 이 PC에 남긴 직전 판, 가장 새 판, 사용자 고정 (네트워크 없음)
async fn get_versions(State(state): State<UpdateState>) -> impl IntoResponse {
    let installed = saba_chan_updater_lib::relay::core_version();
    let previous = saba_chan_updater_lib::live_apply::PreviousVersion::load(&saba_chan_updater_lib::constants::resolve_data_dir());
    let (cfg, latest, checked) = {
        let mgr = state.manager.read().await;
        let status = mgr.get_status();
        (mgr.get_config(), core_update(&status).map(str::to_string), status.last_check.is_some())
    };
    let needs_newer_core = match kept_previous(previous.as_ref(), &installed) {
        Some(p) => addons_needing_core(&p.version, installed_addon_requires(&state).await),
        None => Vec::new(),
    };
    Json(versions_body(&cfg, &installed, previous.as_ref(), needs_newer_core, latest.as_deref(), checked))
}

/// `{ok, channel, installed, kept_previous, latest, held_until_newer_than}`.
/// `kept_previous`: 되돌릴 수 있는 직전 판 `{version, updated_at, needs_newer_core, hold_until_newer_than}` (`GET /api/updates/rollback`과
/// 같은 규칙). `hold_until_newer_than`은 되돌리면 고정할 기준(`hold_baseline`).
/// `latest`: 확인한 코어 새 판(`latest_core`). 아직 확인하지 않았으면(`checked`가 거짓) 고정한 판. 지금 판보다 새 판만.
/// `held_until_newer_than`: 사용자가 되돌려 고정한 기준 (`user_held`). 그 판이나 더 새 판을 깔았거나 더 새 코어가 나왔으면 남은 기록이라 싣지 않는다
fn versions_body(
    cfg: &saba_chan_updater_lib::UpdateConfig,
    installed: &str,
    previous: Option<&saba_chan_updater_lib::live_apply::PreviousVersion>,
    needs_newer_core: Vec<String>,
    latest_core: Option<&str>,
    checked: bool,
) -> Value {
    let newer = |v: &&str| update_phase::failed_release_outdated(installed, v);
    let latest_core = latest_core.filter(newer);
    let held = cfg
        .failed_release
        .as_deref()
        .filter(|_| cfg.user_held)
        .filter(newer)
        .filter(|held| !latest_core.is_some_and(|latest| update_phase::failed_release_outdated(held, latest)));
    json!({
        "ok": true,
        "channel": cfg.channel,
        "installed": installed,
        "kept_previous": kept_previous(previous, installed).map(|p| json!({
            "version": p.version,
            "updated_at": p.updated_at,
            "needs_newer_core": needs_newer_core,
            "hold_until_newer_than": hold_baseline(installed, latest_core),
        })),
        "latest": latest_core.or(held.filter(|_| !checked)),
        "held_until_newer_than": held,
    })
}

/// 지금 되돌리면 고정할 기준: 이미 아는 더 새 코어(`latest_core`), 없으면 지금 판. 그 판보다 새 코어가 나와야 고정이 풀린다
/// (되돌리자마자 이미 나와 있던 판으로 다시 올라가지 않게)
fn hold_baseline<'a>(installed: &'a str, latest_core: Option<&'a str>) -> &'a str {
    latest_core.filter(|latest| update_phase::failed_release_outdated(installed, latest)).unwrap_or(installed)
}

/// 설치된 모듈·익스텐션 키와 그 `saba-core` 요구 (모듈 `[module.dependencies]`, 익스텐션 `dependencies`·`min_app_version`)
async fn installed_addon_requires(state: &UpdateState) -> Vec<(String, std::collections::HashMap<String, String>)> {
    let core = crate::supervisor::module_loader::CORE_COMPONENT.to_string();
    let mut addons = Vec::new();
    if let Some(sup) = &state.supervisor
        && let Ok(modules) = sup.read().await.list_modules()
    {
        addons.extend(modules.into_iter().map(|m| (format!("module-{}", m.metadata.name), m.metadata.dependencies)));
    }
    if let Some(ext) = &state.extension_manager {
        addons.extend(ext.read().await.list().into_iter().map(|e| {
            let mut requires = e.dependencies;
            if let Some(min) = e.min_app_version {
                requires.entry(core.clone()).or_insert(min);
            }
            (format!("ext-{}", e.id), requires)
        }));
    }
    addons
}

/// `version` 코어가 읽지 않을 것: `saba-core` 요구를 채우지 못하는 모듈·익스텐션 키. 사용자 되돌리기는 실행 파일만 되돌려
/// 이것들은 새 판으로 남는다
fn addons_needing_core(version: &str, addons: Vec<(String, std::collections::HashMap<String, String>)>) -> Vec<String> {
    let core = crate::supervisor::module_loader::CORE_COMPONENT.to_string();
    let installed = std::collections::HashMap::from([(core.clone(), version.to_string())]);
    addons
        .into_iter()
        .filter(|(key, requires)| {
            let needs: std::collections::HashMap<String, String> = requires.get(&core).map(|min| [(core.clone(), min.clone())].into()).unwrap_or_default();
            !saba_chan_updater_lib::unmet_requires(key, &needs, &installed).is_empty()
        })
        .map(|(key, _)| key)
        .collect()
}

/// POST /api/updates/rollback — 남겨 둔 직전 판으로 되돌린다: 업데이터를 `--rollback`으로 띄우고 데몬은 꺼진다(`launch-updater`와 같은 길).
/// 업데이터가 직전 판의 실행 파일과 데이터를 되돌리고 지금 판을 `failed_release`에 적은 뒤 다시 띄운다. 결과는 `last-result`(`result: "reverted"`).
/// Body는 `launch-updater`와 같다. 남긴 판이 없으면 409 `no_previous_version`
async fn roll_back(State(state): State<UpdateState>, LooseJson(body): LooseJson<LaunchUpdaterRequest>) -> axum::response::Response {
    let previous = saba_chan_updater_lib::live_apply::PreviousVersion::load(&saba_chan_updater_lib::constants::resolve_data_dir());
    roll_back_with(state, body.map(|Json(b)| b).unwrap_or_default(), previous).await
}

async fn roll_back_with(
    state: UpdateState,
    body: LaunchUpdaterRequest,
    previous: Option<saba_chan_updater_lib::live_apply::PreviousVersion>,
) -> axum::response::Response {
    let installed = saba_chan_updater_lib::relay::core_version();
    let Some(mut previous) = previous.filter(|p| kept_previous(Some(p), &installed).is_some()) else {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "No previous version is kept", "error_code": "no_previous_version" })),
        )
            .into_response();
    };
    // 업데이터가 되돌린 뒤 적을 고정 기준 (`hold_baseline`): 지금 판이 아니면 직전 판 기록에 남겨 넘긴다
    let hold = {
        let status = state.manager.read().await.get_status();
        Some(hold_baseline(&installed, core_update(&status))).filter(|hold| *hold != installed).map(str::to_string)
    };
    if previous.hold != hold
        && let Err(e) = previous.hold_until(hold)
    {
        tracing::warn!("[Updates] Cannot record the version to hold until: {}", e);
    }
    // 되돌리는 실행 파일에 GUI가 있으면 떠 있는 GUI를 끄고 다시 띄운다 (`gui_must_restart`가 보는 키)
    let targets = previous
        .exes
        .iter()
        .map(|(_, exe)| if is_gui_exe(&exe.to_string_lossy()) { "gui".to_string() } else { exe.to_string_lossy().into_owned() })
        .collect();
    hand_off_to_updater(state, body, targets, true).await
}

/// 업데이터를 띄우고 데몬을 끈다. `targets`는 바꿀 구성요소 키(GUI를 다시 띄울지 정한다), `rollback`이면 적용 대신 직전 판으로 되돌린다
async fn hand_off_to_updater(state: UpdateState, body: LaunchUpdaterRequest, targets: Vec<String>, rollback: bool) -> axum::response::Response {
    let Some(updater) = updater_binary() else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "saba-chan-installer not found next to the daemon" })),
        )
            .into_response();
    };
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
    // 떠 있는 GUI는 실행 파일을 쥐고 있어 바꿀 수 없다. GUI가 바뀌면 GUI가 부른 게 아니어도(CLI·자동 업데이트) 데몬이 끄고
    // 업데이터가 같은 GUI를 다시 띄우게 한다(GUI가 뜨면서 데몬도 띄운다). GUI가 불렀으면 그 GUI는 스스로 끝난다.
    // GUI가 그대로면 GUI는 두고 데몬만 다시 띄운다: 떠 있는 GUI가 다시 뜬 데몬에 붙는다
    // GUI가 창을 숨겼다고 알렸으면(트레이) 트레이로 다시 띄운다
    let hidden = *state.gui_window.lock().unwrap_or_else(|e| e.into_inner()) == Some(false);
    let (guis, resident) = match &state.clients {
        Some(clients) => {
            let list = clients.list().await;
            let resident = only_resident_guis(&list) || (hidden && list.iter().any(|(_, c)| c.kind == crate::ipc::ClientKind::Gui));
            (running_guis(&list), resident)
        }
        None => (Vec::new(), false),
    };
    let daemon_line = std::env::current_exe()
        .map(|me| vec![me.to_string_lossy().into_owned(), "--spawned".to_string()])
        .unwrap_or_default();
    let restart = gui_must_restart(&targets, &body.relaunch, body.gui_stays);
    let (guis, relaunch) = restart_plan(restart, body.relaunch, &guis, resident, daemon_line);
    let gui_restart = !guis.is_empty() || relaunch.first().is_some_and(|exe| is_gui_exe(exe));
    let mut args = updater_args(ticket, saba_chan_updater_lib::profile::Profile::load(), relaunch, saba_chan_updater_lib::autostart::service_relaunch());
    if rollback {
        args.insert(1, "--rollback".to_string());
    }
    // 바로 끝난 업데이터(라이브러리 없음·백신 차단)는 데몬을 내리기 전에 실패로 돌린다. --wait-pid로 데몬을 기다리므로 살아 있어야 한다
    if let Err(e) = spawn_and_confirm(&updater, &args, std::time::Duration::from_millis(700)).await {
        state.updater_launched.store(false, Ordering::SeqCst);
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("Failed to start updater: {}", e) })),
        )
            .into_response();
    }
    tracing::info!("[Updates] Launched updater {} {:?}; daemon is shutting down", updater.display(), args);
    if !rollback && state.phase().state != PhaseState::Applying {
        let release = state.manager.read().await.applying_release();
        state.set_phase(Phase::new(PhaseState::Applying, release.as_ref().map(|r| r.version.clone()), release.map(|r| r.notes).filter(|n| !n.is_empty())));
    }
    // 붙은 GUI에 데몬이 곧 내려가 바뀐다고 알린다 (GUI가 부르지 않은 자동 적용도). 남는 GUI는 그동안 데몬을 띄우거나
    // 꺼짐을 알리지 않고 다시 뜬 데몬에 붙는다. `closingGuis`의 GUI는 아래에서 끄기 전에 스스로 끝난다
    crate::events::emit("update.restarting", json!({ "closingGuis": guis.iter().map(|(pid, _)| *pid).collect::<Vec<_>>() }));

    if let Some(stop) = state.stop_requested.clone() {
        let relay = state.relay.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await; // 응답이 나가도록
            // 릴레이가 "PC 꺼짐"으로 보지 않고 디스코드 명령에 "업데이트 중"으로 답하게 (실패해도 진행한다)
            if let Some(relay) = relay {
                if let Err(e) = relay.notify_updating(UPDATING_ETA_SECS).await {
                    tracing::info!("[Updates] Could not tell the relay about the update: {}", e);
                }
            }
            close_guis(&guis).await;
            stop.cancel();
        });
    }
    Json(json!({ "ok": true, "updater_launched": true, "daemon_stopping": true, "gui_restart": gui_restart })).into_response()
}

/// 업데이터 실행 인자. 헤드리스 설치는 창 없이 적용하게 `--headless`를 붙이고,
/// 다시 띄울 것을 받지 않았으면 데몬 자신을 다시 띄우게 한다 (GUI가 없어 데몬을 띄워 줄 쪽이 없다).
/// 데몬이 자동 실행 서비스로 돌면(`service`, 화면 없는 리눅스) 받은 것과 상관없이 그 서비스를 다시 시작하게 한다
fn updater_args(ticket: String, profile: saba_chan_updater_lib::profile::Profile, mut relaunch: Vec<String>, service: Option<Vec<String>>) -> Vec<String> {
    let mut args = vec![
        "--apply".to_string(),
        "--wait-pid".to_string(),
        std::process::id().to_string(),
        "--launch-token".to_string(),
        ticket,
    ];
    if profile == saba_chan_updater_lib::profile::Profile::Headless {
        args.push("--headless".to_string());
        // 서비스로 돈 데몬은 CLI가 데몬을 다시 띄우라고 해도 서비스로 다시 띄운다 (서비스 밖에서 돌지 않게)
        if let Some(service) = service {
            relaunch = service;
        } else if relaunch.is_empty() {
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
    let Ok(mut sys) = sysinfo::System::new() else {
        return Vec::new();
    };
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
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

/// 업데이터를 띄우기 전에 끌 GUI와 적용 뒤 다시 띄울 것. GUI가 바뀌지 않으면 GUI는 끄지 않고 데몬(`daemon`)만 다시 띄운다
/// (요청이 GUI가 아닌 것을 골랐으면 그것). 트레이만 든 GUI는 데몬이 사라지면 따라 끝나므로 바뀌지 않아도 끄고 트레이로 다시 띄운다
fn restart_plan(
    gui_updated: bool,
    requested: Vec<String>,
    guis: &[(u32, std::path::PathBuf)],
    resident: bool,
    daemon: Vec<String>,
) -> (Vec<(u32, std::path::PathBuf)>, Vec<String>) {
    if gui_updated || resident {
        return (guis.to_vec(), relaunch_target(requested, guis, resident));
    }
    let relaunch = match requested.first() {
        Some(exe) if !is_gui_exe(exe) => requested,
        _ => daemon,
    };
    (Vec::new(), relaunch)
}

/// GUI를 끄고 다시 띄워야 하는가: GUI가 적용 대상이거나, 부른 GUI가 남을 줄 모른다(옛 GUI는 성공 응답에 끝나므로 다시 띄워 준다)
fn gui_must_restart(targets: &[String], requested: &[String], gui_stays: bool) -> bool {
    targets.iter().any(|k| k == "gui") || (!gui_stays && requested.first().is_some_and(|exe| is_gui_exe(exe)))
}

fn is_gui_exe(exe: &str) -> bool {
    let name = std::path::Path::new(exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    saba_chan_updater_lib::GUI_EXE_NAMES.iter().any(|g| g.eq_ignore_ascii_case(&name))
}

/// 업데이터가 다시 띄울 것: 떠 있던 GUI가 있으면 그 GUI(요청이 이미 GUI면 그대로), 없으면 요청대로.
/// 떠 있던 GUI가 트레이만 들고 있었으면(`resident`) 트레이 모드로 다시 띄운다
fn relaunch_target(requested: Vec<String>, guis: &[(u32, std::path::PathBuf)], resident: bool) -> Vec<String> {
    match guis.first() {
        Some((_, exe)) if !requested.first().is_some_and(|r| is_gui_exe(r)) => {
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
        let pid = *pid;
        if let Err(e) = tokio::task::spawn_blocking(move || force_kill_pid(pid)).await.unwrap_or_else(|e| Err(e.into())) {
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
    let Ok(mut sys) = sysinfo::System::new() else {
        return Vec::new();
    };
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    guis.iter()
        .filter(|(pid, exe)| sys.process(sysinfo::Pid::from_u32(*pid)).and_then(|p| p.exe()) == Some(exe.as_path()))
        .cloned()
        .collect()
}

/// 업데이트를 적용할 실행 파일. `SABA_UPDATER_PATH`가 있으면 그것, 없으면 데몬 실행 파일 옆(또는 한 단계 위).
fn updater_binary() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SABA_UPDATER_PATH").map(std::path::PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let installed = saba_chan_updater_lib::UpdateManager::load_installed_manifest();
    updater_binary_near(&std::env::current_exe().ok()?, installer_applies(&installed, saba_chan_updater_lib::INSTALLER_APPLIES_SINCE))
}

/// 설치된 설치기가 업데이트 적용(`--apply`)을 아는가: 설치 기록의 판이 `since`(적용을 맡은 첫 판) 이상이다.
/// 0.1.12 전 Windows 설치기는 `--apply`를 모르고 설치 창을 연다. 데몬만 먼저 새로 받으면 그 설치기가 남는다.
/// 데몬 판과 견주지 않는다: 구성요소 판은 따로 오르므로 데몬만 오른 릴리즈에서 멀쩡한 설치기를 옛것으로 보게 된다
fn installer_applies(installed: &std::collections::HashMap<String, String>, since: &str) -> bool {
    use saba_chan_updater_lib::version::SemVer;
    match (installed.get("installer").and_then(|v| SemVer::parse(v)), SemVer::parse(since)) {
        (Some(installer), Some(since)) => installer >= since,
        _ => false,
    }
}

/// 적용을 아는 설치기(`applies`)를 먼저 찾고, 없으면 0.1.12 전 설치에 남은 옛 업데이터를 쓴다 (같은 인자를 받는다).
/// 설치기가 적용을 모르면 옛 업데이터를 먼저 쓰고, 그것도 없을 때만 설치기를 쓴다
fn updater_binary_near(daemon_exe: &std::path::Path, applies: bool) -> Option<std::path::PathBuf> {
    use saba_chan_updater_lib::{INSTALLER_EXE, LEGACY_UPDATER_EXE};
    let order = if applies { [INSTALLER_EXE, LEGACY_UPDATER_EXE] } else { [LEGACY_UPDATER_EXE, INSTALLER_EXE] };
    order.iter().find_map(|name| daemon_exe.ancestors().skip(1).take(2).map(|dir| dir.join(name)).find(|p| p.is_file()))
}

/// 데몬이 끝나도 살아남게 띄운다
/// 띄우고 `grace` 동안 지켜본다. 그 사이 끝나면 종료 코드를 담은 오류
async fn spawn_and_confirm(program: &std::path::Path, args: &[String], grace: std::time::Duration) -> std::io::Result<()> {
    let mut child = saba_chan_updater_lib::process::spawn_detached(std::process::Command::new(program).args(args))?;
    let deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Err(std::io::Error::other(format!("updater exited right away ({status})")));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(())
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

/// 데몬이 바로 적용한 결과를 완료 마커로 남긴다 (업데이터가 적용할 때와 같은 모양). 클라이언트가 결과·패치노트를 본다.
/// 업데이터가 새 코어를 기다리게 둔 것(`deferred`)을 마저 적용했으면 사용자가 고른 그 묶음의 결과에 더한다.
/// 복구(`repair`)는 업데이트로 알리지 않는다 (`UpdateCompletionMarker::repaired`)
fn record_daemon_apply(path: &std::path::Path, targets: &[String], applied: &[String], errors: &[String], release: Option<saba_chan_updater_lib::marker::ReleaseSummary>, repair: bool) {
    use saba_chan_updater_lib::UpdateCompletionMarker;
    let waiting = UpdateCompletionMarker::load_from(path)
        .filter(|m| m.result.is_none() && !targets.is_empty() && targets.iter().all(|k| m.deferred.contains(k)));
    let marker = match waiting {
        Some(bundle) => bundle.finish_deferred(targets, applied, errors),
        None if repair => UpdateCompletionMarker::applied(applied.to_vec(), errors.to_vec(), Vec::new(), None).repaired(),
        None => UpdateCompletionMarker::applied(applied.to_vec(), errors.to_vec(), Vec::new(), release),
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

/// POST /api/updates/phase/ack — 알약의 `done`을 지운다 (다른 단계는 그대로). 답은 지금 단계
async fn ack_phase(State(state): State<UpdateState>) -> impl IntoResponse {
    if state.phase().state == PhaseState::Done {
        state.set_phase(Phase::default());
    }
    Json(json!({ "ok": true, "phase": state.phase() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuiWindowBody {
    open: bool,
}

/// PUT /api/updates/gui-window {open} — GUI가 창을 보이거나 숨길 때(트레이) 알린다. 창이 열려 있어 미룬 적용은 창을 숨기면 이어서 한다
async fn set_gui_window(State(state): State<UpdateState>, Json(body): Json<GuiWindowBody>) -> impl IntoResponse {
    *state.gui_window.lock().unwrap_or_else(|e| e.into_inner()) = Some(body.open);
    let phase = state.phase();
    if !body.open && phase.state == PhaseState::Ready && phase.reason == Some(ReadyReason::GuiWindowOpen) {
        spawn_core_apply(state.clone());
    }
    Json(json!({ "ok": true, "open": body.open }))
}

/// POST /api/updates/restart — `ready`에서 "지금 다시 시작". 실패한 판 기록을 지우고, 받아 둔 것(없으면 받을 것)을 적용한다.
/// 데몬이 내려가야 하면 `launch-updater`와 같은 길을 타고(GUI 재시작 포함, 본문도 같다) 그 답을 그대로 준다.
/// 데몬이 그대로면 `{ok, updater_launched:false, daemon_stopping:false, gui_restart:false}`
async fn restart_now(State(state): State<UpdateState>, body: LooseJson<LaunchUpdaterRequest>) -> axum::response::Response {
    use crate::ipc::internal::into_internal;
    {
        let mut mgr = state.manager.write().await;
        let mut cfg = mgr.get_config();
        if cfg.clear_failed_release() {
            if let Err(e) = save_updater_config(&cfg) {
                tracing::warn!("[Updates] Failed to persist update config: {}", e);
            }
            mgr.update_config(cfg);
        }
        restore_pending(&mut mgr);
    }
    let nothing_staged = state.manager.read().await.get_pending_components().is_empty();
    if nothing_staged {
        let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest::default()).await).await.body;
        if downloaded.get("ok").and_then(Value::as_bool) == Some(false) && downloaded.get("downloaded").is_none() {
            return (axum::http::StatusCode::BAD_GATEWAY, Json(downloaded)).into_response();
        }
    }
    // 받을 것도 없다 (고정만 풀었거나 준비됨이던 판이 내려갔다): 완료로 알리지 않는다
    if nothing_staged && state.manager.read().await.get_pending_components().is_empty() {
        if state.phase().state == PhaseState::Ready {
            state.set_phase(Phase::default());
        }
        return Json(json!({ "ok": true, "updater_launched": false, "daemon_stopping": false, "gui_restart": false, "errors": [] })).into_response();
    }
    let applied = into_internal(apply_updates_with(state.clone(), ApplyRequest::default()).await).await.body;
    if applied.get("requires_updater").and_then(Value::as_bool) == Some(true) {
        return launch_updater(State(state), body).await;
    }
    if applied.get("ok").and_then(Value::as_bool) == Some(true) {
        let release = state.manager.read().await.applying_release();
        state.set_phase(Phase::new(PhaseState::Done, release.as_ref().map(|r| r.version.clone()), release.map(|r| r.notes).filter(|n| !n.is_empty())));
    }
    Json(json!({
        "ok": applied.get("ok").and_then(Value::as_bool).unwrap_or(false),
        "updater_launched": false,
        "daemon_stopping": false,
        "gui_restart": false,
        "errors": applied.get("errors").cloned().unwrap_or_else(|| json!([])),
    }))
    .into_response()
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

/// 설치 점검: 무결성 검사와 지금 판 그대로 다시 받을 계획. 고칠 것이 있는데 확인한 적이 없으면 먼저 확인한다 (받기와 같다).
/// `mark`면 고칠 것을 다시 받을 수 있게 표시한다
async fn repair_plan(state: &UpdateState, mark: bool) -> Result<Value, String> {
    let report = state.manager.read().await.verify_integrity();
    let mut mgr = state.manager.write().await;
    let broken = report.components.iter().any(|c| matches!(c.status, IntegrityStatus::Tampered | IntegrityStatus::FileNotFound));
    if broken && mgr.get_status().components.is_empty() {
        mgr.check_for_updates().await.map_err(|e| format!("Update check failed: {}", e))?;
    }
    let plan = mgr.repair_plan(&report);
    if mark && !plan.repair.is_empty() {
        mgr.mark_for_repair(&plan);
    }
    Ok(json!({
        "ok": true,
        "verified": report.verified,
        "failed": report.failed,
        "skipped": report.skipped,
        "repair": plan.repair,
        "unavailable": plan.unavailable,
    }))
}

/// GET /api/updates/repair — 설치 점검: 무결성 요약(`verified`·`failed`·`skipped`)과 바뀌었거나 없어진 구성요소 중
/// 지금 판 그대로 다시 받을 것(`repair: [{component, display_name, installed, offered}]`)과 받을 수 없는 것(`unavailable`, 다른 판이 나왔거나 릴리즈에 없다)
async fn get_repair(State(state): State<UpdateState>) -> axum::response::Response {
    match repair_plan(&state, false).await {
        Ok(plan) => Json(plan).into_response(),
        Err(e) => Json(json!({ "ok": false, "error": e })).into_response(),
    }
}

/// POST /api/updates/repair — 점검한 뒤 고칠 것만 지금 판 그대로 다시 받아(`download_components_with`) 적용한다(`apply_updates_with`).
/// 데몬·GUI·업데이터가 끼면 적용하지 않고 `apply`처럼 `requires_updater`·`needs_updater`로 돌려준다(부른 쪽이 `launch-updater`).
/// 답: GET의 계획에 고친 키 `repaired`, `needs_updater`, `requires_updater`, 받거나 적용하지 못한 것 `errors`
async fn repair(State(state): State<UpdateState>) -> axum::response::Response {
    use crate::ipc::internal::into_internal;
    let plan = match repair_plan(&state, true).await {
        Ok(plan) => plan,
        Err(e) => return Json(json!({ "ok": false, "error": e })).into_response(),
    };
    let keys: Vec<String> = plan["repair"].as_array().into_iter().flatten().filter_map(|r| r["component"].as_str().map(str::to_string)).collect();
    if keys.is_empty() {
        return Json(repair_result(plan, &json!({}), &json!({}))).into_response();
    }
    let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest { components: keys }).await).await.body;
    let got = downloaded_keys(&downloaded);
    let applied = if got.is_empty() {
        json!({})
    } else {
        into_internal(apply_updates_with(state, ApplyRequest { components: got, ..Default::default() }).await).await.body
    };
    Json(repair_result(plan, &downloaded, &applied)).into_response()
}

fn downloaded_keys(downloaded: &Value) -> Vec<String> {
    downloaded["downloaded"].as_array().into_iter().flatten().filter_map(|d| d["component"].as_str().map(str::to_string)).collect()
}

/// 계획에 받기·적용 결과를 더한다. 받아서 데몬이 바로 적용한 것만 고친 것이다
fn repair_result(mut plan: Value, downloaded: &Value, applied: &Value) -> Value {
    let applied_names: Vec<&str> = applied["applied"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let repaired: Vec<String> = downloaded_keys(downloaded).into_iter().filter(|k| applied_names.contains(&Component::from_manifest_key(k).display_name().as_str())).collect();
    let mut errors: Vec<Value> = downloaded["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| json!(format!("{}: {}", e["component"].as_str().unwrap_or_default(), e["error"].as_str().unwrap_or_default())))
        .collect();
    errors.extend(applied["errors"].as_array().into_iter().flatten().cloned());
    plan["ok"] = json!(errors.is_empty());
    plan["repaired"] = json!(repaired);
    plan["needs_updater"] = applied.get("needs_updater").cloned().unwrap_or_else(|| json!([]));
    plan["requires_updater"] = json!(applied["requires_updater"].as_bool() == Some(true));
    plan["errors"] = json!(errors);
    plan
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

/// 앱에서 고를 수 있는 업데이트 채널 (위 채널은 아래 채널 릴리즈도 받는다, updater `relay::CHANNELS`). 설치할 때 고르는 채널과 같다
use saba_chan_updater_lib::relay::SELECTABLE_CHANNELS;

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
    if let Some(v) = body.get("auto_apply").and_then(|v| v.as_bool()) {
        cfg.auto_apply = v;
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

    let channel_changed = mgr.get_config().channel != cfg.channel;
    mgr.update_config(cfg.clone());
    if let Err(e) = save_updater_config(&cfg) {
        tracing::warn!("[Updates] Failed to persist update config: {}", e);
    }
    drop(mgr);
    // 채널을 바꾸면 곧바로 확인해 자동 적용 길을 탄다 (내림은 하지 않는다: 확인이 낮은 판을 거절한다)
    if channel_changed && cfg.enabled {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = check_and_follow_up(&state).await {
                tracing::warn!("[Updates] Check after a channel change failed: {}", e);
            }
        });
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
            let mut cfg: UpdateConfig = serde_json::from_value(v).unwrap_or_else(|e| {
                tracing::warn!("[Updates] Ignoring invalid 'updates' config: {}", e);
                UpdateConfig::default()
            });
            // 앱 업데이트는 묻지 않고 적용한다 (사용자 결정 2026-11-09): 옛 설정은 한 번만 켠다. 그 뒤 고른 값은 그대로 둔다
            if !cfg.live_update_migrated {
                cfg.auto_apply = true;
                cfg.auto_download = true;
                cfg.live_update_migrated = true;
                if let Err(e) = save_updater_config_to(file, &cfg) {
                    tracing::warn!("[Updates] Failed to persist update config: {}", e);
                }
            }
            cfg
        }
    }
}

fn save_updater_config(cfg: &UpdateConfig) -> std::io::Result<()> {
    save_updater_config_to(&ConfigFile::default_location(), cfg)
}

fn save_updater_config_to(file: &ConfigFile, cfg: &UpdateConfig) -> std::io::Result<()> {
    file.set_section("updates", serde_json::to_value(cfg).map_err(std::io::Error::other)?)
}

// ═══════════════════════════════════════════════════════
// 판 고르기 (모듈·익스텐션)
// ═══════════════════════════════════════════════════════

/// 판 고르기가 받는 키 (`module-<이름>`, `ext-<id>`)
fn addon_component(key: &str) -> Option<Component> {
    Component::try_from_manifest_key(key).filter(is_addon)
}

fn addon_refusal(status: axum::http::StatusCode, code: &str, error: impl std::fmt::Display) -> axum::response::Response {
    (status, Json(json!({ "ok": false, "error": error.to_string(), "error_code": code }))).into_response()
}

fn not_an_addon(key: &str) -> axum::response::Response {
    addon_refusal(axum::http::StatusCode::BAD_REQUEST, "invalid_key", format!("'{key}' is not a module or an extension"))
}

/// 릴레이에 묻지 못했다 (받기 실패와 같은 502)
fn relay_unreachable(e: saba_chan_updater_lib::relay::RelayError) -> axum::response::Response {
    addon_refusal(axum::http::StatusCode::BAD_GATEWAY, "relay_unreachable", e)
}

/// 판 고르기 창의 목록: 고를 수 있는 판과 내린 판을 새 판부터. 가장 새 판(`latest`)은 내리지 않은 것 중에서
fn addon_versions_body(
    key: &str,
    core: &str,
    installed: Option<&str>,
    list: &saba_chan_updater_lib::relay::AddonVersions,
    pinned: Option<&addon_pins::AddonPin>,
) -> Value {
    use saba_chan_updater_lib::version::SemVer;
    let mut rows: Vec<(&str, bool)> = list.versions.iter().map(|c| (c.version.as_str(), false)).chain(list.yanked.iter().map(|v| (v.as_str(), true))).collect();
    rows.sort_by(|a, b| SemVer::parse(b.0).cmp(&SemVer::parse(a.0)));
    let latest = list.versions.first().map(|c| c.version.as_str());
    let versions: Vec<Value> = rows
        .into_iter()
        .map(|(version, yanked)| json!({ "version": version, "installed": installed == Some(version), "latest": latest == Some(version), "yanked": yanked }))
        .collect();
    json!({ "ok": true, "key": key, "core": core, "installed": installed, "versions": versions, "excluded": list.excluded, "pinned": pinned })
}

/// GET /api/updates/addons/{key}/versions — 이 코어로 돌아가는 판(서명 확인)과 내린 판, 더 새 코어가 필요해 뺀 판
async fn get_addon_versions(State(state): State<UpdateState>, Path(key): Path<String>) -> axum::response::Response {
    if addon_component(&key).is_none() {
        return not_an_addon(&key);
    }
    let (ask, core, installed) = {
        let mgr = state.manager.read().await;
        (mgr.addon_versions(&key), mgr.addon_core(), mgr.installed_addon_version(&key))
    };
    match ask.await {
        Ok(list) => {
            let pinned = addon_pins::get(&key).as_installed(installed.as_deref().unwrap_or_default()).pinned;
            Json(addon_versions_body(&key, &core, installed.as_deref(), &list, pinned.as_ref())).into_response()
        }
        Err(e) => relay_unreachable(e),
    }
}

#[derive(Deserialize)]
struct PinRequest {
    version: String,
}

/// POST /api/updates/addons/{key}/pin `{version}` — 고른 판을 받아(서명·sha256 확인) 바꾸고, 그 순간 가장 새 판보다 새 판이 나올 때까지 고정한다
async fn pin_addon(State(state): State<UpdateState>, Path(key): Path<String>, raw: axum::body::Bytes) -> axum::response::Response {
    let body: PinRequest = match serde_json::from_slice(&raw) {
        Ok(b) => b,
        Err(e) => return bad_body(format!("invalid JSON body: {}", e)),
    };
    let Some(component) = addon_component(&key) else { return not_an_addon(&key) };
    let ask = state.manager.read().await.addon_versions(&key);
    let list = match ask.await {
        Ok(list) => list,
        Err(e) => return relay_unreachable(e),
    };
    let conflict = axum::http::StatusCode::CONFLICT;
    let Some(entry) = list.find(&body.version).cloned() else {
        let why = if list.yanked.contains(&body.version) {
            "it was yanked"
        } else if list.excluded.contains(&body.version) {
            "it needs a newer saba-chan"
        } else {
            "it is not offered"
        };
        return addon_refusal(conflict, "version_unavailable", format!("{} {} cannot be chosen: {}", key, body.version, why));
    };
    // 고정은 지금 이 코어에 내는 가장 새 판(보통 바꾸기 전 판)보다 새 판이 나올 때까지. 가장 새 판을 고르면 고정하지 않는다 (고정 풀기와 같다)
    let until = list.versions.first().map(|c| c.version.clone()).unwrap_or_else(|| entry.version.clone());
    let pin = (entry.version != until).then(|| addon_pins::AddonPin { version: entry.version.clone(), until_newer_than: until });
    switch_addon(&state, &component, &entry, pin).await
}

/// DELETE /api/updates/addons/{key}/pin — 고정을 풀고 지금 가장 새 판으로 바로 바꾼다
async fn unpin_addon(State(state): State<UpdateState>, Path(key): Path<String>) -> axum::response::Response {
    let Some(component) = addon_component(&key) else { return not_an_addon(&key) };
    let ask = state.manager.read().await.addon_versions(&key);
    let list = match ask.await {
        Ok(list) => list,
        Err(e) => return relay_unreachable(e),
    };
    let Some(newest) = list.versions.first().cloned() else {
        return addon_refusal(axum::http::StatusCode::CONFLICT, "version_unavailable", format!("no version of {} is offered for this saba-chan", key));
    };
    switch_addon(&state, &component, &newest, None).await
}

/// 판 고르기가 거절하는 사용자 폴더: 자동 교체처럼 카탈로그 그대로가 아닌 폴더는 덮어쓰지 않는다. 오류 코드는 신뢰 표시의 것
/// (`module_modified`·`module_unverified`, 익스텐션은 `extension_…`)
fn hand_modified(key: &str) -> Option<String> {
    use saba_chan_updater_lib::integrity::{self, Trust};
    if replaceable_without_asking(key) {
        return None;
    }
    let kind = if key.starts_with(saba_chan_updater_lib::relay::MODULE_PREFIX) { "module" } else { "extension" };
    let trust = crate::sandbox::component_dir(key).map(|dir| integrity::trust_of(key, &dir));
    Some(format!("{}_{}", kind, if trust == Some(Trust::Unverified) { "unverified" } else { "modified" }))
}

/// `entry` 판으로 바꾸고 `pin`을 남긴다 (`None`이면 고정을 푼다). 받기·검증·교체는 자동 교체와 같은 길(`download_component`,
/// `apply_updates_with`의 `untouched_only`)이고, 켜진 서버가 쓰면 자동 교체 반복이 서버가 꺼진 뒤 바꾼다.
/// 실패하면 고른 것과 상태를 그 전으로 되돌린다
async fn switch_addon(
    state: &UpdateState,
    component: &Component,
    entry: &saba_chan_updater_lib::ManifestComponent,
    pin: Option<addon_pins::AddonPin>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    let key = component.manifest_key();
    let version = entry.version.clone();
    let previous = addon_pins::get(&key);
    let record = |choice: addon_pins::AddonChoice| {
        if let Err(e) = addon_pins::update(&key, |c| *c = choice) {
            tracing::warn!("[Updates] Could not save the chosen version of {}: {}", key, e);
        }
    };
    let answer = |state: &str, waiting_for: Vec<String>| Json(json!({ "ok": true, "state": state, "version": version, "waiting_for": waiting_for })).into_response();
    // 이미 깔린 판이면 고정만 바꾼다. 켜진 서버를 기다리던 다른 판은 버린다
    if state.manager.read().await.installed_addon_version(&key).as_deref() == Some(version.as_str()) {
        state.manager.write().await.settle_addon(&key, entry);
        record(addon_pins::AddonChoice { pinned: pin, pending_version: None });
        return answer("applied", Vec::new());
    }
    // 요구 조건은 깔린 것과 대조한다 (코어는 릴레이가 걸렀다). 맞지 않으면 받지 않는다
    let unmet = state.manager.read().await.unmet_addon_requires(&key, &entry.requires.clone().unwrap_or_default());
    if !unmet.is_empty() {
        return addon_refusal(StatusCode::CONFLICT, "requires_unmet", unmet.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
    }
    if let Some(code) = hand_modified(&key) {
        return addon_refusal(StatusCode::CONFLICT, &code, format!("{} was changed by hand; it is not replaced", key));
    }
    let staged = state.manager.write().await.stage_addon(&key, entry);
    let downloaded = state.manager.write().await.download_component(component).await;
    if let Err(e) = downloaded {
        state.manager.write().await.unstage_addon(staged);
        use saba_chan_updater_lib::download::DownloadError;
        let unverified = matches!(e.downcast_ref::<DownloadError>(), Some(DownloadError::Mismatch { .. } | DownloadError::BadHash(_) | DownloadError::TooLarge { .. }));
        let code = if unverified { "verify_failed" } else { "download_failed" };
        return addon_refusal(StatusCode::BAD_GATEWAY, code, e);
    }
    record(addon_pins::AddonChoice { pinned: pin, pending_version: Some(version.clone()) });
    // 여기부터 바꿀 때까지 서버 시작을 막는다 (자동 교체와 같다). 시작·설치 중이거나 켜진 서버가 쓰면 서버가 꺼진 뒤 자동 교체 반복이 바꾼다
    let starts = try_hold_starts(state).await;
    let waiting_for = if starts.is_some() { servers_using(state, component).await } else { Vec::new() };
    if starts.is_none() || !waiting_for.is_empty() {
        drop(starts);
        spawn_addon_replace(state.clone());
        return answer("waiting", waiting_for);
    }
    let applied = crate::ipc::internal::into_internal(
        apply_updates_with(state.clone(), ApplyRequest { components: vec![key.clone()], untouched_only: true }).await.into_response(),
    )
    .await
    .body;
    drop(starts);
    if let Some(errors) = applied.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
        state.manager.write().await.unstage_addon(staged);
        record(previous);
        let code = hand_modified(&key).unwrap_or_else(|| "apply_failed".to_string());
        return addon_refusal(StatusCode::CONFLICT, &code, errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
    }
    answer("applied", Vec::new())
}

#[cfg(test)]
mod tests {
    /// 릴레이의 새 릴리즈 알림은 데몬이 고른 채널이 그 채널을 받고 지금보다 새 버전일 때만 확인을 돌린다
    #[test]
    fn update_hint_runs_a_check_only_for_a_newer_release_in_the_chosen_channel() {
        let hint = |version: &str, channel: &str| serde_json::json!({ "version": version, "channel": channel });
        // 베타 채널을 쓰지 않으면 베타 릴리즈는 알리지 않는다
        assert!(!super::hint_wants_check(&hint("0.1.13-beta.1", "beta"), "stable", "0.1.12"));
        assert!(!super::hint_wants_check(&hint("0.1.13-beta.1", "beta"), "rc", "0.1.12"));
        assert!(super::hint_wants_check(&hint("0.1.13-beta.1", "beta"), "beta", "0.1.12"));
        // 위 채널은 아래 채널 릴리즈도 받는다
        assert!(super::hint_wants_check(&hint("0.1.13", "stable"), "beta", "0.1.13-beta.1"));
        // 같거나 낮은 버전, 읽을 수 없는 알림은 무시한다
        assert!(!super::hint_wants_check(&hint("0.1.12", "stable"), "stable", "0.1.12"));
        assert!(!super::hint_wants_check(&hint("0.1.11", "stable"), "stable", "0.1.12"));
        assert!(!super::hint_wants_check(&serde_json::json!({ "channel": "stable" }), "stable", "0.1.12"));
        assert!(!super::hint_wants_check(&hint("0.1.13", "nope"), "stable", "0.1.12"));
    }

    /// 띄운 업데이터가 바로 끝나면(라이브러리 없음·백신 차단) 데몬이 내려가기 전에 실패로 알린다
    #[cfg(unix)]
    #[tokio::test]
    async fn updater_that_exits_at_once_is_a_launch_failure() {
        let sh = std::path::Path::new("/bin/sh");
        let quick = std::time::Duration::from_millis(400);
        let err = super::spawn_and_confirm(sh, &["-c".to_string(), "exit 127".to_string()], quick).await.unwrap_err();
        assert!(err.to_string().contains("127"), "{err}");
        super::spawn_and_confirm(sh, &["-c".to_string(), "sleep 3".to_string()], quick).await.unwrap();
    }

    /// 상태 응답은 설치된 번역 판(`locales_version`, 없으면 null)을 늘 싣는다: GUI·TUI가 바뀌면 번역을 다시 읽는다
    #[tokio::test]
    async fn status_carries_installed_locales_version() {
        use axum::response::IntoResponse;
        let resp = super::get_status(axum::extract::State(super::UpdateState::default())).await.into_response();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.get("locales_version"), Some(&serde_json::json!(crate::chat::i18n::installed_version())));
    }

    /// 받아 둔 업데이트의 `pending.json`을 같이 쓰는 시험끼리 차례로
    static STAGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// 받아 둔 뒤 데몬·PC를 다시 켜면 상태가 비어 있다: 다음 실행 때 적용은 디스크의 대기 목록을 되살려 적용한다
    #[tokio::test]
    async fn applying_on_launch_restores_what_was_downloaded_before_a_restart() {
        let _staging = STAGING.lock().await;
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        std::fs::create_dir_all(&staging).unwrap();
        let staged = staging.join("saba-chan-gui-restore-test.zip");
        std::fs::write(&staged, b"zip").unwrap();
        let pending = json!([{
            "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
            "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-restore-test.zip",
            "release_notes": null, "published_at": null, "downloaded": true,
            "downloaded_path": staged.to_string_lossy(), "installed": true
        }]);
        std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

        let state = UpdateState::new();
        let mut mgr = state.manager.write().await;
        assert!(mgr.get_pending_components().is_empty(), "a fresh daemon has not read the staging folder yet");
        super::restore_pending(&mut mgr);
        assert_eq!(mgr.get_pending_components().len(), 1);
        std::fs::remove_file(staging.join("pending.json")).unwrap();
        std::fs::remove_file(&staged).unwrap();
    }

    /// 자동 적용을 끈 설정(받아 두기·알리기만): 받아 둔 앱 업데이트를 적용하지 않고 `ready`(`manual`)로 둔다.
    /// 받을 것이 없어지면 그 단계를 걷는다
    #[tokio::test]
    async fn with_auto_apply_off_the_update_waits_for_a_restart() {
        let _staging = STAGING.lock().await;
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        std::fs::create_dir_all(&staging).unwrap();
        let staged = staging.join("saba-chan-gui-manual-test.zip");
        std::fs::write(&staged, b"zip").unwrap();
        let pending = json!([{
            "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
            "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-manual-test.zip",
            "release_notes": null, "published_at": null, "downloaded": true,
            "downloaded_path": staged.to_string_lossy(), "installed": true
        }]);
        std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        {
            let mut mgr = state.manager.write().await;
            super::restore_pending(&mut mgr);
            let mut cfg = mgr.get_config();
            cfg.auto_apply = false;
            mgr.update_config(cfg);
        }
        super::core_apply(&state).await;
        let phase = state.phase();
        assert_eq!((phase.state, phase.reason), (PhaseState::Ready, Some(ReadyReason::Manual)));
        assert!(!state.updater_launched());

        std::fs::remove_file(staging.join("pending.json")).unwrap();
        std::fs::remove_file(&staged).unwrap();
        let cfg = state.manager.read().await.get_config();
        *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, tmp.path().to_str().unwrap());
        super::core_apply(&state).await;
        assert_eq!(state.phase(), Phase::default(), "nothing left to apply");
    }

    #[test]
    fn auto_apply_is_kept_with_the_update_settings() {
        let mut cfg = saba_chan_updater_lib::UpdateConfig::default();
        assert!(cfg.auto_apply && cfg.auto_download, "a new install applies updates without asking");
        cfg.auto_apply = false;
        let dir = tempfile::tempdir().unwrap();
        super::save_updater_config_to(&saba_chan_updater_lib::config_file::ConfigFile::at(dir.path()), &cfg).unwrap();
        assert!(!super::load_updater_config_from(&saba_chan_updater_lib::config_file::ConfigFile::at(dir.path())).auto_apply);
    }

    /// 옛 설정(알리기만)은 한 번만 묻지 않는 자동 적용으로 바꾼다. 그 뒤 사용자가 끈 값은 그대로 둔다
    #[test]
    fn old_settings_move_to_automatic_updates_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        file.set_section("updates", json!({ "enabled": true, "auto_download": false, "auto_apply": false, "channel": "beta" })).unwrap();
        let loaded = load_updater_config_from(&file);
        assert!(loaded.auto_apply && loaded.auto_download && loaded.live_update_migrated);
        assert_eq!(loaded.channel, "beta");
        assert_eq!(file.section("updates")["live_update_migrated"], true, "the move is saved");

        let mut off = loaded;
        off.auto_apply = false;
        off.auto_download = false;
        save_updater_config_to(&file, &off).unwrap();
        let again = load_updater_config_from(&file);
        assert!(!again.auto_apply && !again.auto_download, "a later choice is kept");
    }

    /// 등록된 GUI 중 GUI 실행 파일로 떠 있는 것만 고르고, 업데이터를 띄울 때 끈다
    #[cfg(unix)]
    #[tokio::test]
    async fn launching_the_updater_closes_only_registered_gui_processes() {
        let tmp = tempfile::tempdir().unwrap();
        let gui_exe = tmp.path().join("saba-chan-gui");
        // 리눅스는 sleep을 쓰지 않는다: uutils coreutils(우분투 러너)는 실행 파일 이름이 유틸리티 이름과 다르면 바로 끝난다.
        // 셸 내장 read로 표준 입력이 닫힐 때까지 기다린다. 맥의 /bin/sh는 다른 셸을 다시 띄우는 실행기라 sleep을 쓴다
        #[cfg(target_os = "linux")]
        let (waiter, args): (&str, &[&str]) = ("/bin/sh", &["-c", "read _"]);
        #[cfg(not(target_os = "linux"))]
        let (waiter, args): (&str, &[&str]) = ("/bin/sleep", &["30"]);
        std::fs::copy(waiter, &gui_exe).unwrap();
        let spawn = |exe: &std::path::Path| {
            // 방금 복사한 파일은 다른 테스트가 fork하는 동안 쓰기 핸들이 남아 ETXTBSY가 날 수 있다
            let mut child = (0..50)
                .find_map(|_| match std::process::Command::new(exe).args(args).stdin(std::process::Stdio::piped()).spawn() {
                    Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        None
                    }
                    other => Some(other.unwrap()),
                })
                .expect("executable stayed busy");
            let pid = child.id();
            let stdin = child.stdin.take(); // wait가 표준 입력을 닫으면 read가 끝나므로 기다리는 동안 쥐고 있는다
            std::thread::spawn(move || {
                let _stdin = stdin;
                child.wait()
            }); // 끝나면 바로 거둬 좀비로 남지 않게
            pid
        };
        let gui = spawn(&gui_exe);
        let other = spawn(std::path::Path::new(waiter));
        let registry = crate::ipc::ClientRegistry::new();
        registry.register(crate::ipc::ClientKind::Gui, Some(gui), None).await;
        registry.register(crate::ipc::ClientKind::Gui, Some(other), None).await; // PID가 남의 프로그램에 다시 쓰인 경우
        registry.register(crate::ipc::ClientKind::Cli, Some(other), None).await;

        let guis = super::running_guis(&registry.list().await);
        // 맥은 임시 폴더를 링크(/var → /private/var)를 풀지 않은 경로로 알려 준다
        let found: Vec<_> = guis.iter().map(|(pid, exe)| (*pid, std::fs::canonicalize(exe).unwrap())).collect();
        assert_eq!(found, vec![(gui, std::fs::canonicalize(&gui_exe).unwrap())]);

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
        let _staging = STAGING.lock().await;
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
        // 진짜 업데이터처럼 데몬을 기다리며 살아 있다 (바로 끝나면 띄우기 실패로 본다)
        let fake = staging.join("fake-updater.sh");
        std::fs::write(&fake, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        // edition 2024: 환경 변수 쓰기는 unsafe다. 테스트 준비에서만 쓴다
        unsafe { std::env::set_var("SABA_UPDATER_PATH", &fake) };

        let state = UpdateState::new();
        state.manager.write().await.load_pending_manifest().unwrap();
        let mut events = crate::events::bus().subscribe(None).rx;
        let first = super::launch_updater(State(state.clone()), crate::ipc::handlers::LooseJson(None)).await;
        assert_eq!(first.status(), axum::http::StatusCode::OK);
        // 데몬이 내려가기 전에 붙은 GUI가 알게 한다: 남는 GUI는 데몬을 띄우지 않고 기다리고, 끌 GUI는 스스로 끝난다
        let swap = std::iter::from_fn(|| events.try_recv().ok())
            .find(|e| e.kind == "update.restarting")
            .expect("the swap is announced on the event stream");
        assert_eq!(swap.data["closingGuis"], json!([]), "no GUI is registered, so none is closed");
        let ticket = std::fs::read_to_string(staging.join("apply-ticket")).unwrap();

        let second = super::launch_updater(State(state.clone()), crate::ipc::handlers::LooseJson(None)).await;
        assert_eq!(second.status(), axum::http::StatusCode::CONFLICT);
        let body = axum::body::to_bytes(second.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error_code"], "updater_already_launched");
        assert_eq!(std::fs::read_to_string(staging.join("apply-ticket")).unwrap(), ticket, "the first ticket stays valid");
    }

    /// 업데이트는 설치기(`saba-chan-installer --apply`)가 적용한다. 설치기가 아직 없는 옛 설치(0.1.12 전)는
    /// 남아 있는 옛 업데이터로 적용해 그 업데이트로 설치기를 받는다. 옛 설치기(`--apply`를 모르고 설치 창을 연다)만
    /// 있는 동안에도 옛 업데이터를 쓴다: 데몬만 먼저 새로 받았거나 설치기 업데이트가 실패한 Windows 설치
    #[test]
    fn the_installer_applies_updates_and_the_old_updater_is_the_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = |name: &str| format!("{name}{}", std::env::consts::EXE_SUFFIX);
        let daemon = bin.join(exe("saba-core"));
        assert_eq!(super::updater_binary_near(&daemon, true), None);
        std::fs::write(bin.join(exe("saba-chan-updater")), b"").unwrap();
        assert_eq!(super::updater_binary_near(&daemon, true), Some(bin.join(exe("saba-chan-updater"))));
        std::fs::write(tmp.path().join(exe("saba-chan-installer")), b"").unwrap();
        assert_eq!(super::updater_binary_near(&daemon, true), Some(tmp.path().join(exe("saba-chan-installer"))), "one folder up");
        std::fs::write(bin.join(exe("saba-chan-installer")), b"").unwrap();
        assert_eq!(super::updater_binary_near(&daemon, true), Some(bin.join(exe("saba-chan-installer"))));
        assert_eq!(
            super::updater_binary_near(&daemon, false),
            Some(bin.join(exe("saba-chan-updater"))),
            "an old installer cannot apply; the old updater still can"
        );
        std::fs::remove_file(bin.join(exe("saba-chan-updater"))).unwrap();
        assert_eq!(super::updater_binary_near(&daemon, false), Some(bin.join(exe("saba-chan-installer"))), "nothing else to use");
    }

    /// 설치기가 적용을 맡은 첫 판 이상으로 기록돼 있어야 `--apply`를 안다고 본다. 데몬만 오른 릴리즈 뒤에도 그대로다
    #[test]
    fn only_an_installer_since_the_merge_counts_as_applying() {
        let manifest = |v: Option<&str>| v.map(|v| [("installer".to_string(), v.to_string())].into_iter().collect()).unwrap_or_default();
        let since = saba_chan_updater_lib::INSTALLER_APPLIES_SINCE;
        assert!(super::installer_applies(&manifest(Some(since)), since));
        assert!(super::installer_applies(&manifest(Some("0.1.12")), since));
        assert!(super::installer_applies(&manifest(Some("0.1.12-beta.3")), since));
        assert!(!super::installer_applies(&manifest(Some("0.1.12-beta.1")), since), "the last installer without --apply");
        assert!(!super::installer_applies(&manifest(Some("0.1.11")), since));
        assert!(!super::installer_applies(&manifest(None), since));
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

    /// GUI가 바뀌지 않으면 GUI는 끄지도 다시 띄우지도 않는다: 데몬만 다시 뜨고 떠 있는 GUI가 다시 붙는다.
    /// 트레이만 든 GUI는 데몬이 사라지면 따라 끝나므로 예전처럼 끄고 트레이로 다시 띄운다
    #[test]
    fn only_a_changed_gui_is_closed_and_relaunched() {
        let gui = std::path::PathBuf::from(format!("/opt/saba/{}", saba_chan_updater_lib::GUI_EXE_NAMES[0]));
        let guis = vec![(42, gui.clone())];
        let daemon = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
        let from_gui = vec![gui.to_string_lossy().into_owned()];

        let (close, relaunch) = super::restart_plan(false, from_gui.clone(), &guis, false, daemon.clone());
        assert!(close.is_empty(), "an unchanged GUI stays open");
        assert_eq!(relaunch, daemon, "only the daemon comes back");
        let (close, relaunch) = super::restart_plan(false, Vec::new(), &guis, false, daemon.clone());
        assert!(close.is_empty());
        assert_eq!(relaunch, daemon);
        let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string(), "--from-cli".to_string()];
        assert_eq!(super::restart_plan(false, cli.clone(), &[], false, daemon.clone()).1, cli, "a non-GUI request stands");

        let (close, relaunch) = super::restart_plan(true, daemon.clone(), &guis, false, daemon.clone());
        assert_eq!(close, guis, "a changed GUI is closed so its files can be replaced");
        assert_eq!(relaunch, from_gui);
        let (close, relaunch) = super::restart_plan(false, daemon.clone(), &guis, true, daemon.clone());
        assert_eq!(close, guis, "a tray-only GUI would quit with the daemon");
        assert_eq!(relaunch, vec![gui.to_string_lossy().into_owned(), "--tray".to_string()]);
    }

    /// GUI가 적용 대상일 때만 GUI를 다시 띄운다. 남을 줄 모르는 옛 GUI가 부른 적용은 예전처럼 그 GUI를 다시 띄운다
    #[test]
    fn an_old_gui_that_always_quits_is_relaunched() {
        let core = vec!["saba-core".to_string()];
        let with_gui = vec!["saba-core".to_string(), "gui".to_string()];
        let gui = vec![format!("/opt/saba/{}", saba_chan_updater_lib::GUI_EXE_NAMES[0])];
        let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
        assert!(!super::gui_must_restart(&core, &gui, true));
        assert!(super::gui_must_restart(&core, &gui, false), "an old GUI quits on success");
        assert!(!super::gui_must_restart(&core, &cli, false), "the CLI does not quit a GUI");
        assert!(super::gui_must_restart(&with_gui, &gui, true));
        assert!(super::gui_must_restart(&with_gui, &cli, false));
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

    /// 받지 못했거나 바꾸지 못한 모듈·익스텐션은 다음 정기 확인(하루 뒤)을 기다리지 않고 5분, 30분, 30분 뒤에 다시 해 본다
    #[test]
    fn a_failed_addon_replacement_is_tried_again_a_few_times_before_the_next_check() {
        use super::{addon_retry_wait, Leftover, ADDON_HOLD_RETRY};
        let minutes = |m: u64| Some(std::time::Duration::from_secs(m * 60));
        let failed = Leftover { held: false, failed: true };
        let mut failures = 0;
        let waits: Vec<_> = (0..4).map(|_| addon_retry_wait(failed, &mut failures)).collect();
        assert_eq!(waits, [minutes(5), minutes(30), minutes(30), None]);

        // 켜진 서버 때문에 미룬 것은 서버가 꺼질 때까지 5분마다 본다. 실패가 섞여도 더 늦추지 않는다
        let mut failures = 0;
        assert_eq!(addon_retry_wait(Leftover { held: true, failed: false }, &mut failures), Some(ADDON_HOLD_RETRY));
        for _ in 0..5 {
            assert_eq!(addon_retry_wait(Leftover { held: true, failed: true }, &mut failures), Some(ADDON_HOLD_RETRY));
        }
        // 잘 끝난 차례 뒤의 실패는 처음부터 센다
        assert_eq!(addon_retry_wait(Leftover::default(), &mut failures), None);
        assert_eq!(addon_retry_wait(failed, &mut failures), minutes(5));
    }

    /// 실패한 교체를 다시 해 보려고 기다리는 사이에 확인이 또 오면(네트워크를 고치고 '업데이트 확인'을 누름) 남은 잠을 기다리지 않는다
    #[tokio::test]
    async fn a_new_check_does_not_wait_out_the_retry_delay_of_a_failed_addon_replacement() {
        use super::{replace_addons_until_done, Leftover};
        let recheck = tokio::sync::Notify::new();
        let (ran, mut runs) = tokio::sync::mpsc::unbounded_channel();
        let rounds = replace_addons_until_done(
            || {
                let _ = ran.send(());
                async { Leftover { held: false, failed: true } }
            },
            &recheck,
        );
        let checks = async {
            runs.recv().await.unwrap();
            recheck.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(5), runs.recv()).await.expect("the replacement ran again right after the check");
        };
        tokio::select! {
            _ = rounds => panic!("the retries ended early"),
            _ = checks => {}
        }
    }

    /// 헤드리스 설치는 창 없이 적용하고, 다시 띄울 것을 받지 않았으면 데몬 자신을 다시 띄운다. 데스크톱은 그대로
    /// 코어가 낀 묶음에서 업데이터가 새 코어를 기다리게 둔 모듈(완료 마커 `deferred`)은 업데이터가 새 데몬을 다 지켜본 뒤
    /// 새 데몬이 마저 바꾼다. 사용자가 고른 묶음이므로 업데이트 확인을 꺼 두었거나 카탈로그 그대로가 아닌 폴더여도 바꾸고,
    /// 결과는 그 묶음의 결과(패치노트 포함)에 더한다
    #[tokio::test]
    async fn an_addon_left_for_the_new_core_is_applied_after_the_updater_stops_watching() {
        use std::io::Write;
        use saba_chan_updater_lib::{live_apply, UpdateCompletionMarker};
        let _staging = STAGING.lock().await;
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        std::fs::create_dir_all(&staging).unwrap();
        let name = format!("deferred-{}", std::process::id());
        let key = format!("module-{name}");
        let staged = staging.join(format!("{key}.zip"));
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&staged).unwrap());
            zw.start_file("module.toml", zip::write::SimpleFileOptions::default()).unwrap();
            zw.write_all(b"new").unwrap();
            zw.finish().unwrap();
        }
        let pending = json!([{
            "component": { "module": name }, "current_version": "0.1.0", "latest_version": "0.2.0",
            "update_available": true, "download_url": null, "asset_name": null,
            "release_notes": null, "published_at": null, "downloaded": true,
            "downloaded_path": staged.to_string_lossy(), "installed": true
        }]);
        std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let module_dir = tmp.path().join("modules").join(&name);
        std::fs::create_dir_all(&module_dir).unwrap();
        std::fs::write(module_dir.join("module.toml"), "old, edited by the user").unwrap();
        let state = state_in(tmp.path());
        let cfg = saba_chan_updater_lib::UpdateConfig { enabled: false, ..Default::default() };
        *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, tmp.path().join("modules").to_str().unwrap());
        let release = saba_chan_updater_lib::marker::ReleaseSummary { version: "9.9.9".into(), ..Default::default() };
        UpdateCompletionMarker { deferred: vec![key.clone()], release: Some(release), ..UpdateCompletionMarker::success(vec!["saba-core".into()]) }
            .save()
            .unwrap();

        live_apply::begin_watch(&staging).unwrap();
        let first = super::apply_deferred_once(&state, &staging).await;
        live_apply::end_watch(&staging);
        assert!(first.is_some(), "waits while the updater may still roll the core back");
        assert_eq!(std::fs::read_to_string(module_dir.join("module.toml")).unwrap(), "old, edited by the user");

        assert_eq!(super::apply_deferred_once(&state, &staging).await, None);
        assert_eq!(std::fs::read_to_string(module_dir.join("module.toml")).unwrap(), "new");
        let marker = UpdateCompletionMarker::load().unwrap();
        assert_eq!(marker.updated_components, vec!["saba-core".to_string(), key.clone()]);
        assert!(marker.success && marker.deferred.is_empty(), "{marker:?}");
        assert_eq!(marker.release.map(|r| r.version).as_deref(), Some("9.9.9"), "the notes of the chosen bundle stay");

        let _ = std::fs::remove_file(staging.join("pending.json"));
        let _ = std::fs::remove_file(&staged);
        let _ = std::fs::remove_file(UpdateCompletionMarker::path());
    }

    #[test]
    fn a_module_or_extension_a_running_server_uses_waits_for_the_server_to_stop() {
        use saba_chan_updater_lib::Component;
        let busy_modules = std::collections::HashSet::from(["palworld".to_string()]);
        let busy_extensions = std::collections::HashSet::from(["steamcmd".to_string()]);
        let pending = vec![
            Component::Module("palworld".into()),
            Component::Module("minecraft".into()),
            Component::Extension("steamcmd".into()),
            Component::Extension("xna".into()),
        ];
        let (ready, held) = super::split_by_busy(pending, &busy_modules, &busy_extensions);
        assert_eq!(ready, [Component::Module("minecraft".into()), Component::Extension("xna".into())]);
        assert_eq!(held, [Component::Module("palworld".into()), Component::Extension("steamcmd".into())]);
    }

    #[test]
    fn a_headless_install_applies_without_a_window_and_relaunches_the_daemon() {
        use saba_chan_updater_lib::profile::Profile;
        let me = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let args = super::updater_args("t".into(), Profile::Headless, Vec::new(), None);
        let relaunch = args.iter().position(|a| a == "--relaunch").expect("relaunches something");
        assert!(args[..relaunch].contains(&"--headless".to_string()), "{args:?}");
        assert_eq!(args[relaunch + 1..], [me, "--spawned".to_string()]);

        let cli = vec!["/opt/saba/saba-core".to_string(), "--spawned".to_string()];
        let args = super::updater_args("t".into(), Profile::Headless, cli.clone(), None);
        assert_eq!(args[args.len() - 2..], cli[..]);

        let args = super::updater_args("t".into(), Profile::Desktop, Vec::new(), None);
        assert!(!args.iter().any(|a| a == "--headless" || a == "--relaunch"), "{args:?}");

        // 서비스로 돈 데몬은 서비스를 다시 시작한다
        let service = vec!["/usr/bin/systemctl".to_string(), "--user".into(), "start".into(), "saba-chan.service".into()];
        let args = super::updater_args("t".into(), Profile::Headless, Vec::new(), Some(service.clone()));
        assert_eq!(args[args.len() - 4..], service[..]);
        // CLI가 데몬을 다시 띄우라고 해도 서비스가 먼저다
        let args = super::updater_args("t".into(), Profile::Headless, cli.clone(), Some(service.clone()));
        assert_eq!(args[args.len() - 4..], service[..]);
        assert!(!args.contains(&cli[0]), "{args:?}");
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
        super::record_daemon_apply(&path, &["module-minecraft".into(), "ext-steamcmd".into()], &["module-minecraft".into()], &["ext-steamcmd: busy".into()], Some(release), false);

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

    /// 데몬이 바로 적용한 복구(`update repair`)는 업데이트 알림이 되지 않는다: 확인한 결과로 남아 `update last`만 본다
    #[test]
    fn a_repair_the_daemon_applied_is_not_announced_as_an_update() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("update-complete.json");
        let release = saba_chan_updater_lib::marker::ReleaseSummary { version: "0.1.8".into(), ..Default::default() };
        super::record_daemon_apply(&path, &["cli".into()], &["cli".into()], &[], Some(release), true);

        assert!(super::last_result_at(&path, false, "ko")["result"].is_null(), "{}", super::last_result_at(&path, false, "ko"));
        let seen = super::last_result_at(&path, true, "ko");
        assert_eq!((seen["result"]["result"].as_str(), seen["result"]["success"].as_bool()), (Some("repaired"), Some(true)), "{seen}");
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

    /// 시험용: 상태 파일을 임시 폴더에 둔다
    fn state_in(dir: &std::path::Path) -> UpdateState {
        let mut state = UpdateState::new();
        state.live = Arc::default();
        state.live_path = Arc::new(dir.join("live-update.json"));
        state
    }

    /// 상태 응답에 알약 단계와 정식 기다림을 싣는다. `done`은 확인(ack)하면 `idle`이 되고 상태 파일에 남는다
    #[tokio::test]
    async fn the_status_carries_the_phase_and_ack_clears_done() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        let body = |r: axum::response::Response| async { crate::ipc::internal::into_internal(r).await.body };
        let status = body(get_status(State(state.clone())).await.into_response()).await;
        assert_eq!(status["phase"]["state"], "idle");
        assert!(status.as_object().unwrap().contains_key("stable_wait"));

        state.set_phase(Phase::new(PhaseState::Done, Some("0.1.14".into()), None));
        let status = body(get_status(State(state.clone())).await.into_response()).await;
        assert_eq!((status["phase"]["state"].clone(), status["phase"]["version"].clone()), (json!("done"), json!("0.1.14")));
        assert_eq!(LiveState::load_from(&tmp.path().join("live-update.json")).phase.state, PhaseState::Done);

        // ready는 ack로 지워지지 않는다 (다시 시작으로만)
        let acked = body(ack_phase(State(state.clone())).await.into_response()).await;
        assert_eq!(acked["phase"]["state"], "idle");
        state.set_phase(Phase::ready(ReadyReason::RolledBack, Some("0.1.14".into()), None));
        let acked = body(ack_phase(State(state.clone())).await.into_response()).await;
        assert_eq!(acked["phase"]["state"], "ready");
    }

    /// 업데이트가 데몬 작업만 기다리는 동안에만 새 예약 작업을 미룬다. 켜진 서버를 기다리면 예약 작업은 돈다
    #[test]
    fn scheduled_work_is_held_only_while_an_update_waits() {
        use crate::supervisor::core_swap::{CoreSwapBlocker, CoreSwapBlockerKind as K};
        let blocker = |kind| CoreSwapBlocker { kind, instance_id: "i".into(), name: "n".into() };
        let waiting = |kinds: &[K]| Phase {
            waiting_for: kinds.iter().map(|k| blocker(*k)).collect(),
            ..Phase::new(PhaseState::Waiting, Some("0.1.14".into()), None)
        };
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        assert!(!state.core_update_waiting());
        state.set_phase(waiting(&[K::Backup, K::ModuleCall]));
        assert!(state.core_update_waiting());
        for server in [K::NonRunnerServer, K::ExtensionBridge, K::RunnerFormat] {
            state.set_phase(waiting(&[K::Backup, server]));
            assert!(!state.core_update_waiting(), "{server:?}");
        }
        state.set_phase(waiting(&[K::ServerUpdate, K::Provision]));
        assert!(state.core_update_waiting());
        state.set_phase(Phase::new(PhaseState::Applying, Some("0.1.14".into()), None));
        assert!(!state.core_update_waiting());
    }

    /// GUI 창 상태: GUI가 알린 값을 쓰고, 알린 적이 없으면 트레이 표시로 본다. GUI 등록이 없으면 닫힘
    #[tokio::test]
    async fn the_gui_window_state_follows_what_the_gui_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = crate::ipc::ClientRegistry::new();
        let state = state_in(tmp.path()).with_clients(registry.clone());
        assert!(!state.gui_window_open().await, "no GUI");
        let gui = registry.register(crate::ipc::ClientKind::Gui, Some(1), None).await;
        assert!(state.gui_window_open().await, "a GUI that never said otherwise has its window open");
        registry.set_resident(&gui, true).await;
        assert!(!state.gui_window_open().await, "a tray-only GUI");
        let _ = set_gui_window(State(state.clone()), Json(GuiWindowBody { open: true })).await;
        assert!(state.gui_window_open().await);
        let _ = set_gui_window(State(state.clone()), Json(GuiWindowBody { open: false })).await;
        assert!(!state.gui_window_open().await);
        registry.unregister(&gui).await;
        let _ = set_gui_window(State(state.clone()), Json(GuiWindowBody { open: true })).await;
        assert!(!state.gui_window_open().await, "a GUI that went away has no window");
    }

    /// 창이 열려 있어 미룬 적용은 창이 열려 있는 동안 그대로 기다리고, GUI가 창을 숨겼다고 알리면 다음 판정 차례(30초)를 기다리지 않고 이어 간다
    #[tokio::test]
    async fn an_apply_held_for_the_gui_window_resumes_as_soon_as_the_window_is_hidden() {
        let _staging = STAGING.lock().await;
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        std::fs::create_dir_all(&staging).unwrap();
        let staged = staging.join("saba-chan-gui-window-test.zip");
        std::fs::write(&staged, b"zip").unwrap();
        let pending = json!([{
            "component": "gui", "current_version": "0.1.0", "latest_version": "0.2.0",
            "update_available": true, "download_url": null, "asset_name": "saba-chan-gui-window-test.zip",
            "release_notes": null, "published_at": null, "downloaded": true,
            "downloaded_path": staged.to_string_lossy(), "installed": true
        }]);
        std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let registry = crate::ipc::ClientRegistry::new();
        let state = state_in(tmp.path()).with_clients(registry.clone());
        registry.register(crate::ipc::ClientKind::Gui, Some(1), None).await;
        super::restore_pending(&mut *state.manager.write().await);
        // 도는 적용 반복이 있다 (창을 숨겼다는 알림은 그 반복을 깨운다)
        assert!(state.begin_core_apply());
        state.take_apply_recheck();
        state.set_phase(Phase::ready(ReadyReason::GuiWindowOpen, None, None));
        let stop = tokio_util::sync::CancellationToken::new();

        let held = tokio::time::timeout(std::time::Duration::from_millis(300), super::hold_while_gui_window_open(&state, &stop)).await;
        assert!(held.is_err(), "the hold must last while the window is open");

        let waiting = tokio::spawn({
            let (state, stop) = (state.clone(), stop.clone());
            async move { super::hold_while_gui_window_open(&state, &stop).await }
        });
        tokio::task::yield_now().await;
        let _ = set_gui_window(State(state.clone()), Json(GuiWindowBody { open: false })).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting).await.expect("resumes without waiting for the next 30 s round").unwrap();
        let _ = std::fs::remove_file(staging.join("pending.json"));
        let _ = std::fs::remove_file(staged);
    }

    /// 같은 heartbeat 판 번호는 다시 확인하지 않는다. 본 것으로 남기는 것은 확인이 성공한 뒤다:
    /// 확인 중에는 같은 번호로 또 묻지 않고, 실패·취소·꺼짐이면 다음 heartbeat가 다시 확인한다
    #[test]
    fn release_ids_count_as_seen_only_after_a_successful_check() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        let ids = std::collections::BTreeMap::from([("stable".to_string(), 12u64)]);
        assert!(state.release_ids_to_check(std::collections::BTreeMap::new()).is_none());
        let checking = state.release_ids_to_check(ids.clone()).expect("new ids are checked");
        assert!(state.release_ids_to_check(ids.clone()).is_none(), "already being checked");
        state.finish_release_check(checking, None);
        assert!(LiveState::load_from(&tmp.path().join("live-update.json")).seen.is_empty());
        let checking = state.release_ids_to_check(ids.clone()).expect("a failed check is tried again");
        state.finish_release_check(checking, Some(Some(12)));
        assert!(state.release_ids_to_check(ids).is_none());
        let newer = std::collections::BTreeMap::from([("stable".to_string(), 13u64)]);
        let checking = state.release_ids_to_check(newer).unwrap();
        state.finish_release_check(checking, Some(Some(14)));
        assert_eq!(LiveState::load_from(&tmp.path().join("live-update.json")).seen["stable"], 13);
    }

    /// heartbeat가 부르지 않은 확인도 확인을 마친 가장 큰 릴리즈 id를 캐시 키로 쓴다
    #[test]
    fn other_checks_reuse_the_largest_confirmed_release_id() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        assert_eq!(state.known_release_id(), None);
        let ids = std::collections::BTreeMap::from([("stable".to_string(), 12u64), ("beta".to_string(), 15u64)]);
        let checking = state.release_ids_to_check(ids).unwrap();
        state.finish_release_check(checking, Some(Some(15)));
        assert_eq!(state.known_release_id(), Some(15));
    }

    /// 확인했는데 응답에 알린 번호가 없으면(캐시된 옛 응답) 본 것으로 두지 않고 캐시 수명 뒤 한 번 더 본다.
    /// 그때도 없으면(이 플랫폼 몫이 아님) 본 것으로 둔다: 한 번호를 두 번 넘게 묻지 않는다
    #[test]
    fn a_release_id_missing_from_the_answer_is_checked_once_more_later() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        let seen = || LiveState::load_from(&tmp.path().join("live-update.json")).seen;
        let ids = std::collections::BTreeMap::from([("stable".to_string(), 12u64)]);
        let checking = state.release_ids_to_check(ids.clone()).unwrap();
        state.finish_release_check(checking, Some(Some(11)));
        assert!(seen().is_empty(), "a stale answer does not count");
        assert!(state.release_ids_to_check(ids.clone()).is_none(), "not again within the cache lifetime");
        // 캐시 수명이 지났다
        state.release_unconfirmed.lock().unwrap().as_mut().unwrap().1 = std::time::Instant::now() - RELEASE_RECHECK;
        let checking = state.release_ids_to_check(ids.clone()).expect("checked once more");
        state.finish_release_check(checking, Some(None));
        assert_eq!(seen(), ids, "the second miss counts as seen");
        assert!(state.release_ids_to_check(ids).is_none());
    }

    /// 적용 반복이 도는 중에 들어온 확인은 버려지지 않는다: 반복이 다음 차례에 새로 받고, 그 뒤에 왔으면 끝난 뒤 다시 돈다
    #[test]
    fn a_check_during_the_apply_loop_leaves_a_recheck() {
        let state = UpdateState::new();
        assert!(state.begin_core_apply(), "nothing running: start");
        assert!(state.take_apply_recheck(), "the first round downloads");
        assert!(!state.take_apply_recheck());
        assert!(!state.begin_core_apply(), "one loop at a time");
        assert!(state.take_apply_recheck(), "the running loop picks up the new check");
        assert!(!state.end_core_apply(), "nothing came after the last round");
        assert!(state.begin_core_apply());
        assert!(!state.begin_core_apply());
        let _ = state.take_apply_recheck();
        assert!(!state.begin_core_apply());
        assert!(state.end_core_apply(), "a check after the last round runs the loop again");
        assert!(state.begin_core_apply());
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
        cfg.failed_release = Some("0.1.14".into());
        cfg.user_held = true;
        let dir = tempfile::tempdir().unwrap();
        save_updater_config_to(&ConfigFile::at(dir.path()), &cfg).unwrap();

        let loaded = load_updater_config_from(&ConfigFile::at(dir.path()));
        assert!(!loaded.enabled);
        assert_eq!(loaded.check_interval_hours, 6);
        assert!(loaded.auto_download);
        assert_eq!((loaded.failed_release.as_deref(), loaded.user_held), (Some("0.1.14"), true));

        // 고정을 지우면 두 기록이 함께 없어진다
        let mut cleared = loaded;
        assert!(cleared.clear_failed_release());
        save_updater_config_to(&ConfigFile::at(dir.path()), &cleared).unwrap();
        let section = ConfigFile::at(dir.path()).section("updates");
        assert!(section.get("user_held").is_none() && section["failed_release"].is_null());
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

    /// 복구 답: 받지 못한 것과 적용하지 못한 것은 오류로 모으고, 받아서 적용한 것만 고친 것(`repaired`)이다
    #[test]
    fn a_repair_reports_what_was_fixed_and_what_failed() {
        let plan = json!({ "ok": true, "verified": 9, "failed": 1, "skipped": 1, "repair": [{ "component": "cli" }, { "component": "module-minecraft" }], "unavailable": [] });
        let downloaded = json!({ "ok": false, "downloaded": [{ "component": "module-minecraft", "asset": "m.zip" }], "errors": [{ "component": "cli", "error": "net down" }] });
        let applied = json!({ "ok": true, "applied": ["Module: minecraft"], "needs_updater": [], "requires_updater": false, "errors": [] });
        let r = super::repair_result(plan.clone(), &downloaded, &applied);
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["repaired"], json!(["module-minecraft"]));
        assert_eq!(r["errors"], json!(["cli: net down"]));
        assert_eq!((r["verified"].clone(), r["requires_updater"].clone()), (json!(9), json!(false)));

        // 데몬·GUI가 끼면 고친 것은 업데이터가 적용한 뒤에 안다: 넘길 목록만 싣는다
        let core = json!({ "ok": true, "downloaded": [{ "component": "saba-core" }], "errors": [] });
        let handed = json!({ "ok": true, "applied": [], "needs_updater": ["saba-core"], "requires_updater": true, "errors": [] });
        let r = super::repair_result(plan, &core, &handed);
        assert_eq!((r["ok"].clone(), r["requires_updater"].clone(), r["needs_updater"].clone()), (json!(true), json!(true), json!(["saba-core"])));
        assert_eq!(r["repaired"], json!([]));
    }

    fn kept(version: &str, replaced_by: &str) -> saba_chan_updater_lib::live_apply::PreviousVersion {
        saba_chan_updater_lib::live_apply::PreviousVersion {
            version: version.into(),
            replaced_by: replaced_by.into(),
            updated_at: "2026-11-12T03:09:00Z".into(),
            copy: saba_chan_updater_lib::live_apply::KeptCopy { dir: "/d/s/1".into(), data_dir: "/d".into(), instances_dir: "/d/i".into() },
            exes: vec![("/d/s/1/exes/0/saba-core".into(), "/opt/saba/saba-core".into())],
            hold: None,
        }
    }

    /// 되돌릴 수 있는 판은 지금 코어가 그 직전 판을 바꾼 판일 때만 알린다 (그 뒤 설치기로 다시 깔았으면 맞지 않는다)
    #[test]
    fn the_previous_version_is_offered_only_over_the_version_that_replaced_it() {
        let body = super::rollback_body(Some(&kept("0.1.13", "0.1.14")), "0.1.14");
        assert_eq!(body["available"], json!(true));
        assert_eq!((body["version"].as_str(), body["current"].as_str()), (Some("0.1.13"), Some("0.1.14")));
        assert_eq!(body["updated_at"], json!("2026-11-12T03:09:00Z"));
        assert_eq!(super::rollback_body(Some(&kept("0.1.13", "0.1.14")), "0.1.15")["available"], json!(false));
        assert_eq!(super::rollback_body(None, "0.1.14")["available"], json!(false));
    }

    /// 앱의 "다른 버전 선택": 지금 판, 이 PC에 남긴 직전 판(되돌리기와 같은 규칙), 가장 새 판, 사용자 고정
    #[test]
    fn the_versions_body_lists_installed_kept_latest_and_held() {
        let cfg = |failed: Option<&str>, user_held: bool| UpdateConfig { channel: "beta".into(), failed_release: failed.map(Into::into), user_held, ..Default::default() };

        // 남긴 판 없음, 최신
        let none = super::versions_body(&cfg(None, false), "0.1.14", None, Vec::new(), None, false);
        assert_eq!(none, json!({ "ok": true, "channel": "beta", "installed": "0.1.14", "kept_previous": null, "latest": null, "held_until_newer_than": null }));

        // 직전 판을 남겼고 더 새 판이 있다. 그 판을 바꾼 코어가 아니면 남긴 판으로 보이지 않는다
        let previous = super::versions_body(&cfg(None, false), "0.1.14", Some(&kept("0.1.13", "0.1.14")), vec!["module-minecraft".into()], Some("0.1.15"), false);
        // 이미 아는 더 새 판(0.1.15)이 있으면 되돌린 뒤 그 판까지 고정한다
        assert_eq!(
            previous["kept_previous"],
            json!({ "version": "0.1.13", "updated_at": "2026-11-12T03:09:00Z", "needs_newer_core": ["module-minecraft"], "hold_until_newer_than": "0.1.15" })
        );
        let no_newer = super::versions_body(&cfg(None, false), "0.1.14", Some(&kept("0.1.13", "0.1.14")), Vec::new(), None, true);
        assert_eq!(no_newer["kept_previous"]["hold_until_newer_than"], json!("0.1.14"));
        assert_eq!(previous["latest"], json!("0.1.15"));
        assert!(super::versions_body(&cfg(None, false), "0.1.15", Some(&kept("0.1.13", "0.1.14")), Vec::new(), None, false)["kept_previous"].is_null());
        // 같은 판 다시 받기(`update_available`인데 같은 판)는 더 새 판이 아니다
        assert!(super::versions_body(&cfg(None, false), "0.1.14", None, Vec::new(), Some("0.1.14"), false)["latest"].is_null());

        // 되돌려 고정했고 확인한 새 판을 모른다: 가장 새 판은 고정한 판이다
        let held = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.13", None, Vec::new(), None, false);
        assert_eq!((held["latest"].clone(), held["held_until_newer_than"].clone()), (json!("0.1.14"), json!("0.1.14")));
        // 확인했는데 받을 새 판이 없다(고정한 판이 내려갔다): 바꿀 새 판은 없고 고정만 남는다
        let withdrawn = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.13", None, Vec::new(), None, true);
        assert_eq!((withdrawn["latest"].clone(), withdrawn["held_until_newer_than"].clone()), (Value::Null, json!("0.1.14")));
        // 고정 기준보다 새 코어가 나왔으면 고정은 풀린 것이다 (자동 적용을 끈 설정은 기록이 남아 있다)
        let released = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.13", None, Vec::new(), Some("0.1.15"), true);
        assert_eq!((released["latest"].clone(), released["held_until_newer_than"].clone()), (json!("0.1.15"), Value::Null));
        // 업데이트 실패로 되돌린 것은 고정이 아니다
        assert!(super::versions_body(&cfg(Some("0.1.14"), false), "0.1.13", None, Vec::new(), None, false)["held_until_newer_than"].is_null());

        // 남은 기록: 그 판이나 더 새 판을 깔았으면 고정이 아니다
        let stale = super::versions_body(&cfg(Some("0.1.14"), true), "0.1.14", None, Vec::new(), None, false);
        assert!(stale["held_until_newer_than"].is_null() && stale["latest"].is_null());
    }

    /// 사용자가 되돌려 고정한 코어를 받아 두어도 알약은 준비됨이 아니다 (자동 적용을 꺼도 같다). 앞서 남은 준비됨도 걷는다
    #[tokio::test]
    async fn a_held_core_leaves_no_ready_phase() {
        let _staging = STAGING.lock().await;
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        std::fs::create_dir_all(&staging).unwrap();
        let staged = staging.join("saba-core-held-test.zip");
        std::fs::write(&staged, b"zip").unwrap();
        let pending = json!([{
            "component": "core_daemon", "current_version": "0.1.13", "latest_version": "99.0.0",
            "update_available": true, "download_url": null, "asset_name": "saba-core-held-test.zip",
            "release_notes": null, "published_at": null, "downloaded": true,
            "downloaded_path": staged.to_string_lossy(), "installed": true
        }]);
        std::fs::write(staging.join("pending.json"), pending.to_string()).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        for auto_apply in [true, false] {
            {
                let mut mgr = state.manager.write().await;
                super::restore_pending(&mut mgr);
                let mut cfg = mgr.get_config();
                cfg.auto_apply = auto_apply;
                cfg.failed_release = Some("99.0.0".into());
                cfg.user_held = true;
                mgr.update_config(cfg);
            }
            state.set_phase(Phase::ready(ReadyReason::FailedRelease, Some("99.0.0".into()), None));
            super::core_apply(&state).await;
            assert_eq!(state.phase(), Phase::default(), "auto_apply={auto_apply}");
            assert!(!state.updater_launched());
        }

        std::fs::remove_file(staging.join("pending.json")).unwrap();
        std::fs::remove_file(&staged).unwrap();
    }

    /// 되돌려도 모듈·익스텐션은 그대로 남는다: 직전 판 코어가 읽지 않을 것(그 판보다 새 `saba-core`를 요구)을 미리 알린다
    #[test]
    fn the_rollback_names_addons_the_previous_core_will_not_load() {
        let needs = |min: &str| std::collections::HashMap::from([("saba-core".to_string(), min.to_string())]);
        let addons = super::addons_needing_core(
            "0.1.13",
            vec![
                ("module-minecraft".to_string(), needs(">=0.1.14")),
                ("module-palworld".to_string(), needs(">=0.1.10")),
                ("ext-steamcmd".to_string(), std::collections::HashMap::new()),
                ("ext-music".to_string(), needs("0.1.14")),
            ],
        );
        assert_eq!(addons, vec!["module-minecraft".to_string(), "ext-music".to_string()]);
    }

    /// 남긴 직전 판이 없으면 업데이터를 띄우지 않고(데몬도 끄지 않고) 그 까닭을 돌려준다
    #[tokio::test]
    async fn rolling_back_without_a_previous_version_launches_nothing() {
        let state = UpdateState::default();
        for previous in [None, Some(kept("0.1.13", "9.9.9"))] {
            let resp = super::roll_back_with(state.clone(), super::LaunchUpdaterRequest::default(), previous).await;
            assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
            let body = crate::ipc::internal::into_internal(resp).await.body;
            assert_eq!(body["error_code"], json!("no_previous_version"));
        }
        assert!(!state.updater_launched.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// 판 고르기 창의 목록: 새 판부터, 내린 판은 고를 수 없게 표시하고, 가장 새 판은 내리지 않은 것 중에서 고른다
    #[test]
    fn the_version_list_marks_installed_latest_and_yanked_versions() {
        let entry = |v: &str| -> saba_chan_updater_lib::ManifestComponent {
            serde_json::from_value(json!({ "version": v, "asset": "m.zip", "sha256": "a".repeat(64), "url": "https://cdn.example/m.zip" })).unwrap()
        };
        let list = saba_chan_updater_lib::relay::AddonVersions {
            versions: vec![entry("0.1.13"), entry("0.1.12"), entry("0.1.10")],
            yanked: vec!["0.1.15".into(), "0.1.11".into()],
            excluded: vec!["0.1.14".into()],
        };
        let pin = addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() };
        let body = addon_versions_body("module-minecraft", "0.1.14", Some("0.1.12"), &list, Some(&pin));
        let rows: Vec<(String, bool, bool, bool)> = body["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["version"].as_str().unwrap().to_string(), v["installed"] == true, v["latest"] == true, v["yanked"] == true))
            .collect();
        let row = |v: &str, installed, latest, yanked| (v.to_string(), installed, latest, yanked);
        assert_eq!(
            rows,
            [row("0.1.15", false, false, true), row("0.1.13", false, true, false), row("0.1.12", true, false, false), row("0.1.11", false, false, true), row("0.1.10", false, false, false)]
        );
        assert_eq!((body["installed"].as_str(), body["core"].as_str()), (Some("0.1.12"), Some("0.1.14")));
        assert_eq!(body["excluded"], json!(["0.1.14"]));
        assert_eq!(body["pinned"], json!({ "version": "0.1.12", "until_newer_than": "0.1.13" }));
    }

    /// 판 고르기 시험용 릴레이: 모듈 하나의 0.1.13(`newest_requires`를 단다)·0.1.12(받을 zip까지)와 내린 0.1.11
    async fn addon_versions_relay(name: &str, newest_requires: Value) -> String {
        use axum::routing::get;
        use sha2::Digest;
        static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let private = KEY.get_or_init(|| {
            let (private, public) = saba_chan_updater_lib::signature::generate_key_pair().unwrap();
            // edition 2024: 환경 변수 쓰기는 unsafe다. 테스트 준비에서만 쓴다
            unsafe { std::env::set_var(saba_chan_updater_lib::signature::TEST_PUBKEY_ENV, &public) };
            private
        });
        let zip_of = |version: &str| {
            use std::io::Write;
            let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            out.start_file("module.toml", zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(format!("[module]\nname = \"{name}\"\nversion = \"{version}\"\n").as_bytes()).unwrap();
            out.finish().unwrap().into_inner()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let key = format!("module-{name}");
        let sign = |m: Value| serde_json::to_value(saba_chan_updater_lib::signature::sign(m, saba_chan_updater_lib::signature::TEST_KEY_ID, private).unwrap()).unwrap();
        let mut app = axum::Router::new();
        let mut versions = Vec::new();
        for version in ["0.1.13", "0.1.12"] {
            let bytes = zip_of(version);
            let asset = format!("{key}-{version}.zip");
            let mut entry = json!({ "key": key, "version": version, "asset": asset, "sha256": hex::encode(sha2::Sha256::digest(&bytes)),
                                    "url": format!("{base}/r2/{asset}") });
            if version == "0.1.13" && !newest_requires.is_null() {
                entry["requires"] = newest_requires.clone();
            }
            versions.push(sign(entry));
            app = axum::Router::route(app, &format!("/r2/{asset}"), get(move || async move { bytes }));
        }
        let body = json!({ "key": key, "versions": versions, "yanked": [sign(json!({ "key": key, "version": "0.1.11", "yanked": true }))], "excluded": [] });
        // 메서드 꼴로 부르지 않는다: 라우트 분류 시험(gate, CLI route_coverage)이 이 파일의 메서드 꼴 라우트를 데몬 라우트로 읽는다
        app = axum::Router::route(app, "/v1/addons/versions", get(move || async move { Json(body) }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    /// 판 고르기: 고른 판을 받아 바꾸고 바꾸기 전의 가장 새 판까지 고정한다. 내린 판은 고르지 못하고, 손으로 고친 폴더는 덮어쓰지 않으며,
    /// 고정을 풀면 가장 새 판으로 바로 돌아간다
    #[tokio::test]
    async fn choosing_a_module_version_installs_and_pins_it_and_unpinning_returns_to_the_newest() {
        let _staging = STAGING.lock().await;
        let name = format!("pick{}", std::process::id());
        let key = format!("module-{name}");
        let base = addon_versions_relay(&name, Value::Null).await;
        let modules = crate::plugin::resolve_modules_dir();
        let dir = modules.join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{name}\"\nversion = \"0.1.13\"\n")).unwrap();
        saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("module.toml")]).unwrap();
        saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        let cfg = saba_chan_updater_lib::UpdateConfig { relay_url: Some(base), ..Default::default() };
        *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, modules.to_str().unwrap());
        let call = |r: axum::response::Response| async { crate::ipc::internal::into_internal(r).await };
        let pin = |version: &str| pin_addon(State(state.clone()), Path(key.clone()), axum::body::Bytes::from(json!({ "version": version }).to_string()));
        let installed = || std::fs::read_to_string(dir.join("module.toml")).unwrap();

        let refused = call(pin("0.1.11").await).await;
        assert_eq!((refused.status, refused.body["error_code"].as_str()), (axum::http::StatusCode::CONFLICT, Some("version_unavailable")));

        let picked = call(pin("0.1.12").await).await;
        assert_eq!(picked.body, json!({ "ok": true, "state": "applied", "version": "0.1.12", "waiting_for": [] }));
        assert!(installed().contains("0.1.12"), "{}", installed());
        let choice = addon_pins::get(&key);
        assert_eq!(choice.pinned, Some(addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() }));
        assert_eq!(choice.pending_version, None, "nothing is waiting once the chosen version is in place");

        // 사용자가 넣은 파일이 있으면 폴더를 바꾸지 않고 고정도 그대로 둔다
        std::fs::write(dir.join("my-notes.txt"), "mine").unwrap();
        let refused = call(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
        assert_eq!(refused.body["error_code"], "module_modified", "{}", refused.body);
        assert!(installed().contains("0.1.12"));
        assert!(addon_pins::get(&key).pinned.is_some());

        std::fs::remove_file(dir.join("my-notes.txt")).unwrap();
        let back = call(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
        assert_eq!(back.body, json!({ "ok": true, "state": "applied", "version": "0.1.13", "waiting_for": [] }));
        assert!(installed().contains("0.1.13"));
        assert_eq!(addon_pins::get(&key), addon_pins::AddonChoice::default());

        // 가장 새 판을 고르면 고정 없이 바꾼다: 고정을 푼 것과 같다
        assert_eq!(call(pin("0.1.12").await).await.body["state"], "applied");
        let newest = call(pin("0.1.13").await).await;
        assert_eq!(newest.body, json!({ "ok": true, "state": "applied", "version": "0.1.13", "waiting_for": [] }));
        assert!(installed().contains("0.1.13"));
        assert_eq!(addon_pins::get(&key), addon_pins::AddonChoice::default());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(modules.join(format!(".{name}.saba-old")));
    }

    /// 시험용: 인스턴스가 켜진 것으로 보이게 하거나 끈다
    async fn set_running(state: &UpdateState, id: &str, on: bool) {
        let sup = state.supervisor.as_ref().unwrap().read().await;
        if on {
            sup.tracker.track(id, std::process::id()).unwrap();
        } else {
            let _ = sup.tracker.untrack(id);
        }
    }

    /// 켜진 서버가 쓰는 모듈은 고른 판을 받아 두고 서버가 꺼질 때까지 기다린다(`waiting`). 꺼진 뒤 교체 한 차례가 바꾸고 기다림을 지우며,
    /// 업데이트 확인을 꺼 두어도 고른 것은 바꾼다. 기다리는 사이 고정을 풀어 깔린 판으로 끝나면 받아 둔 판은 버려, 서버가 꺼져도 바뀌지 않는다
    #[tokio::test]
    async fn a_chosen_version_waits_for_the_running_server_and_cancelling_it_keeps_the_installed_one() {
        use std::sync::atomic::Ordering;
        let _staging = STAGING.lock().await;
        // 교체 반복은 이 시험이 직접 한 차례씩 돌린다
        ADDON_REPLACING.store(true, Ordering::SeqCst);
        let name = format!("wait{}", std::process::id());
        let key = format!("module-{name}");
        let base = addon_versions_relay(&name, Value::Null).await;
        let modules = crate::plugin::resolve_modules_dir();
        let dir = modules.join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{name}\"\nversion = \"0.1.13\"\n")).unwrap();
        saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("module.toml")]).unwrap();
        saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut state = state_in(tmp.path());
        let cfg = saba_chan_updater_lib::UpdateConfig { relay_url: Some(base), enabled: true, ..Default::default() };
        *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, modules.to_str().unwrap());
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let server = crate::instance::ServerInstance::new("srv", &name);
        let id = server.id.clone();
        sup.instance_store.add(server).unwrap();
        state.supervisor = Some(Arc::new(RwLock::new(sup)));
        let call = |r: axum::response::Response| async { crate::ipc::internal::into_internal(r).await };
        let pin = |version: &str| pin_addon(State(state.clone()), Path(key.clone()), axum::body::Bytes::from(json!({ "version": version }).to_string()));
        let installed = || std::fs::read_to_string(dir.join("module.toml")).unwrap();
        let waiting = json!({ "ok": true, "state": "waiting", "version": "0.1.12", "waiting_for": ["srv"] });

        set_running(&state, &id, true).await;
        assert_eq!(call(pin("0.1.12").await).await.body, waiting);
        assert!(installed().contains("0.1.13"), "the running server keeps its version");
        assert_eq!(addon_pins::get(&key).pending_version.as_deref(), Some("0.1.12"));

        // 기다리는 사이 자동 업데이트로 돌아가면 깔린 0.1.13 그대로 끝나고, 서버가 꺼져도 받아 둔 0.1.12로 바꾸지 않는다
        let back = call(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
        assert_eq!(back.body["state"], "applied", "{}", back.body);
        assert_eq!(addon_pins::get(&key), addon_pins::AddonChoice::default());
        set_running(&state, &id, false).await;
        assert_eq!(replace_addons_once(&state).await, Leftover::default());
        assert!(installed().contains("0.1.13"), "a cancelled switch is not applied: {}", installed());

        // 업데이트 확인을 꺼 두어도 고른 판은 서버가 꺼진 뒤 바꾼다
        let mut cfg = state.manager.read().await.get_config();
        cfg.enabled = false;
        state.manager.write().await.update_config(cfg);
        set_running(&state, &id, true).await;
        assert_eq!(call(pin("0.1.12").await).await.body, waiting);
        assert_eq!(replace_addons_once(&state).await, Leftover { held: true, failed: false });
        assert!(installed().contains("0.1.13"));
        set_running(&state, &id, false).await;
        assert_eq!(replace_addons_once(&state).await, Leftover::default());
        assert!(installed().contains("0.1.12"), "{}", installed());
        let choice = addon_pins::get(&key);
        assert_eq!(choice.pending_version, None, "the wait ends once the chosen version is in place");
        assert_eq!(choice.pinned, Some(addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() }));

        ADDON_REPLACING.store(false, Ordering::SeqCst);
        let _ = addon_pins::update(&key, |c| *c = addon_pins::AddonChoice::default());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(modules.join(format!(".{name}.saba-old")));
    }

    /// 고정을 풀 때도 가장 새 판의 요구 조건을 깔린 것과 대조한다: 맞지 않으면 받지 않고 거절하고 고정은 그대로 둔다
    #[tokio::test]
    async fn unpinning_refuses_a_newest_version_whose_requirements_are_unmet() {
        let _staging = STAGING.lock().await;
        let name = format!("needs{}", std::process::id());
        let key = format!("module-{name}");
        let base = addon_versions_relay(&name, json!({ "ext-not-installed": ">=1.0.0" })).await;
        let modules = crate::plugin::resolve_modules_dir();
        let dir = modules.join(&name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{name}\"\nversion = \"0.1.12\"\n")).unwrap();
        saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("module.toml")]).unwrap();
        saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
        let pinned = addon_pins::AddonPin { version: "0.1.12".into(), until_newer_than: "0.1.13".into() };
        addon_pins::update(&key, |c| c.pinned = Some(pinned.clone())).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        let cfg = saba_chan_updater_lib::UpdateConfig { relay_url: Some(base), ..Default::default() };
        *state.manager.write().await = saba_chan_updater_lib::UpdateManager::new(cfg, modules.to_str().unwrap());

        let refused = crate::ipc::internal::into_internal(unpin_addon(State(state.clone()), Path(key.clone())).await).await;
        assert_eq!((refused.status, refused.body["error_code"].as_str()), (axum::http::StatusCode::CONFLICT, Some("requires_unmet")), "{}", refused.body);
        assert!(std::fs::read_to_string(dir.join("module.toml")).unwrap().contains("0.1.12"));
        assert_eq!(addon_pins::get(&key).pinned, Some(pinned));

        let _ = addon_pins::update(&key, |c| *c = addon_pins::AddonChoice::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
