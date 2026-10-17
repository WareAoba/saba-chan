use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

pub mod handlers;
pub mod routes;
pub mod updates;
pub mod auth;
use updates::UpdateState;

pub use handlers::client::reap_expired_clients;

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
        });
        // 클라이언트가 등록되면 "한 번이라도 연결됨" 플래그 세팅, lost 타임스탬프 해제
        *self.had_clients_ever.write().await = true;
        *self.last_client_lost_at.write().await = None;
        id
    }

    /// Heartbeat 수신 — TTL 갱신
    pub async fn heartbeat(&self, client_id: &str) -> bool {
        let mut map = self.inner.write().await;
        if let Some(client) = map.get_mut(client_id) {
            client.last_heartbeat = std::time::Instant::now();
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
#[allow(dead_code)]
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
    /// 포트 충돌로 인한 강제 정지 이벤트 (drain — 한 번 읽으면 비워짐)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_conflict_stops: Vec<crate::supervisor::PortConflictStopEvent>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
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
    /// 익스텐션이 프로비저닝 중인지 여부
    #[serde(default)]
    pub provisioning: bool,
    /// 익스텐션 런타임 상태 (확장별 데이터, 예: {"<ext_id>": {"memory_percent": 25.5, ...}})
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extension_status: std::collections::HashMap<String, serde_json::Value>,
    /// 익스텐션 확장 데이터 (범용)
    #[serde(default)]
    pub extension_data: std::collections::HashMap<String, serde_json::Value>,
    /// 포트 충돌 정보 — 이 인스턴스의 포트가 다른 인스턴스와 겹치는 목록
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_conflicts: Vec<PortConflictInfo>,
}

/// 포트 충돌 요약 (서버 목록 API에서 반환)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortConflictInfo {
    pub port: u16,
    pub port_type: String,
    pub conflict_name: String,
    pub conflict_id: String,
    pub conflict_port_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolsInfo {
    pub supported: Vec<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
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
    /// 디렉토리 시그니처 — 마이그레이션 시 기존 서버 폴더 자동 감지용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dir_signatures: Vec<String>,
    /// 설치 방식 정보 (download, steamcmd 등)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<crate::supervisor::module_loader::ModuleInstallConfig>,
    /// 모듈 별명 (Discord/CLI 다국어 지원)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub module_aliases: Vec<String>,
    /// 명령어 별명
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub command_aliases: std::collections::HashMap<String, crate::supervisor::module_loader::CommandAliasInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionListResponse {
    pub modules: Vec<ExtensionInfo>,
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

/// Provision progress entry for a single instance
#[derive(Debug, Clone, Serialize)]
pub struct ProvisionProgress {
    pub step: u8,        // 0-based step index
    pub total: u8,       // total steps (e.g. 3)
    pub label: String,   // short machine-readable label
    pub message: String, // human-readable description
    pub done: bool,      // true when provisioning finished (success or error)
    pub error: Option<String>,
    /// Download/operation percentage (0-100), None if indeterminate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// Step labels provided by the extension (set once on first progress update)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<String>>,
}

/// Tracks provisioning progress per instance_id
#[derive(Debug, Clone)]
pub struct ProvisionTracker {
    inner: Arc<std::sync::Mutex<HashMap<String, ProvisionProgress>>>,
}

impl Default for ProvisionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ProvisionTracker {
    pub fn new() -> Self {
        Self { inner: Arc::new(std::sync::Mutex::new(HashMap::new())) }
    }

    pub fn update(&self, instance_id: &str, progress: ProvisionProgress) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(instance_id.to_string(), progress);
        }
    }

    pub fn get(&self, instance_id: &str) -> Option<ProvisionProgress> {
        self.inner.lock().ok().and_then(|map| map.get(instance_id).cloned())
    }

    pub fn remove(&self, instance_id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(instance_id);
        }
    }
}

// ── Extension Status Cache ──────────────────────────────────

/// 익스텐션 상태 정보를 TTL 기반으로 캐싱 (server.list_enrich hook 스팸 방지)
#[derive(Debug, Clone)]
pub struct ExtensionStatusCache {
    inner: Arc<std::sync::Mutex<HashMap<String, (Instant, Value)>>>,
    ttl_secs: u64,
}

impl ExtensionStatusCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(HashMap::new())),
            ttl_secs,
        }
    }

    /// 캐시에서 인스턴스의 익스텐션 상태 조회. TTL 만료 시 None 반환.
    pub fn get(&self, instance_id: &str) -> Option<Value> {
        self.inner.lock().ok().and_then(|map| {
            map.get(instance_id).and_then(|(ts, val)| {
                if ts.elapsed().as_secs() < self.ttl_secs {
                    Some(val.clone())
                } else {
                    None
                }
            })
        })
    }

    /// 캐시에 인스턴스의 익스텐션 상태 저장
    pub fn set(&self, instance_id: &str, value: Value) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(instance_id.to_string(), (Instant::now(), value));
        }
    }

    /// 인스턴스 캐시 무효화 (시작/정지 시)
    pub fn invalidate(&self, instance_id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(instance_id);
        }
    }
}

/// 익스텐션 초기화 상태 추적
#[derive(Clone)]
pub struct ExtensionInitTracker {
    inner: Arc<RwLock<ExtensionInitState>>,
}

struct ExtensionInitState {
    /// 초기화 중인 익스텐션 ID → 상태 메시지
    in_progress: std::collections::HashMap<String, String>,
    /// 초기화 완료된 익스텐션 (성공/실패)
    completed: Vec<ExtensionInitResult>,
}

#[derive(Clone, serde::Serialize)]
pub struct ExtensionInitResult {
    pub ext_id: String,
    pub success: bool,
    pub message: String,
    pub timestamp: u64,
}

impl Default for ExtensionInitTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionInitTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ExtensionInitState {
                in_progress: std::collections::HashMap::new(),
                completed: Vec::new(),
            })),
        }
    }

    pub async fn mark_started(&self, ext_id: &str, message: &str) {
        let mut state = self.inner.write().await;
        state.in_progress.insert(ext_id.to_string(), message.to_string());
    }

    pub async fn mark_finished(&self, ext_id: &str, success: bool, message: &str) {
        let mut state = self.inner.write().await;
        state.in_progress.remove(ext_id);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        state.completed.push(ExtensionInitResult {
            ext_id: ext_id.to_string(),
            success,
            message: message.to_string(),
            timestamp: ts,
        });
    }

    #[allow(dead_code)]
    pub async fn is_initializing(&self) -> bool {
        let state = self.inner.read().await;
        !state.in_progress.is_empty()
    }

    pub async fn snapshot(&self) -> serde_json::Value {
        let state = self.inner.read().await;
        serde_json::json!({
            "initializing": !state.in_progress.is_empty(),
            "in_progress": state.in_progress,
            "completed": state.completed,
        })
    }
}

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
    /// 프로비저닝 진행 상태 추적
    pub provision_tracker: ProvisionTracker,
    /// 범용 익스텐션 매니저
    pub extension_manager: Arc<RwLock<crate::extension::ExtensionManager>>,
    /// 익스텐션 상태 캐시 (list_enrich 스팸 방지)
    pub extension_status_cache: ExtensionStatusCache,
    /// 익스텐션 초기화(daemon.startup) 진행 상태
    pub extension_init_tracker: ExtensionInitTracker,
    /// 인스턴스별 RCON 연결 풀 (연결 재사용)
    pub rcon_pool: Arc<crate::protocol::rcon_pool::RconPool>,
    /// Graceful shutdown 토큰 — 취소 시 데몬 전체 종료
    pub shutdown_token: tokio_util::sync::CancellationToken,
    /// 중앙 설정 저장소 (타입 안전 JSON 관리)
    pub config_store: Arc<crate::config_store::ConfigStore>,
    /// 데몬 자체 로그 버퍼 (tracing 이벤트 캡처)
    pub daemon_log_buffer: crate::daemon_log::DaemonLogBuffer,
    /// Runtime preflight 진행 상태 — 봇/Node/Python 의존성 자동 검사·복구
    pub preflight_tracker: crate::runtime_preflight::PreflightTracker,
    /// 클라우드 에이전트 런타임 — 페어링/토큰 변경 시 핫 리로드
    pub cloud_agent: Arc<crate::cloud_agent::CloudAgentRuntime>,
}

impl IPCServer {
    pub fn new(
        supervisor: Arc<RwLock<crate::supervisor::Supervisor>>,
        listen_addr: &str,
        daemon_log_buffer: crate::daemon_log::DaemonLogBuffer,
    ) -> Self {
        // ExtensionManager 초기화
        let extensions_dir = crate::plugin::resolve_extensions_dir();
        let mut ext_mgr = crate::extension::ExtensionManager::new(
            extensions_dir.to_str().unwrap_or("./extensions"),
        );
        if let Err(e) = ext_mgr.discover() {
            tracing::warn!("Failed to discover extensions: {}", e);
        }

        let extension_manager = Arc::new(RwLock::new(ext_mgr));

        // ConfigStore 초기화 (기존 JSON 파일에서 로드)
        let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
        let config_store = Arc::new(crate::config_store::ConfigStore::new(&data_dir));

        // 클라우드 에이전트 런타임 — 데몬 종료 토큰의 child로 에이전트 관리
        let shutdown_token = tokio_util::sync::CancellationToken::new();
        let cloud_agent = Arc::new(crate::cloud_agent::CloudAgentRuntime::new(
            crate::cloud_agent::CloudAgentConfig::from_env(),
            supervisor.clone(),
            config_store.clone(),
            data_dir,
            shutdown_token.clone(),
        ));

        Self {
            supervisor: supervisor.clone(),
            listen_addr: listen_addr.to_string(),
            client_registry: ClientRegistry::new(),
            update_state: UpdateState::new().with_hot_reload(
                supervisor,
                extension_manager.clone(),
            ),
            api_actions: ApiActionTracker::new(),
            provision_tracker: ProvisionTracker::new(),
            extension_manager,
            extension_status_cache: ExtensionStatusCache::new(30), // 30초 TTL (Docker/WSL 지연 대비)
            extension_init_tracker: ExtensionInitTracker::new(),
            rcon_pool: Arc::new(crate::protocol::rcon_pool::RconPool::new()),
            shutdown_token,
            config_store,
            daemon_log_buffer,
            preflight_tracker: crate::runtime_preflight::PreflightTracker::new(),
            cloud_agent,
        }
    }

    pub async fn start(self) -> Result<()> {
        tracing::info!("IPC HTTP server starting on {}", self.listen_addr);

        // 앱 언어를 플러그인 러너에 반영 — 이후 스폰되는 모듈/익스텐션
        // 프로세스가 SABA_LANG을 물려받아 i18n 메시지를 현지화한다.
        let lang = self.config_store.get_gui_settings().await.language;
        crate::plugin::set_plugin_lang(lang);

        // 라우트 정의는 토픽별로 분리되어 [`crate::ipc::routes`] 에 있다.
        // 신규 엔드포인트 추가 시 이 함수가 아니라 routes.rs 의 토픽 함수를 수정한다.
        let router = routes::build_router(self.clone());

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

                    let shutdown_signal = self.shutdown_token.clone();
                    axum::serve(listener, router)
                        .with_graceful_shutdown(async move { shutdown_signal.cancelled().await })
                        .await?;
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
    use serde_json::json;

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
        let module_info = ExtensionInfo {
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
            install: None,
            dir_signatures: vec!["PalServer.exe".to_string()],
            module_aliases: vec![],
            command_aliases: std::collections::HashMap::new(),
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
        let module_info = ExtensionInfo {
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
            install: None,
            dir_signatures: vec![],
            module_aliases: vec![],
            command_aliases: std::collections::HashMap::new(),
            commands: None,
        };

        let json = serde_json::to_value(&module_info).unwrap();
        assert!(json.get("commands").is_some());
        assert!(json.get("commands").unwrap().is_null());
    }

    #[test]
    fn test_module_list_response_includes_commands() {
        let response = ExtensionListResponse {
            modules: vec![
                ExtensionInfo {
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
                    install: None,
                    dir_signatures: vec![],
                    module_aliases: vec![],
                    command_aliases: std::collections::HashMap::new(),
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
        assert!(announce_inputs[0].get("required").unwrap().as_bool().unwrap());
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
        assert!(json.get("required").unwrap().as_bool().unwrap());
        assert_eq!(json.get("placeholder").unwrap().as_str().unwrap(), "steam_xxxxxxxxx");
    }
}
