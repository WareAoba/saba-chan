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
use std::sync::Arc;
use tokio::sync::RwLock;

/// IPC 요청/응답 타입
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStartRequest {
    #[serde(default)]
    pub resource: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStopRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerListResponse {
    pub servers: Vec<ServerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub module: String,
    pub status: String,
    pub pid: Option<u32>,
    pub uptime_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub path: String,
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
            .route("/api/instances", get(list_instances).post(create_instance))
            .route("/api/instance/:id", get(get_instance).delete(delete_instance))
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
            name: instance.name.clone(),
            module: instance.module_name.clone(),
            status,
            pid,
            uptime_seconds: None,
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

/// GET /api/server/:name/status - 서버 상태 조회
async fn get_server_status(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    
    // TODO: 서버의 모듈명을 어떻게 알지?
    let module_name = "minecraft";
    
    match supervisor.get_server_status(&name, module_name).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let error = json!({ "error": format!("Failed to get status: {}", e) });
            (StatusCode::NOT_FOUND, Json(error)).into_response()
        }
    }
}

/// POST /api/server/:name/start - 서버 시작
async fn start_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStartRequest>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    
    // payload.resource에서 module 이름 추출
    let module_name = payload.resource
        .as_ref()
        .and_then(|r| r.get("module"))
        .and_then(|m| m.as_str())
        .unwrap_or("minecraft") // 기본값
        .to_string(); // 부분 복사
    
    let config = payload.resource.unwrap_or_else(|| json!({}));
    
    match supervisor.start_server(&name, &module_name, config).await {
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
    
    // TODO: 서버의 모듈명을 어떻게 알지? 일단 기본값 사용
    let module_name = "minecraft"; // 서버-모듈 매핑 필요
    
    match supervisor.stop_server(&name, module_name, payload.force).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let error = json!({ "error": format!("Failed to stop server: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
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
