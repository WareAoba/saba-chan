mod supervisor;
mod plugin;
mod protocol;
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
    let modules_path = std::env::var("SABA_MODULES_PATH")
        .unwrap_or_else(|_| "./modules".to_string());
    let supervisor = Arc::new(RwLock::new(supervisor::Supervisor::new(&modules_path)));
    {
        let mut sup = supervisor.write().await;
        if let Err(e) = sup.initialize().await {
            tracing::warn!("Failed to initialize supervisor: {}", e);
        }
    }

    // Start IPC HTTP server
    let ipc_server = ipc::IPCServer::new(supervisor.clone(), "127.0.0.1:57474");
    let client_registry = ipc_server.client_registry.clone();
    tracing::info!("Starting IPC server on 127.0.0.1:57474");
    
    // 백그라운드 모니터링 태스크 시작
    let supervisor_monitor = supervisor.clone();
    tokio::spawn(async move {
        let mut error_count = 0;
        let max_consecutive_errors = 10;
        
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            
            let mut sup = supervisor_monitor.write().await;
            match sup.monitor_processes().await {
                Ok(_) => {
                    if error_count > 0 {
                        tracing::info!("Monitor recovered after {} errors", error_count);
                    }
                    error_count = 0;
                }
                Err(e) => {
                    error_count += 1;
                    if error_count <= 3 || error_count % 10 == 0 {
                        // 처음 3번과 이후 10번마다 로깅하여 반복 로그 방지
                        tracing::error!("Monitor error (count: {}): {}", error_count, e);
                    }
                    
                    if error_count >= max_consecutive_errors {
                        tracing::error!("Monitor has failed {} consecutive times, restarting monitoring", error_count);
                        error_count = 0; // 리셋하여 무한 루프 방지
                    }
                }
            }
        }
    });

    // Heartbeat reaper 태스크 — 30초마다 만료 클라이언트 확인, 봇 프로세스 정리
    let registry_reaper = client_registry.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            ipc::reap_expired_clients(&registry_reaper).await;
        }
    });

    // Graceful shutdown: Ctrl+C / SIGTERM 시 정리
    let registry_shutdown = client_registry.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received, cleaning up...");

        // 등록된 모든 클라이언트의 봇 프로세스를 종료
        let timeout = std::time::Duration::from_secs(0); // 즉시 모든 클라이언트 만료 처리
        let all = registry_shutdown.reap_expired(timeout).await;
        for (id, client) in &all {
            tracing::info!("[Shutdown] Cleaning client {} ({:?})", id, client.kind);
            if let Some(pid) = client.bot_pid {
                ipc::kill_bot_pid(pid);
            }
        }

        tracing::info!("Cleanup complete, exiting");
        std::process::exit(0);
    });
    
    if let Err(e) = ipc_server.start().await {
        tracing::error!("IPC server error: {}", e);
    }

    tracing::info!("Core Daemon shutting down");
    Ok(())
}
