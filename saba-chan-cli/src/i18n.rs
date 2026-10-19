//! CLI 문구 — `locales/<lang>/cli.json`
//!
//! en·ko는 실행 파일에 묶어 둔다. 설치된 `locales/<lang>/cli.json`(업데이터가 갱신)이
//! 있으면 그쪽이 먼저다. 언어 파일에 없는 키는 영어, 영어에도 없으면 키 그대로 보인다.
//! 새 언어는 `locales/<lang>/cli.json` 하나만 두면 된다.

use serde_json::Value;
use std::sync::OnceLock;

const EMBEDDED: &[(&str, &str)] = &[
    ("en", include_str!("../../locales/en/cli.json")),
    ("ko", include_str!("../../locales/ko/cli.json")),
];

struct Catalog {
    lang: String,
    primary: Value,
    fallback: Value,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// "ko-KR", "ko_KR.UTF-8" → "ko". 지역이 의미 있는 언어(zh-CN, pt-BR)는 그대로 둔다.
pub fn normalize(lang: &str) -> String {
    let base = lang.split('.').next().unwrap_or("").replace('_', "-");
    let lower = base.to_ascii_lowercase();
    match lower.as_str() {
        "" | "c" | "posix" => "en".into(),
        "zh-cn" | "zh-sg" | "zh-hans" => "zh-CN".into(),
        "zh-tw" | "zh-hk" | "zh-hant" => "zh-TW".into(),
        "pt-br" => "pt-BR".into(),
        _ => lower.split('-').next().unwrap_or("en").to_string(),
    }
}

fn installed(lang: &str) -> Option<Value> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let text = std::fs::read_to_string(dir.join("locales").join(lang).join("cli.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn embedded(lang: &str) -> Option<Value> {
    EMBEDDED
        .iter()
        .find(|(l, _)| *l == lang)
        .and_then(|(_, text)| serde_json::from_str(text).ok())
}

fn load(lang: &str) -> Value {
    installed(lang).or_else(|| embedded(lang)).unwrap_or(Value::Null)
}

/// 한 번만 부른다. 두 번째부터는 무시된다.
pub fn init(lang: &str) {
    let lang = normalize(lang);
    let _ = CATALOG.set(Catalog {
        primary: load(&lang),
        fallback: load("en"),
        lang,
    });
}

pub fn lang() -> &'static str {
    CATALOG.get().map(|c| c.lang.as_str()).unwrap_or("en")
}

/// 점으로 나눠 찾는다. 남은 부분이 통째로 키인 경우도 본다 (`help.cmd`의 "server.start" 같은 키)
fn lookup<'v>(root: &'v Value, key: &str) -> Option<&'v str> {
    if let Some(s) = root.get(key).and_then(Value::as_str) {
        return Some(s);
    }
    let (head, rest) = key.split_once('.')?;
    lookup(root.get(head)?, rest)
}

pub fn t(key: &str) -> String {
    let Some(cat) = CATALOG.get() else {
        return key.to_string();
    };
    lookup(&cat.primary, key)
        .or_else(|| lookup(&cat.fallback, key))
        .map(str::to_string)
        .unwrap_or_else(|| key.to_string())
}

/// 키가 없으면 None (도움말처럼 없으면 원래 문구를 쓰는 곳용)
pub fn try_t(key: &str) -> Option<String> {
    let cat = CATALOG.get()?;
    lookup(&cat.primary, key)
        .or_else(|| lookup(&cat.fallback, key))
        .map(str::to_string)
}

/// `{{name}}` 자리를 채운다
pub fn fill(template: &str, vars: &[(&str, String)]) -> String {
    vars.iter().fold(template.to_string(), |s, (k, v)| {
        s.replace(&format!("{{{{{}}}}}", k), v)
    })
}

pub fn tf(key: &str, vars: &[(&str, String)]) -> String {
    fill(&t(key), vars)
}

#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::t($key)
    };
    ($key:expr, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::tf($key, &[$((stringify!($k), ($v).to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_system_locales() {
        assert_eq!(normalize("ko_KR.UTF-8"), "ko");
        assert_eq!(normalize("en-US"), "en");
        assert_eq!(normalize("zh_CN"), "zh-CN");
        assert_eq!(normalize("pt-br"), "pt-BR");
        assert_eq!(normalize("C"), "en");
    }

    #[test]
    fn fills_placeholders() {
        assert_eq!(
            fill("{{a}} and {{b}}", &[("a", "1".into()), ("b", "2".into())]),
            "1 and 2"
        );
    }

    #[test]
    fn embedded_catalogs_parse_and_share_keys() {
        fn keys(v: &Value, prefix: &str, out: &mut Vec<String>) {
            if let Some(map) = v.as_object() {
                for (k, child) in map {
                    let path = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}.{}", prefix, k)
                    };
                    if child.is_object() {
                        keys(child, &path, out)
                    } else {
                        out.push(path)
                    }
                }
            }
        }
        let en = embedded("en").expect("en parses");
        let ko = embedded("ko").expect("ko parses");
        let (mut a, mut b) = (vec![], vec![]);
        keys(&en, "", &mut a);
        keys(&ko, "", &mut b);
        let missing: Vec<_> = a.iter().filter(|k| !b.contains(k)).collect();
        assert!(missing.is_empty(), "ko is missing keys: {:?}", missing);
    }

    /// 코드가 `t!` 매크로로 부르는 고정 키는 en·ko에 있어야 한다: 없으면 키 이름이 그대로 찍힌다
    #[test]
    fn keys_used_in_code_exist_in_en_and_ko() {
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    sources(&p, out)
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p)
                }
            }
        }
        let mut files = vec![];
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let catalogs = [("en", embedded("en").unwrap()), ("ko", embedded("ko").unwrap())];
        let mut missing = vec![];
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (at, _) in text.match_indices("t!(") {
                // println!·format! 같은 다른 매크로와 문자열이 아닌 인자는 건너뛴다
                if text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let Some(rest) = text[at + 3..].trim_start().strip_prefix('"') else {
                    continue;
                };
                let key = &rest[..rest.find('"').unwrap()];
                if !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                    continue;
                }
                for (lang, cat) in &catalogs {
                    if lookup(cat, key).is_none() {
                        missing.push(format!(
                            "{} {}: {}",
                            lang,
                            file.file_name().unwrap().to_string_lossy(),
                            key
                        ));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "missing keys: {:?}", missing);
    }
}
