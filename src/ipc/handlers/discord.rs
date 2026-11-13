//! 디스코드 길드 설정 (I2): 응답 언어·잠자기(코코낸내: 끌 기능, 자동 시각)·음악 기능·직접 요청 되묻기 단계·접두사·명령 번역·슬래시 모양
//!
//! 원본은 데몬 config.json `relay.nodeSettings[<guild>].discord`다. 바꿀 때마다, 그리고 릴레이 에이전트가 연결될 때
//! 릴레이 `PATCH /api/nodes/:guild/settings`로 맞춘다. 봇 설정 저장(`PUT /api/config/bot`)은 이 블록을 바꾸지 않는다.
//!
//! - `GET /api/discord/:guild/settings` → `{guildId, locale, sleeping, music, tts, confirmLevel, commandRisks, prefixes, translateCommands,
//!   slashPrefix, slashRootName, privateQueries, sleepBlocks, bedtime}` (슬래시 모양 세 값은 2026-10-30, 코코낸내 두 값은 2026-11-03)
//! - `PATCH /api/discord/:guild/settings` 바꿀 키만 (`music`·`tts` 안도 부분) → `{success, settings, sync}`.
//!   틀린 값은 400 `invalid_input {field, reason}`, 페어링이 없으면 409 `not_paired`.
//!   릴레이가 400으로 거절하면 저장하지 않는다. 닿지 않으면 저장하고 `sync.ok:false` (다음 연결 때 맞춘다)
//!
//! 업그레이드 뒤 처음 연결할 때 블록이 없는 길드는 릴레이 값을 한 번 가져와 원본으로 삼는다.
//! 접두사·명령 번역은 0.1.12 전에는 호스트 하나의 봇 설정(`prefixes`, `translateCommands`)이었다: 서버 값이 없으면 그것을 물려받는다.

use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use serde_json::Value;

use super::super::IPCServer;
use crate::service::discord as service;

/// GET /api/discord/:guild/settings
/// 처음 보는 길드인데 릴레이를 읽지 못하면 기본값에 `loaded: false`를 붙인다 (그 값으로 저장하지 않는다)
pub async fn get_settings(State(state): State<IPCServer>, Path(guild): Path<String>) -> Json<Value> {
    Json(service::get(&state, &guild).await)
}

/// PATCH /api/discord/:guild/settings
pub async fn patch_settings(State(state): State<IPCServer>, Path(guild): Path<String>, Json(body): Json<Value>) -> Response {
    super::answer(service::patch(&state, &guild, &body).await)
}
