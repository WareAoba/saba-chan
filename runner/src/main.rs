//! saba-runner — 게임 서버 하나를 띄우고 콘솔을 지킨다 (C8b)
//!
//! 사용법: `saba-runner --run-dir <dir> --log <file> [--cwd <dir>] -- <program> [args...]`
//!
//! 데몬이 이 프로세스를 띄우고 떠나도 된다. 러너가 서버의 stdout/stderr를 로그 파일에
//! 쓰고, stdin을 로컬 포트로 받고, 종료 코드를 `exit.json`에 남긴다.
//! 파일 형식은 `saba_runner`(lib.rs)에 있다.

use saba_runner::{exit_file, format_line, now, runner_file, write_json, ExitInfo, RunnerInfo, Stream, TOKEN_ENV};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

struct Args {
    run_dir: PathBuf,
    log: PathBuf,
    cwd: Option<PathBuf>,
    program: String,
    program_args: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let (mut run_dir, mut log, mut cwd) = (None, None, None);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--run-dir" => run_dir = it.next().map(PathBuf::from),
            "--log" => log = it.next().map(PathBuf::from),
            "--cwd" => cwd = it.next().map(PathBuf::from),
            "--" => break,
            other => return Err(format!("unknown argument '{}'", other)),
        }
    }
    let program = it.next().ok_or("missing program after --")?;
    Ok(Args {
        run_dir: run_dir.ok_or("missing --run-dir")?,
        log: log.ok_or("missing --log")?,
        cwd,
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

/// 자식 출력 한 갈래를 줄 단위로 로그에 옮긴다. 잘못된 UTF-8은 대체 문자로.
fn pump(reader: impl Read + Send + 'static, log: Log, stream: Stream) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    write_log(&log, stream, line.trim_end_matches(['\r', '\n']));
                }
            }
        }
    })
}

/// stdin 연결 하나: 첫 줄이 토큰이면 이후 줄을 자식 stdin으로 넘긴다.
fn serve_stdin(conn: TcpStream, token: &str, stdin: &Mutex<Option<ChildStdin>>) {
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
        if writeln!(pipe, "{}", line).and_then(|_| pipe.flush()).is_err() {
            *guard = None; // 서버가 stdin을 닫았다
            return;
        }
    }
}

fn spawn_child(args: &Args) -> std::io::Result<Child> {
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
    cmd.spawn()
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

    let mut child = match spawn_child(&args) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to spawn process '{}': {}", args.program, e);
            write_log(&log, Stream::System, &msg);
            let _ = write_json(&exit_file(&args.run_dir), &ExitInfo { code: None, success: false, ended_at: now() });
            return Err(msg);
        }
    };
    write_log(&log, Stream::System, &format!("Process started with PID {}", child.id()));

    let info = RunnerInfo { runner_pid: std::process::id(), pid: child.id(), port, log: args.log.clone(), started_at: now() };
    write_json(&runner_file(&args.run_dir), &info).map_err(|e| e.to_string())?;

    let readers = [
        child.stdout.take().map(|s| pump(s, log.clone(), Stream::Stdout)),
        child.stderr.take().map(|s| pump(s, log.clone(), Stream::Stderr)),
    ];
    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    {
        let stdin = stdin.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().map_while(Result::ok) {
                let (stdin, token) = (stdin.clone(), token.clone());
                std::thread::spawn(move || serve_stdin(conn, &token, &stdin));
            }
        });
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    // 남은 출력을 마저 옮긴다. 손자 프로세스가 파이프를 쥐고 있으면 오래 기다리지 않는다.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for reader in readers.into_iter().flatten() {
            let _ = reader.join();
        }
        let _ = done_tx.send(());
    });
    let _ = done_rx.recv_timeout(std::time::Duration::from_secs(2));
    write_log(&log, Stream::System, &format!("Process exited with {}", status));
    let exit = ExitInfo { code: status.code(), success: status.success(), ended_at: now() };
    let _ = write_json(&exit_file(&args.run_dir), &exit);
    let _ = std::fs::remove_file(runner_file(&args.run_dir));
    Ok(status.code().unwrap_or(1))
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
