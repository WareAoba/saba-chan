use super::{CommandType, HttpMethod, ProtocolError, ServerCommand, ServerResponse};
use crate::protocol::rest::RestClient;
use std::time::Duration;

/// 통합 프로토콜 클라이언트
/// REST API (RCON은 보안상 쓰지 않는다)
#[derive(Debug)]
pub struct ProtocolClient {
    rest: Option<RestClient>,
    command_type: CommandType,
}

impl ProtocolClient {

    /// REST 클라이언트만 생성
    pub fn new_rest(rest_host: String, rest_port: u16, use_https: bool) -> Self {
        Self {
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

    pub fn with_response_limit(mut self, limit: usize) -> Self {
        if let Some(rest) = &mut self.rest { rest.set_response_limit(limit); }
        self
    }

    /// REST만 연결
    pub fn connect_rest(&mut self, timeout: Duration) -> Result<(), ProtocolError> {
        if let Some(rest) = &mut self.rest {
            rest.verify_connection(timeout)?;
        }
        Ok(())
    }

    /// 명령어 실행
    pub fn execute(&mut self, cmd: ServerCommand) -> Result<ServerResponse, ProtocolError> {
        match self.command_type {
            CommandType::Rest => self.execute_rest(cmd),
        }
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
    fn test_rest_client_creation() {
        let client = ProtocolClient::new_rest(
            "127.0.0.1".to_string(),
            8212,
            false,
        );
        assert_eq!(client.command_type, CommandType::Rest);
    }

}
