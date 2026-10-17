//! Relay server proxy handlers
//!
//! GUI/CLI must not access the relay server (saba-chan.online) directly.
//! The daemon acts as a proxy for all relay communication.
//!
//! 인증이 필요한 엔드포인트는 노드 토큰으로 HMAC 서명하여 전송합니다.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::IPCServer;

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

/// Create an HTTP client with 5s timeout for relay requests.
/// Automatically includes X-Saba-Client: 1 header required by nginx.
fn relay_client() -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "X-Saba-Client",
        reqwest::header::HeaderValue::from_static("1"),
    );
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .default_headers(headers)
        .build()
}

/// Parse a node token (sbn_{nodeId}.{secret}) into (nodeId, secret).
pub(crate) fn parse_node_token(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix("sbn_")?;
    let dot = rest.find('.')?;
    Some((&rest[..dot], &rest[dot + 1..]))
}

/// Load node token from the data directory file (.node_token).
pub(crate) fn load_node_token() -> Option<String> {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let token_path = data_dir.join(".node_token");
    std::fs::read_to_string(&token_path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Generate HMAC-SHA256 signed headers for relay server authentication.
/// Returns None if no valid node token is available.
pub(crate) fn signed_headers(
    method: &str,
    url_path: &str,
    body: Option<&str>,
) -> Option<Vec<(String, String)>> {
    let token = load_node_token()?;
    let (_, secret) = parse_node_token(&token)?;

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
    match signed_headers(method, url_path, body) {
        Some(headers) => {
            let mut b = builder;
            for (k, v) in headers {
                b = b.header(&k, &v);
            }
            b
        }
        None => builder,
    }
}

#[derive(Deserialize)]
pub struct RelayQueryParams {
    #[serde(default, rename = "relayUrl")]
    relay_url: Option<String>,
}

// ── 공통: 릴레이로 보내고 답을 그대로 돌려준다 ──

fn error(code: StatusCode, message: String) -> Response {
    (code, Json(json!({ "error": message }))).into_response()
}

#[allow(clippy::result_large_err)] // 오류 응답은 드문 경로다
pub(crate) fn client() -> Result<reqwest::Client, Response> {
    relay_client().map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, format!("HTTP client error: {}", e)))
}

/// 요청을 보내고 (릴레이의 상태 코드, JSON 본문)을 돌려준다. 본문이 JSON이 아니면 None
#[allow(clippy::result_large_err)]
pub(crate) async fn send(req: reqwest::RequestBuilder) -> Result<(StatusCode, Option<Value>), Response> {
    let resp = req
        .send()
        .await
        .map_err(|e| error(StatusCode::BAD_GATEWAY, format!("Relay connection failed: {}", e)))?;
    let code = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    Ok((code, resp.json::<Value>().await.ok()))
}

/// 서명한 GET을 보내고 답을 전달한다. 본문이 JSON이 아니면 `fallback(상태)`
async fn signed_get(base: &str, api_path: &str, fallback: impl FnOnce(StatusCode) -> Response) -> Response {
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let url = format!("{}{}", base, api_path);
    match send(apply_signed_headers(client.get(&url), "GET", api_path, None)).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => fallback(code),
        Err(r) => r,
    }
}

fn empty_list(_: StatusCode) -> Response {
    (StatusCode::BAD_GATEWAY, Json(json!([]))).into_response()
}

fn invalid_response(_: StatusCode) -> Response {
    error(StatusCode::BAD_GATEWAY, "Invalid relay response".into())
}

// ── GET /api/relay/host/:host_id/status ──

pub async fn check_host_status(
    Path(host_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> Response {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let path = format!("/api/hosts/{}", encode_path_segment(&host_id));
    signed_get(&base, &path, |code| (code, Json(json!({ "ok": code.is_success() }))).into_response()).await
}

// ── GET /api/relay/host/:host_id/nodes ──

pub async fn list_host_nodes(
    Path(host_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> Response {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let path = format!("/api/hosts/{}/nodes", encode_path_segment(&host_id));
    signed_get(&base, &path, empty_list).await
}

// ── GET /api/relay/node/:guild_id/members ──
// Try discord-members first; fall back to permission-based members on 503

pub async fn list_node_members(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> Response {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let encoded = encode_path_segment(&guild_id);

    // Primary: real-time discord members
    let discord_path = format!("/api/nodes/{}/discord-members", encoded);
    let discord_url = format!("{}{}", base, discord_path);
    match send(apply_signed_headers(client.get(&discord_url), "GET", &discord_path, None)).await {
        Ok((code, Some(data))) if code.is_success() => return (StatusCode::OK, Json(data)).into_response(),
        Ok((StatusCode::SERVICE_UNAVAILABLE, _)) => tracing::debug!(
            "Discord bot unavailable for guild {}, falling back to permission-based members",
            guild_id
        ),
        _ => {}
    }

    // Fallback: permission-based members
    signed_get(&base, &format!("/api/nodes/{}/members", encoded), empty_list).await
}

// ── GET /api/relay/bot-invite ──
// 봇 초대 URL (공개 엔드포인트라 서명하지 않는다)

pub async fn get_bot_invite(
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> Response {
    bot_invite(&resolve_relay_url(params.relay_url.as_deref())).await
}

async fn bot_invite(base: &str) -> Response {
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    match send(client.get(format!("{}/api/bot-invite", base))).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => invalid_response(code),
        Err(r) => r,
    }
}

// ── POST /api/relay/pair/initiate ──

pub async fn initiate_pairing(
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> Response {
    let relay_url = payload
        .get("relayUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(params.relay_url);
    let base = resolve_relay_url(relay_url.as_deref());
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let url = format!("{}/api/pair/initiate", base);
    match send(client.post(&url).json(&payload)).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => invalid_response(code),
        Err(r) => r,
    }
}

// ── GET /api/relay/pair/:code/status ──

#[derive(Deserialize)]
pub struct PairStatusQuery {
    #[serde(default)]
    secret: Option<String>,
    #[serde(default, rename = "relayUrl")]
    relay_url: Option<String>,
}

pub async fn poll_pairing_status(
    Path(code): Path<String>,
    Query(params): Query<PairStatusQuery>,
    State(_state): State<IPCServer>,
) -> Response {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };

    let mut url = format!("{}/api/pair/{}/status", base, encode_path_segment(&code));
    if let Some(ref secret) = params.secret {
        url = format!("{}?secret={}", url, encode_path_segment(secret));
    }

    let (code_status, data) = match send(client.get(&url)).await {
        Ok((code, Some(data))) => (code, data),
        Ok((code, None)) => return invalid_response(code),
        Err(r) => return r,
    };
    // Auto-save node token on successful pairing
    if let Some("claimed") = data.get("status").and_then(|s| s.as_str()) {
        if let Some(token) = data.get("nodeToken").and_then(|t| t.as_str()) {
            let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
            let token_path = data_dir.join(".node_token");
            if let Some(parent) = token_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&token_path, token) {
                tracing::warn!("Failed to auto-save node token: {}", e);
            } else {
                tracing::info!("Node token saved via pairing completion");
            }
        }
    }
    (code_status, Json(data)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};

    async fn mock_relay() -> String {
        let app = Router::new()
            .route("/api/hosts/h1", get(|| async { Json(json!({ "online": true })) }))
            .route("/api/hosts/h2", get(|| async { (StatusCode::NOT_FOUND, "nope") }))
            .route("/api/bot-invite", get(|| async { Json(json!({ "url": "https://discord.com/x" })) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{}", addr)
    }

    async fn body(resp: Response) -> (StatusCode, Value) {
        let code = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (code, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn proxied_json_passes_through_and_non_json_uses_the_fallback() {
        let base = mock_relay().await;
        let fallback = |code: StatusCode| (code, Json(json!({ "ok": code.is_success() }))).into_response();
        assert_eq!(body(signed_get(&base, "/api/hosts/h1", fallback).await).await, (StatusCode::OK, json!({ "online": true })));
        assert_eq!(body(signed_get(&base, "/api/hosts/h2", fallback).await).await, (StatusCode::NOT_FOUND, json!({ "ok": false })));
        assert_eq!(body(bot_invite(&base).await).await.1["url"], "https://discord.com/x");
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_a_bad_gateway() {
        let (code, data) = body(signed_get("http://127.0.0.1:9", "/api/hosts/x", empty_list).await).await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert!(data["error"].as_str().unwrap().starts_with("Relay connection failed"));
    }
}
