pub mod process;
pub mod managed_process;
pub mod crash;
pub mod highlight;
pub mod players;
pub mod idle;
pub mod watchdog;
pub mod ready;
pub mod player_events;
pub mod chat_link;
pub mod game_chat;
pub mod core_swap;
pub mod runtime;
mod start;
mod stop;
mod exit;
mod monitor;
mod reattach;
mod install;
pub(crate) use install::{progress_to_tracker, InstallCall, InstanceInstall};

use anyhow::Result;
use process::ProcessTracker;
use crate::module_loader::{self, ModuleLoader, LoadedModule};
use managed_process::{ManagedProcess, ManagedProcessStore};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command as TokioCommand;
use crate::instance::InstanceStore;

/// Stop 후 auto-detect를 억제하는 쿨다운 시간 (초)
const STOP_COOLDOWN_SECS: u64 = 30;

/// 포트 충돌로 인한 강제 정지 이벤트
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PortConflictStopEvent {
    /// 알림 번호 (데몬이 켜진 뒤 1부터 늘어난다). 클라이언트는 본 번호를 기억해 `notices_after`로 그 뒤만 받는다 (L50)
    #[serde(default)]
    pub id: u64,
    /// 강제 정지된 인스턴스 이름
    pub stopped_name: String,
    /// 강제 정지된 인스턴스 ID
    pub stopped_id: String,
    /// 충돌 상대 인스턴스 이름
    pub existing_name: String,
    /// 충돌 상대 인스턴스 ID
    pub existing_id: String,
    /// 충돌 포트 번호
    pub port: u16,
    /// 타임스탬프 (epoch ms)
    pub timestamp_ms: u64,
}

/// 포트 충돌 강제 정지 알림을 몇 개까지 두는가 (헤드리스 데몬에서 끝없이 쌓이지 않게)
pub const PORT_CONFLICT_STOPS_KEPT: usize = 20;
/// 포트 충돌 강제 정지 알림을 얼마나 두는가. 그사이 여러 클라이언트가 각자 읽는다 (L50)
pub const PORT_CONFLICT_STOPS_TTL_MS: u64 = 10 * 60 * 1000;

/// 포트 충돌 강제 정지 알림. 읽어도 지우지 않고 `PORT_CONFLICT_STOPS_TTL_MS`·`PORT_CONFLICT_STOPS_KEPT`까지 둔다
#[derive(Debug, Default)]
pub struct PortConflictStops {
    events: Vec<PortConflictStopEvent>,
    last_id: u64,
}

impl PortConflictStops {
    fn forget_old(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        self.events.retain(|e| now.saturating_sub(e.timestamp_ms) <= PORT_CONFLICT_STOPS_TTL_MS);
        let over = self.events.len().saturating_sub(PORT_CONFLICT_STOPS_KEPT);
        self.events.drain(..over);
    }
}

/// `install_server`가 설치에 성공한 뒤 백업을 지우기 전에 부르는 저장 단계
pub type InstallCommit<'a> = dyn Fn(&mut Supervisor, &Value) -> Result<()> + Sync + 'a;

pub struct Supervisor {
    pub tracker: ProcessTracker,
    pub module_loader: ModuleLoader,
    pub instance_store: InstanceStore,
    /// Store for processes spawned and managed directly by the daemon (with stdio capture)
    pub managed_store: ManagedProcessStore,
    /// 익스텐션 매니저. 감독자가 만들어 들고, IPC 서버·업데이트 상태는 같은 것을 나눠 쓴다
    pub extension_manager: std::sync::Arc<tokio::sync::RwLock<crate::extension::ExtensionManager>>,
    /// 프로비저닝 진행 상태 추적 (auto-detect 억제용). IPC 서버와 같은 것을 나눠 쓴다
    pub provision_tracker: crate::ipc::ProvisionTracker,
    /// 포트 충돌로 인한 강제 정지 알림 (클라이언트가 `GET /api/servers`로 읽는다)
    port_conflict_stops: std::sync::Mutex<PortConflictStops>,
    /// GUI 설정: 포트 충돌 검사 건너뛰기 (portConflictCheck 비활성화 시 true)
    pub skip_port_check: std::sync::Arc<AtomicBool>,
    /// 모니터가 한 번 돌았는가. 데몬이 켜질 때 이미 돌던 프로세스는 예기치 않은 시작으로 알리지 않는다
    first_scan_done: bool,
    /// 모니터가 정한 재시작 (인스턴스 ID, 대기, 예약 때의 실행 세대). 데몬 루프가 가져가 실행한다
    pending_restarts: Vec<(String, Duration, u64)>,
    /// 인스턴스별 런타임 상태 (정지 표시·실행 세대·재시작과 종료 이력·접속자 수·준비 알림 등).
    /// 인스턴스를 지우면 `Runtimes::forget`으로 한 번에 버린다
    pub runtime: runtime::Runtimes,
    pub(crate) host_runtime: std::sync::OnceLock<crate::plugin::host::Runtime>,
    addon_swap_gate: AddonSwapGate,
}

tokio::task_local! {
    /// 시작 요청을 받았을 때의 실행 세대. 시작 API는 모듈 validate(시작 전 점검)를 끝낸 뒤 시작을 부르므로,
    /// 점검 중에 들어온 정지도 잡도록 그 전에 잡은 세대를 `StartGuard`의 기준으로 쓴다
    pub static START_SINCE: u64;

    /// 이 작업 안에서 부른 정지가 올린 뒤의 실행 세대. 재시작이 자기 정지 뒤의 기준으로 쓴다 (D-S10)
    pub static STOPPED_AT: std::cell::Cell<Option<u64>>;
}

/// 인스턴스 설정의 참/거짓 값 (`true`/`false` 문자열도 받는다)
fn setting_flag(instance: &crate::instance::ServerInstance, key: &str, default: bool) -> bool {
    match instance.module_settings.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s == "true" => true,
        Some(Value::String(s)) if s == "false" => false,
        _ => default,
    }
}

/// 찾는 인스턴스가 없다. 핸들러는 오류 문구가 아니라 이 타입을 보고 404를 고른다
/// (모듈이 낸 "not found" 같은 문구가 "인스턴스 없음"으로 바뀌지 않게)
#[derive(Debug)]
pub struct InstanceNotFound(pub String);

impl std::fmt::Display for InstanceNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Instance '{}' not found", self.0)
    }
}

impl std::error::Error for InstanceNotFound {}

/// 모듈 함수가 성공을 알리지 않았을 때 사용자에게 보일 이유. 모듈이 준 `message`·`error`를 쓰고,
/// 둘 다 없으면 계약 위반(예: `success`·`program` 없이 옛 키만 돌려줌)을 그대로 알린다
pub(crate) fn module_failure(result: &Value, module: &str, function: &str) -> anyhow::Error {
    let field = |k: &str| result.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    if result.get("success").and_then(Value::as_bool) == Some(false) || field("message").or_else(|| field("error")).is_some() {
        let mut response = result.clone();
        response["module"] = json!(module);
        response["function"] = json!(function);
        let error: anyhow::Error = crate::plugin::ModuleCallError::from_response(response).into();
        return match field("message") {
            Some(message) => error.context(message.to_string()),
            None => error,
        };
    }
    let keys: Vec<&str> = result.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    let message = format!(
        "Module '{}' {}() did not report success (returned: {})",
        module,
        function,
        if keys.is_empty() { "nothing".to_string() } else { keys.join(", ") }
    );
    let mut response = result.as_object().cloned().unwrap_or_default();
    response.insert("success".into(), json!(false));
    response.insert("module".into(), json!(module));
    response.insert("function".into(), json!(function));
    response.entry("error".to_string()).or_insert_with(|| json!(message));
    crate::plugin::ModuleCallError::from_response(Value::Object(response)).into()
}

/// 설치는 끝났지만 `install_server`의 `commit`(인스턴스 기록 저장)이 실패해 파일을 설치 전으로 되돌렸다.
/// 되돌리지 못한 파일은 `PutBackFailed`와 같은 모양으로 싣는다
#[derive(Debug)]
pub struct CommitFailed(pub PutBackFailed);

impl std::fmt::Display for CommitFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the server details could not be saved")?;
        if !self.0.files.is_empty() {
            write!(f, "; {}", self.0)?;
        }
        Ok(())
    }
}

/// 설치가 모듈 오류로 끝났는데 되돌리지도 못한 파일 (`restore_failed`와 같은 순서의 남긴 백업 이름). 핸들러가 응답 칸으로 옮긴다
#[derive(Debug)]
pub struct PutBackFailed {
    pub files: Vec<String>,
    pub kept: Vec<String>,
}

impl std::fmt::Display for PutBackFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kept: Vec<String> = self.files.iter().zip(&self.kept).map(|(file, backup)| format!("{file} (kept as {backup})")).collect();
        write!(f, "could not put back {}", kept.join(", "))
    }
}

/// 서버 시작·설치와 모듈·익스텐션 자동 교체가 겹치지 않게 하는 문. 감독자마다 하나다: 다른 감독자(테스트가 따로 만든 것)의 시작은 이 문을 쥐지 않는다
#[derive(Clone, Default)]
pub struct AddonSwapGate(std::sync::Arc<tokio::sync::RwLock<()>>);

impl AddonSwapGate {
    /// 서버 시작·설치를 막는다: 모듈·익스텐션 자동 교체가 사용 확인부터 교체까지 쥔다.
    /// 시작(`start_server`·`start_managed_server`)과 서버 설치·업데이트(`hold_addons`)가 진행 중이면 기다리지 않고 None:
    /// 기다리면 그 뒤로 새 시작이 줄 서고, 시작 안에서 다시 읽기를 잡는 경로가 멈춘다. 교체는 다음 차례에 다시 본다
    pub fn try_hold_starts(&self) -> Option<tokio::sync::OwnedRwLockWriteGuard<()>> {
        self.0.clone().try_write_owned().ok()
    }

    /// 모듈·익스텐션을 쓰는 작업(서버 시작·설치·업데이트)이 끝날 때까지 자동 교체를 막는다. 여럿이 함께 쥘 수 있다
    pub async fn hold_addons(&self) -> tokio::sync::OwnedRwLockReadGuard<()> {
        self.0.clone().read_owned().await
    }
}

impl Supervisor {
    /// 인스턴스를 지운다: 저장소의 기록·폴더와 그 런타임 상태를 함께 (지운 인스턴스의 상태가 데몬이 끝날 때까지 남지 않게)
    pub fn remove_instance(&mut self, id: &str) -> Result<()> {
        let name = self.instance_store.get(id).map(|i| i.name.clone()).unwrap_or_default();
        self.instance_store.remove(id)?;
        // 상태를 버리면 그 프로세스의 준비 알림과 연동 닫기가 할 일을 못 찾는다: 버리기 전에 지우기 전과 같은 알림으로 닫는다
        ready::exited(&self.runtime, id, &name, false);
        chat_link::forget(id, &name);
        self.runtime.forget(id);
        Ok(())
    }

    pub(crate) fn host_context(&self, id: &str) -> Option<crate::plugin::host::Context> {
        self.host_runtime.get().map(|runtime| runtime.for_instance(id))
    }

    pub fn new(modules_dir: &str) -> Self {
        let instances_dir = saba_chan_updater_lib::constants::resolve_instances_dir()
            .to_string_lossy()
            .to_string();
        Self::new_with_instances_dir(modules_dir, &instances_dir)
    }

    /// 인스턴스 디렉토리를 명시적으로 지정하는 생성자 (테스트 격리용)
    pub fn new_with_instances_dir(modules_dir: &str, instances_dir: &str) -> Self {
        Self {
            tracker: ProcessTracker::new(),
            module_loader: ModuleLoader::new(modules_dir),
            instance_store: InstanceStore::new(instances_dir),
            managed_store: ManagedProcessStore::new(),
            extension_manager: Self::new_extension_manager(),
            provision_tracker: crate::ipc::ProvisionTracker::new(),
            port_conflict_stops: std::sync::Mutex::new(PortConflictStops::default()),
            skip_port_check: std::sync::Arc::new(AtomicBool::new(false)),
            first_scan_done: false,
            pending_restarts: Vec::new(),
            runtime: Default::default(),
            host_runtime: Default::default(),
            addon_swap_gate: AddonSwapGate::default(),
        }
    }

    /// 데몬의 익스텐션 폴더를 보는 익스텐션 매니저 (아직 발견 전. `initialize`가 발견한다)
    pub(crate) fn new_extension_manager() -> std::sync::Arc<tokio::sync::RwLock<crate::extension::ExtensionManager>> {
        let dir = crate::plugin::resolve_extensions_dir();
        std::sync::Arc::new(tokio::sync::RwLock::new(crate::extension::ExtensionManager::new(&dir.to_string_lossy())))
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // 모듈 발견
        let modules = self.module_loader.discover_modules()?;
        tracing::info!("Loaded {} modules", modules.len());
        for module in modules {
            tracing::info!("  - {} v{}", module.metadata.name, module.metadata.version);
        }
        
        // 익스텐션 발견
        if let Err(e) = self.extension_manager.write().await.discover() {
            tracing::warn!("Failed to discover extensions: {}", e);
        }

        // 인스턴스 로드
        self.instance_store.load()?;
        tracing::info!("Loaded {} server instances", self.instance_store.list().len());

        self.migrate_settings();

        // 고아 managed 프로세스 재연결
        self.reattach_orphaned_managed().await;
        
        Ok(())
    }

    /// 저장된 인스턴스 설정을 지금 모듈 선언에 맞춘다. 모듈을 다시 읽은 뒤에도 부른다 (모듈이 없는 인스턴스는 그대로 둔다).
    /// - 모듈이 `list`로 바꾼 설정의 옛 문자열 값을 배열로 읽는다 (06 §8.1)
    /// - `protocol_mode`의 `auto`·예전 `rcon`은 모듈 프로토콜로 맞춘다 (메모리에서만, 저장할 때 함께 적힌다)
    /// - 0.1.14-beta.1까지 인스턴스 칸에 있던 REST 접속값을 모듈 설정으로 옮겨 저장한다 (`[protocols.rest]`, 한 번)
    /// - 0.1.14-beta.1까지 인스턴스에 복사해 두던 모듈 `[install]` 값을 지워 저장한다 (한 번)
    pub fn migrate_settings(&mut self) {
        let loader = &self.module_loader;
        let mut moved = Vec::new();
        self.instance_store.adjust_in_memory(|instance| {
            let Ok(module) = loader.get_module(&instance.module_name) else { return };
            if let Some(settings) = &module.metadata.settings {
                crate::validator::migrate_list_values(&settings.fields, &mut instance.module_settings);
            }
            if let Some(mode) = module.metadata.protocol_mode_for(&instance.protocol_mode) {
                instance.protocol_mode = mode;
            }
            let rest_moved = instance.take_legacy_rest(module.metadata.rest());
            let copies_dropped = module.metadata.install.as_ref().is_some_and(|install| instance.drop_install_copies(&install.method));
            if rest_moved || copies_dropped {
                moved.push(instance.clone());
            }
        });
        for instance in moved {
            let id = instance.id.clone();
            if let Err(e) = self.instance_store.update(&id, instance) {
                tracing::warn!("Could not save the migrated settings of instance {}: {}", id, e);
            }
        }
    }

    /// REST를 받는 모듈의 REST 포트 설정 이름 (module_name → `[protocols.rest] port_setting`).
    /// `check_port_conflicts`가 그 설정 값을 REST 포트로 비교한다
    pub(crate) fn rest_port_settings(&self) -> HashMap<String, String> {
        let modules = self.module_loader.modules().unwrap_or_default();
        modules.iter().filter_map(|m| Some((m.metadata.name.clone(), m.metadata.rest()?.port_setting.clone()))).collect()
    }

    /// 인스턴스가 저장해 둔 포트: 게임 포트와 모듈의 포트 설정 값
    pub(crate) fn saved_ports(&self, instance: &crate::instance::ServerInstance) -> Vec<u16> {
        let fields: Vec<String> = self.module_loader.get_module(&instance.module_name).ok()
            .and_then(|m| m.metadata.settings.as_ref().map(|s| s.port_fields().into_iter().map(str::to_string).collect()))
            .unwrap_or_default();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        crate::validator::instance_ports(instance, &fields).into_iter().map(|(_, p)| p).collect()
    }

    /// 인스턴스의 실행 파일 경로를 자동 해석합니다.
    ///
    /// 우선순위:
    /// 1. `instance.executable_path` — 사용자가 명시한 경로 (override)
    /// 2. `working_dir / module.server_executable` — 모듈이 정의한 바이너리 파일명으로 자동 계산
    ///
    /// 모듈에 `server_executable`이 정의되어 있으면 `working_dir`만으로 실행 파일을 찾을 수 있으므로
    /// 사용자가 별도로 `executable_path`를 설정할 필요가 없습니다.
    pub(crate) fn resolve_executable(&self, instance: &crate::instance::ServerInstance, module_name: &str) -> Option<String> {
        // 1. 사용자 명시 경로 우선
        if let Some(ref exe_path) = instance.executable_path {
            if !exe_path.is_empty() {
                return Some(exe_path.clone());
            }
        }

        // 2. module_settings에 executable_path가 있으면 사용 (기존 인스턴스 호환)
        if let Some(exe_val) = instance.module_settings.get("executable_path") {
            if let Some(exe_str) = exe_val.as_str() {
                if !exe_str.is_empty() {
                    return Some(exe_str.to_string());
                }
            }
        }

        // 3. working_dir + module.server_executable 자동 계산
        if let Some(ref work_dir) = instance.working_dir {
            if let Ok(modules) = self.module_loader.modules() {
                if let Some(server_exe) = modules.iter().find(|m| m.metadata.name == module_name).and_then(|m| m.metadata.server_executable.as_ref()) {
                    let resolved = std::path::Path::new(work_dir).join(server_exe);
                    return Some(resolved.to_string_lossy().to_string());
                }
            }
        }

        None
    }

    /// 해석된 실행 파일 경로를 config map에 삽입합니다.
    fn insert_resolved_exe(&self, config: &mut serde_json::Map<String, Value>, instance: &crate::instance::ServerInstance, module_name: &str) {
        if let Some(exe_path) = self.resolve_executable(instance, module_name) {
            config.insert("server_executable".to_string(), json!(&exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            config.insert("working_dir".to_string(), json!(work_dir));
        }
    }

    /// 모듈 함수가 인스턴스를 알아보는 데 쓰는 값 (포트·비밀번호·모듈 설정·데이터 폴더 등). REST 접속값도 모듈 설정이라
    /// 모듈이 선언한 설정 이름 그대로 실린다 (`[protocols.rest]`).
    /// 시작·정지·설정·초기화 모두 같은 값을 받아야 같은 서버 파일을 다룬다. 이미 있는 키는 두다.
    pub(crate) fn insert_instance_fields(cfg: &mut serde_json::Map<String, Value>, instance: &crate::instance::ServerInstance, instance_dir: &std::path::Path) {
        // 모듈이 서버 폴더 옆에 둔 데이터(좀보이드 cache_dir 등): 격리 실행이 쓸 수 있는 폴더로 싣는다
        cfg.insert(crate::sandbox::INSTANCE_DATA_DIRS.to_string(), json!(Self::instance_data_dirs(instance, instance_dir)));
        // RCON 접속 정보는 데몬이 만든 값이 기준이다 (RCON만 받는 모듈, `rcon.json`이 있을 때). 모듈은 이 값을 게임 설정에 쓴다
        if let Some(access) = crate::protocol::rcon::load(instance_dir) {
            cfg.insert("rcon_port".into(), json!(access.port));
            cfg.insert("rcon_password".into(), json!(access.password));
        }
        let mut put = |key: &str, value: Value| {
            cfg.entry(key.to_string()).or_insert(value);
        };
        // 인스턴스 폴더: 모듈이 그 안에 받아 둔 것(가져온 서버의 포터블 런타임 등)을 시작·점검 모두 같은 자리에서 본다
        put("instance_dir", json!(instance_dir.to_string_lossy()));
        for (key, value) in &instance.module_settings {
            // 격리 실행이 쓰기를 받는 폴더 키는 데몬만 정한다 (모듈은 import_settings로 아무 값이나 넣을 수 있다)
            // 호출마다 데몬이 정하는 키(`RESERVED_CALL_KEYS`)도 모듈 설정 값으로는 싣지 않는다
            if !crate::sandbox::grants_write(key) && !module_loader::RESERVED_CALL_KEYS.contains(&key.as_str()) {
                put(key, value.clone());
            }
        }
        put("protocol_mode", json!(&instance.protocol_mode));
        if let Some(v) = &instance.server_version { put("server_version", json!(v)); }
        // 모듈 도우미(daemon_stdin_command 등)가 데몬 API로 이 인스턴스를 가리킬 때 쓴다
        put("instance_id", json!(&instance.id));
        // 사용자가 더한 실행 인자. 넣을 자리를 아는 모듈은 직접 넣고 `extra_args_placed: true`를 돌려준다
        put("extra_args", json!(&instance.extra_args));
    }

    /// 모듈 설정 중 인스턴스 폴더 안을 가리키는 폴더: 그 인스턴스 폴더 바로 아래 항목 단위로 돌려준다.
    /// 인스턴스 폴더 자체, `..`가 든 경로, 데몬이 쓰는 항목(`instance.json`·`run/` 등)은 뺀다
    pub(crate) fn instance_data_dirs(instance: &crate::instance::ServerInstance, instance_dir: &std::path::Path) -> Vec<String> {
        use std::path::Component;
        let mut dirs: Vec<String> = instance
            .module_settings
            .values()
            .filter_map(Value::as_str)
            .filter_map(|value| {
                let rel = std::path::Path::new(value).strip_prefix(instance_dir).ok()?;
                if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
                    return None;
                }
                let first = rel.components().next()?.as_os_str().to_str()?;
                (!is_daemon_entry(first)).then(|| instance_dir.join(first).to_string_lossy().into_owned())
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// 인스턴스 id로 서버 상태를 본다 (응답의 `server`는 이름)
    /// Called by IPC API: GET /api/instance/:id/status
    pub async fn get_server_status(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str) -> Result<Value> {
        tracing::debug!("Getting status for server '{}'", instance_id);

        // 락 안에서는 값만 챙긴다. 익스텐션 hook과 모듈 status()는 락 밖에서 (C12)
        let (instance, hook) = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            let module = s.module_loader.get_module(module_name).ok();
            let mut ctx = crate::extension::hook_context(&instance, &s.instance_store.instance_dir(&instance.id), module.as_ref().map(|m| &m.metadata));
            ctx["process_patterns"] = json!(module.map(|m| m.metadata.process_patterns).unwrap_or_default());
            let hook = (s.extension_manager.clone(), ctx);
            (instance, hook)
        };

        // ── Extension hook: server.status ──
        {
            let (ext_mgr, ctx) = hook;
            let host_context = sup.read().await.host_context(&instance.id);
            let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.status", ctx)).await;
            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        return Ok(val.clone());
                    }
                }
            }
        }

        let (call, tracked_pid) = {
            let s = sup.read().await;
            // Managed 프로세스가 있으면 plugin 호출 없이 직접 판단
            if s.managed_store.get(&instance.id).await.is_some() {
                let (running, pid, start_time) = s.tracked_state(&instance.id).await;
                return Ok(json!({
                    "server": instance.name,
                    "status": if running { "running" } else { "stopped" },
                    "online": running,
                    "pid": pid,
                    "start_time": start_time,
                }));
            }

            // Non-managed: module plugin으로 상태 확인

            // tracker에서 PID를 먼저 확인 — start_server()가 저장한 PID
            let tracked_pid = s.tracker.get_pid(&instance.id).ok();

            // Find module
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            // Build config — 실행 파일, PID 등 Python status()에 필요한 값 포함
            let mut config_obj = serde_json::Map::new();
            s.insert_resolved_exe(&mut config_obj, &instance, module_name);
            if let Some(pid) = tracked_pid {
                config_obj.insert("pid".to_string(), json!(pid));
            }
            // module_settings 포함 (server_name 등 Python이 INI 경로 해석에 사용)
            for (key, value) in instance.module_settings.iter().filter(|(key, _)| !crate::sandbox::grants_write(key)) {
                config_obj.entry(key.clone()).or_insert_with(|| value.clone());
            }
            (crate::plugin::PluginCall::new(module_path, "status", Value::Object(config_obj)).serving(tracked_pid).in_instance(s.host_context(&instance.id)), tracked_pid)
        };

        // Ask module for status
        match call.run().await {
            Ok(result) => {
                // 모듈에 status가 없다: 데몬이 추적하는 PID가 살아 있는지로 답한다
                if result.get("error_code").and_then(Value::as_str) == Some("not_supported") {
                    let pid = tracked_pid.filter(|pid| process::is_process_alive(*pid));
                    let start_time = match pid {
                        Some(_) => sup.read().await.tracker.get_start_time(&instance.id).ok(),
                        None => None,
                    };
                    return Ok(json!({
                        "server": instance.name,
                        "status": if pid.is_some() { "running" } else { "stopped" },
                        "pid": pid,
                        "start_time": start_time,
                    }));
                }
                if result.get("success").and_then(Value::as_bool) == Some(false) {
                    return Ok(result);
                }
                let status = result.get("status").and_then(|s| s.as_str()).unwrap_or("unknown");
                let pid = result.get("pid").and_then(|p| p.as_u64());
                let start_time = match pid {
                    Some(_) => sup.read().await.tracker.get_start_time(&instance.id).ok(),
                    None => None,
                };
                Ok(json!({
                    "server": instance.name,
                    "status": status,
                    "pid": pid,
                    "start_time": start_time,
                }))
            },
            Err(e) => {
                tracing::error!("Failed to get status: {}", e);
                let mut result = crate::plugin::error_response(&e);
                result["server"] = json!(instance.name);
                result["status"] = json!("unknown");
                result["pid"] = Value::Null;
                Ok(result)
            }
        }
    }

    /// List all available modules
    pub fn list_modules(&self) -> Result<Vec<LoadedModule>> {
        self.module_loader.discover_modules()
    }

    /// 모듈 캐시를 새로고침하고 모든 모듈을 다시 발견합니다
    pub fn refresh_modules(&self) -> Result<Vec<LoadedModule>> {
        self.module_loader.invalidate_cache();
        self.module_loader.discover_modules()
    }

    /// 서버에 명령어 실행
    pub async fn execute_command(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        module_name: &str,
        command: &str,
        args: Value,
    ) -> Result<String> {
        let call = {
            let s = sup.read().await;
            tracing::info!(
                "Supervisor executing command '{}' for instance '{}' (module: {})",
                command,
                instance_id,
                module_name
            );

            // 인스턴스 정보 얻기
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            // 모듈 찾기
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            // 명령어 config 구성
            let pid = s.tracker.get_pid(&instance.id).ok();
            let instance_dir = s.instance_store.instance_dir(&instance.id);
            let working_dir = instance.working_dir.clone()
                .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());
            let config = json!({
                "command": command,
                "args": args,
                "protocol_mode": &instance.protocol_mode,
                "working_dir": &working_dir,
                "rest_host": module.metadata.rest_host(instance),
                "rest_port": module.metadata.rest_port(instance),
                "pid": pid,
                "instance_id": instance_id,
            });

            // 플러그인 실행
            crate::plugin::PluginCall::new(module_path, "command", config).serving(pid).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;

        // 결과 처리
        if let Some(success) = result.get("success").and_then(|s| s.as_bool()) {
            if success {
                let message = result
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default();
                Ok(message.to_string())
            } else {
                Err(module_failure(&result, module_name, "command"))
            }
        } else {
            Err(anyhow::anyhow!("Invalid response from module"))
        }
    }

    // ─── Managed Process Methods ─────────────────────────────

    /// 이 인스턴스를 관리형으로 시작하는가: saba-runner가 서버 콘솔(stdout·stdin)을 쥐고 앱 안으로 들여온다.
    /// 끄면 서버를 별도 창의 별도 프로세스로 띄워 사바쨩 콘솔·stdin을 쓸 수 없다.
    /// 인스턴스 설정 `managed_start`가 우선, 없으면 managed가 기본이다(오너 결정).
    /// non-managed는 프로세스를 직접 관리하고 싶은 사용자를 위한 선택지로 남긴다.
    pub fn uses_managed_start(&self, instance: &crate::instance::ServerInstance) -> bool {
        match instance.module_settings.get("managed_start") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) if s == "true" => true,
            Some(Value::String(s)) if s == "false" => false,
            // C8b: 모든 서버가 기본 managed (saba-runner가 PID·로그·종료 코드를 쥔다).
            // stdin이 없는 게임은 명령을 REST로, 정지를 모듈 stop()으로 처리한다.
            _ => true,
        }
    }

    /// 콘솔 명령을 stdin으로 보내야 하는가.
    /// 모듈이 `stdin`을 지원하고 stdin이 열린 managed 프로세스가 돌고 있으면 stdin을 쓴다
    /// (RCON은 보안상 늘 꺼져 있다).
    /// 데몬이 아는 실행 상태 `(실행 중, PID, 시작 시각)`: 서버 목록과 개별 상태 조회가 같이 쓴다.
    /// managed 프로세스가 있으면 그것이 살아 있는지로 정한다(끝났지만 모니터가 아직 치우지 않은 PID를 실행 중으로 보지 않는다).
    /// 없으면 추적 중인 PID가 있는지다
    pub async fn tracked_state(&self, instance_id: &str) -> (bool, Option<u32>, Option<u64>) {
        let pid = self.tracker.get_pid(instance_id).ok();
        let running = match self.managed_store.get(instance_id).await {
            Some(managed) => managed.is_running(),
            None => pid.is_some(),
        };
        if !running {
            return (false, None, None);
        }
        (true, pid, self.tracker.get_start_time(instance_id).ok())
    }

    /// 서버가 켜져 있는가: 추적표(non-managed와 managed 모두 띄운 뒤 올린다)나 managed 저장소에 살아 있으면.
    /// "켜져 있으면 거절" 판정은 모두 이것을 쓴다 (재부착 때 추적표 등록이 실패해 managed에만 남은 경우도 켜진 것이다)
    pub async fn is_running(&self, instance_id: &str) -> bool {
        self.tracker.get_pid(instance_id).is_ok() || self.managed_store.running_instance_ids().await.iter().any(|id| id == instance_id)
    }

    /// 이 감독자의 시작·설치와 자동 교체 사이의 문. 감독자 락을 놓은 뒤에 쥔다(문을 기다리는 동안 락을 잡고 있지 않게)
    pub fn addon_gate(&self) -> AddonSwapGate {
        self.addon_swap_gate.clone()
    }

    /// 인스턴스가 쓰는 익스텐션: 지금 모듈의 `[install] requires_extensions`와 인스턴스가 더한 것 (D2).
    /// 모듈을 업데이트해 요구가 바뀌면 기존 인스턴스에도 바로 맞는다. 예전에 만들 때 복사해 둔 모듈 목록은 그대로 읽는다
    pub fn effective_required_extensions(&self, instance: &crate::instance::ServerInstance) -> Vec<String> {
        let mut required = self
            .module_loader
            .get_module(&instance.module_name)
            .map(|m| m.required_extensions())
            .unwrap_or_default();
        for ext in &instance.required_extensions {
            if !required.contains(ext) {
                required.push(ext.clone());
            }
        }
        required
    }

    /// 켜져 있는 서버 인스턴스 id (`is_running` 판정)
    pub async fn running_instances(&self) -> Vec<String> {
        let mut running = Vec::new();
        for instance in self.instance_store.list() {
            if self.is_running(&instance.id).await {
                running.push(instance.id.clone());
            }
        }
        running
    }

    pub async fn routes_commands_to_stdin(&self, instance_id: &str) -> bool {
        if !self.module_accepts_stdin(instance_id) {
            return false;
        }
        match self.managed_store.get(instance_id).await {
            Some(proc) => proc.is_running() && proc.is_stdin_available(),
            None => false,
        }
    }

    /// 명령을 RCON으로 받을 인스턴스면 그 모듈의 기본 RCON 포트: RCON만 받는 모듈, 또는 stdin·rcon을 받는 모듈인데
    /// 관리형 시작을 꺼서 콘솔 stdin이 없는 인스턴스 (사용자 정의 2026-11-08)
    fn rcon_port_for(&self, instance: &crate::instance::ServerInstance) -> Option<u16> {
        let meta = &self.module_loader.get_module(&instance.module_name).ok()?.metadata;
        meta.rcon_only_port().or_else(|| meta.rcon_port_without_console().filter(|_| !self.uses_managed_start(instance)))
    }

    /// 명령을 RCON으로 받을 인스턴스면(`rcon_port_for`) 접속 정보를 만들어 둔다 (시작 전). 그 밖에는 아무것도 하지 않는다
    /// RCON을 더 쓰지 않으면(관리형 시작을 켬, 모듈 업데이트로 stdin·rest가 생김) 남은 접속 정보를 지운다.
    /// 준비하지 못하면 시작을 멈춘다: 비밀번호 없이 띄우면 명령을 보낼 길이 없다
    pub(crate) fn ensure_rcon_access(&self, instance: &crate::instance::ServerInstance) -> Result<()> {
        let dir = self.instance_store.instance_dir(&instance.id);
        let Some(port) = self.rcon_port_for(instance) else {
            let _ = std::fs::remove_file(dir.join(crate::protocol::rcon::ACCESS_FILE));
            return Ok(());
        };
        // 두 인스턴스가 동시에 처음 켜져도 같은 포트를 고르지 않게, 고르고 저장하는 동안 하나씩만
        static PICKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _picking = PICKING.lock().unwrap_or_else(|e| e.into_inner());
        // 다른 인스턴스의 RCON 포트와 저장해 둔 포트(게임·REST 등), 이 인스턴스의 포트는 고르지 않는다
        let taken: Vec<u16> = self
            .instance_store
            .list()
            .iter()
            .filter(|other| other.id != instance.id)
            .flat_map(|other| {
                let rcon = crate::protocol::rcon::load(&self.instance_store.instance_dir(&other.id)).map(|a| a.port);
                rcon.into_iter().chain(self.saved_ports(other))
            })
            .chain(self.saved_ports(instance))
            .collect();
        crate::protocol::rcon::ensure(&dir, port, &taken)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("Could not prepare RCON access for '{}': {}", instance.name, e))
    }

    /// 명령을 RCON으로 보낼 인스턴스면 접속 정보: 모듈이 RCON을 받고, 시작할 때 접속 정보를 만들어 둔 경우.
    /// 관리형 시작 설정은 보지 않는다: 켜진 동안 설정을 바꿔도(재시작 필요) 지금 도는 서버는 시작 때 방식 그대로다.
    /// 접속 정보는 시작 준비(`ensure_rcon_access`)만 만들고 지우므로 지금 도는 서버의 방식을 따른다
    pub fn rcon_route(&self, instance_id: &str) -> Option<crate::protocol::rcon::Access> {
        let instance = self.instance_store.get(instance_id)?;
        self.module_loader.get_module(&instance.module_name).ok()?.metadata.rcon_port_without_console()?;
        crate::protocol::rcon::load(&self.instance_store.instance_dir(instance_id))
    }

    /// 인스턴스의 모듈이 콘솔 입력(`protocols.supported`에 "stdin")을 받는가
    pub fn module_accepts_stdin(&self, instance_id: &str) -> bool {
        let Some(instance) = self.instance_store.get(instance_id) else { return false };
        let Ok(modules) = self.module_loader.modules() else { return false };
        modules
            .iter()
            .find(|m| m.metadata.name == instance.module_name)
            .is_some_and(|m| m.metadata.supports_protocol("stdin"))
    }

    /// Send a command to a managed process's stdin
    pub async fn send_stdin_command(&self, instance_id: &str, command: &str) -> Result<String> {
        let proc = self.managed_store.get(instance_id).await
            .ok_or_else(|| anyhow::anyhow!("No managed process for instance '{}'", instance_id))?;

        if !proc.is_running() {
            return Err(anyhow::anyhow!("Process is no longer running"));
        }

        proc.send_command(command).await?;
        Ok(format!("Sent to stdin: {}", command))
    }

    /// Get console output from a managed process.
    /// Falls back to reading from the disk log file if no managed process is
    /// currently registered (e.g. after daemon restart before reattachment, or
    /// for recently stopped servers).
    pub async fn get_console_output(
        &self,
        instance_id: &str,
        since_id: Option<u64>,
        count: Option<usize>,
    ) -> Result<Value> {
        // Try in-memory managed process first
        if let Some(proc) = self.managed_store.get(instance_id).await {
            let lines = if let Some(since) = since_id {
                proc.get_console_since(since).await
            } else {
                proc.get_recent_console(count.unwrap_or(100)).await
            };

            return Ok(json!({
                "success": true,
                "lines": lines,
                "epoch": proc.console_epoch().await,
                "running": proc.is_running(),
                // `/stdin`과 같은 판정: REST 전용 모듈은 러너가 stdin을 쥐어도 입력을 받지 않는다
                "stdin_available": self.module_accepts_stdin(instance_id) && proc.is_stdin_available(),
            }));
        }

        // Fallback: read from disk log
        let inst_dir = self.instance_store.instance_dir(instance_id);
        let log_path = managed_process::latest_console_log(&inst_dir);
        if log_path.exists() {
            let effective_count = count.unwrap_or(100);
            let dir_owned = inst_dir.clone();
            let lines = tokio::task::spawn_blocking(move || {
                managed_process::read_console_log_from_disk(&dir_owned, effective_count)
            }).await.unwrap_or_default();

            // Note: since_id is ignored for disk fallback because disk log IDs
            // are re-assigned from 0 on each read and don't correspond to the
            // in-memory session's IDs.

            return Ok(json!({
                "success": true,
                "lines": lines,
                "running": false,
                "source": "disk",
            }));
        }

        if self.instance_store.get(instance_id).is_none() {
            anyhow::bail!("Instance not found: {}", instance_id);
        }
        // 한 번도 시작하지 않은 서버: 빈 콘솔
        Ok(json!({ "success": true, "lines": [], "running": false, "source": "none" }))
    }

    /// `get_console_output`에 줄마다 `spans`(의미 토큰 구간, UTF-16 오프셋)를 붙인다 (`?spans=1`)
    pub async fn get_console_output_with_spans(
        &self,
        instance_id: &str,
        since_id: Option<u64>,
        count: Option<usize>,
    ) -> Result<Value> {
        let mut result = self.get_console_output(instance_id, since_id, count).await?;
        let module = self.instance_store.get(instance_id).map(|i| i.module_name.clone()).unwrap_or_default();
        let loaded = self.module_loader.get_module(&module).ok();
        let hl = highlight::for_module(&module, loaded.as_ref().and_then(|m| m.metadata.syntax_highlight.as_ref()));
        if let Some(lines) = result.get_mut("lines").and_then(Value::as_array_mut) {
            for line in lines {
                let spans: Vec<Value> = line
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|c| hl.spans(c).into_iter().map(|(s, e, t)| json!([s, e, t])).collect())
                    .unwrap_or_default();
                if let Some(obj) = line.as_object_mut() {
                    obj.insert("spans".to_string(), Value::Array(spans));
                }
            }
        }
        Ok(result)
    }

    /// Run validation on an instance (via module's validate function).
    /// 모듈 점검 결과의 `issues`에 데몬이 본 설정 값 문제(`setting_issues`)를 더한다 (사용자 지정 2026-11-08)
    pub async fn validate_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str) -> Result<Value> {
        let (call, settings) = {
            let s = sup.read().await;
            (s.instance_call(instance_id, "validate")?, s.setting_issues(instance_id))
        };
        let result = call.run().await?;
        Ok(with_setting_issues(result, settings))
    }

    /// 저장된 모듈 설정이 module.toml이 선언한 타입·범위·선택지·형식에 맞는지 (서버 점검).
    /// `visible_when`으로 숨긴 칸과 값이 없는 칸은 보지 않는다. 한 칸이 한 건이다:
    /// `{code: "setting_invalid", severity: "error", field, error_type, value, message}` (화면이 칸 이름과 error_type으로 번역한다)
    pub(crate) fn setting_issues(&self, instance_id: &str) -> Vec<Value> {
        let Some(instance) = self.instance_store.get(instance_id) else { return Vec::new() };
        let Ok(module) = self.module_loader.get_module(&instance.module_name) else { return Vec::new() };
        let Some(settings) = module.metadata.settings.as_ref() else { return Vec::new() };
        let hidden = crate::validator::hidden_fields(&settings.fields, |name| instance.module_settings.get(name).cloned());
        settings
            .fields
            .iter()
            .filter(|f| !hidden.contains(&f.name))
            .filter_map(|f| {
                let value = instance.module_settings.get(&f.name).filter(|v| !v.is_null())?;
                let e = crate::validator::validate_setting_value(f, Some(value)).err()?;
                // 비밀 칸(password 타입, 이름에 pass·token 같은 낱말)은 값도, 값이 든 원문 메시지도 싣지 않는다.
                // 원격 웹·디스코드도 점검을 부르고 이 응답은 비밀 가리기를 거치지 않는다 (리뷰 P2)
                let secret = f.field_type == "password" || crate::audit::sensitive(&f.name);
                let mut issue = json!({
                    "code": "setting_invalid",
                    "severity": "error",
                    "field": f.name,
                    "error_type": e.error_type,
                });
                if !secret {
                    issue["value"] = value.clone();
                    issue["message"] = json!(e.message);
                }
                Some(issue)
            })
            .collect()
    }

    /// 모듈 `[[queries]]` 조회 (X-2): `validate`와 같은 config에 `args`(입력 이름 → 값)를 더해 lifecycle `function`을 부른다
    pub async fn query_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str, function: &str, args: serde_json::Map<String, Value>) -> Result<Value> {
        let mut call = sup.read().await.instance_call(instance_id, function)?;
        if let Value::Object(cfg) = call.config_mut() {
            cfg.insert("args".to_string(), Value::Object(args));
        }
        call.run().await
    }

    /// 인스턴스를 알아보는 config로 lifecycle 함수 호출을 만든다 (`validate`·조회가 같은 값을 받는다)
    pub(crate) fn instance_call(&self, instance_id: &str, function: &str) -> Result<crate::plugin::PluginCall> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.module_loader.get_module(&instance.module_name)?;
        let module_path = module.entry_path();

        let mut cfg = serde_json::Map::new();
        // 시작 때와 같은 키와 같은 해석 (옛 module_settings 경로, 작업 폴더 + 모듈 server_executable) (D-S11)
        self.insert_resolved_exe(&mut cfg, instance, &instance.module_name);
        if let Some(port) = instance.port {
            cfg.insert("port".to_string(), json!(port));
        }
        // Extension 데이터 전달 (Python lifecycle 스크립트에서 참조)
        cfg.insert("extension_data".to_string(), json!(&instance.extension_data));
        // 시작 때와 같은 인스턴스 값 (zomboid server_name·cache_dir 등)
        Self::insert_instance_fields(&mut cfg, instance, &self.instance_store.instance_dir(instance_id));
        // 켜져 있으면 자기 포트를 "다른 프로그램이 쓰는 중"으로 보지 않게
        let server = self.tracker.get_pid(instance_id).ok();
        cfg.insert("running".to_string(), json!(server.is_some()));

        Ok(crate::plugin::PluginCall::new(module_path, function, Value::Object(cfg)).serving(server).in_instance(self.host_context(instance_id)))
    }

    /// 게임 설정 파일 읽기·쓰기·초기화·가져오기 (모듈 configure/read_properties/reset_properties/import_settings)
    pub async fn manage_properties(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        action: &str,  // "read" or "write"
        settings: Option<Value>,
    ) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            let module = s.module_loader.get_module(&instance.module_name)?;
            let module_path = module.entry_path();

            let mut cfg = serde_json::Map::new();
            // working_dir 결정: 명시적 설정 > executable_path의 부모 디렉토리
            let effective_working_dir = instance.working_dir.clone()
                .or_else(|| {
                    instance.executable_path.as_ref()
                        .and_then(|p| std::path::Path::new(p).parent())
                        .map(|p| p.to_string_lossy().to_string())
                });
            if let Some(work_dir) = &effective_working_dir {
                cfg.insert("working_dir".to_string(), json!(work_dir));
            }
            // Extension 데이터 전달 — Python lifecycle 스크립트가 플랫폼 경로를 올바르게 판단하도록
            cfg.insert("extension_data".to_string(), json!(&instance.extension_data));
            // 시작 때와 같은 인스턴스 값 (예: zomboid cache_dir — 다른 서버의 파일을 건드리지 않게)
            Self::insert_instance_fields(&mut cfg, instance, &s.instance_store.instance_dir(instance_id));
            if let Some(port) = instance.port {
                cfg.entry("port".to_string()).or_insert(json!(port));
            }

            let function = match action {
                "write" | "configure" => {
                    if let Some(s) = settings {
                        cfg.insert("settings".to_string(), s);
                    }
                    "configure"
                }
                "reset" => "reset_properties",
                // 지울 범위 (모듈 `reset_preview`가 밝힌 id). 없으면 모듈의 기본 범위
                "reset_server" => {
                    if let Some(scope) = settings {
                        cfg.insert("scope".to_string(), scope);
                    }
                    "reset_server"
                }
                "reset_preview" => "reset_preview",
                // 플레이어 목록 (`[[lists]]`)의 지금 명단
                "read_lists" => "read_lists",
                // 실제 설정 파일을 saba-chan 키로 (설정 화면이 캐시 대신 실제 값을 보여 준다)
                "import" => "import_settings",
                // 사바쨩이 고칠 수 있는 게임 설정 파일 (가져온 서버를 해제할 때 되돌린다)
                "config_files" => "config_files",
                _ => "read_properties",
            };

            crate::plugin::PluginCall::new(module_path, function, Value::Object(cfg)).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// 모듈이 선언한 약관(`[[agreements]]`)과 이 인스턴스의 동의 여부. 이름은 모듈 locales `agreements.<id>` (없으면 id)
    pub fn agreement_views(&self, instance: &crate::instance::ServerInstance, lang: &str) -> Vec<Value> {
        let Ok(module) = self.module_loader.get_module(&instance.module_name) else { return Vec::new() };
        if module.metadata.agreements.is_empty() {
            return Vec::new();
        }
        let lang = crate::chat::i18n::normalize_language(lang);
        let locales = [crate::actions::module_locale(&module, &lang), crate::actions::module_locale(&module, "en")];
        module.metadata.agreements.iter().map(|a| {
            let pointer = format!("/agreements/{}", a.id);
            let label = locales.iter().find_map(|l| l.pointer(&pointer).and_then(Value::as_str)).unwrap_or(&a.id);
            let at = instance.accepted_agreements.get(&a.id);
            json!({ "id": a.id, "label": label, "url": a.url, "accepted": at.is_some(), "accepted_at": at })
        }).collect()
    }

    /// 아직 동의하지 않은 약관 (켜기 전 점검)
    pub fn missing_agreements(&self, instance_id: &str, lang: &str) -> Vec<Value> {
        let Some(instance) = self.instance_store.get(instance_id) else { return Vec::new() };
        self.agreement_views(instance, lang).into_iter().filter(|v| v["accepted"] == json!(false)).collect()
    }

    /// 약관 동의를 기록하고 모듈 `accept_agreements`(`accepted_agreements: [id]`)로 모듈 파일(eula.txt 등)을 쓰게 한다.
    /// 모듈이 선언하지 않은 id면 Err(그 id)
    pub async fn accept_agreements(sup: &tokio::sync::RwLock<Self>, instance_id: &str, ids: &[String]) -> Result<std::result::Result<Value, String>> {
        let call = {
            let mut s = sup.write().await;
            let mut instance = s.instance_store.get(instance_id).cloned()
                .ok_or_else(|| InstanceNotFound(instance_id.to_string()))?;
            let module = s.module_loader.get_module(&instance.module_name)?;
            if let Some(unknown) = ids.iter().find(|id| !module.metadata.agreements.iter().any(|a| &a.id == *id)) {
                return Ok(Err(unknown.clone()));
            }
            let now = chrono::Utc::now().to_rfc3339();
            for id in ids {
                instance.accepted_agreements.entry(id.clone()).or_insert_with(|| now.clone());
            }
            let accepted: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
            s.instance_store.update(instance_id, instance)?;
            let mut call = s.instance_call(instance_id, "accept_agreements")?;
            if let Value::Object(cfg) = call.config_mut() {
                cfg.insert("accepted_agreements".to_string(), json!(accepted));
            }
            call
        };
        Ok(Ok(call.run().await?))
    }

    /// Diagnose errors from instance logs
    pub async fn diagnose_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            let module = s.module_loader.get_module(&instance.module_name)?;
            let module_path = module.entry_path();

            let mut cfg = serde_json::Map::new();
            if let Some(work_dir) = &instance.working_dir {
                cfg.insert("working_dir".to_string(), json!(work_dir));
            }

            // If managed, provide recent console output for diagnosis
            if let Some(proc) = s.managed_store.get(&instance.id).await {
                let recent = proc.get_recent_console(500).await;
                // 이번 실행의 줄만 (재부착 때 디스크에서 읽어 온 지난 실행의 오류를 지금 문제로 보지 않게)
                let log_lines: Vec<String> = recent
                    .iter()
                    .filter(|l| l.timestamp >= proc.started_at && l.source != managed_process::LogSource::Input)
                    .map(|l| l.content.clone())
                    .collect();
                cfg.insert("log_lines".to_string(), json!(log_lines));
            }

            crate::plugin::PluginCall::new(module_path, "diagnose_log", Value::Object(cfg)).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// 인스턴스마다 사용자가 런타임 경로 설정에 정한 값: `(모듈 진입 파일, 런타임 실행 파일 경로)` (`sandbox::reconcile_runtimes`)
    pub fn user_runtimes(&self) -> Vec<(String, String)> {
        self.instance_store
            .list()
            .iter()
            .filter_map(|inst| Some((inst, self.module_loader.get_module(&inst.module_name).ok()?)))
            .flat_map(|(inst, module)| {
                let entry = module.entry_path();
                module.metadata.user_runtime_paths(inst).into_iter().map(|path| (entry.clone(), path.to_string())).collect::<Vec<_>>()
            })
            .collect()
    }

    /// 기존 인스턴스의 누락 데이터 루트를 설정별로 한 번 가져온다. 모듈 호출은 데몬 잠금 밖에서 실행한다
    pub async fn backfill_data_roots(sup: std::sync::Arc<tokio::sync::RwLock<Self>>) {
        let ids: Vec<String> = sup.read().await.instance_store.list().iter().map(|i| i.id.clone()).collect();
        for id in ids {
            let (module_name, missing) = {
                let mut s = sup.write().await;
                let Some(mut instance) = s.instance_store.get(&id).cloned() else { continue };
                let Ok(module) = s.module_loader.get_module(&instance.module_name) else { continue };
                let missing: HashSet<String> = module.metadata.data_roots.iter()
                    .filter_map(|pattern| crate::module_paths::data_root_parts(pattern))
                    .map(|(key, _)| key)
                    .filter(|key| *key != "home" && !instance.module_settings.contains_key(*key)
                        && !instance.user_settings.contains(*key) && !instance.data_roots_backfill_attempted.contains(*key))
                    .map(str::to_string)
                    .collect();
                if missing.is_empty() {
                    continue;
                }
                // 빈 결과·실패·중단도 다음 기동에서 반복하지 않는다
                instance.data_roots_backfill_attempted.extend(missing.iter().cloned());
                let module_name = instance.module_name.clone();
                if let Err(e) = s.instance_store.update(&id, instance) {
                    tracing::warn!("Failed to record data root backfill for '{}': {}", id, e);
                    continue;
                }
                (module_name, missing)
            };
            let result = match Self::manage_properties(&sup, &id, "import", None).await {
                Ok(result) if result.get("success").and_then(Value::as_bool) == Some(true) => result,
                Ok(result) => {
                    tracing::warn!("Data root backfill skipped for '{}': {}", id, module_failure(&result, &module_name, "import_settings"));
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Data root backfill skipped for '{}': {}", id, e);
                    continue;
                }
            };
            let Some(settings) = result.get("settings").and_then(Value::as_object) else { continue };
            let mut s = sup.write().await;
            let Some(mut instance) = s.instance_store.get(&id).cloned() else { continue };
            if instance.module_name != module_name {
                continue;
            }
            let mut changed = false;
            for key in missing {
                if instance.module_settings.contains_key(&key) || instance.user_settings.contains(&key) {
                    continue;
                }
                if let Some(value) = settings.get(&key) {
                    instance.module_settings.insert(key.clone(), value.clone());
                    instance.imported_settings.insert(key);
                    changed = true;
                }
            }
            if changed {
                if let Err(e) = s.instance_store.update(&id, instance) {
                    tracing::warn!("Failed to save data root backfill for '{}': {}", id, e);
                }
            }
        }
    }

    /// 지금 실행 세대 (시작할 때마다 1씩 오른다)
    pub fn run_generation_of(&self, instance_id: &str) -> u64 {
        self.runtime.existing(instance_id, |r| r.run_generation).unwrap_or_default()
    }

    /// 포트 충돌 강제 정지 이벤트를 남긴다. 번호를 붙이고, 읽는 클라이언트가 없어도 최근 것만 둔다
    pub fn record_port_conflict_stops(&self, new: impl IntoIterator<Item = PortConflictStopEvent>) {
        if let Ok(mut stops) = self.port_conflict_stops.lock() {
            // 번호는 지금 시각(µs, JS 숫자로도 정확하다)보다 작지 않게 붙인다: 데몬을 다시 켜도 앞 실행의 번호보다 커서,
            // 클라이언트가 들고 있는 번호로 새 알림을 거르지 않는다
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0);
            for mut event in new {
                stops.last_id = (stops.last_id + 1).max(now);
                event.id = stops.last_id;
                stops.events.push(event);
            }
            stops.forget_old();
        }
    }

    /// 번호가 `after`보다 큰 알림. 읽어도 지우지 않는다 (`after`가 0이면 남은 것 모두)
    pub fn port_conflict_stops_after(&self, after: u64) -> Vec<PortConflictStopEvent> {
        self.port_conflict_stops
            .lock()
            .map(|mut stops| {
                stops.forget_old();
                stops.events.iter().filter(|e| e.id > after).cloned().collect()
            })
            .unwrap_or_default()
    }
}

/// 저장 후 종료를 기다리는 시간(초): 인스턴스 설정 `stop_timeout`(큰 월드는 저장이 길다, MK14), 모듈 `stop_timeout`, 기본 30.
/// 인스턴스 값은 1~3600초로 맞춘다
pub(crate) fn stop_timeout_of(instance: &crate::instance::ServerInstance, module_value: Option<u64>) -> u64 {
    instance.module_settings.get("stop_timeout")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
        .filter(|n| *n > 0)
        .map(|n| n.min(3600))
        .or(module_value)
        .unwrap_or(30)
}

/// 인스턴스 폴더 바로 아래 이름이 데몬 항목을 가리킬 수 있는가. Windows는 대소문자를 가리지 않고 끝의 점·공백을
/// 떼며, 짧은 이름(`SETTIN~1.JSO`)과 스트림 표기(`run:x`)로도 같은 항목을 연다. 그런 이름은 모두 데몬 항목으로 본다.
/// 대소문자는 유니코드로 견준다 (`ſ`도 대문자로는 `S`다)
pub(crate) fn is_daemon_entry(name: &str) -> bool {
    if name.contains(['~', ':']) {
        return true;
    }
    let name = name.trim_end_matches(['.', ' ']);
    let same = |a: &str, b: &str| a.to_uppercase() == b.to_uppercase() || a.to_lowercase() == b.to_lowercase();
    // 원자적 저장의 임시 파일 `.<항목>.<pid>-<n>.tmp`
    let temp = name.len() > 5 && name.starts_with('.') && name.is_char_boundary(name.len() - 4) && same(&name[name.len() - 4..], ".tmp");
    temp || crate::instance::DAEMON_ENTRIES.iter().any(|e| same(e, name))
}

/// 시작·정지를 시킨 쪽: `source`(gui·cli·discord·chatbot·web·remote·schedule·restart·daemon·game·external)와 사람·PC 이름.
/// 말하는 사바쨩은 디스코드와 가른다. 사용자 id·길드는 싣지 않는다. 원격 데몬으로 시킨 PC는 `remoteHost`(그 PC의 hostId)로
/// 알린다: 그 PC의 앱이 자기가 누른 것인지 알아 효과음을 내지 않는다
fn origin_fields(origin: &crate::audit::Origin) -> serde_json::Map<String, Value> {
    let source = if origin.via.as_deref() == Some("chatbot") { "chatbot" } else { origin.source.as_str() };
    let mut fields = serde_json::Map::new();
    fields.insert("source".into(), json!(source));
    if let Some(name) = &origin.user_name {
        fields.insert("userName".into(), json!(name));
    }
    if let Some(guild) = &origin.guild_name {
        fields.insert("guildName".into(), json!(guild));
    }
    if let Some(host) = origin.user.as_ref().filter(|_| source == "remote") {
        fields.insert("remoteHost".into(), json!(host));
    }
    fields
}

/// 모듈 점검 결과에 설정 값 문제를 더한다: `issues`가 있으면 뒤에 붙이고, 치명 문제가 아니므로 `success`는 그대로 둔다
fn with_setting_issues(mut result: Value, extra: Vec<Value>) -> Value {
    if extra.is_empty() {
        return result;
    }
    match &mut result {
        Value::Object(map) => match map.get_mut("issues") {
            Some(Value::Array(issues)) => issues.extend(extra),
            _ => {
                map.insert("issues".to_string(), Value::Array(extra));
            }
        },
        Value::Array(issues) => issues.extend(extra),
        _ => return json!({ "issues": extra }),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 켜고 끈 쪽: 디스코드에서 시켰으면 그 디스코드 서버 이름도 싣는다 (id는 싣지 않는다)
    #[test]
    fn origin_fields_carry_the_discord_server_name_but_no_ids() {
        let origin = crate::audit::Origin {
            user: Some("111".into()),
            guild: Some("222".into()),
            user_name: Some("주인님".into()),
            guild_name: Some("사바쨩 서버".into()),
            ..crate::audit::Origin::new("discord")
        };
        assert_eq!(
            Value::Object(origin_fields(&origin)),
            json!({"source": "discord", "userName": "주인님", "guildName": "사바쨩 서버"})
        );
        assert_eq!(Value::Object(origin_fields(&crate::audit::Origin::new("gui"))), json!({"source": "gui"}));
    }

    /// D3·A-09: 0.1.14-beta.1까지 instance.json에 있던 REST 접속값은 모듈이 선언한 설정으로 옮겨 저장한다.
    /// 모듈을 아직 모르는 인스턴스의 값은 파일에 그대로 둔다 (모듈이 생긴 뒤 옮긴다)
    #[tokio::test]
    async fn rest_values_of_old_instance_files_move_to_the_declared_settings_on_load() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("pal");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"pal\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"rest\"]\n\n\
             [protocols.rest]\nhost_setting = \"api_host\"\nport_setting = \"api_port\"\npassword_setting = \"admin_pw\"\nusername = \"admin\"\n\n\
             [install]\nmethod = \"steamcmd\"\napp_id = 2394010\n\n\
             [[settings.fields]]\nname = \"api_port\"\ntype = \"number\"\nport = true\nlabel = \"api\"\ndefault = 8212\n\n\
             [[settings.fields]]\nname = \"admin_pw\"\ntype = \"password\"\nlabel = \"pw\"\n",
        )
        .unwrap();
        let instances = tmp.path().join("instances");
        for (id, module) in [("old-pal", "pal"), ("old-gone", "gone")] {
            std::fs::create_dir_all(instances.join(id)).unwrap();
            let install_copies = json!({ "install_method_steamcmd": true, "steamcmd_app_id": 111, "steamcmd_anonymous": true, "steamcmd_beta": "unstable" });
            let file = json!({
                "id": id, "name": id, "module_name": module, "port": 8211, "extension_data": install_copies,
                "rest_host": "127.0.0.1", "rest_port": 8242, "rest_username": "admin", "rest_password": "pw",
            });
            std::fs::write(instances.join(id).join("instance.json"), file.to_string()).unwrap();
            std::fs::write(instances.join(id).join("settings.json"), json!({ "api_port": 8212 }).to_string()).unwrap();
        }
        let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), instances.to_str().unwrap());
        sup.initialize().await.unwrap();

        let saved = |id: &str, file: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(instances.join(id).join(file)).unwrap()).unwrap() };
        let moved = sup.instance_store.get("old-pal").unwrap();
        assert_eq!(moved.module_settings, saved("old-pal", "settings.json").as_object().unwrap().clone().into_iter().collect());
        assert_eq!(saved("old-pal", "settings.json"), json!({ "api_host": "127.0.0.1", "api_port": 8242, "admin_pw": "pw" }), "the port the daemon used wins over a stale setting");
        assert!(saved("old-pal", "instance.json").get("rest_port").is_none());
        // A-09: 인스턴스에 복사해 두던 `[install]` 값은 지우고, 설치 방식 표시와 사용자가 고른 브랜치만 남긴다
        assert_eq!(saved("old-pal", "instance.json")["extension_data"], json!({ "install_method_steamcmd": true, "steamcmd_beta": "unstable" }));
        let module = sup.module_loader.get_module("pal").unwrap();
        assert_eq!((module.metadata.rest_host(moved), module.metadata.rest_port(moved), module.metadata.rest_password(moved)), (Some("127.0.0.1".into()), Some(8242), Some("pw")));

        assert_eq!(saved("old-gone", "instance.json")["rest_port"], 8242, "kept until its module is known");
        assert_eq!(saved("old-gone", "settings.json"), json!({ "api_port": 8212 }));
    }

    /// A-06: 모듈에 `status`가 없으면 데몬이 추적하는 PID가 살아 있는지로 답한다 (PID만 보던 모듈 status 세 개를 대신한다)
    #[tokio::test]
    async fn a_module_without_status_is_answered_from_the_tracked_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nsys.stdin.read()\nok = sys.argv[1] == 'functions'\n\
             print(json.dumps({'success': True, 'functions': ['stop']} if ok else {'success': False, 'message': 'Unknown function: ' + sys.argv[1]}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let sup = tokio::sync::RwLock::new(sup);

        let stopped = Supervisor::get_server_status(&sup, &id, "game").await.unwrap();
        assert_eq!((stopped["status"].as_str(), stopped["pid"].as_u64()), (Some("stopped"), None), "{stopped}");
        sup.read().await.tracker.track(&id, std::process::id()).unwrap();
        let running = Supervisor::get_server_status(&sup, &id, "game").await.unwrap();
        assert_eq!((running["status"].as_str(), running["pid"].as_u64()), (Some("running"), Some(std::process::id() as u64)), "{running}");
        assert!(running["start_time"].is_number(), "{running}");
        // 추적표에 남았지만 이미 끝난 프로세스는 꺼진 것이다
        sup.read().await.tracker.track(&id, 4_000_001).unwrap();
        let gone = Supervisor::get_server_status(&sup, &id, "game").await.unwrap();
        assert_eq!(gone["status"], "stopped", "{gone}");
    }

    #[tokio::test]
    async fn test_supervisor_initialization() {
        let supervisor = Supervisor::new("./modules");
        
        // Supervisor가 정상적으로 생성되었는지 확인
        assert_eq!(supervisor.instance_store.list().len(), 0);
    }

    #[tokio::test]
    async fn test_module_discovery() {
        let supervisor = Supervisor::new("./modules");
        
        // 모듈 발견 (modules 디렉토리가 없어도 에러 없이 빈 리스트 반환)
        let modules = supervisor.list_modules();
        assert!(modules.is_ok());
    }

    #[tokio::test]
    async fn test_refresh_modules() {
        let supervisor = Supervisor::new("./modules");
        
        // 첫 번째 발견
        let _ = supervisor.list_modules();
        
        // 캐시 무효화 및 재발견
        let refreshed = supervisor.refresh_modules();
        assert!(refreshed.is_ok());
    }

    #[tokio::test]
    async fn test_execute_command_error_handling() {
        let supervisor = tokio::sync::RwLock::new(Supervisor::new("./modules"));
        
        // 존재하지 않는 인스턴스에 명령어 실행 시도
        let result = Supervisor::execute_command(
                &supervisor,
                "nonexistent-instance",
                "test-module",
                "test-command",
                json!({})
            )
            .await;
        
        // 에러가 발생해야 함
        assert!(result.is_err());
        
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Instance not found"));
    }

    /// 인스턴스 함수 호출은 추적 중인 서버 PID를 싣는다: 격리 실행의 루프백 중계가 그 서버의 포트로만 잇는다
    #[tokio::test]
    async fn an_instance_call_carries_the_tracked_server_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        assert_eq!(sup.instance_call(&id, "status").unwrap().server(), None);
        sup.tracker.track(&id, 4_000_001).unwrap();
        let call = sup.instance_call(&id, "status").unwrap();
        assert_eq!(call.server(), Some(4_000_001));
        assert_eq!(call.config()["running"], true);
    }

    /// 데몬이 모듈에 넘기는 값은 범용 키뿐이다: 실행 파일은 `server_executable`, 폴더는 `working_dir` (MK7).
    /// 설치된 버전 확인도 시작·점검과 같은 값을 받는다
    #[tokio::test]
    async fn module_calls_carry_only_generic_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True, 'config': json.load(sys.stdin)}))\n").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.module_loader.discover_modules().unwrap();
        let server_dir = tmp.path().join("server");
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.working_dir = Some(server_dir.to_string_lossy().into_owned());
        inst.executable_path = Some(server_dir.join("paper-1.21.jar").to_string_lossy().into_owned());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let validate = sup.instance_call(&id, "validate").unwrap();
        // 켜기 전 점검도 시작과 같이 인스턴스 폴더를 안다 (그 안에 받은 포터블 런타임을 본다)
        assert_eq!(validate.config()["instance_dir"], json!(sup.instance_store.instance_dir(&id).to_string_lossy()));
        let sup = tokio::sync::RwLock::new(sup);
        let installed = Supervisor::get_installed_version(&sup, "game", &id).await.unwrap();
        for config in [validate.config(), &installed["config"]] {
            assert_eq!(config["server_executable"], json!(server_dir.join("paper-1.21.jar").to_string_lossy()));
            assert_eq!(config["working_dir"], json!(server_dir.to_string_lossy()));
            for key in ["server_jar", "jar_name", "install_dir"] {
                assert!(config.get(key).is_none(), "{key} in {config}");
            }
        }
    }

    /// D-S11: 모듈 함수(설치 버전 확인 등)도 시작과 같은 실행 파일을 받는다. 옛 인스턴스의 module_settings 경로,
    /// 작업 폴더와 모듈의 server_executable로 정해지는 경로를 시작만 보고 이쪽은 보지 않았다
    #[test]
    fn module_calls_get_the_executable_the_start_uses() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nserver_executable = \"server.jar\"\n").unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.module_loader.discover_modules().unwrap();
        let server_dir = tmp.path().join("server");
        let mut legacy = crate::instance::ServerInstance::new("old", "game");
        legacy.working_dir = Some(server_dir.to_string_lossy().into_owned());
        legacy.module_settings.insert("executable_path".into(), json!(server_dir.join("custom.jar").to_string_lossy()));
        let mut plain = crate::instance::ServerInstance::new("new", "game");
        plain.working_dir = Some(server_dir.to_string_lossy().into_owned());
        for (inst, jar) in [(legacy, "custom.jar"), (plain, "server.jar")] {
            let id = inst.id.clone();
            let expected = sup.resolve_executable(&inst, "game");
            sup.instance_store.add(inst).unwrap();
            assert_eq!(expected.as_deref(), Some(server_dir.join(jar).to_string_lossy().as_ref()));
            let call = sup.instance_call(&id, "get_installed_version").unwrap();
            assert_eq!(call.config()["server_executable"], json!(expected), "{jar}");
        }
    }

    #[test]
    fn a_module_failure_says_why_instead_of_unknown_error() {
        assert_eq!(module_failure(&json!({"success": false, "message": "jar missing"}), "mc", "get_launch_command").to_string(), "jar missing");
        assert_eq!(module_failure(&json!({"success": false, "error": "bad port"}), "mc", "get_launch_command").to_string(), "bad port");
        let legacy = module_failure(&json!({"command": "sh", "args": []}), "e2e-mock", "get_launch_command").to_string();
        assert!(legacy.contains("e2e-mock") && legacy.contains("command") && legacy.contains("args"), "{}", legacy);
        assert!(module_failure(&Value::Null, "m", "f").to_string().contains("nothing"));
    }

    #[test]
    fn a_module_failure_keeps_its_fields_through_supervisor_errors() {
        let value = json!({ "success": false, "error_code": "module_path_outside", "message": "outside root", "path": "bad", "module": "game", "function": "get_launch_command" });
        let error = anyhow::anyhow!(module_failure(&value, "game", "get_launch_command"));
        assert_eq!(crate::plugin::error_response(&error), value);
    }

    #[test]
    fn module_failure_reads_as_the_module_message() {
        let value = json!({ "success": false, "error": "cannot start", "message": "jar missing", "module": "game", "function": "get_launch_command" });
        let error = module_failure(&value, "game", "get_launch_command");
        assert_eq!(error.to_string(), "jar missing");
        assert_eq!(crate::plugin::error_response(&error), value);
    }

    /// 서버 점검은 저장된 설정이 선언한 타입·범위·선택지에 맞는지도 본다 (숨긴 칸·빈 값은 빼고). 사용자 지정 2026-11-08
    #[test]
    fn the_server_check_reports_settings_that_do_not_match_their_declared_type() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"players\"\ntype = \"number\"\nmin = 1\nmax = 10\n\n\
             [[settings.fields]]\nname = \"mode\"\ntype = \"select\"\noptions = [\"a\", \"b\"]\n\n\
             [[settings.fields]]\nname = \"hard\"\ntype = \"boolean\"\n\n\
             [[settings.fields]]\nname = \"note\"\ntype = \"number\"\nvisible_when = { field = \"hard\", equals = true }\n\n\
             [[settings.fields]]\nname = \"admin_pass\"\ntype = \"password\"\npattern = \"[0-9]+\"\n\n\
             [[settings.fields]]\nname = \"api_token\"\ntype = \"number\"\n",
        )
        .unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut inst = crate::instance::ServerInstance::new("g", "game");
        inst.id = "g".into();
        for (k, v) in [
            ("players", json!(50)),
            ("mode", json!("minecraft\\:normal")),
            ("hard", json!(false)),
            ("note", json!("x")),
            ("admin_pass", json!("hunter2-secret")),
            ("api_token", json!("tok-secret")),
        ] {
            inst.module_settings.insert(k.into(), v);
        }
        sup.instance_store.add(inst).unwrap();
        let issues = sup.setting_issues("g");
        let fields: Vec<&str> = issues.iter().filter_map(|i| i["field"].as_str()).collect();
        assert_eq!(fields, ["players", "mode", "admin_pass", "api_token"], "note is hidden, hard is fine: {issues:?}");
        // 리뷰 P2 (2026-11-08): 비밀 칸(password 타입·이름에 pass·token 등)은 값과 원문 메시지를 싣지 않는다 (원격 웹도 점검을 부른다)
        let text = serde_json::to_string(&issues).unwrap();
        assert!(!text.contains("hunter2") && !text.contains("tok-secret"), "{text}");
        assert!(issues[2].get("value").is_none() && issues[3].get("value").is_none());
        assert_eq!((issues[0]["code"].as_str(), issues[0]["error_type"].as_str()), (Some("setting_invalid"), Some("out_of_range")));
        assert_eq!(issues[1]["error_type"], "invalid_option");
        // 모듈 결과에 붙인다 (success는 그대로)
        let merged = with_setting_issues(json!({ "success": true, "issues": [{ "code": "X" }] }), issues.clone());
        assert_eq!((merged["success"].clone(), merged["issues"].as_array().unwrap().len()), (json!(true), 5));
        assert_eq!(with_setting_issues(json!({ "success": true }), issues)["issues"].as_array().unwrap().len(), 4);
    }

    /// 06 §2: 1분 접속자 수 갱신에 사용자 설정 `players_detail`이 따라가지 않는다
    #[test]
    fn a_module_setting_never_stands_in_for_a_daemon_call_key() {
        let mut inst = crate::instance::ServerInstance::new("mc", "minecraft");
        inst.module_settings.insert("players_detail".into(), json!(true));
        inst.module_settings.insert("stop_call_limit".into(), json!(9999));
        let mut cfg = serde_json::Map::new();
        Supervisor::insert_instance_fields(&mut cfg, &inst, std::path::Path::new("/data/instances/mc"));
        assert!(cfg.get("players_detail").is_none() && cfg.get("stop_call_limit").is_none(), "{cfg:?}");
    }

    // PC [A]: 미확인 좀보이드의 데이터 폴더(<인스턴스>/zomboid)가 서버 폴더 옆이라 격리 실행이 쓰지 못했다.
    // 인스턴스 폴더 안을 가리키는 모듈 설정 폴더만 쓰기 대상으로 싣고, 데몬 파일(instance.json 등)과 인스턴스
    // 폴더 자체는 싣지 않는다. 모듈 설정에서 온 폴더 키(리뷰 312: import_settings로 아무 경로나)는 config에 넣지 않는다
    #[test]
    fn modules_get_write_folders_only_inside_their_instance() {
        let root = std::path::Path::new("/data/instances/pz");
        let at = |rel: &str| root.join(rel).to_string_lossy().into_owned();
        let mut inst = crate::instance::ServerInstance::new("pz", "zomboid");
        for (key, value) in [
            ("cache_dir", at("zomboid")),
            ("save_dir", at("zomboid/Saves")),
            ("meta", at("instance.json")),
            ("runner", at("run/x")),
            // Windows는 이름의 대소문자, 끝의 점·공백, 짧은 이름(8.3), 스트림 표기를 같은 항목으로 연다
            ("runner_upper", at("RUN/x")),
            ("meta_dot", at("Instance.json.")),
            ("logs_space", at("logs /x")),
            ("short", at("SETTIN~1.JSO")),
            ("stream", at("run:x")),
            ("long_s", at("ſettings.json.tmp")),
            ("atomic_temp", at(".settings.json.42-0.TMP")),
            ("korean", at("세이브")),
            ("up", at("../other")),
            ("whole", root.to_string_lossy().into_owned()),
            ("outside", "/home/user/Documents".to_string()),
            ("install_dir", "/home/user".to_string()),
            ("working_dir", "/home/user".to_string()),
            ("instance_dir", "/home/user".to_string()),
        ] {
            inst.module_settings.insert(key.into(), json!(value));
        }
        let mut cfg = serde_json::Map::new();
        Supervisor::insert_instance_fields(&mut cfg, &inst, root);
        assert_eq!(cfg["instance_data_dirs"], json!([at("zomboid"), at("세이브")]));
        for key in ["install_dir", "working_dir"] {
            assert!(cfg.get(key).is_none(), "{key} from module settings");
        }
        assert_eq!(cfg["instance_dir"], json!(root.to_string_lossy()), "the daemon's instance folder, not the module setting");
        assert_eq!(cfg["cache_dir"], json!(at("zomboid")), "the setting itself still reaches the module");
    }

    /// 시작 경로 정책 (C8): 인스턴스 설정 > 모듈이 콘솔 없음을 밝혔는지 > managed 기본
    #[test]
    fn managed_start_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        for (name, mode) in [("console-game", "\ninteraction_mode = \"console\""), ("api-game", "\ninteraction_mode = \"commands\""), ("plain-game", "")] {
            let dir = modules_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
            std::fs::write(
                dir.join("module.toml"),
                format!("[module]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"rcon\"]{mode}\n"),
            )
            .unwrap();
        }
        let supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let inst = |module: &str, setting: Option<Value>| {
            let mut i = crate::instance::ServerInstance::new("s", module);
            if let Some(v) = setting {
                i.module_settings.insert("managed_start".into(), v);
            }
            i
        };

        assert!(supervisor.uses_managed_start(&inst("console-game", None)));
        assert!(supervisor.uses_managed_start(&inst("plain-game", None)), "managed is the default");
        assert!(supervisor.uses_managed_start(&inst("missing-module", None)));
        assert!(supervisor.uses_managed_start(&inst("api-game", None)), "games without a console are managed too (C8b)");
        assert!(!supervisor.uses_managed_start(&inst("console-game", Some(json!(false)))));
        assert!(!supervisor.uses_managed_start(&inst("console-game", Some(json!("false")))));
        assert!(!supervisor.uses_managed_start(&inst("api-game", Some(json!(false)))));
    }

    /// 크래시 직후(모니터가 치우기 전) 들어온 정지는 크래시로 기록하되 다시 켜지 않는다 (D-S6)
    /// 모듈·익스텐션 자동 교체가 문을 쥔 동안 서버 시작은 기다린다: 교체 직전의 사용 확인과 교체 사이에 서버가 켜지지 않는다
    #[tokio::test]
    async fn starts_wait_while_addons_are_being_swapped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let gate = supervisor.addon_gate().try_hold_starts().expect("no one else starts on this supervisor");
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(supervisor));
        let managed = tokio::spawn({
            let sup = sup.clone();
            async move { Supervisor::start_managed_server(&sup, "none", "game", json!({})).await }
        });
        let plain = tokio::spawn({
            let sup = sup.clone();
            async move { Supervisor::start_server(&sup, "none", "game", json!({})).await }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!managed.is_finished() && !plain.is_finished(), "starts must wait for the swap");
        drop(gate);
        assert!(managed.await.unwrap().is_err(), "then it runs (no such instance here)");
        let _ = plain.await.unwrap();
    }

    /// 시작·설치가 문을 읽기로 쥔 동안 자동 교체는 기다리지 않고 미룬다: 기다리는 교체 뒤에 다른 시작·설치가 줄 서지 않는다
    #[tokio::test]
    async fn an_addon_swap_gives_up_instead_of_queueing_ahead_of_starts() {
        let tmp = tempfile::tempdir().unwrap();
        let supervisor = || Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let gate = supervisor().addon_gate();
        let install = gate.hold_addons().await;
        assert!(gate.try_hold_starts().is_none(), "a swap does not run under an install");
        assert!(supervisor().addon_gate().try_hold_starts().is_some(), "another supervisor's install does not hold this one's swaps");
        let another = tokio::time::timeout(Duration::from_millis(500), gate.hold_addons()).await;
        assert!(another.is_ok(), "nothing waits behind a swap that gave up");
        drop((install, another));
    }

    /// 필수 익스텐션은 지금 모듈의 `[install] requires_extensions`에 인스턴스가 더한 것이다 (시작 검사와 자동 교체 보류가 같은 답).
    /// 만들 때의 모듈 목록만 들고 있는 인스턴스도 모듈이 새로 요구하는 것을 받는다
    #[tokio::test]
    async fn an_instance_needs_the_module_requirements_and_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nrequires_extensions = [\"steamcmd\"]\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        assert_eq!(supervisor.effective_required_extensions(&inst), vec!["steamcmd".to_string()]);
        inst.required_extensions = vec!["docker".into(), "steamcmd".into()];
        assert_eq!(supervisor.effective_required_extensions(&inst), vec!["steamcmd".to_string(), "docker".to_string()]);
    }

    /// 시작·정지마다 누가 시켰는지 `instance.started`·`instance.stopped`에 싣는다 (출처는 요청의 `audit::Origin`).
    /// 예기치 않은 것은 사바쨩이 띄우지 않은 프로세스의 시작(`unexpected`)과 크래시·강제 종료(`instance.exit`)뿐이다
    #[cfg(unix)]
    #[tokio::test]
    async fn starts_and_stops_say_who_asked() {
        use crate::audit::{with_origin, Origin};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let games = [
            ("long", "", "sleep 30"),
            ("quits", "[detection]\ngraceful_exit_patterns = [\"Stopping the server\"]\n", "sleep 0.3; echo Stopping the server; exit 0"),
            ("boom", "", "sleep 0.3; kill -SEGV $$"),
        ];
        for (name, extra, sh) in games {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n{extra}")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), format!(
                "import json, sys\n\
                 if sys.argv[1] == 'get_launch_command':\n    \
                 print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n"
            )).unwrap();
            crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        }
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for (id, module) in [("w-long", "long"), ("w-quits", "quits"), ("w-boom", "boom"), ("w-ext", "long")] {
            let mut inst = crate::instance::ServerInstance::new(id, module);
            inst.id = id.into();
            if id == "w-ext" {
                inst.process_name = Some("fakegame".into());
            }
            sup.instance_store.add(inst).unwrap();
        }
        let ext_dir = sup.instance_store.instance_dir("w-ext").to_string_lossy().to_string();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let mut sub = crate::events::bus().subscribe(None);
        // 받은 이벤트를 모아 두고, 찾은 것만 꺼낸다
        let mut got: Vec<(String, Value)> = Vec::new();
        let mut seen = move |kind: &str, id: &str| -> Vec<Value> {
            got.extend(std::iter::from_fn(|| sub.rx.try_recv().ok()).map(|e| (e.kind.clone(), e.data.clone())));
            let (hit, rest) = std::mem::take(&mut got).into_iter().partition(|(k, d)| k == kind && d["instanceId"] == id);
            got = rest;
            hit.into_iter().map(|(_, d)| d).collect()
        };

        // 사바쨩을 거친 시작·정지: 출처를 싣고 예기치 않은 것이 아니다
        let chatbot = Origin { via: Some("chatbot".into()), user_name: Some("Dana".into()), ..Origin::new("discord") };
        let paths = [
            (Origin::new("gui"), "gui"),
            (Origin::new("cli"), "cli"),
            (Origin { user: Some("42".into()), user_name: Some("Dana".into()), ..Origin::new("discord") }, "discord"),
            (chatbot, "chatbot"),
            (Origin::new("web"), "web"),
            (Origin { user: Some("host-a".into()), user_name: Some("집 PC".into()), ..Origin::new("remote") }, "remote"),
            (Origin::new("schedule"), "schedule"),
            (Origin::new("restart"), "restart"),
            (Origin::new("daemon"), "daemon"),
        ];
        for (origin, source) in paths {
            let user = origin.user_name.clone();
            let started = with_origin(origin.clone(), Supervisor::start_managed_server(&sup, "w-long", "long", json!({}))).await.unwrap();
            assert_eq!(started["success"], true, "{started}");
            let stopped = with_origin(origin, Supervisor::stop_server(&sup, "w-long", "long", true)).await.unwrap();
            assert_eq!(stopped["success"], true, "{stopped}");
            let started = seen("instance.started", "w-long");
            assert_eq!(started.len(), 1, "{source}: {started:?}");
            assert_eq!((started[0]["source"].as_str(), started[0]["unexpected"].as_bool()), (Some(source), Some(false)), "{source}");
            assert_eq!(started[0]["userName"].as_str(), user.as_deref(), "{source}");
            // 원격으로 시킨 PC만 알린다 (디스코드 사용자 id는 싣지 않는다)
            assert_eq!(started[0]["remoteHost"].as_str(), (source == "remote").then_some("host-a"), "{source}");
            let stopped = seen("instance.stopped", "w-long");
            assert_eq!(stopped.len(), 1, "{source}: {stopped:?}");
            assert_eq!((stopped[0]["source"].as_str(), stopped[0]["kind"].as_str()), (Some(source), Some("user_stop")), "{source}");
            assert_eq!(stopped[0]["userName"].as_str(), user.as_deref(), "{source}");
            assert_eq!(stopped[0]["remoteHost"].as_str(), (source == "remote").then_some("host-a"), "{source}");
        }

        let wait_exit = |id: &'static str| {
            let sup = sup.clone();
            async move {
                for _ in 0..100 {
                    if !sup.read().await.managed_store.running_instance_ids().await.iter().any(|i| i == id) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                panic!("{id} did not exit");
            }
        };
        // 사바쨩 밖의 프로세스: 데몬이 켜질 때 이미 돌던 것은 알리지 않는다. 그 뒤에 나타난 것만 예기치 않은 시작이다
        let ext = crate::process_monitor::RunningProcess { pid: 4_000_001, name: "fakegame".into(), executable_path: None, cmd: Vec::new(), cwd: Some(ext_dir) };
        sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
        assert!(seen("instance.started", "w-ext").is_empty(), "a server already running when the daemon started is not unexpected");
        // 사바쨩이 시키지 않았는데 사라졌다: 원인 모를 종료
        sup.write().await.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
        let gone = seen("instance.exit", "w-ext");
        assert_eq!(gone.iter().map(|e| e["kind"].clone()).collect::<Vec<_>>(), vec![json!("unknown_exit")]);
        sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
        let started = seen("instance.started", "w-ext");
        assert_eq!(started.len(), 1, "{started:?}");
        assert_eq!((started[0]["source"].as_str(), started[0]["unexpected"].as_bool()), (Some("external"), Some(true)));
        // 사바쨩이 띄우는 중(아직 추적 전)에 모니터가 먼저 본 프로세스는 밖에서 켠 것이 아니다
        sup.write().await.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
        seen("instance.exit", "w-ext");
        sup.read().await.runtime.with("w-ext", |r| r.starting = true);
        sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
        sup.read().await.runtime.with("w-ext", |r| r.starting = false);
        assert!(seen("instance.started", "w-ext").is_empty(), "a server saba-chan is starting is not unexpected");

        // 끝까지 판정한 정지 뒤의 모니터 순회가 같은 실행을 원인 모를 종료로 다시 보지 않는다
        sup.write().await.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
        assert!(seen("instance.exit", "w-long").iter().all(|e| e["kind"] == "user_stop"), "a judged stop is not judged again");

        // 게임 안에서 끈 것: 정상 정지 (출처 game)
        with_origin(Origin::new("gui"), Supervisor::start_managed_server(&sup, "w-quits", "quits", json!({}))).await.unwrap();
        wait_exit("w-quits").await;
        sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
        let stopped = seen("instance.stopped", "w-quits");
        assert_eq!(stopped.len(), 1, "{stopped:?}");
        assert_eq!((stopped[0]["source"].as_str(), stopped[0]["kind"].as_str()), (Some("game"), Some("in_game_stop")));

        // 크래시: instance.exit 만 (정상 정지 알림은 없다)
        with_origin(Origin::new("gui"), Supervisor::start_managed_server(&sup, "w-boom", "boom", json!({}))).await.unwrap();
        wait_exit("w-boom").await;
        sup.write().await.monitor_processes_taken_at(std::slice::from_ref(&ext), u64::MAX).await.unwrap();
        assert!(seen("instance.stopped", "w-boom").is_empty());
        let exits = seen("instance.exit", "w-boom");
        assert_eq!(exits.iter().map(|e| e["kind"].clone()).collect::<Vec<_>>(), vec![json!("crash")]);
    }

    /// managed 모드에서는 모듈이 stdin을 지원할 때만 명령을 stdin으로 보낸다 (M3)
    #[cfg(unix)]
    #[tokio::test]
    async fn commands_route_to_stdin_only_for_managed_stdin_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        for (name, protocols) in [("console-game", r#"["rcon", "stdin"]"#), ("rest-game", r#"["rest"]"#)] {
            let dir = modules_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
            std::fs::write(
                dir.join("module.toml"),
                format!(
                    "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = {protocols}\n"
                ),
            )
            .unwrap();
        }
        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            instances_dir.to_str().unwrap(),
        );

        for (id, module) in [("console", "console-game"), ("rest", "rest-game"), ("idle", "console-game")] {
            let mut inst = crate::instance::ServerInstance::new(id, module);
            inst.id = id.to_string();
            supervisor.instance_store.add(inst).unwrap();
        }
        for id in ["console", "rest"] {
            let proc = managed_process::ManagedProcess::spawn(
                "cat",
                &[],
                tmp.path().to_str().unwrap(),
                Vec::new(),
                None,
                None,
            )
            .await
            .unwrap();
            supervisor.managed_store.insert(id, proc).await;
        }

        assert!(supervisor.routes_commands_to_stdin("console").await);
        assert!(!supervisor.routes_commands_to_stdin("rest").await, "module without stdin keeps RCON/REST");
        assert!(!supervisor.routes_commands_to_stdin("idle").await, "no managed process keeps RCON");

        for id in ["console", "rest"] {
            if let Some(proc) = supervisor.managed_store.get(id).await {
                let _ = process::force_kill_pid(proc.pid);
            }
        }
    }

    /// `?spans=1`: 디스크 로그 줄에도 모듈 규칙의 토큰 구간이 붙고, 없는 요청에는 안 붙는다
    #[tokio::test]
    async fn console_spans_use_module_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("hl-game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"hl-game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[syntax_highlight.rules]]\nname = \"join\"\npattern = '(?P<hl>\\w+) joined'\ncolor = \"player\"\n",
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut inst = crate::instance::ServerInstance::new("srv", "hl-game");
        inst.id = "srv".into();
        let mut supervisor = supervisor;
        supervisor.instance_store.add(inst).unwrap();
        let log = managed_process::console_log_path(&supervisor.instance_store.instance_dir("srv"));
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "[2026-10-17 10:00:00] [STDOUT/INFO] Steve joined the game\n").unwrap();

        let with = supervisor.get_console_output_with_spans("srv", None, Some(10)).await.unwrap();
        assert_eq!(with["source"], "disk");
        assert_eq!(with["lines"][0]["spans"], json!([[0, 5, "player"]]));
        let without = supervisor.get_console_output("srv", None, Some(10)).await.unwrap();
        assert!(without["lines"][0].get("spans").is_none());
    }

    /// REST 전용 모듈의 서버는 러너가 stdin을 쥐어도(의사 콘솔) 콘솔 입력을 받지 않는다:
    /// 콘솔 응답의 `stdin_available`은 `/stdin`이 받는지와 같다 (다르면 클라이언트가 입력칸을 열고 409를 받는다)
    #[cfg(unix)]
    #[tokio::test]
    async fn console_input_follows_the_module_protocols() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        for (name, supported) in [("rest-game", "[\"rest\"]"), ("stdin-game", "[\"stdin\"]")] {
            let dir = modules_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
            std::fs::write(
                dir.join("module.toml"),
                format!("[module]\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = {supported}\n"),
            )
            .unwrap();
        }
        let mut supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for (id, module) in [("rest", "rest-game"), ("stdin", "stdin-game")] {
            let mut inst = crate::instance::ServerInstance::new(id, module);
            inst.id = id.into();
            supervisor.instance_store.add(inst).unwrap();
            let proc = managed_process::ManagedProcess::spawn("/bin/sh", &["-c".into(), "sleep 30".into()], ".", Vec::new(), None, None)
                .await
                .unwrap();
            supervisor.managed_store.insert(id, proc).await;
        }
        let rest = supervisor.get_console_output("rest", None, Some(10)).await.unwrap();
        let stdin = supervisor.get_console_output("stdin", None, Some(10)).await.unwrap();
        for id in ["rest", "stdin"] {
            let _ = supervisor.managed_store.get(id).await.unwrap().force_kill().await;
        }
        assert_eq!(rest["stdin_available"], false, "{rest}");
        assert_eq!(stdin["stdin_available"], true, "{stdin}");
    }

    /// 데몬을 다시 켜도 알림 번호는 앞 실행보다 크다: 클라이언트가 들고 있는 번호 뒤의 새 알림을 놓치지 않는다 (L50)
    #[test]
    fn notice_ids_keep_growing_across_a_daemon_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let event = || PortConflictStopEvent {
            id: 0,
            stopped_name: "b".into(),
            stopped_id: "b-id".into(),
            existing_name: "a".into(),
            existing_id: "a-id".into(),
            port: 1,
            timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        };
        let daemon = || Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let first = daemon();
        first.record_port_conflict_stops([event(), event()]);
        let seen = first.port_conflict_stops_after(0).last().unwrap().id;
        let restarted = daemon();
        restarted.record_port_conflict_stops([event()]);
        assert_eq!(restarted.port_conflict_stops_after(seen).len(), 1, "a client that saw id {seen} gets the new notice");
    }

    /// 시작 중(준비 문구를 기다림)이고 채팅 연동이 켜진 서버를 지우면, 상태를 버리기 전에 준비 알림은 시작 실패로,
    /// 연동은 프로세스가 끝난 것으로 닫아 알린다 (지우기 전과 같은 알림)
    #[cfg(unix)]
    #[tokio::test]
    async fn removing_a_starting_instance_closes_its_ready_wait_and_chat_links() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let inst = crate::instance::ServerInstance::new("half-up", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let proc = Arc::new(ManagedProcess::spawn("/bin/sh", &["-c".into(), "sleep 30".into()], ".", Vec::new(), None, None).await.unwrap());
        let mut sub = crate::events::bus().subscribe(None);
        let started = Ok(json!({ "success": true, "managed": true }));
        ready::after_start(&sup.runtime, &id, "half-up", &started, Some(proc.clone()), &["^never$".to_string()], std::time::Duration::from_secs(600));
        chat_link::toggle(&id, "111111111111111111", "222222222222222222", None, |_| Ok::<(), ()>(())).unwrap();

        sup.remove_instance(&id).unwrap();
        let (mut ready_seen, mut link_seen) = (None, None);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        while ready_seen.is_none() || link_seen.is_none() {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Ok(e) = tokio::time::timeout(left, sub.rx.recv()).await else { break };
            let e = e.unwrap();
            if e.data["instanceId"] != id.as_str() {
                continue;
            }
            match e.kind.as_str() {
                "instance.ready" => ready_seen = Some((e.data["ok"].clone(), e.data["code"].clone())),
                "instance.chat_link" => link_seen = Some((e.data["active"].clone(), e.data["reason"].clone())),
                _ => {}
            }
        }
        let _ = proc.force_kill().await;
        assert_eq!(ready_seen, Some((json!(false), json!("start_failed"))));
        assert_eq!(link_seen, Some((json!(false), json!("stopped"))));
        assert!(chat_link::list(&id).is_empty());
    }
}
