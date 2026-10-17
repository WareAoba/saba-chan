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
