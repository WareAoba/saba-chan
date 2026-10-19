//! `daemon …` — 데몬 켜기·끄기·상태·로그

use crate::bootstrap;
use crate::cli::DaemonCmd;
use crate::ctx::Ctx;
use crate::error::{failed, CliError, Result};
use crate::output::{self, s};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub async fn run(ctx: &Ctx, cmd: DaemonCmd) -> Result<()> {
    match cmd {
        DaemonCmd::Status => status(ctx).await,
        DaemonCmd::Start => start(ctx).await,
        DaemonCmd::Stop { force } => stop(ctx, force).await,
        DaemonCmd::Restart { force } => {
            stop(ctx, force).await?;
            start(ctx).await
        }
        DaemonCmd::Diagnostics => diagnostics(ctx).await,
        DaemonCmd::Logs { lines, follow } => {
            ctx.ensure_daemon().await?;
            crate::cmd::console::daemon_logs(ctx, lines, follow).await
        }
    }
}

fn is_down(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<CliError>(), Some(CliError::DaemonDown { .. }))
}

async fn status(ctx: &Ctx) -> Result<()> {
    match ctx.api.health().await {
        Ok(h) => {
            let v =
                json!({ "running": true, "version": h.get("version"), "pid": h.get("pid"), "url": ctx.api.base_url() });
            ctx.out.data(&v, |_| {
                println!(
                    "{}  {}",
                    output::status_badge("running"),
                    output::dim(&format!(
                        "v{} · pid {} · {}",
                        s(&h, "version"),
                        h.get("pid").and_then(Value::as_u64).unwrap_or(0),
                        ctx.api.base_url()
                    ))
                );
            });
            Ok(())
        }
        Err(e) if is_down(&e) => {
            let v = json!({ "running": false, "url": ctx.api.base_url() });
            ctx.out.data(&v, |_| {
                println!(
                    "{}  {}",
                    output::status_badge("stopped"),
                    output::dim(ctx.api.base_url())
                );
                ctx.out.hint("saba-chan-cli daemon start");
            });
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn start(ctx: &Ctx) -> Result<()> {
    if let Ok(h) = ctx.api.health().await {
        ctx.out.done(&t!("daemon.already", version = s(&h, "version")), &h);
        return Ok(());
    }
    let h = ctx.start_daemon().await?;
    ctx.out.done(&t!("daemon.started", version = s(&h, "version")), &h);
    Ok(())
}

async fn stop(ctx: &Ctx, force: bool) -> Result<()> {
    let h = match ctx.api.health().await {
        Ok(h) => h,
        Err(e) if is_down(&e) => {
            ctx.out.done(&t!("daemon.not_running"), &json!({ "running": false }));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let pid = h.get("pid").and_then(Value::as_u64);
    let asked = ctx.api.shutdown().await.is_ok();
    let spin = crate::progress::spinner(&t!("daemon.stopping"));
    let deadline = Instant::now() + bootstrap::STOP_TIMEOUT;
    while asked && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if ctx.api.health().await.is_err() {
            spin.finish_and_clear();
            ctx.out.done(&t!("daemon.stopped"), &json!({ "running": false }));
            return Ok(());
        }
    }
    spin.finish_and_clear();
    let pid = pid.ok_or_else(|| failed(t!("daemon.stop_no_pid")))?;
    if !force && !ctx.ask.confirm(&t!("daemon.confirm_kill", pid = pid), false)? {
        return Err(failed(t!("daemon.stop_timeout")));
    }
    bootstrap::kill_pid(pid)?;
    ctx.out.done(
        &t!("daemon.killed", pid = pid),
        &json!({ "running": false, "killed": pid }),
    );
    Ok(())
}

/// 도움을 청할 때 붙여 넣을 진단 (GUI 설정의 진단 정보와 같은 `GET /api/diagnostics`)
async fn diagnostics(ctx: &Ctx) -> Result<()> {
    let d = ctx.api.diagnostics().await?;
    ctx.out.data(&d, |d| {
        output::print_kv(&diagnostics_rows(d));
        let problems = d
            .get("recent_problems")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        println!();
        if problems.is_empty() {
            println!("{}", output::dim(&t!("daemon.diag.no_problems")));
            return;
        }
        output::heading(&t!("daemon.diag.problems"));
        for p in &problems {
            let lvl = s(p, "level").to_uppercase();
            let lvl = if lvl == "ERROR" {
                console::style(lvl).red()
            } else {
                console::style(lvl).yellow()
            };
            println!("  {:5} {}", lvl, s(p, "message"));
        }
    });
    Ok(())
}

fn diagnostics_rows(d: &Value) -> Vec<(String, String)> {
    let list = |v: Option<&Value>| {
        let names: Vec<&str> = v
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if names.is_empty() {
            "-".to_string()
        } else {
            names.join(", ")
        }
    };
    let token = &d["token_file"];
    let mut rows = vec![
        (t!("daemon.diag.version"), s(d, "version").to_string()),
        (
            t!("daemon.diag.uptime"),
            d.get("uptime_secs")
                .and_then(Value::as_u64)
                .map(output::duration)
                .unwrap_or_else(|| "-".into()),
        ),
        (
            "PID".to_string(),
            d.get("pid")
                .and_then(Value::as_u64)
                .map(|p| p.to_string())
                .unwrap_or_default(),
        ),
        (t!("daemon.diag.listen"), s(d, "listen_addr").to_string()),
        (t!("daemon.diag.data_dir"), s(d, "data_dir").to_string()),
        (
            t!("daemon.diag.token"),
            if token.get("exists").and_then(Value::as_bool) == Some(true) {
                s(token, "path").to_string()
            } else {
                t!("daemon.diag.token_missing", path = s(token, "path"))
            },
        ),
        (t!("daemon.diag.audit_log"), s(d, "audit_log").to_string()),
        (
            t!("daemon.diag.modules"),
            match d["modules"].get("count").and_then(Value::as_u64) {
                Some(n) => n.to_string(),
                None => t!("daemon.diag.modules_error", error = s(&d["modules"], "error")),
            },
        ),
        (t!("daemon.diag.extensions"), list(d["extensions"].get("enabled"))),
    ];
    let missing = list(d["extensions"].get("enabled_but_missing"));
    if missing != "-" {
        rows.push((t!("daemon.diag.extensions_missing"), missing));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GUI 진단과 같은 값을 빠짐없이 보인다: 없는 토큰 파일과 설치되지 않은 익스텐션이 눈에 띄어야 한다
    #[test]
    fn diagnostics_show_what_the_gui_shows() {
        let d = json!({
            "ok": true, "version": "0.1.7", "pid": 42, "uptime_secs": 3700,
            "listen_addr": "127.0.0.1:57474", "data_dir": "/data",
            "token_file": { "path": "/data/.ipc_token", "exists": false },
            "audit_log": "/data/audit.log",
            "extensions": { "enabled": ["backup", "music"], "enabled_but_missing": ["ghost"] },
            "modules": { "error": "bad toml" },
            "recent_problems": []
        });
        let rows = diagnostics_rows(&d);
        let text: Vec<&str> = rows.iter().map(|(_, v)| v.as_str()).collect();
        for want in [
            "0.1.7",
            "42",
            "127.0.0.1:57474",
            "/data",
            "/data/audit.log",
            "backup, music",
            "ghost",
        ] {
            assert!(text.contains(&want), "{want} missing from {rows:?}");
        }
        assert!(
            text.iter()
                .any(|v| v.contains("/data/.ipc_token") && *v != "/data/.ipc_token"),
            "missing token is marked: {rows:?}"
        );
        assert!(
            text.iter().any(|v| v.contains("bad toml")),
            "module error is shown: {rows:?}"
        );
        let healthy = json!({ "token_file": { "path": "/t", "exists": true }, "modules": { "count": 3 }, "extensions": { "enabled": [] } });
        let rows = diagnostics_rows(&healthy);
        assert!(rows.iter().any(|(_, v)| v == "/t") && rows.iter().any(|(_, v)| v == "3"));
        assert_eq!(rows.len(), 9, "no missing-extensions row when nothing is missing");
    }
}
