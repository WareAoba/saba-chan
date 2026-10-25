//! 인스턴스별 디스코드 낱말 (봇 설정 `serverWords[instanceId]`)
//!
//! 한 서버를 부르는 낱말(`words`)과, 그 서버의 명령을 부르는 낱말(`commands`)을
//! 저장·검사한다. 낱말은 인스턴스에 딸린다: 어느 디스코드 서버(길드)에서 불러도 같다.
//! 해석은 `chat`이, 저장은 `/api/instance/:id/words`가 한다.

use super::api::{ModuleView, ServerView};
use super::aliases::BUILTIN_COMMANDS;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// 0.1.10까지 길드마다 낱말을 두던 `nodeSettings[guild]` 안의 키. 시작할 때 `serverWords`로 옮긴다 (`migrate`)
pub const INSTANCES_KEY: &str = "instances";
/// 낱말 하나의 최대 글자 수와 항목마다의 최대 개수
pub const WORD_MAX_CHARS: usize = 32;
pub const WORDS_MAX: usize = 10;
/// 최상위 명령이라 서버·명령 낱말로 쓸 수 없는 것
const RESERVED: &[&str] = &["help", "도움", "도움말", "list", "목록"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceWords {
    pub words: Vec<String>,
    /// 명령 정식 이름 → 그 명령을 부르는 낱말
    pub commands: BTreeMap<String, Vec<String>>,
}

impl InstanceWords {
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.commands.values().all(Vec::is_empty)
    }
}

/// 0.1.10까지의 한 길드 인스턴스별 낱말
fn of_guild(node_settings: &Value, guild: &str) -> HashMap<String, InstanceWords> {
    node_settings
        .get(guild)
        .and_then(|n| n.get(INSTANCES_KEY))
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(id, v)| serde_json::from_value(v.clone()).ok().map(|w| (id.clone(), w))).collect())
        .unwrap_or_default()
}

pub fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

/// 이 낱말로 부를 명령 (그 인스턴스의 `commands`)
pub fn command_for<'a>(words: Option<&'a InstanceWords>, input: &str) -> Option<&'a str> {
    words?.commands.iter().find(|(_, list)| list.iter().any(|w| same(w, input))).map(|(cmd, _)| cmd.as_str())
}

/// 검사 결과 한 줄 (`field`는 `words[0]`·`commands.kick[1]` 꼴)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub field: String,
    pub reason: &'static str,
    /// 문제가 된 낱말 (목록 전체의 문제면 없음)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_with: Option<String>,
}

/// 이 인스턴스의 낱말을 검사한다. `servers`는 모든 서버(낱말은 어느 길드에서나 쓰이므로), `others`는 다른 인스턴스의 낱말
pub fn validate(
    instance: &ServerView,
    module: Option<&ModuleView>,
    want: &InstanceWords,
    servers: &[ServerView],
    modules: &[ModuleView],
    others: &HashMap<String, InstanceWords>,
) -> Vec<Problem> {
    let mut out = Vec::new();
    let shape = |field: String, word: &str, out: &mut Vec<Problem>| {
        if word.is_empty() || word.chars().count() > WORD_MAX_CHARS || word.chars().any(char::is_whitespace) {
            out.push(Problem { field, reason: "invalid", word: Some(word.to_string()), conflict_with: None });
            return false;
        }
        if RESERVED.iter().any(|r| same(r, word)) {
            out.push(Problem { field, reason: "reserved", word: Some(word.to_string()), conflict_with: None });
            return false;
        }
        true
    };

    if want.words.len() > WORDS_MAX {
        out.push(Problem { field: "words".into(), reason: "too_many", word: None, conflict_with: None });
    }
    for (i, word) in want.words.iter().enumerate() {
        let field = format!("words[{}]", i);
        if !shape(field.clone(), word, &mut out) {
            continue;
        }
        if want.words[..i].iter().any(|w| same(w, word)) {
            // 같은 서버의 낱말끼리 겹침
            out.push(Problem { field, reason: "conflict", word: Some(word.clone()), conflict_with: Some(instance.name.clone()) });
            continue;
        }
        // 다른 서버의 이름·id·낱말, 모듈 이름·별명과 겹치면 어느 서버인지 알 수 없다
        let clash = servers
            .iter()
            .filter(|s| s.id != instance.id)
            .find_map(|s| {
                let theirs = others.get(&s.id).map(|w| w.words.as_slice()).unwrap_or_default();
                (same(&s.name, word) || same(&s.id, word) || theirs.iter().any(|w| same(w, word))).then(|| s.name.clone())
            })
            .or_else(|| modules.iter().find(|m| same(&m.name, word) || m.module_aliases.iter().any(|a| same(a, word))).map(|m| m.name.clone()));
        if let Some(with) = clash {
            out.push(Problem { field, reason: "conflict", word: Some(word.clone()), conflict_with: Some(with) });
        }
    }

    for (cmd, list) in &want.commands {
        let known = BUILTIN_COMMANDS.contains(&cmd.as_str()) || module.is_some_and(|m| m.command(cmd).is_some());
        if !known {
            out.push(Problem { field: format!("commands.{}", cmd), reason: "unknown_command", word: None, conflict_with: None });
            continue;
        }
        if list.len() > WORDS_MAX {
            out.push(Problem { field: format!("commands.{}", cmd), reason: "too_many", word: None, conflict_with: None });
        }
        for (i, word) in list.iter().enumerate() {
            let field = format!("commands.{}[{}]", cmd, i);
            if !shape(field.clone(), word, &mut out) {
                continue;
            }
            // 같은 서버의 다른 명령 이름·별명·낱말과 겹치면 안 된다
            let clash = module
                .into_iter()
                .flat_map(|m| m.commands.iter().map(|c| c.name.as_str()).chain(m.command_aliases.iter().flat_map(|(c, a)| a.iter().filter(move |_| c != cmd).map(String::as_str))))
                .chain(BUILTIN_COMMANDS.iter().copied())
                .filter(|name| name != cmd)
                .find(|name| same(name, word))
                .map(str::to_string)
                .or_else(|| {
                    want.commands.iter().filter(|(c, _)| *c != cmd).find(|(_, l)| l.iter().any(|w| same(w, word))).map(|(c, _)| c.clone())
                })
                .or_else(|| list[..i].iter().any(|w| same(w, word)).then(|| cmd.clone()));
            if let Some(with) = clash {
                out.push(Problem { field, reason: "conflict", word: Some(word.clone()), conflict_with: Some(with) });
            }
        }
    }
    out
}

/// 옮긴 결과: 길드에서 `instances`를 걷은 `nodeSettings`와 인스턴스별 낱말
pub struct Migrated {
    pub node_settings: Value,
    pub server_words: HashMap<String, InstanceWords>,
}

/// 예전 낱말을 인스턴스 낱말(`serverWords`)로 옮긴다: 0.1.10의 길드별 낱말(`nodeSettings[guild].instances`)과
/// 그 전의 전역 별명(`instanceAliases`, `moduleAliases`, `commandAliases`). `current`는 이미 옮긴 낱말(먼저 둔다).
/// 같은 서버의 낱말은 길드 이름 순으로 합치고, 다른 서버 이름·id·낱말과 겹치는 낱말은 버린다 (낱말끼리는 id 순으로 먼저 온 서버가 갖는다).
/// 옮길 것이 없으면 None
pub fn migrate(
    node_settings: &Value,
    servers: &[ServerView],
    current: &HashMap<String, InstanceWords>,
    instance_aliases: &HashMap<String, String>,
    module_aliases: &HashMap<String, String>,
    command_aliases: &HashMap<String, Value>,
) -> Option<Migrated> {
    let mut guilds: Vec<&String> = node_settings.as_object().into_iter().flatten().filter(|(_, n)| n.get(INSTANCES_KEY).is_some()).map(|(g, _)| g).collect();
    guilds.sort();
    if guilds.is_empty() && instance_aliases.is_empty() && module_aliases.is_empty() && command_aliases.is_empty() {
        return None;
    }
    let split = |s: &str| -> Vec<String> { s.split(',').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect() };
    let mut table = current.clone();
    let add_word = |list: &mut Vec<String>, w: String| {
        let shape = !w.is_empty() && w.chars().count() <= WORD_MAX_CHARS && !w.chars().any(char::is_whitespace);
        if shape && !list.iter().any(|e| same(e, &w)) && list.len() < WORDS_MAX {
            list.push(w);
        }
    };
    // 명령 낱말은 같은 인스턴스의 다른 명령이 이미 가졌으면 버린다 (먼저 둔 명령이 갖는다)
    let add_command_word = |commands: &mut BTreeMap<String, Vec<String>>, cmd: &str, w: String| {
        if commands.iter().any(|(c, l)| c != cmd && l.iter().any(|e| same(e, &w))) {
            tracing::info!("[Chat] word '{}' of command {} dropped (another command has it)", w, cmd);
            return;
        }
        add_word(commands.entry(cmd.to_string()).or_default(), w);
    };
    let known = |id: &str| servers.iter().any(|s| s.id == id);
    for guild in &guilds {
        let mut old: Vec<(String, InstanceWords)> = of_guild(node_settings, guild).into_iter().filter(|(id, _)| known(id)).collect();
        old.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, w) in old {
            let entry = table.entry(id).or_default();
            for word in w.words {
                add_word(&mut entry.words, word);
            }
            for (cmd, list) in w.commands {
                for word in list {
                    add_command_word(&mut entry.commands, &cmd, word);
                }
            }
        }
    }
    for s in servers {
        let entry = table.entry(s.id.clone()).or_default();
        let mut words: Vec<String> = instance_aliases.get(&s.id).map(|a| split(a)).unwrap_or_default();
        // 모듈 별명은 그 모듈 인스턴스가 하나뿐일 때만 그 인스턴스 것이 된다
        if servers.iter().filter(|o| o.module == s.module).count() == 1 {
            words.extend(module_aliases.get(&s.module).map(|a| split(a)).unwrap_or_default());
        } else if module_aliases.contains_key(&s.module) {
            tracing::info!("[Chat] module aliases of {} dropped (several instances)", s.module);
        }
        for w in words {
            add_word(&mut entry.words, w);
        }
        for (cmd, list) in command_aliases.get(&s.module).and_then(Value::as_object).into_iter().flatten() {
            for w in list.as_str().map(split).unwrap_or_default() {
                add_command_word(&mut entry.commands, cmd, w);
            }
        }
    }
    // 다른 서버 이름·id·낱말과 겹치는 낱말은 버린다 (낱말끼리는 먼저 온 서버가 갖는다)
    let mut taken: Vec<String> = Vec::new();
    let mut ids: Vec<String> = table.keys().cloned().collect();
    ids.sort();
    let mut server_words: HashMap<String, InstanceWords> = HashMap::new();
    for id in ids {
        let mut w = table.remove(&id).unwrap_or_default();
        w.words.retain(|word| {
            let other_server = servers.iter().filter(|s| s.id != id).any(|s| same(&s.name, word) || same(&s.id, word));
            let free = !RESERVED.iter().any(|r| same(r, word)) && !other_server && !taken.iter().any(|t| same(t, word));
            if !free {
                tracing::info!("[Chat] word '{}' of instance {} dropped (conflict)", word, id);
            }
            free
        });
        taken.extend(w.words.iter().cloned());
        w.commands.retain(|_, l| !l.is_empty());
        if !w.is_empty() {
            server_words.insert(id, w);
        }
    }
    let mut nodes = node_settings.as_object().cloned().unwrap_or_default();
    for guild in guilds {
        if let Some(node) = nodes.get_mut(guild.as_str()).and_then(Value::as_object_mut) {
            node.remove(INSTANCES_KEY);
        }
    }
    Some(Migrated { node_settings: Value::Object(nodes), server_words })
}

/// heartbeat `serverWords` `{instanceId: {words, commands}}`: 낱말이 있는 인스턴스만 (id 순 200개까지, 릴레이 MAX_SERVERS)
pub fn heartbeat(server_words: &HashMap<String, InstanceWords>) -> Value {
    let mut table: Vec<(&String, &InstanceWords)> = server_words.iter().filter(|(_, w)| !w.is_empty()).collect();
    table.sort_by(|a, b| a.0.cmp(b.0));
    Value::Object(table.into_iter().take(200).map(|(id, w)| (id.clone(), serde_json::to_value(w).unwrap_or_default())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn server(id: &str, name: &str, module: &str) -> ServerView {
        ServerView { id: id.into(), name: name.into(), module: module.into(), status: "stopped".into(), pid: None }
    }

    #[test]
    fn words_that_would_point_at_two_servers_are_refused() {
        let servers = vec![server("a", "survival", "minecraft"), server("b", "creative", "minecraft")];
        let mc = ModuleView { name: "minecraft".into(), module_aliases: vec!["mc".into()], command_aliases: vec![("kick".into(), vec!["추방".into()])], ..Default::default() };
        let others = HashMap::from([("b".to_string(), InstanceWords { words: vec!["크리".into()], ..Default::default() })]);
        let want = InstanceWords {
            words: vec!["서바".into(), "creative".into(), "크리".into(), "mc".into(), "도움".into(), "두 낱말".into()],
            commands: BTreeMap::from([("start".to_string(), vec!["켜".into(), "추방".into()]), ("nope".to_string(), vec!["x".into()]), ("restart".to_string(), vec!["재시작".into()])]),
        };
        let problems = validate(&servers[0], Some(&mc), &want, &servers, std::slice::from_ref(&mc), &others);
        let got: Vec<(&str, &str)> = problems.iter().map(|p| (p.field.as_str(), p.reason)).collect();
        assert_eq!(
            got,
            vec![
                ("words[1]", "conflict"),
                ("words[2]", "conflict"),
                ("words[3]", "conflict"),
                ("words[4]", "reserved"),
                ("words[5]", "invalid"),
                ("commands.nope", "unknown_command"),
                ("commands.restart", "unknown_command"),
                ("commands.start[1]", "conflict"),
            ]
        );
    }

    #[test]
    fn old_guild_words_and_global_aliases_move_to_the_instance() {
        let servers = vec![server("a", "survival", "minecraft"), server("b", "creative", "minecraft"), server("p", "pal", "palworld")];
        let node_settings = json!({
            "g1": { "defaultLevel": 1, "instances": { "b": { "words": ["크리"], "commands": { "kick": ["내보내"] } }, "gone": { "words": ["x"] } } },
            "g2": { "instances": { "b": { "words": ["크리", "cr2"] }, "a": { "words": ["크리", "서바"] } } },
        });
        let moved = migrate(
            &node_settings,
            &servers,
            &HashMap::new(),
            &HashMap::from([("b".to_string(), "cr, p, 두 낱말".to_string())]),
            &HashMap::from([("minecraft".to_string(), "마크".to_string()), ("palworld".to_string(), "팰".to_string())]),
            &HashMap::from([("minecraft".to_string(), json!({ "kick": "추방, 킥" }))]),
        )
        .unwrap();
        let w = &moved.server_words;
        // 길드마다 두었던 낱말을 길드 이름 순으로 합친다. 두 서버가 같은 낱말을 썼으면 id 순으로 먼저 온 서버("a")가 갖는다
        assert_eq!(w["a"].words, vec!["크리", "서바"]);
        // 다른 서버의 id("p")와 같은 낱말, 규칙에 맞지 않는 낱말도 버린다
        assert_eq!(w["b"].words, vec!["cr2", "cr"]);
        assert_eq!(w["b"].commands["kick"], vec!["내보내", "추방", "킥"]);
        assert_eq!(w["a"].commands["kick"], vec!["추방", "킥"]);
        // 인스턴스가 둘인 모듈의 별명은 어느 쪽 것인지 몰라 버린다
        assert_eq!(w["p"].words, vec!["팰"]);
        // 지워진 인스턴스의 낱말은 옮기지 않고, 길드의 다른 설정은 남긴다
        assert!(!w.contains_key("gone"));
        assert_eq!(moved.node_settings, json!({ "g1": { "defaultLevel": 1 }, "g2": {} }));
        assert!(migrate(&moved.node_settings, &servers, w, &HashMap::new(), &HashMap::new(), &HashMap::new()).is_none());
    }

    #[test]
    fn one_word_never_ends_up_on_two_commands_of_an_instance() {
        let servers = vec![server("a", "survival", "minecraft")];
        let node_settings = json!({
            "g1": { "instances": { "a": { "commands": { "stop": ["토글"] } } } },
            "g2": { "instances": { "a": { "commands": { "start": ["토글", "켜"] } } } },
        });
        let moved = migrate(&node_settings, &servers, &HashMap::new(), &HashMap::new(), &HashMap::new(), &HashMap::from([("minecraft".to_string(), json!({ "start": "켜", "stop": "켜, 꺼" }))])).unwrap();
        let w = &moved.server_words["a"];
        // 길드 이름 순으로 먼저 둔 명령이 낱말을 갖는다
        assert_eq!(w.commands["stop"], vec!["토글", "꺼"]);
        assert_eq!(w.commands["start"], vec!["켜"]);
    }

    #[test]
    fn heartbeat_carries_only_instances_with_words() {
        let words = HashMap::from([("b".to_string(), InstanceWords { words: vec!["크리".into()], ..Default::default() }), ("a".to_string(), InstanceWords::default())]);
        assert_eq!(heartbeat(&words), json!({ "b": { "words": ["크리"], "commands": {} } }));
    }
}
