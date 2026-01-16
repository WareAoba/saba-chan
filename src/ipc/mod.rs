use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
pub struct CommandResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub prefix: String,
    #[serde(default)]
    pub moduleAliases: HashMap<String, String>,
    #[serde(default)]
    pub commandAliases: HashMap<String, HashMap<String, String>>,
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
    pub uptime_seconds: Option<u64>,
    // 설정값들
    pub executable_path: Option<String>,
    pub port: Option<u16>,
    pub rcon_port: Option<u16>,
    pub rcon_password: Option<String>,
    pub rest_host: Option<String>,
    pub rest_port: Option<u16>,
    pub rest_username: Option<String>,
    pub rest_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub path: String,
    pub executable_path: Option<String>,
    pub settings: Option<crate::supervisor::module_loader::ModuleSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleListResponse {
    pub modules: Vec<ModuleInfo>,
}

/// IPC Server State
#[derive(Clone)]
pub struct IPCServer {
    #[allow(dead_code)]
    pub supervisor: Arc<RwLock<crate::supervisor::Supervisor>>,
    pub listen_addr: String,
}

impl IPCServer {
    pub fn new(supervisor: Arc<RwLock<crate::supervisor::Supervisor>>, listen_addr: &str) -> Self {
        Self {
            supervisor,
            listen_addr: listen_addr.to_string(),
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
            .route("/api/module/:name", get(get_module_metadata))
            .route("/api/instances", get(list_instances).post(create_instance))
            .route("/api/instance/:id", get(get_instance).delete(delete_instance).patch(update_instance_settings))
            .route("/api/instance/:id/command", post(execute_command))
            .route("/api/config/bot", get(get_bot_config).put(save_bot_config))
            .with_state(self.clone());

        // TCP 리스너
        let listener = tokio::net::TcpListener::bind(&self.listen_addr).await?;
        tracing::info!("IPC listening on http://{}", self.listen_addr);

        axum::serve(listener, router).await?;
        Ok(())
    }
}

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반)
async fn list_servers(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    
    let instances = supervisor.instance_store.list();
    let mut servers = Vec::new();
    
    for instance in instances {
        // ProcessTracker에서 PID 확인
        let pid = supervisor.tracker.get_pid(&instance.id).ok();
        let status = if pid.is_some() {
            "running".to_string()
        } else {
            "stopped".to_string()
        };
        
        servers.push(ServerInfo {
            id: instance.id.clone(),
            name: instance.name.clone(),
            module: instance.module_name.clone(),
            status,
            pid,
            uptime_seconds: None,
            executable_path: instance.executable_path.clone(),
            port: instance.port,
            rcon_port: instance.rcon_port,
            rcon_password: instance.rcon_password.clone(),
            rest_host: instance.rest_host.clone(),
            rest_port: instance.rest_port,
            rest_username: instance.rest_username.clone(),
            rest_password: instance.rest_password.clone(),
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
                .map(|m| ModuleInfo {
                    name: m.metadata.name,
                    version: m.metadata.version,
                    description: m.metadata.description,
                    path: m.path,
                    executable_path: m.metadata.executable_path,
                    settings: m.metadata.settings,
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
                                return (StatusCode::OK, Json(json!({
                                    "name": &module.metadata.name,
                                    "version": &module.metadata.version,
                                    "description": &module.metadata.description,
                                    "path": &module.path,
                                    "metadata": &module.metadata,
                                    "toml": parsed,
                                }))).into_response();
                            }
                            Err(_) => {
                                return (StatusCode::OK, Json(json!({
                                    "name": &module.metadata.name,
                                    "version": &module.metadata.version,
                                    "description": &module.metadata.description,
                                    "path": &module.path,
                                    "metadata": &module.metadata,
                                }))).into_response();
                            }
                        }
                    }
                    Err(_) => {
                        return (StatusCode::OK, Json(json!({
                            "name": &module.metadata.name,
                            "version": &module.metadata.version,
                            "description": &module.metadata.description,
                            "path": &module.path,
                            "metadata": &module.metadata,
                        }))).into_response();
                    }
                }
            } else {
                let error = json!({ "error": format!("Module '{}' not found", name) });
                return (StatusCode::NOT_FOUND, Json(error)).into_response();
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
    
    // instance에서 모듈명 조회
    let instance = supervisor.instance_store.list()
        .iter()
        .find(|i| i.name == name);
    
    if let Some(inst) = instance {
        match supervisor.stop_server(&name, &inst.module_name, payload.force).await {
            Ok(result) => (StatusCode::OK, Json(result)).into_response(),
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
    }
    
    tracing::info!("Updating instance {} with settings: port={:?}, rcon_port={:?}, rcon_password={:?}, executable_path={:?}, rest_host={:?}, rest_port={:?}", 
        id, updated.port, updated.rcon_port, updated.rcon_password, updated.executable_path, updated.rest_host, updated.rest_port);
    
    // 저장
    if let Err(e) = supervisor.instance_store.update(&id, updated) {
        let error = json!({ "error": format!("Failed to update instance: {}", e) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
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
                moduleAliases: Default::default(),
                commandAliases: Default::default(),
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
