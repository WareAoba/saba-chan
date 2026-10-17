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
}

impl UpdateCompletionMarker {
    pub fn success(components: Vec<String>) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: components,
            success: true,
            message: None,
            failed: Vec::new(),
        }
    }

    pub fn failure(message: String) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: Vec::new(),
            success: false,
            message: Some(message),
            failed: Vec::new(),
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
