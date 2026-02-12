use super::{CommandType, HttpMethod, ProtocolError, ServerCommand, ServerResponse};
use crate::protocol::rcon::RconClient;
use crate::protocol::rest::RestClient;
use std::time::Duration;
use serde_json::json;

/// 통합 프로토콜 클라이언트
/// RCON과 REST API를 모두 지원하며, CommandType에 따라 선택적으로 사용
#[derive(Debug)]
pub struct ProtocolClient {
    rcon: Option<RconClient>,
    rest: Option<RestClient>,
    command_type: CommandType,
}

impl ProtocolClient {
    /// RCON 클라이언트만 생성
    pub fn new_rcon(rcon_host: String, rcon_port: u16, rcon_password: String) -> Self {
        Self {
            rcon: Some(RconClient::new(rcon_host, rcon_port, rcon_password)),
            rest: None,
            command_type: CommandType::Rcon,
        }
    }

    /// REST 클라이언트만 생성
    pub fn new_rest(rest_host: String, rest_port: u16, use_https: bool) -> Self {
        Self {
            rcon: None,
            rest: Some(RestClient::new(rest_host, rest_port, use_https)),
            command_type: CommandType::Rest,
        }
    }

    /// REST Basic Auth 설정 (REST 모드에서만 의미 있음)
    pub fn with_basic_auth(mut self, username: String, password: String) -> Self {
        if let Some(rest) = &mut self.rest {
            rest.set_basic_auth(username, password);
        }
        self
    }

    /// RCON과 REST 모두 사용 (RCON 우선, 실패 시 REST로 fallback)
    #[allow(dead_code)]
    pub fn new_both(
        rcon_host: String, rcon_port: u16, rcon_password: String,
        rest_host: String, rest_port: u16, use_https: bool,
    ) -> Self {
        Self {
            rcon: Some(RconClient::new(rcon_host, rcon_port, rcon_password)),
            rest: Some(RestClient::new(rest_host, rest_port, use_https)),
            command_type: CommandType::Both,
        }
    }

    /// 모든 프로토콜 연결
    #[allow(dead_code)]
    pub fn connect_all(&mut self, timeout: Duration) -> Result<(), ProtocolError> {
        if let Some(rcon) = &mut self.rcon {
            if let Err(e) = rcon.connect(timeout) {
                tracing::warn!("RCON connection failed: {}", e);
                if self.command_type == CommandType::Rcon {
                    return Err(e);
                }
            }
        }

        if let Some(rest) = &mut self.rest {
            if let Err(e) = rest.verify_connection(timeout) {
                tracing::warn!("REST connection verification failed: {}", e);
                if self.command_type == CommandType::Rest {
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// RCON만 연결
    pub fn connect_rcon(&mut self, timeout: Duration) -> Result<(), ProtocolError> {
        if let Some(rcon) = &mut self.rcon {
            rcon.connect(timeout)?;
        }
        Ok(())
    }

    /// REST만 연결
    pub fn connect_rest(&mut self, timeout: Duration) -> Result<(), ProtocolError> {
        if let Some(rest) = &mut self.rest {
            rest.verify_connection(timeout)?;
        }
        Ok(())
    }

    /// 명령어 실행 (설정된 프로토콜에 따라 자동 선택)
    pub fn execute(&mut self, cmd: ServerCommand) -> Result<ServerResponse, ProtocolError> {
        match self.command_type {
            CommandType::Rcon => self.execute_rcon(cmd),
            CommandType::Rest => self.execute_rest(cmd),
            CommandType::Both => {
                // RCON 먼저 시도
                match self.execute_rcon(cmd.clone()) {
                    Ok(response) => Ok(response),
                    Err(e) => {
                        tracing::warn!("RCON failed, attempting REST: {}", e);
                        // RCON 실패 시 REST로 폴백
                        self.execute_rest(cmd)
                    }
                }
            }
        }
    }

    /// RCON으로 명령어 실행
    fn execute_rcon(&mut self, cmd: ServerCommand) -> Result<ServerResponse, ProtocolError> {
        if let Some(rcon) = &mut self.rcon {
            if let Some(command) = cmd.command {
                let response_text = rcon.execute_command(&command)?;
                return Ok(ServerResponse {
                    success: true,
                    data: Some(json!({ "response": response_text, "executed": command })),
                    error: None,
                });
            }
        }

        Err(ProtocolError::CommandError("No command provided for RCON".to_string()))
    }

    /// REST API로 명령어 실행
    fn execute_rest(&mut self, cmd: ServerCommand) -> Result<ServerResponse, ProtocolError> {
        if let Some(rest) = &mut self.rest {
            let method = cmd.method.unwrap_or(HttpMethod::Get);
            let endpoint = cmd.endpoint.unwrap_or_default();
            let body = cmd.body;

            let response = match method {
                HttpMethod::Get => rest.get(&endpoint)?,
                HttpMethod::Post => rest.post(&endpoint, body)?,
                HttpMethod::Put => rest.put(&endpoint, body)?,
                HttpMethod::Delete => rest.delete(&endpoint)?,
            };

            return Ok(response);
        }

        Err(ProtocolError::CommandError("No REST client configured".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rcon_client_creation() {
        let client = ProtocolClient::new_rcon(
            "127.0.0.1".to_string(),
            25575,
            "test_password".to_string(),
        );
        assert_eq!(client.command_type, CommandType::Rcon);
    }

    #[test]
    fn test_rest_client_creation() {
        let client = ProtocolClient::new_rest(
            "127.0.0.1".to_string(),
            8212,
            false,
        );
        assert_eq!(client.command_type, CommandType::Rest);
    }

    #[test]
    fn test_both_client_creation() {
        let client = ProtocolClient::new_both(
            "127.0.0.1".to_string(), 25575, "password".to_string(),
            "127.0.0.1".to_string(), 8212, false,
        );
        assert_eq!(client.command_type, CommandType::Both);
    }

    #[test]
    fn test_client_creation_with_debug() {
        let client = ProtocolClient::new_rcon(
            "game.example.com".to_string(),
            25575,
            "secret".to_string(),
        );
        let debug_str = format!("{:?}", client);
        assert!(debug_str.contains("ProtocolClient"));
    }
}
