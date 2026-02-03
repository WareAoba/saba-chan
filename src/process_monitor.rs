use anyhow::Result;
use serde::{Deserialize, Serialize};
use sysinfo::{System, Pid};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningProcess {
    pub pid: u32,
    pub name: String,
    pub executable_path: Option<String>,
}

pub struct ProcessMonitor;

impl Default for ProcessMonitor {
    fn default() -> Self {
        Self
    }
}

impl ProcessMonitor {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }

    /// 크로스 플랫폼: 실행 중인 모든 프로세스 목록 가져오기
    pub fn get_running_processes() -> Result<Vec<RunningProcess>> {
        let mut sys = System::new_all();
        sys.refresh_all();
        
        let processes: Vec<RunningProcess> = sys.processes()
            .iter()
            .map(|(pid, process)| {
                RunningProcess {
                    pid: pid.as_u32(),
                    name: process.name().to_string(),
                    executable_path: process.exe().and_then(|p| p.to_str()).map(String::from),
                }
            })
            .collect();

        tracing::debug!("Found {} running processes", processes.len());
        Ok(processes)
    }

    /// 특정 프로세스 이름으로 검색
    pub fn find_by_name(name: &str) -> Result<Vec<RunningProcess>> {
        let all_processes = Self::get_running_processes()?;
        Ok(all_processes
            .into_iter()
            .filter(|p| p.name.to_lowercase().contains(&name.to_lowercase()))
            .collect())
    }

    /// 특정 PID가 실행 중인지 확인 (크로스 플랫폼)
    pub fn is_running(pid: u32) -> bool {
        let mut sys = System::new();
        sys.refresh_processes();
        sys.process(Pid::from_u32(pid)).is_some()
    }
}
