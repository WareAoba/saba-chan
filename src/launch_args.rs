//! 실행 인자 미리보기: 사용자가 친 추가 인자를 설정 필드로 되돌린다
//!
//! 모듈은 실행 인자로 들어가는 설정 필드에 `arg` 모양을 선언한다 (`-port={value}`, `-Xmx{value}`,
//! `-servername {value}`, `{value}`가 없으면 boolean 플래그). 데몬은 이 모양으로 사용자가 친 토큰을
//! 필드 값으로 읽고, 형식은 `crate::validator`로 검사한다. 실제 실행 줄은 언제나 모듈의
//! `get_launch_command`가 만든다 (이 파일은 줄을 만들지 않는다).

use crate::supervisor::module_loader::SettingField;
use serde::Serialize;
use serde_json::{Map, Value};

/// `arg` 모양 하나: 공백으로 나눈 토큰들, 그중 하나에 `{value}`가 많아야 한 번
#[derive(Debug, Clone, PartialEq)]
pub struct ArgPattern {
    tokens: Vec<String>,
    /// `{value}`가 든 토큰의 자리와 그 앞뒤 글자
    value: Option<(usize, String, String)>,
}

impl ArgPattern {
    pub fn parse(pattern: &str) -> Result<Self, &'static str> {
        let tokens: Vec<String> = pattern.split_whitespace().map(str::to_string).collect();
        if tokens.is_empty() {
            return Err("is empty");
        }
        let mut value = None;
        for (i, t) in tokens.iter().enumerate() {
            match t.matches("{value}").count() {
                0 => {}
                1 if value.is_none() => {
                    let (pre, post) = t.split_once("{value}").expect("one {value}");
                    value = Some((i, pre.to_string(), post.to_string()));
                }
                _ => return Err("must contain {value} at most once"),
            }
        }
        // 글자 없이 `{value}`뿐이면 아무 토큰이나 이 필드로 읽힌다
        if tokens.len() == 1 && value.as_ref().is_some_and(|(_, pre, post)| pre.is_empty() && post.is_empty()) {
            return Err("needs literal text besides {value}");
        }
        Ok(Self { tokens, value })
    }

    /// `words` 앞부분이 이 모양이면 (먹은 토큰 수, `{value}` 자리의 글자)
    fn matches(&self, words: &[String]) -> Option<(usize, Option<String>)> {
        if words.len() < self.tokens.len() {
            return None;
        }
        let mut captured = None;
        for (i, (want, got)) in self.tokens.iter().zip(words).enumerate() {
            match &self.value {
                Some((at, pre, post)) if *at == i => {
                    let inner = got.strip_prefix(pre.as_str())?.strip_suffix(post.as_str())?;
                    if inner.is_empty() {
                        return None;
                    }
                    captured = Some(inner.to_string());
                }
                _ if want != got => return None,
                _ => {}
            }
        }
        Some((self.tokens.len(), captured))
    }

    /// 겹칠 때 더 구체적인 모양을 먼저 (글자가 많은 쪽)
    fn literal_len(&self) -> usize {
        self.tokens.iter().map(|t| t.replace("{value}", "").len()).sum()
    }
}

/// 형식 오류 한 건. `arg`는 친 인자(필드 초안에서 온 오류면 None), `expected`는 `validator::expected_of`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArgError {
    pub arg: Option<String>,
    pub field: String,
    pub expected: String,
    pub got: Value,
}

/// 추가 인자를 읽은 결과
#[derive(Debug, Default, PartialEq)]
pub struct Bound {
    /// 필드로 읽혔고 형식도 맞는 값
    pub settings: Map<String, Value>,
    /// 어느 필드에도 맞지 않아 그대로 붙일 인자
    pub extra: Vec<String>,
    pub errors: Vec<ArgError>,
}

/// 문자열로 읽힌 값을 필드 타입에 맞는 JSON으로
fn typed(field: &SettingField, raw: &str) -> Value {
    match field.field_type.as_str() {
        "number" => match raw.parse::<i64>() {
            Ok(n) => Value::from(n),
            Err(_) => raw.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::from(raw)),
        },
        "boolean" => match raw {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::from(raw),
        },
        _ => Value::from(raw),
    }
}

/// `arg`를 선언한 필드가 있는가 (미리보기를 보여 줄 모듈인가)
pub fn has_launch_args(fields: &[SettingField]) -> bool {
    fields.iter().any(|f| !f.arg.is_empty())
}

/// 추가 인자 `words`를 필드 `arg` 모양과 맞춰 본다. 같은 필드가 여러 번 나오면 뒤엣것이 이긴다
pub fn bind(fields: &[SettingField], words: &[String]) -> Bound {
    let mut patterns: Vec<(&SettingField, ArgPattern)> = fields
        .iter()
        .flat_map(|f| f.arg.iter().filter_map(move |a| ArgPattern::parse(a).ok().map(|p| (f, p))))
        .collect();
    patterns.sort_by_key(|(_, p)| std::cmp::Reverse(p.literal_len()));

    let mut out = Bound::default();
    let mut i = 0;
    while i < words.len() {
        let hit = patterns.iter().find_map(|(f, p)| p.matches(&words[i..]).map(|m| (*f, m)));
        let Some((field, (taken, raw))) = hit else {
            out.extra.push(words[i].clone());
            i += 1;
            continue;
        };
        let arg = words[i..i + taken].join(" ");
        i += taken;
        let value = match raw {
            Some(raw) => typed(field, &raw),
            None => Value::Bool(true),
        };
        match crate::validator::validate_setting_value(field, Some(&value)) {
            Ok(()) => {
                out.errors.retain(|e| e.field != field.name);
                out.settings.insert(field.name.clone(), value);
            }
            Err(_) => out.errors.push(ArgError {
                arg: Some(arg),
                field: field.name.clone(),
                expected: crate::validator::expected_of(field).to_string(),
                got: value,
            }),
        }
    }
    out
}

/// 설정 초안의 형식 오류 (필드를 고친 쪽에서 온 것, `arg`는 None)
pub fn draft_errors(fields: &[SettingField], draft: &Map<String, Value>) -> Vec<ArgError> {
    fields
        .iter()
        .filter_map(|f| {
            let value = draft.get(&f.name)?;
            crate::validator::validate_setting_value(f, Some(value)).err().map(|_| ArgError {
                arg: None,
                field: f.name.clone(),
                expected: crate::validator::expected_of(f).to_string(),
                got: value.clone(),
            })
        })
        .collect()
}

/// 보여 줄 한 줄: 공백·따옴표가 든 인자는 큰따옴표로 감싼다
pub fn command_line(program: &str, args: &[String]) -> String {
    let quote = |s: &str| {
        if s.is_empty() || s.contains(char::is_whitespace) || s.contains('"') {
            format!("\"{}\"", s.replace('"', "\\\""))
        } else {
            s.to_string()
        }
    };
    std::iter::once(program).chain(args.iter().map(String::as_str)).map(quote).collect::<Vec<_>>().join(" ")
}

/// 추가 인자 입력칸에 다시 보여 줄 문자열. `chat::split_quoted`로 나누면 같은 인자들이 된다:
/// 공백이 든 (또는 빈) 인자만 큰따옴표로 감싼다 (`\"` 이스케이프는 없다)
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| if a.is_empty() || a.contains(char::is_whitespace) { format!("\"{}\"", a) } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// password 필드 값은 미리보기에 내보내지 않는다
pub fn mask_secrets(fields: &[SettingField], settings: &Map<String, Value>, args: &mut [String]) {
    let mut secrets: Vec<String> = fields
        .iter()
        .filter(|f| f.field_type == "password")
        .filter_map(|f| settings.get(&f.name).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string))
        .collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for arg in args.iter_mut() {
        for s in &secrets {
            if arg.contains(s.as_str()) {
                *arg = arg.replace(s.as_str(), "***");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field(name: &str, ty: &str, arg: &[&str]) -> SettingField {
        SettingField {
            name: name.into(),
            field_type: ty.into(),
            label: name.into(),
            arg: arg.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn typed_args_become_fields_and_the_rest_stays_extra() {
        let mut ram = field("ram", "text", &["-Xmx{value}", "-Xms{value}"]);
        ram.format = Some("memory_size".into());
        let mut port = field("port", "number", &["--port={value}"]);
        port.format = Some("port".into());
        let name = field("server_name", "text", &["-servername {value}"]);
        let nogui = field("nogui", "boolean", &["nogui"]);
        let fields = vec![ram, port, name, nogui];

        let b = bind(&fields, &words("-Xmx4G --port=8211 -servername my_srv nogui -XX:+UseG1GC"));
        assert_eq!(b.settings, json!({"ram": "4G", "port": 8211, "server_name": "my_srv", "nogui": true}).as_object().unwrap().clone());
        assert_eq!(b.extra, ["-XX:+UseG1GC"]);
        assert!(b.errors.is_empty());

        // 형식이 틀리면 필드로 넣지 않고 오류로
        let b = bind(&fields, &words("-Xmx4096 --port=70000"));
        assert!(b.settings.is_empty() && b.extra.is_empty());
        assert_eq!(
            b.errors.iter().map(|e| (e.arg.as_deref().unwrap(), e.field.as_str(), e.expected.as_str())).collect::<Vec<_>>(),
            [("-Xmx4096", "ram", "memory_size"), ("--port=70000", "port", "port")]
        );
        assert_eq!(b.errors[0].got, json!("-Xmx4096".trim_start_matches("-Xmx")));

        // 뒤에 다시 맞게 쓰면 앞의 오류는 지운다
        let b = bind(&fields, &words("-Xmx4GB -Xms2G"));
        assert_eq!(b.settings.get("ram"), Some(&json!("2G")));
        assert!(b.errors.is_empty());
        assert!(!has_launch_args(&[field("x", "text", &[])]) && has_launch_args(&fields));
    }

    #[test]
    fn joined_extra_args_split_back_the_same() {
        let args: Vec<String> = ["-XX:+UseG1GC", "--motd", "My Server", "nogui"].map(String::from).to_vec();
        let text = join_args(&args);
        assert_eq!(text, "-XX:+UseG1GC --motd \"My Server\" nogui");
        assert_eq!(crate::chat::split_quoted(&text), args);
    }

    #[test]
    fn patterns_need_literal_text_and_one_value() {
        assert!(ArgPattern::parse("{value}").is_err());
        assert!(ArgPattern::parse("-a={value}{value}").is_err());
        assert!(ArgPattern::parse("").is_err());
        assert!(ArgPattern::parse("-servername {value}").is_ok());
        assert!(ArgPattern::parse("--flag").is_ok());
    }

    #[test]
    fn the_preview_line_quotes_and_masks() {
        let pw = field("admin_password", "password", &["-adminpassword {value}"]);
        let settings = json!({"admin_password": "hunter22"}).as_object().unwrap().clone();
        let mut args = vec!["-adminpassword".to_string(), "hunter22".to_string(), "C:\\My Server".to_string()];
        mask_secrets(&[pw], &settings, &mut args);
        assert_eq!(command_line("java", &args), "java -adminpassword *** \"C:\\My Server\"");
    }
}
