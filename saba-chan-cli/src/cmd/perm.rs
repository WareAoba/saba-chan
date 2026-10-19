//! `perm` — 디스코드에서 누가 어떤 명령을 쓸 수 있는지 (데몬 권한 설정, P1)

use crate::cli::PermCmd;
use crate::ctx::Ctx;
use crate::error::{usage, Result};
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
        "chatbot" | "챗봇" => 0,
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
        println!("{}: {}", t!("perm.owner"), owner);
        if let Some(conflict) = r.get("relayOwnerDiscordId").and_then(Value::as_str) {
            ctx.out.warn(&t!("perm.owner_conflict", id = conflict));
        }
        let level = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(1);
        println!("{}: {}", t!("perm.default"), level_name(level("defaultLevel")));
        println!("{}: {}", t!("perm.preset"), s(r, "preset"));
        println!("{}: {}", t!("perm.cap"), level_name(level("chatbotCap")));

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
}
