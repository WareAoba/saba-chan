//! AI 챗봇 설정 (계약: /api/nodes/:guildId/chatbot)
//!
//! 원본은 데몬 config.json `chatbot`이다. 바뀔 때마다, 그리고 릴레이 에이전트가 연결될 때 이 호스트의
//! 길드마다 릴레이에 PUT 해서 맞춘다 (멱등). API 키는 데몬과 릴레이만 안다: 클라이언트에는 `hasApiKey`만.
//!
//! - `GET /api/chatbot` → `{settings, guilds, hasApiKey}`
//! - `PUT /api/chatbot` `{settings?, guilds?: {id: {...}|null}, apiKey?: string|null}` — `settings`·길드 값은
//!   키별로 합치고 `null`인 키는 지운다(릴레이 기본값). 릴레이가 400을 주면 저장하지 않는다.
//!   응답: GET과 같은 내용 + `sync: [{guildId, ok, status?, error?}]`
//! - `DELETE /api/chatbot` — 설정과 키를 지우고 릴레이에서도 지운다
//! - `DELETE /api/chatbot/memory?guildId=` — 장기 기억 지우기 (guildId가 없으면 모든 길드)

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use super::relay::{apply_signed_headers, client, encode_path_segment, resolve_relay_url, send};
use super::super::IPCServer;
use crate::config_store::ChatbotConfig;

fn view(cfg: &ChatbotConfig) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("settings".into(), Value::Object(cfg.settings.clone()));
    out.insert("guilds".into(), json!(cfg.guilds));
    out.insert("hasApiKey".into(), json!(cfg.api_key.as_deref().is_some_and(|k| !k.is_empty())));
    out
}

pub async fn get_chatbot(State(state): State<IPCServer>) -> Response {
    let mut body = view(&state.config_store.get_chatbot());
    body.insert("success".into(), json!(true));
    Json(Value::Object(body)).into_response()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatbotPatch {
    #[serde(default)]
    settings: Option<Map<String, Value>>,
    #[serde(default)]
    guilds: Option<Map<String, Value>>,
    /// 없음 = 그대로, null = 지움, 문자열 = 교체
    #[serde(default, deserialize_with = "some_or_null")]
    api_key: Option<Option<String>>,
}

/// `"apiKey": null`을 "필드 없음"과 구분한다
fn some_or_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// 키별 합치기: null은 지운다
fn merge(into: &mut Map<String, Value>, patch: Map<String, Value>) {
    for (k, v) in patch {
        if v.is_null() {
            into.remove(&k);
        } else {
            into.insert(k, v);
        }
    }
}

fn apply(cfg: &mut ChatbotConfig, patch: ChatbotPatch) -> Result<(), String> {
    if let Some(s) = patch.settings {
        merge(&mut cfg.settings, s);
    }
    for (guild, value) in patch.guilds.unwrap_or_default() {
        match value {
            Value::Null => {
                cfg.guilds.remove(&guild);
            }
            Value::Object(m) => {
                let entry = cfg.guilds.entry(guild.clone()).or_default();
                merge(entry, m);
                if entry.is_empty() {
                    cfg.guilds.remove(&guild);
                }
            }
            _ => return Err(format!("guilds.{} must be an object or null", guild)),
        }
    }
    if let Some(key) = patch.api_key {
        cfg.api_key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    }
    Ok(())
}

/// 이 호스트가 들어 있는 길드: 릴레이의 호스트 노드 목록 + 길드별 설정이 있는 길드
async fn guild_ids(state: &IPCServer, base: &str, cfg: &ChatbotConfig) -> BTreeSet<String> {
    let mut ids: BTreeSet<String> = cfg.guilds.keys().cloned().collect();
    let host = state.config_store.get_bot_config().await.cloud.host_id;
    if host.trim().is_empty() {
        return ids;
    }
    let Ok(http) = client() else { return ids };
    let path = format!("/api/hosts/{}/nodes", encode_path_segment(host.trim()));
    if let Ok((code, Some(Value::Array(nodes)))) = send(apply_signed_headers(http.get(format!("{}{}", base, path)), "GET", &path, None)).await {
        if code.is_success() {
            ids.extend(nodes.iter().filter_map(|n| n.get("guildId").and_then(Value::as_str).map(str::to_string)));
        }
    }
    ids
}

async fn relay_base(state: &IPCServer) -> String {
    let url = state.config_store.get_bot_config().await.cloud.relay_url;
    resolve_relay_url(Some(url.trim()))
}

/// 길드 하나에 서명한 요청. (상태, 본문)
async fn call(base: &str, method: reqwest::Method, path: &str, body: Option<String>) -> Result<(StatusCode, Value), String> {
    let http = client().map_err(|_| "http client".to_string())?;
    let mut req = apply_signed_headers(http.request(method.clone(), format!("{}{}", base, path)), method.as_str(), path, body.as_deref());
    if let Some(b) = body {
        req = req.header("Content-Type", "application/json").body(b);
    }
    match send(req).await {
        Ok((code, data)) => Ok((code, data.unwrap_or(Value::Null))),
        Err(_) => Err("relay connection failed".into()),
    }
}

fn result_row(guild: &str, outcome: &Result<(StatusCode, Value), String>) -> Value {
    match outcome {
        Ok((code, _)) if code.is_success() => json!({ "guildId": guild, "ok": true }),
        Ok((code, body)) => json!({
            "guildId": guild,
            "ok": false,
            "status": code.as_u16(),
            "error": body.get("error").or_else(|| body.get("code")).cloned().unwrap_or(Value::Null),
            "details": body.get("details").cloned().unwrap_or(Value::Null),
        }),
        Err(e) => json!({ "guildId": guild, "ok": false, "error": e }),
    }
}

/// 설정 전체를 길드마다 PUT. 노드 토큰이 없으면 아무것도 하지 않는다
pub async fn push_all(state: &IPCServer, cfg: &ChatbotConfig) -> Vec<(String, Result<(StatusCode, Value), String>)> {
    if super::relay::load_node_token().is_none() {
        return Vec::new();
    }
    let base = relay_base(state).await;
    let mut out = Vec::new();
    for guild in guild_ids(state, &base, cfg).await {
        let mut body = cfg.effective(&guild);
        // 원본은 데몬: 키가 없으면 릴레이의 것도 지운다
        body.insert("apiKey".into(), json!(cfg.api_key));
        let path = format!("/api/nodes/{}/chatbot", encode_path_segment(&guild));
        let outcome = call(&base, reqwest::Method::PUT, &path, Some(Value::Object(body).to_string())).await;
        out.push((guild, outcome));
    }
    out
}

/// 릴레이 에이전트가 연결된 직후 한 번
pub async fn sync_on_connect(state: &IPCServer) {
    let cfg = state.config_store.get_chatbot();
    if cfg == ChatbotConfig::default() {
        return;
    }
    for (guild, outcome) in push_all(state, &cfg).await {
        if !matches!(&outcome, Ok((c, _)) if c.is_success()) {
            tracing::warn!("[Chatbot] sync to guild {} failed: {}", guild, result_row(&guild, &outcome));
        }
    }
}

pub async fn put_chatbot(State(state): State<IPCServer>, Json(patch): Json<ChatbotPatch>) -> Response {
    let mut cfg = state.config_store.get_chatbot();
    if let Err(e) = apply(&mut cfg, patch) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "error": e, "error_code": "validation_failed" }))).into_response();
    }
    let pushed = push_all(&state, &cfg).await;
    // 릴레이가 값을 거절했으면 저장하지 않는다 (연결 실패는 저장하고 다음 연결 때 맞춘다)
    if let Some((guild, outcome)) = pushed.iter().find(|(_, o)| matches!(o, Ok((c, _)) if *c == StatusCode::BAD_REQUEST)) {
        let mut body = result_row(guild, outcome);
        body["success"] = json!(false);
        body["error_code"] = json!("validation_failed");
        return (StatusCode::BAD_REQUEST, Json(body)).into_response();
    }
    if let Err(e) = state.config_store.set_chatbot(&cfg) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response();
    }
    let mut body = view(&cfg);
    body.insert("success".into(), json!(true));
    body.insert("sync".into(), Value::Array(pushed.iter().map(|(g, o)| result_row(g, o)).collect()));
    Json(Value::Object(body)).into_response()
}

pub async fn delete_chatbot(State(state): State<IPCServer>) -> Response {
    let cfg = state.config_store.get_chatbot();
    let base = relay_base(&state).await;
    let mut rows = Vec::new();
    if super::relay::load_node_token().is_some() {
        for guild in guild_ids(&state, &base, &cfg).await {
            let path = format!("/api/nodes/{}/chatbot", encode_path_segment(&guild));
            rows.push(result_row(&guild, &call(&base, reqwest::Method::DELETE, &path, None).await));
        }
    }
    if let Err(e) = state.config_store.set_chatbot(&ChatbotConfig::default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response();
    }
    Json(json!({ "success": true, "sync": rows })).into_response()
}

#[derive(Deserialize)]
pub struct MemoryQuery {
    #[serde(default, rename = "guildId")]
    guild_id: Option<String>,
}

pub async fn delete_memory(State(state): State<IPCServer>, Query(q): Query<MemoryQuery>) -> Response {
    if super::relay::load_node_token().is_none() {
        return (StatusCode::CONFLICT, Json(json!({ "success": false, "error": "node token missing", "error_code": "not_paired" }))).into_response();
    }
    let cfg = state.config_store.get_chatbot();
    let base = relay_base(&state).await;
    let guilds = match q.guild_id.filter(|g| !g.is_empty()) {
        Some(g) => BTreeSet::from([g]),
        None => guild_ids(&state, &base, &cfg).await,
    };
    let mut rows = Vec::new();
    for guild in guilds {
        let path = format!("/api/nodes/{}/chatbot/memory", encode_path_segment(&guild));
        rows.push(result_row(&guild, &call(&base, reqwest::Method::DELETE, &path, None).await));
    }
    let ok = rows.iter().all(|r| r["ok"] == json!(true));
    Json(json!({ "success": ok, "sync": rows })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(v: Value) -> ChatbotPatch {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn patch_merges_removes_and_tracks_the_key() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"enabled": true, "model": "m"}, "apiKey": "sk-1", "guilds": {"g1": {"channelIds": ["1"]}}}))).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("sk-1"));
        apply(&mut cfg, patch(json!({"settings": {"model": null}}))).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("sk-1"), "a missing apiKey keeps the key");
        assert!(!cfg.settings.contains_key("model"));
        assert_eq!(cfg.effective("g1")["channelIds"], json!(["1"]));
        assert_eq!(cfg.effective("g2").get("channelIds"), None);
        apply(&mut cfg, patch(json!({"apiKey": null, "guilds": {"g1": null}}))).unwrap();
        assert_eq!(cfg.api_key, None);
        assert!(cfg.guilds.is_empty());
        assert!(apply(&mut cfg, patch(json!({"guilds": {"g1": 3}}))).is_err());
    }

    #[test]
    fn view_never_shows_the_key() {
        let cfg = ChatbotConfig { api_key: Some("sk-secret".into()), ..Default::default() };
        let text = Value::Object(view(&cfg)).to_string();
        assert!(!text.contains("sk-secret"));
        assert!(text.contains("\"hasApiKey\":true"));
    }
}
