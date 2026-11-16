//! 업데이터에 넘기기: 업데이터를 띄우고 데몬이 내려가는 길(`launch-updater`·`restart`·`rollback`), 실행 인자와
//! GUI 다시 띄우기 계획, 판 목록(`versions`), 마지막 적용 결과(`last-result`)

use super::*;
use super::apply::{
    ApplyRequest, DownloadRequest, UPDATING_ETA_SECS, apply_updates_with, core_update, download_components_with,
    restore_pending,
};

/// POST /api/updates/launch-updater — 데몬이 업데이터를 띄우고 스스로 종료한다
///
/// `apply`가 `requires_updater: true`를 돌려준 뒤 부른다. 어느 클라이언트가 떠 있든
/// 적용 경로가 하나가 되도록 업데이터 실행은 데몬만 한다.
/// Body: `{ "relaunch": ["<exe>", "추가 인자"...], "gui_stays": bool }` — 적용 후 다시 띄울 프로그램(생략하면 업데이터가 GUI를 찾는다).
/// `gui_stays`: 부른 GUI가 `gui_restart:false`를 보고 남을 줄 안다 (모르는 옛 GUI는 성공 응답에 늘 끝난다)
#[derive(Deserialize, Default)]
pub(super) struct LaunchUpdaterRequest {
    #[serde(default)]
    relaunch: Vec<String>,
    #[serde(default)]
    gui_stays: bool,
}

pub(super) async fn launch_updater(
    State(state): State<UpdateState>,
    LooseJson(body): LooseJson<LaunchUpdaterRequest>,
) -> axum::response::Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let targets = state.manager.read().await.updater_targets();
    if targets.is_empty() {
        return crate::ipc::handlers::error(axum::http::StatusCode::CONFLICT, "nothing_to_apply", "No downloaded updates to apply");
    }
    hand_off_to_updater(state, body, targets, false).await
}

/// GET /api/updates/rollback — 되돌릴 수 있는 직전 판 (`live_apply::PreviousVersion`)
pub(super) async fn get_rollback(State(state): State<UpdateState>) -> impl IntoResponse {
    let previous = saba_chan_updater_lib::live_apply::PreviousVersion::load(&saba_chan_updater_lib::constants::resolve_data_dir());
    let mut body = rollback_body(previous.as_ref(), &saba_chan_updater_lib::relay::core_version());
    if let Some(version) = body["version"].as_str().map(str::to_string) {
        body["needs_newer_core"] = json!(addons_needing_core(&version, installed_addon_requires(&state).await));
    }
    Json(body)
}

/// `{ok, available, current}`, 되돌릴 수 있으면 `version`(돌아갈 판)·`updated_at`(지금 판으로 업데이트한 때)도
pub(super) fn rollback_body(previous: Option<&saba_chan_updater_lib::live_apply::PreviousVersion>, current: &str) -> Value {
    match kept_previous(previous, current) {
        Some(p) => json!({ "success": true, "available": true, "version": p.version, "current": current, "updated_at": p.updated_at }),
        None => json!({ "success": true, "available": false, "current": current }),
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
pub(super) async fn get_versions(State(state): State<UpdateState>) -> impl IntoResponse {
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
pub(super) fn versions_body(
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
        "success": true,
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
    let core = crate::module_loader::CORE_COMPONENT.to_string();
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
pub(super) fn addons_needing_core(version: &str, addons: Vec<(String, std::collections::HashMap<String, String>)>) -> Vec<String> {
    let core = crate::module_loader::CORE_COMPONENT.to_string();
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
pub(super) async fn roll_back(State(state): State<UpdateState>, LooseJson(body): LooseJson<LaunchUpdaterRequest>) -> axum::response::Response {
    let previous = saba_chan_updater_lib::live_apply::PreviousVersion::load(&saba_chan_updater_lib::constants::resolve_data_dir());
    roll_back_with(state, body.map(|Json(b)| b).unwrap_or_default(), previous).await
}

pub(super) async fn roll_back_with(
    state: UpdateState,
    body: LaunchUpdaterRequest,
    previous: Option<saba_chan_updater_lib::live_apply::PreviousVersion>,
) -> axum::response::Response {
    let installed = saba_chan_updater_lib::relay::core_version();
    let Some(mut previous) = previous.filter(|p| kept_previous(Some(p), &installed).is_some()) else {
        return crate::ipc::handlers::error(axum::http::StatusCode::CONFLICT, "no_previous_version", "No previous version is kept");
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
        .map(|(_, exe)| if is_gui_exe(exe) { "gui".to_string() } else { exe.to_string_lossy().into_owned() })
        .collect();
    hand_off_to_updater(state, body, targets, true).await
}

/// 업데이터를 띄우고 데몬을 끈다. `targets`는 바꿀 구성요소 키(GUI를 다시 띄울지 정한다), `rollback`이면 적용 대신 직전 판으로 되돌린다
async fn hand_off_to_updater(state: UpdateState, body: LaunchUpdaterRequest, targets: Vec<String>, rollback: bool) -> axum::response::Response {
    let Some(updater) = updater_binary() else {
        return crate::ipc::handlers::error(axum::http::StatusCode::NOT_FOUND, "updater_not_started", "saba-chan-installer not found next to the daemon");
    };
    use std::sync::atomic::Ordering;
    if state.updater_launched.swap(true, Ordering::SeqCst) {
        return crate::ipc::handlers::error(axum::http::StatusCode::CONFLICT, "updater_already_launched", "The updater is already launching");
    }

    // 이번 적용에만 쓰는 실행 표 (업데이터는 표 없이 --apply를 거절한다)
    let ticket = match saba_chan_updater_lib::launch_ticket::issue(&saba_chan_updater_lib::constants::resolve_staging_dir()) {
        Ok(t) => t,
        Err(e) => {
            state.updater_launched.store(false, Ordering::SeqCst);
            return crate::ipc::handlers::error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "updater_not_started", format!("Failed to prepare updater launch: {}", e));
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
    let gui_restart = !guis.is_empty() || relaunch.first().is_some_and(is_gui_exe);
    let mut args = updater_args(ticket, saba_chan_updater_lib::profile::Profile::load(), relaunch, saba_chan_updater_lib::autostart::service_relaunch());
    if rollback {
        args.insert(1, "--rollback".to_string());
    }
    // 바로 끝난 업데이터(라이브러리 없음·백신 차단)는 데몬을 내리기 전에 실패로 돌린다. --wait-pid로 데몬을 기다리므로 살아 있어야 한다
    if let Err(e) = spawn_and_confirm(&updater, &args, std::time::Duration::from_millis(700)).await {
        state.updater_launched.store(false, Ordering::SeqCst);
        return crate::ipc::handlers::error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "updater_not_started", format!("Failed to start updater: {}", e));
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
    Json(json!({ "success": true, "updater_launched": true, "daemon_stopping": true, "gui_restart": gui_restart })).into_response()
}

/// 업데이터 실행 인자. 헤드리스 설치는 창 없이 적용하게 `--headless`를 붙이고,
/// 다시 띄울 것을 받지 않았으면 데몬 자신을 다시 띄우게 한다 (GUI가 없어 데몬을 띄워 줄 쪽이 없다).
/// 데몬이 자동 실행 서비스로 돌면(`service`, 화면 없는 리눅스) 받은 것과 상관없이 그 서비스를 다시 시작하게 한다
pub(super) fn updater_args(ticket: String, profile: saba_chan_updater_lib::profile::Profile, mut relaunch: Vec<String>, service: Option<Vec<String>>) -> Vec<String> {
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
pub(super) fn running_guis(clients: &[(String, crate::ipc::RegisteredClient)]) -> Vec<(u32, std::path::PathBuf)> {
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
            is_gui_exe(&exe).then_some((pid, exe))
        })
        .collect()
}

/// 등록된 GUI가 모두 창 없이 트레이만 들고 있다 (다시 띄울 때도 창을 열지 않는다)
pub(super) fn only_resident_guis(clients: &[(String, crate::ipc::RegisteredClient)]) -> bool {
    let mut guis = clients.iter().filter(|(_, c)| c.kind == crate::ipc::ClientKind::Gui).peekable();
    guis.peek().is_some() && guis.all(|(_, c)| c.resident)
}

/// 업데이터를 띄우기 전에 끌 GUI와 적용 뒤 다시 띄울 것. GUI가 바뀌지 않으면 GUI는 끄지 않고 데몬(`daemon`)만 다시 띄운다
/// (요청이 GUI가 아닌 것을 골랐으면 그것). 트레이만 든 GUI는 데몬이 사라지면 따라 끝나므로 바뀌지 않아도 끄고 트레이로 다시 띄운다
pub(super) fn restart_plan(
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
pub(super) fn gui_must_restart(targets: &[String], requested: &[String], gui_stays: bool) -> bool {
    targets.iter().any(|k| k == "gui") || (!gui_stays && requested.first().is_some_and(is_gui_exe))
}

/// 업데이터가 다시 띄울 것: 떠 있던 GUI가 있으면 그 GUI(요청이 이미 GUI면 그대로), 없으면 요청대로.
/// 떠 있던 GUI가 트레이만 들고 있었으면(`resident`) 트레이 모드로 다시 띄운다
pub(super) fn relaunch_target(requested: Vec<String>, guis: &[(u32, std::path::PathBuf)], resident: bool) -> Vec<String> {
    match guis.first() {
        Some((_, exe)) if !requested.first().is_some_and(is_gui_exe) => {
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
pub(super) async fn close_guis(guis: &[(u32, std::path::PathBuf)]) {
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
pub(super) fn installer_applies(installed: &std::collections::HashMap<String, String>, since: &str) -> bool {
    use saba_chan_updater_lib::version::SemVer;
    match (installed.get("installer").and_then(|v| SemVer::parse(v)), SemVer::parse(since)) {
        (Some(installer), Some(since)) => installer >= since,
        _ => false,
    }
}

/// 적용을 아는 설치기(`applies`)를 먼저 찾고, 없으면 0.1.12 전 설치에 남은 옛 업데이터를 쓴다 (같은 인자를 받는다).
/// 설치기가 적용을 모르면 옛 업데이터를 먼저 쓰고, 그것도 없을 때만 설치기를 쓴다
pub(super) fn updater_binary_near(daemon_exe: &std::path::Path, applies: bool) -> Option<std::path::PathBuf> {
    use saba_chan_updater_lib::{INSTALLER_EXE, LEGACY_UPDATER_EXE};
    let order = if applies { [INSTALLER_EXE, LEGACY_UPDATER_EXE] } else { [LEGACY_UPDATER_EXE, INSTALLER_EXE] };
    order.iter().find_map(|name| daemon_exe.ancestors().skip(1).take(2).map(|dir| dir.join(name)).find(|p| p.is_file()))
}

/// 데몬이 끝나도 살아남게 띄운다
/// 띄우고 `grace` 동안 지켜본다. 그 사이 끝나면 종료 코드를 담은 오류
pub(super) async fn spawn_and_confirm(program: &std::path::Path, args: &[String], grace: std::time::Duration) -> std::io::Result<()> {
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
pub(super) async fn get_last_result(Query(q): Query<LastResultQuery>) -> impl IntoResponse {
    let path = saba_chan_updater_lib::UpdateCompletionMarker::path();
    Json(last_result_at(&path, q.include_seen, q.lang.as_deref().unwrap_or("en")))
}

#[derive(Deserialize, Default)]
pub(super) struct LastResultQuery {
    lang: Option<String>,
    #[serde(default)]
    include_seen: bool,
}

pub(super) fn last_result_at(path: &std::path::Path, include_seen: bool, lang: &str) -> Value {
    let marker = saba_chan_updater_lib::UpdateCompletionMarker::load_from(path).filter(|m| include_seen || !m.acknowledged);
    let acknowledged = marker.as_ref().is_some_and(|m| m.acknowledged);
    let mut body = last_result_body(marker, lang);
    body["acknowledged"] = json!(acknowledged);
    body
}

/// 데몬이 바로 적용한 결과를 완료 마커로 남긴다 (업데이터가 적용할 때와 같은 모양). 클라이언트가 결과·패치노트를 본다.
/// 업데이터가 새 코어를 기다리게 둔 것(`deferred`)을 마저 적용했으면 사용자가 고른 그 묶음의 결과에 더한다.
/// 복구(`repair`)는 업데이트로 알리지 않는다 (`UpdateCompletionMarker::repaired`)
pub(super) fn record_daemon_apply(path: &std::path::Path, targets: &[String], applied: &[String], errors: &[String], release: Option<saba_chan_updater_lib::marker::ReleaseSummary>, repair: bool) {
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
    if let Some(store) = crate::notices::store() {
        store.note_update_result(&marker);
    }
}

pub(super) fn last_result_body(marker: Option<saba_chan_updater_lib::UpdateCompletionMarker>, lang: &str) -> Value {
    let notes = marker.as_ref().and_then(|m| m.release.as_ref()).and_then(|r| r.notes_for(lang)).map(str::to_string);
    let version = marker.as_ref().and_then(|m| m.release.as_ref()).map(|r| r.version.clone());
    json!({ "success": true, "result": marker, "version": version, "notes": notes })
}

/// DELETE /api/updates/last-result — 사용자가 결과를 확인했다. 결과는 지우지 않고 확인했다고만 적는다
/// (다른 클라이언트도 `?include_seen=true`로 패치노트를 다시 본다)
pub(super) async fn clear_last_result() -> impl IntoResponse {
    match saba_chan_updater_lib::UpdateCompletionMarker::acknowledge_at(&saba_chan_updater_lib::UpdateCompletionMarker::path()) {
        Ok(()) => Json(json!({ "success": true })).into_response(),
        Err(e) => crate::ipc::handlers::error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "save_failed", e),
    }
}

/// POST /api/updates/restart — `ready`에서 "지금 다시 시작". 실패한 판 기록을 지우고, 받아 둔 것(없으면 받을 것)을 적용한다.
/// 데몬이 내려가야 하면 `launch-updater`와 같은 길을 타고(GUI 재시작 포함, 본문도 같다) 그 답을 그대로 준다.
/// 데몬이 그대로면 `{ok, updater_launched:false, daemon_stopping:false, gui_restart:false}`
pub(super) async fn restart_now(State(state): State<UpdateState>, body: LooseJson<LaunchUpdaterRequest>) -> axum::response::Response {
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
        let downloaded = into_internal(download_components_with(state.clone(), DownloadRequest::default()).await).await;
        if !downloaded.status.is_success() {
            return (downloaded.status, Json(downloaded.body)).into_response();
        }
    }
    // 받을 것도 없다 (고정만 풀었거나 준비됨이던 판이 내려갔다): 완료로 알리지 않는다
    if nothing_staged && state.manager.read().await.get_pending_components().is_empty() {
        if state.phase().state == PhaseState::Ready {
            state.set_phase(Phase::default());
        }
        return Json(json!({ "success": true, "updater_launched": false, "daemon_stopping": false, "gui_restart": false, "errors": [] })).into_response();
    }
    let applied = into_internal(apply_updates_with(state.clone(), ApplyRequest::default()).await).await.body;
    if applied.get("requires_updater").and_then(Value::as_bool) == Some(true) {
        return launch_updater(State(state), body).await;
    }
    if applied.get("success").and_then(Value::as_bool) == Some(true) {
        let release = state.manager.read().await.applying_release();
        state.set_phase(Phase::new(PhaseState::Done, release.as_ref().map(|r| r.version.clone()), release.map(|r| r.notes).filter(|n| !n.is_empty())));
    }
    Json(json!({
        "success": applied.get("success").and_then(Value::as_bool).unwrap_or(false),
        "updater_launched": false,
        "daemon_stopping": false,
        "gui_restart": false,
        "errors": applied.get("errors").cloned().unwrap_or_else(|| json!([])),
    }))
    .into_response()
}
