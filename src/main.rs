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
    let ipc_port = std::env::var("SABA_IPC_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(57474);
    let ipc_addr = format!("127.0.0.1:{}", ipc_port);
    let ipc_server = ipc::IPCServer::new(supervisor.clone(), &ipc_addr);
    let client_registry = ipc_server.client_registry.clone();
    tracing::info!("Starting IPC server on {}", ipc_addr);
    
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

            // 모든 클라이언트가 사라졌으면 watchdog 타이머 시작
            if registry_reaper.had_clients_ever().await && !registry_reaper.has_clients().await {
                registry_reaper.mark_all_clients_lost().await;
            }
        }
    });

    // ── Renderer Watchdog 태스크 ──────────────────────────────────
    // 렌더러(GUI/CLI) 프로세스가 패닉 등으로 전부 끊기면:
    //   1. 15초 대기 (자연 재접속 기회)
    //   2. GUI → CLI 순으로 재기동 시도  
    //   3. 재기동 후 60초 내 재접속 없으면 코어 데몬 자체 종료
    let registry_watchdog = client_registry.clone();
    tokio::spawn(async move {
        const CHECK_INTERVAL_SECS: u64 = 5;
        const GRACE_PERIOD_SECS: u64 = 15;
        const RESTART_WAIT_SECS: u64 = 60;
        const MAX_RESTART_ATTEMPTS: u32 = 2;

        let mut restart_attempts: u32 = 0;

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(CHECK_INTERVAL_SECS)).await;

            // 아직 클라이언트가 연결된 적 없으면 무시 (데몬 첫 기동 시)
            if !registry_watchdog.had_clients_ever().await {
                continue;
            }

            // 클라이언트가 있으면 카운터 리셋
            if registry_watchdog.has_clients().await {
                if restart_attempts > 0 {
                    tracing::info!("[Watchdog] Renderer reconnected after {} restart attempt(s)", restart_attempts);
                }
                restart_attempts = 0;
                continue;
            }

            // 클라이언트가 사라진 시점 확인
            let lost_at = match registry_watchdog.last_client_lost_at().await {
                Some(t) => t,
                None => continue,
            };

            let elapsed = std::time::Instant::now().duration_since(lost_at);

            // 유예 기간 내라면 자연 재접속 대기
            if elapsed < std::time::Duration::from_secs(GRACE_PERIOD_SECS) {
                continue;
            }

            // 재기동 시도 횟수 초과 → 자살
            if restart_attempts >= MAX_RESTART_ATTEMPTS {
                tracing::error!(
                    "[Watchdog] All {} restart attempts failed. No renderer reconnected within timeout.",
                    restart_attempts
                );
                tracing::error!("[Watchdog] Core daemon is terminating itself (self-destruct).");
                // 봇 프로세스 등 정리
                let timeout = std::time::Duration::from_secs(0);
                let all = registry_watchdog.reap_expired(timeout).await;
                for (id, client) in &all {
                    tracing::info!("[Watchdog] Cleanup client {} ({:?})", id, client.kind);
                    if let Some(pid) = client.bot_pid {
                        ipc::kill_bot_pid(pid);
                    }
                }
                std::process::exit(1);
            }

            // ── 렌더러 프로세스 재기동 시도 ──
            restart_attempts += 1;
            tracing::warn!(
                "[Watchdog] Attempting renderer restart (attempt {}/{})",
                restart_attempts, MAX_RESTART_ATTEMPTS
            );

            let restarted = try_restart_renderer().await;
            if restarted {
                tracing::info!("[Watchdog] Renderer process launched, waiting {}s for reconnection...", RESTART_WAIT_SECS);
                // 재기동 후 재접속 대기
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(RESTART_WAIT_SECS);
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    if registry_watchdog.has_clients().await {
                        tracing::info!("[Watchdog] Renderer successfully reconnected!");
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        tracing::warn!("[Watchdog] Renderer did not reconnect within {}s", RESTART_WAIT_SECS);
                        break;
                    }
                }
            } else {
                tracing::error!("[Watchdog] Failed to launch renderer process");
            }
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

/// 렌더러 프로세스(GUI 또는 CLI) 재기동을 시도합니다.
/// 코어 데몬 exe와 같은 디렉토리에 있는 GUI/CLI를 탐색합니다.
async fn try_restart_renderer() -> bool {
    let exe_dir = match std::env::current_exe() {
        Ok(p) => p.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from(".")),
        Err(_) => std::path::PathBuf::from("."),
    };

    // GUI를 먼저 시도, 실패하면 CLI 시도
    let candidates: Vec<(&str, std::path::PathBuf)> = if cfg!(windows) {
        vec![
            ("GUI", exe_dir.join("saba-chan-gui.exe")),
            ("CLI", exe_dir.join("saba-chan-cli.exe")),
        ]
    } else {
        vec![
            ("GUI", exe_dir.join("saba-chan-gui")),
            ("CLI", exe_dir.join("saba-chan-cli")),
        ]
    };

    for (label, path) in &candidates {
        if !path.exists() {
            tracing::debug!("[Watchdog] {} not found at {}", label, path.display());
            continue;
        }

        tracing::info!("[Watchdog] Launching {} from {}", label, path.display());
        match std::process::Command::new(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                tracing::info!("[Watchdog] {} launched (PID: {})", label, child.id());
                return true;
            }
            Err(e) => {
                tracing::error!("[Watchdog] Failed to launch {}: {}", label, e);
            }
        }
    }

    false
}
