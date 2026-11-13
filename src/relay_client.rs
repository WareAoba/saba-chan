//! 릴레이 서버에 붙는 클라이언트: 주소 해석, HTTP 클라이언트, 노드 토큰 저장, 요청 서명.
//! 릴레이 프록시 핸들러·릴레이 에이전트·포트 확인·원격 데몬이 같이 쓴다

use hmac::{Hmac, KeyInit, Mac};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::ipc::IPCServer;

type HmacSha256 = Hmac<Sha256>;

/// Percent-encode a URL path segment (RFC 3986 unreserved chars only)
pub(crate) fn encode_path_segment(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            write!(out, "%{:02X}", b).unwrap();
        }
    }
    out
}

/// Resolve relay server URL (override via query param, fallback to default)
pub(crate) fn resolve_relay_url(relay_url: Option<&str>) -> String {
    let url = relay_url
        .filter(|u| !u.is_empty())
        .unwrap_or("https://saba-chan.online");
    url.trim_end_matches('/').to_string()
}

/// 노드 토큰을 실어 보내거나 받는(페어링) 요청의 릴레이 주소. 설정한 릴레이(없으면 기본)만 쓴다:
/// `?relayUrl=`로 아무 호스트나 넣어 토큰을 빼내지 못하게. 요청한 주소가 설정과 같거나
/// 루프백(모의 릴레이 시험)일 때만 그것을 따른다
pub(crate) fn signed_relay_url(configured: &str, requested: Option<&str>) -> String {
    let configured = resolve_relay_url(Some(configured.trim()));
    let Some(requested) = requested.map(str::trim).filter(|u| !u.is_empty()) else { return configured };
    let requested = resolve_relay_url(Some(requested));
    let loopback = reqwest::Url::parse(&requested)
        .ok()
        .and_then(|u| u.host_str().map(|h| matches!(h, "localhost" | "127.0.0.1" | "[::1]")))
        .unwrap_or(false);
    if requested == configured || loopback {
        requested
    } else {
        tracing::warn!("Ignoring relayUrl {} for a signed request; using the configured relay", requested);
        configured
    }
}

/// 페어링 주소. 서명 요청과 같은 규칙에 더해, https(또는 루프백 http)가 아니면 거절한다:
/// 요청한 주소가 평문 http면 설정 주소로 바꿔 치우지 않고 알린다
pub(crate) fn pairing_relay_url(configured: &str, requested: Option<&str>) -> Result<String, String> {
    if let Some(problem) = requested.and_then(crate::config_store::relay_url_problem) {
        return Err(problem);
    }
    let base = signed_relay_url(configured, requested);
    crate::config_store::relay_url_problem(&base).map_or(Ok(base), Err)
}

/// Create an HTTP client with 5s timeout for relay requests.
/// Automatically includes X-Saba-Client: 1 header required by nginx.
/// 한 번 만든 클라이언트를 함께 쓴다: 요청마다 TLS 설정을 다시 만들지 않고 릴레이 연결을 이어 쓴다 (상태 확인은 5초마다).
/// 시스템 프록시는 만들 때 읽으므로 `RELAY_CLIENT_LIFETIME`마다 새로 만들어 프록시를 바꿔도 데몬 재시작 없이 따른다
pub(crate) fn relay_client() -> Result<reqwest::Client, reqwest::Error> {
    static CLIENT: std::sync::Mutex<Option<(std::time::Instant, reqwest::Client)>> = std::sync::Mutex::new(None);
    cached_relay_client(&CLIENT, std::time::Instant::now())
}

/// 릴레이 답을 기다리는 기본 시간
pub(crate) const RELAY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// 기본보다 오래 기다리는 릴레이 경로: (GET인가, `/api/nodes/:guild/` 뒤 경로, 초). `/`로 끝나면 그 아래 경로 모두.
/// 릴레이가 그 경로에서 바깥(제공사·디스코드)을 기다리는 시간보다 길게 잡는다. 릴레이 쪽 시간을 바꾸면 여기도 맞춘다
const RELAY_WAITS: &[(bool, &str, u64)] = &[
    // 릴레이가 제공사에 잔액을 8초까지 묻는다 (기본 5초면 느린 제공사 때 사용량까지 못 받는다)
    (true, "chatbot/credit", 12),
    // 릴레이가 제공사에 모델 목록을 15초까지 묻는다 (캐시가 빈 첫 시도)
    (true, "chatbot/models", 18),
    // 길드 전체 멤버: 릴레이는 디스코드에 8초까지 기다린다 (그 뒤에는 캐시로 답한다)
    (true, "discord-members", 15),
    // 음소거 목록을 위해 릴레이가 길드 멤버를 디스코드에서 읽을 수 있다 (8초까지)
    (true, "moderation", 15),
    // 디스코드 쪽 일 (청소는 메시지를 쪽마다 읽는다)
    (false, "moderation/", 60),
];

/// 이 릴레이 요청을 기다릴 시간 (`RELAY_WAITS`, 없으면 `RELAY_WAIT`)
pub(crate) fn relay_wait(method: &reqwest::Method, api_path: &str) -> std::time::Duration {
    let rest = api_path.strip_prefix("/api/nodes/").and_then(|p| p.split_once('/')).map_or("", |(_, rest)| rest);
    let get = method == reqwest::Method::GET;
    RELAY_WAITS
        .iter()
        .find(|(g, tail, _)| *g == get && if tail.ends_with('/') { rest.starts_with(tail) } else { rest == *tail })
        .map_or(RELAY_WAIT, |(_, _, secs)| std::time::Duration::from_secs(*secs))
}

/// 함께 쓰는 릴레이 클라이언트를 이만큼 쓰면 새로 만든다
const RELAY_CLIENT_LIFETIME: std::time::Duration = std::time::Duration::from_secs(600);

fn cached_relay_client(
    slot: &std::sync::Mutex<Option<(std::time::Instant, reqwest::Client)>>,
    now: std::time::Instant,
) -> Result<reqwest::Client, reqwest::Error> {
    let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((built, client)) = slot.as_ref()
        && now.saturating_duration_since(*built) < RELAY_CLIENT_LIFETIME
    {
        return Ok(client.clone());
    }
    let client = relay_client_with_timeout(RELAY_WAIT)?;
    *slot = Some((now, client.clone()));
    Ok(client)
}

/// 릴레이에 붙는 HTTP 클라이언트 (`X-Saba-Client: 1`). 요청 API와 에이전트의 긴 폴링이 시간 제한만 달리해 같이 쓴다
pub(crate) fn relay_client_with_timeout(timeout: std::time::Duration) -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-Saba-Client", reqwest::header::HeaderValue::from_static("1"));
    saba_chan_updater_lib::tls::http_client_builder().timeout(timeout).default_headers(headers).build()
}

/// Parse a node token (sbn_{nodeId}.{secret}) into (nodeId, secret).
pub(crate) fn parse_node_token(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix("sbn_")?;
    let dot = rest.find('.')?;
    Some((&rest[..dot], &rest[dot + 1..]))
}

/// 봇 설정의 `cloud.hostId`를 노드 토큰(`sbn_<hostId>.…`)에 맞춘다. 토큰이 기준이고, 없으면 비운다.
/// 토큰을 쓰거나 지우는 곳(페어링 완료, 토큰 저장, 연결 해제)과 봇 설정 저장이 모두 부른다.
pub(crate) async fn sync_host_id(state: &IPCServer, token: Option<&str>) {
    let host = token.and_then(|t| parse_node_token(t.trim())).map(|(id, _)| id).unwrap_or("");
    let mut cloud = state.config_store.get_bot_config_json().await.get("cloud").cloned().unwrap_or_else(|| json!({}));
    if cloud.get("hostId").and_then(Value::as_str).unwrap_or("") == host {
        return;
    }
    // 병합은 최상위 키 단위라 cloud 묶음을 통째로 보낸다
    cloud["hostId"] = Value::String(host.to_string());
    if let Err(e) = state.config_store.set_bot_config_from_json(json!({ "cloud": cloud })).await {
        tracing::warn!("Failed to sync relay host id: {}", e);
    }
}

/// 노드 토큰. 설정 파일 하나(config.json `node.token`)에 둔다
pub(crate) fn load_node_token() -> Option<String> {
    node_token_in(&ConfigFile::default_location())
}

/// 요청마다 부르는 곳(원격 데몬 중계)용: 잠그거나 이관하지 않고, 바뀌지 않은 설정 파일은 다시 읽지 않는다
pub(crate) fn peek_node_token() -> Option<String> {
    token_of(&ConfigFile::default_location().peek_section("node"))
}

pub(crate) fn node_token_in(file: &ConfigFile) -> Option<String> {
    token_of(&file.section("node"))
}

fn token_of(node: &Value) -> Option<String> {
    node.get("token")
        .and_then(Value::as_str)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// 토큰을 쓴 뒤 따라야 할 것: hostId를 토큰에 맞추고, 바뀌었으면 relay agent를 새 토큰으로 다시 띄운다.
/// 토큰을 쓰는 곳(페어링 완료, 토큰 저장, 가져오기)이 모두 부른다
pub(crate) async fn adopt_node_token(state: &IPCServer, token: &str, changed: bool) {
    sync_host_id(state, Some(token)).await;
    if changed {
        crate::relay_agent::restart(state).await;
        // 원격 포트 주소는 PC(노드 토큰)마다 올린다
        crate::remote_daemon::apply(state).await;
    }
}

/// 연결 해제: 토큰을 지우기 전에 릴레이 에이전트를 멈춰 더는 원격 명령을 받지 않는다.
/// 토큰이 없으면 다음 기동 때도 자동 시작하지 않는다. 연결 해제 API와 개인정보 지우기가 부른다
pub(crate) async fn disconnect(state: &IPCServer) -> std::io::Result<()> {
    state.relay_agent.stop();
    write_node_token(None)?;
    sync_host_id(state, None).await;
    Ok(())
}

/// 노드 토큰을 바꾼다(`None`이나 빈 값이면 지운다). 값이 바뀌었으면 true
pub(crate) fn write_node_token(token: Option<&str>) -> std::io::Result<bool> {
    write_node_token_in(&ConfigFile::default_location(), token)
}

fn write_node_token_in(file: &ConfigFile, token: Option<&str>) -> std::io::Result<bool> {
    let token = token.map(str::trim).filter(|t| !t.is_empty());
    let mut changed = false;
    file.update(|doc| {
        let node = doc.entry("node").or_insert_with(|| json!({}));
        if !node.is_object() {
            *node = json!({});
        }
        let node = node.as_object_mut().expect("just made an object");
        changed = node.get("token").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) != token;
        match token {
            Some(t) => node.insert("token".into(), json!(t)),
            None => node.remove("token"),
        };
    })?;
    Ok(changed)
}

/// Generate HMAC-SHA256 signed headers for relay server authentication.
/// Returns None if no valid node token is available.
pub(crate) fn signed_headers(
    method: &str,
    url_path: &str,
    body: Option<&str>,
) -> Option<Vec<(String, String)>> {
    signed_headers_with(&peek_node_token()?, method, url_path, body)
}

/// 노드 토큰으로 서명한 요청. 본문이 있으면 서명한 문자열 그대로 JSON으로 싣는다. 토큰이 잘못됐으면 None
pub(crate) fn signed_request(
    client: &reqwest::Client,
    method: reqwest::Method,
    base: &str,
    path: &str,
    token: &str,
    body: Option<String>,
) -> Option<reqwest::RequestBuilder> {
    let headers = signed_headers_with(token, method.as_str(), path, body.as_deref())?;
    let mut req = client.request(method, format!("{}{}", base, path));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(body) = body {
        req = req.header("Content-Type", "application/json").body(body);
    }
    Some(req)
}

/// 정해 둔 토큰으로 서명한다 (릴레이 에이전트 세션은 시작할 때의 주소·토큰 쌍을 끝까지 쓴다)
pub(crate) fn signed_headers_with(
    token: &str,
    method: &str,
    url_path: &str,
    body: Option<&str>,
) -> Option<Vec<(String, String)>> {
    let (_, secret) = parse_node_token(token)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let nonce = uuid::Uuid::new_v4().to_string();
    let body_str = body.unwrap_or("");

    let payload = format!(
        "{}\n{}\n{}\n{}\n{}",
        method.to_uppercase(),
        url_path,
        ts,
        nonce,
        body_str
    );

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());

    Some(vec![
        ("Authorization".into(), format!("Bearer {}", token)),
        ("x-request-timestamp".into(), ts.to_string()),
        ("x-request-nonce".into(), nonce),
        ("x-request-signature".into(), sig),
    ])
}

/// Apply signed headers to a RequestBuilder. If no node token is available,
/// the request is sent without authentication.
pub(crate) fn apply_signed_headers(
    builder: reqwest::RequestBuilder,
    method: &str,
    url_path: &str,
    body: Option<&str>,
) -> reqwest::RequestBuilder {
    with_headers_if_secure(builder, signed_headers(method, url_path, body))
}

/// 서명 머리(노드 토큰 포함)는 https 또는 루프백 http 릴레이에만 붙인다. 예전에 저장된 평문 http 주소면
/// 서명 없이 보낸다(릴레이가 인증 실패로 답한다). 주소를 읽을 수 없으면 붙이지 않는다
fn with_headers_if_secure(builder: reqwest::RequestBuilder, headers: Option<Vec<(String, String)>>) -> reqwest::RequestBuilder {
    let Some(headers) = headers else { return builder };
    let secure = builder
        .try_clone()
        .and_then(|b| b.build().ok())
        .is_some_and(|req| crate::config_store::relay_url_problem(req.url().as_str()).is_none());
    if !secure {
        tracing::warn!("Not signing a relay request to a plain-http relay; it must be https (http only for localhost)");
        return builder;
    }
    let mut b = builder;
    for (k, v) in headers {
        b = b.header(&k, &v);
    }
    b
}


/// 페어링으로 받은 노드 토큰을 저장한 결과
#[derive(Debug, PartialEq)]
pub(crate) struct StoredToken {
    /// 이 PC에 그 토큰이 저장돼 있다 (방금 썼거나 이미 같은 값)
    pub(crate) saved: bool,
    /// 파일을 새 값으로 바꿨다 (이때만 호스트 id를 맞추고 relay agent를 다시 띄운다)
    pub(crate) changed: bool,
}

/// 받은 토큰을 설정 파일에 쓴다. 토큰이 없거나(수령 기한이 지남) 쓰지 못하면 저장하지 않은 것이고,
/// 같은 값이 이미 있으면 다시 쓰지 않는다 (폴링이 claimed를 여러 번 받는다)
pub(crate) fn store_node_token(file: &ConfigFile, token: Option<&str>) -> StoredToken {
    let Some(token) = token.filter(|t| !t.trim().is_empty()) else {
        tracing::warn!("Pairing was claimed but the relay no longer hands out the node token");
        return StoredToken { saved: false, changed: false };
    };
    if node_token_in(file).as_deref() == Some(token.trim()) {
        return StoredToken { saved: true, changed: false };
    }
    match write_node_token_in(file, Some(token)) {
        Ok(changed) => StoredToken { saved: true, changed },
        Err(e) => {
            tracing::warn!("Failed to auto-save node token: {}", e);
            StoredToken { saved: false, changed: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시스템 프록시를 바꾸면 데몬을 다시 켜지 않아도 따르게, 함께 쓰는 클라이언트는 일정 시간 뒤 새로 만든다
    #[test]
    fn the_shared_relay_client_is_rebuilt_after_its_lifetime() {
        let slot = std::sync::Mutex::new(None);
        let start = std::time::Instant::now();
        super::cached_relay_client(&slot, start).unwrap();
        super::cached_relay_client(&slot, start + std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(slot.lock().unwrap().as_ref().map(|(at, _)| *at), Some(start), "reused within the lifetime");
        let later = start + super::RELAY_CLIENT_LIFETIME;
        super::cached_relay_client(&slot, later).unwrap();
        assert_eq!(slot.lock().unwrap().as_ref().map(|(at, _)| *at), Some(later), "rebuilt after it");
    }

    /// 토큰을 저장하지 못했으면 연결된 것으로 보이면 안 된다. 같은 토큰을 다시 받으면 다시 쓰지 않는다
    #[test]
    fn a_claimed_pairing_reports_whether_the_token_was_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(&tmp.path().join("data"));
        assert_eq!(store_node_token(&file, None), StoredToken { saved: false, changed: false }, "claimed after the token expired");
        assert_eq!(store_node_token(&file, Some("sbn_a.x")), StoredToken { saved: true, changed: true });
        assert_eq!(file.section("node")["token"], "sbn_a.x", "the token lives in config.json");
        assert_eq!(store_node_token(&file, Some("sbn_a.x")), StoredToken { saved: true, changed: false }, "the next poll sees claimed again");
        // 쓸 수 없는 곳(폴더 자리에 파일)이면 저장하지 않은 것이다
        let blocked = tmp.path().join("file");
        std::fs::write(&blocked, "").unwrap();
        assert_eq!(store_node_token(&ConfigFile::at(&blocked), Some("sbn_b.y")), StoredToken { saved: false, changed: false });
    }

    /// 연결 해제는 토큰만 지우고 같은 파일의 다른 설정은 남긴다
    #[test]
    fn removing_the_node_token_keeps_the_rest_of_the_config() {
        let tmp = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(tmp.path());
        file.set_section("gui", json!({ "language": "ko" })).unwrap();
        assert!(write_node_token_in(&file, Some("sbn_a.x")).unwrap());
        assert!(!write_node_token_in(&file, Some(" sbn_a.x ")).unwrap(), "same token, nothing changed");
        assert!(write_node_token_in(&file, None).unwrap());
        assert_eq!(node_token_in(&file), None);
        assert_eq!(file.section("gui")["language"], "ko");
    }

    #[test]
    fn signing_uses_the_token_it_is_given() {
        let headers = signed_headers_with("sbn_node1.secret", "GET", "/poll", None).unwrap();
        assert!(headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer sbn_node1.secret"));
        assert!(signed_headers_with("not-a-token", "GET", "/poll", None).is_none());
    }

    #[test]
    fn signed_requests_never_follow_an_arbitrary_relay_url() {
        let cfg = "https://saba-chan.online/";
        assert_eq!(signed_relay_url(cfg, None), "https://saba-chan.online");
        assert_eq!(signed_relay_url(cfg, Some("https://evil.example")), "https://saba-chan.online");
        assert_eq!(signed_relay_url("", Some("https://evil.example")), "https://saba-chan.online");
        assert_eq!(signed_relay_url(cfg, Some("https://saba-chan.online")), "https://saba-chan.online");
        assert_eq!(signed_relay_url(cfg, Some("http://127.0.0.1:9000/")), "http://127.0.0.1:9000", "mock relay");
        assert_eq!(signed_relay_url("https://my.relay", None), "https://my.relay");
    }

    /// 예전에 저장된 평문 http 릴레이 주소로는 노드 토큰을 싣지 않는다
    #[test]
    fn signed_headers_are_only_attached_for_an_https_or_loopback_relay() {
        let headers = || Some(vec![("Authorization".to_string(), "Bearer sbn_a.b".to_string())]);
        let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();
        let auth = |url: &str| with_headers_if_secure(client.get(url), headers()).build().unwrap().headers().contains_key("authorization");
        assert!(!auth("http://old.example/api/x"));
        assert!(!auth("http://127.0.0.2:9/api/x"));
        assert!(auth("https://saba-chan.online/api/x"));
        assert!(auth("http://127.0.0.1:9/api/x"));
    }

    /// 페어링은 받은 노드 토큰이 평문으로 오가지 않게 https(또는 루프백 모의 릴레이)에서만 한다
    #[test]
    fn pairing_only_goes_to_an_https_or_loopback_relay() {
        assert_eq!(pairing_relay_url("", None).unwrap(), "https://saba-chan.online");
        assert_eq!(pairing_relay_url("https://my.relay", Some("http://127.0.0.1:9000")).unwrap(), "http://127.0.0.1:9000");
        assert!(pairing_relay_url("https://my.relay", Some("http://evil.example")).is_err(), "a plain-http request is refused, not silently replaced");
        assert!(pairing_relay_url("http://old.example", None).is_err(), "a stored plain-http relay is not used for pairing");
    }

    /// 릴레이 경로마다 기다리는 시간은 표 하나에서 읽는다: 읽기와 쓰기가 다른 경로, 표에 없는 경로, 길드 id가 경로 낱말과 같은 경우
    #[test]
    fn each_relay_path_waits_as_the_table_says() {
        use reqwest::Method;
        let secs = |m: Method, path: &str| relay_wait(&m, path).as_secs();
        assert_eq!(secs(Method::GET, "/api/nodes/g1/chatbot/models"), 18);
        assert_eq!(secs(Method::GET, "/api/nodes/g1/chatbot/credit"), 12);
        assert_eq!(secs(Method::GET, "/api/nodes/g1/discord-members"), 15);
        assert_eq!(secs(Method::GET, "/api/nodes/g1/moderation"), 15);
        assert_eq!(secs(Method::POST, "/api/nodes/g1/moderation/channels/c1/clear"), 60);
        assert_eq!(secs(Method::PUT, "/api/nodes/g1/moderation/mutes/u1"), 60);
        for (m, path) in [(Method::GET, "/api/nodes/g1/channels"), (Method::PUT, "/api/nodes/g1/channels"), (Method::GET, "/api/nodes/moderation/members"), (Method::GET, "/api/hosts/h1"), (Method::DELETE, "/api/mcp/grant")] {
            assert_eq!(relay_wait(&m, path), RELAY_WAIT, "{path}");
        }
    }
}
