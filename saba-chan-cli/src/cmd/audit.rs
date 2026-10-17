//! `audit` — 누가 어디서 서버를 켜고 끄고 명령을 보냈는지 (데몬 감사 로그, A10)

use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::Value;

pub async fn run(ctx: &Ctx, server: Option<&str>, limit: usize) -> Result<()> {
    let id = match server {
        Some(q) => Some(crate::resolve::server(ctx, Some(q)).await?.id),
        None => None,
    };
    let r = ctx.api.audit(id.as_deref(), limit).await?;
    ctx.out.data(&r, |r| {
        let entries = r.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
        if entries.is_empty() {
            println!("{}", output::dim(&t!("audit.empty")));
            return;
        }
        let mut t = output::table(&[t!("audit.when"), t!("audit.from"), t!("noun.server"), t!("audit.what"), t!("audit.result")]);
        for e in &entries {
            let from = match s(e, "user") {
                "" => s(e, "source").to_string(),
                user => format!("{} ({})", s(e, "source"), user),
            };
            let server = match s(e, "instance") {
                "" => s(e, "instanceId"),
                name => name,
            };
            let what = match (s(e, "action"), s(e, "command")) {
                (action, "") => action.to_string(),
                (action, cmd) => format!("{} {} {}", action, cmd, s(e, "args")).trim_end().to_string(),
            };
            let ok = if e.get("ok").and_then(Value::as_bool) == Some(true) {
                console::style("✓").green().to_string()
            } else {
                console::style(format!("✗ {}", e.get("status").and_then(Value::as_u64).unwrap_or(0))).red().to_string()
            };
            let when = e.get("ts").and_then(Value::as_u64).map(output::ago).unwrap_or_default();
            t.add_row(vec![when, from, server.to_string(), what, ok]);
        }
        output::print_table(t);
    });
    Ok(())
}
