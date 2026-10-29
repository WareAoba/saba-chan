//! 백업 API (A2)와 예약 작업 API·실행기 (A3)
//!
//! - `GET/POST /api/instance/:id/backups` — 목록 / 지금 백업
//! - `POST /api/instance/:id/backups/:name/restore` — 복원 (서버가 꺼져 있어야 한다. 먼저 pre-restore 백업)
//! - `DELETE /api/instance/:id/backups/:name`
//! - `GET/PUT /api/instance/:id/schedule` — `{schedule: [...], backup_keep}`
#![allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답

use super::LooseJson;
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

pub(crate) const DEFAULT_KEEP: usize = 10;
/// 원격에서 넣는 반복 작업의 가장 짧은 간격(분)과 보관 개수 상한
const REMOTE_MIN_EVERY_MINUTES: u32 = 30;
const REMOTE_MAX_BACKUP_KEEP: u32 = 30;
/// 원격(디스코드·챗봇)이 만드는 백업의 종류별 상한. 원격 백업은 정리하지 않으므로(D-B11) 끝없이 쌓이지 않게 막는다 (D-B7)
const REMOTE_BACKUP_CAP: usize = 50;
/// 재시작이 작업 잠금이 풀리기를 기다리는 가장 긴 시간 (큰 월드의 백업·업데이트)
const START_WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

pub(crate) use crate::instance::lock::InstanceLock;

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "success": false, "error": message.into(), "error_code": code }))).into_response()
}

pub(crate) async fn is_running(state: &IPCServer, id: &str) -> bool {
    state.supervisor.read().await.is_running(id).await
}

/// 이름 하나로 게임 명령 실행 (저장·공지). 성공 여부만
async fn run_command(state: &IPCServer, id: &str, name: &str, args: Value) -> bool {
    command_reply(state, id, name, args).await.is_some()
}

/// 이름 하나로 게임 명령 실행. 성공하면 서버의 콘솔 답(없으면 빈 글자)
async fn command_reply(state: &IPCServer, id: &str, name: &str, args: Value) -> Option<String> {
    let req = CommandRequest { command: name.to_string(), args };
    let resp = crate::ipc::internal::into_internal(
        super::command::execute_command(Path(id.to_string()), State(state.clone()), Json(req)).await,
    )
    .await;
    let ok = resp.status.is_success() && resp.body.get("success").and_then(Value::as_bool) != Some(false);
    ok.then(|| resp.body.pointer("/data/response").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// 켜진 서버에서 저장 완료 줄(`[backup] save_done_pattern`)을 기다리는 가장 긴 시간 (큰 월드의 save-all flush)
#[cfg(not(test))]
const SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(120);
#[cfg(test)]
const SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// 켜진 서버의 저장을 믿을 수 없어 백업을 만들지 않았다: 서버가 명령을 거절했거나(`backup_save_rejected`, 모듈의 실패 모양)
/// 저장 완료 줄이 `SAVE_WAIT` 안에 나오지 않았다(`backup_save_unconfirmed`)
#[derive(Debug)]
enum SaveNotDone {
    Rejected { command: String, reason: String },
    Unconfirmed { command: String },
}

impl std::fmt::Display for SaveNotDone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveNotDone::Rejected { command, reason } => write!(f, "the server rejected '{}' ({}); no backup was made, stop the server and back up again", command, reason),
            SaveNotDone::Unconfirmed { command } => write!(f, "the server did not report that '{}' finished; no backup was made", command),
        }
    }
}

impl std::error::Error for SaveNotDone {}

/// 서버 답이 모듈의 실패 모양(`[commands] failure_patterns`)에 맞으면 그 까닭. 실패 모양의 `reason` 이름 묶음만 싣고,
/// 없으면 "rejected" (콘솔 원문은 밖으로 내지 않는다)
fn rejected(reply: &str, failures: &[regex::Regex]) -> Option<String> {
    failures
        .iter()
        .find_map(|re| re.captures(reply))
        .map(|c| c.name("reason").map(|m| m.as_str().trim().to_string()).unwrap_or_else(|| "rejected".to_string()))
}

/// 켜진 서버의 자동 저장을 멈추고 저장한다. 서버가 명령을 거절하면(명령 답이 실패 모양) 백업을 만들지 않는다.
/// 저장 완료 줄이 선언돼 있으면 저장 명령을 보낸 뒤의 콘솔에 그 줄이 나올 때까지 기다리고, 나오지 않으면 백업을 만들지 않는다.
/// 선언이 없으면 예전처럼 3초 기다린다
async fn save_for_backup(state: &IPCServer, id: &str, spec: &crate::supervisor::module_loader::ModuleBackup, failures: &[String]) -> Result<(), SaveNotDone> {
    let proc = state.supervisor.read().await.managed_store.get(id).await;
    let failures: Vec<regex::Regex> = failures.iter().filter_map(|p| regex::Regex::new(p).ok()).collect();
    let done = spec.save_done_pattern.as_deref().and_then(|p| match regex::Regex::new(p) {
        Ok(re) => Some(re),
        Err(e) => {
            tracing::warn!("backup.save_done_pattern of '{}' is not a valid regex ({}); waiting a fixed time instead", id, e);
            None
        }
    });
    let mut confirmed = false;
    for cmd in [spec.pause_command.as_deref(), spec.save_command.as_deref()].into_iter().flatten() {
        let is_save = Some(cmd) == spec.save_command.as_deref();
        let cursor = match &proc {
            Some(p) => Some(p.console_cursor().await),
            None => None,
        };
        let Some(reply) = command_reply(state, id, cmd, json!({})).await else {
            tracing::warn!("Command '{}' failed before backing up '{}'", cmd, id);
            // 완료 줄로 확인하기로 한 저장을 보내지도 못했다: 저장 안 된 월드를 묶지 않는다
            if is_save && done.is_some() {
                return Err(SaveNotDone::Unconfirmed { command: cmd.to_string() });
            }
            continue;
        };
        let done_here = done.as_ref().filter(|_| is_save);
        if done_here.is_some_and(|d| reply.lines().any(|l| d.is_match(l))) {
            confirmed = true;
            continue;
        }
        // 답에는 보낸 뒤 잠깐 찍힌 줄이 다 섞인다(다른 명령의 답 포함). 서버가 바로 찍는 첫 줄로만 거절을 가린다
        let first = reply.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
        if let Some(reason) = rejected(first, &failures) {
            return Err(SaveNotDone::Rejected { command: cmd.to_string(), reason });
        }
        let Some(done) = done_here else { continue };
        let (Some(p), Some(mut next)) = (&proc, cursor) else {
            // 콘솔을 볼 수 없는 서버(RCON 등): 답에 없으면 확인할 길이 없다
            return Err(SaveNotDone::Unconfirmed { command: cmd.to_string() });
        };
        // 거절은 명령 답(위)으로만 가린다. 기다리는 동안 다른 명령·플러그인이 찍는 줄로 저장을 버리지 않는다
        let deadline = tokio::time::Instant::now() + SAVE_WAIT;
        loop {
            for line in p.get_console_since(next).await {
                next = line.id + 1;
                if line.source != crate::supervisor::managed_process::LogSource::System && done.is_match(&line.content) {
                    confirmed = true;
                }
            }
            if confirmed {
                break;
            }
            if tokio::time::Instant::now() >= deadline || !p.is_running() {
                return Err(SaveNotDone::Unconfirmed { command: cmd.to_string() });
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    if !confirmed {
        // 저장 명령은 대개 응답 뒤에도 디스크에 쓴다
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    Ok(())
}

/// 백업은 만들었지만 `resume_command`가 실패해 서버의 자동 저장이 멈춘 채다 (`autosave_not_resumed`)
#[derive(Debug)]
struct AutosaveNotResumed {
    backup: String,
}

impl std::fmt::Display for AutosaveNotResumed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backup {} was made but the server's autosave could not be turned back on; restart the server", self.backup)
    }
}

impl std::error::Error for AutosaveNotResumed {}

/// 이 인스턴스의 백업 폴더와 백업할 경로
struct Plan {
    dir: PathBuf,
    paths: Vec<PathBuf>,
    /// 패턴이 가리키는 경로 전부 (지금 없는 것 포함). 복원이 되살려도 되는 범위
    targets: Vec<PathBuf>,
    /// 훅이 더 돌려준 복원 허용 범위 (`restore_roots`). 백업 대상은 아니다 (MA3)
    restore_roots: Vec<PathBuf>,
    /// 켜진 서버에서의 저장·자동 저장 멈춤 명령과 뺄 파일 (`[backup]`, 선언이 없으면 빈 값)
    spec: crate::supervisor::module_loader::ModuleBackup,
    keep: usize,
    boundary: crate::module_paths::Boundary,
    module_name: String,
}

/// 백업 경로의 기준 폴더: 작업 폴더, 없으면 모듈 설치 폴더
fn server_base(instance: &crate::instance::ServerInstance, instance_dir: &FsPath, metadata: Option<&crate::supervisor::module_loader::ModuleMetadata>) -> PathBuf {
    crate::module_paths::server_dir(instance, instance_dir, metadata)
}

/// 모듈이 `[backup]`을 선언하지 않았으면 Ok(None)
async fn plan(state: &IPCServer, id: &str) -> Result<Option<Plan>, Response> {
    let (spec, base, dir, keep, hook_call, lookup, boundary, module_name) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(id).cloned() else {
            return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
        };
        let module = supervisor.module_loader.get_module(&instance.module_name).ok();
        let Some(spec) = module.as_ref().and_then(|m| m.metadata.backup.clone()) else {
            return Ok(None);
        };
        let instance_dir = supervisor.instance_store.instance_dir(id);
        let base = server_base(&instance, &instance_dir, module.as_ref().map(|m| &m.metadata));
        let boundary = crate::module_paths::Boundary::for_instance(&instance, &instance_dir, module.as_ref().map(|m| &m.metadata));
        let hook_call = module.as_ref().filter(|_| spec.hook).map(|m| {
            let mut cfg = serde_json::Map::new();
            cfg.insert("working_dir".into(), json!(base.to_string_lossy()));
            cfg.insert("extension_data".into(), json!(&instance.extension_data));
            crate::supervisor::Supervisor::insert_instance_fields(&mut cfg, &instance, &supervisor.instance_store.instance_dir(id));
            crate::plugin::PluginCall::new(m.entry_path(), "backup_paths", Value::Object(cfg))
                .in_instance(Some(state.host_context(id)))
        });
        // `{설정}` 자리: 인스턴스 설정 값, 없으면 module.toml 기본값
        let defaults = module.as_ref().map(|m| m.metadata.clone());
        let settings = instance.module_settings.clone();
        let lookup = move |key: &str| -> Option<String> {
            let text = |v: &Value| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let value = settings.get(key).map(text).or_else(|| defaults.as_ref()?.get_setting_default::<String>(key))?;
            // 경로 형식이 아닌 설정(월드 이름 같은 글자, 원격에서도 바꿀 수 있다)은 서버 폴더 안 상대 경로만 받는다
            let field = defaults.as_ref().and_then(|m| m.settings.as_ref()).and_then(|s| s.fields.iter().find(|f| f.name == key));
            let path_field = field.is_none_or(|f| f.field_type == "file" || f.format.as_deref() == Some("path"));
            let inside = std::path::Path::new(&value).components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir));
            if !path_field && !inside {
                tracing::warn!("Backup path setting '{}' must stay inside the server folder; skipping it", key);
                return None;
            }
            Some(value)
        };
        let keep = instance.backup_keep.map(|k| k as usize).unwrap_or(DEFAULT_KEEP);
        (spec, base, backup::backups_dir(&instance_dir), keep, hook_call, lookup, boundary, instance.module_name)
    };
    let mut patterns = spec.paths.clone();
    let mut root_patterns = Vec::new();
    if let Some(call) = hook_call {
        match call.run().await {
            Ok(v) if v.get("success").and_then(Value::as_bool) == Some(true) => {
                let strings = |key: &str| v[key].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect::<Vec<_>>();
                patterns.extend(strings("paths"));
                root_patterns = strings("restore_roots");
            }
            // 경로를 모르면 안전 백업 없이 업데이트·복원이 진행되니 여기서 멈춘다
            Ok(v) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(v)).into_response());
            }
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&e))).into_response()),
        }
    }
    let resolve = |paths: Vec<PathBuf>| -> Result<Vec<PathBuf>, Response> {
        paths.into_iter().map(|path| boundary.resolve(&path).ok_or_else(|| {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&crate::module_paths::outside(&module_name, "backup_paths", path.display())))).into_response()
        })).collect()
    };
    let restore_roots = resolve(backup::fill_paths(&root_patterns, &base, &lookup))?;
    let targets = resolve(backup::fill_paths(&patterns, &base, lookup))?;
    let paths = targets.iter().filter(|p| p.exists()).cloned().collect();
    Ok(Some(Plan { dir, paths, targets, restore_roots, spec, keep, boundary, module_name }))
}

/// 백업 하나를 만들고 오래된 자동 백업을 정리한다. 백업할 것이 없으면 Ok(None).
/// 모듈이 백업을 지원하지 않으면 수동 백업만 오류이고, 자동 백업(업데이트 전·예약)은 건너뛴다
pub async fn take_backup(state: &IPCServer, id: &str, kind: BackupKind) -> Result<Option<BackupInfo>, Response> {
    let Some(guard) = InstanceLock::take(id) else {
        return Err(error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance"));
    };
    let (state, id) = (state.clone(), id.to_string());
    detached(async move {
        let _guard = guard;
        take_backup_locked(&state, &id, kind, None, &[]).await
    })
    .await
}

/// 잠금을 쥔 작업을 따로 돌린다: 요청이 끊겨 핸들러가 버려져도 파일 작업이 끝날 때까지 잠금이 남는다
async fn detached<T: Send + 'static>(work: impl std::future::Future<Output = Result<T, Response>> + Send + 'static) -> Result<T, Response> {
    crate::audit::spawn(work).await.unwrap_or_else(|e| Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e.to_string())))
}

/// 작업 잠금이 풀리면 시작한다. 재시작이 백업·복원과 겹쳐도 서버가 꺼진 채 남지 않게 한다.
/// 기다리는 동안 `still_wanted`가 false가 되면 그만두고 None.
/// `generation`은 부른 쪽이 확인한 실행 세대다: 시작도 이 값을 기준으로 삼아, 확인 뒤에 들어온 정지가 시작을 취소하게 한다
pub(crate) async fn start_when_free<F, Fut>(state: &IPCServer, id: &str, generation: u64, still_wanted: F) -> Option<crate::ipc::internal::InternalResponse>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + START_WAIT_LIMIT;
    loop {
        let start = super::server::start_instance_handler(Path(id.to_string()), State(state.clone()));
        let resp = crate::ipc::internal::into_internal(crate::supervisor::START_SINCE.scope(generation, start).await).await;
        if resp.body["error_code"] != "instance_busy" || std::time::Instant::now() >= deadline {
            return Some(resp);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if !still_wanted().await {
            return None;
        }
    }
}

/// 이미 작업 잠금을 잡은 호출자(업데이트)용 `take_backup`
pub(crate) async fn take_backup_held(state: &IPCServer, id: &str, kind: BackupKind, _guard: &InstanceLock) -> Result<Option<BackupInfo>, Response> {
    take_backup_locked(state, id, kind, None, &[]).await
}

/// 다른 작업이 잡고 있을 때의 응답 (시작·업데이트·삭제)
pub(crate) fn instance_busy() -> Response {
    error(StatusCode::CONFLICT, "instance_busy", "A backup, restore or update is running for this instance")
}

/// `also`의 경로를 더한다. 이미 담는 경로 안에 있는 것은 빼고, 담는 경로를 품는 것은 그 경로들을 대신한다
fn add_paths(paths: &mut Vec<PathBuf>, also: &[PathBuf]) {
    for path in also {
        if !path.exists() || paths.iter().any(|p| path.starts_with(p)) {
            continue;
        }
        paths.retain(|p| !p.starts_with(path));
        paths.push(path.clone());
    }
}

/// `[backup]` 선언 없이 경로만 따로 담을 때의 계획: 이 인스턴스의 백업 폴더와 보관 개수
async fn bare_plan(state: &IPCServer, id: &str) -> Result<Plan, Response> {
    let supervisor = state.supervisor.read().await;
    let Some(instance) = supervisor.instance_store.get(id) else {
        return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
    };
    Ok(Plan {
        dir: backup::backups_dir(&supervisor.instance_store.instance_dir(id)),
        paths: Vec::new(),
        targets: Vec::new(),
        restore_roots: Vec::new(),
        spec: Default::default(),
        keep: instance.backup_keep.map(|k| k as usize).unwrap_or(DEFAULT_KEEP),
        boundary: crate::module_paths::Boundary::for_instance(instance, &supervisor.instance_store.instance_dir(id), supervisor.module_loader.get_module(&instance.module_name).ok().as_ref().map(|m| &m.metadata)),
        module_name: instance.module_name.clone(),
    })
}

/// `protect`: 정리에서 빼 둘 백업 이름 (복원 중인 백업)
/// `also`: 지금 설정이 가리키지 않아도 함께 담을 경로 (복원이 덮을 경로). 없는 것과 이미 담는 경로와 겹치는 것은 뺀다
async fn take_backup_locked(state: &IPCServer, id: &str, kind: BackupKind, protect: Option<String>, also: &[PathBuf]) -> Result<Option<BackupInfo>, Response> {
    let mut plan = match plan(state, id).await? {
        Some(plan) => plan,
        None if kind == BackupKind::Manual => {
            return Err(error(StatusCode::BAD_REQUEST, "backup_unsupported", "This module does not declare [backup] paths"));
        }
        // 모듈이 `[backup]`을 더는 선언하지 않아도 복원이 덮을 경로는 남긴다
        None if !also.is_empty() => bare_plan(state, id).await?,
        None => return Ok(None),
    };
    add_paths(&mut plan.paths, also);
    if plan.paths.is_empty() {
        return Ok(None);
    }
    let remote = crate::audit::current().is_relay();
    if remote && backup::list(&plan.dir).iter().filter(|b| b.kind == kind).count() >= REMOTE_BACKUP_CAP {
        return Err(error(
            StatusCode::CONFLICT,
            "backup_limit",
            format!("This instance already has {} backups of this kind; delete old ones on this PC first", REMOTE_BACKUP_CAP),
        ));
    }
    // 켜진 서버: 자동 저장을 멈추고(pause) 저장한 뒤 묶고, 끝나면(실패해도) 자동 저장을 되돌린다(resume)
    let live = (plan.spec.save_command.is_some() || plan.spec.pause_command.is_some()) && is_running(state, id).await;
    let resume = if live { plan.spec.resume_command.clone() } else { None };
    let saved = if live {
        let failures = state.supervisor.read().await.module_loader.get_module(&plan.module_name).ok()
            .and_then(|m| m.metadata.commands.map(|c| c.failure_patterns))
            .unwrap_or_default();
        save_for_backup(state, id, &plan.spec, &failures).await
    } else {
        Ok(())
    };
    let Plan { dir, paths, keep, boundary, module_name, spec, .. } = plan;
    // 원격(디스코드·챗봇)은 백업을 지우지 못한다: 원격 복원을 거듭해 앞서 남긴 안전 백업을 밀어내지 않게, 정리는 이 PC의 다음 백업에 맡긴다 (D-B11)
    let result = if let Err(e) = saved {
        tracing::warn!("Not backing up '{}': {}", id, e);
        Err(e.into())
    } else { tokio::task::spawn_blocking(move || {
        for path in &paths {
            if boundary.resolve(path).is_none() {
                return Err(crate::module_paths::outside(&module_name, "backup_paths", path.display()));
            }
        }
        let made = backup::create_excluding(&dir, kind, &paths, &spec.exclude)?;
        // 방금 만든 백업은 보관 개수와 상관없이 남긴다
        let mut keep_names = vec![made.name.as_str()];
        keep_names.extend(protect.as_deref());
        let pruned = if remote { Vec::new() } else { backup::prune(&dir, keep, &keep_names) };
        anyhow::Ok((made, pruned))
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(|r| r) };
    let mut result = result.map(|(made, pruned)| {
        tracing::info!("Backed up '{}' as {} ({} bytes, pruned {})", id, made.name, made.size, pruned.len());
        made
    });
    if let Some(cmd) = resume.as_deref() {
        if !run_command(state, id, cmd, json!({})).await {
            tracing::warn!("Command '{}' failed after backing up '{}'; the server may not save automatically until it restarts", cmd, id);
            // 백업은 남았어도 서버가 자동 저장을 하지 않는다: 성공으로 알리지 않는다
            if let Ok(made) = &result {
                result = Err(AutosaveNotResumed { backup: made.name.clone() }.into());
            }
        }
    }
    let instance = state.supervisor.read().await.instance_store.get(id).map(|i| i.name.clone()).unwrap_or_default();
    crate::events::emit("backup.done", match &result {
        Ok(made) => json!({ "instanceId": id, "instance": instance, "kind": kind, "name": made.name, "ok": true }),
        Err(e) => json!({ "instanceId": id, "instance": instance, "kind": kind, "ok": false, "error": e.to_string() }),
    });
    result.map(Some).map_err(|e| {
        if e.downcast_ref::<crate::plugin::ModuleCallError>().is_some() {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&e))).into_response()
        } else if let Some(save) = e.downcast_ref::<SaveNotDone>() {
            // 서버 답 원문은 싣지 않는다: 거절 까닭(모듈 실패 모양의 reason)과 명령 이름만
            let body = match save {
                SaveNotDone::Rejected { command, reason } => json!({ "success": false, "error": e.to_string(), "error_code": "backup_save_rejected", "command": command, "reason": reason }),
                SaveNotDone::Unconfirmed { command } => json!({ "success": false, "error": e.to_string(), "error_code": "backup_save_unconfirmed", "command": command }),
            };
            (StatusCode::CONFLICT, Json(body)).into_response()
        } else if let Some(paused) = e.downcast_ref::<AutosaveNotResumed>() {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string(), "error_code": "autosave_not_resumed", "backup": paused.backup }))).into_response()
        } else if let Some(locked) = e.downcast_ref::<backup::LockedFile>() {
            // 켜진 서버가 쥐고 있는 파일: 어느 파일인지 이름만 (경로는 PC 밖으로 내지 않는다)
            let file = locked.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (StatusCode::CONFLICT, Json(json!({ "success": false, "error": e.to_string(), "error_code": "backup_file_locked", "file": file }))).into_response()
        } else {
            error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e.to_string())
        }
    })
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
        Ok(dir) => {
            // 게임이 만든 자동 백업도 함께 (새것부터 사바쨩 백업 뒤에)
            let mut items: Vec<Value> = backup::list(&dir).into_iter().map(|b| json!(b)).collect();
            let snapshots = snapshot_items(&state, &id).await;
            let snapshots_supported = has_snapshots(&state, &id).await;
            items.extend(snapshots);
            Json(json!({ "success": true, "backups": items, "snapshots_supported": snapshots_supported })).into_response()
        }
        Err(r) => r,
    }
}

/// 모듈이 게임 자동 백업을 알려 주는가 (`[backup] snapshots`)
async fn has_snapshots(state: &IPCServer, id: &str) -> bool {
    let supervisor = state.supervisor.read().await;
    let Some(instance) = supervisor.instance_store.get(id) else { return false };
    supervisor.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.backup).is_some_and(|b| b.snapshots)
}

// ── 게임 자동 백업 (스냅샷) ──

/// zip 스냅샷을 잠깐 푸는 자리 (백업 폴더 안, `.snapshot-<uuid>`)
const STAGING_PREFIX: &str = ".snapshot-";

/// 강제 종료로 남은 임시 자리를 지운다 (호출자가 작업 잠금을 쥐고 있다)
fn clear_stale_staging(dir: &FsPath) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(STAGING_PREFIX) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// 백업 목록·복원 이름에서 게임 스냅샷을 가리키는 앞붙임 (`game-snapshot:<id>`)
pub const SNAPSHOT_PREFIX: &str = "game-snapshot:";

/// 백업할 원본: 지금 상태(저장 뒤 복사) 또는 게임이 만든 스냅샷 (None이면 가장 새것)
pub enum Source {
    Current,
    Snapshot(Option<String>),
}

/// 게임이 만든 스냅샷 하나. `paths`는 (스냅샷 안의 경로, 그것이 되돌아갈 월드 자리)
struct GameSnapshot {
    id: String,
    created: u64,
    /// 스냅샷이 zip 하나면 그 파일. 그때 `paths`의 앞쪽은 zip 안의 상대 경로다
    archive: Option<PathBuf>,
    paths: Vec<(PathBuf, PathBuf)>,
}

/// 모듈 `[backup] snapshots`의 lifecycle `backup_snapshots()`. 선언하지 않았으면 빈 목록.
/// 경로는 모두 이 인스턴스의 경계 안이어야 한다 (밖이면 `module_path_outside`)
async fn game_snapshots(state: &IPCServer, id: &str) -> Result<Vec<GameSnapshot>, Response> {
    let (call, boundary, module_name) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(id).cloned() else {
            return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
        };
        let Some(module) = supervisor.module_loader.get_module(&instance.module_name).ok().filter(|m| m.metadata.backup.as_ref().is_some_and(|b| b.snapshots)) else {
            return Ok(Vec::new());
        };
        let instance_dir = supervisor.instance_store.instance_dir(id);
        let base = server_base(&instance, &instance_dir, Some(&module.metadata));
        let boundary = crate::module_paths::Boundary::for_instance(&instance, &instance_dir, Some(&module.metadata));
        let mut cfg = serde_json::Map::new();
        cfg.insert("working_dir".into(), json!(base.to_string_lossy()));
        cfg.insert("extension_data".into(), json!(&instance.extension_data));
        crate::supervisor::Supervisor::insert_instance_fields(&mut cfg, &instance, &instance_dir);
        let call = crate::plugin::PluginCall::new(module.entry_path(), "backup_snapshots", Value::Object(cfg)).in_instance(Some(state.host_context(id)));
        (call, boundary, instance.module_name)
    };
    let v = match call.run().await {
        Ok(v) if v.get("success").and_then(Value::as_bool) == Some(true) => v,
        Ok(v) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(v)).into_response()),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&e))).into_response()),
    };
    let mut out = Vec::new();
    for s in v["snapshots"].as_array().into_iter().flatten() {
        let Some(sid) = s["id"].as_str().filter(|i| !i.is_empty()) else { continue };
        let outside = |path: &str| (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&crate::module_paths::outside(&module_name, "backup_snapshots", path)))).into_response();
        let resolve = |path: &str| boundary.resolve(FsPath::new(path)).ok_or_else(|| outside(path));
        // zip 하나로 된 스냅샷: 그 파일은 경계 안, zip 안의 경로는 위로 올라가지 않는 상대 경로
        let archive = match s["archive"].as_str() {
            Some(a) => Some(resolve(a)?),
            None => None,
        };
        let inner = |path: &str| {
            let p = FsPath::new(path);
            let plain = !path.is_empty() && p.components().all(|c| matches!(c, std::path::Component::Normal(_)));
            if plain { Ok(p.to_path_buf()) } else { Err(outside(path)) }
        };
        let mut paths = Vec::new();
        for p in s["paths"].as_array().into_iter().flatten() {
            let (Some(from), Some(to)) = (p["from"].as_str(), p["to"].as_str()) else { continue };
            let from = if archive.is_some() { inner(from)? } else { resolve(from)? };
            paths.push((from, resolve(to)?));
        }
        out.push(GameSnapshot { id: sid.to_string(), created: s["created"].as_u64().unwrap_or(0), archive, paths });
    }
    Ok(out)
}

/// 백업 목록에 함께 보일 스냅샷 항목 (`kind: game-snapshot`, 지울 수 없다). 읽지 못하면 빼고 로그만 남긴다
async fn snapshot_items(state: &IPCServer, id: &str) -> Vec<Value> {
    match game_snapshots(state, id).await {
        Ok(list) => list
            .into_iter()
            .map(|s| json!({ "name": format!("{}{}", SNAPSHOT_PREFIX, s.id), "kind": "game-snapshot", "created": s.created, "readable": true }))
            .collect(),
        Err(r) => {
            tracing::warn!("Listing game snapshots of '{}' failed: {:?}", id, crate::ipc::internal::into_internal(r).await.body);
            Vec::new()
        }
    }
}

/// 고른 스냅샷 (None이면 가장 새것). 없으면 404 `snapshot_not_found`
async fn find_snapshot(state: &IPCServer, id: &str, which: Option<&str>) -> Result<GameSnapshot, Response> {
    let list = game_snapshots(state, id).await?;
    let found = match which {
        Some(sid) => list.into_iter().find(|s| s.id == sid),
        None => list.into_iter().next(),
    };
    found.ok_or_else(|| error(StatusCode::NOT_FOUND, "snapshot_not_found", "The game has no such automatic backup"))
}

/// 스냅샷을 사바쨩 백업으로 적는다 (작업 잠금을 쥔 채). 없는 경로는 뺀다
async fn keep_snapshot_locked(state: &IPCServer, id: &str, snapshot: GameSnapshot, kind: BackupKind) -> Result<BackupInfo, Response> {
    let dir = backups_dir(state, id).await?;
    if kind == BackupKind::Snapshot && crate::audit::current().is_relay() && backup::list(&dir).iter().filter(|b| b.kind == kind).count() >= REMOTE_BACKUP_CAP {
        return Err(error(StatusCode::CONFLICT, "backup_limit", format!("This instance already has {} backups of this kind; delete old ones on this PC first", REMOTE_BACKUP_CAP)));
    }
    if snapshot.archive.as_ref().is_some_and(|a| !a.is_file()) || (snapshot.archive.is_none() && !snapshot.paths.iter().any(|(from, _)| from.exists())) {
        return Err(error(StatusCode::NOT_FOUND, "snapshot_not_found", "The game's automatic backup has no files"));
    }
    let made = tokio::task::spawn_blocking(move || match &snapshot.archive {
        None => {
            let entries: Vec<(PathBuf, PathBuf)> = snapshot.paths.into_iter().filter(|(from, _)| from.exists()).collect();
            backup::create_from(&dir, kind, &entries, &[])
        }
        // zip 하나로 된 스냅샷: 고른 항목만 백업 폴더 안의 임시 자리에 풀어 적고, 그 자리는 지운다
        Some(archive) => {
            clear_stale_staging(&dir);
            let staging = dir.join(format!("{}{}", STAGING_PREFIX, uuid::Uuid::new_v4().simple()));
            let members: Vec<String> = snapshot.paths.iter().map(|(from, _)| from.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")).collect();
            let made = backup::stage_archive(archive, &members, &staging).and_then(|()| {
                let entries: Vec<(PathBuf, PathBuf)> = snapshot.paths.iter().map(|(from, to)| (staging.join(from), to.clone())).filter(|(from, _)| from.exists()).collect();
                if entries.is_empty() {
                    anyhow::bail!("the archive has none of the listed entries");
                }
                backup::create_from(&dir, kind, &entries, &[])
            });
            let _ = std::fs::remove_dir_all(&staging);
            made
        }
    })
    .await;
    match made {
        Ok(Ok(made)) => Ok(made),
        Ok(Err(e)) if e.downcast_ref::<backup::LockedFile>().is_some() => Err(error(StatusCode::CONFLICT, "backup_file_locked", e.to_string())),
        Ok(Err(e)) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e.to_string())),
        Err(e) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e.to_string())),
    }
}

/// 백업 하나를 만든다: 지금 상태(`Current`, 수동 백업) 또는 게임 스냅샷 보관(`Snapshot`)
pub async fn create_from(state: &IPCServer, id: &str, source: Source) -> Result<BackupInfo, Response> {
    let which = match source {
        Source::Current => {
            return match take_backup(state, id, BackupKind::Manual).await? {
                Some(made) => Ok(made),
                None => Err(error(StatusCode::BAD_REQUEST, "nothing_to_back_up", "None of the module's backup paths exist yet")),
            };
        }
        Source::Snapshot(which) => which,
    };
    let Some(guard) = InstanceLock::take(id) else {
        return Err(error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance"));
    };
    let (state, id) = (state.clone(), id.to_string());
    detached(async move {
        let _guard = guard;
        let snapshot = find_snapshot(&state, &id, which.as_deref()).await?;
        let made = keep_snapshot_locked(&state, &id, snapshot, BackupKind::Snapshot).await;
        let instance = state.supervisor.read().await.instance_store.get(&id).map(|i| i.name.clone()).unwrap_or_default();
        crate::events::emit("backup.done", match &made {
            Ok(b) => json!({ "instanceId": id, "instance": instance, "kind": b.kind, "name": b.name, "ok": true }),
            Err(_) => json!({ "instanceId": id, "instance": instance, "kind": BackupKind::Snapshot, "ok": false }),
        });
        made
    })
    .await
}

/// POST 본문 (없으면 지금 상태). `source`: `current` | `snapshot`, `snapshot`: 게임 스냅샷 id (없으면 가장 새것)
#[derive(Deserialize, Default)]
pub struct CreateBody {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    snapshot: Option<String>,
    /// 원격 액션 `backup.create`의 칸: true면 `source: snapshot`과 같다
    #[serde(default)]
    from_snapshot: bool,
}

pub async fn create_backup(Path(id): Path<String>, State(state): State<IPCServer>, LooseJson(body): LooseJson<CreateBody>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let source = match body.source.as_deref() {
        None if body.from_snapshot => Source::Snapshot(body.snapshot),
        None | Some("current") => Source::Current,
        Some("snapshot") => Source::Snapshot(body.snapshot),
        Some(other) => return error(StatusCode::BAD_REQUEST, "invalid_request", format!("Unknown backup source: {}", other)),
    };
    match create_from(&state, &id, source).await {
        Ok(made) => Json(json!({ "success": true, "backup": made })).into_response(),
        Err(r) => r,
    }
}

pub async fn delete_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>) -> Response {
    if name.starts_with(SNAPSHOT_PREFIX) {
        return error(StatusCode::BAD_REQUEST, "snapshot_not_deletable", "The game manages its own automatic backups");
    }
    let dir = match backups_dir(&state, &id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Some(_guard) = InstanceLock::take(&id) else {
        return error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance");
    };
    // 지운 뒤 쓰지 않는 내용을 치우느라 모든 백업을 읽는다: 비동기 작업자를 막지 않게
    match tokio::task::spawn_blocking(move || backup::delete(&dir, &name)).await {
        Ok(Ok(())) => Json(json!({ "success": true })).into_response(),
        Ok(Err(e)) => error(StatusCode::NOT_FOUND, "backup_not_found", e.to_string()),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e.to_string()),
    }
}

pub async fn restore_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>) -> Response {
    if let Err(r) = backups_dir(&state, &id).await {
        return r;
    }
    // 잠금을 먼저 잡고 실행 여부를 본다: 시작도 같은 잠금을 잡으니 확인 뒤에 켜지지 않는다.
    // 진행 중인 설정 저장이 게임 파일을 쓰는 중이면 기다리고, 복원과 다시 읽기를 마칠 때까지 저장이 끼어들지 않게 한다
    let Some(guard) = InstanceLock::take_for_writes(&id).await else {
        return error(StatusCode::CONFLICT, "backup_busy", "A backup or restore is already running for this instance");
    };
    if is_running(&state, &id).await {
        return error(StatusCode::CONFLICT, "server_running", "Stop the server before restoring a backup");
    }
    let work = async move {
        // 게임 스냅샷은 잠깐 사바쨩 백업으로 적어 같은 복원 길(안전 백업·되돌리기)로 되돌린 뒤 그 백업을 지운다
        let response = match name.strip_prefix(SNAPSHOT_PREFIX) {
            Some(sid) => {
                let snapshot = find_snapshot(&state, &id, Some(sid)).await?;
                let staged = keep_snapshot_locked(&state, &id, snapshot, BackupKind::Manual).await?;
                let restored = restore_held(&state, &id, &staged.name, &guard, true).await;
                if let Ok(dir) = backups_dir(&state, &id).await {
                    let staged = staged.name.clone();
                    let _ = tokio::task::spawn_blocking(move || backup::delete(&dir, &staged)).await;
                }
                restored?
            }
            None => restore_held(&state, &id, &name, &guard, true).await?,
        };
        drop(guard);
        Ok(Json(response).into_response())
    };
    detached(work).await.unwrap_or_else(|r| r)
}

/// 작업 잠금을 쥔 채 백업 하나를 되돌린다. `safety`면 덮기 전에 지금 상태를 안전 백업으로 남긴다
/// (설치 실패 뒤 자동 복원은 반쯤 바뀐 상태를 남길 까닭이 없어 남기지 않는다)
pub(crate) async fn restore_held(state: &IPCServer, id: &str, name: &str, _guard: &InstanceLock, safety: bool) -> Result<Value, Response> {
    let dir = backups_dir(state, id).await?;
    // 백업은 만들 때의 절대 경로로 돌아간다: 서버 폴더를 옮긴 뒤면 옛 폴더(이제 다른 인스턴스 것일 수 있다)에 쓰게 되니 거절한다
    let (mut roots, boundary) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(id) else {
            return Err(error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id)));
        };
        let instance_dir = supervisor.instance_store.instance_dir(id);
        let module = supervisor.module_loader.get_module(&instance.module_name).ok();
        let metadata = module.as_ref().map(|m| &m.metadata);
        (vec![server_base(instance, &instance_dir, metadata), instance_dir.clone()], crate::module_paths::Boundary::for_instance(instance, &instance_dir, metadata))
    };
    // 모듈이 지금 백업하는 경로(훅·설정이 가리키는 서버 폴더 밖 데이터)도 이 인스턴스의 것이다. 지워진 폴더도 되살린다
    match plan(state, id).await {
        Ok(Some(p)) => roots.extend(p.targets.into_iter().chain(p.restore_roots)),
        Ok(None) => {}
        Err(r) => return Err(r),
    }
    let targets = match backup::backed_up_paths(&dir, name) {
        Ok(t) => t,
        Err(e) => return Err(error(StatusCode::BAD_REQUEST, "restore_failed", format!("{:#}", e))),
    };
    if targets.iter().any(|t| boundary.resolve(t).is_none() || !backup::is_inside(t, &roots)) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "restore_outside",
            "This backup was made for a different server folder; it can no longer be restored here",
        ));
    }
    // 지금 상태를 먼저 남긴다: 잘못 고른 백업도 되돌릴 수 있게. 지금 설정이 가리키지 않아도 복원이 덮을 경로는 함께 담는다
    // 복원할 백업은 이 안전 백업의 정리 대상에서 뺀다
    let safety = if safety { take_backup_locked(state, id, BackupKind::PreRestore, Some(name.to_string()), &targets).await?.map(|b| b.name) } else { None };
    let (dir, name) = (dir, name.to_string());
    match tokio::task::spawn_blocking(move || backup::restore(&dir, &name)).await {
        // 되살리지 못한 링크는 수만 알린다 (경로는 데몬 로그에 남는다)
        Ok(Ok(done)) => {
            // 되돌린 게임 설정 파일(포트 등)을 다시 읽어 인스턴스 값과 맞춘다. 실패해도 복원은 끝난 것이다 (MK17)
            let refreshed = match super::managed::refresh_settings_from_files(state, id).await {
                Ok(r) => r.get("success").and_then(Value::as_bool) == Some(true),
                Err(e) => {
                    tracing::warn!("Settings refresh after restore failed for {}: {}", id, e);
                    false
                }
            };
            Ok(json!({
                "success": true,
                "restored": done.paths,
                "pre_restore_backup": safety,
                "links_not_restored": done.links_missed,
                "settings_refreshed": refreshed,
            }))
        }
        Ok(Err(e)) => Err(error(StatusCode::BAD_REQUEST, "restore_failed", format!("{:#}", e))),
        Err(e) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, "restore_failed", e.to_string())),
    }
}

// ── 예약 작업 ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ScheduleBody {
    schedule: Vec<ScheduledTask>,
    #[serde(default)]
    backup_keep: Option<u32>,
    /// GET에서 받은 `revision`. 있으면 그사이 일정이 바뀌었을 때 덮어쓰지 않고 409로 거절한다 (L51)
    #[serde(default)]
    revision: Option<String>,
}

/// 일정과 보관 수의 지문. 목록 전체를 다시 보내는 편집이 그사이 바뀐 일정을 덮어쓰지 않게 한다
fn schedule_revision(instance: &crate::instance::ServerInstance) -> String {
    crate::config_store::patch::revision(&json!({ "schedule": instance.schedule, "backup_keep": instance.backup_keep }))
}

/// 실행기의 시계와 데몬 시작 시각 (화면에 다음 실행 시각을 보일 때 읽는다)
static RUNNER: std::sync::OnceLock<(chrono::NaiveDateTime, std::sync::Arc<Mutex<Clock>>)> = std::sync::OnceLock::new();

/// 작업마다 다음 실행 시각 (유닉스 초, 꺼졌거나 다시 돌 일이 없으면 null). 보는 쪽이 자기 시간대로 보인다
fn next_runs(id: &str, schedule: &[ScheduledTask]) -> Vec<Option<i64>> {
    let now = chrono::Local::now().naive_local();
    let fresh = Clock::default();
    let (started, guard) = match RUNNER.get() {
        Some((started, clock)) => (*started, Some(clock.lock().unwrap_or_else(|e| e.into_inner()))),
        None => (now, None),
    };
    let clock = guard.as_deref().unwrap_or(&fresh);
    schedule
        .iter()
        .map(|task| {
            let wall = clock.peek(&crate::scheduler::task_key(id, task), task, started, now)?;
            crate::scheduler::instant(&chrono::Local, wall).map(|t| t.timestamp())
        })
        .collect()
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
        "revision": schedule_revision(instance),
        "backup_supported": meta.as_ref().is_some_and(|m| m.backup.is_some()),
        "announce_supported": meta.as_ref().is_some_and(|m| m.announce_command.is_some()),
        "save_supported": meta.as_ref().is_some_and(|m| save_command_of(m).is_some()),
        "next_runs": next_runs(&id, &instance.schedule),
        // 작업 시각(`at`·`date`)은 데몬 PC의 벽시계다: 다른 시간대에서 보는 화면이 그 차이를 알린다
        "utc_offset_minutes": chrono::Local::now().offset().local_minus_utc() / 60,
    }))
    .into_response()
}

fn save_command_of(meta: &crate::supervisor::module_loader::ModuleMetadata) -> Option<String> {
    meta.backup.as_ref().and_then(|b| b.save_command.clone())
}

pub async fn put_schedule(Path(id): Path<String>, State(state): State<IPCServer>, Json(mut body): Json<ScheduleBody>) -> Response {
    // N일마다는 세기 시작할 날이 있어야 같은 날에 돈다: 없으면 오늘부터
    let today = chrono::Local::now().date_naive().to_string();
    for task in body.schedule.iter_mut().filter(|t| t.every_days.is_some() && t.start_date.is_none()) {
        task.start_date = Some(today.clone());
    }
    let mut supervisor = state.supervisor.write().await;
    let Some(mut instance) = supervisor.instance_store.get(&id).cloned() else {
        return error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id));
    };
    if body.revision.as_ref().is_some_and(|r| *r != schedule_revision(&instance)) {
        return error(StatusCode::CONFLICT, "schedule_changed", "The schedule was changed elsewhere; read it again");
    }
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
        if task.action == TaskAction::Save && !meta.as_ref().is_some_and(|m| save_command_of(m).is_some()) {
            p.push("this module has no save command".into());
        }
        problems.extend(p.into_iter().map(|m| format!("schedule[{}]: {}", i, m)));
    }
    if body.backup_keep == Some(0) {
        problems.push("backup_keep must be at least 1".into());
    }
    // 보관할 백업 수를 줄이면 다음 백업이 옛 백업을 지운다: 챗봇은 삭제를 하지 못하므로 줄이지 못한다.
    // 쓰기 잠금 안에서 지금 값과 견준다 (계획을 확인하는 사이 이 PC에서 올린 값도 본다)
    if crate::audit::current().via.as_deref() == Some("chatbot") {
        if let Some(asked) = body.backup_keep {
            if asked < instance.backup_keep.unwrap_or(DEFAULT_KEEP as u32) {
                problems.push("backup_keep: the chatbot cannot lower how many backups are kept".into());
            }
        }
        // 더 자주 돌려도 옛 백업이 더 빨리 밀려 지워진다: 보관 수가 덮는 기간(보관 수 ÷ 하루 횟수)이 줄면 거절한다.
        // 지금 백업이 없거나 드물면 하루 한 번까지는 넣을 수 있다
        let keep_before = instance.backup_keep.unwrap_or(DEFAULT_KEEP as u32) as u64;
        let keep_after = body.backup_keep.map_or(keep_before, u64::from);
        let base = daily_runs(&instance.schedule, TaskAction::Backup).max(1) as u64;
        let after = daily_runs(&body.schedule, TaskAction::Backup) as u64;
        if after > base && keep_after * base < keep_before * after {
            problems.push("schedule: the chatbot cannot make backups run so often that older ones are removed sooner".into());
        }
    }
    // 원격(디스코드·챗봇·원격 MCP)은 새로 넣거나 바꾸는 작업만 제한한다 (이 PC에서 정한 작업은 그대로 다시 보낼 수 있다)
    if crate::audit::current().is_relay() {
        for (i, task) in body.schedule.iter().enumerate() {
            if instance.schedule.contains(task) {
                continue;
            }
            // 끈 업데이트 작업은 돌지 않으므로 끄는 것은 원격에서도 된다
            if task.action == TaskAction::Update && task.enabled {
                problems.push(format!("schedule[{}]: updates can only be scheduled on this PC", i));
            }
            // 위험한 게임 명령과 디스코드에 내보내지 않는 명령(`remote = false`)은 이 PC에서 확인하고 정한다
            // (모르는 명령도 막는다: 인스턴스별 명령은 저장할 때 알 수 없다)
            if task.action == TaskAction::Command && task.enabled {
                let risk = meta
                    .as_ref()
                    .and_then(|m| crate::commands::find(m, task.command.as_deref().unwrap_or("")))
                    .filter(|f| f.remote)
                    .map(|f| crate::commands::risk_of(f.risk.as_deref()));
                if !matches!(risk, Some("safe" | "caution")) {
                    problems.push(format!("schedule[{}]: dangerous or unknown commands can only be scheduled on this PC", i));
                }
            }
            if task.every_minutes.is_some_and(|m| m < REMOTE_MIN_EVERY_MINUTES) {
                problems.push(format!("schedule[{}]: every_minutes must be at least {} from Discord", i, REMOTE_MIN_EVERY_MINUTES));
            }
        }
        // 작업 하나의 간격만 보면 매일 시각 작업이나 간격이 다른 작업 여럿으로 우회한다: 종류별 하루 실행 횟수로 본다.
        // 원격은 그 횟수를 30분에 한 번(하루 48번) 넘게 올리지 못한다 (이 PC에서 이미 그보다 잦게 정했으면 늘리지만 못한다)
        let remote_max = 24 * 60 / REMOTE_MIN_EVERY_MINUTES;
        let mut actions: Vec<TaskAction> = Vec::new();
        for task in &body.schedule {
            if !actions.contains(&task.action) {
                actions.push(task.action);
            }
        }
        for action in actions {
            let after = daily_runs(&body.schedule, action);
            if after > remote_max && after > daily_runs(&instance.schedule, action) {
                let name = serde_json::to_value(action).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
                problems.push(format!("{} would run {} times a day; at most {} from Discord", name, after, remote_max));
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
    // 이미 지난 날의 한 번 작업은 돌지 않는다: 받되 알린다 (그대로 다시 보낸 작업은 이미 알렸거나 돈 작업이다)
    let now = chrono::Local::now().naive_local();
    let warnings: Vec<Value> = body
        .schedule
        .iter()
        .enumerate()
        .filter(|(_, task)| task.enabled && task.once_passed(now) && !instance.schedule.contains(task))
        .map(|(i, _)| json!({ "code": "date_passed", "task": i }))
        .collect();
    instance.schedule = body.schedule;
    if body.backup_keep.is_some() {
        instance.backup_keep = body.backup_keep;
    }
    match supervisor.instance_store.update(&id, instance) {
        Ok(()) => Json(json!({ "success": true, "warnings": warnings })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", e.to_string()),
    }
}

// ── 실행기 ────────────────────────────────────────────────

/// 데몬이 끝날 때까지 30초마다 예약 작업을 확인한다
pub fn spawn_scheduler(state: IPCServer) {
    tokio::spawn(async move {
        let started = chrono::Local::now().naive_local();
        let clock = RUNNER.get_or_init(|| (started, std::sync::Arc::new(Mutex::new(Clock::default())))).1.clone();
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
                c.retain(&keys, now);
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
                super::server::restart_instance_handler(Path(id.to_string()), State(state.clone()), LooseJson(None)).await,
            )
            .await;
            if !resp.status.is_success() || resp.body.get("success").and_then(Value::as_bool) == Some(false) {
                tracing::warn!("Scheduled restart of '{}' failed: {}", id, resp.body);
            }
        }
        TaskAction::Update => run_update(state, id, task, fire, key).await,
        TaskAction::Start if !running => {
            tracing::info!("Scheduled start of '{}'", id);
            let resp = crate::ipc::internal::into_internal(
                super::server::start_instance_handler(Path(id.to_string()), State(state.clone())).await,
            )
            .await;
            if !resp.status.is_success() || resp.body.get("success").and_then(Value::as_bool) == Some(false) {
                tracing::warn!("Scheduled start of '{}' failed: {}", id, resp.body);
            }
        }
        // 예고 시작 때 꺼져 있었어도 정지 시각에 켜져 있으면 끈다 (같은 시각 앞의 예약 시작 등)
        TaskAction::Stop => {
            if !count_down(state, id, task, fire, key, "schedule.stop_warning", running).await || !is_running(state, id).await {
                return;
            }
            tracing::info!("Scheduled stop of '{}'", id);
            // 예약 작업은 강제 종료하지 않는다 (저장 후 종료를 못 하는 서버는 그대로 둔다)
            let resp = crate::ipc::internal::into_internal(
                super::server::stop_instance_handler(Path(id.to_string()), State(state.clone()), LooseJson(None)).await,
            )
            .await;
            if !resp.status.is_success() || resp.body.get("success").and_then(Value::as_bool) == Some(false) {
                tracing::warn!("Scheduled stop of '{}' failed: {}", id, resp.body);
            }
        }
        TaskAction::Save if running => {
            let cmd = {
                let supervisor = state.supervisor.read().await;
                supervisor
                    .instance_store
                    .get(id)
                    .and_then(|i| supervisor.module_loader.get_module(&i.module_name).ok())
                    .and_then(|m| save_command_of(&m.metadata))
            };
            if let Some(cmd) = cmd {
                if !run_command(state, id, &cmd, json!({})).await {
                    tracing::warn!("Scheduled save of '{}' failed", id);
                }
            }
        }
        // 익스텐션 명령은 서버가 꺼져 있어도 돈다: 켜짐 여부는 명령 실행이 판단하고, 결과는 감사 로그에 남는다
        TaskAction::Command => {
            let name = task.command.as_deref().unwrap_or_default();
            let args = task.args.clone().map(Value::Object).unwrap_or(Value::Null);
            if !run_command(state, id, name, args).await {
                tracing::warn!("Scheduled command '{}' on '{}' failed", name, id);
            }
        }
        _ => {}
    }
}

/// 켜진 `action` 작업들이 하루에 도는 횟수 (매일 시각 작업은 1번, N분마다는 하루를 N으로 나눈 만큼)
fn daily_runs(schedule: &[ScheduledTask], action: TaskAction) -> u32 {
    schedule
        .iter()
        .filter(|t| t.enabled && t.action == action)
        .map(|t| t.every_minutes.map_or(1, |m| (24 * 60u32).div_ceil(m.max(1))))
        .sum()
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
        // 콘솔이 없어 저장 후 종료를 못 하는 서버(`graceful_stop_unavailable`)도 여기서 멈춘다: 예약 작업은 강제 종료하지 않는다
        if !stop.status.is_success() || stop.body.get("success").and_then(Value::as_bool) == Some(false) {
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
    let generation = state.supervisor.read().await.run_generation_of(id);
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
        start_after_update(state, id, generation).await;
    }
}

/// 예약 업데이트가 끈 서버를 다시 켠다. 그 순간 다른 작업(백업·복원)이 잠금을 쥐고 있으면 끝나기를 기다린다.
/// 그사이 사용자가 켜거나 끄거나 인스턴스를 지우면(`generation`이 바뀌면) 그만둔다
async fn start_after_update(state: &IPCServer, id: &str, generation: u64) {
    let wanted = || async {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.get(id).is_some() && supervisor.run_generation_of(id) == generation
    };
    match start_when_free(state, id, generation, wanted).await {
        None => tracing::info!("Scheduled update of '{}': not starting again, the server was started, stopped or removed meanwhile", id),
        Some(resp) if !resp.status.is_success() || resp.body.get("success").and_then(Value::as_bool) == Some(false) => {
            tracing::warn!("Scheduled update of '{}': could not start the server again: {}", id, resp.body)
        }
        Some(_) => {}
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
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
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

    /// 모듈 호출이 끝나기를 기다리는 한도. 여기서 재는 것은 "끝나면 풀린다"이지 빠르기가 아니다: Windows 전체 실행에서는
    /// 첫 격리 호출(venv 만들기)과 병렬 시험 부하로 Python이 10초 넘게 걸리기도 한다 (L24)
    const WORK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(180);

    /// 잠금이 풀릴 때까지 (WORK_DEADLINE까지) 기다린다. 풀려서 잡았으면 true
    async fn lock_freed(id: &str) -> bool {
        let deadline = std::time::Instant::now() + WORK_DEADLINE;
        while std::time::Instant::now() < deadline {
            if InstanceLock::take(id).is_some() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        false
    }

    async fn code(r: Response) -> String {
        crate::ipc::internal::into_internal(r).await.body["error_code"].as_str().unwrap_or_default().to_string()
    }

    async fn data_roots_backfill_server(import_result: &str) -> (IPCServer, String, tempfile::TempDir, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("backfill-").tempdir_in(&modules).unwrap();
        let name = module.path().file_name().unwrap().to_string_lossy();
        let toml = format!("[module]\nschema_version = 1\nname = '{name}'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[backup]\npaths = []\nhook = true\n\n\
            [[settings.fields]]\nname = 'cache_dir'\ntype = 'file'\n\n\
            [[settings.fields]]\nname = 'data_home'\ntype = 'text'\nformat = 'path'\n\n\
            [[settings.fields]]\nname = 'missing_root'\ntype = 'file'\n\n\
            [extensions.saba-core]\ndata_roots = ['{{cache_dir}}', '{{data_home}}', '{{missing_root}}', '{{home}}/Zomboid']\n");
        let hook = format!(r#"import json, os, sys, time
c = json.load(sys.stdin)
if sys.argv[1] == 'functions':
    print(json.dumps({{'success': True, 'functions': ['import_settings', 'backup_paths']}}))
elif sys.argv[1] == 'import_settings':
    with open(os.path.join(c['working_dir'], 'imports'), 'ab') as f:
        f.write(b'called\n')
    {import_result}
else:
    print(json.dumps({{'success': True, 'paths': [os.path.join(os.path.dirname(c['config_dir']), 'Saves')]}}))
"#);
        std::fs::write(module.path().join("module.toml"), toml).unwrap();
        std::fs::write(module.path().join("lifecycle.py"), hook).unwrap();
        crate::sandbox::consent_test_component(&module.path().join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", &name);
        inst.working_dir = Some(tmp.path().join("server").to_string_lossy().into_owned());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let data = tmp.path().join("legacy-data");
        std::fs::create_dir_all(data.join("Server")).unwrap();
        std::fs::create_dir_all(data.join("Saves")).unwrap();
        std::fs::create_dir_all(tmp.path().join("server")).unwrap();
        std::fs::write(data.join("Saves").join("world.bin"), b"legacy world").unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("config_dir".into(), json!(data.join("Server")));
            inst.module_settings.insert("cache_dir".into(), json!(tmp.path().join("cache")));
            inst.module_settings.insert("server_name".into(), json!("chosen"));
            inst.user_settings.insert("cache_dir".into());
            sup.instance_store.update(&id, inst).unwrap();
        }
        (state, id, tmp, module)
    }

    const ROOT_IMPORT: &str = "print(json.dumps({'success': True, 'settings': {'data_home': os.path.dirname(c['config_dir']), 'cache_dir': 'replace-cache', 'server_name': 'replace-name', 'unrelated': '/outside'}}))";

    #[tokio::test]
    async fn data_roots_backfill_restores_legacy_backups_on_init() {
        let (state, id, tmp, _module) = data_roots_backfill_server(ROOT_IMPORT).await;
        state.supervisor.write().await.initialize().await.unwrap();
        assert_eq!(code(plan(&state, &id).await.err().expect("legacy data has no allowed root")).await, "module_path_outside");

        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        {
            let sup = state.supervisor.read().await;
            let inst = sup.instance_store.get(&id).unwrap();
            assert_eq!(inst.module_settings.get("data_home"), Some(&json!(tmp.path().join("legacy-data"))));
            assert!(inst.imported_settings.contains("data_home"));
            assert!(!inst.user_settings.contains("data_home"));
            assert_eq!(inst.module_settings["cache_dir"], json!(tmp.path().join("cache")));
            assert_eq!(inst.module_settings["config_dir"], json!(tmp.path().join("legacy-data").join("Server")));
            assert_eq!(inst.module_settings["server_name"], "chosen");
            assert!(!inst.module_settings.contains_key("unrelated"));
        }
        let made = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert_eq!(backup::backed_up_paths(&dir, &made).unwrap(), vec![tmp.path().join("legacy-data").join("Saves")]);

        state.supervisor.write().await.initialize().await.unwrap();
        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        assert!(state.supervisor.read().await.instance_store.get(&id).unwrap().imported_settings.contains("data_home"));
        assert_eq!(std::fs::read_to_string(tmp.path().join("server").join("imports")).unwrap(), "called\n");
    }

    #[tokio::test]
    async fn data_roots_backfill_keeps_user_values() {
        let (state, id, tmp, _module) = data_roots_backfill_server(ROOT_IMPORT).await;
        let chosen = json!(tmp.path().join("chosen-data"));
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("data_home".into(), chosen.clone());
            inst.user_settings.insert("data_home".into());
            sup.instance_store.update(&id, inst).unwrap();
            sup.initialize().await.unwrap();
        }
        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        let sup = state.supervisor.read().await;
        let inst = sup.instance_store.get(&id).unwrap();
        assert_eq!(inst.module_settings["data_home"], chosen);
        assert!(inst.user_settings.contains("data_home"));
        assert!(!inst.imported_settings.contains("data_home"));
        assert_eq!(std::fs::read_to_string(tmp.path().join("server").join("imports")).unwrap(), "called\n");
    }

    #[tokio::test]
    async fn data_roots_backfill_failure_does_not_block_init_or_repeat() {
        for result in ["print(json.dumps({'success': False, 'message': 'cannot import'}))", "raise RuntimeError('cannot import')"] {
            let (state, id, tmp, _module) = data_roots_backfill_server(result).await;
            for _ in 0..2 {
                state.supervisor.write().await.initialize().await.unwrap();
                crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
                assert!(!state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.contains_key("data_home"));
            }
            assert_eq!(std::fs::read_to_string(tmp.path().join("server").join("imports")).unwrap(), "called\n");
        }
    }

    #[tokio::test]
    async fn data_roots_backfill_keeps_a_value_saved_while_importing() {
        let result = format!("open(os.path.join(c['working_dir'], 'import-started'), 'w').close()\n    \
            while not os.path.exists(os.path.join(c['working_dir'], 'import-release')):\n        time.sleep(0.01)\n    {ROOT_IMPORT}");
        let (state, id, tmp, _module) = data_roots_backfill_server(&result).await;
        state.supervisor.write().await.initialize().await.unwrap();
        let importing = tokio::spawn(crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()));
        tokio::time::timeout(WORK_DEADLINE, async {
            while !tmp.path().join("server").join("import-started").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.expect("import_settings did not start");
        let chosen = json!(tmp.path().join("chosen-during-import"));
        {
            let mut sup = tokio::time::timeout(std::time::Duration::from_secs(5), state.supervisor.write()).await.expect("import_settings holds the daemon lock");
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("data_home".into(), chosen.clone());
            inst.user_settings.insert("data_home".into());
            sup.instance_store.update(&id, inst).unwrap();
        }
        std::fs::write(tmp.path().join("server").join("import-release"), b"").unwrap();
        importing.await.unwrap();
        let sup = state.supervisor.read().await;
        let inst = sup.instance_store.get(&id).unwrap();
        assert_eq!(inst.module_settings["data_home"], chosen);
        assert!(inst.user_settings.contains("data_home"));
        assert!(!inst.imported_settings.contains("data_home"));
    }

    #[tokio::test]
    async fn data_roots_backfill_empty_import_is_not_repeated_after_reload() {
        let (state, id, tmp, _module) = data_roots_backfill_server("print(json.dumps({'success': True, 'settings': {}}))").await;
        for _ in 0..2 {
            state.supervisor.write().await.initialize().await.unwrap();
            crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
            assert!(!state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.contains_key("data_home"));
        }
        assert_eq!(std::fs::read_to_string(tmp.path().join("server").join("imports")).unwrap(), "called\n");
    }

    #[tokio::test]
    async fn data_roots_backfill_only_attempts_new_settings_after_module_update() {
        let result = "settings = {'new_root': os.path.join(c['working_dir'], 'new-data')}\n    if c['server_name'] == 'updated':\n        settings['missing_root'] = 'must-not-retry'\n    print(json.dumps({'success': True, 'settings': settings}))";
        let (state, id, tmp, module) = data_roots_backfill_server(result).await;
        assert!(serde_json::to_value(state.supervisor.read().await.instance_store.get(&id).unwrap()).unwrap().get("data_roots_backfill_attempted").is_none());
        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        assert!(!state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.contains_key("new_root"));

        let manifest = module.path().join("module.toml");
        let updated = std::fs::read_to_string(&manifest).unwrap()
            .replace("[extensions.saba-core]", "[[settings.fields]]\nname = 'new_root'\ntype = 'file'\n\n[extensions.saba-core]")
            .replace("'{missing_root}',", "'{missing_root}', '{new_root}',");
        std::fs::write(manifest, updated).unwrap();
        crate::sandbox::consent_test_component(&module.path().join("lifecycle.py"));
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("server_name".into(), json!("updated"));
            sup.instance_store.update(&id, inst).unwrap();
            sup.module_loader.invalidate_cache();
            sup.initialize().await.unwrap();
        }
        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        state.supervisor.write().await.initialize().await.unwrap();
        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        let sup = state.supervisor.read().await;
        let inst = sup.instance_store.get(&id).unwrap();
        assert_eq!(inst.module_settings.get("new_root"), Some(&json!(tmp.path().join("server").join("new-data"))));
        assert!(inst.imported_settings.contains("new_root"));
        assert!(!inst.module_settings.contains_key("data_home"));
        assert!(!inst.module_settings.contains_key("missing_root"));
        assert_eq!(serde_json::to_value(inst).unwrap()["data_roots_backfill_attempted"], json!(["data_home", "missing_root", "new_root"]));
        assert_eq!(std::fs::read_to_string(tmp.path().join("server").join("imports")).unwrap(), "called\ncalled\n");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn data_roots_backfill_stores_an_isolated_import_result() {
        let _logs = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().with_max_level(tracing::Level::WARN).finish());
        let result = format!("assert os.environ.get('SABA_LOOPBACK_DIR'), 'import_settings must run isolated'\n    {ROOT_IMPORT}");
        let (state, id, tmp, module) = data_roots_backfill_server(&result).await;
        let name = module.path().file_name().unwrap().to_string_lossy();
        crate::sandbox::set_consent(crate::sandbox::Consent::Unisolated, &format!("module-{name}"), false).unwrap();
        let entry = module.path().join("lifecycle.py");
        assert_eq!(crate::sandbox::trust_of_entry(&entry), saba_chan_updater_lib::integrity::Trust::Unverified);
        crate::supervisor::Supervisor::backfill_data_roots(state.supervisor.clone()).await;
        state.supervisor.write().await.initialize().await.unwrap();
        let sup = state.supervisor.read().await;
        let inst = sup.instance_store.get(&id).unwrap();
        assert_eq!(inst.module_settings.get("data_home"), Some(&json!(tmp.path().join("legacy-data"))));
        assert!(inst.imported_settings.contains("data_home"));
    }

    #[tokio::test]
    async fn contract_paths_backup_stops_if_any_hook_path_or_restore_root_is_outside() {
        for field in ["paths", "restore_roots"] {
            let tmp = tempfile::tempdir().unwrap();
            let modules = crate::plugin::resolve_modules_dir();
            let module = tempfile::Builder::new().prefix("paths-").tempdir_in(&modules).unwrap();
            let name = module.path().file_name().unwrap().to_string_lossy();
            let toml = format!("[module]\nschema_version = 1\nname = '{name}'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[backup]\npaths = []\nhook = true\n");
            std::fs::write(module.path().join("module.toml"), toml).unwrap();
            let outside = tmp.path().join("unrelated");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("private.txt"), "private").unwrap();
            let result = json!({"success": true, field: [outside]});
            let entry = module.path().join("lifecycle.py");
            std::fs::write(&entry, format!("import sys\nprint('{{\"success\":true,\"functions\":[\"backup_paths\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
            crate::sandbox::consent_test_component(&entry);
            let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
            sup.initialize().await.unwrap();
            let mut instance = crate::instance::ServerInstance::new("g", &name);
            instance.working_dir = Some(tmp.path().join("server").to_string_lossy().into_owned());
            let id = instance.id.clone();
            sup.instance_store.add(instance).unwrap();
            let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
            let response = plan(&state, &id).await.err().expect("outside path must fail the whole backup plan");
            let body = crate::ipc::internal::into_internal(response).await.body;
            assert_eq!(body["error_code"], "module_path_outside", "{}: {}", field, body);
        }
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
        let hook = "import json\nprint(json.dumps({'success': False, 'error_code': 'backup_failed', 'message': 'no world'}))\n";
        let (state, id, _tmp) = server_with(&toml, Some(hook)).await;
        let err = take_backup(&state, &id, BackupKind::PreUpdate).await.err().expect("hook failure must not mean 'nothing to back up'");
        assert_eq!(code(err).await, "backup_failed");
    }

    #[tokio::test]
    async fn start_update_and_delete_wait_for_a_running_restore() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let guard = InstanceLock::take(&id).expect("free");
        let start = super::super::server::start_instance_handler(Path(id.clone()), State(state.clone())).await;
        assert_eq!(code(start).await, "instance_busy");
        let managed = super::super::managed::start_managed_handler(Path(id.clone()), State(state.clone()), super::LooseJson(None)).await.into_response();
        assert_eq!(code(managed).await, "instance_busy");
        let update = super::super::instance::begin_server_update(&state, &id).await.err().expect("update must wait");
        assert_eq!(code(update).await, "instance_busy");
        let removed = super::super::instance::delete_instance(Path(id.clone()), State(state.clone()), axum::extract::Query(Default::default())).await.into_response();
        assert_eq!(code(removed).await, "instance_busy");
        assert!(state.supervisor.read().await.instance_store.get(&id).is_some());
        drop(guard);
        assert!(InstanceLock::take(&id).is_some(), "refused calls must not keep the lock");
    }

    #[tokio::test]
    async fn a_crash_restart_waits_for_a_running_backup_instead_of_giving_up() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let guard = InstanceLock::take(&id).expect("free");
        let restart = tokio::spawn(super::super::server::auto_restart(state.clone(), id.clone(), std::time::Duration::ZERO, 0));
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(!restart.is_finished(), "the restart must wait for the backup, not fail with instance_busy");
        drop(guard);
        tokio::time::timeout(WORK_DEADLINE, restart).await.expect("starts once the lock is free").unwrap();
    }

    /// 예약 업데이트가 끝난 뒤 다시 켤 때도, 그 순간 다른 작업(백업)이 잠금을 쥐면 끝나기를 기다린다 (꺼진 채 남지 않게)
    #[tokio::test]
    async fn a_scheduled_update_restart_waits_for_a_running_backup() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let generation = state.supervisor.read().await.run_generation_of(&id);
        let guard = InstanceLock::take(&id).expect("free");
        let restart = tokio::spawn({
            let (state, id) = (state.clone(), id.clone());
            async move { start_after_update(&state, &id, generation).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(!restart.is_finished(), "the restart must wait for the backup, not fail with instance_busy");
        drop(guard);
        tokio::time::timeout(WORK_DEADLINE, restart).await.expect("starts once the lock is free").unwrap();
    }

    #[tokio::test]
    async fn a_dropped_restore_request_keeps_the_lock_until_the_work_ends() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = []\nhook = true\n");
        let hook = "import json, time\ntime.sleep(1.5)\nprint(json.dumps({'success': True, 'paths': []}))\n";
        let (state, id, tmp) = server_with(&toml, Some(hook)).await;
        // 복원은 백업이 돌아갈 경로부터 확인하니 실제 백업이 있어야 안전 백업 단계까지 간다
        let world = tmp.path().join("server/world");
        std::fs::create_dir_all(&world).unwrap();
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        let made = backup::create(&dir, BackupKind::Manual, &[world]).unwrap().name;
        let request = restore_backup(Path((id.clone(), made)), State(state.clone()));
        if let Ok(done) = tokio::time::timeout(std::time::Duration::from_millis(300), request).await {
            panic!("still in the safety backup: {:?}", crate::ipc::internal::into_internal(done).await.body);
        }
        assert!(InstanceLock::take(&id).is_none(), "a client that hangs up must not free the lock while files are still being written");
        assert!(lock_freed(&id).await, "the lock is released when the work ends");
    }

    #[tokio::test]
    async fn a_dropped_reset_request_keeps_the_lock_until_the_module_is_done() {
        let hook = "import json, time\ntime.sleep(1.5)\nprint(json.dumps({'success': True}))\n";
        let (state, id, _tmp) = server_with(NO_BACKUP, Some(hook)).await;
        let request = super::super::managed::reset_server_handler(Path(id.clone()), State(state.clone()));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(300), request).await.is_err(), "the module is still running");
        assert!(InstanceLock::take(&id).is_none(), "a client that hangs up must not free the lock mid-reset");
        assert!(lock_freed(&id).await, "the lock is released when the reset ends");
    }

    /// 복원은 지금 서버 폴더·인스턴스 폴더 밖에 쓰지 않고, 복원 전 안전 백업은 백업이 덮을 경로까지 담는다 (D-B1)
    #[tokio::test]
    async fn a_restore_keeps_what_it_overwrites_and_stays_inside_the_server_folders() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"{{level_name}}\"]\n");
        let (state, id, tmp) = server_with(&toml, None).await;
        let set = |key: &'static str, value: String| {
            let state = state.clone();
            let id = id.clone();
            async move {
                let mut sup = state.supervisor.write().await;
                let mut inst = sup.instance_store.get(&id).cloned().unwrap();
                if key == "working_dir" {
                    inst.working_dir = Some(value);
                } else {
                    inst.module_settings.insert(key.into(), json!(value));
                }
                sup.instance_store.update(&id, inst).unwrap();
            }
        };
        let server = tmp.path().join("server");
        std::fs::create_dir_all(server.join("world")).unwrap();
        std::fs::write(server.join("world/level.dat"), b"old").unwrap();
        set("level_name", "world".into()).await;
        let old = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;

        // 월드 이름을 바꾼 뒤 옛 백업을 복원해도 옛 월드의 그사이 진행분은 안전 백업에 남는다
        std::fs::write(server.join("world/level.dat"), b"progress").unwrap();
        std::fs::create_dir_all(server.join("world2")).unwrap();
        set("level_name", "world2".into()).await;
        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), old.clone())), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        let safety = done.body["pre_restore_backup"].as_str().expect("a safety backup").to_string();
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert!(backup::backed_up_paths(&dir, &safety).unwrap().contains(&server.join("world")), "the overwritten world must be kept");

        // 서버 폴더를 옮긴 뒤에는 옛 폴더에 쓰지 않는다
        let moved = tmp.path().join("moved");
        std::fs::create_dir_all(moved.join("world")).unwrap();
        set("working_dir", moved.to_string_lossy().to_string()).await;
        std::fs::write(server.join("world/level.dat"), b"someone else").unwrap();
        let refused = restore_backup(Path((id.clone(), old)), State(state.clone())).await;
        assert_eq!(code(refused).await, "restore_outside");
        assert_eq!(std::fs::read(server.join("world/level.dat")).unwrap(), b"someone else");
    }

    /// 게임이 스스로 만든 자동 백업(스냅샷)을 `[backup] snapshots`의 `backup_snapshots()`가 알려 준다.
    /// 백업 목록에 함께 보이고, 사바쨩 백업으로 보관할 수 있고, 바로 복원할 수 있다 (사용자 2026-10-24)
    const SNAPSHOT_HOOK: &str = r#"import json, os, sys
c = json.load(sys.stdin)
w = os.path.join(c['working_dir'], 'world')
snap = os.path.join(w, 'backup', 'S1')
print(json.dumps({'success': True, 'snapshots': [{'id': 'S1', 'created': 1792800000, 'paths': [
    {'from': os.path.join(snap, 'Level.sav'), 'to': os.path.join(w, 'Level.sav')},
    {'from': os.path.join(snap, 'Players'), 'to': os.path.join(w, 'Players')}]}]}))
"#;

    #[tokio::test]
    async fn game_snapshots_are_listed_kept_and_restored() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"world\"]\nsnapshots = true\n");
        let (state, id, tmp) = server_with(&toml, Some(SNAPSHOT_HOOK)).await;
        let world = tmp.path().join("server").join("world");
        std::fs::create_dir_all(world.join("backup/S1/Players")).unwrap();
        std::fs::create_dir_all(world.join("Players")).unwrap();
        std::fs::write(world.join("Level.sav"), b"live").unwrap();
        std::fs::write(world.join("Players/p.sav"), b"live player").unwrap();
        std::fs::write(world.join("backup/S1/Level.sav"), b"snap").unwrap();
        std::fs::write(world.join("backup/S1/Players/p.sav"), b"snap player").unwrap();

        // 목록: 사바쨩 백업과 함께 게임 자동 백업이 보인다
        let listed = crate::ipc::internal::into_internal(list_backups(Path(id.clone()), State(state.clone())).await).await.body;
        let game = listed["backups"].as_array().unwrap().iter().find(|b| b["kind"] == "game-snapshot").cloned().expect("the game snapshot is listed");
        assert_eq!((game["name"].as_str(), game["created"].as_u64()), (Some("game-snapshot:S1"), Some(1792800000)));

        // 보관: 스냅샷 내용을 월드 자리 경로로 적은 사바쨩 백업
        let kept = create_from(&state, &id, Source::Snapshot(None)).await.unwrap();
        assert_eq!(kept.kind, BackupKind::Snapshot);
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert!(backup::backed_up_paths(&dir, &kept.name).unwrap().contains(&world.join("Level.sav")));
        // 원격 액션 backup.create 의 칸 from_snapshot 도 같은 길이다
        let body: CreateBody = serde_json::from_value(json!({ "from_snapshot": true })).unwrap();
        let via_action = crate::ipc::internal::into_internal(create_backup(Path(id.clone()), State(state.clone()), super::LooseJson(Some(Json(body)))).await).await.body;
        assert_eq!(via_action["backup"]["kind"], "snapshot", "{via_action}");

        // 바로 복원: 지금 월드는 안전 백업으로 남기고 스냅샷으로 바꾼다
        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), "game-snapshot:S1".into())), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        assert!(done.body["pre_restore_backup"].is_string());
        assert_eq!(std::fs::read(world.join("Level.sav")).unwrap(), b"snap");
        assert_eq!(std::fs::read(world.join("Players/p.sav")).unwrap(), b"snap player");
        assert_eq!(std::fs::read(world.join("backup/S1/Level.sav")).unwrap(), b"snap", "the game's own snapshot stays");

        // 보관한 백업도 스냅샷 내용으로 되돌린다
        std::fs::write(world.join("Level.sav"), b"later").unwrap();
        let again = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), kept.name.clone())), State(state.clone())).await).await;
        assert_eq!(again.body["success"], true, "{:?}", again.body);
        assert_eq!(std::fs::read(world.join("Level.sav")).unwrap(), b"snap");

        // 게임 자동 백업은 사바쨩이 지우지 않는다. 없는 스냅샷은 찾지 못한다
        assert_eq!(code(delete_backup(Path((id.clone(), "game-snapshot:S1".into())), State(state.clone())).await).await, "snapshot_not_deletable");
        assert_eq!(code(restore_backup(Path((id.clone(), "game-snapshot:nope".into())), State(state.clone())).await).await, "snapshot_not_found");
        assert!(matches!(create_from(&state, &id, Source::Snapshot(Some("nope".into()))).await, Err(_)));
    }

    #[tokio::test]
    async fn a_game_snapshot_outside_the_server_folder_is_refused() {
        let hook = r#"import json, os, sys
c = json.load(sys.stdin)
print(json.dumps({'success': True, 'snapshots': [{'id': 'S1', 'created': 1, 'paths': [{'from': os.path.join(c['working_dir'], 'x'), 'to': os.path.abspath(os.path.join(c['working_dir'], '..', '..', 'elsewhere'))}]}]}))
"#;
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"world\"]\nsnapshots = true\n");
        let (state, id, tmp) = server_with(&toml, Some(hook)).await;
        std::fs::create_dir_all(tmp.path().join("server")).unwrap();
        std::fs::write(tmp.path().join("server/x"), b"x").unwrap();
        let err = create_from(&state, &id, Source::Snapshot(None)).await.err().expect("a snapshot that writes outside the server folder");
        assert_eq!(code(err).await, "module_path_outside");
    }

    /// zip 하나로 된 게임 자동 백업(좀보이드 등): `archive`와 zip 안의 경로(`from`)를 주면 데몬이 그 항목만 풀어 보관·복원한다
    const ZIP_SNAPSHOT_HOOK: &str = r#"import json, os, sys
c = json.load(sys.stdin)
w = c['working_dir']
print(json.dumps({'success': True, 'snapshots': [{'id': 'startup/backup_1', 'created': 1792800000,
    'archive': os.path.join(w, 'backups', 'startup', 'backup_1.zip'),
    'paths': [{'from': 'Saves/Multiplayer/srv', 'to': os.path.join(w, 'Saves', 'Multiplayer', 'srv')},
              {'from': 'db/srv.db', 'to': os.path.join(w, 'db', 'srv.db')}]}]}))
"#;

    fn write_zip(path: &FsPath, files: &[(&str, &[u8])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, body) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, body).unwrap();
        }
        z.finish().unwrap();
    }

    #[tokio::test]
    async fn a_zipped_game_snapshot_is_kept_and_restored_from_inside_the_archive() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"Saves\"]\nsnapshots = true\n");
        let (state, id, tmp) = server_with(&toml, Some(ZIP_SNAPSHOT_HOOK)).await;
        let server = tmp.path().join("server");
        let world = server.join("Saves/Multiplayer/srv");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::create_dir_all(server.join("db")).unwrap();
        std::fs::write(world.join("map_0_0.bin"), b"live").unwrap();
        std::fs::write(server.join("db/srv.db"), b"live db").unwrap();
        write_zip(&server.join("backups/startup/backup_1.zip"), &[
            ("Saves/Multiplayer/srv/map_0_0.bin", b"zipped"),
            ("Saves/Multiplayer/srv/players.db", b"zipped players"),
            ("Saves/Multiplayer/other/map_9_9.bin", b"another server"),
            ("db/srv.db", b"zipped db"),
        ]);

        let kept = create_from(&state, &id, Source::Snapshot(None)).await.unwrap();
        assert_eq!(kept.kind, BackupKind::Snapshot);
        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), "game-snapshot:startup/backup_1".into())), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        assert_eq!(std::fs::read(world.join("map_0_0.bin")).unwrap(), b"zipped");
        assert_eq!(std::fs::read(world.join("players.db")).unwrap(), b"zipped players");
        assert_eq!(std::fs::read(server.join("db/srv.db")).unwrap(), b"zipped db");
        assert!(!server.join("Saves/Multiplayer/other").exists(), "only the listed entries come out of the archive");
        // 풀어 둔 자리는 남기지 않는다
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".snapshot-")));
    }

    /// zip 안의 경로가 위로 올라가면(`..`) 받지 않는다 (zip slip)
    #[tokio::test]
    async fn a_zipped_snapshot_path_that_climbs_out_is_refused() {
        let hook = ZIP_SNAPSHOT_HOOK.replace("'from': 'db/srv.db'", "'from': '../../evil'");
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"Saves\"]\nsnapshots = true\n");
        let (state, id, tmp) = server_with(&toml, Some(&hook)).await;
        write_zip(&tmp.path().join("server/backups/startup/backup_1.zip"), &[("Saves/Multiplayer/srv/a", b"a")]);
        let err = create_from(&state, &id, Source::Snapshot(None)).await.err().expect("a climbing path inside the archive");
        assert_eq!(code(err).await, "module_path_outside");
    }

    /// 훅이 `restore_roots`로 넓힌 폴더 안의 옛 백업은 복원된다. 그 폴더는 새 백업 대상이 되지 않는다 (MA3)
    #[tokio::test]
    async fn restore_roots_let_an_older_backup_of_the_whole_data_folder_come_back() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = []\nhook = true\n\n[[settings.fields]]\nname = 'data_home'\ntype = 'file'\n\n[extensions.saba-core]\ndata_roots = ['{{data_home}}']\n");
        let data = tempfile::tempdir().unwrap();
        let server = data.path().join("Server");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("srv.ini"), b"old").unwrap();
        std::fs::write(server.join("srv_SandboxVars.lua"), b"old").unwrap();
        let hook = format!(
            "import json\nprint(json.dumps({{'success': True, 'paths': [{ini:?}], 'restore_roots': [{root:?}]}}))\n",
            ini = server.join("srv.ini").to_string_lossy(),
            root = server.to_string_lossy(),
        );
        let (state, id, _tmp) = server_with(&toml, Some(&hook)).await;
        {
            let mut sup = state.supervisor.write().await;
            let mut instance = sup.instance_store.get(&id).cloned().unwrap();
            instance.module_settings.insert("data_home".into(), json!(data.path()));
            instance.user_settings.insert("data_home".into());
            sup.instance_store.update(&id, instance).unwrap();
        }
        // 예전 모듈은 데이터 폴더를 통째로 담았다
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        let old = backup::create(&dir, BackupKind::Manual, std::slice::from_ref(&server)).unwrap().name;
        std::fs::write(server.join("srv_SandboxVars.lua"), b"new").unwrap();

        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), old)), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        assert_eq!(std::fs::read(server.join("srv_SandboxVars.lua")).unwrap(), b"old");

        let made = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;
        assert_eq!(backup::backed_up_paths(&dir, &made).unwrap(), vec![server.join("srv.ini")], "restore_roots are not backed up");
    }

    /// 경로 형식이 아닌 설정 값(월드 이름 같은 글자)은 백업 경로를 서버 폴더 밖으로 돌리지 못한다:
    /// 원격에서도 바꿀 수 있는 값이라 절대 경로나 `..`이면 그 경로는 백업·복원 범위에서 뺀다
    #[tokio::test]
    async fn a_text_setting_cannot_point_backups_outside_the_server_folder() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"{{level_name}}\"]\n\n[[settings.fields]]\nname = \"level_name\"\ntype = \"text\"\ndefault = \"world\"\n");
        let (state, id, tmp) = server_with(&toml, None).await;
        let outside = tmp.path().join("user-data");
        for value in ["..".to_string(), "world/../..".to_string(), outside.to_string_lossy().to_string()] {
            {
                let mut sup = state.supervisor.write().await;
                let mut inst = sup.instance_store.get(&id).cloned().unwrap();
                inst.module_settings.insert("level_name".into(), json!(value));
                sup.instance_store.update(&id, inst).unwrap();
            }
            let p = plan(&state, &id).await.ok().flatten().expect("the module declares backups");
            assert!(p.targets.is_empty(), "{value} must not become a backup target: {:?}", p.targets);
        }
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("level_name".into(), json!("world2"));
            sup.instance_store.update(&id, inst).unwrap();
        }
        let p = plan(&state, &id).await.ok().flatten().unwrap();
        assert_eq!(p.targets, vec![tmp.path().join("server/world2")]);
    }

    /// 복원은 게임 설정 파일도 되돌린다: 복원 뒤 모듈 `import_settings`로 다시 읽어 인스턴스 값(포트 등)을 파일과 맞춘다 (MK17)
    #[tokio::test]
    async fn a_restore_reads_the_restored_game_settings_back() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"server.properties\"]\n");
        let hook = "import json, os, sys\n\
                    cfg = json.load(sys.stdin)\n\
                    if sys.argv[1] == 'import_settings':\n    \
                    line = open(os.path.join(cfg['working_dir'], 'server.properties')).read()\n    \
                    print(json.dumps({'success': True, 'settings': {'port': int(line.split('=')[1])}}))\n\
                    else:\n    print(json.dumps({'success': True}))\n";
        let (state, id, tmp) = server_with(&toml, Some(hook)).await;
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();
        let set_port = |port: u16| {
            let (state, id, server) = (state.clone(), id.clone(), server.clone());
            async move {
                std::fs::write(server.join("server.properties"), format!("server-port={port}")).unwrap();
                let mut sup = state.supervisor.write().await;
                let mut inst = sup.instance_store.get(&id).cloned().unwrap();
                inst.port = Some(port);
                sup.instance_store.update(&id, inst).unwrap();
            }
        };
        set_port(25565).await;
        let old = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;
        set_port(25570).await;

        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), old)), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        assert_eq!(std::fs::read_to_string(server.join("server.properties")).unwrap(), "server-port=25565");
        assert_eq!(done.body["settings_refreshed"], true, "{:?}", done.body);
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.port, Some(25565), "the instance follows the restored file");
    }

    /// 모듈이 `[backup]`을 더는 선언하지 않아도(모듈 업데이트로 빠짐, 모듈을 못 읽음) 남은 백업을 복원하면
    /// 덮을 경로의 지금 상태는 안전 백업으로 남긴다
    #[tokio::test]
    async fn a_restore_keeps_a_safety_backup_when_the_module_no_longer_declares_backups() {
        let (state, id, tmp) = server_with(NO_BACKUP, None).await;
        let world = tmp.path().join("server/world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"old").unwrap();
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        let old = backup::create(&dir, BackupKind::Manual, std::slice::from_ref(&world)).unwrap().name;
        std::fs::write(world.join("level.dat"), b"progress").unwrap();

        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), old)), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        let safety = done.body["pre_restore_backup"].as_str().expect("the overwritten world needs a safety backup").to_string();
        assert!(backup::backed_up_paths(&dir, &safety).unwrap().contains(&world));
        assert_eq!(std::fs::read(world.join("level.dat")).unwrap(), b"old");
    }

    // 훅이나 설정이 서버 폴더 밖(예: 사용자 폴더의 게임 데이터)을 가리키면, 지금도 가리키는 동안은 복원할 수 있다
    #[tokio::test]
    async fn a_backup_of_a_folder_the_module_still_points_outside_can_be_restored() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"{{cache_dir}}\"]\n\n[[settings.fields]]\nname = 'cache_dir'\ntype = 'file'\n\n[extensions.saba-core]\ndata_roots = ['{{cache_dir}}']\n");
        let (state, id, tmp) = server_with(&toml, None).await;
        let outside = tmp.path().join("user-data");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("save.bin"), b"old").unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.module_settings.insert("cache_dir".into(), json!(outside.to_string_lossy()));
            inst.user_settings.insert("cache_dir".into());
            sup.instance_store.update(&id, inst).unwrap();
        }
        let name = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;
        std::fs::write(outside.join("save.bin"), b"new").unwrap();
        let done = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), name.clone())), State(state.clone())).await).await;
        assert_eq!(done.body["success"], true, "{:?}", done.body);
        assert_eq!(std::fs::read(outside.join("save.bin")).unwrap(), b"old");

        // 그 폴더가 통째로 지워진 뒤에도 그 백업으로 되살린다
        std::fs::remove_dir_all(&outside).unwrap();
        let revived = crate::ipc::internal::into_internal(restore_backup(Path((id.clone(), name)), State(state.clone())).await).await;
        assert_eq!(revived.body["success"], true, "{:?}", revived.body);
        assert_eq!(std::fs::read(outside.join("save.bin")).unwrap(), b"old");
    }

    #[test]
    fn a_restore_target_that_holds_a_planned_path_replaces_it() {
        let tmp = tempfile::tempdir().unwrap();
        let world = tmp.path().join("world");
        std::fs::create_dir_all(world.join("region")).unwrap();
        let mut paths = vec![world.join("region")];
        add_paths(&mut paths, &[world.clone(), world.join("region"), tmp.path().join("missing")]);
        assert_eq!(paths, vec![world]);
    }

    #[tokio::test]
    async fn version_install_and_reset_wait_for_other_work_and_refuse_a_running_server() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let dir = state.supervisor.read().await.instance_store.get(&id).unwrap().working_dir.clone().unwrap();
        let install = || {
            let state = state.clone();
            let dir = dir.clone();
            async move {
                let body = json!({ "version": "1.0", "install_dir": dir });
                code(super::super::managed::install_server_handler(Path("plain".into()), State(state), Json(body)).await.into_response()).await
            }
        };
        let reset = || {
            let (state, id) = (state.clone(), id.clone());
            async move { code(super::super::managed::reset_server_handler(Path(id), State(state)).await.into_response()).await }
        };

        let guard = InstanceLock::take(&id).expect("free");
        assert_eq!(install().await, "instance_busy");
        assert_eq!(reset().await, "instance_busy");
        drop(guard);

        state.supervisor.read().await.tracker.track(&id, std::process::id()).unwrap();
        assert_eq!(install().await, "server_running");
        assert_eq!(reset().await, "server_running");
        assert!(InstanceLock::take(&id).is_some(), "refused calls must not keep the lock");
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
        // 원격 MCP(AI 에이전트)도 릴레이로 온 원격이다: 예약으로 되묻기를 건너뛰지 못한다
        let mcp = |body: Value| crate::audit::with_origin(crate::audit::Origin::new(crate::actions::MCP_VIA), put(body));
        assert_eq!(mcp(json!({ "schedule": [update] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(mcp(json!({ "schedule": [restart(1)] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [restart(60)], "backup_keep": 1000 })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [restart(60)], "backup_keep": 5 })).await, StatusCode::OK);
        // 0개 보관이면 방금 만든 안전 백업까지 지워진다
        assert_eq!(put(json!({ "schedule": [], "backup_keep": 0 })).await, StatusCode::BAD_REQUEST);
        // 이 PC에서 정한 작업은 디스코드에서 그대로 다시 보내도 된다
        assert_eq!(put(json!({ "schedule": [update, restart(1)], "backup_keep": 100 })).await, StatusCode::OK);
        assert_eq!(remote(json!({ "schedule": [update, restart(1)], "backup_keep": 100 })).await, StatusCode::OK);
        // 이미 30분에 한 번보다 잦은 작업에 원격이 더 얹을 수는 없다
        assert_eq!(remote(json!({ "schedule": [update, restart(1), restart(60)], "backup_keep": 100 })).await, StatusCode::BAD_REQUEST);
        // 끄는 것은 된다
        let off = json!({ "action": "update", "at": "04:00", "enabled": false });
        assert_eq!(remote(json!({ "schedule": [off, restart(1)], "backup_keep": 100 })).await, StatusCode::OK);
    }

    /// 챗봇은 보관할 백업 수를 줄이지 못한다(줄이면 다음 백업이 옛 백업을 지운다). 쓰기 잠금 안에서 지금 값과 견주므로,
    /// 계획을 확인하는 사이 이 PC에서 값을 올렸어도 챗봇이 그 값을 다시 내리지 못한다
    #[tokio::test]
    async fn new_schedule_actions_are_checked_against_the_module_and_discord() {
        let module = format!(
            "{NO_BACKUP}\n[[commands.fields]]\nname = \"say\"\nrisk = \"safe\"\n\n[[commands.fields]]\nname = \"wipe\"\nrisk = \"danger\"\n\n[[commands.fields]]\nname = \"op\"\nrisk = \"safe\"\nremote = false\n"
        );
        let (state, id, _tmp) = server_with(&module, None).await;
        let put = |body: Value| {
            let state = state.clone();
            let id = id.clone();
            async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                put_schedule(Path(id), State(state), Json(body)).await.status()
            }
        };
        let remote = |body: Value| crate::audit::with_origin(crate::audit::Origin::new("discord"), put(body));
        let cmd = |name: &str| json!({ "action": "command", "at": "04:00", "weekdays": ["mon"], "command": name });

        // 저장 명령이 없는 모듈은 저장 작업을 받지 않는다
        assert_eq!(put(json!({ "schedule": [{ "action": "save", "at": "04:00" }] })).await, StatusCode::BAD_REQUEST);
        // 위험하거나 모르는 명령은 이 PC에서만
        assert_eq!(remote(json!({ "schedule": [cmd("say")] })).await, StatusCode::OK);
        assert_eq!(remote(json!({ "schedule": [cmd("wipe")] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [cmd("nope")] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote(json!({ "schedule": [cmd("op")] })).await, StatusCode::BAD_REQUEST, "not for Discord");
        assert_eq!(put(json!({ "schedule": [cmd("wipe")] })).await, StatusCode::OK);
        assert_eq!(remote(json!({ "schedule": [cmd("wipe")] })).await, StatusCode::OK, "resending what this PC set");

        // N일마다는 세기 시작할 날을 오늘로 채운다
        let every = json!({ "action": "start", "at": "04:00", "every_days": 2 });
        assert_eq!(put(json!({ "schedule": [every] })).await, StatusCode::OK);
        let got = crate::ipc::internal::into_internal(get_schedule(Path(id.clone()), State(state.clone())).await).await.body;
        assert_eq!(got["schedule"][0]["start_date"], json!(chrono::Local::now().date_naive().to_string()));
        assert!(got["next_runs"][0].as_i64().is_some(), "{got}");
        assert_eq!(got["save_supported"], json!(false));
    }

    #[tokio::test]
    async fn the_chatbot_cannot_lower_how_many_backups_are_kept() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let put = |body: Value, origin: crate::audit::Origin| {
            let (state, id) = (state.clone(), id.clone());
            crate::audit::with_origin(origin, async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                put_schedule(Path(id), State(state), Json(body)).await.status()
            })
        };
        let keep = |n: u32| json!({ "schedule": [], "backup_keep": n });
        let chatbot = || crate::audit::Origin { via: Some("chatbot".into()), ..crate::audit::Origin::new("discord") };
        assert_eq!(put(keep(DEFAULT_KEEP as u32 - 1), chatbot()).await, StatusCode::BAD_REQUEST, "below the default");
        assert_eq!(put(keep(12), chatbot()).await, StatusCode::OK, "raising is fine");
        // 이 PC에서 20으로 올린 뒤 챗봇의 12는 줄이는 것이다
        assert_eq!(put(keep(20), crate::audit::Origin::new("gui")).await, StatusCode::OK);
        assert_eq!(put(keep(12), chatbot()).await, StatusCode::BAD_REQUEST);
        assert_eq!(put(keep(20), chatbot()).await, StatusCode::OK, "the same value is fine");
        // 사람이 직접 보낸 디스코드 요청은 줄일 수 있다 (30 이하)
        assert_eq!(put(keep(5), crate::audit::Origin::new("discord")).await, StatusCode::OK);
    }

    /// 챗봇은 백업을 더 자주 돌려 옛 백업이 더 빨리 밀려 지워지게 하지도 못한다: 보관 수가 덮는 기간이 줄면 거절한다.
    /// 하루 한 번까지는 새로 넣을 수 있고, 보관 수를 함께 올려 기간을 지키면 된다 (L49)
    #[tokio::test]
    async fn the_chatbot_cannot_make_backups_push_old_ones_out_sooner() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"world\"]\n");
        let (state, id, _tmp) = server_with(&toml, None).await;
        let put = |body: Value, origin: crate::audit::Origin| {
            let (state, id) = (state.clone(), id.clone());
            crate::audit::with_origin(origin, async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                put_schedule(Path(id), State(state), Json(body)).await.status()
            })
        };
        let chatbot = || crate::audit::Origin { via: Some("chatbot".into()), ..crate::audit::Origin::new("discord") };
        let every = |m: u32| json!({ "action": "backup", "every_minutes": m });
        let daily = json!({ "action": "backup", "at": "04:00" });
        assert_eq!(put(json!({ "schedule": [daily.clone()] }), chatbot()).await, StatusCode::OK, "once a day is fine");
        assert_eq!(put(json!({ "schedule": [every(60)] }), chatbot()).await, StatusCode::BAD_REQUEST, "24 a day pushes ten backups out in ten hours");
        assert_eq!(put(json!({ "schedule": [every(720)], "backup_keep": 20 }), chatbot()).await, StatusCode::OK, "twice a day with twice the keep covers the same days");
        // 이 PC에서 정한 잦은 백업은 챗봇이 그대로 다시 보낼 수 있다
        assert_eq!(put(json!({ "schedule": [every(60)], "backup_keep": 20 }), crate::audit::Origin::new("gui")).await, StatusCode::OK);
        assert_eq!(put(json!({ "schedule": [every(60)], "backup_keep": 20 }), chatbot()).await, StatusCode::OK);
        assert_eq!(put(json!({ "schedule": [every(30)], "backup_keep": 20 }), chatbot()).await, StatusCode::BAD_REQUEST);
    }

    /// 원격 제한은 작업 하나의 간격이 아니라 하루에 도는 횟수로 본다: 매일 시각 작업이나 간격이 다른 작업을 여럿 넣어도,
    /// 나눠서 보내도 30분에 한 번보다 자주 돌지 않는다
    #[tokio::test]
    async fn remote_schedules_cannot_add_up_to_running_more_often() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let remote = |body: Value| {
            let (state, id) = (state.clone(), id.clone());
            crate::audit::with_origin(crate::audit::Origin::new("discord"), async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                put_schedule(Path(id), State(state), Json(body)).await.status()
            })
        };
        let at = |t: &str| json!({ "action": "restart", "at": t });
        let every = |m: u32| json!({ "action": "restart", "every_minutes": m });
        let daily: Vec<Value> = (0..49).map(|m| at(&format!("{:02}:{:02}", m / 60, m % 60))).collect();
        assert_eq!(remote(json!({ "schedule": daily })).await, StatusCode::BAD_REQUEST, "49 daily restarts");
        let many: Vec<Value> = (30..33).map(every).collect();
        assert_eq!(remote(json!({ "schedule": many })).await, StatusCode::BAD_REQUEST, "three restarts of about 30 minutes each");
        // 하루 48번까지는 된다 (30분마다 하나와 같다)
        let ok: Vec<Value> = (0..48).map(|m| at(&format!("{:02}:{:02}", m / 60, m % 60))).collect();
        assert_eq!(remote(json!({ "schedule": ok.clone() })).await, StatusCode::OK);
        // 이미 받은 작업 위에 하나 더 보내는 것도 막는다
        let mut more = ok;
        more.push(at("23:59"));
        assert_eq!(remote(json!({ "schedule": more })).await, StatusCode::BAD_REQUEST);
        // 다른 종류의 작업은 따로 센다
        let announce_free = json!({ "action": "restart", "every_minutes": 30 });
        assert_eq!(remote(json!({ "schedule": [announce_free] })).await, StatusCode::OK);
    }

    /// 목록 전체를 다시 보내는 편집이 그사이 다른 곳에서 바꾼 일정을 덮어쓰지 않는다: 읽을 때 받은 revision이
    /// 지금 일정과 다르면 409 `schedule_changed`로 거절하고, revision을 보내지 않으면 예전처럼 받는다 (L51)
    #[tokio::test]
    async fn a_schedule_edit_from_a_stale_read_is_refused() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let read = || async { get_schedule(Path(id.clone()), State(state.clone())).await };
        let put = |body: Value| {
            let (state, id) = (state.clone(), id.clone());
            async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                put_schedule(Path(id), State(state), Json(body)).await
            }
        };
        let body_of = |r: Response| async move { serde_json::from_slice::<Value>(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap() };
        let restart = |t: &str| json!({ "action": "restart", "at": t });

        let seen = body_of(read().await).await["revision"].as_str().expect("revision").to_string();
        // 다른 클라이언트가 그사이 작업을 넣었다
        assert_eq!(put(json!({ "schedule": [restart("04:00")] })).await.status(), StatusCode::OK, "without a revision it is taken as before");
        let stale = put(json!({ "schedule": [restart("05:00")], "revision": seen })).await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        let stale = body_of(stale).await;
        assert_eq!(stale["error_code"], "schedule_changed", "{stale}");
        let now = body_of(read().await).await;
        assert_eq!(now["schedule"][0]["at"], "04:00", "the other client's task is kept");
        // 새로 읽은 revision이면 받는다
        let fresh = now["revision"].as_str().unwrap().to_string();
        assert_eq!(put(json!({ "schedule": [restart("05:00")], "revision": fresh.clone() })).await.status(), StatusCode::OK);
        let after = body_of(read().await).await;
        assert_ne!(after["revision"], json!(fresh), "a saved change gives a new revision");
    }

    /// 이미 지난 날짜의 한 번 작업은 돌지 않는다: 거절하지 않고 받되 `warnings`로 알린다.
    /// 그대로 다시 보낸 작업(이미 돈 작업 등)이나 꺼 둔 작업, 아직 오지 않은 날은 알리지 않는다
    #[tokio::test]
    async fn a_once_task_saved_after_its_date_is_kept_with_a_warning() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let put = |body: Value| {
            let (state, id) = (state.clone(), id.clone());
            async move {
                let body: ScheduleBody = serde_json::from_value(body).unwrap();
                let r = put_schedule(Path(id), State(state), Json(body)).await;
                let status = r.status();
                (status, serde_json::from_slice::<Value>(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap())
            }
        };
        let past = json!({ "action": "start", "at": "04:00", "date": "2000-01-01" });
        let future = json!({ "action": "start", "at": "04:00", "date": "2999-12-31" });
        let (status, body) = put(json!({ "schedule": [future.clone(), past.clone()] })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["warnings"], json!([{ "code": "date_passed", "task": 1 }]), "{body}");
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().schedule.len(), 2, "saved anyway");
        // 같은 작업을 그대로 다시 보내면 (다른 작업을 고칠 때) 다시 알리지 않는다
        let (_, body) = put(json!({ "schedule": [future.clone(), past.clone(), { "action": "stop", "at": "05:00" }] })).await;
        assert_eq!(body["warnings"], json!([]), "{body}");
        let off = json!({ "action": "stop", "at": "04:00", "date": "2000-01-02", "enabled": false });
        let (_, body) = put(json!({ "schedule": [off] })).await;
        assert_eq!(body["warnings"], json!([]), "{body}");
    }

    /// 원격(디스코드·챗봇)은 백업을 지우지 못한다: 복원을 거듭해도 앞서 남긴 복원 전 안전 백업이 보관 수만큼 밀려 지워지지 않는다.
    /// 이 PC에서 한 작업은 그대로 보관 수만큼 정리한다 (D-B11)
    #[tokio::test]
    async fn remote_restores_do_not_push_out_earlier_safety_backups() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"world\"]\n");
        let (state, id, tmp) = server_with(&toml, None).await;
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).unwrap().clone();
            inst.backup_keep = Some(1);
            sup.instance_store.update(&id, inst).unwrap();
        }
        let world = tmp.path().join("server/world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"old").unwrap();
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        let old = backup::create(&dir, BackupKind::Manual, std::slice::from_ref(&world)).unwrap().name;
        let restore = |origin: crate::audit::Origin| {
            let (state, id, old) = (state.clone(), id.clone(), old.clone());
            crate::audit::with_origin(origin, async move {
                let done = crate::ipc::internal::into_internal(restore_backup(Path((id, old)), State(state)).await).await;
                assert_eq!(done.body["success"], true, "{:?}", done.body);
            })
        };
        let safety = || backup::list(&dir).into_iter().filter(|b| b.kind == BackupKind::PreRestore).count();
        for _ in 0..3 {
            restore(crate::audit::Origin::new("discord")).await;
        }
        assert_eq!(safety(), 3, "remote restores must keep every safety backup");
        restore(crate::audit::Origin::new("gui")).await;
        assert_eq!(safety(), 1, "a restore on this PC still keeps only backup_keep");
    }

    /// 원격은 백업을 지우지 못하므로 원격이 쌓는 백업에는 종류마다 상한이 있다: 넘으면 409 `backup_limit`로 거절하고
    /// 이 PC에서는 그대로 만든다 (D-B7)
    #[tokio::test]
    async fn remote_backups_stop_at_a_cap_because_remote_cannot_delete_them() {
        let toml = format!("{NO_BACKUP}\n[backup]\npaths = [\"world\"]\n");
        let (state, id, tmp) = server_with(&toml, None).await;
        let world = tmp.path().join("server/world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"x").unwrap();
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        for _ in 0..REMOTE_BACKUP_CAP {
            backup::create(&dir, BackupKind::Manual, std::slice::from_ref(&world)).unwrap();
        }
        let create = |origin: crate::audit::Origin| {
            let (state, id) = (state.clone(), id.clone());
            crate::audit::with_origin(origin, async move { create_backup(Path(id), State(state), LooseJson(None)).await })
        };
        let refused = create(crate::audit::Origin::new("discord")).await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert_eq!(code(refused).await, "backup_limit");
        let local = create(crate::audit::Origin::new("gui")).await;
        assert_eq!(local.status(), StatusCode::OK, "this PC can still back up");
    }

    /// 켜진 서버로 백업하는 시험용 모듈: 콘솔 명령 세 개와 `[backup]`. `save_done`이 있으면 저장 완료 줄로 선언한다
    fn live_module(save_done: Option<&str>) -> String {
        let done = save_done.map(|p| format!("save_done_pattern = '{p}'\n")).unwrap_or_default();
        format!(
            "{NO_BACKUP}\n[protocols]\nsupported = [\"stdin\"]\n\n[commands]\nfailure_patterns = ['(?m)^Unknown command']\n\n\
             [[commands.fields]]\nname = \"save-all\"\nmethod = \"console\"\nconsole_template = \"save-all\"\n\n\
             [[commands.fields]]\nname = \"save-off\"\nmethod = \"console\"\nconsole_template = \"save-off\"\n\n\
             [[commands.fields]]\nname = \"save-on\"\nmethod = \"console\"\nconsole_template = \"save-on\"\n\n\
             [backup]\npaths = [\"world\"]\npause_command = \"save-off\"\nsave_command = \"save-all\"\nresume_command = \"save-on\"\n{done}"
        )
    }

    /// `script`(sh)를 서버로 띄워 managed로 둔다. 받은 줄은 `commands.log`에 남는다
    #[cfg(unix)]
    async fn run_fake_server(state: &IPCServer, id: &str, tmp: &tempfile::TempDir, script: &str) {
        let work = tmp.path().join("server");
        std::fs::create_dir_all(work.join("world")).unwrap();
        std::fs::write(work.join("world/level.dat"), b"old").unwrap();
        let body = format!("while read l; do echo \"$l\" >> commands.log; case \"$l\" in {script} *) echo \"ok $l\";; esac; done");
        let proc = crate::supervisor::managed_process::ManagedProcess::spawn("sh", &["-c".into(), body], work.to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        state.supervisor.read().await.managed_store.insert(id, proc).await;
    }

    #[cfg(unix)]
    fn backed_up_level(state_dir: &FsPath, name: &str, work: &FsPath) -> String {
        std::fs::write(work.join("world/level.dat"), b"after").unwrap();
        backup::restore(state_dir, name).unwrap();
        std::fs::read_to_string(work.join("world/level.dat")).unwrap()
    }

    /// 저장 완료 줄(`save_done_pattern`)이 있으면 고정 대기 대신 그 줄이 나올 때까지 기다린 뒤 묶는다.
    /// 실측: Paper 1.20.4는 빈 월드에서도 save-all 답("Saving the game") 뒤 2초 지나서 "Saved the game"을 찍었다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_live_backup_waits_for_the_declared_save_done_line() {
        let (state, id, tmp) = server_with(&live_module(Some("^Saved the game$")), None).await;
        run_fake_server(&state, &id, &tmp, "save-all) echo 'Saving the game'; sleep 4; echo new > world/level.dat; echo 'Saved the game';;").await;
        let made = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        let work = tmp.path().join("server");
        assert_eq!(backed_up_level(&dir, &made, &work), "new\n", "the backup holds what the save wrote");
        assert_eq!(std::fs::read_to_string(work.join("commands.log")).unwrap(), "save-off\nsave-all\nsave-on\n");
    }

    /// 저장 명령 답 뒤에 섞인 줄이나 기다리는 동안 다른 명령이 찍은 실패 줄로 저장을 버리지 않는다: 거절은 답의 첫 줄로만 가린다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failure_line_while_waiting_for_the_save_does_not_abort_the_backup() {
        let (state, id, tmp) = server_with(&live_module(Some("^Saved the game$")), None).await;
        run_fake_server(&state, &id, &tmp, "save-all) echo 'Saving the game'; echo 'Unknown command'; sleep 2; echo 'Unknown command'; echo new > world/level.dat; echo 'Saved the game';;").await;
        let made = take_backup(&state, &id, BackupKind::Manual).await.unwrap().unwrap().name;
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert_eq!(backed_up_level(&dir, &made, &tmp.path().join("server")), "new\n");
    }

    /// 저장 명령을 서버가 거절하면(모듈의 실패 모양) 백업을 만들지 않고 실패로 알린다. 자동 저장은 되돌린다.
    /// 실측: Folia에는 save-all이 없어("Unknown or incomplete command") 저장 없이 백업이 성공으로 나왔다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_live_backup_fails_when_the_server_rejects_the_save() {
        let (state, id, tmp) = server_with(&live_module(Some("^Saved the game$")), None).await;
        run_fake_server(&state, &id, &tmp, "save-all) echo 'Unknown command';;").await;
        let err = take_backup(&state, &id, BackupKind::Manual).await.err().expect("rejected save");
        let body = crate::ipc::internal::into_internal(err).await.body;
        assert_eq!(body["error_code"], "backup_save_rejected");
        assert_eq!(body["command"], "save-all");
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert!(backup::list(&dir).is_empty(), "no backup of an unsaved world");
        let log = tmp.path().join("server/commands.log");
        assert_eq!(std::fs::read_to_string(log).unwrap(), "save-off\nsave-all\nsave-on\n", "autosave is turned back on");
    }

    /// 저장 완료 줄이 한도 안에 나오지 않으면 백업을 만들지 않는다 (`backup_save_unconfirmed`)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_live_backup_fails_when_the_save_never_finishes() {
        let (state, id, tmp) = server_with(&live_module(Some("^Saved the game$")), None).await;
        run_fake_server(&state, &id, &tmp, "save-all) echo 'Saving the game';;").await;
        let err = take_backup(&state, &id, BackupKind::Manual).await.err().expect("unconfirmed save");
        assert_eq!(code(err).await, "backup_save_unconfirmed");
        let dir = state.supervisor.read().await.instance_store.instance_dir(&id).join("backups");
        assert!(backup::list(&dir).is_empty());
        assert!(std::fs::read_to_string(tmp.path().join("server/commands.log")).unwrap().ends_with("save-on\n"));
    }
}
