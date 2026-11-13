//! 예약 작업 API (A3). 실행기는 `scheduler::runner`
//!
//! - `GET/PUT /api/instance/:id/schedule` — `{schedule: [...], backup_keep}`
//! - `GET /api/schedules/upcoming` — 모든 인스턴스의 다가오는 예약

use super::error;
use crate::service::backup::DEFAULT_KEEP;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

use super::super::IPCServer;
use crate::scheduler::runner::{history_of, next_runs};
use crate::scheduler::{daily_runs, save_command_of, schedule_revision, ScheduledTask, TaskAction};

/// 원격에서 넣는 반복 작업의 가장 짧은 간격(분)과 보관 개수 상한
const REMOTE_MIN_EVERY_MINUTES: u32 = 30;

const REMOTE_MAX_BACKUP_KEEP: u32 = 30;

#[derive(Deserialize)]
pub struct ScheduleBody {
    schedule: Vec<ScheduledTask>,
    #[serde(default)]
    backup_keep: Option<u32>,
    /// GET에서 받은 `revision`. 있으면 그사이 일정이 바뀌었을 때 덮어쓰지 않고 409로 거절한다 (L51)
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Deserialize)]
pub struct UpcomingQuery {
    /// 지금부터 이 시간 안에 도는 것만 (기본 24, 1~720)
    #[serde(default)]
    pub hours: Option<u32>,
}

/// 이만큼 안에 함께 도는 무거운 작업(백업·재시작·업데이트)은 겹친다고 알린다
const CLASH_WINDOW_SECS: i64 = 5 * 60;

/// GET /api/schedules/upcoming?hours= — 모든 인스턴스의 켜 둔 예약의 다음 실행을 가까운 순으로.
/// `{upcoming: [{instance_id, instance, index, task, next, clash}], clashes: [{at, actions}], utc_offset_minutes}`
pub async fn upcoming_schedules(axum::extract::Query(q): axum::extract::Query<UpcomingQuery>, State(state): State<IPCServer>) -> Response {
    let now = chrono::Local::now().timestamp();
    let until = now + i64::from(q.hours.unwrap_or(24).clamp(1, 720)) * 3600;
    let supervisor = state.supervisor.read().await;
    let mut rows: Vec<(i64, &crate::instance::ServerInstance, usize, &ScheduledTask)> = supervisor
        .instance_store
        .list()
        .iter()
        .flat_map(|inst| {
            next_runs(&inst.id, &inst.schedule)
                .into_iter()
                .zip(&inst.schedule)
                .enumerate()
                .filter_map(move |(index, (next, task))| Some((next?, inst, index, task)))
        })
        .filter(|(next, ..)| *next <= until)
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)).then_with(|| a.2.cmp(&b.2)));
    // 무거운 작업을 시각 순으로 훑어, 앞 것과 `CLASH_WINDOW_SECS` 안이면 한 묶음으로 본다
    let heavy: Vec<usize> = (0..rows.len()).filter(|i| matches!(rows[*i].3.action, TaskAction::Backup | TaskAction::Restart | TaskAction::Update)).collect();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in heavy {
        match groups.last_mut() {
            Some(group) if rows[i].0 - rows[*group.last().expect("groups are never empty")].0 <= CLASH_WINDOW_SECS => group.push(i),
            _ => groups.push(vec![i]),
        }
    }
    groups.retain(|group| group.len() > 1);
    let clashing: HashSet<usize> = groups.iter().flatten().copied().collect();
    let clashes: Vec<Value> = groups
        .iter()
        .map(|group| {
            let mut actions: Vec<TaskAction> = Vec::new();
            for i in group {
                if !actions.contains(&rows[*i].3.action) {
                    actions.push(rows[*i].3.action);
                }
            }
            json!({ "at": rows[group[0]].0, "actions": actions })
        })
        .collect();
    let upcoming: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(i, (next, inst, index, task))| {
            json!({ "instance_id": inst.id, "instance": inst.name, "index": index, "task": task, "next": next, "clash": clashing.contains(&i) })
        })
        .collect();
    Json(json!({
        "success": true,
        "upcoming": upcoming,
        "clashes": clashes,
        "utc_offset_minutes": chrono::Local::now().offset().local_minus_utc() / 60,
    }))
    .into_response()
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
        // 작업마다 최근 실행 기록 (새것부터, `scheduler::HISTORY_KEEP`건까지). 돈 적이 없으면 빈 배열
        "history": history_of(&supervisor.instance_store.instance_dir(&id), &id, &instance.schedule),
        // 작업 시각(`at`·`date`)은 데몬 PC의 벽시계다: 다른 시간대에서 보는 화면이 그 차이를 알린다
        "utc_offset_minutes": chrono::Local::now().offset().local_minus_utc() / 60,
    }))
    .into_response()
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
    // 원격(디스코드·챗봇)은 새로 넣거나 바꾸는 작업만 제한한다 (이 PC에서 정한 작업은 그대로 다시 보낼 수 있다).
    // 원격 MCP(계정 주인의 개인 에이전트)는 앱과 같다: 액션 계획이 저장 전에 늘 주인에게 묻는다 (사용자 결정 2026-11-05)
    let origin = crate::audit::current();
    if origin.is_relay() && origin.source != crate::actions::MCP_VIA {
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
            // 공지·저장도 모듈의 게임 명령을 돌린다: 그 명령에 같은 규칙을 건다
            if matches!(task.action, TaskAction::Command | TaskAction::Announce | TaskAction::Save) && task.enabled {
                let risk = meta
                    .as_ref()
                    .and_then(|m| crate::commands::find(m, &crate::actions::scheduled_command_name(m, task)?))
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
            Json(json!({ "success": false, "error": "invalid schedule", "error_code": "validation_failed", "details": problems })),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::backup::tests::{server_with, NO_BACKUP};

    // 모든 서버의 다음 예약을 시각 순서로 한 번에 주고, 같은 PC에서 가까운 시각에 겹치는 무거운 작업을 짚는다
    #[tokio::test]
    async fn upcoming_schedules_lists_every_instance_in_time_order_and_marks_heavy_tasks_that_overlap() {
        let (state, id, _tmp) = server_with(NO_BACKUP, None).await;
        // 지금이 몇 시든 같은 순서가 되게 두 시간 뒤를 기준으로 잡는다
        let at = |minutes: i64| (chrono::Local::now() + chrono::Duration::minutes(120 + minutes)).format("%H:%M").to_string();
        let other = {
            let mut sup = state.supervisor.write().await;
            let mut instance = sup.instance_store.get(&id).unwrap().clone();
            instance.schedule = serde_json::from_value(json!([
                { "action": "backup", "at": at(0) },
                { "action": "announce", "at": at(0), "message": "hi" },
                { "action": "start", "at": at(60), "enabled": false },
            ]))
            .unwrap();
            sup.instance_store.update(&id, instance).unwrap();
            let mut other = crate::instance::ServerInstance::new("other", "plain");
            other.schedule = serde_json::from_value(json!([{ "action": "restart", "at": at(3) }, { "action": "update", "at": at(30) }])).unwrap();
            let other_id = other.id.clone();
            sup.instance_store.add(other).unwrap();
            other_id
        };
        let all = |hours| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(upcoming_schedules(axum::extract::Query(UpcomingQuery { hours }), State(state)).await).await.body }
        };
        let body = all(None).await;
        let rows = body["upcoming"].as_array().unwrap();
        let seen: Vec<(&str, &str, bool)> =
            rows.iter().map(|r| (r["instance"].as_str().unwrap(), r["task"]["action"].as_str().unwrap(), r["clash"].as_bool().unwrap())).collect();
        // 꺼 둔 작업은 없고, 공지는 무겁지 않고, 30분 떨어진 업데이트는 겹치지 않는다
        assert_eq!(seen, [("srv", "backup", true), ("srv", "announce", false), ("other", "restart", true), ("other", "update", false)], "{}", body);
        let restart = rows.iter().find(|r| r["task"]["action"] == "restart").unwrap();
        assert_eq!((restart["instance_id"].as_str(), restart["index"].as_u64()), (Some(other.as_str()), Some(0)));
        assert_eq!(body["clashes"].as_array().unwrap().len(), 1, "{}", body);
        assert_eq!(body["clashes"][0]["actions"], json!(["backup", "restart"]));
        assert_eq!(body["clashes"][0]["at"], rows[0]["next"]);
        let soon = all(Some(1)).await;
        assert_eq!(soon["upcoming"], json!([]), "nothing runs within the hour: {}", soon);
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
        // 원격 MCP(계정 주인의 개인 에이전트)는 앱처럼 무엇이든 예약한다 (액션 계획이 늘 주인에게 묻는다, 사용자 결정 2026-11-05)
        let mcp = |body: Value| crate::audit::with_origin(crate::audit::Origin::new(crate::actions::MCP_VIA), put(body));
        assert_eq!(mcp(json!({ "schedule": [update, restart(1)], "backup_keep": 1000 })).await, StatusCode::OK);
        assert_eq!(put(json!({ "schedule": [], "backup_keep": 10 })).await, StatusCode::OK);
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

        // 공지·저장이 돌릴 명령도 같은 규칙이다: 디스코드에 내보내지 않는 명령이면 원격에서 예약하지 못한다
        let hidden = format!(
            "{NO_BACKUP}\n[config]\nannounce_command = \"op\"\n\n[backup]\npaths = [\"world\"]\nsave_command = \"op\"\n\n[[commands.fields]]\nname = \"op\"\nrisk = \"safe\"\nremote = false\n"
        );
        let (state2, id2, _tmp2) = server_with(&hidden, None).await;
        let remote2 = |body: Value| {
            let (state, id) = (state2.clone(), id2.clone());
            crate::audit::with_origin(crate::audit::Origin::new("discord"), async move {
                put_schedule(Path(id), State(state), Json(serde_json::from_value::<ScheduleBody>(body).unwrap())).await.status()
            })
        };
        assert_eq!(remote2(json!({ "schedule": [{ "action": "announce", "at": "04:00", "message": "hi" }] })).await, StatusCode::BAD_REQUEST);
        assert_eq!(remote2(json!({ "schedule": [{ "action": "save", "at": "04:00" }] })).await, StatusCode::BAD_REQUEST);

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
}
