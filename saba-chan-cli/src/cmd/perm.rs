//! `perm` — 디스코드에서 누가 어떤 명령을 쓸 수 있는지 (데몬 권한 설정, P1)

use crate::cli::PermCmd;
use crate::ctx::Ctx;
use crate::error::{usage, Result};
use crate::output::{self, s};
use serde_json::{json, Value};

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
            let r = ctx.api.permissions(&guild).await?;
            show(ctx, &r);
            return Ok(());
        }
        PermCmd::Grant { guild, member, level } => {
            let level = parse_level(&level)?;
            let r = ctx.api.set_member_level(&guild, &member, Some(level)).await?;
            (guild, r)
        }
        PermCmd::Revoke { guild, member } => {
            let r = ctx.api.set_member_level(&guild, &member, None).await?;
            (guild, r)
        }
        PermCmd::Default { guild, level } => {
            let r = ctx.api.update_permissions(&guild, json!({ "defaultLevel": parse_level(&level)? })).await?;
            (guild, r)
        }
        PermCmd::Preset { guild, preset } => {
            let r = ctx.api.update_permissions(&guild, json!({ "preset": preset })).await?;
            (guild, r)
        }
        PermCmd::Cap { guild, level } => {
            let r = ctx.api.update_permissions(&guild, json!({ "chatbotCap": parse_level(&level)? })).await?;
            (guild, r)
        }
        PermCmd::Command { guild, key, level } => {
            let level = match level {
                Some(l) => json!(parse_level(&l)?),
                None => Value::Null,
            };
            let r = ctx.api.update_permissions(&guild, json!({ "commandLevels": { key: level } })).await?;
            (guild, r)
        }
        PermCmd::Owner { guild, member } => {
            let r = ctx.api.update_permissions(&guild, json!({ "ownerDiscordId": member })).await?;
            (guild, r)
        }
    };
    ctx.out.done(&t!("perm.saved", guild = guild), &r);
    Ok(())
}

fn show(ctx: &Ctx, r: &Value) {
    ctx.out.data(r, |r| {
        let owner = match s(r, "ownerDiscordId") {
            "" => output::dim(&t!("perm.no_owner")),
            id => id.to_string(),
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
                t.add_row(vec![id, level_name(l)]);
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
                t.add_row(vec![k, level_name(l)]);
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
}
