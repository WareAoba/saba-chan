use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{
    IPCServer, ExtensionInfo, ExtensionListResponse, ProtocolsInfo, ServerInfo, ServerListResponse,
    ServerStartRequest, ServerStopRequest,
};

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반)
pub async fn list_servers(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instances = supervisor.instance_store.list();
    let mut servers = Vec::new();

    // Docker 인스턴스가 있으면 WSL2 모드가 올바르게 설정되어 있는지 확인
    let has_docker_instances = instances.iter().any(|i| i.use_docker);
    if has_docker_instances && !crate::docker::is_wsl2_mode() {
        // 가볍게 확인: ensure_docker_engine은 이미 실행 중이면 즉시 반환
        let _ = crate::docker::ensure_docker_engine().await;
    }

    for instance in instances {
        let (status, start_time, pid) = if instance.use_docker {
            // Docker 모드: 컨테이너 상태 + 내부 게임 서버 프로세스 확인
            let instance_dir = supervisor.instance_store.instance_dir(&instance.id);
            let docker_mgr = crate::docker::DockerComposeManager::new(&instance_dir, None);
            if docker_mgr.has_compose_file() {
                match docker_mgr.status().await {
                    Ok(st) => {
                        let container_running = st.get("running")
                            .and_then(|r| r.as_bool())
                            .unwrap_or(false);

                        // 컨테이너가 실행 중이면 내부 프로세스도 확인
                        let status = if container_running {
                            let patterns = supervisor.module_loader.get_module(&instance.module_name)
                                .map(|m| m.metadata.docker_process_patterns.clone())
                                .unwrap_or_default();
                            let container_name = st.get("container_name")
                                .and_then(|n| n.as_str());
                            if let Some(name) = container_name {
                                let (server_ok, _) = docker_mgr.server_process_running(name, &patterns).await;
                                if server_ok { "running" } else { "starting" }
                            } else {
                                "running" // 컨테이너 이름 못 가져오면 fallback
                            }
                        } else {
                            "stopped"
                        };
                        (status.to_string(), None, None)
                    }
                    Err(_) => ("stopped".to_string(), None, None),
                }
            } else {
                ("stopped".to_string(), None, None)
            }
        } else {
            // Native 모드: ProcessTracker에서 PID 및 시작 시간 확인
            let pid = supervisor.tracker.get_pid(&instance.id).ok();
            let (status, start_time) = if pid.is_some() {
                let st = supervisor.tracker.get_start_time(&instance.id).ok();
                ("running".to_string(), st)
            } else {
                ("stopped".to_string(), None)
            };
            (status, start_time, pid)
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

        // Docker 리소스 사용량 수집 (running 상태인 Docker 인스턴스만, 5초 캐시)
        let (docker_memory_usage, docker_memory_percent, docker_cpu_percent) =
            if instance.use_docker && status == "running" {
                // 컨테이너 이름 규칙: saba-{module}-{instance_id_short}
                let short_id = &instance.id[..8.min(instance.id.len())];
                let container_name = format!("saba-{}-{}", instance.module_name, short_id);

                // 캐시에서 먼저 확인
                if let Some(cached) = state.docker_stats_cache.get(&container_name) {
                    cached
                } else if state.docker_stats_cache.is_expired() {
                    // 캐시 만료 → 비동기로 새 통계 수집
                    match crate::docker::docker_container_stats(&container_name).await {
                        Ok(stats) => {
                            let mem_usage = stats.get("MemUsage")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let mem_pct = stats.get("MemPerc")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.trim_end_matches('%').parse::<f64>().ok());
                            let cpu_pct = stats.get("CPUPerc")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.trim_end_matches('%').parse::<f64>().ok());
                            state.docker_stats_cache.update(&container_name, mem_usage.clone(), mem_pct, cpu_pct);
                            (mem_usage, mem_pct, cpu_pct)
                        }
                        Err(e) => {
                            tracing::debug!("docker stats failed for {}: {}", container_name, e);
                            (None, None, None)
                        }
                    }
                } else {
                    // 캐시가 아직 유효하지만 이 컨테이너만 없는 경우
                    (None, None, None)
                }
            } else {
                (None, None, None)
            };

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
            docker_memory_usage,
            docker_memory_percent,
            docker_cpu_percent,
            docker_cpu_limit: instance.docker_cpu_limit,
            docker_memory_limit: instance.docker_memory_limit.clone(),
        });
    }

    Json(ServerListResponse { servers })
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
                let error = json!({ "error": format!("Failed to stop server: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": format!("Server '{}' not found", name) });
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}
