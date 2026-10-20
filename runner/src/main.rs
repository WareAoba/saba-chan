//! saba-runner — 게임 서버 하나를 띄우고 콘솔을 지킨다 (C8b)
//!
//! 사용법: `saba-runner --run-dir <dir> --log <file> [--cwd <dir>] [--console pipe|pty] [--tail <pattern>]... [--memory-limit <MiB>] -- <program> [args...]`
//!
//! `--console pty`: 서버를 의사 콘솔(Windows ConPTY, unix pty)에 띄운다. 출력이 파이프면
//! 끝날 때까지 모아 두는 서버(stdout 블록 버퍼링)도 줄마다 나온다. stdout·stderr는 한 갈래로
//! 합쳐지고, 터미널 제어 문자는 지운다. 기본은 `pipe`.
//!
//! Windows에서는 서버를 Job Object에 넣는다. 러너가 어떻게 끝나든(강제 종료 포함) 서버와
//! 서버가 띄운 프로세스가 같이 끝난다. 런처(곧바로 끝나고 진짜 서버를 남기는 exe)를 띄우면
//! 남은 프로세스가 끝날 때까지 기다린다.
//!
//! `--memory-limit`: 서버와 그 자식들의 커밋 메모리 합을 MiB 단위로 막는다(Windows Job). 넘으면 서버를 끝내고
//! `exit.json`의 `memory_limit_mb`에 남긴다. 다른 OS에서는 로그에 한 줄 남기고 무시한다.
//!
//! `--tail`: 서버가 콘솔 대신 파일에 쓰는 로그(예: `Pal/Saved/Logs/*.log`)를 따라 읽어
//! 같은 세션 로그에 합친다. 상대 경로는 `--cwd` 기준, `*` `?`는 마지막 칸에만.
//!
//! 데몬이 이 프로세스를 띄우고 떠나도 된다. 러너가 서버의 stdout/stderr를 로그 파일에
//! 쓰고, stdin을 로컬 포트로 받고, 종료 코드를 `exit.json`에 남긴다.
//! 파일 형식은 `saba_runner`(lib.rs)에 있다.

use saba_runner::{decode_line, exit_file, Lines, format_line, matching_files, now, runner_file, write_json, ExitInfo, RunnerInfo, Stream, TOKEN_ENV};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod job;

/// 서버 콘솔을 잇는 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Console {
    Pipe,
    Pty,
}

/// 이보다 빨리 끝나고 자식 프로세스를 남기면 런처로 본다
const LAUNCHER_WINDOW: Duration = Duration::from_secs(30);

/// 띄운 프로세스가 끝났는데 job에 프로세스가 남았을 때, 남은 것을 서버로 따라가는가.
/// 런처는 서버를 띄우고 정상 종료한다. 실패 코드로 끝났으면(DLL을 못 찾음 등) 남은 것은 오류 보고·콘솔 호스트
/// 같은 부산물이라 따라가지 않는다 (따라가면 서버가 켜진 것처럼 보인 채 시작 실패가 묻힌다)
fn follows_leftovers(success: bool, left: u32, elapsed: Duration) -> bool {
    success && left > 0 && elapsed < LAUNCHER_WINDOW
}

struct Args {
    run_dir: PathBuf,
    log: PathBuf,
    cwd: Option<PathBuf>,
    console: Console,
    tails: Vec<String>,
    /// MiB
    memory_limit: Option<u64>,
    program: String,
    program_args: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let (mut run_dir, mut log, mut cwd) = (None, None, None);
    let mut console = Console::Pipe;
    let mut tails = Vec::new();
    let mut memory_limit = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--run-dir" => run_dir = it.next().map(PathBuf::from),
            "--log" => log = it.next().map(PathBuf::from),
            "--cwd" => cwd = it.next().map(PathBuf::from),
            "--console" => {
                console = match it.next().as_deref() {
                    Some("pipe") => Console::Pipe,
                    Some("pty") => Console::Pty,
                    other => return Err(format!("--console takes pipe or pty, got {:?}", other)),
                }
            }
            "--tail" => tails.extend(it.next()),
            "--memory-limit" => {
                let value = it.next().unwrap_or_default();
                memory_limit = Some(value.parse::<u64>().ok().filter(|m| *m > 0 && m.checked_mul(1024 * 1024).is_some()).ok_or_else(|| format!("--memory-limit takes MiB, got '{}'", value))?);
            }
            "--" => break,
            other => return Err(format!("unknown argument '{}'", other)),
        }
    }
    let program = it.next().ok_or("missing program after --")?;
    Ok(Args {
        run_dir: run_dir.ok_or("missing --run-dir")?,
        log: log.ok_or("missing --log")?,
        cwd,
        console,
        tails,
        memory_limit,
        program,
        program_args: it.collect(),
    })
}

type Log = Arc<Mutex<File>>;

fn write_log(log: &Log, stream: Stream, content: &str) {
    if let Ok(mut f) = log.lock() {
        let _ = writeln!(f, "{}", format_line(now(), stream, content));
        let _ = f.flush();
    }
}

/// 자식 출력 한 갈래를 줄 단위로 로그에 옮긴다 (`decode_line`).
/// 의사 콘솔 출력은 터미널 제어 문자를 지우고, 그러고 나서 빈 줄은 버린다.
/// 의사 콘솔이 커서 위치를 묻는 요청(DSR). Windows ConPTY는 커서를 물려받는 모드로 열리면 처음에 이것을 보내고,
/// 답이 올 때까지 콘솔을 쓰는 서버(팰월드 Shipping-Cmd 등)를 멈춰 둔다
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
/// 그 답: 1행 1열
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// 줄 끝 없이 온 요청도 바로 보게, 줄 단위가 아니라 읽은 만큼 본다
fn pump(reader: impl Read + Send + 'static, log: Log, stream: Stream, console: Console, reply: Option<Arc<Stdin>>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 8192];
        // UTF-16LE 출력은 글자 안의 0A 바이트에서 자르지 않는다 (D-S13)
        let mut lines = Lines::default();
        let emit = |line: &[u8], lines: &Lines| {
            if console == Console::Pty {
                let line = decode_line(&strip_ansi_escapes::strip(line));
                // 화면을 다시 그린 자리(\r)는 마지막 내용만 남긴다
                let line = line.rsplit('\r').next().unwrap_or_default();
                if !line.trim().is_empty() {
                    write_log(&log, stream, line);
                }
            } else {
                write_log(&log, stream, &lines.decode(line));
            }
        };
        loop {
            let n = match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            // 요청이 두 번 읽기에 걸쳐 올 수 있어 남은 앞부분과 이어서 찾는다
            let scan_from = lines.pending().len().saturating_sub(CURSOR_QUERY.len() - 1);
            lines.push(&chunk[..n]);
            if let Some(stdin) = &reply {
                let asked = lines.pending()[scan_from..].windows(CURSOR_QUERY.len()).filter(|w| *w == CURSOR_QUERY).count();
                if asked > 0 {
                    if let Ok(mut guard) = stdin.lock() {
                        if let Some(pipe) = guard.as_mut() {
                            for _ in 0..asked {
                                let _ = pipe.write_all(CURSOR_REPLY);
                            }
                            let _ = pipe.flush();
                        }
                    }
                }
            }
            while let Some(line) = lines.next_line() {
                emit(&line, &lines);
            }
        }
        if let Some(rest) = lines.finish() {
            emit(&rest, &lines);
        }
    })
}

type Stdin = Mutex<Option<Box<dyn Write + Send>>>;

/// stdin 연결 하나: 첫 줄이 토큰이면 이후 줄을 자식 stdin으로 넘긴다.
/// 의사 콘솔은 Enter를 `\r`로 받는다.
fn serve_stdin(conn: TcpStream, token: &str, stdin: &Stdin, enter: &str) {
    let mut lines = BufReader::new(conn).lines();
    if lines.next().and_then(Result::ok).as_deref() != Some(token) {
        return;
    }
    for line in lines.map_while(Result::ok) {
        let mut guard = match stdin.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(pipe) = guard.as_mut() else { return };
        if write!(pipe, "{}{}", line, enter).and_then(|_| pipe.flush()).is_err() {
            *guard = None; // 서버가 stdin을 닫았다
            return;
        }
    }
}

/// 로그 파일들을 따라 읽는다. 시작 때 있던 파일은 끝에서부터, 새로 생긴 파일은 처음부터.
/// 파일이 줄어들면(새로 쓰기 시작) 처음부터 다시.
fn follow_files(patterns: Vec<PathBuf>, log: Log) {
    std::thread::spawn(move || {
        let mut offsets: std::collections::HashMap<PathBuf, u64> = patterns
            .iter()
            .flat_map(|p| matching_files(p))
            .map(|f| {
                let len = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                (f, len)
            })
            .collect();
        let mut partial: std::collections::HashMap<PathBuf, Vec<u8>> = Default::default();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            for file in patterns.iter().flat_map(|p| matching_files(p)) {
                let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                let pos = offsets.entry(file.clone()).or_insert(0);
                if len < *pos {
                    *pos = 0;
                }
                if len == *pos {
                    continue;
                }
                let Ok(mut f) = File::open(&file) else { continue };
                use std::io::Seek;
                if f.seek(std::io::SeekFrom::Start(*pos)).is_err() {
                    continue;
                }
                let mut bytes = Vec::new();
                if f.read_to_end(&mut bytes).is_err() {
                    continue;
                }
                *pos += bytes.len() as u64;
                let buf = partial.entry(file.clone()).or_default();
                buf.extend_from_slice(&bytes);
                while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&line);
                    write_log(&log, Stream::Stdout, line.trim_end_matches(['\r', '\n']));
                }
            }
        }
    });
}

/// 띄운 서버
struct Started {
    pid: u32,
    outputs: Vec<(Box<dyn Read + Send>, Stream)>,
    stdin: Box<dyn Write + Send>,
    enter: &'static str,
    proc: Proc,
}

enum Proc {
    Pipe(Child),
    Pty {
        child: Box<dyn portable_pty::Child + Send + Sync>,
        // 끝날 때 닫아야 출력 쪽이 EOF를 받는다
        master: Box<dyn portable_pty::MasterPty + Send>,
    },
}

impl Proc {
    fn wait(self) -> Result<ExitInfo, String> {
        match self {
            Proc::Pipe(mut child) => child.wait().map(|s| ExitInfo::from_status(&s)).map_err(|e| e.to_string()),
            Proc::Pty { mut child, master } => {
                let status = child.wait().map_err(|e| e.to_string())?;
                drop(master);
                // Windows 예외 코드(0xC0000005 등)는 i32로 바꿔도 비트가 같다 (데몬이 u32로 다시 읽는다)
                let code = status.exit_code() as i32;
                Ok(ExitInfo { code: Some(code), success: status.success(), ended_at: now(), signal: None, memory_limit_mb: None })
            }
        }
    }
}

fn start(args: &Args) -> Result<Started, String> {
    match args.console {
        Console::Pipe => {
            let mut cmd = Command::new(&args.program);
            cmd.args(&args.program_args)
                .env_remove(TOKEN_ENV)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(cwd) = &args.cwd {
                cmd.current_dir(cwd);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            let mut outputs: Vec<(Box<dyn Read + Send>, Stream)> = Vec::new();
            if let Some(out) = child.stdout.take() {
                outputs.push((Box::new(out), Stream::Stdout));
            }
            if let Some(err) = child.stderr.take() {
                outputs.push((Box::new(err), Stream::Stderr));
            }
            let stdin: Box<dyn Write + Send> = Box::new(child.stdin.take().ok_or("no stdin pipe")?);
            Ok(Started { pid: child.id(), outputs, stdin, enter: "\n", proc: Proc::Pipe(child) })
        }
        Console::Pty => {
            use portable_pty::{native_pty_system, CommandBuilder, PtySize};
            // 넓게 열어 서버 줄이 화면 폭에서 꺾이지 않게 한다
            let size = PtySize { rows: 50, cols: 4000, pixel_width: 0, pixel_height: 0 };
            let pair = native_pty_system().openpty(size).map_err(|e| e.to_string())?;
            let mut cmd = CommandBuilder::new(&args.program);
            cmd.args(&args.program_args);
            cmd.env_remove(TOKEN_ENV);
            if let Some(cwd) = &args.cwd {
                cmd.cwd(cwd);
            }
            let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
            drop(pair.slave);
            let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
            let stdin = pair.master.take_writer().map_err(|e| e.to_string())?;
            let pid = child.process_id().unwrap_or(0);
            Ok(Started {
                pid,
                outputs: vec![(reader, Stream::Stdout)],
                stdin,
                enter: "\r",
                proc: Proc::Pty { child, master: pair.master },
            })
        }
    }
}

fn run(args: Args) -> Result<i32, String> {
    let token = std::env::var(TOKEN_ENV).map_err(|_| format!("{} is not set", TOKEN_ENV))?;
    std::fs::create_dir_all(&args.run_dir).map_err(|e| e.to_string())?;
    if let Some(parent) = args.log.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(exit_file(&args.run_dir));

    let log: Log = Arc::new(Mutex::new(
        OpenOptions::new().create(true).append(true).open(&args.log).map_err(|e| e.to_string())?,
    ));
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let started = match start(&args) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to spawn process '{}': {}", args.program, e);
            write_log(&log, Stream::System, &msg);
            let _ = write_json(&exit_file(&args.run_dir), &ExitInfo { code: None, success: false, ended_at: now(), signal: None, memory_limit_mb: None });
            return Err(msg);
        }
    };
    let Started { pid, outputs, stdin, enter, proc } = started;
    let started_at = Instant::now();
    write_log(&log, Stream::System, &format!("Process started with PID {}", pid));

    #[cfg(windows)]
    let job = match job::Job::contain(pid, args.memory_limit.map(|mb| mb.saturating_mul(1024 * 1024))) {
        Ok(job) => Some(job),
        Err(e) => {
            write_log(&log, Stream::System, &format!("Could not put the server in a job object ({}); it may outlive saba-runner", e));
            None
        }
    };

    #[cfg(not(windows))]
    if let Some(mb) = args.memory_limit {
        write_log(&log, Stream::System, &format!("Memory limit {} MiB is only enforced on Windows; running without it", mb));
    }
    #[cfg(windows)]
    if let (Some(mb), None) = (args.memory_limit, &job) {
        write_log(&log, Stream::System, &format!("Memory limit {} MiB is not enforced because the job object failed", mb));
    }

    let info = RunnerInfo { runner_pid: std::process::id(), pid, port, log: args.log.clone(), started_at: now(), cwd: args.cwd.clone() };
    write_json(&runner_file(&args.run_dir), &info).map_err(|e| e.to_string())?;

    let stdin: Arc<Stdin> = Arc::new(Mutex::new(Some(stdin)));
    // 의사 콘솔의 커서 위치 요청에는 runner가 답한다 (파이프에는 그런 요청이 없다)
    let reply = (args.console == Console::Pty).then(|| stdin.clone());
    let readers: Vec<_> = outputs.into_iter().map(|(r, stream)| pump(r, log.clone(), stream, args.console, reply.clone())).collect();
    if !args.tails.is_empty() {
        let base = args.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
        follow_files(args.tails.iter().map(|t| base.join(t)).collect(), log.clone());
    }
    std::thread::spawn(move || {
        for conn in listener.incoming().map_while(Result::ok) {
            let (stdin, token) = (stdin.clone(), token.clone());
            std::thread::spawn(move || serve_stdin(conn, &token, &stdin, enter));
        }
    });

    #[allow(unused_mut)]
    let mut exit = proc.wait()?;
    #[cfg(windows)]
    if let Some(job) = &job {
        // 런처: 곧바로 끝나고 진짜 서버를 남겼다. 남은 프로세스가 끝날 때까지가 서버의 수명이다.
        let left = job.active_processes();
        if follows_leftovers(exit.success, left, started_at.elapsed()) {
            write_log(&log, Stream::System, &format!(
                "PID {} exited with {:?} and left {} process(es) running; following them as the server. \
                 Point the module at the real server executable to get its console.", pid, exit.code, left));
            // 데몬이 상태를 판단하는 PID를 살아 있는 서버 PID로 바꿔 둔다
            let mut followed = None;
            while job.active_processes() > 0 {
                let current = job.process_ids().into_iter().next();
                if current.is_some() && current != followed {
                    followed = current;
                    let info = RunnerInfo { pid: current.unwrap_or(pid), ..info.clone() };
                    let _ = write_json(&runner_file(&args.run_dir), &info);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            // 진짜 서버의 종료 코드는 알 수 없다
            exit = ExitInfo { code: None, success: false, ended_at: now(), signal: None, memory_limit_mb: None };
        }
    }
    #[cfg(windows)]
    if let (Some(job), Some(mb)) = (&job, args.memory_limit) {
        if job.memory_limit_hit() {
            write_log(&log, Stream::System, &format!("Memory limit of {} MiB reached; the server was stopped", mb));
            exit.memory_limit_mb = Some(mb);
            exit.success = false;
        }
    }
    #[cfg(not(windows))]
    let _ = (started_at, follows_leftovers);
    // 남은 출력을 마저 옮긴다. 손자 프로세스가 파이프를 쥐고 있으면 오래 기다리지 않는다.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for reader in readers {
            let _ = reader.join();
        }
        let _ = done_tx.send(());
    });
    let _ = done_rx.recv_timeout(std::time::Duration::from_secs(2));
    match (exit.code, exit.signal) {
        (Some(code), _) => write_log(&log, Stream::System, &format!("Process exited with code {}", code)),
        (None, Some(sig)) => write_log(&log, Stream::System, &format!("Process killed by signal {}", sig)),
        (None, None) => write_log(&log, Stream::System, "Process exited (status unknown)"),
    }
    let _ = write_json(&exit_file(&args.run_dir), &exit);
    let _ = std::fs::remove_file(runner_file(&args.run_dir));
    Ok(exit.code.unwrap_or(1))
}

fn main() {
    let code = match parse_args().and_then(run) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("saba-runner: {}", e);
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use saba_runner::wildcard as m;
    use std::time::Duration;

    #[test]
    fn only_a_launcher_that_exited_cleanly_hands_over_to_what_it_left() {
        let quick = Duration::from_secs(2);
        assert!(super::follows_leftovers(true, 1, quick));
        assert!(!super::follows_leftovers(false, 1, quick), "a crash at start is a failed start, not a launcher");
        assert!(!super::follows_leftovers(true, 0, quick));
        assert!(!super::follows_leftovers(true, 1, Duration::from_secs(60)));
    }

    /// 쓴 바이트를 모아 두는 가짜 stdin
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // 실제 PC: ConPTY가 처음 보낸 커서 위치 요청(줄 끝 없음)에 아무도 답하지 않아 팰월드가 출력 0줄로
    // 멈춰 있었다(직접 실행하면 5초 만에 REST가 뜸). 요청이 두 번 읽기에 걸쳐 와도 답한다
    #[test]
    fn a_pseudo_console_cursor_query_gets_an_answer_even_without_a_newline() {
        let path = std::env::temp_dir().join(format!("saba-runner-cursor-{}.log", std::process::id()));
        let log: super::Log = std::sync::Arc::new(std::sync::Mutex::new(std::fs::File::create(&path).unwrap()));
        let sink = Sink::default();
        let stdin: std::sync::Arc<super::Stdin> = std::sync::Arc::new(std::sync::Mutex::new(Some(Box::new(sink.clone()))));
        // 앞쪽 요청은 두 조각으로 나뉘어 온다
        struct Chunks(Vec<&'static [u8]>);
        impl std::io::Read for Chunks {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let c = self.0.remove(0);
                buf[..c.len()].copy_from_slice(c);
                Ok(c.len())
            }
        }
        let reader = Chunks(vec![b"\x1b[", b"6n", b"REST API started\r\n\x1b[6n"]);
        super::pump(reader, log, super::Stream::Stdout, super::Console::Pty, Some(stdin)).join().unwrap();
        assert_eq!(sink.0.lock().unwrap().as_slice(), b"\x1b[1;1R\x1b[1;1R");
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(text.contains("REST API started"), "{text}");
    }

    #[test]
    fn wildcard_matches_names() {
        assert!(m("*.log", "Pal.log"));
        assert!(m("Pal-?.log", "Pal-1.log"));
        assert!(!m("*.log", "Pal.txt"));
        assert!(m("*", ""));
        assert!(!m("a?", "a"));
    }
}
