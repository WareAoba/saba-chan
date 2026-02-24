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
/// 우선순위:
///   1. `SABA_EXTENSIONS_DIR` 환경 변수 (절대 경로 오버라이드)
///   2. `%APPDATA%\saba-chan\extensions\` (Windows 프로덕션 설치 경로)
///   3. exe 상위 디렉토리의 `extensions/` (bin/../extensions/ — 포터블 앱 루트)
///   4. exe 옆 `extensions/`
///   5. `./extensions/` (CWD 폴백 — 개발 환경)
///
/// 디렉토리가 존재하지 않으면 생성을 시도합니다 (프로덕션 최초 실행 대응).
fn resolve_extensions_dir() -> std::path::PathBuf {
    // 1) 환경 변수 오버라이드
    if let Ok(dir) = std::env::var("SABA_EXTENSIONS_DIR") {
        let p = std::path::PathBuf::from(&dir);
        // 존재하지 않으면 생성 시도
        if !p.exists() {
            let _ = std::fs::create_dir_all(&p);
        }
        return p;
    }

    // 2) %APPDATA%\saba-chan\extensions (Windows 프로덕션)
    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        let candidate = std::path::PathBuf::from(&appdata)
            .join("saba-chan")
            .join("extensions");
        // 개발 환경(CWD에 extensions/가 있음)이 아닐 때만 사용
        let cwd_ext = std::path::PathBuf::from("./extensions");
        if !cwd_ext.is_dir() {
            if !candidate.exists() {
                let _ = std::fs::create_dir_all(&candidate);
            }
            return candidate;
        }
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    if let Some(ref dir) = exe_dir {
        // 3) exe의 상위 디렉토리 (bin/../extensions/ — 포터블 앱 루트)
        if let Some(parent) = dir.parent() {
            let candidate = parent.join("extensions");
            if candidate.is_dir() {
                return candidate;
            }
        }
        // 4) exe 옆 (exe_dir/extensions/)
        let beside = dir.join("extensions");
        if beside.is_dir() {
            return beside;
        }
    }

    // 5) CWD 폴백 (개발 환경: 프로젝트 루트의 extensions/)
    std::path::PathBuf::from("./extensions")
}

/// extensions/ 디렉토리 경로 (public wrapper)
pub fn resolve_extensions_dir_pub() -> std::path::PathBuf {
    resolve_extensions_dir()
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

/// Like `run_plugin` but with a custom timeout (seconds).
pub async fn run_plugin_with_timeout(module_path: &str, function: &str, config: Value, timeout_secs: u64) -> Result<Value> {
    run_plugin_inner(module_path, function, config, None, timeout_secs).await
}

/// Like `run_plugin` but invokes a callback for each `PROGRESS:{json}` line
/// emitted on stderr by the Python extension.
pub async fn run_plugin_with_progress<F>(
    module_path: &str,
    function: &str,
    config: Value,
    on_progress: F,
) -> Result<Value>
where
    F: Fn(ExtensionProgress) + Send + 'static,
{
    run_plugin_inner(module_path, function, config, Some(Box::new(on_progress)), DEFAULT_PLUGIN_TIMEOUT_SECS).await
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
        .env("PYTHONIOENCODING", "utf-8");

    let extensions_dir = resolve_extensions_dir();
    if extensions_dir.is_dir() {
        let mut pypath = extensions_dir.parent()
            .unwrap_or(&extensions_dir)
            .to_string_lossy()
            .into_owned();
        if let Ok(existing) = std::env::var("PYTHONPATH") {
            pypath = format!("{}{}{}", pypath, std::path::MAIN_SEPARATOR, existing);
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
                tracing::error!("Plugin failed (exit {:?}): {}", exit_status.code(), stderr_str);
                return Err(anyhow::anyhow!("Plugin execution failed: {}", stderr_str));
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

// detect_python_command()는 python_env::get_python_path()로 대체됨.
// 시스템 Python 탐지는 python_env 모듈에서 venv 부트스트랩 시 수행합니다.

/// Extension runner – extensions/<name>.py 를 plugin 프로토콜로 실행합니다.
/// run_plugin과 동일한 프로토콜이지만, 경로를 extensions/ 디렉토리에서 자동으로 해석합니다.
#[allow(dead_code)]
pub async fn run_extension(extension_name: &str, function: &str, config: Value) -> Result<Value> {
    let extensions_dir = resolve_extensions_dir();

    let ext_path = extensions_dir.join(format!("{}.py", extension_name));
    if !ext_path.exists() {
        return Err(anyhow::anyhow!(
            "Extension not found: {} (searched {})",
            extension_name,
            ext_path.display()
        ));
    }

    run_plugin(ext_path.to_string_lossy().as_ref(), function, config).await
}

/// Like `run_extension` but with real-time progress callback.
#[allow(dead_code)]
pub async fn run_extension_with_progress<F>(
    extension_name: &str,
    function: &str,
    config: Value,
    on_progress: F,
) -> Result<Value>
where
    F: Fn(ExtensionProgress) + Send + 'static,
{
    let extensions_dir = resolve_extensions_dir();
    let ext_path = extensions_dir.join(format!("{}.py", extension_name));
    if !ext_path.exists() {
        return Err(anyhow::anyhow!(
            "Extension not found: {} (searched {})",
            extension_name,
            ext_path.display()
        ));
    }
    run_plugin_with_progress(ext_path.to_string_lossy().as_ref(), function, config, on_progress).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
