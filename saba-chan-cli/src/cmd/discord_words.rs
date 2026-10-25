//! `server discord`: 디스코드에서 이 서버와 그 명령을 채팅으로 부르는 낱말 (인스턴스에 딸린다, 봇 설정 `serverWords[id]`)
//!
//! 데몬 `GET`·`PUT /api/instance/:id/words`. 낱말 규칙·충돌은 데몬이 검사한다.

use crate::ctx::Ctx;
use crate::error::{usage, Result};
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

/// 옵션 없이: 이 서버의 낱말과 명령별 낱말을 보여 준다. 바꿀 것이 있으면 지금 값에서 시작해 저장한다
pub async fn run(ctx: &Ctx, server: Option<&str>, change: WordsChange) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    let entry = ctx.api.instance_words(&sv.id).await?;
    if change.is_empty() {
        ctx.out.data(&entry, |e| {
            // 정하지 않아도 부르는 낱말(인스턴스 이름, 게임이 하나뿐이면 게임명)은 GUI처럼 흐리게 덧붙인다
            let mut words = str_list(e.get("words")).join(", ");
            let defaults = str_list(e.get("defaultWords"));
            if !defaults.is_empty() {
                words = format!("{} {}", words, output::dim(&format!("({})", defaults.join(", ")))).trim().to_string();
            }
            let mut rows = vec![(t!("bot.words"), words)];
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
    let r = ctx.api.save_instance_words(&sv.id, body).await?;
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
}
