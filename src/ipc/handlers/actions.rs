//! 데몬 액션 카탈로그 (P2) 라우트: 목록·계획·실행. 판정과 실행은 `crate::actions`
//!
//! HTTP로 오는 요청은 GUI·CLI(로컬)다. 출처는 `X-Saba-Client`, 언어는 `locale` 쿼리 → `gui.language` → en.

use super::super::IPCServer;
use crate::actions::{self, Caller, Surface};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    surface: Option<String>,
    instance: Option<String>,
    locale: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ActionBody {
    #[serde(default)]
    input: Value,
    instance: Option<String>,
    #[serde(rename = "planId")]
    plan_id: Option<String>,
    locale: Option<String>,
}

async fn locale(state: &IPCServer, requested: Option<String>) -> String {
    match requested {
        Some(l) if !l.trim().is_empty() => l,
        _ => state.config_store.get_gui_settings().await.language.unwrap_or_else(|| "en".into()),
    }
}

fn reply(result: Result<Value, actions::ActionError>) -> Response {
    match result {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST), Json(e.to_json())).into_response(),
    }
}

/// GET /api/actions?surface&instance&locale
pub async fn list_actions(State(state): State<IPCServer>, Query(q): Query<ListQuery>) -> Response {
    let surface = match q.surface.as_deref() {
        None | Some("") => None,
        Some(s) => match Surface::parse(s) {
            Some(s) => Some(s),
            None => return reply(Err(actions::ActionError { status: 400, code: "invalid_surface", message: format!("Unknown surface: {}", s), data: Value::Null })),
        },
    };
    let locale = locale(&state, q.locale).await;
    Json(actions::list(&state, surface, q.instance.as_deref(), &locale).await).into_response()
}

fn caller(headers: &HeaderMap, locale: String) -> Caller {
    Caller::local(crate::audit::Origin::from_headers(headers), locale)
}

/// POST /api/actions/:id/plan `{input, instance?, locale?}`
pub async fn plan_action(State(state): State<IPCServer>, Path(id): Path<String>, headers: HeaderMap, body: Option<Json<ActionBody>>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let caller = caller(&headers, locale(&state, body.locale).await);
    reply(actions::plan(&state, &id, body.instance.as_deref(), &body.input, &caller).await)
}

/// POST /api/actions/:id/run `{input, instance?}` 또는 `{planId}`
pub async fn run_action(State(state): State<IPCServer>, Path(id): Path<String>, headers: HeaderMap, body: Option<Json<ActionBody>>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let caller = caller(&headers, locale(&state, body.locale).await);
    reply(actions::run(&state, &id, body.instance.as_deref(), &body.input, body.plan_id.as_deref(), &caller).await)
}
