//! `status` — 한눈에 보기, `version`

use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::output::{self, s};
use serde_json::{json, Value};
use std::collections::HashMap;

pub async fn run(ctx: &Ctx) -> Result<()> {
    let health = match ctx.api.health().await {
        Ok(h) => h,
        Err(e) if matches!(e.downcast_ref::<CliError>(), Some(CliError::DaemonDown { .. })) => {
            let v = json!({ "daemon": { "running": false, "url": ctx.api.base_url() } });
            ctx.out.data(&v, |_| {
                say!(
                    "{}  {}",
                    output::bold(&t!("status.daemon")),
                    output::status_badge("stopped")
                );
                ctx.out.hint(&t!("err.daemon_down_hint"));
            });
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    let (servers, modules, agent, updates, preflight, onboarding) = tokio::join!(
        ctx.api.servers(),
        ctx.api.modules(),
        ctx.api.agent(),
        ctx.api.update_status(),
        ctx.api.preflight(),
        ctx.api.onboarding()
    );
    let preflight = preflight.ok();
    let onboarding = onboarding.ok();
    let servers = servers?;
    let modules = modules.unwrap_or(Value::Null);

    let v = json!({
        "daemon": { "running": true, "version": health.get("version"), "pid": health.get("pid"), "url": ctx.api.base_url() },
        "servers": servers.get("servers"),
        "port_conflict_stops": servers.get("port_conflict_stops"),
        "modules_failed": modules.get("failed"),
        "discord": agent.as_ref().ok(),
        "updates": updates.as_ref().ok(),
        "preflight": preflight,
        "onboarding": onboarding,
    });
    ctx.out.data(&v, |_| {
        say!(
            "{}  {}  {}",
            output::bold(&t!("status.daemon")),
            output::status_badge("running"),
            output::dim(&format!(
                "v{} · pid {}",
                s(&health, "version"),
                health.get("pid").and_then(Value::as_u64).unwrap_or(0)
            ))
        );
        say!();
        print_server_table(
            servers
                .get("servers")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &modules,
        );
        for stop in servers
            .get("port_conflict_stops")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            ctx.out.warn(&t!(
                "status.port_conflict_stop",
                name = s(stop, "stopped_name"),
                other = s(stop, "existing_name"),
                port = stop.get("port").map(output::show).unwrap_or_default()
            ));
        }
        let failed = modules
            .get("failed")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if failed > 0 {
            ctx.out.warn(&t!("status.modules_failed", count = failed));
            ctx.out.hint("saba-chan-cli module list");
        }
        // 부팅 준비 (게임 모듈이 쓰는 Python 등). 끝나고 문제없으면 말하지 않는다.
        if let Some(p) = &preflight {
            if let Some(err) = p.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                ctx.out.warn(&t!("status.preflight_failed", error = err));
            } else if p.get("complete").and_then(Value::as_bool) == Some(false) && s(p, "phase") != "preflight-idle" {
                ctx.out.note(&t!(
                    "status.preflight_running",
                    percent = p.get("percent").and_then(Value::as_u64).unwrap_or(0)
                ));
            }
        }
        say!();
        if let Ok(a) = &agent {
            say!("{}  {}", output::bold(&t!("status.discord")), discord_line(a));
        }
        if let Ok(u) = &updates {
            let n = u.get("updates_available").and_then(Value::as_u64).unwrap_or(0);
            let line = if n > 0 {
                console::style(t!("status.updates_ready", count = n))
                    .yellow()
                    .to_string()
            } else {
                output::dim(&t!("status.up_to_date"))
            };
            say!("{}  {}", output::bold(&t!("status.updates")), line);
            if n > 0 {
                ctx.out.hint("saba-chan-cli update status");
            }
        }
        // 처음 쓰는 사람에게 다음 단계 하나 (J1-a). 다 했으면 말하지 않는다
        let next = onboarding.as_ref().and_then(|o| o.get("next")).and_then(Value::as_str);
        let step = match next {
            Some("module") => Some((t!("status.next_module"), "saba-chan-cli module install <module>")),
            Some("server") => Some((t!("status.next_server"), "saba-chan-cli server add")),
            Some("discord") => Some((t!("status.next_discord"), "saba-chan-cli bot pair")),
            _ => None,
        };
        if let Some((text, cmd)) = step {
            say!();
            ctx.out.note(&text);
            ctx.out.hint(cmd);
        }
    });
    Ok(())
}

/// 서버 상태 표시. 켜져 있지만 상태 질문에 답하지 않으면(`unresponsive`) 그것도 붙인다 (GUI 목록과 같게)
pub fn state_badge(sv: &Value) -> String {
    let mut state = output::status_badge(s(sv, "status"));
    if sv.get("unresponsive").and_then(Value::as_str).is_some() {
        state.push_str(&console::style(format!("  ⚠ {}", t!("server.unresponsive"))).yellow().to_string());
    }
    state
}

pub fn discord_line(a: &Value) -> String {
    let flag = |k: &str| a.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("running") {
        let mut line = output::status_badge("connected");
        if let Some(n) = a.get("commandsHandled").and_then(Value::as_u64).filter(|n| *n > 0) {
            line.push_str(&output::dim(&format!("  {}", t!("status.commands_handled", count = n))));
        }
        line
    } else if !flag("hasToken") {
        format!(
            "{}  {}",
            output::status_badge("not_paired"),
            output::dim("saba-chan-cli bot pair")
        )
    } else {
        format!(
            "{}  {}",
            output::status_badge("disconnected"),
            output::dim("saba-chan-cli bot start")
        )
    }
}

/// 모듈 이름 → 보여 줄 이름
pub fn module_display(modules: &Value) -> HashMap<String, String> {
    modules
        .get("modules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m.get("name")?.as_str()?.to_string();
            let display = m
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_string();
            Some((name, display))
        })
        .collect()
}

pub fn print_server_table(servers: &[Value], modules: &Value) {
    if servers.is_empty() {
        say!("{}", output::dim(&t!("server.none_yet")));
        return;
    }
    let names = module_display(modules);
    let mut t = output::table(&[
        t!("col.name"),
        t!("col.game"),
        t!("col.state"),
        t!("col.port"),
        t!("col.uptime"),
    ]);
    for sv in servers {
        let mut name = s(sv, "name").to_string();
        for b in crate::slots::badges(sv) {
            name.push(' ');
            name.push_str(&b);
        }
        let module = s(sv, "module");
        let uptime = sv
            .get("start_time")
            .and_then(Value::as_u64)
            .filter(|_| s(sv, "status") == "running")
            .map(|st| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(st);
                output::duration(now.saturating_sub(st))
            })
            .unwrap_or_default();
        let mut state = state_badge(sv);
        if let Some(p) = output::players(sv) {
            state.push_str(&format!("  👥 {p}"));
        }
        if sv
            .get("port_conflicts")
            .and_then(Value::as_array)
            .is_some_and(|c| !c.is_empty())
        {
            state.push_str(
                &console::style(format!("  {}", t!("server.port_conflict")))
                    .red()
                    .to_string(),
            );
        }
        t.add_row(vec![
            name,
            names.get(module).cloned().unwrap_or_else(|| module.to_string()),
            state,
            sv.get("port").map(output::show).unwrap_or_default(),
            uptime,
        ]);
    }
    output::print_table(t);
}

pub async fn version(ctx: &Ctx) -> Result<()> {
    let cli = env!("CARGO_PKG_VERSION");
    let (daemon, installed) = tokio::join!(ctx.api.version(), ctx.api.system_components());
    let (daemon, installed) = (daemon.ok(), installed.ok());
    let v = json!({ "cli": cli, "daemon": daemon, "installed": installed });
    ctx.out.data(&v, |_| {
        let mut rows = Vec::new();
        if let Some(rel) = installed.as_ref().map(|i| s(i, "version")).filter(|r| !r.is_empty()) {
            rows.push((t!("version.release"), console::style(rel).bold().to_string()));
        }
        rows.push(("saba-chan-cli".to_string(), cli.to_string()));
        match &daemon {
            Some(d) => {
                if let Some(map) = d.get("components").and_then(Value::as_object) {
                    rows.extend(map.iter().map(|(k, v)| (k.clone(), output::show(v))));
                }
            }
            None => rows.push(("saba-core".into(), output::dim(&t!("status.daemon_off")))),
        }
        // 업데이터가 기록한 구성요소 중 위에 없는 것 (GUI·업데이터 등)
        if let Some(map) = installed
            .as_ref()
            .and_then(|i| i.get("components"))
            .and_then(Value::as_object)
        {
            for (k, v) in map {
                // "cli"는 위의 saba-chan-cli 줄과 같다
                if k != "cli" && !rows.iter().any(|(n, _)| n == k) {
                    rows.push((k.clone(), output::show(v)));
                }
            }
        }
        if let Some(at) = installed
            .as_ref()
            .and_then(|i| i.get("lastUpdated"))
            .and_then(Value::as_str)
            .and_then(|t| t.parse::<u64>().ok())
        {
            rows.push((t!("version.last_updated"), output::ago(at)));
        }
        output::print_kv(&rows);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 켜져 있지만 응답이 없는 서버(H1)를 목록·상태에서도 알 수 있게 (C3)
    #[test]
    fn an_unresponsive_server_is_marked() {
        crate::i18n::init("ko");
        let plain = |v: Value| console::strip_ansi_codes(&state_badge(&v)).to_string();
        assert!(plain(json!({ "status": "running", "unresponsive": "timed out" })).contains(&t!("server.unresponsive")));
        assert!(!plain(json!({ "status": "running" })).contains(&t!("server.unresponsive")));
    }
}
