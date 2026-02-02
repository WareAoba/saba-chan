pub mod rcon;
pub mod rest;
pub mod client;

use thiserror::Error;
use serde::{Deserialize, Serialize};

/// 프로토콜 통신 오류 타입
#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum ProtocolError {
    #[error("Connection failed: {0}")]
    ConnectionError(String),

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Timeout: {0}")]
    TimeoutError(String),

    #[error("Command error: {0}")]
    CommandError(String),

    #[error("Protocol-specific error: {0}")]
    Protocol(String),

    #[error("Invalid configuration: {0}")]
    ConfigError(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

/// 통신 타입
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandType {
    #[serde(rename = "rcon")]
    Rcon,
    #[serde(rename = "rest")]
    Rest,
    #[serde(rename = "both")]
    Both,
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
    /// RCON 명령어 (예: "say hello")
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
    #[allow(dead_code)]
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    #[allow(dead_code)]
    pub fn error(error: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_type_serde() {
        let cmd_type = CommandType::Rcon;
        let json = serde_json::to_string(&cmd_type).unwrap();
        assert_eq!(json, "\"rcon\"");
    }

    #[test]
    fn test_server_command_serialization() {
        let cmd = ServerCommand {
            command_type: CommandType::Rcon,
            command: Some("say hello".to_string()),
            endpoint: None,
            method: None,
            body: None,
            timeout_secs: Some(5),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("rcon"));
        assert!(json.contains("say hello"));
    }

    #[test]
    fn test_server_response() {
        let resp = ServerResponse::success(serde_json::json!({"result": "ok"}));
        assert!(resp.success);
        assert!(resp.data.is_some());
        assert!(resp.error.is_none());
    }
}
