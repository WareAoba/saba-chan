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

/// 붙을 데몬 포트: 설정 포트에 데몬이 없는데 실행 중인 데몬이 남긴 기록(`daemon.lock.owner`)의 포트에서
/// 그 데몬(같은 pid)이 답하면 그쪽이다. 설정에서 포트를 바꾸고 데몬을 아직 다시 켜지 않았을 때 그 데몬을 끄고
/// 다시 켤 수 있게 한다 (L62)
pub fn live_port(configured: u16) -> u16 {
    let record = saba_chan_updater_lib::constants::resolve_data_dir().join("daemon.lock.owner");
    let record = std::fs::read_to_string(record).ok().and_then(|t| recorded(&t));
    choose_port(configured, record, daemon_pid_at)
}

/// 데몬 잠금 기록(`pid=…\naddr=127.0.0.1:포트`, src/instance_lock.rs)의 pid와 포트
fn recorded(text: &str) -> Option<(u32, u16)> {
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(str::trim);
    let pid = field("pid=")?.parse().ok()?;
    let port = field("addr=")?.rsplit_once(':')?.1.parse().ok()?;
    Some((pid, port))
}

fn choose_port(configured: u16, record: Option<(u32, u16)>, daemon_at: impl Fn(u16) -> Option<u32>) -> u16 {
    match record {
        Some((pid, old)) if old != configured && daemon_at(configured).is_none() && daemon_at(old) == Some(pid) => old,
        _ => configured,
    }
}

/// 이 포트에서 답하는 사바쨩 데몬의 pid (토큰이 필요 없는 `/health`로 묻는다). 데몬을 띄우기 전이라 짧게 끝낸다
fn daemon_pid_at(port: u16) -> Option<u32> {
    use std::io::{Read, Write};
    let limit = Duration::from_millis(300);
    let mut stream = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), limit).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream.set_write_timeout(Some(limit)).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut raw = Vec::new();
    // 응답은 작다: 그보다 길면 데몬이 아니다
    stream.take(64 * 1024).read_to_end(&mut raw).ok()?;
    health_pid(&String::from_utf8_lossy(&raw))
}

/// `/health` HTTP 응답에서 데몬 pid. 200이고 `ok:true`인 JSON일 때만
fn health_pid(response: &str) -> Option<u32> {
    let (head, body) = response.split_once("\r\n\r\n")?;
    if head.split_whitespace().nth(1) != Some("200") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        return None;
    }
    v.get("pid")?.as_u64()?.try_into().ok()
}

/// 사용자가 직접 고른 언어: `--lang` > `SABA_LANG`
pub fn requested_language(flag: Option<&str>) -> Option<String> {
    flag.map(str::to_string)
        .or_else(|| std::env::var("SABA_LANG").ok())
        .filter(|s| !s.is_empty())
}

/// 표시 언어: `--lang` > `SABA_LANG` > 앱 설정 언어 > 시스템 로캘 > en
pub fn language(flag: Option<&str>) -> String {
    requested_language(flag)
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
                .filter(|s| !s.is_empty())
                // Windows는 보통 이 변수가 없다: 데몬·GUI처럼 OS 로캘을 묻는다
                .or_else(sys_locale::get_locale)
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
pub fn spawn_daemon(port: u16) -> Result<std::process::Child> {
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
    spawn_detached(&mut cmd).map_err(|e| failed(t!("daemon.spawn_failed", error = e)))
}

/// 이 프로세스의 표준 입출력 핸들을 물려주지 않고 띄운다 (자식의 입출력은 `cmd`에 정한 대로).
/// Windows는 상속 가능한 핸들을 모두 자식에게 넘긴다: CLI의 출력이 부른 쪽의 파이프면 데몬이 그 파이프를 쥐어,
/// CLI가 끝나도 부른 쪽은 출력의 끝을 기다린다
fn spawn_detached(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    let _no_inherit = StdNoInherit::new();
    cmd.spawn()
}

/// 표준 입출력 핸들의 상속 표시를 잠시 지우고, 사라질 때 되돌린다
#[cfg(windows)]
struct StdNoInherit(Vec<windows_sys::Win32::Foundation::HANDLE>);

#[cfg(windows)]
impl StdNoInherit {
    fn new() -> Self {
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        let mut cleared = Vec::new();
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = unsafe { GetStdHandle(which) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            let mut flags = 0;
            // stdout과 stderr이 같은 핸들이면 두 번째는 이미 지워져 있다
            if unsafe { GetHandleInformation(handle, &mut flags) } != 0
                && flags & HANDLE_FLAG_INHERIT != 0
                && unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } != 0
            {
                cleared.push(handle);
            }
        }
        Self(cleared)
    }
}

#[cfg(windows)]
impl Drop for StdNoInherit {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
        for &handle in &self.0 {
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        }
    }
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
/// 데몬의 종료 hook 한도(30초, src/main.rs)보다 길게 기다린다: 그 전에 강제 종료를 묻지 않는다
pub const STOP_TIMEOUT: Duration = Duration::from_secs(40);

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};

    /// 출력이 파이프인 CLI(스크립트, `| more`)가 데몬을 띄워도 데몬이 그 파이프를 쥐지 않는다:
    /// 쥐면 CLI가 끝나도 부른 쪽이 출력의 끝을 받지 못해 `daemon start`가 끝나지 않는 것처럼 보인다
    #[test]
    fn a_detached_child_does_not_hold_our_output_pipe() {
        // 표준 출력 핸들은 프로세스 전체가 함께 쓴다: 바꾸는 동안 다른 시험과 시험 틀의 출력이 이 파이프로 새거나
        // 닫힌 핸들에 쓰다 멈춘다 (L71). 이 시험만 도는 자식 프로세스에서 바꾼다
        const ALONE: &str = "SABA_TEST_STD_HANDLE_ALONE";
        if std::env::var_os(ALONE).is_none() {
            let out = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "bootstrap::tests::a_detached_child_does_not_hold_our_output_pipe", "--test-threads=1"])
                .env(ALONE, "1")
                .stdin(Stdio::null())
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success() && text.contains("1 passed"), "{text}{}", String::from_utf8_lossy(&out.stderr));
            return;
        }
        let (mut reader, writer) = std::io::pipe().unwrap();
        let raw = writer.as_raw_handle() as HANDLE;
        // 부른 쪽이 넘겨준 파이프처럼 상속 가능하게 해서 표준 출력으로 둔다
        unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        let old = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, raw) };
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "6", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = spawn_detached(&mut cmd);
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, old) };
        drop(writer);
        let mut child = child.expect("ping starts");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            let _ = tx.send(());
        });
        let closed = rx.recv_timeout(Duration::from_secs(2)).is_ok();
        let _ = child.kill();
        let _ = child.wait();
        assert!(closed, "the child still holds the write end of our output pipe");
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;

    /// 설정 파일의 포트를 바꾼 뒤에도 옛 포트에서 도는 데몬을 찾는다 (L62).
    /// 설정 포트에 데몬이 없고, 기록된 포트에서 기록된 그 데몬(pid)이 답할 때만 그쪽으로 붙는다
    #[test]
    fn a_daemon_still_on_its_old_port_is_found_from_its_lock_record() {
        assert_eq!(recorded("pid=42\naddr=127.0.0.1:57474\n"), Some((42, 57474)));
        assert_eq!(recorded("pid=42\n"), None);
        assert_eq!(recorded("addr=127.0.0.1:57474\n"), None);
        assert_eq!(recorded("pid=42\naddr=garbage\n"), None);

        // 포트마다 답하는 데몬의 pid
        let up =
            |live: &'static [(u16, u32)]| move |p: u16| live.iter().find(|(port, _)| *port == p).map(|(_, pid)| *pid);
        assert_eq!(choose_port(58000, Some((42, 57474)), up(&[(57474, 42)])), 57474);
        assert_eq!(
            choose_port(58000, Some((42, 57474)), up(&[(58000, 7), (57474, 42)])),
            58000
        );
        assert_eq!(choose_port(58000, Some((42, 57474)), up(&[])), 58000);
        assert_eq!(choose_port(58000, None, up(&[(57474, 42)])), 58000);
        // 옛 포트를 이제 다른 데몬(다른 pid)이 쓰면 붙지 않는다
        assert_eq!(choose_port(58000, Some((42, 57474)), up(&[(57474, 99)])), 58000);
    }

    /// 포트에서 답하는 것이 사바쨩 데몬인지는 `/health` 응답으로 본다: 연결만 받는 다른 프로그램은 아니다
    #[test]
    fn only_a_daemon_health_answer_counts() {
        let ok = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"ok\":true,\"version\":\"0.1.9\",\"pid\":42,\"token\":\"missing\"}";
        assert_eq!(health_pid(ok), Some(42));
        assert_eq!(
            health_pid("HTTP/1.1 404 Not Found\r\n\r\n{\"ok\":true,\"pid\":42}"),
            None
        );
        assert_eq!(health_pid("HTTP/1.1 200 OK\r\n\r\n<html>hello</html>"), None);
        assert_eq!(health_pid("HTTP/1.1 200 OK\r\n\r\n{\"ok\":false,\"pid\":42}"), None);
        assert_eq!(health_pid(""), None);
    }
}
