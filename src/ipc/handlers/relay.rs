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

/// 릴레이 서버 기본 URL (SSOT)
pub const DEFAULT_RELAY_URL: &str = "https://saba-chan.online";

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
        .unwrap_or(DEFAULT_RELAY_URL);
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

/// bot-config → 릴레이 PATCH 페이로드(prefix/moduleAliases/commandAliases)를 만든다.
/// 네트워크 없는 순수 함수라 단위 테스트로 검증 가능하다.
fn build_bot_config_payload(config: &crate::config_store::BotConfig) -> Value {
    json!({
        "prefix": config.prefix,
        "moduleAliases": config.module_aliases,
        "commandAliases": config.command_aliases,
    })
}

/// module.toml의 기본 별명(`module_aliases`/`[aliases.commands]`)과 사용자가
/// GUI에서 저장한 별명(`BotConfig.module_aliases`/`command_aliases`)을 병합한다.
///
/// 사용자가 명시적으로 저장한 값이 있으면 그것을 우선하고, 없으면 module.toml
/// 기본값(CSV로 join)을 채운다. 이렇게 하지 않으면 "Discord 별명 설정 화면을 한
/// 번도 연 적 없는" 사용자는 module.toml에 정의된 기본 별명(예: 시작/실행/켜기)이
/// 릴레이 봇에 전혀 반영되지 않아 영어 명령(`start`) 외에는 전부 실패한다.
fn merge_aliases_with_defaults(
    supervisor: &crate::supervisor::Supervisor,
    config: &crate::config_store::BotConfig,
) -> (
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, Value>,
) {
    let modules = supervisor
        .module_loader
        .discover_modules()
        .unwrap_or_default();

    let mut module_aliases = config.module_aliases.clone();
    let mut command_aliases = config.command_aliases.clone();

    for m in &modules {
        let name = &m.metadata.name;

        // module_aliases: 사용자가 저장하지 않은 모듈만 기본값으로 채운다.
        module_aliases
            .entry(name.clone())
            .or_insert_with(|| m.metadata.module_aliases.join(","));

        // command_aliases: 커맨드 단위로 병합 (사용자 저장값 우선, 없으면 기본값).
        let user_cmds = command_aliases
            .get(name)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let mut merged_cmds = user_cmds.clone();
        for (cmd, info) in &m.metadata.command_aliases {
            if !merged_cmds.contains_key(cmd) {
                merged_cmds.insert(cmd.clone(), Value::String(info.aliases.join(",")));
            }
        }

        if !merged_cmds.is_empty() {
            command_aliases.insert(name.clone(), Value::Object(merged_cmds));
        }
    }

    (module_aliases, command_aliases)
}

/// 데몬 내부용: bot-config 저장 후 릴레이 서버로 prefix/별명을 전파한다.
///
/// 인터페이스(GUI/CLI)는 저장만 요청하고, 릴레이 전파는 데몬이 책임진다.
/// hostId는 노드 토큰에서 도출하며, 노드 토큰이 없으면(클라우드 미연결)
/// 아무 것도 하지 않는다. best-effort 이므로 실패는 stderr 로그만 남긴다.
pub async fn push_bot_config_to_relay(
    config: &crate::config_store::BotConfig,
    supervisor: &std::sync::Arc<tokio::sync::RwLock<crate::supervisor::Supervisor>>,
) {
    let token = match load_node_token() {
        Some(t) => t,
        None => return, // 클라우드 미연결 — 전파 대상 없음
    };
    let host_id = match parse_node_token(&token) {
        Some((id, _)) => id.to_string(),
        None => {
            eprintln!("[Relay] push_bot_config: invalid node token format");
            return;
        }
    };

    let base = resolve_relay_url(Some(config.cloud.relay_url.as_str()));
    let encoded = encode_path_segment(&host_id);
    let api_path = format!("/api/hosts/{}/bot-config", encoded);
    let url = format!("{}{}", base, api_path);

    // module.toml 기본 별명과 사용자 저장값을 병합 (lock은 await 이전에 해제).
    let (module_aliases, command_aliases) = {
        let guard = supervisor.read().await;
        merge_aliases_with_defaults(&guard, config)
    };
    let mut merged_config = config.clone();
    merged_config.module_aliases = module_aliases;
    merged_config.command_aliases = command_aliases;

    let payload = build_bot_config_payload(&merged_config);

    let client = match relay_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[Relay] push_bot_config: HTTP client error: {}", e);
            return;
        }
    };

    let body_str = serde_json::to_string(&payload).unwrap_or_default();
    let req = apply_signed_headers(
        client.patch(&url).json(&payload),
        "PATCH",
        &api_path,
        Some(&body_str),
    );

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                eprintln!("[Relay] push_bot_config failed: {}", status);
            }
        }
        Err(e) => {
            eprintln!("[Relay] push_bot_config connection failed: {}", e);
        }
    }
}// ── GET /api/relay/host/:host_id/status ──

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

// ── GET /api/relay/node/:guild_id/settings ──
// GUI → daemon proxy → relay server: 노드별 음악 설정 조회

pub async fn get_node_settings(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let encoded = encode_path_segment(&guild_id);
    let api_path = format!("/api/nodes/{}/settings", encoded);
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

// ── PATCH /api/relay/node/:guild_id/settings ──
// GUI → daemon proxy → relay server: 노드별 음악 설정 업데이트

pub async fn update_node_settings(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let encoded = encode_path_segment(&guild_id);
    let api_path = format!("/api/nodes/{}/settings", encoded);
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

    let body_str = serde_json::to_string(&payload).unwrap_or_default();
    let req = apply_signed_headers(
        client.patch(&url).json(&payload),
        "PATCH",
        &api_path,
        Some(&body_str),
    );
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => (code, Json(data)).into_response(),
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
    State(state): State<IPCServer>,
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

    let url = format!("{}/api/pair/{}/status", base, encode_path_segment(&code));
    let mut req = client.get(&url);
    if let Some(ref secret) = params.secret {
        req = req.header("X-Poll-Secret", secret.as_str());
    }

    match req.send().await {
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
                                // 페어링 직후 데몬 재시작 없이 하트비트/폴링 시작
                                let started = state.cloud_agent.restart_from_file().await;
                                tracing::info!(
                                    "[Relay] 페어링 완료 — cloud agent 핫 리로드 (running={})",
                                    started
                                );
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

// ── PATCH /api/relay/host/:host_id/bot-config ──
// GUI → daemon proxy → relay server: 봇 설정 동기화

pub async fn sync_bot_config(
    Path(host_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let encoded = encode_path_segment(&host_id);
    let api_path = format!("/api/hosts/{}/bot-config", encoded);
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

    let body_str = serde_json::to_string(&payload).unwrap_or_default();
    let req = apply_signed_headers(
        client.patch(&url).json(&payload),
        "PATCH",
        &api_path,
        Some(&body_str),
    );
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => (code, Json(data)).into_response(),
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

// ── GET /api/relay/bot-invite ──
// GUI → daemon proxy → relay server: 봇 초대 URL 조회

pub async fn get_bot_invite(
    Query(params): Query<RelayQueryParams>,
    State(_state): State<IPCServer>,
) -> impl IntoResponse {
    let base = resolve_relay_url(params.relay_url.as_deref());
    let url = format!("{}/api/bot-invite", base);

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

    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<Value>().await {
                Ok(data) => (code, Json(data)).into_response(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_store::BotConfig;
    use std::collections::HashMap;

    #[test]
    fn build_payload_contains_prefix_and_aliases_only() {
        let mut config = BotConfig {
            prefix: "!mybot".to_string(),
            module_aliases: HashMap::from([("mc".to_string(), "minecraft".to_string())]),
            command_aliases: HashMap::from([("mc".to_string(), json!({ "start": "go" }))]),
            node_settings: json!({ "local": { "ids": ["a"] } }),
            ..Default::default()
        };
        // cloud/nodeSettings 등은 릴레이로 전파되지 않아야 한다.
        config.cloud.host_id = "host-xyz".to_string();

        let payload = build_bot_config_payload(&config);

        assert_eq!(payload["prefix"], "!mybot");
        assert_eq!(payload["moduleAliases"]["mc"], "minecraft");
        assert_eq!(payload["commandAliases"]["mc"]["start"], "go");
        // 릴레이 bot-config 스키마 외 필드는 포함되지 않는다.
        assert!(payload.get("cloud").is_none());
        assert!(payload.get("nodeSettings").is_none());
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.len(), 3);
    }

    #[test]
    fn parse_node_token_extracts_host_id() {
        let parsed = parse_node_token("sbn_hostABC.secretXYZ");
        assert_eq!(parsed, Some(("hostABC", "secretXYZ")));
        assert!(parse_node_token("invalid").is_none());
        assert!(parse_node_token("sbn_nodot").is_none());
    }

    /// 임시 모듈 디렉토리에 module.toml 하나를 만들고 그 위에 Supervisor를 올린다.
    fn supervisor_with_fixture_module(module_toml: &str) -> (tempfile::TempDir, crate::supervisor::Supervisor) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mod_dir = tmp.path().join("palworld");
        std::fs::create_dir_all(&mod_dir).expect("mkdir");
        std::fs::write(mod_dir.join("module.toml"), module_toml).expect("write module.toml");
        let supervisor = crate::supervisor::Supervisor::new(tmp.path().to_str().unwrap());
        (tmp, supervisor)
    }

    const FIXTURE_MODULE_TOML: &str = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[aliases]
module_aliases = ["팰월드", "pw"]

[aliases.commands]
start = { aliases = ["시작", "실행", "켜기"], description = "Start the server" }
stop = { aliases = ["정지", "끄기"], description = "Stop the server" }
"#;

    #[test]
    fn merge_aliases_fills_defaults_when_user_never_saved_anything() {
        // 사용자가 Discord 별명 설정 화면을 한 번도 저장하지 않은 상태(빈 BotConfig)에서도
        // module.toml의 기본 별명(팰월드/시작/실행/켜기 등)이 릴레이로 나가야 한다.
        // 이 테스트가 실패하면 "실행" 같은 기본 별명이 Discord에서 전혀 동작하지 않는다.
        let (_tmp, supervisor) = supervisor_with_fixture_module(FIXTURE_MODULE_TOML);
        let config = BotConfig::default();

        let (module_aliases, command_aliases) = merge_aliases_with_defaults(&supervisor, &config);

        assert_eq!(module_aliases.get("palworld").map(|s| s.as_str()), Some("팰월드,pw"));
        let start = command_aliases["palworld"]["start"].as_str().unwrap();
        assert_eq!(start, "시작,실행,켜기");
        let stop = command_aliases["palworld"]["stop"].as_str().unwrap();
        assert_eq!(stop, "정지,끄기");
    }

    #[test]
    fn merge_aliases_prefers_user_saved_value_over_default() {
        // 사용자가 GUI에서 특정 명령의 별명을 커스텀으로 저장했다면 그 값이 우선해야 한다.
        let (_tmp, supervisor) = supervisor_with_fixture_module(FIXTURE_MODULE_TOML);
        let config = BotConfig {
            module_aliases: HashMap::from([("palworld".to_string(), "팜서버".to_string())]),
            command_aliases: HashMap::from([("palworld".to_string(), json!({ "start": "go" }))]),
            ..Default::default()
        };

        let (module_aliases, command_aliases) = merge_aliases_with_defaults(&supervisor, &config);

        // module_aliases: 사용자가 저장한 값이 그대로 유지된다.
        assert_eq!(module_aliases.get("palworld").map(|s| s.as_str()), Some("팜서버"));
        // command_aliases: "start"는 사용자 값 유지, "stop"은 저장 안 했으므로 기본값이 채워진다.
        assert_eq!(command_aliases["palworld"]["start"].as_str().unwrap(), "go");
        assert_eq!(command_aliases["palworld"]["stop"].as_str().unwrap(), "정지,끄기");
    }
}
