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
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::IPCServer;
use crate::relay_client::{
    adopt_node_token, apply_signed_headers, encode_path_segment, load_node_token, pairing_relay_url, parse_node_token, relay_client, relay_wait,
    resolve_relay_url, signed_relay_url, store_node_token,
};
use saba_chan_updater_lib::config_file::ConfigFile;

fn invalid_relay_url(problem: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": problem, "error_code": "invalid_relay_url" }))).into_response()
}

async fn signed_base(state: &IPCServer, requested: Option<&str>) -> String {
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    signed_relay_url(&configured, requested)
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
    crate::relay_client::send(req).await.map_err(|e| error(StatusCode::BAD_GATEWAY, format!("Relay connection failed: {}", e)))
}

/// 서명한 GET을 보내고 답을 전달한다. 본문이 JSON이 아니면 `fallback(상태)`
async fn signed_get(base: &str, api_path: &str, fallback: impl FnOnce(StatusCode) -> Response) -> Response {
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let req = client.get(format!("{}{}", base, api_path)).timeout(relay_wait(&reqwest::Method::GET, api_path));
    match send(apply_signed_headers(req, "GET", api_path, None)).await {
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
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    match send(apply_signed_headers(client.get(format!("{}{}", base, path)), "GET", &path, None)).await {
        Ok((code, Some(data))) => {
            if code.is_success() {
                pin_listed_owners(&state.config_store, &data).await;
            }
            (code, Json(data)).into_response()
        }
        Ok((code, None)) => empty_list(code),
        Err(r) => r,
    }
}

/// 노드 목록이 알려 준 주인(디스코드에서 길드를 연결한 사람)을 아직 주인이 없는 길드에 고정한다.
/// 명령이 처음 올 때와 같은 `pin_owner` 규칙이라, 고정된 주인은 바꾸지 않고 다른 값은 `relayOwnerDiscordId`에 적는다
async fn pin_listed_owners(store: &crate::config_store::ConfigStore, nodes: &Value) {
    for n in nodes.as_array().into_iter().flatten() {
        let guild = n.get("guildId").and_then(Value::as_str).filter(|g| !g.is_empty());
        let owner = n.get("ownerDiscordId").and_then(Value::as_str).filter(|o| !o.trim().is_empty());
        let (Some(guild), Some(owner)) = (guild, owner) else { continue };
        let pinned = store
            .update_node_settings(guild, |node| {
                let (_, changed, _) = crate::chat::permissions::pin_owner(node, Some(owner));
                ((), changed)
            })
            .await;
        if let Err(e) = pinned {
            tracing::warn!("[Relay] could not pin the owner of guild {}: {}", guild, e);
        }
    }
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

// ── /api/relay/node/:guild_id/moderation… ──
// 앱의 서버 관리 화면: 채널 상태(주제·슬로우·잠금·자동 청소)와 음소거 목록, 채널 정보 수정, 청소, 음소거.
// 디스코드 서버 상태라 원본은 릴레이다. 본문과 응답은 릴레이 것을 그대로 전달한다 (오류는 `{error: CODE, …}`)

pub async fn get_node_moderation(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, &format!("/api/nodes/{}/moderation", encode_path_segment(&guild_id)), invalid_response).await
}

pub async fn patch_node_moderation_channel(
    Path((guild_id, channel_id)): Path<(String, String)>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
    Json(body): Json<Value>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!(
        "/api/nodes/{}/moderation/channels/{}",
        encode_path_segment(&guild_id),
        encode_path_segment(&channel_id)
    );
    signed_json(&base, reqwest::Method::PATCH, &path, &body).await
}

pub async fn clear_node_moderation_channel(
    Path((guild_id, channel_id)): Path<(String, String)>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
    Json(body): Json<Value>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!(
        "/api/nodes/{}/moderation/channels/{}/clear",
        encode_path_segment(&guild_id),
        encode_path_segment(&channel_id)
    );
    signed_json(&base, reqwest::Method::POST, &path, &body).await
}

// 확인 창 전의 검사 — 청소와 같은 권한을 보고 지우지 않는다
pub async fn check_clear_node_moderation_channel(
    Path((guild_id, channel_id)): Path<(String, String)>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
    Json(body): Json<Value>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!(
        "/api/nodes/{}/moderation/channels/{}/clear/check",
        encode_path_segment(&guild_id),
        encode_path_segment(&channel_id)
    );
    signed_json(&base, reqwest::Method::POST, &path, &body).await
}

pub async fn put_node_moderation_mute(
    Path((guild_id, user_id)): Path<(String, String)>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
    Json(body): Json<Value>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    let path = format!(
        "/api/nodes/{}/moderation/mutes/{}",
        encode_path_segment(&guild_id),
        encode_path_segment(&user_id)
    );
    signed_json(&base, reqwest::Method::PUT, &path, &body).await
}

/// 서명한 JSON 본문 요청을 보내고 답을 전달한다 (본문은 객체만)
async fn signed_json(base: &str, method: reqwest::Method, api_path: &str, body: &Value) -> Response {
    if !body.is_object() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "INVALID_BODY" }))).into_response();
    }
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    let text = body.to_string();
    let req = client.request(method.clone(), format!("{}{}", base, api_path)).timeout(relay_wait(&method, api_path));
    let req = apply_signed_headers(req, method.as_str(), api_path, Some(&text))
        .header("Content-Type", "application/json")
        .body(text);
    match send(req).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => invalid_response(code),
        Err(r) => r,
    }
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

// ── GET /api/relay/node/:guild_id/chatbot/tools ──
// 말하는 사바쨩이 고를 수 있는 도구 목록 (`{categories, tools: [{name, category, available, unavailable?}]}`). 도구 목록의 원본은 릴레이다

pub async fn get_node_chatbot_tools(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, &format!("/api/nodes/{}/chatbot/tools", encode_path_segment(&guild_id)), invalid_response).await
}

// ── GET /api/relay/node/:guild_id/chatbot/credit ──
// 남은 크레딧(제공사가 일반 키로 알려 줄 때만)과 이번 달 사용량 (`{provider, balanceSupported, balance, usage, exhaustedAt}`, 409 NO_API_KEY는 그대로)

pub async fn get_node_chatbot_credit(
    Path(guild_id): Path<String>,
    Query(params): Query<RelayQueryParams>,
    State(state): State<IPCServer>,
) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, &format!("/api/nodes/{}/chatbot/credit", encode_path_segment(&guild_id)), invalid_response).await
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

// ── GET·DELETE /api/relay/mcp/grant ──
// 이 PC 주인 계정의 원격 MCP 에이전트 연결들 (05 §4.4 (l)). 보기는 `{grants:[{id, clientName, createdAt, lastUsedAt}],
// grant, url}`(grant는 첫째), 끊기는 204. `?id=`면 그 연결만, 없으면 모두 끊는다.
// 연결은 주인 계정에 묶여 주인의 다른 PC에서 끊어도 같다

pub async fn get_mcp_grant(Query(params): Query<RelayQueryParams>, State(state): State<IPCServer>) -> Response {
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_get(&base, "/api/mcp/grant", invalid_response).await
}

#[derive(Deserialize)]
pub struct RevokeGrantParams {
    #[serde(default, rename = "relayUrl")]
    relay_url: Option<String>,
    /// 끊을 연결 (GET이 준 `id`). 없으면 모든 연결
    #[serde(default)]
    id: Option<String>,
}

/// 릴레이가 준 연결 id만 그대로 넘긴다 (16진수 글자). 다른 글자는 주소를 바꿀 수 있어 받지 않는다
fn grant_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub async fn revoke_mcp_grant(Query(params): Query<RevokeGrantParams>, State(state): State<IPCServer>) -> Response {
    let path = match params.id.as_deref() {
        None => "/api/mcp/grant".to_string(),
        Some(id) if grant_id_ok(id) => format!("/api/mcp/grant?id={id}"),
        Some(_) => return error(StatusCode::BAD_REQUEST, "Invalid grant id".into()),
    };
    let base = signed_base(&state, params.relay_url.as_deref()).await;
    signed_delete(&base, &path).await
}

/// 서명한 DELETE를 보낸다. 성공은 204, 실패는 릴레이 답 그대로
async fn signed_delete(base: &str, api_path: &str) -> Response {
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };
    match send(apply_signed_headers(client.delete(format!("{}{}", base, api_path)), "DELETE", api_path, None)).await {
        Ok((code, _)) if code.is_success() => StatusCode::NO_CONTENT.into_response(),
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
    let reason = match send(apply_signed_headers(client.get(&discord_url).timeout(relay_wait(&reqwest::Method::GET, &discord_path)), "GET", &discord_path, None)).await {
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
    let body = pairing_body(&payload, &base, &configured, load_node_token().as_deref(), device_name().as_deref());
    match send(client.post(&url).json(&body)).await {
        Ok((code, Some(data))) => (code, Json(data)).into_response(),
        Ok((code, None)) => invalid_response(code),
        Err(r) => r,
    }
}

/// 릴레이로 보내는 페어링 요청. GUI 가 보낸 것 중 `relayUrl` 만 옮기고, 이 PC 의 이름과
/// (토큰을 준 릴레이일 때만) 지금 토큰을 싣는다 — 릴레이는 그 토큰의 PC 를 다시 연결한다
fn pairing_body(payload: &Value, base: &str, configured: &str, token: Option<&str>, device: Option<&str>) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(url) = payload.get("relayUrl").and_then(Value::as_str) {
        body.insert("relayUrl".into(), json!(url));
    }
    if let Some(token) = token.map(str::trim).filter(|t| parse_node_token(t).is_some()) {
        if base == resolve_relay_url(Some(configured.trim())) {
            body.insert("currentToken".into(), json!(token));
        }
    }
    if let Some(name) = device.map(str::trim).filter(|n| !n.is_empty()) {
        body.insert("deviceName".into(), json!(name.chars().take(64).collect::<String>()));
    }
    Value::Object(body)
}

/// 새 PC 의 이름 (릴레이에서 새 PC 의 이름): OS 이름 (사용자 2026-11-12). 내 PC 카드·디스코드에서 바꾼다
fn device_name() -> Option<String> {
    Some(os_name(std::env::consts::OS, crate::relay_agent::platform()["distro"]["name"].as_str()))
}

/// OS 이름: Windows·macOS, 리눅스는 배포판 이름(모르면 Linux)
fn os_name(os: &str, distro: Option<&str>) -> String {
    match os {
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        "linux" => distro.map(str::trim).filter(|d| !d.is_empty()).unwrap_or("Linux").into(),
        other => other.into(),
    }
}

// ── POST /api/relay/pair/status ──

/// 페어링 코드와 poll secret은 본문으로 받는다 (경로·쿼리는 접근 로그와 프록시 기록에 남는다)
#[derive(Deserialize)]
pub struct PairStatusBody {
    code: String,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default, rename = "relayUrl")]
    relay_url: Option<String>,
}

pub async fn pairing_status(State(state): State<IPCServer>, Json(body): Json<PairStatusBody>) -> Response {
    pairing_status_of(&state, &body.code, body.secret.as_deref(), body.relay_url.as_deref()).await
}

async fn pairing_status_of(state: &IPCServer, code: &str, secret: Option<&str>, relay_url: Option<&str>) -> Response {
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    let base = match pairing_relay_url(&configured, relay_url) {
        Ok(base) => base,
        Err(problem) => return invalid_relay_url(problem),
    };
    let client = match client() {
        Ok(c) => c,
        Err(r) => return r,
    };

    let url = format!("{}/api/pair/{}/status", base, encode_path_segment(code));
    // relay는 poll secret을 헤더로 받는다 (쿼리에 두면 접근 로그에도 남는다)
    let mut request = client.get(&url);
    if let Some(secret) = secret {
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
        let stored = store_node_token(&ConfigFile::default_location(), token.as_deref());
        if let (true, Some(token)) = (stored.changed, token.as_deref()) {
            tracing::info!("Node token saved via pairing completion");
            adopt_node_token(state, token, true).await;
        }
        if let Some(obj) = data.as_object_mut() {
            obj.insert("tokenSaved".into(), json!(stored.saved));
        }
    }
    (code_status, Json(data)).into_response()
}

#[cfg(test)]
mod tests {
    use super::pairing_body;
    use serde_json::json;

    /// 페어링 요청은 정한 칸만 보낸다. 지금 토큰은 그 토큰을 준 릴레이(설정 주소)에만 싣는다:
    /// 릴레이는 그 토큰의 PC 를 다시 연결하고(계정 하나에 PC 여러 대), PC 이름은 새 PC 의 이름이 된다
    #[test]
    fn a_pairing_request_names_this_pc_and_only_proves_it_to_the_relay_that_issued_the_token() {
        let gui = json!({ "relayUrl": "https://saba-chan.online", "currentToken": "sbn_forged.x", "extra": 1 });
        let body = pairing_body(&gui, "https://saba-chan.online", "https://saba-chan.online/", Some("sbn_me.s"), Some("gcp-1"));
        assert_eq!(body, json!({ "relayUrl": "https://saba-chan.online", "currentToken": "sbn_me.s", "deviceName": "gcp-1" }));
        let mock = pairing_body(&json!({}), "http://127.0.0.1:9000", "https://saba-chan.online", Some("sbn_me.s"), None);
        assert_eq!(mock, json!({}), "a mock relay never gets the real token");
        let fresh = pairing_body(&json!({}), "https://saba-chan.online", "", None, Some(&"x".repeat(80)));
        assert_eq!(fresh["deviceName"].as_str().unwrap().chars().count(), 64);
        assert!(fresh.get("currentToken").is_none());
    }

    #[test]
    fn a_new_pc_is_named_after_its_os() {
        assert_eq!(os_name("windows", None), "Windows");
        assert_eq!(os_name("macos", None), "macOS");
        assert_eq!(os_name("linux", Some("Ubuntu 24.04 LTS")), "Ubuntu 24.04 LTS");
        assert_eq!(os_name("linux", None), "Linux");
    }

    use super::*;
    use axum::{routing::get, Router};

    async fn mock_relay() -> String {
        let app = Router::new()
            .route("/api/hosts/h1", get(|| async { Json(json!({ "online": true })) }))
            .route("/api/hosts/h2", get(|| async { (StatusCode::NOT_FOUND, "nope") }))
            .route("/api/bot-invite", get(|| async { Json(json!({ "url": "https://discord.com/x" })) }))
            .route("/api/mcp/grant", axum::routing::delete(|| async { StatusCode::NO_CONTENT }))
            .route("/api/mcp/gone", axum::routing::delete(|| async { (StatusCode::NOT_FOUND, Json(json!({ "error": "HOST_NOT_FOUND" }))) }))
            .route(
                "/api/pair/{code}/status",
                get(|Path(code): Path<String>, headers: axum::http::HeaderMap| async move {
                    Json(json!({ "status": "pending", "code": code, "secret": headers.get("x-poll-secret").and_then(|v| v.to_str().ok()) }))
                }),
            );
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

    /// 페어링 상태는 코드·secret을 본문으로 받아 릴레이에는 지금처럼(코드는 경로, secret은 헤더) 묻는다
    #[tokio::test]
    async fn pairing_status_takes_the_code_from_the_body() {
        let base = mock_relay().await;
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let state = IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let request: PairStatusBody = serde_json::from_value(json!({ "code": "AB/12", "secret": "s3", "relayUrl": base })).unwrap();
        let (code, data) = body(pairing_status(State(state), Json(request)).await).await;
        assert_eq!((code, data), (StatusCode::OK, json!({ "status": "pending", "code": "AB/12", "secret": "s3" })));
    }

    /// 릴레이는 제공사의 모델 목록을 15초까지 기다린다. 데몬이 먼저 끊으면 첫 시도에 목록이 안 보인다
    #[tokio::test]
    async fn the_model_list_outlasts_a_provider_slower_than_the_default_relay_wait() {
        let app = Router::new().route(
            "/api/nodes/g1/chatbot/models",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_millis(5500)).await;
                Json(json!({ "models": ["m1"] }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let state = IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let params: RelayQueryParams = serde_json::from_value(json!({ "relayUrl": base })).unwrap();
        let (code, data) = body(get_node_chatbot_models(Path("g1".into()), Query(params), State(state)).await).await;
        assert_eq!((code, data), (StatusCode::OK, json!({ "models": ["m1"] })));
    }

    /// 에이전트 연결 끊기: 릴레이가 지웠으면 204, 아니면 릴레이 답 그대로
    #[tokio::test]
    async fn revoking_the_agent_link_passes_the_relay_answer_through() {
        let base = mock_relay().await;
        assert_eq!(signed_delete(&base, "/api/mcp/grant").await.status(), StatusCode::NO_CONTENT);
        assert_eq!(body(signed_delete(&base, "/api/mcp/gone").await).await, (StatusCode::NOT_FOUND, json!({ "error": "HOST_NOT_FOUND" })));
        assert_eq!(signed_delete("http://127.0.0.1:9", "/api/mcp/grant").await.status(), StatusCode::BAD_GATEWAY);
        // 한 연결만 끊을 때는 릴레이가 준 16진수 id만 넘긴다 (사용자 요청 2026-11-08: 여러 에이전트)
        assert!(grant_id_ok("0123456789abcdef"));
        assert!(!grant_id_ok(""));
        assert!(!grant_id_ok("abc&id=x"));
        assert!(!grant_id_ok("../x"));
    }

    /// 노드 목록의 주인은 아직 주인이 없는 길드에만 고정한다 (명령이 처음 올 때와 같은 pin_owner 규칙)
    #[tokio::test]
    async fn listed_node_owners_pin_only_guilds_without_an_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::config_store::ConfigStore::new(tmp.path());
        store
            .update_node_settings("g2", |node| {
                node.insert("ownerDiscordId".into(), json!("pinned"));
                ((), true)
            })
            .await
            .unwrap();
        let nodes = json!([
            { "guildId": "g1", "ownerDiscordId": "o1" },
            { "guildId": "g2", "ownerDiscordId": "other" },
            { "guildId": "g3" },
        ]);
        pin_listed_owners(&store, &nodes).await;
        let settings = store.get_bot_config().await.node_settings;
        assert_eq!(settings["g1"]["ownerDiscordId"], "o1");
        assert_eq!(settings["g2"]["ownerDiscordId"], "pinned", "a pinned owner is never replaced");
        assert_eq!(settings["g2"]["relayOwnerDiscordId"], "other", "the mismatch is shown, as with commands");
        assert!(settings.get("g3").is_none(), "no owner reported, nothing written");
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_a_bad_gateway() {
        let (code, data) = body(signed_get("http://127.0.0.1:9", "/api/hosts/x", empty_list).await).await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert!(data["error"].as_str().unwrap().starts_with("Relay connection failed"));
    }
}
