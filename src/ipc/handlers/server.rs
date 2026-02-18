use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{
    IPCServer, ExtensionInfo, ExtensionListResponse, PortConflictInfo, ProtocolsInfo, ServerInfo, ServerListResponse,
    ServerStartRequest, ServerStopRequest,
};

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반)
pub async fn list_servers(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instances = supervisor.instance_store.list();
    let mut servers = Vec::new();

    for instance in instances {
        // ── Extension hook: server.list_enrich ──
        // Extension이 Docker 상태/통계를 제공할 수 있음 (TTL 캐시 적용)
        let mut ext_handled = false;
        if instance.use_docker {
            // 캐시 확인 — TTL 내 결과가 있으면 Python 프로세스 스폰 생략
            let enrich_result = if let Some(cached_val) = state.docker_status_cache.get(&instance.id) {
                cached_val
            } else {
                let ext_mgr = state.extension_manager.clone();
                // 모듈 메타데이터에서 Docker 프로세스 패턴 조회
                let process_patterns = supervisor.module_loader.get_module(&instance.module_name)
                    .map(|m| m.metadata.docker_process_patterns.clone())
                    .unwrap_or_default();
                let ctx = serde_json::json!({
                    "instance_id": &instance.id,
                    "instance_name": &instance.name,
                    "module": &instance.module_name,
                    "use_docker": instance.use_docker,
                    "extension_data": &instance.extension_data,
                    "instance_dir": supervisor.instance_store.instance_dir(&instance.id).to_string_lossy(),
                    "process_patterns": process_patterns,
                });
                let mgr = ext_mgr.read().await;
                let results = mgr.dispatch_hook("server.list_enrich", ctx).await;
                // 첫 번째 성공 응답을 캐시 (handled 여부 무관)
                let result = results.into_iter()
                    .find_map(|(_id, r)| r.ok())
                    .unwrap_or_else(|| json!({"handled": false}));
                state.docker_status_cache.set(&instance.id, result.clone());
                result
            };

            // handled: true인 경우만 Extension이 서버 정보를 제공
            let is_handled = enrich_result.get("handled")
                .and_then(|h| h.as_bool()) == Some(true);

            if is_handled {
                let res = &enrich_result;
                // Extension이 서버 정보를 완전히 제공
                let ext_status = res.get("status").and_then(|s| s.as_str())
                    .unwrap_or("stopped").to_string();
                let provisioning = state.provision_tracker.get(&instance.name)
                    .map(|p| !p.done)
                    .unwrap_or(false);
                let status = if provisioning { "provisioning".to_string() } else { ext_status };
                let last_api = state.api_actions.get(&instance.name)
                    .or_else(|| state.api_actions.get(&instance.id));

                let docker_memory_usage = res.get("memory_usage").and_then(|v| v.as_str()).map(|s| s.to_string());
                let docker_memory_percent = res.get("memory_percent").and_then(|v| v.as_f64());
                let docker_cpu_percent = res.get("cpu_percent").and_then(|v| v.as_f64());

                servers.push(ServerInfo {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    module: instance.module_name.clone(),
                    status,
                    pid: None,
                    start_time: None,
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
                    provisioning,
                    use_docker: instance.use_docker,
                    docker_memory_usage,
                    docker_memory_percent,
                    docker_cpu_percent,
                    docker_cpu_limit: instance.docker_cpu_limit,
                    docker_memory_limit: instance.docker_memory_limit.clone(),
                    extension_data: instance.extension_data.clone(),
                    port_conflicts: vec![],
                });
                ext_handled = true;
            }
        }
        if ext_handled {
            continue;
        }

        // Native 모드: ProcessTracker에서 PID 및 시작 시간 확인
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

        // Docker 프로비저닝 진행 여부 확인
        let provisioning = state.provision_tracker.get(&instance.name)
            .map(|p| !p.done)
            .unwrap_or(false);

        // 프로비저닝 중이면 상태를 "provisioning"으로 오버라이드
        let status = if provisioning { "provisioning".to_string() } else { status };

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
            provisioning,
            use_docker: instance.use_docker,
            docker_memory_usage: None,
            docker_memory_percent: None,
            docker_cpu_percent: None,
            docker_cpu_limit: instance.docker_cpu_limit,
            docker_memory_limit: instance.docker_memory_limit.clone(),
            extension_data: instance.extension_data.clone(),
            port_conflicts: vec![],
        });
    }

    // ── 포트 충돌 계산: 모든 서버 간 포트 겹침을 검사하여 per-server 정보 채움 ──
    {
        // 포트 → (server_index, port_type) 매핑
        let mut port_map: std::collections::HashMap<u16, Vec<(usize, &str)>> = std::collections::HashMap::new();
        for (idx, srv) in servers.iter().enumerate() {
            for (p, pt) in [
                (srv.port, "port"),
                (srv.rcon_port, "rcon_port"),
                (srv.rest_port, "rest_port"),
            ] {
                if let Some(port) = p {
                    port_map.entry(port).or_default().push((idx, pt));
                }
            }
        }
        // 2개 이상 겹치는 포트에 대해 충돌 정보 추가
        for (&port, entries) in &port_map {
            if entries.len() < 2 { continue; }
            for &(idx, pt) in entries {
                let mut conflicts_for_this: Vec<PortConflictInfo> = Vec::new();
                for &(other_idx, other_pt) in entries {
                    if other_idx == idx { continue; }
                    conflicts_for_this.push(PortConflictInfo {
                        port,
                        port_type: pt.to_string(),
                        conflict_name: servers[other_idx].name.clone(),
                        conflict_id: servers[other_idx].id.clone(),
                        conflict_port_type: other_pt.to_string(),
                    });
                }
                servers[idx].port_conflicts.extend(conflicts_for_this);
            }
        }
    }

    // 포트 충돌 강제 정지 이벤트 drain
    let port_conflict_stops = supervisor.drain_port_conflict_stops();

    Json(ServerListResponse { servers, port_conflict_stops })
}

/// GET /api/modules - 모든 모듈 목록
pub async fn list_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.list_modules() {
        Ok(modules) => {
            let module_infos: Vec<ExtensionInfo> = modules
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

                    ExtensionInfo {
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
            (StatusCode::OK, Json(ExtensionListResponse { modules: module_infos })).into_response()
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
            let module_infos: Vec<ExtensionInfo> = modules
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

                    ExtensionInfo {
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
            (StatusCode::OK, Json(ExtensionListResponse { modules: module_infos })).into_response()
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
        let error = json!({
            "error": format!("Server '{}' not found", name),
            "error_code": "instance_not_found",
        });
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
    // Docker 상태 캐시 무효화 — 시작 직후 fresh 상태 반영
    {
        let sup = state.supervisor.read().await;
        if let Some(inst) = sup.instance_store.list().iter().find(|i| i.name == name) {
            state.docker_status_cache.invalidate(&inst.id);
        }
    }
    let supervisor = state.supervisor.read().await;

    match supervisor
        .start_server(&name, &payload.module, payload.config)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let msg = e.to_string();
            let (code, error_code) = if msg.contains("not found") {
                (StatusCode::NOT_FOUND, "instance_not_found")
            } else if msg.contains("jar") || msg.contains("executable") {
                (StatusCode::UNPROCESSABLE_ENTITY, "executable_missing")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "start_failed")
            };
            let error = json!({
                "error": format!("Failed to start server: {}", msg),
                "error_code": error_code,
            });
            (code, Json(error)).into_response()
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
    // Docker 상태 캐시 무효화 — 정지 직후 fresh 상태 반영
    {
        let sup = state.supervisor.read().await;
        if let Some(inst) = sup.instance_store.list().iter().find(|i| i.name == name) {
            state.docker_status_cache.invalidate(&inst.id);
        }
    }

    // instance에서 모듈명과 ID 조회 (read lock으로 조회 후 즉시 해제)
    let instance = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .list()
            .iter()
            .find(|i| i.name == name)
            .cloned()
    };

    if let Some(inst) = instance {
        // instance id로도 API 액션 기록
        state.api_actions.record(&inst.id);
        let mut supervisor = state.supervisor.write().await;
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
                    // Docker 모드에서는 ProcessTracker를 사용하지 않으므로 skip
                    if !inst.use_docker {
                        // name과 id 둘 다로 untrack 시도
                        let _ = supervisor.tracker.untrack(&name);
                        if let Err(e) = supervisor.tracker.untrack(&inst.id) {
                            tracing::warn!("Failed to untrack stopped server '{}': {}", name, e);
                        } else {
                            tracing::info!("Server '{}' untracked from process tracker", name);
                        }
                    }
                }
                (StatusCode::OK, Json(result)).into_response()
            }
            Err(e) => {
                let msg = e.to_string();
                let (code, error_code) = if msg.contains("not found") {
                    (StatusCode::NOT_FOUND, "instance_not_found")
                } else {
                    (StatusCode::INTERNAL_SERVER_ERROR, "stop_failed")
                };
                let error = json!({
                    "error": format!("Failed to stop server: {}", msg),
                    "error_code": error_code,
                });
                (code, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({
            "error": format!("Server '{}' not found", name),
            "error_code": "instance_not_found",
        });
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}
