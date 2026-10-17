//! CLI 전역 설정 — %APPDATA%/saba-chan/cli-settings.json
//!
//! GUI 설정과 분리된 CLI 전용 설정 파일.
//! language, autoStart, refreshInterval 등 CLI 고유 옵션을 관리합니다.

use saba_chan_updater_lib::constants;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// CLI 전역 설정
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliSettings {
    /// 표시 언어 (en, ko, ja, …) — 비어있으면 GUI 설정을 따름
    #[serde(default)]
    pub language: String,

    /// TUI 시작 시 데몬 자동 시작
    #[serde(default = "default_true")]
    pub auto_start: bool,

    /// 상태 모니터 새로고침 간격 (초)
    #[serde(default = "default_refresh")]
    pub refresh_interval: u64,
}

fn default_true() -> bool { true }
fn default_refresh() -> u64 { 2 }

impl Default for CliSettings {
    fn default() -> Self {
        Self {
            language: String::new(),
            auto_start: true,
            refresh_interval: 2,
        }
    }
}

impl CliSettings {
    /// 설정 파일 경로
    fn path() -> anyhow::Result<PathBuf> {
        Ok(constants::resolve_data_dir().join("cli-settings.json"))
    }

    /// 로드 (없으면 기본값)
    pub fn load() -> Self {
        Self::path()
            .ok()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// 저장
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// 실효 언어 (blocking 가능): CLI 설정 → GUI 설정 → 시스템 언어 순으로 폴백.
    pub fn effective_language(&self) -> String {
        resolve_language(
            &self.language,
            || crate::config::setting_language(crate::config::load_settings().ok().as_ref()),
            crate::config::get_system_language,
        )
    }

    /// 키-값 문자열로 설정값 가져오기 (저장된 값 그대로, 언어 미지정은 "auto")
    pub fn get_value(&self, key: &str) -> Option<String> {
        match key {
            "language" | "lang" => Some(if self.language.is_empty() {
                "auto".to_string()
            } else {
                self.language.clone()
            }),
            "auto_start" | "autostart" => Some(self.auto_start.to_string()),
            "refresh_interval" | "refresh" => Some(self.refresh_interval.to_string()),
            _ => None,
        }
    }

    /// 키-값 문자열로 설정값 변경
    pub fn set_value(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "language" | "lang" => {
                // "auto"는 빈 값으로 저장한다 (= GUI/시스템 언어를 따름).
                self.language = if value == "auto" { String::new() } else { value.to_string() };
                Ok(())
            }
            "auto_start" | "autostart" => {
                self.auto_start = value.parse().map_err(|_| "Expected true/false".to_string())?;
                Ok(())
            }
            "refresh_interval" | "refresh" => {
                let n: u64 = value.parse().map_err(|_| "Expected a number (seconds)".to_string())?;
                if n == 0 || n > 60 {
                    return Err("Must be 1-60".to_string());
                }
                self.refresh_interval = n;
                Ok(())
            }
            _ => Err(format!("Unknown key '{}'", key)),
        }
    }

    /// 기본값으로 리셋
    pub fn reset_value(&mut self, key: &str) -> Result<(), String> {
        let defaults = Self::default();
        match key {
            "language" | "lang" => { self.language = defaults.language; Ok(()) }
            "auto_start" | "autostart" => { self.auto_start = defaults.auto_start; Ok(()) }
            "refresh_interval" | "refresh" => { self.refresh_interval = defaults.refresh_interval; Ok(()) }
            _ => Err(format!("Unknown key '{}'", key)),
        }
    }

    /// 사용 가능한 설정 키 목록
    pub fn available_keys() -> &'static [(&'static str, &'static str)] {
        &[
            ("language", "Display language (en, ko, ja, ...)"),
            ("auto_start", "Auto-start daemon on launch (true/false)"),
            ("refresh_interval", "Status refresh interval in seconds (1-60)"),
        ]
    }
}

/// 언어 폴백 규칙: CLI 지정값 → GUI 언어 → 시스템 언어.
/// CLI 값이 비어 있거나 "auto"면 다음 단계로 넘어간다. 조회 함수는 필요할 때만 호출한다.
pub(crate) fn resolve_language(
    cli: &str,
    gui: impl FnOnce() -> Option<String>,
    system: impl FnOnce() -> String,
) -> String {
    if !cli.is_empty() && cli != "auto" {
        return cli.to_string();
    }
    gui().unwrap_or_else(system)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_language_auto_is_stored_as_empty() {
        let mut s = CliSettings::default();
        s.set_value("language", "ko").unwrap();
        s.set_value("language", "auto").unwrap();
        assert_eq!(s.language, "");
        assert_eq!(s.get_value("language"), Some("auto".into()));
    }

    #[test]
    fn resolve_language_falls_back_past_auto() {
        assert_eq!(resolve_language("ja", || Some("ko".into()), || "en".into()), "ja");
        assert_eq!(resolve_language("auto", || Some("ko".into()), || "en".into()), "ko");
        assert_eq!(resolve_language("", || None, || "de".into()), "de");
    }

    #[test]
    fn test_default_values() {
        let s = CliSettings::default();
        assert_eq!(s.language, "");
        assert!(s.auto_start);
        assert_eq!(s.refresh_interval, 2);
    }

    #[test]
    fn test_set_get_value() {
        let mut s = CliSettings::default();

        // language
        assert!(s.set_value("language", "ko").is_ok());
        assert_eq!(s.get_value("language"), Some("ko".into()));

        // alias
        assert!(s.set_value("lang", "ja").is_ok());
        assert_eq!(s.get_value("lang"), Some("ja".into()));

        // auto_start
        assert!(s.set_value("auto_start", "false").is_ok());
        assert!(!s.auto_start);
        assert_eq!(s.get_value("auto_start"), Some("false".into()));

        assert!(s.set_value("auto_start", "notbool").is_err());

        // refresh_interval
        assert!(s.set_value("refresh_interval", "5").is_ok());
        assert_eq!(s.refresh_interval, 5);
        assert!(s.set_value("refresh_interval", "0").is_err());
        assert!(s.set_value("refresh_interval", "61").is_err());
        assert!(s.set_value("refresh_interval", "abc").is_err());

        // unknown key
        assert!(s.set_value("nonexistent", "x").is_err());
        assert!(s.get_value("nonexistent").is_none());
    }

    #[test]
    fn test_reset_value() {
        let mut s = CliSettings::default();
        s.set_value("language", "ko").unwrap();
        s.set_value("auto_start", "false").unwrap();
        s.set_value("refresh_interval", "10").unwrap();

        s.reset_value("language").unwrap();
        assert_eq!(s.language, "");

        s.reset_value("auto_start").unwrap();
        assert!(s.auto_start);

        s.reset_value("refresh_interval").unwrap();
        assert_eq!(s.refresh_interval, 2);

        assert!(s.reset_value("unknown").is_err());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let s = CliSettings {
            language: "ko".into(),
            auto_start: false,
            refresh_interval: 5,
        };

        let json = serde_json::to_string(&s).unwrap();
        let s2: CliSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(s2.language, "ko");
        assert!(!s2.auto_start);
        assert_eq!(s2.refresh_interval, 5);
    }

    #[test]
    fn test_available_keys_not_empty() {
        let keys = CliSettings::available_keys();
        assert!(keys.len() >= 3);
        let key_names: Vec<&str> = keys.iter().map(|(k, _)| *k).collect();
        assert!(key_names.contains(&"language"));
        assert!(key_names.contains(&"auto_start"));
        assert!(key_names.contains(&"refresh_interval"));
    }
}
