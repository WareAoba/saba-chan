pub mod rest;
pub mod rcon;

use thiserror::Error;
use serde::{Deserialize, Serialize};

/// 프로토콜 통신 오류 타입
#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("REST response exceeded the allowed size")]
    ResponseTooLarge,
    #[error("Connection failed: {0}")]
    ConnectionError(String),

    #[error("Protocol-specific error: {0}")]
    Protocol(String),

    /// 서버가 2xx가 아닌 상태로 답했다 (상태 코드, 설명)
    #[error("{1}")]
    HttpStatus(u16, String),
}

/// 서버 명령어 응답
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
