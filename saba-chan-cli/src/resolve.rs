//! 사람이 친 이름으로 대상 찾기 — 서버·모듈·익스텐션 공통
//!
//! 순서: 정확히 같음 → 대소문자 무시 같음 → 앞부분 일치 → 포함. 처음으로 결과가 나온 단계에서 멈춘다.
//! 여럿이면 터미널에서는 고르게 하고, 아니면 후보를 보여 주며 실패한다.
//! 없으면 비슷한 이름을 제안한다. 지우기·권한처럼 되돌리기 어려운 대상(`find_exact`)은 앞의 두 단계만 쓴다.

use crate::ctx::Ctx;
use crate::error::{usage, CliError, Result};
use crate::output::s;
use serde_json::Value;

pub enum Found<T> {
    One(T),
    Many(Vec<T>),
    None(Vec<String>),
}

pub fn find<T: Clone>(
    items: &[T],
    query: &str,
    names: impl Fn(&T) -> Vec<String>,
    label: impl Fn(&T) -> String,
) -> Found<T> {
    find_upto(items, query, names, label, 4)
}

/// 지우기·권한 대상: 정확히(대소문자 무시) 맞을 때만 고른다. 앞부분·포함으로만 맞는 이름은 고르지 않고 제안으로 보인다
/// ("bob"으로 "bobby"를 지우거나 권한을 주지 않게)
pub fn find_exact<T: Clone>(
    items: &[T],
    query: &str,
    names: impl Fn(&T) -> Vec<String>,
    label: impl Fn(&T) -> String,
) -> Found<T> {
    match find_upto(items, query, &names, &label, 2) {
        Found::None(close) => match find(items, query, &names, &label) {
            Found::One(t) => Found::None(vec![label(&t)]),
            Found::Many(list) => Found::None(list.iter().map(&label).collect()),
            Found::None(_) => Found::None(close),
        },
        found => found,
    }
}

fn find_upto<T: Clone>(
    items: &[T],
    query: &str,
    names: impl Fn(&T) -> Vec<String>,
    label: impl Fn(&T) -> String,
    stages_used: usize,
) -> Found<T> {
    let q = query.trim();
    let ql = q.to_lowercase();
    type Stage = fn(&str, &str, &str) -> bool;
    let stages: [Stage; 4] = [
        |n, q, _| n == q,
        |n, _, ql| n.to_lowercase() == ql,
        |n, _, ql| n.to_lowercase().starts_with(ql),
        |n, _, ql| n.to_lowercase().contains(ql),
    ];
    for stage in stages.into_iter().take(stages_used) {
        let hits: Vec<T> = items
            .iter()
            .filter(|it| names(it).iter().any(|n| stage(n, q, &ql)))
            .cloned()
            .collect();
        match hits.len() {
            0 => continue,
            1 => return Found::One(hits.into_iter().next().unwrap()),
            _ => return Found::Many(hits),
        }
    }
    let mut close: Vec<(usize, String)> = items
        .iter()
        .filter_map(|it| {
            let best = names(it).iter().map(|n| distance(&n.to_lowercase(), &ql)).min()?;
            (best <= (ql.chars().count() / 3).max(2)).then(|| (best, label(it)))
        })
        .collect();
    close.sort();
    close.dedup_by(|a, b| a.1 == b.1);
    Found::None(close.into_iter().take(3).map(|(_, l)| l).collect())
}

/// 편집 거리 (짧은 이름용)
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 찾은 결과를 하나로 좁힌다
pub fn settle<T: Clone>(
    ctx: &Ctx,
    what: &str,
    query: &str,
    found: Found<T>,
    label: impl Fn(&T) -> String,
) -> Result<T> {
    match found {
        Found::One(t) => Ok(t),
        Found::Many(list) => {
            let labels: Vec<String> = list.iter().map(&label).collect();
            match ctx
                .ask
                .select(&t!("resolve.which", what = what, query = query), &labels, 0)?
            {
                Some(i) => Ok(list[i].clone()),
                None => Err(CliError::Ambiguous {
                    what: what.into(),
                    query: query.into(),
                    candidates: labels,
                }
                .into()),
            }
        }
        Found::None(suggestions) => Err(CliError::NotFound {
            what: what.into(),
            query: query.into(),
            suggestions,
        }
        .into()),
    }
}

/// 이름을 안 줬을 때: 하나뿐이면 그것, 터미널이면 고르기, 아니면 사용법 오류
pub fn choose<T: Clone>(ctx: &Ctx, what: &str, items: Vec<T>, label: impl Fn(&T) -> String) -> Result<T> {
    if items.is_empty() {
        return Err(CliError::Failed(t!("resolve.none_exist", what = what)).into());
    }
    if items.len() == 1 && !ctx.ask.enabled {
        return Ok(items.into_iter().next().unwrap());
    }
    let labels: Vec<String> = items.iter().map(&label).collect();
    match ctx.ask.select(&t!("resolve.pick", what = what), &labels, 0)? {
        Some(i) => Ok(items[i].clone()),
        None => Err(usage(t!("resolve.name_required", what = what))),
    }
}

// ── 서버 ──

#[derive(Clone, Debug)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub module: String,
    /// GET /api/instances 항목 그대로
    pub raw: Value,
}

impl Server {
    pub fn from_instance(v: &Value) -> Self {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Self {
            id: s("id"),
            name: s("name"),
            module: s("module_name"),
            raw: v.clone(),
        }
    }

    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.module)
    }
}

pub async fn servers(ctx: &Ctx) -> Result<Vec<Server>> {
    Ok(ctx.api.instances().await?.iter().map(Server::from_instance).collect())
}

pub async fn server(ctx: &Ctx, query: Option<&str>) -> Result<Server> {
    server_by(ctx, query, find).await
}

/// 지우거나 초기화할 서버: 이름·id가 정확히(대소문자 무시) 맞아야 한다 (`find_exact`)
pub async fn server_exact(ctx: &Ctx, query: Option<&str>) -> Result<Server> {
    server_by(ctx, query, find_exact).await
}

type Finder<T> = fn(&[T], &str, fn(&T) -> Vec<String>, fn(&T) -> String) -> Found<T>;

async fn server_by(ctx: &Ctx, query: Option<&str>, finder: Finder<Server>) -> Result<Server> {
    let all = servers(ctx).await?;
    let what = t!("noun.server");
    match query.filter(|q| !q.trim().is_empty()) {
        None => choose(ctx, &what, all, Server::label),
        Some(q) => {
            let found = finder(&all, q, |s| vec![s.name.clone(), s.id.clone()], |s| s.name.clone());
            settle(ctx, &what, q, found, Server::label)
        }
    }
}

// ── 모듈 ──

/// 모듈 목록 항목에서 사람이 부를 수 있는 이름들
pub fn module_names(m: &Value) -> Vec<String> {
    let mut names: Vec<String> = ["name", "display_name", "game_name"]
        .iter()
        .filter_map(|k| m.get(*k).and_then(Value::as_str).map(str::to_string))
        .collect();
    if let Some(a) = m.get("module_aliases").and_then(Value::as_array) {
        names.extend(a.iter().filter_map(Value::as_str).map(str::to_string));
    }
    names
}

pub fn module_label(m: &Value) -> String {
    let name = m.get("name").and_then(Value::as_str).unwrap_or("?");
    match m.get("display_name").and_then(Value::as_str) {
        Some(d) if d != name => format!("{} ({})", d, name),
        _ => name.to_string(),
    }
}

pub async fn modules(ctx: &Ctx) -> Result<Vec<Value>> {
    Ok(ctx
        .api
        .modules()
        .await?
        .get("modules")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

pub async fn module(ctx: &Ctx, query: Option<&str>) -> Result<Value> {
    module_by(ctx, query, find).await
}

/// 지울 모듈: 이름·별칭이 정확히(대소문자 무시) 맞아야 한다 (`find_exact`)
pub async fn module_exact(ctx: &Ctx, query: Option<&str>) -> Result<Value> {
    module_by(ctx, query, find_exact).await
}

async fn module_by(ctx: &Ctx, query: Option<&str>, finder: Finder<Value>) -> Result<Value> {
    let all = modules(ctx).await?;
    let what = t!("noun.module");
    match query.filter(|q| !q.trim().is_empty()) {
        None => choose(ctx, &what, all, module_label),
        Some(q) => {
            let found = finder(&all, q, module_names, |m| {
                m.get("name").and_then(Value::as_str).unwrap_or("?").to_string()
            });
            settle(ctx, &what, q, found, module_label)
        }
    }
}

// ── 익스텐션 ──

pub async fn extensions(ctx: &Ctx) -> Result<Vec<Value>> {
    Ok(ctx
        .api
        .extensions()
        .await?
        .get("extensions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

pub fn extension_label(e: &Value) -> String {
    let id = e.get("id").and_then(Value::as_str).unwrap_or("?");
    match e.get("name").and_then(Value::as_str) {
        Some(n) if n != id => format!("{} ({})", n, id),
        _ => id.to_string(),
    }
}

pub async fn extension(ctx: &Ctx, query: Option<&str>) -> Result<Value> {
    extension_by(ctx, query, find).await
}

/// 지울 익스텐션: id·이름이 정확히(대소문자 무시) 맞아야 한다 (`find_exact`)
pub async fn extension_exact(ctx: &Ctx, query: Option<&str>) -> Result<Value> {
    extension_by(ctx, query, find_exact).await
}

async fn extension_by(ctx: &Ctx, query: Option<&str>, finder: Finder<Value>) -> Result<Value> {
    let all = extensions(ctx).await?;
    let what = t!("noun.extension");
    let names = |e: &Value| {
        ["id", "name"]
            .iter()
            .filter_map(|k| e.get(*k).and_then(Value::as_str).map(str::to_string))
            .collect()
    };
    match query.filter(|q| !q.trim().is_empty()) {
        None => choose(ctx, &what, all, extension_label),
        Some(q) => {
            let found = finder(&all, q, names, |e| {
                e.get("id").and_then(Value::as_str).unwrap_or("?").to_string()
            });
            settle(ctx, &what, q, found, extension_label)
        }
    }
}

// ── 디스코드 서버·멤버 ──

/// 디스코드 id(숫자 snowflake)면 목록을 받지 않고 그대로 쓴다 (릴레이에 닿지 않아도 데몬에 저장하는 설정은 바뀌게)
fn discord_id(query: &str) -> Option<&str> {
    let q = query.trim();
    (q.len() >= 15 && q.bytes().all(|b| b.is_ascii_digit())).then_some(q)
}

/// 페어링된 디스코드 서버 `(id, 이름)` (릴레이 노드 목록: 맨 목록이거나 `{nodes}`)
pub fn guild_items(nodes: &Value) -> Vec<(String, String)> {
    nodes
        .get("nodes")
        .or(Some(nodes))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|n| (s(n, "guildId").to_string(), s(n, "guildName").to_string()))
        .filter(|(id, _)| !id.is_empty())
        .collect()
}

fn guild_label(g: &(String, String)) -> String {
    if g.1.is_empty() {
        g.0.clone()
    } else {
        format!("{} ({})", g.1, g.0)
    }
}

/// 디스코드 서버 이름이나 id → id. 안 주면 페어링된 서버가 하나일 때 그것 (터미널이면 고르기)
pub async fn guild(ctx: &Ctx, query: Option<&str>) -> Result<String> {
    if let Some(id) = query.and_then(discord_id) {
        return Ok(id.to_string());
    }
    let items = guild_items(&crate::cmd::bot::paired_nodes(ctx).await?);
    let what = t!("noun.guild");
    match query.filter(|q| !q.trim().is_empty()) {
        // 페어링된 서버가 하나뿐이면 묻지 않는다
        None if items.len() == 1 => Ok(items[0].0.clone()),
        None => choose(ctx, &what, items, guild_label).map(|g| g.0),
        Some(q) => {
            let found = find(&items, q, |g| vec![g.0.clone(), g.1.clone()], guild_label);
            settle(ctx, &what, q, found, guild_label).map(|g| g.0)
        }
    }
}

/// 멤버 목록 (맨 목록이거나 `{members, partial}`)
pub fn member_items(r: &Value) -> Vec<Value> {
    r.get("members").or(Some(r)).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn member_names(m: &Value) -> Vec<String> {
    ["displayName", "username", "id"].iter().map(|k| s(m, k).to_string()).filter(|n| !n.is_empty()).collect()
}

pub fn member_label(m: &Value) -> String {
    let name = [s(m, "displayName"), s(m, "username")].into_iter().find(|n| !n.is_empty()).unwrap_or("");
    match (name, s(m, "username")) {
        ("", _) => s(m, "id").to_string(),
        (n, u) if !u.is_empty() && u != n => format!("{} (@{})", n, u),
        (n, _) => n.to_string(),
    }
}

/// 멤버 이름(표시 이름·사용자 이름, 앞의 `@` 무시)이나 id → 디스코드 id.
/// 권한을 줄 사람이라 이름이 정확히(대소문자 무시) 맞아야 한다 (`find_exact`, 멤버 목록이 일부만 와도 다른 사람을 고르지 않게)
pub async fn member(ctx: &Ctx, guild: &str, query: &str) -> Result<String> {
    let q = query.trim().trim_start_matches('@');
    if let Some(id) = discord_id(q) {
        return Ok(id.to_string());
    }
    let items = member_items(&crate::cmd::bot::members(ctx, guild).await?);
    let found = find_exact(&items, q, member_names, member_label);
    settle(ctx, &t!("noun.member"), q, found, member_label).map(|m| s(&m, "id").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discord_ids_are_used_as_given_and_names_are_looked_up() {
        assert_eq!(discord_id(" 332007306403250178 "), Some("332007306403250178"));
        assert_eq!(discord_id("1234"), None);
        assert_eq!(discord_id("사바 서버"), None);
        let nodes = json!({ "nodes": [{ "guildId": "111111111111111111", "guildName": "사바 서버" }, { "guildId": "222222222222222222", "guildName": "친구들" }] });
        let items = guild_items(&nodes);
        let hit = |q| match find(&items, q, |g| vec![g.0.clone(), g.1.clone()], guild_label) {
            Found::One(g) => Some(g.0),
            _ => None,
        };
        assert_eq!(hit("친구").as_deref(), Some("222222222222222222"));
        assert_eq!(hit("사바 서버").as_deref(), Some("111111111111111111"));
        assert_eq!(guild_items(&json!([{ "guildId": "1", "guildName": "a" }])).len(), 1);
    }

    #[test]
    fn members_are_found_by_display_or_user_name() {
        let r = json!({ "members": [
            { "id": "1", "username": "kimchi", "displayName": "김치마요" },
            { "id": "2", "username": "saba", "displayName": "saba" }
        ], "partial": false });
        let items = member_items(&r);
        let hit = |q: &str| match find(&items, q.trim_start_matches('@'), member_names, member_label) {
            Found::One(m) => Some(s(&m, "id").to_string()),
            _ => None,
        };
        assert_eq!(hit("@kimchi").as_deref(), Some("1"));
        assert_eq!(hit("김치마요").as_deref(), Some("1"));
        assert_eq!(member_label(&items[0]), "김치마요 (@kimchi)");
        assert_eq!(member_label(&items[1]), "saba");
        assert_eq!(member_items(&json!([{ "id": "3" }])).len(), 1);
    }

    fn names(v: &&str) -> Vec<String> {
        vec![v.to_string()]
    }

    fn one(f: Found<&'static str>) -> Option<&'static str> {
        match f {
            Found::One(x) => Some(x),
            _ => None,
        }
    }

    #[test]
    fn exact_beats_prefix() {
        let items = ["pal", "palworld"];
        assert_eq!(one(find(&items, "pal", names, |s| s.to_string())), Some("pal"));
    }

    #[test]
    fn case_insensitive_and_prefix() {
        let items = ["Palworld", "Minecraft"];
        assert_eq!(
            one(find(&items, "palworld", names, |s| s.to_string())),
            Some("Palworld")
        );
        assert_eq!(one(find(&items, "mine", names, |s| s.to_string())), Some("Minecraft"));
        assert_eq!(one(find(&items, "craft", names, |s| s.to_string())), Some("Minecraft"));
    }

    #[test]
    fn ambiguous_prefix_returns_all() {
        let items = ["survival-1", "survival-2"];
        assert!(matches!(find(&items, "surv", names, |s| s.to_string()), Found::Many(v) if v.len() == 2));
    }

    #[test]
    fn typo_gets_suggestion() {
        let items = ["minecraft", "palworld"];
        match find(&items, "minecarft", names, |s| s.to_string()) {
            Found::None(s) => assert_eq!(s, vec!["minecraft".to_string()]),
            _ => panic!("expected no match"),
        }
    }

    #[test]
    fn korean_alias_matches_module() {
        let m = serde_json::json!({ "name": "minecraft", "module_aliases": ["마인크래프트", "마크"] });
        let items = [m];
        assert!(matches!(
            find(&items, "마크", module_names, |_| String::new()),
            Found::One(_)
        ));
    }

    /// 지우기·권한 대상은 앞부분·포함 일치로 고르지 않는다: "bob"이 "bobby"를 가리키면 고르지 않고 제안으로 보인다
    #[test]
    fn exact_targets_are_not_picked_by_a_partial_match() {
        let items = ["bobby", "Saba"];
        assert_eq!(one(find_exact(&items, "saba", names, |s| s.to_string())), Some("Saba"));
        match find_exact(&items, "bob", names, |s| s.to_string()) {
            Found::None(s) => assert_eq!(s, vec!["bobby".to_string()]),
            _ => panic!("a prefix match must not be picked"),
        }
        let r = json!({ "members": [{ "id": "1", "username": "bobby", "displayName": "kimbob" }] });
        assert!(matches!(find_exact(&member_items(&r), "bob", member_names, member_label), Found::None(s) if s == vec!["kimbob (@bobby)".to_string()]));
    }

    #[test]
    fn edit_distance() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "abc"), 3);
    }
}
