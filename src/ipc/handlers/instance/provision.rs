use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::ipc::IPCServer;

/// DELETE /api/provision-progress/:name - 프로비저닝 상태 클리어 (에러 dismiss용)
/// 프로비저닝 실패 상태였으면 인스턴스도 자동 롤백(삭제)
pub async fn dismiss_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 프로비저닝 에러가 있었는지 확인
    let had_error = state
        .provision_tracker
        .get(&name)
        .map(|p| p.error.is_some())
        .unwrap_or(false);

    state.provision_tracker.remove(&name);

    // 프로비저닝 실패 시: 자동 롤백 — 인스턴스 삭제
    if had_error {
        let mut supervisor = state.supervisor.write().await;
        if let Some(id) = supervisor
            .instance_store
            .list()
            .iter()
            .find(|i| i.name == name)
            .map(|i| i.id.clone())
        {
            if let Err(e) = supervisor.instance_store.remove(&id) {
                tracing::warn!("Failed to rollback instance '{}': {}", name, e);
            } else {
                tracing::info!(
                    "Provisioning rollback: removed instance '{}' (id={})",
                    name,
                    id
                );
            }
        }
    }

    (StatusCode::OK, Json(json!({ "success": true, "rolled_back": had_error }))).into_response()
}

/// GET /api/provision-progress/:name - 프로비저닝 진행 상태 (서버 이름 기준)
pub async fn get_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if let Some(progress) = state.provision_tracker.get(&name) {
        let mut resp = json!({
            "active": true,
            "step": progress.step,
            "total": progress.total,
            "label": progress.label,
            "message": progress.message,
            "done": progress.done,
            "error": progress.error,
        });
        if let Some(pct) = progress.percent {
            resp["percent"] = json!(pct);
        }
        if let Some(ref steps) = progress.steps {
            resp["steps"] = json!(steps);
        }
        (StatusCode::OK, Json(resp)).into_response()
    } else {
        (StatusCode::OK, Json(json!({
            "active": false,
        }))).into_response()
    }
}

