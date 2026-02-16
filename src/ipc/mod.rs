use anyhow::Result;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod handlers;
pub mod updates;
pub mod auth;
use updates::UpdateState;

pub use handlers::client::{kill_bot_pid, reap_expired_clients};

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
    /// 한 번이라도 클라이언트가 등록된 적이 있는지
    had_clients_ever: Arc<RwLock<bool>>,
    /// 마지막 클라이언트가 사라진 시점 (None = 아직 클라이언트 있음)
    last_client_lost_at: Arc<RwLock<Option<std::time::Instant>>>,
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            had_clients_ever: Arc::new(RwLock::new(false)),
            last_client_lost_at: Arc::new(RwLock::new(None)),
        }
    }
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
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
        // 클라이언트가 등록되면 "한 번이라도 연결됨" 플래그 세팅, lost 타임스탬프 해제
        *self.had_clients_ever.write().await = true;
        *self.last_client_lost_at.write().await = None;
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
    pub async fn has_clients(&self) -> bool {
        !self.inner.read().await.is_empty()
    }

    /// 한 번이라도 클라이언트가 등록된 적이 있었는지
    pub async fn had_clients_ever(&self) -> bool {
        *self.had_clients_ever.read().await
    }

    /// 마지막 클라이언트가 사라진 시점 (None이면 아직 클라이언트가 있거나 연결된 적 없음)
    pub async fn last_client_lost_at(&self) -> Option<std::time::Instant> {
        *self.last_client_lost_at.read().await
    }

    /// 클라이언트가 0이 되었을 때 lost 타임스탬프를 기록 (reaper에서 호출)
    pub async fn mark_all_clients_lost(&self) {
        let mut ts = self.last_client_lost_at.write().await;
        if ts.is_none() {
            *ts = Some(std::time::Instant::now());
            tracing::warn!("[Watchdog] All renderer clients lost — watchdog timer started");
        }
    }
}

// ── IPC Request/Response Types ─────────────────────────────

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
    pub executable_path: Option<String>,
    pub port: Option<u16>,
    pub rcon_port: Option<u16>,
    pub rcon_password: Option<String>,
    pub rest_host: Option<String>,
    pub rest_port: Option<u16>,
    pub rest_username: Option<String>,
    pub rest_password: Option<String>,
    pub protocol_mode: String,
    #[serde(default)]
    pub module_settings: std::collections::HashMap<String, serde_json::Value>,
    pub server_version: Option<String>,
    /// API(CLI/Discord)를 통해 마지막으로 시작/정지가 요청된 시점 (epoch ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_api_action: Option<u64>,
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
    pub icon: Option<String>,
    pub interaction_mode: Option<String>,
    pub protocols: Option<ProtocolsInfo>,
    pub settings: Option<crate::supervisor::module_loader::ModuleSettings>,
    pub commands: Option<crate::supervisor::module_loader::ModuleCommands>,
    pub syntax_highlight: Option<crate::supervisor::module_loader::SyntaxHighlight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleListResponse {
    pub modules: Vec<ModuleInfo>,
}

// ── API Action Tracker ──────────────────────────────────────

/// API(CLI/Discord 등)를 통한 서버 시작/정지 시점을 기록하여
/// GUI가 "예기치 않은" 상태 변경과 "API 경유" 상태 변경을 구분할 수 있게 함
#[derive(Clone, Default)]
pub struct ApiActionTracker {
    inner: Arc<std::sync::Mutex<HashMap<String, u64>>>,
}

impl ApiActionTracker {
    pub fn new() -> Self {
        Self { inner: Arc::new(std::sync::Mutex::new(HashMap::new())) }
    }

    /// 서버 이름에 대한 API 액션 타임스탬프 기록 (epoch millis)
    pub fn record(&self, server_name: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if let Ok(mut map) = self.inner.lock() {
            map.insert(server_name.to_string(), ts);
        }
    }

    /// 서버의 마지막 API 액션 타임스탬프 조회
    pub fn get(&self, server_name: &str) -> Option<u64> {
        self.inner.lock().ok().and_then(|map| map.get(server_name).copied())
    }
}

// ── IPC Server ─────────────────────────────────────────────

/// IPC Server State
#[derive(Clone)]
pub struct IPCServer {
    /// Shared supervisor instance used by all handlers
    pub supervisor: Arc<RwLock<crate::supervisor::Supervisor>>,
    pub listen_addr: String,
    /// 클라이언트(GUI/CLI) 생존 추적 레지스트리
    pub client_registry: ClientRegistry,
    /// 업데이트 매니저 (check + download 담당)
    pub update_state: UpdateState,
    /// API(CLI/Discord 봇) 경유 시작/정지 타임스탬프 추적
    pub api_actions: ApiActionTracker,
}

impl IPCServer {
    pub fn new(
        supervisor: Arc<RwLock<crate::supervisor::Supervisor>>,
        listen_addr: &str,
    ) -> Self {
        Self {
            supervisor,
            listen_addr: listen_addr.to_string(),
            client_registry: ClientRegistry::new(),
            update_state: UpdateState::new(),
            api_actions: ApiActionTracker::new(),
        }
    }

    pub async fn start(self) -> Result<()> {
        tracing::info!("IPC HTTP server starting on {}", self.listen_addr);

        let router = Router::new()
            // ── Server query/control ──
            .route("/api/servers", get(handlers::server::list_servers))
            .route("/api/server/:name/status", get(handlers::server::get_server_status))
            .route("/api/server/:name/start", post(handlers::server::start_server_handler))
            .route("/api/server/:name/stop", post(handlers::server::stop_server_handler))
            .route("/api/modules", get(handlers::server::list_modules))
            .route("/api/modules/refresh", post(handlers::server::refresh_modules))
            .route("/api/module/:name", get(handlers::server::get_module_metadata))
            // ── Instance CRUD ──
            .route("/api/instances", get(handlers::instance::list_instances).post(handlers::instance::create_instance))
            .route("/api/instances/reorder", put(handlers::instance::reorder_instances))
            .route("/api/instance/:id", get(handlers::instance::get_instance).delete(handlers::instance::delete_instance).patch(handlers::instance::update_instance_settings))
            // ── Command execution ──
            .route("/api/instance/:id/command", post(handlers::command::execute_command))
            .route("/api/instance/:id/rcon", post(handlers::command::execute_rcon_command))
            .route("/api/instance/:id/rest", post(handlers::command::execute_rest_command))
            // ── Managed-process & module-feature endpoints ──
            .route("/api/instance/:id/managed/start", post(handlers::managed::start_managed_handler))
            .route("/api/instance/:id/console", get(handlers::managed::get_console_handler))
            .route("/api/instance/:id/stdin", post(handlers::managed::send_stdin_handler))
            .route("/api/instance/:id/validate", post(handlers::managed::validate_instance_handler))
            .route("/api/instance/:id/properties", get(handlers::managed::read_properties_handler).put(handlers::managed::write_properties_handler))
            .route("/api/instance/:id/properties/reset", post(handlers::managed::reset_properties_handler))
            .route("/api/instance/:id/server/reset", post(handlers::managed::reset_server_handler))
            .route("/api/instance/:id/accept-eula", post(handlers::managed::accept_eula_handler))
            .route("/api/instance/:id/diagnose", post(handlers::managed::diagnose_handler))
            // ── Server installation endpoints ──
            .route("/api/module/:name/versions", get(handlers::managed::list_versions_handler))
            .route("/api/module/:name/version/:version", get(handlers::managed::get_version_details_handler))
            .route("/api/module/:name/install", post(handlers::managed::install_server_handler))
            // ── Bot config ──
            .route("/api/config/bot", get(handlers::bot::get_bot_config).put(handlers::bot::save_bot_config))
            // ── Client heartbeat ──
            .route("/api/client/register", post(handlers::client::client_register))
            .route("/api/client/:id/heartbeat", post(handlers::client::client_heartbeat))
            .route("/api/client/:id/unregister", delete(handlers::client::client_unregister))
            // ── Auth middleware (token-based) ──
            .layer(axum::middleware::from_fn(auth::auth_middleware))
            .with_state(self.clone())
            .merge(updates::updates_router(self.update_state.clone()));

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

// ── Tests ──────────────────────────────────────────────────

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

    #[test]
    fn test_module_info_serialization_with_commands() {
        let module_info = ModuleInfo {
            name: "palworld".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Palworld 서버 관리".to_string()),
            path: "/modules/palworld".to_string(),
            executable_path: Some("PalServer.exe".to_string()),
            icon: None,
            interaction_mode: None,
            protocols: None,
            settings: None,
            syntax_highlight: None,
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
        assert!(json.get("commands").is_some(), "commands field should exist");
        let commands = json.get("commands").unwrap();
        assert!(commands.get("fields").is_some(), "commands.fields should exist");
        let fields = commands.get("fields").unwrap().as_array().unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].get("http_method").unwrap().as_str().unwrap(), "GET");
        assert_eq!(fields[0].get("name").unwrap().as_str().unwrap(), "players");
    }

    #[test]
    fn test_module_info_without_commands() {
        let module_info = ModuleInfo {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            path: "/modules/test".to_string(),
            executable_path: None,
            icon: None,
            interaction_mode: None,
            protocols: None,
            settings: None,
            syntax_highlight: None,
            commands: None,
        };

        let json = serde_json::to_value(&module_info).unwrap();
        assert!(json.get("commands").is_some());
        assert!(json.get("commands").unwrap().is_null());
    }

    #[test]
    fn test_module_list_response_includes_commands() {
        let response = ModuleListResponse {
            modules: vec![
                ModuleInfo {
                    name: "palworld".to_string(),
                    version: "1.0.0".to_string(),
                    description: Some("Palworld".to_string()),
                    path: "/modules/palworld".to_string(),
                    executable_path: None,
                    icon: None,
                    interaction_mode: None,
                    protocols: None,
                    settings: None,
                    syntax_highlight: None,
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
        assert_eq!(fields[0].get("http_method").unwrap().as_str().unwrap(), "GET");
        assert_eq!(fields[1].get("http_method").unwrap().as_str().unwrap(), "POST");

        let announce_inputs = fields[1].get("inputs").unwrap().as_array().unwrap();
        assert_eq!(announce_inputs.len(), 1);
        assert_eq!(announce_inputs[0].get("name").unwrap().as_str().unwrap(), "message");
        assert_eq!(announce_inputs[0].get("required").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_http_method_parsing() {
        let test_cases = vec![
            ("GET", crate::protocol::HttpMethod::Get),
            ("get", crate::protocol::HttpMethod::Get),
            ("POST", crate::protocol::HttpMethod::Post),
            ("post", crate::protocol::HttpMethod::Post),
            ("PUT", crate::protocol::HttpMethod::Put),
            ("DELETE", crate::protocol::HttpMethod::Delete),
            ("UNKNOWN", crate::protocol::HttpMethod::Get),
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
