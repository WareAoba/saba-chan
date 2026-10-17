mod supervisor;
mod plugin;
mod protocol;
mod ipc;
mod config_store;
mod instance;
mod process_monitor;
mod python_env;
mod utils;
mod extension;
mod validator;
mod boot_selector;
mod daemon_log;
mod instance_lock;
mod chat;
mod commands;
mod relay_agent;

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

/// `saba-core module schema` — module.toml JSON Schema 출력
/// `saba-core module validate <모듈 폴더>...` — 데몬과 같은 규칙으로 검사 (실패하면 종료 코드 1)
fn module_tool(args: &[String]) -> i32 {
    use saba_core::supervisor::module_loader::{load_module_dir, module_json_schema};
    match args.first().map(String::as_str) {
        Some("schema") => {
            println!("{}", serde_json::to_string_pretty(&module_json_schema()).unwrap_or_default());
            0
        }
        Some("validate") if args.len() > 1 => {
            let mut failed = 0;
            for dir in &args[1..] {
                match load_module_dir(std::path::Path::new(dir)) {
                    Ok(m) => {
                        println!("ok    {} ({} v{})", dir, m.metadata.name, m.metadata.version);
                        for w in &m.metadata.warnings {
                            println!("      warning: {}", w);
                        }
                    }
                    Err(e) => {
                        println!("FAIL  {}: {}", dir, e);
                        failed += 1;
                    }
                }
            }
            i32::from(failed > 0)
        }
        _ => {
            eprintln!("usage: saba-core module schema | saba-core module validate <module dir>...");
            2
        }
    }
}

/// `saba-core extension validate <익스텐션 폴더>...` — manifest.json을 데몬과 같은 규칙으로 검사
fn extension_tool(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("validate") || args.len() < 2 {
        eprintln!("usage: saba-core extension validate <extension dir>...");
        return 2;
    }
    let mut failed = 0;
    for dir in &args[1..] {
        let path = std::path::Path::new(dir).join("manifest.json");
        let result = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|c| saba_core::extension::parse_manifest(&c));
        match result {
            Ok((m, warnings)) => {
                println!("ok    {} ({} v{})", dir, m.id, m.version);
                for w in warnings {
                    println!("      warning: {}", w);
                }
            }
            Err(e) => {
                println!("FAIL  {}: {:#}", dir, e);
                failed += 1;
            }
        }
    }
    i32::from(failed > 0)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Boot Mode Determination ────────────────────────────────
    // GUI/CLI가 데몬을 스폰할 때는 --spawned 플래그를 전달하거나
    // stdin이 터미널이 아닌 경우(파이프/null) 부트 선택기를 건너뜀
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("module") {
        std::process::exit(module_tool(&args[2..]));
    }
    if args.get(1).map(String::as_str) == Some("extension") {
        std::process::exit(extension_tool(&args[2..]));
    }
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

    // 포트: SABA_IPC_PORT > config.json의 gui.ipcPort > 기본값 (GUI·CLI가 같은 값을 읽는다)
    let ipc_port = std::env::var("SABA_IPC_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .or_else(|| {
            saba_chan_updater_lib::config_file::ConfigFile::default_location()
                .section("gui")
                .get("ipcPort")
                .and_then(|v| v.as_u64())
                .and_then(|p| u16::try_from(p).ok())
        })
        .unwrap_or(constants::DEFAULT_IPC_PORT);
    let ipc_addr = format!("127.0.0.1:{}", ipc_port);
    plugin::set_ipc_port(ipc_port);

    // ── Single Instance ───────────────────────────────────────
    // 토큰 생성·Supervisor 초기화보다 먼저 잠근다. 두 번째 데몬이 .ipc_token을
    // 덮어써 실행 중인 데몬이 401을 내는 문제를 막는다.
    let _instance_lock = match instance_lock::acquire_default(&ipc_addr) {
        Ok(lock) => Some(lock),
        Err(e @ instance_lock::AcquireError::AlreadyRunning(_)) => {
            tracing::error!("{}", e);
            eprintln!("{}", e);
            std::process::exit(2);
        }
        Err(e) => {
            // 잠금 파일을 못 여는 환경(읽기 전용 등)에서는 기존 동작대로 계속한다.
            tracing::warn!("{} — continuing without single-instance guard", e);
            None
        }
    };

    // 표를 끊는 데몬이라는 기록. 업데이터는 이게 없으면 옛 데몬으로 본다 (C13 전환 경로)
    saba_chan_updater_lib::launch_ticket::record_core_version(env!("CARGO_PKG_VERSION"));
    // Python을 첫 모듈 호출 전에 미리 준비한다 (GUI는 /api/runtime/preflight로 진행을 본다)
    tokio::spawn(async {
        if let Err(e) = python_env::get_python_path().await {
            tracing::warn!("Python preflight failed: {:#}", e);
        }
    });

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
    // server_version이 비어 있는 인스턴스의 버전 감지 (모듈 Python 호출이라 기동을 막지 않게 뒤에서)
    tokio::spawn(supervisor::Supervisor::backfill_server_versions(supervisor.clone()));

    // Generate IPC auth token
    match ipc::auth::generate_and_save_token() {
        Ok(token) => tracing::info!("IPC auth token generated ({} chars)", token.len()),
        Err(e) => tracing::warn!("Failed to generate IPC auth token (auth disabled): {}", e),
    }

    // Start IPC HTTP server
    let ipc_server = ipc::IPCServer::new(supervisor.clone(), &ipc_addr, daemon_log_buffer.clone());

    // Supervisor에 ExtensionManager 연결
    {
        let mut sup = supervisor.write().await;
        sup.extension_manager = Some(ipc_server.extension_manager.clone());
        sup.provision_tracker = Some(ipc_server.provision_tracker.clone());
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
            let results = extension::run_hook(&ext_mgr, "daemon.startup", ctx).await;

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

    // ── CancellationToken: 모든 백그라운드 태스크에 graceful shutdown 전파 ──
    let shutdown_token = ipc_server.shutdown_token.clone();
    let stop_requested_token = ipc_server.stop_requested.clone();

    // 백그라운드 모니터링 태스크 시작
    let supervisor_monitor = supervisor.clone();
    let monitor_cancel = shutdown_token.clone();
    let restart_ipc = ipc_server.clone();
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
            // 크래시로 끝난 서버 다시 띄우기 (A1). 대기는 락 밖에서.
            let restarts = sup.take_pending_restarts();
            drop(sup);
            for (instance_id, delay) in restarts {
                tokio::spawn(ipc::handlers::server::auto_restart(restart_ipc.clone(), instance_id, delay));
            }
        }
    });

    // 업데이트 주기 확인 — GUI가 꺼져 있어도 데몬이 확인한다 (U6)
    ipc::updates::spawn_update_scheduler(ipc_server.update_state.clone(), shutdown_token.clone());

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
        // 데몬 온리 모드: 터미널 폴링 루프 시작 — 데몬 로그 + 프로세스 콘솔을 stderr에 표시
        let term_buf = daemon_log_buffer.clone();
        let term_sup = supervisor.clone();
        let term_cancel = shutdown_token.clone();
        tokio::spawn(async move {
            daemon_log::daemon_terminal_loop(term_buf, term_sup, term_cancel).await;
        });
    } else {
    let registry_watchdog = client_registry.clone();
    let watchdog_cancel = shutdown_token.clone();
    let watchdog_stop = stop_requested_token.clone();
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
                // 정리는 종료 경로 하나(아래 Graceful shutdown 태스크)가 맡는다
                watchdog_stop.cancel();
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
    let ctrl_c_cancel = shutdown_token.clone();
    let stop_requested = stop_requested_token.clone();
    tokio::spawn(async move {
        // 종료 경로는 하나: Ctrl+C/SIGTERM, API(/api/daemon/shutdown, unregister), 워치독 모두 여기로
        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("Shutdown signal received, cleaning up..."),
            _ = stop_requested.cancelled() => tracing::info!("Shutdown requested, cleaning up..."),
        }

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
                // 익스텐션이 멈춰도 종료는 진행되게 상한을 둔다
                let results = match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    extension::run_hook(ext_mgr, "daemon.shutdown", ctx),
                )
                .await
                {
                    Ok(results) => results,
                    Err(_) => {
                        tracing::warn!("[Shutdown] daemon.shutdown hooks timed out after 30s");
                        Vec::new()
                    }
                };
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

        // 2. 등록된 클라이언트 정리
        let timeout = std::time::Duration::from_secs(0); // 즉시 모든 클라이언트 만료 처리
        let all = registry_shutdown.reap_expired(timeout).await;
        for (id, client) in &all {
            tracing::info!("[Shutdown] Cleaning client {} ({:?})", id, client.kind);
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

    // GUI만 다시 띄운다. CLI는 터미널 없이 띄워 봐야 쓸 수 없고, heartbeat 클라이언트도 아니다
    let candidates: Vec<(&str, std::path::PathBuf)> =
        vec![("GUI", exe_dir.join(format!("saba-chan-gui{}", std::env::consts::EXE_SUFFIX)))];

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

