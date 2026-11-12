//! 게임 서버 업데이트를 켜진 서버에도: 게임 안에 예고 → 저장하고 정지 → 업데이트(업데이트 전 백업 포함) → 다시 켜기.
//! 그리고 인스턴스별 자동 업데이트 (`auto_update`): 새 빌드가 나오면 꺼져 있거나 접속자가 없을 때 바로,
//! 접속자가 있으면 `instance.update_waiting`으로 알리고 모두 나가기를 기다린다 (알림의 버튼은 `apply-update`로 5분 예고 후 바로)
#![allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use super::super::IPCServer;

/// 켜진 서버를 업데이트할 때 게임 안에 예고하는 기본 시간(분)
pub const WARN_MINUTES: u32 = 5;
/// 자동 업데이트가 새 빌드를 확인하는 간격
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(3 * 60 * 60);
/// 데몬이 켜진 뒤 첫 확인까지 (켜자마자 SteamCMD를 부르지 않게)
const FIRST_CHECK_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// 기다리는 서버의 접속자 수를 다시 보는 간격
const TICK: std::time::Duration = std::time::Duration::from_secs(60);
/// 정지를 기다리는 가장 긴 시간 (500ms × 240)
const STOP_POLLS: usize = 240;

/// 인스턴스별 마지막 게임 서버 업데이트 확인 결과 (`server_update_status`의 답 + `checked_at`)
fn checks() -> &'static Mutex<HashMap<String, Value>> {
    static CHECKS: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    CHECKS.get_or_init(Default::default)
}

/// 확인 결과를 기억한다. 화면에 보이는 값(새 빌드 여부·빌드·실패 이유)이 바뀌면 `instance.update_checked`를 낸다
/// (지금 붙은 클라이언트에게만: 목록을 다시 읽으라는 신호다)
pub fn remember_check(id: &str, status: &Value) {
    let shown = |v: &Value| (v.get("update_available").cloned(), v.get("reason").cloned(), v.get("remote_buildid").cloned(), v.get("error").cloned());
    let mut kept = status.clone();
    if let Some(map) = kept.as_object_mut() {
        map.insert("checked_at".into(), json!(unix_secs()));
    }
    let before = checks().lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), kept);
    if before.as_ref().map(shown) != Some(shown(status)) {
        crate::events::emit_live("instance.update_checked", json!({ "instanceId": id }));
    }
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 마지막 확인 결과 (목록의 `update_check`). 확인하지 않았으면 없다
pub fn last_check(id: &str) -> Option<Value> {
    checks().lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// 업데이트를 마쳤거나 서버가 지워졌다: 지난 결과를 버린다
pub fn forget_check(id: &str) {
    if checks().lock().unwrap_or_else(|e| e.into_inner()).remove(id).is_some() {
        crate::events::emit_live("instance.update_checked", json!({ "instanceId": id }));
    }
}

/// 게임 서버 새 빌드를 데몬이 주기적으로 확인한다 (자동 업데이트를 켜지 않은 서버. 켠 서버는 `spawn_auto_updater`가 본다).
/// 화면은 확인하지 않고 목록의 결과만 읽는다. 확인하지 않는 서버(직접 받은 서버 등)는 hook 없이 바로 끝난다
pub fn spawn_update_checks(state: IPCServer) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut tried: HashMap<String, std::time::Instant> = HashMap::new();
        loop {
            tokio::select! {
                _ = state.shutdown_token.cancelled() => break,
                _ = tokio::time::sleep(TICK) => {}
            }
            if started.elapsed() < FIRST_CHECK_AFTER {
                continue;
            }
            let due = {
                let supervisor = state.supervisor.read().await;
                let list = supervisor.instance_store.list();
                tried.retain(|id, _| list.iter().any(|i| &i.id == id));
                due_checks(list, &tried, std::time::Instant::now())
            };
            for (id, _) in due {
                // 설치·업데이트 중이면 다음 틱에 본다
                if is_pending(&id) || super::backup::InstanceLock::held(&id) || state.provision_tracker.get(&id).is_some_and(|p| !p.done) {
                    continue;
                }
                tried.insert(id.clone(), std::time::Instant::now());
                super::instance::server_update_status(&state, &id).await;
            }
        }
    });
}

/// 이번 틱에 확인할 서버 `(id, 이름)`: 자동 업데이트를 켜지 않았고 `CHECK_EVERY` 안에 확인하지 않은 것
fn due_checks(
    list: &[crate::instance::ServerInstance],
    tried: &HashMap<String, std::time::Instant>,
    now: std::time::Instant,
) -> Vec<(String, String)> {
    list.iter()
        .filter(|i| !i.auto_update)
        .filter(|i| tried.get(&i.id).is_none_or(|at| now.duration_since(*at) >= CHECK_EVERY))
        .map(|i| (i.id.clone(), i.name.clone()))
        .collect()
}

/// 예고·정지·업데이트가 진행 중인 인스턴스. 같은 서버에 두 번 걸지 않는다
fn pending() -> &'static Mutex<HashSet<String>> {
    static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// 진행 중 표시. 작업이 끝나면(성공·실패·취소) 지운다
pub(crate) struct Pending(String);

impl Pending {
    pub(crate) fn take(id: &str) -> Option<Self> {
        pending()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string())
            .then(|| Self(id.to_string()))
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        pending().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

pub fn is_pending(id: &str) -> bool {
    pending().lock().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// 켜진 서버면 무엇을 할까: 접속자가 없으면 바로(예고 없이) 다시 켜며 업데이트, 있거나 셀 수 없으면 기다린다
#[derive(Debug, PartialEq, Eq)]
pub enum AutoStep {
    /// 꺼져 있다: 받기만 한다
    Update,
    /// 켜져 있고 아무도 없다: 정지 → 업데이트 → 다시 켜기
    Restart,
    /// 접속자가 있다(또는 셀 수 없다): 알리고 기다린다
    Wait,
}

pub fn auto_step(running: bool, players: Option<u32>) -> AutoStep {
    match (running, players) {
        (false, _) => AutoStep::Update,
        (true, Some(0)) => AutoStep::Restart,
        (true, _) => AutoStep::Wait,
    }
}

/// 켜진 서버의 업데이트를 뒤에서 시작한다: `warn_minutes`분 전부터 게임 안에 예고(5분·1분 전 중 그 안의 것) →
/// 저장하고 정지 → 업데이트 → 다시 켜기. 업데이트가 실패해도 예전 버전으로 다시 켠다.
/// 이미 진행 중이면 409 `update_pending`
pub async fn begin_restart_update(state: &IPCServer, id: &str, warn_minutes: u32) -> Result<(), Response> {
    let Some(mark) = Pending::take(id) else {
        return Err(error(
            StatusCode::CONFLICT,
            "update_pending",
            "An update of this server is already under way",
        ));
    };
    let token = tokio_util::sync::CancellationToken::new();
    warnings().lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), token.clone());
    let (state, id) = (state.clone(), id.to_string());
    crate::audit::spawn(async move {
        let _mark = mark;
        let go = tokio::select! {
            go = warn(&state, &id, warn_minutes) => go,
            _ = token.cancelled() => false,
        };
        // 여기부터는 취소할 수 없다 (끄기 시작). 취소 요청과 같은 잠금 안에서 정한다:
        // 취소가 먼저면 끄지 않고, 이것이 먼저면 취소 요청은 409를 받는다
        let cancelled = {
            let mut warnings = warnings().lock().unwrap_or_else(|e| e.into_inner());
            warnings.remove(&id);
            token.is_cancelled()
        };
        if cancelled {
            let catalog = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
            if super::backup::is_running(&state, &id).await {
                super::backup::announce(&state, &id, &catalog.t("schedule.update_cancelled", &[])).await;
            }
            tracing::info!("Update of '{}' was cancelled during the warning", id);
            return;
        }
        if go {
            stop_update_start(&state, &id).await;
        }
    });
    Ok(())
}

/// 예고 중인 켜진 서버 업데이트 (취소할 수 있는 동안만 있다)
fn warnings() -> &'static Mutex<HashMap<String, tokio_util::sync::CancellationToken>> {
    static WARNINGS: OnceLock<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>> = OnceLock::new();
    WARNINGS.get_or_init(Default::default)
}

/// `DELETE /api/instance/:id/apply-update`: 예고 중인 업데이트를 취소한다. 서버는 그대로 둔다(게임 안에 취소를 알린다).
/// 이미 끄기·업데이트를 시작했으면 409 `update_not_cancellable`, 걸린 업데이트가 없으면 404 `no_pending_update`
pub async fn cancel_server_update(
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::State(_state): axum::extract::State<IPCServer>,
) -> Response {
    // 예고 작업이 끄기로 넘어가는 결정과 같은 잠금 안에서 취소한다
    let cancelled = warnings().lock().unwrap_or_else(|e| e.into_inner()).remove(&id).map(|token| token.cancel());
    match cancelled {
        Some(()) => {
            (StatusCode::OK, Json(json!({ "success": true, "id": id, "cancelled": true }))).into_response()
        }
        None if is_pending(&id) => error(
            StatusCode::CONFLICT,
            "update_not_cancellable",
            "The update is already stopping or updating the server",
        ),
        None => error(StatusCode::NOT_FOUND, "no_pending_update", "No update of this server is waiting"),
    }
}

/// 예고 시각마다 게임 안에 알린다. 그 사이 서버가 꺼졌어도 업데이트는 이어 간다(다시 켜지는 않는다). 인스턴스가 지워졌으면 false
async fn warn(state: &IPCServer, id: &str, minutes: u32) -> bool {
    let catalog = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
    let fire = std::time::Instant::now() + std::time::Duration::from_secs(u64::from(minutes) * 60);
    for mark in [5u32, 1].into_iter().filter(|m| *m <= minutes) {
        let at = fire - std::time::Duration::from_secs(u64::from(mark) * 60);
        tokio::time::sleep_until(at.into()).await;
        if !super::backup::is_running(state, id).await {
            break;
        }
        super::backup::announce(
            state,
            id,
            &catalog.t("schedule.update_warning", &[("minutes", &mark.to_string())]),
        )
        .await;
    }
    tokio::time::sleep_until(fire.into()).await;
    state.supervisor.read().await.instance_store.get(id).is_some()
}

/// 켜져 있으면 저장하고 정지 → 업데이트 → 켜져 있었으면 다시 켠다. 정지하지 못하면 업데이트하지 않는다.
/// 예약 업데이트(`backup::run_update`)와 켜진 서버의 업데이트가 함께 쓴다. 업데이트가 됐으면 true
pub(crate) async fn stop_update_start(state: &IPCServer, id: &str) -> bool {
    stop_update_start_when(state, id, false).await.unwrap_or(false)
}

/// `stop_update_start`. `only_if_empty`(자동 업데이트)면 작업 잠금을 잡아 다른 경로의 시작을 막은 채로 켜져 있는지와
/// 지금 접속자 수를 보고, 켜져 있는데 0명으로 확인되지 않으면 끄지 않고 None (잠금이 잡혀 있어도 None)
/// 같은 서버 폴더를 쓰는 다른 인스턴스가 켜져 있거나 작업 중이어도 None (미룬다)
async fn stop_update_start_when(state: &IPCServer, id: &str, only_if_empty: bool) -> Option<bool> {
    // 시작은 이 잠금을 잡고 켠다(`server::start_instance`): 잡고 있는 동안에는 확인한 뒤 새로 켜지지 않는다
    let start_lock = if only_if_empty {
        let Some(lock) = super::backup::InstanceLock::take_for_writes(id).await else {
            postpone(id, false);
            return None;
        };
        Some(lock)
    } else {
        None
    };
    // 같은 서버 폴더를 쓰는 다른 인스턴스가 켜져 있거나 작업 중이면 업데이트를 시작하지 못한다: 끄지 않고 미룬다
    if only_if_empty && super::instance::update_blocked_by_shared_folder(state, id).await {
        postpone(id, false);
        return None;
    }
    let was_running = super::backup::is_running(state, id).await;
    if was_running && only_if_empty {
        let players = crate::supervisor::players::fresh(&state.supervisor, id).await;
        if players != Some(0) {
            tracing::info!("Auto update of '{}': not stopping, players {:?}", id, players);
            emit_waiting(state, id, players).await;
            postpone(id, true);
            return None;
        }
    }
    if was_running {
        let stop =
            crate::ipc::internal::into_internal(super::server::stop_instance(state.clone(), id, false).await).await;
        // 콘솔이 없어 저장 후 종료를 못 하는 서버(`graceful_stop_unavailable`)도 여기서 멈춘다: 강제 종료하지 않는다
        if !stop.status.is_success() || stop.body.get("success").and_then(Value::as_bool) == Some(false) {
            tracing::warn!("Update of '{}': stop failed: {}", id, stop.body);
            let error = stop
                .body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("stop failed")
                .to_string();
            super::instance::emit_update_failed(state, id, &error).await;
            return Some(false);
        }
        for _ in 0..STOP_POLLS {
            if !super::backup::is_running(state, id).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    tracing::info!("Updating '{}' (was running: {})", id, was_running);
    let generation = state.supervisor.read().await.run_generation_of(id);
    // 잡은 잠금은 업데이트에 넘긴다: 놓았다 다시 잡는 틈에 다른 작업이 끼어 업데이트를 못 하는 일이 없게
    let updated = match super::instance::begin_server_update_holding(state, id, start_lock).await {
        Ok(job) => Some(job.await.unwrap_or(false)),
        Err(r) => {
            let body = crate::ipc::internal::into_internal(r).await.body;
            tracing::warn!("Update of '{}' could not start: {}", id, body);
            // 위에서 본 뒤 끄는 사이에 같은 폴더의 다른 인스턴스가 켜졌거나 작업을 시작했다: 자동 업데이트는 실패로 남기지 않고 미룬다
            let in_use = matches!(
                body.get("error_code").and_then(Value::as_str),
                Some("instance_busy" | "server_running")
            );
            if only_if_empty && in_use {
                postpone(id, false);
                None
            } else {
                super::instance::emit_update_failed(
                    state,
                    id,
                    body.get("error").and_then(Value::as_str).unwrap_or("update failed"),
                )
                .await;
                Some(false)
            }
        }
    };
    if was_running {
        if updated != Some(true) {
            tracing::warn!("Update of '{}' did not finish; starting the old version again", id);
        }
        super::backup::start_after_update(state, id, generation).await;
    }
    updated
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "success": false, "error": message, "error_code": code })),
    )
        .into_response()
}

/// 끄기 직전에 미룬 자동 업데이트 (값: 기다림을 이미 알렸다). 다음 틱에 기다림으로 되돌려 1분마다 다시 본다
fn postponed() -> &'static Mutex<HashMap<String, bool>> {
    static POSTPONED: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    POSTPONED.get_or_init(Default::default)
}

fn postpone(id: &str, notified: bool) {
    postponed().lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), notified);
}

/// 미뤘으면 기다림으로 되돌린다 (확인 간격 3시간을 다시 기다리지 않게)
fn resume_postponed(id: &str, watch: &mut Watch) {
    if let Some(notified) = postponed().lock().unwrap_or_else(|e| e.into_inner()).remove(id) {
        watch.waiting = true;
        watch.notified = notified;
    }
}

/// 자동 업데이트가 인스턴스마다 기억하는 것 (메모리에만: 데몬이 다시 켜지면 처음부터 확인한다)
#[derive(Default)]
struct Watch {
    checked: Option<std::time::Instant>,
    /// 새 빌드가 있어 업데이트를 기다린다 (접속자가 있다)
    waiting: bool,
    /// 기다린다고 알렸다. 기다림 하나에 한 번만 알린다
    notified: bool,
    /// 마지막 확인에서 본 새 빌드 (`remote_buildid`)
    remote: Option<String>,
}

/// 자동 업데이트가 실패한 새 빌드 (인스턴스별). 같은 빌드는 다시 시도하지 않는다:
/// 3시간마다 끄기 → 실패 → 다시 켜기와 실패 알림이 되풀이되지 않게. 더 새 빌드가 나오거나 데몬이 다시 켜지면 다시 시도한다
fn failed_builds() -> &'static Mutex<HashMap<String, String>> {
    static FAILED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    FAILED.get_or_init(Default::default)
}

fn failed_before(id: &str, remote: &str) -> bool {
    failed_builds().lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|b| b == remote)
}

/// 자동 업데이트 결과를 남긴다: 실패면 그 빌드를 기억하고, 성공이면 지운다
fn record_auto_result(id: &str, remote: Option<&str>, updated: bool) {
    let mut failed = failed_builds().lock().unwrap_or_else(|e| e.into_inner());
    match (updated, remote) {
        (false, Some(remote)) => {
            failed.insert(id.to_string(), remote.to_string());
        }
        _ => {
            failed.remove(id);
        }
    }
}

/// 인스턴스별 자동 업데이트 (`auto_update`). 1분마다 돌며, 확인은 서버마다 3시간에 한 번
pub fn spawn_auto_updater(state: IPCServer) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut watches: HashMap<String, Watch> = HashMap::new();
        loop {
            tokio::select! {
                _ = state.shutdown_token.cancelled() => break,
                _ = tokio::time::sleep(TICK) => {}
            }
            if started.elapsed() < FIRST_CHECK_AFTER {
                continue;
            }
            let ids: Vec<String> = state
                .supervisor
                .read()
                .await
                .instance_store
                .list()
                .iter()
                .filter(|i| i.auto_update)
                .map(|i| i.id.clone())
                .collect();
            watches.retain(|id, _| ids.contains(id));
            for id in ids {
                let watch = watches.entry(id.clone()).or_default();
                crate::audit::with_origin(crate::audit::Origin::new("auto_update"), auto_tick(&state, &id, watch))
                    .await;
            }
        }
    });
}

async fn auto_tick(state: &IPCServer, id: &str, watch: &mut Watch) {
    if is_pending(id) || super::backup::InstanceLock::held(id) {
        return;
    }
    resume_postponed(id, watch);
    let fresh = !watch.waiting;
    if fresh {
        if watch.checked.is_some_and(|at| at.elapsed() < CHECK_EVERY) {
            return;
        }
        if update_check(state, id, watch).await != Checked::Available {
            return;
        }
        watch.waiting = true;
        watch.notified = false;
    }
    // 같은 서버 폴더를 쓰는 다른 인스턴스가 켜져 있거나 작업 중이면 기다린다 (풀리면 다음 틱에 이어 간다)
    if super::instance::update_blocked_by_shared_folder(state, id).await {
        return;
    }
    let running = super::backup::is_running(state, id).await;
    let runtime = state.supervisor.read().await.runtime.clone();
    let mut players = crate::supervisor::players::get(&runtime, id).map(|c| c.online);
    let mut step = auto_step(running, players);
    if step != AutoStep::Wait {
        // 기다리는 동안 알림 버튼·앱·예약으로 이미 업데이트했을 수 있다: 끄기 전에 한 번 더 본다
        if !fresh {
            match update_check(state, id, watch).await {
                Checked::Available => {}
                Checked::UpToDate => {
                    watch.waiting = false;
                    return;
                }
                // 확인하지 못했다(잠깐의 장애): 기다림을 풀지 않고 다음 틱에 다시 본다 (풀면 다음 확인까지 3시간을 기다린다)
                Checked::Unknown => return,
            }
        }
        // 기억한 접속자 수는 최대 1분 전이고 확인하는 동안 들어왔을 수 있다: 끄기 바로 전에 지금 수를 묻는다 (모르면 기다린다)
        let running = super::backup::is_running(state, id).await;
        if running {
            players = crate::supervisor::players::fresh(&state.supervisor, id).await;
        }
        step = auto_step(running, players);
    }
    if step != AutoStep::Wait {
        let Some(mark) = Pending::take(id) else { return };
        watch.waiting = false;
        let (state, id, remote) = (state.clone(), id.to_string(), watch.remote.clone());
        crate::audit::spawn(async move {
            let _mark = mark;
            match stop_update_start_when(&state, &id, true).await {
                Some(updated) => record_auto_result(&id, remote.as_deref(), updated),
                // 끄기 직전에 누가 들어왔거나(셀 수 없거나, 기다림을 알렸다) 다른 작업 중: 다음 틱부터 1분마다 다시 본다 (`postponed`)
                None => tracing::info!("Auto update of '{}' postponed", id),
            }
        });
        return;
    }
    if watch.notified {
        return;
    }
    watch.notified = true;
    emit_waiting(state, id, players).await;
}

/// 자동 업데이트가 접속자 때문에 기다린다고 알린다 (`instance.update_waiting`)
async fn emit_waiting(state: &IPCServer, id: &str, players: Option<u32>) {
    let name = state
        .supervisor
        .read()
        .await
        .instance_store
        .get(id)
        .map(|i| i.name.clone())
        .unwrap_or_default();
    crate::events::emit(
        "instance.update_waiting",
        json!({ "instanceId": id, "instance": name, "players": players }),
    );
}

/// 업데이트 확인 결과
#[derive(Debug, PartialEq, Eq)]
enum Checked {
    /// 새 빌드가 있다
    Available,
    /// 새 빌드가 없다 (자동 업데이트가 실패했던 빌드뿐일 때도)
    UpToDate,
    /// 확인하지 못했다 (`check_failed`, 확인 hook이 답하지 않은 `no_hook_handled`)
    Unknown,
}

/// 새 빌드가 있는지 확인한다 (확인 시각을 남기고, 확인하지 못했으면 경고 로그)
async fn update_check(state: &IPCServer, id: &str, watch: &mut Watch) -> Checked {
    watch.checked = Some(std::time::Instant::now());
    let Some(status) = super::instance::server_update_status(state, id).await else {
        return Checked::Unknown;
    };
    match status.get("reason").and_then(Value::as_str) {
        Some("check_failed") => {
            tracing::warn!("Auto update of '{}': update check failed: {}", id, status["error"]);
            return Checked::Unknown;
        }
        // 확인 hook을 가진 익스텐션이 답하지 않았다 (꺼졌거나 다시 읽는 중)
        Some("no_hook_handled") => return Checked::Unknown,
        _ => {}
    }
    if status.get("update_available").and_then(Value::as_bool) != Some(true) {
        return Checked::UpToDate;
    }
    watch.remote = match status.get("remote_buildid") {
        Some(Value::String(b)) => Some(b.clone()),
        Some(v @ Value::Number(_)) => Some(v.to_string()),
        _ => None,
    };
    if watch.remote.as_deref().is_some_and(|remote| failed_before(id, remote)) {
        tracing::info!("Auto update of '{}': build {:?} failed before; waiting for a newer build", id, watch.remote);
        return Checked::UpToDate;
    }
    Checked::Available
}

/// `apply-update`의 본문 (`warn_minutes`: 켜진 서버면 게임 안에 예고할 시간, 기본 5분)
#[derive(Debug, Default, serde::Deserialize)]
pub struct ApplyBody {
    #[serde(default)]
    pub warn_minutes: Option<u32>,
}

/// 예고 시간: 정한 값(최대 60분), 없으면 접속자가 없다고 확인될 때만 예고 없이, 아니면 5분
fn warn_minutes_for(requested: Option<u32>, players: Option<u32>) -> u32 {
    match (requested, players) {
        (Some(m), _) => m.min(60),
        (None, Some(0)) => 0,
        (None, _) => WARN_MINUTES,
    }
}

/// 켜진 서버의 `apply-update`: 예고 뒤 업데이트를 걸고 바로 답한다
pub async fn apply_to_running(state: &IPCServer, id: &str, body: &ApplyBody) -> Response {
    // 업데이트할 수 없는 서버(직접 설치 등)는 예고하거나 끄기 전에 거절한다 (꺼진 서버와 같은 400)
    if let Err(r) = super::instance::updatable(&*state.supervisor.read().await, id) {
        return r;
    }
    // 예고 없이 끌지(0명)는 기억한 수(최대 1분 전)가 아니라 지금 수로 정한다 (모르면 예고한다)
    let players = crate::supervisor::players::fresh(&state.supervisor, id).await;
    let minutes = warn_minutes_for(body.warn_minutes, players);
    match begin_restart_update(state, id, minutes).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "success": true, "id": id, "scheduled": true, "warn_minutes": minutes, "players": players })),
        )
            .into_response(),
        Err(r) => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 데몬의 주기 확인: 자동 업데이트를 켠 서버는 자동 업데이트 쪽이 보고, 나머지는 3시간에 한 번
    #[test]
    fn periodic_checks_skip_auto_update_servers_and_recent_ones() {
        let mut auto = crate::instance::ServerInstance::new("auto", "m");
        auto.auto_update = true;
        let (fresh, stale, never) = (
            crate::instance::ServerInstance::new("fresh", "m"),
            crate::instance::ServerInstance::new("stale", "m"),
            crate::instance::ServerInstance::new("never", "m"),
        );
        let now = std::time::Instant::now();
        let tried: HashMap<String, std::time::Instant> = [
            (fresh.id.clone(), now - std::time::Duration::from_secs(60)),
            (stale.id.clone(), now - CHECK_EVERY),
        ]
        .into();
        let due: Vec<String> = due_checks(&[auto, fresh, stale, never], &tried, now).into_iter().map(|(_, name)| name).collect();
        assert_eq!(due, vec!["stale", "never"]);
    }

    /// 같은 결과를 다시 기억하면 알리지 않는다 (확인 시각만 바뀐다). 결과가 바뀌면 목록을 다시 읽으라고 알린다
    #[tokio::test]
    async fn a_new_check_result_is_announced_once() {
        let mut rx = crate::events::bus().subscribe(None).rx;
        let id = "update-check-announce";
        let mut announced = || {
            let mut n = 0;
            while let Ok(e) = rx.try_recv() {
                n += usize::from(e.kind == "instance.update_checked" && e.data["instanceId"] == json!(id));
            }
            n
        };
        remember_check(id, &json!({ "update_available": false }));
        remember_check(id, &json!({ "update_available": false }));
        assert_eq!(announced(), 1);
        remember_check(id, &json!({ "update_available": true, "remote_buildid": "2" }));
        assert_eq!(announced(), 1);
        assert_eq!(last_check(id).unwrap()["remote_buildid"], json!("2"));
        forget_check(id);
        assert_eq!(announced(), 1);
        assert!(last_check(id).is_none());
    }

    /// 자동 업데이트는 작업 잠금(시작도 잡는다)을 잡은 채로만 끄기를 정한다: 다른 작업·시작이 잡고 있으면 끄지 않고 미룬다
    #[tokio::test]
    async fn auto_update_does_not_stop_while_another_job_holds_the_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let state = IPCServer::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        let id = "auto-update-lock-test";
        let held = super::super::backup::InstanceLock::take(id).expect("free");
        let decided = tokio::time::timeout(std::time::Duration::from_secs(5), stop_update_start_when(&state, id, true))
            .await
            .expect("does not wait for the lock");
        assert_eq!(decided, None);
        drop(held);
        // 미룬 업데이트는 3시간 뒤가 아니라 다음 틱부터 기다림으로 다시 본다
        let mut watch = Watch::default();
        resume_postponed(id, &mut watch);
        assert!(watch.waiting && !watch.notified);
        let mut again = Watch::default();
        resume_postponed(id, &mut again);
        assert!(!again.waiting, "resumed once");
    }

    /// 같은 서버 폴더를 쓰는 다른 인스턴스가 작업 중이거나 켜져 있어 시작하지 못한 자동 업데이트는 실패가 아니다:
    /// 그 빌드를 실패로 기억하지 않고 미뤘다가, 풀리면 다시 시도한다
    #[tokio::test]
    async fn auto_update_blocked_by_an_instance_sharing_the_folder_is_postponed_not_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"steamcmd\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() {
            crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path()));
        }
        sup.initialize().await.unwrap();
        let shared = tmp.path().join("PalServer").to_string_lossy().to_string();
        let mut ids = Vec::new();
        for name in ["a", "b"] {
            let mut inst = crate::instance::ServerInstance::new(name, "game");
            inst.working_dir = Some(shared.clone());
            inst.extension_data.insert(crate::instance::install_method_key("steamcmd"), json!(true));
            ids.push(inst.id.clone());
            sup.instance_store.add(inst).unwrap();
        }
        let state = IPCServer::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        let other = super::super::backup::InstanceLock::take(&ids[1]).expect("free");
        assert!(super::super::instance::update_blocked_by_shared_folder(&state, &ids[0]).await);
        let decided =
            tokio::time::timeout(std::time::Duration::from_secs(5), stop_update_start_when(&state, &ids[0], true))
                .await
                .expect("does not wait for the other instance");
        assert_eq!(decided, None, "a folder in use is not a failed build");
        let mut watch = Watch::default();
        resume_postponed(&ids[0], &mut watch);
        assert!(watch.waiting, "tried again from the next tick");
        drop(other);
        assert!(!super::super::instance::update_blocked_by_shared_folder(&state, &ids[0]).await);
        assert!(super::super::backup::InstanceLock::take(&ids[0]).is_some(), "a postponed update does not keep its lock");
    }

    /// 새 빌드를 보고 기다리던 중 끄기 전 재확인을 하지 못하면(잠깐의 장애) 기다림을 풀지 않는다: 다음 틱에 다시 본다
    #[tokio::test]
    async fn a_failed_recheck_keeps_waiting_for_the_update() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"steamcmd\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() {
            crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path()));
        }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.extension_data.insert(crate::instance::install_method_key("steamcmd"), json!(true));
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        // 확인 hook을 가진 익스텐션이 없어 확인하지 못한다
        let mut watch = Watch { waiting: true, notified: true, ..Watch::default() };
        assert_eq!(update_check(&state, &id, &mut watch).await, Checked::Unknown);
        auto_tick(&state, &id, &mut watch).await;
        assert!(watch.waiting, "an unknown recheck is not 'up to date'");
        assert!(!is_pending(&id), "nothing is started on an unknown recheck");
    }

    /// 꺼져 있으면 받기만, 켜져 있고 아무도 없으면 다시 켜며 업데이트, 접속자가 있거나 셀 수 없으면 기다린다
    #[test]
    fn auto_update_waits_only_while_someone_may_be_playing() {
        assert_eq!(auto_step(false, Some(3)), AutoStep::Update);
        assert_eq!(auto_step(false, None), AutoStep::Update);
        assert_eq!(auto_step(true, Some(0)), AutoStep::Restart);
        assert_eq!(auto_step(true, Some(2)), AutoStep::Wait);
        assert_eq!(
            auto_step(true, None),
            AutoStep::Wait,
            "a server that cannot count players is not restarted under them"
        );
    }

    /// 자동 업데이트가 실패한 빌드는 다시 시도하지 않고, 더 새 빌드나 성공하면 다시 시도한다
    #[test]
    fn a_build_that_failed_to_auto_update_is_not_retried() {
        let id = "srv-failed-build-test";
        record_auto_result(id, Some("100"), false);
        assert!(failed_before(id, "100"));
        assert!(!failed_before(id, "101"), "a newer build is tried");
        record_auto_result(id, Some("101"), true);
        assert!(!failed_before(id, "100"));
    }

    /// 아무도 없다고 확인된 서버는 기다리지 않고, 접속자가 있거나 셀 수 없으면 5분 예고
    #[test]
    fn players_get_five_minutes_and_an_empty_server_none() {
        assert_eq!(warn_minutes_for(None, Some(0)), 0);
        assert_eq!(warn_minutes_for(None, Some(2)), 5);
        assert_eq!(warn_minutes_for(None, None), 5);
        assert_eq!(warn_minutes_for(Some(90), Some(0)), 60);
    }

    /// 같은 서버에 업데이트를 두 번 걸지 않는다. 작업이 끝나면 다시 걸 수 있다
    #[test]
    fn one_update_per_server_at_a_time() {
        let first = Pending::take("srv-pending-test").expect("first");
        assert!(Pending::take("srv-pending-test").is_none());
        assert!(is_pending("srv-pending-test"));
        drop(first);
        assert!(Pending::take("srv-pending-test").is_some());
    }
}
