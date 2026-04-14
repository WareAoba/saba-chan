//! Relay server proxy handlers
//!
//! GUI/CLI must not access the relay server (saba-chan.online) directly.
//! The daemon acts as a proxy for all relay communication.
//!
//! 인증이 필요한 엔드포인트는 노드 토큰으로 HMAC 서명하여 전송합니다.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::IPCServer;

type HmacSha256 = Hmac<Sha256>;

/// Percent-encode a URL path segment (RFC 3986 unreserved chars only)
fn encode_path_segment(s: &str) -> String {
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
fn resolve_relay_url(relay_url: Option<&str>) -> String {
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
fn parse_node_token(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix("sbn_")?;
    let dot = rest.find('.')?;
    Some((&rest[..dot], &rest[dot + 1..]))
}

/// Load node token from the data directory file (.node_token).
fn load_node_token() -> Option<String> {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let token_path = data_dir.join(".node_token");
    std::fs::read_to_string(&token_path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Generate HMAC-SHA256 signed headers for relay server authentication.
/// Returns None if no valid node token is available.
fn signed_headers(
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
fn apply_signed_headers(
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

// ── GET /api/relay/host/:host_id/status ──

pub async fn check_host_status(
    Path(host_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let encoded = encode_path_segment(&host_id);
    let api_path = format!("/api/hosts/{}", encoded);
    let url = format!("{}{}", base, api_path);

    let client = match relay_client() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("HTTP client error: {}", e) })),
            )
                .into_response();
        }
    };

    let req = apply_signed_headers(client.get(&url), "GET", &api_path, None);
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.text().await {
                Ok(body) => {
                    let parsed: Value =
                        serde_json::from_str(&body).unwrap_or(json!({ "ok": status.is_success() }));
                    (code, Json(parsed)).into_response()
                }
                Err(_) => (code, Json(json!({ "ok": status.is_success() }))).into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Relay connection failed: {}", e) })),
        )
            .into_response(),
    }
}

// ── GET /api/relay/host/:host_id/nodes ──

pub async fn list_host_nodes(
    Path(host_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let encoded = encode_path_segment(&host_id);
    let api_path = format!("/api/hosts/{}/nodes", encoded);
    let url = format!("{}{}", base, api_path);

    let client = match relay_client() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("HTTP client error: {}", e) })),
            )
                .into_response();
        }
    };

    let req = apply_signed_headers(client.get(&url), "GET", &api_path, None);
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => (code, Json(data)).into_response(),
                Err(_) => (StatusCode::BAD_GATEWAY, Json(json!([]))).into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Relay connection failed: {}", e) })),
        )
            .into_response(),
    }
}

// ── GET /api/relay/node/:guild_id/members ──
// Try discord-members first; fall back to permission-based members on 503

pub async fn list_node_members(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let client = match relay_client() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("HTTP client error: {}", e) })),
            )
                .into_response();
        }
    };

    let encoded = encode_path_segment(&guild_id);

    // Primary: real-time discord members
    let discord_path = format!("/api/nodes/{}/discord-members", encoded);
    let discord_url = format!("{}{}", base, discord_path);
    let req = apply_signed_headers(client.get(&discord_url), "GET", &discord_path, None);
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(data) = resp.json::<Value>().await {
                return (StatusCode::OK, Json(data)).into_response();
            }
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            tracing::debug!(
                "Discord bot unavailable for guild {}, falling back to permission-based members",
                guild_id
            );
        }
        _ => {}
    }

    // Fallback: permission-based members
    let fallback_path = format!("/api/nodes/{}/members", encoded);
    let fallback_url = format!("{}{}", base, fallback_path);
    let req = apply_signed_headers(client.get(&fallback_url), "GET", &fallback_path, None);
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => (code, Json(data)).into_response(),
                Err(_) => (StatusCode::BAD_GATEWAY, Json(json!([]))).into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Relay connection failed: {}", e) })),
        )
            .into_response(),
    }
}

// ── POST /api/relay/pair/initiate ──

pub async fn initiate_pairing(
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let relay_url = payload
        .get("relayUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(params.relay_url);
    let base = resolve_relay_url(relay_url.as_deref());

    let client = match relay_client() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("HTTP client error: {}", e) })),
            )
                .into_response();
        }
    };

    let url = format!("{}/api/pair/initiate", base);
    match client.post(&url).json(&payload).send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => (code, Json(data)).into_response(),
                Err(_) => {
                    (StatusCode::BAD_GATEWAY, Json(json!({ "error": "Invalid relay response" })))
                        .into_response()
                }
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Relay connection failed: {}", e) })),
        )
            .into_response(),
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
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let client = match relay_client() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("HTTP client error: {}", e) })),
            )
                .into_response();
        }
    };

    let mut url = format!("{}/api/pair/{}/status", base, encode_path_segment(&code));
    if let Some(ref secret) = params.secret {
        url = format!("{}?secret={}", url, encode_path_segment(secret));
    }

    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let code_status =
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => {
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
                Err(_) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": "Invalid relay response" })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Relay connection failed: {}", e) })),
        )
            .into_response(),
    }
}
