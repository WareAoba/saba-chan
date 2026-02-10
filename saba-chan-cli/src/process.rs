use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::gui_config;

/// Windows에서 콘솔 창 없이 프로세스를 실행하는 헬퍼
#[cfg(target_os = "windows")]
fn no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
fn no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

/// 프로젝트 루트 디렉토리 찾기 (Cargo.toml 또는 config/instances.json 기준)
pub fn find_project_root() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let mut dir = cwd.clone();
    for _ in 0..5 {
        if dir.join("Cargo.toml").exists()
            || dir.join("config").join("instances.json").exists()
        {
            return Ok(dir);
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }
    Ok(cwd)
}

/// Daemon 실행 여부 확인 (TCP 연결만으로 판단 — 외부 프로세스 없음)
pub fn check_daemon_running() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:57474".parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Discord Bot (node.exe) 실행 여부 확인
pub fn check_bot_running() -> bool {
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("tasklist");
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        no_window(&mut cmd);
        cmd.output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("node.exe"))
            .unwrap_or(false)
    } else {
        Command::new("pgrep")
            .args(["-f", "discord_bot"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Daemon 바이너리 탐색
pub fn find_daemon_binary() -> anyhow::Result<PathBuf> {
    let root = find_project_root()?;
    let cwd = std::env::current_dir()?;
    let exe = if cfg!(target_os = "windows") { "core_daemon.exe" } else { "core_daemon" };

    let candidates = [
        root.join("target/release").join(exe),
        root.join("target/debug").join(exe),
        root.join(exe),
        cwd.join(exe),
        cwd.join("../target/release").join(exe),
        cwd.join("../target/debug").join(exe),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    anyhow::bail!("Daemon binary not found")
}

/// Discord Bot 디렉토리 탐색
pub fn find_bot_directory() -> anyhow::Result<PathBuf> {
    let root = find_project_root()?;
    let cwd = std::env::current_dir()?;

    let candidates = [
        root.join("discord_bot"),
        cwd.join("discord_bot"),
        cwd.join("../discord_bot"),
    ];

    for dir in &candidates {
        if dir.join("index.js").exists() {
            return Ok(dir.clone());
        }
    }

    anyhow::bail!("Discord bot directory not found")
}

/// 프로세스를 분리(detach)하여 실행 — 부모 종료 후에도 유지
fn spawn_detached(cmd: &mut Command) -> anyhow::Result<()> {
    cmd.stdin(Stdio::null())
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    cmd.spawn()?;
    Ok(())
}

/// Daemon 시작 — GUI와 동일한 환경변수 세팅
pub fn start_daemon() -> anyhow::Result<String> {
    let root = find_project_root()?;
    let binary = find_daemon_binary()?;

    if check_daemon_running() {
        return Ok("✓ Daemon is already running".into());
    }

    let modules = gui_config::get_modules_path()
        .unwrap_or_else(|_| root.join("modules").to_string_lossy().into());
    let instances = gui_config::get_instances_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| root.join("config/instances.json").to_string_lossy().into());
    let lang = gui_config::get_language().unwrap_or_else(|_| "en".into());

    let mut cmd = Command::new(&binary);
    cmd.current_dir(&root)
       .env("RUST_LOG", "info")
       .env("SABA_MODULES_PATH", &modules)
       .env("SABA_INSTANCES_PATH", &instances)
       .env("SABA_LANG", &lang);

    spawn_detached(&mut cmd)?;
    std::thread::sleep(Duration::from_secs(1));

    Ok(format!("✓ Daemon started\n  modules: {}\n  instances: {}", modules, instances))
}

/// Daemon 종료
pub fn stop_daemon() -> anyhow::Result<String> {
    if !check_daemon_running() {
        return Ok("ℹ Daemon is not running".into());
    }

    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/IM", "core_daemon.exe", "/F"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut cmd);
        cmd.status()?;
    } else {
        Command::new("pkill")
            .args(["-f", "core_daemon"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status()?;
    }

    std::thread::sleep(Duration::from_secs(1));
    Ok("✓ Daemon stopped".into())
}

/// Discord Bot 시작 — GUI settings.json에서 토큰 읽기
pub fn start_bot() -> anyhow::Result<String> {
    let bot_dir = find_bot_directory()?;

    if check_bot_running() {
        return Ok("✓ Discord bot is already running".into());
    }

    let token = gui_config::get_discord_token()?
        .ok_or_else(|| anyhow::anyhow!("Discord token not set — configure it in GUI"))?;
    let lang = gui_config::get_language().unwrap_or_else(|_| "en".into());
    let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());

    // bot-config.json을 discord_bot/ 폴더에 복사 (봇이 직접 읽음)
    let config = gui_config::load_bot_config()?;
    fs::write(
        bot_dir.join("bot-config.json"),
        serde_json::to_string_pretty(&config)?,
    )?;

    let mut cmd = Command::new("node");
    cmd.arg(bot_dir.join("index.js"))
       .current_dir(&bot_dir)
       .env("DISCORD_TOKEN", &token)
       .env("IPC_BASE", "http://127.0.0.1:57474")
       .env("SABA_LANG", &lang);

    spawn_detached(&mut cmd)?;
    std::thread::sleep(Duration::from_millis(500));

    Ok(format!("✓ Discord bot started (prefix: {})", prefix))
}

/// Discord Bot 종료
pub fn stop_bot() -> anyhow::Result<String> {
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/IM", "node.exe", "/F"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut cmd);
        let _ = cmd.status();
    } else {
        let _ = Command::new("pkill")
            .args(["-f", "discord_bot"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status();
    }

    std::thread::sleep(Duration::from_millis(500));
    Ok("✓ Discord bot stopped".into())
}
