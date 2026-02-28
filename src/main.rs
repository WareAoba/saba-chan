mod supervisor;
mod plugin;
mod protocol;
mod ipc;
mod config;
mod instance;
mod process_monitor;
mod python_env;
mod node_env;
mod utils;
mod extension;
mod validator;

use std::sync::Arc;
use tokio::sync::RwLock;

/// 기본 IPC 서버 포트
const DEFAULT_IPC_PORT: u16 = 57474;
/// 프로세스 모니터링 폴링 간격 (초)
const MONITOR_INTERVAL_SECS: u64 = 2;
/// 하트비트 reaper 간격 (초)
const HEARTBEAT_REAPER_INTERVAL_SECS: u64 = 30;
/// 모니터 연속 실패 허용 횟수
const MONITOR_MAX_CONSECUTIVE_ERRORS: u32 = 10;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("Core Daemon starting");

    // Load config
    let cfg = config::GlobalConfig::load().ok();
    let _ = &cfg; // 향후 설정 참조를 위해 유지

    // Initialize supervisor with module loader
    // 모듈 경로: %APPDATA%/saba-chan/modules (환경 변수 오버라이드 가능)
    let modules_path = plugin::resolve_modules_dir();
    let modules_path_str = modules_path.to_string_lossy().to_string();
    let supervisor = Arc::new(RwLock::new(supervisor::Supervisor::new(&modules_path_str)));
    {
        let mut sup = supervisor.write().await;
        if let Err(e) = sup.initialize().await {
            tracing::warn!("Failed to initialize supervisor: {}", e);
        }
    }

    // Generate IPC auth token
    match ipc::auth::generate_and_save_token() {
        Ok(token) => tracing::info!("IPC auth token generated ({} chars)", token.len()),
        Err(e) => tracing::warn!("Failed to generate IPC auth token (auth disabled): {}", e),
    }

    // Start IPC HTTP server
    let ipc_port = std::env::var("SABA_IPC_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_IPC_PORT);
    let ipc_addr = format!("127.0.0.1:{}", ipc_port);
    let ipc_server = ipc::IPCServer::new(supervisor.clone(), &ipc_addr);

    // Supervisor에 ExtensionManager 연결
    {
        let mut sup = supervisor.write().await;
        sup.extension_manager = Some(ipc_server.extension_manager.clone());
    }

    let client_registry = ipc_server.client_registry.clone();
    tracing::info!("Starting IPC server on {}", ipc_addr);

    // ── Extension hook: daemon.startup (비동기) ──────────────────
    // 익스텐션 초기화를 백그라운드에서 실행하여 서버 시작을 차단하지 않음.
    // GUI는 /api/extensions/init-status 로 진행 상태를 폴링.
    {
        let ext_mgr = ipc_server.extension_manager.clone();
        let init_tracker = ipc_server.extension_init_tracker.clone();
        tokio::spawn(async move {
            // 활성 익스텐션 중 daemon.startup hook이 있는 것을 찾아 개별 디스패치
            let mgr = ext_mgr.read().await;
            let hooks = mgr.hooks_for("daemon.startup");
            if hooks.is_empty() {
                tracing::debug!("No extensions have daemon.startup hook");
                return;
            }
            let ext_ids: Vec<String> = hooks.iter().map(|(ext, _)| ext.manifest.id.clone()).collect();
            drop(mgr); // 릴리즈 후 개별 디스패치

            tracing::info!("Dispatching daemon.startup hooks for {} extension(s)", ext_ids.len());

            for ext_id in &ext_ids {
                init_tracker.mark_started(ext_id, "Initializing...").await;
            }

            let ctx = serde_json::json!({});
            let mgr = ext_mgr.read().await;
            let results = mgr.dispatch_hook("daemon.startup", ctx).await;

            for (ext_id, result) in results {
                match result {
                    Ok(val) => {
                        let success = val.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
                        let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("OK");
                        if success {
                            tracing::info!("Extension '{}' startup complete: {}", ext_id, msg);
                        } else {
                            let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                            tracing::warn!("Extension '{}' startup failed: {}", ext_id, err);
                        }
                        init_tracker.mark_finished(&ext_id, success, msg).await;
                    }
                    Err(e) => {
                        tracing::error!("Extension '{}' startup error: {}", ext_id, e);
                        init_tracker.mark_finished(&ext_id, false, &e.to_string()).await;
                    }
                }
            }
        });
    }

    // 백그라운드 모니터링 태스크 시작
    let supervisor_monitor = supervisor.clone();
    tokio::spawn(async move {
        let mut error_count = 0;
        
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(MONITOR_INTERVAL_SECS)).await;
            
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
                        tracing::error!("Monitor error (count: {}): {}", error_count, e);
                    }
                    
                    if error_count >= MONITOR_MAX_CONSECUTIVE_ERRORS {
                        tracing::error!("Monitor has failed {} consecutive times, restarting monitoring", error_count);
                        error_count = 0;
                    }
                }
            }
        }
    });

    // Heartbeat reaper 태스크 — 30초마다 만료 클라이언트 확인, 봇 프로세스 정리
    let registry_reaper = client_registry.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(HEARTBEAT_REAPER_INTERVAL_SECS)).await;
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
    let supervisor_shutdown = supervisor.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutdown signal received, cleaning up...");

        // 1. 익스텐션 정리 (컨테이너 종료 등은 extension hook으로 위임)
        {
            let sup = supervisor_shutdown.read().await;
            let all_instances: Vec<_> = sup.instance_store.list()
                .iter()
                .collect();

            // Extension hook: daemon.shutdown — 익스텐션이 자체 정리 수행
            if let Some(ref ext_mgr) = sup.extension_manager {
                let ctx = serde_json::json!({
                    "instances": all_instances.iter().map(|i| {
                        serde_json::json!({
                            "id": &i.id,
                            "name": &i.name,
                            "module": &i.module_name,
                            "extension_data": &i.extension_data,
                            "instance_dir": sup.instance_store.instance_dir(&i.id).to_string_lossy().to_string(),
                        })
                    }).collect::<Vec<_>>(),
                });
                let mgr = ext_mgr.read().await;
                let results = mgr.dispatch_hook("daemon.shutdown", ctx).await;
                let handled = results.iter().any(|(_id, r)| {
                    r.as_ref()
                        .map(|v| v.get("handled").and_then(|h| h.as_bool()) == Some(true))
                        .unwrap_or(false)
                });
                if handled {
                    tracing::info!("[Shutdown] Extensions handled cleanup");
                }
            }
        }

        // 2. 등록된 모든 클라이언트의 봇 프로세스를 종료
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
