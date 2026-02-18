use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

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

/// POST /api/instances - 새 인스턴스 생성
pub async fn create_instance(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));

    let module_name = payload
        .get("module_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing module_name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));

    if let (Ok(name), Ok(module)) = (name, module_name) {
        let mut instance = crate::instance::ServerInstance::new(name, module);

        // Docker 모드 플래그
        let use_docker = payload
            .get("use_docker")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Docker 모드 요청 시 Docker 익스텐션이 활성화되어 있는지 검증
        if use_docker {
            let ext_mgr = state.extension_manager.read().await;
            if !ext_mgr.is_enabled("docker") {
                let error = json!({
                    "error": "Cannot create Docker instance: the 'docker' extension is not enabled. Enable it in Settings → Extensions first.",
                    "error_code": "extension_required",
                    "extension_id": "docker",
                });
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
            }
            drop(ext_mgr);
        }

        // Extension 시스템: extension_data 초기 설정
        if use_docker {
            instance.extension_data.insert(
                "docker".to_string(),
                serde_json::json!({ "enabled": true }),
            );
        }

        instance.use_docker = use_docker;

        // 모듈 정보에서 process_name, default_port, install/docker config 가져오기
        let module_install = if let Ok(loaded_module) = supervisor.module_loader.get_module(module) {
            instance.process_name = loaded_module.metadata.process_name.clone();
            if instance.port.is_none() {
                instance.port = loaded_module.metadata.default_port;
            }
            // rcon_port / rest_port の기본값을 모듈 설정에서 가져오기
            if instance.rcon_port.is_none() {
                instance.rcon_port = Some(loaded_module.metadata.default_rcon_port());
            }
            if instance.rest_port.is_none() {
                instance.rest_port = Some(loaded_module.metadata.default_rest_port());
            }
            Some((
                loaded_module.metadata.install.clone(),
                loaded_module.metadata.docker.clone(),
            ))
        } else {
            None
        };

        // 선택적 필드 설정
        if let Some(path) = payload.get("executable_path").and_then(|v| v.as_str()) {
            instance.executable_path = Some(path.to_string());
        }
        if let Some(dir) = payload.get("working_dir").and_then(|v| v.as_str()) {
            instance.working_dir = Some(dir.to_string());
        }
        if let Some(port) = payload.get("port").and_then(|v| v.as_u64()) {
            instance.port = Some(port as u16);
        }

        let id = instance.id.clone();
        let instance_name = instance.name.clone();
        let module_name_owned = module.to_string();

        // Docker 모드일 때: working_dir을 인스턴스 디렉토리의 server/ 하위 경로로 설정
        let instance_dir = supervisor.instance_store.instance_dir(&id);
        if use_docker {
            let server_dir = instance_dir.join("server");
            instance.working_dir = Some(server_dir.to_string_lossy().to_string());
        }

        // 인스턴스 저장 (Docker 프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // ── Extension hook: server.post_create ──
                if use_docker {
                    let ext_mgr = state.extension_manager.clone();
                    let ctx = serde_json::json!({
                        "instance_id": &id,
                        "instance_name": &instance_name,
                        "module": &module_name_owned,
                        "use_docker": use_docker,
                        "instance_dir": instance_dir.to_string_lossy(),
                        "extension_data": &instance.extension_data,
                        "module_install": module_install.as_ref().map(|(install, docker)| {
                            serde_json::json!({
                                "install": install,
                                "docker": docker,
                            })
                        }),
                    });
                    let tracker = state.provision_tracker.clone();
                    let inst_clone = instance.clone();
                    tokio::spawn(async move {
                        let mgr = ext_mgr.read().await;
                        let _results = mgr.dispatch_hook("server.post_create", ctx).await;
                        tracing::info!("Extension post_create dispatched for '{}'", inst_clone.name);
                    });

                    let response = json!({
                        "success": true,
                        "id": id,
                        "provisioning": true,
                    });
                    (StatusCode::CREATED, Json(response)).into_response()
                } else {
                    let response = json!({ "success": true, "id": id });
                    (StatusCode::CREATED, Json(response)).into_response()
                }
            }
            Err(e) => {
                let error = json!({ "error": format!("Failed to create instance: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": "Invalid request" });
        (StatusCode::BAD_REQUEST, Json(error)).into_response()
    }
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
        (StatusCode::OK, Json(resp)).into_response()
    } else {
        (StatusCode::OK, Json(json!({
            "active": false,
        }))).into_response()
    }
}

/// DELETE /api/instance/:id - 인스턴스 삭제
pub async fn delete_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    // ── Extension hook: server.pre_delete ──
    if let Some(instance) = supervisor.instance_store.get(&id) {
        let ext_mgr = state.extension_manager.clone();
        let ctx = serde_json::json!({
            "instance_id": &id,
            "instance_name": &instance.name,
            "module": &instance.module_name,
            "use_docker": instance.use_docker,
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
        }
    }

    match supervisor.instance_store.remove(&id) {
        Ok(_) => {
            let response = json!({ "success": true });
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to delete instance: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// PATCH /api/instance/:id - 인스턴스 설정 업데이트
pub async fn update_instance_settings(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(settings): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    // 인스턴스 찾기
    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst,
        None => {
            let error = json!({ "error": "Instance not found" });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    // ── 설정값 타입/범위 스키마 검증 ──
    if let Some(settings_obj) = settings.as_object() {
        if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
            if let Some(ref settings_meta) = module.metadata.settings {
                let errors = crate::validator::validate_all_settings(
                    &settings_meta.fields,
                    settings_obj,
                );
                if !errors.is_empty() {
                    let error_details: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                    tracing::warn!(
                        "Settings validation failed for instance {}: {:?}",
                        id, error_details
                    );
                    let error = json!({
                        "error": "validation_failed",
                        "error_code": "validation_failed",
                        "message": "Settings validation failed",
                        "details": error_details,
                    });
                    return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                }
            }
        }
    }

    // 설정값 업데이트
    let mut updated = instance.clone();

    // working_dir이 null인데 executable_path가 있으면 자동 보정
    if updated.working_dir.is_none() {
        if let Some(ref exe_path) = updated.executable_path {
            if let Some(parent) = std::path::Path::new(exe_path).parent() {
                updated.working_dir = Some(parent.to_string_lossy().to_string());
                tracing::info!(
                    "Auto-inferred working_dir to {} from existing executable_path",
                    parent.display()
                );
            }
        }
    }

    // 하드코딩된 공통 필드 목록
    let known_fields: std::collections::HashSet<&str> = [
        "port",
        "rcon_port",
        "rcon_password",
        "rest_host",
        "rest_port",
        "rest_username",
        "rest_password",
        "executable_path",
        "protocol_mode",
        "server_version",
        "docker_cpu_limit",
        "docker_memory_limit",
    ]
    .iter()
    .cloned()
    .collect();

    // common settings
    // port: 숫자 또는 문자열 수용
    if let Some(port_value) = settings.get("port") {
        match port_value {
            serde_json::Value::Number(n) => {
                if let Some(port) = n.as_u64() {
                    updated.port = Some(port as u16);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(port) = s.parse::<u16>() {
                    updated.port = Some(port);
                }
            }
            _ => {}
        }
    }

    // rcon_port: 숫자 또는 문자열 수용
    if let Some(rcon_port_value) = settings.get("rcon_port") {
        match rcon_port_value {
            serde_json::Value::Number(n) => {
                if let Some(rcon_port) = n.as_u64() {
                    updated.rcon_port = Some(rcon_port as u16);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(rcon_port) = s.parse::<u16>() {
                    updated.rcon_port = Some(rcon_port);
                }
            }
            _ => {}
        }
    }

    if let Some(rcon_password) = settings.get("rcon_password").and_then(|v| v.as_str()) {
        updated.rcon_password = Some(rcon_password.to_string());
    }

    // managed_start ↔ RCON 연동:
    //   managed=true  → enable_rcon=false (stdin으로 제어, RCON 불필요)
    //   managed=false → enable_rcon=true  + 비밀번호 자동생성 (RCON이 유일한 제어 수단)
    let managed_start = settings.get("managed_start").and_then(|v| match v {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => Some(s == "true"),
        _ => None,
    });
    let enable_rcon = match managed_start {
        Some(true) => Some(false),  // managed → RCON 비활성화
        Some(false) => Some(true),  // native → RCON 강제 활성화
        None => {
            // managed_start가 전송되지 않은 경우: 기존 enable_rcon 값 유지
            settings.get("enable_rcon").and_then(|v| match v {
                serde_json::Value::Bool(b) => Some(*b),
                serde_json::Value::String(s) => Some(s == "true"),
                _ => None,
            })
        }
    };
    if enable_rcon == Some(true) {
        let current_password = updated.rcon_password.as_deref().unwrap_or("");
        if current_password.is_empty() {
            // UUID 기반 16자 비밀번호 생성 (영숫자만)
            let password: String = uuid::Uuid::new_v4()
                .to_string()
                .replace('-', "")
                .chars()
                .take(16)
                .collect();
            tracing::info!(
                "Auto-generated RCON password for instance {} (native mode, no password set)",
                id
            );
            updated.rcon_password = Some(password.clone());
        }
    }

    if let Some(rest_host) = settings.get("rest_host").and_then(|v| v.as_str()) {
        updated.rest_host = Some(rest_host.to_string());
    }
    if let Some(rest_port_value) = settings.get("rest_port") {
        match rest_port_value {
            serde_json::Value::Number(n) => {
                if let Some(rest_port) = n.as_u64() {
                    updated.rest_port = Some(rest_port as u16);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(rest_port) = s.parse::<u16>() {
                    updated.rest_port = Some(rest_port);
                }
            }
            _ => {}
        }
    }
    if let Some(rest_username) = settings.get("rest_username").and_then(|v| v.as_str()) {
        updated.rest_username = Some(rest_username.to_string());
    }
    if let Some(rest_password) = settings.get("rest_password").and_then(|v| v.as_str()) {
        updated.rest_password = Some(rest_password.to_string());
    }

    if let Some(executable_path) = settings.get("executable_path").and_then(|v| v.as_str()) {
        updated.executable_path = Some(executable_path.to_string());
        // working_dir이 미설정이면 executable_path의 부모 디렉토리로 자동 설정
        if updated.working_dir.is_none() {
            if let Some(parent) = std::path::Path::new(executable_path).parent() {
                updated.working_dir = Some(parent.to_string_lossy().to_string());
                tracing::info!(
                    "Auto-set working_dir to {} from executable_path",
                    parent.display()
                );
            }
        }
    }

    // protocol_mode 설정 (rest 또는 rcon)
    if let Some(protocol_mode) = settings.get("protocol_mode").and_then(|v| v.as_str()) {
        updated.protocol_mode = protocol_mode.to_string();
    }

    // server_version 설정
    if let Some(version) = settings.get("server_version").and_then(|v| v.as_str()) {
        updated.server_version = Some(version.to_string());
    }

    // Docker 리소스 제한 설정
    if let Some(cpu_value) = settings.get("docker_cpu_limit") {
        match cpu_value {
            serde_json::Value::Number(n) => {
                updated.docker_cpu_limit = n.as_f64();
            }
            serde_json::Value::String(s) if s.is_empty() => {
                updated.docker_cpu_limit = None;
            }
            serde_json::Value::String(s) => {
                if let Ok(cpu) = s.parse::<f64>() {
                    updated.docker_cpu_limit = Some(cpu);
                }
            }
            serde_json::Value::Null => {
                updated.docker_cpu_limit = None;
            }
            _ => {}
        }
    }
    if let Some(mem_value) = settings.get("docker_memory_limit") {
        match mem_value {
            serde_json::Value::String(s) if s.is_empty() => {
                updated.docker_memory_limit = None;
            }
            serde_json::Value::String(s) => {
                updated.docker_memory_limit = Some(s.clone());
            }
            serde_json::Value::Null => {
                updated.docker_memory_limit = None;
            }
            _ => {}
        }
    }

    // 동적 모듈 설정 저장 (하드코딩 필드 이외의 모든 설정을 module_settings에 저장)
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if !known_fields.contains(key.as_str()) {
                updated.module_settings.insert(key.clone(), value.clone());
            }
        }
    }

    tracing::info!(
        "Updating instance {} with settings: port={:?}, rcon_port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}",
        id,
        updated.port,
        updated.rcon_port,
        updated.executable_path,
        updated.protocol_mode,
        updated.module_settings.len()
    );

    // 모든 설정을 server.properties에 동기화 (configure lifecycle 호출)
    let mut props_sync = serde_json::Map::new();
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            // protocol_mode, executable_path 등은 server.properties에 관련 없으므로 제외
            if key == "protocol_mode"
                || key == "executable_path"
                || key == "server_version"
                || key == "rest_host"
                || key == "rest_port"
                || key == "rest_username"
                || key == "rest_password"
                || key == "java_path"
                || key == "ram"
                || key == "use_aikar_flags"
                || key == "managed_start"
                || key == "graceful_stop"
                || key == "docker_cpu_limit"
                || key == "docker_memory_limit"
            {
                continue;
            }
            props_sync.insert(key.clone(), value.clone());
        }
    }

    // managed_start ↔ RCON: enable_rcon을 props_sync에 주입
    if let Some(rcon_on) = enable_rcon {
        props_sync.insert("enable_rcon".to_string(), json!(rcon_on));
    }
    // 자동 생성된 RCON 비밀번호가 있으면 props_sync에도 추가
    if let Some(auto_password) = &updated.rcon_password {
        if !props_sync.contains_key("rcon_password") && enable_rcon == Some(true) {
            props_sync.insert("rcon_password".to_string(), json!(auto_password));
        }
    }

    // 저장
    let updated_clone = updated.clone();
    if let Err(e) = supervisor.instance_store.update(&id, updated) {
        let error = json!({ "error": format!("Failed to update instance: {}", e) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // server.properties 동기화 (변경된 항목이 있을 때만)
    if !props_sync.is_empty() {
        tracing::info!(
            "Syncing settings to server.properties for instance {}: {:?}",
            id,
            props_sync
        );
        let props_value = Value::Object(props_sync);
        match supervisor
            .manage_properties(&id, "write", Some(props_value))
            .await
        {
            Ok(_) => tracing::info!("server.properties synced successfully for instance {}", id),
            Err(e) => tracing::warn!(
                "Failed to sync server.properties for instance {}: {}",
                id,
                e
            ),
        }
    }

    // 인스턴스 설정 변경 시 Extension hook 디스패치
    if updated_clone.use_docker {
        // ── Extension hook: server.settings_changed ──
        let ext_mgr = state.extension_manager.clone();
        let ctx = serde_json::json!({
            "instance_id": &id,
            "instance": {
                "name": &updated_clone.name,
                "module_name": &updated_clone.module_name,
                "use_docker": updated_clone.use_docker,
                "port": updated_clone.port,
                "rcon_port": updated_clone.rcon_port,
                "rest_port": updated_clone.rest_port,
                "rest_password": &updated_clone.rest_password,
                "docker_cpu_limit": updated_clone.docker_cpu_limit,
                "docker_memory_limit": &updated_clone.docker_memory_limit,
                "module_settings": &updated_clone.module_settings,
                "extension_data": &updated_clone.extension_data,
            },
            "settings": &settings,
        });
        let mgr = ext_mgr.read().await;
        let _results = mgr.dispatch_hook("server.settings_changed", ctx).await;
    }

    (StatusCode::OK, Json(json!({ "success": true }))).into_response()
}
