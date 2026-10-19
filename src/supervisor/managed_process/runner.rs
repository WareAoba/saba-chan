//! saba-runner로 띄운 서버에 붙기 (C8b)
//!
//! 러너(`runner/` 크레이트)가 서버를 자식으로 쥐고 로그 파일·stdin 포트·`exit.json`을 남긴다.
//! 데몬은 그 파일과 포트에 붙을 뿐이라, 데몬이 재시작해도 `attach_runner`로 콘솔 기록,
//! stdin, 종료 코드를 그대로 다시 잡는다. 프로토콜은 `saba_runner`에 있다.
//!
//! 러너 실행 파일이 없으면(개발 중 빌드 누락 등) 호출 쪽이 예전 직접 실행(`spawn`)으로 돌아간다.

use super::*;
use crate::supervisor::module_loader::ConsoleMode;
use saba_runner::{exit_file, read_json, runner_file, ExitInfo, RunnerInfo, TOKEN_ENV};
use std::io::{Read, Seek, SeekFrom};

const RUN_DIR: &str = "run";
const TOKEN_FILE: &str = "token";
const SESSIONS_DIR: &str = "sessions";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// 세션 로그 기본 보관 개수
pub const DEFAULT_LOG_SESSIONS: usize = 20;

/// `<instance>/run/`
pub fn run_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join(RUN_DIR)
}

/// 러너가 지금 서버로 보는 PID. 런처가 끝나고 남긴 서버를 따라가면 러너가 runner.json을 고친다
pub fn runner_server_pid(instance_dir: &Path) -> Option<u32> {
    read_json::<RunnerInfo>(&runner_file(&run_dir(instance_dir))).map(|i| i.pid)
}

/// 시작 중 서버가 메모리 상한에 걸려 끝났다. 호출 쪽이 `downcast_ref`로 알아본다
#[derive(Debug)]
pub struct MemoryLimitAtStart {
    pub limit_mb: u64,
    pub exit_code: Option<i32>,
}

impl std::fmt::Display for MemoryLimitAtStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The server reached its memory limit of {} MiB while starting; set memory_limit_mb above its RAM setting or clear it", self.limit_mb)
    }
}

impl std::error::Error for MemoryLimitAtStart {}

/// runner.json 전에 exit.json이 생겼을 때의 시작 오류. 상한 때문이면 그렇다고, 아니면 로그 마지막 줄
fn start_failure(run: &Path, log: &Path) -> anyhow::Error {
    if let Some(exit) = read_json::<ExitInfo>(&exit_file(run)) {
        if let Some(limit_mb) = exit.memory_limit_mb {
            return MemoryLimitAtStart { limit_mb, exit_code: exit.code }.into();
        }
    }
    let last = read_log_tail(log, 1, 64 * 1024).and_then(|(lines, _)| lines.into_iter().next());
    anyhow::anyhow!("{}", last.as_deref().unwrap_or("saba-runner exited"))
}

/// `<instance>/logs/sessions/`
pub fn sessions_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join("logs").join(SESSIONS_DIR)
}

/// 러너 실행 파일. `SABA_RUNNER_PATH`가 있으면 그것, 없으면 데몬 실행 파일 옆(또는 한 단계 위).
pub fn runner_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SABA_RUNNER_PATH").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let exe = std::env::current_exe().ok()?;
    let name = format!("saba-runner{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|dir| dir.join(&name))
        .find(|p| p.is_file())
}

/// 세션 로그를 최근 `keep`개만 남긴다 (이름이 시작 시각이라 이름 순이 시간 순).
pub fn prune_sessions(instance_dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(sessions_dir(instance_dir)) else { return };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

/// 가장 최근 세션 로그. 없으면 None.
pub fn latest_session_log(instance_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(sessions_dir(instance_dir))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .max()
}

/// 러너에 넘기는 모듈 `[launch]` 선언
#[derive(Debug, Clone, Copy, Default)]
pub struct RunnerLaunch<'a> {
    pub log_files: &'a [String],
    pub console: ConsoleMode,
    /// 인스턴스 메모리 상한(MiB) → `--memory-limit`
    pub memory_limit_mb: Option<u64>,
}

impl ManagedProcess {
    /// 러너로 서버를 띄우고 붙는다.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_with_runner(
        runner: &Path,
        program: &str,
        args: &[String],
        working_dir: &str,
        env_vars: Vec<(String, String)>,
        log_pattern: Option<&str>,
        instance_dir: &Path,
        keep_sessions: usize,
        launch: RunnerLaunch<'_>,
    ) -> Result<Self> {
        let run = run_dir(instance_dir);
        std::fs::create_dir_all(&run)?;
        let _ = std::fs::remove_file(runner_file(&run));
        let _ = std::fs::remove_file(exit_file(&run));
        let token = uuid::Uuid::new_v4().to_string();
        std::fs::write(run.join(TOKEN_FILE), &token)?;

        let log = sessions_dir(instance_dir).join(format!("{}.log", saba_runner::session_stamp(current_timestamp())));
        std::fs::create_dir_all(sessions_dir(instance_dir))?;
        prune_sessions(instance_dir, keep_sessions.saturating_sub(1));

        let mut cmd = TokioCommand::new(runner);
        cmd.arg("--run-dir").arg(&run)
            .arg("--log").arg(&log)
            .arg("--cwd").arg(working_dir)
            .arg("--console").arg(launch.console.as_arg());
        for pattern in launch.log_files {
            cmd.arg("--tail").arg(pattern);
        }
        if let Some(mb) = launch.memory_limit_mb {
            cmd.arg("--memory-limit").arg(mb.to_string());
        }
        cmd.arg("--")
            .arg(program)
            .args(args)
            .envs(env_vars)
            .env(TOKEN_ENV, &token)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(false);
        // 데몬의 터미널 시그널(Ctrl+C)이 서버까지 가지 않게 따로 프로세스 그룹을 둔다
        #[cfg(unix)]
        cmd.process_group(0);
        // Windows: 데몬을 띄운 쪽(예: Electron)의 Job Object가 닫혀도 러너가 살아남도록 빠져나온다.
        // Job이 빠져나가기를 허용하지 않으면 그 플래그 없이 다시 띄운다.
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        #[cfg(not(windows))]
        crate::utils::apply_creation_flags(&mut cmd);

        let spawned = cmd.spawn();
        #[cfg(windows)]
        let spawned = match spawned {
            Err(e) if e.raw_os_error() == Some(5) => {
                tracing::warn!("saba-runner cannot leave the parent job ({}); its server stops if the daemon's parent closes", e);
                cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
                cmd.spawn()
            }
            other => other,
        };
        let mut child = spawned
            .map_err(|e| anyhow::anyhow!("Failed to spawn saba-runner '{}': {}", runner.display(), e))?;
        let runner_pid = child.id();
        let reaped = tokio::spawn(async move {
            let _ = child.wait().await; // 데몬이 사는 동안 좀비를 남기지 않는다
        });

        // 러너가 서버를 띄우고 runner.json을 쓸 때까지
        for _ in 0..RUNNER_START_TICKS {
            if let Some(info) = read_json::<RunnerInfo>(&runner_file(&run)) {
                return Ok(Self::from_runner(info, token, run, log_pattern, Vec::new(), 0, None));
            }
            if exit_file(&run).exists() {
                return Err(start_failure(&run, &log));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // 늦게라도 서버를 띄우면 데몬이 모르는 서버가 남고, 다시 시작하면 같은 폴더·포트에 두 번째 서버가 뜬다.
        // 러너와 그 자식을 끝내고(Unix는 러너의 프로세스 그룹, Windows는 트리) 러너가 끝난 것을 본 뒤 실패를 돌려준다
        if let Some(pid) = runner_pid {
            crate::plugin::kill_tree(pid);
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), reaped).await;
        Err(anyhow::anyhow!("saba-runner did not start '{}' in time", program))
    }

    /// 데몬 재시작 뒤, 아직 살아 있는 러너에 다시 붙는다. 러너가 없거나 끝났으면 None.
    pub async fn attach_runner(instance_dir: &Path, log_pattern: Option<&str>) -> Option<Self> {
        let run = run_dir(instance_dir);
        let info: RunnerInfo = read_json(&runner_file(&run))?;
        // 재부팅 뒤 그 PID를 다른 프로세스가 쓰면 시작 시각이 기록보다 늦다: 그것에 붙거나 그것을 끄지 않는다
        if exit_file(&run).exists() || !crate::supervisor::process::is_same_process(info.runner_pid, info.started_at) {
            return None;
        }
        let token = std::fs::read_to_string(run.join(TOKEN_FILE)).ok()?.trim().to_string();

        let path = info.log.clone();
        let (history, offset) = tokio::task::spawn_blocking(move || {
            // 끝부분만 읽는다 (러너 로그는 서버가 도는 내내 자란다)
            let (lines, len) = read_log_tail(&path, DEFAULT_LOG_BUFFER, LOG_TAIL_MAX_BYTES).unwrap_or_default();
            let history: Vec<LogLine> = lines.iter().enumerate().map(|(i, raw)| parse_disk_log_line(i as u64, raw)).collect();
            (history, len)
        })
        .await
        .ok()?;

        let notice = format!("── Daemon restarted. Reattached to saba-runner (server PID {}). ──", info.pid);
        Some(Self::from_runner(info, token, run, log_pattern, history, offset, Some(notice)))
    }

    fn from_runner(
        info: RunnerInfo,
        token: String,
        run: PathBuf,
        log_pattern: Option<&str>,
        history: Vec<LogLine>,
        offset: u64,
        notice: Option<String>,
    ) -> Self {
        let mut buf = LogBuffer::new();
        buf.next_id = history.last().map(|l| l.id + 1).unwrap_or(0);
        buf.lines.extend(history);
        if let Some(msg) = notice {
            buf.push(LogSource::System, msg, LogLevel::Info);
        }
        let log_buffer = Arc::new(Mutex::new(buf));
        let (running_tx, running_rx) = watch::channel(true);
        // 러너가 끝났다는 신호. 로그 따라 읽기를 마저 끝낸 뒤에 running을 내린다.
        let (ended_tx, ended_rx) = watch::channel(false);
        let exit_slot: Arc<std::sync::Mutex<Option<ProcessExit>>> = Default::default();
        let regex = log_pattern.and_then(|p| Regex::new(p).ok());

        // 러너 로그 파일을 따라 읽어 콘솔 버퍼로 (러너가 디스크 기록을 맡는다)
        let tail = {
            let buf = log_buffer.clone();
            let path = info.log.clone();
            let ended = ended_rx;
            tokio::spawn(async move {
                let mut pos = offset;
                let mut partial: Vec<u8> = Vec::new();
                loop {
                    let stopped = *ended.borrow();
                    let path = path.clone();
                    let chunk = tokio::task::spawn_blocking(move || read_from(&path, pos)).await.ok().flatten();
                    if let Some(bytes) = chunk {
                        pos += bytes.len() as u64;
                        partial.extend_from_slice(&bytes);
                        while let Some(nl) = partial.iter().position(|b| *b == b'\n') {
                            let raw: Vec<u8> = partial.drain(..=nl).collect();
                            let raw = String::from_utf8_lossy(&raw);
                            let mut line = parse_disk_log_line(0, raw.trim_end_matches(['\r', '\n']));
                            line.level = match line.source {
                                LogSource::System => line.level,
                                source => {
                                    let level = parse_log_level(&line.content, regex.as_ref());
                                    if source == LogSource::Stderr && level == LogLevel::Info { LogLevel::Warn } else { level }
                                }
                            };
                            buf.lock().await.push(line.source, line.content, line.level);
                        }
                    }
                    if stopped {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            })
        };

        // 종료 감시: exit.json이 생기거나 러너가 사라지면 끝
        {
            let running = running_tx;
            let buf = log_buffer.clone();
            let run = run.clone();
            let runner_pid = info.runner_pid;
            let exit_slot = exit_slot.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let exit: Option<ExitInfo> = read_json(&exit_file(&run));
                    if exit.is_some() || !crate::supervisor::process::is_process_alive(runner_pid) {
                        *exit_slot.lock().unwrap_or_else(|e| e.into_inner()) = exit.as_ref().map(ProcessExit::from);
                        if exit.is_none() {
                            buf.lock().await.push(LogSource::System, "saba-runner exited without an exit record".into(), LogLevel::Warn);
                        }
                        let _ = ended_tx.send(true);
                        let _ = tail.await; // 마지막 출력까지 버퍼에 옮긴 뒤 끝났다고 알린다
                        let _ = running.send(false);
                        break;
                    }
                }
            });
        }

        // stdin: 러너 포트로 한 줄씩. 연결이 끊기면 다음 명령 때 다시 붙는다.
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(256);
        {
            let port = info.port;
            tokio::spawn(async move {
                let mut conn: Option<tokio::net::TcpStream> = None;
                while let Some(cmd) = stdin_rx.recv().await {
                    let line = format!("{}\n", cmd.trim_end_matches(['\r', '\n']));
                    for _attempt in 0..2 {
                        if conn.is_none() {
                            conn = connect_stdin(port, &token).await;
                        }
                        let Some(stream) = conn.as_mut() else { break };
                        if stream.write_all(line.as_bytes()).await.is_ok() {
                            break;
                        }
                        conn = None;
                    }
                }
            });
        }

        Self {
            stdin_tx,
            log_buffer,
            pid: info.pid,
            running_rx,
            poller_handle: None,
            stdin_available: true,
            exit: exit_slot,
            started_at: info.started_at,
            reattached: false,
            working_dir: info.cwd.clone(),
            log_follower: false,
            runner_pid: Some(info.runner_pid),
        }
    }
}

/// 러너가 runner.json을 쓰기까지 기다리는 횟수 (100ms 간격)
#[cfg(not(test))]
const RUNNER_START_TICKS: u32 = 100;
#[cfg(test)]
const RUNNER_START_TICKS: u32 = 50;

async fn connect_stdin(port: u16, token: &str) -> Option<tokio::net::TcpStream> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
    stream.write_all(format!("{}\n", token).as_bytes()).await.ok()?;
    Some(stream)
}

/// `pos`부터 파일 끝까지. 새 내용이 없으면 None.
fn read_from(path: &Path, pos: u64) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() <= pos {
        return None;
    }
    file.seek(SeekFrom::Start(pos)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn wait_for(proc: &ManagedProcess, text: &str) -> bool {
        for _ in 0..50 {
            if proc.get_recent_console(100).await.iter().any(|l| l.content == text) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// 시간 안에 서버를 띄우지 못한 러너는 그 자식까지 끝낸다: 늦게 뜬 서버 옆에 두 번째 서버가 뜨지 않게 (D-S4)
    #[tokio::test]
    async fn a_runner_that_never_reports_is_stopped_with_its_server() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let pids = inst.join("pids");
        // runner.json을 쓰지 않고 서버(자식)만 띄우는 러너
        let fake = inst.join("fake-runner.sh");
        std::fs::write(&fake, format!("#!/bin/sh\nsleep 60 &\necho \"$$ $!\" > '{}'\nwait\n", pids.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let started = ManagedProcess::spawn_with_runner(&fake, "server", &[], inst.to_str().unwrap(), Vec::new(), None, inst, 3, RunnerLaunch::default()).await;
        assert!(started.is_err());
        let ids: Vec<u32> = std::fs::read_to_string(&pids).unwrap().split_whitespace().map(|p| p.parse().unwrap()).collect();
        tokio::time::sleep(Duration::from_millis(300)).await;
        for pid in ids {
            // 끝났지만 아직 거둬지지 않은 프로세스(좀비)는 끝난 것으로 본다
            let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
            assert!(zombie || !crate::supervisor::process::is_process_alive(pid), "process {pid} must be gone after the failed start");
        }
    }

    /// 데몬이 떨어졌다 다시 붙어도 콘솔 기록과 stdin이 이어지고, 끝나면 종료 기록이 남는다
    #[tokio::test]
    async fn runner_keeps_console_and_stdin_across_reattach() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();

        let proc = ManagedProcess::spawn_with_runner(&runner, "cat", &[], inst.to_str().unwrap(), Vec::new(), None, inst, 3, RunnerLaunch::default())
            .await
            .unwrap();
        proc.send_command("before restart").await.unwrap();
        assert!(wait_for(&proc, "before restart").await);
        let pid = proc.pid;
        drop(proc); // 데몬 재시작 흉내

        let again = ManagedProcess::attach_runner(inst, None).await.expect("runner still alive");
        assert_eq!(again.pid, pid);
        assert!(wait_for(&again, "before restart").await, "history comes back from the session log");
        assert!(again.is_stdin_available());
        again.send_command("after restart").await.unwrap();
        assert!(wait_for(&again, "after restart").await);

        crate::supervisor::process::force_kill_pid(pid).unwrap();
        for _ in 0..50 {
            if !again.is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!again.is_running());
        let exit: ExitInfo = read_json(&exit_file(&run_dir(inst))).expect("exit.json");
        assert!(!exit.success);
        assert!(ManagedProcess::attach_runner(inst, None).await.is_none());
        assert!(latest_session_log(inst).is_some());
    }

    /// 모듈이 [launch] log_files로 밝힌 파일 로그도 콘솔에 합쳐진다
    #[tokio::test]
    async fn declared_log_files_join_the_console() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        std::fs::write(inst.join("game.log"), "old line\n").unwrap();
        let script = "sleep 1; echo 'from the log file' >> game.log; sleep 5".to_string();
        let proc = ManagedProcess::spawn_with_runner(
            &runner, "sh", &["-c".into(), script], inst.to_str().unwrap(), Vec::new(), None, inst, 3,
            RunnerLaunch { log_files: &["*.log".into()], console: ConsoleMode::Pipe, ..Default::default() },
        )
        .await
        .unwrap();
        assert!(wait_for(&proc, "from the log file").await);
        assert!(!proc.get_recent_console(100).await.iter().any(|l| l.content == "old line"), "only new lines");
        crate::supervisor::process::force_kill_pid(proc.pid).unwrap();
    }

    /// [launch] console = "pty": 파이프면 출력을 모아 두는 서버도 줄마다 나오고, stdin도 의사 콘솔로 간다
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_console_streams_block_buffered_output() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        // python은 stdout이 파이프면 블록 버퍼링한다. 첫 줄은 끝나기 한참 전에 보여야 한다.
        let script = "import sys, time\nprint('early line')\nfor line in sys.stdin:\n    print('got ' + line.strip())\n    time.sleep(30)".to_string();
        let env = vec![("PYTHONUNBUFFERED".to_string(), String::new())];
        let proc = ManagedProcess::spawn_with_runner(
            &runner, "python3", &["-c".into(), script], inst.to_str().unwrap(), env, None, inst, 3,
            RunnerLaunch { log_files: &[], console: ConsoleMode::Pty, ..Default::default() },
        )
        .await
        .unwrap();
        assert!(wait_for(&proc, "early line").await, "output arrives while the server runs");
        proc.send_command("ping").await.unwrap();
        assert!(wait_for(&proc, "got ping").await);
        let _ = proc.force_kill();
    }

    #[test]
    fn old_session_logs_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["20260101-000000.log", "20260102-000000.log", "20260103-000000.log"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        prune_sessions(tmp.path(), 2);
        let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        left.sort();
        assert_eq!(left, ["20260102-000000.log", "20260103-000000.log"]);
        assert_eq!(latest_session_log(tmp.path()), Some(dir.join("20260103-000000.log")));
    }
}

#[cfg(test)]
mod start_failure_tests {
    use super::*;

    #[test]
    fn start_failure_names_the_memory_limit_from_the_exit_record() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("console.log");
        std::fs::write(&log, "[SYSTEM/INFO] Process exited with code 8\n").unwrap();
        let exit = ExitInfo { code: Some(8), success: false, ended_at: 0, signal: None, memory_limit_mb: Some(512) };
        saba_runner::write_json(&exit_file(tmp.path()), &exit).unwrap();
        let err = start_failure(tmp.path(), &log);
        assert_eq!(err.downcast_ref::<MemoryLimitAtStart>().map(|e| (e.limit_mb, e.exit_code)), Some((512, Some(8))));

        let exit = ExitInfo { memory_limit_mb: None, ..exit };
        saba_runner::write_json(&exit_file(tmp.path()), &exit).unwrap();
        let err = start_failure(tmp.path(), &log);
        assert!(err.downcast_ref::<MemoryLimitAtStart>().is_none());
        assert!(err.to_string().contains("code 8"), "{}", err);
    }
}
