//! Supervisor 전용 에러 타입 — 에러 종류를 구분하여 IPC 핸들러에서
//! 적절한 HTTP 상태 코드를 반환할 수 있게 합니다.

use axum::http::StatusCode;

/// Supervisor 작업 중 발생할 수 있는 에러 유형
#[derive(thiserror::Error, Debug)]
pub enum SupervisorError {
    #[error("Module '{0}' not found")]
    ModuleNotFound(String),

    #[error("Instance '{0}' not found")]
    InstanceNotFound(String),

    #[error("Server '{0}' is already running")]
    AlreadyRunning(String),

    #[error("Server '{0}' is not running")]
    NotRunning(String),

    #[error("No managed process for instance '{0}'")]
    NoManagedProcess(String),

    #[error("Plugin execution failed: {0}")]
    PluginError(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl SupervisorError {
    /// HTTP 상태 코드 매핑
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::ModuleNotFound(_) | Self::InstanceNotFound(_) => StatusCode::NOT_FOUND,
            Self::AlreadyRunning(_) => StatusCode::CONFLICT,
            Self::NotRunning(_) | Self::NoManagedProcess(_) => StatusCode::CONFLICT,
            Self::InvalidConfig(_) => StatusCode::BAD_REQUEST,
            Self::PluginError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// JSON 에러 응답 생성
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "success": false,
            "error": self.to_string(),
            "error_code": self.error_code(),
        })
    }

    /// 머신 리더블 에러 코드
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::ModuleNotFound(_) => "MODULE_NOT_FOUND",
            Self::InstanceNotFound(_) => "INSTANCE_NOT_FOUND",
            Self::AlreadyRunning(_) => "ALREADY_RUNNING",
            Self::NotRunning(_) => "NOT_RUNNING",
            Self::NoManagedProcess(_) => "NO_MANAGED_PROCESS",
            Self::PluginError(_) => "PLUGIN_ERROR",
            Self::InvalidConfig(_) => "INVALID_CONFIG",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

/// axum 핸들러에서 SupervisorError를 직접 반환할 수 있도록 IntoResponse 구현
impl axum::response::IntoResponse for SupervisorError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status_code();
        let body = axum::Json(self.to_json());
        (status, body).into_response()
    }
}
