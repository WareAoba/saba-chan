//! 설치 구성: 데스크톱(GUI 포함) 또는 헤드리스(GUI 없음)
//!
//! 설치기·`saba-chan-cli install`이 installed-manifest의 `profile` 키에 적고, 업데이터·데몬이 읽는다.
//! 컴포넌트가 어느 구성에 드는지는 `versions.toml`의 `profiles` 하나에서 정한다 (없으면 모든 구성).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    /// 0.1.8까지의 설치는 기록이 없고 모두 데스크톱이다
    #[default]
    Desktop,
    Headless,
}

impl Profile {
    /// installed-manifest에 적는 키
    pub const MANIFEST_KEY: &'static str = "profile";

    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Desktop => "desktop",
            Profile::Headless => "headless",
        }
    }

    pub fn parse(value: &str) -> Option<Profile> {
        match value.trim().to_ascii_lowercase().as_str() {
            "desktop" => Some(Profile::Desktop),
            "headless" => Some(Profile::Headless),
            _ => None,
        }
    }

    /// 설치 기록의 구성. 없거나 모르는 값이면 데스크톱
    pub fn from_installed(installed: &std::collections::HashMap<String, String>) -> Profile {
        installed.get(Self::MANIFEST_KEY).and_then(|v| Profile::parse(v)).unwrap_or_default()
    }

    /// 이 구성에 드는 컴포넌트인지 (`versions.toml`의 `profiles`. 표시가 없으면 모든 구성)
    pub fn includes(self, component_key: &str) -> bool {
        crate::versions::profiles(component_key).is_none_or(|p| p.iter().any(|name| name == self.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_unknown_record_is_desktop() {
        let mut installed = std::collections::HashMap::new();
        assert_eq!(Profile::from_installed(&installed), Profile::Desktop);
        installed.insert("profile".to_string(), "Headless".to_string());
        assert_eq!(Profile::from_installed(&installed), Profile::Headless);
        installed.insert("profile".to_string(), "kiosk".to_string());
        assert_eq!(Profile::from_installed(&installed), Profile::Desktop);
    }

    #[test]
    fn the_gui_belongs_only_to_the_desktop() {
        assert!(Profile::Desktop.includes("gui"));
        assert!(!Profile::Headless.includes("gui"));
        for key in ["saba-core", "cli", "updater", "locales", "module-minecraft"] {
            assert!(Profile::Headless.includes(key), "{key}");
            assert!(Profile::Desktop.includes(key), "{key}");
        }
    }

    #[test]
    fn serializes_in_lowercase() {
        assert_eq!(serde_json::to_string(&Profile::Headless).unwrap(), "\"headless\"");
    }
}
