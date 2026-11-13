//! 원격(디스코드·챗봇) 결과의 사람용 요약 (`text`). 요약이 없는 결과는 릴레이가 결과 그대로 보인다.
//! 문장은 릴레이가 만든다 (`Text`). 백업 종류 이름은 다른 화면(복원 칸)도 쓰는 데몬 문구다

use crate::chat::i18n::Catalog;
use crate::chat::Text;
use serde_json::Value;

/// 목록은 이만큼만 줄로 보이고 나머지는 개수로 센다
const MAX_ITEMS: usize = 10;

fn t(key: &str, vars: &[(&str, &str)]) -> Text {
    Text::key_with(format!("actions.summary.{key}"), vars)
}

/// 액션 결과(`execute`의 `result`) → 요약. 요약할 줄 모르는 액션이면 None
pub(crate) fn text(id: &str, result: &Value, cat: &Catalog) -> Option<Text> {
    match id {
        "instance.list" => Some(servers(result)),
        "backup.list" => Some(backups(result, cat)),
        "server.players" => Some(players(result)),
        "instance.events" => Some(events(result)),
        "update.check" => updates(result),
        "monitor.summary" => monitor(result),
        "server.port_check" => Some(port_check(result)),
        "server.address" => Some(address(result)),
        "server.update.check" => Some(server_update_check(result)),
        "server.update.apply" => Some(server_update_apply(result)),
        "server.update.cancel" => Some(t("server_update.cancelled", &[])),
        // 모듈 조회(접속자 목록 등)는 모양이 모듈마다 달라 채팅 명령 결과와 같은 범용 포맷으로
        _ if id.starts_with(super::QUERY_PREFIX) => {
            let mut shown = result.clone();
            if let Some(map) = shown.as_object_mut() {
                map.remove("success");
            }
            Some(crate::chat::format::format_response(&shown))
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

/// 머리 줄 + 항목 줄(`max`까지) + 남은 개수 (`more_key`)
fn head_and_items(head: Text, lines: Vec<Text>, more_key: &str) -> Text {
    let rest = lines.len().saturating_sub(MAX_ITEMS);
    let mut out = vec![head];
    out.extend(lines.into_iter().take(MAX_ITEMS));
    if rest > 0 {
        out.push(t(more_key, &[("count", &rest.to_string())]));
    }
    Text::lines(out)
}

fn with_rest(head: Text, lines: Vec<Text>) -> Text {
    head_and_items(head, lines, "more")
}

fn servers(result: &Value) -> Text {
    let servers = items(result, "servers");
    if servers.is_empty() {
        return Text::key("list.empty");
    }
    let lines = servers
        .iter()
        .map(|s| {
            let status = Text::key(if str_of(s, "status") == "running" { "status.running" } else { "status.stopped" });
            t("server_item", &[("name", str_of(s, "name")), ("module", str_of(s, "module"))]).var("status", status)
        })
        .collect();
    with_rest(Text::key("list.title"), lines)
}

fn backups(result: &Value, cat: &Catalog) -> Text {
    let mut backups = items(result, "backups");
    if backups.is_empty() {
        return t("backups_empty", &[]);
    }
    backups.sort_by_key(|b| std::cmp::Reverse(b.get("created").and_then(Value::as_u64).unwrap_or(0)));
    let lines = backups
        .iter()
        .map(|b| {
            let kind = backup_kind(b, cat);
            let when = local_time(b.get("created").and_then(Value::as_u64).unwrap_or(0));
            // 게임 자동 백업(game-snapshot)은 크기를 재지 않는다: 크기 자리가 없는 문구를 쓴다
            let line = match b.get("size").and_then(Value::as_u64).map(bytes) {
                Some(size) => t("backup_item", &[("name", str_of(b, "name")), ("kind", &kind), ("when", &when), ("size", &size)]),
                None => t("backup_item_unsized", &[("name", str_of(b, "name")), ("kind", &kind), ("when", &when)]),
            };
            Text::seq([line, unreadable_note(b, cat).into()])
        })
        .collect();
    with_rest(t("backups", &[("count", &backups.len().to_string())]), lines)
}

/// `접속 3/20명` 다음 줄마다 이름 (최대를 모르면 `접속 3명`)
fn players(result: &Value) -> Text {
    let online = result.get("online").and_then(Value::as_u64).unwrap_or(0).to_string();
    let head = match result.get("max").and_then(Value::as_u64) {
        Some(max) => t("players_max", &[("online", &online), ("max", &max.to_string())]),
        None => t("players", &[("online", &online)]),
    };
    let lines: Vec<Text> = items(result, "players").iter().map(|p| t("player_item", &[("name", str_of(p, "name"))])).collect();
    if lines.is_empty() {
        return head;
    }
    head_and_items(head, lines, "players_more")
}

/// 판정 한 줄 + 포트마다 상태 + 닫혔으면 무엇을 볼지. 공인 주소는 싣지 않는다 (채널의 다른 사람도 본다)
fn port_check(result: &Value) -> Text {
    let verdict = str_of(result, "verdict");
    let mut out = vec![t(&format!("port_check.{}", verdict), &[])];
    for p in items(result, "ports") {
        let status = t(&format!("port_status.{}", str_of(&p, "status")), &[]);
        let port = p.get("port").and_then(Value::as_u64).unwrap_or(0).to_string();
        out.push(t("port_item", &[("port", &port), ("protocol", &str_of(&p, "protocol").to_uppercase())]).var("status", status));
    }
    if matches!(verdict, "firewall" | "router" | "closed" | "cgnat" | "unknown") {
        out.push(t(&format!("port_hint.{}", verdict), &[]));
    }
    Text::lines(out)
}

/// `주소:포트` (IPv6는 `[주소]:포트`). 호스트 이름·IP가 아니면 없다
pub(crate) fn host_port(host: &str, port: u16) -> Option<String> {
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => Some(std::net::SocketAddr::new(ip, port).to_string()),
        Err(_) => crate::remote_daemon::valid_address(host).then(|| format!("{host}:{port}")),
    }
}

/// 포트마다 `주소:포트` 한 줄 (포트가 여럿이면 칸 이름을 앞에). 서버가 꺼져 있으면 그 말을 덧붙인다.
/// 주소는 PC의 공개 주소(도메인·IP)이고, 모르거나 밖에 여는 포트가 없으면 그 이유만
fn address(result: &Value) -> Text {
    // PC 주인이 디스코드에 알려 주지 않게 해 두었다 (`actions::hide_address`)
    if result.get("hidden").and_then(Value::as_bool) == Some(true) {
        return t("address.hidden", &[]);
    }
    let ports = items(result, "ports");
    if ports.is_empty() {
        return t("address.no_ports", &[]);
    }
    let Some(host) = result.get("address").and_then(Value::as_str).filter(|a| host_port(a, 1).is_some()) else {
        return t("address.unknown", &[]);
    };
    let mut out: Vec<Text> = ports
        .iter()
        .filter_map(|p| {
            let port = u16::try_from(p.get("port").and_then(Value::as_u64)?).ok()?;
            let shown = host_port(host, port)?;
            Some(if ports.len() == 1 {
                t("address.item", &[("address", &shown)])
            } else {
                t("address.item_named", &[("label", str_of(p, "label")), ("address", &shown)])
            })
        })
        .collect();
    if result.get("running").and_then(Value::as_bool) == Some(false) {
        out.push(t("address.stopped", &[]));
    }
    Text::lines(out)
}

/// 백업 종류 이름 (`backups.kind.<종류>`, 데몬 문구)
fn backup_kind(b: &Value, cat: &Catalog) -> String {
    cat.t(&format!("backups.kind.{}", str_of(b, "kind")), &[])
}

/// `backups[]` 항목마다 `label` (`2026-10-20 21:03 · 직접 · 120MB`, PC 시간대). GUI·CLI·디스코드의 복원 칸이 고를 때 보인다
pub(super) fn label_backups(result: &mut Value, cat: &Catalog) {
    for b in result.get_mut("backups").and_then(Value::as_array_mut).into_iter().flatten() {
        let kind = backup_kind(b, cat);
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
        Some(false) => format!(" · {}", cat.t("backups.unreadable", &[])),
        _ => String::new(),
    }
}

fn events(result: &Value) -> Text {
    let mut events = items(result, "events");
    if events.is_empty() {
        return t("events_empty", &[]);
    }
    events.sort_by_key(|e| std::cmp::Reverse(e.get("at").and_then(Value::as_u64).unwrap_or(0)));
    let lines = events
        .iter()
        .map(|e| {
            let kind = t(&format!("event_kind.{}", str_of(e, "kind")), &[]);
            t("event_item", &[("when", &local_time(e.get("at").and_then(Value::as_u64).unwrap_or(0)))]).var("kind", kind)
        })
        .collect();
    with_rest(t("events", &[]), lines)
}

/// 확인에 실패한 결과(`ok:false`)는 요약하지 않는다 (실패 문구는 따로 보인다)
fn updates(result: &Value) -> Option<Text> {
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let lines: Vec<Text> = items(result, "components")
        .iter()
        .filter(|c| c.get("update_available").and_then(Value::as_bool) == Some(true))
        .map(|c| {
            let version = |key: &str| c.get(key).and_then(Value::as_str).unwrap_or("?").to_string();
            t("update_item", &[("name", str_of(c, "display_name")), ("current", &version("current_version")), ("latest", &version("latest_version"))])
        })
        .collect();
    if lines.is_empty() {
        return Some(t("updates_none", &[]));
    }
    Some(with_rest(t("updates", &[("count", &lines.len().to_string())]), lines))
}

fn monitor(result: &Value) -> Option<Text> {
    let cpu = result.pointer("/cpu/total_percent")?.as_f64()?;
    let used = result.pointer("/memory/used_bytes").and_then(Value::as_u64).unwrap_or(0);
    let total = result.pointer("/memory/total_bytes").and_then(Value::as_u64).unwrap_or(0);
    let head = t("monitor", &[("cpu", &format!("{:.0}", cpu)), ("used", &bytes(used)), ("total", &bytes(total))]);
    let lines = items(result, "instances")
        .iter()
        .filter(|i| i.get("running").and_then(Value::as_bool) == Some(true))
        .map(|i| {
            let cpu = i.get("cpu_percent").and_then(Value::as_f64).unwrap_or(0.0);
            let memory = bytes(i.get("memory_bytes").and_then(Value::as_u64).unwrap_or(0));
            t("monitor_item", &[("name", str_of(i, "name")), ("cpu", &format!("{:.0}", cpu)), ("memory", &memory)])
        })
        .collect();
    Some(with_rest(head, lines))
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

/// 게임 서버 업데이트 확인: 새 빌드(빌드 번호), 최신, 확인 실패(이유), 사바쨩이 업데이트하지 않는 서버, 확인하지 않음
fn server_update_check(result: &Value) -> Text {
    let build = |key: &str| result.get(key).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).filter(|b| b != "null").unwrap_or_default();
    if result.get("update_available").and_then(Value::as_bool) == Some(true) {
        return t("server_update.available", &[("local", &build("local_buildid")), ("remote", &build("remote_buildid"))]);
    }
    match str_of(result, "reason") {
        "check_failed" => t("server_update.check_failed", &[("error", str_of(result, "error"))]),
        "update_unsupported" => t("server_update.unsupported", &[]),
        // 확인할 hook·데이터가 없어 확인하지 않았다 (no_hook_handled·no_extension_data 등): 최신이라고 하지 않는다
        r if !r.is_empty() => t("server_update.unchecked", &[]),
        _ => t("server_update.latest", &[("build", &build("local_buildid"))]),
    }
}

/// 게임 서버 업데이트를 걸었다: 켜진 서버는 예고 시간(과 지금 접속자), 꺼진 서버는 시작만
fn server_update_apply(result: &Value) -> Text {
    let minutes = result.get("warn_minutes").and_then(Value::as_u64).unwrap_or(0);
    // 아무도 없는 켜진 서버는 예고 없이 바로 끄고 업데이트한다
    if result.get("scheduled").and_then(Value::as_bool) != Some(true) || minutes == 0 {
        return t("server_update.started", &[]);
    }
    let head = t("server_update.scheduled", &[("minutes", &minutes.to_string())]);
    match result.get("players").and_then(Value::as_u64).filter(|p| *p > 0) {
        Some(players) => Text::lines([head, t("server_update.players", &[("count", &players.to_string())])]),
        None => head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 확인 실패는 "최신"이 아니라 그 이유로, 켜진 서버의 업데이트는 예고 시간과 접속자로 요약한다
    #[test]
    fn game_server_update_results_read_as_sentences() {
        let cat = Catalog::for_language("ko");
        let text = |id: &str, r: Value| super::text(id, &r, &cat).unwrap().plain();
        let failed = text("server.update.check", json!({"update_available": false, "reason": "check_failed", "error": "boom"}));
        assert!(failed.contains("check_failed") && failed.contains("boom"), "{failed}");
        // 확인할 hook·데이터가 없어 확인하지 않은 것도 "최신"이 아니다
        for reason in ["no_hook_handled", "no_extension_data"] {
            let unchecked = text("server.update.check", json!({"update_available": false, "reason": reason}));
            assert!(unchecked.contains("unchecked"), "{reason}: {unchecked}");
        }
        let newer = text("server.update.check", json!({"update_available": true, "local_buildid": "1", "remote_buildid": "2"}));
        assert!(newer.contains("local=1") && newer.contains("remote=2"), "{newer}");
        let now = text("server.update.apply", json!({"success": true, "scheduled": true, "warn_minutes": 0, "players": 0}));
        assert!(now.contains("server_update.started"), "{now}");
        let scheduled = text("server.update.apply", json!({"success": true, "scheduled": true, "warn_minutes": 5, "players": 2}));
        assert!(scheduled.contains("minutes=5") && scheduled.contains("count=2"), "{scheduled}");
        let started = text("server.update.apply", json!({"success": true, "provisioning": true}));
        assert!(started.contains("server_update.started"), "{started}");
    }
}
