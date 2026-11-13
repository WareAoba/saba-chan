//! CLI 문구 — `locales/<lang>/cli.json`
//!
//! en·ko는 실행 파일에 묶어 둔다. 설치된 `locales/<lang>/cli.json`(업데이터가 갱신)이
//! 있으면 그쪽이 먼저다. 언어 파일에 없는 키는 영어, 영어에도 없으면 키 그대로 보인다.
//! 새 언어는 `locales/<lang>/cli.json` 하나만 두면 된다.

use serde_json::Value;
use std::sync::{OnceLock, RwLock};

/// 고를 수 있는 언어 (코드, 그 언어로 쓴 이름)
pub const LANGUAGES: &[(&str, &str)] = &[
    ("ko", "한국어"),
    ("en", "English"),
    ("ja", "日本語"),
    ("zh-CN", "简体中文"),
    ("zh-TW", "繁體中文"),
    ("es", "Español"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("ru", "Русский"),
    ("pt-BR", "Português (Brasil)"),
];

const EMBEDDED: &[(&str, &str)] = &[
    ("en", include_str!("../../locales/en/cli.json")),
    ("ko", include_str!("../../locales/ko/cli.json")),
];

struct Catalog {
    primary: Value,
    fallback: Value,
}

static LANG: OnceLock<String> = OnceLock::new();
/// TUI가 번역 판이 바뀌면 다시 읽으므로 바꿀 수 있게 둔다 (`reload`)
static CATALOG: RwLock<Option<Catalog>> = RwLock::new(None);

/// "ko-KR", "ko_KR.UTF-8" → "ko", "pt" → "pt-BR". 해석은 GUI·데몬과 같고(`constants::resolve_locale`), 지원하지 않는 언어는 en
pub fn normalize(lang: &str) -> String {
    saba_chan_updater_lib::constants::resolve_locale(lang).unwrap_or_else(|| "en".into())
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

fn catalog() -> std::sync::RwLockReadGuard<'static, Option<Catalog>> {
    CATALOG.read().unwrap_or_else(|e| e.into_inner())
}

/// 한 번만 부른다. 두 번째부터는 무시된다.
pub fn init(lang: &str) {
    let _ = LANG.set(normalize(lang));
    // 동시에 부른 쪽도 문구가 채워진 뒤에 돌아간다
    if catalog().is_none() {
        reload();
    }
}

/// 같은 언어의 문구를 다시 읽는다 (업데이터가 설치 폴더 번역을 바꾼 뒤. 단발 명령은 다음 실행부터 새 번역을 쓴다)
pub fn reload() {
    let Some(lang) = LANG.get() else { return };
    let fresh = Catalog { primary: load(lang), fallback: load("en") };
    *CATALOG.write().unwrap_or_else(|e| e.into_inner()) = Some(fresh);
}

pub fn lang() -> &'static str {
    LANG.get().map(String::as_str).unwrap_or("en")
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
    let guard = catalog();
    let Some(cat) = guard.as_ref() else {
        return key.to_string();
    };
    lookup(&cat.primary, key)
        .or_else(|| lookup(&cat.fallback, key))
        .map(str::to_string)
        .unwrap_or_else(|| key.to_string())
}

/// 키가 없으면 None (도움말처럼 없으면 원래 문구를 쓰는 곳용)
pub fn try_t(key: &str) -> Option<String> {
    let guard = catalog();
    let cat = guard.as_ref()?;
    lookup(&cat.primary, key)
        .or_else(|| lookup(&cat.fallback, key))
        .map(str::to_string)
}

/// 정해진 언어의 내장 문구에서 찾는다. 시험은 병렬로 돌고 `init`은 처음 한 번만 먹으므로,
/// 특정 언어 문구를 확인하는 시험은 전역 카탈로그 대신 이것을 쓴다
#[cfg(test)]
pub fn try_t_in(lang: &str, key: &str) -> Option<String> {
    lookup(&embedded(lang)?, key).map(str::to_string)
}

/// `{{name}}` 자리를 채운다. 한국어 조사 꼴(`{{name}}이(가)`)은 채운 값의 받침에 맞는 하나로 고른다
pub fn fill(template: &str, vars: &[(&str, String)]) -> String {
    vars.iter().fold(template.to_string(), |s, (k, v)| {
        let slot = format!("{{{{{}}}}}", k);
        let s = JOSA.iter().fold(s, |s, (pair, with, without, rieul)| {
            let Some(batchim) = final_consonant(v) else { return s };
            let pick = if batchim == Some(8) { rieul } else if batchim.is_some() { with } else { without };
            s.replace(&format!("{slot}{pair}"), &format!("{v}{pick}"))
        });
        s.replace(&slot, v)
    })
}

/// (꼴, 받침 있을 때, 없을 때, ㄹ 받침일 때)
const JOSA: [(&str, &str, &str, &str); 6] = [
    ("이(가)", "이", "가", "이"),
    ("을(를)", "을", "를", "을"),
    ("은(는)", "은", "는", "은"),
    ("과(와)", "과", "와", "과"),
    ("와(과)", "과", "와", "과"),
    ("(으)로", "으로", "로", "로"),
];

/// 값의 마지막 한글 글자 받침 (끝의 따옴표·괄호는 건너뛴다). 숫자는 읽는 소리(영·일·이…)로 본다: 0으로 끝나는 수(십·백·천·만)도
/// 영처럼 받침이 있다. 한글·숫자로 끝나지 않으면 None (조사 꼴을 그대로 둔다).
/// 받침이 없으면 Some(None), 있으면 Some(Some(종성 번호)): ㄹ은 8
fn final_consonant(value: &str) -> Option<Option<u32>> {
    let last = value.chars().rev().find(|c| !matches!(c, '"' | '\'' | '”' | '’' | '」' | '』' | ')' | ']' | ' '))?;
    let last = match last.to_digit(10) {
        Some(d) => ['영', '일', '이', '삼', '사', '오', '육', '칠', '팔', '구'][d as usize],
        None => last,
    };
    let code = last as u32;
    (0xAC00..=0xD7A3).contains(&code).then(|| match (code - 0xAC00) % 28 {
        0 => None,
        n => Some(n),
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
        // GUI·데몬과 같은 해석(`constants::resolve_locale`): 지역 없는 pt·zh도 지원 언어로
        assert_eq!(normalize("pt"), "pt-BR");
        assert_eq!(normalize("zh"), "zh-CN");
    }

    /// 고를 수 있는 언어는 데몬·GUI의 지원 언어와 같다
    #[test]
    fn offers_the_supported_languages() {
        let mut codes: Vec<&str> = LANGUAGES.iter().map(|(c, _)| *c).collect();
        let mut supported = saba_chan_updater_lib::constants::SUPPORTED_LANGUAGES.to_vec();
        codes.sort();
        supported.sort();
        assert_eq!(codes, supported);
    }

    /// 다시 읽어도 언어는 그대로이고 문구를 계속 찾는다
    #[test]
    fn reload_keeps_the_language() {
        init("ko");
        let before = lang();
        reload();
        assert_eq!(lang(), before);
        assert_ne!(t("update_required.title"), "update_required.title");
        assert!(try_t("no.such.key").is_none());
    }

    #[test]
    fn korean_particles_follow_the_last_syllable() {
        let f = |v: &str, t: &str| fill(t, &[("name", v.to_string())]);
        assert_eq!(f("데모서버", "{{name}}이(가) 시작됐어요."), "데모서버가 시작됐어요.");
        assert_eq!(f("마크야생", "{{name}}이(가) 시작됐어요."), "마크야생이 시작됐어요.");
        assert_eq!(f("팰월드", "{{name}}을(를) 지울까요?"), "팰월드를 지울까요?");
        assert_eq!(f("서울", "{{name}}(으)로 옮겨요"), "서울로 옮겨요");
        assert_eq!(f("집", "{{name}}(으)로 옮겨요"), "집으로 옮겨요");
        assert_eq!(f("“야생”", "{{name}}은(는) 꺼져 있어요"), "“야생”은 꺼져 있어요");
        // 숫자로 끝나면 읽는 소리(일·삼·십…)의 받침을 따른다 (판 번호, `my-server-2`)
        assert_eq!(f("0.1.13", "{{name}}(으)로 되돌려요"), "0.1.13으로 되돌려요");
        assert_eq!(f("0.1.12", "{{name}}(으)로 되돌려요"), "0.1.12로 되돌려요");
        assert_eq!(f("0.1.11", "{{name}}(으)로 되돌려요"), "0.1.11로 되돌려요");
        assert_eq!(f("0.1.10", "{{name}}을(를) 받아요"), "0.1.10을 받아요");
        assert_eq!(f("my-server-2", "{{name}}이(가) 시작됐어요."), "my-server-2가 시작됐어요.");
        // 한글로 끝나지 않으면 꼴을 그대로 둔다
        assert_eq!(f("pal", "{{name}}이(가) 시작됐어요."), "pal이(가) 시작됐어요.");
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

    /// CLI 문구는 저장소의 모든 언어 파일에 있다: 한 언어만 영어로 섞여 보이지 않게
    #[test]
    fn cli_text_exists_in_every_language() {
        fn all_keys(v: &Value, prefix: &str, out: &mut Vec<String>) {
            for (k, child) in v.as_object().into_iter().flatten() {
                let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                if child.is_object() {
                    all_keys(child, &path, out)
                } else {
                    out.push(path)
                }
            }
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../locales");
        let mut want = vec![];
        all_keys(&embedded("en").unwrap(), "", &mut want);
        let mut problems = vec![];
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("cli.json")) else { continue };
            let lang = e.file_name().to_string_lossy().to_string();
            let mut have = vec![];
            all_keys(&serde_json::from_str(&text).unwrap(), "", &mut have);
            let missing: Vec<_> = want.iter().filter(|k| !have.contains(k)).collect();
            // 영어 도움말은 clap doc 주석이 원문이라 en 의 help.* 는 일부만 있다
            let stale: Vec<_> = have.iter().filter(|k| !k.starts_with("help.") && !want.contains(k)).collect();
            if !missing.is_empty() || !stale.is_empty() {
                problems.push(format!("{lang}: missing {missing:?}, not in en {stale:?}"));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
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
