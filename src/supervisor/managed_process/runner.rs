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
/// 인스턴스 하나의 지난 세션 로그를 모두 합친 크기 상한. 서버를 켤 때 오래된 세션부터 지워 맞춘다
pub const SESSION_LOGS_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// 이 데몬이 붙을 수 있는 러너 형식(`saba_runner::RunnerInfo::format`) 범위, 양 끝 포함.
/// 발행 때 manifest `saba-core.reads_runner_formats`로 실린다. 켜진 러너가 새 코어의 범위 밖이면 코어를 바꾸지 않는다 (`core_swap`)
#[allow(dead_code)] // 발행 스크립트가 소스에서 읽는다. 바이너리에서는 아직 부르는 곳이 없다
pub const READS_RUNNER_FORMATS: (u32, u32) = (0, 1);

/// `<state_dir>/run/`. `state_dir`은 인스턴스의 데몬 전용 기록 폴더(`InstanceStore::state_dir`)다
pub fn run_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(RUN_DIR)
}

/// 러너가 지금 서버로 보는 PID. 런처가 끝나고 남긴 서버를 따라가면 러너가 runner.json을 고친다
pub fn runner_server_pid(state_dir: &Path) -> Option<u32> {
    read_json::<RunnerInfo>(&runner_file(&run_dir(state_dir))).map(|i| i.pid)
}

/// 더 따라가지 않는 러너 기록 폴더를 치운다: 러너가 남기는 파일만 지우고 비면 폴더도 지운다. 링크는 따라가지 않는다
pub fn clear_run_dir(run: &Path) {
    if !std::fs::symlink_metadata(run).is_ok_and(|m| m.is_dir()) {
        return;
    }
    for file in [runner_file(run), exit_file(run), run.join(TOKEN_FILE)] {
        let _ = std::fs::remove_file(file);
    }
    let _ = std::fs::remove_dir(run);
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

/// 관리형 시작인데 saba-runner 실행 파일이 없다. 관리형 시작은 러너가 콘솔을 쥐는 시작이라 데몬이 대신 띄우지 않는다.
/// 호출 쪽이 `downcast_ref`로 알아본다
#[derive(Debug)]
pub struct RunnerMissing;

impl std::fmt::Display for RunnerMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "saba-runner was not found next to saba-core, so managed start is not possible. Turn off managed start for this server or repair the installation")
    }
}

impl std::error::Error for RunnerMissing {}

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

/// 세션 파일 이름(`<시각>.log` 또는 앞 조각 `<시각>.log.<n>`)에서 세션 이름 `<시각>`
fn session_of(file_name: &str) -> Option<&str> {
    let (session, rest) = file_name.split_once(".log")?;
    let piece = rest.strip_prefix('.').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    (rest.is_empty() || piece).then_some(session)
}

/// 세션 로그를 최근 `keep`개만, 모두 합쳐 `max_bytes`까지만 남긴다 (이름이 시작 시각이라 이름 순이 시간 순).
/// 세션은 앞 조각(`<시각>.log.<n>`)까지 한 묶음으로 지운다
/// 가장 최근 세션은 크기 때문에 지우지 않는다 (개수로만 지운다): 방금 끝난 실행의 로그는 다시 켠 뒤에도 볼 수 있어야 한다
pub fn prune_sessions(instance_dir: &Path, keep: usize, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(sessions_dir(instance_dir)) else { return };
    let mut sessions: std::collections::BTreeMap<String, (Vec<PathBuf>, u64)> = Default::default();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(session) = name.to_str().and_then(session_of) else { continue };
        let slot = sessions.entry(session.to_string()).or_default();
        slot.0.push(entry.path());
        slot.1 += entry.metadata().map(|m| m.len()).unwrap_or(0);
    }
    let mut total: u64 = sessions.values().map(|(_, size)| size).sum();
    let mut left = sessions.len();
    for (files, size) in sessions.into_values() {
        if left <= keep && (total <= max_bytes || left == 1) {
            break;
        }
        for file in &files {
            let _ = std::fs::remove_file(file);
        }
        left -= 1;
        total -= size;
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
    /// `[launch] line_start` → `--line-start`
    pub line_start: Option<&'a str>,
    /// `[extensions.saba-core] progress_patterns` → `--progress`
    pub progress_patterns: &'a [String],
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
        state_dir: &Path,
        keep_sessions: usize,
        launch: RunnerLaunch<'_>,
    ) -> Result<Self> {
        let run = run_dir(state_dir);
        std::fs::create_dir_all(&run)?;
        let _ = std::fs::remove_file(runner_file(&run));
        let _ = std::fs::remove_file(exit_file(&run));
        let token = uuid::Uuid::new_v4().to_string();
        saba_atomic_file::write(&run.join(TOKEN_FILE), token.as_bytes(), saba_atomic_file::Mode::Secret)?;

        let log = sessions_dir(instance_dir).join(format!("{}.log", saba_runner::session_stamp(current_timestamp())));
        std::fs::create_dir_all(sessions_dir(instance_dir))?;
        prune_sessions(instance_dir, keep_sessions.saturating_sub(1), SESSION_LOGS_MAX_BYTES);

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
        if let Some(pattern) = launch.line_start {
            cmd.arg("--line-start").arg(pattern);
        }
        for pattern in launch.progress_patterns {
            cmd.arg("--progress").arg(pattern);
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
        #[cfg_attr(not(windows), allow(unused_mut))] // 살아 있는지 보는 try_wait는 Windows에서만
        let mut child = spawned
            .map_err(|e| anyhow::anyhow!("Failed to spawn saba-runner '{}': {}", runner.display(), e))?;
        let runner_pid = child.id();
        // 러너를 다 볼 때까지 거두지 않는다: 거두지 않은 러너의 pid(Unix는 프로세스 그룹 id)는 다른 프로세스가 받지 못한다
        let reap = |mut child: tokio::process::Child| {
            tokio::spawn(async move {
                let _ = child.wait().await; // 데몬이 사는 동안 좀비를 남기지 않는다
            })
        };

        // 러너가 서버를 띄우고 runner.json을 쓸 때까지
        for _ in 0..RUNNER_START_TICKS {
            if let Some(info) = read_json::<RunnerInfo>(&runner_file(&run)) {
                reap(child);
                return Ok(Self::from_runner(info, token, run, log_pattern, Vec::new(), 0, None));
            }
            if exit_file(&run).exists() {
                reap(child);
                return Err(start_failure(&run, &log));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // 늦게라도 서버를 띄우면 데몬이 모르는 서버가 남고, 다시 시작하면 같은 폴더·포트에 두 번째 서버가 뜬다.
        // 러너와 그 자식을 끝내고(Unix는 러너의 프로세스 그룹, Windows는 트리) 러너가 끝난 것을 본 뒤 실패를 돌려준다.
        // Unix: 러너가 보고 없이 먼저 죽었어도 아직 거두지 않았으니 그 그룹에 남은 서버만 끝난다.
        // Windows: 트리는 살아 있는 러너에서만 따라갈 수 있다
        #[cfg(windows)]
        let runner_pid = runner_pid.filter(|_| matches!(child.try_wait(), Ok(None)));
        if let Some(pid) = runner_pid {
            let _ = tokio::task::spawn_blocking(move || crate::plugin::kill_tree(pid)).await;
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), reap(child)).await;
        Err(anyhow::anyhow!("saba-runner did not start '{}' in time", program))
    }

    /// 데몬 재시작 뒤, 아직 살아 있는 러너에 다시 붙는다. 러너가 없거나 끝났으면 None.
    pub async fn attach_runner(state_dir: &Path, log_pattern: Option<&str>) -> Option<Self> {
        let run = run_dir(state_dir);
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
        // 다시 붙었다는 안내는 남기지 않는다 (데몬을 켤 때마다 쌓인다). 이 뒤로 데몬이 넣는 줄만 세션 로그에 덧붙인다
        buf.session_log = Some(info.log.clone());
        let log_buffer = Arc::new(Mutex::new(buf));
        let (running_tx, running_rx) = watch::channel(true);
        // 러너가 끝났다는 신호. 로그 따라 읽기를 마저 끝낸 뒤에 running을 내린다.
        let (ended_tx, ended_rx) = watch::channel(false);
        let exit_slot: Arc<std::sync::Mutex<Option<ProcessExit>>> = Default::default();
        let regex = log_level_regex(log_pattern);

        // 러너 로그 파일을 따라 읽어 콘솔 버퍼로 (러너가 디스크 기록을 맡는다)
        let tail = {
            let buf = log_buffer.clone();
            let path = info.log.clone();
            let ended = ended_rx;
            tokio::spawn(async move {
                let mut partial: Vec<u8> = Vec::new();
                // 연 파일을 계속 쓴다: 서버마다 100ms마다 파일을 다시 열지 않게
                let mut follow = Some(Follow { path, file: None, pos: offset });
                loop {
                    let stopped = *ended.borrow();
                    let Some(mut reading) = follow.take() else { break };
                    let chunk = tokio::task::spawn_blocking(move || {
                        let bytes = reading.read_new();
                        (reading, bytes)
                    })
                    .await
                    .ok();
                    let chunk = chunk.and_then(|(kept, bytes)| {
                        follow = Some(kept);
                        bytes
                    });
                    if let Some(bytes) = chunk {
                        partial.extend_from_slice(&bytes);
                        // 줄마다 버퍼를 앞으로 당기지 않는다: 위치만 옮기며 자르고, 남은 꼬리는 끝에서 한 번만 정리한다
                        let mut start = 0;
                        while let Some(nl) = partial[start..].iter().position(|b| *b == b'\n') {
                            let raw = String::from_utf8_lossy(&partial[start..start + nl + 1]);
                            start += nl + 1;
                            let raw = raw.trim_end_matches(['\r', '\n']);
                            if buf.lock().await.take_own(raw) {
                                continue;
                            }
                            let mut line = parse_disk_log_line(0, raw);
                            line.level = match line.source {
                                LogSource::System => line.level,
                                source => {
                                    let level = parse_log_level(&line.content, regex.as_ref());
                                    if source == LogSource::Stderr && level == LogLevel::Info { LogLevel::Warn } else { level }
                                }
                            };
                            let mut buf = buf.lock().await;
                            if !(line.source == LogSource::Stdout && buf.take_echo(&line.content)) {
                                buf.push_at(line.timestamp, line.source, line.content, line.level);
                            }
                        }
                        partial.drain(..start);
                    }
                    if stopped {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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

        let token_for_interrupt = token.clone();
        // stdin: 러너 포트로 한 줄씩. 연결이 끊기면 다음 명령 때 다시 붙는다.
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<super::StdinLine>(256);
        {
            let port = info.port;
            tokio::spawn(async move {
                let mut conn: Option<tokio::net::TcpStream> = None;
                while let Some((cmd, delivered)) = stdin_rx.recv().await {
                    let line = format!("{}\n", cmd.trim_end_matches(['\r', '\n']));
                    let mut sent = false;
                    for _attempt in 0..2 {
                        if conn.is_none() {
                            conn = connect_stdin(port, &token).await;
                        }
                        let Some(stream) = conn.as_mut() else { break };
                        if stream.write_all(line.as_bytes()).await.is_ok() {
                            sent = true;
                            break;
                        }
                        conn = None;
                    }
                    // 러너에 닿지 못한 명령은 버리되, 보낸 쪽에 실패로 알린다
                    let _ = delivered.send(if sent { Ok(()) } else { Err("saba-runner is not reachable".to_string()) });
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
            runner_format: Some(info.format),
            runner_control: Some((info.port, token_for_interrupt)),
        }
    }
}

/// 러너가 runner.json을 쓰기까지 기다리는 횟수 (100ms 간격)
#[cfg(not(test))]
const RUNNER_START_TICKS: u32 = 100;
#[cfg(test)]
const RUNNER_START_TICKS: u32 = 50;

/// 러너가 인터럽트의 답을 주기까지 기다리는 가장 긴 시간 (Windows는 콘솔에 붙었다 떨어지는 데 0.2초쯤 든다)
const INTERRUPT_ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// 러너에 서버 인터럽트를 부탁하고 답(`ok` / `error: 까닭`)을 읽는다
pub(crate) async fn ask_interrupt(port: u16, token: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncBufReadExt;
    let asked = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(format!("{}\n", saba_runner::interrupt_request(token)).as_bytes()).await?;
        let mut answer = String::new();
        tokio::io::BufReader::new(stream).read_line(&mut answer).await?;
        Ok::<_, std::io::Error>(answer)
    };
    let answer = tokio::time::timeout(INTERRUPT_ANSWER_WAIT, asked)
        .await
        .map_err(|_| anyhow::anyhow!("saba-runner did not answer the interrupt within {}s", INTERRUPT_ANSWER_WAIT.as_secs()))?
        .map_err(|e| anyhow::anyhow!("saba-runner is not reachable: {}", e))?;
    let answer = answer.trim_end_matches(['\r', '\n']);
    if answer == saba_runner::INTERRUPT_OK {
        return Ok(());
    }
    // 빈 답: 이 기능이 없는 옛 러너는 토큰이 다르다며 그냥 닫는다
    let reason = answer.strip_prefix("error: ").unwrap_or(if answer.is_empty() { "saba-runner closed the connection (too old for Ctrl+C?)" } else { answer });
    anyhow::bail!("Ctrl+C was not sent: {}", reason)
}

async fn connect_stdin(port: u16, token: &str) -> Option<tokio::net::TcpStream> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
    stream.write_all(format!("{}\n", token).as_bytes()).await.ok()?;
    Some(stream)
}

/// 따라 읽는 세션 로그와 다음에 읽을 자리
struct Follow {
    path: PathBuf,
    file: Option<std::fs::File>,
    pos: u64,
}

impl Follow {
    /// 지난번 뒤로 쓴 바이트. 없으면 None.
    /// 러너가 파일을 앞 조각으로 넘겼으면(`saba_runner::session_log_piece`) 쥐고 있던 파일을 끝까지 읽은 다음 새 파일을 처음부터 읽는다
    fn read_new(&mut self) -> Option<Vec<u8>> {
        if self.file.is_none() {
            self.file = std::fs::File::open(&self.path).ok();
        }
        let mut bytes = self.file.as_mut().and_then(|f| read_from(f, self.pos));
        // 넘긴 뒤의 새 파일은 읽던 자리보다 짧다 (새 파일이 0.1초 안에 상한만큼 자라지는 않는다)
        if bytes.is_none() && std::fs::metadata(&self.path).is_ok_and(|m| m.len() < self.pos) {
            self.file = std::fs::File::open(&self.path).ok();
            self.pos = 0;
            bytes = self.file.as_mut().and_then(|f| read_from(f, 0));
        }
        self.pos += bytes.as_ref().map_or(0, |b| b.len() as u64);
        bytes
    }
}

/// `pos`부터 파일 끝까지. 새 내용이 없으면 None.
fn read_from(file: &mut std::fs::File, pos: u64) -> Option<Vec<u8>> {
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

    /// 러너 로그에서 따라 읽은 줄은 러너가 쓴 시각을 지킨다. 데몬이 읽은 시각을 달면 서버가 꺼진 뒤
    /// 디스크 기록과 겹치는 곳을 맞추지 못해(GUI `unseenDiskTail`) 직전 실행의 줄이 콘솔에 한 번 더 붙는다
    #[tokio::test]
    async fn followed_runner_lines_keep_the_time_the_runner_wrote() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let run = run_dir(inst);
        std::fs::create_dir_all(&run).unwrap();
        let log = inst.join("session.log");
        std::fs::write(&log, "").unwrap();
        std::fs::write(run.join(TOKEN_FILE), "tok").unwrap();
        // 살아 있는 러너 대신 이 테스트 프로세스
        let info = RunnerInfo { runner_pid: std::process::id(), pid: std::process::id(), port: 1, log: log.clone(), started_at: saba_runner::now(), cwd: None, format: saba_runner::RUNNER_FORMAT };
        saba_runner::write_json(&runner_file(&run), &info).unwrap();
        let proc = ManagedProcess::attach_runner(inst, None).await.expect("attached");

        let written = 1_700_000_000; // 2023-11-14 22:13:20 UTC
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        std::io::Write::write_all(&mut f, format!("{}\n", saba_runner::format_line(written, saba_runner::Stream::Stdout, "old news")).as_bytes()).unwrap();
        assert!(wait_for(&proc, "old news").await);
        let line = proc.get_recent_console(100).await.into_iter().find(|l| l.content == "old news").unwrap();
        assert_eq!(line.timestamp, written);
    }

    /// 데몬이 콘솔에 넣는 줄(보낸 명령, 크래시 안내)은 메모리에만 있어, 서버가 꺼지거나 데몬이 다시 붙으면 사라졌다.
    /// 크래시 안내는 프로세스가 이미 목록에서 빠진 뒤에 쓰여 한 번도 보이지 않았다. 세션 로그에도 남아야 한다
    #[tokio::test]
    async fn lines_the_daemon_adds_to_the_console_stay_in_the_session_log() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let run = run_dir(inst);
        std::fs::create_dir_all(&run).unwrap();
        let log = inst.join("session.log");
        std::fs::write(&log, "").unwrap();
        std::fs::write(run.join(TOKEN_FILE), "tok").unwrap();
        let info = RunnerInfo { runner_pid: std::process::id(), pid: std::process::id(), port: 1, log: log.clone(), started_at: saba_runner::now(), cwd: None, format: saba_runner::RUNNER_FORMAT };
        saba_runner::write_json(&runner_file(&run), &info).unwrap();
        let proc = ManagedProcess::attach_runner(inst, None).await.expect("attached");

        // 받는 러너가 없어 전달은 실패하지만 보낸 줄은 콘솔에 남는다
        let _ = proc.send_command("say hi").await;
        proc.push_system_line("── Crash detected (signal 6). Restarting in 5s. ──".into(), LogLevel::Warn).await;
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        std::io::Write::write_all(&mut f, format!("{}\n", saba_runner::format_line(saba_runner::now(), saba_runner::Stream::Stdout, "server said hi")).as_bytes()).unwrap();
        assert!(wait_for(&proc, "server said hi").await);
        let seen = |lines: &[LogLine], source: LogSource, text: &str| lines.iter().filter(|l| l.source == source && l.content.contains(text)).count();
        let live = proc.get_recent_console(100).await;
        assert_eq!((seen(&live, LogSource::Input, "say hi"), seen(&live, LogSource::System, "Crash detected")), (1, 1), "shown once while running: {live:?}");

        // 데몬이 다시 붙거나 서버가 꺼진 뒤에는 세션 로그에서 읽는다
        drop(proc);
        let again = ManagedProcess::attach_runner(inst, None).await.expect("attached again");
        let restored = again.get_recent_console(100).await;
        assert_eq!((seen(&restored, LogSource::Input, "say hi"), seen(&restored, LogSource::System, "Crash detected")), (1, 1), "{restored:?}");
    }

    /// 한 번에 여러 줄과 끝나지 않은 줄이 들어와도 줄마다 한 번씩, 순서대로 콘솔에 붙는다
    #[tokio::test]
    async fn followed_runner_lines_split_a_chunk_and_keep_the_unfinished_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let run = run_dir(inst);
        std::fs::create_dir_all(&run).unwrap();
        let log = inst.join("session.log");
        std::fs::write(&log, "").unwrap();
        std::fs::write(run.join(TOKEN_FILE), "tok").unwrap();
        let info = RunnerInfo { runner_pid: std::process::id(), pid: std::process::id(), port: 1, log: log.clone(), started_at: saba_runner::now(), cwd: None, format: saba_runner::RUNNER_FORMAT };
        saba_runner::write_json(&runner_file(&run), &info).unwrap();
        let proc = ManagedProcess::attach_runner(inst, None).await.expect("attached");

        let line = |text: &str| saba_runner::format_line(1_700_000_000, saba_runner::Stream::Stdout, text);
        let last = line("chunk line 3");
        let (head, tail) = last.split_at(last.len() / 2);
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        std::io::Write::write_all(&mut f, format!("{}\n{}\n{}", line("chunk line 1"), line("chunk line 2"), head).as_bytes()).unwrap();
        assert!(wait_for(&proc, "chunk line 2").await);
        std::io::Write::write_all(&mut f, format!("{}\n", tail).as_bytes()).unwrap();
        assert!(wait_for(&proc, "chunk line 3").await);
        let got: Vec<String> = proc.get_recent_console(100).await.into_iter().map(|l| l.content).filter(|c| c.starts_with("chunk line")).collect();
        assert_eq!(got, ["chunk line 1", "chunk line 2", "chunk line 3"]);
    }

    /// 러너에 닿지 못한 명령은 성공으로 알리지 않는다 (정지 명령이 버려진 채 기다리다 강제 종료되지 않게)
    #[tokio::test]
    async fn a_command_the_runner_never_received_is_reported_as_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let run = run_dir(inst);
        std::fs::create_dir_all(&run).unwrap();
        let log = inst.join("session.log");
        std::fs::write(&log, "").unwrap();
        std::fs::write(run.join(TOKEN_FILE), "tok").unwrap();
        // 아무도 받지 않는 포트
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let info = RunnerInfo { runner_pid: std::process::id(), pid: std::process::id(), port: closed, log, started_at: saba_runner::now(), cwd: None, format: saba_runner::RUNNER_FORMAT };
        saba_runner::write_json(&runner_file(&run), &info).unwrap();
        let proc = ManagedProcess::attach_runner(inst, None).await.expect("attached");
        assert!(proc.send_command("stop").await.is_err());
    }

    /// 인터럽트(Ctrl+C)는 러너에 `토큰 interrupt` 연결로 부탁하고, 러너의 답으로 성공·실패를 정한다
    #[tokio::test]
    async fn an_interrupt_asks_the_runner_and_reports_its_answer() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let run = run_dir(inst);
        std::fs::create_dir_all(&run).unwrap();
        let log = inst.join("session.log");
        std::fs::write(&log, "").unwrap();
        std::fs::write(run.join(TOKEN_FILE), "tok").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let info = RunnerInfo { runner_pid: std::process::id(), pid: std::process::id(), port, log, started_at: saba_runner::now(), cwd: None, format: saba_runner::RUNNER_FORMAT };
        saba_runner::write_json(&runner_file(&run), &info).unwrap();
        let proc = ManagedProcess::attach_runner(inst, None).await.expect("attached");

        let runner = tokio::spawn(async move {
            let mut asked = Vec::new();
            for answer in ["ok", "error: cannot attach to the server console"] {
                let (conn, _) = listener.accept().await.unwrap();
                let mut conn = tokio::io::BufReader::new(conn);
                let mut first = String::new();
                conn.read_line(&mut first).await.unwrap();
                asked.push(first);
                conn.get_mut().write_all(format!("{}\n", answer).as_bytes()).await.unwrap();
            }
            asked
        });
        assert!(proc.interrupt().await.is_ok());
        let refused = proc.interrupt().await.unwrap_err().to_string();
        assert!(refused.contains("cannot attach"), "{}", refused);
        assert_eq!(runner.await.unwrap(), ["tok interrupt\n", "tok interrupt\n"]);
    }

    /// 러너가 없는(닫힌) 포트면 실패다: 정지가 인터럽트를 보낸 척하고 기다리지 않게
    #[tokio::test]
    async fn an_interrupt_the_runner_never_received_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let run = run_dir(inst);
        std::fs::create_dir_all(&run).unwrap();
        let log = inst.join("session.log");
        std::fs::write(&log, "").unwrap();
        std::fs::write(run.join(TOKEN_FILE), "tok").unwrap();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let info = RunnerInfo { runner_pid: std::process::id(), pid: std::process::id(), port: closed, log, started_at: saba_runner::now(), cwd: None, format: saba_runner::RUNNER_FORMAT };
        saba_runner::write_json(&runner_file(&run), &info).unwrap();
        let proc = ManagedProcess::attach_runner(inst, None).await.expect("attached");
        assert!(proc.interrupt().await.is_err());
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

        let started = ManagedProcess::spawn_with_runner(&fake, "server", &[], inst.to_str().unwrap(), Vec::new(), None, inst, inst, 3, RunnerLaunch::default()).await;
        assert!(started.is_err());
        let ids: Vec<u32> = std::fs::read_to_string(&pids).unwrap().split_whitespace().map(|p| p.parse().unwrap()).collect();
        tokio::time::sleep(Duration::from_millis(300)).await;
        for pid in ids {
            // 끝났지만 아직 거둬지지 않은 프로세스(좀비)는 끝난 것으로 본다
            let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
            assert!(zombie || !crate::supervisor::process::is_process_alive(pid), "process {pid} must be gone after the failed start");
        }
    }

    /// 서버만 띄우고 보고 없이 먼저 죽은 러너도 그 서버를 남기지 않는다
    #[tokio::test]
    async fn a_runner_that_dies_without_reporting_does_not_leave_its_server() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let pids = inst.join("pids");
        let fake = inst.join("fake-runner.sh");
        std::fs::write(&fake, format!("#!/bin/sh\nsleep 60 &\necho \"$!\" > '{}'\nexit 0\n", pids.display())).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let started = ManagedProcess::spawn_with_runner(&fake, "server", &[], inst.to_str().unwrap(), Vec::new(), None, inst, inst, 3, RunnerLaunch::default()).await;
        assert!(started.is_err());
        let server: u32 = std::fs::read_to_string(&pids).unwrap().trim().parse().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let zombie = std::fs::read_to_string(format!("/proc/{server}/stat")).is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
        assert!(zombie || !crate::supervisor::process::is_process_alive(server), "server {server} must be gone after the failed start");
    }

    /// 데몬이 떨어졌다 다시 붙어도 콘솔 기록과 stdin이 이어지고, 끝나면 종료 기록이 남는다
    #[tokio::test]
    async fn runner_keeps_console_and_stdin_across_reattach() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let inst = &tmp.path().join("inst");
        let state = &tmp.path().join("state");

        // 입력을 되풀이하는 줄은 콘솔에서 빠지므로(`take_echo`) 답에 머리를 붙이는 서버로 본다
        let args = ["-u".to_string(), "s/^/got /".to_string()];
        let proc = ManagedProcess::spawn_with_runner(&runner, "sed", &args, tmp.path().to_str().unwrap(), Vec::new(), None, inst, state, 3, RunnerLaunch::default())
            .await
            .unwrap();
        proc.send_command("before restart").await.unwrap();
        assert!(wait_for(&proc, "got before restart").await);
        let pid = proc.pid;
        drop(proc); // 데몬 재시작 흉내

        // 러너 기록은 데몬 전용 폴더에만 있다
        assert!(!inst.join("run").exists());
        let again = ManagedProcess::attach_runner(state, None).await.expect("runner still alive");
        assert_eq!(again.pid, pid);
        assert!(wait_for(&again, "got before restart").await, "history comes back from the session log");
        assert!(again.is_stdin_available());
        again.send_command("after restart").await.unwrap();
        assert!(wait_for(&again, "got after restart").await);

        crate::supervisor::process::force_kill_pid(pid).unwrap();
        for _ in 0..50 {
            if !again.is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!again.is_running());
        let exit: ExitInfo = read_json(&exit_file(&run_dir(state))).expect("exit.json");
        assert!(!exit.success);
        assert!(ManagedProcess::attach_runner(state, None).await.is_none());
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
            &runner, "sh", &["-c".into(), script], inst.to_str().unwrap(), Vec::new(), None, inst, inst, 3,
            RunnerLaunch { log_files: &["*.log".into()], console: ConsoleMode::Pipe, ..Default::default() },
        )
        .await
        .unwrap();
        assert!(wait_for(&proc, "from the log file").await);
        assert!(!proc.get_recent_console(100).await.iter().any(|l| l.content == "old line"), "only new lines");
        crate::supervisor::process::force_kill_pid(proc.pid).unwrap();
    }

    /// [launch] line_start: 서버가 줄바꿈 없이 이어 쓴 출력이 줄 머리마다 따로 보인다
    #[cfg(unix)]
    #[tokio::test]
    async fn declared_line_starts_split_the_console() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let script = "printf '[LOG] save OK[LOG] shutdown OK\\n'; sleep 5".to_string();
        let proc = ManagedProcess::spawn_with_runner(
            &runner, "sh", &["-c".into(), script], inst.to_str().unwrap(), Vec::new(), None, inst, inst, 3,
            RunnerLaunch { line_start: Some(r"\[LOG\]"), ..Default::default() },
        )
        .await
        .unwrap();
        assert!(wait_for(&proc, "[LOG] shutdown OK").await);
        assert!(wait_for(&proc, "[LOG] save OK").await);
        let _ = proc.force_kill().await;
    }

    /// [extensions.saba-core] progress_patterns: 숫자만 바뀌며 이어지는 진행 줄은 첫 줄과 마지막 줄만 보인다
    #[cfg(unix)]
    #[tokio::test]
    async fn declared_progress_lines_fold_in_the_console() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let inst = tmp.path();
        let script = "for p in 1 2 3 4 5; do echo \"Saving $p%\"; done; echo saved; sleep 5".to_string();
        let progress = vec![r"^Saving \d+%$".to_string()];
        let proc = ManagedProcess::spawn_with_runner(
            &runner, "sh", &["-c".into(), script], inst.to_str().unwrap(), Vec::new(), None, inst, inst, 3,
            RunnerLaunch { progress_patterns: &progress, ..Default::default() },
        )
        .await
        .unwrap();
        assert!(wait_for(&proc, "saved").await);
        let lines: Vec<String> = proc.get_console_since(0).await.into_iter().filter(|l| l.source != LogSource::System).map(|l| l.content).collect();
        assert_eq!(lines, ["Saving 1%", "Saving 5%", "saved"]);
        let _ = proc.force_kill().await;
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
            &runner, "python3", &["-c".into(), script], inst.to_str().unwrap(), env, None, inst, inst, 3,
            RunnerLaunch { log_files: &[], console: ConsoleMode::Pty, ..Default::default() },
        )
        .await
        .unwrap();
        assert!(wait_for(&proc, "early line").await, "output arrives while the server runs");
        proc.send_command("ping").await.unwrap();
        assert!(wait_for(&proc, "got ping").await);
        let _ = proc.force_kill().await;
    }

    /// 강제 종료는 런처 스크립트가 띄운 실제 서버까지 끝낸다 (HUP을 무시하는 자식도, 파이프·의사 콘솔 모두)
    #[cfg(unix)]
    #[tokio::test]
    async fn force_kill_ends_what_the_launcher_script_started() {
        let Some(runner) = runner_binary() else {
            eprintln!("SKIP: saba-runner binary not built (cargo build -p saba-runner)");
            return;
        };
        for console in [ConsoleMode::Pipe, ConsoleMode::Pty] {
            let tmp = tempfile::tempdir().unwrap();
            let inst = tmp.path();
            let script = "(trap '' HUP; exec sleep 600) & echo \"child $!\"; echo ready; wait".to_string();
            let proc = ManagedProcess::spawn_with_runner(
                &runner, "sh", &["-c".into(), script], inst.to_str().unwrap(), Vec::new(), None, inst, inst, 3,
                RunnerLaunch { console, ..Default::default() },
            )
            .await
            .unwrap();
            assert!(wait_for(&proc, "ready").await);
            let lines = proc.get_recent_console(100).await;
            let child: u32 = lines.iter().find_map(|l| l.content.trim().strip_prefix("child ")?.parse().ok()).expect("child pid");
            proc.force_kill().await.unwrap();
            // 거둬 주는 부모가 없는 환경에서는 죽은 프로세스가 좀비로 남는다
            let gone = || {
                let zombie = std::fs::read_to_string(format!("/proc/{child}/stat")).is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
                zombie || !crate::supervisor::process::is_process_alive(child)
            };
            for _ in 0..30 {
                if gone() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let left = !gone();
            if left {
                unsafe { nix::libc::kill(child as i32, nix::libc::SIGKILL) };
            }
            assert!(!left, "the server behind the launcher (pid {child}) outlived a force kill");
        }
    }

    #[test]
    fn old_session_logs_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["20260101-000000.log", "20260102-000000.log", "20260103-000000.log"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        prune_sessions(tmp.path(), 2, u64::MAX);
        let mut left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        left.sort();
        assert_eq!(left, ["20260102-000000.log", "20260103-000000.log"]);
        assert_eq!(latest_session_log(tmp.path()), Some(dir.join("20260103-000000.log")));
    }

    // 세션 하나가 수 GB까지 자라던 것을 러너가 조각으로 나눈다: 조각은 그 세션과 같이 지우고, 합친 크기도 상한을 지킨다
    #[test]
    fn session_logs_are_pruned_with_their_pieces_and_to_a_total_size() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        let names = || {
            let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            left.sort();
            left
        };
        for name in ["20260101-000000.log", "20260101-000000.log.1", "20260102-000000.log", "20260102-000000.log.1", "20260103-000000.log", "notes.txt"] {
            std::fs::write(dir.join(name), [b'x'; 100]).unwrap();
        }

        prune_sessions(tmp.path(), 2, u64::MAX);
        assert_eq!(names(), ["20260102-000000.log", "20260102-000000.log.1", "20260103-000000.log", "notes.txt"]);

        prune_sessions(tmp.path(), 2, 250);
        assert_eq!(names(), ["20260103-000000.log", "notes.txt"]);
    }

    // 회전이 없던 때의 큰 로그가 가장 최근 세션이어도 크기 때문에 지우지 않는다: 크래시 뒤 다시 켰을 때 그 로그가 남아 있어야 한다
    #[test]
    fn the_latest_session_log_survives_the_size_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sessions_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("20260101-000000.log"), [b'x'; 10]).unwrap();
        std::fs::write(dir.join("20260102-000000.log"), [b'x'; 500]).unwrap();
        prune_sessions(tmp.path(), 2, 250);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["20260102-000000.log"]);
    }

    // 러너가 세션 로그를 앞 조각으로 넘겨도 콘솔은 넘기기 전의 마지막 줄과 새 파일의 줄을 빠짐없이 차례로 받는다
    #[test]
    fn following_continues_across_a_rotated_session_log() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("session.log");
        let mut follow = Follow { path: path.clone(), file: None, pos: 0 };
        let mut seen = Vec::new();
        let mut read = |follow: &mut Follow| {
            while let Some(bytes) = follow.read_new() {
                seen.extend(bytes);
            }
        };

        let mut old = std::fs::File::create(&path).unwrap();
        old.write_all(b"one\ntwo\n").unwrap();
        read(&mut follow);
        old.write_all(b"three\n").unwrap();
        std::fs::rename(&path, saba_runner::session_log_piece(&path, 1)).unwrap();
        std::fs::write(&path, b"four\n").unwrap();
        read(&mut follow);

        assert_eq!(String::from_utf8(seen).unwrap(), "one\ntwo\nthree\nfour\n");
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
