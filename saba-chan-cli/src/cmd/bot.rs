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
            let _ = ctx.api.agent_stop().await;
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
    let err = s(&agent, "lastError");
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
    let relay = relay_url.clone().or_else(|| relay_of(&cfg).map(str::to_string));
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
    // 데몬은 노드 토큰만 저장한다. 호스트 id·릴레이 주소는 봇 설정에 넣는다.
    let mut cloud = cfg.get("cloud").cloned().unwrap_or_else(|| json!({}));
    if !s(&result, "hostId").is_empty() {
        cloud["hostId"] = Value::String(s(&result, "hostId").to_string());
    }
    if let Some(u) = relay_url {
        cloud["relayUrl"] = Value::String(u);
    }
    ctx.api.save_bot_config(json!({ "cloud": cloud })).await?;
    let mut out = result.clone();
    if let Some(o) = out.as_object_mut() {
        o.remove("nodeToken");
    }
    ctx.out.done(&t!("bot.paired"), &out);
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
