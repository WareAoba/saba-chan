//! saba-runner — 게임 서버 하나를 띄우고 콘솔을 지킨다 (C8b)
//!
//! 사용법: `saba-runner --run-dir <dir> --log <file> [--cwd <dir>] [--console pipe|pty] [--tail <pattern>]... -- <program> [args...]`
//!
//! `--console pty`: 서버를 의사 콘솔(Windows ConPTY, unix pty)에 띄운다. 출력이 파이프면
//! 끝날 때까지 모아 두는 서버(stdout 블록 버퍼링)도 줄마다 나온다. stdout·stderr는 한 갈래로
//! 합쳐지고, 터미널 제어 문자는 지운다. 기본은 `pipe`.
//!
//! Windows에서는 서버를 Job Object에 넣는다. 러너가 어떻게 끝나든(강제 종료 포함) 서버와
//! 서버가 띄운 프로세스가 같이 끝난다. 런처(곧바로 끝나고 진짜 서버를 남기는 exe)를 띄우면
//! 남은 프로세스가 끝날 때까지 기다린다.
//!
//! `--tail`: 서버가 콘솔 대신 파일에 쓰는 로그(예: `Pal/Saved/Logs/*.log`)를 따라 읽어
//! 같은 세션 로그에 합친다. 상대 경로는 `--cwd` 기준, `*` `?`는 마지막 칸에만.
//!
//! 데몬이 이 프로세스를 띄우고 떠나도 된다. 러너가 서버의 stdout/stderr를 로그 파일에
//! 쓰고, stdin을 로컬 포트로 받고, 종료 코드를 `exit.json`에 남긴다.
//! 파일 형식은 `saba_runner`(lib.rs)에 있다.

use saba_runner::{decode_line, exit_file, format_line, matching_files, now, runner_file, write_json, ExitInfo, RunnerInfo, Stream, TOKEN_ENV};
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

struct Args {
    run_dir: PathBuf,
    log: PathBuf,
    cwd: Option<PathBuf>,
    console: Console,
    tails: Vec<String>,
    program: String,
    program_args: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let (mut run_dir, mut log, mut cwd) = (None, None, None);
    let mut console = Console::Pipe;
    let mut tails = Vec::new();
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
fn pump(reader: impl Read + Send + 'static, log: Log, stream: Stream, console: Console) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if console == Console::Pty {
                        let line = decode_line(&strip_ansi_escapes::strip(&buf));
                        // 화면을 다시 그린 자리(\r)는 마지막 내용만 남긴다
                        let line = line.rsplit('\r').next().unwrap_or_default();
                        if !line.trim().is_empty() {
                            write_log(&log, stream, line);
                        }
                    } else {
                        write_log(&log, stream, &decode_line(&buf));
                    }
                }
            }
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
                Ok(ExitInfo { code: Some(code), success: status.success(), ended_at: now(), signal: None })
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
            let _ = write_json(&exit_file(&args.run_dir), &ExitInfo { code: None, success: false, ended_at: now(), signal: None });
            return Err(msg);
        }
    };
    let Started { pid, outputs, stdin, enter, proc } = started;
    let started_at = Instant::now();
    write_log(&log, Stream::System, &format!("Process started with PID {}", pid));

    #[cfg(windows)]
    let job = match job::Job::contain(pid) {
        Ok(job) => Some(job),
        Err(e) => {
            write_log(&log, Stream::System, &format!("Could not put the server in a job object ({}); it may outlive saba-runner", e));
            None
        }
    };

    let info = RunnerInfo { runner_pid: std::process::id(), pid, port, log: args.log.clone(), started_at: now(), cwd: args.cwd.clone() };
    write_json(&runner_file(&args.run_dir), &info).map_err(|e| e.to_string())?;

    let readers: Vec<_> = outputs.into_iter().map(|(r, stream)| pump(r, log.clone(), stream, args.console)).collect();
    if !args.tails.is_empty() {
        let base = args.cwd.clone().unwrap_or_else(|| PathBuf::from("."));
        follow_files(args.tails.iter().map(|t| base.join(t)).collect(), log.clone());
    }
    let stdin: Arc<Stdin> = Arc::new(Mutex::new(Some(stdin)));
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
        if left > 0 && started_at.elapsed() < LAUNCHER_WINDOW {
            write_log(&log, Stream::System, &format!(
                "PID {} exited with {:?} and left {} process(es) running; following them as the server. \
                 Point the module at the real server executable to get its console.", pid, exit.code, left));
            while job.active_processes() > 0 {
                std::thread::sleep(Duration::from_secs(1));
            }
            // 진짜 서버의 종료 코드는 알 수 없다
            exit = ExitInfo { code: None, success: false, ended_at: now(), signal: None };
        }
    }
    #[cfg(not(windows))]
    let _ = (started_at, LAUNCHER_WINDOW);
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

    #[test]
    fn wildcard_matches_names() {
        assert!(m("*.log", "Pal.log"));
        assert!(m("Pal-?.log", "Pal-1.log"));
        assert!(!m("*.log", "Pal.txt"));
        assert!(m("*", ""));
        assert!(!m("a?", "a"));
    }
}
