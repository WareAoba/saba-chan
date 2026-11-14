//! 데몬 문구 카탈로그 (locales/<lang>/daemon.json): 게임 안 공지·강퇴 사유, 설치 진행 문구, 액션 이름표, 채팅 명령 낱말.
//! 디스코드 봇 답장 문장은 여기 없다 (릴레이가 만든다, `chat::Text`)
//!
//! 설치 폴더의 locales/가 있으면 그것을 우선 읽고, 없으면 바이너리에 내장된
//! 사본을 씁니다. 그래서 locales 컴포넌트가 빠져 있어도 응답이 깨지지 않습니다.
//! 키가 없으면 영어 → 키 이름 순으로 폴백합니다.

use serde_json::Value;
use std::path::PathBuf;

/// 지원 언어와 내장 사본. 새 언어는 이 표에 한 줄 추가하면 됩니다.
const EMBEDDED: &[(&str, &str)] = &[
    ("en", include_str!("../../locales/en/daemon.json")),
    ("ko", include_str!("../../locales/ko/daemon.json")),
    ("ja", include_str!("../../locales/ja/daemon.json")),
    ("zh-CN", include_str!("../../locales/zh-CN/daemon.json")),
    ("zh-TW", include_str!("../../locales/zh-TW/daemon.json")),
    ("es", include_str!("../../locales/es/daemon.json")),
    ("pt-BR", include_str!("../../locales/pt-BR/daemon.json")),
    ("ru", include_str!("../../locales/ru/daemon.json")),
    ("de", include_str!("../../locales/de/daemon.json")),
    ("fr", include_str!("../../locales/fr/daemon.json")),
];

pub const FALLBACK_LANGUAGE: &str = "en";

/// 지원 언어 코드 (내장 사본이 있는 것)
pub fn languages() -> Vec<&'static str> {
    EMBEDDED.iter().map(|(c, _)| *c).collect()
}

#[derive(Debug, Clone)]
pub struct Catalog {
    language: String,
    primary: Value,
    fallback: Value,
}

impl Catalog {
    /// 언어 코드("ko", "ko-KR", "pt-br" 등)에 맞는 카탈로그를 만듭니다.
    pub fn for_language(requested: &str) -> Self {
        let language = normalize_language(requested);
        let primary = load_bundle(&language);
        let fallback = if language == FALLBACK_LANGUAGE {
            Value::Null
        } else {
            load_bundle(FALLBACK_LANGUAGE)
        };
        Self { language, primary, fallback }
    }

    /// 정규화한 언어 코드 (모듈 locales 파일 이름과 같다)
    pub fn language(&self) -> &str {
        &self.language
    }

    /// 점 표기 키("errors.unknown_input")를 번역하고 `{{name}}` 자리를 채웁니다.
    pub fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let template = lookup(&self.primary, key)
            .or_else(|| lookup(&self.fallback, key))
            .unwrap_or(key);
        interpolate(template, vars)
    }
}

/// OS 언어를 지원 언어 코드로. 언어를 고르기 전의 기본값이다 (GUI 셸과 같은 규칙)
pub fn os_language() -> String {
    normalize_language(&sys_locale::get_locale().unwrap_or_else(|| FALLBACK_LANGUAGE.into()))
}

/// 요청 언어를 지원 언어 코드로 맞춥니다. 모르는 언어는 영어.
pub fn normalize_language(requested: &str) -> String {
    saba_chan_updater_lib::constants::resolve_locale(requested).unwrap_or_else(|| FALLBACK_LANGUAGE.to_string())
}

fn load_bundle(language: &str) -> Value {
    for dir in locale_dirs() {
        let path = dir.join(language).join("daemon.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str(&text) {
                Ok(v) => return v,
                Err(e) => tracing::warn!("[i18n] {} 파싱 실패, 내장 사본 사용: {}", path.display(), e),
            }
        }
    }
    EMBEDDED
        .iter()
        .find(|(c, _)| *c == language)
        .and_then(|(_, text)| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null)
}

/// 디스크 locales 탐색 순서: SABA_LOCALES_DIR → 실행 파일 옆 locales/
fn locale_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("SABA_LOCALES_DIR") {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir));
        }
    }
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        dirs.push(exe_dir.join("locales"));
    }
    dirs
}

/// 설치 폴더 `locales/` (디스크 탐색 순서에서 처음 있는 폴더). 없으면 None
pub fn installed_dir() -> Option<PathBuf> {
    locale_dirs().into_iter().find(|d| d.is_dir())
}

/// 설치된 번역의 판 번호 (`locales/version`, 업데이터가 바꿀 때 쓴다). GUI·TUI가 이 값이 바뀌면 다시 읽는다
pub fn installed_version() -> Option<String> {
    installed_dir().and_then(|d| version_in(&d))
}

/// `dir/version`의 판 번호 (앞뒤 공백 제외, 비었으면 None)
pub fn version_in(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("version")).ok()?;
    let v = text.trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn lookup<'a>(bundle: &'a Value, key: &str) -> Option<&'a str> {
    let mut node = bundle;
    for part in key.split('.') {
        node = node.get(part)?;
    }
    node.as_str()
}

pub(crate) fn interpolate(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{}}}}}", name), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_bundle_parses_and_has_core_keys() {
        for (code, text) in EMBEDDED {
            let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{} daemon.json: {}", code, e));
            for key in ["default_prefix", "schedule.restart_warning", "actions.server.start.name"] {
                assert!(lookup(&v, key).is_some(), "{} is missing {}", code, key);
            }
        }
    }

    #[test]
    fn normalizes_region_variants() {
        assert_eq!(normalize_language("ko-KR"), "ko");
        assert_eq!(normalize_language("zh_tw"), "zh-TW");
        assert_eq!(normalize_language("zh"), "zh-CN");
        assert_eq!(normalize_language("zh-HK"), "zh-TW");
        assert_eq!(normalize_language("zh-Hant-TW"), "zh-TW");
        assert_eq!(normalize_language("zh-Hans-SG"), "zh-CN");
        assert_eq!(normalize_language("zh-Hans-HK"), "zh-CN");
        assert_eq!(normalize_language("pt"), "pt-BR");
        assert_eq!(normalize_language("xx"), "en");
    }

    #[test]
    fn version_file_is_trimmed_and_empty_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(version_in(tmp.path()), None);
        std::fs::write(tmp.path().join("version"), " \n").unwrap();
        assert_eq!(version_in(tmp.path()), None);
        std::fs::write(tmp.path().join("version"), "0.1.14\n").unwrap();
        assert_eq!(version_in(tmp.path()).as_deref(), Some("0.1.14"));
    }

    #[test]
    fn interpolates_and_falls_back_to_key() {
        let cat = Catalog::for_language("en");
        let s = cat.t("schedule.restart_warning", &[("minutes", "5")]);
        assert!(s.contains('5') && !s.contains("{{"), "{s}");
        assert_eq!(cat.t("no.such.key", &[]), "no.such.key");
    }
}
