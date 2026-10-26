//! `audit` — 누가 어디서 서버를 켜고 끄고 명령을 보냈는지 (데몬 감사 로그, A10)
//!
//! `-f`는 이어서 데몬 이벤트 스트림(P4)의 `action.run`을 한 줄씩 따라간다

use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::Value;

/// 스트림이 끊겼을 때 다시 붙기 전 기다림
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);

pub async fn run(ctx: &Ctx, server: Option<&str>, limit: usize, follow: bool) -> Result<()> {
    let id = match server {
        Some(q) => Some(crate::resolve::server(ctx, Some(q)).await?.id),
        None => None,
    };
    // 목록을 읽기 전에 붙어야 그 사이에 일어난 일을 놓치지 않는다
    let mut events = if follow { Some(ctx.api.events(None).await?) } else { None };
    let r = ctx.api.audit(id.as_deref(), limit).await?;
    ctx.out.data(&r, |r| {
        let entries = r.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
        if entries.is_empty() {
            say!("{}", output::dim(&t!("audit.empty")));
            return;
        }
        let mut t = output::table(&[t!("audit.when"), t!("audit.from"), t!("noun.server"), t!("audit.what"), t!("audit.result")]);
        for e in &entries {
            t.add_row(row(e));
        }
        output::print_table(t);
    });
    let Some(mut stream) = events.take() else { return Ok(()) };

    let mut last = None;
    loop {
        match stream.next().await {
            Ok(Some(event)) => {
                last = event.id.or(last);
                if event.kind != "action.run" || id.as_deref().is_some_and(|id| s(&event.data, "instanceId") != id) {
                    continue;
                }
                if ctx.out.json {
                    say!("{}", output::event_line(serde_json::json!({ "event": event.kind, "data": event.data })));
                } else {
                    say!("{}", row(&event.data).join("  "));
                }
            }
            // 데몬이 다시 켜지는 중일 수 있다: 마지막 번호로 다시 붙는다
            Ok(None) | Err(_) => loop {
                tokio::time::sleep(RETRY).await;
                if let Ok(fresh) = ctx.api.events(last).await {
                    stream = fresh;
                    break;
                }
            },
        }
    }
}

/// 기록 한 줄 (`/api/audit`의 `ts`, 이벤트의 `at`)
fn row(e: &Value) -> Vec<String> {
    let from = match s(e, "user") {
        "" => s(e, "source").to_string(),
        user => format!("{} ({})", s(e, "source"), user),
    };
    // 원격 데몬에 중계한 조작(`remote`)은 서버 대신 그 데몬
    let server = [s(e, "instance"), s(e, "instanceId"), s(e, "daemon")].into_iter().find(|v| !v.is_empty()).unwrap_or("");
    let what = match (s(e, "action"), s(e, "command")) {
        (action, "") => action.to_string(),
        (action, cmd) => format!("{} {} {}", action, cmd, s(e, "args")).trim_end().to_string(),
    };
    let ok = if e.get("ok").and_then(Value::as_bool) == Some(true) {
        console::style("✓").green().to_string()
    } else {
        console::style(format!("✗ {}", e.get("status").and_then(Value::as_u64).unwrap_or(0))).red().to_string()
    };
    let when = e.get("ts").or_else(|| e.get("at")).and_then(Value::as_u64).map(output::ago).unwrap_or_default();
    vec![when, from, server.to_string(), what, ok]
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_relayed_change_names_the_remote_daemon() {
        let e = serde_json::json!({ "source": "gui", "action": "remote", "instanceId": "", "daemon": "게임 PC", "command": "POST /api/instance/x/start", "ok": true });
        let row = super::row(&e);
        assert_eq!(row[2], "게임 PC");
        assert_eq!(row[3], "remote POST /api/instance/x/start");
    }
}
