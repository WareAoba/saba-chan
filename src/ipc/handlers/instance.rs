use super::LooseJson;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

mod create;
mod delete;
mod provision;
mod settings;
mod update;

pub use create::{check_new_instance, create_instance};
pub use delete::{delete_instance, DeleteQuery};
pub use provision::{dismiss_provision_progress, get_provision_progress};
pub use settings::update_instance_settings;
pub use update::{apply_server_update, cancel_server_update, check_server_update};

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
            return super::error(StatusCode::BAD_REQUEST, "invalid_request", "Missing 'order' array");
        }
    };

    match supervisor.instance_store.reorder(&ordered_ids) {
        Ok(_) => {
            let response = json!({ "success": true });
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            super::error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", format!("Failed to reorder: {}", e))
        }
    }
}

/// GET /api/instance/:id - 특정 인스턴스 조회
pub async fn get_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let language = state.config_store.ui_language().await;
    let supervisor = state.supervisor.read().await;

    match supervisor.instance_store.get(&id) {
        Some(instance) => {
            // 모듈 약관과 동의 여부 (`agreements`)를 붙인다
            let mut body = serde_json::to_value(instance).unwrap_or_default();
            body["agreements"] = json!(supervisor.agreement_views(instance, &language));
            // 06 §8.2: 저장된 값 기준으로 지금 숨겨진 설정 (CLI가 쓴다)
            let fields = supervisor.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.settings.clone()).map(|s| s.fields).unwrap_or_default();
            // 설치된 버전에 없는 설정 (`versions` 밖). 숨긴 설정이므로 `hidden`에도 든다
            let version_hidden = crate::validator::version_hidden(&fields, instance.server_version.as_deref());
            let mut hidden = crate::validator::hidden_fields(&fields, |k| crate::instance::stored_setting(&body, instance, k));
            hidden.extend(version_hidden.iter().filter(|name| !hidden.contains(name)).cloned().collect::<Vec<_>>());
            body["hidden"] = json!(hidden);
            body["version_hidden"] = json!(version_hidden);
            body["stored"] = Value::Object(crate::instance::stored_settings(instance, &fields));
            body["revision"] = json!(instance.settings_revision());
            // 이 데몬이 메모리 상한을 거는가 (다른 PC의 데몬을 보는 클라이언트가 자기 OS로 짐작하지 않게)
            body["memory_limit_supported"] = json!(crate::instance::MEMORY_LIMIT_ENFORCED);
            // 이 OS에서는 걸리지 않는 상한이 저장돼 있다 (상한을 거절하기 전 릴리스가 저장한 값, `update_instance_settings`)
            if !crate::instance::MEMORY_LIMIT_ENFORCED && instance.memory_limit_mb.is_some() {
                body["memory_limit_unsupported"] = json!(true);
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        None => {
            super::error(StatusCode::NOT_FOUND, "instance_not_found", format!("Instance not found: {}", id))
        }
    }
}

/// POST /api/instance/:id/rename — 이름 바꾸기 (`{name}`). 실행 중이면 409 `server_running`,
/// 같은 이름이 있으면 409 `name_taken`, 쉼표가 있으면 400 `invalid_name`. 응답 `{success, id, name, previous}`
pub async fn rename_instance(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let fail = |status: StatusCode, error: String, code: &str| {
        super::error(status, code, error)
    };
    let Some(new_name) = body.get("name").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()) else {
        return fail(StatusCode::BAD_REQUEST, "Missing 'name'".into(), "invalid_request");
    };
    if new_name.contains(crate::chat::words::is_comma) {
        return crate::service::instance::invalid_name().into_response();
    }
    // 설치·업데이트·복원 중에는 바꾸지 않는다: 진행 알림(`provision.progress`)이 시작 때의 이름을 싣는다
    let Some(_guard) = crate::instance::lock::InstanceLock::take_for_writes(&id).await else {
        return super::backup::instance_busy();
    };
    let mut supervisor = state.supervisor.write().await;
    let Some(mut instance) = supervisor.instance_store.get(&id).cloned() else {
        return fail(StatusCode::NOT_FOUND, format!("Instance not found: {}", id), "not_found");
    };
    if instance.name == new_name {
        return Json(json!({ "success": true, "id": id, "name": new_name, "previous": new_name })).into_response();
    }
    if supervisor.instance_store.list().iter().any(|i| i.id != id && crate::chat::words::same(&i.name, new_name)) {
        return fail(StatusCode::CONFLICT, format!("An instance named '{}' already exists", new_name), "name_taken");
    }
    // 실행 중인 서버는 이름으로도 추적되므로 꺼진 뒤에만 바꾼다
    let running = supervisor.is_running(&id).await;
    if running {
        return fail(StatusCode::CONFLICT, "Stop the server before renaming it".into(), "server_running");
    }
    let previous = std::mem::replace(&mut instance.name, new_name.to_string());
    if let Err(e) = supervisor.instance_store.update(&id, instance) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal");
    }
    Json(json!({ "success": true, "id": id, "name": new_name, "previous": previous })).into_response()
}

/// POST /api/instance/:id/launch-preview — 실행 인자 미리보기 `{settings?, extra_args?}`.
/// 서버를 띄우지도 파일을 고치지도 않는다. 응답 모양은 `Supervisor::launch_preview`
pub async fn launch_preview(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    LooseJson(body): LooseJson<Value>,
) -> impl IntoResponse {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let draft = match body.get("settings") {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "error": "settings must be an object", "error_code": "invalid_body", "message": "settings must be an object" }))).into_response(),
    };
    // 보내지 않으면 저장된 추가 인자로 본다
    let extra = match body.get("extra_args").map(crate::service::instance::settings::extra_args_from) {
        None => None,
        Some(Some(args)) => Some(args),
        Some(None) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "error": "extra_args must be a string or an array of strings", "error_code": "invalid_format", "message": "extra_args must be a string or an array of strings" }))).into_response()
        }
    };
    match crate::supervisor::Supervisor::launch_preview(&state.supervisor, &id, &draft, extra.as_deref()).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) if e.downcast_ref::<crate::supervisor::InstanceNotFound>().is_some() => (StatusCode::NOT_FOUND, Json(crate::plugin::error_response(&e))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&e))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    /// 미리보기의 404는 인스턴스가 없을 때뿐이다. 인스턴스의 모듈이 없는 것("Module 'gone' not found")은 404가 아니다
    #[tokio::test]
    async fn launch_preview_is_not_found_only_for_a_missing_instance() {
        use axum::{extract::{Path, State}, response::IntoResponse};
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let mut inst = crate::instance::ServerInstance::new("orphan", "gone");
        inst.id = "orphan".into();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let preview = |id: &str| {
            let (state, id) = (state.clone(), id.to_string());
            async move { crate::ipc::internal::into_internal(super::launch_preview(State(state), Path(id), super::LooseJson(None)).await.into_response()).await }
        };
        let missing = preview("nope").await;
        assert_eq!((missing.status.as_u16(), missing.body["error"].as_str()), (404, Some("Instance 'nope' not found")), "{}", missing.body);
        let orphan = preview("orphan").await;
        assert_eq!(orphan.status.as_u16(), 500, "{}", orphan.body);
    }
}
