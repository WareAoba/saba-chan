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
    /// 작업 폴더 (읽을 수 있으면). 같은 게임 인스턴스가 여럿일 때 주인을 가리는 데 쓴다
    #[serde(default)]
    pub cwd: Option<String>,
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
                cwd: process.cwd().and_then(|p| p.to_str()).map(String::from),
            }
        })
        .collect();

    tracing::debug!("Found {} running processes", processes.len());
    processes
}

// ── Snapshot-based lookups ──────────────────────────────────

fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/").to_lowercase();
    while s.ends_with('/') {
        s.pop();
    }
    s
}

fn under(path: &str, root: &str) -> bool {
    let (path, root) = (normalize_path(path), normalize_path(root));
    !root.is_empty() && (path == root || path.starts_with(&format!("{}/", root)))
}

/// 프로세스의 실행 파일·작업 폴더·인수 중 하나라도 `roots` 아래에 있는가
pub fn process_under_any(process: &RunningProcess, roots: &[String]) -> bool {
    let paths = process.executable_path.iter().chain(process.cwd.iter()).chain(process.cmd.iter());
    paths.into_iter().any(|p| roots.iter().any(|r| under(p, r)))
}

/// 자동 감지 후보 중 이 인스턴스의 프로세스를 고른다.
/// - `taken`: 다른 인스턴스가 이미 추적하는 PID — 절대 고르지 않는다
/// - `own_roots`: 이 인스턴스의 폴더들, `other_roots`: 같은 모듈의 다른 인스턴스 폴더들
///
/// 같은 모듈 인스턴스가 이것뿐이면 폴더가 안 맞아도 첫 후보를 쓴다(예전 동작).
/// 여럿이면 이 인스턴스 폴더 아래의 프로세스만, 다른 인스턴스 폴더 아래의 것은 빼고 고른다.
pub fn pick_owned_process<'a>(
    candidates: &[&'a RunningProcess],
    taken: &std::collections::HashSet<u32>,
    own_roots: &[String],
    other_roots: &[String],
    sole_instance_of_module: bool,
) -> Option<&'a RunningProcess> {
    let free: Vec<&'a RunningProcess> = candidates.iter().copied().filter(|p| !taken.contains(&p.pid)).collect();
    if let Some(p) = free.iter().copied().find(|p| process_under_any(p, own_roots) && !process_under_any_more_specific(p, own_roots, other_roots)) {
        return Some(p);
    }
    if sole_instance_of_module {
        return free.first().copied();
    }
    None
}

/// 다른 인스턴스 폴더가 이 인스턴스 폴더 안에 있을 때(겹친 경로) 더 깊은 쪽이 주인이다
fn process_under_any_more_specific(process: &RunningProcess, own_roots: &[String], other_roots: &[String]) -> bool {
    let deepest_own = own_roots.iter().filter(|r| process_under_any(process, std::slice::from_ref(*r))).map(|r| normalize_path(r).len()).max().unwrap_or(0);
    other_roots
        .iter()
        .any(|r| process_under_any(process, std::slice::from_ref(r)) && normalize_path(r).len() > deepest_own)
}
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn proc(pid: u32, cwd: &str) -> RunningProcess {
        RunningProcess {
            pid,
            name: "java.exe".into(),
            executable_path: Some("C:/Program Files/Java/bin/java.exe".into()),
            cmd: vec!["java".into(), "-jar".into(), "server.jar".into()],
            cwd: Some(cwd.into()),
        }
    }

    #[test]
    fn auto_detect_assigns_each_process_to_its_own_instance() {
        let a = proc(10, r"C:\Saba\instances\a");
        let b = proc(20, r"C:\Saba\instances\b");
        let cands = vec![&a, &b];
        let roots_a = vec![r"C:\Saba\instances\a".to_string()];
        let roots_b = vec![r"C:\Saba\instances\b".to_string()];
        let none = HashSet::new();

        assert_eq!(pick_owned_process(&cands, &none, &roots_b, &roots_a, false).map(|p| p.pid), Some(20));
        // B만 남은 인스턴스로 A 하나만 돌고 있으면 B는 아무것도 잡지 않는다
        assert!(pick_owned_process(&[&a], &none, &roots_b, &roots_a, false).is_none());
        // 다른 인스턴스가 이미 잡은 PID는 고르지 않는다
        let taken: HashSet<u32> = [10].into();
        assert!(pick_owned_process(&[&a], &taken, &roots_a, &[], true).is_none());
        // 같은 모듈 인스턴스가 하나뿐이면 폴더를 몰라도 예전처럼 첫 후보
        assert_eq!(pick_owned_process(&[&a], &none, &[], &[], true).map(|p| p.pid), Some(10));
        // "a"가 "ab"의 접두어여도 섞이지 않는다
        let ab = proc(30, r"C:\Saba\instances\ab");
        assert!(pick_owned_process(&[&ab], &none, &roots_a, &[], false).is_none());
    }
}
