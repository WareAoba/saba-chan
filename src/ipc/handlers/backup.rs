//! 백업 API (A2)와 예약 작업 API·실행기 (A3)
//!
//! - `GET/POST /api/instance/:id/backups` — 목록 / 지금 백업
//! - `POST /api/instance/:id/backups/:name/restore` — 복원 (서버가 꺼져 있어야 한다. 먼저 pre-restore 백업)
//! - `DELETE /api/instance/:id/backups/:name`
//! - `GET/PUT /api/instance/:id/schedule` — `{schedule: [...], backup_keep}`
#![allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Mutex;

use super::super::{CommandRequest, IPCServer};
use crate::backup::{self, BackupInfo, BackupKind};
use crate::scheduler::{Clock, ScheduledTask, TaskAction};

const DEFAULT_KEEP: usize = 10;
/// 원격에서 넣는 반복 작업의 가장 짧은 간격(분)과 보관 개수 상한
const REMOTE_MIN_EVERY_MINUTES: u32 = 30;
const REMOTE_MAX_BACKUP_KEEP: u32 = 30;

/// 파일을 다루는 작업(백업·복원·업데이트·시작·삭제) 중인 인스턴스. 같은 인스턴스에서 둘이 겹치지 않게
static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// 인스턴스 작업 잠금. 놓아질 때 풀린다
pub(crate) struct BusyGuard(String);

impl BusyGuard {
    pub(crate) fn take(id: &str) -> Option<Self> {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        busy.get_or_insert_with(HashSet::new).insert(id.to_string()).then(|| Self(id.to_string()))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Some(set) = BUSY.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            set.remove(&self.0);
        }
    }
}

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "success": false, "error": message.into(), "error_code": code }))).into_response()
}

async fn is_running(state: &IPCServer, id: &str) -> bool {
    let supervisor = state.supervisor.read().await;
    supervisor.tracker.get_pid(id).is_ok() || supervisor.managed_store.running_instance_ids().await.contains(&id.to_string())
}

/// 이름 하나로 게임 명령 실행 (저장·공지). 성공 여부만
async fn run_command(state: &IPCServer, id: &str, name: &str, args: Value) -> bool {
    let req = CommandRequest { command: name.to_string(), args };
    let resp = crate::ipc::internal::into_internal(
        super::command::execute_command(Path(id.to_string()), State(state.clone()), Json(req)).await,
    )
    .await;
    resp.status.is_success() && resp.body.get("success").and_then(Value::as_bool) != Some(false)
}

/// 이 인스턴스의 백업 폴더와 백업할 경로
struct Plan {
    dir: PathBuf,
    paths: Vec<PathBuf>,
    save_command: Option<String>,
    keep: usize,
}

/// 모듈이 `[backup]`을 선언하지 않았으면 Ok(None)
async fn plan(state: &IPCServer, id: &str) -> Result<Option<Plan>, Response> {
    let (spec, base, dir, keep, hook_call, lookup) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(id).cloned() else {
            return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
        };
        let module = supervisor.module_loader.get_module(&instance.module_name).ok();
        let Some(spec) = module.as_ref().and_then(|m| m.metadata.backup.clone()) else {
            return Ok(None);
        };
        let base = instance
            .working_dir
            .clone()
            .or_else(|| instance.executable_path.as_ref().and_then(|p| FsPath::new(p).parent()).map(|p| p.to_string_lossy().to_string()))
            .map(PathBuf::from)
            .unwrap_or_else(|| supervisor.instance_store.instance_dir(id));
        let hook_call = module.as_ref().filter(|_| spec.hook).map(|m| {
            let mut cfg = serde_json::Map::new();
            cfg.insert("working_dir".into(), json!(base.to_string_lossy()));
            cfg.insert("extension_data".into(), json!(&instance.extension_data));
            crate::supervisor::Supervisor::insert_instance_fields(&mut cfg, &instance);
            crate::plugin::PluginCall::new(m.entry_path(), "backup_paths", Value::Object(cfg))
        });
        // `{설정}` 자리: 인스턴스 설정 값, 없으면 module.toml 기본값
        let defaults = module.as_ref().map(|m| m.metadata.clone());
        let settings = instance.module_settings.clone();
        let lookup = move |key: &str| -> Option<String> {
            let text = |v: &Value| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            settings.get(key).map(text).or_else(|| defaults.as_ref()?.get_setting_default::<String>(key))
        };
        let keep = instance.backup_keep.map(|k| k as usize).unwrap_or(DEFAULT_KEEP);
        (spec, base, backup::backups_dir(&supervisor.instance_store.instance_dir(id)), keep, hook_call, lookup)
    };
    let mut patterns = spec.paths.clone();
    if let Some(call) = hook_call {
        match call.run().await {
            Ok(v) if v.get("success").and_then(Value::as_bool) == Some(true) => {
                patterns.extend(v["paths"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)));
            }
            // 경로를 모르면 안전 백업 없이 업데이트·복원이 진행되니 여기서 멈춘다
            Ok(v) => {
                let reason = v.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| v.to_string());
                return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", format!("backup_paths failed: {}", reason)));
            }
            Err(e) => return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", format!("backup_paths failed: {:#}", e))),
        }
    }
    let paths = backup::resolve_paths(&patterns, &base, lookup);
    Ok(Some(Plan { dir, paths, save_command: spec.save_command, keep }))
}

/// 백업 하나를 만들고 오래된 자동 백업을 정리한다. 백업할 것이 없으면 Ok(None).
/// 모듈이 백업을 지원하지 않으면 수동 백업만 오류이고, 자동 백업(업데이트 전·예약)은 건너뛴다
pub async fn take_backup(state: &IPCServer, id: &str, kind: BackupKind) -> Result<Option<BackupInfo>, Response> {
    let Some(_guard) = BusyGuard::take(id) else {
        return Err(error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance"));
    };
    take_backup_locked(state, id, kind, None).await
}

/// 이미 작업 잠금을 잡은 호출자(업데이트)용 `take_backup`
pub(crate) async fn take_backup_held(state: &IPCServer, id: &str, kind: BackupKind, _guard: &BusyGuard) -> Result<Option<BackupInfo>, Response> {
    take_backup_locked(state, id, kind, None).await
}

/// 다른 작업이 잡고 있을 때의 응답 (시작·업데이트·삭제)
pub(crate) fn instance_busy() -> Response {
    error(StatusCode::CONFLICT, "instance_busy", "A backup, restore or update is running for this instance")
}

/// `protect`: 정리에서 빼 둘 백업 이름 (복원 중인 백업)
async fn take_backup_locked(state: &IPCServer, id: &str, kind: BackupKind, protect: Option<String>) -> Result<Option<BackupInfo>, Response> {
    let Some(plan) = plan(state, id).await? else {
        if kind == BackupKind::Manual {
            return Err(error(StatusCode::BAD_REQUEST, "backup_unsupported", "This module does not declare [backup] paths"));
        }
        return Ok(None);
    };
    if plan.paths.is_empty() {
        return Ok(None);
    }
    if let Some(cmd) = plan.save_command.as_deref() {
        if is_running(state, id).await {
            if !run_command(state, id, cmd, json!({})).await {
                tracing::warn!("Save command '{}' failed before backing up '{}'", cmd, id);
            }
            // 저장 명령은 대개 응답 뒤에도 디스크에 쓴다
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
    let Plan { dir, paths, keep, .. } = plan;
    let result = tokio::task::spawn_blocking(move || {
        let made = backup::create(&dir, kind, &paths)?;
        // 방금 만든 백업은 보관 개수와 상관없이 남긴다
        let mut keep_names = vec![made.name.as_str()];
        keep_names.extend(protect.as_deref());
        let pruned = backup::prune(&dir, keep, &keep_names);
        anyhow::Ok((made, pruned))
    })
    .await;
    let result = match result {
        Ok(Ok((made, pruned))) => {
            tracing::info!("Backed up '{}' as {} ({} bytes, pruned {})", id, made.name, made.size, pruned.len());
            Ok(made)
        }
        Ok(Err(e)) => Err(format!("{:#}", e)),
        Err(e) => Err(e.to_string()),
    };
    let instance = state.supervisor.read().await.instance_store.get(id).map(|i| i.name.clone()).unwrap_or_default();
    crate::events::emit("backup.done", match &result {
        Ok(made) => json!({ "instanceId": id, "instance": instance, "kind": kind, "name": made.name, "ok": true }),
        Err(e) => json!({ "instanceId": id, "instance": instance, "kind": kind, "ok": false, "error": e }),
    });
    result.map(Some).map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e))
}

async fn backups_dir(state: &IPCServer, id: &str) -> Result<PathBuf, Response> {
    let supervisor = state.supervisor.read().await;
    if supervisor.instance_store.get(id).is_none() {
        return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
    }
    Ok(backup::backups_dir(&supervisor.instance_store.instance_dir(id)))
}

pub async fn list_backups(Path(id): Path<String>, State(state): State<IPCServer>) -> Response {
    match backups_dir(&state, &id).await {
        Ok(dir) => Json(json!({ "success": true, "backups": backup::list(&dir) })).into_response(),
        Err(r) => r,
    }
}

pub async fn create_backup(Path(id): Path<String>, State(state): State<IPCServer>) -> Response {
    match take_backup(&state, &id, BackupKind::Manual).await {
        Ok(Some(made)) => Json(json!({ "success": true, "backup": made })).into_response(),
        Ok(None) => error(StatusCode::BAD_REQUEST, "nothing_to_back_up", "None of the module's backup paths exist yet"),
        Err(r) => r,
    }
}

pub async fn delete_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>) -> Response {
    let dir = match backups_dir(&state, &id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Some(_guard) = BusyGuard::take(&id) else {
        return error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance");
    };
    match backup::delete(&dir, &name) {
        Ok(()) => Json(json!({ "success": true })).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, "backup_not_found", e.to_string()),
    }
}

pub async fn restore_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>) -> Response {
    let dir = match backups_dir(&state, &id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    // 잠금을 먼저 잡고 실행 여부를 본다: 시작도 같은 잠금을 잡으니 확인 뒤에 켜지지 않는다
    let Some(_guard) = BusyGuard::take(&id) else {
        return error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance");
    };
    if is_running(&state, &id).await {
        return error(StatusCode::CONFLICT, "server_running", "Stop the server before restoring a backup");
    }
    // 지금 상태를 먼저 남긴다: 잘못 고른 백업도 되돌릴 수 있게
    // 복원할 백업은 이 안전 백업의 정리 대상에서 뺀다
    let safety = match take_backup_locked(&state, &id, BackupKind::PreRestore, Some(name.clone())).await {
        Ok(made) => made.map(|b| b.name),
        Err(r) => return r,
    };
    match tokio::task::spawn_blocking(move || backup::restore(&dir, &name)).await {
        Ok(Ok(paths)) => Json(json!({ "success": true, "restored": paths, "pre_restore_backup": safety })).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, "restore_failed", format!("{:#}", e)),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, "restore_failed", e.to_string()),
    }
}

// ── 예약 작업 ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ScheduleBody {
    schedule: Vec<ScheduledTask>,
    #[serde(default)]
    backup_keep: Option<u32>,
}

pub async fn get_schedule(Path(id): Path<String>, State(state): State<IPCServer>) -> Response {
    let supervisor = state.supervisor.read().await;
    let Some(instance) = supervisor.instance_store.get(&id) else {
        return error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id));
    };
    let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
    Json(json!({
        "success": true,
        "schedule": instance.schedule,
        "backup_keep": instance.backup_keep.unwrap_or(DEFAULT_KEEP as u32),
        "backup_supported": meta.as_ref().is_some_and(|m| m.backup.is_some()),
        "announce_supported": meta.as_ref().is_some_and(|m| m.announce_command.is_some()),
    }))
    .into_response()
}

pub async fn put_schedule(Path(id): Path<String>, State(state): State<IPCServer>, Json(body): Json<ScheduleBody>) -> Response {
    let mut supervisor = state.supervisor.write().await;
    let Some(mut instance) = supervisor.instance_store.get(&id).cloned() else {
        return error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id));
    };
    let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
    let can_backup = meta.as_ref().is_some_and(|m| m.backup.is_some());
    let can_announce = meta.as_ref().is_some_and(|m| m.announce_command.is_some());
    let mut problems = Vec::new();
    for (i, task) in body.schedule.iter().enumerate() {
        let mut p = task.problems();
        if task.action == TaskAction::Backup && !can_backup {
            p.push("this module does not support backups".into());
        }
        if (task.action == TaskAction::Announce || !task.warn_minutes.is_empty()) && !can_announce {
            p.push("this module has no announce command".into());
        }
        problems.extend(p.into_iter().map(|m| format!("schedule[{}]: {}", i, m)));
    }
    if body.backup_keep == Some(0) {
        problems.push("backup_keep must be at least 1".into());
    }
    // 원격(디스코드·챗봇)은 새로 넣거나 바꾸는 작업만 제한한다 (이 PC에서 정한 작업은 그대로 다시 보낼 수 있다)
    if crate::audit::current().source == "discord" {
        for (i, task) in body.schedule.iter().enumerate() {
            if instance.schedule.contains(task) {
                continue;
            }
            // 끈 업데이트 작업은 돌지 않으므로 끄는 것은 원격에서도 된다
            if task.action == TaskAction::Update && task.enabled {
                problems.push(format!("schedule[{}]: updates can only be scheduled on this PC", i));
            }
            if task.every_minutes.is_some_and(|m| m < REMOTE_MIN_EVERY_MINUTES) {
                problems.push(format!("schedule[{}]: every_minutes must be at least {} from Discord", i, REMOTE_MIN_EVERY_MINUTES));
            }
        }
        if body.backup_keep.is_some_and(|k| k > REMOTE_MAX_BACKUP_KEEP && Some(k) != instance.backup_keep) {
            problems.push(format!("backup_keep must be at most {} from Discord", REMOTE_MAX_BACKUP_KEEP));
        }
    }
    if !problems.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": "validation_failed", "error_code": "validation_failed", "details": problems })),
        )
            .into_response();
    }
    instance.schedule = body.schedule;
    if body.backup_keep.is_some() {
        instance.backup_keep = body.backup_keep;
    }
    match supervisor.instance_store.update(&id, instance) {
        Ok(()) => Json(json!({ "success": true })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", e.to_string()),
    }
}

// ── 실행기 ────────────────────────────────────────────────

/// 데몬이 끝날 때까지 30초마다 예약 작업을 확인한다
pub fn spawn_scheduler(state: IPCServer) {
    tokio::spawn(async move {
        let started = chrono::Local::now().naive_local();
        let clock = std::sync::Arc::new(Mutex::new(Clock::default()));
        loop {
            tokio::select! {
                _ = state.shutdown_token.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            }
            let now = chrono::Local::now().naive_local();
            let tasks: Vec<(String, ScheduledTask)> = {
                let supervisor = state.supervisor.read().await;
                supervisor
                    .instance_store
                    .list()
                    .iter()
                    .flat_map(|i| i.schedule.iter().map(move |t| (i.id.clone(), t.clone())))
                    .collect()
            };
            let mut due = Vec::new();
            {
                let mut c = clock.lock().unwrap_or_else(|e| e.into_inner());
                let keys: HashSet<String> = tasks.iter().map(|(id, t)| crate::scheduler::task_key(id, t)).collect();
                c.retain(&keys);
                for (id, task) in tasks {
                    let key = crate::scheduler::task_key(&id, &task);
                    if let Some(fire) = c.due(&key, &task, started, now) {
                        due.push((key, id, task, fire));
                    }
                }
            }
            for (key, id, task, fire) in due {
                let state = state.clone();
                let clock = clock.clone();
                tokio::spawn(async move {
                    run_task(&state, &id, &task, fire, &key).await;
                    clock.lock().unwrap_or_else(|e| e.into_inner()).finish(&key, fire);
                });
            }
        }
    });
}

async fn announce(state: &IPCServer, id: &str, message: &str) -> bool {
    let cmd = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .get(id)
            .and_then(|i| supervisor.module_loader.get_module(&i.module_name).ok())
            .and_then(|m| m.metadata.announce_command)
    };
    match cmd {
        Some(cmd) => run_command(state, id, &cmd, json!({ "message": message })).await,
        None => false,
    }
}

/// 예고하는 사이에 사용자가 그 예약을 지우거나 껐으면 false
async fn still_scheduled(state: &IPCServer, id: &str, key: &str) -> bool {
    let supervisor = state.supervisor.read().await;
    supervisor
        .instance_store
        .get(id)
        .is_some_and(|i| crate::scheduler::still_scheduled(id, &i.schedule, key))
}

/// 실행 시각까지 `warn_minutes`마다 게임 안에 예고한다 (`announce`가 false면 기다리기만).
/// 그 사이 예약이 지워지거나 꺼지면 false
async fn count_down(
    state: &IPCServer,
    id: &str,
    task: &ScheduledTask,
    fire: chrono::NaiveDateTime,
    task_key: &str,
    message_key: &str,
    announce_on: bool,
) -> bool {
    let language = state.config_store.ui_language().await;
    let catalog = crate::chat::i18n::Catalog::for_language(&language);
    let mut marks = if announce_on { task.warn_minutes.clone() } else { Vec::new() };
    marks.sort_unstable_by(|a, b| b.cmp(a));
    marks.dedup();
    for minutes in marks {
        let at = fire - chrono::Duration::minutes(minutes as i64);
        let wait = (at - chrono::Local::now().naive_local()).to_std().unwrap_or_default();
        tokio::time::sleep(wait).await;
        // 예고 시각이 이미 지났으면 (데몬이 그 사이 멈췄다 등) 그 예고는 건너뛴다
        if chrono::Local::now().naive_local() - at > chrono::Duration::minutes(1) {
            continue;
        }
        if !still_scheduled(state, id, task_key).await {
            return false;
        }
        let name = state.supervisor.read().await.instance_store.get(id).map(|i| i.name.clone()).unwrap_or_default();
        crate::events::emit(
            "schedule.warn",
            json!({ "instanceId": id, "instance": name, "action": task.action, "minutesLeft": minutes }),
        );
        announce(state, id, &catalog.t(message_key, &[("minutes", &minutes.to_string())])).await;
    }
    let wait = (fire - chrono::Local::now().naive_local()).to_std().unwrap_or_default();
    tokio::time::sleep(wait).await;
    let alive = still_scheduled(state, id, task_key).await;
    if !alive {
        tracing::info!("Scheduled task for '{}' was removed or disabled during its countdown; skipping", id);
    }
    alive
}

async fn run_task(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) {
    // 감사 로그 (A10): 예약 작업이 한 일은 출처 schedule
    crate::audit::with_origin(crate::audit::Origin::new("schedule"), run_task_now(state, id, task, fire, key)).await
}

async fn run_task_now(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) {
    let running = is_running(state, id).await;
    match task.action {
        TaskAction::Backup => {
            // 꺼져 있으면 바뀐 게 없으니 건너뛰되, 건너뛴 것도 감사 로그에 남긴다
            let status = if !running {
                tracing::info!("Scheduled backup of '{}' skipped: the server is not running", id);
                409
            } else {
                match take_backup(state, id, BackupKind::Scheduled).await {
                    Ok(Some(_)) => 200,
                    Ok(None) => 204,
                    Err(r) => {
                        let resp = crate::ipc::internal::into_internal(r).await;
                        tracing::warn!("Scheduled backup of '{}' failed: {}", id, resp.body["error"]);
                        resp.status.as_u16()
                    }
                }
            };
            crate::audit::record(crate::audit::Event {
                action: "backup",
                instance_id: id,
                ok: status < 300,
                status,
                ..Default::default()
            });
        }
        TaskAction::Announce if running => {
            announce(state, id, task.message.as_deref().unwrap_or_default()).await;
        }
        TaskAction::Restart if running => {
            if !count_down(state, id, task, fire, key, "schedule.restart_warning", true).await || !is_running(state, id).await {
                return;
            }
            tracing::info!("Scheduled restart of '{}'", id);
            let resp = crate::ipc::internal::into_internal(
                super::server::restart_instance_handler(Path(id.to_string()), State(state.clone()), None).await,
            )
            .await;
            if !resp.status.is_success() {
                tracing::warn!("Scheduled restart of '{}' failed: {}", id, resp.body);
            }
        }
        TaskAction::Update => run_update(state, id, task, fire, key).await,
        _ => {}
    }
}

/// 업데이트가 있으면: (켜져 있으면) 예고 → 정지 → 업데이트(업데이트 전 백업 포함) → 다시 시작.
/// 꺼져 있어도 정한 시각까지는 기다린다 (예고 시각에 바로 시작하지 않게)
async fn run_update(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) {
    let check = crate::ipc::internal::into_internal(
        super::instance::check_server_update(Path(id.to_string()), State(state.clone())).await,
    )
    .await;
    if check.body.get("update_available").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let running_now = is_running(state, id).await;
    if !count_down(state, id, task, fire, key, "schedule.update_warning", running_now).await {
        return;
    }
    let was_running = is_running(state, id).await;
    if was_running {
        let stop = crate::ipc::internal::into_internal(super::server::stop_instance(state.clone(), id, false).await).await;
        if !stop.status.is_success() {
            tracing::warn!("Scheduled update of '{}': stop failed: {}", id, stop.body);
            return;
        }
        for _ in 0..240 {
            if !is_running(state, id).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    tracing::info!("Scheduled update of '{}'", id);
    let updated = match super::instance::begin_server_update(state, id).await {
        Ok(job) => job.await.unwrap_or(false),
        Err(r) => {
            tracing::warn!("Scheduled update of '{}' failed: {}", id, crate::ipc::internal::into_internal(r).await.body);
            false
        }
    };
    if was_running {
        if !updated {
            tracing::warn!("Scheduled update of '{}' failed; starting the old version again", id);
        }
        let _ = super::server::start_instance_handler(Path(id.to_string()), State(state.clone())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    const NO_BACKUP: &str = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";

    async fn server_with(module_toml: &str, hook: Option<&str>) -> (IPCServer, String, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("plain");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), module_toml).unwrap();
        std::fs::write(dir.join("lifecycle.py"), hook.unwrap_or("")).unwrap();
        let instances = tmp.path().join("instances");
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), instances.to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "plain");
        inst.working_dir = Some(tmp.path().join("server").to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        (state, id, tmp)
    }

    async fn code(r: Response) -> String {
        crate::ipc::internal::into_internal(r).await.body["error_code"].as_str().unwrap_or_default().to_string()
    }

    #[tokio::test]
    async fn modules_without_backup_skip_automatic_backups_but_reject_manual_ones() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        assert!(matches!(take_backup(&state, &id, BackupKind::PreUpdate).await, Ok(None)), "an update must not be blocked");
        assert!(matches!(take_backup(&state, &id, BackupKind::Scheduled).await, Ok(None)));
        let err = take_backup(&state, &id, BackupKind::Manual).await.err().expect("manual backup of an unsupported module");
        assert_eq!(code(err).await, "backup_unsupported");
    }

    #[tokio::test]
    async fn a_scheduled_backup_of_a_stopped_server_is_logged_as_skipped() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let task: ScheduledTask = serde_json::from_value(json!({"action": "backup", "every_minutes": 60})).unwrap();
        run_task(&state, &id, &task, chrono::Local::now().naive_local(), "k").await;
        let entries = crate::audit::log().recent(10, Some(&id));
        let entry = entries.first().expect("the skipped backup must leave a record");
        assert_eq!(entry["action"], "backup");
        assert_eq!(entry["source"], "schedule");
        assert_eq!(entry["ok"], false);
        assert_eq!(entry["status"], 409);
    }

    #[tokio::test]
    async fn a_failing_backup_paths_hook_stops_the_backup() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = []\nhook = true\n");
        let hook = "def backup_paths(config):\n    return {\"success\": False, \"message\": \"no world\"}\n";
        let (state, id, _tmp) = server_with(&toml, Some(hook)).await;
        let err = take_backup(&state, &id, BackupKind::PreUpdate).await.err().expect("hook failure must not mean 'nothing to back up'");
        assert_eq!(code(err).await, "backup_failed");
    }

    #[tokio::test]
    async fn start_update_and_delete_wait_for_a_running_restore() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let guard = BusyGuard::take(&id).expect("free");
        let start = super::super::server::start_instance_handler(Path(id.clone()), State(state.clone())).await;
        assert_eq!(code(start).await, "instance_busy");
        let managed = super::super::managed::start_managed_handler(Path(id.clone()), State(state.clone()), None).await.into_response();
        assert_eq!(code(managed).await, "instance_busy");
        let update = super::super::instance::begin_server_update(&state, &id).await.err().expect("update must wait");
        assert_eq!(code(update).await, "instance_busy");
        let removed = super::super::instance::delete_instance(Path(id.clone()), State(state.clone())).await.into_response();
        assert_eq!(code(removed).await, "instance_busy");
        assert!(state.supervisor.read().await.instance_store.get(&id).is_some());
        drop(guard);
        assert!(BusyGuard::take(&id).is_some(), "refused calls must not keep the lock");
    }

    #[tokio::test]
    async fn schedules_from_discord_cannot_add_updates_or_run_too_often() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let put = |body: Value| {
            let state = state.clone();
            let id = id.clone();
            async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                put_schedule(Path(id), State(state), Json(body)).await.status()
            }
        };
        let remote = |body: Value| crate::audit::with_origin(crate::audit::Origin::new("discord"), put(body));
        let update = json!({ "action": "update", "at": "04:00" });
        let restart = |m: u32| json!({ "action": "restart", "every_minutes": m });

        assert_eq!(remote(json!({ "schedule": [update] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [restart(1)] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [restart(60)], "backup_keep": 1000 })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [restart(60)], "backup_keep": 5 })).await, StatusCode::OK);
        // 0개 보관이면 방금 만든 안전 백업까지 지워진다
        assert_eq!(put(json!({ "schedule": [], "backup_keep": 0 })).await, StatusCode::BAD_REQUEST);
        // 이 PC에서 정한 작업은 디스코드에서 그대로 다시 보내도 된다
        assert_eq!(put(json!({ "schedule": [update, restart(1)], "backup_keep": 100 })).await, StatusCode::OK);
        assert_eq!(remote(json!({ "schedule": [update, restart(1), restart(60)], "backup_keep": 100 })).await, StatusCode::OK);
        // 끄는 것은 된다
        let off = json!({ "action": "update", "at": "04:00", "enabled": false });
        assert_eq!(remote(json!({ "schedule": [off, restart(1)], "backup_keep": 100 })).await, StatusCode::OK);
    }
}
