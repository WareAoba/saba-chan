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
    /// 데몬이 바뀐 모듈이 쓰는 익스텐션을 맞췄다 (`take_unprepared_at`). 업데이터는 적지 않는다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extensions_prepared: bool,
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
            extensions_prepared: false,
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
            extensions_prepared: false,
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

    /// 데몬이 아직 익스텐션을 맞추지 않은 마커면 맞췄다고 먼저 적고 돌려준다 (없거나 이미 맞췄으면 None).
    /// 먼저 적으므로 맞추다 끊겨도 다시 하지 않는다: 빠진 익스텐션은 시작할 때 `extension_required`로 알린다
    pub fn take_unprepared_at(path: &std::path::Path) -> Option<Self> {
        let marker = Self::load_from(path).filter(|m| !m.extensions_prepared)?;
        if let Err(e) = (Self { extensions_prepared: true, ..marker.clone() }).save_to(path) {
            tracing::warn!("[Updater] Cannot mark the update result as handled: {}", e);
        }
        Some(marker)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 업데이트로 바뀐 모듈의 익스텐션은 데몬이 마커 하나에 한 번만 맞춘다. 예전 업데이터가 남긴 마커(표시 없음)도 맞출 대상이다
    #[test]
    fn a_marker_is_handed_out_for_extension_setup_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-complete.json");
        assert!(UpdateCompletionMarker::take_unprepared_at(&path).is_none(), "no marker");

        std::fs::write(&path, r#"{"timestamp":"t","updated_components":["saba-core","module-x"],"success":true,"message":null,"acknowledged":true}"#).unwrap();
        let taken = UpdateCompletionMarker::take_unprepared_at(&path).unwrap();
        assert_eq!(taken.updated_components, ["saba-core", "module-x"]);
        assert!(UpdateCompletionMarker::take_unprepared_at(&path).is_none(), "only once");
        let kept = UpdateCompletionMarker::load_from(&path).unwrap();
        assert!(kept.acknowledged && kept.extensions_prepared, "other fields stay");

        UpdateCompletionMarker::success(vec!["module-y".into()]).save_to(&path).unwrap();
        assert!(UpdateCompletionMarker::take_unprepared_at(&path).is_some(), "a new update is handed out again");
    }
}
