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
//! 공통(`settings`)에는 모델만 둔다. 켜기(`enabled`)·켠 도구(`tools`)·답변 깊이·끼어들기·기억·되묻기·메모는 서버마다 둔다
//! (기본: 꺼짐, 도구 없음). 옛 설정(공통 `enabled`·`effort` 같은 서버 값, 전체 허용 도구 `allowedTools`, `disabledTools`,
//! `webSearch`)은 길드 목록을 받았을 때 서버마다 적어 넣는다 (`settle_legacy`). 그 전까지는 옛 값으로 셈해 보낸다
//! - `PATCH /api/chatbot/common` `{model?, apiKey?: string|null}` — 내 PC 공통 값만 (액션 `chatbot.common.set`)
//! - `DELETE /api/chatbot/memory?guildId=` — 장기 기억 지우기 (guildId가 없으면 모든 길드)
//! - `GET /api/chatbot/memo-template?locale=` — 서버 프롬프트 기본 틀 `{template, locale}` (daemon.json `chatbot.memo_template`)

use axum::{
    extract::{Path, Query, State},
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::super::IPCServer;
use crate::service::chatbot as service;

pub async fn get_chatbot(State(state): State<IPCServer>) -> Json<Value> {
    Json(service::get(&state))
}

pub async fn put_chatbot(State(state): State<IPCServer>, Json(patch): Json<service::ChatbotPatch>) -> Response {
    super::answer(service::put(&state, patch).await)
}

pub async fn delete_chatbot(State(state): State<IPCServer>) -> Response {
    super::answer(service::delete(&state).await)
}

/// `GET /api/chatbot/guilds/:guild`
pub async fn get_guild(State(state): State<IPCServer>, Path(guild): Path<String>) -> Json<Value> {
    Json(service::guild(&state, &guild))
}

/// `PATCH /api/chatbot/guilds/:guild` `{설정 키: 값|null}`: 그 길드 값만 합친다. 키(`apiKey`)와 모르는 키는 받지 않는다
pub async fn patch_guild(State(state): State<IPCServer>, Path(guild): Path<String>, Json(patch): Json<Map<String, Value>>) -> Response {
    super::answer(service::patch_guild(&state, &guild, patch).await)
}

/// `PATCH /api/chatbot/common` `{model?, apiKey?: string|null}` (액션 `chatbot.common.set`, 디스코드 /설정 "사바쨩 전체")
pub async fn patch_common(State(state): State<IPCServer>, Json(patch): Json<Map<String, Value>>) -> Response {
    super::answer(service::patch_common(&state, patch).await)
}

#[derive(Deserialize)]
pub struct TemplateQuery {
    #[serde(default)]
    locale: Option<String>,
}

/// `GET /api/chatbot/memo-template?locale=` (없으면 앱 언어)
pub async fn get_memo_template(State(state): State<IPCServer>, Query(q): Query<TemplateQuery>) -> Json<Value> {
    Json(service::memo_template_in(&state, q.locale).await)
}

#[derive(Deserialize)]
pub struct MemoryQuery {

    #[serde(default, rename = "guildId")]
    guild_id: Option<String>,
}

pub async fn delete_memory(State(state): State<IPCServer>, Query(q): Query<MemoryQuery>) -> Response {
    super::answer(service::delete_memory(&state, q.guild_id).await)
}
