//! 게임 명령 카탈로그 — 모듈이 선언한 명령(`module.toml` `[[commands.fields]]`)을 CLI에서 쓰는 모양으로
//!
//! CLI는 게임별 명령을 하드코딩하지 않는다. 데몬 `GET /api/modules`가 주는 `commands`·`command_aliases`와
//! 모듈 문구(`GET /api/module/:name/locales`)로 목록·사용법·입력 칸을 만든다.
//! 인자 검사와 기본값 채우기의 최종 판단은 데몬(`src/commands`)이 하고, 여기서는 빠진 값을 물어보려고
//! 같은 규칙으로 미리 묶어 본다. 결과는 항상 이름 붙은 인자(`{"이름": 값}`)로 보낸다.
//!
//! gap(C7): 익스텐션이 선언한 명령은 아직 데몬 목록에 없다. 데몬이 인스턴스별 카탈로그
//! (`GET /api/instance/:id/commands`)를 주면 `Catalog::from_module` 대신 그것을 읽으면 된다.

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Safe,
    Caution,
    Danger,
}

impl Risk {
    /// 없으면 caution (데몬과 같다)
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("safe") => Risk::Safe,
            Some("danger") => Risk::Danger,
            _ => Risk::Caution,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Risk::Safe => "safe",
            Risk::Caution => "caution",
            Risk::Danger => "danger",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Input {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub placeholder: Option<String>,
    pub default: Option<Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub options: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub name: String,
    pub label: String,
    pub description: String,
    pub risk: Risk,
    pub method: String,
    pub aliases: Vec<String>,
    pub inputs: Vec<Input>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub commands: Vec<Command>,
    /// "console"이면 정의에 없는 한 줄도 콘솔로 보낼 수 있다
    pub interaction_mode: Option<String>,
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Catalog {
    /// `module`: `/api/modules`의 항목 하나. `locales`: 그 모듈의 현재 언어 문구 (`locales/<lang>.json`)
    pub fn from_module(module: &Value, locales: Option<&Value>) -> Self {
        let fields = module
            .pointer("/commands/fields")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let aliases = module.get("command_aliases").and_then(Value::as_object);
        let tr = |path: &str| {
            locales
                .and_then(|l| l.pointer(path))
                .and_then(Value::as_str)
                .map(str::to_string)
        };

        let commands = fields
            .iter()
            .filter_map(|f| {
                let name = str_of(f, "name")?;
                let base = format!("/commands/{}", name);
                let inputs = f
                    .get("inputs")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|i| {
                                let iname = str_of(i, "name")?;
                                let ib = format!("{}/inputs/{}", base, iname);
                                Some(Input {
                                    label: tr(&format!("{}/label", ib))
                                        .or_else(|| str_of(i, "label"))
                                        .unwrap_or_else(|| iname.clone()),
                                    placeholder: tr(&format!("{}/placeholder", ib))
                                        .or_else(|| str_of(i, "placeholder")),
                                    kind: str_of(i, "type").unwrap_or_else(|| "text".into()),
                                    required: i.get("required").and_then(Value::as_bool).unwrap_or(false),
                                    default: i.get("default").filter(|d| !d.is_null()).cloned(),
                                    min: i.get("min").and_then(Value::as_f64),
                                    max: i.get("max").and_then(Value::as_f64),
                                    options: i
                                        .get("options")
                                        .and_then(Value::as_array)
                                        .map(|o| o.iter().filter_map(Value::as_str).map(str::to_string).collect())
                                        .unwrap_or_default(),
                                    name: iname,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let alias_info = aliases.and_then(|a| a.get(&name));
                Some(Command {
                    label: tr(&format!("{}/label", base))
                        .or_else(|| str_of(f, "label"))
                        .unwrap_or_else(|| name.clone()),
                    description: tr(&format!("{}/description", base))
                        .or_else(|| str_of(f, "description"))
                        .or_else(|| alias_info.and_then(|a| str_of(a, "description")))
                        .unwrap_or_default(),
                    risk: Risk::parse(f.get("risk").and_then(Value::as_str)),
                    method: str_of(f, "method").unwrap_or_else(|| "module".into()),
                    aliases: alias_info
                        .and_then(|a| a.get("aliases"))
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                        .unwrap_or_default(),
                    inputs,
                    name,
                })
            })
            .collect();
        Self {
            commands,
            interaction_mode: str_of(module, "interaction_mode"),
        }
    }

    /// 이름(대소문자 무시) → 별명. 데몬 `commands::find`와 같은 순서.
    pub fn find(&self, name: &str) -> Option<&Command> {
        self.commands
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .or_else(|| {
                self.commands
                    .iter()
                    .find(|c| c.aliases.iter().any(|a| a.eq_ignore_ascii_case(name)))
            })
    }

    /// 정의에 없는 줄을 콘솔로 보낼 수 있는 서버인가
    pub fn accepts_raw(&self) -> bool {
        self.interaction_mode.as_deref() != Some("commands")
    }

    pub fn names(&self) -> Vec<String> {
        self.commands
            .iter()
            .flat_map(|c| std::iter::once(c.name.clone()).chain(c.aliases.iter().cloned()))
            .collect()
    }
}

impl Command {
    /// `give <player> <item> [amount]`
    pub fn usage(&self) -> String {
        let mut s = self.name.clone();
        for i in &self.inputs {
            let shown = if i.options.is_empty() {
                i.name.clone()
            } else {
                i.options.join("|")
            };
            if i.required && i.default.is_none() {
                s.push_str(&format!(" <{}>", shown));
            } else {
                s.push_str(&format!(" [{}]", shown));
            }
        }
        s
    }

    /// 입력을 칸에 묶는다. `이름=값`은 그 칸으로, 나머지는 빈 칸에 순서대로, 마지막 칸은 남은 단어를 모두 받는다.
    /// 돌려주는 것: (묶인 값, 아직 비어 있는 필수 칸)
    pub fn bind(&self, words: &[String]) -> Result<(Map<String, Value>, Vec<Input>), String> {
        let mut named = Map::new();
        let mut rest: Vec<String> = Vec::new();
        for w in words {
            match w.split_once('=') {
                Some((k, v)) if self.inputs.iter().any(|i| i.name.eq_ignore_ascii_case(k)) => {
                    let input = self.inputs.iter().find(|i| i.name.eq_ignore_ascii_case(k)).unwrap();
                    named.insert(input.name.clone(), Value::String(v.to_string()));
                }
                _ => rest.push(w.clone()),
            }
        }
        let open: Vec<&Input> = self.inputs.iter().filter(|i| !named.contains_key(&i.name)).collect();
        if !rest.is_empty() && open.is_empty() {
            return Err(t!("run.too_many_args", usage = self.usage()));
        }
        for (idx, input) in open.iter().enumerate() {
            if idx >= rest.len() {
                break;
            }
            let value = if idx == open.len() - 1 {
                rest[idx..].join(" ")
            } else {
                rest[idx].clone()
            };
            named.insert(input.name.clone(), Value::String(value));
        }

        let mut out = Map::new();
        let mut missing = Vec::new();
        for input in &self.inputs {
            match named.remove(&input.name) {
                Some(v) if !v.as_str().is_some_and(|s| s.trim().is_empty()) => {
                    out.insert(input.name.clone(), input.coerce(v)?);
                }
                _ if input.required && input.default.is_none() => missing.push(input.clone()),
                _ => {}
            }
        }
        Ok((out, missing))
    }
}

impl Input {
    /// 숫자·선택지 검사 (데몬과 같은 규칙). 선택지는 대소문자를 무시해 원래 표기로 맞춘다.
    pub fn coerce(&self, v: Value) -> Result<Value, String> {
        let text = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
        if self.kind == "number" {
            let n: f64 = text
                .trim()
                .parse()
                .map_err(|_| t!("run.must_be_number", label = self.label))?;
            if self.min.is_some_and(|m| n < m) || self.max.is_some_and(|m| n > m) {
                let fmt = |v: Option<f64>, inf: &str| v.map(|x| x.to_string()).unwrap_or_else(|| inf.into());
                return Err(t!(
                    "run.out_of_range",
                    label = self.label,
                    min = fmt(self.min, "-∞"),
                    max = fmt(self.max, "∞")
                ));
            }
            return Ok(if n.fract() == 0.0 && n.abs() < 9.0e15 {
                Value::from(n as i64)
            } else {
                Value::from(n)
            });
        }
        if !self.options.is_empty() {
            return self
                .options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(&text))
                .map(|o| Value::String(o.clone()))
                .ok_or_else(|| {
                    t!(
                        "run.must_be_one_of",
                        label = self.label,
                        options = self.options.join(", ")
                    )
                });
        }
        Ok(Value::String(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn module() -> Value {
        json!({
            "name": "minecraft",
            "interaction_mode": "console",
            "commands": { "fields": [
                { "name": "give", "label": "Give", "method": "rcon", "risk": "caution",
                  "inputs": [
                    { "name": "player", "required": true },
                    { "name": "item", "required": true },
                    { "name": "amount", "type": "number", "min": 1, "max": 64, "default": 1 }
                  ] },
                { "name": "say", "label": "Say", "method": "rcon", "risk": "safe",
                  "inputs": [ { "name": "message", "required": true } ] },
                { "name": "difficulty", "label": "Difficulty", "risk": "danger",
                  "inputs": [ { "name": "level", "options": ["peaceful", "easy", "hard"], "required": true } ] },
                { "name": "save-all", "label": "Save" }
            ] },
            "command_aliases": { "give": { "aliases": ["지급", "줘"], "description": "Give an item" } }
        })
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn finds_by_name_and_korean_alias() {
        let cat = Catalog::from_module(&module(), None);
        assert_eq!(cat.find("GIVE").unwrap().name, "give");
        assert_eq!(cat.find("지급").unwrap().name, "give");
        assert!(cat.find("nope").is_none());
        assert!(cat.accepts_raw());
    }

    #[test]
    fn positional_binding_last_takes_rest() {
        let cat = Catalog::from_module(&module(), None);
        let (args, missing) = cat.find("say").unwrap().bind(&words("hello there world")).unwrap();
        assert_eq!(args["message"], "hello there world");
        assert!(missing.is_empty());
    }

    #[test]
    fn named_and_positional_mix_and_numbers_coerce() {
        let cat = Catalog::from_module(&module(), None);
        let (args, _) = cat
            .find("give")
            .unwrap()
            .bind(&words("amount=5 Steve diamond"))
            .unwrap();
        assert_eq!(args["player"], "Steve");
        assert_eq!(args["item"], "diamond");
        assert_eq!(args["amount"], 5);
    }

    #[test]
    fn reports_missing_required_inputs() {
        let cat = Catalog::from_module(&module(), None);
        let (_, missing) = cat.find("give").unwrap().bind(&words("Steve")).unwrap();
        assert_eq!(
            missing.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            vec!["item"]
        );
    }

    #[test]
    fn validates_range_and_options() {
        let cat = Catalog::from_module(&module(), None);
        assert!(cat.find("give").unwrap().bind(&words("Steve dirt 99")).is_err());
        let (args, _) = cat.find("difficulty").unwrap().bind(&words("HARD")).unwrap();
        assert_eq!(args["level"], "hard");
        assert!(cat.find("difficulty").unwrap().bind(&words("insane")).is_err());
        assert!(cat.find("save-all").unwrap().bind(&words("extra")).is_err());
    }

    #[test]
    fn usage_marks_optional_inputs() {
        let cat = Catalog::from_module(&module(), None);
        assert_eq!(cat.find("give").unwrap().usage(), "give <player> <item> [amount]");
        assert_eq!(
            cat.find("difficulty").unwrap().usage(),
            "difficulty <peaceful|easy|hard>"
        );
        assert_eq!(cat.find("give").unwrap().risk, Risk::Caution);
        assert_eq!(cat.find("save-all").unwrap().risk, Risk::Caution);
    }

    #[test]
    fn module_locales_override_labels() {
        let locales = json!({ "commands": { "give": { "label": "아이템 지급", "inputs": { "player": { "label": "플레이어" } } } } });
        let cat = Catalog::from_module(&module(), Some(&locales));
        let give = cat.find("give").unwrap();
        assert_eq!(give.label, "아이템 지급");
        assert_eq!(give.inputs[0].label, "플레이어");
        assert_eq!(give.description, "Give an item");
    }
}
