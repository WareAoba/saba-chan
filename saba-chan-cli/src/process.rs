//! Saba-Core 데몬 프로세스 생명주기 관리.
//!
//! 책임 범위:
//!   - 데몬 프로세스의 시작, 종료, 상태 확인
//!   - 바이너리 및 프로젝트 루트 경로 탐색
//!
//! ⚠️ [에이전트 주의] 이 모듈은 blocking 호출(sleep, blocking HTTP)을 쓴다.
//!   TUI/async 코드에서는 `tokio::task::spawn_blocking`으로 감싼다.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::config;
use crate::ipc;

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
    let port = ipc::ipc_port();
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
    let instances = config::get_instances_path().to_string_lossy().into_owned();
    let lang = crate::cli_config::CliSettings::load().effective_language();

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
    let api_ok = ipc::blocking_request(
        reqwest::Method::POST,
        "/api/daemon/shutdown",
        Some(&serde_json::json!({})),
    )
    .is_ok();

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
    let (program, args) = force_kill_command();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    no_window(&mut cmd);
    cmd.status()?;
    Ok(())
}

/// 강제 종료 명령. 프로세스 이름이 정확히 일치하는 것만 대상으로 한다
/// (`pkill -f`는 명령줄 부분 일치라 `saba-core`를 인자로 가진 다른 프로세스까지 죽인다).
fn force_kill_command() -> (&'static str, &'static [&'static str]) {
    if cfg!(target_os = "windows") {
        ("taskkill", &["/IM", "saba-core.exe", "/F"])
    } else {
        ("pkill", &["-x", "saba-core"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_kill_matches_exact_process_name_only() {
        let (program, args) = force_kill_command();
        if cfg!(target_os = "windows") {
            assert_eq!(program, "taskkill");
            assert_eq!(args, &["/IM", "saba-core.exe", "/F"]);
        } else {
            assert_eq!(program, "pkill");
            assert!(!args.contains(&"-f"), "pkill -f matches whole command lines");
            assert!(args.contains(&"-x"));
        }
    }
}
