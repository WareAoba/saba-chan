//! CLI 전역 설정 — %APPDATA%/saba-chan/cli-settings.json
//!
//! GUI 설정과 분리된 CLI 전용 설정 파일.
//! language, autoStart, refreshInterval 등 CLI 고유 옵션을 관리합니다.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// CLI 전역 설정
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliSettings {
    /// 표시 언어 (en, ko, ja, …) — 비어있으면 GUI 설정을 따름
    #[serde(default)]
    pub language: String,

    /// TUI 시작 시 데몬/봇 자동 시작
    #[serde(default = "default_true")]
    pub auto_start: bool,

    /// 상태 모니터 새로고침 간격 (초)
    #[serde(default = "default_refresh")]
    pub refresh_interval: u64,

    /// 봇 prefix 오버라이드 (비어있으면 GUI 설정을 따름)
    #[serde(default)]
    pub bot_prefix: String,
}

fn default_true() -> bool { true }
fn default_refresh() -> u64 { 2 }

impl Default for CliSettings {
    fn default() -> Self {
        Self {
            language: String::new(),
            auto_start: true,
            refresh_interval: 2,
            bot_prefix: String::new(),
        }
    }
}

impl CliSettings {
    /// 설정 파일 경로
    fn path() -> anyhow::Result<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            let appdata = std::env::var("APPDATA")?;
            Ok(PathBuf::from(appdata).join("saba-chan").join("cli-settings.json"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let home = std::env::var("HOME")?;
            Ok(PathBuf::from(home).join(".config").join("saba-chan").join("cli-settings.json"))
        }
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

    /// 실효 언어: CLI 설정 → GUI 설정 → "en" 순으로 폴백
    pub fn effective_language(&self) -> String {
        if !self.language.is_empty() {
            return self.language.clone();
        }
        crate::gui_config::get_language().unwrap_or_else(|_| "en".into())
    }

    /// 키-값 문자열로 설정값 가져오기
    pub fn get_value(&self, key: &str) -> Option<String> {
        match key {
            "language" | "lang" => Some(self.effective_language()),
            "auto_start" | "autostart" => Some(self.auto_start.to_string()),
            "refresh_interval" | "refresh" => Some(self.refresh_interval.to_string()),
            "bot_prefix" | "prefix" => {
                let p = if self.bot_prefix.is_empty() {
                    crate::gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into())
                } else {
                    self.bot_prefix.clone()
                };
                Some(p)
            }
            _ => None,
        }
    }

    /// 키-값 문자열로 설정값 변경
    pub fn set_value(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "language" | "lang" => {
                self.language = value.to_string();
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
            "bot_prefix" | "prefix" => {
                self.bot_prefix = value.to_string();
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
            "bot_prefix" | "prefix" => { self.bot_prefix = defaults.bot_prefix; Ok(()) }
            _ => Err(format!("Unknown key '{}'", key)),
        }
    }

    /// 사용 가능한 설정 키 목록
    pub fn available_keys() -> &'static [(&'static str, &'static str)] {
        &[
            ("language", "Display language (en, ko, ja, ...)"),
            ("auto_start", "Auto-start daemon/bot on launch (true/false)"),
            ("refresh_interval", "Status refresh interval in seconds (1-60)"),
            ("bot_prefix", "Discord bot prefix override"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let s = CliSettings::default();
        assert_eq!(s.language, "");
        assert!(s.auto_start);
        assert_eq!(s.refresh_interval, 2);
        assert_eq!(s.bot_prefix, "");
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
        assert_eq!(s.auto_start, false);
        assert_eq!(s.get_value("auto_start"), Some("false".into()));

        assert!(s.set_value("auto_start", "notbool").is_err());

        // refresh_interval
        assert!(s.set_value("refresh_interval", "5").is_ok());
        assert_eq!(s.refresh_interval, 5);
        assert!(s.set_value("refresh_interval", "0").is_err());
        assert!(s.set_value("refresh_interval", "61").is_err());
        assert!(s.set_value("refresh_interval", "abc").is_err());

        // bot_prefix
        assert!(s.set_value("bot_prefix", "!test").is_ok());
        assert_eq!(s.bot_prefix, "!test");

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
        s.set_value("bot_prefix", "!x").unwrap();

        s.reset_value("language").unwrap();
        assert_eq!(s.language, "");

        s.reset_value("auto_start").unwrap();
        assert!(s.auto_start);

        s.reset_value("refresh_interval").unwrap();
        assert_eq!(s.refresh_interval, 2);

        s.reset_value("bot_prefix").unwrap();
        assert_eq!(s.bot_prefix, "");

        assert!(s.reset_value("unknown").is_err());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut s = CliSettings::default();
        s.language = "ko".into();
        s.auto_start = false;
        s.refresh_interval = 5;
        s.bot_prefix = "!cmd".into();

        let json = serde_json::to_string(&s).unwrap();
        let s2: CliSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(s2.language, "ko");
        assert_eq!(s2.auto_start, false);
        assert_eq!(s2.refresh_interval, 5);
        assert_eq!(s2.bot_prefix, "!cmd");
    }

    #[test]
    fn test_available_keys_not_empty() {
        let keys = CliSettings::available_keys();
        assert!(keys.len() >= 4);
        let key_names: Vec<&str> = keys.iter().map(|(k, _)| *k).collect();
        assert!(key_names.contains(&"language"));
        assert!(key_names.contains(&"auto_start"));
        assert!(key_names.contains(&"refresh_interval"));
        assert!(key_names.contains(&"bot_prefix"));
    }
}
