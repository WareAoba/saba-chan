//! 데몬이 뜨기 전에 필요한 것 — 포트, 언어, 데몬 실행·종료
//!
//! 인터페이스는 데몬 API만 쓴다. 예외는 데몬을 찾고 띄우는 이 파일뿐이다:
//! - `config.json`의 `gui.ipcPort`·`gui.language` 읽기 (데몬과 같은 우선순위: env > 파일 > 기본값)
//! - `saba-core` 실행 파일 찾기와 분리 실행
//! - 정상 종료가 안 될 때 그 PID만 강제 종료

use crate::error::{failed, Result};
use saba_chan_updater_lib::{config_file::ConfigFile, constants};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn gui_section() -> serde_json::Value {
    ConfigFile::default_location().section("gui")
}

/// 데몬 포트: `SABA_IPC_PORT` > `config.json` gui.ipcPort > 기본값 (데몬 main.rs와 같은 순서)
pub fn ipc_port() -> u16 {
    std::env::var("SABA_IPC_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| gui_section().get("ipcPort").and_then(|v| v.as_u64()).map(|p| p as u16))
        .unwrap_or(constants::DEFAULT_IPC_PORT)
}

/// 표시 언어: `--lang` > `SABA_LANG` > 앱 설정 언어 > 시스템 로캘 > en
pub fn language(flag: Option<&str>) -> String {
    flag.map(str::to_string)
        .or_else(|| std::env::var("SABA_LANG").ok())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            gui_section()
                .get("language")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .or_else(|| {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .find_map(|k| std::env::var(k).ok())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en".into())
}

/// `saba-core` 실행 파일. 설치본은 CLI 옆, 개발 중에는 워크스페이스 target/.
pub fn daemon_binary() -> Option<PathBuf> {
    let exe = format!("saba-core{}", std::env::consts::EXE_SUFFIX);
    if let Some(p) = std::env::var_os("SABA_CORE_PATH").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let self_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
    let mut candidates = Vec::new();
    if let Some(dir) = &self_dir {
        candidates.push(dir.join(&exe));
        // target/<profile>/saba-chan-cli → 같은 profile의 saba-core
        if let Some(target) = dir.parent() {
            candidates.push(target.join("release").join(&exe));
            candidates.push(target.join("debug").join(&exe));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// 데몬을 분리 실행한다 (CLI가 끝나도 살아 있게)
/// `port`는 CLI가 붙을 포트 (`--port`까지 반영된 값)
pub fn spawn_daemon(port: u16) -> Result<PathBuf> {
    let binary = daemon_binary().ok_or_else(|| failed(t!("daemon.binary_not_found")))?;
    let mut cmd = Command::new(&binary);
    cmd.arg("--spawned")
        .env("SABA_IPC_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = binary.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map_err(|e| failed(t!("daemon.spawn_failed", error = e)))?;
    Ok(binary)
}

/// 정상 종료가 안 된 데몬을 PID로만 끝낸다 (이름으로 죽이면 다른 사용자의 데몬까지 죽는다)
pub fn kill_pid(pid: u64) -> Result<()> {
    let pid = pid.to_string();
    let status = if cfg!(windows) {
        // /T 없이: 게임 서버는 saba-runner가 붙들고 있다
        Command::new("taskkill")
            .args(["/PID", &pid, "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    } else {
        Command::new("kill")
            .args(["-KILL", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(failed(format!("kill {} exited with {}", pid, s))),
        Err(e) => Err(failed(e.to_string())),
    }
}

pub const START_TIMEOUT: Duration = Duration::from_secs(20);
pub const STOP_TIMEOUT: Duration = Duration::from_secs(20);
