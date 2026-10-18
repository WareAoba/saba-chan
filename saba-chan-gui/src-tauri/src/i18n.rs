//! 셸 문구(트레이·알림·상태) 번역 — locales/<lang>/common.json
//!
//! locales는 따로 업데이트되므로 설치 루트의 파일을 먼저 읽고, 없으면 exe에 넣어 둔 사본을 쓴다.
//! 없는 키는 영어 → 키 그대로.

use serde_json::Value;
use std::path::Path;

/// exe에 넣어 둔 common.json (설치 루트에 locales가 없어도 문구가 나오게)
const EMBEDDED: &[(&str, &str)] = &[
    ("en", include_str!("../../../locales/en/common.json")),
    ("ko", include_str!("../../../locales/ko/common.json")),
    ("ja", include_str!("../../../locales/ja/common.json")),
    ("zh-CN", include_str!("../../../locales/zh-CN/common.json")),
    ("zh-TW", include_str!("../../../locales/zh-TW/common.json")),
    ("es", include_str!("../../../locales/es/common.json")),
    ("pt-BR", include_str!("../../../locales/pt-BR/common.json")),
    ("ru", include_str!("../../../locales/ru/common.json")),
    ("de", include_str!("../../../locales/de/common.json")),
    ("fr", include_str!("../../../locales/fr/common.json")),
];

pub struct I18n {
    lang: Value,
    en: Value,
}

impl I18n {
    /// `locales_dir`(설치 루트의 locales)가 있으면 그 파일을, 없거나 깨졌으면 내장본을 쓴다
    pub fn load(lang: &str, locales_dir: Option<&Path>) -> Self {
        Self {
            lang: load_common(lang, locales_dir).unwrap_or(Value::Null),
            en: load_common("en", locales_dir).unwrap_or(Value::Null),
        }
    }

    pub fn from_values(lang: Value, en: Value) -> Self {
        Self { lang, en }
    }

    /// `a.b.c` 키를 찾아 `{{var}}`를 채운다
    pub fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        match lookup(&self.lang, key).or_else(|| lookup(&self.en, key)) {
            Some(template) => interpolate(template, vars),
            None => key.to_string(),
        }
    }
}

fn load_common(lang: &str, locales_dir: Option<&Path>) -> Option<Value> {
    if let Some(dir) = locales_dir {
        let path = dir.join(lang).join("common.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str(strip_bom(&text)) {
                Ok(v) => return Some(v),
                Err(e) => tracing::error!("Failed to load translations {}: {}", path.display(), e),
            }
        }
    }
    let (_, text) = EMBEDDED.iter().find(|(l, _)| *l == lang)?;
    serde_json::from_str(strip_bom(text)).ok()
}

pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn lookup<'a>(root: &'a Value, key: &str) -> Option<&'a str> {
    key.split('.').try_fold(root, |v, k| v.get(k))?.as_str()
}

/// `{{name}}` → vars의 값. 없거나 빈 값이면 그대로 둔다 (main.js t()와 같음)
pub fn interpolate(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}");
        let name = end.map(|e| &after[..e]);
        match name.filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            Some(n) => {
                match vars
                    .iter()
                    .find(|(k, _)| *k == n)
                    .map(|(_, v)| *v)
                    .filter(|v| !v.is_empty())
                {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 4 + n.len()]),
                }
                rest = &after[n.len() + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interpolates_known_vars_and_keeps_unknown() {
        assert_eq!(
            interpolate("포트 {{port}} / {{name}}", &[("port", "25565")]),
            "포트 25565 / {{name}}"
        );
        assert_eq!(interpolate("{{a}}{{a}}", &[("a", "x")]), "xx");
        // 빈 값은 JS의 `variables[v] || match`처럼 자리표시를 남긴다
        assert_eq!(interpolate("[{{a}}]", &[("a", "")]), "[{{a}}]");
        assert_eq!(interpolate("{{ broken", &[]), "{{ broken");
    }

    #[test]
    fn falls_back_to_english_then_key() {
        let i = I18n::from_values(
            json!({"tray": {"tooltip": "사바쨩"}}),
            json!({"tray": {"tooltip": "Saba", "quit_all": "Quit"}, "daemon": {"check_warning": "warn: {{error}}"}}),
        );
        assert_eq!(i.t("tray.tooltip", &[]), "사바쨩");
        assert_eq!(i.t("tray.quit_all", &[]), "Quit");
        assert_eq!(i.t("daemon.check_warning", &[("error", "boom")]), "warn: boom");
        assert_eq!(i.t("tray.nope", &[]), "tray.nope");
        // 객체 키는 문자열이 아니므로 키 그대로
        assert_eq!(i.t("tray", &[]), "tray");
    }

    #[test]
    fn embedded_locales_parse_and_runtime_file_wins() {
        let i = I18n::load("ko", None);
        assert_ne!(i.t("tray.tooltip", &[]), "tray.tooltip");

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ko")).unwrap();
        std::fs::write(
            dir.path().join("ko/common.json"),
            "\u{feff}{\"tray\":{\"tooltip\":\"런타임\"}}",
        )
        .unwrap();
        let i = I18n::load("ko", Some(dir.path()));
        assert_eq!(i.t("tray.tooltip", &[]), "런타임");
        // 런타임 ko에 없는 키는 영어 내장본으로
        assert_ne!(i.t("tray.quit_all", &[]), "tray.quit_all");
    }
}
