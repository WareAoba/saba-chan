mod supervisor;
mod plugin;
mod ipc;
mod resource;
mod config;
mod instance;
mod process_monitor;
mod path_detector;

use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Core Daemon starting");

    // Load config (stub)
    let _cfg = config::GlobalConfig::load().ok();

    // Initialize supervisor with module loader
    let supervisor = Arc::new(RwLock::new(supervisor::Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        if let Err(e) = sup.initialize().await {
            tracing::warn!("Failed to initialize supervisor: {}", e);
        }
    }

    // Start IPC HTTP server
    let ipc_server = ipc::IPCServer::new(supervisor.clone(), "127.0.0.1:57474");
    tracing::info!("Starting IPC server on 127.0.0.1:57474");
    
    // 백그라운드 모니터링 태스크 시작
    let supervisor_monitor = supervisor.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            
            let mut sup = supervisor_monitor.write().await;
            if let Err(e) = sup.monitor_processes().await {
                tracing::error!("Monitor error: {}", e);
            }
        }
    });
    
    if let Err(e) = ipc_server.start().await {
        tracing::error!("IPC server error: {}", e);
    }

    tracing::info!("Core Daemon shutting down");
    Ok(())
}
