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

        // 익스텐션 모드 플래그 (예: 컨테이너 격리)
        // "use_container" 우선, "use_docker" 레거시 호환
        let use_container_ext = payload
            .get("use_container")
            .or_else(|| payload.get("use_docker"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 컨테이너 격리 요청 시 해당 익스텐션이 활성화되어 있는지 검증
        if use_container_ext {
            let ext_mgr = state.extension_manager.read().await;
            if !ext_mgr.is_enabled("docker") {
                let error = json!({
                    "error": "Cannot create instance: the required extension is not enabled. Enable it in Settings → Extensions first.",
                    "error_code": "extension_required",
                    "extension_id": "docker",
                });
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
            }
            drop(ext_mgr);
        }

        // extension_data 설정 (컨테이너 격리 플래그 → extension_data에 저장)
        if use_container_ext {
            instance.extension_data.insert(
                "docker_enabled".to_string(),
                serde_json::json!(true),
            );
        }

        // 모듈 정보에서 process_name, default_port, install/container config 가져오기
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
                loaded_module.metadata.extensions.get("docker").cloned(),  // 컨테이너 익스텐션 설정
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

        // 컨테이너 모드일 때: working_dir를 인스턴스 디렉토리의 server/ 하위 경로로 설정
        let instance_dir = supervisor.instance_store.instance_dir(&id);
        if use_container_ext {
            let server_dir = instance_dir.join("server");
            instance.working_dir = Some(server_dir.to_string_lossy().to_string());
        }

        // 인스턴스 저장 (프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // ── Extension hook: server.post_create ──
                if use_container_ext {
                    let ext_mgr = state.extension_manager.clone();
                    let tracker = state.provision_tracker.clone();
                    let ctx = serde_json::json!({
                        "instance_id": &id,
                        "instance_name": &instance_name,
                        "module": &module_name_owned,
                        "use_container": use_container_ext,
                        "instance_dir": instance_dir.to_string_lossy(),
                        "extension_data": &instance.extension_data,
                        "instance": serde_json::to_value(&instance).unwrap_or_default(),
                        "module_install": module_install.as_ref().map(|(install, ext_container)| {
                            serde_json::json!({
                                "install": install,
                                "container": ext_container,
                            })
                        }),
                    });
                    let inst_clone = instance.clone();

                    // 초기 프로비저닝 상태 등록 (범용 — 스텝 정보는 extension이 제공)
                    tracker.update(&inst_clone.name, crate::ipc::ProvisionProgress {
                        step: 0,
                        total: 1,
                        label: "initializing".to_string(),
                        message: "Initializing...".to_string(),
                        done: false,
                        error: None,
                        percent: Some(0),
                        steps: None,
                    });

                    let tracker_cb = tracker.clone();
                    let name_cb = inst_clone.name.clone();
                    let steps_store = std::sync::Arc::new(std::sync::Mutex::new(None::<Vec<String>>));
                    let steps_write = steps_store.clone();
                    let steps_final = steps_store.clone();
                    let on_progress = move |prog: crate::plugin::ExtensionProgress| {
                        let pct = prog.percent.unwrap_or(0);
                        let msg = prog.message.clone().unwrap_or_default();
                        // extension이 steps 목록을 보내면 저장
                        if let Some(ref new_steps) = prog.steps {
                            if let Ok(mut s) = steps_write.lock() {
                                *s = Some(new_steps.clone());
                            }
                        }
                        let stored_steps = steps_store.lock().ok().and_then(|s| s.clone());
                        tracker_cb.update(&name_cb, crate::ipc::ProvisionProgress {
                            step: prog.step.unwrap_or(0),
                            total: prog.total.unwrap_or(1),
                            label: prog.label.unwrap_or_default(),
                            message: msg,
                            done: false,  // progress 콜백에서는 절대 done 설정하지 않음
                            error: None,
                            percent: Some(pct),
                            steps: stored_steps,
                        });
                    };

                    let tracker_done = tracker.clone();
                    let name_done = inst_clone.name.clone();
                    tokio::spawn(async move {
                        let mgr = ext_mgr.read().await;
                        let results = mgr.dispatch_hook_with_progress(
                            "server.post_create", ctx, on_progress,
                        ).await;
                        // 완료 또는 에러 상태 기록 (Err variant + Python success:false 모두 체크)
                        let err = results.iter().find_map(|(_, r)| match r {
                            Err(e) => Some(e.to_string()),
                            Ok(val) => {
                                if val.get("success").and_then(|s| s.as_bool()) == Some(false) {
                                    val.get("error")
                                        .and_then(|e| e.as_str())
                                        .map(|s| s.to_string())
                                        .or_else(|| Some("Extension reported failure".to_string()))
                                } else {
                                    None
                                }
                            }
                        });
                        let final_steps = steps_final.lock().ok().and_then(|s| s.clone());
                        if let Some(ref e) = err {
                            tracing::warn!("Provisioning failed for '{}': {}", inst_clone.name, e);
                        }
                        let has_error = err.is_some();
                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: if has_error { "Provisioning failed".to_string() } else { "Provisioning complete".to_string() },
                            done: true,
                            error: err,
                            percent: Some(100),
                            steps: final_steps,
                        });
                        tracing::info!("Extension post_create dispatched for '{}' (error={})", inst_clone.name, has_error);

                        // 성공 시 5초 후 tracker 자동 정리 → provisioning UI 자연스럽게 사라짐
                        // 에러 시에는 유지 → 사용자가 dismiss할 때까지 UI 표시
                        if !has_error {
                            let tracker_cleanup = tracker_done.clone();
                            let name_cleanup = name_done.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                tracker_cleanup.remove(&name_cleanup);
                            });
                        }
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
        "extension_data",
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

    // ── 범용 extension_data 갱신 ──
    // 프론트엔드에서 { "extension_data": { "key": value, ... } } 형태로 전달
    // null 값은 해당 키 삭제, 빈 문자열도 삭제 처리
    if let Some(ext_updates) = settings.get("extension_data") {
        if let Some(obj) = ext_updates.as_object() {
            for (key, value) in obj {
                match value {
                    serde_json::Value::Null => {
                        updated.extension_data.remove(key);
                    }
                    serde_json::Value::String(s) if s.is_empty() => {
                        updated.extension_data.remove(key);
                    }
                    serde_json::Value::String(s) => {
                        // 숫자로 변환 가능하면 Number로 저장 (CPU 제한 등)
                        if let Ok(n) = s.parse::<f64>() {
                            updated.extension_data.insert(key.clone(), json!(n));
                        } else {
                            updated.extension_data.insert(key.clone(), value.clone());
                        }
                    }
                    _ => {
                        updated.extension_data.insert(key.clone(), value.clone());
                    }
                }
            }
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
                || key == "extension_data"
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

    // 인스턴스 설정 변경 시 Extension hook 디스패치 (extension_data가 있는 경우)
    // instance_dir + 모듈 익스텐션 설정을 범용으로 전달 → 각 extension이 자체 판단
    if !updated_clone.extension_data.is_empty() {
        let ext_mgr = state.extension_manager.clone();
        let instance_dir = supervisor.instance_store.instance_dir(&id);

        // 모듈의 모든 익스텐션 설정을 범용으로 전달 (특정 익스텐션 이름 참조 없음)
        let module_extensions = supervisor.module_loader.get_module(&updated_clone.module_name)
            .ok()
            .map(|m| serde_json::to_value(&m.metadata.extensions).unwrap_or_default())
            .unwrap_or_else(|| serde_json::json!({}));

        let ctx = serde_json::json!({
            "instance_id": &id,
            "instance_dir": instance_dir.to_string_lossy(),
            "instance": serde_json::to_value(&updated_clone).unwrap_or_default(),
            "module_extensions": module_extensions,
            "extension_data": &updated_clone.extension_data,
            "settings": &settings,
        });
        let mgr = ext_mgr.read().await;
        let _results = mgr.dispatch_hook("server.settings_changed", ctx).await;
    }

    (StatusCode::OK, Json(json!({ "success": true }))).into_response()
}
