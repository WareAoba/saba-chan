//! 디스코드 길드 설정 (I2): 응답 언어·잠자기·음악 기능·직접 요청 되묻기 단계
//!
//! 원본은 데몬 config.json `relay.nodeSettings[<guild>].discord`다. 바꿀 때마다, 그리고 릴레이 에이전트가 연결될 때
//! 릴레이 `PATCH /api/nodes/:guild/settings`로 맞춘다. 봇 설정 저장(`PUT /api/config/bot`)은 이 블록을 바꾸지 않는다.
//!
//! - `GET /api/discord/:guild/settings` → `{guildId, locale, sleeping, music, confirmLevel}`
//! - `PATCH /api/discord/:guild/settings` 바꿀 키만 (`music` 안도 부분) → `{success, settings, sync}`.
//!   틀린 값은 400 `invalid_input {field, reason}`, 페어링이 없으면 409 `not_paired`.
//!   릴레이가 400으로 거절하면 저장하지 않는다. 닿지 않으면 저장하고 `sync.ok:false` (다음 연결 때 맞춘다)
//!
//! 업그레이드 뒤 처음 연결할 때 블록이 없는 길드는 릴레이 값을 한 번 가져와 원본으로 삼는다.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::chatbot::{call, host_guilds, relay_base, result_row};
use super::relay::{encode_path_segment, load_node_token};
use super::super::IPCServer;

/// `nodeSettings[guild]`에서 이 API만 바꾸는 키
pub const SETTINGS_KEY: &str = "discord";

/// 언어를 길드 언어 대신 데몬의 GUI 언어(heartbeat `language`)로 정한다
pub const AUTO_LOCALE: &str = "auto";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GuildSettings {
    /// `auto` 또는 지원 언어 코드 (`crate::chat::i18n::languages`)
    pub locale: String,
    pub sleeping: bool,
    pub music: Music,
    /// 봇에게 직접 요청한 명령을 버튼으로 되묻는 단계 (`CONFIRM_LEVELS`). 데몬만 보고 릴레이에는 보내지 않는다
    pub confirm_level: String,
}

/// 되묻기 단계: 모두 / 주의 이상 / 위험만 / 묻지 않음 (`crate::actions::level_asks`)
pub const CONFIRM_LEVELS: &[&str] = &["all", "caution", "danger", "none"];

/// 범위는 릴레이 `nodeSettingsSchema`와 같다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Music {
    pub enabled: bool,
    pub ui: bool,
    pub queue_lines: i64,
    pub refresh_interval: i64,
    pub normalize: bool,
}

impl Default for GuildSettings {
    fn default() -> Self {
        Self { locale: AUTO_LOCALE.into(), sleeping: false, music: Music::default(), confirm_level: "danger".into() }
    }
}

impl Default for Music {
    /// 릴레이 `DEFAULT_MUSIC_SETTINGS`와 같다
    fn default() -> Self {
        Self { enabled: true, ui: true, queue_lines: 5, refresh_interval: 4000, normalize: true }
    }
}

const QUEUE_LINES: (i64, i64) = (1, 20);
const REFRESH_INTERVAL: (i64, i64) = (2000, 10000);

impl GuildSettings {
    pub fn from_node(node: Option<&Value>) -> Option<Self> {
        serde_json::from_value(node?.get(SETTINGS_KEY)?.clone()).ok()
    }

    /// 릴레이 `PATCH /api/nodes/:guild/settings` 본문
    fn relay_body(&self) -> Value {
        json!({
            "locale": self.locale,
            "sleeping": self.sleeping,
            "musicEnabled": self.music.enabled,
            "musicUIEnabled": self.music.ui,
            "musicSettings": {
                "queueLines": self.music.queue_lines,
                "refreshInterval": self.music.refresh_interval,
                "normalize": self.music.normalize,
            },
        })
    }

    /// 릴레이 `GET /api/nodes/:guild/settings` 응답에서 (처음 한 번 가져오기). 모르는 값은 기본값
    fn from_relay(body: &Value) -> Self {
        let mut out = Self::default();
        let mut patch = Map::new();
        if let Some(l) = body.get("locale").and_then(Value::as_str) {
            // 릴레이는 `auto`나 언어 태그를 준다 (예전 릴레이는 태그와 `localeSource`)
            let auto = body.get("localeSource").and_then(Value::as_str) == Some("auto");
            patch.insert("locale".into(), json!(if auto { AUTO_LOCALE } else { l }));
        }
        if let Some(s) = body.get("sleeping").or_else(|| body.get("isSleep")) {
            patch.insert("sleeping".into(), s.clone());
        }
        let mut music = Map::new();
        for (from, to) in [("musicEnabled", "enabled"), ("musicUIEnabled", "ui")] {
            if let Some(v) = body.get(from) {
                music.insert(to.into(), v.clone());
            }
        }
        if let Some(m) = body.get("musicSettings").and_then(Value::as_object) {
            for key in ["queueLines", "refreshInterval", "normalize"] {
                if let Some(v) = m.get(key) {
                    music.insert(key.into(), v.clone());
                }
            }
        }
        patch.insert("music".into(), Value::Object(music));
        // 하나씩 넣어 틀린 값 하나 때문에 나머지를 잃지 않는다
        for (k, v) in patch {
            match (k.as_str(), v) {
                ("music", Value::Object(m)) => {
                    for (mk, mv) in m {
                        let one = json!({ "music": { mk: mv } });
                        let _ = out.apply(one.as_object().expect("object"));
                    }
                }
                (k, v) => {
                    let _ = out.apply(&Map::from_iter([(k.to_string(), v)]));
                }
            }
        }
        out
    }

    /// 바꿀 키만 적용한다. 틀리면 (필드, 이유)를 돌려주고 아무것도 바꾸지 않는다
    pub fn apply(&mut self, patch: &Map<String, Value>) -> Result<(), (String, &'static str)> {
        let mut next = self.clone();
        for (key, v) in patch {
            match key.as_str() {
                "locale" => {
                    let l = v.as_str().ok_or(("locale".into(), "must be a string"))?;
                    if l != AUTO_LOCALE && !crate::chat::i18n::languages().contains(&l) {
                        return Err(("locale".into(), "unsupported language"));
                    }
                    next.locale = l.to_string();
                }
                "sleeping" => next.sleeping = v.as_bool().ok_or(("sleeping".into(), "must be a boolean"))?,
                "confirmLevel" => match v.as_str() {
                    Some(l) if CONFIRM_LEVELS.contains(&l) => next.confirm_level = l.to_string(),
                    _ => return Err(("confirmLevel".into(), "unknown level")),
                },
                "music" => {
                    let m = v.as_object().ok_or(("music".into(), "must be an object"))?;
                    for (mk, mv) in m {
                        let field = format!("music.{}", mk);
                        let flag = || mv.as_bool().ok_or((field.clone(), "must be a boolean"));
                        let ranged = |(lo, hi): (i64, i64)| match mv.as_i64() {
                            Some(n) if (lo..=hi).contains(&n) => Ok(n),
                            Some(_) => Err((field.clone(), "out of range")),
                            None => Err((field.clone(), "must be an integer")),
                        };
                        match mk.as_str() {
                            "enabled" => next.music.enabled = flag()?,
                            "ui" => next.music.ui = flag()?,
                            "normalize" => next.music.normalize = flag()?,
                            "queueLines" => next.music.queue_lines = ranged(QUEUE_LINES)?,
                            "refreshInterval" => next.music.refresh_interval = ranged(REFRESH_INTERVAL)?,
                            _ => return Err((field, "unknown field")),
                        }
                    }
                }
                _ => return Err((key.clone(), "unknown field")),
            }
        }
        *self = next;
        Ok(())
    }
}

fn view(guild: &str, s: &GuildSettings) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or_default();
    v["guildId"] = json!(guild);
    v
}

fn invalid(field: &str, reason: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "success": false, "error": format!("Invalid input '{}': {}", field, reason), "error_code": "invalid_input", "field": field, "reason": reason })),
    )
        .into_response()
}

/// 읽고-바꾸고-저장하기를 한 번에 하나씩 (동시에 온 두 PATCH가 서로의 키를 지우지 않게)
static SETTINGS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 릴레이에 닿지 않아 보내지 못한 설정이 있다. 하트비트가 다시 성공하면 `sync_on_connect`로 다시 보낸다
static PENDING_SYNC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn take_pending_sync() -> bool {
    PENDING_SYNC.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// 연결 실패나 릴레이 쪽 오류(5xx)면 다시 보낼 만하다. 거절(4xx)은 다시 보내도 같다
fn retry_later(outcome: &Result<(StatusCode, Value), String>) -> bool {
    match outcome {
        Ok((c, _)) => c.is_server_error(),
        Err(_) => true,
    }
}

/// 데몬에 저장된 값. 아직 없으면(업그레이드 뒤 첫 연결 전) 릴레이 값을, 릴레이에 노드 설정이 없으면 기본값.
/// 페어링했는데 릴레이를 읽지 못하면 `None`: 기본값으로 착각해 저장하면 다음 동기화가 릴레이 값을 덮어쓴다
async fn current(state: &IPCServer, guild: &str) -> Option<GuildSettings> {
    let bot = state.config_store.get_bot_config().await;
    if let Some(s) = GuildSettings::from_node(bot.node_settings.get(guild)) {
        return Some(s);
    }
    match relay_settings(&relay_base(state).await, guild).await {
        RelaySettings::Found(body) => Some(GuildSettings::from_relay(&body)),
        RelaySettings::Absent => Some(GuildSettings::default()),
        RelaySettings::Unreachable => None,
    }
}

enum RelaySettings {
    Found(Value),
    /// 페어링이 없거나 릴레이에 이 길드 설정이 없다 (404)
    Absent,
    /// 연결 실패나 그 밖의 오류
    Unreachable,
}

/// 릴레이 `GET /api/nodes/:guild/settings`
async fn relay_settings(base: &str, guild: &str) -> RelaySettings {
    if load_node_token().is_none() {
        return RelaySettings::Absent;
    }
    let path = format!("/api/nodes/{}/settings", encode_path_segment(guild));
    match call(base, reqwest::Method::GET, &path, None).await {
        Ok((c, body)) if c.is_success() => RelaySettings::Found(body),
        Ok((StatusCode::NOT_FOUND, _)) => RelaySettings::Absent,
        _ => RelaySettings::Unreachable,
    }
}

async fn save(state: &IPCServer, guild: &str, s: &GuildSettings) -> Result<(), crate::config_store::ConfigStoreError> {
    let value = serde_json::to_value(s).unwrap_or_default();
    state
        .config_store
        .update_node_settings(guild, |node| {
            let changed = node.get(SETTINGS_KEY) != Some(&value);
            node.insert(SETTINGS_KEY.into(), value.clone());
            ((), changed)
        })
        .await
}

/// `want`에서 `have`와 다른 키만 (`musicSettings`처럼 객체인 값은 그 안의 키 단위로. 릴레이가 키별로 합친다)
fn changed(have: &Value, want: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in want.as_object().into_iter().flatten() {
        match (have.get(k), v) {
            (Some(Value::Object(h)), Value::Object(w)) => {
                let diff: Map<String, Value> = w.iter().filter(|(sk, sv)| h.get(*sk) != Some(*sv)).map(|(sk, sv)| (sk.clone(), sv.clone())).collect();
                if !diff.is_empty() {
                    out.insert(k.clone(), Value::Object(diff));
                }
            }
            (h, v) if h != Some(v) => {
                out.insert(k.clone(), v.clone());
            }
            _ => {}
        }
    }
    out
}

/// 릴레이에 지금 있는 값과 다른 키만 보낸다. 같으면 보내지 않는다 (음악 플레이어가 바뀔 때마다 다시 그려진다).
/// 릴레이 값을 못 읽으면 `before`(데몬이 마지막으로 맞춘 값)와 비교한다
async fn push(state: &IPCServer, base: &str, guild: &str, before: &Value, s: &GuildSettings) -> Result<(StatusCode, Value), String> {
    let path = format!("/api/nodes/{}/settings", encode_path_segment(guild));
    let have = match relay_settings(base, guild).await {
        RelaySettings::Found(body) => body,
        _ => before.clone(),
    };
    let mut diff = changed(&have, &s.relay_body());
    if diff.is_empty() {
        return Ok((StatusCode::OK, Value::Null));
    }
    // 자동 언어로 바꿀 때는 따를 언어도 지금 값으로 (heartbeat `language`는 다음 주기에야 간다)
    if diff.get("locale").and_then(Value::as_str) == Some(AUTO_LOCALE) {
        if let Some(language) = Some(state.config_store.pc_language().await).filter(|l| l.len() <= 10) {
            diff.insert("language".into(), Value::String(language));
        }
    }
    call(base, reqwest::Method::PATCH, &path, Some(Value::Object(diff).to_string())).await
}

/// GET /api/discord/:guild/settings
/// 처음 보는 길드인데 릴레이를 읽지 못하면 기본값에 `loaded: false`를 붙인다 (그 값으로 저장하지 않는다)
pub async fn get_settings(State(state): State<IPCServer>, Path(guild): Path<String>) -> Response {
    match current(&state, &guild).await {
        Some(s) => Json(view(&guild, &s)).into_response(),
        None => {
            let mut v = view(&guild, &GuildSettings::default());
            v["loaded"] = json!(false);
            Json(v).into_response()
        }
    }
}

/// PATCH /api/discord/:guild/settings
pub async fn patch_settings(State(state): State<IPCServer>, Path(guild): Path<String>, Json(body): Json<Value>) -> Response {
    let Some(patch) = body.as_object() else { return invalid("input", "must be an object") };
    if load_node_token().is_none() {
        return (StatusCode::CONFLICT, Json(json!({ "success": false, "error": "Not paired with the relay", "error_code": "not_paired" }))).into_response();
    }
    let _turn = SETTINGS_LOCK.lock().await;
    let Some(before) = current(&state, &guild).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "success": false, "error": "Could not read this server's settings from the relay", "error_code": "relay_unavailable" })),
        )
            .into_response();
    };
    let mut next = before.clone();
    if let Err((field, reason)) = next.apply(patch) {
        return invalid(&field, reason);
    }
    let base = relay_base(&state).await;
    let outcome = push(&state, &base, &guild, &before.relay_body(), &next).await;
    // 릴레이가 값을 거절했으면 저장하지 않는다 (연결 실패는 저장하고 다음 연결 때 맞춘다)
    if matches!(&outcome, Ok((c, _)) if *c == StatusCode::BAD_REQUEST) {
        let mut row = result_row(&guild, &outcome);
        row["success"] = json!(false);
        row["error_code"] = json!("invalid_input");
        return (StatusCode::BAD_REQUEST, Json(row)).into_response();
    }
    if let Err(e) = save(&state, &guild, &next).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string(), "error_code": "save_failed" }))).into_response();
    }
    if retry_later(&outcome) {
        PENDING_SYNC.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Json(json!({ "success": true, "settings": view(&guild, &next), "sync": result_row(&guild, &outcome) })).into_response()
}

/// 릴레이 에이전트가 연결된 직후 한 번: 원본이 있는 길드는 보내고, 없는 길드는 릴레이 값을 한 번 가져온다
pub async fn sync_on_connect(state: &IPCServer) {
    if load_node_token().is_none() {
        return;
    }
    let _turn = SETTINGS_LOCK.lock().await;
    let base = relay_base(state).await;
    let bot = state.config_store.get_bot_config().await;
    let mut guilds: Vec<String> = bot.node_settings.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for g in host_guilds(state, &base).await.unwrap_or_default() {
        if !guilds.contains(&g) {
            guilds.push(g);
        }
    }
    for guild in guilds {
        match GuildSettings::from_node(bot.node_settings.get(&guild)) {
            Some(s) => {
                let outcome = push(state, &base, &guild, &Value::Null, &s).await;
                if !matches!(&outcome, Ok((c, _)) if c.is_success()) {
                    tracing::warn!("[Discord] settings sync to guild {} failed: {}", guild, result_row(&guild, &outcome));
                }
                if retry_later(&outcome) {
                    PENDING_SYNC.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            None => {
                let path = format!("/api/nodes/{}/settings", encode_path_segment(&guild));
                match call(&base, reqwest::Method::GET, &path, None).await {
                    Ok((c, body)) if c.is_success() => {
                        if let Err(e) = save(state, &guild, &GuildSettings::from_relay(&body)).await {
                            tracing::warn!("[Discord] could not keep the relay settings of guild {}: {}", guild, e);
                        }
                    }
                    // 릴레이에 노드가 없거나 닿지 않으면 다음 연결 때 다시
                    other => tracing::debug!("[Discord] no relay settings for guild {}: {}", guild, result_row(&guild, &other)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_patch_changes_only_the_keys_it_names() {
        let mut s = GuildSettings::default();
        s.apply(&patch(json!({ "locale": "ko", "music": { "queueLines": 12 } }))).unwrap();
        assert_eq!(s.locale, "ko");
        assert_eq!(s.music.queue_lines, 12);
        assert_eq!(s.music.refresh_interval, Music::default().refresh_interval);
        assert!(!s.sleeping);
        s.apply(&patch(json!({ "sleeping": true, "locale": "auto" }))).unwrap();
        assert!(s.sleeping);
        assert_eq!(s.locale, AUTO_LOCALE);
    }

    #[test]
    fn a_wrong_value_changes_nothing_and_names_its_field() {
        let mut s = GuildSettings::default();
        for (body, field) in [
            (json!({ "locale": "xx" }), "locale"),
            (json!({ "sleeping": "yes" }), "sleeping"),
            (json!({ "music": { "queueLines": 21 } }), "music.queueLines"),
            (json!({ "music": { "refreshInterval": 1999 } }), "music.refreshInterval"),
            (json!({ "music": { "volume": 3 } }), "music.volume"),
            (json!({ "prefix": "!" }), "prefix"),
            (json!({ "locale": "ja", "music": { "ui": 1 } }), "music.ui"),
        ] {
            let err = s.apply(&patch(body.clone())).unwrap_err();
            assert_eq!(err.0, field, "{body}");
            assert_eq!(s, GuildSettings::default(), "{body}");
        }
    }

    #[test]
    fn relay_values_are_kept_once_when_the_daemon_has_none() {
        let s = GuildSettings::from_relay(&json!({
            "guildId": "1", "locale": "ja", "localeSource": "manual",
            "musicEnabled": false, "musicUIEnabled": true,
            "musicSettings": { "queueLines": 99, "refreshInterval": 3000, "normalize": false },
        }));
        assert_eq!(s.locale, "ja");
        assert!(!s.music.enabled);
        // 범위 밖 값은 버리고 나머지는 남긴다
        assert_eq!(s.music.queue_lines, Music::default().queue_lines);
        assert_eq!(s.music.refresh_interval, 3000);
        assert!(!s.music.normalize);
        // 자동으로 정해진 언어는 auto로
        assert_eq!(GuildSettings::from_relay(&json!({ "locale": "ko", "localeSource": "auto" })).locale, AUTO_LOCALE);
        assert_eq!(GuildSettings::from_relay(&json!({ "locale": "auto" })).locale, AUTO_LOCALE);
        assert_eq!(GuildSettings::from_relay(&json!({ "locale": "ko", "sleeping": true })).locale, "ko");
        assert!(GuildSettings::from_relay(&json!({ "sleeping": true })).sleeping);
        assert_eq!(GuildSettings::from_relay(&json!({})), GuildSettings::default());
    }

    #[test]
    fn only_keys_that_differ_from_the_relay_are_sent() {
        let mut s = GuildSettings::default();
        let relay = s.relay_body();
        assert!(changed(&relay, &s.relay_body()).is_empty());
        s.apply(&patch(json!({ "music": { "queueLines": 9 } }))).unwrap();
        assert_eq!(Value::Object(changed(&relay, &s.relay_body())), json!({ "musicSettings": { "queueLines": 9 } }));
        // 릴레이에 없는 키(예전 릴레이)는 보낸다
        let old_relay = json!({ "musicEnabled": true, "musicUIEnabled": true, "musicSettings": { "queueLines": 9, "refreshInterval": 4000, "normalize": true } });
        assert_eq!(Value::Object(changed(&old_relay, &s.relay_body())), json!({ "locale": "auto", "sleeping": false }));
    }

    #[test]
    fn the_relay_gets_its_own_field_names() {
        let body = GuildSettings::default().relay_body();
        assert_eq!(body["musicUIEnabled"], json!(true));
        assert_eq!(body["musicSettings"]["queueLines"], json!(5));
        assert_eq!(body["locale"], json!("auto"));
        assert_eq!(body["sleeping"], json!(false));
    }
}
