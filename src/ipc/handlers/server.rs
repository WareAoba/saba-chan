use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::{
    IPCServer, ExtensionInfo, ExtensionListResponse, PortConflictInfo, ProtocolsInfo, ServerInfo, ServerListResponse,
    ServerStartRequest, ServerStopRequest,
};

/// GET /health - 데몬 활성 확인용 경량 ping
///
/// supervisor lock이나 디스크 I/O 없이 즉시 응답합니다.
/// `daemon:status` 폴링 및 데몬 ready 체크에 사용됩니다.
pub async fn health_check() -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    (StatusCode::OK, Json(json!({ "ok": true, "version": version, "pid": std::process::id() })))
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

    let (snapshots, all_instances, module_protocols): (Vec<InstanceSnapshot>, Vec<_>, _) = {
        let supervisor = state.supervisor.read().await;
        let instances = supervisor.instance_store.list();
        let all_instances = instances.to_vec();
        let module_protocols = supervisor.build_module_protocols_map();
        let snapshots =         instances.iter().map(|instance| {
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
        }).collect();
        (snapshots, all_instances, module_protocols)
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

    // ── 포트 충돌 계산: 시작 시 검사(check_port_conflicts)와 같은 기준 ──
    // 모듈이 지원하지 않는 프로토콜의 포트는 제외한다. 목록은 "켜면 부딪힐" 서버를
    // 미리 보여주는 용도라 정지된 서버끼리도 비교한다(시작 시에는 실행 중인 것만).
    {
        let every_id: std::collections::HashSet<String> =
            all_instances.iter().map(|i| i.id.clone()).collect();
        for srv in servers.iter_mut() {
            let Some(target) = all_instances.iter().find(|i| i.id == srv.id) else { continue };
            srv.port_conflicts = crate::validator::check_port_conflicts(
                target,
                &all_instances,
                &every_id,
                Some(&module_protocols),
            )
            .into_iter()
            .map(|c| PortConflictInfo {
                port: c.port,
                port_type: c.port_type,
                conflict_name: c.conflicting_instance_name,
                conflict_id: c.conflicting_instance_id,
                conflict_port_type: c.conflicting_port_type,
            })
            .collect();
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
/// 모듈 하나를 API 응답 모양으로 (아이콘은 base64 data URL로)
fn module_info(m: crate::supervisor::module_loader::LoadedModule) -> ExtensionInfo {
    let icon_base64 = m.metadata.icon.as_ref().and_then(|icon_file| {
        let icon_path = std::path::Path::new(&m.path).join(icon_file);
        std::fs::read(&icon_path).ok().map(|data| {
            use base64::{engine::general_purpose, Engine as _};
            format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(&data))
        })
    });
    let meta = m.metadata;
    ExtensionInfo {
        name: meta.name,
        version: meta.version,
        description: meta.description,
        schema_version: meta.schema_version,
        display_name: meta.display_name,
        game_name: meta.game_name,
        author: meta.author,
        warnings: meta.warnings,
        path: m.path,
        executable_path: meta.executable_path,
        icon: icon_base64,
        interaction_mode: meta.interaction_mode,
        protocols: meta.protocols_supported.map(|supported| ProtocolsInfo {
            supported,
            default: meta.protocols_default,
        }),
        settings: meta.settings,
        commands: meta.commands,
        syntax_highlight: meta.syntax_highlight,
        dir_signatures: meta.dir_signatures,
        install: meta.install,
        module_aliases: meta.module_aliases,
        command_aliases: meta.command_aliases,
    }
}

fn module_list_response(
    supervisor: &crate::supervisor::Supervisor,
    modules: anyhow::Result<Vec<crate::supervisor::module_loader::LoadedModule>>,
) -> axum::response::Response {
    match modules {
        Ok(modules) => {
            let modules = modules.into_iter().map(module_info).collect();
            let failed = supervisor.module_loader.load_errors();
            (StatusCode::OK, Json(ExtensionListResponse { modules, failed })).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to list modules: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

pub async fn list_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    module_list_response(&supervisor, supervisor.list_modules())
}

/// POST /api/modules/refresh - 모듈 캐시를 새로고침하고 다시 발견
pub async fn refresh_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    let modules = supervisor.refresh_modules();
    if let Ok(m) = &modules {
        tracing::info!("Module cache refreshed. Found {} modules", m.len());
    }
    module_list_response(&supervisor, modules)
}

/// GET /api/modules/schema - module.toml 스키마 v1 (JSON Schema)
pub async fn module_schema() -> impl IntoResponse {
    Json(crate::supervisor::module_loader::module_json_schema())
}

/// POST /api/modules/detect `{path}` - 이 폴더가 어느 모듈의 게임 서버 폴더인지
/// (`[detection] dir_signatures`가 전부 있는 모듈). 기존 서버 가져오기에서 쓴다.
pub async fn detect_module_dir(
    State(state): State<IPCServer>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(path) = body.get("path").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "path is required" }))).into_response();
    };
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("not a folder: {}", path) }))).into_response();
    }
    let supervisor = state.supervisor.read().await;
    let modules = supervisor.list_modules().unwrap_or_default();
    let matches: Vec<_> = modules
        .iter()
        .filter(|m| m.matches_dir(dir))
        .map(|m| json!({ "name": m.metadata.name, "display_name": m.metadata.display_name }))
        .collect();
    (StatusCode::OK, Json(json!({ "matches": matches }))).into_response()
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

/// GET /api/modules/manifest - saba-chan-modules 최신 릴리스 manifest 가져오기
pub async fn fetch_module_manifest() -> impl IntoResponse {
    let url = saba_chan_updater_lib::constants::modules_manifest_url();
    match reqwest::get(url).await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(data) => (StatusCode::OK, Json(json!({ "ok": true, "manifest": data }))).into_response(),
                Err(e) => (StatusCode::OK, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
            }
        }
        Ok(resp) => {
            (StatusCode::OK, Json(json!({ "ok": false, "error": format!("HTTP {}", resp.status()) }))).into_response()
        }
        Err(e) => (StatusCode::OK, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

/// POST /api/modules/manifest/:id/install - 모듈 매니페스트에서 모듈 설치
pub async fn install_module_from_manifest(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 레지스트리에서 다운로드 URL 구성
    let asset_name = format!("module-{}.zip", module_id);
    let download_url = saba_chan_updater_lib::constants::module_asset_url(&asset_name);

    // 설치 디렉토리 결정
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        std::path::PathBuf::from(supervisor.module_loader.modules_dir())
    };

    let target_dir = modules_dir.join(&module_id);

    // 임시 zip 다운로드
    let zip_path = modules_dir.join(format!("_tmp_module_{}.zip", module_id));

    match reqwest::get(&download_url).await {
        Ok(resp) if resp.status().is_success() => {
            match resp.bytes().await {
                Ok(bytes) => {
                    if let Err(e) = std::fs::write(&zip_path, &bytes) {
                        return (StatusCode::OK, Json(json!({ "ok": false, "error": format!("Failed to write zip: {}", e) }))).into_response();
                    }
                    // zip 압축 해제
                    match std::fs::File::open(&zip_path) {
                        Ok(file) => {
                            match zip::ZipArchive::new(std::io::BufReader::new(file)) {
                                Ok(mut archive) => {
                                    if let Err(e) = archive.extract(&target_dir) {
                                        let _ = std::fs::remove_file(&zip_path);
                                        return (StatusCode::OK, Json(json!({ "ok": false, "error": format!("Failed to extract: {}", e) }))).into_response();
                                    }
                                    let _ = std::fs::remove_file(&zip_path);
                                    // 모듈 캐시 새로고침
                                    let supervisor = state.supervisor.read().await;
                                    let _ = supervisor.refresh_modules();
                                    (StatusCode::OK, Json(json!({ "ok": true, "module_id": module_id }))).into_response()
                                }
                                Err(e) => {
                                    let _ = std::fs::remove_file(&zip_path);
                                    (StatusCode::OK, Json(json!({ "ok": false, "error": format!("Invalid zip: {}", e) }))).into_response()
                                }
                            }
                        }
                        Err(e) => (StatusCode::OK, Json(json!({ "ok": false, "error": format!("Failed to open zip: {}", e) }))).into_response(),
                    }
                }
                Err(e) => (StatusCode::OK, Json(json!({ "ok": false, "error": format!("Failed to read response: {}", e) }))).into_response(),
            }
        }
        Ok(resp) => (StatusCode::OK, Json(json!({ "ok": false, "error": format!("HTTP {}", resp.status()) }))).into_response(),
        Err(e) => (StatusCode::OK, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
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

/// POST /api/instance/:id/start — 서버 시작. 경로(managed / non-managed)는 데몬이 고른다.
///
/// `:id`는 인스턴스 ID, 없으면 이름으로 찾는다(CLI·채팅처럼 이름만 아는 쪽용).
/// 정책은 `Supervisor::uses_managed_start`, 포트 충돌 검사는 전역 설정 하나다.
/// 응답은 고른 경로의 응답 그대로에 `managed`를 더한 것이다.
pub async fn start_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> axum::response::Response {
    let picked = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .get(&id)
            .or_else(|| supervisor.instance_store.list().iter().find(|i| i.name == id))
            .cloned()
            .map(|inst| {
                let managed = supervisor.uses_managed_start(&inst);
                (inst, managed)
            })
    };
    let Some((instance, managed)) = picked else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" })),
        )
            .into_response();
    };

    let resp = if managed {
        crate::ipc::internal::into_internal(
            super::managed::start_managed_handler(Path(instance.id.clone()), State(state.clone()), None).await,
        )
        .await
    } else {
        let req = ServerStartRequest { module: instance.module_name.clone(), config: json!({}) };
        crate::ipc::internal::into_internal(start_server_handler(Path(instance.name.clone()), State(state.clone()), Json(req)).await)
            .await
    };
    let mut body = match resp.body {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("data".into(), other);
            map
        }
    };
    body.insert("managed".into(), json!(managed));
    (resp.status, Json(Value::Object(body))).into_response()
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