mod supervisor;
mod plugin;
mod protocol;
mod ipc;
mod config_store;
mod instance;
mod backup;
mod module_paths;
mod scheduler;
mod process_monitor;
mod system_monitor;
mod python_env;
mod utils;
mod extension;
mod validator;
mod boot_selector;
mod daemon_log;
mod instance_lock;
mod chat;
mod commands;
mod module_audit;
mod audit;
mod events;
mod sandbox;
mod actions;
mod launch_args;
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
/// 데몬 로그 파일 상한. 넘으면 `daemon.log.1`로 넘긴다 (둘이 합쳐 이 값의 두 배까지)
const DAEMON_LOG_CAP: u64 = 5 * 1024 * 1024;

/// `saba-core module schema` — module.toml JSON Schema 출력
/// `saba-core module validate <모듈 폴더>...` — 데몬과 같은 규칙으로 검사 (실패하면 종료 코드 1)
/// `saba-core module audit <모듈 폴더>...` — Python 코드가 `[permissions]`에 없는 능력을 쓰면 종료 코드 1
/// (Python은 `SABA_PYTHON`, 없으면 PATH의 python3/python)
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
        Some("audit") if args.len() > 1 => {
            let python = module_audit::tool_python();
            let mut failed = 0;
            for dir in &args[1..] {
                let result = load_module_dir(std::path::Path::new(dir)).and_then(|m| {
                    let found = saba_core::module_audit::scan_blocking(&python, std::path::Path::new(dir))?;
                    Ok((saba_core::module_audit::undeclared(&m.metadata.permissions, &found), found.len()))
                });
                match result {
                    Ok((missing, _)) if missing.is_empty() => println!("ok    {}", dir),
                    Ok((missing, _)) => {
                        println!("FAIL  {}: uses capabilities not declared in [permissions]", dir);
                        for f in missing {
                            println!("      {} {}:{} {}", f.permission, f.file, f.line, f.detail);
                        }
                        failed += 1;
                    }
                    Err(e) => {
                        println!("FAIL  {}: {:#}", dir, e);
                        failed += 1;
                    }
                }
            }
            i32::from(failed > 0)
        }
        _ => {
            eprintln!("usage: saba-core module schema | module validate <module dir>... | module audit <module dir>...");
            2
        }
    }
}

/// `saba-core extension validate <익스텐션 폴더>...` — manifest.json을 데몬과 같은 규칙으로 검사
fn extension_tool(args: &[String]) -> i32 {
    let audit = match args.first().map(String::as_str) {
        Some("validate") if args.len() > 1 => false,
        Some("audit") if args.len() > 1 => true,
        _ => {
            eprintln!("usage: saba-core extension validate <extension dir>... | extension audit <extension dir>...");
            return 2;
        }
    };
    let python = module_audit::tool_python();
    let mut failed = 0;
    for dir in &args[1..] {
        let path = std::path::Path::new(dir).join("manifest.json");
        let result = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|c| saba_core::extension::parse_manifest(&c));
        let (m, warnings) = match result {
            Ok(v) => v,
            Err(e) => {
                println!("FAIL  {}: {:#}", dir, e);
                failed += 1;
                continue;
            }
        };
        if !audit {
            println!("ok    {} ({} v{})", dir, m.id, m.version);
            for w in warnings {
                println!("      warning: {}", w);
            }
            continue;
        }
        match saba_core::module_audit::scan_blocking(&python, std::path::Path::new(dir)) {
            Ok(found) => {
                let missing = saba_core::module_audit::undeclared(&m.permissions, &found);
                if missing.is_empty() {
                    println!("ok    {}", dir);
                } else {
                    println!("FAIL  {}: uses capabilities not declared in \"permissions\"", dir);
                    for f in missing {
                        println!("      {} {}:{} {}", f.permission, f.file, f.line, f.detail);
                    }
                    failed += 1;
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

const DAEMON_USAGE: &str = "Usage: saba-core [--spawned]
       saba-core module schema | module validate <dir>... | module audit <dir>...
       saba-core extension validate <dir>... | extension audit <dir>...
       saba-core --version | --help

  --spawned   started by the GUI/CLI (skips the boot selector)";

enum DaemonArgs {
    Run,
    Version,
    Help,
}

/// 데몬 실행 인자. 알 수 있는 것만 받고, 나머지는 그 인자를 돌려준다
fn daemon_args(args: &[String]) -> Result<DaemonArgs, String> {
    let mut mode = DaemonArgs::Run;
    for arg in args {
        match arg.as_str() {
            "--version" | "-V" => return Ok(DaemonArgs::Version),
            "--help" | "-h" => mode = DaemonArgs::Help,
            // --after-update: 업데이터가 다시 띄울 때 붙인다
            "--spawned" | "--after-update" => {}
            other => return Err(other.to_string()),
        }
    }
    Ok(mode)
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
    // 데이터 폴더를 건드리기 전에 인자를 본다: 모르는 인자로 데몬이 떠서 설정을 이관하면 안 된다
    match daemon_args(&args[1..]) {
        Ok(DaemonArgs::Run) => {}
        Ok(DaemonArgs::Version) => {
            println!("saba-core {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(DaemonArgs::Help) => {
            println!("{}", DAEMON_USAGE);
            return Ok(());
        }
        Err(bad) => {
            eprintln!("saba-core: unknown argument '{}'\n\n{}", bad, DAEMON_USAGE);
            std::process::exit(2);
        }
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
                // stderr는 대개 GUI의 파이프라 GUI가 사라지면 남지 않으니 데몬 자신의 파일에도 쓴다. 패닉도 거기 남긴다
                let log_file = daemon_log::LogFile::open(&constants::resolve_data_dir().join("logs").join("daemon.log"), DAEMON_LOG_CAP);
                let file_writer = log_file.clone();
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stderr))
                    .with(tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(move || file_writer.writer()))
                    .with(daemon_log_layer)
                    .init();
                let default_hook = std::panic::take_hook();
                std::panic::set_hook(Box::new(move |info| {
                    log_file.write_panic(&info.to_string());
                    default_hook(info);
                }));
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
    // 설치 도중 끝났으면 남은 준비·옛 폴더를 지운다 (아직 아무 설치도 돌지 않는다).
    // 단일 실행 잠금을 못 잡았으면 같은 폴더를 쓰는 다른 데몬이 설치 중일 수 있어 건너뛴다
    if _instance_lock.is_some() {
        utils::remove_install_leftovers(&modules_path);
        utils::remove_install_leftovers(&plugin::resolve_extensions_dir());
    } else {
        tracing::warn!("Skipping cleanup of interrupted installs without the single-instance guard");
    }
    let supervisor = Arc::new(RwLock::new(supervisor::Supervisor::new(&modules_path_str)));
    {
        let mut sup = supervisor.write().await;
        if let Err(e) = sup.initialize().await {
            tracing::warn!("Failed to initialize supervisor: {}", e);
        }
    }
    // 예전에 준 런타임 폴더 읽기 권한 중 지금 쓰지 않는 것은 거둔다 (확인되지 않은 구성요소의 격리, Windows)
    {
        let users = supervisor.read().await.user_runtimes();
        tokio::task::spawn_blocking(move || sandbox::reconcile_runtimes(&users));
    }
    // 모듈이 구현한 함수 목록을 미리 물어 둔다 (GET /api/modules가 기다리지 않게)
    if let Ok(modules) = supervisor.read().await.list_modules() {
        plugin::prefetch_module_functions(modules.iter().map(|m| m.entry_path()).collect());
    }
    // server_version이 비어 있는 인스턴스의 버전 감지 (모듈 Python 호출이라 기동을 막지 않게 뒤에서)
    tokio::spawn(supervisor::Supervisor::backfill_server_versions(supervisor.clone()));
    tokio::spawn(supervisor::Supervisor::backfill_data_roots(supervisor.clone()));

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
            let taken_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let process_snapshot = crate::process_monitor::get_running_processes_async().await;
            
            // Phase 2: write lock 안에서는 snapshot 기반 매칭만 수행 (시스템 콜 없음)
            let mut sup = supervisor_monitor.write().await;
            match sup.monitor_processes_taken_at(&process_snapshot, taken_at).await {
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
            for (instance_id, delay, generation) in restarts {
                tokio::spawn(ipc::handlers::server::auto_restart(restart_ipc.clone(), instance_id, delay, generation));
            }
        }
    });

    // 업데이트 주기 확인 — GUI가 꺼져 있어도 데몬이 확인한다 (U6)
    ipc::updates::spawn_update_scheduler(ipc_server.update_state.clone(), shutdown_token.clone());
    // 인스턴스 예약 작업: 백업·재시작·공지·업데이트 (A3)
    ipc::handlers::backup::spawn_scheduler(ipc_server.clone());

    // 접속자 수 (Q-5): 모듈이 `[status] players`를 선언한 켜진 서버만 1분마다 묻는다
    let players_sup = supervisor.clone();
    let players_cancel = shutdown_token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = players_cancel.cancelled() => break,
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(supervisor::players::REFRESH_SECS)) => {}
            }
            supervisor::players::refresh(&players_sup).await;
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
        }
    });

    // 인터페이스가 모두 떠나도 데몬은 남는다 (데몬이 서비스의 주체, C14). GUI를 다시 띄우거나 스스로 끝내지 않고,
    // 다음에 켠 GUI·CLI가 다시 붙는다. 끄는 것은 /api/daemon/shutdown 또는 Ctrl+C·SIGTERM뿐이다.
    if daemon_only {
        // 데몬 온리 모드: 터미널 폴링 루프 시작 — 데몬 로그 + 프로세스 콘솔을 stderr에 표시
        let term_buf = daemon_log_buffer.clone();
        let term_sup = supervisor.clone();
        let term_cancel = shutdown_token.clone();
        tokio::spawn(async move {
            daemon_log::daemon_terminal_loop(term_buf, term_sup, term_cancel).await;
        });
    }

    // Graceful shutdown: Ctrl+C / SIGTERM 시 정리
    let registry_shutdown = client_registry.clone();
    let supervisor_shutdown = supervisor.clone();
    let ctrl_c_cancel = shutdown_token.clone();
    let stop_requested = stop_requested_token.clone();
    tokio::spawn(async move {
        // 종료 경로는 하나: Ctrl+C·SIGTERM, API(/api/daemon/shutdown, unregister?shutdown=true, 업데이터) 모두 여기로
        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("Shutdown signal received, cleaning up..."),
            _ = sigterm() => tracing::info!("SIGTERM received, cleaning up..."),
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


/// SIGTERM을 기다린다 (Unix). Windows에는 없으므로 끝나지 않는다
async fn sigterm() {
    #[cfg(unix)]
    {
        if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            term.recv().await;
            return;
        }
    }
    std::future::pending::<()>().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<DaemonArgs, String> {
        daemon_args(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn unknown_arguments_never_start_the_daemon() {
        assert!(matches!(parse(&[]), Ok(DaemonArgs::Run)));
        assert!(matches!(parse(&["--spawned"]), Ok(DaemonArgs::Run)));
        assert!(matches!(parse(&["--version"]), Ok(DaemonArgs::Version)));
        assert!(matches!(parse(&["--spawned", "--help"]), Ok(DaemonArgs::Help)));
        assert_eq!(parse(&["--verison"]).err().as_deref(), Some("--verison"));
        assert_eq!(parse(&["serve"]).err().as_deref(), Some("serve"));
    }
}
