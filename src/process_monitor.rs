use serde::{Deserialize, Serialize};
use sysinfo::System;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningProcess {
    pub pid: u32,
    pub name: String,
    pub executable_path: Option<String>,
    /// 프로세스 커맨드라인 인수 (예: ["java", "-jar", "server.jar"])
    #[serde(default)]
    pub cmd: Vec<String>,
}

/// 크로스 플랫폼: 실행 중인 모든 프로세스 목록 가져오기
pub fn get_running_processes() -> Vec<RunningProcess> {
    let mut sys = System::new();
    sys.refresh_processes();
    
    let processes: Vec<RunningProcess> = sys.processes()
        .iter()
        .map(|(pid, process)| {
            RunningProcess {
                pid: pid.as_u32(),
                name: process.name().to_string(),
                executable_path: process.exe().and_then(|p| p.to_str()).map(String::from),
                cmd: process.cmd().to_vec(),
            }
        })
        .collect();

    tracing::debug!("Found {} running processes", processes.len());
    processes
}

// ── Snapshot-based lookups ──────────────────────────────────
// 프로세스 목록을 한 번 스캔한 후, 그 snapshot으로 여러 검색을 수행합니다.
// RwLock write lock 밖에서 스캔하여 lock 점유 시간을 최소화합니다.

/// 사전 스캔된 프로세스 목록에서 PID 존재 여부 확인
pub fn is_running_in(processes: &[RunningProcess], pid: u32) -> bool {
    processes.iter().any(|p| p.pid == pid)
}

/// 사전 스캔된 프로세스 목록에서 이름으로 검색
pub fn find_by_name_in<'a>(processes: &'a [RunningProcess], name: &str) -> Vec<&'a RunningProcess> {
    let name_lower = name.to_lowercase();
    processes.iter()
        .filter(|p| p.name.to_lowercase().contains(&name_lower))
        .collect()
}

/// 사전 스캔된 프로세스 목록에서 이름 + 커맨드라인 패턴으로 검색
pub fn find_by_name_and_cmd_in<'a>(processes: &'a [RunningProcess], name: &str, cmd_patterns: &[String]) -> Vec<&'a RunningProcess> {
    let name_lower = name.to_lowercase();
    processes.iter()
        .filter(|p| {
            if !p.name.to_lowercase().contains(&name_lower) {
                return false;
            }
            if cmd_patterns.is_empty() {
                return true;
            }
            let cmdline = p.cmd.join(" ").to_lowercase();
            cmd_patterns.iter().any(|pat| cmdline.contains(&pat.to_lowercase()))
        })
        .collect()
}

// ── Async wrappers ─────────────────────────────────────────
// sysinfo 시스템 콜은 동기적으로 OS 프로세스 테이블 전체를 스캔합니다.
// tokio 워커 스레드에서 직접 호출하면 런타임 전체가 블로킹되므로,
// spawn_blocking을 통해 전용 블로킹 스레드풀에서 실행합니다.

/// `get_running_processes`의 비동기 래퍼.
/// write lock 밖에서 한 번 호출하여 snapshot을 만들고, lock 안에서 snapshot 기반 검색을 사용합니다.
pub async fn get_running_processes_async() -> Vec<RunningProcess> {
    tokio::task::spawn_blocking(get_running_processes)
        .await
        .unwrap_or_default()
}
