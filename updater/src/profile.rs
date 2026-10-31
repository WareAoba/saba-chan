//! 설치 구성: 데스크톱(GUI 포함) 또는 헤드리스(GUI 없음)
//!
//! 설치기·`saba-chan-cli install`이 installed-manifest 옆의 `install-profile.json`(`{"profile":"headless"}`)에 적고,
//! 업데이터·데몬이 읽는다. installed-manifest는 구성요소 → 버전 맵이라 섞지 않는다 (0.1.8이 그 맵을 구성요소로 보인다).
//! 컴포넌트가 어느 구성에 드는지는 `versions.toml`의 `profiles` 하나에서 정한다 (없으면 모든 구성).

use std::path::{Path, PathBuf};

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
    /// installed-manifest와 같은 데이터 폴더에 두는 파일 이름
    pub const FILE_NAME: &'static str = "install-profile.json";

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

    pub fn path() -> PathBuf {
        crate::constants::resolve_data_dir().join(Self::FILE_NAME)
    }

    /// 기록된 구성. 파일이 없거나 읽지 못하거나 모르는 값이면 데스크톱 (0.1.8까지의 설치)
    pub fn load() -> Profile {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Profile {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| v.get("profile").and_then(|p| p.as_str()).and_then(Profile::parse))
            .unwrap_or_default()
    }

    pub fn save(self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::write_atomic(path, serde_json::json!({ "profile": self.as_str() }).to_string().as_bytes())
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
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(Profile::FILE_NAME);
        assert_eq!(Profile::load_from(&path), Profile::Desktop);
        Profile::Headless.save_to(&path).unwrap();
        assert_eq!(Profile::load_from(&path), Profile::Headless);
        std::fs::write(&path, r#"{"profile":"kiosk"}"#).unwrap();
        assert_eq!(Profile::load_from(&path), Profile::Desktop);
        std::fs::write(&path, "{").unwrap();
        assert_eq!(Profile::load_from(&path), Profile::Desktop);
    }

    #[test]
    fn the_gui_belongs_only_to_the_desktop() {
        assert!(Profile::Desktop.includes("gui"));
        assert!(!Profile::Headless.includes("gui"));
        for key in ["saba-core", "cli", "installer", "locales", "module-minecraft"] {
            assert!(Profile::Headless.includes(key), "{key}");
            assert!(Profile::Desktop.includes(key), "{key}");
        }
    }

    #[test]
    fn serializes_in_lowercase() {
        assert_eq!(serde_json::to_string(&Profile::Headless).unwrap(), "\"headless\"");
    }
}
