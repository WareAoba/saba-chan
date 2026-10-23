//! `do` — 데몬 액션 카탈로그(P2)의 액션을 id로 실행한다. 액션 없이 부르면 목록

use crate::ctx::Ctx;
use crate::error::{usage, Result};
use crate::output::{self, s};
use serde_json::{json, Map, Value};

/// `key=value` 목록 → 입력 객체. 값이 JSON으로 읽히면(숫자·참거짓·배열·객체) 그대로, 아니면 글자
fn parse_input(pairs: &[String]) -> Result<Map<String, Value>> {
    let mut input = Map::new();
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(usage(t!("do.bad_pair", pair = pair)));
        };
        let value = serde_json::from_str::<Value>(value).ok().filter(|v| !v.is_string()).unwrap_or_else(|| Value::String(value.to_string()));
        input.insert(key.trim().to_string(), value);
    }
    Ok(input)
}

/// 계획의 요약 줄 (`summary: [{label, value}]`)
fn print_summary(p: &Value) {
    let rows: Vec<(String, String)> = p
        .get("summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|i| (s(i, "label").to_string(), i.get("value").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default()))
        .collect();
    output::print_kv(&rows);
}

/// 실패한 실행의 원인: 단일 액션은 `result.error`, 여러 단계는 멈춘 단계의 `error`(없으면 `code`)
fn failure(r: &Value) -> String {
    let result = r.get("result").cloned().unwrap_or(Value::Null);
    if let Some(m) = result.get("error").or_else(|| result.get("message")).and_then(Value::as_str) {
        return m.to_string();
    }
    let failed = r.get("steps").and_then(Value::as_array).into_iter().flatten().find(|st| s(st, "status") == "failed");
    match failed {
        Some(st) => {
            let why = st.get("error").or_else(|| st.get("code")).and_then(Value::as_str).unwrap_or("failed");
            format!("{}. {}: {why}", st.get("n").and_then(Value::as_u64).unwrap_or(0), s(st, "actionId"))
        }
        None => "failed".to_string(),
    }
}

/// 실행 결과: 실패면 오류로, 아니면 끝난 알림
fn finish(ctx: &Ctx, r: &Value, done: &str) -> Result<()> {
    if r.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(crate::error::CliError::Failed(failure(r)).into());
    }
    ctx.out.done(done, r);
    Ok(())
}

pub async fn run(ctx: &Ctx, action: Option<&str>, server: Option<&str>, plan_only: bool, pairs: &[String]) -> Result<()> {
    let instance = match server {
        Some(q) => Some(crate::resolve::server(ctx, Some(q)).await?.id),
        None => None,
    };
    let Some(action) = action else {
        let r = ctx.api.actions(instance.as_deref()).await?;
        ctx.out.data(&r, |r| {
            let mut t = output::table(&[t!("do.col.id"), t!("do.col.title"), t!("do.col.risk")]);
            for a in r.get("actions").and_then(Value::as_array).into_iter().flatten() {
                t.add_row(vec![s(a, "id"), s(a, "title"), s(a, "risk")]);
            }
            output::print_table(t);
        });
        return Ok(());
    };
    let input = parse_input(pairs)?;
    let body = json!({ "input": input, "instance": instance });
    let planned = ctx.api.action(action, "plan", body.clone()).await?;
    let show = print_summary;
    if plan_only {
        ctx.out.data(&planned, show);
        return Ok(());
    }
    if planned.get("needsConfirm").and_then(Value::as_bool) == Some(true) {
        if !ctx.out.json {
            show(&planned);
        }
        ctx.ask.ensure(&t!("do.confirm", action = action))?;
    }
    let plan_id = s(&planned, "planId").to_string();
    let r = ctx.api.action(action, "run", json!({ "planId": plan_id })).await?;
    finish(ctx, &r, &t!("do.done", action = action))
}

/// 뒤에서 이어 가는 여러 단계 실행을 다시 묻는 간격
const POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// `--steps`: 여러 단계를 한 번에 계획하고 실행한다. 오래 걸리면 데몬이 뒤에서 이어 가므로 끝날 때까지 진행을 묻는다
pub async fn steps(ctx: &Ctx, raw: &str, plan_only: bool) -> Result<()> {
    let steps: Value = serde_json::from_str(raw).map_err(|e| usage(t!("err.bad_json", error = e)))?;
    if !steps.is_array() {
        return Err(usage(t!("do.steps_array")));
    }
    let planned = ctx.api.plan_steps(steps).await?;
    let show = print_summary;
    if plan_only {
        ctx.out.data(&planned, show);
        return Ok(());
    }
    if planned.get("needsConfirm").and_then(Value::as_bool) == Some(true) {
        if !ctx.out.json {
            show(&planned);
        }
        ctx.ask.ensure(&t!("do.confirm_steps"))?;
    }
    let mut r = ctx.api.action("plan", "run", json!({ "planId": s(&planned, "planId") })).await?;
    if r.get("running").and_then(Value::as_bool) == Some(true) {
        let run = s(&r, "runId").to_string();
        // 기다리다 끊겨도 다시 볼 수 있게 실행 id를 먼저 알린다
        if !ctx.out.json {
            say!("{}", output::dim(&t!("do.run_hint", id = run)));
        }
        let bar = crate::progress::spinner(&t!("do.running"));
        loop {
            tokio::time::sleep(POLL).await;
            let status = ctx.api.action_run(&run).await?;
            if status.get("done").and_then(Value::as_bool) == Some(true) {
                r = status.get("result").cloned().unwrap_or(Value::Null);
                break;
            }
        }
        bar.finish_and_clear();
    }
    finish(ctx, &r, &t!("do.done_steps"))
}

/// `--run ID`: 여러 단계 실행의 진행
pub async fn run_status(ctx: &Ctx, run_id: &str) -> Result<()> {
    let r = ctx.api.action_run(run_id).await?;
    ctx.out.data(&r, |r| {
        let done = r.get("done").and_then(Value::as_bool) == Some(true);
        let state = if done { t!("do.state_done") } else { t!("do.state_running") };
        say!("{}  {}", run_id, state);
        let steps = if done { r.pointer("/result/steps") } else { r.get("steps") };
        for step in steps.and_then(Value::as_array).into_iter().flatten() {
            let mark = match s(step, "status") {
                "done" => "✓",
                "failed" => "✗",
                "running" => "…",
                _ => "·",
            };
            say!("  {mark} {}. {}", step.get("n").and_then(Value::as_u64).unwrap_or(0), s(step, "actionId"));
        }
    });
    // 끝났는데 실패했으면 종료 코드로도 알린다 (스크립트가 성공으로 읽지 않게)
    match finished_failure(&r) {
        Some(why) => Err(crate::error::CliError::Failed(why).into()),
        None => Ok(()),
    }
}

/// `GET /api/actions/runs/:id`가 끝났고 실패했으면 그 원인
fn finished_failure(r: &Value) -> Option<String> {
    let result = r.get("result").filter(|_| r.get("done").and_then(Value::as_bool) == Some(true))?;
    (result.get("success").and_then(Value::as_bool) == Some(false)).then(|| failure(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_that_read_as_json_stay_json() {
        let input = parse_input(&["count=3".into(), "on=true".into(), "name=world".into(), "list=[1,2]".into(), "quoted=\"x\"".into(), "msg=a=b".into()]).unwrap();
        assert_eq!(Value::Object(input), json!({ "count": 3, "on": true, "name": "world", "list": [1, 2], "quoted": "\"x\"", "msg": "a=b" }));
        assert!(parse_input(&["novalue".into()]).is_err());
    }

    /// 여러 단계가 실패하면 멈춘 단계와 그 원인을 알린다
    #[test]
    fn a_failed_step_says_which_step_and_why() {
        let r = json!({ "success": false, "steps": [
            { "n": 1, "actionId": "server.stop", "status": "done" },
            { "n": 2, "actionId": "server.start", "status": "failed", "code": "port_conflict" },
            { "n": 3, "actionId": "backup.create", "status": "skipped" },
        ] });
        assert_eq!(failure(&r), "2. server.start: port_conflict");
        assert_eq!(failure(&json!({ "success": false, "result": { "error": "boom" } })), "boom");
    }

    /// `do --run`: 끝난 실행이 실패였으면 실패로, 아직 도는 중이거나 성공이면 아니다
    #[test]
    fn a_finished_failed_run_is_a_failure() {
        let failed = json!({ "done": true, "result": { "success": false, "steps": [{ "n": 1, "actionId": "server.start", "status": "failed", "code": "x" }] } });
        assert_eq!(finished_failure(&failed).as_deref(), Some("1. server.start: x"));
        assert_eq!(finished_failure(&json!({ "done": true, "result": { "success": true } })), None);
        assert_eq!(finished_failure(&json!({ "done": false, "steps": [] })), None);
    }
}
