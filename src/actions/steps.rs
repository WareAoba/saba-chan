//! 여러 단계 계획 (I1): "모듈 설치 → 서버 만들기 → 시작"을 확인 한 번으로
//!
//! - 단계 `{id, instance?, input}`. `instance`가 `@N`이면 N번째 단계(1부터)의 `instance.create`가 만들 서버다.
//!   그런 단계는 고정 액션만 된다(모듈 명령·조회는 서버가 있어야 펼쳐진다).
//! - 계획 때 단계마다 검증·판정한다. 실행 때 새 서버를 쓰는 단계는 실제 id로 다시 풀고 다시 판정한다.
//! - 실행은 순서대로, 실패한 단계에서 멈춘다. 새 서버를 쓰는 단계 앞에서는 그 서버 설치가 끝나기를 기다린다.
//! - 실행이 `INLINE`보다 길어지면 남은 단계는 뒤에서 이어 가고 `{running:true, runId}`를 돌려준다.
//!   끝나면 `plan.done` 이벤트를 낸다. 진행은 [`run_status`]로 본다.
//! - 원격에서 만든 서버는 그 길드가 `allowedInstances`를 정해 두었으면 목록에 더한다(만든 길드에서 바로 쓰게).

use super::*;
use std::sync::Arc;

/// 여러 단계 계획의 액션 id (`POST /api/actions/plan`, run의 `:id`)
pub const PLAN_ID: &str = "plan";
pub const MAX_STEPS: usize = 8;
/// 이보다 길면 남은 단계를 뒤에서 이어 간다 (릴레이가 결과를 55초 기다린다)
const INLINE: Duration = Duration::from_secs(40);
/// 새 서버 설치를 기다리는 최대 시간
const PROVISION_WAIT: Duration = Duration::from_secs(30 * 60);
/// 실행 기록을 두는 시간
const RUN_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub(super) enum Step {
    Ready(Resolved),
    /// 앞 단계(`from`, 0부터)가 만든 서버를 쓴다: 실행 때 실제 id로 다시 푼다
    OnNew { id: String, from: usize, input: Value },
}

impl Step {
    /// 관리 단계인가 (원격 관리 실행 횟수는 이런 단계가 있는 계획만 센다)
    pub(super) fn is_admin(&self) -> bool {
        match self {
            Step::Ready(r) => r.kind == Kind::Admin,
            Step::OnNew { id, .. } => find(id).is_some_and(|d| d.kind == Kind::Admin),
        }
    }
}

fn step_error(n: usize, mut e: ActionError) -> ActionError {
    if let Value::Object(m) = &mut e.data {
        if let Some(field) = m.get("field").and_then(Value::as_str) {
            let field = format!("steps[{}].{}", n, field);
            m.insert("field".into(), json!(field));
        }
    } else {
        e.data = json!({});
    }
    e.data["step"] = json!(n + 1);
    e
}

/// `POST /api/actions/plan {steps}`: 단계마다 검증·판정하고 요약을 이어 붙인다 (부작용 없음)
pub async fn plan_steps(server: &IPCServer, steps: &Value, caller: &Caller) -> Result<Value, ActionError> {
    let Some(list) = steps.as_array().filter(|l| !l.is_empty() && l.len() <= MAX_STEPS) else {
        return Err(bad_input("steps", &format!("must be a list of 1 to {} steps", MAX_STEPS)));
    };
    let cat = catalog(&caller.locale);
    let mut planned = Vec::new();
    let mut summary = Vec::new();
    let mut views = Vec::new();
    let mut confirm = list.len() > 1;
    let mut risk = "safe";
    for (n, raw) in list.iter().enumerate() {
        let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
        let input = raw.get("input").cloned().unwrap_or(Value::Null);
        let instance = raw.get("instance").and_then(Value::as_str);
        let (step, r, new_name) = match instance.and_then(|i| i.strip_prefix('@')) {
            Some(target) => {
                let from = target.parse::<usize>().ok().filter(|t| *t >= 1 && *t <= n).map(|t| t - 1);
                let maker = from.map(|f| &list[f]).filter(|m| m.get("id").and_then(Value::as_str) == Some("instance.create"));
                let (Some(from), Some(maker)) = (from, maker) else {
                    return Err(step_error(n, bad_input("instance", "must name an earlier instance.create step")));
                };
                // 서버 이름으로 찾는 라우트·모듈 명령은 서버가 있어야 한다
                if !find(id).is_some_and(|d| d.instance && !d.path.contains(":iname")) {
                    return Err(step_error(n, bad_input("id", "cannot run on a server made in this plan")));
                }
                let module = maker.pointer("/input/module_name").and_then(Value::as_str);
                let mut r = resolve_in(server, id, instance, &input, caller, module).await.map_err(|e| step_error(n, e))?;
                // 아직 없는 서버라 길드 허용 목록은 실행 때 본다
                r.instance = None;
                authorize_in(server, &r, caller, Some(module.unwrap_or_default())).await.map_err(|e| step_error(n, e))?;
                let name = maker.pointer("/input/name").cloned().unwrap_or(Value::Null);
                (Step::OnNew { id: id.to_string(), from, input }, r, Some(name))
            }
            None => {
                let r = resolve(server, id, instance, &input, caller).await.map_err(|e| step_error(n, e))?;
                authorize(server, &r, caller).await.map_err(|e| step_error(n, e))?;
                // 같은 서버를 먼저 끄는 단계가 앞에 있으면 켜짐은 실행 때 본다 (끄고 복원)
                let stopped_first = list[..n].iter().any(|p| p.get("id").and_then(Value::as_str) == Some("server.stop") && p.get("instance").and_then(Value::as_str) == instance);
                if !stopped_first {
                    refuse_restore_while_running(server, &r).await.map_err(|e| step_error(n, e))?;
                }
                (Step::Ready(r.clone()), r, None)
            }
        };
        confirm |= needs_confirm(server, &r.id, r.kind, &r.risk, caller).await;
        if r.risk == "danger" || (r.risk == "caution" && risk == "safe") {
            risk = if r.risk == "danger" { "danger" } else { "caution" };
        }
        let title = translated(&cat, &format!("actions.{}.title", r.id)).unwrap_or_else(|| r.id.clone());
        summary.push(json!({ "labelKey": "actions.field.step", "label": translated(&cat, "actions.field.step").unwrap_or_else(|| "step".into()), "value": format!("{}. {}", n + 1, title) }));
        let mut rows = summarize(server, &r, &caller.locale).await;
        rows.remove(0); // 액션 이름은 단계 줄에 있다
        if let Some(name) = new_name {
            rows.insert(0, json!({ "labelKey": "actions.field.instance", "label": translated(&cat, "actions.field.instance").unwrap_or_else(|| "instance".into()), "value": name }));
        }
        summary.extend(rows);
        views.push(json!({ "n": n + 1, "actionId": r.id, "instance": instance }));
        planned.push(step);
    }
    let plan_id = new_plan_id();
    let view = json!({
        "success": true,
        "planId": plan_id,
        "actionId": PLAN_ID,
        "steps": views,
        "summary": summary,
        "risk": risk,
        "needsConfirm": confirm,
        "expiresAt": expires_at(),
    });
    store_plan(plan_id, StoredPlan::new(PlanBody::Steps(planned), caller, confirm));
    Ok(view)
}

// ── 실행 ────────────────────────────────────────────────

/// 실행 하나의 기록
struct Record {
    owner: String,
    created: Instant,
    steps: Vec<Value>,
    result: Option<Value>,
    /// 응답을 이미 돌려주고 뒤에서 이어 가는 중
    detached: bool,
}

fn runs() -> &'static Mutex<HashMap<String, Arc<Mutex<Record>>>> {
    static RUNS: std::sync::OnceLock<Mutex<HashMap<String, Arc<Mutex<Record>>>>> = std::sync::OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// 길드(로컬이면 출처)당 여러 단계 실행은 하나씩
pub(super) struct Lane(String);

fn lanes() -> &'static Mutex<std::collections::HashSet<String>> {
    static LANES: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    LANES.get_or_init(Default::default)
}


impl Lane {
    pub(super) fn take(caller: &Caller) -> Result<Self, ActionError> {
        let key = match &caller.remote {
            Some(r) => format!("relay|{}", r.guild_id.as_deref().unwrap_or("")),
            None => format!("local|{}", caller.origin.source),
        };
        if !lanes().lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone()) {
            return Err(ActionError::new(409, "plan_running", "Another plan is still running here"));
        }
        Ok(Self(key))
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        lanes().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

fn lock(rec: &Mutex<Record>) -> std::sync::MutexGuard<'_, Record> {
    rec.lock().unwrap_or_else(|e| e.into_inner())
}

/// `lane`은 계획을 꺼내기 전에 잡아 둔 자리다: 꺼낸 뒤에 잡다가 409가 나면 계획만 쓰이고 사라진다 (D-F6)
pub(super) async fn run_steps(server: &IPCServer, steps: Vec<Step>, caller: Caller, lane: Option<Lane>) -> Result<Value, ActionError> {
    let lane = match lane {
        Some(lane) => lane,
        None => Lane::take(&caller)?,
    };
    let run_id = new_plan_id();
    let rows = steps
        .iter()
        .enumerate()
        .map(|(n, s)| {
            let id = match s {
                Step::Ready(r) => r.id.clone(),
                Step::OnNew { id, .. } => id.clone(),
            };
            json!({ "n": n + 1, "actionId": id, "status": "pending" })
        })
        .collect();
    let record = Arc::new(Mutex::new(Record { owner: caller.owner_key(), created: Instant::now(), steps: rows, result: None, detached: false }));
    {
        let mut all = runs().lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|_, r| lock(r).created.elapsed() < RUN_TTL);
        all.insert(run_id.clone(), record.clone());
    }
    let (server2, rec, rid) = (server.clone(), record.clone(), run_id.clone());
    let mut task = tokio::spawn(crate::audit::with_origin(caller.origin.clone(), async move {
        let _lane = lane;
        let outcome = execute_steps(&server2, steps, &caller, &rec, &rid).await;
        let mut r = lock(&rec);
        if r.detached {
            let mut event = json!({ "runId": rid, "ok": outcome["success"], "steps": outcome["steps"], "stoppedAt": outcome["stoppedAt"] });
            if let Some(g) = caller.remote.as_ref().and_then(|w| w.guild_id.clone()) {
                event["guildId"] = json!(g);
            }
            crate::events::emit("plan.done", event);
        }
        r.result = Some(outcome);
    }));
    let finished = tokio::time::timeout(INLINE, &mut task).await.is_ok();
    let mut r = lock(&record);
    match (&r.result, finished) {
        (Some(done), _) => Ok(done.clone()),
        _ => {
            r.detached = true;
            Ok(json!({ "success": true, "running": true, "actionId": PLAN_ID, "runId": run_id, "steps": r.steps }))
        }
    }
}

/// 실행 중이거나 끝난 여러 단계 실행 (`GET /api/actions/runs/:runId`). 그 실행을 시작한 쪽만 본다
pub fn run_status(run_id: &str, caller: &Caller) -> Result<Value, ActionError> {
    let rec = runs().lock().unwrap_or_else(|e| e.into_inner()).get(run_id).cloned();
    let Some(rec) = rec else { return Err(ActionError::new(404, "run_not_found", "No such run")) };
    let r = lock(&rec);
    if r.owner != caller.owner_key() {
        return Err(ActionError::new(403, "plan_not_yours", "This run belongs to another request"));
    }
    Ok(match &r.result {
        Some(done) => json!({ "done": true, "result": done }),
        None => json!({ "done": false, "steps": r.steps }),
    })
}

/// 새 서버 설치가 끝나기를 기다린다 (설치 진행이 없으면 바로)
async fn wait_installed(server: &IPCServer, instance: &str) -> Result<(), ActionError> {
    let Some(name) = server.supervisor.read().await.instance_store.get(instance).map(|i| i.name.clone()) else {
        return Err(ActionError::new(404, "not_found", "The new server is gone"));
    };
    let started = Instant::now();
    loop {
        match server.provision_tracker.get(&name) {
            None => return Ok(()),
            Some(p) if p.done && p.error.is_some() => return Err(ActionError::new(502, "provision_failed", "Installing the server failed")),
            Some(p) if p.done => return Ok(()),
            Some(_) if started.elapsed() > PROVISION_WAIT => return Err(ActionError::new(504, "provision_timeout", "Installing the server took too long")),
            Some(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

/// 원격에서 만든 서버를 그 길드의 허용 목록에 더한다 (목록을 정해 둔 길드만)
pub(super) async fn allow_in_guild(server: &IPCServer, caller: &Caller, instance: &str) {
    let Some(guild) = caller.remote.as_ref().and_then(|w| w.guild_id.clone()) else { return };
    let added = server
        .config_store
        .update_node_settings(&guild, |n| match n.get_mut("allowedInstances").and_then(Value::as_array_mut) {
            Some(list) if !list.iter().any(|v| v.as_str() == Some(instance)) => {
                list.push(json!(instance));
                ((), true)
            }
            _ => ((), false),
        })
        .await;
    if let Err(e) = added {
        tracing::warn!("[actions] could not allow the new server in guild {}: {}", guild, e);
    }
}

/// 실패한 단계 줄: 원격이면 오류 코드만 (경로·원문은 PC 밖으로 내보내지 않는다)
fn failed_row(row: &mut Value, code: &str, message: Option<&str>, caller: &Caller) {
    row["status"] = json!("failed");
    row["code"] = json!(code);
    if let (None, Some(m)) = (&caller.remote, message) {
        row["error"] = json!(m);
    }
}

async fn execute_steps(server: &IPCServer, steps: Vec<Step>, caller: &Caller, rec: &Mutex<Record>, run_id: &str) -> Value {
    let mut created: Vec<Option<String>> = vec![None; steps.len()];
    let mut stopped = None;
    for (n, step) in steps.into_iter().enumerate() {
        lock(rec).steps[n]["status"] = json!("running");
        let resolved = match step {
            Step::Ready(r) => Ok(r),
            Step::OnNew { id, from, input } => match created[from].clone() {
                None => Err(ActionError::new(409, "step_skipped", "The server this step needs was not made")),
                Some(new_id) => match wait_installed(server, &new_id).await {
                    Err(e) => Err(e),
                    Ok(()) => match resolve(server, &id, Some(&new_id), &input, caller).await {
                        Ok(r) => authorize(server, &r, caller).await.map(|()| r),
                        Err(e) => Err(e),
                    },
                },
            },
        };
        let r = match resolved {
            Ok(r) => r,
            Err(e) => {
                failed_row(&mut lock(rec).steps[n], e.code, Some(&e.message), caller);
                stopped = Some(n);
                break;
            }
        };
        let out = execute(server, &r, caller).await;
        let ok = {
            let mut guard = lock(rec);
            let row = &mut guard.steps[n];
            if let Some(i) = &r.instance {
                row["instance"] = json!(i);
            }
            if out["success"] == json!(true) {
                row["status"] = json!("done");
                if let Some(undo) = out.get("undoPlanId") {
                    row["undoPlanId"] = undo.clone();
                }
                true
            } else {
                let result = &out["result"];
                let code = result.get("error_code").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("HTTP {}", out["status"]));
                let message = result.get("error").or_else(|| result.get("message")).and_then(Value::as_str);
                failed_row(row, &code, message, caller);
                false
            }
        };
        if !ok {
            stopped = Some(n);
            break;
        }

        if r.id == "instance.create" {
            if let Some(new_id) = out.pointer("/result/id").and_then(Value::as_str) {
                created[n] = Some(new_id.to_string());
                lock(rec).steps[n]["instance"] = json!(new_id);
            }
        }
    }
    let mut guard = lock(rec);
    for row in guard.steps.iter_mut().filter(|r| r["status"] == json!("pending")) {
        row["status"] = json!("skipped");
    }
    json!({
        "success": stopped.is_none(),
        "actionId": PLAN_ID,
        "runId": run_id,
        "steps": guard.steps,
        "stoppedAt": stopped.map(|n| n + 1),
    })
}
