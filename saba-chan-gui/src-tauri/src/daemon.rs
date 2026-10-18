//! 데몬 HTTP 클라이언트 — 토큰은 데몬 요청에만, 401이면 토큰을 다시 읽고 한 번 재시도

use serde_json::Value;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Duration;

/// 연결 단계에서 실패한 요청 (응답 없음)
#[derive(Debug, Clone)]
pub struct NetError {
    pub message: String,
    pub code: &'static str,
}

impl NetError {
    fn from_reqwest(e: &reqwest::Error) -> Self {
        let code = if e.is_timeout() {
            "ETIMEDOUT"
        } else if e.is_connect() {
            "ECONNREFUSED"
        } else {
            "EOTHER"
        };
        Self {
            message: e.to_string(),
            code,
        }
    }
}

/// 받은 응답 (HTTP 오류 포함)
#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// JSON이면 값, 아니면 문자열 (axios 기본 동작)
    pub fn data(&self) -> Value {
        self.json()
            .unwrap_or_else(|| Value::String(String::from_utf8_lossy(&self.body).into_owned()))
    }
}

/// axios 의미의 실패: HTTP 오류면 status·body, 연결 실패면 code
#[derive(Debug, Clone)]
pub struct CallError {
    pub status: Option<u16>,
    pub body: Option<Value>,
    pub message: String,
    pub code: &'static str,
}

impl CallError {
    /// main.js errorBody: 데몬이 준 오류 본문을 살리고 extra를 덮는다
    pub fn error_body(&self, extra: Value) -> Value {
        let mut out = match &self.body {
            Some(Value::Object(m)) => m.clone(),
            _ => serde_json::Map::new(),
        };
        if let Value::Object(extra) = extra {
            out.extend(extra);
        }
        let error = match self.body.as_ref().and_then(|b| b.get("error")) {
            Some(e) if is_truthy(e) => e.clone(),
            _ => Value::String(self.message.clone()),
        };
        out.insert("error".into(), error);
        Value::Object(out)
    }
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => true,
    }
}

/// 렌더러가 부를 수 있는 데몬 경로: `/api/…`와 `/health`만. 경로를 벗어나는 표기는 막는다
pub fn is_allowed_path(path: &str) -> bool {
    if !(path.starts_with("/api/") || path == "/health" || path.starts_with("/health?")) {
        return false;
    }
    let route = path.split(['?', '#']).next().unwrap_or("");
    if path.contains('#') || path.contains('\\') || path.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    // `..`·`.` 조각(인코딩된 %2e 포함)으로 /api/ 밖으로 나가지 못하게.
    // 데몬(axum)은 원시 경로로 라우팅하므로 한 조각 안의 %2f·%5c는 이름의 일부로만 쓰인다.
    let dot_segment = route.split('/').any(|seg| {
        let seg = seg.to_ascii_lowercase().replace("%2e", ".");
        seg == ".." || seg == "."
    });
    if dot_segment {
        return false;
    }
    // Rust 셸이 직접 정하는 경로는 렌더러(확장 GUI 스크립트 포함)에 열지 않는다
    let lowered = route.to_ascii_lowercase();
    !RESERVED_ROUTES
        .iter()
        .any(|r| lowered == *r || lowered.starts_with(&format!("{r}/")))
}

/// 업데이터 실행(relaunch 대상), 클라이언트 등록, 데몬 종료는 셸 명령으로만 부른다
const RESERVED_ROUTES: &[&str] = &["/api/updates/launch-updater", "/api/client", "/api/daemon/shutdown"];

pub fn parse_method(method: &str) -> Option<reqwest::Method> {
    match method.to_ascii_uppercase().as_str() {
        "GET" => Some(reqwest::Method::GET),
        "POST" => Some(reqwest::Method::POST),
        "PUT" => Some(reqwest::Method::PUT),
        "PATCH" => Some(reqwest::Method::PATCH),
        "DELETE" => Some(reqwest::Method::DELETE),
        _ => None,
    }
}

pub struct DaemonClient {
    http: reqwest::Client,
    base: RwLock<String>,
    token: RwLock<String>,
    token_path: PathBuf,
}

impl DaemonClient {
    pub fn new(base: String, token_path: PathBuf) -> Self {
        // 리다이렉트를 따라가지 않는다 (토큰이 다른 곳으로 새지 않게). 루프백이라 프록시도 쓰지 않는다
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("reqwest client");
        Self {
            http,
            base: RwLock::new(base),
            token: RwLock::new(String::new()),
            token_path,
        }
    }

    pub fn base(&self) -> String {
        self.base.read().unwrap().clone()
    }

    pub fn set_base(&self, base: String) {
        *self.base.write().unwrap() = base;
    }

    /// 토큰 파일을 다시 읽는다. 읽었으면 true
    pub fn load_token(&self) -> bool {
        match std::fs::read_to_string(&self.token_path) {
            Ok(text) => {
                let token = text.trim().to_string();
                if token.is_empty() {
                    return false;
                }
                let mut cur = self.token.write().unwrap();
                if *cur != token {
                    let short: String = token.chars().take(8).collect();
                    tracing::info!("[Auth] IPC token loaded: {}… from {}", short, self.token_path.display());
                    *cur = token;
                }
                true
            }
            Err(e) => {
                tracing::warn!("[Auth] IPC token not found, auth may fail: {}", e);
                false
            }
        }
    }

    pub fn token(&self) -> String {
        if self.token.read().unwrap().is_empty() {
            self.load_token();
        }
        self.token.read().unwrap().clone()
    }

    /// 토큰·출처 헤더를 붙인 요청. 시간 제한은 부르는 쪽이 정한다 (SSE는 없음)
    pub fn builder(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base(), path))
            .header("X-Saba-Client", "gui");
        // 토큰은 이 PC의 데몬에만 보낸다 (IPC_BASE가 다른 호스트를 가리켜도)
        let token = self.token();
        if !token.is_empty() && self.base_is_loopback() {
            req = req.header("X-Saba-Token", token);
        }
        req
    }

    fn base_is_loopback(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base()) else {
            return false;
        };
        let host = url
            .host_str()
            .unwrap_or("")
            .trim_start_matches('[')
            .trim_end_matches(']');
        host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }

    async fn send_once(
        &self,
        method: &reqwest::Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Resp, NetError> {
        let mut req = self.builder(method.clone(), path).timeout(timeout);
        if let Some(b) = body {
            req = req.json(b);
        }
        let res = req.send().await.map_err(|e| NetError::from_reqwest(&e))?;
        let status = res.status().as_u16();
        let body = res.bytes().await.map_err(|e| NetError::from_reqwest(&e))?.to_vec();
        Ok(Resp { status, body })
    }

    /// 요청 하나. 401이면 토큰을 다시 읽어 한 번 재시도 (데몬 재시작으로 토큰이 바뀐 경우).
    /// 릴레이 프록시(/api/relay/)의 401은 릴레이 인증 실패라 재시도하지 않는다
    pub async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Resp, NetError> {
        let res = self.send_once(&method, path, body, timeout).await?;
        if res.status != 401 || path.contains("/api/relay/") {
            return Ok(res);
        }
        if !self.load_token() {
            return Ok(res);
        }
        tracing::info!("[Auth] Token reloaded after 401, retrying {}", path);
        self.send_once(&method, path, body, timeout).await
    }

    /// axios처럼 2xx만 성공으로 본다
    pub async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        timeout_ms: u64,
    ) -> Result<Value, CallError> {
        match self.send(method, path, body, Duration::from_millis(timeout_ms)).await {
            Ok(r) if r.is_success() => Ok(r.data()),
            Ok(r) => Err(CallError {
                status: Some(r.status),
                body: Some(r.data()),
                message: format!("Request failed with status code {}", r.status),
                code: "ERR_BAD_RESPONSE",
            }),
            Err(e) => Err(CallError {
                status: None,
                body: None,
                message: e.message,
                code: e.code,
            }),
        }
    }

    pub async fn get(&self, path: &str, timeout_ms: u64) -> Result<Value, CallError> {
        self.call(reqwest::Method::GET, path, None, timeout_ms).await
    }

    pub async fn post(&self, path: &str, body: &Value, timeout_ms: u64) -> Result<Value, CallError> {
        self.call(reqwest::Method::POST, path, Some(body), timeout_ms).await
    }

    pub async fn put(&self, path: &str, body: &Value, timeout_ms: u64) -> Result<Value, CallError> {
        self.call(reqwest::Method::PUT, path, Some(body), timeout_ms).await
    }

    pub async fn delete(&self, path: &str, timeout_ms: u64) -> Result<Value, CallError> {
        self.call(reqwest::Method::DELETE, path, None, timeout_ms).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn allows_only_api_and_health() {
        for ok in [
            "/api/servers",
            "/api/instance/a1/stop",
            "/health",
            "/api/daemon/console?since=3&count=10",
            "/api/server/My%20Server/status",
            "/api/provision-progress/a%2Fb",
            "/api/clients/launch",
        ] {
            assert!(is_allowed_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/",
            "/apix/servers",
            "api/servers",
            "http://evil/api/x",
            "//evil/api/",
            "/api/../secret",
            "/api/%2e%2e/secret",
            "/api/%2E/secret",
            "/api/a/.%2e",
            "/api/a\\b",
            "/api/a b",
            "/api/a\nb",
            "/healthz",
            "/api/x#frag",
            "/api/updates/launch-updater",
            "/api/client/register",
            "/api/Client/heartbeat",
            "/api/daemon/shutdown",
        ] {
            assert!(!is_allowed_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn error_body_keeps_daemon_fields() {
        let e = CallError {
            status: Some(409),
            body: Some(serde_json::json!({"error_code": "busy", "error": "in use"})),
            message: "Request failed with status code 409".into(),
            code: "ERR_BAD_RESPONSE",
        };
        let b = e.error_body(serde_json::json!({"ok": false}));
        assert_eq!(b["error_code"], "busy");
        assert_eq!(b["error"], "in use");
        assert_eq!(b["ok"], false);
        let e = CallError {
            status: None,
            body: None,
            message: "refused".into(),
            code: "ECONNREFUSED",
        };
        assert_eq!(e.error_body(Value::Null)["error"], "refused");
    }

    /// 토큰이 맞을 때만 200을 주는 작은 서버. 받은 토큰을 기록한다
    async fn token_server(valid: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let token = req
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("x-saba-token: ")
                            .or_else(|| l.strip_prefix("X-Saba-Token: "))
                    })
                    .unwrap_or("")
                    .to_string();
                seen2.lock().unwrap().push(token.clone());
                let (status, body) = if token == valid {
                    ("200 OK", "{\"ok\":true}")
                } else {
                    ("401 Unauthorized", "{}")
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), seen)
    }

    #[tokio::test]
    async fn reloads_token_and_retries_once_on_401() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join(".ipc_token");
        std::fs::write(&token_path, "old-token\n").unwrap();
        let (base, seen) = token_server("new-token").await;
        let client = DaemonClient::new(base, token_path.clone());
        assert_eq!(client.token(), "old-token");

        // 데몬이 재시작해 토큰을 바꿨다
        std::fs::write(&token_path, "new-token").unwrap();
        let res = client
            .send(reqwest::Method::GET, "/api/servers", None, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(res.status, 200);
        assert_eq!(*seen.lock().unwrap(), vec!["old-token", "new-token"]);
    }

    #[tokio::test]
    async fn relay_401_is_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join(".ipc_token");
        std::fs::write(&token_path, "old-token").unwrap();
        let (base, seen) = token_server("new-token").await;
        let client = DaemonClient::new(base, token_path.clone());
        client.load_token();
        std::fs::write(&token_path, "new-token").unwrap();
        let res = client
            .send(reqwest::Method::GET, "/api/relay/agent", None, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(res.status, 401);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn token_goes_only_to_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join(".ipc_token");
        std::fs::write(&token_path, "secret").unwrap();
        let has_token = |base: &str| {
            let client = DaemonClient::new(base.into(), token_path.clone());
            let req = client.builder(reqwest::Method::GET, "/health").build().unwrap();
            req.headers().contains_key("X-Saba-Token")
        };
        assert!(has_token("http://127.0.0.1:57474"));
        assert!(has_token("http://localhost:57474"));
        assert!(has_token("http://[::1]:57474"));
        assert!(!has_token("http://192.168.0.10:57474"));
        assert!(!has_token("http://example.com:57474"));
    }

    #[tokio::test]
    async fn refused_connection_reports_code() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let dir = tempfile::tempdir().unwrap();
        let client = DaemonClient::new(format!("http://{addr}"), dir.path().join("none"));
        // Windows는 닫힌 로컬 포트에도 SYN을 몇 번 다시 보내 거부까지 2초쯤 걸린다
        let err = client.get("/health", 5000).await.unwrap_err();
        assert_eq!(err.code, "ECONNREFUSED");
        assert!(err.status.is_none());
    }
}
