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

fn invalid_relay_url(problem: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": problem, "error_code": "invalid_relay_url" }))).into_response()
}

async fn signed_base(state: &IPCServer, requested: Option<&str>) -> String {
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    signed_relay_url(&configured, requested)
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

/// 노드 토큰 파일 (`<data_dir>/.node_token`)
pub(crate) fn node_token_path() -> std::path::PathBuf {
    saba_chan_updater_lib::constants::resolve_data_dir().join(".node_token")
}

/// Load node token from the data directory file (.node_token).
pub(crate) fn load_node_token() -> Option<String> {
    std::fs::read_to_string(node_token_path())
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// 노드 토큰을 쓴다 (원자적, 주인만 읽기). 같은 값이 이미 있으면 쓰지 않고 `Ok(false)`
pub(crate) fn write_node_token(path: &std::path::Path, token: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|t| t.trim() == token.trim()) {
        return Ok(false);
    }
    saba_atomic_file::write(path, token.as_bytes(), saba_atomic_file::Mode::Secret)?;
    Ok(true)
}

/// 토큰을 쓴 뒤 따라야 할 것: hostId를 토큰에 맞추고, 바뀌었으면 relay agent를 새 토큰으로 다시 띄운다.
/// 토큰을 쓰는 곳(페어링 완료, 토큰 저장, 가져오기)이 모두 부른다
pub(crate) async fn adopt_node_token(state: &IPCServer, token: &str, changed: bool) {
    sync_host_id(state, Some(token)).await;
    if changed {
        crate::relay_agent::restart(state).await;
    }
}

/// Generate HMAC-SHA256 signed headers for relay server authentication.
/// Returns None if no valid node token is available.
pub(crate) fn signed_headers(
    method: &str,
    url_path: &str,
    body: Option<&str>,
) -> Option<Vec<(String, String)>> {
    signed_headers_with(&load_node_token()?, method, url_path, body)
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
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!("/api/hosts/{}", encode_path_segment(&host_id));
    signed_get(&base, &path, |code| (code, Json(json!({ "ok": code.is_success() }))).into_response()).await
}

// ── GET /api/relay/host/:host_id/nodes ──

pub async fn list_host_nodes(
    Path(host_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!("/api/hosts/{}/nodes", encode_path_segment(&host_id));
    signed_get(&base, &path, empty_list).await
}

// ── GET/PUT /api/relay/node/:guild_id/channels ──
// 챗봇 채널(여럿)·음악 채널(하나) 배정. 디스코드 채널 상태라 원본은 릴레이다 (/mnt/project-files/assign/contract.md).
// 본문과 응답은 릴레이 것을 그대로 전달한다 (오류는 `{error: CODE, channelId?}`)

pub async fn get_node_channels(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, &format!("/api/nodes/{}/channels", encode_path_segment(&guild_id)), invalid_response).await
}

// ── GET /api/relay/node/:guild_id/chatbot/models ──
// 길드에 저장된 사용자 키로 relay가 받은 대화 모델 목록 (`{provider, models, selected, effectiveModel}`, 409 NO_API_KEY 등은 그대로)

pub async fn get_node_chatbot_models(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, &format!("/api/nodes/{}/chatbot/models", encode_path_segment(&guild_id)), invalid_response).await
}

pub async fn put_node_channels(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
    Json(body): Json<Value>,
) -> Response {
    if !body.is_object() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "INVALID_BODY" }))).into_response();
    }
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let path = format!("/api/nodes/{}/channels", encode_path_segment(&guild_id));
    let text = body.to_string();
    let req = apply_signed_headers(client.put(format!("{}{}", base, path)), "PUT", &path, Some(&text))
        .header("Content-Type", "application/json")
        .body(text);
    match send(req).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => invalid_response(code),
        Err(r) => r,
    }
}

// ── GET /api/relay/link-requests · POST /api/relay/link-requests/:id ──
// 디스코드에서 코드 없이 `/사바쨩 연결`한 요청 (J1-b). 요청은 릴레이가 들고 있고, PC는 보고 수락·거절만 한다.
// 수락하면 릴레이가 그 길드를 이 PC에 묶는다. 본문과 응답은 릴레이 것을 그대로 전달한다

pub async fn list_link_requests(Query(params): Query<RelayQueryParams>, State(state): State<IPCServer>) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, "/api/link-requests", empty_list).await
}

#[derive(Deserialize)]
pub struct LinkAnswer {
    accept: bool,
}

pub async fn answer_link_request(
    Path(request_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
    Json(answer): Json<LinkAnswer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!("/api/link-requests/{}", encode_path_segment(&request_id));
    signed_post(&base, &path, &json!({ "accept": answer.accept })).await
}

/// 서명한 JSON POST를 보내고 답을 전달한다
async fn signed_post(base: &str, api_path: &str, body: &Value) -> Response {
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let text = body.to_string();
    let req = apply_signed_headers(client.post(format!("{}{}", base, api_path)), "POST", api_path, Some(&text))
        .header("Content-Type", "application/json")
        .body(text);
    match send(req).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => invalid_response(code),
        Err(r) => r,
    }
}

// ── GET /api/relay/node/:guild_id/members ──
// Try discord-members first; fall back to permission-based members on 503

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MembersQuery {
    pub relay_url: Option<String>,
    /// `1`이면 `{members, partial, reason?}`로 답한다. 없으면 예전처럼 배열만
    #[serde(default)]
    pub with_status: Option<String>,
}

/// 길드 전체 멤버 요청: 릴레이는 디스코드에 8초까지 기다린다(그 뒤에는 캐시로 답한다)
const DISCORD_MEMBERS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub async fn list_node_members(
    Path(guild_id): Path<String>,
    Query(params): Query<MembersQuery>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let with_status = params.with_status.as_deref() == Some("1");
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let encoded = encode_path_segment(&guild_id);

    // Primary: real-time discord members
    let discord_path = format!("/api/nodes/{}/discord-members", encoded);
    let discord_url = format!("{}{}", base, discord_path);
    let reason = match send(apply_signed_headers(client.get(&discord_url).timeout(DISCORD_MEMBERS_TIMEOUT), "GET", &discord_path, None)).await {
        Ok((code, Some(data))) if code.is_success() => {
            let body = if with_status { json!({ "members": data, "partial": false }) } else { data };
            return (StatusCode::OK, Json(body)).into_response();
        }
        Ok((StatusCode::SERVICE_UNAVAILABLE, _)) => "bot_unavailable",
        Ok((code, _)) => {
            tracing::warn!("Discord members of guild {} failed ({}), falling back to permission-based members", guild_id, code);
            "relay_error"
        }
        Err(_) => "relay_unreachable",
    };

    // Fallback: permission-based members (권한을 등록한 사람만이라 전체 목록이 아니다)
    let fallback_path = format!("/api/nodes/{}/members", encoded);
    if !with_status {
        return signed_get(&base, &fallback_path, empty_list).await;
    }
    match send(apply_signed_headers(client.get(format!("{}{}", base, fallback_path)), "GET", &fallback_path, None)).await {
        Ok((code, Some(data))) if code.is_success() => Json(json!({ "members": data, "partial": true, "reason": reason })).into_response(),
        _ => (StatusCode::BAD_GATEWAY, Json(json!({ "members": [], "partial": true, "reason": reason }))).into_response(),
    }
}

// ── GET /api/relay/node/:guild_id/legacy-members ──
// 권한을 등록한 멤버 목록(`/api/nodes/:id/members`, `[{id, username, displayName, level}]`) 그대로. 실패하면 오류로 답한다(빈 목록을 돌려주지 않는다)

pub async fn list_node_legacy_members(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let path = format!("/api/nodes/{}/members", encode_path_segment(&guild_id));
    match send(apply_signed_headers(client.get(format!("{}{}", base, path)), "GET", &path, None)).await {
        Ok((code, Some(data))) if code.is_success() && data.is_array() => (StatusCode::OK, Json(data)).into_response(),
        Ok((code, Some(data))) if !code.is_success() => (code, Json(data)).into_response(),
        Ok(_) => invalid_response(StatusCode::BAD_GATEWAY),
        Err(r) => r,
    }
}

// ── GET /api/relay/bot-invite ──
// 봇 초대 URL (공개 엔드포인트라 서명하지 않는다)

pub async fn get_bot_invite(
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    bot_invite(&signed_base(&state, params.relay_url.as_deref()).await).await
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
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> Response {
    let relay_url = payload
        .get("relayUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(params.relay_url);
    // 페어링이 받은 노드 토큰은 그 릴레이 것이다: 설정한 릴레이(또는 모의 릴레이)와만, https로만 페어링한다
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    let base = match pairing_relay_url(&configured, relay_url.as_deref()) {
        Ok(base) => base,
        Err(problem) => return invalid_relay_url(problem),
    };
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
    State(state): State<IPCServer>,
) -> Response {
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    let base = match pairing_relay_url(&configured, params.relay_url.as_deref()) {
        Ok(base) => base,
        Err(problem) => return invalid_relay_url(problem),
    };
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };

    let url = format!("{}/api/pair/{}/status", base, encode_path_segment(&code));
    // relay는 poll secret을 헤더로 받는다 (쿼리에 두면 접근 로그에도 남는다)
    let mut request = client.get(&url);
    if let Some(ref secret) = params.secret {
        request = request.header("x-poll-secret", secret);
    }

    let (code_status, mut data) = match send(request).await {
        Ok((code, Some(data))) => (code, data),
        Ok((code, None)) => return invalid_response(code),
        Err(r) => return r,
    };
    // 페어링이 끝났으면 노드 토큰을 저장하고, 저장했는지를 `tokenSaved`로 알린다 (클라이언트는 이 값으로 "연결됨"을 보인다)
    if let Some("claimed") = data.get("status").and_then(|s| s.as_str()) {
        let token = data.get("nodeToken").and_then(|t| t.as_str()).map(str::to_string);
        let stored = store_node_token(&node_token_path(), token.as_deref());
        if let (true, Some(token)) = (stored.changed, token.as_deref()) {
            tracing::info!("Node token saved via pairing completion");
            adopt_node_token(&state, token, true).await;
        }
        if let Some(obj) = data.as_object_mut() {
            obj.insert("tokenSaved".into(), json!(stored.saved));
        }
    }
    (code_status, Json(data)).into_response()
}

/// 페어링으로 받은 노드 토큰을 저장한 결과
#[derive(Debug, PartialEq)]
struct StoredToken {
    /// 이 PC에 그 토큰이 저장돼 있다 (방금 썼거나 이미 같은 값)
    saved: bool,
    /// 파일을 새 값으로 바꿨다 (이때만 호스트 id를 맞추고 relay agent를 다시 띄운다)
    changed: bool,
}

/// 받은 토큰을 `.node_token`에 쓴다. 토큰이 없거나(수령 기한이 지남) 쓰지 못하면 저장하지 않은 것이고,
/// 같은 값이 이미 있으면 다시 쓰지 않는다 (폴링이 claimed를 여러 번 받는다)
fn store_node_token(path: &std::path::Path, token: Option<&str>) -> StoredToken {
    let Some(token) = token.filter(|t| !t.trim().is_empty()) else {
        tracing::warn!("Pairing was claimed but the relay no longer hands out the node token");
        return StoredToken { saved: false, changed: false };
    };
    match write_node_token(path, token) {
        Ok(changed) => StoredToken { saved: true, changed },
        Err(e) => {
            tracing::warn!("Failed to auto-save node token: {}", e);
            StoredToken { saved: false, changed: false }
        }
    }
}

#[cfg(test)]
mod tests {
    /// 토큰을 저장하지 못했으면 연결된 것으로 보이면 안 된다. 같은 토큰을 다시 받으면 다시 쓰지 않는다
    #[test]
    fn a_claimed_pairing_reports_whether_the_token_was_saved() {
        use super::{store_node_token, StoredToken};
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data").join(".node_token");
        assert_eq!(store_node_token(&path, None), StoredToken { saved: false, changed: false }, "claimed after the token expired");
        assert_eq!(store_node_token(&path, Some("sbn_a.x")), StoredToken { saved: true, changed: true });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "sbn_a.x");
        assert_eq!(store_node_token(&path, Some("sbn_a.x")), StoredToken { saved: true, changed: false }, "the next poll sees claimed again");
        // 쓸 수 없는 곳(폴더 자리에 파일)이면 저장하지 않은 것이다
        let blocked = tmp.path().join("file");
        std::fs::write(&blocked, "").unwrap();
        assert_eq!(store_node_token(&blocked.join(".node_token"), Some("sbn_b.y")), StoredToken { saved: false, changed: false });
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

    /// 페어링은 받은 노드 토큰이 평문으로 오가지 않게 https(또는 루프백 모의 릴레이)에서만 한다
    #[test]
    fn pairing_only_goes_to_an_https_or_loopback_relay() {
        assert_eq!(pairing_relay_url("", None).unwrap(), "https://saba-chan.online");
        assert_eq!(pairing_relay_url("https://my.relay", Some("http://127.0.0.1:9000")).unwrap(), "http://127.0.0.1:9000");
        assert!(pairing_relay_url("https://my.relay", Some("http://evil.example")).is_err(), "a plain-http request is refused, not silently replaced");
        assert!(pairing_relay_url("http://old.example", None).is_err(), "a stored plain-http relay is not used for pairing");
    }

    use super::*;
    use axum::{routing::{get, post}, Router};

    async fn mock_relay() -> String {
        let app = Router::new()
            .route("/api/hosts/h1", get(|| async { Json(json!({ "online": true })) }))
            .route("/api/hosts/h2", get(|| async { (StatusCode::NOT_FOUND, "nope") }))
            .route("/api/bot-invite", get(|| async { Json(json!({ "url": "https://discord.com/x" })) }))
            .route("/api/link-requests/:id", post(|Path(id): Path<String>, Json(b): Json<Value>| async move { Json(json!({ "id": id, "accept": b["accept"] })) }));
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
    async fn a_link_request_answer_goes_to_the_relay_as_is() {
        let base = mock_relay().await;
        let (code, data) = body(signed_post(&base, "/api/link-requests/r1", &json!({ "accept": true })).await).await;
        assert_eq!((code, data), (StatusCode::OK, json!({ "id": "r1", "accept": true })));
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_a_bad_gateway() {
        let (code, data) = body(signed_get("http://127.0.0.1:9", "/api/hosts/x", empty_list).await).await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert!(data["error"].as_str().unwrap().starts_with("Relay connection failed"));
    }
}
