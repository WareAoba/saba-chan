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
    /// 서버 작업 폴더. 크래시 파일(`crash_artifacts`)을 찾는 기준
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExitInfo {
    /// 프로세스 종료 코드. 시그널로 끝나 코드가 없으면 None
    pub code: Option<i32>,
    pub success: bool,
    pub ended_at: u64,
    /// 시그널로 끝났으면 그 번호 (unix)
    #[serde(default)]
    pub signal: Option<i32>,
    /// 메모리 상한(`--memory-limit`, MiB)을 넘어 러너가 서버를 끝냈으면 그 상한
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u64>,
}

impl ExitInfo {
    pub fn from_status(status: &std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(status);
        #[cfg(not(unix))]
        let signal = None;
        Self { code: status.code(), success: status.success(), ended_at: now(), signal, memory_limit_mb: None }
    }
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

/// 마지막 칸의 `*`(아무 글자 여럿)와 `?`(한 글자)만 아는 이름 비교
pub fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    go(&pattern.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>())
}

/// `dir/이름패턴`에 맞는 파일들 (정렬). 와일드카드는 마지막 칸에만.
pub fn matching_files(pattern: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (pattern.parent(), pattern.file_name()) else { return Vec::new() };
    let name = name.to_string_lossy();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| p.file_name().is_some_and(|n| wildcard(&name, &n.to_string_lossy())))
        .collect();
    files.sort();
    files
}

/// 자식 출력 한 줄(바이트)을 글자로. 줄 끝 `\r\n`은 뗀다.
///
/// 어떤 Windows 서버는 첫 줄을 UTF-16LE로 쓰고 이후는 UTF-8로 쓴다. 바이트 절반 가까이가
/// 홀수 자리 0이면 UTF-16LE로 읽는다. UTF-16 줄의 `\n` 뒤에 남은 0 바이트가 다음 줄
/// 앞에 붙으므로 앞뒤의 NUL은 버린다. 잘못된 UTF-8은 대체 문자로.
pub fn decode_line(bytes: &[u8]) -> String {
    let bytes = trim_nul(bytes);
    decode_as(bytes, odd_nuls(bytes))
}

/// 바이트 절반 가까이가 홀수 자리 0인가 (UTF-16LE로 쓴 ASCII)
fn odd_nuls(bytes: &[u8]) -> bool {
    let nuls = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    bytes.len() >= 2 && nuls * 2 >= bytes.len() / 2
}

fn decode_as(bytes: &[u8], utf16: bool) -> String {
    let text = if utf16 {
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_string()
    } else {
        String::from_utf8_lossy(bytes).trim_start_matches('\u{feff}').to_string()
    };
    text.trim_end_matches(['\r', '\n', '\0']).to_string()
}

/// `\n` 바이트(0A)를 찾은 자리가 줄 끝인가 (`Lines::check`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// 줄이 끝났다
    Line,
    /// UTF-16 글자의 한 바이트다 (예: U+4E0A 上 = 0A 4E). 다음 `\n`까지 더 읽는다
    Char,
    /// 다음 바이트가 0이면 UTF-16 줄 끝(0A 00)이고 그 0까지 이 줄이다. 0이 아니면 글자의 한 바이트다. 스트림이 끝났으면 줄 끝이다
    Peek,
}

/// 자식 출력 한 갈래를 줄로 자른다. UTF-16LE 출력은 `\n` 바이트만 보고 자르면 0A·00이 낀 글자(上, 一 등)에서 줄이
/// 어긋난다 (D-S13). 줄마다 UTF-16LE인지 정한다. NUL이 적은 한자 줄이나 빈 앞부분처럼 바이트만으로 정하기 어려우면 앞 줄의 판단을 잇되,
/// NUL이 하나도 없으면 UTF-8(또는 ANSI 코드 페이지)로 본다: 첫 줄만 UTF-16LE로 쓰고 이후는 다른 인코딩으로 쓰는 서버가 있다
#[derive(Debug, Default)]
pub struct Lines {
    utf16: bool,
}

impl Lines {
    /// `line`: 지금 줄의 처음부터 방금 찾은 `\n`까지 (포함)
    pub fn check(&mut self, line: &[u8]) -> End {
        let body = &line[..line.len().saturating_sub(1)];
        self.utf16 = match body.len() {
            0 => self.utf16,
            // UTF-16 줄이면 `\n`이 홀수 자리에 올 수 없다
            1 => false,
            _ => body.starts_with(&[0xff, 0xfe]) || odd_nuls(body) || (self.utf16 && body.contains(&0)),
        };
        match (self.utf16, body.len() % 2) {
            (false, _) => End::Line,
            (true, 1) => End::Char,
            (true, _) => End::Peek,
        }
    }

    /// `buf` 앞의 첫 줄이 끝나는 자리(포함). 아직 끝나지 않았으면 None.
    /// `from`: 이미 본 자리. 다음 바이트를 기다리는 동안 같은 `buf`를 처음부터 다시 보지 않는다 (줄을 꺼내면 0으로 돌린다)
    pub fn line_end(&mut self, buf: &[u8], from: &mut usize) -> Option<usize> {
        while let Some(i) = buf.get(*from..)?.iter().position(|b| *b == b'\n') {
            let nl = *from + i;
            let end = match self.check(&buf[..=nl]) {
                End::Line => nl,
                End::Char => {
                    *from = nl + 1;
                    continue;
                }
                End::Peek => match buf.get(nl + 1) {
                    Some(0) => nl + 1,
                    Some(_) => {
                        *from = nl + 1;
                        continue;
                    }
                    None => {
                        *from = nl;
                        return None;
                    }
                },
            };
            *from = 0;
            return Some(end);
        }
        *from = buf.len();
        None
    }

    /// `check`·`line_end`로 자른 줄을 글자로. 줄 끝 `\r\n`은 뗀다
    pub fn decode(&self, line: &[u8]) -> String {
        if self.utf16 {
            decode_as(line, true)
        } else {
            decode_line(line)
        }
    }
}

fn trim_nul(mut b: &[u8]) -> &[u8] {
    while let [0, rest @ ..] = b {
        b = rest;
    }
    while let [rest @ .., 0] = b {
        b = rest;
    }
    b
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
    fn lines_decode_utf16_and_utf8() {
        let utf16: Vec<u8> = "\u{feff}Server starting\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        // read_until(b'\n')가 자른 모양: 앞 줄은 `\n`에서 끝나고, 그 뒤 0은 다음 줄 앞에 붙는다
        let nl = utf16.iter().rposition(|b| *b == b'\n').unwrap();
        assert_eq!(decode_line(&utf16[..=nl]), "Server starting");
        let mut next = utf16[nl + 1..].to_vec();
        next.extend_from_slice("다음 줄\n".as_bytes());
        assert_eq!(decode_line(&next), "다음 줄");
        assert_eq!(decode_line(b"plain\r\n"), "plain");
        assert_eq!(decode_line(b""), "");
    }

    fn split(bytes: &[u8]) -> Vec<String> {
        let mut lines = Lines::default();
        let (mut buf, mut from, mut out) = (bytes.to_vec(), 0, Vec::new());
        while let Some(end) = lines.line_end(&buf, &mut from) {
            let line: Vec<u8> = buf.drain(..=end).collect();
            out.push(lines.decode(&line));
        }
        if !buf.is_empty() {
            out.push(lines.decode(&buf));
        }
        out
    }

    /// UTF-16LE 줄은 0A·00이 낀 글자(上 = 0A 4E, 一 = 00 4E)에서 끊기지 않는다. 첫 줄만 UTF-16LE인 출력도 그대로 읽는다 (D-S13)
    #[test]
    fn utf16_lines_split_only_at_their_own_line_ends() {
        let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect() };
        assert_eq!(split(&utf16("\u{feff}Ready 上一\r\n上\r\n一番\r\nend\r\n")), ["Ready 上一", "上", "一番", "end"]);
        let mut mixed = utf16("\u{feff}Server starting\r\n");
        mixed.extend_from_slice("다음 줄\na\n".as_bytes());
        assert_eq!(split(&mixed), ["Server starting", "다음 줄", "a"]);
        assert_eq!(split(b"plain\r\nb\n\nlast"), ["plain", "b", "", "last"]);
    }

    #[test]
    fn json_files_round_trip() {
        let dir = std::env::temp_dir().join(format!("saba-runner-proto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let info = RunnerInfo { runner_pid: 1, pid: 2, port: 3, log: dir.join("a.log"), started_at: 4, cwd: None };
        write_json(&runner_file(&dir), &info).unwrap();
        assert_eq!(read_json::<RunnerInfo>(&runner_file(&dir)), Some(info));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
