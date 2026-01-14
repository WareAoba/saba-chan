pub mod process;
pub mod state_machine;
pub mod module_loader;

use anyhow::Result;
use process::ProcessTracker;
use module_loader::{ModuleLoader, LoadedModule};
use serde_json::{json, Value};
use crate::instance::{InstanceStore, ServerInstance};

pub struct Supervisor {
    pub tracker: ProcessTracker,
    #[allow(dead_code)]
    pub module_loader: ModuleLoader,
    pub instance_store: InstanceStore,
}

impl Supervisor {
    pub fn new(modules_dir: &str) -> Self {
        Self {
            tracker: ProcessTracker::new(),
            module_loader: ModuleLoader::new(modules_dir),
            instance_store: InstanceStore::new("./instances.json"),
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
    #[allow(dead_code)]
    pub async fn start_server(&self, server_name: &str, module_name: &str, config: Value) -> Result<Value> {
        tracing::info!("Starting server '{}' with module '{}'", server_name, module_name);

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Execute start function via plugin runner
        let result = crate::plugin::run_plugin(&module_path, "start", config).await?;

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
            tracing::error!("Module returned no PID: {:?}", result);
            Err(anyhow::anyhow!("Module did not return PID"))
        }
    }

    /// Stop a server by name
    #[allow(dead_code)]
    pub async fn stop_server(&self, server_name: &str, module_name: &str, force: bool) -> Result<Value> {
        tracing::info!("Stopping server '{}' (force: {})", server_name, force);

        // Get PID from tracker
        let pid = self.tracker.get_pid(server_name)?;

        // Find module
        let module = self.module_loader.get_module(module_name)?;
        let module_path = format!("{}/lifecycle.py", module.path);

        // Execute stop function
        let config = json!({ "pid": pid, "force": force });
        let _result = crate::plugin::run_plugin(&module_path, "stop", config).await?;

        // Update tracker
        // (Note: terminate needs &mut, so it's a limitation for now)
        // self.tracker.terminate(server_name, force)?;

        tracing::info!("Server '{}' stopped", server_name);
        Ok(json!({
            "success": true,
            "server": server_name,
            "message": format!("Server '{}' stopped", server_name)
        }))
    }

    /// Get server status by name
    #[allow(dead_code)]
    pub async fn get_server_status(&self, server_name: &str, module_name: &str) -> Result<Value> {
        tracing::info!("Getting status for server '{}'", server_name);

        // Try to get PID from tracker
        let status = self.tracker.get_status(server_name).ok();
        let pid = self.tracker.get_pid(server_name).ok();

        // If running, ask module for detailed status
        if pid.is_some() {
            let module = self.module_loader.get_module(module_name)?;
            let module_path = format!("{}/lifecycle.py", module.path);
            let config = json!({ "pid": pid.unwrap() });
            let _result = crate::plugin::run_plugin(&module_path, "status", config).await?;
        }

        Ok(json!({
            "server": server_name,
            "status": status.map(|s| format!("{:?}", s)).unwrap_or_else(|| "unknown".to_string()),
            "pid": pid,
        }))
    }

    /// List all available modules
    pub fn list_modules(&self) -> Result<Vec<LoadedModule>> {
        self.module_loader.discover_modules()
    }

    /// 백그라운드 프로세스 모니터링 (주기적 실행)
    pub async fn monitor_processes(&mut self) -> Result<()> {
        use crate::process_monitor::ProcessMonitor;
        
        let instances = self.instance_store.list().to_vec();
        
        for instance in instances {
            // 이미 추적 중이면 상태만 확인
            if let Ok(pid) = self.tracker.get_pid(&instance.id) {
                if !ProcessMonitor::is_running(pid) {
                    tracing::warn!("Process {} for instance '{}' is no longer running, removing from tracker", pid, instance.name);
                    // tracker에서 제거하여 다음 사이클에서 다시 감지할 수 있도록 함
                    let _ = self.tracker.untrack(&instance.id);
                }
                continue;
            }
            
            // auto_detect가 활성화되어 있고 process_name이 설정되어 있으면 감지 시도
            if instance.auto_detect {
                if let Some(process_name) = &instance.process_name {
                    match ProcessMonitor::find_by_name(process_name) {
                        Ok(processes) => {
                            if let Some(process) = processes.first() {
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
                        }
                    }
                }
            }
        }
        
        Ok(())
    }

    pub async fn monitor(&mut self) -> Result<()> {
        // Periodically check process health
        tracing::info!("Supervisor monitoring started");
        Ok(())
    }
}

#[allow(dead_code)]
pub async fn run() -> Result<()> {
    tracing::info!("Supervisor starting");
    let mut supervisor = Supervisor::new("./modules");
    supervisor.initialize().await?;
    supervisor.monitor().await?;
    Ok(())
}
