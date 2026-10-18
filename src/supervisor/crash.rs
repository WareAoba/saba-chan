//! 서버가 왜 끝났는지 가르고, 크래시면 다시 띄울지 정한다 (A1)
//!
//! 판정 순서:
//! 1. 데몬이 정지를 보냈다 (GUI·CLI·디스코드의 정지, 재시작) → `user_stop`
//! 2. 크래시 흔적 → `crash`
//!    - Windows 예외 종료 코드 (0xC0000005 같은 0xC… 값, Ctrl+C 종료 0xC000013A는 뺀다)
//!    - unix 크래시 시그널 (SIGSEGV, SIGABRT, SIGBUS, SIGFPE, SIGILL)
//!    - 모듈 `[detection] crash_artifacts`에 맞는 파일이 이번 실행 중에 생겼다
//! 3. 모듈 `[detection] graceful_exit_patterns`가 콘솔 끝부분에 있다 → `in_game_stop`
//! 4. 시작하고 `STARTUP_GRACE` 안에 끝났다 (포트 충돌·설정 오류 같은 시작 실패) → `start_failed`. 다시 띄워도 같으므로 재시작하지 않는다
//! 5. 종료 코드 0 → `in_game_stop`
//! 6. 그 밖 (작업 관리자 강제 종료, 창 닫기, 원인 모를 종료) → `unknown_exit`
//!
//! 사용자 종료와 강제 종료를 크래시로 잘못 보지 않는 것이 우선이라, 흔적이 없으면 크래시로 보지 않는다.

use super::managed_process::ProcessExit;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 재시작 횟수를 세는 구간
pub const RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);
/// 구간 안에서 최대 재시작 횟수. 넘으면 포기하고 사용자에게 맡긴다
pub const MAX_RESTARTS: usize = 3;
/// 첫 재시작 대기. 재시작할 때마다 두 배
pub const FIRST_RESTART_DELAY: Duration = Duration::from_secs(5);
/// 콘솔 끝에서 정상 종료 문구를 찾는 줄 수
pub const CONSOLE_TAIL_LINES: usize = 50;
/// 시작하고 이 안에 끝나면 시작에 실패한 것으로 본다
pub const STARTUP_GRACE: Duration = Duration::from_secs(30);
/// 인스턴스마다 남기는 종료 이력 수
pub const EVENTS_KEPT: usize = 20;

/// Windows STATUS_CONTROL_C_EXIT: Ctrl+C나 콘솔 창 닫기
const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
#[cfg(target_os = "linux")]
const CRASH_SIGNALS: &[i32] = &[4, 6, 7, 8, 11];
#[cfg(not(target_os = "linux"))]
const CRASH_SIGNALS: &[i32] = &[4, 6, 10, 8, 11];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKind {
    UserStop,
    InGameStop,
    Crash,
    StartFailed,
    UnknownExit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExitVerdict {
    pub kind: ExitKind,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// 판정 근거 한 줄
    pub evidence: String,
}

/// 판정에 쓰는 사실들
pub struct ExitFacts<'a> {
    pub stop_requested: bool,
    pub exit: Option<ProcessExit>,
    pub console_tail: &'a [String],
    pub graceful_patterns: &'a [String],
    pub new_crash_files: &'a [PathBuf],
    /// 시작부터 끝날 때까지 (초)
    pub ran_secs: u64,
}

pub fn classify(f: &ExitFacts) -> ExitVerdict {
    let (exit_code, signal) = f.exit.map(|e| (e.code, e.signal)).unwrap_or((None, None));
    let verdict = |kind, evidence: String| ExitVerdict { kind, exit_code, signal, evidence };

    if f.stop_requested {
        return verdict(ExitKind::UserStop, "stop requested through saba-chan".into());
    }
    if let Some(mb) = f.exit.and_then(|e| e.memory_limit_mb) {
        return verdict(ExitKind::Crash, format!("memory limit {} MiB reached", mb));
    }
    if let Some(code) = exit_code {
        let raw = code as u32;
        if raw & 0xF000_0000 == 0xC000_0000 && raw != STATUS_CONTROL_C_EXIT {
            return verdict(ExitKind::Crash, format!("exit code 0x{:08X}", raw));
        }
    }
    if let Some(sig) = signal.filter(|s| CRASH_SIGNALS.contains(s)) {
        return verdict(ExitKind::Crash, format!("signal {}", sig));
    }
    if let Some(file) = f.new_crash_files.first() {
        return verdict(ExitKind::Crash, format!("crash file {}", file.display()));
    }
    for pattern in f.graceful_patterns {
        let Ok(re) = regex::Regex::new(pattern) else { continue };
        if let Some(line) = f.console_tail.iter().rev().find(|l| re.is_match(l)) {
            return verdict(ExitKind::InGameStop, format!("console: {}", line.trim()));
        }
    }
    if f.ran_secs < STARTUP_GRACE.as_secs() {
        let last = f.console_tail.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty());
        let evidence = match last {
            Some(line) => format!("ended {}s after start: {}", f.ran_secs, line),
            None => format!("ended {}s after start", f.ran_secs),
        };
        return verdict(ExitKind::StartFailed, evidence);
    }
    match (exit_code, signal) {
        (Some(0), _) => verdict(ExitKind::InGameStop, "exit code 0".into()),
        (Some(code), _) => verdict(ExitKind::UnknownExit, format!("exit code {}", code)),
        (None, Some(sig)) => verdict(ExitKind::UnknownExit, format!("signal {}", sig)),
        (None, None) => verdict(ExitKind::UnknownExit, "exit status unknown".into()),
    }
}

/// `crash_artifacts` 패턴(작업 폴더 기준)에 맞고 `since`(unix 초) 이후에 바뀐 파일
pub fn new_crash_files(working_dir: &Path, patterns: &[String], since: u64) -> Vec<PathBuf> {
    let since = std::time::UNIX_EPOCH + Duration::from_secs(since);
    patterns
        .iter()
        .flat_map(|p| saba_runner::matching_files(&working_dir.join(p)))
        .filter(|f| std::fs::metadata(f).and_then(|m| m.modified()).is_ok_and(|t| t >= since))
        .collect()
}

/// 이 판정으로 다시 띄우는가. `restart_on_unknown_exit`이면 원인 모를 종료도 다시 띄운다.
pub fn wants_restart(kind: ExitKind, auto_restart: bool, restart_on_unknown_exit: bool) -> bool {
    auto_restart && (kind == ExitKind::Crash || (kind == ExitKind::UnknownExit && restart_on_unknown_exit))
}

/// 최근 재시작 기록으로 다음 대기 시간을 정한다. 한도를 넘었으면 None.
pub fn next_restart_delay(history: &mut VecDeque<Instant>, now: Instant) -> Option<Duration> {
    while history.front().is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW) {
        history.pop_front();
    }
    if history.len() >= MAX_RESTARTS {
        return None;
    }
    let delay = FIRST_RESTART_DELAY * 2u32.pow(history.len() as u32);
    history.push_back(now);
    Some(delay)
}

/// PC 밖(디스코드 알림·원격 조회)으로 보내도 되는 종료 근거: 종료 코드·신호·메모리 상한만.
/// 콘솔 줄(`console: …`, `ended …: <줄>`)과 크래시 파일 경로는 PC에 남긴다 (I3)
pub fn shareable_evidence(evidence: &str) -> bool {
    ["exit code", "signal", "memory limit"].iter().any(|p| evidence.starts_with(p))
}

/// GET /api/instance/:id/events 한 줄. 인스턴스 폴더 `exit-events.json`에 남아 데몬을 다시 켜도 보인다
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitEvent {
    /// unix 초
    pub at: u64,
    #[serde(flatten)]
    pub verdict: ExitVerdict,
    /// 다시 띄우기로 했으면 대기 초
    pub restart_in_secs: Option<u64>,
    /// 다시 띄우지 않은 이유 (자동 재시작 꺼짐, 한도 초과 등). 영어 문장, 화면 문구는 `note_code`로 고른다
    pub note: Option<String>,
    /// `note`의 종류: `auto_restart_off` · `gave_up`(`note_args.count`) · `restart_failed`(`note_args.reason`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_args: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(exit: Option<ProcessExit>, tail: &'a [String], files: &'a [PathBuf]) -> ExitFacts<'a> {
        static PATTERNS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        ExitFacts {
            stop_requested: false,
            exit,
            console_tail: tail,
            graceful_patterns: PATTERNS.get_or_init(|| vec!["Stopping the server".into()]),
            new_crash_files: files,
            ran_secs: 600,
        }
    }

    fn code(c: i32) -> Option<ProcessExit> {
        Some(ProcessExit { code: Some(c), ..Default::default() })
    }

    #[test]
    fn exits_are_classified_by_evidence() {
        let none: [String; 0] = [];
        let no_files: [PathBuf; 0] = [];
        let k = |f: ExitFacts| classify(&f).kind;

        let mut user = facts(code(0xC000_0005u32 as i32), &none, &no_files);
        user.stop_requested = true;
        assert_eq!(k(user), ExitKind::UserStop, "a stop we sent wins over everything");

        assert_eq!(k(facts(code(0xC000_0005u32 as i32), &none, &no_files)), ExitKind::Crash);
        assert_eq!(k(facts(code(0xC000_013Au32 as i32), &none, &no_files)), ExitKind::UnknownExit, "Ctrl+C / closed window");
        assert_eq!(k(facts(Some(ProcessExit { signal: Some(11), ..Default::default() }), &none, &no_files)), ExitKind::Crash);
        assert_eq!(k(facts(Some(ProcessExit { signal: Some(9), ..Default::default() }), &none, &no_files)), ExitKind::UnknownExit, "SIGKILL is a kill");
        let over = facts(Some(ProcessExit { code: Some(8), memory_limit_mb: Some(4096), ..Default::default() }), &none, &no_files);
        assert_eq!(classify(&over).kind, ExitKind::Crash);
        assert_eq!(classify(&over).evidence, "memory limit 4096 MiB reached");

        let crash_file = [PathBuf::from("crash-reports/crash.txt")];
        let stopped = ["[Server thread/INFO]: Stopping the server".to_string()];
        assert_eq!(k(facts(code(0), &stopped, &crash_file)), ExitKind::Crash, "a fresh crash report beats the stop line");
        assert_eq!(k(facts(code(1), &stopped, &no_files)), ExitKind::InGameStop);
        assert_eq!(k(facts(code(0), &none, &no_files)), ExitKind::InGameStop);
        assert_eq!(k(facts(code(1), &none, &no_files)), ExitKind::UnknownExit, "task manager kill");
        assert_eq!(k(facts(None, &none, &no_files)), ExitKind::UnknownExit);

        let bind = ["**** FAILED TO BIND TO PORT!".to_string(), "[Server thread/INFO]: Stopping server".to_string()];
        let early = |exit, tail| ExitFacts { ran_secs: 4, ..facts(exit, tail, &no_files) };
        assert_eq!(classify(&early(code(0), &bind)).kind, ExitKind::StartFailed, "ending right after start is a failed start");
        assert_eq!(classify(&early(code(0), &bind)).evidence, "ended 4s after start: [Server thread/INFO]: Stopping server");
        assert_eq!(k(early(code(0xC000_0005u32 as i32), &bind)), ExitKind::Crash, "crash evidence still wins");
        assert_eq!(k(early(code(0), &stopped)), ExitKind::InGameStop, "an in-game stop right after start is still a stop");
        assert!(!wants_restart(ExitKind::StartFailed, true, true), "restarting would fail the same way");
    }

    #[test]
    fn restart_policy() {
        assert!(wants_restart(ExitKind::Crash, true, false));
        assert!(!wants_restart(ExitKind::Crash, false, true));
        assert!(!wants_restart(ExitKind::UnknownExit, true, false));
        assert!(wants_restart(ExitKind::UnknownExit, true, true));
        assert!(!wants_restart(ExitKind::InGameStop, true, true));
        assert!(!wants_restart(ExitKind::UserStop, true, true));

        let mut history = VecDeque::new();
        let t0 = Instant::now();
        assert_eq!(next_restart_delay(&mut history, t0), Some(Duration::from_secs(5)));
        assert_eq!(next_restart_delay(&mut history, t0), Some(Duration::from_secs(10)));
        assert_eq!(next_restart_delay(&mut history, t0), Some(Duration::from_secs(20)));
        assert_eq!(next_restart_delay(&mut history, t0), None, "gives up after three in the window");
        let later = t0 + RESTART_WINDOW + Duration::from_secs(1);
        assert_eq!(next_restart_delay(&mut history, later), Some(Duration::from_secs(5)), "window slides");
    }

    #[test]
    fn crash_files_must_be_new() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("crash-reports")).unwrap();
        std::fs::write(dir.path().join("crash-reports/a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("hs_err_pid1.log"), "x").unwrap();
        let patterns = vec!["crash-reports/*.txt".to_string(), "hs_err_pid*.log".to_string()];
        assert_eq!(new_crash_files(dir.path(), &patterns, 0).len(), 2);
        let future = saba_runner::now() + 3600;
        assert!(new_crash_files(dir.path(), &patterns, future).is_empty(), "files older than the start are ignored");
    }
}
