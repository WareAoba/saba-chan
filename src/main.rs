mod bootstrap;

use saba_core::{boot_selector, daemon_log, ipc, plugin, process_monitor, runtime_preflight, supervisor};

use std::sync::Arc;
use std::io::IsTerminal;
use tokio::sync::RwLock;
use saba_chan_updater_lib::constants;
/// 프로세스 모니터링 폴링 간격 (초)
const MONITOR_INTERVAL_SECS: u64 = 2;
/// 하트비트 reaper 간격 (초)
const HEARTBEAT_REAPER_INTERVAL_SECS: u64 = 30;
/// 모니터 연속 실패 허용 횟수
const MONITOR_MAX_CONSECUTIVE_ERRORS: u32 = 10;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Windows UTF-8 콘솔 출력 강제 ───────────────────────────
    // 시스템 로캘이 비-UTF-8(CP932 등)이면 eprintln! 출력이 깨짐
    #[cfg(windows)]
    unsafe {
        winapi::um::wincon::SetConsoleOutputCP(65001);
        winapi::um::wincon::SetConsoleCP(65001);
    }

    // ── Boot Mode Determination ────────────────────────────────
    // GUI/CLI가 데몬을 스폰할 때는 --spawned 플래그를 전달하거나
    // stdin이 터미널이 아닌 경우(파이프/null) 부트 선택기를 건너뜀
    let args: Vec<String> = std::env::args().collect();
    let spawned = args.iter().any(|a| a == "--spawned");
    let interactive =
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    let boot_mode = if !spawned && interactive {
        Some(boot_selector::run())
    } else {
        None
    };

    let daemon_only = matches!(boot_mode, Some(boot_selector::BootMode::DaemonOnly));

    // ── Daemon Log Buffer (모든 모드 공통) ─────────────────────
    // tracing 이벤트를 인메모리 링 버퍼에 캡처 → /api/daemon/console 로 노출
    let daemon_log_buffer = daemon_log::DaemonLogBuffer::new();
    let daemon_log_layer = daemon_log::DaemonLogLayer::new(daemon_log_buffer.clone());

    // ── Tracing Init (모드에 따라 출력 대상 결정) ──────────────
    // RUST_LOG 미설정 시 기본 info 레벨 (GUI/CLI 스폰 시와 동일)
    // 모든 모드에서 DaemonLogLayer를 함께 합성하여 API 접근 보장
    {
        use tracing_subscriber::prelude::*;

        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        match boot_mode {
            Some(boot_selector::BootMode::Cli) => {
                // CLI TUI가 터미널을 점유하므로 데몬 로그는 파일로 출력
                let log_path = constants::resolve_data_dir().join("daemon.log");
                if let Some(parent) = log_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&log_path)
                {
                    Ok(file) => {
                        tracing_subscriber::registry()
                            .with(env_filter)
                            .with(tracing_subscriber::fmt::layer()
                                .with_writer(std::sync::Mutex::new(file)))
                            .with(daemon_log_layer)
                            .init();
                    }
                    Err(_) => {
                        tracing_subscriber::registry()
                            .with(env_filter)
                            .with(tracing_subscriber::fmt::layer()
                                .with_writer(std::io::stderr))
                            .with(daemon_log_layer)
                            .init();
                    }
                }
            }
            _ => {
                // GUI / DaemonOnly / spawned 모드: stderr 출력
                // DaemonLogLayer가 함께 합성되어 버퍼에도 캡처
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr))
                    .with(daemon_log_layer)
                    .init();
            }
        }
    }

    tracing::info!("Core Daemon starting");
    if let Some(ref mode) = boot_mode {
        tracing::info!("[Boot] Mode: {}", mode);
    }

    // ── Self-bootstrap ─────────────────────────────────────────
    // 인스톨러/업데이터 미경유 환경(예: 빌드 산출물 수동 복사)에서도
    // 공유 Python 유틸리티(i18n.py, daemon_rcon.py)를 AppData/modules에 보장.
    bootstrap::run();

    // ── Integrity Check (무결성 검증) ──────────────────────────
    // 서버(GitHub)에서 매니페스트를 가져와 설치된 컴포넌트의 SHA256을 검증
    run_integrity_check_on_startup().await;

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
        Err(e) => tracing::warn!("Failed to generate IPC auth token; authenticated requests will be rejected: {}", e),
    }

    // Start IPC HTTP server
    let ipc_port = std::env::var("SABA_IPC_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(constants::DEFAULT_IPC_PORT);
    let ipc_addr = format!("127.0.0.1:{}", ipc_port);
    let ipc_server = ipc::IPCServer::new(supervisor.clone(), &ipc_addr, daemon_log_buffer.clone());

    // Supervisor에 ExtensionManager 연결
    {
        let mut sup = supervisor.write().await;
        sup.extension_manager = Some(ipc_server.extension_manager.clone());
        sup.provision_tracker = Some(ipc_server.provision_tracker.clone());
    }

    let client_registry = ipc_server.client_registry.clone();
    tracing::info!("Starting IPC server on {}", ipc_addr);    // ── Extension hook: daemon.startup (비동기) ──────────────────
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

    // ── Runtime preflight (Node/Python 자동 검사·복구) ──────────
    //
    // ★ 데몬 기동 직후 비동기 단일 태스크로 실행:
    //     1) 포터블 Node.js (없으면 다운로드)
    //     2) 포터블 Python venv (없으면 부트스트랩)
    //
    //   GUI는 `/api/runtime/preflight`을 폴링하여 LoadingScreen
    //   준비중 링에 단계별 진행을 표시한다.
    {
        let tracker = ipc_server.preflight_tracker.clone();
        tokio::spawn(async move {
            crate::runtime_preflight::run(tracker).await;
        });
    }

    // ── 인스턴스 server_version 백필 (IPC 비차단 백그라운드) ─────
    //
    // backfill_server_versions 는 모듈 lifecycle.py 호출을 수반해 Python venv
    // 최초 부트스트랩(수 초)을 트리거할 수 있다. IPC 시작 이전에 실행하면
    // GUI 가 `/api/modules` 를 호출할 때 ECONNREFUSED 가 발생하므로,
    // IPC 가 listen 을 시작한 뒤 별도 태스크로 수행한다.
    {
        let supervisor_backfill = supervisor.clone();
        tokio::spawn(async move {
            let mut sup = supervisor_backfill.write().await;
            sup.backfill_server_versions().await;
        });
    }

    // ── CancellationToken: 모든 백그라운드 태스크에 graceful shutdown 전파 ──
    let shutdown_token = ipc_server.shutdown_token.clone();

    // ── Cloud agent (long-poll + heartbeat) ─────────────────────
    // `.node_token` 이 존재하면 릴레이 서버에 연결해 명령을 수신한다.
    // 토큰이 없으면 idle 상태로 대기하며, 페어링 완료/토큰 저장 시 IPC 핸들러가
    // 데몬 재시작 없이 핫 리로드한다. 모든 부트 모드에서 동일하게 동작한다.
    ipc_server.cloud_agent.restart_from_file().await;

    // 백그라운드 모니터링 태스크 시작
    let supervisor_monitor = supervisor.clone();
    let monitor_cancel = shutdown_token.clone();
    tokio::spawn(async move {
        let mut error_count = 0;
        
        loop {
            tokio::select! {
                _ = monitor_cancel.cancelled() => {
                    tracing::info!("Monitor task shutting down");
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(MONITOR_INTERVAL_SECS)) => {}
            }
            
            // Phase 1: write lock 밖에서 프로세스 목록을 한 번 스캔 (비용이 큰 sysinfo 호출)
            let process_snapshot = crate::process_monitor::get_running_processes_async().await;
            
            // Phase 2: write lock 안에서는 snapshot 기반 매칭만 수행 (시스템 콜 없음)
            let mut sup = supervisor_monitor.write().await;
            match sup.monitor_processes(&process_snapshot).await {
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
    let reaper_cancel = shutdown_token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = reaper_cancel.cancelled() => {
                    tracing::info!("Heartbeat reaper shutting down");
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(HEARTBEAT_REAPER_INTERVAL_SECS)) => {}
            }
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
    //
    // ⚠️ daemon-only 모드에서는 렌더러가 없으므로 watchdog를 건너뜀
    //    → 메모리 절약 및 불필요한 자살 방지
    if daemon_only {
        tracing::info!("[DaemonOnly] Renderer watchdog disabled (no renderer expected)");
        // 데몬 온리 모드는 데몬 자체의 tracing 로그만 stderr에 출력한다.
        // 인스턴스(관리형 프로세스/컨테이너)의 콘솔 로그는 GUI/CLI에서 확인할 일이며,
        // 데몬 온리 환경에서 stderr로 끌어올리지 않는다.
    } else {
    let registry_watchdog = client_registry.clone();
    let watchdog_cancel = shutdown_token.clone();
    tokio::spawn(async move {
        const CHECK_INTERVAL_SECS: u64 = 5;
        const GRACE_PERIOD_SECS: u64 = 15;
        const RESTART_WAIT_SECS: u64 = 60;
        const MAX_RESTART_ATTEMPTS: u32 = 2;

        let mut restart_attempts: u32 = 0;

        loop {
            tokio::select! {
                _ = watchdog_cancel.cancelled() => {
                    tracing::info!("[Watchdog] Shutting down");
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(CHECK_INTERVAL_SECS)) => {}
            }

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
                // 만료 클라이언트 정리 (자원 해제)
                let timeout = std::time::Duration::from_secs(0);
                let all = registry_watchdog.reap_expired(timeout).await;
                for (id, client) in &all {
                    tracing::info!("[Watchdog] Cleanup client {} ({:?})", id, client.kind);
                }
                watchdog_cancel.cancel();
                break;
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
    } // end of `if !daemon_only` — watchdog block

    // Graceful shutdown: Ctrl+C / SIGTERM 시 정리
    let registry_shutdown = client_registry.clone();
    let supervisor_shutdown = supervisor.clone();
    let ext_process_shutdown = ipc_server.ext_process_manager.clone();
    let ctrl_c_cancel = shutdown_token.clone();
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

        // 2. 등록된 모든 클라이언트 정리 (봇 프로세스 관리는 더 이상 데몬 책임이 아님)
        let timeout = std::time::Duration::from_secs(0);
        let all = registry_shutdown.reap_expired(timeout).await;
        for (id, client) in &all {
            tracing::info!("[Shutdown] Cleaning client {} ({:?})", id, client.kind);
        }

        // 3. ExtProcessManager의 모든 실행 중 프로세스 정리
        //    daemon-only 모드에서는 클라이언트가 없어 2번이 무효하므로,
        //    ExtProcessManager를 통해 직접 시작된 봇 등을 반드시 정리
        {
            let mut mgr = ext_process_shutdown.lock().await;
            mgr.shutdown_all().await;
        }

        tracing::info!("Cleanup complete, signaling shutdown");
        // CancellationToken으로 모든 태스크에 종료 전파 (IPC 서버 포함)
        ctrl_c_cancel.cancel();
    });

    // ── Interface Spawn (부트 선택기 모드에 따른 인터페이스 기동) ──
    // IPC 서버가 바인드된 후 인터페이스를 스폰하기 위해 백그라운드 태스크 사용.
    // IPC 포트 연결 가능 확인 후 스폰 → GUI/CLI의 기존 데몬 감지 로직과 연동.
    if let Some(mode) = boot_mode {
        let port = ipc_port;
        tokio::spawn(async move {
            // IPC 서버가 리스닝을 시작할 때까지 대기
            let ready = boot_selector::wait_for_ipc_port(
                port,
                std::time::Duration::from_secs(15),
            )
            .await;

            if !ready {
                eprintln!("[Boot] IPC server failed to start within timeout");
                tracing::error!("[Boot] IPC server did not become ready in 15s");
                return;
            }

            match mode {
                boot_selector::BootMode::Gui => {
                    // GUI 스폰 시도 — 성공한 경우에만 콘솔 숨김
                    match boot_selector::spawn_gui() {
                        Ok(()) => {
                            tracing::info!("[Boot] GUI process launched, hiding console");
                            boot_selector::hide_console_window();
                        }
                        Err(e) => {
                            tracing::error!("[Boot] Failed to launch GUI: {}", e);
                            boot_selector::clear_for_daemon_only(port);
                        }
                    }
                }
                boot_selector::BootMode::Cli => {
                    // CLI를 현재 콘솔에 마운트
                    match boot_selector::spawn_cli() {
                        Ok(mut child) => {
                            tracing::info!("[Boot] CLI process launched");
                            // CLI 종료를 백그라운드에서 대기
                            let _ = tokio::task::spawn_blocking(move || {
                                let _ = child.wait();
                                eprintln!();
                                eprintln!("CLI exited. Daemon still running. Press Ctrl+C to shut down.");
                            })
                            .await;
                        }
                        Err(e) => {
                            tracing::error!("[Boot] Failed to launch CLI: {}", e);
                            boot_selector::clear_for_daemon_only(port);
                        }
                    }
                }
                boot_selector::BootMode::DaemonOnly => {
                    tracing::info!("[Boot] Daemon-only mode — no interface spawned");
                    boot_selector::clear_for_daemon_only(port);
                }
            }
        });
    }

    // IPC 서버 시작 — shutdown_token.cancel() 시 graceful shutdown
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

/// 시작 시 서버에서 매니페스트를 가져와 무결성 검증을 수행하고 결과를 터미널에 출력합니다.
async fn run_integrity_check_on_startup() {
    use saba_chan_updater_lib::integrity::{
        IntegrityStatus, OverallIntegrity,
    };
    use saba_chan_updater_lib::UpdateManager;

    // 업데이트 설정 로드 (relay_base_url 필요)
    let cfg = load_updater_config_for_integrity();
    if cfg.relay_base_url.trim().is_empty() {
        tracing::info!("[Integrity] Relay URL이 설정되지 않아 무결성 검증을 건너뜁니다");
        return;
    }

    let modules_dir = plugin::resolve_modules_dir();
    let modules_str = modules_dir.to_string_lossy().to_string();
    let mut manager = UpdateManager::new(cfg, &modules_str);

    let report = match manager.verify_integrity().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[Integrity] 서버 매니페스트 fetch 실패, 검증 건너뜀: {}", e);
            return;
        }
    };

    // ── 터미널 출력 ──
    tracing::info!("══════════════════════════════════════════════════");
    tracing::info!("  Component Integrity Check (server-verified)");
    tracing::info!("══════════════════════════════════════════════════");

    for c in &report.components {
        let icon = match c.status {
            IntegrityStatus::Verified    => "✅",
            IntegrityStatus::Tampered    => "❌",
            IntegrityStatus::NoHash      => "⚪",
            IntegrityStatus::FileNotFound => "⬜",
            IntegrityStatus::Error       => "⚠️",
        };
        let status_text = match c.status {
            IntegrityStatus::Verified    => "Verified",
            IntegrityStatus::Tampered    => "TAMPERED",
            IntegrityStatus::NoHash      => "No Hash",
            IntegrityStatus::FileNotFound => "Not Found",
            IntegrityStatus::Error       => "Error",
        };
        tracing::info!(
            "  {} {:<20} [{}] {}",
            icon, c.display_name, status_text, c.message
        );
    }

    tracing::info!("──────────────────────────────────────────────────");

    match report.overall {
        OverallIntegrity::AllVerified => {
            tracing::info!(
                "  ✅ All {} component(s) verified",
                report.verified
            );
        }
        OverallIntegrity::Partial => {
            tracing::info!(
                "  ⚪ {}/{} verified, {} skipped (no hash or not found)",
                report.verified, report.total, report.skipped
            );
        }
        OverallIntegrity::TamperDetected => {
            tracing::warn!(
                "  ❌ INTEGRITY FAILURE: {}/{} component(s) may be tampered!",
                report.failed, report.total
            );
        }
        OverallIntegrity::Empty => {
            tracing::info!("  No components to verify");
        }
    }

    tracing::info!("══════════════════════════════════════════════════");
}

/// 무결성 검증용 업데이터 설정 로드 (하드코딩 기본값)
fn load_updater_config_for_integrity() -> saba_chan_updater_lib::UpdateConfig {
    saba_chan_updater_lib::UpdateConfig::default()
}
