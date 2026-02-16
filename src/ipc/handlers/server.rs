use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{
    IPCServer, ModuleInfo, ModuleListResponse, ProtocolsInfo, ServerInfo, ServerListResponse,
    ServerStartRequest, ServerStopRequest,
};

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반)
pub async fn list_servers(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instances = supervisor.instance_store.list();
    let mut servers = Vec::new();

    for instance in instances {
        // ProcessTracker에서 PID 및 시작 시간 확인
        let pid = supervisor.tracker.get_pid(&instance.id).ok();
        let (status, start_time) = if pid.is_some() {
            let st = supervisor.tracker.get_start_time(&instance.id).ok();
            ("running".to_string(), st)
        } else {
            ("stopped".to_string(), None)
        };
        // API(CLI/Discord)를 통한 마지막 시작/정지 타임스탬프 조회
        let last_api = state.api_actions.get(&instance.name)
            .or_else(|| state.api_actions.get(&instance.id));
        servers.push(ServerInfo {
            id: instance.id.clone(),
            name: instance.name.clone(),
            module: instance.module_name.clone(),
            status,
            pid,
            start_time,
            executable_path: instance.executable_path.clone(),
            port: instance.port,
            rcon_port: instance.rcon_port,
            rcon_password: instance.rcon_password.clone(),
            rest_host: instance.rest_host.clone(),
            rest_port: instance.rest_port,
            rest_username: instance.rest_username.clone(),
            rest_password: instance.rest_password.clone(),
            protocol_mode: instance.protocol_mode.clone(),
            module_settings: instance.module_settings.clone(),
            server_version: instance.server_version.clone(),
            last_api_action: last_api,
        });
    }

    Json(ServerListResponse { servers })
}

/// GET /api/modules - 모든 모듈 목록
pub async fn list_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.list_modules() {
        Ok(modules) => {
            let module_infos: Vec<ModuleInfo> = modules
                .into_iter()
                .map(|m| {
                    // 아이콘 파일을 base64로 인코딩
                    let icon_base64 = m.metadata.icon.as_ref().and_then(|icon_file| {
                        let icon_path = std::path::Path::new(&m.path).join(icon_file);
                        std::fs::read(&icon_path).ok().map(|data| {
                            use base64::{engine::general_purpose, Engine as _};
                            format!(
                                "data:image/png;base64,{}",
                                general_purpose::STANDARD.encode(&data)
                            )
                        })
                    });

                    ModuleInfo {
                        name: m.metadata.name,
                        version: m.metadata.version,
                        description: m.metadata.description,
                        path: m.path,
                        executable_path: m.metadata.executable_path,
                        icon: icon_base64,
                        interaction_mode: m.metadata.interaction_mode,
                        protocols: m.metadata.protocols_supported.map(|supported| {
                            ProtocolsInfo {
                                supported,
                                default: m.metadata.protocols_default,
                            }
                        }),
                        settings: m.metadata.settings,
                        commands: m.metadata.commands,
                        syntax_highlight: m.metadata.syntax_highlight,
                    }
                })
                .collect();
            (StatusCode::OK, Json(ModuleListResponse { modules: module_infos })).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to list modules: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// POST /api/modules/refresh - 모듈 캐시를 새로고침하고 다시 발견
pub async fn refresh_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.refresh_modules() {
        Ok(modules) => {
            let module_infos: Vec<ModuleInfo> = modules
                .into_iter()
                .map(|m| {
                    // 아이콘 파일을 base64로 인코딩
                    let icon_base64 = m.metadata.icon.as_ref().and_then(|icon_file| {
                        let icon_path = std::path::Path::new(&m.path).join(icon_file);
                        std::fs::read(&icon_path).ok().map(|data| {
                            use base64::{engine::general_purpose, Engine as _};
                            format!(
                                "data:image/png;base64,{}",
                                general_purpose::STANDARD.encode(&data)
                            )
                        })
                    });

                    ModuleInfo {
                        name: m.metadata.name,
                        version: m.metadata.version,
                        description: m.metadata.description,
                        path: m.path,
                        executable_path: m.metadata.executable_path,
                        icon: icon_base64,
                        interaction_mode: m.metadata.interaction_mode,
                        protocols: m.metadata.protocols_supported.map(|supported| {
                            ProtocolsInfo {
                                supported,
                                default: m.metadata.protocols_default,
                            }
                        }),
                        settings: m.metadata.settings,
                        commands: m.metadata.commands,
                        syntax_highlight: m.metadata.syntax_highlight,
                    }
                })
                .collect();
            tracing::info!(
                "Module cache refreshed. Found {} modules",
                module_infos.len()
            );
            (StatusCode::OK, Json(ModuleListResponse { modules: module_infos })).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to refresh modules: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// GET /api/module/:name - 모듈 메타데이터 조회 (별명 포함)
pub async fn get_module_metadata(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.list_modules() {
        Ok(modules) => {
            if let Some(module) = modules.iter().find(|m| m.metadata.name == name) {
                // TOML에서 aliases 섹션 읽기 시도
                let module_path = format!("{}/module.toml", module.path);
                match std::fs::read_to_string(&module_path) {
                    Ok(content) => match toml::from_str::<serde_json::Value>(&content) {
                        Ok(parsed) => (
                            StatusCode::OK,
                            Json(json!({
                                "name": &module.metadata.name,
                                "version": &module.metadata.version,
                                "description": &module.metadata.description,
                                "path": &module.path,
                                "metadata": &module.metadata,
                                "toml": parsed,
                            })),
                        )
                            .into_response(),
                        Err(_) => (
                            StatusCode::OK,
                            Json(json!({
                                "name": &module.metadata.name,
                                "version": &module.metadata.version,
                                "description": &module.metadata.description,
                                "path": &module.path,
                                "metadata": &module.metadata,
                            })),
                        )
                            .into_response(),
                    },
                    Err(_) => (
                        StatusCode::OK,
                        Json(json!({
                            "name": &module.metadata.name,
                            "version": &module.metadata.version,
                            "description": &module.metadata.description,
                            "path": &module.path,
                            "metadata": &module.metadata,
                        })),
                    )
                        .into_response(),
                }
            } else {
                let error = json!({ "error": format!("Module '{}' not found", name) });
                (StatusCode::NOT_FOUND, Json(error)).into_response()
            }
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to list modules: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// GET /api/server/:name/status - 서버 상태 조회
pub async fn get_server_status(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // instance에서 모듈명 조회
    let instance = supervisor
        .instance_store
        .list()
        .iter()
        .find(|i| i.name == name);

    if let Some(inst) = instance {
        match supervisor
            .get_server_status(&name, &inst.module_name)
            .await
        {
            Ok(result) => (StatusCode::OK, Json(result)).into_response(),
            Err(e) => {
                let error = json!({ "error": format!("Failed to get status: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": format!("Server '{}' not found", name) });
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}

/// POST /api/server/:name/start - 서버 시작
pub async fn start_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStartRequest>,
) -> impl IntoResponse {
    // API 경유 시작 기록 (GUI 외부 시작 감지용)
    state.api_actions.record(&name);
    let supervisor = state.supervisor.read().await;

    match supervisor
        .start_server(&name, &payload.module, payload.config)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let error = json!({ "error": format!("Failed to start server: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// POST /api/server/:name/stop - 서버 중지
pub async fn stop_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStopRequest>,
) -> impl IntoResponse {
    // API 경유 정지 기록 (GUI 외부 정지 감지용)
    state.api_actions.record(&name);
    let supervisor = state.supervisor.read().await;

    // instance에서 모듈명과 ID 조회
    let instance = supervisor
        .instance_store
        .list()
        .iter()
        .find(|i| i.name == name)
        .cloned();

    if let Some(inst) = instance {
        // instance id로도 API 액션 기록
        state.api_actions.record(&inst.id);
        match supervisor
            .stop_server(&name, &inst.module_name, payload.force)
            .await
        {
            Ok(result) => {
                // 실제 종료 성공 시에만 tracker에서 제거
                let success = result
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                if success {
                    // name과 id 둘 다로 untrack 시도
                    let _ = supervisor.tracker.untrack(&name);
                    if let Err(e) = supervisor.tracker.untrack(&inst.id) {
                        tracing::warn!("Failed to untrack stopped server '{}': {}", name, e);
                    } else {
                        tracing::info!("Server '{}' untracked from process tracker", name);
                    }
                }
                (StatusCode::OK, Json(result)).into_response()
            }
            Err(e) => {
                let error = json!({ "error": format!("Failed to stop server: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": format!("Server '{}' not found", name) });
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}
