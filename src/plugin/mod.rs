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
/// - `SABA_LANG`: 지금 GUI 언어. 언어를 바꿔도 데몬은 재시작하지 않으므로 매번 읽는다.
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
        .map(str::to_string);
    if let Some(lang) = lang {
        env.push(("SABA_LANG", lang));
    }
    env
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
const SHARED_MODULES: [(&str, &str); 2] = [
    ("i18n.py", include_str!("../../modules/i18n.py")),
    ("daemon_rcon.py", include_str!("../../modules/daemon_rcon.py")),
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

async fn run_plugin_inner(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<Box<dyn Fn(ExtensionProgress) + Send>>,
    timeout_secs: u64,
) -> Result<Value> {
    tracing::debug!("Executing plugin: {} -> {}", module_path, function);

    let config_json = serde_json::to_string(&config)?;
    let python_exe = crate::python_env::get_python_path().await?;
    tracing::debug!("Using Python: {}", python_exe.display());

    let mut cmd = Command::new(&python_exe);
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

    apply_creation_flags(&mut cmd);

    let mut child = cmd.spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config_json.as_bytes()).await?;
        stdin.shutdown().await?;
    }

    // Stream stderr line-by-line to capture PROGRESS: events in real-time
    let stderr_pipe = child.stderr.take();
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
    let stdout_pipe = child.stdout.take();
    let stdout_handle = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe {
            let mut reader = tokio::io::BufReader::new(pipe);
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
        }
        String::from_utf8_lossy(&buf).to_string()
    });

    let status = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait(),
    ).await;

    match status {
        Ok(Ok(exit_status)) => {
            let stderr_str = stderr_handle.await.unwrap_or_default();
            let stdout_str = stdout_handle.await.unwrap_or_default();

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
            // 타임아웃 — 프로세스 강제 종료
            tracing::warn!(
                "Plugin timed out after {}s: {} -> {} — killing process",
                timeout_secs, module_path, function
            );
            let _ = child.kill().await;
            Err(anyhow::anyhow!("Plugin timed out after {}s", timeout_secs))
        }
    }
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
