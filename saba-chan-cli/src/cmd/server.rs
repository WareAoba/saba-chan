//! `server …` 와 바로가기 `start/stop/restart/run`

use crate::catalog::{Catalog, Command, Input, Risk};
use crate::cli::{AddArgs, PropsCmd, RestartArgs, RunArgs, ServerCmd, StartArgs, StopArgs};
use crate::cmd::{split_pairs, typed};
use crate::ctx::Ctx;
use crate::error::{failed, usage, CliError, Result};
use crate::output::{self, dim, s, show, status_of};
use crate::resolve::{self, Server};
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

mod add;
mod checkup;
mod command;
mod settings;

use self::add::{add, follow_provision, has_comma, remove};
use self::checkup::{
    agree, bool_option, checkup, diagnose, install_choices, launch_preview, module_agreements, offer_agreements,
    option_label, platform_refusal, reorder, reset, setting_label, show_agreements, split_agreements, validate,
};
pub use self::command::{exec_line, print_commands, print_value, run_command};
use self::command::{
    add_settings_failure, address, api_code, chat, commands, lists, log, move_refusal, players, port_check, query, rcon,
    rest, send, sync_failure, tell_moved_ports,
};
pub use self::settings::load_settings;
use self::settings::{insert_patch, properties, set, setting_fields, settings, typed_setting};

pub async fn run(ctx: &Ctx, cmd: ServerCmd) -> Result<()> {
    match cmd {
        ServerCmd::List => list(ctx).await,
        ServerCmd::Info { server } => info(ctx, server.as_deref()).await,
        ServerCmd::Add(a) => add(ctx, a).await,
        ServerCmd::Remove { server, keep_settings } => remove(ctx, server.as_deref(), keep_settings).await,
        ServerCmd::Start(a) => start(ctx, a).await,
        ServerCmd::Stop(a) => stop(ctx, a).await,
        ServerCmd::Restart(a) => restart(ctx, a).await,
        ServerCmd::Rename { server, name } => rename(ctx, Some(&server), &name).await,
        ServerCmd::Console(a) => crate::cmd::console::run(ctx, a).await,
        ServerCmd::Run(a) => run_command(ctx, a).await,
        ServerCmd::Commands { server } => commands(ctx, server.as_deref()).await,
        ServerCmd::Discord { server, word, command, clear } => {
            let change = crate::cmd::discord_words::WordsChange { words: word, commands: command, clear };
            crate::cmd::discord_words::run(ctx, server.as_deref(), change).await
        }
        ServerCmd::Send { server, line } => send(ctx, &server, &line.join(" ")).await,
        ServerCmd::Chat { server, name, text } => chat(ctx, &server, &name, &text.join(" ")).await,
        ServerCmd::Players { server } => players(ctx, server.as_deref()).await,
        ServerCmd::Lists { server } => lists(ctx, server.as_deref()).await,
        ServerCmd::PortCheck { server } => port_check(ctx, server.as_deref()).await,
        ServerCmd::Address { server } => address(ctx, server.as_deref()).await,
        ServerCmd::Rcon { server, line } => rcon(ctx, &server, &line.join(" ")).await,
        ServerCmd::Log { server, source, lines, runs, all } => log(ctx, server.as_deref(), source.as_deref(), lines, runs, all).await,
        ServerCmd::Query { server, name, args } => query(ctx, &server, name.as_deref(), &args).await,
        ServerCmd::Settings { server } => settings(ctx, server.as_deref()).await,
        ServerCmd::Set { server, pairs } => set(ctx, &server, &pairs).await,
        ServerCmd::Properties { server, action } => properties(ctx, server.as_deref(), action).await,
        ServerCmd::Rest {
            server,
            method,
            endpoint,
            body,
        } => rest(ctx, &server, &method, &endpoint, body.as_deref()).await,
        ServerCmd::Version { server } => version(ctx, server.as_deref()).await,
        ServerCmd::Update { server, check } => update(ctx, server.as_deref(), check).await,
        ServerCmd::Progress { server, dismiss } => progress(ctx, &server, dismiss).await,
        ServerCmd::History { server } => history(ctx, server.as_deref()).await,
        ServerCmd::Validate { server } => validate(ctx, server.as_deref()).await,
        ServerCmd::Diagnose { server } => diagnose(ctx, server.as_deref()).await,
        ServerCmd::LaunchPreview { server, extra_args } => launch_preview(ctx, server.as_deref(), extra_args.as_deref()).await,
        ServerCmd::Agree { server } => agree(ctx, server.as_deref()).await,
        ServerCmd::Reset { server } => reset(ctx, server.as_deref()).await,
        ServerCmd::Reorder { servers } => reorder(ctx, &servers).await,
    }
}

// ── 공용 도우미 ──

/// `/api/servers`에서 이 서버의 실행 상태
pub async fn runtime(ctx: &Ctx, sv: &Server) -> Result<Value> {
    let list = ctx.api.servers(true).await?;
    Ok(list
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|x| s(x, "id") == sv.id).cloned())
        .unwrap_or(Value::Null))
}

/// `/api/modules` 항목 하나
pub async fn module_info(ctx: &Ctx, name: &str) -> Result<Value> {
    resolve::modules(ctx)
        .await?
        .into_iter()
        .find(|m| s(m, "name") == name)
        .ok_or_else(|| {
            CliError::NotFound {
                what: t!("noun.module"),
                query: name.into(),
                suggestions: vec![],
            }
            .into()
        })
}

/// 지금 언어의 모듈 문구 (없으면 영어)
pub async fn module_locales(ctx: &Ctx, name: &str) -> Option<Value> {
    let all = ctx.api.module_locales(name).await.ok()?;
    all.get(crate::i18n::lang()).or_else(|| all.get("en")).cloned()
}

pub async fn catalog_for(ctx: &Ctx, module: &str) -> Result<Catalog> {
    let (info, locales) = tokio::join!(module_info(ctx, module), module_locales(ctx, module));
    Ok(Catalog::from_module(&info?, locales.as_ref()))
}

/// 서버 하나의 명령: 모듈 명령(번역됨) + 그 서버에 켜진 익스텐션 명령
pub async fn catalog_for_server(ctx: &Ctx, sv: &Server) -> Result<Catalog> {
    let (cat, list) = tokio::join!(catalog_for(ctx, &sv.module), ctx.api.instance_commands(&sv.id));
    let mut cat = cat?;
    if let Some(list) = list
        .ok()
        .as_ref()
        .and_then(|v| v.get("commands"))
        .and_then(Value::as_array)
    {
        cat.add_instance_commands(list);
    }
    Ok(cat)
}

/// 서버 상태가 원하는 값이 될 때까지 기다린다. 시간이 넘으면 마지막 상태를 돌려준다.
async fn wait_status(ctx: &Ctx, sv: &Server, want: &[&str], timeout: Duration, msg: &str) -> Result<String> {
    let spin = crate::progress::spinner(msg);
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    loop {
        if let Ok(rt) = runtime(ctx, sv).await {
            last = status_of(&rt).to_string();
            if want.contains(&last.as_str()) {
                break;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(800)) => {}
            _ = tokio::signal::ctrl_c() => { spin.finish_and_clear(); return Err(CliError::Cancelled.into()); }
        }
    }
    spin.finish_and_clear();
    Ok(last)
}

/// 상태가 `window` 동안 running으로 버티는지 본다. 꺼지면 그 상태를 돌려준다.
async fn settle(ctx: &Ctx, sv: &Server, window: Duration) -> String {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(rt) = runtime(ctx, sv).await {
            let st = status_of(&rt);
            if st != "running" && st != "starting" {
                return st.to_string();
            }
        }
    }
    "running".to_string()
}

/// 이름에 공백이 있으면 따옴표
pub fn shell_word(s: &str) -> String {
    if s.contains(char::is_whitespace) {
        format!("\"{}\"", s)
    } else {
        s.to_string()
    }
}
// ── 목록·정보 ──

async fn list(ctx: &Ctx) -> Result<()> {
    let (servers, modules) = tokio::join!(ctx.api.servers(true), ctx.api.modules());
    let servers = servers?;
    let modules = modules.unwrap_or(Value::Null);
    ctx.out.data(servers.get("servers").unwrap_or(&json!([])), |list| {
        crate::cmd::status::print_server_table(list.as_array().map(Vec::as_slice).unwrap_or(&[]), &modules);
    });
    Ok(())
}

async fn info(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let (rt, module, events, progress) = tokio::join!(
        runtime(ctx, &sv),
        module_info(ctx, &sv.module),
        ctx.api.exit_events(&sv.id),
        ctx.api.provision_progress(&sv.name)
    );
    let rt = rt?;
    let module = module.unwrap_or(Value::Null);
    let last_exit = events.ok().and_then(|e| {
        e.get("events")
            .and_then(Value::as_array)
            .and_then(|a| a.last().cloned())
    });
    let progress = progress
        .ok()
        .filter(|p| p.get("active").and_then(Value::as_bool) == Some(true));

    let missing = crate::cmd::module::unsupported(&module);
    let v = json!({ "instance": sv.raw, "runtime": rt, "last_exit": last_exit, "provisioning": progress, "unsupported": missing });
    ctx.out.data(&v, |_| {
        let game = s(&module, "display_name");
        output::heading(&format!(
            "{}  {}",
            sv.name,
            output::dim(if game.is_empty() { &sv.module } else { game })
        ));
        let mut rows = vec![
            (t!("col.state"), output::status_badge(status_of(&rt))),
            (t!("col.id"), output::dim(&sv.id)),
        ];
        // 켜져 있지만 상태 질문에 답하지 않음: 명령·정상 종료도 닿지 않을 수 있다
        if let Some(reason) = rt.get("unresponsive").and_then(Value::as_str) {
            rows.push((t!("server.unresponsive"), console::style(t!("server.unresponsive_reason", reason = reason)).yellow().to_string()));
        }
        if let Some(st) = rt
            .get("start_time")
            .and_then(Value::as_u64)
            .filter(|_| status_of(&rt) == "running")
        {
            rows.push((t!("col.started"), output::ago(st)));
        }
        if let Some(p) = output::players(&rt) {
            rows.push((t!("col.players"), p));
        }
        if let Some(pid) = rt.get("pid").and_then(Value::as_u64) {
            rows.push(("PID".into(), pid.to_string()));
        }
        for (key, label) in [("port", "col.port"), ("rest_port", "col.rest_port")] {
            if let Some(p) = sv.raw.get(key).filter(|p| !p.is_null()) {
                rows.push((t!(label), show(p)));
            }
        }
        if let Some(v) = installed_version(&sv.raw) {
            rows.push((t!("col.version"), v));
        }
        if let Some(dir) = sv.raw.get("working_dir").and_then(Value::as_str) {
            rows.push((t!("col.folder"), dir.to_string()));
        }
        rows.extend(crate::slots::detail_rows(&rt));
        for c in rt.get("port_conflicts").and_then(Value::as_array).into_iter().flatten() {
            rows.push((
                t!("server.port_conflict"),
                t!(
                    "server.port_conflict_with",
                    port = show(&c["port"]),
                    name = s(c, "conflict_name")
                ),
            ));
        }
        if let Some(e) = &last_exit {
            rows.push((t!("col.last_exit"), exit_line(e)));
        }
        if !missing.is_empty() {
            rows.push((t!("col.unsupported"), missing.join(", ")));
        }
        output::print_kv(&rows);
        if let Some(p) = &progress {
            say!();
            say!("{}", progress_line(p));
        }
    });
    Ok(())
}

fn exit_line(e: &Value) -> String {
    let kind = s(e, "kind");
    let label = crate::i18n::try_t(&format!("exit.{}", kind)).unwrap_or_else(|| kind.to_string());
    let when = e.get("at").and_then(Value::as_u64).map(output::ago).unwrap_or_default();
    let mut line = format!("{} · {}", label, when);
    if let Some(secs) = e.get("restart_in_secs").and_then(Value::as_u64) {
        line.push_str(&format!(" · {}", t!("exit.restarting_in", secs = secs)));
    }
    line
}

fn progress_line(p: &Value) -> String {
    let pct = p
        .get("percent")
        .and_then(Value::as_u64)
        .map(|n| format!("{}% ", n))
        .unwrap_or_default();
    match p.get("error").and_then(Value::as_str) {
        Some(err) => format!("{} {}", console::style("✗").red(), err),
        None => format!("{} {}{}", console::style("◐").dim(), pct, s(p, "message")),
    }
}

// ── 시작·정지 ──

pub async fn start(ctx: &Ctx, a: StartArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    let rt = runtime(ctx, &sv).await?;
    match status_of(&rt) {
        "running" => {
            ctx.out.done(
                &t!("start.already", name = sv.name),
                &json!({ "success": true, "status": "running" }),
            );
        }
        "provisioning" => return Err(failed(t!("start.provisioning", name = sv.name))),
        _ => {
            let res = ctx.api.start(&sv.id).await;
            tell_moved_ports(ctx, &res);
            finish_start(ctx, &sv, res, a.no_wait, t!("start.done", name = sv.name)).await?;
        }
    }
    if a.console {
        return Box::pin(crate::cmd::console::run(
            ctx,
            crate::cli::ConsoleArgs {
                server: Some(sv.id),
                lines: 30,
                ..Default::default()
            },
        ))
        .await;
    }
    Ok(())
}

/// 시작 요청의 결과를 마무리한다: 점검 실패·약관 안내, 켜질 때까지 기다리고 바로 꺼지지 않는지 본다.
/// 데몬이 켜기 전에 점검한다 (약관 미동의 등). 막히면 409 `preflight_failed`와 문제 목록·해결법. `start`와 `restart`가 같이 쓴다 (L56)
async fn finish_start(ctx: &Ctx, sv: &Server, res: Result<Value>, no_wait: bool, done: String) -> Result<()> {
    let r = match res {
        Err(e) if api_code(&e) == Some("preflight_failed") => {
            return checkup(ctx, Err(e), "", &sv.name).await;
        }
        Err(e) if api_code(&e) == Some("agreement_required") => {
            ctx.out.hint(&format!("saba-chan-cli server agree {}", shlex::try_quote(&sv.name).unwrap_or_else(|_| sv.name.as_str().into())));
            return Err(e);
        }
        r => r?,
    };
    if no_wait {
        ctx.out.done(&t!("start.requested", name = sv.name), &r);
    } else {
        let st = wait_status(
            ctx,
            sv,
            &["running"],
            Duration::from_secs(90),
            &t!("start.waiting", name = sv.name),
        )
        .await?;
        // 프로세스가 떴다가 바로 꺼지는 경우를 잡는다 (설정 오류, 약관 등)
        let st = if st == "running" {
            settle(ctx, sv, Duration::from_secs(3)).await
        } else {
            st
        };
        if st == "running" {
            ctx.out.done(&done, &r);
        } else if matches!(st.as_str(), "stopped" | "crashed") {
            if !ctx.out.json {
                let tail = ctx.api.console(&sv.id, None, Some(6)).await.unwrap_or(Value::Null);
                for l in tail.get("lines").and_then(Value::as_array).into_iter().flatten() {
                    tell!("  {}", crate::cmd::console::format_line(l));
                }
            }
            ctx.out
                .hint(&format!("saba-chan-cli server history {}", shell_word(&sv.name)));
            return Err(CliError::Failed(t!("start.exited", name = sv.name)).into());
        } else {
            ctx.out.warn(&t!("start.not_yet", name = sv.name, status = st));
            ctx.out.hint(&format!("saba-chan-cli console {}", shell_word(&sv.name)));
            ctx.out
                .hint(&format!("saba-chan-cli server history {}", shell_word(&sv.name)));
            return Err(CliError::Failed(t!("start.not_yet", name = sv.name, status = st)).into());
        }
    }
    Ok(())
}

pub async fn stop(ctx: &Ctx, a: StopArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    ensure_stop(ctx, &sv, "stop").await?;
    stop_server(ctx, &sv, a.force, a.no_wait).await
}

/// 켜진 서버를 끊는 명령(정지·재시작)은 앱·웹·CLI에서 늘 묻는다 (GUI 확인 창과 같은 고정 규칙). 꺼진 서버는 끊을 것이 없어 묻지 않는다
async fn ensure_stop(ctx: &Ctx, sv: &Server, cmd: &str) -> Result<()> {
    if status_of(&runtime(ctx, sv).await?) == "stopped" {
        return Ok(());
    }
    ask_stop(ctx, sv, cmd)
}

/// `--yes`면 묻지 않는다
fn ask_stop(ctx: &Ctx, sv: &Server, cmd: &str) -> Result<()> {
    ctx.ask.ensure(&t!(&format!("{cmd}.confirm"), name = sv.name))
}

pub async fn stop_server(ctx: &Ctx, sv: &Server, force: bool, no_wait: bool) -> Result<()> {
    if status_of(&runtime(ctx, sv).await?) == "stopped" {
        ctx.out.done(
            &t!("stop.already", name = sv.name),
            &json!({ "success": true, "status": "stopped" }),
        );
        return Ok(());
    }
    let r = ctx.api.stop(&sv.id, force).await?;
    if no_wait {
        ctx.out.done(&t!("stop.requested", name = sv.name), &r);
        return Ok(());
    }
    let st = wait_status(
        ctx,
        sv,
        &["stopped"],
        Duration::from_secs(90),
        &t!("stop.waiting", name = sv.name),
    )
    .await?;
    if st == "stopped" {
        ctx.out.done(&t!("stop.done", name = sv.name), &r);
        Ok(())
    } else {
        ctx.out
            .hint(&format!("saba-chan-cli stop {} --force", shell_word(&sv.name)));
        Err(failed(t!("stop.not_yet", name = sv.name)))
    }
}

pub async fn restart(ctx: &Ctx, a: RestartArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    ensure_stop(ctx, &sv, "restart").await?;
    let spin = crate::progress::spinner(&t!("restart.waiting", name = sv.name));
    let r = ctx.api.restart(&sv.id, a.force).await;
    spin.finish_and_clear();
    tell_moved_ports(ctx, &r);
    finish_start(ctx, &sv, r, false, t!("restart.done", name = sv.name)).await
}

pub async fn rename(ctx: &Ctx, query: Option<&str>, new_name: &str) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(usage(t!("add.name_empty")));
    }
    if has_comma(new_name) {
        return Err(usage(t!("add.name_comma")));
    }
    let r = ctx.api.rename(&sv.id, new_name).await?;
    ctx.out.done(&t!("rename.done", old = sv.name, name = new_name), &r);
    Ok(())
}

// ── 버전·업데이트 ──

/// 서버 정보의 버전 칸: 데몬이 적어 둔 설치 빌드(설치 방식 익스텐션으로 받은 서버), 없으면 서버 버전
fn installed_version(raw: &Value) -> Option<String> {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
    match text(raw.pointer("/server_build/id")) {
        Some(id) => Some(t!("col.build", id = id)),
        None => text(raw.get("server_version")),
    }
}

async fn version(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.installed_version(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let v = r
            .get("version")
            .or_else(|| r.get("server_version"))
            .map(show)
            .unwrap_or_else(|| output::dim(&t!("server.version_unknown")));
        say!("{}  {}", sv.name, v);
    });
    Ok(())
}

async fn update(ctx: &Ctx, query: Option<&str>, check_only: bool) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let spin = crate::progress::spinner(&t!("server.checking_update", name = sv.name));
    let r = ctx.api.check_server_update(&sv.id).await;
    spin.finish_and_clear();
    let r = r?;
    let available = r.get("update_available").and_then(Value::as_bool) == Some(true);
    if check_only || !available {
        ctx.out.data(&r, |r| {
            if available {
                say!("{}", t!("server.update_available", name = sv.name));
                ctx.out
                    .hint(&format!("saba-chan-cli server update {}", shell_word(&sv.name)));
            } else if s(r, "reason") == "check_failed" {
                // 확인하지 못했다: "최신"이라고 말하지 않는다
                say!("{}", t!("server.update_check_failed", name = sv.name, error = s(r, "error")));
            } else if s(r, "reason") == "update_unsupported" {
                // 다운로드·수동 설치 서버: 최신인지 알 수 없다
                say!("{}", t!("err.hint.update_unsupported"));
            } else {
                let reason = s(r, "reason");
                let tail = crate::i18n::try_t(&format!("server.update_reason.{}", reason)).unwrap_or_default();
                say!("{} {}", t!("server.update_none", name = sv.name), output::dim(&tail));
            }
        });
        return Ok(());
    }
    if status_of(&runtime(ctx, &sv).await?) == "running" {
        return Err(failed(t!("err.hint.server_running")));
    }
    ctx.ask.ensure(&t!("server.confirm_update", name = sv.name))?;
    let spin = crate::progress::spinner(&t!("server.updating", name = sv.name));
    let r = ctx.api.apply_server_update(&sv.id).await;
    spin.finish_and_clear();
    let r = r?;
    // 적용은 백그라운드 설치 진행으로 이어질 수 있다
    if ctx
        .api
        .provision_progress(&sv.name)
        .await
        .ok()
        .and_then(|p| p.get("active").and_then(Value::as_bool))
        == Some(true)
    {
        let p = follow_provision(ctx, &sv.name, false).await?;
        if let Some(err) = p.get("error").and_then(Value::as_str) {
            let _ = ctx.api.dismiss_provision(&sv.name).await;
            return Err(failed(err.to_string()));
        }
    }
    ctx.out.done(&t!("server.updated", name = sv.name), &r);
    Ok(())
}

async fn progress(ctx: &Ctx, query: &str, dismiss: bool) -> Result<()> {
    // 실패로 롤백된 서버는 목록에 없을 수 있으니 이름 그대로도 받는다
    let name = match resolve::server(ctx, Some(query)).await {
        Ok(sv) => sv.name,
        Err(_) => query.to_string(),
    };
    if dismiss {
        let r = ctx.api.dismiss_provision(&name).await?;
        let msg = if r.get("rolled_back").and_then(Value::as_bool) == Some(true) {
            t!("progress.rolled_back", name = name)
        } else {
            t!("progress.dismissed", name = name)
        };
        ctx.out.done(&msg, &r);
        return Ok(());
    }
    let p = ctx.api.provision_progress(&name).await?;
    if p.get("active").and_then(Value::as_bool) != Some(true) {
        ctx.out.data(&p, |_| say!("{}", t!("progress.none", name = name)));
        return Ok(());
    }
    if ctx.out.json || p.get("done").and_then(Value::as_bool) == Some(true) {
        // 끝난 작업이 실패였으면 진행 줄만 보이고 성공으로 끝내지 않는다
        if let Some(err) = p.get("done").and_then(Value::as_bool).filter(|d| *d).and(p.get("error")).and_then(Value::as_str) {
            return Err(CliError::Partial { message: err.to_string(), data: p.clone() }.into());
        }
        ctx.out.data(&p, |p| say!("{}", progress_line(p)));
        return Ok(());
    }
    let r = follow_provision(ctx, &name, false).await?;
    match r.get("error").and_then(Value::as_str) {
        Some(err) => Err(failed(err.to_string())),
        None => {
            ctx.out.done(&t!("progress.finished", name = name), &r);
            Ok(())
        }
    }
}

async fn history(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.exit_events(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let events = r.get("events").and_then(Value::as_array).cloned().unwrap_or_default();
        if events.is_empty() {
            say!("{}", output::dim(&t!("history.none")));
            return;
        }
        let mut t = output::table(&[t!("col.when"), t!("col.kind"), t!("col.code"), t!("col.why")]);
        for e in events.iter().rev() {
            let kind = s(e, "kind");
            let label = crate::i18n::try_t(&format!("exit.{}", kind)).unwrap_or_else(|| kind.to_string());
            let label = if kind == "crash" {
                console::style(label).red().to_string()
            } else {
                label
            };
            let mut why = s(e, "evidence").to_string();
            if let Some(note) = e.get("note").and_then(Value::as_str) {
                why = format!("{} ({})", why, note);
            }
            t.add_row(vec![
                e.get("at").and_then(Value::as_u64).map(output::ago).unwrap_or_default(),
                label,
                e.get("exit_code").map(show).unwrap_or_default(),
                why,
            ]);
        }
        output::print_table(t);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{add::*, checkup::*, command::*, settings::*};
    /// 설치 방식 익스텐션으로 받은 서버는 데몬이 적어 둔 빌드 번호를 버전으로 보인다
    #[test]
    fn server_info_shows_the_recorded_build_as_the_version() {
        crate::i18n::init("ko");
        assert_eq!(super::installed_version(&serde_json::json!({ "server_build": { "id": "19283746" } })).as_deref(), Some("빌드 19283746"));
        assert_eq!(super::installed_version(&serde_json::json!({ "server_version": "1.21.10" })).as_deref(), Some("1.21.10"));
        assert_eq!(super::installed_version(&serde_json::json!({ "server_version": null })), None);
    }

    use super::*;

    /// 데몬이 정리한 명령 결과(`output`)의 이름: 값 목록은 이름 폭을 맞춰 한 줄씩 (한글 이름도)
    #[test]
    fn output_fields_line_up_their_values() {
        let fields = json!([
            { "key": "serverfps", "label": "서버 FPS", "value": "59" },
            { "key": "uptime", "label": "uptime", "value": "7234" },
        ]);
        assert_eq!(field_lines(fields.as_array().unwrap()), vec!["서버 FPS  59", "uptime    7234"]);
    }

    /// 설치 진행 항목이 끝(done)을 보이기 전에 사라지면(데몬 재시작, 다른 곳에서 이름 바꾸기·삭제) 성공이 아니다 (L52)
    #[test]
    fn a_vanished_install_is_not_reported_as_finished() {
        assert!(provision_lost(&json!({ "active": false })));
        assert!(provision_lost(&json!({})));
        assert!(!provision_lost(&json!({ "active": true, "done": false })));
        assert!(!provision_lost(&json!({ "active": true, "done": true })));
    }

    /// 정지·재시작은 묻고, `--yes`면 묻지 않는다. 물을 수 없는데 `--yes`도 없으면 실행하지 않는다
    #[test]
    fn stopping_or_restarting_asks_unless_yes_is_given() {
        crate::i18n::init("ko");
        let ctx = |ask| Ctx {
            api: crate::client::Api::new(1, None),
            spawn_port: 1,
            port_fixed: true,
            out: crate::output::Out::new(true, false),
            ask,
            in_shell: false,
            attached: false,
        };
        let sv = Server { id: "i".into(), name: "mc".into(), module: "minecraft".into(), raw: json!({}) };
        for cmd in ["stop", "restart"] {
            assert!(ask_stop(&ctx(crate::prompt::Prompt { enabled: false, yes: true }), &sv, cmd).is_ok());
            assert!(ask_stop(&ctx(crate::prompt::Prompt { enabled: false, yes: false }), &sv, cmd).is_err());
            assert!(t!(&format!("{cmd}.confirm"), name = "mc").contains("mc"), "{cmd}");
        }
    }

    /// 가져오기 "옮기기"가 거절되면 데몬 영어 문구 대신 현지화된 까닭과 할 일 한 줄만 보인다
    #[test]
    fn a_refused_move_reads_in_the_users_language() {
        crate::i18n::init("ko");
        let refused = crate::client::check(409, json!({ "success": false, "error_code": "move_cross_volume", "error": "Could not move the server folder into saba-chan (move_cross_volume)" })).unwrap_err();
        let shown = move_refusal(refused).to_string();
        assert_eq!(shown, t!("err.hint.move_cross_volume"));
        let other = crate::client::check(409, json!({ "success": false, "error_code": "name_taken", "error": "x" })).unwrap_err();
        assert_eq!(move_refusal(other).to_string(), "x");
    }

    /// 가져온 서버는 지우기 전에 서버 폴더가 남는다는 것과 설정을 되돌리는지를 알리고, 끝나면 폴더 자리를 알린다
    #[test]
    fn removing_an_imported_server_says_the_folder_stays_and_what_happens_to_its_settings() {
        crate::i18n::init("ko");
        let rt = json!({ "linked_folder": "D:\\pal", "restorable_settings": true });
        let q = remove_question("pal", &rt, false, false);
        assert!(q.contains("D:\\pal") && q.contains("--keep-settings"), "{q}");
        assert!(!remove_question("pal", &rt, false, true).contains("--keep-settings"));
        assert_eq!(remove_question("mc", &json!({}), false, false), t!("remove.confirm", name = "mc"));

        let (done, failed) = removed_message("pal", &json!({ "success": true, "linked_folder": "D:\\pal", "settings_restore_failed": 2 }));
        assert!(done.contains("D:\\pal"));
        assert!(failed.is_some_and(|f| f.contains('2')));
        assert_eq!(removed_message("mc", &json!({ "success": true })), (t!("remove.done", name = "mc"), None));

        // 옮겨 온 서버도 밖에 남는 설정은 되돌린다고 알린다
        let moved = json!({ "restorable_settings": true });
        assert!(remove_question("mv", &moved, false, false).contains("--keep-settings"));
        assert!(removed_message("mv", &json!({ "success": true, "settings_restore_failed": 1 })).1.is_some());
    }

    /// 끝난 설치 카드를 GUI가 다음 폴링 전에 닫으면 항목은 사라져도 서버는 설치 중이 아닌 채로 남는다 (L65)
    #[test]
    fn an_install_closed_elsewhere_after_it_finished_counts_as_finished() {
        let list = json!({ "servers": [
            { "name": "mc", "status": "stopped" },
            { "name": "pal", "status": "provisioning" },
        ] });
        assert!(closed_after_finish(&list, "mc"));
        // 아직 설치 중이거나, 이름이 바뀌었거나 지워졌으면(실패한 새 설치는 닫을 때 지워진다) 알 수 없다
        assert!(!closed_after_finish(&list, "pal"));
        assert!(!closed_after_finish(&list, "gone"));
    }

    struct Typing;

    impl crate::prompt::Asker for Typing {
        fn confirm(&self, _q: &str, _d: bool) -> Option<bool> {
            Some(true)
        }
        fn select(&self, _q: &str, _items: &[String], _d: usize) -> Option<usize> {
            Some(0)
        }
        fn text(&self, _q: &str, _d: Option<&str>, _e: bool) -> Option<String> {
            Some("shown".into())
        }
        fn secret(&self, _q: &str) -> Option<String> {
            Some("hidden".into())
        }
    }

    /// 게임 명령의 비밀번호 칸은 화면에 보이지 않게 묻는다 (L54)
    #[test]
    fn a_password_input_is_asked_without_echo() {
        let cat = Catalog::from_module(
            &json!({ "commands": { "fields": [
                { "name": "login", "inputs": [
                    { "name": "pw", "type": "password", "required": true },
                    { "name": "who", "required": true }
                ] }
            ] } }),
            None,
        );
        let def = cat.find("login").unwrap();
        let _lock = crate::prompt::ASKER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = crate::prompt::AskerGuard::install(Box::new(Typing));
        let ask = crate::prompt::Prompt { enabled: true, yes: false };
        assert_eq!(ask_input(&ask, def, &def.inputs[0]).unwrap(), json!("hidden"));
        assert_eq!(ask_input(&ask, def, &def.inputs[1]).unwrap(), json!("shown"));
    }

    /// 게임 설정 파일에 쓰지 못한 저장(`synced: false`)은 실패로 알린다 (L57)
    #[test]
    fn an_unsynced_save_is_a_failure() {
        let why = sync_failure(&json!({ "success": true, "synced": false, "sync_error": "locked" })).unwrap();
        assert!(why.contains("locked"), "{why}");
        assert!(sync_failure(&json!({ "success": true, "synced": true })).is_none());
        assert!(sync_failure(&json!({ "success": true })).is_none());
    }

    /// 데몬이 비우지 않는 칸에 빈 값을 주면 성공으로 알리지 않고 거절한다 (L33)
    #[test]
    fn an_empty_value_is_refused_where_the_daemon_keeps_the_old_one() {
        let inst = json!({ "module_settings": {} });
        assert!(typed_setting(&[], &inst, "working_dir", "").is_err());
        assert!(typed_setting(&[], &inst, "port", " ").is_err());
        assert_eq!(typed_setting(&[], &inst, "memory_limit_mb", "").unwrap().1, Value::Null);
        assert_eq!(typed_setting(&[], &inst, "working_dir", "C:/srv").unwrap().1, json!("C:/srv"));
    }

    /// `server settings`가 선택형 설정의 선택지를 함께 보인다 (C4)
    #[test]
    fn choices_are_shown_for_a_select_setting() {
        let select = json!({ "name": "difficulty", "field_type": "select", "options": ["easy", "normal", "hard"] });
        assert_eq!(options_line(&select).as_deref(), Some("easy | normal | hard"));
        assert_eq!(options_line(&json!({ "name": "max_players", "field_type": "number" })), None);
        assert_eq!(options_line(&json!({ "name": "x", "options": [] })), None);
    }

    /// `add --set`이 저장은 했지만 게임 설정 파일에 못 썼으면 "저장하지 못했다"고 하지 않는다 (L63)
    #[test]
    fn an_add_whose_settings_were_saved_but_not_synced_says_so() {
        let msg = add_settings_failure("srv", &json!({ "success": true, "synced": false, "sync_error": "locked" })).unwrap();
        assert!(msg.contains("srv") && msg.contains("locked"), "{msg}");
        assert_ne!(msg, t!("add.created_settings_failed", name = "srv", error = "locked"));
        assert!(!msg.contains(&t!("set.sync_failed", error = "locked")), "the sync reason is not nested in another sentence: {msg}");
        assert!(add_settings_failure("srv", &json!({ "success": true, "synced": true })).is_none());
    }

    /// 선택형 설정은 대소문자를 가리지 않고 받되, 데몬에는 모듈이 정한 선택지 그대로 보낸다 (L58)
    #[test]
    fn a_select_setting_is_sent_as_the_option_it_matched() {
        let fields = [json!({ "name": "difficulty", "field_type": "select", "options": ["easy", "Normal", "hard"] })];
        let inst = json!({});
        assert_eq!(typed_setting(&fields, &inst, "difficulty", "NORMAL").unwrap().1, json!("Normal"));
        assert_eq!(typed_setting(&fields, &inst, "difficulty", "Hard").unwrap().1, json!("hard"));
        assert!(typed_setting(&fields, &inst, "difficulty", "peaceful").is_err());
    }

    /// 초기화 확인에 무엇이 지워지는지(모듈 `reset.description`)를 붙인다 (C5)
    #[test]
    fn the_reset_question_says_what_the_module_wipes() {
        let locales = json!({ "reset": { "description": "월드 세이브만 지워요." } });
        assert!(reset_question("a", Some(&locales)).ends_with("월드 세이브만 지워요."));
        assert!(!reset_question("a", None).contains('\n'));
        assert_eq!(reset_question("a", Some(&json!({}))), reset_question("a", None));
    }

    /// `--accept`에 이 서버 약관에 없는 id(오타)를 주면 알린다
    #[test]
    fn accept_ids_that_are_not_agreements_of_the_server_are_reported() {
        let inst = json!({ "agreements": [{ "id": "eula", "accepted": false }, { "id": "privacy", "accepted": true }] });
        let named = vec!["eula".to_string(), "eulla".to_string(), "privacy".to_string()];
        assert_eq!(unknown_accepts(&inst, &named), vec!["eulla"]);
        assert!(unknown_accepts(&json!({}), &[]).is_empty());
    }

    #[test]
    fn typed_settings_follow_schema() {
        let fields = vec![
            json!({ "name": "max_players", "field_type": "number" }),
            json!({ "name": "difficulty", "field_type": "select", "options": ["easy", "hard"] }),
            json!({ "name": "admin_password", "field_type": "password" }),
        ];
        let inst = json!({ "module_settings": { "auto_restart": true } });
        assert_eq!(typed_setting(&fields, &inst, "max_players", "20").unwrap().1, json!(20));
        assert_eq!(
            typed_setting(&fields, &inst, "admin_password", "0042").unwrap().1,
            json!("0042")
        );
        assert_eq!(typed_setting(&fields, &inst, "port", "25565").unwrap().1, json!(25565));
        assert_eq!(
            typed_setting(&fields, &inst, "auto_restart", "off").unwrap().1,
            json!(false)
        );
        assert!(typed_setting(&fields, &inst, "difficulty", "insane").is_err());
        assert_eq!(typed_setting(&fields, &inst, "memory_limit_mb", "6144").unwrap().1, json!(6144));
        assert_eq!(typed_setting(&fields, &inst, "memory_limit_mb", "").unwrap().1, Value::Null);
        assert_eq!(typed_setting(&fields, &inst, "auto_update", "on").unwrap().1, json!(true));
        let (k, v) = typed_setting(&fields, &inst, "ext.box", "").unwrap();
        assert_eq!((k.as_str(), v), ("extension_data.box", Value::Null));
    }

    /// 여러 개 고르는 `list`는 항목마다 선택지에 있어야 하고, 대소문자만 다른 항목은 선택지대로 보낸다
    #[test]
    fn each_item_of_a_list_setting_is_one_of_its_options() {
        let fields = vec![json!({ "name": "maps", "field_type": "list", "options": ["Island", "Desert"] })];
        let inst = json!({});
        assert_eq!(
            typed_setting(&fields, &inst, "maps", "island, Desert").unwrap().1,
            json!(["Island", "Desert"])
        );
        assert!(typed_setting(&fields, &inst, "maps", "Island, Moon").is_err());
        assert_eq!(typed_setting(&fields, &inst, "maps", "").unwrap().1, json!([]));
    }

    /// 목록과 여러 줄 값은 한 줄로 보이고, 켜져 있을 때 다시 켜야 하는 설정에는 표시가 붙는다
    #[test]
    fn list_and_multiline_values_are_shown_on_one_line() {
        crate::i18n::init("ko");
        assert_eq!(setting_text("admins", &json!(["a", "b"])), "a, b");
        assert_eq!(setting_text("motd", &json!("hi\nthere")), "hi ⏎ there");
        assert!(setting_marks(&json!({ "restart_required": true })).contains(&t!("set.restart_required_mark")));
        assert!(setting_marks(&json!({})).is_empty());
    }

    /// 저장 응답의 `restart_required`(켜져 있을 때 바꾼, 다시 켜야 적용되는 설정)를 그대로 알린다
    #[test]
    fn settings_that_need_a_restart_are_named() {
        assert_eq!(
            restart_required(&json!({ "restart_required": ["admins", "motd"] })),
            vec!["admins", "motd"]
        );
        assert!(restart_required(&json!({ "restart_required": [] })).is_empty());
        assert!(restart_required(&json!({})).is_empty());
    }

    /// 새 서버(아직 값이 없음)에서도 데몬이 읽는 설정(auto_restart 등)과 공통 칸을 받는다 (C1)
    #[test]
    fn daemon_settings_are_accepted_on_a_new_server() {
        let fields = vec![json!({ "name": "max_players", "field_type": "number" })];
        let fresh = json!({ "module_settings": {} });
        assert_eq!(typed_setting(&fields, &fresh, "auto_restart", "off").unwrap().1, json!(false));
        assert_eq!(typed_setting(&fields, &fresh, "restart_on_unknown_exit", "on").unwrap().1, json!(true));
        assert_eq!(typed_setting(&fields, &fresh, "managed_start", "false").unwrap().1, json!(false));
        assert_eq!(typed_setting(&fields, &fresh, "log_sessions_keep", "5").unwrap().1, json!(5));
        assert_eq!(typed_setting(&fields, &fresh, "extra_args", "-nogui -x").unwrap().1, json!("-nogui -x"));
        assert_eq!(typed_setting(&fields, &fresh, "working_dir", "D:/srv").unwrap().1, json!("D:/srv"));
    }

    /// 요청 줄(`POST /api/instances/check`)과 본문으로 답하는 가짜 데몬. 받은 요청 줄을 모은다.
    /// 토큰 파일이 있으면 CLI가 신원을 확인하므로 `/health`는 그 토큰으로 증명한다
    async fn fake_daemon(answer: fn(&str, &Value) -> (u16, Value)) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let (mut req, mut buf) = (Vec::new(), [0; 4096]);
                let (head, body) = loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break (String::new(), Value::Null);
                    }
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                            .unwrap_or(0);
                        if body.len() >= len {
                            break (head.to_string(), serde_json::from_str(body).unwrap_or(Value::Null));
                        }
                    }
                };
                let line = head.lines().next().unwrap_or("").rsplit_once(' ').map(|(l, _)| l.to_string()).unwrap_or_default();
                let (status, reply) = if line.starts_with("GET /health") {
                    use hmac::{Hmac, KeyInit, Mac};
                    let challenge = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("x-saba-challenge:").map(|v| v.trim().to_string()));
                    let token = std::fs::read_to_string(saba_chan_updater_lib::constants::token_file_path()).unwrap_or_default();
                    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(token.trim().as_bytes()).unwrap();
                    mac.update(challenge.unwrap_or_default().as_bytes());
                    (200, json!({ "ok": true, "proof": hex::encode(mac.finalize().into_bytes()) }))
                } else {
                    answer(&line, &body)
                };
                log.lock().unwrap().push(line);
                let text = reply.to_string();
                let _ = sock
                    .write_all(format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", text.len(), text).as_bytes())
                    .await;
            }
        });
        (port, seen)
    }

    fn quiet_ctx(port: u16) -> Ctx {
        Ctx {
            api: crate::client::Api::new(port, None),
            spawn_port: port,
            port_fixed: true,
            out: crate::output::Out::new(true, false),
            ask: crate::prompt::Prompt { enabled: false, yes: false },
            in_shell: false,
            attached: false,
        }
    }

    /// 마인크래프트 모듈 하나와 서버 둘(mc, 폴더 /srv/lobby를 쓰는 lobby)이 있는 데몬. 만들기 전 확인은 데몬이 판단한다
    fn add_daemon(line: &str, body: &Value) -> (u16, Value) {
        let reply = match line {
            "GET /api/modules" => json!({ "modules": [{ "name": "minecraft", "display_name": "Minecraft", "install": null }] }),
            "GET /api/instances" => json!([
                { "id": "1", "name": "mc", "module_name": "minecraft" },
                { "id": "2", "name": "lobby", "module_name": "minecraft", "working_dir": "/srv/lobby" },
            ]),
            "POST /api/modules/detect" => json!({ "matches": [{ "name": "minecraft" }] }),
            "POST /api/instances/check" => {
                let name = s(body, "name");
                let lobby = body.get("migration_source").and_then(Value::as_str).is_some_and(|d| d.ends_with("lobby"));
                // 옮기기로 가져올 폴더가 겹치면 확인도 만들기처럼 409로 답한다
                if lobby && s(body, "migration_mode") == "move" {
                    return (409, json!({ "success": false, "error": "Server 'lobby' already uses this folder", "error_code": "working_dir_in_use", "instance": "lobby" }));
                }
                json!({
                    "success": true,
                    "name": { "value": name, "taken": name == "mc", "suggested": "minecraft" },
                    "folder": { "custom": false, "path": null, "in_use_by": if lobby { json!("lobby") } else { Value::Null }, "not_empty": false },
                    "ports": [],
                })
            }
            _ => json!({}),
        };
        (200, reply)
    }

    /// 이름이 겹치면 다른 것을 묻거나 만들기 전에 막는다 (아무것도 만들지 않는다)
    #[tokio::test]
    async fn adding_a_server_with_a_taken_name_stops_before_creating() {
        crate::i18n::init("en");
        let (port, seen) = fake_daemon(add_daemon).await;
        let a = AddArgs { name: Some("mc".into()), module: Some("minecraft".into()), ..Default::default() };
        let e = add(&quiet_ctx(port), a).await.unwrap_err();
        assert_eq!(e.to_string(), t!("add.name_taken", name = "mc"));
        assert!(!seen.lock().unwrap().iter().any(|l| l == "POST /api/instances"), "{:?}", seen.lock().unwrap());
    }

    /// 대소문자만 다른 이름도 겹침이다 (데몬은 정확히 같은 이름만 막는다)
    #[tokio::test]
    async fn a_name_that_differs_only_in_case_is_taken() {
        crate::i18n::init("en");
        let (port, seen) = fake_daemon(add_daemon).await;
        let a = AddArgs { name: Some("MC".into()), module: Some("minecraft".into()), ..Default::default() };
        let e = add(&quiet_ctx(port), a).await.unwrap_err();
        assert_eq!(e.to_string(), t!("add.name_taken", name = "MC"));
        assert!(!seen.lock().unwrap().iter().any(|l| l == "POST /api/instances"), "{:?}", seen.lock().unwrap());
    }

    /// 다른 서버가 쓰는 폴더는 가져오지 않는다 (두 서버가 같은 파일을 덮어쓴다)
    #[tokio::test]
    async fn importing_a_folder_another_server_uses_stops_before_creating() {
        crate::i18n::init("en");
        let (port, seen) = fake_daemon(add_daemon).await;
        let ctx = quiet_ctx(port);
        // 데몬에는 전체 경로로 보낸다 (Windows에서는 드라이브가 붙는다)
        let dir = crate::cmd::config::daemon_path(&ctx, std::path::Path::new("/srv/lobby")).unwrap();
        let a = AddArgs { name: Some("survival".into()), module: Some("minecraft".into()), from: Some("/srv/lobby".into()), ..Default::default() };
        let e = add(&ctx, a.clone()).await.unwrap_err();
        assert_eq!(e.to_string(), t!("add.folder_in_use", dir = dir, name = "lobby"));
        let e = add(&ctx, AddArgs { move_folder: true, ..a }).await.unwrap_err();
        assert_eq!(e.to_string(), t!("add.folder_in_use", dir = dir, name = "lobby"));
        assert!(!seen.lock().unwrap().iter().any(|l| l == "POST /api/instances"), "{:?}", seen.lock().unwrap());
    }

    /// 만들기 전 확인(`/api/instances/check`)은 GUI 마법사와 같다: 이름·폴더는 막고, 포트는 알리기만 한다
    #[test]
    fn the_check_before_adding_blocks_like_the_gui_wizard() {
        crate::i18n::init("en");
        let clash = json!({
            "name": { "value": "mc", "taken": true, "suggested": "mc-2" },
            "folder": { "custom": true, "path": "D:/srv", "in_use_by": "lobby", "not_empty": true },
            "ports": [
                { "field": "port", "port": 25565, "in_use_by": ["lobby"], "reserved": false },
                { "field": "rcon_port", "port": 50000, "in_use_by": [], "reserved": true },
                { "field": "query_port", "port": 25570, "in_use_by": [], "reserved": false },
            ],
        });
        let (blockers, warnings) = check_problems(&clash, None);
        assert_eq!(blockers, vec![
            t!("add.name_taken", name = "mc"),
            t!("add.folder_in_use", dir = "D:/srv", name = "lobby"),
            t!("add.folder_not_empty", dir = "D:/srv"),
        ]);
        assert_eq!(warnings, vec![t!("add.port_in_use", port = 25565, servers = "lobby"), t!("add.port_reserved", port = 50000)]);
        let clear = json!({ "name": { "taken": false }, "folder": { "custom": false }, "ports": [] });
        assert_eq!(check_problems(&clear, None), (vec![], vec![]));
        // 쉼표가 든 이름 (디스코드에서 부르는 말을 쉼표로 나눈다, 사용자 11-06 #2)
        let comma = json!({ "name": { "value": "a,b", "taken": false, "invalid": "comma" }, "folder": { "custom": false }, "ports": [] });
        assert_eq!(check_problems(&comma, None).0, vec![t!("add.name_comma")]);
        assert!(has_comma("우리，마크") && has_comma("a、b") && !has_comma("우리 마크"));
    }

    /// 고를 수 있는 버전은 GUI처럼 정식 릴리스만 (스냅숏·베타는 빼고, 종류가 없는 항목은 둔다)
    #[test]
    fn only_releases_are_offered_as_versions() {
        let list = json!({ "versions": [{ "id": "1.21.4", "type": "release" }, { "id": "24w10a", "type": "snapshot" }, { "id": "1.20.6" }, "1.19"] });
        assert_eq!(release_versions(&list), vec!["1.21.4", "1.20.6", "1.19"]);
        assert!(release_versions(&json!({})).is_empty());
    }

    /// 만들 때 고르는 칸은 GUI와 같다: 설치할 수 있는 값(`install_options`)만, 이름은 모듈 locales로
    #[test]
    fn install_choices_and_labels_follow_the_gui() {
        let field = json!({ "name": "server_type", "options": ["vanilla", "paper", "spigot"], "install_options": ["vanilla", "paper"] });
        assert_eq!(install_choices(&field), vec!["vanilla", "paper"]);
        assert_eq!(install_choices(&json!({ "options": ["a"] })), vec!["a"]);
        let locales = json!({ "settings": { "server_type": { "label": "서버 종류", "options": { "paper": "페이퍼" } } } });
        assert_eq!(option_label(Some(&locales), &field, "paper"), "페이퍼");
        assert_eq!(option_label(Some(&locales), &field, "vanilla"), "vanilla");
        assert_eq!(setting_label(Some(&locales), &field), "서버 종류");
        assert_eq!(setting_label(None, &json!({ "name": "seed", "label": "Seed" })), "Seed");
    }

    /// 모듈 약관은 GUI처럼 만들기 전에 묻는다: 모듈 목록의 약관을 locales 이름으로 펼친다
    /// 이 데몬에서 돌지 않는 게임은 고른 직후 멈춘다 (익스텐션을 받기 전에)
    #[test]
    fn a_module_the_daemon_cannot_run_stops_right_after_picking_it() {
        crate::i18n::init("en");
        assert!(platform_refusal(&json!({ "platform_supported": true })).is_none());
        assert!(platform_refusal(&json!({})).is_none());
        let os = platform_refusal(&json!({ "platform_supported": false, "unsupported_reason": "os" })).unwrap();
        assert_eq!(os, crate::i18n::t("module_errors.module_platform_unsupported"));
        // CPU 문구가 없는 판에서도 OS 문구로 멈춘다
        let arch = platform_refusal(&json!({ "platform_supported": false, "unsupported_reason": "arch" })).unwrap();
        assert!(!arch.is_empty() && !arch.contains("module_errors."));
    }

    #[test]
    fn module_agreements_are_listed_before_creating() {
        let module = json!({ "agreements": [{ "id": "eula", "url": "https://aka.ms/MinecraftEULA" }] });
        let locales = json!({ "agreements": { "eula": "마인크래프트 EULA" } });
        let list = module_agreements(&module, Some(&locales));
        assert_eq!(list, vec![json!({ "id": "eula", "label": "마인크래프트 EULA", "url": "https://aka.ms/MinecraftEULA" })]);
        assert_eq!(s(&module_agreements(&module, None)[0], "label"), "eula");
        assert!(module_agreements(&json!({}), None).is_empty());
    }

    /// 만들 때·`server agree`가 묻는 약관은 아직 동의하지 않은 것만 (C2)
    #[test]
    fn only_unaccepted_agreements_are_asked() {
        let inst = json!({ "agreements": [
            { "id": "eula", "label": "Minecraft EULA", "url": "https://aka.ms/MinecraftEULA", "accepted": false },
            { "id": "tos", "label": "ToS", "url": "https://example.com", "accepted": true },
        ] });
        let ids: Vec<String> = pending_agreements(&inst).iter().map(|a| s(a, "id").to_string()).collect();
        assert_eq!(ids, vec!["eula"]);
        assert!(pending_agreements(&json!({})).is_empty());
    }

    /// 약관은 `--accept`로 이름을 댄 것만 바로 동의하고, 나머지는 사람에게 물을 수 있을 때만 묻는다.
    /// `--yes`나 `--json`은 동의가 아니다 (리뷰 315)
    #[test]
    fn agreements_are_accepted_only_by_name_or_asked() {
        let pending = vec![json!({ "id": "eula", "label": "EULA" }), json!({ "id": "tos", "label": "ToS" })];
        let ids = |v: &[Value]| v.iter().map(|a| s(a, "id").to_string()).collect::<Vec<_>>();
        let (named, ask) = split_agreements(&pending, &["eula".to_string()], true);
        assert_eq!((ids(&named), ids(&ask)), (vec!["eula".to_string()], vec!["tos".to_string()]));
        let (named, ask) = split_agreements(&pending, &[], false);
        assert!(named.is_empty() && ask.is_empty(), "cannot ask: nothing is accepted, only guidance");
        let (named, ask) = split_agreements(&pending, &[], true);
        assert_eq!((ids(&named).len(), ids(&ask)), (0, vec!["eula".to_string(), "tos".to_string()]));
    }

    #[test]
    fn patch_nests_extension_data() {
        let mut p = Map::new();
        insert_patch(&mut p, "extension_data.a", json!(1));
        insert_patch(&mut p, "port", json!(2));
        assert_eq!(Value::Object(p), json!({ "extension_data": { "a": 1 }, "port": 2 }));
    }

    /// 시작 전에 예약 범위를 피해 옮긴 포트를 알린다 (M13. 성공 답과 실패 답 모두)
    #[test]
    fn a_moved_port_is_told_on_start() {
        let body = json!({ "success": false, "port_moved": [{ "field": "rest_port", "from": 8212, "to": 8242 }] });
        let lines = port_moved_lines(&body);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("8212") && lines[0].contains("8242"), "{}", lines[0]);
        assert!(port_moved_lines(&json!({ "success": true })).is_empty());
    }

    /// 켠 서버의 모듈에 새 버전이 있으면 시작과 함께 알린다 (데몬 `module_update`)
    #[test]
    fn a_newer_module_is_told_on_start() {
        let body = json!({ "success": true, "module_update": { "module": "palworld", "current": "1.0.0", "latest": "1.1.0" } });
        let line = module_update_line(&body).unwrap();
        assert!(line.contains("palworld") && line.contains("1.1.0"), "{line}");
        assert!(module_update_line(&json!({ "success": true })).is_none());
    }

    /// 약관 동의는 사람이 직접 답한다: `server agree`도 `--yes`로 동의하지 않는다 (확인 `ensure`가 아니라 동의 `agreed`)
    #[test]
    fn server_agree_asks_for_a_consent_not_a_confirmation() {
        // Windows 체크아웃은 줄 끝이 CRLF일 수 있다
        let src = include_str!("server/checkup.rs").replace("\r\n", "\n");
        let body = &src[src.find("async fn agree(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("ctx.ask.agreed(") && !body.contains("ensure("), "{body}");
    }

    /// 익스텐션이 묻는 예/아니오 칸은 `--yes`여도 켜지 않는다: 기본값을 쓴다 (`--yes`는 확인을 넘길 뿐 값을 정하지 않는다)
    #[test]
    fn yes_keeps_the_default_of_a_yes_no_option() {
        let ask = crate::prompt::Prompt { enabled: true, yes: true };
        assert_eq!(bool_option(&ask, "q", &json!(false)).unwrap(), json!(false));
        assert_eq!(bool_option(&ask, "q", &json!(true)).unwrap(), json!(true));
        let quiet = crate::prompt::Prompt { enabled: false, yes: false };
        assert_eq!(bool_option(&quiet, "q", &Value::Null).unwrap(), Value::Null);
    }
}
