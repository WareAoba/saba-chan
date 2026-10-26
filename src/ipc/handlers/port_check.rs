//! 포트포워딩 확인 (`crate::port_check`)
//!
//!   POST /api/instance/:id/port-check   → 확인 결과 (`port_check::Report`). 릴레이에 연결하지 않았으면 409 `node_token_missing`
//!   GET  /api/instance/:id/port-check   → 마지막 결과, 확인한 적 없으면 `{instanceId, verdict:"not_checked"}`
//!
//! `:id`는 인스턴스 id, 없으면 이름으로 찾는다 (CLI·채팅처럼 이름만 아는 쪽)

use axum::{extract::Path, extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use super::super::IPCServer;
use crate::port_check::CheckError;

async fn resolve(state: &IPCServer, id: &str) -> Option<String> {
    let sup = state.supervisor.read().await;
    sup.instance_store.get(id).or_else(|| sup.instance_store.list().iter().find(|i| i.name == id)).map(|i| i.id.clone())
}

fn not_found(id: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response()
}

/// POST /api/instance/:id/port-check
pub async fn check(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    let Some(id) = resolve(&state, &id).await else { return not_found(&id) };
    match crate::port_check::check(&state, std::slice::from_ref(&id)).await {
        Ok(mut reports) => match reports.pop() {
            Some(report) => Json(report).into_response(),
            None => not_found(&id),
        },
        Err(e) => {
            let status = match e {
                CheckError::NotPaired | CheckError::Busy => StatusCode::CONFLICT,
                CheckError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
                CheckError::Relay(_) => StatusCode::BAD_GATEWAY,
            };
            if let CheckError::Relay(detail) = &e {
                tracing::warn!("Port check for {} failed: {}", id, detail);
            }
            (status, Json(json!({ "success": false, "error": e.code(), "error_code": e.code() }))).into_response()
        }
    }
}

/// GET /api/instance/:id/port-check
pub async fn last(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    let Some(id) = resolve(&state, &id).await else { return not_found(&id) };
    match crate::port_check::last(&id) {
        Some(report) => Json(report).into_response(),
        None => Json(json!({ "instanceId": id, "verdict": "not_checked" })).into_response(),
    }
}
