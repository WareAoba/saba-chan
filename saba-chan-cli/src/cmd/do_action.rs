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
    let show = |p: &Value| {
        let rows: Vec<(String, String)> = p
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|i| (s(i, "label").to_string(), i.get("value").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default()))
            .collect();
        output::print_kv(&rows);
    };
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
    if r.get("success").and_then(Value::as_bool) == Some(false) {
        let result = r.get("result").cloned().unwrap_or(Value::Null);
        let msg = result.get("error").or_else(|| result.get("message")).and_then(Value::as_str).unwrap_or("failed").to_string();
        return Err(crate::error::CliError::Failed(msg).into());
    }
    ctx.out.done(&t!("do.done", action = action), &r);
    Ok(())
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
}
