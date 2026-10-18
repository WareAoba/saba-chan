//! `bot` — 디스코드 봇 연결 (클라우드 릴레이)
//!
//! 봇은 클라우드에서만 돈다. 이 PC는 릴레이 에이전트로 붙는다:
//! 페어링 → 노드 토큰 저장(데몬이 함) → 에이전트 시작.
//! 봇 설정(`/api/config/bot`)은 최상위 키 단위로 머지되므로, 맵(`moduleAliases`)은 통째로 보낸다.

use crate::cli::{AliasCmd, BotCmd};
use crate::cmd::{flatten, get_path, guess, set_path, split_pairs};
use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use crate::output::{self, s};
use serde_json::{json, Map, Value};
use std::time::Duration;

pub async fn run(ctx: &Ctx, cmd: BotCmd) -> Result<()> {
    match cmd {
        BotCmd::Status => status(ctx).await,
        BotCmd::Start => {
            let r = ctx.api.agent_start().await?;
            ctx.out.done(&t!("bot.started"), &r);
            Ok(())
        }
        BotCmd::Stop => {
            let r = ctx.api.agent_stop().await?;
            ctx.out.done(&t!("bot.stopped"), &r);
            Ok(())
        }
        BotCmd::Pair { relay_url } => pair(ctx, relay_url).await,
        BotCmd::Unpair => {
            ctx.ask.ensure(&t!("bot.confirm_unpair"))?;
            // 데몬이 토큰을 지우기 전에 에이전트를 먼저 끈다
            let r = ctx.api.delete_node_token().await?;
            ctx.out.done(&t!("bot.unpaired"), &r);
            Ok(())
        }
        BotCmd::Token { token } => {
            let token = match token {
                Some(t) => t,
                None => ctx
                    .ask
                    .secret(&t!("bot.ask_token"))?
                    .ok_or_else(|| usage(t!("bot.token_required")))?,
            };
            if token.trim().is_empty() {
                return Err(usage(t!("bot.token_required")));
            }
            let r = ctx.api.save_node_token(token.trim()).await?;
            ctx.out.done(&t!("bot.token_saved"), &r);
            ctx.out.hint("saba-chan-cli bot start");
            Ok(())
        }
        BotCmd::Prefix { value } => {
            let cfg = ctx.api.bot_config().await?;
            match value {
                None => {
                    let p = cfg.get("prefix").cloned().unwrap_or(Value::Null);
                    ctx.out
                        .data(&json!({ "prefix": p }), |_| println!("{}", output::show(&p)));
                    Ok(())
                }
                Some(v) => {
                    let r = ctx.api.save_bot_config(json!({ "prefix": v })).await?;
                    ctx.out.done(&t!("bot.prefix_set", prefix = v), &r);
                    Ok(())
                }
            }
        }
        BotCmd::Alias { action } => alias(ctx, action.unwrap_or(AliasCmd::List)).await,
        BotCmd::Nodes => nodes(ctx).await,
        BotCmd::Members { guild } => {
            let cfg = ctx.api.bot_config().await?;
            let r = ctx.api.node_members(&guild, relay_of(&cfg)).await?;
            ctx.out.data(&r, |r| print_list(r, "members"));
            Ok(())
        }
        BotCmd::Channels { guild, chatbot, no_chatbot, music, no_music, notify, no_notify } => {
            channels(ctx, &guild, ChannelChange { chatbot, no_chatbot, music, no_music, notify, no_notify }).await
        }
        BotCmd::Lang { guild, value } => {
            let patch = value.map(|v| json!({ "locale": v }));
            discord_settings(ctx, &guild, patch).await
        }
        BotCmd::Sleep { guild } => discord_settings(ctx, &guild, Some(json!({ "sleeping": true }))).await,
        BotCmd::Wake { guild } => discord_settings(ctx, &guild, Some(json!({ "sleeping": false }))).await,
        BotCmd::Music { guild, on, off, ui, queue_lines, refresh, normalize } => {
            let mut music = Map::new();
            if on || off {
                music.insert("enabled".into(), json!(on));
            }
            for (key, v) in [("ui", ui.map(Value::from)), ("queueLines", queue_lines.map(Value::from)), ("refreshInterval", refresh.map(Value::from)), ("normalize", normalize.map(Value::from))] {
                if let Some(v) = v {
                    music.insert(key.into(), v);
                }
            }
            let patch = (!music.is_empty()).then(|| json!({ "music": music }));
            discord_settings(ctx, &guild, patch).await
        }
        BotCmd::Invite => {
            let cfg = ctx.api.bot_config().await?;
            let r = ctx.api.bot_invite(relay_of(&cfg)).await?;
            ctx.out.data(&r, |r| {
                println!("{}", t!("bot.invite"));
                println!("  {}", console::style(s(r, "url")).cyan().underlined());
            });
            Ok(())
        }
        BotCmd::ServerAlias { server, names, clear } => server_alias(ctx, server.as_deref(), &names, clear).await,
        BotCmd::Link { id, decline } => link(ctx, id.as_deref(), decline).await,
        BotCmd::Allow { guild, servers, all } => allow(ctx, &guild, &servers, all).await,
        BotCmd::Word { module, command, words, clear } => word(ctx, module.as_deref(), command.as_deref(), &words, clear).await,
        BotCmd::Reset { keys } => {
            let q = if keys.is_empty() {
                t!("config.confirm_reset_all")
            } else {
                t!("config.confirm_reset", keys = keys.join(", "))
            };
            ctx.ask.ensure(&q)?;
            let r = ctx.api.config_reset("bot", &keys).await?;
            let what = if keys.is_empty() {
                t!("config.all")
            } else {
                keys.join(", ")
            };
            ctx.out.done(&t!("config.reset", key = what), &r);
            Ok(())
        }
        BotCmd::Config { pairs } => config(ctx, &pairs).await,
    }
}

fn relay_of(cfg: &Value) -> Option<&str> {
    cfg.pointer("/cloud/relayUrl")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

async fn status(ctx: &Ctx) -> Result<()> {
    let (agent, cfg) = tokio::join!(ctx.api.agent(), ctx.api.bot_config());
    let agent = agent?;
    let cfg = cfg.unwrap_or(Value::Null);
    if ctx.out.json {
        ctx.out.data(&json!({ "agent": agent, "config": cfg }), |_| {});
        return Ok(());
    }
    let mut rows = vec![(t!("bot.connection"), crate::cmd::status::discord_line(&agent))];
    rows.push((
        t!("bot.prefix"),
        output::show(cfg.get("prefix").unwrap_or(&Value::Null)),
    ));
    let relay = s(&agent, "relayUrl");
    if !relay.is_empty() {
        rows.push((t!("bot.relay"), relay.to_string()));
    }
    if let Some(at) = agent.get("lastHeartbeatAt").and_then(Value::as_u64) {
        // 밀리초로 올 수도 있다
        let secs = if at > 10_000_000_000 { at / 1000 } else { at };
        rows.push((t!("bot.last_heartbeat"), output::ago(secs)));
    }
    // 데몬이 코드를 주면 현지화한 문구로
    let err = match s(&agent, "lastErrorCode") {
        "token_replaced" => t!("bot.token_replaced"),
        _ => s(&agent, "lastError").to_string(),
    };
    if !err.is_empty() {
        rows.push((t!("bot.last_error"), console::style(err).red().to_string()));
    }
    let n = cfg
        .get("moduleAliases")
        .and_then(Value::as_object)
        .map_or(0, |m| m.len());
    if n > 0 {
        rows.push((t!("bot.aliases"), n.to_string()));
    }
    output::print_kv(&rows);
    Ok(())
}

async fn pair(ctx: &Ctx, relay_url: Option<String>) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    // 데몬은 설정한 릴레이와만 페어링한다: 다른 릴레이를 주면 설정부터 바꾼다
    // (나중에 바꾸면 노드 토큰은 예전 릴레이 것이 된다)
    let previous = cfg.get("cloud").cloned().unwrap_or_else(|| json!({}));
    let mut cloud = previous.clone();
    let switched = relay_url.as_deref().is_some_and(|u| relay_of(&cfg) != Some(u));
    if switched {
        cloud["relayUrl"] = Value::String(relay_url.clone().unwrap_or_default());
        ctx.api.save_bot_config(json!({ "cloud": cloud.clone() })).await?;
    }
    let relay = relay_url.or_else(|| relay_of(&cfg).map(str::to_string));
    let result = pair_with(ctx, relay, cloud).await;
    // 페어링이 끝나지 않았으면 예전 릴레이로 되돌린다 (그대로 두면 다음 재시작에 기존 토큰을 새 주소로 보낸다)
    if result.is_err() && switched {
        let _ = ctx.api.save_bot_config(json!({ "cloud": previous })).await;
    }
    result
}

async fn pair_with(ctx: &Ctx, relay: Option<String>, mut cloud: Value) -> Result<()> {
    let init = ctx.api.pair_initiate(relay.as_deref()).await?;
    let code = s(&init, "code").to_string();
    let secret = s(&init, "pollSecret").to_string();
    if code.is_empty() {
        return Err(failed(t!("bot.pair_no_code")));
    }
    if ctx.out.json && !ctx.ask.yes {
        // 스크립트: 코드만 주고 끝 (폴링은 `--yes`로 기다린다)
        ctx.out.data(&init, |_| {});
        return Ok(());
    }
    eprintln!();
    eprintln!("  {}", t!("bot.pair_enter"));
    eprintln!();
    eprintln!("      {}", console::style(&code).bold().cyan());
    eprintln!();
    eprintln!("  {}", output::dim(&t!("bot.pair_waiting")));
    let spin = crate::progress::spinner(&t!("bot.pair_polling"));
    let result = loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = tokio::signal::ctrl_c() => {
                spin.finish_and_clear();
                return Err(crate::error::CliError::Cancelled.into());
            }
        }
        let Ok(st) = ctx.api.pair_status(&code, &secret, relay.as_deref()).await else {
            continue;
        };
        match s(&st, "status") {
            "claimed" => break st,
            "expired" => {
                spin.finish_and_clear();
                return Err(failed(t!("bot.pair_expired")));
            }
            _ => {}
        }
    };
    spin.finish_and_clear();
    // 데몬은 노드 토큰만 저장한다. 호스트 id는 봇 설정에 넣는다.
    if !s(&result, "hostId").is_empty() {
        cloud["hostId"] = Value::String(s(&result, "hostId").to_string());
        ctx.api.save_bot_config(json!({ "cloud": cloud })).await?;
    }
    let mut out = result.clone();
    if let Some(o) = out.as_object_mut() {
        o.remove("nodeToken");
    }
    ctx.out.done(&t!("bot.paired"), &out);
    ctx.out.hint(&t!("bot.pair_gui_hint"));
    let agent = ctx.api.agent().await.unwrap_or(Value::Null);
    if agent.get("running").and_then(Value::as_bool) != Some(true)
        && ctx.ask.confirm(&t!("bot.offer_start"), true).unwrap_or(false)
    {
        ctx.api.agent_start().await?;
        if !ctx.out.json {
            println!("{} {}", console::style("✓").green().bold(), t!("bot.started"));
        }
    }
    Ok(())
}

async fn alias(ctx: &Ctx, action: AliasCmd) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let mut map: Map<String, Value> = cfg
        .get("moduleAliases")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    match action {
        AliasCmd::List => {
            let v = Value::Object(map.clone());
            ctx.out.data(&v, |_| {
                if map.is_empty() {
                    println!("{}", output::dim(&t!("bot.no_aliases")));
                    return;
                }
                let mut t = output::table(&[t!("bot.alias"), t!("noun.module")]);
                for (k, v) in &map {
                    t.add_row(vec![k.clone(), output::show(v)]);
                }
                output::print_table(t);
            });
            return Ok(());
        }
        AliasCmd::Add { alias, module } => {
            let m = crate::resolve::module(ctx, Some(&module)).await?;
            let name = s(&m, "name").to_string();
            map.insert(alias.clone(), Value::String(name.clone()));
            let r = ctx.api.save_bot_config(json!({ "moduleAliases": map })).await?;
            ctx.out.done(&t!("bot.alias_added", alias = alias, module = name), &r);
        }
        AliasCmd::Remove { alias } => {
            if map.remove(&alias).is_none() {
                return Err(crate::error::CliError::NotFound {
                    what: t!("bot.alias"),
                    query: alias,
                    suggestions: map.keys().take(3).cloned().collect(),
                }
                .into());
            }
            let r = ctx.api.save_bot_config(json!({ "moduleAliases": map })).await?;
            ctx.out.done(&t!("bot.alias_removed", alias = alias), &r);
        }
    }
    Ok(())
}

/// `instanceAliases`: 서버 id → "별명1, 별명2" (같은 게임 서버가 여럿일 때 채팅에서 고르는 이름)
async fn server_alias(ctx: &Ctx, query: Option<&str>, names: &[String], clear: bool) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let mut map: Map<String, Value> = cfg
        .get("instanceAliases")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let Some(q) = query else {
        // 목록: id 대신 서버 이름으로
        let servers = crate::resolve::servers(ctx).await.unwrap_or_default();
        let rows: Vec<(String, String)> = map
            .iter()
            .map(|(id, v)| {
                let name = servers
                    .iter()
                    .find(|s| &s.id == id)
                    .map_or(id.clone(), |s| s.name.clone());
                (name, output::show(v))
            })
            .collect();
        ctx.out.data(&Value::Object(map.clone()), |_| {
            if rows.is_empty() {
                println!("{}", output::dim(&t!("bot.no_server_aliases")));
            } else {
                output::print_kv(&rows);
            }
        });
        return Ok(());
    };
    let sv = crate::resolve::server(ctx, Some(q)).await?;
    if names.is_empty() && !clear {
        let v = map.get(&sv.id).cloned().unwrap_or(Value::Null);
        ctx.out.data(&json!({ "server": sv.name, "aliases": v }), |_| {
            println!("{}", output::show(&v))
        });
        return Ok(());
    }
    if clear {
        map.remove(&sv.id);
    } else {
        let joined = names
            .iter()
            .flat_map(|n| n.split(','))
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        map.insert(sv.id.clone(), Value::String(joined));
    }
    let r = ctx.api.save_bot_config(json!({ "instanceAliases": map })).await?;
    let msg = if clear {
        t!("bot.server_alias_cleared", name = sv.name)
    } else {
        t!("bot.server_alias_set", name = sv.name, aliases = names.join(", "))
    };
    ctx.out.done(&msg, &r);
    Ok(())
}

/// `nodeSettings[guild].allowedInstances`: 그 디스코드 서버가 보고 부를 수 있는 서버 (키가 없으면 전부)
async fn allow(ctx: &Ctx, guild: &str, queries: &[String], all: bool) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let mut nodes: Map<String, Value> = cfg.get("nodeSettings").and_then(Value::as_object).cloned().unwrap_or_default();
    let servers = crate::resolve::servers(ctx).await.unwrap_or_default();
    let name_of = |id: &str| servers.iter().find(|s| s.id == id).map_or(id.to_string(), |s| s.name.clone());
    if queries.is_empty() && !all {
        let list = nodes.get(guild).and_then(|n| n.get("allowedInstances")).and_then(Value::as_array).cloned();
        ctx.out.data(&json!({ "guild": guild, "allowedInstances": list }), |_| match &list {
            None => println!("{}", t!("bot.allow_all")),
            Some(l) if l.is_empty() => println!("{}", output::dim(&t!("bot.allow_none"))),
            Some(l) => l.iter().filter_map(Value::as_str).for_each(|id| println!("{}", name_of(id))),
        });
        return Ok(());
    }
    let node = nodes.entry(guild.to_string()).or_insert_with(|| json!({}));
    let Some(node) = node.as_object_mut() else { return Err(usage(t!("bot.allow_bad_node", guild = guild))) };
    let msg = if all {
        node.remove("allowedInstances");
        t!("bot.allow_all_saved", guild = guild)
    } else {
        let mut ids = Vec::new();
        for q in queries.iter().flat_map(|q| q.split(',')).map(str::trim).filter(|q| !q.is_empty()) {
            let id = crate::resolve::server(ctx, Some(q)).await?.id;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let names: Vec<String> = ids.iter().map(|id| name_of(id)).collect();
        node.insert("allowedInstances".into(), json!(ids));
        t!("bot.allow_saved", guild = guild, servers = names.join(", "))
    };
    let r = ctx.api.save_bot_config(json!({ "nodeSettings": nodes })).await?;
    ctx.out.done(&msg, &r);
    Ok(())
}

/// `commandAliases`: 모듈 → (명령 → "말1, 말2"). 채팅에서 명령 대신 쓰는 말
async fn word(ctx: &Ctx, module: Option<&str>, command: Option<&str>, words: &[String], clear: bool) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let mut map: Map<String, Value> = cfg.get("commandAliases").and_then(Value::as_object).cloned().unwrap_or_default();
    let (Some(module), Some(command)) = (module, command) else {
        let rows: Vec<(String, String)> = map
            .iter()
            .filter(|(m, _)| module.is_none_or(|q| q == m.as_str()))
            .flat_map(|(m, cmds)| cmds.as_object().into_iter().flatten().map(move |(c, w)| (format!("{} {}", m, c), output::show(w))))
            .collect();
        ctx.out.data(&Value::Object(map.clone()), |_| {
            if rows.is_empty() {
                println!("{}", output::dim(&t!("bot.no_words")));
            } else {
                output::print_kv(&rows);
            }
        });
        return Ok(());
    };
    if words.is_empty() && !clear {
        let v = map.get(module).and_then(|m| m.get(command)).cloned().unwrap_or(Value::Null);
        ctx.out.data(&json!({ "module": module, "command": command, "words": v }), |_| println!("{}", output::show(&v)));
        return Ok(());
    }
    let entry = map.entry(module.to_string()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    let cmds = entry.as_object_mut().expect("object");
    let joined = words.iter().flat_map(|w| w.split(',')).map(str::trim).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(", ");
    if clear || joined.is_empty() {
        cmds.remove(command);
    } else {
        cmds.insert(command.to_string(), Value::String(joined.clone()));
    }
    if cmds.is_empty() {
        map.remove(module);
    }
    let r = ctx.api.save_bot_config(json!({ "commandAliases": map })).await?;
    let msg = if clear || joined.is_empty() {
        t!("bot.words_cleared", module = module, command = command)
    } else {
        t!("bot.words_set", module = module, command = command, words = joined)
    };
    ctx.out.done(&msg, &r);
    Ok(())
}

struct ChannelChange {

    chatbot: Vec<String>,
    no_chatbot: bool,
    music: Option<String>,
    no_music: bool,
    notify: Option<String>,
    no_notify: bool,
}

/// 이름(앞의 `#` 무시) 또는 id → 채널 id
fn channel_id(list: &[Value], query: &str) -> Result<String> {
    let q = query.trim().trim_start_matches('#');
    if let Some(c) = list.iter().find(|c| s(c, "id") == q) {
        return Ok(s(c, "id").to_string());
    }
    let named: Vec<&Value> = list.iter().filter(|c| s(c, "name").eq_ignore_ascii_case(q)).collect();
    match named.as_slice() {
        [one] => Ok(s(one, "id").to_string()),
        [] => Err(crate::error::CliError::NotFound {
            what: t!("bot.channel"),
            query: query.to_string(),
            suggestions: list
                .iter()
                .map(|c| s(c, "name").to_string())
                .filter(|n| n.to_lowercase().contains(&q.to_lowercase()))
                .take(3)
                .collect(),
        }
        .into()),
        _ => Err(usage(t!("bot.channel_ambiguous", name = q))),
    }
}

async fn channels(ctx: &Ctx, guild: &str, change: ChannelChange) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let relay = relay_of(&cfg);
    let current = ctx.api.node_channels(guild, relay).await?;
    let list = current.get("channels").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut patch = Map::new();
    if change.no_chatbot {
        patch.insert("chatbot".into(), json!([]));
    } else if !change.chatbot.is_empty() {
        let ids = change.chatbot.iter().map(|q| channel_id(&list, q)).collect::<Result<Vec<_>>>()?;
        patch.insert("chatbot".into(), json!(ids));
    }
    if change.no_music {
        patch.insert("music".into(), Value::Null);
    } else if let Some(q) = &change.music {
        let id = channel_id(&list, q)?;
        if current.get("music").and_then(Value::as_str) != Some(id.as_str()) {
            let name = list.iter().find(|c| s(c, "id") == id).map_or(id.clone(), |c| s(c, "name").to_string());
            ctx.ask.ensure(&t!("bot.confirm_music_channel", name = name))?;
        }
        patch.insert("music".into(), json!(id));
    }
    if change.no_notify {
        patch.insert("notify".into(), Value::Null);
    } else if let Some(q) = &change.notify {
        patch.insert("notify".into(), json!(channel_id(&list, q)?));
    }
    let r = if patch.is_empty() {
        current
    } else {
        let r = ctx.api.set_node_channels(guild, Value::Object(patch), relay).await?;
        if !ctx.out.json {
            println!("{} {}", console::style("✓").green().bold(), t!("bot.channels_saved"));
        }
        r
    };
    ctx.out.data(&r, print_channels);
    Ok(())
}

/// 길드 설정 보기·바꾸기 (I2). 원본은 데몬이고 데몬이 릴레이에 맞춘다
async fn discord_settings(ctx: &Ctx, guild: &str, patch: Option<Value>) -> Result<()> {
    let r = match patch {
        None => ctx.api.discord_settings(guild).await?,
        Some(p) => {
            let r = ctx.api.set_discord_settings(guild, p).await?;
            if !ctx.out.json {
                println!("{} {}", console::style("✓").green().bold(), t!("bot.discord_saved"));
                if r.pointer("/sync/ok").and_then(Value::as_bool) == Some(false) {
                    eprintln!("{} {}", console::style("!").yellow().bold(), t!("bot.discord_not_synced"));
                }
            }
            r.get("settings").cloned().unwrap_or(r)
        }
    };
    ctx.out.data(&r, print_discord_settings);
    Ok(())
}

fn print_discord_settings(r: &Value) {
    let on_off = |v: Option<bool>| if v == Some(true) { t!("bot.on") } else { t!("bot.off") };
    let locale = s(r, "locale");
    let music = |k: &str| r.get("music").and_then(|m| m.get(k));
    output::print_kv(&[
        (t!("bot.locale"), if locale == "auto" { t!("bot.locale_auto") } else { locale.to_string() }),
        (t!("bot.sleeping"), on_off(r.get("sleeping").and_then(Value::as_bool))),
        (t!("bot.music"), on_off(music("enabled").and_then(Value::as_bool))),
        (t!("bot.music_ui"), on_off(music("ui").and_then(Value::as_bool))),
        (t!("bot.music_queue_lines"), music("queueLines").map(|v| v.to_string()).unwrap_or_default()),
        (t!("bot.music_refresh"), music("refreshInterval").map(|v| format!("{} ms", v)).unwrap_or_default()),
        (t!("bot.music_normalize"), on_off(music("normalize").and_then(Value::as_bool))),
    ]);
}

fn print_channels(r: &Value) {
    let list = r.get("channels").and_then(Value::as_array).cloned().unwrap_or_default();
    let name = |id: &str| {
        list.iter()
            .find(|c| s(c, "id") == id)
            .map_or(id.to_string(), |c| format!("#{}", s(c, "name")))
    };
    let chatbot: Vec<String> = r
        .get("chatbot")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(name)
        .collect();
    let none = || output::dim(&t!("bot.channel_none"));
    let music = r.get("music").and_then(Value::as_str).map(name);
    let notify = r.get("notify").and_then(Value::as_str).map(name);
    output::print_kv(&[
        (t!("bot.chatbot_channels"), if chatbot.is_empty() { none() } else { chatbot.join(", ") }),
        (t!("bot.music_channel"), music.unwrap_or_else(none)),
        (t!("bot.notify_channel"), notify.unwrap_or_else(none)),
    ]);
    if r.get("botAvailable").and_then(Value::as_bool) == Some(false) {
        eprintln!("{} {}", console::style("!").yellow().bold(), t!("bot.channels_bot_missing"));
    }
}

/// 디스코드에서 코드 없이 `/사바쨩 연결`한 요청 (J1-b). 수락하면 릴레이가 그 길드를 이 PC에 묶는다
async fn link(ctx: &Ctx, id: Option<&str>, decline: bool) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let Some(id) = id else {
        let r = ctx.api.link_requests(relay_of(&cfg)).await?;
        ctx.out.data(&r, |r| print_list(r, "requests"));
        return Ok(());
    };
    let r = ctx.api.answer_link_request(id, !decline, relay_of(&cfg)).await?;
    if decline {
        ctx.out.done(&t!("bot.link_declined"), &r);
    } else {
        let guild = [s(&r, "guildName"), s(&r, "guildId"), id].into_iter().find(|g| !g.is_empty()).unwrap_or(id);
        ctx.out.done(&t!("bot.link_accepted", guild = guild), &r);
    }
    Ok(())
}

async fn nodes(ctx: &Ctx) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let host = cfg.pointer("/cloud/hostId").and_then(Value::as_str).unwrap_or("");
    if host.is_empty() {
        return Err(failed(t!("bot.not_paired")));
    }
    let r = ctx.api.host_nodes(host, relay_of(&cfg)).await?;
    ctx.out.data(&r, |r| print_list(r, "nodes"));
    Ok(())
}

/// 릴레이 응답 목록을 표로 (모양을 모르니 첫 항목의 단순 값 칸만)
fn print_list(r: &Value, key: &str) {
    let list = r
        .get(key)
        .or_else(|| r.get("data"))
        .or(Some(r))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        println!("{}", output::dim(&t!("bot.list_empty")));
        return;
    }
    crate::cmd::server::print_value(&Value::Array(list));
}

async fn config(ctx: &Ctx, pairs: &[String]) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    if pairs.is_empty() {
        ctx.out.data(&cfg, |c| {
            let mut rows = Vec::new();
            flatten(c, "", &mut rows);
            output::print_kv(
                &rows
                    .iter()
                    .map(|(k, v)| (k.clone(), output::mask(k, v)))
                    .collect::<Vec<_>>(),
            );
        });
        return Ok(());
    }
    // 최상위 키 단위로 머지되므로, 바뀐 최상위 키는 현재 값 전체에 덮어써서 보낸다
    let mut full = cfg.clone();
    let mut touched = Vec::new();
    for (k, raw) in split_pairs(pairs)? {
        let v = if raw.is_empty() { Value::Null } else { guess(&raw) };
        if get_path(&cfg, &k).is_none() && !v.is_null() {
            ctx.out.warn(&t!("set.unknown_key", key = k));
        }
        set_path(&mut full, &k, v);
        let top = k.split('.').next().unwrap_or("").to_string();
        if !touched.contains(&top) {
            touched.push(top);
        }
    }
    let patch: Map<String, Value> = touched
        .iter()
        .map(|k| (k.clone(), full.get(k).cloned().unwrap_or(Value::Null)))
        .collect();
    let r = ctx.api.save_bot_config(Value::Object(patch)).await?;
    ctx.out.done(&t!("bot.config_saved"), &r);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_found_by_id_or_name_and_a_shared_name_must_use_the_id() {
        let list = vec![
            json!({ "id": "1", "name": "잡담" }),
            json!({ "id": "2", "name": "music" }),
            json!({ "id": "3", "name": "Music" }),
        ];
        assert_eq!(channel_id(&list, "1").unwrap(), "1");
        assert_eq!(channel_id(&list, "#잡담").unwrap(), "1");
        assert!(channel_id(&list, "music").is_err());
        assert_eq!(channel_id(&list, "3").unwrap(), "3");
        assert!(channel_id(&list, "공지").is_err());
    }
}
