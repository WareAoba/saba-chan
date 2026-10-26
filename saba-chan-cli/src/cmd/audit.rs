//! `audit` — 누가 어디서 서버를 켜고 끄고 명령을 보냈는지 (데몬 감사 로그, A10)
//!
//! `-f`는 이어서 데몬 이벤트 스트림(P4)의 `action.run`을 한 줄씩 따라간다.
//! `--since`·`--until`은 기간, `--export`는 그 기간의 기록을 모두 데몬이 CSV로 쓴다

use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::Value;

/// 스트림이 끊겼을 때 다시 붙기 전 기다림
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);

pub struct Opts<'a> {
    pub server: Option<&'a str>,
    pub limit: usize,
    pub follow: bool,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub export: Option<&'a std::path::Path>,
}

pub async fn run(ctx: &Ctx, o: Opts<'_>) -> Result<()> {
    let id = match o.server {
        Some(q) => Some(crate::resolve::server(ctx, Some(q)).await?.id),
        None => None,
    };
    let since = o.since.map(|v| day(v, false)).transpose()?;
    let until = o.until.map(|v| day(v, true)).transpose()?;
    if let Some(file) = o.export {
        let path = crate::cmd::config::daemon_path(ctx, file)?;
        let r = ctx.api.export_audit(&path, id.as_deref(), since, until).await?;
        let count = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        ctx.out.done(&t!("audit.exported", path = path, count = count), &r);
        return Ok(());
    }
    // 목록을 읽기 전에 붙어야 그 사이에 일어난 일을 놓치지 않는다
    let mut events = if o.follow { Some(ctx.api.events(None).await?) } else { None };
    let r = ctx.api.audit(id.as_deref(), o.limit, since, until).await?;
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

/// `2026-10-01`·`today`·`7d`(오늘 포함 7일) → 그날 0시(이 PC 시간대, unix 초). `end`면 그날 끝(다음 날 0시)
fn day(value: &str, end: bool) -> Result<u64> {
    use chrono::{Days, Local, NaiveDate, TimeZone};
    let bad = || crate::error::usage(t!("audit.bad_date", value = value));
    let today = Local::now().date_naive();
    let date = if value == "today" {
        today
    } else if let Some(n) = value.strip_suffix('d').and_then(|n| n.parse::<u64>().ok()).filter(|n| *n >= 1) {
        today.checked_sub_days(Days::new(n - 1)).ok_or_else(bad)?
    } else {
        let parts: Vec<&str> = value.split('-').collect();
        match parts[..] {
            [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 => {
                NaiveDate::from_ymd_opt(y.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?)
                    .ok_or_else(bad)?
            }
            _ => return Err(bad()),
        }
    };
    let date = if end { date.checked_add_days(Days::new(1)).ok_or_else(bad)? } else { date };
    let midnight = date.and_hms_opt(0, 0, 0).ok_or_else(bad)?;
    Local.from_local_datetime(&midnight).earliest().map(|t| t.timestamp().max(0) as u64).ok_or_else(bad)
}

/// 기록 한 줄 (`/api/audit`의 `ts`, 이벤트의 `at`)
fn row(e: &Value) -> Vec<String> {
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
    let when = e.get("ts").or_else(|| e.get("at")).and_then(Value::as_u64).map(output::ago).unwrap_or_default();
    vec![when, from, server.to_string(), what, ok]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_range_covers_whole_days_and_the_end_day_too() {
        let since = day("2026-10-01", false).unwrap();
        let until = day("2026-10-03", true).unwrap();
        // 1일 0시부터 3일 끝까지 (서머타임이 끼면 한 시간 차이)
        assert!((until - since).abs_diff(3 * 86_400) <= 3_600, "{since}..{until}");
    }

    #[test]
    fn today_and_n_days_count_back_from_the_start_of_today() {
        let now = chrono::Local::now().timestamp() as u64;
        let today = day("today", false).unwrap();
        assert!(today <= now && now - today < 86_400);
        assert!((today - day("7d", false).unwrap()).abs_diff(6 * 86_400) <= 3_600);
    }

    #[test]
    fn a_wrong_date_is_refused() {
        for bad in ["", "2026-13-01", "yesterday", "0d", "26-10-01"] {
            assert!(day(bad, false).is_err(), "{bad}");
        }
    }
}
