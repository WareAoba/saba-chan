//! 업데이터 CLI 모드
//!
//! `saba-chan-updater --cli <command>` 로 실행 시
//! GUI 윈도우 없이 터미널에서 동작합니다.
//!
//! ## 사용법
//! ```text
//! saba-chan-updater --cli check                    # 업데이트 확인
//! saba-chan-updater --cli check --json             # JSON 출력
//! saba-chan-updater --cli check --notify           # OS 알림용 출력
//! saba-chan-updater --cli download                 # 전체 다운로드
//! saba-chan-updater --cli download <component>     # 특정 컴포넌트 다운로드
//! saba-chan-updater --cli apply                    # 업데이트 적용
//! saba-chan-updater --cli status                   # 상태 표시
//! saba-chan-updater --cli install                  # 전체 설치
//! saba-chan-updater --cli install <component>      # 특정 컴포넌트 설치
//! saba-chan-updater --cli install-status           # 설치 상태
//! saba-chan-updater --cli install-progress         # 설치 진행
//! saba-chan-updater --cli config                   # 설정 표시
//! saba-chan-updater --cli config set <key> <value> # 설정 변경
//! saba-chan-updater --cli help                     # 도움말
//! ```
//!
//! ## 종료 코드 (check --json / check --notify)
//! - `0` — 업데이트 있음
//! - `1` — 에러
//! - `2` — 업데이트 없음 (최신 상태)

use std::sync::Arc;
use tokio::sync::RwLock;

use saba_chan_updater_lib::{Component, UpdateManager};
use crate::config::{load_updater_config, set_config_value};

/// CLI 모드 실행 — `--cli` 이후의 인자를 받아 처리
pub fn run_cli(cli_args: Vec<String>) {
    // Windows release 빌드는 subsystem "windows"이므로 콘솔이 없음
    // CLI 모드에서는 부모 프로세스의 콘솔에 attach하거나 새로 할당
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::Console::{AttachConsole, AllocConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                // 부모 콘솔이 없으면 (spawn pipe 등) 새로 할당 시도
                let _ = AllocConsole();
            }
        }
    }

    // 로깅 초기화 (CLI 모드에서는 stderr로 출력하여 stdout의 JSON을 오염시키지 않음)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async move {
        if let Err(e) = run_cli_async(cli_args).await {
            eprintln!("✗ Error: {}", e);
            std::process::exit(1);
        }
    });
}

async fn run_cli_async(args: Vec<String>) -> anyhow::Result<()> {
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    if args_ref.is_empty() || matches!(args_ref[0], "help" | "--help" | "-h") {
        print_help();
        return Ok(());
    }

    if matches!(args_ref[0], "--version" | "-V") {
        println!("saba-chan-updater {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // 설정 로드
    let cfg = load_updater_config()?;
    if cfg.github_owner.is_empty() {
        eprintln!("⚠ github_owner가 설정되지 않았습니다.");
        eprintln!("  saba-chan-updater --cli config set github_owner <owner>");
        if args_ref[0] != "config" {
            std::process::exit(1);
        }
    }

    let modules_dir = resolve_modules_dir();
    let manager = Arc::new(RwLock::new(UpdateManager::new(cfg, &modules_dir)));

    match args_ref[0] {
        "check" => cmd_check(manager, &args_ref[1..]).await,
        "status" => cmd_status(manager).await,
        "download" => cmd_download(manager, &args_ref[1..]).await,
        "apply" => cmd_apply(manager).await,
        "silent" => cmd_silent(manager, &args_ref[1..]).await,
        "install" => cmd_install(manager, &args_ref[1..]).await,
        "install-status" => cmd_install_status(manager).await,
        "install-progress" => cmd_install_progress(manager).await,
        "config" => cmd_config(&args_ref[1..]),
        other => {
            eprintln!("✗ Unknown command: {}", other);
            eprintln!("  Run 'saba-chan-updater --cli help' for usage.");
            std::process::exit(1);
        }
    }
}

// ═══════════════════════════════════════════════════════
// 명령어 핸들러
// ═══════════════════════════════════════════════════════

async fn cmd_check(manager: Arc<RwLock<UpdateManager>>, flags: &[&str]) -> anyhow::Result<()> {
    let json_mode = flags.contains(&"--json");
    let notify_mode = flags.contains(&"--notify");

    let result = saba_chan_updater_lib::scheduler::check_once(&manager).await;

    if json_mode {
        println!("{}", saba_chan_updater_lib::scheduler::result_to_json(&result));
        std::process::exit(saba_chan_updater_lib::scheduler::exit_code(&result));
    }

    if notify_mode {
        if result.updates_available > 0 {
            println!(
                "UPDATE_AVAILABLE|{}|{}",
                result.updates_available,
                result.update_names.join(",")
            );
            std::process::exit(0);
        } else if result.error.is_some() {
            eprintln!("CHECK_ERROR|{}", result.error.unwrap());
            std::process::exit(1);
        } else {
            std::process::exit(2);
        }
    }

    // 대화형 모드
    if let Some(err) = result.error {
        eprintln!("✗ Check failed: {}", err);
        return Ok(());
    }

    let mgr = manager.read().await;
    let status = mgr.get_status();

    if result.updates_available > 0 {
        println!("📦 {} update(s) available:", result.updates_available);
        println!();
        print_component_table(&status.components);
        println!();
        println!("💡 Run with '--cli download' to download updates.");
    } else if status.components.is_empty() {
        println!("⚠ No components found. Is the GitHub release configured correctly?");
    } else {
        println!("✓ All components are up to date.");
        println!();
        print_component_table(&status.components);
    }

    Ok(())
}

async fn cmd_status(manager: Arc<RwLock<UpdateManager>>) -> anyhow::Result<()> {
    let mgr = manager.read().await;
    let status = mgr.get_status();

    println!("📋 Update Status");
    println!("─────────────────────────────────────────────");
    println!("  Last check: {}", status.last_check.as_deref().unwrap_or("never"));
    println!("  Next check: {}", status.next_check.as_deref().unwrap_or("-"));
    if let Some(ref err) = status.error {
        println!("  Error:      {}", err);
    }
    println!();

    if status.components.is_empty() {
        println!("  No component data. Run '--cli check' first.");
    } else {
        print_component_table(&status.components);
    }

    Ok(())
}

async fn cmd_download(
    manager: Arc<RwLock<UpdateManager>>,
    args: &[&str],
) -> anyhow::Result<()> {
    {
        let mut mgr = manager.write().await;
        let status = mgr.get_status();
        if status.components.is_empty() {
            println!("⏳ Checking for updates first...");
            mgr.check_for_updates().await?;
        }
    }

    if let Some(key) = args.first() {
        let component = Component::from_manifest_key(key);
        println!("⏳ Downloading {}...", component.display_name());
        let mut mgr = manager.write().await;
        let asset = mgr.download_component(&component).await?;
        println!("✓ Downloaded: {}", asset);
    } else {
        println!("⏳ Downloading all available updates...");
        let mut mgr = manager.write().await;
        let downloaded = mgr.download_available_updates().await?;
        if downloaded.is_empty() {
            println!("  No updates to download.");
        } else {
            println!("✓ Downloaded {} component(s):", downloaded.len());
            for d in &downloaded {
                println!("  • {}", d);
            }
        }
    }

    Ok(())
}

async fn cmd_apply(manager: Arc<RwLock<UpdateManager>>) -> anyhow::Result<()> {
    println!("⏳ Applying downloaded updates...");
    let mut mgr = manager.write().await;
    let applied = mgr.apply_updates().await?;

    if !applied.is_empty() {
        println!("✓ Applied {} update(s):", applied.len());
        for a in &applied {
            println!("  • {}", a);
        }
    } else {
        println!("  No updates to apply. Run '--cli download' first.");
    }

    Ok(())
}

/// 사일런트 모드 — GUI/CLI 재시작이 불필요한 업데이트를 자동으로
/// 체크→다운로드→적용까지 수행합니다.
///
/// GUI/CLI 컴포넌트 업데이트가 있으면 `requires_gui_updater: true`를 반환하여
/// 호출 측에서 업데이터 GUI를 스폰하도록 안내합니다.
///
/// ## JSON 출력 (--json)
/// ```json
/// {
///   "ok": true,
///   "checked": 5,
///   "downloaded": 2,
///   "applied": ["Module: minecraft", "Core Daemon"],
///   "skipped_self_update": ["GUI", "CLI"],
///   "requires_gui_updater": true,
///   "errors": []
/// }
/// ```
async fn cmd_silent(
    manager: Arc<RwLock<UpdateManager>>,
    flags: &[&str],
) -> anyhow::Result<()> {
    let json_mode = flags.contains(&"--json");

    // 1. 체크
    if !json_mode { eprintln!("⏳ [Silent] Checking for updates..."); }
    {
        let mut mgr = manager.write().await;
        mgr.check_for_updates().await?;
    }

    let (targets, self_updates) = {
        let mgr = manager.read().await;
        let status = mgr.get_status();
        let mut targets = Vec::new();
        let mut self_updates = Vec::new();

        for c in &status.components {
            if !c.update_available { continue; }
            match &c.component {
                Component::Gui | Component::Cli => {
                    self_updates.push(c.component.display_name());
                }
                _ => {
                    targets.push(c.component.clone());
                }
            }
        }
        (targets, self_updates)
    };

    let checked = {
        let mgr = manager.read().await;
        mgr.get_status().components.len()
    };

    if targets.is_empty() && self_updates.is_empty() {
        if json_mode {
            let out = serde_json::json!({
                "ok": true,
                "checked": checked,
                "downloaded": 0,
                "applied": Vec::<String>::new(),
                "skipped_self_update": Vec::<String>::new(),
                "requires_gui_updater": false,
                "errors": Vec::<String>::new(),
            });
            println!("{}", serde_json::to_string(&out).unwrap());
        } else {
            println!("✓ All components are up to date.");
        }
        std::process::exit(2); // exit code 2 = no updates
    }

    // 2. 다운로드 (비-셀프 업데이트 컴포넌트만)
    let mut downloaded_count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    {
        let mut mgr = manager.write().await;
        for comp in &targets {
            if !json_mode {
                eprintln!("⏳ [Silent] Downloading {}...", comp.display_name());
            }
            match mgr.download_component(comp).await {
                Ok(_) => downloaded_count += 1,
                Err(e) => {
                    let msg = format!("Download {} failed: {}", comp.display_name(), e);
                    if !json_mode { eprintln!("✗ {}", msg); }
                    errors.push(msg);
                }
            }
        }
    }

    // 3. 적용
    let mut applied_names: Vec<String> = Vec::new();
    if downloaded_count > 0 {
        let mut mgr = manager.write().await;
        if !json_mode { eprintln!("⏳ [Silent] Applying updates..."); }

        for comp in &targets {
            match mgr.apply_single_component(comp).await {
                Ok(result) if result.success => {
                    applied_names.push(comp.display_name());
                }
                Ok(result) => {
                    errors.push(format!("Apply {} failed: {}", comp.display_name(), result.message));
                }
                Err(e) => {
                    errors.push(format!("Apply {} error: {}", comp.display_name(), e));
                }
            }
        }
    }

    let requires_gui_updater = !self_updates.is_empty();

    if json_mode {
        let out = serde_json::json!({
            "ok": errors.is_empty(),
            "checked": checked,
            "downloaded": downloaded_count,
            "applied": applied_names,
            "skipped_self_update": self_updates,
            "requires_gui_updater": requires_gui_updater,
            "errors": errors,
        });
        println!("{}", serde_json::to_string(&out).unwrap());
    } else {
        if !applied_names.is_empty() {
            println!("✓ Applied {} update(s):", applied_names.len());
            for a in &applied_names {
                println!("  • {}", a);
            }
        }
        if requires_gui_updater {
            println!();
            println!("⚠ GUI/CLI updates require the updater GUI:");
            for s in &self_updates {
                println!("  • {}", s);
            }
            println!("  Run 'saba-chan-updater' (without --cli) to apply these.");
        }
        if !errors.is_empty() {
            eprintln!();
            eprintln!("✗ {} error(s):", errors.len());
            for e in &errors {
                eprintln!("  • {}", e);
            }
        }
    }

    // exit 0 = updates applied, exit 2 = nothing applied
    if !applied_names.is_empty() || requires_gui_updater {
        std::process::exit(0);
    } else {
        std::process::exit(2);
    }
}

async fn cmd_install(
    manager: Arc<RwLock<UpdateManager>>,
    args: &[&str],
) -> anyhow::Result<()> {
    if let Some(key) = args.first() {
        let component = Component::from_manifest_key(key);
        println!("⏳ Installing {}...", component.display_name());
        let mut mgr = manager.write().await;
        mgr.install_component(&component).await?;
        println!("✓ {} installed successfully.", component.display_name());
    } else {
        println!("⏳ Starting full installation...");
        println!();
        let mut mgr = manager.write().await;
        let progress = mgr.fresh_install(None).await?;

        println!();
        if progress.errors.is_empty() {
            println!("✓ Installation complete! ({} components)", progress.installed_components.len());
        } else {
            println!("⚠ Installation finished with {} error(s):", progress.errors.len());
            for e in &progress.errors {
                eprintln!("  • {}", e);
            }
        }
        if !progress.installed_components.is_empty() {
            println!();
            println!("  Installed:");
            for c in &progress.installed_components {
                println!("  ✓ {}", c);
            }
        }
    }

    Ok(())
}

async fn cmd_install_status(manager: Arc<RwLock<UpdateManager>>) -> anyhow::Result<()> {
    let mgr = manager.read().await;
    let status = mgr.get_install_status();

    println!("📋 Install Status");
    println!("─────────────────────────────────────────────");
    println!(
        "  Fresh install: {}",
        if status.is_fresh_install { "Yes" } else { "No" }
    );
    println!(
        "  Components:    {}/{} installed",
        status.installed_components, status.total_components
    );
    println!();

    for c in &status.components {
        let sym = if c.installed { "✓" } else { "✗" };
        println!("  {} {}", sym, c.display_name);
    }

    Ok(())
}

async fn cmd_install_progress(manager: Arc<RwLock<UpdateManager>>) -> anyhow::Result<()> {
    let mgr = manager.read().await;
    match mgr.get_install_progress() {
        Some(p) => {
            println!("📋 Install Progress");
            println!("─────────────────────────────────────────────");
            println!(
                "  Status:  {}",
                if p.complete { "Complete" } else { "In progress" }
            );
            println!("  Progress: {}/{}", p.done, p.total);
            if let Some(ref cur) = p.current_component {
                println!("  Current: {}", cur);
            }
            if !p.installed_components.is_empty() {
                println!("  Installed:");
                for c in &p.installed_components {
                    println!("    ✓ {}", c);
                }
            }
            if !p.errors.is_empty() {
                println!("  Errors:");
                for e in &p.errors {
                    eprintln!("    ✗ {}", e);
                }
            }
        }
        None => {
            println!("  No install in progress.");
        }
    }

    Ok(())
}

fn cmd_config(args: &[&str]) -> anyhow::Result<()> {
    match args.first().copied() {
        Some("set") if args.len() >= 3 => {
            let key = args[1];
            let value = args[2..].join(" ");
            set_config_value(key, &value)?;
            println!("✓ {} = {}", key, value);
        }
        Some("set") => {
            eprintln!("Usage: --cli config set <key> <value>");
            eprintln!("Keys: github_owner, github_repo, check_interval_hours, auto_download,");
            eprintln!("      auto_apply, include_prerelease, install_root, api_base_url");
        }
        Some("--json") => {
            // JSON 출력 모드 (GUI/CLI 프로세스 간 통신용)
            let cfg = load_updater_config()?;
            let json = serde_json::json!({
                "enabled": cfg.enabled,
                "github_owner": cfg.github_owner,
                "github_repo": cfg.github_repo,
                "check_interval_hours": cfg.check_interval_hours,
                "auto_download": cfg.auto_download,
                "auto_apply": cfg.auto_apply,
                "include_prerelease": cfg.include_prerelease,
                "install_root": cfg.install_root,
                "api_base_url": cfg.api_base_url,
            });
            println!("{}", serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string()));
        }
        _ => {
            let cfg = load_updater_config()?;
            println!("📋 Updater Configuration");
            println!("─────────────────────────────────────────────");
            println!("  enabled:              {}", cfg.enabled);
            println!("  github_owner:         {}", if cfg.github_owner.is_empty() { "(not set)" } else { &cfg.github_owner });
            println!("  github_repo:          {}", cfg.github_repo);
            println!("  check_interval_hours: {}", cfg.check_interval_hours);
            println!("  auto_download:        {}", cfg.auto_download);
            println!("  auto_apply:           {}", cfg.auto_apply);
            println!("  include_prerelease:   {}", cfg.include_prerelease);
            println!("  install_root:         {}", cfg.install_root.as_deref().unwrap_or("(auto — next to executable)"));
            println!();
            println!("  Config: embedded defaults (no config file)");
            println!();
            println!("  Change with: --cli config set <key> <value>");
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════
// 유틸리티
// ═══════════════════════════════════════════════════════

fn print_component_table(components: &[saba_chan_updater_lib::ComponentVersion]) {
    println!(
        "  {:<22} {:<12} {:<12} {:<8} {:<8}",
        "Component", "Current", "Latest", "Update", "DL'd"
    );
    println!("  {}", "─".repeat(66));
    for c in components {
        let latest = c.latest_version.as_deref().unwrap_or("-");
        let upd = if c.update_available { "⬆ yes" } else { "✓ no" };
        let dl = if c.downloaded { "✓" } else { "-" };
        let installed = if c.installed { "" } else { " [not installed]" };
        println!(
            "  {:<22} {:<12} {:<12} {:<8} {:<8}{}",
            c.component.display_name(),
            c.current_version,
            latest,
            upd,
            dl,
            installed,
        );
    }
}

fn print_help() {
    println!("saba-chan-updater — Saba-chan Updater / Installer");
    println!();
    println!("USAGE:");
    println!("  saba-chan-updater --cli <command> [args...]    CLI mode");
    println!("  saba-chan-updater [--test ...]                  GUI mode");
    println!();
    println!("CLI COMMANDS:");
    println!("  check                       Check for available updates");
    println!("  check --json                Check and output JSON");
    println!("  check --notify              Check and output notification format");
    println!("  status                      Show current update status");
    println!("  download [component]        Download updates (all or specific)");
    println!("  apply                       Apply downloaded updates");
    println!("  silent [--json]             Auto check+download+apply (non-restart only)");
    println!("  install [component]         Install components (all or specific)");
    println!("  install-status              Show installation status");
    println!("  install-progress            Show install progress");
    println!("  config                      Show updater configuration");
    println!("  config set <key> <value>    Change a config value");
    println!("  help                        This help message");
    println!();
    println!("COMPONENT KEYS:");
    println!("  saba-core                    Core daemon process");
    println!("  cli                         CLI tool");
    println!("  gui                         GUI application");
    println!("  module-<name>               Server module (e.g., module-minecraft)");
    println!();
    println!("EXAMPLES:");
    println!("  saba-chan-updater --cli check");
    println!("  saba-chan-updater --cli download saba-core");
    println!("  saba-chan-updater --cli install");
    println!("  saba-chan-updater --cli config set github_owner myuser");
}

fn resolve_modules_dir() -> String {
    let p = saba_chan_updater_lib::constants::resolve_modules_dir();
    if !p.exists() {
        let _ = std::fs::create_dir_all(&p);
    }
    p.to_string_lossy().to_string()
}
