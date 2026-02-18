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
            let error = json!({ "error": format!("Instance not found: {}", id) });
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
                // ── Docker 프로비저닝: 백그라운드로 실행 (fire-and-forget) ──
                if use_docker {
                    let tracker = state.provision_tracker.clone();
                    // 즉시 tracker 초기화 — list_servers가 provisioning 상태를 인식하도록
                    tracker.update(&instance_name, super::super::ProvisionProgress {
                        step: 0, total: 3,
                        label: "docker_engine".to_string(),
                        message: "Preparing...".to_string(),
                        done: false, error: None,
                        percent: None,
                    });

                    let inst_clone = instance.clone();
                    let dir_clone = instance_dir.clone();
                    tokio::spawn(async move {
                        match docker_provision(
                            &inst_clone,
                            &dir_clone,
                            module_install,
                            &tracker,
                        ).await {
                            Ok(msg) => tracing::info!(
                                "Docker provisioning complete for '{}': {}",
                                inst_clone.name, msg
                            ),
                            Err(e) => tracing::error!(
                                "Docker provisioning failed for '{}': {}",
                                inst_clone.name, e
                            ),
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

/// Docker 프로비저닝 파이프라인:
/// 1) Docker 사용 가능 확인 (없으면 자동 설치 시도)
/// 2) SteamCMD로 서버 파일 다운로드 (install.method == "steamcmd"일 때)
/// 3) docker-compose.yml 생성
async fn docker_provision(
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    module_config: Option<(
        Option<crate::supervisor::module_loader::ModuleInstallConfig>,
        Option<crate::supervisor::module_loader::DockerExtensionConfig>,
    )>,
    tracker: &super::super::ProvisionTracker,
) -> Result<String, String> {
    let key = &instance.name;
    let total: u8 = 3;

    // Helper to update progress
    let progress = |step: u8, label: &str, message: &str| {
        tracker.update(key, super::super::ProvisionProgress {
            step, total,
            label: label.to_string(),
            message: message.to_string(),
            done: false, error: None,
            percent: None,
        });
    };
    let progress_pct = |step: u8, label: &str, message: &str, pct: u8| {
        tracker.update(key, super::super::ProvisionProgress {
            step, total,
            label: label.to_string(),
            message: message.to_string(),
            done: false, error: None,
            percent: Some(pct),
        });
    };
    let finish_ok = |message: &str| {
        tracker.update(key, super::super::ProvisionProgress {
            step: total, total,
            label: "done".to_string(),
            message: message.to_string(),
            done: true, error: None,
            percent: None,
        });
    };
    let finish_err = |step: u8, label: &str, err: &str| {
        tracker.update(key, super::super::ProvisionProgress {
            step, total,
            label: label.to_string(),
            message: err.to_string(),
            done: true, error: Some(err.to_string()),
            percent: None,
        });
    };

    let (install_config, docker_config): (
        Option<crate::supervisor::module_loader::ModuleInstallConfig>,
        Option<crate::supervisor::module_loader::DockerExtensionConfig>,
    ) = module_config
        .ok_or_else(|| "모듈 정보를 찾을 수 없습니다".to_string())?;
    let docker_config: crate::supervisor::module_loader::DockerExtensionConfig = docker_config
        .ok_or_else(|| "이 모듈에는 [docker] 설정이 정의되어 있지 않습니다".to_string())?;

    // ── Step 1: Docker 사용 가능 확인 ──
    progress(0, "docker_engine", "Checking Docker Engine...");
    if !crate::docker::is_docker_available() || !crate::docker::is_docker_daemon_running() {
        tracing::info!("Docker not ready — ensuring portable Docker Engine...");
        progress(0, "docker_engine", "Downloading and starting Docker Engine...");

        let tracker_d = tracker.clone();
        let key_d = key.clone();
        let result = crate::docker::ensure_docker_engine_with_progress(move |p| {
            if let (Some(pct), Some(msg)) = (p.percent, &p.message) {
                tracker_d.update(&key_d, super::super::ProvisionProgress {
                    step: 0, total,
                    label: "docker_engine".to_string(),
                    message: msg.clone(),
                    done: false, error: None,
                    percent: Some(pct),
                });
            }
        }).await;

        if !result.daemon_ready {
            let err = format!("Docker를 사용할 수 없습니다: {}", result.message);
            finish_err(0, "docker_engine", &err);
            return Err(err);
        }
    }

    // ── Step 2: SteamCMD로 서버 파일 다운로드 ──
    progress(1, "steamcmd", "Preparing server files...");
    let server_dir = instance_dir.join("server");
    std::fs::create_dir_all(&server_dir)
        .map_err(|e| {
            let err = format!("서버 디렉토리 생성 실패: {}", e);
            finish_err(1, "steamcmd", &err);
            err
        })?;

    if let Some(ref install) = install_config {
        if install.method == "steamcmd" {
            if let Some(app_id) = install.app_id {
                tracing::info!(
                    "SteamCMD: 서버 파일 다운로드 시작 (app_id: {}, dir: {})",
                    app_id,
                    server_dir.display()
                );
                progress(1, "steamcmd", &format!("Downloading server files (app {})...", app_id));
                let install_dir_str = server_dir.to_string_lossy().to_string();
                // Docker mode uses Linux containers -- force Linux platform
                // even if module.toml specifies "windows" for native mode.
                let platform = if crate::docker::is_wsl2_mode() {
                    Some("linux".to_string())
                } else {
                    install.platform.clone()
                };
                let steamcmd_config = serde_json::json!({
                    "app_id": app_id,
                    "install_dir": install_dir_str,
                    "anonymous": install.anonymous,
                    "platform": platform,
                    "beta": install.beta,
                });
                // SteamCMD Python 확장을 통해 설치 실행
                let tracker_s = tracker.clone();
                let key_s = key.clone();
                match crate::plugin::run_extension_with_progress(
                    "steamcmd", "install", steamcmd_config,
                    move |p| {
                        if let (Some(pct), Some(msg)) = (p.percent, &p.message) {
                            tracker_s.update(&key_s, super::super::ProvisionProgress {
                                step: 1, total,
                                label: "steamcmd".to_string(),
                                message: msg.clone(),
                                done: false, error: None,
                                percent: Some(pct),
                            });
                        }
                    },
                ).await {
                    Ok(result) => {
                        let success = result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
                        if !success {
                            let msg = result.get("error")
                                .or_else(|| result.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("SteamCMD 설치 실패");
                            let err = format!("SteamCMD 설치 실패: {}", msg);
                            finish_err(1, "steamcmd", &err);
                            return Err(err);
                        }
                        tracing::info!("SteamCMD: 서버 파일 다운로드 완료");
                    }
                    Err(e) => {
                        let err = format!("SteamCMD 실행 실패: {}", e);
                        finish_err(1, "steamcmd", &err);
                        return Err(err);
                    }
                }
            }
        }
    }

    // ── Step 3: docker-compose.yml 생성 ──
    progress(2, "compose", "Generating docker-compose.yml...");
    let extra_vars: std::collections::HashMap<String, String> = instance
        .module_settings
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();

    let ctx = crate::docker::ComposeTemplateContext {
        instance_id: instance.id.clone(),
        instance_name: instance.name.clone(),
        module_name: instance.module_name.clone(),
        port: instance.port,
        rcon_port: instance.rcon_port,
        rest_port: instance.rest_port,
        rest_password: instance.rest_password.clone(),
        extra_vars,
    };

    crate::docker::provision_compose_file(instance_dir, &docker_config, &ctx)
        .map_err(|e| {
            let err = format!("docker-compose.yml 생성 실패: {}", e);
            finish_err(2, "compose", &err);
            err
        })?;

    let msg = "Docker 프로비저닝 완료: docker-compose.yml 생성됨".to_string();
    finish_ok(&msg);
    Ok(msg)
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

    // Docker 인스턴스라면 먼저 컨테이너를 정리 (docker compose down)
    if let Some(instance) = supervisor.instance_store.get(&id) {
        if instance.use_docker {
            let instance_dir = supervisor.instance_store.instance_dir(&id);
            let docker_mgr = crate::docker::DockerComposeManager::new(&instance_dir, None);
            if docker_mgr.has_compose_file() {
                tracing::info!("Cleaning up Docker containers for instance {}", id);
                let _ = docker_mgr.down().await;
            }
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

    // Docker 인스턴스: docker-compose.yml 재생성 (리소스 제한 반영)
    if updated_clone.use_docker {
        if let Some(module) = supervisor.list_modules().ok().and_then(|mods| {
            mods.into_iter().find(|m| m.metadata.name == updated_clone.module_name)
        }) {
            if let Some(ref mut docker_config) = module.metadata.docker.clone() {
                // 인스턴스별 오버라이드 적용
                if updated_clone.docker_cpu_limit.is_some() {
                    docker_config.cpu_limit = updated_clone.docker_cpu_limit;
                }
                if updated_clone.docker_memory_limit.is_some() {
                    docker_config.memory_limit = updated_clone.docker_memory_limit.clone();
                }

                let extra_vars: std::collections::HashMap<String, String> = updated_clone
                    .module_settings
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect();

                let ctx = crate::docker::ComposeTemplateContext {
                    instance_id: updated_clone.id.clone(),
                    instance_name: updated_clone.name.clone(),
                    module_name: updated_clone.module_name.clone(),
                    port: updated_clone.port,
                    rcon_port: updated_clone.rcon_port,
                    rest_port: updated_clone.rest_port,
                    rest_password: updated_clone.rest_password.clone(),
                    extra_vars,
                };

                let instance_dir = supervisor.instance_store.instance_dir(&id);
                match crate::docker::provision_compose_file(&instance_dir, docker_config, &ctx) {
                    Ok(path) => tracing::info!("Regenerated docker-compose.yml at {}", path.display()),
                    Err(e) => tracing::warn!("Failed to regenerate docker-compose.yml: {}", e),
                }
            }
        }
    }

    (StatusCode::OK, Json(json!({ "success": true }))).into_response()
}
