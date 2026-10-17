//! 사람이 친 이름으로 대상 찾기 — 서버·모듈·익스텐션 공통
//!
//! 순서: 정확히 같음 → 대소문자 무시 같음 → 앞부분 일치 → 포함. 처음으로 결과가 나온 단계에서 멈춘다.
//! 여럿이면 터미널에서는 고르게 하고, 아니면 후보를 보여 주며 실패한다.
//! 없으면 비슷한 이름을 제안한다.

use crate::ctx::Ctx;
use crate::error::{usage, CliError, Result};
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
    let q = query.trim();
    let ql = q.to_lowercase();
    type Stage = fn(&str, &str, &str) -> bool;
    let stages: [Stage; 4] = [
        |n, q, _| n == q,
        |n, _, ql| n.to_lowercase() == ql,
        |n, _, ql| n.to_lowercase().starts_with(ql),
        |n, _, ql| n.to_lowercase().contains(ql),
    ];
    for stage in stages {
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
    let all = servers(ctx).await?;
    let what = t!("noun.server");
    match query.filter(|q| !q.trim().is_empty()) {
        None => choose(ctx, &what, all, Server::label),
        Some(q) => {
            let found = find(&all, q, |s| vec![s.name.clone(), s.id.clone()], |s| s.name.clone());
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
    let all = modules(ctx).await?;
    let what = t!("noun.module");
    match query.filter(|q| !q.trim().is_empty()) {
        None => choose(ctx, &what, all, module_label),
        Some(q) => {
            let found = find(&all, q, module_names, |m| {
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
            let found = find(&all, q, names, |e| {
                e.get("id").and_then(Value::as_str).unwrap_or("?").to_string()
            });
            settle(ctx, &what, q, found, extension_label)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn edit_distance() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "abc"), 3);
    }
}
