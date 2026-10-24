//! 업데이트 완료 마커 — 업데이터가 남기고 데몬(`/api/updates/last-result`)·GUI가 읽는다

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 업데이트 완료 마커 — GUI/CLI 재시작 시 확인
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCompletionMarker {
    pub timestamp: String,
    pub updated_components: Vec<String>,
    pub success: bool,
    pub message: Option<String>,
    /// 적용하지 못한 컴포넌트와 이유 (`"gui: ..."`)
    #[serde(default)]
    pub failed: Vec<String>,
    /// 적용한 릴리즈의 버전과 패치노트 (A5). 예전 업데이터가 남긴 마커에는 없다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseSummary>,
    /// 클라이언트가 결과를 확인했다. 확인해도 지우지 않아 패치노트를 다시 볼 수 있다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub acknowledged: bool,
}

/// 릴리즈 버전과 언어별 패치노트
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseSummary {
    pub version: String,
    #[serde(default)]
    pub released_at: Option<String>,
    /// 언어 코드 → 패치노트 (`ko`, `en` …)
    #[serde(default)]
    pub notes: std::collections::HashMap<String, String>,
}

impl From<&crate::relay::LatestRelease> for ReleaseSummary {
    fn from(r: &crate::relay::LatestRelease) -> Self {
        Self { version: r.version.clone(), released_at: r.released_at.clone(), notes: r.notes.clone().unwrap_or_default() }
    }
}

impl ReleaseSummary {
    /// `lang`의 패치노트. 없으면 같은 언어의 지역 변형(`zh-CN` → `zh`), 그다음 영어, 한국어 순
    pub fn notes_for(&self, lang: &str) -> Option<&str> {
        let base = lang.split(['-', '_']).next().unwrap_or(lang);
        [lang, base, "en", "ko"]
            .iter()
            .find_map(|l| self.notes.get(*l))
            .or_else(|| self.notes.values().next())
            .map(String::as_str)
    }
}

impl UpdateCompletionMarker {
    pub fn success(components: Vec<String>) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: components,
            success: true,
            message: None,
            failed: Vec::new(),
            release: None,
            acknowledged: false,
        }
    }

    pub fn failure(message: String) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: Vec::new(),
            success: false,
            message: Some(message),
            failed: Vec::new(),
            release: None,
            acknowledged: false,
        }
    }

    /// 마커 파일 경로 (`update-complete.json`)
    pub fn path() -> PathBuf {
        crate::constants::resolve_update_complete_path()
    }

    /// 마커 저장
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;

        crate::write_atomic(path, json.as_bytes())
            .map_err(|e| format!("Write error: {}", e))
    }

    /// 마커 로드
    pub fn load() -> Option<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 클라이언트가 결과를 확인했다고 적는다. 마커는 남긴다 (없으면 할 일 없음)
    pub fn acknowledge_at(path: &std::path::Path) -> Result<(), String> {
        match Self::load_from(path) {
            Some(marker) if !marker.acknowledged => Self { acknowledged: true, ..marker }.save_to(path),
            _ => Ok(()),
        }
    }

    /// 마커 삭제
    pub fn clear() -> Result<(), String> {
        let path = Self::path();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Delete error: {}", e))?;
        }
        Ok(())
    }

    /// 마커 존재 여부
    pub fn exists() -> bool {
        Self::path().exists()
    }
}
