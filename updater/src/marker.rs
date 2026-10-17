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
        }
    }

    fn marker_path() -> PathBuf {
        crate::constants::resolve_update_complete_path()
    }

    /// 마커 저장
    pub fn save(&self) -> Result<(), String> {
        let path = Self::marker_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;

        std::fs::write(&path, json)
            .map_err(|e| format!("Write error: {}", e))
    }

    /// 마커 로드
    pub fn load() -> Option<Self> {
        let path = Self::marker_path();
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 마커 삭제
    pub fn clear() -> Result<(), String> {
        let path = Self::marker_path();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Delete error: {}", e))?;
        }
        Ok(())
    }

    /// 마커 존재 여부
    pub fn exists() -> bool {
        Self::marker_path().exists()
    }
}
