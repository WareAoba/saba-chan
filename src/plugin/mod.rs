use anyhow::Result;
use serde_json::Value;
use tokio::process::Command;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader};
use crate::utils::apply_creation_flags;

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
    /// 전체 단계 목록 (첫 progress에서만 전송)
    pub steps: Option<Vec<String>>,
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

/// 기본 플러그인 타임아웃 (초)
pub const DEFAULT_PLUGIN_TIMEOUT_SECS: u64 = 120;

/// 설치·프로비저닝 등 장시간 작업용 타임아웃 (초) — SteamCMD/다운로드 허용
pub const LONG_PLUGIN_TIMEOUT_SECS: u64 = 1800;

/// `PROGRESS:{json}` 이벤트 콜백. 여러 훅/호출에서 공유할 수 있도록 `Arc`로 둔다.
pub type ProgressCallback = std::sync::Arc<dyn Fn(ExtensionProgress) + Send + Sync>;

/// 모듈/익스텐션 프로세스에 주입할 앱 언어 (`SABA_LANG`).
///
/// 데몬이 GUI 없이 단독 기동되면 프로세스 env에 `SABA_LANG`이 없어
/// 모듈 i18n이 전부 영어로 폴백된다. 데몬 부팅 시(그리고 GUI 설정 저장 시)
/// settings.json의 language로 이 전역을 갱신해, 이후 스폰되는 모든
/// 플러그인 프로세스가 앱 언어를 물려받게 한다.
static PLUGIN_LANG: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// 플러그인 프로세스에 주입할 데몬 IPC 기본 URL (`DAEMON_API_URL`).
/// 모듈이 `/api/instance/:id/rcon` 등으로 콜백할 때 실제 바인드된 포트를 쓰도록 한다.
static DAEMON_API_URL: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// 플러그인 프로세스에 주입할 언어를 설정한다 (`None`이면 주입 안 함 → env 상속).
pub fn set_plugin_lang(lang: Option<String>) {
    let normalized = lang.filter(|l| !l.trim().is_empty());
    if let Ok(mut slot) = PLUGIN_LANG.write() {
        *slot = normalized;
    }
}

/// 현재 설정된 플러그인 언어를 반환한다.
pub fn plugin_lang() -> Option<String> {
    PLUGIN_LANG.read().ok().and_then(|s| s.clone())
}

/// IPC 서버가 바인드된 뒤 호출한다 (예: `http://127.0.0.1:57474`).
pub fn set_daemon_api_url(url: &str) {
    if let Ok(mut slot) = DAEMON_API_URL.write() {
        *slot = Some(url.trim_end_matches('/').to_string());
    }
}

/// 플러그인에 전달할 데몬 IPC 기본 URL.
pub fn daemon_api_url() -> Option<String> {
    DAEMON_API_URL.read().ok().and_then(|s| s.clone())
}

/// Python 모듈 함수를 실행하고 stdout의 JSON 결과를 반환한다 (기본 타임아웃).
pub async fn run_plugin(module_path: &str, function: &str, config: Value) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, DEFAULT_PLUGIN_TIMEOUT_SECS).await
}

/// `run_plugin`과 같지만 타임아웃(초)을 지정한다.
pub async fn run_plugin_with_timeout(module_path: &str, function: &str, config: Value, timeout_secs: u64) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, timeout_secs).await
}

/// stderr의 `PROGRESS:{json}` 줄마다 콜백을 호출하는 장시간 실행용 러너.
pub async fn run_plugin_with_progress(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: ProgressCallback,
    timeout_secs: u64,
) -> Result<Value> {
    run_plugin_inner(module_path, function, config, Some(on_progress), timeout_secs).await
}

async fn run_plugin_inner(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: Option<ProgressCallback>,
    timeout_secs: u64,
) -> Result<Value> {
    tracing::debug!("Executing plugin: {} -> {}", module_path, function);

    // 존재하지 않는 스크립트는 Python 부트스트랩(다운로드)까지 가지 않고 즉시 실패한다.
    if !std::path::Path::new(module_path).is_file() {
        return Err(anyhow::anyhow!("Plugin script not found: {}", module_path));
    }

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
        .env("PYTHONUNBUFFERED", "1");

    // 앱 언어 주입 — 모듈 i18n(SABA_LANG 기반)이 데몬 기동 방식과 무관하게
    // settings.json의 언어를 따르도록 한다. 미설정이면 데몬 env 상속.
    if let Some(lang) = plugin_lang() {
        cmd.env("SABA_LANG", lang);
    }
    if let Some(url) = daemon_api_url() {
        cmd.env("DAEMON_API_URL", url);
    }

    // 활성화된 익스텐션 목록을 Python에 전달 (비활성 익스텐션 import 방지용).
    // 상태 파일은 데몬 공통 데이터 디렉토리(SABA_DATA_DIR 존중)에서 읽는다.
    let state_path = saba_chan_updater_lib::constants::resolve_extensions_state_path();
    if let Ok(content) = std::fs::read_to_string(&state_path) {
        if let Ok(enabled_list) = serde_json::from_str::<Vec<String>>(&content) {
            cmd.env("SABA_ENABLED_EXTENSIONS", enabled_list.join(","));
        }
    }

    let extensions_dir = resolve_extensions_dir();
    if extensions_dir.is_dir() {

        let mut pypath = extensions_dir.parent()
            .unwrap_or(&extensions_dir)
            .to_string_lossy()
            .into_owned();

        // PYTHONPATH 경로 리스트 구분자 (Windows: ';', Unix: ':')
        #[cfg(windows)]
        const PYPATH_SEP: char = ';';
        #[cfg(not(windows))]
        const PYPATH_SEP: char = ':';

        // 공용 모듈 유틸리티 경로 (daemon_rcon.py, i18n.py 등)
        let modules_lib_dir = resolve_modules_dir();
        if modules_lib_dir.is_dir() {
            pypath = format!("{}{}{}", pypath, PYPATH_SEP, modules_lib_dir.display());
        }

        if let Ok(existing) = std::env::var("PYTHONPATH") {
            pypath = format!("{}{}{}", pypath, PYPATH_SEP, existing);
        }
        cmd.env("PYTHONPATH", &pypath);
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

            match serde_json::from_str::<Value>(&stdout_str) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 없는 스크립트는 Python 부트스트랩(네트워크) 없이 즉시 실패해야 한다.
    #[tokio::test]
    async fn run_plugin_missing_script_fails_without_bootstrap() {
        let err = run_plugin("nonexistent/module.py", "test_function", json!({"test": "data"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"), "{}", err);
    }

    #[test]
    fn daemon_api_url_is_trimmed() {
        set_daemon_api_url("http://127.0.0.1:1234/");
        assert_eq!(daemon_api_url().as_deref(), Some("http://127.0.0.1:1234"));
    }
}
