//! Saba-Core 데몬 및 Discord Bot 프로세스 생명주기 관리.
//!
//! 책임 범위:
//!   - 데몬/봇 프로세스의 시작, 종료, 상태 확인
//!   - 바이너리 및 프로젝트 루트 경로 탐색
//!
//! ⚠️ [에이전트 주의] 이 모듈은 **blocking HTTP 클라이언트**를 사용한다.
//!   TUI에서 호출 시 반드시 `tokio::task::spawn_blocking`으로 감싼다.
//!   async DaemonClient(client.rs)와 혼동하지 말 것.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::config;

// ═══════════════════════════════════════════════════
//  §1. 플랫폼별 프로세스 헬퍼
// ═══════════════════════════════════════════════════

/// Windows에서 콘솔 창 없이 프로세스를 실행하는 헬퍼.
///
/// ⚠️ [에이전트 주의] `spawn_detached()`와 동시 사용 금지!
///   `CREATE_NO_WINDOW(0x08000000)`과 `DETACHED_PROCESS(0x00000008)`는
///   Microsoft 문서상 상호 배타적 플래그이다.
///   - 콘솔 숨김만 필요 → `no_window()`  (예: taskkill)
///   - 프로세스 분리 필요 → `spawn_detached()`  (예: 데몬 기동)
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

// ═══════════════════════════════════════════════════
//  §2. 경로 탐색
// ═══════════════════════════════════════════════════

/// 프로젝트(또는 설치) 루트 디렉토리 탐색.
///
/// 탐색 전략 (현재 디렉토리에서 상위 MAX_DEPTH단계까지):
///   Pass 1: `modules/` 디렉토리 또는 `saba-core.exe` 존재 → 설치/실행 루트
///   Pass 2: `Cargo.toml` 기반 폴백 → 개발 환경 workspace root
///   둘 다 없으면 → 현재 디렉토리 반환
///
/// ⚠️ [에이전트 주의] MAX_DEPTH 제한은 경험적 값.
///   깊은 디렉토리 구조에서는 실패할 수 있다.
///   릴리즈 환경에서는 Pass 1이 매칭되어야 정상.
pub fn find_project_root() -> anyhow::Result<PathBuf> {
    const MAX_DEPTH: usize = 5;
    let cwd = std::env::current_dir()?;

    // Pass 1: 실제 saba-chan 루트를 식별하는 확실한 마커로 먼저 탐색
    let mut candidate = cwd.clone();
    for _ in 0..MAX_DEPTH {
        if candidate.join("modules").is_dir()
            || candidate.join("saba-core.exe").exists()
        {
            return Ok(candidate);
        }
        match candidate.parent() {
            Some(p) => candidate = p.to_path_buf(),
            None => break,
        }
    }

    // Pass 2: Cargo.toml 기반 폴백 (개발 환경 전용)
    //   하위 크레이트가 아닌 최상위 Cargo.toml 우선
    let mut cargo_dirs = Vec::new();
    let mut scan = cwd.clone();
    for _ in 0..MAX_DEPTH {
        if scan.join("Cargo.toml").exists() {
            cargo_dirs.push(scan.clone());
        }
        match scan.parent() {
            Some(p) => scan = p.to_path_buf(),
            None => break,
        }
    }
    // 가장 상위 Cargo.toml 디렉토리 선택 (멤버 크레이트보다 workspace root 우선)
    if let Some(root) = cargo_dirs.last() {
        return Ok(root.clone());
    }

    Ok(cwd)
}

// ═══════════════════════════════════════════════════
//  §3. 데몬 생명주기
// ═══════════════════════════════════════════════════

/// Saba-Core 실행 여부 확인 (TCP 연결 프로브 — 외부 프로세스 없음).
pub fn check_daemon_running() -> bool {
    let port = config::get_ipc_port();
    let addr: std::net::SocketAddr = match format!("127.0.0.1:{}", port).parse() {
        Ok(a) => a,
        // ⚠️ [에이전트 주의] 포트 파싱 실패 = 설정 이상. false 반환이 안전.
        Err(_) => return false,
    };
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// Saba-Core 바이너리 경로 탐색.
///
/// ⚠️ [에이전트 주의] 후보 경로 추가 시 릴리즈/개발 양쪽 환경을 고려할 것.
///   릴리즈: self_dir, root/exe 에서 매칭
///   개발:   target/release, target/debug 에서 매칭
pub fn find_daemon_binary() -> anyhow::Result<PathBuf> {
    let root = find_project_root()?;
    let cwd = std::env::current_dir()?;
    let exe = if cfg!(target_os = "windows") { "saba-core.exe" } else { "saba-core" };

    let self_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let mut candidates = Vec::new();

    // 릴리즈: 현재 실행 파일 옆이 가장 확실한 후보
    if let Some(ref dir) = self_dir {
        candidates.push(dir.join(exe));
    }

    candidates.extend([
        root.join(exe),
        cwd.join(exe),
        // 개발 환경: Cargo 빌드 산출물
        root.join("target/release").join(exe),
        root.join("target/debug").join(exe),
        cwd.join("../target/release").join(exe),
        cwd.join("../target/debug").join(exe),
        // GUI bin/ 디렉토리 (GUI 빌드 산출물)
        root.join("saba-chan-gui/bin").join(exe),
        cwd.join("../saba-chan-gui/bin").join(exe),
    ]);

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    anyhow::bail!("Saba-Core binary not found. Searched: {}",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )
}

/// 프로세스를 분리(detach)하여 실행 — 부모 종료 후에도 유지.
///
/// ⚠️ [에이전트 주의]
///   - stdio를 null로 리다이렉트해 부모-자식 파이프를 끊는다.
///   - Windows: DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP 조합 사용.
///   - `no_window()`와 플래그 충돌하므로 함께 호출하지 않는다.
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

/// 데몬 시작 후 실제로 응답하는지 폴링 확인.
const DAEMON_STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(300);
const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

fn wait_for_daemon_startup() -> bool {
    let deadline = std::time::Instant::now() + DAEMON_STARTUP_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if check_daemon_running() {
            return true;
        }
        std::thread::sleep(DAEMON_STARTUP_POLL_INTERVAL);
    }
    false
}

/// Saba-Core 시작 — GUI와 동일한 환경변수 세팅.
pub fn start_daemon() -> anyhow::Result<String> {
    if check_daemon_running() {
        return Ok("✓ Saba-Core is already running".into());
    }

    let root = find_project_root()?;
    let binary = find_daemon_binary()?;

    let modules = config::get_modules_path()
        .unwrap_or_else(|_| {
            saba_chan_updater_lib::constants::resolve_modules_dir()
                .to_string_lossy()
                .into_owned()
        });
    let instances = config::get_instances_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| {
            saba_chan_updater_lib::constants::resolve_instances_dir()
                .to_string_lossy()
                .into_owned()
        });
    let lang = config::get_language().unwrap_or_else(|_| "en".into());

    let mut cmd = Command::new(&binary);
    cmd.arg("--spawned")
       .current_dir(&root)
       .env("RUST_LOG", "info")
       .env("SABA_MODULES_PATH", &modules)
       .env("SABA_INSTANCES_PATH", &instances)
       .env("SABA_LANG", &lang);

    spawn_detached(&mut cmd)?;

    // sleep(1) 대신 실제 TCP 프로브로 기동 확인
    if wait_for_daemon_startup() {
        Ok(format!("✓ Saba-Core started\n  modules: {}\n  instances: {}", modules, instances))
    } else {
        Ok(format!("⚠ Saba-Core spawned but not yet responding\n  modules: {}\n  instances: {}", modules, instances))
    }
}

/// Saba-Core 종료 — 데몬 API graceful shutdown 후, 실패 시 OS 강제 종료.
pub fn stop_daemon() -> anyhow::Result<String> {
    if !check_daemon_running() {
        return Ok("ℹ Saba-Core is not running".into());
    }

    // 1차: 데몬 API를 통한 graceful shutdown
    let base = config::get_ipc_base_url();
    let token_ipc = load_ipc_token();
    let client = build_daemon_client(3)?;

    let api_ok = with_token(
        client.post(format!("{}/api/daemon/shutdown", base)).json(&serde_json::json!({})),
        &token_ipc,
    ).send().map(|r| r.status().is_success()).unwrap_or(false);

    if api_ok {
        std::thread::sleep(Duration::from_secs(2));
        if !check_daemon_running() {
            return Ok("✓ Saba-Core stopped (graceful)".into());
        }
    }

    // 2차: graceful 실패 → OS 강제 종료
    force_kill_daemon()?;
    std::thread::sleep(Duration::from_secs(1));
    Ok("✓ Saba-Core stopped (forced)".into())
}

/// OS 수준에서 데몬 프로세스를 강제 종료.
///
/// ⚠️ [에이전트 주의] 프로세스 **이름** 기반으로 kill하므로,
///   동일 이름의 프로세스가 여러 개이면 전부 종료된다.
///   PID 기반 종료가 필요하면 데몬 API에서 PID를 반환받아야 한다.
fn force_kill_daemon() -> anyhow::Result<()> {
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/IM", "saba-core.exe", "/F"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut cmd);
        cmd.status()?;
    } else {
        Command::new("pkill")
            .args(["-f", "saba-core"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status()?;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════
//  §4. Blocking IPC 헬퍼 (프로세스 관리 전용)
// ═══════════════════════════════════════════════════
//
// ⚠️ [에이전트 주의] config.rs, client.rs 에도 유사한 IPC 헬퍼가 있다.
//   **의도적 분리**:
//     - client.rs    → async DaemonClient (TUI 렌더링, 서버/인스턴스 CRUD)
//     - config.rs → blocking, 설정 읽기/쓰기 전용
//     - process.rs   → blocking, 프로세스 생명주기 전용
//   하나로 통합하면 순환 의존이 발생한다.
//   새로운 IPC 호출이 필요하면 용도에 맞는 모듈에 추가할 것.

/// IPC 인증 토큰 로드.
fn load_ipc_token() -> Option<String> {
    let path = saba_chan_updater_lib::constants::token_file_path();
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// 데몬 API에 인증 헤더가 포함된 HTTP 클라이언트를 생성합니다.
fn build_daemon_client(timeout_secs: u64) -> anyhow::Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()?)
}

/// 요청에 IPC 토큰 헤더를 추가합니다.
fn with_token(req: reqwest::blocking::RequestBuilder, token: &Option<String>) -> reqwest::blocking::RequestBuilder {
    match token {
        Some(t) => req.header("X-Saba-Token", t),
        None => req,
    }
}

// ═══════════════════════════════════════════════════
//  §5. Discord Bot 생명주기
// ═══════════════════════════════════════════════════

/// 봇 시작에 필요한 설정(Discord 토큰, 모드, 언어) 로드.
fn load_bot_startup_config() -> anyhow::Result<(String, String, String, serde_json::Value)> {
    let settings = config::load_settings().unwrap_or_default();
    let bot_config = config::load_bot_config().unwrap_or_default();

    let discord_token = settings.get("discordToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Discord token not set — configure it in GUI"))?
        .to_string();
    let mode = bot_config.get("mode").and_then(|v| v.as_str()).unwrap_or("local").to_string();
    let lang = settings.get("language").and_then(|v| v.as_str()).unwrap_or("en").to_string();

    Ok((discord_token, mode, lang, bot_config))
}

/// 데몬에서 포터블 Node.js 경로를 요청합니다.
fn request_node_path(client: &reqwest::blocking::Client, base: &str, token: &Option<String>) -> anyhow::Result<String> {
    let req = with_token(
        client.post(format!("{}/api/node-env/setup", base)).json(&serde_json::json!({})),
        token,
    );
    let resp = req.send()?.json::<serde_json::Value>()?;
    Ok(resp.get("node_path").and_then(|v| v.as_str()).unwrap_or("node").to_string())
}

/// 봇 프로세스의 환경변수를 구성합니다.
fn build_bot_env(base: &str, lang: &str, discord_token: &str) -> serde_json::Map<String, serde_json::Value> {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let bot_config_path = data_dir.join("bot-config.json");
    let extensions_dir = saba_chan_updater_lib::constants::resolve_extensions_dir();

    let mut env_vars = serde_json::Map::new();
    env_vars.insert("IPC_BASE".into(), serde_json::json!(base));
    env_vars.insert("SABA_LANG".into(), serde_json::json!(lang));
    env_vars.insert("BOT_CONFIG_PATH".into(), serde_json::json!(bot_config_path.to_string_lossy()));
    env_vars.insert("SABA_EXTENSIONS_DIR".into(), serde_json::json!(extensions_dir.to_string_lossy()));
    env_vars.insert("DISCORD_TOKEN".into(), serde_json::json!(discord_token));
    env_vars
}

/// 봇 디렉토리와 index.js 경로를 탐색.
fn find_bot_paths() -> anyhow::Result<(PathBuf, PathBuf)> {
    let install_root = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let bot_dir = install_root.join("discord_bot");
    let index_path = bot_dir.join("index.js");
    if !index_path.exists() {
        anyhow::bail!("discord_bot/index.js not found at {}", index_path.display());
    }
    Ok((bot_dir, index_path))
}

/// Discord Bot 시작 — 데몬의 ext-process API 경유.
///
/// ⚠️ [에이전트 주의] 봇은 데몬이 관리하는 자식 프로세스다.
///   이 함수에서 직접 spawn하지 않고, 데몬 API에 실행 명령을 위임한다.
///   데몬이 미실행이면 이 호출도 실패한다.
pub fn start_bot() -> anyhow::Result<String> {
    let base = config::get_ipc_base_url();
    let token_ipc = load_ipc_token();

    let (discord_token, mode, lang, bot_config) = load_bot_startup_config()?;
    let client = build_daemon_client(30)?;
    let node_path = request_node_path(&client, &base, &token_ipc)?;
    let (bot_dir, index_path) = find_bot_paths()?;
    let env_vars = build_bot_env(&base, &lang, &discord_token);

    let body = serde_json::json!({
        "command": node_path,
        "args": [index_path.to_string_lossy()],
        "cwd": bot_dir.to_string_lossy(),
        "env": serde_json::Value::Object(env_vars),
        "meta": { "mode": mode },
    });

    let req = with_token(
        client.post(format!("{}/api/ext-process/discord-bot/start", base)).json(&body),
        &token_ipc,
    );
    let resp = req.send()?;

    if resp.status().is_success() {
        let prefix = bot_config.get("prefix").and_then(|v| v.as_str()).unwrap_or("!saba");
        Ok(format!("✓ Discord bot started (prefix: {})", prefix))
    } else {
        let status = resp.status();
        let body_text = resp.text().unwrap_or_default();
        anyhow::bail!("Failed to start bot: {} {}", status, body_text)
    }
}

/// Discord Bot 종료 — 데몬 API 경유 (POST /api/ext-process/discord-bot/stop)
pub fn stop_bot() -> anyhow::Result<String> {
    let base = config::get_ipc_base_url();
    let token_ipc = load_ipc_token();

    let client = build_daemon_client(5)?;

    let req = with_token(
        client.post(format!("{}/api/ext-process/discord-bot/stop", base)).json(&serde_json::json!({})),
        &token_ipc,
    );
    let resp = req.send()?;

    if resp.status().is_success() {
        Ok("✓ Discord bot stopped".into())
    } else {
        let status = resp.status();
        let body_text = resp.text().unwrap_or_default();
        anyhow::bail!("Failed to stop bot: {} {}", status, body_text)
    }
}

/// Discord Bot 실행 상태 확인 — 데몬 API 경유 (GET /api/ext-process/discord-bot/status)
pub fn check_bot_running() -> bool {
    let base = config::get_ipc_base_url();
    let token_ipc = load_ipc_token();

    let client = match build_daemon_client(2) {
        Ok(c) => c,
        Err(_) => return false,
    };

    with_token(client.get(format!("{}/api/ext-process/discord-bot/status", base)), &token_ipc)
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json::<serde_json::Value>().ok())
        .and_then(|v| v.get("status").and_then(|s| s.as_str().map(|s| s == "running")))
        .unwrap_or(false)
}
