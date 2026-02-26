use serde::{Deserialize, Serialize};
use sysinfo::{System, Pid};

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
    let mut sys = System::new_all();
    sys.refresh_all();
    
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

/// 특정 프로세스 이름으로 검색
pub fn find_by_name(name: &str) -> Vec<RunningProcess> {
    let name_lower = name.to_lowercase();
    get_running_processes()
        .into_iter()
        .filter(|p| p.name.to_lowercase().contains(&name_lower))
        .collect()
}

/// 프로세스 이름 + 커맨드라인 패턴으로 검색
///
/// `cmd_patterns`가 비어 있으면 `find_by_name`과 동일하게 동작.
/// 비어 있지 않으면 커맨드라인 전체 문자열에 패턴 중 하나 이상이 포함된 프로세스만 반환.
/// 이를 통해 같은 java.exe를 쓰는 Minecraft와 Zomboid를 구분할 수 있다.
pub fn find_by_name_and_cmd(name: &str, cmd_patterns: &[String]) -> Vec<RunningProcess> {
    let name_lower = name.to_lowercase();

    get_running_processes()
        .into_iter()
        .filter(|p| {
            // 1단계: 프로세스 이름 매칭
            if !p.name.to_lowercase().contains(&name_lower) {
                return false;
            }
            // 2단계: cmd_patterns가 있으면 커맨드라인도 매칭
            if cmd_patterns.is_empty() {
                return true;
            }
            let cmdline = p.cmd.join(" ").to_lowercase();
            cmd_patterns.iter().any(|pat| cmdline.contains(&pat.to_lowercase()))
        })
        .collect()
}

/// 특정 PID가 실행 중인지 확인 (크로스 플랫폼)
pub fn is_running(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes();
    sys.process(Pid::from_u32(pid)).is_some()
}

// ── Async wrappers ─────────────────────────────────────────
// sysinfo 시스템 콜은 동기적으로 OS 프로세스 테이블 전체를 스캔합니다.
// tokio 워커 스레드에서 직접 호출하면 런타임 전체가 블로킹되므로,
// spawn_blocking을 통해 전용 블로킹 스레드풀에서 실행합니다.

/// `is_running`의 비동기 래퍼.
pub async fn is_running_async(pid: u32) -> bool {
    tokio::task::spawn_blocking(move || is_running(pid))
        .await
        .unwrap_or(false)
}

/// `find_by_name`의 비동기 래퍼.
pub async fn find_by_name_async(name: &str) -> Vec<RunningProcess> {
    let name = name.to_string();
    tokio::task::spawn_blocking(move || find_by_name(&name))
        .await
        .unwrap_or_default()
}

/// `find_by_name_and_cmd`의 비동기 래퍼.
pub async fn find_by_name_and_cmd_async(name: &str, cmd_patterns: &[String]) -> Vec<RunningProcess> {
    let name = name.to_string();
    let patterns = cmd_patterns.to_vec();
    tokio::task::spawn_blocking(move || find_by_name_and_cmd(&name, &patterns))
        .await
        .unwrap_or_default()
}
