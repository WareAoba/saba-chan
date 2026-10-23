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
    response_limit: Option<usize>,
}

impl RestClient {
    pub fn new(host: String, port: u16, use_https: bool) -> Self {
        Self {
            host,
            port,
            use_https,
            username: None,
            password: None,
            response_limit: None,
        }
    }

    /// Basic Auth 설정 (가변 참조 버전)
    pub fn set_basic_auth(&mut self, username: String, password: String) {
        self.username = Some(username);
        self.password = Some(password);
    }

    pub fn set_response_limit(&mut self, limit: usize) {
        self.response_limit = Some(limit);
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

        let mut builder = AgentBuilder::new().timeout(Duration::from_secs(10));
        if self.use_https {
            // TLS는 reqwest와 같은 native-tls를 쓴다 (rustls·webpki 루트를 따로 싣지 않는다)
            let tls = native_tls::TlsConnector::new()
                .map_err(|e| ProtocolError::Protocol(format!("TLS 초기화 실패: {}", e)))?;
            builder = builder.tls_connector(std::sync::Arc::new(tls));
        }
        let agent = builder.build();

        let mut req = agent.request(method, url).set("Content-Type", "application/json");

        if let (Some(user), Some(pass)) = (&self.username, &self.password) {
            let token = BASE64.encode(format!("{}:{}", user, pass));
            tracing::debug!("REST auth: user={}", user);
            req = req.set("Authorization", &format!("Basic {}", token));
        }

        let resp = if let Some(ref body_val) = body {
            req.send_json(body_val.clone())
        } else {
            req.call()
        };

        let resp = match resp {
            Ok(r) => r,
            // 4xx·5xx도 응답이다: 아래에서 상태와 본문을 함께 알린다
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => {
                // 응답 없이 연결이 닫히면(EOF·reset·abort) 서버가 요청을 처리했는지 알 수 없다.
                // 종료 명령처럼 서버가 끊는 것이 자연스러운 경우도 있어 전송 오류와 나누되, 성공으로 보고하지 않는다
                if closed_without_response(&e) {
                    return Ok(ServerResponse {
                        success: false,
                        data: Some(json!({
                            "outcome": "unknown",
                            "url": url,
                            "method": method,
                            "body": body,
                            "note": "Connection closed before a response",
                        })),
                        error: Some(format!("HTTP {} {}: connection closed before a response; the result is unknown", method, url)),
                    });
                }
                return Err(ProtocolError::Protocol(format!(
                    "HTTP {} {} failed: {}",
                    method,
                    url,
                    e
                )))
            }
        };

        let status = resp.status();
        let text = if let Some(limit) = self.response_limit {
            use std::io::Read;
            let mut bytes = Vec::new();
            resp.into_reader().take(limit as u64 + 1).read_to_end(&mut bytes)
                .map_err(|error| ProtocolError::Protocol(error.to_string()))?;
            if bytes.len() > limit { return Err(ProtocolError::ResponseTooLarge); }
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            resp.into_string().unwrap_or_default()
        };
        let data_json: Option<Value> = serde_json::from_str(&text).ok();
        let response_json = data_json.unwrap_or_else(|| json!({"raw": &text}));

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
            // 본문은 앞부분만: 오류 문자열은 클라이언트·감사 기록·디스코드로 간다
            let head: String = text.chars().take(200).collect();
            Err(ProtocolError::HttpStatus(status, format!(
                "HTTP {} {} failed with status {}: {}",
                method,
                url,
                status,
                head
            )))
        }
    }
}

/// 연결은 됐는데 응답 없이 닫혔는지. 오류 종류로 본다: Windows는 os error 10053(abort)·10054(reset)로,
/// Unix는 EOF·reset으로 온다. 종류로 가릴 수 없으면 ureq 문구로 본다
fn closed_without_response(e: &ureq::Error) -> bool {
    use std::io::ErrorKind;
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(err) = source {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if matches!(io.kind(), ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe) {
                return true;
            }
            if matches!(io.kind(), ErrorKind::ConnectionRefused | ErrorKind::TimedOut) {
                return false;
            }
        }
        source = err.source();
    }
    let msg = e.to_string();
    msg.contains("EOF") || msg.contains("connection closed") || msg.contains("Connection reset")
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
        let mut client = RestClient::new("127.0.0.1".to_string(), 8212, false);
        client.set_basic_auth("admin".to_string(), "password".to_string());

        assert_eq!(client.username, Some("admin".to_string()));
        assert_eq!(client.password, Some("password".to_string()));
    }

    #[test]
    fn test_rest_client_debug() {
        let client = RestClient::new("127.0.0.1".to_string(), 8212, false);
        let debug_str = format!("{:?}", client);
        assert!(debug_str.contains("RestClient"));
    }

    /// Windows는 응답 없이 닫힌 연결을 os error 10053·10054로 돌려준다. 문구가 아니라 종류로 알아본다
    #[test]
    fn a_closed_connection_is_recognized_by_its_kind_on_every_platform() {
        use std::io::{Error, ErrorKind};
        let windows_abort = Error::new(ErrorKind::ConnectionAborted, "An established connection was aborted by the software in your host machine. (os error 10053)");
        let windows_reset = Error::new(ErrorKind::ConnectionReset, "An existing connection was forcibly closed by the remote host. (os error 10054)");
        for e in [windows_abort, windows_reset, Error::new(ErrorKind::UnexpectedEof, "eof")] {
            assert!(closed_without_response(&ureq::Error::from(e)));
        }
        // 서버가 없거나 닿지 않으면 요청이 가지 않았다: 전송 오류다
        for kind in [ErrorKind::ConnectionRefused, ErrorKind::TimedOut] {
            assert!(!closed_without_response(&ureq::Error::from(Error::new(kind, "x"))));
        }
    }

    #[test]
    fn a_connection_closed_without_a_response_is_not_reported_as_success() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut sock, &mut buf);
            // 응답 없이 닫는다
        });
        let client = RestClient::new("127.0.0.1".to_string(), port, false);
        let resp = client.post("/v1/api/shutdown", Some(json!({ "waittime": 1 }))).expect("an unknown result, not a transport error");
        server.join().unwrap();
        assert!(!resp.success);
        assert_eq!(resp.data.as_ref().unwrap()["outcome"], "unknown");
        assert!(resp.error.is_some());
    }

    /// 게임 서버가 답한 HTTP 상태를 잃지 않는다: 401·403(관리자 비밀번호가 틀림)을 다른 실패와 나눠 알리게
    #[test]
    fn a_rejected_request_keeps_the_servers_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut sock, &mut buf);
            let _ = std::io::Write::write_all(&mut sock, b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\nConnection: close\r\n\r\nUnauthorized");
        });
        let client = RestClient::new("127.0.0.1".to_string(), port, false);
        let err = client.get("/v1/api/info").expect_err("a 401 is a failure");
        server.join().unwrap();
        assert!(matches!(err, ProtocolError::HttpStatus(401, _)), "{err:?}");
    }
}
