//! 익스텐션 오류: 클라이언트가 읽는 `error_code`와 사람이 읽는 문구

use super::*;

/// 익스텐션 조작 시 발생할 수 있는 에러.
/// `error_code` 필드를 통해 GUI에서 종류별로 분기할 수 있음.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionError {
    /// 머신 판별용 코드 (예: "not_found", "dependency_missing", "dependency_not_enabled",
    /// "has_dependents", "in_use", "not_mounted", "id_mismatch", "manifest_not_found")
    pub error_code: String,
    /// 사람이 읽을 수 있는 메시지
    pub message: String,
    /// 관련 식별자 목록 (의존 익스텐션 ID, 인스턴스 이름 등)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

impl std::fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ExtensionError {}

impl ExtensionError {
    /// 사바쨩(saba-core)이 모자라 거절했으면 요구 버전 (`>=0.1.10` → `0.1.10`). 응답의 `required_core`로 싣는다 (F8)
    pub fn required_core(&self) -> Option<String> {
        let required = match self.error_code.as_str() {
            "app_version_unsatisfied" => self.related.first(),
            "component_version_unsatisfied" if self.related.first().is_some_and(|c| c == "saba-core") => self.related.get(1),
            _ => None,
        }?;
        Some(required.trim().trim_start_matches(">=").trim().to_string())
    }

    pub(crate) fn not_found(ext_id: &str) -> Self {
        Self {
            error_code: "not_found".to_string(),
            message: format!("Extension '{}' not found in discovered extensions", ext_id),
            related: vec![ext_id.to_string()],
        }
    }
    pub(super) fn dependency_missing(ext_id: &str, dep: &str) -> Self {
        Self {
            error_code: "dependency_missing".to_string(),
            message: format!("Cannot enable '{}': dependency '{}' is not mounted", ext_id, dep),
            related: vec![dep.to_string()],
        }
    }
    pub(super) fn dependency_not_enabled(ext_id: &str, dep: &str) -> Self {
        Self {
            error_code: "dependency_not_enabled".to_string(),
            message: format!("Cannot enable '{}': dependency '{}' is not enabled", ext_id, dep),
            related: vec![dep.to_string()],
        }
    }
    pub(super) fn component_version_unsatisfied(ext_id: &str, component: &str, required: &str, installed: Option<&str>) -> Self {
        Self {
            error_code: "component_version_unsatisfied".to_string(),
            message: format!(
                "Cannot enable '{}': requires {} {} but {} is installed",
                ext_id, component, required,
                installed.unwrap_or("not installed")
            ),
            related: vec![component.to_string(), required.to_string()],
        }
    }
    pub(super) fn has_dependents(ext_id: &str, dependents: &[String]) -> Self {
        Self {
            error_code: "has_dependents".to_string(),
            message: format!(
                "Cannot disable/unmount '{}': depended on by active extension(s): {}",
                ext_id,
                dependents.join(", ")
            ),
            related: dependents.to_vec(),
        }
    }
    pub(super) fn in_use(ext_id: &str, instances: &[String]) -> Self {
        Self {
            error_code: "in_use".to_string(),
            message: format!(
                "Cannot disable/unmount '{}': in use by instance(s): {}",
                ext_id,
                instances.join(", ")
            ),
            related: instances.to_vec(),
        }
    }
    pub(super) fn not_mounted(ext_id: &str) -> Self {
        Self {
            error_code: "not_mounted".to_string(),
            message: format!("Extension '{}' is not mounted", ext_id),
            related: vec![ext_id.to_string()],
        }
    }
    pub(super) fn manifest_not_found(path: &str) -> Self {
        Self {
            error_code: "manifest_not_found".to_string(),
            message: format!("Extension directory or manifest not found: {}", path),
            related: vec![],
        }
    }
    pub(super) fn app_too_old(ext_id: &str, required: &str) -> Self {
        Self {
            error_code: "app_version_unsatisfied".to_string(),
            message: format!(
                "Cannot enable '{}': requires saba-chan {} or newer (this is {})",
                ext_id, required, APP_VERSION
            ),
            related: vec![required.to_string()],
        }
    }
    pub(super) fn folder_in_use(ext_id: &str, owner: &str) -> Self {
        Self {
            error_code: "folder_in_use".to_string(),
            message: format!("Cannot install '{}': its folder is used by extension '{}'", ext_id, owner),
            related: vec![ext_id.to_string(), owner.to_string()],
        }
    }
    pub(super) fn id_mismatch(manifest_id: &str, dir_name: &str) -> Self {
        Self {
            error_code: "id_mismatch".to_string(),
            message: format!(
                "Manifest id '{}' does not match directory name '{}'",
                manifest_id, dir_name
            ),
            related: vec![manifest_id.to_string(), dir_name.to_string()],
        }
    }
}
