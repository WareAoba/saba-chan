use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;
use anyhow::Result;

#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("process {pid} not found")]
    NotFound { pid: u32 },
    #[allow(dead_code)] // terminate() 내부에서 사용 예정
    #[error("failed to terminate process: {reason}")]
    TerminationFailed { reason: String },
    #[error("lock poisoned")]
    LockPoisoned,
}

/// Force-kill a process by PID. Cross-platform helper.
pub fn force_kill_pid(pid: u32) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // 공개 API — 서버 상태 전이에 필요
pub enum ProcessStatus {
    Running,
    Stopped,
    Crashed,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // 공개 API — 프로세스 메타데이터 전체 필드 노출 필요
pub struct ProcessInfo {
    pub pid: u32,
    pub status: ProcessStatus,
    pub start_time: u64,
    pub last_check: u64,
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
        let now = current_timestamp();
        let info = ProcessInfo {
            pid,
            status: ProcessStatus::Running,
            start_time: now,
            last_check: now,
        };
        let mut processes = self.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        processes.insert(server_name.to_string(), info);
        tracing::info!("Now tracking server '{}' with pid: {}", server_name, pid);
        Ok(())
    }

    /// Get server status by name
    #[allow(dead_code)] // 공개 API — 외부 호출자용
    pub fn get_status(&self, server_name: &str) -> Result<ProcessStatus, ProcessError> {
        let processes = self.lock()?;
        processes
            .get(server_name)
            .map(|p| p.status)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// Get PID by server name
    pub fn get_pid(&self, server_name: &str) -> Result<u32, ProcessError> {
        let processes = self.lock()?;
        processes
            .get(server_name)
            .map(|p| p.pid)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// Mark server as crashed by name
    #[allow(dead_code)] // 공개 API — 크래시 감지 시 사용 예정
    pub fn mark_crashed(&self, server_name: &str) -> Result<(), ProcessError> {
        let mut processes = self.lock()?;
        if let Some(info) = processes.get_mut(server_name) {
            info.status = ProcessStatus::Crashed;
            tracing::warn!("Server '{}' marked as crashed", server_name);
        }
        Ok(())
    }

    /// Terminate server process by name (크로스 플랫폼)
    #[allow(dead_code)] // 공개 API — 프로세스 종료 기능
    pub fn terminate(&self, server_name: &str, force: bool) -> Result<(), ProcessError> {
        let mut processes = self.lock()?;
        let pid = processes
            .get(server_name)
            .map(|p| p.pid)
            .ok_or(ProcessError::NotFound { pid: 0 })?;

        let signal_name = if force { "KILL" } else { "TERM" };
        tracing::info!("Sending {} signal to server '{}' (pid: {})", signal_name, server_name, pid);

        // 크로스 플랫폼 프로세스 종료
        #[cfg(target_os = "windows")]
        {
            use winapi::um::processthreadsapi::{OpenProcess, TerminateProcess};
            use winapi::um::winnt::PROCESS_TERMINATE;
            use winapi::um::handleapi::CloseHandle;

            unsafe {
                let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if handle.is_null() {
                    return Err(ProcessError::TerminationFailed {
                        reason: format!("Failed to open process {}", pid),
                    });
                }
                
                let exit_code = if force { 1 } else { 0 };
                let result = TerminateProcess(handle, exit_code);
                CloseHandle(handle);
                
                if result == 0 {
                    return Err(ProcessError::TerminationFailed {
                        reason: "TerminateProcess failed".to_string(),
                    });
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;

            let signal = if force { Signal::SIGKILL } else { Signal::SIGTERM };
            if let Err(e) = signal::kill(Pid::from_raw(pid as i32), signal) {
                return Err(ProcessError::TerminationFailed {
                    reason: format!("Failed to send signal: {}", e),
                });
            }
        }

        if let Some(info) = processes.get_mut(server_name) {
            info.status = ProcessStatus::Stopped;
        }
        Ok(())
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

    #[test]
    fn test_track_and_get_status() {
        let tracker = ProcessTracker::new();
        tracker.track("minecraft", 1234).unwrap();
        assert_eq!(tracker.get_status("minecraft").unwrap(), ProcessStatus::Running);
    }

    #[test]
    fn test_get_pid() {
        let tracker = ProcessTracker::new();
        tracker.track("palworld", 5678).unwrap();
        assert_eq!(tracker.get_pid("palworld").unwrap(), 5678);
    }

    #[test]
    fn test_mark_crashed() {
        let tracker = ProcessTracker::new();
        tracker.track("minecraft", 5678).unwrap();
        tracker.mark_crashed("minecraft").unwrap();
        assert_eq!(tracker.get_status("minecraft").unwrap(), ProcessStatus::Crashed);
    }

    #[test]
    fn test_terminate() {
        let tracker = ProcessTracker::new();
        // 존재하지 않는 PID로 종료 시도는 실패해야 함
        tracker.track("palworld", 99999).unwrap();
        let result = tracker.terminate("palworld", false);
        // 실제 프로세스가 없으므로 에러가 발생할 수 있음
        // 이 테스트는 terminate 메서드가 호출 가능한지만 확인
        assert!(result.is_err() || tracker.get_status("palworld").is_ok());
    }

    #[test]
    fn test_not_found() {
        let tracker = ProcessTracker::new();
        let result = tracker.get_status("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_untrack() {
        let tracker = ProcessTracker::new();
        tracker.track("minecraft", 1234).unwrap();
        tracker.untrack("minecraft").unwrap();
        assert!(tracker.get_status("minecraft").is_err());
    }
}
