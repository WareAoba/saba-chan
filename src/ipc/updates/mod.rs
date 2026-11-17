//! 데몬 내장 업데이트 API — `/api/updates/*`
//!
//! 업데이트 확인(check)과 다운로드(download)를 데몬 프로세스 내에서 수행합니다.
//! 파일 적용(apply)은 별도 업데이터 프로세스에서 처리합니다.
//!
//! 확인은 데몬이 한다: 릴레이 heartbeat의 판 번호(`latestReleases`)가 바뀌면 확인하고(`on_latest_releases`), 연결과 상관없이
//! 24시간마다 한 번 더 본다(`spawn_update_scheduler`). GUI·CLI는 `/api/updates/status`를 읽어 표시만 한다.
//!
//! ## 엔드포인트
//! - `GET  /api/updates/status`              — 캐시된 업데이트 상태 조회
//! - `POST /api/updates/check`               — 업데이트 확인 (릴레이의 서명된 manifest)
//! - `POST /api/updates/download`            — 선택 컴포넌트 다운로드 (본문 없으면 전부)
//! - `POST /api/updates/apply`               — 모듈 등은 데몬이 바로 적용, 재시작이 필요한 것은
//!   `requires_updater`로 돌려주고 실제 적용은 `launch-updater`가 한다 (본문 없으면 전부)
//! - `POST /api/updates/launch-updater`      — 데몬이 업데이터를 띄우고 스스로 종료
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
//!
//! 이 파일은 공유 상태(`UpdateState`)·라우터·상태와 확인·주기 확인을 둔다. 나머지는 하위 모듈에 있다:
//! `addons`(모듈·익스텐션 교체와 판 고르기), `apply`(앱 업데이트 적용과 받기·적용·설치), `handoff`(업데이터에
//! 넘기기·되돌리기·마지막 결과), `repair`(무결성·복구), `config`(업데이트 설정)

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
    addon_pins, is_gui_exe, Component, DownloadProgress, IntegrityStatus, UpdateConfig, UpdateManager,
};

use super::update_phase::{self, Decision, LiveState, Phase, PhaseState, ReadyReason};

mod addons;
mod apply;
mod config;
mod handoff;
mod repair;
#[cfg(test)]
mod tests;

pub use addons::spawn_deferred_apply;
use addons::{get_addon_versions, is_addon, pin_addon, spawn_addon_replace, unpin_addon};
use apply::{
    RELEASE_RECHECK, apply_updates, download_components, get_daemon_update, install_updates, put_daemon_update,
    spawn_core_apply,
};
pub(crate) use config::load_updater_config;
use config::{get_config, save_updater_config, set_config};
use handoff::{clear_last_result, get_last_result, get_rollback, get_versions, launch_updater, restart_now, roll_back};
use repair::{check_integrity, get_repair, repair};
#[cfg(test)]
use addons::{
    ADDON_HOLD_RETRY, ADDON_REPLACING, Leftover, addon_retry_wait, addon_versions_body, apply_deferred_once,
    replace_addons_once, replace_addons_until_done, split_by_busy,
};
#[cfg(test)]
use apply::{ApplyRequest, DownloadRequest, body_or_default, core_apply, restore_pending};
#[cfg(test)]
use config::{adopt_config, load_updater_config_from, save_updater_config_to};
#[cfg(test)]
use handoff::{
    LaunchUpdaterRequest, addons_needing_core, close_guis, gui_must_restart, installer_applies, last_result_at,
    last_result_body, only_resident_guis, record_daemon_apply, relaunch_target, restart_plan, roll_back_with,
    rollback_body, running_guis, spawn_and_confirm, updater_args, updater_binary_near, versions_body,
};
#[cfg(test)]
use repair::repair_result;

// ── 공유 상태 ──

/// heartbeat `latestReleases` 중 이 데몬이 보는 채널별 릴리즈 id
type ReleaseIds = std::collections::BTreeMap<String, u64>;

/// 데몬 업데이트 매니저 — Arc<RwLock>으로 공유
#[derive(Clone)]
pub struct UpdateState {
    pub manager: Arc<RwLock<UpdateManager>>,
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
    /// 내려가기 직전 릴레이에 "업데이트로 잠깐 끊김"을 알린다
    pub relay: Option<crate::relay_agent::RelayAgent>,
    /// 확인 중인 heartbeat 판 번호. 확인이 성공해야 본 것(`LiveState::seen`)으로 남긴다
    release_check: Arc<std::sync::Mutex<Option<std::collections::BTreeMap<String, u64>>>>,
    /// 확인했지만 응답에 그 번호가 담기지 않은 판 번호와 그때 (`RELEASE_RECHECK` 뒤 한 번 더 본다)
    release_unconfirmed: Arc<std::sync::Mutex<Option<(ReleaseIds, std::time::Instant)>>>,
    /// 확인 중인 heartbeat 애드온 판 번호 (`latestAddons`, 한 번에 하나)
    addon_check: Arc<std::sync::Mutex<Option<String>>>,
    /// 앱 업데이트 적용 반복이 도는 중인가 (`spawn_core_apply`, 한 번에 하나)
    core_applying: Arc<std::sync::atomic::AtomicBool>,
    /// 상태가 바뀌었을 수 있다: 상태 알림 반복(`spawn_status_events`)을 깨운다
    status_changed: Arc<tokio::sync::Notify>,
    /// 마지막으로 `update.status`로 알린 본문
    announced: Arc<std::sync::Mutex<Option<Value>>>,
    /// 적용 반복이 새로 받아 볼 확인이 있었다: 반복이 다음 차례에 받고, 끝난 뒤에 왔으면 반복을 다시 띄운다
    apply_recheck: Arc<std::sync::atomic::AtomicBool>,
}

impl UpdateState {
    /// 내장 기본값으로 업데이트 매니저 생성
    pub fn new() -> Self {
        let cfg = load_updater_config();
        let modules_dir = crate::plugin::resolve_modules_dir();
        let mut mgr = UpdateManager::new(cfg, &modules_dir.to_string_lossy());
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
        // 받는 진행은 지금 붙은 클라이언트에게만 알린다: 클라이언트는 진행을 묻지 않는다
        mgr.set_download_listener(Arc::new(|p: &DownloadProgress| crate::events::emit_live("update.progress", progress_event(p))));
        let manager = Arc::new(RwLock::new(mgr));
        Self {
            manager,
            supervisor: None,
            extension_manager: None,
            stop_requested: None,
            clients: None,
            updater_launched: Arc::default(),
            modules_updated: Arc::default(),
            live: Arc::new(std::sync::Mutex::new(live)),
            live_path: Arc::new(live_path),
            gui_window: Arc::default(),
            relay: None,
            release_check: Arc::default(),
            release_unconfirmed: Arc::default(),
            addon_check: Arc::default(),
            core_applying: Arc::default(),
            apply_recheck: Arc::default(),
            status_changed: Arc::default(),
            announced: Arc::default(),
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

    /// 이번에 본 업데이트 (구성요소 키, 새 판)를 남기고, 지난번 알린 것과 달라 새로 알릴 것이 있으면 true.
    /// 같은 구성요소라도 더 새 판이 나오면 다시 알린다. 상태 파일에 남겨 데몬을 다시 켜도 같은 것을 또 알리지 않는다
    fn note_updates(&self, found: Vec<(String, Option<String>)>) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if live.announced == found {
            return false;
        }
        let announce = !found.is_empty();
        live.announced = found;
        live.save_to(&self.live_path);
        announce
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
        drop(live);
        self.status_may_have_changed();
    }

    /// 클라이언트가 보는 상태(단계·구성요소·고정·번역 판)를 바꿨다: 상태 알림 반복이 다시 보고 달라졌으면 `update.status`를 낸다.
    /// 잠금을 쥔 채 불러도 된다 (기다리지 않는다)
    fn status_may_have_changed(&self) {
        self.status_changed.notify_one();
    }

    /// 마지막으로 알린 상태에서 구성요소 하나(`module-<이름>` 등)의 새 판. 확인이 찾았을 때만.
    /// 업데이트 잠금을 기다리지 않는다: 모듈 목록이 적용 중에도 바로 답하게
    pub fn announced_latest(&self, key: &str) -> Option<String> {
        let last = self.announced.lock().unwrap_or_else(|e| e.into_inner());
        let comp = last.as_ref()?["components"].as_array()?.iter().find(|c| c["component"] == key)?;
        (comp["update_available"] == true).then(|| comp["latest_version"].as_str().map(str::to_string)).flatten()
    }

    /// 상태 본문이 지난번 알린 것과 다르면 `update.status`로 낸다
    async fn announce_status(&self) {
        let body = status_body(self).await;
        {
            let mut last = self.announced.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref() == Some(&body) {
                return;
            }
            *last = Some(body.clone());
        }
        crate::events::emit("update.status", body);
    }

    /// 코어 업데이트가 데몬 작업만 기다리는 중이다: 새 예약 작업(백업·게임 서버 업데이트)은 업데이트 뒤로 미룬다.
    /// 켜진 서버를 기다리면 며칠이 걸릴 수 있으니 예약 작업은 그대로 돈다
    pub fn core_update_waiting(&self) -> bool {
        let phase = self.phase();
        phase.state == PhaseState::Waiting
            && !phase.waiting_for.is_empty()
            && phase.waiting_for.iter().all(|b| b.kind.is_daemon_job())
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

    /// heartbeat `latestAddons`에서 `channel`의 값이 확인을 마친 것과 다르고 확인 중이 아니면 그 값 (확인을 시작한다)
    fn addon_revision_to_check(&self, revs: &Value, channel: &str) -> Option<String> {
        let rev = revs.get(channel)?.as_str()?.to_string();
        if self.live.lock().unwrap_or_else(|e| e.into_inner()).addons_seen.as_deref() == Some(rev.as_str()) {
            return None;
        }
        let mut checking = self.addon_check.lock().unwrap_or_else(|e| e.into_inner());
        if checking.is_some() {
            return None;
        }
        *checking = Some(rev.clone());
        Some(rev)
    }

    /// 애드온 판 번호 확인을 마쳤다. 성공했을 때만 본 것으로 남긴다 (실패면 다음 heartbeat가 다시 확인한다)
    fn finish_addon_check(&self, rev: String, checked: bool) {
        *self.addon_check.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if checked {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            live.addons_seen = Some(rev);
            live.save_to(&self.live_path);
        }
    }

    /// 적용 반복을 시작해도 되는가. 이미 돌고 있으면 그 반복이 새로 받아 보도록 남긴다
    fn begin_core_apply(&self) -> bool {
        use std::sync::atomic::Ordering;
        // 먼저 남긴다: 반복이 막 끝나는 중이어도 `end_core_apply`가 이것을 본다
        self.apply_recheck.store(true, Ordering::SeqCst);
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

// ── 라우터 ──

/// `/api/updates/*` 라우트를 포함하는 axum Router 생성
pub fn updates_router(state: UpdateState) -> Router {
    Router::new()
        .route("/api/updates/status", get(get_status))
        .route("/api/updates/check", post(check_updates))
        .route("/api/updates/download", post(download_components))
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

// ── 핸들러 ──

/// GET /api/updates/status — 캐시된 상태 반환 (네트워크 없음)
///
/// Locales 컴포넌트는 사용자에게 비표시 — 백그라운드 자동 다운로드/적용 대상이므로
/// 응답의 `components`, `updates_available` 에서 제외합니다.
async fn get_status(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    Json(status_body(&state).await)
}

/// 상태 본문. 조회(`GET /api/updates/status`)와 상태 알림(`update.status`)이 같은 모양을 쓴다
async fn status_body(state: &UpdateState) -> Value {
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
            let choice = pins.get(&key).cloned().unwrap_or_default();
            json!({
                "display_name": c.component.display_name(),
                "current_version": c.current_version,
                "latest_version": c.latest_version,
                "update_available": c.update_available,
                // 바꿔 끼울 새 판 (고정·바꾸기 대기면 null). 화면의 "업데이트 있음"은 이것으로 판정한다
                "update_version": addon_update_version(&choice, c),
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

    json!({
        "success": true,
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
    })
}

/// 구성요소 하나의 바꿔 끼울 새 판 (`AddonChoice::update_version`). 확인이 새 판을 찾았을 때만
fn addon_update_version(choice: &addon_pins::AddonChoice, c: &saba_chan_updater_lib::ComponentVersion) -> Option<String> {
    choice.update_version(c.latest_version.as_deref().filter(|_| c.update_available), &c.current_version)
}

/// 상태가 바뀔 때마다(`UpdateState::status_may_have_changed`) 상태 본문을 다시 만들어, 달라졌으면 `update.status`로 낸다.
/// 띄우자마자 한 번 낸다: 데몬이 다시 뜬 뒤 마지막 번호로 다시 붙는 클라이언트가 버퍼에서 받는다
pub fn spawn_status_events(state: UpdateState, shutdown: tokio_util::sync::CancellationToken) {
    /// 한꺼번에 바뀌는 것(확인 → 받기 → 단계)을 한 번에 알린다
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(200);
    tokio::spawn(async move {
        loop {
            state.announce_status().await;
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = state.status_changed.notified() => {}
            }
            tokio::time::sleep(SETTLE).await;
        }
    });
}

/// 받기·적용·판 고르기가 어느 길로 끝나든 상태가 바뀌었을 수 있다고 알린다 (`UpdateState::status_may_have_changed`)
struct StatusTouched<'a>(&'a UpdateState);

impl Drop for StatusTouched<'_> {
    fn drop(&mut self) {
        self.0.status_may_have_changed();
    }
}

/// 받는 진행 한 번 → `update.progress` 본문. 퍼센트는 데몬이 계산한다(전체 크기를 모르면 null)
fn progress_event(p: &DownloadProgress) -> Value {
    let percent = (p.total_bytes > 0).then(|| (p.bytes_received as f64 * 100.0 / p.total_bytes as f64).round() as u64);
    json!({
        "component": p.component,
        "active": p.active,
        "bytesReceived": p.bytes_received,
        "totalBytes": p.total_bytes,
        "percent": percent,
    })
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
                "success": true,
                "updates_available": update_names.len(),
                "update_names": update_names,
                "components": components,
                "last_check": status.last_check,
            }))
            .into_response()
        }
        Err(e) => crate::ipc::handlers::error(axum::http::StatusCode::BAD_GATEWAY, "update_check_failed", e.to_string()),
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
    let (status, latest_id) = saba_chan_updater_lib::check_shared(&state.manager, hint).await?;
    announce_new_updates(state, &status);
    state.status_may_have_changed();

    // 모듈·익스텐션은 묻지 않고 바꿔 끼운다. 확인을 끈 설정이면 주기 확인 자체가 돌지 않는다
    if status.components.iter().any(|c| c.update_available && is_addon(&c.component)) {
        spawn_addon_replace(state.clone());
    }

    let has_locale_update = status.components.iter()
        .any(|c| matches!(c.component, Component::Locales) && c.update_available);
    if has_locale_update {
        let state = state.clone();
        tokio::spawn(async move {
            silent_apply_locales(&state.manager).await;
            // 번역 판이 바뀌었다
            state.status_may_have_changed();
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

/// 데몬 기동 후 업데이트를 주기적으로 확인한다. 24시간마다.
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
            // 사바 스토리지 목록·아이콘도 이때 받아 둔다: 창을 열면 바로 보이게
            crate::ipc::store_cache::store().refresh_in_background(&cfg.channel);
            if !cfg.enabled {
                wait = RETRY;
                continue;
            }
            // 연결된 데몬은 heartbeat의 판 번호로 알게 된다. 이것은 연결과 상관없는 안전망이다
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

/// 릴레이 heartbeat의 채널별 모듈·익스텐션 카탈로그 판 번호(`latestAddons`). 이 데몬 채널의 값이 바뀌면
/// 0~60초 사이에서 늦춘 뒤 한 번 확인한다: 확인이 새 판을 찾으면 묻지 않고 바꿔 끼운다 (`check_and_follow_up`)
pub fn on_latest_addons(state: &UpdateState, revs: &Value, shutdown: tokio_util::sync::CancellationToken) {
    let revs = revs.clone();
    let state = state.clone();
    tokio::spawn(async move {
        let cfg = state.manager.read().await.get_config();
        if !cfg.enabled {
            return;
        }
        let Some(rev) = state.addon_revision_to_check(&revs, &cfg.channel) else {
            return;
        };
        let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_millis()) as u64 * 60;
        let checked = tokio::select! {
            _ = shutdown.cancelled() => false,
            checked = async {
                tokio::time::sleep(std::time::Duration::from_millis(jitter)).await;
                check_and_follow_up(&state).await.map_err(|e| tracing::warn!("[Updates] Check after a new addon publish failed: {}", e)).is_ok()
            } => checked,
        };
        state.finish_addon_check(rev, checked);
    });
}

/// 사용자에게 보이는 업데이트가 지난번 알린 것과 달라졌을 때만 `update.available`
fn announce_new_updates(state: &UpdateState, status: &saba_chan_updater_lib::UpdateStatus) {
    let visible: Vec<_> = status.components.iter()
        .filter(|c| c.update_available && !matches!(c.component, Component::Locales))
        .collect();
    let found = visible.iter().map(|c| (c.component.manifest_key(), c.latest_version.clone())).collect();
    if state.note_updates(found) {
        // `/api/updates/check`의 `update_names`와 같은 표시 이름
        let names: Vec<String> = visible.iter().map(|c| c.component.display_name()).collect();
        crate::events::emit("update.available", json!({ "components": names }));
    }
}

/// Locales 컴포넌트를 사용자 비표시로 다운로드+적용합니다.
///
/// 실패 시에도 에러만 로깅 — 사용자 흐름에 영향 없음.
async fn silent_apply_locales(manager: &Arc<RwLock<UpdateManager>>) {
    // 1. 다운로드
    let download_ok = {
        match saba_chan_updater_lib::download_shared(manager, &Component::Locales).await {
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
        Ok(()) => {
            tracing::info!("[Updates] Locales silently applied");
        }
        Err(e) => {
            tracing::warn!("[Updates] Silent locales apply failed: {}", e);
        }
    }
}

/// POST /api/updates/phase/ack — 알약의 `done`을 지운다 (다른 단계는 그대로). 답은 지금 단계
async fn ack_phase(State(state): State<UpdateState>) -> impl IntoResponse {
    if state.phase().state == PhaseState::Done {
        state.set_phase(Phase::default());
    }
    Json(json!({ "success": true, "phase": state.phase() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuiWindowBody {
    open: bool,
}

/// PUT /api/updates/gui-window {open} — GUI가 창을 보이거나 숨길 때(트레이) 알린다. 업데이트 뒤 GUI를 창으로 다시 띄울지
/// 트레이로 띄울지 정할 때 쓴다 (적용은 창이 열려 있어도 한다)
async fn set_gui_window(State(state): State<UpdateState>, Json(body): Json<GuiWindowBody>) -> impl IntoResponse {
    *state.gui_window.lock().unwrap_or_else(|e| e.into_inner()) = Some(body.open);
    Json(json!({ "success": true, "open": body.open }))
}
