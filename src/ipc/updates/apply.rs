//! 앱 업데이트 적용: 묻지 않고 적용하는 반복(`spawn_core_apply`), 원격 업데이트 설정(`/api/daemon/update`),
//! 받기·적용·한 번에 설치(`/api/updates/download`·`apply`·`install`)

use super::*;
use super::config::adopt_config;
use super::handoff::record_daemon_apply;

/// 막는 것이 있어 미룬 앱 업데이트 적용을 다시 판정하는 간격
const CORE_HOLD_RETRY: std::time::Duration = std::time::Duration::from_secs(30);
/// 모듈 호출만 막고 있으면(상태 조회처럼 금방 끝나는 것) 이만큼씩 몇 번 더 보고 나서 기다린다고 알린다
const MODULE_CALL_SETTLE: std::time::Duration = std::time::Duration::from_secs(2);
/// 릴레이에 알리는 업데이트로 끊길 시간(초). 릴레이는 여기에 2분을 더해 기다린다
pub(super) const UPDATING_ETA_SECS: u64 = 90;
/// heartbeat 판 번호를 확인했는데 응답에 담기지 않았으면 다시 보기까지 (릴레이 `/latest`의 캐시 수명)
pub(super) const RELEASE_RECHECK: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// 앱 업데이트를 묻지 않고 적용한다: 받아 두고, 막는 것이 없으면 적용하고(데몬이 내려가야 하면 업데이터를 띄운다),
/// 막는 것이 있으면 `waiting`으로 두고 30초마다 다시 본다. 자동으로 할 수 없으면 `ready`. 한 번에 하나만 돈다.
/// 도는 중에 다시 불리면(그사이 확인이 더 새 판을 찾았다) 도는 반복이 그 판을 받는다
pub(super) fn spawn_core_apply(state: UpdateState) {
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

pub(super) async fn core_apply(state: &UpdateState) {
    use crate::ipc::internal::into_internal;
    let stop = state.stop_requested.clone().unwrap_or_default();
    loop {
        if state.updater_launched() || stop.is_cancelled() {
            return;
        }
        let cfg = state.manager.read().await.get_config();
        // 사용자가 고른 설치(`/api/updates/install`)가 끊길 작업을 기다리는 중이면 자동 적용 설정과 상관없이 이어서 설치한다
        let requested = state.install_requested.load(std::sync::atomic::Ordering::SeqCst);
        // 처음 차례와, 기다리는 사이 확인이 더 새 판을 찾았을 때 받는다. '확인만'(받기·적용 모두 끔)이면 받지 않는다
        if state.take_apply_recheck() && (cfg.auto_apply || cfg.auto_download || requested) {
            download_app_updates(state).await;
        }
        // 자동 적용을 껐다: 사용자가 다시 시작을 누를 때 적용한다 (받아 두지 않았으면 그때 받는다)
        if !cfg.auto_apply && !requested {
            hold_for_restart(state).await;
            return;
        }
        let (decision, version, notes) = plan_core_apply(state, requested).await;
        if !matches!(decision, Decision::Wait(_)) {
            state.install_requested.store(false, std::sync::atomic::Ordering::SeqCst);
        }
        match decision {
            Decision::Nothing => {
                // 기다리던 것이 사라졌으면(받아 둔 것을 적용했거나 지웠다) 걷는다. 되돌림·실패한 판의 준비됨은 사용자가 고를 때까지 둔다
                let phase = state.phase();
                if phase.state == PhaseState::Waiting {
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
                return;
            }
            Decision::Wait(blockers) => {
                tracing::info!("[Updates] Waiting for {:?} before applying {:?}", blockers.iter().map(|b| (b.kind, &b.name)).collect::<Vec<_>>(), version);
                state.set_phase(Phase { waiting_for: blockers, ..Phase::new(PhaseState::Waiting, version, notes) });
            }
            Decision::Apply(targets) => {
                tracing::info!("[Updates] Applying {:?} without asking", targets);
                state.set_phase(Phase::new(PhaseState::Applying, version.clone(), notes.clone()));
                // 사용자가 고른 설치는 받아 둔 것 전부다 (기다리지 않는 `install_checked`와 같다): 모듈·익스텐션이 빠지지 않게
                let components = if requested { Vec::new() } else { targets };
                let answer = into_internal(apply_updates_with(state.clone(), ApplyRequest { components, ..Default::default() }).await).await.body;
                if answer.get("requires_updater").and_then(Value::as_bool) == Some(true) {
                    let launched = into_internal(launch_updater(State(state.clone()), LooseJson(None)).await).await.body;
                    if launched.get("success").and_then(Value::as_bool) != Some(true) {
                        tracing::warn!("[Updates] Could not start the updater: {}", launched);
                        state.set_phase(Phase::default());
                    }
                } else if answer.get("success").and_then(Value::as_bool) == Some(true) {
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

/// 확인해 둔 코어 새 판 (없으면 None)
pub(super) fn core_update(status: &saba_chan_updater_lib::UpdateStatus) -> Option<&str> {
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

/// 받아 둔 앱 업데이트를 지금 적용할 수 있는가 (`update_phase::decide`)와 그 판·패치노트.
/// `requested`: 사용자가 고른 설치라 되돌린·고정한·미룬 판도 적용한다 (끊길 작업만 기다린다)
async fn plan_core_apply(state: &UpdateState, requested: bool) -> (Decision, Option<String>, Option<std::collections::HashMap<String, String>>) {
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
        failed_release: failed.as_deref().filter(|_| !requested),
        user_held: user_held && !requested,
        deferred_release: deferred.as_deref().filter(|_| !requested),
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
pub(super) async fn get_daemon_update(State(state): State<UpdateState>) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let cfg = mgr.get_config();
    let status = mgr.get_status();
    let visible = status.components.iter().filter(|c| c.update_available && !matches!(c.component, Component::Locales));
    let latest = status.components.iter().find(|c| matches!(c.component, Component::CoreDaemon) && c.update_available).and_then(|c| c.latest_version.clone());
    Json(json!({
        "success": true,
        "version": crate::relay_agent::AGENT_VERSION,
        "profile": saba_chan_updater_lib::profile::Profile::load().as_str(),
        "auto": cfg.enabled && cfg.auto_apply,
        "updatesAvailable": visible.count(),
        "latestVersion": latest,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DaemonUpdateBody {
    auto: bool,
}

/// PUT /api/daemon/update {auto} — 자동 업데이트(확인·받기·적용)를 켜고 끈다. 켜면 바로 한 번 확인한다
pub(super) async fn put_daemon_update(State(state): State<UpdateState>, Json(body): Json<DaemonUpdateBody>) -> axum::response::Response {
    {
        let mut mgr = state.manager.write().await;
        let mut cfg = mgr.get_config();
        if body.auto {
            cfg.enabled = true;
            cfg.auto_download = true;
        }
        cfg.auto_apply = body.auto;
        if let Err(resp) = adopt_config(&mut mgr, &ConfigFile::default_location(), cfg) {
            return resp;
        }
    }
    if body.auto {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = check_and_follow_up(&state).await {
                tracing::warn!("[Updates] Check after enabling auto-update failed: {}", e);
            }
        });
    }
    Json(json!({ "success": true, "auto": body.auto })).into_response()
}

/// POST /api/updates/download — 선택한 컴포넌트 다운로드
///
/// Body: `{ "components": ["gui", "cli", "module-minecraft"] }`
/// 비어있으면 업데이트 가능한 전체를 다운로드
/// 본문이 없거나 비면 기본값 (CLI는 본문 없이 POST한다). 있는데 깨졌으면 400:
/// 잘못된 본문을 "전부"로 읽어 모든 컴포넌트를 건드리지 않게
pub(super) fn body_or_default<T: serde::de::DeserializeOwned + Default>(bytes: &[u8]) -> Result<T, String> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON body: {}", e))
}

pub(super) fn bad_body(error: String) -> axum::response::Response {
    crate::ipc::handlers::error(axum::http::StatusCode::BAD_REQUEST, "invalid_body", error)
}

#[derive(Deserialize, Default)]
pub(super) struct DownloadRequest {
    /// 다운로드할 컴포넌트 키 목록. 비어있으면 전체.
    #[serde(default)]
    pub(super) components: Vec<String>,
}

pub(super) async fn download_components(State(state): State<UpdateState>, raw: axum::body::Bytes) -> axum::response::Response {
    match body_or_default::<DownloadRequest>(&raw) {
        Ok(body) => download_components_with(state, body).await.into_response(),
        Err(e) => bad_body(e),
    }
}

pub(super) async fn download_components_with(state: UpdateState, body: DownloadRequest) -> axum::response::Response {
    let _touched = StatusTouched(&state);
    // 아직 체크하지 않았으면 먼저 체크. 묻고 받는 동안은 관리자를 잠그지 않는다 (`check_shared`·`download_shared`):
    // 상태 조회·`update.status`가 기다리지 않는다
    let unchecked = state.manager.read().await.get_status().components.is_empty();
    if unchecked {
        if let Err(e) = saba_chan_updater_lib::check_shared(&state.manager, None).await {
            return crate::ipc::handlers::error(axum::http::StatusCode::BAD_GATEWAY, "update_check_failed", format!("Auto-check failed: {}", e));
        }
    }

    if body.components.is_empty() {
        // 전체 다운로드. 하나가 실패해도 받은 것은 적용할 수 있게 남기고, 선택 다운로드처럼 받은 것과 실패한 것을 함께 알린다
        match saba_chan_updater_lib::download_available_shared(&state.manager).await {
            Ok((downloaded, failed)) => {
                // 업데이터 --apply 모드를 위해 매니페스트 저장
                if let Err(e) = state.manager.read().await.save_pending_manifest() {
                    tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
                }
                let errors: Vec<Value> = failed.iter().map(|(k, e)| json!({ "component": k, "error": e })).collect();
                Json(json!({
                    "success": errors.is_empty(),
                    "downloaded": downloaded,
                    "count": downloaded.len(),
                    "errors": errors,
                }))
                .into_response()
            }
            Err(e) => crate::ipc::handlers::error(axum::http::StatusCode::BAD_GATEWAY, "download_failed", e.to_string()),
        }
    } else {
        // 선택 컴포넌트만 다운로드
        let mut downloaded = Vec::new();
        let mut errors = Vec::new();

        for key in &body.components {
            let component = Component::from_manifest_key(key);
            match saba_chan_updater_lib::download_shared(&state.manager, &component).await {
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
            if let Err(e) = state.manager.read().await.save_pending_manifest() {
                tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
            }
        }

        Json(json!({
            "success": errors.is_empty(),
            "downloaded": downloaded,
            "count": downloaded.len(),
            "errors": errors,
        }))
        .into_response()
    }
}

/// POST /api/updates/apply — 다운로드된 업데이트 적용
///
/// - 모듈·익스텐션·번역·CLI: 데몬이 직접 적용 (파일 교체. 실행 중인 CLI는 비켜 두고 바꾼다)
/// - 데몬/GUI/업데이터(`Component::needs_daemon_down`): 적용하지 않고 `requires_updater: true`로 돌려준다. 실제 적용은 `POST /api/updates/launch-updater`
///
/// Body: `{ "components": ["module-minecraft", "saba-core"] }` (선택 적용, 비어있으면 전체)
#[derive(Deserialize, Default)]
pub(super) struct ApplyRequest {
    #[serde(default)]
    pub(super) components: Vec<String>,
    /// 묻지 않는 교체(`replace_addons_once`): 교체 순간의 폴더가 카탈로그 그대로인 모듈·익스텐션만 바꾼다. API로는 받지 않는다
    #[serde(skip)]
    pub(super) untouched_only: bool,
}

pub(super) async fn apply_updates(State(state): State<UpdateState>, raw: axum::body::Bytes) -> axum::response::Response {
    match body_or_default::<ApplyRequest>(&raw) {
        Ok(body) => apply_updates_with(state, body).await.into_response(),
        Err(e) => bad_body(e),
    }
}

pub(super) async fn apply_updates_with(state: UpdateState, body: ApplyRequest) -> impl IntoResponse {
    let _touched = StatusTouched(&state);
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
    let any_needs_restart = targets.iter().any(Component::needs_daemon_down);

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
                Ok(()) => {
                    // 고른 판으로 바꾸기를 기다리던 것이면 기다림을 지운다 (바뀐 판이 무엇이든 그 기다림은 끝났다)
                    if is_addon(comp) {
                        if let Err(e) = addon_pins::update(&comp.manifest_key(), |c| c.pending_version = None) {
                            tracing::warn!("[Updates] Could not clear the chosen version of {}: {}", comp.manifest_key(), e);
                        }
                    }
                    applied.push(comp.display_name());
                    applied_keys.push(comp.manifest_key());
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
                        sup.migrate_settings();
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
        "success": errors.is_empty(),
        "applied": applied,
        "needs_updater": needs_updater,
        "requires_updater": !needs_updater.is_empty(),
        "errors": errors,
    }))
}

/// 데몬을 다시 켠 뒤 아직 확인 전이면(상태가 비어 있음) 지난번에 받아 둔 목록(`pending.json`)을 되살린다.
/// 이미 확인했으면 그 상태를 그대로 쓴다 (확인이 받아 둔 파일을 알아본다)
pub(super) fn restore_pending(mgr: &mut saba_chan_updater_lib::UpdateManager) {
    if mgr.get_status().components.is_empty() {
        let _ = mgr.load_pending_manifest();
    }
}

/// POST /api/updates/install — 확인 → 받을 것 전부 받기 → 적용. 데몬·GUI처럼 재시작이 필요한 것이 있으면 업데이터를 띄운다
/// (그 뒤 데몬은 꺼지고 업데이터가 GUI나 데몬을 다시 띄운다). GUI가 없는 원격(디스코드)에서도 끝까지 가게 하는 경로다.
/// 재시작이 필요한데 끊길 작업(`core_blockers`)이 있으면 띄우지 않고 `waiting`으로 답한다: 적용 반복이 그 작업이 끝나면
/// 자동 적용 설정과 상관없이 이어서 설치한다.
/// 답: apply의 답(`ok, applied, requires_updater, errors`)에 `downloaded`, 받지 못한 것 `download_errors`, 업데이터를 띄웠으면
/// `updater_launched: true`, 기다리면 `waiting: [{kind, instance_id, name}]`(상태의 `phase.waiting_for`와 같은 모양)
pub(super) async fn install_updates(State(state): State<UpdateState>) -> axum::response::Response {
    if let Err(e) = check_and_follow_up(&state).await {
        return crate::ipc::handlers::error(axum::http::StatusCode::BAD_GATEWAY, "update_check_failed", format!("Update check failed: {}", e));
    }
    install_checked(state).await
}

/// 확인을 마친 뒤의 설치 (`install_updates`)
pub(super) async fn install_checked(state: UpdateState) -> axum::response::Response {
    use crate::ipc::internal::into_internal;
    let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest::default()).await).await;
    if !downloaded.status.is_success() {
        return (downloaded.status, Json(downloaded.body)).into_response();
    }
    let downloaded = downloaded.body;
    let (decision, version, notes) = plan_core_apply(&state, true).await;
    let mut applied = match decision {
        Decision::Wait(blockers) => {
            state.install_requested.store(true, std::sync::atomic::Ordering::SeqCst);
            state.set_phase(Phase { waiting_for: blockers.clone(), ..Phase::new(PhaseState::Waiting, version, notes) });
            spawn_core_apply(state.clone());
            // 업데이터는 적용 반복이 띄운다 (`requires_updater`를 싣지 않는다)
            json!({ "success": true, "applied": [], "errors": [], "waiting": blockers })
        }
        _ => into_internal(apply_updates_with(state.clone(), ApplyRequest::default()).await).await.body,
    };
    applied["downloaded"] = downloaded.get("downloaded").cloned().unwrap_or(Value::Null);
    // 일부만 받았으면 받은 것은 적용하고 받지 못한 것을 함께 알린다
    if let Some(failed) = downloaded.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
        applied["success"] = json!(false);
        applied["download_errors"] = json!(failed);
    }
    if applied.get("requires_updater").and_then(Value::as_bool) == Some(true) {
        let launched = into_internal(launch_updater(State(state), LooseJson(None)).await).await.body;
        if launched.get("success").and_then(Value::as_bool) == Some(false) {
            applied["success"] = json!(false);
            applied["error"] = launched.get("error").cloned().unwrap_or(Value::Null);
        } else {
            applied["updater_launched"] = json!(true);
        }
    }
    Json(applied).into_response()
}
