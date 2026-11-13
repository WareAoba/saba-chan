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
mod instance_config;
mod status;
mod console;
mod queries;
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

    /// List all available modules
    pub fn list_modules(&self) -> Result<Vec<LoadedModule>> {
        self.module_loader.discover_modules()
    }

    /// 모듈 캐시를 새로고침하고 모든 모듈을 다시 발견합니다
    pub fn refresh_modules(&self) -> Result<Vec<LoadedModule>> {
        self.module_loader.invalidate_cache();
        self.module_loader.discover_modules()
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

#[cfg(test)]
mod tests;
