use super::{ProtocolError, ServerResponse};
use serde_json::{json, Value};
use std::time::Duration;
use ureq::AgentBuilder;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// REST API 클라이언트 (Palworld, 기타 게임)
#[derive(Debug, Clone)]
pub struct RestClient {
    host: String,
    port: u16,
    use_https: bool,
    username: Option<String>,
    password: Option<String>,
}

impl RestClient {
    pub fn new(host: String, port: u16, use_https: bool) -> Self {
        Self {
            host,
            port,
            use_https,
            username: None,
            password: None,
        }
    }

    /// Basic Auth 설정
    pub fn with_basic_auth(mut self, username: String, password: String) -> Self {
        self.username = Some(username);
        self.password = Some(password);
        self
    }

    /// Basic Auth 설정 (가변 참조 버전)
    pub fn set_basic_auth(&mut self, username: String, password: String) {
        self.username = Some(username);
        self.password = Some(password);
    }

    /// Base URL 생성
    fn base_url(&self) -> String {
        let protocol = if self.use_https { "https" } else { "http" };
        format!("{}://{}:{}", protocol, self.host, self.port)
    }

    /// 연결 검증
    pub fn verify_connection(&self, _timeout: Duration) -> Result<(), ProtocolError> {
        // 기본적인 유효성 검사만 수행 (실제 HTTP는 비동기에서 처리)
        if self.host.is_empty() || self.port == 0 {
            return Err(ProtocolError::ConnectionError("Invalid REST endpoint".to_string()));
        }
        Ok(())
    }

    /// GET 요청
    pub fn get(&self, endpoint: &str) -> Result<ServerResponse, ProtocolError> {
        let url = format!("{}{}", self.base_url(), endpoint);
        self.send_http_request(&url, "GET", None)
    }

    /// POST 요청
    pub fn post(&self, endpoint: &str, body: Option<Value>) -> Result<ServerResponse, ProtocolError> {
        let url = format!("{}{}", self.base_url(), endpoint);
        self.send_http_request(&url, "POST", body)
    }

    /// PUT 요청
    pub fn put(&self, endpoint: &str, body: Option<Value>) -> Result<ServerResponse, ProtocolError> {
        let url = format!("{}{}", self.base_url(), endpoint);
        self.send_http_request(&url, "PUT", body)
    }

    /// DELETE 요청
    pub fn delete(&self, endpoint: &str) -> Result<ServerResponse, ProtocolError> {
        let url = format!("{}{}", self.base_url(), endpoint);
        self.send_http_request(&url, "DELETE", None)
    }

    /// HTTP 요청 전송 (실제 요청 수행)
    fn send_http_request(&self, url: &str, method: &str, body: Option<Value>) -> Result<ServerResponse, ProtocolError> {
        tracing::debug!("REST {} {}", method, url);

        let agent = AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build();

        let mut req = agent.request(method, url).set("Content-Type", "application/json");

        if let (Some(user), Some(pass)) = (&self.username, &self.password) {
            let token = BASE64.encode(format!("{}:{}", user, pass));
            req = req.set("Authorization", &format!("Basic {}", token));
        }

        let resp = if let Some(body_val) = body.clone() {
            req.send_json(body_val)
        } else {
            req.call()
        };

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                return Err(ProtocolError::ProtocolError(format!(
                    "HTTP {} {} failed: {}",
                    method,
                    url,
                    e
                )))
            }
        };

        let status = resp.status();
        let text = resp.into_string().unwrap_or_else(|_| "".to_string());
        let data_json: Option<Value> = serde_json::from_str(&text).ok();
        let response_json = data_json.clone().unwrap_or_else(|| json!({"raw": text.clone()}));

        if (200..300).contains(&status) {
            Ok(ServerResponse {
                success: true,
                data: Some(json!({
                    "status": status,
                    "url": url,
                    "method": method,
                    "body": body,
                    "response": response_json,
                    "response_text": text,
                })),
                error: None,
            })
        } else {
            Err(ProtocolError::ProtocolError(format!(
                "HTTP {} {} failed with status {}: {}",
                method,
                url,
                status,
                text
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rest_client_creation() {
        let client = RestClient::new("127.0.0.1".to_string(), 8212, false);
        assert_eq!(client.host, "127.0.0.1");
        assert_eq!(client.port, 8212);
        assert!(!client.use_https);
    }

    #[test]
    fn test_rest_client_base_url() {
        let client = RestClient::new("example.com".to_string(), 443, true);
        assert_eq!(client.base_url(), "https://example.com:443");
    }

    #[test]
    fn test_rest_client_with_auth() {
        let client = RestClient::new("127.0.0.1".to_string(), 8212, false)
            .with_basic_auth("admin".to_string(), "password".to_string());
        
        assert_eq!(client.username, Some("admin".to_string()));
        assert_eq!(client.password, Some("password".to_string()));
    }

    #[test]
    fn test_rest_client_debug() {
        let client = RestClient::new("127.0.0.1".to_string(), 8212, false);
        let debug_str = format!("{:?}", client);
        assert!(debug_str.contains("RestClient"));
    }
}
