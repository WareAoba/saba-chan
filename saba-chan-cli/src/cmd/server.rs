//! `server …` 와 바로가기 `start/stop/restart/run`

use crate::catalog::{Catalog, Command, Input, Risk};
use crate::cli::{AddArgs, PropsCmd, RestartArgs, RunArgs, ServerCmd, StartArgs, StopArgs};
use crate::cmd::{split_pairs, typed};
use crate::ctx::Ctx;
use crate::error::{failed, usage, CliError, Result};
use crate::output::{self, dim, s, show};
use crate::resolve::{self, Server};
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
        ServerCmd::Discord { server, guild, word, command, clear } => {
            let change = crate::cmd::discord_words::WordsChange { words: word, commands: command, clear };
            crate::cmd::discord_words::run(ctx, server.as_deref(), guild.as_deref(), change).await
        }
        ServerCmd::Send { server, line } => send(ctx, &server, &line.join(" ")).await,
        ServerCmd::Players { server } => players(ctx, server.as_deref()).await,
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

pub fn status_of(rt: &Value) -> &str {
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
        None => format!("{} {}{}", console::style("◐").yellow(), pct, s(p, "message")),
    }
}

// ── 추가·삭제 ──

async fn add(ctx: &Ctx, a: AddArgs) -> Result<()> {
    let all = resolve::modules(ctx).await?;
    if all.is_empty() {
        return Err(failed(t!("add.no_modules")));
    }
    let given = match a.module.as_deref() {
        Some(q) => Some(resolve::module(ctx, Some(q)).await?),
        None => None,
    };

    // 1) 새로 설치 / 기존 폴더 (폴더를 고르면 게임은 폴더를 보고 알아낸다)
    let mut from = a.from.clone();
    if from.is_none() && ctx.ask.enabled && a.name.is_none() {
        let installs = given
            .as_ref()
            .is_none_or(|m| m.get("install").is_some_and(|i| !i.is_null()));
        let choices = [t!("add.fresh_install"), t!("add.use_existing")];
        if ctx
            .ask
            .select(&t!("add.ask_source"), &choices, if installs { 0 } else { 1 })?
            == Some(1)
        {
            from = ctx.ask.text(&t!("add.ask_folder"), None, false)?;
        }
    }

    // 이미 다른 서버가 쓰는 폴더면 막는다 (두 서버가 같은 파일을 덮어쓴다)
    if let Some(dir) = from.as_deref() {
        let norm = |p: &str| {
            std::fs::canonicalize(p)
                .map(|x| x.display().to_string())
                .unwrap_or_else(|_| p.trim_end_matches(['/', '\\']).to_string())
        };
        let want = norm(dir);
        for other in resolve::servers(ctx).await.unwrap_or_default() {
            let wd = s(&other.raw, "working_dir");
            if !wd.is_empty() && norm(wd) == want {
                return Err(usage(t!("add.folder_in_use", dir = dir, name = other.name)));
            }
        }
    }

    // 2) 게임
    let module = match (given, from.as_deref()) {
        (Some(m), Some(dir)) => {
            check_folder(ctx, dir, s(&m, "name")).await?;
            m
        }
        (Some(m), None) => m,
        (None, Some(dir)) => {
            let found = detected(ctx, dir).await?;
            let hits: Vec<Value> = all
                .iter()
                .filter(|m| found.iter().any(|f| f == s(m, "name")))
                .cloned()
                .collect();
            match hits.len() {
                1 => {
                    ctx.out
                        .note(&t!("add.folder_is", dir = dir, game = resolve::module_label(&hits[0])));
                    hits.into_iter().next().unwrap()
                }
                0 => {
                    ctx.out.warn(&t!("add.folder_unrecognized", dir = dir));
                    if !ctx.ask.enabled {
                        return Err(usage(t!("add.pick_game")));
                    }
                    resolve::choose(ctx, &t!("noun.game"), all.clone(), resolve::module_label)?
                }
                _ => resolve::choose(ctx, &t!("noun.game"), hits, resolve::module_label)?,
            }
        }
        (None, None) => {
            if !ctx.ask.enabled {
                return Err(usage(t!("add.module_required")));
            }
            resolve::choose(ctx, &t!("noun.game"), all.clone(), resolve::module_label)?
        }
    };
    let module_name = s(&module, "name").to_string();
    let display = if s(&module, "display_name").is_empty() {
        module_name.clone()
    } else {
        s(&module, "display_name").to_string()
    };
    if from.is_none() && module.get("install").is_none_or(|i| i.is_null()) {
        ctx.out.warn(&t!("add.no_installer", game = display));
    }

    // 3) 이름
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

    // 4) 필요한 익스텐션
    ensure_extensions(ctx, &module).await?;

    // 5) 익스텐션이 묻는 칸 (AddServer.options). 답은 만들 때 `extension_data`로 보낸다
    let mut ext_data = Map::new();
    for d in &crate::slots::create_options(&resolve::extensions(ctx).await.unwrap_or_default()) {
        let Some(field) = d.get("field").and_then(Value::as_str) else {
            continue;
        };
        let label = d.get("label").and_then(Value::as_str).unwrap_or(field);
        let default = d.get("default").cloned().unwrap_or(Value::Null);
        let value = if s(d, "type") == "boolean" {
            bool_option(&ctx.ask, label, &default)?
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

    // 5-1) 만들 때 고르는 모듈 설정 (`install = true`, 예: 마인크래프트 서버 종류). 설치할 파일이
    // 달라지므로 만든 뒤가 아니라 만들 때 보낸다. --set으로 주지 않았으면 새 서버일 때 묻는다
    let fields = setting_fields(&module);
    let is_install = |key: &str| fields.iter().any(|f| s(f, "name") == key && f.get("install") == Some(&json!(true)));
    let mut module_settings = Map::new();
    let mut later_set = Vec::new();
    for (k, v) in split_pairs(&a.set)? {
        if is_install(&k) {
            let (key, value) = typed_setting(&fields, &Value::Null, &k, &v)?;
            module_settings.insert(key, value);
        } else {
            later_set.push((k, v));
        }
    }
    // 5-1-a) 추천 설정 묶음 (`[[presets]]`). --preset으로 주지 않았으면 새 서버일 때 묻는다.
    // 설치 설정보다 먼저 고른다: 프리셋이 정한 설치 설정은 다시 묻지 않는다
    let mut preset = a.preset.clone();
    let presets: Vec<&Value> = module.get("presets").and_then(Value::as_array).map(|p| p.iter().collect()).unwrap_or_default();
    if preset.is_none() && from.is_none() && ctx.ask.enabled && !presets.is_empty() {
        let locales = module_locales(ctx, &module_name).await;
        let label = |p: &Value| {
            let id = s(p, "id");
            let localized = locales.as_ref().and_then(|l| l.pointer(&format!("/presets/{}/label", id))).and_then(Value::as_str);
            localized.filter(|l| !l.is_empty()).unwrap_or(if s(p, "label").is_empty() { id } else { s(p, "label") }).to_string()
        };
        let mut choices = vec![t!("add.no_preset")];
        choices.extend(presets.iter().map(|p| label(p)));
        if let Some(i) = ctx.ask.select(&t!("add.ask_preset"), &choices, 0)?.filter(|i| *i > 0) {
            preset = Some(s(presets[i - 1], "id").to_string());
        }
    }
    let preset_sets = |key: &str| {
        presets.iter().any(|p| Some(s(p, "id")) == preset.as_deref() && p.pointer(&format!("/settings/{}", key)).is_some())
    };
    if from.is_none() && ctx.ask.enabled {
        for f in fields.iter().filter(|f| is_install(s(f, "name"))) {
            if module_settings.contains_key(s(f, "name")) || preset_sets(s(f, "name")) {
                continue;
            }
            let options: Vec<String> = f
                .get("options")
                .and_then(Value::as_array)
                .map(|o| o.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            if options.is_empty() {
                continue;
            }
            let label = if s(f, "label").is_empty() { s(f, "name") } else { s(f, "label") };
            let default = options.iter().position(|o| Some(o.as_str()) == f.get("default").and_then(Value::as_str));
            if let Some(i) = ctx.ask.select(label, &options, default.unwrap_or(0))? {
                module_settings.insert(s(f, "name").to_string(), json!(options[i]));
            }
        }
    }

    // --set 값은 만들기 전에 검사한다 (틀리면 서버도 설치 작업도 남기지 않는다)
    let mut patch = Map::new();
    for (k, v) in later_set {
        let (key, value) = typed_setting(&fields, &Value::Null, &k, &v)?;
        insert_patch(&mut patch, &key, value);
    }

    // 6) 만들기
    let mut body = json!({ "name": name, "module_name": module_name });
    if let Some(p) = &preset {
        body["preset"] = json!(p);
    }
    if !module_settings.is_empty() {
        body["module_settings"] = Value::Object(module_settings);
    }
    if let Some(p) = a.game_port {
        body["port"] = json!(p);
    }
    if let Some(dir) = &from {
        body["migration_source"] = json!(dir);
    }
    if !ext_data.is_empty() {
        body["extension_data"] = Value::Object(ext_data);
    }
    let created = ctx.api.create_instance(body).await?;
    let id = s(&created, "id").to_string();
    super::print_trust_warnings(ctx, &created);
    // 7-0) 약관 (예: 게임 EULA). 설치를 기다리기 전에 묻는다
    offer_agreements(ctx, &id, &name, &a.accept).await;

    // 7) 설정 (--set). 틀린 값은 위에서 이미 걸렀다. 데몬이 거절하면 서버는 남기고 끝에 실패로 알린다
    let mut settings_error = None;
    if !patch.is_empty() {
        match ctx.api.update_instance(&id, Value::Object(patch)).await {
            Err(e) => {
                ctx.out.warn(&t!("add.settings_failed", error = e));
                settings_error = Some(t!("add.created_settings_failed", name = name, error = e));
            }
            Ok(r) => {
                if let Some(e) = sync_failure(&r) {
                    ctx.out.warn(&e);
                    settings_error = add_settings_failure(&name, &r);
                }
            }
        }
    }

    // 8) 설치 진행
    let provisioning = created.get("provisioning").and_then(Value::as_bool) == Some(true);
    if provisioning {
        if a.no_wait {
            if let Some(error) = settings_error {
                return Err(CliError::Partial {
                    message: error,
                    data: json!({ "id": id, "name": name, "module": module_name, "provisioning": true }),
                }
                .into());
            }
            ctx.out.done(&t!("add.created_installing", name = name), &created);
            ctx.out.hint(&format!("saba-chan-cli server progress {}", name));
            return Ok(());
        }
        let result = follow_provision(ctx, &name, true).await?;
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
    if let Some(error) = settings_error {
        return Err(CliError::Partial {
            message: error,
            data: json!({ "id": id, "name": name, "module": module_name, "provisioned": provisioning }),
        }
        .into());
    }
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
pub fn shell_word(s: &str) -> String {
    if s.contains(char::is_whitespace) {
        format!("\"{}\"", s)
    } else {
        s.to_string()
    }
}

/// 폴더를 보고 어떤 게임인지 (모듈 이름들)
async fn detected(ctx: &Ctx, dir: &str) -> Result<Vec<String>> {
    let r = ctx.api.detect(dir).await?;
    Ok(r.get("matches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| s(m, "name").to_string())
        .collect())
}

/// 기존 폴더가 이 게임 서버인지 데몬에게 물어본다
async fn check_folder(ctx: &Ctx, dir: &str, module: &str) -> Result<()> {
    let matches = detected(ctx, dir).await?;
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
        let r = ctx.api.enable_extension(id).await?;
        ctx.out.done(&t!("ext.enabled", id = id), &json!({ "id": id }));
        // `extension enable`처럼 미확인·변경된 익스텐션이면 알린다
        super::print_trust_warnings(ctx, &r);
    }
    Ok(())
}

/// 따라가던 설치 항목이 끝을 보이기 전에 사라졌다. 부르는 쪽은 항목이 있을 때만 따라가므로 성공으로 치지 않는다
fn provision_lost(p: &Value) -> bool {
    p.get("active").and_then(Value::as_bool) != Some(true)
}

/// 항목이 사라졌는데 그 이름의 서버가 설치 중이 아닌 채로 있으면, 끝난 항목을 다른 곳(GUI 카드)에서 다음 폴링 전에 닫은 것이다.
/// 실패한 새 설치는 닫을 때 지워지므로 남은 서버는 성공으로 본다 (L65). 새로 만든 서버의 설치에만 쓴다
fn closed_after_finish(servers: &Value, name: &str) -> bool {
    servers
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|x| s(x, "name") == name))
        .is_some_and(|x| status_of(x) != "provisioning")
}

/// 설치 진행률을 끝날 때까지 보여 준다. Ctrl+C는 기다리기만 멈춘다 (설치는 계속).
/// `created`: 방금 만든 서버의 첫 설치다. 기존 서버의 업데이트는 실패해도 닫을 때 서버가 남으므로 사라진 항목을 성공으로 보지 않는다
async fn follow_provision(ctx: &Ctx, name: &str, created: bool) -> Result<Value> {
    let bar = crate::progress::percent_bar(&t!("add.installing"));
    loop {
        let p = ctx.api.provision_progress(name).await?;
        if provision_lost(&p) {
            bar.finish_and_clear();
            if created && closed_after_finish(&ctx.api.servers(true).await?, name) {
                return Ok(json!({ "active": false, "done": true }));
            }
            return Err(failed(t!("progress.lost", name = name)));
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
    let sv = resolve::server_exact(ctx, query).await?;
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
    stop_server(ctx, &sv, a.force, a.no_wait).await
}

pub async fn stop_server(ctx: &Ctx, sv: &Server, force: bool, no_wait: bool) -> Result<()> {
    if status_of(&runtime(ctx, sv).await?) == "stopped" {
        ctx.out.done(
            &t!("stop.already", name = sv.name),
            &json!({ "success": true, "status": "stopped" }),
        );
        return Ok(());
    }
    let r = match ctx.api.stop(&sv.id, force).await {
        Err(e) if !force && cannot_save_and_exit(&e) => {
            agree_to_unsaved_stop(ctx, sv, e, "stop")?;
            ctx.api.stop(&sv.id, true).await?
        }
        r => r?,
    };
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
    let mut r = ctx.api.restart(&sv.id, a.force).await;
    spin.finish_and_clear();
    if let Err(e) = r {
        r = if !a.force && cannot_save_and_exit(&e) {
            agree_to_unsaved_stop(ctx, &sv, e, "restart")?;
            let spin = crate::progress::spinner(&t!("restart.waiting", name = sv.name));
            let r = ctx.api.restart(&sv.id, true).await;
            spin.finish_and_clear();
            r
        } else {
            Err(e)
        };
    }
    tell_moved_ports(ctx, &r);
    finish_start(ctx, &sv, r, false, t!("restart.done", name = sv.name)).await
}

/// 모듈이 저장하고 끄라고 알릴 길이 없다며 정지를 거절했다 (서버는 그대로 켜져 있다)
fn cannot_save_and_exit(e: &anyhow::Error) -> bool {
    api_code(e) == Some("graceful_stop_unavailable")
}

/// 저장 없이 강제로 끌지 사람에게 묻는다. `--yes`는 동의로 치지 않고, 물을 수 없으면 데몬의 오류를 그대로 낸다.
fn agree_to_unsaved_stop(ctx: &Ctx, sv: &Server, e: anyhow::Error, cmd: &str) -> Result<()> {
    if !ctx.ask.enabled {
        ctx.out
            .hint(&format!("saba-chan-cli {cmd} {} --force", shell_word(&sv.name)));
        return Err(e);
    }
    ctx.out.warn(&t!("stop.no_graceful", name = sv.name));
    if ctx.ask.consent(&t!("stop.force_confirm")) {
        Ok(())
    } else {
        Err(CliError::Cancelled.into())
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
/// 콘솔에 친 한 줄. 정의된 명령이면 인자를 나눠 묶고, 아니면 친 그대로 보낸다
/// (따옴표·JSON이 든 게임 명령을 다시 이어 붙이면 모양이 바뀐다)
pub async fn exec_line(ctx: &Ctx, sv: &Server, cat: &Catalog, text: &str) -> Result<()> {
    let words = shlex::split(text).unwrap_or_else(|| text.split_whitespace().map(str::to_string).collect());
    let name = words.first().map(String::as_str).unwrap_or_default();
    if cat.find(name).is_none() && cat.accepts_raw() {
        let r = ctx.api.command(&sv.id, text, Value::Null).await?;
        print_result(ctx, &r);
        return Ok(());
    }
    let (name, rest) = words.split_first().map(|(n, r)| (n.clone(), r.to_vec())).unwrap_or_default();
    exec(ctx, sv, cat, &name, &rest).await
}

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
        let v = ask_input(&ctx.ask, def, &input)?;
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

/// 시작·재시작 답의 `port_moved: [{field, from, to}]` → 알릴 줄
fn port_moved_lines(body: &Value) -> Vec<String> {
    body.get("port_moved")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| Some(t!("start.port_moved", from = m.get("from")?, to = m.get("to")?)))
        .collect()
}

/// 예약 범위를 피해 옮긴 포트는 시작이 실패해도 저장돼 있으므로, 답이 성공이든 오류든 알린다
fn tell_moved_ports(ctx: &Ctx, res: &Result<Value>) {
    let body = match res {
        Ok(v) => Some(v),
        Err(e) => match e.downcast_ref::<CliError>() {
            Some(CliError::Api { body, .. }) => Some(body),
            _ => None,
        },
    };
    for line in body.map(port_moved_lines).unwrap_or_default() {
        ctx.out.note(&line);
    }
}

/// API 오류의 `error_code`
fn api_code(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<CliError>() {
        Some(CliError::Api { body, .. }) => body.get("error_code").and_then(Value::as_str),
        _ => None,
    }
}

fn ask_input(ask: &crate::prompt::Prompt, def: &Command, input: &Input) -> Result<Value> {
    let missing = || usage(t!("run.missing_arg", label = input.label, usage = def.usage()));
    if !input.options.is_empty() {
        return match ask.select(&input.label, &input.options, 0)? {
            Some(i) => Ok(Value::String(input.options[i].clone())),
            None => Err(missing()),
        };
    }
    let prompt = match &input.placeholder {
        Some(p) => format!("{} {}", input.label, output::dim(&format!("({})", p))),
        None => input.label.clone(),
    };
    // 비밀번호 칸은 화면에 보이지 않게 묻는다
    let answer = if input.kind == "password" {
        ask.secret(&prompt)?.filter(|v| !v.is_empty())
    } else {
        ask.text(&prompt, None, false)?
    };
    match answer {
        Some(v) => Ok(Value::String(v)),
        None => Err(missing()),
    }
}

/// `add --set` 저장 응답이 `synced: false`면 끝에 알릴 문구. 설정은 저장했으므로 "저장하지 못했다"가 아니다 (L63)
fn add_settings_failure(name: &str, r: &Value) -> Option<String> {
    (r.get("synced").and_then(Value::as_bool) == Some(false))
        .then(|| t!("add.created_settings_unsynced", name = name, error = s(r, "sync_error")))
}

/// 설정은 저장했지만 게임 설정 파일에 쓰지 못했으면(`synced: false`) 그 이유
fn sync_failure(r: &Value) -> Option<String> {
    (r.get("synced").and_then(Value::as_bool) == Some(false)).then(|| t!("set.sync_failed", error = s(r, "sync_error")))
}

/// 명령 응답을 사람이 읽게: 문자열 → 그대로, 객체 배열 → 표, 객체 → 이름: 값
pub fn print_result(ctx: &Ctx, r: &Value) {
    // 콘솔에 붙어 있으면 stdin 답은 콘솔 줄로 이미 보인다
    if ctx.attached && !ctx.out.json && s(r, "protocol") == "stdin" {
        return;
    }
    ctx.out.data(r, |r| {
        let data = r.get("data").filter(|d| !d.is_null());
        let text = data
            .and_then(|d| d.get("response").and_then(Value::as_str).or_else(|| d.as_str()))
            .or_else(|| r.get("response").and_then(Value::as_str))
            .or_else(|| r.get("message").and_then(Value::as_str))
            .or_else(|| r.get("output").and_then(Value::as_str));
        if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
            say!("{}", t.trim_end());
            return;
        }
        // 빈 답(`{response: "", executed}`)은 보낸 것만 알린다
        let empty_reply = data.is_some_and(|d| {
            d.get("response")
                .is_some_and(|x| x.as_str().is_some_and(|s| s.trim().is_empty()))
        });
        match data.filter(|_| !empty_reply) {
            Some(d) => print_value(d),
            None => say!("{} {}", console::style("✓").green().bold(), t!("run.sent")),
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
                say!("{}", show(it));
            }
        }
        other => say!("{}", show(other)),
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
            say!("{}", output::dim(&t!("run.no_commands")));
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
            say!();
            say!("{}", output::dim(&t!("run.raw_ok")));
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

async fn players(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.players(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let online = r.get("online").and_then(Value::as_u64).unwrap_or(0);
        let count = match r.get("max").and_then(Value::as_u64) {
            Some(max) => format!("{online}/{max}"),
            None => online.to_string(),
        };
        output::heading(&t!("players.title", server = sv.name, count = count));
        let names: Vec<&str> = r.get("players").and_then(Value::as_array).into_iter().flatten().map(|p| s(p, "name")).filter(|n| !n.is_empty()).collect();
        if names.is_empty() {
            say!("{}", output::dim(&t!("players.none")));
        }
        for n in names {
            say!("  {}", n);
        }
    });
    Ok(())
}

async fn log(ctx: &Ctx, query: Option<&str>, source: Option<&str>, lines: Option<usize>, runs: bool, all: bool) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    if all {
        let r = ctx.api.export_log(&sv.id, source).await?;
        ctx.out.data(&r, |r| {
            say!("{}", s(r, "content").trim_end());
            if r.get("truncated").and_then(Value::as_bool) == Some(true) {
                ctx.out.warn(&t!("log.truncated"));
            }
        });
        return Ok(());
    }
    let r = ctx.api.logs(&sv.id, source, lines, None).await?;
    ctx.out.data(&r, |r| {
        let sources = r.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
        if sources.is_empty() {
            say!("{}", output::dim(&t!("log.none")));
            return;
        }
        if runs {
            let mut t = output::table(&[t!("log.col.run"), t!("log.col.started")]);
            for src in &sources {
                t.add_row(vec![s(src, "id"), s(src, "label")]);
            }
            output::print_table(t);
            return;
        }
        for l in r.get("lines").and_then(Value::as_array).into_iter().flatten() {
            say!("{}", l.as_str().unwrap_or_default());
        }
    });
    Ok(())
}

async fn query(ctx: &Ctx, server: &str, name: Option<&str>, args: &[String]) -> Result<()> {
    let sv = resolve::server(ctx, Some(server)).await?;
    let Some(name) = name else {
        let m = ctx.api.module_metadata(&sv.module).await?;
        let list = m.pointer("/metadata/queries").cloned().unwrap_or(Value::Array(Vec::new()));
        ctx.out.data(&list, |list| {
            let items = list.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                say!("{}", output::dim(&t!("query.none")));
                return;
            }
            let mut t = output::table(&[t!("query.col.name"), t!("query.col.about"), t!("query.col.inputs")]);
            for q in &items {
                let about = q.get("label").or_else(|| q.get("description")).and_then(Value::as_str).unwrap_or("");
                let inputs: Vec<&str> = q.get("inputs").and_then(Value::as_array).into_iter().flatten().map(|i| s(i, "name")).collect();
                t.add_row(vec![s(q, "name").to_string(), about.to_string(), inputs.join(", ")]);
            }
            output::print_table(t);
        });
        return Ok(());
    };
    let input: serde_json::Map<String, Value> = crate::cmd::split_pairs(args)?.into_iter().map(|(k, v)| (k, crate::cmd::guess(&v))).collect();
    let r = ctx.api.query(&sv.id, name, Value::Object(input)).await?;
    ctx.out.data(&r, |r| print_value(r.get("data").unwrap_or(&Value::Null)));
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

/// 모듈이 선언하지 않아도 모든 서버에 있는 칸과 그 형식: 인스턴스 공통 칸과 데몬이 읽는 설정 (데몬과 같은 표)
fn common() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    use saba_chan_updater_lib::constants::{DAEMON_SETTINGS, INSTANCE_FIELDS};
    INSTANCE_FIELDS.iter().chain(DAEMON_SETTINGS)
}

/// KEY=VALUE 하나를 PATCH 키와 형식 맞춘 값으로. `ext.키`는 extension_data, 빈 값은 지우기(null).
/// 모듈 정의에도, 공통 칸에도, 지금 값에도 없는 키 (오타일 가능성이 높다)
fn unknown_setting(fields: &[Value], key: &str) -> anyhow::Error {
    let names: Vec<String> = fields
        .iter()
        .map(|f| s(f, "name").to_string())
        .chain(common().map(|(k, _)| k.to_string()))
        .collect();
    let suggestions = match resolve::find(&names, key, |n| vec![n.clone()], |n| n.clone()) {
        resolve::Found::None(s) => s,
        resolve::Found::One(n) => vec![n],
        resolve::Found::Many(v) => v.into_iter().take(3).collect(),
    };
    CliError::NotFound {
        what: t!("noun.setting"),
        query: key.into(),
        suggestions,
    }
    .into()
}

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
        .or_else(|| common().find(|(k, _)| *k == key).map(|(_, t)| t.to_string()))
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
        .ok_or_else(|| unknown_setting(fields, key))?;
    // 공통 칸은 빈 값이 지우기다 (예: `memory_limit_mb=` → 제한 없음). 데몬이 비우지 않는 칸은 거절한다
    if raw.trim().is_empty() && common().any(|(k, _)| *k == key) && !fields.iter().any(|f| s(f, "name") == key) {
        if saba_chan_updater_lib::constants::UNCLEARABLE_INSTANCE_FIELDS.contains(&key) {
            return Err(usage(t!("set.cannot_clear", key = key)));
        }
        return Ok((key.to_string(), Value::Null));
    }
    let field = fields.iter().find(|f| s(f, "name") == key);
    if let Some(opts) = field.and_then(|f| f.get("options")).and_then(Value::as_array) {
        let opts: Vec<&str> = opts.iter().filter_map(Value::as_str).collect();
        if !opts.is_empty() {
            // 데몬은 선택지를 그대로 비교하므로, 대소문자만 다르게 친 값은 모듈이 정한 선택지로 보낸다 (L58)
            let pick = |raw: &str| {
                opts.iter()
                    .find(|o| **o == raw)
                    .or_else(|| opts.iter().find(|o| o.eq_ignore_ascii_case(raw)))
                    .map(|o| o.to_string())
                    .ok_or_else(|| usage(t!("set.must_be_one_of", key = key, options = opts.join(", "))))
            };
            // 여러 개 고르는 목록은 항목마다
            if kind == "list" {
                let items = typed(raw, &kind)?;
                let picked = items
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|i| pick(i.as_str().unwrap_or_default()).map(Value::String))
                    .collect::<Result<Vec<_>>>()?;
                return Ok((key.to_string(), Value::Array(picked)));
            }
            return Ok((key.to_string(), typed(&pick(raw)?, &kind)?));
        }
    }
    Ok((key.to_string(), typed(raw, &kind)?))
}

/// 선택형 설정의 선택지 (`a | b | c`). 선택지가 없으면 None
fn options_line(f: &Value) -> Option<String> {
    let opts: Vec<&str> = f.get("options")?.as_array()?.iter().filter_map(Value::as_str).collect();
    (!opts.is_empty()).then(|| opts.join(" | "))
}

/// 설정 값 한 줄: 목록은 쉼표로, 여러 줄은 ⏎로 잇는다. 비밀번호 같은 값은 가린다
fn setting_text(name: &str, val: &Value) -> String {
    match val {
        Value::Array(items) if items.iter().all(Value::is_string) => output::mask(
            name,
            &json!(items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
        ),
        Value::String(text) if text.contains('\n') => {
            output::mask(name, &json!(text.lines().collect::<Vec<_>>().join(" ⏎ ")))
        }
        _ => output::mask(name, val),
    }
}

/// 설정 옆 표시: 켜져 있을 때 바꾸면 다시 켜야 적용되는 설정 (06 §8.2)
fn setting_marks(f: &Value) -> String {
    if f.get("restart_required").and_then(Value::as_bool) == Some(true) {
        output::dim(&format!("({})", t!("set.restart_required_mark")))
    } else {
        String::new()
    }
}

/// 저장 응답의 `restart_required`: 켜져 있을 때 값이 바뀌어 다시 켜야 적용되는 설정 이름
fn restart_required(r: &Value) -> Vec<String> {
    r.get("restart_required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
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
    let (module, locales, live) = tokio::join!(
        module_info(ctx, &sv.module),
        module_locales(ctx, &sv.module),
        ctx.api.live_settings(&sv.id)
    );
    let module = module.unwrap_or(Value::Null);
    let fields = setting_fields(&module);
    // 다른 설정 값에 따라 지금 숨긴 설정 (저장된 값 기준, 06 §8.2)
    let hidden: Vec<String> = ctx
        .api
        .instance(&sv.id)
        .await
        .ok()
        .and_then(|r| r.get("hidden").cloned())
        .and_then(|h| serde_json::from_value(h).ok())
        .unwrap_or_default();
    // 실제 설정 파일 값이 우선. 못 읽으면 (설치 전, 모듈이 지원 안 함) 저장된 값으로.
    let live = live
        .ok()
        .and_then(|r| r.get("settings").cloned())
        .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    // 데몬이 읽은 값으로 인스턴스(포트·비밀번호 칸 등)도 맞췄으니 다시 받는다
    let sv = match &live {
        Some(_) => resolve::server(ctx, Some(&sv.id)).await.unwrap_or(sv),
        None => sv,
    };
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
        live.as_ref()
            .and_then(|l| l.get(name))
            .filter(|v| !v.is_null())
            .or_else(|| sv.raw.get(name).filter(|v| !v.is_null()))
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
            "hidden": hidden.iter().any(|h| h == name),
            "restart_required": f.get("restart_required").and_then(Value::as_bool).unwrap_or(false),
        }));
    }
    let v = json!({ "server": sv.name, "live": live.is_some(), "fields": out_fields, "instance": sv.raw });
    ctx.out.data(&v, |_| {
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut push = |group: &str, k: String, v: String| match groups.iter_mut().find(|(g, _)| g == group) {
            Some((_, rows)) => rows.push((k, v)),
            None => groups.push((group.to_string(), vec![(k, v)])),
        };
        let mut seen: Vec<String> = Vec::new();
        for f in &fields {
            let name = s(f, "name");
            if hidden.iter().any(|h| h == name) {
                seen.push(name.to_string());
                continue;
            }
            let group = f.get("group").and_then(Value::as_str).unwrap_or("basic");
            let val = current(name);
            let mut shown = setting_text(name, &val);
            if val.is_null() {
                if let Some(d) = f.get("default").filter(|d| !d.is_null()) {
                    shown = output::dim(&format!("{} ({})", setting_text(name, d), t!("set.default")));
                }
            }
            if let Some(line) = options_line(f) {
                shown = format!("{}  {}", shown, output::dim(&format!("({})", line)));
            }
            let marks = setting_marks(f);
            if !marks.is_empty() {
                shown = format!("{}  {}", shown, marks);
            }
            push(
                group,
                format!("{} {}", label(f), output::dim(&format!("[{}]", name))),
                shown,
            );
            seen.push(name.to_string());
        }
        for (k, _) in saba_chan_updater_lib::constants::INSTANCE_FIELDS {
            if seen.iter().any(|s| s == k) {
                continue;
            }
            if let Some(val) = sv.raw.get(*k).filter(|v| !v.is_null()) {
                push("saba-chan", output::dim(&format!("[{}]", k)), output::mask(k, val));
            }
        }
        // 데몬이 읽는 설정은 값이 없어도 보인다: 바꿀 수 있다는 것을 알 수 있게
        for (k, _) in saba_chan_updater_lib::constants::DAEMON_SETTINGS {
            if seen.iter().any(|s| s == k) {
                continue;
            }
            let shown = match ms.get(*k).filter(|v| !v.is_null()) {
                Some(val) => output::mask(k, val),
                None => output::dim(&format!("({})", t!("set.default"))),
            };
            push("saba-chan", output::dim(&format!("[{}]", k)), shown);
            seen.push(k.to_string());
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
            say!();
        }
        if !hidden.is_empty() {
            say!("{}", output::dim(&t!("set.hidden", keys = hidden.join(", "))));
        }
        say!(
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
    let data = json!({ "success": true, "changed": patch, "response": r });
    // 게임 설정 파일에 쓰지 못했으면 끝에 실패로 알린다 (--json은 오류 문서 하나에 결과를 싣는다)
    let sync_error = sync_failure(&r);
    if sync_error.is_none() || !ctx.out.json {
        ctx.out.done(&t!("set.done", name = sv.name, count = pairs.len()), &data);
    }
    let ignored: Vec<&str> = r.get("ignored").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    if !ignored.is_empty() {
        ctx.out.warn(&t!("set.ignored", keys = ignored.join(", ")));
    }
    // 다시 켜야 적용되는 설정은 데몬이 알려 준다 (06 §8.2). 모르는 옛 데몬이면 켜져 있을 때 늘 안내한다
    let needs_restart = restart_required(&r);
    if !needs_restart.is_empty() {
        ctx.out
            .warn(&t!("set.restart_required", keys = needs_restart.join(", ")));
        ctx.out.hint(&t!("set.restart_hint", name = shell_word(&sv.name)));
    } else if r.get("restart_required").is_none()
        && status_of(&runtime(ctx, &sv).await.unwrap_or(Value::Null)) == "running"
    {
        ctx.out.hint(&t!("set.restart_hint", name = shell_word(&sv.name)));
    }
    match sync_error {
        Some(message) => Err(CliError::Partial { message, data }.into()),
        None => Ok(()),
    }
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
            if let Ok(m) = module_info(ctx, &sv.module).await {
                crate::cmd::module::ensure_supported(&m, "reset_properties")?;
            }
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

// ── 점검·초기화 ──

/// 실패여도 issues가 있으면 결과로 보여 준다 (validate는 치명 문제가 있으면 success:false)
async fn checkup(ctx: &Ctx, r: Result<Value>, ok_msg: &str, server: &str) -> Result<()> {
    let (value, failed_flag) = match r {
        Ok(v) => (v, false),
        Err(e) => match e.downcast_ref::<CliError>() {
            // JSON이면 데몬 오류를 그대로 (error_code·issues·solution이 다 들어 있다)
            Some(CliError::Api { body, .. }) if body.get("issues").is_some() && !ctx.out.json => (body.clone(), true),
            _ => return Err(e),
        },
    };
    ctx.out.data(&value, |v| {
        if !crate::cmd::print_issues(v) {
            say!("{} {}", console::style("✓").green().bold(), ok_msg);
        }
    });
    // 동의하지 않은 모듈 약관이 있으면 바로 풀 수 있는 명령을 알려 준다
    if value.get("error_code").and_then(Value::as_str) == Some("agreement_required") {
        ctx.out.hint(&format!(
            "saba-chan-cli server agree {}",
            shlex::try_quote(server).unwrap_or_else(|_| server.into())
        ));
    }
    if failed_flag {
        return Err(CliError::Failed(t!("validate.blocked")).into());
    }
    Ok(())
}

async fn validate(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.validate(&sv.id).await;
    checkup(ctx, r, &t!("validate.ok", name = sv.name), &sv.name).await
}

async fn diagnose(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.diagnose(&sv.id).await;
    checkup(ctx, r, &t!("diagnose.ok", name = sv.name), &sv.name).await
}

/// 데몬이 실제로 띄울 줄 (서버는 켜지 않는다). 형식이 틀린 인자는 경고로
async fn launch_preview(ctx: &Ctx, query: Option<&str>, extra: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.launch_preview(&sv.id, extra).await?;
    ctx.out.data(&r, |r| {
        if r.get("has_launch_args").and_then(Value::as_bool) == Some(false) {
            say!("{}", dim(&t!("launch_preview.no_args", name = sv.name)));
        }
        match r.get("command_line").and_then(Value::as_str) {
            Some(line) if r.get("success").and_then(Value::as_bool) == Some(true) => say!("{}", line),
            _ => say!("{} {}", console::style("✗").red(), s(r, "message")),
        }
        for e in r.get("errors").and_then(Value::as_array).into_iter().flatten() {
            let what = e.get("arg").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| show(&e["got"]));
            say!("{} {}", console::style("!").yellow(), t!("launch_preview.invalid_format", arg = what, field = s(e, "field")));
        }
    });
    Ok(())
}

/// 모듈이 선언한 약관 중 아직 동의하지 않은 것을 원문 주소와 함께 보여 주고, 확인을 받아 동의한다
async fn agree(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let inst = ctx.api.instance(&sv.id).await?;
    let pending = pending_agreements(&inst);
    if pending.is_empty() {
        ctx.out.done(&t!("agree.none", name = sv.name), &json!({ "success": true, "agreements": inst["agreements"] }));
        return Ok(());
    }
    show_agreements(ctx, &pending);
    let names: Vec<&str> = pending.iter().map(|a| s(a, "label")).collect();
    ctx.ask.agreed(&t!("agree.confirm", name = sv.name, names = names.join(", ")))?;
    let r = accept(ctx, &sv.id, &pending).await?;
    ctx.out.done(&t!("agree.done", name = sv.name), &r);
    Ok(())
}

/// 익스텐션이 묻는 예/아니오 칸 (AddServer.options). `--yes`는 확인을 넘길 뿐 값을 정하지 않으므로 기본값을 쓴다
fn bool_option(ask: &crate::prompt::Prompt, label: &str, default: &Value) -> Result<Value> {
    if ask.enabled && !ask.yes {
        Ok(Value::Bool(ask.confirm(label, default.as_bool().unwrap_or(false))?))
    } else {
        Ok(default.clone())
    }
}

/// 모듈이 선언했지만 아직 동의하지 않은 약관 (인스턴스 응답의 `agreements`)
fn pending_agreements(inst: &Value) -> Vec<Value> {
    inst["agreements"].as_array().into_iter().flatten().filter(|a| a["accepted"] == json!(false)).cloned().collect()
}

/// `--accept`로 댔지만 이 서버의 약관에 없는 id (오타)
fn unknown_accepts<'a>(inst: &Value, named: &'a [String]) -> Vec<&'a str> {
    let agreements = inst["agreements"].as_array().cloned().unwrap_or_default();
    named.iter().map(String::as_str).filter(|n| !agreements.iter().any(|a| s(a, "id") == *n)).collect()
}

fn show_agreements(ctx: &Ctx, pending: &[Value]) {
    if !ctx.out.json {
        for a in pending {
            say!("  {}  {}", console::style(s(a, "label")).bold(), s(a, "url"));
        }
    }
}

async fn accept(ctx: &Ctx, id: &str, pending: &[Value]) -> Result<Value> {
    let ids: Vec<String> = pending.iter().map(|a| s(a, "id").to_string()).collect();
    ctx.api.accept_agreements(id, &ids).await
}

/// 동의할 약관을 나눈다: `--accept`로 id를 댄 것(바로 동의)과 사람에게 물을 것.
/// 물을 수 없으면(`--json`, 터미널 아님) 묻지 않는다. `--yes`는 약관 동의로 치지 않는다
fn split_agreements(pending: &[Value], named: &[String], can_ask: bool) -> (Vec<Value>, Vec<Value>) {
    let (by_name, rest): (Vec<Value>, Vec<Value>) = pending.iter().cloned().partition(|a| named.iter().any(|n| n == s(a, "id")));
    (by_name, if can_ask { rest } else { Vec::new() })
}

/// 만든 서버에 동의할 약관이 있으면 GUI처럼 만들 때 묻는다. 동의하지 않았거나 실패해도 서버 만들기는
/// 이어 가고 `server agree`를 안내한다 (첫 start가 약관 때문에 막히지 않게, 만든 서버의 남은 단계는 그대로)
async fn offer_agreements(ctx: &Ctx, id: &str, name: &str, named: &[String]) {
    let Ok(inst) = ctx.api.instance(id).await else {
        return;
    };
    let unknown = unknown_accepts(&inst, named);
    if !unknown.is_empty() {
        let known: Vec<&str> = inst["agreements"].as_array().into_iter().flatten().map(|a| s(a, "id")).collect();
        ctx.out.warn(&t!("agree.unknown_id", ids = unknown.join(", "), known = known.join(", ")));
    }
    let pending = pending_agreements(&inst);
    if pending.is_empty() {
        return;
    }
    let (by_name, ask) = split_agreements(&pending, named, ctx.ask.enabled);
    let mut chosen = by_name;
    if !ask.is_empty() {
        show_agreements(ctx, &ask);
        let names: Vec<&str> = ask.iter().map(|a| s(a, "label")).collect();
        if ctx.ask.consent(&t!("agree.confirm", name = name, names = names.join(", "))) {
            chosen.extend(ask);
        }
    }
    let accepted = if chosen.is_empty() {
        false
    } else {
        match accept(ctx, id, &chosen).await {
            Ok(_) => true,
            Err(e) => {
                ctx.out.warn(&e.to_string());
                false
            }
        }
    };
    if accepted && !ctx.out.json {
        say!("{} {}", console::style("✓").green().bold(), t!("agree.done", name = name));
    }
    if !accepted || chosen.len() < pending.len() {
        ctx.out.hint(&format!("saba-chan-cli server agree {}", shell_word(name)));
    }
}

async fn reset(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server_exact(ctx, query).await?;
    if status_of(&runtime(ctx, &sv).await?) == "running" {
        return Err(failed(t!("err.hint.server_running")));
    }
    if let Ok(m) = module_info(ctx, &sv.module).await {
        crate::cmd::module::ensure_supported(&m, "reset_server")?;
    }
    let locales = module_locales(ctx, &sv.module).await;
    ctx.ask.ensure(&reset_question(&sv.name, locales.as_ref()))?;
    let r = ctx.api.reset_server(&sv.id).await?;
    ctx.out.done(&t!("reset.done", name = sv.name), &r);
    Ok(())
}

/// 초기화 확인 문구. 무엇이 지워지는지는 게임마다 달라 모듈 locales의 `reset.description`을 덧붙인다 (GUI와 같다)
fn reset_question(name: &str, locales: Option<&Value>) -> String {
    let question = t!("reset.confirm", name = name);
    match locales.and_then(|l| l.pointer("/reset/description")).and_then(Value::as_str).filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("{}\n{}", question, d),
        None => question,
    }
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

    /// 설치 진행 항목이 끝(done)을 보이기 전에 사라지면(데몬 재시작, 다른 곳에서 이름 바꾸기·삭제) 성공이 아니다 (L52)
    #[test]
    fn a_vanished_install_is_not_reported_as_finished() {
        assert!(provision_lost(&json!({ "active": false })));
        assert!(provision_lost(&json!({})));
        assert!(!provision_lost(&json!({ "active": true, "done": false })));
        assert!(!provision_lost(&json!({ "active": true, "done": true })));
    }

    /// 콘솔이 없어 저장 후 종료를 요청할 수 없다는 데몬 답(MK10)만 강제 정지를 물을 거리다. 다른 실패는 그대로 낸다
    #[test]
    fn only_a_refused_save_and_exit_offers_a_force_stop() {
        let refused = crate::client::check(200, json!({ "success": false, "error_code": "graceful_stop_unavailable", "error": "no console" })).unwrap_err();
        assert!(cannot_save_and_exit(&refused));
        let other = crate::client::check(500, json!({ "success": false, "error_code": "stop_failed", "error": "x" })).unwrap_err();
        assert!(!cannot_save_and_exit(&other));
        assert!(!cannot_save_and_exit(&anyhow::anyhow!("graceful_stop_unavailable")));
    }

    /// 저장 없이 끄는 것은 사람이 고른다: `--yes`로 대신하지 않고, 물을 수 없으면 원래 오류로 끝낸다
    #[test]
    fn an_unsaved_stop_is_never_agreed_on_by_yes() {
        crate::i18n::init("ko");
        let ctx = Ctx {
            api: crate::client::Api::new(1, None),
            spawn_port: 1,
            port_fixed: true,
            out: crate::output::Out::new(true, false),
            ask: crate::prompt::Prompt { enabled: false, yes: true },
            in_shell: false,
            attached: false,
        };
        let sv = Server { id: "i".into(), name: "mc".into(), module: "minecraft".into(), raw: json!({}) };
        let refused = crate::client::check(200, json!({ "success": false, "error_code": "graceful_stop_unavailable" })).unwrap_err();
        let e = agree_to_unsaved_stop(&ctx, &sv, refused, "stop").unwrap_err();
        assert!(cannot_save_and_exit(&e), "the daemon's code is kept: {e}");
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
        assert_eq!(typed_setting(&[], &inst, "rest_port", "").unwrap().1, Value::Null);
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

    /// 약관 동의는 사람이 직접 답한다: `server agree`도 `--yes`로 동의하지 않는다 (확인 `ensure`가 아니라 동의 `agreed`)
    #[test]
    fn server_agree_asks_for_a_consent_not_a_confirmation() {
        // Windows 체크아웃은 줄 끝이 CRLF일 수 있다
        let src = include_str!("server.rs").replace("\r\n", "\n");
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
