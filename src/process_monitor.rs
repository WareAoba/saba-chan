use std::process::Command;
use anyhow::Result;
use serde::{Deserialize, Serialize};

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
    pub fn new() -> Self {
        Self::default()
    }

    /// Windows에서 실행 중인 모든 프로세스 목록 가져오기 (PowerShell 사용)
    pub fn get_running_processes() -> Result<Vec<RunningProcess>> {
        // PowerShell 명령 실행 (오류 시 안전하게 처리)
        let output = match Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-Process | Select-Object Id,ProcessName,Path | ConvertTo-Csv -NoTypeInformation",
            ])
            .output() {
                Ok(out) => out,
                Err(e) => {
                    tracing::warn!("Failed to execute PowerShell: {}", e);
                    return Ok(Vec::new()); // 빈 목록 반환 (Panic 방지)
                }
            };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("PowerShell command failed: {}", stderr);
            return Ok(Vec::new()); // 빈 목록 반환 (Panic 방지)
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let mut processes = Vec::new();

        for line in output_str.lines().skip(1) {
            // Skip header
            // CSV format: "Id","ProcessName","Path"
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            
            // Simple CSV parsing (handles quoted strings)
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let pid_str = parts[0].trim().trim_matches('"');
                let name = parts[1].trim().trim_matches('"');
                let path = if parts.len() >= 3 {
                    let p = parts[2..].join(","); // Path might contain commas
                    let p = p.trim().trim_matches('"');
                    if p.is_empty() { None } else { Some(p.to_string()) }
                } else {
                    None
                };
                
                match pid_str.parse::<u32>() {
                    Ok(pid) => {
                        processes.push(RunningProcess {
                            pid,
                            name: name.to_string(),
                            executable_path: path,
                        });
                    }
                    Err(e) => {
                        tracing::debug!("Failed to parse PID '{}': {}", pid_str, e);
                        // 파싱 실패한 줄은 무시하고 계속
                    }
                }
            }
        }

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

    /// 특정 PID가 실행 중인지 확인
    pub fn is_running(pid: u32) -> bool {
        #[cfg(target_os = "windows")]
        {
            use winapi::um::processthreadsapi::OpenProcess;
            use winapi::um::winnt::PROCESS_QUERY_INFORMATION;
            use winapi::um::handleapi::CloseHandle;

            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
                if !handle.is_null() {
                    CloseHandle(handle);
                    true
                } else {
                    false
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            // Linux/Mac: /proc/{pid} 확인 또는 kill 0 사용
            std::path::Path::new(&format!("/proc/{}", pid)).exists()
        }
    }
}
