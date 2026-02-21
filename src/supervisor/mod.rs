pub mod process;
pub mod state_machine;
pub mod module_loader;
pub mod managed_process;
pub mod error;

use anyhow::Result;
use process::{ProcessTracker, ProcessManager};
use module_loader::{ModuleLoader, LoadedModule};
use managed_process::{ManagedProcess, ManagedProcessStore};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;
use crate::instance::InstanceStore;

/// Stop 후 auto-detect를 억제하는 쿨다운 시간 (초)
const STOP_COOLDOWN_SECS: u64 = 30;

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
    #[allow(dead_code)]
    pub module_loader: ModuleLoader,
    pub instance_store: InstanceStore,
    #[allow(dead_code)]
    pub process_manager: ProcessManager,
    /// Store for processes spawned and managed directly by the daemon (with stdio capture)
    pub managed_store: ManagedProcessStore,
    /// Stop 이후 auto-detect 억제 쿨다운 (instance_id → 정지 시각)
    stop_cooldowns: HashMap<String, Instant>,
    /// 익스텐션 매니저 (Phase 3에서 optional로 추가)
    pub extension_manager: Option<std::sync::Arc<tokio::sync::RwLock<crate::extension::ExtensionManager>>>,
    /// 포트 충돌로 인한 강제 정지 이벤트 큐 (GUI에서 폴링 후 drain)
    pub port_conflict_stops: std::sync::Mutex<Vec<PortConflictStopEvent>>,
}

impl Supervisor {
    pub fn new(modules_dir: &str) -> Self {
        // instances.json은 %APPDATA%/saba-chan/instances.json에 저장
        let instances_path = std::env::var("SABA_INSTANCES_PATH")
            .unwrap_or_else(|_| {
                #[cfg(target_os = "windows")]
                {
                    std::env::var("APPDATA")
                        .map(|appdata| format!("{}\\saba-chan\\instances.json", appdata))
                        .unwrap_or_else(|_| "./instances.json".to_string())
                }
                #[cfg(not(target_os = "windows"))]
                {
                    std::env::var("HOME")
                        .map(|home| format!("{}/.config/saba-chan/instances.json", home))
                        .unwrap_or_else(|_| "./instances.json".to_string())
                }
            });
        
        Self {
            tracker: ProcessTracker::new(),
            module_loader: ModuleLoader::new(modules_dir),
            instance_store: InstanceStore::new(&instances_path),
            process_manager: ProcessManager::new(),
            managed_store: ManagedProcessStore::new(),
            stop_cooldowns: HashMap::new(),
            extension_manager: None,
            port_conflict_stops: std::sync::Mutex::new(Vec::new()),
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
        
        Ok(())
    }

    /// Start a server by name (e.g., "my-server-1")
    /// Called by IPC API: POST /api/server/:name/start
    pub async fn start_server(&self, server_name: &str, module_name: &str, config: Value) -> Result<Value> {
        tracing::info!("Starting server '{}' with module '{}'", server_name, module_name);

        // Find instance to get executable_path and working_dir
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // ── 포트 충돌 검사 ──
        let all_instances = self.instance_store.list();
        let running_ids: std::collections::HashSet<String> = all_instances
            .iter()
            .filter(|i| self.tracker.get_pid(&i.id).is_ok())
            .map(|i| i.id.clone())
            .collect();
        let conflicts = crate::validator::check_port_conflicts(instance, &all_instances, &running_ids);
        if !conflicts.is_empty() {
            let details: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
            tracing::warn!("Port conflict detected for '{}': {:?}", server_name, details);
            return Ok(json!({
                "success": false,
                "error": "port_conflict",
                "error_code": "port_conflict",
                "message": format!("Cannot start '{}': port conflict detected", server_name),
                "conflicts": details,
            }));
        }

        // ── Extension hook: server.pre_start ──
        if let Some(ref ext_mgr) = self.extension_manager {
            let instance_dir = self.instance_store.instance_dir(&instance.id);
            let results = ext_mgr.read().await
                .dispatch_hook("server.pre_start", json!({
                    "instance_id": instance.id.clone(),
                    "instance_dir": instance_dir.to_string_lossy(),
                    "module": module_name,
                    "extension_data": &instance.extension_data,
                })).await;

            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        // Extension이 시작 성공 → log_follower가 있으면 로그 스트리머 등록
                        if val.get("success").and_then(|s| s.as_bool()) == Some(true) {
                            if let Some(log_cfg) = val.get("log_follower") {
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

                                // interactive=true → spawn() (bidirectional stdin/stdout)
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
                                    ).await
                                } else {
                                    managed_process::ManagedProcess::spawn_log_follower(
                                        program,
                                        &args,
                                        &work_dir,
                                        desc,
                                        log_pattern.as_deref(),
                                        strip_prefix,
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
                        }
                        return Ok(val.clone());
                    }
                }
            }
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

        // ── Native 모드: 기존 Python lifecycle 실행 ──

        // Merge instance info into config
        let mut merged_config = config.as_object().cloned().unwrap_or_default();
        if let Some(exe_path) = &instance.executable_path {
            merged_config.insert("server_executable".to_string(), json!(exe_path));
            merged_config.entry("server_jar".to_string()).or_insert_with(|| json!(exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            merged_config.insert("working_dir".to_string(), json!(work_dir));
        }
        if let Some(port) = instance.port {
            merged_config.insert("port".to_string(), json!(port));
        }
        if let Some(rcon_port) = instance.rcon_port {
            merged_config.entry("rcon_port".to_string()).or_insert_with(|| json!(rcon_port));
        }
        if let Some(rcon_pw) = &instance.rcon_password {
            merged_config.entry("rcon_password".to_string()).or_insert_with(|| json!(rcon_pw));
        }
        // Merge module_settings (ram, cpu_threads, use_aikar_flags, etc.)
        for (key, value) in &instance.module_settings {
            merged_config.entry(key.clone()).or_insert_with(|| value.clone());
        }
        // REST API fields for modules that use REST protocol
        if let Some(rest_host) = &instance.rest_host {
            merged_config.entry("rest_host".to_string()).or_insert_with(|| json!(rest_host));
        }
        if let Some(rest_port) = instance.rest_port {
            merged_config.entry("rest_port".to_string()).or_insert_with(|| json!(rest_port));
        }
        if let Some(rest_user) = &instance.rest_username {
            merged_config.entry("rest_username".to_string()).or_insert_with(|| json!(rest_user));
        }
        if let Some(rest_pw) = &instance.rest_password {
            merged_config.entry("rest_password".to_string()).or_insert_with(|| json!(rest_pw));
        }
        merged_config.entry("protocol_mode".to_string()).or_insert_with(|| json!(&instance.protocol_mode));
        if let Some(ver) = &instance.server_version {
            merged_config.entry("server_version".to_string()).or_insert_with(|| json!(ver));
        }
        let final_config = Value::Object(merged_config);

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Execute start function via plugin runner
        let result = crate::plugin::run_plugin(&module_path, "start", final_config).await?;

        if let Some(pid) = result.get("pid").and_then(|p| p.as_u64()) {
            let pid = pid as u32;
            self.tracker.track(server_name, pid)?;
            tracing::info!("Server '{}' started with PID {}", server_name, pid);
            Ok(json!({
                "success": true,
                "server": server_name,
                "pid": pid,
                "message": format!("Server '{}' started with PID {}", server_name, pid)
            }))
        } else {
            // If the module requires user action (e.g. server jar not found), pass through
            if result.get("action_required").is_some() {
                tracing::warn!("Module requires user action: {:?}", result.get("action_required"));
                return Ok(result);
            }
            // Return the error from the module
            if result.get("success").and_then(|s| s.as_bool()) == Some(false) {
                let error_msg = result.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error from module");
                tracing::error!("Module failed to start server: {}", error_msg);
                return Err(anyhow::anyhow!("{}", error_msg));
            }
            tracing::error!("Module returned no PID: {:?}", result);
            Err(anyhow::anyhow!("Module did not return PID"))
        }
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
                if force {
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

                // 프로세스 종료 대기 (최대 30초)
                let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
                loop {
                    if !managed.is_running() {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        tracing::warn!("Managed server '{}' did not exit in 30s, force killing", server_name);
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

                tracing::info!("Managed server '{}' stopped", server_name);
                // auto-detect 쿨다운 등록
                self.stop_cooldowns.insert(instance.id.clone(), Instant::now());
                return Ok(json!({
                    "success": true,
                    "server": server_name,
                    "message": format!("Server '{}' stopped", server_name)
                }));
            }
        }

        // ── Non-managed mode: Python lifecycle.py를 통한 정지 ──
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Build config with all necessary info for stop
        let mut config_obj = serde_json::Map::new();
        if let Some(exe_path) = &instance.executable_path {
            config_obj.insert("server_executable".to_string(), json!(exe_path));
            config_obj.insert("server_jar".to_string(), json!(exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            config_obj.insert("working_dir".to_string(), json!(work_dir));
        }
        // Pass PID from tracker so the module can kill the actual process
        if let Ok(pid) = self.tracker.get_pid(server_name).or_else(|_| self.tracker.get_pid(&instance.id)) {
            config_obj.insert("pid".to_string(), json!(pid));
        }
        // Pass RCON settings for graceful shutdown
        if let Some(rcon_port) = instance.rcon_port {
            config_obj.insert("rcon_port".to_string(), json!(rcon_port));
        }
        if let Some(rcon_pw) = &instance.rcon_password {
            config_obj.insert("rcon_password".to_string(), json!(rcon_pw));
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

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Build config with executable_path
        let mut config_obj = serde_json::Map::new();
        if let Some(exe_path) = &instance.executable_path {
            config_obj.insert("server_executable".to_string(), json!(exe_path));
            config_obj.insert("server_jar".to_string(), json!(exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            config_obj.insert("working_dir".to_string(), json!(work_dir));
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

    /// 서버에 명령어 실행
    pub async fn execute_command(
        &self,
        instance_id: &str,
        module_name: &str,
        command: &str,
        args: Value,
    ) -> Result<String> {
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
        let config = json!({
            "command": command,
            "args": args,
            "protocol_mode": &instance.protocol_mode,  // "rest" 또는 "rcon"
            "rcon_host": "127.0.0.1",
            "rcon_port": instance.rcon_port.unwrap_or(default_rcon_port),
            "rcon_password": instance.rcon_password.clone().unwrap_or_default(),
            "rest_host": instance.rest_host.clone().unwrap_or(default_rest_host),
            "rest_port": instance.rest_port.unwrap_or(default_rest_port),
            "rest_username": instance.rest_username.clone().unwrap_or_default(),
            "rest_password": instance.rest_password.clone().unwrap_or_default(),
            "pid": pid,
            "instance_id": instance_id,
        });

        // 플러그인 실행
        let result = crate::plugin::run_plugin(&module_path, "command", config).await?;

        // 결과 처리
        if let Some(success) = result.get("success").and_then(|s| s.as_bool()) {
            if success {
                let message = result
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Command executed successfully");
                Ok(message.to_string())
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
        &self,
        instance_id: &str,
        module_name: &str,
        config: Value,
    ) -> Result<Value> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        // ── 포트 충돌 검사 ──
        let all_instances = self.instance_store.list();
        let running_ids: std::collections::HashSet<String> = all_instances
            .iter()
            .filter(|i| self.tracker.get_pid(&i.id).is_ok())
            .map(|i| i.id.clone())
            .collect();
        let conflicts = crate::validator::check_port_conflicts(&instance, &all_instances, &running_ids);
        if !conflicts.is_empty() {
            let details: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
            tracing::warn!("Port conflict detected for managed instance '{}': {:?}", instance.name, details);
            return Ok(json!({
                "success": false,
                "error": "port_conflict",
                "error_code": "port_conflict",
                "message": format!("Cannot start '{}': port conflict detected", instance.name),
                "conflicts": details,
            }));
        }

        // ── Extension hook: server.pre_start (managed) ──
        if let Some(ext_mgr) = &self.extension_manager {
            let instance_dir = self.instance_store.instance_dir(&instance.id);
            let ctx = json!({
                "instance_id": instance_id,
                "instance_dir": instance_dir.to_string_lossy(),
                "module": module_name,
                "instance": serde_json::to_value(&instance).unwrap_or_default(),
                "config": &config,
                "managed": true,
                "extension_data": &instance.extension_data,
            });
            let mgr = ext_mgr.read().await;
            let results = mgr.dispatch_hook("server.pre_start", ctx).await;
            for (_ext_id, result) in &results {
                if let Ok(val) = result {
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        // Extension이 시작 성공 → log_follower가 있으면 로그 스트리머 등록
                        if val.get("success").and_then(|s| s.as_bool()) == Some(true) {
                            if let Some(log_cfg) = val.get("log_follower") {
                                let inst_dir = self.instance_store.instance_dir(&instance.id);
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
                                let log_pattern = self.module_loader.get_module(module_name)
                                    .ok()
                                    .and_then(|m| m.metadata.log_pattern.clone());

                                // interactive=true → spawn() (bidirectional stdin/stdout)
                                // interactive=false → spawn_log_follower() (read-only stdout)
                                let spawn_result = if interactive {
                                    let env_vars: Vec<(String, String)> = log_cfg.get("env_vars")
                                        .and_then(|e| e.as_object())
                                        .map(|obj| obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                                        .unwrap_or_default();
                                    tracing::info!("Spawning interactive IO bridge for '{}'", instance.name);
                                    managed_process::ManagedProcess::spawn(
                                        program,
                                        &args,
                                        work_dir.to_str().unwrap_or("."),
                                        env_vars,
                                        log_pattern.as_deref(),
                                    ).await
                                } else {
                                    managed_process::ManagedProcess::spawn_log_follower(
                                        program,
                                        &args,
                                        &work_dir,
                                        desc,
                                        log_pattern.as_deref(),
                                        strip_prefix,
                                    ).await
                                };
                                match spawn_result {
                                    Ok(follower) => {
                                        self.managed_store.insert(instance_id, follower).await;
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

        // Build config for the module
        let mut cfg = config.as_object().cloned().unwrap_or_default();
        if let Some(exe_path) = &instance.executable_path {
            cfg.insert("server_executable".to_string(), json!(exe_path));
            cfg.insert("server_jar".to_string(), json!(exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            cfg.insert("working_dir".to_string(), json!(work_dir));
        }
        if let Some(port) = instance.port {
            cfg.insert("port".to_string(), json!(port));
        }
        if let Some(rcon_port) = instance.rcon_port {
            cfg.entry("rcon_port".to_string()).or_insert_with(|| json!(rcon_port));
        }
        if let Some(rcon_pw) = &instance.rcon_password {
            cfg.entry("rcon_password".to_string()).or_insert_with(|| json!(rcon_pw));
        }
        // Merge module_settings (ram, cpu_threads, use_aikar_flags, etc.)
        for (key, value) in &instance.module_settings {
            cfg.entry(key.clone()).or_insert_with(|| value.clone());
        }
        // REST API fields for modules that use REST protocol
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
        // Managed mode indicator → Python module uses this to enforce RCON policy
        cfg.insert("managed".to_string(), json!(true));
        let final_config = Value::Object(cfg);

        // Get module and call get_launch_command
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

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

        // Spawn managed process
        let log_pattern = module.metadata.log_pattern.as_deref();
        let managed = ManagedProcess::spawn(program, &args, working_dir, env_vars, log_pattern).await?;
        let pid = managed.pid;

        // Track the process
        self.tracker.track(&instance.id, pid)?;
        self.managed_store.insert(&instance.id, managed).await;

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

    /// Get console output from a managed process
    pub async fn get_console_output(
        &self,
        instance_id: &str,
        since_id: Option<u64>,
        count: Option<usize>,
    ) -> Result<Value> {
        let proc = self.managed_store.get(instance_id).await
            .ok_or_else(|| anyhow::anyhow!("No managed process for instance '{}'", instance_id))?;

        let lines = if let Some(since) = since_id {
            proc.get_console_since(since).await
        } else {
            proc.get_recent_console(count.unwrap_or(100)).await
        };

        Ok(json!({
            "success": true,
            "lines": lines,
            "running": proc.is_running(),
        }))
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

    /// Install a server: download binary, setup directory, optional initial settings
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

        let mut config = json!({
            "version": version,
            "install_dir": install_dir,
            "accept_eula": accept_eula,
        });

        if let Some(name) = jar_name {
            config["jar_name"] = json!(name);
        }
        if let Some(settings) = initial_settings {
            config["initial_settings"] = settings;
        }

        let result = crate::plugin::run_plugin(&module_path, "install_server", config).await?;
        Ok(result)
    }

    /// 백그라운드 프로세스 모니터링 (주기적 실행)
    pub async fn monitor_processes(&mut self) -> Result<()> {
        use crate::process_monitor::ProcessMonitor;

        // Clean up dead managed processes
        self.managed_store.cleanup_dead().await;
        
        let instances = self.instance_store.list().to_vec();
        let mut tracked_count = 0;
        let mut auto_detected_count = 0;
        
        for instance in instances {
            // 이미 추적 중이면 상태만 확인
            if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                tracked_count += 1;
                
                if !ProcessMonitor::is_running(pid) {
                    tracing::warn!("Process {} for instance '{}' is no longer running, removing from tracker", pid, instance.name);
                    // tracker에서 제거하여 다음 사이클에서 다시 감지할 수 있도록 함
                    if let Err(e) = self.tracker.untrack(&instance.id) {
                        tracing::error!("Failed to untrack process: {}", e);
                    }
                }
                continue;
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
                    match ProcessMonitor::find_by_name(process_name) {
                        Ok(processes) => {
                            if let Some(process) = processes.first() {
                                auto_detected_count += 1;
                                tracing::info!(
                                    "Auto-detected process '{}' (PID: {}) for instance '{}'",
                                    process_name, process.pid, instance.name
                                );
                                if let Err(e) = self.tracker.track(&instance.id, process.pid) {
                                    tracing::error!("Failed to track process: {}", e);
                                }

                                // ── 포트 충돌 검사: 새로 감지된 프로세스의 포트가 이미 실행 중인 인스턴스와 겹치면 강제 종료 ──
                                let all_instances = self.instance_store.list();
                                let running_ids: std::collections::HashSet<String> = all_instances
                                    .iter()
                                    .filter(|i| i.id != instance.id && self.tracker.get_pid(&i.id).is_ok())
                                    .map(|i| i.id.clone())
                                    .collect();
                                let conflicts = crate::validator::check_port_conflicts(&instance, &all_instances, &running_ids);
                                if !conflicts.is_empty() {
                                    let details: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
                                    tracing::warn!(
                                        "Port conflict detected for auto-detected '{}' — force stopping (PID {}). Conflicts: {:?}",
                                        instance.name, process.pid, details
                                    );

                                    // 프로세스 강제 종료
                                    let mut sys = sysinfo::System::new();
                                    sys.refresh_processes();
                                    if let Some(proc) = sys.process(sysinfo::Pid::from_u32(process.pid)) {
                                        proc.kill();
                                        tracing::info!("Force-killed PID {} for instance '{}'", process.pid, instance.name);
                                    }

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
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Failed to search for process '{}': {}", process_name, e);
                            // ProcessMonitor 오류는 로깅만 하고 계속
                        }
                    }
                }
            }
        }
        
        tracing::debug!("Monitor cycle: {} tracked, {} auto-detected", tracked_count, auto_detected_count);
        Ok(())
    }

    pub async fn monitor(&mut self) -> Result<()> {
        // Periodically check process health
        tracing::info!("Supervisor monitoring started");
        Ok(())
    }

    /// 포트 충돌 강제 정지 이벤트를 drain (GUI 폴링에서 한 번 읽으면 비움)
    pub fn drain_port_conflict_stops(&self) -> Vec<PortConflictStopEvent> {
        self.port_conflict_stops
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }
}

/// Get Discord bot config file path
pub fn get_discord_bot_config_path() -> String {
    // 프로젝트 루트 기준으로 경로 설정
    let current_dir = std::env::current_dir().unwrap_or_default();
    current_dir
        .join("discord_bot")
        .join("bot-config.json")
        .to_string_lossy()
        .to_string()
}

#[allow(dead_code)]
pub async fn run() -> Result<()> {
    tracing::info!("Supervisor starting");
    let mut supervisor = Supervisor::new("./modules");
    supervisor.initialize().await?;
    supervisor.monitor().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn test_discord_bot_config_path() {
        let path = get_discord_bot_config_path();
        
        // 경로에 discord_bot/bot-config.json이 포함되어야 함
        assert!(path.contains("discord_bot"));
        assert!(path.contains("bot-config.json"));
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
        let result = supervisor.monitor_processes().await;
        assert!(result.is_ok());
    }
}
