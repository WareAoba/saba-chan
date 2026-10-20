//! `perm` — 디스코드에서 누가 어떤 명령을 쓸 수 있는지 (데몬 권한 설정, P1)

use crate::cli::PermCmd;
use crate::ctx::Ctx;
use crate::error::{usage, CliError, Result};
use crate::output::{self, s};
use crate::resolve;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 레벨 이름(영어·한국어) 또는 숫자 → -1..3
fn parse_level(raw: &str) -> Result<i64> {
    let level = match raw.trim().to_lowercase().as_str() {
        "owner" | "소유자" => 3,
        "admin" | "관리자" => 2,
        "user" | "유저" => 1,
        "chatbot" | "챗봇" | "말하는 사바쨩" | "talking saba-chan" => 0,
        "banned" | "금지" => -1,
        n => n.parse().map_err(|_| usage(t!("perm.bad_level", level = raw)))?,
    };
    Ok(level)
}

fn level_name(level: i64) -> String {
    let key = match level {
        3 => "owner",
        2 => "admin",
        1 => "user",
        0 => "chatbot",
        _ => "banned",
    };
    t!(&format!("perm.level.{}", key))
}

pub async fn run(ctx: &Ctx, cmd: PermCmd) -> Result<()> {
    let (guild, r) = match cmd {
        PermCmd::Show { guild } => {
            let guild = resolve::guild(ctx, guild.as_deref()).await?;
            let r = ctx.api.permissions(&guild).await?;
            let names = if ctx.out.json { Names::default() } else { names(ctx, &guild).await };
            show(ctx, &r, &names);
            return Ok(());
        }
        PermCmd::Grant { guild, member, level } => {
            let level = parse_level(&level)?;
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let member = resolve::member(ctx, &guild, &member).await?;
            let r = ctx.api.set_member_level(&guild, &member, Some(level)).await?;
            (guild, r)
        }
        PermCmd::Revoke { guild, member } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let member = resolve::member(ctx, &guild, &member).await?;
            let r = ctx.api.set_member_level(&guild, &member, None).await?;
            (guild, r)
        }
        PermCmd::Default { guild, level } => {
            let level = parse_level(&level)?;
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "defaultLevel": level })).await?;
            (guild, r)
        }
        PermCmd::Preset { guild, preset } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "preset": preset })).await?;
            (guild, r)
        }
        PermCmd::Cap { guild, level } => {
            let level = parse_level(&level)?;
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "chatbotCap": level })).await?;
            (guild, r)
        }
        PermCmd::Command { guild, key, level } => {
            let level = match level {
                Some(l) => json!(parse_level(&l)?),
                None => Value::Null,
            };
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let key = command_key(ctx, &key).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "commandLevels": { key: level } })).await?;
            (guild, r)
        }
        PermCmd::Import { guild, members } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            return import(ctx, &guild, &members).await;
        }
        PermCmd::Owner { guild, member } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let member = resolve::member(ctx, &guild, &member).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "ownerDiscordId": member })).await?;
            (guild, r)
        }
    };
    ctx.out.done(&t!("perm.saved", guild = guild), &r);
    Ok(())
}

/// 릴레이에 남은 부여값을 골라 데몬에 쓴다 (C10). 데몬은 저절로 가져오지 않는다: 원본이 둘이 되지 않게 사용자가 고른다
async fn import(ctx: &Ctx, guild: &str, wanted: &[String]) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let (perm, legacy) = tokio::try_join!(
        ctx.api.permissions(guild),
        ctx.api.legacy_members(guild, crate::cmd::bot::relay_of(&cfg))
    )?;
    let candidates = import_candidates(&perm, &legacy);
    if candidates.is_empty() {
        ctx.out.done(&t!("perm.import_none"), &json!({ "imported": [] }));
        return Ok(());
    }
    let label = |id: &str| {
        legacy.as_array().into_iter().flatten().find(|m| member_id(m) == id).map(resolve::member_label).filter(|l| !l.is_empty()).unwrap_or_else(|| id.to_string())
    };
    let chosen: Vec<(String, i64)> = if wanted.is_empty() {
        if !ctx.out.json {
            for (id, level) in &candidates {
                tell!("  {}: {}", label(id), level_name(*level));
            }
        }
        ctx.ask.ensure(&t!("perm.import_confirm", count = candidates.len()))?;
        candidates
    } else {
        let mut chosen = Vec::new();
        for q in wanted {
            // 길드를 떠난 예전 멤버도 이름으로 고르게, 릴레이 목록의 이름을 먼저 본다
            let id = match legacy_named(&legacy, q) {
                Some(id) => id,
                None => resolve::member(ctx, guild, q).await?,
            };
            match candidates.iter().find(|(c, _)| *c == id) {
                Some(c) => chosen.push(c.clone()),
                None => return Err(usage(t!("perm.import_not_candidate", member = q))),
            }
        }
        chosen
    };
    let mut imported = Vec::new();
    let mut failed = Vec::new();
    for (id, level) in &chosen {
        match ctx.api.set_member_level(guild, id, Some(*level)).await {
            Ok(_) => imported.push(json!({ "id": id, "level": level })),
            Err(e) => {
                ctx.out.warn(&format!("{}: {}", label(id), e));
                failed.push(json!({ "id": id, "level": level, "error": e.to_string() }));
            }
        }
    }
    let data = json!({ "imported": imported, "failed": failed });
    if !failed.is_empty() {
        return Err(CliError::Partial { message: t!("perm.import_partial", ok = imported.len(), failed = failed.len()), data }.into());
    }
    ctx.out.done(&t!("perm.imported", count = imported.len(), guild = guild), &data);
    Ok(())
}

/// 릴레이 목록에서 표시 이름·사용자 이름(앞의 `@` 무시, 대소문자 무시)이 꼭 맞는 하나의 id
fn legacy_named(legacy: &Value, query: &str) -> Option<String> {
    let q = query.trim().trim_start_matches('@').to_lowercase();
    let mut hits = legacy.as_array().into_iter().flatten().filter(|m| {
        [s(m, "displayName"), s(m, "username")].iter().any(|n| !n.is_empty() && n.to_lowercase() == q)
    });
    match (hits.next(), hits.next()) {
        (Some(m), None) => Some(member_id(m)),
        _ => None,
    }
}

/// 디스코드 id (문자열이나 숫자)
fn member_id(m: &Value) -> String {
    match m.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 가져오기 후보 `(id, level)`: 릴레이에 남은 부여값 중 데몬 것과 다른 것. 주인과 줄 수 없는 레벨(소유자 등)은 뺀다
fn import_candidates(perm: &Value, legacy: &Value) -> Vec<(String, i64)> {
    let owner = match perm.get("ownerDiscordId") {
        Some(v) if !v.is_null() => member_id(&json!({ "id": v })),
        _ => String::new(),
    };
    legacy
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let level = m.get("level")?.as_i64().filter(|l| (-1..=2).contains(l))?;
            let id = member_id(m);
            let same = perm.pointer(&format!("/members/{}", id)).and_then(Value::as_i64) == Some(level);
            (!id.is_empty() && id != owner && !same).then_some((id, level))
        })
        .collect()
}

/// `<서버>/<명령>`의 서버를 이름으로도 받는다 (저장은 서버 id로)
async fn command_key(ctx: &Ctx, key: &str) -> Result<String> {
    match key.split_once('/') {
        Some((server, command)) => Ok(format!("{}/{}", resolve::server(ctx, Some(server)).await?.id, command)),
        None => Ok(key.to_string()),
    }
}

/// 표에 id 대신 보일 이름 (못 받으면 id 그대로)
#[derive(Default)]
struct Names {
    members: HashMap<String, String>,
    servers: HashMap<String, String>,
}

async fn names(ctx: &Ctx, guild: &str) -> Names {
    let (members, servers) = tokio::join!(crate::cmd::bot::members(ctx, guild), resolve::servers(ctx));
    Names {
        members: members
            .map(|r| resolve::member_items(&r).iter().map(|m| (s(m, "id").to_string(), resolve::member_label(m))).collect())
            .unwrap_or_default(),
        servers: servers.map(|list| list.into_iter().map(|sv| (sv.id, sv.name)).collect()).unwrap_or_default(),
    }
}

fn member_name(names: &Names, id: &str) -> String {
    match names.members.get(id) {
        Some(n) if n != id => format!("{} {}", n, output::dim(&format!("({})", id))),
        _ => id.to_string(),
    }
}

fn command_name(names: &Names, key: &str) -> String {
    match key.split_once('/').and_then(|(id, command)| Some((names.servers.get(id)?, command))) {
        Some((server, command)) => format!("{}/{}", server, command),
        None => key.to_string(),
    }
}

fn show(ctx: &Ctx, r: &Value, names: &Names) {
    ctx.out.data(r, |r| {
        let owner = match s(r, "ownerDiscordId") {
            "" => output::dim(&t!("perm.no_owner")),
            id => member_name(names, id),
        };
        say!("{}: {}", t!("perm.owner"), owner);
        if let Some(conflict) = r.get("relayOwnerDiscordId").and_then(Value::as_str) {
            ctx.out.warn(&t!("perm.owner_conflict", id = conflict));
        }
        let level = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(1);
        say!("{}: {}", t!("perm.default"), level_name(level("defaultLevel")));
        say!("{}: {}", t!("perm.preset"), s(r, "preset"));
        say!("{}: {}", t!("perm.cap"), level_name(level("chatbotCap")));

        let mut members: Vec<(String, i64)> = r
            .get("members")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(id, l)| (id.clone(), l.as_i64().unwrap_or(1)))
            .collect();
        members.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if !members.is_empty() {
            let mut t = output::table(&[t!("perm.member"), t!("perm.level_col")]);
            for (id, l) in members {
                t.add_row(vec![member_name(names, &id), level_name(l)]);
            }
            output::print_table(t);
        }
        let overrides: Vec<(String, i64)> = r
            .get("commandLevels")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(k, l)| (k.clone(), l.as_i64().unwrap_or(1)))
            .collect();
        if !overrides.is_empty() {
            let mut t = output::table(&[t!("perm.command"), t!("perm.level_col")]);
            for (k, l) in overrides {
                t.add_row(vec![command_name(names, &k), level_name(l)]);
            }
            output::print_table(t);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_names_in_either_language_or_numbers() {
        assert_eq!(parse_level("admin").unwrap(), 2);
        assert_eq!(parse_level("금지").unwrap(), -1);
        assert_eq!(parse_level("0").unwrap(), 0);
        assert!(parse_level("boss").is_err());
        // 서비스 이름이 바뀌어도(말하는 사바쨩) 예전 입력 낱말은 그대로 받는다
        for name in ["chatbot", "챗봇", "말하는 사바쨩", "Talking Saba-chan"] {
            assert_eq!(parse_level(name).unwrap(), 0, "{name}");
        }
    }

    /// 권한 표는 디스코드 id 대신 멤버·서버 이름을 보인다 (C8). 모르는 id는 그대로
    #[test]
    fn members_and_command_servers_show_by_name() {
        let names = Names {
            members: HashMap::from([("1".to_string(), "김치마요 (@kimchi)".to_string())]),
            servers: HashMap::from([("abc".to_string(), "마크".to_string())]),
        };
        assert!(console::strip_ansi_codes(&member_name(&names, "1")).starts_with("김치마요 (@kimchi)"));
        assert_eq!(member_name(&names, "2"), "2");
        assert_eq!(command_name(&names, "abc/start"), "마크/start");
        assert_eq!(command_name(&names, "zzz/start"), "zzz/start");
        assert_eq!(command_name(&names, "start"), "start");
    }

    /// 가져오기 후보: 릴레이에 남은 부여값 중 데몬과 다른 것. 주인과 줄 수 없는 레벨은 뺀다 (C10, GUI importCandidates와 같은 규칙)
    #[test]
    fn import_candidates_are_relay_grants_that_differ() {
        let perm = json!({ "ownerDiscordId": "9", "members": { "1": 2, "2": 1 } });
        let legacy = json!([
            { "id": "1", "level": 2 },
            { "id": "2", "level": 2 },
            { "id": 3, "level": -1 },
            { "id": "9", "level": 1 },
            { "id": "4", "level": 3 },
            { "id": "5" }
        ]);
        assert_eq!(import_candidates(&perm, &legacy), vec![("2".to_string(), 2), ("3".to_string(), -1)]);
        // 길드를 떠나 멤버 목록에 없는 예전 멤버도 릴레이 목록의 이름으로 고른다
        let named = json!([{ "id": "7", "username": "old_friend", "displayName": "옛친구" }, { "id": "8", "username": "dup" }, { "id": "9", "username": "Dup" }]);
        assert_eq!(legacy_named(&named, "@Old_Friend").as_deref(), Some("7"));
        assert_eq!(legacy_named(&named, "옛친구").as_deref(), Some("7"));
        assert_eq!(legacy_named(&named, "dup"), None);
    }
}
