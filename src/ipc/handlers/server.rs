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

/// GET /health - 데몬 활성 확인용 경량 ping (인증 없음)
///
/// supervisor lock이나 디스크 I/O 없이 즉시 응답합니다. 버전 외의 정보는 노출하지 않는다.
/// 데몬 listen 확인용. 인증을 거치지 않으므로 토큰 mismatch 상황도 200을 반환한다
/// (실제 호출 가능 여부는 `/api/ping`으로 판정).
pub async fn health_check() -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    (StatusCode::OK, Json(json!({ "ok": true, "version": version })))
}

/// GET /api/ping - 인증된 경량 ping
///
/// `/health` 와 달리 auth_middleware 를 통과하므로 IPC 토큰 mismatch 시 401 을 반환합니다.
/// GUI 의 Background 배지가 "실제로 호출 가능한 상태" 인지를 판정하는 용도로 사용됩니다.
pub async fn auth_ping() -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    (StatusCode::OK, Json(json!({ "ok": true, "version": version })))
}

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반)
pub async fn list_servers(State(state): State<IPCServer>) -> impl IntoResponse {
    // ── Phase 1: supervisor lock을 최소 시간만 잡고 필요한 데이터 복사 ──
    struct InstanceSnapshot {
        id: String,
        name: String,
        module_name: String,
        executable_path: Option<String>,
        working_dir: Option<String>,
        port: Option<u16>,
        rcon_port: Option<u16>,
        rcon_password: Option<String>,
        rest_host: Option<String>,
        rest_port: Option<u16>,
        rest_username: Option<String>,
        rest_password: Option<String>,
        protocol_mode: String,
        module_settings: std::collections::HashMap<String, serde_json::Value>,
        server_version: Option<String>,
        extension_data: std::collections::HashMap<String, serde_json::Value>,
        instance_dir: String,
        process_patterns: Vec<String>,
        // Native 모드 데이터
        pid: Option<u32>,
        start_time: Option<u64>,
    }

    let snapshots: Vec<InstanceSnapshot> = {
        let supervisor = state.supervisor.read().await;
        let instances = supervisor.instance_store.list();
        instances.iter().map(|instance| {
            let pid = supervisor.tracker.get_pid(&instance.id).ok();
            let start_time = if pid.is_some() {
                supervisor.tracker.get_start_time(&instance.id).ok()
            } else {
                None
            };
            let process_patterns = supervisor.module_loader.get_module(&instance.module_name)
                .map(|m| m.metadata.process_patterns.clone())
                .unwrap_or_default();
            InstanceSnapshot {
                id: instance.id.clone(),
                name: instance.name.clone(),
                module_name: instance.module_name.clone(),
                executable_path: instance.executable_path.clone(),
                working_dir: instance.working_dir.clone(),
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
                extension_data: instance.extension_data.clone(),
                instance_dir: supervisor.instance_store.instance_dir(&instance.id).to_string_lossy().to_string(),
                process_patterns,
                pid,
                start_time,
            }
        }).collect()
    };
    // supervisor read lock 해제됨 — 이후 extension dispatch에 의해 블로킹되어도
    // 다른 API 핸들러가 supervisor write lock을 획득할 수 있음

    // ── Phase 2: Extension hooks (lock 없이 실행) ──
    let mut servers = Vec::new();

    for instance in &snapshots {
        // ── Extension hook: server.list_enrich ──
        // Extension이 런타임 상태/통계를 제공할 수 있음 (TTL 캐시 적용)
        let mut ext_handled = false;
        let has_ext_hooks = !instance.extension_data.is_empty();
        if has_ext_hooks {
            // 캐시 확인 — TTL 내 결과가 있으면 Python 프로세스 스폰 생략
            let enrich_result = if let Some(cached_val) = state.extension_status_cache.get(&instance.id) {
                cached_val
            } else {
                let ext_mgr = state.extension_manager.clone();
                let ctx = serde_json::json!({
                    "instance_id": &instance.id,
                    "instance_name": &instance.name,
                    "module": &instance.module_name,
                    "extension_data": &instance.extension_data,
                    "instance_dir": &instance.instance_dir,
                    "process_patterns": &instance.process_patterns,
                });
                let mgr = ext_mgr.read().await;
                let results = mgr.dispatch_hook_timed("server.list_enrich", ctx, 10).await;
                // 첫 번째 성공 응답을 캐시 (handled 여부 무관)
                let result = results.into_iter()
                    .find_map(|(_id, r)| r.ok())
                    .unwrap_or_else(|| json!({"handled": false}));
                state.extension_status_cache.set(&instance.id, result.clone());
                result
            };

            // handled: true인 경우만 Extension이 서버 정보를 제공
            let is_handled = enrich_result.get("handled")
                .and_then(|h| h.as_bool()) == Some(true);

            if is_handled {
                let res = &enrich_result;
                let ext_status = res.get("status").and_then(|s| s.as_str())
                    .unwrap_or("stopped").to_string();
                let provision_entry = state.provision_tracker.get(&instance.name);
                let provisioning = provision_entry.is_some();
                let actively_provisioning = provision_entry.as_ref().map(|p| !p.done).unwrap_or(false);
                let status = if actively_provisioning { "provisioning".to_string() } else { ext_status };
                let last_api = state.api_actions.get(&instance.name)
                    .or_else(|| state.api_actions.get(&instance.id));

                // Extension 런타임 상태를 extension_status에 저장
                let mut extension_status = std::collections::HashMap::new();
                // hook 응답에서 extension_id 필드가 있으면 해당 키로, 아니면 "default"
                let ext_key = res.get("extension_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                extension_status.insert(ext_key, enrich_result.clone());

                servers.push(ServerInfo {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    module: instance.module_name.clone(),
                    status,
                    pid: None,
                    start_time: None,
                    executable_path: instance.executable_path.clone(),
                    working_dir: instance.working_dir.clone(),
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
                    extension_status,
                    extension_data: instance.extension_data.clone(),
                    port_conflicts: vec![],
                });
                ext_handled = true;
            }
        }
        if ext_handled {
            continue;
        }

        // Native 모드: 스냅샷에서 PID 및 시작 시간 사용
        let (status, start_time) = if instance.pid.is_some() {
            ("running".to_string(), instance.start_time)
        } else {
            ("stopped".to_string(), None)
        };

        // API(CLI/Discord)를 통한 마지막 시작/정지 타임스탬프 조회
        let last_api = state.api_actions.get(&instance.name)
            .or_else(|| state.api_actions.get(&instance.id));

        // 프로비저닝 진행 여부 확인 (tracker entry 존재 = provisioning UI 표시)
        let provision_entry = state.provision_tracker.get(&instance.name);
        let provisioning = provision_entry.is_some();
        let actively_provisioning = provision_entry.as_ref().map(|p| !p.done).unwrap_or(false);

        // 프로비저닝 중이면 상태를 "provisioning"으로 오버라이드
        let status = if actively_provisioning { "provisioning".to_string() } else { status };

        servers.push(ServerInfo {
            id: instance.id.clone(),
            name: instance.name.clone(),
            module: instance.module_name.clone(),
            status,
            pid: instance.pid,
            start_time,
            executable_path: instance.executable_path.clone(),
            working_dir: instance.working_dir.clone(),
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
            extension_status: std::collections::HashMap::new(),
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

    // 포트 충돌 강제 정지 이벤트 drain (짧은 read lock)
    let port_conflict_stops = {
        let supervisor = state.supervisor.read().await;
        supervisor.drain_port_conflict_stops()
    };

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
                        dir_signatures: m.metadata.dir_signatures,
                        install: m.metadata.install,
                        module_aliases: m.metadata.module_aliases,
                        command_aliases: m.metadata.command_aliases,
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
                        dir_signatures: m.metadata.dir_signatures,
                        install: m.metadata.install,
                        module_aliases: m.metadata.module_aliases,
                        command_aliases: m.metadata.command_aliases,
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

/// GET /api/modules/manifest - Relay 서명 매니페스트에서 설치 가능한 모듈 목록 반환
///
/// 흐름: RelayClient `/v1/updates/check` + `/v1/updates/manifest/:version` (Ed25519 검증)
/// → `components` 중 `module-*` 키만 추출 → GUI 호환 스키마 `{ manifest: { modules: { id: info } } }` 로 변환.
pub async fn fetch_module_manifest() -> impl IntoResponse {
    use saba_chan_updater_lib::{UpdateConfig, UpdateManager};

    let cfg = UpdateConfig::default();
    let modules_dir = saba_chan_updater_lib::constants::resolve_modules_dir();
    let modules_dir_str = modules_dir.to_string_lossy().to_string();
    let mut mgr = UpdateManager::new(cfg, &modules_dir_str);

    match mgr.fetch_remote_manifest().await {
        Ok(manifest) => {
            let mut modules = serde_json::Map::new();
            for (key, info) in &manifest.components {
                let Some(id) = key.strip_prefix("module-") else {
                    continue;
                };
                let mut entry = serde_json::Map::new();
                entry.insert("version".into(), json!(info.version));
                if let Some(asset) = &info.asset {
                    entry.insert("asset".into(), json!(asset));
                }
                if let Some(url) = &info.url {
                    entry.insert("url".into(), json!(url));
                }
                if let Some(sha) = &info.sha256 {
                    entry.insert("sha256".into(), json!(sha));
                }
                if let Some(size) = info.size_bytes {
                    entry.insert("size_bytes".into(), json!(size));
                }
                if let Some(dir) = &info.install_dir {
                    entry.insert("install_dir".into(), json!(dir));
                }
                // GUI 호환: display_name/description 은 매니페스트에 없으면 id 그대로
                entry.insert("display_name".into(), json!(id));
                modules.insert(id.to_string(), serde_json::Value::Object(entry));
            }
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "manifest": {
                        "release_version": manifest.version,
                        "modules": modules,
                    }
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::OK,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /api/modules/manifest/:id/install - Relay 서명 매니페스트로부터 모듈 설치
///
/// 흐름: UpdateManager::install_component(Module(id)) 위임
/// → Relay 매니페스트 fetch + Ed25519 검증 → R2 다운로드 + SHA256 검증 → 압축 해제.
pub async fn install_module_from_manifest(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    use saba_chan_updater_lib::{Component, UpdateConfig, UpdateManager};

    // 경로 탐색 방지
    if module_id.contains("..") || module_id.contains('/') || module_id.contains('\\') {
        return (
            StatusCode::OK,
            Json(json!({ "ok": false, "error": "Invalid module ID" })),
        )
            .into_response();
    }

    let cfg = UpdateConfig::default();
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        std::path::PathBuf::from(supervisor.module_loader.modules_dir())
    };
    let modules_dir_str = modules_dir.to_string_lossy().to_string();
    let mut mgr = UpdateManager::new(cfg, &modules_dir_str);

    let component = Component::Module(module_id.clone());
    match mgr.install_component(&component).await {
        Ok(install_path) => {
            let supervisor = state.supervisor.read().await;
            let _ = supervisor.refresh_modules();
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "module_id": module_id,
                    "install_path": install_path,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::OK,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /api/modules/:id — 모듈 제거 (디렉토리 삭제 + 캐시 갱신)
pub async fn remove_module(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 경로 탐색 방지
    if module_id.contains("..") || module_id.contains('/') || module_id.contains('\\') {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "Invalid module ID" }))).into_response();
    }
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        std::path::PathBuf::from(supervisor.module_loader.modules_dir())
    };
    let module_path = modules_dir.join(&module_id);
    if !module_path.exists() {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "Module not found" }))).into_response();
    }
    if let Err(e) = std::fs::remove_dir_all(&module_path) {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": e.to_string() }))).into_response();
    }
    // 캐시 갱신 (write lock)
    let supervisor = state.supervisor.write().await;
    let _ = supervisor.refresh_modules();
    (StatusCode::OK, Json(json!({ "ok": true, "id": module_id }))).into_response()
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
    // 익스텐션 상태 캐시 무효화 — 시작 직후 fresh 상태 반영
    {
        let sup = state.supervisor.read().await;
        if let Some(inst) = sup.instance_store.list().iter().find(|i| i.name == name) {
            state.extension_status_cache.invalidate(&inst.id);
        }
    }

    // ── 실행 전 비밀번호 자동 생성 (비어있으면 채움) ──
    {
        let mut supervisor = state.supervisor.write().await;
        let inst_opt = supervisor.instance_store.list().iter()
            .find(|i| i.name == name)
            .cloned();
        if let Some(mut inst) = inst_opt {
            let changed = inst.ensure_passwords();

            if changed {
                let inst_id = inst.id.clone();
                if let Err(e) = supervisor.instance_store.update(&inst_id, inst) {
                    tracing::warn!("Failed to save auto-generated passwords for {}: {}", name, e);
                }
            }
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
    // 익스텐션 상태 캐시 무효화 — 정지 직후 fresh 상태 반영
    {
        let sup = state.supervisor.read().await;
        if let Some(inst) = sup.instance_store.list().iter().find(|i| i.name == name) {
            state.extension_status_cache.invalidate(&inst.id);
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
                    // RCON 연결 풀에서 해당 인스턴스 연결 제거
                    state.rcon_pool.remove(&inst.id);

                    // 익스텐션이 외부 프로세스 관리(예: 컨테이너)를 사용하면 ProcessTracker skip
                    if inst.extension_data.is_empty() || !inst.ext_enabled("docker_enabled") {
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

// NOTE: sync_gui_config は config::save_gui_config に統合されました (portConflictCheck 同期含む)