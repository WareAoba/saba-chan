pub mod process;
pub mod module_loader;
pub mod managed_process;
pub mod crash;
pub mod highlight;
pub mod players;
pub mod ready;
pub mod player_events;

use anyhow::Result;
use process::ProcessTracker;
use module_loader::{ModuleLoader, LoadedModule};
use managed_process::{ManagedProcess, ManagedProcessStore};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
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
    /// 매개변수 없이 읽는 옛 클라이언트에게 마지막으로 보인 번호 (한 번 읽으면 비던 예전 동작을 지운 것 없이 흉내 낸다)
    legacy_seen: u64,
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
    /// Stop 이후 auto-detect 억제 쿨다운 (instance_id → 정지 시각)
    stop_cooldowns: HashMap<String, Instant>,
    /// 익스텐션 매니저 (Phase 3에서 optional로 추가)
    pub extension_manager: Option<std::sync::Arc<tokio::sync::RwLock<crate::extension::ExtensionManager>>>,
    /// 프로비저닝 진행 상태 추적 (auto-detect 억제용)
    pub provision_tracker: Option<crate::ipc::ProvisionTracker>,
    /// 포트 충돌로 인한 강제 정지 알림 (클라이언트가 `GET /api/servers`로 읽는다)
    port_conflict_stops: std::sync::Mutex<PortConflictStops>,
    /// GUI 설정: 포트 충돌 검사 건너뛰기 (portConflictCheck 비활성화 시 true)
    pub skip_port_check: std::sync::Arc<AtomicBool>,
    /// 데몬이 정지를 보낸 인스턴스. 이 상태로 끝나면 사용자 정지다 (A1)
    stop_requested: HashSet<String>,
    /// 자동 재시작 시각 (한도 계산용)
    restart_history: HashMap<String, VecDeque<Instant>>,
    /// 종료 이력 (GET /api/instance/:id/events)
    exit_events: HashMap<String, VecDeque<crash::ExitEvent>>,
    /// 모니터가 정한 재시작 (인스턴스 ID, 대기, 예약 때의 실행 세대). 데몬 루프가 가져가 실행한다
    pending_restarts: Vec<(String, Duration, u64)>,
    /// 정지 요청과 시작마다 오르는 인스턴스별 세대. 예약된 재시작은 세대가 그대로일 때만 실행한다
    run_generation: HashMap<String, u64>,
    /// 지금 시작 중인 인스턴스. 시작은 락을 놓고 진행하므로 중복 시작을 이것으로 막는다 (C12)
    starting: std::sync::Arc<std::sync::Mutex<HashSet<String>>>,
    pub(crate) host_runtime: std::sync::OnceLock<crate::plugin::host::Runtime>,
}

/// 시작하는 동안 쥐는 표시. 놓으면(drop) 지워진다
struct StartGuard {
    set: std::sync::Arc<std::sync::Mutex<HashSet<String>>>,
    id: String,
    /// 시작을 받았을 때의 실행 세대. 정지 요청은 세대를 올리므로, 띄우기 전후에 이 값과 다르면
    /// 시작 준비 중에 정지가 들어온 것이다 (그 정지는 PID가 없어 바로 성공으로 끝난다)
    generation: u64,
}

tokio::task_local! {
    /// 시작 요청을 받았을 때의 실행 세대. 시작 API는 모듈 validate(시작 전 점검)를 끝낸 뒤 시작을 부르므로,
    /// 점검 중에 들어온 정지도 잡도록 그 전에 잡은 세대를 `StartGuard`의 기준으로 쓴다
    pub static START_SINCE: u64;

    /// 이 작업 안에서 부른 정지가 올린 뒤의 실행 세대. 재시작이 자기 정지 뒤의 기준으로 쓴다 (D-S10)
    pub static STOPPED_AT: std::cell::Cell<Option<u64>>;
}

/// `StartGuard`의 기준 세대: 시작 API가 미리 잡아 두었으면 그 값, 아니면 지금 값
fn start_baseline(s: &Supervisor, id: &str) -> u64 {
    START_SINCE.try_with(|g| *g).unwrap_or_else(|_| s.run_generation_of(id))
}

impl StartGuard {
    fn claim(set: &std::sync::Arc<std::sync::Mutex<HashSet<String>>>, id: &str, generation: u64) -> Option<Self> {
        let fresh = set.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        fresh.then(|| Self { set: set.clone(), id: id.to_string(), generation })
    }

    /// 시작 준비 중에 정지(또는 삭제)가 들어왔는가
    fn stop_requested(&self, s: &Supervisor) -> bool {
        s.run_generation_of(&self.id) != self.generation || s.instance_store.get(&self.id).is_none()
    }
}

/// 시작 전에 예약 범위를 피해 옮긴 포트를 시작 응답(`success:false`도)에 싣는다: `port_moved: [{field, from, to}]`.
/// 응답을 받지 않는 경로를 위해 옮길 때 `instance.port_moved` 이벤트도 낸다 (`move_default_rest_port_if`)
fn with_port_moved(mut result: Value, moved: Option<(u16, u16)>) -> Value {
    if let (Some((from, to)), Value::Object(map)) = (moved, &mut result) {
        map.insert("port_moved".to_string(), json!([{ "field": "rest_port", "from": from, "to": to }]));
    }
    result
}

/// 시작 준비 중에 정지가 들어와 띄우지 않았다 (또는 띄운 것을 바로 껐다)
fn start_cancelled(server_name: &str) -> Value {
    json!({
        "success": false,
        "error": "start_cancelled",
        "error_code": "start_cancelled",
        "message": format!("Server '{}' was stopped while it was starting", server_name),
    })
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        self.set.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
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

/// `get_launch_command` 결과의 실행 인자. 모듈이 추가 인자를 넣지 않았으면 끝에 붙인다
pub(crate) fn launch_args(launch_result: &Value, instance: &crate::instance::ServerInstance) -> Vec<String> {
    let mut args: Vec<String> = launch_result
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if launch_result.get("extra_args_placed").and_then(Value::as_bool) != Some(true) {
        args.extend(instance.extra_args.iter().cloned());
    }
    args
}

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

/// `original` 옆의 비어 있는 이름(`이름.bak`, `이름.1.bak`, …)에 복사본을 만든다. 이미 있는 파일은 덮지 않는다
fn backup_beside(original: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let name = original.file_name().unwrap_or_default().to_string_lossy().into_owned();
    for n in 0..100 {
        let backup = original.with_file_name(if n == 0 { format!("{name}.bak") } else { format!("{name}.{n}.bak") });
        let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&backup) {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let copied = std::fs::File::open(original)
            .and_then(|mut input| std::io::copy(&mut input, &mut out))
            .and_then(|_| out.sync_all())
            .and_then(|_| std::fs::set_permissions(&backup, std::fs::metadata(original)?.permissions()));
        return match copied {
            Ok(()) => Ok(backup),
            Err(e) => {
                drop(out);
                let _ = std::fs::remove_file(&backup);
                Err(e)
            }
        };
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("no free backup name next to {name}")))
}

/// 같은 파일을 가리키는가 (있으면 실제 경로로, 없으면 쓴 그대로 비교한다)
fn same_file_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

impl Supervisor {
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
            stop_cooldowns: HashMap::new(),
            extension_manager: None,
            provision_tracker: None,
            port_conflict_stops: std::sync::Mutex::new(PortConflictStops::default()),
            skip_port_check: std::sync::Arc::new(AtomicBool::new(false)),
            stop_requested: HashSet::new(),
            restart_history: HashMap::new(),
            exit_events: HashMap::new(),
            pending_restarts: Vec::new(),
            run_generation: HashMap::new(),
            starting: Default::default(),
            host_runtime: Default::default(),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // 모듈 발견
        let modules = self.module_loader.discover_modules()?;
        tracing::info!("Loaded {} modules", modules.len());
        for module in modules {
            tracing::info!("  - {} v{}", module.metadata.name, module.metadata.version);
        }
        
        // 인스턴스 로드
        self.instance_store.load()?;
        tracing::info!("Loaded {} server instances", self.instance_store.list().len());

        self.migrate_list_settings();

        // 고아 managed 프로세스 재연결
        self.reattach_orphaned_managed().await;
        
        Ok(())
    }

    /// 모듈이 `list`로 바꾼 설정의 옛 문자열 값을 배열로 읽는다 (06 §8.1). 모듈을 다시 읽은 뒤에도 부른다
    pub fn migrate_list_settings(&mut self) {
        let loader = &self.module_loader;
        self.instance_store.adjust_in_memory(|instance| {
            if let Some(settings) = loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.settings.clone()) {
                crate::validator::migrate_list_values(&settings.fields, &mut instance.module_settings);
            }
        });
    }

    /// Detect orphaned managed processes from a previous daemon run and restore
    /// their console log history from disk.
    ///
    /// For each instance that has a `managed.pid` file:
    /// 1. Read the PID and check if it's still alive
    /// 2. If alive: re-register in tracker + create a stub ManagedProcess with
    ///    console history loaded from `logs/console.log`
    /// 3. If dead: clean up the stale PID file
    async fn reattach_orphaned_managed(&mut self) {
        let instances: Vec<_> = self.instance_store.list().to_vec();
        let mut reattached = 0u32;

        for instance in &instances {
            let inst_dir = self.instance_store.instance_dir(&instance.id);
            let pid_file = inst_dir.join("managed.pid");

            // saba-runner가 쥔 서버면 콘솔·stdin까지 그대로 다시 붙는다 (C8b)
            let log_pattern = self.module_loader.get_module(&instance.module_name)
                .ok()
                .and_then(|m| m.metadata.log_pattern.clone());
            if let Some(proc) = managed_process::ManagedProcess::attach_runner(&inst_dir, log_pattern.as_deref()).await {
                let pid = proc.pid;
                // 가동 시간은 데몬이 다시 붙은 때가 아니라 러너가 서버를 띄운 때부터
                if let Err(e) = self.tracker.track_since(&instance.id, pid, proc.started_at) {
                    tracing::warn!("Failed to re-track '{}' (PID {}): {}", instance.name, pid, e);
                    continue;
                }
                let _ = saba_atomic_file::write(&pid_file, pid.to_string().as_bytes(), saba_atomic_file::Mode::Plain);
                self.managed_store.insert(&instance.id, proc).await;
                reattached += 1;
                tracing::info!("Reattached '{}' through saba-runner (PID {})", instance.name, pid);
                continue;
            }

            let pid_str = match std::fs::read_to_string(&pid_file) {
                Ok(s) => s.trim().to_string(),
                Err(_) => continue, // no managed.pid → not a managed instance
            };

            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    let _ = std::fs::remove_file(&pid_file);
                    continue;
                }
            };

            // 기록한 뒤에 시작한 프로세스는 재부팅 뒤 같은 PID를 받은 다른 프로세스다 (기록 시각 = managed.pid를 쓴 때)
            let recorded_at = std::fs::metadata(&pid_file)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            let same = match recorded_at {
                Some(at) => process::is_same_process(pid, at),
                None => process::is_process_alive(pid),
            };
            if same {
                // Process is still running — re-register it
                if let Err(e) = self.tracker.track(&instance.id, pid) {
                    tracing::warn!("Failed to re-track orphan '{}' (PID {}): {}", instance.name, pid, e);
                    continue;
                }

                // Create a stub ManagedProcess with history from disk
                let stub = managed_process::ManagedProcess::create_reattached_stub(
                    pid,
                    &inst_dir,
                ).await;

                self.managed_store.insert(&instance.id, stub).await;
                reattached += 1;
                tracing::info!(
                    "Reattached orphaned managed process '{}' (PID {})",
                    instance.name, pid
                );
            } else {
                // Process is dead (or the PID now belongs to another process) — clean up stale PID file
                tracing::info!(
                    "Cleaning up stale managed.pid for '{}' (PID {} is no longer the server)",
                    instance.name, pid
                );
                let _ = std::fs::remove_file(&pid_file);
            }
        }

        if reattached > 0 {
            tracing::info!("Reattached {} orphaned managed process(es)", reattached);
        }
    }

    /// 로드된 모듈로부터 module_name → protocols_supported 맵을 빌드합니다.
    /// `check_port_conflicts`에 전달하여 미지원 프로토콜 포트를 충돌 검사에서 제외합니다.
    pub(crate) fn build_module_protocols_map(&self) -> HashMap<String, Vec<String>> {
        let mut map = HashMap::new();
        if let Ok(modules) = self.module_loader.discover_modules() {
            for m in modules {
                if let Some(protocols) = m.metadata.protocols_supported {
                    map.insert(m.metadata.name, protocols);
                }
            }
        }
        map
    }
    /// 0.1.6까지 만든 인스턴스는 모듈 기본 REST 포트(팰월드 8212)를 그대로 저장해 두었다. 그 포트가 Windows 예약
    /// 범위에 들면 서버가 REST를 열지 못하므로, 시작 전에 새 인스턴스처럼 예약 범위와 다른 인스턴스의 포트를 피해
    /// 옮겨 저장한다(모듈이 시작 때 게임 설정 파일을 이 값으로 맞춘다). 기본값이 아닌 값은 사용자가 정한 것이므로
    /// 옮기지 않는다(시작 전 검사가 port_conflict로 알린다)
    /// 옮겼으면 `(옮기기 전, 옮긴 뒤)`
    fn move_reserved_default_rest_port(&mut self, id: &str) -> Option<(u16, u16)> {
        // 이미 떠 있는 서버는 지금 포트로 붙어 있다
        if self.tracker.get_pid(id).is_ok_and(process::is_process_alive) {
            return None;
        }
        let before = self.instance_store.get(id)?.rest_port?;
        self.move_default_rest_port_if(id, crate::validator::tcp_port_reserved_by_windows, crate::validator::port_reserved_by_windows)
            .map(|after| (before, after))
    }

    fn move_default_rest_port_if(&mut self, id: &str, reserved: impl Fn(u16) -> bool, avoid: impl Fn(u16) -> bool) -> Option<u16> {
        let instance = self.instance_store.get(id)?.clone();
        let rest = instance.rest_port?;
        let module = self.module_loader.get_module(&instance.module_name).ok()?;
        let supports_rest = module.metadata.protocols_supported.as_deref().unwrap_or(&[]).iter().any(|p| p == "rest");
        if !supports_rest || rest != module.metadata.default_rest_port() || !reserved(rest) {
            return None;
        }
        let port_fields_of = |name: &str| -> Vec<String> {
            self.module_loader.get_module(name).ok()
                .and_then(|m| m.metadata.settings.as_ref().map(|s| s.port_fields().into_iter().map(str::to_string).collect()))
                .unwrap_or_default()
        };
        let mut taken = std::collections::HashSet::new();
        for other in self.instance_store.list().iter().filter(|o| o.id != id) {
            let fields = port_fields_of(&other.module_name);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            taken.extend(crate::validator::instance_ports(other, &fields).into_iter().map(|(_, p)| p));
        }
        let free = crate::validator::next_free_port(rest, |p| taken.contains(&p) || avoid(p))?;
        let mut moved = instance;
        moved.rest_port = Some(free);
        if moved.module_settings.contains_key("rest_port") {
            moved.module_settings.insert("rest_port".to_string(), json!(free));
        }
        let name = moved.name.clone();
        if let Err(e) = self.instance_store.update(id, moved) {
            tracing::warn!("Instance '{}': could not save the moved REST port: {}", name, e);
            return None;
        }
        tracing::info!("Instance '{}': REST port {} is reserved by Windows, using {}", name, rest, free);
        // 자동·예약 재시작처럼 시작 응답을 아무도 보지 않는 경로에서도 바뀐 포트를 알 수 있게
        crate::events::emit("instance.port_moved", json!({ "instanceId": id, "instance": name, "field": "rest_port", "from": rest, "to": free }));
        Some(free)
    }

    /// 시작 전 포트 검사. 실행 중인 다른 인스턴스와 겹치거나 다른 프로그램이 잡고 있는 포트가 있으면
    /// `port_conflict` 응답을 돌려준다. 끄는 방법은 전역 설정(gui.portConflictCheck) 하나뿐이다
    fn port_conflict_response(&self, instance: &crate::instance::ServerInstance) -> Option<Value> {
        if self.skip_port_check.load(Ordering::Relaxed) {
            return None;
        }
        let all_instances = self.instance_store.list();
        let running_ids: std::collections::HashSet<String> = all_instances
            .iter()
            .filter(|i| self.tracker.get_pid(&i.id).is_ok())
            .map(|i| i.id.clone())
            .collect();
        let module_protocols = self.build_module_protocols_map();
        let mut details: Vec<String> = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, Some(&module_protocols))
            .iter()
            .map(|c| c.to_string())
            .collect();
        // 인스턴스끼리 겹치지 않을 때만 다른 프로그램이 잡은 포트를 직접 묶어 본다.
        // 이 인스턴스가 이미 실행 중이면 그 포트는 자기 것이므로 건너뛴다
        if details.is_empty() && !running_ids.contains(&instance.id) {
            let module = self.module_loader.get_module(&instance.module_name).ok();
            let port_fields = module.as_ref().and_then(|m| m.metadata.settings.as_ref()).map(|s| s.port_fields()).unwrap_or_default();
            let ports = crate::validator::start_check_ports(instance, Some(&module_protocols), &port_fields);
            details = crate::validator::ports_held_elsewhere(&ports)
                .into_iter()
                .map(|(name, port)| format!("Port {} ({}) is in use by another program", port, name))
                .collect();
            // 예약 범위의 포트는 아무도 쓰지 않아도 서버가 열지 못한다 (사용 중 목록에 없어 따로 본다)
            let reserved: Vec<String> = crate::validator::ports_reserved_by_windows(&ports)
                .into_iter()
                .filter(|(_, port)| !details.iter().any(|d| d.starts_with(&format!("Port {} ", port))))
                .map(|(name, port)| format!("Port {} ({}) is reserved by Windows (excluded port range); choose another port", port, name))
                .collect();
            details.extend(reserved);
        }
        if details.is_empty() {
            return None;
        }
        tracing::warn!("Port conflict detected for '{}': {:?}", instance.name, details);
        Some(json!({
            "success": false,
            "error": "port_conflict",
            "error_code": "port_conflict",
            "message": format!("Cannot start '{}': port conflict detected", instance.name),
            "conflicts": details,
        }))
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
            if let Ok(module) = self.module_loader.get_module(module_name) {
                if let Some(ref server_exe) = module.metadata.server_executable {
                    let resolved = std::path::Path::new(work_dir).join(server_exe);
                    return Some(resolved.to_string_lossy().to_string());
                }
            }
        }

        None
    }

    /// 실행 인자 미리보기: 저장된 설정 위에 초안(`draft`)을, 그 위에 추가 인자(`extra`, 없으면 저장된 것)에서 읽은 필드를 얹어
    /// `get_launch_command`를 `preview: true`로 부른다. 익스텐션 hook·서버 시작·파일 쓰기는 없다
    /// (모듈은 `preview`이면 설정 파일을 고치지 않는다). 응답 모양은 `docs/architecture/01-daemon-core.md`
    pub async fn launch_preview(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        draft: &serde_json::Map<String, Value>,
        extra: Option<&[String]>,
    ) -> Result<Value> {
        let (call, fields, settings, preview_instance) = {
            let s = sup.read().await;
            let instance = s
                .instance_store
                .get(instance_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", instance_id))?;
            let module = s.module_loader.get_module(&instance.module_name)?;
            let fields = module.metadata.settings.as_ref().map(|m| m.fields.clone()).unwrap_or_default();

            let draft_errors = crate::launch_args::draft_errors(&fields, draft);
            // 추가 인자를 보내지 않았으면 저장된 것으로
            let bound = crate::launch_args::bind(&fields, extra.unwrap_or(&instance.extra_args));
            let mut settings: serde_json::Map<String, Value> = instance.module_settings.clone().into_iter().collect();
            for (k, v) in draft {
                if !draft_errors.iter().any(|e| &e.field == k) {
                    settings.insert(k.clone(), v.clone());
                }
            }
            settings.extend(bound.settings.clone());

            let mut preview_instance = instance.clone();
            preview_instance.module_settings = settings.clone().into_iter().collect();
            preview_instance.extra_args = bound.extra.clone();
            let mut cfg = serde_json::Map::new();
            s.insert_resolved_exe(&mut cfg, &preview_instance, &instance.module_name);
            if let Some(port) = settings.get("port").cloned().or_else(|| instance.port.map(|p| json!(p))) {
                cfg.insert("port".to_string(), port);
            }
            Self::insert_instance_fields(&mut cfg, &preview_instance, &s.instance_store.instance_dir(&instance.id));
            cfg.insert("managed".to_string(), json!(s.uses_managed_start(&preview_instance)));
            cfg.insert("preview".to_string(), json!(true));
            let call = crate::plugin::PluginCall::new(module.entry_path(), "get_launch_command", Value::Object(cfg)).in_instance(s.host_context(&instance.id));

            let mut errors = draft_errors;
            errors.extend(bound.errors.clone());
            let base = json!({
                "has_launch_args": crate::launch_args::has_launch_args(&fields),
                "settings_from_args": bound.settings,
                "extra_args": bound.extra,
                "extra_args_text": crate::launch_args::join_args(&bound.extra),
                "errors": errors,
            });
            ((call, base), fields, settings, preview_instance)
        };
        let (call, mut out) = call;
        // 실행 인자로 들어가는 설정이 없는 모듈은 부르지 않는다 (preview를 모르는 모듈이 파일을 고칠 수 있다)
        if !crate::launch_args::has_launch_args(&fields) {
            out.as_object_mut().expect("object").insert("success".into(), json!(true));
            return Ok(out);
        }
        let launch_result = call.run().await?;
        let obj = out.as_object_mut().expect("object");
        if launch_result.get("success").and_then(Value::as_bool) != Some(true) {
            obj.extend(launch_result.as_object().cloned().unwrap_or_default());
            obj.insert("success".into(), json!(false));
            if let Some(action) = launch_result.get("action_required") {
                obj.insert("action_required".into(), action.clone());
            } else if launch_result.get("success").and_then(Value::as_bool) != Some(false) {
                obj.insert("error".into(), json!("launch_command_failed"));
            }
            let message = launch_result.get("message").and_then(Value::as_str).map(str::to_string);
            let message = message.or_else(|| {
                launch_result.get("action_required").is_none().then(|| module_failure(&launch_result, &preview_instance.module_name, "get_launch_command").to_string())
            });
            if let Some(m) = message {
                obj.insert("message".into(), json!(m));
            }
            return Ok(out);
        }
        let program = launch_result.get("program").and_then(Value::as_str).unwrap_or("").to_string();
        let mut args = launch_args(&launch_result, &preview_instance);
        let rest_password = preview_instance.rest_password.as_deref().unwrap_or("");
        let rcon = crate::protocol::rcon::load(&sup.read().await.instance_store.instance_dir(instance_id));
        let rcon_password = rcon.as_ref().map(|a| a.password.as_str()).unwrap_or("");
        crate::launch_args::mask_secrets(&fields, &settings, &[rest_password, rcon_password], &mut args);
        obj.insert("success".into(), json!(true));
        obj.insert("command_line".into(), json!(crate::launch_args::command_line(&program, &args)));
        obj.insert("program".into(), json!(program));
        obj.insert("args".into(), json!(args));
        Ok(out)
    }

    /// 확인되지 않은 모듈의 실행 대상 검사 (Windows). 띄울 프로그램 경로를 돌려주고, 막으면 `action_required` 응답
    async fn guard_launch(
        sup: &tokio::sync::RwLock<Self>,
        instance: &crate::instance::ServerInstance,
        module_name: &str,
        program: &str,
        args: &[String],
        working_dir: &str,
        env_vars: &mut Vec<(String, String)>,
    ) -> Result<(String, crate::sandbox::LaunchGate), Value> {
        let (entry, roots) = {
            let s = sup.read().await;
            let mut dirs = vec![s.instance_store.instance_dir(&instance.id)];
            // 사용자가 정한 서버 폴더 (모듈이 돌려준 working_dir은 믿지 않는다)
            dirs.extend(instance.working_dir.iter().map(std::path::PathBuf::from));
            let Ok(module) = s.module_loader.get_module(module_name) else {
                // 모듈을 못 찾으면 예전처럼 검사 없이 띄우되 서버 폴더의 표시는 걷는다
                drop(s);
                let roots = crate::sandbox::LaunchRoots { dirs, runtimes: Vec::new() };
                return crate::sandbox::guard_launch_for(|| saba_chan_updater_lib::integrity::Trust::Verified, program, args, std::path::Path::new(working_dir), roots, env_vars)
                    .await
                    .map_err(|blocked| blocked.to_json(crate::sandbox::message(&blocked)));
            };
            // 사용자가 설정한 java 경로 (이름이 java일 때만 런타임으로 인정된다).
            // 모듈 기본값·프리셋처럼 사용자가 정하지 않은 값은 인정하지 않는다
            let mut runtimes: Vec<_> = instance.module_settings.get("java_path").and_then(Value::as_str)
                .filter(|_| instance.user_settings.contains("java_path"))
                .map(std::path::PathBuf::from)
                .filter(|p| crate::sandbox::plain_local_path(p))
                .into_iter().collect();
            // 데몬의 JAVA_HOME·PATH가 가리키는 JDK (격리 실행에 읽기를 준 런타임과 같다)
            runtimes.extend(crate::sandbox::daemon_runtimes());
            (module.entry_path(), crate::sandbox::LaunchRoots { dirs, runtimes })
        };
        let (program, gate) = crate::sandbox::guard_launch(&entry, program, args, std::path::Path::new(working_dir), roots, env_vars).await.map_err(|blocked| {
            tracing::warn!("Launch of '{}' blocked: {:?}", instance.name, blocked);
            blocked.to_json(crate::sandbox::message(&blocked))
        })?;
        tracing::info!("Starting '{}': program '{}' in '{}'", instance.name, program, working_dir);
        Ok((program, gate))
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

    /// 모듈 함수가 인스턴스를 알아보는 데 쓰는 값 (포트·비밀번호·모듈 설정·REST·데이터 폴더 등).
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
        // REST 포트는 데몬이 접속하는 인스턴스 값이 기준이다 (정지도 그 값을 쓴다). 모듈 설정에 남은 옛 값
        // (예: 설치가 기본 INI에서 읽어 온 8212)이 먼저 들어가면 모듈이 INI를 그 값으로 맞춰 데몬이 못 붙는다
        if let Some(v) = instance.rest_port { put("rest_port", json!(v)); }
        for (key, value) in &instance.module_settings {
            // 격리 실행이 쓰기를 받는 폴더 키는 데몬만 정한다 (모듈은 import_settings로 아무 값이나 넣을 수 있다)
            // 호출마다 데몬이 정하는 키(`RESERVED_CALL_KEYS`)도 모듈 설정 값으로는 싣지 않는다
            if !crate::sandbox::grants_write(key) && !module_loader::RESERVED_CALL_KEYS.contains(&key.as_str()) {
                put(key, value.clone());
            }
        }
        if let Some(v) = &instance.rest_host { put("rest_host", json!(v)); }
        if let Some(v) = &instance.rest_username { put("rest_username", json!(v)); }
        if let Some(v) = &instance.rest_password { put("rest_password", json!(v)); }
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

    /// 모듈 `[module.dependencies]`를 못 맞추면 시작을 막는 응답. 다 맞으면 None
    async fn unmet_dependencies_response(&self, module_name: &str, instance_name: &str) -> Option<Value> {
        // 데몬이 모자라 아예 로드하지 못한 모듈도 같은 응답을 준다
        let failed = self.module_loader.get_module(module_name).err().and_then(|_| self.module_loader.load_failure(module_name));
        // 데몬보다 새 스키마는 그 이유 그대로 준다 (클라이언트가 "업데이트가 필요해요"로 보인다)
        if let Some(f) = failed.as_ref().filter(|f| f.error_code.as_deref() == Some("module_schema_unsupported")) {
            tracing::warn!("Module '{}' needs a newer saba-chan for '{}': {}", module_name, instance_name, f.error);
            return Some(json!({
                "success": false,
                "error": "module_schema_unsupported",
                "error_code": "module_schema_unsupported",
                "message": format!("Cannot start '{}': {}", instance_name, f.error),
                "module": module_name,
                "required": f.required,
            }));
        }
        let unmet = match failed {
            // 의존성 때문에 로드하지 못한 것만. 스키마가 틀린 모듈 등은 빈 의존성 오류로 바꾸지 않는다
            Some(f) if f.error_code.as_deref() == Some("module_dependency_unsatisfied") => f.missing_dependencies,
            _ => self.unmet_module_dependencies(module_name).await?,
        };
        tracing::warn!("Module '{}' dependencies unmet for '{}': {}", module_name, instance_name, unmet.join(", "));
        Some(json!({
            "success": false,
            "error": "module_dependency_unsatisfied",
            "error_code": "module_dependency_unsatisfied",
            "message": format!("Cannot start '{}': module '{}' needs {}", instance_name, module_name, unmet.join(", ")),
            "module": module_name,
            "missing_dependencies": unmet,
        }))
    }

    /// 모듈 `[module.dependencies]` 중 못 맞춘 것 (`"ext-steamcmd >=1.0.0 (installed 0.9.0)"` 꼴). 다 맞으면 None.
    /// 데몬(`saba-core`)과 설치·활성화된 익스텐션(`ext-<id>`)만 본다. 다른 키(gui, cli 등)는 데몬이 알 수 없어 넘긴다
    async fn unmet_module_dependencies(&self, module_name: &str) -> Option<Vec<String>> {
        let module = self.module_loader.get_module(module_name).ok()?;
        if module.metadata.dependencies.is_empty() {
            return None;
        }
        let mut installed = std::collections::HashMap::from([("saba-core".to_string(), env!("CARGO_PKG_VERSION").to_string())]);
        if let Some(ext_mgr) = &self.extension_manager {
            let mgr = ext_mgr.read().await;
            let ready = mgr.installed_and_enabled_set();
            for ext in mgr.list().into_iter().filter(|e| ready.contains(&e.id)) {
                installed.insert(format!("ext-{}", ext.id), ext.version);
            }
        }
        let unmet: Vec<String> = module
            .metadata
            .check_dependencies(&installed)
            .into_iter()
            .filter(|(key, _, _)| key == "saba-core" || key.starts_with("ext-"))
            .map(|(key, req, got)| match got {
                Some(v) => format!("{} {} (installed {})", key, req, v),
                None => format!("{} {} (not installed)", key, req),
            })
            .collect();
        (!unmet.is_empty()).then_some(unmet)
    }

    /// Start a server by name (e.g., "my-server-1")
    /// Called by IPC API: POST /api/server/:name/start
    ///
    /// managed 시작과 같이, 익스텐션 hook과 `get_launch_command`는 데몬 락 밖에서 부른다 (C12).
    pub async fn start_server(sup: &tokio::sync::RwLock<Self>, server_name: &str, module_name: &str, config: Value) -> Result<Value> {
        tracing::info!("Starting server '{}' with module '{}'", server_name, module_name);

        let moved = {
            let mut s = sup.write().await;
            let id = s.instance_store.get(server_name).or_else(|| s.instance_store.list().iter().find(|i| i.name == server_name)).map(|i| i.id.clone());
            id.and_then(|id| s.move_reserved_default_rest_port(&id))
        };
        let result = Self::start_server_inner(sup, server_name, module_name, config).await;
        Self::announce_ready(sup, server_name, &result).await;
        result.map(|result| with_port_moved(result, moved))
    }

    /// 시작 결과로 준비 알림(R39, `ready.rs`)과 접속·퇴장 알림(MK8, `player_events.rs`)을 시작한다. 콘솔을 읽는 managed 프로세스면 준비 문구를 기다린다
    async fn announce_ready(sup: &tokio::sync::RwLock<Self>, key: &str, result: &Result<Value>) {
        let (id, name, console, patterns, timeout, join, leave) = {
            let s = sup.read().await;
            let Some(instance) = s.instance_store.get(key).or_else(|| s.instance_store.list().iter().find(|i| i.name == key)) else { return };
            let module = s.module_loader.get_module(&instance.module_name).ok();
            let meta = module.as_ref().map(|m| &m.metadata);
            let patterns = meta.map(|m| m.ready_patterns.clone()).unwrap_or_default();
            let timeout = meta.and_then(|m| m.ready_timeout).unwrap_or(ready::DEFAULT_READY_TIMEOUT_SECS);
            let (join, leave) = meta.map(|m| (m.player_join_patterns.clone(), m.player_leave_patterns.clone())).unwrap_or_default();
            let console = s.managed_store.get(&instance.id).await.filter(|p| !p.log_follower);
            (instance.id.clone(), instance.name.clone(), console, patterns, Duration::from_secs(timeout), join, leave)
        };
        // 접속·퇴장 알림(MK8)도 같은 콘솔을 본다
        player_events::after_start(&id, &name, result, console.clone(), &join, &leave);
        ready::after_start(&id, &name, result, console, &patterns, timeout);
    }

    async fn start_server_inner(sup: &tokio::sync::RwLock<Self>, server_name: &str, module_name: &str, config: Value) -> Result<Value> {

        // ── 검사 (읽기 락) ──
        let (instance, starting, hook) = {
            let s = sup.read().await;
            // Find instance to get executable_path and working_dir
            let instance = s.instance_store.get(server_name)
                .or_else(|| s.instance_store.list().iter().find(|i| i.name == server_name))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

            // ── 중복 시작 방어: 추적 중인 PID가 살아 있으면 새로 띄우지 않는다 (추적을 덮어쓰지 않게) ──
            if let Ok(existing_pid) = s.tracker.get_pid(&instance.id) {
                if process::is_process_alive(existing_pid) {
                    return Ok(json!({
                        "success": true,
                        "server": instance.name,
                        "pid": existing_pid,
                        "already_running": true,
                        "message": format!("Server '{}' is already running with PID {}", instance.name, existing_pid)
                    }));
                }
            }

            // ── 포트 충돌 검사 (모듈별 프로토콜 인지) ──
            if let Some(blocked) = s.port_conflict_response(&instance) {
                return Ok(blocked);
            }

            // ── 모듈 의존성 버전 검사 ([module.dependencies]의 saba-core, ext-<id>) ──
            if let Some(blocked) = s.unmet_dependencies_response(module_name, &instance.name).await {
                return Ok(blocked);
            }

            // ── 인스턴스 필수 익스텐션 검증 ──
            // 기존 인스턴스의 required_extensions가 비어있으면 모듈의 install.requires_extensions에서 보충
            let effective_required: Vec<String> = if instance.required_extensions.is_empty() {
                if let Ok(loaded_module) = s.module_loader.get_module(module_name) {
                    let mut reqs = Vec::new();
                    if let Some(ref install) = loaded_module.metadata.install {
                        for ext_id in &install.requires_extensions {
                            reqs.push(ext_id.clone());
                        }
                    }
                    reqs
                } else {
                    Vec::new()
                }
            } else {
                instance.required_extensions.clone()
            };

            if !effective_required.is_empty() {
                if let Some(ref ext_mgr) = s.extension_manager {
                    let ext_mgr_guard = ext_mgr.read().await;
                    let ready = ext_mgr_guard.installed_and_enabled_set();
                    let enabled_not_installed = ext_mgr_guard.enabled_but_not_installed();
                    drop(ext_mgr_guard);
                    // missing: 활성화되지 않았거나 디스크에 설치되지 않은 익스텐션
                    let missing: Vec<String> = effective_required.iter()
                        .filter(|ext_id| !ready.contains(ext_id.as_str()))
                        .cloned()
                        .collect();
                    if !missing.is_empty() {
                        // 활성화되었지만 설치되지 않은 항목 구분
                        let not_installed: Vec<&String> = missing.iter()
                            .filter(|id| enabled_not_installed.contains(id))
                            .collect();
                        if !not_installed.is_empty() {
                            tracing::error!(
                                "Instance '{}' requires extensions {:?} which are enabled but not installed on disk",
                                server_name, not_installed
                            );
                        } else {
                            tracing::error!(
                                "Instance '{}' requires extensions {:?} but they are not enabled",
                                server_name, missing
                            );
                        }
                        return Ok(json!({
                            "success": false,
                            "error": "extension_required",
                            "error_code": "extension_required",
                            "instance_name": server_name,
                            "missing_extensions": missing,
                            "message": format!(
                                "Instance '{}' requires extension(s) {} to start. Please enable them first.",
                                server_name,
                                missing.join(", ")
                            )
                        }));
                    }
                }
            }


            let Some(starting) = StartGuard::claim(&s.starting, &instance.id, start_baseline(&s, &instance.id)) else {
                return Ok(json!({
                    "success": false,
                    "error": "start_in_progress",
                    "error_code": "start_in_progress",
                    "message": format!("Server '{}' is already starting", server_name),
                }));
            };

            // ── Extension hook: server.pre_start ──
            let hook = s.extension_manager.clone().map(|ext_mgr| {
                let instance_dir = s.instance_store.instance_dir(&instance.id);
                let ctx = json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                });
                (ext_mgr, ctx)
            });
            (instance, starting, hook)
        };
        // 받은 키가 id여도 메시지·응답에는 이름을 쓴다
        let server_name = instance.name.as_str();

        if let Some((ext_mgr, ctx)) = hook {
            let host_context = sup.read().await.host_context(&instance.id);
            let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.pre_start", ctx)).await;
            for (ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        // Extension handled but failed → ensure error/message fields exist
                        if val.get("success").and_then(|s| s.as_bool()) != Some(true) {
                            let mut fail_val = val.clone();
                            if let Some(obj) = fail_val.as_object_mut() {
                                obj.entry("error".to_string())
                                    .or_insert_with(|| json!("Extension failed to start server"));
                                obj.entry("message".to_string())
                                    .or_insert_with(|| json!("Extension handled the start request but reported failure"));
                            }
                            let err_detail = fail_val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                            tracing::warn!(
                                ext = ext_id,
                                error = err_detail,
                                "Extension hook 'server.pre_start' handled but failed for '{}'",
                                server_name
                            );
                            return Ok(fail_val);
                        }
                        // 익스텐션이 띄우는 사이 정지가 들어왔으면 그 정지(server.stop)는 이미 끝났다: 다시 꺼서 되돌린다
                        if starting.stop_requested(&*sup.read().await) {
                            Self::undo_extension_start(sup, &instance.id, module_name).await;
                            return Ok(start_cancelled(&instance.name));
                        }
                        // Extension이 시작 성공 → log_follower가 있으면 로그 스트리머 등록
                        if val.get("success").and_then(|s| s.as_bool()) == Some(true) {
                            if let Some(log_cfg) = val.get("log_follower") {
                                let instance_dir = sup.read().await.instance_store.instance_dir(&instance.id);
                                let program = log_cfg.get("program").and_then(|p| p.as_str()).unwrap_or_default();
                                let args: Vec<String> = log_cfg.get("args")
                                    .and_then(|a| a.as_array())
                                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                    .unwrap_or_default();
                                let work_dir = log_cfg.get("working_dir")
                                    .and_then(|w| w.as_str())
                                    .map(std::path::PathBuf::from)
                                    .unwrap_or_else(|| instance_dir.clone());
                                let desc = log_cfg.get("description")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("Extension log follower");
                                let interactive = log_cfg.get("interactive")
                                    .and_then(|i| i.as_bool())
                                    .unwrap_or(false);
                                let strip_prefix = log_cfg.get("strip_prefix").and_then(|s| s.as_str());
                                let log_pattern = sup.read().await.module_loader.get_module(module_name)
                                    .ok()
                                    .and_then(|m| m.metadata.log_pattern.clone());

                                // interactive=true → spawn() (bidirectional stdin/stdout)
                                // interactive=false → spawn_log_follower() (read-only stdout)
                                let mut env_vars: Vec<(String, String)> = log_cfg.get("env_vars")
                                    .and_then(|e| e.as_object())
                                    .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                                    .unwrap_or_default();
                                // 확인되지 않은 익스텐션이 띄우는 프로그램도 모듈과 같은 검사를 받는다
                                let roots = crate::sandbox::LaunchRoots {
                                    dirs: std::iter::once(instance_dir.clone()).chain(instance.working_dir.iter().map(std::path::PathBuf::from)).collect(),
                                    runtimes: Vec::new(),
                                };
                                let (program, gate) = match crate::sandbox::guard_launch_for(|| crate::sandbox::extension_trust(ext_id), program, &args, &work_dir, roots, &mut env_vars).await {
                                    Ok(launch) => launch,
                                    Err(blocked) => {
                                        tracing::warn!("Extension '{}' launch blocked: {:?}", ext_id, blocked);
                                        return Ok(blocked.to_json(crate::sandbox::message(&blocked)));
                                    }
                                };
                                let program = program.as_str();
                                let spawn_result = if interactive {
                                    tracing::info!("Spawning interactive IO bridge for '{}'", server_name);
                                    managed_process::ManagedProcess::spawn(
                                        program,
                                        &args,
                                        work_dir.to_str().unwrap_or("."),
                                        env_vars,
                                        log_pattern.as_deref(),
                                        Some(&instance_dir),
                                    ).await
                                } else {
                                    managed_process::ManagedProcess::spawn_log_follower(
                                        program,
                                        &args,
                                        &work_dir,
                                        desc,
                                        log_pattern.as_deref(),
                                        strip_prefix,
                                        Some(&instance_dir),
                                    ).await
                                };
                                drop(gate);
                                match spawn_result {
                                    Ok(mut follower) => {
                                        // 익스텐션이 붙인 다리라 서버 종료 판정(A1) 대상이 아니다
                                        follower.log_follower = true;
                                        sup.read().await.managed_store.insert(&instance.id, follower).await;
                                        tracing::info!(
                                            "{} registered for '{}'",
                                            if interactive { "Interactive IO bridge" } else { "Log follower" },
                                            server_name
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to start {} for '{}': {}",
                                            if interactive { "IO bridge" } else { "log follower" },
                                            server_name, e
                                        );
                                    }
                                }
                            }
                        }
                        return Ok(val.clone());
                    }
                }
            }
        }

        let call = {
            let s = sup.read().await;
            // ── Non-managed 모드: get_launch_command로 명령어를 받아 데몬이 직접 spawn ──

            tracing::info!("Starting non-managed server for instance '{}' (module: {})", server_name, module_name);

            // Build config — managed start와 동일한 병합 로직
            let mut cfg = config.as_object().cloned().unwrap_or_default();
            s.insert_resolved_exe(&mut cfg, &instance, module_name);
            if let Some(port) = instance.port {
                cfg.insert("port".to_string(), json!(port));
            }
            s.ensure_rcon_access(&instance)?;
            Self::insert_instance_fields(&mut cfg, &instance, &s.instance_store.instance_dir(&instance.id));
            // Non-managed → managed=false로 명시 (모듈이 실행 방식을 구분)
            cfg.insert("managed".to_string(), json!(false));
            let final_config = Value::Object(cfg);

            let module = s.module_loader.get_module(module_name)?;
            crate::plugin::PluginCall::new(module.entry_path(), "get_launch_command", final_config).in_instance(s.host_context(&instance.id))
        };
        let launch_result = call.run().await?;

        // action_required (e.g. server jar not found) → pass through to caller
        if launch_result.get("action_required").is_some() {
            tracing::warn!("Module requires user action: {:?}", launch_result.get("action_required"));
            return Ok(launch_result);
        }

        if launch_result.get("success").and_then(|s| s.as_bool()) != Some(true) {
            return Err(module_failure(&launch_result, module_name, "get_launch_command"));
        }

        let program = launch_result.get("program")
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow::anyhow!("Module did not return program"))?;

        let args = launch_args(&launch_result, &instance);

        let working_dir = launch_result.get("working_dir")
            .and_then(|w| w.as_str())
            .unwrap_or(".");

        let mut env_vars: Vec<(String, String)> = launch_result.get("env_vars")
            .and_then(|e| e.as_object())
            .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();

        let (program, gate) = match Self::guard_launch(sup, &instance, module_name, program, &args, working_dir, &mut env_vars).await {
            Ok(launch) => launch,
            Err(blocked) => return Ok(blocked),
        };
        let program = program.as_str();
        if starting.stop_requested(&*sup.read().await) {
            return Ok(start_cancelled(server_name));
        }

        // 데몬이 입출력을 잡지 않는다. Windows는 서버를 새 콘솔 창에 띄워 사용자가 그 창에서 보고 입력한다
        // (`.exe`가 아니면 예전처럼 창 없이 띄운다, `utils::new_console_program`)
        #[cfg(windows)]
        let console = crate::utils::new_console_program(program, std::path::Path::new(working_dir), &env_vars);
        #[cfg(not(windows))]
        let console: Option<std::path::PathBuf> = None;
        // 새 콘솔로 띄운 서버는 tokio Child가 없다. 정지 대비로 프로세스 핸들을 들고 있는다
        #[cfg(windows)]
        let mut console_handle: Option<std::os::windows::io::OwnedHandle> = None;
        let (pid, mut child) = match console {
            #[cfg(windows)]
            Some(exe) => {
                let spawned = crate::utils::spawn_in_new_console(&exe, &args, Some(std::path::Path::new(working_dir)), &env_vars);
                drop(gate);
                let (pid, handle) = spawned.map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", program, e))?;
                console_handle = Some(handle);
                (pid, None)
            }
            _ => {
                let mut cmd = TokioCommand::new(program);
                cmd.args(&args)
                    .current_dir(working_dir)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());

                for (key, value) in &env_vars {
                    cmd.env(key, value);
                }

                crate::utils::apply_creation_flags(&mut cmd);

                let child = cmd.spawn();
                drop(gate);
                let child = child.map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", program, e))?;
                let pid = child.id().ok_or_else(|| anyhow::anyhow!("Failed to get PID of spawned process"))?;
                (pid, Some(child))
            }
        };

        // 다른 경로(포트 충돌 검사·상태·정지)와 같은 키(인스턴스 id)로 추적한다.
        // managed 시작과 같이 정지 표시를 지우고 세대를 올린다 (기다리던 재시작이 이 시작 뒤에 또 켜지 않게)
        {
            let mut s = sup.write().await;
            // 띄우는 사이 정지가 들어왔으면 추적하지 않고 끈다: 정지는 이미 "꺼져 있음"으로 끝났다
            if starting.stop_requested(&s) {
                drop(s);
                // 자식까지 끝낸다: 런처형 서버는 런처만 끝내면 실제 서버가 남는다
                let killed = tokio::task::spawn_blocking(move || process::force_kill_tree(pid)).await;
                if let Err(e) = killed.map_err(anyhow::Error::from).and_then(|r| r) {
                    tracing::warn!("Could not stop '{}' after a stop request during start: {}", server_name, e);
                    if let Some(child) = child.as_mut() {
                        let _ = child.kill().await;
                    }
                    #[cfg(windows)]
                    if let Some(handle) = console_handle.as_ref() {
                        use std::os::windows::io::AsRawHandle;
                        unsafe { winapi::um::processthreadsapi::TerminateProcess(handle.as_raw_handle() as _, 1) };
                    }
                }
                if let Some(child) = child.as_mut() {
                    let _ = child.wait().await;
                }
                return Ok(start_cancelled(server_name));
            }
            s.stop_requested.remove(&instance.id);
            *s.run_generation.entry(instance.id.clone()).or_default() += 1;
            s.tracker.track(&instance.id, pid)?;
        }
        tracing::info!("Non-managed server '{}' started with PID {}", server_name, pid);

        Ok(json!({
            "success": true,
            "server": server_name,
            "pid": pid,
            "message": format!("Server '{}' started with PID {} (non-managed mode)", server_name, pid)
        }))
    }

    /// 익스텐션이 맡은 시작이 끝났을 때 이미 정지가 들어와 있었다: 그 정지의 `server.stop`은 서버가 뜨기 전에 돌았으므로
    /// 정지를 한 번 더 불러 익스텐션이 띄운 서버를 끈다
    async fn undo_extension_start(sup: &tokio::sync::RwLock<Self>, instance_id: &str, module_name: &str) {
        match Self::stop_server(sup, instance_id, module_name, false).await {
            Ok(v) if v.get("success").and_then(Value::as_bool) != Some(false) => {}
            other => tracing::warn!("Could not stop '{}' after a stop request during an extension start: {:?}", instance_id, other),
        }
    }

    /// 서버 정지. `server_name`은 인스턴스 id (이름은 겹칠 수 있다); 예전 호출을 위해 이름도 받는다
    ///
    /// 데몬 락은 준비와 정리 때만 잡는다. 익스텐션 hook, 모듈 `stop()`, 종료 대기(최대
    /// `stop_timeout`초)는 락 밖에서 한다 (C12).
    pub async fn stop_server(sup: &tokio::sync::RwLock<Self>, server_name: &str, module_name: &str, force: bool) -> Result<Value> {
        let result = Self::stop_server_inner(sup, server_name, module_name, force).await;
        // 정지가 실패하면 정지 표시를 지운다: 남겨 두면 뒤에 오는 진짜 크래시를 "사용자가 멈춤"으로 판정한다
        let failed = match &result {
            Err(_) => true,
            Ok(v) => v.get("success").and_then(Value::as_bool) == Some(false),
        };
        if failed {
            let mut s = sup.write().await;
            let id = s.instance_store.get(server_name)
                .or_else(|| s.instance_store.list().iter().find(|i| i.name == server_name))
                .map(|i| i.id.clone());
            if let Some(id) = id {
                s.stop_requested.remove(&id);
            }
        }
        result
    }

    async fn stop_server_inner(sup: &tokio::sync::RwLock<Self>, server_name: &str, module_name: &str, force: bool) -> Result<Value> {
        tracing::info!("Stopping server '{}' (force: {})", server_name, force);

        // ── 1) 준비 (쓰기 락: 정지 표시를 남긴다) ──
        let (instance, hook, managed) = {
            let mut s = sup.write().await;
            let instance = s.instance_store.get(server_name)
                .or_else(|| s.instance_store.list().iter().find(|i| i.name == server_name))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;
            s.stop_requested.insert(instance.id.clone());
            let generation = s.run_generation.entry(instance.id.clone()).or_default();
            *generation += 1;
            let _ = STOPPED_AT.try_with(|at| at.set(Some(*generation)));
            let hook = s.extension_manager.clone().map(|ext_mgr| {
                let instance_dir = s.instance_store.instance_dir(&instance.id);
                let ctx = json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                });
                (ext_mgr, ctx)
            });
            let managed = s.managed_store.get(&instance.id).await;
            (instance, hook, managed)
        };
        // 받은 키가 id여도 메시지·응답에는 이름을 쓴다
        let server_name = instance.name.as_str();

        // ── Extension hook: server.stop ──
        // 정지 전에 불린다. 익스텐션(예: 컨테이너)이 정지를 맡으면 handled=true를 돌려준다.
        if let Some((ext_mgr, ctx)) = hook {
            let host_context = sup.read().await.host_context(&instance.id);
            let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.stop", ctx)).await;
            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        sup.write().await.stop_cooldowns.insert(instance.id.clone(), Instant::now());
                        return Ok(val.clone());
                    }
                }
            }
            // 정지를 맡을 수 있는 익스텐션이 실패했으면 "안 돌고 있음"으로 덮지 않고 실패를 돌려준다
            let failed = results.iter().find_map(|(_, result)| match result {
                Err(e) => Some(crate::plugin::error_response(e)),
                Ok(val) if val.get("success").and_then(Value::as_bool) == Some(false) => Some(val.clone()),
                Ok(_) => None,
            });
            if let Some(mut error) = failed {
                error["server"] = json!(server_name);
                return Ok(error);
            }
        }

        // ── Native 모드 ──
        // 모듈 정보와 모듈 stop()에 넘길 값은 읽기 락 안에서 만든다
        let (module, stop_config, tracked_pid, host_context) = {
            let s = sup.read().await;
            let module = s.module_loader.get_module(module_name)?;
            let mut config_obj = serde_json::Map::new();
            s.insert_resolved_exe(&mut config_obj, &instance, module_name);
            Self::insert_instance_fields(&mut config_obj, &instance, &s.instance_store.instance_dir(&instance.id));
            // 모듈이 기다릴 시간은 데몬이 기다리는 시간과 같다 (인스턴스 값, 모듈 값, 기본 30초 순)
            config_obj.insert("stop_timeout".to_string(), json!(stop_timeout_of(&instance, module.metadata.stop_timeout)));
            // 추적표는 id로만 찾는다 (다른 인스턴스 id와 이름이 같아도 섞이지 않게)
            let tracked_pid = s.tracker.get_pid(&instance.id).ok();
            (module, config_obj, tracked_pid, s.host_context(&instance.id))
        };
        let module_path = module.entry_path();

        // ── Managed mode: stdin에 stop_command 전송으로 graceful shutdown ──
        if let Some(managed) = managed {
            if managed.is_running() {
                // 정지 방법
                // - force: 즉시 PID 강제 종료
                // - 모듈이 stdin을 받고 stdin이 열려 있음: stop_command를 stdin으로
                // - 그 밖(REST 전용 게임, stdin 없는 재부착): 모듈 lifecycle.py `stop()`에
                //   PID와 managed=true를 넘겨 맡긴다. 시간 안에 안 끝나면 아래에서 강제 종료.
                let module_has_stdin = module.metadata.protocols_supported.as_ref()
                    .is_some_and(|p| p.iter().any(|s| s == "stdin"));
                let via_stdin = module_has_stdin && managed.is_stdin_available();
                // 모듈에서 stop_command를 가져옴 (없으면 "stop" 기본값)
                let stop_cmd = module.metadata.stop_command.clone().unwrap_or_else(|| "stop".to_string());

                if force {
                    tracing::info!("Force-killing managed server '{}'", server_name);
                    if let Err(e) = managed.force_kill() {
                        tracing::warn!("Force kill of '{}' failed: {}", server_name, e);
                    }
                } else if via_stdin {
                    // 준비 문구 전에 받은 명령을 실패하는 서버가 있다 (마인크래프트 26.3은 오류만 찍고 계속 돈다).
                    // 켜지는 중이면 준비될 때까지(정지 대기 시간 안에서) 들고 있다가 보낸다
                    if ready::is_waiting(&instance.id) {
                        tracing::info!("'{}' is still starting; holding '{}' until it is ready", server_name, stop_cmd);
                        let hold = Duration::from_secs(stop_timeout_of(&instance, module.metadata.stop_timeout));
                        let until = tokio::time::Instant::now() + hold;
                        while ready::is_waiting(&instance.id) && managed.is_running() && tokio::time::Instant::now() < until {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    }
                    tracing::info!("Sending '{}' to managed server '{}' via stdin", stop_cmd, server_name);
                    if let Err(e) = managed.send_command(&stop_cmd).await {
                        tracing::warn!("Failed to send stop command via stdin: {}", e);
                    }
                } else {
                    tracing::info!("Asking module '{}' to stop managed server '{}'", module_name, server_name);
                    let mut config_obj = stop_config;
                    config_obj.insert("pid".to_string(), json!(managed.pid));
                    config_obj.insert("managed".to_string(), json!(true));
                    config_obj.insert("force".to_string(), json!(false));
                    config_obj.insert("stop_call_limit".to_string(), json!(crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS));
                    if let Some(port) = instance.rest_port {
                        config_obj.insert("rest_port".to_string(), json!(port));
                    }
                    match crate::plugin::PluginCall::new(module_path.clone(), "stop", Value::Object(config_obj)).serving(Some(managed.pid)).in_instance(host_context.clone()).run().await {
                        // 저장 후 종료를 요청할 길이 없다: 기다렸다 강제 종료하지 않고 사용자가 고르게 돌려준다 (MK10)
                        Ok(result) if is_graceful_stop_unavailable(&result) => return Ok(graceful_stop_unavailable(server_name, &result)),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("Module stop for '{}' failed: {} (will force kill on timeout)", server_name, e),
                    }
                }

                // 프로세스 종료 대기 (force kill 시 5초, graceful 시 인스턴스 설정 `stop_timeout`, 모듈 값, 기본 30초 순)
                let timeout_secs = if force { 5 } else { stop_timeout_of(&instance, module.metadata.stop_timeout) };
                let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
                loop {
                    if !managed.is_running() {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        tracing::warn!("Managed server '{}' did not exit in {}s, force killing", server_name, timeout_secs);
                        if let Err(e) = managed.force_kill() {
                            tracing::warn!("Force kill of '{}' failed: {}", server_name, e);
                        }
                        // 끝난 것을 확인하기 전에는 추적을 놓지 않는다 (종료가 거부됐으면 아직 돌고 있다)
                        if !process::wait_gone(Duration::from_secs(5), || managed.is_running()).await {
                            return Err(anyhow::anyhow!("Server '{}' is still running: force kill did not take effect", server_name));
                        }
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }

                let mut s = sup.write().await;
                // managed store에서 제거 (그새 모니터가 치웠으면 그쪽이 이미 종료를 기록했다)
                if s.managed_store.get(&instance.id).await.is_some_and(|p| Arc::ptr_eq(&p, &managed)) {
                    s.managed_store.remove(&instance.id).await;
                    // 종료 기록 (user_stop)과 정지 표시 정리
                    s.judge_exit(&instance.id, &managed, false).await;
                    if force {
                        if let Some(last) = s.exit_events.get_mut(&instance.id).and_then(|e| e.back_mut()) {
                            last.verdict.evidence = "force stop requested through saba-chan".into();
                        }
                        s.save_exit_events(&instance.id);
                    }
                }

                // Clean up managed.pid
                let pid_file = s.instance_store.instance_dir(&instance.id).join("managed.pid");
                let _ = std::fs::remove_file(&pid_file);

                tracing::info!("Managed server '{}' stopped", server_name);
                // auto-detect 쿨다운 등록
                s.stop_cooldowns.insert(instance.id.clone(), Instant::now());
                return Ok(json!({
                    "success": true,
                    "server": server_name,
                    "message": format!("Server '{}' stopped", server_name)
                }));
            } else {
                // 프로세스가 이미 종료됨 (유저가 콘솔에서 직접 stop 명령어 입력 등)
                // managed store 정리 후 정상 종료로 간주
                tracing::info!(
                    "Managed server '{}' already exited (user-initiated stop command), cleaning up",
                    server_name
                );
                let mut s = sup.write().await;
                // 이미 스스로 끝났다: 정지 요청 때문이 아니므로 표시를 지우고 기록한다
                s.stop_requested.remove(&instance.id);
                // 크래시였어도 다시 켜지 않는다: 사용자가 정지를 눌렀다 (D-S6)
                if let Some(proc) = s.managed_store.remove(&instance.id).await {
                    s.judge_exit(&instance.id, &proc, false).await;
                }
                let pid_file = s.instance_store.instance_dir(&instance.id).join("managed.pid");
                let _ = std::fs::remove_file(&pid_file);
                s.stop_cooldowns.insert(instance.id.clone(), Instant::now());

                // tracker에서도 제거
                let _ = s.tracker.untrack(&instance.id);

                return Ok(json!({
                    "success": true,
                    "server": server_name,
                    "message": format!("Server '{}' already stopped", server_name)
                }));
            }
        }

        // ── Non-managed mode: Python lifecycle.py를 통한 정지 ──
        // 데몬이 추적하는 PID가 없으면 멈출 것이 없다. 모듈 stop()을 PID 없이 부르면
        // 이미지 이름으로 같은 게임의 다른 서버까지 끝낼 수 있다
        let Some(pid) = tracked_pid else {
            sup.write().await.stop_requested.remove(&instance.id);
            return Ok(json!({
                "success": true,
                "server": server_name,
                "was_running": false,
                "message": format!("Server '{}' is not running", server_name)
            }));
        };
        let mut config_obj = stop_config;
        // Pass PID from tracker so the module can kill the actual process
        config_obj.insert("pid".to_string(), json!(pid));
        config_obj.insert("force".to_string(), json!(force));
        // managed 서버는 위에서 데몬이 직접 멈춘다. 여기까지 오면 항상 non-managed.
        config_obj.insert("managed".to_string(), json!(false));
        // 모듈이 `stop_timeout`만큼 기다리므로 호출은 그보다 조금 더 기다린다. 그 한도를 모듈에도 알린다
        let limit = module_stop_limit(stop_timeout_of(&instance, module.metadata.stop_timeout));
        config_obj.insert("stop_call_limit".to_string(), json!(limit));
        let config = Value::Object(config_obj);

        // Execute stop function
        let result = crate::plugin::PluginCall::new(module_path.clone(), "stop", config).serving(Some(pid)).in_instance(host_context).timeout(limit).run().await?;

        // Check if the module actually succeeded
        let plugin_success = result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
        let plugin_message = result.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        if !force && is_graceful_stop_unavailable(&result) {
            return Ok(graceful_stop_unavailable(server_name, &result));
        }
        if plugin_success {
            tracing::info!("Server '{}' stopped successfully: {}", server_name, plugin_message);
            // auto-detect 쿨다운 등록
            sup.write().await.stop_cooldowns.insert(instance.id.clone(), Instant::now());
            Ok(json!({
                "success": true,
                "server": server_name,
                "message": format!("Server '{}' stopped", server_name)
            }))
        } else {
            tracing::error!("Failed to stop server '{}': {}", server_name, plugin_message);
            Err(module_failure(&result, module_name, "stop"))
        }
    }

    /// Get server status by name
    /// Called by IPC API: GET /api/server/:name/status
    pub async fn get_server_status(sup: &tokio::sync::RwLock<Self>, server_name: &str, module_name: &str) -> Result<Value> {
        tracing::debug!("Getting status for server '{}'", server_name);

        // 락 안에서는 값만 챙긴다. 익스텐션 hook과 모듈 status()는 락 밖에서 (C12)
        let (instance, hook) = {
            let s = sup.read().await;
            let instance = s.instance_store.list()
                .iter()
                .find(|i| i.name == server_name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;
            let hook = s.extension_manager.clone().map(|ext_mgr| {
                let instance_dir = s.instance_store.instance_dir(&instance.id);
                let process_patterns = s.module_loader.get_module(module_name)
                    .map(|m| m.metadata.process_patterns.clone())
                    .unwrap_or_default();
                let ctx = json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                    "process_patterns": process_patterns,
                });
                (ext_mgr, ctx)
            });
            (instance, hook)
        };

        // ── Extension hook: server.status ──
        if let Some((ext_mgr, ctx)) = hook {
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

        let call = {
            let s = sup.read().await;
            // Managed 프로세스가 있으면 plugin 호출 없이 직접 판단
            if let Some(managed) = s.managed_store.get(&instance.id).await {
                let running = managed.is_running();
                let pid = s.tracker.get_pid(&instance.id).ok();
                let start_time = if running { s.tracker.get_start_time(&instance.id).ok() } else { None };
                return Ok(json!({
                    "server": server_name,
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
            crate::plugin::PluginCall::new(module_path, "status", Value::Object(config_obj)).serving(tracked_pid).in_instance(s.host_context(&instance.id))
        };

        // Ask module for status
        match call.run().await {
            Ok(result) => {
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
                    "server": server_name,
                    "status": status,
                    "pid": pid,
                    "start_time": start_time,
                }))
            },
            Err(e) => {
                tracing::error!("Failed to get status: {}", e);
                let mut result = crate::plugin::error_response(&e);
                result["server"] = json!(server_name);
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

            // 모듈의 기본값 가져오기 (하드코딩 대신 모듈 설정 사용)
            let default_rest_port = module.metadata.default_rest_port();
            let default_rest_host = module.metadata.default_rest_host();

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
                "rest_host": instance.rest_host.clone().unwrap_or(default_rest_host),
                "rest_port": instance.rest_port.unwrap_or(default_rest_port),
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

    /// Start a server as a managed process with full stdio capture.
    /// Uses the module's `get_launch_command` to build the command, then spawns it natively.
    /// Extension 관리 모드에서는 extension hook을 통해 위임합니다.
    ///
    /// 데몬 락은 검사·준비·등록 때만 짧게 잡는다. 익스텐션 hook, `get_launch_command`,
    /// 프로세스 띄우기는 락 밖에서 한다 (C12). 같은 인스턴스를 동시에 두 번 시작하지 않도록
    /// 시작하는 동안 `starting` 표시를 쥔다.
    pub async fn start_managed_server(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        module_name: &str,
        config: Value,
    ) -> Result<Value> {
        let moved = sup.write().await.move_reserved_default_rest_port(instance_id);
        let result = Self::start_managed_server_inner(sup, instance_id, module_name, config).await;
        Self::announce_ready(sup, instance_id, &result).await;
        result.map(|result| with_port_moved(result, moved))
    }

    async fn start_managed_server_inner(
        sup: &tokio::sync::RwLock<Self>,
        instance_id: &str,
        module_name: &str,
        config: Value,
    ) -> Result<Value> {

        // ── 1) 검사 (읽기 락) ──
        let (mut instance, starting, hook) = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

            // ── 중복 프로세스 방어: 이미 실행 중인 managed 프로세스가 있으면 새로 생성하지 않음 ──
            if let Some(existing) = s.managed_store.get(instance_id).await {
                if existing.is_running() {
                    tracing::info!(
                        "Managed process for '{}' already running (PID {}), skipping duplicate start",
                        instance.name, existing.pid
                    );
                    return Ok(json!({
                        "success": true,
                        "server": instance.name,
                        "pid": existing.pid,
                        "managed": true,
                        "already_running": true,
                        "message": format!("Server '{}' is already running with PID {} (managed mode)", instance.name, existing.pid)
                    }));
                }
            }
            // managed_store에 없더라도 tracker에 PID가 있고 프로세스가 살아있으면 중복 방지
            if let Ok(existing_pid) = s.tracker.get_pid(instance_id) {
                if crate::supervisor::process::is_process_alive(existing_pid) {
                    tracing::warn!(
                        "Tracked process for '{}' still alive (PID {}) but not in managed store — preventing duplicate start",
                        instance.name, existing_pid
                    );
                    return Ok(json!({
                        "success": true,
                        "server": instance.name,
                        "pid": existing_pid,
                        "managed": true,
                        "already_running": true,
                        "message": format!("Server '{}' has a tracked process (PID {}) still alive", instance.name, existing_pid)
                    }));
                } else {
                    // 프로세스가 죽어있으면 tracker 정리
                    tracing::info!("Cleaning stale tracker entry for '{}' (PID {} no longer alive)", instance.name, existing_pid);
                    let _ = s.tracker.untrack(instance_id);
                }
            }

            if let Some(blocked) = s.unmet_dependencies_response(module_name, &instance.name).await {
                return Ok(blocked);
            }

            // ── 포트 충돌 검사 (모듈별 프로토콜 인지) ──
            if let Some(blocked) = s.port_conflict_response(&instance) {
                return Ok(blocked);
            }


            let Some(starting) = StartGuard::claim(&s.starting, instance_id, start_baseline(&s, instance_id)) else {
                return Ok(json!({
                    "success": false,
                    "error": "start_in_progress",
                    "error_code": "start_in_progress",
                    "message": format!("Server '{}' is already starting", instance.name),
                }));
            };

            // ── Extension hook: server.pre_start (managed) ──
            let hook = s.extension_manager.clone().map(|ext_mgr| {
                let instance_dir = s.instance_store.instance_dir(&instance.id);
                // 인스턴스와 시작 설정의 비밀 값은 post_create·settings_changed와 같은 기준으로 가린다 (D-E2)
                let module = s.module_loader.get_module(module_name).ok();
                let hidden = |v: &Value| crate::audit::redact(module.as_ref().map(|m| &m.metadata), None, None, Some(v)).1.unwrap_or_default();
                let ctx = json!({
                    "instance_id": instance_id,
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "instance": hidden(&serde_json::to_value(&instance).unwrap_or_default()),
                    "config": hidden(&config),
                    "managed": true,
                    "extension_data": &instance.extension_data,
                });
                (ext_mgr, ctx)
            });
            (instance, starting, hook)
        };

        // ── 2) 익스텐션 hook (락 밖) ──
        if let Some((ext_mgr, ctx)) = hook {
            let host_context = sup.read().await.host_context(&instance.id);
            let results = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.pre_start", ctx)).await;
            for (ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        // Extension handled but failed → ensure error/message fields exist
                        if val.get("success").and_then(|s| s.as_bool()) != Some(true) {
                            let mut fail_val = val.clone();
                            if let Some(obj) = fail_val.as_object_mut() {
                                obj.entry("error".to_string())
                                    .or_insert_with(|| json!("Extension failed to start server"));
                                obj.entry("message".to_string())
                                    .or_insert_with(|| json!("Extension handled the start request but reported failure"));
                            }
                            let err_detail = fail_val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                            tracing::warn!(
                                ext = ext_id,
                                error = err_detail,
                                "Extension hook 'server.pre_start' (managed) handled but failed for '{}'",
                                instance.name
                            );
                            return Ok(fail_val);
                        }
                        // 익스텐션이 띄우는 사이 정지가 들어왔으면 그 정지(server.stop)는 이미 끝났다: 다시 꺼서 되돌린다
                        if starting.stop_requested(&*sup.read().await) {
                            Self::undo_extension_start(sup, &instance.id, module_name).await;
                            return Ok(start_cancelled(&instance.name));
                        }
                        // Extension이 시작 성공 → log_follower가 있으면 로그 스트리머 등록
                        if val.get("success").and_then(|s| s.as_bool()) == Some(true) {
                            if let Some(log_cfg) = val.get("log_follower") {
                                let inst_dir = sup.read().await.instance_store.instance_dir(&instance.id);
                                let program = log_cfg.get("program").and_then(|p| p.as_str()).unwrap_or_default();
                                let args: Vec<String> = log_cfg.get("args")
                                    .and_then(|a| a.as_array())
                                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                    .unwrap_or_default();
                                let work_dir = log_cfg.get("working_dir")
                                    .and_then(|w| w.as_str())
                                    .map(std::path::PathBuf::from)
                                    .unwrap_or_else(|| inst_dir.clone());
                                let desc = log_cfg.get("description")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("Extension log follower");
                                let interactive = log_cfg.get("interactive")
                                    .and_then(|i| i.as_bool())
                                    .unwrap_or(false);
                                let strip_prefix = log_cfg.get("strip_prefix").and_then(|s| s.as_str());
                                let log_pattern = sup.read().await.module_loader.get_module(module_name)
                                    .ok()
                                    .and_then(|m| m.metadata.log_pattern.clone());

                                // interactive=true → spawn() (bidirectional stdin/stdout)
                                // interactive=false → spawn_log_follower() (read-only stdout)
                                let mut env_vars: Vec<(String, String)> = log_cfg.get("env_vars")
                                    .and_then(|e| e.as_object())
                                    .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                                    .unwrap_or_default();
                                // 확인되지 않은 익스텐션이 띄우는 프로그램도 모듈과 같은 검사를 받는다
                                let roots = crate::sandbox::LaunchRoots {
                                    dirs: std::iter::once(inst_dir.clone()).chain(instance.working_dir.iter().map(std::path::PathBuf::from)).collect(),
                                    runtimes: Vec::new(),
                                };
                                let (program, gate) = match crate::sandbox::guard_launch_for(|| crate::sandbox::extension_trust(ext_id), program, &args, &work_dir, roots, &mut env_vars).await {
                                    Ok(launch) => launch,
                                    Err(blocked) => {
                                        tracing::warn!("Extension '{}' launch blocked: {:?}", ext_id, blocked);
                                        return Ok(blocked.to_json(crate::sandbox::message(&blocked)));
                                    }
                                };
                                let program = program.as_str();
                                let spawn_result = if interactive {
                                    tracing::info!("Spawning interactive IO bridge for '{}'", instance.name);
                                    managed_process::ManagedProcess::spawn(
                                        program,
                                        &args,
                                        work_dir.to_str().unwrap_or("."),
                                        env_vars,
                                        log_pattern.as_deref(),
                                        Some(&inst_dir),
                                    ).await
                                } else {
                                    managed_process::ManagedProcess::spawn_log_follower(
                                        program,
                                        &args,
                                        &work_dir,
                                        desc,
                                        log_pattern.as_deref(),
                                        strip_prefix,
                                        Some(&inst_dir),
                                    ).await
                                };
                                drop(gate);
                                match spawn_result {
                                    Ok(mut follower) => {
                                        // 익스텐션이 붙인 다리라 서버 종료 판정(A1) 대상이 아니다
                                        follower.log_follower = true;
                                        sup.read().await.managed_store.insert(instance_id, follower).await;
                                        tracing::info!(
                                            "{} registered for '{}'",
                                            if interactive { "Interactive IO bridge" } else { "Log follower" },
                                            instance.name
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to start {} for '{}': {}",
                                            if interactive { "IO bridge" } else { "log follower" },
                                            instance.name, e
                                        );
                                    }
                                }
                            }
                        }
                        return Ok(val.clone());
                    }
                }
            }
        }

        // ── 3) 실행 명령 받기: 준비는 읽기 락, 모듈 호출은 락 밖 ──
        let (call, module) = {
            let s = sup.read().await;
            tracing::info!("Starting managed server for instance '{}' (module: {})", instance.name, module_name);

            // Build config for the module — 실행 파일은 모듈 정의에서 자동 해석
            let mut cfg = config.as_object().cloned().unwrap_or_default();
            s.insert_resolved_exe(&mut cfg, &instance, module_name);
            if let Some(port) = instance.port {
                cfg.insert("port".to_string(), json!(port));
            }
            s.ensure_rcon_access(&instance)?;
            Self::insert_instance_fields(&mut cfg, &instance, &s.instance_store.instance_dir(&instance.id));
            // Managed mode indicator → 모듈이 실행 방식을 구분
            cfg.insert("managed".to_string(), json!(true));
            let final_config = Value::Object(cfg);

            // Get module and call get_launch_command
            let module = s.module_loader.get_module(module_name)?;
            // 비밀번호류 설정값과 REST 비밀번호는 가려서 남긴다
            let (_, logged) = crate::audit::redact(Some(&module.metadata), Some(&instance), None, Some(&final_config));
            tracing::info!("get_launch_command config: {}", logged.unwrap_or_default());
            (crate::plugin::PluginCall::new(module.entry_path(), "get_launch_command", final_config).in_instance(s.host_context(&instance.id)), module)
        };
        let launch_result = call.run().await?;

        // If the module requires user action (e.g. server jar not found), pass through to GUI
        if launch_result.get("action_required").is_some() {
            tracing::warn!("Module requires user action: {:?}", launch_result.get("action_required"));
            return Ok(launch_result);
        }

        if launch_result.get("success").and_then(|s| s.as_bool()) != Some(true) {
            return Err(module_failure(&launch_result, module_name, "get_launch_command"));
        }

        let program = launch_result.get("program")
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow::anyhow!("Module did not return program"))?;

        let args = launch_args(&launch_result, &instance);

        let working_dir = launch_result.get("working_dir")
            .and_then(|w| w.as_str())
            .unwrap_or(".");

        let mut env_vars: Vec<(String, String)> = launch_result.get("env_vars")
            .and_then(|e| e.as_object())
            .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();

        let (program, gate) = match Self::guard_launch(sup, &instance, module_name, program, &args, working_dir, &mut env_vars).await {
            Ok(launch) => launch,
            Err(blocked) => return Ok(blocked),
        };
        let program = program.as_str();

        // Sync REST credentials from module via credential_map or rest_credentials
        {
            let mut creds_changed = false;

            // 1) 명시적 rest_credentials (모듈이 직접 반환한 경우)
            if let Some(rest_creds) = launch_result.get("rest_credentials").and_then(|v| v.as_object()) {
                if let Some(ru) = rest_creds.get("rest_username").and_then(|v| v.as_str()) {
                    if instance.rest_username.as_deref() != Some(ru) {
                        instance.rest_username = Some(ru.to_string());
                        creds_changed = true;
                        tracing::info!("Synced rest_username='{}' from module for instance '{}'", ru, instance.id);
                    }
                }
                if let Some(rp) = rest_creds.get("rest_password").and_then(|v| v.as_str()) {
                    if instance.rest_password.as_deref() != Some(rp) {
                        instance.rest_password = Some(rp.to_string());
                        creds_changed = true;
                        tracing::info!("Synced rest_password from module for instance '{}'", instance.id);
                    }
                }
            }
            // 2) credential_map 기반 자동 동기화 (module.toml에 선언된 매핑)
            //    get_launch_command가 module_settings를 반환하면 그 값을 읽어서 매핑
            else if !module.metadata.credential_map.is_empty() {
                for (daemon_key, game_key) in &module.metadata.credential_map {
                    // launch_result의 module_settings 또는 최상위에서 게임 키 값을 찾는다
                    let game_value = launch_result.get("module_settings")
                        .and_then(|ms| ms.get(game_key))
                        .and_then(|v| v.as_str())
                        .or_else(|| launch_result.get(game_key).and_then(|v| v.as_str()));

                    if let Some(val) = game_value {
                        match daemon_key.as_str() {
                            "rest_password" => {
                                if instance.rest_password.as_deref() != Some(val) {
                                    instance.rest_password = Some(val.to_string());
                                    creds_changed = true;
                                    tracing::info!("credential_map: synced {} -> {} for '{}'", game_key, daemon_key, instance.id);
                                }
                            }
                            "rest_username" => {
                                if instance.rest_username.as_deref() != Some(val) {
                                    instance.rest_username = Some(val.to_string());
                                    creds_changed = true;
                                    tracing::info!("credential_map: synced {} -> {} for '{}'", game_key, daemon_key, instance.id);
                                }
                            }
                            _ => {
                                tracing::debug!("credential_map: unknown daemon key '{}', skipping", daemon_key);
                            }
                        }
                    }
                }
            }

            // 시작하는 동안 다른 설정이 바뀌었을 수 있으니 지금 값에 자격 증명만 덮는다
            if creds_changed {
                let mut s = sup.write().await;
                if let Some(mut current) = s.instance_store.get(&instance.id).cloned() {
                    current.rest_username = instance.rest_username.clone();
                    current.rest_password = instance.rest_password.clone();
                    if let Err(e) = s.instance_store.update(&instance.id, current) {
                        tracing::warn!("Failed to persist synced credentials: {}", e);
                    }
                }
            }
        }

        if starting.stop_requested(&*sup.read().await) {
            return Ok(start_cancelled(&instance.name));
        }

        // Spawn managed process
        let log_pattern = module.metadata.log_pattern.as_deref();
        let inst_dir = sup.read().await.instance_store.instance_dir(&instance.id);
        // saba-runner가 서버를 쥐면 데몬이 재시작해도 콘솔·stdin·종료 코드가 이어진다 (C8b)
        let managed = match managed_process::runner_binary() {
            Some(runner) => {
                let keep = instance.module_settings.get("log_sessions_keep")
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .map(|n| n.max(1) as usize)
                    .unwrap_or(managed_process::DEFAULT_LOG_SESSIONS);
                let spawned = ManagedProcess::spawn_with_runner(
                    &runner, program, &args, working_dir, env_vars, log_pattern, &inst_dir, keep,
                    managed_process::RunnerLaunch {
                        log_files: &module.metadata.log_files,
                        console: module.metadata.console,
                        memory_limit_mb: instance.memory_limit_mb,
                    },
                ).await;
                // 시작 중 상한에 걸린 것도 종료 이력에 시작 실패로 남긴다
                if let Some(hit) = spawned.as_ref().err().and_then(|e| e.downcast_ref::<managed_process::MemoryLimitAtStart>()) {
                    let verdict = crash::ExitVerdict {
                        kind: crash::ExitKind::StartFailed,
                        exit_code: hit.exit_code,
                        signal: None,
                        evidence: format!("memory limit {} MiB reached while starting", hit.limit_mb),
                    };
                    let event = crash::ExitEvent { at: saba_runner::now(), verdict, restart_in_secs: None, note: None, note_code: None, note_args: None };
                    sup.write().await.record_exit(&instance.id, &instance.name, event, None);
                }
                spawned?
            }
            None => {
                tracing::warn!("saba-runner not found next to the daemon; '{}' is attached directly and will lose its console if the daemon restarts", instance.name);
                if let Some(mb) = instance.memory_limit_mb {
                    tracing::warn!("'{}': memory limit {} MiB needs saba-runner and is not enforced", instance.name, mb);
                }
                ManagedProcess::spawn(program, &args, working_dir, env_vars, log_pattern, Some(&inst_dir)).await?
            }
        };
        drop(gate);
        let pid = managed.pid;

        // Track the process
        {
            let mut s = sup.write().await;
            // 띄우는 사이 정지가 들어왔으면 추적하지 않고 끈다: 정지는 이미 "꺼져 있음"으로 끝났다
            if starting.stop_requested(&s) {
                drop(s);
                if let Err(e) = managed.force_kill() {
                    tracing::warn!("Could not stop '{}' after a stop request during start: {}", instance.name, e);
                }
                return Ok(start_cancelled(&instance.name));
            }
            s.stop_requested.remove(instance_id);
            *s.run_generation.entry(instance.id.clone()).or_default() += 1;
            s.tracker.track(&instance.id, pid)?;
            s.managed_store.insert(&instance.id, managed).await;
        }

        // Persist PID to disk for orphan reattachment on daemon restart
        let pid_file = inst_dir.join("managed.pid");
        if let Err(e) = saba_atomic_file::write(&pid_file, pid.to_string().as_bytes(), saba_atomic_file::Mode::Plain) {
            tracing::warn!("Failed to write managed.pid for '{}': {}", instance.name, e);
        }

        tracing::info!("Managed server '{}' started with PID {}", instance.name, pid);
        Ok(json!({
            "success": true,
            "server": instance.name,
            "pid": pid,
            "managed": true,
            "message": format!("Server '{}' started with PID {} (managed mode)", instance.name, pid)
        }))
    }

    /// 이 인스턴스를 managed(데몬이 자식 프로세스로 stdin/stdout을 쥐고)로 시작하는가.
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
    /// 서버가 켜져 있는가: 추적표(non-managed와 managed 모두 띄운 뒤 올린다)나 managed 저장소에 살아 있으면.
    /// "켜져 있으면 거절" 판정은 모두 이것을 쓴다 (재부착 때 추적표 등록이 실패해 managed에만 남은 경우도 켜진 것이다)
    pub async fn is_running(&self, instance_id: &str) -> bool {
        self.tracker.get_pid(instance_id).is_ok() || self.managed_store.running_instance_ids().await.iter().any(|id| id == instance_id)
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

    /// RCON만 받는 모듈의 인스턴스면 접속 정보를 만들어 둔다 (시작 전). 그 밖의 모듈은 아무것도 하지 않는다
    /// 모듈이 RCON을 더 쓰지 않으면(업데이트로 stdin·rest가 생김) 남은 접속 정보를 지운다.
    /// 준비하지 못하면 시작을 멈춘다: 비밀번호 없이 띄우면 명령을 보낼 길이 없다
    pub(crate) fn ensure_rcon_access(&self, instance: &crate::instance::ServerInstance) -> Result<()> {
        let dir = self.instance_store.instance_dir(&instance.id);
        let Some(port) = self.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.rcon_only_port()) else {
            let _ = std::fs::remove_file(dir.join(crate::protocol::rcon::ACCESS_FILE));
            return Ok(());
        };
        // 두 인스턴스가 동시에 처음 켜져도 같은 포트를 고르지 않게, 고르고 저장하는 동안 하나씩만
        static PICKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _picking = PICKING.lock().unwrap_or_else(|e| e.into_inner());
        // 다른 인스턴스의 RCON·게임·REST 포트는 고르지 않는다
        let taken: Vec<u16> = self
            .instance_store
            .list()
            .iter()
            .filter(|other| other.id != instance.id)
            .flat_map(|other| {
                let rcon = crate::protocol::rcon::load(&self.instance_store.instance_dir(&other.id)).map(|a| a.port);
                [rcon, other.port, other.rest_port]
            })
            .flatten()
            .chain(instance.port)
            .chain(instance.rest_port)
            .collect();
        crate::protocol::rcon::ensure(&dir, port, &taken)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("Could not prepare RCON access for '{}': {}", instance.name, e))
    }

    /// 명령을 RCON으로 보낼 인스턴스면 접속 정보 (모듈이 RCON만 받고, 시작할 때 접속 정보를 만들어 둔 경우)
    pub fn rcon_route(&self, instance_id: &str) -> Option<crate::protocol::rcon::Access> {
        let instance = self.instance_store.get(instance_id)?;
        self.module_loader.get_module(&instance.module_name).ok()?.metadata.rcon_only_port()?;
        crate::protocol::rcon::load(&self.instance_store.instance_dir(instance_id))
    }

    /// 인스턴스의 모듈이 콘솔 입력(`protocols.supported`에 "stdin")을 받는가
    pub fn module_accepts_stdin(&self, instance_id: &str) -> bool {
        let Some(instance) = self.instance_store.get(instance_id) else { return false };
        self.module_loader
            .get_module(&instance.module_name)
            .ok()
            .and_then(|m| m.metadata.protocols_supported)
            .is_some_and(|p| p.iter().any(|s| s == "stdin"))
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

    /// Run validation on an instance (via module's validate function)
    pub async fn validate_instance(sup: &tokio::sync::RwLock<Self>, instance_id: &str) -> Result<Value> {
        let call = sup.read().await.instance_call(instance_id, "validate")?;
        let result = call.run().await?;
        Ok(result)
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
                "reset_server" => "reset_server",
                // 실제 설정 파일을 saba-chan 키로 (설정 화면이 캐시 대신 실제 값을 보여 준다)
                "import" => "import_settings",
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
                .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;
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
                    .filter(|l| l.timestamp >= proc.started_at)
                    .map(|l| l.content.clone())
                    .collect();
                cfg.insert("log_lines".to_string(), json!(log_lines));
            }

            crate::plugin::PluginCall::new(module_path, "diagnose_log", Value::Object(cfg)).in_instance(s.host_context(instance_id))
        };
        let result = call.run().await?;
        Ok(result)
    }

    // ─── Server Installation Methods ─────────────────────────

    /// List available server versions (delegates to module lifecycle)
    pub async fn list_versions(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        include_snapshots: bool,
        page: u32,
        per_page: u32,
    ) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            let config = json!({
                "include_snapshots": include_snapshots,
                "page": page,
                "per_page": per_page,
            });

            crate::plugin::PluginCall::new(module_path, "list_versions", config)
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// Get detailed info for a specific version
    pub async fn get_version_details(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        version: &str,
    ) -> Result<Value> {
        let call = {
            let s = sup.read().await;
            let module = s.module_loader.get_module(module_name)?;
            let module_path = module.entry_path();

            let config = json!({ "version": version });
            crate::plugin::PluginCall::new(module_path, "get_version_details", config)
        };
        let result = call.run().await?;
        Ok(result)
    }

    /// 모듈 `list_versions`로 그 설정(서버 종류 등)에서 받을 수 있는 최신 버전 (`latest.release`, 예전 모양 `latest_release`).
    /// 찾지 못하면 그 까닭(모듈 실행 오류, 모듈이 준 메시지)
    pub async fn latest_version(module_path: &str, settings: &Value) -> Result<String> {
        match crate::plugin::run_plugin(module_path, "list_versions", json!({ "settings": settings })).await {
            Ok(v) if v.get("success").and_then(Value::as_bool) == Some(false) => Err(module_failure(&v, v.get("module").and_then(Value::as_str).unwrap_or(""), "list_versions")),
            Ok(v) => v
                .pointer("/latest/release")
                .or_else(|| v.get("latest_release"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("{}", v.get("message").and_then(Value::as_str).unwrap_or("list_versions gave no latest release"))),
            Err(e) => {
                tracing::warn!("Failed to fetch versions from '{}': {}", module_path, e);
                Err(e)
            }
        }
    }

    /// Install a server: download binary, setup directory, optional initial settings.
    /// 기존 실행 파일이 존재하면 `.bak`으로 백업하고, 실패 시 복원합니다.
    /// `instance`: 그 인스턴스의 설정과 격리 실행이 쓸 데이터 폴더 (처음 설치와 같은 값을 받는다)
    /// `keep_executable`: 그 인스턴스가 실제로 띄우는 파일 (모듈 실행 파일과 다르면 함께 백업한다)
    /// `commit`: 설치가 성공하면 백업을 지우기 전에 부른다 (인스턴스 기록 저장). 실패하면 백업으로 되돌리고 그 오류를 돌려준다
    #[allow(clippy::too_many_arguments)]
    pub async fn install_server(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        version: &str,
        install_dir: &str,
        executable: Option<&str>,
        accepted_agreements: &[String],
        initial_settings: Option<Value>,
        instance: Option<(Value, Vec<String>, crate::module_paths::Boundary, crate::plugin::host::Context)>,
        keep_executable: Option<&str>,
        commit: Option<&InstallCommit<'_>>,
    ) -> Result<Value> {
        let module = sup.read().await.module_loader.get_module(module_name)?;
        let module_path = module.entry_path();

        // 서버 실행 파일 이름: 명시 > 모듈 metadata.server_executable > 기본값 없음
        let effective_jar = executable
            .map(|s| s.to_string())
            .or_else(|| {
                module.metadata.server_executable.clone()
            });

        // ── 기존 바이너리 백업: 모듈 실행 파일과 인스턴스가 실제로 띄우는 파일 (모듈이 어느 쪽을 바꾸다 실패해도 되돌린다) ──
        // 설치 폴더 밖의 파일(밖을 가리키는 링크 포함)은 이 설치가 바꾸는 대상이 아니므로 건드리지 않는다
        let inside = |path: &std::path::Path| match (std::fs::canonicalize(path), std::fs::canonicalize(install_dir)) {
            (Ok(path), Ok(dir)) => path.starts_with(dir),
            _ => false,
        };
        let mut originals: Vec<std::path::PathBuf> = effective_jar.iter().map(|jar| std::path::Path::new(install_dir).join(jar)).collect();
        if let Some(exe) = keep_executable {
            let exe = std::path::Path::new(install_dir).join(exe);
            if !originals.iter().any(|o| same_file_path(o, &exe)) {
                originals.push(exe);
            }
        }
        // (원래 경로, 백업, 심볼릭 링크면 그 대상): 백업은 비어 있는 새 이름에만 만든다 (사용자의 기존 .bak를 덮지 않는다)
        let mut backups: Vec<(std::path::PathBuf, std::path::PathBuf, Option<std::path::PathBuf>)> = Vec::new();
        for original in originals.into_iter().filter(|original| original.is_file() && inside(original)) {
            let link_target = match std::fs::symlink_metadata(&original) {
                Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(&original).ok(),
                _ => None,
            };
            match backup_beside(&original) {
                Ok(backup) => {
                    tracing::info!("Backed up existing binary: {} → {}", original.display(), backup.display());
                    backups.push((original, backup, link_target));
                }
                Err(e) => {
                    // 되돌릴 수 없는 채로 모듈을 부르지 않는다
                    tracing::warn!("Failed to backup '{}': {}", original.display(), e);
                    for (_, made, _) in &backups {
                        let _ = std::fs::remove_file(made);
                    }
                    return Ok(json!({
                        "success": false,
                        "error_code": "backup_failed",
                        "file": original.file_name().unwrap_or_default().to_string_lossy(),
                        "message": format!("Could not back up {} before installing: {}", original.file_name().unwrap_or_default().to_string_lossy(), e),
                    }));
                }
            }
        }
        // 되돌리지 못한 파일은 백업을 남기고 (파일 이름, 남긴 백업 이름)을 돌려준다 (rename은 같은 폴더라 공간이 필요 없다).
        // 심볼릭 링크가 그대로 있으면 링크를 두고 백업 내용을 링크 대상에 쓴다. 모듈이 링크를 바꿨으면 그 자리에 되돌린다
        let restore = |why: &str| -> Vec<(String, String)> {
            let mut failed = Vec::new();
            for (original, backup, link_target) in &backups {
                let still_linked = |target: &std::path::PathBuf| {
                    std::fs::symlink_metadata(original).is_ok_and(|m| m.file_type().is_symlink())
                        && std::fs::canonicalize(original).is_ok_and(|now| now == *target)
                };
                let put_back = match link_target.as_ref().filter(|target| still_linked(target)) {
                    Some(target) => std::fs::copy(backup, target).map(|_| ()),
                    None => std::fs::rename(backup, original).or_else(|_| std::fs::copy(backup, original).map(|_| ())),
                };
                match put_back {
                    Ok(()) => {
                        tracing::info!("Restored backup after {}: {}", why, original.display());
                        let _ = std::fs::remove_file(backup);
                    }
                    Err(e) => {
                        tracing::error!("Failed to restore backup '{}': {}", backup.display(), e);
                        let name = |p: &std::path::Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        failed.push((name(original), name(backup)));
                    }
                }
            }
            failed
        };

        let mut config = json!({
            "version": version,
            "install_dir": install_dir,
            "accepted_agreements": accepted_agreements,
        });

        if let Some(ref name) = effective_jar {
            config["server_executable"] = json!(name);
        }
        if let Some(settings) = initial_settings {
            config["initial_settings"] = settings;
        }
        let (path_boundary, host) = if let Some((settings, data_dirs, boundary, host)) = instance {
            config["settings"] = settings;
            config[crate::sandbox::INSTANCE_DATA_DIRS] = json!(data_dirs);
            (crate::module_paths::Boundary::for_install(std::path::Path::new(install_dir), Some(boundary)), Some(host))
        } else {
            (crate::module_paths::Boundary::for_install(std::path::Path::new(install_dir), None), None)
        };

        // 서버 파일과 Java를 받는다: 기본 120초로는 느린 회선에서 중간에 끊긴다 (처음 설치와 같은 한도)
        let result = crate::plugin::PluginCall::new(module_path, "install_server", config)
            .in_instance(host).timeout(crate::plugin::INSTALL_TIMEOUT_SECS).run().await
            .and_then(|mut value| {
                if value.get("success").and_then(Value::as_bool) == Some(true) {
                    path_boundary.returned_paths(&mut value, module_name, "install_server")?;
                }
                Ok(value)
            });

        match result {
            Ok(ref val) => {
                if val.get("success").and_then(|s| s.as_bool()) == Some(true) {
                    // 기록을 저장하지 못하면 파일도 설치 전으로 되돌린다 (옛 기록과 새 실행 파일이 섞이지 않게)
                    if let Some(commit) = commit {
                        let saved = commit(&mut *sup.write().await, val);
                        if let Err(e) = saved {
                            let (files, kept) = restore("unsaved install").into_iter().unzip();
                            return Err(e.context(CommitFailed(PutBackFailed { files, kept })));
                        }
                    }
                    // 설치 성공 → 백업 파일 정리
                    for (_, backup, _) in &backups {
                        let _ = std::fs::remove_file(backup);
                        tracing::info!("Removed backup after successful install: {}", backup.display());
                    }
                    result
                } else {
                    // 모듈이 success=false 반환 → 복원
                    let failed = restore("failed install");
                    let mut val = val.clone();
                    if let (false, Some(obj)) = (failed.is_empty(), val.as_object_mut()) {
                        let (files, kept): (Vec<String>, Vec<String>) = failed.into_iter().unzip();
                        obj.insert("restore_failed".into(), json!(files));
                        obj.insert("kept_backups".into(), json!(kept));
                    }
                    Ok(val)
                }
            }
            Err(e) => {
                // 플러그인 에러 → 복원
                let failed = restore("plugin error");
                if failed.is_empty() {
                    Err(e)
                } else {
                    let (files, kept) = failed.into_iter().unzip();
                    Err(e.context(PutBackFailed { files, kept }))
                }
            }
        }
    }

    /// Detect the installed server version by inspecting the server binary (e.g. parsing server.jar).
    /// Delegates to the module's `get_installed_version` lifecycle function.
    pub async fn get_installed_version(
        sup: &tokio::sync::RwLock<Self>,
        module_name: &str,
        instance_id: &str,
    ) -> Result<Value> {
        // 시작·점검과 같은 값(`server_executable`·`working_dir` 등)을 받는다
        let call = {
            let s = sup.read().await;
            let instance = s.instance_store.get(instance_id)
                .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", instance_id))?;
            if instance.module_name != module_name {
                anyhow::bail!("Instance '{}' is not a '{}' server", instance_id, module_name);
            }
            s.instance_call(instance_id, "get_installed_version")?
        };
        call.run().await
    }

    /// 감지한 서버 버전을 저장한다. 감지를 시작할 때 본 값(`seen`)에서 그사이 바뀌었으면 더 새 값이므로 두고 false (D-S12)
    pub fn save_detected_version(&mut self, instance_id: &str, seen: Option<&str>, version: &str) -> Result<bool> {
        let Some(mut instance) = self.instance_store.get(instance_id).cloned() else { return Ok(false) };
        if instance.server_version.as_deref() != seen || seen == Some(version) {
            return Ok(false);
        }
        instance.server_version = Some(version.to_string());
        self.instance_store.update(instance_id, instance)?;
        Ok(true)
    }

    /// 인스턴스마다 사용자가 정한 `java_path`: `(모듈 진입 파일, java_path)` (`sandbox::reconcile_runtimes`)
    pub fn user_runtimes(&self) -> Vec<(String, String)> {
        self.instance_store
            .list()
            .iter()
            .filter(|inst| inst.user_settings.contains("java_path"))
            .filter_map(|inst| {
                let java = inst.module_settings.get("java_path")?.as_str()?.to_string();
                Some((self.module_loader.get_module(&inst.module_name).ok()?.entry_path(), java))
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

    /// server_version이 비어있고 working_dir이 설정된 인스턴스에 대해
    /// 모듈의 get_installed_version을 호출하여 버전을 감지·저장합니다.
    /// 데몬 초기화 뒤 한 번 호출하여 기존 서버의 버전을 보충합니다.
    pub async fn backfill_server_versions(sup: std::sync::Arc<tokio::sync::RwLock<Self>>) {
        // 감지 대상 수집 (borrow 분리를 위해 먼저 복사)
        let targets: Vec<(String, String, String)> = sup.read().await.instance_store.list().iter()
            .filter(|i| i.server_version.is_none())
            .filter(|i| {
                // module_settings.working_dir 또는 instance.working_dir이 있어야 감지 가능
                let has_ms_wd = i.module_settings.get("working_dir")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_inst_wd = i.working_dir.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
                has_ms_wd || has_inst_wd
            })
            .map(|i| (i.id.clone(), i.module_name.clone(), i.name.clone()))
            .collect();

        if targets.is_empty() {
            return;
        }

        tracing::info!("Backfilling server versions for {} instance(s)...", targets.len());

        for (id, module_name, name) in targets {
            match Self::get_installed_version(&sup, &module_name, &id).await {
                Ok(result) => {
                    if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                        if let Some(version) = result.get("version").and_then(|v| v.as_str()) {
                            // 모으던 때 비어 있던 값만 채운다. 그사이 저장된 버전은 덮지 않는다
                            match sup.write().await.save_detected_version(&id, None, version) {
                                Ok(true) => tracing::info!("Backfilled server version '{}' for instance '{}'", version, name),
                                Ok(false) => {}
                                Err(e) => tracing::warn!("Failed to save backfilled version for '{}': {}", name, e),
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Version backfill skipped for '{}': {}", name, e);
                }
            }
        }
    }

    /// 백그라운드 프로세스 모니터링 (주기적 실행)
    ///
    /// `process_snapshot`은 write lock 획득 전에 미리 스캔한 프로세스 목록입니다.
    /// 이를 통해 비용이 큰 시스템 콜(sysinfo)이 write lock 밖에서 실행되어
    /// GUI read 요청에 대한 블로킹을 최소화합니다.
    /// 끝난 managed 프로세스가 왜 끝났는지 가르고, 크래시면 재시작을 예약한다 (A1). `may_restart`가 false면 예약하지 않는다
    async fn judge_exit(&mut self, instance_id: &str, proc: &ManagedProcess, may_restart: bool) {
        use managed_process::{LogLevel, LogSource};
        if proc.log_follower {
            return;
        }
        let stop_requested = self.stop_requested.remove(instance_id);
        let Some(instance) = self.instance_store.get(instance_id).cloned() else { return };
        // 준비되기 전에 끝났으면 그 시작의 준비 알림을 닫는다 (R39)
        ready::exited(instance_id, &instance.name, stop_requested);
        let (graceful, artifacts) = self
            .module_loader
            .get_module(&instance.module_name)
            .map(|m| (m.metadata.graceful_exit_patterns.clone(), m.metadata.crash_artifacts.clone()))
            .unwrap_or_default();
        let tail: Vec<String> = proc
            .get_recent_console(crash::CONSOLE_TAIL_LINES)
            .await
            .into_iter()
            .filter(|l| l.source != LogSource::System)
            .map(|l| l.content)
            .collect();
        let working_dir = proc.working_dir.clone().or_else(|| instance.working_dir.as_ref().map(std::path::PathBuf::from));
        let crash_files = match &working_dir {
            Some(dir) if !stop_requested => crash::new_crash_files(dir, &artifacts, proc.started_at),
            _ => Vec::new(),
        };
        let verdict = crash::classify(&crash::ExitFacts {
            stop_requested,
            exit: proc.exit(),
            console_tail: &tail,
            graceful_patterns: &graceful,
            new_crash_files: &crash_files,
            ran_secs: if proc.reattached { u64::MAX } else { saba_runner::now().saturating_sub(proc.started_at) },
        });

        let auto_restart = setting_flag(&instance, "auto_restart", true);
        let on_unknown = setting_flag(&instance, "restart_on_unknown_exit", false);
        let mut attempt = None;
        let (restart_in, note, note_code) = if may_restart && crash::wants_restart(verdict.kind, auto_restart, on_unknown) {
            let history = self.restart_history.entry(instance_id.to_string()).or_default();
            match crash::next_restart_delay(history, Instant::now()) {
                Some(delay) => {
                    let generation = self.run_generation.get(instance_id).copied().unwrap_or_default();
                    self.pending_restarts.push((instance_id.to_string(), delay, generation));
                    attempt = Some(generation);
                    (Some(delay.as_secs()), None, None)
                }
                None => (
                    None,
                    Some(format!("auto restart gave up after {} restarts in 10 minutes", crash::MAX_RESTARTS)),
                    Some(("gave_up", serde_json::json!({ "count": crash::MAX_RESTARTS }))),
                ),
            }
        } else if verdict.kind == crash::ExitKind::Crash && !auto_restart {
            (None, Some("auto_restart is off".to_string()), Some(("auto_restart_off", serde_json::json!({}))))
        } else {
            (None, None, None)
        };

        let line = match (verdict.kind, restart_in, &note) {
            (crash::ExitKind::Crash, Some(secs), _) => Some(format!("── Crash detected ({}). Restarting in {}s. ──", verdict.evidence, secs)),
            (crash::ExitKind::UnknownExit, Some(secs), _) => Some(format!("── Server ended unexpectedly ({}). Restarting in {}s. ──", verdict.evidence, secs)),
            (crash::ExitKind::Crash, None, Some(why)) => Some(format!("── Crash detected ({}). Not restarting: {}. ──", verdict.evidence, why)),
            (crash::ExitKind::UnknownExit, None, Some(why)) => Some(format!("── Server ended unexpectedly ({}). Not restarting: {}. ──", verdict.evidence, why)),
            _ => None,
        };
        if let Some(line) = line {
            tracing::warn!("'{}': {}", instance.name, line);
            proc.push_system_line(line, LogLevel::Warn).await;
        } else {
            tracing::info!("'{}' exited: {:?} ({})", instance.name, verdict.kind, verdict.evidence);
        }

        let (note_code, note_args) = note_code.map(|(c, a)| (c.to_string(), a)).unzip();
        let event = crash::ExitEvent { at: saba_runner::now(), verdict, restart_in_secs: restart_in, note, note_code, note_args };
        self.record_exit(instance_id, &instance.name, event, attempt);
    }

    /// 종료 판정 하나를 알리고(`instance.exit`) 이력에 남긴다.
    /// `attempt`는 다시 띄우기로 했을 때 그 복구 시도의 번호 (끝을 알리는 이벤트와 같은 `restartAttempt`)
    fn record_exit(&mut self, instance_id: &str, instance_name: &str, event: crash::ExitEvent, attempt: Option<u64>) {
        crate::events::emit("instance.exit", serde_json::json!({
            "instanceId": instance_id,
            "instance": instance_name,
            "kind": event.verdict.kind,
            "exitCode": event.verdict.exit_code,
            "signal": event.verdict.signal,
            "evidence": event.verdict.evidence,
            "restartInSecs": event.restart_in_secs,
            "restartAttempt": attempt,
            "note": event.note,
            "noteCode": event.note_code,
            "noteArgs": event.note_args,
        }));
        if !self.exit_events.contains_key(instance_id) {
            let saved = self.saved_exit_events(instance_id);
            self.exit_events.insert(instance_id.to_string(), saved.into());
        }
        let events = self.exit_events.entry(instance_id.to_string()).or_default();
        events.push_back(event);
        while events.len() > crash::EVENTS_KEPT {
            events.pop_front();
        }
        self.save_exit_events(instance_id);
    }

    fn exit_events_path(&self, instance_id: &str) -> std::path::PathBuf {
        self.instance_store.instance_dir(instance_id).join("exit-events.json")
    }

    /// 지난 데몬 실행에서 남긴 종료 이력
    fn saved_exit_events(&self, instance_id: &str) -> Vec<crash::ExitEvent> {
        std::fs::read_to_string(self.exit_events_path(instance_id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save_exit_events(&self, instance_id: &str) {
        let Some(events) = self.exit_events.get(instance_id) else { return };
        let path = self.exit_events_path(instance_id);
        if !path.parent().is_some_and(|d| d.is_dir()) {
            return;
        }
        match serde_json::to_vec(events) {
            Ok(bytes) => {
                if let Err(e) = saba_atomic_file::write(&path, &bytes, saba_atomic_file::Mode::Plain) {
                    tracing::warn!("Failed to save exit history for '{}': {}", instance_id, e);
                }
            }
            Err(e) => tracing::warn!("Failed to encode exit history for '{}': {}", instance_id, e),
        }
    }

    /// 모니터가 정한 재시작을 가져간다. 대기 뒤 `restart_still_wanted`로 다시 확인하고 시작한다.
    pub fn take_pending_restarts(&mut self) -> Vec<(String, Duration, u64)> {
        std::mem::take(&mut self.pending_restarts)
    }

    /// 대기하는 동안 사용자가 정지·시작했거나 인스턴스가 지워졌으면 false.
    /// `generation`은 예약 때의 실행 세대 (꺼진 서버에 정지를 보내면 정지 표시가 바로 지워지므로 세대로 가른다)
    pub async fn restart_still_wanted(&self, instance_id: &str, generation: u64) -> bool {
        if self.stop_requested.contains(instance_id)
            || self.instance_store.get(instance_id).is_none()
            || self.run_generation.get(instance_id).copied().unwrap_or_default() != generation
        {
            return false;
        }
        !matches!(self.managed_store.get(instance_id).await, Some(p) if p.is_running())
    }

    /// 지금 실행 세대 (시작할 때마다 1씩 오른다)
    pub fn run_generation_of(&self, instance_id: &str) -> u64 {
        self.run_generation.get(instance_id).copied().unwrap_or_default()
    }

    /// 자동 재시작이 실패했을 때 이력에 남긴다
    pub fn record_restart_failure(&mut self, instance_id: &str, attempt: u64, reason: String) {
        if let Some(last) = self.exit_events.get_mut(instance_id).and_then(|e| e.back_mut()) {
            last.note = Some(format!("restart failed: {}", reason));
            last.note_code = Some("restart_failed".into());
            last.note_args = Some(serde_json::json!({ "reason": reason }));
            // `instance.exit`는 이미 나갔으므로 실패는 따로 알린다 (오류 원문이 있어 릴레이로는 올리지 않는다)
            crate::events::emit("instance.restart_failed", serde_json::json!({
                "instanceId": instance_id,
                "instance": self.instance_store.get(instance_id).map(|i| i.name.clone()).unwrap_or_default(),
                "noteCode": last.note_code,
                "noteArgs": last.note_args,
                "restartAttempt": attempt,
            }));
        }
        self.save_exit_events(instance_id);
    }

    /// 자동 재시작 대기가 끝났을 때 알린다: 다시 켜졌으면 `instance.restarted`,
    /// 대기 중에 정지·시작·삭제돼 그만뒀으면 `instance.restart_cancelled` (실패는 `record_restart_failure`).
    /// `attempt`는 시작을 알린 `instance.exit`의 `restartAttempt`: 그 사이 새 크래시로 새 시도가 시작됐으면 번호가 다르다
    pub fn record_restart_outcome(&self, instance_id: &str, attempt: u64, restarted: bool) {
        let kind = if restarted { "instance.restarted" } else { "instance.restart_cancelled" };
        crate::events::emit(kind, serde_json::json!({
            "instanceId": instance_id,
            "instance": self.instance_store.get(instance_id).map(|i| i.name.clone()).unwrap_or_default(),
            "restartAttempt": attempt,
        }));
    }

    /// 최근 종료 이력, 오래된 것부터
    pub fn exit_events(&self, instance_id: &str) -> Vec<crash::ExitEvent> {
        match self.exit_events.get(instance_id) {
            Some(events) => events.iter().cloned().collect(),
            None => self.saved_exit_events(instance_id),
        }
    }

    /// 인스턴스 서버 파일이 있을 수 있는 폴더들 (작업 폴더, 실행 파일 폴더, 인스턴스 폴더)
    fn instance_roots(&self, instance: &crate::instance::ServerInstance) -> Vec<String> {
        let mut roots = Vec::new();
        if let Some(dir) = instance.working_dir.as_deref().filter(|d| !d.is_empty()) {
            roots.push(dir.to_string());
        }
        // module_settings의 실행 파일 경로까지 본 실제 실행 파일
        let exe = self.resolve_executable(instance, &instance.module_name);
        if let Some(parent) = exe.as_deref().and_then(|p| std::path::Path::new(p).parent()) {
            let parent = parent.to_string_lossy().to_string();
            if !parent.is_empty() {
                roots.push(parent);
            }
        }
        roots.push(self.instance_store.instance_dir(&instance.id).to_string_lossy().to_string());
        roots
    }

    /// `snapshot_taken_at`(unix 초): 프로세스 목록을 찍기 시작한 시각. 그 뒤에 추적한 PID는 목록에 없어도
    /// 끝났다고 보지 않는다 (목록을 락 밖에서 찍는 사이에 막 시작한 서버)
    pub async fn monitor_processes_taken_at(&mut self, process_snapshot: &[crate::process_monitor::RunningProcess], snapshot_taken_at: u64) -> Result<()> {
        use crate::process_monitor;

        // Clean up dead managed processes and their PID files
        let dead = self.managed_store.cleanup_dead().await;
        for (dead_id, proc) in &dead {
            self.judge_exit(dead_id, proc, true).await;
            let pid_file = self.instance_store.instance_dir(dead_id).join("managed.pid");
            let _ = std::fs::remove_file(&pid_file);
            // 죽은 managed process에 대해 stop cooldown 등록 → auto_detect 재탐색 억제
            // 이전 세션에서 실행되어 reattach된 프로세스가 종료된 경우,
            // cooldown 없이 auto_detect가 즉시 동명 프로세스를 재추적하는 것을 방지
            self.stop_cooldowns.insert(dead_id.clone(), Instant::now());
        }
        
        let instances = self.instance_store.list().to_vec();
        let all_instances_snapshot = instances.clone();
        let mut tracked_count = 0;
        let mut auto_detected_count = 0;
        
        for instance in instances {
            // 이미 추적 중이면 상태만 확인 (snapshot 기반 — 시스템 콜 없음)
            if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                tracked_count += 1;
                
                let tracked_after_snapshot = self.tracker.get_start_time(&instance.id).is_ok_and(|t| t >= snapshot_taken_at);
                if !tracked_after_snapshot && !process_monitor::is_running_in(process_snapshot, pid) {
                    // 러너가 아직 서버를 돌리고 있으면(런처가 끝나고 진짜 서버가 남은 경우) 러너가 보는 PID로 옮긴다
                    // 로그만 따라 읽는 익스텐션 follower는 서버 수명을 말해 주지 않는다: 러너일 때만
                    if matches!(self.managed_store.get(&instance.id).await, Some(p) if p.is_running() && p.runner_pid.is_some()) {
                        let dir = self.instance_store.instance_dir(&instance.id);
                        match managed_process::runner_server_pid(&dir)
                            .filter(|p| *p != pid && process_monitor::is_running_in(process_snapshot, *p))
                        {
                            Some(new_pid) => {
                                tracing::info!("Instance '{}': launcher {} exited, following server PID {}", instance.name, pid, new_pid);
                                let _ = self.tracker.replace_pid(&instance.id, new_pid);
                            }
                            None => tracing::debug!("Instance '{}': PID {} gone but saba-runner still runs it", instance.name, pid),
                        }
                        continue;
                    }
                    tracing::warn!("Process {} for instance '{}' is no longer running, removing from tracker", pid, instance.name);
                    // tracker에서 제거하여 다음 사이클에서 다시 감지할 수 있도록 함
                    if let Err(e) = self.tracker.untrack(&instance.id) {
                        tracing::error!("Failed to untrack process: {}", e);
                    }
                }
                continue;
            }
            
            // 프로비저닝 중인 인스턴스는 auto-detect 건너뛰기
            if let Some(ref pt) = self.provision_tracker {
                if let Some(progress) = pt.get(&instance.name) {
                    if !progress.done {
                        tracing::debug!(
                            "Skipping auto-detect for '{}' (provisioning in progress)",
                            instance.name
                        );
                        continue;
                    }
                }
            }

            // auto_detect가 활성화되어 있고 process_name이 설정되어 있으면 감지 시도
            if instance.auto_detect {
                // stop 쿨다운 중이면 건너뛰기
                if let Some(stopped_at) = self.stop_cooldowns.get(&instance.id) {
                    if stopped_at.elapsed().as_secs() < STOP_COOLDOWN_SECS {
                        tracing::debug!(
                            "Skipping auto-detect for '{}' (stop cooldown: {}s remaining)",
                            instance.name,
                            STOP_COOLDOWN_SECS - stopped_at.elapsed().as_secs()
                        );
                        continue;
                    } else {
                        // 쿨다운 만료 → 제거
                        self.stop_cooldowns.remove(&instance.id);
                    }
                }
                if let Some(process_name) = &instance.process_name {
                    // 모듈에서 cmd_patterns 가져오기 — java.exe 같은 범용 프로세스를 구분하기 위한 커맨드라인 패턴
                    let cmd_patterns = self.module_loader.get_module(&instance.module_name)
                        .map(|m| m.metadata.cmd_patterns.clone())
                        .unwrap_or_default();

                    // snapshot 기반 검색 — 시스템 콜 없음
                    let processes = if cmd_patterns.is_empty() {
                        process_monitor::find_by_name_in(process_snapshot, process_name)
                    } else {
                        process_monitor::find_by_name_and_cmd_in(process_snapshot, process_name, &cmd_patterns)
                    };

                    // 같은 게임 인스턴스가 여럿이면 폴더로 주인을 가리고, 다른 인스턴스가 잡은 PID는 빼고 고른다
                    let siblings: Vec<&crate::instance::ServerInstance> = all_instances_snapshot
                        .iter()
                        .filter(|i| i.module_name == instance.module_name && i.id != instance.id)
                        .collect();
                    let taken: std::collections::HashSet<u32> = all_instances_snapshot
                        .iter()
                        .filter(|i| i.id != instance.id)
                        .filter_map(|i| self.tracker.get_pid(&i.id).ok())
                        .collect();
                    let (own_roots, other_roots) = process_monitor::drop_shared_roots(
                        self.instance_roots(&instance),
                        siblings.iter().flat_map(|i| self.instance_roots(i)).collect(),
                    );
                    let picked = process_monitor::pick_owned_process(&processes, &taken, &own_roots, &other_roots);

                    if let Some(process) = picked {
                                auto_detected_count += 1;
                                tracing::info!(
                                    "Auto-detected process '{}' (PID: {}) for instance '{}'",
                                    process_name, process.pid, instance.name
                                );
                                if let Err(e) = self.tracker.track(&instance.id, process.pid) {
                                    tracing::error!("Failed to track process: {}", e);
                                }

                                // ── 포트 충돌 검사: 새로 감지된 프로세스의 포트가 이미 실행 중인 인스턴스와 겹치면 강제 종료 ──
                                // GUI에서 portConflictCheck를 비활성화하면 auto-detect 시에도 강제 종료를 건너뜀
                                if self.skip_port_check.load(Ordering::Relaxed) {
                                    tracing::debug!("Port conflict check skipped for auto-detected '{}' (disabled by GUI setting)", instance.name);
                                } else {
                                let all_instances = self.instance_store.list();
                                let running_ids: std::collections::HashSet<String> = all_instances
                                    .iter()
                                    .filter(|i| i.id != instance.id && self.tracker.get_pid(&i.id).is_ok())
                                    .map(|i| i.id.clone())
                                    .collect();
                                let module_protocols = self.build_module_protocols_map();
                                let conflicts = crate::validator::check_port_conflicts(&instance, all_instances, &running_ids, Some(&module_protocols));
                                if !conflicts.is_empty() {
                                    let details: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
                                    tracing::warn!(
                                        "Port conflict detected for auto-detected '{}' — force stopping (PID {}). Conflicts: {:?}",
                                        instance.name, process.pid, details
                                    );

                                    // 자식까지 강제 종료 (런처형 서버는 런처만 끝내면 실제 서버가 남는다. taskkill은 동기라 spawn_blocking)
                                    let kill_pid = process.pid;
                                    let kill_name = instance.name.clone();
                                    tokio::task::spawn_blocking(move || match process::force_kill_tree(kill_pid) {
                                        Ok(()) => tracing::info!("Force-killed PID {} for instance '{}'", kill_pid, kill_name),
                                        Err(e) => tracing::warn!("Could not force-kill PID {} for instance '{}': {}", kill_pid, kill_name, e),
                                    }).await.ok();

                                    // tracker에서 제거 + 쿨다운 설정
                                    let _ = self.tracker.untrack(&instance.id);
                                    self.stop_cooldowns.insert(instance.id.clone(), Instant::now());

                                    // 이벤트 기록 (GUI 폴링에서 읽어감) — 충돌마다 개별 이벤트 생성
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0);
                                    self.record_port_conflict_stops(conflicts.iter().map(|conflict| PortConflictStopEvent {
                                        id: 0,
                                        stopped_name: instance.name.clone(),
                                        stopped_id: instance.id.clone(),
                                        existing_name: conflict.conflicting_instance_name.clone(),
                                        existing_id: conflict.conflicting_instance_id.clone(),
                                        port: conflict.port,
                                        timestamp_ms: ts,
                                    }));
                                } // if !skip_port_check
                                }
                            }
                }
            }
        }
        
        tracing::debug!("Monitor cycle: {} tracked, {} auto-detected", tracked_count, auto_detected_count);
        Ok(())
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

    /// 매개변수 없이 읽는 옛 클라이언트용: 아직 이 방식으로 보이지 않은 알림만 주고 본 것으로 적는다.
    /// 알림은 지우지 않으므로 `notices_after`로 읽는 클라이언트는 그대로 본다
    pub fn drain_port_conflict_stops(&self) -> Vec<PortConflictStopEvent> {
        self.port_conflict_stops
            .lock()
            .map(|mut stops| {
                stops.forget_old();
                let seen = stops.legacy_seen;
                let unseen: Vec<_> = stops.events.iter().filter(|e| e.id > seen).cloned().collect();
                stops.legacy_seen = stops.last_id;
                unseen
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

/// non-managed 정지에서 모듈 `stop()` 호출의 시간 제한: 모듈이 기다리는 `stop_timeout`에 저장·정리 여유 30초
pub(crate) fn module_stop_limit(stop_timeout: u64) -> u64 {
    stop_timeout + 30
}

/// 모듈 `stop()`이 저장 후 종료를 요청할 길이 없다고 답했는가 (콘솔·REST가 없는 서버. 모듈은 이때 서버를 끝내지 않는다)
fn is_graceful_stop_unavailable(result: &Value) -> bool {
    result.get("success").and_then(Value::as_bool) != Some(true) && result.get("error_code").and_then(Value::as_str) == Some("graceful_stop_unavailable")
}

/// 정지하지 않았다는 응답. 강제 종료(저장 없음)는 사용자가 `force`로 다시 요청한다
fn graceful_stop_unavailable(server_name: &str, result: &Value) -> Value {
    tracing::warn!("Server '{}' has no way to save and exit; waiting for a force stop", server_name);
    let mut response = result.clone();
    response["success"] = json!(false);
    response["server"] = json!(server_name);
    if response.get("error").is_none() {
        response["error"] = result.get("message").cloned().unwrap_or_else(|| json!("graceful stop unavailable"));
    }
    response
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// R39 준비 알림을 기다린다 (그 인스턴스의 것만)
    async fn ready_event(sub: &mut crate::events::Subscription, id: &str) -> Value {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let e = tokio::time::timeout(left, sub.rx.recv()).await.expect("instance.ready arrives").unwrap();
            if e.kind == "instance.ready" && e.data["instanceId"] == id {
                return e.data.clone();
            }
        }
    }

    /// R39: 시작마다 한 번 `instance.ready`. 준비 문구가 콘솔에 찍히면 ok, 준비 전에 끝나면 start_failed,
    /// 사용자가 끄면 stopped_while_starting, 시간이 지나면 ready_timeout. 선언이 없으면 시작이 성공한 때 ok
    #[cfg(unix)]
    #[tokio::test]
    async fn each_start_says_once_whether_the_server_became_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let script = |sh: &str| format!(
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
             else:\n    print(json.dumps({{'success': True}}))\n"
        );
        let games = [
            ("ready", "ready_patterns = [\"^Done \\\\(\"]\n", "echo booting; sleep 0.5; echo 'Done (1.2s)'; sleep 30"),
            ("dies", "ready_patterns = [\"^Done\"]\n", "echo booting; sleep 0.5; exit 3"),
            ("slow", "ready_patterns = [\"^Done\"]\nready_timeout = 1\n", "echo booting; sleep 30"),
            ("quiet", "ready_patterns = [\"^Done\"]\n", "echo booting; sleep 30"),
            ("plain", "", "sleep 30"),
        ];
        for (name, detection, sh) in games {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\n{detection}")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), script(sh)).unwrap();
            crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        }
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for (id, module) in [("r-ready", "ready"), ("r-dies", "dies"), ("r-slow", "slow"), ("r-plain", "plain"), ("r-stopped", "quiet")] {
            let mut inst = crate::instance::ServerInstance::new(id, module);
            inst.id = id.into();
            sup.instance_store.add(inst).unwrap();
        }
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let mut sub = crate::events::bus().subscribe(None);
        let start = |id: &'static str, module: &'static str| {
            let sup = sup.clone();
            async move { Supervisor::start_managed_server(&sup, id, module, json!({})).await.unwrap() }
        };

        assert_eq!(start("r-plain", "plain").await["success"], true);
        assert_eq!(ready_event(&mut sub, "r-plain").await["ok"], true);

        assert_eq!(start("r-ready", "ready").await["success"], true);
        let ready = ready_event(&mut sub, "r-ready").await;
        assert_eq!((ready["ok"].clone(), ready["instance"].clone(), ready.get("code").cloned()), (json!(true), json!("r-ready"), None));

        let code = |e: Value| (e["ok"].clone(), e["code"].clone());
        start("r-dies", "dies").await;
        assert_eq!(code(ready_event(&mut sub, "r-dies").await), (json!(false), json!("start_failed")));

        start("r-slow", "slow").await;
        assert_eq!(code(ready_event(&mut sub, "r-slow").await), (json!(false), json!("ready_timeout")));

        start("r-stopped", "quiet").await;
        let stopped = Supervisor::stop_server(&sup, "r-stopped", "quiet", true).await.unwrap();
        assert_eq!(stopped["success"], true, "{stopped}");
        assert_eq!(code(ready_event(&mut sub, "r-stopped").await), (json!(false), json!("stopped_while_starting")));

        for id in ["r-plain", "r-ready", "r-slow"] {
            let _ = Supervisor::stop_server(&sup, id, "plain", true).await;
        }
    }

    /// MK8 §8.3: managed 시작에서 접속·퇴장 문구가 콘솔에 찍히면 `instance.player`. 시스템 줄과 다른 그룹은 싣지 않는다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_managed_server_says_who_joined_and_left() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("chatty");
        std::fs::create_dir_all(&dir).unwrap();
        let head = "^\\[\\d{2}:\\d{2}:\\d{2}(?:\\] \\[[^\\]]*/| )INFO\\]: ";
        let table = format!("player_join_patterns = ['{head}(?P<name>[A-Za-z0-9_]{{1,16}}) joined the game\\s*$']\nplayer_leave_patterns = ['{head}(?P<name>[A-Za-z0-9_]{{1,16}}) left the game\\s*$']\n");
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"chatty\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\n{table}")).unwrap();
        let sh = "echo '[20:07:05] [Server thread/INFO]: Steve joined the game'; echo '[20:07:06] [Server thread/INFO]: <Alex> ]: Bob joined the game'; sleep 0.6; echo '[20:07:09] [Server thread/INFO]: Steve left the game'; sleep 30";
        std::fs::write(dir.join("lifecycle.py"), format!(
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({{'success': True, 'program': '/bin/sh', 'args': ['-c', {sh:?}], 'working_dir': '.'}}))\n\
             else:\n    print(json.dumps({{'success': True}}))\n"
        )).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("p-chatty", "chatty");
        inst.id = "p-chatty".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let mut sub = crate::events::bus().subscribe(None);
        assert_eq!(Supervisor::start_managed_server(&sup, "p-chatty", "chatty", json!({})).await.unwrap()["success"], true);
        if let Some(proc) = sup.read().await.managed_store.get("p-chatty").await {
            proc.push_system_line("[20:07:07] [Server thread/INFO]: Mallory joined the game".into(), crate::supervisor::managed_process::LogLevel::Info).await;
        }

        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.len() < 2 {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let e = tokio::time::timeout(left, sub.rx.recv()).await.expect("instance.player arrives").unwrap();
            if e.kind == "instance.player" && e.data["instanceId"] == "p-chatty" {
                assert_eq!(e.data["instance"], "p-chatty");
                assert!(e.data["at"].is_u64());
                seen.push((e.data["player"].clone(), e.data["change"].clone()));
            }
        }
        assert_eq!(seen, vec![(json!("Steve"), json!("join")), (json!("Steve"), json!("leave"))], "a chat line and a system line are no player");
        let _ = Supervisor::stop_server(&sup, "p-chatty", "chatty", true).await;
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

    #[tokio::test]
    async fn test_process_monitoring() {
        let mut supervisor = Supervisor::new("./modules");
        
        // 모니터링이 에러 없이 실행되어야 함
        let snapshot = crate::process_monitor::get_running_processes();
        let result = supervisor.monitor_processes_taken_at(&snapshot, u64::MAX).await;
        assert!(result.is_ok());
    }

    // 백로그 L8: 감시는 프로세스 목록을 락 밖에서 먼저 찍는다. 그 사이에 막 시작해 추적한 PID는 목록에 없어
    // "끝났다"며 추적에서 풀렸다. 목록을 찍은 뒤에 추적한 PID는 다음 목록까지 기다린다
    #[tokio::test]
    async fn a_pid_tracked_after_the_snapshot_is_not_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let inst = crate::instance::ServerInstance::new("fresh", "none");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        sup.tracker.track(&id, 4_000_000).unwrap();
        let tracked_at = sup.tracker.get_start_time(&id).unwrap();

        sup.monitor_processes_taken_at(&[], tracked_at - 1).await.unwrap();
        assert!(sup.tracker.get_pid(&id).is_ok(), "the snapshot predates the start");
        sup.monitor_processes_taken_at(&[], tracked_at + 1).await.unwrap();
        assert!(sup.tracker.get_pid(&id).is_err(), "a later snapshot without the pid means it is gone");
    }

    /// cleanup_dead()가 죽은 managed process를 정리할 때
    /// stop_cooldowns에 등록되어 auto_detect 재탐색이 억제되는지 검증
    #[tokio::test]
    async fn test_cleanup_dead_registers_stop_cooldown() {
        let mut supervisor = Supervisor::new("./modules");

        // 가짜 인스턴스를 등록 (파일 시스템 없이 인메모리 테스트)
        // cleanup_dead가 dead_ids를 반환하면 monitor_processes가 cooldown을 삽입해야 함
        // managed_store에 직접 죽은 프로세스를 넣을 순 없으므로,
        // 쿨다운 삽입 로직 자체를 단위 테스트
        assert!(!supervisor.stop_cooldowns.contains_key("test-instance-id"));

        // dead_ids에 해당하는 쿨다운이 삽입됨을 시뮬레이션
        let dead_ids = vec!["test-instance-id".to_string()];
        for dead_id in &dead_ids {
            supervisor.stop_cooldowns.insert(dead_id.clone(), Instant::now());
        }

        assert!(supervisor.stop_cooldowns.contains_key("test-instance-id"));
        // 쿨다운이 30초 내이면 auto_detect를 건너뛰어야 함
        let stopped_at = supervisor.stop_cooldowns.get("test-instance-id").unwrap();
        assert!(stopped_at.elapsed().as_secs() < STOP_COOLDOWN_SECS);
    }

    /// D-S10: 정지는 자기가 올린 세대를 알려 준다. 재시작이 정지 전에 따로 읽은 세대를 기준으로 삼으면,
    /// 그 틈에 끼어든 자동 재시작 시작을 "다른 정지·시작"으로 보고 스스로 취소해 서버가 꺼진 채 남았다
    #[tokio::test]
    async fn a_stop_reports_the_generation_it_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.module_loader.discover_modules().unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        // 정지 전에 다른 쪽(자동 재시작)이 세대를 올려 둔다
        *sup.run_generation.entry(id.clone()).or_default() += 5;
        let sup = tokio::sync::RwLock::new(sup);
        let stopped_at = STOPPED_AT
            .scope(std::cell::Cell::new(None), async {
                let _ = Supervisor::stop_server(&sup, &id, "game", false).await;
                STOPPED_AT.with(|c| c.get())
            })
            .await;
        assert_eq!(stopped_at, Some(6));
        assert_eq!(sup.read().await.run_generation_of(&id), 6);
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

    /// 데몬보다 새 saba-core를 요구해 로드되지 못한 모듈도 시작 응답과 목록이 이유를 준다
    #[tokio::test]
    async fn module_needing_newer_core_reports_unsatisfied() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("future");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"future\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[module.dependencies]\nsaba-core = \">=99.0.0\"\n",
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        assert!(supervisor.module_loader.get_module("future").unwrap_err().to_string().contains("requires saba-core"));
        let failed = supervisor.module_loader.load_failure("future").unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("module_dependency_unsatisfied"));
        let resp = supervisor.unmet_dependencies_response("future", "srv").await.unwrap();
        assert_eq!(resp["error_code"], "module_dependency_unsatisfied");
        assert_eq!(resp["module"], "future", "clients name the module in \"update required\"");
        assert!(resp["missing_dependencies"][0].as_str().unwrap().starts_with("saba-core >=99.0.0"));
    }

    /// 데몬보다 새 스키마라 로드되지 못한 모듈은 시작 응답도 그 이유(module_schema_unsupported)를 준다.
    /// 빈 의존성 목록의 module_dependency_unsatisfied로 바꾸지 않는다
    #[tokio::test]
    async fn module_with_newer_schema_reports_unsupported_on_start() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("future");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 99\nname = \"future\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        let supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let resp = supervisor.unmet_dependencies_response("future", "srv").await.unwrap();
        assert_eq!(resp["error_code"], "module_schema_unsupported", "{resp}");
        assert_eq!((resp["required"].clone(), resp["module"].clone()), (json!(99), json!("future")), "{resp}");
    }

    /// 스키마가 틀려 로드되지 못한 모듈(module_schema_invalid)은 빈 의존성 오류로 바꾸지 않는다
    #[tokio::test]
    async fn module_with_invalid_schema_is_not_reported_as_unmet_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"broken\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let _ = supervisor.initialize().await;
        assert_eq!(supervisor.module_loader.load_failure("broken").and_then(|f| f.error_code).as_deref(), Some("module_schema_invalid"));
        assert_eq!(supervisor.unmet_dependencies_response("broken", "srv").await, None);
    }

    /// [module.dependencies]: 익스텐션이 없거나 낮으면 막고, 맞으면 통과. 데몬이 모르는 키(gui)는 넘긴다
    #[tokio::test]
    async fn module_dependencies_check_extension_versions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[module.dependencies]\next-foo = \">=0.2.0\"\ngui = \">=99.0.0\"\n",
        )
        .unwrap();
        let ext_root = tmp.path().join("extensions");
        let write_ext = |version: &str| {
            std::fs::create_dir_all(ext_root.join("foo")).unwrap();
            std::fs::write(ext_root.join("foo").join("manifest.json"), format!(r#"{{"id":"foo","name":"Foo","version":"{version}"}}"#)).unwrap();
        };
        let mut supervisor = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        supervisor.module_loader.discover_modules().unwrap();
        let unmet = supervisor.unmet_module_dependencies("game").await.unwrap();
        assert_eq!(unmet, vec!["ext-foo >=0.2.0 (not installed)".to_string()]);

        write_ext("0.1.0");
        let mut mgr = crate::extension::ExtensionManager::new_isolated(ext_root.to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("foo").unwrap();
        supervisor.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(mgr)));
        assert_eq!(supervisor.unmet_module_dependencies("game").await.unwrap(), vec!["ext-foo >=0.2.0 (installed 0.1.0)".to_string()]);

        write_ext("0.2.0");
        supervisor.extension_manager.as_ref().unwrap().write().await.discover().unwrap();
        assert!(supervisor.unmet_module_dependencies("game").await.is_none());
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
    fn module_failure_uses_message_for_the_existing_http_status_selection() {
        let value = json!({ "success": false, "error": "cannot start", "message": "jar missing", "module": "game", "function": "get_launch_command" });
        let error = module_failure(&value, "game", "get_launch_command");
        assert_eq!(error.to_string(), "jar missing");
        assert_eq!(crate::plugin::error_response(&error), value);
    }

    #[test]
    fn a_legacy_stop_unavailable_is_still_a_failure() {
        let result = graceful_stop_unavailable("game", &json!({ "error_code": "graceful_stop_unavailable", "message": "no stop path", "module": "game", "function": "stop" }));
        assert_eq!(result["success"], false);
        assert_eq!(result["message"], "no stop path");
        assert_eq!(result["module"], "game");
        assert_eq!(result["function"], "stop");
    }

    // 0.1.7 업그레이드: 0.1.6에서 만든 팰월드는 기본 REST 8212를 저장해 두었다. 8212가 Windows 예약 범위(8135-8234)에
    // 들면 시작 전에 새 인스턴스처럼 옮겨 저장해야 서버가 뜬다. 사용자가 정한 기본값 아닌 포트는 그대로 둔다
    #[test]
    fn a_reserved_default_rest_port_moves_before_start() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("pal");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"pal\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"rest\"]\n\n\
             [[settings.fields]]\nname = \"rest_port\"\ntype = \"number\"\nport = true\nlabel = \"REST\"\ndefault = 8212\n",
        )
        .unwrap();
        let mut sup = Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut old = crate::instance::ServerInstance::new("pal-016", "pal");
        old.rest_port = Some(8212);
        old.module_settings.insert("rest_port".into(), json!(8212));
        let mut other = crate::instance::ServerInstance::new("pal-other", "pal");
        other.rest_port = Some(8242);
        let mut picked = crate::instance::ServerInstance::new("pal-picked", "pal");
        picked.rest_port = Some(8200);
        let (old_id, picked_id) = (old.id.clone(), picked.id.clone());
        for inst in [old, other, picked] {
            sup.instance_store.add(inst).unwrap();
        }
        let reserved = |p: u16| (8135..=8234).contains(&p);

        let mut events = crate::events::bus().subscribe(None);
        assert_eq!(sup.move_default_rest_port_if(&old_id, reserved, reserved), Some(8252));
        // 리뷰 289: 자동·예약 재시작은 시작 응답을 로그에만 남기므로, 옮긴 일은 이벤트로도 알린다
        let told = std::iter::from_fn(|| events.rx.try_recv().ok()).find(|e| e.kind == "instance.port_moved" && e.data["instanceId"] == old_id.as_str());
        let told = told.expect("instance.port_moved was not emitted");
        assert_eq!((told.data["field"].clone(), told.data["from"].clone(), told.data["to"].clone()), (json!("rest_port"), json!(8212), json!(8252)));
        let moved = sup.instance_store.get(&old_id).unwrap();
        assert_eq!((moved.rest_port, moved.module_settings["rest_port"].clone()), (Some(8252), json!(8252)));
        let mut reloaded = crate::instance::InstanceStore::new(tmp.path().join("instances").to_str().unwrap());
        reloaded.load().unwrap();
        assert_eq!(reloaded.get(&old_id).unwrap().rest_port, Some(8252));

        assert_eq!(sup.move_default_rest_port_if(&picked_id, reserved, reserved), None);
        assert_eq!(sup.instance_store.get(&picked_id).unwrap().rest_port, Some(8200));
    }

    // 실제 PC: 새 팰월드의 REST 포트를 예약 범위 밖(8242)으로 옮겼는데 모듈 설정에 기본 INI의 8212가 남아,
    // 모듈이 시작 때 INI를 8212로 맞추고 데몬은 8242로 접속해 REST가 끝내 닿지 않았다
    #[test]
    fn modules_get_the_instance_rest_port_over_a_stale_module_setting() {
        let mut inst = crate::instance::ServerInstance::new("pal", "palworld");
        inst.rest_port = Some(8242);
        inst.module_settings.insert("rest_port".into(), json!(8212));
        let mut cfg = serde_json::Map::new();
        Supervisor::insert_instance_fields(&mut cfg, &inst, std::path::Path::new("/data/instances/pal"));
        assert_eq!(cfg["rest_port"], json!(8242));
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
        for key in ["install_dir", "working_dir", "instance_dir"] {
            assert!(cfg.get(key).is_none(), "{key} from module settings");
        }
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
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_right_after_a_crash_does_not_restart_the_server() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[detection]\ncrash_artifacts = [\"hs_err_pid*.log\"]\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        supervisor.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "crashed".into();
        supervisor.instance_store.add(inst).unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mut proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), "touch hs_err_pid1.log; exit 1".into()], work.to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        proc.started_at -= 600;
        supervisor.managed_store.insert("crashed", proc).await;
        for _ in 0..50 {
            if supervisor.managed_store.running_instance_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let sup = tokio::sync::RwLock::new(supervisor);
        Supervisor::stop_server(&sup, "crashed", "game", false).await.unwrap();

        let mut s = sup.write().await;
        assert_eq!(s.exit_events("crashed").last().map(|e| e.verdict.kind), Some(crash::ExitKind::Crash), "it did crash");
        assert!(s.take_pending_restarts().iter().all(|(id, ..)| id != "crashed"), "but the stop keeps it down");
    }

    /// 끝난 managed 서버를 크래시·정상 종료·사용자 정지·강제 종료로 가르고 크래시만 재시작한다 (A1)
    #[cfg(unix)]
    #[tokio::test]
    async fn exits_are_judged_and_only_crashes_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [detection]\ngraceful_exit_patterns = [\"Stopping the server\"]\ncrash_artifacts = [\"hs_err_pid*.log\"]\n",
        )
        .unwrap();
        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), instances_dir.to_str().unwrap());

        let cases = [
            ("crash", "touch hs_err_pid1.log; exit 1", None),
            ("clean", "echo Stopping the server; exit 1", None),
            ("killed", "exit 1", None),
            ("stopped", "touch hs_err_pid1.log; exit 1", None),
            ("off", "touch hs_err_pid1.log; exit 1", Some(false)),
            ("reattached", "exit 1", None),
        ];
        for (id, script, auto_restart) in cases {
            let mut inst = crate::instance::ServerInstance::new(id, "game");
            inst.id = id.to_string();
            if let Some(flag) = auto_restart {
                inst.module_settings.insert("auto_restart".into(), json!(flag));
            }
            supervisor.instance_store.add(inst).unwrap();
            let work = tmp.path().join(id);
            std::fs::create_dir_all(&work).unwrap();
            let mut proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), script.into()], work.to_str().unwrap(), Vec::new(), None, None)
                .await
                .unwrap();
            // 켜지자마자 끝난 것(start_failed)과 가르려고, 한동안 돌다 끝난 것으로 둔다.
            // 다시 붙은 프로세스는 붙은 시각만 알므로 그대로 두어도 시작 실패가 아니다
            if id == "reattached" {
                proc.reattached = true;
            } else {
                proc.started_at -= 600;
            }
            supervisor.managed_store.insert(id, proc).await;
        }
        supervisor.stop_requested.insert("stopped".into());
        for _ in 0..50 {
            if supervisor.managed_store.running_instance_ids().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let mut exits = crate::events::bus().subscribe(None);
        supervisor.monitor_processes_taken_at(&[], u64::MAX).await.unwrap();
        // 다시 띄우기로 한 종료는 그 복구 시도의 번호(예약한 재시작의 실행 세대)를 싣는다
        let crash_exit = std::iter::from_fn(|| exits.rx.try_recv().ok()).find(|e| e.kind == "instance.exit" && e.data["instanceId"] == "crash").unwrap();
        assert_eq!(crash_exit.data["restartAttempt"], 0);

        let kind = |id: &str| supervisor.exit_events(id).last().map(|e| e.verdict.kind);
        assert_eq!(kind("crash"), Some(crash::ExitKind::Crash));
        assert_eq!(kind("clean"), Some(crash::ExitKind::InGameStop));
        assert_eq!(kind("killed"), Some(crash::ExitKind::UnknownExit));
        assert_eq!(kind("stopped"), Some(crash::ExitKind::UserStop));
        assert_eq!(kind("off"), Some(crash::ExitKind::Crash));
        assert_eq!(kind("reattached"), Some(crash::ExitKind::UnknownExit), "a server running before the daemon restart did not fail to start");
        assert_eq!(supervisor.exit_events("off")[0].note.as_deref(), Some("auto_restart is off"));
        assert_eq!(supervisor.exit_events("off")[0].note_code.as_deref(), Some("auto_restart_off"));
        let mut sub = crate::events::bus().subscribe(None);
        supervisor.record_restart_failure("crash", 0, "no java".into());
        let failed = supervisor.exit_events("crash").last().cloned().unwrap();
        assert_eq!((failed.note_code.as_deref(), failed.note_args), (Some("restart_failed"), Some(json!({"reason": "no java"}))));
        // 이미 나간 instance.exit 뒤에 실패를 따로 알린다
        let mut told = false;
        while let Ok(e) = sub.rx.try_recv() {
            told |= e.kind == "instance.restart_failed" && e.data["noteArgs"]["reason"] == "no java" && e.data["restartAttempt"] == 0;
        }
        assert!(told, "restart failure was not announced on the event stream");
        // 복구 시도의 끝도 알린다: 다시 켜졌거나, 대기 중에 정지·시작해서 그만뒀거나.
        // 어느 시도의 끝인지 시작(`instance.exit`)과 같은 `restartAttempt`로 가른다
        supervisor.record_restart_outcome("crash", 7, true);
        supervisor.record_restart_outcome("crash", 8, false);
        let ends: Vec<(String, Value)> = std::iter::from_fn(|| sub.rx.try_recv().ok())
            .filter(|e| e.data["instanceId"] == "crash")
            .map(|e| (e.kind.clone(), e.data["restartAttempt"].clone()))
            .collect();
        assert_eq!(ends, vec![("instance.restarted".to_string(), json!(7)), ("instance.restart_cancelled".to_string(), json!(8))]);
        assert_eq!(supervisor.take_pending_restarts(), vec![("crash".to_string(), Duration::from_secs(5), 0)]);
        assert!(supervisor.restart_still_wanted("crash", 0).await);
        supervisor.stop_requested.insert("crash".into());
        assert!(!supervisor.restart_still_wanted("crash", 0).await, "a stop during the wait cancels the restart");
        // 꺼진 서버에 보낸 정지는 정지 표시를 바로 지운다: 그래도 대기 중 정지가 있었으면 재시작하지 않는다
        supervisor.stop_requested.remove("crash");
        let sup = tokio::sync::RwLock::new(supervisor);
        Supervisor::stop_server(&sup, "crash", "game", false).await.unwrap();
        assert!(!sup.read().await.restart_still_wanted("crash", 0).await, "a stop of the already exited server cancels the restart");

        // 데몬을 다시 켜도 종료 이력이 남는다
        let restarted = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), instances_dir.to_str().unwrap());
        assert_eq!(restarted.exit_events("crash").last().map(|e| e.verdict.kind), Some(crash::ExitKind::Crash));
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

    /// 콘솔이 있는 managed 서버의 일반 정지는 stop_command를 stdin으로 보내고 스스로 끝나기를 기다린다
    /// (강제 종료 먼저가 아니다: 세이브를 건너뛰지 않게)
    #[cfg(unix)]
    #[tokio::test]
    async fn normal_stop_asks_the_console_and_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let dir = modules_dir.join("console-game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nname = \"console-game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_command = \"quit-now\"\n\n[protocols]\nsupported = [\"stdin\"]\n",
        )
        .unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let mut inst = crate::instance::ServerInstance::new("srv", "console-game");
        inst.id = "srv".into();
        supervisor.instance_store.add(inst).unwrap();
        // 받은 줄을 남기고 스스로 끝나는 "게임 서버"
        let got = tmp.path().join("got");
        let script = format!("read line; printf '%s' \"$line\" > '{}'; exit 0", got.display());
        let proc = managed_process::ManagedProcess::spawn("sh", &["-c".into(), script], tmp.path().to_str().unwrap(), Vec::new(), None, None)
            .await
            .unwrap();
        supervisor.managed_store.insert("srv", proc).await;

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let started = std::time::Instant::now();
        let result = Supervisor::stop_server(&supervisor, "srv", "console-game", false).await.unwrap();
        assert_eq!(result["success"], true, "{}", result);
        assert_eq!(std::fs::read_to_string(&got).unwrap(), "quit-now", "the stop command went over stdin");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "it exited on its own, not after the kill timeout");
    }

    /// 준비 문구 전에 받은 명령을 실패하는 서버(마인크래프트 26.3: "An unexpected error occurred")도
    /// 켜지는 중에 정지하면 준비된 뒤 stop_command를 받아 스스로 저장하고 끝난다 (강제 종료로 넘어가지 않는다)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_while_starting_waits_for_the_ready_line() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("slow-console");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"slow-console\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [config]\nstop_command = \"quit-now\"\nstop_timeout = 4\n\n[protocols]\nsupported = [\"stdin\"]\n\n\
             [extensions.saba-core]\nready_patterns = [\"^Done\"]\n",
        )
        .unwrap();
        // 준비 전에 온 줄은 버리고(실패), 준비 뒤의 quit-now에만 저장 표시를 남기고 끝나는 서버
        let saved = tmp.path().join("saved");
        let game = format!(
            "import os, select, sys, time\n\
             print('booting', flush=True)\n\
             time.sleep(1.5)\n\
             while select.select([0], [], [], 0)[0] and os.read(0, 4096):\n    pass\n\
             print('Done (1.5s)!', flush=True)\n\
             for line in sys.stdin:\n    \
             if line.strip() == 'quit-now':\n        open({saved:?}, 'w').write('saved')\n        sys.exit(0)\n",
            saved = saved.display().to_string()
        );
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, sys\n\
                 if sys.argv[1] == 'get_launch_command':\n    \
                 print(json.dumps({{'success': True, 'program': 'python3', 'args': ['-c', {game:?}], 'working_dir': '.'}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n"
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("s-early", "slow-console");
        inst.id = "s-early".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));
        let started = Supervisor::start_managed_server(&sup, "s-early", "slow-console", json!({})).await.unwrap();
        assert_eq!(started["success"], true, "{started}");

        let stopped = Supervisor::stop_server(&sup, "s-early", "slow-console", false).await.unwrap();
        assert_eq!(stopped["success"], true, "{stopped}");
        assert_eq!(std::fs::read_to_string(&saved).ok().as_deref(), Some("saved"), "the stop command reached the ready server");
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

    /// 재부팅 뒤 기록된 PID를 다른 프로세스가 쓰면 다시 붙지 않는다 (붙으면 정지할 때 그 프로세스를 끈다)
    #[tokio::test]
    async fn a_pid_reused_after_reboot_is_not_reattached() {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        for (id, before_start) in [("reused", true), ("same", false)] {
            let mut inst = crate::instance::ServerInstance::new(id, "minecraft");
            inst.id = id.into();
            supervisor.instance_store.add(inst).unwrap();
            let inst_dir = supervisor.instance_store.instance_dir(id);
            std::fs::create_dir_all(&inst_dir).unwrap();
            let pid_file = inst_dir.join("managed.pid");
            std::fs::write(&pid_file, std::process::id().to_string()).unwrap();
            if before_start {
                // 이 프로세스가 뜨기 한 시간 전에 기록한 PID
                let started = process::process_start_time(std::process::id()).unwrap();
                let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(started - 3600);
                std::fs::File::options().write(true).open(&pid_file).unwrap().set_modified(at).unwrap();
            }
        }
        supervisor.reattach_orphaned_managed().await;
        assert!(supervisor.tracker.get_pid("reused").is_err(), "a process that started after the record is someone else");
        assert!(!supervisor.instance_store.instance_dir("reused").join("managed.pid").exists());
        assert_eq!(supervisor.tracker.get_pid("same").unwrap(), std::process::id());
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
            let _ = supervisor.managed_store.get(id).await.unwrap().force_kill();
        }
        assert_eq!(rest["stdin_available"], false, "{rest}");
        assert_eq!(stdin["stdin_available"], true, "{stdin}");
    }

    /// 데몬이 다시 붙으면 콘솔 줄 번호가 0부터 다시 매겨진다. PID가 같아도 클라이언트가 이어 읽던 번호를
    /// 버리게 콘솔 응답의 `epoch`가 바뀐다 (그대로면 새 줄이 옛 번호를 넘을 때까지 콘솔이 멈추거나 줄이 겹친다)
    #[tokio::test]
    async fn a_reattached_console_reports_a_new_epoch() {
        let tmp = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new_with_instances_dir("./modules", tmp.path().to_str().unwrap());
        let inst_dir = tmp.path().join("srv");
        let mut epochs = Vec::new();
        for _ in 0..2 {
            let stub = managed_process::ManagedProcess::create_reattached_stub(std::process::id(), &inst_dir).await;
            supervisor.managed_store.insert("srv", stub).await;
            let out = supervisor.get_console_output("srv", Some(0), None).await.unwrap();
            epochs.push(out["epoch"].as_u64().unwrap_or_else(|| panic!("no epoch: {out}")));
        }
        assert_ne!(epochs[0], epochs[1]);
    }

    /// start_managed_server()가 이미 실행 중인 managed 프로세스를 감지하여
    /// 중복 프로세스 생성을 방지하는지 검증
    #[tokio::test]
    async fn test_start_managed_rejects_duplicate_process() {
        let tmp_dir = std::env::temp_dir().join(format!("saba-test-dup-guard-{}", std::process::id()));
        let instances_dir = tmp_dir.join("instances");
        let _ = std::fs::create_dir_all(&instances_dir);

        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());

        // 테스트 인스턴스 등록
        let inst = crate::instance::ServerInstance {
            id: "test-dup-guard".to_string(),
            name: "TestServer".to_string(),
            module_name: "minecraft".to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            port: Some(25565),
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            schedule: Vec::new(),
            backup_keep: None,
            extra_args: Vec::new(),
            memory_limit_mb: None,
            accepted_agreements: Default::default(),
            user_settings: Default::default(),
            imported_settings: Default::default(),
            data_roots_backfill_attempted: Default::default(),
        };
        supervisor.instance_store.add(inst).unwrap();

        // 현재 프로세스의 PID로 reattached stub 생성 (is_running() == true 보장)
        let current_pid = std::process::id();
        let inst_dir = instances_dir.join("test-dup-guard");
        let _ = std::fs::create_dir_all(inst_dir.join("logs"));
        let stub = managed_process::ManagedProcess::create_reattached_stub(current_pid, &inst_dir).await;
        assert!(stub.is_running(), "Reattached stub should report running");

        supervisor.managed_store.insert("test-dup-guard", stub).await;

        // start_managed_server 호출 → 이미 실행 중이므로 중복 방지되어야 함
        let supervisor = tokio::sync::RwLock::new(supervisor);
        let result = Supervisor::start_managed_server(&supervisor, "test-dup-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["success"], true);
        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// tracker에 PID가 등록되어 있고 프로세스가 살아있으면
    /// managed_store에 없더라도 중복 시작을 방지하는지 검증
    #[tokio::test]
    async fn test_start_managed_rejects_when_tracker_has_live_pid() {
        let tmp_dir = std::env::temp_dir().join(format!("saba-test-tracker-guard-{}", std::process::id()));
        let instances_dir = tmp_dir.join("instances");
        let _ = std::fs::create_dir_all(&instances_dir);

        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());

        let inst = crate::instance::ServerInstance {
            id: "test-tracker-guard".to_string(),
            name: "TestServer2".to_string(),
            module_name: "minecraft".to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            port: Some(25566),
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            schedule: Vec::new(),
            backup_keep: None,
            extra_args: Vec::new(),
            memory_limit_mb: None,
            accepted_agreements: Default::default(),
            user_settings: Default::default(),
            imported_settings: Default::default(),
            data_roots_backfill_attempted: Default::default(),
        };
        let inst_dir = instances_dir.join("test-tracker-guard");
        let _ = std::fs::create_dir_all(&inst_dir);
        supervisor.instance_store.add(inst).unwrap();

        // managed_store에는 없지만 tracker에 현재 프로세스 PID를 등록
        let current_pid = std::process::id();
        supervisor.tracker.track("test-tracker-guard", current_pid).unwrap();

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let result = Supervisor::start_managed_server(&supervisor, "test-tracker-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["success"], true);
        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// non-managed 시작도 추적 중인 PID가 살아 있으면 새로 띄우지 않고 추적을 덮어쓰지 않는다
    #[tokio::test]
    async fn test_start_non_managed_rejects_when_tracker_has_live_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let instances_dir = tmp.path().join("instances");
        std::fs::create_dir_all(&instances_dir).unwrap();
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        let mut inst = crate::instance::ServerInstance::new("NonManaged", "minecraft");
        inst.id = "test-nm-guard".into();
        supervisor.instance_store.add(inst).unwrap();
        let current_pid = std::process::id();
        supervisor.tracker.track("test-nm-guard", current_pid).unwrap();

        let supervisor = tokio::sync::RwLock::new(supervisor);
        let result = Supervisor::start_server(&supervisor, "test-nm-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);
        assert_eq!(supervisor.read().await.tracker.get_pid("test-nm-guard").unwrap(), current_pid);
    }

    /// 시작 준비(모듈 호출) 중에 들어온 정지는 PID가 없어 바로 성공으로 끝난다. 그 뒤 서버가 떠서는 안 된다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_start_preparation_keeps_the_server_down() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\n\
             if sys.argv[1] == 'get_launch_command':\n    time.sleep(2)\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-start".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));

        let starter = {
            let sup = sup.clone();
            tokio::spawn(async move { Supervisor::start_managed_server(&sup, "stop-during-start", "game", json!({})).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let stopped = Supervisor::stop_server(&sup, "stop-during-start", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");

        let started = starter.await.unwrap().unwrap();
        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let s = sup.read().await;
        assert!(s.tracker.get_pid("stop-during-start").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-during-start").await.is_none(), "no server was left running");
    }

    /// 익스텐션이 맡은 시작(pre_start handled) 중에 들어온 정지: 정지 hook이 먼저 끝나 익스텐션 서버가 뒤늦게 뜬다.
    /// 시작은 취소로 끝나고, 익스텐션의 정지를 다시 불러 그 서버를 끈다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_an_extension_start_stops_what_the_extension_started() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let ext_root = tmp.path().join("extensions");
        let ext = ext_root.join("box");
        std::fs::create_dir_all(&ext).unwrap();
        let manifest = json!({
            "id": "box", "name": "box", "version": "0.1.0",
            "python_modules": { "m": "m.py" },
            "hooks": {
                "server.pre_start": { "module": "m", "function": "start" },
                "server.stop": { "module": "m", "function": "stop" },
            },
        });
        std::fs::write(ext.join("manifest.json"), manifest.to_string()).unwrap();
        let log = tmp.path().join("hook.log");
        std::fs::write(
            ext.join("m.py"),
            format!(
                "import json, sys, time\nfn = sys.argv[1]\nif fn == 'start':\n    time.sleep(2)\n\
                 open({log:?}, 'a').write(fn + '\\n')\nprint(json.dumps({{'handled': True, 'success': True}}))\n",
                log = log.to_str().unwrap()
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        crate::sandbox::consent_test_component(&ext.join("m.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut mgr = crate::extension::ExtensionManager::new_isolated(ext_root.to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("box").unwrap();
        sup.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(mgr)));
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-ext-start".into();
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));

        let starter = {
            let sup = sup.clone();
            tokio::spawn(async move { Supervisor::start_server(&sup, "stop-during-ext-start", "game", json!({})).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let stopped = Supervisor::stop_server(&sup, "stop-during-ext-start", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");

        let started = starter.await.unwrap().unwrap();
        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["stop", "start", "stop"], "the extension's server is stopped again");
    }

    /// managed 시작의 server.pre_start hook도 인스턴스와 시작 설정의 비밀 값을 가려서 넘긴다 (D-E2와 같은 기준)
    #[cfg(unix)]
    #[tokio::test]
    async fn the_managed_pre_start_hook_gets_no_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let ext_root = tmp.path().join("extensions");
        let ext = ext_root.join("box");
        std::fs::create_dir_all(&ext).unwrap();
        let manifest = json!({
            "id": "box", "name": "box", "version": "0.1.0",
            "python_modules": { "m": "m.py" },
            "hooks": { "server.pre_start": { "module": "m", "function": "start" } },
        });
        std::fs::write(ext.join("manifest.json"), manifest.to_string()).unwrap();
        let seen = tmp.path().join("seen.txt");
        std::fs::write(
            ext.join("m.py"),
            format!(
                "import json, sys\nopen({seen:?}, 'w').write(' '.join(sys.argv) + sys.stdin.read())\n\
                 print(json.dumps({{'handled': True, 'success': False, 'error': 'stop here'}}))\n",
                seen = seen.to_str().unwrap()
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        crate::sandbox::consent_test_component(&ext.join("m.py"));
        let mut sup = Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut mgr = crate::extension::ExtensionManager::new_isolated(ext_root.to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("box").unwrap();
        sup.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(mgr)));
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "pre-start-secrets".into();
        inst.rest_password = Some("hunter2-rest".into());
        inst.module_settings.insert("server_password".into(), json!("hunter2-join"));
        sup.instance_store.add(inst).unwrap();
        let sup = std::sync::Arc::new(tokio::sync::RwLock::new(sup));

        let config = json!({ "server_password": "hunter2-join", "rest_password": "hunter2-rest", "port": 1 });
        let started = Supervisor::start_managed_server(&sup, "pre-start-secrets", "game", config).await.unwrap();
        assert_eq!(started["success"], false, "{started}");
        let ctx = std::fs::read_to_string(&seen).unwrap();
        assert!(ctx.contains("pre-start-secrets"), "the hook ran with the context: {ctx}");
        assert!(!ctx.contains("hunter2"), "secrets reached the extension: {ctx}");
    }

    // 백로그 M13: 예약 범위 때문에 옮긴 REST 포트를 알리지 않고 저장값만 바꿨다
    #[test]
    fn a_start_reports_the_port_it_moved() {
        let moved = with_port_moved(json!({ "success": false, "error_code": "port_conflict" }), Some((8212, 8242)));
        assert_eq!(moved["port_moved"], json!([{ "field": "rest_port", "from": 8212, "to": 8242 }]));
        assert_eq!(moved["error_code"], "port_conflict");
        assert!(with_port_moved(json!({ "success": true }), None).get("port_moved").is_none());
    }

    /// 시작은 락 밖에서 진행되므로, 같은 인스턴스의 두 번째 시작은 표시로 막는다 (C12)
    #[test]
    fn start_guard_blocks_a_second_start_until_dropped() {
        let set = std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));
        let first = StartGuard::claim(&set, "a", 0).expect("first start");
        assert!(StartGuard::claim(&set, "a", 0).is_none());
        assert!(StartGuard::claim(&set, "b", 0).is_some(), "other instances are not blocked");
        drop(first);
        assert!(StartGuard::claim(&set, "a", 0).is_some());
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
}
