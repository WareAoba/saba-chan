use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::ipc::IPCServer;

/// GET /api/instances - 모든 인스턴스 목록
pub async fn list_instances(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    let instances = supervisor.instance_store.list();
    (StatusCode::OK, Json(instances)).into_response()
}

/// PUT /api/instances/reorder - 인스턴스 순서 변경
pub async fn reorder_instances(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    let ordered_ids: Vec<String> = match payload.get("order").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => {
            let error = json!({ "error": "Missing 'order' array" });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };

    match supervisor.instance_store.reorder(&ordered_ids) {
        Ok(_) => {
            let response = json!({ "success": true });
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to reorder: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// GET /api/instance/:id - 특정 인스턴스 조회
pub async fn get_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.instance_store.get(&id) {
        Some(instance) => (StatusCode::OK, Json(instance)).into_response(),
        None => {
            let error = json!({
                "error": format!("Instance not found: {}", id),
                "error_code": "instance_not_found",
            });
            (StatusCode::NOT_FOUND, Json(error)).into_response()
        }
    }
}

/// DELETE /api/instance/:id - 인스턴스 삭제
pub async fn delete_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    // ── 인스턴스 존재 확인 ──
    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst.clone(),
        None => {
            let error = json!({ "success": false, "error": "Instance not found" });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    // ── 실행 중이면 먼저 정지 ──
    let is_running = supervisor.tracker.get_pid(&id).is_ok();
    if is_running {
        tracing::info!("Instance '{}' is running, force-stopping before delete...", instance.name);

        // Managed process가 있으면 force kill
        if let Some(managed) = supervisor.managed_store.remove(&id).await {
            if managed.is_running() {
                if let Ok(pid) = supervisor.tracker.get_pid(&id) {
                    let _ = crate::supervisor::process::force_kill_pid(pid);
                }
                // 프로세스 종료 대기
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        } else {
            // Managed가 아니면 PID로 직접 kill
            if let Ok(pid) = supervisor.tracker.get_pid(&id) {
                let _ = crate::supervisor::process::force_kill_pid(pid);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        let _ = supervisor.tracker.untrack(&id);
    }

    // ── Extension hook: server.pre_delete ──
    {
        let ext_mgr = state.extension_manager.clone();
        let ctx = serde_json::json!({
            "instance_id": &id,
            "instance_name": &instance.name,
            "module": &instance.module_name,
            "extension_data": &instance.extension_data,
        });
        let mgr = ext_mgr.read().await;
        let results = mgr.dispatch_hook("server.pre_delete", ctx).await;
        let handled = results.iter().any(|(_id, r)| {
            r.as_ref()
                .map(|v| v.get("handled").and_then(|h| h.as_bool()) == Some(true))
                .unwrap_or(false)
        });
        if handled {
            tracing::info!("Extension handled pre_delete cleanup for '{}'", instance.name);
            // Docker compose down 후 파일 잠금 해제 대기
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    // ── 디렉토리 삭제 (Windows 파일 잠금 대비 재시도) ──
    let mut last_err = None;
    for attempt in 0..3 {
        match supervisor.instance_store.remove(&id) {
            Ok(_) => {
                let response = json!({ "success": true });
                return (StatusCode::OK, Json(response)).into_response();
            }
            Err(e) => {
                tracing::warn!(
                    "Instance '{}' delete attempt {}/3 failed: {}",
                    instance.name, attempt + 1, e
                );
                last_err = Some(e);
                if attempt < 2 {
                    // 재시도 전 대기 — 파일 잠금 해제 기다림
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }

    let err_msg = last_err.map(|e| format!("{}", e)).unwrap_or_else(|| "Unknown error".to_string());
    let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", err_msg) });
    (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
}

