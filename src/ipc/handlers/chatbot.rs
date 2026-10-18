//! AI 챗봇 설정 (계약: /api/nodes/:guildId/chatbot)
//!
//! 원본은 데몬 config.json `chatbot`이다. 바뀔 때마다, 그리고 릴레이 에이전트가 연결될 때 이 호스트의
//! 길드마다 릴레이에 PUT 해서 맞춘다 (멱등). API 키는 데몬과 릴레이만 안다: 클라이언트에는 `hasApiKey`만.
//!
//! - `GET /api/chatbot` → `{settings, guilds, hasApiKey}`
//! - `PUT /api/chatbot` `{settings?, guilds?: {id: {...}|null}, apiKey?: string|null}` — `settings`·길드 값은
//!   키별로 합치고 `null`인 키는 지운다(릴레이 기본값). 릴레이가 400을 주면 저장하지 않는다.
//!   응답: GET과 같은 내용 + `sync: [{guildId, ok, status?, error?}]`
//! - `DELETE /api/chatbot` — 설정과 키를 지우고 릴레이에서도 지운다. 릴레이에 못 닿으면
//!   `pendingRelayDelete: true`로 답하고 다음 연결 때 다시 지운다
//!
//! 릴레이 PUT은 받은 키만 합치므로 데몬은 늘 전체 설정을 보낸다: 지운 키(null)는 `RELAY_DEFAULTS` 값으로.
//! 릴레이에 새 설정이 생기면 그 기본값을 이 표에 한 줄 더한다
//! - `DELETE /api/chatbot/memory?guildId=` — 장기 기억 지우기 (guildId가 없으면 모든 길드)

use axum::{
    extract::{Path, Query, State},
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

/// 릴레이 `ChatbotSettings` 기본값 (/mnt/project-files/chatbot/contract.md §1).
/// 기본값으로 되돌린 키도 릴레이에 실제로 닿게 PUT마다 깔고 그 위에 사용자 설정을 얹는다
fn relay_defaults() -> Map<String, Value> {
    let Value::Object(map) = json!({
        "enabled": false,
        "provider": "openai",
        "model": "gpt-6-luna",
        "memo": "",
        "confirmCommands": true,
        "memory": true,
        "webSearch": true,
        "disabledTools": [],
    }) else {
        unreachable!()
    };
    map
}

/// 릴레이가 고정값으로 바꾼 예전 설정 (챗봇 설정 v2 §2·§4). 받아도 저장하지 않고 릴레이에도 보내지 않는다
const RETIRED: &[&str] = &["triggerWords", "sessionSeconds", "ambientSeconds"];

fn drop_retired(map: &mut Map<String, Value>) {
    for k in RETIRED {
        map.remove(*k);
    }
}

/// 길드 하나에 PUT할 본문: 기본값 위에 사용자 설정, 키는 데몬 것 (없으면 릴레이 것도 지운다)
fn put_body(cfg: &ChatbotConfig, guild: &str) -> Map<String, Value> {
    let mut body = relay_defaults();
    body.extend(cfg.effective(guild));
    drop_retired(&mut body);
    body.insert("apiKey".into(), json!(cfg.api_key));
    body
}

fn view(cfg: &ChatbotConfig) -> Map<String, Value> {
    let mut out = Map::new();
    let mut settings = cfg.settings.clone();
    drop_retired(&mut settings);
    out.insert("settings".into(), Value::Object(settings));
    out.insert("guilds".into(), json!(cfg.guilds));
    out.insert("hasApiKey".into(), json!(cfg.api_key.as_deref().is_some_and(|k| !k.is_empty())));
    out.insert("pendingRelayDelete".into(), json!(cfg.pending_relay_delete));
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
    drop_retired(&mut cfg.settings);
    for (guild, value) in patch.guilds.unwrap_or_default() {
        match value {
            Value::Null => {
                cfg.guilds.remove(&guild);
            }
            Value::Object(m) => {
                let entry = cfg.guilds.entry(guild.clone()).or_default();
                merge(entry, m);
                drop_retired(entry);
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
    // PUT은 전체 설정을 보내므로 남은 삭제를 덮는다
    cfg.pending_relay_delete = false;
    Ok(())
}

/// 이 호스트가 들어 있는 길드: 릴레이의 호스트 노드 목록 + 길드별 설정이 있는 길드.
/// 둘째 값은 릴레이 목록을 받았는지 (호스트가 없으면 받을 것도 없으니 true)
async fn guild_ids(state: &IPCServer, base: &str, cfg: &ChatbotConfig) -> (BTreeSet<String>, bool) {
    let mut ids: BTreeSet<String> = cfg.guilds.keys().cloned().collect();
    match host_guilds(state, base).await {
        Some(hosted) => {
            ids.extend(hosted);
            (ids, true)
        }
        None => (ids, false),
    }
}

/// 릴레이에 등록된 이 호스트의 길드. 못 받으면 None (호스트가 없으면 빈 목록)
pub(super) async fn host_guilds(state: &IPCServer, base: &str) -> Option<Vec<String>> {
    let host = state.config_store.get_bot_config().await.cloud.host_id;
    if host.trim().is_empty() {
        return Some(Vec::new());
    }
    let http = client().ok()?;
    let path = format!("/api/hosts/{}/nodes", encode_path_segment(host.trim()));
    match send(apply_signed_headers(http.get(format!("{}{}", base, path)), "GET", &path, None)).await {
        Ok((code, Some(Value::Array(nodes)))) if code.is_success() => {
            Some(nodes.iter().filter_map(|n| n.get("guildId").and_then(Value::as_str).map(str::to_string)).collect())
        }
        _ => None,
    }
}

pub(super) async fn relay_base(state: &IPCServer) -> String {
    let url = state.config_store.get_bot_config().await.cloud.relay_url;
    resolve_relay_url(Some(url.trim()))
}

/// 길드 하나에 서명한 요청. (상태, 본문)
pub(super) async fn call(base: &str, method: reqwest::Method, path: &str, body: Option<String>) -> Result<(StatusCode, Value), String> {
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

pub(super) fn result_row(guild: &str, outcome: &Result<(StatusCode, Value), String>) -> Value {
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
    for guild in guild_ids(state, &base, cfg).await.0 {
        let body = put_body(cfg, &guild);
        let path = format!("/api/nodes/{}/chatbot", encode_path_segment(&guild));
        let outcome = call(&base, reqwest::Method::PUT, &path, Some(Value::Object(body).to_string())).await;
        out.push((guild, outcome));
    }
    out
}

/// 릴레이에서 이 호스트 길드들의 설정과 키를 지운다. 모두 지워졌으면 true
async fn delete_remote(state: &IPCServer, cfg: &ChatbotConfig) -> (bool, Vec<Value>) {
    if super::relay::load_node_token().is_none() {
        return (true, Vec::new());
    }
    let base = relay_base(state).await;
    let (guilds, listed) = guild_ids(state, &base, cfg).await;
    let mut rows = Vec::new();
    for guild in guilds {
        let path = format!("/api/nodes/{}/chatbot", encode_path_segment(&guild));
        let outcome = call(&base, reqwest::Method::DELETE, &path, None).await;
        // 이미 없는 노드(404)는 지워진 것으로 본다
        let gone = matches!(&outcome, Ok((c, _)) if c.is_success() || *c == StatusCode::NOT_FOUND);
        let mut row = result_row(&guild, &outcome);
        row["ok"] = json!(gone);
        rows.push(row);
    }
    // 길드 목록을 못 받았으면 빠진 길드가 있을 수 있다
    (listed && rows.iter().all(|r| r["ok"] == json!(true)), rows)
}

/// 릴레이 에이전트가 연결된 직후 한 번
pub async fn sync_on_connect(state: &IPCServer) {
    let cfg = state.config_store.get_chatbot();
    if cfg.pending_relay_delete {
        let (done, rows) = delete_remote(state, &cfg).await;
        if done {
            let mut cleared = cfg.clone();
            cleared.pending_relay_delete = false;
            if let Err(e) = state.config_store.set_chatbot(&cleared) {
                tracing::warn!("[Chatbot] could not clear the pending delete: {}", e);
            }
        } else {
            tracing::warn!("[Chatbot] relay delete still pending: {}", json!(rows));
        }
        return;
    }
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
    let (done, rows) = delete_remote(&state, &cfg).await;
    let cleared = ChatbotConfig { pending_relay_delete: !done, ..ChatbotConfig::default() };
    if let Err(e) = state.config_store.set_chatbot(&cleared) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response();
    }
    Json(json!({ "success": true, "sync": rows, "pendingRelayDelete": !done })).into_response()
}

/// 길드 하나의 챗봇 설정 (I2-b, 디스코드 `/사바쨩 설정`이 액션으로 부른다): 릴레이 기본값 위에 실제 값, 키는 `hasApiKey`만
fn guild_view(cfg: &ChatbotConfig, guild: &str) -> Value {
    let mut settings = relay_defaults();
    settings.extend(cfg.effective(guild));
    drop_retired(&mut settings);
    json!({ "success": true, "guildId": guild, "settings": settings, "hasApiKey": cfg.api_key.as_deref().is_some_and(|k| !k.is_empty()) })
}

/// `GET /api/chatbot/guilds/:guild`
pub async fn get_guild(State(state): State<IPCServer>, Path(guild): Path<String>) -> Response {
    Json(guild_view(&state.config_store.get_chatbot(), &guild)).into_response()
}

/// `PATCH /api/chatbot/guilds/:guild` `{설정 키: 값|null}`: 그 길드 값만 합친다. 키(`apiKey`)와 모르는 키는 받지 않는다
pub async fn patch_guild(State(state): State<IPCServer>, Path(guild): Path<String>, Json(mut patch): Json<Map<String, Value>>) -> Response {
    drop_retired(&mut patch);
    let known = relay_defaults();
    if let Some(bad) = patch.keys().find(|k| !known.contains_key(*k)) {
        let error = json!({ "success": false, "error": format!("unknown chatbot setting: {}", bad), "error_code": "validation_failed", "fields": [{ "field": bad }] });
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }
    let body = ChatbotPatch { guilds: Some(Map::from_iter([(guild.clone(), Value::Object(patch))])), ..Default::default() };
    let resp = put_chatbot(State(state.clone()), Json(body)).await;
    if !resp.status().is_success() {
        return resp;
    }
    Json(guild_view(&state.config_store.get_chatbot(), &guild)).into_response()
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
        None => guild_ids(&state, &base, &cfg).await.0,
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
    fn settings_reset_to_default_still_reach_the_relay() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"enabled": true, "confirmCommands": false}}))).unwrap();
        // GUI는 기본값으로 되돌릴 때 null을 보낸다. 릴레이는 받은 키만 합치므로 빠뜨리면 예전 값이 남는다
        apply(&mut cfg, patch(json!({"settings": {"enabled": null, "confirmCommands": null}}))).unwrap();
        let body = put_body(&cfg, "g1");
        assert_eq!(body["enabled"], json!(false));
        assert_eq!(body["confirmCommands"], json!(true));
        assert_eq!(body["apiKey"], Value::Null, "no key here means delete it there");
        apply(&mut cfg, patch(json!({"settings": {"model": "m2"}, "guilds": {"g1": {"memo": "g1 only"}}}))).unwrap();
        let body = put_body(&cfg, "g1");
        assert_eq!(body["model"], json!("m2"));
        assert_eq!(body["memo"], json!("g1 only"));
        assert_eq!(put_body(&cfg, "g2")["memo"], json!(""));
        // 챗봇 채널은 릴레이가 원본: 데몬은 보내지 않는다
        assert!(!body.contains_key("channelIds"));
    }

    #[test]
    fn a_pending_relay_delete_survives_until_a_new_put() {
        let pending = ChatbotConfig { pending_relay_delete: true, ..ChatbotConfig::default() };
        assert_ne!(pending, ChatbotConfig::default(), "sync_on_connect must not skip it as empty");
        let text = serde_json::to_string(&pending).unwrap();
        assert_eq!(serde_json::from_str::<ChatbotConfig>(&text).unwrap(), pending);
        let mut cfg = pending;
        apply(&mut cfg, patch(json!({"settings": {"enabled": true}}))).unwrap();
        assert!(!cfg.pending_relay_delete, "a full PUT replaces what the delete would have removed");
    }

    #[test]
    fn patch_merges_removes_and_tracks_the_key() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"enabled": true, "model": "m"}, "apiKey": "sk-1", "guilds": {"g1": {"memo": "1"}}}))).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("sk-1"));
        apply(&mut cfg, patch(json!({"settings": {"model": null}}))).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("sk-1"), "a missing apiKey keeps the key");
        assert!(!cfg.settings.contains_key("model"));
        assert_eq!(cfg.effective("g1")["memo"], json!("1"));
        assert_eq!(cfg.effective("g2").get("memo"), None);
        apply(&mut cfg, patch(json!({"apiKey": null, "guilds": {"g1": null}}))).unwrap();
        assert_eq!(cfg.api_key, None);
        assert!(cfg.guilds.is_empty());
        assert!(apply(&mut cfg, patch(json!({"guilds": {"g1": 3}}))).is_err());
    }

    #[test]
    fn retired_settings_are_accepted_but_never_kept_or_sent() {
        // 옛 GUI·옛 설정 파일이 보내는 값: 릴레이는 고정값을 쓰니 데몬도 버린다
        let mut cfg = ChatbotConfig::default();
        cfg.guilds.insert("g1".into(), Map::from_iter([("sessionSeconds".to_string(), json!(60))]));
        apply(&mut cfg, patch(json!({"settings": {"enabled": true, "triggerWords": ["사바"], "ambientSeconds": 30}}))).unwrap();
        assert_eq!(cfg.settings, Map::from_iter([("enabled".to_string(), json!(true))]));
        let body = put_body(&cfg, "g1");
        assert!(RETIRED.iter().all(|k| !body.contains_key(*k)));
        assert!(RETIRED.iter().all(|k| guild_view(&cfg, "g1")["settings"].get(*k).is_none()));
    }

    #[test]
    fn view_never_shows_the_key() {
        let cfg = ChatbotConfig { api_key: Some("sk-secret".into()), ..Default::default() };
        let text = Value::Object(view(&cfg)).to_string();
        assert!(!text.contains("sk-secret"));
        assert!(text.contains("\"hasApiKey\":true"));
    }
}
