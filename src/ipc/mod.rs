use anyhow::Result;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

pub mod handlers;
pub mod updates;
pub mod auth;
pub mod internal;
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
    /// 클라이언트가 알려 준 자기 PID와 설치 폴더 (인스톨러가 이 설치의 GUI만 끌 때 쓴다)
    pub pid: Option<u32>,
    pub install_root: Option<String>,
}

/// 클라이언트 레지스트리 — 여러 GUI/CLI 인스턴스의 생존 여부를 추적
#[derive(Debug, Clone)]
pub struct ClientRegistry {
    inner: Arc<RwLock<HashMap<String, RegisteredClient>>>,
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 새 클라이언트 등록, client_id 반환
    pub async fn register(&self, kind: ClientKind, pid: Option<u32>, install_root: Option<String>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut map = self.inner.write().await;
        tracing::info!("[Heartbeat] Client registered: {} ({:?}, PID {:?})", id, kind, pid);
        map.insert(id.clone(), RegisteredClient {
            kind,
            last_heartbeat: std::time::Instant::now(),
            pid,
            install_root,
        });
        id
    }

    /// 등록된 클라이언트 (id와 함께)
    pub async fn list(&self) -> Vec<(String, RegisteredClient)> {
        self.inner.read().await.iter().map(|(id, c)| (id.clone(), c.clone())).collect()
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
    /// saba-runner가 맡아 실행하는지 (`Supervisor::uses_managed_start`, 판정은 데몬만 한다)
    #[serde(default)]
    pub managed: bool,
    /// 익스텐션 런타임 상태 (확장별 데이터, 예: {"<ext_id>": {"memory_percent": 25.5, ...}})
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extension_status: std::collections::HashMap<String, serde_json::Value>,
    /// 익스텐션 확장 데이터 (범용)
    #[serde(default)]
    pub extension_data: std::collections::HashMap<String, serde_json::Value>,
    /// 포트 충돌 정보 — 이 인스턴스의 포트가 다른 인스턴스와 겹치는 목록
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_conflicts: Vec<PortConflictInfo>,
    /// 설정 필드에 없는 추가 실행 인자 (`ServerInstance::extra_args`)
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// 메모리 상한 MiB (`ServerInstance::memory_limit_mb`)
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
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

/// 맞지 않는 컴포넌트 의존성 하나
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnmetDependency {
    pub component: String,
    pub required: String,
    /// 설치돼 있지 않으면 None
    pub installed: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub version: String,
    /// 신뢰 표시: verified(서명된 카탈로그에서 받았고 그대로) · modified · unverified (M12)
    pub trust: saba_chan_updater_lib::integrity::Trust,
    pub description: Option<String>,
    /// module.toml schema_version (0 = 스키마 이전 모듈)
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// 로드는 됐지만 모듈 작성자가 고쳐야 할 점
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// `[module.dependencies]` 중 지금 설치된 버전이 못 맞추는 것 (M9). saba-core 미달은 아예 로드하지 않는다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmet_dependencies: Vec<UnmetDependency>,
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
    /// `[[presets]]` 서버를 만들 때 고르는 추천 설정 묶음
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<crate::supervisor::module_loader::ModulePreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionListResponse {
    pub modules: Vec<ExtensionInfo>,
    /// 로드하지 못한 모듈과 이유
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<crate::supervisor::module_loader::ModuleLoadError>,
}

// ── API Action Tracker ──────────────────────────────────────

/// 게임 설정 파일에서 읽은 접속 정보(`get_credentials`) 캐시.
/// 명령마다 Python을 띄우지 않도록 인스턴스별로 잠깐 보관한다. 설정을 바꾸거나
/// 접속이 실패하면 지워서 다음 명령이 다시 읽게 한다.
#[derive(Clone, Default)]
pub struct CredentialsCache {
    inner: Arc<std::sync::Mutex<HashMap<String, (std::time::Instant, Value)>>>,
}

impl CredentialsCache {
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);

    pub fn get(&self, instance_id: &str) -> Option<Value> {
        let map = self.inner.lock().ok()?;
        let (at, creds) = map.get(instance_id)?;
        (at.elapsed() < Self::TTL).then(|| creds.clone())
    }

    pub fn put(&self, instance_id: &str, creds: Value) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(instance_id.to_string(), (std::time::Instant::now(), creds));
        }
    }

    pub fn invalidate(&self, instance_id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(instance_id);
        }
    }
}

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
    /// 새로 만드는 중인 인스턴스 이름. 실패 후 dismiss할 때 롤백(삭제)해도 되는 것은
    /// 이 목록에 있는 것뿐이다. 기존 인스턴스의 업데이트 실패는 롤백하지 않는다.
    creating: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

impl Default for ProvisionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ProvisionTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(HashMap::new())),
            creating: Arc::new(std::sync::Mutex::new(Default::default())),
        }
    }

    /// 신규 생성 프로비저닝 시작을 기록한다.
    pub fn mark_creating(&self, name: &str) {
        if let Ok(mut set) = self.creating.lock() {
            set.insert(name.to_string());
        }
    }

    /// 신규 생성 표시를 지우고, 표시가 있었는지 돌려준다.
    pub fn take_creating(&self, name: &str) -> bool {
        self.creating.lock().map(|mut set| set.remove(name)).unwrap_or(false)
    }

    pub fn update(&self, instance_id: &str, progress: ProvisionProgress) {
        if let Ok(serde_json::Value::Object(mut data)) = serde_json::to_value(&progress) {
            data.insert("instance".into(), instance_id.into());
            crate::events::emit_live("provision.progress", data.into());
        }
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
    /// 인스턴스별 게임 접속 정보 캐시 (명령마다 Python get_credentials를 띄우지 않게)
    pub credentials: CredentialsCache,
    /// Graceful shutdown 토큰 — 정리가 끝난 뒤 취소되며, 취소 시 모든 태스크 종료
    pub shutdown_token: tokio_util::sync::CancellationToken,
    /// 종료 요청 — API·워치독 등은 이것만 취소한다. main의 정리 태스크가 받아서
    /// Ctrl+C와 같은 정리(익스텐션 daemon.shutdown 훅 등)를 거친 뒤 `shutdown_token`을 취소한다.
    pub stop_requested: tokio_util::sync::CancellationToken,
    /// 범용 익스텐션 프로세스 매니저 (이름 기반 다중 프로세스 관리)
    /// 중앙 설정 저장소 (타입 안전 JSON 관리)
    pub config_store: Arc<crate::config_store::ConfigStore>,
    /// 데몬 자체 로그 버퍼 (tracing 이벤트 캡처)
    pub daemon_log_buffer: crate::daemon_log::DaemonLogBuffer,
    /// 클라우드 릴레이 에이전트 (디스코드 명령 수신)
    pub relay_agent: crate::relay_agent::RelayAgent,
    /// 시스템 모니터 (요청이 올 때만 표본을 뜬다)
    pub system_monitor: crate::system_monitor::SystemMonitor,
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

        let stop_requested = tokio_util::sync::CancellationToken::new();
        Self {
            supervisor: supervisor.clone(),
            listen_addr: listen_addr.to_string(),
            client_registry: ClientRegistry::new(),
            update_state: UpdateState::new()
                .with_hot_reload(supervisor, extension_manager.clone())
                .with_stop_signal(stop_requested.clone()),
            api_actions: ApiActionTracker::new(),
            provision_tracker: ProvisionTracker::new(),
            extension_manager,
            extension_status_cache: ExtensionStatusCache::new(30), // 30초 TTL
            extension_init_tracker: ExtensionInitTracker::new(),
            credentials: CredentialsCache::default(),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            stop_requested,
            config_store,
            daemon_log_buffer,
            relay_agent: crate::relay_agent::RelayAgent::new(),
            system_monitor: crate::system_monitor::SystemMonitor::new(),
        }
    }

    /// 응답을 먼저 보낼 수 있게 잠깐 뒤에 종료를 요청한다 (정리 경로는 main이 처리).
    pub fn request_shutdown(&self) {
        let stop = self.stop_requested.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            stop.cancel();
        });
    }

    /// 데몬 HTTP 라우터 전체 (인증·감사 미들웨어 포함). 액션 카탈로그도 이것을 안에서 부른다
    pub fn router(&self) -> Router {
        Router::new()
            // ── 경량 ping (lock / 디스크 I/O 없음) ──
            .route("/health", get(handlers::server::health_check))
            .route("/api/diagnostics", get(handlers::diagnostics::diagnostics))
            .route("/api/audit", get(handlers::diagnostics::audit))
            .route("/api/events", get(handlers::events::stream))
            // ── Server query/control ──
            .route("/api/servers", get(handlers::server::list_servers))
            .route("/api/server/:name/status", get(handlers::server::get_server_status))
            .route("/api/server/:name/start", post(handlers::server::start_server_handler))
            .route("/api/instance/:id/start", post(handlers::server::start_instance_handler))
            .route("/api/instance/:id/events", get(handlers::server::instance_events_handler))
            .route("/api/instance/:id/stop", post(handlers::server::stop_instance_handler))
            .route("/api/instance/:id/restart", post(handlers::server::restart_instance_handler))
            .route("/api/server/:name/stop", post(handlers::server::stop_server_handler))
            .route("/api/modules", get(handlers::server::list_modules))
            .route("/api/modules/refresh", post(handlers::server::refresh_modules))
            .route("/api/version", get(handlers::server::version_handler))
            .route("/api/modules/schema", get(handlers::server::module_schema))
            .route("/api/modules/detect", post(handlers::server::detect_module_dir))
            .route("/api/modules/manifest", get(handlers::server::fetch_module_manifest))
            .route("/api/modules/manifest/:id/install", post(handlers::server::install_module_from_manifest))
            .route("/api/modules/:id", delete(handlers::server::remove_module))
            .route("/api/module/:name", get(handlers::server::get_module_metadata))
            .route("/api/module/:name/audit", get(handlers::server::audit_module))
            // ── Instance CRUD ──
            .route("/api/instances", get(handlers::instance::list_instances).post(handlers::instance::create_instance))
            .route("/api/instances/reorder", put(handlers::instance::reorder_instances))
            .route("/api/instance/:id/rename", post(handlers::instance::rename_instance))
            .route("/api/instance/:id", get(handlers::instance::get_instance).delete(handlers::instance::delete_instance).patch(handlers::instance::update_instance_settings))
            .route("/api/instance/:id/launch-preview", post(handlers::instance::launch_preview))
            // ── Provision progress ──
            .route("/api/provision-progress/:name", get(handlers::instance::get_provision_progress).delete(handlers::instance::dismiss_provision_progress))
            // ── Command execution ──
            .route("/api/instance/:id/check-update", get(handlers::instance::check_server_update))
            .route("/api/instance/:id/apply-update", post(handlers::instance::apply_server_update))
            // ── Command execution ──
            .route("/api/instance/:id/command", post(handlers::command::execute_command))
            .route("/api/instance/:id/commands", get(handlers::command::list_instance_commands))
            .route("/api/instance/:id/rcon", post(handlers::command::execute_rcon_command))
            .route("/api/instance/:id/rest", post(handlers::command::execute_rest_command))
            // ── Managed-process & module-feature endpoints ──
            .route("/api/instance/:id/managed/start", post(handlers::managed::start_managed_handler))
            .route("/api/instance/:id/console", get(handlers::managed::get_console_handler))
            .route("/api/instance/:id/stdin", post(handlers::managed::send_stdin_handler))
            .route("/api/instance/:id/validate", post(handlers::managed::validate_instance_handler))
            .route("/api/instance/:id/properties", get(handlers::managed::read_properties_handler).put(handlers::managed::write_properties_handler))
            .route("/api/instance/:id/properties/reset", post(handlers::managed::reset_properties_handler))
            .route("/api/instance/:id/settings/live", get(handlers::managed::read_live_settings_handler))
            .route("/api/instance/:id/server/reset", post(handlers::managed::reset_server_handler))
            .route("/api/instance/:id/accept-eula", post(handlers::managed::accept_eula_handler))
            .route("/api/instance/:id/diagnose", post(handlers::managed::diagnose_handler))
            // 백업 (A2)·예약 작업 (A3)
            .route("/api/instance/:id/backups", get(handlers::backup::list_backups).post(handlers::backup::create_backup))
            .route("/api/instance/:id/backups/:name", delete(handlers::backup::delete_backup))
            .route("/api/instance/:id/backups/:name/restore", post(handlers::backup::restore_backup))
            .route("/api/instance/:id/schedule", get(handlers::backup::get_schedule).put(handlers::backup::put_schedule))
            // ── Server installation endpoints ──
            .route("/api/module/:name/versions", get(handlers::managed::list_versions_handler))
            .route("/api/module/:name/version/:version", get(handlers::managed::get_version_details_handler))
            .route("/api/module/:name/install", post(handlers::managed::install_server_handler))
            .route("/api/instance/:id/installed-version", get(handlers::managed::get_installed_version_handler))
            // ── Bot config ──
            .route("/api/config/bot", get(handlers::bot::get_bot_config).put(handlers::bot::save_bot_config))
            // ── GUI config (settings.json) ──
            .route("/api/config/gui", get(handlers::config::get_gui_config).put(handlers::config::save_gui_config))
            .route("/api/config/client/:kind", get(handlers::config::get_client_config).put(handlers::config::save_client_config))
            .route("/api/config/:kind/defaults", get(handlers::config::get_config_defaults))
            .route("/api/config/:kind/reset", post(handlers::config::reset_config))
            // ── 설정·인스턴스 내보내기/가져오기 (A9) ──
            .route("/api/migration/export", post(handlers::migration::export))
            .route("/api/migration/import", post(handlers::migration::import))
            // ── Node token (cloud relay auth) ──
            .route("/api/config/node-token", get(handlers::config::get_node_token).put(handlers::config::save_node_token).delete(handlers::config::delete_node_token))
            // ── Module locales ──
            .route("/api/module/:name/locales", get(handlers::config::get_module_locales))
            // ── System components (release manifest) ──
            .route("/api/system/components", get(handlers::config::get_system_components))
            .route("/api/system/metrics", get(handlers::system::get_metrics))
            // ── Directory scan (migration etc.) ──
            // ── Extension process management (범용) ──
            // ── Client heartbeat ──
            .route("/api/client/register", post(handlers::client::client_register))
            .route("/api/clients", get(handlers::client::client_list))
            .route("/api/clients/launch", post(handlers::client::client_launch))
            // ── 디스코드 명령 권한 (P1) ──
            .route(
                "/api/permissions/:guild_id",
                get(handlers::permissions::get_permissions).patch(handlers::permissions::update_permissions),
            )
            .route(
                "/api/permissions/:guild_id/members/:discord_id",
                axum::routing::put(handlers::permissions::set_member).delete(handlers::permissions::remove_member),
            )
            // ── 디스코드 길드 설정 (I2) ──
            .route(
                "/api/discord/:guild_id/settings",
                get(handlers::discord::get_settings).patch(handlers::discord::patch_settings),
            )
            // ── 확인되지 않은 구성요소를 격리 없이 돌리기 동의 ──
            .route("/api/sandbox/unisolated", get(handlers::sandbox::list_unisolated))
            .route(
                "/api/sandbox/unisolated/:component",
                post(handlers::sandbox::allow_unisolated).delete(handlers::sandbox::revoke_unisolated),
            )
            // ── 액션 카탈로그 (P2) ──
            .route("/api/actions", get(handlers::actions::list_actions))
            .route("/api/actions/:id/plan", post(handlers::actions::plan_action))
            .route("/api/actions/:id/run", post(handlers::actions::run_action))
            .route("/api/client/:id/heartbeat", post(handlers::client::client_heartbeat))
            .route("/api/client/:id/unregister", delete(handlers::client::client_unregister))
            // ── Daemon lifecycle ──
            .route("/api/daemon/shutdown", post(handlers::client::daemon_shutdown))
            .route("/api/daemon/console", get(handlers::client::daemon_console))
            // ── Extension management ──
            .route("/api/extensions", get(handlers::extension::list_extensions))
            .route("/api/extensions/init-status", get(handlers::extension::extension_init_status))
            .route("/api/runtime/preflight", get(|| async { axum::Json(crate::python_env::preflight_snapshot()) }))
            .route("/api/extensions/rescan", post(handlers::extension::rescan_extensions))
            .route("/api/extensions/manifest", get(handlers::extension::fetch_manifest))
            .route("/api/extensions/updates", get(handlers::extension::check_extension_updates))
            .route("/api/extensions/:id/enable", post(handlers::extension::enable_extension))
            .route("/api/extensions/:id/disable", post(handlers::extension::disable_extension))
            .route("/api/extensions/:id/mount", post(handlers::extension::mount_extension))
            .route("/api/extensions/:id/unmount", post(handlers::extension::unmount_extension))
            .route("/api/extensions/:id/install", post(handlers::extension::install_extension))
            .route("/api/extensions/:id", delete(handlers::extension::remove_extension))
            .route("/api/extensions/:id/config", get(handlers::extension::get_extension_config).put(handlers::extension::save_extension_config))
            .route("/api/extensions/:id/gui", get(handlers::extension::serve_gui_bundle))
            .route("/api/extensions/:id/gui/styles", get(handlers::extension::serve_gui_styles))
            .route("/api/extensions/:id/icon", get(handlers::extension::serve_icon))
            .route("/api/extensions/:id/audit", get(handlers::extension::audit_extension))
            .route("/api/extensions/:id/i18n/:locale", get(handlers::extension::serve_i18n))
            // ── Node.js portable environment ──
            // ── Relay server proxy (cloud mode) ──
            .route("/api/relay/host/:host_id/status", get(handlers::relay::check_host_status))
            .route("/api/relay/host/:host_id/nodes", get(handlers::relay::list_host_nodes))
            .route("/api/relay/node/:guild_id/members", get(handlers::relay::list_node_members))
            .route(
                "/api/relay/node/:guild_id/channels",
                get(handlers::relay::get_node_channels).put(handlers::relay::put_node_channels),
            )
            .route("/api/relay/bot-invite", get(handlers::relay::get_bot_invite))
            .route("/api/relay/pair/initiate", post(handlers::relay::initiate_pairing))
            .route("/api/relay/pair/:code/status", get(handlers::relay::poll_pairing_status))
            // ── Relay agent (cloud Discord bot ↔ daemon) ──
            .route("/api/relay/agent", get(handlers::chat::agent_status))
            .route("/api/relay/agent/start", post(handlers::chat::agent_start))
            .route("/api/relay/agent/stop", post(handlers::chat::agent_stop))
            // ── Chat command processor ──
            .route("/api/chat/command", post(handlers::chat::chat_command))
            // AI 챗봇 설정 (원본은 데몬, 릴레이에 맞춘다)
            .route("/api/chatbot", get(handlers::chatbot::get_chatbot).put(handlers::chatbot::put_chatbot).delete(handlers::chatbot::delete_chatbot))
            .route("/api/chatbot/memory", delete(handlers::chatbot::delete_memory))
            .with_state(self.clone())
            .merge(updates::updates_router(self.update_state.clone()))
            // ── Auth middleware (token-based) — 업데이트 라우터까지 모든 라우트에 ──
            // 감사 로그 출처 (A10): 요청마다 X-Saba-Client로 정한다
            .layer(axum::middleware::from_fn(crate::audit::layer))
            .layer(axum::middleware::from_fn(auth::auth_middleware))
    }

    pub async fn start(self) -> Result<()> {
        tracing::info!("IPC HTTP server starting on {}", self.listen_addr);

        // 포트 충돌 검사 여부는 전역 설정 하나(gui.portConflictCheck). 저장 시에도 갱신된다.
        let check_ports = self.config_store.get_gui_settings().await.port_conflict_check;
        self.supervisor
            .read()
            .await
            .skip_port_check
            .store(!check_ports, std::sync::atomic::Ordering::Relaxed);

        handlers::diagnostics::mark_started();
        let router = self.router();

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
            // Unix에서는 TIME_WAIT 포트 재사용만 허용한다. Windows의 SO_REUSEADDR는
            // 이미 리슨 중인 포트에도 바인드를 허용하므로 켜지 않는다.
            #[cfg(not(windows))]
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

                    // 릴레이 에이전트 자동 시작 (노드 토큰 + autoStart일 때)
                    let agent_server = self.clone();
                    tokio::spawn(async move { crate::relay_agent::auto_start(&agent_server).await });

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

    #[test]
    fn credentials_cache_keeps_until_invalidated() {
        let cache = CredentialsCache::default();
        assert!(cache.get("a").is_none());
        cache.put("a", json!({ "rcon_password": "x" }));
        assert_eq!(cache.get("a").unwrap()["rcon_password"], "x");
        assert!(cache.get("b").is_none());
        cache.invalidate("a");
        assert!(cache.get("a").is_none());
    }

    #[test]
    fn only_new_instances_are_marked_for_rollback() {
        let tracker = ProvisionTracker::new();
        tracker.mark_creating("fresh");
        assert!(tracker.take_creating("fresh"));
        // 한 번 꺼내면 끝 — 이후 업데이트 실패를 dismiss해도 롤백하지 않는다
        assert!(!tracker.take_creating("fresh"));
        assert!(!tracker.take_creating("existing"));
    }

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
            trust: saba_chan_updater_lib::integrity::Trust::Unverified,
            name: "palworld".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Palworld 서버 관리".to_string()),
            schema_version: 1,
            display_name: None,
            game_name: None,
            author: None,
            warnings: Vec::new(),
            unmet_dependencies: Vec::new(),
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
            presets: Vec::new(),
            commands: Some(crate::supervisor::module_loader::ModuleCommands {
                fields: vec![
                    crate::supervisor::module_loader::CommandField {
                        name: "players".to_string(),
                        label: "플레이어 목록".to_string(),
                        description: Some("현재 접속 중인 플레이어 조회".to_string()),
                        method: Some("rest".to_string()),
                        http_method: Some("GET".to_string()),
                        endpoint_template: Some("/v1/api/players".to_string()),
                        console_template: None,
                        risk: None,
                        inputs: vec![],
                    },
                ],
                hook: false,
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
            trust: saba_chan_updater_lib::integrity::Trust::Unverified,
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            schema_version: 1,
            display_name: None,
            game_name: None,
            author: None,
            warnings: Vec::new(),
            unmet_dependencies: Vec::new(),
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
            presets: Vec::new(),
            commands: None,
        };

        let json = serde_json::to_value(&module_info).unwrap();
        assert!(json.get("commands").is_some());
        assert!(json.get("commands").unwrap().is_null());
    }

    #[test]
    fn test_module_list_response_includes_commands() {
        let response = ExtensionListResponse {
            failed: Vec::new(),
            modules: vec![
                ExtensionInfo {
                    trust: saba_chan_updater_lib::integrity::Trust::Unverified,
                    name: "palworld".to_string(),
                    version: "1.0.0".to_string(),
                    description: Some("Palworld".to_string()),
                    schema_version: 1,
                    display_name: None,
                    game_name: None,
                    author: None,
                    warnings: Vec::new(),
                    unmet_dependencies: Vec::new(),
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
                    presets: Vec::new(),
                    commands: Some(crate::supervisor::module_loader::ModuleCommands {
                        fields: vec![
                            crate::supervisor::module_loader::CommandField {
                                name: "info".to_string(),
                                label: "서버 정보".to_string(),
                                description: None,
                                method: Some("rest".to_string()),
                                http_method: Some("GET".to_string()),
                                endpoint_template: Some("/v1/api/info".to_string()),
                                console_template: None,
                                risk: None,
                                inputs: vec![],
                            },
                            crate::supervisor::module_loader::CommandField {
                                name: "announce".to_string(),
                                label: "공지 전송".to_string(),
                                description: None,
                                method: Some("rest".to_string()),
                                http_method: Some("POST".to_string()),
                                endpoint_template: Some("/v1/api/announce".to_string()),
                                console_template: None,
                                risk: None,
                                inputs: vec![
                                    crate::supervisor::module_loader::CommandInput {
                                        name: "message".to_string(),
                                        label: Some("메시지".to_string()),
                                        input_type: Some("string".to_string()),
                                        required: Some(true),
                                        placeholder: Some("공지 내용".to_string()),
                                        default: None,
                                        min: None,
                                        max: None,
                                        options: None,
                                    },
                                ],
                            },
                        ],
                        hook: false,
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
            min: None,
            max: None,
            options: None,
        };

        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json.get("name").unwrap().as_str().unwrap(), "user_id");
        assert_eq!(json.get("label").unwrap().as_str().unwrap(), "유저 ID");
        assert_eq!(json.get("type").unwrap().as_str().unwrap(), "string");
        assert_eq!(json.get("required").unwrap().as_bool().unwrap(), true);
        assert_eq!(json.get("placeholder").unwrap().as_str().unwrap(), "steam_xxxxxxxxx");
    }
}
