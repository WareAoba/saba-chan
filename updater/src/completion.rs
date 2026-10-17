//! 업데이트 완료 마커 (`update-complete.json`)
//!
//! 업데이터 exe 가 적용을 마친 뒤 기록한다. 실패한 컴포넌트도 함께 남긴다.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 업데이트 완료 마커 — GUI/CLI 재시작 시 확인
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCompletionMarker {
    pub timestamp: String,
    pub updated_components: Vec<String>,
    /// 적용에 실패하여 `pending.json` 에 남겨 둔 컴포넌트 키
    #[serde(default)]
    pub failed_components: Vec<String>,
    /// 실패 없이 모두 적용되었는지 여부
    pub success: bool,
    pub message: Option<String>,
}

impl UpdateCompletionMarker {
    /// 적용 결과로 마커를 만든다. `failed` 또는 `error` 가 있으면 `success = false`.
    pub fn from_results(applied: Vec<String>, failed: Vec<String>, error: Option<String>) -> Self {
        let success = failed.is_empty() && error.is_none();
        let message = match error {
            Some(error) => Some(error),
            None if !failed.is_empty() => Some(format!(
                "{} applied, {} failed: {}",
                applied.len(),
                failed.len(),
                failed.join(", ")
            )),
            None if !applied.is_empty() => Some(format!("{} updates applied: {}", applied.len(), applied.join(", "))),
            None => None,
        };
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: applied,
            failed_components: failed,
            success,
            message,
        }
    }

    fn marker_path() -> PathBuf {
        crate::constants::resolve_update_complete_path()
    }

    /// 마커 저장
    pub fn save(&self) -> Result<(), String> {
        let path = Self::marker_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(&path, json).map_err(|e| format!("Write error: {}", e))
    }

    /// 마커 로드
    pub fn load() -> Option<Self> {
        let content = std::fs::read_to_string(Self::marker_path()).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 마커 삭제
    pub fn clear() -> Result<(), String> {
        let path = Self::marker_path();
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("Delete error: {}", e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_reports_failures() {
        let marker = UpdateCompletionMarker::from_results(
            vec!["module-a".into()],
            vec!["saba-core".into()],
            None,
        );
        assert!(!marker.success);
        assert_eq!(marker.failed_components, vec!["saba-core".to_string()]);
        assert!(marker.message.unwrap().contains("saba-core"));
    }

    #[test]
    fn marker_success_without_failures() {
        let marker = UpdateCompletionMarker::from_results(vec!["gui".into()], vec![], None);
        assert!(marker.success);
        assert!(marker.failed_components.is_empty());
    }

    #[test]
    fn marker_error_is_failure() {
        let marker = UpdateCompletionMarker::from_results(vec![], vec![], Some("no targets".into()));
        assert!(!marker.success);
        assert_eq!(marker.message.as_deref(), Some("no targets"));
    }

    #[test]
    fn marker_reads_legacy_json_without_failed_components() {
        let legacy = r#"{"timestamp":"t","updated_components":["gui"],"success":true,"message":null}"#;
        let marker: UpdateCompletionMarker = serde_json::from_str(legacy).unwrap();
        assert!(marker.failed_components.is_empty());
    }
}
