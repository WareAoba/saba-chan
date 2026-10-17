pub mod process;
pub mod module_loader;
pub mod managed_process;

use anyhow::Result;
use process::ProcessTracker;
use module_loader::{ModuleLoader, LoadedModule};
use managed_process::{ManagedProcess, ManagedProcessStore};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::process::Command as TokioCommand;
use crate::instance::InstanceStore;

/// Stop 후 auto-detect를 억제하는 쿨다운 시간 (초)
const STOP_COOLDOWN_SECS: u64 = 30;

/// track() 직후 이 시간(초) 동안은 monitor_processes의 "snapshot에 없으면 untrack"
/// 판정을 보류한다. process_snapshot은 write lock 획득 *전에* 미리 스캔되므로,
/// 스캔 이후·락 획득 이전 사이에 막 시작된 프로세스는 그 스냅샷에 잡히지 않을 수 있다.
/// 이 grace period가 없으면 방금 시작한(특히 non-managed 경로로 시작한) 서버가
/// 첫 모니터 사이클에서 즉시 오추적 해제되어, 실제로는 실행 중인데도 대시보드에는
/// 영구히 "정지중"으로 표시되는 문제가 발생한다(추적이 auto_detect 없이는 다시
/// 채워지지 않기 때문). MONITOR_INTERVAL_SECS(2초)의 배수 이상으로 넉넉히 잡아
/// 다음 사이클에서는 반드시 최신 스냅샷으로 재평가되도록 한다.
const TRACK_GRACE_SECS: u64 = 10;

/// 포트 충돌로 인한 강제 정지 이벤트
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PortConflictStopEvent {
    /// 강제 정지된 인스턴스 이름
    pub stopped_name: String,
    /// 강제 정지된 인스턴스 ID
    pub stopped_id: String,
    /// 충돌 상대 인스턴스 이름
    pub existing_name: String,
    /// 충돌 포트 번호
    pub port: u16,
    /// 타임스탬프 (epoch ms)
    pub timestamp_ms: u64,
}

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
    /// 포트 충돌로 인한 강제 정지 이벤트 큐 (GUI에서 폴링 후 drain)
    pub port_conflict_stops: std::sync::Mutex<Vec<PortConflictStopEvent>>,
    /// GUI 설정: 포트 충돌 검사 건너뛰기 (portConflictCheck 비활성화 시 true)
    pub skip_port_check: std::sync::Arc<AtomicBool>,
}

impl Supervisor {
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
            port_conflict_stops: std::sync::Mutex::new(Vec::new()),
            skip_port_check: std::sync::Arc::new(AtomicBool::new(false)),
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

        // 고아 managed 프로세스 재연결 + 초기 auto-detect 패스.
        // 스냅샷을 한 번만 스캔해 양쪽에 재사용한다.
        // 초기 monitor 패스가 없으면 첫 틱(2초)까지 non-managed 러닝 서버가
        // stopped로 보이고, 그 사이 start 요청 시 중복 spawn될 수 있다.
        let process_snapshot = crate::process_monitor::get_running_processes_async().await;
        self.reattach_orphaned_managed(&process_snapshot).await;
        if let Err(e) = self.monitor_processes(&process_snapshot).await {
            tracing::warn!("Initial monitor pass failed: {}", e);
        }

        // server_version 백필은 IPC 시작을 차단하지 않도록 main 에서 IPC up 이후
        // 별도 태스크로 호출한다. 여기서 호출하지 않는다.

        Ok(())
    }

    /// Detect orphaned managed processes from a previous daemon run and restore
    /// their console log history from disk.
    ///
    /// For each instance that has a `managed.pid` file:
    /// 1. Read the PID and check if it's still alive
    /// 2. Verify process identity against `managed.meta.json` (PID 재사용 오인 방지)
    /// 3. If alive & identity matches: re-register in tracker + create a stub
    ///    ManagedProcess with console history loaded from `logs/console.log`
    /// 4. Otherwise: clean up the stale PID files
    async fn reattach_orphaned_managed(&mut self, process_snapshot: &[crate::process_monitor::RunningProcess]) {
        let instances: Vec<_> = self.instance_store.list().to_vec();
        let mut reattached = 0u32;

        for instance in &instances {
            let inst_dir = self.instance_store.instance_dir(&instance.id);
            let pid_file = inst_dir.join("managed.pid");

            let pid_str = match std::fs::read_to_string(&pid_file) {
                Ok(s) => s.trim().to_string(),
                Err(_) => continue, // no managed.pid → not a managed instance
            };

            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    remove_managed_pid_files(&inst_dir);
                    continue;
                }
            };

            if !process::is_process_alive(pid) {
                // Process is dead — clean up stale PID files
                tracing::info!(
                    "Cleaning up stale managed.pid for '{}' (PID {} no longer running)",
                    instance.name, pid
                );
                remove_managed_pid_files(&inst_dir);
                continue;
            }

            // PID 재사용 검증: 데몬이 꺼져 있는 동안 서버가 종료되고 같은 PID가
            // 무관한 프로세스에 재할당됐을 수 있다 (Windows는 PID 재사용이 빈번하고,
            // OS 재부팅 후에도 pid 파일이 남는다). 오인 재연결하면 stop 시 무관한
            // 프로세스를 강제 종료하게 되므로, spawn 시 기록한 프로그램과 실제
            // 프로세스의 바이너리가 일치할 때만 재연결한다.
            if !reattach_identity_matches(&inst_dir, pid, process_snapshot) {
                tracing::warn!(
                    "PID {} for '{}' is alive but belongs to a different program (PID reuse) — skipping reattach",
                    pid, instance.name
                );
                remove_managed_pid_files(&inst_dir);
                continue;
            }

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
        }

        if reattached > 0 {
            tracing::info!("Reattached {} orphaned managed process(es)", reattached);
        }
    }

    /// 로드된 모듈로부터 module_name → protocols_supported 맵을 빌드합니다.
    /// `check_port_conflicts`에 전달하여 미지원 프로토콜 포트를 충돌 검사에서 제외합니다.
    fn build_module_protocols_map(&self) -> HashMap<String, Vec<String>> {
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
    /// 인스턴스의 실행 파일 경로를 자동 해석합니다.
    ///
    /// 우선순위:
    /// 1. `instance.executable_path` — 사용자가 명시한 경로 (override)
    /// 2. `working_dir / module.server_executable` — 모듈이 정의한 바이너리 파일명으로 자동 계산
    ///
    /// 모듈에 `server_executable`이 정의되어 있으면 `working_dir`만으로 실행 파일을 찾을 수 있으므로
    /// 사용자가 별도로 `executable_path`를 설정할 필요가 없습니다.
    fn resolve_executable(&self, instance: &crate::instance::ServerInstance, module_name: &str) -> Option<String> {
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

    /// 해석된 실행 파일 경로를 config map에 삽입합니다.
    fn insert_resolved_exe(&self, config: &mut serde_json::Map<String, Value>, instance: &crate::instance::ServerInstance, module_name: &str) {
        if let Some(exe_path) = self.resolve_executable(instance, module_name) {
            config.insert("server_executable".to_string(), json!(&exe_path));
            config.entry("server_jar".to_string()).or_insert_with(|| json!(&exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            config.insert("working_dir".to_string(), json!(work_dir));
        }
    }

    /// Start a server by name (e.g., "my-server-1")
    /// Called by IPC API: POST /api/server/:name/start
    /// 모듈 `[module.dependencies]` 요구 버전을 설치 매니페스트와 대조한다.
    /// 불충족 시 에러 응답(Value)을 반환해 시작을 단락시킨다.
    ///
    /// `installed-manifest.json`에는 saba-core/cli/gui 등 코어 컴포넌트 버전만 기록되고
    /// 익스텐션 버전은 없다. module.toml이 `ext-steamcmd` 처럼 익스텐션에 대한 최소 버전을
    /// 선언한 경우를 지원하기 위해, `ExtensionManager`가 디스크에서 발견한 실제 버전을
    /// `ext-{id}` 네임스페이스로 보충한다 (없으면 항상 "not installed"로 오판되어
    /// 정상 설치된 모듈도 시작이 막히는 사고가 났었다).
    async fn check_module_dependencies(&self, module_name: &str, server_name: &str) -> Option<Value> {
        let loaded_module = self.module_loader.get_module(module_name).ok()?;
        if loaded_module.metadata.dependencies.is_empty() {
            return None;
        }
        // 개발 빌드에서 핵심 컴포넌트가 누락되면 데몬의 CARGO_PKG_VERSION으로 보충해
        // 잘못된 의존성 차단을 방지한다.
        let mut installed = saba_chan_updater_lib::UpdateManager::load_installed_manifest();
        let core_ver = env!("CARGO_PKG_VERSION");
        for key in ["saba-core", "cli", "gui", "updater", "installer"] {
            installed.entry(key.to_string()).or_insert_with(|| core_ver.to_string());
        }
        // 익스텐션 버전 보충: manifest.json에서 실제 발견된 버전을 "ext-{id}" 키로 채운다.
        if let Some(ext_mgr) = &self.extension_manager {
            let guard = ext_mgr.read().await;
            for ext in guard.list() {
                installed.entry(format!("ext-{}", ext.id)).or_insert(ext.version);
            }
        }
        let issues = loaded_module.metadata.check_dependencies(&installed);
        if issues.is_empty() {
            return None;
        }
        let formatted: Vec<String> = issues.iter().map(|(k, req, got)| {
            match got {
                Some(v) => format!("{} requires {} (installed: {})", k, req, v),
                None => format!("{} requires {} (not installed)", k, req),
            }
        }).collect();
        tracing::error!(
            "Module '{}' dependency check failed for instance '{}': {:?}",
            module_name, server_name, formatted
        );
        Some(json!({
            "success": false,
            "error": "module_dependency_unsatisfied",
            "error_code": "module_dependency_unsatisfied",
            "instance_name": server_name,
            "module": module_name,
            "missing_dependencies": issues.iter().map(|(k, req, got)| json!({
                "component": k,
                "required": req,
                "installed": got,
            })).collect::<Vec<_>>(),
            "message": format!(
                "Module '{}' dependency check failed: {}",
                module_name, formatted.join(", ")
            )
        }))
    }

    /// 모듈 프로토콜을 인지한 포트 충돌 검사. 충돌 시 에러 응답(Value)을 반환한다.
    fn check_port_conflict(&self, instance: &crate::instance::ServerInstance, config: &Value, server_name: &str) -> Option<Value> {
        // GUI에서 portConflictCheck 비활성화 시 skip_port_check: true가 전달됨
        if config.get("skip_port_check").and_then(|v| v.as_bool()).unwrap_or(false) {
            return None;
        }
        let all_instances = self.instance_store.list();
        let running_ids: std::collections::HashSet<String> = all_instances
            .iter()
            .filter(|i| self.tracker.get_pid(&i.id).is_ok())
            .map(|i| i.id.clone())
            .collect();
        let module_protocols = self.build_module_protocols_map();
        let conflicts = crate::validator::check_port_conflicts(instance, all_instances, &running_ids, Some(&module_protocols));
        if conflicts.is_empty() {
            return None;
        }
        let details: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
        tracing::warn!("Port conflict detected for '{}': {:?}", server_name, details);
        Some(json!({
            "success": false,
            "error": "port_conflict",
            "error_code": "port_conflict",
            "message": format!("Cannot start '{}': port conflict detected", server_name),
            "conflicts": details,
        }))
    }

    /// 인스턴스 필수 익스텐션이 활성화+설치되어 있는지 검증한다. 인스턴스에 명시되지
    /// 않으면 모듈의 `install.requires_extensions`로 보충한다. 누락 시 에러 응답을 반환한다.
    async fn check_required_extensions(&self, instance: &crate::instance::ServerInstance, module_name: &str, server_name: &str) -> Option<Value> {
        let effective_required: Vec<String> = if instance.required_extensions.is_empty() {
            if let Ok(loaded_module) = self.module_loader.get_module(module_name) {
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
        if effective_required.is_empty() {
            return None;
        }
        let ext_mgr = self.extension_manager.as_ref()?;
        let ext_mgr_guard = ext_mgr.read().await;
        let ready = ext_mgr_guard.installed_and_enabled_set();
        let enabled_not_installed = ext_mgr_guard.enabled_but_not_installed();
        drop(ext_mgr_guard);
        // missing: 활성화되지 않았거나 디스크에 설치되지 않은 익스텐션
        let missing: Vec<String> = effective_required.iter()
            .filter(|ext_id| !ready.contains(ext_id.as_str()))
            .cloned()
            .collect();
        if missing.is_empty() {
            return None;
        }
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
        Some(json!({
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
        }))
    }

    /// `server.pre_start` 훅을 디스패치한다. 어떤 익스텐션이 요청을 처리(handled=true)하면
    /// 그 결과를 반환해 시작을 단락시키고, 성공 시 필요하면 로그 팔로워/IO 브리지를 등록한다.
    /// 아무도 처리하지 않으면 None을 반환해 데몬 직접 spawn으로 진행한다.
    /// `server.pre_start` 훅에 전달할 기본 컨텍스트(non-managed)를 만든다.
    fn pre_start_ctx(&self, instance: &crate::instance::ServerInstance, module_name: &str) -> Value {
        let instance_dir = self.instance_store.instance_dir(&instance.id);
        json!({
            "instance_id": instance.id.clone(),
            "instance_dir": instance_dir.to_string_lossy(),
            "module": module_name,
            "extension_data": &instance.extension_data,
        })
    }

    /// 인스턴스 필드를 모듈 config에 병합해 get_launch_command용 최종 config를 만든다.
    /// managed=true 시 모듈이 RCON 정책 등을 구분하도록 managed 플래그를 명시한다.
    fn build_launch_config(&self, instance: &crate::instance::ServerInstance, module_name: &str, config: &Value, managed: bool) -> Value {
        let mut cfg = config.as_object().cloned().unwrap_or_default();
        self.insert_resolved_exe(&mut cfg, instance, module_name);
        if let Some(port) = instance.port {
            cfg.insert("port".to_string(), json!(port));
        }
        if let Some(rcon_port) = instance.rcon_port {
            cfg.entry("rcon_port".to_string()).or_insert_with(|| json!(rcon_port));
        }
        if let Some(rcon_pw) = &instance.rcon_password {
            cfg.entry("rcon_password".to_string()).or_insert_with(|| json!(rcon_pw));
        }
        for (key, value) in &instance.module_settings {
            cfg.entry(key.clone()).or_insert_with(|| value.clone());
        }
        if let Some(rest_host) = &instance.rest_host {
            cfg.entry("rest_host".to_string()).or_insert_with(|| json!(rest_host));
        }
        if let Some(rest_port) = instance.rest_port {
            cfg.entry("rest_port".to_string()).or_insert_with(|| json!(rest_port));
        }
        if let Some(rest_user) = &instance.rest_username {
            cfg.entry("rest_username".to_string()).or_insert_with(|| json!(rest_user));
        }
        if let Some(rest_pw) = &instance.rest_password {
            cfg.entry("rest_password".to_string()).or_insert_with(|| json!(rest_pw));
        }
        cfg.entry("protocol_mode".to_string()).or_insert_with(|| json!(&instance.protocol_mode));
        if let Some(ver) = &instance.server_version {
            cfg.entry("server_version".to_string()).or_insert_with(|| json!(ver));
        }
        cfg.insert("managed".to_string(), json!(managed));
        Value::Object(cfg)
    }

    /// 이미 실행 중인 managed 프로세스가 있으면 중복 생성을 막고 그 상태를 반환한다.
    /// managed_store와 tracker 양쪽을 확인하며, 죽은 tracker 엔트리는 정리한다.
    async fn managed_already_running(&self, instance_id: &str, instance_name: &str) -> Option<Value> {
        if let Some(existing) = self.managed_store.get(instance_id).await {
            if existing.is_running() {
                tracing::info!(
                    "Managed process for '{}' already running (PID {}), skipping duplicate start",
                    instance_name, existing.pid
                );
                return Some(json!({
                    "success": true,
                    "server": instance_name,
                    "pid": existing.pid,
                    "managed": true,
                    "already_running": true,
                    "message": format!("Server '{}' is already running with PID {} (managed mode)", instance_name, existing.pid)
                }));
            }
        }
        // managed_store에 없더라도 tracker에 살아있는 PID가 있으면 중복 방지
        if let Ok(existing_pid) = self.tracker.get_pid(instance_id) {
            if crate::supervisor::process::is_process_alive(existing_pid) {
                tracing::warn!(
                    "Tracked process for '{}' still alive (PID {}) but not in managed store — preventing duplicate start",
                    instance_name, existing_pid
                );
                return Some(json!({
                    "success": true,
                    "server": instance_name,
                    "pid": existing_pid,
                    "managed": true,
                    "already_running": true,
                    "message": format!("Server '{}' has a tracked process (PID {}) still alive", instance_name, existing_pid)
                }));
            }
            tracing::info!("Cleaning stale tracker entry for '{}' (PID {} no longer alive)", instance_name, existing_pid);
            let _ = self.tracker.untrack(instance_id);
        }
        None
    }

    /// 모듈이 반환한 launch_result에서 REST/RCON 자격증명을 인스턴스로 동기화한다.
    /// rest_credentials(명시) 우선, 없으면 module.toml credential_map 기반. 변경 여부를 반환한다.
    fn sync_rest_credentials(instance: &mut crate::instance::ServerInstance, module: &LoadedModule, launch_result: &Value) -> bool {
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
        else if !module.metadata.credential_map.is_empty() {
            for (daemon_key, game_key) in &module.metadata.credential_map {
                let game_value = launch_result.get("module_settings")
                    .and_then(|ms| ms.get(game_key))
                    .and_then(|v| v.as_str())
                    .or_else(|| launch_result.get(game_key).and_then(|v| v.as_str()));
                let Some(val) = game_value else { continue };
                let target = match daemon_key.as_str() {
                    "rest_password" => &mut instance.rest_password,
                    "rest_username" => &mut instance.rest_username,
                    "rcon_password" => &mut instance.rcon_password,
                    _ => {
                        tracing::debug!("credential_map: unknown daemon key '{}', skipping", daemon_key);
                        continue;
                    }
                };
                if target.as_deref() != Some(val) {
                    *target = Some(val.to_string());
                    creds_changed = true;
                    tracing::info!("credential_map: synced {} -> {} for '{}'", game_key, daemon_key, instance.id);
                }
            }
        }
        creds_changed
    }

    async fn dispatch_pre_start_hook(&self, instance: &crate::instance::ServerInstance, module_name: &str, server_name: &str, ctx: Value) -> Option<Value> {
        let ext_mgr = self.extension_manager.as_ref()?;
        let results = ext_mgr.read().await.dispatch_hook("server.pre_start", ctx).await;

        for (ext_id, result) in &results {
            let Ok(val) = result else { continue };
            if val.get("handled").and_then(|h| h.as_bool()) != Some(true) {
                continue;
            }
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
                return Some(fail_val);
            }
            // 시작 성공 → log_follower 설정이 있으면 스트리머/IO 브리지 등록
            if let Some(log_cfg) = val.get("log_follower") {
                self.register_log_follower(instance, module_name, server_name, log_cfg).await;
            }
            return Some(val.clone());
        }
        None
    }

    /// 익스텐션이 반환한 `log_follower` 설정으로 로그 팔로워(read-only) 또는
    /// 대화형 IO 브리지를 spawn 하여 managed_store에 등록한다.
    async fn register_log_follower(&self, instance: &crate::instance::ServerInstance, module_name: &str, server_name: &str, log_cfg: &Value) {
        let instance_dir = self.instance_store.instance_dir(&instance.id);
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
        let log_pattern = self.module_loader.get_module(module_name)
            .ok()
            .and_then(|m| m.metadata.log_pattern.clone());

        // interactive=true → spawn() (양방향 stdin/stdout)
        // interactive=false → spawn_log_follower() (read-only stdout)
        let spawn_result = if interactive {
            let env_vars: Vec<(String, String)> = log_cfg.get("env_vars")
                .and_then(|e| e.as_object())
                .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default();
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
        match spawn_result {
            Ok(follower) => {
                self.managed_store.insert(&instance.id, follower).await;
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

    /// Non-managed 모드: 모듈에서 launch command를 받아 데몬이 detached로 spawn한다.
    /// (stdin/stdout 캡처 없음)
    async fn spawn_non_managed(&self, instance: &crate::instance::ServerInstance, module_name: &str, config: Value, server_name: &str) -> Result<Value> {
        tracing::info!("Starting non-managed server for instance '{}' (module: {})", server_name, module_name);

        // Build config — 인스턴스 필드를 병합 (non-managed)
        let final_config = self.build_launch_config(instance, module_name, &config, false);

        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let launch_result = crate::plugin::run_plugin(&module_path, "get_launch_command", final_config).await?;

        // action_required (e.g. server jar not found) → pass through to caller
        if launch_result.get("action_required").is_some() {
            tracing::warn!("Module requires user action: {:?}", launch_result.get("action_required"));
            return Ok(launch_result);
        }

        if launch_result.get("success").and_then(|s| s.as_bool()) != Some(true) {
            let msg = launch_result.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("{}", msg));
        }

        let program = launch_result.get("program")
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow::anyhow!("Module did not return program"))?;

        let args: Vec<String> = launch_result.get("args")
            .and_then(|a| a.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let working_dir = launch_result.get("working_dir")
            .and_then(|w| w.as_str())
            .unwrap_or(".");

        let env_vars: Vec<(String, String)> = launch_result.get("env_vars")
            .and_then(|e| e.as_object())
            .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();

        // Spawn detached process (no stdin/stdout capture)
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

        let child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", program, e))?;

        let pid = child.id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get PID of spawned process"))? ;

        // ⚠ 반드시 instance.id로 추적한다. 모니터(monitor_processes)·GUI 상태 조회·
        // check-update 등 모든 조회가 id 키를 사용하므로, 이름으로 추적하면
        // "시작했는데 stopped로 표시" · 죽은 launcher PID가 이름 키로 영구 잔존하는
        // 추적 불일치가 발생한다.
        self.tracker.track(&instance.id, pid)?;
        tracing::info!("Non-managed server '{}' started with PID {}", server_name, pid);

        Ok(json!({
            "success": true,
            "server": server_name,
            "pid": pid,
            "message": format!("Server '{}' started with PID {} (non-managed mode)", server_name, pid)
        }))
    }

    pub async fn start_server(&self, server_name: &str, module_name: &str, config: Value) -> Result<Value> {
        tracing::info!("Starting server '{}' with module '{}'", server_name, module_name);

        // Find instance to get executable_path and working_dir
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // 시작 전 검증: 모듈 의존성 → 포트 충돌 → 필수 익스텐션 순서로 확인하고,
        // 하나라도 실패하면 즉시 해당 에러 응답을 반환한다.
        if let Some(err) = self.check_module_dependencies(module_name, server_name).await {
            return Ok(err);
        }

        if let Some(err) = self.check_port_conflict(instance, &config, server_name) {
            return Ok(err);
        }

        if let Some(err) = self.check_required_extensions(instance, module_name, server_name).await {
            return Ok(err);
        }

        // Extension hook: server.pre_start — 익스텐션이 처리하면 그 결과로 단락
        let pre_start_ctx = self.pre_start_ctx(instance, module_name);
        if let Some(result) = self.dispatch_pre_start_hook(instance, module_name, server_name, pre_start_ctx).await {
            return Ok(result);
        }

        // ── Extension이 처리하지 않았으나 extension_data에 외부 프로세스 관리 플래그가 있는 경우 → 에러 ──
        if instance.ext_enabled("docker_enabled") {
            tracing::error!(
                "Instance '{}' has extension flags but no extension handled it. \
                 Is the required extension enabled?",
                server_name
            );
            return Ok(json!({
                "success": false,
                "action_required": "extension_required",
                "instance_name": server_name,
                "message": format!(
                    "Instance '{}' requires an extension to start, but no extension handled the request. \
                     Please check that the required extension is enabled.",
                    server_name
                )
            }));
        }

        // 익스텐션 플래그가 있으나 아무도 처리하지 않은 경우는 위 docker 체크에서 처리됨.
        // Non-managed 모드: 모듈에서 launch command를 받아 데몬이 detached로 spawn
        self.spawn_non_managed(instance, module_name, config, server_name).await
    }

    /// Stop a server by name
    /// Called by IPC API: POST /api/server/:name/stop
    pub async fn stop_server(&mut self, server_name: &str, module_name: &str, force: bool) -> Result<Value> {
        tracing::info!("Stopping server '{}' (force: {})", server_name, force);

        // Find instance
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // ── Extension hook: server.post_stop ──
        if let Some(ref ext_mgr) = self.extension_manager {
            let instance_dir = self.instance_store.instance_dir(&instance.id);
            let results = ext_mgr.read().await
                .dispatch_hook("server.post_stop", json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                })).await;

            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        self.stop_cooldowns.insert(instance.id.clone(), Instant::now());
                        return Ok(val.clone());
                    }
                }
            }
        }

        // ── Extension이 처리하지 않았으나 extension 플래그가 있는 경우 → 에러 ──
        if instance.ext_enabled("docker_enabled") {
            tracing::error!(
                "Instance '{}' has extension flags but no extension handled stop.",
                server_name
            );
            return Ok(json!({
                "success": false,
                "action_required": "extension_required",
                "instance_name": server_name,
                "message": format!(
                    "Instance '{}' requires an extension to stop, but no extension handled the request.",
                    server_name
                )
            }));
        }

        // ── Native 모드: 기존 정지 로직 ──

        // 모듈에서 stop_command를 가져옴 (없으면 "stop" 기본값)
        let stop_cmd = self.module_loader.get_module(module_name)
            .ok()
            .and_then(|m| m.metadata.stop_command.clone())
            .unwrap_or_else(|| "stop".to_string());

        // ── Managed mode: stdin에 stop_command 전송으로 graceful shutdown ──
        if let Some(managed) = self.managed_store.get(&instance.id).await {
            if managed.is_running() {
                // stdin 사용 가능 여부에 따라 분기
                // - stdin 있음 (정상 프로세스): graceful stop via stdin
                // - stdin 없음 (재연결 프로세스): 즉시 force kill
                //   (managed 모드에서는 RCON이 비활성이므로 force kill이 유일한 수단)
                let effective_force = force || !managed.is_stdin_available();

                if effective_force {
                    if !force && !managed.is_stdin_available() {
                        tracing::info!(
                            "stdin unavailable for '{}' (reattached process), force killing",
                            server_name
                        );
                    }
                    // Force kill: 즉시 종료
                    tracing::info!("Force-killing managed server '{}'", server_name);
                    if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                        let _ = process::force_kill_pid(pid);
                    }
                } else {
                    // Graceful: stdin에 stop_command 전송
                    tracing::info!("Sending '{}' to managed server '{}' via stdin", stop_cmd, server_name);
                    if let Err(e) = managed.send_command(&stop_cmd).await {
                        tracing::warn!("Failed to send stop command via stdin: {}", e);
                    }
                }

                // 프로세스 종료 대기 (force kill 시 5초, graceful 시 모듈 설정값 또는 기본 30초)
                let module_stop_timeout = self.module_loader.get_module(module_name)
                    .ok()
                    .and_then(|m| m.metadata.stop_timeout)
                    .unwrap_or(30);
                let timeout_secs = if effective_force { 5 } else { module_stop_timeout };
                let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
                loop {
                    if !managed.is_running() {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        tracing::warn!("Managed server '{}' did not exit in {}s, force killing", server_name, timeout_secs);
                        if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                            let _ = process::force_kill_pid(pid);
                        }
                        // 추가 대기
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }

                // managed store에서 제거
                self.managed_store.remove(&instance.id).await;

                // Clean up managed.pid + managed.meta.json
                remove_managed_pid_files(&self.instance_store.instance_dir(&instance.id));

                tracing::info!("Managed server '{}' stopped", server_name);
                // auto-detect 쿨다운 등록
                self.stop_cooldowns.insert(instance.id.clone(), Instant::now());
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
                self.managed_store.remove(&instance.id).await;
                remove_managed_pid_files(&self.instance_store.instance_dir(&instance.id));
                self.stop_cooldowns.insert(instance.id.clone(), Instant::now());

                // tracker에서도 제거
                let _ = self.tracker.untrack(&instance.id);

                return Ok(json!({
                    "success": true,
                    "server": server_name,
                    "message": format!("Server '{}' already stopped", server_name)
                }));
            }
        }

        // ── Non-managed mode: Python lifecycle.py를 통한 정지 ──
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Build config with all necessary info for stop
        let mut config_obj = serde_json::Map::new();
        self.insert_resolved_exe(&mut config_obj, instance, module_name);
        // Pass PID from tracker so the module can kill the actual process
        if let Ok(pid) = self.tracker.get_pid(&instance.id).or_else(|_| self.tracker.get_pid(server_name)) {
            config_obj.insert("pid".to_string(), json!(pid));
        }
        // Pass RCON port for graceful shutdown (password is read from game config by Python)
        if let Some(rcon_port) = instance.rcon_port {
            config_obj.insert("rcon_port".to_string(), json!(rcon_port));
        }
        config_obj.insert("force".to_string(), json!(force));
        let config = Value::Object(config_obj);

        // Execute stop function
        let result = crate::plugin::run_plugin(&module_path, "stop", config).await?;

        // Check if the module actually succeeded
        let plugin_success = result.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
        let plugin_message = result.get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        if plugin_success {
            tracing::info!("Server '{}' stopped successfully: {}", server_name, plugin_message);
            // auto-detect 쿨다운 등록
            self.stop_cooldowns.insert(instance.id.clone(), Instant::now());
            Ok(json!({
                "success": true,
                "server": server_name,
                "message": format!("Server '{}' stopped", server_name)
            }))
        } else {
            tracing::error!("Failed to stop server '{}': {}", server_name, plugin_message);
            Err(anyhow::anyhow!("Failed to stop server '{}': {}", server_name, plugin_message))
        }
    }

    /// Get server status by name
    /// Called by IPC API: GET /api/server/:name/status
    pub async fn get_server_status(&self, server_name: &str, module_name: &str) -> Result<Value> {
        tracing::debug!("Getting status for server '{}'", server_name);

        // Find instance to get executable_path
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // ── Extension hook: server.status ──
        if let Some(ref ext_mgr) = self.extension_manager {
            let instance_dir = self.instance_store.instance_dir(&instance.id);
            let process_patterns = self.module_loader.get_module(module_name)
                .map(|m| m.metadata.process_patterns.clone())
                .unwrap_or_default();
            let results = ext_mgr.read().await
                .dispatch_hook("server.status", json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                    "process_patterns": process_patterns,
                })).await;

            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        return Ok(val.clone());
                    }
                }
            }
        }

        // ── Extension 플래그가 있으나 처리되지 않은 경우 → 최소 응답 ──
        if instance.ext_enabled("docker_enabled") {
            return Ok(json!({
                "server": server_name,
                "status": "stopped",
                "online": false,
                "message": "Required extension is not enabled"
            }));
        }

        // Managed 프로세스가 있으면 plugin 호출 없이 직접 판단
        if let Some(managed) = self.managed_store.get(&instance.id).await {
            let running = managed.is_running();
            let pid = self.tracker.get_pid(&instance.id).ok();
            let start_time = if running { self.tracker.get_start_time(&instance.id).ok() } else { None };
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
        let tracked_pid = self.tracker.get_pid(&instance.id)
            .or_else(|_| self.tracker.get_pid(server_name))
            .ok();

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Build config — 실행 파일, PID, RCON 정보 등 Python status()에 필요한 값 포함
        let mut config_obj = serde_json::Map::new();
        self.insert_resolved_exe(&mut config_obj, instance, module_name);
        if let Some(pid) = tracked_pid {
            config_obj.insert("pid".to_string(), json!(pid));
        }
        if let Some(rcon_port) = instance.rcon_port {
            config_obj.insert("rcon_port".to_string(), json!(rcon_port));
        }
        if let Some(rcon_pw) = &instance.rcon_password {
            config_obj.insert("rcon_password".to_string(), json!(rcon_pw));
        }
        // module_settings 포함 (server_name 등 Python이 INI 경로 해석에 사용)
        for (key, value) in &instance.module_settings {
            config_obj.entry(key.clone()).or_insert_with(|| value.clone());
        }
        let config = Value::Object(config_obj);

        // Ask module for status
        match crate::plugin::run_plugin(&module_path, "status", config).await {
            Ok(result) => {
                let status = result.get("status").and_then(|s| s.as_str()).unwrap_or("unknown");
                let pid = result.get("pid").and_then(|p| p.as_u64());
                let start_time = pid.and_then(|_| self.tracker.get_start_time(&instance.id).ok());
                Ok(json!({
                    "server": server_name,
                    "status": status,
                    "pid": pid,
                    "start_time": start_time,
                }))
            },
            Err(e) => {
                tracing::error!("Failed to get status: {}", e);
                Ok(json!({
                    "server": server_name,
                    "status": "unknown",
                    "pid": null,
                }))
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

    /// 서버에 명령어 실행 — 사람이 읽을 메시지만 반환한다.
    ///
    /// 모듈이 돌려준 구조화 데이터(`data` 필드 등)가 필요하면
    /// [`Self::execute_command_value`]를 사용한다.
    pub async fn execute_command(
        &self,
        instance_id: &str,
        module_name: &str,
        command: &str,
        args: Value,
    ) -> Result<String> {
        let result = self
            .execute_command_value(instance_id, module_name, command, args)
            .await?;
        let message = result
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Command executed successfully");
        Ok(message.to_string())
    }

    /// 서버에 명령어 실행 — 모듈이 돌려준 응답 전체(`message`/`data` 등)를 반환한다.
    ///
    /// 클라우드 디스패처가 `data`(플레이어 목록 등)를 포매팅해 디스코드에
    /// 표시할 수 있도록 원본 Value를 그대로 넘긴다. `success: false`는 Err.
    pub async fn execute_command_value(
        &self,
        instance_id: &str,
        module_name: &str,
        command: &str,
        args: Value,
    ) -> Result<Value> {
        tracing::info!(
            "Supervisor executing command '{}' for instance '{}' (module: {})",
            command,
            instance_id,
            module_name
        );

        // 인스턴스 정보 얻기
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        // ── Extension 관리 인스턴스에서 명령어 직접 실행은 미지원 → 에러 ──
        if instance.ext_enabled("docker_enabled") {
            return Err(anyhow::anyhow!(
                "Instance '{}' is managed by an extension. Direct command execution is not supported.",
                instance.name
            ));
        }

        // 모듈 찾기
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // 모듈의 기본값 가져오기 (하드코딩 대신 모듈 설정 사용)
        let default_rcon_port = module.metadata.default_rcon_port();
        let default_rest_port = module.metadata.default_rest_port();
        let default_rest_host = module.metadata.default_rest_host();

        // 명령어 config 구성 (RCON 설정 포함)
        let pid = self.tracker.get_pid(&instance.id).ok();
        let instance_dir = self.instance_store.instance_dir(&instance.id);
        let working_dir = instance.working_dir.clone()
            .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());
        let config = json!({
            "command": command,
            "args": args,
            "protocol_mode": &instance.protocol_mode,  // "rest" 또는 "rcon"
            "working_dir": &working_dir,
            "rcon_host": "127.0.0.1",
            "rcon_port": instance.rcon_port.unwrap_or(default_rcon_port),
            "rest_host": instance.rest_host.clone().unwrap_or(default_rest_host),
            "rest_port": instance.rest_port.unwrap_or(default_rest_port),
            "pid": pid,
            "instance_id": instance_id,
        });

        // 플러그인 실행
        let result = crate::plugin::run_plugin(&module_path, "command", config).await?;

        // 결과 처리
        if let Some(success) = result.get("success").and_then(|s| s.as_bool()) {
            if success {
                Ok(result)
            } else {
                let error_msg = result
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Command failed");
                Err(anyhow::anyhow!("{}", error_msg))
            }
        } else {
            Err(anyhow::anyhow!("Invalid response from module"))
        }
    }

    // ─── Managed Process Methods ─────────────────────────────

    /// Start a server as a managed process with full stdio capture.
    /// Uses the module's `get_launch_command` to build the command, then spawns it natively.
    /// Extension 관리 모드에서는 extension hook을 통해 위임합니다.
    pub async fn start_managed_server(
        &mut self,
        instance_id: &str,
        module_name: &str,
        config: Value,
    ) -> Result<Value> {
        let mut instance = self.instance_store.get(instance_id).cloned()
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        // 중복 프로세스 방어: 이미 실행 중이면 그 상태를 그대로 반환
        if let Some(resp) = self.managed_already_running(instance_id, &instance.name).await {
            return Ok(resp);
        }

        // 포트 충돌 검사 (모듈별 프로토콜 인지)
        if let Some(err) = self.check_port_conflict(&instance, &config, &instance.name) {
            return Ok(err);
        }

        // Extension hook: server.pre_start (managed) — 익스텐션이 처리하면 그 결과로 단락
        let pre_start_ctx = json!({
            "instance_id": instance_id,
            "instance_dir": self.instance_store.instance_dir(&instance.id).to_string_lossy(),
            "module": module_name,
            "instance": serde_json::to_value(&instance).unwrap_or_default(),
            "config": &config,
            "managed": true,
            "extension_data": &instance.extension_data,
        });
        if let Some(result) = self.dispatch_pre_start_hook(&instance, module_name, &instance.name, pre_start_ctx).await {
            return Ok(result);
        }

        // ── Extension이 처리하지 않았으나 extension 플래그가 있는 경우 → 에러 ──
        if instance.ext_enabled("docker_enabled") {
            tracing::error!(
                "Managed instance '{}' has extension flags but no extension handled it.",
                instance.name
            );
            return Ok(json!({
                "success": false,
                "action_required": "extension_required",
                "instance_name": instance.name,
                "message": format!(
                    "Instance '{}' requires an extension to start, but no extension handled the request. \
                     Please check that the required extension is enabled.",
                    instance.name
                )
            }));
        }

        tracing::info!("Starting managed server for instance '{}' (module: {})", instance.name, module_name);

        // Build config — 인스턴스 필드를 병합 (managed)
        let final_config = self.build_launch_config(&instance, module_name, &config, true);

        // Get module and call get_launch_command
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        tracing::info!("get_launch_command config: {}", serde_json::to_string_pretty(&final_config).unwrap_or_default());

        let launch_result = crate::plugin::run_plugin(&module_path, "get_launch_command", final_config).await?;

        // If the module requires user action (e.g. server jar not found), pass through to GUI
        if launch_result.get("action_required").is_some() {
            tracing::warn!("Module requires user action: {:?}", launch_result.get("action_required"));
            return Ok(launch_result);
        }

        if launch_result.get("success").and_then(|s| s.as_bool()) != Some(true) {
            let msg = launch_result.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("{}", msg));
        }

        let program = launch_result.get("program")
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow::anyhow!("Module did not return program"))?;

        let args: Vec<String> = launch_result.get("args")
            .and_then(|a| a.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let working_dir = launch_result.get("working_dir")
            .and_then(|w| w.as_str())
            .unwrap_or(".");

        let env_vars: Vec<(String, String)> = launch_result.get("env_vars")
            .and_then(|e| e.as_object())
            .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();

        // Sync REST/RCON credentials from module (rest_credentials 우선, 없으면 credential_map)
        if Self::sync_rest_credentials(&mut instance, &module, &launch_result) {
            if let Err(e) = self.instance_store.update(&instance.id, instance.clone()) {
                tracing::warn!("Failed to persist synced credentials: {}", e);
            }
        }

        // Spawn managed process
        let log_pattern = module.metadata.log_pattern.as_deref();
        let inst_dir = self.instance_store.instance_dir(&instance.id);
        let managed = ManagedProcess::spawn(program, &args, working_dir, env_vars, log_pattern, Some(&inst_dir)).await?;
        let pid = managed.pid;

        // Track the process
        self.tracker.track(&instance.id, pid)?;
        self.managed_store.insert(&instance.id, managed).await;

        // Persist PID + program identity to disk for orphan reattachment on daemon restart
        persist_managed_pid(&inst_dir, pid, program, &instance.name);

        tracing::info!("Managed server '{}' started with PID {}", instance.name, pid);
        Ok(json!({
            "success": true,
            "server": instance.name,
            "pid": pid,
            "managed": true,
            "message": format!("Server '{}' started with PID {} (managed mode)", instance.name, pid)
        }))
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
                "running": proc.is_running(),
                "stdin_available": proc.is_stdin_available(),
            }));
        }

        // Fallback: read from disk log
        let inst_dir = self.instance_store.instance_dir(instance_id);
        let log_path = managed_process::console_log_path(&inst_dir);
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

        Err(anyhow::anyhow!("No managed process or console log for instance '{}'", instance_id))
    }

    /// Run validation on an instance (via module's validate function)
    pub async fn validate_instance(&self, instance_id: &str) -> Result<Value> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.module_loader.get_module(&instance.module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let mut cfg = serde_json::Map::new();
        if let Some(exe_path) = &instance.executable_path {
            cfg.insert("server_jar".to_string(), json!(exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            cfg.insert("working_dir".to_string(), json!(work_dir));
        }
        if let Some(port) = instance.port {
            cfg.insert("port".to_string(), json!(port));
        }
        // Extension 데이터 전달 (Python lifecycle 스크립트에서 참조)
        cfg.insert("extension_data".to_string(), json!(&instance.extension_data));

        let result = crate::plugin::run_plugin(&module_path, "validate", Value::Object(cfg)).await?;
        Ok(result)
    }

    /// Read or update server.properties (via module's configure/read_properties)
    pub async fn manage_properties(
        &self,
        instance_id: &str,
        action: &str,  // "read" or "write"
        settings: Option<Value>,
    ) -> Result<Value> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.module_loader.get_module(&instance.module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

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

        let function = match action {
            "write" | "configure" => {
                if let Some(s) = settings {
                    cfg.insert("settings".to_string(), s);
                }
                "configure"
            }
            "reset" => "reset_properties",
            "reset_server" => "reset_server",
            // 실제 설정 파일을 다시 읽어 saba-chan 키 포맷으로 변환 (표시값 새로고침용).
            // module_settings 캐시가 아니라 디스크의 실제 값을 반환한다.
            "import" => "import_settings",
            _ => "read_properties",
        };

        let result = crate::plugin::run_plugin(&module_path, function, Value::Object(cfg)).await?;
        Ok(result)
    }

    /// Accept EULA for an instance
    pub async fn accept_eula(&self, instance_id: &str) -> Result<Value> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.module_loader.get_module(&instance.module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let mut cfg = serde_json::Map::new();
        if let Some(work_dir) = &instance.working_dir {
            cfg.insert("working_dir".to_string(), json!(work_dir));
        }

        let result = crate::plugin::run_plugin(&module_path, "accept_eula", Value::Object(cfg)).await?;
        Ok(result)
    }

    /// Diagnose errors from instance logs
    pub async fn diagnose_instance(&self, instance_id: &str) -> Result<Value> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.module_loader.get_module(&instance.module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let mut cfg = serde_json::Map::new();
        if let Some(work_dir) = &instance.working_dir {
            cfg.insert("working_dir".to_string(), json!(work_dir));
        }

        // If managed, provide recent console output for diagnosis
        if let Some(proc) = self.managed_store.get(&instance.id).await {
            let recent = proc.get_recent_console(500).await;
            let log_lines: Vec<String> = recent.iter().map(|l| l.content.clone()).collect();
            cfg.insert("log_lines".to_string(), json!(log_lines));
        }

        let result = crate::plugin::run_plugin(&module_path, "diagnose_log", Value::Object(cfg)).await?;
        Ok(result)
    }

    // ─── Server Installation Methods ─────────────────────────

    /// List available server versions (delegates to module lifecycle)
    pub async fn list_versions(
        &self,
        module_name: &str,
        include_snapshots: bool,
        page: u32,
        per_page: u32,
    ) -> Result<Value> {
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let config = json!({
            "include_snapshots": include_snapshots,
            "page": page,
            "per_page": per_page,
        });

        let result = crate::plugin::run_plugin(&module_path, "list_versions", config).await?;
        Ok(result)
    }

    /// Get detailed info for a specific version
    pub async fn get_version_details(
        &self,
        module_name: &str,
        version: &str,
    ) -> Result<Value> {
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let config = json!({ "version": version });
        let result = crate::plugin::run_plugin(&module_path, "get_version_details", config).await?;
        Ok(result)
    }

    /// Install a server: download binary, setup directory, optional initial settings.
    /// 기존 실행 파일이 존재하면 `.bak`으로 백업하고, 실패 시 복원합니다.
    pub async fn install_server(
        &self,
        module_name: &str,
        version: &str,
        install_dir: &str,
        jar_name: Option<&str>,
        accept_eula: bool,
        initial_settings: Option<Value>,
    ) -> Result<Value> {
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // jar_name 결정: 명시 > 모듈 metadata.server_executable > 기본값 없음
        let effective_jar = jar_name
            .map(|s| s.to_string())
            .or_else(|| {
                module.metadata.server_executable.clone()
            });

        // ── 기존 바이너리 백업 ──
        let backup_path = if let Some(ref jar) = effective_jar {
            let original = std::path::Path::new(install_dir).join(jar);
            if original.is_file() {
                let backup = original.with_extension(format!(
                    "{}.bak",
                    original.extension().unwrap_or_default().to_string_lossy()
                ));
                if let Err(e) = std::fs::copy(&original, &backup) {
                    tracing::warn!("Failed to backup '{}': {}", original.display(), e);
                    None
                } else {
                    tracing::info!("Backed up existing binary: {} → {}", original.display(), backup.display());
                    Some(backup)
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut config = json!({
            "version": version,
            "install_dir": install_dir,
            "accept_eula": accept_eula,
        });

        if let Some(ref name) = effective_jar {
            config["jar_name"] = json!(name);
        }
        if let Some(settings) = initial_settings {
            config["initial_settings"] = settings;
        }

        let result = crate::plugin::run_plugin(&module_path, "install_server", config).await;

        match result {
            Ok(ref val) => {
                let success = val.get("success").and_then(|s| s.as_bool()) == Some(true);
                if success {
                    // 설치 성공 → 백업 파일 정리
                    if let Some(ref bp) = backup_path {
                        let _ = std::fs::remove_file(bp);
                        tracing::info!("Removed backup after successful install: {}", bp.display());
                    }
                } else {
                    // 모듈이 success=false 반환 → 복원
                    if let Some(ref bp) = backup_path {
                        if let Some(ref jar) = effective_jar {
                            let original = std::path::Path::new(install_dir).join(jar);
                            if let Err(e) = std::fs::copy(bp, &original) {
                                tracing::error!("Failed to restore backup '{}': {}", bp.display(), e);
                            } else {
                                tracing::info!("Restored backup after failed install: {}", original.display());
                            }
                        }
                        let _ = std::fs::remove_file(bp);
                    }
                }
                result
            }
            Err(e) => {
                // 플러그인 에러 → 복원
                if let Some(ref bp) = backup_path {
                    if let Some(ref jar) = effective_jar {
                        let original = std::path::Path::new(install_dir).join(jar);
                        if let Err(re) = std::fs::copy(bp, &original) {
                            tracing::error!("Failed to restore backup '{}': {}", bp.display(), re);
                        } else {
                            tracing::info!("Restored backup after plugin error: {}", original.display());
                        }
                    }
                    let _ = std::fs::remove_file(bp);
                }
                Err(e)
            }
        }
    }

    /// Detect the installed server version by inspecting the server binary (e.g. parsing server.jar).
    /// Delegates to the module's `get_installed_version` lifecycle function.
    pub async fn get_installed_version(
        &self,
        module_name: &str,
        instance_id: &str,
    ) -> Result<Value> {
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.id == instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", instance_id))?;

        let install_dir = instance.module_settings
            .get("working_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or(instance.working_dir.as_deref())
            .unwrap_or("");
        let jar_name = instance.module_settings
            .get("server_jar")
            .and_then(|v| v.as_str())
            .unwrap_or("server.jar");

        let config = json!({
            "install_dir": install_dir,
            "jar_name": jar_name,
        });

        let result = crate::plugin::run_plugin(&module_path, "get_installed_version", config).await?;
        Ok(result)
    }

    /// server_version이 비어있고 working_dir이 설정된 인스턴스에 대해
    /// 모듈의 get_installed_version을 호출하여 버전을 감지·저장합니다.
    /// 데몬 초기화 시 한 번 호출하여 기존 서버의 버전을 보충합니다.
    pub async fn backfill_server_versions(&mut self) {
        // 감지 대상 수집 (borrow 분리를 위해 먼저 복사)
        let targets: Vec<(String, String, String)> = self.instance_store.list().iter()
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
            match self.get_installed_version(&module_name, &id).await {
                Ok(result) => {
                    if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                        if let Some(version) = result.get("version").and_then(|v| v.as_str()) {
                            if let Some(mut instance) = self.instance_store.get(&id).cloned() {
                                instance.server_version = Some(version.to_string());
                                if let Err(e) = self.instance_store.update(&id, instance) {
                                    tracing::warn!("Failed to save backfilled version for '{}': {}", name, e);
                                } else {
                                    tracing::info!("Backfilled server version '{}' for instance '{}'", version, name);
                                }
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
    pub async fn monitor_processes(&mut self, process_snapshot: &[crate::process_monitor::RunningProcess]) -> Result<()> {
        use crate::process_monitor;

        // Clean up dead managed processes and their PID files
        let dead_ids = self.managed_store.cleanup_dead().await;
        for dead_id in &dead_ids {
            remove_managed_pid_files(&self.instance_store.instance_dir(dead_id));
            // 죽은 managed process에 대해 stop cooldown 등록 → auto_detect 재탐색 억제
            // 이전 세션에서 실행되어 reattach된 프로세스가 종료된 경우,
            // cooldown 없이 auto_detect가 즉시 동명 프로세스를 재추적하는 것을 방지
            self.stop_cooldowns.insert(dead_id.clone(), Instant::now());
        }
        
        let instances = self.instance_store.list().to_vec();
        let mut tracked_count = 0;
        let mut auto_detected_count = 0;

        // 같은 process_name을 공유하는 auto-detect 인스턴스 수 — 경로 매칭 실패 시
        // 이름만으로 첫 후보에 폴백해도 안전한지(유일 인스턴스인지) 판단하는 데 쓴다.
        // 마이그레이션 등으로 동일 게임 인스턴스가 여러 개일 때, 남의 프로세스를
        // 임의로 추적해 포트 충돌 강제 종료까지 이어지는 오동작을 방지한다.
        let mut process_name_counts: HashMap<String, usize> = HashMap::new();
        for inst in &instances {
            if inst.auto_detect {
                if let Some(pn) = &inst.process_name {
                    *process_name_counts.entry(pn.to_lowercase()).or_insert(0) += 1;
                }
            }
        }

        // 기록된 추적 경로(tracked_process_path)의 인스턴스 간 공유 여부 —
        // java.exe처럼 여러 인스턴스가 같은 바이너리를 공유하면 경로가 인스턴스를
        // 구분하지 못하므로 정확 경로 매칭에 사용하지 않는다 (오추적 방지)
        let mut recorded_path_counts: HashMap<String, usize> = HashMap::new();
        for inst in &instances {
            if inst.auto_detect {
                if let Some(p) = inst.tracked_process_path.as_deref().filter(|s| !s.is_empty()) {
                    *recorded_path_counts.entry(normalize_path_for_match(p)).or_insert(0) += 1;
                }
            }
        }

        // 이미 추적 중인 PID 집합 — 다른 인스턴스가 추적 중인 프로세스를 auto-detect가
        // 중복으로 잡아 여러 인스턴스가 동시에 '실행 중'으로 표시되는 것을 방지
        let mut tracked_pids: std::collections::HashSet<u32> = instances
            .iter()
            .filter_map(|i| self.tracker.get_pid(&i.id).ok())
            .collect();

        for instance in instances {
            // 이미 추적 중이면 상태만 확인 (snapshot 기반 — 시스템 콜 없음)
            if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                tracked_count += 1;

                if !process_monitor::is_running_in(process_snapshot, pid) {
                    // 방금 추적을 시작한 프로세스는 판정을 보류한다 — 이 snapshot이
                    // track() 호출보다 먼저 스캔되었을 가능성이 있다 (레이스).
                    // 다음 사이클은 반드시 그 이후 시점의 스냅샷을 사용하므로 안전하게 재평가된다.
                    let recently_tracked = self.tracker
                        .seconds_since_tracked(&instance.id)
                        .map(|secs| secs < TRACK_GRACE_SECS)
                        .unwrap_or(false);

                    if recently_tracked {
                        tracing::debug!(
                            "Process {} for instance '{}' not in snapshot yet, but tracked <{}s ago — deferring untrack",
                            pid, instance.name, TRACK_GRACE_SECS
                        );
                    } else {
                        tracing::warn!("Process {} for instance '{}' is no longer running, removing from tracker", pid, instance.name);
                        // tracker에서 제거하여 다음 사이클에서 다시 감지할 수 있도록 함
                        if let Err(e) = self.tracker.untrack(&instance.id) {
                            tracing::error!("Failed to untrack process: {}", e);
                        }
                        tracked_pids.remove(&pid);
                    }
                } else if instance.tracked_process_path.is_none() {
                    // 시작 직후 최초 1회: 추적 중인 프로세스의 exe 경로를 기록해 둔다.
                    // (이후 auto-detect가 다중 인스턴스를 경로로 구분하는 데 사용)
                    if let Some(proc_info) = process_snapshot.iter().find(|p| p.pid == pid) {
                        if let Some(exe) = proc_info.executable_path.clone() {
                            self.record_tracked_path(&instance.id, exe);
                        }
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
                    // 다른 인스턴스가 이미 추적 중인 PID는 후보에서 제외
                    let processes: Vec<&crate::process_monitor::RunningProcess> = processes
                        .into_iter()
                        .filter(|p| !tracked_pids.contains(&p.pid))
                        .collect();

                    let name_is_unique = process_name_counts
                        .get(&process_name.to_lowercase())
                        .copied()
                        .unwrap_or(0)
                        <= 1;

                    let recorded_path_is_unique = instance
                        .tracked_process_path
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|p| {
                            recorded_path_counts
                                .get(&normalize_path_for_match(p))
                                .copied()
                                .unwrap_or(0)
                                <= 1
                        })
                        .unwrap_or(true);

                    if let Some(process) = pick_detected_process(&processes, &instance, name_is_unique, recorded_path_is_unique) {
                                auto_detected_count += 1;
                                tracing::info!(
                                    "Auto-detected process '{}' (PID: {}) for instance '{}'",
                                    process_name, process.pid, instance.name
                                );
                                if let Err(e) = self.tracker.track(&instance.id, process.pid) {
                                    tracing::error!("Failed to track process: {}", e);
                                }
                                tracked_pids.insert(process.pid);
                                // 실제 감지된 exe 경로를 기록 — 다음 감지부터 정확 경로 매칭
                                if let Some(exe) = process.executable_path.clone() {
                                    self.record_tracked_path(&instance.id, exe);
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

                                    // 프로세스 강제 종료 (spawn_blocking — sysinfo는 동기 시스템 콜)
                                    let kill_pid = process.pid;
                                    let kill_name = instance.name.clone();
                                    tokio::task::spawn_blocking(move || {
                                        let mut sys = sysinfo::System::new();
                                        sys.refresh_processes();
                                        if let Some(proc) = sys.process(sysinfo::Pid::from_u32(kill_pid)) {
                                            proc.kill();
                                            tracing::info!("Force-killed PID {} for instance '{}'", kill_pid, kill_name);
                                        }
                                    }).await.ok();

                                    // tracker에서 제거 + 쿨다운 설정
                                    let _ = self.tracker.untrack(&instance.id);
                                    self.stop_cooldowns.insert(instance.id.clone(), Instant::now());

                                    // 이벤트 기록 (GUI 폴링에서 읽어감) — 충돌마다 개별 이벤트 생성
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0);
                                    if let Ok(mut events) = self.port_conflict_stops.lock() {
                                        for conflict in &conflicts {
                                            events.push(PortConflictStopEvent {
                                                stopped_name: instance.name.clone(),
                                                stopped_id: instance.id.clone(),
                                                existing_name: conflict.conflicting_instance_name.clone(),
                                                port: conflict.port,
                                                timestamp_ms: ts,
                                            });
                                        }
                                    }
                                } // if !skip_port_check
                                }
                            }
                }
            }
        }
        
        tracing::debug!("Monitor cycle: {} tracked, {} auto-detected", tracked_count, auto_detected_count);
        Ok(())
    }

    /// 추적에 성공한 프로세스의 exe 경로를 인스턴스에 기록한다.
    /// auto-detect가 같은 process_name의 다중 인스턴스를 경로로 구분하는 데 사용된다.
    fn record_tracked_path(&mut self, instance_id: &str, exe_path: String) {
        let Some(inst) = self.instance_store.get(instance_id) else { return };
        if inst.tracked_process_path.as_deref() == Some(exe_path.as_str()) {
            return; // 변경 없음 — 불필요한 디스크 쓰기 방지
        }
        let mut updated = inst.clone();
        tracing::debug!(
            "Recording tracked process path for '{}': {}",
            updated.name, exe_path
        );
        updated.tracked_process_path = Some(exe_path);
        if let Err(e) = self.instance_store.update(instance_id, updated) {
            tracing::warn!("Failed to record tracked process path for '{}': {}", instance_id, e);
        }
    }

    /// 포트 충돌 강제 정지 이벤트를 drain (GUI 폴링에서 한 번 읽으면 비움)
    pub fn drain_port_conflict_stops(&self) -> Vec<PortConflictStopEvent> {
        self.port_conflict_stops
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }
}

/// 경로 비교용 정규화 — 구분자 통일 + 소문자화 (Windows 대소문자 무시 경로 대응)
fn normalize_path_for_match(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

// ── Managed PID 영속화 (데몬 재시작 시 재연결용) ─────────────────

/// managed.pid와 함께 기록되는 프로세스 신원 메타 파일명
const MANAGED_META_FILENAME: &str = "managed.meta.json";

/// managed 프로세스의 신원 정보 — 재연결 시 PID 재사용을 감지하는 데 사용
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ManagedPidMeta {
    pid: u32,
    /// spawn 시 실행한 프로그램 경로 (재연결 시 실제 프로세스 exe와 대조)
    program: String,
}

/// managed.pid(+ 신원 메타)를 인스턴스 디렉토리에 기록한다.
fn persist_managed_pid(inst_dir: &std::path::Path, pid: u32, program: &str, instance_name: &str) {
    if let Err(e) = std::fs::write(inst_dir.join("managed.pid"), pid.to_string()) {
        tracing::warn!("Failed to write managed.pid for '{}': {}", instance_name, e);
    }
    let meta = ManagedPidMeta { pid, program: program.to_string() };
    match serde_json::to_string(&meta) {
        Ok(s) => {
            if let Err(e) = std::fs::write(inst_dir.join(MANAGED_META_FILENAME), s) {
                tracing::warn!("Failed to write {} for '{}': {}", MANAGED_META_FILENAME, instance_name, e);
            }
        }
        Err(e) => tracing::warn!("Failed to serialize managed meta for '{}': {}", instance_name, e),
    }
}

/// managed.pid와 신원 메타를 함께 삭제한다.
fn remove_managed_pid_files(inst_dir: &std::path::Path) {
    let _ = std::fs::remove_file(inst_dir.join("managed.pid"));
    let _ = std::fs::remove_file(inst_dir.join(MANAGED_META_FILENAME));
}

/// 경로에서 바이너리 이름만 추출해 정규화한다 (소문자, ".exe" 제거).
/// "java" vs "C:\...\java.exe" 처럼 표기가 달라도 같은 바이너리로 비교하기 위함.
fn normalize_binary_name(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| path.to_lowercase());
    name.strip_suffix(".exe").map(str::to_string).unwrap_or(name)
}

/// 재연결 대상 PID가 spawn 당시 기록된 프로그램과 같은 바이너리인지 검증한다.
///
/// - 메타 파일이 없거나 PID가 메타와 다르면(구버전 기록·부분 기록) 검증 불가 →
///   기존 동작대로 재연결을 허용한다 (레거시 호환).
/// - 메타가 있으면 스냅샷에서 해당 PID의 exe(없으면 프로세스 이름)와 바이너리
///   이름을 대조하고, 스냅샷에서 찾을 수 없으면 신원 확인 불가로 재연결을 포기한다.
fn reattach_identity_matches(
    inst_dir: &std::path::Path,
    pid: u32,
    snapshot: &[crate::process_monitor::RunningProcess],
) -> bool {
    let meta: ManagedPidMeta = match std::fs::read_to_string(inst_dir.join(MANAGED_META_FILENAME))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(m) => m,
        None => return true, // 레거시 pid 파일 — 검증 불가, 기존 동작 유지
    };
    if meta.pid != pid {
        tracing::debug!(
            "managed meta pid {} does not match managed.pid {} — skipping identity check",
            meta.pid, pid
        );
        return true; // 메타가 다른 세대의 기록 — 검증 불가, 레거시와 동일 취급
    }
    match snapshot.iter().find(|p| p.pid == pid) {
        Some(proc_info) => {
            let expected = normalize_binary_name(&meta.program);
            let actual = proc_info
                .executable_path
                .as_deref()
                .map(normalize_binary_name)
                .unwrap_or_else(|| normalize_binary_name(&proc_info.name));
            actual == expected
        }
        // alive인데 스냅샷에 없음 — 신원 확인 불가. 오인 강제종료 위험을 피하기 위해
        // 재연결하지 않는다 (auto-detect가 가능한 인스턴스는 monitor가 다시 잡는다).
        None => false,
    }
}

/// auto-detect 후보 중 이 인스턴스에 속하는 프로세스를 고른다.
///
/// 매칭 우선순위:
/// 1. `tracked_process_path`(마지막으로 추적에 성공한 실제 exe 경로)와 정확 일치.
///    같은 process_name의 다중 인스턴스도 각자 자기 프로세스를 구분할 수 있다.
///    단, java.exe처럼 인스턴스 간 공유되는 바이너리는 경로가 겹치므로
///    `recorded_path_is_unique=false`일 때는 이 단계(와 그 parent 루트)를 건너뛴다.
/// 2. 경로 루트 매칭: `working_dir`/`executable_path` parent(+ 기록 경로의 parent)
///    하위에서 실행 중인 후보 (마이그레이션 서버, 런처→자식 프로세스 케이스 포함).
/// 3. 경로 매칭 실패 시:
///    - 같은 process_name을 쓰는 인스턴스가 이 하나뿐일 때만 첫 후보로 폴백
///      (기존 동작 유지 — working_dir이 부정확해도 추적이 끊기지 않도록)
///    - 동일 이름 인스턴스가 여러 개면 None (남의 프로세스 오추적 방지)
fn pick_detected_process<'a>(
    candidates: &[&'a crate::process_monitor::RunningProcess],
    instance: &crate::instance::ServerInstance,
    name_is_unique: bool,
    recorded_path_is_unique: bool,
) -> Option<&'a crate::process_monitor::RunningProcess> {
    let recorded = instance
        .tracked_process_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .filter(|_| recorded_path_is_unique)
        .map(normalize_path_for_match);

    // 1) 기록된 exe 경로 정확 일치 최우선
    if let Some(rec) = &recorded {
        if let Some(p) = candidates.iter().find(|p| {
            p.executable_path
                .as_deref()
                .map(normalize_path_for_match)
                .as_ref()
                == Some(rec)
        }) {
            return Some(p);
        }
    }

    let mut roots: Vec<String> = Vec::new();
    if let Some(wd) = instance.working_dir.as_deref().filter(|s| !s.is_empty()) {
        roots.push(normalize_path_for_match(wd));
    }
    if let Some(exe) = instance.executable_path.as_deref().filter(|s| !s.is_empty()) {
        if let Some(parent) = std::path::Path::new(exe).parent() {
            roots.push(normalize_path_for_match(&parent.to_string_lossy()));
        }
    }
    // 기록 경로가 런처(예: PalServer.exe)였다면 그 상위 디렉토리 하위의 실제 자식
    // 프로세스(예: PalServer-Win64-Shipping-Cmd.exe)를 찾을 수 있도록 루트에 추가
    if let Some(rec) = &recorded {
        if let Some(parent) = std::path::Path::new(rec).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() {
                roots.push(parent_str.to_string());
            }
        }
    }

    if !roots.is_empty() {
        for p in candidates {
            let matches_root = |path: &str| {
                let norm = normalize_path_for_match(path);
                roots.iter().any(|r| norm.starts_with(r.as_str()))
            };
            let exe_match = p.executable_path.as_deref().map(matches_root).unwrap_or(false);
            let cmd_match = p.cmd.iter().any(|c| matches_root(c));
            if exe_match || cmd_match {
                return Some(p);
            }
        }
    }

    if name_is_unique {
        candidates.first().copied()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── pick_detected_process (경로 기반 auto-detect 매칭) ──

    fn mk_proc(pid: u32, exe: Option<&str>, cmd: Vec<&str>) -> crate::process_monitor::RunningProcess {
        crate::process_monitor::RunningProcess {
            pid,
            name: "PalServer-Win64-Shipping-Cmd.exe".to_string(),
            executable_path: exe.map(String::from),
            cmd: cmd.into_iter().map(String::from).collect(),
        }
    }

    fn mk_detect_inst(name: &str, working_dir: Option<&str>) -> crate::instance::ServerInstance {
        let mut inst = crate::instance::ServerInstance::new(name, "palworld");
        inst.working_dir = working_dir.map(String::from);
        inst
    }

    /// 마이그레이션 서버: 같은 process_name의 프로세스가 여러 개 떠 있어도
    /// working_dir(외부 Steam 라이브러리 경로) 하위에서 실행 중인 프로세스를 골라야 한다.
    #[test]
    fn pick_detected_process_prefers_path_match() {
        let p_old = mk_proc(100, Some(r"C:\Games\PalServer\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let p_migrated = mk_proc(200, Some(r"D:\SteamLibrary\steamapps\common\PalServer\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let candidates = vec![&p_old, &p_migrated];

        let inst = mk_detect_inst("migrated", Some(r"D:\SteamLibrary\steamapps\common\PalServer"));
        let picked = pick_detected_process(&candidates, &inst, false, true).unwrap();
        assert_eq!(picked.pid, 200);
    }

    /// exe 경로가 없으면 커맨드라인 토큰으로도 매칭할 수 있어야 한다.
    #[test]
    fn pick_detected_process_matches_via_cmdline() {
        let p = mk_proc(300, None, vec![r"D:\SteamLibrary\steamapps\common\PalServer\PalServer.exe", "-port=8211"]);
        let candidates = vec![&p];
        let inst = mk_detect_inst("migrated", Some(r"D:/SteamLibrary/steamapps/common/PalServer"));
        let picked = pick_detected_process(&candidates, &inst, false, true).unwrap();
        assert_eq!(picked.pid, 300);
    }

    /// 경로 매칭 실패 시: 동일 process_name 인스턴스가 유일하면 기존 동작(첫 후보 폴백)을
    /// 유지하고, 여러 개면 오추적을 막기 위해 None을 반환해야 한다.
    #[test]
    fn pick_detected_process_fallback_only_when_unique() {
        let p = mk_proc(400, Some(r"E:\Elsewhere\PalServer\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let candidates = vec![&p];
        let inst = mk_detect_inst("inst", Some(r"D:\SteamLibrary\steamapps\common\PalServer"));

        // 유일 인스턴스 → 폴백 허용 (working_dir이 부정확해도 추적 유지)
        assert_eq!(pick_detected_process(&candidates, &inst, true, true).map(|p| p.pid), Some(400));
        // 동일 이름 인스턴스 다수 → 남의 프로세스를 추적하지 않는다
        assert!(pick_detected_process(&candidates, &inst, false, true).is_none());
    }

    /// 경로 정보가 전혀 없는 인스턴스는 기존 동작(첫 후보)을 따른다 (유일할 때만).
    #[test]
    fn pick_detected_process_no_path_info_uses_first_when_unique() {
        let p = mk_proc(500, Some(r"C:\x\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let candidates = vec![&p];
        let inst = mk_detect_inst("inst", None);
        assert_eq!(pick_detected_process(&candidates, &inst, true, true).map(|p| p.pid), Some(500));
        assert!(pick_detected_process(&candidates, &inst, false, true).is_none());
    }

    /// 같은 게임 다중 인스턴스: working_dir이 없어도 기록된 exe 경로(tracked_process_path)로
    /// 각자 자기 프로세스를 정확히 찾아야 한다. 이 매칭이 없으면 동명 프로세스가 여러 개일 때
    /// 어느 인스턴스도 추적되지 않거나(None) 여러 인스턴스가 동시에 실행 중으로 표시된다.
    #[test]
    fn pick_detected_process_uses_recorded_exact_path() {
        let p_a = mk_proc(600, Some(r"C:\Servers\A\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let p_b = mk_proc(601, Some(r"D:\Servers\B\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let candidates = vec![&p_a, &p_b];

        let mut inst_b = mk_detect_inst("b", None);
        inst_b.tracked_process_path =
            Some(r"D:\Servers\B\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe".to_string());
        // 이름이 유일하지 않아도 (다중 인스턴스) 기록 경로로 정확히 매칭
        assert_eq!(
            pick_detected_process(&candidates, &inst_b, false, true).map(|p| p.pid),
            Some(601)
        );
    }

    /// 기록 경로가 런처(예: PalServer.exe)여도 그 상위 디렉토리를 루트로 삼아
    /// 하위에서 실행 중인 실제 자식 프로세스를 찾아야 한다.
    /// (non-managed 시작 직후에는 런처 경로만 기록되어 있을 수 있음)
    #[test]
    fn pick_detected_process_recorded_launcher_parent_matches_child() {
        let p = mk_proc(610, Some(r"D:\Servers\B\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let candidates = vec![&p];

        let mut inst = mk_detect_inst("b", None);
        inst.tracked_process_path = Some(r"D:\Servers\B\PalServer.exe".to_string());
        assert_eq!(
            pick_detected_process(&candidates, &inst, false, true).map(|p| p.pid),
            Some(610)
        );
    }

    /// 공유 바이너리(java.exe 등): 기록 경로가 인스턴스 간에 겹치면(recorded_path_is_unique=false)
    /// 경로 매칭에 사용하지 않는다 — 남의 프로세스 오추적 방지.
    #[test]
    fn pick_detected_process_shared_recorded_path_not_used() {
        let p = mk_proc(620, Some(r"C:\Java\bin\java.exe"), vec![r"C:\Java\bin\java.exe", "-jar", r"C:\Other\server.jar"]);
        let candidates = vec![&p];

        let mut inst = mk_detect_inst("mc-a", None);
        inst.tracked_process_path = Some(r"C:\Java\bin\java.exe".to_string());
        // 기록 경로 공유 → 무시. working_dir도 없고 이름도 유일하지 않음 → None
        assert!(pick_detected_process(&candidates, &inst, false, false).is_none());
    }

    /// 기록 경로가 stale(서버 이전 등)이어도 working_dir 루트 매칭은 계속 동작해야 한다.
    #[test]
    fn pick_detected_process_stale_recorded_path_falls_back_to_roots() {
        let p = mk_proc(630, Some(r"D:\SteamLibrary\steamapps\common\PalServer\Pal\Binaries\Win64\PalServer-Win64-Shipping-Cmd.exe"), vec![]);
        let candidates = vec![&p];

        let mut inst = mk_detect_inst("b", Some(r"D:\SteamLibrary\steamapps\common\PalServer"));
        inst.tracked_process_path = Some(r"E:\OldPlace\PalServer.exe".to_string());
        assert_eq!(
            pick_detected_process(&candidates, &inst, false, true).map(|p| p.pid),
            Some(630)
        );
    }

    #[tokio::test]
    async fn test_supervisor_initialization() {
        let supervisor = Supervisor::new("./modules");
        
        // Supervisor가 정상적으로 생성되었는지 확인
        assert_eq!(supervisor.instance_store.list().len(), 0);
    }

    /// `[module.dependencies]`에 `ext-{id}` 형태로 선언된 익스텐션 최소 버전 요구사항이
    /// 실제로 디스크에 설치된 익스텐션 버전과 대조되어야 한다.
    /// 이 병합이 없으면 정상 설치된 모듈도 항상 module_dependency_unsatisfied로 시작이
    /// 막힌다 (installed-manifest.json에는 익스텐션 버전이 절대 기록되지 않기 때문).
    #[tokio::test]
    async fn check_module_dependencies_satisfied_by_installed_extension() {
        let modules_tmp = tempfile::tempdir().unwrap();
        let mod_dir = modules_tmp.path().join("testgame");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(
            mod_dir.join("module.toml"),
            r#"
[module]
name = "testgame"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
ext-foo = ">=0.2.0"
"#,
        )
        .unwrap();

        let ext_tmp = tempfile::tempdir().unwrap();
        let ext_dir = ext_tmp.path().join("foo");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{ "id": "foo", "name": "Foo Extension", "version": "0.2.0" }"#,
        )
        .unwrap();

        let mut ext_mgr = crate::extension::ExtensionManager::new_isolated(ext_tmp.path().to_str().unwrap());
        ext_mgr.discover().unwrap();

        let mut supervisor = Supervisor::new(mod_dir.parent().unwrap().to_str().unwrap());
        supervisor.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(ext_mgr)));

        let result = supervisor.check_module_dependencies("testgame", "instance-1").await;
        assert!(result.is_none(), "설치된 익스텐션 버전이 요구사항을 만족하면 통과해야 함: {:?}", result);
    }

    #[tokio::test]
    async fn check_module_dependencies_fails_when_extension_version_too_old() {
        let modules_tmp = tempfile::tempdir().unwrap();
        let mod_dir = modules_tmp.path().join("testgame");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(
            mod_dir.join("module.toml"),
            r#"
[module]
name = "testgame"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
ext-foo = ">=0.5.0"
"#,
        )
        .unwrap();

        let ext_tmp = tempfile::tempdir().unwrap();
        let ext_dir = ext_tmp.path().join("foo");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{ "id": "foo", "name": "Foo Extension", "version": "0.1.0" }"#,
        )
        .unwrap();

        let mut ext_mgr = crate::extension::ExtensionManager::new_isolated(ext_tmp.path().to_str().unwrap());
        ext_mgr.discover().unwrap();

        let mut supervisor = Supervisor::new(mod_dir.parent().unwrap().to_str().unwrap());
        supervisor.extension_manager = Some(std::sync::Arc::new(tokio::sync::RwLock::new(ext_mgr)));

        let result = supervisor.check_module_dependencies("testgame", "instance-1").await;
        assert!(result.is_some(), "설치된 버전이 요구사항보다 낮으면 실패해야 함");
        assert_eq!(result.unwrap()["error"], "module_dependency_unsatisfied");
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
        let supervisor = Supervisor::new("./modules");
        
        // 존재하지 않는 인스턴스에 명령어 실행 시도
        let result = supervisor
            .execute_command(
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
        let result = supervisor.monitor_processes(&snapshot).await;
        assert!(result.is_ok());
    }

    /// 회귀 테스트: monitor_processes가 "방금 추적을 시작한" PID를,
    /// track() 호출보다 먼저 스캔된 stale snapshot 때문에 즉시 untrack해버리면
    /// (auto_detect가 꺼진 인스턴스의 경우) 실제로는 실행 중인데도 대시보드에
    /// 영구히 "정지중"으로 표시되는 버그가 재발한다. grace period가 이를 막아야 한다.
    #[tokio::test]
    async fn monitor_processes_defers_untrack_for_freshly_tracked_pid() {
        let tmp_dir = std::env::temp_dir().join(format!("saba-test-fresh-track-{}", std::process::id()));
        let instances_dir = tmp_dir.join("instances");
        let _ = std::fs::create_dir_all(&instances_dir);

        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());

        let inst = crate::instance::ServerInstance {
            id: "fresh-track-inst".to_string(),
            name: "FreshTrackServer".to_string(),
            module_name: "minecraft".to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            tracked_process_path: None,
            port: None,
            rcon_port: None,
            rcon_password: None,
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            settings_import_pending: false,
        };
        supervisor.instance_store.add(inst).unwrap();

        // start_server가 spawn 직후 하는 것과 동일하게 track() 호출
        let fake_pid = 999_999u32;
        supervisor.tracker.track("fresh-track-inst", fake_pid).unwrap();

        // monitor_processes에 전달되는 snapshot은 write lock 밖에서 미리 스캔된 것이므로,
        // 방금 시작된 fake_pid를 포함하지 않을 수 있다 — 빈 snapshot으로 이 상황을 재현한다.
        let stale_snapshot: Vec<crate::process_monitor::RunningProcess> = vec![];
        supervisor.monitor_processes(&stale_snapshot).await.unwrap();

        // grace period 이내이므로 untrack되면 안 된다
        assert_eq!(
            supervisor.tracker.get_pid("fresh-track-inst").ok(),
            Some(fake_pid),
            "freshly tracked PID must survive a monitor cycle using a stale (pre-start) snapshot"
        );

        // grace period가 지난 뒤에는 여전히 snapshot에 없으면 정상적으로 untrack되어야 한다
        supervisor.tracker.backdate_for_test("fresh-track-inst", TRACK_GRACE_SECS + 5);
        supervisor.monitor_processes(&stale_snapshot).await.unwrap();
        assert!(
            supervisor.tracker.get_pid("fresh-track-inst").is_err(),
            "a genuinely dead PID must still be untracked once the grace period has elapsed"
        );
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
            tracked_process_path: None,
            port: Some(25565),
            rcon_port: Some(25575),
            rcon_password: Some("test".to_string()),
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            settings_import_pending: false,
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
        let result = supervisor
            .start_managed_server("test-dup-guard", "minecraft", json!({}))
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
            tracked_process_path: None,
            port: Some(25566),
            rcon_port: Some(25576),
            rcon_password: Some("test".to_string()),
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            settings_import_pending: false,
        };
        let inst_dir = instances_dir.join("test-tracker-guard");
        let _ = std::fs::create_dir_all(&inst_dir);
        supervisor.instance_store.add(inst).unwrap();

        // managed_store에는 없지만 tracker에 현재 프로세스 PID를 등록
        let current_pid = std::process::id();
        supervisor.tracker.track("test-tracker-guard", current_pid).unwrap();

        let result = supervisor
            .start_managed_server("test-tracker-guard", "minecraft", json!({}))
            .await
            .expect("should return Ok, not Err");

        assert_eq!(result["success"], true);
        assert_eq!(result["already_running"], true);
        assert_eq!(result["pid"], current_pid);

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    // ── reattach_orphaned_managed (PID 재사용 검증) ──

    fn mk_reattach_inst(id: &str) -> crate::instance::ServerInstance {
        crate::instance::ServerInstance {
            id: id.to_string(),
            name: format!("Server-{}", id),
            module_name: "minecraft".to_string(),
            executable_path: None,
            working_dir: None,
            auto_detect: false,
            process_name: None,
            tracked_process_path: None,
            port: None,
            rcon_port: None,
            rcon_password: None,
            rest_host: None,
            rest_port: None,
            rest_username: None,
            rest_password: None,
            protocol_mode: "auto".to_string(),
            module_settings: std::collections::HashMap::new(),
            server_version: None,
            extension_data: std::collections::HashMap::new(),
            required_extensions: Vec::new(),
            settings_import_pending: false,
        }
    }

    /// 현재 테스트 프로세스만 담긴 스냅샷 — PID는 살아있음이 보장된다.
    fn snapshot_with_current_process() -> Vec<crate::process_monitor::RunningProcess> {
        let exe = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        let name = exe
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "test-binary".to_string());
        vec![crate::process_monitor::RunningProcess {
            pid: std::process::id(),
            name,
            executable_path: exe,
            cmd: vec![],
        }]
    }

    fn setup_reattach_supervisor(tag: &str) -> (Supervisor, std::path::PathBuf, std::path::PathBuf) {
        let tmp_dir = std::env::temp_dir().join(format!("saba-test-reattach-{}-{}", tag, std::process::id()));
        let instances_dir = tmp_dir.join("instances");
        let _ = std::fs::create_dir_all(&instances_dir);
        let mut supervisor = Supervisor::new_with_instances_dir("./modules", instances_dir.to_str().unwrap());
        let inst_id = format!("reattach-{}", tag);
        supervisor.instance_store.add(mk_reattach_inst(&inst_id)).unwrap();
        let inst_dir = instances_dir.join(&inst_id);
        let _ = std::fs::create_dir_all(&inst_dir);
        (supervisor, tmp_dir, inst_dir)
    }

    /// PID는 살아있지만 다른 프로그램이면(PID 재사용) 재연결하지 않고 pid 파일을
    /// 정리해야 한다. 오인 재연결하면 stop 시 무관한 프로세스를 강제 종료하게 된다.
    #[tokio::test]
    async fn reattach_skips_pid_reuse_mismatch() {
        let (mut supervisor, tmp_dir, inst_dir) = setup_reattach_supervisor("mismatch");
        let inst_id = "reattach-mismatch";
        let current_pid = std::process::id();

        std::fs::write(inst_dir.join("managed.pid"), current_pid.to_string()).unwrap();
        // spawn 당시 프로그램이 전혀 다른 바이너리였다고 기록 → 신원 불일치
        std::fs::write(
            inst_dir.join(MANAGED_META_FILENAME),
            serde_json::to_string(&ManagedPidMeta {
                pid: current_pid,
                program: r"C:\Games\PalServer\PalServer.exe".to_string(),
            }).unwrap(),
        ).unwrap();

        let snapshot = snapshot_with_current_process();
        supervisor.reattach_orphaned_managed(&snapshot).await;

        assert!(supervisor.tracker.get_pid(inst_id).is_err(), "PID 재사용 프로세스를 추적하면 안 됨");
        assert!(supervisor.managed_store.get(inst_id).await.is_none(), "managed store에 등록되면 안 됨");
        assert!(!inst_dir.join("managed.pid").exists(), "stale pid 파일은 정리되어야 함");
        assert!(!inst_dir.join(MANAGED_META_FILENAME).exists(), "stale meta 파일은 정리되어야 함");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// 신원이 일치하면 정상 재연결되어야 한다 (tracker + managed store 등록).
    #[tokio::test]
    async fn reattach_succeeds_when_identity_matches() {
        let (mut supervisor, tmp_dir, inst_dir) = setup_reattach_supervisor("match");
        let inst_id = "reattach-match";
        let current_pid = std::process::id();
        let current_exe = std::env::current_exe().unwrap().to_string_lossy().to_string();

        std::fs::write(inst_dir.join("managed.pid"), current_pid.to_string()).unwrap();
        std::fs::write(
            inst_dir.join(MANAGED_META_FILENAME),
            serde_json::to_string(&ManagedPidMeta { pid: current_pid, program: current_exe }).unwrap(),
        ).unwrap();

        let snapshot = snapshot_with_current_process();
        supervisor.reattach_orphaned_managed(&snapshot).await;

        assert_eq!(supervisor.tracker.get_pid(inst_id).ok(), Some(current_pid));
        assert!(supervisor.managed_store.get(inst_id).await.is_some());
        assert!(inst_dir.join("managed.pid").exists(), "정상 재연결 시 pid 파일은 유지");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// 메타 파일이 없는 레거시 pid 파일은 기존 동작대로 재연결을 허용한다.
    #[tokio::test]
    async fn reattach_allows_legacy_pid_file_without_meta() {
        let (mut supervisor, tmp_dir, inst_dir) = setup_reattach_supervisor("legacy");
        let inst_id = "reattach-legacy";
        let current_pid = std::process::id();

        std::fs::write(inst_dir.join("managed.pid"), current_pid.to_string()).unwrap();

        let snapshot = snapshot_with_current_process();
        supervisor.reattach_orphaned_managed(&snapshot).await;

        assert_eq!(supervisor.tracker.get_pid(inst_id).ok(), Some(current_pid));
        assert!(supervisor.managed_store.get(inst_id).await.is_some());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// 죽은 PID의 pid/meta 파일은 재연결 없이 정리되어야 한다.
    #[tokio::test]
    async fn reattach_cleans_dead_pid_files() {
        let (mut supervisor, tmp_dir, inst_dir) = setup_reattach_supervisor("dead");
        let inst_id = "reattach-dead";

        // Windows/Unix 공통으로 존재할 가능성이 사실상 없는 PID
        let dead_pid: u32 = 0xFFFF_FFF0;
        std::fs::write(inst_dir.join("managed.pid"), dead_pid.to_string()).unwrap();
        std::fs::write(
            inst_dir.join(MANAGED_META_FILENAME),
            serde_json::to_string(&ManagedPidMeta { pid: dead_pid, program: "x".to_string() }).unwrap(),
        ).unwrap();

        supervisor.reattach_orphaned_managed(&[]).await;

        assert!(supervisor.tracker.get_pid(inst_id).is_err());
        assert!(!inst_dir.join("managed.pid").exists());
        assert!(!inst_dir.join(MANAGED_META_FILENAME).exists());

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn normalize_binary_name_handles_variants() {
        assert_eq!(normalize_binary_name(r"C:\java\bin\java.exe"), "java");
        assert_eq!(normalize_binary_name("java"), "java");
        assert_eq!(normalize_binary_name("/usr/bin/Java"), "java");
        assert_eq!(
            normalize_binary_name(r"D:\Srv\PalServer-Win64-Shipping-Cmd.EXE"),
            "palserver-win64-shipping-cmd"
        );
    }
}
