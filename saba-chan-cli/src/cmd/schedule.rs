//! `schedule` — 서버별 예약 작업 (시작·정지·재시작·저장·백업·공지·업데이트·게임 명령)
//!
//! 데몬은 목록 전체를 한 번에 바꾼다 (`PUT {schedule, backup_keep?, revision?}`). 그래서 늘 읽고 → 고치고 → 통째로 보낸다.
//! 읽을 때 받은 `revision`을 함께 보내서, 그사이 다른 곳에서 바뀐 일정을 덮어쓰지 않는다 (409 `schedule_changed`).
//! 작업 하나는 `every_minutes`(N분마다)나 `at`(HH:MM, 서버 PC 시계) 중 하나로 돈다. `at`은 매일,
//! `every_days`(N일마다, `start_date`부터), `weekdays`(요일), `date`(그날 한 번) 중 하나로 날을 고를 수 있다.
//! 백업·공지·저장은 모듈이 지원해야 한다 (`backup_supported`, `announce_supported`, `save_supported`).

use crate::cli::{ScheduleCmd, ScheduleWhen};
use crate::cmd::server::shell_word;
use crate::ctx::Ctx;
use crate::error::{usage, Result};
use crate::output::{self, s};
use crate::resolve::{self, Server};
use serde_json::{json, Map, Value};

const ACTIONS: [&str; 8] = ["start", "stop", "restart", "save", "backup", "announce", "update", "command"];
/// 예고를 받는 작업 (데몬 `problems()`와 같다)
const WARNS: [&str; 3] = ["restart", "update", "stop"];
/// 데몬 `scheduler::WEEKDAYS`
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

pub async fn run(ctx: &Ctx, cmd: ScheduleCmd) -> Result<()> {
    match cmd {
        ScheduleCmd::Show { server } => show(ctx, server.as_deref()).await,
        ScheduleCmd::Add {
            server,
            action,
            when,
            warn,
            message,
            command,
            args,
        } => {
            let sv = resolve::server(ctx, Some(&server)).await?;
            let cur = ctx.api.schedule(&sv.id).await?;
            let task = build(&cur, &action, &when, &warn, message.as_deref(), command.as_deref(), &parse_args(&args)?)?;
            confirm_danger(ctx, &sv, &task).await?;
            let label = describe(&task);
            let mut list = tasks(&cur);
            list.push(task);
            save(ctx, &sv, Some(&cur), list, None, &t!("schedule.added", task = label)).await
        }
        ScheduleCmd::Edit {
            server,
            number,
            when,
            warn,
            no_warn,
            message,
            args,
        } => {
            let sv = resolve::server(ctx, Some(&server)).await?;
            let cur = ctx.api.schedule(&sv.id).await?;
            let change = Edit {
                when,
                warn,
                no_warn,
                message,
                args: if args.is_empty() { None } else { Some(parse_args(&args)?) },
            };
            // 바꾼 작업이 켜진 위험 명령이면 추가처럼 묻는다 (번호가 틀리면 edit_on이 알린다)
            let new = match tasks(&cur).get(number.wrapping_sub(1)) {
                Some(task) => Some(edited(&cur, task, &change)?),
                None => None,
            };
            if let Some(task) = new.as_ref().filter(|t| t.get("enabled").and_then(Value::as_bool) != Some(false)) {
                confirm_danger(ctx, &sv, task).await?;
            }
            edit_on(ctx, &sv, &cur, number, |list, i| {
                list[i] = new.unwrap_or_else(|| list[i].clone());
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
    when: ScheduleWhen,
    warn: Vec<u32>,
    no_warn: bool,
    message: Option<String>,
    args: Option<Map<String, Value>>,
}

fn gives_when(w: &ScheduleWhen) -> bool {
    w.every.is_some() || w.at.is_some() || w.from.is_some() || w.every_days.is_some() || !w.on.is_empty() || w.date.is_some() || w.daily
}

/// 지금 작업의 때 (고치기에서 주지 않은 것을 채운다)
fn when_of(task: &Value) -> ScheduleWhen {
    let text = |k: &str| task.get(k).and_then(Value::as_str).map(str::to_string);
    ScheduleWhen {
        every: task.get("every_minutes").and_then(Value::as_u64).map(|n| n.to_string()),
        at: text("at"),
        every_days: task.get("every_days").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        from: text("start_date"),
        on: task
            .get("weekdays")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        date: text("date"),
        daily: false,
    }
}

/// 고치기의 때: N분마다(`--every`)로 바꾸거나, 시각(`--at`)·날 규칙 중 준 것만 바꾼다
fn merged_when(old: ScheduleWhen, new: &ScheduleWhen) -> ScheduleWhen {
    if new.every.is_some() {
        return ScheduleWhen { every: new.every.clone(), ..ScheduleWhen::default() };
    }
    let at = new.at.clone().or(old.at);
    let rule_given = new.every_days.is_some() || !new.on.is_empty() || new.date.is_some() || new.daily;
    // N일마다를 다시 주면 세기 시작한 날은 그대로 둔다 (`--from`으로 바꾼다)
    let from = new.from.clone().or(old.from.clone().filter(|_| old.every_days.is_some()));
    if rule_given {
        let from = from.filter(|_| new.every_days.is_some());
        return ScheduleWhen { at, from, every: None, daily: false, ..new.clone() };
    }
    // 시각만 주면 날 규칙은 그대로, N분마다였으면 매일로
    ScheduleWhen { at, from, every: if new.at.is_some() { None } else { old.every }, ..old }
}

/// 작업 하나에 바꿀 것을 얹고, 추가와 같은 검사(`build`)를 거친다. 켜짐 여부는 그대로
fn edited(cur: &Value, task: &Value, change: &Edit) -> Result<Value> {
    if !gives_when(&change.when) && change.warn.is_empty() && !change.no_warn && change.message.is_none() && change.args.is_none() {
        return Err(usage(t!("schedule.nothing_to_edit")));
    }
    let when = merged_when(when_of(task), &change.when);
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
    let command = task.get("command").and_then(Value::as_str);
    let args = match &change.args {
        Some(a) => a.clone(),
        None => task.get("args").and_then(Value::as_object).cloned().unwrap_or_default(),
    };
    let mut out = build(cur, s(task, "action"), &when, &warn, message.as_deref(), command, &args)?;
    out["enabled"] = json!(task.get("enabled").and_then(Value::as_bool).unwrap_or(true));
    Ok(out)
}

/// 되돌리기 어려운 게임 명령(module.toml `risk = "danger"`)은 예약하기 전에 한 번 더 묻는다 (GUI와 같다)
async fn confirm_danger(ctx: &Ctx, sv: &Server, task: &Value) -> Result<()> {
    let Some(name) = task.get("command").and_then(Value::as_str) else {
        return Ok(());
    };
    // 목록을 받지 못하면 위험도를 모르니 묻는다. 별명으로 준 이름도 본다
    let danger = match ctx.api.instance_commands(&sv.id).await {
        Ok(list) => list.get("commands").and_then(Value::as_array).into_iter().flatten().any(|c| {
            let named = s(c, "name").eq_ignore_ascii_case(name)
                || c.get("aliases").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).any(|a| a.eq_ignore_ascii_case(name));
            named && s(c, "risk") == "danger"
        }),
        Err(_) => true,
    };
    if danger {
        ctx.ask.ensure(&t!("schedule.confirm_danger", command = name))?;
    }
    Ok(())
}

/// `NAME=VALUE` 목록 → 인자. 숫자처럼 보여도 글자로 보낸다 (데몬이 입력 형식에 맞춘다)
fn parse_args(raw: &[String]) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for a in raw {
        let (k, v) = a.split_once('=').filter(|(k, _)| !k.trim().is_empty()).ok_or_else(|| usage(t!("schedule.bad_arg", got = a)))?;
        out.insert(k.trim().to_string(), json!(v));
    }
    Ok(out)
}

async fn toggle(ctx: &Ctx, server: &str, number: usize, on: bool) -> Result<()> {
    let sv = resolve::server(ctx, Some(server)).await?;
    let cur = ctx.api.schedule(&sv.id).await?;
    if on {
        if let Some(task) = tasks(&cur).get(number.wrapping_sub(1)) {
            confirm_danger(ctx, &sv, task).await?;
        }
    }
    edit_on(ctx, &sv, &cur, number, |list, i| {
        list[i]["enabled"] = json!(on);
        let task = describe(&list[i]);
        Ok(if on {
            t!("schedule.enabled", task = task)
        } else {
            t!("schedule.disabled", task = task)
        })
    })
    .await
}

async fn show(ctx: &Ctx, server: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    let cur = ctx.api.schedule(&sv.id).await?;
    ctx.out.data(&cur, |cur| {
        let list = tasks(cur);
        let next_runs = cur.get("next_runs").and_then(Value::as_array).cloned().unwrap_or_default();
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
                t!("schedule.col.next"),
                t!("schedule.col.on"),
            ]);
            for (i, task) in list.iter().enumerate() {
                t.add_row(vec![
                    (i + 1).to_string(),
                    what(task),
                    when(task),
                    next_runs.get(i).and_then(Value::as_i64).map(local_time).unwrap_or_else(|| "—".into()),
                    output::yes_no(task.get("enabled").and_then(Value::as_bool).unwrap_or(true)),
                ]);
            }
            output::print_table(t);
        }
        // 시각은 서버 PC 시계다: 이 PC와 시간대가 다르면 알린다 (다음 실행은 이 PC 시간)
        if let Some(offset) = cur.get("utc_offset_minutes").and_then(Value::as_i64) {
            if offset != i64::from(chrono::Local::now().offset().local_minus_utc() / 60) {
                say!("{}", output::dim(&t!("schedule.server_clock", zone = zone_text(offset))));
            }
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

/// 유닉스 초 → 이 PC 시간 "10-27 (화) 04:00"이 아니라 숫자만: "2026-10-27 04:00"
fn local_time(secs: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(secs, 0).single().map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

fn zone_text(minutes: i64) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let (h, m) = (minutes.abs() / 60, minutes.abs() % 60);
    if m == 0 {
        format!("UTC{}{}", sign, h)
    } else {
        format!("UTC{}{}:{:02}", sign, h, m)
    }
}

/// 작업 이름과 덧붙일 말 (예고·공지 문구·게임 명령)
fn what(task: &Value) -> String {
    let action = s(task, "action");
    let mut out = crate::i18n::try_t(&format!("schedule.action.{}", action)).unwrap_or_else(|| action.to_string());
    let command = s(task, "command");
    if !command.is_empty() {
        out.push_str(&format!(" {}", command));
        for (k, v) in task.get("args").and_then(Value::as_object).into_iter().flatten() {
            out.push_str(&format!(" {}={}", k, v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())));
        }
    }
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
        return if n % 60 == 0 {
            t!("schedule.every_hours", n = n / 60)
        } else {
            t!("schedule.every_minutes", n = n)
        };
    }
    let at = s(task, "at");
    if let Some(n) = task.get("every_days").and_then(Value::as_u64) {
        return t!("schedule.every_days_at", n = n, at = at, from = s(task, "start_date"));
    }
    let days: Vec<String> = task
        .get("weekdays")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|w| crate::i18n::try_t(&format!("schedule.weekday.{}", w)).unwrap_or_else(|| w.to_string()))
        .collect();
    if !days.is_empty() {
        return t!("schedule.weekdays_at", days = days.join(", "), at = at);
    }
    let date = s(task, "date");
    if !date.is_empty() {
        return t!("schedule.date_at", date = date, at = at);
    }
    t!("schedule.daily_at", at = at)
}

fn describe(task: &Value) -> String {
    console::strip_ansi_codes(&format!("{} · {}", what(task), when(task))).into_owned()
}

/// 인자 → 작업 하나. 데몬도 검사하지만 여기서 먼저 사람 말로 알려 준다.
fn build(
    cur: &Value,
    action: &str,
    when: &ScheduleWhen,
    warn: &[u32],
    message: Option<&str>,
    command: Option<&str>,
    args: &Map<String, Value>,
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
    if action == "save" && !supported("save_supported") {
        return Err(usage(t!("schedule.no_save")));
    }
    let announces = action == "announce" || !warn.is_empty();
    if announces && !supported("announce_supported") {
        return Err(usage(t!("schedule.no_announce")));
    }
    if !warn.is_empty() && !WARNS.contains(&action.as_str()) {
        return Err(usage(t!("schedule.warn_only_restart")));
    }
    let message = message.map(str::trim).filter(|m| !m.is_empty());
    if action == "announce" && message.is_none() {
        return Err(usage(t!("schedule.message_required")));
    }
    let command = command.map(str::trim).filter(|c| !c.is_empty());
    match (action.as_str(), command) {
        ("command", None) => return Err(usage(t!("schedule.command_required"))),
        ("command", Some(_)) => {}
        (_, Some(_)) => return Err(usage(t!("schedule.command_only"))),
        _ if !args.is_empty() => return Err(usage(t!("schedule.command_only"))),
        _ => {}
    }
    let mut task = Map::new();
    task.insert("action".into(), json!(action));
    match (&when.every, &when.at) {
        (Some(e), None) => {
            if when.every_days.is_some() || !when.on.is_empty() || when.date.is_some() {
                return Err(usage(t!("schedule.day_rule_needs_at")));
            }
            task.insert("every_minutes".into(), json!(parse_every(e)?));
        }
        (None, Some(a)) => {
            task.insert("at".into(), json!(parse_at(a)?));
        }
        _ => return Err(usage(t!("schedule.every_or_at"))),
    }
    if when.from.is_some() && when.every_days.is_none() {
        return Err(usage(t!("schedule.from_needs_every_days")));
    }
    if let Some(n) = when.every_days {
        if n == 0 || n > 365 {
            return Err(usage(t!("schedule.bad_every_days", got = n)));
        }
        task.insert("every_days".into(), json!(n));
        if let Some(from) = &when.from {
            task.insert("start_date".into(), json!(parse_date(from)?));
        }
    }
    if !when.on.is_empty() {
        task.insert("weekdays".into(), json!(parse_weekdays(&when.on)?));
    }
    if let Some(d) = &when.date {
        task.insert("date".into(), json!(parse_date(d)?));
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
    if let Some(c) = command {
        task.insert("command".into(), json!(c));
        if !args.is_empty() {
            task.insert("args".into(), Value::Object(args.clone()));
        }
    }
    task.insert("enabled".into(), json!(true));
    Ok(Value::Object(task))
}

/// "mon,FRI" → 월요일부터 정렬한 데몬 이름. 영어 세 글자나 전체 이름을 받는다
fn parse_weekdays(raw: &[String]) -> Result<Vec<&'static str>> {
    let mut picked = [false; 7];
    for w in raw {
        let day: chrono::Weekday = w
            .trim()
            .parse()
            .map_err(|_| usage(t!("schedule.bad_weekday", got = w, days = WEEKDAYS.join(", "))))?;
        picked[day.num_days_from_monday() as usize] = true;
    }
    Ok(WEEKDAYS.iter().zip(picked).filter(|(_, on)| *on).map(|(d, _)| *d).collect())
}

/// "2026-12-24" (그런 날이 있어야 한다)
fn parse_date(raw: &str) -> Result<String> {
    chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map(|d| d.to_string())
        .map_err(|_| usage(t!("schedule.bad_date", got = raw)))
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

        let every = Edit { when: ScheduleWhen { every: Some("2h".into()), ..ScheduleWhen::default() }, ..Edit::default() };
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

    fn when(f: impl FnOnce(&mut ScheduleWhen)) -> ScheduleWhen {
        let mut w = ScheduleWhen::default();
        f(&mut w);
        w
    }

    #[test]
    fn day_rules_and_new_actions_build_daemon_tasks() {
        let cur = json!({ "backup_supported": true, "announce_supported": true, "save_supported": false });
        let none = Map::new();
        let weekly = when(|w| {
            w.at = Some("23:00".into());
            w.on = vec!["FRI".into(), "monday".into()];
        });
        assert_eq!(
            build(&cur, "stop", &weekly, &[5], None, None, &none).unwrap(),
            json!({ "action": "stop", "at": "23:00", "weekdays": ["mon", "fri"], "warn_minutes": [5], "enabled": true })
        );
        let every3 = when(|w| {
            w.at = Some("4:00".into());
            w.every_days = Some(3);
            w.from = Some("2026-10-30".into());
        });
        assert_eq!(
            build(&cur, "start", &every3, &[], None, None, &none).unwrap(),
            json!({ "action": "start", "at": "04:00", "every_days": 3, "start_date": "2026-10-30", "enabled": true })
        );
        let once = when(|w| {
            w.at = Some("18:30".into());
            w.date = Some("2026-12-24".into());
        });
        let args = parse_args(&["message=hi there".into()]).unwrap();
        assert_eq!(
            build(&cur, "command", &once, &[], None, Some("say"), &args).unwrap(),
            json!({ "action": "command", "at": "18:30", "date": "2026-12-24", "command": "say", "args": { "message": "hi there" }, "enabled": true })
        );
        // 저장 명령이 없는 모듈, 명령 없는 command, 다른 작업의 명령, 시각 없는 날 규칙, 없는 날
        assert!(build(&cur, "save", &once, &[], None, None, &none).is_err());
        assert!(build(&cur, "command", &once, &[], None, None, &none).is_err());
        assert!(build(&cur, "stop", &once, &[], None, Some("say"), &none).is_err());
        let minutes_on = when(|w| {
            w.every = Some("30".into());
            w.on = vec!["mon".into()];
        });
        assert!(build(&cur, "stop", &minutes_on, &[], None, None, &none).is_err());
        assert!(parse_date("2027-02-29").is_err());
        assert!(parse_weekdays(&["noday".into()]).is_err());
        assert!(parse_args(&["=x".into()]).is_err());
    }

    #[test]
    fn an_edit_keeps_the_day_rule_unless_another_is_given() {
        let cur = json!({ "announce_supported": true });
        let task = json!({ "action": "stop", "at": "23:00", "weekdays": ["mon"], "enabled": true });
        let at = Edit { when: when(|w| w.at = Some("22:00".into())), ..Edit::default() };
        assert_eq!(edited(&cur, &task, &at).unwrap()["weekdays"], json!(["mon"]));
        let daily = Edit { when: when(|w| w.daily = true), ..Edit::default() };
        let t = edited(&cur, &task, &daily).unwrap();
        assert!(t.get("weekdays").is_none());
        assert_eq!(t["at"], "23:00");
        // N분마다였던 작업에 시각만 주면 매일 그 시각
        let every = json!({ "action": "restart", "every_minutes": 60, "enabled": true });
        let t = edited(&cur, &every, &at).unwrap();
        assert_eq!((t["at"].clone(), t.get("every_minutes")), (json!("22:00"), None));
        // N일마다의 시작일만 바꿀 수 있고, 다른 날 규칙에는 쓸 수 없다
        let days = json!({ "action": "stop", "at": "04:00", "every_days": 3, "start_date": "2026-10-30", "enabled": true });
        let from = Edit { when: when(|w| w.from = Some("2026-11-02".into())), ..Edit::default() };
        assert_eq!(edited(&cur, &days, &from).unwrap()["start_date"], "2026-11-02");
        let n = Edit { when: when(|w| w.every_days = Some(5)), ..Edit::default() };
        assert_eq!(edited(&cur, &days, &n).unwrap()["start_date"], "2026-10-30");
        assert!(edited(&cur, &task, &from).is_err());
        // 게임 명령의 인자는 통째로 바꾼다
        let cmd = json!({ "action": "command", "at": "04:00", "command": "say", "args": { "message": "a" }, "enabled": true });
        let args = Edit { args: Some(parse_args(&["message=b".into()]).unwrap()), ..Edit::default() };
        assert_eq!(edited(&cur, &cmd, &args).unwrap()["args"], json!({ "message": "b" }));
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
