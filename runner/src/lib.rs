//! saba-runner 프로토콜 — 러너와 데몬이 같이 쓰는 파일 이름·형식
//!
//! 러너는 게임 서버 하나를 자식으로 띄우고, 데몬은 러너에 붙었다 떨어질 뿐이다.
//! 그래서 데몬이 재시작해도 서버의 콘솔 로그, stdin, 종료 코드가 이어진다.
//!
//! 실행 디렉터리(`<instance>/run/`)에 남기는 파일:
//! - `runner.json` (`RunnerInfo`): 러너가 살아 있는 동안. 자식 PID, stdin 포트, 로그 파일.
//! - `exit.json` (`ExitInfo`): 자식이 끝나면. 종료 코드. 크래시 판별(A1)이 읽는다.
//!
//! stdin: 러너가 127.0.0.1 임의 포트를 연다. 연결의 첫 줄은 토큰(환경 변수
//! `SABA_RUNNER_TOKEN`으로 받은 값), 이후 한 줄이 명령 하나다.
//!
//! 로그: 세션마다 파일 하나, 한 줄 형식은 `format_line` (데몬의 콘솔 로그 형식과 같다).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const RUNNER_FILE: &str = "runner.json";
pub const EXIT_FILE: &str = "exit.json";
pub const TOKEN_ENV: &str = "SABA_RUNNER_TOKEN";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunnerInfo {
    pub runner_pid: u32,
    /// 게임 서버(자식) PID
    pub pid: u32,
    /// stdin 연결 포트 (127.0.0.1)
    pub port: u16,
    /// 이 세션의 콘솔 로그 파일
    pub log: PathBuf,
    pub started_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExitInfo {
    /// 프로세스 종료 코드. 시그널로 끝나 코드가 없으면 None
    pub code: Option<i32>,
    pub success: bool,
    pub ended_at: u64,
}

pub fn runner_file(run_dir: &Path) -> PathBuf {
    run_dir.join(RUNNER_FILE)
}

pub fn exit_file(run_dir: &Path) -> PathBuf {
    run_dir.join(EXIT_FILE)
}

/// 다른 쪽이 반쯤 쓴 파일을 읽지 않도록 임시 파일에 쓰고 이름을 바꾼다.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// 출력 갈래
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
    System,
}

impl Stream {
    fn tag(self) -> &'static str {
        match self {
            Stream::Stdout => "STDOUT/INFO",
            Stream::Stderr => "STDERR/WARN",
            Stream::System => "SYSTEM/INFO",
        }
    }
}

/// 로그 한 줄: `[2026-03-05 12:34:56] [STDOUT/INFO] 내용` (UTC)
pub fn format_line(ts: u64, stream: Stream, content: &str) -> String {
    format!("[{}] [{}] {}", format_timestamp(ts), stream.tag(), content)
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Unix 시각을 `YYYY-MM-DD HH:MM:SS`(UTC)로
pub fn format_timestamp(ts: u64) -> String {
    let (y, m, d) = civil_from_days((ts / 86400) as i64);
    let t = ts % 86400;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, t / 3600, (t % 3600) / 60, t % 60)
}

/// 세션 로그 파일 이름에 쓰는 `YYYYMMDD-HHMMSS`(UTC)
pub fn session_stamp(ts: u64) -> String {
    let (y, m, d) = civil_from_days((ts / 86400) as i64);
    let t = ts % 86400;
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", y, m, d, t / 3600, (t % 3600) / 60, t % 60)
}

fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe as i64 + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_format_in_utc() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(format_timestamp(1_772_713_496), "2026-03-05 12:24:56");
        assert_eq!(session_stamp(1_772_713_496), "20260305-122456");
    }

    #[test]
    fn json_files_round_trip() {
        let dir = std::env::temp_dir().join(format!("saba-runner-proto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let info = RunnerInfo { runner_pid: 1, pid: 2, port: 3, log: dir.join("a.log"), started_at: 4 };
        write_json(&runner_file(&dir), &info).unwrap();
        assert_eq!(read_json::<RunnerInfo>(&runner_file(&dir)), Some(info));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
