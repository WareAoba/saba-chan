//! 예약 작업 실행기: 30초마다 때가 된 작업을 돌리고, 결과를 인스턴스 폴더의 이력에 남긴다.
//! 복원한 뒤 오래 돈 서버의 복원 전 안전 백업도 이때 치운다.
//! 시작·정지·재시작·백업은 HTTP 핸들러와 같은 서비스 함수(`service::lifecycle`·`service::backup`)를 부른다

use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path as FsPath;
use std::sync::Mutex;

use super::{Clock, ScheduledTask, TaskAction};
use crate::backup::{self, BackupKind};
use crate::instance::lock::InstanceLock;
use crate::service::backup::{announce, backups_dir, is_running, run_command, take_backup};
use super::save_command_of;
use crate::service::server_update;
use crate::ipc::IPCServer;

/// 실행기의 시계와 데몬 시작 시각 (화면에 다음 실행 시각을 보일 때 읽는다)
static RUNNER: std::sync::OnceLock<(chrono::NaiveDateTime, std::sync::Arc<Mutex<Clock>>)> = std::sync::OnceLock::new();

/// 작업마다 다음 실행 시각 (유닉스 초, 꺼졌거나 다시 돌 일이 없으면 null). 보는 쪽이 자기 시간대로 보인다
pub(crate) fn next_runs(id: &str, schedule: &[ScheduledTask]) -> Vec<Option<i64>> {
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

/// 일정 순서대로 작업마다 실행 이력 (`schedule-history.json`)
pub(crate) fn history_of(instance_dir: &FsPath, id: &str, schedule: &[ScheduledTask]) -> Vec<Vec<crate::scheduler::Run>> {
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut history = crate::scheduler::read_history(&instance_dir.join(crate::scheduler::HISTORY_FILE));
    schedule.iter().map(|task| history.remove(&crate::scheduler::history_key(id, task)).unwrap_or_default()).collect()
}

/// 이력 파일을 읽고 고쳐 쓰는 동안 쥔다 (같은 인스턴스의 두 작업이 함께 끝나도 한 건을 잃지 않게)
static HISTORY_LOCK: Mutex<()> = Mutex::new(());

/// 예약 작업 한 번의 결과. 이유 코드는 실패·건너뜀에만 있다
type TaskOutcome = (crate::scheduler::RunResult, Option<String>);

fn done() -> Option<TaskOutcome> {
    Some((crate::scheduler::RunResult::Ok, None))
}

fn failed(code: Option<String>) -> Option<TaskOutcome> {
    Some((crate::scheduler::RunResult::Failed, code))
}

fn skipped(code: &str) -> Option<TaskOutcome> {
    Some((crate::scheduler::RunResult::Skipped, Some(code.to_string())))
}

/// 서비스 함수의 결과 → 작업 결과. 실패면 그 `error_code`를 이유로 남긴다.
/// 사용자의 정지·시작이 끊은 것은 실패가 아니라 건너뜀이다 (직접 한 조작이 알림함 실패 카드가 되지 않게)
fn outcome_of(result: &crate::service::Outcome) -> Option<TaskOutcome> {
    match result {
        Ok(_) => done(),
        Err(failure) => match failure.code() {
            Some(code) if matches!(code.as_str(), "restart_cancelled" | "start_cancelled") => skipped(&code),
            code => failed(code),
        },
    }
}

/// 한 건을 인스턴스 폴더의 이력에 남긴다. 일정에서 사라진 작업의 이력은 이때 치운다 (꺼 둔 작업의 것은 둔다). 새로 남겼으면 true.
/// 경로와 키만 감독자 잠금 안에서 모으고, 파일 읽기·쓰기는 Tokio 작업 스레드 밖(`spawn_blocking`)에서 한다
async fn record_run(state: &IPCServer, id: &str, key: &str, run: crate::scheduler::Run) -> bool {
    let (name, dir, keys) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(id) else { return false };
        let keys: HashSet<String> = instance.schedule.iter().map(|t| crate::scheduler::history_key(id, t)).collect();
        (instance.name.clone(), supervisor.instance_store.instance_dir(id), keys)
    };
    let key = key.to_string();
    tokio::task::spawn_blocking(move || {
        if !dir.is_dir() {
            return false;
        }
        let file = dir.join(crate::scheduler::HISTORY_FILE);
        let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut history = crate::scheduler::read_history(&file);
        history.retain(|k, _| keys.contains(k) || *k == key);
        if !crate::scheduler::push_run(&mut history, &key, run) {
            return false;
        }
        if let Err(e) = crate::scheduler::write_history(&file, &history) {
            tracing::warn!("Could not save the schedule history of '{}': {}", name, e);
        }
        true
    })
    .await
    .unwrap_or(false)
}

/// 켜 둔 예약 한 번의 다음 실행 (`next`는 Unix 초, `index`는 그 인스턴스 일정 안의 순서)
pub(crate) struct Upcoming<'a> {
    pub next: i64,
    pub instance: &'a crate::instance::ServerInstance,
    pub index: usize,
    pub task: &'a ScheduledTask,
}

/// 모든 인스턴스의 켜 둔 예약의 다음 실행을 가까운 순으로 (같은 시각은 인스턴스 이름, 일정 순서로).
/// 화면(`GET /api/schedules/upcoming`)과 하트비트가 이것을 잘라 쓴다
pub(crate) fn upcoming(instances: &[crate::instance::ServerInstance]) -> Vec<Upcoming<'_>> {
    let mut rows: Vec<Upcoming> = instances
        .iter()
        .flat_map(|instance| {
            next_runs(&instance.id, &instance.schedule)
                .into_iter()
                .zip(&instance.schedule)
                .enumerate()
                .filter_map(move |(index, (next, task))| Some(Upcoming { next: next?, instance, index, task }))
        })
        .collect();
    rows.sort_by(|a, b| a.next.cmp(&b.next).then_with(|| a.instance.name.cmp(&b.instance.name)).then_with(|| a.index.cmp(&b.index)));
    rows
}

/// 이만큼 안에 함께 도는 무거운 작업(백업·재시작·업데이트)은 겹친다고 알린다
const CLASH_WINDOW_SECS: i64 = 5 * 60;

/// 겹치는 무거운 작업 묶음 (`rows`의 위치, 둘 이상인 것만): 시각 순으로 훑어 앞 것과 `CLASH_WINDOW_SECS` 안이면 한 묶음이다
pub(crate) fn clash_groups(rows: &[Upcoming]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let heavy = (0..rows.len()).filter(|i| matches!(rows[*i].task.action, TaskAction::Backup | TaskAction::Restart | TaskAction::Update));
    for i in heavy {
        match groups.last_mut() {
            Some(group) if rows[i].next - rows[*group.last().expect("groups are never empty")].next <= CLASH_WINDOW_SECS => group.push(i),
            _ => groups.push(vec![i]),
        }
    }
    groups.retain(|group| group.len() > 1);
    groups
}

/// 하트비트의 예약 목록 `[{instanceId, action, next}]` (`next`는 Unix 초): 켜 둔 예약의 다음 실행을 가까운 순으로 `limit`개까지.
/// 릴레이가 알림 채널 맨 위 글에 보인다
pub fn upcoming_runs(instances: &[crate::instance::ServerInstance], limit: usize) -> Vec<Value> {
    upcoming(instances)
        .into_iter()
        .take(limit)
        .map(|run| json!({ "instanceId": run.instance.id, "action": run.task.action, "next": run.next }))
        .collect()
}

/// 데몬이 끝날 때까지 30초마다 예약 작업을 확인한다
pub fn spawn_scheduler(state: IPCServer) {
    tokio::spawn(async move {
        let started = chrono::Local::now().naive_local();
        let clock = RUNNER.get_or_init(|| (started, std::sync::Arc::new(Mutex::new(Clock::default())))).1.clone();
        {
            let supervisor = state.supervisor.read().await;
            let store = &supervisor.instance_store;
            let saved = store.list().iter().flat_map(|i| crate::scheduler::read_runs(&store.instance_dir(&i.id).join(crate::scheduler::RUNS_FILE))).collect();
            clock.lock().unwrap_or_else(|e| e.into_inner()).restore(saved);
        }
        record_missed(&state, &clock, started).await;
        // 복원 전 백업을 이미 살핀 (인스턴스, 켠 시각). 한 번 켠 뒤로 한 번만 살핀다
        let mut swept: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        loop {
            tokio::select! {
                _ = state.shutdown_token.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            }
            sweep_restore_safeties(&state, &mut swept).await;
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
            // 코어 업데이트가 기다리는 동안 새 예약 백업·업데이트는 미룬다: 잦은 예약이 업데이트를 끝없이 미루지 않게 (13절)
            let update_waiting = state.update_state.core_update_waiting();
            {
                let mut c = clock.lock().unwrap_or_else(|e| e.into_inner());
                let keys: HashSet<String> = tasks.iter().map(|(id, t)| crate::scheduler::task_key(id, t)).collect();
                c.retain(&keys, now);
                for (id, task) in tasks {
                    if update_waiting && crate::scheduler::waits_for_update(&task.action) {
                        continue;
                    }
                    let key = crate::scheduler::task_key(&id, &task);
                    if let Some(fire) = c.due(&key, &task, started, now) {
                        due.push((key, id, task, fire));
                    }
                }
            }
            save_runs(&state, &clock).await;
            for (key, id, task, fire) in due {
                let state = state.clone();
                let clock = clock.clone();
                tokio::spawn(async move {
                    run_task(&state, &id, &task, fire, &key).await;
                    clock.lock().unwrap_or_else(|e| e.into_inner()).finish(&key, fire);
                    save_runs(&state, &clock).await;
                });
            }
        }
    });
}

/// 데몬이 꺼져 있던 사이 지나간 시각 작업을 이력에 건너뜀으로 남긴다 (그 차례는 따라잡지 않는다)
async fn record_missed(state: &IPCServer, clock: &Mutex<Clock>, started: chrono::NaiveDateTime) {
    let missed: Vec<(String, String, i64)> = {
        let supervisor = state.supervisor.read().await;
        let clock = clock.lock().unwrap_or_else(|e| e.into_inner());
        supervisor
            .instance_store
            .list()
            .iter()
            .flat_map(|i| i.schedule.iter().map(move |t| (i.id.clone(), t)))
            .filter_map(|(id, task)| {
                let key = crate::scheduler::task_key(&id, task);
                let at = crate::scheduler::instant(&chrono::Local, clock.missed(&key, task, started)?)?.timestamp();
                Some((id, key, at))
            })
            .collect()
    };
    for (id, key, at) in missed {
        let run = crate::scheduler::Run { at, result: crate::scheduler::RunResult::Skipped, code: Some("daemon_off".into()), seconds: 0 };
        record_run(state, &id, &key, run).await;
    }
}

/// 마지막 실행 시각이 바뀌었으면 인스턴스 폴더마다 남긴다: 데몬을 다시 켜도 주기 작업이 이어 센다.
/// 쓸 내용만 감독자 잠금 안에서 모으고, 쓰기는 `spawn_blocking`에서 한다
async fn save_runs(state: &IPCServer, clock: &Mutex<Clock>) {
    let Some(runs) = clock.lock().unwrap_or_else(|e| e.into_inner()).take_changed() else { return };
    let writes: Vec<(String, std::path::PathBuf, std::collections::HashMap<String, _>)> = {
        let supervisor = state.supervisor.read().await;
        let store = &supervisor.instance_store;
        store
            .list()
            .iter()
            .map(|instance| {
                let of_instance = instance
                    .schedule
                    .iter()
                    .map(|t| crate::scheduler::task_key(&instance.id, t))
                    .filter_map(|k| runs.get(&k).map(|at| (k, *at)))
                    .collect();
                (instance.name.clone(), store.instance_dir(&instance.id), of_instance)
            })
            .collect()
    };
    let wrote = tokio::task::spawn_blocking(move || {
        for (name, dir, of_instance) in writes {
            let file = dir.join(crate::scheduler::RUNS_FILE);
            // 예약이 없는 인스턴스에는 파일을 만들지 않고, 지워진 인스턴스의 폴더를 되살리지 않는다
            if (of_instance.is_empty() && !file.exists()) || !dir.is_dir() {
                continue;
            }
            if let Err(e) = crate::scheduler::write_runs(&file, &of_instance) {
                tracing::warn!("Could not save the schedule state of '{}': {}", name, e);
            }
        }
    })
    .await;
    if let Err(e) = wrote {
        tracing::warn!("Saving the schedule state stopped: {}", e);
    }
}

/// 복원한 뒤 켠 서버가 15분 넘게 돌면 복원 직전 안전 백업을 치운다 (그동안 접속해 확인한다고 본다).
/// 서버를 켜기 전이나 켠 지 15분 안에는 남겨 둔다. 켠 시각은 데몬이 다시 떠도 그대로라 상태를 따로 저장하지 않는다
async fn sweep_restore_safeties(state: &IPCServer, swept: &mut std::collections::HashMap<String, u64>) {
    // 지금 도는 서버가 켠 시각(초). 켜져 있지 않거나 켠 지 15분이 안 됐으면 None
    async fn long_running(state: &IPCServer, id: &str, now: u64) -> Option<u64> {
        let supervisor = state.supervisor.read().await;
        if !supervisor.is_running(id).await {
            return None;
        }
        let started = supervisor.tracker.get_start_time(id)?;
        (now >= started.saturating_add(backup::RESTORE_SAFETY_GRACE_SECS)).then_some(started)
    }
    let now = saba_runner::now();
    let ids: Vec<String> = state.supervisor.read().await.instance_store.list().iter().map(|i| i.id.clone()).collect();
    for id in ids {
        match long_running(state, &id, now).await {
            Some(started) if swept.get(&id) != Some(&started) => {}
            _ => continue,
        }
        // 복원·백업·업데이트가 잡고 있으면 다음 차례에 다시 본다
        let Some(_guard) = InstanceLock::take(&id) else { continue };
        // 정지가 진행 중이면 다음 차례로 미루고, 지우는 동안에는 정지를 붙잡아 둔다
        let gate = state.supervisor.read().await.runtime.stop_gate(&id);
        let Some(_stopping) = gate.try_lock_owned().ok() else { continue };
        let Some(started) = long_running(state, &id, saba_runner::now()).await else { continue };
        let Ok(dir) = backups_dir(state, &id).await else { continue };
        // 폴더를 읽지 못하거나 작업이 실패하면 빈 목록으로 보지 않고 다음 차례에 다시 시도한다
        let listing = {
            let dir = dir.clone();
            tokio::task::spawn_blocking(move || backup::list_checked(&dir)).await
        };
        let list = match listing {
            Ok(Ok(list)) => list,
            Ok(Err(e)) => {
                tracing::warn!("Could not list backups of '{}' to clear restore safety backups; will retry: {}", id, e);
                continue;
            }
            Err(e) => {
                tracing::warn!("Listing backups of '{}' failed; will retry: {}", id, e);
                continue;
            }
        };
        let expired = backup::expired_restore_safeties(&list, started, saba_runner::now());
        let mut removed = 0;
        for name in &expired {
            // 하나씩 지우기 직전에 서버가 그대로 같은 실행인지 다시 본다: 그 사이 멈추거나 다시 켜졌으면 남긴다
            if long_running(state, &id, saba_runner::now()).await != Some(started) {
                break;
            }
            let (dir, name) = (dir.clone(), name.clone());
            if matches!(tokio::task::spawn_blocking(move || backup::delete(&dir, &name)).await, Ok(Ok(()))) {
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!("Removed {} restore safety backup(s) of '{}' after the server ran {} minutes", removed, id, backup::RESTORE_SAFETY_GRACE_SECS / 60);
        }
        // 못 지운 것(파일 잠금, 도중에 멈춤 등)이 있으면 처리한 것으로 적지 않아 다음 차례(30초 뒤)에 다시 본다
        if removed == expired.len() {
            swept.insert(id, started);
        } else {
            tracing::warn!("Could not remove all restore safety backups of '{}'; will retry", id);
        }
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
        let _ = announce(state, id, &catalog.t(message_key, &[("minutes", &minutes.to_string())])).await;
    }
    let wait = (fire - chrono::Local::now().naive_local()).to_std().unwrap_or_default();
    tokio::time::sleep(wait).await;
    let alive = still_scheduled(state, id, task_key).await;
    if !alive {
        tracing::info!("Scheduled task for '{}' was removed or disabled during its countdown; skipping", id);
    }
    alive
}

/// 꺼진 서버가 가장 새 백업 뒤에 돈 적이 있는가: 가장 최근 세션 로그가 그 백업보다 뒤에 쓰였으면.
/// 세션 로그가 없으면(돈 적이 없다) false. 꺼진 PC에서 밀린 예약 백업이 데몬이 켜진 직후에 지난 실행을 담게 한다
async fn ran_since_last_backup(state: &IPCServer, id: &str) -> bool {
    let instance_dir = state.supervisor.read().await.instance_store.instance_dir(id);
    let check = move || {
        let wrote = crate::supervisor::managed_process::latest_session_log(&instance_dir)?.metadata().ok()?.modified().ok()?;
        let wrote = wrote.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        Some(backup::list(&backup::backups_dir(&instance_dir)).iter().map(|b| b.created).max().is_none_or(|made| wrote > made))
    };
    tokio::task::spawn_blocking(check).await.ok().flatten().unwrap_or(false)
}

async fn run_task(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) {
    // 감사 로그: 예약 작업이 한 일은 출처 schedule
    // 갈래마다 큰 핸들러 future를 품으므로 힙에 둔다 (디버그 빌드 스택 넘침)
    let called = chrono::Local::now().naive_local();
    let outcome = crate::audit::with_origin(crate::audit::Origin::new("schedule"), Box::pin(run_task_now(state, id, task, fire, key))).await;
    // 예고 중에 지우거나 끈 작업은 남기지 않는다
    let Some((result, code)) = outcome else { return };
    let Some(at) = crate::scheduler::instant(&chrono::Local, fire).map(|t| t.timestamp()) else { return };
    // 예고를 기다린 시간은 빼고 잰다
    let seconds = (chrono::Local::now().naive_local() - called.max(fire)).num_seconds().max(0) as u64;
    if !record_run(state, id, key, crate::scheduler::Run { at, result, code: code.clone(), seconds }).await {
        return;
    }
    let kind = match result {
        crate::scheduler::RunResult::Ok => "schedule.ran",
        crate::scheduler::RunResult::Failed => "schedule.failed",
        crate::scheduler::RunResult::Skipped => return,
    };
    let name = state.supervisor.read().await.instance_store.get(id).map(|i| i.name.clone()).unwrap_or_default();
    let mut data = json!({ "instanceId": id, "instance": name, "action": task.action, "seconds": seconds });
    if let Some(code) = code {
        data["code"] = Value::from(code);
    }
    crate::events::emit(kind, data);
}

/// 작업을 하고 결과를 돌려준다. 예고 중에 작업이 지워지거나 꺼졌으면 None
async fn run_task_now(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) -> Option<TaskOutcome> {
    let running = is_running(state, id).await;
    match task.action {
        TaskAction::Backup => {
            // 꺼져 있고 마지막 백업 뒤로 돈 적도 없으면 바뀐 게 없으니 건너뛰되, 건너뛴 것도 감사 로그에 남긴다
            let (status, code, outcome) = if !running && !ran_since_last_backup(state, id).await {
                tracing::info!("Scheduled backup of '{}' skipped: the server is not running and has not run since the last backup", id);
                (409, None, skipped("not_running"))
            } else {
                match take_backup(state, id, BackupKind::Scheduled).await {
                    Ok(Some(_)) => (200, None, done()),
                    Ok(None) => (204, None, skipped("backup_unsupported")),
                    // 다른 백업·복원·설치가 잠금을 쥐었다: 사용자가 직접 한 작업이니 실패로 알리지 않는다
                    Err(failure) if failure.code().as_deref() == Some("backup_busy") => {
                        tracing::info!("Scheduled backup of '{}' skipped: another task holds the instance", id);
                        (failure.status.as_u16(), failure.code(), skipped("backup_busy"))
                    }
                    Err(failure) => {
                        tracing::warn!("Scheduled backup of '{}' failed: {}", id, failure.body["error"]);
                        let code = failure.code();
                        (failure.status.as_u16(), code.clone(), failed(code))
                    }
                }
            };
            crate::audit::record(crate::audit::Event {
                action: "backup",
                instance_id: id,
                ok: status < 300,
                status,
                code: code.as_deref(),
                ..Default::default()
            });
            outcome
        }
        TaskAction::Announce if running => {
            match announce(state, id, task.message.as_deref().unwrap_or_default()).await {
                Ok(()) => done(),
                Err(code) => failed(Some(code)),
            }
        }
        // 예고 시작 때 꺼져 있었어도 재시작 시각에 켜져 있으면 다시 시작한다 (정지와 같다)
        TaskAction::Restart => {
            if !count_down(state, id, task, fire, key, "schedule.restart_warning", running).await {
                return None;
            }
            if !is_running(state, id).await {
                return failed(Some("not_running".into()));
            }
            tracing::info!("Scheduled restart of '{}'", id);
            let result = Box::pin(crate::service::lifecycle::restart(state, id, false)).await;
            if let Err(failure) = &result {
                tracing::warn!("Scheduled restart of '{}' failed: {}", id, failure.body);
            }
            outcome_of(&result)
        }
        TaskAction::Update => Box::pin(run_update(state, id, task, fire, key)).await,
        TaskAction::Start if !running => {
            tracing::info!("Scheduled start of '{}'", id);
            // 백업·업데이트가 잠금을 쥐고 있으면 끝나기를 기다렸다 켠다. 그사이 사용자가 켜거나 끄거나, 서버나 이 예약을 지우거나 끄면 그만둔다
            let generation = state.supervisor.read().await.run_generation_of(id);
            let wanted = || async { still_scheduled(state, id, key).await && state.supervisor.read().await.run_generation_of(id) == generation };
            let Some(result) = Box::pin(crate::service::lifecycle::start_when_free(state, id, generation, wanted)).await else {
                tracing::info!("Scheduled start of '{}' skipped: started, stopped or removed while waiting", id);
                return skipped("start_cancelled");
            };
            if let Err(failure) = &result {
                tracing::warn!("Scheduled start of '{}' failed: {}", id, failure.body);
            }
            outcome_of(&result)
        }
        // 예고 시작 때 꺼져 있었어도 정지 시각에 켜져 있으면 끈다 (같은 시각 앞의 예약 시작 등)
        TaskAction::Stop => {
            if !count_down(state, id, task, fire, key, "schedule.stop_warning", running).await {
                return None;
            }
            if !is_running(state, id).await {
                return skipped("not_running");
            }
            tracing::info!("Scheduled stop of '{}'", id);
            // 예약 작업은 강제 종료하지 않는다 (저장 후 종료를 못 하는 서버는 그대로 둔다)
            let result = Box::pin(crate::service::lifecycle::stop(state, id, false)).await;
            if let Err(failure) = &result {
                tracing::warn!("Scheduled stop of '{}' failed: {}", id, failure.body);
            }
            outcome_of(&result)
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
            let Some(cmd) = cmd else { return skipped("save_unsupported") };
            match run_command(state, id, &cmd, json!({})).await {
                Ok(()) => done(),
                Err(code) => {
                    tracing::warn!("Scheduled save of '{}' failed: {}", id, code);
                    failed(Some(code))
                }
            }
        }
        // 익스텐션 명령은 서버가 꺼져 있어도 돈다: 켜짐 여부는 명령 실행이 판단하고, 결과는 감사 로그에 남는다.
        // 꺼져 있어 못 한 것은 예약 저장처럼 건너뜀이다
        TaskAction::Command => {
            let name = task.command.as_deref().unwrap_or_default();
            let args = task.args.clone().map(Value::Object).unwrap_or(Value::Null);
            match run_command(state, id, name, args).await {
                Ok(()) => done(),
                Err(code) if code == "server_not_running" => skipped("not_running"),
                Err(code) => {
                    tracing::warn!("Scheduled command '{}' on '{}' failed: {}", name, id, code);
                    failed(Some(code))
                }
            }
        }
        TaskAction::Start => skipped("already_running"),
        TaskAction::Announce | TaskAction::Save => skipped("not_running"),
    }
}

/// 업데이트가 있으면: (켜져 있으면) 예고 → 정지 → 업데이트(업데이트 전 백업 포함) → 다시 시작.
/// 꺼져 있어도 정한 시각까지는 기다린다 (예고 시각에 바로 시작하지 않게)
async fn run_update(state: &IPCServer, id: &str, task: &ScheduledTask, fire: chrono::NaiveDateTime, key: &str) -> Option<TaskOutcome> {
    // 인스턴스가 없으면 업데이트할 것도 없다
    let check = crate::service::server_update::apply::server_update_status(state, id).await.unwrap_or_default();
    if check.get("update_available").and_then(Value::as_bool) != Some(true) {
        if check.get("reason").and_then(Value::as_str) == Some("check_failed") {
            tracing::warn!("Scheduled update of '{}': update check failed: {}", id, check["error"]);
            return failed(Some("check_failed".into()));
        }
        return skipped("no_update");
    }
    let running_now = is_running(state, id).await;
    if !count_down(state, id, task, fire, key, "schedule.update_warning", running_now).await {
        return None;
    }
    tracing::info!("Scheduled update of '{}'", id);
    // 디스코드·자동 업데이트가 같은 서버에 걸어 둔 업데이트와 겹치지 않게 한다
    let Some(_mark) = server_update::Pending::take(id) else {
        tracing::info!("Scheduled update of '{}': another update is already under way", id);
        return skipped("update_in_progress");
    };
    if server_update::stop_update_start(state, id).await {
        done()
    } else {
        failed(Some("update_failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::handlers::schedule::get_schedule;
    use crate::service::backup::tests::{live_module, server_with, NO_BACKUP};
    use crate::service::backup::tests::WORK_DEADLINE;
    #[cfg(unix)]
    use crate::service::backup::tests::run_fake_server;
    use axum::extract::{Path, State};

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

    // 예약이 실제로 돌았는지, 실패했는지를 일정 조회가 작업마다 알려 준다 (새것부터)
    #[tokio::test]
    async fn a_scheduled_run_leaves_its_result_in_the_schedule_history() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let restart: ScheduledTask = serde_json::from_value(json!({"action": "restart", "at": "04:00"})).unwrap();
        let backup: ScheduledTask = serde_json::from_value(json!({"action": "backup", "every_minutes": 60})).unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut instance = sup.instance_store.get(&id).unwrap().clone();
            instance.schedule = vec![restart.clone(), backup.clone()];
            sup.instance_store.update(&id, instance).unwrap();
        }
        let mut events = crate::events::bus().subscribe(None);
        let fire = chrono::Local::now().naive_local() - chrono::Duration::seconds(5);
        run_task(&state, &id, &restart, fire, &crate::scheduler::task_key(&id, &restart)).await;
        run_task(&state, &id, &backup, fire, &crate::scheduler::task_key(&id, &backup)).await;

        let body = crate::ipc::internal::into_internal(get_schedule(Path(id.clone()), State(state.clone())).await).await.body;
        let at = crate::scheduler::instant(&chrono::Local, fire).unwrap().timestamp();
        assert_eq!(body["history"][0], json!([{ "at": at, "result": "failed", "code": "not_running", "seconds": 0 }]), "{}", body);
        assert_eq!(body["history"][1][0]["result"], "skipped", "a stopped server has nothing new to back up: {}", body);
        assert_eq!(body["history"][1][0]["code"], "not_running");

        let seen: Vec<_> = std::iter::from_fn(|| events.rx.try_recv().ok()).collect();
        let failed: Vec<_> = seen.iter().filter(|e| e.kind == "schedule.failed" && e.data["instanceId"] == id.as_str()).collect();
        assert_eq!(failed.len(), 1, "only the failed restart is announced as failed");
        assert_eq!(failed[0].data["action"], "restart");
        assert_eq!(failed[0].data["code"], "not_running");
    }

    // 꺼 둔 서버의 예약 명령은 예약 저장처럼 건너뛴다: 직접 끈 것이 알림함의 실패 카드가 되지 않게
    #[tokio::test]
    async fn a_scheduled_command_on_a_stopped_server_is_skipped_not_failed() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let task: ScheduledTask = serde_json::from_value(json!({"action": "command", "command": "save-all", "every_minutes": 60})).unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut instance = sup.instance_store.get(&id).unwrap().clone();
            instance.schedule = vec![task.clone()];
            sup.instance_store.update(&id, instance).unwrap();
        }
        let mut events = crate::events::bus().subscribe(None);
        let fire = chrono::Local::now().naive_local() - chrono::Duration::seconds(5);
        run_task(&state, &id, &task, fire, &crate::scheduler::task_key(&id, &task)).await;

        let body = crate::ipc::internal::into_internal(get_schedule(Path(id.clone()), State(state.clone())).await).await.body;
        assert_eq!(body["history"][0][0]["result"], "skipped", "{}", body);
        assert_eq!(body["history"][0][0]["code"], "not_running", "{}", body);
        let seen: Vec<_> = std::iter::from_fn(|| events.rx.try_recv().ok()).collect();
        assert!(!seen.iter().any(|e| e.kind == "schedule.failed" && e.data["instanceId"] == id.as_str()), "{seen:?}");
    }

    // 예약 재시작·시작을 사용자가 정지·시작으로 끊은 것은 실패가 아니다 (알림함의 실패 카드가 되지 않게 건너뜀으로)
    #[test]
    fn a_scheduled_run_cut_short_by_the_user_is_skipped() {
        for code in ["restart_cancelled", "start_cancelled"] {
            let cut: crate::service::Outcome = Err(crate::service::Failure::coded(axum::http::StatusCode::CONFLICT, code, "x"));
            assert_eq!(outcome_of(&cut), skipped(code), "{code}");
        }
        let broken: crate::service::Outcome = Err(crate::service::Failure::coded(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "start_failed", "x"));
        assert_eq!(outcome_of(&broken), failed(Some("start_failed".into())));
    }

    // 다른 백업·복원과 겹친 예약 백업은 그 작업이 같은 내용을 담으니 건너뛴다: 실패로 알리지 않는다
    #[tokio::test]
    async fn a_scheduled_backup_that_meets_a_running_backup_is_skipped_without_a_failure_notice() {
        let (state, id, tmp) = server_with(&live_module(None), None).await;
        std::fs::create_dir_all(tmp.path().join("server/world")).unwrap();
        let instance_dir = state.supervisor.read().await.instance_store.instance_dir(&id);
        let sessions = crate::supervisor::managed_process::sessions_dir(&instance_dir);
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("20260101-000000.log"), "Server stopped\n").unwrap();
        let task: ScheduledTask = serde_json::from_value(json!({"action": "backup", "every_minutes": 60})).unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut instance = sup.instance_store.get(&id).unwrap().clone();
            instance.schedule = vec![task.clone()];
            sup.instance_store.update(&id, instance).unwrap();
        }
        let _busy = crate::instance::lock::InstanceLock::take(&id).expect("free");
        let mut events = crate::events::bus().subscribe(None);
        let fire = chrono::Local::now().naive_local() - chrono::Duration::seconds(5);
        run_task(&state, &id, &task, fire, &crate::scheduler::task_key(&id, &task)).await;

        let body = crate::ipc::internal::into_internal(get_schedule(Path(id.clone()), State(state.clone())).await).await.body;
        assert_eq!(body["history"][0][0]["result"], "skipped", "{}", body);
        assert_eq!(body["history"][0][0]["code"], "backup_busy", "{}", body);
        let seen: Vec<_> = std::iter::from_fn(|| events.rx.try_recv().ok()).filter(|e| e.data["instanceId"] == id.as_str()).collect();
        assert!(!seen.iter().any(|e| e.kind == "backup.done" || e.kind == "schedule.failed"), "{seen:?}");
    }

    // 작업을 꺼 두어도 같은 작업이다: 이력이 그대로 보이고, 그사이 다른 작업이 돌아도 지워지지 않는다
    #[tokio::test]
    async fn turning_a_task_off_keeps_its_history() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let restart: ScheduledTask = serde_json::from_value(json!({"action": "restart", "at": "04:00"})).unwrap();
        let stop: ScheduledTask = serde_json::from_value(json!({"action": "stop", "at": "05:00"})).unwrap();
        let set = |schedule: Vec<ScheduledTask>| {
            let (state, id) = (state.clone(), id.clone());
            async move {
                let mut sup = state.supervisor.write().await;
                let mut instance = sup.instance_store.get(&id).unwrap().clone();
                instance.schedule = schedule;
                sup.instance_store.update(&id, instance).unwrap();
            }
        };
        let history = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(get_schedule(Path(id), State(state)).await).await.body["history"].clone() }
        };
        set(vec![restart.clone(), stop.clone()]).await;
        let fire = chrono::Local::now().naive_local() - chrono::Duration::seconds(5);
        run_task(&state, &id, &restart, fire, &crate::scheduler::task_key(&id, &restart)).await;

        set(vec![ScheduledTask { enabled: false, ..restart.clone() }, stop.clone()]).await;
        assert_eq!(history().await[0][0]["code"], "not_running", "the switched-off task still shows its last run");
        run_task(&state, &id, &stop, fire, &crate::scheduler::task_key(&id, &stop)).await;
        set(vec![restart.clone(), stop.clone()]).await;
        assert_eq!(history().await[0][0]["code"], "not_running", "another task's run did not drop it");
    }

    // 꺼진 PC에서 밀린 주기 백업은 데몬이 켜진 직후(서버는 아직 꺼져 있다)에 돈다: 지난 실행이 바꾼 것을 그때 담아야 한다
    #[tokio::test]
    async fn a_scheduled_backup_of_a_stopped_server_runs_once_when_it_ran_since_the_last_backup() {
        let (state, id, tmp) = server_with(&live_module(None), None).await;
        std::fs::create_dir_all(tmp.path().join("server/world")).unwrap();
        std::fs::write(tmp.path().join("server/world/level.dat"), b"played").unwrap();
        let instance_dir = state.supervisor.read().await.instance_store.instance_dir(&id);
        let sessions = crate::supervisor::managed_process::sessions_dir(&instance_dir);
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("20260101-000000.log"), "Server stopped\n").unwrap();
        let task: ScheduledTask = serde_json::from_value(json!({"action": "backup", "every_minutes": 720})).unwrap();
        let last_status = || crate::audit::log().recent(10, Some(&id)).first().map(|e| e["status"].clone());

        run_task(&state, &id, &task, chrono::Local::now().naive_local(), "k").await;
        assert_eq!(last_status(), Some(json!(200)), "the last run's changes were never backed up");
        let made = backup::list(&backup::backups_dir(&instance_dir));
        assert_eq!(made.iter().map(|b| b.kind).collect::<Vec<_>>(), [BackupKind::Scheduled]);

        run_task(&state, &id, &task, chrono::Local::now().naive_local(), "k").await;
        assert_eq!(last_status(), Some(json!(409)), "nothing ran since that backup");
        assert_eq!(backup::list(&backup::backups_dir(&instance_dir)).len(), 1);
    }

    /// 예약 재시작은 예고를 시작할 때 꺼져 있었어도 실행 시각에 켜져 있으면 다시 시작한다 (예약 정지와 같다)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_scheduled_restart_runs_when_the_server_comes_up_during_the_countdown() {
        let (state, id, tmp) = server_with(&live_module(None), None).await;
        let task: ScheduledTask = serde_json::from_value(json!({"action": "restart", "at": "04:00", "warn_minutes": [10]})).unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.schedule = vec![task.clone()];
            sup.instance_store.update(&id, inst).unwrap();
        }
        let key = crate::scheduler::task_key(&id, &task);
        let fire = chrono::Local::now().naive_local() + chrono::Duration::seconds(2);
        let run = {
            let (state, id, task) = (state.clone(), id.clone(), task.clone());
            tokio::spawn(async move { run_task(&state, &id, &task, fire, &key).await })
        };
        // 예고가 시작된 뒤에 서버가 켜진다
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        run_fake_server(&state, &id, &tmp, "").await;
        tokio::time::timeout(WORK_DEADLINE, run).await.unwrap().unwrap();
        let entries = crate::audit::log().recent(10, Some(&id));
        assert!(entries.iter().any(|e| e["action"] == "restart"), "the restart was skipped: {entries:?}");
    }

    // 예약 시작이 백업·업데이트와 겹치면 잠금이 풀리기를 기다렸다 켠다: 바로 instance_busy로 실패하지 않는다
    #[tokio::test]
    async fn a_scheduled_start_waits_for_a_running_backup() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let task: ScheduledTask = serde_json::from_value(json!({"action": "start", "at": "04:00"})).unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut instance = sup.instance_store.get(&id).unwrap().clone();
            instance.schedule = vec![task.clone()];
            sup.instance_store.update(&id, instance).unwrap();
        }
        let backup = crate::instance::lock::InstanceLock::take(&id).expect("free");
        let fire = chrono::Local::now().naive_local() - chrono::Duration::seconds(5);
        let run = {
            let (state, id, task) = (state.clone(), id.clone(), task.clone());
            let key = crate::scheduler::task_key(&id, &task);
            tokio::spawn(async move { run_task(&state, &id, &task, fire, &key).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert!(!run.is_finished(), "the start gave up while the backup held the instance");
        drop(backup);
        tokio::time::timeout(WORK_DEADLINE, run).await.unwrap().unwrap();
        let body = crate::ipc::internal::into_internal(get_schedule(Path(id.clone()), State(state.clone())).await).await.body;
        assert_ne!(body["history"][0][0]["code"], "instance_busy", "{}", body);
    }

    // 잠금을 기다리는 사이에 지운 예약 시작은 잠금이 풀려도 서버를 켜지 않는다
    #[tokio::test]
    async fn a_scheduled_start_deleted_while_waiting_gives_up() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        let task: ScheduledTask = serde_json::from_value(json!({"action": "start", "at": "04:00"})).unwrap();
        let set_schedule = |schedule: Vec<ScheduledTask>| {
            let (state, id) = (state.clone(), id.clone());
            async move {
                let mut sup = state.supervisor.write().await;
                let mut instance = sup.instance_store.get(&id).unwrap().clone();
                instance.schedule = schedule;
                sup.instance_store.update(&id, instance).unwrap();
            }
        };
        set_schedule(vec![task.clone()]).await;
        let _backup = crate::instance::lock::InstanceLock::take(&id).expect("free");
        let fire = chrono::Local::now().naive_local() - chrono::Duration::seconds(5);
        let run = {
            let (state, id, task) = (state.clone(), id.clone(), task.clone());
            let key = crate::scheduler::task_key(&id, &task);
            tokio::spawn(async move { run_task(&state, &id, &task, fire, &key).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        set_schedule(vec![]).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), run).await.expect("the deleted start kept waiting for the lock").unwrap();
    }

    /// 하트비트의 예약 목록: 켜 둔 것만, 가까운 순으로, 상한까지
    #[test]
    fn upcoming_runs_list_enabled_tasks_soonest_first() {
        let task = |v: Value| serde_json::from_value::<ScheduledTask>(v).unwrap();
        let mut a = crate::instance::ServerInstance::new("a", "plain");
        a.schedule = vec![
            task(json!({ "action": "restart", "every_minutes": 120 })),
            task(json!({ "action": "backup", "every_minutes": 30, "enabled": false })),
        ];
        let mut b = crate::instance::ServerInstance::new("b", "plain");
        b.schedule = vec![task(json!({ "action": "backup", "every_minutes": 60 }))];
        let runs = upcoming_runs(&[a.clone(), b.clone()], 10);
        let ids: Vec<_> = runs.iter().map(|r| (r["instanceId"].as_str().unwrap().to_string(), r["action"].clone())).collect();
        assert_eq!(ids, vec![(b.id.clone(), json!("backup")), (a.id.clone(), json!("restart"))], "{runs:?}");
        assert!(runs.iter().all(|r| r["next"].as_i64().is_some()));
        assert_eq!(upcoming_runs(&[a, b], 1).len(), 1);
    }
}
