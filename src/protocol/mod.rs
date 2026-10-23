pub mod rest;
pub mod client;
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

    #[error("Command error: {0}")]
    CommandError(String),

    #[error("Protocol-specific error: {0}")]
    Protocol(String),

    /// 서버가 2xx가 아닌 상태로 답했다 (상태 코드, 설명)
    #[error("{1}")]
    HttpStatus(u16, String),
}

/// 통신 타입
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandType {
    #[serde(rename = "rest")]
    Rest,
}

/// HTTP 메서드
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "PUT")]
    Put,
    #[serde(rename = "DELETE")]
    Delete,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
            Self::Put => write!(f, "PUT"),
            Self::Delete => write!(f, "DELETE"),
        }
    }
}

/// 서버 명령어 요청
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCommand {
    pub command_type: CommandType,
    /// 콘솔 명령어 (예: "say hello")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// REST API 엔드포인트 (예: "/api/info")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// HTTP 메서드 (기본: GET)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<HttpMethod>,
    /// HTTP 요청 본문
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    /// 타임아웃 (초)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
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

impl ServerResponse {

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_type_serde() {
        let cmd_type = CommandType::Rest;
        let json = serde_json::to_string(&cmd_type).unwrap();
        assert_eq!(json, "\"rest\"");
    }

    #[test]
    fn test_server_command_serialization() {
        let cmd = ServerCommand {
            command_type: CommandType::Rest,
            command: Some("say hello".to_string()),
            endpoint: None,
            method: None,
            body: None,
            timeout_secs: Some(5),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("rest"));
        assert!(json.contains("say hello"));
    }

}
