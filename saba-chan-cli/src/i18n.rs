//! CLI 다국어 지원 — locales/{lang}/cli.json 로드 + 템플릿 치환
//!
//! 사용법:
//! ```
//! let i18n = I18n::load("ko");
//! let msg = i18n.t("server_cmd.started");
//! let msg = i18n.t_with("server_cmd.not_found", &[("name", "mc-1")]);
//! ```

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub struct I18n {
    data: Value,
    fallback: Value,
}

impl I18n {
    /// 로케일 로드 (폴백: en)
    pub fn load(lang: &str) -> Self {
        let data = load_locale(lang).unwrap_or_else(|| Value::Object(Default::default()));
        let fallback = if lang == "en" {
            data.clone()
        } else {
            load_locale("en").unwrap_or_else(|| Value::Object(Default::default()))
        };
        Self { data, fallback }
    }

    /// 도트 표기법 키로 문자열 조회 (예: "server_cmd.started")
    pub fn t(&self, key: &str) -> String {
        self.resolve(key).unwrap_or_else(|| key.to_string())
    }

    /// 템플릿 변수 치환 ({{key}} → value)
    pub fn t_with(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut s = self.t(key);
        for (k, v) in vars {
            s = s.replace(&format!("{{{{{}}}}}", k), v);
        }
        s
    }

    fn resolve(&self, key: &str) -> Option<String> {
        // 우선: 현재 로케일
        if let Some(v) = resolve_dotted(&self.data, key) {
            return Some(v);
        }
        // 폴백: en
        resolve_dotted(&self.fallback, key)
    }
}

fn resolve_dotted(root: &Value, key: &str) -> Option<String> {
    let mut current = root;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    current.as_str().map(|s| s.to_string())
}

fn load_locale(lang: &str) -> Option<Value> {
    // 1. 프로젝트 루트/locales/{lang}/cli.json (개발용)
    if let Ok(root) = crate::process::find_project_root() {
        let path = root.join("locales").join(lang).join("cli.json");
        if let Some(v) = read_json(&path) {
            return Some(v);
        }
    }

    // 2. 실행 파일 옆의 locales/{lang}/cli.json (배포용)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join("locales").join(lang).join("cli.json");
            if let Some(v) = read_json(&path) {
                return Some(v);
            }
        }
    }

    None
}

fn read_json(path: &PathBuf) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_dotted_simple() {
        let data: Value = serde_json::json!({
            "welcome": "Hello",
            "status": {
                "daemon_on": "Running",
                "daemon_off": "Stopped"
            }
        });
        assert_eq!(resolve_dotted(&data, "welcome"), Some("Hello".into()));
        assert_eq!(resolve_dotted(&data, "status.daemon_on"), Some("Running".into()));
        assert_eq!(resolve_dotted(&data, "status.daemon_off"), Some("Stopped".into()));
        assert_eq!(resolve_dotted(&data, "nonexistent"), None);
        assert_eq!(resolve_dotted(&data, "status.unknown"), None);
    }

    #[test]
    fn test_i18n_t_with_template() {
        // I18n에 직접 데이터를 주입하여 테스트
        let data: Value = serde_json::json!({
            "greeting": "안녕 {{name}}님, {{count}}개의 서버가 있습니다."
        });
        let i18n = I18n {
            data: data.clone(),
            fallback: data,
        };
        let result = i18n.t_with("greeting", &[("name", "사바"), ("count", "3")]);
        assert_eq!(result, "안녕 사바님, 3개의 서버가 있습니다.");
    }

    #[test]
    fn test_i18n_fallback() {
        let data: Value = serde_json::json!({
            "local_only": "로컬 전용"
        });
        let fallback: Value = serde_json::json!({
            "local_only": "Local Only",
            "fallback_key": "From English"
        });
        let i18n = I18n { data, fallback };

        // 현재 로케일에 있으면 그것을 사용
        assert_eq!(i18n.t("local_only"), "로컬 전용");
        // 없으면 폴백
        assert_eq!(i18n.t("fallback_key"), "From English");
        // 둘 다 없으면 키 이름 그대로
        assert_eq!(i18n.t("missing.key"), "missing.key");
    }

    #[test]
    fn test_i18n_load_en() {
        // 프로젝트 루트에 locales/en/cli.json이 있으면 로드
        let i18n = I18n::load("en");
        let welcome = i18n.t("welcome");
        // en 로케일이 있으면 실제 번역, 없거나 키 없으면 "welcome" 반환
        assert!(!welcome.is_empty());
    }

    #[test]
    fn test_i18n_load_nonexistent_locale() {
        let i18n = I18n::load("zz-FAKE");
        // 존재하지 않는 로케일이면 폴백(en)이나 키 이름 반환
        let result = i18n.t("welcome");
        assert!(!result.is_empty());
    }
}
