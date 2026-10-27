//! 원격(디스코드·챗봇) 결과의 사람용 요약 (`text`). 요약이 없는 결과는 릴레이가 결과 그대로 보인다

use crate::chat::i18n::Catalog;
use serde_json::Value;

/// 목록은 이만큼만 줄로 보이고 나머지는 개수로 센다
const MAX_ITEMS: usize = 10;

/// 액션 결과(`execute`의 `result`) → 요약. 요약할 줄 모르는 액션이면 None
pub(crate) fn text(id: &str, result: &Value, cat: &Catalog) -> Option<String> {
    match id {
        "instance.list" => Some(servers(result, cat)),
        "backup.list" => Some(backups(result, cat)),
        "server.players" => Some(players(result, cat)),
        "instance.events" => Some(events(result, cat)),
        "update.check" => updates(result, cat),
        "monitor.summary" => monitor(result, cat),
        "server.port_check" => Some(port_check(result, cat)),
        // 모듈 조회(접속자 목록 등)는 모양이 모듈마다 달라 채팅 명령 결과와 같은 범용 포맷으로
        _ if id.starts_with(super::QUERY_PREFIX) => {
            let mut shown = result.clone();
            if let Some(map) = shown.as_object_mut() {
                map.remove("success");
            }
            Some(crate::chat::format::format_response(&shown, cat))
        }
        _ => None,
    }
}

fn items(result: &Value, key: &str) -> Vec<Value> {
    result.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 머리 줄 + 항목 줄(MAX_ITEMS까지) + 남은 개수
fn with_rest(head: String, lines: Vec<String>, cat: &Catalog) -> String {
    let rest = lines.len().saturating_sub(MAX_ITEMS);
    let mut out = vec![head];
    out.extend(lines.into_iter().take(MAX_ITEMS));
    if rest > 0 {
        out.push(cat.t("actions.summary.more", &[("count", &rest.to_string())]));
    }
    out.join("\n")
}

fn servers(result: &Value, cat: &Catalog) -> String {
    let servers = items(result, "servers");
    if servers.is_empty() {
        return cat.t("list.empty", &[]);
    }
    let lines = servers
        .iter()
        .map(|s| {
            let status = cat.t(if str_of(s, "status") == "running" { "status.running" } else { "status.stopped" }, &[]);
            cat.t("actions.summary.server_item", &[("name", str_of(s, "name")), ("module", str_of(s, "module")), ("status", &status)])
        })
        .collect();
    with_rest(cat.t("list.title", &[]), lines, cat)
}

fn backups(result: &Value, cat: &Catalog) -> String {
    let mut backups = items(result, "backups");
    if backups.is_empty() {
        return cat.t("actions.summary.backups_empty", &[]);
    }
    backups.sort_by_key(|b| std::cmp::Reverse(b.get("created").and_then(Value::as_u64).unwrap_or(0)));
    let lines = backups
        .iter()
        .map(|b| {
            let kind = cat.t(&format!("actions.summary.backup_kind.{}", str_of(b, "kind")), &[]);
            let when = local_time(b.get("created").and_then(Value::as_u64).unwrap_or(0));
            // 게임 자동 백업(game-snapshot)은 크기를 재지 않는다: 크기 자리와 그 앞 구분자를 뺀다
            let size = b.get("size").and_then(Value::as_u64).map(bytes);
            let line = cat.t("actions.summary.backup_item", &[("name", str_of(b, "name")), ("kind", &kind), ("when", &when), ("size", size.as_deref().unwrap_or(""))]);
            let line = if size.is_none() { line.trim_end().trim_end_matches('·').trim_end().to_string() } else { line };
            line + &unreadable_note(b, cat)
        })
        .collect();
    with_rest(cat.t("actions.summary.backups", &[("count", &backups.len().to_string())]), lines, cat)
}

/// `접속 3/20명` 다음 줄마다 이름 (최대를 모르면 `접속 3명`)
fn players(result: &Value, cat: &Catalog) -> String {
    let online = result.get("online").and_then(Value::as_u64).unwrap_or(0).to_string();
    let head = match result.get("max").and_then(Value::as_u64) {
        Some(max) => cat.t("actions.summary.players_max", &[("online", &online), ("max", &max.to_string())]),
        None => cat.t("actions.summary.players", &[("online", &online)]),
    };
    let lines: Vec<String> = items(result, "players").iter().map(|p| cat.t("actions.summary.player_item", &[("name", str_of(p, "name"))])).collect();
    if lines.is_empty() {
        return head;
    }
    let rest = lines.len().saturating_sub(MAX_ITEMS);
    let mut out = vec![head];
    out.extend(lines.into_iter().take(MAX_ITEMS));
    if rest > 0 {
        out.push(cat.t("actions.summary.players_more", &[("count", &rest.to_string())]));
    }
    out.join("\n")
}

/// 판정 한 줄 + 포트마다 상태 + 닫혔으면 무엇을 볼지. 공인 주소는 싣지 않는다 (채널의 다른 사람도 본다)
fn port_check(result: &Value, cat: &Catalog) -> String {
    let verdict = str_of(result, "verdict");
    let mut out = vec![cat.t(&format!("actions.summary.port_check.{}", verdict), &[])];
    for p in items(result, "ports") {
        let status = cat.t(&format!("actions.summary.port_status.{}", str_of(&p, "status")), &[]);
        let port = p.get("port").and_then(Value::as_u64).unwrap_or(0).to_string();
        out.push(cat.t("actions.summary.port_item", &[("port", &port), ("protocol", &str_of(&p, "protocol").to_uppercase()), ("status", &status)]));
    }
    if matches!(verdict, "firewall" | "router" | "closed" | "cgnat" | "unknown") {
        out.push(cat.t(&format!("actions.summary.port_hint.{}", verdict), &[]));
    }
    out.join("\n")
}

/// `backups[]` 항목마다 `label` (`2026-10-20 21:03 · 직접 · 120MB`, PC 시간대)
pub(super) fn label_backups(result: &mut Value, cat: &Catalog) {
    for b in result.get_mut("backups").and_then(Value::as_array_mut).into_iter().flatten() {
        let kind = cat.t(&format!("actions.summary.backup_kind.{}", str_of(b, "kind")), &[]);
        let when = local_time(b.get("created").and_then(Value::as_u64).unwrap_or(0));
        let note = unreadable_note(b, cat);
        // 게임 자동 백업(game-snapshot)은 크기를 재지 않는다
        b["label"] = Value::String(match b.get("size").and_then(Value::as_u64) {
            Some(size) => format!("{} · {} · {}{}", when, kind, bytes(size), note),
            None => format!("{} · {}{}", when, kind, note),
        });
    }
}

/// 열리지 않는 백업(`readable:false`)이면 ` · 열 수 없음` (옛 데몬처럼 칸이 없으면 붙이지 않는다)
fn unreadable_note(b: &Value, cat: &Catalog) -> String {
    match b.get("readable").and_then(Value::as_bool) {
        Some(false) => format!(" · {}", cat.t("actions.summary.backup_unreadable", &[])),
        _ => String::new(),
    }
}

fn events(result: &Value, cat: &Catalog) -> String {
    let mut events = items(result, "events");
    if events.is_empty() {
        return cat.t("actions.summary.events_empty", &[]);
    }
    events.sort_by_key(|e| std::cmp::Reverse(e.get("at").and_then(Value::as_u64).unwrap_or(0)));
    let lines = events
        .iter()
        .map(|e| {
            let kind = cat.t(&format!("actions.summary.event_kind.{}", str_of(e, "kind")), &[]);
            cat.t("actions.summary.event_item", &[("when", &local_time(e.get("at").and_then(Value::as_u64).unwrap_or(0))), ("kind", &kind)])
        })
        .collect();
    with_rest(cat.t("actions.summary.events", &[]), lines, cat)
}

/// 확인에 실패한 결과(`ok:false`)는 요약하지 않는다 (실패 문구는 따로 보인다)
fn updates(result: &Value, cat: &Catalog) -> Option<String> {
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let lines: Vec<String> = items(result, "components")
        .iter()
        .filter(|c| c.get("update_available").and_then(Value::as_bool) == Some(true))
        .map(|c| {
            let version = |key: &str| c.get(key).and_then(Value::as_str).unwrap_or("?").to_string();
            cat.t("actions.summary.update_item", &[("name", str_of(c, "display_name")), ("current", &version("current_version")), ("latest", &version("latest_version"))])
        })
        .collect();
    if lines.is_empty() {
        return Some(cat.t("actions.summary.updates_none", &[]));
    }
    Some(with_rest(cat.t("actions.summary.updates", &[("count", &lines.len().to_string())]), lines, cat))
}

fn monitor(result: &Value, cat: &Catalog) -> Option<String> {
    let cpu = result.pointer("/cpu/total_percent")?.as_f64()?;
    let used = result.pointer("/memory/used_bytes").and_then(Value::as_u64).unwrap_or(0);
    let total = result.pointer("/memory/total_bytes").and_then(Value::as_u64).unwrap_or(0);
    let head = cat.t("actions.summary.monitor", &[("cpu", &format!("{:.0}", cpu)), ("used", &bytes(used)), ("total", &bytes(total))]);
    let lines = items(result, "instances")
        .iter()
        .filter(|i| i.get("running").and_then(Value::as_bool) == Some(true))
        .map(|i| {
            let cpu = i.get("cpu_percent").and_then(Value::as_f64).unwrap_or(0.0);
            let memory = bytes(i.get("memory_bytes").and_then(Value::as_u64).unwrap_or(0));
            cat.t("actions.summary.monitor_item", &[("name", str_of(i, "name")), ("cpu", &format!("{:.0}", cpu)), ("memory", &memory)])
        })
        .collect();
    Some(with_rest(head, lines, cat))
}

/// 이 PC의 시각 (디스코드 사용자는 대개 PC 주인과 같은 시간대다)
pub(crate) fn local_time(unix_secs: u64) -> String {
    use chrono::TimeZone;
    if unix_secs == 0 {
        return "?".into();
    }
    chrono::Local.timestamp_opt(unix_secs as i64, 0).single().map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let n = n as f64;
    if n >= GB {
        format!("{:.1} GB", n / GB)
    } else {
        format!("{:.0} MB", n / MB)
    }
}
