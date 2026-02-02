pub mod process;
pub mod state_machine;  // Keep for now - used in supervisor logic
pub mod module_loader;

use anyhow::Result;
use process::{ProcessTracker, ProcessManager};
use module_loader::{ModuleLoader, LoadedModule};
use serde_json::{json, Value};
use crate::instance::InstanceStore;

pub struct Supervisor {
    pub tracker: ProcessTracker,
    #[allow(dead_code)]
    pub module_loader: ModuleLoader,
    pub instance_store: InstanceStore,
    pub process_manager: ProcessManager,
}

impl Supervisor {
    pub fn new(modules_dir: &str) -> Self {
        Self {
            tracker: ProcessTracker::new(),
            module_loader: ModuleLoader::new(modules_dir),
            instance_store: InstanceStore::new("./instances.json"),
            process_manager: ProcessManager::new(),
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

    /// Start a server by name (e.g., "minecraft-main")
    /// Called by IPC API: POST /api/server/:name/start
    pub async fn start_server(&self, server_name: &str, module_name: &str, config: Value) -> Result<Value> {
        tracing::info!("Starting server '{}' with module '{}'", server_name, module_name);

        // Find instance to get executable_path and working_dir
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // Merge instance info into config
        let mut merged_config = config.as_object().cloned().unwrap_or_default();
        if let Some(exe_path) = &instance.executable_path {
            merged_config.insert("server_executable".to_string(), json!(exe_path));
        }
        if let Some(work_dir) = &instance.working_dir {
            merged_config.insert("working_dir".to_string(), json!(work_dir));
        }
        if let Some(port) = instance.port {
            merged_config.insert("port".to_string(), json!(port));
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
    pub async fn stop_server(&self, server_name: &str, module_name: &str, force: bool) -> Result<Value> {
        tracing::info!("Stopping server '{}' (force: {})", server_name, force);

        // Find instance to get executable_path
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Build config with executable_path (or fallback to PID)
        let mut config_obj = serde_json::Map::new();
        if let Some(exe_path) = &instance.executable_path {
            config_obj.insert("server_executable".to_string(), json!(exe_path));
        }
        config_obj.insert("force".to_string(), json!(force));
        let config = Value::Object(config_obj);

        // Execute stop function
        let _result = crate::plugin::run_plugin(&module_path, "stop", config).await?;

        tracing::info!("Server '{}' stopped", server_name);
        Ok(json!({
            "success": true,
            "server": server_name,
            "message": format!("Server '{}' stopped", server_name)
        }))
    }

    /// Get server status by name
    /// Called by IPC API: GET /api/server/:name/status
    pub async fn get_server_status(&self, server_name: &str, module_name: &str) -> Result<Value> {
        tracing::info!("Getting status for server '{}'", server_name);

        // Find instance to get executable_path
        let instance = self.instance_store.list()
            .iter()
            .find(|i| i.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", server_name))?;

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Build config with executable_path
        let mut config_obj = serde_json::Map::new();
        if let Some(exe_path) = &instance.executable_path {
            config_obj.insert("server_executable".to_string(), json!(exe_path));
        }
        let config = Value::Object(config_obj);

        // Ask module for status
        match crate::plugin::run_plugin(&module_path, "status", config).await {
            Ok(result) => {
                let status = result.get("status").and_then(|s| s.as_str()).unwrap_or("unknown");
                let pid = result.get("pid").and_then(|p| p.as_u64());
                Ok(json!({
                    "server": server_name,
                    "status": status,
                    "pid": pid,
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

    /// 백그라운드 프로세스 모니터링 (주기적 실행)
    pub async fn monitor_processes(&mut self) -> Result<()> {
        use crate::process_monitor::ProcessMonitor;
        
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
