use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::ipc::IPCServer;

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
    //   managed=false → enable_rcon=true  (RCON이 유일한 제어 수단)
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

    // NOTE: RCON/REST 비밀번호 자동 생성은 인스턴스 실행 시점에 supervisor에서 처리합니다.

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
    // credential_map 범용 역동기화: module_settings 키 → daemon first-class 필드
    // 예: Palworld credential_map { rest_password = "AdminPassword" }
    //   → module_settings의 "admin_password" (snake_case) 값을 instance.rest_password에 동기화
    if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
        for (daemon_key, game_key) in &module.metadata.credential_map {
            // game_key를 snake_case로 변환하여 module_settings에서 탐색
            let snake_key = game_key
                .chars()
                .enumerate()
                .fold(String::new(), |mut acc, (i, c)| {
                    if c.is_uppercase() && i > 0 {
                        acc.push('_');
                    }
                    acc.push(c.to_lowercase().next().unwrap());
                    acc
                });
            let value = settings.get(&snake_key)
                .or_else(|| settings.get(game_key.as_str()))
                .and_then(|v| v.as_str());
            if let Some(pwd) = value {
                if !pwd.is_empty() {
                    match daemon_key.as_str() {
                        "rest_password" if updated.rest_password.as_deref().unwrap_or("").is_empty() => {
                            updated.rest_password = Some(pwd.to_string());
                        }
                        "rcon_password" if updated.rcon_password.as_deref().unwrap_or("").is_empty() => {
                            updated.rcon_password = Some(pwd.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let Some(executable_path) = settings.get("executable_path").and_then(|v| v.as_str()) {
        // executable_path는 보통 모듈의 server_executable + working_dir에서 자동 해석되지만,
        // 명시적으로 설정하면 override로 저장합니다.
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
    // 인스턴스에 저장된 RCON 비밀번호가 있으면 props_sync에도 반영
    if let Some(pwd) = &updated.rcon_password {
        if !props_sync.contains_key("rcon_password") {
            props_sync.insert("rcon_password".to_string(), json!(pwd));
        }
    }

    // 저장
    let updated_clone = updated.clone();
    if let Err(e) = supervisor.instance_store.update(&id, updated) {
        let error = json!({ "error": format!("Failed to update instance: {}", e) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // server.properties 동기화 (변경된 항목이 있을 때만)
    // configure()가 실행은 되었지만 내부적으로 실패(success:false)할 수 있으므로
    // run_plugin의 Err뿐 아니라 반환된 JSON의 success 필드도 확인해야 한다.
    // 그렇지 않으면 실제 설정 파일 쓰기가 실패해도 API는 항상 success:true를
    // 반환해 사용자가 "적용했는데 실제 서버엔 반영이 안 된" 상태를 알아채지 못한다.
    let mut properties_warning: Option<String> = None;
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
            Ok(result) => {
                let ok = result.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                if ok {
                    tracing::info!("server.properties synced successfully for instance {}", id);
                } else {
                    let msg = result
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error")
                        .to_string();
                    tracing::warn!(
                        "configure() reported failure syncing server.properties for instance {}: {}",
                        id,
                        msg
                    );
                    properties_warning = Some(msg);
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to sync server.properties for instance {}: {}",
                    id,
                    e
                );
                properties_warning = Some(e.to_string());
            }
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

    let mut response = json!({ "success": true });
    if let Some(warning) = properties_warning {
        response["properties_warning"] = json!(warning);
    }
    (StatusCode::OK, Json(response)).into_response()
}

