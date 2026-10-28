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
//!
//! 켜기(`enabled`)와 이 서버에서 켠 도구(`tools`)는 서버마다 둔다 (기본: 꺼짐, 도구 없음). 공통 `allowedTools`는
//! 전체 허용 도구로 데몬만 안다(없으면 모두 허용): 릴레이에는 서버의 `tools` 중 허용된 것만 간다.
//! 옛 설정(공통 `enabled`, `disabledTools`, `webSearch`)은 길드 목록을 받았을 때 서버마다 적어 넣는다 (`settle_legacy`)
//! - `DELETE /api/chatbot/memory?guildId=` — 장기 기억 지우기 (guildId가 없으면 모든 길드)
//! - `GET /api/chatbot/memo-template?locale=` — 서버 프롬프트 기본 틀 `{template, locale}` (bot.json `chatbot.memo_template`)

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
use crate::chat::i18n::Catalog;

/// 릴레이 `ChatbotSettings` 기본값 (/mnt/project-files/chatbot/contract.md §1).
/// 기본값으로 되돌린 키도 릴레이에 실제로 닿게 PUT마다 깔고 그 위에 사용자 설정을 얹는다.
/// 제공사(`provider`)는 릴레이가 키로 정하므로 바꿀 수 있는 키가 아니다
fn relay_defaults() -> Map<String, Value> {
    let Value::Object(map) = json!({
        "enabled": false,
        "model": "gpt-6-luna",
        "memo": "",
        "effort": "medium",
        "ambient": true,
        "ambientMinutes": 3,
        "ambientEffort": "low",
        "confirmLevel": "all",
        "memory": true,
        "tools": [],
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
    // 옛 GUI·액션이 보내는 확인 설정은 확인 단계로 옮긴다
    crate::config_store::upgrade_confirm(map);
}

/// 공통 전체 허용 도구 (데몬만 안다). 없으면 모두 허용
const ALLOWED_TOOLS: &str = "allowedTools";
/// 서버마다만 두는 키
const GUILD_ONLY: &[&str] = &["enabled", "tools"];
/// 옛 도구 설정 — 서버별 `tools`로 옮긴다
const LEGACY_TOOL_KEYS: &[&str] = &["disabledTools", "webSearch"];
/// 옛 데몬이 끌 수 있던 도구 (릴레이 toolkit `LEGACY_TOOL_NAMES`). 옛 설정을 옮길 때 여기서 끈 것만 뺀다
const LEGACY_TOOL_NAMES: &[&str] = &["game_servers", "research", "lol", "images"];

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// 옛 설정이 남아 있다: 공통 켜기, 또는 어디든 끈 도구·웹 검색 스위치
fn has_legacy(cfg: &ChatbotConfig) -> bool {
    cfg.settings.contains_key("enabled")
        || cfg.settings.contains_key("tools")
        || std::iter::once(&cfg.settings).chain(cfg.guilds.values()).any(|m| LEGACY_TOOL_KEYS.iter().any(|k| m.contains_key(*k)))
}

/// 서버 하나의 켜기와 켠 도구 (허용 목록을 거르기 전). 서버에 값이 없으면 기본(꺼짐, 도구 없음)이고,
/// 옛 설정이 남아 있으면 옛 값으로 셈한다: 켜진 서버는 끈 도구 밖의 옛 도구 + 웹 검색 스위치
fn selection(cfg: &ChatbotConfig, guild: &str) -> (bool, Vec<String>) {
    let own = cfg.guilds.get(guild);
    let legacy = has_legacy(cfg);
    let eff = cfg.effective(guild);
    let enabled = own.and_then(|m| m.get("enabled")).or(if legacy { eff.get("enabled") } else { None }).and_then(Value::as_bool).unwrap_or(false);
    if let Some(tools) = own.and_then(|m| m.get("tools")) {
        return (enabled, strings(Some(tools)));
    }
    if !legacy || !enabled {
        return (enabled, Vec::new());
    }
    let off = strings(eff.get("disabledTools"));
    let mut tools: Vec<String> = LEGACY_TOOL_NAMES.iter().filter(|n| !off.iter().any(|o| o == *n)).map(|n| n.to_string()).collect();
    if eff.get("webSearch").and_then(Value::as_bool).unwrap_or(true) {
        tools.push("web_search".into());
    }
    (enabled, tools)
}

/// 서버에 값이 없을 때 쓰는 켜기·도구 — 클라이언트가 노드 목록의 서버를 그릴 때 쓴다 (옮기기 전에는 옛 공통 값)
fn guild_default(cfg: &ChatbotConfig) -> Value {
    // 어떤 길드 id와도 겹치지 않는 이름 (디스코드 id는 숫자)
    let (enabled, tools) = selection(cfg, "");
    json!({ "enabled": enabled, "tools": tools })
}

/// 옛 설정을 받은 길드마다 적어 넣고 옛 키를 지운다. 바꿨으면 true
pub(super) fn settle_legacy(cfg: &mut ChatbotConfig, guilds: impl IntoIterator<Item = String>) -> bool {
    if !has_legacy(cfg) {
        return false;
    }
    let settled: Vec<(String, bool, Vec<String>)> = guilds.into_iter().map(|g| {
        let (enabled, tools) = selection(cfg, &g);
        (g, enabled, tools)
    }).collect();
    for (guild, enabled, tools) in settled {
        let entry = cfg.guilds.entry(guild).or_default();
        entry.insert("enabled".into(), json!(enabled));
        entry.insert("tools".into(), json!(tools));
    }
    cfg.settings.remove("enabled");
    cfg.settings.remove("tools");
    for map in std::iter::once(&mut cfg.settings).chain(cfg.guilds.values_mut()) {
        for k in LEGACY_TOOL_KEYS {
            map.remove(*k);
        }
    }
    true
}

/// 길드 하나에 PUT할 본문: 기본값 위에 사용자 설정, 키는 데몬 것 (없으면 릴레이 것도 지운다)
fn put_body(cfg: &ChatbotConfig, guild: &str) -> Map<String, Value> {
    let mut body = relay_defaults();
    body.extend(cfg.effective(guild));
    drop_retired(&mut body);
    let (enabled, mut tools) = selection(cfg, guild);
    if let Some(allowed) = cfg.settings.get(ALLOWED_TOOLS).filter(|v| v.is_array()) {
        let allowed = strings(Some(allowed));
        tools.retain(|t| allowed.contains(t));
    }
    body.remove(ALLOWED_TOOLS);
    for k in LEGACY_TOOL_KEYS {
        body.remove(*k);
    }
    body.insert("enabled".into(), json!(enabled));
    body.insert("tools".into(), json!(tools));
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
    out.insert("guildDefault".into(), guild_default(cfg));
    // 공통에서 바꿀 수 있는 키와 그 기본값. 클라이언트가 키 이름을 따로 두지 않는다
    let mut defaults = relay_defaults();
    for k in GUILD_ONLY {
        defaults.remove(*k);
    }
    defaults.insert(ALLOWED_TOOLS.into(), Value::Null);
    out.insert("defaults".into(), Value::Object(defaults));
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

/// 받은 patch의 옛 확인 설정을 합치기 전에 옮긴다 (합친 뒤에 옮기면 저장된 단계에 밀린다). null = 기본값으로
fn upgrade_patch(map: &mut Map<String, Value>) {
    if map.get("confirmCommands") == Some(&Value::Null) {
        map.remove("confirmCommands");
        map.entry("confirmLevel").or_insert(Value::Null);
    }
    crate::config_store::upgrade_confirm(map);
}

fn apply(cfg: &mut ChatbotConfig, patch: ChatbotPatch) -> Result<(), String> {
    if let Some(mut s) = patch.settings {
        if let Some(k) = GUILD_ONLY.iter().find(|k| s.get(**k).is_some_and(|v| !v.is_null())) {
            return Err(format!("{} is set per server (guilds.<id>.{})", k, k));
        }
        upgrade_patch(&mut s);
        merge(&mut cfg.settings, s);
    }
    drop_retired(&mut cfg.settings);
    for (guild, value) in patch.guilds.unwrap_or_default() {
        match value {
            Value::Null => {
                cfg.guilds.remove(&guild);
            }
            Value::Object(mut m) => {
                if m.contains_key(ALLOWED_TOOLS) {
                    return Err(format!("{} is a common setting (settings.{})", ALLOWED_TOOLS, ALLOWED_TOOLS));
                }
                upgrade_patch(&mut m);
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

/// 길드 하나에 보낸 결과: (릴레이 상태, 본문) 또는 연결 실패
type PushOutcome = Result<(StatusCode, Value), String>;

/// 설정 전체를 길드마다 PUT. 노드 토큰이 없으면 아무것도 하지 않는다.
/// `previous`가 있으면 거절(400)된 길드에서 멈추고 이미 받은 길드를 그 값으로 되돌린다
pub async fn push_all(state: &IPCServer, cfg: &ChatbotConfig, previous: Option<&ChatbotConfig>) -> Vec<(String, PushOutcome)> {
    if super::relay::load_node_token().is_none() {
        return Vec::new();
    }
    let base = relay_base(state).await;
    let guilds = guild_ids(state, &base, cfg).await.0;
    let put = |guild: String, body: Map<String, Value>| {
        let base = base.clone();
        async move {
            let path = format!("/api/nodes/{}/chatbot", encode_path_segment(&guild));
            call(&base, reqwest::Method::PUT, &path, Some(Value::Object(body).to_string())).await
        }
    };
    match previous {
        Some(old) => push_or_restore(guilds, cfg, old, put).await,
        None => {
            let mut out = Vec::new();
            for guild in guilds {
                let outcome = put(guild.clone(), put_body(cfg, &guild)).await;
                out.push((guild, outcome));
            }
            out
        }
    }
}

/// 길드마다 `new`를 보낸다. 한 길드가 값을 거절(400)하면 거기서 멈추고, 이미 받은 길드에 `old`를 다시 보낸다.
/// 거절되면 데몬은 저장하지 않으므로 받은 길드만 새 값으로 남지 않게 한다 (D-F7)
async fn push_or_restore<F, Fut>(guilds: impl IntoIterator<Item = String>, new: &ChatbotConfig, old: &ChatbotConfig, put: F) -> Vec<(String, PushOutcome)>
where
    F: Fn(String, Map<String, Value>) -> Fut,
    Fut: std::future::Future<Output = PushOutcome>,
{
    let mut out: Vec<(String, PushOutcome)> = Vec::new();
    for guild in guilds {
        let outcome = put(guild.clone(), put_body(new, &guild)).await;
        let rejected = matches!(&outcome, Ok((c, _)) if *c == StatusCode::BAD_REQUEST);
        out.push((guild, outcome));
        if rejected {
            for (guild, outcome) in &out {
                if matches!(outcome, Ok((c, _)) if c.is_success()) {
                    let undo = put(guild.clone(), put_body(old, guild)).await;
                    if !matches!(&undo, Ok((c, _)) if c.is_success()) {
                        tracing::warn!("[Chatbot] could not put guild {} back after a rejected save: {}", guild, result_row(guild, &undo));
                    }
                }
            }
            break;
        }
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
    let _turn = SAVE_LOCK.lock().await;
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
    let cfg = settled(state, cfg).await;
    for (guild, outcome) in push_all(state, &cfg, None).await {
        if !matches!(&outcome, Ok((c, _)) if c.is_success()) {
            tracing::warn!("[Chatbot] sync to guild {} failed: {}", guild, result_row(&guild, &outcome));
        }
    }
}

/// 챗봇 설정의 읽고-바꾸고-저장하기를 한 번에 하나씩 (키 교체와 설정 저장이 겹쳐도 서로의 값을 지우지 않게)
pub(super) static SAVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 옛 설정이 남았고 길드 목록을 받을 수 있으면 서버마다 적어 넣고 저장한다 (SAVE_LOCK 안에서).
/// 페어링 전이거나 목록을 못 받으면 그대로 둔다: 빠진 서버의 옛 값이 사라지지 않게
async fn settled(state: &IPCServer, mut cfg: ChatbotConfig) -> ChatbotConfig {
    if !has_legacy(&cfg) || super::relay::load_node_token().is_none() {
        return cfg;
    }
    let base = relay_base(state).await;
    let (guilds, listed) = guild_ids(state, &base, &cfg).await;
    if listed && settle_legacy(&mut cfg, guilds) {
        if let Err(e) = state.config_store.set_chatbot(&cfg) {
            tracing::warn!("[Chatbot] could not save the per-server settings: {}", e);
        }
    }
    cfg
}

pub async fn put_chatbot(State(state): State<IPCServer>, Json(patch): Json<ChatbotPatch>) -> Response {
    let _turn = SAVE_LOCK.lock().await;
    let mut cfg = settled(&state, state.config_store.get_chatbot()).await;
    let previous = cfg.clone();
    if let Err(e) = apply(&mut cfg, patch) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "error": e, "error_code": "validation_failed" }))).into_response();
    }
    let pushed = push_all(&state, &cfg, Some(&previous)).await;
    // 릴레이가 값을 거절했으면 저장하지 않는다(받은 길드는 push_all이 되돌렸다). 연결 실패는 저장하고 다음 연결 때 맞춘다
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
    let _turn = SAVE_LOCK.lock().await;
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
    for k in LEGACY_TOOL_KEYS {
        settings.remove(*k);
    }
    // 이 서버의 선택(허용 목록을 거르기 전)과 공통 허용 목록 (null = 모두)
    let (enabled, tools) = selection(cfg, guild);
    settings.insert("enabled".into(), json!(enabled));
    settings.insert("tools".into(), json!(tools));
    settings.insert(ALLOWED_TOOLS.into(), cfg.settings.get(ALLOWED_TOOLS).cloned().unwrap_or(Value::Null));
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

/// 서버 프롬프트 기본 틀 — 제목과 HTML 주석 안내. 릴레이는 주석과 채우지 않은 제목을 모델에게 보내지 않는다
fn memo_template(language: &str) -> String {
    Catalog::for_language(language).t("chatbot.memo_template", &[])
}

#[derive(Deserialize)]
pub struct TemplateQuery {
    #[serde(default)]
    locale: Option<String>,
}

/// `GET /api/chatbot/memo-template?locale=` (없으면 앱 언어)
pub async fn get_memo_template(State(state): State<IPCServer>, Query(q): Query<TemplateQuery>) -> Response {
    let requested = match q.locale.filter(|l| !l.trim().is_empty()) {
        Some(l) => l,
        None => state.config_store.ui_language().await,
    };
    let locale = crate::chat::i18n::normalize_language(&requested);
    Json(json!({ "success": true, "locale": locale, "template": memo_template(&locale) })).into_response()
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

    /// 읽고-바꾸고-저장하기는 한 번에 하나씩: 키 교체와 설정 저장이 겹치면 나중 것이 옛 값으로 새 키를 지운다
    #[tokio::test]
    async fn a_save_waits_for_the_one_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            dir.path().join("modules").to_str().unwrap(),
            dir.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let turn = SAVE_LOCK.lock().await;
        // 거절되는 값이라 공유 설정 파일은 바뀌지 않는다
        let save = tokio::spawn(put_chatbot(State(state), Json(patch(json!({"guilds": {"g1": 1}})))));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!save.is_finished(), "a save read the settings while another save was between its read and its write");
        drop(turn);
        let res = tokio::time::timeout(std::time::Duration::from_secs(5), save).await.unwrap().unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    /// D-F7: 길드 하나가 값을 거절하면 저장하지 않으므로, 이미 받은 길드도 옛 값으로 되돌려 데몬과 어긋나지 않게 한다
    #[tokio::test]
    async fn a_value_one_guild_rejects_is_rolled_back_on_the_guilds_that_took_it() {
        let old = ChatbotConfig::default();
        let mut new = old.clone();
        apply(&mut new, patch(json!({"settings": {"model": "m2"}}))).unwrap();
        let calls = std::sync::Mutex::new(Vec::new());
        let guilds = vec!["g1".to_string(), "g2".to_string(), "g3".to_string()];
        let pushed = push_or_restore(guilds, &new, &old, |guild, body| {
            let model = body["model"].clone();
            calls.lock().unwrap().push((guild.clone(), model.clone()));
            async move {
                let code = if guild == "g2" && model == json!("m2") { StatusCode::BAD_REQUEST } else { StatusCode::OK };
                Ok((code, Value::Null))
            }
        })
        .await;
        assert!(pushed.iter().any(|(g, o)| g == "g2" && matches!(o, Ok((c, _)) if *c == StatusCode::BAD_REQUEST)));
        let calls = calls.into_inner().unwrap();
        let old_model = put_body(&old, "g1")["model"].clone();
        assert_eq!(
            calls,
            vec![("g1".to_string(), json!("m2")), ("g2".to_string(), json!("m2")), ("g1".to_string(), old_model)],
            "stop at the rejection, leave g3 untouched and put g1 back"
        );
    }

    #[test]
    fn settings_reset_to_default_still_reach_the_relay() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"memory": false, "confirmLevel": "danger"}}))).unwrap();
        // GUI는 기본값으로 되돌릴 때 null을 보낸다. 릴레이는 받은 키만 합치므로 빠뜨리면 예전 값이 남는다
        apply(&mut cfg, patch(json!({"settings": {"memory": null, "confirmLevel": null}}))).unwrap();
        let body = put_body(&cfg, "g1");
        assert_eq!(body["memory"], json!(true));
        assert_eq!(body["confirmLevel"], json!("all"));
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
        apply(&mut cfg, patch(json!({"settings": {"memory": false}}))).unwrap();
        assert!(!cfg.pending_relay_delete, "a full PUT replaces what the delete would have removed");
    }

    #[test]
    fn patch_merges_removes_and_tracks_the_key() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"memory": false, "model": "m"}, "apiKey": "sk-1", "guilds": {"g1": {"memo": "1"}}}))).unwrap();
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
        apply(&mut cfg, patch(json!({"settings": {"memory": false, "triggerWords": ["사바"], "ambientSeconds": 30}}))).unwrap();
        assert_eq!(cfg.settings, Map::from_iter([("memory".to_string(), json!(false))]));
        let body = put_body(&cfg, "g1");
        assert!(RETIRED.iter().all(|k| !body.contains_key(*k)));
        assert!(RETIRED.iter().all(|k| guild_view(&cfg, "g1")["settings"].get(*k).is_none()));
    }

    /// 옛 GUI·옛 설정 파일의 `confirmCommands`는 확인 단계로 옮겨 저장하고 보낸다 (켜짐 = 모두 묻기, 꺼짐 = 위험만)
    #[test]
    fn the_old_confirm_switch_becomes_a_confirm_level() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"confirmCommands": false}, "guilds": {"g1": {"confirmCommands": true}}}))).unwrap();
        assert_eq!(cfg.settings, Map::from_iter([("confirmLevel".to_string(), json!("danger"))]));
        assert_eq!(put_body(&cfg, "g1")["confirmLevel"], json!("all"));
        assert!(!put_body(&cfg, "g2").contains_key("confirmCommands"));
        // 새 값이 함께 오면 새 값을 둔다
        apply(&mut cfg, patch(json!({"settings": {"confirmCommands": true, "confirmLevel": "caution"}}))).unwrap();
        assert_eq!(cfg.settings["confirmLevel"], json!("caution"));
        // 옛 키로 보낸 새 저장은 저장된 단계를 바꾸고, null이면 기본값으로 돌린다
        apply(&mut cfg, patch(json!({"settings": {"confirmCommands": false}}))).unwrap();
        assert_eq!(cfg.settings["confirmLevel"], json!("danger"));
        apply(&mut cfg, patch(json!({"settings": {"confirmCommands": null}}))).unwrap();
        assert!(!cfg.settings.contains_key("confirmLevel"));
    }

    /// 켜기와 도구는 서버마다: 공통 허용 도구와 겹친 것만 릴레이에 간다. 허용 목록과 옛 도구 설정은 보내지 않는다
    #[test]
    fn each_server_sends_its_own_switch_and_the_allowed_part_of_its_tools() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({
            "settings": {"allowedTools": ["lol", "pubg", "web_search"]},
            "guilds": {"g1": {"enabled": true, "tools": ["lol", "images", "web_search"]}},
        }))).unwrap();
        let g1 = put_body(&cfg, "g1");
        assert_eq!(g1["enabled"], json!(true));
        assert_eq!(g1["tools"], json!(["lol", "web_search"]));
        for key in ["allowedTools", "disabledTools", "webSearch"] {
            assert!(!g1.contains_key(key), "{key} must not reach the relay");
        }
        // 새 서버는 꺼져 있고 도구도 없다
        let g2 = put_body(&cfg, "g2");
        assert_eq!(g2["enabled"], json!(false));
        assert_eq!(g2["tools"], json!([]));
        // 허용 목록이 없으면 모두 허용
        apply(&mut cfg, patch(json!({"settings": {"allowedTools": null}}))).unwrap();
        assert_eq!(put_body(&cfg, "g1")["tools"], json!(["lol", "images", "web_search"]));
    }

    /// 옛 설정(공통 켜기·끈 도구·웹 검색 스위치)은 서버마다 적어 넣는다: 켜 둔 서버는 쓰던 도구 그대로, 꺼진 서버는 도구 없이
    #[test]
    fn old_settings_are_written_out_per_server() {
        let mut cfg = ChatbotConfig::default();
        cfg.settings = Map::from_iter([
            ("enabled".to_string(), json!(true)),
            ("disabledTools".to_string(), json!(["images"])),
            ("webSearch".to_string(), json!(false)),
            ("memory".to_string(), json!(false)),
        ]);
        cfg.guilds.insert("off".into(), Map::from_iter([("enabled".to_string(), json!(false))]));
        // 옮기기 전에도 같은 값을 보낸다
        assert_eq!(put_body(&cfg, "on")["tools"], json!(["game_servers", "research", "lol"]));
        assert_eq!(guild_default(&cfg), json!({"enabled": true, "tools": ["game_servers", "research", "lol"]}));

        assert!(settle_legacy(&mut cfg, ["on".to_string(), "off".to_string()]));
        assert_eq!(cfg.settings, Map::from_iter([("memory".to_string(), json!(false))]));
        assert_eq!(cfg.guilds["on"]["enabled"], json!(true));
        assert_eq!(cfg.guilds["on"]["tools"], json!(["game_servers", "research", "lol"]));
        assert_eq!(cfg.guilds["off"]["enabled"], json!(false));
        assert_eq!(cfg.guilds["off"]["tools"], json!([]));
        // 그 뒤에 연결한 서버는 기본값(꺼짐)이다
        assert_eq!(put_body(&cfg, "new")["enabled"], json!(false));
        assert_eq!(guild_default(&cfg), json!({"enabled": false, "tools": []}));
        assert!(!settle_legacy(&mut cfg, ["on".to_string()]), "nothing left to move");

        // 웹 검색 스위치를 건드리지 않았으면 켜져 있던 것이다
        let mut plain = ChatbotConfig::default();
        plain.guilds.insert("g".into(), Map::from_iter([("enabled".to_string(), json!(true)), ("disabledTools".to_string(), json!([]))]));
        settle_legacy(&mut plain, ["g".to_string()]);
        assert_eq!(plain.guilds["g"]["tools"], json!(["game_servers", "research", "lol", "images", "web_search"]));
    }

    /// 켜기·도구는 서버에만, 전체 허용 도구는 공통에만 둔다. 공통 보기의 바꿀 수 있는 키도 그렇다
    #[test]
    fn switches_belong_to_servers_and_the_allowed_list_to_the_common_menu() {
        let mut cfg = ChatbotConfig::default();
        assert!(apply(&mut cfg, patch(json!({"settings": {"enabled": true}}))).is_err());
        assert!(apply(&mut cfg, patch(json!({"settings": {"tools": ["lol"]}}))).is_err());
        assert!(apply(&mut cfg, patch(json!({"guilds": {"g1": {"allowedTools": ["lol"]}}}))).is_err());
        assert_eq!(cfg, ChatbotConfig::default(), "a refused patch changes nothing");
        // 지우기(null)는 받는다: 옛 공통 켜기를 손으로 지울 수 있게
        apply(&mut cfg, patch(json!({"settings": {"enabled": null, "allowedTools": ["lol"]}}))).unwrap();
        let defaults = view(&cfg)["defaults"].clone();
        assert!(defaults.get("enabled").is_none() && defaults.get("tools").is_none(), "{defaults}");
        assert_eq!(defaults["allowedTools"], Value::Null);
    }

    /// 디스코드 `/사바쨩 설정`은 서버 하나의 켜기·도구를 바꾸고, 공통 허용 목록을 함께 본다
    #[test]
    fn the_guild_view_shows_its_tools_and_the_allowed_list() {
        let mut cfg = ChatbotConfig::default();
        apply(&mut cfg, patch(json!({"settings": {"allowedTools": ["lol"]}, "guilds": {"g1": {"tools": ["lol", "pubg"]}}}))).unwrap();
        let v = guild_view(&cfg, "g1");
        assert_eq!(v["settings"]["tools"], json!(["lol", "pubg"]));
        assert_eq!(v["settings"]["allowedTools"], json!(["lol"]));
        assert_eq!(v["settings"]["enabled"], json!(false));
        assert!(v["settings"].get("disabledTools").is_none());
        assert_eq!(guild_view(&ChatbotConfig::default(), "g1")["settings"]["allowedTools"], Value::Null);
    }

    /// 서버 프롬프트 기본 틀: 요청 언어로, 안내는 주석이라 릴레이가 모델에게 보내지 않는다
    #[test]
    fn the_memo_template_comes_in_the_requested_language() {
        for (lang, heading) in [("ko", "## 호칭"), ("en", "## How to address people")] {
            let text = memo_template(lang);
            assert!(text.contains(heading), "{lang}: {text}");
            assert!(text.contains("<!--") && text.matches("<!--").count() == text.matches("-->").count());
        }
    }

    #[test]
    fn view_never_shows_the_key() {
        let cfg = ChatbotConfig { api_key: Some("sk-secret".into()), ..Default::default() };
        let text = Value::Object(view(&cfg)).to_string();
        assert!(!text.contains("sk-secret"));
        assert!(text.contains("\"hasApiKey\":true"));
    }

    /// 클라이언트가 키 이름을 따로 두지 않게, 바꿀 수 있는 키와 기본값을 함께 준다
    #[test]
    fn view_lists_every_key_with_its_default() {
        let v = view(&ChatbotConfig::default());
        assert_eq!(v["defaults"]["model"], relay_defaults()["model"]);
        assert_eq!(v["defaults"]["confirmLevel"], json!("all"));
    }
}
