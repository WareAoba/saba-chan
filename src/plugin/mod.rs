use anyhow::Result;
use serde_json::Value;
use tokio::process::Command;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader};
use crate::utils::apply_creation_flags;

/// 데몬 IPC 포트. 모듈이 데몬 API를 역호출할 때 쓴다(main이 기동 시 한 번 설정).
static IPC_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

pub fn set_ipc_port(port: u16) {
    let _ = IPC_PORT.set(port);
}

/// Python 실행마다 넣는 데몬 연결 정보.
/// - `SABA_IPC_PORT`, `SABA_IPC_URL`: 데몬 API 주소
/// - `SABA_TOKEN_PATH`: `X-Saba-Token` 값이 든 파일 (토큰 자체는 env에 넣지 않는다)
/// - `SABA_LANG`: 지금 GUI 언어(고르기 전에는 OS 언어). 언어를 바꿔도 데몬은 재시작하지 않으므로 매번 읽는다.
fn daemon_env() -> Vec<(&'static str, String)> {
    let port = *IPC_PORT.get().unwrap_or(&saba_chan_updater_lib::constants::DEFAULT_IPC_PORT);
    let mut env = vec![
        ("SABA_IPC_PORT", port.to_string()),
        ("SABA_IPC_URL", format!("http://127.0.0.1:{}", port)),
        (
            "SABA_TOKEN_PATH",
            saba_chan_updater_lib::constants::token_file_path().to_string_lossy().into_owned(),
        ),
    ];
    let lang = saba_chan_updater_lib::config_file::ConfigFile::default_location()
        .section("gui")
        .get("language")
        .and_then(|v| v.as_str())
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        // 언어를 고르기 전에는 GUI처럼 OS 언어를 따른다
        .unwrap_or_else(crate::chat::i18n::os_language);
    env.push(("SABA_LANG", lang));
    env
}

/// 데몬 환경에서 물려받지 않는 변수 이름 (M12 실행 시 제한): 이름에 이 말이 든 것.
/// 모듈·익스텐션 코드는 데몬이 넣어 주는 값(`daemon_env`)과 평범한 시스템 변수만 본다
const SECRET_ENV_MARKERS: &[&str] = &["TOKEN", "SECRET", "PASSWORD", "PASSWD", "API_KEY", "APIKEY", "PRIVATE_KEY", "CREDENTIAL", "ACCESS_KEY"];

fn is_secret_env(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper != "SABA_TOKEN_PATH" && SECRET_ENV_MARKERS.iter().any(|m| upper.contains(m))
}

/// 시간 초과한 Python과 그것이 띄운 프로세스를 함께 끝낸다.
/// Unix는 자기 프로세스 그룹으로 띄우므로 그룹째, Windows는 `taskkill /T`
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    unsafe {
        nix::libc::kill(-(pid as i32), nix::libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/T", "/F", "/PID", &pid.to_string()]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let _ = cmd.output();
    }
}

/// Progress info emitted by Python extensions via stderr "PROGRESS:{json}" lines
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExtensionProgress {
    pub percent: Option<u8>,
    pub message: Option<String>,
    /// 현재 단계 인덱스 (0-based)
    pub step: Option<u8>,
    /// 전체 단계 수
    pub total: Option<u8>,
    /// 현재 단계 식별자 (예: "checking_engine")
    pub label: Option<String>,
}

/// extensions/ 디렉토리 경로를 해석합니다.
///
/// 고정 경로: `%APPDATA%/saba-chan/extensions` (Windows)
///            `~/.config/saba-chan/extensions` (Linux/macOS)
///
/// `SABA_EXTENSIONS_DIR` 환경 변수가 설정되면 해당 경로를 우선 사용합니다 (테스트/개발용).
///
/// 디렉토리가 존재하지 않으면 생성을 시도합니다.
pub fn resolve_extensions_dir() -> std::path::PathBuf {
    let candidate = saba_chan_updater_lib::constants::resolve_extensions_dir();
    if !candidate.exists() {
        let _ = std::fs::create_dir_all(&candidate);
    }
    candidate
}

/// modules/ 디렉토리 경로를 해석합니다.
///
/// 고정 경로: `%APPDATA%/saba-chan/modules` (Windows)
///            `~/.config/saba-chan/modules` (Linux/macOS)
///
/// `SABA_MODULES_PATH` 환경 변수가 설정되면 해당 경로를 우선 사용합니다 (테스트/개발용).
///
/// 디렉토리가 존재하지 않으면 생성을 시도합니다.
pub fn resolve_modules_dir() -> std::path::PathBuf {
    let candidate = saba_chan_updater_lib::constants::resolve_modules_dir();
    if !candidate.exists() {
        let _ = std::fs::create_dir_all(&candidate);
    }
    candidate
}

/// import 가드(`sitecustomize.py`)를 둔 폴더. 데이터 폴더에 한 번 쓰고, 내용이 바뀌었을 때만 다시 쓴다.
fn import_guard_dir() -> Option<std::path::PathBuf> {
    const GUARD: &str = include_str!("sitecustomize.py");
    let dir = saba_chan_updater_lib::constants::resolve_data_dir().join("python-guard");
    let file = dir.join("sitecustomize.py");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(GUARD) {
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, GUARD)) {
            tracing::warn!("Cannot write the extension import guard to {}: {}", file.display(), e);
            return None;
        }
    }
    Some(dir)
}

/// 데몬에 넣어 둔 공용 Python 유틸리티. 모듈 폴더에 이것과 다른 판이 있으면 덮어쓴다.
/// 설치·업데이트 경로와 상관없이(개발 빌드를 복사만 해도) 모듈이 데몬과 같은 판을 쓴다 (C7)
const SHARED_MODULES: [(&str, &str); 3] = [
    ("i18n.py", include_str!("../../modules/i18n.py")),
    ("daemon_rcon.py", include_str!("../../modules/daemon_rcon.py")),
    // 모듈 코드 검사기 (M12): 적합성 키트가 `--shared-libs`에서 찾는다
    ("saba_audit.py", crate::module_audit::SCRIPT),
];

/// 공용 유틸리티를 `dir`에 맞춰 둔다. 내용이 같으면 쓰지 않는다
pub fn ensure_shared_modules(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, content) in SHARED_MODULES {
        let file = dir.join(name);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(content) {
            std::fs::write(&file, content)?;
            tracing::info!("Shared module refreshed: {}", file.display());
        }
    }
    Ok(())
}

/// 기본 플러그인 타임아웃 (초)
pub const DEFAULT_PLUGIN_TIMEOUT_SECS: u64 = 120;

/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
/// Called by Supervisor for module lifecycle management
/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
pub async fn run_plugin(module_path: &str, function: &str, config: Value) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, DEFAULT_PLUGIN_TIMEOUT_SECS).await
}

/// 데몬 락 안에서 만들고 락을 놓은 뒤 실행하는 모듈 호출 (C12).
/// Python은 몇 분까지 걸릴 수 있어서, 락을 쥔 채 기다리면 모니터와 다른 요청이 모두 멈춘다.
pub struct PluginCall {
    pub entry: String,
    pub function: String,
    pub config: Value,
}

impl PluginCall {
    pub fn new(entry: impl Into<String>, function: &str, config: Value) -> Self {
        Self { entry: entry.into(), function: function.to_string(), config }
    }

    pub async fn run(self) -> Result<Value> {
        run_plugin(&self.entry, &self.function, self.config).await
    }
}

/// Like `run_plugin` but with a custom timeout (seconds).
pub async fn run_plugin_with_timeout(module_path: &str, function: &str, config: Value, timeout_secs: u64) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, timeout_secs).await
}

/// Like `run_plugin_with_progress` but with a custom timeout.
/// SteamCMD 다운로드 등 장시간 작업에 사용합니다.
pub async fn run_plugin_with_progress_and_timeout<F>(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: F,
    timeout_secs: u64,
) -> Result<Value>
where
    F: Fn(ExtensionProgress) + Send + 'static,
{
    run_plugin_inner(module_path, function, config, Some(Box::new(on_progress)), timeout_secs).await
}

type Writer = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;
type Reader = Box<dyn tokio::io::AsyncRead + Unpin + Send>;

/// 띄운 Python: 평소대로 띄운 것과 AppContainer 안에서 띄운 것 (실행마다 하나라 크기는 상관없다)
#[allow(clippy::large_enum_variant)]
enum Process {
    Plain(tokio::process::Child),
    #[cfg(windows)]
    Contained(crate::sandbox::appcontainer::Contained),
}

impl Process {
    fn pipes(&mut self) -> (Option<Writer>, Option<Reader>, Option<Reader>) {
        match self {
            Process::Plain(child) => (
                child.stdin.take().map(|p| Box::new(p) as Writer),
                child.stdout.take().map(|p| Box::new(p) as Reader),
                child.stderr.take().map(|p| Box::new(p) as Reader),
            ),
            #[cfg(windows)]
            Process::Contained(c) => (
                c.stdin.take().map(|f| Box::new(tokio::fs::File::from_std(f)) as Writer),
                c.stdout.take().map(|f| Box::new(tokio::fs::File::from_std(f)) as Reader),
                c.stderr.take().map(|f| Box::new(tokio::fs::File::from_std(f)) as Reader),
            ),
        }
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self {
            Process::Plain(child) => child.wait().await,
            #[cfg(windows)]
            Process::Contained(c) => {
                use std::os::windows::process::ExitStatusExt;
                let handle = c.wait_handle();
                let code = tokio::task::spawn_blocking(move || crate::sandbox::appcontainer::wait(handle))
                    .await
                    .map_err(std::io::Error::other)??;
                Ok(std::process::ExitStatus::from_raw(code))
            }
        }
    }

    /// 그것이 띄운 프로세스까지 끝낸다. `pid`는 띄울 때 받아 둔 것 (이미 거둔 뒤에는 `child.id()`가 없다)
    async fn kill(&mut self, pid: Option<u32>) {
        match self {
            Process::Plain(child) => {
                if let Some(pid) = pid.or(child.id()) {
                    kill_tree(pid);
                }
                let _ = child.kill().await;
            }
            #[cfg(windows)]
            Process::Contained(c) => {
                c.kill();
                // 핸들을 닫기 전에 끝난 것을 확인한다
                let handle = c.wait_handle();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::task::spawn_blocking(move || crate::sandbox::appcontainer::wait(handle)),
                )
                .await;
            }
        }
    }
}

async fn run_plugin_inner(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout_secs: u64,
) -> Result<Value> {
    tracing::debug!("Executing plugin: {} -> {}", module_path, function);

    // 확인되지 않은 모듈·익스텐션: 실행 전에 스크립트를 백신으로 본다 (Windows)
    if let Err(blocked) = crate::sandbox::guard_entry(module_path).await {
        tracing::warn!("Plugin {} blocked: {:?}", module_path, blocked);
        return Ok(blocked.to_json(crate::sandbox::message(&blocked)));
    }

    let config_json = serde_json::to_string(&config)?;
    let python_exe = crate::python_env::get_python_path().await?;
    tracing::debug!("Using Python: {}", python_exe.display());

    let mut cmd = Command::new(&python_exe);
    // 실행 시 제한 (M12): 비밀값처럼 보이는 변수는 물려주지 않고, 작업 폴더는 모듈·익스텐션 폴더로 고정
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(is_secret_env) {
            cmd.env_remove(&name);
        }
    }
    if let Some(dir) = std::path::Path::new(module_path).parent().filter(|d| d.is_dir()) {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    // 바이트코드 캐시는 데이터 폴더에만: 모듈 폴더의 __pycache__에 넣어 둔 .pyc가 소스 대신 실행되지 않게
    cmd.env("PYTHONPYCACHEPREFIX", saba_chan_updater_lib::constants::resolve_data_dir().join("pycache"));
    cmd.arg(module_path)
        .arg(function)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .envs(daemon_env());

    // PYTHONPATH 경로 리스트 구분자 (Windows: ';', Unix: ':')
    #[cfg(windows)]
    const PYPATH_SEP: char = ';';
    #[cfg(not(windows))]
    const PYPATH_SEP: char = ':';
    let mut paths: Vec<String> = Vec::new();
    // 활성화된 익스텐션 목록을 Python에 전달 (비활성 익스텐션 import 방지용)
    let extensions_dir = resolve_extensions_dir();
    if extensions_dir.is_dir() {
        let enabled = saba_chan_updater_lib::config_file::ConfigFile::default_location()
            .section("extensions")
            .get("enabled")
            .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok());
        // 목록이 없으면 활성 익스텐션도 없다 (ExtensionManager와 같은 규칙)
        cmd.env("SABA_ENABLED_EXTENSIONS", enabled.unwrap_or_default().join(","));

        let mut pypath = extensions_dir.parent()
            .unwrap_or(&extensions_dir)
            .to_string_lossy()
            .into_owned();
        // 활성 익스텐션만 import되게 하는 가드 (M6, sitecustomize.py)
        if let Some(guard) = import_guard_dir() {
            pypath = format!("{}{}{}", guard.display(), PYPATH_SEP, pypath);
        }
        paths.push(pypath);
    }
    // 공용 모듈 유틸리티 경로 (daemon_rcon.py, i18n.py 등). 익스텐션 폴더가 없어도 필요하다
    static SHARED: std::sync::Once = std::sync::Once::new();
    let modules_lib_dir = resolve_modules_dir();
    SHARED.call_once(|| {
        if let Err(e) = ensure_shared_modules(&modules_lib_dir) {
            tracing::warn!("Cannot write shared Python modules to {}: {}", modules_lib_dir.display(), e);
        }
    });
    if modules_lib_dir.is_dir() {
        paths.push(modules_lib_dir.display().to_string());
    }
    if !paths.is_empty() {
        if let Ok(existing) = std::env::var("PYTHONPATH") {
            paths.push(existing);
        }
        cmd.env("PYTHONPATH", paths.join(&PYPATH_SEP.to_string()));
    }

    // 데몬이 찾은 Java 런타임 (`[{java, home, version, major}]`, JAVA_HOME·PATH·설치 위치 순). 모듈은 필요한 주 버전을 여기서 고른다
    if let Ok(runtimes) = serde_json::to_string(&crate::sandbox::java_runtimes()) {
        cmd.env("SABA_JAVA_RUNTIMES", runtimes);
    }

    // 확인되지 않은 구성요소: AppContainer 안에서 돌린다 (Windows). 토큰 파일 경로는 주지 않는다
    let isolation = crate::sandbox::isolation_for(std::path::Path::new(module_path), &config, &python_exe);
    if let Some(iso) = &isolation {
        cmd.env("PYTHONPYCACHEPREFIX", &iso.pycache)
            .env("TEMP", &iso.temp)
            .env("TMP", &iso.temp)
            .env_remove("SABA_TOKEN_PATH");
        if let Some(data) = &iso.data {
            cmd.env("SABA_EXTENSION_DATA_DIR", data);
        }
    }

    apply_creation_flags(&mut cmd);

    let mut child = match &isolation {
        None => Process::Plain(cmd.spawn()?),
        Some(iso) => match crate::sandbox::spawn_isolated(cmd.as_std(), iso).await {
            Ok(crate::sandbox::Spawned::Unisolated) => Process::Plain(cmd.spawn()?),
            #[cfg(windows)]
            Ok(crate::sandbox::Spawned::Contained(contained)) => Process::Contained(contained),
            Err(blocked) => {
                let mut value = blocked.to_json(crate::sandbox::message(&blocked));
                value["component"] = Value::String(iso.key.clone());
                return Ok(value);
            }
        },
    };
    let status = drive(&mut child, config_json.as_bytes(), on_progress, std::time::Duration::from_secs(timeout_secs)).await;

    match status {
        Ok(Ok((exit_status, stdout_str, stderr_str))) => {
            if !exit_status.success() {
                tracing::error!("Plugin failed (exit {:?}):\n  stderr: {}\n  stdout: {}", exit_status.code(), stderr_str, stdout_str.trim());
                return Err(anyhow::anyhow!("Plugin execution failed (exit {:?}):\nstderr: {}\nstdout: {}", exit_status.code(), stderr_str, stdout_str.trim()));
            }

            match parse_plugin_output(&stdout_str) {
                Ok(result) => {
                    tracing::debug!("Plugin result (raw): {}", stdout_str.trim());
                    Ok(result)
                }
                Err(e) => {
                    tracing::error!("Failed to parse plugin JSON: {} | stdout: {}", e, stdout_str);
                    Err(anyhow::anyhow!("Invalid JSON from plugin: {}\nOutput: {}", e, stdout_str))
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!("Plugin process error: {}", e);
            Err(anyhow::anyhow!("Plugin process error: {}", e))
        }
        Err(_) => {
            // 타임아웃 — drive가 프로세스 트리를 이미 끝냈다
            tracing::warn!(
                "Plugin timed out after {}s: {} -> {} — killed process",
                timeout_secs, module_path, function
            );
            Err(anyhow::anyhow!("Plugin timed out after {}s", timeout_secs))
        }
    }
}

type Collected = std::io::Result<(std::process::ExitStatus, String, String)>;

/// 입력 쓰기부터 종료 대기, stdout·stderr 수집까지 한 기한 안에 한다.
/// 기한을 넘기거나 입출력이 실패하면 프로세스 트리를 끝내고 읽기 작업을 멈춘다.
/// (손자 프로세스가 파이프를 물려받아 쥐고 있으면 부모가 끝나도 출력이 닫히지 않는다)
async fn drive(
    child: &mut Process,
    input: &[u8],
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout: std::time::Duration,
) -> std::result::Result<Collected, tokio::time::error::Elapsed> {
    let pid = match child {
        Process::Plain(c) => c.id(),
        #[cfg(windows)]
        Process::Contained(_) => None,
    };
    let (stdin_pipe, stdout_pipe, stderr_pipe) = child.pipes();

    // Stream stderr line-by-line to capture PROGRESS: events in real-time
    let stderr_handle = tokio::spawn(async move {
        let mut log_lines = Vec::new();
        if let Some(pipe) = stderr_pipe {
            let mut reader = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(json_str) = line.strip_prefix("PROGRESS:") {
                    if let Ok(prog) = serde_json::from_str::<ExtensionProgress>(json_str) {
                        if let Some(ref cb) = on_progress {
                            cb(prog);
                        }
                    }
                } else {
                    tracing::debug!("Plugin stderr: {}", line);
                }
                log_lines.push(line);
            }
        }
        log_lines.join("\n")
    });

    // Read all of stdout (final JSON result)
    let stdout_handle = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe {
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
        }
        String::from_utf8_lossy(&buf).to_string()
    });

    let readers = [stderr_handle.abort_handle(), stdout_handle.abort_handle()];

    let work = async {
        if let Some(mut stdin) = stdin_pipe {
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
        }
        let exit_status = child.wait().await?;
        let stderr_str = stderr_handle.await.unwrap_or_default();
        let stdout_str = stdout_handle.await.unwrap_or_default();
        Ok((exit_status, stdout_str, stderr_str))
    };
    let result = tokio::time::timeout(timeout, work).await;
    if !matches!(result, Ok(Ok(_))) {
        child.kill(pid).await;
        for reader in readers {
            reader.abort();
        }
    }
    result
}

// detect_python_command()는 python_env::get_python_path()로 대체됨.
// 시스템 Python 탐지는 python_env 모듈에서 venv 부트스트랩 시 수행합니다.

/// 결과는 stdout 전체 또는 마지막 줄의 JSON (앞에 찍힌 print는 무시)
fn parse_plugin_output(stdout: &str) -> serde_json::Result<Value> {
    serde_json::from_str::<Value>(stdout).or_else(|e| {
        stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l.trim()))
            .unwrap_or(Err(e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_looking_variables_are_not_passed_to_python() {
        for name in ["DISCORD_TOKEN", "openai_api_key", "AWS_SECRET_ACCESS_KEY", "DB_PASSWORD", "GH_APIKEY", "GOOGLE_CREDENTIALS"] {
            assert!(is_secret_env(name), "{name}");
        }
        for name in ["SABA_TOKEN_PATH", "PATH", "APPDATA", "JAVA_HOME", "SYSTEMROOT", "KEYBOARD_LAYOUT"] {
            assert!(!is_secret_env(name), "{name}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_tree_is_killed_with_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child.pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!("sleep 30 & echo $! > {}; wait", pidfile.display())).process_group(0);
        let mut child = cmd.spawn().unwrap();
        let mut grandchild = None;
        for _ in 0..50 {
            if let Some(pid) = std::fs::read_to_string(&pidfile).ok().and_then(|s| s.trim().parse::<u32>().ok()) {
                grandchild = Some(pid);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let grandchild = grandchild.expect("child started");
        kill_tree(child.id().unwrap());
        let _ = child.wait().await;
        for _ in 0..50 {
            // 거둘 부모가 없으면 좀비로 남으니 상태도 본다
            let zombie = std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
                .is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
            if zombie || !crate::supervisor::process::is_process_alive(grandchild) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild} survived");
    }

    /// 부모가 끝나도 자식이 출력 파이프를 쥐고 있으면, 출력 수집까지 기한 안에 끊고 자식도 끝낸다
    #[cfg(unix)]
    #[tokio::test]
    async fn the_deadline_covers_output_collection_after_the_process_exits() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child.pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            // 입력을 다 읽고 끝나야 한다: 먼저 끝나면 입력 쓰기가 끊긴 파이프로 실패해 기한까지 가지 않는다
            .arg(format!("cat > /dev/null; sleep 30 & echo $! > {}; echo '{{}}'", pidfile.display()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        let mut child = Process::Plain(cmd.spawn().unwrap());
        let started = std::time::Instant::now();
        let result = drive(&mut child, b"{}", None, std::time::Duration::from_secs(1)).await;
        assert!(result.is_err(), "timed out instead of waiting for the held pipe");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let grandchild: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        for _ in 0..50 {
            let zombie = std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
                .is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')));
            if zombie || !crate::supervisor::process::is_process_alive(grandchild) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild} survived");
    }

    #[test]
    fn shared_modules_are_written_and_refreshed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules");
        ensure_shared_modules(&dir).unwrap();
        std::fs::write(dir.join("i18n.py"), "old").unwrap();
        ensure_shared_modules(&dir).unwrap();
        for (name, content) in SHARED_MODULES {
            assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), content);
        }
    }

    /// 가드는 활성 목록에 없는 `extensions.<이름>` import만 막는다 (`-`와 `_`는 같다)
    #[test]
    fn import_guard_blocks_only_disabled_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let guard = tmp.path().join("guard");
        std::fs::create_dir_all(&guard).unwrap();
        std::fs::write(guard.join("sitecustomize.py"), include_str!("sitecustomize.py")).unwrap();
        for name in ["steamcmd", "ue4_ini"] {
            let dir = tmp.path().join("root").join("extensions").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("__init__.py"), "").unwrap();
        }
        let sep = if cfg!(windows) { ";" } else { ":" };
        let pypath = format!("{}{}{}", guard.display(), sep, tmp.path().join("root").display());
        let probe = "import importlib\nfor m in ('steamcmd', 'ue4_ini'):\n    try:\n        importlib.import_module('extensions.' + m); print(m, 'ok')\n    except ImportError:\n        print(m, 'blocked')";
        let Ok(out) = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .args(["-c", probe])
            .env("PYTHONPATH", pypath)
            .env("SABA_ENABLED_EXTENSIONS", "ue4-ini")
            .output()
        else {
            eprintln!("SKIP: python not found");
            return;
        };
        assert_eq!(String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n").trim(), "steamcmd blocked\nue4_ini ok");
    }

    #[test]
    fn plugin_result_is_whole_stdout_or_last_line() {
        assert_eq!(parse_plugin_output("{\n  \"ok\": 1\n}\n").unwrap(), json!({"ok": 1}));
        assert_eq!(parse_plugin_output("debug print\n{\"ok\": 2}\n\n").unwrap(), json!({"ok": 2}));
        assert!(parse_plugin_output("{\"ok\": 3}\nnot json\n").is_err());
    }

    #[tokio::test]
    async fn test_run_plugin_with_invalid_path() {
        // venv가 없는 환경에서는 get_python_path부터 실패할 수 있음
        let result = run_plugin(
            "nonexistent/module.py",
            "test_function",
            json!({"test": "data"})
        ).await;

        assert!(result.is_err());
    }
}
