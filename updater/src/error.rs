//! 에러 처리 및 복구 로직
//!
//! ## 지원하는 에러 상황
//! - 네트워크 끊김 / 타임아웃
//! - 다운로드 중단
//! - 파일 시스템 오류
//! - API 응답 오류

use std::fmt;
use std::time::Duration;
use serde::{Deserialize, Serialize};

/// 업데이터 에러 타입
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "details")]
pub enum UpdaterError {
    /// 네트워크 연결 실패
    NetworkError {
        message: String,
        recoverable: bool,
    },
    /// HTTP 요청 타임아웃
    Timeout {
        operation: String,
        duration_secs: u64,
    },
    /// API 응답 오류
    ApiError {
        status_code: u16,
        message: String,
    },
    /// 다운로드 중단됨
    DownloadInterrupted {
        component: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    /// 파일 시스템 오류
    FileSystemError {
        operation: String,
        path: String,
        message: String,
    },
    /// 검증 실패 (해시 불일치 등)
    ValidationError {
        component: String,
        expected: String,
        actual: String,
    },
    /// 설정 오류
    ConfigError {
        message: String,
    },
    /// 알 수 없는 오류
    Unknown {
        message: String,
    },
}

impl fmt::Display for UpdaterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdaterError::NetworkError { message, .. } => {
                write!(f, "Network error: {}", message)
            }
            UpdaterError::Timeout { operation, duration_secs } => {
                write!(f, "Timeout after {}s: {}", duration_secs, operation)
            }
            UpdaterError::ApiError { status_code, message } => {
                write!(f, "API error ({}): {}", status_code, message)
            }
            UpdaterError::DownloadInterrupted { component, downloaded_bytes, total_bytes } => {
                write!(
                    f,
                    "Download interrupted for {}: {}/{} bytes",
                    component, downloaded_bytes, total_bytes
                )
            }
            UpdaterError::FileSystemError { operation, path, message } => {
                write!(f, "File system error during {} on '{}': {}", operation, path, message)
            }
            UpdaterError::ValidationError { component, expected, actual } => {
                write!(
                    f,
                    "Validation failed for {}: expected {}, got {}",
                    component, expected, actual
                )
            }
            UpdaterError::ConfigError { message } => {
                write!(f, "Configuration error: {}", message)
            }
            UpdaterError::Unknown { message } => {
                write!(f, "Unknown error: {}", message)
            }
        }
    }
}

impl std::error::Error for UpdaterError {}

impl UpdaterError {
    /// 복구 가능한 에러인지 확인
    pub fn is_recoverable(&self) -> bool {
        match self {
            UpdaterError::NetworkError { recoverable, .. } => *recoverable,
            UpdaterError::Timeout { .. } => true,
            UpdaterError::ApiError { status_code, .. } => {
                // 5xx 에러는 재시도 가능, 4xx는 불가능
                *status_code >= 500
            }
            UpdaterError::DownloadInterrupted { .. } => true,
            UpdaterError::FileSystemError { .. } => false,
            UpdaterError::ValidationError { .. } => true, // 재다운로드로 복구 가능
            UpdaterError::ConfigError { .. } => false,
            UpdaterError::Unknown { .. } => false,
        }
    }

    /// 권장 재시도 대기 시간
    pub fn retry_delay(&self, attempt: u32) -> Duration {
        let base_delay = match self {
            UpdaterError::NetworkError { .. } => Duration::from_secs(2),
            UpdaterError::Timeout { .. } => Duration::from_secs(5),
            UpdaterError::ApiError { status_code, .. } => {
                if *status_code == 429 {
                    // Rate limit — 더 긴 대기
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(3)
                }
            }
            UpdaterError::DownloadInterrupted { .. } => Duration::from_secs(1),
            _ => Duration::from_secs(1),
        };

        // Exponential backoff with jitter
        let multiplier = 2u64.pow(attempt);
        let delay_secs = base_delay.as_secs() * multiplier;
        let max_delay = 60u64; // 최대 1분
        Duration::from_secs(delay_secs.min(max_delay))
    }

    /// 사용자에게 표시할 메시지
    pub fn user_message(&self) -> String {
        match self {
            UpdaterError::NetworkError { .. } => {
                "인터넷 연결을 확인해주세요.".to_string()
            }
            UpdaterError::Timeout { .. } => {
                "서버 응답이 지연되고 있습니다. 잠시 후 다시 시도해주세요.".to_string()
            }
            UpdaterError::ApiError { status_code, .. } => {
                if *status_code == 404 {
                    "요청한 업데이트를 찾을 수 없습니다.".to_string()
                } else if *status_code == 403 {
                    "접근이 거부되었습니다. API 제한일 수 있습니다.".to_string()
                } else if *status_code >= 500 {
                    "서버에 일시적인 문제가 있습니다. 잠시 후 다시 시도해주세요.".to_string()
                } else {
                    format!("서버 오류 ({})", status_code)
                }
            }
            UpdaterError::DownloadInterrupted { .. } => {
                "다운로드가 중단되었습니다. 다시 시도합니다...".to_string()
            }
            UpdaterError::FileSystemError { .. } => {
                "파일 저장 중 오류가 발생했습니다. 디스크 공간을 확인해주세요.".to_string()
            }
            UpdaterError::ValidationError { .. } => {
                "다운로드 파일 검증에 실패했습니다. 다시 다운로드합니다...".to_string()
            }
            UpdaterError::ConfigError { message } => {
                format!("설정 오류: {}", message)
            }
            UpdaterError::Unknown { message } => {
                format!("오류가 발생했습니다: {}", message)
            }
        }
    }

    /// reqwest 에러를 UpdaterError로 변환
    pub fn from_reqwest(err: &reqwest::Error, operation: &str) -> Self {
        if err.is_timeout() {
            UpdaterError::Timeout {
                operation: operation.to_string(),
                duration_secs: 30,
            }
        } else if err.is_connect() {
            UpdaterError::NetworkError {
                message: "연결 실패".to_string(),
                recoverable: true,
            }
        } else if let Some(status) = err.status() {
            UpdaterError::ApiError {
                status_code: status.as_u16(),
                message: err.to_string(),
            }
        } else {
            UpdaterError::NetworkError {
                message: err.to_string(),
                recoverable: err.is_request() || err.is_body(),
            }
        }
    }

    /// IO 에러를 UpdaterError로 변환
    pub fn from_io(err: &std::io::Error, operation: &str, path: &str) -> Self {
        UpdaterError::FileSystemError {
            operation: operation.to_string(),
            path: path.to_string(),
            message: err.to_string(),
        }
    }
}

/// 에러 복구 전략
#[derive(Debug, Clone)]
pub struct RecoveryStrategy {
    /// 최대 재시도 횟수
    pub max_retries: u32,
    /// 현재 시도 횟수
    pub current_attempt: u32,
    /// 재시도 간 기본 대기 시간
    pub base_delay: Duration,
    /// 지수 백오프 사용 여부
    pub use_backoff: bool,
}

impl Default for RecoveryStrategy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            current_attempt: 0,
            base_delay: Duration::from_secs(2),
            use_backoff: true,
        }
    }
}

impl RecoveryStrategy {
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            ..Default::default()
        }
    }

    /// 다음 시도 전 대기 시간 계산
    pub fn next_delay(&self) -> Duration {
        if self.use_backoff {
            let multiplier = 2u64.pow(self.current_attempt);
            let delay_secs = self.base_delay.as_secs() * multiplier;
            Duration::from_secs(delay_secs.min(60)) // 최대 1분
        } else {
            self.base_delay
        }
    }

    /// 재시도 가능한지 확인
    pub fn can_retry(&self) -> bool {
        self.current_attempt < self.max_retries
    }

    /// 시도 횟수 증가
    pub fn increment(&mut self) {
        self.current_attempt += 1;
    }

    /// 리셋
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }
}

/// 에러 컨텍스트 (디버깅/로깅용)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    pub timestamp: String,
    pub operation: String,
    pub component: Option<String>,
    pub error: UpdaterError,
    pub stack_trace: Option<String>,
}

impl ErrorContext {
    pub fn new(operation: &str, error: UpdaterError) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            operation: operation.to_string(),
            component: None,
            error,
            stack_trace: None,
        }
    }

    pub fn with_component(mut self, component: &str) -> Self {
        self.component = Some(component.to_string());
        self
    }

    /// 로그 출력
    pub fn log(&self) {
        if self.error.is_recoverable() {
            tracing::warn!(
                "[Error] {} - {} (recoverable): {}",
                self.operation,
                self.component.as_deref().unwrap_or("N/A"),
                self.error
            );
        } else {
            tracing::error!(
                "[Error] {} - {} (fatal): {}",
                self.operation,
                self.component.as_deref().unwrap_or("N/A"),
                self.error
            );
        }
    }
}

/// 네트워크 상태 체커
pub struct NetworkChecker {
    /// 체크할 엔드포인트 목록
    endpoints: Vec<String>,
    /// 타임아웃
    timeout: Duration,
}

impl NetworkChecker {
    pub fn new() -> Self {
        Self {
            endpoints: vec![
                "https://api.github.com".to_string(),
                "https://github.com".to_string(),
            ],
            timeout: Duration::from_secs(5),
        }
    }

    /// 네트워크 연결 상태 확인
    pub async fn check_connectivity(&self) -> bool {
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .ok();

        let client = match client {
            Some(c) => c,
            None => return false,
        };

        for endpoint in &self.endpoints {
            match client.head(endpoint).send().await {
                Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                    return true;
                }
                _ => continue,
            }
        }

        false
    }

    /// 연결 대기 (연결될 때까지)
    pub async fn wait_for_connection(&self, max_wait: Duration) -> bool {
        let start = std::time::Instant::now();
        let check_interval = Duration::from_secs(2);

        while start.elapsed() < max_wait {
            if self.check_connectivity().await {
                return true;
            }
            tokio::time::sleep(check_interval).await;
        }

        false
    }
}

impl Default for NetworkChecker {
    fn default() -> Self {
        Self::new()
    }
}
