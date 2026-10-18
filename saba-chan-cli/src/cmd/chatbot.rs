//! `chatbot` — 디스코드 AI 챗봇 설정 (데몬이 원본, 릴레이에 맞춰 보낸다)
//!
//! 설정 키는 릴레이가 정하고 검사한다 (400이면 데몬이 저장하지 않는다). CLI는 키 이름을 고정하지 않는다.
//! 값이 비면 그 키를 지운다(기본값으로). 길드별 값은 `--guild`로 공통 값 위에 덮는다.
//! API 키는 숨긴 입력이나 표준 입력으로만 받는다 (인자로 받으면 셸 기록에 남는다). 화면에는 저장 여부만.

use crate::cli::ChatbotCmd;
use crate::cmd::{guess, split_pairs};
use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use crate::output::{self, s};
use serde_json::{json, Map, Value};
use std::io::{BufRead, IsTerminal};

pub async fn run(ctx: &Ctx, cmd: ChatbotCmd) -> Result<()> {
    match cmd {
        ChatbotCmd::Show => show(ctx).await,
        ChatbotCmd::Set { pairs, guild } => set(ctx, &pairs, guild.as_deref()).await,
        ChatbotCmd::Key { delete } => key(ctx, delete).await,
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
            let q = match &guild {
                Some(g) => t!("chatbot.confirm_forget_guild", guild = g),
                None => t!("chatbot.confirm_forget"),
            };
            ctx.ask.ensure(&q)?;
            let r = ctx.api.forget_chatbot(guild.as_deref()).await?;
            ctx.out.done(&t!("chatbot.forgot"), &r);
            report_sync(ctx, &r);
            Ok(())
        }
    }
}

async fn show(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.chatbot().await?;
    ctx.out.data(&r, |r| {
        let key = r.get("hasApiKey").and_then(Value::as_bool).unwrap_or(false);
        output::print_kv(&[(t!("chatbot.api_key"), output::yes_no(key))]);
        let settings = r
            .get("settings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        println!();
        output::heading(&t!("chatbot.common"));
        print_settings(&settings);
        for (guild, m) in r.get("guilds").and_then(Value::as_object).into_iter().flatten() {
            println!();
            output::heading(&t!("chatbot.guild", guild = guild));
            print_settings(m.as_object().unwrap_or(&Map::new()));
        }
        if !key {
            ctx.out.hint("saba-chan-cli chatbot key");
        }
    });
    Ok(())
}

fn print_settings(m: &Map<String, Value>) {
    if m.is_empty() {
        println!("  {}", output::dim(&t!("chatbot.defaults")));
        return;
    }
    let rows: Vec<(String, String)> = m
        .iter()
        .map(|(k, v)| {
            let mut shown = output::mask(k, v);
            // 긴 메모 같은 글은 앞부분만
            if shown.chars().count() > 80 {
                shown = format!("{}…", shown.chars().take(80).collect::<String>().replace('\n', " "));
            }
            (k.clone(), shown)
        })
        .collect();
    output::print_kv(&rows);
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
    let key = match ctx.ask.secret(&t!("chatbot.ask_key"))? {
        Some(k) => k,
        // 터미널이 아니면 표준 입력 한 줄 (예: `cat key.txt | saba-chan-cli chatbot key`)
        None if !std::io::stdin().is_terminal() => {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            line
        }
        None => String::new(),
    };
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
