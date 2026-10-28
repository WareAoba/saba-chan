//! `perm` — 디스코드에서 누가 어떤 명령을 쓸 수 있는지 (데몬 권한 설정, P1)

use crate::cli::{LevelCmd, PermCmd};
use crate::ctx::Ctx;
use crate::error::{usage, CliError, Result};
use crate::output::{self, s};
use crate::resolve;
use serde_json::{json, Value};
use std::collections::HashMap;

const OWNER: i64 = 3;

/// 단계 사다리 한 칸 (높은 것부터). 기본 단계는 이름이 없고, 새로 더할 단계는 id가 없다 (데몬이 번호를 준다)
#[derive(Debug, Clone, PartialEq)]
struct Rung {
    id: Option<i64>,
    name: Option<String>,
}

/// 권한 응답의 `ladder` (없으면 기본 다섯 단계)
fn ladder_of(perm: &Value) -> Vec<Rung> {
    let rungs: Vec<Rung> = perm
        .get("ladder")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| Some(Rung { id: Some(l.get("id")?.as_i64()?), name: l.get("name").and_then(Value::as_str).map(str::to_string) }))
        .collect();
    if rungs.len() >= 3 {
        return rungs;
    }
    [3, 2, 1, 0, -1].into_iter().map(|id| Rung { id: Some(id), name: None }).collect()
}

/// 데몬에 보낼 사다리: 기본 단계는 `{id}`, 더한 단계는 `{id?, name}`
fn ladder_patch(ladder: &[Rung]) -> Value {
    Value::Array(
        ladder
            .iter()
            .map(|r| match (r.id, &r.name) {
                (Some(id), None) => json!({ "id": id }),
                (Some(id), Some(name)) => json!({ "id": id, "name": name }),
                (None, name) => json!({ "name": name.clone().unwrap_or_default() }),
            })
            .collect(),
    )
}

/// 레벨 이름(영어·한국어, 이 서버에 더한 단계 이름) 또는 숫자 → 이 서버 사다리의 id
fn parse_level(raw: &str, ladder: &[Rung]) -> Result<i64> {
    let q = raw.trim().to_lowercase();
    let builtin = match q.as_str() {
        "owner" | "소유자" => Some(3),
        "admin" | "관리자" => Some(2),
        "user" | "유저" => Some(1),
        "guest" | "게스트" | "손님" | "chatbot" | "챗봇" | "말하는 사바쨩" | "talking saba-chan" => Some(0),
        "banned" | "금지" => Some(-1),
        n => n.parse().ok(),
    };
    let named = || ladder.iter().find(|r| r.name.as_deref().is_some_and(|n| n.to_lowercase() == q)).and_then(|r| r.id);
    // 기본 단계 낱말이 이 사다리에 없으면 같은 이름으로 더한 단계를 본다
    builtin.filter(|id| ladder.iter().any(|r| r.id == Some(*id))).or_else(named).ok_or_else(|| usage(t!("perm.bad_level", level = raw)))
}

/// 프리셋을 고르면 지워질 명령별 레벨 수
fn overrides(perm: &Value) -> usize {
    perm["commandLevels"].as_object().map_or(0, |m| m.len())
}

fn level_name(level: i64, ladder: &[Rung]) -> String {
    if let Some(name) = ladder.iter().find(|r| r.id == Some(level)).and_then(|r| r.name.clone()) {
        return name;
    }
    let key = match level {
        3 => "owner",
        2 => "admin",
        1 => "user",
        0 => "chatbot",
        _ => "banned",
    };
    t!(&format!("perm.level.{}", key))
}

/// 사다리 칸 위치. 소유자·금지는 움직이거나 지울 수 없다
fn movable_at(ladder: &[Rung], raw: &str) -> Result<usize> {
    let id = parse_level(raw, ladder)?;
    match ladder.iter().position(|r| r.id == Some(id)) {
        Some(i) if i > 0 && i < ladder.len() - 1 => Ok(i),
        _ => Err(usage(t!("perm.level_fixed", level = raw))),
    }
}

/// `above` 바로 위에 넣을 자리 (없으면 소유자 바로 아래). 소유자 위는 안 된다
fn slot_above(ladder: &[Rung], above: Option<&str>) -> Result<usize> {
    let Some(raw) = above else { return Ok(1) };
    let id = parse_level(raw, ladder)?;
    match ladder.iter().position(|r| r.id == Some(id)) {
        Some(i) if i > 0 => Ok(i),
        _ => Err(usage(t!("perm.level_fixed", level = raw))),
    }
}

/// 사다리 편집 하나. 이름 규칙(1~32자)과 개수는 데몬이 다시 검사한다
fn edit_ladder(mut ladder: Vec<Rung>, cmd: &LevelCmd) -> Result<Vec<Rung>> {
    match cmd {
        LevelCmd::Add { name, above, .. } => {
            let at = slot_above(&ladder, above.as_deref())?;
            ladder.insert(at, Rung { id: None, name: Some(name.trim().to_string()) });
        }
        LevelCmd::Rename { level, name, .. } => {
            let at = movable_at(&ladder, level)?;
            if ladder[at].name.is_none() {
                return Err(usage(t!("perm.level_builtin", level = level)));
            }
            ladder[at].name = Some(name.trim().to_string());
        }
        LevelCmd::Remove { level, .. } => {
            let at = movable_at(&ladder, level)?;
            ladder.remove(at);
        }
        LevelCmd::Move { level, above, .. } => {
            let from = movable_at(&ladder, level)?;
            let rung = ladder.remove(from);
            let to = slot_above(&ladder, Some(above))?;
            ladder.insert(to, rung);
        }
    }
    Ok(ladder)
}

/// 레벨 인자를 이 서버 사다리로 읽는다 (더한 단계 이름도 받게 권한을 먼저 읽는다)
async fn level_in(ctx: &Ctx, guild: &str, raw: &str) -> Result<i64> {
    parse_level(raw, &ladder_of(&ctx.api.permissions(guild).await?))
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
        PermCmd::Commands { guild, server } => {
            let guild = resolve::guild(ctx, guild.as_deref()).await?;
            let instance = match server {
                Some(q) => Some(resolve::server(ctx, Some(&q)).await?.id),
                None => None,
            };
            let (perm, board) = tokio::try_join!(ctx.api.permissions(&guild), ctx.api.permission_commands(&guild, crate::i18n::lang()))?;
            ctx.out.data(&board, |b| stairs(b, &ladder_of(&perm), instance.as_deref()));
            return Ok(());
        }
        PermCmd::Grant { guild, member, level } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let level = level_in(ctx, &guild, &level).await?;
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
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let level = level_in(ctx, &guild, &level).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "defaultLevel": level })).await?;
            (guild, r)
        }
        PermCmd::Preset { guild, preset } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            // GUI 처럼: 프리셋은 명령별 레벨을 지우므로 있으면 먼저 묻는다
            let count = overrides(&ctx.api.permissions(&guild).await?);
            if count > 0 && !ctx.ask.confirm(&t!("perm.preset_reset", count = count, preset = preset), false)? {
                return Err(CliError::Cancelled.into());
            }
            let r = ctx.api.update_permissions(&guild, json!({ "preset": preset })).await?;
            (guild, r)
        }
        PermCmd::Cap { guild, level } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let level = level_in(ctx, &guild, &level).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "chatbotCap": level })).await?;
            (guild, r)
        }
        PermCmd::Command { guild, key, level } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            let level = match level {
                Some(l) => json!(level_in(ctx, &guild, &l).await?),
                None => Value::Null,
            };
            let key = command_key(ctx, &key).await?;
            let r = ctx.api.update_permissions(&guild, json!({ "commandLevels": { key: level } })).await?;
            (guild, r)
        }
        PermCmd::Import { guild, members } => {
            let guild = resolve::guild(ctx, Some(&guild)).await?;
            return import(ctx, &guild, &members).await;
        }
        PermCmd::Level(cmd) => {
            let guild = resolve::guild(ctx, Some(cmd.guild())).await?;
            let ladder = edit_ladder(ladder_of(&ctx.api.permissions(&guild).await?), &cmd)?;
            let r = ctx.api.update_permissions(&guild, json!({ "ladder": ladder_patch(&ladder) })).await?;
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

/// 릴레이에 남은 부여값을 골라 데몬에 쓴다 (C10). 데몬은 저절로 가져오지 않는다: 원본이 둘이 되지 않게 사용자가 고른다
async fn import(ctx: &Ctx, guild: &str, wanted: &[String]) -> Result<()> {
    let cfg = ctx.api.bot_config().await?;
    let (perm, legacy) = tokio::try_join!(
        ctx.api.permissions(guild),
        ctx.api.legacy_members(guild, crate::cmd::bot::relay_of(&cfg))
    )?;
    let candidates = import_candidates(&perm, &legacy);
    let ladder = ladder_of(&perm);
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
                tell!("  {}: {}", label(id), level_name(*level, &ladder));
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

/// 가져오기 후보 `(id, level)`: 릴레이에 남은 부여값 중 데몬 것과 다른 것. 주인과 줄 수 없는 레벨(소유자, 이 서버에 없는 단계)은 뺀다
fn import_candidates(perm: &Value, legacy: &Value) -> Vec<(String, i64)> {
    let owner = match perm.get("ownerDiscordId") {
        Some(v) if !v.is_null() => member_id(&json!({ "id": v })),
        _ => String::new(),
    };
    let ladder = ladder_of(perm);
    let grantable = |l: &i64| *l != OWNER && ladder.iter().any(|r| r.id == Some(*l));
    legacy
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let level = m.get("level")?.as_i64().filter(grantable)?;
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

/// 레벨마다 그 레벨부터 쓸 수 있는 명령 (GUI 레벨별 명령 계단과 같다, 낮은 레벨부터).
/// `instance`가 있으면 그 서버 명령과 서버 전체 명령만
fn stairs(board: &Value, ladder: &[Rung], instance: Option<&str>) {
    let servers: HashMap<&str, &str> =
        board["instances"].as_array().into_iter().flatten().filter_map(|i| Some((i["id"].as_str()?, i["name"].as_str()?))).collect();
    let items: Vec<&Value> = board["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| instance.is_none_or(|id| i["instance"].as_str().is_none_or(|x| x == id)))
        .collect();
    for (n, id) in ladder.iter().rev().filter_map(|r| r.id).enumerate() {
        let here: Vec<String> = items
            .iter()
            .filter(|i| i["level"].as_i64() == Some(id))
            .map(|i| {
                let title = i["title"].as_str().or_else(|| i["name"].as_str()).unwrap_or("?");
                match (instance, i["instance"].as_str()) {
                    (None, Some(x)) => format!("{} ({})", title, servers.get(x).copied().unwrap_or(x)),
                    _ => title.to_string(),
                }
            })
            .collect();
        say!("Lv {} {}: {}", n, level_name(id, ladder), if here.is_empty() { "-".to_string() } else { here.join(", ") });
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
        let ladder = ladder_of(r);
        let level = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(1);
        let order: Vec<String> = ladder.iter().filter_map(|l| l.id).map(|id| level_name(id, &ladder)).collect();
        say!("{}: {}", t!("perm.ladder"), order.join(" > "));
        say!("{}: {}", t!("perm.default"), level_name(level("defaultLevel"), &ladder));
        say!("{}: {}", t!("perm.preset"), s(r, "preset"));
        say!("{}: {}", t!("perm.cap"), level_name(level("chatbotCap"), &ladder));
        // 높은 단계부터 (사다리 순서)
        let rank = |l: i64| ladder.iter().position(|r| r.id == Some(l)).unwrap_or(usize::MAX);

        let mut members: Vec<(String, i64)> = r
            .get("members")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(id, l)| (id.clone(), l.as_i64().unwrap_or(1)))
            .collect();
        members.sort_by(|a, b| rank(a.1).cmp(&rank(b.1)).then(a.0.cmp(&b.0)));
        if !members.is_empty() {
            let mut t = output::table(&[t!("perm.member"), t!("perm.level_col")]);
            for (id, l) in members {
                t.add_row(vec![member_name(names, &id), level_name(l, &ladder)]);
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
                t.add_row(vec![command_name(names, &k), level_name(l, &ladder)]);
            }
            output::print_table(t);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vip_ladder() -> Vec<Rung> {
        ladder_of(&json!({ "ladder": [{ "id": 3 }, { "id": 2 }, { "id": 10, "name": "VIP" }, { "id": 1 }, { "id": 0 }, { "id": -1 }] }))
    }

    #[test]
    fn level_names_in_either_language_or_numbers() {
        let ladder = ladder_of(&json!({}));
        assert_eq!(parse_level("admin", &ladder).unwrap(), 2);
        assert_eq!(parse_level("금지", &ladder).unwrap(), -1);
        assert_eq!(parse_level("0", &ladder).unwrap(), 0);
        assert!(parse_level("boss", &ladder).is_err());
        // 레벨 0 이름이 게스트로 바뀌어도(2026-10-28) 예전 입력 낱말은 그대로 받는다
        for name in ["guest", "게스트", "손님", "chatbot", "챗봇", "말하는 사바쨩", "Talking Saba-chan"] {
            assert_eq!(parse_level(name, &ladder).unwrap(), 0, "{name}");
        }
    }

    /// 사용자 지정 2026-10-25: 디스코드 서버마다 단계를 더하고 옮기고 지운다 (GUI 단계 탭과 같은 규칙)
    #[test]
    fn added_levels_are_named_and_the_ladder_can_be_edited() {
        let ladder = vip_ladder();
        assert_eq!(parse_level("vip", &ladder).unwrap(), 10);
        assert_eq!(level_name(10, &ladder), "VIP");
        let gone = edit_ladder(ladder.clone(), &LevelCmd::Remove { guild: "g".into(), level: "user".into() }).unwrap();
        assert!(parse_level("user", &gone).is_err(), "지운 단계는 고를 수 없다");
        // 지운 기본 단계와 같은 이름으로 더한 단계는 그 이름으로 고른다
        let renamed = ladder_of(&json!({ "ladder": [{ "id": 3 }, { "id": 2 }, { "id": 11, "name": "User" }, { "id": 0 }, { "id": -1 }] }));
        assert_eq!(parse_level("user", &renamed).unwrap(), 11);

        let added = edit_ladder(ladder.clone(), &LevelCmd::Add { guild: "g".into(), name: " Mod ".into(), above: Some("user".into()) }).unwrap();
        let moved = edit_ladder(added, &LevelCmd::Move { guild: "g".into(), level: "vip".into(), above: "admin".into() }).unwrap();
        let renamed = edit_ladder(moved, &LevelCmd::Rename { guild: "g".into(), level: "vip".into(), name: "단골".into() }).unwrap();
        assert_eq!(
            ladder_patch(&renamed),
            json!([{ "id": 3 }, { "id": 10, "name": "단골" }, { "id": 2 }, { "name": "Mod" }, { "id": 1 }, { "id": 0 }, { "id": -1 }])
        );
        // 소유자·금지는 고정, 기본 단계는 이름을 바꾸지 않는다
        assert!(edit_ladder(ladder.clone(), &LevelCmd::Remove { guild: "g".into(), level: "owner".into() }).is_err());
        assert!(edit_ladder(ladder.clone(), &LevelCmd::Move { guild: "g".into(), level: "vip".into(), above: "owner".into() }).is_err());
        assert!(edit_ladder(ladder, &LevelCmd::Rename { guild: "g".into(), level: "admin".into(), name: "x".into() }).is_err());
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
