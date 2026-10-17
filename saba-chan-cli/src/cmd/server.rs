//! `server …` 와 바로가기 `start/stop/restart/run`

use crate::catalog::{Catalog, Command, Input, Risk};
use crate::cli::{AddArgs, PropsCmd, RestartArgs, RunArgs, ServerCmd, StartArgs, StopArgs};
use crate::cmd::{split_pairs, typed};
use crate::ctx::Ctx;
use crate::error::{failed, usage, CliError, Result};
use crate::output::{self, s, show};
use crate::resolve::{self, Server};
use crate::slots::Slots;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

pub async fn run(ctx: &Ctx, cmd: ServerCmd) -> Result<()> {
    match cmd {
        ServerCmd::List => list(ctx).await,
        ServerCmd::Info { server } => info(ctx, server.as_deref()).await,
        ServerCmd::Add(a) => add(ctx, a).await,
        ServerCmd::Remove { server } => remove(ctx, server.as_deref()).await,
        ServerCmd::Start(a) => start(ctx, a).await,
        ServerCmd::Stop(a) => stop(ctx, a).await,
        ServerCmd::Restart(a) => restart(ctx, a).await,
        ServerCmd::Rename { server, name } => rename(ctx, Some(&server), &name).await,
        ServerCmd::Console(a) => crate::cmd::console::run(ctx, a).await,
        ServerCmd::Run(a) => run_command(ctx, a).await,
        ServerCmd::Commands { server } => commands(ctx, server.as_deref()).await,
        ServerCmd::Send { server, line } => send(ctx, &server, &line.join(" ")).await,
        ServerCmd::Settings { server } => settings(ctx, server.as_deref()).await,
        ServerCmd::Set { server, pairs } => set(ctx, &server, &pairs).await,
        ServerCmd::Properties { server, action } => properties(ctx, server.as_deref(), action).await,
        ServerCmd::Rcon { server, line } => rcon(ctx, &server, &line.join(" ")).await,
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
        ServerCmd::AcceptEula { server } => accept_eula(ctx, server.as_deref()).await,
        ServerCmd::Reset { server } => reset(ctx, server.as_deref()).await,
        ServerCmd::Reorder { servers } => reorder(ctx, &servers).await,
    }
}

// ── 공용 도우미 ──

/// `/api/servers`에서 이 서버의 실행 상태
async fn runtime(ctx: &Ctx, sv: &Server) -> Result<Value> {
    let list = ctx.api.servers().await?;
    Ok(list
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|x| s(x, "id") == sv.id).cloned())
        .unwrap_or(Value::Null))
}

fn status_of(rt: &Value) -> &str {
    let st = s(rt, "status");
    if st.is_empty() {
        "stopped"
    } else {
        st
    }
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
        cat.add_extension_commands(list);
    }
    Ok(cat)
}

async fn slots(ctx: &Ctx) -> Slots {
    resolve::extensions(ctx)
        .await
        .map(|l| Slots::from_extensions(&l))
        .unwrap_or_default()
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

// ── 목록·정보 ──

async fn list(ctx: &Ctx) -> Result<()> {
    let (servers, modules, slots) = tokio::join!(ctx.api.servers(), ctx.api.modules(), slots(ctx));
    let servers = servers?;
    let modules = modules.unwrap_or(Value::Null);
    ctx.out.data(servers.get("servers").unwrap_or(&json!([])), |list| {
        crate::cmd::status::print_server_table(list.as_array().map(Vec::as_slice).unwrap_or(&[]), &modules, &slots);
    });
    Ok(())
}

async fn info(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let (rt, module, events, slots, progress) = tokio::join!(
        runtime(ctx, &sv),
        module_info(ctx, &sv.module),
        ctx.api.exit_events(&sv.id),
        slots(ctx),
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

    let v = json!({ "instance": sv.raw, "runtime": rt, "last_exit": last_exit, "provisioning": progress });
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
        if let Some(st) = rt
            .get("start_time")
            .and_then(Value::as_u64)
            .filter(|_| status_of(&rt) == "running")
        {
            rows.push((t!("col.started"), output::ago(st)));
        }
        if let Some(pid) = rt.get("pid").and_then(Value::as_u64) {
            rows.push(("PID".into(), pid.to_string()));
        }
        for (key, label) in [
            ("port", "col.port"),
            ("rcon_port", "col.rcon_port"),
            ("rest_port", "col.rest_port"),
        ] {
            if let Some(p) = sv.raw.get(key).filter(|p| !p.is_null()) {
                rows.push((t!(label), show(p)));
            }
        }
        if let Some(v) = sv
            .raw
            .get("server_version")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            rows.push((t!("col.version"), v.to_string()));
        }
        if let Some(dir) = sv.raw.get("working_dir").and_then(Value::as_str) {
            rows.push((t!("col.folder"), dir.to_string()));
        }
        rows.extend(slots.detail_rows(&rt));
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
        output::print_kv(&rows);
        if let Some(p) = &progress {
            println!();
            println!("{}", progress_line(p));
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
        None => format!("{} {}{}", console::style("◐").yellow(), pct, s(p, "message")),
    }
}

// ── 추가·삭제 ──

async fn add(ctx: &Ctx, a: AddArgs) -> Result<()> {
    // 1) 게임
    let module = match a.module.as_deref() {
        Some(q) => resolve::module(ctx, Some(q)).await?,
        None => {
            let all = resolve::modules(ctx).await?;
            if all.is_empty() {
                return Err(failed(t!("add.no_modules")));
            }
            if !ctx.ask.enabled {
                return Err(usage(t!("add.module_required")));
            }
            resolve::choose(ctx, &t!("noun.game"), all, resolve::module_label)?
        }
    };
    let module_name = s(&module, "name").to_string();
    let display = if s(&module, "display_name").is_empty() {
        module_name.clone()
    } else {
        s(&module, "display_name").to_string()
    };

    // 2) 이름
    let existing: Vec<String> = resolve::servers(ctx)
        .await?
        .into_iter()
        .map(|x| x.name.to_lowercase())
        .collect();
    let taken = |n: &str| existing.contains(&n.trim().to_lowercase());
    let suggested = (1..)
        .map(|i| {
            if i == 1 {
                module_name.clone()
            } else {
                format!("{}-{}", module_name, i)
            }
        })
        .find(|n| !taken(n))
        .unwrap();
    let name = match a.name.clone() {
        Some(n) => n,
        None => match ctx.ask.text(&t!("add.ask_name"), Some(&suggested), false)? {
            Some(n) => n,
            None => suggested.clone(),
        },
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(usage(t!("add.name_empty")));
    }
    if taken(&name) {
        return Err(usage(t!("add.name_taken", name = name)));
    }

    // 3) 새로 설치 / 기존 폴더
    let mut from = a.from.clone();
    let installs = module.get("install").is_some_and(|i| !i.is_null());
    if from.is_none() && ctx.ask.enabled && a.name.is_none() {
        let choices = [t!("add.fresh_install"), t!("add.use_existing")];
        if ctx
            .ask
            .select(&t!("add.ask_source"), &choices, if installs { 0 } else { 1 })?
            == Some(1)
        {
            from = ctx.ask.text(&t!("add.ask_folder"), None, false)?;
        }
    }
    if let Some(dir) = from.as_deref() {
        check_folder(ctx, dir, &module_name).await?;
    } else if !installs {
        ctx.out.warn(&t!("add.no_installer", game = display));
    }

    // 4) 필요한 익스텐션
    ensure_extensions(ctx, &module).await?;

    // 5) 익스텐션이 묻는 칸 (CreateInstance.options)
    let mut ext_data = Map::new();
    for opt in slots(ctx).await.create_options() {
        let d = &opt.data;
        let Some(field) = d.get("field").and_then(Value::as_str) else {
            continue;
        };
        let label = d.get("label").and_then(Value::as_str).unwrap_or(field);
        let default = d.get("default").cloned().unwrap_or(Value::Null);
        let value = if s(d, "type") == "boolean" {
            if ctx.ask.enabled {
                Value::Bool(ctx.ask.confirm(label, default.as_bool().unwrap_or(false))?)
            } else {
                default
            }
        } else {
            match ctx.ask.text(label, default.as_str(), true)? {
                Some(v) if !v.is_empty() => Value::String(v),
                _ => default,
            }
        };
        if !value.is_null() {
            ext_data.insert(field.to_string(), value);
        }
    }

    // 6) 만들기
    let mut body = json!({ "name": name, "module_name": module_name });
    if let Some(p) = a.game_port {
        body["port"] = json!(p);
    }
    if let Some(dir) = &from {
        body["migration_source"] = json!(dir);
    }
    let created = ctx.api.create_instance(body).await?;
    let id = s(&created, "id").to_string();

    // 7) 설정 (--set, 익스텐션 칸)
    let mut patch = Map::new();
    if !a.set.is_empty() {
        let fields = setting_fields(&module);
        for (k, v) in split_pairs(&a.set)? {
            let (key, value) = typed_setting(&fields, &Value::Null, &k, &v)?;
            insert_patch(&mut patch, &key, value);
        }
    }
    if !ext_data.is_empty() {
        patch
            .entry("extension_data")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .extend(ext_data);
    }
    if !patch.is_empty() {
        if let Err(e) = ctx.api.update_instance(&id, Value::Object(patch)).await {
            ctx.out.warn(&t!("add.settings_failed", error = e));
        }
    }

    // 8) 설치 진행
    let provisioning = created.get("provisioning").and_then(Value::as_bool) == Some(true);
    if provisioning {
        if a.no_wait {
            ctx.out.done(&t!("add.created_installing", name = name), &created);
            ctx.out.hint(&format!("saba-chan-cli server progress {}", name));
            return Ok(());
        }
        let result = follow_provision(ctx, &name).await?;
        if let Some(err) = result.get("error").and_then(Value::as_str) {
            let rolled = ctx.api.dismiss_provision(&name).await.ok();
            let removed = rolled.and_then(|r| r.get("rolled_back").and_then(Value::as_bool)) == Some(true);
            return Err(failed(if removed {
                t!("add.install_failed_removed", error = err)
            } else {
                t!("add.install_failed", error = err)
            }));
        }
    }
    let out = json!({ "success": true, "id": id, "name": name, "module": module_name, "provisioned": provisioning });
    ctx.out.done(&t!("add.ready", name = name, game = display), &out);
    if a.start {
        return start(
            ctx,
            StartArgs {
                server: Some(id),
                ..Default::default()
            },
        )
        .await;
    }
    ctx.out.hint(&format!("saba-chan-cli start {}", shell_word(&name)));
    Ok(())
}

/// 이름에 공백이 있으면 따옴표
fn shell_word(s: &str) -> String {
    if s.contains(char::is_whitespace) {
        format!("\"{}\"", s)
    } else {
        s.to_string()
    }
}

/// 기존 폴더가 이 게임 서버인지 데몬에게 물어본다
async fn check_folder(ctx: &Ctx, dir: &str, module: &str) -> Result<()> {
    let r = ctx.api.detect(dir).await?;
    let matches: Vec<String> = r
        .get("matches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| s(m, "name").to_string())
        .collect();
    if matches.iter().any(|m| m == module) {
        return Ok(());
    }
    let msg = if matches.is_empty() {
        t!("add.folder_unrecognized", dir = dir)
    } else {
        t!("add.folder_other_game", dir = dir, games = matches.join(", "))
    };
    ctx.out.warn(&msg);
    ctx.ask.ensure(&t!("add.use_anyway"))
}

/// 모듈이 요구하는 익스텐션이 꺼져 있거나 없으면 켜고 설치한다 (물어본 뒤)
async fn ensure_extensions(ctx: &Ctx, module: &Value) -> Result<()> {
    let required: Vec<String> = module
        .pointer("/install/requires_extensions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if required.is_empty() {
        return Ok(());
    }
    let exts = resolve::extensions(ctx).await?;
    let state = |id: &str| {
        exts.iter()
            .find(|e| s(e, "id") == id)
            .map(|e| e.get("enabled").and_then(Value::as_bool) == Some(true))
    };
    let missing: Vec<&String> = required.iter().filter(|id| state(id) != Some(true)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let names = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
    if !ctx.ask.confirm(&t!("add.needs_extensions", ids = names), true)? {
        return Err(CliError::Cancelled.into());
    }
    for id in missing {
        if state(id).is_none() {
            let spin = crate::progress::spinner(&t!("ext.installing", id = id));
            let r = ctx.api.install_extension(id, json!({})).await;
            spin.finish_and_clear();
            r?;
        }
        ctx.api.enable_extension(id).await?;
        ctx.out.done(&t!("ext.enabled", id = id), &json!({ "id": id }));
    }
    Ok(())
}

/// 설치 진행률을 끝날 때까지 보여 준다. Ctrl+C는 기다리기만 멈춘다 (설치는 계속).
async fn follow_provision(ctx: &Ctx, name: &str) -> Result<Value> {
    let bar = crate::progress::percent_bar(&t!("add.installing"));
    loop {
        let p = ctx.api.provision_progress(name).await?;
        if p.get("active").and_then(Value::as_bool) != Some(true) {
            bar.finish_and_clear();
            return Ok(json!({ "done": true }));
        }
        if let Some(pct) = p.get("percent").and_then(Value::as_u64) {
            bar.set_position(pct.min(100));
        }
        let msg = s(&p, "message");
        if !msg.is_empty() {
            bar.set_message(msg.to_string());
        }
        if p.get("done").and_then(Value::as_bool) == Some(true) {
            bar.finish_and_clear();
            return Ok(p);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(700)) => {}
            _ = tokio::signal::ctrl_c() => {
                bar.finish_and_clear();
                ctx.out.note(&t!("add.still_installing", name = name));
                ctx.out.hint(&format!("saba-chan-cli server progress {}", shell_word(name)));
                return Err(CliError::Cancelled.into());
            }
        }
    }
}

async fn remove(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let running = status_of(&runtime(ctx, &sv).await?) == "running";
    let question = if running {
        t!("remove.confirm_running", name = sv.name)
    } else {
        t!("remove.confirm", name = sv.name)
    };
    ctx.ask.ensure(&question)?;
    let r = ctx.api.delete_instance(&sv.id).await?;
    ctx.out.done(&t!("remove.done", name = sv.name), &r);
    Ok(())
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
            let r = ctx.api.start(&sv.id).await?;
            if a.no_wait {
                ctx.out.done(&t!("start.requested", name = sv.name), &r);
            } else {
                let st = wait_status(
                    ctx,
                    &sv,
                    &["running"],
                    Duration::from_secs(90),
                    &t!("start.waiting", name = sv.name),
                )
                .await?;
                if st == "running" {
                    ctx.out.done(&t!("start.done", name = sv.name), &r);
                } else {
                    ctx.out.warn(&t!("start.not_yet", name = sv.name, status = st));
                    ctx.out.hint(&format!("saba-chan-cli console {}", shell_word(&sv.name)));
                    ctx.out
                        .hint(&format!("saba-chan-cli server history {}", shell_word(&sv.name)));
                    return Err(CliError::Failed(t!("start.not_yet", name = sv.name, status = st)).into());
                }
            }
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

pub async fn stop(ctx: &Ctx, a: StopArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    stop_server(ctx, &sv, a.force, a.no_wait).await
}

async fn stop_server(ctx: &Ctx, sv: &Server, force: bool, no_wait: bool) -> Result<()> {
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
    let spin = crate::progress::spinner(&t!("restart.waiting", name = sv.name));
    let r = ctx.api.restart(&sv.id, a.force).await;
    spin.finish_and_clear();
    let r = r?;
    let st = wait_status(
        ctx,
        &sv,
        &["running"],
        Duration::from_secs(90),
        &t!("start.waiting", name = sv.name),
    )
    .await?;
    if st == "running" {
        ctx.out.done(&t!("restart.done", name = sv.name), &r);
        Ok(())
    } else {
        Err(failed(t!("start.not_yet", name = sv.name, status = st)))
    }
}

pub async fn rename(ctx: &Ctx, query: Option<&str>, new_name: &str) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(usage(t!("add.name_empty")));
    }
    let r = ctx.api.rename(&sv.id, new_name).await?;
    ctx.out.done(&t!("rename.done", old = sv.name, name = new_name), &r);
    Ok(())
}

// ── 게임 명령 ──

pub async fn run_command(ctx: &Ctx, a: RunArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    let cat = catalog_for_server(ctx, &sv).await?;
    let name = match a.command {
        Some(c) => c,
        None => {
            if cat.commands.is_empty() || !ctx.ask.enabled {
                return print_commands(ctx, &sv.module, &cat);
            }
            let items: Vec<String> = cat
                .commands
                .iter()
                .map(|c| format!("{}  {}", c.label, output::dim(&c.usage())))
                .collect();
            match ctx.ask.select(&t!("run.pick", name = sv.name), &items, 0)? {
                Some(i) => cat.commands[i].name.clone(),
                None => return Ok(()),
            }
        }
    };
    exec(ctx, &sv, &cat, &name, &a.args).await
}

/// 명령 하나 실행 (콘솔 입력에서도 쓴다)
pub async fn exec(ctx: &Ctx, sv: &Server, cat: &Catalog, name: &str, words: &[String]) -> Result<()> {
    let Some(def) = cat.find(name) else {
        if !cat.accepts_raw() {
            let found = resolve::find(&cat.names(), name, |n| vec![n.clone()], |n| n.clone());
            let suggestions = match found {
                resolve::Found::None(s) => s,
                resolve::Found::One(n) => vec![n],
                resolve::Found::Many(v) => v,
            };
            return Err(CliError::NotFound {
                what: t!("noun.command"),
                query: name.into(),
                suggestions,
            }
            .into());
        }
        // 정의에 없는 명령은 한 줄 그대로 (데몬이 콘솔로 보낸다)
        let line = std::iter::once(name.to_string())
            .chain(words.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let r = ctx.api.command(&sv.id, &line, Value::Null).await?;
        print_result(ctx, &r);
        return Ok(());
    };
    let (mut args, missing) = def.bind(words).map_err(usage)?;
    for input in missing {
        let v = ask_input(ctx, def, &input)?;
        args.insert(input.name.clone(), input.coerce(v).map_err(usage)?);
    }
    if def.risk == Risk::Danger {
        ctx.ask
            .ensure(&t!("run.confirm_danger", label = def.label, name = sv.name))?;
    }
    let r = ctx.api.command(&sv.id, &def.name, Value::Object(args)).await?;
    print_result(ctx, &r);
    Ok(())
}

fn ask_input(ctx: &Ctx, def: &Command, input: &Input) -> Result<Value> {
    let missing = || usage(t!("run.missing_arg", label = input.label, usage = def.usage()));
    if !input.options.is_empty() {
        return match ctx.ask.select(&input.label, &input.options, 0)? {
            Some(i) => Ok(Value::String(input.options[i].clone())),
            None => Err(missing()),
        };
    }
    let prompt = match &input.placeholder {
        Some(p) => format!("{} {}", input.label, output::dim(&format!("({})", p))),
        None => input.label.clone(),
    };
    match ctx.ask.text(&prompt, None, false)? {
        Some(v) => Ok(Value::String(v)),
        None => Err(missing()),
    }
}

/// 명령 응답을 사람이 읽게: 문자열 → 그대로, 객체 배열 → 표, 객체 → 이름: 값
pub fn print_result(ctx: &Ctx, r: &Value) {
    ctx.out.data(r, |r| {
        let data = r.get("data").filter(|d| !d.is_null());
        let text = data
            .and_then(|d| d.get("response").and_then(Value::as_str).or_else(|| d.as_str()))
            .or_else(|| r.get("response").and_then(Value::as_str))
            .or_else(|| r.get("message").and_then(Value::as_str))
            .or_else(|| r.get("output").and_then(Value::as_str));
        if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
            println!("{}", t.trim_end());
            return;
        }
        match data {
            Some(d) => print_value(d),
            None => println!("{} {}", console::style("✓").green().bold(), t!("run.sent")),
        }
    });
}

/// 모양을 모르는 JSON을 읽기 좋게
pub fn print_value(v: &Value) {
    match v {
        Value::Array(items) if items.iter().all(Value::is_object) && !items.is_empty() => {
            let mut cols: Vec<String> = Vec::new();
            for it in items {
                for k in it.as_object().unwrap().keys() {
                    if !cols.contains(k) {
                        cols.push(k.clone());
                    }
                }
            }
            let mut t = output::table(&cols);
            for it in items {
                t.add_row(
                    cols.iter()
                        .map(|c| it.get(c).map(show).unwrap_or_default())
                        .collect::<Vec<_>>(),
                );
            }
            output::print_table(t);
        }
        Value::Object(map) => {
            // 객체 안에 목록 하나뿐이면 그 목록을 표로 (예: {"players": [...]})
            if map.len() == 1 {
                if let Some(inner @ Value::Array(_)) = map.values().next() {
                    return print_value(inner);
                }
            }
            let mut flat = Vec::new();
            crate::cmd::flatten(v, "", &mut flat);
            output::print_kv(&flat.into_iter().map(|(k, v)| (k, show(&v))).collect::<Vec<_>>());
        }
        Value::Array(items) => {
            for it in items {
                println!("{}", show(it));
            }
        }
        other => println!("{}", show(other)),
    }
}

async fn commands(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let cat = catalog_for_server(ctx, &sv).await?;
    print_commands(ctx, &sv.module, &cat)
}

pub fn print_commands(ctx: &Ctx, module: &str, cat: &Catalog) -> Result<()> {
    let v: Vec<Value> = cat
        .commands
        .iter()
        .map(|c| {
            json!({
                "name": c.name, "label": c.label, "description": c.description, "risk": c.risk.key(),
                "method": c.method, "aliases": c.aliases, "usage": c.usage(), "extension": c.extension,
                "inputs": c.inputs.iter().map(|i| json!({
                    "name": i.name, "label": i.label, "type": i.kind, "required": i.required,
                    "default": i.default, "options": i.options, "min": i.min, "max": i.max,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let v = json!({ "module": module, "accepts_raw": cat.accepts_raw(), "commands": v });
    ctx.out.data(&v, |_| {
        if cat.commands.is_empty() {
            println!("{}", output::dim(&t!("run.no_commands")));
        } else {
            let mut t = output::table(&[t!("col.command"), t!("col.what"), t!("col.aliases")]);
            for c in &cat.commands {
                let mut usage = c.usage();
                if c.risk == Risk::Danger {
                    usage.push_str(&console::style(" ⚠").red().to_string());
                }
                let mut what = if c.description.is_empty() || c.description == c.label {
                    c.label.clone()
                } else {
                    format!("{} — {}", c.label, c.description)
                };
                if let Some(ext) = &c.extension {
                    what.push_str(&output::dim(&format!("  [{}]", ext)));
                }
                t.add_row(vec![usage, what, output::dim(&c.aliases.join(", "))]);
            }
            output::print_table(t);
        }
        if cat.accepts_raw() {
            println!();
            println!("{}", output::dim(&t!("run.raw_ok")));
        }
    });
    Ok(())
}

async fn send(ctx: &Ctx, query: &str, line: &str) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let r = ctx.api.stdin(&sv.id, line).await?;
    print_result(ctx, &r);
    Ok(())
}

async fn rcon(ctx: &Ctx, query: &str, line: &str) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let r = ctx.api.rcon(&sv.id, line).await?;
    print_result(ctx, &r);
    Ok(())
}

async fn rest(ctx: &Ctx, query: &str, method: &str, endpoint: &str, body: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let body = body
        .map(|b| serde_json::from_str::<Value>(b).map_err(|e| usage(t!("err.bad_json", error = e))))
        .transpose()?;
    let r = ctx.api.rest(&sv.id, &method.to_uppercase(), endpoint, body).await?;
    print_result(ctx, &r);
    Ok(())
}

// ── 설정 ──

/// 모듈 설정 칸 (이름 → 칸 정의)
fn setting_fields(module: &Value) -> Vec<Value> {
    module
        .pointer("/settings/fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// 인스턴스 최상위에 있는 공통 칸과 그 형식 (데몬 `update_instance_settings`의 known_fields)
const COMMON: &[(&str, &str)] = &[
    ("port", "number"),
    ("rcon_port", "number"),
    ("rcon_password", "password"),
    ("rest_host", "text"),
    ("rest_port", "number"),
    ("rest_username", "text"),
    ("rest_password", "password"),
    ("executable_path", "text"),
    ("protocol_mode", "text"),
    ("server_version", "text"),
];

/// KEY=VALUE 하나를 PATCH 키와 형식 맞춘 값으로. `ext.키`는 extension_data, 빈 값은 지우기(null).
fn typed_setting(fields: &[Value], instance: &Value, key: &str, raw: &str) -> Result<(String, Value)> {
    if let Some(ext_key) = key.strip_prefix("ext.") {
        let v = if raw.is_empty() {
            Value::Null
        } else {
            crate::cmd::guess(raw)
        };
        return Ok((format!("extension_data.{}", ext_key), v));
    }
    let kind = fields
        .iter()
        .find(|f| s(f, "name") == key)
        .map(|f| s(f, "field_type").to_string())
        .or_else(|| COMMON.iter().find(|(k, _)| *k == key).map(|(_, t)| t.to_string()))
        .or_else(|| {
            // 정의에 없는 키는 지금 값의 형식을 따른다
            let cur = instance
                .get(key)
                .or_else(|| instance.pointer(&format!("/module_settings/{}", key)))?;
            Some(match cur {
                Value::Bool(_) => "boolean".into(),
                Value::Number(_) => "number".into(),
                _ => "text".into(),
            })
        })
        .unwrap_or_else(|| "text".into());
    let field = fields.iter().find(|f| s(f, "name") == key);
    if let Some(opts) = field.and_then(|f| f.get("options")).and_then(Value::as_array) {
        let opts: Vec<&str> = opts.iter().filter_map(Value::as_str).collect();
        if !opts.is_empty() && !opts.iter().any(|o| o.eq_ignore_ascii_case(raw)) {
            return Err(usage(t!("set.must_be_one_of", key = key, options = opts.join(", "))));
        }
    }
    Ok((key.to_string(), typed(raw, &kind)?))
}

fn insert_patch(patch: &mut Map<String, Value>, key: &str, value: Value) {
    match key.strip_prefix("extension_data.") {
        Some(k) => {
            let ext = patch.entry("extension_data").or_insert_with(|| json!({}));
            ext.as_object_mut().unwrap().insert(k.to_string(), value);
        }
        None => {
            patch.insert(key.to_string(), value);
        }
    }
}

async fn settings(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let (module, locales) = tokio::join!(module_info(ctx, &sv.module), module_locales(ctx, &sv.module));
    let module = module.unwrap_or(Value::Null);
    let fields = setting_fields(&module);
    let label = |f: &Value| {
        locales
            .as_ref()
            .and_then(|l| l.pointer(&format!("/settings/{}/label", s(f, "name"))))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if s(f, "label").is_empty() {
                    s(f, "name").to_string()
                } else {
                    s(f, "label").to_string()
                }
            })
    };
    let ms = sv.raw.get("module_settings").cloned().unwrap_or(json!({}));
    let current = |name: &str| {
        sv.raw
            .get(name)
            .filter(|v| !v.is_null())
            .or_else(|| ms.get(name))
            .cloned()
            .unwrap_or(Value::Null)
    };

    let mut out_fields = Vec::new();
    for f in &fields {
        let name = s(f, "name");
        out_fields.push(json!({
            "key": name, "label": label(f), "type": s(f, "field_type"), "group": f.get("group"),
            "value": current(name), "default": f.get("default"), "options": f.get("options"),
        }));
    }
    let v = json!({ "server": sv.name, "fields": out_fields, "instance": sv.raw });
    ctx.out.data(&v, |_| {
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut push = |group: &str, k: String, v: String| match groups.iter_mut().find(|(g, _)| g == group) {
            Some((_, rows)) => rows.push((k, v)),
            None => groups.push((group.to_string(), vec![(k, v)])),
        };
        let mut seen: Vec<String> = Vec::new();
        for f in &fields {
            let name = s(f, "name");
            let group = f.get("group").and_then(Value::as_str).unwrap_or("basic");
            let val = current(name);
            let mut shown = output::mask(name, &val);
            if val.is_null() {
                if let Some(d) = f.get("default").filter(|d| !d.is_null()) {
                    shown = output::dim(&format!("{} ({})", show(d), t!("set.default")));
                }
            }
            push(
                group,
                format!("{} {}", label(f), output::dim(&format!("[{}]", name))),
                shown,
            );
            seen.push(name.to_string());
        }
        for (k, _) in COMMON {
            if seen.iter().any(|s| s == k) {
                continue;
            }
            if let Some(val) = sv.raw.get(*k).filter(|v| !v.is_null()) {
                push("saba-chan", output::dim(&format!("[{}]", k)), output::mask(k, val));
            }
        }
        if let Some(map) = ms.as_object() {
            for (k, val) in map.iter().filter(|(k, _)| !seen.contains(k)) {
                push("other", output::dim(&format!("[{}]", k)), output::mask(k, val));
            }
        }
        if let Some(map) = sv.raw.get("extension_data").and_then(Value::as_object) {
            for (k, val) in map {
                push("extension", output::dim(&format!("[ext.{}]", k)), output::mask(k, val));
            }
        }
        for (group, rows) in groups {
            let title = crate::i18n::try_t(&format!("set.group.{}", group)).unwrap_or(group);
            output::heading(&title);
            output::print_kv(&rows);
            println!();
        }
        println!(
            "{}",
            output::dim(&format!("saba-chan-cli server set {} KEY=VALUE", shell_word(&sv.name)))
        );
    });
    Ok(())
}

async fn set(ctx: &Ctx, query: &str, pairs: &[String]) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let module = module_info(ctx, &sv.module).await.unwrap_or(Value::Null);
    let fields = setting_fields(&module);
    let mut patch = Map::new();
    for (k, v) in split_pairs(pairs)? {
        let (key, value) = typed_setting(&fields, &sv.raw, &k, &v)?;
        insert_patch(&mut patch, &key, value);
    }
    let r = ctx.api.update_instance(&sv.id, Value::Object(patch.clone())).await?;
    ctx.out.done(
        &t!("set.done", name = sv.name, count = pairs.len()),
        &json!({ "success": true, "changed": patch, "response": r }),
    );
    if status_of(&runtime(ctx, &sv).await.unwrap_or(Value::Null)) == "running" {
        ctx.out.hint(&t!("set.restart_hint", name = shell_word(&sv.name)));
    }
    Ok(())
}

async fn properties(ctx: &Ctx, query: Option<&str>, action: Option<PropsCmd>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    match action.unwrap_or(PropsCmd::Show) {
        PropsCmd::Show => {
            let r = ctx.api.properties(&sv.id).await?;
            ctx.out.data(&r, |r| {
                if r.get("exists").and_then(Value::as_bool) == Some(false) {
                    ctx.out.note(&t!("props.defaults_shown"));
                }
                let props = r.get("properties").cloned().unwrap_or(json!({}));
                let mut rows: Vec<(String, String)> = props
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, v)| (k.clone(), output::mask(k, v)))
                    .collect();
                rows.sort();
                output::print_kv(&rows);
            });
        }
        PropsCmd::Set { pairs } => {
            let mut settings = Map::new();
            for (k, v) in split_pairs(&pairs)? {
                settings.insert(k, Value::String(v));
            }
            let r = ctx.api.write_properties(&sv.id, Value::Object(settings)).await?;
            ctx.out.done(&t!("props.saved", name = sv.name), &r);
        }
        PropsCmd::Reset => {
            ctx.ask.ensure(&t!("props.confirm_reset", name = sv.name))?;
            let r = ctx.api.reset_properties(&sv.id).await?;
            ctx.out.done(&t!("props.reset_done", name = sv.name), &r);
        }
    }
    Ok(())
}

// ── 버전·업데이트 ──

async fn version(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.installed_version(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let v = r
            .get("version")
            .or_else(|| r.get("server_version"))
            .map(show)
            .unwrap_or_else(|| output::dim(&t!("server.version_unknown")));
        println!("{}  {}", sv.name, v);
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
                println!("{}", t!("server.update_available", name = sv.name));
                ctx.out
                    .hint(&format!("saba-chan-cli server update {}", shell_word(&sv.name)));
            } else {
                let reason = s(r, "reason");
                let tail = crate::i18n::try_t(&format!("server.update_reason.{}", reason)).unwrap_or_default();
                println!("{} {}", t!("server.update_none", name = sv.name), output::dim(&tail));
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
        let p = follow_provision(ctx, &sv.name).await?;
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
        ctx.out.data(&p, |_| println!("{}", t!("progress.none", name = name)));
        return Ok(());
    }
    if ctx.out.json || p.get("done").and_then(Value::as_bool) == Some(true) {
        ctx.out.data(&p, |p| println!("{}", progress_line(p)));
        return Ok(());
    }
    let r = follow_provision(ctx, &name).await?;
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
            println!("{}", output::dim(&t!("history.none")));
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

// ── 점검·초기화 ──

/// 실패여도 issues가 있으면 결과로 보여 준다 (validate는 치명 문제가 있으면 success:false)
async fn checkup(ctx: &Ctx, r: Result<Value>, ok_msg: &str) -> Result<()> {
    let (value, failed_flag) = match r {
        Ok(v) => (v, false),
        Err(e) => match e.downcast_ref::<CliError>() {
            Some(CliError::Api { body, .. }) if body.get("issues").is_some() => (body.clone(), true),
            _ => return Err(e),
        },
    };
    ctx.out.data(&value, |v| {
        if !crate::cmd::print_issues(v) {
            println!("{} {}", console::style("✓").green().bold(), ok_msg);
        }
    });
    if failed_flag {
        return Err(CliError::Failed(t!("validate.blocked")).into());
    }
    Ok(())
}

async fn validate(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.validate(&sv.id).await;
    checkup(ctx, r, &t!("validate.ok", name = sv.name)).await
}

async fn diagnose(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.diagnose(&sv.id).await;
    checkup(ctx, r, &t!("diagnose.ok", name = sv.name)).await
}

async fn accept_eula(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    ctx.ask.ensure(&t!("eula.confirm", name = sv.name))?;
    let r = ctx.api.accept_eula(&sv.id).await?;
    ctx.out.done(&t!("eula.done", name = sv.name), &r);
    Ok(())
}

async fn reset(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    if status_of(&runtime(ctx, &sv).await?) == "running" {
        return Err(failed(t!("err.hint.server_running")));
    }
    ctx.ask.ensure(&t!("reset.confirm", name = sv.name))?;
    let r = ctx.api.reset_server(&sv.id).await?;
    ctx.out.done(&t!("reset.done", name = sv.name), &r);
    Ok(())
}

async fn reorder(ctx: &Ctx, names: &[String]) -> Result<()> {
    let all = resolve::servers(ctx).await?;
    let mut order = Vec::new();
    for n in names {
        let sv = resolve::server(ctx, Some(n)).await?;
        if !order.contains(&sv.id) {
            order.push(sv.id);
        }
    }
    // 적지 않은 서버는 원래 순서대로 뒤에
    let rest: Vec<String> = all
        .iter()
        .map(|s| s.id.clone())
        .filter(|id| !order.contains(id))
        .collect();
    order.extend(rest);
    let r = ctx.api.reorder(&order).await?;
    ctx.out.done(&t!("reorder.done"), &r);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (k, v) = typed_setting(&fields, &inst, "ext.box", "").unwrap();
        assert_eq!((k.as_str(), v), ("extension_data.box", Value::Null));
    }

    #[test]
    fn patch_nests_extension_data() {
        let mut p = Map::new();
        insert_patch(&mut p, "extension_data.a", json!(1));
        insert_patch(&mut p, "port", json!(2));
        assert_eq!(Value::Object(p), json!({ "extension_data": { "a": 1 }, "port": 2 }));
    }
}
