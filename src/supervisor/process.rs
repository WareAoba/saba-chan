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
        use winapi::um::processthreadsapi::OpenProcess;
        use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
        use winapi::um::handleapi::CloseHandle;

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
            true
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // kill(pid, 0) checks process existence without actually sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

/// Windows `taskkill` 인자 — `/T`로 자식 프로세스 트리까지 종료한다.
///
/// 런처형 서버(PalServer.exe → PalServer-Win64-Shipping-Cmd.exe 등)는 런처만 죽이면
/// 실제 서버 프로세스가 남는다.
pub fn taskkill_args(pid: u32) -> [String; 4] {
    ["/F".to_string(), "/T".to_string(), "/PID".to_string(), pid.to_string()]
}

/// 프로세스(와 Windows에서는 그 자식 트리)를 강제 종료한다.
pub fn force_kill_pid(pid: u32) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("taskkill")
            .args(taskkill_args(pid))
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to kill PID {}: {}", pid, e))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    start_time: u64,
}

/// 인스턴스 id → 추적 중인 PID. 내부 Mutex로 `&self`에서 갱신할 수 있다.
#[derive(Default)]
pub struct ProcessTracker {
    processes: Mutex<HashMap<String, ProcessInfo>>,
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

    /// 추적 시작 시각 (epoch 초)
    pub fn get_start_time(&self, key: &str) -> Result<u64, ProcessError> {
        let processes = self.lock()?;
        processes
            .get(key)
            .map(|p| p.start_time)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// 마지막 track() 호출 이후 경과 시간(초). 존재하지 않으면 에러.
    ///
    /// 백그라운드 모니터가 "방금 추적을 시작한" 프로세스를, 그 추적 시점보다
    /// 먼저 스캔된 stale snapshot으로 오판하여 즉시 untrack하는 것을 막는 데 사용한다.
    pub fn seconds_since_tracked(&self, key: &str) -> Result<u64, ProcessError> {
        let start_time = self.get_start_time(key)?;
        Ok(current_timestamp().saturating_sub(start_time))
    }

    /// 테스트 전용: track된 시각을 과거로 조작해 grace period 만료를 시뮬레이션한다.
    #[cfg(test)]
    pub fn backdate_for_test(&self, key: &str, seconds_ago: u64) {
        let mut processes = self.processes.lock().expect("lock poisoned in test");
        if let Some(info) = processes.get_mut(key) {
            info.start_time = current_timestamp().saturating_sub(seconds_ago);
        }
    }

    /// 프로세스 추적 시작 (같은 키면 덮어쓴다)
    pub fn track(&self, key: &str, pid: u32) -> Result<()> {
        let info = ProcessInfo { pid, start_time: current_timestamp() };
        let mut processes = self.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        processes.insert(key.to_string(), info);
        tracing::info!("Now tracking server '{}' with pid: {}", key, pid);
        Ok(())
    }

    pub fn get_pid(&self, key: &str) -> Result<u32, ProcessError> {
        let processes = self.lock()?;
        processes
            .get(key)
            .map(|p| p.pid)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// 추적 중지
    pub fn untrack(&self, key: &str) -> Result<(), ProcessError> {
        let mut processes = self.lock()?;
        processes
            .remove(key)
            .ok_or(ProcessError::NotFound { pid: 0 })?;
        tracing::info!("Stopped tracking server '{}'", key);
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

    #[test]
    fn taskkill_kills_whole_tree() {
        let args = taskkill_args(42);
        assert!(args.contains(&"/T".to_string()), "taskkill must include /T: {:?}", args);
        assert!(args.contains(&"/F".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("42"));
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

    #[test]
    fn test_start_time_is_nonzero() {
        let tracker = ProcessTracker::new();
        tracker.track("srv", 1234).unwrap();
        let start_time = tracker.get_start_time("srv").unwrap();
        assert!(start_time > 0, "Start time should be a valid Unix timestamp, got: {}", start_time);
    }

    #[test]
    fn test_start_time_not_found() {
        let tracker = ProcessTracker::new();
        assert!(tracker.get_start_time("ghost").is_err());
    }

    #[test]
    fn test_seconds_since_tracked_is_small_right_after_track() {
        let tracker = ProcessTracker::new();
        tracker.track("srv", 1234).unwrap();
        let elapsed = tracker.seconds_since_tracked("srv").unwrap();
        assert!(elapsed < 2, "Freshly tracked process should report ~0s elapsed, got {}", elapsed);
    }

    #[test]
    fn test_seconds_since_tracked_not_found() {
        let tracker = ProcessTracker::new();
        assert!(tracker.seconds_since_tracked("ghost").is_err());
    }

    #[test]
    fn test_track_overwrites_existing() {
        let tracker = ProcessTracker::new();
        tracker.track("srv", 1000).unwrap();
        tracker.track("srv", 2000).unwrap();
        assert_eq!(tracker.get_pid("srv").unwrap(), 2000);
    }

    #[test]
    fn test_untrack_nonexistent_returns_error() {
        let tracker = ProcessTracker::new();
        assert!(tracker.untrack("never-existed").is_err());
    }

    #[test]
    fn test_multiple_servers_independent() {
        let tracker = ProcessTracker::new();
        tracker.track("mc", 100).unwrap();
        tracker.track("pw", 200).unwrap();
        tracker.untrack("mc").unwrap();
        assert!(tracker.get_pid("mc").is_err());
        assert_eq!(tracker.get_pid("pw").unwrap(), 200);
    }
}
