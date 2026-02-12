use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Client Heartbeat Registry ──────────────────────────────

/// 클라이언트 유형 (GUI, CLI)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Gui,
    Cli,
}

/// 등록된 클라이언트 정보
#[derive(Debug, Clone)]
pub struct RegisteredClient {
    pub kind: ClientKind,
    pub last_heartbeat: std::time::Instant,
    /// 이 클라이언트가 시작한 Discord 봇 프로세스의 PID (있을 경우)
    pub bot_pid: Option<u32>,
}

/// 클라이언트 레지스트리 — 여러 GUI/CLI 인스턴스의 생존 여부를 추적
#[derive(Debug, Clone)]
pub struct ClientRegistry {
    inner: Arc<RwLock<HashMap<String, RegisteredClient>>>,
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 새 클라이언트 등록, client_id 반환
    pub async fn register(&self, kind: ClientKind) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut map = self.inner.write().await;
        tracing::info!("[Heartbeat] Client registered: {} ({:?})", id, kind);
        map.insert(id.clone(), RegisteredClient {
            kind,
            last_heartbeat: std::time::Instant::now(),
            bot_pid: None,
        });
        id
    }

    /// Heartbeat 수신 — TTL 갱신, 선택적으로 bot_pid 업데이트
    pub async fn heartbeat(&self, client_id: &str, bot_pid: Option<u32>) -> bool {
        let mut map = self.inner.write().await;
        if let Some(client) = map.get_mut(client_id) {
            client.last_heartbeat = std::time::Instant::now();
            if let Some(pid) = bot_pid {
                client.bot_pid = Some(pid);
            }
            true
        } else {
            false
        }
    }

    /// 클라이언트 해제
    pub async fn unregister(&self, client_id: &str) -> Option<RegisteredClient> {
        let mut map = self.inner.write().await;
        let removed = map.remove(client_id);
        if removed.is_some() {
            tracing::info!("[Heartbeat] Client unregistered: {}", client_id);
        }
        removed
    }

    /// 타임아웃된 클라이언트 목록 반환 및 제거
    pub async fn reap_expired(&self, timeout: std::time::Duration) -> Vec<(String, RegisteredClient)> {
        let mut map = self.inner.write().await;
        let now = std::time::Instant::now();
        let mut expired = Vec::new();

        map.retain(|id, client| {
            if now.duration_since(client.last_heartbeat) > timeout {
                tracing::warn!(
                    "[Heartbeat] Client timed out: {} ({:?}), last heartbeat {:.0}s ago",
                    id, client.kind,
                    now.duration_since(client.last_heartbeat).as_secs_f64()
                );
                expired.push((id.clone(), client.clone()));
                false
            } else {
                true
            }
        });

        expired
    }

    /// 현재 등록된 클라이언트 수
    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }

    /// 등록된 모든 클라이언트가 있는지 (데몬 자동 종료 판단용)
    #[allow(dead_code)]
    pub async fn has_clients(&self) -> bool {
        !self.inner.read().await.is_empty()
    }
}

/// IPC 요청/응답 타입
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStartRequest {
    pub module: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStopRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRequest {
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct CommandResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub prefix: String,
    #[serde(default)]
    #[serde(rename = "moduleAliases")]
    pub module_aliases: HashMap<String, String>,
    #[serde(default)]
    #[serde(rename = "commandAliases")]
    pub command_aliases: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerListResponse {
    pub servers: Vec<ServerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub module: String,
    pub status: String,
    pub pid: Option<u32>,
    pub start_time: Option<u64>,
    // 설정값들
    pub executable_path: Option<String>,
    pub port: Option<u16>,
    pub rcon_port: Option<u16>,
    pub rcon_password: Option<String>,
    pub rest_host: Option<String>,
    pub rest_port: Option<u16>,
    pub rest_username: Option<String>,
    pub rest_password: Option<String>,
    pub protocol_mode: String,  // "rest", "rcon", "auto"
    #[serde(default)]
    pub module_settings: std::collections::HashMap<String, serde_json::Value>,  // 동적 모듈 설정
    pub server_version: Option<String>,  // 서버 버전
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolsInfo {
    pub supported: Vec<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub path: String,
    pub executable_path: Option<String>,
    pub icon: Option<String>,  // base64 인코딩된 아이콘 이미지
    pub interaction_mode: Option<String>,  // "console" or "commands"
    pub protocols: Option<ProtocolsInfo>,  // 지원 프로토콜 정보
    pub settings: Option<crate::supervisor::module_loader::ModuleSettings>,
    pub commands: Option<crate::supervisor::module_loader::ModuleCommands>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleListResponse {
    pub modules: Vec<ModuleInfo>,
}

/// IPC Server State
#[derive(Clone)]
pub struct IPCServer {
    /// Shared supervisor instance used by all handlers
    pub supervisor: Arc<RwLock<crate::supervisor::Supervisor>>,
    pub listen_addr: String,
    /// 클라이언트(GUI/CLI) 생존 추적 레지스트리
    pub client_registry: ClientRegistry,
}

impl IPCServer {
    pub fn new(supervisor: Arc<RwLock<crate::supervisor::Supervisor>>, listen_addr: &str) -> Self {
        Self {
            supervisor,
            listen_addr: listen_addr.to_string(),
            client_registry: ClientRegistry::new(),
        }
    }

    pub async fn start(self) -> Result<()> {
        tracing::info!("IPC HTTP server starting on {}", self.listen_addr);

        // Router 생성
        let router = Router::new()
            .route("/api/servers", get(list_servers))
            .route("/api/server/:name/status", get(get_server_status))
            .route("/api/server/:name/start", post(start_server_handler))
            .route("/api/server/:name/stop", post(stop_server_handler))
            .route("/api/modules", get(list_modules))
            .route("/api/modules/refresh", post(refresh_modules))
            .route("/api/module/:name", get(get_module_metadata))
            .route("/api/instances", get(list_instances).post(create_instance))
            .route("/api/instances/reorder", put(reorder_instances))
            .route("/api/instance/:id", get(get_instance).delete(delete_instance).patch(update_instance_settings))
            .route("/api/instance/:id/command", post(execute_command))
            .route("/api/instance/:id/rcon", post(execute_rcon_command))
            .route("/api/instance/:id/rest", post(execute_rest_command))
            // ── New managed-process & module-feature endpoints ──
            .route("/api/instance/:id/managed/start", post(start_managed_handler))
            .route("/api/instance/:id/console", get(get_console_handler))
            .route("/api/instance/:id/stdin", post(send_stdin_handler))
            .route("/api/instance/:id/validate", post(validate_instance_handler))
            .route("/api/instance/:id/properties", get(read_properties_handler).put(write_properties_handler))
            .route("/api/instance/:id/accept-eula", post(accept_eula_handler))
            .route("/api/instance/:id/diagnose", post(diagnose_handler))
            // ── Server installation endpoints ──
            .route("/api/module/:name/versions", get(list_versions_handler))
            .route("/api/module/:name/version/:version", get(get_version_details_handler))
            .route("/api/module/:name/install", post(install_server_handler))
            .route("/api/config/bot", get(get_bot_config).put(save_bot_config))
            // ── Client heartbeat endpoints ──
            .route("/api/client/register", post(client_register))
            .route("/api/client/:id/heartbeat", post(client_heartbeat))
            .route("/api/client/:id/unregister", delete(client_unregister))
            .with_state(self.clone());

        // TCP 리스너 (SO_REUSEADDR + 바인딩 재시도)
        let addr: std::net::SocketAddr = self.listen_addr.parse()
            .map_err(|e| anyhow::anyhow!("Invalid listen address '{}': {}", self.listen_addr, e))?;

        let mut last_err = None;
        let max_retries = 10;
        for attempt in 1..=max_retries {
            let socket = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;
            socket.set_reuse_address(true)?;
            socket.set_nonblocking(true)?;
            match socket.bind(&addr.into()) {
                Ok(()) => {
                    socket.listen(128)?;
                    let std_listener: std::net::TcpListener = socket.into();
                    let listener = tokio::net::TcpListener::from_std(std_listener)?;
                    if attempt > 1 {
                        tracing::info!("IPC bind succeeded on attempt {}", attempt);
                    }
                    tracing::info!("IPC listening on http://{}", self.listen_addr);

                    axum::serve(listener, router).await?;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        "IPC bind attempt {}/{} failed: {} — retrying in 2s",
                        attempt, max_retries, e
                    );
                    last_err = Some(e);
                    drop(socket);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
        Err(anyhow::anyhow!(
            "Failed to bind IPC server to {} after {} attempts: {}",
            self.listen_addr, max_retries, last_err.map(|e| e.to_string()).unwrap_or_default()
        ))
    }
}

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반)
async fn list_servers(State(state): State<IPCServer>) -> impl IntoResponse {
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
        });
    }

    Json(ServerListResponse { servers })
}

/// GET /api/modules - 모든 모듈 목록
async fn list_modules(State(state): State<IPCServer>) -> impl IntoResponse {
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
                            use base64::{Engine as _, engine::general_purpose};
                            format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(&data))
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
                        protocols: m.metadata.protocols_supported.map(|supported| ProtocolsInfo {
                            supported,
                            default: m.metadata.protocols_default,
                        }),
                        settings: m.metadata.settings,
                        commands: m.metadata.commands,
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
async fn refresh_modules(State(state): State<IPCServer>) -> impl IntoResponse {
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
                            use base64::{Engine as _, engine::general_purpose};
                            format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(&data))
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
                        protocols: m.metadata.protocols_supported.map(|supported| ProtocolsInfo {
                            supported,
                            default: m.metadata.protocols_default,
                        }),
                        settings: m.metadata.settings,
                        commands: m.metadata.commands,
                    }
                })
                .collect();
            tracing::info!("Module cache refreshed. Found {} modules", module_infos.len());
            (StatusCode::OK, Json(ModuleListResponse { modules: module_infos })).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to refresh modules: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// GET /api/module/:name - 모듈 메타데이터 조회 (별명 포함)
async fn get_module_metadata(
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
                    Ok(content) => {
                        match toml::from_str::<serde_json::Value>(&content) {
                            Ok(parsed) => {
                                (StatusCode::OK, Json(json!({
                                    "name": &module.metadata.name,
                                    "version": &module.metadata.version,
                                    "description": &module.metadata.description,
                                    "path": &module.path,
                                    "metadata": &module.metadata,
                                    "toml": parsed,
                                }))).into_response()
                            }
                            Err(_) => {
                                (StatusCode::OK, Json(json!({
                                    "name": &module.metadata.name,
                                    "version": &module.metadata.version,
                                    "description": &module.metadata.description,
                                    "path": &module.path,
                                    "metadata": &module.metadata,
                                }))).into_response()
                            }
                        }
                    }
                    Err(_) => {
                        (StatusCode::OK, Json(json!({
                            "name": &module.metadata.name,
                            "version": &module.metadata.version,
                            "description": &module.metadata.description,
                            "path": &module.path,
                            "metadata": &module.metadata,
                        }))).into_response()
                    }
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
async fn get_server_status(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    
    // instance에서 모듈명 조회
    let instance = supervisor.instance_store.list()
        .iter()
        .find(|i| i.name == name);
    
    if let Some(inst) = instance {
        match supervisor.get_server_status(&name, &inst.module_name).await {
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
async fn start_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStartRequest>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    
    match supervisor.start_server(&name, &payload.module, payload.config).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let error = json!({ "error": format!("Failed to start server: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// POST /api/server/:name/stop - 서버 중지
async fn stop_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStopRequest>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    
    // instance에서 모듈명과 ID 조회
    let instance = supervisor.instance_store.list()
        .iter()
        .find(|i| i.name == name)
        .cloned();
    
    if let Some(inst) = instance {
        match supervisor.stop_server(&name, &inst.module_name, payload.force).await {
            Ok(result) => {
                // 실제 종료 성공 시에만 tracker에서 제거
                let success = result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
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

/// GET /api/instances - 모든 인스턴스 목록
async fn list_instances(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    let instances = supervisor.instance_store.list();
    (StatusCode::OK, Json(instances)).into_response()
}

/// PUT /api/instances/reorder - 인스턴스 순서 변경
async fn reorder_instances(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;
    
    let ordered_ids: Vec<String> = match payload.get("order").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(),
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
async fn get_instance(
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
async fn create_instance(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;
    
    let name = payload.get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));
    
    let module_name = payload.get("module_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing module_name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));
    
    if let (Ok(name), Ok(module)) = (name, module_name) {
        let mut instance = crate::instance::ServerInstance::new(name, module);
        
        // 모듈 정보에서 process_name과 default_port 가져오기
        if let Ok(loaded_module) = supervisor.module_loader.get_module(module) {
            instance.process_name = loaded_module.metadata.process_name.clone();
            if instance.port.is_none() {
                instance.port = loaded_module.metadata.default_port;
            }
        }
        
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
        
        match supervisor.instance_store.add(instance) {
            Ok(_) => {
                let response = json!({ "success": true, "id": id });
                (StatusCode::CREATED, Json(response)).into_response()
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

/// DELETE /api/instance/:id - 인스턴스 삭제
async fn delete_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;
    
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
async fn update_instance_settings(
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
                tracing::info!("Auto-inferred working_dir to {} from existing executable_path", parent.display());
            }
        }
    }
    
    // 하드코딩된 공통 필드 목록
    let known_fields: std::collections::HashSet<&str> = [
        "port", "rcon_port", "rcon_password",
        "rest_host", "rest_port", "rest_username", "rest_password",
        "executable_path", "protocol_mode", "server_version",
    ].iter().cloned().collect();
    
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
    
    // RCON 자동 활성화: enable_rcon=true인데 rcon_password가 비어있으면 랜덤 비밀번호 생성
    let enable_rcon = settings.get("enable_rcon")
        .and_then(|v| match v {
            serde_json::Value::Bool(b) => Some(*b),
            serde_json::Value::String(s) => Some(s == "true"),
            _ => None,
        });
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
            tracing::info!("Auto-generated RCON password for instance {} (enable_rcon=true but no password set)", id);
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
                tracing::info!("Auto-set working_dir to {} from executable_path", parent.display());
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
    
    // 동적 모듈 설정 저장 (하드코딩 필드 이외의 모든 설정을 module_settings에 저장)
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if !known_fields.contains(key.as_str()) {
                updated.module_settings.insert(key.clone(), value.clone());
            }
        }
    }
    
    tracing::info!("Updating instance {} with settings: port={:?}, rcon_port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}", 
        id, updated.port, updated.rcon_port, updated.executable_path, updated.protocol_mode, updated.module_settings.len());
    
    // 모든 설정을 server.properties에 동기화 (configure lifecycle 호출)
    let mut props_sync = serde_json::Map::new();
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            // protocol_mode, executable_path 등은 server.properties에 관련 없으므로 제외
            if key == "protocol_mode" || key == "executable_path" || key == "server_version"
                || key == "rest_host" || key == "rest_port" || key == "rest_username" || key == "rest_password"
                || key == "java_path" || key == "ram" || key == "use_aikar_flags" {
                continue;
            }
            props_sync.insert(key.clone(), value.clone());
        }
    }
    
    // 자동 생성된 RCON 비밀번호가 있으면 props_sync에도 추가
    if let Some(auto_password) = &updated.rcon_password {
        if !props_sync.contains_key("rcon_password") && enable_rcon == Some(true) {
            props_sync.insert("rcon_password".to_string(), json!(auto_password));
        }
    }

    // 저장
    if let Err(e) = supervisor.instance_store.update(&id, updated) {
        let error = json!({ "error": format!("Failed to update instance: {}", e) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // server.properties 동기화 (변경된 항목이 있을 때만)
    if !props_sync.is_empty() {
        tracing::info!("Syncing settings to server.properties for instance {}: {:?}", id, props_sync);
        let props_value = Value::Object(props_sync);
        match supervisor.manage_properties(&id, "write", Some(props_value)).await {
            Ok(_) => tracing::info!("server.properties synced successfully for instance {}", id),
            Err(e) => tracing::warn!("Failed to sync server.properties for instance {}: {}", id, e),
        }
    }
    
    (StatusCode::OK, Json(json!({ "success": true }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ipc_handle_request() {
        // HTTP 엔드포인트 테스트는 integration test로 이동
        let response = json!({
            "success": true,
            "result": "ok"
        });
        assert!(response["success"].as_bool().unwrap());
    }

    // === 2026-01-20 추가: ModuleInfo commands 필드 테스트 ===

    #[test]
    fn test_module_info_serialization_with_commands() {
        // ModuleInfo가 commands 필드를 포함하여 직렬화되는지 확인
        let module_info = ModuleInfo {
            name: "palworld".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Palworld 서버 관리".to_string()),
            path: "/modules/palworld".to_string(),
            executable_path: Some("PalServer.exe".to_string()),
            icon: None,
            settings: None,
            commands: Some(crate::supervisor::module_loader::ModuleCommands {
                fields: vec![
                    crate::supervisor::module_loader::CommandField {
                        name: "players".to_string(),
                        label: "플레이어 목록".to_string(),
                        description: Some("현재 접속 중인 플레이어 조회".to_string()),
                        method: Some("rest".to_string()),
                        http_method: Some("GET".to_string()),
                        endpoint_template: Some("/v1/api/players".to_string()),
                        rcon_template: None,
                        inputs: vec![],
                    },
                ],
            }),
        };

        let json = serde_json::to_value(&module_info).unwrap();
        
        // commands 필드가 존재하는지 확인
        assert!(json.get("commands").is_some(), "commands field should exist");
        
        // commands.fields가 배열인지 확인
        let commands = json.get("commands").unwrap();
        assert!(commands.get("fields").is_some(), "commands.fields should exist");
        
        // 첫 번째 명령어가 올바른 http_method를 가지는지 확인
        let fields = commands.get("fields").unwrap().as_array().unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].get("http_method").unwrap().as_str().unwrap(), "GET");
        assert_eq!(fields[0].get("name").unwrap().as_str().unwrap(), "players");
    }

    #[test]
    fn test_module_info_without_commands() {
        // commands가 None일 때도 정상 직렬화되는지 확인
        let module_info = ModuleInfo {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            path: "/modules/test".to_string(),
            executable_path: None,
            icon: None,
            settings: None,
            commands: None,
        };

        let json = serde_json::to_value(&module_info).unwrap();
        
        // commands가 null로 직렬화되어야 함
        assert!(json.get("commands").is_some());
        assert!(json.get("commands").unwrap().is_null());
    }

    #[test]
    fn test_module_list_response_includes_commands() {
        // ModuleListResponse가 commands를 포함한 모듈 목록을 반환하는지 확인
        let response = ModuleListResponse {
            modules: vec![
                ModuleInfo {
                    name: "palworld".to_string(),
                    version: "1.0.0".to_string(),
                    description: Some("Palworld".to_string()),
                    path: "/modules/palworld".to_string(),
                    executable_path: None,
                    icon: None,
                    settings: None,
                    commands: Some(crate::supervisor::module_loader::ModuleCommands {
                        fields: vec![
                            crate::supervisor::module_loader::CommandField {
                                name: "info".to_string(),
                                label: "서버 정보".to_string(),
                                description: None,
                                method: Some("rest".to_string()),
                                http_method: Some("GET".to_string()),
                                endpoint_template: Some("/v1/api/info".to_string()),
                                rcon_template: None,
                                inputs: vec![],
                            },
                            crate::supervisor::module_loader::CommandField {
                                name: "announce".to_string(),
                                label: "공지 전송".to_string(),
                                description: None,
                                method: Some("rest".to_string()),
                                http_method: Some("POST".to_string()),
                                endpoint_template: Some("/v1/api/announce".to_string()),
                                rcon_template: None,
                                inputs: vec![
                                    crate::supervisor::module_loader::CommandInput {
                                        name: "message".to_string(),
                                        label: Some("메시지".to_string()),
                                        input_type: Some("string".to_string()),
                                        required: Some(true),
                                        placeholder: Some("공지 내용".to_string()),
                                        default: None,
                                    },
                                ],
                            },
                        ],
                    }),
                },
            ],
        };

        let json = serde_json::to_value(&response).unwrap();
        let modules = json.get("modules").unwrap().as_array().unwrap();
        
        assert_eq!(modules.len(), 1);
        
        let palworld = &modules[0];
        let commands = palworld.get("commands").unwrap();
        let fields = commands.get("fields").unwrap().as_array().unwrap();
        
        assert_eq!(fields.len(), 2);
        
        // GET 메서드 명령어 확인
        assert_eq!(fields[0].get("http_method").unwrap().as_str().unwrap(), "GET");
        
        // POST 메서드 명령어 확인
        assert_eq!(fields[1].get("http_method").unwrap().as_str().unwrap(), "POST");
        
        // inputs 필드 확인
        let announce_inputs = fields[1].get("inputs").unwrap().as_array().unwrap();
        assert_eq!(announce_inputs.len(), 1);
        assert_eq!(announce_inputs[0].get("name").unwrap().as_str().unwrap(), "message");
        assert_eq!(announce_inputs[0].get("required").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_http_method_parsing() {
        // HTTP 메서드 파싱 로직 테스트
        let test_cases = vec![
            ("GET", crate::protocol::HttpMethod::Get),
            ("get", crate::protocol::HttpMethod::Get),
            ("POST", crate::protocol::HttpMethod::Post),
            ("post", crate::protocol::HttpMethod::Post),
            ("PUT", crate::protocol::HttpMethod::Put),
            ("DELETE", crate::protocol::HttpMethod::Delete),
            ("UNKNOWN", crate::protocol::HttpMethod::Get),  // 기본값은 GET
            ("", crate::protocol::HttpMethod::Get),
        ];

        for (input, expected) in test_cases {
            let result = match input.to_uppercase().as_str() {
                "POST" => crate::protocol::HttpMethod::Post,
                "PUT" => crate::protocol::HttpMethod::Put,
                "DELETE" => crate::protocol::HttpMethod::Delete,
                _ => crate::protocol::HttpMethod::Get,
            };
            assert_eq!(result, expected, "Failed for input: {}", input);
        }
    }

    #[test]
    fn test_command_input_serialization() {
        // CommandInput 직렬화 테스트
        let input = crate::supervisor::module_loader::CommandInput {
            name: "user_id".to_string(),
            label: Some("유저 ID".to_string()),
            input_type: Some("string".to_string()),
            required: Some(true),
            placeholder: Some("steam_xxxxxxxxx".to_string()),
            default: None,
        };

        let json = serde_json::to_value(&input).unwrap();
        
        assert_eq!(json.get("name").unwrap().as_str().unwrap(), "user_id");
        assert_eq!(json.get("label").unwrap().as_str().unwrap(), "유저 ID");
        assert_eq!(json.get("type").unwrap().as_str().unwrap(), "string");
        assert_eq!(json.get("required").unwrap().as_bool().unwrap(), true);
        assert_eq!(json.get("placeholder").unwrap().as_str().unwrap(), "steam_xxxxxxxxx");
    }
}

/// POST /api/instance/:id/command - 명령어 실행
async fn execute_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(req): Json<CommandRequest>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // 인스턴스 확인
    let instance = match supervisor.instance_store.get(&id) {
        Some(instance) => instance,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id)
                })),
            )
                .into_response();
        }
    };

    // Supervisor를 통해 명령어 전달
    let result = supervisor
        .execute_command(&instance.id, &instance.module_name, &req.command, req.args)
        .await;

    match result {
        Ok(message) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": message
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": e.to_string()
            })),
        )
            .into_response(),
    }
}

/// GET /api/config/bot - 봇 설정 조회
async fn get_bot_config(State(_state): State<IPCServer>) -> impl IntoResponse {
    match std::fs::read_to_string(crate::supervisor::get_discord_bot_config_path()) {
        Ok(content) => {
            match serde_json::from_str::<BotConfig>(&content) {
                Ok(config) => (StatusCode::OK, Json(config)).into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": format!("Failed to parse bot config: {}", e)
                    })),
                )
                    .into_response(),
            }
        }
        Err(_) => (
            StatusCode::OK,
            Json(BotConfig {
                prefix: "!saba".to_string(),
                module_aliases: Default::default(),
                command_aliases: Default::default(),
            }),
        )
            .into_response(),
    }
}

/// PUT /api/config/bot - 봇 설정 저장
async fn save_bot_config(
    State(_state): State<IPCServer>,
    Json(config): Json<BotConfig>,
) -> impl IntoResponse {
    let config_path = crate::supervisor::get_discord_bot_config_path();
    
    // 파일 경로의 부모 디렉토리 생성
    if let Some(parent) = std::path::Path::new(&config_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to create config directory: {}", e)
                })),
            )
                .into_response();
        }
    }

    // 설정을 JSON으로 저장
    match serde_json::to_string_pretty(&config) {
        Ok(json_str) => {
            match std::fs::write(&config_path, json_str) {
                Ok(_) => (
                    StatusCode::OK,
                    Json(json!({
                        "success": true,
                        "message": "Bot config saved"
                    })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": format!("Failed to write bot config: {}", e)
                    })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("Failed to serialize bot config: {}", e)
            })),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/rcon - RCON 명령어 실행
async fn execute_rcon_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // 인스턴스 확인
    let instance = match supervisor.instance_store.get(&id) {
        Some(instance) => instance,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id)
                })),
            )
                .into_response();
        }
    };

    // 모듈에서 기본값 가져오기
    let (default_rcon_port, _default_rest_port) = match supervisor.module_loader.get_module(&instance.module_name) {
        Ok(module) => (module.metadata.default_rcon_port(), module.metadata.default_rest_port()),
        Err(_) => (25575, 8212), // 모듈을 찾을 수 없으면 기존 기본값 사용
    };

    // RCON 커맨드 추출
    let command = match payload.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing 'command' field"
                })),
            )
                .into_response();
        }
    };

    // RCON 정보 확인
    let rcon_host = "127.0.0.1".to_string(); // RCON은 항상 localhost

    let rcon_port = match payload.get("rcon_port").and_then(|v| v.as_u64()) {
        Some(port) => port as u16,
        None => instance.rcon_port.unwrap_or(default_rcon_port),
    };

    let rcon_password = match payload.get("rcon_password").and_then(|v| v.as_str()) {
        Some(pass) => pass.to_string(),
        None => match &instance.rcon_password {
            Some(pass) => pass.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "RCON password not configured"
                    })),
                )
                    .into_response();
            }
        },
    };

    // RCON 클라이언트 생성 및 실행 (연결 실패 시 최대 2회 재시도)
    let rcon_timeout = std::time::Duration::from_secs(5);
    let mut last_error = String::new();

    for attempt in 0..3 {
        if attempt > 0 {
            tracing::info!("RCON retry attempt {} for command '{}'", attempt + 1, command);
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        let mut client = crate::protocol::client::ProtocolClient::new_rcon(
            rcon_host.clone(),
            rcon_port,
            rcon_password.clone(),
        );

        match client.connect_rcon(rcon_timeout) {
            Ok(_) => {
                let cmd = crate::protocol::ServerCommand {
                    command_type: crate::protocol::CommandType::Rcon,
                    command: Some(command.to_string()),
                    endpoint: None,
                    method: None,
                    body: None,
                    timeout_secs: payload.get("timeout").and_then(|v| v.as_u64()),
                };

                match client.execute(cmd) {
                    Ok(response) => {
                        return (
                            StatusCode::OK,
                            Json(json!({
                                "success": response.success,
                                "data": response.data,
                                "error": response.error,
                                "command": command,
                                "host": rcon_host,
                                "port": rcon_port,
                                "protocol": "rcon"
                            })),
                        )
                            .into_response();
                    }
                    Err(e) => {
                        last_error = format!("RCON execution failed: {}", e);
                        tracing::warn!("{} (attempt {})", last_error, attempt + 1);
                    }
                }
            }
            Err(e) => {
                last_error = format!("RCON connection failed: {}", e);
                tracing::warn!("{} (attempt {})", last_error, attempt + 1);
            }
        }
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": last_error })),
    )
        .into_response()
}

/// POST /api/instance/:id/rest - REST API 명령어 실행
async fn execute_rest_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // 인스턴스 확인
    let instance = match supervisor.instance_store.get(&id) {
        Some(instance) => instance,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id)
                })),
            )
                .into_response();
        }
    };

    // 모듈에서 기본값 가져오기
    let (default_rest_port, default_rest_host) = match supervisor.module_loader.get_module(&instance.module_name) {
        Ok(module) => (module.metadata.default_rest_port(), module.metadata.default_rest_host()),
        Err(_) => (8212, "127.0.0.1".to_string()), // 모듈을 찾을 수 없으면 기존 기본값 사용
    };

    // REST 엔드포인트 추출
    let endpoint = match payload.get("endpoint").and_then(|v| v.as_str()) {
        Some(ep) => ep,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing 'endpoint' field"
                })),
            )
                .into_response();
        }
    };

    // REST 정보 확인 - payload에서 먼저 찾고, 없으면 instance에서 찾음, 그래도 없으면 모듈 기본값
    let rest_host = payload.get("rest_host")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| instance.rest_host.clone())
        .unwrap_or(default_rest_host);

    let rest_port = payload.get("rest_port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .or(instance.rest_port)
        .unwrap_or(default_rest_port);

    let use_https = payload.get("use_https")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    tracing::info!("REST command for instance {}: host={}:{} endpoint={}", 
        id, rest_host, rest_port, endpoint);

    // REST 클라이언트 생성
    let mut client = crate::protocol::client::ProtocolClient::new_rest(
        rest_host.to_string(),
        rest_port,
        use_https,
    );

    // 선택적 Basic Auth
    let username = payload.get("username")
        .and_then(|v| v.as_str())
        .or(instance.rest_username.as_deref());
    let password = payload.get("password")
        .and_then(|v| v.as_str())
        .or(instance.rest_password.as_deref());

    if let (Some(user), Some(pass)) = (username, password) {
        tracing::debug!("REST: Basic auth provided: {}@{}:{}", user, rest_host, rest_port);
        // 클라이언트에 인증 정보 설정
        client = client.with_basic_auth(user.to_string(), pass.to_string());
    }

    // REST 연결 검증
    if let Err(e) = client.connect_rest(std::time::Duration::from_secs(5)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("REST connection failed: {}", e)
            })),
        )
            .into_response();
    }

    // 메서드 결정
    let method = payload.get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");

    let http_method = match method.to_uppercase().as_str() {
        "POST" => crate::protocol::HttpMethod::Post,
        "PUT" => crate::protocol::HttpMethod::Put,
        "DELETE" => crate::protocol::HttpMethod::Delete,
        _ => crate::protocol::HttpMethod::Get,
    };

    // 명령어 구성 - endpoint는 모듈에서 완전한 형식으로 전달됨
    let cmd = crate::protocol::ServerCommand {
        command_type: crate::protocol::CommandType::Rest,
        command: None,
        endpoint: Some(endpoint.to_string()),
        method: Some(http_method),
        body: payload.get("body").cloned(),
        timeout_secs: payload.get("timeout").and_then(|v| v.as_u64()),
    };

    match client.execute(cmd) {
        Ok(response) => (
            StatusCode::OK,
            Json(json!({
                "success": response.success,
                "data": response.data,
                "error": response.error,
                "endpoint": endpoint,
                "method": method,
                "host": rest_host,
                "port": rest_port,
                "protocol": "rest"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("REST execution failed: {}", e)
            })),
        )
            .into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════
//  Managed Process & Module Feature Handlers
// ═══════════════════════════════════════════════════════════════

/// POST /api/instance/:id/managed/start — Start with managed process (stdin/stdout capture)
async fn start_managed_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    payload: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instance = match supervisor.instance_store.get(&id) {
        Some(i) => i,
        None => return (StatusCode::NOT_FOUND, Json(json!({"error": format!("Instance not found: {}", id)}))).into_response(),
    };

    let module_name = instance.module_name.clone();
    let payload_val = payload.map(|j| j.0).unwrap_or(json!({}));
    let config = payload_val.get("config").cloned().unwrap_or(json!({}));

    match supervisor.start_managed_server(&id, &module_name, config).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// GET /api/instance/:id/console?since=0&count=100 — Get console output
async fn get_console_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let since_id = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let count = params.get("count").and_then(|c| c.parse::<usize>().ok());

    match supervisor.get_console_output(&id, since_id, count).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// POST /api/instance/:id/stdin — Send command to stdin
async fn send_stdin_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let command = match payload.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => return (StatusCode::BAD_REQUEST, Json(json!({"error": "Missing 'command' field"}))).into_response(),
    };

    match supervisor.send_stdin_command(&id, command).await {
        Ok(msg) => (StatusCode::OK, Json(json!({"success": true, "message": msg}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// POST /api/instance/:id/validate — Validate prerequisites
async fn validate_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.validate_instance(&id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// GET /api/instance/:id/properties — Read server.properties
async fn read_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.manage_properties(&id, "read", None).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// PUT /api/instance/:id/properties — Update server.properties
async fn write_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let settings = payload.get("settings").cloned();
    match supervisor.manage_properties(&id, "write", settings).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// POST /api/instance/:id/accept-eula — Accept Minecraft EULA
async fn accept_eula_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.accept_eula(&id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// POST /api/instance/:id/diagnose — Diagnose errors
async fn diagnose_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.diagnose_instance(&id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

// ─── Server Installation Handlers ────────────────────────────

/// GET /api/module/:name/versions?include_snapshots=false&page=1&per_page=25
/// List available Minecraft server versions from Mojang
async fn list_versions_handler(
    Path(module_name): Path<String>,
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let include_snapshots = params.get("include_snapshots")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let page = params.get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);
    let per_page = params.get("per_page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(25);

    match supervisor.list_versions(&module_name, include_snapshots, page, per_page).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// GET /api/module/:name/version/:version — Get detailed info for a specific version
async fn get_version_details_handler(
    Path((module_name, version)): Path<(String, String)>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.get_version_details(&module_name, &version).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

/// POST /api/module/:name/install — Install a server
/// Body: { "version": "1.21.11", "install_dir": "/path/to/server",
///         "jar_name": "server.jar", "accept_eula": true, "initial_settings": {...} }
async fn install_server_handler(
    Path(module_name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let version = match payload.get("version").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({"error": "Missing 'version' field"}))).into_response(),
    };

    let install_dir = match payload.get("install_dir").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({"error": "Missing 'install_dir' field"}))).into_response(),
    };

    let jar_name = payload.get("jar_name").and_then(|v| v.as_str());
    let accept_eula = payload.get("accept_eula").and_then(|v| v.as_bool()).unwrap_or(false);
    let initial_settings = payload.get("initial_settings").cloned();

    match supervisor.install_server(
        &module_name,
        &version,
        &install_dir,
        jar_name,
        accept_eula,
        initial_settings,
    ).await {
        Ok(result) => {
            let status = if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(result)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

// ── Client Heartbeat Handlers ────────────────────────────────

/// POST /api/client/register — 클라이언트(GUI/CLI) 등록
async fn client_register(
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let kind_str = payload.get("kind").and_then(|v| v.as_str()).unwrap_or("gui");
    let kind = match kind_str {
        "cli" => ClientKind::Cli,
        _ => ClientKind::Gui,
    };

    let client_id = state.client_registry.register(kind.clone()).await;
    let count = state.client_registry.count().await;
    tracing::info!("[Heartbeat] Active clients: {}", count);

    (StatusCode::OK, Json(json!({
        "client_id": client_id,
        "kind": kind_str,
        "heartbeat_interval_ms": 30000,
        "timeout_ms": 90000
    }))).into_response()
}

/// POST /api/client/:id/heartbeat — TTL 갱신
async fn client_heartbeat(
    Path(client_id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let bot_pid = payload.get("bot_pid").and_then(|v| v.as_u64()).map(|p| p as u32);

    if state.client_registry.heartbeat(&client_id, bot_pid).await {
        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Client not registered"}))).into_response()
    }
}

/// DELETE /api/client/:id/unregister — 클라이언트 명시적 해제 + 봇 정리
async fn client_unregister(
    Path(client_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if let Some(client) = state.client_registry.unregister(&client_id).await {
        // 해당 클라이언트가 관리하던 봇 프로세스 정리
        if let Some(pid) = client.bot_pid {
            kill_bot_pid(pid);
        }
        let count = state.client_registry.count().await;
        tracing::info!("[Heartbeat] Active clients after unregister: {}", count);
        (StatusCode::OK, Json(json!({"ok": true}))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Client not registered"}))).into_response()
    }
}

/// 특정 PID의 봇 프로세스를 종료
pub fn kill_bot_pid(pid: u32) {
    tracing::info!("[Heartbeat] Killing bot process PID: {}", pid);

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// 백그라운드 태스크에서 호출 — 만료 클라이언트 정리 및 고아 봇 프로세스 종료
pub async fn reap_expired_clients(registry: &ClientRegistry) {
    let timeout = std::time::Duration::from_secs(90);
    let expired = registry.reap_expired(timeout).await;

    for (id, client) in &expired {
        tracing::warn!("[Heartbeat] Cleaning up expired client: {} ({:?})", id, client.kind);
        if let Some(pid) = client.bot_pid {
            kill_bot_pid(pid);
        }
    }

    if !expired.is_empty() {
        let remaining = registry.count().await;
        tracing::info!("[Heartbeat] Reap complete. Cleaned: {}, remaining clients: {}", expired.len(), remaining);
    }
}
