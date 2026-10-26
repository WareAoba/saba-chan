//! `bot` — 디스코드 봇 연결 (클라우드 릴레이)
//!
//! 봇은 클라우드에서만 돈다. 이 PC는 릴레이 에이전트로 붙는다:
//! 페어링 → 노드 토큰 저장(데몬이 함) → 에이전트 시작.
//! 봇 설정(`/api/config/bot`)은 최상위 키 단위로 머지되므로, 맵(`nodeSettings`)은 통째로 보낸다.

use crate::cli::BotCmd;
use crate::cmd::{coerce, config_patch, flatten, get_path, split_pairs};
use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use crate::output::{self, s};
use crate::resolve;
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
        BotCmd::Erase => erase(ctx).await,
        BotCmd::Token => {
            // 셸 기록에 남지 않게 인자로 받지 않는다 (터미널이 아니면 표준 입력 한 줄)
            let token = ctx.ask.secret_or_stdin(&t!("bot.ask_token"))?;
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
                ctx.out.data(&json!({ "prefixes": p }), |_| say!("{}", prefixes_line(&p)));
                return Ok(());
            }
            let r = ctx.api.save_bot_config(json!({ "prefixes": values })).await?;
            let msg = if default { t!("bot.prefix_reset") } else { t!("bot.prefix_set", prefix = values.join(", ")) };
            ctx.out.done(&msg, &r);
            Ok(())
        }
        BotCmd::Nodes => nodes(ctx).await,
        BotCmd::Members { guild } => {
            let guild = resolve::guild(ctx, guild.as_deref()).await?;
            let r = members(ctx, &guild).await?;
            ctx.out.data(&r, |r| {
                let rows = member_rows(r);
                if rows.is_empty() {
                    say!("{}", output::dim(&t!("bot.list_empty")));
                } else {
                    let mut t = output::table(&[t!("col.name"), t!("bot.username"), t!("col.id")]);
                    for row in rows {
                        t.add_row(row);
                    }
                    output::print_table(t);
                }
                if r.get("partial").and_then(Value::as_bool) == Some(true) {
                    ctx.out.warn(&t!("bot.members_partial"));
                }
            });
            Ok(())
        }
        BotCmd::Channels { guild, chatbot, no_chatbot, music, no_music, notify, no_notify } => {
            let change = ChannelChange { chatbot, no_chatbot, music, no_music, notify, no_notify };
            match guild {
                Some(guild) => channels(ctx, &resolve::guild(ctx, Some(&guild)).await?, change).await,
                None => channels_of_paired(ctx, change).await,
            }
        }
        BotCmd::Lang { guild, value } => {
            let patch = value.map(|v| json!({ "locale": v }));
            discord_settings(ctx, &resolve::guild(ctx, Some(&guild)).await?, patch).await
        }
        BotCmd::Sleep { guild } => discord_settings(ctx, &resolve::guild(ctx, guild.as_deref()).await?, Some(json!({ "sleeping": true }))).await,
        BotCmd::Wake { guild } => discord_settings(ctx, &resolve::guild(ctx, guild.as_deref()).await?, Some(json!({ "sleeping": false }))).await,
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
            discord_settings(ctx, &resolve::guild(ctx, guild.as_deref()).await?, patch).await
        }
        BotCmd::Invite => {
            let cfg = ctx.api.bot_config().await?;
            let r = ctx.api.bot_invite(relay_of(&cfg)).await?;
            ctx.out.data(&r, |r| {
                say!("{}", t!("bot.invite"));
                say!("  {}", console::style(s(r, "url")).cyan().underlined());
            });
            Ok(())
        }
        BotCmd::Link { id, decline } => link(ctx, id.as_deref(), decline).await,
        BotCmd::Allow { guild, servers, all } => allow(ctx, &resolve::guild(ctx, Some(&guild)).await?, &servers, all).await,
        BotCmd::Notice { guild, server, on, off } => notice(ctx, &resolve::guild(ctx, Some(&guild)).await?, server.as_deref(), (on || off).then_some(on)).await,
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

pub(crate) fn relay_of(cfg: &Value) -> Option<&str> {
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
    // 릴레이가 이 PC를 아는지도 본다 (GUI와 같다): 페어링했을 때만
    let host = cfg.pointer("/cloud/hostId").and_then(Value::as_str).unwrap_or("");
    let relay_check = match host {
        "" => None,
        host => Some(ctx.api.host_status(host, relay_of(&cfg)).await.map_err(|e| e.to_string())),
    };
    if ctx.out.json {
        let relay = relay_check.as_ref().map(|r| match r {
            Ok(v) => json!({ "ok": v.get("error").is_none(), "status": v }),
            Err(e) => json!({ "ok": false, "error": e }),
        });
        ctx.out.data(&json!({ "agent": agent, "config": cfg, "relay": relay }), |_| {});
        return Ok(());
    }
    let mut rows = vec![(t!("bot.connection"), crate::cmd::status::discord_line(&agent))];
    if let Some(r) = &relay_check {
        rows.push((t!("bot.relay_check"), relay_line(r)));
    }
    rows.push((t!("bot.prefix"), prefixes_line(cfg.get("prefixes").unwrap_or(&Value::Null))));
    // 끄기: saba-chan-cli bot config translateCommands=false (GUI 디스코드 > 봇 설정과 같은 값)
    let translate = cfg.get("translateCommands").and_then(Value::as_bool) != Some(false);
    rows.push((t!("bot.translate_commands"), if translate { t!("bot.on") } else { t!("bot.off") }));
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
        // 연결 줄이 이미 업데이트를 안내한다
        "agent_too_old" => String::new(),
        _ => s(&agent, "lastError").to_string(),
    };
    if !err.is_empty() {
        rows.push((t!("bot.last_error"), console::style(err).red().to_string()));
    }
    output::print_kv(&rows);
    Ok(())
}

/// 릴레이 호스트 확인 결과 한 줄: 응답함, 아니면 오류
fn relay_line(r: &std::result::Result<Value, String>) -> String {
    let err = match r {
        Ok(v) => v.get("error").map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())),
        Err(e) => Some(e.clone()),
    };
    match err {
        None => console::style(t!("bot.relay_ok")).green().to_string(),
        Some(e) => console::style(t!("bot.relay_failed", error = e)).red().to_string(),
    }
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
    // 데몬이 토큰을 저장하지 못했으면(수령 기한이 지났거나 쓰기 실패) 끝나지 않은 페어링이다 (R15)
    let claimed = wait_for_claim(ctx, relay.as_deref()).await.and_then(|r| match r {
        Some(r) if !token_saved(&r) => Err(failed(t!("bot.pair_token_not_saved"))),
        r => Ok(r),
    });
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

/// 개인정보 모두 지우기: 데몬이 받은 웹사이트 동의 페이지를 보여 주고, 동의해서 데몬이 연결을 끊을 때까지 기다린다.
/// 기다리기를 그만둬도(Ctrl+C) 데몬은 계속 묻고, 동의하면 연결을 끊는다
async fn erase(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.privacy_erase(crate::i18n::lang()).await?;
    let url = s(&r, "url").to_string();
    if url.is_empty() {
        return Err(failed(t!("bot.erase_no_url")));
    }
    if ctx.out.json && !ctx.ask.yes {
        ctx.out.data(&r, |_| {});
        return Ok(());
    }
    tell!();
    tell!("  {}", t!("bot.erase_open"));
    tell!();
    tell!("      {}", console::style(&url).cyan().underlined());
    tell!();
    tell!("  {}", output::dim(&t!("bot.erase_waiting")));
    let spin = crate::progress::spinner(&t!("bot.erase_polling"));
    let status = loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            _ = tokio::signal::ctrl_c() => {
                spin.finish_and_clear();
                return Err(crate::error::CliError::Cancelled.into());
            }
        }
        // 데몬에 잠깐 닿지 않아도 다시 묻는다 (기한은 데몬이 센다)
        let Ok(st) = ctx.api.privacy_erase_status().await else { continue };
        match s(&st, "status") {
            "pending" => {}
            other => break (other.to_string(), st),
        }
    };
    spin.finish_and_clear();
    match status.0.as_str() {
        "done" => {
            ctx.out.done(&t!("bot.erase_done"), &status.1);
            Ok(())
        }
        "failed" => Err(failed(t!("bot.erase_failed"))),
        _ => Err(failed(t!("bot.erase_expired"))),
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
    tell!();
    tell!("  {}", t!("bot.pair_enter"));
    tell!();
    tell!("      {}", console::style(&code).bold().cyan());
    tell!();
    tell!("  {}", output::dim(&t!("bot.pair_waiting")));
    let spin = crate::progress::spinner(&t!("bot.pair_polling"));
    let mut wait = PairWait::new(std::time::Instant::now());
    let result = loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = tokio::signal::ctrl_c() => {
                spin.finish_and_clear();
                return Err(crate::error::CliError::Cancelled.into());
            }
        }
        if wait.timed_out(std::time::Instant::now()) {
            spin.finish_and_clear();
            return Err(failed(t!("bot.pair_expired")));
        }
        let st = match ctx.api.pair_status(&code, &secret, relay).await {
            Ok(st) => {
                wait.ok();
                st
            }
            Err(e) if wait.failed() => {
                spin.finish_and_clear();
                return Err(e);
            }
            Err(_) => continue,
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

/// 이 PC에 노드 토큰이 저장됐는지: 데몬의 `tokenSaved`, 없으면(옛 데몬) 받은 `nodeToken`이 있는지
fn token_saved(result: &Value) -> bool {
    match result.get("tokenSaved").and_then(Value::as_bool) {
        Some(saved) => saved,
        None => !s(result, "nodeToken").is_empty(),
    }
}

/// 코드 유효기간(릴레이 5분)에 여유를 더한 시간. 릴레이가 `expired`를 알려 주지 못해도 여기서 멈춘다
const PAIR_WAIT_LIMIT: Duration = Duration::from_secs(6 * 60);
/// 상태 조회가 이만큼 연달아 실패하면 마지막 오류로 멈춘다 (2초 간격, 30초 남짓)
const PAIR_POLL_FAILURES: u32 = 15;

/// 페어링 대기를 끝낼 때를 센다 (R18)
struct PairWait {
    started: std::time::Instant,
    failures: u32,
}

impl PairWait {
    fn new(started: std::time::Instant) -> Self {
        Self { started, failures: 0 }
    }
    fn timed_out(&self, now: std::time::Instant) -> bool {
        now.duration_since(self.started) >= PAIR_WAIT_LIMIT
    }
    /// 실패를 하나 더 센다. 그만둘 때면 true
    fn failed(&mut self) -> bool {
        self.failures += 1;
        self.failures >= PAIR_POLL_FAILURES
    }
    fn ok(&mut self) {
        self.failures = 0;
    }
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
            say!("{} {}", console::style("✓").green().bold(), t!("bot.started"));
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
            None => say!("{}", t!("bot.allow_all")),
            Some(l) if l.is_empty() => say!("{}", output::dim(&t!("bot.allow_none"))),
            Some(l) => l.iter().filter_map(Value::as_str).for_each(|id| say!("{}", name_of(id))),
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

/// 켜짐 알림: 사바쨩이 켠 서버를 그 디스코드 서버의 알림 채널에 알린다 (`nodeSettings[guild].startNotice`, 기본 꺼짐)
async fn notice(ctx: &Ctx, guild: &str, server: Option<&str>, enabled: Option<bool>) -> Result<()> {
    let servers = crate::resolve::servers(ctx).await.unwrap_or_default();
    let name_of = |id: &str| servers.iter().find(|s| s.id == id).map_or(id.to_string(), |s| s.name.clone());
    let id = match server {
        Some(q) => Some(crate::resolve::server(ctx, Some(q)).await?.id),
        None => None,
    };
    let (Some(id), Some(enabled)) = (&id, enabled) else {
        let r = ctx.api.start_notice(guild).await?;
        let on: Vec<String> = r.get("instances").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        ctx.out.data(&r, |_| match &id {
            Some(id) if on.contains(id) => say!("{}", t!("bot.notice_is_on", server = name_of(id))),
            Some(id) => say!("{}", t!("bot.notice_is_off", server = name_of(id))),
            None if on.is_empty() => say!("{}", output::dim(&t!("bot.notice_none"))),
            None => on.iter().for_each(|id| say!("{}", name_of(id))),
        });
        return Ok(());
    };
    let r = ctx.api.set_start_notice(id, guild, enabled).await?;
    let key = if enabled { "bot.notice_on_saved" } else { "bot.notice_off_saved" };
    ctx.out.done(&t!(key, guild = guild, server = name_of(id)), &r);
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

impl ChannelChange {
    fn is_empty(&self) -> bool {
        self.chatbot.is_empty() && !self.no_chatbot && self.music.is_none() && !self.no_music && self.notify.is_none() && !self.no_notify
    }
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
            say!("{} {}", console::style("✓").green().bold(), t!("bot.channels_saved"));
        }
        r
    };
    ctx.out.data(&r, print_channels);
    Ok(())
}

/// 서버 id 없이: 페어링된 디스코드 서버마다 보여 준다. 바꾸기는 서버가 하나일 때만
async fn channels_of_paired(ctx: &Ctx, change: ChannelChange) -> Result<()> {
    let list = paired_nodes(ctx).await?;
    let targets = target_guilds(&list, !change.is_empty())?;
    if let [(guild, _)] = targets.as_slice() {
        return channels(ctx, guild, change).await;
    }
    let cfg = ctx.api.bot_config().await?;
    let mut all = Vec::new();
    // 서버 하나를 못 읽어도 나머지는 보여 준다
    for (guild, name) in &targets {
        let r = ctx.api.node_channels(guild, relay_of(&cfg)).await;
        if !ctx.out.json {
            say!();
            output::heading(&if name.is_empty() { guild.clone() } else { format!("{} ({})", name, guild) });
            match &r {
                Ok(r) => print_channels(r),
                Err(e) => ctx.out.warn(&e.to_string()),
            }
        }
        all.push(match r {
            Ok(r) => json!({ "guildId": guild, "guildName": name, "channels": r }),
            Err(e) => json!({ "guildId": guild, "guildName": name, "error": e.to_string() }),
        });
    }
    if ctx.out.json {
        ctx.out.data(&Value::Array(all), |_| {});
    }
    Ok(())
}

/// 페어링된 디스코드 서버 `(id, 이름)`. 바꾸기(`changing`)는 어느 서버인지 분명할 때(하나뿐)만
fn target_guilds(nodes: &Value, changing: bool) -> Result<Vec<(String, String)>> {
    let list = resolve::guild_items(nodes);
    if list.is_empty() {
        return Err(failed(t!("bot.no_paired_servers")));
    }
    if changing && list.len() > 1 {
        let ids = list.iter().map(|(id, name)| format!("{} ({})", id, name)).collect::<Vec<_>>().join(", ");
        return Err(usage(t!("bot.pick_server", servers = ids)));
    }
    Ok(list)
}

/// 길드 설정 보기·바꾸기 (I2). 원본은 데몬이고 데몬이 릴레이에 맞춘다
async fn discord_settings(ctx: &Ctx, guild: &str, patch: Option<Value>) -> Result<()> {
    let r = match patch {
        None => ctx.api.discord_settings(guild).await?,
        Some(p) => {
            let r = ctx.api.set_discord_settings(guild, p).await?;
            if !ctx.out.json {
                say!("{} {}", console::style("✓").green().bold(), t!("bot.discord_saved"));
                if r.pointer("/sync/ok").and_then(Value::as_bool) == Some(false) {
                    tell!("{} {}", console::style("!").yellow().bold(), t!("bot.discord_not_synced"));
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
        tell!("{} {}", console::style("!").yellow().bold(), t!("bot.channels_bot_missing"));
    }
}

/// 디스코드에서 코드 없이 `/사바쨩 연결`한 요청 (J1-b). 수락하면 릴레이가 그 길드를 이 PC에 묶는다
async fn link(ctx: &Ctx, id: Option<&str>, decline: bool) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let Some(id) = id else {
        let r = ctx.api.link_requests(relay_of(&cfg)).await?;
        ctx.out.data(&r, |r| {
            let rows = link_rows(r, now());
            if rows.is_empty() {
                say!("{}", output::dim(&t!("bot.list_empty")));
                return;
            }
            let mut t = output::table(&[t!("col.id"), t!("noun.guild"), t!("bot.requested_by"), t!("bot.expires_in")]);
            for row in rows {
                t.add_row(row);
            }
            output::print_table(t);
            ctx.out.hint("saba-chan-cli bot link ID  |  bot link ID --decline");
        });
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

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 연결 요청 표의 줄: 요청 id(수락할 때 쓴다), 디스코드 서버, 요청한 사람, 남은 시간
fn link_rows(r: &Value, now: u64) -> Vec<Vec<String>> {
    r.get("requests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|q| {
            let left = match output::iso_epoch(s(q, "expiresAt")) {
                Some(at) if at > now => output::duration(at - now),
                Some(_) => output::dim(&t!("bot.expired")),
                None => String::new(),
            };
            let who = [s(q, "userName"), s(q, "requestedBy")].into_iter().find(|n| !n.is_empty()).unwrap_or("");
            vec![s(q, "requestId").to_string(), [s(q, "guildName"), s(q, "guildId")].into_iter().find(|n| !n.is_empty()).unwrap_or("").to_string(), who.to_string(), left]
        })
        .collect()
}

/// 디스코드 서버 멤버 (`{members, partial}`)
pub async fn members(ctx: &Ctx, guild: &str) -> Result<Value> {
    let cfg = ctx.api.bot_config().await?;
    ctx.api.node_members(guild, relay_of(&cfg)).await
}

/// 멤버 표의 줄: 표시 이름, 사용자 이름, id (이름순). 프로필 사진 주소는 `--json`으로만
fn member_rows(r: &Value) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = resolve::member_items(r)
        .iter()
        .map(|m| vec![s(m, "displayName").to_string(), s(m, "username").to_string(), s(m, "id").to_string()])
        .collect();
    rows.sort_by_key(|r| r[0].to_lowercase());
    rows
}

pub async fn paired_nodes(ctx: &Ctx) -> Result<Value> {
    let cfg = ctx.api.bot_config().await?;
    let host = cfg.pointer("/cloud/hostId").and_then(Value::as_str).unwrap_or("");
    if host.is_empty() {
        return Err(failed(t!("bot.not_paired")));
    }
    ctx.api.host_nodes(host, relay_of(&cfg)).await
}

async fn nodes(ctx: &Ctx) -> Result<()> {
    let r = paired_nodes(ctx).await?;
    ctx.out.data(&r, |r| {
        let rows = node_rows(r);
        if rows.is_empty() {
            say!("{}", output::dim(&t!("bot.list_empty")));
            return;
        }
        let mut t = output::table(&[t!("col.name"), t!("col.id"), t!("bot.locale"), t!("bot.prefix")]);
        for row in rows {
            t.add_row(row);
        }
        output::print_table(t);
    });
    Ok(())
}

/// 릴레이 노드 목록 (맨 목록이거나 `{nodes: [...]}`)
fn node_list(r: &Value) -> Vec<Value> {
    r.get("nodes").or(Some(r)).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// 노드 표의 줄: 이름, id, 언어, 접두사. 프로필 사진 주소·시각 같은 긴 값은 `--json`으로만
fn node_rows(r: &Value) -> Vec<Vec<String>> {
    node_list(r)
        .iter()
        .map(|n| {
            vec![
                s(n, "guildName").to_string(),
                s(n, "guildId").to_string(),
                s(n, "locale").to_string(),
                prefixes_line(n.get("prefixes").unwrap_or(&Value::Null)),
            ]
        })
        .collect()
}

async fn config(ctx: &Ctx, pairs: &[String]) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    if pairs.is_empty() {
        ctx.out.data(&cfg, |c| {
            let mut rows = Vec::new();
            flatten(c, "", &mut rows);
            // revision은 저장 확인용 표식이라 설정 칸이 아니다
            rows.retain(|(k, _)| k != "revision");
            output::print_kv(
                &rows
                    .iter()
                    .map(|(k, v)| (k.clone(), output::mask(k, v)))
                    .collect::<Vec<_>>(),
            );
        });
        return Ok(());
    }
    let defaults = ctx.api.config_defaults("bot").await?;
    let defaults = defaults.get("defaults").cloned().unwrap_or(Value::Null);
    let mut changes = Vec::new();
    for (k, raw) in split_pairs(pairs)? {
        let v = bot_value(&cfg, &defaults, &k, &raw)?;
        changes.push((k, v));
    }
    let r = ctx.api.patch_bot_config(config_patch(&cfg, changes)).await?;
    saved(&r)?;
    ctx.out.done(&t!("bot.config_saved"), &r);
    Ok(())
}

/// 데몬이 권한 키라 저장하지 않고 이전 값으로 둔 경로(`ignored`, L61)가 있으면 일부 실패다(--json에 응답을 싣는다). 권한은 `perm`으로만 바뀐다
fn saved(r: &Value) -> Result<()> {
    let ignored: Vec<&str> = r.get("ignored").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    if ignored.is_empty() {
        return Ok(());
    }
    let message = t!("bot.config_ignored", keys = ignored.join(", "));
    Err(crate::error::CliError::Partial { message, data: r.clone() }.into())
}

/// `bot config` 한 칸의 값 (C14). 키는 데몬 기본값의 모양으로 확인한다: 정해진 칸 아래는 있는 키만,
/// 비어 있는 표(별명 등) 아래는 새 키도. 데몬은 모르는 키를 버리므로 저장했다고 말하기 전에 막는다.
/// 값은 지금 값(없으면 기본값)의 형식으로, 빈 값은 그 형식의 빈 값(이름이 자유로운 표 아래의 키는 지운다)
fn bot_value(cfg: &Value, defaults: &Value, key: &str, raw: &str) -> Result<Value> {
    let mut shape = defaults;
    let mut free = false;
    for part in key.split('.') {
        match shape.as_object() {
            Some(m) if m.is_empty() => { free = true; break }
            Some(m) if m.contains_key(part) => shape = &m[part],
            _ => {
                let mut rows = Vec::new();
                flatten(defaults, "", &mut rows);
                let names: Vec<String> = rows.into_iter().map(|(k, _)| k).collect();
                return Err(crate::cmd::unknown_setting(key, &names));
            }
        }
    }
    let current = get_path(cfg, key).or_else(|| get_path(defaults, key));
    if raw.is_empty() {
        return Ok(match current {
            _ if free => Value::Null,
            Some(Value::Array(_)) => json!([]),
            Some(Value::String(_)) => json!(""),
            _ => Value::Null,
        });
    }
    coerce(raw, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 권한 키는 봇 설정 저장으로 바뀌지 않는다(L61, 데몬 #469). 데몬이 `ignored`로 알린 키는 저장했다고 말하지 않고
    /// 실패로 끝내 그 키와 권한 명령을 보인다. 목록이 없거나 비면(옛 데몬 포함) 저장한 것이다
    #[test]
    fn permission_keys_the_daemon_kept_are_not_reported_as_saved() {
        crate::i18n::init("en");
        assert!(saved(&json!({ "success": true })).is_ok());
        assert!(saved(&json!({ "success": true, "ignored": [] })).is_ok());
        let e = saved(&json!({ "success": true, "ignored": ["nodeSettings.g1.defaultLevel", "nodeSettings.g1.members"] })).unwrap_err();
        let err = e.to_string();
        assert!(err.contains("nodeSettings.g1.defaultLevel") && err.contains("nodeSettings.g1.members"), "{err}");
        assert!(err.contains("saba-chan-cli perm"), "{err}");
        // --json도 유지된 키를 기계가 읽을 수 있게 싣는다
        let doc = e.downcast_ref::<crate::error::CliError>().unwrap().to_json();
        assert_eq!(doc["ignored"], json!(["nodeSettings.g1.defaultLevel", "nodeSettings.g1.members"]), "{doc}");
    }

    /// 페어링 결과: 이 PC에 노드 토큰이 저장됐을 때만 연결됨 (R15). 옛 데몬은 tokenSaved 없이 nodeToken만 준다
    #[test]
    fn pairing_counts_only_when_the_token_was_saved_on_this_pc() {
        assert!(token_saved(&json!({ "status": "claimed", "nodeToken": "t", "tokenSaved": true })));
        assert!(!token_saved(&json!({ "status": "claimed", "nodeToken": null, "tokenSaved": false })));
        // 쓰기 실패: 토큰은 받았지만 저장하지 못했다
        assert!(!token_saved(&json!({ "status": "claimed", "nodeToken": "t", "tokenSaved": false })));
        // 옛 데몬
        assert!(token_saved(&json!({ "status": "claimed", "nodeToken": "t" })));
        assert!(!token_saved(&json!({ "status": "claimed", "nodeToken": null })));
    }

    /// 페어링 대기는 끝이 있다 (R18): 상태 조회가 연달아 실패하거나 코드 유효기간이 지나면 멈춘다
    #[test]
    fn pairing_wait_stops_after_repeated_failures_or_past_the_code_lifetime() {
        let start = std::time::Instant::now();
        let mut wait = PairWait::new(start);
        for _ in 1..PAIR_POLL_FAILURES {
            assert!(!wait.failed());
        }
        // 한 번 성공하면 다시 센다
        wait.ok();
        for _ in 1..PAIR_POLL_FAILURES {
            assert!(!wait.failed());
        }
        assert!(wait.failed());
        assert!(!wait.timed_out(start + PAIR_WAIT_LIMIT - Duration::from_secs(1)));
        assert!(wait.timed_out(start + PAIR_WAIT_LIMIT));
    }

    /// `bot config` 값: 모르는 키는 저장하지 않고, 값은 지금 형식으로 (C14)
    #[test]
    fn bot_config_keys_are_checked_and_values_follow_the_current_type() {
        let defaults = json!({
            "prefixes": [], "cloud": { "relayUrl": "", "hostId": "" },
            "moduleAliases": {}, "nodeSettings": {}
        });
        let cfg = json!({ "prefixes": ["사바쨩"], "cloud": { "relayUrl": "https://a", "hostId": "h" }, "moduleAliases": {} });
        // 목록 칸은 쉼표로
        assert_eq!(bot_value(&cfg, &defaults, "prefixes", "사바, saba").unwrap(), json!(["사바", "saba"]));
        assert_eq!(bot_value(&cfg, &defaults, "prefixes", "").unwrap(), json!([]));
        assert_eq!(bot_value(&cfg, &defaults, "cloud.relayUrl", "https://b").unwrap(), json!("https://b"));
        // 이름이 자유로운 칸(별명 표) 아래에는 새 키를 둘 수 있다
        assert_eq!(bot_value(&cfg, &defaults, "moduleAliases.palworld", "pal").unwrap(), json!("pal"));
        // 자유로운 칸 아래의 빈 값은 그 키를 지운다
        let cfg2 = json!({ "moduleAliases": { "palworld": "pal" } });
        assert_eq!(bot_value(&cfg2, &defaults, "moduleAliases.palworld", "").unwrap(), Value::Null);
        // 모르는 키는 거절 (최상위 오타, 정해진 칸 아래 오타)
        assert!(bot_value(&cfg, &defaults, "prefix", "x").is_err());
        assert!(bot_value(&cfg, &defaults, "cloud.relayurl", "x").is_err());
    }

    /// `bot config` 저장은 고친 경로만 싣는 merge patch다: 같은 최상위 키 아래 다른 길드 값을 읽은 때의 값으로
    /// 덮어쓰지 않는다. 읽은 때의 revision을 실어 그사이 바뀌었으면 데몬이 거절한다. 빈 값(null)은 그 키를 지운다
    #[test]
    fn bot_config_saves_only_the_changed_paths_with_the_read_revision() {
        let cfg = json!({ "revision": "r1", "nodeSettings": { "g1": { "a": 1 }, "g2": { "a": 2 } } });
        let patch = config_patch(&cfg, vec![("nodeSettings.g1.a".into(), json!(5)), ("moduleAliases.pal".into(), Value::Null)]);
        assert_eq!(patch, json!({ "revision": "r1", "nodeSettings": { "g1": { "a": 5 } }, "moduleAliases": { "pal": null } }));
        // 옛 데몬은 revision을 주지 않는다: 그때는 싣지 않는다
        assert_eq!(config_patch(&json!({}), vec![("prefixes".into(), json!(["x"]))]), json!({ "prefixes": ["x"] }));
    }

    /// 릴레이가 이 PC를 아는지 한 줄로 (C11)
    #[test]
    fn the_relay_check_says_ok_or_why_not() {
        let line = |r| console::strip_ansi_codes(&relay_line(&r)).to_string();
        assert_eq!(line(Ok(json!({ "id": "h", "status": "online" }))), t!("bot.relay_ok"));
        assert_eq!(line(Ok(json!({ "error": "HOST_NOT_FOUND" }))), t!("bot.relay_failed", error = "HOST_NOT_FOUND"));
        assert_eq!(line(Err("timeout".into())), t!("bot.relay_failed", error = "timeout"));
    }

    /// 멤버 표는 이름·사용자 이름·id만 이름순으로 (C7)
    #[test]
    fn member_rows_show_names_and_ids_only() {
        let r = json!({ "members": [
            { "id": "2", "username": "saba", "displayName": "사바", "avatarUrl": "https://cdn.example/a.png" },
            { "id": "1", "username": "kimchi", "displayName": "김치마요" }
        ], "partial": true });
        assert_eq!(member_rows(&r), vec![vec!["김치마요", "kimchi", "1"], vec!["사바", "saba", "2"]]);
    }

    /// 연결 요청은 id·서버 이름·요청한 사람·남은 시간만 (C9)
    #[test]
    fn link_rows_show_who_asked_and_the_time_left() {
        let r = json!({ "requests": [
            { "requestId": "r1", "guildId": "9", "guildName": "친구들", "requestedBy": "5", "userName": "kimchi", "expiresAt": "2026-10-19T17:10:00.000Z" },
            { "requestId": "r2", "guildId": "8", "guildName": "", "requestedBy": "6", "userName": "", "expiresAt": "2026-10-19T17:00:00.000Z" }
        ] });
        let now = output::iso_epoch("2026-10-19T17:05:00Z").unwrap();
        let rows = link_rows(&r, now);
        assert_eq!(rows[0][..3], ["r1", "친구들", "kimchi"]);
        assert_eq!(rows[0][3], output::duration(300));
        assert_eq!(rows[1][..3], ["r2", "8", "6"]);
        assert_eq!(console::strip_ansi_codes(&rows[1][3]), t!("bot.expired"));
    }

    /// 표에는 알아볼 칸만: 디스코드 프로필 사진 주소 같은 긴 값이 줄을 깨지 않게
    #[test]
    fn node_rows_show_the_name_id_language_and_prefixes_only() {
        let r = json!([{
            "guildId": "1", "hostId": "h", "guildName": "사바 서버", "locale": "ko",
            "createdAt": "2026-10-19T00:00:00Z", "updatedAt": "2026-10-19T00:00:00Z",
            "iconUrl": "https://cdn.discordapp.com/icons/1/abcdef.png?size=128",
            "prefixes": ["사바쨩", "!"], "prefixDefault": false,
        }]);
        assert_eq!(node_rows(&r), vec![vec!["사바 서버", "1", "ko", "사바쨩, !"]]);
    }

    /// `bot channels`만 치면 페어링된 디스코드 서버마다 보여 준다. 바꾸기는 서버가 하나일 때만 id 없이 된다
    #[test]
    fn channels_without_a_guild_cover_every_paired_server_and_a_change_needs_exactly_one() {
        let two = json!([{ "guildId": "1", "guildName": "a" }, { "guildId": "2", "guildName": "b" }]);
        let one = json!([{ "guildId": "1", "guildName": "a" }]);
        let ids = |v: Vec<(String, String)>| v.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        assert_eq!(ids(target_guilds(&two, false).unwrap()), vec!["1", "2"]);
        assert!(target_guilds(&two, true).is_err());
        assert_eq!(ids(target_guilds(&one, true).unwrap()), vec!["1"]);
        assert!(target_guilds(&json!([]), false).is_err());
    }

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
