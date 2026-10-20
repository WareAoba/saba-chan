//! `schedule` — 서버별 예약 작업 (백업·재시작·공지·업데이트)
//!
//! 데몬은 목록 전체를 한 번에 바꾼다 (`PUT {schedule, backup_keep?, revision?}`). 그래서 늘 읽고 → 고치고 → 통째로 보낸다.
//! 읽을 때 받은 `revision`을 함께 보내서, 그사이 다른 곳에서 바뀐 일정을 덮어쓰지 않는다 (409 `schedule_changed`).
//! 작업 하나는 `every_minutes`(N분마다)나 `at`(매일 HH:MM, 데몬 PC 시간) 중 하나로 돈다.
//! 백업·공지는 모듈이 지원해야 한다 (`backup_supported`, `announce_supported`).

use crate::cli::ScheduleCmd;
use crate::cmd::server::shell_word;
use crate::ctx::Ctx;
use crate::error::{usage, Result};
use crate::output::{self, s};
use crate::resolve::{self, Server};
use serde_json::{json, Map, Value};

const ACTIONS: [&str; 4] = ["backup", "restart", "announce", "update"];

pub async fn run(ctx: &Ctx, cmd: ScheduleCmd) -> Result<()> {
    match cmd {
        ScheduleCmd::Show { server } => show(ctx, server.as_deref()).await,
        ScheduleCmd::Add {
            server,
            action,
            every,
            at,
            warn,
            message,
        } => {
            let sv = resolve::server(ctx, Some(&server)).await?;
            let cur = ctx.api.schedule(&sv.id).await?;
            let task = build(
                &cur,
                &action,
                every.as_deref(),
                at.as_deref(),
                &warn,
                message.as_deref(),
            )?;
            let label = describe(&task);
            let mut list = tasks(&cur);
            list.push(task);
            save(ctx, &sv, Some(&cur), list, None, &t!("schedule.added", task = label)).await
        }
        ScheduleCmd::Edit {
            server,
            number,
            every,
            at,
            warn,
            no_warn,
            message,
        } => {
            let sv = resolve::server(ctx, Some(&server)).await?;
            let cur = ctx.api.schedule(&sv.id).await?;
            let change = Edit {
                every,
                at,
                warn,
                no_warn,
                message,
            };
            edit_on(ctx, &sv, &cur, number, |list, i| {
                list[i] = edited(&cur, &list[i], &change)?;
                Ok(t!("schedule.edited", task = describe(&list[i])))
            })
            .await
        }
        ScheduleCmd::Remove { server, number } => {
            edit(ctx, &server, number, |list, i| {
                let t = list.remove(i);
                t!("schedule.removed", task = describe(&t))
            })
            .await
        }
        ScheduleCmd::On { server, number } => toggle(ctx, &server, number, true).await,
        ScheduleCmd::Off { server, number } => toggle(ctx, &server, number, false).await,
        ScheduleCmd::Keep { server, count } => {
            if count == 0 {
                return Err(usage(t!("schedule.keep_zero")));
            }
            let sv = resolve::server(ctx, Some(&server)).await?;
            let cur = ctx.api.schedule(&sv.id).await?;
            save(
                ctx,
                &sv,
                Some(&cur),
                tasks(&cur),
                Some(count),
                &t!("schedule.keep_set", count = count),
            )
            .await
        }
        ScheduleCmd::Clear { server } => {
            let sv = resolve::server(ctx, Some(&server)).await?;
            ctx.ask.ensure(&t!("schedule.confirm_clear", name = sv.name))?;
            // 모두 지우기는 무엇이 있든 비우므로 읽은 판을 따지지 않는다
            save(ctx, &sv, None, Vec::new(), None, &t!("schedule.cleared", name = sv.name)).await
        }
    }
}

fn tasks(cur: &Value) -> Vec<Value> {
    cur.get("schedule")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// 보낼 본문. `read`는 고치기 전에 읽은 일정: 그 `revision`을 실어 그사이 바뀌었으면 데몬이 거절하게 한다
fn save_body(read: Option<&Value>, list: Vec<Value>, keep: Option<u32>) -> Value {
    let mut body = json!({ "schedule": list });
    if let Some(k) = keep {
        body["backup_keep"] = json!(k);
    }
    if let Some(rev) = read.and_then(|r| r.get("revision")).and_then(Value::as_str) {
        body["revision"] = json!(rev);
    }
    body
}

async fn save(ctx: &Ctx, sv: &Server, read: Option<&Value>, list: Vec<Value>, keep: Option<u32>, msg: &str) -> Result<()> {
    let r = ctx.api.save_schedule(&sv.id, save_body(read, list, keep)).await?;
    ctx.out.done(msg, &r);
    Ok(())
}

/// 1부터 세는 번호로 작업 하나를 고친다
async fn edit(ctx: &Ctx, server: &str, number: usize, f: impl FnOnce(&mut Vec<Value>, usize) -> String) -> Result<()> {
    let sv = resolve::server(ctx, Some(server)).await?;
    let cur = ctx.api.schedule(&sv.id).await?;
    edit_on(ctx, &sv, &cur, number, |list, i| Ok(f(list, i))).await
}

async fn edit_on(
    ctx: &Ctx,
    sv: &Server,
    cur: &Value,
    number: usize,
    f: impl FnOnce(&mut Vec<Value>, usize) -> Result<String>,
) -> Result<()> {
    let mut list = tasks(cur);
    if number == 0 || number > list.len() {
        ctx.out
            .hint(&format!("saba-chan-cli schedule show {}", shell_word(&sv.name)));
        return Err(usage(t!("schedule.no_such", number = number, count = list.len())));
    }
    let msg = f(&mut list, number - 1)?;
    save(ctx, sv, Some(cur), list, None, &msg).await
}

/// `schedule edit`에서 바꿀 것 — 준 것만
#[derive(Default)]
struct Edit {
    every: Option<String>,
    at: Option<String>,
    warn: Vec<u32>,
    no_warn: bool,
    message: Option<String>,
}

/// 작업 하나에 바꿀 것을 얹고, 추가와 같은 검사(`build`)를 거친다. 켜짐 여부는 그대로
fn edited(cur: &Value, task: &Value, change: &Edit) -> Result<Value> {
    if change.every.is_none() && change.at.is_none() && change.warn.is_empty() && !change.no_warn && change.message.is_none() {
        return Err(usage(t!("schedule.nothing_to_edit")));
    }
    let old_every = task.get("every_minutes").and_then(Value::as_u64).map(|n| n.to_string());
    let old_at = task.get("at").and_then(Value::as_str).map(str::to_string);
    let (every, at) = match (&change.every, &change.at) {
        (Some(e), _) => (Some(e.clone()), None),
        (None, Some(a)) => (None, Some(a.clone())),
        (None, None) => (old_every, old_at),
    };
    let warn: Vec<u32> = if change.no_warn {
        Vec::new()
    } else if !change.warn.is_empty() {
        change.warn.clone()
    } else {
        task.get("warn_minutes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter_map(|m| u32::try_from(m).ok())
            .collect()
    };
    let message = change.message.clone().or_else(|| task.get("message").and_then(Value::as_str).map(str::to_string));
    let mut out = build(cur, s(task, "action"), every.as_deref(), at.as_deref(), &warn, message.as_deref())?;
    out["enabled"] = json!(task.get("enabled").and_then(Value::as_bool).unwrap_or(true));
    Ok(out)
}

async fn toggle(ctx: &Ctx, server: &str, number: usize, on: bool) -> Result<()> {
    edit(ctx, server, number, |list, i| {
        list[i]["enabled"] = json!(on);
        let task = describe(&list[i]);
        if on {
            t!("schedule.enabled", task = task)
        } else {
            t!("schedule.disabled", task = task)
        }
    })
    .await
}

async fn show(ctx: &Ctx, server: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    let cur = ctx.api.schedule(&sv.id).await?;
    ctx.out.data(&cur, |cur| {
        let list = tasks(cur);
        if list.is_empty() {
            say!("{}", output::dim(&t!("schedule.none", name = sv.name)));
            ctx.out.hint(&format!(
                "saba-chan-cli schedule add {} restart --at 04:00 --warn 10,1",
                shell_word(&sv.name)
            ));
        } else {
            let mut t = output::table(&[
                "#".into(),
                t!("schedule.col.task"),
                t!("schedule.col.when"),
                t!("schedule.col.on"),
            ]);
            for (i, task) in list.iter().enumerate() {
                t.add_row(vec![
                    (i + 1).to_string(),
                    what(task),
                    when(task),
                    output::yes_no(task.get("enabled").and_then(Value::as_bool).unwrap_or(true)),
                ]);
            }
            output::print_table(t);
        }
        if cur.get("backup_supported").and_then(Value::as_bool) == Some(true) {
            let keep = cur.get("backup_keep").map(output::show).unwrap_or_default();
            say!("{}", output::dim(&t!("schedule.keep", count = keep)));
        } else {
            say!("{}", output::dim(&t!("schedule.no_backup")));
        }
        if cur.get("announce_supported").and_then(Value::as_bool) != Some(true) {
            say!("{}", output::dim(&t!("schedule.no_announce")));
        }
    });
    Ok(())
}

/// 작업 이름과 덧붙일 말 (예고·공지 문구)
fn what(task: &Value) -> String {
    let action = s(task, "action");
    let mut out = crate::i18n::try_t(&format!("schedule.action.{}", action)).unwrap_or_else(|| action.to_string());
    let warn: Vec<String> = task
        .get("warn_minutes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|m| m.to_string())
        .collect();
    if !warn.is_empty() {
        out.push_str(&output::dim(&format!(
            " ({})",
            t!("schedule.warns", minutes = warn.join(", "))
        )));
    }
    let msg = s(task, "message");
    if !msg.is_empty() {
        out.push_str(&format!(" “{}”", msg));
    }
    out
}

fn when(task: &Value) -> String {
    if let Some(n) = task.get("every_minutes").and_then(Value::as_u64) {
        if n % 60 == 0 {
            t!("schedule.every_hours", n = n / 60)
        } else {
            t!("schedule.every_minutes", n = n)
        }
    } else {
        t!("schedule.daily_at", at = s(task, "at"))
    }
}

fn describe(task: &Value) -> String {
    console::strip_ansi_codes(&format!("{} · {}", what(task), when(task))).into_owned()
}

/// 인자 → 작업 하나. 데몬도 검사하지만 여기서 먼저 사람 말로 알려 준다.
fn build(
    cur: &Value,
    action: &str,
    every: Option<&str>,
    at: Option<&str>,
    warn: &[u32],
    message: Option<&str>,
) -> Result<Value> {
    let action = action.trim().to_ascii_lowercase();
    if !ACTIONS.contains(&action.as_str()) {
        return Err(usage(t!(
            "schedule.bad_action",
            got = action,
            actions = ACTIONS.join(", ")
        )));
    }
    let supported = |k: &str| cur.get(k).and_then(Value::as_bool).unwrap_or(true);
    if action == "backup" && !supported("backup_supported") {
        return Err(usage(t!("schedule.no_backup")));
    }
    let announces = action == "announce" || !warn.is_empty();
    if announces && !supported("announce_supported") {
        return Err(usage(t!("schedule.no_announce")));
    }
    if !warn.is_empty() && !matches!(action.as_str(), "restart" | "update") {
        return Err(usage(t!("schedule.warn_only_restart")));
    }
    let message = message.map(str::trim).filter(|m| !m.is_empty());
    if action == "announce" && message.is_none() {
        return Err(usage(t!("schedule.message_required")));
    }
    let mut task = Map::new();
    task.insert("action".into(), json!(action));
    match (every, at) {
        (Some(e), None) => {
            task.insert("every_minutes".into(), json!(parse_every(e)?));
        }
        (None, Some(a)) => {
            task.insert("at".into(), json!(parse_at(a)?));
        }
        _ => return Err(usage(t!("schedule.every_or_at"))),
    }
    if !warn.is_empty() {
        let mut w = warn.to_vec();
        w.sort_unstable_by(|a, b| b.cmp(a));
        w.dedup();
        task.insert("warn_minutes".into(), json!(w));
    }
    if let Some(m) = message {
        task.insert("message".into(), json!(m));
    }
    task.insert("enabled".into(), json!(true));
    Ok(Value::Object(task))
}

/// "30", "30m", "2h", "1h30m" → 분
fn parse_every(raw: &str) -> Result<u32> {
    let bad = || usage(t!("schedule.bad_every", got = raw));
    let r = raw.trim().to_ascii_lowercase();
    if let Ok(n) = r.parse::<u32>() {
        return (n > 0).then_some(n).ok_or_else(bad);
    }
    let mut total: u32 = 0;
    let mut num = String::new();
    for c in r.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u32 = num.parse().map_err(|_| bad())?;
        num.clear();
        total = match c {
            'h' => total.checked_add(n.checked_mul(60).ok_or_else(bad)?),
            'm' => total.checked_add(n),
            'd' => total.checked_add(n.checked_mul(1440).ok_or_else(bad)?),
            _ => None,
        }
        .ok_or_else(bad)?;
    }
    if !num.is_empty() || total == 0 {
        return Err(bad());
    }
    Ok(total)
}

/// "4:00", "04:00" → "04:00"
fn parse_at(raw: &str) -> Result<String> {
    let bad = || usage(t!("schedule.bad_at", got = raw));
    let (h, m) = raw.trim().split_once(':').ok_or_else(bad)?;
    let h: u32 = h.parse().map_err(|_| bad())?;
    let m: u32 = m.parse().map_err(|_| bad())?;
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(format!("{:02}:{:02}", h, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 작업 하나를 지우지 않고 고친다. 준 것만 바뀌고 켜짐 여부는 그대로 (C19)
    /// 고치기 전에 읽은 일정의 revision을 실어 보낸다: 그사이 바뀌었으면 데몬이 409로 거절해 엉뚱한 작업을 지우지 않는다 (L51)
    #[test]
    fn a_save_carries_the_revision_it_read() {
        let read = json!({ "schedule": [], "backup_keep": 10, "revision": "ab12" });
        assert_eq!(save_body(Some(&read), vec![], Some(3)), json!({ "schedule": [], "backup_keep": 3, "revision": "ab12" }));
        // 옛 데몬은 revision을 주지 않는다: 예전처럼 보낸다
        assert_eq!(save_body(Some(&json!({ "schedule": [] })), vec![], None), json!({ "schedule": [] }));
    }

    #[test]
    fn an_edit_changes_only_what_is_given() {
        let cur = json!({ "backup_supported": true, "announce_supported": true });
        let task = json!({ "action": "restart", "at": "04:00", "warn_minutes": [10, 1], "enabled": false });
        let none = Edit::default();
        assert!(edited(&cur, &task, &none).is_err(), "nothing to change");

        let every = Edit { every: Some("2h".into()), ..Edit::default() };
        let t = edited(&cur, &task, &every).unwrap();
        assert_eq!(t["every_minutes"], 120);
        assert!(t.get("at").is_none());
        assert_eq!(t["warn_minutes"], json!([10, 1]));
        assert_eq!(t["enabled"], false);

        let warn = Edit { warn: vec![5], ..Edit::default() };
        assert_eq!(edited(&cur, &task, &warn).unwrap()["warn_minutes"], json!([5]));
        let no_warn = Edit { no_warn: true, ..Edit::default() };
        assert!(edited(&cur, &task, &no_warn).unwrap().get("warn_minutes").is_none());

        // 검사는 추가와 같다: 공지 문구를 비울 수 없다
        let announce = json!({ "action": "announce", "every_minutes": 60, "message": "hi", "enabled": true });
        let t = edited(&cur, &announce, &Edit { message: Some("bye".into()), ..Edit::default() }).unwrap();
        assert_eq!((t["message"].clone(), t["every_minutes"].clone()), (json!("bye"), json!(60)));
        assert!(edited(&cur, &announce, &Edit { message: Some(" ".into()), ..Edit::default() }).is_err());
    }

    #[test]
    fn every_forms() {
        assert_eq!(parse_every("30").unwrap(), 30);
        assert_eq!(parse_every("30m").unwrap(), 30);
        assert_eq!(parse_every("2h").unwrap(), 120);
        assert_eq!(parse_every("1h30m").unwrap(), 90);
        assert_eq!(parse_every("1d").unwrap(), 1440);
        assert!(parse_every("0").is_err());
        assert!(parse_every("h").is_err());
        assert!(parse_every("5x").is_err());
        assert!(parse_every("1h5").is_err());
    }

    #[test]
    fn at_forms() {
        assert_eq!(parse_at("4:00").unwrap(), "04:00");
        assert_eq!(parse_at("23:59").unwrap(), "23:59");
        assert!(parse_at("24:00").is_err());
        assert!(parse_at("0400").is_err());
    }
}
