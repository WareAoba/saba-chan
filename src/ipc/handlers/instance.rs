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

pub(crate) use create::extension_required;
pub use create::{check_new_instance, create_instance};
pub use delete::{delete_instance, DeleteQuery};
pub use provision::{dismiss_provision_progress, get_provision_progress};
use settings::extra_args_from;
pub(crate) use settings::settings_rejection;
pub use settings::update_instance_settings;
pub(crate) use update::{begin_server_update_holding, emit_update_failed, updatable, update_blocked_by_shared_folder};
pub use update::{apply_server_update, check_server_update, server_update_status};
#[cfg(test)]
pub(crate) use update::begin_server_update;

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
            let mut hidden = crate::validator::hidden_fields(&fields, |k| stored_setting(&body, instance, k));
            hidden.extend(version_hidden.iter().filter(|name| !hidden.contains(name)).cloned().collect::<Vec<_>>());
            body["hidden"] = json!(hidden);
            body["version_hidden"] = json!(version_hidden);
            body["stored"] = Value::Object(stored_settings(instance, &fields));
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
            let error = json!({
                "error": format!("Instance not found: {}", id),
                "error_code": "instance_not_found",
            });
            (StatusCode::NOT_FOUND, Json(error)).into_response()
        }
    }
}

/// 사용자가 정한 `java_path`를 그 모듈의 격리 실행에 연다 (폴더 권한 고치기는 오래 걸릴 수 있어 따로)
fn open_user_runtime(entry: String, instance: &crate::instance::ServerInstance) {
    let Some(java) = instance.module_settings.get("java_path").and_then(Value::as_str).filter(|_| instance.user_settings.contains("java_path")) else { return };
    let java = java.to_string();
    tokio::task::spawn_blocking(move || crate::sandbox::open_user_runtime(std::path::Path::new(&entry), &java));
}

/// 요청 값 하나가 잘못됐다는 400 응답: `error`에 이유
pub(crate) fn invalid_field(field: &str, error: impl Into<String>) -> axum::response::Response {
    let error = json!({ "error": error.into(), "error_code": "invalid_format", "fields": [{"field": field}] });
    (StatusCode::BAD_REQUEST, Json(error)).into_response()
}

/// 쉼표가 든 서버 이름 (만들기·이름 바꾸기 공통 400 `invalid_name`)
fn invalid_name() -> (StatusCode, Json<Value>) {
    let error = json!({ "success": false, "error": "A server name cannot contain a comma", "error_code": "invalid_name", "fields": [{ "field": "name" }] });
    (StatusCode::BAD_REQUEST, Json(error))
}

/// POST /api/instance/:id/rename — 이름 바꾸기 (`{name}`). 실행 중이면 409 `server_running`,
/// 같은 이름이 있으면 409 `name_taken`, 쉼표가 있으면 400 `invalid_name`. 응답 `{success, id, name, previous}`
pub async fn rename_instance(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let fail = |status: StatusCode, error: String, code: &str| {
        (status, Json(json!({ "success": false, "error": error, "error_code": code }))).into_response()
    };
    let Some(new_name) = body.get("name").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()) else {
        return fail(StatusCode::BAD_REQUEST, "Missing 'name'".into(), "invalid_request");
    };
    if new_name.contains(crate::chat::words::is_comma) {
        return invalid_name().into_response();
    }
    // 설치·업데이트·복원 중에는 바꾸지 않는다: 진행 알림(`provision.progress`)이 시작 때의 이름을 싣는다
    let Some(_guard) = super::backup::InstanceLock::take_for_writes(&id).await else {
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
        Some(_) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_body", "message": "settings must be an object" }))).into_response(),
    };
    // 보내지 않으면 저장된 추가 인자로 본다
    let extra = match body.get("extra_args").map(extra_args_from) {
        None => None,
        Some(Some(args)) => Some(args),
        Some(None) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_format", "error_code": "invalid_format", "message": "extra_args must be a string or an array of strings" }))).into_response()
        }
    };
    match crate::supervisor::Supervisor::launch_preview(&state.supervisor, &id, &draft, extra.as_deref()).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) if e.downcast_ref::<crate::supervisor::InstanceNotFound>().is_some() => (StatusCode::NOT_FOUND, Json(crate::plugin::error_response(&e))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&e))).into_response(),
    }
}

/// 설정의 저장된 값: 공통 칸(`constants::INSTANCE_FIELDS`, 예: port)은 PATCH가 인스턴스 칸에 쓰므로 그 값을,
/// 아니면 `module_settings`를 본다. `view`는 그 인스턴스를 JSON으로 펼친 것
pub(crate) fn stored_setting(view: &Value, instance: &crate::instance::ServerInstance, key: &str) -> Option<Value> {
    let own = saba_chan_updater_lib::constants::INSTANCE_FIELDS.iter().any(|(k, _)| *k == key);
    own.then(|| view.get(key).filter(|v| !v.is_null()).cloned()).flatten().or_else(|| instance.module_settings.get(key).cloned())
}

/// 모듈 설정 칸마다 저장된 값 (`stored_setting`, 값이 없는 칸은 뺀다). 클라이언트는 이 값을 그대로 칸 값으로 쓴다
pub(crate) fn stored_settings(instance: &crate::instance::ServerInstance, fields: &[crate::supervisor::module_loader::SettingField]) -> serde_json::Map<String, Value> {
    let view = serde_json::to_value(instance).unwrap_or_default();
    fields.iter().filter_map(|f| stored_setting(&view, instance, &f.name).map(|v| (f.name.clone(), v))).collect()
}

/// 익스텐션 hook에 넘기는 인스턴스: REST 비밀번호, 모듈이 password로 선언했거나 이름이 비밀스러운 설정 값은 가린다 (D-E2)
fn instance_for_hooks(module: Option<&crate::supervisor::module_loader::ModuleMetadata>, instance: &crate::instance::ServerInstance) -> Value {
    settings_for_hooks(module, &serde_json::to_value(instance).unwrap_or_default())
}

/// hook에 넘기는 설정 값에서 같은 기준으로 비밀을 가린다. 키 이름으로만 가린다:
/// 인스턴스를 넘기면 비밀값과 같은 글자가 이름·경로에서도 지워진다
fn settings_for_hooks(module: Option<&crate::supervisor::module_loader::ModuleMetadata>, settings: &Value) -> Value {
    crate::audit::redact(module, None, None, Some(settings)).1.unwrap_or_default()
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

    use serde_json::json;

    /// 익스텐션 hook에 넘기는 인스턴스는 REST 비밀번호와 비밀스러운 설정 값을 가리고, 나머지는 그대로 둔다 (D-E2)
    #[test]
    fn hooks_get_the_instance_without_its_secrets() {
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.rest_password = Some("hunter2-rest".into());
        inst.module_settings.insert("server_password".into(), json!("hunter2-join"));
        inst.module_settings.insert("max_players".into(), json!(16));
        let value = super::instance_for_hooks(None, &inst);
        let text = value.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert_eq!(value["module_settings"]["max_players"], 16);
        assert_eq!(value["name"], "srv");
        let settings = super::settings_for_hooks(None, &json!({ "rest_password": "hunter2-new", "port": 7777 }));
        assert!(!settings.to_string().contains("hunter2"), "{settings}");
        assert_eq!(settings["port"], 7777);
        // 비밀값과 같은 글자의 이름·경로는 비밀이 아니다: 값 대신 키 이름으로만 가린다
        let mut same = crate::instance::ServerInstance::new("palworld", "game");
        same.working_dir = Some("C:/srv/palworld/data".into());
        same.rest_password = Some("palworld".into());
        let value = super::instance_for_hooks(None, &same);
        assert_eq!(value["name"], "palworld");
        assert_eq!(value["working_dir"], "C:/srv/palworld/data");
        assert_ne!(value["rest_password"], "palworld");
    }
}
