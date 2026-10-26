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

/// 렌더러의 데몬 요청을 받을지. 경로 검사에 더해, 동의 기록은 셸 명령만 보낸다
pub fn is_allowed_request(method: &reqwest::Method, path: &str) -> bool {
    if !is_allowed_path(path) {
        return false;
    }
    // 목록(GET)과 철회(DELETE)는 렌더러에 남긴다. 기록하는 쪽만 막는다
    let route = path.split('?').next().unwrap_or("").to_ascii_lowercase();
    let grants = CONSENT_ROUTES.iter().any(|r| route.starts_with(&format!("{r}/")));
    !(grants && *method != reqwest::Method::DELETE && *method != reqwest::Method::GET)
}

/// 확인되지 않은 구성요소 동의 기록 (native::sandbox_consent_allow가 확인 창을 띄운 뒤 보낸다)
const CONSENT_ROUTES: &[&str] = &["/api/sandbox/unisolated", "/api/sandbox/extension-gui"];

/// 업데이터 실행(relaunch 대상), 클라이언트 등록, 데몬 종료는 셸 명령으로만 부른다
const RESERVED_ROUTES: &[&str] = &["/api/updates/launch-updater", "/api/client", "/api/daemon/shutdown"];

/// 머리에 실을 수 있게 퍼센트 인코딩 (이름은 한국어일 수 있다)
fn encode_target(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

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
    /// 고른 원격 데몬 (G1, 설정 `gui.daemonTarget`). 없으면 이 PC의 데몬
    target: RwLock<Option<String>>,
    /// 대상이 바뀔 때마다 오른다 (이벤트 스트림이 새 대상으로 다시 붙는다)
    target_gen: tokio::sync::watch::Sender<u64>,
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
            target: RwLock::new(None),
            target_gen: tokio::sync::watch::Sender::new(0),
        }
    }

    pub fn target(&self) -> Option<String> {
        self.target.read().unwrap().clone()
    }

    /// 저장·사용하는 꼴: 앞뒤 공백을 떼고, 비면 이 PC(None)
    pub fn normalize_target(target: Option<String>) -> Option<String> {
        target.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    }

    /// 다룰 데몬을 바꾼다 (빈 값은 이 PC). 바뀌었으면 세대를 올린다
    pub fn set_target(&self, target: Option<String>) {
        let target = Self::normalize_target(target);
        let mut cur = self.target.write().unwrap();
        if *cur != target {
            *cur = target;
            self.target_gen.send_modify(|g| *g += 1);
        }
    }

    pub fn target_generation(&self) -> u64 {
        *self.target_gen.borrow()
    }

    pub fn watch_target(&self) -> tokio::sync::watch::Receiver<u64> {
        self.target_gen.subscribe()
    }

    pub async fn identity_valid(&self, timeout_ms: u64) -> bool {
        if self.token().is_empty() || !self.base_is_loopback() { return false; }
        self.verified_health(timeout_ms).await.1
    }

    /// `/health` 한 번: 본문(또는 연결 오류 문구)과, 그 응답이 이 토큰의 신원 증명을 실었는가
    pub async fn verified_health(&self, timeout_ms: u64) -> (Result<Value, String>, bool) {
        let token = self.token();
        let challenge = uuid::Uuid::new_v4().to_string();
        let result = self.builder(reqwest::Method::GET, "/health").timeout(Duration::from_millis(timeout_ms))
            .header("X-Saba-Challenge", &challenge).send().await;
        let response = match result {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => return (Err(format!("Request failed with status code {}", r.status().as_u16())), false),
            Err(e) => return (Err(e.to_string()), false),
        };
        // JSON이 아닌 본문도 `get`처럼 글자로 받는다 (다른 프로그램이 포트를 쥔 경우 "토큰 불일치"로 보이게)
        let body = match response.bytes().await {
            Ok(bytes) => Resp { status: 200, body: bytes.to_vec() }.data(),
            Err(e) => return (Err(e.to_string()), false),
        };
        let valid = !token.is_empty() && self.base_is_loopback() && body.get("proof").and_then(Value::as_str).is_some_and(|proof| {
            use hmac::{Hmac, Mac};
            let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(token.as_bytes()) else { return false };
            mac.update(challenge.as_bytes());
            hex::decode(proof).ok().is_some_and(|bytes| mac.verify_slice(&bytes).is_ok())
        });
        (Ok(body), valid)
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
                    tracing::info!("[Auth] IPC token loaded from {}", self.token_path.display());
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
        if !token.is_empty() && self.base_is_loopback() && path != "/health" {
            req = req.header("X-Saba-Token", token);
        }
        // 고른 원격 데몬은 이 PC의 데몬이 중계한다 (01 §1.4.2 7). 데몬이 살아 있는지는 늘 이 PC에 묻는다
        if let Some(target) = self.target().filter(|_| !path.starts_with("/health")) {
            req = req.header("X-Saba-Target", encode_target(&target));
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
        // 원격 데몬이 이 PC의 짝을 받지 않은 것(중계된 401)은 이 PC의 토큰 문제가 아니다
        if res.data()["error_code"] == "remote_unauthorized" {
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
    fn consent_grants_go_through_the_shell_only() {
        use reqwest::Method;
        for (m, p) in [
            (Method::POST, "/api/sandbox/unisolated/module-x"),
            (Method::POST, "/api/sandbox/extension-gui/ext-x"),
            (Method::PUT, "/api/sandbox/extension-gui/ext-x"),
            (Method::POST, "/api/Sandbox/Extension-GUI/ext-x?y=1"),
        ] {
            assert!(!is_allowed_request(&m, p), "{m} {p}");
        }
        // 목록과 철회(권한을 줄이는 쪽)는 렌더러에 남긴다
        for (m, p) in [
            (Method::GET, "/api/sandbox/unisolated"),
            (Method::GET, "/api/sandbox/extension-gui"),
            (Method::DELETE, "/api/sandbox/unisolated/module-x"),
            (Method::DELETE, "/api/sandbox/extension-gui/ext-x"),
            (Method::POST, "/api/servers"),
        ] {
            assert!(is_allowed_request(&m, p), "{m} {p}");
        }
        assert!(!is_allowed_request(&Method::GET, "/api/updates/launch-updater"));
    }

    /// 로그는 파일로 남는다: 토큰은 앞부분도 쓰지 않는다
    #[test]
    fn token_load_log_has_no_token() {
        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "abcdef1234567890secret").unwrap();
        let buf = Buf::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        // 스레드 범위 구독자는 병렬 시험에서 로그 자리 캐시 때문에 놓칠 수 있어 전역으로 건다 (이 시험만 건다)
        tracing::subscriber::set_global_default(subscriber).unwrap();
        assert!(DaemonClient::new("http://127.0.0.1:1".into(), path).load_token());
        let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("IPC token loaded"), "{log}");
        assert!(!log.contains("abcd"), "{log}");
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

    /// 고른 원격 데몬이 이 PC의 짝을 받지 않은 401(`remote_unauthorized`)은 토큰을 다시 읽어 또 보내지 않는다
    #[tokio::test]
    async fn a_relayed_remote_refusal_is_not_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(Mutex::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await;
                *seen.lock().unwrap() += 1;
                let body = "{\"error\":\"remote_unauthorized\",\"error_code\":\"remote_unauthorized\"}";
                let resp = format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join(".ipc_token");
        std::fs::write(&token_path, "t").unwrap();
        let client = DaemonClient::new(base, token_path);
        client.set_target(Some("host-b".into()));
        let res = client.send(reqwest::Method::POST, "/api/instance/a/stop", None, Duration::from_secs(2)).await.unwrap();
        assert_eq!(res.status, 401);
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn token_goes_only_to_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join(".ipc_token");
        std::fs::write(&token_path, "secret").unwrap();
        let has_token = |base: &str| {
            let client = DaemonClient::new(base.into(), token_path.clone());
            let req = client.builder(reqwest::Method::GET, "/api/version").build().unwrap();
            req.headers().contains_key("X-Saba-Token")
        };
        assert!(has_token("http://127.0.0.1:57474"));
        assert!(has_token("http://localhost:57474"));
        assert!(has_token("http://[::1]:57474"));
        assert!(!has_token("http://192.168.0.10:57474"));
        assert!(!has_token("http://example.com:57474"));
        let local = DaemonClient::new("http://127.0.0.1:57474".into(), token_path);
        assert!(!local.builder(reqwest::Method::GET, "/health").build().unwrap().headers().contains_key("X-Saba-Token"));
    }

    /// 고른 원격 데몬(G1)은 데몬 요청마다 `X-Saba-Target`(퍼센트 인코딩)으로 이 PC의 데몬에 알린다.
    /// 이 PC의 데몬이 살아 있는지 보는 `/health`에는 싣지 않는다. 바꾸면 세대가 올라 이벤트 스트림이 다시 붙는다
    #[test]
    fn a_chosen_remote_daemon_is_named_on_daemon_requests_but_not_health() {
        let dir = tempfile::tempdir().unwrap();
        let client = DaemonClient::new("http://127.0.0.1:57474".into(), dir.path().join(".ipc_token"));
        let target = |path: &str| {
            let req = client.builder(reqwest::Method::GET, path).build().unwrap();
            req.headers().get("X-Saba-Target").map(|v| v.to_str().unwrap().to_string())
        };
        assert_eq!(target("/api/servers"), None);
        let before = client.target_generation();
        client.set_target(Some("게임 PC".into()));
        assert_eq!(client.target().as_deref(), Some("게임 PC"));
        assert_ne!(client.target_generation(), before);
        assert_eq!(target("/api/servers").as_deref(), Some("%EA%B2%8C%EC%9E%84%20PC"));
        assert_eq!(target("/health"), None);
        let same = client.target_generation();
        client.set_target(Some("게임 PC".into()));
        assert_eq!(client.target_generation(), same);
        client.set_target(Some("  ".into()));
        assert_eq!(client.target(), None);
        assert_eq!(target("/api/servers"), None);
    }

    /// 상태 확인 한 번에 /health 하나: 신원 증명이 실린 그 응답을 상태 본문으로도 쓴다 (폴링마다 두 번 묻지 않게)
    #[tokio::test]
    async fn one_health_request_gives_the_status_and_the_identity() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(Mutex::new(0));
        let count2 = count.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                let challenge = req.lines().find_map(|l| l.strip_prefix("x-saba-challenge: ")).unwrap_or("").to_string();
                *count2.lock().unwrap() += 1;
                use hmac::{Hmac, Mac};
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"secret").unwrap();
                mac.update(challenge.as_bytes());
                let body = format!("{{\"status\":\"ok\",\"proof\":\"{}\"}}", hex::encode(mac.finalize().into_bytes()));
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join(".ipc_token");
        std::fs::write(&token_path, "secret").unwrap();
        let client = DaemonClient::new(format!("http://{addr}"), token_path);
        let (health, identity) = client.verified_health(2000).await;
        assert_eq!(health.unwrap()["status"], "ok");
        assert!(identity);
        assert_eq!(*count.lock().unwrap(), 1);
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
