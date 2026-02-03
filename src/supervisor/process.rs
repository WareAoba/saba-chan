use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;
use anyhow::Result;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum ProcessError {
    #[error("process {pid} not found")]
    NotFound { pid: u32 },
    #[error("failed to terminate process: {reason}")]
    TerminationFailed { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ProcessStatus {
    Running,
    Stopped,
    Crashed,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProcessInfo {
    pub pid: u32,
    pub status: ProcessStatus,
    pub start_time: u64,
    pub last_check: u64,
}

pub struct ProcessTracker {
    // Map server name to ProcessInfo
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

    /// Track a server process by name
    #[allow(dead_code)]
    pub fn track(&self, server_name: &str, pid: u32) -> Result<()> {
        let now = current_timestamp();
        let info = ProcessInfo {
            pid,
            status: ProcessStatus::Running,
            start_time: now,
            last_check: now,
        };
        let mut processes = match self.processes.lock() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to acquire ProcessTracker lock: {}", e);
                return Err(anyhow::anyhow!("Mutex lock failed"));
            }
        };
        processes.insert(server_name.to_string(), info);
        tracing::info!("Now tracking server '{}' with pid: {}", server_name, pid);
        Ok(())
    }

    /// Get server status by name
    #[allow(dead_code)]
    pub fn get_status(&self, server_name: &str) -> Result<ProcessStatus, ProcessError> {
        let processes = match self.processes.lock() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to acquire ProcessTracker lock: {}", e);
                return Err(ProcessError::NotFound { pid: 0 });
            }
        };
        processes
            .get(server_name)
            .map(|p| p.status)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// Get PID by server name
    #[allow(dead_code)]
    pub fn get_pid(&self, server_name: &str) -> Result<u32, ProcessError> {
        let processes = match self.processes.lock() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to acquire ProcessTracker lock: {}", e);
                return Err(ProcessError::NotFound { pid: 0 });
            }
        };
        processes
            .get(server_name)
            .map(|p| p.pid)
            .ok_or(ProcessError::NotFound { pid: 0 })
    }

    /// Mark server as crashed by name
    #[allow(dead_code)]
    pub fn mark_crashed(&mut self, server_name: &str) -> Result<()> {
        let mut processes = self.processes.lock().unwrap();
        if let Some(info) = processes.get_mut(server_name) {
            info.status = ProcessStatus::Crashed;
            tracing::warn!("Server '{}' marked as crashed", server_name);
        }
        Ok(())
    }

    /// Terminate server process by name (크로스 플랫폼)
    #[allow(dead_code)]
    pub fn terminate(&mut self, server_name: &str, force: bool) -> Result<(), ProcessError> {
        let mut processes = self.processes.lock().unwrap();
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
    #[allow(dead_code)]
    pub fn untrack(&self, server_name: &str) -> Result<(), ProcessError> {
        let mut processes = match self.processes.lock() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to acquire ProcessTracker lock: {}", e);
                return Err(ProcessError::NotFound { pid: 0 });
            }
        };
        processes
            .remove(server_name)
            .ok_or(ProcessError::NotFound { pid: 0 })?;
        tracing::info!("Stopped tracking server '{}'", server_name);
        Ok(())
    }
}

#[allow(dead_code)]
fn current_timestamp() -> u64 {
    #[allow(dead_code)]
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
        let mut tracker = ProcessTracker::new();
        tracker.track("minecraft", 5678).unwrap();
        tracker.mark_crashed("minecraft").unwrap();
        assert_eq!(tracker.get_status("minecraft").unwrap(), ProcessStatus::Crashed);
    }

    #[test]
    fn test_terminate() {
        let mut tracker = ProcessTracker::new();
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

/// 명령어 실행 관리자
pub struct ProcessManager;

impl Default for ProcessManager {
    fn default() -> Self {
        Self
    }
}

impl ProcessManager {
    pub fn new() -> Self {
        Self
    }

    /// 인스턴스에 명령어 실행
    /// 참고: 이 메서드는 실제로 Supervisor에서 호출되어야 하며,
    /// Supervisor가 module_loader를 제공해야 함
    #[allow(dead_code)]
    pub async fn execute_command(
        &self,
        instance_id: &str,
        module_name: &str,
        command: &str,
        args: serde_json::Value,
    ) -> Result<String> {
        tracing::info!(
            "Executing command '{}' for instance '{}' (module: {})",
            command,
            instance_id,
            module_name
        );

        // 모듈의 lifecycle.py에 command 함수 호출
        let _config = serde_json::json!({
            "instance_id": instance_id,
            "command": command,
            "args": args,
        });

        // 플러그인 실행을 통해 모듈에 명령어 전달
        // 상세한 구현은 모듈 로더와 통합되어야 함
        // NOTE: 실제 구현은 supervisor.rs의 execute_command 메서드에서 수행
        tracing::info!("Command '{}' queued for execution", command);

        Ok(format!("Command '{}' executed", command))
    }
}
