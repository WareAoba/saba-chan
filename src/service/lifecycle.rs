//! 서버 시작·정지·재시작과 그 감시(자동 재시작, 유휴 정지, 무응답 감시).
//! HTTP 핸들러·채팅·예약·릴레이가 같은 함수를 부른다

use axum::http::StatusCode;
use serde_json::{json, Value};

use super::{audited_result, settled, Failure, Outcome};
use crate::ipc::IPCServer;

/// 서버 시작. 경로(managed / non-managed)는 데몬이 고르고, 작업 기록에 `start` 한 줄로 남는다.
///
/// `id`는 인스턴스 ID, 없으면 이름으로 찾는다(CLI·채팅처럼 이름만 아는 쪽용).
/// 정책은 `Supervisor::uses_managed_start`, 포트 충돌 검사는 전역 설정 하나다.
/// 답은 고른 경로의 답 그대로에 `managed`를 더한 것이다 (실패도 같다)
pub(crate) async fn start(state: &IPCServer, id: &str) -> Outcome {
    let result = start_unrecorded(state, id).await;
    audited_result(state, "start", id, None, None, result).await
}

async fn start_unrecorded(state: &IPCServer, id: &str) -> Outcome {
    let picked = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .find(id)
            .cloned()
            .map(|inst| {
                let managed = supervisor.uses_managed_start(&inst);
                (inst, managed)
            })
    };
    let Some((instance, managed)) = picked else {
        return Err(instance_not_found(id));
    };

    // 점검 중에 들어온 정지도 시작을 취소하도록 세대를 점검 전에 잡는다.
    // 재시작처럼 부른 쪽이 이미 확인한 세대가 있으면 그 값을 쓴다 (`start_when_free`)
    let since = match crate::supervisor::START_SINCE.try_with(|g| *g) {
        Ok(checked) => checked,
        Err(_) => state.supervisor.read().await.run_generation_of(&instance.id),
    };
    crate::supervisor::START_SINCE.scope(since, start_checked(state, instance, managed)).await
}

async fn start_checked(state: &IPCServer, instance: crate::instance::ServerInstance, managed: bool) -> Outcome {
    let _transition = crate::instance::transition::Transition::begin(&instance.id, crate::instance::transition::Kind::Starting);
    if let Some(blocked) = preflight(state, &instance.id).await {
        return Err(blocked);
    }

    // id로 넘긴다 (이름은 겹칠 수 있다)
    let mut result = launch(state, &instance.id, &instance.module_name, managed).await;
    let body = match &mut result {
        Ok(body) => body,
        Err(failure) => &mut failure.body,
    };
    let mut map = match std::mem::take(body) {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("data".into(), other);
            map
        }
    };
    map.insert("managed".into(), json!(managed));
    *body = announce_updates(state, &instance.id, Value::Object(map)).await;
    result
}

/// 켜기 전 점검: 동의하지 않은 모듈 약관이 있거나, 모듈 validate가 critical 문제(EULA 미동의, 서버 파일 없음 등)를 내면 켜지 않는다.
/// 이미 켜져 있거나, 모듈에 validate가 없거나, 점검 자체가 실패하면 막지 않는다
async fn preflight(state: &IPCServer, id: &str) -> Option<Failure> {
    {
        let supervisor = state.supervisor.read().await;
        if supervisor.is_running(id).await {
            return None;
        }
    }
    // 모듈 약관(`[[agreements]]`)을 모두 동의해야 켠다. 파이썬을 부르지 않는 검사라 validate보다 먼저
    let language = state.config_store.ui_language().await;
    let missing = state.supervisor.read().await.missing_agreements(id, &language);
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().filter_map(|a| a["label"].as_str()).collect();
        let message = crate::chat::i18n::Catalog::for_language(&language).t("errors.agreement_required", &[("names", &names.join(", "))]);
        tracing::info!("Start of '{}' blocked: agreements not accepted", id);
        return Some(Failure::new(
            StatusCode::CONFLICT,
            json!({ "success": false, "error": message, "error_code": "agreement_required", "agreements": missing }),
        ));
    }
    let result = match crate::supervisor::Supervisor::validate_instance(&state.supervisor, id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Pre-start check for '{}' failed to run: {}", id, e);
            return None;
        }
    };
    let issues = result.get("issues").and_then(Value::as_array)?;
    let critical: Vec<&Value> = issues.iter().filter(|i| i.get("severity").and_then(Value::as_str) == Some("critical")).collect();
    let first = critical.first()?;
    let code = first.get("code").and_then(Value::as_str).unwrap_or("preflight_failed");
    let message = first.get("message").and_then(Value::as_str).unwrap_or("Pre-start check failed");
    tracing::info!("Start of '{}' blocked by pre-start check: {}", id, code);
    Some(Failure::new(
        StatusCode::CONFLICT,
        json!({
            "success": false,
            "error": message,
            "error_code": "preflight_failed",
            "issue_code": code,
            "solution": first.get("solution"),
            "issues": critical,
        }),
    ))
}

/// id(또는 고유한 이름)로 인스턴스를 찾아 (id, 이름)을 돌려준다
async fn instance_key_of(state: &IPCServer, key: &str) -> Option<(String, String)> {
    let supervisor = state.supervisor.read().await;
    supervisor
        .instance_store
        .find(key)
        .map(|i| (i.id.clone(), i.name.clone()))
}

pub(crate) fn instance_not_found(id: &str) -> Failure {
    Failure::coded(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id))
}

/// 정지 뒤 시작. 꺼져 있으면 시작만 한다. 작업 기록에는 `restart` 한 줄로 남는다.
/// 답은 시작 답에 `stopped`(정지했는가)를 더한 것. 정지가 실패하면 그 답을 그대로 준다
pub(crate) async fn restart(state: &IPCServer, id: &str, force: bool) -> Outcome {
    // 안에서 부르는 정지·시작은 따로 남기지 않는다
    let result = crate::audit::quietly(restart_unrecorded(state, id, StopMode::of(force, false))).await;
    audited_result(state, "restart", id, None, None, result).await
}

/// 사람이 부른 재시작 (`user_stop`과 같이 저장하지 못하는 서버도 끄고 다시 켠다)
pub(crate) async fn user_restart(state: &IPCServer, id: &str, force: bool) -> Outcome {
    let result = crate::audit::quietly(restart_unrecorded(state, id, StopMode::of(force, true))).await;
    audited_result(state, "restart", id, None, None, result).await
}

/// 정지 방식. 저장 후 종료를 요청할 길이 없는 서버(MK10)를 사람이 부른 정지는 강제 정지로 잇고(`SaveIfPossible`),
/// 예약·유휴·업데이트 정지(`Graceful`)는 끄지 않고 그만둔다
#[derive(Clone, Copy, PartialEq, Eq)]
enum StopMode {
    Graceful,
    SaveIfPossible,
    Force,
}

impl StopMode {
    fn of(force: bool, by_user: bool) -> Self {
        match (force, by_user) {
            (true, _) => Self::Force,
            (false, true) => Self::SaveIfPossible,
            (false, false) => Self::Graceful,
        }
    }
}

/// 재시작이 정지 뒤 이전 프로세스가 끝나기를 기다리는 최소 시간 (정지 대기 시간이 더 길면 그만큼)
const RESTART_EXIT_WAIT_FLOOR: std::time::Duration = std::time::Duration::from_secs(if cfg!(test) { 2 } else { 120 });

async fn restart_unrecorded(state: &IPCServer, id: &str, mode: StopMode) -> Outcome {
    let picked = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.find(id).map(|i| {
            let module_timeout = supervisor.module_loader.get_module(&i.module_name).ok().and_then(|m| m.metadata.stop_timeout);
            (i.id.clone(), crate::supervisor::stop_timeout_of(i, module_timeout), supervisor.tracker.get_pid(&i.id).ok())
        })
    };
    let Some((inst_id, stop_timeout, old_pid)) = picked else {
        return Err(instance_not_found(id));
    };
    // 이 재시작의 정지가 올린 세대보다 더 올랐으면 그사이 다른 정지·시작이 들어온 것이다 (D-S8).
    // 기준은 정지가 알려 준 값이다: 정지 전에 따로 읽으면 그 틈에 끼어든 자동 재시작 시작까지 세어 스스로 취소한다 (D-S10)
    let before_stop = state.supervisor.read().await.run_generation_of(&inst_id);
    let running;
    let stopped_at;
    {
        // 데몬이 추적하지 않아도 익스텐션이 맡은 서버일 수 있다: 정지를 먼저 맡긴다
        let stopping = async {
            let result = stop_recorded(state, &inst_id, mode).await;
            (result, crate::supervisor::STOPPED_AT.with(|at| at.get()))
        };
        let (stopped, at) = crate::supervisor::STOPPED_AT.scope(std::cell::Cell::new(None), stopping).await;
        stopped_at = at.unwrap_or(before_stop + 1);
        // 꺼져 있던 일반 인스턴스는 정지가 아무것도 하지 않고 was_running=false를 준다
        running = stopped?.get("was_running").and_then(Value::as_bool) != Some(false);
        // 정지 응답이 프로세스가 끝나기 전에 올 수 있다: 끝날 때까지 기다린다 (정지 대기 시간만큼, 적어도 2분).
        // non-managed 정지는 모듈이 성공이라고 답하면 추적을 바로 놓으므로, 정지 전에 알던 PID가 살아 있는지도 본다
        let deadline = tokio::time::Instant::now() + RESTART_EXIT_WAIT_FLOOR.max(std::time::Duration::from_secs(stop_timeout));
        loop {
            let alive = old_pid.is_some_and(crate::supervisor::process::is_process_alive) || {
                let supervisor = state.supervisor.read().await;
                supervisor.is_running(&inst_id).await
            };
            if !alive {
                break;
            }
            // 이전 프로세스가 아직 저장 중일 수 있다: 같은 월드를 여는 새 서버를 띄우지 않는다
            if tokio::time::Instant::now() >= deadline {
                let error = json!({ "success": false, "error": "The server is still shutting down; start it again once it has stopped", "error_code": "still_stopping", "stopped": false });
                return Err(Failure::new(StatusCode::CONFLICT, error));
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    // 백업·복원이 잠금을 쥐고 있으면 끝나기를 기다렸다 켠다. 요청이 끊겨도 꺼진 채 남지 않게 따로 돌린다.
    // 그사이 다른 쪽에서 켜거나 인스턴스를 지우면 그만둔다
    let generation = state.supervisor.read().await.run_generation_of(&inst_id);
    if generation > stopped_at {
        let error = json!({ "success": false, "error": "The server was stopped or started while restarting", "error_code": "restart_cancelled", "stopped": running });
        return Err(Failure::new(StatusCode::CONFLICT, error));
    }
    let state = state.clone();
    let start = crate::audit::spawn(async move {
        let wanted = || async {
            let supervisor = state.supervisor.read().await;
            supervisor.instance_store.get(&inst_id).is_some() && supervisor.run_generation_of(&inst_id) == generation
        };
        start_when_free(&state, &inst_id, generation, wanted).await
    })
    .await;
    let mut result = match start {
        Ok(Some(result)) => result,
        Ok(None) => {
            let error = json!({ "success": false, "error": "The server was started or removed while waiting", "error_code": "restart_cancelled", "stopped": running });
            return Err(Failure::new(StatusCode::CONFLICT, error));
        }
        Err(e) => return Err(Failure::coded(StatusCode::INTERNAL_SERVER_ERROR, "internal_server_error", e.to_string())),
    };
    let body = match &mut result {
        Ok(body) => body,
        Err(failure) => &mut failure.body,
    };
    let mut map = match std::mem::take(body) {
        Value::Object(map) => map,
        other => serde_json::Map::from_iter([("data".to_string(), other)]),
    };
    map.insert("stopped".into(), json!(running));
    *body = Value::Object(map);
    result
}

/// 작업 잠금이 풀리면 시작한다. 재시작이 백업·복원과 겹쳐도 서버가 꺼진 채 남지 않게 한다.
/// 기다리는 동안 `still_wanted`가 false가 되면 그만두고 None.
/// `generation`은 부른 쪽이 확인한 실행 세대다: 시작도 이 값을 기준으로 삼아, 확인 뒤에 들어온 정지가 시작을 취소하게 한다
pub(crate) async fn start_when_free<F, Fut>(state: &IPCServer, id: &str, generation: u64, still_wanted: F) -> Option<Outcome>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + START_WAIT_LIMIT;
    loop {
        let result = crate::supervisor::START_SINCE.scope(generation, Box::pin(start(state, id))).await;
        if !result.as_ref().is_err_and(Failure::is_busy) || std::time::Instant::now() >= deadline {
            return Some(result);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if !still_wanted().await {
            return None;
        }
    }
}

/// 재시작이 작업 잠금이 풀리기를 기다리는 가장 긴 시간 (큰 월드의 백업·업데이트)
const START_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// 크래시 뒤 자동 재시작 (A1). 대기하는 동안 사용자가 정지·시작했으면 그만둔다.
pub async fn auto_restart(state: IPCServer, instance_id: String, delay: std::time::Duration, generation: u64) {
    tokio::time::sleep(delay).await;
    if !state.supervisor.read().await.restart_still_wanted(&instance_id, generation).await {
        tracing::info!("Auto restart of '{}' skipped: stopped or started meanwhile", instance_id);
        state.supervisor.read().await.record_restart_outcome(&instance_id, generation, false);
        return;
    }
    tracing::info!("Auto restarting '{}' after a crash", instance_id);
    // 백업이 잠금을 쥐고 있으면 끝나기를 기다린다 (기다리는 동안 사용자가 정지·시작하면 그만둔다)
    let wanted = || async { state.supervisor.read().await.restart_still_wanted(&instance_id, generation).await };
    // 다시 띄운 시작의 출처는 자동 재시작이다 (`instance.started`·감사 로그)
    let restart = crate::audit::Origin::new("restart");
    let Some(result) = crate::audit::with_origin(restart, start_when_free(&state, &instance_id, generation, wanted)).await else {
        tracing::info!("Auto restart of '{}' skipped: stopped or started meanwhile", instance_id);
        state.supervisor.read().await.record_restart_outcome(&instance_id, generation, false);
        return;
    };
    match result {
        Err(failure) => {
            let reason = failure.body.get("error").or_else(|| failure.body.get("message")).and_then(Value::as_str).unwrap_or("start failed").to_string();
            tracing::warn!("Auto restart of '{}' failed: {}", instance_id, reason);
            state.supervisor.write().await.record_restart_failure(&instance_id, generation, reason);
        }
        Ok(_) => state.supervisor.read().await.record_restart_outcome(&instance_id, generation, true),
    }
}

/// 시작 준비와 띄우기 (`start_checked`가 id로 부른다). 관리형·비관리형이 같은 준비를 거치고,
/// 시작 전 검사는 감독자(`Supervisor::plan_start`)가 두 방식에 같게 한다 (D2). 작업 기록은 `start`가 남긴다
pub(crate) async fn launch(state: &IPCServer, id: &str, module: &str, managed: bool) -> Outcome {
    // 백업·복원·업데이트가 서버 파일을 다루는 동안에는 켜지 않는다 (시작을 마칠 때까지 잡는다)
    let Some(_guard) = crate::instance::lock::InstanceLock::take_for_writes(id).await else {
        return Err(Failure::busy());
    };
    // 포트포워딩 확인이 시험 리스너로 포트를 잡고 있으면 놓을 때까지 (몇 초). 시작하는 동안에는 새 확인이 포트를 잡지 않는다
    let _starting = crate::port_check::begin_start(id).await;
    fill_passwords(state, id).await;
    // API 경유 시작 기록 (GUI 외부 시작 감지용)
    state.api_actions.record(id);
    // 익스텐션 상태 캐시 무효화 — 시작 직후 fresh 상태 반영
    state.extension_status_cache.invalidate(id);

    // 락은 감독자의 시작이 단계마다 짧게 잡는다 (C12)
    if managed {
        match crate::supervisor::Supervisor::start_managed_server(&state.supervisor, id, module, json!({})).await {
            Ok(result) => settled(result),
            Err(e) => Err(start_failure(state, &e).await),
        }
    } else {
        match crate::supervisor::Supervisor::start_server(&state.supervisor, id, module, json!({})).await {
            Ok(result) => settled(result),
            Err(e) => Err(lifecycle_failure(&e, "start", "start_failed")),
        }
    }
}

/// 시작·정지가 실패한 응답. 상태는 오류 문구가 아니라 종류로 고른다: 인스턴스가 없으면 404 `instance_not_found`,
/// 모듈 호출 실패는 모듈이 준 본문 그대로 500, 그 밖은 500 `failed_code`
fn lifecycle_failure(e: &anyhow::Error, action: &str, failed_code: &str) -> Failure {
    if e.downcast_ref::<crate::plugin::ModuleCallError>().is_some() {
        return Failure::new(StatusCode::INTERNAL_SERVER_ERROR, crate::plugin::error_response(e));
    }
    let (code, error_code) = if e.downcast_ref::<crate::supervisor::InstanceNotFound>().is_some() {
        (StatusCode::NOT_FOUND, "instance_not_found")
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, failed_code)
    };
    Failure::coded(code, error_code, format!("Failed to {} server: {}", action, e))
}

/// 받아 둔 업데이트 확인 결과에서 이 모듈의 새 버전 `{module, current, latest}`
fn module_update_of(status: &saba_chan_updater_lib::UpdateStatus, module: &str) -> Option<Value> {
    use saba_chan_updater_lib::Component;
    status
        .components
        .iter()
        .find(|c| c.update_available && matches!(&c.component, Component::Module(m) if m == module))
        .map(|c| json!({ "module": module, "current": c.current_version, "latest": c.latest_version }))
}

/// 새로 켠 서버의 업데이트 알림 `instance.update_notice {instanceId, instance, kind, module?, latest?}`.
/// - `module`: 모듈에 새 버전이 있다 (받아 둔 확인 결과라 바로 낸다). 시작 답에도 `module_update`로 싣는다
/// - `server`: 게임 서버에 새 빌드가 있다 (`server.check_update` hook, 오래 걸릴 수 있어 뒤에서)
async fn announce_updates(state: &IPCServer, id: &str, mut result: Value) -> Value {
    let fresh = result.get("success").and_then(Value::as_bool) == Some(true)
        && result.get("already_running").and_then(Value::as_bool) != Some(true);
    if !fresh {
        return result;
    }
    let Some((name, module)) = state.supervisor.read().await.instance_store.get(id).map(|i| (i.name.clone(), i.module_name.clone())) else {
        return result;
    };
    // 받거나 적용하는 중이면 매니저가 잠겨 있다: 덧붙이는 안내 때문에 시작 답을 기다리게 하지 않는다
    let status = state.update_state.manager.try_read().ok().map(|m| m.get_status());
    if let Some(update) = status.as_ref().and_then(|s| module_update_of(s, &module)) {
        crate::events::emit(
            "instance.update_notice",
            json!({ "instanceId": id, "instance": name, "kind": "module", "module": module, "latest": update["latest"] }),
        );
        result["module_update"] = update;
    }
    let (state, id) = (state.clone(), id.to_string());
    tokio::spawn(async move {
        let found = crate::service::server_update::apply::server_update_status(&state, &id).await;
        if found.and_then(|s| s.get("update_available").and_then(Value::as_bool)) == Some(true) {
            crate::events::emit("instance.update_notice", json!({ "instanceId": id, "instance": name, "kind": "server" }));
        }
    });
    result
}

/// 응답 없는 서버 감시 (`supervisor::watchdog`): 켜진 서버가 인스턴스 설정의 시간 넘게 상태 질문에 답하지 않으면
/// `instance.unresponsive`를 내고, 설정이 `restart`면 다시 시작한다. 접속자 수를 갱신한 바로 뒤에 1분마다 부른다
pub async fn watch_unresponsive(state: &IPCServer) {
    watch_unresponsive_at(state, std::time::Instant::now()).await
}

pub(crate) async fn watch_unresponsive_at(state: &IPCServer, now: std::time::Instant) {
    use crate::supervisor::{players, watchdog};
    let due: Vec<(String, String, u64, watchdog::Verdict)> = {
        let supervisor = state.supervisor.read().await;
        let mut due = Vec::new();
        for instance in supervisor.instance_store.list() {
            let (running, pid, _) = supervisor.tracked_state(&instance.id).await;
            if !running {
                watchdog::forget(&instance.id);
                continue;
            }
            let silent = players::silent_for(&supervisor.runtime, &instance.id, pid, now).map(|(silent, _)| silent);
            if let Some(verdict) = watchdog::observe(&instance.id, pid, watchdog::rule_of(instance), silent, now) {
                due.push((instance.id.clone(), instance.name.clone(), silent.unwrap_or_default().as_secs() / 60, verdict));
            }
        }
        due
    };
    for (id, name, minutes, verdict) in due {
        let restarting = verdict == watchdog::Verdict::Restart;
        tracing::warn!("Server '{}' has not answered for {} minutes{}", id, minutes, if restarting { "; restarting it" } else { "" });
        crate::events::emit("instance.unresponsive", json!({ "instanceId": id, "instance": name, "minutes": minutes, "restarting": restarting }));
        if restarting {
            // 재시작은 정지 대기 시간만큼 걸린다: 다른 서버의 감시를 붙잡지 않게 뒤에서 한다
            let state = state.clone();
            tokio::spawn(async move {
                let restarted = crate::audit::with_origin(crate::audit::Origin::new("watchdog"), Box::pin(restart(&state, &id, false))).await;
                if let Err(failure) = restarted {
                    tracing::warn!("Could not restart the unresponsive server '{}': {}", id, failure.body);
                }
            });
        }
    }
}

/// 접속자 0명이 정한 시간 이어진 켜진 서버를 저장하고 정지한다 (인스턴스 설정 `idle_stop`, `supervisor::idle`).
/// 접속자 수를 갱신한 바로 뒤에 1분마다 부른다. 강제 종료하지 않는다: 정지하지 못하면 다음 차례에 다시 본다
pub async fn stop_idle_servers(state: &IPCServer) {
    stop_idle_servers_at(state, saba_runner::now()).await
}

pub(crate) async fn stop_idle_servers_at(state: &IPCServer, now: u64) {
    use crate::supervisor::{idle, players};
    let (runtime, due): (_, Vec<(String, u64)>) = {
        let supervisor = state.supervisor.read().await;
        let mut due = Vec::new();
        for instance in supervisor.instance_store.list() {
            let (running, _, started) = supervisor.tracked_state(&instance.id).await;
            if !running {
                idle::forget(&instance.id);
                continue;
            }
            // 켠 시각을 모르는 서버(다시 붙은 뒤 추적표에 없는 것)는 켠 직후가 아니라고 본다
            let started = started.unwrap_or_default();
            let count = players::get(&supervisor.runtime, &instance.id).map(|c| c.online);
            if let Some(minutes) = idle::observe(&instance.id, idle::rule_of(instance), started, count, now) {
                due.push((instance.id.clone(), minutes));
            }
        }
        (supervisor.runtime.clone(), due)
    };
    for (id, minutes) in due {
        // 그사이 누가 들어왔으면 끄지 않는다
        if players::get(&runtime, &id).map(|c| c.online) != Some(0) {
            continue;
        }
        tracing::info!("Stopping '{}': no players for {} minutes", id, minutes);
        // 정지 전에 적어 둔다: 정지 알림(`instance.stopped` `idleMinutes`)이 이 분을 싣는다
        idle::mark_stopped(&id, minutes, saba_runner::now());
        match crate::audit::with_origin(crate::audit::Origin::new("idle"), Box::pin(stop(state, &id, false))).await {
            Ok(_) => idle::forget(&id),
            Err(failure) => {
                idle::clear_stopped(&id);
                tracing::warn!("Could not stop the idle server '{}': {}", id, failure.body);
            }
        }
    }
}

/// 인스턴스 하나를 id로 정지한다. 이름은 겹칠 수 있으니 정지 경로는 모두 id를 쓴다. 작업 기록에 `stop`으로 남는다
pub(crate) async fn stop(state: &IPCServer, key: &str, force: bool) -> Outcome {
    stop_recorded(state, key, StopMode::of(force, false)).await
}

/// 사람이 부른 정지 (정지 라우트: 앱·웹·CLI·디스코드·챗봇). 저장 후 종료를 요청할 길이 없는 서버(MK10)도 묻지 않고
/// 바로 강제 정지한다 (사용자 2026-11-13). 챗봇은 실행 전 되묻기(`actions::needs_confirm`)를 거친다
pub(crate) async fn user_stop(state: &IPCServer, key: &str, force: bool) -> Outcome {
    stop_recorded(state, key, StopMode::of(force, true)).await
}

async fn stop_recorded(state: &IPCServer, key: &str, mode: StopMode) -> Outcome {
    let result = match stop_unrecorded(state, key, mode == StopMode::Force).await {
        Err(failure) if mode == StopMode::SaveIfPossible && failure.body.get("error_code").and_then(Value::as_str) == Some("graceful_stop_unavailable") => {
            tracing::info!("'{}' has no way to save and exit; stopping it without saving", key);
            stop_unrecorded(state, key, true).await
        }
        result => result,
    };
    audited_result(state, "stop", key, None, None, result).await
}

async fn stop_unrecorded(state: &IPCServer, key: &str, force: bool) -> Outcome {
    let Some((id, name)) = instance_key_of(state, key).await else {
        return Err(instance_not_found(key));
    };
    let id = id.as_str();
    let _transition = crate::instance::transition::Transition::begin(id, crate::instance::transition::Kind::Stopping);
    // 정지가 끝날 때까지 복원 전 백업 정리를 붙잡아 둔다 (정리가 "돌고 있다"고 본 서버를 지우는 동안 멈추지 않게, 반대로 멈추는 중에는 지우지 않게)
    let gate = state.supervisor.read().await.runtime.stop_gate(id);
    let _stopping = gate.lock_owned().await;
    // API 경유 정지 기록 (GUI 외부 정지 감지용)
    state.api_actions.record(id);
    // 익스텐션 상태 캐시 무효화 — 정지 직후 fresh 상태 반영
    {
        let sup = state.supervisor.read().await;
        if sup.instance_store.get(id).is_some() {
            state.extension_status_cache.invalidate(id);
        }
    }

    // instance에서 모듈명과 ID 조회 (read lock으로 조회 후 즉시 해제)
    let instance = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.get(id).cloned()
    };

    if let Some(inst) = instance {
        // 락은 stop_server가 단계마다 짧게 잡는다 (C12)
        match crate::supervisor::Supervisor::stop_server(&state.supervisor, &inst.id, &inst.module_name, force).await {
            Ok(result) => {
                // 실제 종료 성공 시에만 tracker에서 제거
                let success = result
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                if success {
                    // tracker는 id로만 찾는다: 이름이 다른 인스턴스의 id와 같아도 그쪽을 건드리지 않는다
                    let supervisor = state.supervisor.read().await;
                    // managed 정지는 stop_server 안에서 이미 tracker를 비운다: NotFound는 정상이다
                    match supervisor.tracker.untrack(&inst.id) {
                        Ok(()) => tracing::info!("Server '{}' untracked from process tracker", name),
                        Err(crate::supervisor::process::ProcessError::NotFound { .. }) => {}
                        Err(e) => tracing::warn!("Failed to untrack stopped server '{}': {}", name, e),
                    }
                }
                settled(result)
            }
            Err(e) => Err(lifecycle_failure(&e, "stop", "stop_failed")),
        }
    } else {
        Err(Failure::coded(StatusCode::NOT_FOUND, "instance_not_found", format!("Server '{}' not found", name)))
    }
}

// NOTE: sync_gui_config は config::save_gui_config に統合されました (portConflictCheck 同期含む)
/// 실행 전 비밀번호: 빈 REST 비밀번호는 GUI "비밀번호 자동 생성"(`autoGeneratePasswords`, 기본 켬)이 켜져 있을 때만 채운다.
/// 꺼져 있으면 비워 두고, 모듈이 게임 설정의 값을 쓰거나 스스로 만든다(`rest_credentials`로 데몬에 돌아온다).
/// 채우는 곳은 모듈이 선언한 설정(`[protocols.rest] password_setting`)이다: 선언이 없으면 만들지 않는다. `key`: 인스턴스 id 또는 이름
async fn fill_passwords(state: &IPCServer, key: &str) {
    let generate = state.config_store.get_gui_settings().await.auto_generate_passwords;
    let mut supervisor = state.supervisor.write().await;
    let found = supervisor.instance_store.find(key).cloned();
    let Some(mut inst) = found else { return };
    let password_setting = supervisor.module_loader.get_module(&inst.module_name).ok().and_then(|m| m.metadata.rest()?.password_setting.clone());
    let changed = generate && password_setting.is_some_and(|key| inst.ensure_rest_password(&key));
    if changed {
        let id = inst.id.clone();
        if let Err(e) = supervisor.instance_store.update(&id, inst) {
            tracing::warn!("Failed to save auto-generated passwords for {}: {}", key, e);
        }
    }
}

/// 관리형 시작(saba-runner가 콘솔 stdin/stdout을 잡는다)이 실패한 응답
async fn start_failure(state: &IPCServer, e: &anyhow::Error) -> Failure {
    match e.downcast_ref::<crate::supervisor::managed_process::MemoryLimitAtStart>() {
        Some(hit) => {
            let language = state.config_store.ui_language().await;
            let message = crate::chat::i18n::Catalog::for_language(&language)
                .t("errors.memory_limit_at_start", &[("limit", &hit.limit_mb.to_string())]);
            Failure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "success": false, "error": message, "error_code": "memory_limit_reached", "limit_mb": hit.limit_mb }),
            )
        }
        None if e.downcast_ref::<crate::supervisor::managed_process::RunnerMissing>().is_some() => {
            let language = state.config_store.ui_language().await;
            let message = crate::chat::i18n::Catalog::for_language(&language).t("errors.runner_missing", &[]);
            Failure::coded(StatusCode::UNPROCESSABLE_ENTITY, "runner_missing", message)
        }
        // 시작하는 사이 지워졌다: 비관리 시작과 같은 404 `instance_not_found`
        None if e.downcast_ref::<crate::supervisor::InstanceNotFound>().is_some() => {
            lifecycle_failure(e, "start", "start_failed")
        }
        None => Failure::new(StatusCode::INTERNAL_SERVER_ERROR, crate::plugin::error_response(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// 켜려는 서버의 모듈에 새 버전이 있으면 시작 답과 알림에 싣는다 (받아 둔 업데이트 확인 결과, 다른 모듈·없는 새 버전은 아니다)
    #[test]
    fn a_newer_module_version_is_found_from_the_last_update_check() {
        let component = |module: &str, available: bool| {
            json!({ "component": { "module": module }, "current_version": "1.0.0", "latest_version": "1.1.0", "update_available": available,
                    "download_url": null, "asset_name": null, "release_notes": null, "published_at": null, "downloaded": false, "downloaded_path": null, "installed": true })
        };
        let status: saba_chan_updater_lib::UpdateStatus = serde_json::from_value(json!({
            "last_check": null, "next_check": null, "checking": false, "error": null,
            "components": [component("palworld", true), component("minecraft", false)],
        }))
        .unwrap();
        assert_eq!(module_update_of(&status, "palworld"), Some(json!({ "module": "palworld", "current": "1.0.0", "latest": "1.1.0" })));
        assert_eq!(module_update_of(&status, "minecraft"), None);
        assert_eq!(module_update_of(&status, "ark"), None);
    }

    /// GUI "비밀번호 자동 생성"을 끄면 시작할 때 빈 REST 비밀번호를 채우지 않는다. 켜져 있으면(기본) 채운다.
    /// REST를 지원하지 않는 모듈의 서버에는 REST 계정을 만들지 않는다 (테라리아 점검: 설정 화면에 쓸모없는 값이 보였다)
    #[tokio::test]
    async fn rest_passwords_are_generated_only_when_the_setting_is_on() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, supported) in [("game", "rest"), ("console", "stdin")] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let toml = format!(
                "[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"{supported}\"]\n\n\
                 [protocols.rest]\nport_setting = \"api_port\"\npassword_setting = \"admin_pw\"\n\n\
                 [[settings.fields]]\nname = \"api_port\"\ntype = \"number\"\nport = true\nlabel = \"api\"\n\n\
                 [[settings.fields]]\nname = \"admin_pw\"\ntype = \"password\"\nlabel = \"pw\"\n"
            );
            std::fs::write(dir.join("module.toml"), toml).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        state.config_store = Arc::new(crate::config_store::ConfigStore::new(&tmp.path().join("data")));
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        let plain = crate::instance::ServerInstance::new("terra", "console");
        let plain_id = plain.id.clone();
        state.supervisor.write().await.instance_store.add(inst).unwrap();
        state.supervisor.write().await.instance_store.add(plain).unwrap();
        let rest = |id: String| {
            let state = state.clone();
            async move {
                let sup = state.supervisor.read().await;
                sup.instance_store.get(&id).unwrap().module_settings.get("admin_pw").and_then(|p| p.as_str()).map(str::to_string)
            }
        };

        state.config_store.set_gui_settings_from_json(json!({ "autoGeneratePasswords": false })).await.unwrap();
        fill_passwords(&state, &id).await;
        assert_eq!(rest(id.clone()).await, None, "the setting is off");
        state.config_store.set_gui_settings_from_json(json!({ "autoGeneratePasswords": true })).await.unwrap();
        fill_passwords(&state, "srv").await;
        assert_eq!(rest(id.clone()).await.map(|p| p.len()), Some(16), "found by name too");
        fill_passwords(&state, &plain_id).await;
        assert_eq!(rest(plain_id).await, None, "a module without REST gets no REST password");
    }

    /// 재시작의 정지 단계(서버가 끝나기를 기다리는 동안)에 들어온 정지도 재시작을 취소한다 (D-S8)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_a_restart_keeps_the_server_down() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_timeout = 3\n",
        )
        .unwrap();
        // 정상 종료 요청(TERM)을 무시해 정지가 stop_timeout까지 걸리는 서버
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', \"trap '' TERM; sleep 60\"], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-restart".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let started = crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, "stop-during-restart").await).body;
        assert_ne!(started["success"], false, "{started}");

        let restarter = {
            let state = state.clone();
            tokio::spawn(async move {
                crate::ipc::internal::InternalResponse::from(restart_unrecorded(&state, "stop-during-restart", StopMode::Graceful).await).body
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        let _ = crate::supervisor::Supervisor::stop_server(&state.supervisor, "stop-during-restart", "game", false).await;

        let restarted = restarter.await.unwrap();
        assert_eq!(restarted["error_code"], "restart_cancelled", "{restarted}");
        let s = state.supervisor.read().await;
        assert!(s.tracker.get_pid("stop-during-restart").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-during-restart").await.is_none(), "the restart did not bring the server back");
    }

    /// 정지를 받고도 기다리는 시간 안에 끝나지 않은 서버(월드 저장이 긴 서버)를 두고 재시작이 성공했다고 답했다:
    /// 시작이 "이미 켜져 있음"을 성공으로 돌려줬고, 그 서버는 곧 꺼진 채로 남았다. 끝나지 않았으면 켜지 않고 오류로 답한다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_restart_does_not_report_success_while_the_old_server_is_still_stopping() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_timeout = 1\n",
        )
        .unwrap();
        // 정지를 요청만 하고 바로 답하는 모듈: 서버는 그 뒤로도 한참 저장한다
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True, 'issues': []}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "slow-save".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track("slow-save", child.id()).unwrap();

        let restarted = crate::ipc::internal::InternalResponse::from(restart_unrecorded(&state, "slow-save", StopMode::Graceful).await);
        assert_eq!(restarted.status, StatusCode::CONFLICT, "{}", restarted.body);
        assert_eq!((restarted.body["success"].clone(), restarted.body["error_code"].clone()), (json!(false), json!("still_stopping")), "{}", restarted.body);
        assert!(child.try_wait().unwrap().is_none(), "the old server is left to finish saving");
        let _ = child.kill();
        let _ = child.wait();
    }

    /// 시작 전 점검(모듈 validate) 중에 들어온 정지도 시작을 취소한다: 그 정지는 PID가 없어 바로 성공으로 끝난다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_the_pre_start_check_keeps_the_server_down() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\n\
             if sys.argv[1] == 'validate':\n    time.sleep(2)\n    print(json.dumps({'success': True, 'issues': []}))\n\
             elif sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-check".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let starter = {
            let state = state.clone();
            tokio::spawn(async move {
                crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, "stop-during-check").await).body
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let stopped = crate::supervisor::Supervisor::stop_server(&state.supervisor, "stop-during-check", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");

        let started = starter.await.unwrap();
        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let s = state.supervisor.read().await;
        assert!(s.tracker.get_pid("stop-during-check").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-during-check").await.is_none(), "no server was left running");
    }

    /// 재시작·자동 재시작은 세대를 확인한 뒤 시작한다. 확인과 시작 사이에 들어온 정지도 시작을 취소해야 한다:
    /// 시작은 확인한 세대를 기준으로 삼는다(새로 읽으면 그 정지가 기준에 들어가 버린다)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_between_the_restart_check_and_its_start_keeps_the_server_down() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True, 'issues': []}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-after-check".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let checked = state.supervisor.read().await.run_generation_of("stop-after-check");
        let stopped = crate::supervisor::Supervisor::stop_server(&state.supervisor, "stop-after-check", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");
        let started = start_when_free(&state, "stop-after-check", checked, || async { true }).await.unwrap().unwrap_err().body;

        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let s = state.supervisor.read().await;
        assert!(s.tracker.get_pid("stop-after-check").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-after-check").await.is_none(), "no server was left running");
    }

    /// 정지 대기 시간은 인스턴스 설정 `stop_timeout`(초)이 모듈 값보다 앞선다: 큰 월드는 늘리고, 여기서는 줄여 바로 끝나는지 본다 (MK14)
    #[cfg(unix)]
    #[tokio::test]
    async fn an_instance_stop_timeout_overrides_the_module_one() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n[config]\nstop_timeout = 60\n",
        )
        .unwrap();
        // stop()은 아무것도 하지 않는다: 서버는 대기 시간이 지나 강제 종료돼야 끝난다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 60'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True, 'issues': []}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "short-stop".into();
        inst.module_settings.insert("stop_timeout".into(), json!(1));
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let started = crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, "short-stop").await).body;
        assert_eq!(started["success"], true, "{started}");

        let begun = std::time::Instant::now();
        let stopped = crate::ipc::internal::InternalResponse::from(stop(&state, "short-stop", false).await).body;
        assert_eq!(stopped["success"], true, "{stopped}");
        assert!(begun.elapsed() < std::time::Duration::from_secs(15), "waited {:?}, not the instance's 1s", begun.elapsed());
    }

    /// 콘솔이 없어 저장 후 종료를 요청할 길이 없으면 모듈 `stop()`은 `graceful_stop_unavailable`로 답한다. 데몬은 그때
    /// 서버를 죽이지 않고 그 코드를 돌려준다: 강제 종료(저장 없음)는 사용자가 `force`로 고른다 (MK10)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_server_that_cannot_save_and_exit_is_not_killed_by_a_plain_stop() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, signal, sys\n\
             config = json.load(sys.stdin)\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             elif sys.argv[1] == 'stop' and not config.get('force'):\n    \
             print(json.dumps({'success': False, 'error_code': 'graceful_stop_unavailable', 'message': 'no console'}))\n\
             elif sys.argv[1] == 'stop':\n    os.kill(config['pid'], signal.SIGKILL)\n    print(json.dumps({'success': True}))\n\
             else:\n    print(json.dumps({'success': True, 'issues': []}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for id in ["no-console-managed", "no-console-native"] {
            let mut inst = crate::instance::ServerInstance::new(id, "game");
            inst.id = id.into();
            sup.instance_store.add(inst).unwrap();
        }
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let stop = |id: &'static str, force: bool| {
            let state = state.clone();
            async move { crate::ipc::internal::InternalResponse::from(stop(&state, id, force).await).body }
        };

        // managed인데 모듈이 stdin을 받지 않는다: 데몬은 모듈 stop()에 맡긴다
        let started = crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, "no-console-managed").await).body;
        assert_eq!(started["success"], true, "{started}");
        let refused = stop("no-console-managed", false).await;
        assert_eq!((refused["success"].clone(), refused["error_code"].clone()), (json!(false), json!("graceful_stop_unavailable")), "{refused}");
        let managed = state.supervisor.read().await.managed_store.get("no-console-managed").await.expect("still managed");
        assert!(managed.is_running(), "a plain stop does not kill it");
        assert_eq!(stop("no-console-managed", true).await["success"], true);

        // non-managed(데몬이 PID만 안다): 모듈 stop()의 답을 그대로 전한다
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track("no-console-native", child.id()).unwrap();
        let refused = stop("no-console-native", false).await;
        assert_eq!(refused["error_code"], "graceful_stop_unavailable", "{refused}");
        assert!(child.try_wait().unwrap().is_none(), "a plain stop does not kill it");
        assert_eq!(stop("no-console-native", true).await["success"], true);
        assert!(child.wait().is_ok());

        // 사람이 부른 정지(정지·재시작 라우트, 디스코드)는 묻지 않고 바로 강제 정지한다 (사용자 2026-11-13). 작업 기록은 정지 한 줄
        let started = crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, "no-console-managed").await).body;
        assert_eq!(started["success"], true, "{started}");
        let stopped = crate::ipc::internal::InternalResponse::from(user_stop(&state, "no-console-managed", false).await).body;
        assert_eq!(stopped["success"], true, "{stopped}");
        assert!(state.supervisor.read().await.managed_store.get("no-console-managed").await.is_none_or(|m| !m.is_running()));
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track("no-console-native", child.id()).unwrap();
        let stopped = crate::ipc::internal::InternalResponse::from(user_stop(&state, "no-console-native", false).await).body;
        assert_eq!(stopped["success"], true, "{stopped}");
        assert!(child.wait().is_ok());

        // 재시작도 같다: 저장하지 못해도 끄고 다시 켠다
        let started = crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, "no-console-managed").await).body;
        assert_eq!(started["success"], true, "{started}");
        let restarted = crate::ipc::internal::InternalResponse::from(user_restart(&state, "no-console-managed", false).await).body;
        assert_eq!(restarted["success"], true, "{restarted}");
        assert_eq!(restarted["stopped"], true, "{restarted}");
        assert_eq!(stop("no-console-managed", true).await["success"], true);
    }

    /// non-managed 정지에서 모듈 `stop()`은 이번 정지에 쓸 대기 시간(`stop_timeout`)과 그 호출의 한도(`stop_call_limit`,
    /// `stop_timeout + 30`초)를 config로 받는다. 인스턴스 값이 없으면 모듈 값이다 (06 계약 6절)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_module_stop_is_told_how_long_to_wait() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_timeout = 200\n",
        )
        .unwrap();
        let seen = tmp.path().join("seen.txt");
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, os, signal, sys\n\
                 config = json.load(sys.stdin)\n\
                 if sys.argv[1] == 'stop':\n    \
                 open({seen:?}, 'w').write('%s %s' % (config.get('stop_timeout'), config.get('stop_call_limit')))\n    \
                 os.kill(config['pid'], signal.SIGKILL)\n    print(json.dumps({{'success': True}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n",
                seen = seen.to_string_lossy()
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("native", "game");
        inst.id = "native".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track("native", child.id()).unwrap();
        let stopped = crate::ipc::internal::InternalResponse::from(stop(&state, "native", false).await).body;
        assert_eq!(stopped["success"], true, "{stopped}");
        assert!(child.wait().is_ok());
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "200 230");
    }

    /// 시작 실패의 상태는 오류 종류로 고른다. 모듈이 낸 문구에 "not found"나 "executable"이 들어 있어도
    /// 404 "인스턴스 없음"이나 422 "실행 파일 없음"이 되지 않고, 모듈이 준 본문 그대로 500이다
    #[tokio::test]
    async fn a_module_start_failure_is_not_classified_by_its_message() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\n\
             messages = {'missing-jar': 'Server jar not found: server.jar', 'missing-java': 'Java executable is missing'}\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             config = json.load(sys.stdin)\n    \
             print(json.dumps({'success': False, 'message': messages[os.path.basename(config['instance_dir'])]}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for id in ["missing-jar", "missing-java"] {
            let mut inst = crate::instance::ServerInstance::new(id, "game");
            inst.id = id.into();
            inst.module_settings.insert("managed_start".into(), json!(false));
            sup.instance_store.add(inst).unwrap();
        }
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        for (id, message) in [("missing-jar", "Server jar not found: server.jar"), ("missing-java", "Java executable is missing")] {
            let started = crate::ipc::internal::InternalResponse::from(start_unrecorded(&state, id).await);
            assert_eq!(started.status, StatusCode::INTERNAL_SERVER_ERROR, "{}", started.body);
            assert_eq!((started.body["module"].as_str(), started.body["function"].as_str()), (Some("game"), Some("get_launch_command")), "{}", started.body);
            assert_eq!(started.body["message"], message);
        }
    }

    /// 시작 한 번은 작업 기록에 `start` 한 줄이다 (비관리 시작이 안에서 또 남기지 않는다, D2)
    #[tokio::test]
    async fn an_unmanaged_start_is_one_audit_record() {
        use crate::service::backup::tests::{server_with, NO_BACKUP};
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("managed_start".into(), json!(false));
            sup.instance_store.update(&id, inst).unwrap();
        }
        // 백업이 잡고 있어 시작이 거절돼도 기록은 한 줄
        let lock = crate::instance::lock::InstanceLock::take(&id).expect("free");
        let refused = start(&state, &id).await.unwrap_err();
        drop(lock);
        assert_eq!(refused.body["error_code"], "instance_busy", "{}", refused.body);
        let entries = crate::audit::log().recent(10, Some(&id));
        let starts = entries.iter().filter(|e| e["action"] == "start").count();
        assert_eq!(starts, 1, "{entries:?}");
    }

    /// 관리형 시작도 모듈 `[install] requires_extensions`가 꺼져 있으면 `extension_required`로 거절한다 (D2)
    #[tokio::test]
    async fn a_managed_start_needs_the_module_required_extensions() {
        use crate::service::backup::tests::{server_with, NO_BACKUP};
        let toml = format!("{NO_BACKUP}\n[install]\nrequires_extensions = [\"needed-for-test\"]\n");
        let (state, id, _tmp) = server_with(&toml, None).await;
        assert!(state.supervisor.read().await.uses_managed_start(state.supervisor.read().await.instance_store.get(&id).unwrap()));
        let refused = start(&state, &id).await;
        let body = match refused {
            Ok(body) => body,
            Err(failure) => failure.body,
        };
        assert_eq!(body["error_code"], "extension_required", "{body}");
        assert_eq!(body["missing_extensions"], json!(["needed-for-test"]), "{body}");
        assert_eq!(body["managed"], true, "{body}");
    }
}
