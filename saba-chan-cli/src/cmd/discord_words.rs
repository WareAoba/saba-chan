//! `server discord`: 디스코드 서버(길드)마다 이 서버와 그 명령을 채팅에서 부르는 낱말 (`nodeSettings[guild].instances[id]`)
//!
//! 데몬 `GET /api/guilds/:guild/instances`, `PUT /api/guilds/:guild/instances/:id/words`. 낱말 규칙·충돌은 데몬이 검사한다.

use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use crate::output::{self, s};
use crate::resolve;
use serde_json::{json, Map, Value};

pub struct WordsChange {
    /// `Some`: 서버 낱말을 이것으로 바꾼다 (빈 값만 있으면 지운다)
    pub words: Option<Vec<String>>,
    /// `명령=낱말,낱말`: 그 명령만 바꾼다 (`명령=`은 지운다)
    pub commands: Vec<String>,
    pub clear: bool,
}

impl WordsChange {
    fn is_empty(&self) -> bool {
        self.words.is_none() && self.commands.is_empty() && !self.clear
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

fn clean_words<'a>(raw: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    raw.into_iter().map(str::trim).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// PUT 본문 `{words, commands}`. 지금 값에서 시작해 준 것만 바꾼다. 낱말 규칙·충돌은 데몬이 검사한다
fn words_body(entry: &Value, change: &WordsChange) -> Result<Value> {
    if change.clear {
        return Ok(json!({ "words": [], "commands": {} }));
    }
    let words = match &change.words {
        Some(w) => clean_words(w.iter().map(String::as_str)),
        None => str_list(entry.get("words")),
    };
    let known: Vec<Value> = entry.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut commands = Map::new();
    for c in &known {
        let aliases = str_list(c.get("aliases"));
        if !aliases.is_empty() {
            commands.insert(s(c, "name").to_string(), json!(aliases));
        }
    }
    for spec in &change.commands {
        let (name, list) = spec.split_once('=').ok_or_else(|| usage(t!("bot.words_bad_command", spec = spec)))?;
        let name = name.trim();
        let canonical = match known.iter().find(|c| s(c, "name").eq_ignore_ascii_case(name)) {
            Some(c) => s(c, "name").to_string(),
            None => {
                let suggestions = match crate::resolve::find(&known, name, |c| vec![s(c, "name").to_string()], |c| s(c, "name").to_string()) {
                    crate::resolve::Found::None(close) => close,
                    crate::resolve::Found::One(c) => vec![s(&c, "name").to_string()],
                    crate::resolve::Found::Many(list) => list.iter().take(3).map(|c| s(c, "name").to_string()).collect(),
                };
                return Err(crate::error::CliError::NotFound { what: t!("noun.command"), query: name.to_string(), suggestions }.into());
            }
        };
        let list = clean_words(list.split(','));
        if list.is_empty() {
            commands.remove(&canonical);
        } else {
            commands.insert(canonical, json!(list));
        }
    }
    Ok(json!({ "words": words, "commands": commands }))
}

/// 목록은 배열 그대로이거나 `{instances: [...]}`
fn instance_list(r: &Value) -> Vec<Value> {
    r.get("instances").or(Some(r)).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// `명령: 낱말, 낱말` — 사용자 낱말만 (module.toml 별명은 기본값이라 뺀다)
fn command_words_line(entry: &Value) -> String {
    entry
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let a = str_list(c.get("aliases"));
            (!a.is_empty()).then(|| format!("{}: {}", s(c, "name"), a.join(", ")))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// 그 디스코드 서버가 쓰는 서버 중 이 서버 (`allowedInstances`에 없으면 None)
async fn entry_in(ctx: &Ctx, guild: &str, id: &str) -> Result<Option<Value>> {
    Ok(instance_list(&ctx.api.guild_instances(guild).await?).into_iter().find(|i| s(i, "id") == id))
}

/// 옵션 없이: 페어링된 디스코드 서버마다 보기. `--guild`만: 그 디스코드 서버의 명령별 낱말. 바꿀 것이 있으면 저장한다
pub async fn run(ctx: &Ctx, server: Option<&str>, guild: Option<&str>, change: WordsChange) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    if guild.is_none() && change.is_empty() {
        let mut rows = Vec::new();
        for (id, name) in resolve::guild_items(&crate::cmd::bot::paired_nodes(ctx).await?) {
            let entry = entry_in(ctx, &id, &sv.id).await?;
            rows.push(json!({ "guildId": id, "guildName": name, "instance": entry }));
        }
        ctx.out.data(&Value::Array(rows.clone()), |_| {
            if rows.is_empty() {
                say!("{}", output::dim(&t!("bot.list_empty")));
                return;
            }
            let mut t = output::table(&[t!("noun.guild"), t!("bot.words"), t!("bot.command_words")]);
            for r in &rows {
                let name = [s(r, "guildName"), s(r, "guildId")].into_iter().find(|n| !n.is_empty()).unwrap_or("").to_string();
                match r.get("instance").filter(|e| !e.is_null()) {
                    Some(e) => t.add_row(vec![name, str_list(e.get("words")).join(", "), command_words_line(e)]),
                    None => t.add_row(vec![name, output::dim(&t!("bot.words_not_used")), String::new()]),
                };
            }
            output::print_table(t);
        });
        return Ok(());
    }
    let guild = resolve::guild(ctx, guild).await?;
    let Some(entry) = entry_in(ctx, &guild, &sv.id).await? else {
        return Err(failed(t!("bot.words_not_used")));
    };
    if change.is_empty() {
        ctx.out.data(&entry, |e| {
            let mut rows = vec![(t!("bot.words"), str_list(e.get("words")).join(", "))];
            for c in e.get("commands").and_then(Value::as_array).into_iter().flatten() {
                let mut line = str_list(c.get("aliases")).join(", ");
                let defaults = str_list(c.get("moduleAliases"));
                if !defaults.is_empty() {
                    line = format!("{} {}", line, output::dim(&format!("({})", defaults.join(", ")))).trim().to_string();
                }
                rows.push((s(c, "name").to_string(), line));
            }
            output::print_kv(&rows);
        });
        return Ok(());
    }
    let body = words_body(&entry, &change)?;
    let r = ctx.api.save_instance_words(&guild, s(&entry, "id"), body).await?;
    ctx.out.done(&t!("bot.words_saved", server = s(&entry, "name")), &r);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Value {
        json!({
            "id": "i1", "name": "크리에이티브", "module": "minecraft", "words": ["크리"],
            "commands": [
                { "name": "start", "builtin": true, "moduleAliases": [], "aliases": ["켜"] },
                { "name": "stop", "builtin": true, "moduleAliases": [], "aliases": ["꺼"] },
                { "name": "kick", "builtin": false, "moduleAliases": ["추방"], "aliases": [] }
            ]
        })
    }

    fn change(words: Option<&[&str]>, commands: &[&str], clear: bool) -> WordsChange {
        WordsChange {
            words: words.map(|w| w.iter().map(|s| s.to_string()).collect()),
            commands: commands.iter().map(|s| s.to_string()).collect(),
            clear,
        }
    }

    #[test]
    fn a_command_change_keeps_the_other_words() {
        let body = words_body(&entry(), &change(None, &["KICK=내보내, 쫓아내", "stop="], false)).unwrap();
        assert_eq!(body, json!({ "words": ["크리"], "commands": { "start": ["켜"], "kick": ["내보내", "쫓아내"] } }));
    }

    #[test]
    fn server_words_are_replaced_and_an_empty_word_clears_them() {
        let body = words_body(&entry(), &change(Some(&["크", " 크2 "]), &[], false)).unwrap();
        assert_eq!(body["words"], json!(["크", "크2"]));
        assert_eq!(body["commands"], json!({ "start": ["켜"], "stop": ["꺼"] }));
        assert_eq!(words_body(&entry(), &change(Some(&[""]), &[], false)).unwrap()["words"], json!([]));
    }

    #[test]
    fn clear_empties_everything() {
        assert_eq!(words_body(&entry(), &change(None, &[], true)).unwrap(), json!({ "words": [], "commands": {} }));
    }

    #[test]
    fn an_unknown_command_or_a_spec_without_equals_is_refused() {
        let e = words_body(&entry(), &change(None, &["kik=추방"], false)).unwrap_err();
        match e.downcast_ref::<crate::error::CliError>() {
            Some(crate::error::CliError::NotFound { suggestions, .. }) => assert_eq!(suggestions, &vec!["kick".to_string()]),
            other => panic!("expected NotFound, got {:?}", other),
        }
        assert!(words_body(&entry(), &change(None, &["kick"], false)).is_err());
    }

    #[test]
    fn the_list_comes_bare_or_wrapped() {
        assert_eq!(instance_list(&json!([entry()])).len(), 1);
        assert_eq!(instance_list(&json!({ "instances": [entry()] })).len(), 1);
    }
}
