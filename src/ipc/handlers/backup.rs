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

/// 백업·복원 중인 인스턴스 (같은 인스턴스에서 둘이 겹치지 않게)
static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);

struct BusyGuard(String);

impl BusyGuard {
    fn take(id: &str) -> Option<Self> {
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

async fn plan(state: &IPCServer, id: &str) -> Result<Plan, Response> {
    let (spec, base, dir, keep, hook_call, lookup) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(id).cloned() else {
            return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
        };
        let module = supervisor.module_loader.get_module(&instance.module_name).ok();
        let Some(spec) = module.as_ref().and_then(|m| m.metadata.backup.clone()) else {
            return Err(error(StatusCode::BAD_REQUEST, "backup_unsupported", "This module does not declare [backup] paths"));
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
            Ok(v) => tracing::warn!("backup_paths for '{}' failed: {}", id, v.get("message").unwrap_or(&v)),
            Err(e) => tracing::warn!("backup_paths for '{}' failed: {}", id, e),
        }
    }
    let paths = backup::resolve_paths(&patterns, &base, lookup);
    Ok(Plan { dir, paths, save_command: spec.save_command, keep })
}

/// 백업 하나를 만들고 오래된 자동 백업을 정리한다. 백업할 것이 없으면 Ok(None)
pub async fn take_backup(state: &IPCServer, id: &str, kind: BackupKind) -> Result<Option<BackupInfo>, Response> {
    let Some(_guard) = BusyGuard::take(id) else {
        return Err(error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance"));
    };
    take_backup_locked(state, id, kind).await
}

async fn take_backup_locked(state: &IPCServer, id: &str, kind: BackupKind) -> Result<Option<BackupInfo>, Response> {
    let plan = plan(state, id).await?;
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
        let pruned = backup::prune(&dir, keep);
        anyhow::Ok((made, pruned))
    })
    .await;
    match result {
        Ok(Ok((made, pruned))) => {
            tracing::info!("Backed up '{}' as {} ({} bytes, pruned {})", id, made.name, made.size, pruned.len());
            Ok(Some(made))
        }
        Ok(Err(e)) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", format!("{:#}", e))),
        Err(e) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e.to_string())),
    }
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
    if is_running(&state, &id).await {
        return error(StatusCode::CONFLICT, "server_running", "Stop the server before restoring a backup");
    }
    let Some(_guard) = BusyGuard::take(&id) else {
        return error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance");
    };
    // 지금 상태를 먼저 남긴다: 잘못 고른 백업도 되돌릴 수 있게
    let safety = match take_backup_locked(&state, &id, BackupKind::PreRestore).await {
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
                    run_task(&state, &id, &task, fire).await;
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

/// 실행 시각까지 `warn_minutes`마다 게임 안에 예고한다
async fn count_down(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) {
    let language = state.config_store.get_gui_settings().await.language.unwrap_or_else(|| "en".into());
    let catalog = crate::chat::i18n::Catalog::for_language(&language);
    let mut marks = task.warn_minutes.clone();
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
        announce(state, id, &catalog.t(key, &[("minutes", &minutes.to_string())])).await;
    }
    let wait = (fire - chrono::Local::now().naive_local()).to_std().unwrap_or_default();
    tokio::time::sleep(wait).await;
}

async fn run_task(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime) {
    let running = is_running(state, id).await;
    match task.action {
        TaskAction::Backup if running => {
            if let Err(r) = take_backup(state, id, BackupKind::Scheduled).await {
                let body = crate::ipc::internal::into_internal(r).await.body;
                tracing::warn!("Scheduled backup of '{}' failed: {}", id, body["error"]);
            }
        }
        TaskAction::Announce if running => {
            announce(state, id, task.message.as_deref().unwrap_or_default()).await;
        }
        TaskAction::Restart if running => {
            count_down(state, id, task, fire, "schedule.restart_warning").await;
            if !is_running(state, id).await {
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
        TaskAction::Update => run_update(state, id, task, fire).await,
        _ => {}
    }
}

/// 업데이트가 있으면: (켜져 있으면) 예고 → 정지 → 업데이트(업데이트 전 백업 포함) → 다시 시작
async fn run_update(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime) {
    let check = crate::ipc::internal::into_internal(
        super::instance::check_server_update(Path(id.to_string()), State(state.clone())).await,
    )
    .await;
    if check.body.get("update_available").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let was_running = is_running(state, id).await;
    if was_running {
        count_down(state, id, task, fire, "schedule.update_warning").await;
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
