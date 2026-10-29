//! `chatbot` — 디스코드 AI 챗봇 설정 (데몬이 원본, 릴레이에 맞춰 보낸다)
//!
//! 설정 키는 릴레이가 정하고 검사한다 (400이면 데몬이 저장하지 않는다). CLI는 키 이름을 고정하지 않는다.
//! 값이 비면 그 키를 지운다(기본값으로). 길드별 값은 `--guild`로 공통 값 위에 덮는다.
//! API 키는 숨긴 입력이나 표준 입력으로만 받는다 (인자로 받으면 셸 기록에 남는다). 화면에는 저장 여부만.

use crate::cli::ChatbotCmd;
use crate::cmd::{guess, split_pairs};
use crate::ctx::Ctx;
use crate::error::{failed, usage, CliError, Result};
use crate::output::{self, s};
use serde_json::{json, Map, Value};
use crate::resolve;

pub async fn run(ctx: &Ctx, cmd: ChatbotCmd) -> Result<()> {
    match cmd {
        ChatbotCmd::Show { guild: None } => show(ctx).await,
        ChatbotCmd::Show { guild: Some(g) } => {
            let guild = resolve::guild(ctx, Some(&g)).await?;
            show_guild(ctx, &guild).await
        }
        ChatbotCmd::Set { pairs, guild } => {
            let guild = match guild {
                Some(g) => Some(resolve::guild(ctx, Some(&g)).await?),
                None => None,
            };
            set(ctx, &pairs, guild.as_deref()).await
        }
        ChatbotCmd::Key { delete } => key(ctx, delete).await,
        ChatbotCmd::Models { guild } => models(ctx, guild.as_deref()).await,
        ChatbotCmd::Credit { guild } => credit(ctx, guild.as_deref()).await,
        ChatbotCmd::Tools { guild } => tools(ctx, guild.as_deref()).await,
        ChatbotCmd::Template { locale } => {
            let r = ctx.api.chatbot_memo_template(locale.as_deref()).await?;
            // 파일로 받아 `set memo=@파일`에 쓰도록 글만 그대로 낸다
            ctx.out.data(&r, |r| say!("{}", s(r, "template")));
            Ok(())
        }
        ChatbotCmd::Clear => {
            ctx.ask.ensure(&t!("chatbot.confirm_clear"))?;
            let r = ctx.api.clear_chatbot().await?;
            // 릴레이에 닿지 못했으면 이 PC에서만 지운 것이다 (다음 연결 때 데몬이 다시 지운다)
            let msg = if r.get("pendingRelayDelete").and_then(Value::as_bool) == Some(true) {
                t!("chatbot.cleared_pending")
            } else {
                t!("chatbot.cleared")
            };
            ctx.out.done(&msg, &r);
            Ok(())
        }
        ChatbotCmd::Forget { guild } => {
            let guild = match guild {
                Some(g) => Some(resolve::guild(ctx, Some(&g)).await?),
                None => None,
            };
            let q = match &guild {
                Some(g) => t!("chatbot.confirm_forget_guild", guild = g),
                None => t!("chatbot.confirm_forget"),
            };
            ctx.ask.ensure(&q)?;
            // 릴레이에 일부만 닿으면 데몬은 200에 success:false와 길드별 sync를 준다: 어느 길드가 실패했는지 보이고 실패로 끝낸다
            let r = crate::client::partial(ctx.api.forget_chatbot(guild.as_deref()).await)?;
            if r.get("success").and_then(Value::as_bool) == Some(false) {
                report_sync(ctx, &r);
                return Err(CliError::Partial { message: t!("chatbot.forget_failed"), data: r }.into());
            }
            ctx.out.done(&t!("chatbot.forgot"), &r);
            report_sync(ctx, &r);
            Ok(())
        }
    }
}

async fn show(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.chatbot().await?;
    // 길드 제목에 쓸 이름 (페어링 전이거나 릴레이에 닿지 않으면 id 그대로)
    let names = match ctx.out.json || r.get("guilds").and_then(Value::as_object).is_none_or(|g| g.is_empty()) {
        true => Vec::new(),
        false => crate::cmd::bot::paired_nodes(ctx).await.map(|n| resolve::guild_items(&n)).unwrap_or_default(),
    };
    ctx.out.data(&r, |r| {
        let key = r.get("hasApiKey").and_then(Value::as_bool).unwrap_or(false);
        output::print_kv(&[(t!("chatbot.api_key"), output::yes_no(key))]);
        let settings = r
            .get("settings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        // 예전 데몬은 기본값 표를 주지 않는다: 그때는 저장된 값만
        let defaults = r.get("defaults").and_then(Value::as_object).cloned().unwrap_or_default();
        say!();
        output::heading(&t!("chatbot.common"));
        if defaults.is_empty() {
            print_settings(&settings);
        } else {
            output::print_kv(&setting_rows(&defaults, &settings));
        }
        for (guild, m) in r.get("guilds").and_then(Value::as_object).into_iter().flatten() {
            say!();
            let label = match names.iter().find(|(id, _)| id == guild) {
                Some((_, name)) if !name.is_empty() => format!("{} ({})", name, guild),
                _ => guild.clone(),
            };
            output::heading(&t!("chatbot.guild", guild = label));
            print_settings(m.as_object().unwrap_or(&Map::new()));
        }
        if !key {
            ctx.out.hint("saba-chan-cli chatbot key");
        }
    });
    Ok(())
}

/// 디스코드 서버 하나에 실제로 쓰이는 값 (공통 값 위에 그 서버 값)
async fn show_guild(ctx: &Ctx, guild: &str) -> Result<()> {
    let r = ctx.api.chatbot_guild(guild).await?;
    ctx.out.data(&r, |r| print_guild(guild, r));
    Ok(())
}

fn print_guild(guild: &str, r: &Value) {
    let key = r.get("hasApiKey").and_then(Value::as_bool).unwrap_or(false);
    output::print_kv(&[(t!("chatbot.api_key"), output::yes_no(key))]);
    say!();
    output::heading(&t!("chatbot.guild", guild = guild));
    print_settings(r.get("settings").and_then(Value::as_object).unwrap_or(&Map::new()));
}

fn print_settings(m: &Map<String, Value>) {
    if m.is_empty() {
        say!("  {}", output::dim(&t!("chatbot.defaults")));
        return;
    }
    let rows: Vec<(String, String)> = m
        .iter()
        .map(|(k, v)| (k.clone(), shown(k, v)))
        .collect();
    output::print_kv(&rows);
}

/// 공통 설정 표: 바꿀 수 있는 키(데몬의 기본값 표)와 저장된 키를 이름순으로, 저장하지 않은 키는 기본값에 표시를 붙여
fn setting_rows(defaults: &Map<String, Value>, settings: &Map<String, Value>) -> Vec<(String, String)> {
    let keys: std::collections::BTreeSet<&String> = defaults.keys().chain(settings.keys()).collect();
    keys.into_iter()
        .map(|k| {
            let row = match settings.get(k) {
                Some(v) => shown(k, v),
                None => format!("{} {}", shown(k, &defaults[k]), output::dim(&t!("chatbot.defaults"))),
            };
            (k.clone(), row)
        })
        .collect()
}

fn shown(k: &str, v: &Value) -> String {
    let mut shown = output::mask(k, v);
    // 긴 메모 같은 글은 앞부분만
    if shown.chars().count() > 80 {
        shown = format!("{}…", shown.chars().take(80).collect::<String>().replace('\n', " "));
    }
    shown
}

async fn set(ctx: &Ctx, pairs: &[String], guild: Option<&str>) -> Result<()> {
    let current = ctx.api.chatbot().await?;
    let mut patch = Map::new();
    for (k, raw) in split_pairs(pairs)? {
        let existing = guild
            .and_then(|g| current.pointer(&format!("/guilds/{}/{}", g, k)))
            .or_else(|| current.pointer(&format!("/settings/{}", k)));
        patch.insert(k, value_of(&raw, existing)?);
    }
    let body = match guild {
        Some(g) => json!({ "guilds": { g: patch } }),
        None => json!({ "settings": patch }),
    };
    let r = ctx.api.save_chatbot(body).await?;
    ctx.out.done(&t!("chatbot.saved"), &r);
    report_sync(ctx, &r);
    Ok(())
}

/// 빈 값 = 지우기(null), `@파일` = 파일 내용, `[a, b]`나 지금 값이 목록이면 쉼표로 나눈다, 아니면 추측
fn value_of(raw: &str, existing: Option<&Value>) -> Result<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }
    if let Some(path) = trimmed.strip_prefix('@') {
        if std::path::Path::new(path).is_file() {
            let text =
                std::fs::read_to_string(path).map_err(|e| failed(t!("chatbot.read_failed", path = path, error = e)))?;
            return Ok(Value::String(text.trim_end().to_string()));
        }
    }
    let list = |inner: &str| {
        Value::Array(
            inner
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                // 디스코드 id는 숫자처럼 보여도 문자열이다
                .map(|x| Value::String(x.to_string()))
                .collect(),
        )
    };
    // `[a, b]`: JSON이 아니어도 목록으로 (따옴표 없이 쓰게)
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return Ok(serde_json::from_str(trimmed).unwrap_or_else(|_| list(inner)));
    }
    if existing.is_some_and(Value::is_array) {
        return Ok(list(trimmed));
    }
    Ok(guess(raw))
}

async fn key(ctx: &Ctx, delete: bool) -> Result<()> {
    if delete {
        ctx.ask.ensure(&t!("chatbot.confirm_delete_key"))?;
        let r = ctx.api.save_chatbot(json!({ "apiKey": null })).await?;
        ctx.out.done(&t!("chatbot.key_deleted"), &r);
        report_sync(ctx, &r);
        return Ok(());
    }
    let key = ctx.ask.secret_or_stdin(&t!("chatbot.ask_key"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(usage(t!("chatbot.key_required")));
    }
    let r = ctx.api.save_chatbot(json!({ "apiKey": key })).await?;
    ctx.out.done(&t!("chatbot.key_saved"), &without_key(r));
    Ok(())
}

/// 혹시라도 응답에 키가 섞여 오면 지운다
fn without_key(mut r: Value) -> Value {
    if let Some(m) = r.as_object_mut() {
        m.remove("apiKey");
    }
    r
}

/// 릴레이에 못 보낸 길드를 알린다 (연결 실패는 다음 연결 때 데몬이 다시 맞춘다)
fn report_sync(ctx: &Ctx, r: &Value) {
    let rows = r.get("sync").and_then(Value::as_array).cloned().unwrap_or_default();
    if rows.is_empty() {
        if r.get("sync").is_some() {
            ctx.out.note(&t!("chatbot.not_synced"));
        }
        return;
    }
    for row in rows
        .iter()
        .filter(|x| x.get("ok").and_then(Value::as_bool) != Some(true))
    {
        let err = row.get("error").map(output::show).unwrap_or_default();
        ctx.out
            .warn(&t!("chatbot.sync_failed", guild = s(row, "guildId"), error = err));
    }
}

/// 저장된 키로 쓸 수 있는 모델 (릴레이가 제공자에게 묻는다). `chatbot set model=` 값을 고를 때
async fn models(ctx: &Ctx, guild: Option<&str>) -> Result<()> {
    let guild = resolve::guild(ctx, guild).await?;
    let cfg = ctx.api.bot_config().await?;
    let relay = cfg.pointer("/cloud/relayUrl").and_then(Value::as_str).filter(|u| !u.is_empty());
    let r = ctx.api.chatbot_models(&guild, relay).await?;
    ctx.out.data(&r, |r| {
        say!("{}: {}", t!("chatbot.provider"), s(r, "provider"));
        for line in model_lines(r) {
            say!("{}", line);
        }
        // 고른 모델이 목록에서 사라져 대신 쓰는 모델
        let (selected, effective) = (s(r, "selected"), s(r, "effectiveModel"));
        if !effective.is_empty() && effective != selected {
            ctx.out.warn(&t!("chatbot.model_replaced", selected = selected, effective = effective));
        }
        ctx.out.hint("saba-chan-cli chatbot set model=MODEL");
    });
    Ok(())
}

/// 카테고리별 도구와 그 서버에서 켰는지 (앱의 서버 › 말하는 사바쨩 탭과 같은 내용). 도구 목록은 릴레이가 원본
async fn tools(ctx: &Ctx, guild: Option<&str>) -> Result<()> {
    let guild = resolve::guild(ctx, guild).await?;
    let cfg = ctx.api.bot_config().await?;
    let relay = cfg.pointer("/cloud/relayUrl").and_then(Value::as_str).filter(|u| !u.is_empty());
    let catalog = ctx.api.chatbot_tools(&guild, relay).await?;
    let view = ctx.api.chatbot_guild(&guild).await?;
    let r = json!({ "guildId": guild, "catalog": catalog, "settings": view.get("settings").cloned().unwrap_or(Value::Null) });
    ctx.out.data(&r, |r| {
        for line in tool_lines(&r["catalog"], &r["settings"]) {
            say!("{}", line);
        }
        ctx.out.hint(&format!("saba-chan-cli chatbot set tools=[lol,pubg] --guild {}", guild));
    });
    Ok(())
}

/// 카테고리 제목 아래 `이름  상태` 줄. 공통에서 막힌 도구는 막힘, 운영 키·제공사 때문에 못 쓰면 덧붙인다
fn tool_lines(catalog: &Value, settings: &Value) -> Vec<String> {
    let names = |v: &Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    let on = names(&settings["tools"]);
    let allowed = settings["allowedTools"].as_array().map(|_| names(&settings["allowedTools"]));
    let tools = catalog["tools"].as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    for category in catalog["categories"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let rows: Vec<&Value> = tools.iter().filter(|t| t["category"] == category).collect();
        if rows.is_empty() {
            continue;
        }
        // 카테고리 이름은 릴레이가 언어별로 준다 (앱과 같은 이름)
        let labels = &catalog["categoryLabels"][category];
        let label = labels[crate::i18n::lang()].as_str().or(labels["en"].as_str()).unwrap_or(category);
        lines.push(format!("[{}]", label));
        for tool in rows {
            let name = s(tool, "name");
            let mut state = if allowed.as_ref().is_some_and(|a| !a.iter().any(|x| x == name)) {
                t!("chatbot.tool_blocked")
            } else if on.iter().any(|x| x == name) {
                t!("chatbot.tool_on")
            } else {
                t!("chatbot.tool_off")
            };
            if tool["available"] == json!(false) {
                state = format!("{} ({})", state, t!("chatbot.tool_unavailable"));
            }
            lines.push(format!("  {:<14} {}", name, state));
        }
    }
    lines
}

/// 남은 크레딧과 이번 달 사용량 (앱의 "남은 크레딧" 카드와 같은 내용)
async fn credit(ctx: &Ctx, guild: Option<&str>) -> Result<()> {
    let guild = resolve::guild(ctx, guild).await?;
    let cfg = ctx.api.bot_config().await?;
    let relay = cfg.pointer("/cloud/relayUrl").and_then(Value::as_str).filter(|u| !u.is_empty());
    let r = ctx.api.chatbot_credit(&guild, relay).await?;
    ctx.out.data(&r, |r| {
        for line in credit_lines(r) {
            say!("{}", line);
        }
        if let Some(at) = r.get("exhaustedAt").and_then(Value::as_str) {
            ctx.out.warn(&t!("chatbot.credit_exhausted", time = at));
        }
    });
    Ok(())
}

fn credit_lines(r: &Value) -> Vec<String> {
    let provider = s(r, "provider");
    let balance = match r.pointer("/balance/amount").and_then(Value::as_f64) {
        Some(amount) => format!("{} {}", amount, s(&r["balance"], "currency")),
        None if r.get("balanceSupported").and_then(Value::as_bool) == Some(true) => t!("chatbot.credit_balance_none"),
        None => t!("chatbot.credit_balance_unsupported", provider = provider),
    };
    let n = |k: &str| r.pointer(&format!("/usage/{}", k)).and_then(Value::as_u64).unwrap_or(0);
    vec![
        format!("{}: {}", t!("chatbot.provider"), provider),
        format!("{}: {}", t!("chatbot.credit_balance"), balance),
        format!("{}: {}", t!("chatbot.credit_usage", month = s(&r["usage"], "month")), t!(
            "chatbot.credit_usage_line",
            calls = n("calls"),
            input = n("input"),
            cached = n("cached"),
            output = n("output")
        )),
    ]
}

/// 모델 목록 한 줄씩, 지금 쓰는 모델에 ● 표시
fn model_lines(r: &Value) -> Vec<String> {
    let using = [s(r, "effectiveModel"), s(r, "selected")].into_iter().find(|m| !m.is_empty()).unwrap_or("");
    r.get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|m| if m == using { format!("{} {}", console::style("●").green(), output::bold(m)) } else { format!("  {}", m) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 모델 목록에서 지금 쓰는 모델(대신 쓰는 모델이 있으면 그것)을 표시한다 (C12)
    #[test]
    fn the_model_in_use_is_marked() {
        let r = json!({ "provider": "openai", "models": ["a", "b"], "selected": "gone", "effectiveModel": "b" });
        let lines: Vec<String> = model_lines(&r).iter().map(|l| console::strip_ansi_codes(l).to_string()).collect();
        assert_eq!(lines, vec!["  a", "● b"]);
    }

    /// 잔액을 주는 제공사는 잔액을, 아니면 주지 않는다는 안내를 보이고, 사용량은 늘 보인다
    #[test]
    fn credit_shows_the_balance_or_why_not_and_the_usage() {
        crate::i18n::init("ko");
        let usage = json!({ "month": "2026-10", "calls": 2, "input": 1500, "cached": 400, "output": 75 });
        let paid = credit_lines(&json!({
            "provider": "deepseek", "balanceSupported": true,
            "balance": { "amount": 12.34, "currency": "USD" }, "usage": usage, "exhaustedAt": null,
        }));
        assert_eq!(paid[1], format!("{}: 12.34 USD", t!("chatbot.credit_balance")));
        assert!(paid[2].contains("1500") && paid[2].contains("2026-10"), "{paid:?}");
        let none = credit_lines(&json!({ "provider": "openai", "balanceSupported": false, "balance": null, "usage": usage }));
        assert_eq!(none[1], format!("{}: {}", t!("chatbot.credit_balance"), t!("chatbot.credit_balance_unsupported", provider = "openai")));
    }

    /// 서버별 보기도 공통 보기처럼 API 키 저장 여부를 보인다
    #[test]
    fn the_guild_view_says_whether_a_key_is_saved() {
        crate::i18n::init("ko");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let rec = seen.clone();
        let _guard = output::SinkGuard::install(Box::new(move |_, text| {
            rec.lock().unwrap().push(console::strip_ansi_codes(text).to_string())
        }));
        print_guild("1", &json!({ "guildId": "1", "settings": {}, "hasApiKey": true }));
        let got = seen.lock().unwrap().clone();
        let row = format!("{}  {}", t!("chatbot.api_key"), console::strip_ansi_codes(&output::yes_no(true)));
        assert!(got.iter().any(|l| l.contains(&row)), "{got:?}");
    }

    /// 도구는 카테고리별로, 공통에서 막은 도구는 막힘, 운영 키가 없는 도구는 못 쓴다고 보인다
    #[test]
    fn tools_are_grouped_by_category_with_their_state() {
        crate::i18n::init("ko");
        let catalog = json!({
            "categories": ["game_servers", "games", "info"],
            "categoryLabels": { "game_servers": { "ko": "게임 서버" }, "games": { "ko": "게임 관련", "en": "Games" } },
            "tools": [
                { "name": "game_servers", "category": "game_servers", "available": true },
                { "name": "lol", "category": "games", "available": true },
                { "name": "pubg", "category": "games", "available": false, "unavailable": "operator_key" },
                { "name": "maplestory", "category": "games", "available": true },
            ],
        });
        let settings = json!({ "tools": ["lol", "maplestory"], "allowedTools": ["game_servers", "lol", "pubg"] });
        let lines = tool_lines(&catalog, &settings);
        assert_eq!(lines, vec![
            "[게임 서버]".to_string(),
            format!("  {:<14} {}", "game_servers", t!("chatbot.tool_off")),
            "[게임 관련]".to_string(),
            format!("  {:<14} {}", "lol", t!("chatbot.tool_on")),
            format!("  {:<14} {} ({})", "pubg", t!("chatbot.tool_off"), t!("chatbot.tool_unavailable")),
            format!("  {:<14} {}", "maplestory", t!("chatbot.tool_blocked")),
        ]);
    }

    /// 바꿀 수 있는 키는 데몬이 주는 기본값 표에서 온다: 바꾸지 않은 키도 기본값과 함께 보인다
    #[test]
    fn every_key_shows_with_its_value_or_its_default() {
        crate::i18n::init("ko");
        let defaults = json!({ "enabled": false, "model": "m" });
        let settings = json!({ "enabled": true, "memo": "hi" });
        let rows = setting_rows(defaults.as_object().unwrap(), settings.as_object().unwrap());
        let plain: Vec<(String, String)> = rows.into_iter().map(|(k, v)| (k, console::strip_ansi_codes(&v).to_string())).collect();
        assert_eq!(plain, vec![
            ("enabled".to_string(), console::strip_ansi_codes(&output::show(&json!(true))).to_string()),
            ("memo".to_string(), "hi".to_string()),
            ("model".to_string(), format!("m {}", t!("chatbot.defaults"))),
        ]);
    }
}
