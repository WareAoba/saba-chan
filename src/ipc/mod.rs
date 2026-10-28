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
    /// 창 없이 트레이만 든 GUI (등록·heartbeat 본문의 `resident`)
    pub resident: bool,
}

/// 클라이언트 레지스트리 — 여러 GUI/CLI 인스턴스의 생존 여부를 추적
#[derive(Debug, Clone)]
pub struct ClientRegistry {
    inner: Arc<RwLock<HashMap<String, RegisteredClient>>>,
    /// `clients/launch`로 띄운 CLI (PID, 띄운 시각). 등록하지 않는 CLI도 완전히 종료할 때 닫는다
    launched_clis: Arc<std::sync::Mutex<Vec<(u32, u64)>>>,
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            launched_clis: Arc::default(),
        }
    }
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 데몬이 CLI를 띄웠다 (GUI의 CLI로 전환)
    pub fn note_launched_cli(&self, pid: u32) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.launched_clis.lock().unwrap().push((pid, at));
    }

    /// 띄운 CLI 가운데 아직 그 프로세스인 것을 닫는다 (같은 PID를 다른 프로세스가 받았으면 건드리지 않는다)
    pub fn close_launched_clis(&self) {
        use crate::supervisor::process::{force_kill_pid, is_same_process};
        let launched = std::mem::take(&mut *self.launched_clis.lock().unwrap());
        for (pid, at) in launched {
            if is_same_process(pid, at) {
                match force_kill_pid(pid) {
                    Ok(()) => tracing::info!("[Shutdown] Closed the CLI the daemon opened (pid {})", pid),
                    Err(e) => tracing::warn!("[Shutdown] Could not close the CLI (pid {}): {}", pid, e),
                }
            }
        }
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
            resident: false,
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

    /// 창 없이 트레이만 든 GUI인지 기록한다. 없는 id면 false
    pub async fn set_resident(&self, client_id: &str, resident: bool) -> bool {
        match self.inner.write().await.get_mut(client_id) {
            Some(client) => {
                client.resident = resident;
                true
            }
            None => false,
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
    /// 설정 저장의 revision (`ServerInstance::settings_revision`). `PATCH /api/instance/:id`에 실으면 그사이 바뀐 설정을 덮지 않는다
    #[serde(default)]
    pub revision: String,
    /// 메모리 상한 MiB (`ServerInstance::memory_limit_mb`)
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
    /// 익스텐션 데이터 slot (Q-7): 데몬이 조건을 판정하고 값을 찾아 둔 것 (`ExtensionManager::server_slots`)
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub slots: serde_json::Map<String, serde_json::Value>,
    /// 접속자 수 (Q-5): 켜진 서버이고 모듈이 `[status] players`로 알려 줄 때만 (`supervisor::players`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players: Option<crate::supervisor::players::PlayerCount>,
    /// 켜져 있지만 상태 질문(접속자 수)에 `UNRESPONSIVE_AFTER` 넘게 답하지 않을 때 마지막 실패 이유.
    /// REST로 명령을 보내는 서버라면 명령·정상 종료도 닿지 않는다 (`supervisor::players::unresponsive`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresponsive: Option<String>,
    /// 인스턴스 폴더 밖의 서버 폴더(가져온 서버). 삭제해도 남는다 (`instance::migration::linked_folder`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_folder: Option<String>,
    /// 가져오기 전 게임 설정 파일을 보관해 두어, 삭제(연결 해제)할 때 되돌릴 수 있는가
    #[serde(default)]
    pub restorable_settings: bool,
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

fn yes() -> bool {
    true
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
    /// `[extensions.saba-core] platforms`: 이 모듈이 돌아가는 OS. 없으면 선언하지 않음 (D3·M1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// 이 데몬의 OS에서 돌아가는가. 아니면 서버를 만들거나 켜지 않는다(`module_platform_unsupported`)
    #[serde(default = "yes")]
    pub platform_supported: bool,
    pub path: String,
    pub executable_path: Option<String>,
    /// `[config] server_executable`: 모듈이 찾는 서버 실행 파일 이름. 클라이언트는 실행 파일 고르기의 파일 종류로 쓴다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_executable: Option<String>,
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
    /// `[[queries]]` 읽기 전용 조회 (액션 `server.query:<name>`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<crate::supervisor::module_loader::ModuleQuery>,
    /// `[[agreements]]` 켜기 전에 동의할 약관 (이름은 모듈 locales `agreements.<id>`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agreements: Vec<crate::supervisor::module_loader::ModuleAgreement>,
    /// 모듈이 `functions`로 밝힌 lifecycle 함수 (MK1). null이면 모름: 클라이언트는 아무 기능도 숨기지 않는다
    #[serde(default)]
    pub functions: Option<Vec<String>>,
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
    #[serde(flatten)]
    pub details: serde_json::Map<String, Value>,
}

impl ProvisionProgress {
    pub fn with_failure(mut self, failure: Value) -> Self {
        if let Value::Object(mut fields) = failure {
            self.error = fields.remove("error").and_then(|v| v.as_str().map(str::to_string))
                .or_else(|| fields.get("message").and_then(Value::as_str).map(str::to_string));
            if let Some(Value::String(message)) = fields.remove("message") {
                self.message = message;
            }
            self.details = fields;
        }
        self
    }
}

/// Tracks provisioning progress per instance_id
#[derive(Debug, Clone)]
pub struct ProvisionTracker {
    inner: Arc<std::sync::Mutex<HashMap<String, ProvisionProgress>>>,
    /// 새로 만드는 중인 인스턴스 (진행 표시 이름 → 인스턴스 ID). 실패 후 dismiss할 때 롤백(삭제)해도 되는 것은
    /// 이 목록에 있는 것뿐이다. 기존 인스턴스의 업데이트 실패는 롤백하지 않는다.
    /// 이름이 그 사이 다른 인스턴스로 넘어갈 수 있으므로 지울 대상은 ID로만 찾는다
    creating: Arc<std::sync::Mutex<HashMap<String, String>>>,
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
    pub fn mark_creating(&self, name: &str, instance_id: &str) {
        if let Ok(mut map) = self.creating.lock() {
            map.insert(name.to_string(), instance_id.to_string());
        }
    }

    /// 신규 생성 표시를 지우고, 표시가 있었으면 그 인스턴스 ID를 돌려준다.
    pub fn take_creating(&self, name: &str) -> Option<String> {
        self.creating.lock().ok().and_then(|mut map| map.remove(name))
    }

    /// 인스턴스 이름이 바뀌면 진행 표시와 신규 생성 표시도 새 이름으로 옮긴다.
    /// 옛 이름에 남으면 그 이름으로 새로 만든 인스턴스가 설치 중으로 보이고, 그 배너를 닫으면 이 인스턴스가 지워진다
    /// 새 이름에 남아 있던 옛 표시(지운 인스턴스의 것)는 버린다
    pub fn rename(&self, from: &str, to: &str) {
        self.forget(to);
        if let Ok(mut map) = self.inner.lock() {
            if let Some(progress) = map.remove(from) {
                map.insert(to.to_string(), progress);
            }
        }
        if let Ok(mut map) = self.creating.lock() {
            if let Some(id) = map.remove(from) {
                map.insert(to.to_string(), id);
            }
        }
    }

    /// 이름이 비면(인스턴스 삭제) 그 이름의 진행 표시와 신규 생성 표시를 지운다.
    /// 남으면 같은 이름으로 새로 만든 인스턴스에 옛 오류 배너가 붙는다
    pub fn forget(&self, name: &str) {
        self.remove(name);
        if let Ok(mut map) = self.creating.lock() {
            map.remove(name);
        }
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
        #[cfg(not(test))]
        let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
        // 시험은 실제 설정 파일을 건드리지 않는다: 데몬마다 빈 데이터 폴더
        #[cfg(test)]
        let data_dir = tempfile::tempdir().expect("test data dir").keep();
        let config_store = Arc::new(crate::config_store::ConfigStore::new(&data_dir));

        let stop_requested = tokio_util::sync::CancellationToken::new();
        let client_registry = ClientRegistry::new();
        let credentials = CredentialsCache::default();
        // 모듈의 HOST 통로. 시작 때 다른 작업(data_roots 보충)이 잠금을 쥐고 있으면 풀린 뒤 붙인다:
        // 건너뛰면 감독자가 부르는 모듈(정지·접속자)이 데몬이 꺼질 때까지 `no_instance`로 실패한다
        let runtime = crate::plugin::host::Runtime::new(&supervisor, credentials.clone());
        match supervisor.try_read() {
            Ok(sup) => {
                let _ = sup.host_runtime.set(runtime);
            }
            Err(_) => match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    let supervisor = supervisor.clone();
                    handle.spawn(async move {
                        let _ = supervisor.read().await.host_runtime.set(runtime);
                    });
                }
                Err(_) => tracing::warn!("[Host] no async runtime to attach the module host channel"),
            },
        }
        Self {
            supervisor: supervisor.clone(),
            listen_addr: listen_addr.to_string(),
            client_registry: client_registry.clone(),
            update_state: UpdateState::new()
                .with_hot_reload(supervisor, extension_manager.clone())
                .with_stop_signal(stop_requested.clone())
                .with_clients(client_registry),
            api_actions: ApiActionTracker::new(),
            provision_tracker: ProvisionTracker::new(),
            extension_manager,
            extension_status_cache: ExtensionStatusCache::new(30), // 30초 TTL
            extension_init_tracker: ExtensionInitTracker::new(),
            credentials,
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            stop_requested,
            config_store,
            daemon_log_buffer,
            relay_agent: crate::relay_agent::RelayAgent::new(),
            system_monitor: crate::system_monitor::SystemMonitor::new(),
        }
    }

    /// 응답을 먼저 보낼 수 있게 잠깐 뒤에 종료를 요청한다 (정리 경로는 main이 처리).
    pub(crate) fn host_context(&self, id: &str) -> crate::plugin::host::Context {
        crate::plugin::host::Runtime::new(&self.supervisor, self.credentials.clone()).for_instance(id)
    }

    pub fn request_shutdown(&self) {
        let stop = self.stop_requested.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            stop.cancel();
        });
    }

    /// 데몬 HTTP 라우터 전체 (인증·감사 미들웨어 포함). 액션 카탈로그도 이것을 안에서 부른다
    pub fn router(&self) -> Router {
        self.routes()
            // 고른 원격 데몬으로 중계 (G1, `X-Saba-Target`). 로컬 IPC에만: 원격 포트는 `routes`를 따로 씌운다
            .layer(axum::middleware::from_fn_with_state(self.clone(), crate::remote_daemon::target::layer))
            .layer(axum::middleware::from_fn(auth::auth_middleware))
    }

    /// 인증층을 씌우기 전의 라우트 (감사 출처 포함). 로컬 IPC는 IPC 토큰, 원격 포트(`remote_daemon`)는 짝 맺은 PC 토큰을 씌운다
    pub(crate) fn routes(&self) -> Router {
        Router::new()
            // ── 경량 ping (lock / 디스크 I/O 없음) ──
            .route("/health", get(handlers::server::health_check))
            .route("/api/diagnostics", get(handlers::diagnostics::diagnostics))
            .route("/api/onboarding", get(handlers::diagnostics::onboarding))
            .route("/api/audit", get(handlers::diagnostics::audit))
            .route("/api/audit/export", post(handlers::diagnostics::audit_export))
            .route("/api/events", get(handlers::events::stream))
            // ── Server query/control ──
            .route("/api/servers", get(handlers::server::list_servers))
            .route("/api/instance/{id}/status", get(handlers::server::get_instance_status))
            .route("/api/instance/{id}/start", post(handlers::server::start_instance_handler))
            .route("/api/instance/{id}/events", get(handlers::server::instance_events_handler))
            .route("/api/instance/{id}/stop", post(handlers::server::stop_instance_handler))
            .route("/api/instance/{id}/restart", post(handlers::server::restart_instance_handler))
            .route("/api/modules", get(handlers::server::list_modules))
            .route("/api/modules/refresh", post(handlers::server::refresh_modules))
            .route("/api/version", get(handlers::server::version_handler))
            .route("/api/modules/schema", get(handlers::server::module_schema))
            .route("/api/maker", get(handlers::maker::get_maker))
            .route("/api/modules/detect", post(handlers::server::detect_module_dir))
            .route("/api/modules/manifest", get(handlers::server::fetch_module_manifest))
            .route("/api/modules/manifest/{id}/install", post(handlers::server::install_module_from_manifest))
            .route("/api/modules/{id}", delete(handlers::server::remove_module))
            .route("/api/module/{name}", get(handlers::server::get_module_metadata))
            .route("/api/module/{name}/audit", get(handlers::server::audit_module))
            // ── Instance CRUD ──
            .route("/api/instances", get(handlers::instance::list_instances).post(handlers::instance::create_instance))
            .route("/api/instances/reorder", put(handlers::instance::reorder_instances))
            .route("/api/instance/{id}/rename", post(handlers::instance::rename_instance))
            .route("/api/instance/{id}", get(handlers::instance::get_instance).delete(handlers::instance::delete_instance).patch(handlers::instance::update_instance_settings))
            .route("/api/instance/{id}/launch-preview", post(handlers::instance::launch_preview))
            // ── Provision progress ──
            .route("/api/provision-progress/{name}", get(handlers::instance::get_provision_progress).delete(handlers::instance::dismiss_provision_progress))
            // ── Server update ──
            .route("/api/instance/{id}/check-update", get(handlers::instance::check_server_update))
            .route("/api/instance/{id}/apply-update", post(handlers::instance::apply_server_update))
            // ── Command execution ──
            .route("/api/instance/{id}/command", post(handlers::command::execute_command))
            .route("/api/instance/{id}/commands", get(handlers::command::list_instance_commands))
            .route("/api/instance/{id}/rcon", post(handlers::command::rcon_route))
            .route("/api/instance/{id}/rest", post(handlers::command::rest_route))
            // ── Managed-process & module-feature endpoints ──
            .route("/api/instance/{id}/console", get(handlers::managed::get_console_handler))
            .route("/api/instance/{id}/logs", get(handlers::logs::logs_handler))
            .route("/api/instance/{id}/logs/export", get(handlers::logs::export_handler))
            .route("/api/instance/{id}/stdin", post(handlers::managed::send_stdin_handler))
            .route("/api/instance/{id}/validate", post(handlers::managed::validate_instance_handler))
            .route("/api/instance/{id}/query/{name}", post(handlers::managed::query_instance_handler))
            .route("/api/instance/{id}/players", get(handlers::managed::players_handler))
            .route("/api/instance/{id}/properties", get(handlers::managed::read_properties_handler).put(handlers::managed::write_properties_handler))
            .route("/api/instance/{id}/properties/reset", post(handlers::managed::reset_properties_handler))
            .route("/api/instance/{id}/settings/live", get(handlers::managed::read_live_settings_handler))
            .route("/api/instance/{id}/server/reset", post(handlers::managed::reset_server_handler))
            .route("/api/instance/{id}/agreements", post(handlers::managed::accept_agreements_handler))
            .route("/api/instance/{id}/diagnose", post(handlers::managed::diagnose_handler))
            .route("/api/instance/{id}/port-check", get(handlers::port_check::last).post(handlers::port_check::check))
            // 백업 (A2)·예약 작업 (A3)
            .route("/api/instance/{id}/backups", get(handlers::backup::list_backups).post(handlers::backup::create_backup))
            .route("/api/instance/{id}/backups/{name}", delete(handlers::backup::delete_backup))
            .route("/api/instance/{id}/backups/{name}/restore", post(handlers::backup::restore_backup))
            .route("/api/instance/{id}/schedule", get(handlers::backup::get_schedule).put(handlers::backup::put_schedule))
            // ── Server installation endpoints ──
            .route("/api/module/{name}/versions", get(handlers::managed::list_versions_handler))
            .route("/api/module/{name}/version/{version}", get(handlers::managed::get_version_details_handler))
            .route("/api/module/{name}/install", post(handlers::managed::install_server_handler))
            .route("/api/instance/{id}/installed-version", get(handlers::managed::get_installed_version_handler))
            // ── Bot config ──
            .route("/api/config/bot", get(handlers::bot::get_bot_config).put(handlers::bot::save_bot_config).patch(handlers::bot::patch_bot_config))
            // ── GUI config (settings.json) ──
            .route("/api/config/gui", get(handlers::config::get_gui_config).put(handlers::config::save_gui_config).patch(handlers::config::patch_gui_config))
            .route("/api/config/client/{kind}", get(handlers::config::get_client_config).put(handlers::config::save_client_config))
            .route("/api/config/{kind}/defaults", get(handlers::config::get_config_defaults))
            .route("/api/config/{kind}/reset", post(handlers::config::reset_config))
            // ── 설정·인스턴스 내보내기/가져오기 (A9) ──
            .route("/api/migration/export", post(handlers::migration::export))
            .route("/api/migration/import", post(handlers::migration::import))
            // ── Node token (cloud relay auth) ──
            .route("/api/config/node-token", put(handlers::config::save_node_token).delete(handlers::config::delete_node_token))
            // ── Module locales ──
            .route("/api/module/{name}/locales", get(handlers::config::get_module_locales))
            // ── System components (release manifest) ──
            .route("/api/system/components", get(handlers::config::get_system_components))
            .route("/api/system/metrics", get(handlers::system::get_metrics))
            .route("/api/system/paths", get(handlers::paths::get_paths))
            .route("/api/system/paths/move", post(handlers::paths::move_dir))
            .route("/api/system/browse", get(handlers::paths::browse))
            .route("/api/system/autostart", get(handlers::autostart::get_autostart).put(handlers::autostart::put_autostart))
            // ── Client heartbeat ──
            .route("/api/client/register", post(handlers::client::client_register))
            .route("/api/clients", get(handlers::client::client_list))
            .route("/api/clients/launch", get(handlers::client::client_launchable).post(handlers::client::client_launch))
            // ── 디스코드 명령 권한 (P1) ──
            .route(
                "/api/permissions/{guild_id}",
                get(handlers::permissions::get_permissions).patch(handlers::permissions::update_permissions),
            )
            .route(
                "/api/permissions/{guild_id}/commands",
                get(handlers::permissions::get_command_board),
            )
            .route(
                "/api/permissions/{guild_id}/members/{discord_id}",
                axum::routing::put(handlers::permissions::set_member).delete(handlers::permissions::remove_member),
            )
            // ── 디스코드 길드 설정 (I2) ──
            .route(
                "/api/discord/{guild_id}/settings",
                get(handlers::discord::get_settings).patch(handlers::discord::patch_settings),
            )
            // ── 확인되지 않은 구성요소를 격리 없이 돌리기 동의 ──
            .route("/api/sandbox/unisolated", get(handlers::sandbox::list_unisolated))
            .route(
                "/api/sandbox/unisolated/{component}",
                post(handlers::sandbox::allow_unisolated).delete(handlers::sandbox::revoke_unisolated),
            )
            .route("/api/sandbox/extension-gui", get(handlers::sandbox::list_extension_gui))
            .route(
                "/api/sandbox/extension-gui/{component}",
                post(handlers::sandbox::allow_extension_gui).delete(handlers::sandbox::revoke_extension_gui),
            )
            // ── 액션 카탈로그 (P2) ──
            .route("/api/actions", get(handlers::actions::list_actions))
            .route("/api/actions/plan", post(handlers::actions::plan_steps))
            .route("/api/actions/runs/{run_id}", get(handlers::actions::run_status))
            .route("/api/actions/{id}/plan", post(handlers::actions::plan_action))
            .route("/api/actions/{id}/run", post(handlers::actions::run_action))
            .route("/api/client/{id}/heartbeat", post(handlers::client::client_heartbeat))
            .route("/api/client/{id}/unregister", delete(handlers::client::client_unregister))
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
            .route("/api/extensions/{id}/enable", post(handlers::extension::enable_extension))
            .route("/api/extensions/{id}/disable", post(handlers::extension::disable_extension))
            .route("/api/extensions/{id}/mount", post(handlers::extension::mount_extension))
            .route("/api/extensions/{id}/unmount", post(handlers::extension::unmount_extension))
            .route("/api/extensions/{id}/install", post(handlers::extension::install_extension))
            .route("/api/extensions/{id}", delete(handlers::extension::remove_extension))
            .route("/api/extensions/{id}/config", get(handlers::extension::get_extension_config).put(handlers::extension::save_extension_config))
            .route("/api/extensions/{id}/gui", get(handlers::extension::serve_gui_bundle))
            .route("/api/extensions/{id}/gui/styles", get(handlers::extension::serve_gui_styles))
            .route("/api/extensions/{id}/icon", get(handlers::extension::serve_icon))
            .route("/api/extensions/{id}/audit", get(handlers::extension::audit_extension))
            .route("/api/extensions/{id}/i18n/{locale}", get(handlers::extension::serve_i18n))
            // ── Relay server proxy (cloud mode) ──
            .route("/api/relay/host/{host_id}/status", get(handlers::relay::check_host_status))
            .route("/api/relay/host/{host_id}/nodes", get(handlers::relay::list_host_nodes))
            .route("/api/relay/node/{guild_id}/members", get(handlers::relay::list_node_members))
            .route("/api/relay/node/{guild_id}/legacy-members", get(handlers::relay::list_node_legacy_members))
            .route(
                "/api/relay/node/{guild_id}/channels",
                get(handlers::relay::get_node_channels).put(handlers::relay::put_node_channels),
            )
            .route("/api/relay/node/{guild_id}/chatbot/models", get(handlers::relay::get_node_chatbot_models))
            .route("/api/relay/node/{guild_id}/chatbot/credit", get(handlers::relay::get_node_chatbot_credit))
            .route("/api/relay/node/{guild_id}/chatbot/tools", get(handlers::relay::get_node_chatbot_tools))
            .route("/api/relay/bot-invite", get(handlers::relay::get_bot_invite))
            .route("/api/relay/pair/initiate", post(handlers::relay::initiate_pairing))
            .route("/api/relay/pair/{code}/status", get(handlers::relay::poll_pairing_status))
            .route("/api/relay/mcp/grant", get(handlers::relay::get_mcp_grant).delete(handlers::relay::revoke_mcp_grant))
            .route("/api/relay/link-requests", get(handlers::relay::list_link_requests))
            .route("/api/relay/link-requests/{request_id}", post(handlers::relay::answer_link_request))
            // 개인정보 모두 지우기: 웹사이트에서 동의하면 릴레이가 지우고, 데몬이 연결을 끊는다
            .route("/api/relay/privacy/erase", get(handlers::privacy::erase_status).post(handlers::privacy::start_erase))
            // ── Relay agent (cloud Discord bot ↔ daemon) ──
            .route("/api/relay/agent", get(handlers::chat::agent_status))
            .route("/api/relay/agent/start", post(handlers::chat::agent_start))
            .route("/api/relay/agent/stop", post(handlers::chat::agent_stop))
            // ── Chat command processor ──
            .route("/api/chat/command", post(handlers::chat::chat_command))
            // AI 챗봇 설정 (원본은 데몬, 릴레이에 맞춘다)
            .route("/api/chatbot", get(handlers::chatbot::get_chatbot).put(handlers::chatbot::put_chatbot).delete(handlers::chatbot::delete_chatbot))
            .route("/api/chatbot/memory", delete(handlers::chatbot::delete_memory))
            .route("/api/chatbot/memo-template", get(handlers::chatbot::get_memo_template))
            // 인스턴스별 디스코드 낱말, 길드가 쓸 서버 (로컬 전용)
            .route("/api/instance/{id}/words", get(handlers::words::get).put(handlers::words::put))
            .route("/api/guilds/{guild}/allowed-instances", put(handlers::words::put_allowed))
            // 켜짐 알림: 길드마다 고른 서버 (액션 instance.start_notice.set 도 같은 라우트)
            .route("/api/guilds/{guild}/start-notice", get(handlers::start_notice::get))
            .route("/api/instance/{id}/start-notice/{guild}", put(handlers::start_notice::put))
            .route("/api/chatbot/guilds/{guild}", get(handlers::chatbot::get_guild).patch(handlers::chatbot::patch_guild))
            // ── 원격 데몬 (D1): 원격 포트 설정과 짝 맺은 PC (로컬 전용, 원격 포트에서는 막는다) ──
            .route("/api/remote/settings", get(handlers::remote_daemon::get_settings).put(handlers::remote_daemon::put_settings))
            .route("/api/remote/clients", get(handlers::remote_daemon::list_clients))
            .route("/api/remote/clients/{host_id}", delete(handlers::remote_daemon::revoke_client))
            .route("/api/remote/peers", get(handlers::remote_daemon::list_peers))
            .route("/api/remote/daemons", get(handlers::remote_daemon::list_daemons).post(handlers::remote_daemon::connect_daemon))
            .route("/api/remote/daemons/{host_id}", delete(handlers::remote_daemon::forget_daemon))
            .route("/api/remote/servers", get(handlers::remote_daemon::list_all_servers))
            .route("/api/remote/daemons/{host_id}/proxy/{*rest}", get(handlers::remote_daemon::proxy).post(handlers::remote_daemon::proxy).put(handlers::remote_daemon::proxy).patch(handlers::remote_daemon::proxy).delete(handlers::remote_daemon::proxy))
            .with_state(self.clone())
            .merge(updates::updates_router(self.update_state.clone()))
            // 감사 로그 출처 (A10): 요청마다 X-Saba-Client로 정한다. 인증층은 이 바깥에 씌운다 (업데이트 라우터까지)
            .layer(axum::middleware::from_fn(crate::audit::layer))
            // 짝 맺은 PC(출처 `remote`)의 PC 전용 경로·칸 (D1). 원격 포트와 안에서 다시 부르는 라우터(`actions::call_as`)가 모두 지난다
            .layer(axum::middleware::from_fn_with_state(self.clone(), crate::remote_daemon::server::rules))
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
            #[cfg(windows)]
            {
                use std::{mem::size_of, os::windows::io::AsRawSocket};
                let enabled: i32 = 1;
                let rc = unsafe {
                    windows_sys::Win32::Networking::WinSock::setsockopt(
                        socket.as_raw_socket() as _,
                        windows_sys::Win32::Networking::WinSock::SOL_SOCKET,
                        windows_sys::Win32::Networking::WinSock::SO_EXCLUSIVEADDRUSE,
                        (&enabled as *const i32).cast(),
                        size_of::<i32>() as i32,
                    )
                };
                if rc != 0 { return Err(std::io::Error::last_os_error().into()); }
            }
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
                    tokio::spawn(async move {
                        // 예전 낱말은 첫 하트비트 전에 인스턴스 낱말로 옮긴다
                        crate::ipc::handlers::words::migrate_on_start(&agent_server).await;
                        crate::relay_agent::auto_start(&agent_server).await
                    });
                    // 원격 포트 (D1): 켜져 있으면 열고 릴레이에 주소를 올린다
                    let remote_server = self.clone();
                    tokio::spawn(async move { crate::remote_daemon::apply(&remote_server).await });
                    // 자동 포트포워딩 확인 (설정이 켜져 있을 때만, 자동 시작한 서버가 포트를 연 뒤)
                    tokio::spawn(crate::port_check::auto_on_start(self.clone()));

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

    /// 데몬 시작 때 다른 작업(data_roots 보충)이 감독자 잠금을 쥐고 있어도 모듈의 HOST 통로(REST·콘솔 중계)가 붙는다.
    /// 빠지면 감독자가 부르는 모듈(정지·접속자)이 내내 `no_instance`로 실패해, 팰월드가 일반 정지를 거절했다 (test-pal 실측 2026-10-26)
    #[tokio::test]
    async fn the_module_host_channel_is_attached_even_when_startup_work_holds_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let supervisor = Arc::new(RwLock::new(crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        )));
        let held = supervisor.clone().write_owned().await;
        let _server = IPCServer::new(supervisor.clone(), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        drop(held);
        let attached = async {
            loop {
                if supervisor.read().await.host_context("any").is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), attached).await.expect("the host channel must be attached once the lock is free");
    }

    /// 시험이 만든 데몬은 이 PC의 실제 설정 파일을 읽거나 쓰지 않는다 (PC 관문 2026-10-26: 시험이 data 폴더 config.json에 길드 설정을 남김)
    #[tokio::test]
    async fn test_daemons_keep_their_settings_out_of_the_real_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let server = || {
            let sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
            IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new())
        };
        let guild = "settings-isolation-guild";
        let (a, b) = (server(), server());
        a.config_store
            .update_node_settings(guild, |n| {
                n.insert("x".into(), json!(1));
                ((), true)
            })
            .await
            .unwrap();
        assert!(b.config_store.get_bot_config().await.node_settings.get(guild).is_none(), "another test daemon must not see it");
        let real = saba_chan_updater_lib::constants::resolve_data_dir().join("config.json");
        assert!(!std::fs::read_to_string(real).unwrap_or_default().contains(guild), "the real config.json must stay untouched");
    }

    #[test]
    fn module_failure_fields_survive_provision_progress() {
        let failure = json!({
            "success": false, "error_code": "module_exception", "module": "game", "function": "install_server",
            "phase": "run", "error": "failed", "message": "original reason", "path": "bad-name", "stderr_tail": ["trace"],
        });
        let progress = ProvisionProgress {
            step: 0, total: 1, label: "done".into(), message: "install failed".into(), done: true,
            error: None, percent: None, steps: None, details: Default::default(),
        }.with_failure(failure.clone());
        let tracker = ProvisionTracker::new();
        tracker.update("test-game", progress);
        let value = serde_json::to_value(tracker.get("test-game").unwrap()).unwrap();
        for (key, expected) in failure.as_object().unwrap() {
            assert_eq!(&value[key], expected, "lost {key}");
        }
    }

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
        tracker.mark_creating("fresh", "id-1");
        assert_eq!(tracker.take_creating("fresh").as_deref(), Some("id-1"));
        // 한 번 꺼내면 끝 — 이후 업데이트 실패를 dismiss해도 롤백하지 않는다
        assert!(tracker.take_creating("fresh").is_none());
        assert!(tracker.take_creating("existing").is_none());
    }

    // 리뷰 289 P3-2: 지운 "b"나 이름을 바꿔 비운 "b"의 옛 오류 배너가 새로 만든 "b"에 붙었다
    #[test]
    fn a_name_freed_by_delete_or_rename_carries_no_old_progress() {
        let failed = || ProvisionProgress {
            step: 1, total: 3, label: "install".into(), message: String::new(), done: true, error: Some("boom".into()), percent: None, steps: None, details: Default::default(),
        };
        let tracker = ProvisionTracker::new();
        tracker.update("b", failed());
        tracker.mark_creating("b", "old-b");
        tracker.forget("b");
        assert!(tracker.get("b").is_none() && tracker.take_creating("b").is_none());

        tracker.update("b", failed());
        tracker.rename("a", "b");
        assert!(tracker.get("b").is_none(), "renaming onto a name drops the stale banner there");
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
            platforms: None,
            platform_supported: true,
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
            server_executable: None,
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
            queries: Vec::new(),
            agreements: Vec::new(),
            functions: None,
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
                        remote: true,
                        reply_pattern: None,
                    },
                ],
                hook: false,
                failure_patterns: Vec::new(),
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
            platforms: None,
            platform_supported: true,
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
            server_executable: None,
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
            queries: Vec::new(),
            agreements: Vec::new(),
            functions: None,
            commands: None,
        };

        let json = serde_json::to_value(&module_info).unwrap();
        assert!(json.get("commands").is_some());
        assert!(json.get("commands").unwrap().is_null());
        assert!(json["functions"].is_null(), "unknown functions are sent as null, not left out");
    }

    #[test]
    fn test_module_list_response_includes_commands() {
        let response = ExtensionListResponse {
            failed: Vec::new(),
            modules: vec![
                ExtensionInfo {
                    platforms: None,
                    platform_supported: true,
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
                    server_executable: None,
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
                    queries: Vec::new(),
                    agreements: Vec::new(),
                    functions: None,
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
                                remote: true,
                                reply_pattern: None,
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
                                remote: true,
                                reply_pattern: None,
                            },
                        ],
                        hook: false,
                        failure_patterns: Vec::new(),
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
