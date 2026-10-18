//! `bot` — 디스코드 봇 연결 (클라우드 릴레이)
//!
//! 봇은 클라우드에서만 돈다. 이 PC는 릴레이 에이전트로 붙는다:
//! 페어링 → 노드 토큰 저장(데몬이 함) → 에이전트 시작.
//! 봇 설정(`/api/config/bot`)은 최상위 키 단위로 머지되므로, 맵(`nodeSettings`)은 통째로 보낸다.

use crate::cli::BotCmd;
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
        BotCmd::Prefix { values, default } => {
            if values.is_empty() && !default {
                let cfg = ctx.api.bot_config().await?;
                let p = cfg.get("prefixes").cloned().unwrap_or_else(|| json!([]));
                ctx.out.data(&json!({ "prefixes": p }), |_| println!("{}", prefixes_line(&p)));
                return Ok(());
            }
            let r = ctx.api.save_bot_config(json!({ "prefixes": values })).await?;
            let msg = if default { t!("bot.prefix_reset") } else { t!("bot.prefix_set", prefix = values.join(", ")) };
            ctx.out.done(&msg, &r);
            Ok(())
        }
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
        BotCmd::Link { id, decline } => link(ctx, id.as_deref(), decline).await,
        BotCmd::Allow { guild, servers, all } => allow(ctx, &guild, &servers, all).await,
        BotCmd::Words { guild, server, word, command, clear } => words(ctx, &guild, server.as_deref(), WordsChange { words: word, commands: command, clear }).await,
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

/// 빈 목록은 앱 언어의 기본 접두사(릴레이가 고른다)
fn prefixes_line(p: &Value) -> String {
    let list: Vec<&str> = p.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if list.is_empty() {
        t!("bot.prefix_default")
    } else {
        list.join(", ")
    }
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
    rows.push((t!("bot.prefix"), prefixes_line(cfg.get("prefixes").unwrap_or(&Value::Null))));
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
    let claimed = wait_for_claim(ctx, relay.as_deref()).await;
    // 페어링이 끝나지 않았으면 예전 릴레이로 되돌린다 (그대로 두면 다음 재시작에 기존 토큰을 새 주소로 보낸다).
    // 끝난 뒤의 오류는 되돌리지 않는다: 데몬은 이미 새 릴레이의 토큰을 저장했다.
    // `--json`으로 코드만 낸 경우도 그대로 둔다: 이어서 기다리는 쪽이 새 릴레이로 claim을 받는다
    if switched && claimed.is_err() {
        let _ = ctx.api.save_bot_config(json!({ "cloud": previous })).await;
    }
    match claimed? {
        Some(result) => finish_pair(ctx, result, cloud).await,
        None => Ok(()),
    }
}

/// 페어링 코드를 받아 보여 주고 claim될 때까지 기다린다. `--json`만 주면 코드만 내고 `None`
async fn wait_for_claim(ctx: &Ctx, relay: Option<&str>) -> Result<Option<Value>> {
    let init = ctx.api.pair_initiate(relay).await?;
    let code = s(&init, "code").to_string();
    let secret = s(&init, "pollSecret").to_string();
    if code.is_empty() {
        return Err(failed(t!("bot.pair_no_code")));
    }
    if ctx.out.json && !ctx.ask.yes {
        // 스크립트: 코드만 주고 끝 (폴링은 `--yes`로 기다린다)
        ctx.out.data(&init, |_| {});
        return Ok(None);
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
        let Ok(st) = ctx.api.pair_status(&code, &secret, relay).await else {
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
    Ok(Some(result))
}

async fn finish_pair(ctx: &Ctx, result: Value, mut cloud: Value) -> Result<()> {
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

/// `nodeSettings[guild].allowedInstances`: 그 디스코드 서버가 보고 부를 수 있는 서버 (키가 없으면 전부)
async fn allow(ctx: &Ctx, guild: &str, queries: &[String], all: bool) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let nodes: Map<String, Value> = cfg.get("nodeSettings").and_then(Value::as_object).cloned().unwrap_or_default();
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
    // bot-config 일괄 저장은 이 키를 바꾸지 않는다: 길드 단위 API로 저장한다
    let (msg, r) = if all {
        (t!("bot.allow_all_saved", guild = guild), ctx.api.save_allowed_instances(guild, None).await?)
    } else {
        let mut ids = Vec::new();
        for q in queries.iter().flat_map(|q| q.split(',')).map(str::trim).filter(|q| !q.is_empty()) {
            let id = crate::resolve::server(ctx, Some(q)).await?.id;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let names: Vec<String> = ids.iter().map(|id| name_of(id)).collect();
        let r = ctx.api.save_allowed_instances(guild, Some(&ids)).await?;
        (t!("bot.allow_saved", guild = guild, servers = names.join(", ")), r)
    };
    ctx.out.done(&msg, &r);
    Ok(())
}

struct WordsChange {
    /// `Some`: 서버 낱말을 이것으로 바꾼다 (빈 값만 있으면 지운다)
    words: Option<Vec<String>>,
    /// `명령=낱말,낱말`: 그 명령만 바꾼다 (`명령=`은 지운다)
    commands: Vec<String>,
    clear: bool,
}

impl WordsChange {
    fn is_empty(&self) -> bool {
        self.words.is_none() && self.commands.is_empty() && !self.clear
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

fn clean_words<'a>(raw: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    raw.into_iter().map(str::trim).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// PUT 본문 `{words, commands}`. 지금 값에서 시작해 준 것만 바꾼다. 낱말 규칙·충돌은 데몬이 검사한다
fn words_body(entry: &Value, change: &WordsChange) -> Result<Value> {
    if change.clear {
        return Ok(json!({ "words": [], "commands": {} }));
    }
    let words = match &change.words {
        Some(w) => clean_words(w.iter().map(String::as_str)),
        None => str_list(entry.get("words")),
    };
    let known: Vec<Value> = entry.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut commands = Map::new();
    for c in &known {
        let aliases = str_list(c.get("aliases"));
        if !aliases.is_empty() {
            commands.insert(s(c, "name").to_string(), json!(aliases));
        }
    }
    for spec in &change.commands {
        let (name, list) = spec.split_once('=').ok_or_else(|| usage(t!("bot.words_bad_command", spec = spec)))?;
        let name = name.trim();
        let canonical = match known.iter().find(|c| s(c, "name").eq_ignore_ascii_case(name)) {
            Some(c) => s(c, "name").to_string(),
            None => {
                let suggestions = match crate::resolve::find(&known, name, |c| vec![s(c, "name").to_string()], |c| s(c, "name").to_string()) {
                    crate::resolve::Found::None(close) => close,
                    crate::resolve::Found::One(c) => vec![s(&c, "name").to_string()],
                    crate::resolve::Found::Many(list) => list.iter().take(3).map(|c| s(c, "name").to_string()).collect(),
                };
                return Err(crate::error::CliError::NotFound { what: t!("noun.command"), query: name.to_string(), suggestions }.into());
            }
        };
        let list = clean_words(list.split(','));
        if list.is_empty() {
            commands.remove(&canonical);
        } else {
            commands.insert(canonical, json!(list));
        }
    }
    Ok(json!({ "words": words, "commands": commands }))
}

/// 목록은 배열 그대로이거나 `{instances: [...]}`
fn instance_list(r: &Value) -> Vec<Value> {
    r.get("instances").or(Some(r)).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `명령: 낱말, 낱말` — 사용자 낱말만 (module.toml 별명은 기본값이라 뺀다)
fn command_words_line(entry: &Value) -> String {
    entry
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let a = str_list(c.get("aliases"));
            (!a.is_empty()).then(|| format!("{}: {}", s(c, "name"), a.join(", ")))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// `nodeSettings[guild].instances[id]`: 그 디스코드 서버에서 이 PC의 서버와 명령을 부르는 낱말 (`/api/guilds/:guild/instances`)
async fn words(ctx: &Ctx, guild: &str, query: Option<&str>, change: WordsChange) -> Result<()> {
    let list = instance_list(&ctx.api.guild_instances(guild).await?);
    let Some(q) = query else {
        ctx.out.data(&Value::Array(list.clone()), |_| {
            if list.is_empty() {
                println!("{}", output::dim(&t!("bot.list_empty")));
                return;
            }
            let mut t = output::table(&[t!("noun.server"), t!("bot.words"), t!("bot.command_words")]);
            for i in &list {
                t.add_row(vec![s(i, "name").to_string(), str_list(i.get("words")).join(", "), command_words_line(i)]);
            }
            output::print_table(t);
        });
        return Ok(());
    };
    let what = t!("noun.server");
    let found = crate::resolve::find(&list, q, |i| vec![s(i, "name").to_string(), s(i, "id").to_string()], |i| s(i, "name").to_string());
    let entry = crate::resolve::settle(ctx, &what, q, found, |i| s(i, "name").to_string())?;
    if change.is_empty() {
        ctx.out.data(&entry, |e| {
            let mut rows = vec![(t!("bot.words"), str_list(e.get("words")).join(", "))];
            for c in e.get("commands").and_then(Value::as_array).into_iter().flatten() {
                let mut line = str_list(c.get("aliases")).join(", ");
                let defaults = str_list(c.get("moduleAliases"));
                if !defaults.is_empty() {
                    line = format!("{} {}", line, output::dim(&format!("({})", defaults.join(", ")))).trim().to_string();
                }
                rows.push((s(c, "name").to_string(), line));
            }
            output::print_kv(&rows);
        });
        return Ok(());
    }
    let body = words_body(&entry, &change)?;
    let r = ctx.api.save_instance_words(guild, s(&entry, "id"), body).await?;
    ctx.out.done(&t!("bot.words_saved", server = s(&entry, "name")), &r);
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
            // JSON에는 동기화 결과(`sync`)도 그대로 싣는다
            if ctx.out.json {
                r
            } else {
                r.get("settings").cloned().unwrap_or(r)
            }
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

    fn entry() -> Value {
        json!({
            "id": "i1", "name": "크리에이티브", "module": "minecraft", "words": ["크리"],
            "commands": [
                { "name": "start", "builtin": true, "moduleAliases": [], "aliases": ["켜"] },
                { "name": "stop", "builtin": true, "moduleAliases": [], "aliases": ["꺼"] },
                { "name": "kick", "builtin": false, "moduleAliases": ["추방"], "aliases": [] }
            ]
        })
    }

    fn change(words: Option<&[&str]>, commands: &[&str], clear: bool) -> WordsChange {
        WordsChange {
            words: words.map(|w| w.iter().map(|s| s.to_string()).collect()),
            commands: commands.iter().map(|s| s.to_string()).collect(),
            clear,
        }
    }

    #[test]
    fn a_command_change_keeps_the_other_words() {
        let body = words_body(&entry(), &change(None, &["KICK=내보내, 쫓아내", "stop="], false)).unwrap();
        assert_eq!(body, json!({ "words": ["크리"], "commands": { "start": ["켜"], "kick": ["내보내", "쫓아내"] } }));
    }

    #[test]
    fn server_words_are_replaced_and_an_empty_word_clears_them() {
        let body = words_body(&entry(), &change(Some(&["크", " 크2 "]), &[], false)).unwrap();
        assert_eq!(body["words"], json!(["크", "크2"]));
        assert_eq!(body["commands"], json!({ "start": ["켜"], "stop": ["꺼"] }));
        assert_eq!(words_body(&entry(), &change(Some(&[""]), &[], false)).unwrap()["words"], json!([]));
    }

    #[test]
    fn clear_empties_everything() {
        assert_eq!(words_body(&entry(), &change(None, &[], true)).unwrap(), json!({ "words": [], "commands": {} }));
    }

    #[test]
    fn an_unknown_command_or_a_spec_without_equals_is_refused() {
        let e = words_body(&entry(), &change(None, &["kik=추방"], false)).unwrap_err();
        match e.downcast_ref::<crate::error::CliError>() {
            Some(crate::error::CliError::NotFound { suggestions, .. }) => assert_eq!(suggestions, &vec!["kick".to_string()]),
            other => panic!("expected NotFound, got {:?}", other),
        }
        assert!(words_body(&entry(), &change(None, &["kick"], false)).is_err());
    }

    #[test]
    fn the_list_comes_bare_or_wrapped() {
        assert_eq!(instance_list(&json!([entry()])).len(), 1);
        assert_eq!(instance_list(&json!({ "instances": [entry()] })).len(), 1);
    }
}
