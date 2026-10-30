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
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    // 감시 루프가 2초마다 부른다. 목록을 들고 있어야 `OnlyIfNotSet`이 새로 뜬 프로세스의 경로·커맨드라인만 읽는다
    // (매번 새로 만들면 모든 프로세스를 다시 열어 읽는다). 끝난 프로세스는 refresh가 빼고, PID 재사용은 시작 시각으로 가린다
    static SYSTEM: std::sync::LazyLock<std::sync::Mutex<System>> = std::sync::LazyLock::new(|| std::sync::Mutex::new(System::new()));
    let mut sys = SYSTEM.lock().unwrap_or_else(|e| e.into_inner());
    // 기본 refresh_processes()는 커맨드라인·작업 폴더를 읽지 않는다: 자동 감지(cmd_patterns, 주인 가리기)에 필요하다
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet),
    );
    
    let processes: Vec<RunningProcess> = sys.processes()
        .iter()
        .map(|(pid, process)| {
            RunningProcess {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                executable_path: process.exe().and_then(|p| p.to_str()).map(String::from),
                cmd: process.cmd().iter().map(|arg| arg.to_string_lossy().into_owned()).collect(),
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

/// 프로세스의 실행 파일이나 작업 폴더가 `folder` 아래인가 (그 폴더에서 도는 서버). 인수는 보지 않는다:
/// 그 폴더의 파일을 연 편집기까지 서버로 보지 않게
pub fn runs_from(process: &RunningProcess, folder: &std::path::Path) -> bool {
    let root = folder.to_string_lossy();
    process.executable_path.iter().chain(process.cwd.iter()).any(|p| under(p, &root))
}

/// 여러 인스턴스가 함께 쓰는 폴더(예: 같은 설치 폴더의 실행 파일)는 주인을 가리는 근거가 못 된다: 양쪽에서 뺀다
pub fn drop_shared_roots(own: Vec<String>, other: Vec<String>) -> (Vec<String>, Vec<String>) {
    let own_n: Vec<String> = own.iter().map(|r| normalize_path(r)).collect();
    let other_n: Vec<String> = other.iter().map(|r| normalize_path(r)).collect();
    let own_kept = own.into_iter().zip(&own_n).filter(|(_, n)| !other_n.contains(n)).map(|(r, _)| r).collect();
    let other_kept = other.into_iter().zip(&other_n).filter(|(_, n)| !own_n.contains(n)).map(|(r, _)| r).collect();
    (own_kept, other_kept)
}

/// 자동 감지 후보 중 이 인스턴스의 프로세스를 고른다.
/// - `taken`: 다른 인스턴스가 이미 추적하는 PID — 절대 고르지 않는다
/// - `own_roots`: 이 인스턴스의 폴더들, `other_roots`: 같은 모듈의 다른 인스턴스 폴더들
///
/// 이 인스턴스 폴더 아래의 프로세스만, 다른 인스턴스 폴더 아래의 것은 빼고 고른다.
/// 같은 모듈 인스턴스가 하나뿐이어도 폴더가 안 맞으면 고르지 않는다: java.exe처럼 흔한 이름은
/// 이 PC의 다른 프로그램(다른 데이터 폴더의 서버, 버전 확인 등)일 수 있다
pub fn pick_owned_process<'a>(
    candidates: &[&'a RunningProcess],
    taken: &std::collections::HashSet<u32>,
    own_roots: &[String],
    other_roots: &[String],
) -> Option<&'a RunningProcess> {
    candidates
        .iter()
        .copied()
        .filter(|p| !taken.contains(&p.pid))
        .find(|p| process_under_any(p, own_roots) && !process_under_any_more_specific(p, own_roots, other_roots))
}

/// 프로세스가 다른 인스턴스 폴더에도 들어맞으면 이 인스턴스 것이 아니다.
/// - 작업 폴더가 어느 인스턴스 폴더 안에 있으면 그것이 가장 강한 근거다: 가장 깊이 맞는 쪽이 주인
/// - 아니면 다른 쪽 폴더에 맞는 순간 빠진다. 다만 다른 쪽 폴더가 이쪽의 더 깊은 폴더를 감싸면(겹친 폴더) 이쪽이 주인
fn process_under_any_more_specific(process: &RunningProcess, own_roots: &[String], other_roots: &[String]) -> bool {
    let deepest = |roots: &[String], path: &str| roots.iter().filter(|r| under(path, r)).map(|r| normalize_path(r).len()).max();
    if let Some(cwd) = process.cwd.as_deref() {
        match (deepest(own_roots, cwd), deepest(other_roots, cwd)) {
            (Some(own), other) => return other.is_some_and(|o| o >= own),
            (None, Some(_)) => return true,
            (None, None) => {}
        }
    }
    let own_hits: Vec<&String> = own_roots.iter().filter(|r| process_under_any(process, std::slice::from_ref(*r))).collect();
    other_roots.iter().filter(|r| process_under_any(process, std::slice::from_ref(r))).any(|r| {
        !own_hits.iter().any(|o| normalize_path(o).len() > normalize_path(r).len() && under(o, r))
    })
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

    /// 목록을 들고 있어도 나중에 뜬 프로세스는 커맨드라인까지 잡히고, 끝난 프로세스는 빠진다
    #[cfg(unix)]
    #[test]
    fn a_later_process_is_listed_with_its_command_and_drops_out_after_it_ends() {
        get_running_processes();
        let mut child = std::process::Command::new("sleep").arg("31.4159").spawn().unwrap();
        let pid = child.id();
        let listed = get_running_processes().into_iter().find(|p| p.pid == pid).expect("new process listed");
        assert!(listed.cmd.iter().any(|a| a == "31.4159"), "{:?}", listed.cmd);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(get_running_processes().iter().all(|p| p.pid != pid));
    }

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
    fn a_server_runs_from_its_folder_but_an_editor_with_a_file_open_there_does_not() {
        let folder = std::path::Path::new(r"D:\게임 서버\pal");
        assert!(runs_from(&proc(1, r"D:\게임 서버\pal\Pal\Binaries"), folder));
        let editor = RunningProcess { cmd: vec!["notepad.exe".into(), r"D:\게임 서버\pal\a.ini".into()], cwd: Some(r"C:\Users\me".into()), ..proc(2, "") };
        assert!(!runs_from(&editor, folder));
        assert!(!runs_from(&proc(3, r"D:\게임 서버\palworld"), folder));
    }

    #[test]
    fn auto_detect_assigns_each_process_to_its_own_instance() {
        let a = proc(10, r"C:\Saba\instances\a");
        let b = proc(20, r"C:\Saba\instances\b");
        let cands = vec![&a, &b];
        let roots_a = vec![r"C:\Saba\instances\a".to_string()];
        let roots_b = vec![r"C:\Saba\instances\b".to_string()];
        let none = HashSet::new();

        assert_eq!(pick_owned_process(&cands, &none, &roots_b, &roots_a).map(|p| p.pid), Some(20));
        // B만 남은 인스턴스로 A 하나만 돌고 있으면 B는 아무것도 잡지 않는다
        assert!(pick_owned_process(&[&a], &none, &roots_b, &roots_a).is_none());
        // 다른 인스턴스가 이미 잡은 PID는 고르지 않는다
        let taken: HashSet<u32> = [10].into();
        assert!(pick_owned_process(&[&a], &taken, &roots_a, &[]).is_none());
        // 같은 모듈 인스턴스가 하나뿐이어도 다른 폴더에서 도는 같은 이름 프로세스는 가져가지 않는다
        let elsewhere = proc(11, r"C:\SomewhereElse\server");
        assert!(pick_owned_process(&[&elsewhere], &none, &roots_a, &[]).is_none());
        assert!(pick_owned_process(&[&a], &none, &[], &[]).is_none());
        // 같은 실행 파일 폴더를 함께 쓰면 그 폴더로는 고르지 않고 작업 폴더로 가린다
        let shared = |pid: u32, cwd: &str| RunningProcess {
            pid,
            name: "server.exe".into(),
            executable_path: Some(r"C:\Games\bin\server.exe".into()),
            cmd: vec![],
            cwd: Some(cwd.into()),
        };
        let (sa, sb) = (shared(40, r"C:\Saba\instances\a"), shared(50, r"C:\Saba\instances\b"));
        let (own_a, own_b) = drop_shared_roots(
            vec![r"C:\Games\bin".to_string(), r"C:\Saba\instances\a".to_string()],
            vec![r"C:\Games\bin".to_string(), r"C:\Saba\instances\b".to_string()],
        );
        assert_eq!(pick_owned_process(&[&sb, &sa], &none, &own_a, &own_b).map(|p| p.pid), Some(40));
        assert!(pick_owned_process(&[&sb], &none, &own_a, &own_b).is_none());
        // 작업 폴더가 공유 실행 폴더보다 짧아도, 공유 폴더를 빼면 각자 자기 서버를 찾는다
        let short = |pid: u32, cwd: &str| RunningProcess {
            pid,
            name: "server.exe".into(),
            executable_path: Some(r"C:\Program Files\Game\bin\server.exe".into()),
            cmd: vec![],
            cwd: Some(cwd.into()),
        };
        let (pa, pb) = (short(60, r"C:\A"), short(70, r"C:\B"));
        let bin = r"C:\Program Files\Game\bin".to_string();
        let (own, other) = drop_shared_roots(vec![bin.clone(), r"C:\A".into()], vec![bin.clone(), r"C:\B".into()]);
        assert_eq!(own, vec![r"C:\A".to_string()]);
        assert_eq!(pick_owned_process(&[&pb, &pa], &none, &own, &other).map(|p| p.pid), Some(60));
        let (own_b, other_b) = drop_shared_roots(vec![bin.clone(), r"C:\B".into()], vec![bin, r"C:\A".into()]);
        assert_eq!(pick_owned_process(&[&pa, &pb], &none, &own_b, &other_b).map(|p| p.pid), Some(70));
        // A의 실행 폴더에만 맞고 작업 폴더가 B 폴더면 A는 가져가지 않는다
        let odd = RunningProcess {
            pid: 80,
            name: "server.exe".into(),
            executable_path: Some(r"C:\shared\bin\server.exe".into()),
            cmd: vec![],
            cwd: Some(r"C:\b".into()),
        };
        let own_odd = vec![r"C:\shared\bin".to_string(), r"C:\a".to_string()];
        let other_odd = vec![r"C:\b".to_string()];
        assert!(pick_owned_process(&[&odd], &none, &own_odd, &other_odd).is_none());
        assert_eq!(pick_owned_process(&[&odd], &none, &other_odd, &own_odd).map(|p| p.pid), Some(80));
        // 인스턴스 폴더가 다른 인스턴스 폴더 안에 있으면 더 깊은 쪽이 주인이다
        let nested = proc(90, r"C:\Saba\outer\inner");
        let (outer, inner) = (vec![r"C:\Saba\outer".to_string()], vec![r"C:\Saba\outer\inner".to_string()]);
        assert_eq!(pick_owned_process(&[&nested], &none, &inner, &outer).map(|p| p.pid), Some(90));
        assert!(pick_owned_process(&[&nested], &none, &outer, &inner).is_none());
        // "a"가 "ab"의 접두어여도 섞이지 않는다
        let ab = proc(30, r"C:\Saba\instances\ab");
        assert!(pick_owned_process(&[&ab], &none, &roots_a, &[]).is_none());
    }
}
