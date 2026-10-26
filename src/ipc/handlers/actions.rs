//! 데몬 액션 카탈로그 (P2) 라우트: 목록·계획·실행. 판정과 실행은 `crate::actions`
//!
//! HTTP로 오는 요청은 GUI·CLI(로컬)다. 출처는 `X-Saba-Client`, 언어는 `locale` 쿼리 → `Accept-Language` → `gui.language` → en.

use super::super::IPCServer;
use crate::actions::{self, Caller, Surface};
use super::LooseJson;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
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
        _ => state.config_store.ui_language().await,
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

/// 출처는 감사 미들웨어가 정한 것을 쓴다 (원격 포트에서는 인증층이 정한 짝 맺은 PC)
fn caller(locale: String) -> Caller {
    Caller::local(crate::audit::current(), locale)
}

/// POST /api/actions/:id/plan `{input, instance?, locale?}`
pub async fn plan_action(State(state): State<IPCServer>, Path(id): Path<String>, LooseJson(body): LooseJson<ActionBody>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let caller = caller(locale(&state, body.locale).await);
    reply(actions::plan(&state, &id, body.instance.as_deref(), &body.input, &caller).await)
}

/// POST /api/actions/:id/run `{input, instance?}` 또는 `{planId}`
pub async fn run_action(State(state): State<IPCServer>, Path(id): Path<String>, LooseJson(body): LooseJson<ActionBody>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let caller = caller(locale(&state, body.locale).await);
    reply(actions::run(&state, &id, body.instance.as_deref(), &body.input, body.plan_id.as_deref(), &caller).await)
}

#[derive(Debug, Deserialize, Default)]
pub struct StepsBody {
    #[serde(default)]
    steps: Value,
    locale: Option<String>,
}

/// POST /api/actions/plan `{steps:[{id, instance?, input}], locale?}`: 여러 단계 계획 (실행은 `/api/actions/plan/run {planId}`)
pub async fn plan_steps(State(state): State<IPCServer>, LooseJson(body): LooseJson<StepsBody>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let caller = caller(locale(&state, body.locale).await);
    reply(actions::plan_steps(&state, &body.steps, &caller).await)
}

/// GET /api/actions/runs/:runId: 여러 단계 실행의 진행
pub async fn run_status(State(state): State<IPCServer>, Path(run_id): Path<String>) -> Response {
    let caller = caller(locale(&state, None).await);
    reply(actions::run_status(&run_id, &caller))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 원격 포트 인증층이 정한 출처(짝 맺은 PC)가 액션 실행에도 그대로 남는다
    #[tokio::test]
    async fn an_action_caller_keeps_the_origin_set_before_the_handler() {
        let origin = crate::audit::Origin { user_name: Some("집 PC".into()), ..crate::audit::Origin::new("remote") };
        let caller = crate::audit::with_origin(origin, async { caller("ko".into()) }).await;
        assert_eq!((caller.origin.source.as_str(), caller.origin.user_name.as_deref()), ("remote", Some("집 PC")));
    }
}
