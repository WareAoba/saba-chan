use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;
use anyhow::Result;

#[cfg(not(target_os = "windows"))]
use nix::libc;

#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("process {pid} not found")]
    NotFound { pid: u32 },
    #[error("lock poisoned")]
    LockPoisoned,
}

/// Check if a process with the given PID is alive. Cross-platform helper.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess};
        use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::minwinbase::STILL_ACTIVE;

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            // 끝난 프로세스도 누가 핸들을 쥐고 있으면 열린다: 종료 코드로 끝났는지 본다
            let mut code = 0;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok == 0 || code == STILL_ACTIVE
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // kill(pid, 0) checks process existence without actually sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

/// 기록과 실제 프로세스 시작 시각의 차이로 볼 여유 (시계·기록 시점 차이)
const START_TIME_SLACK_SECS: u64 = 10;

/// 프로세스가 시작된 시각 (Unix 초). 읽지 못하면 None
pub fn process_start_time(pid: u32) -> Option<u64> {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    if !sys.refresh_process(pid) {
        return None;
    }
    sys.process(pid).map(|p| p.start_time()).filter(|&t| t > 0)
}

/// 기록해 둔 PID가 아직 그때의 프로세스인가: 살아 있고, `recorded_at`(그 프로세스를 띄운 뒤 기록한 시각)보다
/// 늦게 시작하지 않았다. 재부팅 뒤 같은 PID를 다른 프로세스가 쓰면 시작 시각이 기록보다 늦다.
/// 시작 시각을 읽지 못하면(권한) 살아 있는지만 본다
pub fn is_same_process(pid: u32, recorded_at: u64) -> bool {
    if !is_process_alive(pid) {
        return false;
    }
    process_start_time(pid).is_none_or(|started| started <= recorded_at.saturating_add(START_TIME_SLACK_SECS))
}

/// Force-kill a process by PID. Cross-platform helper.
/// 종료 요청이 거부됐고(권한 없음 등) 프로세스가 아직 살아 있으면 오류를 돌려준다.
/// 그 프로세스 하나만 끝낸다: GUI가 띄운 데몬처럼 자식이 따로 살아야 하는 경우가 있다
pub fn force_kill_pid(pid: u32) -> Result<()> {
    kill(pid, false)
}

/// 게임 서버 강제 종료. Windows는 자식까지 끝낸다: 런처형 서버(PalServer.exe 등)는 런처만 죽이면
/// 실제 서버가 남는다. Unix는 `force_kill_pid`와 같다
pub fn force_kill_tree(pid: u32) -> Result<()> {
    kill(pid, true)
}

#[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
fn kill(pid: u32, tree: bool) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let pid_arg = pid.to_string();
        let mut args = vec!["/F", "/PID", pid_arg.as_str()];
        if tree {
            args.insert(1, "/T");
        }
        let out = std::process::Command::new(saba_chan_updater_lib::process::system_tool("taskkill"))
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to kill PID {}: {}", pid, e))?;
        // 128: 그런 프로세스가 없다 (이미 끝났다). 접근 거부 등은 살아 있는지 조회도 막힐 수 있어 실패로 본다
        if !out.status.success() && out.status.code() != Some(128) {
            return Err(anyhow::anyhow!(
                "Failed to kill PID {}: {}",
                pid,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let ret = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            // ESRCH: 이미 없다
            if err.raw_os_error() != Some(libc::ESRCH) {
                return Err(anyhow::anyhow!("Failed to kill PID {}: {}", pid, err));
            }
        }
    }
    Ok(())
}

/// `alive`가 거짓이 될 때까지 최대 `timeout` 기다린다. 끝난 것을 확인하면 true
pub async fn wait_gone(timeout: std::time::Duration, alive: impl Fn() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while alive() {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    true
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub start_time: u64,
}

pub struct ProcessTracker {
    processes: Mutex<HashMap<String, ProcessInfo>>,
}

impl Default for ProcessTracker {
    fn default() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }
}

impl ProcessTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mutex 락 획득 헬퍼 — 보일러플레이트 제거
    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, ProcessInfo>>, ProcessError> {
        self.processes.lock().map_err(|e| {
            tracing::error!("ProcessTracker lock poisoned: {}", e);
            ProcessError::LockPoisoned
        })
    }

    /// Get start_time by server name
    pub fn get_start_time(&self, server_name: &str) -> Result<u64, ProcessError> {
        let processes = self.lock()?;
        processes
            .get(server_name)
            .map(|p| p.start_time)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// Track a server process by name
    pub fn track(&self, server_name: &str, pid: u32) -> Result<()> {
        self.track_since(server_name, pid, current_timestamp())
    }

    /// 이미 돌던 서버를 다시 붙일 때: 실제로 시작한 시각(유닉스 초)을 그대로 쓴다
    pub fn track_since(&self, server_name: &str, pid: u32, start_time: u64) -> Result<()> {
        let info = ProcessInfo { pid, start_time };
        let mut processes = self.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        processes.insert(server_name.to_string(), info);
        tracing::info!("Now tracking server '{}' with pid: {}", server_name, pid);
        Ok(())
    }

    /// 같은 서버의 PID만 바꾼다 (런처가 끝나고 진짜 서버로 넘어갈 때). 시작 시각은 그대로
    pub fn replace_pid(&self, server_name: &str, pid: u32) -> Result<()> {
        let mut processes = self.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        match processes.get_mut(server_name) {
            Some(info) => info.pid = pid,
            None => {
                processes.insert(server_name.to_string(), ProcessInfo { pid, start_time: current_timestamp() });
            }
        }
        Ok(())
    }

    /// Get PID by server name
    pub fn get_pid(&self, server_name: &str) -> Result<u32, ProcessError> {
        let processes = self.lock()?;
        processes
            .get(server_name)
            .map(|p| p.pid)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// Stop tracking a server by name
    pub fn untrack(&self, server_name: &str) -> Result<(), ProcessError> {
        let mut processes = self.lock()?;
        processes
            .remove(server_name)
            .ok_or(ProcessError::NotFound { pid: 0 })?;
        tracing::info!("Stopped tracking server '{}'", server_name);
        Ok(())
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 재부팅 뒤 같은 PID를 다른 프로세스가 쓰면, 그 프로세스는 기록보다 늦게 시작했다
    #[test]
    fn a_pid_reused_after_the_record_is_not_the_same_process() {
        let me = std::process::id();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert!(is_same_process(me, now), "the recording process itself");
        let started = process_start_time(me).expect("own start time");
        assert!(!is_same_process(me, started.saturating_sub(3600)), "started an hour after the record");
        assert!(!is_same_process(u32::MAX - 1, now), "not running");
    }

    #[test]
    fn test_get_pid() {
        let tracker = ProcessTracker::new();
        tracker.track("palworld", 5678).unwrap();
        assert_eq!(tracker.get_pid("palworld").unwrap(), 5678);
    }

    #[test]
    fn test_not_found() {
        let tracker = ProcessTracker::new();
        assert!(tracker.get_pid("nonexistent").is_err());
    }

    #[test]
    fn test_untrack() {
        let tracker = ProcessTracker::new();
        tracker.track("minecraft", 1234).unwrap();
        tracker.untrack("minecraft").unwrap();
        assert!(tracker.get_pid("minecraft").is_err());
    }

    // ── 추가 심층 테스트 ──

    #[test]
    fn test_start_time_is_nonzero() {
        let tracker = ProcessTracker::new();
        tracker.track("srv", 1234).unwrap();
        let start_time = tracker.get_start_time("srv").unwrap();
        assert!(start_time > 0, "Start time should be a valid Unix timestamp, got: {}", start_time);
    }

    #[test]
    fn reattached_server_keeps_its_real_start_time() {
        let tracker = ProcessTracker::new();
        tracker.track_since("srv", 1234, 1_000).unwrap();
        assert_eq!(tracker.get_start_time("srv").unwrap(), 1_000);
    }

    #[test]
    fn test_start_time_not_found() {
        let tracker = ProcessTracker::new();
        assert!(tracker.get_start_time("ghost").is_err());
    }

    #[test]
    fn test_track_overwrites_existing() {
        let tracker = ProcessTracker::new();
        tracker.track("srv", 1000).unwrap();
        assert_eq!(tracker.get_pid("srv").unwrap(), 1000);

        // 같은 이름으로 재등록 → 새 PID로 업데이트
        tracker.track("srv", 2000).unwrap();
        assert_eq!(tracker.get_pid("srv").unwrap(), 2000);
    }

    #[test]
    fn test_untrack_nonexistent_returns_error() {
        let tracker = ProcessTracker::new();
        let result = tracker.untrack("never-existed");
        assert!(result.is_err(), "untrack on unknown server should return NotFound");
    }

    #[test]
    fn test_multiple_servers_independent() {
        let tracker = ProcessTracker::new();
        tracker.track("mc", 100).unwrap();
        tracker.track("pw", 200).unwrap();
        tracker.track("zomboid", 300).unwrap();

        assert_eq!(tracker.get_pid("mc").unwrap(), 100);
        assert_eq!(tracker.get_pid("pw").unwrap(), 200);
        assert_eq!(tracker.get_pid("zomboid").unwrap(), 300);

        // 하나만 삭제 → 나머지 존재
        tracker.untrack("mc").unwrap();
        assert!(tracker.get_pid("mc").is_err());
        assert_eq!(tracker.get_pid("zomboid").unwrap(), 300);
    }

    /// 동일 PID를 다른 서버 이름으로 등록 — 허용되어야 함 (추적 기반이 이름이므로)
    #[test]
    fn test_same_pid_different_servers() {
        let tracker = ProcessTracker::new();
        tracker.track("srv-a", 9999).unwrap();
        tracker.track("srv-b", 9999).unwrap();

        assert_eq!(tracker.get_pid("srv-a").unwrap(), 9999);
        assert_eq!(tracker.get_pid("srv-b").unwrap(), 9999);
    }

    /// 자식 하나(ping)를 띄운 런처(cmd)와 그 자식 PID. cmd는 ping보다 먼저 콘솔 호스트(conhost)를 자식으로 둔다:
    /// 그것을 자식으로 잡으면 ping이 뜨기 전에 cmd를 죽였을 때 콘솔과 함께 사라진다 (L24). 이름으로 ping을 기다린다
    #[cfg(windows)]
    async fn launcher_with_child() -> (std::process::Child, u32) {
        use std::os::windows::process::CommandExt;
        let launcher = std::process::Command::new(saba_chan_updater_lib::process::system_tool("cmd"))
            .args(["/C", "ping -n 60 127.0.0.1 > nul"])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let parent = sysinfo::Pid::from_u32(launcher.id());
        let mut child = None;
        for _ in 0..600 {
            let mut sys = sysinfo::System::new();
            sys.refresh_processes();
            child = sys
                .processes()
                .iter()
                .find(|(_, p)| p.parent() == Some(parent) && p.name().eq_ignore_ascii_case("ping.exe"))
                .map(|(pid, _)| pid.as_u32());
            if child.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        (launcher, child.expect("the launcher started its child"))
    }

    /// 끝난 프로세스는 다른 핸들(여기서는 Child)이 남아 있어도 살아 있지 않다
    #[cfg(windows)]
    #[test]
    fn an_exited_process_with_an_open_handle_is_not_alive() {
        let mut child = std::process::Command::new(saba_chan_updater_lib::process::system_tool("cmd")).args(["/C", "exit 23"]).spawn().unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(23));
        assert!(!is_process_alive(child.id()), "an exited process counts as running");
    }

    /// 런처형 서버(PalServer.exe 등)는 실제 서버를 자식으로 띄운다: 런처만 죽이면 서버가 남는다
    #[cfg(windows)]
    #[tokio::test]
    async fn a_server_kill_also_ends_the_children() {
        let (launcher, child) = launcher_with_child().await;
        force_kill_tree(launcher.id()).unwrap();
        assert!(
            wait_gone(std::time::Duration::from_secs(5), || is_process_alive(child)).await,
            "the server the launcher started is still running"
        );
    }

    /// 단일 강제 종료는 자식을 남긴다: GUI를 닫을 때 GUI가 띄운 데몬·업데이터가 같이 죽으면 안 된다
    #[cfg(windows)]
    #[tokio::test]
    async fn a_plain_force_kill_leaves_the_children() {
        let (mut launcher, child) = launcher_with_child().await;
        force_kill_pid(launcher.id()).unwrap();
        let _ = launcher.wait();
        assert!(is_process_alive(child), "the child of a killed process was killed too");
        let _ = force_kill_pid(child);
    }

    /// 종료 요청이 거부되면 성공으로 치지 않는다 (권한 없는 프로세스: PID 1)
    #[cfg(unix)]
    #[test]
    fn refused_kill_is_an_error() {
        if unsafe { libc::geteuid() } == 0 {
            return; // root는 PID 1에도 신호를 보낼 수 있어 시험하지 않는다
        }
        assert!(force_kill_pid(1).is_err());
    }

    /// 이미 없는 프로세스는 끝난 것으로 본다
    #[cfg(unix)]
    #[test]
    fn killing_a_gone_process_is_ok() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(force_kill_pid(pid).is_ok());
    }

    /// 빈 서버 이름 — 허용되지만 의미 없는 엣지 케이스
    #[test]
    fn test_empty_server_name() {
        let tracker = ProcessTracker::new();
        tracker.track("", 1234).unwrap();
        assert_eq!(tracker.get_pid("").unwrap(), 1234);
        tracker.untrack("").unwrap();
        assert!(tracker.get_pid("").is_err());
    }
}
